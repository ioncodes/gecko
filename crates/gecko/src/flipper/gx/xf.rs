use super::{
    GraphicsProcessor,
    constants::{XF_PROJECTION_BASE, XF_PROJECTION_END},
    draw,
    math::Vec3,
};

impl GraphicsProcessor {
    pub(crate) fn xf_f32(&self, reg: usize) -> f32 {
        f32::from_bits(self.xf_mem[reg])
    }

    pub(crate) fn xf_vec3(&self, reg: usize) -> Vec3 {
        Vec3(self.xf_f32(reg), self.xf_f32(reg + 1), self.xf_f32(reg + 2))
    }

    pub(crate) fn xf_transform_3x4(&self, base: usize, v: [f32; 3]) -> Vec3 {
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

    pub(crate) fn rebuild_projection(&mut self) {
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

    pub(crate) fn load_cp(&mut self, data: &[u8]) {
        let idx = data[0] as usize;
        let val = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
        self.cp_regs[idx] = val;

        tracing::debug!(
            reg_idx = format!("{idx:02X}"),
            value = format!("{val:08X}"),
            "CP register write"
        );
    }

    pub(crate) fn load_xf(&mut self, data: &[u8]) {
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
