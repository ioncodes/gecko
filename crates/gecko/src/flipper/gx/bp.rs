use super::{
    GraphicsProcessor,
    constants::{
        BP_GEN_MODE, BP_PE_ALPHA_COMPARE, BP_PE_CMODE0, BP_PE_DONE, BP_PE_DONE_FINISH_BIT, BP_PE_ZMODE,
        BP_RAS1_TREF_COUNT, BP_RAS1_TREF0, BP_TEV_COLOR_ENV_0, BP_TEV_REGISTERL_0, BP_TX_SETIMAGE0_I0,
        BP_TX_SETIMAGE0_I4, BP_TX_SETIMAGE3_I0, BP_TX_SETIMAGE3_I4, BP_TX_SETMODE0_I0, BP_TX_SETMODE0_I4,
    },
    draw,
    regs::{
        AlphaCompare, BlendMode, GenMode, TevAlphaEnv, TevColorEnv, TevOrder, TevRegType, TevRegisterH, TevRegisterL,
        TxSetImage0, TxSetImage3, TxSetMode0, ZMode,
    },
};

impl GraphicsProcessor {
    pub(crate) fn load_bp(&mut self, data: &[u8]) {
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
            self.snapshot_texture(slot, val);
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
            self.load_tev_env(idx, val);
        }

        // TEV rasterizer order registers (RAS1_TREF0-7, 0x28-0x2F)
        if idx >= BP_RAS1_TREF0 && idx < BP_RAS1_TREF0 + BP_RAS1_TREF_COUNT {
            self.cur_tev_orders[idx - BP_RAS1_TREF0] = TevOrder::from_raw(val);
        }

        // TEV color registers (0xE0-0xE7): pairs of lo/hi writes
        if idx >= BP_TEV_REGISTERL_0 && idx <= BP_TEV_REGISTERL_0 + 7 {
            self.load_tev_register(idx, val);
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

    fn snapshot_texture(&mut self, slot: usize, image3_val: u32) {
        let image0_reg = if slot < 4 {
            BP_TX_SETIMAGE0_I0 + slot
        } else {
            BP_TX_SETIMAGE0_I4 + (slot - 4)
        };

        let image0 = TxSetImage0::from_raw(self.bp_regs[image0_reg]);
        let image3 = TxSetImage3::from_raw(image3_val);

        let mode0_reg = if slot < 4 {
            BP_TX_SETMODE0_I0 + slot
        } else {
            BP_TX_SETMODE0_I4 + (slot - 4)
        };
        let mode0 = TxSetMode0::from_raw(self.bp_regs[mode0_reg]);

        let width = image0.width() + 1;
        let height = image0.height() + 1;
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

    fn load_tev_env(&mut self, idx: usize, val: u32) {
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

    fn load_tev_register(&mut self, idx: usize, val: u32) {
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
}
