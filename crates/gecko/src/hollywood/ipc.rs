pub mod di;
pub mod es;
pub mod fs;
pub mod sdio;
pub mod stm;
pub mod sysconf;
pub mod usb;

use crate::dvd::DvdInterface;
use crate::flipper::pi::ProcessorInterface;
use crate::hollywood::regs::{ArmCtrl, ArmMsg, PpcCtrl, PpcMsg};
use crate::mmio::Mmio;
use crate::scheduler::Scheduler;
use crate::system::{System, SystemId, WII};

pub const IPC_EINVAL: i32 = -4;
pub const IPC_ENOENT: i32 = -6;

pub struct DeviceContext<'a> {
    pub mmio: &'a mut Mmio<{ WII }>,
    pub scheduler: &'a mut Scheduler<{ WII }>,
    pub di: &'a mut DvdInterface,
    pub pi: &'a mut ProcessorInterface,
    pub device_path: String,
}

pub trait IosDevice: Send {
    fn open(&mut self, _ctx: &mut DeviceContext<'_>, _mode: u32) -> i32 {
        0
    }

    fn close(&mut self, _ctx: &mut DeviceContext<'_>) -> i32 {
        0
    }

    fn read(&mut self, ctx: &mut DeviceContext<'_>, out_ptr: u32, out_len: u32) -> i32 {
        tracing::warn!(
            device = &ctx.device_path,
            out_ptr = format!("{out_ptr:#010X}"),
            out_len,
            "IOS_Read: unimplemented"
        );
        IPC_EINVAL
    }

    fn write(&mut self, ctx: &mut DeviceContext<'_>, in_ptr: u32, in_len: u32) -> i32 {
        tracing::warn!(
            device = &ctx.device_path,
            in_ptr = format!("{in_ptr:#010X}"),
            in_len,
            "IOS_Write: unimplemented"
        );
        IPC_EINVAL
    }

    fn seek(&mut self, ctx: &mut DeviceContext<'_>, offset: i32, whence: i32) -> i32 {
        tracing::warn!(device = &ctx.device_path, offset, whence, "IOS_Seek: unimplemented");
        0
    }

    fn ioctl(
        &mut self,
        ctx: &mut DeviceContext<'_>,
        cmd: u32,
        in_ptr: u32,
        in_len: u32,
        out_ptr: u32,
        out_len: u32,
    ) -> i32 {
        tracing::warn!(
            device = &ctx.device_path,
            cmd = format!("{cmd:#010X}"),
            in_buf = format!("{:02X?}", ctx.mmio.phys_slice(in_ptr, in_len as usize)),
            in_len,
            out_buf = format!("{:02X?}", ctx.mmio.phys_slice(out_ptr, out_len as usize)),
            out_len,
            "IOS_Ioctl: unimplemented"
        );
        IPC_EINVAL
    }

    fn ioctlv(&mut self, ctx: &mut DeviceContext<'_>, cmd: u32, in_count: u32, io_count: u32, vec_ptr: u32) -> i32 {
        tracing::warn!(
            device = &ctx.device_path,
            cmd = format!("{cmd:#010X}"),
            in_count,
            io_count,
            vec_ptr = format!("{vec_ptr:#010X}"),
            "IOS_Ioctlv: unimplemented"
        );
        IPC_EINVAL
    }

    fn set_wiimote_buttons(&mut self, _buttons: u16) -> bool {
        false
    }

    fn set_wiimote_shake(&mut self, _active: bool) {}

    fn set_nunchuk(&mut self, _buttons: u8, _stick_x: u8, _stick_y: u8) -> bool {
        false
    }

    fn set_ir_pointer(&mut self, _pointer: Option<(u16, u16)>) -> bool {
        false
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
    let cmd_type = sys.mmio.phys_read_u32(cmd_paddr);
    sys.mmio.phys_write_u32(cmd_paddr + 0x04, result as u32);
    sys.mmio.phys_write_u32(cmd_paddr + 0x08, cmd_type);
    sys.hollywood.ipc.armmsg = ArmMsg::from_raw(cmd_paddr);
    sys.hollywood.ipc.ppcctrl = sys
        .hollywood
        .ipc
        .ppcctrl
        .with_arm_response(true)
        .with_arm_post_ack(true);
    tracing::debug!(
        cmd_paddr = format!("{cmd_paddr:#010X}"),
        cmd_type,
        result,
        irq_mask = sys.hollywood.irq.mask.raw(),
        "deliver_response: posting reply"
    );
    crate::hollywood::irq::assert_ipc(sys);
}
