pub mod regs;

#[derive(Clone, Copy)]
pub struct ProcessorInterface {
    pub intsr: regs::InterruptCause,
    pub intmr: regs::InterruptMask,
    pub reset_code: regs::ResetCode,
    /// CPU FIFO base address in physical memory
    pub fifo_base: u32,
    /// CPU FIFO end address in physical memory
    pub fifo_end: u32,
    /// CPU FIFO write pointer (32 byte aligned)
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
    Hw = 1 << 14,
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

crate::mmio_device_dispatch! {
    read = pi_read,
    write = pi_write,
    registers = [
        regs::InterruptCause,
        regs::InterruptMask,
        regs::FifoBase,
        regs::FifoEnd,
        regs::FifoWritePtr,
        regs::ResetCode,
        regs::FlipperRev,
    ],
}
