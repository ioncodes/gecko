use cranelift_codegen::Context;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{AliasRegion, AliasRegionData, Block, FuncRef, InstBuilder, MemFlagsData, Value, types};

use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::Module;
use rustc_hash::FxHashMap;

use crate::gekko::instruction::Instruction;
use crate::gekko::jit::block::{BlockSpec, TermKind};
use crate::gekko::jit::idle::{self, IdleClass};
use crate::gekko::jit::{ExternFuncs, abi};
use crate::system::SystemId;

struct GqrCache {
    values: [Option<Value>; 8],
}

fn should_use_native_self_loop(spec: &BlockSpec, idle_class: IdleClass) -> bool {
    matches!(idle_class, IdleClass::None)
        && spec.terminator == TermKind::BranchCond
        && terminator_static_taken_pc(spec) == Some(spec.start_pc)
}

pub(crate) fn register_alias_regions(builder: &mut FunctionBuilder) {
    builder.func.dfg.alias_regions.insert(AliasRegionData {
        user_id: 0,
        description: "vmctx".into(),
    });
    builder.func.dfg.alias_regions.insert(AliasRegionData {
        user_id: 1,
        description: "guest".into(),
    });
}

#[inline(always)]
pub(crate) fn vmctx_flags() -> MemFlagsData {
    MemFlagsData::trusted().with_alias_region(Some(AliasRegion::from_u32(0)))
}

#[inline(always)]
pub(crate) fn vmctx_readonly_flags() -> MemFlagsData {
    vmctx_flags().with_readonly()
}

#[inline(always)]
pub(crate) fn heap_flags() -> MemFlagsData {
    MemFlagsData::new().with_alias_region(Some(AliasRegion::from_u32(1)))
}

pub struct ChainContext<'a> {
    pub self_pc: u32,
    pub target_slots: &'a std::cell::RefCell<FxHashMap<u32, usize>>,
    pub block_lookup_table_addr: i64,
}

impl<'a> ChainContext<'a> {
    fn target_for(&self, target_pc: u32) -> Option<i64> {
        let mut slots = self.target_slots.borrow_mut();
        let addr = *slots.entry(target_pc).or_insert_with(|| {
            let slot: &'static mut usize = Box::leak(Box::new(0usize));
            slot as *mut usize as usize
        });
        Some(addr as i64)
    }
}

pub(crate) const DEFAULT_CYCLES: i64 = crate::gekko::cycles::DEFAULT_CYCLES;

#[allow(dead_code)]
pub(crate) struct BlockEmitContext {
    pub(crate) cause_invalid_opcode: FuncRef,
    pub(crate) advance_to_deadline: FuncRef,
    pub(crate) read_u8: FuncRef,
    pub(crate) read_u16: FuncRef,
    pub(crate) read_u32: FuncRef,
    pub(crate) write_u8: FuncRef,
    pub(crate) write_u16: FuncRef,
    pub(crate) write_u32: FuncRef,
    pub(crate) read_f32: FuncRef,
    pub(crate) read_f64: FuncRef,
    pub(crate) write_f32: FuncRef,
    pub(crate) write_f64: FuncRef,
    pub(crate) write_msr: FuncRef,
    pub(crate) read_spr: FuncRef,
    pub(crate) write_spr: FuncRef,
    pub(crate) read_sr: FuncRef,
    pub(crate) write_sr: FuncRef,
    pub(crate) cause_trap_exception: FuncRef,
    pub(crate) cause_syscall_interrupt: FuncRef,
    pub(crate) do_rfi: FuncRef,
    pub(crate) cause_fp_unavailable: FuncRef,
    pub(crate) set_reservation: FuncRef,
    pub(crate) try_clear_reservation: FuncRef,
    pub(crate) do_lswi: FuncRef,
    pub(crate) do_stswi: FuncRef,
    pub(crate) do_lswx: FuncRef,
    pub(crate) do_stswx: FuncRef,
    pub(crate) do_psq_load: FuncRef,
    pub(crate) do_psq_store: FuncRef,
    pub(crate) read_timebase: FuncRef,
    pub(crate) cause_icbi: FuncRef,
    pub(crate) mark_memory_writeback: FuncRef,
    pub(crate) cause_smc_write: FuncRef,
    pub(crate) dcbz: FuncRef,
    pub(crate) fp_guard_emitted: bool,
    gqr_cache: GqrCache,
}

fn gqr_load<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    emit: &mut BlockEmitContext,
    index: u8,
) -> Value {
    if let Some(value) = emit.gqr_cache.values[index as usize] {
        return value;
    }

    let value = builder.ins().load(
        types::I32,
        vmctx_flags(),
        ctx_ptr,
        abi::gqr_offset::<SYSTEM>(index) as i32,
    );
    emit.gqr_cache.values[index as usize] = Some(value);

    value
}

fn gqr_cache_store(emit: &mut BlockEmitContext, index: u8, value: Value) {
    emit.gqr_cache.values[index as usize] = Some(value);
}

pub struct JitTranslator {
    pub(crate) builder_ptr: usize,
    pub(crate) emit_ctx_ptr: usize,
    pub ctx_ptr: Value,
    pub host_return_pc: Value,
    pub pc: u32,
    pub exit_block: cranelift_codegen::ir::Block,
    pub chain_slot_addr: Option<i64>,
    pub chain_fall_addr: Option<i64>,
    pub block_sig_ref: cranelift_codegen::ir::SigRef,
    pub block_lookup_table_addr: i64,
    pub self_pc: u32,
    pub self_loop_header: Option<Block>,
    pub is_terminator: bool,
    pub chained: bool,
    pub last_terminator_nia: Option<Value>,
    pub handled_natively: bool,
    pub op_cycles: i64,
}

pub fn translate<const SYSTEM: SystemId>(
    ctx: &mut Context,
    builder_ctx: &mut FunctionBuilderContext,
    module: &mut JITModule,
    extern_funcs: &ExternFuncs,
    spec: &BlockSpec,
    gprs: &[u32; 32],
    chain: &ChainContext<'_>,
    entry_counter_addr: Option<usize>,
) {
    let mut local = BlockEmitContext {
        cause_invalid_opcode: module.declare_func_in_func(extern_funcs.cause_invalid_opcode, &mut ctx.func),
        advance_to_deadline: module.declare_func_in_func(extern_funcs.advance_to_deadline, &mut ctx.func),
        read_u8: module.declare_func_in_func(extern_funcs.read_u8, &mut ctx.func),
        read_u16: module.declare_func_in_func(extern_funcs.read_u16, &mut ctx.func),
        read_u32: module.declare_func_in_func(extern_funcs.read_u32, &mut ctx.func),
        write_u8: module.declare_func_in_func(extern_funcs.write_u8, &mut ctx.func),
        write_u16: module.declare_func_in_func(extern_funcs.write_u16, &mut ctx.func),
        write_u32: module.declare_func_in_func(extern_funcs.write_u32, &mut ctx.func),
        read_f32: module.declare_func_in_func(extern_funcs.read_f32, &mut ctx.func),
        read_f64: module.declare_func_in_func(extern_funcs.read_f64, &mut ctx.func),
        write_f32: module.declare_func_in_func(extern_funcs.write_f32, &mut ctx.func),
        write_f64: module.declare_func_in_func(extern_funcs.write_f64, &mut ctx.func),
        write_msr: module.declare_func_in_func(extern_funcs.write_msr, &mut ctx.func),
        read_spr: module.declare_func_in_func(extern_funcs.read_spr, &mut ctx.func),
        write_spr: module.declare_func_in_func(extern_funcs.write_spr, &mut ctx.func),
        read_sr: module.declare_func_in_func(extern_funcs.read_sr, &mut ctx.func),
        write_sr: module.declare_func_in_func(extern_funcs.write_sr, &mut ctx.func),
        cause_trap_exception: module.declare_func_in_func(extern_funcs.cause_trap_exception, &mut ctx.func),
        cause_syscall_interrupt: module.declare_func_in_func(extern_funcs.cause_syscall_interrupt, &mut ctx.func),
        do_rfi: module.declare_func_in_func(extern_funcs.do_rfi, &mut ctx.func),
        cause_fp_unavailable: module.declare_func_in_func(extern_funcs.cause_fp_unavailable, &mut ctx.func),
        set_reservation: module.declare_func_in_func(extern_funcs.set_reservation, &mut ctx.func),
        try_clear_reservation: module.declare_func_in_func(extern_funcs.try_clear_reservation, &mut ctx.func),
        do_lswi: module.declare_func_in_func(extern_funcs.do_lswi, &mut ctx.func),
        do_stswi: module.declare_func_in_func(extern_funcs.do_stswi, &mut ctx.func),
        do_lswx: module.declare_func_in_func(extern_funcs.do_lswx, &mut ctx.func),
        do_stswx: module.declare_func_in_func(extern_funcs.do_stswx, &mut ctx.func),
        do_psq_load: module.declare_func_in_func(extern_funcs.do_psq_load, &mut ctx.func),
        do_psq_store: module.declare_func_in_func(extern_funcs.do_psq_store, &mut ctx.func),
        read_timebase: module.declare_func_in_func(extern_funcs.read_timebase, &mut ctx.func),
        cause_icbi: module.declare_func_in_func(extern_funcs.cause_icbi, &mut ctx.func),
        mark_memory_writeback: module.declare_func_in_func(extern_funcs.mark_memory_writeback, &mut ctx.func),
        cause_smc_write: module.declare_func_in_func(extern_funcs.cause_smc_write, &mut ctx.func),
        dcbz: module.declare_func_in_func(extern_funcs.dcbz, &mut ctx.func),
        fp_guard_emitted: false,
        gqr_cache: GqrCache { values: [None; 8] },
    };

    let idle_class = idle::classify::<SYSTEM>(spec, gprs);

    let (chain_slot_addr, chain_fall_addr): (Option<i64>, Option<i64>) = if matches!(idle_class, IdleClass::None) {
        let taken = terminator_static_taken_pc(spec).and_then(|t| chain.target_for(t));
        let fall = terminator_fall_pc(spec).and_then(|t| chain.target_for(t));
        (taken, fall)
    } else {
        (None, None)
    };

    let sig_clone = ctx.func.signature.clone();
    let block_sig_ref = ctx.func.import_signature(sig_clone);

    let mut builder = FunctionBuilder::new(&mut ctx.func, builder_ctx);
    register_alias_regions(&mut builder);
    let entry = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    builder.seal_block(entry);

    let ctx_ptr: Value = builder.block_params(entry)[0];
    let host_return_pc: Value = builder.block_params(entry)[1];

    let is_self_loop = should_use_native_self_loop(spec, idle_class);

    if let Some(addr) = entry_counter_addr {
        let slot_v = builder.ins().iconst(types::I64, addr as i64);
        let cur = builder.ins().load(types::I64, vmctx_flags(), slot_v, 0);
        let next = builder.ins().iadd_imm(cur, 1);
        builder.ins().store(vmctx_flags(), next, slot_v, 0);
    }

    let exit_block = builder.create_block();
    builder.append_block_param(exit_block, types::I32);
    let dirty_exit_block = builder.create_block();
    builder.append_block_param(dirty_exit_block, types::I32);

    let dirty_off = abi::jit_dirty_offset::<SYSTEM>() as i32;
    let dirty = builder.ins().load(types::I8, vmctx_flags(), ctx_ptr, dirty_off);
    let nz = builder.ins().icmp_imm(IntCC::NotEqual, dirty, 0);
    let alive = builder.create_block();
    let start_nia = builder.ins().iconst(types::I32, spec.start_pc as i64);
    builder
        .ins()
        .brif(nz, dirty_exit_block, &[start_nia.into()], alive, &[]);
    builder.switch_to_block(alive);
    builder.seal_block(alive);

    if let IdleClass::PointerIterLoop { gpr, stride } = idle_class {
        let rb_off = self::gpr_offset::<SYSTEM>(gpr);
        let ctr_off = abi::ctr_offset::<SYSTEM>() as i32;
        let cyc_off = abi::cycles_offset::<SYSTEM>() as i32;

        let rb_val = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, rb_off);
        let ctr_val = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, ctr_off);

        let stride_v = builder.ins().iconst(types::I32, stride as i64);
        let delta = builder.ins().imul(ctr_val, stride_v);
        let new_rb = builder.ins().iadd(rb_val, delta);
        builder.ins().store(vmctx_flags(), new_rb, ctx_ptr, rb_off);

        let zero_v = builder.ins().iconst(types::I32, 0);
        builder.ins().store(vmctx_flags(), zero_v, ctx_ptr, ctr_off);

        let cyc_val = builder.ins().load(types::I64, vmctx_flags(), ctx_ptr, cyc_off);
        let ctr_64 = builder.ins().uextend(types::I64, ctr_val);
        let per_iter_cycles = pointer_iter_loop_cycles_per_iter(&spec.instrs);
        let per_iter = builder.ins().iconst(types::I64, per_iter_cycles);
        let cyc_delta = builder.ins().imul(ctr_64, per_iter);
        let new_cyc = builder.ins().iadd(cyc_val, cyc_delta);
        builder.ins().store(vmctx_flags(), new_cyc, ctx_ptr, cyc_off);

        let final_nia = builder.ins().iconst(types::I32, spec.start_pc.wrapping_add(12) as i64);

        if let Some(slot_addr) = chain_fall_addr {
            emit_chain_or_exit::<SYSTEM>(
                &mut builder,
                ctx_ptr,
                host_return_pc,
                slot_addr,
                block_sig_ref,
                final_nia,
                exit_block,
            );
        } else {
            builder.ins().jump(exit_block, &[final_nia.into()]);
        }

        builder.switch_to_block(exit_block);
        builder.seal_block(exit_block);
        let exit_nia = builder.block_params(exit_block)[0];
        builder.ins().return_(&[exit_nia]);

        builder.switch_to_block(dirty_exit_block);
        builder.seal_block(dirty_exit_block);
        let dirty_nia = builder.block_params(dirty_exit_block)[0];
        builder.ins().return_(&[dirty_nia]);
        builder.finalize();
        return;
    }

    let mut last_terminator_nia: Option<Value> = None;
    let mut chained = false;
    let mut pending_cycles: i64 = 0;

    let self_loop_header = is_self_loop.then(|| {
        let header = builder.create_block();
        builder.ins().jump(header, &[]);
        builder.switch_to_block(header);

        header
    });

    let mut t = JitTranslator {
        builder_ptr: &mut builder as *mut FunctionBuilder<'_> as *mut () as usize,
        emit_ctx_ptr: &mut local as *mut BlockEmitContext as usize,
        ctx_ptr,
        host_return_pc,
        pc: 0,
        exit_block,
        chain_slot_addr,
        chain_fall_addr,
        block_sig_ref,
        block_lookup_table_addr: chain.block_lookup_table_addr,
        self_pc: spec.start_pc,
        self_loop_header,
        is_terminator: false,
        chained: false,
        last_terminator_nia: None,
        handled_natively: false,
        op_cycles: DEFAULT_CYCLES,
    };

    for (i, &word) in spec.instrs.iter().enumerate() {
        let instr = Instruction(word);
        let pc = spec.pc_of(i);
        let is_terminator = i + 1 == spec.instrs.len() && !matches!(spec.terminator, TermKind::LengthCap);

        t.pc = pc;
        t.is_terminator = is_terminator;
        t.handled_natively = false;
        t.op_cycles = DEFAULT_CYCLES;
        let chained_before = t.chained;

        if is_terminator {
            let term_cycles = terminator_op_cycles(spec.terminator, instr);
            let total = pending_cycles + term_cycles;
            emit_cycles_add::<SYSTEM>(&mut builder, ctx_ptr, total);
            pending_cycles = 0;
            t.op_cycles = term_cycles;
        }

        super::dispatch::<SYSTEM>(&mut t, instr);

        if t.handled_natively {
            if !is_terminator && (!t.chained || chained_before) {
                pending_cycles += t.op_cycles;
            }

            if t.chained && !chained_before {
                chained = true;
            }

            if let Some(nia) = t.last_terminator_nia.take() {
                last_terminator_nia = Some(nia);
            }
        } else {
            if pending_cycles > 0 {
                emit_cycles_add::<SYSTEM>(&mut builder, ctx_ptr, pending_cycles);
                pending_cycles = 0;
            }

            let pc_const = builder.ins().iconst(types::I32, pc as i64);
            let raw_const = builder.ins().iconst(types::I32, instr.0 as i64);
            let call = builder
                .ins()
                .call(local.cause_invalid_opcode, &[ctx_ptr, raw_const, pc_const]);
            let nia = builder.inst_results(call)[0];

            emit_cycles_add::<SYSTEM>(&mut builder, ctx_ptr, DEFAULT_CYCLES);

            if is_terminator {
                last_terminator_nia = Some(nia);
            } else {
                let expected = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);
                let is_seq = builder.ins().icmp(IntCC::Equal, nia, expected);
                let cont = builder.create_block();

                builder.ins().brif(is_seq, cont, &[], exit_block, &[nia.into()]);
                builder.switch_to_block(cont);
                builder.seal_block(cont);
            }
        }
    }
    drop(t);

    if let Some(header) = self_loop_header {
        builder.seal_block(header);
    }

    if pending_cycles > 0 {
        self::emit_cycles_add::<SYSTEM>(&mut builder, ctx_ptr, pending_cycles);
    }

    if !chained {
        let final_nia = match spec.terminator {
            TermKind::LengthCap => builder.ins().iconst(types::I32, spec.end_pc() as i64),
            _ => last_terminator_nia.unwrap_or_else(|| {
                let nia_off = abi::nia_offset::<SYSTEM>() as i64;
                let nia_addr = builder.ins().iadd_imm(ctx_ptr, nia_off);
                builder.ins().load(types::I32, vmctx_flags(), nia_addr, 0)
            }),
        };

        match idle_class {
            IdleClass::None | IdleClass::PointerIterLoop { .. } => {}
            IdleClass::BranchToSelf => {
                builder.ins().call(local.advance_to_deadline, &[ctx_ptr]);
            }
            IdleClass::PollingLoop => {
                let taken_pc =
                    terminator_static_taken_pc(spec).expect("polling-loop classifier requires a static branch target");
                let taken = builder.ins().icmp_imm(IntCC::Equal, final_nia, taken_pc as i64);
                let advance = builder.create_block();
                let resume = builder.create_block();
                builder.ins().brif(taken, advance, &[], resume, &[]);
                builder.switch_to_block(advance);
                builder.seal_block(advance);
                builder.ins().call(local.advance_to_deadline, &[ctx_ptr]);
                builder.ins().jump(resume, &[]);
                builder.switch_to_block(resume);
                builder.seal_block(resume);
            }
        }

        if let Some(slot_addr) = chain_slot_addr {
            emit_chain_or_exit::<SYSTEM>(
                &mut builder,
                ctx_ptr,
                host_return_pc,
                slot_addr,
                block_sig_ref,
                final_nia,
                exit_block,
            );
        } else {
            builder.ins().jump(exit_block, &[final_nia.into()]);
        }
    }

    builder.switch_to_block(exit_block);
    builder.seal_block(exit_block);

    let exit_nia = builder.block_params(exit_block)[0];
    builder.ins().return_(&[exit_nia]);

    builder.switch_to_block(dirty_exit_block);
    builder.seal_block(dirty_exit_block);
    let dirty_nia = builder.block_params(dirty_exit_block)[0];
    builder.ins().return_(&[dirty_nia]);

    builder.finalize();
}

pub(crate) fn emit_add_xform<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    let r = builder.ins().iadd(a, b);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
    r
}

pub(crate) fn emit_subf_xform<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    let r = builder.ins().isub(b, a);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
    r
}

pub(crate) fn emit_neg<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let r = builder.ins().ineg(a);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
    r
}

pub(crate) fn emit_mullw<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    let r = builder.ins().imul(a, b);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
    r
}

pub(crate) fn emit_divwu<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    let zero = builder.ins().iconst(types::I32, 0);

    let div_block = builder.create_block();
    let zero_block = builder.create_block();
    builder.set_cold_block(zero_block);
    let merge = builder.create_block();
    builder.append_block_param(merge, types::I32);

    let is_zero = builder.ins().icmp_imm(IntCC::Equal, b, 0);
    builder.ins().brif(is_zero, zero_block, &[], div_block, &[]);

    builder.switch_to_block(div_block);
    builder.seal_block(div_block);
    let q = builder.ins().udiv(a, b);
    builder.ins().jump(merge, &[q.into()]);

    builder.switch_to_block(zero_block);
    builder.seal_block(zero_block);
    builder.ins().jump(merge, &[zero.into()]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
    let r = builder.block_params(merge)[0];
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
    r
}

pub(crate) fn emit_divw<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());

    let div_block = builder.create_block();
    let bad_block = builder.create_block();
    builder.set_cold_block(bad_block);
    let merge = builder.create_block();
    builder.append_block_param(merge, types::I32);

    let is_zero = builder.ins().icmp_imm(IntCC::Equal, b, 0);
    let a_is_min = builder.ins().icmp_imm(IntCC::Equal, a, i32::MIN as i64);
    let b_is_neg1 = builder.ins().icmp_imm(IntCC::Equal, b, -1i64);
    let overflow = builder.ins().band(a_is_min, b_is_neg1);
    let bad = builder.ins().bor(is_zero, overflow);

    builder.ins().brif(bad, bad_block, &[], div_block, &[]);

    builder.switch_to_block(div_block);
    builder.seal_block(div_block);
    let q = builder.ins().sdiv(a, b);
    builder.ins().jump(merge, &[q.into()]);

    builder.switch_to_block(bad_block);
    builder.seal_block(bad_block);
    let neg = builder.ins().icmp_imm(IntCC::SignedLessThan, a, 0);
    let neg_one = builder.ins().iconst(types::I32, 0xFFFF_FFFFu32 as i64);
    let zero = builder.ins().iconst(types::I32, 0);
    let result_bad = builder.ins().select(neg, neg_one, zero);
    builder.ins().jump(merge, &[result_bad.into()]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
    let r = builder.block_params(merge)[0];
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
    r
}

pub(crate) fn emit_mulhw<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    signed: bool,
) -> Value {
    let a32 = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b32 = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    let (a64, b64) = if signed {
        (
            builder.ins().sextend(types::I64, a32),
            builder.ins().sextend(types::I64, b32),
        )
    } else {
        (
            builder.ins().uextend(types::I64, a32),
            builder.ins().uextend(types::I64, b32),
        )
    };

    let prod = builder.ins().imul(a64, b64);
    let high = builder.ins().ushr_imm(prod, 32);

    let r = builder.ins().ireduce(types::I32, high);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);

    r
}

pub(crate) fn emit_addic<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    rc: bool,
) {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let imm_v = builder.ins().iconst(types::I32, instr.simm() as i64);
    let (sum, carry) = builder.ins().uadd_overflow(a, imm_v);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), sum);

    let carry32 = builder.ins().uextend(types::I32, carry);
    write_xer_carry::<SYSTEM>(builder, ctx_ptr, carry32);

    if rc {
        emit_update_cr0::<SYSTEM>(builder, ctx_ptr, sum);
    }
}

pub(crate) fn emit_subfic<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, instr: Instruction) {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let imm_v = builder.ins().iconst(types::I32, instr.simm() as i64);
    let res = builder.ins().isub(imm_v, a);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), res);

    let carry_b = builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, imm_v, a);
    let carry32 = builder.ins().uextend(types::I32, carry_b);
    write_xer_carry::<SYSTEM>(builder, ctx_ptr, carry32);
}

pub(crate) fn write_xer_carry<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, carry: Value) {
    let xer = xer_load::<SYSTEM>(builder, ctx_ptr);
    let cleared = builder.ins().band_imm(xer, !0x2000_0000u32 as i64);
    let positioned = builder.ins().ishl_imm(carry, 29);
    let merged = builder.ins().bor(cleared, positioned);
    let off = abi::xer_offset::<SYSTEM>() as i32;
    builder.ins().store(vmctx_flags(), merged, ctx_ptr, off);
}

pub(crate) fn write_xer_ov_so<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, ov: Value) {
    let xer = xer_load::<SYSTEM>(builder, ctx_ptr);

    let cleared_ov = builder.ins().band_imm(xer, !0x4000_0000u32 as i64);
    let ov_pos = builder.ins().ishl_imm(ov, 30);
    let with_ov = builder.ins().bor(cleared_ov, ov_pos);

    let so_pos = builder.ins().ishl_imm(ov, 31);
    let merged = builder.ins().bor(with_ov, so_pos);
    let off = abi::xer_offset::<SYSTEM>() as i32;
    builder.ins().store(vmctx_flags(), merged, ctx_ptr, off);
}

pub(crate) fn compute_add_overflow(builder: &mut FunctionBuilder, a: Value, b: Value, r: Value) -> Value {
    let xor_a = builder.ins().bxor(a, r);
    let xor_b = builder.ins().bxor(b, r);
    let and = builder.ins().band(xor_a, xor_b);
    let shifted = builder.ins().ushr_imm(and, 31);
    builder.ins().band_imm(shifted, 1)
}

pub(crate) fn compute_sub_overflow(builder: &mut FunctionBuilder, a: Value, b: Value, r: Value) -> Value {
    let xor_ab = builder.ins().bxor(b, a);
    let xor_br = builder.ins().bxor(b, r);
    let and = builder.ins().band(xor_ab, xor_br);
    let shifted = builder.ins().ushr_imm(and, 31);
    builder.ins().band_imm(shifted, 1)
}

pub(crate) fn emit_addc_xform<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    let (sum, carry) = builder.ins().uadd_overflow(a, b);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), sum);
    let carry32 = builder.ins().uextend(types::I32, carry);
    write_xer_carry::<SYSTEM>(builder, ctx_ptr, carry32);
    sum
}

pub(crate) fn emit_subfc_xform<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    let res = builder.ins().isub(b, a);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), res);

    let carry_b = builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, b, a);
    let carry32 = builder.ins().uextend(types::I32, carry_b);
    write_xer_carry::<SYSTEM>(builder, ctx_ptr, carry32);

    res
}

pub(crate) fn emit_twi<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    pc: u32,
    exit_block: Block,
    local: &BlockEmitContext,
) {
    let to = (instr.bo() as u32) as u32;
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = builder.ins().iconst(types::I32, instr.simm() as i64);
    emit_trap_check::<SYSTEM>(builder, ctx_ptr, a, b, to, pc, exit_block, local);
}

pub(crate) fn emit_tw<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    pc: u32,
    exit_block: Block,
    local: &BlockEmitContext,
) {
    let to = (instr.bo() as u32) as u32;
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    emit_trap_check::<SYSTEM>(builder, ctx_ptr, a, b, to, pc, exit_block, local);
}

pub(crate) fn emit_trap_check<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    a: Value,
    b: Value,
    to: u32,
    pc: u32,
    exit_block: Block,
    local: &BlockEmitContext,
) {
    let _ = local;
    let mut cond: Option<Value> = None;
    let or_in = |builder: &mut FunctionBuilder, c: Value, cond: &mut Option<Value>| match *cond {
        Some(prev) => *cond = Some(builder.ins().bor(prev, c)),
        None => *cond = Some(c),
    };

    if to & 0x10 != 0 {
        let c = builder.ins().icmp(IntCC::SignedLessThan, a, b);
        or_in(builder, c, &mut cond);
    }

    if to & 0x08 != 0 {
        let c = builder.ins().icmp(IntCC::SignedGreaterThan, a, b);
        or_in(builder, c, &mut cond);
    }

    if to & 0x04 != 0 {
        let c = builder.ins().icmp(IntCC::Equal, a, b);
        or_in(builder, c, &mut cond);
    }

    if to & 0x02 != 0 {
        let c = builder.ins().icmp(IntCC::UnsignedLessThan, a, b);
        or_in(builder, c, &mut cond);
    }

    if to & 0x01 != 0 {
        let c = builder.ins().icmp(IntCC::UnsignedGreaterThan, a, b);
        or_in(builder, c, &mut cond);
    }

    if let Some(c) = cond {
        let trap_block = builder.create_block();
        builder.set_cold_block(trap_block);
        let cont_block = builder.create_block();
        builder.ins().brif(c, trap_block, &[], cont_block, &[]);

        builder.switch_to_block(trap_block);
        builder.seal_block(trap_block);
        let srr0_value = builder.ins().iconst(types::I32, pc as i64);
        let nia = emit_exception_dispatch::<SYSTEM>(builder, ctx_ptr, srr0_value, 0x0002_0000, 0x0000_0700);
        builder.ins().jump(exit_block, &[nia.into()]);

        builder.switch_to_block(cont_block);
        builder.seal_block(cont_block);
    }
}

pub(crate) fn emit_mcrxr<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, instr: Instruction) {
    let xer = xer_load::<SYSTEM>(builder, ctx_ptr);
    let top3 = builder.ins().ushr_imm(xer, 29);
    let masked3 = builder.ins().band_imm(top3, 0x7);
    let shifted = builder.ins().ishl_imm(masked3, 1);
    let dst_shift = 28 - 4 * (instr.crfd() as u32);
    let positioned = builder.ins().ishl_imm(shifted, dst_shift as i64);
    let cr = cr_load::<SYSTEM>(builder, ctx_ptr);
    let cleared = builder.ins().band_imm(cr, !(0xFi64 << dst_shift));
    let new_cr = builder.ins().bor(cleared, positioned);
    cr_store::<SYSTEM>(builder, ctx_ptr, new_cr);

    let new_xer = builder.ins().band_imm(xer, !(0xE000_0000u32 as i64));
    let off = abi::xer_offset::<SYSTEM>() as i32;
    builder.ins().store(vmctx_flags(), new_xer, ctx_ptr, off);
}

fn read_xer_carry<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value) -> Value {
    let xer = xer_load::<SYSTEM>(builder, ctx_ptr);
    let shifted = builder.ins().ushr_imm(xer, 29);
    builder.ins().band_imm(shifted, 1)
}

fn add_with_carry_in<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    a: Value,
    b: Value,
    cin: Value,
) -> Value {
    let (s1, c1) = builder.ins().uadd_overflow(a, b);
    let (s2, c2) = builder.ins().uadd_overflow(s1, cin);
    let c1_32 = builder.ins().uextend(types::I32, c1);
    let c2_32 = builder.ins().uextend(types::I32, c2);
    let carry_out = builder.ins().bor(c1_32, c2_32);
    write_xer_carry::<SYSTEM>(builder, ctx_ptr, carry_out);
    s2
}

pub(crate) fn emit_adde_xform<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    let cin = read_xer_carry::<SYSTEM>(builder, ctx_ptr);
    let r = add_with_carry_in::<SYSTEM>(builder, ctx_ptr, a, b, cin);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
    r
}

pub(crate) fn emit_subfe_xform<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    let not_a = builder.ins().bxor_imm(a, !0i64);
    let cin = read_xer_carry::<SYSTEM>(builder, ctx_ptr);
    let r = add_with_carry_in::<SYSTEM>(builder, ctx_ptr, not_a, b, cin);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
    r
}

#[derive(Clone, Copy)]
pub(crate) enum AddzeSubfzeKind {
    Addze,
    Subfze,
}

pub(crate) fn emit_addze_subfze<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    kind: AddzeSubfzeKind,
) -> Value {
    let a_raw = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let a = match kind {
        AddzeSubfzeKind::Addze => a_raw,
        AddzeSubfzeKind::Subfze => builder.ins().bxor_imm(a_raw, !0i64),
    };
    let zero = builder.ins().iconst(types::I32, 0);
    let cin = read_xer_carry::<SYSTEM>(builder, ctx_ptr);
    let r = add_with_carry_in::<SYSTEM>(builder, ctx_ptr, a, zero, cin);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
    r
}

#[derive(Clone, Copy)]
pub(crate) enum AddmeSubfmeKind {
    Addme,
    Subfme,
}

pub(crate) fn emit_addme_subfme<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    kind: AddmeSubfmeKind,
) -> Value {
    let a_raw = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let a = match kind {
        AddmeSubfmeKind::Addme => a_raw,
        AddmeSubfmeKind::Subfme => builder.ins().bxor_imm(a_raw, !0i64),
    };
    let neg_one = builder.ins().iconst(types::I32, -1i64);
    let cin = read_xer_carry::<SYSTEM>(builder, ctx_ptr);
    let r = add_with_carry_in::<SYSTEM>(builder, ctx_ptr, a, neg_one, cin);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
    r
}

pub(crate) fn emit_mulli<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, instr: Instruction) {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let r = builder.ins().imul_imm(a, instr.simm() as i64);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
}

pub(crate) fn emit_srawi<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let s = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());

    let result = if instr.rb() == 0 {
        s
    } else {
        let amt = builder.ins().iconst(types::I32, instr.rb() as i64);
        builder.ins().sshr(s, amt)
    };
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), result);

    let ca: Value = if instr.rb() == 0 {
        builder.ins().iconst(types::I32, 0)
    } else {
        let mask = ((1u32 << instr.rb()) - 1) as i64;
        let lower = builder.ins().band_imm(s, mask);
        let lower_nonzero = builder.ins().icmp_imm(IntCC::NotEqual, lower, 0);
        let neg = builder.ins().icmp_imm(IntCC::SignedLessThan, s, 0);
        let both = builder.ins().band(neg, lower_nonzero);
        builder.ins().uextend(types::I32, both)
    };

    let xer = xer_load::<SYSTEM>(builder, ctx_ptr);
    let cleared = builder.ins().band_imm(xer, !0x2000_0000u32 as i64);
    let positioned = builder.ins().ishl_imm(ca, 29);
    let merged = builder.ins().bor(cleared, positioned);
    let off = abi::xer_offset::<SYSTEM>() as i32;
    builder.ins().store(vmctx_flags(), merged, ctx_ptr, off);

    result
}

pub(crate) fn emit_sraw<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let s = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());

    let count = builder.ins().band_imm(b, 0x3F);
    let big = builder.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, count, 32);
    let count_low5 = builder.ins().band_imm(count, 0x1F);

    let thirtyone = builder.ins().iconst(types::I32, 31);
    let eff_shift = builder.ins().select(big, thirtyone, count_low5);
    let result = builder.ins().sshr(s, eff_shift);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), result);

    let s_neg = builder.ins().icmp_imm(IntCC::SignedLessThan, s, 0);
    let s_neg_v = builder.ins().uextend(types::I32, s_neg);

    let one = builder.ins().iconst(types::I32, 1);
    let shifted_one = builder.ins().ishl(one, count_low5);
    let mask = builder.ins().iadd_imm(shifted_one, -1);
    let lower = builder.ins().band(s, mask);
    let lower_nonzero = builder.ins().icmp_imm(IntCC::NotEqual, lower, 0);
    let lower_nonzero_v = builder.ins().uextend(types::I32, lower_nonzero);
    let small_ca = builder.ins().band(s_neg_v, lower_nonzero_v);

    let ca = builder.ins().select(big, s_neg_v, small_ca);

    let xer = xer_load::<SYSTEM>(builder, ctx_ptr);
    let cleared = builder.ins().band_imm(xer, !0x2000_0000u32 as i64);
    let positioned = builder.ins().ishl_imm(ca, 29);
    let merged = builder.ins().bor(cleared, positioned);
    let off = abi::xer_offset::<SYSTEM>() as i32;
    builder.ins().store(vmctx_flags(), merged, ctx_ptr, off);

    result
}

pub(crate) fn emit_shift_xform<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    kind: ShiftKind,
) -> Value {
    let s = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    let count = builder.ins().band_imm(b, 0x3F);
    let count_low5 = builder.ins().band_imm(count, 0x1F);
    let shifted = match kind {
        ShiftKind::Slw => builder.ins().ishl(s, count_low5),
        ShiftKind::Srw => builder.ins().ushr(s, count_low5),
    };
    let high_bit = builder.ins().band_imm(count, 0x20);
    let out_of_range = builder.ins().icmp_imm(IntCC::NotEqual, high_bit, 0);
    let zero = builder.ins().iconst(types::I32, 0);
    let r = builder.ins().select(out_of_range, zero, shifted);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), r);
    r
}

pub(crate) fn emit_mfspr<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    local: &BlockEmitContext,
) -> bool {
    let spr_num = instr.spr_swapped() as u16;
    if let Some(off) = abi::spr_field_offset::<SYSTEM>(spr_num).map(|off| off as i32) {
        let val = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, off);
        gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), val);
    } else {
        let num_v = builder.ins().iconst(types::I32, spr_num as i64);
        let call = builder.ins().call(local.read_spr, &[ctx_ptr, num_v]);
        let val = builder.inst_results(call)[0];
        gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), val);
    }
    true
}

pub(crate) fn emit_mtspr<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    local: &mut BlockEmitContext,
) -> bool {
    let spr_num = instr.spr_swapped() as u16;
    let val = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    if let Some(off) = abi::spr_field_offset::<SYSTEM>(spr_num).map(|off| off as i32) {
        builder.ins().store(vmctx_flags(), val, ctx_ptr, off);

        if matches!(spr_num, 912..=919) {
            gqr_cache_store(local, (spr_num - 912) as u8, val);
        }
    } else {
        let num_v = builder.ins().iconst(types::I32, spr_num as i64);
        builder.ins().call(local.write_spr, &[ctx_ptr, num_v, val]);
    }
    true
}

fn rlw_mask(mb: u8, me: u8) -> u32 {
    debug_assert!(mb < 32 && me < 32);

    let begin = 31u8 - mb;
    let end = 31u8 - me;
    if begin >= end {
        let width = (begin - end + 1) as u32;

        if width >= 32 {
            0xFFFF_FFFF
        } else {
            (((1u64 << width) - 1) as u32) << end
        }
    } else {
        let lo_part = ((1u64 << (begin + 1)) - 1) as u32;
        let hi_part = !(((1u64 << end) - 1) as u32);
        lo_part | hi_part
    }
}

pub(crate) fn emit_rlwnm<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let mask = rlw_mask(instr.mb(), instr.me());
    let s = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    let amt = builder.ins().band_imm(b, 0x1F);
    let rotated = builder.ins().rotl(s, amt);
    let masked = builder.ins().band_imm(rotated, mask as i64);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), masked);
    masked
}

pub(crate) fn emit_rlwimi<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let mask = rlw_mask(instr.mb(), instr.me());
    let s = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());

    let rotated = if instr.rb() == 0 {
        s
    } else {
        let amt = builder.ins().iconst(types::I32, instr.rb() as i64);
        builder.ins().rotl(s, amt)
    };

    let inserted_bits = builder.ins().band_imm(rotated, mask as i64);
    let kept_bits = builder.ins().band_imm(a, !mask as i64);
    let val = builder.ins().bor(kept_bits, inserted_bits);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), val);

    val
}

pub(crate) fn emit_rlwinm<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let mask = rlw_mask(instr.mb(), instr.me());
    let s = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    let rotated = if instr.rb() == 0 {
        s
    } else {
        let amt = builder.ins().iconst(types::I32, instr.rb() as i64);
        builder.ins().rotl(s, amt)
    };

    let masked = builder.ins().band_imm(rotated, mask as i64);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), masked);

    masked
}

fn cr_bit_load<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, bi: u8) -> Value {
    let cr = cr_load::<SYSTEM>(builder, ctx_ptr);
    let shifted = builder.ins().ushr_imm(cr, (31 - bi) as i64);
    builder.ins().band_imm(shifted, 1)
}

pub(crate) fn ctr_load<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value) -> Value {
    let off = abi::ctr_offset::<SYSTEM>() as i32;
    builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, off)
}

pub(crate) fn ctr_store<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, val: Value) {
    let off = abi::ctr_offset::<SYSTEM>() as i32;
    builder.ins().store(vmctx_flags(), val, ctx_ptr, off);
}

pub(crate) fn lr_store<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, val: Value) {
    let off = abi::lr_offset::<SYSTEM>() as i32;
    builder.ins().store(vmctx_flags(), val, ctx_ptr, off);
}

pub(crate) fn emit_b<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    pc: u32,
) -> Value {
    let target: u32 = if instr.aa() {
        instr.li() as u32
    } else {
        pc.wrapping_add_signed(instr.li())
    };
    if instr.lk() {
        let lr_val = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);
        lr_store::<SYSTEM>(builder, ctx_ptr, lr_val);
    }
    builder.ins().iconst(types::I32, target as i64)
}

pub(crate) fn emit_bc_with_chain<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    host_return_pc: Value,
    instr: Instruction,
    pc: u32,
    exit_block: Block,
    taken_slot: Option<i64>,
    fall_slot: Option<i64>,
    block_sig_ref: cranelift_codegen::ir::SigRef,
    self_loop: Option<(u32, Block)>,
) -> TermEmit {
    if taken_slot.is_none() && fall_slot.is_none() && self_loop.is_none() {
        return TermEmit::HandledNia(emit_bc::<SYSTEM>(builder, ctx_ptr, instr, pc));
    }

    let bo = (instr.bo() as u32) as u8;

    let taken_target: u32 = if instr.aa() {
        instr.bd() as u32
    } else {
        pc.wrapping_add_signed(instr.bd())
    };
    let fall_target: u32 = pc.wrapping_add(4);

    let decrement_ctr = (bo & 0x04) == 0;
    let mut ctr_after: Option<Value> = None;
    if decrement_ctr {
        let ctr = ctr_load::<SYSTEM>(builder, ctx_ptr);
        let ctr_minus_1 = builder.ins().iadd_imm(ctr, -1);
        ctr_store::<SYSTEM>(builder, ctx_ptr, ctr_minus_1);
        ctr_after = Some(ctr_minus_1);
    }

    let ctr_ok: Value = if decrement_ctr {
        let ctr = ctr_after.unwrap();
        let cc = if (bo & 0x02) != 0 {
            IntCC::Equal
        } else {
            IntCC::NotEqual
        };
        let r = builder.ins().icmp_imm(cc, ctr, 0);
        builder.ins().uextend(types::I32, r)
    } else {
        builder.ins().iconst(types::I32, 1)
    };

    let cond_ok: Value = if (bo & 0x10) != 0 {
        builder.ins().iconst(types::I32, 1)
    } else {
        let bit = cr_bit_load::<SYSTEM>(builder, ctx_ptr, instr.bi());
        let want = if (bo & 0x08) != 0 { 1 } else { 0 };
        let r = builder.ins().icmp_imm(IntCC::Equal, bit, want);
        builder.ins().uextend(types::I32, r)
    };

    let take = builder.ins().band(ctr_ok, cond_ok);
    let take_b = builder.ins().icmp_imm(IntCC::NotEqual, take, 0);

    let taken_block = builder.create_block();
    let fall_block = builder.create_block();
    builder.ins().brif(take_b, taken_block, &[], fall_block, &[]);

    builder.switch_to_block(taken_block);
    builder.seal_block(taken_block);
    if instr.lk() {
        let lr_val = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);
        lr_store::<SYSTEM>(builder, ctx_ptr, lr_val);
    }
    let taken_v = builder.ins().iconst(types::I32, taken_target as i64);

    if let Some((_, header)) = self_loop.filter(|(self_pc, _)| *self_pc == taken_target && !instr.lk()) {
        emit_loop_or_exit::<SYSTEM>(builder, ctx_ptr, header, taken_v, exit_block);
    } else if let Some(slot_addr) = taken_slot {
        emit_chain_or_exit::<SYSTEM>(
            builder,
            ctx_ptr,
            host_return_pc,
            slot_addr,
            block_sig_ref,
            taken_v,
            exit_block,
        );
    } else {
        builder.ins().jump(exit_block, &[taken_v.into()]);
    }

    builder.switch_to_block(fall_block);
    builder.seal_block(fall_block);

    let fall_v = builder.ins().iconst(types::I32, fall_target as i64);
    if let Some(slot_addr) = fall_slot {
        emit_chain_or_exit::<SYSTEM>(
            builder,
            ctx_ptr,
            host_return_pc,
            slot_addr,
            block_sig_ref,
            fall_v,
            exit_block,
        );
    } else {
        builder.ins().jump(exit_block, &[fall_v.into()]);
    }

    TermEmit::HandledChained
}

fn emit_loop_or_exit<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    loop_header: Block,
    nia_if_exit: Value,
    exit_block: Block,
) {
    let cycles = builder.ins().load(
        types::I64,
        vmctx_flags(),
        ctx_ptr,
        abi::cycles_offset::<SYSTEM>() as i32,
    );
    let deadline = builder.ins().load(
        types::I64,
        vmctx_flags(),
        ctx_ptr,
        abi::next_deadline_offset::<SYSTEM>() as i32,
    );
    let exhausted = builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, cycles, deadline);
    builder
        .ins()
        .brif(exhausted, exit_block, &[nia_if_exit.into()], loop_header, &[]);
}

pub(crate) fn emit_bc<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    pc: u32,
) -> Value {
    let bo = (instr.bo() as u32) as u8;

    let target: u32 = if instr.aa() {
        instr.bd() as u32
    } else {
        pc.wrapping_add_signed(instr.bd())
    };
    let target_v = builder.ins().iconst(types::I32, target as i64);
    let fall_v = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);

    let decrement_ctr = (bo & 0x04) == 0;
    let mut ctr_after: Option<Value> = None;
    if decrement_ctr {
        let ctr = ctr_load::<SYSTEM>(builder, ctx_ptr);
        let ctr_minus_1 = builder.ins().iadd_imm(ctr, -1);
        ctr_store::<SYSTEM>(builder, ctx_ptr, ctr_minus_1);
        ctr_after = Some(ctr_minus_1);
    }

    let ctr_ok: Value = if decrement_ctr {
        let ctr = ctr_after.unwrap();
        if (bo & 0x02) != 0 {
            let r = builder.ins().icmp_imm(IntCC::Equal, ctr, 0);
            builder.ins().uextend(types::I32, r)
        } else {
            let r = builder.ins().icmp_imm(IntCC::NotEqual, ctr, 0);
            builder.ins().uextend(types::I32, r)
        }
    } else {
        builder.ins().iconst(types::I32, 1)
    };

    let cond_ok: Value = if (bo & 0x10) != 0 {
        builder.ins().iconst(types::I32, 1)
    } else {
        let bit = cr_bit_load::<SYSTEM>(builder, ctx_ptr, instr.bi());
        let want = if (bo & 0x08) != 0 { 1 } else { 0 };
        let r = builder.ins().icmp_imm(IntCC::Equal, bit, want);
        builder.ins().uextend(types::I32, r)
    };

    let take = builder.ins().band(ctr_ok, cond_ok);
    let take_b = builder.ins().icmp_imm(IntCC::NotEqual, take, 0);

    if instr.lk() {
        let lk_block = builder.create_block();
        let after_lk = builder.create_block();
        builder.ins().brif(take_b, lk_block, &[], after_lk, &[]);
        builder.switch_to_block(lk_block);
        builder.seal_block(lk_block);

        let lr_val = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);
        lr_store::<SYSTEM>(builder, ctx_ptr, lr_val);

        builder.ins().jump(after_lk, &[]);
        builder.switch_to_block(after_lk);
        builder.seal_block(after_lk);
    }

    builder.ins().select(take_b, target_v, fall_v)
}

pub(crate) fn lr_load<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value) -> Value {
    let off = abi::lr_offset::<SYSTEM>() as i32;
    builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, off)
}

pub(crate) fn emit_bclr_dispatch<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    host_return_pc: Value,
    instr: Instruction,
    pc: u32,
    exit_block: Block,
    block_sig_ref: cranelift_codegen::ir::SigRef,
    lookup_table_addr: i64,
    fall_slot: Option<i64>,
) -> TermEmit {
    let bo = (instr.bo() as u32) as u8;

    let unconditional = (bo & 0x14) == 0x14;
    if unconditional && !instr.lk() {
        emit_blr_with_ic::<SYSTEM>(
            builder,
            ctx_ptr,
            host_return_pc,
            pc,
            exit_block,
            block_sig_ref,
            lookup_table_addr,
        )
    } else if !unconditional {
        self::emit_bclr_chain_fall::<SYSTEM>(
            builder,
            ctx_ptr,
            host_return_pc,
            instr,
            pc,
            exit_block,
            block_sig_ref,
            lookup_table_addr,
            fall_slot,
        )
    } else {
        TermEmit::HandledNia(emit_bclr::<SYSTEM>(builder, ctx_ptr, instr, pc))
    }
}

pub(crate) fn emit_bcctr_dispatch<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    host_return_pc: Value,
    instr: Instruction,
    pc: u32,
    exit_block: Block,
    block_sig_ref: cranelift_codegen::ir::SigRef,
    lookup_table_addr: i64,
    fall_slot: Option<i64>,
) -> TermEmit {
    let bo = (instr.bo() as u32) as u8;
    let unconditional = (bo & 0x14) == 0x14;
    if unconditional && !instr.lk() {
        emit_bctr_with_ic::<SYSTEM>(
            builder,
            ctx_ptr,
            host_return_pc,
            pc,
            exit_block,
            block_sig_ref,
            lookup_table_addr,
        )
    } else if !unconditional {
        self::emit_bcctr_chain_fall::<SYSTEM>(
            builder,
            ctx_ptr,
            host_return_pc,
            instr,
            pc,
            exit_block,
            block_sig_ref,
            lookup_table_addr,
            fall_slot,
        )
    } else {
        TermEmit::HandledNia(emit_bcctr::<SYSTEM>(builder, ctx_ptr, instr, pc))
    }
}

fn emit_bclr_chain_fall<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    host_return_pc: Value,
    instr: Instruction,
    pc: u32,
    exit_block: Block,
    block_sig_ref: cranelift_codegen::ir::SigRef,
    lookup_table_addr: i64,
    fall_slot: Option<i64>,
) -> TermEmit {
    let bo = (instr.bo() as u32) as u8;

    let decrement_ctr = (bo & 0x04) == 0;
    let mut ctr_after: Option<Value> = None;
    if decrement_ctr {
        let ctr = ctr_load::<SYSTEM>(builder, ctx_ptr);
        let ctr_minus_1 = builder.ins().iadd_imm(ctr, -1);
        ctr_store::<SYSTEM>(builder, ctx_ptr, ctr_minus_1);
        ctr_after = Some(ctr_minus_1);
    }

    let ctr_ok: Value = if decrement_ctr {
        let ctr = ctr_after.unwrap();
        let cc = if (bo & 0x02) != 0 {
            IntCC::Equal
        } else {
            IntCC::NotEqual
        };
        let r = builder.ins().icmp_imm(cc, ctr, 0);
        builder.ins().uextend(types::I32, r)
    } else {
        builder.ins().iconst(types::I32, 1)
    };

    let cond_ok: Value = if (bo & 0x10) != 0 {
        builder.ins().iconst(types::I32, 1)
    } else {
        let bit = cr_bit_load::<SYSTEM>(builder, ctx_ptr, instr.bi());
        let want = if (bo & 0x08) != 0 { 1 } else { 0 };
        let r = builder.ins().icmp_imm(IntCC::Equal, bit, want);
        builder.ins().uextend(types::I32, r)
    };

    let take = builder.ins().band(ctr_ok, cond_ok);
    let take_b = builder.ins().icmp_imm(IntCC::NotEqual, take, 0);

    let taken_block = builder.create_block();
    let fall_block = builder.create_block();
    builder.ins().brif(take_b, taken_block, &[], fall_block, &[]);

    builder.switch_to_block(taken_block);
    builder.seal_block(taken_block);

    if instr.lk() {
        let lr_val = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);
        lr_store::<SYSTEM>(builder, ctx_ptr, lr_val);
    }

    let lr = lr_load::<SYSTEM>(builder, ctx_ptr);
    let target_pc = builder.ins().band_imm(lr, !3i64);
    emit_inline_cache_dispatch::<SYSTEM>(
        builder,
        ctx_ptr,
        host_return_pc,
        target_pc,
        exit_block,
        block_sig_ref,
        lookup_table_addr,
    );

    builder.switch_to_block(fall_block);
    builder.seal_block(fall_block);

    let fall_v = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);

    if let Some(slot_addr) = fall_slot {
        emit_chain_or_exit::<SYSTEM>(
            builder,
            ctx_ptr,
            host_return_pc,
            slot_addr,
            block_sig_ref,
            fall_v,
            exit_block,
        );
    } else {
        builder.ins().jump(exit_block, &[fall_v.into()]);
    }

    TermEmit::HandledChained
}

fn emit_bcctr_chain_fall<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    host_return_pc: Value,
    instr: Instruction,
    pc: u32,
    exit_block: Block,
    block_sig_ref: cranelift_codegen::ir::SigRef,
    lookup_table_addr: i64,
    fall_slot: Option<i64>,
) -> TermEmit {
    let bo = (instr.bo() as u32) as u8;

    let unconditional = (bo & 0x10) != 0;
    let take_b = if unconditional {
        builder.ins().iconst(types::I8, 1)
    } else {
        let bit = cr_bit_load::<SYSTEM>(builder, ctx_ptr, instr.bi());
        let want = if (bo & 0x08) != 0 { 1 } else { 0 };
        builder.ins().icmp_imm(IntCC::Equal, bit, want)
    };

    let taken_block = builder.create_block();
    let fall_block = builder.create_block();
    builder.ins().brif(take_b, taken_block, &[], fall_block, &[]);

    builder.switch_to_block(taken_block);
    builder.seal_block(taken_block);
    if instr.lk() {
        let lr_val = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);
        lr_store::<SYSTEM>(builder, ctx_ptr, lr_val);
    }
    let ctr = ctr_load::<SYSTEM>(builder, ctx_ptr);
    let target_pc = builder.ins().band_imm(ctr, !3i64);
    emit_inline_cache_dispatch::<SYSTEM>(
        builder,
        ctx_ptr,
        host_return_pc,
        target_pc,
        exit_block,
        block_sig_ref,
        lookup_table_addr,
    );

    builder.switch_to_block(fall_block);
    builder.seal_block(fall_block);
    let fall_v = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);
    if let Some(slot_addr) = fall_slot {
        emit_chain_or_exit::<SYSTEM>(
            builder,
            ctx_ptr,
            host_return_pc,
            slot_addr,
            block_sig_ref,
            fall_v,
            exit_block,
        );
    } else {
        builder.ins().jump(exit_block, &[fall_v.into()]);
    }

    TermEmit::HandledChained
}

pub(crate) fn emit_blr_with_ic<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    host_return_pc: Value,
    _pc: u32,
    exit_block: Block,
    block_sig_ref: cranelift_codegen::ir::SigRef,
    lookup_table_addr: i64,
) -> TermEmit {
    let lr = lr_load::<SYSTEM>(builder, ctx_ptr);
    let target_pc = builder.ins().band_imm(lr, !3i64);
    let direct = builder.ins().icmp(IntCC::Equal, target_pc, host_return_pc);
    let return_block = builder.create_block();
    let dispatch_block = builder.create_block();
    builder.ins().brif(direct, return_block, &[], dispatch_block, &[]);

    builder.switch_to_block(return_block);
    builder.seal_block(return_block);
    builder.ins().return_(&[target_pc]);

    builder.switch_to_block(dispatch_block);
    builder.seal_block(dispatch_block);
    emit_inline_cache_dispatch::<SYSTEM>(
        builder,
        ctx_ptr,
        host_return_pc,
        target_pc,
        exit_block,
        block_sig_ref,
        lookup_table_addr,
    );
    TermEmit::HandledChained
}

pub(crate) fn emit_bctr_with_ic<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    host_return_pc: Value,
    _pc: u32,
    exit_block: Block,
    block_sig_ref: cranelift_codegen::ir::SigRef,
    lookup_table_addr: i64,
) -> TermEmit {
    let ctr = ctr_load::<SYSTEM>(builder, ctx_ptr);
    let target_pc = builder.ins().band_imm(ctr, !3i64);
    emit_inline_cache_dispatch::<SYSTEM>(
        builder,
        ctx_ptr,
        host_return_pc,
        target_pc,
        exit_block,
        block_sig_ref,
        lookup_table_addr,
    );
    TermEmit::HandledChained
}

pub(crate) fn emit_inline_cache_dispatch<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    host_return_pc: Value,
    target_pc: Value,
    exit_block: Block,
    block_sig_ref: cranelift_codegen::ir::SigRef,
    lookup_table_addr: i64,
) {
    let mask_v = builder.ins().iconst(types::I32, super::BLOCK_LOOKUP_TABLE_MASK as i64);
    let idx_word = builder.ins().ushr_imm(target_pc, 2);
    let idx = builder.ins().band(idx_word, mask_v);
    let idx64 = builder.ins().uextend(types::I64, idx);
    let entry_size = std::mem::size_of::<super::BlockLookupSlot>() as i64;
    let off64 = builder.ins().imul_imm(idx64, entry_size);
    let table_base = builder.ins().iconst(types::I64, lookup_table_addr);
    let slot_addr = builder.ins().iadd(table_base, off64);

    let slot_pc = builder.ins().load(types::I32, vmctx_flags(), slot_addr, 0);
    let slot_entry = builder.ins().load(types::I64, vmctx_flags(), slot_addr, 8);
    let pc_eq = builder.ins().icmp(IntCC::Equal, slot_pc, target_pc);

    let cyc_off = abi::cycles_offset::<SYSTEM>() as i32;
    let dl_off = abi::next_deadline_offset::<SYSTEM>() as i32;
    let cycles = builder.ins().load(types::I64, vmctx_flags(), ctx_ptr, cyc_off);
    let deadline = builder.ins().load(types::I64, vmctx_flags(), ctx_ptr, dl_off);
    let in_budget = builder.ins().icmp(IntCC::UnsignedLessThan, cycles, deadline);

    let pc_eq_i8 = pc_eq;
    let budget_i8 = in_budget;
    let nonzero_i8 = builder.ins().icmp_imm(IntCC::NotEqual, slot_entry, 0);
    let pc_and_budget = builder.ins().band(pc_eq_i8, budget_i8);
    let ok = builder.ins().band(pc_and_budget, nonzero_i8);

    let hit_block = builder.create_block();
    let miss_block = builder.create_block();
    builder.set_cold_block(miss_block);
    builder.ins().brif(ok, hit_block, &[], miss_block, &[]);

    builder.switch_to_block(miss_block);
    builder.seal_block(miss_block);
    builder.ins().jump(exit_block, &[target_pc.into()]);

    builder.switch_to_block(hit_block);
    builder.seal_block(hit_block);
    builder
        .ins()
        .return_call_indirect(block_sig_ref, slot_entry, &[ctx_ptr, host_return_pc]);
}

pub(crate) fn emit_bclr<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    pc: u32,
) -> Value {
    let bo = (instr.bo() as u32) as u8;

    let lr = lr_load::<SYSTEM>(builder, ctx_ptr);
    let target_v = builder.ins().band_imm(lr, !3i64);
    let fall_v = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);

    let decrement_ctr = (bo & 0x04) == 0;
    let mut ctr_after: Option<Value> = None;
    if decrement_ctr {
        let ctr = ctr_load::<SYSTEM>(builder, ctx_ptr);
        let ctr_minus_1 = builder.ins().iadd_imm(ctr, -1);
        ctr_store::<SYSTEM>(builder, ctx_ptr, ctr_minus_1);
        ctr_after = Some(ctr_minus_1);
    }

    let ctr_ok: Value = if decrement_ctr {
        let ctr = ctr_after.unwrap();
        let cc = if (bo & 0x02) != 0 {
            IntCC::Equal
        } else {
            IntCC::NotEqual
        };
        let r = builder.ins().icmp_imm(cc, ctr, 0);
        builder.ins().uextend(types::I32, r)
    } else {
        builder.ins().iconst(types::I32, 1)
    };

    let cond_ok: Value = if (bo & 0x10) != 0 {
        builder.ins().iconst(types::I32, 1)
    } else {
        let bit = cr_bit_load::<SYSTEM>(builder, ctx_ptr, instr.bi());
        let want = if (bo & 0x08) != 0 { 1 } else { 0 };
        let r = builder.ins().icmp_imm(IntCC::Equal, bit, want);
        builder.ins().uextend(types::I32, r)
    };

    let take = builder.ins().band(ctr_ok, cond_ok);
    let take_b = builder.ins().icmp_imm(IntCC::NotEqual, take, 0);

    if instr.lk() {
        let lk_block = builder.create_block();
        let after_lk = builder.create_block();
        builder.ins().brif(take_b, lk_block, &[], after_lk, &[]);
        builder.switch_to_block(lk_block);
        builder.seal_block(lk_block);
        let lr_val = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);
        lr_store::<SYSTEM>(builder, ctx_ptr, lr_val);
        builder.ins().jump(after_lk, &[]);
        builder.switch_to_block(after_lk);
        builder.seal_block(after_lk);
    }

    builder.ins().select(take_b, target_v, fall_v)
}

pub(crate) fn emit_bcctr<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    pc: u32,
) -> Value {
    let bo = (instr.bo() as u32) as u8;

    let ctr = ctr_load::<SYSTEM>(builder, ctx_ptr);
    let target_v = builder.ins().band_imm(ctr, !3i64);
    let fall_v = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);

    let unconditional = (bo & 0x10) != 0;
    let take_b = if unconditional {
        builder.ins().iconst(types::I8, 1)
    } else {
        let bit = cr_bit_load::<SYSTEM>(builder, ctx_ptr, instr.bi());
        let want = if (bo & 0x08) != 0 { 1 } else { 0 };
        builder.ins().icmp_imm(IntCC::Equal, bit, want)
    };

    if instr.lk() {
        let lk_block = builder.create_block();
        let after_lk = builder.create_block();
        builder.ins().brif(take_b, lk_block, &[], after_lk, &[]);
        builder.switch_to_block(lk_block);
        builder.seal_block(lk_block);
        let lr_val = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);
        lr_store::<SYSTEM>(builder, ctx_ptr, lr_val);
        builder.ins().jump(after_lk, &[]);
        builder.switch_to_block(after_lk);
        builder.seal_block(after_lk);
    }

    builder.ins().select(take_b, target_v, fall_v)
}

pub(crate) fn emit_cycles_add<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, n: i64) {
    let off = abi::cycles_offset::<SYSTEM>() as i64;
    let addr = builder.ins().iadd_imm(ctx_ptr, off);
    let now = builder.ins().load(types::I64, vmctx_flags(), addr, 0);
    let next = builder.ins().iadd_imm(now, n);
    builder.ins().store(vmctx_flags(), next, addr, 0);
}

pub(crate) fn emit_mcrf<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, instr: Instruction) {
    let src_shift = 28 - 4 * (instr.crfs() as u32);
    let dst_shift = 28 - 4 * (instr.crfd() as u32);
    let cr = cr_load::<SYSTEM>(builder, ctx_ptr);
    let src_field = builder.ins().ushr_imm(cr, src_shift as i64);
    let src_nibble = builder.ins().band_imm(src_field, 0xF);
    let cleared = builder.ins().band_imm(cr, !(0xFi64 << dst_shift));
    let positioned = builder.ins().ishl_imm(src_nibble, dst_shift as i64);
    let new_cr = builder.ins().bor(cleared, positioned);
    cr_store::<SYSTEM>(builder, ctx_ptr, new_cr);
}

pub(crate) fn emit_cr_bit_op<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    xo: u32,
) {
    let bit_a = 31 - (instr.crba() as u32);
    let bit_b = 31 - (instr.crbb() as u32);
    let bit_d = 31 - (instr.crbd() as u32);

    let cr = cr_load::<SYSTEM>(builder, ctx_ptr);
    let a_shifted = builder.ins().ushr_imm(cr, bit_a as i64);
    let a = builder.ins().band_imm(a_shifted, 1);
    let b_shifted = builder.ins().ushr_imm(cr, bit_b as i64);
    let b = builder.ins().band_imm(b_shifted, 1);

    let result = match xo {
        33 => {
            let or_v = builder.ins().bor(a, b);
            let inv = builder.ins().bxor_imm(or_v, 1);
            inv
        }
        129 => {
            let not_b = builder.ins().bxor_imm(b, 1);
            builder.ins().band(a, not_b)
        }
        193 => builder.ins().bxor(a, b),
        225 => {
            let and_v = builder.ins().band(a, b);
            builder.ins().bxor_imm(and_v, 1)
        }
        257 => builder.ins().band(a, b),
        289 => {
            let xor_v = builder.ins().bxor(a, b);
            builder.ins().bxor_imm(xor_v, 1)
        }
        417 => {
            let not_b = builder.ins().bxor_imm(b, 1);
            builder.ins().bor(a, not_b)
        }
        449 => builder.ins().bor(a, b),
        _ => unreachable!("unexpected CR-bit XO {xo}"),
    };

    let cleared = builder.ins().band_imm(cr, !(1i64 << bit_d));
    let positioned = builder.ins().ishl_imm(result, bit_d as i64);
    let new_cr = builder.ins().bor(cleared, positioned);
    cr_store::<SYSTEM>(builder, ctx_ptr, new_cr);
}

#[derive(Clone, Copy)]
pub(crate) enum ShiftKind {
    Slw,
    Srw,
}

#[derive(Clone, Copy)]
pub(crate) enum LogicalOp {
    And,
    Or,
    Xor,
}

fn pointer_iter_loop_cycles_per_iter(instrs: &[u32]) -> i64 {
    use crate::gekko::cycles::{DEFAULT_CYCLES, cycles_for_op};
    use crate::gekko::lut::{OP_DCBA, OP_DCBF, OP_DCBI, OP_DCBST, OP_DCBT, OP_DCBTST, OP_ICBI};

    let cache = instrs.first().copied().map(Instruction).map(|i| i.xo10()).unwrap_or(0);
    let cache_op = match cache {
        86 => OP_DCBF,
        470 => OP_DCBI,
        54 => OP_DCBST,
        278 => OP_DCBT,
        246 => OP_DCBTST,
        982 => OP_ICBI,
        758 => OP_DCBA,
        _ => return 3 * DEFAULT_CYCLES,
    };

    cycles_for_op(cache_op) + 2 * DEFAULT_CYCLES
}

fn terminator_op_cycles(terminator: TermKind, instr: Instruction) -> i64 {
    use crate::gekko::cycles::cycles_for_op;
    use crate::gekko::lut::{OP_BCCTRX, OP_BCLRX, OP_BCX, OP_BX, OP_ISYNC, OP_MTMSR, OP_MTSPR, OP_RFI, OP_SC};
    let op = match terminator {
        TermKind::Branch => OP_BX,
        TermKind::BranchCond => OP_BCX,
        TermKind::BranchToReg => {
            if instr.xo10() == 16 {
                OP_BCLRX
            } else {
                OP_BCCTRX
            }
        }
        TermKind::SystemCall => OP_SC,
        TermKind::Rfi => OP_RFI,
        TermKind::Mtmsr => OP_MTMSR,
        TermKind::Mtspr => OP_MTSPR,
        TermKind::Isync => OP_ISYNC,
        TermKind::LengthCap => return crate::gekko::cycles::DEFAULT_CYCLES,
    };
    cycles_for_op(op)
}

fn terminator_fall_pc(spec: &BlockSpec) -> Option<u32> {
    let last_idx = spec.instrs.len().checked_sub(1)?;
    let pc = spec.pc_of(last_idx);
    let fall_pc = pc.wrapping_add(4);

    match spec.terminator {
        TermKind::Branch => {
            let instr = Instruction(spec.instrs[last_idx]);

            instr.lk().then_some(fall_pc)
        }
        TermKind::BranchCond | TermKind::Mtmsr => Some(fall_pc),
        TermKind::BranchToReg => {
            let raw = spec.instrs[last_idx];
            let bo = (raw >> 21) & 0x1F;
            let unconditional = (bo & 0x14) == 0x14;
            if unconditional { None } else { Some(fall_pc) }
        }
        _ => None,
    }
}

fn terminator_static_taken_pc(spec: &BlockSpec) -> Option<u32> {
    if spec.instrs.is_empty() {
        return None;
    }
    let last_idx = spec.instrs.len() - 1;
    let pc = spec.pc_of(last_idx);
    let instr = Instruction(spec.instrs[last_idx]);
    match spec.terminator {
        TermKind::Branch => Some(if instr.aa() {
            instr.li() as u32
        } else {
            pc.wrapping_add_signed(instr.li())
        }),
        TermKind::BranchCond => Some(if instr.aa() {
            instr.bd() as u32
        } else {
            pc.wrapping_add_signed(instr.bd())
        }),
        TermKind::LengthCap => Some(spec.end_pc()),
        _ => None,
    }
}

pub(crate) enum TermEmit {
    #[allow(dead_code)]
    NotHandled,

    HandledNia(Value),

    HandledChained,
}

pub(crate) fn emit_chain_or_exit<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    host_return_pc: Value,
    slot_addr: i64,
    block_sig_ref: cranelift_codegen::ir::SigRef,
    nia_if_exit: Value,
    exit_block: Block,
) {
    let cyc_off = abi::cycles_offset::<SYSTEM>() as i32;
    let dl_off = abi::next_deadline_offset::<SYSTEM>() as i32;
    let cycles = builder.ins().load(types::I64, vmctx_flags(), ctx_ptr, cyc_off);
    let deadline = builder.ins().load(types::I64, vmctx_flags(), ctx_ptr, dl_off);
    let exhausted = builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, cycles, deadline);

    let chain_block = builder.create_block();
    builder
        .ins()
        .brif(exhausted, exit_block, &[nia_if_exit.into()], chain_block, &[]);

    builder.switch_to_block(chain_block);
    builder.seal_block(chain_block);

    let slot_const = builder.ins().iconst(types::I64, slot_addr);
    let target_ptr = builder.ins().load(types::I64, vmctx_flags(), slot_const, 0);
    let is_null = builder.ins().icmp_imm(IntCC::Equal, target_ptr, 0);
    let dispatch_block = builder.create_block();
    builder.set_cold_block(dispatch_block);
    let tailcall_block = builder.create_block();
    builder.ins().brif(is_null, dispatch_block, &[], tailcall_block, &[]);

    builder.switch_to_block(dispatch_block);
    builder.seal_block(dispatch_block);
    builder.ins().jump(exit_block, &[nia_if_exit.into()]);

    builder.switch_to_block(tailcall_block);
    builder.seal_block(tailcall_block);
    builder
        .ins()
        .return_call_indirect(block_sig_ref, target_ptr, &[ctx_ptr, host_return_pc]);
}

pub(crate) fn emit_call_or_exit<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    caller_host_return_pc: Value,
    slot_addr: i64,
    block_sig_ref: cranelift_codegen::ir::SigRef,
    nia_if_exit: Value,
    return_pc: u32,
    return_slot_addr: Option<i64>,
    exit_block: Block,
) {
    let cyc_off = abi::cycles_offset::<SYSTEM>() as i32;
    let dl_off = abi::next_deadline_offset::<SYSTEM>() as i32;
    let cycles = builder.ins().load(types::I64, vmctx_flags(), ctx_ptr, cyc_off);
    let deadline = builder.ins().load(types::I64, vmctx_flags(), ctx_ptr, dl_off);
    let exhausted = builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, cycles, deadline);

    let chain_block = builder.create_block();
    builder
        .ins()
        .brif(exhausted, exit_block, &[nia_if_exit.into()], chain_block, &[]);

    builder.switch_to_block(chain_block);
    builder.seal_block(chain_block);
    let slot_const = builder.ins().iconst(types::I64, slot_addr);
    let target_ptr = builder.ins().load(types::I64, vmctx_flags(), slot_const, 0);
    let is_null = builder.ins().icmp_imm(IntCC::Equal, target_ptr, 0);

    let dispatch_block = builder.create_block();
    builder.set_cold_block(dispatch_block);
    let call_block = builder.create_block();
    builder.ins().brif(is_null, dispatch_block, &[], call_block, &[]);

    builder.switch_to_block(dispatch_block);
    builder.seal_block(dispatch_block);
    builder.ins().jump(exit_block, &[nia_if_exit.into()]);

    builder.switch_to_block(call_block);
    builder.seal_block(call_block);

    let return_pc_v = builder.ins().iconst(types::I32, return_pc as i64);
    let call = builder
        .ins()
        .call_indirect(block_sig_ref, target_ptr, &[ctx_ptr, return_pc_v]);
    let returned_nia = builder.inst_results(call)[0];

    let is_expected_return = builder.ins().icmp(IntCC::Equal, returned_nia, return_pc_v);
    let continue_block = builder.create_block();
    let bubble_block = builder.create_block();
    builder.set_cold_block(bubble_block);
    builder
        .ins()
        .brif(is_expected_return, continue_block, &[], bubble_block, &[]);

    builder.switch_to_block(bubble_block);
    builder.seal_block(bubble_block);
    builder.ins().jump(exit_block, &[returned_nia.into()]);

    builder.switch_to_block(continue_block);
    builder.seal_block(continue_block);

    if let Some(return_slot_addr) = return_slot_addr {
        emit_chain_or_exit::<SYSTEM>(
            builder,
            ctx_ptr,
            caller_host_return_pc,
            return_slot_addr,
            block_sig_ref,
            returned_nia,
            exit_block,
        );
    } else {
        builder.ins().jump(exit_block, &[returned_nia.into()]);
    }
}

fn gpr_offset<const SYSTEM: SystemId>(i: u8) -> i32 {
    debug_assert!(i < 32);
    (abi::gpr_base_offset::<SYSTEM>() + 4 * i as usize) as i32
}

pub(crate) fn gpr_load<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, i: u8) -> Value {
    let off = gpr_offset::<SYSTEM>(i);
    builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, off)
}

pub(crate) fn gpr_load_or_zero<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, i: u8) -> Value {
    if i == 0 {
        builder.ins().iconst(types::I32, 0)
    } else {
        gpr_load::<SYSTEM>(builder, ctx_ptr, i)
    }
}

pub(crate) fn gpr_store<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, i: u8, val: Value) {
    let off = gpr_offset::<SYSTEM>(i);
    builder.ins().store(vmctx_flags(), val, ctx_ptr, off);
}

fn fpr_offset<const SYSTEM: SystemId>(i: u8) -> i32 {
    debug_assert!(i < 32);
    (abi::fpr_base_offset::<SYSTEM>() + 16 * i as usize) as i32
}

pub(crate) fn fpr_load<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, i: u8) -> Value {
    let off = fpr_offset::<SYSTEM>(i);
    builder.ins().load(types::F64, vmctx_flags(), ctx_ptr, off)
}

pub(crate) fn fpr_store<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, i: u8, val: Value) {
    let off = fpr_offset::<SYSTEM>(i);
    builder.ins().store(vmctx_flags(), val, ctx_ptr, off);
}

fn ps1_offset<const SYSTEM: SystemId>(i: u8) -> i32 {
    debug_assert!(i < 32);
    (abi::ps1_base_offset::<SYSTEM>() + 16 * i as usize) as i32
}

pub(crate) fn ps1_load<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, i: u8) -> Value {
    let off = ps1_offset::<SYSTEM>(i);
    builder.ins().load(types::F64, vmctx_flags(), ctx_ptr, off)
}

pub(crate) fn ps1_store<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, i: u8, val: Value) {
    let off = ps1_offset::<SYSTEM>(i);
    builder.ins().store(vmctx_flags(), val, ctx_ptr, off);
}

pub(crate) fn round_to_single(builder: &mut FunctionBuilder, val: Value) -> Value {
    let f32v = builder.ins().fdemote(types::F32, val);
    builder.ins().fpromote(types::F64, f32v)
}

pub(crate) fn round_frc_value(builder: &mut FunctionBuilder, c: Value) -> Value {
    use crate::gekko::interpreter::{FRC_KEEP_MASK, FRC_ROUND_BIT};

    let bits = builder.ins().bitcast(types::I64, MemFlagsData::new(), c);
    let keep = builder.ins().band_imm(bits, FRC_KEEP_MASK as i64);
    let round = builder.ins().band_imm(bits, FRC_ROUND_BIT as i64);
    let sum = builder.ins().iadd(keep, round);
    builder.ins().bitcast(types::F64, MemFlagsData::new(), sum)
}

pub(crate) fn neg_unless_nan(builder: &mut FunctionBuilder, v: Value) -> Value {
    use cranelift_codegen::ir::condcodes::FloatCC;

    let negged = builder.ins().fneg(v);
    let nan = builder.ins().fcmp(FloatCC::Unordered, v, v);
    builder.ins().select(nan, v, negged)
}

pub(crate) fn fpr_pair_load<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, i: u8) -> Value {
    let off = fpr_offset::<SYSTEM>(i);
    builder.ins().load(types::F64X2, vmctx_flags(), ctx_ptr, off)
}

pub(crate) fn fpr_pair_store<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, i: u8, v: Value) {
    let off = fpr_offset::<SYSTEM>(i);
    builder.ins().store(vmctx_flags(), v, ctx_ptr, off);
}

pub(crate) fn pair_round_to_single(builder: &mut FunctionBuilder, v: Value) -> Value {
    let f32v = builder.ins().fvdemote(v);
    builder.ins().fvpromote_low(f32v)
}

pub(crate) fn pair_round_frc(builder: &mut FunctionBuilder, c: Value) -> Value {
    use crate::gekko::interpreter::{FRC_KEEP_MASK, FRC_ROUND_BIT};

    let bits = builder.ins().bitcast(types::I64X2, MemFlagsData::new(), c);
    let keep_mask = builder.ins().iconst(types::I64, FRC_KEEP_MASK as i64);
    let keep_splat = builder.ins().splat(types::I64X2, keep_mask);
    let round_mask = builder.ins().iconst(types::I64, FRC_ROUND_BIT as i64);
    let round_splat = builder.ins().splat(types::I64X2, round_mask);
    let keep = builder.ins().band(bits, keep_splat);
    let round = builder.ins().band(bits, round_splat);
    let sum = builder.ins().iadd(keep, round);
    builder.ins().bitcast(types::F64X2, MemFlagsData::new(), sum)
}

pub(crate) fn pair_neg_unless_nan(builder: &mut FunctionBuilder, v: Value) -> Value {
    use cranelift_codegen::ir::condcodes::FloatCC;

    let negged = builder.ins().fneg(v);
    let nan = builder.ins().fcmp(FloatCC::Unordered, v, v);
    pair_bitselect(builder, nan, v, negged)
}

pub(crate) fn pair_bitselect(builder: &mut FunctionBuilder, mask: Value, x: Value, y: Value) -> Value {
    let xi = builder.ins().bitcast(types::I64X2, MemFlagsData::new(), x);
    let yi = builder.ins().bitcast(types::I64X2, MemFlagsData::new(), y);
    let ri = builder.ins().bitselect(mask, xi, yi);
    builder.ins().bitcast(types::F64X2, MemFlagsData::new(), ri)
}

pub(crate) fn build_pair(builder: &mut FunctionBuilder, ps0: Value, ps1: Value) -> Value {
    let v = builder.ins().scalar_to_vector(types::F64X2, ps0);
    builder.ins().insertlane(v, ps1, 1)
}

pub(crate) fn f32_reciprocal(builder: &mut FunctionBuilder, b_f64: Value) -> Value {
    let b32 = builder.ins().fdemote(types::F32, b_f64);
    let one32 = builder.ins().f32const(1.0f32);
    let q32 = builder.ins().fdiv(one32, b32);
    builder.ins().fpromote(types::F64, q32)
}

pub(crate) fn emit_recompute_fpscr_summary<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value) {
    const VX_MASK: u32 = 0x01F8_0700;
    let off = abi::fpscr_offset::<SYSTEM>() as i32;
    let fpscr = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, off);

    let vx_in = builder.ins().band_imm(fpscr, VX_MASK as i64);
    let vx_b = builder.ins().icmp_imm(IntCC::NotEqual, vx_in, 0);
    let vx_v = builder.ins().uextend(types::I32, vx_b);
    let vx_pos = builder.ins().ishl_imm(vx_v, 29);
    let cleared_vx = builder.ins().band_imm(fpscr, !(1i64 << 29));
    let with_vx = builder.ins().bor(cleared_vx, vx_pos);

    let upper = builder.ins().ushr_imm(with_vx, 22);
    let upper_mask = builder.ins().band_imm(upper, 0xF8);
    let lower_mask = builder.ins().band_imm(with_vx, 0xF8);
    let pairs = builder.ins().band(upper_mask, lower_mask);
    let fex_b = builder.ins().icmp_imm(IntCC::NotEqual, pairs, 0);
    let fex_v = builder.ins().uextend(types::I32, fex_b);
    let fex_pos = builder.ins().ishl_imm(fex_v, 30);
    let cleared_fex = builder.ins().band_imm(with_vx, !(1i64 << 30));
    let new_fpscr = builder.ins().bor(cleared_fex, fex_pos);

    builder.ins().store(vmctx_flags(), new_fpscr, ctx_ptr, off);
}

pub(crate) fn fpr_store_paired<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    i: u8,
    val: Value,
) {
    let v = builder.ins().splat(types::F64X2, val);
    fpr_pair_store::<SYSTEM>(builder, ctx_ptr, i, v);
}

pub(crate) fn cr_load<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value) -> Value {
    let off = abi::cr_offset::<SYSTEM>() as i32;
    builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, off)
}

pub(crate) fn cr_store<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, val: Value) {
    let off = abi::cr_offset::<SYSTEM>() as i32;
    builder.ins().store(vmctx_flags(), val, ctx_ptr, off);
}

pub(crate) fn emit_update_cr1_from_fpscr<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value) {
    let fpscr = builder
        .ins()
        .load(types::I32, vmctx_flags(), ctx_ptr, abi::fpscr_offset::<SYSTEM>() as i32);

    let top4 = builder.ins().ushr_imm(fpscr, 28);
    let positioned = builder.ins().ishl_imm(top4, 24);
    let cr = cr_load::<SYSTEM>(builder, ctx_ptr);
    let cleared = builder.ins().band_imm(cr, !(0xFi64 << 24));
    let new_cr = builder.ins().bor(cleared, positioned);
    cr_store::<SYSTEM>(builder, ctx_ptr, new_cr);
}

pub(crate) fn xer_load<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value) -> Value {
    let off = abi::xer_offset::<SYSTEM>() as i32;
    builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, off)
}

fn xer_so_bit<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value) -> Value {
    let xer = xer_load::<SYSTEM>(builder, ctx_ptr);
    let shifted = builder.ins().ushr_imm(xer, 31);
    builder.ins().band_imm(shifted, 1)
}

fn cr_set_field<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, crfd: u8, nibble: Value) {
    debug_assert!(crfd < 8);
    let shift = 28 - 4 * crfd as i64;
    let cr = cr_load::<SYSTEM>(builder, ctx_ptr);
    let mask = !(0xFu32 << shift) as i64 as u64 as i64;
    let cleared = builder.ins().band_imm(cr, mask);
    let nibble_only = builder.ins().band_imm(nibble, 0xF);
    let positioned = builder.ins().ishl_imm(nibble_only, shift);
    let merged = builder.ins().bor(cleared, positioned);
    cr_store::<SYSTEM>(builder, ctx_ptr, merged);
}

fn build_cr_nibble(builder: &mut FunctionBuilder, lt: Value, gt: Value, eq: Value, so: Value) -> Value {
    let lt8 = builder.ins().ishl_imm(lt, 3);
    let gt4 = builder.ins().ishl_imm(gt, 2);
    let eq2 = builder.ins().ishl_imm(eq, 1);
    let a = builder.ins().bor(lt8, gt4);
    let b = builder.ins().bor(eq2, so);
    builder.ins().bor(a, b)
}

fn cmp_to_i32(builder: &mut FunctionBuilder, cc: IntCC, a: Value, b: Value) -> Value {
    let r = builder.ins().icmp(cc, a, b);
    builder.ins().uextend(types::I32, r)
}

pub(crate) fn emit_update_cr0<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, val: Value) {
    let zero = builder.ins().iconst(types::I32, 0);
    let lt = cmp_to_i32(builder, IntCC::SignedLessThan, val, zero);
    let gt = cmp_to_i32(builder, IntCC::SignedGreaterThan, val, zero);
    let eq = cmp_to_i32(builder, IntCC::Equal, val, zero);
    let so = xer_so_bit::<SYSTEM>(builder, ctx_ptr);
    let nibble = build_cr_nibble(builder, lt, gt, eq, so);
    cr_set_field::<SYSTEM>(builder, ctx_ptr, 0, nibble);
}

#[derive(Clone, Copy)]
pub(crate) enum MemSize {
    U8,
    U16,
    U32,
}

impl MemSize {
    fn ir_type(self) -> cranelift_codegen::ir::Type {
        match self {
            MemSize::U8 => types::I8,
            MemSize::U16 => types::I16,
            MemSize::U32 => types::I32,
        }
    }
}

pub(crate) fn emit_d_form_ea<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let base = gpr_load_or_zero::<SYSTEM>(builder, ctx_ptr, instr.ra());
    builder.ins().iadd_imm(base, instr.simm() as i64)
}

pub(crate) fn emit_x_form_ea<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let base = gpr_load_or_zero::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    builder.ins().iadd(base, b)
}

pub(crate) fn emit_smc_check<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    size_bytes: u32,
    cause_smc_write: FuncRef,
) {
    let phys = builder.ins().band_imm(ea, crate::mmio::PHYS_MASK as i64);
    let in_mem1 = builder
        .ins()
        .icmp_imm(IntCC::UnsignedLessThan, phys, crate::mmio::constants::RAM_SIZE as i64);

    let check_block = builder.create_block();
    let post_block = builder.create_block();
    builder.ins().brif(in_mem1, check_block, &[], post_block, &[]);

    builder.switch_to_block(check_block);
    builder.seal_block(check_block);

    let rc_off = abi::code_refcount_ptr_offset::<SYSTEM>() as i32;
    let rc_base = builder.ins().load(types::I64, vmctx_readonly_flags(), ctx_ptr, rc_off);

    let lo_idx_u32 = builder.ins().ushr_imm(phys, crate::mmio::CODE_LINE_SHIFT as i64);
    let lo_idx = builder.ins().uextend(types::I64, lo_idx_u32);
    let lo_addr = builder.ins().iadd(rc_base, lo_idx);
    let rc_lo = builder.ins().load(types::I8, vmctx_flags(), lo_addr, 0);

    let combined = if size_bytes <= 1 {
        rc_lo
    } else {
        let hi_phys = builder.ins().iadd_imm(phys, (size_bytes as i64) - 1);
        let hi_idx_u32 = builder.ins().ushr_imm(hi_phys, crate::mmio::CODE_LINE_SHIFT as i64);
        let hi_idx = builder.ins().uextend(types::I64, hi_idx_u32);
        let hi_addr = builder.ins().iadd(rc_base, hi_idx);
        let rc_hi = builder.ins().load(types::I8, vmctx_flags(), hi_addr, 0);
        builder.ins().bor(rc_lo, rc_hi)
    };
    let nz = builder.ins().icmp_imm(IntCC::NotEqual, combined, 0);

    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    builder.ins().brif(nz, slow_block, &[], post_block, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let size_v = builder.ins().iconst(types::I32, size_bytes as i64);
    builder.ins().call(cause_smc_write, &[ctx_ptr, ea, size_v]);
    builder.ins().jump(post_block, &[]);

    builder.switch_to_block(post_block);
    builder.seal_block(post_block);
}

pub(crate) fn emit_fastmem_lookup<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
) -> (Value, Value) {
    let lut_base = {
        let off = abi::fastmem_lut_ptr_offset::<SYSTEM>() as i32;
        builder.ins().load(types::I64, vmctx_readonly_flags(), ctx_ptr, off)
    };

    let idx_u32 = builder.ins().ushr_imm(ea, crate::mmio::FASTMEM_PAGE_SHIFT as i64);
    let idx = builder.ins().uextend(types::I64, idx_u32);
    let entry_off = builder.ins().ishl_imm(idx, 3);
    let entry_addr = builder.ins().iadd(lut_base, entry_off);
    let host_ptr = builder.ins().load(types::I64, vmctx_flags(), entry_addr, 0);

    let page_off_u32 = builder.ins().band_imm(ea, crate::mmio::FASTMEM_PAGE_MASK as i64);
    let page_off = builder.ins().uextend(types::I64, page_off_u32);

    (host_ptr, page_off)
}

pub(crate) fn emit_load<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    size: MemSize,
    slow: FuncRef,
    update: bool,
) {
    let ea = emit_d_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    emit_load_at_ea::<SYSTEM>(builder, ctx_ptr, ea, instr.rd(), instr.ra(), size, slow, update);
}

pub(crate) fn emit_lha_d_form<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    slow: FuncRef,
    update: bool,
) {
    let ea = emit_d_form_ea::<SYSTEM>(builder, ctx_ptr, instr);

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();
    builder.append_block_param(merge, types::I32);

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let raw_val = builder.ins().load(types::I16, heap_flags(), addr, 0);
    let swapped = builder.ins().bswap(raw_val);

    let val32 = builder.ins().sextend(types::I32, swapped);
    builder.ins().jump(merge, &[val32.into()]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let call = builder.ins().call(slow, &[ctx_ptr, ea]);
    let raw_u16_in_u32 = builder.inst_results(call)[0];

    let trunc = builder.ins().ireduce(types::I16, raw_u16_in_u32);
    let sext = builder.ins().sextend(types::I32, trunc);
    builder.ins().jump(merge, &[sext.into()]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
    let val = builder.block_params(merge)[0];
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), val);

    if update {
        gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), ea);
    }
}

pub(crate) fn emit_lfx_update<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    single: bool,
    local: &BlockEmitContext,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    if single {
        emit_lfs_at_ea::<SYSTEM>(builder, ctx_ptr, instr.rd(), ea, local.read_f32);
    } else {
        emit_lfd_at_ea::<SYSTEM>(builder, ctx_ptr, instr.rd(), ea, local.read_f64);
    }
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), ea);
}

pub(crate) fn emit_stfx_update<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    single: bool,
    local: &BlockEmitContext,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    if single {
        emit_stfs_at_ea::<SYSTEM>(builder, ctx_ptr, instr.rs(), ea, local.write_f32, local.cause_smc_write);
    } else {
        emit_stfd_at_ea::<SYSTEM>(builder, ctx_ptr, instr.rs(), ea, local.write_f64, local.cause_smc_write);
    }
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), ea);
}

pub(crate) fn emit_stfiwx<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    slow_u32: FuncRef,
    cause_smc_write: FuncRef,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);

    let f = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    let u64v = builder.ins().bitcast(types::I64, MemFlagsData::new(), f);
    let u32v = builder.ins().ireduce(types::I32, u64v);

    emit_u32_store_at_ea::<SYSTEM>(builder, ctx_ptr, ea, u32v, slow_u32, cause_smc_write);
}

pub(crate) fn emit_u32_store_at_ea<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    val: Value,
    slow: FuncRef,
    cause_smc_write: FuncRef,
) {
    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let bswapped = builder.ins().bswap(val);
    builder.ins().store(heap_flags(), bswapped, addr, 0);
    emit_smc_check::<SYSTEM>(builder, ctx_ptr, ea, 4, cause_smc_write);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    builder.ins().call(slow, &[ctx_ptr, ea, val]);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
}

pub(crate) fn emit_psq_x<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    local: &mut BlockEmitContext,
    store: bool,
    update: bool,
) {
    let base = gpr_load_or_zero::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    let ea = builder.ins().iadd(base, b);

    let easy = emit_gqr_type_zero_check::<SYSTEM>(builder, ctx_ptr, local, instr.i_22_24(), store);
    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();
    builder.ins().brif(easy, fast_block, &[], slow_block, &[]);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    if store {
        if instr.w_21_21() {
            let ps0 = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rd());
            emit_write_f32::<SYSTEM>(builder, ctx_ptr, ea, ps0, local.write_f32, local.cause_smc_write);
        } else {
            let pair = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rd());
            emit_write_ps_pair::<SYSTEM>(builder, ctx_ptr, ea, pair, local.write_f32, local.cause_smc_write);
        }
    } else if instr.w_21_21() {
        let ps0 = emit_read_f32::<SYSTEM>(builder, ctx_ptr, ea, local.read_f32);
        let one = builder.ins().f64const(1.0);
        let pair = build_pair(builder, ps0, one);
        fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), pair);
    } else {
        let pair = emit_read_ps_pair::<SYSTEM>(builder, ctx_ptr, ea, local.read_f32);
        fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), pair);
    }
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    if store {
        emit_psq_store_quantized::<SYSTEM>(
            builder,
            ctx_ptr,
            ea,
            instr.rd(),
            instr.w_21_21(),
            instr.i_22_24(),
            merge,
            local,
        );
    } else {
        emit_psq_load_quantized::<SYSTEM>(
            builder,
            ctx_ptr,
            ea,
            instr.rd(),
            instr.w_21_21(),
            instr.i_22_24(),
            merge,
            local,
        );
    }

    builder.switch_to_block(merge);
    builder.seal_block(merge);

    if update {
        gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), ea);
    }
}

pub(crate) fn emit_lwarx<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    local: &BlockEmitContext,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    emit_load_at_ea::<SYSTEM>(
        builder,
        ctx_ptr,
        ea,
        instr.rd(),
        instr.ra(),
        MemSize::U32,
        local.read_u32,
        false,
    );
    let reserve_off = abi::reserve_addr_offset::<SYSTEM>() as i32;
    builder.ins().store(vmctx_flags(), ea, ctx_ptr, reserve_off);
}

pub(crate) fn emit_stwcx_dot<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    local: &BlockEmitContext,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);

    let reserve_off = abi::reserve_addr_offset::<SYSTEM>() as i32;
    let reserved = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, reserve_off);
    let matched_b = builder.ins().icmp(IntCC::Equal, reserved, ea);
    let matched = builder.ins().uextend(types::I32, matched_b);

    let store_block = builder.create_block();
    let cr_block = builder.create_block();
    builder.append_block_param(cr_block, types::I32);
    builder
        .ins()
        .brif(matched_b, store_block, &[], cr_block, &[matched.into()]);

    builder.switch_to_block(store_block);
    builder.seal_block(store_block);

    let no_reservation = builder
        .ins()
        .iconst(types::I32, crate::gekko::Gekko::NO_RESERVATION as i64);
    builder.ins().store(vmctx_flags(), no_reservation, ctx_ptr, reserve_off);
    emit_store_at_ea::<SYSTEM>(
        builder,
        ctx_ptr,
        ea,
        instr.rs(),
        0,
        MemSize::U32,
        local.write_u32,
        local.cause_smc_write,
        false,
    );
    builder.ins().jump(cr_block, &[matched.into()]);

    builder.switch_to_block(cr_block);
    builder.seal_block(cr_block);
    let eq_flag = builder.block_params(cr_block)[0];

    let xer = xer_load::<SYSTEM>(builder, ctx_ptr);
    let so = builder.ins().ushr_imm(xer, 31);
    let so_clean = builder.ins().band_imm(so, 1);
    let eq_shifted = builder.ins().ishl_imm(eq_flag, 1);
    let nibble = builder.ins().bor(eq_shifted, so_clean);
    let positioned = builder.ins().ishl_imm(nibble, 28);
    let cr = cr_load::<SYSTEM>(builder, ctx_ptr);
    let cleared = builder.ins().band_imm(cr, !(0xFi64 << 28));
    let new_cr = builder.ins().bor(cleared, positioned);
    cr_store::<SYSTEM>(builder, ctx_ptr, new_cr);
}

pub(crate) fn emit_lmw_stmw<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    local: &BlockEmitContext,
    store: bool,
) {
    let ea_base = emit_d_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    for r in instr.rd()..32 {
        let off = ((r - instr.rd()) as i32) * 4;
        let ea = if off == 0 {
            ea_base
        } else {
            builder.ins().iadd_imm(ea_base, off as i64)
        };
        if store {
            emit_store_at_ea::<SYSTEM>(
                builder,
                ctx_ptr,
                ea,
                r,
                0,
                MemSize::U32,
                local.write_u32,
                local.cause_smc_write,
                false,
            );
        } else {
            emit_load_at_ea::<SYSTEM>(builder, ctx_ptr, ea, r, 0, MemSize::U32, local.read_u32, false);
        }
    }
}

pub(crate) fn emit_lha_xform<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    slow: FuncRef,
    update: bool,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();
    builder.append_block_param(merge, types::I32);

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let raw_val = builder.ins().load(types::I16, heap_flags(), addr, 0);
    let swapped = builder.ins().bswap(raw_val);
    let val32 = builder.ins().sextend(types::I32, swapped);
    builder.ins().jump(merge, &[val32.into()]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let call = builder.ins().call(slow, &[ctx_ptr, ea]);
    let raw_u16_in_u32 = builder.inst_results(call)[0];
    let trunc = builder.ins().ireduce(types::I16, raw_u16_in_u32);
    let sext = builder.ins().sextend(types::I32, trunc);
    builder.ins().jump(merge, &[sext.into()]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
    let val = builder.block_params(merge)[0];
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), val);
    if update {
        gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), ea);
    }
}

pub(crate) fn emit_load_xform_brx<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    size: MemSize,
    slow: FuncRef,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();
    builder.append_block_param(merge, types::I32);

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let raw_val = builder.ins().load(size.ir_type(), heap_flags(), addr, 0);
    let val32 = match size {
        MemSize::U32 => raw_val,
        MemSize::U16 | MemSize::U8 => builder.ins().uextend(types::I32, raw_val),
    };
    builder.ins().jump(merge, &[val32.into()]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let call = builder.ins().call(slow, &[ctx_ptr, ea]);
    let raw_be_in_u32 = builder.inst_results(call)[0];

    let swapped = match size {
        MemSize::U32 => builder.ins().bswap(raw_be_in_u32),
        MemSize::U16 => {
            let trunc = builder.ins().ireduce(types::I16, raw_be_in_u32);
            let sw = builder.ins().bswap(trunc);
            builder.ins().uextend(types::I32, sw)
        }
        MemSize::U8 => raw_be_in_u32,
    };
    builder.ins().jump(merge, &[swapped.into()]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
    let val = builder.block_params(merge)[0];
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), val);
}

pub(crate) fn emit_store_xform_brx<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    size: MemSize,
    slow: FuncRef,
    cause_smc_write: FuncRef,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    let val32 = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];

    let truncated = match size {
        MemSize::U32 => val32,
        MemSize::U16 => builder.ins().ireduce(types::I16, val32),
        MemSize::U8 => builder.ins().ireduce(types::I8, val32),
    };
    builder.ins().store(heap_flags(), truncated, addr, 0);
    let size_bytes = match size {
        MemSize::U8 => 1,
        MemSize::U16 => 2,
        MemSize::U32 => 4,
    };
    emit_smc_check::<SYSTEM>(builder, ctx_ptr, ea, size_bytes, cause_smc_write);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);

    let to_write = match size {
        MemSize::U32 => builder.ins().bswap(val32),
        MemSize::U16 => {
            let trunc = builder.ins().ireduce(types::I16, val32);
            let sw = builder.ins().bswap(trunc);
            builder.ins().uextend(types::I32, sw)
        }
        MemSize::U8 => val32,
    };
    builder.ins().call(slow, &[ctx_ptr, ea, to_write]);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
}

pub(crate) fn emit_exception_dispatch<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    srr0_value: Value,
    srr1_extra_bits: u32,
    vector_offset: u32,
) -> Value {
    const MSR_CLEAR_MASK: i64 = 0x4_EF37;

    const MSR_PRESERVE_MASK: i64 = 0x87C0_FFFF;

    let msr_off = abi::msr_offset::<SYSTEM>() as i32;
    let srr0_off = abi::spr_field_offset::<SYSTEM>(26).unwrap() as i32;
    let srr1_off = abi::spr_field_offset::<SYSTEM>(27).unwrap() as i32;
    let nia_off = abi::nia_offset::<SYSTEM>() as i32;

    let msr = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, msr_off);

    builder.ins().store(vmctx_flags(), srr0_value, ctx_ptr, srr0_off);

    let preserved = builder.ins().band_imm(msr, MSR_PRESERVE_MASK);
    let srr1 = if srr1_extra_bits != 0 {
        builder.ins().bor_imm(preserved, srr1_extra_bits as i64)
    } else {
        preserved
    };
    builder.ins().store(vmctx_flags(), srr1, ctx_ptr, srr1_off);

    let ip_bit = builder.ins().band_imm(msr, 0x40);
    let want_high = builder.ins().icmp_imm(IntCC::NotEqual, ip_bit, 0);
    let high_base = builder.ins().iconst(types::I32, 0xFFF0_0000i64);
    let zero = builder.ins().iconst(types::I32, 0);
    let base = builder.ins().select(want_high, high_base, zero);
    let nia = builder.ins().bor_imm(base, vector_offset as i64);
    builder.ins().store(vmctx_flags(), nia, ctx_ptr, nia_off);

    let msr_cleared = builder.ins().band_imm(msr, !MSR_CLEAR_MASK & 0xFFFF_FFFFi64);
    let ile_shifted = builder.ins().ushr_imm(msr, 16);
    let ile_bit = builder.ins().band_imm(ile_shifted, 1);
    let new_msr = builder.ins().bor(msr_cleared, ile_bit);
    builder.ins().store(vmctx_flags(), new_msr, ctx_ptr, msr_off);

    nia
}

pub(crate) fn emit_msr_fp_guard<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    _instr: Instruction,
    pc: u32,
    exit_block: Block,
    local: &mut BlockEmitContext,
    op_cycles: i64,
) {
    if local.fp_guard_emitted {
        return;
    }
    local.fp_guard_emitted = true;

    let msr_off = abi::msr_offset::<SYSTEM>() as i32;
    let msr = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, msr_off);
    let fp_bit = builder.ins().band_imm(msr, 0x2000);
    let fp_disabled = builder.ins().icmp_imm(IntCC::Equal, fp_bit, 0);
    let trap_block = builder.create_block();
    builder.set_cold_block(trap_block);
    let cont_block = builder.create_block();
    builder.ins().brif(fp_disabled, trap_block, &[], cont_block, &[]);

    builder.switch_to_block(trap_block);
    builder.seal_block(trap_block);

    let _ = local;
    let cia_off = abi::cia_offset::<SYSTEM>() as i32;
    let pc_const = builder.ins().iconst(types::I32, pc as i64);
    builder.ins().store(vmctx_flags(), pc_const, ctx_ptr, cia_off);
    let nia_off = abi::nia_offset::<SYSTEM>() as i32;
    let pc_plus4 = builder.ins().iconst(types::I32, pc.wrapping_add(4) as i64);
    builder.ins().store(vmctx_flags(), pc_plus4, ctx_ptr, nia_off);
    let nia = emit_exception_dispatch::<SYSTEM>(builder, ctx_ptr, pc_const, 0, 0x0000_0800);
    emit_cycles_add::<SYSTEM>(builder, ctx_ptr, op_cycles);
    builder.ins().jump(exit_block, &[nia.into()]);

    builder.switch_to_block(cont_block);
    builder.seal_block(cont_block);
}

pub(crate) fn emit_lf_d_form_update<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    single: bool,
    local: &BlockEmitContext,
) {
    let ea = emit_d_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    if single {
        emit_lfs_at_ea::<SYSTEM>(builder, ctx_ptr, instr.rd(), ea, local.read_f32);
    } else {
        emit_lfd_at_ea::<SYSTEM>(builder, ctx_ptr, instr.rd(), ea, local.read_f64);
    }
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), ea);
}

pub(crate) fn emit_stf_d_form_update<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    single: bool,
    local: &BlockEmitContext,
) {
    let ea = emit_d_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    if single {
        emit_stfs_at_ea::<SYSTEM>(builder, ctx_ptr, instr.rs(), ea, local.write_f32, local.cause_smc_write);
    } else {
        emit_stfd_at_ea::<SYSTEM>(builder, ctx_ptr, instr.rs(), ea, local.write_f64, local.cause_smc_write);
    }
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), ea);
}

pub(crate) fn emit_lfs<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    slow_f32: FuncRef,
    _slow_u32: FuncRef,
) {
    let ea = emit_d_form_ea::<SYSTEM>(builder, ctx_ptr, instr);

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();
    builder.append_block_param(merge, types::F64);

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let f64v = emit_direct_read_f32(builder, addr);
    builder.ins().jump(merge, &[f64v.into()]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let call = builder.ins().call(slow_f32, &[ctx_ptr, ea]);
    let f64v_slow = builder.inst_results(call)[0];
    builder.ins().jump(merge, &[f64v_slow.into()]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
    let val = builder.block_params(merge)[0];

    fpr_store_paired::<SYSTEM>(builder, ctx_ptr, instr.rd(), val);
}

pub(crate) fn emit_lfd<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    slow_f64: FuncRef,
) {
    let ea = emit_d_form_ea::<SYSTEM>(builder, ctx_ptr, instr);

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();
    builder.append_block_param(merge, types::F64);

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let raw_u64 = builder.ins().load(types::I64, heap_flags(), addr, 0);
    let be_u64 = builder.ins().bswap(raw_u64);
    let f64v = builder.ins().bitcast(types::F64, MemFlagsData::new(), be_u64);
    builder.ins().jump(merge, &[f64v.into()]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let call = builder.ins().call(slow_f64, &[ctx_ptr, ea]);
    let f64v_slow = builder.inst_results(call)[0];
    builder.ins().jump(merge, &[f64v_slow.into()]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
    let val = builder.block_params(merge)[0];
    fpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), val);
}

pub(crate) fn emit_stfs<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    slow_f32: FuncRef,
    _slow_u32: FuncRef,
    cause_smc_write: FuncRef,
) {
    let ea = emit_d_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    let f64v = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    emit_direct_write_f32(builder, addr, f64v);
    emit_smc_check::<SYSTEM>(builder, ctx_ptr, ea, 4, cause_smc_write);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    builder.ins().call(slow_f32, &[ctx_ptr, ea, f64v]);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
}

pub(crate) fn emit_stfd<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    slow_f64: FuncRef,
    cause_smc_write: FuncRef,
) {
    let ea = emit_d_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    let f64v = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let bits = builder.ins().bitcast(types::I64, MemFlagsData::new(), f64v);
    let swapped = builder.ins().bswap(bits);
    builder.ins().store(heap_flags(), swapped, addr, 0);
    emit_smc_check::<SYSTEM>(builder, ctx_ptr, ea, 8, cause_smc_write);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    builder.ins().call(slow_f64, &[ctx_ptr, ea, f64v]);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
}

pub(crate) fn emit_lfsx<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    slow_f32: FuncRef,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    emit_lfs_at_ea::<SYSTEM>(builder, ctx_ptr, instr.rd(), ea, slow_f32);
}

pub(crate) fn emit_lfdx<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    slow_f64: FuncRef,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    emit_lfd_at_ea::<SYSTEM>(builder, ctx_ptr, instr.rd(), ea, slow_f64);
}

pub(crate) fn emit_stfsx<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    slow_f32: FuncRef,
    cause_smc_write: FuncRef,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    emit_stfs_at_ea::<SYSTEM>(builder, ctx_ptr, instr.rs(), ea, slow_f32, cause_smc_write);
}

pub(crate) fn emit_stfdx<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    slow_f64: FuncRef,
    cause_smc_write: FuncRef,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    emit_stfd_at_ea::<SYSTEM>(builder, ctx_ptr, instr.rs(), ea, slow_f64, cause_smc_write);
}

pub(crate) fn emit_lfs_at_ea<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    rd: u8,
    ea: Value,
    slow_f32: FuncRef,
) {
    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();
    builder.append_block_param(merge, types::F64);

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let raw_u32 = builder.ins().load(types::I32, heap_flags(), addr, 0);
    let be_u32 = builder.ins().bswap(raw_u32);
    let f32v = builder.ins().bitcast(types::F32, MemFlagsData::new(), be_u32);
    let f64v = builder.ins().fpromote(types::F64, f32v);
    builder.ins().jump(merge, &[f64v.into()]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let call = builder.ins().call(slow_f32, &[ctx_ptr, ea]);
    let f64v_slow = builder.inst_results(call)[0];
    builder.ins().jump(merge, &[f64v_slow.into()]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
    let val = builder.block_params(merge)[0];
    fpr_store_paired::<SYSTEM>(builder, ctx_ptr, rd, val);
}

pub(crate) fn emit_lfd_at_ea<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    rd: u8,
    ea: Value,
    slow_f64: FuncRef,
) {
    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();
    builder.append_block_param(merge, types::F64);

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let raw_u64 = builder.ins().load(types::I64, heap_flags(), addr, 0);
    let be_u64 = builder.ins().bswap(raw_u64);
    let f64v = builder.ins().bitcast(types::F64, MemFlagsData::new(), be_u64);
    builder.ins().jump(merge, &[f64v.into()]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let call = builder.ins().call(slow_f64, &[ctx_ptr, ea]);
    let f64v_slow = builder.inst_results(call)[0];
    builder.ins().jump(merge, &[f64v_slow.into()]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
    let val = builder.block_params(merge)[0];
    fpr_store::<SYSTEM>(builder, ctx_ptr, rd, val);
}

pub(crate) fn emit_stfs_at_ea<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    rs: u8,
    ea: Value,
    slow_f32: FuncRef,
    cause_smc_write: FuncRef,
) {
    let f64v = fpr_load::<SYSTEM>(builder, ctx_ptr, rs);

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let f32v = builder.ins().fdemote(types::F32, f64v);
    let bits = builder.ins().bitcast(types::I32, MemFlagsData::new(), f32v);
    let swapped = builder.ins().bswap(bits);
    builder.ins().store(heap_flags(), swapped, addr, 0);
    emit_smc_check::<SYSTEM>(builder, ctx_ptr, ea, 4, cause_smc_write);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    builder.ins().call(slow_f32, &[ctx_ptr, ea, f64v]);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
}

pub(crate) fn emit_stfd_at_ea<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    rs: u8,
    ea: Value,
    slow_f64: FuncRef,
    cause_smc_write: FuncRef,
) {
    let f64v = fpr_load::<SYSTEM>(builder, ctx_ptr, rs);

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let bits = builder.ins().bitcast(types::I64, MemFlagsData::new(), f64v);
    let swapped = builder.ins().bswap(bits);
    builder.ins().store(heap_flags(), swapped, addr, 0);
    emit_smc_check::<SYSTEM>(builder, ctx_ptr, ea, 8, cause_smc_write);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    builder.ins().call(slow_f64, &[ctx_ptr, ea, f64v]);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
}

fn emit_direct_read_f32(builder: &mut FunctionBuilder, addr: Value) -> Value {
    let raw = builder.ins().load(types::I32, heap_flags(), addr, 0);
    let be = builder.ins().bswap(raw);
    let value = builder.ins().bitcast(types::F32, MemFlagsData::new(), be);

    builder.ins().fpromote(types::F64, value)
}

fn emit_direct_read_ps_pair(builder: &mut FunctionBuilder, addr: Value) -> Value {
    let raw = builder.ins().load(types::I64, heap_flags(), addr, 0);
    let swapped = builder.ins().bswap(raw);
    let rotated = builder.ins().rotl_imm(swapped, 32);
    let vector = builder.ins().scalar_to_vector(types::I64X2, rotated);
    let f32s = builder.ins().bitcast(types::F32X4, le_vec_flags(), vector);

    builder.ins().fvpromote_low(f32s)
}

fn emit_direct_write_f32(builder: &mut FunctionBuilder, addr: Value, value: Value) {
    let value = builder.ins().fdemote(types::F32, value);
    let bits = builder.ins().bitcast(types::I32, MemFlagsData::new(), value);
    let swapped = builder.ins().bswap(bits);
    builder.ins().store(heap_flags(), swapped, addr, 0);
}

fn emit_direct_write_ps_pair(builder: &mut FunctionBuilder, addr: Value, pair: Value) {
    let f32s = builder.ins().fvdemote(pair);
    let vector = builder.ins().bitcast(types::I64X2, le_vec_flags(), f32s);
    let low = builder.ins().extractlane(vector, 0);
    let rotated = builder.ins().rotl_imm(low, 32);
    let swapped = builder.ins().bswap(rotated);
    builder.ins().store(heap_flags(), swapped, addr, 0);
}

pub(crate) fn emit_psq_l<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    pc: u32,
    exit_block: Block,
    local: &mut BlockEmitContext,
    update: bool,
) {
    let base = gpr_load_or_zero::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let ea = builder.ins().iadd_imm(base, instr.d_20_31() as i64);

    let easy = emit_gqr_type_zero_check::<SYSTEM>(builder, ctx_ptr, local, instr.i_17_19(), false);

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();
    builder.ins().brif(easy, fast_block, &[], slow_block, &[]);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    if instr.w_16_16() {
        let ps0 = emit_read_f32::<SYSTEM>(builder, ctx_ptr, ea, local.read_f32);
        let one = builder.ins().f64const(1.0);
        let pair = build_pair(builder, ps0, one);
        fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), pair);
    } else {
        let pair = emit_read_ps_pair::<SYSTEM>(builder, ctx_ptr, ea, local.read_f32);
        fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), pair);
    }
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let _ = pc;
    let _ = exit_block;
    emit_psq_load_quantized::<SYSTEM>(
        builder,
        ctx_ptr,
        ea,
        instr.rd(),
        instr.w_16_16(),
        instr.i_17_19(),
        merge,
        local,
    );

    builder.switch_to_block(merge);
    builder.seal_block(merge);

    if update {
        gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), ea);
    }
}

pub(crate) fn emit_psq_st<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    pc: u32,
    exit_block: Block,
    local: &mut BlockEmitContext,
    update: bool,
) {
    let base = gpr_load_or_zero::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let ea = builder.ins().iadd_imm(base, instr.d_20_31() as i64);

    let easy = emit_gqr_type_zero_check::<SYSTEM>(builder, ctx_ptr, local, instr.i_17_19(), true);

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();
    builder.ins().brif(easy, fast_block, &[], slow_block, &[]);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    if instr.w_16_16() {
        let ps0 = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
        emit_write_f32::<SYSTEM>(builder, ctx_ptr, ea, ps0, local.write_f32, local.cause_smc_write);
    } else {
        let pair = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
        emit_write_ps_pair::<SYSTEM>(builder, ctx_ptr, ea, pair, local.write_f32, local.cause_smc_write);
    }
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let _ = pc;
    let _ = exit_block;
    emit_psq_store_quantized::<SYSTEM>(
        builder,
        ctx_ptr,
        ea,
        instr.rs(),
        instr.w_16_16(),
        instr.i_17_19(),
        merge,
        local,
    );

    builder.switch_to_block(merge);
    builder.seal_block(merge);

    if update {
        gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), ea);
    }
}

pub(crate) fn emit_gqr_type_zero_check<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    local: &mut BlockEmitContext,
    gqr_idx: u8,
    store: bool,
) -> Value {
    let gqr = gqr_load::<SYSTEM>(builder, ctx_ptr, local, gqr_idx);
    let mask = if store { 0x7 } else { 0x70000 };
    let bits = builder.ins().band_imm(gqr, mask);
    builder.ins().icmp_imm(IntCC::Equal, bits, 0)
}

pub(crate) fn emit_psq_dequant_element(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    type_v: Value,
    scale_f32: Value,
    read_u8_fn: FuncRef,
    read_u16_fn: FuncRef,
) -> Value {
    let half_bit = builder.ins().band_imm(type_v, 1);
    let is_half = builder.ins().icmp_imm(IntCC::NotEqual, half_bit, 0);
    let signed_bit = builder.ins().band_imm(type_v, 2);
    let is_signed = builder.ins().icmp_imm(IntCC::NotEqual, signed_bit, 0);

    let byte_block = builder.create_block();
    let half_block = builder.create_block();
    let merge = builder.create_block();
    builder.append_block_param(merge, types::F32);

    builder.ins().brif(is_half, half_block, &[], byte_block, &[]);

    builder.switch_to_block(byte_block);
    builder.seal_block(byte_block);
    let call = builder.ins().call(read_u8_fn, &[ctx_ptr, ea]);
    let raw_byte = builder.inst_results(call)[0];
    let trunc = builder.ins().ireduce(types::I8, raw_byte);
    let sext = builder.ins().sextend(types::I32, trunc);
    let val_i32 = builder.ins().select(is_signed, sext, raw_byte);
    let val_f32 = builder.ins().fcvt_from_sint(types::F32, val_i32);
    let scaled = builder.ins().fmul(val_f32, scale_f32);
    builder.ins().jump(merge, &[scaled.into()]);

    builder.switch_to_block(half_block);
    builder.seal_block(half_block);
    let call = builder.ins().call(read_u16_fn, &[ctx_ptr, ea]);
    let raw_half = builder.inst_results(call)[0];
    let trunc = builder.ins().ireduce(types::I16, raw_half);
    let sext = builder.ins().sextend(types::I32, trunc);
    let val_i32 = builder.ins().select(is_signed, sext, raw_half);
    let val_f32 = builder.ins().fcvt_from_sint(types::F32, val_i32);
    let scaled = builder.ins().fmul(val_f32, scale_f32);
    builder.ins().jump(merge, &[scaled.into()]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
    let scaled_f32 = builder.block_params(merge)[0];
    builder.ins().fpromote(types::F64, scaled_f32)
}

pub(crate) fn emit_psq_quant_element(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    value_f64: Value,
    type_v: Value,
    scale_f32: Value,
    write_u8_fn: FuncRef,
    write_u16_fn: FuncRef,
) {
    let val_f32 = builder.ins().fdemote(types::F32, value_f64);
    let scaled_f32 = builder.ins().fmul(val_f32, scale_f32);

    let half_bit = builder.ins().band_imm(type_v, 1);
    let is_half = builder.ins().icmp_imm(IntCC::NotEqual, half_bit, 0);
    let signed_bit = builder.ins().band_imm(type_v, 2);
    let is_signed = builder.ins().icmp_imm(IntCC::NotEqual, signed_bit, 0);

    let zero_f = builder.ins().f32const(0.0_f32);
    let u8_max = builder.ins().f32const(255.0_f32);
    let u16_max = builder.ins().f32const(65535.0_f32);
    let i8_min = builder.ins().f32const(-128.0_f32);
    let i8_max = builder.ins().f32const(127.0_f32);
    let i16_min = builder.ins().f32const(-32768.0_f32);
    let i16_max = builder.ins().f32const(32767.0_f32);

    let signed_min = builder.ins().select(is_half, i16_min, i8_min);
    let min_f32 = builder.ins().select(is_signed, signed_min, zero_f);
    let signed_max = builder.ins().select(is_half, i16_max, i8_max);
    let unsigned_max = builder.ins().select(is_half, u16_max, u8_max);
    let max_f32 = builder.ins().select(is_signed, signed_max, unsigned_max);

    let lo_clamped = builder.ins().fmax(scaled_f32, min_f32);
    let clamped = builder.ins().fmin(lo_clamped, max_f32);

    let val_i32 = builder.ins().fcvt_to_sint_sat(types::I32, clamped);

    let mask_byte = builder.ins().iconst(types::I32, 0xFFi64);
    let mask_half = builder.ins().iconst(types::I32, 0xFFFFi64);
    let mask = builder.ins().select(is_half, mask_half, mask_byte);
    let val_to_write = builder.ins().band(val_i32, mask);

    let byte_block = builder.create_block();
    let half_block = builder.create_block();
    let merge = builder.create_block();
    builder.ins().brif(is_half, half_block, &[], byte_block, &[]);

    builder.switch_to_block(byte_block);
    builder.seal_block(byte_block);
    builder.ins().call(write_u8_fn, &[ctx_ptr, ea, val_to_write]);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(half_block);
    builder.seal_block(half_block);
    builder.ins().call(write_u16_fn, &[ctx_ptr, ea, val_to_write]);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
}

pub(crate) fn emit_psq_elem_size(builder: &mut FunctionBuilder, type_v: Value) -> Value {
    let half_bit = builder.ins().band_imm(type_v, 1);
    let is_half = builder.ins().icmp_imm(IntCC::NotEqual, half_bit, 0);
    let two = builder.ins().iconst(types::I32, 2);
    let one = builder.ins().iconst(types::I32, 1);
    builder.ins().select(is_half, two, one)
}

pub(crate) fn emit_psq_load_quantized<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    fd: u8,
    w: bool,
    gqr_idx: u8,
    merge: Block,
    local: &mut BlockEmitContext,
) {
    let gqr_v = gqr_load::<SYSTEM>(builder, ctx_ptr, local, gqr_idx);
    let type_shifted = builder.ins().ushr_imm(gqr_v, 16);
    let type_v = builder.ins().band_imm(type_shifted, 0x7);

    let q_bit = builder.ins().band_imm(type_v, 0x4);
    let is_quant = builder.ins().icmp_imm(IntCC::NotEqual, q_bit, 0);
    let inline_block = builder.create_block();
    let tramp_block = builder.create_block();
    builder.ins().brif(is_quant, inline_block, &[], tramp_block, &[]);

    builder.switch_to_block(inline_block);
    builder.seal_block(inline_block);
    let scale_idx_shifted = builder.ins().ushr_imm(gqr_v, 24);
    let scale_idx = builder.ins().band_imm(scale_idx_shifted, 0x3F);
    let scale_idx_64 = builder.ins().uextend(types::I64, scale_idx);
    let scale_offset = builder.ins().ishl_imm(scale_idx_64, 2);
    let table_addr = builder
        .ins()
        .iconst(types::I64, crate::gekko::jit::runtime::DEQUANT_TABLE.as_ptr() as i64);
    let scale_addr = builder.ins().iadd(table_addr, scale_offset);
    let scale_f32 = builder.ins().load(types::F32, vmctx_flags(), scale_addr, 0);

    let ps0 = emit_psq_dequant_element(builder, ctx_ptr, ea, type_v, scale_f32, local.read_u8, local.read_u16);
    let ps1 = if w {
        builder.ins().f64const(1.0)
    } else {
        let elem_size = emit_psq_elem_size(builder, type_v);
        let ea1 = builder.ins().iadd(ea, elem_size);
        emit_psq_dequant_element(builder, ctx_ptr, ea1, type_v, scale_f32, local.read_u8, local.read_u16)
    };
    fpr_store::<SYSTEM>(builder, ctx_ptr, fd, ps0);
    ps1_store::<SYSTEM>(builder, ctx_ptr, fd, ps1);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(tramp_block);
    builder.seal_block(tramp_block);
    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    let fast_f32_block = builder.create_block();
    let slow_f32_block = builder.create_block();
    builder
        .ins()
        .brif(host_ptr, fast_f32_block, &[host_addr.into()], slow_f32_block, &[]);
    builder.append_block_param(fast_f32_block, types::I64);

    builder.switch_to_block(fast_f32_block);
    builder.seal_block(fast_f32_block);
    let addr = builder.block_params(fast_f32_block)[0];
    let raw0 = builder.ins().load(types::I32, heap_flags(), addr, 0);
    let be0 = builder.ins().bswap(raw0);
    let f32_0 = builder.ins().bitcast(types::F32, MemFlagsData::new(), be0);
    let f64_0 = builder.ins().fpromote(types::F64, f32_0);
    fpr_store::<SYSTEM>(builder, ctx_ptr, fd, f64_0);
    let f64_1 = if w {
        builder.ins().f64const(1.0)
    } else {
        let raw1 = builder.ins().load(types::I32, heap_flags(), addr, 4);
        let be1 = builder.ins().bswap(raw1);
        let f32_1 = builder.ins().bitcast(types::F32, MemFlagsData::new(), be1);
        builder.ins().fpromote(types::F64, f32_1)
    };
    ps1_store::<SYSTEM>(builder, ctx_ptr, fd, f64_1);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_f32_block);
    builder.seal_block(slow_f32_block);
    let fd_v = builder.ins().iconst(types::I32, fd as i64);
    let w_v = builder.ins().iconst(types::I32, if w { 1 } else { 0 });
    let gqr_const = builder.ins().iconst(types::I32, gqr_idx as i64);
    builder
        .ins()
        .call(local.do_psq_load, &[ctx_ptr, fd_v, ea, w_v, gqr_const]);
    builder.ins().jump(merge, &[]);
}

pub(crate) fn emit_psq_store_quantized<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    fs: u8,
    w: bool,
    gqr_idx: u8,
    merge: Block,
    local: &mut BlockEmitContext,
) {
    let gqr_v = gqr_load::<SYSTEM>(builder, ctx_ptr, local, gqr_idx);
    let type_v = builder.ins().band_imm(gqr_v, 0x7);

    let q_bit = builder.ins().band_imm(type_v, 0x4);
    let is_quant = builder.ins().icmp_imm(IntCC::NotEqual, q_bit, 0);
    let inline_block = builder.create_block();
    let tramp_block = builder.create_block();
    builder.ins().brif(is_quant, inline_block, &[], tramp_block, &[]);

    builder.switch_to_block(inline_block);
    builder.seal_block(inline_block);
    let scale_idx_shifted = builder.ins().ushr_imm(gqr_v, 8);
    let scale_idx = builder.ins().band_imm(scale_idx_shifted, 0x3F);
    let scale_idx_64 = builder.ins().uextend(types::I64, scale_idx);
    let scale_offset = builder.ins().ishl_imm(scale_idx_64, 2);
    let table_addr = builder
        .ins()
        .iconst(types::I64, crate::gekko::jit::runtime::QUANT_TABLE.as_ptr() as i64);
    let scale_addr = builder.ins().iadd(table_addr, scale_offset);
    let scale_f32 = builder.ins().load(types::F32, vmctx_flags(), scale_addr, 0);

    let ps0 = fpr_load::<SYSTEM>(builder, ctx_ptr, fs);
    emit_psq_quant_element(
        builder,
        ctx_ptr,
        ea,
        ps0,
        type_v,
        scale_f32,
        local.write_u8,
        local.write_u16,
    );
    if !w {
        let ps1 = ps1_load::<SYSTEM>(builder, ctx_ptr, fs);
        let elem_size = emit_psq_elem_size(builder, type_v);
        let ea1 = builder.ins().iadd(ea, elem_size);
        emit_psq_quant_element(
            builder,
            ctx_ptr,
            ea1,
            ps1,
            type_v,
            scale_f32,
            local.write_u8,
            local.write_u16,
        );
    }
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(tramp_block);
    builder.seal_block(tramp_block);
    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    let fast_f32_block = builder.create_block();
    let slow_f32_block = builder.create_block();
    builder
        .ins()
        .brif(host_ptr, fast_f32_block, &[host_addr.into()], slow_f32_block, &[]);
    builder.append_block_param(fast_f32_block, types::I64);

    builder.switch_to_block(fast_f32_block);
    builder.seal_block(fast_f32_block);

    let addr = builder.block_params(fast_f32_block)[0];
    let ps0_f64 = fpr_load::<SYSTEM>(builder, ctx_ptr, fs);
    let ps0_f32 = builder.ins().fdemote(types::F32, ps0_f64);
    let ps0_bits = builder.ins().bitcast(types::I32, MemFlagsData::new(), ps0_f32);
    let ps0_be = builder.ins().bswap(ps0_bits);
    builder.ins().store(heap_flags(), ps0_be, addr, 0);

    if !w {
        let ps1_f64 = ps1_load::<SYSTEM>(builder, ctx_ptr, fs);
        let ps1_f32 = builder.ins().fdemote(types::F32, ps1_f64);
        let ps1_bits = builder.ins().bitcast(types::I32, MemFlagsData::new(), ps1_f32);
        let ps1_be = builder.ins().bswap(ps1_bits);
        builder.ins().store(heap_flags(), ps1_be, addr, 4);
    }

    let psq_size = if w { 4 } else { 8 };
    emit_smc_check::<SYSTEM>(builder, ctx_ptr, ea, psq_size, local.cause_smc_write);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_f32_block);
    builder.seal_block(slow_f32_block);

    let fs_v = builder.ins().iconst(types::I32, fs as i64);
    let w_v = builder.ins().iconst(types::I32, if w { 1 } else { 0 });
    let gqr_const = builder.ins().iconst(types::I32, gqr_idx as i64);
    builder
        .ins()
        .call(local.do_psq_store, &[ctx_ptr, fs_v, ea, w_v, gqr_const]);

    builder.ins().jump(merge, &[]);
}

pub(crate) fn emit_read_f32<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    slow_f32: FuncRef,
) -> Value {
    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();
    builder.append_block_param(merge, types::F64);

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let raw_u32 = builder.ins().load(types::I32, heap_flags(), addr, 0);
    let be_u32 = builder.ins().bswap(raw_u32);
    let f32v = builder.ins().bitcast(types::F32, MemFlagsData::new(), be_u32);
    let f64v = builder.ins().fpromote(types::F64, f32v);
    builder.ins().jump(merge, &[f64v.into()]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let call = builder.ins().call(slow_f32, &[ctx_ptr, ea]);
    let f64v_slow = builder.inst_results(call)[0];
    builder.ins().jump(merge, &[f64v_slow.into()]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
    builder.block_params(merge)[0]
}

pub(crate) fn emit_write_f32<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    f64v: Value,
    slow_f32: FuncRef,
    cause_smc_write: FuncRef,
) {
    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let f32v = builder.ins().fdemote(types::F32, f64v);
    let bits = builder.ins().bitcast(types::I32, MemFlagsData::new(), f32v);
    let swapped = builder.ins().bswap(bits);
    builder.ins().store(heap_flags(), swapped, addr, 0);
    emit_smc_check::<SYSTEM>(builder, ctx_ptr, ea, 4, cause_smc_write);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    builder.ins().call(slow_f32, &[ctx_ptr, ea, f64v]);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
}

fn le_vec_flags() -> MemFlagsData {
    MemFlagsData::new().with_endianness(cranelift_codegen::ir::Endianness::Little)
}

pub(crate) fn emit_read_ps_pair<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    slow_f32: FuncRef,
) -> Value {
    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();
    builder.append_block_param(merge, types::F64X2);

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let pair = emit_direct_read_ps_pair(builder, addr);
    builder.ins().jump(merge, &[pair.into()]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let c0 = builder.ins().call(slow_f32, &[ctx_ptr, ea]);
    let p0 = builder.inst_results(c0)[0];
    let ea1 = builder.ins().iadd_imm(ea, 4);
    let c1 = builder.ins().call(slow_f32, &[ctx_ptr, ea1]);
    let p1 = builder.inst_results(c1)[0];
    let slow_pair = build_pair(builder, p0, p1);
    builder.ins().jump(merge, &[slow_pair.into()]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
    builder.block_params(merge)[0]
}

pub(crate) fn emit_write_ps_pair<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    pair: Value,
    slow_f32: FuncRef,
    cause_smc_write: FuncRef,
) {
    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    emit_direct_write_ps_pair(builder, addr, pair);
    emit_smc_check::<SYSTEM>(builder, ctx_ptr, ea, 8, cause_smc_write);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let p0 = builder.ins().extractlane(pair, 0);
    builder.ins().call(slow_f32, &[ctx_ptr, ea, p0]);
    let p1 = builder.ins().extractlane(pair, 1);
    let ea1 = builder.ins().iadd_imm(ea, 4);
    builder.ins().call(slow_f32, &[ctx_ptr, ea1, p1]);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
}

pub(crate) fn emit_ps_arith<const OP: u32, const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> bool {
    use crate::gekko::jit::lut;

    match OP {
        lut::OP_PS_ADD => {
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let b = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let s = builder.ins().fadd(a, b);
            let r = pair_round_to_single(builder, s);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            true
        }
        lut::OP_PS_SUB => {
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let b = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let s = builder.ins().fsub(a, b);
            let r = pair_round_to_single(builder, s);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            true
        }
        lut::OP_PS_MUL => {
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let c = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let c25 = pair_round_frc(builder, c);
            let m = builder.ins().fmul(a, c25);
            let r = pair_round_to_single(builder, m);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            true
        }
        lut::OP_PS_DIV => {
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let b = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let q = builder.ins().fdiv(a, b);
            let r = pair_round_to_single(builder, q);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            true
        }
        lut::OP_PS_MADD => {
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let c = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let b = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let c25 = pair_round_frc(builder, c);
            let t = builder.ins().fma(a, c25, b);
            let r = pair_round_to_single(builder, t);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            true
        }
        lut::OP_PS_MSUB => {
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let c = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let b = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let c25 = pair_round_frc(builder, c);
            let nb = builder.ins().fneg(b);
            let t = builder.ins().fma(a, c25, nb);
            let r = pair_round_to_single(builder, t);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            true
        }
        lut::OP_PS_NMADD => {
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let c = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let b = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let c25 = pair_round_frc(builder, c);
            let t = builder.ins().fma(a, c25, b);
            let r = pair_round_to_single(builder, t);
            let n = pair_neg_unless_nan(builder, r);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), n);
            true
        }
        lut::OP_PS_NMSUB => {
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let c = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let b = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let c25 = pair_round_frc(builder, c);
            let nb = builder.ins().fneg(b);
            let t = builder.ins().fma(a, c25, nb);
            let r = pair_round_to_single(builder, t);
            let n = pair_neg_unless_nan(builder, r);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), n);
            true
        }
        lut::OP_PS_MULS0 => {
            let c = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let c25 = round_frc_value(builder, c);
            let cs = builder.ins().splat(types::F64X2, c25);
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let m = builder.ins().fmul(a, cs);
            let r = pair_round_to_single(builder, m);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            true
        }
        lut::OP_PS_MULS1 => {
            let c = ps1_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let c25 = round_frc_value(builder, c);
            let cs = builder.ins().splat(types::F64X2, c25);
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let m = builder.ins().fmul(a, cs);
            let r = pair_round_to_single(builder, m);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            true
        }
        lut::OP_PS_MADDS0 => {
            let c = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let c25 = round_frc_value(builder, c);
            let cs = builder.ins().splat(types::F64X2, c25);
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let b = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let t = builder.ins().fma(a, cs, b);
            let r = pair_round_to_single(builder, t);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            true
        }
        lut::OP_PS_MADDS1 => {
            let c = ps1_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let c25 = round_frc_value(builder, c);
            let cs = builder.ins().splat(types::F64X2, c25);
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let b = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let t = builder.ins().fma(a, cs, b);
            let r = pair_round_to_single(builder, t);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            true
        }
        lut::OP_PS_SUM0 => {
            let a0 = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let b1 = ps1_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let s = builder.ins().fadd(a0, b1);
            let r0 = round_to_single(builder, s);
            let c1 = ps1_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            ps_write::<SYSTEM>(builder, ctx_ptr, instr.rd(), r0, c1);
            true
        }
        lut::OP_PS_SUM1 => {
            let c0 = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let a0 = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let b1 = ps1_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let s = builder.ins().fadd(a0, b1);
            let r1 = round_to_single(builder, s);
            ps_write::<SYSTEM>(builder, ctx_ptr, instr.rd(), c0, r1);
            true
        }
        lut::OP_PS_SEL => {
            use cranelift_codegen::ir::condcodes::FloatCC;

            let zero = builder.ins().f64const(0.0);
            let zs = builder.ins().splat(types::F64X2, zero);
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let c = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let b = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let mask = builder.ins().fcmp(FloatCC::GreaterThanOrEqual, a, zs);
            let r = pair_bitselect(builder, mask, c, b);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            true
        }
        lut::OP_PS_RES => {
            let b = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let b32 = builder.ins().fvdemote(b);
            let one = builder.ins().f32const(1.0);
            let ones = builder.ins().splat(types::F32X4, one);
            let q = builder.ins().fdiv(ones, b32);
            let r = builder.ins().fvpromote_low(q);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            true
        }
        lut::OP_PS_RSQRTE => {
            let b = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let b32 = builder.ins().fvdemote(b);
            let s = builder.ins().sqrt(b32);
            let one = builder.ins().f32const(1.0);
            let ones = builder.ins().splat(types::F32X4, one);
            let q = builder.ins().fdiv(ones, s);
            let r = builder.ins().fvpromote_low(q);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            true
        }
        lut::OP_PS_MR => {
            let v = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), v);
            true
        }
        lut::OP_PS_NEG => {
            let v = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let n = builder.ins().fneg(v);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), n);
            true
        }
        lut::OP_PS_ABS => {
            let v = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let a = builder.ins().fabs(v);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), a);
            true
        }
        lut::OP_PS_NABS => {
            let v = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let a = builder.ins().fabs(v);
            let n = builder.ins().fneg(a);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), n);
            true
        }
        lut::OP_PS_MERGE00 => {
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let b0 = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let v = builder.ins().insertlane(a, b0, 1);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), v);
            true
        }
        lut::OP_PS_MERGE01 => {
            let a = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let b1 = ps1_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let v = builder.ins().insertlane(a, b1, 1);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), v);
            true
        }
        lut::OP_PS_MERGE10 => {
            let p0 = ps1_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let p1 = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            ps_write::<SYSTEM>(builder, ctx_ptr, instr.rd(), p0, p1);
            true
        }
        lut::OP_PS_MERGE11 => {
            let b = fpr_pair_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let a1 = ps1_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let v = builder.ins().insertlane(b, a1, 0);
            fpr_pair_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), v);
            true
        }
        lut::OP_PS_CMPU0 | lut::OP_PS_CMPO0 => {
            let a = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let b = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            emit_fp_compare::<SYSTEM>(builder, ctx_ptr, instr.crfd(), a, b);
            true
        }
        lut::OP_PS_CMPU1 | lut::OP_PS_CMPO1 => {
            let a = ps1_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let b = ps1_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            emit_fp_compare::<SYSTEM>(builder, ctx_ptr, instr.crfd(), a, b);
            true
        }
        _ => false,
    }
}

pub(crate) fn ps_write<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    rd: u8,
    ps0: Value,
    ps1: Value,
) {
    let v = build_pair(builder, ps0, ps1);
    fpr_pair_store::<SYSTEM>(builder, ctx_ptr, rd, v);
}

pub(crate) fn emit_fp_compare<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    crfd: u8,
    a: Value,
    b: Value,
) {
    use cranelift_codegen::ir::condcodes::FloatCC;

    let a_nan = builder.ins().fcmp(FloatCC::NotEqual, a, a);
    let b_nan = builder.ins().fcmp(FloatCC::NotEqual, b, b);
    let so_b = builder.ins().bor(a_nan, b_nan);
    let so = builder.ins().uextend(types::I32, so_b);

    let lt_b = builder.ins().fcmp(FloatCC::LessThan, a, b);
    let gt_b = builder.ins().fcmp(FloatCC::GreaterThan, a, b);
    let eq_b = builder.ins().fcmp(FloatCC::Equal, a, b);
    let lt = builder.ins().uextend(types::I32, lt_b);
    let gt = builder.ins().uextend(types::I32, gt_b);
    let eq = builder.ins().uextend(types::I32, eq_b);
    let nibble = build_cr_nibble(builder, lt, gt, eq, so);
    cr_set_field::<SYSTEM>(builder, ctx_ptr, crfd, nibble);
}

pub(crate) fn emit_fp_arith<const OP: u32, const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> bool {
    use crate::gekko::jit::lut;

    let single = matches!(
        OP,
        lut::OP_FADDSX
            | lut::OP_FSUBSX
            | lut::OP_FMULSX
            | lut::OP_FDIVSX
            | lut::OP_FMADDSX
            | lut::OP_FMSUBSX
            | lut::OP_FNMADDSX
            | lut::OP_FNMSUBSX
            | lut::OP_FSQRTSX
            | lut::OP_FRESX
    );

    match OP {
        lut::OP_FADDSX | lut::OP_FADDX => {
            emit_fp_binop::<SYSTEM>(
                builder,
                ctx_ptr,
                FpBinOp::Add,
                instr.rd(),
                instr.ra(),
                instr.rb(),
                single,
            );
            return true;
        }
        lut::OP_FSUBSX | lut::OP_FSUBX => {
            emit_fp_binop::<SYSTEM>(
                builder,
                ctx_ptr,
                FpBinOp::Sub,
                instr.rd(),
                instr.ra(),
                instr.rb(),
                single,
            );
            return true;
        }
        lut::OP_FMULSX => {
            let a = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let c = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let c25 = round_frc_value(builder, c);
            let m = builder.ins().fmul(a, c25);
            let r = round_to_single(builder, m);
            fpr_store_paired::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            return true;
        }
        lut::OP_FMULX => {
            emit_fp_binop::<SYSTEM>(
                builder,
                ctx_ptr,
                FpBinOp::Mul,
                instr.rd(),
                instr.ra(),
                instr.fc(),
                single,
            );
            return true;
        }
        lut::OP_FDIVSX | lut::OP_FDIVX => {
            emit_fp_binop::<SYSTEM>(
                builder,
                ctx_ptr,
                FpBinOp::Div,
                instr.rd(),
                instr.ra(),
                instr.rb(),
                single,
            );
            return true;
        }
        lut::OP_FMADDSX
        | lut::OP_FMADDX
        | lut::OP_FMSUBSX
        | lut::OP_FMSUBX
        | lut::OP_FNMADDSX
        | lut::OP_FNMADDX
        | lut::OP_FNMSUBSX
        | lut::OP_FNMSUBX => {
            let is_add = matches!(
                OP,
                lut::OP_FMADDSX | lut::OP_FMADDX | lut::OP_FNMADDSX | lut::OP_FNMADDX
            );
            let neg = matches!(
                OP,
                lut::OP_FNMADDSX | lut::OP_FNMADDX | lut::OP_FNMSUBSX | lut::OP_FNMSUBX
            );
            let a = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let c = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let b = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());

            let c_in = if single { round_frc_value(builder, c) } else { c };
            let b_in = if is_add { b } else { builder.ins().fneg(b) };
            let core = builder.ins().fma(a, c_in, b_in);

            let rounded = if single { round_to_single(builder, core) } else { core };
            let stored = if neg { neg_unless_nan(builder, rounded) } else { rounded };

            if single {
                fpr_store_paired::<SYSTEM>(builder, ctx_ptr, instr.rd(), stored);
            } else {
                fpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), stored);
            }
            return true;
        }
        lut::OP_FSQRTSX | lut::OP_FSQRTX => {
            let b = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let r = builder.ins().sqrt(b);
            let stored = if single { round_to_single(builder, r) } else { r };
            if single {
                fpr_store_paired::<SYSTEM>(builder, ctx_ptr, instr.rd(), stored);
            } else {
                fpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), stored);
            }
            return true;
        }
        lut::OP_FRESX => {
            let b = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let r = f32_reciprocal(builder, b);
            fpr_store_paired::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            return true;
        }
        lut::OP_FRSQRTEX => {
            let b = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let s = builder.ins().sqrt(b);
            let one = builder.ins().f64const(1.0);
            let r = builder.ins().fdiv(one, s);
            fpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            return true;
        }
        lut::OP_FSELX => {
            use cranelift_codegen::ir::condcodes::FloatCC;
            let zero = builder.ins().f64const(0.0);
            let a = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let c = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.fc());
            let b = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let cond = builder.ins().fcmp(FloatCC::GreaterThanOrEqual, a, zero);
            let r = builder.ins().select(cond, c, b);
            fpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            return true;
        }
        lut::OP_FMRX => {
            let v = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            fpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), v);
            return true;
        }
        lut::OP_FNEGX => {
            let v = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let r = builder.ins().fneg(v);
            fpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            return true;
        }
        lut::OP_FABSX => {
            let v = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let r = builder.ins().fabs(v);
            fpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            return true;
        }
        lut::OP_FNABSX => {
            let v = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let a = builder.ins().fabs(v);
            let r = builder.ins().fneg(a);
            fpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            return true;
        }
        lut::OP_FRSPX => {
            let v = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let r = round_to_single(builder, v);
            fpr_store_paired::<SYSTEM>(builder, ctx_ptr, instr.rd(), r);
            return true;
        }
        lut::OP_FCTIWZX | lut::OP_FCTIWX => {
            let v = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let i = builder.ins().fcvt_to_sint_sat(types::I32, v);
            let i64v = builder.ins().uextend(types::I64, i);
            let f = builder.ins().bitcast(types::F64, MemFlagsData::new(), i64v);
            fpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), f);
            return true;
        }
        lut::OP_MTFSB1X => {
            const FPSCR_ANY_X: u32 = 0x1FF8_0700;
            let bit_pos = 31 - (instr.crbd() as u32);
            let bit_value = 1u32 << bit_pos;
            let off = abi::fpscr_offset::<SYSTEM>() as i32;
            let cur = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, off);
            let mut new_v = builder.ins().bor_imm(cur, bit_value as i64);
            if (bit_value & FPSCR_ANY_X) != 0 {
                let was_set = builder.ins().band_imm(cur, bit_value as i64);
                let was_unset = builder.ins().icmp_imm(IntCC::Equal, was_set, 0);
                let was_unset_v = builder.ins().uextend(types::I32, was_unset);
                let fx_bit = builder.ins().ishl_imm(was_unset_v, 31);
                new_v = builder.ins().bor(new_v, fx_bit);
            }
            builder.ins().store(vmctx_flags(), new_v, ctx_ptr, off);
            emit_recompute_fpscr_summary::<SYSTEM>(builder, ctx_ptr);
            if instr.rc() {
                emit_update_cr1_from_fpscr::<SYSTEM>(builder, ctx_ptr);
            }
            true
        }
        lut::OP_MTFSB0X => {
            let bit_pos = 31 - (instr.crbd() as u32);
            let off = abi::fpscr_offset::<SYSTEM>() as i32;
            let cur = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, off);
            let new_v = builder.ins().band_imm(cur, !(1i64 << bit_pos));
            builder.ins().store(vmctx_flags(), new_v, ctx_ptr, off);
            emit_recompute_fpscr_summary::<SYSTEM>(builder, ctx_ptr);
            if instr.rc() {
                emit_update_cr1_from_fpscr::<SYSTEM>(builder, ctx_ptr);
            }
            true
        }
        lut::OP_MTFSFIX => {
            let dst_shift = 28 - 4 * (instr.crfd() as u32);
            let off = abi::fpscr_offset::<SYSTEM>() as i32;
            let cur = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, off);
            let cleared = builder.ins().band_imm(cur, !(0xFi64 << dst_shift));
            let new_v = builder.ins().bor_imm(cleared, (instr.imm() as i64) << dst_shift);
            builder.ins().store(vmctx_flags(), new_v, ctx_ptr, off);
            emit_recompute_fpscr_summary::<SYSTEM>(builder, ctx_ptr);
            if instr.rc() {
                emit_update_cr1_from_fpscr::<SYSTEM>(builder, ctx_ptr);
            }
            true
        }
        lut::OP_MFFSX => {
            let off = abi::fpscr_offset::<SYSTEM>() as i32;
            let cur = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, off);
            let cur_64 = builder.ins().uextend(types::I64, cur);
            let with_nan_hi = builder.ins().bor_imm(cur_64, 0xFFF8_0000_0000_0000u64 as i64);
            let f = builder.ins().bitcast(types::F64, MemFlagsData::new(), with_nan_hi);
            fpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), f);
            if instr.rc() {
                emit_update_cr1_from_fpscr::<SYSTEM>(builder, ctx_ptr);
            }
            true
        }
        lut::OP_MTFSFX => {
            let mut mask: u32 = 0;
            for i in 0..8 {
                if (instr.fm() as u32) & (1 << (7 - i)) != 0 {
                    mask |= 0xFu32 << (28 - 4 * i);
                }
            }
            let off = abi::fpscr_offset::<SYSTEM>() as i32;
            let frb_v = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            let frb_bits = builder.ins().bitcast(types::I64, MemFlagsData::new(), frb_v);
            let frb_low = builder.ins().ireduce(types::I32, frb_bits);
            let cur = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, off);
            let cleared = builder.ins().band_imm(cur, !(mask as i64) & 0xFFFF_FFFF);
            let from_src = builder.ins().band_imm(frb_low, mask as i64);
            let new_v = builder.ins().bor(cleared, from_src);
            builder.ins().store(vmctx_flags(), new_v, ctx_ptr, off);
            emit_recompute_fpscr_summary::<SYSTEM>(builder, ctx_ptr);
            if instr.rc() {
                emit_update_cr1_from_fpscr::<SYSTEM>(builder, ctx_ptr);
            }
            true
        }
        lut::OP_FCMPU | lut::OP_FCMPO => {
            let a = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
            let b = fpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
            emit_fp_compare::<SYSTEM>(builder, ctx_ptr, instr.crfd(), a, b);
            true
        }
        lut::OP_MCRFS => {
            let src_shift = (7 - (instr.crfs() as u32)) * 4;
            let dst_shift = 28 - 4 * (instr.crfd() as u32);

            const FX_ANY_X: u32 = 0x9FF8_0700;
            let fpscr_off = abi::fpscr_offset::<SYSTEM>() as i32;
            let fpscr = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, fpscr_off);

            let nibble = builder.ins().ushr_imm(fpscr, src_shift as i64);
            let nibble = builder.ins().band_imm(nibble, 0xF);

            let cr_off = abi::cr_offset::<SYSTEM>() as i32;
            let cr = builder.ins().load(types::I32, vmctx_flags(), ctx_ptr, cr_off);
            let cr_cleared = builder.ins().band_imm(cr, !(0xFi64 << dst_shift));
            let positioned = builder.ins().ishl_imm(nibble, dst_shift as i64);
            let cr_new = builder.ins().bor(cr_cleared, positioned);
            builder.ins().store(vmctx_flags(), cr_new, ctx_ptr, cr_off);

            let clear_mask = (0xFu32 << src_shift) & FX_ANY_X;
            if clear_mask != 0 {
                let fpscr_new = builder.ins().band_imm(fpscr, !(clear_mask as i64) & 0xFFFF_FFFF);
                builder.ins().store(vmctx_flags(), fpscr_new, ctx_ptr, fpscr_off);
            }
            emit_recompute_fpscr_summary::<SYSTEM>(builder, ctx_ptr);
            true
        }
        _ => false,
    }
}

#[derive(Clone, Copy)]
pub(crate) enum FpBinOp {
    Add,
    Sub,
    Mul,
    Div,
}

pub(crate) fn emit_fp_binop<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    op: FpBinOp,
    frd: u8,
    fra: u8,
    frb_or_c: u8,
    single: bool,
) {
    let a = fpr_load::<SYSTEM>(builder, ctx_ptr, fra);
    let b = fpr_load::<SYSTEM>(builder, ctx_ptr, frb_or_c);

    let res = match op {
        FpBinOp::Add => builder.ins().fadd(a, b),
        FpBinOp::Sub => builder.ins().fsub(a, b),
        FpBinOp::Mul => builder.ins().fmul(a, b),
        FpBinOp::Div => builder.ins().fdiv(a, b),
    };
    let stored = if single {
        let demoted = builder.ins().fdemote(types::F32, res);
        builder.ins().fpromote(types::F64, demoted)
    } else {
        res
    };
    if single {
        fpr_store_paired::<SYSTEM>(builder, ctx_ptr, frd, stored);
    } else {
        fpr_store::<SYSTEM>(builder, ctx_ptr, frd, stored);
    }
}

pub(crate) fn emit_load_xform<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    size: MemSize,
    slow: FuncRef,
    update: bool,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    emit_load_at_ea::<SYSTEM>(builder, ctx_ptr, ea, instr.rd(), instr.ra(), size, slow, update);
}

pub(crate) fn emit_load_at_ea<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    rd: u8,
    ra: u8,
    size: MemSize,
    slow: FuncRef,
    update: bool,
) {
    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();
    builder.append_block_param(merge, types::I32);

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let flags = heap_flags();
    let raw_val = builder.ins().load(size.ir_type(), flags, addr, 0);
    let val_swapped = match size {
        MemSize::U8 => raw_val,
        MemSize::U16 | MemSize::U32 => builder.ins().bswap(raw_val),
    };
    let val32 = match size {
        MemSize::U32 => val_swapped,
        MemSize::U16 | MemSize::U8 => builder.ins().uextend(types::I32, val_swapped),
    };
    builder.ins().jump(merge, &[val32.into()]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let call = builder.ins().call(slow, &[ctx_ptr, ea]);
    let val32_slow = builder.inst_results(call)[0];
    builder.ins().jump(merge, &[val32_slow.into()]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);
    let val = builder.block_params(merge)[0];
    gpr_store::<SYSTEM>(builder, ctx_ptr, rd, val);

    if update {
        gpr_store::<SYSTEM>(builder, ctx_ptr, ra, ea);
    }
}

pub(crate) fn emit_store<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    size: MemSize,
    slow: FuncRef,
    cause_smc_write: FuncRef,
    update: bool,
) {
    let ea = emit_d_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    emit_store_at_ea::<SYSTEM>(
        builder,
        ctx_ptr,
        ea,
        instr.rs(),
        instr.ra(),
        size,
        slow,
        cause_smc_write,
        update,
    );
}

pub(crate) fn emit_store_xform<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    size: MemSize,
    slow: FuncRef,
    cause_smc_write: FuncRef,
    update: bool,
) {
    let ea = emit_x_form_ea::<SYSTEM>(builder, ctx_ptr, instr);
    emit_store_at_ea::<SYSTEM>(
        builder,
        ctx_ptr,
        ea,
        instr.rs(),
        instr.ra(),
        size,
        slow,
        cause_smc_write,
        update,
    );
}

pub(crate) fn emit_store_at_ea<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    rs: u8,
    ra: u8,
    size: MemSize,
    slow: FuncRef,
    cause_smc_write: FuncRef,
    update: bool,
) {
    let val32 = gpr_load::<SYSTEM>(builder, ctx_ptr, rs);

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    builder.set_cold_block(slow_block);
    let merge = builder.create_block();

    let (host_ptr, page_off) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let host_addr = builder.ins().iadd(host_ptr, page_off);
    builder
        .ins()
        .brif(host_ptr, fast_block, &[host_addr.into()], slow_block, &[]);
    builder.append_block_param(fast_block, types::I64);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addr = builder.block_params(fast_block)[0];
    let flags = heap_flags();
    let truncated = match size {
        MemSize::U32 => val32,
        MemSize::U16 => builder.ins().ireduce(types::I16, val32),
        MemSize::U8 => builder.ins().ireduce(types::I8, val32),
    };
    let val_to_store = match size {
        MemSize::U8 => truncated,
        MemSize::U16 | MemSize::U32 => builder.ins().bswap(truncated),
    };
    builder.ins().store(flags, val_to_store, addr, 0);
    let size_bytes = match size {
        MemSize::U8 => 1,
        MemSize::U16 => 2,
        MemSize::U32 => 4,
    };
    emit_smc_check::<SYSTEM>(builder, ctx_ptr, ea, size_bytes, cause_smc_write);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    builder.ins().call(slow, &[ctx_ptr, ea, val32]);
    builder.ins().jump(merge, &[]);

    builder.switch_to_block(merge);
    builder.seal_block(merge);

    if update {
        gpr_store::<SYSTEM>(builder, ctx_ptr, ra, ea);
    }
}

pub(crate) fn emit_addi<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, instr: Instruction) {
    let base = gpr_load_or_zero::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let val = builder.ins().iadd_imm(base, instr.simm() as i64);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), val);
}

pub(crate) fn emit_addis<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, instr: Instruction) {
    let base = gpr_load_or_zero::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let val = builder.ins().iadd_imm(base, (instr.simm() as i64) << 16);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.rd(), val);
}

pub(crate) fn emit_ori<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, instr: Instruction) {
    let uimm = (instr.uimm() as u32) as i64;
    let src = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    let val = builder.ins().bor_imm(src, uimm);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), val);
}

#[derive(Clone, Copy)]
pub(crate) enum ImmLogical {
    And,
    Xor,
}

pub(crate) fn emit_imm_logical<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    op: ImmLogical,
    shift_high: bool,
) -> Value {
    let uimm = (instr.uimm() as u32) as i64;
    let imm = if shift_high { uimm << 16 } else { uimm };
    let src = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    let val = match op {
        ImmLogical::And => builder.ins().band_imm(src, imm),
        ImmLogical::Xor => builder.ins().bxor_imm(src, imm),
    };
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), val);
    val
}

pub(crate) fn emit_oris<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, instr: Instruction) {
    let uimm = (instr.uimm() as u32) as i64;
    let src = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    let val = builder.ins().bor_imm(src, uimm << 16);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), val);
}

pub(crate) fn emit_cmp_imm<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    signed: bool,
) {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let imm_val: i64 = if signed {
        instr.simm() as i64
    } else {
        (instr.uimm() as u32) as i64
    };
    let imm = builder.ins().iconst(types::I32, imm_val);
    emit_cmp_common::<SYSTEM>(builder, ctx_ptr, instr.crfd(), a, imm, signed);
}

pub(crate) fn emit_cmp_xform<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    signed: bool,
) {
    let a = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.ra());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());
    emit_cmp_common::<SYSTEM>(builder, ctx_ptr, instr.crfd(), a, b, signed);
}

pub(crate) fn emit_cmp_common<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    crfd: u8,
    a: Value,
    b: Value,
    signed: bool,
) {
    let (lt_cc, gt_cc) = if signed {
        (IntCC::SignedLessThan, IntCC::SignedGreaterThan)
    } else {
        (IntCC::UnsignedLessThan, IntCC::UnsignedGreaterThan)
    };
    let lt = cmp_to_i32(builder, lt_cc, a, b);
    let gt = cmp_to_i32(builder, gt_cc, a, b);
    let eq = cmp_to_i32(builder, IntCC::Equal, a, b);
    let so = xer_so_bit::<SYSTEM>(builder, ctx_ptr);
    let nibble = build_cr_nibble(builder, lt, gt, eq, so);
    cr_set_field::<SYSTEM>(builder, ctx_ptr, crfd, nibble);
}

pub(crate) fn emit_cntlzw<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
) -> Value {
    let s = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    let r = builder.ins().clz(s);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), r);
    r
}

pub(crate) fn emit_extend<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    from_ty: cranelift_codegen::ir::Type,
) -> Value {
    let s = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    let truncated = builder.ins().ireduce(from_ty, s);
    let extended = builder.ins().sextend(types::I32, truncated);
    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), extended);
    extended
}

#[derive(Clone, Copy)]
pub(crate) enum LogicalFullOp {
    Nor,
    Nand,
    Andc,
    Orc,
    Eqv,
}

pub(crate) fn emit_logical_xform_full<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    op: LogicalFullOp,
) -> Value {
    let s = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());

    let val = match op {
        LogicalFullOp::Nor => {
            let or = builder.ins().bor(s, b);
            builder.ins().bnot(or)
        }
        LogicalFullOp::Nand => {
            let and = builder.ins().band(s, b);
            builder.ins().bnot(and)
        }
        LogicalFullOp::Eqv => {
            let xor = builder.ins().bxor(s, b);
            builder.ins().bnot(xor)
        }
        LogicalFullOp::Andc => {
            let nb = builder.ins().bnot(b);
            builder.ins().band(s, nb)
        }
        LogicalFullOp::Orc => {
            let nb = builder.ins().bnot(b);
            builder.ins().bor(s, nb)
        }
    };

    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), val);
    val
}

pub(crate) fn emit_logical_xform<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    instr: Instruction,
    op: LogicalOp,
) -> Value {
    let s = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rs());
    let b = gpr_load::<SYSTEM>(builder, ctx_ptr, instr.rb());

    let val = match op {
        LogicalOp::And => builder.ins().band(s, b),
        LogicalOp::Or => builder.ins().bor(s, b),
        LogicalOp::Xor => builder.ins().bxor(s, b),
    };

    gpr_store::<SYSTEM>(builder, ctx_ptr, instr.ra(), val);
    val
}
