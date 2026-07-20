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

    // Page-flip cadence in fields. Frame emission is paced to it so a
    // generation that completes early can't cut the previous frame short.
    pub xfb_last_seen_base: u32,
    pub xfb_prev_base: u32,
    pub xfb_fields_since_flip: u32,
    pub xfb_flip_interval: u32,
    pub xfb_fields_since_emit: u32,
    pub xfb_last_emit_gen: u64,

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
            xfb_last_seen_base: 0,
            xfb_prev_base: 0,
            xfb_fields_since_flip: 0,
            xfb_flip_interval: 1,
            xfb_fields_since_emit: 0,
            xfb_last_emit_gen: 0,
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

    sys.gx.xfb_fields_since_flip = sys.gx.xfb_fields_since_flip.saturating_add(1);
    sys.gx.xfb_fields_since_emit = sys.gx.xfb_fields_since_emit.saturating_add(1);

    let (frame_w, frame_h) = sys.vi.frame_dimensions();

    if sys.gx.xfb_copy_seq == 0 {
        let base = sys.vi.latched_xfb_base;
        self::present_raw_xfb(sys, base, frame_w, frame_h);
        return;
    }

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

    let vi_base = sys.vi.latched_xfb_base;
    let frame_base = if sys.vi.dcr.interlaced() && sys.vi.latched_even_field {
        vi_base.saturating_sub(bytes_per_row as u32)
    } else {
        vi_base
    };

    // Measure the page-flip cadence. Emission is paced to it further down.
    if frame_base != sys.gx.xfb_last_seen_base {
        sys.gx.xfb_prev_base = sys.gx.xfb_last_seen_base;
        sys.gx.xfb_last_seen_base = frame_base;
        sys.gx.xfb_flip_interval = sys.gx.xfb_fields_since_flip.clamp(1, 4);
        sys.gx.xfb_fields_since_flip = 0;
    } else if sys.gx.xfb_fields_since_flip > 8 {
        sys.gx.xfb_flip_interval = 1;
    }

    // Present when new copies arrived or the VI flipped to a buffer whose
    // copies were already consumed (pageflip).
    if !sys.gx.xfb_dirty && frame_base == sys.gx.xfb_last_present_base {
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

    // A split XFB generation often completes while the VI is scanning the
    // other buffer, so consider the window the VI just flipped away from
    // too. New complete generations are shown oldest first, paced to the
    // flip cadence so an early completion can't cut the previous frame
    // short (Another Code: R needs both or it drops to half rate).
    let mut chosen: Option<(u64, Vec<(u64, u64, XfbPart)>)> = None;
    let mut bases = [sys.gx.xfb_prev_base, frame_base];
    if bases[0] == bases[1] {
        bases[0] = 0;
    }

    for base in bases {
        if base == 0 {
            continue;
        }

        let parts = build_parts(base);
        let Some(closer_copy_seq) = parts.last().map(|(_, copy_seq, _)| *copy_seq) else {
            continue;
        };
        if parts.iter().any(|(_, copy_seq, _)| *copy_seq > closer_copy_seq) {
            continue;
        }
        if closer_copy_seq <= sys.gx.xfb_last_emit_gen {
            continue;
        }

        if chosen.as_ref().is_none_or(|(g, _)| closer_copy_seq < *g) {
            chosen = Some((closer_copy_seq, parts));
        }
    }

    if let Some((new_gen, parts)) = chosen {
        if sys.gx.xfb_fields_since_emit < sys.gx.xfb_flip_interval {
            sys.gx.xfb_dirty = true;
            return;
        }

        sys.render_sink.exec(GxAction::PresentXfb {
            width: frame_w,
            height: frame_h,
            parts: parts.into_iter().map(|(_, _, p)| p).collect(),
        });

        sys.gx.xfb_last_emit_gen = new_gen;
        sys.gx.xfb_fields_since_emit = 0;
        sys.gx.xfb_last_present_base = frame_base;
        return;
    }

    // Nothing new completed. Present the scanned window when the VI
    // flipped again, so page flip games still show the buffer it moved to.
    if frame_base == sys.gx.xfb_last_present_base {
        return;
    }

    let min_base = sys.gx.xfb_regions.keys().min().copied().unwrap_or(0);

    let parts = if frame_base != 0 {
        let mut p = build_parts(frame_base);
        if p.is_empty() {
            // The VI can scan from an address that lands *inside* a copy
            // region rather than at its start: Animal Crossing copies the
            // full frame to the buffer base, then points the scan a few rows
            // in.
            let containing = sys
                .gx
                .xfb_regions
                .iter()
                .filter(|&(&addr, _)| addr <= frame_base && (frame_base as u64) < addr as u64 + xfb_bytes)
                .max_by_key(|&(&addr, _)| addr)
                .map(|(&addr, region)| (region.first_seq, region.copy_seq, addr));

            let (first_seq, copy_seq, id) = containing.unwrap_or((0, 0, frame_base));
            p.push((
                first_seq,
                copy_seq,
                XfbPart {
                    id,
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
        return;
    }

    sys.render_sink.exec(GxAction::PresentXfb {
        width: frame_w,
        height: frame_h,
        parts: parts.into_iter().map(|(_, _, p)| p).collect(),
    });
    sys.gx.xfb_last_present_base = frame_base;
}

fn present_raw_xfb<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, base: u32, width: u32, height: u32) {
    if base == 0 || width == 0 || height == 0 {
        return;
    }

    let pixel_count = (width as usize) * (height as usize);
    let mut pixels = vec![0u32; pixel_count];

    let to_bgra = |y: f32, cb: f32, cr: f32| -> u32 {
        let r = (1.164 * y + 1.596 * cr).clamp(0.0, 255.0) as u32;
        let g = (1.164 * y - 0.813 * cr - 0.391 * cb).clamp(0.0, 255.0) as u32;
        let b = (1.164 * y + 2.018 * cb).clamp(0.0, 255.0) as u32;
        0xFF00_0000 | (r << 16) | (g << 8) | b
    };

    for i in 0..pixel_count / 2 {
        let word = sys.mmio.phys_read_u32(base + (i as u32) * 4);
        let y0 = ((word >> 24) & 0xFF) as f32 - 16.0;
        let cb = ((word >> 16) & 0xFF) as f32 - 128.0;
        let y1 = ((word >> 8) & 0xFF) as f32 - 16.0;
        let cr = (word & 0xFF) as f32 - 128.0;
        pixels[i * 2] = to_bgra(y0, cb, cr);
        pixels[i * 2 + 1] = to_bgra(y1, cb, cr);
    }

    sys.render_sink.exec(GxAction::PresentRawXfb { width, height, pixels });
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

impl GraphicsProcessor {
    pub fn save_state(&self, w: &mut crate::savestate::StateWriter) {
        w.bool(self.raise_interrupt);
        w.bool(self.raise_token_interrupt);
        w.u16(self.pending_token);
        w.bool(self.token_dirty);
        w.pod(&self.projection);
        w.u32(self.bp_mask);

        w.bytes(bytemuck::cast_slice(&self.bp_regs));
        w.bytes(bytemuck::cast_slice(&self.cp_regs));
        w.bytes(bytemuck::cast_slice(&self.xf_mem));
        w.bytes(bytemuck::cast_slice(&self.palette_mem));
        w.bytes(&self.fifo);

        w.pod(&self.cur_textures);
        w.pod(&self.cur_tluts);
        w.pod(&self.cur_tev_color_env);
        w.pod(&self.cur_tev_alpha_env);
        w.pod(&self.cur_tev_color_regs_lo);
        w.pod(&self.cur_tev_color_regs_hi);
        w.pod(&self.cur_tev_const_regs_lo);
        w.pod(&self.cur_tev_const_regs_hi);
        w.pod(&self.cur_tev_orders);
        w.u8(self.cur_num_tev_stages);
        w.pod(&self.cur_tev_konst_colors);
        w.pod(&self.cur_indirect_matrices);
        w.pod(&self.cur_indirect_scales);
        w.pod(&self.cur_indirect_refs);
        w.pod(&self.cur_tev_indirect);
        w.u8(self.cur_num_indirect_stages);
        w.u32(self.cur_bump_imask);
        w.pod(&self.cur_zmode);
        w.pod(&self.cur_pe_control);
        w.pod(&self.cur_blend_mode);
        w.pod(&self.cur_alpha_compare);
        w.pod(&self.cur_viewport);
        w.pod(&self.cur_scissor);
        w.i32(self.cur_scissor_offset_x);
        w.i32(self.cur_scissor_offset_y);

        w.u32(self.xfb_regions.len() as u32);
        for (&base, region) in &self.xfb_regions {
            w.u32(base);
            w.u32(region.stride);
            w.u64(region.first_seq);
            w.u64(region.copy_seq);
            w.u64(region.seen_present_seq);
        }

        w.bool(self.xfb_dirty);
        w.u64(self.xfb_copy_seq);
        w.u64(self.xfb_present_seq);
        w.u32(self.xfb_last_present_base);
        w.u32(self.xfb_last_seen_base);
        w.u32(self.xfb_prev_base);
        w.u32(self.xfb_fields_since_flip);
        w.u32(self.xfb_flip_interval);
        w.u32(self.xfb_fields_since_emit);
        w.u64(self.xfb_last_emit_gen);

        w.pod(&self.cached_color_ctrl);
        w.pod(&self.cached_alpha_ctrl);
        w.pod(&self.cached_ambient_color);
        w.pod(&self.cached_material_color);
        w.pod(&self.cached_lights);
    }

    pub fn load_state(
        &mut self,
        r: &mut crate::savestate::StateReader<'_>,
    ) -> Result<(), crate::savestate::StateError> {
        self.raise_interrupt = r.bool()?;
        self.raise_token_interrupt = r.bool()?;
        self.pending_token = r.u16()?;
        self.token_dirty = r.bool()?;
        self.projection = r.pod()?;
        self.bp_mask = r.u32()?;

        r.bytes_into(bytemuck::cast_slice_mut(&mut self.bp_regs))?;
        r.bytes_into(bytemuck::cast_slice_mut(&mut self.cp_regs))?;
        r.bytes_into(bytemuck::cast_slice_mut(&mut self.xf_mem))?;
        r.bytes_into(bytemuck::cast_slice_mut(&mut self.palette_mem))?;

        self.fifo.clear();
        self.fifo.extend_from_slice(r.bytes()?);

        self.cur_textures = r.pod()?;
        self.cur_tluts = r.pod()?;
        self.cur_tev_color_env = r.pod()?;
        self.cur_tev_alpha_env = r.pod()?;
        self.cur_tev_color_regs_lo = r.pod()?;
        self.cur_tev_color_regs_hi = r.pod()?;
        self.cur_tev_const_regs_lo = r.pod()?;
        self.cur_tev_const_regs_hi = r.pod()?;
        self.cur_tev_orders = r.pod()?;
        self.cur_num_tev_stages = r.u8()?;
        self.cur_tev_konst_colors = r.pod()?;
        self.cur_indirect_matrices = r.pod()?;
        self.cur_indirect_scales = r.pod()?;
        self.cur_indirect_refs = r.pod()?;
        self.cur_tev_indirect = r.pod()?;
        self.cur_num_indirect_stages = r.u8()?;
        self.cur_bump_imask = r.u32()?;
        self.cur_zmode = r.pod()?;
        self.cur_pe_control = r.pod()?;
        self.cur_blend_mode = r.pod()?;
        self.cur_alpha_compare = r.pod()?;
        self.cur_viewport = r.pod()?;
        self.cur_scissor = r.pod()?;
        self.cur_scissor_offset_x = r.i32()?;
        self.cur_scissor_offset_y = r.i32()?;

        self.xfb_regions.clear();
        let region_count = r.u32()?;
        for _ in 0..region_count {
            let base = r.u32()?;
            let region = XfbRegion {
                stride: r.u32()?,
                first_seq: r.u64()?,
                copy_seq: r.u64()?,
                seen_present_seq: r.u64()?,
            };
            self.xfb_regions.insert(base, region);
        }

        self.xfb_dirty = r.bool()?;
        self.xfb_copy_seq = r.u64()?;
        self.xfb_present_seq = r.u64()?;
        self.xfb_last_present_base = r.u32()?;
        self.xfb_last_seen_base = r.u32()?;
        self.xfb_prev_base = r.u32()?;
        self.xfb_fields_since_flip = r.u32()?;
        self.xfb_flip_interval = r.u32()?;
        self.xfb_fields_since_emit = r.u32()?;
        self.xfb_last_emit_gen = r.u64()?;

        self.cached_color_ctrl = r.pod()?;
        self.cached_alpha_ctrl = r.pod()?;
        self.cached_ambient_color = r.pod()?;
        self.cached_material_color = r.pod()?;
        self.cached_lights = r.pod()?;

        self.dl_scratch.clear();
        self.texture_hashes.clear();
        self.tex_dirty = 0xFF;
        self.lighting_dirty = true;
        self.konst_dirty = true;
        self.frame_state_dirty = true;

        Ok(())
    }
}
