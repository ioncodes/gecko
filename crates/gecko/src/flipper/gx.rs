mod bp;
pub mod constants;
pub mod draw;
pub mod fifo;
pub mod math;
pub mod regs;
pub mod tev;
mod texgen;
pub mod texture;
mod vertex;
mod xf;

use crate::flipper::gx::constants::{BP_REG_SIZE, CP_REG_SIZE, TLUT_MEM_ENTRIES, XF_MEM_SIZE};
use crate::flipper::gx::draw::Matrix4;
use crate::flipper::gx::regs::{AlphaCompare, BlendMode, TevAlphaEnv, TevColorEnv, TevRegisterH, TevRegisterL, ZMode};
#[cfg(feature = "efb-writeback")]
use crate::host::EfbWriteback;
use crate::host::{GxAction, RenderSink, XfbPart};
use crate::mmio::Mmio;
use crate::system::{System, SystemId};
use fifo::FifoCmd;
use rustc_hash::FxHashMap;

pub struct GraphicsProcessor {
    pub raise_interrupt: bool,
    pub raise_token_interrupt: bool,
    pub pending_token: u16,
    pub token_dirty: bool,
    pub projection: Matrix4,
    pub bp_regs: Vec<u32>,
    pub cp_regs: Vec<u32>,
    pub xf_mem: Vec<u32>,
    pub fifo: Vec<u8>,

    // Current GX state to snapshot into a Draw action later
    pub cur_textures: [Option<draw::TextureDescriptor>; 8],
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
    // XFB copies accumulated since the last vblank. `present_xfb()` drains
    // this at each field boundary to emit a PresentXfb action.
    pub xfb_copies: Vec<XfbCopy>,
    // Hash of the raw texture data at each RAM address; used to detect when
    // texture content changes and avoid redundant decodes + LoadTexture sends.
    pub texture_hashes: FxHashMap<u32, u64>,
    // Receiver for encoded EFB-to-texture bytes coming back from the
    // renderer worker. `efb_copy` drains this synchronously right after
    // emitting the copy action, so the next FIFO command in the same burst
    // (usually a `TX_SETIMAGE3` that samples the just-written region)
    // sees fresh RAM. Only present when the `efb-writeback` feature is on.
    #[cfg(feature = "efb-writeback")]
    pub efb_writeback_rx: Option<crossbeam_channel::Receiver<EfbWriteback>>,
}

/// A single EFB-to-XFB copy, stored until `present_xfb` computes the layout.
pub struct XfbCopy {
    pub dest_addr: u32,
    pub dest_stride: u32,
    pub src_h: u32,
}

impl GraphicsProcessor {
    pub fn new() -> Self {
        GraphicsProcessor {
            raise_interrupt: false,
            raise_token_interrupt: false,
            pending_token: 0,
            token_dirty: false,
            bp_regs: vec![0; BP_REG_SIZE],
            cp_regs: vec![0; CP_REG_SIZE],
            xf_mem: vec![0; XF_MEM_SIZE],
            fifo: Vec::with_capacity(256),
            projection: Matrix4::default(),
            cur_textures: Default::default(),
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
            xfb_copies: Vec::new(),
            texture_hashes: FxHashMap::default(),
            #[cfg(feature = "efb-writeback")]
            efb_writeback_rx: None,
        }
    }

    pub fn mmio_write_u8(&mut self, mmio: &mut Mmio, renderer: &mut dyn RenderSink, val: u8) {
        self.push_u8(val);
        self.drain_fifo(mmio, renderer);
    }

    pub fn mmio_write_u16(&mut self, mmio: &mut Mmio, renderer: &mut dyn RenderSink, val: u16) {
        self.push_u16(val);
        self.drain_fifo(mmio, renderer);
    }

    pub fn mmio_write_u32(&mut self, mmio: &mut Mmio, renderer: &mut dyn RenderSink, val: u32) {
        self.push_u32(val);
        self.drain_fifo(mmio, renderer);
    }

    fn drain_fifo(&mut self, mmio: &mut Mmio, renderer: &mut dyn RenderSink) {
        for cmd in self.drain() {
            match cmd {
                FifoCmd::Cp(data) => self.load_cp(&data),
                FifoCmd::Xf(data) => self.load_xf(renderer, &data),
                FifoCmd::Bp(data) => self.load_bp(renderer, &mut mmio.ram, &data),
                FifoCmd::LoadIndexedXf {
                    cp_array_index,
                    index,
                    xf_addr,
                    xf_count,
                } => {
                    self.load_indexed_xf(renderer, &mmio.ram, cp_array_index, index, xf_addr, xf_count);
                }
                FifoCmd::CallDisplayList { phys_addr, nbytes } => {
                    let addr = (phys_addr & 0x3FFFFFFF) as usize;
                    let len = nbytes as usize;
                    self.execute_display_list(mmio, renderer, &mmio.ram[addr..addr + len].to_vec());
                }
                FifoCmd::DrawCall(cmd, data) => self.create_draw_call(mmio, renderer, cmd, data),
            }
        }
    }

    fn execute_display_list(&mut self, mmio: &mut Mmio, renderer: &mut dyn RenderSink, data: &[u8]) {
        let saved = std::mem::take(&mut self.fifo);
        self.fifo = data.to_vec();
        self.drain_fifo(mmio, renderer);
        self.fifo = saved;
    }
}

pub fn present_xfb<const SYSTEM: SystemId>(gc: &mut System<SYSTEM>) {
    if gc.gx.xfb_copies.is_empty() {
        return;
    }

    let (frame_w, frame_h) = gc.vi.frame_dimensions();
    let vi_base = gc.vi.xfb_addr();

    // All copies in a frame share the same stride.
    let bytes_per_row = gc.gx.xfb_copies[0].dest_stride as u64;
    if bytes_per_row == 0 {
        tracing::warn!("present_xfb: zero bytes_per_row, dropping XFB copies");
        gc.gx.xfb_copies.clear();
        return;
    }
    let xfb_bytes = bytes_per_row * frame_h as u64;
    let stride_in_pixels = (bytes_per_row / 2) as u32;

    let build_parts = |base_addr: u32| -> Vec<XfbPart> {
        let mut parts = Vec::with_capacity(gc.gx.xfb_copies.len());
        for (id, copy) in gc.gx.xfb_copies.iter().enumerate() {
            if copy.dest_addr < base_addr {
                continue;
            }
            let delta_bytes = (copy.dest_addr - base_addr) as u64;
            if delta_bytes >= xfb_bytes {
                continue;
            }
            let delta_pixels = (delta_bytes / 2) as u32;
            let offset_x = delta_pixels % stride_in_pixels;
            let offset_y = delta_pixels / stride_in_pixels;

            // Real XFB copies always land at row boundaries (offset_x == 0).
            // A non-zero offset_x means this copy belongs to a different
            // buffer that happens to sit nearby in memory, reject it? TODO
            if offset_x != 0 || offset_y >= frame_h as u32 {
                continue;
            }

            parts.push(XfbPart {
                id: id as u32,
                offset_x,
                offset_y,
            });
        }
        parts
    };

    let min_base = gc.gx.xfb_copies.iter().map(|c| c.dest_addr).min().unwrap_or(0);

    let parts = if vi_base != 0 {
        let p = build_parts(vi_base);
        if !p.is_empty() { p } else { build_parts(min_base) }
    } else {
        build_parts(min_base)
    };

    if parts.is_empty() {
        tracing::warn!("present_xfb: no XFB copies matched the frame buffer region");
        gc.gx.xfb_copies.clear();
        return;
    }

    gc.render_sink.exec(GxAction::PresentXfb {
        width: frame_w,
        height: frame_h,
        parts,
    });
    gc.gx.xfb_copies.clear();
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
