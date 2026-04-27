// 0xCC00401C  2  R/W  Memory Interface Interrupt Mask

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct MiInterruptMask {
    #[bits(0)]
    pub mem0_enable: bool,

    #[bits(1)]
    pub mem1_enable: bool,

    #[bits(2)]
    pub mem2_enable: bool,

    #[bits(3)]
    pub mem3_enable: bool,

    #[bits(4)]
    pub master_enable: bool,
}
crate::mmio_reg!(MiInterruptMask: u16 @ 0xCC00401C);
crate::mmio_default_access!(MiInterruptMask => System.mi.interrupt_mask);
