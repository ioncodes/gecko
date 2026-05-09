use super::constants::*;
use super::regs::*;
use super::{GraphicsProcessor, draw, texture};
use crate::host::{GxAction, RenderSink, TextureKey};
use crate::mmio::{RamView, RamViewMut};
#[cfg(feature = "efb-writeback")]
use std::time::Duration;

impl GraphicsProcessor {
    pub fn load_bp(&mut self, renderer: &mut dyn RenderSink, ram: &mut RamViewMut<'_>, data: &[u8]) {
        let idx = data[0] as usize;
        let val = u32::from_be_bytes([0, data[1], data[2], data[3]]);
        self.bp_regs[idx] = val;

        #[cfg(feature = "gx-stats")]
        {
            self.stats.bp_writes += 1;
        }

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
            self.snapshot_texture(renderer, &ram.as_view(), slot, val);
        }

        // TX_SETTLUT: per-texture-slot binding of the palette (tmem offset +
        // palette pixel format). Layout mirrors TX_SETIMAGE*: slots 0-3 at
        // BP_TX_SETTLUT_I0, slots 4-7 at BP_TX_SETTLUT_I4. Rebinding a TLUT
        // invalidates the cached decode of any paletted texture on that slot,
        // since both palette format and tmem offset feed the decoded pixels.
        let tlut_slot = if idx >= BP_TX_SETTLUT_I0 && idx < BP_TX_SETTLUT_I0 + 4 {
            Some(idx - BP_TX_SETTLUT_I0)
        } else if idx >= BP_TX_SETTLUT_I4 && idx < BP_TX_SETTLUT_I4 + 4 {
            Some(idx - BP_TX_SETTLUT_I4 + 4)
        } else {
            None
        };
        if let Some(slot) = tlut_slot {
            self.cur_tluts[slot] = draw::TlutRef::from_raw(val);
            self.resnapshot_paletted_slot(renderer, &ram.as_view(), slot);
        }

        // LOADTLUT: writing TLUT1 triggers a copy from main RAM (address in
        // TLUT0, stored pre-shifted by 5) into our palette TMEM. Any paletted
        // texture already bound is now looking at fresh palette bytes, so we
        // must redecode it. We don't know which slots' TLUT regions overlap
        // the load, so rescan all bound paletted slots (saw this in Dolphin @
        // TMEM::InvalidateAll on LOADTLUT1).
        if idx == BP_LOAD_TLUT1 {
            self.load_tlut(&ram.as_view(), val);
            for slot in 0..self.cur_textures.len() {
                self.resnapshot_paletted_slot(renderer, &ram.as_view(), slot);
            }
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
            BP_PE_ZCOMPARE => {
                self.cur_pe_control = PeControl::from_raw(val);
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

        // GEN_MODE, extract num TEV stages + cull mode + num indirect stages
        if idx == BP_GEN_MODE {
            let gen_mode = GenMode::from_raw(val);
            let stages = gen_mode.num_tev_stages() + 1;
            let indirect_stages = gen_mode.num_ind_stages();
            tracing::debug!(
                num_tev_stages = stages,
                num_indirect_stages = indirect_stages,
                "GEN_MODE"
            );
            self.cur_num_tev_stages = stages;
            self.cur_num_indirect_stages = indirect_stages;
            self.konst_dirty = true;
            renderer.exec(GxAction::SetCullMode(gen_mode.cull_mode()));
        }

        // Indirect-matrix rows at 0x06..=0x0E. Each write lands in one
        // of the A, B, or C rows of one of three matrices.
        match idx {
            BP_IND_MTX_A0 => self.cur_indirect_matrices[0].a = val,
            BP_IND_MTX_B0 => self.cur_indirect_matrices[0].b = val,
            BP_IND_MTX_C0 => self.cur_indirect_matrices[0].c = val,
            BP_IND_MTX_A1 => self.cur_indirect_matrices[1].a = val,
            BP_IND_MTX_B1 => self.cur_indirect_matrices[1].b = val,
            BP_IND_MTX_C1 => self.cur_indirect_matrices[1].c = val,
            BP_IND_MTX_A2 => self.cur_indirect_matrices[2].a = val,
            BP_IND_MTX_B2 => self.cur_indirect_matrices[2].b = val,
            BP_IND_MTX_C2 => self.cur_indirect_matrices[2].c = val,
            _ => {}
        }

        // TODO: BumpIMask is captured for measure but never read on the
        // GPU side. AFAICT it's unused. Dolphin agrees.
        if idx == BP_BUMP_IMASK {
            self.cur_bump_imask = val;
        }

        // Per TEV stage indirect commands at 0x10..=0x1F.
        if idx >= BP_IND_CMD_0 && idx < BP_IND_CMD_0 + BP_IND_CMD_COUNT {
            self.cur_tev_indirect[idx - BP_IND_CMD_0] = TevIndirect::from_raw(val);
        }

        // Indirect texcoord scale pair. SS0 covers indirect stages 0-1,
        // SS1 covers 2-3.
        if idx == BP_RAS1_SS0 {
            self.cur_indirect_scales[0] = Ras1Ss::from_raw(val);
        } else if idx == BP_RAS1_SS1 {
            self.cur_indirect_scales[1] = Ras1Ss::from_raw(val);
        }

        if idx == BP_RAS1_IREF {
            self.cur_indirect_refs = Ras1IRef::from_raw(val);
        }

        // TEV KSEL (constant color selection) registers: recalculate konst colors
        if idx >= BP_TEV_KSEL_0 && idx <= BP_TEV_KSEL_0 + 7 {
            self.konst_dirty = true;
        }

        // TEV color register writes also affect konst colors
        if idx >= BP_TEV_REGISTERL_0 && idx <= BP_TEV_REGISTERL_0 + 7 {
            self.konst_dirty = true;
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

    fn snapshot_texture(&mut self, renderer: &mut dyn RenderSink, ram: &RamView<'_>, slot: usize, image3_val: u32) {
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

        let tlut = self.cur_tluts[slot];
        let palette = slot_palette(&self.palette_mem, tlut.tmem_offset);

        // Cache id mixes a hash of the bound palette content + tlut identity
        // into the high 32 bits so each distinct palette state for a CI*
        // texture lands in its own renderer cache slot. Without that, games
        // (FFCC title logo) that load several palettes per frame at the same
        // tmem_offset would silently let the latest load clobber the earlier
        // ones, since bind groups are built lazily and resolve to whichever
        // GPU texture is current at render-pass time.
        let cache_id = self::texture_cache_id(ram_addr, format, tlut, palette);

        // Resolve the texture's raw bytes once, against MEM1 or MEM2. If the
        // address doesn't fall in either bank, leave the slot's last binding
        // alone but skip the decode (the renderer will keep its previous
        // texture for this id).
        let raw_size = texture::raw_data_size(width, height, format);
        let tex_slice = ram.slice(ram_addr, raw_size);

        let changed = match tex_slice {
            Some(tex) => self::texture_data_changed(&mut self.texture_hashes, tex, cache_id, palette, tlut, format),
            None => {
                tracing::warn!(
                    addr = format!("{ram_addr:#010X}"),
                    raw_size,
                    "texture: address not mapped to MEM1/MEM2, skipping decode"
                );
                false
            }
        };

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

            #[cfg(feature = "gx-stats")]
            {
                self.stats.texture_loads += 1;
            }

            renderer.exec(GxAction::LoadTexture {
                id: cache_id,
                width,
                height,
                fmt: format,
                rgba: texture::decode_to_rgba(tex_slice.unwrap(), &desc, palette, tlut.format),
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
            id: cache_id,
            wrap_s: mode0.wrap_s(),
            wrap_t: mode0.wrap_t(),
            mag_filter: mode0.mag_filter(),
            min_filter: mode0.min_filter(),
        });
    }

    /// If the slot currently binds a paletted texture, rerun the snapshot
    /// pipeline so `texture_data_changed` rehashes the (now possibly fresh)
    /// palette bytes + TLUT format and decodes a new RGBA if anything moved.
    fn resnapshot_paletted_slot(&mut self, renderer: &mut dyn RenderSink, ram: &RamView<'_>, slot: usize) {
        let Some(desc) = self.cur_textures[slot] else {
            return;
        };
        if !desc.format.is_paletted() {
            return;
        }
        let image3_reg = if slot < 4 {
            BP_TX_SETIMAGE3_I0 + slot
        } else {
            BP_TX_SETIMAGE3_I4 + (slot - 4)
        };
        let image3_val = self.bp_regs[image3_reg];
        self.snapshot_texture(renderer, ram, slot, image3_val);
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

    fn efb_copy(&mut self, renderer: &mut dyn RenderSink, ram: &mut RamViewMut<'_>, trigger: u32) {
        let src = EfbCopySrc::from_raw(self.bp_regs[BP_PE_COPY_SRC]);
        let dims = EfbCopyDims::from_raw(self.bp_regs[BP_PE_COPY_DIMS]);
        let src_x = src.left() as u32;
        let src_y = src.top() as u32;
        let src_w = dims.width_minus1() as u32 + 1;
        let src_h = dims.height_minus1() as u32 + 1;

        let dest_addr = EfbCopyDst::from_raw(self.bp_regs[BP_PE_COPY_DST]).addr();
        let dest_stride = EfbCopyDstStride::from_raw(self.bp_regs[BP_PE_COPY_DST_STRIDE]).stride_bytes();

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
            let yscale_reg = DispCopyYScale::from_raw(self.bp_regs[BP_PE_COPY_YSCALE]).scale() as f32;
            let y_scale = if cmd.scale_invert() {
                if yscale_reg == 0.0 { 1.0 } else { 256.0 / yscale_reg }
            } else {
                yscale_reg / 256.0
            };
            let y_scale = if y_scale.is_finite() && y_scale > 0.0 {
                y_scale
            } else {
                1.0
            };
            let dst_h = (1.0 + dims.height_minus1() as f32 * y_scale).floor().max(1.0) as u32;
            let gamma = match cmd.gamma() {
                0 => 1.0,
                1 => 1.7,
                2 | 3 => 2.2,
                _ => 1.0,
            };
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
                dst_h,
                gamma,
                clear,
                clear_color: [r, g, b, a],
                clear_z,
                color_update: self.cur_blend_mode.color_update(),
                alpha_update: self.cur_blend_mode.alpha_update(),
                z_update: self.cur_zmode.update_enable(),
                alpha_supported: self.cur_pe_control.pixel_format().has_alpha(),
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
                alpha_supported: self.cur_pe_control.pixel_format().has_alpha(),
                depth_copy: self.cur_pe_control.pixel_format().is_depth_only(),
            });

            // Writeback overwrites RAM, so drop every cached hash that
            // points at this ram_addr (any TLUT variant) to force a redecode.
            #[cfg(feature = "efb-writeback")]
            self.texture_hashes.retain(|k, _| k.ram_addr != dest_addr);
            let _ = ram;

            // With `efb-writeback`: block until the renderer finishes the
            // readback + encode and ships the encoded bytes back. This
            // preserves ordering with subsequent FIFO commands (a
            // `TX_SETIMAGE3` at `dest_addr` immediately after this copy
            // sees up-to-date RAM + a freshly re-hashable texture).
            #[cfg(feature = "efb-writeback")]
            if let Some(rx) = &self.efb_writeback_rx {
                match rx.recv_timeout(Duration::from_secs(2)) {
                    Ok(wb) => {
                        texture::write_strided_copy_to_ram(
                            ram,
                            wb.dest_addr,
                            &wb.bytes,
                            wb.row_bytes,
                            wb.row_count,
                            wb.dest_stride_bytes,
                        );
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

fn slot_palette(palette_mem: &[u16], tmem_offset: u16) -> &[u16] {
    let base = (tmem_offset as usize) * TLUT_ENTRIES_PER_UNIT;
    palette_mem.get(base..).unwrap_or(&[])
}

/// Mint the renderer cache key for the given texture binding.
///
/// `variant` is `0` for non-paletted formats. For paletted (CI*) formats it's
/// a 32-bit hash of `(palette content, tlut.format, tmem_offset)` so the
/// same RAM index stream sampled through different palettes lands
/// in distinct cache slots. FFCC's title-logo fade animation alternates
/// between palettes at a fixed `tmem_offset`; without the variant in the
/// key, later uploads would silently overwrite earlier ones in the
/// renderer's cache and bind groups built at render-pass time would all
/// resolve to whichever decode landed last.
#[inline(always)]
fn texture_cache_id(ram_addr: usize, format: draw::TextureFormat, tlut: draw::TlutRef, palette: &[u16]) -> TextureKey {
    let variant = if format.is_paletted() {
        let max_entries = match format {
            draw::TextureFormat::CI4 => 16,
            draw::TextureFormat::CI8 => 256,
            draw::TextureFormat::CI14 => 16384,
            _ => 0,
        };
        let take = max_entries.min(palette.len());
        let palette_bytes: &[u8] = bytemuck::cast_slice(&palette[..take]);
        let mut h = twox_hash::xxhash3_64::Hasher::oneshot(palette_bytes);
        h ^= tlut.format as u64;
        h ^= (tlut.tmem_offset as u64) << 8;
        // Fold the 64-bit hash to 32 bits.
        (h ^ (h >> 32)) as u32
    } else {
        0
    };
    TextureKey {
        ram_addr: ram_addr as u32,
        variant,
    }
}

/// Returns `true` when the raw texture data in `tex` differs from the last
/// hash recorded for this cache key. `tex` is the already resolved slice for
/// the texture (caller has already mapped MEM1/MEM2). The hash is keyed by
/// `cache_id` so paletted textures bound with multiple TLUTs are tracked
/// independently.
fn texture_data_changed(
    hashes: &mut rustc_hash::FxHashMap<TextureKey, u64>,
    tex: &[u8],
    cache_id: TextureKey,
    palette: &[u16],
    tlut: draw::TlutRef,
    format: draw::TextureFormat,
) -> bool {
    let mut hash = twox_hash::xxhash3_64::Hasher::oneshot(tex);
    if format.is_paletted() {
        let max_entries = match format {
            draw::TextureFormat::CI4 => 16,
            draw::TextureFormat::CI8 => 256,
            draw::TextureFormat::CI14 => 16384,
            _ => 0,
        };
        let take = max_entries.min(palette.len());
        let palette_bytes: &[u8] = bytemuck::cast_slice(&palette[..take]);
        hash ^= twox_hash::xxhash3_64::Hasher::oneshot(palette_bytes);
        // Palette pixel format is part of the visual state too, so fold it
        // into the hash or a format switch alone wouldn't force a redecode.
        hash ^= tlut.format as u64;
    }
    let prev = hashes.insert(cache_id, hash);
    prev != Some(hash)
}

impl GraphicsProcessor {
    /// Service a write to BP_LOAD_TLUT1: pulls `count * 16` big-endian u16
    /// palette entries from main RAM (source address is BP_LOAD_TLUT0 << 5)
    /// into palette TMEM starting at `tmem_offset * 256`. The source address
    /// can fall in either MEM1 or MEM2 on Wii titles.
    fn load_tlut(&mut self, ram: &RamView<'_>, load_val: u32) {
        let tmem_offset = (load_val & 0x3FF) as usize;
        let count = ((load_val >> 10) & 0x7FF) as usize;
        let ram_base = (self.bp_regs[BP_LOAD_TLUT0] as usize) << 5;
        let entries = count * TLUT_LOAD_ENTRIES_PER_UNIT;
        let byte_count = entries * 2;
        let dst_base = tmem_offset * TLUT_ENTRIES_PER_UNIT;

        if entries == 0 {
            return;
        }
        let Some(src) = ram.slice(ram_base, byte_count) else {
            tracing::warn!(
                ram_base = format!("{ram_base:#010X}"),
                byte_count,
                "LOADTLUT: source address not mapped to MEM1/MEM2, skipping"
            );
            return;
        };
        if dst_base.saturating_add(entries) > self.palette_mem.len() {
            tracing::warn!(
                dst_base,
                entries,
                tmem_limit = self.palette_mem.len(),
                "LOADTLUT: destination palette range OOB, skipping"
            );
            return;
        }

        let dst = &mut self.palette_mem[dst_base..dst_base + entries];
        for (entry, chunk) in dst.iter_mut().zip(src.chunks_exact(2)) {
            *entry = u16::from_be_bytes([chunk[0], chunk[1]]);
        }

        tracing::debug!(ram_base = format!("{ram_base:#010X}"), tmem_offset, entries, "LOADTLUT");
    }
}
