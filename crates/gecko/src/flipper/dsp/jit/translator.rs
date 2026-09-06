use cranelift_codegen::Context;
use cranelift_codegen::ir::{AbiParam, BlockArg, InstBuilder, Signature, Value, types};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Module};

use super::block::{BlockSpec, TermKind};
#[derive(Clone, Copy)]
pub struct ExternFuncs {
    pub read_dmem: FuncId,
    pub write_dmem: FuncId,
    pub read_reg_full: FuncId,
    pub write_reg_full: FuncId,
}

fn szat_liveness(spec: &BlockSpec, loop_end_table: &[u8; 0x10000]) -> Vec<bool> {
    use disasm::dsp::GcDspInstruction as I;
    let mut live = true;
    let mut needed = vec![true; spec.instrs.len()];
    let is_flag_independent_register = |reg: u8| reg != 19 && !(12..=15).contains(&reg);

    for (idx, entry) in spec.instrs.iter().enumerate().rev() {
        if spec.unrolled_loop_start.is_none_or(|start| idx < start) && loop_end_table[entry.pc as usize] != 0 {
            live = true;
        }

        let bytes = [
            (entry.raw >> 8) as u8,
            entry.raw as u8,
            (entry.raw >> 24) as u8,
            (entry.raw >> 16) as u8,
        ];
        let Some((insn, _)) = I::decode(&bytes) else {
            live = true;

            continue;
        };

        match insn {
            I::Add { .. }
            | I::Addax { .. }
            | I::Addr { .. }
            | I::Addp { .. }
            | I::Sub { .. }
            | I::Subax { .. }
            | I::Subr { .. }
            | I::Subp { .. }
            | I::Neg { .. }
            | I::Movp { .. }
            | I::Movpz { .. }
            | I::Tst { .. }
            | I::Clr { .. }
            | I::Lsl16 { .. }
            | I::Asr16 { .. }
            | I::Lsr16 { .. }
            | I::Mulac { .. }
            | I::Mulmv { .. }
            | I::Mulmvz { .. }
            | I::Mulxac { .. }
            | I::Mulxmv { .. }
            | I::Mulxmvz { .. }
            | I::Mulcac { .. }
            | I::Mulcmv { .. }
            | I::Mulcmvz { .. } => {
                needed[idx] = live;
                live = false;
            }
            I::Nop
            | I::Dar { .. }
            | I::Iar { .. }
            | I::Addarn { .. }
            | I::Subarn { .. }
            | I::Mul { .. }
            | I::Mulx { .. }
            | I::Mulc { .. }
            | I::Nx0 { .. }
            | I::Nx1 { .. }
            | I::M0 { .. }
            | I::M2 { .. }
            | I::Set15 { .. }
            | I::Clr15 { .. }
            | I::Set16 { .. }
            | I::Set40 { .. } => {}
            I::Lri { rd, .. } | I::Lr { rd, .. } if is_flag_independent_register(rd) => {}
            I::Sr { rs, .. } if is_flag_independent_register(rs) => {}
            I::Lrr { d, .. } | I::Lrrd { d, .. } | I::Lrri { d, .. } | I::Lrrn { d, .. }
                if is_flag_independent_register(d) => {}
            I::Srr { s, .. } | I::Srrd { s, .. } | I::Srri { s, .. } | I::Srrn { s, .. }
                if is_flag_independent_register(s) => {}
            I::Mrr { dst, src } if is_flag_independent_register(dst) && is_flag_independent_register(src) => {}
            _ => live = true,
        }
    }

    needed
}

pub fn translate(
    ctx: &mut Context,
    builder_ctx: &mut FunctionBuilderContext,
    module: &mut JITModule,
    extern_funcs: &ExternFuncs,
    spec: &BlockSpec,
    block_lookup_table_addr: i64,
    entry_counter_addr: Option<usize>,
    loop_end_table: &[u8; 0x10000],
) {
    let pointer_type = module.target_config().pointer_type();

    let mut builder = FunctionBuilder::new(&mut ctx.func, builder_ctx);
    let entry = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    builder.seal_block(entry);

    let ctx_ptr: Value = builder.block_params(entry)[0];

    use crate::flipper::dsp::core::stack::DspStack;
    use crate::flipper::dsp::instruction::Instruction;
    use crate::flipper::dsp::jit::{jit_lut, translate as t};
    use cranelift_codegen::ir::MemFlagsData;
    let nia_offset = super::abi::dsp_nia_offset_max() as i32;
    let pc_offset = super::abi::dsp_pc_offset_max() as i32;
    let loop_addr_ptr_offset = super::abi::dsp_loop_addr_ptr_offset_max() as i32;
    let loop_addr_data_offset = (super::abi::dsp_loop_addr_offset() + DspStack::<32>::data_offset()) as i32;
    let loop_counter_ptr_offset = (super::abi::dsp_loop_counter_offset() + DspStack::<32>::ptr_offset()) as i32;
    let loop_counter_data_offset = (super::abi::dsp_loop_counter_offset() + DspStack::<32>::data_offset()) as i32;
    let call_stack_ptr_offset = (super::abi::dsp_call_stack_offset() + DspStack::<32>::ptr_offset()) as i32;
    let call_stack_data_offset = (super::abi::dsp_call_stack_offset() + DspStack::<32>::data_offset()) as i32;

    if let Some(addr) = entry_counter_addr {
        let slot_v = builder.ins().iconst(types::I64, addr as i64);
        let cur = builder.ins().load(types::I64, MemFlagsData::trusted(), slot_v, 0);
        let next = builder.ins().iadd_imm(cur, 1);
        builder.ins().store(MemFlagsData::trusted(), next, slot_v, 0);
    }

    let block_sig_ref = builder.import_signature(block_signature(pointer_type));
    let mut reg_cache = t::DspRegCache::new(&mut builder, ctx_ptr);
    let szat_needed = szat_liveness(spec, loop_end_table);

    for (idx, entry) in spec.instrs.iter().enumerate() {
        let natural_nia = entry.pc.wrapping_add(entry.size as u16);
        let is_last = idx + 1 == spec.instrs.len();
        let dynamic_nia = is_last && spec.terminator != TermKind::LengthLimit;
        let marked_loop_end =
            spec.unrolled_loop_start.is_none_or(|start| idx < start) && loop_end_table[entry.pc as usize] != 0;

        if is_last || marked_loop_end {
            let nia_v = builder.ins().iconst(types::I16, natural_nia as i64);
            builder.ins().store(MemFlagsData::trusted(), nia_v, ctx_ptr, nia_offset);
        }

        let primary = (entry.raw & 0xFFFF) as u16;
        let has_ext = ((primary >> 12) & 0xF) >= 3;
        let ext_byte = has_ext.then_some({
            if ((primary >> 12) & 0xF) == 3 {
                primary & 0x7F
            } else {
                primary & 0xFF
            }
        });
        let ext_ac_source =
            ext_byte.and_then(|ext| emit_ext_ac_source_value(&mut builder, ctx_ptr, &reg_cache, ext as u8));
        if let Some((source, value)) = ext_ac_source {
            let cache_off = super::abi::dsp_ext_ac_cache_base_offset() as i32 + source as i32 * 2;
            builder.ins().store(MemFlagsData::trusted(), value, ctx_ptr, cache_off);
        }

        let mut tctx = t::TranslatorCtx {
            builder: &mut builder,
            module,
            extern_funcs: *extern_funcs,
            sys_ptr: ctx_ptr,
            reg_cache: &mut reg_cache,
            pc: entry.pc,
            size: entry.size,
            szat_needed: szat_needed[idx],
        };
        jit_lut::dispatch(&mut tctx, Instruction(entry.raw));

        if has_ext {
            let mut tctx2 = t::TranslatorCtx {
                builder: &mut builder,
                module,
                extern_funcs: *extern_funcs,
                sys_ptr: ctx_ptr,
                reg_cache: &mut reg_cache,
                pc: entry.pc,
                size: entry.size,
                szat_needed: szat_needed[idx],
            };
            jit_lut::dispatch_gc_dsp_ext(
                &mut tctx2,
                crate::flipper::dsp::instruction::GcDspExt(ext_byte.unwrap() as u8),
            );
        }

        if !dynamic_nia && !marked_loop_end {
            if is_last {
                let pc_v = builder.ins().iconst(types::I16, natural_nia as i64);
                builder.ins().store(MemFlagsData::trusted(), pc_v, ctx_ptr, pc_offset);
            }
            continue;
        }

        let loop_ptr = builder
            .ins()
            .load(types::I8, MemFlagsData::trusted(), ctx_ptr, loop_addr_ptr_offset);
        let in_loop = builder
            .ins()
            .icmp_imm(cranelift_codegen::ir::condcodes::IntCC::NotEqual, loop_ptr, 0);
        let slow_block = builder.create_block();
        let fast_block = builder.create_block();
        let continue_block = builder.create_block();
        builder.ins().brif(in_loop, slow_block, &[], fast_block, &[]);

        builder.switch_to_block(fast_block);
        builder.seal_block(fast_block);
        let nia_v = builder
            .ins()
            .load(types::I16, MemFlagsData::trusted(), ctx_ptr, nia_offset);
        builder.ins().store(MemFlagsData::trusted(), nia_v, ctx_ptr, pc_offset);
        builder.ins().jump(continue_block, &[]);

        builder.switch_to_block(slow_block);
        builder.seal_block(slow_block);
        // Inline loop_tail: when nia matches the active loop's end address,
        // either decrement the counter and jump back to the loop top (from the
        // call stack), or pop all three loop stacks once the count runs out.
        // If the final nia != natural_nia, exit the block early so the chain
        // link can dispatch the correct next block.
        let loopend_block = builder.create_block();
        let cont_loop_block = builder.create_block();
        let pop_block = builder.create_block();
        let finish_block = builder.create_block();
        builder.append_block_param(finish_block, types::I16);

        let nia_cur = builder
            .ins()
            .load(types::I16, MemFlagsData::trusted(), ctx_ptr, nia_offset);
        let la_idx = builder.ins().uextend(types::I64, loop_ptr);
        let la_byte = builder.ins().ishl_imm(la_idx, 1);
        let la_addr = builder.ins().iadd(ctx_ptr, la_byte);
        let la_top = builder
            .ins()
            .load(types::I16, MemFlagsData::trusted(), la_addr, loop_addr_data_offset);
        let at_end = builder
            .ins()
            .icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, nia_cur, la_top);
        builder
            .ins()
            .brif(at_end, loopend_block, &[], finish_block, &[BlockArg::Value(nia_cur)]);

        builder.switch_to_block(loopend_block);
        builder.seal_block(loopend_block);
        let lc_ptr = builder
            .ins()
            .load(types::I8, MemFlagsData::trusted(), ctx_ptr, loop_counter_ptr_offset);
        let lc_idx = builder.ins().uextend(types::I64, lc_ptr);
        let lc_byte = builder.ins().ishl_imm(lc_idx, 1);
        let lc_addr = builder.ins().iadd(ctx_ptr, lc_byte);
        let counter = builder
            .ins()
            .load(types::I16, MemFlagsData::trusted(), lc_addr, loop_counter_data_offset);
        let cnt1 = builder.ins().iadd_imm(counter, -1);
        let nz = builder
            .ins()
            .icmp_imm(cranelift_codegen::ir::condcodes::IntCC::NotEqual, cnt1, 0);
        builder.ins().brif(nz, cont_loop_block, &[], pop_block, &[]);

        builder.switch_to_block(cont_loop_block);
        builder.seal_block(cont_loop_block);
        builder
            .ins()
            .store(MemFlagsData::trusted(), cnt1, lc_addr, loop_counter_data_offset);
        let cs_ptr = builder
            .ins()
            .load(types::I8, MemFlagsData::trusted(), ctx_ptr, call_stack_ptr_offset);
        let cs_idx = builder.ins().uextend(types::I64, cs_ptr);
        let cs_byte = builder.ins().ishl_imm(cs_idx, 1);
        let cs_addr = builder.ins().iadd(ctx_ptr, cs_byte);
        let cs_top = builder
            .ins()
            .load(types::I16, MemFlagsData::trusted(), cs_addr, call_stack_data_offset);
        builder.ins().jump(finish_block, &[BlockArg::Value(cs_top)]);

        builder.switch_to_block(pop_block);
        builder.seal_block(pop_block);
        let lc_dec = builder.ins().iadd_imm(lc_ptr, -1);
        let lc_new = builder.ins().band_imm(lc_dec, 31);
        builder
            .ins()
            .store(MemFlagsData::trusted(), lc_new, ctx_ptr, loop_counter_ptr_offset);
        let la_dec = builder.ins().iadd_imm(loop_ptr, -1);
        let la_new = builder.ins().band_imm(la_dec, 31);
        builder
            .ins()
            .store(MemFlagsData::trusted(), la_new, ctx_ptr, loop_addr_ptr_offset);
        let csp = builder
            .ins()
            .load(types::I8, MemFlagsData::trusted(), ctx_ptr, call_stack_ptr_offset);
        let csp_dec = builder.ins().iadd_imm(csp, -1);
        let csp_new = builder.ins().band_imm(csp_dec, 31);
        builder
            .ins()
            .store(MemFlagsData::trusted(), csp_new, ctx_ptr, call_stack_ptr_offset);
        builder.ins().jump(finish_block, &[BlockArg::Value(nia_cur)]);

        builder.switch_to_block(finish_block);
        builder.seal_block(finish_block);
        let nia_final = builder.block_params(finish_block)[0];
        builder
            .ins()
            .store(MemFlagsData::trusted(), nia_final, ctx_ptr, nia_offset);
        builder
            .ins()
            .store(MemFlagsData::trusted(), nia_final, ctx_ptr, pc_offset);
        let expected = builder.ins().iconst(types::I16, natural_nia as i64);
        let same = builder
            .ins()
            .icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, nia_final, expected);
        let exit_block = builder.create_block();
        builder.ins().brif(same, continue_block, &[], exit_block, &[]);

        builder.switch_to_block(exit_block);
        builder.seal_block(exit_block);
        emit_block_tail_chain(
            &mut builder,
            ctx_ptr,
            block_sig_ref,
            block_lookup_table_addr,
            idx as i64 + 1,
            &reg_cache,
        );

        builder.switch_to_block(continue_block);
        builder.seal_block(continue_block);
    }

    emit_block_tail_chain(
        &mut builder,
        ctx_ptr,
        block_sig_ref,
        block_lookup_table_addr,
        spec.instrs.len() as i64,
        &reg_cache,
    );

    builder.finalize();
}

fn emit_block_tail_chain(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    block_sig_ref: cranelift_codegen::ir::SigRef,
    block_lookup_table_addr: i64,
    block_instr_count: i64,
    reg_cache: &super::translate::DspRegCache,
) {
    use cranelift_codegen::ir::MemFlagsData;
    use cranelift_codegen::ir::condcodes::IntCC;

    let pc_offset = super::abi::dsp_pc_offset_max() as i32;
    let chain_budget_offset = super::abi::dsp_chain_budget_offset() as i32;
    let instr_count_offset = super::abi::dsp_instr_count_offset() as i32;

    reg_cache.flush(builder, ctx_ptr);
    let instr_count = builder
        .ins()
        .load(types::I32, MemFlagsData::trusted(), ctx_ptr, instr_count_offset);
    let new_instr_count = builder.ins().iadd_imm(instr_count, block_instr_count);
    builder
        .ins()
        .store(MemFlagsData::trusted(), new_instr_count, ctx_ptr, instr_count_offset);

    let pc_u16 = builder
        .ins()
        .load(types::I16, MemFlagsData::trusted(), ctx_ptr, pc_offset);
    let pc_u32 = builder.ins().uextend(types::I32, pc_u16);

    let budget = builder
        .ins()
        .load(types::I32, MemFlagsData::trusted(), ctx_ptr, chain_budget_offset);

    let pc_low12 = builder.ins().band_imm(pc_u32, 0xFFF);
    let pc_shifted = builder.ins().ushr_imm(pc_u32, 3);
    let pc_high_bit = builder.ins().band_imm(pc_shifted, 0x1000);
    let idx = builder.ins().bor(pc_low12, pc_high_bit);
    let idx64 = builder.ins().uextend(types::I64, idx);

    let entry_size = std::mem::size_of::<super::DspBlockLookupSlot>() as i64;
    let off64 = builder.ins().imul_imm(idx64, entry_size);
    let table_base = builder.ins().iconst(types::I64, block_lookup_table_addr);
    let slot_addr = builder.ins().iadd(table_base, off64);

    let slot_pc = builder.ins().load(types::I32, MemFlagsData::trusted(), slot_addr, 0);
    let slot_entry = builder.ins().load(types::I64, MemFlagsData::trusted(), slot_addr, 8);

    let pc_match = builder.ins().icmp(IntCC::Equal, slot_pc, pc_u32);
    let entry_nonzero = builder.ins().icmp_imm(IntCC::NotEqual, slot_entry, 0);
    let budget_nonzero = builder.ins().icmp_imm(IntCC::NotEqual, budget, 0);
    let pc_and_entry = builder.ins().band(pc_match, entry_nonzero);
    let ok = builder.ins().band(pc_and_entry, budget_nonzero);

    // Return to the dispatcher before the next instruction when an internal
    // exception is ready
    let pending = builder.ins().load(
        types::I8,
        MemFlagsData::trusted(),
        ctx_ptr,
        super::abi::dsp_pending_exceptions_offset() as i32,
    );
    let status = builder.ins().load(
        types::I16,
        MemFlagsData::trusted(),
        ctx_ptr,
        super::abi::dsp_status_offset() as i32,
    );
    let enabled = builder.ins().band_imm(status, 1 << 9);
    let masked = builder.ins().icmp_imm(IntCC::Equal, enabled, 0);
    let no_pending = builder.ins().icmp_imm(IntCC::Equal, pending, 0);
    let no_exception = builder.ins().bor(masked, no_pending);
    let ok = builder.ins().band(ok, no_exception);

    let chain_block = builder.create_block();
    let return_block = builder.create_block();
    builder.ins().brif(ok, chain_block, &[], return_block, &[]);

    builder.switch_to_block(return_block);
    builder.seal_block(return_block);
    builder.ins().return_(&[pc_u32]);

    builder.switch_to_block(chain_block);
    builder.seal_block(chain_block);
    let new_budget = builder.ins().iadd_imm(budget, -1);
    builder
        .ins()
        .store(MemFlagsData::trusted(), new_budget, ctx_ptr, chain_budget_offset);
    builder
        .ins()
        .return_call_indirect(block_sig_ref, slot_entry, &[ctx_ptr]);
}

fn emit_ext_ac_source_value(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    reg_cache: &super::translate::DspRegCache,
    ext: u8,
) -> Option<(u8, Value)> {
    use cranelift_codegen::ir::condcodes::IntCC;

    let source = match ext {
        0x10..=0x1F => ext & 0x3,
        0x20..=0x3F => (ext >> 3) & 0x3,
        0x80..=0xBF => 4 + (ext & 0x1),
        _ => return None,
    };

    let ac = (source & 1) as usize;
    let mid_off = [super::abi::dsp_ac0_mid_offset(), super::abi::dsp_ac1_mid_offset()][ac] as i32;
    let value = match source {
        0 | 1 => {
            let low_off = [super::abi::dsp_ac0_low_offset(), super::abi::dsp_ac1_low_offset()][ac] as i32;

            reg_cache.load(builder, ctx_ptr, low_off)
        }
        2 | 3 => {
            let high_off = [super::abi::dsp_ac0_high_offset(), super::abi::dsp_ac1_high_offset()][ac] as i32;
            let mid = reg_cache.load(builder, ctx_ptr, mid_off);
            let high = reg_cache.load(builder, ctx_ptr, high_off);
            let status = reg_cache.load(builder, ctx_ptr, super::abi::dsp_status_offset() as i32);
            let sxm_shifted = builder.ins().ushr_imm(status, 14);
            let sxm = builder.ins().band_imm(sxm_shifted, 1);
            let sxm_set = builder.ins().icmp_imm(IntCC::NotEqual, sxm, 0);
            let saturated = emit_saturate_ac_mid(builder, high, mid);

            builder.ins().select(sxm_set, saturated, mid)
        }
        4 | 5 => reg_cache.load(builder, ctx_ptr, mid_off),
        _ => unreachable!(),
    };

    Some((source, value))
}

pub(super) fn emit_saturate_ac_mid(builder: &mut FunctionBuilder, high: Value, mid: Value) -> Value {
    use cranelift_codegen::ir::condcodes::IntCC;

    let sign_ext = builder.ins().sshr_imm(mid, 15);

    let high_eq_signext = builder.ins().icmp(IntCC::Equal, high, sign_ext);

    let high_neg = builder.ins().band_imm(high, 0x80);
    let high_neg_set = builder.ins().icmp_imm(IntCC::NotEqual, high_neg, 0);
    let neg_max = builder.ins().iconst(types::I16, 0x8000_u16 as i64);
    let pos_max = builder.ins().iconst(types::I16, 0x7FFF);
    let sat = builder.ins().select(high_neg_set, neg_max, pos_max);

    builder.ins().select(high_eq_signext, mid, sat)
}

pub fn block_signature(pointer_type: cranelift_codegen::ir::Type) -> Signature {
    let mut sig = Signature::new(CallConv::Tail);
    sig.params.push(AbiParam::new(pointer_type));
    sig.returns.push(AbiParam::new(types::I32));
    sig
}

pub fn dmem_read_signature(pointer_type: cranelift_codegen::ir::Type, host_cc: CallConv) -> Signature {
    let mut sig = Signature::new(host_cc);
    sig.params.push(AbiParam::new(pointer_type));
    sig.params.push(AbiParam::new(types::I32));
    sig.returns.push(AbiParam::new(types::I32));
    sig
}

pub fn dmem_write_signature(pointer_type: cranelift_codegen::ir::Type, host_cc: CallConv) -> Signature {
    let mut sig = Signature::new(host_cc);
    sig.params.push(AbiParam::new(pointer_type));
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(types::I32));
    sig
}

pub fn loop_setup_signature(pointer_type: cranelift_codegen::ir::Type, host_cc: CallConv) -> Signature {
    let mut sig = Signature::new(host_cc);
    sig.params.push(AbiParam::new(pointer_type));
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(types::I32));
    sig
}
