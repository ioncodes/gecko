pub mod regs;

use crate::mmio::constants::PI_BASE;
use crate::mmio::traits::{MmioAccess, MmioRegister, MmioRw};

// PI FIFO register offsets (from PI_BASE 0xCC003000)
const PI_FIFO_BASE_OFFSET: u32 = 0x0C;
const PI_FIFO_END_OFFSET: u32 = 0x10;
const PI_FIFO_WPTR_OFFSET: u32 = 0x14;

pub struct ProcessorInterface {
    pub intsr: regs::InterruptCause,
    pub intmr: regs::InterruptMask,
    pub reset_code: regs::ResetCode,
    /// CPU FIFO base address in physical memory
    pub fifo_base: u32,
    /// CPU FIFO end address in physical memory
    pub fifo_end: u32,
    /// CPU FIFO write pointer (32-byte aligned)
    pub fifo_wptr: u32,
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

impl ProcessorInterface {
    pub fn new() -> Self {
        ProcessorInterface {
            intsr: regs::InterruptCause::default(),
            intmr: regs::InterruptMask::from_raw(0),
            reset_code: regs::ResetCode::from_raw(0),
            fifo_base: 0,
            fifo_end: 0,
            fifo_wptr: 0,
        }
    }

    pub fn advance_fifo_wptr(&mut self, nbytes: u32) {
        self.fifo_wptr = self.fifo_wptr.wrapping_add(nbytes);
        if self.fifo_end != 0 && self.fifo_wptr >= self.fifo_end {
            self.fifo_wptr = self.fifo_base;
        }
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
}

impl MmioRw for ProcessorInterface {
    const BASE: u32 = PI_BASE;
    const NAME: &'static str = "PI";

    crate::impl_mmio_dispatch!(
        regs::InterruptCause,
        regs::InterruptMask,
        regs::ResetCode,
        regs::FlipperRev,
    );

    fn mmio_write_u32(&mut self, offset: u32, val: u32) {
        match offset {
            PI_FIFO_BASE_OFFSET => {
                self.fifo_base = val;
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
