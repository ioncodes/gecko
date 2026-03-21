pub mod pad;
pub mod regs;

use crate::mmio::constants::SI_BASE;
use crate::mmio::traits::{MmioAccess, MmioRegister};
use pad::{GC_CONTROLLER_ID, PadStatus};

const NUM_CHANNELS: usize = 4;

#[derive(Clone, Copy, Debug)]
pub struct SiChannel {
    pub out: u32,
    pub in_hi: u32,
    pub in_lo: u32,
}

impl Default for SiChannel {
    fn default() -> Self {
        Self {
            out: 0,
            in_hi: 0,
            in_lo: 0,
        }
    }
}

pub struct Si {
    pub channels: [SiChannel; NUM_CHANNELS],
    pub poll: regs::SiPoll,
    pub comcsr: regs::SiComcsr,
    pub status: regs::SiStatusRegister,
    pub exi_clock_count: u32,
    pub io_buffer: [u8; 128],
    pub pad_state: [PadStatus; NUM_CHANNELS],
}

impl Si {
    pub fn new() -> Self {
        Self {
            channels: [SiChannel::default(); NUM_CHANNELS],
            poll: regs::SiPoll::from_raw(0),
            comcsr: regs::SiComcsr::from_raw(0),
            status: regs::SiStatusRegister::from_raw(0),
            exi_clock_count: 0,
            io_buffer: [0u8; 128],
            pad_state: [PadStatus::default(); NUM_CHANNELS],
        }
    }

    pub fn interrupt_active(&self) -> bool {
        (self.comcsr.tc_interrupt() && self.comcsr.tc_interrupt_mask())
            || (self.comcsr.rdst_interrupt() && self.comcsr.rdst_interrupt_mask())
    }

    crate::impl_mmio_dispatch!(regs::SiPoll, regs::SiComcsr, regs::SiStatusRegister,);

    pub fn mmio_read_u32(&mut self, offset: u32) -> u32 {
        match offset {
            // Channel registers: each channel occupies 0x0C bytes
            0x00..=0x2F => self.read_channel_reg(offset),
            // EXI clock count
            0x3C => self.exi_clock_count,
            // SI I/O buffer
            0x80..=0xFC => {
                let i = (offset - 0x80) as usize;
                u32::from_be_bytes([
                    self.io_buffer[i],
                    self.io_buffer[i + 1],
                    self.io_buffer[i + 2],
                    self.io_buffer[i + 3],
                ])
            }
            // SIPOLL / SICOMCSR / SISR
            _ => self.read_raw(SI_BASE + offset, 4).unwrap_or_else(|| {
                tracing::error!(offset = format!("{offset:08X}"), "unhandled SI read_u32");
                0
            }),
        }
    }

    pub fn mmio_read_u16(&mut self, offset: u32) -> u16 {
        match offset {
            0x00..=0x2F | 0x3C..=0x3E | 0x80..=0xFE => {
                let aligned = offset & !3;
                let word = self.mmio_read_u32(aligned);
                if offset & 2 == 0 {
                    (word >> 16) as u16
                } else {
                    word as u16
                }
            }
            _ => self.read_raw(SI_BASE + offset, 2).unwrap_or_else(|| {
                tracing::error!(offset = format!("{offset:08X}"), "unhandled SI read_u16");
                0
            }) as u16,
        }
    }

    pub fn mmio_read_u8(&mut self, offset: u32) -> u8 {
        match offset {
            0x00..=0x2F | 0x3C..=0x3F | 0x80..=0xFF => {
                let aligned = offset & !3;
                let word = self.mmio_read_u32(aligned);
                let byte_pos = offset & 3;
                ((word >> ((3 - byte_pos) * 8)) & 0xFF) as u8
            }
            _ => self.read_raw(SI_BASE + offset, 1).unwrap_or_else(|| {
                tracing::error!(offset = format!("{offset:08X}"), "unhandled SI read_u8");
                0
            }) as u8,
        }
    }

    pub fn mmio_write_u32(&mut self, offset: u32, val: u32) {
        match offset {
            0x00..=0x2F => self.write_channel_reg(offset, val),
            0x3C => self.exi_clock_count = val,
            0x80..=0xFC => {
                let i = (offset - 0x80) as usize;
                self.io_buffer[i..i + 4].copy_from_slice(&val.to_be_bytes());
            }
            _ => {
                if !self.write_raw(SI_BASE + offset, 4, val) {
                    tracing::error!(offset = format!("{offset:08X}"), "unhandled SI write_u32");
                }
            }
        }
    }

    pub fn mmio_write_u16(&mut self, offset: u32, val: u16) {
        match offset {
            0x00..=0x2F | 0x3C..=0x3E | 0x80..=0xFE => {
                let aligned = offset & !3;
                let mut word = self.read_channel_or_buf_u32_raw(aligned);
                if offset & 2 == 0 {
                    word = (word & 0x0000_FFFF) | ((val as u32) << 16);
                } else {
                    word = (word & 0xFFFF_0000) | (val as u32);
                }
                self.mmio_write_u32(aligned, word);
            }
            _ => {
                if !self.write_raw(SI_BASE + offset, 2, val as u32) {
                    tracing::error!(offset = format!("{offset:08X}"), "unhandled SI write_u16");
                }
            }
        }
    }

    pub fn mmio_write_u8(&mut self, offset: u32, val: u8) {
        match offset {
            0x00..=0x2F | 0x3C..=0x3F | 0x80..=0xFF => {
                let aligned = offset & !3;
                let mut word = self.read_channel_or_buf_u32_raw(aligned);
                let byte_pos = offset & 3;
                let shift = (3 - byte_pos) * 8;
                word = (word & !(0xFF << shift)) | ((val as u32) << shift);
                self.mmio_write_u32(aligned, word);
            }
            _ => {
                if !self.write_raw(SI_BASE + offset, 1, val as u32) {
                    tracing::error!(offset = format!("{offset:08X}"), "unhandled SI write_u8");
                }
            }
        }
    }

    fn read_channel_reg(&mut self, offset: u32) -> u32 {
        let ch = (offset / 0x0C) as usize;
        match offset % 0x0C {
            0x00 => self.channels[ch].out,
            0x04 => {
                self.clear_rdst(ch);
                self.channels[ch].in_hi
            }
            0x08 => {
                self.clear_rdst(ch);
                self.channels[ch].in_lo
            }
            _ => 0,
        }
    }

    fn write_channel_reg(&mut self, offset: u32, val: u32) {
        let ch = (offset / 0x0C) as usize;
        match offset % 0x0C {
            0x00 => self.channels[ch].out = val,
            0x04 => self.channels[ch].in_hi = val,
            0x08 => self.channels[ch].in_lo = val,
            _ => {}
        }
    }

    fn read_channel_or_buf_u32_raw(&self, offset: u32) -> u32 {
        match offset {
            0x00..=0x2F => {
                let ch = (offset / 0x0C) as usize;
                match offset % 0x0C {
                    0x00 => self.channels[ch].out,
                    0x04 => self.channels[ch].in_hi,
                    0x08 => self.channels[ch].in_lo,
                    _ => 0,
                }
            }
            0x3C => self.exi_clock_count,
            0x80..=0xFC => {
                let i = (offset - 0x80) as usize;
                u32::from_be_bytes([
                    self.io_buffer[i],
                    self.io_buffer[i + 1],
                    self.io_buffer[i + 2],
                    self.io_buffer[i + 3],
                ])
            }
            _ => 0,
        }
    }

    fn clear_rdst(&mut self, ch: usize) {
        match ch {
            0 => self.status = self.status.with_rdst0(false),
            1 => self.status = self.status.with_rdst1(false),
            2 => self.status = self.status.with_rdst2(false),
            3 => self.status = self.status.with_rdst3(false),
            _ => {}
        }
    }

    fn set_rdst(&mut self, ch: usize) {
        match ch {
            0 => self.status = self.status.with_rdst0(true),
            1 => self.status = self.status.with_rdst1(true),
            2 => self.status = self.status.with_rdst2(true),
            3 => self.status = self.status.with_rdst3(true),
            _ => {}
        }
    }

    pub fn update_polling(&mut self) {
        let enable = self.poll.enable();
        for ch in 0..NUM_CHANNELS {
            // EN0 is bit 7 of the 4-bit enable field, EN3 is bit 4
            let ch_enabled = (enable >> (3 - ch)) & 1 != 0;
            if ch_enabled && self.pad_state[ch].connected {
                self.channels[ch].in_hi = self.pad_state[ch].encode_hi();
                self.channels[ch].in_lo = self.pad_state[ch].encode_lo();
                self.set_rdst(ch);
            }
        }

        // Update RDST interrupt in COMCSR
        let any_rdst = self.status.rdst0() || self.status.rdst1() || self.status.rdst2() || self.status.rdst3();
        self.comcsr = self.comcsr.with_rdst_interrupt(any_rdst);
    }

    pub fn run_si_buffer(&mut self) {
        let channel = self.comcsr.channel() as usize;
        let cmd = self.io_buffer[0];

        let connected = self.pad_state[channel].connected;

        if connected {
            match cmd {
                // return 3-byte device ID
                0x00 | 0xFF => {
                    self.io_buffer[0] = (GC_CONTROLLER_ID) as u8;
                    self.io_buffer[1] = (GC_CONTROLLER_ID >> 16) as u8;
                    self.io_buffer[2] = (GC_CONTROLLER_ID >> 8) as u8;
                }
                // return current pad data (8 bytes)
                0x40 => {
                    let hi = self.pad_state[channel].encode_hi();
                    let lo = self.pad_state[channel].encode_lo();
                    self.io_buffer[0..4].copy_from_slice(&hi.to_be_bytes());
                    self.io_buffer[4..8].copy_from_slice(&lo.to_be_bytes());
                }
                // return 10-byte origin data / recalibration payload
                0x41 | 0x42 => {
                    let origin = PadStatus::encode_origin();
                    self.io_buffer[0..10].copy_from_slice(&origin);
                }
                _ => {
                    tracing::warn!(cmd = format!("{cmd:02X}"), channel, "unknown SI buffer command");
                }
            }
            self.comcsr = self.comcsr.with_com_error(false);
        } else {
            self.comcsr = self.comcsr.with_com_error(true);
        }

        self.comcsr = self.comcsr.with_tstart(false).with_tc_interrupt(true);
    }

    pub fn send_channel_commands(&mut self) {
        let enable = self.poll.enable();
        for ch in 0..NUM_CHANNELS {
            let ch_enabled = (enable >> (3 - ch)) & 1 != 0;
            if !ch_enabled {
                continue;
            }

            if !self.pad_state[ch].connected {
                match ch {
                    0 => self.status = self.status.with_norep0(true),
                    1 => self.status = self.status.with_norep1(true),
                    2 => self.status = self.status.with_norep2(true),
                    3 => self.status = self.status.with_norep3(true),
                    _ => {}
                }
                continue;
            }
        }
    }
}

impl crate::gekko::Gekko {
    pub fn check_si_interrupts(&mut self) {
        if self.si.interrupt_active() {
            self.pi.assert_interrupt(crate::flipper::pi::InterruptFlag::Si);
        } else {
            self.pi.clear_interrupt(crate::flipper::pi::InterruptFlag::Si);
        }
    }
}
