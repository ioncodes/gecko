use crate::common::Address;
use crate::flipper::gx::draw::{Primitive, Scissor, TextureFormat, Viewport};
use crate::flipper::gx::regs::{AlphaCompare, BlendMode, ChanCtrl, CullMode, MagFilter, MinFilter, WrapMode, ZMode};
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

/// Renderer-side texture cache key. Identifies one unique decoded RGBA
/// texture: same `ram_addr` sampled through different palettes must occupy
/// distinct cache slots, otherwise alternating palette uploads (FFCC fade /
/// settle animations swap palette content at a fixed `tmem_offset` each
/// frame) silently overwrite each other and bind groups built lazily at
/// render-pass time all resolve to whichever decode landed last.
///
/// `variant` is `0` for non-paletted formats and a 32-bit hash of
/// `(palette content, tlut.format, tmem_offset)` for paletted ones.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TextureKey {
    pub ram_addr: Address,
    pub variant: u32,
}

impl TextureKey {
    pub const fn non_paletted(ram_addr: Address) -> Self {
        Self { ram_addr, variant: 0 }
    }
}

#[derive(Debug)]
pub enum GxAction {
    // XF
    SetProjection {
        matrix: [[f32; 4]; 4],
        is_perspective: bool,
    },
    SetViewport(Viewport),

    // BP
    SetScissor(Scissor),
    SetDepthMode(ZMode),
    SetBlendMode(BlendMode),
    SetAlphaCompare(AlphaCompare),
    SetCullMode(CullMode),

    /// Upload pre-decoded texture data. Emitted when texture content at a
    /// given address changes (detected by hash).
    LoadTexture {
        id: TextureKey,
        width: u32,
        height: u32,
        fmt: TextureFormat,
        rgba: Vec<u8>,
    },

    /// Debug action: Drop every cached pipeline, bind group, and texture on
    /// the renderer side. Used by the GX debug window to force fresh decodes.
    InvalidateCaches,

    /// Debug action: Dump every currently cached texture to `dir` as a PNG,
    /// filename including the GX format. Native only.
    #[cfg(not(target_arch = "wasm32"))]
    DumpTextures {
        dir: PathBuf,
    },

    /// Bind a previously loaded texture to a TEV texture slot. `id` matches
    /// the cache key used in [`Self::LoadTexture`].
    SetTexture {
        slot: usize,
        id: TextureKey,
        wrap_s: WrapMode,
        wrap_t: WrapMode,
        mag_filter: MagFilter,
        min_filter: MinFilter,
    },

    /// Issue a draw call. The renderer uses its tracked state (projection,
    /// viewport, scissor, depth, blend, alpha, textures) plus the per-draw
    /// TEV/lighting snapshot carried here.
    Draw(Box<DrawData>),

    /// Copy the EFB source region to a temporary texture identified by `id`.
    /// The renderer stores this until the next [`PresentXfb`] composites it
    /// into the output framebuffer.
    CopyXfb {
        id: Address,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        dst_h: u32,
        gamma: f32,
        clear: bool,
        clear_color: [f32; 4],
        clear_z: f32,
        color_update: bool,
        alpha_update: bool,
        z_update: bool,
        alpha_supported: bool,
    },

    /// Composite all XFB copies from this frame into the output framebuffer.
    /// Emitted once per vblank by `present_xfb()`. Each [`XfbPart`]
    /// identifies a copy by `id` and places it at `(offset_x, offset_y)`.
    PresentXfb {
        width: u32,
        height: u32,
        parts: Vec<XfbPart>,
    },

    /// Copy an EFB region back into system RAM, encoded in a GX texture
    /// format. The renderer does a GPU readback, converts the pixels to
    /// `copy_format`, and ships the encoded bytes back over the writeback
    /// channel. The emu side spits them into `Mmio::ram` synchronously so
    /// subsequent texture loads see fresh data.
    ///
    /// Per Dolphin (`BPFunctions::ClearScreen`), the `clear` bit on BP 0x52
    /// only affects channels whose write mask is enabled. We carry the
    /// current `color_update`/`alpha_update`/`z_update` so the backend can
    /// gate the post-copy clear correctly; when color writes are off, the
    /// clear must be a no-op for color?
    CopyEfbToTexture {
        dest_addr: Address,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        copy_format: u8,
        mipmap: bool,
        stride: u32,
        clear: bool,
        clear_color: [f32; 4],
        clear_z: f32,
        color_update: bool,
        alpha_update: bool,
        z_update: bool,
        alpha_supported: bool,
        depth_copy: bool,
        is_intensity: bool,
    },
}

/// Identifies one tile in a composited XFB frame. The `id` matches the
/// `CopyXfb::id` that produced the source texture; `offset_x`/`offset_y`
/// are the pixel coordinates in the output framebuffer.
#[derive(Debug, Clone, Copy)]
pub struct XfbPart {
    pub id: Address,
    pub offset_x: u32,
    pub offset_y: u32,
}

/// Per-draw data: primitive type, vertex range, modelview transform,
/// and TEV/lighting configuration (snapshotted at draw time since TEV is
/// built up incrementally via BP writes). Vertices live in the renderer's
/// scratch buffer (see [`RenderSink::vertex_scratch`]); `base_vertex` is
/// the index into that buffer where this draw's vertices start and
/// `vertex_count` is how many of them belong to this draw.
#[derive(Debug, Default)]
pub struct DrawData {
    pub primitive: Primitive,
    pub base_vertex: u32,
    pub vertex_count: u32,
    pub active_texcoords: u8,
    pub modelview: [[f32; 4]; 4],
    // TEV combiner state
    pub tev_color_env: [u32; 16],
    pub tev_alpha_env: [u32; 16],
    pub tev_orders: [u32; 16],
    pub tev_ksel: [u32; 8],
    pub tev_color_regs: [[f32; 4]; 4],
    pub tev_konst_colors: [[f32; 4]; 16],
    pub num_tev_stages: u8,
    // Indirect texturing state. `indirect_matrices` is 6 rows (2 per
    // matrix, matrix N at rows 2*N and 2*N+1) with .xyz holding the
    // 11-bit signed elements and .w holding `17 - scale_exponent`.
    // `tev_indirect` holds the raw IND_CMD per TEV stage (16 entries).
    pub indirect_matrices: [[i32; 4]; 6],
    pub indirect_scales: [[u32; 4]; 2],
    pub indirect_refs: u32,
    pub num_indirect_stages: u8,
    pub bump_imask: u32,
    pub tev_indirect: [u32; 16],
    // Lighting state (2 channels: COLOR0/ALPHA0 and COLOR1/ALPHA1)
    pub color_ctrl: [ChanCtrl; 2],
    pub alpha_ctrl: [ChanCtrl; 2],
    pub ambient_color: [[f32; 4]; 2],
    pub material_color: [[f32; 4]; 2],
    pub lights: [LightData; 8],
    pub frame_dirty: bool,
}

/// Per-vertex data after decode, ready for the renderer. Field order
/// matches `backend_wgpu::GpuVertex` and the wgpu vertex attribute layout
/// in `backend_wgpu::pipeline`, so a slice of `DrawVertex` can be uploaded
/// directly via `bytemuck::cast_slice` without any field-shuffle copy.
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct DrawVertex {
    pub position: [f32; 3],
    pub color0: [f32; 4],
    pub color1: [f32; 4],
    pub normal: [f32; 3],
    pub pos_view: [f32; 3],
    pub texcoords: [[f32; 3]; 8],
}

/// Per-light snapshot for the draw call.
#[derive(Debug, Clone, Copy, Default)]
pub struct LightData {
    pub color: [f32; 4],
    pub cosatt: [f32; 4],
    pub distatt: [f32; 4],
    pub position: [f32; 4],
    pub direction: [f32; 4],
}

/// One-way sink for GX actions. The emulator pushes actions here.
pub trait RenderSink: Send {
    /// Submit a single action.
    fn exec(&mut self, action: GxAction);

    /// Mutable handle to the sink's vertex scratch buffer. Callers append
    /// per-draw vertices here before issuing [`GxAction::Draw`] and store the
    /// pre-append length as [`DrawData::base_vertex`]. Real renderers
    /// (e.g. wgpu) keep this buffer alive across draws and upload it in one
    /// shot at flush time; headless sinks can use a throwaway local.
    fn vertex_scratch(&mut self) -> &mut Vec<DrawVertex>;

    fn flush_efb_copies(&mut self, ram: &mut crate::mmio::RamViewMut<'_>) {
        let _ = ram;
    }

    /// Acquire a `DrawData` box for the next draw call. The default impl
    /// allocates fresh. Real renderers override to recycle boxes that come
    /// back through [`Self::exec`] as `GxAction::Draw(box)`. The caller
    /// overwrites every field before issuing the action, so pool entries
    /// don't need to be reset.
    fn take_draw_data(&mut self) -> Box<DrawData> {
        Box::default()
    }
}

/// Swallows every action. Used by headless runners (tinybench, tinytracer)
/// and as the default when no renderer is installed.
#[derive(Debug, Default)]
pub struct EmptyRenderSink {
    scratch: Vec<DrawVertex>,
}

impl RenderSink for EmptyRenderSink {
    fn exec(&mut self, _action: GxAction) {
        #[cfg(feature = "rendersink-blackbox")]
        std::hint::black_box(&_action);
        self.scratch.clear();
    }

    fn vertex_scratch(&mut self) -> &mut Vec<DrawVertex> {
        &mut self.scratch
    }
}
