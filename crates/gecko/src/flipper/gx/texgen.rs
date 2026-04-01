use super::{
    GraphicsProcessor,
    constants::{
        XF_DUAL_TEX_ENABLE, XF_DUALTEX_BASE, XF_MATRIX_INDEX_A, XF_MATRIX_INDEX_B, XF_POS_MTX_STRIDE, XF_POST_MTX_BASE,
        XF_TEXGEN_BASE,
    },
    regs::{DualTexGenReg, MatrixIndex0, MatrixIndex1, TexGenReg},
};

impl GraphicsProcessor {
    pub(crate) fn compute_texgen(
        &self,
        texgen_idx: usize,
        position: [f32; 3],
        normal: [f32; 3],
        raw_texcoords: &[Option<[f32; 2]>; 8],
    ) -> [f32; 2] {
        let tg = TexGenReg::from_raw(self.xf_mem[XF_TEXGEN_BASE + texgen_idx]);
        let dt = DualTexGenReg::from_raw(self.xf_mem[XF_DUALTEX_BASE + texgen_idx]);

        // Select the source input vector based on texgen source_row
        let src = self.select_texgen_source(&tg, position, normal, raw_texcoords);

        // Form input vector based on input_form
        let input = match tg.input_form() {
            super::regs::TexGenInputForm::Ab11 => [src[0], src[1], 1.0, 1.0],
            super::regs::TexGenInputForm::Abc1 => [src[0], src[1], src[2], 1.0],
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
        let (s, t, q) = self.apply_tex_matrix(&tg, tex_mtx_base, &input);

        // Dual texgen post-transform (normalization + post-matrix multiply)
        let (s, t, q) = self.apply_dual_texgen(s, t, q, &dt);

        // When q is 0, the GameCube has a special case (Dolphin VertexShaderGen.cpp)
        if q.abs() < f32::EPSILON {
            [(s / 2.0).clamp(-1.0, 1.0), (t / 2.0).clamp(-1.0, 1.0)]
        } else {
            [s / q, t / q]
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

    fn apply_tex_matrix(&self, tg: &TexGenReg, tex_mtx_base: usize, input: &[f32; 4]) -> (f32, f32, f32) {
        match tg.projection() {
            super::regs::TexGenProjection::St => {
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
            super::regs::TexGenProjection::Stq => {
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
        }
    }

    fn apply_dual_texgen(&self, s: f32, t: f32, q: f32, dt: &DualTexGenReg) -> (f32, f32, f32) {
        let dual_tex_enabled = self.xf_mem[XF_DUAL_TEX_ENABLE] != 0;
        if !dual_tex_enabled {
            return (s, t, q);
        }

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
    }
}
