use crate::hollywood::regs::{HwCompat, HwPllAi, HwPllAiExt};

pub struct Compat {
    pub compat: HwCompat,
    pub pll_ai: HwPllAi,
    pub pll_ai_ext: HwPllAiExt,
}

impl Compat {
    pub fn new() -> Self {
        Compat {
            compat: HwCompat::from_raw(0),
            pll_ai: HwPllAi::from_raw(0),
            pll_ai_ext: HwPllAiExt::from_raw(0),
        }
    }
}

crate::mmio_device_dispatch! {
    read = compat_read,
    write = compat_write,
    registers = [HwCompat, HwPllAi, HwPllAiExt],
}
