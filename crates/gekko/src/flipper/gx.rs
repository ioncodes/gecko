pub mod constants;
pub mod draw;
pub mod fifo;
pub mod math;
pub mod regs;

use super::pi::InterruptFlag;
use crate::{
    flipper::gx::{
        constants::{
            ARRAY_BASE_REG, ARRAY_CLR0, ARRAY_NRM, ARRAY_POS, ARRAY_STRIDE_REG, BP_GEN_MODE, BP_PE_ALPHA_COMPARE,
            BP_PE_CMODE0, BP_PE_DONE, BP_PE_DONE_FINISH_BIT, BP_PE_ZMODE, BP_RAS1_TREF_COUNT, BP_RAS1_TREF0,
            BP_REG_SIZE, BP_TEV_COLOR_ENV_0, BP_TEV_REGISTERL_0, BP_TX_SETIMAGE0_I0, BP_TX_SETIMAGE0_I4,
            BP_TX_SETIMAGE3_I0, BP_TX_SETIMAGE3_I4, BP_TX_SETMODE0_I0, BP_TX_SETMODE0_I4, CP_REG_SIZE, VATA_REG,
            VCD_HI_REG, VCD_LO_REG, XF_AMBIENT_COLOR0, XF_CHAN_CTRL0, XF_LIGHT_A0, XF_LIGHT_BASE, XF_LIGHT_COLOR,
            XF_LIGHT_K0, XF_LIGHT_NX, XF_LIGHT_PX, XF_LIGHT_STRIDE, XF_MATERIAL_COLOR0, XF_MATRIX_INDEX_A, XF_MEM_SIZE,
            XF_NRM_MTX_BASE, XF_POS_MTX_STRIDE, XF_PROJECTION_BASE, XF_PROJECTION_END,
        },
        draw::DrawCommands,
        regs::{
            AlphaCompare, AttnFn, BlendMode, ChanCtrl, GenMode, MatrixIndex0, TevAlphaEnv, TevColorEnv, TevRegType,
            TevRegisterH, TevRegisterL, TxSetImage0, TxSetImage3, TxSetMode0, VatA, VcdHi, VcdLo, ZMode,
        },
    },
    gekko::Gekko,
    mmio::Mmio,
};
use fifo::FifoCmd;
use math::{Vec3, saturating_div, unpack_rgba};
use std::io::{Cursor, Read};

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

    fn create_draw_call(&mut self, mmio: &mut Mmio, cmd: u8, data: Vec<u8>) {
        let Some(primitive) = draw::Primitive::from_cmd(cmd) else {
            tracing::error!(cmd, "goofy draw command");
            return;
        };

        let fmt = (cmd & 0b111) as usize;
        let vcd_lo = VcdLo::from_raw(self.cp_regs[VCD_LO_REG + fmt]);
        let vcd_hi = VcdHi::from_raw(self.cp_regs[VCD_HI_REG + fmt]);
        let vat_a = VatA::from_raw(self.cp_regs[VATA_REG + fmt]);

        let pos_base = self.cp_regs[ARRAY_BASE_REG + ARRAY_POS] as usize;
        let pos_stride = self.cp_regs[ARRAY_STRIDE_REG + ARRAY_POS] as usize;
        let clr0_base = self.cp_regs[ARRAY_BASE_REG + ARRAY_CLR0] as usize;
        let clr0_stride = self.cp_regs[ARRAY_STRIDE_REG + ARRAY_CLR0] as usize;

        let tex0_attr = vcd_hi.tex0();
        let tex0_size = match tex0_attr {
            regs::AttributeType::Direct => vat_a.tex0_data_size(),
            regs::AttributeType::Index8 => 1,
            regs::AttributeType::Index16 => 2,
            regs::AttributeType::None => 0,
        };

        let pos_attr = vcd_lo.position();
        let nrm_attr = vcd_lo.normal();
        let clr0_attr = vcd_lo.color0();
        let pos_data_size = vat_a.pos_data_size();
        let nrm_data_size = vat_a.nrm_data_size();
        let clr0_data_size = vat_a.clr0_data_size();

        let pos_stream_size = match pos_attr {
            regs::AttributeType::Direct => pos_data_size,
            regs::AttributeType::Index8 => 1,
            regs::AttributeType::Index16 => 2,
            regs::AttributeType::None => 0,
        };
        let nrm_stream_size = match nrm_attr {
            regs::AttributeType::Direct => nrm_data_size,
            regs::AttributeType::Index8 => 1,
            regs::AttributeType::Index16 => 2,
            regs::AttributeType::None => 0,
        };
        let clr0_stream_size = match clr0_attr {
            regs::AttributeType::Direct => clr0_data_size,
            regs::AttributeType::Index8 => 1,
            regs::AttributeType::Index16 => 2,
            regs::AttributeType::None => 0,
        };

        let nrm_base_addr = self.cp_regs[ARRAY_BASE_REG + ARRAY_NRM] as usize;
        let nrm_stride = self.cp_regs[ARRAY_STRIDE_REG + ARRAY_NRM] as usize;

        let vertex_stride = pos_stream_size + nrm_stream_size + clr0_stream_size + tex0_size;
        let vertex_count = data.len() / vertex_stride;

        // Read matrix indices and lighting state before the vertex loop
        let mtx_index_a = MatrixIndex0::from_raw(self.xf_mem[XF_MATRIX_INDEX_A]);
        let pos_mtx_base = mtx_index_a.pos_mtx_idx() as usize * XF_POS_MTX_STRIDE;

        // Channel 0 lighting state
        let chan_ctrl = ChanCtrl::from_raw(self.xf_mem[XF_CHAN_CTRL0]);
        let ambient_reg = unpack_rgba(self.xf_mem[XF_AMBIENT_COLOR0]);
        let material_reg = unpack_rgba(self.xf_mem[XF_MATERIAL_COLOR0]);

        // Normal matrix: hardware derives index from pos_mtx_idx (SDK Eq 5-6)
        let nrm_mtx_idx = (mtx_index_a.pos_mtx_idx() as usize) % 32;
        let nrm_mtx_base = XF_NRM_MTX_BASE + nrm_mtx_idx * 3;
        let nrm_mtx: [f32; 9] = std::array::from_fn(|i| self.xf_f32(nrm_mtx_base + i));

        let mut vertices: Vec<draw::Vertex> = Vec::with_capacity(vertex_count);
        let mut cur = Cursor::new(&data);

        for i in 0..vertex_count {
            // Read position
            let position = if pos_attr == regs::AttributeType::Direct {
                let start = cur.position() as usize;
                cur.set_position(cur.position() + pos_data_size as u64);
                Self::decode_position(&data[start..start + pos_data_size], &vat_a)
            } else {
                let pos_index = read_index(&mut cur, pos_attr);
                let pos_addr = pos_base + pos_index * pos_stride;
                Self::decode_position(&mmio.ram[pos_addr..pos_addr + pos_data_size], &vat_a)
            };

            // Read normal
            let normal = if nrm_attr == regs::AttributeType::Direct {
                let start = cur.position() as usize;
                cur.set_position(cur.position() + nrm_data_size as u64);
                Self::decode_normal(&data[start..start + nrm_data_size], &vat_a)
            } else if nrm_attr != regs::AttributeType::None {
                let nrm_index = read_index(&mut cur, nrm_attr);
                let nrm_addr = nrm_base_addr + nrm_index * nrm_stride;
                Self::decode_normal(&mmio.ram[nrm_addr..nrm_addr + nrm_data_size], &vat_a)
            } else {
                [0.0, 0.0, 1.0]
            };

            // Read color0
            let color0 = if clr0_attr == regs::AttributeType::None {
                [1.0, 1.0, 1.0, 1.0]
            } else if clr0_attr == regs::AttributeType::Direct {
                let start = cur.position() as usize;
                cur.set_position(cur.position() + clr0_data_size as u64);
                Self::decode_color(&data[start..start + clr0_data_size], &vat_a)
            } else {
                let clr0_index = read_index(&mut cur, clr0_attr);
                let clr0_addr = clr0_base + clr0_index * clr0_stride;
                Self::decode_color(&mmio.ram[clr0_addr..clr0_addr + clr0_data_size], &vat_a)
            };

            // Read tex0
            let tex0 = if tex0_attr == regs::AttributeType::Direct && vat_a.tex0_cnt() == regs::TexCount::St {
                let mut s_buf = [0u8; 4];
                let mut t_buf = [0u8; 4];
                cur.read_exact(&mut s_buf).unwrap();
                cur.read_exact(&mut t_buf).unwrap();
                Some([f32::from_be_bytes(s_buf), f32::from_be_bytes(t_buf)])
            } else {
                cur.set_position(cur.position() + tex0_size as u64);
                None
            };

            // Compute per-vertex lighting
            let normal_view = Vec3::from(normal).transform(&nrm_mtx).normalize();
            let pos_view = self.xf_transform_3x4(pos_mtx_base, position);
            let final_color =
                self.compute_channel_lighting(&chan_ctrl, ambient_reg, material_reg, color0, normal_view, pos_view);

            tracing::debug!(
                vertex = i,
                position = format!("{:02X?}", position),
                color0 = format!("{:?}", final_color),
                "Vertex"
            );

            vertices.push(draw::Vertex {
                position,
                color0: final_color,
                tex0,
            });
        }

        let modelview = draw::Matrix4([
            [
                self.xf_f32(pos_mtx_base),
                self.xf_f32(pos_mtx_base + 4),
                self.xf_f32(pos_mtx_base + 8),
                0.0,
            ],
            [
                self.xf_f32(pos_mtx_base + 1),
                self.xf_f32(pos_mtx_base + 5),
                self.xf_f32(pos_mtx_base + 9),
                0.0,
            ],
            [
                self.xf_f32(pos_mtx_base + 2),
                self.xf_f32(pos_mtx_base + 6),
                self.xf_f32(pos_mtx_base + 10),
                0.0,
            ],
            [
                self.xf_f32(pos_mtx_base + 3),
                self.xf_f32(pos_mtx_base + 7),
                self.xf_f32(pos_mtx_base + 11),
                1.0,
            ],
        ]);

        tracing::debug!(
            primitive = format!("{:?}", primitive),
            vertices = format!("{:?}", vertices),
            pos_mtx_idx = mtx_index_a.pos_mtx_idx(),
            modelview = format!("{:?}", modelview),
            projection = format!("{:?}", self.draw_commands.projection),
            "draw call created"
        );
        self.draw_commands.commands.push(draw::DrawCall {
            primitive,
            vertices,
            modelview,
        });
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

    fn decode_normal(data: &[u8], vat: &VatA) -> [f32; 3] {
        let cnt = vat.nrm_cnt().components().min(3);
        let fmt = vat.nrm_fmt();
        let mut result = [0.0f32; 3];
        let mut off = 0;

        for i in 0..cnt {
            result[i] = match fmt {
                regs::ComponentFormat::U8 => {
                    let v = data[off] as f32 / 255.0;
                    off += 1;
                    v
                }
                regs::ComponentFormat::S8 => {
                    let v = data[off] as i8 as f32 / 127.0;
                    off += 1;
                    v
                }
                regs::ComponentFormat::U16 => {
                    let v = u16::from_be_bytes([data[off], data[off + 1]]) as f32 / 65535.0;
                    off += 2;
                    v
                }
                regs::ComponentFormat::S16 => {
                    let v = i16::from_be_bytes([data[off], data[off + 1]]) as f32 / 32767.0;
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

    fn compute_channel_lighting(
        &self,
        chan_ctrl: &ChanCtrl,
        ambient_reg: [f32; 4],
        material_reg: [f32; 4],
        vertex_color: [f32; 4],
        normal: Vec3,
        pos: Vec3,
    ) -> [f32; 4] {
        let mat_color = if chan_ctrl.mat_src() {
            vertex_color
        } else {
            material_reg
        };
        let amb_color = if chan_ctrl.amb_src() { vertex_color } else { ambient_reg };

        if !chan_ctrl.enable() {
            return mat_color;
        }

        let light_mask = chan_ctrl.light_mask();
        let mut acc = amb_color;

        for light_id in 0..8u32 {
            if (light_mask >> light_id) & 1 == 0 {
                continue;
            }

            let base = XF_LIGHT_BASE + (light_id as usize) * XF_LIGHT_STRIDE;
            let light_color = unpack_rgba(self.xf_mem[base + XF_LIGHT_COLOR]);
            let cosatt = self.xf_vec3(base + XF_LIGHT_A0);
            let distatt = self.xf_vec3(base + XF_LIGHT_K0);
            let light_pos = self.xf_vec3(base + XF_LIGHT_PX);
            let light_dir = self.xf_vec3(base + XF_LIGHT_NX);

            let to_light = light_pos - pos;
            let to_light_dir = to_light.normalize();
            let dot = normal.dot(to_light_dir);

            let diffuse = match chan_ctrl.diff_fn() {
                regs::DiffuseFn::None => 1.0,
                regs::DiffuseFn::Signed => dot,
                regs::DiffuseFn::Clamp => dot.max(0.0),
            };

            let attn = match chan_ctrl.attn_fn() {
                AttnFn::None => 1.0,
                AttnFn::Spot => {
                    let cos_angle = light_dir.dot(to_light_dir);
                    let angle_attn = (cosatt.0 + cosatt.1 * cos_angle + cosatt.2 * cos_angle * cos_angle).max(0.0);
                    let dist = to_light.length();
                    saturating_div(angle_attn, distatt.0 + distatt.1 * dist + distatt.2 * dist * dist)
                }
                AttnFn::Spec => {
                    let cos_angle = normal.dot(to_light_dir);
                    let angle_attn = (cosatt.0 + cosatt.1 * cos_angle + cosatt.2 * cos_angle * cos_angle).max(0.0);
                    let cos_ha = normal.dot(light_dir.normalize()).max(0.0);
                    saturating_div(angle_attn, distatt.0 + distatt.1 * cos_ha + distatt.2 * cos_ha * cos_ha)
                }
            };

            let factor = attn * diffuse;
            acc[0] += light_color[0] * factor;
            acc[1] += light_color[1] * factor;
            acc[2] += light_color[2] * factor;
            acc[3] += light_color[3] * factor;
        }

        // Clamp illumination before material multiply
        // Material multiplies clamped illumination
        std::array::from_fn(|i| mat_color[i] * acc[i].clamp(0.0, 1.0))
    }

    fn xf_f32(&self, reg: usize) -> f32 {
        f32::from_bits(self.xf_mem[reg])
    }

    fn xf_vec3(&self, reg: usize) -> Vec3 {
        Vec3(self.xf_f32(reg), self.xf_f32(reg + 1), self.xf_f32(reg + 2))
    }

    fn xf_transform_3x4(&self, base: usize, v: [f32; 3]) -> Vec3 {
        Vec3(
            self.xf_f32(base) * v[0]
                + self.xf_f32(base + 1) * v[1]
                + self.xf_f32(base + 2) * v[2]
                + self.xf_f32(base + 3),
            self.xf_f32(base + 4) * v[0]
                + self.xf_f32(base + 5) * v[1]
                + self.xf_f32(base + 6) * v[2]
                + self.xf_f32(base + 7),
            self.xf_f32(base + 8) * v[0]
                + self.xf_f32(base + 9) * v[1]
                + self.xf_f32(base + 10) * v[2]
                + self.xf_f32(base + 11),
        )
    }

    fn rebuild_projection(&mut self) {
        let pm1 = self.xf_f32(XF_PROJECTION_BASE);
        let pm2 = self.xf_f32(XF_PROJECTION_BASE + 1);
        let pm3 = self.xf_f32(XF_PROJECTION_BASE + 2);
        let pm4 = self.xf_f32(XF_PROJECTION_BASE + 3);
        let pm5 = self.xf_f32(XF_PROJECTION_BASE + 4);
        let pm6 = self.xf_f32(XF_PROJECTION_BASE + 5);
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

        // TX_SETIMAGE3 is written last for each texture slot, so we use it as
        // the trigger to snapshot the full texture descriptor
        let texture_slot = if idx >= BP_TX_SETIMAGE3_I0 && idx < BP_TX_SETIMAGE3_I0 + 4 {
            Some(idx - BP_TX_SETIMAGE3_I0)
        } else if idx >= BP_TX_SETIMAGE3_I4 && idx < BP_TX_SETIMAGE3_I4 + 4 {
            Some(idx - BP_TX_SETIMAGE3_I4 + 4)
        } else {
            None
        };

        if let Some(slot) = texture_slot {
            let image0_reg = if slot < 4 {
                BP_TX_SETIMAGE0_I0 + slot
            } else {
                BP_TX_SETIMAGE0_I4 + (slot - 4)
            };

            let image0 = TxSetImage0::from_raw(self.bp_regs[image0_reg]);
            let image3 = TxSetImage3::from_raw(val);

            let mode0_reg = if slot < 4 {
                BP_TX_SETMODE0_I0 + slot
            } else {
                BP_TX_SETMODE0_I4 + (slot - 4)
            };
            let mode0 = TxSetMode0::from_raw(self.bp_regs[mode0_reg]);

            let width = image0.width();
            let height = image0.height();
            let ram_addr = image3.ram_addr();

            tracing::debug!(
                slot,
                width,
                height,
                format = format!("{:?}", image0.format()),
                ram_addr = format!("{ram_addr:#010X}"),
                wrap_s = format!("{:?}", mode0.wrap_s()),
                wrap_t = format!("{:?}", mode0.wrap_t()),
                "texture descriptor updated"
            );

            self.draw_commands.textures[slot] = Some(draw::TextureDescriptor {
                ram_addr,
                width: width as u32,
                height: height as u32,
                format: image0.format(),
                wrap_s: mode0.wrap_s(),
                wrap_t: mode0.wrap_t(),
                mag_filter: mode0.mag_filter(),
                min_filter: mode0.min_filter(),
            });
        }

        // Forward PE render state to draw commands
        match idx {
            BP_PE_ZMODE => self.draw_commands.bp_zmode = ZMode::from_raw(val),
            BP_PE_CMODE0 => self.draw_commands.bp_blend_mode = BlendMode::from_raw(val),
            BP_PE_ALPHA_COMPARE => self.draw_commands.bp_alpha_compare = AlphaCompare::from_raw(val),
            _ => {}
        }

        // TEV color/alpha environment registers (0xC0-0xDF)
        // Even addresses = color env, odd = alpha env
        if idx >= BP_TEV_COLOR_ENV_0 && idx < BP_TEV_COLOR_ENV_0 + 32 {
            let stage = (idx - BP_TEV_COLOR_ENV_0) / 2;
            if idx % 2 == 0 {
                let env = TevColorEnv::from_raw(val);
                tracing::debug!(
                    stage,
                    a = format!("{:?}", env.a()),
                    b = format!("{:?}", env.b()),
                    c = format!("{:?}", env.c()),
                    d = format!("{:?}", env.d()),
                    bias = format!("{:?}", env.bias()),
                    sub = env.sub(),
                    scale = format!("{:?}", env.scale()),
                    dest = format!("{:?}", env.dest()),
                    "TEV color env"
                );
                self.draw_commands.tev_color_env[stage] = env;
            } else {
                let env = TevAlphaEnv::from_raw(val);
                tracing::debug!(
                    stage,
                    a = format!("{:?}", env.a()),
                    b = format!("{:?}", env.b()),
                    c = format!("{:?}", env.c()),
                    d = format!("{:?}", env.d()),
                    bias = format!("{:?}", env.bias()),
                    sub = env.sub(),
                    scale = format!("{:?}", env.scale()),
                    dest = format!("{:?}", env.dest()),
                    "TEV alpha env"
                );
                self.draw_commands.tev_alpha_env[stage] = env;
            }
        }

        // TEV rasterizer order registers (RAS1_TREF0-7, 0x28-0x2F)
        if idx >= BP_RAS1_TREF0 && idx < BP_RAS1_TREF0 + BP_RAS1_TREF_COUNT {
            self.draw_commands.tev_orders[idx - BP_RAS1_TREF0] = val;
        }

        // TEV color registers (0xE0-0xE7): pairs of lo/hi writes
        if idx >= BP_TEV_REGISTERL_0 && idx <= BP_TEV_REGISTERL_0 + 7 {
            let reg_idx = (idx - BP_TEV_REGISTERL_0) / 2;
            if idx % 2 == 0 {
                let reg = TevRegisterL::from_raw(val);
                tracing::debug!(
                    reg_idx,
                    r = reg.r(),
                    a = reg.a(),
                    reg_type = format!("{:?}", reg.reg_type()),
                    "TEV register lo"
                );
                match reg.reg_type() {
                    TevRegType::Color => self.draw_commands.tev_color_regs_lo[reg_idx] = reg,
                    TevRegType::Constant => self.draw_commands.tev_const_regs_lo[reg_idx] = reg,
                }
            } else {
                let reg = TevRegisterH::from_raw(val);
                tracing::debug!(
                    reg_idx,
                    g = reg.g(),
                    b = reg.b(),
                    reg_type = format!("{:?}", reg.reg_type()),
                    "TEV register hi"
                );
                match reg.reg_type() {
                    TevRegType::Color => self.draw_commands.tev_color_regs_hi[reg_idx] = reg,
                    TevRegType::Constant => self.draw_commands.tev_const_regs_hi[reg_idx] = reg,
                }
            }
        }

        // GEN_MODE, extract num TEV stages
        if idx == BP_GEN_MODE {
            let gen_mode = GenMode::from_raw(val);
            let stages = gen_mode.num_tev_stages() + 1;
            tracing::debug!(num_tev_stages = stages, "GEN_MODE");
            self.draw_commands.num_tev_stages = stages;
        }

        // PE finish
        if idx == BP_PE_DONE && (val & BP_PE_DONE_FINISH_BIT) != 0 {
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

        // Rebuild projection if the write touched its address range
        // (modelview is resolved lazily at draw call time from the current position matrix slot)
        if addr <= XF_PROJECTION_END && end > XF_PROJECTION_BASE {
            self.rebuild_projection();
        }
    }
}

fn read_index(cur: &mut Cursor<&Vec<u8>>, attr: regs::AttributeType) -> usize {
    match attr {
        regs::AttributeType::Index8 => {
            let mut buf = [0u8; 1];
            cur.read_exact(&mut buf).unwrap();
            buf[0] as usize
        }
        regs::AttributeType::Index16 => {
            let mut buf = [0u8; 2];
            cur.read_exact(&mut buf).unwrap();
            u16::from_be_bytes(buf) as usize
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
