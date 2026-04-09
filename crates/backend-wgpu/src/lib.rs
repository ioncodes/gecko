mod helpers;
mod pipeline;
mod render;
pub mod texture;
mod triangulate;

use encase::ShaderType as _;
use gecko::flipper::gx::draw::TextureFormat;
use gecko::flipper::gx::regs::{MagFilter, MinFilter, WrapMode};
use pipeline::PipelineKey;
use std::collections::HashMap;
use triangulate::{GpuVertex, align_up};

type TexKey = (usize, u32, u32, TextureFormat);
type SamplerKey = (WrapMode, WrapMode, MagFilter, MinFilter);

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct BindGroupCacheKey {
    tex_keys: [Option<TexKey>; 8],
    sampler_keys: [Option<SamplerKey>; 8],
}

#[derive(encase::ShaderType)]
pub(crate) struct FrameUniforms {
    tev_color_regs: [glam::Vec4; 4],
    tev_konst_colors: [glam::Vec4; 16],
    tev_color_env: [glam::UVec4; 4],
    tev_alpha_env: [glam::UVec4; 4],
    tev_orders: [glam::UVec4; 4],
    num_tev_stages: u32,
    alpha_ref0: f32,
    alpha_ref1: f32,
    alpha_comp0: u32,
    alpha_comp1: u32,
    alpha_op: u32,
    light_colors: [glam::Vec4; 8],
    light_cosatt: [glam::Vec4; 8],
    light_distatt: [glam::Vec4; 8],
    light_pos: [glam::Vec4; 8],
    light_dir: [glam::Vec4; 8],
    color_ctrl: u32,
    alpha_ctrl: u32,
    ambient_color: glam::Vec4,
    material_color: glam::Vec4,
}

#[derive(encase::ShaderType)]
pub(crate) struct DrawUniforms {
    mvp: glam::Mat4,
}

const SHADER: &str = wesl::include_wesl!("gx_shader");

pub struct GxRenderer {
    pub(crate) pipeline_cache: HashMap<PipelineKey, wgpu::RenderPipeline>,
    pub(crate) shader: wgpu::ShaderModule,
    pub(crate) pipeline_layout: wgpu::PipelineLayout,
    pub(crate) surface_format: wgpu::TextureFormat,
    pub(crate) bind_group_layout: wgpu::BindGroupLayout,
    pub(crate) frame_uniform_buffer: wgpu::Buffer,
    pub(crate) draw_uniform_buffer: wgpu::Buffer,
    pub(crate) draw_uniform_stride: u64,
    pub(crate) draw_uniform_capacity: usize,
    pub(crate) vertex_buffer: wgpu::Buffer,
    pub(crate) vertex_capacity: usize,
    pub(crate) depth_texture: wgpu::Texture,
    pub(crate) depth_view: wgpu::TextureView,
    pub(crate) depth_width: u32,
    pub(crate) depth_height: u32,
    pub(crate) sampler_cache: HashMap<(WrapMode, WrapMode, MagFilter, MinFilter), wgpu::Sampler>,
    pub(crate) texture_cache: HashMap<(usize, u32, u32, TextureFormat), (wgpu::Texture, wgpu::TextureView)>,
    pub(crate) fallback_view: wgpu::TextureView,
    pub(crate) scratch_vertices: Vec<GpuVertex>,
    pub(crate) scratch_draws: Vec<(u32, u32)>,
    pub(crate) scratch_uniform_bytes: Vec<u8>,
    pub(crate) bind_group_cache: HashMap<BindGroupCacheKey, wgpu::BindGroup>,
}

impl GxRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let frame_uniform_size = FrameUniforms::min_size().get();
        let draw_uniform_size = DrawUniforms::min_size().get();
        let draw_uniform_stride = align_up(
            draw_uniform_size,
            device.limits().min_uniform_buffer_offset_alignment as u64,
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gx_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        // Bindings: 0=FrameUniforms, 1=DrawUniforms, 2-9=textures 0-7, 10-17=samplers 0-7
        let mut layout_entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: Some(FrameUniforms::min_size()),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: Some(DrawUniforms::min_size()),
                },
                count: None,
            },
        ];
        for i in 0..8u32 {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 2 + i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        for i in 0..8u32 {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 10 + i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &layout_entries,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });

        let frame_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx_frame_uniforms"),
            size: frame_uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let draw_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx_draw_uniforms"),
            size: draw_uniform_stride,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gx_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let fallback_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gx_fallback_tex"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            fallback_texture.as_image_copy(),
            &[255u8, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let fallback_view = fallback_texture.create_view(&Default::default());

        let initial_capacity = 1024;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx_vertices"),
            size: (initial_capacity * std::mem::size_of::<GpuVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let depth_width = width.max(1);
        let depth_height = height.max(1);
        let (depth_texture, depth_view) = create_depth_texture(device, depth_width, depth_height);

        GxRenderer {
            pipeline_cache: HashMap::new(),
            shader,
            pipeline_layout,
            surface_format,
            bind_group_layout,
            frame_uniform_buffer,
            draw_uniform_buffer,
            draw_uniform_stride,
            draw_uniform_capacity: 1,
            vertex_buffer,
            vertex_capacity: initial_capacity,
            depth_texture,
            depth_view,
            depth_width,
            depth_height,
            sampler_cache: HashMap::from([(
                (WrapMode::Clamp, WrapMode::Clamp, MagFilter::Linear, MinFilter::Linear),
                sampler,
            )]),
            texture_cache: HashMap::new(),
            fallback_view,
            scratch_vertices: Vec::new(),
            scratch_draws: Vec::new(),
            scratch_uniform_bytes: Vec::new(),
            bind_group_cache: HashMap::new(),
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.ensure_depth_texture(device, width, height);
    }

    pub(crate) fn ensure_depth_texture(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if (width, height) == (self.depth_width, self.depth_height) {
            return;
        }
        let (tex, view) = create_depth_texture(device, width, height);
        self.depth_texture = tex;
        self.depth_view = view;
        self.depth_width = width;
        self.depth_height = height;
    }
}

fn create_depth_texture(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gx_depth"),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth24Plus,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    (tex, view)
}
