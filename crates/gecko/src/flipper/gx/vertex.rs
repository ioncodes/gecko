use super::constants::*;
use super::math::{Vec3, unpack_rgba};
use super::regs::{self, *};
use super::{GraphicsProcessor, draw};
use crate::host::{DrawSegment, DrawState, DrawVertex, RenderSink};
use crate::mmio::{Mmio, RamView};
#[cfg(feature = "jit")]
use crate::system::ExecutionMode;
use crate::system::SystemId;
use std::io::{Cursor, Read};

#[cfg(feature = "jit")]
use super::jit;

#[derive(Clone, Copy)]
pub(super) struct DrawBatchEntry {
    pub cmd: u8,
    pub vertex_count: usize,
}

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
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(super) fn create_draw_batch<const SYSTEM: SystemId>(
        &mut self,
        mmio: &mut Mmio<SYSTEM>,
        renderer: &mut dyn RenderSink,
        data: &[u8],
        entries: &[DrawBatchEntry],
    ) {
        #[cfg(feature = "gx-stats")]
        let _gx_stats_t0 = std::time::Instant::now();

        let Some(first) = entries.first() else {
            return;
        };

        let cmd = first.cmd;

        let vertex_format_index = (cmd & 0b111) as usize;
        let vertex_stride = super::fifo::vertex_stride_from_cp(&self.cp_regs, vertex_format_index);

        if vertex_stride == 0 {
            tracing::warn!("draw call with zero vertex stride, skipping");
            return;
        }

        let vertex_count: usize = entries.iter().map(|entry| entry.vertex_count).sum();
        debug_assert_eq!(data.len(), vertex_count * vertex_stride);
        debug_assert!(entries.iter().all(|entry| (entry.cmd & 7) == (cmd & 7)));

        // Resolve any texture slots whose descriptor regs changed since the
        // last draw. Done lazily here (not at BP write time) because games
        // write SETMODE0/SETIMAGE0-3 in arbitrary order; only at draw time is
        // the descriptor guaranteed consistent. Runs before the recorder so
        // recorded draws reference the resolved textures.
        if self.tex_dirty != 0 {
            self.defer_pending_efb_writebacks(mmio);

            if self.dirty_texture_overlaps_efb_writeback(mmio, renderer) {
                renderer.flush_efb_copies(&mut mmio.ram_view_mut());
                mmio.clear_deferred_efb_writebacks();
            }

            self.snapshot_dirty_textures(renderer, &mmio.ram_view());
        }

        if self.recorder.as_ref().is_some_and(|rec| rec.is_recording()) {
            let vf = self.build_vertex_format(cmd);
            let view = mmio.ram_view();
            let rec = self.recorder.as_deref_mut().unwrap();
            let mut offset = 0;

            for entry in entries {
                let len = entry.vertex_count * vertex_stride;
                self::record_draw_arrays(rec, &view, &vf, &data[offset..offset + len], entry.vertex_count);
                offset += len;
            }

            for desc in self.cur_textures.iter().flatten() {
                let len = super::texture::raw_data_size(desc.width, desc.height, desc.format);
                rec.use_draw_texture(&view, desc.ram_addr as u32, len);
            }
        }

        #[cfg(feature = "gx-stats")]
        {
            self.stats.draw_calls += entries.len() as u64;
            self.stats.vertices += vertex_count as u64;
            self.stats.fifo_bytes += data.len() as u64;

            for entry in entries {
                if let Some(primitive) = draw::Primitive::from_cmd(entry.cmd) {
                    self.stats.draws_by_primitive[(primitive as usize) & 0x7] += 1;
                }
            }
        }

        // Decode directly into the renderer's vertex scratch. No
        // intermediate `draw_vertices_scratch`, no append memcpy. The
        // interpreter pushes into `verts`, and the Cranelift JIT writes
        // through `verts.as_mut_ptr().add(base)` then sets the new length.
        let verts = renderer.vertex_scratch();
        let base_vertex = verts.len() as u32;
        verts.reserve(vertex_count);
        self::dispatch_decode(self, mmio, cmd, data, vertex_count, verts);

        let state = self.frame_state_dirty.then(|| {
            let tev_color_regs = self.resolve_tev_color_regs();
            let tev_orders = self.resolve_tev_orders();

            self.rebuild_lighting_cache_if_dirty();

            if self.konst_dirty {
                self.resolve_konst_colors();
                self.konst_dirty = false;
            }

            let mut indirect_matrices = [[0i32; 4]; 6];
            for (m, rows) in indirect_matrices.chunks_exact_mut(2).enumerate() {
                let mtx = &self.cur_indirect_matrices[m];
                let exp = mtx.scale_exponent();
                let r0 = mtx.row0();
                let r1 = mtx.row1();
                rows[0] = [r0[0], r0[1], r0[2], exp];
                rows[1] = [r1[0], r1[1], r1[2], exp];
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

            let ztex2 = TevZtex2::from_raw(self.bp_regs[BP_TEV_ZTEX2]);

            DrawState {
                tev_color_env: std::array::from_fn(|i| self.cur_tev_color_env[i].raw()),
                tev_alpha_env: std::array::from_fn(|i| self.cur_tev_alpha_env[i].raw()),
                tev_orders: std::array::from_fn(|i| tev_orders[i].raw()),
                tev_ksel: std::array::from_fn(|i| self.bp_regs[BP_TEV_KSEL_0 + i]),
                tev_color_regs,
                tev_konst_colors: self.cur_tev_konst_colors,
                num_tev_stages: self.cur_num_tev_stages,
                indirect_matrices,
                indirect_scales,
                indirect_refs: self.cur_indirect_refs.raw(),
                num_indirect_stages: self.cur_num_indirect_stages,
                bump_imask: self.cur_bump_imask,
                tev_indirect: std::array::from_fn(|i| self.cur_tev_indirect[i].raw()),
                color_ctrl: self.cached_color_ctrl,
                alpha_ctrl: self.cached_alpha_ctrl,
                ambient_color: self.cached_ambient_color,
                material_color: self.cached_material_color,
                lights: self.cached_lights,
                active_texcoords: (self.xf_mem[crate::flipper::gx::constants::XF_NUM_TEXGENS] as u8).min(8),
                ztex_bias: TevZtex1::from_raw(self.bp_regs[BP_TEV_ZTEX1]).bias(),
                ztex_type: ztex2.tex_type(),
                ztex_op: if self.cur_pe_control.early_ztest() {
                    0
                } else {
                    ztex2.op()
                },
            }
        });
        self.frame_state_dirty = false;
        self.draw_segments_scratch.clear();
        let mut segment_base = base_vertex;

        for entry in entries {
            let Some(primitive) = draw::Primitive::from_cmd(entry.cmd) else {
                tracing::error!(cmd = entry.cmd, "goofy draw command");
                segment_base = segment_base.wrapping_add(entry.vertex_count as u32);

                continue;
            };

            self.draw_segments_scratch.push(DrawSegment {
                primitive,
                base_vertex: segment_base,
                vertex_count: entry.vertex_count as u32,
            });
            segment_base = segment_base.wrapping_add(entry.vertex_count as u32);
        }

        renderer.exec_draw_batch(&self.draw_segments_scratch, state);

        #[cfg(feature = "gx-stats")]
        {
            self.stats.create_draw_call_ns += _gx_stats_t0.elapsed().as_nanos() as u64;
        }
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

    fn decode_vertex(&self, cur: &mut Cursor<&[u8]>, data: &[u8], ram: &RamView<'_>, vf: &VertexFormat) -> DrawVertex {
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
            decode_color(&data[start..start + vf.clr0_data_size], vf.vat_a.clr0_fmt())
        } else {
            let clr0_index = read_index(cur, vf.clr0_attr);
            let clr0_addr = vf.clr0_base + clr0_index * vf.clr0_stride;
            decode_color(
                fetch(ram, clr0_addr, vf.clr0_data_size, &mut scratch),
                vf.vat_a.clr0_fmt(),
            )
        };

        // Read color1
        let color1 = if vf.clr1_attr == AttributeType::None {
            [1.0, 1.0, 1.0, 1.0]
        } else if vf.clr1_attr == AttributeType::Direct {
            let start = cur.position() as usize;
            cur.set_position(cur.position() + vf.clr1_data_size as u64);
            decode_color(&data[start..start + vf.clr1_data_size], vf.vat_a.clr1_fmt())
        } else {
            let clr1_index = read_index(cur, vf.clr1_attr);
            let clr1_addr = vf.clr1_base + clr1_index * vf.clr1_stride;
            decode_color(
                fetch(ram, clr1_addr, vf.clr1_data_size, &mut scratch),
                vf.vat_a.clr1_fmt(),
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

        let mut texcoords: [[f32; 3]; 8] = [[0.0, 0.0, 1.0]; 8];
        self.apply_all_texgens(position, normal, &raw_texcoords, &tex_mtx_idx, &mut texcoords);

        DrawVertex {
            position,
            color0,
            color1,
            normal: [normal_view.0, normal_view.1, normal_view.2],
            pos_view: [pos_view.0, pos_view.1, pos_view.2],
            texcoords,
        }
    }

    fn rebuild_lighting_cache_if_dirty(&mut self) {
        if !self.lighting_dirty {
            return;
        }

        self.cached_color_ctrl = [
            regs::ChanCtrl::from_raw(self.xf_mem[XF_COLOR_CTRL0]),
            regs::ChanCtrl::from_raw(self.xf_mem[XF_COLOR_CTRL1]),
        ];

        self.cached_alpha_ctrl = [
            regs::ChanCtrl::from_raw(self.xf_mem[XF_ALPHA_CTRL0]),
            regs::ChanCtrl::from_raw(self.xf_mem[XF_ALPHA_CTRL1]),
        ];

        self.cached_ambient_color = [
            unpack_rgba(self.xf_mem[XF_AMBIENT_COLOR0]),
            unpack_rgba(self.xf_mem[XF_AMBIENT_COLOR1]),
        ];

        self.cached_material_color = [
            unpack_rgba(self.xf_mem[XF_MATERIAL_COLOR0]),
            unpack_rgba(self.xf_mem[XF_MATERIAL_COLOR1]),
        ];

        for i in 0..8 {
            let base = XF_LIGHT_BASE + i * XF_LIGHT_STRIDE;

            self.cached_lights[i].color = unpack_rgba(self.xf_mem[base + XF_LIGHT_COLOR]);

            self.cached_lights[i].cosatt = [
                f32::from_bits(self.xf_mem[base + XF_LIGHT_A0]),
                f32::from_bits(self.xf_mem[base + XF_LIGHT_A0 + 1]),
                f32::from_bits(self.xf_mem[base + XF_LIGHT_A0 + 2]),
                0.0,
            ];

            self.cached_lights[i].distatt = [
                f32::from_bits(self.xf_mem[base + XF_LIGHT_K0]),
                f32::from_bits(self.xf_mem[base + XF_LIGHT_K0 + 1]),
                f32::from_bits(self.xf_mem[base + XF_LIGHT_K0 + 2]),
                0.0,
            ];

            self.cached_lights[i].position = [
                f32::from_bits(self.xf_mem[base + XF_LIGHT_PX]),
                f32::from_bits(self.xf_mem[base + XF_LIGHT_PX + 1]),
                f32::from_bits(self.xf_mem[base + XF_LIGHT_PX + 2]),
                0.0,
            ];

            self.cached_lights[i].direction = [
                f32::from_bits(self.xf_mem[base + XF_LIGHT_NX]),
                f32::from_bits(self.xf_mem[base + XF_LIGHT_NX + 1]),
                f32::from_bits(self.xf_mem[base + XF_LIGHT_NX + 2]),
                0.0,
            ];
        }

        self.lighting_dirty = false;
    }
}

fn record_draw_arrays(
    rec: &mut super::recorder::FifoRecorder,
    ram: &RamView<'_>,
    vf: &VertexFormat,
    data: &[u8],
    vertex_count: usize,
) {
    let attr_stream_size = |attr: AttributeType, direct_size: usize| -> usize {
        match attr {
            AttributeType::Direct => direct_size,
            AttributeType::Index8 => 1,
            AttributeType::Index16 => 2,
            AttributeType::None => 0,
        }
    };

    let mut offset = 0usize;
    offset += vf.has_pnmtxidx as usize;
    for has in vf.has_tex_mtx_idx {
        offset += has as usize;
    }

    let mut scan = |offset: usize, attr: AttributeType, base: usize, stride: usize, data_size: usize| {
        self::scan_indexed_component(
            rec,
            ram,
            data,
            vf.vertex_stride,
            vertex_count,
            offset,
            attr,
            base,
            stride,
            data_size,
        );
    };

    scan(offset, vf.pos_attr, vf.pos_base, vf.pos_stride, vf.pos_data_size);
    offset += attr_stream_size(vf.pos_attr, vf.pos_data_size);

    scan(offset, vf.nrm_attr, vf.nrm_base_addr, vf.nrm_stride, vf.nrm_data_size);
    offset += vf.vat_a.nrm_stream_size(vf.nrm_attr);

    scan(offset, vf.clr0_attr, vf.clr0_base, vf.clr0_stride, vf.clr0_data_size);
    offset += attr_stream_size(vf.clr0_attr, vf.clr0_data_size);

    scan(offset, vf.clr1_attr, vf.clr1_base, vf.clr1_stride, vf.clr1_data_size);
    offset += attr_stream_size(vf.clr1_attr, vf.clr1_data_size);

    for tc in 0..8 {
        scan(
            offset,
            vf.tex_attrs[tc],
            vf.tex_bases[tc],
            vf.tex_strides[tc],
            vf.tex_data_sizes[tc],
        );
        offset += attr_stream_size(vf.tex_attrs[tc], vf.tex_data_sizes[tc]);
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_indexed_component(
    rec: &mut super::recorder::FifoRecorder,
    ram: &RamView<'_>,
    data: &[u8],
    vertex_stride: usize,
    vertex_count: usize,
    offset: usize,
    attr: AttributeType,
    base: usize,
    stride: usize,
    data_size: usize,
) {
    let idx_size = match attr {
        AttributeType::Index8 => 1,
        AttributeType::Index16 => 2,
        _ => return,
    };

    // All-ones indices skip the vertex on real hardware.
    let mut max_index: Option<usize> = None;
    for v in 0..vertex_count {
        let p = v * vertex_stride + offset;
        if p + idx_size > data.len() {
            break;
        }
        let index = if idx_size == 1 {
            match data[p] {
                0xFF => continue,
                i => i as usize,
            }
        } else {
            match u16::from_be_bytes([data[p], data[p + 1]]) {
                0xFFFF => continue,
                i => i as usize,
            }
        };
        if max_index.is_none_or(|m| index > m) {
            max_index = Some(index);
        }
    }

    if let Some(max) = max_index {
        let len = stride * max + data_size;
        rec.use_memory(ram, base as u32, len, super::recorder::MemoryUpdateType::VertexStream);
    }
}

fn read_index(cur: &mut Cursor<&[u8]>, attr: AttributeType) -> usize {
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

#[inline(always)]
fn decode_position(data: &[u8], vat: &VatA) -> [f32; 3] {
    let num = vat.pos_cnt().components();
    let recip = 1.0f32 / ((1u32 << vat.pos_shift()) as f32);
    self::decode_components::<3>(data, num, vat.pos_fmt(), recip)
}

#[inline(always)]
fn decode_normal(data: &[u8], vat: &VatA) -> [f32; 3] {
    let cnt = vat.nrm_cnt().components().min(3);
    let fmt = vat.nrm_fmt();
    let recip = match fmt {
        ComponentFormat::U8 | ComponentFormat::S8 => 1.0f32 / 64.0,
        ComponentFormat::U16 | ComponentFormat::S16 => 1.0f32 / 16384.0,
        ComponentFormat::F32 => 1.0f32,
    };
    self::decode_components::<3>(data, cnt, fmt, recip)
}

#[inline(always)]
fn decode_texcoord(data: &[u8], fmt: ComponentFormat, shift: u8, cnt: TexCount) -> [f32; 2] {
    let num = cnt.components();
    let recip = 1.0f32 / ((1u32 << shift) as f32);
    let r3 = self::decode_components::<3>(data, num, fmt, recip);
    [r3[0], r3[1]]
}

#[inline(always)]
fn decode_components<const N: usize>(data: &[u8], num: usize, fmt: ComponentFormat, recip: f32) -> [f32; N] {
    let mut result = [0.0f32; N];

    match fmt {
        ComponentFormat::U8 => {
            for i in 0..num {
                result[i] = data[i] as f32 * recip;
            }
        }
        ComponentFormat::S8 => {
            for i in 0..num {
                result[i] = (data[i] as i8) as f32 * recip;
            }
        }
        ComponentFormat::U16 => {
            for i in 0..num {
                let off = i * 2;
                result[i] = u16::from_be_bytes([data[off], data[off + 1]]) as f32 * recip;
            }
        }
        ComponentFormat::S16 => {
            for i in 0..num {
                let off = i * 2;
                result[i] = i16::from_be_bytes([data[off], data[off + 1]]) as f32 * recip;
            }
        }
        ComponentFormat::F32 => {
            for i in 0..num {
                let off = i * 4;
                result[i] = f32::from_bits(u32::from_be_bytes([
                    data[off],
                    data[off + 1],
                    data[off + 2],
                    data[off + 3],
                ]));
            }
        }
    }

    result
}

fn decode_color(data: &[u8], fmt: regs::ColorFormat) -> [f32; 4] {
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
            let a = data[3] as f32 / 255.0;
            [r, g, b, a]
        }
    }
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
fn dispatch_decode<const SYSTEM: SystemId>(
    gp: &mut GraphicsProcessor,
    mmio: &mut Mmio<SYSTEM>,
    #[cfg_attr(not(feature = "jit"), allow(unused_variables))] cmd: u8,
    data: &[u8],
    vertex_count: usize,
    verts: &mut Vec<DrawVertex>,
) {
    #[cfg(feature = "jit")]
    if gp.execution_mode == ExecutionMode::Jit {
        let vat_index = (cmd & 0b111) as usize;
        let key = jit::VtxKey::from_cp_regs(&gp.cp_regs, vat_index);
        let arrays_ok = if gp.jit_vtx_arrays_key == Some(key) {
            gp.jit_vtx_arrays_ok
        } else {
            let view = mmio.ram_view();
            let ok = jit::resolve_arrays_for_draw(&gp.cp_regs, &key, view.mem1, view.mem2, &mut gp.jit_vtx_arrays);
            gp.jit_vtx_arrays_key = Some(key);
            gp.jit_vtx_arrays_ok = ok;
            ok
        };
        let parser = if arrays_ok {
            gp.jit_vtx.lookup_or_compile(key)
        } else {
            None
        };

        if let Some(parser) = parser {
            let xf_mem_ptr = gp.xf_mem.as_ptr();
            let arrays_ptr = gp.jit_vtx_arrays.0.as_ptr();
            let base = verts.len();
            let out_ptr = unsafe { verts.as_mut_ptr().add(base) };

            unsafe {
                parser(xf_mem_ptr, arrays_ptr, data.as_ptr(), out_ptr, vertex_count as u32);
                verts.set_len(base + vertex_count);
            }

            #[cfg(feature = "vtx-jit-validate")]
            {
                let vf = gp.build_vertex_format(cmd);
                self::run_validator(gp, mmio, cmd, key, data, vertex_count, &vf, verts, base);
            }
            return;
        }
    }

    let vf = gp.build_vertex_format(cmd);
    self::run_interpreter(gp, mmio, data, vertex_count, &vf, verts);
}

#[cfg_attr(feature = "hotpath", hotpath::measure(label = "vtx_run_interpreter"))]
fn run_interpreter<const SYSTEM: SystemId>(
    gp: &mut GraphicsProcessor,
    mmio: &mut Mmio<SYSTEM>,
    data: &[u8],
    vertex_count: usize,
    vf: &VertexFormat,
    verts: &mut Vec<DrawVertex>,
) {
    self::decode_vertices(gp, mmio, data, vertex_count, vf, verts);
}

fn decode_vertices<const SYSTEM: SystemId>(
    gp: &mut GraphicsProcessor,
    mmio: &mut Mmio<SYSTEM>,
    data: &[u8],
    vertex_count: usize,
    vf: &VertexFormat,
    dest: &mut Vec<DrawVertex>,
) {
    let view = mmio.ram_view();
    let mut cur = Cursor::new(data);
    for _ in 0..vertex_count {
        let v = gp.decode_vertex(&mut cur, data, &view, vf);
        dest.push(v);
    }
}

#[cfg(feature = "vtx-jit-validate")]
fn run_validator<const SYSTEM: SystemId>(
    gp: &mut GraphicsProcessor,
    mmio: &mut Mmio<SYSTEM>,
    cmd: u8,
    key: jit::VtxKey,
    data: &[u8],
    vertex_count: usize,
    vf: &VertexFormat,
    verts: &[DrawVertex],
    base: usize,
) {
    let mut interp_buf = std::mem::take(&mut gp.jit_vtx_validator.interp_scratch);
    interp_buf.clear();
    interp_buf.reserve(vertex_count);

    self::decode_vertices(gp, mmio, data, vertex_count, vf, &mut interp_buf);

    let ctx = jit::validate::CompareCtx {
        key,
        draw_cmd: cmd,
        vertex_count: vertex_count as u32,
    };
    let jit_slice = &verts[base..base + vertex_count];
    let mismatches = jit::validate::compare_draw_vertices(jit_slice, &interp_buf, &ctx);
    gp.jit_vtx_validator.record(&ctx, &mismatches);

    gp.jit_vtx_validator.interp_scratch = interp_buf;
}
