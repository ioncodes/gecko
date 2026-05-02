use crate::gekko::instruction::Instruction;
use crate::gekko::lut::*;
use crate::system::{System, SystemId};

#[inline(always)]
fn set_overflow<const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, overflow: bool) {
    ctx.gekko.spr.xer = ctx
        .gekko
        .spr
        .xer
        .with_overflow(overflow)
        .with_summary_overflow(ctx.gekko.spr.xer.summary_overflow() || overflow);
}

#[inline(always)]
fn add_overflow(a: u32, b: u32, result: u32) -> bool {
    (((a ^ result) & (b ^ result)) >> 31) != 0
}

#[inline(always)]
pub fn alu<const OP: u32, const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    match OP {
        OP_ADDX => {
            let a = ctx.gekko.read_gpr(instr.ra());
            let b = ctx.gekko.read_gpr(instr.rb());
            let res = a.wrapping_add(b);
            ctx.gekko.write_gpr(instr.rd(), res);
            if instr.oe() {
                set_overflow(ctx, add_overflow(a, b, res));
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_ADDI | OP_ADDIS => {
            let ra = ctx.gekko.read_gpr_or_zero(instr.ra());
            let simm = if OP == OP_ADDIS {
                instr.simm() << 16
            } else {
                instr.simm()
            };
            ctx.gekko.write_gpr(instr.rd(), ra.wrapping_add_signed(simm));
        }
        OP_ORI | OP_ORIS => {
            let imm = if OP == OP_ORIS {
                (instr.uimm() as u32) << 16
            } else {
                instr.uimm() as u32
            };
            ctx.gekko.write_gpr(instr.ra(), ctx.gekko.read_gpr(instr.rs()) | imm);
        }
        OP_XORI | OP_XORIS => {
            let imm = if OP == OP_XORIS {
                (instr.uimm() as u32) << 16
            } else {
                instr.uimm() as u32
            };
            ctx.gekko.write_gpr(instr.ra(), ctx.gekko.read_gpr(instr.rs()) ^ imm);
        }
        OP_ANDI_DOT | OP_ANDIS_DOT => {
            let mask = if OP == OP_ANDIS_DOT {
                (instr.uimm() as u32) << 16
            } else {
                instr.uimm() as u32
            };
            let val = ctx.gekko.read_gpr(instr.rs()) & mask;
            ctx.gekko.write_gpr(instr.ra(), val);
            ctx.gekko.update_cr0(val);
        }
        OP_SUBFX => {
            let a = !ctx.gekko.read_gpr(instr.ra());
            let b = ctx.gekko.read_gpr(instr.rb());
            let res = a.wrapping_add(b).wrapping_add(1);
            ctx.gekko.write_gpr(instr.rd(), res);
            if instr.oe() {
                set_overflow(ctx, add_overflow(a, b, res));
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_NEGX => {
            let a = ctx.gekko.read_gpr(instr.ra());
            let res = (!a).wrapping_add(1);
            ctx.gekko.write_gpr(instr.rd(), res);
            if instr.oe() {
                set_overflow(ctx, a == 0x8000_0000);
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_ADDCX => {
            let ra = ctx.gekko.read_gpr(instr.ra());
            let rb = ctx.gekko.read_gpr(instr.rb());
            let (res, carry) = ra.overflowing_add(rb);
            ctx.gekko.write_gpr(instr.rd(), res);
            ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(carry);
            if instr.oe() {
                set_overflow(ctx, add_overflow(ra, rb, res));
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_SUBFCX => {
            let ra = ctx.gekko.read_gpr(instr.ra());
            let rb = ctx.gekko.read_gpr(instr.rb());
            let res = rb.wrapping_sub(ra);
            ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(rb >= ra);
            ctx.gekko.write_gpr(instr.rd(), res);
            if instr.oe() {
                set_overflow(ctx, add_overflow(!ra, rb, res));
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_ADDEX => {
            let ra = ctx.gekko.read_gpr(instr.ra());
            let rb = ctx.gekko.read_gpr(instr.rb());
            let ca = ctx.gekko.spr.xer.carry() as u32;
            let (t1, c1) = ra.overflowing_add(rb);
            let (res, c2) = t1.overflowing_add(ca);
            ctx.gekko.write_gpr(instr.rd(), res);
            ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(c1 || c2);
            if instr.oe() {
                set_overflow(ctx, add_overflow(ra, rb, res));
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_SUBFEX => {
            let ra = ctx.gekko.read_gpr(instr.ra());
            let rb = ctx.gekko.read_gpr(instr.rb());
            let ca = ctx.gekko.spr.xer.carry() as u32;
            let (t1, c1) = (!ra).overflowing_add(rb);
            let (res, c2) = t1.overflowing_add(ca);
            ctx.gekko.write_gpr(instr.rd(), res);
            ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(c1 || c2);
            if instr.oe() {
                set_overflow(ctx, add_overflow(!ra, rb, res));
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_ADDZEX => {
            let ra = ctx.gekko.read_gpr(instr.ra());
            let ca = ctx.gekko.spr.xer.carry() as u32;
            let (res, carry) = ra.overflowing_add(ca);
            ctx.gekko.write_gpr(instr.rd(), res);
            ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(carry);
            if instr.oe() {
                set_overflow(ctx, add_overflow(ra, 0, res));
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_SUBFZEX => {
            let ra = ctx.gekko.read_gpr(instr.ra());
            let ca = ctx.gekko.spr.xer.carry() as u32;
            let (res, carry) = (!ra).overflowing_add(ca);
            ctx.gekko.write_gpr(instr.rd(), res);
            ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(carry);
            if instr.oe() {
                set_overflow(ctx, add_overflow(!ra, 0, res));
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_ADDMEX => {
            let ra = ctx.gekko.read_gpr(instr.ra());
            let ca = ctx.gekko.spr.xer.carry() as u32;
            let (t1, c1) = ra.overflowing_add(ca);
            let (res, c2) = t1.overflowing_add(0xFFFF_FFFF);
            ctx.gekko.write_gpr(instr.rd(), res);
            ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(c1 || c2);
            if instr.oe() {
                set_overflow(ctx, add_overflow(ra, 0xFFFF_FFFF, res));
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_SUBFMEX => {
            let ra = ctx.gekko.read_gpr(instr.ra());
            let ca = ctx.gekko.spr.xer.carry() as u32;
            let (t1, c1) = (!ra).overflowing_add(ca);
            let (res, c2) = t1.overflowing_add(0xFFFF_FFFF);
            ctx.gekko.write_gpr(instr.rd(), res);
            ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(c1 || c2);
            if instr.oe() {
                set_overflow(ctx, add_overflow(!ra, 0xFFFF_FFFF, res));
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_MULLWX => {
            let full = (ctx.gekko.read_gpr(instr.ra()) as i32 as i64)
                .wrapping_mul(ctx.gekko.read_gpr(instr.rb()) as i32 as i64);
            let res = full as u32;
            ctx.gekko.write_gpr(instr.rd(), res);
            if instr.oe() {
                set_overflow(ctx, full < i32::MIN as i64 || full > i32::MAX as i64);
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_DIVWUX => {
            let ra = ctx.gekko.read_gpr(instr.ra());
            let rb = ctx.gekko.read_gpr(instr.rb());
            let res = if rb == 0 { 0 } else { ra / rb };
            ctx.gekko.write_gpr(instr.rd(), res);
            if instr.oe() {
                set_overflow(ctx, rb == 0);
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_DIVWX => {
            let ra = ctx.gekko.read_gpr(instr.ra()) as i32;
            let rb = ctx.gekko.read_gpr(instr.rb()) as i32;
            let res = if rb == 0 || (ra == i32::MIN && rb == -1) {
                if ra < 0 { u32::MAX } else { 0 }
            } else {
                (ra / rb) as u32
            };
            ctx.gekko.write_gpr(instr.rd(), res);
            if instr.oe() {
                set_overflow(ctx, rb == 0 || (ra == i32::MIN && rb == -1));
            }
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_MULHWUX => {
            let res = ((ctx.gekko.read_gpr(instr.ra()) as u64 * ctx.gekko.read_gpr(instr.rb()) as u64) >> 32) as u32;
            ctx.gekko.write_gpr(instr.rd(), res);
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_MULHWX => {
            let res = ((ctx.gekko.read_gpr(instr.ra()) as i32 as i64 * ctx.gekko.read_gpr(instr.rb()) as i32 as i64)
                >> 32) as u32;
            ctx.gekko.write_gpr(instr.rd(), res);
            if instr.rc() {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_SUBFIC => {
            let ra = ctx.gekko.read_gpr(instr.ra());
            let simm = instr.simm() as u32;
            ctx.gekko.write_gpr(instr.rd(), simm.wrapping_sub(ra));
            ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(simm >= ra);
        }
        OP_ADDIC | OP_ADDIC_DOT => {
            let ra = ctx.gekko.read_gpr(instr.ra());
            let (res, carry) = ra.overflowing_add(instr.simm() as u32);
            ctx.gekko.write_gpr(instr.rd(), res);
            ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(carry);
            if OP == OP_ADDIC_DOT {
                ctx.gekko.update_cr0(res);
            }
        }
        OP_MULLI => {
            let res = (ctx.gekko.read_gpr(instr.ra()) as i32 as i64).wrapping_mul(instr.simm() as i64) as u32;
            ctx.gekko.write_gpr(instr.rd(), res);
        }
        _ => todo!("ALU instruction with OP = {OP:#x}"),
    }
}

#[inline(always)]
pub fn logical<const OP: u32, const SYSTEM: SystemId>(ctx: &mut System<SYSTEM>, instr: Instruction) {
    let rs = ctx.gekko.read_gpr(instr.rs());
    let rb = ctx.gekko.read_gpr(instr.rb());

    let res = match OP {
        OP_ANDX => rs & rb,
        OP_ORX => rs | rb,
        OP_XORX => rs ^ rb,
        OP_NANDX => !(rs & rb),
        OP_NORX => !(rs | rb),
        OP_ANDCX => rs & !rb,
        OP_ORCX => rs | !rb,
        OP_EQVX => !(rs ^ rb),
        OP_SLWX => {
            let sh = rb & 0x3F;
            if sh >= 32 { 0 } else { rs << sh }
        }
        OP_SRWX => {
            let sh = rb & 0x3F;
            if sh >= 32 { 0 } else { rs >> sh }
        }
        OP_SRAWX => {
            let sh = rb & 0x3F;
            let signed = rs as i32;
            if sh >= 32 {
                ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(signed < 0);
                (signed >> 31) as u32
            } else if sh == 0 {
                ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(false);
                rs
            } else {
                let mask = (1u32 << sh) - 1;
                ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(signed < 0 && (rs & mask) != 0);
                (signed >> sh) as u32
            }
        }
        OP_SRAWIX => {
            let sh = instr.sh() as u32;
            let signed = rs as i32;
            if sh == 0 {
                ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(false);
                rs
            } else {
                let mask = (1u32 << sh) - 1;
                ctx.gekko.spr.xer = ctx.gekko.spr.xer.with_carry(signed < 0 && (rs & mask) != 0);
                (signed >> sh) as u32
            }
        }
        OP_CNTLZWX => rs.leading_zeros(),
        OP_EXTSHX => rs as i16 as i32 as u32,
        OP_EXTSBX => rs as i8 as i32 as u32,
        _ => todo!("Logical instruction with OP = {OP:#x}"),
    };

    ctx.gekko.write_gpr(instr.ra(), res);
    if instr.rc() {
        ctx.gekko.update_cr0(res);
    }
}
