pub mod regs;

use crate::flipper::pi::InterruptFlag;
use crate::system::{System, SystemId};

pub const GP_BURST: u32 = 32;

const GP_PIPE_CAPACITY: usize = 64;

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
    pub fifo_bp_lo: regs::FifoBpLo,
    pub fifo_bp_hi: regs::FifoBpHi,

    pub gather_pipe: [u8; GP_PIPE_CAPACITY],
    pub gather_pos: u32,
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
            fifo_bp_lo: regs::FifoBpLo::from_raw(0),
            fifo_bp_hi: regs::FifoBpHi::from_raw(0),
            gather_pipe: [0; GP_PIPE_CAPACITY],
            gather_pos: 0,
        }
    }

    #[inline(always)]
    pub fn interrupt_active(&self) -> bool {
        (self.status.bp_interrupt() && self.control.bp_interrupt_enable())
            || (self.status.fifo_overflow() && self.control.fifo_overflow_interrupt_enable())
            || (self.status.fifo_underflow() && self.control.fifo_underflow_interrupt_enable())
    }

    #[inline(always)]
    pub fn fifo_base(&self) -> u32 {
        (((self.fifo_base_hi.raw() as u32) << 16) | (self.fifo_base_lo.raw() as u32)) & !0x1F
    }

    #[inline(always)]
    pub fn fifo_end(&self) -> u32 {
        (((self.fifo_end_hi.raw() as u32) << 16) | (self.fifo_end_lo.raw() as u32)) & !0x1F
    }

    #[inline(always)]
    pub fn fifo_hi_watermark(&self) -> u32 {
        ((self.fifo_hi_watermark_hi.raw() as u32) << 16) | (self.fifo_hi_watermark_lo.raw() as u32)
    }

    #[inline(always)]
    pub fn fifo_lo_watermark(&self) -> u32 {
        ((self.fifo_lo_watermark_hi.raw() as u32) << 16) | (self.fifo_lo_watermark_lo.raw() as u32)
    }

    #[inline(always)]
    pub fn fifo_rw_distance(&self) -> u32 {
        ((self.fifo_rw_distance_hi.raw() as u32) << 16) | (self.fifo_rw_distance_lo.raw() as u32)
    }

    #[inline(always)]
    pub fn fifo_write_ptr(&self) -> u32 {
        (((self.fifo_write_ptr_hi.raw() as u32) << 16) | (self.fifo_write_ptr_lo.raw() as u32)) & !0x1F
    }

    #[inline(always)]
    pub fn fifo_read_ptr(&self) -> u32 {
        (((self.fifo_read_ptr_hi.raw() as u32) << 16) | (self.fifo_read_ptr_lo.raw() as u32)) & !0x1F
    }

    #[inline(always)]
    pub fn fifo_bp(&self) -> u32 {
        (((self.fifo_bp_hi.raw() as u32) << 16) | (self.fifo_bp_lo.raw() as u32)) & !0x1F
    }

    #[inline(always)]
    pub fn set_fifo_rw_distance(&mut self, v: u32) {
        self.fifo_rw_distance_lo = regs::FifoRwDistanceLo::from_raw(v as u16);
        self.fifo_rw_distance_hi = regs::FifoRwDistanceHi::from_raw((v >> 16) as u16);
    }

    #[inline(always)]
    pub fn set_fifo_write_ptr(&mut self, v: u32) {
        self.fifo_write_ptr_lo = regs::FifoWritePtrLo::from_raw(v as u16);
        self.fifo_write_ptr_hi = regs::FifoWritePtrHi::from_raw((v >> 16) as u16);
    }

    #[inline(always)]
    pub fn set_fifo_read_ptr(&mut self, v: u32) {
        self.fifo_read_ptr_lo = regs::FifoReadPtrLo::from_raw(v as u16);
        self.fifo_read_ptr_hi = regs::FifoReadPtrHi::from_raw((v >> 16) as u16);
    }

    pub fn refresh_status(&mut self) {
        let dist = self.fifo_rw_distance();
        let hi = self.fifo_hi_watermark();
        let lo = self.fifo_lo_watermark();
        let read_idle = dist == 0;
        let bp = self.status.bp_interrupt();
        self.status = regs::CpStatus::from_raw(0)
            .with_fifo_overflow(hi != 0 && dist > hi)
            .with_fifo_underflow(lo != 0 && dist < lo)
            .with_read_idle(read_idle)
            .with_cmd_idle(read_idle)
            .with_bp_interrupt(bp);
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
        regs::FifoBpLo,
        regs::FifoBpHi,
    ],
}

#[inline(always)]
pub fn refresh_interrupts<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    if sys.cp.interrupt_active() {
        sys.pi.assert_interrupt(InterruptFlag::Cp);
    } else {
        sys.pi.clear_interrupt(InterruptFlag::Cp);
    }
}

#[inline(always)]
pub fn ack_breakpoint<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    sys.cp.status = sys.cp.status.with_bp_interrupt(false);
    self::refresh_interrupts(sys);
    self::pump_fifo(sys);
}

#[inline(always)]
pub fn gather_pipe_write_u8<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, val: u8) {
    let pos = sys.cp.gather_pos as usize;
    sys.cp.gather_pipe[pos] = val;
    sys.cp.gather_pos += 1;
    if sys.cp.gather_pos >= GP_BURST {
        gather_pipe_bursted(sys);
    }
}

#[inline(always)]
pub fn gather_pipe_write_u16<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, val: u16) {
    let pos = sys.cp.gather_pos as usize;
    sys.cp.gather_pipe[pos..pos + 2].copy_from_slice(&val.to_be_bytes());
    sys.cp.gather_pos += 2;
    if sys.cp.gather_pos >= GP_BURST {
        gather_pipe_bursted(sys);
    }
}

#[inline(always)]
pub fn gather_pipe_write_u32<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, val: u32) {
    let pos = sys.cp.gather_pos as usize;
    sys.cp.gather_pipe[pos..pos + 4].copy_from_slice(&val.to_be_bytes());
    sys.cp.gather_pos += 4;
    if sys.cp.gather_pos >= GP_BURST {
        gather_pipe_bursted(sys);
    }
}

#[cold]
pub fn gather_pipe_bursted<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    let linked = sys.cp.control.gp_link_enable();

    while sys.cp.gather_pos >= GP_BURST {
        if linked {
            let wptr = sys.pi.fifo_wptr;

            let mut burst = [0u8; GP_BURST as usize];
            burst.copy_from_slice(&sys.cp.gather_pipe[..GP_BURST as usize]);

            match sys.mmio.ram_view_mut().slice_mut(wptr as usize, GP_BURST as usize) {
                Some(dst) => dst.copy_from_slice(&burst),
                None => tracing::warn!(
                    wptr = format!("{wptr:#010X}"),
                    "gather_pipe_bursted: PI fifo_wptr unmapped"
                ),
            }

            let advanced = wptr.wrapping_add(GP_BURST);
            let next_wptr = if sys.pi.fifo_end != 0 && advanced >= sys.pi.fifo_end {
                sys.pi.fifo_base
            } else {
                advanced
            };

            sys.pi.fifo_wptr = next_wptr;
            sys.cp.set_fifo_write_ptr(next_wptr);
            sys.cp
                .set_fifo_rw_distance(sys.cp.fifo_rw_distance().saturating_add(GP_BURST));
        }

        let leftover = sys.cp.gather_pos - GP_BURST;
        if leftover > 0 {
            sys.cp
                .gather_pipe
                .copy_within(GP_BURST as usize..(GP_BURST + leftover) as usize, 0);
        }

        sys.cp.gather_pos = leftover;
    }

    if linked {
        sys.cp.refresh_status();
        refresh_interrupts(sys);
    }

    pump_fifo(sys);
}

pub fn pump_fifo<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    let mut consumed = 0u32;

    while !sys.cp.interrupt_active() && sys.cp.control.gp_fifo_read_enable() && sys.cp.fifo_rw_distance() >= GP_BURST {
        let read_ptr = sys.cp.fifo_read_ptr();
        if sys.cp.control.bp_interrupt_enable() && read_ptr == sys.cp.fifo_bp() {
            sys.cp.status = sys.cp.status.with_bp_interrupt(true);
            refresh_interrupts(sys);
            break;
        }

        let mut burst = [0u8; GP_BURST as usize];
        match sys.mmio.ram_view().slice(read_ptr as usize, GP_BURST as usize) {
            Some(src) => burst.copy_from_slice(src),
            None => unreachable!("pump_fifo: FIFO read pointer unmapped"),
        }
        sys.gx.fifo.extend_from_slice(&burst);

        let next_read_ptr = if read_ptr == sys.cp.fifo_end() {
            sys.cp.fifo_base()
        } else {
            read_ptr.wrapping_add(GP_BURST)
        };
        sys.cp.set_fifo_read_ptr(next_read_ptr);
        sys.cp
            .set_fifo_rw_distance(sys.cp.fifo_rw_distance().saturating_sub(GP_BURST));
        consumed += 1;
    }

    if consumed > 0 {
        sys.gx.drain_fifo(&mut sys.mmio, sys.render_sink.as_mut());
        sys.check_gx_pe_interrupts();
        sys.cp.refresh_status();
        refresh_interrupts(sys);
    }
}
