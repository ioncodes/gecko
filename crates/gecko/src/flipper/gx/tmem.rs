use super::bp::tx_slot_reg;
use super::constants::*;
use super::draw::TextureFormat;
use super::recorder::MemoryUpdateType;
use super::regs::{PreloadAddress, PreloadMode, TxSetImage0, TxSetImage1, TxSetImage2};
use super::{GraphicsProcessor, texture};
use crate::mmio::RamView;
use crate::savestate::{StateError, StateReader, StateWriter};

impl GraphicsProcessor {
    pub fn load_tmem(&mut self, bytes: &[u8]) {
        self.tmem.fill(0);
        self.write_tmem(0, bytes);
        self.invalidate_tmem();
    }

    pub(crate) fn save_tmem_tail(&self, w: &mut StateWriter) {
        w.bytes(bytemuck::cast_slice(&self.tmem[TLUT_MEM_ENTRIES..]));
    }

    pub(crate) fn load_tmem_tail(&mut self, r: &mut StateReader<'_>, present: bool) -> Result<(), StateError> {
        let tail = &mut self.tmem[TLUT_MEM_ENTRIES..];
        if !present {
            tail.fill(0);
            return Ok(());
        }

        r.bytes_into(bytemuck::cast_slice_mut(tail))
    }

    pub fn texture_is_preloaded(&self, slot: usize) -> bool {
        TxSetImage1::from_raw(self.bp_regs[tx_slot_reg(BP_TX_SETIMAGE1_I0, BP_TX_SETIMAGE1_I4, slot)])
            .manually_managed()
    }

    pub(crate) fn texture_tmem_bases(&self, slot: usize) -> [usize; 2] {
        let image1 = TxSetImage1::from_raw(self.bp_regs[tx_slot_reg(BP_TX_SETIMAGE1_I0, BP_TX_SETIMAGE1_I4, slot)]);
        let image2 = TxSetImage2::from_raw(self.bp_regs[tx_slot_reg(BP_TX_SETIMAGE2_I0, BP_TX_SETIMAGE2_I4, slot)]);
        [image1.tmem_even(), image2.tmem_odd()].map(|line| usize::from(line) * TMEM_LINE_SIZE)
    }

    pub(crate) fn invalidate_tmem(&mut self) {
        self.tmem_generation = self.tmem_generation.wrapping_add(1);

        for slot in 0..self.cur_textures.len() {
            let image0 = TxSetImage0::from_raw(self.bp_regs[tx_slot_reg(BP_TX_SETIMAGE0_I0, BP_TX_SETIMAGE0_I4, slot)]);
            if image0.format().is_paletted() || self.texture_is_preloaded(slot) {
                self.tex_dirty |= 1 << slot;
            }
        }
    }

    pub(crate) fn preload_texture(&mut self, ram: &RamView<'_>, mode: PreloadMode) {
        let split_banks = mode.split_banks();
        let src = PreloadAddress::from_raw(self.bp_regs[BP_PRELOAD_ADDR]).byte_address();
        let even = PreloadAddress::from_raw(self.bp_regs[BP_PRELOAD_TMEM_EVEN]).byte_address();
        let odd = PreloadAddress::from_raw(self.bp_regs[BP_PRELOAD_TMEM_ODD]).byte_address();
        let len = usize::from(mode.count()) * TMEM_LINE_SIZE * if split_banks { 2 } else { 1 };

        if len == 0 {
            return;
        }
        let Some(bytes) = ram.slice(src, len) else {
            return;
        };

        if let Some(rec) = self.recorder.as_deref_mut() {
            rec.use_memory(ram, src as u32, len, MemoryUpdateType::Tmem);
        }

        if split_banks {
            for (i, tile) in bytes.chunks_exact(2 * TMEM_LINE_SIZE).enumerate() {
                self.write_tmem(even + i * TMEM_LINE_SIZE, &tile[..TMEM_LINE_SIZE]);
                self.write_tmem(odd + i * TMEM_LINE_SIZE, &tile[TMEM_LINE_SIZE..]);
            }
        } else {
            self.write_tmem(even, bytes);
        }

        self.invalidate_tmem();
    }

    pub(super) fn write_tmem(&mut self, addr: usize, bytes: &[u8]) {
        let Some(dst) = self.tmem.get_mut(addr / 2..) else {
            return;
        };

        for (entry, pair) in dst.iter_mut().zip(bytes.chunks_exact(2)) {
            *entry = u16::from_be_bytes([pair[0], pair[1]]);
        }
    }

    fn read_tmem(&self, addr: usize, len: usize, out: &mut Vec<u8>) -> Option<()> {
        let entries = self.tmem.get(addr / 2..(addr + len) / 2)?;
        out.extend(entries.iter().flat_map(|entry| entry.to_be_bytes()));
        Some(())
    }

    pub(crate) fn preloaded_texture_bytes(
        &self,
        slot: usize,
        width: u32,
        height: u32,
        format: TextureFormat,
        levels: u32,
    ) -> Option<Vec<u8>> {
        let mut bases = self.texture_tmem_bases(slot);
        let mut out = Vec::with_capacity(texture::mip_data_size(width, height, format, levels));

        for (level, (w, h)) in texture::mip_dimensions(width, height, levels).enumerate() {
            let len = texture::raw_data_size(w, h, format);
            if format == TextureFormat::RGBA8 {
                for _ in 0..len / (2 * TMEM_LINE_SIZE) {
                    for base in &mut bases {
                        self.read_tmem(*base, TMEM_LINE_SIZE, &mut out)?;
                        *base += TMEM_LINE_SIZE;
                    }
                }
            } else {
                let base = &mut bases[level % 2];
                self.read_tmem(*base, len, &mut out)?;
                *base += len;
            }
        }

        Some(out)
    }
}
