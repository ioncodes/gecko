use crate::flipper::dsp::{
    condition::BranchControl,
    core::{SignExtensionMode, StatusRegister, reg},
    lut::*,
};

#[inline(always)]
pub fn add_sub<const OP: u32>(
    _ctx: &mut crate::gamecube::GameCube,
    _instr: crate::flipper::dsp::instruction::Instruction,
) {
    match OP {
        OP_ADDR => todo!("addr"),
        OP_ADDAX => todo!("addax"),
        OP_ADD => todo!("add"),
        OP_ADDP => todo!("addp"),
        OP_SUBR => todo!("subr"),
        OP_SUBAX => todo!("subax"),
        OP_SUB => todo!("sub"),
        OP_SUBP => todo!("subp"),
        OP_ADDAXL => todo!("addaxl"),
        OP_ADDPAXZ => todo!("addpaxz"),
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn addr_reg<const OP: u32>(
    _ctx: &mut crate::gamecube::GameCube,
    _instr: crate::flipper::dsp::instruction::Instruction,
) {
    match OP {
        OP_DAR => todo!("dar"),
        OP_IAR => todo!("iar"),
        OP_SUBARN => todo!("subarn"),
        OP_ADDARN => todo!("addarn"),
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn cmp_test<const OP: u32>(
    ctx: &mut crate::gamecube::GameCube,
    instr: crate::flipper::dsp::instruction::Instruction,
) {
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
            let a = ctx.dsp.registers.ac(r);
            let b = (ctx.dsp.registers.axh[s as usize] as i16 as i64) << 16;
            let result = a.wrapping_sub(b);
            ctx.dsp.registers.update_flags_sub(a, b, result);
        }
        OP_TST => todo!("tst"),
        OP_TSTPROD => todo!("tstprod"),
        OP_TSTAXH => todo!("tstaxh"),
        OP_NX_0 => todo!("nx_0"),
        OP_NX_1 => todo!("nx_1"),
        OP_CLR => {
            let r = instr.r_4_4();
            // Clear all three parts of the accumulator
            ctx.dsp.registers.write::<false>(reg::AC0H + r, 0);
            ctx.dsp.registers.write::<false>(reg::AC0L + r, 0);
            ctx.dsp.registers.write::<false>(reg::AC0M + r, 0);

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
            let r = instr.r_4_4();
            let ac = ctx.dsp.registers.ac(r);
            let rounded = if (ac & 0x10000) != 0 {
                (ac.wrapping_add(0x8000)) & !0xFFFF
            } else {
                (ac.wrapping_add(0x7FFF)) & !0xFFFF
            };
            ctx.dsp.registers.set_ac(r, rounded);
            ctx.dsp.registers.update_flags_ac(rounded);
            ctx.dsp.registers.status.set_o(false);
            ctx.dsp.registers.status.set_c(false);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn control<const OP: u32>(
    ctx: &mut crate::gamecube::GameCube,
    instr: crate::flipper::dsp::instruction::Instruction,
) {
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
                // Skip the next instruction
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
                ctx.dsp.registers.nia = ctx.dsp.registers.read::<true>(instr.reg_5_7());
            }
        }
        OP_CALLRCC => {
            let branch_control = BranchControl::from(instr.cond());
            if branch_control.evaluate(&ctx.dsp) {
                ctx.dsp.registers.call_stack.push(ctx.dsp.registers.nia);
                ctx.dsp.registers.nia = ctx.dsp.registers.read::<true>(instr.reg_5_7());
            }
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn imm_alu<const OP: u32>(
    ctx: &mut crate::gamecube::GameCube,
    instr: crate::flipper::dsp::instruction::Instruction,
) {
    match OP {
        OP_ADDI => todo!("addi"),
        OP_XORI => todo!("xori"),
        OP_ANDI => todo!("andi"),
        OP_ORI => todo!("ori"),
        OP_CMPI => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let b = (instr.imm_16_31() as i16 as i64) << 16;
            let result = a.wrapping_sub(b);
            ctx.dsp.registers.update_flags_sub(a, b, result);
        }
        OP_ANDF | OP_ANDCF => {
            let ac_mid = if instr.d_7_7() != 0 {
                ctx.dsp.registers.ac1_mid
            } else {
                ctx.dsp.registers.ac0_mid
            };
            let imm = instr.imm_16_31();
            let result = ac_mid & imm;
            let lz = match OP {
                OP_ANDF => result == 0,
                OP_ANDCF => result == imm,
                _ => unreachable!(),
            };
            ctx.dsp.registers.status.set_lz(lz);
        }
        OP_ADDIS => todo!("addis"),
        OP_CMPIS => {
            let d = instr.d_7_7();
            let a = ctx.dsp.registers.ac(d);
            let b = (instr.imm_8_15_i16() as i64) << 16;
            let result = a.wrapping_sub(b);
            ctx.dsp.registers.update_flags_sub(a, b, result);
        }
        OP_LRIS => todo!("lris"),
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn inc_dec<const OP: u32>(
    _ctx: &mut crate::gamecube::GameCube,
    _instr: crate::flipper::dsp::instruction::Instruction,
) {
    match OP {
        OP_INCM => todo!("incm"),
        OP_INC => todo!("inc"),
        OP_DECM => todo!("decm"),
        OP_DEC => todo!("dec"),
        OP_NEG => todo!("neg"),
        OP_ABS_AC => todo!("abs_ac"),
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn load_store<const OP: u32>(
    ctx: &mut crate::gamecube::GameCube,
    instr: crate::flipper::dsp::instruction::Instruction,
) {
    match OP {
        OP_LRI => {
            ctx.dsp.registers.write::<true>(instr.d_11_15(), instr.imm_16_31());
        }
        OP_LR => {
            let value = ctx.dsp.read_dmem(instr.imm_16_31());
            ctx.dsp.registers.write::<true>(instr.d_11_15(), value);
        }
        OP_SR => {
            let value = ctx.dsp.registers.read::<true>(instr.d_11_15());
            ctx.dsp.write_dmem(instr.imm_16_31(), value);
        }
        OP_MRR => {
            let value = ctx.dsp.registers.read::<true>(instr.src());
            ctx.dsp.registers.write::<true>(instr.dst(), value);
        }
        OP_SI => {
            let addr = 0xFF00 | (instr.mem_8_15_u16());
            ctx.dsp.write_dmem(addr, instr.imm_16_31());
        }
        OP_LRR | OP_LRRD | OP_LRRI | OP_LRRN => {
            let s = instr.s_9_10() as usize;
            let d = instr.d_11_15();
            let value = ctx.dsp.read_dmem(ctx.dsp.registers.ar[s]);
            ctx.dsp.registers.write::<true>(d, value);
            match OP {
                OP_LRRD => ctx.dsp.registers.ar[s] = ctx.dsp.registers.ar[s].wrapping_sub(1),
                OP_LRRI => ctx.dsp.registers.ar[s] = ctx.dsp.registers.ar[s].wrapping_add(1),
                OP_LRRN => ctx.dsp.registers.ar[s] = ctx.dsp.registers.ar[s].wrapping_add(ctx.dsp.registers.ix[s]),
                _ => {}
            }
        }
        OP_SRR | OP_SRRD | OP_SRRI | OP_SRRN => {
            let d = instr.s_9_10() as usize;
            let value = ctx.dsp.registers.read::<true>(instr.d_11_15());
            ctx.dsp.write_dmem(ctx.dsp.registers.ar[d], value);
            match OP {
                OP_SRRD => ctx.dsp.registers.ar[d] = ctx.dsp.registers.ar[d].wrapping_sub(1),
                OP_SRRI => ctx.dsp.registers.ar[d] = ctx.dsp.registers.ar[d].wrapping_add(1),
                OP_SRRN => ctx.dsp.registers.ar[d] = ctx.dsp.registers.ar[d].wrapping_add(ctx.dsp.registers.ix[d]),
                _ => {}
            }
        }
        OP_LRS => {
            let dst = reg::AX0L + instr.reg_5_7();
            let addr = ((ctx.dsp.registers.config as u16) << 8) | instr.mem_8_15_u16();
            let value = ctx.dsp.read_dmem(addr);
            ctx.dsp.registers.write::<true>(dst, value);
        }
        OP_SRSH | OP_SRS => {
            let addr = ((ctx.dsp.registers.config as u16) << 8) | instr.mem_8_15_u16();
            let src = match OP {
                OP_SRSH => {
                    if instr.s_7_7() != 0 {
                        reg::AC0H
                    } else {
                        reg::AC1H
                    }
                }
                OP_SRS => reg::AC0L + instr.reg_6_7(),
                _ => unreachable!(),
            };
            let value = ctx.dsp.registers.read::<true>(src);
            ctx.dsp.write_dmem(addr, value);
        }
        OP_ILRR | OP_ILRRD | OP_ILRRI | OP_ILRRN => {
            let src = instr.s_14_15() as usize;
            let dst = if instr.d_7_7() != 0 { reg::AC1M } else { reg::AC0M };
            let value = ctx.dsp.read_imem(ctx.dsp.registers.ar[src]);
            ctx.dsp.registers.write::<true>(dst, value);
            match OP {
                OP_ILRRD => ctx.dsp.registers.ar[src] = ctx.dsp.registers.ar[src].wrapping_sub(1),
                OP_ILRRI => ctx.dsp.registers.ar[src] = ctx.dsp.registers.ar[src].wrapping_add(1),
                OP_ILRRN => {
                    ctx.dsp.registers.ar[src] = ctx.dsp.registers.ar[src].wrapping_add(ctx.dsp.registers.ix[src])
                }
                _ => {}
            }
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn logic<const OP: u32>(
    _ctx: &mut crate::gamecube::GameCube,
    _instr: crate::flipper::dsp::instruction::Instruction,
) {
    match OP {
        OP_XORR => todo!("xorr"),
        OP_ANDR => todo!("andr"),
        OP_ORR => todo!("orr"),
        OP_ANDC => todo!("andc"),
        OP_ORC => todo!("orc"),
        OP_XORC => todo!("xorc"),
        OP_NOT_AC => todo!("not_ac"),
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn loops<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: crate::flipper::dsp::instruction::Instruction) {
    match OP {
        OP_LOOP_REG | OP_LOOPI => {
            let counter = match OP {
                OP_LOOP_REG => ctx.dsp.registers.read::<true>(instr.reg_11_15()),
                OP_LOOPI => instr.imm_8_15_u8() as u16,
                _ => unreachable!(),
            };
            let end_addr = ctx.dsp.registers.nia; // the instruction to loop on
            if counter != 0 {
                ctx.dsp.registers.call_stack.push(end_addr);
                ctx.dsp.registers.loop_addr.push(end_addr.wrapping_add(1));
                ctx.dsp.registers.loop_counter.push(counter);
            } else {
                // Skip the looped instruction
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
pub fn move_ops<const OP: u32>(
    _ctx: &mut crate::gamecube::GameCube,
    _instr: crate::flipper::dsp::instruction::Instruction,
) {
    match OP {
        OP_MOVR => todo!("movr"),
        OP_MOVAX => todo!("movax"),
        OP_MOV => todo!("mov"),
        OP_MOVP => todo!("movp"),
        OP_MOVPZ => todo!("movpz"),
        OP_MOVNP => todo!("movnp"),
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn mul<const OP: u32>(_ctx: &mut crate::gamecube::GameCube, _instr: crate::flipper::dsp::instruction::Instruction) {
    match OP {
        OP_MUL => todo!("mul"),
        OP_MULX => todo!("mulx"),
        OP_MULC => todo!("mulc"),
        OP_MULAXH => todo!("mulaxh"),
        OP_MULMV => todo!("mulmv"),
        OP_MULMVZ => todo!("mulmvz"),
        OP_MULAC => todo!("mulac"),
        OP_MULXMV => todo!("mulxmv"),
        OP_MULXMVZ => todo!("mulxmvz"),
        OP_MULXAC => todo!("mulxac"),
        OP_MULCMV => todo!("mulcmv"),
        OP_MULCMVZ => todo!("mulcmvz"),
        OP_MULCAC => todo!("mulcac"),
        OP_MADD => todo!("madd"),
        OP_MSUB => todo!("msub"),
        OP_MADDX => todo!("maddx"),
        OP_MSUBX => todo!("msubx"),
        OP_MADDC => todo!("maddc"),
        OP_MSUBC => todo!("msubc"),
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn shifts<const OP: u32>(
    _ctx: &mut crate::gamecube::GameCube,
    _instr: crate::flipper::dsp::instruction::Instruction,
) {
    match OP {
        OP_LSL => todo!("lsl"),
        OP_LSR => todo!("lsr"),
        OP_ASL => todo!("asl"),
        OP_ASR => todo!("asr"),
        OP_LSL16 => todo!("lsl16"),
        OP_LSR16 => todo!("lsr16"),
        OP_ASR16 => todo!("asr16"),
        OP_LSRN => todo!("lsrn"),
        OP_ASRN => todo!("asrn"),
        OP_LSRNRX => todo!("lsrnrx"),
        OP_ASRNRX => todo!("asrnrx"),
        OP_LSRNR => todo!("lsrnr"),
        OP_ASRNR => todo!("asrnr"),
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn status<const OP: u32>(
    ctx: &mut crate::gamecube::GameCube,
    instr: crate::flipper::dsp::instruction::Instruction,
) {
    match OP {
        OP_SBCLR => {
            ctx.dsp.registers.status &= !(1 << (6 + instr.bit())) as u16;
        }
        OP_SBSET => {
            ctx.dsp.registers.status |= (1 << (6 + instr.bit())) as u16;
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
use crate::flipper::dsp::instruction::GcDspExt;

#[inline(always)]
pub fn ext_nop(_ctx: &mut crate::gamecube::GameCube, _instr: GcDspExt) {}

#[inline(always)]
pub fn ext_addr<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    let r = instr.r_6_7() as usize;
    match OP {
        OP_EXT_DR => ctx.dsp.registers.ar[r] = ctx.dsp.registers.ar[r].wrapping_sub(1),
        OP_EXT_IR => ctx.dsp.registers.ar[r] = ctx.dsp.registers.ar[r].wrapping_add(1),
        OP_EXT_NR => ctx.dsp.registers.ar[r] = ctx.dsp.registers.ar[r].wrapping_add(ctx.dsp.registers.ix[r]),
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn ext_mv(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    let d = instr.d_4_5();
    let s = instr.s_6_7();
    let value = ctx.dsp.registers.read::<true>(reg::AC0L + s);
    ctx.dsp.registers.write::<false>(reg::AX0L + d, value);
}

#[inline(always)]
pub fn ext_store<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    let s = instr.s_3_4();
    let d = instr.d_6_7() as usize;
    let value = ctx.dsp.registers.read::<true>(reg::AC0L + s);
    ctx.dsp.write_dmem(ctx.dsp.registers.ar[d], value);
    match OP {
        OP_EXT_S => ctx.dsp.registers.ar[d] = ctx.dsp.registers.ar[d].wrapping_add(1),
        OP_EXT_SN => ctx.dsp.registers.ar[d] = ctx.dsp.registers.ar[d].wrapping_add(ctx.dsp.registers.ix[d]),
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn ext_load<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    let d = instr.d_2_4();
    let s = instr.s_6_7() as usize;
    let value = ctx.dsp.read_dmem(ctx.dsp.registers.ar[s]);
    ctx.dsp.registers.write::<false>(reg::AX0L + d, value);
    match OP {
        OP_EXT_L => ctx.dsp.registers.ar[s] = ctx.dsp.registers.ar[s].wrapping_add(1),
        OP_EXT_LN => ctx.dsp.registers.ar[s] = ctx.dsp.registers.ar[s].wrapping_add(ctx.dsp.registers.ix[s]),
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn ext_load_store<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    // 'LS: load from $ar0 into $(AX0L+D), store $acS.m to $ar3
    // 'SL: load from $ar3 into $(AX0L+D), store $acS.m to $ar0
    let store_val = ctx.dsp.registers.read::<true>(reg::AC0M + instr.s_7_7()); // $acS.m
    let is_sl = matches!(OP, OP_EXT_SL | OP_EXT_SLN | OP_EXT_SLM | OP_EXT_SLNM);
    let (load_ar, store_ar) = if is_sl { (3, 0) } else { (0, 3) };
    let load_val = ctx.dsp.read_dmem(ctx.dsp.registers.ar[load_ar]);
    ctx.dsp.registers.write::<false>(reg::AX0L + instr.d_2_3(), load_val);
    ctx.dsp.write_dmem(ctx.dsp.registers.ar[store_ar], store_val);
    // Post-modify: N = ar0 += ix0, M = ar3 += ix3
    match OP {
        OP_EXT_LS | OP_EXT_SL => {
            ctx.dsp.registers.ar[0] = ctx.dsp.registers.ar[0].wrapping_add(1);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.ar[3].wrapping_add(1);
        }
        OP_EXT_LSN | OP_EXT_SLN => {
            ctx.dsp.registers.ar[0] = ctx.dsp.registers.ar[0].wrapping_add(ctx.dsp.registers.ix[0]);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.ar[3].wrapping_add(1);
        }
        OP_EXT_LSM | OP_EXT_SLM => {
            ctx.dsp.registers.ar[0] = ctx.dsp.registers.ar[0].wrapping_add(1);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.ar[3].wrapping_add(ctx.dsp.registers.ix[3]);
        }
        OP_EXT_LSNM | OP_EXT_SLNM => {
            ctx.dsp.registers.ar[0] = ctx.dsp.registers.ar[0].wrapping_add(ctx.dsp.registers.ix[0]);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.ar[3].wrapping_add(ctx.dsp.registers.ix[3]);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn ext_ld<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    let d = instr.d_2_2();
    let r = instr.r_3_3();
    let s = instr.s_6_7() as usize;
    let val_d = ctx.dsp.read_dmem(ctx.dsp.registers.ar[s]);
    ctx.dsp.registers.write::<false>(reg::AX0L + d * 2, val_d);
    let val_r = ctx.dsp.read_dmem(ctx.dsp.registers.ar[3]);
    ctx.dsp.registers.write::<false>(reg::AX0L + 1 + r * 2, val_r);
    match OP {
        OP_EXT_LD_00 => {
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.ar[s].wrapping_add(1);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.ar[3].wrapping_add(1);
        }
        OP_EXT_LDN_01 => {
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.ar[s].wrapping_add(ctx.dsp.registers.ix[s]);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.ar[3].wrapping_add(1);
        }
        OP_EXT_LDM_10 => {
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.ar[s].wrapping_add(1);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.ar[3].wrapping_add(ctx.dsp.registers.ix[3]);
        }
        OP_EXT_LDNM_11 => {
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.ar[s].wrapping_add(ctx.dsp.registers.ix[s]);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.ar[3].wrapping_add(ctx.dsp.registers.ix[3]);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub fn ext_ldax<const OP: u32>(ctx: &mut crate::gamecube::GameCube, instr: GcDspExt) {
    let r = instr.r_3_3() as usize;
    let s = instr.s_2_2() as usize;
    ctx.dsp.registers.axh[r] = ctx.dsp.read_dmem(ctx.dsp.registers.ar[s]);
    ctx.dsp.registers.ax[r] = ctx.dsp.read_dmem(ctx.dsp.registers.ar[3]);
    match OP {
        OP_EXT_LDAX => {
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.ar[s].wrapping_add(1);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.ar[3].wrapping_add(1);
        }
        OP_EXT_LDAXN => {
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.ar[s].wrapping_add(ctx.dsp.registers.ix[s]);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.ar[3].wrapping_add(1);
        }
        OP_EXT_LDAXM => {
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.ar[s].wrapping_add(1);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.ar[3].wrapping_add(ctx.dsp.registers.ix[3]);
        }
        OP_EXT_LDAXNM => {
            ctx.dsp.registers.ar[s] = ctx.dsp.registers.ar[s].wrapping_add(ctx.dsp.registers.ix[s]);
            ctx.dsp.registers.ar[3] = ctx.dsp.registers.ar[3].wrapping_add(ctx.dsp.registers.ix[3]);
        }
        _ => unreachable!(),
    }
}
