use cranelift_codegen::Context;
use cranelift_codegen::ir::{AbiParam, InstBuilder, Signature, Value, types};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Module};

use super::block::{BlockSpec, TermKind};
#[derive(Clone, Copy)]
pub struct ExternFuncs {
    pub loop_tail: FuncId,
    pub read_dmem: FuncId,
    pub write_dmem: FuncId,
    pub inc_ar: FuncId,
    pub dec_ar: FuncId,
    pub increase_ar: FuncId,
    pub decrease_ar_ix: FuncId,
    pub dynamic_shift: FuncId,
    pub read_imem: FuncId,
    pub call_stack_push: FuncId,
    pub call_stack_pop: FuncId,
    pub data_stack_pop: FuncId,
    pub read_reg_full: FuncId,
    pub write_reg_full: FuncId,
    pub loop_setup: FuncId,
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

    use crate::flipper::dsp::instruction::Instruction;
    use crate::flipper::dsp::jit::{jit_lut, translate as t};
    use cranelift_codegen::ir::MemFlagsData;
    let nia_offset = super::abi::dsp_nia_offset_max() as i32;
    let pc_offset = super::abi::dsp_pc_offset_max() as i32;
    let loop_addr_ptr_offset = super::abi::dsp_loop_addr_ptr_offset_max() as i32;
    let loop_tail_ref = module.declare_func_in_func(extern_funcs.loop_tail, builder.func);

    if let Some(addr) = entry_counter_addr {
        let slot_v = builder.ins().iconst(types::I64, addr as i64);
        let cur = builder.ins().load(types::I64, MemFlagsData::trusted(), slot_v, 0);
        let next = builder.ins().iadd_imm(cur, 1);
        builder.ins().store(MemFlagsData::trusted(), next, slot_v, 0);
    }

    let block_sig_ref = builder.import_signature(block_signature(pointer_type));

    for (idx, entry) in spec.instrs.iter().enumerate() {
        let natural_nia = entry.pc.wrapping_add(entry.size as u16);
        let nia_v = builder.ins().iconst(types::I16, natural_nia as i64);
        builder.ins().store(MemFlagsData::trusted(), nia_v, ctx_ptr, nia_offset);

        let primary = (entry.raw & 0xFFFF) as u16;
        let has_ext = ((primary >> 12) & 0xF) >= 3;
        if has_ext {
            emit_cache_ext_ac_inline(&mut builder, ctx_ptr);
        }

        let mut tctx = t::TranslatorCtx {
            builder: &mut builder,
            module,
            extern_funcs: *extern_funcs,
            sys_ptr: ctx_ptr,
            pc: entry.pc,
            size: entry.size,
        };
        jit_lut::dispatch(&mut tctx, Instruction(entry.raw));

        if has_ext {
            let ext_byte = if ((primary >> 12) & 0xF) == 3 {
                primary & 0x7F
            } else {
                primary & 0xFF
            };

            let mut tctx2 = t::TranslatorCtx {
                builder: &mut builder,
                module,
                extern_funcs: *extern_funcs,
                sys_ptr: ctx_ptr,
                pc: entry.pc,
                size: entry.size,
            };
            jit_lut::dispatch_gc_dsp_ext(&mut tctx2, crate::flipper::dsp::instruction::GcDspExt(ext_byte as u8));
        }

        let is_last = idx + 1 == spec.instrs.len();
        let dynamic_nia = is_last && spec.terminator != TermKind::LengthLimit;
        let marked_loop_end = loop_end_table[entry.pc as usize] != 0;

        if !dynamic_nia && !marked_loop_end {
            let pc_v = builder.ins().iconst(types::I16, natural_nia as i64);
            builder.ins().store(MemFlagsData::trusted(), pc_v, ctx_ptr, pc_offset);
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
        builder.ins().call(loop_tail_ref, &[ctx_ptr]);
        // After loop_tail, PC may have been redirected (jump back from loop iteration).
        // If PC != natural_nia, exit the block early so the chain link can dispatch
        // the correct next block. Otherwise, fall through to the next instruction.
        let pc_after = builder
            .ins()
            .load(types::I16, MemFlagsData::trusted(), ctx_ptr, pc_offset);
        let expected = builder.ins().iconst(types::I16, natural_nia as i64);
        let same = builder
            .ins()
            .icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, pc_after, expected);
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
    );

    builder.finalize();
}

fn emit_block_tail_chain(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    block_sig_ref: cranelift_codegen::ir::SigRef,
    block_lookup_table_addr: i64,
    block_instr_count: i64,
) {
    use cranelift_codegen::ir::MemFlagsData;
    use cranelift_codegen::ir::condcodes::IntCC;

    let pc_offset = super::abi::dsp_pc_offset_max() as i32;
    let chain_budget_offset = super::abi::dsp_chain_budget_offset() as i32;
    let instr_count_offset = super::abi::dsp_instr_count_offset() as i32;

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

fn emit_cache_ext_ac_inline(builder: &mut FunctionBuilder, ctx_ptr: Value) {
    use cranelift_codegen::ir::MemFlagsData;
    use cranelift_codegen::ir::condcodes::IntCC;

    let ac0_low_off = super::abi::dsp_ac0_low_offset() as i32;
    let ac1_low_off = super::abi::dsp_ac1_low_offset() as i32;
    let ac0_mid_off = super::abi::dsp_ac0_mid_offset() as i32;
    let ac1_mid_off = super::abi::dsp_ac1_mid_offset() as i32;
    let ac0_hi_off = super::abi::dsp_ac0_high_offset() as i32;
    let ac1_hi_off = super::abi::dsp_ac1_high_offset() as i32;
    let status_off = super::abi::dsp_status_offset() as i32;
    let cache_base = super::abi::dsp_ext_ac_cache_base_offset() as i32;

    let ac0_low = builder
        .ins()
        .load(types::I16, MemFlagsData::trusted(), ctx_ptr, ac0_low_off);
    let ac1_low = builder
        .ins()
        .load(types::I16, MemFlagsData::trusted(), ctx_ptr, ac1_low_off);
    let ac0_mid = builder
        .ins()
        .load(types::I16, MemFlagsData::trusted(), ctx_ptr, ac0_mid_off);
    let ac1_mid = builder
        .ins()
        .load(types::I16, MemFlagsData::trusted(), ctx_ptr, ac1_mid_off);
    let ac0_hi = builder
        .ins()
        .load(types::I16, MemFlagsData::trusted(), ctx_ptr, ac0_hi_off);
    let ac1_hi = builder
        .ins()
        .load(types::I16, MemFlagsData::trusted(), ctx_ptr, ac1_hi_off);

    builder
        .ins()
        .store(MemFlagsData::trusted(), ac0_low, ctx_ptr, cache_base);
    builder
        .ins()
        .store(MemFlagsData::trusted(), ac1_low, ctx_ptr, cache_base + 2);
    builder
        .ins()
        .store(MemFlagsData::trusted(), ac0_mid, ctx_ptr, cache_base + 8);
    builder
        .ins()
        .store(MemFlagsData::trusted(), ac1_mid, ctx_ptr, cache_base + 10);

    let status = builder
        .ins()
        .load(types::I16, MemFlagsData::trusted(), ctx_ptr, status_off);
    let sxm_shifted = builder.ins().ushr_imm(status, 14);
    let sxm = builder.ins().band_imm(sxm_shifted, 1);
    let sxm_set = builder.ins().icmp_imm(IntCC::NotEqual, sxm, 0);

    let sat0 = emit_saturate_ac_mid(builder, ac0_hi, ac0_mid);
    let sat1 = emit_saturate_ac_mid(builder, ac1_hi, ac1_mid);

    let sel0 = builder.ins().select(sxm_set, sat0, ac0_mid);
    let sel1 = builder.ins().select(sxm_set, sat1, ac1_mid);
    builder
        .ins()
        .store(MemFlagsData::trusted(), sel0, ctx_ptr, cache_base + 4);
    builder
        .ins()
        .store(MemFlagsData::trusted(), sel1, ctx_ptr, cache_base + 6);
}

fn emit_saturate_ac_mid(builder: &mut FunctionBuilder, high: Value, mid: Value) -> Value {
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

pub fn void_thunk_signature(pointer_type: cranelift_codegen::ir::Type, host_cc: CallConv) -> Signature {
    let mut sig = Signature::new(host_cc);
    sig.params.push(AbiParam::new(pointer_type));
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

pub fn ar_unary_signature(pointer_type: cranelift_codegen::ir::Type, host_cc: CallConv) -> Signature {
    let mut sig = Signature::new(host_cc);
    sig.params.push(AbiParam::new(pointer_type));
    sig.params.push(AbiParam::new(types::I32));
    sig.returns.push(AbiParam::new(types::I32));
    sig
}

pub fn ar_binary_signature(pointer_type: cranelift_codegen::ir::Type, host_cc: CallConv) -> Signature {
    let mut sig = Signature::new(host_cc);
    sig.params.push(AbiParam::new(pointer_type));
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(types::I32));
    sig.returns.push(AbiParam::new(types::I32));
    sig
}

pub fn dynamic_shift_signature(pointer_type: cranelift_codegen::ir::Type, host_cc: CallConv) -> Signature {
    let mut sig = Signature::new(host_cc);
    sig.params.push(AbiParam::new(pointer_type));
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(types::I32));
    sig
}

pub fn stack_pop_signature(pointer_type: cranelift_codegen::ir::Type, host_cc: CallConv) -> Signature {
    let mut sig = Signature::new(host_cc);
    sig.params.push(AbiParam::new(pointer_type));
    sig.returns.push(AbiParam::new(types::I32));
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
