use crate::hollywood::regs::{Cause, Mask};
use crate::system::{System, SystemId};

pub struct Irq {
    pub cause: Cause,
    pub mask: Mask,
}

impl Irq {
    pub fn new() -> Self {
        Irq {
            cause: Cause::from_raw(0),
            mask: Mask::from_raw(0),
        }
    }
}

#[inline(always)]
pub fn route_to_pi<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    let firing = (sys.hollywood.irq.cause.raw() & sys.hollywood.irq.mask.raw()) != 0;
    if firing {
        sys.pi.assert_interrupt(crate::flipper::pi::InterruptFlag::Hsp);
    } else {
        sys.pi.clear_interrupt(crate::flipper::pi::InterruptFlag::Hsp);
    }
}

#[inline(always)]
pub fn assert_ipc<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    sys.hollywood.irq.cause = sys.hollywood.irq.cause.with_ipc(true);
    super::irq::route_to_pi(sys);
}

#[inline(always)]
pub fn ack_ipc<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    sys.hollywood.irq.cause = sys.hollywood.irq.cause.with_ipc(false);
    super::irq::route_to_pi(sys);
}

crate::mmio_device_dispatch! {
    read = irq_read,
    write = irq_write,
    registers = [Cause, Mask],
}
