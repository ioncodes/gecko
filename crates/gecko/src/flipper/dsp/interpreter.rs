use crate::flipper::dsp;
use crate::flipper::dsp::condition::BranchControl;
use crate::flipper::dsp::core::regs::{SignExtensionMode, StatusRegister};
use crate::flipper::dsp::core::{Registers, reg};
use crate::flipper::dsp::instruction::{GcDspExt, Instruction};
use crate::flipper::dsp::lut::*;

#[cold]
#[inline(never)]
pub fn invalid(_ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    panic!("unimplemented DSP opcode {:#06x}", instr.0);
}

#[cold]
#[inline(never)]
pub fn invalid_ext(_ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    panic!("unimplemented DSP ext opcode {:#04x}", instr.0);
}

#[inline(always)]
fn multiply(regs: &mut Registers, a: i16, b: i16) {
    let mut result = a as i32 as i64 * b as i32 as i64;
    if !regs.status.am() {
        result <<= 1;
    }
    regs.write_product(result);
}

/// Compute a * b (with AM shift), then add/sub to current product.
#[inline(always)]
fn multiply_accumulate<const ADD: bool>(regs: &mut Registers, a: i16, b: i16) {
    let mut mul_result = a as i32 as i64 * b as i32 as i64;
    if !regs.status.am() {
        mul_result <<= 1;
    }
    let prod = regs.product();
    let result = if ADD {
        prod.wrapping_add(mul_result)
    } else {
        prod.wrapping_sub(mul_result)
    };
    regs.write_product(result);
}

/// Apply product carry/overflow to status: set O and OS from product flags.
#[inline(always)]
fn apply_product_oc(regs: &mut Registers, carry: bool, overflow: bool) {
    regs.status.set_o(overflow);
    if overflow {
        regs.status.set_os(true);
    }
    regs.status.set_c(carry);
}

/// Round a 40-bit value to the nearest 0x10000 boundary.
/// Returns (rounded_value, carry_out).
#[inline(always)]
fn round_half_to_even(val: i64) -> (i64, bool) {
    let val40 = val as u64 & 0xFF_FFFF_FFFF;
    let lower = (val40 & 0xFFFF) as u16;
    let round_up = lower > 0x8000 || (lower == 0x8000 && val40 & 0x10000 != 0);
    if round_up {
        let sum = val40.wrapping_add(0x10000);
        ((sum & !0xFFFFu64) as i64, sum > 0xFF_FFFF_FFFF)
    } else {
        ((val40 & !0xFFFFu64) as i64, false)
    }
}

#[inline(always)]
fn round_addpaxz_product(val: i64, addend: i64) -> (i64, bool) {
    let val40 = val as u64 & 0xFF_FFFF_FFFF;
    let sum40 = val40.wrapping_add(addend as u64) & 0xFF_FFFF_FFFF;
    let lower = (val40 & 0xFFFF) as u16;
    let round_up = lower > 0x8000 || (lower == 0x8000 && sum40 & 0x10000 != 0);
    if round_up {
        let sum = val40.wrapping_add(0x10000);
        ((sum & !0xFFFFu64) as i64, sum > 0xFF_FFFF_FFFF)
    } else {
        ((val40 & !0xFFFFu64) as i64, false)
    }
}

/// Apply combined arithmetic + product carry/overflow for ADD+product operations.
/// Preserves OS from before the arithmetic flag update, then sets based on the XORed values.
#[inline(always)]
fn apply_combined_add_product_oc(regs: &mut Registers, pc: bool, po: bool, os_before: bool) {
    regs.status.set_os(os_before);
    let c = regs.status.c();
    let o = regs.status.o();
    regs.status.set_c(c ^ pc);
    regs.status.set_o(o ^ po);
    if regs.status.o() {
        regs.status.set_os(true);
    }
}

#[inline(always)]
fn move_prod_to_ac(ctx: &mut crate::gamecube::GameCube, r: u8) {
    let prod = ctx.dsp.registers.product();
    let (carry, overflow) = ctx.dsp.registers.product_flags();
    ctx.dsp.registers.set_ac(r, prod);
    ctx.dsp.registers.update_flags_ac(prod);
    apply_product_oc(&mut ctx.dsp.registers, carry, overflow);
}

#[inline(always)]
fn move_prod_to_ac_zero(ctx: &mut crate::gamecube::GameCube, r: u8) {
    let (carry, overflow) = ctx.dsp.registers.product_flags();
    let raw = ctx.dsp.registers.product();
    let (prod, rounding_carry) = round_half_to_even(raw);
    ctx.dsp.registers.set_ac(r, prod);
    ctx.dsp.registers.update_flags_ac(prod);
    apply_product_oc(&mut ctx.dsp.registers, carry || rounding_carry, overflow);
}

#[inline(always)]
fn add_prod_to_ac(ctx: &mut crate::gamecube::GameCube, r: u8) {
    let a = ctx.dsp.registers.ac(r);
    let (pc, po) = ctx.dsp.registers.product_flags();
    let b = ctx.dsp.registers.product();
    let result = a.wrapping_add(b);
    let os_before = ctx.dsp.registers.status.os();
    ctx.dsp.registers.set_ac(r, result);
    ctx.dsp.registers.update_flags_add(a, b, result);
    apply_combined_add_product_oc(&mut ctx.dsp.registers, pc, po, os_before);
}

/// LOGICAL controls whether right shifts are logical.
/// REVERSED controls direction: when false (ASRN/LSRN), bit6=LEFT/!bit6=RIGHT.
/// When true (ASRNR/NRX variants), bit6=RIGHT/!bit6=LEFT.
#[inline(always)]
fn dynamic_shift<const LOGICAL: bool, const REVERSED: bool>(regs: &mut Registers, d: u8, shift_val: i16) {
    let low6 = (shift_val & 63) as u32;
    let bit6 = shift_val & 64 != 0;
    let amount = if bit6 { (64 - low6) % 64 } else { low6 };
    let shift_left = if REVERSED { !bit6 } else { bit6 };
    if shift_left {
        let ac = ((regs.ac(d) as u64 & 0xFF_FFFF_FFFF) << amount) as i64;
        regs.set_ac(d, ac);
    } else if amount != 0 {
        let ac = if LOGICAL {
            ((regs.ac(d) as u64 & 0xFF_FFFF_FFFF) >> amount) as i64
        } else {
            regs.ac(d) >> amount
        };
        regs.set_ac(d, ac);
    }
    let ac = regs.ac(d);
    regs.update_flags_ac(ac);
    regs.status.set_o(false);
    regs.status.set_c(false);
}

/// Get the ax0/ax1 operands for MULX instructions.
#[inline(always)]
fn mulx_operands(regs: &Registers, s: u8, t: u8) -> (u16, u16) {
    let a = if s != 0 { regs.axh[0] } else { regs.ax[0] };
    let b = if t != 0 { regs.axh[1] } else { regs.ax[1] };
    (a, b)
}

/// Multiply for MULX family: handles unsigned/mixed modes based on SU flag and s/t operands.
#[inline(always)]
fn multiply_mulx(regs: &mut Registers, s: u8, t: u8) {
    let (a_raw, b_raw) = mulx_operands(regs, s, t);
    let unsigned = regs.status.su();
    let (a, b) = if !unsigned {
        // Signed mode: both sign-extended
        (a_raw as i16 as i64, b_raw as i16 as i64)
    } else {
        match (s != 0, t != 0) {
            (false, false) => (a_raw as u64 as i64, b_raw as u64 as i64),
            (false, true) => (a_raw as u64 as i64, b_raw as i16 as i64),
            (true, false) => (b_raw as u64 as i64, a_raw as i16 as i64),
            (true, true) => (a_raw as i16 as i64, b_raw as i16 as i64),
        }
    };
    let mut result = a * b;
    if !regs.status.am() {
        result <<= 1;
    }
    regs.write_product(result);
}

/// Get the acS.m / axT.h operands for MULC instructions.
#[inline(always)]
fn mulc_operands(regs: &Registers, s: u8, t: u8) -> (i16, i16) {
    let a = regs.ac_mid(s) as i16;
    let b = regs.axh[t as usize] as i16;
    (a, b)
}

#[inline(always)]
pub fn add_sub<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    match OP {
        OP_ADDR => {
            let ss = instr.ss() as usize;
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let b = (ctx.dsp.registers.read::<true>(reg::AX0L + ss as u8) as i16 as i64) << 16;
            let result = a.wrapping_add(b);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_add(a, b, result);
        }
        OP_ADDAX => {
            let s = instr.s_6_6() as usize;
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let b = ((ctx.dsp.registers.axh[s] as i16 as i64) << 16) | (ctx.dsp.registers.ax[s] as i64);
            let result = a.wrapping_add(b);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_add(a, b, result);
        }
        OP_ADD => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let b = ctx.dsp.registers.ac(1 - d);
            let result = a.wrapping_add(b);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_add(a, b, result);
        }
        OP_ADDP => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let (pc, po) = ctx.dsp.registers.product_flags();
            let b = ctx.dsp.registers.product();
            let result = a.wrapping_add(b);
            let os_before = ctx.dsp.registers.status.os();
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_add(a, b, result);
            apply_combined_add_product_oc(&mut ctx.dsp.registers, pc, po, os_before);
        }
        OP_SUBR => {
            let ss = instr.ss();
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let b = (ctx.dsp.registers.read::<true>(reg::AX0L + ss) as i16 as i64) << 16;
            let result = a.wrapping_sub(b);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_sub(a, b, result);
        }
        OP_SUBAX => {
            let s = instr.s_6_6() as usize;
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let b = ((ctx.dsp.registers.axh[s] as i16 as i64) << 16) | (ctx.dsp.registers.ax[s] as i64);
            let result = a.wrapping_sub(b);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_sub(a, b, result);
        }
        OP_SUB => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let b = ctx.dsp.registers.ac(1 - d);
            let result = a.wrapping_sub(b);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_sub(a, b, result);
        }
        OP_SUBP => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let (pc, po) = ctx.dsp.registers.product_flags();
            let b = ctx.dsp.registers.product();
            let result = a.wrapping_sub(b);
            let os_before = ctx.dsp.registers.status.os();
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_sub(a, b, result);
            // SUBP: carry = sub_carry XOR !product_carry
            ctx.dsp.registers.status.set_os(os_before);
            let c = ctx.dsp.registers.status.c();
            let o = ctx.dsp.registers.status.o();
            ctx.dsp.registers.status.set_c(c ^ !pc);
            ctx.dsp.registers.status.set_o(o ^ po);
            if ctx.dsp.registers.status.o() {
                ctx.dsp.registers.status.set_os(true);
            }
        }
        OP_ADDAXL => {
            let s = instr.s_6_6() as usize;
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let b = ctx.dsp.registers.ax[s] as i64;
            let result = a.wrapping_add(b);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_add(a, b, result);
        }
        OP_ADDPAXZ => {
            let s = instr.s_6_6() as usize;
            let d = instr.d_7_7();
            let (pc, po) = ctx.dsp.registers.product_flags();
            let a = ctx.dsp.registers.product();
            let b = (ctx.dsp.registers.axh[s] as i16 as i64) << 16;
            let (rounded, rounding_carry) = self::round_addpaxz_product(a, b);
            let result = rounded.wrapping_add(b) & !0xFFFFi64;
            let os_before = ctx.dsp.registers.status.os();
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_add(rounded, b, result);
            apply_combined_add_product_oc(&mut ctx.dsp.registers, pc || rounding_carry, po, os_before);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn addr_reg<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    match OP {
        OP_DAR => {
            let d = instr.d_14_15() as usize;
            ctx.dsp.registers.ar[d] = ctx.dsp.registers.decrement_ar(d);
        }
        OP_IAR => {
            let d = instr.d_14_15() as usize;
            ctx.dsp.registers.ar[d] = ctx.dsp.registers.increment_ar(d);
        }
        OP_SUBARN => {
            let d = instr.d_14_15() as usize;
            let ix = ctx.dsp.registers.ix[d] as i16;
            ctx.dsp.registers.ar[d] = ctx.dsp.registers.decrease_ar_ix(d, ix);
        }
        OP_ADDARN => {
            let s = instr.s_12_13() as usize;
            let d = instr.d_14_15() as usize;
            let ix = ctx.dsp.registers.ix[s] as i16;
            ctx.dsp.registers.ar[d] = ctx.dsp.registers.increase_ar(d, ix);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn cmp_test<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    match OP {
        OP_CMP => {
            let a = ctx.dsp.registers.ac(0);
            let b = ctx.dsp.registers.ac(1);
            let result = a.wrapping_sub(b);
            ctx.dsp.registers.update_flags_sub(a, b, result);
        }
        OP_CMPAXH => {
            let s = instr.s_4_4();
            let r = instr.s_3_3();
            let a = ctx.dsp.registers.ac(s);
            let b = (ctx.dsp.registers.axh[r as usize] as i16 as i64) << 16;
            let result = a.wrapping_sub(b);
            ctx.dsp.registers.update_flags_sub(a, b, result);
        }
        OP_TST => {
            let r = instr.r_4_4();
            let ac = ctx.dsp.registers.ac(r);
            ctx.dsp.registers.update_flags_ac(ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_TSTPROD => {
            let (pc, po) = ctx.dsp.registers.product_flags();
            let prod = ctx.dsp.registers.product();
            ctx.dsp.registers.update_flags_ac(prod);
            ctx.dsp.registers.status.set_o(po);
            if po {
                ctx.dsp.registers.status.set_os(true);
            }
            ctx.dsp.registers.status.set_c(pc);
        }
        OP_TSTAXH => {
            let r = instr.r_7_7() as usize;
            let val = (ctx.dsp.registers.axh[r] as i16 as i64) << 16;
            ctx.dsp.registers.update_flags_ac(val);
            ctx.dsp.registers.status.set_as32(false);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_NX_0 | OP_NX_1 => {}
        OP_CLR => {
            let r = instr.r_4_4();
            ctx.dsp.registers.set_ac(r, 0);
            ctx.dsp.registers.status.set_tb(true);
            ctx.dsp.registers.status.set_as32(false);
            ctx.dsp.registers.status.set_s(false);
            ctx.dsp.registers.status.set_z(true);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_CLRP => {
            ctx.dsp.registers.product_low = 0x0000;
            ctx.dsp.registers.product_mid1 = 0xFFF0;
            ctx.dsp.registers.product_high = 0x00FF;
            ctx.dsp.registers.product_mid2 = 0x0010;
        }
        OP_CLRL => {
            let r = instr.r_7_7();
            let ac = ctx.dsp.registers.ac(r);
            let (rounded, carry) = round_half_to_even(ac);
            ctx.dsp.registers.set_ac(r, rounded);
            ctx.dsp.registers.update_flags_ac(rounded);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(carry);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn control<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    match OP {
        OP_JCC => {
            let branch_control = BranchControl::from(instr.cond());
            if branch_control.evaluate(&ctx.dsp) {
                ctx.dsp.registers.nia = instr.addr();
            }
        }
        OP_NOP => {}
        OP_HALT => {
            ctx.dsp.csr.set_halt(true);
        }
        OP_IFCC => {
            let branch_control = BranchControl::from(instr.cond());
            if !branch_control.evaluate(&ctx.dsp) {
                ctx.dsp.registers.nia = ctx.dsp.registers.nia.wrapping_add(1);
            }
        }
        OP_CALLCC => {
            let branch_control = BranchControl::from(instr.cond());
            if branch_control.evaluate(&ctx.dsp) {
                ctx.dsp.registers.call_stack.push(ctx.dsp.registers.nia);
                ctx.dsp.registers.nia = instr.addr();
            }
        }
        OP_RETCC => {
            let branch_control = BranchControl::from(instr.cond());
            if branch_control.evaluate(&ctx.dsp) {
                ctx.dsp.registers.nia = ctx.dsp.registers.call_stack.pop();
            }
        }
        OP_RTICC => {
            let branch_control = BranchControl::from(instr.cond());
            if branch_control.evaluate(&ctx.dsp) {
                ctx.dsp.registers.status = StatusRegister::from(ctx.dsp.registers.data_stack.pop());
                ctx.dsp.registers.nia = ctx.dsp.registers.call_stack.pop();
            }
        }
        OP_JRCC => {
            let branch_control = BranchControl::from(instr.cond());
            if branch_control.evaluate(&ctx.dsp) {
                ctx.dsp.registers.nia = ctx.dsp.registers.read::<true>(instr.reg_8_10());
            }
        }
        OP_CALLRCC => {
            let branch_control = BranchControl::from(instr.cond());
            if branch_control.evaluate(&ctx.dsp) {
                ctx.dsp.registers.call_stack.push(ctx.dsp.registers.nia);
                ctx.dsp.registers.nia = ctx.dsp.registers.read::<true>(instr.reg_8_10());
            }
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn imm_alu<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    match OP {
        OP_ADDI => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let b = (instr.imm_16_31() as i16 as i64) << 16;
            let result = a.wrapping_add(b);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_add(a, b, result);
        }
        OP_XORI => {
            let d = instr.d_7_7();
            let result = ctx.dsp.registers.ac_mid(d) ^ instr.imm_16_31();
            ctx.dsp.registers.write::<false>(reg::AC0M + d, result);
            let ac = ctx.dsp.registers.ac(d);
            ctx.dsp.registers.update_flags_logic(result, ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_ANDI => {
            let d = instr.d_7_7();
            let result = ctx.dsp.registers.ac_mid(d) & instr.imm_16_31();
            ctx.dsp.registers.write::<false>(reg::AC0M + d, result);
            let ac = ctx.dsp.registers.ac(d);
            ctx.dsp.registers.update_flags_logic(result, ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_ORI => {
            let d = instr.d_7_7();
            let result = ctx.dsp.registers.ac_mid(d) | instr.imm_16_31();
            ctx.dsp.registers.write::<false>(reg::AC0M + d, result);
            let ac = ctx.dsp.registers.ac(d);
            ctx.dsp.registers.update_flags_logic(result, ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_CMPI => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let b = (instr.imm_16_31() as i16 as i64) << 16;
            let result = a.wrapping_sub(b);
            ctx.dsp.registers.update_flags_sub(a, b, result);
        }
        OP_ANDF | OP_ANDCF => {
            let imm = instr.imm_16_31();
            let result = ctx.dsp.registers.ac_mid(instr.d_7_7()) & imm;
            let lz = match OP {
                OP_ANDF => result == 0,
                OP_ANDCF => result == imm,
                _ => unreachable!(),
            };
            ctx.dsp.registers.status.set_lz(lz);
        }
        OP_ADDIS => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let b = (instr.imm_8_15_i16() as i64) << 16;
            let result = a.wrapping_add(b);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_add(a, b, result);
        }
        OP_CMPIS => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let b = (instr.imm_8_15_i16() as i64) << 16;
            let result = a.wrapping_sub(b);
            ctx.dsp.registers.update_flags_sub(a, b, result);
        }
        OP_LRIS => {
            let dst = reg::AX0L + instr.reg_5_7();
            let imm = instr.imm_8_15_i16() as u16;
            ctx.dsp.registers.write::<true>(dst, imm);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn inc_dec<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    match OP {
        OP_INCM => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let result = a.wrapping_add(0x10000);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_add(a, 0x10000, result);
        }
        OP_INC => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let result = a.wrapping_add(1);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_add(a, 1, result);
        }
        OP_DECM => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let result = a.wrapping_sub(0x10000);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_sub(a, 0x10000, result);
        }
        OP_DEC => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let result = a.wrapping_sub(1);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_sub(a, 1, result);
        }
        OP_NEG => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let result = 0i64.wrapping_sub(a);
            ctx.dsp.registers.set_ac(d, result);
            ctx.dsp.registers.update_flags_sub(0, a, result);
        }
        OP_ABS_AC => {
            let d = instr.d_4_4();
            let a = ctx.dsp.registers.ac(d);
            let was_negative = a < 0;
            if was_negative {
                ctx.dsp.registers.set_ac(d, a.wrapping_neg());
            }
            let ac = ctx.dsp.registers.ac(d);
            ctx.dsp.registers.update_flags_ac(ac);
            // Overflow if the value was negative and is still negative
            ctx.dsp.registers.status.set_o(was_negative && ac < 0);
            if ctx.dsp.registers.status.o() {
                ctx.dsp.registers.status.set_os(true);
            }
            ctx.dsp.registers.status.set_c(false);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn load_store<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    match OP {
        OP_LRI => {
            ctx.dsp.registers.write::<true>(instr.d_11_15(), instr.imm_16_31());
        }
        OP_LR => {
            let value = dsp::read_dmem(ctx, instr.imm_16_31());
            ctx.dsp.registers.write::<true>(instr.d_11_15(), value);
        }
        OP_SR => {
            let value = ctx.dsp.registers.read::<true>(instr.d_11_15());
            dsp::write_dmem(ctx, instr.imm_16_31(), value);
        }
        OP_MRR => {
            let value = ctx.dsp.registers.read::<true>(instr.src());
            ctx.dsp.registers.write::<true>(instr.dst(), value);
        }
        OP_SI => {
            let addr = 0xFF00 | (instr.mem_8_15_u16());
            dsp::write_dmem(ctx, addr, instr.imm_16_31());
        }
        OP_LRR | OP_LRRD | OP_LRRI | OP_LRRN => {
            let s = instr.s_9_10() as usize;
            let d = instr.d_11_15();
            let addr = ctx.dsp.registers.ar[s];
            let value = dsp::read_dmem(ctx, addr);
            // Compute new AR before the write (write may modify WR/IX used by AR increment)
            let new_ar = match OP {
                OP_LRRD => Some(ctx.dsp.registers.decrement_ar(s)),
                OP_LRRI => Some(ctx.dsp.registers.increment_ar(s)),
                OP_LRRN => {
                    let ix = ctx.dsp.registers.ix[s] as i16;
                    Some(ctx.dsp.registers.increase_ar(s, ix))
                }
                _ => None,
            };
            ctx.dsp.registers.write::<true>(d, value);
            // Apply AR update only if destination register doesn't alias the AR
            if let Some(ar) = new_ar {
                if d as usize != s {
                    ctx.dsp.registers.ar[s] = ar;
                }
            }
        }
        OP_SRR | OP_SRRD | OP_SRRI | OP_SRRN => {
            let d = instr.s_9_10() as usize;
            let value = ctx.dsp.registers.read::<true>(instr.d_11_15());
            // Compute new AR before the DMEM write
            let new_ar = match OP {
                OP_SRRD => Some(ctx.dsp.registers.decrement_ar(d)),
                OP_SRRI => Some(ctx.dsp.registers.increment_ar(d)),
                OP_SRRN => {
                    let ix = ctx.dsp.registers.ix[d] as i16;
                    Some(ctx.dsp.registers.increase_ar(d, ix))
                }
                _ => None,
            };
            let addr = ctx.dsp.registers.ar[d];
            dsp::write_dmem(ctx, addr, value);
            if let Some(ar) = new_ar {
                ctx.dsp.registers.ar[d] = ar;
            }
        }
        OP_LRS => {
            let dst = reg::AX0L + instr.reg_5_7();
            let addr = (ctx.dsp.registers.config << 8) | instr.mem_8_15_u16();
            let value = dsp::read_dmem(ctx, addr);
            ctx.dsp.registers.write::<true>(dst, value);
        }
        OP_SRSH | OP_SRS => {
            let addr = (ctx.dsp.registers.config << 8) | instr.mem_8_15_u16();
            let src = match OP {
                OP_SRSH => {
                    if instr.s_7_7() != 0 {
                        reg::AC1H
                    } else {
                        reg::AC0H
                    }
                }
                OP_SRS => reg::AC0L + instr.reg_6_7(),
                _ => unreachable!(),
            };
            let value = ctx.dsp.registers.read::<true>(src);
            dsp::write_dmem(ctx, addr, value);
        }
        OP_ILRR | OP_ILRRD | OP_ILRRI | OP_ILRRN => {
            let src = instr.s_14_15() as usize;
            let dst = if instr.d_7_7() != 0 { reg::AC1M } else { reg::AC0M };
            let value = ctx.dsp.read_imem(ctx.dsp.registers.ar[src]);
            ctx.dsp.registers.write::<true>(dst, value);
            match OP {
                OP_ILRRD => ctx.dsp.registers.ar[src] = ctx.dsp.registers.decrement_ar(src),
                OP_ILRRI => ctx.dsp.registers.ar[src] = ctx.dsp.registers.increment_ar(src),
                OP_ILRRN => {
                    let ix = ctx.dsp.registers.ix[src] as i16;
                    ctx.dsp.registers.ar[src] = ctx.dsp.registers.increase_ar(src, ix);
                }
                _ => {}
            }
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn logic<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    match OP {
        OP_XORR => {
            let s = instr.s_6_6() as usize;
            let d = instr.d_7_7();
            let result = ctx.dsp.registers.ac_mid(d) ^ ctx.dsp.registers.axh[s];
            ctx.dsp.registers.write::<false>(reg::AC0M + d, result);
            let ac = ctx.dsp.registers.ac(d);
            ctx.dsp.registers.update_flags_logic(result, ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_ANDR => {
            let s = instr.s_6_6() as usize;
            let d = instr.d_7_7();
            let result = ctx.dsp.registers.ac_mid(d) & ctx.dsp.registers.axh[s];
            ctx.dsp.registers.write::<false>(reg::AC0M + d, result);
            let ac = ctx.dsp.registers.ac(d);
            ctx.dsp.registers.update_flags_logic(result, ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_ORR => {
            let s = instr.s_6_6() as usize;
            let d = instr.d_7_7();
            let result = ctx.dsp.registers.ac_mid(d) | ctx.dsp.registers.axh[s];
            ctx.dsp.registers.write::<false>(reg::AC0M + d, result);
            let ac = ctx.dsp.registers.ac(d);
            ctx.dsp.registers.update_flags_logic(result, ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_ANDC => {
            let d = instr.d_7_7();
            let result = ctx.dsp.registers.ac_mid(d) & ctx.dsp.registers.ac_mid(1 - d);
            ctx.dsp.registers.write::<false>(reg::AC0M + d, result);
            let ac = ctx.dsp.registers.ac(d);
            ctx.dsp.registers.update_flags_logic(result, ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_ORC => {
            let d = instr.d_7_7();
            let result = ctx.dsp.registers.ac_mid(d) | ctx.dsp.registers.ac_mid(1 - d);
            ctx.dsp.registers.write::<false>(reg::AC0M + d, result);
            let ac = ctx.dsp.registers.ac(d);
            ctx.dsp.registers.update_flags_logic(result, ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_XORC => {
            let d = instr.d_7_7();
            let result = ctx.dsp.registers.ac_mid(d) ^ ctx.dsp.registers.ac_mid(1 - d);
            ctx.dsp.registers.write::<false>(reg::AC0M + d, result);
            let ac = ctx.dsp.registers.ac(d);
            ctx.dsp.registers.update_flags_logic(result, ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_NOT_AC => {
            let d = instr.d_7_7();
            let result = !ctx.dsp.registers.ac_mid(d);
            ctx.dsp.registers.write::<false>(reg::AC0M + d, result);
            let ac = ctx.dsp.registers.ac(d);
            ctx.dsp.registers.update_flags_logic(result, ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn loops<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    match OP {
        OP_LOOP_REG | OP_LOOPI => {
            let counter = match OP {
                OP_LOOP_REG => ctx.dsp.registers.read::<true>(instr.reg_11_15()),
                OP_LOOPI => instr.imm_8_15_u8() as u16,
                _ => unreachable!(),
            };
            let end_addr = ctx.dsp.registers.nia;
            if counter != 0 {
                ctx.dsp.registers.call_stack.push(end_addr);
                ctx.dsp.registers.loop_addr.push(end_addr.wrapping_add(1));
                ctx.dsp.registers.loop_counter.push(counter);
            } else {
                ctx.dsp.registers.nia = end_addr.wrapping_add(1);
            }
        }
        OP_BLOOP | OP_BLOOPI => {
            let counter = match OP {
                OP_BLOOP => ctx.dsp.registers.read::<true>(instr.reg_11_15()),
                OP_BLOOPI => instr.imm_8_15_u8() as u16,
                _ => unreachable!(),
            };
            let end_addr = instr.addr();
            if counter != 0 {
                ctx.dsp.registers.call_stack.push(ctx.dsp.registers.nia);
                ctx.dsp.registers.loop_addr.push(end_addr.wrapping_add(1));
                ctx.dsp.registers.loop_counter.push(counter);
            } else {
                ctx.dsp.registers.nia = end_addr.wrapping_add(1);
            }
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn move_ops<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    match OP {
        OP_MOVR => {
            let ss = instr.ss();
            let d = instr.d_7_7();
            let val = (ctx.dsp.registers.read::<true>(reg::AX0L + ss) as i16 as i64) << 16;
            ctx.dsp.registers.set_ac(d, val);
            ctx.dsp.registers.update_flags_ac(val);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_MOVAX => {
            let s = instr.s_6_6() as usize;
            let d = instr.d_7_7();
            let val = ((ctx.dsp.registers.axh[s] as i16 as i64) << 16) | (ctx.dsp.registers.ax[s] as i64);
            ctx.dsp.registers.set_ac(d, val);
            ctx.dsp.registers.update_flags_ac(val);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_MOV => {
            let d = instr.d_7_7();
            let val = ctx.dsp.registers.ac(1 - d);
            ctx.dsp.registers.set_ac(d, val);
            ctx.dsp.registers.update_flags_ac(val);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_MOVP => {
            let d = instr.d_7_7();
            move_prod_to_ac(ctx, d);
        }
        OP_MOVPZ => {
            let d = instr.d_7_7();
            move_prod_to_ac_zero(ctx, d);
        }
        OP_MOVNP => {
            let d = instr.d_7_7();
            let prod = ctx.dsp.registers.product();
            let (carry, overflow) = ctx.dsp.registers.product_flags();
            let val = prod.wrapping_neg();
            ctx.dsp.registers.set_ac(d, val);
            ctx.dsp.registers.update_flags_ac(val);
            ctx.dsp.registers.status.set_o(overflow);
            if overflow {
                ctx.dsp.registers.status.set_os(true);
            }
            let prod_zero = prod as u64 & 0xFF_FFFF_FFFF == 0;
            ctx.dsp.registers.status.set_c(!carry ^ prod_zero);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn mul<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    match OP {
        OP_MUL => {
            let s = instr.r_4_4() as usize;
            let (a, b) = (ctx.dsp.registers.ax[s] as i16, ctx.dsp.registers.axh[s] as i16);
            multiply(&mut ctx.dsp.registers, a, b);
        }
        OP_MULX => {
            multiply_mulx(&mut ctx.dsp.registers, instr.s_3_3(), instr.t_4_4());
        }
        OP_MULC => {
            let (a, b) = mulc_operands(&ctx.dsp.registers, instr.s_3_3(), instr.t_4_4());
            multiply(&mut ctx.dsp.registers, a, b);
        }
        OP_MULAXH => {
            let val = ctx.dsp.registers.axh[0] as i16;
            multiply(&mut ctx.dsp.registers, val, val);
        }
        OP_MULMV => {
            let s = instr.r_4_4() as usize;
            let (a, b) = (ctx.dsp.registers.ax[s] as i16, ctx.dsp.registers.axh[s] as i16);
            move_prod_to_ac(ctx, instr.r_7_7());
            multiply(&mut ctx.dsp.registers, a, b);
        }
        OP_MULMVZ => {
            let s = instr.r_4_4() as usize;
            let (a, b) = (ctx.dsp.registers.ax[s] as i16, ctx.dsp.registers.axh[s] as i16);
            move_prod_to_ac_zero(ctx, instr.r_7_7());
            multiply(&mut ctx.dsp.registers, a, b);
        }
        OP_MULAC => {
            let s = instr.r_4_4() as usize;
            let (a, b) = (ctx.dsp.registers.ax[s] as i16, ctx.dsp.registers.axh[s] as i16);
            add_prod_to_ac(ctx, instr.r_7_7());
            multiply(&mut ctx.dsp.registers, a, b);
        }
        OP_MULXMV => {
            let (s, t) = (instr.s_3_3(), instr.t_4_4());
            move_prod_to_ac(ctx, instr.r_7_7());
            multiply_mulx(&mut ctx.dsp.registers, s, t);
        }
        OP_MULXMVZ => {
            let (s, t) = (instr.s_3_3(), instr.t_4_4());
            move_prod_to_ac_zero(ctx, instr.r_7_7());
            multiply_mulx(&mut ctx.dsp.registers, s, t);
        }
        OP_MULXAC => {
            let (s, t) = (instr.s_3_3(), instr.t_4_4());
            add_prod_to_ac(ctx, instr.r_7_7());
            multiply_mulx(&mut ctx.dsp.registers, s, t);
        }
        OP_MULCMV => {
            let (a, b) = mulc_operands(&ctx.dsp.registers, instr.s_3_3(), instr.t_4_4());
            move_prod_to_ac(ctx, instr.r_7_7());
            multiply(&mut ctx.dsp.registers, a, b);
        }
        OP_MULCMVZ => {
            let (a, b) = mulc_operands(&ctx.dsp.registers, instr.s_3_3(), instr.t_4_4());
            move_prod_to_ac_zero(ctx, instr.r_7_7());
            multiply(&mut ctx.dsp.registers, a, b);
        }
        OP_MULCAC => {
            let (a, b) = mulc_operands(&ctx.dsp.registers, instr.s_3_3(), instr.t_4_4());
            add_prod_to_ac(ctx, instr.r_7_7());
            multiply(&mut ctx.dsp.registers, a, b);
        }
        OP_MADD => {
            let s = instr.s_7_7() as usize;
            let (a, b) = (ctx.dsp.registers.ax[s] as i16, ctx.dsp.registers.axh[s] as i16);
            multiply_accumulate::<true>(&mut ctx.dsp.registers, a, b);
        }
        OP_MSUB => {
            let s = instr.s_7_7() as usize;
            let (a, b) = (ctx.dsp.registers.ax[s] as i16, ctx.dsp.registers.axh[s] as i16);
            multiply_accumulate::<false>(&mut ctx.dsp.registers, a, b);
        }
        OP_MADDX | OP_MSUBX => {
            let (a, b) = mulx_operands(&ctx.dsp.registers, instr.s_6_6(), instr.t_7_7());
            if matches!(OP, OP_MADDX) {
                multiply_accumulate::<true>(&mut ctx.dsp.registers, a as i16, b as i16);
            } else {
                multiply_accumulate::<false>(&mut ctx.dsp.registers, a as i16, b as i16);
            }
        }
        OP_MADDC | OP_MSUBC => {
            let (a, b) = mulc_operands(&ctx.dsp.registers, instr.s_6_6(), instr.t_7_7());
            if matches!(OP, OP_MADDC) {
                multiply_accumulate::<true>(&mut ctx.dsp.registers, a, b);
            } else {
                multiply_accumulate::<false>(&mut ctx.dsp.registers, a, b);
            }
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn shifts<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    match OP {
        OP_LSL | OP_ASL => {
            let r = instr.r_7_7();
            let i = instr.n() as u32;
            let ac = ((ctx.dsp.registers.ac(r) as u64 & 0xFF_FFFF_FFFF) << i) as i64;
            ctx.dsp.registers.set_ac(r, ac);
            let ac = ctx.dsp.registers.ac(r);
            ctx.dsp.registers.update_flags_ac(ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_LSR => {
            let r = instr.r_7_7();
            let i = instr.n() as u32;
            if i != 0 {
                let ac = (ctx.dsp.registers.ac(r) as u64 & 0xFF_FFFF_FFFF) >> (64 - i);
                ctx.dsp.registers.set_ac(r, ac as i64);
            }
            let ac = ctx.dsp.registers.ac(r);
            ctx.dsp.registers.update_flags_ac(ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_ASR => {
            let r = instr.r_7_7();
            let i = instr.n() as u32;
            if i != 0 {
                let ac = ctx.dsp.registers.ac(r) >> (64 - i);
                ctx.dsp.registers.set_ac(r, ac);
            }
            let ac = ctx.dsp.registers.ac(r);
            ctx.dsp.registers.update_flags_ac(ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_LSL16 => {
            let r = instr.r_7_7();
            let ac = ((ctx.dsp.registers.ac(r) as u64 & 0xFF_FFFF_FFFF) << 16) as i64;
            ctx.dsp.registers.set_ac(r, ac);
            let ac = ctx.dsp.registers.ac(r);
            ctx.dsp.registers.update_flags_ac(ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_LSR16 => {
            let r = instr.r_7_7();
            let ac = (ctx.dsp.registers.ac(r) as u64 & 0xFF_FFFF_FFFF) >> 16;
            ctx.dsp.registers.set_ac(r, ac as i64);
            let ac = ctx.dsp.registers.ac(r);
            ctx.dsp.registers.update_flags_ac(ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_ASR16 => {
            let r = instr.r_4_4();
            let ac = ctx.dsp.registers.ac(r) >> 16;
            ctx.dsp.registers.set_ac(r, ac);
            let ac = ctx.dsp.registers.ac(r);
            ctx.dsp.registers.update_flags_ac(ac);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        OP_LSRN => {
            let sv = ctx.dsp.registers.ac1_mid as i16;
            dynamic_shift::<true, false>(&mut ctx.dsp.registers, 0, sv);
        }
        OP_ASRN => {
            let sv = ctx.dsp.registers.ac1_mid as i16;
            dynamic_shift::<false, false>(&mut ctx.dsp.registers, 0, sv);
        }
        OP_LSRNRX => {
            let s = instr.s_6_6() as usize;
            let d = instr.d_7_7();
            let sv = ctx.dsp.registers.axh[s] as i16;
            dynamic_shift::<true, true>(&mut ctx.dsp.registers, d, sv);
        }
        OP_ASRNRX => {
            let s = instr.s_6_6() as usize;
            let d = instr.d_7_7();
            let sv = ctx.dsp.registers.axh[s] as i16;
            dynamic_shift::<false, true>(&mut ctx.dsp.registers, d, sv);
        }
        OP_LSRNR => {
            let d = instr.d_7_7();
            let sv = ctx.dsp.registers.ac_mid(1 - d) as i16;
            dynamic_shift::<true, true>(&mut ctx.dsp.registers, d, sv);
        }
        OP_ASRNR => {
            let d = instr.d_7_7();
            let sv = ctx.dsp.registers.ac_mid(1 - d) as i16;
            dynamic_shift::<false, true>(&mut ctx.dsp.registers, d, sv);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn status<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: Instruction) {
    match OP {
        OP_SBCLR => {
            let idx = 6 + instr.bit();
            if idx != 13 {
                ctx.dsp.registers.status &= !(1u16 << idx);
            }
        }
        OP_SBSET => {
            let idx = 6 + instr.bit();
            if idx != 13 && idx != 8 {
                ctx.dsp.registers.status |= 1u16 << idx;
            }
        }
        OP_M2 => {
            ctx.dsp.registers.status.set_am(false);
        }
        OP_M0 => {
            ctx.dsp.registers.status.set_am(true);
        }
        OP_CLR15 => {
            ctx.dsp.registers.status.set_su(false);
        }
        OP_SET15 => {
            ctx.dsp.registers.status.set_su(true);
        }
        OP_SET16 => {
            ctx.dsp.registers.status.set_sxm(SignExtensionMode::Bits16);
        }
        OP_SET40 => {
            ctx.dsp.registers.status.set_sxm(SignExtensionMode::Bits40);
        }
        _ => unreachable!(),
    }
}

// Extension opcode handlers
#[inline(always)]
pub fn ext_nop(_ctx: &mut crate::gamecube::GameCube, _instr: GcDspExt) {}

#[inline(always)]
pub fn ext_addr<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    let r = instr.r_6_7() as usize;
    match OP {
        OP_EXT_DR => ctx.dsp.registers.ar[r] = ctx.dsp.registers.decrement_ar(r),
        OP_EXT_IR => ctx.dsp.registers.ar[r] = ctx.dsp.registers.increment_ar(r),
        OP_EXT_NR => {
            let ix = ctx.dsp.registers.ix[r] as i16;
            ctx.dsp.registers.ar[r] = ctx.dsp.registers.increase_ar(r, ix);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn ext_mv(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    let d = instr.d_4_5();
    let s = instr.s_6_7();
    let value = ctx.dsp.registers.ext_ac_cache[s as usize];
    ctx.dsp.registers.write::<true>(reg::AX0L + d, value);
}

#[inline(always)]
pub fn ext_store<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    let d = instr.d_6_7() as usize;
    let s = instr.s_3_4();
    let value = ctx.dsp.registers.ext_ac_cache[s as usize];
    let addr = ctx.dsp.registers.ar[d];
    dsp::write_dmem(ctx, addr, value);
    match OP {
        OP_EXT_S => {
            ctx.dsp.registers.ar[d] = ctx.dsp.registers.increment_ar(d);
        }
        OP_EXT_SN => {
            let ix = ctx.dsp.registers.ix[d] as i16;
            ctx.dsp.registers.ar[d] = ctx.dsp.registers.increase_ar(d, ix);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn ext_load<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    let d = instr.d_2_4();
    let s = instr.s_6_7() as usize;
    let addr = ctx.dsp.registers.ar[s];
    let value = dsp::read_dmem(ctx, addr);
    ctx.dsp.registers.write::<true>(reg::AX0L + d, value);
    match OP {
        OP_EXT_L => {
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.increment_ar(s);
        }
        OP_EXT_LN => {
            let ix = ctx.dsp.registers.ix[s] as i16;
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.increase_ar(s, ix);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn ext_load_store<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    let s = instr.s_7_7() as usize;
    let d = instr.d_2_3();

    // LS variants: Load from ar[0] to AX, Store ac[s].m to ar[3]
    // SL variants: Store ac[s].m to ar[0], Load from ar[3] to AX
    match OP {
        OP_EXT_LS | OP_EXT_LSM | OP_EXT_LSN | OP_EXT_LSNM => {
            let load_addr = ctx.dsp.registers.ar[0];
            let load_value = dsp::read_dmem(ctx, load_addr);
            ctx.dsp.registers.write::<true>(reg::AX0L + d, load_value);
            let store_value = ctx.dsp.registers.ext_ac_cache[4 + s];
            let store_addr = ctx.dsp.registers.ar[3];
            dsp::write_dmem(ctx, store_addr, store_value);
        }
        _ => {
            // SL: Store first, then Load
            let store_value = ctx.dsp.registers.ext_ac_cache[4 + s];
            let store_addr = ctx.dsp.registers.ar[0];
            dsp::write_dmem(ctx, store_addr, store_value);
            let load_addr = ctx.dsp.registers.ar[3];
            let load_value = dsp::read_dmem(ctx, load_addr);
            ctx.dsp.registers.write::<true>(reg::AX0L + d, load_value);
        }
    }

    // AR increments: same pattern for both LS and SL
    // "" = ar[0]+=1, ar[3]+=1  "N" = ar[0]+=ix[0], ar[3]+=1
    // "M" = ar[0]+=1, ar[3]+=ix[3]  "NM" = ar[0]+=ix[0], ar[3]+=ix[3]
    match OP {
        OP_EXT_LS | OP_EXT_SL => {
            ctx.dsp.registers.ar[0] = ctx.dsp.registers.increment_ar(0);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.increment_ar(3);
        }
        OP_EXT_LSM | OP_EXT_SLM => {
            ctx.dsp.registers.ar[0] = ctx.dsp.registers.increment_ar(0);
            let ix3 = ctx.dsp.registers.ix[3] as i16;
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.increase_ar(3, ix3);
        }
        OP_EXT_LSN | OP_EXT_SLN => {
            let ix0 = ctx.dsp.registers.ix[0] as i16;
            ctx.dsp.registers.ar[0] = ctx.dsp.registers.increase_ar(0, ix0);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.increment_ar(3);
        }
        OP_EXT_LSNM | OP_EXT_SLNM => {
            let ix0 = ctx.dsp.registers.ix[0] as i16;
            ctx.dsp.registers.ar[0] = ctx.dsp.registers.increase_ar(0, ix0);
            let ix3 = ctx.dsp.registers.ix[3] as i16;
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.increase_ar(3, ix3);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn ext_ld<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    let d = instr.d_2_2();
    let r = instr.r_3_3();
    let s = instr.s_6_7() as usize;

    let ar_s = ctx.dsp.registers.ar[s];
    let ar_3 = ctx.dsp.registers.ar[3];
    let same_bank = (ar_s >> 10) == (ar_3 >> 10);
    let d_reg = if d != 0 { reg::AX0H } else { reg::AX0L };
    let value0 = dsp::read_dmem(ctx, ar_s);
    ctx.dsp.registers.write::<true>(d_reg, value0);

    let r_reg = if r != 0 { reg::AX1H } else { reg::AX1L };
    let value1 = if same_bank { value0 } else { dsp::read_dmem(ctx, ar_3) };
    ctx.dsp.registers.write::<true>(r_reg, value1);

    // AR increments: "" = +1/+1, "N" = +ix[s]/+1, "M" = +1/+ix[3], "NM" = +ix[s]/+ix[3]
    match OP {
        OP_EXT_LD_00 => {
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.increment_ar(s);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.increment_ar(3);
        }
        OP_EXT_LDN_01 => {
            let ixs = ctx.dsp.registers.ix[s] as i16;
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.increase_ar(s, ixs);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.increment_ar(3);
        }
        OP_EXT_LDM_10 => {
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.increment_ar(s);
            let ix3 = ctx.dsp.registers.ix[3] as i16;
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.increase_ar(3, ix3);
        }
        OP_EXT_LDNM_11 => {
            let ixs = ctx.dsp.registers.ix[s] as i16;
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.increase_ar(s, ixs);
            let ix3 = ctx.dsp.registers.ix[3] as i16;
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.increase_ar(3, ix3);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn ext_ldax<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    let s = instr.d_2_2() as usize;
    let r = instr.r_3_3() as usize;

    let ar_s = ctx.dsp.registers.ar[s];
    let ar_3 = ctx.dsp.registers.ar[3];
    let same_bank = (ar_s >> 10) == (ar_3 >> 10);
    let high = dsp::read_dmem(ctx, ar_s);
    let low = if same_bank { high } else { dsp::read_dmem(ctx, ar_3) };

    ctx.dsp.registers.axh[r] = high;
    ctx.dsp.registers.ax[r] = low;

    // AR increments
    match OP {
        OP_EXT_LDAX => {
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.increment_ar(s);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.increment_ar(3);
        }
        OP_EXT_LDAXN => {
            let ixs = ctx.dsp.registers.ix[s] as i16;
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.increase_ar(s, ixs);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.increment_ar(3);
        }
        OP_EXT_LDAXM => {
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.increment_ar(s);
            let ix3 = ctx.dsp.registers.ix[3] as i16;
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.increase_ar(3, ix3);
        }
        OP_EXT_LDAXNM => {
            let ixs = ctx.dsp.registers.ix[s] as i16;
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.increase_ar(s, ixs);
            let ix3 = ctx.dsp.registers.ix[3] as i16;
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.increase_ar(3, ix3);
        }
        _ => unreachable!(),
    }
}
