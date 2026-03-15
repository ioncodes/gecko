pub mod regs;

use crate::mmio::constants::PI_BASE;
use crate::mmio::traits::{MmioAccess, MmioRegister};

// PI FIFO register offsets (from PI_BASE 0xCC003000)
const PI_FIFO_BASE_OFFSET: u32 = 0x0C;
const PI_FIFO_END_OFFSET: u32 = 0x10;
const PI_FIFO_WPTR_OFFSET: u32 = 0x14;

pub struct Pi {
    pub intsr: regs::InterruptCause,
    pub intmr: regs::InterruptMask,
    /// CPU FIFO base address in physical memory
    pub fifo_base: u32,
    /// CPU FIFO end address in physical memory
    pub fifo_end: u32,
    /// CPU FIFO write pointer (32-byte aligned)
    pub fifo_wptr: u32,
    /// The initial FIFO base set by GX_Init (0 = not yet set)
    initial_fifo_base: u32,
}

/// Bitmask constants for each PI interrupt source
#[repr(u32)]
pub enum InterruptFlag {
    Error = 1 << 0,
    Rsw = 1 << 1,
    Di = 1 << 2,
    Si = 1 << 3,
    Exi = 1 << 4,
    Ai = 1 << 5,
    Dsp = 1 << 6,
    Mem = 1 << 7,
    Vi = 1 << 8,
    PeToken = 1 << 9,
    PeFinish = 1 << 10,
    Cp = 1 << 11,
    Debug = 1 << 12,
    Hsp = 1 << 13,
}

impl Pi {
    pub fn new() -> Self {
        Pi {
            intsr: regs::InterruptCause::default(),
            intmr: regs::InterruptMask::from_raw(0),
            fifo_base: 0,
            fifo_end: 0,
            fifo_wptr: 0,
            initial_fifo_base: 0,
        }
    }

    /// Returns true when the CPU FIFO is redirected to a display list buffer
    pub fn is_fifo_redirected(&self) -> bool {
        self.initial_fifo_base != 0 && self.fifo_base != self.initial_fifo_base
    }

    pub fn assert_interrupt(&mut self, flag: InterruptFlag) {
        let raw = self.intsr.raw() | (flag as u32);
        self.intsr = regs::InterruptCause::from_raw(raw);
    }

    pub fn clear_interrupt(&mut self, flag: InterruptFlag) {
        let raw = self.intsr.raw() & !(flag as u32);
        self.intsr = regs::InterruptCause::from_raw(raw);
    }

    pub fn interrupt_pending(&self) -> bool {
        (self.intsr.raw() & self.intmr.raw()) != 0
    }

    crate::impl_mmio_dispatch!(regs::InterruptCause, regs::InterruptMask,);

    pub fn mmio_read_u8(&mut self, offset: u32) -> u8 {
        self.read_raw(PI_BASE + offset, 1).unwrap_or_else(|| {
            tracing::error!(offset = format!("{offset:08X}"), "unhandled PI read_u8");
            0
        }) as u8
    }

    pub fn mmio_write_u8(&mut self, offset: u32, val: u8) {
        if !self.write_raw(PI_BASE + offset, 1, val as u32) {
            tracing::error!(offset = format!("{offset:08X}"), "unhandled PI write_u8");
        }
    }

    pub fn mmio_read_u16(&mut self, offset: u32) -> u16 {
        self.read_raw(PI_BASE + offset, 2).unwrap_or_else(|| {
            tracing::error!(offset = format!("{offset:08X}"), "unhandled PI read_u16");
            0
        }) as u16
    }

    pub fn mmio_write_u16(&mut self, offset: u32, val: u16) {
        if !self.write_raw(PI_BASE + offset, 2, val as u32) {
            tracing::error!(offset = format!("{offset:08X}"), "unhandled PI write_u16");
        }
    }

    pub fn mmio_read_u32(&mut self, offset: u32) -> u32 {
        self.read_raw(PI_BASE + offset, 4).unwrap_or_else(|| {
            tracing::error!(offset = format!("{offset:08X}"), "unhandled PI read_u32");
            0
        })
    }

    pub fn mmio_write_u32(&mut self, offset: u32, val: u32) {
        match offset {
            PI_FIFO_BASE_OFFSET => {
                self.fifo_base = val;
                // Record the first FIFO base as the "normal" GX FIFO
                if self.initial_fifo_base == 0 {
                    self.initial_fifo_base = val;
                }
            }
            PI_FIFO_END_OFFSET => {
                self.fifo_end = val;
            }
            PI_FIFO_WPTR_OFFSET => {
                self.fifo_wptr = val & 0x1FFF_FFE0; // 32-byte aligned
            }
            _ => {
                if !self.write_raw(PI_BASE + offset, 4, val) {
                    tracing::error!(offset = format!("{offset:08X}"), "unhandled PI write_u32");
                }
            }
        }
    }
}
