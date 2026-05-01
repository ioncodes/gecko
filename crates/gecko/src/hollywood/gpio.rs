use crate::hollywood::regs::{
    GpioBDir, GpioBIn, GpioBIntFlag, GpioBIntLvl, GpioBIntMask, GpioBOut, GpioBOwner, GpioBStraps,
};

pub struct Gpio {
    pub out: GpioBOut,
    pub dir: GpioBDir,
    pub intlvl: GpioBIntLvl,
    pub intmask: GpioBIntMask,
    pub straps: GpioBStraps,
    pub owner: GpioBOwner,
}

impl Gpio {
    pub fn new() -> Self {
        Gpio {
            out: GpioBOut::from_raw(0),
            dir: GpioBDir::from_raw(0),
            intlvl: GpioBIntLvl::from_raw(0),
            intmask: GpioBIntMask::from_raw(0),
            straps: GpioBStraps::from_raw(0),
            owner: GpioBOwner::from_raw(0),
        }
    }
}

crate::mmio_device_dispatch! {
    read = gpio_read,
    write = gpio_write,
    registers = [
        GpioBOut,
        GpioBDir,
        GpioBIn,
        GpioBIntLvl,
        GpioBIntFlag,
        GpioBIntMask,
        GpioBStraps,
        GpioBOwner,
    ],
}
