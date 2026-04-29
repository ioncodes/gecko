use crate::hollywood::regs::{ArmCtrl, ArmMsg, PpcCtrl, PpcMsg};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IpcState {
    Idle,
    Processing,
    Done,
}

pub struct Ipc {
    pub ppcmsg: PpcMsg,
    pub ppcctrl: PpcCtrl,
    pub armmsg: ArmMsg,
    pub armctrl: ArmCtrl,
    pub state: IpcState,
}

impl Ipc {
    pub fn new() -> Self {
        Ipc {
            ppcmsg: PpcMsg::from_raw(0),
            ppcctrl: PpcCtrl::from_raw(0),
            armmsg: ArmMsg::from_raw(0),
            armctrl: ArmCtrl::from_raw(0),
            state: IpcState::Idle,
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
