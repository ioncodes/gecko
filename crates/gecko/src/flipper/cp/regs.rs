use crate::flipper::cp;
use crate::mmio::traits::{MmioAccess, WriteMask};
use crate::system::{System, SystemId};

// 0xCC000000  2  R/W   CP_STATUS (CP Status Register)

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct CpStatus {
    #[bits(0)]
    pub fifo_overflow: bool,

    #[bits(1)]
    pub fifo_underflow: bool,

    #[bits(2)]
    pub read_idle: bool,

    #[bits(3)]
    pub cmd_idle: bool,

    #[bits(4)]
    pub bp_interrupt: bool,
}
crate::mmio_reg!(CpStatus: u16 @ 0xCC000000);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for CpStatus {
    fn read(gc: &mut System<SYSTEM>) -> Self {
        gc.cp.status
    }

    fn write(self, gc: &mut System<SYSTEM>, _: WriteMask) {
        gc.cp.status = self;
        cp::refresh_interrupts(gc);
    }
}

// 0xCC000002  2  R/W  CP_CTRL (CP Control Register)

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct CpControl {
    #[bits(0)]
    pub gp_fifo_read_enable: bool,

    #[bits(1)]
    pub cp_interrupt_enable: bool,

    #[bits(2)]
    pub fifo_overflow_interrupt_enable: bool,

    #[bits(3)]
    pub fifo_underflow_interrupt_enable: bool,

    #[bits(4)]
    pub gp_link_enable: bool,

    #[bits(5)]
    pub bp_interrupt_enable: bool,
}
crate::mmio_reg!(CpControl: u16 @ 0xCC000002);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for CpControl {
    fn read(gc: &mut System<SYSTEM>) -> Self {
        gc.cp.control
    }

    fn write(self, gc: &mut System<SYSTEM>, _: WriteMask) {
        gc.cp.control = self;
        cp::refresh_interrupts(gc);
    }
}

// 0xCC000004  2  W   Clear Register

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct CpClear {
    #[bits(0)]
    pub clear_overflow: bool,

    #[bits(1)]
    pub clear_underflow: bool,
}
crate::mmio_reg!(CpClear: u16 @ 0xCC000004);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for CpClear {
    fn read(_gc: &mut System<SYSTEM>) -> Self {
        tracing::warn!("attempted to read from write only CpClear register");
        Self::from_raw(0)
    }

    fn write(self, gc: &mut System<SYSTEM>, _: WriteMask) {
        if self.clear_overflow() {
            gc.cp.status = gc.cp.status.with_fifo_overflow(false);
        }
        if self.clear_underflow() {
            gc.cp.status = gc.cp.status.with_fifo_underflow(false);
        }
        cp::refresh_interrupts(gc);
    }
}

// 0xCC000020..=0xCC00003B: FIFO pointer registers. All plain 16 bit storage.

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoBaseLo {}
crate::mmio_reg!(FifoBaseLo: u16 @ 0xCC000020);
crate::mmio_default_access!(FifoBaseLo => System.cp.fifo_base_lo);

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoBaseHi {}
crate::mmio_reg!(FifoBaseHi: u16 @ 0xCC000022);
crate::mmio_default_access!(FifoBaseHi => System.cp.fifo_base_hi);

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoEndLo {}
crate::mmio_reg!(FifoEndLo: u16 @ 0xCC000024);
crate::mmio_default_access!(FifoEndLo => System.cp.fifo_end_lo);

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoEndHi {}
crate::mmio_reg!(FifoEndHi: u16 @ 0xCC000026);
crate::mmio_default_access!(FifoEndHi => System.cp.fifo_end_hi);

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoHiWatermarkLo {}
crate::mmio_reg!(FifoHiWatermarkLo: u16 @ 0xCC000028);
crate::mmio_default_access!(FifoHiWatermarkLo => System.cp.fifo_hi_watermark_lo);

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoHiWatermarkHi {}
crate::mmio_reg!(FifoHiWatermarkHi: u16 @ 0xCC00002A);
crate::mmio_default_access!(FifoHiWatermarkHi => System.cp.fifo_hi_watermark_hi);

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoLoWatermarkLo {}
crate::mmio_reg!(FifoLoWatermarkLo: u16 @ 0xCC00002C);
crate::mmio_default_access!(FifoLoWatermarkLo => System.cp.fifo_lo_watermark_lo);

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoLoWatermarkHi {}
crate::mmio_reg!(FifoLoWatermarkHi: u16 @ 0xCC00002E);
crate::mmio_default_access!(FifoLoWatermarkHi => System.cp.fifo_lo_watermark_hi);

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoRwDistanceLo {}
crate::mmio_reg!(FifoRwDistanceLo: u16 @ 0xCC000030);
crate::mmio_default_access!(FifoRwDistanceLo => System.cp.fifo_rw_distance_lo);

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoRwDistanceHi {}
crate::mmio_reg!(FifoRwDistanceHi: u16 @ 0xCC000032);
crate::mmio_default_access!(FifoRwDistanceHi => System.cp.fifo_rw_distance_hi);

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoWritePtrLo {}
crate::mmio_reg!(FifoWritePtrLo: u16 @ 0xCC000034);
crate::mmio_default_access!(FifoWritePtrLo => System.cp.fifo_write_ptr_lo);

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoWritePtrHi {}
crate::mmio_reg!(FifoWritePtrHi: u16 @ 0xCC000036);
crate::mmio_default_access!(FifoWritePtrHi => System.cp.fifo_write_ptr_hi);

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoReadPtrLo {}
crate::mmio_reg!(FifoReadPtrLo: u16 @ 0xCC000038);
crate::mmio_default_access!(FifoReadPtrLo => System.cp.fifo_read_ptr_lo);

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoReadPtrHi {}
crate::mmio_reg!(FifoReadPtrHi: u16 @ 0xCC00003A);
crate::mmio_default_access!(FifoReadPtrHi => System.cp.fifo_read_ptr_hi);
