use crate::dvd;
use crate::mmio::traits::{MmioAccess, WriteMask};
use crate::system::{System, SystemId};
use chapa::BitEnum;

// 0xCC006000  4  R/W  DISR - DI Status Register

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DiStatusRegister {
    #[bits(0)]
    pub brk: bool,

    #[bits(1)]
    pub device_error_mask: bool,

    #[bits(2)]
    pub device_error: bool,

    #[bits(3)]
    pub transfer_complete_mask: bool,

    #[bits(4)]
    pub transfer_complete: bool,

    #[bits(5)]
    pub break_complete_mask: bool,

    #[bits(6)]
    pub break_complete: bool,
}
crate::mmio_reg!(DiStatusRegister: u32 @ 0xCC006000);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DiStatusRegister {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.di.status
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        let mut sr = sys.di.status;

        if self.break_complete() {
            sr = sr.with_break_complete(false);
        }
        if self.device_error() {
            sr = sr.with_device_error(false);
        }
        if self.transfer_complete() {
            sr = sr.with_transfer_complete(false);
        }

        // TODO: 0 has no effect?
        if self.brk() {
            sr = sr.with_brk(true);
        }

        sr = sr
            .with_break_complete_mask(self.break_complete_mask())
            .with_device_error_mask(self.device_error_mask())
            .with_transfer_complete_mask(self.transfer_complete_mask());

        sys.di.status = sr;
        dvd::refresh_interrupts(sys);
    }
}

// 0xCC006004  4  R/W  DICVR - DI Cover Register

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DiCoverRegister {
    #[bits(0)]
    pub cover_status: bool,

    #[bits(1)]
    pub cover_interrupt_mask: bool,

    #[bits(2)]
    pub cover_interrupt: bool,
}
crate::mmio_reg!(DiCoverRegister: u32 @ 0xCC006004);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DiCoverRegister {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.di.cover
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        let mut cvr = sys.di.cover;

        if self.cover_interrupt() {
            cvr = cvr.with_cover_interrupt(false);
        }

        cvr = cvr.with_cover_interrupt_mask(self.cover_interrupt_mask());

        sys.di.cover = cvr;
        dvd::refresh_interrupts(sys);
    }
}

// 0xCC006008  4  W  DICMDBUF0 - DI Command Buffer 0
// 0xCC00600C  4  W  DICMDBUF1 - DI Command Buffer 1
// 0xCC006010  4  W  DICMDBUF2 - DI Command Buffer 2

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DiCommandBuf0 {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(DiCommandBuf0: u32 @ 0xCC006008);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DiCommandBuf0 {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        DiCommandBuf0::from_raw(sys.di.cmdbuf0)
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.di.cmdbuf0 = self.raw();
        let val = self.raw();
        tracing::debug!(
            cmd = format!("{:02X}", val >> 24),
            sub1 = format!("{:02X}", (val >> 16) & 0xFF),
            sub2 = format!("{:04X}", val & 0xFFFF),
            "DICMDBUF0 write"
        );
    }
}

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DiCommandBuf1 {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(DiCommandBuf1: u32 @ 0xCC00600C);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DiCommandBuf1 {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        DiCommandBuf1::from_raw(sys.di.cmdbuf1)
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.di.cmdbuf1 = self.raw();
        tracing::debug!(val = format!("{:08X}", self.raw()), "DICMDBUF1 write");
    }
}

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DiCommandBuf2 {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(DiCommandBuf2: u32 @ 0xCC006010);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DiCommandBuf2 {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        DiCommandBuf2::from_raw(sys.di.cmdbuf2)
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.di.cmdbuf2 = self.raw();
        tracing::debug!(val = format!("{:08X}", self.raw()), "DICMDBUF2 write");
    }
}

// 0xCC006014  4  R/W  DIMAR - DMA Memory Address Register

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DiDmaAddressRegister {
    #[bits(0..=25)]
    pub address: u32,
}
crate::mmio_reg!(DiDmaAddressRegister: u32 @ 0xCC006014);
crate::mmio_default_access!(DiDmaAddressRegister => System.di.dma_address);

// 0xCC006018  4  R/W  DILENGTH - DI DMA Transfer Length Register

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DiDmaLengthRegister {
    #[bits(0..=25)]
    pub length: u32,
}
crate::mmio_reg!(DiDmaLengthRegister: u32 @ 0xCC006018);
crate::mmio_default_access!(DiDmaLengthRegister => System.di.dma_length);

// 0xCC00601C  4  R/W  DICR - DI Control Register

#[derive(BitEnum, Debug, PartialEq, Eq)]
pub enum TransferMode {
    Immediate = 0,
    Dma = 1,
}

#[derive(BitEnum, Debug, PartialEq, Eq)]
pub enum AccessMode {
    Read = 0,
    Write = 1,
}

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DiControlRegister {
    #[bits(0)]
    pub tstart: bool,

    #[bits(1)]
    pub dma: TransferMode,

    #[bits(2)]
    pub access_mode: AccessMode,
}
crate::mmio_reg!(DiControlRegister: u32 @ 0xCC00601C);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DiControlRegister {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.di.control
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.di.control = self;

        // tstart latches a transfer: resolve the command and run it now.
        // The scheduled "transfer complete" callback fires after the canonical
        // delay so the CPU side-effect ordering matches real hardware.
        if self.tstart() {
            dvd::start_transfer(sys);
        }

        dvd::refresh_interrupts(sys);
    }
}

// 0xCC006020  4  W  DIIMMBUF - DI Immediate Data Buffer

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DiImmBuf {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(DiImmBuf: u32 @ 0xCC006020);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DiImmBuf {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        DiImmBuf::from_raw(sys.di.immbuf)
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.di.immbuf = self.raw();
    }
}

// 0xCC006024  4  R  DICFG - DI Configuration Register (read-only)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DiConfigurationRegister {
    #[bits(0, readonly)]
    pub config: bool,
}
crate::mmio_reg!(DiConfigurationRegister: u32 @ 0xCC006024);

impl Default for DiConfigurationRegister {
    fn default() -> Self {
        DiConfigurationRegister::from_raw(0b1)
    }
}

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DiConfigurationRegister {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.di.config
    }

    fn write(self, _gc: &mut System<SYSTEM>, _: WriteMask) {
        // Read-only
    }
}
