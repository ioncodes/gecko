use chapa::BitEnum;

use crate::flipper::exi;
use crate::mmio::traits::{MmioAccess, MmioRegister, WriteMask};
use crate::system::{System, SystemId};

pub trait ChannelStatus {
    fn exi_interrupt(&self) -> bool;
    fn exi_interrupt_mask(&self) -> bool;
    fn tc_interrupt(&self) -> bool;
    fn tc_interrupt_mask(&self) -> bool;
    fn ext_interrupt(&self) -> bool;
    fn ext_interrupt_mask(&self) -> bool;
}

macro_rules! impl_channel_status {
    ($($ty:ty),*) => {
        $(impl ChannelStatus for $ty {
            fn exi_interrupt(&self) -> bool { Self::exi_interrupt(self) }
            fn exi_interrupt_mask(&self) -> bool { Self::exi_interrupt_mask(self) }
            fn tc_interrupt(&self) -> bool { Self::tc_interrupt(self) }
            fn tc_interrupt_mask(&self) -> bool { Self::tc_interrupt_mask(self) }
            fn ext_interrupt(&self) -> bool { Self::ext_interrupt(self) }
            fn ext_interrupt_mask(&self) -> bool { Self::ext_interrupt_mask(self) }
        })*
    };
}

/// Write-1-to-clear helper for EXI CSR registers
/// Bits 1 (EXIINT), 3 (TCINT), 11 (EXTINT) are write-1-to-clear
/// Bit 12 (EXT) is read-only (device presence)
fn write_csr<T: MmioRegister + Copy>(current: &mut T, new: T) {
    let cur_raw = (*current).to_raw();
    let new_raw = new.to_raw();
    const W1C_MASK: u32 = (1 << 1) | (1 << 3) | (1 << 11);
    const RO_MASK: u32 = 1 << 12;
    let w1c_bits = (cur_raw & W1C_MASK) & !(new_raw & W1C_MASK);
    let ro_bits = cur_raw & RO_MASK;
    let rw_bits = new_raw & !(W1C_MASK | RO_MASK);
    *current = T::from_raw(w1c_bits | ro_bits | rw_bits);
}

/// Used for the RW field in EXI Control registers to specify transfer type
#[derive(BitEnum, PartialEq, Eq)]
pub enum TransferType {
    Read = 0b00,
    Write = 0b01,
    ReadAndWrite = 0b10,
    Reserved = 0b11,
}

impl std::fmt::Debug for TransferType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransferType::Read => write!(f, "read"),
            TransferType::Write => write!(f, "write"),
            TransferType::ReadAndWrite => write!(f, "read+write"),
            TransferType::Reserved => write!(f, "reserved"),
        }
    }
}

// --- Channel 0 ---

// 0xCC006800	4	R/W	EXI0CSR - EXI Channel 0 Status Register

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel0Status {
    #[bits(0, alias = "exiintmask")]
    pub exi_interrupt_mask: bool,
    #[bits(1, alias = "exiint")]
    pub exi_interrupt: bool,
    #[bits(2, alias = "tcintmask")]
    pub tc_interrupt_mask: bool,
    #[bits(3, alias = "tcint")]
    pub tc_interrupt: bool,
    #[bits(4..=6, alias = "clk")]
    pub clock: u8,
    #[bits(7..=9, alias = "cs")]
    pub chip_select: u8,
    #[bits(10, alias = "extintmask")]
    pub ext_interrupt_mask: bool,
    #[bits(11, alias = "extint")]
    pub ext_interrupt: bool,
    #[bits(12, alias = "ext")]
    pub device_connected: bool,
    #[bits(13, alias = "romdis")]
    pub rom_descramble_disabled: bool,
}
crate::mmio_reg!(Channel0Status: u32 @ 0xCC006800);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for Channel0Status {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.exi.ch0_csr
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        write_csr(&mut sys.exi.ch0_csr, self);
        exi::on_chip_select_written::<0, SYSTEM>(sys, self.chip_select());
        exi::refresh_interrupts(sys);
    }
}

// 0xCC006804	4	R/W	EXI0MAR - EXI Channel 0 DMA Start Address

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel0DmaAddress {
    #[bits(5..=25, alias = "addr")]
    pub address: u32,
}
crate::mmio_reg!(Channel0DmaAddress: u32 @ 0xCC006804);
crate::mmio_default_access!(Channel0DmaAddress => System.exi.ch0_mar);

// 0xCC006808	4	R/W	EXI0LENGTH - EXI Channel 0 DMA Transfer Length

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel0DmaLength {
    #[bits(5..=25, alias = "len")]
    pub length: u32,
}
crate::mmio_reg!(Channel0DmaLength: u32 @ 0xCC006808);
crate::mmio_default_access!(Channel0DmaLength => System.exi.ch0_length);

// 0xCC00680C	4	R/W	EXI0CR - EXI Channel 0 Control Register

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel0Control {
    #[bits(0, alias = "tstart")]
    pub transfer_start: bool,
    #[bits(1, alias = "dma")]
    pub dma_mode: bool,
    #[bits(2..=3, alias = "rw")]
    pub transfer_type: TransferType,
    #[bits(4..=5, alias = "tlen")]
    pub transfer_length: u8,
}
crate::mmio_reg!(Channel0Control: u32 @ 0xCC00680C);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for Channel0Control {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.exi.ch0_cr
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        let was_started = sys.exi.ch0_cr.transfer_start();
        sys.exi.ch0_cr = self;
        if self.transfer_start() && !was_started {
            if self.dma_mode() {
                exi::run_dma::<0, SYSTEM>(sys);
            } else {
                sys.exi.start_immediate_transfer::<0>();
            }
        }
        exi::refresh_interrupts(sys);
    }
}

// 0xCC006810	4	R/W	EXI0DATA - EXI Channel 0 Immediate Data

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel0Data {}
crate::mmio_reg!(Channel0Data: u32 @ 0xCC006810);
crate::mmio_default_access!(Channel0Data => System.exi.ch0_data);

// --- Channel 1 ---

// 0xCC006814	4	R/W	EXI1CSR - EXI Channel 1 Status Register

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel1Status {
    #[bits(0, alias = "exiintmask")]
    pub exi_interrupt_mask: bool,
    #[bits(1, alias = "exiint")]
    pub exi_interrupt: bool,
    #[bits(2, alias = "tcintmask")]
    pub tc_interrupt_mask: bool,
    #[bits(3, alias = "tcint")]
    pub tc_interrupt: bool,
    #[bits(4..=6, alias = "clk")]
    pub clock: u8,
    #[bits(7..=9, alias = "cs")]
    pub chip_select: u8,
    #[bits(10, alias = "extintmask")]
    pub ext_interrupt_mask: bool,
    #[bits(11, alias = "extint")]
    pub ext_interrupt: bool,
    #[bits(12, alias = "ext")]
    pub device_connected: bool,
}
crate::mmio_reg!(Channel1Status: u32 @ 0xCC006814);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for Channel1Status {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.exi.ch1_csr
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        write_csr(&mut sys.exi.ch1_csr, self);
        exi::on_chip_select_written::<1, SYSTEM>(sys, self.chip_select());
        exi::refresh_interrupts(sys);
    }
}

// 0xCC006818	4	R/W	EXI1MAR - EXI Channel 1 DMA Start Address

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel1DmaAddress {
    #[bits(5..=25, alias = "addr")]
    pub address: u32,
}
crate::mmio_reg!(Channel1DmaAddress: u32 @ 0xCC006818);
crate::mmio_default_access!(Channel1DmaAddress => System.exi.ch1_mar);

// 0xCC00681C	4	R/W	EXI1LENGTH - EXI Channel 1 DMA Transfer Length

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel1DmaLength {
    #[bits(5..=25, alias = "len")]
    pub length: u32,
}
crate::mmio_reg!(Channel1DmaLength: u32 @ 0xCC00681C);
crate::mmio_default_access!(Channel1DmaLength => System.exi.ch1_length);

// 0xCC006820	4	R/W	EXI1CR - EXI Channel 1 Control Register

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel1Control {
    #[bits(0, alias = "tstart")]
    pub transfer_start: bool,
    #[bits(1, alias = "dma")]
    pub dma_mode: bool,
    #[bits(2..=3, alias = "rw")]
    pub transfer_type: TransferType,
    #[bits(4..=5, alias = "tlen")]
    pub transfer_length: u8,
}
crate::mmio_reg!(Channel1Control: u32 @ 0xCC006820);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for Channel1Control {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.exi.ch1_cr
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        let was_started = sys.exi.ch1_cr.transfer_start();
        sys.exi.ch1_cr = self;
        if self.transfer_start() && !was_started {
            if self.dma_mode() {
                exi::run_dma::<1, SYSTEM>(sys);
            } else {
                sys.exi.start_immediate_transfer::<1>();
            }
        }
        exi::refresh_interrupts(sys);
    }
}

// 0xCC006824	4	R/W	EXI1DATA - EXI Channel 1 Immediate Data

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel1Data {}
crate::mmio_reg!(Channel1Data: u32 @ 0xCC006824);
crate::mmio_default_access!(Channel1Data => System.exi.ch1_data);

// --- Channel 2 ---

// 0xCC006828	4	R/W	EXI2CSR - EXI Channel 2 Status Register

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel2Status {
    #[bits(0, alias = "exiintmask")]
    pub exi_interrupt_mask: bool,
    #[bits(1, alias = "exiint")]
    pub exi_interrupt: bool,
    #[bits(2, alias = "tcintmask")]
    pub tc_interrupt_mask: bool,
    #[bits(3, alias = "tcint")]
    pub tc_interrupt: bool,
    #[bits(4..=6, alias = "clk")]
    pub clock: u8,
    #[bits(7..=9, alias = "cs")]
    pub chip_select: u8,
    #[bits(10, alias = "extintmask")]
    pub ext_interrupt_mask: bool,
    #[bits(11, alias = "extint")]
    pub ext_interrupt: bool,
    #[bits(12, alias = "ext")]
    pub device_connected: bool,
}
crate::mmio_reg!(Channel2Status: u32 @ 0xCC006828);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for Channel2Status {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.exi.ch2_csr
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        write_csr(&mut sys.exi.ch2_csr, self);
        exi::on_chip_select_written::<2, SYSTEM>(sys, self.chip_select());
        exi::refresh_interrupts(sys);
    }
}

impl_channel_status!(Channel0Status, Channel1Status, Channel2Status);

// 0xCC00682C	4	R/W	EXI2MAR - EXI Channel 2 DMA Start Address

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel2DmaAddress {
    #[bits(5..=25, alias = "addr")]
    pub address: u32,
}
crate::mmio_reg!(Channel2DmaAddress: u32 @ 0xCC00682C);
crate::mmio_default_access!(Channel2DmaAddress => System.exi.ch2_mar);

// 0xCC006830	4	R/W	EXI2LENGTH - EXI Channel 2 DMA Transfer Length

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel2DmaLength {
    #[bits(5..=25, alias = "len")]
    pub length: u32,
}
crate::mmio_reg!(Channel2DmaLength: u32 @ 0xCC006830);
crate::mmio_default_access!(Channel2DmaLength => System.exi.ch2_length);

// 0xCC006834	4	R/W	EXI2CR - EXI Channel 2 Control Register

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel2Control {
    #[bits(0, alias = "tstart")]
    pub transfer_start: bool,
    #[bits(1, alias = "dma")]
    pub dma_mode: bool,
    #[bits(2..=3, alias = "rw")]
    pub transfer_type: TransferType,
    #[bits(4..=5, alias = "tlen")]
    pub transfer_length: u8,
}
crate::mmio_reg!(Channel2Control: u32 @ 0xCC006834);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for Channel2Control {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.exi.ch2_cr
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        let was_started = sys.exi.ch2_cr.transfer_start();
        sys.exi.ch2_cr = self;
        if self.transfer_start() && !was_started {
            if self.dma_mode() {
                exi::run_dma::<2, SYSTEM>(sys);
            } else {
                sys.exi.start_immediate_transfer::<2>();
            }
        }
        exi::refresh_interrupts(sys);
    }
}

// 0xCC006838	4	R/W	EXI2DATA - EXI Channel 2 Immediate Data

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Channel2Data {}
crate::mmio_reg!(Channel2Data: u32 @ 0xCC006838);
crate::mmio_default_access!(Channel2Data => System.exi.ch2_data);
