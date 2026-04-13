use crate::common::Address;
use crate::flipper::gx::draw::{Primitive, Scissor, Viewport};
use crate::flipper::gx::regs::{AlphaCompare, BlendMode, ChanCtrl, CullMode, MagFilter, MinFilter, WrapMode, ZMode};

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
        id: Address,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },

    /// Bind a previously loaded texture to a TEV texture slot.
    SetTexture {
        slot: usize,
        id: Address,
        wrap_s: WrapMode,
        wrap_t: WrapMode,
        mag_filter: MagFilter,
        min_filter: MinFilter,
    },

    /// Issue a draw call. The renderer uses its tracked state (projection,
    /// viewport, scissor, depth, blend, alpha, textures) plus the per-draw
    /// TEV/lighting snapshot carried here.
    Draw(DrawData),

    /// Copy the EFB source region to a temporary texture identified by `id`.
    /// The renderer stores this until the next [`PresentXfb`] composites it
    /// into the output framebuffer.
    CopyXfb {
        id: Address,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        clear: bool,
        clear_color: [f32; 4],
        clear_z: f32,
    },

    /// Composite all XFB copies from this frame into the output framebuffer.
    /// Emitted once per vblank by `present_xfb()`. Each [`XfbPart`]
    /// identifies a copy by `id` and places it at `(offset_x, offset_y)`.
    PresentXfb {
        width: u32,
        height: u32,
        parts: Vec<XfbPart>,
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

/// Per-draw data: primitive type, decoded vertices, modelview transform,
/// and TEV/lighting configuration (snapshotted at draw time since TEV is
/// built up incrementally via BP writes).
#[derive(Debug)]
pub struct DrawData {
    pub primitive: Primitive,
    pub vertices: Vec<DrawVertex>,
    pub modelview: [[f32; 4]; 4],
    // TEV combiner state
    pub tev_color_env: Vec<u32>,
    pub tev_alpha_env: Vec<u32>,
    pub tev_orders: Vec<u32>,
    pub tev_color_regs: [[f32; 4]; 4],
    pub tev_konst_colors: [[f32; 4]; 16],
    pub num_tev_stages: u8,
    // Lighting state (2 channels: COLOR0/ALPHA0 and COLOR1/ALPHA1)
    pub color_ctrl: [ChanCtrl; 2],
    pub alpha_ctrl: [ChanCtrl; 2],
    pub ambient_color: [[f32; 4]; 2],
    pub material_color: [[f32; 4]; 2],
    pub lights: [LightData; 8],
}

/// Per-vertex data after decode, ready for the renderer.
#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct DrawVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color0: [f32; 4],
    pub color1: [f32; 4],
    pub pos_view: [f32; 3],
    pub texcoords: [[f32; 2]; 8],
}

/// Per-light snapshot for the draw call.
#[derive(Debug, Clone, Default)]
pub struct LightData {
    pub color: [f32; 4],
    pub cosatt: [f32; 4],
    pub distatt: [f32; 4],
    pub position: [f32; 4],
    pub direction: [f32; 4],
}

/// One-way sink for GX actions. The emulator pushes actions here; the
/// renderer consumes them (typically on a separate thread).
pub trait RenderSink: Send {
    /// Submit a single action. Implementations should not block unless
    /// back-pressure from the renderer requires it.
    fn exec(&mut self, action: GxAction);
}

/// Swallows every action. Used by headless runners (tinybench, tinytracer)
/// and as the default when no renderer is installed.
#[derive(Debug, Clone, Copy)]
pub struct EmptyRenderSink;

impl RenderSink for EmptyRenderSink {
    fn exec(&mut self, _: GxAction) {}
}
