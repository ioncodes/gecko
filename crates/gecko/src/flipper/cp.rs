pub mod regs;

use crate::system::{System, SystemId};

pub struct CommandProcessor {
    pub status: regs::CpStatus,
    pub control: regs::CpControl,
    pub fifo_base_lo: regs::FifoBaseLo,
    pub fifo_base_hi: regs::FifoBaseHi,
    pub fifo_end_lo: regs::FifoEndLo,
    pub fifo_end_hi: regs::FifoEndHi,
    pub fifo_hi_watermark_lo: regs::FifoHiWatermarkLo,
    pub fifo_hi_watermark_hi: regs::FifoHiWatermarkHi,
    pub fifo_lo_watermark_lo: regs::FifoLoWatermarkLo,
    pub fifo_lo_watermark_hi: regs::FifoLoWatermarkHi,
    pub fifo_rw_distance_lo: regs::FifoRwDistanceLo,
    pub fifo_rw_distance_hi: regs::FifoRwDistanceHi,
    pub fifo_write_ptr_lo: regs::FifoWritePtrLo,
    pub fifo_write_ptr_hi: regs::FifoWritePtrHi,
    pub fifo_read_ptr_lo: regs::FifoReadPtrLo,
    pub fifo_read_ptr_hi: regs::FifoReadPtrHi,
}

impl CommandProcessor {
    pub fn new() -> Self {
        Self {
            status: regs::CpStatus::from_raw(0),
            control: regs::CpControl::from_raw(0),
            fifo_base_lo: regs::FifoBaseLo::from_raw(0),
            fifo_base_hi: regs::FifoBaseHi::from_raw(0),
            fifo_end_lo: regs::FifoEndLo::from_raw(0),
            fifo_end_hi: regs::FifoEndHi::from_raw(0),
            fifo_hi_watermark_lo: regs::FifoHiWatermarkLo::from_raw(0),
            fifo_hi_watermark_hi: regs::FifoHiWatermarkHi::from_raw(0),
            fifo_lo_watermark_lo: regs::FifoLoWatermarkLo::from_raw(0),
            fifo_lo_watermark_hi: regs::FifoLoWatermarkHi::from_raw(0),
            fifo_rw_distance_lo: regs::FifoRwDistanceLo::from_raw(0),
            fifo_rw_distance_hi: regs::FifoRwDistanceHi::from_raw(0),
            fifo_write_ptr_lo: regs::FifoWritePtrLo::from_raw(0),
            fifo_write_ptr_hi: regs::FifoWritePtrHi::from_raw(0),
            fifo_read_ptr_lo: regs::FifoReadPtrLo::from_raw(0),
            fifo_read_ptr_hi: regs::FifoReadPtrHi::from_raw(0),
        }
    }

    #[inline(always)]
    pub fn interrupt_active(&self) -> bool {
        (self.status.bp_interrupt() && self.control.bp_interrupt_enable())
            || (self.status.fifo_overflow() && self.control.fifo_overflow_interrupt_enable())
            || (self.status.fifo_underflow() && self.control.fifo_underflow_interrupt_enable())
    }
}

crate::mmio_device_dispatch! {
    read = cp_read,
    write = cp_write,
    registers = [
        regs::CpStatus,
        regs::CpControl,
        regs::CpClear,
        regs::FifoBaseLo,
        regs::FifoBaseHi,
        regs::FifoEndLo,
        regs::FifoEndHi,
        regs::FifoHiWatermarkLo,
        regs::FifoHiWatermarkHi,
        regs::FifoLoWatermarkLo,
        regs::FifoLoWatermarkHi,
        regs::FifoRwDistanceLo,
        regs::FifoRwDistanceHi,
        regs::FifoWritePtrLo,
        regs::FifoWritePtrHi,
        regs::FifoReadPtrLo,
        regs::FifoReadPtrHi,
    ],
}

#[inline(always)]
pub fn refresh_interrupts<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    use crate::flipper::pi::InterruptFlag;

    if sys.cp.interrupt_active() {
        sys.pi.assert_interrupt(InterruptFlag::Cp);
    } else {
        sys.pi.clear_interrupt(InterruptFlag::Cp);
    }
}
