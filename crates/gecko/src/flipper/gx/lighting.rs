use super::{
    GraphicsProcessor,
    constants::{XF_LIGHT_A0, XF_LIGHT_BASE, XF_LIGHT_COLOR, XF_LIGHT_K0, XF_LIGHT_NX, XF_LIGHT_PX, XF_LIGHT_STRIDE},
    math::{Vec3, saturating_div, unpack_rgba},
    regs::{AttnFn, ChanCtrl},
};

impl GraphicsProcessor {
    pub(crate) fn compute_channel_lighting(
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
                let da = if ctrl.diff_fn() != super::regs::DiffuseFn::None {
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
            super::regs::DiffuseFn::None => 1.0,
            super::regs::DiffuseFn::Signed => dif_attn,
            super::regs::DiffuseFn::Clamp => dif_attn.max(0.0),
        };

        attn * diffuse
    }
}
