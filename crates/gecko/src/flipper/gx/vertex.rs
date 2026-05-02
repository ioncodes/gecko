use super::constants::*;
use super::math::{Vec3, unpack_rgba};
use super::regs::{self, *};
use super::{GraphicsProcessor, draw};
use crate::host::{DrawData, DrawVertex, GxAction, LightData, RenderSink};
use crate::mmio::{Mmio, RamView};
use crate::system::SystemId;
use std::io::{Cursor, Read};

/// Parsed vertex format descriptor from CP/VAT registers.
struct VertexFormat {
    vat_a: VatA,
    pos_base: usize,
    pos_stride: usize,
    pos_data_size: usize,
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
    has_tex_mtx_idx: [bool; 8],
    default_pos_mtx_idx: u8,
    default_tex_mtx_idx: [u8; 8],
    pos_attr: AttributeType,
    nrm_attr: AttributeType,
    clr0_attr: AttributeType,
    clr0_base: usize,
    clr0_stride: usize,
    clr0_data_size: usize,
    clr1_attr: AttributeType,
    clr1_base: usize,
    clr1_stride: usize,
    clr1_data_size: usize,
}

impl GraphicsProcessor {
    pub fn create_draw_call<const SYSTEM: SystemId>(
        &mut self,
        mmio: &mut Mmio<SYSTEM>,
        renderer: &mut dyn RenderSink,
        cmd: u8,
        data: Vec<u8>,
    ) {
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

        let mut vertices: Vec<draw::Vertex> = Vec::with_capacity(vertex_count);
        let mut cur = Cursor::new(&data);

        let view = mmio.ram_view();
        for i in 0..vertex_count {
            let vertex = self.decode_vertex(&mut cur, &data, &view, &vf, i);
            vertices.push(vertex);
        }

        let modelview = self.build_modelview_matrix(vf.default_pos_mtx_idx);

        tracing::debug!(
            primitive = format!("{:?}", primitive),
            vertices = format!("{:?}", vertices),
            pos_mtx_idx = vf.default_pos_mtx_idx,
            modelview = format!("{:?}", modelview),
            projection = format!("{:?}", self.projection),
            "draw call created"
        );

        // Resolve TEV color registers to f32 arrays for the snapshot
        let tev_color_regs = self.resolve_tev_color_regs();
        let tev_orders = self.resolve_tev_orders();

        // Snapshot light data for the action stream
        let light_colors = self.snapshot_light_field(XF_LIGHT_COLOR);
        let light_cosatt = self.snapshot_light_field(XF_LIGHT_A0);
        let light_distatt = self.snapshot_light_field(XF_LIGHT_K0);
        let light_pos = self.snapshot_light_field(XF_LIGHT_PX);
        let light_dir = self.snapshot_light_field(XF_LIGHT_NX);

        let color_ctrl = [
            regs::ChanCtrl::from_raw(self.xf_mem[XF_COLOR_CTRL0]),
            regs::ChanCtrl::from_raw(self.xf_mem[XF_COLOR_CTRL1]),
        ];
        let alpha_ctrl = [
            regs::ChanCtrl::from_raw(self.xf_mem[XF_ALPHA_CTRL0]),
            regs::ChanCtrl::from_raw(self.xf_mem[XF_ALPHA_CTRL1]),
        ];
        let ambient_color = [
            unpack_rgba(self.xf_mem[XF_AMBIENT_COLOR0]),
            unpack_rgba(self.xf_mem[XF_AMBIENT_COLOR1]),
        ];
        let material_color = [
            unpack_rgba(self.xf_mem[XF_MATERIAL_COLOR0]),
            unpack_rgba(self.xf_mem[XF_MATERIAL_COLOR1]),
        ];

        // Emit action to the render sink
        let draw_vertices: Vec<DrawVertex> = vertices
            .iter()
            .map(|v| DrawVertex {
                position: v.position,
                normal: v.normal,
                color0: v.color0,
                color1: v.color1,
                pos_view: v.pos_view,
                texcoords: std::array::from_fn(|i| v.texcoords[i].unwrap_or([0.0, 0.0, 1.0])),
            })
            .collect();

        let lights: [LightData; 8] = std::array::from_fn(|i| LightData {
            color: light_colors[i],
            cosatt: light_cosatt[i],
            distatt: light_distatt[i],
            position: light_pos[i],
            direction: light_dir[i],
        });

        // Flatten the three indirect matrices to 6 rows. Row 2M is row
        // 0 of matrix M, row 2M+1 is row 1. The .w lane carries the
        // shared scale exponent (positive means right-shift, negative
        // means left-shift of the mat-mul result).
        let mut indirect_matrices = [[0i32; 4]; 6];
        for m in 0..3 {
            let mtx = &self.cur_indirect_matrices[m];
            let exp = mtx.scale_exponent();
            let r0 = mtx.row0();
            let r1 = mtx.row1();
            indirect_matrices[2 * m] = [r0[0], r0[1], r0[2], exp];
            indirect_matrices[2 * m + 1] = [r1[0], r1[1], r1[2], exp];
        }

        let indirect_scales = [
            [
                self.cur_indirect_scales[0].ss0() as u32,
                self.cur_indirect_scales[0].ts0() as u32,
                self.cur_indirect_scales[0].ss1() as u32,
                self.cur_indirect_scales[0].ts1() as u32,
            ],
            [
                self.cur_indirect_scales[1].ss0() as u32,
                self.cur_indirect_scales[1].ts0() as u32,
                self.cur_indirect_scales[1].ss1() as u32,
                self.cur_indirect_scales[1].ts1() as u32,
            ],
        ];

        renderer.exec(GxAction::Draw(DrawData {
            primitive,
            vertices: draw_vertices,
            modelview: modelview.0,
            tev_color_env: self.cur_tev_color_env.iter().map(|e| e.raw()).collect(),
            tev_alpha_env: self.cur_tev_alpha_env.iter().map(|e| e.raw()).collect(),
            tev_orders: tev_orders.iter().map(|o| o.raw()).collect(),
            tev_ksel: (0..8).map(|i| self.bp_regs[BP_TEV_KSEL_0 + i]).collect(),
            tev_color_regs,
            tev_konst_colors: self.cur_tev_konst_colors,
            num_tev_stages: self.cur_num_tev_stages,
            indirect_matrices,
            indirect_scales,
            indirect_refs: self.cur_indirect_refs.raw(),
            num_indirect_stages: self.cur_num_indirect_stages,
            bump_imask: self.cur_bump_imask,
            tev_indirect: self.cur_tev_indirect.iter().map(|c| c.raw()).collect(),
            color_ctrl,
            alpha_ctrl,
            ambient_color,
            material_color,
            lights,
        }));
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
        let clr1_attr = vcd_lo.color1();
        let clr1_data_size = vat_a.clr1_data_size();
        let clr1_stream_size = attr_stream_size(clr1_attr, clr1_data_size);

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
        let has_tex_mtx_idx = [
            vcd_lo.tex0_mtx_idx(),
            vcd_lo.tex1_mtx_idx(),
            vcd_lo.tex2_mtx_idx(),
            vcd_lo.tex3_mtx_idx(),
            vcd_lo.tex4_mtx_idx(),
            vcd_lo.tex5_mtx_idx(),
            vcd_lo.tex6_mtx_idx(),
            vcd_lo.tex7_mtx_idx(),
        ];

        let mtx_index_a = MatrixIndex0::from_raw(self.xf_mem[XF_MATRIX_INDEX_A]);
        let mtx_index_b = MatrixIndex1::from_raw(self.xf_mem[XF_MATRIX_INDEX_B]);
        let default_pos_mtx_idx = mtx_index_a.pos_mtx_idx();
        let default_tex_mtx_idx = [
            mtx_index_a.tex_mtx_idx(0),
            mtx_index_a.tex_mtx_idx(1),
            mtx_index_a.tex_mtx_idx(2),
            mtx_index_a.tex_mtx_idx(3),
            mtx_index_b.tex_mtx_idx(4),
            mtx_index_b.tex_mtx_idx(5),
            mtx_index_b.tex_mtx_idx(6),
            mtx_index_b.tex_mtx_idx(7),
        ];

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
            has_tex_mtx_idx,
            default_pos_mtx_idx,
            default_tex_mtx_idx,
            pos_attr,
            nrm_attr,
            clr0_attr,
            clr1_attr,
            clr1_base: self.cp_regs[ARRAY_BASE_REG + ARRAY_CLR1] as usize,
            clr1_stride: self.cp_regs[ARRAY_STRIDE_REG + ARRAY_CLR1] as usize,
            clr1_data_size,
        }
    }

    fn decode_vertex(
        &self,
        cur: &mut Cursor<&Vec<u8>>,
        data: &[u8],
        ram: &RamView<'_>,
        vf: &VertexFormat,
        vertex_idx: usize,
    ) -> draw::Vertex {
        // Resolve an indexed vertex attribute (positions, normals, colors,
        // texcoords) to a slice in whichever bank holds it. If the address
        // falls outside both MEM1 and MEM2, fall back to a zero-filled
        // buffer.
        fn fetch<'a>(view: &'a RamView<'_>, addr: usize, len: usize, scratch: &'a mut [u8]) -> &'a [u8] {
            if let Some(s) = view.slice(addr, len) {
                s
            } else {
                tracing::warn!(
                    addr = format!("{addr:#010X}"),
                    len,
                    "vertex attribute fetch unmapped, using zeros"
                );
                let n = len.min(scratch.len());
                scratch[..n].fill(0);
                &scratch[..n]
            }
        }

        let mut scratch = [0u8; 32];
        // Read per-vertex position/normal matrix index, or use register default
        let pos_mtx_idx = if vf.has_pnmtxidx {
            let mut buf = [0u8; 1];
            cur.read_exact(&mut buf).unwrap();
            buf[0] & 0x3F
        } else {
            vf.default_pos_mtx_idx
        };

        // Read per-vertex texture matrix indices, or use register defaults
        let mut tex_mtx_idx = vf.default_tex_mtx_idx;
        for i in 0..8 {
            if vf.has_tex_mtx_idx[i] {
                let mut buf = [0u8; 1];
                cur.read_exact(&mut buf).unwrap();
                tex_mtx_idx[i] = buf[0];
            }
        }

        // Derive per-vertex position and normal matrix bases
        let pos_mtx_base = pos_mtx_idx as usize * XF_POS_MTX_STRIDE;
        let nrm_mtx_idx = (pos_mtx_idx as usize) & 31;
        let nrm_mtx_base = XF_NRM_MTX_BASE + nrm_mtx_idx * 3;
        let nrm_mtx: [f32; 9] = std::array::from_fn(|i| f32::from_bits(self.xf_mem[nrm_mtx_base + i]));

        // Read position
        let position = if vf.pos_attr == AttributeType::Direct {
            let start = cur.position() as usize;
            cur.set_position(cur.position() + vf.pos_data_size as u64);
            decode_position(&data[start..start + vf.pos_data_size], &vf.vat_a)
        } else {
            let pos_index = read_index(cur, vf.pos_attr);
            let pos_addr = vf.pos_base + pos_index * vf.pos_stride;
            decode_position(fetch(ram, pos_addr, vf.pos_data_size, &mut scratch), &vf.vat_a)
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
            decode_normal(fetch(ram, nrm_addr, vf.nrm_data_size, &mut scratch), &vf.vat_a)
        } else {
            [0.0, 0.0, 1.0]
        };

        // Read color0
        let color0 = if vf.clr0_attr == AttributeType::None {
            [1.0, 1.0, 1.0, 1.0]
        } else if vf.clr0_attr == AttributeType::Direct {
            let start = cur.position() as usize;
            cur.set_position(cur.position() + vf.clr0_data_size as u64);
            decode_color(
                &data[start..start + vf.clr0_data_size],
                vf.vat_a.clr0_fmt(),
                vf.vat_a.clr0_cnt(),
            )
        } else {
            let clr0_index = read_index(cur, vf.clr0_attr);
            let clr0_addr = vf.clr0_base + clr0_index * vf.clr0_stride;
            decode_color(
                fetch(ram, clr0_addr, vf.clr0_data_size, &mut scratch),
                vf.vat_a.clr0_fmt(),
                vf.vat_a.clr0_cnt(),
            )
        };

        // Read color1
        let color1 = if vf.clr1_attr == AttributeType::None {
            [1.0, 1.0, 1.0, 1.0]
        } else if vf.clr1_attr == AttributeType::Direct {
            let start = cur.position() as usize;
            cur.set_position(cur.position() + vf.clr1_data_size as u64);
            decode_color(
                &data[start..start + vf.clr1_data_size],
                vf.vat_a.clr1_fmt(),
                vf.vat_a.clr1_cnt(),
            )
        } else {
            let clr1_index = read_index(cur, vf.clr1_attr);
            let clr1_addr = vf.clr1_base + clr1_index * vf.clr1_stride;
            decode_color(
                fetch(ram, clr1_addr, vf.clr1_data_size, &mut scratch),
                vf.vat_a.clr1_fmt(),
                vf.vat_a.clr1_cnt(),
            )
        };

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
                    fetch(ram, tc_addr, tc_data_size, &mut scratch),
                    vf.tex_fmts[tc],
                    vf.tex_shifts[tc],
                    vf.tex_cnts[tc],
                )
            });
        }

        // Transform position and normal to view space (per-vertex matrix dependent)
        let normal_view = Vec3::from(normal).transform(&nrm_mtx).normalize();
        let pos_view = self.xf_transform_3x4(pos_mtx_base, position);

        // Texture coordinate generation (XF texgen)
        // compute_texgen now returns [f32; 3] (s, t, q) with perspective
        // divide deferred to the fragment shader.
        let num_texgens = (self.xf_mem[XF_NUM_TEXGENS] as usize).min(8);
        let mut texcoords: [Option<[f32; 3]>; 8] = [None; 8];
        for tg_idx in 0..num_texgens {
            texcoords[tg_idx] = Some(self.compute_texgen(tg_idx, position, normal, &raw_texcoords, &tex_mtx_idx));
        }
        // For texcoords beyond num_texgens, pass through raw values with q=1
        for tg_idx in num_texgens..8 {
            texcoords[tg_idx] = raw_texcoords[tg_idx].map(|st| [st[0], st[1], 1.0]);
        }

        tracing::debug!(
            vertex = vertex_idx,
            position = format!("{:02X?}", position),
            color0 = format!("{:?}", color0),
            "Vertex"
        );

        draw::Vertex {
            position,
            color0,
            color1,
            normal: [normal_view.0, normal_view.1, normal_view.2],
            pos_view: [pos_view.0, pos_view.1, pos_view.2],
            texcoords,
        }
    }

    fn build_modelview_matrix(&self, pos_mtx_idx: u8) -> draw::Matrix4 {
        let default_mtx_base = pos_mtx_idx as usize * XF_POS_MTX_STRIDE;
        draw::Matrix4([
            [
                f32::from_bits(self.xf_mem[default_mtx_base]),
                f32::from_bits(self.xf_mem[default_mtx_base + 4]),
                f32::from_bits(self.xf_mem[default_mtx_base + 8]),
                0.0,
            ],
            [
                f32::from_bits(self.xf_mem[default_mtx_base + 1]),
                f32::from_bits(self.xf_mem[default_mtx_base + 5]),
                f32::from_bits(self.xf_mem[default_mtx_base + 9]),
                0.0,
            ],
            [
                f32::from_bits(self.xf_mem[default_mtx_base + 2]),
                f32::from_bits(self.xf_mem[default_mtx_base + 6]),
                f32::from_bits(self.xf_mem[default_mtx_base + 10]),
                0.0,
            ],
            [
                f32::from_bits(self.xf_mem[default_mtx_base + 3]),
                f32::from_bits(self.xf_mem[default_mtx_base + 7]),
                f32::from_bits(self.xf_mem[default_mtx_base + 11]),
                1.0,
            ],
        ])
    }

    fn snapshot_light_field(&self, field_offset: usize) -> [[f32; 4]; 8] {
        std::array::from_fn(|i| {
            let base = XF_LIGHT_BASE + i * XF_LIGHT_STRIDE + field_offset;
            if field_offset == XF_LIGHT_COLOR {
                // Color is stored as packed RGBA, not float
                unpack_rgba(self.xf_mem[base])
            } else {
                // Float vec3, w = 0
                [
                    f32::from_bits(self.xf_mem[base]),
                    f32::from_bits(self.xf_mem[base + 1]),
                    f32::from_bits(self.xf_mem[base + 2]),
                    0.0,
                ]
            }
        })
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

fn decode_color(data: &[u8], fmt: regs::ColorFormat, cnt: regs::ColorCount) -> [f32; 4] {
    let has_alpha = cnt == regs::ColorCount::Rgba;
    match fmt {
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
            let a = if has_alpha { (raw & 0xF) as f32 / 15.0 } else { 1.0 };
            [r, g, b, a]
        }
        regs::ColorFormat::Rgba6 => {
            let raw = u32::from_be_bytes([0, data[0], data[1], data[2]]);
            let r = ((raw >> 18) & 0x3F) as f32 / 63.0;
            let g = ((raw >> 12) & 0x3F) as f32 / 63.0;
            let b = ((raw >> 6) & 0x3F) as f32 / 63.0;
            let a = if has_alpha { (raw & 0x3F) as f32 / 63.0 } else { 1.0 };
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

    // Hardware uses fixed-point: 6 fractional bits for byte types, 14 for word types.
    const SHIFT_BYTE: f32 = (1u32 << 6) as f32; // 64.0
    const SHIFT_WORD: f32 = (1u32 << 14) as f32; // 16384.0

    for i in 0..cnt {
        result[i] = match fmt {
            ComponentFormat::U8 => {
                let v = data[off] as f32 / SHIFT_BYTE;
                off += 1;
                v
            }
            ComponentFormat::S8 => {
                let v = data[off] as i8 as f32 / SHIFT_BYTE;
                off += 1;
                v
            }
            ComponentFormat::U16 => {
                let v = u16::from_be_bytes([data[off], data[off + 1]]) as f32 / SHIFT_WORD;
                off += 2;
                v
            }
            ComponentFormat::S16 => {
                let v = i16::from_be_bytes([data[off], data[off + 1]]) as f32 / SHIFT_WORD;
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
