mod bp;
pub mod constants;
pub mod draw;
pub mod fifo;
#[cfg(feature = "jit")]
pub mod jit;
pub mod math;
pub mod recorder;
pub mod regs;
pub mod tev;
mod texgen;
pub mod texture;
mod vertex;
mod xf;

use crate::flipper::gx::constants::{BP_REG_SIZE, CP_REG_SIZE, TLUT_MEM_ENTRIES, XF_MEM_SIZE};
use crate::flipper::gx::draw::Matrix4;
use crate::flipper::gx::regs::{
    AlphaCompare, BlendMode, ChanCtrl, TevAlphaEnv, TevColorEnv, TevRegisterH, TevRegisterL, ZMode,
};
use crate::host::{GxAction, LightData, TextureKey, XfbPart};
use crate::system::{ExecutionMode, System, SystemId};
use rustc_hash::FxHashMap;

pub struct GraphicsProcessor {
    pub raise_interrupt: bool,
    pub raise_token_interrupt: bool,
    pub pending_token: u16,
    pub token_dirty: bool,
    pub projection: Matrix4,
    pub bp_regs: Vec<u32>,
    pub bp_mask: u32,
    pub cp_regs: Vec<u32>,
    pub xf_mem: Vec<u32>,
    pub fifo: Vec<u8>,
    pub dl_scratch: Vec<u8>,

    // FIFO recording stuff
    pub recorder: Option<Box<recorder::FifoRecorder>>,

    // Current GX state to snapshot into a Draw action later
    pub cur_textures: [Option<draw::TextureDescriptor>; 8],
    // Bitmask of slots whose TX_SETMODE0/SETIMAGE0-3/SETTLUT regs changed
    // since the last snapshot. Games write these regs in arbitrary order
    // (SMG's J3D binds SETIMAGE3 before SETIMAGE0), so the descriptor is
    // only consistent at draw time; `snapshot_dirty_textures` resolves them
    // right before each draw call.
    pub tex_dirty: u8,
    // Per-texture-slot TLUT binding (tmem offset + palette pixel format),
    // populated by BP_TX_SETTLUT writes.
    pub cur_tluts: [draw::TlutRef; 8],
    // Palette TMEM: backing store for indexed texture palettes. Addressed as
    // u16 entries; a LOADTLUT copies count*16 entries starting at
    // (tmem_offset * 256). Fixed-size so indexing is branch-free.
    pub palette_mem: Vec<u16>,
    pub cur_tev_color_env: [TevColorEnv; 16],
    pub cur_tev_alpha_env: [TevAlphaEnv; 16],
    pub cur_tev_color_regs_lo: [TevRegisterL; 4],
    pub cur_tev_color_regs_hi: [TevRegisterH; 4],
    pub cur_tev_const_regs_lo: [TevRegisterL; 4],
    pub cur_tev_const_regs_hi: [TevRegisterH; 4],
    pub cur_tev_orders: [regs::TevOrder; 8],
    pub cur_num_tev_stages: u8,
    pub cur_tev_konst_colors: [[f32; 4]; 16],
    // Indirect texturing state. Brain damage.
    pub cur_indirect_matrices: [regs::IndMtx; 3],
    pub cur_indirect_scales: [regs::Ras1Ss; 2],
    pub cur_indirect_refs: regs::Ras1IRef,
    pub cur_tev_indirect: [regs::TevIndirect; 16],
    pub cur_num_indirect_stages: u8,
    pub cur_bump_imask: u32,
    pub cur_zmode: ZMode,
    pub cur_pe_control: regs::PeControl,
    pub cur_blend_mode: BlendMode,
    pub cur_alpha_compare: AlphaCompare,
    pub cur_viewport: draw::Viewport,
    pub cur_scissor: draw::Scissor,
    // BP_SU_SCIS_OFFSET: applied to both the scissor rect and the viewport
    // origin. Games use this to do tiled rendering without changing their
    // projection or logical viewport.
    pub cur_scissor_offset_x: i32,
    pub cur_scissor_offset_y: i32,

    // Every XFB address the game has recently copied to. `present_xfb()`
    // composes the regions that overlap the buffer the VI is scanning.
    pub xfb_regions: FxHashMap<u32, XfbRegion>,
    pub xfb_dirty: bool,
    pub xfb_copy_seq: u64,
    pub xfb_present_seq: u64,
    pub xfb_last_present_base: u32,

    #[cfg(feature = "jit")]
    pub jit_vtx: jit::JitVertexEngine,
    #[cfg(feature = "jit")]
    pub jit_vtx_arrays: jit::ResolvedArrays,
    #[cfg(feature = "vtx-jit-validate")]
    pub jit_vtx_validator: jit::validate::VertexJitValidator,
    pub lighting_dirty: bool,
    pub konst_dirty: bool,
    pub frame_state_dirty: bool,
    pub cached_color_ctrl: [ChanCtrl; 2],
    pub cached_alpha_ctrl: [ChanCtrl; 2],
    pub cached_ambient_color: [[f32; 4]; 2],
    pub cached_material_color: [[f32; 4]; 2],
    pub cached_lights: [LightData; 8],
    #[cfg(feature = "gx-stats")]
    pub(crate) stats: GxStats,
    // Hash of the raw texture data at each cache key; used to detect when
    // texture content changes and avoid redundant decodes + LoadTexture
    // sends. Keyed by the same `TextureKey` sent to the renderer in
    // [`GxAction::LoadTexture`].
    pub texture_hashes: FxHashMap<TextureKey, u64>,
    pub execution_mode: ExecutionMode,
}

/// One known EFB-to-XFB copy destination. Compositing in `first_seq` order
/// keeps a split XFB's bottom copy over the top copy's junk padding rows,
/// like Dolphin. `seen_present_seq` ages out dead regions. As seen in
/// Another Code: R or whatever it's called, it builds each frame from a
/// 230+228 line copy pair and showed a black seam plus a lagging bottom half.
pub struct XfbRegion {
    pub stride: u32,
    pub first_seq: u64,
    pub copy_seq: u64,
    pub seen_present_seq: u64,
}

// Drop regions that haven't been re-copied for a few frames so dead layouts
// don't composite over fresh ones.
const XFB_REGION_MAX_AGE_PRESENTS: u64 = 8;

#[cfg(feature = "gx-stats")]
#[derive(Default, Clone)]
pub(crate) struct GxStats {
    pub draw_calls: u64,
    pub vertices: u64,
    pub fifo_bytes: u64,
    pub create_draw_call_ns: u64,
    pub draws_by_primitive: [u64; 8],
    pub texture_loads: u64,
    pub xfb_presents: u64,
    pub bp_writes: u64,
    pub xf_writes: u64,
}

impl GraphicsProcessor {
    pub fn new() -> Self {
        GraphicsProcessor {
            raise_interrupt: false,
            raise_token_interrupt: false,
            pending_token: 0,
            token_dirty: false,
            bp_regs: vec![0; BP_REG_SIZE],
            bp_mask: 0x00ff_ffff,
            cp_regs: vec![0; CP_REG_SIZE],
            xf_mem: vec![0; XF_MEM_SIZE],
            fifo: Vec::with_capacity(256),
            dl_scratch: Vec::with_capacity(4096),
            recorder: None,
            projection: Matrix4::default(),
            cur_textures: Default::default(),
            tex_dirty: 0,
            cur_tluts: [draw::TlutRef::default(); 8],
            palette_mem: vec![0u16; TLUT_MEM_ENTRIES],
            cur_tev_color_env: Default::default(),
            cur_tev_alpha_env: Default::default(),
            cur_tev_color_regs_lo: Default::default(),
            cur_tev_color_regs_hi: Default::default(),
            cur_tev_const_regs_lo: Default::default(),
            cur_tev_const_regs_hi: Default::default(),
            cur_tev_orders: Default::default(),
            cur_num_tev_stages: 0,
            cur_tev_konst_colors: [[0.0; 4]; 16],
            cur_indirect_matrices: Default::default(),
            cur_indirect_scales: Default::default(),
            cur_indirect_refs: Default::default(),
            cur_tev_indirect: Default::default(),
            cur_num_indirect_stages: 0,
            cur_bump_imask: 0,
            cur_zmode: Default::default(),
            cur_pe_control: Default::default(),
            cur_blend_mode: BlendMode::from_raw(0).with_color_update(true).with_alpha_update(true),
            cur_alpha_compare: Default::default(),
            cur_viewport: Default::default(),
            cur_scissor: Default::default(),
            cur_scissor_offset_x: 0,
            cur_scissor_offset_y: 0,
            xfb_regions: FxHashMap::default(),
            xfb_dirty: false,
            xfb_copy_seq: 0,
            xfb_present_seq: 0,
            xfb_last_present_base: 0,
            #[cfg(feature = "jit")]
            jit_vtx: jit::JitVertexEngine::new(),
            #[cfg(feature = "jit")]
            jit_vtx_arrays: jit::ResolvedArrays::default(),
            #[cfg(feature = "vtx-jit-validate")]
            jit_vtx_validator: jit::validate::VertexJitValidator::new(),
            lighting_dirty: true,
            konst_dirty: true,
            frame_state_dirty: true,
            cached_color_ctrl: [ChanCtrl::default(); 2],
            cached_alpha_ctrl: [ChanCtrl::default(); 2],
            cached_ambient_color: [[0.0; 4]; 2],
            cached_material_color: [[0.0; 4]; 2],
            cached_lights: std::array::from_fn(|_| LightData::default()),
            #[cfg(feature = "gx-stats")]
            stats: GxStats::default(),
            texture_hashes: FxHashMap::default(),
            execution_mode: ExecutionMode::default(),
        }
    }
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn present_xfb<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    sys.vi_present_seen_this_frame = true;
    sys.vsync_pending = true;

    #[cfg(feature = "fps-counter")]
    {
        sys.fps_counter.vsync_count += 1;
    }

    // Present when new copies arrived or the VI flipped to a buffer whose
    // copies were already consumed (pageflip).
    let vi_base = sys.vi.latched_xfb_base;
    if !sys.gx.xfb_dirty && vi_base == sys.gx.xfb_last_present_base {
        return;
    }
    sys.gx.xfb_dirty = false;
    sys.gx.xfb_present_seq += 1;

    if sys.gx.recorder.is_some() {
        let mut rec = sys.gx.recorder.take().unwrap();
        rec.on_frame_boundary(
            &sys.gx,
            sys.cp.fifo_base(),
            sys.cp.fifo_end(),
            SYSTEM == crate::system::WII,
        );
        sys.gx.recorder = Some(rec);
    }

    #[cfg(feature = "gx-stats")]
    {
        sys.gx.stats.xfb_presents += 1;
    }

    let seq = sys.gx.xfb_present_seq;
    sys.gx
        .xfb_regions
        .retain(|_, r| seq - r.seen_present_seq <= XFB_REGION_MAX_AGE_PRESENTS);

    let (frame_w, frame_h) = sys.vi.frame_dimensions();

    let Some(bytes_per_row) = sys
        .gx
        .xfb_regions
        .values()
        .max_by_key(|r| r.copy_seq)
        .map(|r| r.stride as u64)
    else {
        return;
    };
    if bytes_per_row == 0 {
        tracing::warn!("present_xfb: zero bytes_per_row, skipping present");
        return;
    }

    let xfb_bytes = bytes_per_row * frame_h as u64;
    let stride_in_pixels = (bytes_per_row / 2) as u32;

    let frame_base = if sys.vi.dcr.interlaced() && sys.vi.latched_even_field {
        vi_base.saturating_sub(bytes_per_row as u32)
    } else {
        vi_base
    };

    let build_parts = |base_addr: u32| -> Vec<(u64, u64, XfbPart)> {
        let mut parts = Vec::with_capacity(sys.gx.xfb_regions.len());

        for (&addr, region) in sys.gx.xfb_regions.iter() {
            if addr < base_addr {
                continue;
            }

            let delta_bytes = (addr - base_addr) as u64;
            if delta_bytes >= xfb_bytes {
                continue;
            }

            let delta_pixels = (delta_bytes / 2) as u32;
            let offset_x = delta_pixels % stride_in_pixels;
            let offset_y = delta_pixels / stride_in_pixels;

            // Real XFB copies always land at row boundaries (offset_x == 0).
            // A non-zero offset_x means this region belongs to a different
            // buffer that happens to sit nearby in memory, reject it? TODO
            if offset_x != 0 || offset_y >= frame_h as u32 {
                tracing::debug!(
                    region = addr,
                    base = base_addr,
                    offset_x,
                    offset_y,
                    "present_xfb: rejecting XFB region with invalid offset"
                );
                continue;
            }

            parts.push((
                region.first_seq,
                region.copy_seq,
                XfbPart {
                    id: addr,
                    offset_x,
                    offset_y,
                },
            ));
        }

        parts.sort_by_key(|(first_seq, _, _)| *first_seq);
        parts
    };

    let min_base = sys.gx.xfb_regions.keys().min().copied().unwrap_or(0);

    let parts = if frame_base != 0 {
        let mut p = build_parts(frame_base);
        if p.is_empty() {
            p.push((
                0,
                0,
                XfbPart {
                    id: frame_base,
                    offset_x: 0,
                    offset_y: 0,
                },
            ));
        }
        p
    } else {
        build_parts(min_base)
    };

    if parts.is_empty() {
        tracing::debug!("present_xfb: no XFB regions matched the frame buffer region");
        return;
    }

    let closer_copy_seq = parts.last().map(|(_, copy_seq, _)| *copy_seq).unwrap_or(0);
    if parts.iter().any(|(_, copy_seq, _)| *copy_seq > closer_copy_seq) {
        sys.gx.xfb_dirty = true;
        return;
    }

    let parts: Vec<XfbPart> = parts.into_iter().map(|(_, _, p)| p).collect();

    sys.render_sink.exec(GxAction::PresentXfb {
        width: frame_w,
        height: frame_h,
        parts,
    });
    sys.gx.xfb_last_present_base = vi_base;
}

impl<const SYSTEM: SystemId> System<SYSTEM> {
    /// Check if the GX stub detected a finish or token command and signal PE
    pub fn check_gx_pe_interrupts(&mut self) {
        if self.gx.raise_interrupt {
            self.gx.raise_interrupt = false;
            self.pe.signal_finish();
        }

        if self.gx.token_dirty {
            self.gx.token_dirty = false;
            if self.gx.raise_token_interrupt {
                self.gx.raise_token_interrupt = false;
                self.pe.signal_token(self.gx.pending_token);
            } else {
                self.pe.set_token(self.gx.pending_token);
            }
        }

        crate::flipper::pe::refresh_interrupts(self);
    }
}
