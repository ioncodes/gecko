use crate::mmio::traits::{MmioAccess, WriteMask};
use crate::system::{System, SystemId};

// 0xCC003000  4  r    INTSR (Interrupt Cause)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct InterruptCause {
    #[bits(0, alias = "error")]
    pub gp_runtime_error: bool,

    #[bits(1, alias = "rsw")]
    pub reset_switch: bool,

    #[bits(2, alias = "di")]
    pub dvd: bool,

    #[bits(3, alias = "si")]
    pub serial: bool,

    #[bits(4)]
    pub exi: bool,

    #[bits(5, alias = "ai")]
    pub streaming: bool,

    #[bits(6)]
    pub dsp: bool,

    #[bits(7, alias = "mem")]
    pub memory: bool,

    #[bits(8, alias = "vi")]
    pub video: bool,

    #[bits(9, alias = "pe_token")]
    pub token_assertion_in_cmd_list: bool,

    #[bits(10, alias = "pe_finish")]
    pub frame_is_ready: bool,

    #[bits(11, alias = "cp")]
    pub command_fifo: bool,

    #[bits(12)]
    pub debug: bool,

    #[bits(13, alias = "hsp")]
    pub highspeed_port: bool,

    #[bits(14, alias = "iop")]
    pub hollywood: bool,

    #[bits(16)]
    pub rswst: bool,
}
crate::mmio_reg!(InterruptCause: u32 @ 0xCC003000);

impl Default for InterruptCause {
    fn default() -> Self {
        // RSWST needs to be set.
        Self::from_raw(0).with_rswst(true)
    }
}

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for InterruptCause {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.pi.intsr
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        // yagcd seems to be wrong, we should not clear everything on read,
        // but just do the usual w1c instead.
        const RSWST_MASK: u32 = 1 << 16;
        let cleared = sys.pi.intsr.raw() & !self.raw();
        sys.pi.intsr = InterruptCause::from_raw(cleared | (sys.pi.intsr.raw() & RSWST_MASK));
    }
}

// 0xCC003004  4  r/w  INTMR (Interrupt Mask)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct InterruptMask {
    #[bits(0, alias = "error")]
    pub gp_runtime_error: bool,

    #[bits(1, alias = "rsw")]
    pub reset_switch: bool,

    #[bits(2, alias = "di")]
    pub dvd: bool,

    #[bits(3, alias = "si")]
    pub serial: bool,

    #[bits(4)]
    pub exi: bool,

    #[bits(5, alias = "ai")]
    pub streaming: bool,

    #[bits(6)]
    pub dsp: bool,

    #[bits(7, alias = "mem")]
    pub memory: bool,

    #[bits(8, alias = "vi")]
    pub video: bool,

    #[bits(9, alias = "pe_token")]
    pub token_assertion_in_cmd_list: bool,

    #[bits(10, alias = "pe_finish")]
    pub frame_is_ready: bool,

    #[bits(11, alias = "cp")]
    pub command_fifo: bool,

    #[bits(12)]
    pub debug: bool,

    #[bits(13, alias = "hsp")]
    pub highspeed_port: bool,

    #[bits(14, alias = "iop")]
    pub hollywood: bool,
}
crate::mmio_reg!(InterruptMask: u32 @ 0xCC003004);
crate::mmio_default_access!(InterruptMask => System.pi.intmr);

// 0xCC00300C  4  r/w  PI_FIFO_BASE - CPU FIFO Base Address

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoBase {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(FifoBase: u32 @ 0xCC00300C);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for FifoBase {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        FifoBase::from_raw(sys.pi.fifo_base)
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.pi.fifo_base = self.raw();
    }
}

// 0xCC003010  4  r/w  PI_FIFO_END - CPU FIFO End Address

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoEnd {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(FifoEnd: u32 @ 0xCC003010);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for FifoEnd {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        FifoEnd::from_raw(sys.pi.fifo_end)
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.pi.fifo_end = self.raw();
    }
}

// 0xCC003014  4  r/w  PI_FIFO_WPTR - CPU FIFO Write Pointer

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FifoWritePtr {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(FifoWritePtr: u32 @ 0xCC003014);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for FifoWritePtr {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        FifoWritePtr::from_raw(sys.pi.fifo_wptr)
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.pi.fifo_wptr = self.raw() & 0x1FFF_FFE0;
    }
}

// 0xCC003024  4  r/w  Console Reset Code

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct ResetCode {}
crate::mmio_reg!(ResetCode: u32 @ 0xCC003024);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for ResetCode {
    fn read(_gc: &mut System<SYSTEM>) -> Self {
        Self::from_raw(0)
    }
    fn write(self, _gc: &mut System<SYSTEM>, _: WriteMask) {
        tracing::warn!("TODO: reset DVD");
    }
}

// 0xCC00302C  4  r    PI_FLIPPER_REV (Flipper Chip Revision)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FlipperRev {
    #[bits(28..=31)]
    pub revision: u8,
}
crate::mmio_reg!(FlipperRev: u32 @ 0xCC00302C);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for FlipperRev {
    fn read(_gc: &mut System<SYSTEM>) -> Self {
        // FLIPPER_REV_C from Dolphin.
        Self::from_raw(0x2465_00B1)
    }
    fn write(self, _gc: &mut System<SYSTEM>, _: WriteMask) {
        tracing::warn!("writing to FlipperRev???");
    }
}
