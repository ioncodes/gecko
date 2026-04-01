pub mod constants;
pub mod draw;
pub mod fifo;
pub mod math;
pub mod regs;

use crate::{
    flipper::gx::{
        constants::{
            ARRAY_BASE_REG, ARRAY_CLR0, ARRAY_NRM, ARRAY_POS, ARRAY_STRIDE_REG, ARRAY_TEX0, BP_GEN_MODE,
            BP_PE_ALPHA_COMPARE, BP_PE_CMODE0, BP_PE_DONE, BP_PE_DONE_FINISH_BIT, BP_PE_ZMODE, BP_RAS1_TREF_COUNT,
            BP_RAS1_TREF0, BP_REG_SIZE, BP_TEV_COLOR_ENV_0, BP_TEV_KSEL_0, BP_TEV_REGISTERL_0, BP_TX_SETIMAGE0_I0,
            BP_TX_SETIMAGE0_I4, BP_TX_SETIMAGE3_I0, BP_TX_SETIMAGE3_I4, BP_TX_SETMODE0_I0, BP_TX_SETMODE0_I4,
            CP_REG_SIZE, VATA_REG, VATB_REG, VATC_REG, VCD_HI_REG, VCD_LO_REG, XF_ALPHA_CTRL0, XF_AMBIENT_COLOR0,
            XF_COLOR_CTRL0, XF_DUAL_TEX_ENABLE, XF_DUALTEX_BASE, XF_LIGHT_A0, XF_LIGHT_BASE, XF_LIGHT_COLOR,
            XF_LIGHT_K0, XF_LIGHT_NX, XF_LIGHT_PX, XF_LIGHT_STRIDE, XF_MATERIAL_COLOR0, XF_MATRIX_INDEX_A,
            XF_MATRIX_INDEX_B, XF_MEM_SIZE, XF_NRM_MTX_BASE, XF_NUM_TEXGENS, XF_POS_MTX_STRIDE, XF_POST_MTX_BASE,
            XF_PROJECTION_BASE, XF_PROJECTION_END, XF_TEXGEN_BASE,
        },
        draw::DrawCommands,
        regs::{
            AlphaCompare, AttnFn, BlendMode, ChanCtrl, DualTexGenReg, GenMode, MatrixIndex0, MatrixIndex1, TevAlphaEnv,
            TevColorEnv, TevRegType, TevRegisterH, TevRegisterL, TexGenReg, TxSetImage0, TxSetImage3, TxSetMode0, VatA,
            VatB, VatC, VcdHi, VcdLo, ZMode,
        },
    },
    gamecube::GameCube,
    mmio::Mmio,
};
use fifo::FifoCmd;
use math::{Vec3, saturating_div, unpack_rgba};
use std::io::{Cursor, Read};

pub struct GraphicsProcessor {
    pub raise_interrupt: bool,
    pub draw_commands: DrawCommands,
    bp_regs: Vec<u32>,
    cp_regs: Vec<u32>,
    xf_mem: Vec<u32>,
    fifo: Vec<u8>,

    // Current GX state to snapshot into a DrawCall later
    cur_textures: [Option<draw::TextureDescriptor>; 8],
    cur_tev_color_env: [TevColorEnv; 16],
    cur_tev_alpha_env: [TevAlphaEnv; 16],
    cur_tev_color_regs_lo: [TevRegisterL; 4],
    cur_tev_color_regs_hi: [TevRegisterH; 4],
    cur_tev_const_regs_lo: [TevRegisterL; 4],
    cur_tev_const_regs_hi: [TevRegisterH; 4],
    cur_tev_orders: [regs::TevOrder; 8],
    cur_num_tev_stages: u8,
    cur_tev_konst_colors: [[f32; 4]; 16],
    cur_zmode: ZMode,
    cur_blend_mode: BlendMode,
    cur_alpha_compare: AlphaCompare,
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

    fn create_draw_call(&mut self, mmio: &mut Mmio, cmd: u8, data: Vec<u8>) {
        let Some(primitive) = draw::Primitive::from_cmd(cmd) else {
            tracing::error!(cmd, "goofy draw command");
            return;
        };

        let fmt = (cmd & 0b111) as usize;
        // VCD is global state (single register), VAT is per-format
        let vcd_lo = VcdLo::from_raw(self.cp_regs[VCD_LO_REG]);
        let vcd_hi = VcdHi::from_raw(self.cp_regs[VCD_HI_REG]);
        let vat_a = VatA::from_raw(self.cp_regs[VATA_REG + fmt]);
        let vat_b = VatB::from_raw(self.cp_regs[VATB_REG + fmt]);
        let vat_c = VatC::from_raw(self.cp_regs[VATC_REG + fmt]);

        let attr_stream_size = |attr: regs::AttributeType, direct_size: usize| -> usize {
            match attr {
                regs::AttributeType::Direct => direct_size,
                regs::AttributeType::Index8 => 1,
                regs::AttributeType::Index16 => 2,
                regs::AttributeType::None => 0,
            }
        };

        let pos_base = self.cp_regs[ARRAY_BASE_REG + ARRAY_POS] as usize;
        let pos_stride = self.cp_regs[ARRAY_STRIDE_REG + ARRAY_POS] as usize;
        let clr0_base = self.cp_regs[ARRAY_BASE_REG + ARRAY_CLR0] as usize;
        let clr0_stride = self.cp_regs[ARRAY_STRIDE_REG + ARRAY_CLR0] as usize;

        let pos_attr = vcd_lo.position();
        let nrm_attr = vcd_lo.normal();
        let clr0_attr = vcd_lo.color0();
        let pos_data_size = vat_a.pos_data_size();
        let nrm_data_size = vat_a.nrm_data_size();
        let clr0_data_size = vat_a.clr0_data_size();

        let mtx_idx_size = vcd_lo.mtx_idx_count();
        let pos_stream_size = attr_stream_size(pos_attr, pos_data_size);
        let nrm_stream_size = vat_a.nrm_stream_size(nrm_attr);
        let clr0_stream_size = attr_stream_size(clr0_attr, clr0_data_size);
        let clr1_stream_size = attr_stream_size(vcd_lo.color1(), vat_a.clr1_data_size());

        // Build per-texcoord metadata arrays
        let tex_attrs = [
            vcd_hi.tex0(),
            vcd_hi.tex1(),
            vcd_hi.tex2(),
            vcd_hi.tex3(),
            vcd_hi.tex4(),
            vcd_hi.tex5(),
            vcd_hi.tex6(),
            vcd_hi.tex7(),
        ];
        let tex_data_sizes = [
            vat_a.tex0_data_size(),
            vat_b.tex1_data_size(),
            vat_b.tex2_data_size(),
            vat_b.tex3_data_size(),
            vat_b.tex4_data_size(),
            vat_c.tex5_data_size(),
            vat_c.tex6_data_size(),
            vat_c.tex7_data_size(),
        ];
        let tex_fmts = [
            vat_a.tex0_fmt(),
            vat_b.tex1_fmt(),
            vat_b.tex2_fmt(),
            vat_b.tex3_fmt(),
            vat_b.tex4_fmt(),
            vat_c.tex5_fmt(),
            vat_c.tex6_fmt(),
            vat_c.tex7_fmt(),
        ];
        let tex_shifts = [
            vat_a.tex0_shift(),
            vat_b.tex1_shift(),
            vat_b.tex2_shift(),
            vat_b.tex3_shift(),
            vat_c.tex4_shift(),
            vat_c.tex5_shift(),
            vat_c.tex6_shift(),
            vat_c.tex7_shift(),
        ];
        let tex_cnts = [
            vat_a.tex0_cnt(),
            vat_b.tex1_cnt(),
            vat_b.tex2_cnt(),
            vat_b.tex3_cnt(),
            vat_b.tex4_cnt(),
            vat_c.tex5_cnt(),
            vat_c.tex6_cnt(),
            vat_c.tex7_cnt(),
        ];
        let tex_stream_sizes: [usize; 8] = std::array::from_fn(|i| attr_stream_size(tex_attrs[i], tex_data_sizes[i]));
        let tex_bases: [usize; 8] = std::array::from_fn(|i| self.cp_regs[ARRAY_BASE_REG + ARRAY_TEX0 + i] as usize);
        let tex_strides: [usize; 8] = std::array::from_fn(|i| self.cp_regs[ARRAY_STRIDE_REG + ARRAY_TEX0 + i] as usize);

        let nrm_base_addr = self.cp_regs[ARRAY_BASE_REG + ARRAY_NRM] as usize;
        let nrm_stride = self.cp_regs[ARRAY_STRIDE_REG + ARRAY_NRM] as usize;

        let vertex_stride = mtx_idx_size
            + pos_stream_size
            + nrm_stream_size
            + clr0_stream_size
            + clr1_stream_size
            + tex_stream_sizes.iter().sum::<usize>();

        if vertex_stride == 0 {
            tracing::warn!("draw call with zero vertex stride, skipping");
            return;
        }

        let vertex_count = data.len() / vertex_stride;

        // Per-vertex matrix index handling
        let has_pnmtxidx = vcd_lo.pos_nrm_mtx_idx();
        let tex_mtx_idx_size = mtx_idx_size - has_pnmtxidx as usize; // remaining mtx idx bytes after pnmtxidx

        // Default matrix index from register (used when per-vertex index is absent)
        let mtx_index_a = MatrixIndex0::from_raw(self.xf_mem[XF_MATRIX_INDEX_A]);
        let default_pos_mtx_idx = mtx_index_a.pos_mtx_idx();

        // Channel 0 lighting state (COLOR0 and ALPHA0 have separate controls)
        let color_ctrl = ChanCtrl::from_raw(self.xf_mem[XF_COLOR_CTRL0]);
        let alpha_ctrl = ChanCtrl::from_raw(self.xf_mem[XF_ALPHA_CTRL0]);
        let ambient_reg = unpack_rgba(self.xf_mem[XF_AMBIENT_COLOR0]);
        let material_reg = unpack_rgba(self.xf_mem[XF_MATERIAL_COLOR0]);

        let mut vertices: Vec<draw::Vertex> = Vec::with_capacity(vertex_count);
        let mut cur = Cursor::new(&data);

        for i in 0..vertex_count {
            // Read per-vertex position/normal matrix index, or use register default
            let pos_mtx_idx = if has_pnmtxidx {
                let mut buf = [0u8; 1];
                cur.read_exact(&mut buf).unwrap();
                buf[0] & 0x3F
            } else {
                default_pos_mtx_idx
            };

            // Skip remaining tex matrix index bytes (not yet used)
            cur.set_position(cur.position() + tex_mtx_idx_size as u64);

            // Derive per-vertex position and normal matrix bases
            let pos_mtx_base = pos_mtx_idx as usize * XF_POS_MTX_STRIDE;
            let nrm_mtx_idx = (pos_mtx_idx as usize) & 31;
            let nrm_mtx_base = XF_NRM_MTX_BASE + nrm_mtx_idx * 3;
            let nrm_mtx: [f32; 9] = std::array::from_fn(|i| self.xf_f32(nrm_mtx_base + i));

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
                // When nrm_index3 && NBT, the stream contains 3 separate indices
                // (normal, binormal, tangent). Skip the extra 2 — only the normal
                // vector (first index) is needed for lighting.
                if vat_a.nrm_index3() && vat_a.nrm_cnt() == regs::NrmCount::Nbt {
                    let idx_size = if nrm_attr == regs::AttributeType::Index8 { 1 } else { 2 };
                    cur.set_position(cur.position() + (2 * idx_size) as u64);
                }
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

            // Skip color1 (not yet used for rendering)
            cur.set_position(cur.position() + clr1_stream_size as u64);

            // Read all texcoords (tex0-tex7)
            let mut raw_texcoords: [Option<[f32; 2]>; 8] = [None; 8];
            for tc in 0..8 {
                let tc_attr = tex_attrs[tc];
                let tc_data_size = tex_data_sizes[tc];
                if tc_attr == regs::AttributeType::None {
                    continue;
                }
                raw_texcoords[tc] = Some(if tc_attr == regs::AttributeType::Direct {
                    let start = cur.position() as usize;
                    cur.set_position(cur.position() + tc_data_size as u64);
                    Self::decode_texcoord(
                        &data[start..start + tc_data_size],
                        tex_fmts[tc],
                        tex_shifts[tc],
                        tex_cnts[tc],
                    )
                } else {
                    let tc_index = read_index(&mut cur, tc_attr);
                    let tc_addr = tex_bases[tc] + tc_index * tex_strides[tc];
                    Self::decode_texcoord(
                        &mmio.ram[tc_addr..tc_addr + tc_data_size],
                        tex_fmts[tc],
                        tex_shifts[tc],
                        tex_cnts[tc],
                    )
                });
            }

            // Compute per-vertex lighting
            let normal_view = Vec3::from(normal).transform(&nrm_mtx).normalize();
            let pos_view = self.xf_transform_3x4(pos_mtx_base, position);
            let final_color = self.compute_channel_lighting(
                &color_ctrl,
                &alpha_ctrl,
                ambient_reg,
                material_reg,
                color0,
                normal_view,
                pos_view,
            );

            // Texture coordinate generation (XF texgen)
            let num_texgens = (self.xf_mem[XF_NUM_TEXGENS] as usize).min(8);
            let mut texcoords: [Option<[f32; 2]>; 8] = [None; 8];
            for tg_idx in 0..num_texgens {
                texcoords[tg_idx] = Some(self.compute_texgen(tg_idx, position, normal, &raw_texcoords));
            }
            // For texcoords beyond num_texgens, pass through raw values
            for tg_idx in num_texgens..8 {
                texcoords[tg_idx] = raw_texcoords[tg_idx];
            }

            tracing::debug!(
                vertex = i,
                position = format!("{:02X?}", position),
                color0 = format!("{:?}", final_color),
                "Vertex"
            );

            vertices.push(draw::Vertex {
                position,
                color0: final_color,
                texcoords,
            });
        }

        // Modelview uses the register-level default matrix (per-vertex transforms are
        // already applied during lighting via xf_transform_3x4 per vertex)
        let default_mtx_base = default_pos_mtx_idx as usize * XF_POS_MTX_STRIDE;
        let modelview = draw::Matrix4([
            [
                self.xf_f32(default_mtx_base),
                self.xf_f32(default_mtx_base + 4),
                self.xf_f32(default_mtx_base + 8),
                0.0,
            ],
            [
                self.xf_f32(default_mtx_base + 1),
                self.xf_f32(default_mtx_base + 5),
                self.xf_f32(default_mtx_base + 9),
                0.0,
            ],
            [
                self.xf_f32(default_mtx_base + 2),
                self.xf_f32(default_mtx_base + 6),
                self.xf_f32(default_mtx_base + 10),
                0.0,
            ],
            [
                self.xf_f32(default_mtx_base + 3),
                self.xf_f32(default_mtx_base + 7),
                self.xf_f32(default_mtx_base + 11),
                1.0,
            ],
        ]);

        tracing::debug!(
            primitive = format!("{:?}", primitive),
            vertices = format!("{:?}", vertices),
            pos_mtx_idx = default_pos_mtx_idx,
            modelview = format!("{:?}", modelview),
            projection = format!("{:?}", self.draw_commands.projection),
            "draw call created"
        );

        // Resolve TEV color registers to f32 arrays for the snapshot
        let tev_color_regs = self.resolve_tev_color_regs();

        self.draw_commands.commands.push(draw::DrawCall {
            primitive,
            vertices,
            modelview,
            textures: self.cur_textures,
            tev_orders: self.resolve_tev_orders(),
            tev_color_env: self.cur_tev_color_env,
            tev_alpha_env: self.cur_tev_alpha_env,
            tev_color_regs,
            tev_konst_colors: self.cur_tev_konst_colors,
            num_tev_stages: self.cur_num_tev_stages,
            bp_zmode: self.cur_zmode,
            bp_blend_mode: self.cur_blend_mode,
            bp_alpha_compare: self.cur_alpha_compare,
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

    fn decode_texcoord(data: &[u8], fmt: regs::ComponentFormat, shift: u8, cnt: regs::TexCount) -> [f32; 2] {
        let num = cnt.components();
        let divisor = (1u32 << shift) as f32;
        let mut result = [0.0f32; 2];
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

    fn compute_channel_lighting(
        &self,
        color_ctrl: &ChanCtrl,
        alpha_ctrl: &ChanCtrl,
        ambient_reg: [f32; 4],
        material_reg: [f32; 4],
        vertex_color: [f32; 4],
        normal: Vec3,
        pos: Vec3,
    ) -> [f32; 4] {
        let mat_rgb: [f32; 3] = if color_ctrl.mat_src() {
            [vertex_color[0], vertex_color[1], vertex_color[2]]
        } else {
            [material_reg[0], material_reg[1], material_reg[2]]
        };
        let amb_rgb: [f32; 3] = if color_ctrl.amb_src() {
            [vertex_color[0], vertex_color[1], vertex_color[2]]
        } else {
            [ambient_reg[0], ambient_reg[1], ambient_reg[2]]
        };

        let mat_a = if alpha_ctrl.mat_src() {
            vertex_color[3]
        } else {
            material_reg[3]
        };
        let amb_a = if alpha_ctrl.amb_src() {
            vertex_color[3]
        } else {
            ambient_reg[3]
        };

        // When lighting is disabled, return material color directly
        let color_lit_off = !color_ctrl.enable();
        let alpha_lit_off = !alpha_ctrl.enable();
        if color_lit_off && alpha_lit_off {
            return [mat_rgb[0], mat_rgb[1], mat_rgb[2], mat_a];
        }

        // Compute RGB lighting accumulator
        let mut rgb_out = mat_rgb;
        if !color_lit_off {
            let light_mask = color_ctrl.light_mask();
            let mut acc = amb_rgb;
            for light_id in 0..8u32 {
                if (light_mask >> light_id) & 1 == 0 {
                    continue;
                }
                let base = XF_LIGHT_BASE + (light_id as usize) * XF_LIGHT_STRIDE;
                let light_color = unpack_rgba(self.xf_mem[base + XF_LIGHT_COLOR]);
                let factor = self.compute_light_factor(color_ctrl, base, normal, pos);
                acc[0] += light_color[0] * factor;
                acc[1] += light_color[1] * factor;
                acc[2] += light_color[2] * factor;
            }
            rgb_out = std::array::from_fn(|i| mat_rgb[i] * acc[i].clamp(0.0, 1.0));
        }

        // Compute Alpha lighting accumulator
        let mut a_out = mat_a;
        if !alpha_lit_off {
            let light_mask = alpha_ctrl.light_mask();
            let mut acc_a = amb_a;
            for light_id in 0..8u32 {
                if (light_mask >> light_id) & 1 == 0 {
                    continue;
                }
                let base = XF_LIGHT_BASE + (light_id as usize) * XF_LIGHT_STRIDE;
                let light_color = unpack_rgba(self.xf_mem[base + XF_LIGHT_COLOR]);
                let factor = self.compute_light_factor(alpha_ctrl, base, normal, pos);
                acc_a += light_color[3] * factor;
            }
            a_out = mat_a * acc_a.clamp(0.0, 1.0);
        }

        [rgb_out[0], rgb_out[1], rgb_out[2], a_out]
    }

    fn compute_light_factor(&self, ctrl: &ChanCtrl, base: usize, normal: Vec3, pos: Vec3) -> f32 {
        let cosatt = self.xf_vec3(base + XF_LIGHT_A0);
        let distatt = self.xf_vec3(base + XF_LIGHT_K0);
        let light_pos = self.xf_vec3(base + XF_LIGHT_PX);
        let light_dir = self.xf_vec3(base + XF_LIGHT_NX);

        // ldir starts as light_pos - vertex_pos for all modes
        // In Spot mode, light_pos is a world position
        // In Spec mode, light_pos is repurposed as the half-angle direction vector
        let mut ldir = light_pos - pos;

        let attn = match ctrl.attn_fn() {
            AttnFn::None => {
                ldir = ldir.normalize();
                1.0
            }
            AttnFn::Spot => {
                let dist = ldir.length();
                ldir = ldir.normalize();
                let cos_angle = light_dir.dot(ldir);
                let angle_attn = (cosatt.0 + cosatt.1 * cos_angle + cosatt.2 * cos_angle * cos_angle).max(0.0);
                saturating_div(angle_attn, distatt.0 + distatt.1 * dist + distatt.2 * dist * dist)
            }
            AttnFn::Spec => {
                ldir = ldir.normalize();
                let half_dot = ldir.dot(normal);
                if half_dot < 0.0 {
                    return 0.0;
                }
                let s = normal.dot(light_dir).max(0.0);
                let att_len = Vec3(1.0, s, s * s);
                let da = if ctrl.diff_fn() != regs::DiffuseFn::None {
                    distatt.normalize()
                } else {
                    distatt
                };
                saturating_div(att_len.dot(cosatt).max(0.0), att_len.dot(da))
            }
        };

        // Diffuse is computed from ldir dotted with normal
        let dif_attn = ldir.dot(normal);
        let diffuse = match ctrl.diff_fn() {
            regs::DiffuseFn::None => 1.0,
            regs::DiffuseFn::Signed => dif_attn,
            regs::DiffuseFn::Clamp => dif_attn.max(0.0),
        };

        attn * diffuse
    }

    fn compute_texgen(
        &self,
        texgen_idx: usize,
        position: [f32; 3],
        normal: [f32; 3],
        raw_texcoords: &[Option<[f32; 2]>; 8],
    ) -> [f32; 2] {
        let tg = TexGenReg::from_raw(self.xf_mem[XF_TEXGEN_BASE + texgen_idx]);
        let dt = DualTexGenReg::from_raw(self.xf_mem[XF_DUALTEX_BASE + texgen_idx]);

        // Select the source input vector based on texgen source_row
        let src = match tg.source_row() {
            regs::TexGenSrc::Pos => position,
            regs::TexGenSrc::Nrm => normal,
            regs::TexGenSrc::Tex0 => {
                let t = raw_texcoords[0].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            regs::TexGenSrc::Tex1 => {
                let t = raw_texcoords[1].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            regs::TexGenSrc::Tex2 => {
                let t = raw_texcoords[2].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            regs::TexGenSrc::Tex3 => {
                let t = raw_texcoords[3].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            regs::TexGenSrc::Tex4 => {
                let t = raw_texcoords[4].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            regs::TexGenSrc::Tex5 => {
                let t = raw_texcoords[5].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            regs::TexGenSrc::Tex6 => {
                let t = raw_texcoords[6].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            regs::TexGenSrc::Tex7 => {
                let t = raw_texcoords[7].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            // TODO: ???
            _ => [0.0, 0.0, 1.0],
        };

        // Form input vector based on input_form
        let input = match tg.input_form() {
            regs::TexGenInputForm::Ab11 => [src[0], src[1], 1.0, 1.0],
            regs::TexGenInputForm::Abc1 => [src[0], src[1], src[2], 1.0],
        };

        // Texture matrix base from MatrixIndex register
        let tex_mtx_idx = if texgen_idx < 4 {
            let mtx_index_a = MatrixIndex0::from_raw(self.xf_mem[XF_MATRIX_INDEX_A]);
            mtx_index_a.tex_mtx_idx(texgen_idx) as usize
        } else {
            let mtx_index_b = MatrixIndex1::from_raw(self.xf_mem[XF_MATRIX_INDEX_B]);
            mtx_index_b.tex_mtx_idx(texgen_idx) as usize
        };
        let tex_mtx_base = tex_mtx_idx * XF_POS_MTX_STRIDE;

        // Multiply input by texture matrix (2x4 or 3x4 depending on projection)
        let (s, t, q) = match tg.projection() {
            regs::TexGenProjection::St => {
                // 2x4 matrix -> (s, t)
                let s = self.xf_f32(tex_mtx_base) * input[0]
                    + self.xf_f32(tex_mtx_base + 1) * input[1]
                    + self.xf_f32(tex_mtx_base + 2) * input[2]
                    + self.xf_f32(tex_mtx_base + 3) * input[3];
                let t = self.xf_f32(tex_mtx_base + 4) * input[0]
                    + self.xf_f32(tex_mtx_base + 5) * input[1]
                    + self.xf_f32(tex_mtx_base + 6) * input[2]
                    + self.xf_f32(tex_mtx_base + 7) * input[3];
                (s, t, 1.0)
            }
            regs::TexGenProjection::Stq => {
                // 3x4 matrix -> (s, t, q)
                let s = self.xf_f32(tex_mtx_base) * input[0]
                    + self.xf_f32(tex_mtx_base + 1) * input[1]
                    + self.xf_f32(tex_mtx_base + 2) * input[2]
                    + self.xf_f32(tex_mtx_base + 3) * input[3];
                let t = self.xf_f32(tex_mtx_base + 4) * input[0]
                    + self.xf_f32(tex_mtx_base + 5) * input[1]
                    + self.xf_f32(tex_mtx_base + 6) * input[2]
                    + self.xf_f32(tex_mtx_base + 7) * input[3];
                let q = self.xf_f32(tex_mtx_base + 8) * input[0]
                    + self.xf_f32(tex_mtx_base + 9) * input[1]
                    + self.xf_f32(tex_mtx_base + 10) * input[2]
                    + self.xf_f32(tex_mtx_base + 11) * input[3];
                (s, t, q)
            }
        };

        // Dual texgen post-transform (normalization + post-matrix multiply)
        let dual_tex_enabled = self.xf_mem[XF_DUAL_TEX_ENABLE] != 0;
        let (s, t, q) = if dual_tex_enabled {
            let post_base = XF_POST_MTX_BASE + dt.post_mtx_idx() as usize * 4;
            // Normalize (s, t, q) only if dt.normalize() is set
            let (ns, nt, nq) = if dt.normalize() {
                let inv_q = if q.abs() > f32::EPSILON { 1.0 / q } else { 1.0 };
                (s * inv_q, t * inv_q, inv_q)
            } else {
                (s, t, q)
            };
            // Post-transform: 3x4 matrix multiply on (ns, nt, nq)
            let ps = self.xf_f32(post_base) * ns
                + self.xf_f32(post_base + 1) * nt
                + self.xf_f32(post_base + 2) * nq
                + self.xf_f32(post_base + 3);
            let pt = self.xf_f32(post_base + 4) * ns
                + self.xf_f32(post_base + 5) * nt
                + self.xf_f32(post_base + 6) * nq
                + self.xf_f32(post_base + 7);
            let pq = self.xf_f32(post_base + 8) * ns
                + self.xf_f32(post_base + 9) * nt
                + self.xf_f32(post_base + 10) * nq
                + self.xf_f32(post_base + 11);
            (ps, pt, pq)
        } else {
            (s, t, q)
        };

        // When q is 0, the GameCube has a special case (Dolphin VertexShaderGen.cpp)
        if q.abs() < f32::EPSILON {
            [(s / 2.0).clamp(-1.0, 1.0), (t / 2.0).clamp(-1.0, 1.0)]
        } else {
            [s / q, t / q]
        }
    }

    fn xf_f32(&self, reg: usize) -> f32 {
        f32::from_bits(self.xf_mem[reg])
    }

    fn xf_vec3(&self, reg: usize) -> Vec3 {
        Vec3(self.xf_f32(reg), self.xf_f32(reg + 1), self.xf_f32(reg + 2))
    }

    fn s11_to_f32(val: u16) -> f32 {
        let signed = if val & 0x400 != 0 {
            val as i32 - 0x800
        } else {
            val as i32
        };
        signed as f32 / 255.0
    }

    /// Resolve TEV color registers (lo+hi) into [r,g,b,a] float arrays
    fn resolve_tev_orders(&self) -> [regs::TevStageOrder; 16] {
        let mut orders = [regs::TevStageOrder::default(); 16];
        for i in 0..8 {
            let reg = self.cur_tev_orders[i];
            orders[2 * i] = regs::TevStageOrder::default()
                .with_texmap(reg.texmap0())
                .with_texcoord(reg.texcoord0())
                .with_tex_enable(reg.tex_enable0());
            orders[2 * i + 1] = regs::TevStageOrder::default()
                .with_texmap(reg.texmap1())
                .with_texcoord(reg.texcoord1())
                .with_tex_enable(reg.tex_enable1());
        }
        orders
    }

    fn resolve_tev_color_regs(&self) -> [[f32; 4]; 4] {
        std::array::from_fn(|i| {
            let lo = self.cur_tev_color_regs_lo[i];
            let hi = self.cur_tev_color_regs_hi[i];
            [
                Self::s11_to_f32(lo.r()),
                Self::s11_to_f32(hi.g()),
                Self::s11_to_f32(hi.b()),
                Self::s11_to_f32(lo.a()),
            ]
        })
    }

    /// Resolve per-stage Konst colors from KSEL registers + Konst color registers
    fn resolve_konst_colors(&mut self) {
        let kregs: [[f32; 4]; 4] = std::array::from_fn(|i| {
            let lo = self.cur_tev_const_regs_lo[i];
            let hi = self.cur_tev_const_regs_hi[i];
            [
                Self::s11_to_f32(lo.r()),
                Self::s11_to_f32(hi.g()),
                Self::s11_to_f32(hi.b()),
                Self::s11_to_f32(lo.a()),
            ]
        });

        for stage in 0..16usize {
            let ksel_reg = self.bp_regs[BP_TEV_KSEL_0 + stage / 2];
            let (kcsel, kasel) = if stage % 2 == 0 {
                ((ksel_reg >> 4) & 0x1F, (ksel_reg >> 9) & 0x1F)
            } else {
                ((ksel_reg >> 14) & 0x1F, (ksel_reg >> 19) & 0x1F)
            };

            let rgb = Self::resolve_kcsel(kcsel, &kregs);
            let a = Self::resolve_kasel(kasel, &kregs);
            self.cur_tev_konst_colors[stage] = [rgb[0], rgb[1], rgb[2], a];
        }
    }

    fn resolve_kcsel(sel: u32, kregs: &[[f32; 4]; 4]) -> [f32; 3] {
        match sel {
            0 => [1.0, 1.0, 1.0],                          // 1
            1 => [0.875, 0.875, 0.875],                    // 7/8
            2 => [0.75, 0.75, 0.75],                       // 3/4
            3 => [0.625, 0.625, 0.625],                    // 5/8
            4 => [0.5, 0.5, 0.5],                          // 1/2
            5 => [0.375, 0.375, 0.375],                    // 3/8
            6 => [0.25, 0.25, 0.25],                       // 1/4
            7 => [0.125, 0.125, 0.125],                    // 1/8
            12 => [kregs[0][0], kregs[0][1], kregs[0][2]], // K0.RGB
            13 => [kregs[1][0], kregs[1][1], kregs[1][2]], // K1.RGB
            14 => [kregs[2][0], kregs[2][1], kregs[2][2]], // K2.RGB
            15 => [kregs[3][0], kregs[3][1], kregs[3][2]], // K3.RGB
            16 => [kregs[0][0]; 3],                        // K0.RRR
            17 => [kregs[1][0]; 3],                        // K1.RRR
            18 => [kregs[2][0]; 3],                        // K2.RRR
            19 => [kregs[3][0]; 3],                        // K3.RRR
            20 => [kregs[0][1]; 3],                        // K0.GGG
            21 => [kregs[1][1]; 3],                        // K1.GGG
            22 => [kregs[2][1]; 3],                        // K2.GGG
            23 => [kregs[3][1]; 3],                        // K3.GGG
            24 => [kregs[0][2]; 3],                        // K0.BBB
            25 => [kregs[1][2]; 3],                        // K1.BBB
            26 => [kregs[2][2]; 3],                        // K2.BBB
            27 => [kregs[3][2]; 3],                        // K3.BBB
            28 => [kregs[0][3]; 3],                        // K0.AAA
            29 => [kregs[1][3]; 3],                        // K1.AAA
            30 => [kregs[2][3]; 3],                        // K2.AAA
            31 => [kregs[3][3]; 3],                        // K3.AAA
            _ => [0.0, 0.0, 0.0],
        }
    }

    fn resolve_kasel(sel: u32, kregs: &[[f32; 4]; 4]) -> f32 {
        match sel {
            0 => 1.0,          // 1
            1 => 0.875,        // 7/8
            2 => 0.75,         // 3/4
            3 => 0.625,        // 5/8
            4 => 0.5,          // 1/2
            5 => 0.375,        // 3/8
            6 => 0.25,         // 1/4
            7 => 0.125,        // 1/8
            16 => kregs[0][0], // K0.R
            17 => kregs[1][0], // K1.R
            18 => kregs[2][0], // K2.R
            19 => kregs[3][0], // K3.R
            20 => kregs[0][1], // K0.G
            21 => kregs[1][1], // K1.G
            22 => kregs[2][1], // K2.G
            23 => kregs[3][1], // K3.G
            24 => kregs[0][2], // K0.B
            25 => kregs[1][2], // K1.B
            26 => kregs[2][2], // K2.B
            27 => kregs[3][2], // K3.B
            28 => kregs[0][3], // K0.A
            29 => kregs[1][3], // K1.A
            30 => kregs[2][3], // K2.A
            31 => kregs[3][3], // K3.A
            _ => 0.0,
        }
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

            self.cur_textures[slot] = Some(draw::TextureDescriptor {
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

        // Forward PE render state
        match idx {
            BP_PE_ZMODE => self.cur_zmode = ZMode::from_raw(val),
            BP_PE_CMODE0 => self.cur_blend_mode = BlendMode::from_raw(val),
            BP_PE_ALPHA_COMPARE => self.cur_alpha_compare = AlphaCompare::from_raw(val),
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
                self.cur_tev_color_env[stage] = env;
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
                self.cur_tev_alpha_env[stage] = env;
            }
        }

        // TEV rasterizer order registers (RAS1_TREF0-7, 0x28-0x2F)
        if idx >= BP_RAS1_TREF0 && idx < BP_RAS1_TREF0 + BP_RAS1_TREF_COUNT {
            self.cur_tev_orders[idx - BP_RAS1_TREF0] = regs::TevOrder::from_raw(val);
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
                    TevRegType::Color => self.cur_tev_color_regs_lo[reg_idx] = reg,
                    TevRegType::Constant => self.cur_tev_const_regs_lo[reg_idx] = reg,
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
                    TevRegType::Color => self.cur_tev_color_regs_hi[reg_idx] = reg,
                    TevRegType::Constant => self.cur_tev_const_regs_hi[reg_idx] = reg,
                }
            }
        }

        // GEN_MODE, extract num TEV stages
        if idx == BP_GEN_MODE {
            let gen_mode = GenMode::from_raw(val);
            let stages = gen_mode.num_tev_stages() + 1;
            tracing::debug!(num_tev_stages = stages, "GEN_MODE");
            self.cur_num_tev_stages = stages;
            self.resolve_konst_colors();
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
