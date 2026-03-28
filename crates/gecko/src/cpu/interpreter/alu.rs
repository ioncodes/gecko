#[inline(always)]
pub fn alu<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: crate::cpu::semantics::Instruction) {
    match OP {
        crate::cpu::lut::OP_ADDX => {
            let res = ctx.cpu.read_gpr(instr.ra()).wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            ctx.cpu.write_gpr(instr.rd(), res);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_ADDI | crate::cpu::lut::OP_ADDIS => {
            let ra = ctx.cpu.read_gpr_or_zero(instr.ra());
            let simm = if OP == crate::cpu::lut::OP_ADDIS {
                instr.simm() << 16
            } else {
                instr.simm()
            };
            ctx.cpu.write_gpr(instr.rd(), ra.wrapping_add_signed(simm));
        }
        crate::cpu::lut::OP_ORI | crate::cpu::lut::OP_ORIS => {
            let imm = if OP == crate::cpu::lut::OP_ORIS {
                (instr.uimm() as u32) << 16
            } else {
                instr.uimm() as u32
            };
            ctx.cpu.write_gpr(instr.ra(), ctx.cpu.read_gpr(instr.rs()) | imm);
        }
        crate::cpu::lut::OP_XORI | crate::cpu::lut::OP_XORIS => {
            let imm = if OP == crate::cpu::lut::OP_XORIS {
                (instr.uimm() as u32) << 16
            } else {
                instr.uimm() as u32
            };
            ctx.cpu.write_gpr(instr.ra(), ctx.cpu.read_gpr(instr.rs()) ^ imm);
        }
        crate::cpu::lut::OP_ANDI_DOT | crate::cpu::lut::OP_ANDIS_DOT => {
            let mask = if OP == crate::cpu::lut::OP_ANDIS_DOT {
                (instr.uimm() as u32) << 16
            } else {
                instr.uimm() as u32
            };
            let val = ctx.cpu.read_gpr(instr.rs()) & mask;
            ctx.cpu.write_gpr(instr.ra(), val);
            ctx.cpu.update_cr0(val);
        }
        crate::cpu::lut::OP_SUBFX => {
            let res = ctx.cpu.read_gpr(instr.rb()).wrapping_sub(ctx.cpu.read_gpr(instr.ra()));
            ctx.cpu.write_gpr(instr.rd(), res);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_NEGX => {
            let res = (!ctx.cpu.read_gpr(instr.ra())).wrapping_add(1);
            ctx.cpu.write_gpr(instr.rd(), res);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_ADDCX => {
            let ra = ctx.cpu.read_gpr(instr.ra());
            let rb = ctx.cpu.read_gpr(instr.rb());
            let (res, carry) = ra.overflowing_add(rb);
            ctx.cpu.write_gpr(instr.rd(), res);
            ctx.cpu.set_xer_ca(carry);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_SUBFCX => {
            let ra = ctx.cpu.read_gpr(instr.ra());
            let rb = ctx.cpu.read_gpr(instr.rb());
            let res = rb.wrapping_sub(ra);
            ctx.cpu.set_xer_ca(rb >= ra);
            ctx.cpu.write_gpr(instr.rd(), res);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_ADDEX => {
            let ra = ctx.cpu.read_gpr(instr.ra());
            let rb = ctx.cpu.read_gpr(instr.rb());
            let ca = ctx.cpu.xer_ca();
            let (t1, c1) = ra.overflowing_add(rb);
            let (res, c2) = t1.overflowing_add(ca);
            ctx.cpu.write_gpr(instr.rd(), res);
            ctx.cpu.set_xer_ca(c1 || c2);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_SUBFEX => {
            let ra = ctx.cpu.read_gpr(instr.ra());
            let rb = ctx.cpu.read_gpr(instr.rb());
            let ca = ctx.cpu.xer_ca();
            let (t1, c1) = (!ra).overflowing_add(rb);
            let (res, c2) = t1.overflowing_add(ca);
            ctx.cpu.write_gpr(instr.rd(), res);
            ctx.cpu.set_xer_ca(c1 || c2);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_ADDZEX => {
            let ra = ctx.cpu.read_gpr(instr.ra());
            let ca = ctx.cpu.xer_ca();
            let (res, carry) = ra.overflowing_add(ca);
            ctx.cpu.write_gpr(instr.rd(), res);
            ctx.cpu.set_xer_ca(carry);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_SUBFZEX => {
            let ra = ctx.cpu.read_gpr(instr.ra());
            let ca = ctx.cpu.xer_ca();
            let (res, carry) = (!ra).overflowing_add(ca);
            ctx.cpu.write_gpr(instr.rd(), res);
            ctx.cpu.set_xer_ca(carry);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_ADDMEX => {
            let ra = ctx.cpu.read_gpr(instr.ra());
            let ca = ctx.cpu.xer_ca();
            let (t1, c1) = ra.overflowing_add(ca);
            let (res, c2) = t1.overflowing_add(0xFFFF_FFFF);
            ctx.cpu.write_gpr(instr.rd(), res);
            ctx.cpu.set_xer_ca(c1 || c2);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_SUBFMEX => {
            let ra = ctx.cpu.read_gpr(instr.ra());
            let ca = ctx.cpu.xer_ca();
            let (t1, c1) = (!ra).overflowing_add(ca);
            let (res, c2) = t1.overflowing_add(0xFFFF_FFFF);
            ctx.cpu.write_gpr(instr.rd(), res);
            ctx.cpu.set_xer_ca(c1 || c2);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_MULLWX => {
            let res = (ctx.cpu.read_gpr(instr.ra()) as i32 as i64)
                .wrapping_mul(ctx.cpu.read_gpr(instr.rb()) as i32 as i64) as u32;
            ctx.cpu.write_gpr(instr.rd(), res);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_DIVWUX => {
            let ra = ctx.cpu.read_gpr(instr.ra());
            let rb = ctx.cpu.read_gpr(instr.rb());
            let res = if rb == 0 { 0 } else { ra / rb };
            ctx.cpu.write_gpr(instr.rd(), res);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_DIVWX => {
            let ra = ctx.cpu.read_gpr(instr.ra()) as i32;
            let rb = ctx.cpu.read_gpr(instr.rb()) as i32;
            let res = if rb == 0 || (ra == i32::MIN && rb == -1) {
                0u32
            } else {
                (ra / rb) as u32
            };
            ctx.cpu.write_gpr(instr.rd(), res);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_MULHWUX => {
            let res = ((ctx.cpu.read_gpr(instr.ra()) as u64 * ctx.cpu.read_gpr(instr.rb()) as u64) >> 32) as u32;
            ctx.cpu.write_gpr(instr.rd(), res);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_MULHWX => {
            let res = ((ctx.cpu.read_gpr(instr.ra()) as i32 as i64 * ctx.cpu.read_gpr(instr.rb()) as i32 as i64) >> 32)
                as u32;
            ctx.cpu.write_gpr(instr.rd(), res);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_SUBFIC => {
            let ra = ctx.cpu.read_gpr(instr.ra());
            let simm = instr.simm() as u32;
            ctx.cpu.write_gpr(instr.rd(), simm.wrapping_sub(ra));
            ctx.cpu.set_xer_ca(simm >= ra);
        }
        crate::cpu::lut::OP_ADDIC | crate::cpu::lut::OP_ADDIC_DOT => {
            let ra = ctx.cpu.read_gpr(instr.ra());
            let (res, carry) = ra.overflowing_add(instr.simm() as u32);
            ctx.cpu.write_gpr(instr.rd(), res);
            ctx.cpu.set_xer_ca(carry);
            if OP == crate::cpu::lut::OP_ADDIC_DOT {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_MULLI => {
            let res = (ctx.cpu.read_gpr(instr.ra()) as i32 as i64).wrapping_mul(instr.simm() as i64) as u32;
            ctx.cpu.write_gpr(instr.rd(), res);
        }
        _ => todo!("ALU instruction with OP = {OP:#x}"),
    }
}

#[inline(always)]
pub fn logical<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: crate::cpu::semantics::Instruction) {
    let rs = ctx.cpu.read_gpr(instr.rs());
    let rb = ctx.cpu.read_gpr(instr.rb());

    let res = match OP {
        crate::cpu::lut::OP_ANDX => rs & rb,
        crate::cpu::lut::OP_ORX => rs | rb,
        crate::cpu::lut::OP_XORX => rs ^ rb,
        crate::cpu::lut::OP_NANDX => !(rs & rb),
        crate::cpu::lut::OP_NORX => !(rs | rb),
        crate::cpu::lut::OP_ANDCX => rs & !rb,
        crate::cpu::lut::OP_ORCX => rs | !rb,
        crate::cpu::lut::OP_EQVX => !(rs ^ rb),
        crate::cpu::lut::OP_SLWX => {
            let sh = rb & 0x3F;
            if sh >= 32 { 0 } else { rs << sh }
        }
        crate::cpu::lut::OP_SRWX => {
            let sh = rb & 0x3F;
            if sh >= 32 { 0 } else { rs >> sh }
        }
        crate::cpu::lut::OP_SRAWX => {
            let sh = rb & 0x3F;
            let signed = rs as i32;
            if sh >= 32 {
                ctx.cpu.set_xer_ca(signed < 0);
                (signed >> 31) as u32
            } else if sh == 0 {
                ctx.cpu.set_xer_ca(false);
                rs
            } else {
                let mask = (1u32 << sh) - 1;
                ctx.cpu.set_xer_ca(signed < 0 && (rs & mask) != 0);
                (signed >> sh) as u32
            }
        }
        crate::cpu::lut::OP_SRAWIX => {
            let sh = instr.sh() as u32;
            let signed = rs as i32;
            if sh == 0 {
                ctx.cpu.set_xer_ca(false);
                rs
            } else {
                let mask = (1u32 << sh) - 1;
                ctx.cpu.set_xer_ca(signed < 0 && (rs & mask) != 0);
                (signed >> sh) as u32
            }
        }
        crate::cpu::lut::OP_CNTLZWX => rs.leading_zeros(),
        crate::cpu::lut::OP_EXTSHX => rs as i16 as i32 as u32,
        crate::cpu::lut::OP_EXTSBX => rs as i8 as i32 as u32,
        _ => todo!("Logical instruction with OP = {OP:#x}"),
    };

    ctx.cpu.write_gpr(instr.ra(), res);
    if instr.rc() {
        ctx.cpu.update_cr0(res);
    }
}
