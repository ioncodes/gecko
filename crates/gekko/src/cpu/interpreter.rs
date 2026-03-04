use crate::cpu::{
    condition::{BranchControl, ConditionField},
    sr::Sr,
};

pub fn branch<const OP: u32>(
    ctx: &mut crate::gekko::Gekko,
    instr: crate::cpu::semantics::Instruction,
) {
    if instr.lk() {
        ctx.cpu.spr.lr = ctx.cpu.cia.wrapping_add(4);
    }

    match OP {
        crate::cpu::lut::OP_BX => {
            ctx.cpu.nia = if instr.aa() {
                instr.li() as u32
            } else {
                ctx.cpu.cia.wrapping_add_signed(instr.li())
            }
        }
        crate::cpu::lut::OP_BCLRX | crate::cpu::lut::OP_BCX => {
            let ctrl = BranchControl::from_bo(instr.bo());
            tracing::trace!("Branch control: {ctrl:?}");

            if ctrl.should_decrement_ctr() {
                ctx.cpu.spr.ctr = ctx.cpu.spr.ctr.wrapping_sub(1);
            }

            let (crf, bit) = (instr.bi() >> 2, instr.bi() & 0b11);
            let condition = match bit {
                0 => ctx.cpu.cr.get_field(crf).lt(),
                1 => ctx.cpu.cr.get_field(crf).gt(),
                2 => ctx.cpu.cr.get_field(crf).eq(),
                3 => ctx.cpu.cr.get_field(crf).so(),
                _ => panic!("Invalid CR bit index: {}", bit),
            };
            if !ctrl.should_branch(ctx.cpu.spr.ctr, condition) {
                return;
            }

            match OP {
                crate::cpu::lut::OP_BCLRX => {
                    ctx.cpu.nia = ctx.cpu.spr.lr;
                }
                crate::cpu::lut::OP_BCX => {
                    ctx.cpu.nia = if instr.aa() {
                        instr.bd() as u32
                    } else {
                        ctx.cpu.cia.wrapping_add_signed(instr.bd())
                    }
                }
                _ => {
                    tracing::error!("missing OP = {OP:#x}");
                }
            }
        }
        _ => todo!("branch instruction with OP = {OP:#x}"),
    };
}

pub fn alu<const OP: u32>(
    ctx: &mut crate::gekko::Gekko,
    instr: crate::cpu::semantics::Instruction,
) {
    match OP {
        crate::cpu::lut::OP_ADDX => {
            let res = ctx
                .cpu
                .read_gpr(instr.ra())
                .wrapping_add(ctx.cpu.read_gpr(instr.rb()));
            ctx.cpu.write_gpr(instr.rd(), res);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        crate::cpu::lut::OP_ADDI | crate::cpu::lut::OP_ADDIS => {
            let ra = if instr.ra() == 0 {
                0
            } else {
                ctx.cpu.read_gpr(instr.ra())
            };
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
            ctx.cpu
                .write_gpr(instr.ra(), ctx.cpu.read_gpr(instr.rs()) | imm);
        }
        crate::cpu::lut::OP_XORI | crate::cpu::lut::OP_XORIS => {
            let imm = if OP == crate::cpu::lut::OP_XORIS {
                (instr.uimm() as u32) << 16
            } else {
                instr.uimm() as u32
            };
            ctx.cpu
                .write_gpr(instr.ra(), ctx.cpu.read_gpr(instr.rs()) ^ imm);
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
        _ => todo!("ALU instruction with OP = {OP:#x}"),
    }
}

pub fn rotate<const OP: u32>(
    ctx: &mut crate::gekko::Gekko,
    instr: crate::cpu::semantics::Instruction,
) {
    match OP {
        crate::cpu::lut::OP_RLWINMX => {
            let r = ctx.cpu.read_gpr(instr.rs()).rotate_left(instr.sh() as u32);
            let mb = instr.mb() as u32;
            let me = instr.me() as u32;
            let begin = 0xFFFF_FFFFu32 >> mb;
            let end = if me >= 31 {
                0
            } else {
                0xFFFF_FFFFu32 >> (me + 1)
            };
            let m = if mb <= me { begin & !end } else { begin | !end };
            let res = r & m;
            ctx.cpu.write_gpr(instr.ra(), res);
            if instr.rc() {
                ctx.cpu.update_cr0(res);
            }
        }
        _ => todo!("Rotate instruction with OP = {OP:#x}"),
    }
}

pub fn msr<const OP: u32>(
    ctx: &mut crate::gekko::Gekko,
    _instr: crate::cpu::semantics::Instruction,
) {
    match OP {
        crate::cpu::lut::OP_MTMSR => {
            tracing::error!("OP_MTMSR!!");
        }
        crate::cpu::lut::OP_MFMSR => {
            tracing::error!("OP_MFMSR!!");
        }
        crate::cpu::lut::OP_RFI => {
            // "Bits SRR1[16–23, 25–27, 30–31] are placed into the corresponding bits of the MSR."
            const RFI_MSR_MASK: u32 = 0x0000_FF73;
            ctx.cpu.msr = (ctx.cpu.msr & !RFI_MSR_MASK) | (ctx.cpu.spr.srr1 & RFI_MSR_MASK);
            // "If the new MSR value does not enable any pending exceptions, then the
            // next instruction is fetched, under control of the new MSR value,
            // from the address SRR0[0–29] || 0b00"
            ctx.cpu.pc = ctx.cpu.spr.srr0.value() << 2;
        }
        _ => todo!("MSR instruction with OP = {OP:#x}"),
    }
}

pub fn spr<const OP: u32>(
    ctx: &mut crate::gekko::Gekko,
    instr: crate::cpu::semantics::Instruction,
) {
    match OP {
        crate::cpu::lut::OP_MTSPR => ctx
            .cpu
            .spr
            .write(instr.spr_swapped(), ctx.cpu.read_gpr(instr.rs())),
        crate::cpu::lut::OP_MFSPR => ctx
            .cpu
            .write_gpr(instr.rd(), ctx.cpu.spr.read(instr.spr_swapped())),
        _ => todo!("SPR instruction with OP = {OP:#x}"),
    }
}

pub fn segment<const OP: u32>(
    ctx: &mut crate::gekko::Gekko,
    instr: crate::cpu::semantics::Instruction,
) {
    match OP {
        crate::cpu::lut::OP_MTSR => {
            ctx.cpu.sr[instr.sr() as usize] = Sr::from_raw(ctx.cpu.read_gpr(instr.rs()))
        }
        crate::cpu::lut::OP_MFSR => ctx
            .cpu
            .write_gpr(instr.rd(), ctx.cpu.sr[instr.sr() as usize].raw()),
        _ => todo!("Segment Register instruction with OP = {OP:#x}"),
    }
}

pub fn store_load<const OP: u32>(
    ctx: &mut crate::gekko::Gekko,
    instr: crate::cpu::semantics::Instruction,
) {
    match OP {
        crate::cpu::lut::OP_STW | crate::cpu::lut::OP_STWU => {
            let addr = ctx
                .cpu
                .read_gpr(instr.ra())
                .wrapping_add_signed(instr.disp());
            ctx.write_u32(addr, ctx.cpu.read_gpr(instr.rs()));
            if OP == crate::cpu::lut::OP_STWU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_STH | crate::cpu::lut::OP_STHU => {
            let addr = ctx
                .cpu
                .read_gpr(instr.ra())
                .wrapping_add_signed(instr.disp());
            ctx.write_u16(addr, (ctx.cpu.read_gpr(instr.rs()) & 0xffff) as u16);
            if OP == crate::cpu::lut::OP_STHU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_LWZ | crate::cpu::lut::OP_LWZU => {
            let addr = ctx
                .cpu
                .read_gpr(instr.ra())
                .wrapping_add_signed(instr.disp());
            let val = ctx.read_u32(addr);
            ctx.cpu.write_gpr(instr.rd(), val);
            if OP == crate::cpu::lut::OP_LWZU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        _ => todo!("Store/Load instruction with OP = {OP:#x}"),
    }
}

pub fn store_load_fp<const OP: u32>(
    ctx: &mut crate::gekko::Gekko,
    instr: crate::cpu::semantics::Instruction,
) {
    match OP {
        crate::cpu::lut::OP_LFD | crate::cpu::lut::OP_LFDU => {
            let addr = ctx
                .cpu
                .read_gpr(instr.ra())
                .wrapping_add_signed(instr.disp());
            let value =
                (ctx.read_u32(addr) as u64 | ((ctx.read_u32(addr + 4) as u64) << 32)) as f64;
            ctx.cpu.write_fpr(instr.rd(), value);

            if OP == crate::cpu::lut::OP_LFDU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        crate::cpu::lut::OP_STFD | crate::cpu::lut::OP_STFDU => {
            let addr = ctx
                .cpu
                .read_gpr(instr.ra())
                .wrapping_add_signed(instr.disp());
            let value = ctx.cpu.read_fpr(instr.rs()) as u64;
            ctx.write_u32(addr, value as u32);
            ctx.write_u32(addr + 4, (value >> 32) as u32);

            if OP == crate::cpu::lut::OP_STFDU {
                ctx.cpu.write_gpr(instr.ra(), addr);
            }
        }
        _ => todo!("FP Store/Load instruction with OP = {OP:#x}"),
    }
}

pub fn compare<const OP: u32>(
    ctx: &mut crate::gekko::Gekko,
    instr: crate::cpu::semantics::Instruction,
) {
    let (a, b) = match OP {
        crate::cpu::lut::OP_CMP => (
            ctx.cpu.read_gpr(instr.ra()) as i32,
            ctx.cpu.read_gpr(instr.rb()) as i32,
        ),
        crate::cpu::lut::OP_CMPI => (ctx.cpu.read_gpr(instr.ra()) as i32, instr.simm()),
        _ => todo!("Compare instruction with OP = {OP:#x}"),
    };

    ctx.cpu.cr.set_field(
        instr.crfd(),
        match a.cmp(&b) {
            std::cmp::Ordering::Less => ConditionField::new().with_lt(true),
            std::cmp::Ordering::Equal => ConditionField::new().with_eq(true),
            std::cmp::Ordering::Greater => ConditionField::new().with_gt(true),
        },
    );
}

pub fn nop<const OP: u32>(
    _ctx: &mut crate::gekko::Gekko,
    _instr: crate::cpu::semantics::Instruction,
) {
    tracing::warn!("Executing instruction treated as NOP: OP = {OP:#x}");
}

#[rustfmt::skip] pub fn twi(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("twi") }
#[rustfmt::skip] pub fn ps_cmpu0(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_cmpu0") }
#[rustfmt::skip] pub fn ps_cmpo0(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_cmpo0") }
#[rustfmt::skip] pub fn ps_cmpu1(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_cmpu1") }
#[rustfmt::skip] pub fn ps_cmpo1(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_cmpo1") }
#[rustfmt::skip] pub fn ps_res(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_res") }
#[rustfmt::skip] pub fn ps_rsqrte(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_rsqrte") }
#[rustfmt::skip] pub fn ps_neg(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_neg") }
#[rustfmt::skip] pub fn ps_mr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_mr") }
#[rustfmt::skip] pub fn ps_nabs(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_nabs") }
#[rustfmt::skip] pub fn ps_abs(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_abs") }
#[rustfmt::skip] pub fn ps_merge00(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_merge00") }
#[rustfmt::skip] pub fn ps_merge01(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_merge01") }
#[rustfmt::skip] pub fn ps_merge10(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_merge10") }
#[rustfmt::skip] pub fn ps_merge11(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_merge11") }
#[rustfmt::skip] pub fn psq_lx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_lx") }
#[rustfmt::skip] pub fn psq_stx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_stx") }
#[rustfmt::skip] pub fn psq_lux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_lux") }
#[rustfmt::skip] pub fn psq_stux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_stux") }
#[rustfmt::skip] pub fn ps_sum0(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_sum0") }
#[rustfmt::skip] pub fn ps_sum1(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_sum1") }
#[rustfmt::skip] pub fn ps_muls0(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_muls0") }
#[rustfmt::skip] pub fn ps_muls1(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_muls1") }
#[rustfmt::skip] pub fn ps_madds0(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_madds0") }
#[rustfmt::skip] pub fn ps_madds1(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_madds1") }
#[rustfmt::skip] pub fn ps_div(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_div") }
#[rustfmt::skip] pub fn ps_sub(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_sub") }
#[rustfmt::skip] pub fn ps_add(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_add") }
#[rustfmt::skip] pub fn ps_sel(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_sel") }
#[rustfmt::skip] pub fn ps_mul(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_mul") }
#[rustfmt::skip] pub fn ps_msub(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_msub") }
#[rustfmt::skip] pub fn ps_madd(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_madd") }
#[rustfmt::skip] pub fn ps_nmsub(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_nmsub") }
#[rustfmt::skip] pub fn ps_nmadd(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ps_nmadd") }
#[rustfmt::skip] pub fn dcbz_l(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbz_l") }
#[rustfmt::skip] pub fn mulli(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mulli") }
#[rustfmt::skip] pub fn subfic(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("subfic") }
#[rustfmt::skip] pub fn cmpli(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("cmpli") }
#[rustfmt::skip] pub fn addic(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addic") }
#[rustfmt::skip] pub fn addic_dot(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addic_dot") }
#[rustfmt::skip] pub fn bcx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("bcx") }
#[rustfmt::skip] pub fn sc(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("sc") }
#[rustfmt::skip] pub fn mcrf(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mcrf") }
#[rustfmt::skip] pub fn bclrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("bclrx") }
#[rustfmt::skip] pub fn crnor(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("crnor") }
#[rustfmt::skip] pub fn crandc(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("crandc") }
#[rustfmt::skip] pub fn crxor(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("crxor") }
#[rustfmt::skip] pub fn crnand(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("crnand") }
#[rustfmt::skip] pub fn crand(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("crand") }
#[rustfmt::skip] pub fn creqv(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("creqv") }
#[rustfmt::skip] pub fn crorc(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("crorc") }
#[rustfmt::skip] pub fn cror(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("cror") }
#[rustfmt::skip] pub fn bcctrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("bcctrx") }
#[rustfmt::skip] pub fn rlwimix(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("rlwimix") }
#[rustfmt::skip] pub fn rlwinmx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("rlwinmx") }
#[rustfmt::skip] pub fn rlwnmx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("rlwnmx") }
#[rustfmt::skip] pub fn ori(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ori") }
#[rustfmt::skip] pub fn oris(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("oris") }
#[rustfmt::skip] pub fn xori(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("xori") }
#[rustfmt::skip] pub fn xoris(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("xoris") }
#[rustfmt::skip] pub fn andi_dot(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("andi_dot") }
#[rustfmt::skip] pub fn andis_dot(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("andis_dot") }
#[rustfmt::skip] pub fn cmp(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("cmp") }
#[rustfmt::skip] pub fn tw(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("tw") }
#[rustfmt::skip] pub fn subfcx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("subfcx") }
#[rustfmt::skip] pub fn mulhwux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mulhwux") }
#[rustfmt::skip] pub fn addcx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addcx") }
#[rustfmt::skip] pub fn lwarx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lwarx") }
#[rustfmt::skip] pub fn lwzx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lwzx") }
#[rustfmt::skip] pub fn slwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("slwx") }
#[rustfmt::skip] pub fn cntlzwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("cntlzwx") }
#[rustfmt::skip] pub fn andx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("andx") }
#[rustfmt::skip] pub fn cmpl(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("cmpl") }
#[rustfmt::skip] pub fn subfx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("subfx") }
#[rustfmt::skip] pub fn dcbst(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbst") }
#[rustfmt::skip] pub fn lwzux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lwzux") }
#[rustfmt::skip] pub fn andcx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("andcx") }
#[rustfmt::skip] pub fn mulhwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mulhwx") }
#[rustfmt::skip] pub fn mfcr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mfcr") }
#[rustfmt::skip] pub fn dcbf(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbf") }
#[rustfmt::skip] pub fn lbzx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lbzx") }
#[rustfmt::skip] pub fn negx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("negx") }
#[rustfmt::skip] pub fn lbzux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lbzux") }
#[rustfmt::skip] pub fn norx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("norx") }
#[rustfmt::skip] pub fn subfex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("subfex") }
#[rustfmt::skip] pub fn addex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addex") }
#[rustfmt::skip] pub fn mtcrf(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtcrf") }
#[rustfmt::skip] pub fn stwcx_dot(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stwcx_dot") }
#[rustfmt::skip] pub fn stwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stwx") }
#[rustfmt::skip] pub fn stwux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stwux") }
#[rustfmt::skip] pub fn subfzex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("subfzex") }
#[rustfmt::skip] pub fn addzex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addzex") }
#[rustfmt::skip] pub fn mtsr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtsr") }
#[rustfmt::skip] pub fn stbx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stbx") }
#[rustfmt::skip] pub fn subfmex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("subfmex") }
#[rustfmt::skip] pub fn addmex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addmex") }
#[rustfmt::skip] pub fn mullwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mullwx") }
#[rustfmt::skip] pub fn mtsrin(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtsrin") }
#[rustfmt::skip] pub fn dcbtst(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbtst") }
#[rustfmt::skip] pub fn stbux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stbux") }
#[rustfmt::skip] pub fn addx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("addx") }
#[rustfmt::skip] pub fn dcbt(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbt") }
#[rustfmt::skip] pub fn lhzx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhzx") }
#[rustfmt::skip] pub fn eqvx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("eqvx") }
#[rustfmt::skip] pub fn tlbie(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("tlbie") }
#[rustfmt::skip] pub fn eciwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("eciwx") }
#[rustfmt::skip] pub fn lhzux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhzux") }
#[rustfmt::skip] pub fn xorx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("xorx") }
#[rustfmt::skip] pub fn lhax(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhax") }
#[rustfmt::skip] pub fn tlbia(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("tlbia") }
#[rustfmt::skip] pub fn mftb(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mftb") }
#[rustfmt::skip] pub fn lhaux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhaux") }
#[rustfmt::skip] pub fn sthx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("sthx") }
#[rustfmt::skip] pub fn orcx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("orcx") }
#[rustfmt::skip] pub fn ecowx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("ecowx") }
#[rustfmt::skip] pub fn sthux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("sthux") }
#[rustfmt::skip] pub fn orx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("orx") }
#[rustfmt::skip] pub fn divwux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("divwux") }
#[rustfmt::skip] pub fn dcbi(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbi") }
#[rustfmt::skip] pub fn nandx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("nandx") }
#[rustfmt::skip] pub fn divwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("divwx") }
#[rustfmt::skip] pub fn mcrxr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mcrxr") }
#[rustfmt::skip] pub fn lwbrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lwbrx") }
#[rustfmt::skip] pub fn lfsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfsx") }
#[rustfmt::skip] pub fn srwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("srwx") }
#[rustfmt::skip] pub fn lswx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lswx") }
#[rustfmt::skip] pub fn tlbsync(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("tlbsync") }
#[rustfmt::skip] pub fn lfsux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfsux") }
#[rustfmt::skip] pub fn mfsr(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mfsr") }
#[rustfmt::skip] pub fn lswi(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lswi") }
#[rustfmt::skip] pub fn sync(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("sync") }
#[rustfmt::skip] pub fn lfdx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfdx") }
#[rustfmt::skip] pub fn lfdux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfdux") }
#[rustfmt::skip] pub fn mfsrin(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mfsrin") }
#[rustfmt::skip] pub fn stswx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stswx") }
#[rustfmt::skip] pub fn stwbrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stwbrx") }
#[rustfmt::skip] pub fn stfsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfsx") }
#[rustfmt::skip] pub fn stfsux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfsux") }
#[rustfmt::skip] pub fn stswi(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stswi") }
#[rustfmt::skip] pub fn stfdx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfdx") }
#[rustfmt::skip] pub fn stfdux(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfdux") }
#[rustfmt::skip] pub fn dcba(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcba") }
#[rustfmt::skip] pub fn lhbrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhbrx") }
#[rustfmt::skip] pub fn srawx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("srawx") }
#[rustfmt::skip] pub fn srawix(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("srawix") }
#[rustfmt::skip] pub fn eieio(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("eieio") }
#[rustfmt::skip] pub fn sthbrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("sthbrx") }
#[rustfmt::skip] pub fn extshx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("extshx") }
#[rustfmt::skip] pub fn extsbx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("extsbx") }
#[rustfmt::skip] pub fn icbi(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("icbi") }
#[rustfmt::skip] pub fn stfiwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfiwx") }
#[rustfmt::skip] pub fn tlbld(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("tlbld") }
#[rustfmt::skip] pub fn tlbli(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("tlbli") }
#[rustfmt::skip] pub fn dcbz(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("dcbz") }
#[rustfmt::skip] pub fn lbz(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lbz") }
#[rustfmt::skip] pub fn lbzu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lbzu") }
#[rustfmt::skip] pub fn stw(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stw") }
#[rustfmt::skip] pub fn stb(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stb") }
#[rustfmt::skip] pub fn stbu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stbu") }
#[rustfmt::skip] pub fn lhz(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhz") }
#[rustfmt::skip] pub fn lhzu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhzu") }
#[rustfmt::skip] pub fn lha(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lha") }
#[rustfmt::skip] pub fn lhau(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lhau") }
#[rustfmt::skip] pub fn lmw(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lmw") }
#[rustfmt::skip] pub fn stmw(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stmw") }
#[rustfmt::skip] pub fn lfs(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfs") }
#[rustfmt::skip] pub fn lfsu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfsu") }
#[rustfmt::skip] pub fn lfd(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfd") }
#[rustfmt::skip] pub fn lfdu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("lfdu") }
#[rustfmt::skip] pub fn stfs(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfs") }
#[rustfmt::skip] pub fn stfsu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfsu") }
#[rustfmt::skip] pub fn stfd(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfd") }
#[rustfmt::skip] pub fn stfdu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("stfdu") }
#[rustfmt::skip] pub fn psq_l(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_l") }
#[rustfmt::skip] pub fn psq_lu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_lu") }
#[rustfmt::skip] pub fn fdivsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fdivsx") }
#[rustfmt::skip] pub fn fsubsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fsubsx") }
#[rustfmt::skip] pub fn faddsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("faddsx") }
#[rustfmt::skip] pub fn fsqrtsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fsqrtsx") }
#[rustfmt::skip] pub fn fresx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fresx") }
#[rustfmt::skip] pub fn fmulsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmulsx") }
#[rustfmt::skip] pub fn fmsubsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmsubsx") }
#[rustfmt::skip] pub fn fmaddsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmaddsx") }
#[rustfmt::skip] pub fn fnmsubsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fnmsubsx") }
#[rustfmt::skip] pub fn fnmaddsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fnmaddsx") }
#[rustfmt::skip] pub fn psq_st(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_st") }
#[rustfmt::skip] pub fn psq_stu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("psq_stu") }
#[rustfmt::skip] pub fn fcmpu(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fcmpu") }
#[rustfmt::skip] pub fn frspx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("frspx") }
#[rustfmt::skip] pub fn fctiwx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fctiwx") }
#[rustfmt::skip] pub fn fctiwzx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fctiwzx") }
#[rustfmt::skip] pub fn fcmpo(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fcmpo") }
#[rustfmt::skip] pub fn mtfsb1x(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtfsb1x") }
#[rustfmt::skip] pub fn fnegx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fnegx") }
#[rustfmt::skip] pub fn mcrfs(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mcrfs") }
#[rustfmt::skip] pub fn mtfsb0x(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtfsb0x") }
#[rustfmt::skip] pub fn fmrx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmrx") }
#[rustfmt::skip] pub fn mtfsfix(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtfsfix") }
#[rustfmt::skip] pub fn fnabsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fnabsx") }
#[rustfmt::skip] pub fn fabsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fabsx") }
#[rustfmt::skip] pub fn mffsx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mffsx") }
#[rustfmt::skip] pub fn mtfsfx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("mtfsfx") }
#[rustfmt::skip] pub fn fdivx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fdivx") }
#[rustfmt::skip] pub fn fsubx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fsubx") }
#[rustfmt::skip] pub fn faddx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("faddx") }
#[rustfmt::skip] pub fn fsqrtx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fsqrtx") }
#[rustfmt::skip] pub fn fselx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fselx") }
#[rustfmt::skip] pub fn fmulx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmulx") }
#[rustfmt::skip] pub fn frsqrtex(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("frsqrtex") }
#[rustfmt::skip] pub fn fmsubx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmsubx") }
#[rustfmt::skip] pub fn fmaddx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fmaddx") }
#[rustfmt::skip] pub fn fnmsubx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fnmsubx") }
#[rustfmt::skip] pub fn fnmaddx(_ctx: &mut crate::gekko::Gekko, _instr: crate::cpu::semantics::Instruction) { todo!("fnmaddx") }
