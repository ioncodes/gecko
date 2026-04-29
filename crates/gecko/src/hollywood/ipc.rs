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

    fn read(&mut self, _ctx: &mut DeviceContext<'_>, _buf: u32, _len: u32) -> i32 {
        IPC_EINVAL
    }

    fn write(&mut self, _ctx: &mut DeviceContext<'_>, _buf: u32, _len: u32) -> i32 {
        IPC_EINVAL
    }

    fn seek(&mut self, _ctx: &mut DeviceContext<'_>, _where_: i32, _whence: i32) -> i32 {
        0
    }

    fn ioctl(
        &mut self,
        _ctx: &mut DeviceContext<'_>,
        _cmd: u32,
        _in_buf: u32,
        _in_len: u32,
        _out_buf: u32,
        _out_len: u32,
    ) -> i32 {
        IPC_EINVAL
    }

    fn ioctlv(&mut self, _ctx: &mut DeviceContext<'_>, _cmd: u32, _argcin: u32, _argcio: u32, _vec: u32) -> i32 {
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
    sys.hollywood.ipc.ppcctrl = sys.hollywood.ipc.ppcctrl.with_ack_reply(true);
    crate::hollywood::irq::assert_ipc(sys);
}
