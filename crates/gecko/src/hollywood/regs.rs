use crate::{
    System, SystemId,
    mmio::traits::{MmioAccess, WriteMask},
};

// 0x0D00_0000 PPCMSG
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct PpcMsg {
    #[bits(0..=31)]
    pub value: u32,
}

crate::mmio_reg!(PpcMsg: u32 @ 0x0D00_0000);
crate::mmio_default_access!(PpcMsg => System.hollywood.ipc.ppcmsg);

// 0x0D00_0004 PPCCTRL
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
#[rustfmt::skip]
pub struct PpcCtrl {
    /// X1: PPC writes 1 to launch a command. Self-clearing once Starlet consumes.
    #[bits(0, alias = "x1")] pub execute: bool,

    /// Y2: Starlet sets to confirm transaction complete. W1C from PPC.
    #[bits(1, alias = "y2")] pub arm_post_ack: bool,

    /// Y1: Starlet sets to deliver the main response (with data). W1C from PPC.
    #[bits(2, alias = "y1")] pub arm_response: bool,

    /// X2: PPC writes 1 to acknowledge the Y1 response.
    #[bits(3, alias = "x2")] pub ack_response: bool,

    /// IY1: enable IRQ when Y1 (arm_response) is set.
    #[bits(4, alias = "iy1")] pub irq_arm_response: bool,

    /// IY2: enable IRQ when Y2 (arm_post_ack) is set.
    #[bits(5, alias = "iy2")] pub irq_arm_post_ack: bool,
}
crate::mmio_reg!(PpcCtrl: u32 @ 0x0D00_0004);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for PpcCtrl {
    #[inline(always)]
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.hollywood.ipc.ppcctrl
    }

    #[inline(always)]
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        let incoming = self;
        let current = sys.hollywood.ipc.ppcctrl;

        // these are write 1 to clear
        sys.hollywood.ipc.ppcctrl = incoming
            .with_arm_post_ack(current.arm_post_ack() && !incoming.arm_post_ack())
            .with_arm_response(current.arm_response() && !incoming.arm_response());

        tracing::debug!(
            incoming = format!("{:#010X}", incoming.raw()),
            current = format!("{:#010X}", current.raw()),
            after = format!("{:#010X}", sys.hollywood.ipc.ppcctrl.raw()),
            "PPCCTRL write"
        );

        // dispatch via starlet
        if sys.hollywood.ipc.ppcctrl.execute() {
            let cmd_paddr = sys.hollywood.ipc.ppcmsg.raw();
            sys.hollywood.ipc.ppcctrl = sys.hollywood.ipc.ppcctrl.with_execute(false);
            tracing::debug!(cmd_paddr = format!("{cmd_paddr:#010X}"), "PPC launched IPC command");

            crate::starlet::dispatch_command(sys, cmd_paddr);
        }

        // PPC just ACKed a pending reply
        let ack_happened =
            (current.arm_post_ack() && incoming.arm_post_ack()) || (current.arm_response() && incoming.arm_response());
        if ack_happened {
            tracing::debug!(
                cleared_y2 = current.arm_post_ack() && incoming.arm_post_ack(),
                cleared_y1 = current.arm_response() && incoming.arm_response(),
                "PPC acked"
            );
            crate::hollywood::irq::ack_ipc(sys);
            crate::starlet::schedule_drain(sys);
        }
    }
}

// 0x0D00_0008 ARMMSG
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct ArmMsg {
    #[bits(0..=31)]
    pub value: u32,
}

crate::mmio_reg!(ArmMsg: u32 @ 0x0D00_0008);
crate::mmio_default_access!(ArmMsg => System.hollywood.ipc.armmsg);

// 0x0D00_000C ARMCTRL
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct ArmCtrl {
    #[bits(0..=31)]
    pub value: u32,
}

crate::mmio_reg!(ArmCtrl: u32 @ 0x0D00_000C);
crate::mmio_default_access!(ArmCtrl => System.hollywood.ipc.armctrl);

// 0x0D00_0030 HW_IRQ_CAUSE
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Cause {
    #[bits(30)]
    pub ipc: bool,
}

crate::mmio_reg!(Cause: u32 @ 0x0D00_0030);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for Cause {
    #[inline(always)]
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.hollywood.irq.cause
    }

    #[inline(always)]
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        let cleared = sys.hollywood.irq.cause.raw() & !self.raw();
        sys.hollywood.irq.cause = Cause::from_raw(cleared);
        crate::hollywood::irq::route_to_pi(sys);
    }
}

// 0x0D00_0034 HW_IRQ_MASK
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct Mask {
    #[bits(30)]
    pub ipc: bool,
}

crate::mmio_reg!(Mask: u32 @ 0x0D00_0034);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for Mask {
    #[inline(always)]
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.hollywood.irq.mask
    }

    #[inline(always)]
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.hollywood.irq.mask = self;
        crate::hollywood::irq::route_to_pi(sys);
    }
}

// 0x0D80_00C0 HW_GPIOB_OUT
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioBOut {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioBOut: u32 @ 0x0D80_00C0);
crate::mmio_default_access!(GpioBOut => System.hollywood.gpio.ppc_out);

// 0x0D80_00C4 HW_GPIOB_DIR
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioBDir {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioBDir: u32 @ 0x0D80_00C4);
crate::mmio_default_access!(GpioBDir => System.hollywood.gpio.ppc_dir);

// 0x0D80_00C8 HW_GPIOB_IN
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioBIn {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioBIn: u32 @ 0x0D80_00C8);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for GpioBIn {
    #[inline(always)]
    fn read(_sys: &mut System<SYSTEM>) -> Self {
        Self::from_raw(0)
    }

    #[inline(always)]
    fn write(self, _sys: &mut System<SYSTEM>, _: WriteMask) {}
}

// 0x0D80_00CC HW_GPIOB_INTLVL
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioBIntLvl {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioBIntLvl: u32 @ 0x0D80_00CC);
crate::mmio_default_access!(GpioBIntLvl => System.hollywood.gpio.ppc_intlvl);

// 0x0D80_00D0 HW_GPIOB_INTFLAG
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioBIntFlag {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioBIntFlag: u32 @ 0x0D80_00D0);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for GpioBIntFlag {
    #[inline(always)]
    fn read(_sys: &mut System<SYSTEM>) -> Self {
        Self::from_raw(0)
    }

    #[inline(always)]
    fn write(self, _sys: &mut System<SYSTEM>, _: WriteMask) {}
}

// 0x0D80_00D4 HW_GPIOB_INTMASK
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioBIntMask {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioBIntMask: u32 @ 0x0D80_00D4);
crate::mmio_default_access!(GpioBIntMask => System.hollywood.gpio.ppc_intmask);

// 0x0D80_00D8 HW_GPIOB_STRAPS
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioBStraps {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioBStraps: u32 @ 0x0D80_00D8);
crate::mmio_default_access!(GpioBStraps => System.hollywood.gpio.ppc_straps);

// 0x0D80_00DC HW_GPIOB_OWNER
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioBOwner {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioBOwner: u32 @ 0x0D80_00DC);
crate::mmio_default_access!(GpioBOwner => System.hollywood.gpio.ppc_owner);

// 0x0D00_00C0 HW_GPIO_OUT
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioOut {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioOut: u32 @ 0x0D00_00C0);
crate::mmio_default_access!(GpioOut => System.hollywood.gpio.arm_out);

// 0x0D00_00C4 HW_GPIO_DIR
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioDir {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioDir: u32 @ 0x0D00_00C4);
crate::mmio_default_access!(GpioDir => System.hollywood.gpio.arm_dir);

// 0x0D00_00C8 HW_GPIO_IN (read-only)
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioIn {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioIn: u32 @ 0x0D00_00C8);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for GpioIn {
    #[inline(always)]
    fn read(_sys: &mut System<SYSTEM>) -> Self {
        Self::from_raw(0)
    }

    #[inline(always)]
    fn write(self, _sys: &mut System<SYSTEM>, _: WriteMask) {}
}

// 0x0D00_00CC HW_GPIO_INTLVL
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioIntLvl {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioIntLvl: u32 @ 0x0D00_00CC);
crate::mmio_default_access!(GpioIntLvl => System.hollywood.gpio.arm_intlvl);

// 0x0D00_00D0 HW_GPIO_INTFLAG (W1C against zero)
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioIntFlag {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioIntFlag: u32 @ 0x0D00_00D0);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for GpioIntFlag {
    #[inline(always)]
    fn read(_sys: &mut System<SYSTEM>) -> Self {
        Self::from_raw(0)
    }

    #[inline(always)]
    fn write(self, _sys: &mut System<SYSTEM>, _: WriteMask) {}
}

// 0x0D00_00D4 HW_GPIO_INTMASK
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioIntMask {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioIntMask: u32 @ 0x0D00_00D4);
crate::mmio_default_access!(GpioIntMask => System.hollywood.gpio.arm_intmask);

// 0x0D00_00D8 HW_GPIO_STRAPS
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioStraps {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioStraps: u32 @ 0x0D00_00D8);
crate::mmio_default_access!(GpioStraps => System.hollywood.gpio.arm_straps);

// 0x0D00_00DC HW_GPIO_OWNER
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct GpioOwner {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(GpioOwner: u32 @ 0x0D00_00DC);
crate::mmio_default_access!(GpioOwner => System.hollywood.gpio.arm_owner);

// 0x0D80_0180 HW_COMPAT
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct HwCompat {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(HwCompat: u32 @ 0x0D80_0180);
crate::mmio_default_access!(HwCompat => System.hollywood.compat.compat);

// 0x0D80_01CC HW_PLLAI
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct HwPllAi {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(HwPllAi: u32 @ 0x0D80_01CC);
crate::mmio_default_access!(HwPllAi => System.hollywood.compat.pll_ai);

// 0x0D80_01D0 HW_PLLAIEXT
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct HwPllAiExt {
    #[bits(0..=31)]
    pub value: u32,
}
crate::mmio_reg!(HwPllAiExt: u32 @ 0x0D80_01D0);
crate::mmio_default_access!(HwPllAiExt => System.hollywood.compat.pll_ai_ext);
