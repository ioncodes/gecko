use super::{
    GraphicsProcessor,
    constants::{
        ARRAY_BASE_REG, ARRAY_CLR0, ARRAY_NRM, ARRAY_POS, ARRAY_STRIDE_REG, ARRAY_TEX0, VATA_REG, VATB_REG, VATC_REG,
        VCD_HI_REG, VCD_LO_REG, XF_ALPHA_CTRL0, XF_AMBIENT_COLOR0, XF_COLOR_CTRL0, XF_MATERIAL_COLOR0,
        XF_MATRIX_INDEX_A, XF_NRM_MTX_BASE, XF_NUM_TEXGENS, XF_POS_MTX_STRIDE,
    },
    draw,
    math::{Vec3, unpack_rgba},
    regs::{self, AttributeType, ChanCtrl, ComponentFormat, MatrixIndex0, TexCount, VatA, VatB, VatC, VcdHi, VcdLo},
};
use crate::mmio::Mmio;
use std::io::{Cursor, Read};

/// Parsed vertex format descriptor from CP/VAT registers.
struct VertexFormat {
    vat_a: VatA,
    pos_base: usize,
    pos_stride: usize,
    pos_data_size: usize,
    clr0_base: usize,
    clr0_stride: usize,
    clr0_data_size: usize,
    nrm_base_addr: usize,
    nrm_stride: usize,
    nrm_data_size: usize,
    tex_attrs: [AttributeType; 8],
    tex_data_sizes: [usize; 8],
    tex_fmts: [ComponentFormat; 8],
    tex_shifts: [u8; 8],
    tex_cnts: [TexCount; 8],
    tex_bases: [usize; 8],
    tex_strides: [usize; 8],
    vertex_stride: usize,
    has_pnmtxidx: bool,
    tex_mtx_idx_size: usize,
    default_pos_mtx_idx: u8,
    pos_attr: AttributeType,
    nrm_attr: AttributeType,
    clr0_attr: AttributeType,
    clr1_stream_size: usize,
}

impl GraphicsProcessor {
    pub(crate) fn create_draw_call(&mut self, mmio: &mut Mmio, cmd: u8, data: Vec<u8>) {
        let Some(primitive) = draw::Primitive::from_cmd(cmd) else {
            tracing::error!(cmd, "goofy draw command");
            return;
        };

        let vf = self.build_vertex_format(cmd);

        if vf.vertex_stride == 0 {
            tracing::warn!("draw call with zero vertex stride, skipping");
            return;
        }

        let vertex_count = data.len() / vf.vertex_stride;

        // Channel 0 lighting state (COLOR0 and ALPHA0 have separate controls)
        let color_ctrl = ChanCtrl::from_raw(self.xf_mem[XF_COLOR_CTRL0]);
        let alpha_ctrl = ChanCtrl::from_raw(self.xf_mem[XF_ALPHA_CTRL0]);
        let ambient_reg = unpack_rgba(self.xf_mem[XF_AMBIENT_COLOR0]);
        let material_reg = unpack_rgba(self.xf_mem[XF_MATERIAL_COLOR0]);

        let mut vertices: Vec<draw::Vertex> = Vec::with_capacity(vertex_count);
        let mut cur = Cursor::new(&data);

        for i in 0..vertex_count {
            let vertex = self.decode_vertex(
                &mut cur,
                &data,
                &mmio.ram,
                &vf,
                &color_ctrl,
                &alpha_ctrl,
                ambient_reg,
                material_reg,
                i,
            );
            vertices.push(vertex);
        }

        let modelview = self.build_modelview_matrix(vf.default_pos_mtx_idx);

        tracing::debug!(
            primitive = format!("{:?}", primitive),
            vertices = format!("{:?}", vertices),
            pos_mtx_idx = vf.default_pos_mtx_idx,
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

    fn build_vertex_format(&self, cmd: u8) -> VertexFormat {
        let fmt = (cmd & 0b111) as usize;
        let vcd_lo = VcdLo::from_raw(self.cp_regs[VCD_LO_REG]);
        let vcd_hi = VcdHi::from_raw(self.cp_regs[VCD_HI_REG]);
        let vat_a = VatA::from_raw(self.cp_regs[VATA_REG + fmt]);
        let vat_b = VatB::from_raw(self.cp_regs[VATB_REG + fmt]);
        let vat_c = VatC::from_raw(self.cp_regs[VATC_REG + fmt]);

        let attr_stream_size = |attr: AttributeType, direct_size: usize| -> usize {
            match attr {
                AttributeType::Direct => direct_size,
                AttributeType::Index8 => 1,
                AttributeType::Index16 => 2,
                AttributeType::None => 0,
            }
        };

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

        let has_pnmtxidx = vcd_lo.pos_nrm_mtx_idx();
        let tex_mtx_idx_size = mtx_idx_size - has_pnmtxidx as usize;

        let mtx_index_a = MatrixIndex0::from_raw(self.xf_mem[XF_MATRIX_INDEX_A]);
        let default_pos_mtx_idx = mtx_index_a.pos_mtx_idx();

        let vertex_stride = mtx_idx_size
            + pos_stream_size
            + nrm_stream_size
            + clr0_stream_size
            + clr1_stream_size
            + tex_stream_sizes.iter().sum::<usize>();

        VertexFormat {
            vat_a,
            pos_base: self.cp_regs[ARRAY_BASE_REG + ARRAY_POS] as usize,
            pos_stride: self.cp_regs[ARRAY_STRIDE_REG + ARRAY_POS] as usize,
            pos_data_size,
            clr0_base: self.cp_regs[ARRAY_BASE_REG + ARRAY_CLR0] as usize,
            clr0_stride: self.cp_regs[ARRAY_STRIDE_REG + ARRAY_CLR0] as usize,
            clr0_data_size,
            nrm_base_addr: self.cp_regs[ARRAY_BASE_REG + ARRAY_NRM] as usize,
            nrm_stride: self.cp_regs[ARRAY_STRIDE_REG + ARRAY_NRM] as usize,
            nrm_data_size,
            tex_attrs,
            tex_data_sizes,
            tex_fmts,
            tex_shifts,
            tex_cnts,
            tex_bases,
            tex_strides,
            vertex_stride,
            has_pnmtxidx,
            tex_mtx_idx_size,
            default_pos_mtx_idx,
            pos_attr,
            nrm_attr,
            clr0_attr,
            clr1_stream_size,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_vertex(
        &self,
        cur: &mut Cursor<&Vec<u8>>,
        data: &[u8],
        ram: &[u8],
        vf: &VertexFormat,
        color_ctrl: &ChanCtrl,
        alpha_ctrl: &ChanCtrl,
        ambient_reg: [f32; 4],
        material_reg: [f32; 4],
        vertex_idx: usize,
    ) -> draw::Vertex {
        // Read per-vertex position/normal matrix index, or use register default
        let pos_mtx_idx = if vf.has_pnmtxidx {
            let mut buf = [0u8; 1];
            cur.read_exact(&mut buf).unwrap();
            buf[0] & 0x3F
        } else {
            vf.default_pos_mtx_idx
        };

        // Skip remaining tex matrix index bytes (not yet used)
        cur.set_position(cur.position() + vf.tex_mtx_idx_size as u64);

        // Derive per-vertex position and normal matrix bases
        let pos_mtx_base = pos_mtx_idx as usize * XF_POS_MTX_STRIDE;
        let nrm_mtx_idx = (pos_mtx_idx as usize) & 31;
        let nrm_mtx_base = XF_NRM_MTX_BASE + nrm_mtx_idx * 3;
        let nrm_mtx: [f32; 9] = std::array::from_fn(|i| self.xf_f32(nrm_mtx_base + i));

        // Read position
        let position = if vf.pos_attr == AttributeType::Direct {
            let start = cur.position() as usize;
            cur.set_position(cur.position() + vf.pos_data_size as u64);
            decode_position(&data[start..start + vf.pos_data_size], &vf.vat_a)
        } else {
            let pos_index = read_index(cur, vf.pos_attr);
            let pos_addr = vf.pos_base + pos_index * vf.pos_stride;
            decode_position(&ram[pos_addr..pos_addr + vf.pos_data_size], &vf.vat_a)
        };

        // Read normal
        let normal = if vf.nrm_attr == AttributeType::Direct {
            let start = cur.position() as usize;
            cur.set_position(cur.position() + vf.nrm_data_size as u64);
            decode_normal(&data[start..start + vf.nrm_data_size], &vf.vat_a)
        } else if vf.nrm_attr != AttributeType::None {
            let nrm_index = read_index(cur, vf.nrm_attr);
            // When nrm_index3 && NBT, the stream contains 3 separate indices
            // (normal, binormal, tangent). Skip the extra 2, only the normal
            // vector (first index) is needed for lighting.
            if vf.vat_a.nrm_index3() && vf.vat_a.nrm_cnt() == regs::NrmCount::Nbt {
                let idx_size = if vf.nrm_attr == AttributeType::Index8 { 1 } else { 2 };
                cur.set_position(cur.position() + (2 * idx_size) as u64);
            }
            let nrm_addr = vf.nrm_base_addr + nrm_index * vf.nrm_stride;
            decode_normal(&ram[nrm_addr..nrm_addr + vf.nrm_data_size], &vf.vat_a)
        } else {
            [0.0, 0.0, 1.0]
        };

        // Read color0
        let color0 = if vf.clr0_attr == AttributeType::None {
            [1.0, 1.0, 1.0, 1.0]
        } else if vf.clr0_attr == AttributeType::Direct {
            let start = cur.position() as usize;
            cur.set_position(cur.position() + vf.clr0_data_size as u64);
            decode_color(&data[start..start + vf.clr0_data_size], &vf.vat_a)
        } else {
            let clr0_index = read_index(cur, vf.clr0_attr);
            let clr0_addr = vf.clr0_base + clr0_index * vf.clr0_stride;
            decode_color(&ram[clr0_addr..clr0_addr + vf.clr0_data_size], &vf.vat_a)
        };

        // Skip color1 (TODO: not yet used for rendering)
        cur.set_position(cur.position() + vf.clr1_stream_size as u64);

        // Read all texcoords (tex0-tex7)
        let mut raw_texcoords: [Option<[f32; 2]>; 8] = [None; 8];
        for tc in 0..8 {
            let tc_attr = vf.tex_attrs[tc];
            let tc_data_size = vf.tex_data_sizes[tc];
            if tc_attr == AttributeType::None {
                continue;
            }
            raw_texcoords[tc] = Some(if tc_attr == AttributeType::Direct {
                let start = cur.position() as usize;
                cur.set_position(cur.position() + tc_data_size as u64);
                decode_texcoord(
                    &data[start..start + tc_data_size],
                    vf.tex_fmts[tc],
                    vf.tex_shifts[tc],
                    vf.tex_cnts[tc],
                )
            } else {
                let tc_index = read_index(cur, tc_attr);
                let tc_addr = vf.tex_bases[tc] + tc_index * vf.tex_strides[tc];
                decode_texcoord(
                    &ram[tc_addr..tc_addr + tc_data_size],
                    vf.tex_fmts[tc],
                    vf.tex_shifts[tc],
                    vf.tex_cnts[tc],
                )
            });
        }

        // Compute per-vertex lighting
        let normal_view = Vec3::from(normal).transform(&nrm_mtx).normalize();
        let pos_view = self.xf_transform_3x4(pos_mtx_base, position);
        let final_color = self.compute_channel_lighting(
            color_ctrl,
            alpha_ctrl,
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
            vertex = vertex_idx,
            position = format!("{:02X?}", position),
            color0 = format!("{:?}", final_color),
            "Vertex"
        );

        draw::Vertex {
            position,
            color0: final_color,
            texcoords,
        }
    }

    fn build_modelview_matrix(&self, pos_mtx_idx: u8) -> draw::Matrix4 {
        let default_mtx_base = pos_mtx_idx as usize * XF_POS_MTX_STRIDE;
        draw::Matrix4([
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
        ])
    }
}

fn read_index(cur: &mut Cursor<&Vec<u8>>, attr: AttributeType) -> usize {
    match attr {
        AttributeType::Index8 => {
            let mut buf = [0u8; 1];
            cur.read_exact(&mut buf).unwrap();
            buf[0] as usize
        }
        AttributeType::Index16 => {
            let mut buf = [0u8; 2];
            cur.read_exact(&mut buf).unwrap();
            u16::from_be_bytes(buf) as usize
        }
        _ => 0,
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
            ComponentFormat::U8 => {
                let v = data[off] as f32 / divisor;
                off += 1;
                v
            }
            ComponentFormat::S8 => {
                let v = data[off] as i8 as f32 / divisor;
                off += 1;
                v
            }
            ComponentFormat::U16 => {
                let v = u16::from_be_bytes([data[off], data[off + 1]]) as f32 / divisor;
                off += 2;
                v
            }
            ComponentFormat::S16 => {
                let v = i16::from_be_bytes([data[off], data[off + 1]]) as f32 / divisor;
                off += 2;
                v
            }
            ComponentFormat::F32 => {
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
            ComponentFormat::U8 => {
                let v = data[off] as f32 / 255.0;
                off += 1;
                v
            }
            ComponentFormat::S8 => {
                let v = data[off] as i8 as f32 / 127.0;
                off += 1;
                v
            }
            ComponentFormat::U16 => {
                let v = u16::from_be_bytes([data[off], data[off + 1]]) as f32 / 65535.0;
                off += 2;
                v
            }
            ComponentFormat::S16 => {
                let v = i16::from_be_bytes([data[off], data[off + 1]]) as f32 / 32767.0;
                off += 2;
                v
            }
            ComponentFormat::F32 => {
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

fn decode_texcoord(data: &[u8], fmt: ComponentFormat, shift: u8, cnt: TexCount) -> [f32; 2] {
    let num = cnt.components();
    let divisor = (1u32 << shift) as f32;
    let mut result = [0.0f32; 2];
    let mut off = 0;

    for i in 0..num {
        result[i] = match fmt {
            ComponentFormat::U8 => {
                let v = data[off] as f32 / divisor;
                off += 1;
                v
            }
            ComponentFormat::S8 => {
                let v = data[off] as i8 as f32 / divisor;
                off += 1;
                v
            }
            ComponentFormat::U16 => {
                let v = u16::from_be_bytes([data[off], data[off + 1]]) as f32 / divisor;
                off += 2;
                v
            }
            ComponentFormat::S16 => {
                let v = i16::from_be_bytes([data[off], data[off + 1]]) as f32 / divisor;
                off += 2;
                v
            }
            ComponentFormat::F32 => {
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
