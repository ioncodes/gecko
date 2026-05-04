use crate::flipper::pe;
use crate::mmio::traits::{MmioAccess, WriteMask};
use crate::system::{System, SystemId};

// 0xCC001000	2	R/W	Z Configuration

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct ZConfig {}
crate::mmio_reg!(ZConfig: u16 @ 0xCC001000);
crate::mmio_default_access!(ZConfig => System.pe.zconf);

// 0xCC001002	2	R/W	Alpha Configuration

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AlphaConfig {}
crate::mmio_reg!(AlphaConfig: u16 @ 0xCC001002);
crate::mmio_default_access!(AlphaConfig => System.pe.alphaconf);

// 0xCC001004	2	R/W	Destination Alpha

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DstAlphaConfig {}
crate::mmio_reg!(DstAlphaConfig: u16 @ 0xCC001004);
crate::mmio_default_access!(DstAlphaConfig => System.pe.dst_alphaconf);

// 0xCC001006	2	R/W	Alpha Compare Mode

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AlphaMode {}
crate::mmio_reg!(AlphaMode: u16 @ 0xCC001006);
crate::mmio_default_access!(AlphaMode => System.pe.alphamode);

// 0xCC001008	2	R/W	Alpha Read Mode

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AlphaRead {}
crate::mmio_reg!(AlphaRead: u16 @ 0xCC001008);
crate::mmio_default_access!(AlphaRead => System.pe.alpharead);

// 0xCC00100A	2	R/W	Interrupt Status

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct InterruptStatus {
    #[bits(0)]
    pub pe_token_enable: bool,

    #[bits(1)]
    pub pe_finish_enable: bool,

    #[bits(2)]
    pub pe_token: bool,

    #[bits(3)]
    pub pe_finish: bool,
}
crate::mmio_reg!(InterruptStatus: u16 @ 0xCC00100A);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for InterruptStatus {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        // Status bits (token/finish) always read back as zero.
        sys.pe.sr.with_pe_token(false).with_pe_finish(false)
    }

    fn write(self, sys: &mut System<SYSTEM>, mask: WriteMask) {
        if !mask.byte(1) {
            return;
        }

        let mut sr = sys.pe.sr;
        sr = sr
            .with_pe_token_enable(self.pe_token_enable())
            .with_pe_finish_enable(self.pe_finish_enable());
        if self.pe_token() {
            sr = sr.with_pe_token(false);
        }
        if self.pe_finish() {
            sr = sr.with_pe_finish(false);
        }
        sys.pe.sr = sr;
        pe::refresh_interrupts(sys);
    }
}

// 0xCC00100E	2	R/W	Token

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Token {}
crate::mmio_reg!(Token: u16 @ 0xCC00100E);
crate::mmio_default_access!(Token => System.pe.token);
