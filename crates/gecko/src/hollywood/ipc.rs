use crate::system::{System, SystemId};

#[inline(always)]
pub fn ipc_read<const SYSTEM: SystemId>(_sys: &mut System<SYSTEM>, addr: u32, size: u32) -> Option<u32> {
    tracing::error!("unhandled IPC read at {:#010X} (size {})", addr, size);
    Some(0)
}

#[inline(always)]
pub fn ipc_write<const SYSTEM: SystemId>(_sys: &mut System<SYSTEM>, addr: u32, size: u32, val: u32) -> bool {
    tracing::error!(
        "unhandled IPC write at {:#010X} (size {}, value {:#010X})",
        addr,
        size,
        val
    );
    true
}
