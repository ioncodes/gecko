pub mod di;
pub mod fs;
pub mod stm;

use crate::hollywood::regs::{ArmCtrl, ArmMsg, PpcCtrl, PpcMsg};
use crate::mmio::Mmio;
use crate::scheduler::Scheduler;
use crate::system::{System, SystemId, WII};

pub const IPC_EINVAL: i32 = -4;
pub const IPC_ENOENT: i32 = -6;

pub struct DeviceContext<'a> {
    pub mmio: &'a mut Mmio<{ WII }>,
    pub scheduler: &'a mut Scheduler<{ WII }>,
}

pub trait IosDevice: Send {
    fn open(&mut self, _ctx: &mut DeviceContext<'_>, _mode: u32) -> i32 {
        0
    }

    fn close(&mut self, _ctx: &mut DeviceContext<'_>) -> i32 {
        0
    }

    fn read(&mut self, _ctx: &mut DeviceContext<'_>, buf: u32, len: u32) -> i32 {
        tracing::warn!(buf = format!("{buf:#010X}"), len, "IOS_Read: unimplemented");
        IPC_EINVAL
    }

    fn write(&mut self, _ctx: &mut DeviceContext<'_>, buf: u32, len: u32) -> i32 {
        tracing::warn!(buf = format!("{buf:#010X}"), len, "IOS_Write: unimplemented");
        IPC_EINVAL
    }

    fn seek(&mut self, _ctx: &mut DeviceContext<'_>, where_: i32, whence: i32) -> i32 {
        tracing::warn!(where_ = where_, whence = whence, "IOS_Seek: unimplemented");
        0
    }

    fn ioctl(
        &mut self,
        _ctx: &mut DeviceContext<'_>,
        cmd: u32,
        in_buf: u32,
        in_len: u32,
        out_buf: u32,
        out_len: u32,
    ) -> i32 {
        tracing::warn!(
            cmd = format!("{cmd:#010X}"),
            in_buf = format!("{in_buf:#010X}"),
            in_len = in_len,
            out_buf = format!("{out_buf:#010X}"),
            out_len = out_len,
            "IOS_Ioctl: unimplemented"
        );
        IPC_EINVAL
    }

    fn ioctlv(&mut self, _ctx: &mut DeviceContext<'_>, cmd: u32, argcin: u32, argcio: u32, vec: u32) -> i32 {
        tracing::warn!(
            cmd = format!("{cmd:#010X}"),
            argcin = argcin,
            argcio = argcio,
            vec = format!("{vec:#010X}"),
            "IOS_Ioctlv: unimplemented"
        );
        IPC_EINVAL
    }
}

pub struct Ipc {
    pub ppcmsg: PpcMsg,
    pub ppcctrl: PpcCtrl,
    pub armmsg: ArmMsg,
    pub armctrl: ArmCtrl,
}

impl Ipc {
    pub fn new() -> Self {
        Ipc {
            ppcmsg: PpcMsg::from_raw(0),
            ppcctrl: PpcCtrl::from_raw(0),
            armmsg: ArmMsg::from_raw(0),
            armctrl: ArmCtrl::from_raw(0),
        }
    }
}

crate::mmio_device_dispatch! {
    read = ipc_read,
    write = ipc_write,
    registers = [
        crate::hollywood::regs::PpcMsg,
        crate::hollywood::regs::PpcCtrl,
        crate::hollywood::regs::ArmMsg,
        crate::hollywood::regs::ArmCtrl,
    ],
}

pub fn deliver_response<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, cmd_paddr: u32, result: i32) {
    sys.mmio.phys_write_u32(cmd_paddr + 0x04, result as u32);
    sys.hollywood.ipc.armmsg = ArmMsg::from_raw(cmd_paddr);
    sys.hollywood.ipc.ppcctrl = sys
        .hollywood
        .ipc
        .ppcctrl
        .with_arm_response(true)
        .with_arm_post_ack(true);
    crate::hollywood::irq::assert_ipc(sys);
}
