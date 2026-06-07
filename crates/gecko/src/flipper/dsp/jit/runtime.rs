use crate::flipper::dsp::{self};
use crate::system::{GC, System, SystemId, WII};

pub extern "C" fn dsp_read_dmem_gc(sys: *mut core::ffi::c_void, addr: u32) -> u32 {
    let sys = unsafe { &mut *(sys as *mut System<GC>) };
    dsp::read_dmem(sys, addr as u16) as u32
}

pub extern "C" fn dsp_read_dmem_wii(sys: *mut core::ffi::c_void, addr: u32) -> u32 {
    let sys = unsafe { &mut *(sys as *mut System<WII>) };
    dsp::read_dmem(sys, addr as u16) as u32
}

pub extern "C" fn dsp_write_dmem_gc(sys: *mut core::ffi::c_void, addr: u32, value: u32) {
    let sys = unsafe { &mut *(sys as *mut System<GC>) };
    dsp::write_dmem(sys, addr as u16, value as u16);
}

pub extern "C" fn dsp_write_dmem_wii(sys: *mut core::ffi::c_void, addr: u32, value: u32) {
    let sys = unsafe { &mut *(sys as *mut System<WII>) };
    dsp::write_dmem(sys, addr as u16, value as u16);
}

#[inline(always)]
fn read_reg_full<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, slot: u32) -> u32 {
    let sys = unsafe { &mut *sys };
    sys.dsp.registers.read::<true>(slot as u8) as u32
}

pub extern "C" fn dsp_read_reg_full_gc(sys: *mut core::ffi::c_void, slot: u32) -> u32 {
    read_reg_full::<GC>(sys.cast(), slot)
}

pub extern "C" fn dsp_read_reg_full_wii(sys: *mut core::ffi::c_void, slot: u32) -> u32 {
    read_reg_full::<WII>(sys.cast(), slot)
}

#[inline(always)]
fn write_reg_full<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, slot: u32, value: u32) {
    let sys = unsafe { &mut *sys };
    sys.dsp.registers.write::<true>(slot as u8, value as u16);
}

pub extern "C" fn dsp_write_reg_full_gc(sys: *mut core::ffi::c_void, slot: u32, value: u32) {
    write_reg_full::<GC>(sys.cast(), slot, value);
}

pub extern "C" fn dsp_write_reg_full_wii(sys: *mut core::ffi::c_void, slot: u32, value: u32) {
    write_reg_full::<WII>(sys.cast(), slot, value);
}
