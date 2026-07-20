use super::constants::*;
use super::math::Vec3;
use super::{GraphicsProcessor, draw};
use crate::host::{GxAction, RenderSink};
use crate::mmio::RamView;

const XF_LIGHT_END: usize = XF_LIGHT_BASE + 8 * XF_LIGHT_STRIDE;
const XF_CHAN_CFG_BEGIN: usize = XF_AMBIENT_COLOR0;
const XF_CHAN_CFG_END: usize = XF_ALPHA_CTRL1 + 1;

#[inline(always)]
fn ranges_overlap(a_begin: usize, a_end: usize, b_begin: usize, b_end: usize) -> bool {
    a_begin < b_end && b_begin < a_end
}

#[derive(Clone, Copy, Default)]
struct XfDirty {
    projection: bool,
    viewport: bool,
    lighting: bool,
}

impl GraphicsProcessor {
    fn store_xf_regs(&mut self, base: usize, values: impl ExactSizeIterator<Item = u32>) -> XfDirty {
        let end = base + values.len();

        let tracked = self::ranges_overlap(base, end, XF_PROJECTION_BASE, XF_PROJECTION_END + 1)
            || self::ranges_overlap(base, end, XF_VIEWPORT_BASE, XF_VIEWPORT_END + 1)
            || self::ranges_overlap(base, end, XF_LIGHT_BASE, XF_LIGHT_END)
            || self::ranges_overlap(base, end, XF_CHAN_CFG_BEGIN, XF_CHAN_CFG_END);

        let mut dirty = XfDirty::default();

        for (i, val) in values.enumerate() {
            let reg = base + i;
            if reg < self.xf_mem.len() {
                if tracked && self.xf_mem[reg] != val {
                    dirty.projection |= (XF_PROJECTION_BASE..=XF_PROJECTION_END).contains(&reg);
                    dirty.viewport |= (XF_VIEWPORT_BASE..=XF_VIEWPORT_END).contains(&reg);
                    dirty.lighting |= (XF_LIGHT_BASE..XF_LIGHT_END).contains(&reg)
                        || (XF_CHAN_CFG_BEGIN..XF_CHAN_CFG_END).contains(&reg);
                }
                self.xf_mem[reg] = val;
            }

            tracing::debug!(
                reg_idx = format!("{reg:04X}"),
                value = format!("{val:08X}"),
                "XF register write"
            );
        }

        dirty
    }

    fn apply_xf_dirty(&mut self, renderer: &mut dyn RenderSink, dirty: XfDirty) {
        if dirty.projection {
            self.rebuild_projection();
            renderer.exec(GxAction::SetProjection {
                matrix: self.projection.0,
                is_perspective: self.xf_mem[XF_PROJECTION_END] == 0,
            });
        }

        if dirty.viewport {
            self.rebuild_viewport();
            renderer.exec(GxAction::SetViewport(self.cur_viewport));
        }

        if dirty.lighting {
            self.lighting_dirty = true;
            self.frame_state_dirty = true;
        }
    }

    #[inline(always)]
    pub fn xf_transform_3x4(&self, base: usize, v: [f32; 3]) -> Vec3 {
        let m: [f32; 12] = std::array::from_fn(|i| f32::from_bits(self.xf_mem[base + i]));
        Vec3(
            m[0] * v[0] + m[1] * v[1] + m[2] * v[2] + m[3],
            m[4] * v[0] + m[5] * v[1] + m[6] * v[2] + m[7],
            m[8] * v[0] + m[9] * v[1] + m[10] * v[2] + m[11],
        )
    }

    pub fn rebuild_viewport(&mut self) {
        let scale_x = f32::from_bits(self.xf_mem[XF_VIEWPORT_SCALE_X]);
        let scale_y = f32::from_bits(self.xf_mem[XF_VIEWPORT_SCALE_Y]);
        let scale_z = f32::from_bits(self.xf_mem[XF_VIEWPORT_SCALE_Z]);
        let offset_x = f32::from_bits(self.xf_mem[XF_VIEWPORT_OFFSET_X]);
        let offset_y = f32::from_bits(self.xf_mem[XF_VIEWPORT_OFFSET_Y]);
        let offset_z = f32::from_bits(self.xf_mem[XF_VIEWPORT_OFFSET_Z]);

        // Decode: scale_x = wd*0.5, scale_y = (-ht)*0.5
        // offset_x = (xOrig + wd*0.5) + 342, offset_y = (yOrig + ht*0.5) + 342
        let w = scale_x * 2.0;
        let h = scale_y * -2.0;
        let x = offset_x - 342.0 - scale_x;
        let y = offset_y - 342.0 + scale_y; // +scale_y because scale_y is negative

        // Apply BP_SU_SCIS_OFFSET: it shifts both the scissor rect and the
        // viewport origin in the EFB, so games can tile-render without
        // touching their projection matrix.
        let x = x - self.cur_scissor_offset_x as f32;
        let y = y - self.cur_scissor_offset_y as f32;

        let far = (offset_z / DEPTH_24_BIT_RANGE).clamp(0.0, MAX_EFB_DEPTH);
        let near = ((offset_z - scale_z) / DEPTH_24_BIT_RANGE).clamp(0.0, MAX_EFB_DEPTH);

        self.cur_viewport = draw::Viewport {
            x,
            y,
            w,
            h,
            min_depth: near,
            max_depth: far,
        };
    }

    pub fn rebuild_projection(&mut self) {
        let pm1 = f32::from_bits(self.xf_mem[XF_PROJECTION_BASE + 0]);
        let pm2 = f32::from_bits(self.xf_mem[XF_PROJECTION_BASE + 1]);
        let pm3 = f32::from_bits(self.xf_mem[XF_PROJECTION_BASE + 2]);
        let pm4 = f32::from_bits(self.xf_mem[XF_PROJECTION_BASE + 3]);
        let pm5 = f32::from_bits(self.xf_mem[XF_PROJECTION_BASE + 4]);
        let pm6 = f32::from_bits(self.xf_mem[XF_PROJECTION_BASE + 5]);
        let proj_type = self.xf_mem[XF_PROJECTION_END];

        self.projection = if proj_type == 0 {
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

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn load_cp(&mut self, data: &[u8]) {
        let idx = data[0] as usize;
        let val = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
        self.cp_regs[idx] = val;

        tracing::debug!(
            reg_idx = format!("{idx:02X}"),
            value = format!("{val:08X}"),
            "CP register write"
        );
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn load_xf(&mut self, renderer: &mut dyn RenderSink, data: &[u8]) {
        let length = u16::from_be_bytes([data[0], data[1]]) as usize;
        let addr = u16::from_be_bytes([data[2], data[3]]) as usize;
        let n = length + 1;

        #[cfg(feature = "gx-stats")]
        {
            self.stats.xf_writes += n as u64;
        }

        let values = data[4..4 + n * 4]
            .chunks_exact(4)
            .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]));

        let dirty = self.store_xf_regs(addr, values);

        self.apply_xf_dirty(renderer, dirty);
    }

    #[inline(always)]
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn load_indexed_xf(
        &mut self,
        renderer: &mut dyn RenderSink,
        ram: &RamView<'_>,
        cp_array_index: u8,
        index: u16,
        xf_addr: u16,
        xf_count: u8,
    ) {
        let arr_idx = cp_array_index as usize;
        let base = (self.cp_regs[ARRAY_BASE_REG + arr_idx] & 0x3FFFFFFF) as usize;
        let stride = self.cp_regs[ARRAY_STRIDE_REG + arr_idx] as usize;
        let src_addr = base + (index as usize) * stride;
        let dst_addr = xf_addr as usize;
        let n = xf_count as usize;

        let Some(src) = ram.slice(src_addr, n * 4) else {
            tracing::warn!(
                src_addr = format!("{src_addr:#010X}"),
                bytes = n * 4,
                "load_indexed_xf: source not mapped to MEM1/MEM2, skipping"
            );
            return;
        };

        if let Some(rec) = self.recorder.as_deref_mut() {
            rec.use_memory(ram, src_addr as u32, n * 4, super::recorder::MemoryUpdateType::XfData);
        }

        let values = src
            .chunks_exact(4)
            .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]));

        let dirty = self.store_xf_regs(dst_addr, values);

        self.apply_xf_dirty(renderer, dirty);
    }
}
