use crate::gekko::msr::Msr;
use crate::gekko::sr::Sr;
use crate::system::{GC, System, SystemId, WII};

#[cold]
#[inline(never)]
fn cause_invalid_opcode<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, raw_instr: u32, pc: u32) -> u32 {
    let sys = unsafe { &mut *sys };
    panic!(
        "unimplemented Gekko opcode {:#010x} at pc={:#010x} lr={:#010x}",
        raw_instr, pc, sys.gekko.spr.lr,
    );
}

pub extern "C" fn cause_invalid_opcode_gc(sys: *mut core::ffi::c_void, raw_instr: u32, pc: u32) -> u32 {
    cause_invalid_opcode::<GC>(sys.cast(), raw_instr, pc)
}

pub extern "C" fn cause_invalid_opcode_wii(sys: *mut core::ffi::c_void, raw_instr: u32, pc: u32) -> u32 {
    cause_invalid_opcode::<WII>(sys.cast(), raw_instr, pc)
}

#[inline(always)]
fn advance_to_deadline<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>) {
    let sys = unsafe { &mut *sys };
    let deadline = sys.scheduler.next_deadline();
    if sys.scheduler.cycles < deadline {
        sys.scheduler.cycles = deadline;
    }
}

pub extern "C" fn advance_to_deadline_gc(sys: *mut core::ffi::c_void) {
    advance_to_deadline::<GC>(sys.cast());
}

pub extern "C" fn advance_to_deadline_wii(sys: *mut core::ffi::c_void) {
    advance_to_deadline::<WII>(sys.cast());
}

#[inline(always)]
fn slow_read_u8<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, ea: u32) -> u8 {
    unsafe { (*sys).read_u8(ea) }
}
#[inline(always)]
fn slow_read_u16<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, ea: u32) -> u16 {
    unsafe { (*sys).read_u16(ea) }
}
#[inline(always)]
fn slow_read_u32<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, ea: u32) -> u32 {
    unsafe { (*sys).read_u32(ea) }
}
#[inline(always)]
fn slow_write_u8<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, ea: u32, v: u8) {
    unsafe { (*sys).write_u8(ea, v) }
}
#[inline(always)]
fn slow_write_u16<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, ea: u32, v: u16) {
    unsafe { (*sys).write_u16(ea, v) }
}
#[inline(always)]
fn slow_write_u32<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, ea: u32, v: u32) {
    unsafe { (*sys).write_u32(ea, v) }
}

pub extern "C" fn read_u8_gc(sys: *mut core::ffi::c_void, ea: u32) -> u32 {
    slow_read_u8::<GC>(sys.cast(), ea) as u32
}
pub extern "C" fn read_u16_gc(sys: *mut core::ffi::c_void, ea: u32) -> u32 {
    slow_read_u16::<GC>(sys.cast(), ea) as u32
}
pub extern "C" fn read_u32_gc(sys: *mut core::ffi::c_void, ea: u32) -> u32 {
    slow_read_u32::<GC>(sys.cast(), ea)
}
pub extern "C" fn write_u8_gc(sys: *mut core::ffi::c_void, ea: u32, v: u32) {
    slow_write_u8::<GC>(sys.cast(), ea, v as u8);
}
pub extern "C" fn write_u16_gc(sys: *mut core::ffi::c_void, ea: u32, v: u32) {
    slow_write_u16::<GC>(sys.cast(), ea, v as u16);
}
pub extern "C" fn write_u32_gc(sys: *mut core::ffi::c_void, ea: u32, v: u32) {
    slow_write_u32::<GC>(sys.cast(), ea, v);
}

pub extern "C" fn read_u8_wii(sys: *mut core::ffi::c_void, ea: u32) -> u32 {
    slow_read_u8::<WII>(sys.cast(), ea) as u32
}
pub extern "C" fn read_u16_wii(sys: *mut core::ffi::c_void, ea: u32) -> u32 {
    slow_read_u16::<WII>(sys.cast(), ea) as u32
}
pub extern "C" fn read_u32_wii(sys: *mut core::ffi::c_void, ea: u32) -> u32 {
    slow_read_u32::<WII>(sys.cast(), ea)
}
pub extern "C" fn write_u8_wii(sys: *mut core::ffi::c_void, ea: u32, v: u32) {
    slow_write_u8::<WII>(sys.cast(), ea, v as u8);
}
pub extern "C" fn write_u16_wii(sys: *mut core::ffi::c_void, ea: u32, v: u32) {
    slow_write_u16::<WII>(sys.cast(), ea, v as u16);
}
pub extern "C" fn write_u32_wii(sys: *mut core::ffi::c_void, ea: u32, v: u32) {
    slow_write_u32::<WII>(sys.cast(), ea, v);
}

#[inline(always)]
fn slow_read_f32<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, ea: u32) -> f64 {
    unsafe { (*sys).read_f32(ea) }
}
#[inline(always)]
fn slow_write_f32<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, ea: u32, v: f64) {
    unsafe { (*sys).write_f32(ea, v) }
}
#[inline(always)]
fn slow_read_f64<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, ea: u32) -> f64 {
    unsafe { (*sys).read_f64(ea) }
}
#[inline(always)]
fn slow_write_f64<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, ea: u32, v: f64) {
    unsafe { (*sys).write_f64(ea, v) }
}

pub extern "C" fn read_f32_gc(sys: *mut core::ffi::c_void, ea: u32) -> f64 {
    slow_read_f32::<GC>(sys.cast(), ea)
}
pub extern "C" fn write_f32_gc(sys: *mut core::ffi::c_void, ea: u32, v: f64) {
    slow_write_f32::<GC>(sys.cast(), ea, v)
}
pub extern "C" fn read_f64_gc(sys: *mut core::ffi::c_void, ea: u32) -> f64 {
    slow_read_f64::<GC>(sys.cast(), ea)
}
pub extern "C" fn write_f64_gc(sys: *mut core::ffi::c_void, ea: u32, v: f64) {
    slow_write_f64::<GC>(sys.cast(), ea, v)
}
pub extern "C" fn read_f32_wii(sys: *mut core::ffi::c_void, ea: u32) -> f64 {
    slow_read_f32::<WII>(sys.cast(), ea)
}
pub extern "C" fn write_f32_wii(sys: *mut core::ffi::c_void, ea: u32, v: f64) {
    slow_write_f32::<WII>(sys.cast(), ea, v)
}
pub extern "C" fn read_f64_wii(sys: *mut core::ffi::c_void, ea: u32) -> f64 {
    slow_read_f64::<WII>(sys.cast(), ea)
}
pub extern "C" fn write_f64_wii(sys: *mut core::ffi::c_void, ea: u32, v: f64) {
    slow_write_f64::<WII>(sys.cast(), ea, v)
}

#[inline(always)]
fn write_msr<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, val: u32) {
    unsafe { (*sys).gekko.msr = Msr::from(val) }
}

pub extern "C" fn write_msr_gc(sys: *mut core::ffi::c_void, val: u32) {
    write_msr::<GC>(sys.cast(), val);
}
pub extern "C" fn write_msr_wii(sys: *mut core::ffi::c_void, val: u32) {
    write_msr::<WII>(sys.cast(), val);
}

#[inline(always)]
fn read_spr<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, num: u32) -> u32 {
    let sys = unsafe { &mut *sys };
    match num {
        22 => {
            sys.gekko.spr.dec = sys.gekko.dec.read(sys.scheduler.cycles);
            sys.gekko.spr.dec
        }
        268 => sys.scheduler.timebase_lower(),
        269 => sys.scheduler.timebase_upper(),
        _ => sys.gekko.spr.read(num),
    }
}

pub extern "C" fn read_spr_gc(sys: *mut core::ffi::c_void, num: u32) -> u32 {
    read_spr::<GC>(sys.cast(), num)
}
pub extern "C" fn read_spr_wii(sys: *mut core::ffi::c_void, num: u32) -> u32 {
    read_spr::<WII>(sys.cast(), num)
}

#[inline(always)]
fn write_spr<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, num: u32, val: u32) {
    let sys = unsafe { &mut *sys };
    match num {
        22 => {
            sys.scheduler.cancel(crate::gekko::dec::underflow_handler::<SYSTEM>);
            sys.gekko.dec.write(sys.scheduler.cycles, val);
            sys.gekko.spr.dec = val;
            sys.scheduler.schedule_in(
                crate::gekko::dec::cycles_until_underflow(val),
                crate::gekko::dec::underflow_handler::<SYSTEM>,
            );
        }
        284 => sys.scheduler.set_timebase_lower(val),
        285 => sys.scheduler.set_timebase_upper(val),
        923 => {
            sys.gekko.spr.dmal = crate::gekko::spr::DmaLower::from_raw(val);
            if sys.gekko.spr.dmal.trigger() {
                sys.mmio
                    .process_locked_cache_dma(&sys.gekko.spr.dmau, &sys.gekko.spr.dmal);
            }
        }
        _ => sys.gekko.spr.write(num, val),
    }
}

pub extern "C" fn write_spr_gc(sys: *mut core::ffi::c_void, num: u32, val: u32) {
    write_spr::<GC>(sys.cast(), num, val);
}
pub extern "C" fn write_spr_wii(sys: *mut core::ffi::c_void, num: u32, val: u32) {
    write_spr::<WII>(sys.cast(), num, val);
}

#[inline(always)]
fn read_sr<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, idx: u32) -> u32 {
    unsafe { (*sys).gekko.sr[(idx & 0xF) as usize].raw() }
}
pub extern "C" fn read_sr_gc(sys: *mut core::ffi::c_void, idx: u32) -> u32 {
    read_sr::<GC>(sys.cast(), idx)
}
pub extern "C" fn read_sr_wii(sys: *mut core::ffi::c_void, idx: u32) -> u32 {
    read_sr::<WII>(sys.cast(), idx)
}

#[inline(always)]
fn write_sr<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, idx: u32, val: u32) {
    unsafe { (*sys).gekko.sr[(idx & 0xF) as usize] = Sr::from_raw(val) }
}
pub extern "C" fn write_sr_gc(sys: *mut core::ffi::c_void, idx: u32, val: u32) {
    write_sr::<GC>(sys.cast(), idx, val);
}
pub extern "C" fn write_sr_wii(sys: *mut core::ffi::c_void, idx: u32, val: u32) {
    write_sr::<WII>(sys.cast(), idx, val);
}

#[inline(always)]
#[allow(dead_code)]
fn cause_trap_exception<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>) {
    unsafe { (*sys).cause_trap_exception() }
}
#[allow(dead_code)]
pub extern "C" fn cause_trap_exception_gc(sys: *mut core::ffi::c_void) {
    cause_trap_exception::<GC>(sys.cast());
}
#[allow(dead_code)]
pub extern "C" fn cause_trap_exception_wii(sys: *mut core::ffi::c_void) {
    cause_trap_exception::<WII>(sys.cast());
}

#[inline(always)]
#[allow(dead_code)]
fn cause_syscall_interrupt<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>) {
    unsafe { (*sys).cause_syscall_interrupt() }
}
#[allow(dead_code)]
pub extern "C" fn cause_syscall_interrupt_gc(sys: *mut core::ffi::c_void) {
    cause_syscall_interrupt::<GC>(sys.cast());
}
#[allow(dead_code)]
pub extern "C" fn cause_syscall_interrupt_wii(sys: *mut core::ffi::c_void) {
    cause_syscall_interrupt::<WII>(sys.cast());
}

#[inline(always)]
#[allow(dead_code)]
fn cause_fp_unavailable<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, pc: u32) {
    let sys = unsafe { &mut *sys };
    sys.gekko.cia = pc;
    sys.gekko.nia = pc.wrapping_add(4);
    sys.cause_fp_unavailable();
}
#[allow(dead_code)]
pub extern "C" fn cause_fp_unavailable_gc(sys: *mut core::ffi::c_void, pc: u32) {
    cause_fp_unavailable::<GC>(sys.cast(), pc);
}
#[allow(dead_code)]
pub extern "C" fn cause_fp_unavailable_wii(sys: *mut core::ffi::c_void, pc: u32) {
    cause_fp_unavailable::<WII>(sys.cast(), pc);
}

#[inline(always)]
#[allow(dead_code)]
fn set_reservation<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, addr: u32) {
    unsafe { (*sys).gekko.reserve_addr = addr }
}
#[allow(dead_code)]
pub extern "C" fn set_reservation_gc(sys: *mut core::ffi::c_void, addr: u32) {
    set_reservation::<GC>(sys.cast(), addr);
}
#[allow(dead_code)]
pub extern "C" fn set_reservation_wii(sys: *mut core::ffi::c_void, addr: u32) {
    set_reservation::<WII>(sys.cast(), addr);
}

#[inline(always)]
#[allow(dead_code)]
fn try_clear_reservation<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, addr: u32) -> u32 {
    let sys = unsafe { &mut *sys };
    if sys.gekko.reserve_addr == addr {
        sys.gekko.reserve_addr = crate::gekko::Gekko::NO_RESERVATION;
        1
    } else {
        0
    }
}
#[allow(dead_code)]
pub extern "C" fn try_clear_reservation_gc(sys: *mut core::ffi::c_void, addr: u32) -> u32 {
    try_clear_reservation::<GC>(sys.cast(), addr)
}
#[allow(dead_code)]
pub extern "C" fn try_clear_reservation_wii(sys: *mut core::ffi::c_void, addr: u32) -> u32 {
    try_clear_reservation::<WII>(sys.cast(), addr)
}

#[inline(always)]
fn do_lsw_packed<const SYSTEM: SystemId, const STORE: bool>(
    sys: *mut System<SYSTEM>,
    ea: u32,
    rd_or_rs: u32,
    n_bytes: u32,
) {
    let sys = unsafe { &mut *sys };
    let mut n = n_bytes;
    let mut r = rd_or_rs.wrapping_sub(1) & 31;
    let mut i = 0u32;
    let mut addr = ea;
    while n > 0 {
        if i == 0 {
            r = (r + 1) & 31;
            if !STORE {
                sys.gekko.write_gpr(r as u8, 0);
            }
        }
        if STORE {
            let byte = (sys.gekko.read_gpr(r as u8) >> (24 - i)) as u8;
            sys.write_u8(addr, byte);
        } else {
            let byte = sys.read_u8(addr) as u32;
            let shift = 24 - i;
            let val = sys.gekko.read_gpr(r as u8) | (byte << shift);
            sys.gekko.write_gpr(r as u8, val);
        }
        i += 8;
        if i == 32 {
            i = 0;
        }
        addr = addr.wrapping_add(1);
        n -= 1;
    }
}

pub extern "C" fn do_lswi_gc(sys: *mut core::ffi::c_void, ea: u32, rd: u32, nb: u32) {
    do_lsw_packed::<GC, false>(sys.cast(), ea, rd, nb);
}
pub extern "C" fn do_lswi_wii(sys: *mut core::ffi::c_void, ea: u32, rd: u32, nb: u32) {
    do_lsw_packed::<WII, false>(sys.cast(), ea, rd, nb);
}
pub extern "C" fn do_stswi_gc(sys: *mut core::ffi::c_void, ea: u32, rs: u32, nb: u32) {
    do_lsw_packed::<GC, true>(sys.cast(), ea, rs, nb);
}
pub extern "C" fn do_stswi_wii(sys: *mut core::ffi::c_void, ea: u32, rs: u32, nb: u32) {
    do_lsw_packed::<WII, true>(sys.cast(), ea, rs, nb);
}

pub extern "C" fn do_lswx_gc(sys_v: *mut core::ffi::c_void, ea: u32, rd: u32) {
    let sys = unsafe { &mut *(sys_v as *mut System<GC>) };
    let n = sys.gekko.spr.xer.byte_count() as u32;
    if n == 0 {
        return;
    }
    do_lsw_packed::<GC, false>(sys_v.cast(), ea, rd, n);
}
pub extern "C" fn do_lswx_wii(sys_v: *mut core::ffi::c_void, ea: u32, rd: u32) {
    let sys = unsafe { &mut *(sys_v as *mut System<WII>) };
    let n = sys.gekko.spr.xer.byte_count() as u32;
    if n == 0 {
        return;
    }
    do_lsw_packed::<WII, false>(sys_v.cast(), ea, rd, n);
}
pub extern "C" fn do_stswx_gc(sys_v: *mut core::ffi::c_void, ea: u32, rs: u32) {
    let sys = unsafe { &mut *(sys_v as *mut System<GC>) };
    let n = sys.gekko.spr.xer.byte_count() as u32;
    do_lsw_packed::<GC, true>(sys_v.cast(), ea, rs, n);
}
pub extern "C" fn do_stswx_wii(sys_v: *mut core::ffi::c_void, ea: u32, rs: u32) {
    let sys = unsafe { &mut *(sys_v as *mut System<WII>) };
    let n = sys.gekko.spr.xer.byte_count() as u32;
    do_lsw_packed::<WII, true>(sys_v.cast(), ea, rs, n);
}

pub static DEQUANT_TABLE: [f32; 64] = {
    let mut t = [0.0f32; 64];
    let mut i = 0u32;
    while i < 32 {
        t[i as usize] = 1.0 / (1u64 << i) as f32;
        i += 1;
    }
    while i < 64 {
        t[i as usize] = (1u64 << (64 - i)) as f32;
        i += 1;
    }
    t
};
pub static QUANT_TABLE: [f32; 64] = {
    let mut t = [0.0f32; 64];
    let mut i = 0u32;
    while i < 32 {
        t[i as usize] = (1u64 << i) as f32;
        i += 1;
    }
    while i < 64 {
        t[i as usize] = 1.0 / (1u64 << (64 - i)) as f32;
        i += 1;
    }
    t
};

#[inline(always)]
fn quant_element_size(qtype: u8) -> u32 {
    match qtype {
        0 => 4,
        4 | 6 => 1,
        5 | 7 => 2,
        _ => 4,
    }
}

#[inline(always)]
fn psq_dequant<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, addr: u32, ld_type: u8, ld_scale: u8) -> f64 {
    let scale = DEQUANT_TABLE[ld_scale as usize];
    match ld_type {
        0 => sys.read_f32(addr),
        4 => (sys.read_u8(addr) as f32 * scale) as f64,
        5 => (sys.read_u16(addr) as f32 * scale) as f64,
        6 => (sys.read_u8(addr) as i8 as f32 * scale) as f64,
        7 => (sys.read_u16(addr) as i16 as f32 * scale) as f64,
        _ => 0.0,
    }
}

#[inline(always)]
fn psq_quant<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>, addr: u32, value: f64, st_type: u8, st_scale: u8) {
    let scale = QUANT_TABLE[st_scale as usize];
    match st_type {
        0 => sys.write_f32(addr, value),
        4 => {
            let v = (value as f32 * scale).clamp(u8::MIN as f32, u8::MAX as f32) as u8;
            sys.write_u8(addr, v);
        }
        5 => {
            let v = (value as f32 * scale).clamp(u16::MIN as f32, u16::MAX as f32) as u16;
            sys.write_u16(addr, v);
        }
        6 => {
            let v = (value as f32 * scale).clamp(i8::MIN as f32, i8::MAX as f32) as i8;
            sys.write_u8(addr, v as u8);
        }
        7 => {
            let v = (value as f32 * scale).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            sys.write_u16(addr, v as u16);
        }
        _ => {}
    }
}

#[inline(always)]
fn psq_load_inner<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, fd: u32, ea: u32, w: u32, gqr_idx: u32) {
    let sys = unsafe { &mut *sys };
    let gqr = sys.gekko.spr.read_gqr((gqr_idx & 0x7) as u8);
    let ld_type = ((gqr >> 16) & 0x7) as u8;
    let ld_scale = ((gqr >> 24) & 0x3F) as u8;
    let ps0 = psq_dequant(sys, ea, ld_type, ld_scale);
    let ps1 = if w != 0 {
        1.0
    } else {
        let elem = quant_element_size(ld_type);
        psq_dequant(sys, ea.wrapping_add(elem), ld_type, ld_scale)
    };
    sys.gekko.write_fpr(fd as u8, ps0);
    sys.gekko.write_ps1(fd as u8, ps1);
}

#[inline(always)]
fn psq_store_inner<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, fs: u32, ea: u32, w: u32, gqr_idx: u32) {
    let sys = unsafe { &mut *sys };
    let gqr = sys.gekko.spr.read_gqr((gqr_idx & 0x7) as u8);
    let st_type = (gqr & 0x7) as u8;
    let st_scale = ((gqr >> 8) & 0x3F) as u8;
    let ps0 = sys.gekko.read_fpr(fs as u8);
    psq_quant(sys, ea, ps0, st_type, st_scale);
    if w == 0 {
        let ps1 = sys.gekko.read_ps1(fs as u8);
        let elem = quant_element_size(st_type);
        psq_quant(sys, ea.wrapping_add(elem), ps1, st_type, st_scale);
    }
}

pub extern "C" fn do_psq_load_gc(sys: *mut core::ffi::c_void, fd: u32, ea: u32, w: u32, gqr: u32) {
    psq_load_inner::<GC>(sys.cast(), fd, ea, w, gqr);
}
pub extern "C" fn do_psq_load_wii(sys: *mut core::ffi::c_void, fd: u32, ea: u32, w: u32, gqr: u32) {
    psq_load_inner::<WII>(sys.cast(), fd, ea, w, gqr);
}
pub extern "C" fn do_psq_store_gc(sys: *mut core::ffi::c_void, fs: u32, ea: u32, w: u32, gqr: u32) {
    psq_store_inner::<GC>(sys.cast(), fs, ea, w, gqr);
}
pub extern "C" fn do_psq_store_wii(sys: *mut core::ffi::c_void, fs: u32, ea: u32, w: u32, gqr: u32) {
    psq_store_inner::<WII>(sys.cast(), fs, ea, w, gqr);
}

#[inline(always)]
fn read_timebase<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>, tbr: u32) -> u32 {
    let sys = unsafe { &*sys };
    match tbr {
        268 => sys.scheduler.timebase_lower(),
        269 => sys.scheduler.timebase_upper(),
        _ => 0,
    }
}
pub extern "C" fn read_timebase_gc(sys: *mut core::ffi::c_void, tbr: u32) -> u32 {
    read_timebase::<GC>(sys.cast(), tbr)
}
pub extern "C" fn read_timebase_wii(sys: *mut core::ffi::c_void, tbr: u32) -> u32 {
    read_timebase::<WII>(sys.cast(), tbr)
}

#[inline(always)]
fn do_rfi<const SYSTEM: SystemId>(sys: *mut System<SYSTEM>) {
    let sys = unsafe { &mut *sys };
    const RFI_MSR_MASK: u32 = 0x87C0_FFFF;
    let new_msr = (sys.gekko.msr.raw() & !RFI_MSR_MASK) | (sys.gekko.spr.srr1 & RFI_MSR_MASK);
    sys.gekko.msr = Msr::from(new_msr & !0x0004_0000);
    sys.gekko.nia = sys.gekko.spr.srr0.value() << 2;
}
pub extern "C" fn do_rfi_gc(sys: *mut core::ffi::c_void) {
    do_rfi::<GC>(sys.cast());
}
pub extern "C" fn do_rfi_wii(sys: *mut core::ffi::c_void) {
    do_rfi::<WII>(sys.cast());
}
