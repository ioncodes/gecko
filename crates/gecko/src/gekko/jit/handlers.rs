use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{InstBuilder, MemFlags, types};
use cranelift_frontend::FunctionBuilder;

use crate::gekko::instruction::Instruction;
use crate::gekko::jit::translator::{
    self, AddmeSubfmeKind, AddzeSubfzeKind, CYCLES_PER_INSTR, ImmLogical, JitTranslator, LocalFuncs, LogicalFullOp,
    LogicalOp, MemSize, ShiftKind, TermEmit,
};
use crate::gekko::jit::{abi, lut};
use crate::system::SystemId;

#[inline(always)]
unsafe fn parts<'a>(builder_ptr: usize, local_ptr: usize) -> (&'a mut FunctionBuilder<'a>, &'a LocalFuncs) {
    (unsafe { &mut *(builder_ptr as *mut FunctionBuilder<'a>) }, unsafe {
        &*(local_ptr as *const LocalFuncs)
    })
}

#[inline(always)]
pub fn alu<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, _) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    match OP {
        lut::OP_ADDI => translator::emit_addi::<SYSTEM>(builder, t.ctx_ptr, instr),
        lut::OP_ADDIS => translator::emit_addis::<SYSTEM>(builder, t.ctx_ptr, instr),
        lut::OP_ORI => translator::emit_ori::<SYSTEM>(builder, t.ctx_ptr, instr),
        lut::OP_ORIS => translator::emit_oris::<SYSTEM>(builder, t.ctx_ptr, instr),

        lut::OP_XORI => {
            translator::emit_imm_logical::<SYSTEM>(builder, t.ctx_ptr, instr, ImmLogical::Xor, false);
        }
        lut::OP_XORIS => {
            translator::emit_imm_logical::<SYSTEM>(builder, t.ctx_ptr, instr, ImmLogical::Xor, true);
        }
        lut::OP_ANDI_DOT => {
            let r = translator::emit_imm_logical::<SYSTEM>(builder, t.ctx_ptr, instr, ImmLogical::And, false);
            translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
        }
        lut::OP_ANDIS_DOT => {
            let r = translator::emit_imm_logical::<SYSTEM>(builder, t.ctx_ptr, instr, ImmLogical::And, true);
            translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
        }

        lut::OP_MULLI => translator::emit_mulli::<SYSTEM>(builder, t.ctx_ptr, instr),
        lut::OP_SUBFIC => translator::emit_subfic::<SYSTEM>(builder, t.ctx_ptr, instr),
        lut::OP_ADDIC => translator::emit_addic::<SYSTEM>(builder, t.ctx_ptr, instr, false),
        lut::OP_ADDIC_DOT => translator::emit_addic::<SYSTEM>(builder, t.ctx_ptr, instr, true),

        lut::OP_ADDX => {
            let a = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra());
            let b = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rb());
            let r = translator::emit_add_xform::<SYSTEM>(builder, t.ctx_ptr, instr);

            if instr.oe() {
                let ov = translator::compute_add_overflow(builder, a, b, r);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }
        lut::OP_ADDCX => {
            let a = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra());
            let b = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rb());
            let r = translator::emit_addc_xform::<SYSTEM>(builder, t.ctx_ptr, instr);

            if instr.oe() {
                let ov = translator::compute_add_overflow(builder, a, b, r);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }
        lut::OP_ADDEX => {
            let a = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra());
            let b = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rb());
            let r = translator::emit_adde_xform::<SYSTEM>(builder, t.ctx_ptr, instr);

            if instr.oe() {
                let ov = translator::compute_add_overflow(builder, a, b, r);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }
        lut::OP_ADDZEX => {
            let a = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra());
            let r = translator::emit_addze_subfze::<SYSTEM>(builder, t.ctx_ptr, instr, AddzeSubfzeKind::Addze);

            if instr.oe() {
                let zero = builder.ins().iconst(types::I32, 0);
                let ov = translator::compute_add_overflow(builder, a, zero, r);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }
        lut::OP_ADDMEX => {
            let a = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra());
            let r = translator::emit_addme_subfme::<SYSTEM>(builder, t.ctx_ptr, instr, AddmeSubfmeKind::Addme);

            if instr.oe() {
                let neg_one = builder.ins().iconst(types::I32, -1i64);
                let ov = translator::compute_add_overflow(builder, a, neg_one, r);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }

        lut::OP_SUBFX => {
            let a = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra());
            let b = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rb());
            let r = translator::emit_subf_xform::<SYSTEM>(builder, t.ctx_ptr, instr);

            if instr.oe() {
                let ov = translator::compute_sub_overflow(builder, a, b, r);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }
        lut::OP_SUBFCX => {
            let a = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra());
            let b = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rb());
            let r = translator::emit_subfc_xform::<SYSTEM>(builder, t.ctx_ptr, instr);

            if instr.oe() {
                let ov = translator::compute_sub_overflow(builder, a, b, r);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }
        lut::OP_SUBFEX => {
            let a = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra());
            let b = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rb());
            let r = translator::emit_subfe_xform::<SYSTEM>(builder, t.ctx_ptr, instr);

            if instr.oe() {
                let ov = translator::compute_sub_overflow(builder, a, b, r);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }
        lut::OP_SUBFZEX => {
            let a = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra());
            let r = translator::emit_addze_subfze::<SYSTEM>(builder, t.ctx_ptr, instr, AddzeSubfzeKind::Subfze);

            if instr.oe() {
                let not_a = builder.ins().bxor_imm(a, !0i64);
                let zero = builder.ins().iconst(types::I32, 0);
                let ov = translator::compute_add_overflow(builder, not_a, zero, r);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }
        lut::OP_SUBFMEX => {
            let a = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra());
            let r = translator::emit_addme_subfme::<SYSTEM>(builder, t.ctx_ptr, instr, AddmeSubfmeKind::Subfme);

            if instr.oe() {
                let not_a = builder.ins().bxor_imm(a, !0i64);
                let neg_one = builder.ins().iconst(types::I32, -1i64);
                let ov = translator::compute_add_overflow(builder, not_a, neg_one, r);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }

        lut::OP_NEGX => {
            let a = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra());
            let r = translator::emit_neg::<SYSTEM>(builder, t.ctx_ptr, instr);

            if instr.oe() {
                let ov_b = builder.ins().icmp_imm(IntCC::Equal, a, 0x8000_0000u32 as i64);
                let ov = builder.ins().uextend(types::I32, ov_b);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }

        lut::OP_MULLWX => {
            let a = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra());
            let b = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rb());
            let r = translator::emit_mullw::<SYSTEM>(builder, t.ctx_ptr, instr);

            if instr.oe() {
                let a64 = builder.ins().sextend(types::I64, a);
                let b64 = builder.ins().sextend(types::I64, b);
                let full = builder.ins().imul(a64, b64);
                let r_sext = builder.ins().sextend(types::I64, r);
                let neq = builder.ins().icmp(IntCC::NotEqual, full, r_sext);
                let ov = builder.ins().uextend(types::I32, neq);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }
        lut::OP_MULHWX => {
            let r = translator::emit_mulhw::<SYSTEM>(builder, t.ctx_ptr, instr, true);

            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }
        lut::OP_MULHWUX => {
            let r = translator::emit_mulhw::<SYSTEM>(builder, t.ctx_ptr, instr, false);

            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }

        lut::OP_DIVWUX => {
            let b = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rb());
            let r = translator::emit_divwu::<SYSTEM>(builder, t.ctx_ptr, instr);

            if instr.oe() {
                let ov_b = builder.ins().icmp_imm(IntCC::Equal, b, 0);
                let ov = builder.ins().uextend(types::I32, ov_b);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }
        lut::OP_DIVWX => {
            let a = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra());
            let b = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rb());
            let r = translator::emit_divw::<SYSTEM>(builder, t.ctx_ptr, instr);

            if instr.oe() {
                let b_zero = builder.ins().icmp_imm(IntCC::Equal, b, 0);
                let a_min = builder.ins().icmp_imm(IntCC::Equal, a, 0x8000_0000u32 as i64);
                let b_neg1 = builder.ins().icmp_imm(IntCC::Equal, b, -1i64);
                let signed_min_neg1 = builder.ins().band(a_min, b_neg1);
                let any = builder.ins().bor(b_zero, signed_min_neg1);
                let ov = builder.ins().uextend(types::I32, any);
                translator::write_xer_ov_so::<SYSTEM>(builder, t.ctx_ptr, ov);
            }
            if instr.rc() {
                translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
            }
        }

        _ => return,
    }

    t.handled_natively = true;
}

#[inline(always)]
pub fn rotate<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, _) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    let r = match OP {
        lut::OP_RLWIMIX => translator::emit_rlwimi::<SYSTEM>(builder, t.ctx_ptr, instr),
        lut::OP_RLWINMX => translator::emit_rlwinm::<SYSTEM>(builder, t.ctx_ptr, instr),
        lut::OP_RLWNMX => translator::emit_rlwnm::<SYSTEM>(builder, t.ctx_ptr, instr),
        _ => return,
    };

    if instr.rc() {
        translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
    }

    t.handled_natively = true;
}

#[inline(always)]
pub fn logical<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, _) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    let r = match OP {
        lut::OP_ANDX => translator::emit_logical_xform::<SYSTEM>(builder, t.ctx_ptr, instr, LogicalOp::And),
        lut::OP_ORX => translator::emit_logical_xform::<SYSTEM>(builder, t.ctx_ptr, instr, LogicalOp::Or),
        lut::OP_XORX => translator::emit_logical_xform::<SYSTEM>(builder, t.ctx_ptr, instr, LogicalOp::Xor),

        lut::OP_NORX => translator::emit_logical_xform_full::<SYSTEM>(builder, t.ctx_ptr, instr, LogicalFullOp::Nor),
        lut::OP_NANDX => translator::emit_logical_xform_full::<SYSTEM>(builder, t.ctx_ptr, instr, LogicalFullOp::Nand),
        lut::OP_ANDCX => translator::emit_logical_xform_full::<SYSTEM>(builder, t.ctx_ptr, instr, LogicalFullOp::Andc),
        lut::OP_ORCX => translator::emit_logical_xform_full::<SYSTEM>(builder, t.ctx_ptr, instr, LogicalFullOp::Orc),
        lut::OP_EQVX => translator::emit_logical_xform_full::<SYSTEM>(builder, t.ctx_ptr, instr, LogicalFullOp::Eqv),

        lut::OP_SLWX => translator::emit_shift_xform::<SYSTEM>(builder, t.ctx_ptr, instr, ShiftKind::Slw),
        lut::OP_SRWX => translator::emit_shift_xform::<SYSTEM>(builder, t.ctx_ptr, instr, ShiftKind::Srw),
        lut::OP_SRAWX => translator::emit_sraw::<SYSTEM>(builder, t.ctx_ptr, instr),
        lut::OP_SRAWIX => translator::emit_srawi::<SYSTEM>(builder, t.ctx_ptr, instr),

        lut::OP_CNTLZWX => translator::emit_cntlzw::<SYSTEM>(builder, t.ctx_ptr, instr),
        lut::OP_EXTSBX => translator::emit_extend::<SYSTEM>(builder, t.ctx_ptr, instr, types::I8),
        lut::OP_EXTSHX => translator::emit_extend::<SYSTEM>(builder, t.ctx_ptr, instr, types::I16),

        _ => return,
    };

    if instr.rc() {
        translator::emit_update_cr0::<SYSTEM>(builder, t.ctx_ptr, r);
    }

    t.handled_natively = true;
}

#[inline(always)]
pub fn compare<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, _) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    match OP {
        lut::OP_CMPI => translator::emit_cmp_imm::<SYSTEM>(builder, t.ctx_ptr, instr, true),
        lut::OP_CMPLI => translator::emit_cmp_imm::<SYSTEM>(builder, t.ctx_ptr, instr, false),
        lut::OP_CMP => translator::emit_cmp_xform::<SYSTEM>(builder, t.ctx_ptr, instr, true),
        lut::OP_CMPL => translator::emit_cmp_xform::<SYSTEM>(builder, t.ctx_ptr, instr, false),
        _ => return,
    }

    t.handled_natively = true;
}

#[inline(always)]
pub fn cr_ops<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, _) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    match OP {
        lut::OP_MFCR => {
            let cr = translator::cr_load::<SYSTEM>(builder, t.ctx_ptr);
            translator::gpr_store::<SYSTEM>(builder, t.ctx_ptr, instr.rd(), cr);
        }
        lut::OP_MTCRF => {
            let s = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rs());

            let mut mask: u32 = 0;
            for i in 0..8 {
                if (instr.crm() >> (7 - i)) & 1 != 0 {
                    mask |= 0xF << ((7 - i) * 4);
                }
            }

            let cr = translator::cr_load::<SYSTEM>(builder, t.ctx_ptr);
            let kept = builder.ins().band_imm(cr, !mask as i64 as u32 as i64);
            let new_bits = builder.ins().band_imm(s, mask as i64);
            let merged = builder.ins().bor(kept, new_bits);
            translator::cr_store::<SYSTEM>(builder, t.ctx_ptr, merged);
        }

        lut::OP_MCRF => translator::emit_mcrf::<SYSTEM>(builder, t.ctx_ptr, instr),

        lut::OP_CRNOR => translator::emit_cr_bit_op::<SYSTEM>(builder, t.ctx_ptr, instr, 33),
        lut::OP_CRANDC => translator::emit_cr_bit_op::<SYSTEM>(builder, t.ctx_ptr, instr, 129),
        lut::OP_CRXOR => translator::emit_cr_bit_op::<SYSTEM>(builder, t.ctx_ptr, instr, 193),
        lut::OP_CRNAND => translator::emit_cr_bit_op::<SYSTEM>(builder, t.ctx_ptr, instr, 225),
        lut::OP_CRAND => translator::emit_cr_bit_op::<SYSTEM>(builder, t.ctx_ptr, instr, 257),
        lut::OP_CREQV => translator::emit_cr_bit_op::<SYSTEM>(builder, t.ctx_ptr, instr, 289),
        lut::OP_CRORC => translator::emit_cr_bit_op::<SYSTEM>(builder, t.ctx_ptr, instr, 417),
        lut::OP_CROR => translator::emit_cr_bit_op::<SYSTEM>(builder, t.ctx_ptr, instr, 449),

        _ => return,
    }

    t.handled_natively = true;
}

#[inline(always)]
pub fn segment<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, _) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    match OP {
        lut::OP_MFSR => {
            let off = (abi::sr_base_offset::<SYSTEM>() + instr.sr() as usize * 4) as i32;
            let val = builder.ins().load(types::I32, MemFlags::trusted(), t.ctx_ptr, off);
            translator::gpr_store::<SYSTEM>(builder, t.ctx_ptr, instr.rd(), val);
        }
        lut::OP_MTSR => {
            let off = (abi::sr_base_offset::<SYSTEM>() + instr.sr() as usize * 4) as i32;
            let val = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rs());
            builder.ins().store(MemFlags::trusted(), val, t.ctx_ptr, off);
        }
        _ => return,
    }

    t.handled_natively = true;
}

#[inline(always)]
pub fn msr<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, _) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    match OP {
        lut::OP_MFMSR => {
            let msr = builder.ins().load(
                types::I32,
                MemFlags::trusted(),
                t.ctx_ptr,
                abi::msr_offset::<SYSTEM>() as i32,
            );
            translator::gpr_store::<SYSTEM>(builder, t.ctx_ptr, instr.rd(), msr);

            t.handled_natively = true;
        }
        lut::OP_MTMSR if t.is_terminator => {
            let val = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rs());
            builder
                .ins()
                .store(MemFlags::trusted(), val, t.ctx_ptr, abi::msr_offset::<SYSTEM>() as i32);

            t.last_terminator_nia = Some(builder.ins().iconst(types::I32, t.pc.wrapping_add(4) as i64));
            t.handled_natively = true;
        }
        lut::OP_RFI if t.is_terminator => {
            const RFI_MSR_MASK: i64 = 0x87C0_FFFF;

            let msr_off = abi::msr_offset::<SYSTEM>() as i32;
            let srr0_off = abi::spr_field_offset::<SYSTEM>(26).unwrap() as i32;
            let srr1_off = abi::spr_field_offset::<SYSTEM>(27).unwrap() as i32;
            let nia_off = abi::nia_offset::<SYSTEM>() as i32;

            let msr_v = builder.ins().load(types::I32, MemFlags::trusted(), t.ctx_ptr, msr_off);
            let srr1 = builder.ins().load(types::I32, MemFlags::trusted(), t.ctx_ptr, srr1_off);
            let msr_keep = builder.ins().band_imm(msr_v, !RFI_MSR_MASK & 0xFFFF_FFFFi64);
            let srr1_take = builder.ins().band_imm(srr1, RFI_MSR_MASK);
            let new_msr = builder.ins().bor(msr_keep, srr1_take);
            let new_msr_clr = builder.ins().band_imm(new_msr, !0x0004_0000i64 & 0xFFFF_FFFFi64);
            builder
                .ins()
                .store(MemFlags::trusted(), new_msr_clr, t.ctx_ptr, msr_off);

            let srr0_raw = builder.ins().load(types::I32, MemFlags::trusted(), t.ctx_ptr, srr0_off);
            let nia = builder.ins().band_imm(srr0_raw, 0xFFFF_FFFCi64);
            builder.ins().store(MemFlags::trusted(), nia, t.ctx_ptr, nia_off);

            t.last_terminator_nia = Some(nia);
            t.handled_natively = true;
        }
        _ => {}
    }
}

#[inline(always)]
pub fn spr<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    match OP {
        lut::OP_MFSPR => {
            if translator::emit_mfspr::<SYSTEM>(builder, t.ctx_ptr, instr, local) {
                t.handled_natively = true;
            }
        }
        lut::OP_MTSPR => {
            if translator::emit_mtspr::<SYSTEM>(builder, t.ctx_ptr, instr, local) {
                if t.is_terminator {
                    t.last_terminator_nia = Some(builder.ins().iconst(types::I32, t.pc.wrapping_add(4) as i64));
                }
                t.handled_natively = true;
            }
        }
        _ => {}
    }
}

#[inline(always)]
pub fn store_load<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    match OP {
        lut::OP_LWZ => translator::emit_load::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U32, local.read_u32, false),
        lut::OP_LWZU => translator::emit_load::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U32, local.read_u32, true),
        lut::OP_LBZ => translator::emit_load::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U8, local.read_u8, false),
        lut::OP_LBZU => translator::emit_load::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U8, local.read_u8, true),
        lut::OP_LHZ => translator::emit_load::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U16, local.read_u16, false),
        lut::OP_LHZU => translator::emit_load::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U16, local.read_u16, true),
        lut::OP_LHA => translator::emit_lha_d_form::<SYSTEM>(builder, t.ctx_ptr, instr, local.read_u16, false),
        lut::OP_LHAU => translator::emit_lha_d_form::<SYSTEM>(builder, t.ctx_ptr, instr, local.read_u16, true),

        lut::OP_STW => {
            translator::emit_store::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U32, local.write_u32, false)
        }
        lut::OP_STWU => {
            translator::emit_store::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U32, local.write_u32, true)
        }
        lut::OP_STB => translator::emit_store::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U8, local.write_u8, false),
        lut::OP_STBU => translator::emit_store::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U8, local.write_u8, true),
        lut::OP_STH => {
            translator::emit_store::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U16, local.write_u16, false)
        }
        lut::OP_STHU => {
            translator::emit_store::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U16, local.write_u16, true)
        }

        lut::OP_LMW => translator::emit_lmw_stmw::<SYSTEM>(builder, t.ctx_ptr, instr, local, false),
        lut::OP_STMW => translator::emit_lmw_stmw::<SYSTEM>(builder, t.ctx_ptr, instr, local, true),

        lut::OP_LWZX => {
            translator::emit_load_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U32, local.read_u32, false)
        }
        lut::OP_LWZUX => {
            translator::emit_load_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U32, local.read_u32, true)
        }
        lut::OP_LBZX => {
            translator::emit_load_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U8, local.read_u8, false)
        }
        lut::OP_LBZUX => {
            translator::emit_load_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U8, local.read_u8, true)
        }
        lut::OP_LHZX => {
            translator::emit_load_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U16, local.read_u16, false)
        }
        lut::OP_LHZUX => {
            translator::emit_load_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U16, local.read_u16, true)
        }
        lut::OP_LHAX => translator::emit_lha_xform::<SYSTEM>(builder, t.ctx_ptr, instr, local.read_u16, false),
        lut::OP_LHAUX => translator::emit_lha_xform::<SYSTEM>(builder, t.ctx_ptr, instr, local.read_u16, true),

        lut::OP_STWX => {
            translator::emit_store_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U32, local.write_u32, false)
        }
        lut::OP_STWUX => {
            translator::emit_store_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U32, local.write_u32, true)
        }
        lut::OP_STBX => {
            translator::emit_store_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U8, local.write_u8, false)
        }
        lut::OP_STBUX => {
            translator::emit_store_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U8, local.write_u8, true)
        }
        lut::OP_STHX => {
            translator::emit_store_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U16, local.write_u16, false)
        }
        lut::OP_STHUX => {
            translator::emit_store_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U16, local.write_u16, true)
        }

        lut::OP_LWBRX => {
            translator::emit_load_xform_brx::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U32, local.read_u32)
        }
        lut::OP_LHBRX => {
            translator::emit_load_xform_brx::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U16, local.read_u16)
        }
        lut::OP_STWBRX => {
            translator::emit_store_xform_brx::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U32, local.write_u32)
        }
        lut::OP_STHBRX => {
            translator::emit_store_xform_brx::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U16, local.write_u16)
        }

        _ => return,
    }

    t.handled_natively = true;
}

#[inline(always)]
pub fn store_load_fp<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    translator::emit_msr_fp_guard::<SYSTEM>(builder, t.ctx_ptr, instr, t.pc, t.exit_block, local);

    match OP {
        lut::OP_LFS => translator::emit_lfs::<SYSTEM>(builder, t.ctx_ptr, instr, local.read_f32, local.read_u32),
        lut::OP_LFSU => translator::emit_lf_d_form_update::<SYSTEM>(builder, t.ctx_ptr, instr, true, local),
        lut::OP_LFD => translator::emit_lfd::<SYSTEM>(builder, t.ctx_ptr, instr, local.read_f64),
        lut::OP_LFDU => translator::emit_lf_d_form_update::<SYSTEM>(builder, t.ctx_ptr, instr, false, local),

        lut::OP_STFS => translator::emit_stfs::<SYSTEM>(builder, t.ctx_ptr, instr, local.write_f32, local.write_u32),
        lut::OP_STFSU => translator::emit_stf_d_form_update::<SYSTEM>(builder, t.ctx_ptr, instr, true, local),
        lut::OP_STFD => translator::emit_stfd::<SYSTEM>(builder, t.ctx_ptr, instr, local.write_f64),
        lut::OP_STFDU => translator::emit_stf_d_form_update::<SYSTEM>(builder, t.ctx_ptr, instr, false, local),

        lut::OP_LFSX => translator::emit_lfsx::<SYSTEM>(builder, t.ctx_ptr, instr, local.read_f32),
        lut::OP_LFSUX => translator::emit_lfx_update::<SYSTEM>(builder, t.ctx_ptr, instr, true, local),
        lut::OP_LFDX => translator::emit_lfdx::<SYSTEM>(builder, t.ctx_ptr, instr, local.read_f64),
        lut::OP_LFDUX => translator::emit_lfx_update::<SYSTEM>(builder, t.ctx_ptr, instr, false, local),

        lut::OP_STFSX => translator::emit_stfsx::<SYSTEM>(builder, t.ctx_ptr, instr, local.write_f32),
        lut::OP_STFSUX => translator::emit_stfx_update::<SYSTEM>(builder, t.ctx_ptr, instr, true, local),
        lut::OP_STFDX => translator::emit_stfdx::<SYSTEM>(builder, t.ctx_ptr, instr, local.write_f64),
        lut::OP_STFDUX => translator::emit_stfx_update::<SYSTEM>(builder, t.ctx_ptr, instr, false, local),

        lut::OP_STFIWX => translator::emit_stfiwx::<SYSTEM>(builder, t.ctx_ptr, instr, local.write_u32),

        _ => return,
    }

    t.handled_natively = true;
}

#[inline(always)]
pub fn store_load_psq<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    translator::emit_msr_fp_guard::<SYSTEM>(builder, t.ctx_ptr, instr, t.pc, t.exit_block, local);

    match OP {
        lut::OP_PSQ_L => translator::emit_psq_l::<SYSTEM>(builder, t.ctx_ptr, instr, t.pc, t.exit_block, local, false),
        lut::OP_PSQ_LU => translator::emit_psq_l::<SYSTEM>(builder, t.ctx_ptr, instr, t.pc, t.exit_block, local, true),
        lut::OP_PSQ_ST => {
            translator::emit_psq_st::<SYSTEM>(builder, t.ctx_ptr, instr, t.pc, t.exit_block, local, false)
        }
        lut::OP_PSQ_STU => {
            translator::emit_psq_st::<SYSTEM>(builder, t.ctx_ptr, instr, t.pc, t.exit_block, local, true)
        }

        lut::OP_PSQ_LX => translator::emit_psq_x::<SYSTEM>(builder, t.ctx_ptr, instr, local, false, false),
        lut::OP_PSQ_LUX => translator::emit_psq_x::<SYSTEM>(builder, t.ctx_ptr, instr, local, false, true),
        lut::OP_PSQ_STX => translator::emit_psq_x::<SYSTEM>(builder, t.ctx_ptr, instr, local, true, false),
        lut::OP_PSQ_STUX => translator::emit_psq_x::<SYSTEM>(builder, t.ctx_ptr, instr, local, true, true),

        _ => return,
    }

    t.handled_natively = true;
}

#[inline(always)]
pub fn nop<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let _ = (instr, SYSTEM);
    let (builder, _) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    match OP {
        lut::OP_ISYNC if t.is_terminator => {
            t.last_terminator_nia = Some(builder.ins().iconst(types::I32, t.pc.wrapping_add(4) as i64));
            t.handled_natively = true;
        }
        lut::OP_SYNC
        | lut::OP_EIEIO
        | lut::OP_DCBF
        | lut::OP_DCBI
        | lut::OP_DCBST
        | lut::OP_DCBT
        | lut::OP_DCBTST
        | lut::OP_DCBA
        | lut::OP_DCBZ
        | lut::OP_DCBZ_L
        | lut::OP_ICBI
        | lut::OP_TLBIE
        | lut::OP_TLBIA
        | lut::OP_TLBSYNC
        | lut::OP_TLBLD
        | lut::OP_TLBLI => {
            t.handled_natively = true;
        }
        _ => {}
    }
}

#[inline(always)]
pub fn branch<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    if !t.is_terminator {
        return;
    }

    let (builder, _) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    match OP {
        lut::OP_BX => {
            if let Some(slot_addr) = t.chain_slot_addr {
                let target = if instr.aa() {
                    instr.li() as u32
                } else {
                    t.pc.wrapping_add_signed(instr.li())
                };

                if instr.lk() {
                    let lr_val = builder.ins().iconst(types::I32, t.pc.wrapping_add(4) as i64);
                    translator::lr_store::<SYSTEM>(builder, t.ctx_ptr, lr_val);
                }

                translator::emit_cycles_add::<SYSTEM>(builder, t.ctx_ptr, CYCLES_PER_INSTR);

                let target_v = builder.ins().iconst(types::I32, target as i64);
                if instr.lk() {
                    translator::emit_call_or_exit::<SYSTEM>(
                        builder,
                        t.ctx_ptr,
                        slot_addr,
                        t.block_sig_ref,
                        target_v,
                        t.exit_block,
                    );
                } else {
                    translator::emit_chain_or_exit::<SYSTEM>(
                        builder,
                        t.ctx_ptr,
                        slot_addr,
                        t.block_sig_ref,
                        target_v,
                        t.exit_block,
                    );
                }

                t.chained = true;
                t.handled_natively = true;
            } else {
                t.last_terminator_nia = Some(translator::emit_b::<SYSTEM>(builder, t.ctx_ptr, instr, t.pc));
                t.handled_natively = true;
            }
        }

        lut::OP_BCX => {
            match translator::emit_bc_with_chain::<SYSTEM>(
                builder,
                t.ctx_ptr,
                instr,
                t.pc,
                t.exit_block,
                t.chain_slot_addr,
                t.chain_fall_addr,
                t.block_sig_ref,
            ) {
                TermEmit::HandledNia(nia) => {
                    t.last_terminator_nia = Some(nia);
                    t.handled_natively = true;
                }
                TermEmit::HandledChained => {
                    t.chained = true;
                    t.handled_natively = true;
                }
                TermEmit::NotHandled => {}
            }
        }

        lut::OP_BCLRX => {
            match translator::emit_bclr_dispatch::<SYSTEM>(
                builder,
                t.ctx_ptr,
                instr,
                t.pc,
                t.exit_block,
                t.block_sig_ref,
                t.block_lookup_table_addr,
            ) {
                TermEmit::HandledNia(nia) => {
                    t.last_terminator_nia = Some(nia);
                    t.handled_natively = true;
                }
                TermEmit::HandledChained => {
                    t.chained = true;
                    t.handled_natively = true;
                }
                TermEmit::NotHandled => {}
            }
        }

        lut::OP_BCCTRX => {
            match translator::emit_bcctr_dispatch::<SYSTEM>(
                builder,
                t.ctx_ptr,
                instr,
                t.pc,
                t.exit_block,
                t.block_sig_ref,
                t.block_lookup_table_addr,
            ) {
                TermEmit::HandledNia(nia) => {
                    t.last_terminator_nia = Some(nia);
                    t.handled_natively = true;
                }
                TermEmit::HandledChained => {
                    t.chained = true;
                    t.handled_natively = true;
                }
                TermEmit::NotHandled => {}
            }
        }

        _ => {}
    }
}

#[inline(always)]
pub fn fp_ops<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    translator::emit_msr_fp_guard::<SYSTEM>(builder, t.ctx_ptr, instr, t.pc, t.exit_block, local);

    if translator::emit_fp_arith::<OP, SYSTEM>(builder, t.ctx_ptr, instr) {
        if instr.rc() {
            translator::emit_update_cr1_from_fpscr::<SYSTEM>(builder, t.ctx_ptr);
        }
        t.handled_natively = true;
    }
}

#[inline(always)]
pub fn ps_ops<const OP: u32, const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    translator::emit_msr_fp_guard::<SYSTEM>(builder, t.ctx_ptr, instr, t.pc, t.exit_block, local);

    if translator::emit_ps_arith::<OP, SYSTEM>(builder, t.ctx_ptr, instr) {
        if instr.rc() {
            translator::emit_update_cr1_from_fpscr::<SYSTEM>(builder, t.ctx_ptr);
        }
        t.handled_natively = true;
    }
}

#[inline(always)]
pub fn sc<const SYSTEM: SystemId>(t: &mut JitTranslator, _instr: Instruction) {
    if !t.is_terminator {
        return;
    }

    let (builder, _) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    let cia_off = abi::cia_offset::<SYSTEM>() as i32;
    let pc_const = builder.ins().iconst(types::I32, t.pc as i64);
    builder.ins().store(MemFlags::trusted(), pc_const, t.ctx_ptr, cia_off);

    let srr0_value = builder.ins().iconst(types::I32, t.pc.wrapping_add(4) as i64);
    t.last_terminator_nia = Some(translator::emit_exception_dispatch::<SYSTEM>(
        builder,
        t.ctx_ptr,
        srr0_value,
        0,
        0x0000_0C00,
    ));

    t.handled_natively = true;
}

#[inline(always)]
pub fn mcrxr<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, _) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    translator::emit_mcrxr::<SYSTEM>(builder, t.ctx_ptr, instr);

    t.handled_natively = true;
}

#[inline(always)]
pub fn lwarx<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    translator::emit_lwarx::<SYSTEM>(builder, t.ctx_ptr, instr, local);

    t.handled_natively = true;
}

#[inline(always)]
pub fn stwcx_dot<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    translator::emit_stwcx_dot::<SYSTEM>(builder, t.ctx_ptr, instr, local);

    t.handled_natively = true;
}

#[inline(always)]
pub fn lswi<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    let n = if instr.nb() == 0 { 32 } else { instr.nb() as u32 };
    let ea = if instr.ra() == 0 {
        builder.ins().iconst(types::I32, 0)
    } else {
        translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra())
    };

    let rd_v = builder.ins().iconst(types::I32, instr.rd() as i64);
    let n_v = builder.ins().iconst(types::I32, n as i64);
    builder.ins().call(local.do_lswi, &[t.ctx_ptr, ea, rd_v, n_v]);

    t.handled_natively = true;
}

#[inline(always)]
pub fn stswi<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    let n = if instr.nb() == 0 { 32 } else { instr.nb() as u32 };
    let ea = if instr.ra() == 0 {
        builder.ins().iconst(types::I32, 0)
    } else {
        translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.ra())
    };

    let rs_v = builder.ins().iconst(types::I32, instr.rs() as i64);
    let n_v = builder.ins().iconst(types::I32, n as i64);
    builder.ins().call(local.do_stswi, &[t.ctx_ptr, ea, rs_v, n_v]);

    t.handled_natively = true;
}

#[inline(always)]
pub fn lswx<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    let ea = translator::emit_x_form_ea::<SYSTEM>(builder, t.ctx_ptr, instr);
    let rd_v = builder.ins().iconst(types::I32, instr.rd() as i64);
    builder.ins().call(local.do_lswx, &[t.ctx_ptr, ea, rd_v]);

    t.handled_natively = true;
}

#[inline(always)]
pub fn stswx<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    let ea = translator::emit_x_form_ea::<SYSTEM>(builder, t.ctx_ptr, instr);
    let rs_v = builder.ins().iconst(types::I32, instr.rs() as i64);
    builder.ins().call(local.do_stswx, &[t.ctx_ptr, ea, rs_v]);

    t.handled_natively = true;
}

#[inline(always)]
pub fn eciwx<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    translator::emit_load_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U32, local.read_u32, false);

    t.handled_natively = true;
}

#[inline(always)]
pub fn ecowx<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    translator::emit_store_xform::<SYSTEM>(builder, t.ctx_ptr, instr, MemSize::U32, local.write_u32, false);

    t.handled_natively = true;
}

#[inline(always)]
pub fn mfsrin<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, _) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    let b = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rb());
    let idx = builder.ins().ushr_imm(b, 28);
    let idx_masked = builder.ins().band_imm(idx, 0xF);
    let byte_off = builder.ins().ishl_imm(idx_masked, 2);
    let byte_off64 = builder.ins().uextend(types::I64, byte_off);

    let base = builder
        .ins()
        .iadd_imm(t.ctx_ptr, abi::sr_base_offset::<SYSTEM>() as i64);
    let addr = builder.ins().iadd(base, byte_off64);

    let val = builder.ins().load(types::I32, MemFlags::trusted(), addr, 0);
    translator::gpr_store::<SYSTEM>(builder, t.ctx_ptr, instr.rd(), val);

    t.handled_natively = true;
}

#[inline(always)]
pub fn mtsrin<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, _) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    let b = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rb());
    let idx = builder.ins().ushr_imm(b, 28);
    let idx_masked = builder.ins().band_imm(idx, 0xF);
    let byte_off = builder.ins().ishl_imm(idx_masked, 2);
    let byte_off64 = builder.ins().uextend(types::I64, byte_off);

    let base = builder
        .ins()
        .iadd_imm(t.ctx_ptr, abi::sr_base_offset::<SYSTEM>() as i64);
    let addr = builder.ins().iadd(base, byte_off64);

    let val = translator::gpr_load::<SYSTEM>(builder, t.ctx_ptr, instr.rs());
    builder.ins().store(MemFlags::trusted(), val, addr, 0);

    t.handled_natively = true;
}

#[inline(always)]
pub fn mftb<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    let tbr = instr.spr_swapped() as u16;

    if tbr == 268 || tbr == 269 {
        let cycles = builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            t.ctx_ptr,
            abi::cycles_offset::<SYSTEM>() as i32,
        );
        let cycles_div = builder.ins().udiv_imm(cycles, 12);
        let tb_off = builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            t.ctx_ptr,
            abi::timebase_offset_offset::<SYSTEM>() as i32,
        );
        let tb = builder.ins().iadd(cycles_div, tb_off);

        let val = if tbr == 269 {
            let shifted = builder.ins().ushr_imm(tb, 32);
            builder.ins().ireduce(types::I32, shifted)
        } else {
            builder.ins().ireduce(types::I32, tb)
        };

        translator::gpr_store::<SYSTEM>(builder, t.ctx_ptr, instr.rd(), val);
    } else {
        let tbr_v = builder.ins().iconst(types::I32, tbr as i64);
        let call = builder.ins().call(local.read_timebase, &[t.ctx_ptr, tbr_v]);
        let val = builder.inst_results(call)[0];
        translator::gpr_store::<SYSTEM>(builder, t.ctx_ptr, instr.rd(), val);
    }

    t.handled_natively = true;
}

#[inline(always)]
pub fn tw<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    translator::emit_tw::<SYSTEM>(builder, t.ctx_ptr, instr, t.pc, t.exit_block, local);

    t.handled_natively = true;
}

#[inline(always)]
pub fn twi<const SYSTEM: SystemId>(t: &mut JitTranslator, instr: Instruction) {
    let (builder, local) = unsafe { parts(t.builder_ptr, t.local_ptr) };

    translator::emit_twi::<SYSTEM>(builder, t.ctx_ptr, instr, t.pc, t.exit_block, local);

    t.handled_natively = true;
}

#[cold]
#[inline(never)]
pub fn invalid(_t: &mut JitTranslator, _instr: Instruction) {}
