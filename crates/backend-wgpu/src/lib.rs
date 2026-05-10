mod action;
pub mod capture;
mod clear;
#[cfg(not(target_arch = "wasm32"))]
mod dump;
mod helpers;
mod pipeline;
mod render;
#[cfg(feature = "renderdoc-capture")]
mod renderdoc_capture;
pub mod sink;

use gecko::common::Address;
#[cfg(feature = "renderdoc-capture")]
use gecko::flipper::gx::draw::Primitive;
use gecko::flipper::gx::draw::{Scissor, TextureFormat, Viewport};
use gecko::flipper::gx::regs::{AlphaCompare, BlendMode, CompareFunc, CullMode, MagFilter, MinFilter, WrapMode, ZMode};
#[cfg(feature = "efb-writeback")]
use gecko::host::EfbWriteback;
use gecko::host::TextureKey;
use glam::Mat4;
use pipeline::PipelineKey;
use rustc_hash::FxHashMap;
use std::num::NonZeroU64;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GpuVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
    pub color1: [f32; 4],
    pub normal: [f32; 3],
    pub pos_view: [f32; 3],
    pub tex0: [f32; 3],
    pub tex1: [f32; 3],
    pub tex2: [f32; 3],
    pub tex3: [f32; 3],
    pub tex4: [f32; 3],
    pub tex5: [f32; 3],
    pub tex6: [f32; 3],
    pub tex7: [f32; 3],
}

pub(crate) fn align_up(value: u64, alignment: u64) -> u64 {
    (value + alignment - 1) & !(alignment - 1)
}

type SamplerKey = (WrapMode, WrapMode, MagFilter, MinFilter);

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct BindGroupCacheKey {
    tex_keys: [Option<TextureKey>; 8],
    sampler_keys: [Option<SamplerKey>; 8],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct FrameUniforms {
    pub tev_color_regs: [glam::Vec4; 4],
    pub tev_konst_colors: [glam::Vec4; 16],
    pub tev_color_env: [glam::UVec4; 4],
    pub tev_alpha_env: [glam::UVec4; 4],
    pub tev_orders: [glam::UVec4; 4],
    pub tev_ksel: [glam::UVec4; 4],
    pub num_tev_stages: u32,
    pub alpha_ref0: f32,
    pub alpha_ref1: f32,
    pub alpha_comp0: u32,
    pub alpha_comp1: u32,
    pub alpha_op: u32,
    pub _pad0: [u32; 2],
    // Indirect texturing state. `indirect_matrices` stores the 6 rows of
    // the three 2x3 matrices.
    pub indirect_matrices: [glam::IVec4; 6],
    // Two packed RAS1_SS registers holding four 4-bit divisor exponents
    // each. Stage i reads from `indirect_scales[i / 2]`.
    pub indirect_scales: [glam::UVec4; 2],
    // Packed RAS1_IREF. Four 3+3 bit (texmap, texcoord) pairs.
    pub indirect_refs: u32,
    pub num_indirect_stages: u32,
    pub bump_imask: u32,
    pub _pad1: u32,
    // Per-TEV-stage IND_CMD, packed four per UVec4.
    pub tev_indirect: [glam::UVec4; 4],
    pub light_colors: [glam::Vec4; 8],
    pub light_cosatt: [glam::Vec4; 8],
    pub light_distatt: [glam::Vec4; 8],
    pub light_pos: [glam::Vec4; 8],
    pub light_dir: [glam::Vec4; 8],
    pub color_ctrl0: u32,
    pub alpha_ctrl0: u32,
    pub color_ctrl1: u32,
    pub alpha_ctrl1: u32,
    pub ambient_color0: glam::Vec4,
    pub ambient_color1: glam::Vec4,
    pub material_color0: glam::Vec4,
    pub material_color1: glam::Vec4,
}

pub(crate) const FRAME_UNIFORMS_SIZE: NonZeroU64 = match NonZeroU64::new(std::mem::size_of::<FrameUniforms>() as u64) {
    Some(v) => v,
    None => panic!("FrameUniforms must be non-zero sized"),
};
const _: () = assert!(std::mem::size_of::<FrameUniforms>() == 1536);

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct DrawUniforms {
    pub mvp: glam::Mat4,
}

pub(crate) const DRAW_UNIFORMS_SIZE: NonZeroU64 = match NonZeroU64::new(std::mem::size_of::<DrawUniforms>() as u64) {
    Some(v) => v,
    None => panic!("DrawUniforms must be non-zero sized"),
};

const SHADER: &str = wesl::include_wesl!("gx_shader");

pub const EFB_WIDTH: u32 = 640;
pub const EFB_HEIGHT: u32 = 528;
pub const EFB_SAMPLE_COUNT: u32 = 4;

pub struct GxRenderer {
    pub(crate) pipeline_cache: FxHashMap<PipelineKey, wgpu::RenderPipeline>,
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
    pub(crate) sampler_cache: FxHashMap<(WrapMode, WrapMode, MagFilter, MinFilter), wgpu::Sampler>,
    pub(crate) texture_cache: FxHashMap<TextureKey, (TextureFormat, wgpu::Texture, wgpu::TextureView)>,
    // GPU textures holding EFB copy results, checked before `texture_cache` on every texture bind.
    pub(crate) efb_copy_cache: FxHashMap<Address, (wgpu::Texture, wgpu::TextureView)>,
    // Retired EFB-copy textures grouped by size, popped by the next copy to skip reallocating.
    pub(crate) efb_copy_pool: FxHashMap<(u32, u32), Vec<(wgpu::Texture, wgpu::TextureView)>>,
    // Blits a sub-rect of `efb_view` into an `Rgba8Unorm` target, reusing xfb_copy's shader and layout.
    pub(crate) efb_copy_pipeline: wgpu::RenderPipeline,
    // Retired depth copy textures grouped by size, kept separate so the (w, h) key isn't shared with color.
    pub(crate) efb_depth_pool: FxHashMap<(u32, u32), Vec<(wgpu::Texture, wgpu::TextureView)>>,
    // Resolves a sub-rect of the 4x MSAA `efb_depth_view` into an R32Float target.
    pub(crate) efb_depth_resolve_pipeline: wgpu::RenderPipeline,
    pub(crate) efb_depth_resolve_bg_layout: wgpu::BindGroupLayout,
    pub(crate) efb_depth_resolve_uniform_buffer: wgpu::Buffer,
    pub(crate) fallback_view: wgpu::TextureView,
    pub(crate) scratch_vertices: Vec<GpuVertex>,
    pub(crate) scratch_draws: Vec<(u32, u32)>,
    pub(crate) scratch_uniform_bytes: Vec<u8>,
    pub(crate) bind_group_cache: FxHashMap<BindGroupCacheKey, wgpu::BindGroup>,
    // Per-frame draw accumulation (persists across process_action calls,
    // flushed by flush_pending_draws).
    pub(crate) frame_uniform_bytes: Vec<u8>,
    pub(crate) draw_pipeline_keys: Vec<PipelineKey>,
    pub(crate) draw_bg_keys: Vec<BindGroupCacheKey>,
    pub(crate) draw_viewports: Vec<Viewport>,
    pub(crate) draw_scissors: Vec<Scissor>,
    /// Per-draw index into the FrameUniforms array. Multiple consecutive
    /// draws that share TEV/lighting/alpha/indirect state point at the same
    /// slot via dynamic offset; only state-changing draws push a fresh slot
    /// (driven by `DrawData::frame_dirty` from the producer).
    pub(crate) draw_frame_indices: Vec<u32>,
    /// Index of the most recently appended FrameUniforms slot, reused when
    /// `frame_dirty` is false. Reset to `None` at each `flush_pending_draws`.
    pub(crate) last_frame_uniform_index: Option<u32>,
    #[cfg(feature = "renderdoc-capture")]
    pub(crate) draw_primitives: Vec<Primitive>,
    pub(crate) frame_stride: usize,
    // Tracked GX state (updated by state-change actions)
    pub(crate) current_projection: Mat4,
    pub(crate) current_viewport: Viewport,
    pub(crate) current_scissor: Scissor,
    pub(crate) current_zmode: ZMode,
    pub(crate) current_blend_mode: BlendMode,
    pub(crate) current_alpha_compare: AlphaCompare,
    pub(crate) current_cull_mode: CullMode,
    pub(crate) current_texture_ids: [Option<TextureKey>; 8],
    pub(crate) current_sampler_keys: [Option<SamplerKey>; 8],
    // XFB output texture: composited from per-copy snapshots by PresentXfb.
    pub xfb_texture: wgpu::Texture,
    pub xfb_view: wgpu::TextureView,
    pub(crate) xfb_has_content: bool,
    // Per-copy temporary textures stored by CopyXfb, composited by PresentXfb.
    pub(crate) xfb_copies: FxHashMap<u32, (wgpu::Texture, wgpu::TextureView)>,
    pub(crate) xfb_copy_pipeline: wgpu::RenderPipeline,
    pub(crate) xfb_copy_bind_group_layout: wgpu::BindGroupLayout,
    pub(crate) xfb_copy_sampler: wgpu::Sampler,
    pub(crate) xfb_copy_uniform_buffer: wgpu::Buffer,
    // Region-scoped EFB clear.
    pub(crate) efb_clear: clear::EfbClear,
    // EFB-to-texture readback. Only allocated with `efb-writeback`.
    #[cfg(feature = "efb-writeback")]
    pub(crate) efb_readback_staging: Option<wgpu::Buffer>,
    #[cfg(feature = "efb-writeback")]
    pub(crate) efb_readback_capacity: u64,
    // Sender used to ship encoded EFB-to-texture bytes back to the emu
    // thread. Installed by `sink::Renderer::new` via
    // [`GxRenderer::set_efb_writeback_tx`].
    #[cfg(feature = "efb-writeback")]
    pub(crate) efb_writeback_tx: Option<crossbeam_channel::Sender<EfbWriteback>>,
}

impl GxRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, surface_format: wgpu::TextureFormat) -> Self {
        let frame_uniform_size = FRAME_UNIFORMS_SIZE.get();
        let draw_uniform_size = DRAW_UNIFORMS_SIZE.get();
        let draw_uniform_stride = align_up(
            draw_uniform_size,
            device.limits().min_uniform_buffer_offset_alignment as u64,
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gx_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let xfb_copy_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xfb_copy_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/xfb_copy.wgsl").into()),
        });
        let efb_depth_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("efb_depth_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/efb_depth.wgsl").into()),
        });

        // Bindings: 0=FrameUniforms, 1=DrawUniforms, 2-9=textures 0-7, 10-17=samplers 0-7
        let mut layout_entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: Some(FRAME_UNIFORMS_SIZE),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: Some(DRAW_UNIFORMS_SIZE),
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
            label: Some("gx_bind_group_layout"),
            entries: &layout_entries,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gx_pipeline_layout"),
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let efb_depth_view = efb_depth_texture.create_view(&Default::default());

        // XFB accumulation texture. Holds the composited output of all
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
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let xfb_view = xfb_texture.create_view(&Default::default());

        let xfb_copy_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xfb_copy_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let xfb_copy_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xfb_copy_layout"),
            bind_group_layouts: &[&xfb_copy_bind_group_layout],
            immediate_size: 0,
        });
        let xfb_copy_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xfb_copy_pipeline"),
            layout: Some(&xfb_copy_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &xfb_copy_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &xfb_copy_shader,
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
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });

        // Same shader and layout as xfb_copy, but writes `Rgba8Unorm` so the output matches `LoadTexture`.
        let efb_copy_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("efb_copy_pipeline"),
            layout: Some(&xfb_copy_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &xfb_copy_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &xfb_copy_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
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
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });

        let xfb_copy_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xfb_copy_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let xfb_copy_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xfb_copy_uniforms"),
            size: std::mem::size_of::<render::XfbCopyUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let efb_depth_resolve_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("efb_depth_resolve_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: true,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Depth,
                    },
                    count: None,
                },
            ],
        });
        let efb_depth_resolve_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("efb_depth_resolve_layout"),
            bind_group_layouts: &[&efb_depth_resolve_bg_layout],
            immediate_size: 0,
        });
        let efb_depth_resolve_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("efb_depth_resolve_pipeline"),
            layout: Some(&efb_depth_resolve_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &efb_depth_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &efb_depth_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R16Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::RED,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });
        let efb_depth_resolve_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("efb_depth_resolve_uniforms"),
            size: std::mem::size_of::<render::XfbCopyUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let efb_clear = clear::EfbClear::new(
            device,
            surface_format,
            wgpu::TextureFormat::Depth24Plus,
            EFB_SAMPLE_COUNT,
        );

        GxRenderer {
            pipeline_cache: FxHashMap::default(),
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
            sampler_cache: {
                let mut m = FxHashMap::default();
                m.insert(
                    (WrapMode::Clamp, WrapMode::Clamp, MagFilter::Linear, MinFilter::Linear),
                    sampler,
                );
                m
            },
            texture_cache: FxHashMap::default(),
            efb_copy_cache: FxHashMap::default(),
            efb_copy_pool: FxHashMap::default(),
            efb_copy_pipeline,
            efb_depth_pool: FxHashMap::default(),
            efb_depth_resolve_pipeline,
            efb_depth_resolve_bg_layout,
            efb_depth_resolve_uniform_buffer,
            fallback_view,
            scratch_vertices: Vec::new(),
            scratch_draws: Vec::new(),
            scratch_uniform_bytes: Vec::new(),
            bind_group_cache: FxHashMap::default(),
            frame_uniform_bytes: Vec::new(),
            draw_pipeline_keys: Vec::new(),
            draw_bg_keys: Vec::new(),
            draw_viewports: Vec::new(),
            draw_scissors: Vec::new(),
            draw_frame_indices: Vec::new(),
            last_frame_uniform_index: None,
            #[cfg(feature = "renderdoc-capture")]
            draw_primitives: Vec::new(),
            frame_stride: align_up(
                FRAME_UNIFORMS_SIZE.get(),
                device.limits().min_uniform_buffer_offset_alignment as u64,
            ) as usize,
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
            xfb_texture,
            xfb_view,
            xfb_has_content: false,
            xfb_copies: FxHashMap::default(),
            xfb_copy_pipeline,
            xfb_copy_bind_group_layout,
            xfb_copy_sampler,
            xfb_copy_uniform_buffer,
            efb_clear,
            #[cfg(feature = "efb-writeback")]
            efb_readback_staging: None,
            #[cfg(feature = "efb-writeback")]
            efb_readback_capacity: 0,
            #[cfg(feature = "efb-writeback")]
            efb_writeback_tx: None,
        }
    }

    /// Install the sender used to ship encoded EFB-to-texture bytes back
    /// to the emulator thread. Called by `sink::Renderer` during setup.
    #[cfg(feature = "efb-writeback")]
    pub fn set_efb_writeback_tx(&mut self, tx: crossbeam_channel::Sender<EfbWriteback>) {
        self.efb_writeback_tx = Some(tx);
    }
}
