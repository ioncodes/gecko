use crate::mmio::fastmem::{GUEST_ADDRESS_SPACE, RESERVATION_BYTES};
use crate::system::{System, SystemId};
use rustc_hash::FxHashSet;
use std::cell::Cell;
use std::ffi::c_void;
use std::ops::Range;

#[cfg(target_arch = "aarch64")]
mod arm64;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

#[cfg(target_arch = "x86_64")]
mod x64;

#[derive(Clone, Copy)]
pub(super) struct Site {
    pub start: usize,
    pub end: usize,
    pub guest_pc: u32,
}

pub(super) struct FaultState {
    pub arena: Range<usize>,
    pub sites: Vec<Site>,
    pub slowmem: FxHashSet<u32>,
    pub pending_sites: Vec<u32>,
}

impl FaultState {
    pub fn new(arena: Range<usize>) -> Self {
        Self {
            arena,
            sites: Vec::new(),
            slowmem: FxHashSet::default(),
            pending_sites: Vec::new(),
        }
    }

    fn site(&self, pc: usize) -> Option<Site> {
        let index = self.sites.partition_point(|site| site.start <= pc).checked_sub(1)?;
        let site = self.sites[index];

        (pc < site.end).then_some(site)
    }
}

#[derive(Clone, Copy)]
struct Active {
    system: *mut c_void,
    state: *mut FaultState,
    base: usize,
    access: unsafe fn(*mut c_void, u32, &mut [u8], bool),
}

thread_local! {
    static ACTIVE: Cell<Option<Active>> = const { Cell::new(None) };
}

pub(super) struct Guard(Option<Active>);

impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.set(self.0));
    }
}

pub(super) unsafe fn enter<const SYSTEM: SystemId>(system: *mut System<SYSTEM>, state: *mut FaultState) -> Guard {
    let active = Active {
        system: system.cast(),
        state,
        base: unsafe { (*system).mmio.fastmem_base },
        access: access::<SYSTEM>,
    };

    Guard(ACTIVE.with(|slot| slot.replace(Some(active))))
}

pub(super) fn install() -> bool {
    #[cfg(windows)]
    return windows::install();
    #[cfg(unix)]
    return unix::install();
}

#[cfg(unix)]
pub(super) fn prepare_thread() -> bool {
    unix::prepare_thread()
}

#[derive(Clone, Default)]
struct Registers {
    pc: u64,
    // x64 register encoding order or ARM x0..x30 and SP.
    gpr: [u64; 32],
}

#[cfg(target_arch = "aarch64")]
const ARM64_SP: usize = 31;

fn handle(fault_address: usize, registers: &mut Registers) -> bool {
    ACTIVE.with(|slot| {
        let Some(active) = slot.replace(None) else {
            return false;
        };

        let handled = unsafe { handle_active(active, fault_address, registers) };
        slot.set(Some(active));

        handled
    })
}

unsafe fn handle_active(active: Active, address: usize, registers: &mut Registers) -> bool {
    let state = unsafe { &mut *active.state };
    let pc = registers.pc as usize;
    if active.base == 0 || address.wrapping_sub(active.base) >= RESERVATION_BYTES || !state.arena.contains(&pc) {
        return false;
    }

    let Some(site) = state.site(pc) else {
        fatal(pc, address, "no guest source location for faulting JIT instruction");
    };

    let mut memory = |host: u64, bytes: &mut [u8], write: bool| {
        let offset = host.wrapping_sub(active.base as u64);
        let faulting_byte = (address as u64).wrapping_sub(host);
        if offset >= GUEST_ADDRESS_SPACE as u64 || faulting_byte >= bytes.len() as u64 {
            return false;
        }

        unsafe {
            (active.access)(active.system, offset as u32, bytes, write);
        }

        true
    };

    #[cfg(target_arch = "x86_64")]
    let result = {
        let len = (state.arena.end - pc).min(x64::MAX_INSTRUCTION_BYTES);
        x64::emulate(
            unsafe { std::slice::from_raw_parts(pc as *const u8, len) },
            registers,
            &mut memory,
        )
    };
    #[cfg(target_arch = "aarch64")]
    let result = arm64::emulate(unsafe { (pc as *const u32).read() }, registers, &mut memory);

    match result {
        Ok(true) => {
            if state.slowmem.insert(site.guest_pc) {
                state.pending_sites.push(site.guest_pc);
            }

            true
        }
        Ok(false) => false,
        Err(reason) => fatal(pc, address, reason),
    }
}

#[cold]
fn fatal(pc: usize, address: usize, reason: &str) -> ! {
    eprintln!("unsupported fastmem fault: host PC={pc:#x}, address={address:#x}: {reason}");
    std::process::abort()
}

unsafe fn access<const SYSTEM: SystemId>(system: *mut c_void, ea: u32, bytes: &mut [u8], write: bool) {
    let sys = unsafe { &mut *system.cast::<System<SYSTEM>>() };

    if write {
        match bytes.len() {
            1 => sys.write_u8(ea, bytes[0]),
            2 => sys.write_u16(ea, u16::from_be_bytes(bytes.try_into().unwrap())),
            4 => sys.write_u32(ea, u32::from_be_bytes(bytes.try_into().unwrap())),
            8 => sys.write_f64(ea, f64::from_bits(u64::from_be_bytes(bytes.try_into().unwrap()))),
            _ => unreachable!(),
        }
    } else {
        match bytes.len() {
            1 => bytes[0] = sys.read_u8(ea),
            2 => bytes.copy_from_slice(&sys.read_u16(ea).to_be_bytes()),
            4 => bytes.copy_from_slice(&sys.read_u32(ea).to_be_bytes()),
            8 => bytes.copy_from_slice(&sys.read_f64(ea).to_bits().to_be_bytes()),
            _ => unreachable!(),
        }
    }
}
