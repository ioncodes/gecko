mod action;
mod clear;
mod helpers;
mod pipeline;
mod render;
pub mod sink;

use encase::ShaderType as _;
use gecko::flipper::gx::draw::{Scissor, Viewport};
use gecko::flipper::gx::regs::{AlphaCompare, BlendMode, CompareFunc, CullMode, MagFilter, MinFilter, WrapMode, ZMode};
use gecko::host::TextureId;
use glam::Mat4;
use pipeline::PipelineKey;
use std::collections::HashMap;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GpuVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
    pub color1: [f32; 4],
    pub normal: [f32; 3],
    pub pos_view: [f32; 3],
    pub tex0: [f32; 2],
    pub tex1: [f32; 2],
    pub tex2: [f32; 2],
    pub tex3: [f32; 2],
    pub tex4: [f32; 2],
    pub tex5: [f32; 2],
    pub tex6: [f32; 2],
    pub tex7: [f32; 2],
}

pub(crate) fn align_up(value: u64, alignment: u64) -> u64 {
    (value + alignment - 1) & !(alignment - 1)
}

type SamplerKey = (WrapMode, WrapMode, MagFilter, MinFilter);

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct BindGroupCacheKey {
    tex_keys: [Option<TextureId>; 8],
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
    color_ctrl0: u32,
    alpha_ctrl0: u32,
    color_ctrl1: u32,
    alpha_ctrl1: u32,
    ambient_color0: glam::Vec4,
    ambient_color1: glam::Vec4,
    material_color0: glam::Vec4,
    material_color1: glam::Vec4,
}

#[derive(encase::ShaderType)]
pub(crate) struct DrawUniforms {
    mvp: glam::Mat4,
}

const SHADER: &str = wesl::include_wesl!("gx_shader");

pub const EFB_WIDTH: u32 = 640;
pub const EFB_HEIGHT: u32 = 528;
pub const EFB_SAMPLE_COUNT: u32 = 4;

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
    // EFB: resolved (1x) color used for CopyXfb reads + texture binding.
    pub(crate) efb_texture: wgpu::Texture,
    pub(crate) efb_view: wgpu::TextureView,
    // EFB: multisampled (4x) color, actual render target for draws.
    pub(crate) _efb_msaa_texture: wgpu::Texture,
    pub(crate) efb_msaa_view: wgpu::TextureView,
    // EFB: multisampled (4x) depth.
    pub(crate) efb_depth_view: wgpu::TextureView,
    pub(crate) efb_needs_clear: bool,
    pub(crate) sampler_cache: HashMap<(WrapMode, WrapMode, MagFilter, MinFilter), wgpu::Sampler>,
    pub(crate) texture_cache: HashMap<TextureId, (wgpu::Texture, wgpu::TextureView)>,
    pub(crate) fallback_view: wgpu::TextureView,
    pub(crate) scratch_vertices: Vec<GpuVertex>,
    pub(crate) scratch_draws: Vec<(u32, u32)>,
    pub(crate) scratch_uniform_bytes: Vec<u8>,
    pub(crate) bind_group_cache: HashMap<BindGroupCacheKey, wgpu::BindGroup>,
    // Tracked GX state (updated by state-change actions)
    pub(crate) current_projection: Mat4,
    pub(crate) current_viewport: Viewport,
    pub(crate) current_scissor: Scissor,
    pub(crate) current_zmode: ZMode,
    pub(crate) current_blend_mode: BlendMode,
    pub(crate) current_alpha_compare: AlphaCompare,
    pub(crate) current_cull_mode: CullMode,
    pub(crate) current_texture_ids: [Option<TextureId>; 8],
    pub(crate) current_sampler_keys: [Option<SamplerKey>; 8],
    // Blit pipeline (EFB -> swapchain)
    pub(crate) blit_pipeline: wgpu::RenderPipeline,
    pub(crate) blit_bind_group_layout: wgpu::BindGroupLayout,
    pub(crate) blit_sampler: wgpu::Sampler,
    // XFB output texture: composited from per-copy snapshots by PresentXfb.
    pub(crate) xfb_texture: wgpu::Texture,
    pub(crate) xfb_view: wgpu::TextureView,
    pub(crate) xfb_has_content: bool,
    // Per-copy temporary textures stored by CopyXfb, composited by PresentXfb.
    pub(crate) xfb_copies: HashMap<u32, (wgpu::Texture, wgpu::TextureView)>,
    // Region-scoped EFB clear.
    pub(crate) efb_clear: clear::EfbClear,
}

impl GxRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, surface_format: wgpu::TextureFormat) -> Self {
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
            format: wgpu::TextureFormat::Rgba8Unorm,
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

        // Create fixed-size EFB with 4x MSAA.
        // Resolved (1x) color: CopyXfb reads from here.
        let efb_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("efb_color_resolved"),
            size: wgpu::Extent3d {
                width: EFB_WIDTH,
                height: EFB_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: surface_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let efb_view = efb_texture.create_view(&Default::default());

        // Multisampled (4x) color: actual render target for draws.
        let efb_msaa_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("efb_color_msaa"),
            size: wgpu::Extent3d {
                width: EFB_WIDTH,
                height: EFB_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: EFB_SAMPLE_COUNT,
            dimension: wgpu::TextureDimension::D2,
            format: surface_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let efb_msaa_view = efb_msaa_texture.create_view(&Default::default());

        // Multisampled (4x) depth.
        let efb_depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("efb_depth"),
            size: wgpu::Extent3d {
                width: EFB_WIDTH,
                height: EFB_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: EFB_SAMPLE_COUNT,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24Plus,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let efb_depth_view = efb_depth_texture.create_view(&Default::default());

        // XFB accumulation texture. Holds the composited output of all
        // CopyEfb(copy_to_xfb) ops for the current frame.
        let xfb_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xfb_accum"),
            size: wgpu::Extent3d {
                width: EFB_WIDTH,
                height: EFB_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: surface_format,
            usage: wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let xfb_view = xfb_texture.create_view(&Default::default());

        // Blit pipeline EFB -> swapchain shit
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("efb_blit_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/efb_blit.wgsl").into()),
        });
        let blit_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("efb_blit_bg_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&blit_bind_group_layout],
            immediate_size: 0,
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("efb_blit_pipeline"),
            layout: Some(&blit_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("efb_blit_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let efb_clear = clear::EfbClear::new(
            device,
            surface_format,
            wgpu::TextureFormat::Depth24Plus,
            EFB_SAMPLE_COUNT,
        );

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
            efb_texture,
            efb_view,
            _efb_msaa_texture: efb_msaa_texture,
            efb_msaa_view,
            efb_depth_view,
            efb_needs_clear: true,
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
            current_projection: Mat4::IDENTITY,
            current_viewport: Viewport::default(),
            current_scissor: Scissor::default(),
            current_zmode: ZMode::default(),
            current_blend_mode: BlendMode::from_raw(0).with_color_update(true).with_alpha_update(true),
            current_cull_mode: CullMode::None,
            current_alpha_compare: AlphaCompare::from_raw(0)
                .with_comp0(CompareFunc::Always)
                .with_comp1(CompareFunc::Always),
            current_texture_ids: Default::default(),
            current_sampler_keys: Default::default(),
            blit_pipeline,
            blit_bind_group_layout,
            blit_sampler,
            xfb_texture,
            xfb_view,
            xfb_has_content: false,
            xfb_copies: HashMap::new(),
            efb_clear,
        }
    }
}
