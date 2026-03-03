use super::Vi;
use crate::mmio::{
    Mmio,
    traits::{MmioAccess, MmioRegister},
};
use chapa::BitEnum;

#[derive(Debug, BitEnum)]
pub enum VideoFormat {
    Ntsc = 0,
    Pal = 1,
    Mpal = 2,
    Debug = 3,
}

#[rustfmt::skip]
#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DisplayConfiguration {
    #[bits(0, alias = "enb")] pub enable: bool,
    #[bits(1, alias = "rst")] pub reset: bool,
    #[bits(2, alias = "nin")] pub interlace_selector: bool,
    #[bits(3, alias = "dlr")] pub display_mode_3d: bool,
    #[bits(4..=5, alias = "le0")] pub display_latch0: u8,
    #[bits(6..=7, alias = "le1")] pub display_latch1: u8,
    #[bits(8..=9, alias = "fmt")] pub video_format: VideoFormat,
}

impl MmioAccess<Vi> for DisplayConfiguration {
    fn read(vi: &Vi) -> Self {
        vi.dcr
    }

    fn write(self, vi: &mut Vi) {
        // TODO: Rising-edge on RST clears the register? Just to test for now
        if self.reset() && !vi.dcr.reset() {
            vi.dcr = <DisplayConfiguration as MmioRegister>::from_raw(0);
        } else {
            vi.dcr = self;
        }
    }
}

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct TopFieldBase {
    #[bits(9..=23, alias = "fbb")]
    pub xfb_addr: u32,

    #[bits(24..=27, alias = "xof")]
    pub horizontal_offset: u8,

    #[bits(28)]
    pub page_offset: bool,
    // TODO: 29-31	y	always zero (maybe some write only control register stuff?, setting bit 31 clears bits 31-28 (?))
}

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct BottomFieldBase {
    #[bits(9..=23, alias = "fbb")]
    pub xfb_addr: u32,

    #[bits(28)]
    pub page_offset: bool,
    // TODO:  	y	always zero (maybe some write-only control register stuff?)
}

#[rustfmt::skip]
macro_rules! impl_mmio_access {
    ($reg:ty, $owner:ty, $field:ident) => {
        impl MmioAccess<$owner> for $reg {
            fn read(dev: &$owner) -> Self { dev.$field }
            fn write(self, dev: &mut $owner) { dev.$field = self; }
        }
    };
}

#[rustfmt::skip]
macro_rules! size_to_raw_type {
    (1, $raw:expr) => { $raw as u8 };
    (2, $raw:expr) => { $raw as u16 };
    (4, $raw:expr) => { $raw };
}

#[rustfmt::skip]
macro_rules! impl_mmio_register {
    ($reg:ty, $addr:expr, $size:tt) => {
        impl MmioRegister for $reg {
            const ADDR: u32 = Mmio::virt_to_phys($addr);
            const SIZE: usize = $size;
            fn from_raw(raw: u32) -> Self { size_to_raw_type!($size, raw).into() }
            fn to_raw(self) -> u32 { self.raw() as u32 }
        }
    };
}

impl_mmio_register!(DisplayConfiguration, 0xCC002002, 2);
impl_mmio_register!(TopFieldBase, 0xCC00201C, 4);
impl_mmio_register!(BottomFieldBase, 0xCC002024, 4);

impl_mmio_access!(TopFieldBase, Vi, top_field_base);
impl_mmio_access!(BottomFieldBase, Vi, bottom_field_base);
