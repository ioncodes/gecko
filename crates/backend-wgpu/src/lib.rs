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
mod shader_specialization;
pub mod sink;

use gecko::common::Address;
#[cfg(feature = "renderdoc-capture")]
use gecko::flipper::gx::draw::Primitive;
use gecko::flipper::gx::draw::{Scissor, TextureFormat, Viewport};
use gecko::flipper::gx::regs::{AlphaCompare, BlendMode, CompareFunc, CullMode, MagFilter, MinFilter, WrapMode, ZMode};

use gecko::host::TextureKey;
use glam::Mat4;
use pipeline::{FullPipelineKey, UberPipelineKey};
use rustc_hash::FxHashMap;
#[cfg(not(target_arch = "wasm32"))]
use rustc_hash::FxHashSet;
use shader_specialization::ShaderKey;
use std::num::NonZeroU64;
#[cfg(feature = "gx-stats")]
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub(crate) type GpuVertex = gecko::host::DrawVertex;

#[cfg(feature = "gx-stats")]
#[derive(Default)]
pub(crate) struct RendererStats {
    pub(crate) actions_sent: AtomicU64,
    pub(crate) batches_sent: AtomicU64,
    pub(crate) channel_high_water: AtomicU64,
    pub(crate) queue_wait_ns: AtomicU64,
    pub(crate) efb_drain_wait_ns: AtomicU64,
    pub(crate) efb_drain_requests: AtomicU64,
    pub(crate) efb_drain_nonempty: AtomicU64,
    pub(crate) efb_writebacks: AtomicU64,
    pub(crate) efb_writeback_cpu_ns: AtomicU64,
    pub(crate) worker_batch_cpu_ns: AtomicU64,
    pub(crate) draws_encoded: AtomicU64,
    pub(crate) draw_render_passes: AtomicU64,
    pub(crate) pipeline_changes: AtomicU64,
    pub(crate) pipelines_created: AtomicU64,
    pub(crate) pipeline_create_cpu_ns: AtomicU64,
    pub(crate) shader_modules_created: AtomicU64,
    pub(crate) shader_create_cpu_ns: AtomicU64,
    pub(crate) bind_group_sets: AtomicU64,
    pub(crate) bind_groups_created: AtomicU64,
    pub(crate) bind_group_key_changes: AtomicU64,
    pub(crate) frame_uniform_changes: AtomicU64,
    pub(crate) draw_uniform_changes: AtomicU64,
    pub(crate) vertex_stride_changes: AtomicU64,
    pub(crate) potential_merged_draws: AtomicU64,
    pub(crate) viewport_changes: AtomicU64,
    pub(crate) scissor_changes: AtomicU64,
    pub(crate) draw_pass_encode_ns: AtomicU64,
    pub(crate) queue_submits: AtomicU64,
    pub(crate) command_buffers_submitted: AtomicU64,
    pub(crate) queue_submit_cpu_ns: AtomicU64,
}

#[cfg(feature = "gx-stats")]
impl RendererStats {
    pub(crate) fn snapshot(&self, channel_len: usize, channel_cap: usize) -> gecko::host::RenderStats {
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        gecko::host::RenderStats {
            actions_sent: load(&self.actions_sent),
            batches_sent: load(&self.batches_sent),
            channel_len,
            channel_cap,
            channel_high_water: load(&self.channel_high_water) as usize,
            queue_wait_ns: load(&self.queue_wait_ns),
            efb_drain_wait_ns: load(&self.efb_drain_wait_ns),
            efb_drain_requests: load(&self.efb_drain_requests),
            efb_drain_nonempty: load(&self.efb_drain_nonempty),
            efb_writebacks: load(&self.efb_writebacks),
            efb_writeback_cpu_ns: load(&self.efb_writeback_cpu_ns),
            worker_batch_cpu_ns: load(&self.worker_batch_cpu_ns),
            draws_encoded: load(&self.draws_encoded),
            draw_render_passes: load(&self.draw_render_passes),
            pipeline_changes: load(&self.pipeline_changes),
            pipelines_created: load(&self.pipelines_created),
            pipeline_create_cpu_ns: load(&self.pipeline_create_cpu_ns),
            shader_modules_created: load(&self.shader_modules_created),
            shader_create_cpu_ns: load(&self.shader_create_cpu_ns),
            bind_group_sets: load(&self.bind_group_sets),
            bind_groups_created: load(&self.bind_groups_created),
            bind_group_key_changes: load(&self.bind_group_key_changes),
            frame_uniform_changes: load(&self.frame_uniform_changes),
            draw_uniform_changes: load(&self.draw_uniform_changes),
            vertex_stride_changes: load(&self.vertex_stride_changes),
            potential_merged_draws: load(&self.potential_merged_draws),
            viewport_changes: load(&self.viewport_changes),
            scissor_changes: load(&self.scissor_changes),
            draw_pass_encode_ns: load(&self.draw_pass_encode_ns),
            queue_submits: load(&self.queue_submits),
            command_buffers_submitted: load(&self.command_buffers_submitted),
            queue_submit_cpu_ns: load(&self.queue_submit_cpu_ns),
        }
    }
}

pub(crate) fn align_up(value: u64, alignment: u64) -> u64 {
    (value + alignment - 1) & !(alignment - 1)
}

pub(crate) fn compute_draw_buffer_layout(
    uniform_alignment: u64,
    frame_capacity: u64,
    draw_capacity: u64,
    vertex_capacity: u64,
    index_capacity: u64,
) -> DrawBufferLayout {
    let frame_offset = 0;
    let draw_offset = align_up(frame_offset + frame_capacity, uniform_alignment);
    let vertex_offset = align_up(draw_offset + draw_capacity, uniform_alignment);
    let index_offset = align_up(vertex_offset + vertex_capacity, 4);
    let total_size = (index_offset + index_capacity).max(uniform_alignment);

    DrawBufferLayout {
        frame_offset,
        frame_capacity,
        draw_offset,
        draw_capacity,
        vertex_offset,
        vertex_capacity,
        index_offset,
        index_capacity,
        total_size,
    }
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
    // Z-texture state: bias (24-bit), texel format (0=u8/1=u16/2=u24) and
    // op (0=disabled, 1=add, 2=replace). Consumed by the fs_main_ztex entry
    // point, which derives the fragment depth from the last texture lookup.
    pub ztex_bias: u32,
    pub ztex_type: u32,
    pub ztex_op: u32,
    pub _pad2: u32,
}

pub(crate) const FRAME_UNIFORMS_SIZE: NonZeroU64 = match NonZeroU64::new(std::mem::size_of::<FrameUniforms>() as u64) {
    Some(v) => v,
    None => panic!("FrameUniforms must be non-zero sized"),
};
const _: () = assert!(std::mem::size_of::<FrameUniforms>() == 1552);

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct DrawUniforms {
    pub mvp: glam::Mat4,
    /// Guest texture dimensions for texmaps 0-7, two slots per
    /// vector (slot i at `tex_dims[i / 2]`, xy for even i, zw for odd).
    /// Indirect texturing needs these instead of `textureDimensions`
    /// because scaled EFB copies bind a larger GPU texture.
    pub tex_dims: [glam::UVec4; 4],
}

pub(crate) const DRAW_UNIFORMS_SIZE: NonZeroU64 = match NonZeroU64::new(std::mem::size_of::<DrawUniforms>() as u64) {
    Some(v) => v,
    None => panic!("DrawUniforms must be non-zero sized"),
};

pub const EFB_WIDTH: u32 = 640;
pub const EFB_HEIGHT: u32 = 528;
pub const EFB_SAMPLE_COUNT: u32 = 4;
pub const MAX_EFB_SCALE: u32 = 4;

pub(crate) struct EfbCopyEntry {
    pub(crate) format: gecko::flipper::gx::texture::CopyFormat,
    /// Guest dimensions. The GPU texture itself is `efb_scale`
    /// times larger.
    pub(crate) native_w: u32,
    pub(crate) native_h: u32,
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
}

impl EfbCopyEntry {
    pub(crate) fn matches(&self, fmt: TextureFormat, w: u32, h: u32) -> bool {
        self.format.base_texture_format() == fmt && self.native_w == w && self.native_h == h
    }
}

/// Layout of [`GxRenderer::draw_buffer`]: a single backing wgpu buffer with
/// usage `COPY_DST | UNIFORM | VERTEX | INDEX` that holds all per-frame
/// upload data in four sections at fixed offsets.
#[derive(Copy, Clone, Debug)]
pub(crate) struct DrawBufferLayout {
    pub(crate) frame_offset: u64,
    pub(crate) frame_capacity: u64,
    pub(crate) draw_offset: u64,
    pub(crate) draw_capacity: u64,
    pub(crate) vertex_offset: u64,
    pub(crate) vertex_capacity: u64,
    pub(crate) index_offset: u64,
    pub(crate) index_capacity: u64,
    pub(crate) total_size: u64,
}

pub struct GxRenderer {
    #[cfg(feature = "gx-stats")]
    pub(crate) stats: Arc<RendererStats>,
    pub(crate) pipeline_cache: FxHashMap<FullPipelineKey, wgpu::RenderPipeline>,
    pub(crate) uber_pipeline_cache: FxHashMap<UberPipelineKey, wgpu::RenderPipeline>,
    pub(crate) shader_cache: FxHashMap<ShaderKey, wgpu::ShaderModule>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) pipeline_compiler: pipeline::PipelineCompiler,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) pending_pipeline_keys: FxHashSet<FullPipelineKey>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) pipeline_generation: u64,
    pub(crate) pipeline_layout: wgpu::PipelineLayout,
    pub(crate) surface_format: wgpu::TextureFormat,
    pub(crate) bind_group_layout: wgpu::BindGroupLayout,
    pub(crate) draw_buffer: wgpu::Buffer,
    pub(crate) draw_buffer_layout: DrawBufferLayout,
    pub(crate) uniform_alignment: u64,
    pub(crate) draw_uniform_stride: u64,
    pub(crate) draw_uniform_capacity: usize,
    pub(crate) scratch_indices: Vec<u32>,
    pub(crate) efb_scale: u32,
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
    // Retired LoadTexture allocations grouped by (w, h).
    pub(crate) texture_pool: FxHashMap<(u32, u32), Vec<wgpu::Texture>>,
    pub(crate) efb_copy_cache: FxHashMap<Address, EfbCopyEntry>,
    pub(crate) efb_copy_pool: FxHashMap<(u32, u32), Vec<(wgpu::Texture, wgpu::TextureView)>>,
    pub(crate) efb_pack_pipelines: render::EfbPackPipelines,
    pub(crate) efb_depth_resolve_bg_layout: wgpu::BindGroupLayout,
    pub(crate) efb_depth_resolve_uniform_buffer: wgpu::Buffer,

    pub(crate) efb_depth_writeback_pipeline: wgpu::RenderPipeline,
    /// Reusable Rgba8Unorm intermediate the Z24 pack writes into before
    /// being copied to staging for the deferred RAM writeback.
    pub(crate) efb_depth_writeback_target: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// Reusable native-resolution intermediate the scaled EFB is downsampled
    /// into before color RAM readbacks when `efb_scale > 1`.
    pub(crate) efb_color_readback_target: Option<(wgpu::Texture, wgpu::TextureView)>,
    pub(crate) fallback_view: wgpu::TextureView,
    pub(crate) scratch_vertices: Vec<GpuVertex>,
    pub(crate) scratch_draws: Vec<DrawRecord>,
    pub(crate) scratch_uniform_bytes: Vec<u8>,
    pub(crate) bind_group_cache: FxHashMap<BindGroupCacheKey, wgpu::BindGroup>,
    // Per-frame draw accumulation (persists across process_action calls,
    // flushed by flush_pending_draws).
    pub(crate) frame_uniform_bytes: Vec<u8>,
    pub(crate) draw_pipeline_keys: Vec<FullPipelineKey>,
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
    pub(crate) xfb_copy_uniform_buffer: wgpu::Buffer,
    pub(crate) xfb_copy_bind_group: wgpu::BindGroup,
    pub(crate) efb_color_readback_uniform_buffer: wgpu::Buffer,
    pub(crate) efb_color_readback_bind_group: wgpu::BindGroup,
    // Region-scoped EFB clear.
    pub(crate) efb_clear: clear::EfbClear,
    pub(crate) pending_command_buffers: Vec<wgpu::CommandBuffer>,
    /// Shared staging buffer for `LoadTexture` uploads. Each upload appends
    /// padded RGBA into `texture_staging_scratch` and records a
    /// `copy_buffer_to_texture` into the persistent encoder; at submit time
    /// the whole scratch is shipped through one `write_buffer_with` call.
    pub(crate) texture_staging_buffer: wgpu::Buffer,
    pub(crate) texture_staging_capacity: u64,
    pub(crate) texture_staging_scratch: Vec<u8>,
    /// Largest scratch size observed within the current run. Used at submit
    /// boundaries to decide whether to grow [`Self::texture_staging_buffer`]
    /// for the next frame (so growth never invalidates a buffer the
    /// in-flight encoder still references).
    pub(crate) texture_staging_peak: u64,
    /// Persistent encoder accumulating GPU commands across operations within
    /// a frame.
    pub(crate) current_encoder: Option<wgpu::CommandEncoder>,
    pub(crate) draw_bufs_write_pending: bool,
    pub(crate) xfb_copy_uniform_write_pending: bool,
    pub(crate) efb_color_readback_uniform_write_pending: bool,
    pub(crate) efb_clear_uniform_write_pending: bool,
    pub(crate) efb_depth_resolve_uniform_write_pending: bool,
    pub(crate) efb_readback_staging_pool: FxHashMap<u64, Vec<wgpu::Buffer>>,
    pub(crate) pending_writebacks: Vec<PendingWriteback>,
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct DrawRecord {
    /// Index into [`GxRenderer::scratch_vertices`] (Vec<DrawVertex>) where
    /// this draw's source vertices start. Used at upload time to pack each
    /// vertex into the destination stride.
    pub src_vertex_index: u32,
    pub vertex_count: u32,
    pub first_index: u32,
    pub index_count: u32,
    /// Byte offset of this draw's vertices inside the packed vertex
    /// section of `draw_buffer` (relative to section start, not buffer
    /// start). Used to compute the buffer slice for `set_vertex_buffer`.
    pub packed_vertex_byte_offset: u32,
    /// Bytes per vertex in the packed stream
    /// (`68 + 12 * active_texcoords`).
    pub packed_vertex_stride: u32,
}

pub(crate) struct PendingWriteback {
    pub dest_addr: Address,
    pub staging: wgpu::Buffer,
    pub staging_capacity: u64,
    pub bytes_per_row: u64,
    pub staging_size: u64,
    pub width: u32,
    pub height: u32,
    pub copy_format: gecko::flipper::gx::texture::CopyFormat,
    pub stride: u32,
    pub swap_bgra: bool,
    pub box_filter_downsample: bool,
}

impl GxRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        efb_scale: u32,
    ) -> Self {
        let max_scale = (device.limits().max_texture_dimension_2d / EFB_WIDTH).max(1);
        let requested_scale = efb_scale;
        let efb_scale = efb_scale.clamp(1, MAX_EFB_SCALE).min(max_scale);
        if efb_scale != requested_scale {
            tracing::warn!(requested = requested_scale, effective = efb_scale, "EFB scale clamped");
        }

        let efb_w = EFB_WIDTH * efb_scale;
        let efb_h = EFB_HEIGHT * efb_scale;

        let frame_uniform_size = FRAME_UNIFORMS_SIZE.get();
        let draw_uniform_size = DRAW_UNIFORMS_SIZE.get();
        let draw_uniform_stride = align_up(
            draw_uniform_size,
            device.limits().min_uniform_buffer_offset_alignment as u64,
        );

        let xfb_copy_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xfb_copy_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/xfb_copy.wgsl").into()),
        });
        let efb_depth_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("efb_depth_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/efb_depth.wgsl").into()),
        });
        let efb_pack_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("efb_pack_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/efb_pack.wgsl").into()),
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
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
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
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let uniform_alignment = device.limits().min_uniform_buffer_offset_alignment as u64;

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
        let initial_index_capacity = 4096;
        let initial_frame_slot_count = 8u64;
        let initial_draw_slot_count = 256u64;
        let draw_buffer_layout = compute_draw_buffer_layout(
            uniform_alignment,
            frame_uniform_size * initial_frame_slot_count,
            draw_uniform_stride * initial_draw_slot_count,
            (initial_capacity * std::mem::size_of::<GpuVertex>()) as u64,
            (initial_index_capacity * std::mem::size_of::<u32>()) as u64,
        );
        let draw_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx_draw_buffer"),
            size: draw_buffer_layout.total_size,
            usage: wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::UNIFORM
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::INDEX,
            mapped_at_creation: false,
        });

        let initial_texture_staging_capacity: u64 = 4 * 1024 * 1024;
        let texture_staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx_texture_staging"),
            size: initial_texture_staging_capacity,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        // Create fixed-size EFB with 4x MSAA.
        // Resolved (1x) color: CopyXfb reads from here.
        let efb_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("efb_color_resolved"),
            size: wgpu::Extent3d {
                width: efb_w,
                height: efb_h,
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
                width: efb_w,
                height: efb_h,
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
                width: efb_w,
                height: efb_h,
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
                width: efb_w,
                height: efb_h,
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
            bind_group_layouts: &[Some(&xfb_copy_bind_group_layout)],
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

        let efb_color_readback_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("efb_color_readback_uniforms"),
            size: std::mem::size_of::<render::XfbCopyUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let make_efb_sample_bg = |label: &str, buffer: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &xfb_copy_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&efb_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&xfb_copy_sampler),
                    },
                ],
            })
        };
        let xfb_copy_bind_group = make_efb_sample_bg("xfb_copy_bg", &xfb_copy_uniform_buffer);
        let efb_color_readback_bind_group =
            make_efb_sample_bg("efb_color_readback_bg", &efb_color_readback_uniform_buffer);

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
            label: Some("efb_depth_pack_layout"),
            bind_group_layouts: &[Some(&efb_depth_resolve_bg_layout)],
            immediate_size: 0,
        });
        let efb_depth_resolve_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("efb_depth_resolve_uniforms"),
            size: std::mem::size_of::<render::XfbCopyUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let make_pack_pipeline =
            |label: &str, entry: &str, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(layout),
                    vertex: wgpu::VertexState {
                        module: shader,
                        entry_point: Some("vs_main"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: shader,
                        entry_point: Some(entry),
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
                })
            };
        let color_pack =
            |label: &str, entry: &str| make_pack_pipeline(label, entry, &xfb_copy_pipeline_layout, &efb_pack_shader);
        let efb_pack_pipelines = render::EfbPackPipelines {
            rgba8: color_pack("efb_pack_rgba8", "fs_rgba8"),
            rgba8_intensity: color_pack("efb_pack_rgba8_intensity", "fs_rgba8_intensity"),
            i8: color_pack("efb_pack_i8", "fs_i8"),
            i4: color_pack("efb_pack_i4", "fs_i4"),
            ia8: color_pack("efb_pack_ia8", "fs_ia8"),
            ia4: color_pack("efb_pack_ia4", "fs_ia4"),
            rgb565: color_pack("efb_pack_rgb565", "fs_rgb565"),
            rgb565_intensity: color_pack("efb_pack_rgb565_intensity", "fs_rgb565_intensity"),
            rgb5a3: color_pack("efb_pack_rgb5a3", "fs_rgb5a3"),
            rgb5a3_intensity: color_pack("efb_pack_rgb5a3_intensity", "fs_rgb5a3_intensity"),
            a8: color_pack("efb_pack_a8", "fs_a8"),
            r8: color_pack("efb_pack_r8", "fs_r8"),
            rg8: color_pack("efb_pack_rg8", "fs_rg8"),
        };
        let efb_depth_writeback_pipeline = make_pack_pipeline(
            "efb_depth_writeback_pipeline",
            "fs_writeback_z24",
            &efb_depth_resolve_pipeline_layout,
            &efb_depth_shader,
        );
        let efb_clear = clear::EfbClear::new(
            device,
            surface_format,
            wgpu::TextureFormat::Depth24Plus,
            EFB_SAMPLE_COUNT,
        );

        let cache_path = std::path::Path::new(shader_specialization::SHADER_CACHE_PATH);
        let cached_keys = shader_specialization::load_cached_keys(cache_path);
        let prewarmed = prewarm_shader_variants(device, &cached_keys);

        let mut shader_cache: FxHashMap<ShaderKey, wgpu::ShaderModule> =
            FxHashMap::with_capacity_and_hasher(prewarmed.len().max(64), Default::default());
        for (k, m) in prewarmed {
            shader_cache.insert(k, m);
        }

        let pipeline_cache: FxHashMap<pipeline::FullPipelineKey, wgpu::RenderPipeline> =
            FxHashMap::with_capacity_and_hasher(64, Default::default());
        let uber_pipeline_cache: FxHashMap<pipeline::UberPipelineKey, wgpu::RenderPipeline> =
            FxHashMap::with_capacity_and_hasher(64, Default::default());
        #[cfg(not(target_arch = "wasm32"))]
        let pipeline_compiler =
            pipeline::PipelineCompiler::new(device.clone(), pipeline_layout.clone(), surface_format);

        GxRenderer {
            #[cfg(feature = "gx-stats")]
            stats: Arc::new(RendererStats::default()),
            pipeline_cache,
            uber_pipeline_cache,
            shader_cache,
            #[cfg(not(target_arch = "wasm32"))]
            pipeline_compiler,
            #[cfg(not(target_arch = "wasm32"))]
            pending_pipeline_keys: FxHashSet::default(),
            #[cfg(not(target_arch = "wasm32"))]
            pipeline_generation: 0,
            pipeline_layout,
            surface_format,
            bind_group_layout,
            draw_buffer,
            draw_buffer_layout,
            uniform_alignment,
            draw_uniform_stride,
            draw_uniform_capacity: initial_draw_slot_count as usize,
            scratch_indices: Vec::new(),
            efb_scale,
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
            texture_pool: FxHashMap::default(),
            efb_copy_cache: FxHashMap::default(),
            efb_copy_pool: FxHashMap::default(),
            efb_pack_pipelines,
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
            xfb_copy_uniform_buffer,
            xfb_copy_bind_group,
            efb_color_readback_uniform_buffer,
            efb_color_readback_bind_group,
            efb_clear,
            pending_command_buffers: Vec::with_capacity(8),
            texture_staging_buffer,
            texture_staging_capacity: initial_texture_staging_capacity,
            texture_staging_scratch: Vec::with_capacity(initial_texture_staging_capacity as usize / 4),
            texture_staging_peak: 0,
            current_encoder: None,
            draw_bufs_write_pending: false,
            xfb_copy_uniform_write_pending: false,
            efb_color_readback_uniform_write_pending: false,
            efb_clear_uniform_write_pending: false,
            efb_depth_resolve_uniform_write_pending: false,

            efb_readback_staging_pool: FxHashMap::default(),
            pending_writebacks: Vec::new(),

            efb_depth_writeback_pipeline,
            efb_depth_writeback_target: None,
            efb_color_readback_target: None,
        }
    }

    /// Take ownership of the persistent frame encoder, or create a fresh
    /// one if there isn't one yet. Caller appends commands and must put it
    /// back via `self.current_encoder = Some(encoder)`.
    pub(crate) fn take_or_create_encoder(&mut self, device: &wgpu::Device) -> wgpu::CommandEncoder {
        self.current_encoder.take().unwrap_or_else(|| {
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gx_frame_encoder"),
            })
        })
    }

    pub fn debug_flush(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.flush_pending_draws(device, queue);
        self.submit_pending(queue);
    }

    pub fn reset_efb(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.flush_pending_draws(device, queue);
        self.submit_pending(queue);

        self.clear_efb_region(
            device,
            queue,
            0,
            0,
            EFB_WIDTH,
            EFB_HEIGHT,
            [0.0, 0.0, 0.0, 0.0],
            1.0,
            true,
            true,
            true,
        );

        self.submit_pending(queue);
    }

    pub fn efb_view(&self) -> &wgpu::TextureView {
        &self.efb_view
    }

    pub(crate) fn scaled(&self, v: u32) -> u32 {
        v * self.efb_scale
    }

    pub(crate) fn scaled_src_rect(&self, x: u32, y: u32, w: u32, h: u32) -> [f32; 4] {
        [
            self.scaled(x) as f32,
            self.scaled(y) as f32,
            self.scaled(w) as f32,
            self.scaled(h) as f32,
        ]
    }

    pub(crate) fn submit_pending(&mut self, queue: &wgpu::Queue) {
        // Ship any staged texture-upload bytes through ONE `write_buffer_with`
        // before finishing the encoder. wgpu orders queue writes ahead of
        // submitted commands, so the encoder's `copy_buffer_to_texture`
        // commands (recorded earlier with offsets into this buffer) will
        // read the freshly-written data.
        if !self.texture_staging_scratch.is_empty()
            && let Some(size) = std::num::NonZeroU64::new(self.texture_staging_scratch.len() as u64)
        {
            let mut view = queue
                .write_buffer_with(&self.texture_staging_buffer, 0, size)
                .expect("texture staging buffer too small");
            view.copy_from_slice(&self.texture_staging_scratch);
            drop(view);
            self.texture_staging_scratch.clear();
        }

        if let Some(encoder) = self.current_encoder.take() {
            self.pending_command_buffers.push(encoder.finish());
        }
        if self.pending_command_buffers.is_empty() {
            return;
        }

        #[cfg(feature = "gx-stats")]
        let command_buffer_count = self.pending_command_buffers.len() as u64;
        #[cfg(feature = "gx-stats")]
        let submit_started = std::time::Instant::now();

        queue.submit(self.pending_command_buffers.drain(..));

        #[cfg(feature = "gx-stats")]
        {
            self.stats.queue_submits.fetch_add(1, Ordering::Relaxed);
            self.stats
                .command_buffers_submitted
                .fetch_add(command_buffer_count, Ordering::Relaxed);
            self.stats
                .queue_submit_cpu_ns
                .fetch_add(submit_started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }

        self.draw_bufs_write_pending = false;
        self.xfb_copy_uniform_write_pending = false;
        self.efb_color_readback_uniform_write_pending = false;
        self.efb_clear_uniform_write_pending = false;
        self.efb_depth_resolve_uniform_write_pending = false;
    }

    /// Append `rgba` (W*H*4 tight, no row padding) into
    /// [`Self::texture_staging_scratch`] with COPY_BYTES_PER_ROW_ALIGNMENT
    /// (256 bytes) row padding, and record a `copy_buffer_to_texture` into the
    /// persistent encoder that copies the staged bytes into `dest`.
    /// Returns `false` if the upload won't fit the current staging capacity.
    /// Caller should fall back to `queue.write_texture` for that upload
    /// and rely on the next submit boundary to grow the buffer via
    /// [`Self::maybe_grow_texture_staging`].
    pub(crate) fn stage_texture_upload(
        &mut self,
        device: &wgpu::Device,
        dest: &wgpu::Texture,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> bool {
        const ROW_ALIGNMENT: u64 = 256;
        const COPY_BUFFER_ALIGNMENT: u64 = 4;
        let row_bytes = (width as u64) * 4;
        let padded_row_bytes = align_up(row_bytes, ROW_ALIGNMENT);
        let upload_size = padded_row_bytes * height as u64;
        let offset = align_up(self.texture_staging_scratch.len() as u64, COPY_BUFFER_ALIGNMENT);
        let end = offset + upload_size;
        self.texture_staging_peak = self.texture_staging_peak.max(end);
        if end > self.texture_staging_capacity {
            return false;
        }

        // Pad to aligned start offset, then append each row + per-row padding.
        self.texture_staging_scratch.resize(offset as usize, 0);
        let pad = (padded_row_bytes - row_bytes) as usize;
        for row in 0..height as usize {
            let src_start = row * row_bytes as usize;
            let src_end = src_start + row_bytes as usize;
            self.texture_staging_scratch
                .extend_from_slice(&rgba[src_start..src_end]);
            if pad > 0 {
                let new_len = self.texture_staging_scratch.len() + pad;
                self.texture_staging_scratch.resize(new_len, 0);
            }
        }

        let mut encoder = self.take_or_create_encoder(device);
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &self.texture_staging_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset,
                    bytes_per_row: Some(padded_row_bytes as u32),
                    rows_per_image: None,
                },
            },
            dest.as_image_copy(),
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.current_encoder = Some(encoder);
        true
    }

    /// If the per-frame staging peak has exceeded the buffer's capacity,
    /// reallocate (next power of two of peak). Safe to call only when no
    /// encoder commands reference the existing buffer.
    pub(crate) fn maybe_grow_texture_staging(&mut self, device: &wgpu::Device) {
        if self.texture_staging_peak <= self.texture_staging_capacity {
            return;
        }
        let new_cap = self
            .texture_staging_peak
            .next_power_of_two()
            .max(self.texture_staging_peak);
        self.texture_staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx_texture_staging"),
            size: new_cap,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        self.texture_staging_capacity = new_cap;
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn clear_efb_region(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        color: [f32; 4],
        depth: f32,
        color_update: bool,
        alpha_update: bool,
        z_update: bool,
    ) {
        if self.efb_clear_uniform_write_pending {
            self.submit_pending(queue);
        }

        let mut encoder = self.take_or_create_encoder(device);
        let did_clear = self.efb_clear.clear_region_masked(
            queue,
            &mut encoder,
            &self.efb_msaa_view,
            &self.efb_view,
            &self.efb_depth_view,
            self.scaled(EFB_WIDTH),
            self.scaled(EFB_HEIGHT),
            self.scaled(x),
            self.scaled(y),
            self.scaled(w),
            self.scaled(h),
            color,
            depth,
            color_update,
            alpha_update,
            z_update,
        );
        self.current_encoder = Some(encoder);
        if did_clear {
            self.efb_clear_uniform_write_pending = true;
        }
    }

    pub fn save_shader_cache(&self) -> std::io::Result<usize> {
        let path = std::path::Path::new(shader_specialization::SHADER_CACHE_PATH);
        let keys: Vec<ShaderKey> = self.shader_cache.keys().copied().collect();
        shader_specialization::save_keys(path, &keys)?;
        Ok(keys.len())
    }

    pub fn save_pipeline_cache(&self) -> std::io::Result<usize> {
        let path = std::path::Path::new(pipeline::PIPELINE_CACHE_PATH);
        let keys: Vec<pipeline::FullPipelineKey> = self.pipeline_cache.keys().copied().collect();
        pipeline::save_pipeline_keys(path, &keys)?;
        Ok(keys.len())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn poll_compiled_pipelines(&mut self) {
        while let Some(result) = self.pipeline_compiler.try_recv() {
            if result.generation != self.pipeline_generation {
                continue;
            }
            self.pending_pipeline_keys.remove(&result.key);
            self.pipeline_cache.insert(result.key, result.pipeline);
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn poll_compiled_pipelines(&mut self) {}

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn request_specialized_pipeline(&mut self, key: FullPipelineKey) {
        if self.pipeline_cache.contains_key(&key) || self.pending_pipeline_keys.contains(&key) {
            return;
        }
        let shader = self.shader_cache[&key.shader].clone();
        if self.pipeline_compiler.request(self.pipeline_generation, key, shader) {
            self.pending_pipeline_keys.insert(key);
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn request_specialized_pipeline(&mut self, _key: FullPipelineKey) {}

    pub(crate) fn invalidate_pipeline_caches(&mut self) {
        self.pipeline_cache.clear();
        self.uber_pipeline_cache.clear();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.pipeline_generation = self.pipeline_generation.wrapping_add(1);
            self.pending_pipeline_keys.clear();
        }
    }

    pub fn prewarm_pipeline_cache(&mut self, device: &wgpu::Device) {
        let path = std::path::Path::new(pipeline::PIPELINE_CACHE_PATH);
        let keys: Vec<pipeline::FullPipelineKey> = pipeline::load_cached_pipeline_keys(path)
            .into_iter()
            .filter(|k| !self.pipeline_cache.contains_key(k) && self.shader_cache.contains_key(&k.shader))
            .collect();

        if keys.is_empty() {
            return;
        }

        let t0 = std::time::Instant::now();

        // wgpu 29's WebGPU backend wraps JS handles in Rc<Cell<_>>, making
        // Device/Queue !Send — so the threaded compile path can't even
        // type-check on wasm32. Fall back to a sequential pass there.
        #[cfg(not(target_arch = "wasm32"))]
        let compiled: Vec<(pipeline::FullPipelineKey, wgpu::RenderPipeline)> = {
            let num_threads = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(keys.len());
            let chunk_size = keys.len().div_ceil(num_threads);

            let self_ref = &*self;
            std::thread::scope(|s| {
                let handles: Vec<_> = keys
                    .chunks(chunk_size)
                    .map(|chunk| {
                        s.spawn(move || {
                            chunk
                                .iter()
                                .map(|&k| {
                                    let module = &self_ref.shader_cache[&k.shader];
                                    (
                                        k,
                                        pipeline::create_specialized_pipeline(
                                            device,
                                            &self_ref.pipeline_layout,
                                            self_ref.surface_format,
                                            module,
                                            &k,
                                        ),
                                    )
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
            })
        };

        #[cfg(target_arch = "wasm32")]
        let compiled: Vec<(pipeline::FullPipelineKey, wgpu::RenderPipeline)> = keys
            .iter()
            .map(|&k| {
                let module = &self.shader_cache[&k.shader];
                (
                    k,
                    pipeline::create_specialized_pipeline(
                        device,
                        &self.pipeline_layout,
                        self.surface_format,
                        module,
                        &k,
                    ),
                )
            })
            .collect();

        for (k, p) in compiled {
            self.pipeline_cache.insert(k, p);
        }

        tracing::info!(
            num_pipelines = keys.len(),
            elapsed_ms = t0.elapsed().as_millis() as u64,
            "prewarmed pipeline cache",
        );
    }
}

fn prewarm_shader_variants(device: &wgpu::Device, keys: &[ShaderKey]) -> Vec<(ShaderKey, wgpu::ShaderModule)> {
    if keys.is_empty() {
        return Vec::new();
    }

    let t0 = std::time::Instant::now();
    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(keys.len());

    let chunk_size = keys.len().div_ceil(num_threads);
    let compiled: Vec<(ShaderKey, String)> = std::thread::scope(|s| {
        let handles: Vec<_> = keys
            .chunks(chunk_size)
            .map(|chunk| {
                s.spawn(move || {
                    chunk
                        .iter()
                        .map(|&k| (k, shader_specialization::compile_variant(k)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });

    let modules: Vec<(ShaderKey, wgpu::ShaderModule)> = compiled
        .into_iter()
        .map(|(key, wgsl)| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&format!("gx_shader_{key:?}")),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            });
            (key, module)
        })
        .collect();

    tracing::info!(
        num_variants = modules.len(),
        elapsed_ms = t0.elapsed().as_millis() as u64,
        "prewarmed shader variants from cache"
    );

    modules
}
