use super::constants::{
    BP_GEN_MODE, BP_PE_ALPHA_COMPARE, BP_PE_CMODE0, BP_PE_COPY_CLEAR_AR, BP_PE_COPY_CLEAR_GB, BP_PE_COPY_CLEAR_Z,
    BP_PE_COPY_CMD, BP_PE_COPY_DIMS, BP_PE_COPY_DST, BP_PE_COPY_DST_STRIDE, BP_PE_COPY_SRC, BP_PE_DONE,
    BP_PE_DONE_FINISH_BIT, BP_PE_TOKEN, BP_PE_TOKEN_INT, BP_PE_ZMODE, BP_RAS1_TREF_COUNT, BP_RAS1_TREF0, BP_SU_SCIS_BR,
    BP_SU_SCIS_OFFSET, BP_SU_SCIS_TL, BP_TEV_COLOR_ENV_0, BP_TEV_KSEL_0, BP_TEV_REGISTERL_0, BP_TX_SETIMAGE0_I0,
    BP_TX_SETIMAGE0_I4, BP_TX_SETIMAGE3_I0, BP_TX_SETIMAGE3_I4, BP_TX_SETMODE0_I0, BP_TX_SETMODE0_I4, EFB_HEIGHT,
    EFB_WIDTH,
};
use super::regs::{
    AlphaCompare, BlendMode, EfbCopyDims, EfbCopyDst, EfbCopyDstStride, EfbCopySrc, GenMode, PeClearAr, PeClearGb,
    PeClearZ, PeCopyCmd, SuScisOffset, SuScisRect, TevAlphaEnv, TevColorEnv, TevOrder, TevRegType, TevRegisterH,
    TevRegisterL, TxSetImage0, TxSetImage3, TxSetMode0, ZMode,
};
use super::{GraphicsProcessor, draw, texture};
use crate::common::Address;
use crate::host::{GxAction, RenderSink};
use std::time::Duration;

impl GraphicsProcessor {
    pub fn load_bp(&mut self, renderer: &mut dyn RenderSink, ram: &mut [u8], data: &[u8]) {
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
            self.snapshot_texture(renderer, ram, slot, val);
        }

        // Forward PE render state
        match idx {
            BP_PE_ZMODE => {
                self.cur_zmode = ZMode::from_raw(val);
                renderer.exec(GxAction::SetDepthMode(self.cur_zmode));
            }
            BP_PE_CMODE0 => {
                self.cur_blend_mode = BlendMode::from_raw(val);
                renderer.exec(GxAction::SetBlendMode(self.cur_blend_mode));
            }
            BP_PE_ALPHA_COMPARE => {
                self.cur_alpha_compare = AlphaCompare::from_raw(val);
                renderer.exec(GxAction::SetAlphaCompare(self.cur_alpha_compare));
            }
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

        // GEN_MODE, extract num TEV stages + cull mode
        if idx == BP_GEN_MODE {
            let gen_mode = GenMode::from_raw(val);
            let stages = gen_mode.num_tev_stages() + 1;
            tracing::debug!(num_tev_stages = stages, "GEN_MODE");
            self.cur_num_tev_stages = stages;
            self.resolve_konst_colors();
            renderer.exec(GxAction::SetCullMode(gen_mode.cull_mode()));
        }

        // TEV KSEL (constant color selection) registers: recalculate konst colors
        if idx >= BP_TEV_KSEL_0 && idx <= BP_TEV_KSEL_0 + 7 {
            self.resolve_konst_colors();
        }

        // TEV color register writes also affect konst colors
        if idx >= BP_TEV_REGISTERL_0 && idx <= BP_TEV_REGISTERL_0 + 7 {
            self.resolve_konst_colors();
        }

        // PE finish
        if idx == BP_PE_DONE && (val & BP_PE_DONE_FINISH_BIT) != 0 {
            self.raise_interrupt = true;
        }

        // PE token (0x47): store token value only
        if idx == BP_PE_TOKEN {
            self.pending_token = (val & 0xFFFF) as u16;
            self.token_dirty = true;
        }

        // PE token interrupt (0x48): store token value + raise interrupt
        if idx == BP_PE_TOKEN_INT {
            tracing::debug!(token = val & 0xFFFF, "PE token interrupt raised");
            self.pending_token = (val & 0xFFFF) as u16;
            self.token_dirty = true;
            self.raise_token_interrupt = true;
        }

        // Scissor registers (BP 0x20 = TL, 0x21 = BR, 0x59 = OFFSET).
        // An offset change also affects the effective viewport, so we refresh
        // both when the offset register is written.
        if idx == BP_SU_SCIS_TL || idx == BP_SU_SCIS_BR {
            self.recompute_scissor();
            renderer.exec(GxAction::SetScissor(self.cur_scissor));
        } else if idx == BP_SU_SCIS_OFFSET {
            self.recompute_scissor_offset();
            self.recompute_scissor();
            self.rebuild_viewport();
            renderer.exec(GxAction::SetScissor(self.cur_scissor));
            renderer.exec(GxAction::SetViewport(self.cur_viewport));
        }

        // EFB copy trigger (BP 0x52)
        if idx == BP_PE_COPY_CMD {
            self.efb_copy(renderer, ram, val);
        }
    }

    fn snapshot_texture(&mut self, renderer: &mut dyn RenderSink, ram: &[u8], slot: usize, image3_val: u32) {
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

        let width = (image0.width() + 1) as u32;
        let height = (image0.height() + 1) as u32;
        let ram_addr = image3.ram_addr();
        let format = image0.format();

        tracing::debug!(
            slot,
            width,
            height,
            format = format!("{:?}", format),
            ram_addr = format!("{ram_addr:#010X}"),
            wrap_s = format!("{:?}", mode0.wrap_s()),
            wrap_t = format!("{:?}", mode0.wrap_t()),
            "texture descriptor updated"
        );

        let changed = self::texture_data_changed(&mut self.texture_hashes, ram, ram_addr, width, height, format);

        if changed {
            let desc = draw::TextureDescriptor {
                ram_addr,
                width,
                height,
                format,
                wrap_s: mode0.wrap_s(),
                wrap_t: mode0.wrap_t(),
                mag_filter: mode0.mag_filter(),
                min_filter: mode0.min_filter(),
            };
            renderer.exec(GxAction::LoadTexture {
                id: ram_addr as Address,
                width,
                height,
                rgba: texture::decode_to_rgba(ram, &desc),
            });
        }

        self.cur_textures[slot] = Some(draw::TextureDescriptor {
            ram_addr,
            width,
            height,
            format,
            wrap_s: mode0.wrap_s(),
            wrap_t: mode0.wrap_t(),
            mag_filter: mode0.mag_filter(),
            min_filter: mode0.min_filter(),
        });

        renderer.exec(GxAction::SetTexture {
            slot,
            id: ram_addr as Address,
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

    fn efb_copy(&mut self, renderer: &mut dyn RenderSink, ram: &mut [u8], trigger: u32) {
        let src = EfbCopySrc::from_raw(self.bp_regs[BP_PE_COPY_SRC]);
        let dims = EfbCopyDims::from_raw(self.bp_regs[BP_PE_COPY_DIMS]);
        let src_x = src.left() as u32;
        let src_y = src.top() as u32;
        let src_w = dims.width_minus1() as u32 + 1;
        let src_h = dims.height_minus1() as u32 + 1;

        let dest_addr = EfbCopyDst::from_raw(self.bp_regs[BP_PE_COPY_DST]).addr();
        let dest_stride = EfbCopyDstStride::from_raw(self.bp_regs[BP_PE_COPY_DST_STRIDE]).stride() as u32;

        let cmd = PeCopyCmd::from_raw(trigger);
        let copy_to_xfb = cmd.copy_to_xfb();
        let clear = cmd.clear();
        let half = cmd.half();

        let clear_ar = PeClearAr::from_raw(self.bp_regs[BP_PE_COPY_CLEAR_AR]);
        let clear_gb = PeClearGb::from_raw(self.bp_regs[BP_PE_COPY_CLEAR_GB]);
        let a = clear_ar.alpha() as f32 / 255.0;
        let r = clear_ar.red() as f32 / 255.0;
        let g = clear_gb.green() as f32 / 255.0;
        let b = clear_gb.blue() as f32 / 255.0;

        let clear_z = PeClearZ::from_raw(self.bp_regs[BP_PE_COPY_CLEAR_Z]).z() as f32 / 16777215.0;

        tracing::debug!(
            src_x,
            src_y,
            src_w,
            src_h,
            dest_addr = format!("{dest_addr:#010X}"),
            copy_to_xfb,
            clear,
            half,
            "EFB copy triggered"
        );

        if copy_to_xfb {
            // XFB copy: queue the copy for present_xfb() to compose at vblank,
            // and tell the renderer to snapshot the EFB region now.
            let id = self.xfb_copies.len() as u32;
            self.xfb_copies.push(super::XfbCopy {
                dest_addr,
                dest_stride,
                src_h,
            });
            renderer.exec(GxAction::CopyXfb {
                id,
                src_x,
                src_y,
                src_w,
                src_h,
                clear,
                clear_color: [r, g, b, a],
                clear_z,
            });
        } else {
            let copy_format = cmd.copy_format();
            // Per Dolphin BPFunctions::ClearScreen: the `clear` bit only
            // affects channels whose write mask is currently enabled.
            // Carry the masks along so the backend can gate correctly.
            let color_update = self.cur_blend_mode.color_update();
            let alpha_update = self.cur_blend_mode.alpha_update();
            let z_update = self.cur_zmode.update_enable();

            renderer.exec(GxAction::CopyEfbToTexture {
                dest_addr,
                src_x,
                src_y,
                src_w,
                src_h,
                copy_format,
                mipmap: half,
                stride: dest_stride,
                clear,
                clear_color: [r, g, b, a],
                clear_z,
                color_update,
                alpha_update,
                z_update,
            });

            // Block until the renderer finishes the readback + encode and
            // ships the encoded bytes back. This preserves ordering with
            // subsequent FIFO commands (e.g. a TX_SETIMAGE3 at `dest_addr`
            // immediately after this copy): by the time efb_copy returns,
            // RAM is up to date and the next texture load re-hashes.
            if let Some(rx) = &self.efb_writeback_rx {
                match rx.recv_timeout(Duration::from_secs(2)) {
                    Ok(wb) => {
                        let start = wb.dest_addr as usize;
                        let end = start + wb.bytes.len();
                        if end <= ram.len() {
                            ram[start..end].copy_from_slice(&wb.bytes);
                            self.texture_hashes.remove(&wb.dest_addr);
                        } else {
                            tracing::warn!(
                                addr = format!("{:#010X}", wb.dest_addr),
                                len = wb.bytes.len(),
                                "efb writeback OOB, dropping"
                            );
                        }
                    }
                    Err(err) => {
                        tracing::warn!(?err, "efb writeback recv timed out");
                    }
                }
            }
        }
    }

    fn recompute_scissor(&mut self) {
        let tl = SuScisRect::from_raw(self.bp_regs[BP_SU_SCIS_TL]);
        let br = SuScisRect::from_raw(self.bp_regs[BP_SU_SCIS_BR]);

        let tl_x = tl.x() as i32 - 342;
        let tl_y = tl.y() as i32 - 342;
        let br_x = br.x() as i32 - 342 + 1; // BR is inclusive
        let br_y = br.y() as i32 - 342 + 1;

        // BP_SU_SCIS_OFFSET shifts the scissor (and viewport) origin in the
        // EFB. Games use it for tiled rendering: the logical scissor stays
        // the same, but the offset moves where that rectangle lands.
        let eff_tl_x = tl_x - self.cur_scissor_offset_x;
        let eff_tl_y = tl_y - self.cur_scissor_offset_y;
        let eff_br_x = br_x - self.cur_scissor_offset_x;
        let eff_br_y = br_y - self.cur_scissor_offset_y;

        let x = eff_tl_x.max(0) as u32;
        let y = eff_tl_y.max(0) as u32;
        let x2 = (eff_br_x.max(0) as u32).min(EFB_WIDTH);
        let y2 = (eff_br_y.max(0) as u32).min(EFB_HEIGHT);

        self.cur_scissor = draw::Scissor {
            x,
            y,
            w: x2.saturating_sub(x),
            h: y2.saturating_sub(y),
        };
    }

    fn recompute_scissor_offset(&mut self) {
        let reg = SuScisOffset::from_raw(self.bp_regs[BP_SU_SCIS_OFFSET]);
        self.cur_scissor_offset_x = reg.x() as i32 * 2 - 342;
        self.cur_scissor_offset_y = reg.y() as i32 * 2 - 342;
    }
}

/// Returns `true` when the raw texture data in RAM differs from the last
/// hash recorded for the given address.
fn texture_data_changed(
    hashes: &mut std::collections::HashMap<u32, u64>,
    ram: &[u8],
    addr: usize,
    width: u32,
    height: u32,
    format: draw::TextureFormat,
) -> bool {
    let size = texture::raw_data_size(width, height, format);
    let Some(slice) = ram.get(addr..addr + size) else {
        tracing::warn!(addr, size, "texture_changed: RAM slice OOB, assuming changed");
        return true;
    };
    let hash = twox_hash::xxhash3_64::Hasher::oneshot(slice);
    let prev = hashes.insert(addr as u32, hash);
    prev != Some(hash)
}
