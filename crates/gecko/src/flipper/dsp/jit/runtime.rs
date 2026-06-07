use crate::flipper::dsp::{self};
use crate::system::{GC, System, SystemId, WII};

#[inline(always)]
fn cache_ext_ac<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>) {
    let sys = unsafe { &mut *sys };
    sys.dsp.registers.cache_ext_ac();
}

pub extern "C" fn dsp_cache_ext_ac_gc(sys: *mut core::ffi::c_void) {
    cache_ext_ac::<GC>(sys.cast());
}

pub extern "C" fn dsp_cache_ext_ac_wii(sys: *mut core::ffi::c_void) {
    cache_ext_ac::<WII>(sys.cast());
}

#[inline(always)]
fn loop_tail<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>) {
    let sys = unsafe { &mut *sys };
    let dsp = &mut sys.dsp;
    let at_loop_end = !dsp.registers.loop_addr.is_empty() && dsp.registers.nia == dsp.registers.loop_addr.top();

    if at_loop_end {
        let counter = dsp.registers.loop_counter.top().wrapping_sub(1);
        if counter != 0 {
            dsp.registers.loop_counter.set_top(counter);
            dsp.registers.nia = dsp.registers.call_stack.top();
        } else {
            dsp.registers.loop_counter.pop();
            dsp.registers.loop_addr.pop();
            dsp.registers.call_stack.pop();
        }
    }

    dsp.registers.pc = dsp.registers.nia;
}

pub extern "C" fn dsp_loop_tail_gc(sys: *mut core::ffi::c_void) {
    loop_tail::<GC>(sys.cast());
}

pub extern "C" fn dsp_loop_tail_wii(sys: *mut core::ffi::c_void) {
    loop_tail::<WII>(sys.cast());
}

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

pub extern "C" fn dsp_increment_ar_gc(sys: *mut core::ffi::c_void, reg: u32) -> u32 {
    let sys = unsafe { &mut *(sys as *mut System<GC>) };
    sys.dsp.registers.increment_ar(reg as usize) as u32
}

pub extern "C" fn dsp_increment_ar_wii(sys: *mut core::ffi::c_void, reg: u32) -> u32 {
    let sys = unsafe { &mut *(sys as *mut System<WII>) };
    sys.dsp.registers.increment_ar(reg as usize) as u32
}

pub extern "C" fn dsp_decrement_ar_gc(sys: *mut core::ffi::c_void, reg: u32) -> u32 {
    let sys = unsafe { &mut *(sys as *mut System<GC>) };
    sys.dsp.registers.decrement_ar(reg as usize) as u32
}

pub extern "C" fn dsp_decrement_ar_wii(sys: *mut core::ffi::c_void, reg: u32) -> u32 {
    let sys = unsafe { &mut *(sys as *mut System<WII>) };
    sys.dsp.registers.decrement_ar(reg as usize) as u32
}

pub extern "C" fn dsp_increase_ar_gc(sys: *mut core::ffi::c_void, reg: u32, ix: u32) -> u32 {
    let sys = unsafe { &mut *(sys as *mut System<GC>) };
    sys.dsp.registers.increase_ar(reg as usize, ix as i16) as u32
}

pub extern "C" fn dsp_increase_ar_wii(sys: *mut core::ffi::c_void, reg: u32, ix: u32) -> u32 {
    let sys = unsafe { &mut *(sys as *mut System<WII>) };
    sys.dsp.registers.increase_ar(reg as usize, ix as i16) as u32
}

pub extern "C" fn dsp_decrease_ar_ix_gc(sys: *mut core::ffi::c_void, reg: u32, ix: u32) -> u32 {
    let sys = unsafe { &mut *(sys as *mut System<GC>) };
    sys.dsp.registers.decrease_ar_ix(reg as usize, ix as i16) as u32
}

pub extern "C" fn dsp_decrease_ar_ix_wii(sys: *mut core::ffi::c_void, reg: u32, ix: u32) -> u32 {
    let sys = unsafe { &mut *(sys as *mut System<WII>) };
    sys.dsp.registers.decrease_ar_ix(reg as usize, ix as i16) as u32
}

#[inline(always)]
fn dynamic_shift_impl<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, d: u32, shift_val: u32, mode: u32) {
    let logical = mode & 1 != 0;
    let reversed = mode & 2 != 0;

    let sys = unsafe { &mut *sys };

    let regs = &mut sys.dsp.registers;
    let shift_val = shift_val as i16;
    let low6 = (shift_val & 63) as u32;
    let bit6 = shift_val & 64 != 0;
    let amount = if bit6 { (64 - low6) % 64 } else { low6 };
    let shift_left = if reversed { !bit6 } else { bit6 };
    let dn = d as u8;

    if shift_left {
        let ac = ((regs.ac(dn) as u64 & 0xFF_FFFF_FFFF) << amount) as i64;
        regs.set_ac(dn, ac);
    } else if amount != 0 {
        let ac = if logical {
            ((regs.ac(dn) as u64 & 0xFF_FFFF_FFFF) >> amount) as i64
        } else {
            regs.ac(dn) >> amount
        };
        regs.set_ac(dn, ac);
    }

    let ac = regs.ac(dn);
    regs.update_flags_ac(ac);
    regs.status.set_o(false);
    regs.status.set_c(false);
}

pub extern "C" fn dsp_dynamic_shift_gc(sys: *mut core::ffi::c_void, d: u32, shift_val: u32, mode: u32) {
    dynamic_shift_impl::<GC>(sys.cast(), d, shift_val, mode);
}

pub extern "C" fn dsp_dynamic_shift_wii(sys: *mut core::ffi::c_void, d: u32, shift_val: u32, mode: u32) {
    dynamic_shift_impl::<WII>(sys.cast(), d, shift_val, mode);
}

#[inline(always)]
fn read_imem<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, addr: u32) -> u32 {
    let sys = unsafe { &mut *sys };
    sys.dsp.read_imem(addr as u16) as u32
}

pub extern "C" fn dsp_read_imem_gc(sys: *mut core::ffi::c_void, addr: u32) -> u32 {
    read_imem::<GC>(sys.cast(), addr)
}

pub extern "C" fn dsp_read_imem_wii(sys: *mut core::ffi::c_void, addr: u32) -> u32 {
    read_imem::<WII>(sys.cast(), addr)
}

#[inline(always)]
fn call_stack_push<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, value: u32) {
    let sys = unsafe { &mut *sys };
    sys.dsp.registers.call_stack.push(value as u16);
}

pub extern "C" fn dsp_call_stack_push_gc(sys: *mut core::ffi::c_void, value: u32) {
    call_stack_push::<GC>(sys.cast(), value);
}

pub extern "C" fn dsp_call_stack_push_wii(sys: *mut core::ffi::c_void, value: u32) {
    call_stack_push::<WII>(sys.cast(), value);
}

#[inline(always)]
fn call_stack_pop<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>) -> u32 {
    let sys = unsafe { &mut *sys };
    sys.dsp.registers.call_stack.pop() as u32
}

#[inline(always)]
fn data_stack_pop<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>) -> u32 {
    let sys = unsafe { &mut *sys };
    sys.dsp.registers.data_stack.pop() as u32
}

pub extern "C" fn dsp_call_stack_pop_gc(sys: *mut core::ffi::c_void) -> u32 {
    call_stack_pop::<GC>(sys.cast())
}

pub extern "C" fn dsp_call_stack_pop_wii(sys: *mut core::ffi::c_void) -> u32 {
    call_stack_pop::<WII>(sys.cast())
}

pub extern "C" fn dsp_data_stack_pop_gc(sys: *mut core::ffi::c_void) -> u32 {
    data_stack_pop::<GC>(sys.cast())
}

pub extern "C" fn dsp_data_stack_pop_wii(sys: *mut core::ffi::c_void) -> u32 {
    data_stack_pop::<WII>(sys.cast())
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

#[inline(always)]
fn loop_setup<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, end_addr_p1: u32, counter: u32, call_stack_val: u32) {
    let sys = unsafe { &mut *sys };
    let regs = &mut sys.dsp.registers;

    if counter != 0 {
        regs.call_stack.push(call_stack_val as u16);
        regs.loop_addr.push(end_addr_p1 as u16);
        regs.loop_counter.push(counter as u16);
    } else {
        regs.nia = end_addr_p1 as u16;
    }
}

pub extern "C" fn dsp_loop_setup_gc(sys: *mut core::ffi::c_void, end_addr_p1: u32, counter: u32, call_stack_val: u32) {
    loop_setup::<GC>(sys.cast(), end_addr_p1, counter, call_stack_val);
}

pub extern "C" fn dsp_loop_setup_wii(sys: *mut core::ffi::c_void, end_addr_p1: u32, counter: u32, call_stack_val: u32) {
    loop_setup::<WII>(sys.cast(), end_addr_p1, counter, call_stack_val);
}
