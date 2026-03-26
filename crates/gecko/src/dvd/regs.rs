use chapa::BitEnum;

use super::DvdInterface;
use crate::mmio::traits::MmioAccess;

// 0xCC006000  4  R/W  DISR - DI Status Register

crate::mmio_register! {
    DiStatusRegister: u32 @ 0xCC006000 {
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
}

impl MmioAccess<DvdInterface> for DiStatusRegister {
    fn read(di: &DvdInterface) -> Self {
        di.status
    }

    fn write(self, di: &mut DvdInterface) {
        let mut sr = di.status;

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

        di.status = sr;
    }
}

// 0xCC006004  4  R/W  DICVR - DI Cover Register

crate::mmio_register! {
    DiCoverRegister: u32 @ 0xCC006004 {
        #[bits(0)]
        pub cover_status: bool,

        #[bits(1)]
        pub cover_interrupt_mask: bool,

        #[bits(2)]
        pub cover_interrupt: bool,
    }
}

impl MmioAccess<DvdInterface> for DiCoverRegister {
    fn read(di: &DvdInterface) -> Self {
        di.cover
    }

    fn write(self, di: &mut DvdInterface) {
        let mut cvr = di.cover;

        if self.cover_interrupt() {
            cvr = cvr.with_cover_interrupt(false);
        }

        cvr = cvr.with_cover_interrupt_mask(self.cover_interrupt_mask());

        di.cover = cvr;
    }
}

// 0xCC006014  4  R/W  DIMAR - DMA Memory Address Register

crate::mmio_register! {
    DiDmaAddressRegister: u32 @ 0xCC006014 {
        #[bits(0..=25)]
        pub address: u32,
    }
}

impl MmioAccess<DvdInterface> for DiDmaAddressRegister {
    fn read(di: &DvdInterface) -> Self {
        di.dma_address
    }

    fn write(self, di: &mut DvdInterface) {
        di.dma_address = self;
    }
}

// 0xCC006018  4  R/W  DILENGTH - DI DMA Transfer Length Register

crate::mmio_register! {
    DiDmaLengthRegister: u32 @ 0xCC006018 {
        #[bits(0..=25)]
        pub length: u32,
    }
}

impl MmioAccess<DvdInterface> for DiDmaLengthRegister {
    fn read(di: &DvdInterface) -> Self {
        di.dma_length
    }

    fn write(self, di: &mut DvdInterface) {
        di.dma_length = self;
    }
}

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

crate::mmio_register! {
    DiControlRegister: u32 @ 0xCC00601C {
        #[bits(0)]
        pub tstart: bool,

        #[bits(1)]
        pub dma: TransferMode,

        #[bits(2)]
        pub access_mode: AccessMode,
    }
}

impl MmioAccess<DvdInterface> for DiControlRegister {
    fn read(di: &DvdInterface) -> Self {
        di.control
    }

    fn write(self, di: &mut DvdInterface) {
        di.control = self;

        if self.tstart() {
            di.transfer_started = true;
        }
    }
}
