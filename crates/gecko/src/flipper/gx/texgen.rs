use super::GraphicsProcessor;
use super::constants::*;
use super::regs::*;

impl GraphicsProcessor {
    pub fn compute_texgen(
        &self,
        texgen_idx: usize,
        position: [f32; 3],
        normal: [f32; 3],
        raw_texcoords: &[Option<[f32; 2]>; 8],
        tex_mtx_indices: &[u8; 8],
    ) -> [f32; 3] {
        let tg = TexGenReg::from_raw(self.xf_mem[XF_TEXGEN_BASE + texgen_idx]);

        let src = self.select_texgen_source(&tg, position, normal, raw_texcoords);

        let input = match tg.input_form() {
            super::regs::TexGenInputForm::Ab11 => [src[0], src[1], 1.0, 1.0],
            super::regs::TexGenInputForm::Abc1 => [src[0], src[1], src[2], 1.0],
        };

        let tex_mtx_base = tex_mtx_indices[texgen_idx] as usize * XF_POS_MTX_STRIDE;

        let (s, t, q) = self.apply_tex_matrix(&tg, tex_mtx_base, &input);

        let (s, t, q) = if self.xf_mem[XF_DUAL_TEX_ENABLE] != 0 {
            let dt = DualTexGenReg::from_raw(self.xf_mem[XF_DUALTEX_BASE + texgen_idx]);
            self.apply_dual_texgen(s, t, q, &dt)
        } else {
            (s, t, q)
        };

        // Return (s, t, q) as a 3-component vector. The perspective divide
        // (s/q, t/q) is deferred to the fragment shader so the rasterizer
        // can perform correct perspective-correct interpolation.
        //
        // When q is 0, the GameCube uses a special path (Dolphin
        // VertexShaderGen.cpp): clamp(xy / 2, -1, 1) with q set to 0.
        // The fragment shader checks for q~=0 and uses xy directly.
        if q.abs() < f32::EPSILON {
            [(s / 2.0).clamp(-1.0, 1.0), (t / 2.0).clamp(-1.0, 1.0), 0.0]
        } else {
            [s, t, q]
        }
    }

    fn select_texgen_source(
        &self,
        tg: &TexGenReg,
        position: [f32; 3],
        normal: [f32; 3],
        raw_texcoords: &[Option<[f32; 2]>; 8],
    ) -> [f32; 3] {
        match tg.source_row() {
            super::regs::TexGenSrc::Pos => position,
            super::regs::TexGenSrc::Nrm => normal,
            super::regs::TexGenSrc::Tex0 => {
                let t = raw_texcoords[0].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            super::regs::TexGenSrc::Tex1 => {
                let t = raw_texcoords[1].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            super::regs::TexGenSrc::Tex2 => {
                let t = raw_texcoords[2].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            super::regs::TexGenSrc::Tex3 => {
                let t = raw_texcoords[3].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            super::regs::TexGenSrc::Tex4 => {
                let t = raw_texcoords[4].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            super::regs::TexGenSrc::Tex5 => {
                let t = raw_texcoords[5].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            super::regs::TexGenSrc::Tex6 => {
                let t = raw_texcoords[6].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            super::regs::TexGenSrc::Tex7 => {
                let t = raw_texcoords[7].unwrap_or([0.0, 0.0]);
                [t[0], t[1], 1.0]
            }
            // TODO: ???
            _ => [0.0, 0.0, 1.0],
        }
    }

    #[inline(always)]
    fn apply_tex_matrix(&self, tg: &TexGenReg, tex_mtx_base: usize, input: &[f32; 4]) -> (f32, f32, f32) {
        let m: [f32; 12] = std::array::from_fn(|i| f32::from_bits(self.xf_mem[tex_mtx_base + i]));
        let s = m[0] * input[0] + m[1] * input[1] + m[2] * input[2] + m[3] * input[3];
        let t = m[4] * input[0] + m[5] * input[1] + m[6] * input[2] + m[7] * input[3];

        match tg.projection() {
            super::regs::TexGenProjection::St => (s, t, 1.0),
            super::regs::TexGenProjection::Stq => {
                let q = m[8] * input[0] + m[9] * input[1] + m[10] * input[2] + m[11] * input[3];
                (s, t, q)
            }
        }
    }

    #[inline(always)]
    fn apply_dual_texgen(&self, s: f32, t: f32, q: f32, dt: &DualTexGenReg) -> (f32, f32, f32) {
        let post_base = XF_POST_MTX_BASE + dt.post_mtx_idx() as usize * 4;
        let (ns, nt, nq) = if dt.normalize() {
            let inv_q = if q.abs() > f32::EPSILON { 1.0 / q } else { 1.0 };
            (s * inv_q, t * inv_q, inv_q)
        } else {
            (s, t, q)
        };

        let m: [f32; 12] = std::array::from_fn(|i| f32::from_bits(self.xf_mem[post_base + i]));

        let ps = m[0] * ns + m[1] * nt + m[2] * nq + m[3];
        let pt = m[4] * ns + m[5] * nt + m[6] * nq + m[7];
        let pq = m[8] * ns + m[9] * nt + m[10] * nq + m[11];

        (ps, pt, pq)
    }
}
