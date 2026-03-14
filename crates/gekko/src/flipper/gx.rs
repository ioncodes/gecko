pub mod constants;
pub mod draw;
pub mod fifo;
pub mod regs;

use super::pi::InterruptFlag;
use crate::{
    flipper::gx::{
        constants::{
            ARRAY_BASE_REG, ARRAY_CLR0, ARRAY_POS, ARRAY_STRIDE_REG, BP_REG_SIZE, CP_REG_SIZE, VATA_REG, VCD_HI_REG,
            VCD_LO_REG, XF_MEM_SIZE, XF_MODELVIEW_BASE, XF_MODELVIEW_END, XF_PROJECTION_BASE, XF_PROJECTION_END,
        },
        draw::DrawCommands,
        regs::{VatA, VcdHi, VcdLo},
    },
    gekko::Gekko,
    mmio::Mmio,
};
use fifo::FifoCmd;

pub struct Gx {
    pub raise_interrupt: bool,
    pub draw_commands: DrawCommands,
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
            draw_commands: DrawCommands::default(),
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
        if let FifoCmd::DrawCall(cmd, data) = cmd {
            let fmt = (cmd & 0b111) as usize;
            let vcd_lo = VcdLo::from_raw(self.cp_regs[VCD_LO_REG + fmt]);
            let vcd_hi = VcdHi::from_raw(self.cp_regs[VCD_HI_REG + fmt]);
            let vat_a = VatA::from_raw(self.cp_regs[VATA_REG + fmt]);

            let pos_base = self.cp_regs[ARRAY_BASE_REG + ARRAY_POS] as usize;
            let pos_stride = self.cp_regs[ARRAY_STRIDE_REG + ARRAY_POS] as usize;
            let clr0_base = self.cp_regs[ARRAY_BASE_REG + ARRAY_CLR0] as usize;
            let clr0_stride = self.cp_regs[ARRAY_STRIDE_REG + ARRAY_CLR0] as usize;

            let tex0_size = match vcd_hi.tex0() {
                regs::AttributeType::Direct => vat_a.tex0_data_size(),
                regs::AttributeType::Index8 => 1,
                regs::AttributeType::Index16 => 2,
                regs::AttributeType::None => 0,
            };

            let vertex_stride = vcd_lo.position().size() + vcd_lo.color0().size() + tex0_size;
            let vertex_count = data.len() / vertex_stride;

            let mut vertices: Vec<draw::Vertex> = Vec::with_capacity(vertex_count);
            let mut cursor = 0;

            for i in 0..vertex_count {
                // Read position index
                let pos_index = read_index(&data, &mut cursor, vcd_lo.position());
                let pos_addr = pos_base + pos_index * pos_stride;
                let pos_size = vat_a.pos_data_size();
                let pos_data = &mmio.ram[pos_addr..pos_addr + pos_size];

                // Read color0 index
                let clr0_index = read_index(&data, &mut cursor, vcd_lo.color0());
                let clr0_addr = clr0_base + clr0_index * clr0_stride;
                let clr0_size = vat_a.clr0_data_size();
                let clr0_data = &mmio.ram[clr0_addr..clr0_addr + clr0_size];

                // Skip tex0 bytes
                cursor += tex0_size;

                tracing::debug!(
                    vertex = i,
                    pos_index,
                    pos_data = format!("{:02X?}", pos_data),
                    clr0_index,
                    clr0_data = format!("{:02X?}", clr0_data),
                    "Vertex"
                );

                let position = Self::decode_position(pos_data, &vat_a);
                let color0 = Self::decode_color(clr0_data, &vat_a);

                vertices.push(draw::Vertex { position, color0 });
            }

            if let Some(primitive) = draw::Primitive::from_cmd(cmd) {
                tracing::debug!(
                    primitive = format!("{:?}", primitive),
                    vertices = format!("{:?}", vertices),
                    modelview = format!("{:?}", self.draw_commands.modelview),
                    projection = format!("{:?}", self.draw_commands.projection),
                    "draw call created"
                );
                self.draw_commands.commands.push(draw::DrawCall { primitive, vertices });
            }
        }
    }

    fn decode_position(data: &[u8], vat: &VatA) -> [f32; 3] {
        let num = vat.pos_cnt().components();
        let fmt = vat.pos_fmt();
        let divisor = (1u32 << vat.pos_shift()) as f32;
        let mut result = [0.0f32; 3];
        let mut off = 0;

        for i in 0..num {
            result[i] = match fmt {
                regs::ComponentFormat::U8 => {
                    let v = data[off] as f32 / divisor;
                    off += 1;
                    v
                }
                regs::ComponentFormat::S8 => {
                    let v = data[off] as i8 as f32 / divisor;
                    off += 1;
                    v
                }
                regs::ComponentFormat::U16 => {
                    let v = u16::from_be_bytes([data[off], data[off + 1]]) as f32 / divisor;
                    off += 2;
                    v
                }
                regs::ComponentFormat::S16 => {
                    let v = i16::from_be_bytes([data[off], data[off + 1]]) as f32 / divisor;
                    off += 2;
                    v
                }
                regs::ComponentFormat::F32 => {
                    let v = f32::from_bits(u32::from_be_bytes([
                        data[off],
                        data[off + 1],
                        data[off + 2],
                        data[off + 3],
                    ]));
                    off += 4;
                    v
                }
            };
        }
        result
    }

    fn decode_color(data: &[u8], vat: &VatA) -> [f32; 4] {
        let has_alpha = vat.clr0_cnt() == regs::ColorCount::Rgba;
        match vat.clr0_fmt() {
            regs::ColorFormat::Rgb565 => {
                let raw = u16::from_be_bytes([data[0], data[1]]);
                let r = ((raw >> 11) & 0x1F) as f32 / 31.0;
                let g = ((raw >> 5) & 0x3F) as f32 / 63.0;
                let b = (raw & 0x1F) as f32 / 31.0;
                [r, g, b, 1.0]
            }
            regs::ColorFormat::Rgb8 => [
                data[0] as f32 / 255.0,
                data[1] as f32 / 255.0,
                data[2] as f32 / 255.0,
                1.0,
            ],
            regs::ColorFormat::Rgbx8 => [
                data[0] as f32 / 255.0,
                data[1] as f32 / 255.0,
                data[2] as f32 / 255.0,
                1.0,
            ],
            regs::ColorFormat::Rgba4 => {
                let raw = u16::from_be_bytes([data[0], data[1]]);
                let r = ((raw >> 12) & 0xF) as f32 / 15.0;
                let g = ((raw >> 8) & 0xF) as f32 / 15.0;
                let b = ((raw >> 4) & 0xF) as f32 / 15.0;
                let a = (raw & 0xF) as f32 / 15.0;
                [r, g, b, a]
            }
            regs::ColorFormat::Rgba6 => {
                let raw = u32::from_be_bytes([0, data[0], data[1], data[2]]);
                let r = ((raw >> 18) & 0x3F) as f32 / 63.0;
                let g = ((raw >> 12) & 0x3F) as f32 / 63.0;
                let b = ((raw >> 6) & 0x3F) as f32 / 63.0;
                let a = (raw & 0x3F) as f32 / 63.0;
                [r, g, b, a]
            }
            regs::ColorFormat::Rgba8 => {
                let r = data[0] as f32 / 255.0;
                let g = data[1] as f32 / 255.0;
                let b = data[2] as f32 / 255.0;
                let a = if has_alpha { data[3] as f32 / 255.0 } else { 1.0 };
                [r, g, b, a]
            }
        }
    }

    fn xf_f32(&self, reg: usize) -> f32 {
        f32::from_bits(self.xf_mem[reg])
    }

    fn rebuild_modelview(&mut self) {
        let b = XF_MODELVIEW_BASE;
        self.draw_commands.modelview = draw::Matrix4([
            [self.xf_f32(b), self.xf_f32(b + 4), self.xf_f32(b + 8), 0.0],
            [self.xf_f32(b + 1), self.xf_f32(b + 5), self.xf_f32(b + 9), 0.0],
            [self.xf_f32(b + 2), self.xf_f32(b + 6), self.xf_f32(b + 10), 0.0],
            [self.xf_f32(b + 3), self.xf_f32(b + 7), self.xf_f32(b + 11), 1.0],
        ]);
    }

    fn rebuild_projection(&mut self) {
        let b = XF_PROJECTION_BASE;
        let pm1 = self.xf_f32(b);
        let pm2 = self.xf_f32(b + 1);
        let pm3 = self.xf_f32(b + 2);
        let pm4 = self.xf_f32(b + 3);
        let pm5 = self.xf_f32(b + 4);
        let pm6 = self.xf_f32(b + 5);
        let proj_type = self.xf_mem[XF_PROJECTION_END];

        self.draw_commands.projection = if proj_type == 0 {
            // Perspective
            draw::Matrix4([
                [pm1, 0.0, 0.0, 0.0],
                [0.0, pm3, 0.0, 0.0],
                [pm2, pm4, pm5, -1.0],
                [0.0, 0.0, pm6, 0.0],
            ])
        } else {
            // Orthographic
            draw::Matrix4([
                [pm1, 0.0, 0.0, 0.0],
                [0.0, pm3, 0.0, 0.0],
                [0.0, 0.0, pm5, 0.0],
                [pm2, pm4, pm6, 1.0],
            ])
        };
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
        let end = addr + n;

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

        // Rebuild matrices if the write touched their address ranges
        if addr <= XF_MODELVIEW_END && end > XF_MODELVIEW_BASE {
            self.rebuild_modelview();
        }
        if addr <= XF_PROJECTION_END && end > XF_PROJECTION_BASE {
            self.rebuild_projection();
        }
    }
}

fn read_index(data: &[u8], cursor: &mut usize, attr: regs::AttributeType) -> usize {
    match attr {
        regs::AttributeType::Index8 => {
            let idx = data[*cursor] as usize;
            *cursor += 1;
            idx
        }
        regs::AttributeType::Index16 => {
            let idx = u16::from_be_bytes([data[*cursor], data[*cursor + 1]]) as usize;
            *cursor += 2;
            idx
        }
        _ => 0,
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
