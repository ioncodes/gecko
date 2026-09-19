use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{InstBuilder, Value, types};
use cranelift_frontend::FunctionBuilder;

use crate::gekko::jit::abi;
use crate::mmio::FASTMEM_PAGE_BYTES;
use crate::mmio::constants::{LCACHE_BASE, LCACHE_SIZE};
use crate::system::SystemId;

use super::{BlockEmitContext, MemSize, emit_fastmem_load, emit_fastmem_lookup, emit_fastmem_store, vmctx_flags};

fn pointer<const SYSTEM: SystemId>(builder: &mut FunctionBuilder, ctx_ptr: Value, ea: Value, size: MemSize) -> Value {
    let bytes = size.bytes() as usize;

    let (base, offset) = emit_fastmem_lookup::<SYSTEM>(builder, ctx_ptr, ea);
    let top = builder.ins().ushr_imm(ea, 28);
    let cached = builder.ins().icmp_imm(IntCC::Equal, top, 8);
    let uncached = builder.ins().icmp_imm(IntCC::Equal, top, 12);
    let alias = builder.ins().bor(cached, uncached);
    let mapped = builder.ins().icmp_imm(IntCC::NotEqual, base, 0);
    let in_page = builder.ins().icmp_imm(
        IntCC::UnsignedLessThanOrEqual,
        offset,
        (FASTMEM_PAGE_BYTES - bytes) as i64,
    );
    let valid = builder.ins().band(mapped, alias);
    let valid = builder.ins().band(valid, in_page);
    let addr = builder.ins().iadd(base, offset);
    let zero = builder.ins().iconst(types::I64, 0);
    let ram = builder.ins().select(valid, addr, zero);

    let lc_offset = builder.ins().iadd_imm(ea, -(LCACHE_BASE as i64));
    let in_cache = builder
        .ins()
        .icmp_imm(IntCC::UnsignedLessThanOrEqual, lc_offset, (LCACHE_SIZE - bytes) as i64);
    let lc_base = builder.ins().load(
        types::I64,
        vmctx_flags(),
        ctx_ptr,
        abi::lcache_fastmem_ptr_offset::<SYSTEM>() as i32,
    );
    let lc_mapped = builder.ins().icmp_imm(IntCC::NotEqual, lc_base, 0);
    let lc_valid = builder.ins().band(in_cache, lc_mapped);
    let lc_offset = builder.ins().uextend(types::I64, lc_offset);
    let lc_addr = builder.ins().iadd(lc_base, lc_offset);
    builder.ins().select(lc_valid, lc_addr, ram)
}

pub(super) fn load<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    size: MemSize,
    local: &BlockEmitContext,
) -> Value {
    let slow = match size {
        MemSize::U8 => local.read_u8,
        MemSize::U16 => local.read_u16,
        MemSize::U32 => local.read_u32,
    };

    if cfg!(feature = "hooks") {
        let call = builder.ins().call(slow, &[ctx_ptr, ea]);
        return builder.inst_results(call)[0];
    }

    let ptr = self::pointer::<SYSTEM>(builder, ctx_ptr, ea, size);
    emit_fastmem_load(builder, ctx_ptr, ea, ptr, ptr, size, slow)
}

pub(super) fn store<const SYSTEM: SystemId>(
    builder: &mut FunctionBuilder,
    ctx_ptr: Value,
    ea: Value,
    value: Value,
    size: MemSize,
    local: &BlockEmitContext,
) {
    let slow = match size {
        MemSize::U8 => local.write_u8,
        MemSize::U16 => local.write_u16,
        MemSize::U32 => local.write_u32,
    };

    if cfg!(feature = "hooks") {
        builder.ins().call(slow, &[ctx_ptr, ea, value]);
        return;
    }

    let ptr = self::pointer::<SYSTEM>(builder, ctx_ptr, ea, size);
    emit_fastmem_store::<SYSTEM>(builder, ctx_ptr, ea, ptr, ptr, value, size, slow, local.cause_smc_write);
}
