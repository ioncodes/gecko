use crate::{
    System, SystemId,
    hollywood::ipc::IpcState,
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
    #[bits(0)] pub execute: bool,
    #[bits(1)] pub ack_reply: bool,
    #[bits(2)] pub ack_relaunch: bool,
    #[bits(3)] pub relaunch: bool,
    #[bits(4)] pub irq_relaunch_enable: bool,
    #[bits(5)] pub irq_reply_enable: bool,
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
            .with_ack_reply(current.ack_reply() && !incoming.ack_reply())
            .with_ack_relaunch(current.ack_relaunch() && !incoming.ack_relaunch());

        // dispatch via starlet
        if sys.hollywood.ipc.ppcctrl.execute() {
            let cmd_paddr = sys.hollywood.ipc.ppcmsg.raw();
            sys.hollywood.ipc.ppcctrl = sys.hollywood.ipc.ppcctrl.with_execute(false);
            sys.hollywood.ipc.state = IpcState::Processing;
            tracing::info!(cmd_paddr = format!("{cmd_paddr:#010X}"), "PPC launched IPC command");

            // TODO: crate::starlet::dispatch_command(sys, cmd_paddr);
        }

        // PPC has ACKed the response, reset
        if !sys.hollywood.ipc.ppcctrl.ack_reply()
            && !sys.hollywood.ipc.ppcctrl.ack_relaunch()
            && sys.hollywood.ipc.state == IpcState::Done
        {
            // TODO: check state here?
            sys.hollywood.ipc.state = IpcState::Idle;
            crate::hollywood::irq::ack_ipc(sys);
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
