pub mod constants;
pub mod fifo;
pub mod regs;

use super::pi::InterruptFlag;
use crate::{
    flipper::gx::{
        constants::{BP_REG_SIZE, CP_REG_SIZE, VAT_REG, VCD_LO_REG, XF_MEM_SIZE},
        regs::{AttributeType, VatA, VcdLo},
    },
    gekko::Gekko,
    mmio::Mmio,
};
use fifo::FifoCmd;

pub struct Gx {
    pub raise_interrupt: bool,
    bp_regs: Vec<u32>,
    cp_regs: Vec<u32>,
    xf_mem: Vec<u32>,
    fifo: Vec<u8>,
}

impl Gx {
    pub fn new() -> Self {
        Gx {
            raise_interrupt: false,
            bp_regs: vec![0; BP_REG_SIZE],
            cp_regs: vec![0; CP_REG_SIZE],
            xf_mem: vec![0; XF_MEM_SIZE],
            fifo: Vec::with_capacity(256),
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
                _ => self.create_draw_call(mmio, cmd),
            }
        }
    }

    fn create_draw_call(&mut self, mmio: &mut Mmio, cmd: FifoCmd) {
        if let FifoCmd::DrawTriangles(cmd, data) = cmd {
            let vertex_format_index = cmd & 0b111;
            let (pos_attr, color0_attr) = self.vcd_attr_type(vertex_format_index as usize);
            tracing::debug!(
                data = format!("{:02X?}", data),
                ?pos_attr,
                ?color0_attr,
                vertex_format_index,
                "DrawTriangles"
            );
        }
    }

    fn vcd_attr_type(&self, vertex_format_index: usize) -> (AttributeType, AttributeType) {
        let vcd_lo = VcdLo::from_raw(self.cp_regs[VCD_LO_REG + vertex_format_index]);
        (vcd_lo.pos_attr(), vcd_lo.color0_attr())
    }

    fn load_bp(&mut self, data: &[u8]) {
        let idx = data[0] as usize;
        let val = u32::from_be_bytes([0, data[1], data[2], data[3]]);
        self.bp_regs[idx] = val;

        tracing::debug!(
            reg_idx = format!("{idx:02X}"),
            value = format!("{val:08X}"),
            "BP register write"
        );

        // PE finish: register 0x45, bit 1
        if idx == 0x45 && (val & 0x02) != 0 {
            self.raise_interrupt = true;
        }
    }

    fn load_cp(&mut self, data: &[u8]) {
        let idx = data[0] as usize;
        let val = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
        self.cp_regs[idx] = val;

        tracing::debug!(
            reg_idx = format!("{idx:02X}"),
            value = format!("{val:08X}"),
            "CP register write"
        );
    }

    fn load_xf(&mut self, data: &[u8]) {
        let length = u16::from_be_bytes([data[0], data[1]]) as usize;
        let addr = u16::from_be_bytes([data[2], data[3]]) as usize;
        let n = length + 1;

        for i in 0..n {
            let offset = 4 + i * 4;
            let val = u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]);
            let reg = addr + i;
            if reg < self.xf_mem.len() {
                self.xf_mem[reg] = val;
            }

            tracing::debug!(
                reg_idx = format!("{reg:04X}"),
                value = format!("{val:08X}"),
                "XF register write"
            );
        }
    }
}

impl Gekko {
    /// Check if the GX stub detected a finish command and assert the PI interrupt
    pub fn check_gx_pe_finish(&mut self) {
        if self.gx.raise_interrupt {
            self.gx.raise_interrupt = false;
            self.pi.assert_interrupt(InterruptFlag::PeFinish);
        }
    }
}
