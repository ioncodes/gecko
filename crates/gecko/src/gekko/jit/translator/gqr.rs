use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{InstBuilder, SigRef, Value, types};
use cranelift_frontend::FunctionBuilder;

use crate::gekko::instruction::Instruction;
use crate::gekko::interpreter::psq::{gqr_ld_type, gqr_st_type};
use crate::gekko::jit::BlockEntry;
use crate::gekko::jit::block::BlockSpec;
use crate::system::SystemId;

use super::{BlockEmitContext, gqr_cache_store, gqr_load};

const MIN_QUANTIZED_ACCESSES: usize = 4;

pub(crate) struct GqrAssumptions {
    expected: [Option<u32>; 8],
}

impl GqrAssumptions {
    pub(crate) fn for_block(spec: &BlockSpec, gqrs: &[u32; 8]) -> Option<Self> {
        if cfg!(feature = "hooks") {
            return None;
        }

        let mut values = [None; 8];
        let mut count = 0;
        for &raw in &spec.instrs {
            let instr = Instruction(raw);
            let primary = instr.primary_opcode();
            if primary == 31 && instr.xo10() == 467 && matches!(instr.spr_swapped(), 912..=919) {
                return None;
            }

            let (index, store) = match primary {
                56 | 57 => (instr.psq_i(), false),
                60 | 61 => (instr.psq_i(), true),
                4 => match instr.xo6() {
                    6 | 38 => (instr.psq_ix(), false),
                    7 | 39 => (instr.psq_ix(), true),
                    _ => continue,
                },
                _ => continue,
            };
            let gqr = gqrs[index as usize];
            let qtype = if store { gqr_st_type(gqr) } else { gqr_ld_type(gqr) };
            if qtype >= 4 {
                values[index as usize] = Some(gqr);
                count += 1;
            }
        }

        (count >= MIN_QUANTIZED_ACCESSES).then_some(Self { expected: values })
    }

    pub(super) fn emit_guard<const SYSTEM: SystemId>(
        &self,
        builder: &mut FunctionBuilder,
        ctx_ptr: Value,
        host_return_pc: Value,
        block_sig_ref: SigRef,
        local: &mut BlockEmitContext,
        fallback_entry: BlockEntry,
    ) {
        let mut matches = builder.ins().iconst(types::I8, 1);
        for (index, expected) in self.expected.iter().enumerate() {
            let Some(expected) = *expected else { continue };
            let actual = gqr_load::<SYSTEM>(builder, ctx_ptr, local, index as u8);
            let same = builder.ins().icmp_imm(IntCC::Equal, actual, expected as i64);
            matches = builder.ins().band(matches, same);
            let constant = builder.ins().iconst(types::I32, expected as i64);
            gqr_cache_store(local, index as u8, constant);
        }

        let specialized = builder.create_block();
        let fallback = builder.create_block();
        builder.ins().brif(matches, specialized, &[], fallback, &[]);

        builder.switch_to_block(fallback);
        builder.seal_block(fallback);
        let target = builder.ins().iconst(types::I64, fallback_entry as i64);
        builder
            .ins()
            .return_call_indirect(block_sig_ref, target, &[ctx_ptr, host_return_pc]);

        builder.switch_to_block(specialized);
        builder.seal_block(specialized);
    }
}
