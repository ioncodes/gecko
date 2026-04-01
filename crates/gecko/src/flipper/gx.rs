mod bp;
pub mod constants;
pub mod draw;
pub mod fifo;
mod lighting;
pub mod math;
pub mod regs;
mod tev;
mod texgen;
mod vertex;
mod xf;

use crate::{
    flipper::gx::{
        constants::{BP_REG_SIZE, CP_REG_SIZE, XF_MEM_SIZE},
        draw::DrawCommands,
        regs::{AlphaCompare, BlendMode, TevAlphaEnv, TevColorEnv, TevRegisterH, TevRegisterL, ZMode},
    },
    gamecube::GameCube,
    mmio::Mmio,
};
use fifo::FifoCmd;

pub struct GraphicsProcessor {
    pub raise_interrupt: bool,
    pub draw_commands: DrawCommands,
    pub(crate) bp_regs: Vec<u32>,
    pub(crate) cp_regs: Vec<u32>,
    pub(crate) xf_mem: Vec<u32>,
    pub(crate) fifo: Vec<u8>,

    // Current GX state to snapshot into a DrawCall later
    pub(crate) cur_textures: [Option<draw::TextureDescriptor>; 8],
    pub(crate) cur_tev_color_env: [TevColorEnv; 16],
    pub(crate) cur_tev_alpha_env: [TevAlphaEnv; 16],
    pub(crate) cur_tev_color_regs_lo: [TevRegisterL; 4],
    pub(crate) cur_tev_color_regs_hi: [TevRegisterH; 4],
    pub(crate) cur_tev_const_regs_lo: [TevRegisterL; 4],
    pub(crate) cur_tev_const_regs_hi: [TevRegisterH; 4],
    pub(crate) cur_tev_orders: [regs::TevOrder; 8],
    pub(crate) cur_num_tev_stages: u8,
    pub(crate) cur_tev_konst_colors: [[f32; 4]; 16],
    pub(crate) cur_zmode: ZMode,
    pub(crate) cur_blend_mode: BlendMode,
    pub(crate) cur_alpha_compare: AlphaCompare,
}

impl GraphicsProcessor {
    pub fn new() -> Self {
        GraphicsProcessor {
            raise_interrupt: false,
            bp_regs: vec![0; BP_REG_SIZE],
            cp_regs: vec![0; CP_REG_SIZE],
            xf_mem: vec![0; XF_MEM_SIZE],
            fifo: Vec::with_capacity(256),
            draw_commands: DrawCommands::default(),
            cur_textures: Default::default(),
            cur_tev_color_env: Default::default(),
            cur_tev_alpha_env: Default::default(),
            cur_tev_color_regs_lo: Default::default(),
            cur_tev_color_regs_hi: Default::default(),
            cur_tev_const_regs_lo: Default::default(),
            cur_tev_const_regs_hi: Default::default(),
            cur_tev_orders: Default::default(),
            cur_num_tev_stages: 0,
            cur_tev_konst_colors: [[0.0; 4]; 16],
            cur_zmode: Default::default(),
            cur_blend_mode: Default::default(),
            cur_alpha_compare: Default::default(),
        }
    }

    pub fn mmio_write_u8(&mut self, mmio: &mut Mmio, val: u8) {
        self.push_u8(val);
        self.drain_fifo(mmio);
    }

    pub fn mmio_write_u16(&mut self, mmio: &mut Mmio, val: u16) {
        self.push_u16(val);
        self.drain_fifo(mmio);
    }

    pub fn mmio_write_u32(&mut self, mmio: &mut Mmio, val: u32) {
        self.push_u32(val);
        self.drain_fifo(mmio);
    }

    fn drain_fifo(&mut self, mmio: &mut Mmio) {
        for cmd in self.drain() {
            match cmd {
                FifoCmd::Cp(data) => self.load_cp(&data),
                FifoCmd::Xf(data) => self.load_xf(&data),
                FifoCmd::Bp(data) => self.load_bp(&data),
                FifoCmd::CallDisplayList { phys_addr, nbytes } => {
                    let addr = phys_addr as usize;
                    let len = nbytes as usize;
                    self.execute_display_list(mmio, &mmio.ram[addr..addr + len].to_vec());
                }
                FifoCmd::DrawCall(cmd, data) => self.create_draw_call(mmio, cmd, data),
            }
        }
    }

    fn execute_display_list(&mut self, mmio: &mut Mmio, data: &[u8]) {
        let saved = std::mem::take(&mut self.fifo);
        self.fifo = data.to_vec();
        self.drain_fifo(mmio);
        self.fifo = saved;
    }
}

impl GameCube {
    /// Check if the GX stub detected a finish command and signal PE
    pub fn check_gx_pe_finish(&mut self) {
        if self.gx.raise_interrupt {
            self.gx.raise_interrupt = false;
            self.pe.signal_finish();
        }
        self.check_pe_interrupts();
    }
}
