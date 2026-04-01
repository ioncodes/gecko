use super::{GraphicsProcessor, constants::BP_TEV_KSEL_0, regs::TevStageOrder};

impl GraphicsProcessor {
    pub(crate) fn resolve_tev_orders(&self) -> [TevStageOrder; 16] {
        let mut orders = [TevStageOrder::default(); 16];
        for i in 0..8 {
            let reg = self.cur_tev_orders[i];
            orders[2 * i] = TevStageOrder::default()
                .with_texmap(reg.texmap0())
                .with_texcoord(reg.texcoord0())
                .with_tex_enable(reg.tex_enable0());
            orders[2 * i + 1] = TevStageOrder::default()
                .with_texmap(reg.texmap1())
                .with_texcoord(reg.texcoord1())
                .with_tex_enable(reg.tex_enable1());
        }
        orders
    }

    pub(crate) fn resolve_tev_color_regs(&self) -> [[f32; 4]; 4] {
        std::array::from_fn(|i| {
            let lo = self.cur_tev_color_regs_lo[i];
            let hi = self.cur_tev_color_regs_hi[i];
            [
                s11_to_f32(lo.r()),
                s11_to_f32(hi.g()),
                s11_to_f32(hi.b()),
                s11_to_f32(lo.a()),
            ]
        })
    }

    pub(crate) fn resolve_konst_colors(&mut self) {
        let kregs: [[f32; 4]; 4] = std::array::from_fn(|i| {
            let lo = self.cur_tev_const_regs_lo[i];
            let hi = self.cur_tev_const_regs_hi[i];
            [
                s11_to_f32(lo.r()),
                s11_to_f32(hi.g()),
                s11_to_f32(hi.b()),
                s11_to_f32(lo.a()),
            ]
        });

        for stage in 0..16usize {
            let ksel_reg = self.bp_regs[BP_TEV_KSEL_0 + stage / 2];
            let (kcsel, kasel) = if stage % 2 == 0 {
                ((ksel_reg >> 4) & 0x1F, (ksel_reg >> 9) & 0x1F)
            } else {
                ((ksel_reg >> 14) & 0x1F, (ksel_reg >> 19) & 0x1F)
            };

            let rgb = resolve_kcsel(kcsel, &kregs);
            let a = resolve_kasel(kasel, &kregs);
            self.cur_tev_konst_colors[stage] = [rgb[0], rgb[1], rgb[2], a];
        }
    }
}

pub(crate) fn s11_to_f32(val: u16) -> f32 {
    let signed = if val & 0x400 != 0 {
        val as i32 - 0x800
    } else {
        val as i32
    };
    signed as f32 / 255.0
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
