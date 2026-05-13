pub mod pad;
pub mod regs;

use crate::mmio::constants::SI_BASE;
use crate::system::{System, SystemId};
use pad::{GC_CONTROLLER_ID, PadStatus};

const NUM_CHANNELS: usize = 4;

#[derive(Clone, Copy, Debug)]
pub struct SerialChannel {
    pub out: u32,
    pub in_hi: u32,
    pub in_lo: u32,
}

impl Default for SerialChannel {
    fn default() -> Self {
        Self {
            out: 0,
            in_hi: 0,
            in_lo: 0,
        }
    }
}

pub struct SerialInterface {
    pub channels: [SerialChannel; NUM_CHANNELS],
    pub poll: regs::SiPoll,
    pub comcsr: regs::SiComcsr,
    pub status: regs::SiStatusRegister,
    pub exi_clock_count: u32,
    pub io_buffer: [u8; 128],
    pub pad_state: [PadStatus; NUM_CHANNELS],
}

impl SerialInterface {
    pub fn new() -> Self {
        Self {
            channels: [SerialChannel::default(); NUM_CHANNELS],
            poll: regs::SiPoll::from_raw(0),
            comcsr: regs::SiComcsr::from_raw(0),
            status: regs::SiStatusRegister::from_raw(0),
            exi_clock_count: 0,
            io_buffer: [0u8; 128],
            pad_state: [PadStatus::default(); NUM_CHANNELS],
        }
    }

    #[inline(always)]
    pub fn interrupt_active(&self) -> bool {
        (self.comcsr.tc_interrupt() && self.comcsr.tc_interrupt_mask())
            || (self.comcsr.rdst_interrupt() && self.comcsr.rdst_interrupt_mask())
    }

    #[inline(always)]
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

    #[inline(always)]
    fn write_channel_reg(&mut self, offset: u32, val: u32) {
        let ch = (offset / 0x0C) as usize;
        match offset % 0x0C {
            0x00 => self.channels[ch].out = val,
            0x04 => self.channels[ch].in_hi = val,
            0x08 => self.channels[ch].in_lo = val,
            _ => {}
        }
    }

    #[inline(always)]
    fn channel_reg_raw(&self, aligned_offset: u32) -> u32 {
        let ch = (aligned_offset / 0x0C) as usize;
        match aligned_offset % 0x0C {
            0x00 => self.channels[ch].out,
            0x04 => self.channels[ch].in_hi,
            0x08 => self.channels[ch].in_lo,
            _ => 0,
        }
    }

    #[inline(always)]
    fn clear_rdst(&mut self, ch: usize) {
        match ch {
            0 => self.status = self.status.with_rdst0(false),
            1 => self.status = self.status.with_rdst1(false),
            2 => self.status = self.status.with_rdst2(false),
            3 => self.status = self.status.with_rdst3(false),
            _ => {}
        }
    }

    #[inline(always)]
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
                // Return 3 byte device ID.
                0x00 | 0xFF => {
                    self.io_buffer[0] = (GC_CONTROLLER_ID >> 24) as u8;
                    self.io_buffer[1] = (GC_CONTROLLER_ID >> 16) as u8;
                    self.io_buffer[2] = (GC_CONTROLLER_ID >> 8) as u8;
                }
                // Return current pad data (8 bytes).
                0x40 => {
                    let hi = self.pad_state[channel].encode_hi();
                    let lo = self.pad_state[channel].encode_lo();
                    self.io_buffer[0..4].copy_from_slice(&hi.to_be_bytes());
                    self.io_buffer[4..8].copy_from_slice(&lo.to_be_bytes());
                }
                // Return 10 byte origin data / recalibration payload.
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

crate::mmio_device_dispatch! {
    read = si_regs_read,
    write = si_regs_write,
    registers = [
        regs::SiPoll,
        regs::SiComcsr,
        regs::SiStatusRegister,
    ],
}

#[inline(always)]
pub fn si_read<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, phys: u32, size: u32) -> Option<u32> {
    let offset = phys - SI_BASE;

    if (0x80..=0xFF).contains(&offset) {
        let i = (offset - 0x80) as usize;
        return Some(match size {
            1 => sys.si.io_buffer[i] as u32,
            2 => u16::from_be_bytes([sys.si.io_buffer[i], sys.si.io_buffer[i + 1]]) as u32,
            4 => u32::from_be_bytes([
                sys.si.io_buffer[i],
                sys.si.io_buffer[i + 1],
                sys.si.io_buffer[i + 2],
                sys.si.io_buffer[i + 3],
            ]),
            _ => return None,
        });
    }

    if (0x00..=0x2F).contains(&offset) || (0x3C..=0x3F).contains(&offset) {
        let aligned = offset & !3;
        let word = match aligned {
            0x00..=0x2C => sys.si.read_channel_reg(aligned),
            0x3C => sys.si.exi_clock_count,
            _ => return None,
        };
        return Some(crate::mmio::traits::read_be_subword(word, offset & 3, size));
    }

    self::si_regs_read(sys, phys, size)
}

#[inline(always)]
pub fn si_write<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, phys: u32, size: u32, val: u32) -> bool {
    let offset = phys - SI_BASE;

    if (0x80..=0xFF).contains(&offset) {
        let i = (offset - 0x80) as usize;
        match size {
            1 => sys.si.io_buffer[i] = val as u8,
            2 => sys.si.io_buffer[i..i + 2].copy_from_slice(&(val as u16).to_be_bytes()),
            4 => sys.si.io_buffer[i..i + 4].copy_from_slice(&val.to_be_bytes()),
            _ => return false,
        }
        return true;
    }

    if (0x00..=0x2F).contains(&offset) || (0x3C..=0x3F).contains(&offset) {
        let aligned = offset & !3;
        let merged = if size == 4 {
            val
        } else {
            let current = match aligned {
                0x00..=0x2C => sys.si.channel_reg_raw(aligned),
                0x3C => sys.si.exi_clock_count,
                _ => return false,
            };
            crate::mmio::traits::write_be_subword(current, offset & 3, size, val)
        };
        match aligned {
            0x00..=0x2C => sys.si.write_channel_reg(aligned, merged),
            0x3C => sys.si.exi_clock_count = merged,
            _ => return false,
        }
        return true;
    }

    self::si_regs_write(sys, phys, size, val)
}

#[inline(always)]
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn refresh_interrupts<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    use crate::flipper::pi::InterruptFlag;

    if sys.si.interrupt_active() {
        sys.pi.assert_interrupt(InterruptFlag::Si);
    } else {
        sys.pi.clear_interrupt(InterruptFlag::Si);
    }
}
