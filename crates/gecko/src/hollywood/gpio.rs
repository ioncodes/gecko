use crate::hollywood::regs::{
    GpioBDir, GpioBIn, GpioBIntFlag, GpioBIntLvl, GpioBIntMask, GpioBOut, GpioBOwner, GpioBStraps, GpioDir, GpioIn,
    GpioIntFlag, GpioIntLvl, GpioIntMask, GpioOut, GpioOwner, GpioStraps,
};

pub struct Gpio {
    pub ppc_out: GpioBOut,
    pub ppc_dir: GpioBDir,
    pub ppc_intlvl: GpioBIntLvl,
    pub ppc_intmask: GpioBIntMask,
    pub ppc_straps: GpioBStraps,
    pub ppc_owner: GpioBOwner,

    pub arm_out: GpioOut,
    pub arm_dir: GpioDir,
    pub arm_intlvl: GpioIntLvl,
    pub arm_intmask: GpioIntMask,
    pub arm_straps: GpioStraps,
    pub arm_owner: GpioOwner,
}

impl Gpio {
    pub fn new() -> Self {
        Gpio {
            ppc_out: GpioBOut::from_raw(0),
            ppc_dir: GpioBDir::from_raw(0),
            ppc_intlvl: GpioBIntLvl::from_raw(0),
            ppc_intmask: GpioBIntMask::from_raw(0),
            ppc_straps: GpioBStraps::from_raw(0),
            ppc_owner: GpioBOwner::from_raw(0),
            arm_out: GpioOut::from_raw(0),
            arm_dir: GpioDir::from_raw(0),
            arm_intlvl: GpioIntLvl::from_raw(0),
            arm_intmask: GpioIntMask::from_raw(0),
            arm_straps: GpioStraps::from_raw(0),
            arm_owner: GpioOwner::from_raw(0),
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
        GpioOut,
        GpioDir,
        GpioIn,
        GpioIntLvl,
        GpioIntFlag,
        GpioIntMask,
        GpioStraps,
        GpioOwner,
    ],
}
