pub mod abi;
pub mod block;
pub mod handlers;
pub mod idle;
pub mod insn;
pub mod runtime;
pub mod translator;

#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod lut {
    include!(concat!(env!("OUT_DIR"), "/gekko_jit_lut.rs"));
}

#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod lut_wii {
    include!(concat!(env!("OUT_DIR"), "/gekko_jit_lut_wii.rs"));
}

use cranelift_codegen::Context;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{AbiParam, InstBuilder, Signature, types};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, default_libcall_names};
use rustc_hash::FxHashMap;

use crate::system::{GC, System, SystemId, WII};

pub type BlockEntry = usize;

type TrampolineFn = unsafe extern "C" fn(*mut core::ffi::c_void, usize) -> u32;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BlockLookupSlot {
    pub pc: u32,
    pub _pad: u32,
    pub entry: usize,
}

pub const BLOCK_LOOKUP_TABLE_SIZE: usize = 131072;
pub const BLOCK_LOOKUP_TABLE_MASK: u32 = (BLOCK_LOOKUP_TABLE_SIZE as u32) - 1;

const DIV2I_SIZE: u32 = 312;
const DIV2I_CHECKSUM: u32 = 0xFB31_C123;

fn signature_checksum<const SYSTEM: SystemId>(sys: &System<SYSTEM>, start: u32, size: u32) -> u32 {
    let mut sum = 0u32;

    for offset in (0..size).step_by(4) {
        let opcode = sys.mmio.fetch_instruction(start.wrapping_add(offset));
        let primary = opcode >> 26;
        let op = opcode & 0xFC00_0000;
        let mut op2 = 0;
        let mut op3 = 0;

        match primary {
            4 => {
                op2 = opcode & 0x3F;

                if matches!(op2, 0 | 8 | 16 | 21 | 22) {
                    op3 = opcode & 0x7C0;
                }
            }
            7 | 8 | 10..=15 => op2 = opcode & 0x03FF_0000,
            19 | 31 | 63 => op2 = opcode & 0x7FF,
            59 => {
                op2 = opcode & 0x3F;

                if op2 < 16 {
                    op3 = opcode & 0x7C0;
                }
            }
            32..=55 => op2 = opcode & 0x03FF_0000,
            _ => {}
        }

        sum = sum.rotate_left(17) ^ (op | op2 | op3);
    }

    sum
}

fn is_div2i<const SYSTEM: SystemId>(sys: &System<SYSTEM>, pc: u32) -> bool {
    sys.mmio.fetch_instruction(pc) == 0x9421_FFF0
        && sys.mmio.fetch_instruction(pc.wrapping_add(4)) == 0x5469_0001
        && sys.mmio.fetch_instruction(pc.wrapping_add(8)) == 0x4182_000C
        && signature_checksum(sys, pc, DIV2I_SIZE) == DIV2I_CHECKSUM
}

#[derive(Clone, Copy)]
pub struct ExternFuncs {
    pub cause_invalid_opcode: FuncId,
    pub advance_to_deadline: FuncId,
    pub read_u8: FuncId,
    pub read_u16: FuncId,
    pub read_u32: FuncId,
    pub write_u8: FuncId,
    pub write_u16: FuncId,
    pub write_u32: FuncId,
    pub read_f32: FuncId,
    pub read_f64: FuncId,
    pub write_f32: FuncId,
    pub write_f64: FuncId,
    pub write_msr: FuncId,
    pub read_spr: FuncId,
    pub write_spr: FuncId,
    pub read_sr: FuncId,
    pub write_sr: FuncId,
    pub cause_trap_exception: FuncId,
    pub cause_syscall_interrupt: FuncId,
    pub do_rfi: FuncId,
    pub cause_fp_unavailable: FuncId,
    pub set_reservation: FuncId,
    pub try_clear_reservation: FuncId,
    pub do_lswi: FuncId,
    pub do_stswi: FuncId,
    pub do_lswx: FuncId,
    pub do_stswx: FuncId,
    pub do_psq_load: FuncId,
    pub do_psq_store: FuncId,
    pub read_timebase: FuncId,
    pub cause_icbi: FuncId,
    pub mark_memory_writeback: FuncId,
    pub cause_smc_write: FuncId,
    pub dcbz: FuncId,
}

pub struct JitEngine<const SYSTEM: SystemId> {
    module: JITModule,
    ctx: Context,
    builder_ctx: FunctionBuilderContext,
    cache: FxHashMap<u32, BlockEntry>,
    block_func_ids: FxHashMap<u32, FuncId>,
    hle_code_sizes: FxHashMap<u32, u32>,
    target_slots: std::cell::RefCell<FxHashMap<u32, usize>>,
    blocks_by_line: FxHashMap<u32, smallvec::SmallVec<[u32; 2]>>,
    pub(crate) drain_scratch: Vec<u32>,
    #[cfg(feature = "jit-stats")]
    hits: FxHashMap<u32, u64>,
    pub(crate) block_specs: FxHashMap<u32, block::BlockSpec>,
    #[cfg(feature = "jit-stats")]
    pub(crate) block_entry_counter_ptrs: FxHashMap<u32, usize>,
    block_sig: Signature,
    extern_funcs: ExternFuncs,
    trampoline_fn: TrampolineFn,
    block_lookup_table_addr: usize,
    block_seq: u64,
}

impl<const SYSTEM: SystemId> JitEngine<SYSTEM> {
    pub fn new() -> Self {
        let mut flag_builder = settings::builder();
        flag_builder.set("use_colocated_libcalls", "false").unwrap();
        flag_builder.set("is_pic", "false").unwrap();
        flag_builder.set("opt_level", "speed").unwrap();
        flag_builder.set("preserve_frame_pointers", "true").unwrap();
        flag_builder.set("enable_verifier", "false").unwrap();
        flag_builder.set("enable_probestack", "false").unwrap();
        flag_builder.set("unwind_info", "false").unwrap();
        flag_builder
            .set("enable_heap_access_spectre_mitigation", "false")
            .unwrap();
        flag_builder
            .set("enable_table_access_spectre_mitigation", "false")
            .unwrap();
        let isa_builder = cranelift_native::builder().expect("host ISA");
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .expect("ISA finish");

        let mut jit_builder = JITBuilder::with_isa(isa, default_libcall_names());

        struct SymTable {
            cause_invalid_opcode: (&'static str, *const u8),
            advance_to_deadline: (&'static str, *const u8),
            read_u8: (&'static str, *const u8),
            read_u16: (&'static str, *const u8),
            read_u32: (&'static str, *const u8),
            write_u8: (&'static str, *const u8),
            write_u16: (&'static str, *const u8),
            write_u32: (&'static str, *const u8),
            read_f32: (&'static str, *const u8),
            read_f64: (&'static str, *const u8),
            write_f32: (&'static str, *const u8),
            write_f64: (&'static str, *const u8),
            write_msr: (&'static str, *const u8),
            read_spr: (&'static str, *const u8),
            write_spr: (&'static str, *const u8),
            read_sr: (&'static str, *const u8),
            write_sr: (&'static str, *const u8),
            cause_trap_exception: (&'static str, *const u8),
            cause_syscall_interrupt: (&'static str, *const u8),
            do_rfi: (&'static str, *const u8),
            cause_fp_unavailable: (&'static str, *const u8),
            set_reservation: (&'static str, *const u8),
            try_clear_reservation: (&'static str, *const u8),
            do_lswi: (&'static str, *const u8),
            do_stswi: (&'static str, *const u8),
            do_lswx: (&'static str, *const u8),
            do_stswx: (&'static str, *const u8),
            do_psq_load: (&'static str, *const u8),
            do_psq_store: (&'static str, *const u8),
            read_timebase: (&'static str, *const u8),
            cause_icbi: (&'static str, *const u8),
            mark_memory_writeback: (&'static str, *const u8),
            cause_smc_write: (&'static str, *const u8),
            dcbz: (&'static str, *const u8),
        }
        let syms: SymTable = match SYSTEM {
            GC => SymTable {
                cause_invalid_opcode: (
                    "gecko_jit_cause_invalid_opcode_gc",
                    runtime::cause_invalid_opcode_gc as *const u8,
                ),
                advance_to_deadline: (
                    "gecko_jit_advance_to_deadline_gc",
                    runtime::advance_to_deadline_gc as *const u8,
                ),
                read_u8: ("gecko_jit_read_u8_gc", runtime::read_u8_gc as *const u8),
                read_u16: ("gecko_jit_read_u16_gc", runtime::read_u16_gc as *const u8),
                read_u32: ("gecko_jit_read_u32_gc", runtime::read_u32_gc as *const u8),
                write_u8: ("gecko_jit_write_u8_gc", runtime::write_u8_gc as *const u8),
                write_u16: ("gecko_jit_write_u16_gc", runtime::write_u16_gc as *const u8),
                write_u32: ("gecko_jit_write_u32_gc", runtime::write_u32_gc as *const u8),
                read_f32: ("gecko_jit_read_f32_gc", runtime::read_f32_gc as *const u8),
                read_f64: ("gecko_jit_read_f64_gc", runtime::read_f64_gc as *const u8),
                write_f32: ("gecko_jit_write_f32_gc", runtime::write_f32_gc as *const u8),
                write_f64: ("gecko_jit_write_f64_gc", runtime::write_f64_gc as *const u8),
                write_msr: ("gecko_jit_write_msr_gc", runtime::write_msr_gc as *const u8),
                read_spr: ("gecko_jit_read_spr_gc", runtime::read_spr_gc as *const u8),
                write_spr: ("gecko_jit_write_spr_gc", runtime::write_spr_gc as *const u8),
                read_sr: ("gecko_jit_read_sr_gc", runtime::read_sr_gc as *const u8),
                write_sr: ("gecko_jit_write_sr_gc", runtime::write_sr_gc as *const u8),
                cause_trap_exception: (
                    "gecko_jit_cause_trap_exception_gc",
                    runtime::cause_trap_exception_gc as *const u8,
                ),
                cause_syscall_interrupt: (
                    "gecko_jit_cause_syscall_interrupt_gc",
                    runtime::cause_syscall_interrupt_gc as *const u8,
                ),
                do_rfi: ("gecko_jit_do_rfi_gc", runtime::do_rfi_gc as *const u8),
                cause_fp_unavailable: (
                    "gecko_jit_cause_fp_unavailable_gc",
                    runtime::cause_fp_unavailable_gc as *const u8,
                ),
                set_reservation: ("gecko_jit_set_reservation_gc", runtime::set_reservation_gc as *const u8),
                try_clear_reservation: (
                    "gecko_jit_try_clear_reservation_gc",
                    runtime::try_clear_reservation_gc as *const u8,
                ),
                do_lswi: ("gecko_jit_do_lswi_gc", runtime::do_lswi_gc as *const u8),
                do_stswi: ("gecko_jit_do_stswi_gc", runtime::do_stswi_gc as *const u8),
                do_lswx: ("gecko_jit_do_lswx_gc", runtime::do_lswx_gc as *const u8),
                do_stswx: ("gecko_jit_do_stswx_gc", runtime::do_stswx_gc as *const u8),
                do_psq_load: ("gecko_jit_do_psq_load_gc", runtime::do_psq_load_gc as *const u8),
                do_psq_store: ("gecko_jit_do_psq_store_gc", runtime::do_psq_store_gc as *const u8),
                read_timebase: ("gecko_jit_read_timebase_gc", runtime::read_timebase_gc as *const u8),
                cause_icbi: ("gecko_jit_cause_icbi_gc", runtime::cause_icbi_gc as *const u8),
                mark_memory_writeback: (
                    "gecko_jit_mark_memory_writeback_gc",
                    runtime::mark_memory_writeback_gc as *const u8,
                ),
                cause_smc_write: ("gecko_jit_cause_smc_write_gc", runtime::cause_smc_write_gc as *const u8),
                dcbz: ("gecko_jit_dcbz_gc", runtime::dcbz_gc as *const u8),
            },
            WII => SymTable {
                cause_invalid_opcode: (
                    "gecko_jit_cause_invalid_opcode_wii",
                    runtime::cause_invalid_opcode_wii as *const u8,
                ),
                advance_to_deadline: (
                    "gecko_jit_advance_to_deadline_wii",
                    runtime::advance_to_deadline_wii as *const u8,
                ),
                read_u8: ("gecko_jit_read_u8_wii", runtime::read_u8_wii as *const u8),
                read_u16: ("gecko_jit_read_u16_wii", runtime::read_u16_wii as *const u8),
                read_u32: ("gecko_jit_read_u32_wii", runtime::read_u32_wii as *const u8),
                write_u8: ("gecko_jit_write_u8_wii", runtime::write_u8_wii as *const u8),
                write_u16: ("gecko_jit_write_u16_wii", runtime::write_u16_wii as *const u8),
                write_u32: ("gecko_jit_write_u32_wii", runtime::write_u32_wii as *const u8),
                read_f32: ("gecko_jit_read_f32_wii", runtime::read_f32_wii as *const u8),
                read_f64: ("gecko_jit_read_f64_wii", runtime::read_f64_wii as *const u8),
                write_f32: ("gecko_jit_write_f32_wii", runtime::write_f32_wii as *const u8),
                write_f64: ("gecko_jit_write_f64_wii", runtime::write_f64_wii as *const u8),
                write_msr: ("gecko_jit_write_msr_wii", runtime::write_msr_wii as *const u8),
                read_spr: ("gecko_jit_read_spr_wii", runtime::read_spr_wii as *const u8),
                write_spr: ("gecko_jit_write_spr_wii", runtime::write_spr_wii as *const u8),
                read_sr: ("gecko_jit_read_sr_wii", runtime::read_sr_wii as *const u8),
                write_sr: ("gecko_jit_write_sr_wii", runtime::write_sr_wii as *const u8),
                cause_trap_exception: (
                    "gecko_jit_cause_trap_exception_wii",
                    runtime::cause_trap_exception_wii as *const u8,
                ),
                cause_syscall_interrupt: (
                    "gecko_jit_cause_syscall_interrupt_wii",
                    runtime::cause_syscall_interrupt_wii as *const u8,
                ),
                do_rfi: ("gecko_jit_do_rfi_wii", runtime::do_rfi_wii as *const u8),
                cause_fp_unavailable: (
                    "gecko_jit_cause_fp_unavailable_wii",
                    runtime::cause_fp_unavailable_wii as *const u8,
                ),
                set_reservation: (
                    "gecko_jit_set_reservation_wii",
                    runtime::set_reservation_wii as *const u8,
                ),
                try_clear_reservation: (
                    "gecko_jit_try_clear_reservation_wii",
                    runtime::try_clear_reservation_wii as *const u8,
                ),
                do_lswi: ("gecko_jit_do_lswi_wii", runtime::do_lswi_wii as *const u8),
                do_stswi: ("gecko_jit_do_stswi_wii", runtime::do_stswi_wii as *const u8),
                do_lswx: ("gecko_jit_do_lswx_wii", runtime::do_lswx_wii as *const u8),
                do_stswx: ("gecko_jit_do_stswx_wii", runtime::do_stswx_wii as *const u8),
                do_psq_load: ("gecko_jit_do_psq_load_wii", runtime::do_psq_load_wii as *const u8),
                do_psq_store: ("gecko_jit_do_psq_store_wii", runtime::do_psq_store_wii as *const u8),
                read_timebase: ("gecko_jit_read_timebase_wii", runtime::read_timebase_wii as *const u8),
                cause_icbi: ("gecko_jit_cause_icbi_wii", runtime::cause_icbi_wii as *const u8),
                mark_memory_writeback: (
                    "gecko_jit_mark_memory_writeback_wii",
                    runtime::mark_memory_writeback_wii as *const u8,
                ),
                cause_smc_write: (
                    "gecko_jit_cause_smc_write_wii",
                    runtime::cause_smc_write_wii as *const u8,
                ),
                dcbz: ("gecko_jit_dcbz_wii", runtime::dcbz_wii as *const u8),
            },
            _ => unreachable!(),
        };

        for &(name, addr) in &[
            syms.cause_invalid_opcode,
            syms.advance_to_deadline,
            syms.read_u8,
            syms.read_u16,
            syms.read_u32,
            syms.write_u8,
            syms.write_u16,
            syms.write_u32,
            syms.read_f32,
            syms.read_f64,
            syms.write_f32,
            syms.write_f64,
            syms.write_msr,
            syms.read_spr,
            syms.write_spr,
            syms.read_sr,
            syms.write_sr,
            syms.cause_trap_exception,
            syms.cause_syscall_interrupt,
            syms.do_rfi,
            syms.cause_fp_unavailable,
            syms.set_reservation,
            syms.try_clear_reservation,
            syms.do_lswi,
            syms.do_stswi,
            syms.do_lswx,
            syms.do_stswx,
            syms.do_psq_load,
            syms.do_psq_store,
            syms.read_timebase,
            syms.cause_icbi,
            syms.mark_memory_writeback,
            syms.cause_smc_write,
            syms.dcbz,
        ] {
            jit_builder.symbol(name, addr);
        }

        let arena = crate::jit::arena::DenseArena::new(512 << 20).expect("reserve JIT code arena");
        jit_builder.memory_provider(Box::new(arena));

        let mut module = JITModule::new(jit_builder);

        let pointer_type = module.target_config().pointer_type();
        let call_conv = module.target_config().default_call_conv;

        let mut block_sig = Signature::new(CallConv::Tail);
        block_sig.params.push(AbiParam::new(pointer_type));
        block_sig.params.push(AbiParam::new(types::I32));
        block_sig.returns.push(AbiParam::new(types::I32));

        let mut fallback_sig = Signature::new(call_conv);
        fallback_sig.params.push(AbiParam::new(pointer_type));
        fallback_sig.params.push(AbiParam::new(types::I32));
        fallback_sig.params.push(AbiParam::new(types::I32));
        fallback_sig.returns.push(AbiParam::new(types::I32));

        let mut deadline_sig = Signature::new(call_conv);
        deadline_sig.params.push(AbiParam::new(pointer_type));

        let mut mem_read_sig = Signature::new(call_conv);
        mem_read_sig.params.push(AbiParam::new(pointer_type));
        mem_read_sig.params.push(AbiParam::new(types::I32));
        mem_read_sig.returns.push(AbiParam::new(types::I32));

        let mut mem_write_sig = Signature::new(call_conv);
        mem_write_sig.params.push(AbiParam::new(pointer_type));
        mem_write_sig.params.push(AbiParam::new(types::I32));
        mem_write_sig.params.push(AbiParam::new(types::I32));

        let mut fp_read_sig = Signature::new(call_conv);
        fp_read_sig.params.push(AbiParam::new(pointer_type));
        fp_read_sig.params.push(AbiParam::new(types::I32));
        fp_read_sig.returns.push(AbiParam::new(types::F64));

        let mut fp_write_sig = Signature::new(call_conv);
        fp_write_sig.params.push(AbiParam::new(pointer_type));
        fp_write_sig.params.push(AbiParam::new(types::I32));
        fp_write_sig.params.push(AbiParam::new(types::F64));

        let mut ctx_u32_sig = Signature::new(call_conv);
        ctx_u32_sig.params.push(AbiParam::new(pointer_type));
        ctx_u32_sig.params.push(AbiParam::new(types::I32));

        let extern_funcs = ExternFuncs {
            cause_invalid_opcode: module
                .declare_function(syms.cause_invalid_opcode.0, Linkage::Import, &fallback_sig)
                .expect("declare cause_invalid_opcode"),
            advance_to_deadline: module
                .declare_function(syms.advance_to_deadline.0, Linkage::Import, &deadline_sig)
                .expect("declare advance_to_deadline"),
            read_u8: module
                .declare_function(syms.read_u8.0, Linkage::Import, &mem_read_sig)
                .expect("declare read_u8"),
            read_u16: module
                .declare_function(syms.read_u16.0, Linkage::Import, &mem_read_sig)
                .expect("declare read_u16"),
            read_u32: module
                .declare_function(syms.read_u32.0, Linkage::Import, &mem_read_sig)
                .expect("declare read_u32"),
            write_u8: module
                .declare_function(syms.write_u8.0, Linkage::Import, &mem_write_sig)
                .expect("declare write_u8"),
            write_u16: module
                .declare_function(syms.write_u16.0, Linkage::Import, &mem_write_sig)
                .expect("declare write_u16"),
            write_u32: module
                .declare_function(syms.write_u32.0, Linkage::Import, &mem_write_sig)
                .expect("declare write_u32"),
            read_f32: module
                .declare_function(syms.read_f32.0, Linkage::Import, &fp_read_sig)
                .expect("declare read_f32"),
            read_f64: module
                .declare_function(syms.read_f64.0, Linkage::Import, &fp_read_sig)
                .expect("declare read_f64"),
            write_f32: module
                .declare_function(syms.write_f32.0, Linkage::Import, &fp_write_sig)
                .expect("declare write_f32"),
            write_f64: module
                .declare_function(syms.write_f64.0, Linkage::Import, &fp_write_sig)
                .expect("declare write_f64"),
            write_msr: module
                .declare_function(syms.write_msr.0, Linkage::Import, &ctx_u32_sig)
                .expect("declare write_msr"),
            read_spr: module
                .declare_function(syms.read_spr.0, Linkage::Import, &mem_read_sig)
                .expect("declare read_spr"),
            write_spr: module
                .declare_function(syms.write_spr.0, Linkage::Import, &mem_write_sig)
                .expect("declare write_spr"),
            read_sr: module
                .declare_function(syms.read_sr.0, Linkage::Import, &mem_read_sig)
                .expect("declare read_sr"),
            write_sr: module
                .declare_function(syms.write_sr.0, Linkage::Import, &mem_write_sig)
                .expect("declare write_sr"),
            cause_trap_exception: module
                .declare_function(syms.cause_trap_exception.0, Linkage::Import, &deadline_sig)
                .expect("declare cause_trap_exception"),
            cause_syscall_interrupt: module
                .declare_function(syms.cause_syscall_interrupt.0, Linkage::Import, &deadline_sig)
                .expect("declare cause_syscall_interrupt"),
            do_rfi: module
                .declare_function(syms.do_rfi.0, Linkage::Import, &deadline_sig)
                .expect("declare do_rfi"),
            cause_fp_unavailable: module
                .declare_function(syms.cause_fp_unavailable.0, Linkage::Import, &ctx_u32_sig)
                .expect("declare cause_fp_unavailable"),
            set_reservation: module
                .declare_function(syms.set_reservation.0, Linkage::Import, &ctx_u32_sig)
                .expect("declare set_reservation"),
            try_clear_reservation: module
                .declare_function(syms.try_clear_reservation.0, Linkage::Import, &mem_read_sig)
                .expect("declare try_clear_reservation"),
            do_lswi: module
                .declare_function(syms.do_lswi.0, Linkage::Import, &{
                    let mut sig = Signature::new(call_conv);
                    sig.params.push(AbiParam::new(pointer_type));
                    sig.params.push(AbiParam::new(types::I32));
                    sig.params.push(AbiParam::new(types::I32));
                    sig.params.push(AbiParam::new(types::I32));
                    sig
                })
                .expect("declare do_lswi"),
            do_stswi: module
                .declare_function(syms.do_stswi.0, Linkage::Import, &{
                    let mut sig = Signature::new(call_conv);
                    sig.params.push(AbiParam::new(pointer_type));
                    sig.params.push(AbiParam::new(types::I32));
                    sig.params.push(AbiParam::new(types::I32));
                    sig.params.push(AbiParam::new(types::I32));
                    sig
                })
                .expect("declare do_stswi"),
            do_lswx: module
                .declare_function(syms.do_lswx.0, Linkage::Import, &mem_write_sig)
                .expect("declare do_lswx"),
            do_stswx: module
                .declare_function(syms.do_stswx.0, Linkage::Import, &mem_write_sig)
                .expect("declare do_stswx"),
            do_psq_load: module
                .declare_function(syms.do_psq_load.0, Linkage::Import, &{
                    let mut sig = Signature::new(call_conv);
                    sig.params.push(AbiParam::new(pointer_type));
                    sig.params.push(AbiParam::new(types::I32));
                    sig.params.push(AbiParam::new(types::I32));
                    sig.params.push(AbiParam::new(types::I32));
                    sig.params.push(AbiParam::new(types::I32));
                    sig
                })
                .expect("declare do_psq_load"),
            do_psq_store: module
                .declare_function(syms.do_psq_store.0, Linkage::Import, &{
                    let mut sig = Signature::new(call_conv);
                    sig.params.push(AbiParam::new(pointer_type));
                    sig.params.push(AbiParam::new(types::I32));
                    sig.params.push(AbiParam::new(types::I32));
                    sig.params.push(AbiParam::new(types::I32));
                    sig.params.push(AbiParam::new(types::I32));
                    sig
                })
                .expect("declare do_psq_store"),
            read_timebase: module
                .declare_function(syms.read_timebase.0, Linkage::Import, &mem_read_sig)
                .expect("declare read_timebase"),
            cause_icbi: module
                .declare_function(syms.cause_icbi.0, Linkage::Import, &ctx_u32_sig)
                .expect("declare cause_icbi"),
            mark_memory_writeback: module
                .declare_function(syms.mark_memory_writeback.0, Linkage::Import, &ctx_u32_sig)
                .expect("declare mark_memory_writeback"),
            dcbz: module
                .declare_function(syms.dcbz.0, Linkage::Import, &ctx_u32_sig)
                .expect("declare dcbz"),
            cause_smc_write: module
                .declare_function(syms.cause_smc_write.0, Linkage::Import, &mem_write_sig)
                .expect("declare cause_smc_write"),
        };

        let trampoline_fn = build_trampoline(&mut module, call_conv, pointer_type, &block_sig);

        let block_lookup_table = vec![
            BlockLookupSlot {
                pc: 0,
                _pad: 0,
                entry: 0
            };
            BLOCK_LOOKUP_TABLE_SIZE
        ]
        .into_boxed_slice();
        let block_lookup_table_addr = Box::leak(block_lookup_table).as_mut_ptr() as usize;

        Self {
            module,
            ctx: Context::new(),
            builder_ctx: FunctionBuilderContext::new(),
            cache: FxHashMap::default(),
            block_func_ids: FxHashMap::default(),
            hle_code_sizes: FxHashMap::default(),
            target_slots: std::cell::RefCell::new(FxHashMap::default()),
            blocks_by_line: FxHashMap::default(),
            drain_scratch: Vec::new(),
            #[cfg(feature = "jit-stats")]
            hits: FxHashMap::default(),
            block_specs: FxHashMap::default(),
            #[cfg(feature = "jit-stats")]
            block_entry_counter_ptrs: FxHashMap::default(),
            block_sig,
            extern_funcs,
            trampoline_fn,
            block_lookup_table_addr,
            block_seq: 0,
        }
    }

    #[inline]
    fn target_slot_addr(&self, target_pc: u32) -> usize {
        *self.target_slots.borrow_mut().entry(target_pc).or_insert_with(|| {
            let slot: &'static mut usize = Box::leak(Box::new(0usize));
            slot as *mut usize as usize
        })
    }

    pub fn block_lookup_table_addr(&self) -> usize {
        self.block_lookup_table_addr
    }

    fn register_in_lookup_table(&self, pc: u32, entry: usize) {
        let idx = ((pc >> 2) & BLOCK_LOOKUP_TABLE_MASK) as usize;
        unsafe {
            let table = self.block_lookup_table_addr as *mut BlockLookupSlot;
            (*table.add(idx)).pc = pc;
            (*table.add(idx)).entry = entry;
        }
    }

    pub fn cached_blocks(&self) -> Vec<crate::jit::cache::CachedBlockPpc> {
        self.cache
            .keys()
            .filter_map(|&pc| {
                let spec = self.block_specs.get(&pc)?;
                Some(crate::jit::cache::CachedBlockPpc {
                    pc,
                    instr_count: spec.instrs.len() as u16,
                    hash: crate::jit::cache::hash_words(spec.instrs.iter().copied()),
                })
            })
            .collect()
    }

    pub fn precompile_blocks(
        &mut self,
        sys: &mut System<SYSTEM>,
        blocks: &[crate::jit::cache::CachedBlockPpc],
    ) -> (usize, usize) {
        let mut compiled = 0usize;
        let mut skipped = 0usize;

        for b in blocks {
            if self.cache.contains_key(&b.pc) {
                continue;
            }

            let count = b.instr_count as u32;
            if count == 0 {
                skipped += 1;
                continue;
            }

            let mut buf = Vec::with_capacity(count as usize);
            for i in 0..count {
                let instr_pc = b.pc.wrapping_add(i * 4);
                buf.push(sys.mmio.fetch_instruction(instr_pc));
            }

            let actual = crate::jit::cache::hash_words(buf.into_iter());
            if actual != b.hash {
                skipped += 1;
                continue;
            }

            tracing::info!(
                pc = format!("{:08X}", b.pc),
                count = b.instr_count,
                "compiling PPC JIT block from cache"
            );
            self.lookup_or_compile(sys, b.pc);

            compiled += 1;
        }
        (compiled, skipped)
    }

    pub fn lookup_or_compile(&mut self, sys: &mut System<SYSTEM>, pc: u32) -> BlockEntry {
        unsafe {
            let table = self.block_lookup_table_addr as *const BlockLookupSlot;
            let idx = ((pc >> 2) & BLOCK_LOOKUP_TABLE_MASK) as usize;
            let slot = &*table.add(idx);
            if slot.pc == pc && slot.entry != 0 {
                #[cfg(feature = "jit-stats")]
                {
                    *self.hits.entry(pc).or_insert(0) += 1;
                }
                return slot.entry;
            }
        }

        if let Some(&entry) = self.cache.get(&pc) {
            #[cfg(feature = "jit-stats")]
            {
                *self.hits.entry(pc).or_insert(0) += 1;
            }
            return entry;
        }

        let hle_div2i = is_div2i(sys, pc);
        let spec = block::discover::<SYSTEM>(sys, pc);
        let gprs_snapshot = sys.gekko.gprs;

        let entry = if hle_div2i {
            self.hle_code_sizes.insert(pc, DIV2I_SIZE);
            self.compile_div2i(pc)
        } else {
            self.compile(&spec, &gprs_snapshot)
        };

        self.cache.insert(pc, entry);
        self.block_specs.insert(pc, spec);
        self.register_block(&mut sys.mmio, pc);

        entry
    }

    fn register_block(&mut self, mmio: &mut crate::mmio::Mmio<SYSTEM>, pc: u32) {
        let Some(spec) = self.block_specs.get(&pc) else { return };

        let tracked_pcs = if let Some(&size) = self.hle_code_sizes.get(&pc) {
            std::borrow::Cow::Owned(
                (0..size)
                    .step_by(4)
                    .map(|offset| pc.wrapping_add(offset))
                    .collect::<Vec<_>>(),
            )
        } else {
            std::borrow::Cow::Borrowed(spec.pcs.as_slice())
        };

        let mut last = u32::MAX;

        for &vpc in tracked_pcs.iter() {
            let line = crate::mmio::virt_to_phys(vpc) & crate::mmio::CODE_LINE_MASK;
            if line == last {
                continue;
            }

            last = line;
            self.blocks_by_line.entry(line).or_default().push(pc);
            mmio.mark_code(line, crate::mmio::CODE_LINE_BYTES);
        }
    }

    fn unregister_block(&mut self, mmio: &mut crate::mmio::Mmio<SYSTEM>, pc: u32) {
        let Some(spec) = self.block_specs.remove(&pc) else {
            return;
        };

        let tracked_pcs = if let Some(size) = self.hle_code_sizes.remove(&pc) {
            std::borrow::Cow::Owned(
                (0..size)
                    .step_by(4)
                    .map(|offset| pc.wrapping_add(offset))
                    .collect::<Vec<_>>(),
            )
        } else {
            std::borrow::Cow::Borrowed(spec.pcs.as_slice())
        };

        self.cache.remove(&pc);
        self.block_func_ids.remove(&pc);

        if let Some(&slot) = self.target_slots.borrow().get(&pc) {
            unsafe {
                *(slot as *mut usize) = 0;
            }
        }

        let idx = ((pc >> 2) & BLOCK_LOOKUP_TABLE_MASK) as usize;
        unsafe {
            let table = self.block_lookup_table_addr as *mut BlockLookupSlot;
            if (*table.add(idx)).pc == pc {
                (*table.add(idx)).entry = 0;
            }
        }

        let mut last = u32::MAX;

        for &vpc in tracked_pcs.iter() {
            let line = crate::mmio::virt_to_phys(vpc) & crate::mmio::CODE_LINE_MASK;
            if line == last {
                continue;
            }

            last = line;
            mmio.unmark_code(line, crate::mmio::CODE_LINE_BYTES);
            if let Some(v) = self.blocks_by_line.get_mut(&line) {
                v.retain(|p| *p != pc);
                if v.is_empty() {
                    self.blocks_by_line.remove(&line);
                }
            }
        }
    }

    pub fn invalidate_line(&mut self, mmio: &mut crate::mmio::Mmio<SYSTEM>, line: u32) -> bool {
        let Some(pcs) = self.blocks_by_line.remove(&line) else {
            return false;
        };

        for pc in pcs.into_iter() {
            self.unregister_block(mmio, pc);
        }

        true
    }

    #[cfg(feature = "jit-stats")]
    fn block_entry_counter_slot(&mut self, pc: u32) -> usize {
        if let Some(&addr) = self.block_entry_counter_ptrs.get(&pc) {
            return addr;
        }

        let slot: &'static mut u64 = Box::leak(Box::new(0u64));
        let addr = slot as *mut u64 as usize;
        self.block_entry_counter_ptrs.insert(pc, addr);
        addr
    }

    fn func_id_for(&mut self, pc: u32) -> FuncId {
        if let Some(&id) = self.block_func_ids.get(&pc) {
            return id;
        }

        self.block_seq = self.block_seq.wrapping_add(1);
        let name = format!("gecko_block_{:08x}_{}", pc, self.block_seq);
        let id = self
            .module
            .declare_function(&name, Linkage::Local, &self.block_sig)
            .expect("declare block");
        self.block_func_ids.insert(pc, id);

        id
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure(label = "ppc_jit_run_block"))]
    pub fn run_block(&mut self, sys: &mut System<SYSTEM>) {
        let pc = sys.gekko.pc;
        let entry = self.lookup_or_compile(sys, pc);
        let ctx_ptr = sys as *mut System<SYSTEM> as *mut core::ffi::c_void;
        let next_pc = unsafe { (self.trampoline_fn)(ctx_ptr, entry) };
        sys.gekko.pc = next_pc;
    }

    #[cfg(feature = "jit-stats")]
    pub fn dump_hot_blocks_csv(&self, top_k: usize, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;

        let gprs_dummy = [0u32; 32];
        let mut entries: Vec<(u32, u64, u64, usize, String, String)> = self
            .block_entry_counter_ptrs
            .iter()
            .map(|(&pc, &addr)| {
                let executions = unsafe { *(addr as *const u64) };
                let dispatch_hits = self.hits.get(&pc).copied().unwrap_or(0);
                let (len, term, idle) = self
                    .block_specs
                    .get(&pc)
                    .map(|s| {
                        (
                            s.instrs.len(),
                            format!("{:?}", s.terminator),
                            format!("{:?}", idle::classify::<SYSTEM>(s, &gprs_dummy)),
                        )
                    })
                    .unwrap_or((0, "?".to_string(), "?".to_string()));
                (pc, executions, dispatch_hits, len, term, idle)
            })
            .collect();

        entries.sort_by(|a, b| {
            let ca = a.1.saturating_mul(a.3 as u64);
            let cb = b.1.saturating_mul(b.3 as u64);
            cb.cmp(&ca).then(b.1.cmp(&a.1))
        });

        crate::profile::write_file_atomic(path, |file| {
            writeln!(
                file,
                "rank,start_pc,executions,dispatch_hits,instr_count,cycles_estimated,terminator,idle_class"
            )?;

            for (rank, (pc, exec, hits, len, term, idle)) in entries.iter().take(top_k).enumerate() {
                let cycles = exec.saturating_mul(*len as u64);
                writeln!(
                    file,
                    "{},{:#010X},{},{},{},{},{},{}",
                    rank, pc, exec, hits, len, cycles, term, idle
                )?;
            }

            Ok(())
        })?;

        let disasm_path = path.with_file_name(format!(
            "{}-disasm.txt",
            path.file_stem().and_then(|s| s.to_str()).unwrap_or("ppc-heatmap")
        ));

        crate::profile::write_file_atomic(&disasm_path, |dfile| {
            writeln!(
                dfile,
                "# top {} hot PPC blocks (instructions)",
                top_k.min(entries.len())
            )?;

            for (rank, (pc, exec, hits, len, term, idle)) in entries.iter().take(top_k).enumerate() {
                writeln!(
                    dfile,
                    "\n#{} pc={:#010X} executions={} dispatch_hits={} len={} term={} idle={}",
                    rank, pc, exec, hits, len, term, idle
                )?;

                if let Some(spec) = self.block_specs.get(pc) {
                    for (i, &raw) in spec.instrs.iter().enumerate() {
                        let ipc = spec.start_pc.wrapping_add((i as u32) * 4);
                        let text = decode_gekko_instr(raw);
                        writeln!(dfile, "  {:08X}  {:08X}  {}", ipc, raw, text)?;
                    }
                }
            }

            Ok(())
        })
    }

    #[cfg(feature = "jit-stats")]
    pub fn dump_idle_candidates(&self, sys: &System<SYSTEM>, top_k: usize) {
        const SHAPE_BODY_CAP: usize = 12;

        let mut entries: Vec<(u32, u64, block::BlockSpec)> = self
            .hits
            .iter()
            .filter_map(|(&pc, &hits)| {
                let spec = block::discover::<SYSTEM>(sys, pc);
                if !looks_like_polling_shape(&spec, SHAPE_BODY_CAP) {
                    return None;
                }

                let class = idle::classify::<SYSTEM>(&spec, &sys.gekko.gprs);
                if class != idle::IdleClass::None {
                    return None;
                }

                Some((pc, hits, spec))
            })
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));

        if entries.is_empty() {
            tracing::info!("no idle-skip candidates recorded");
            return;
        }

        let total: u64 = entries.iter().map(|e| e.1).sum();
        tracing::info!(
            "idle-skip candidates (top {}, total hits = {})",
            top_k.min(entries.len()),
            total
        );
        for (rank, (pc, hits, spec)) in entries.iter().take(top_k).enumerate() {
            let pct = (*hits as f64) * 100.0 / (total as f64);
            tracing::info!(
                "  #{rank}  pc={:08X}  hits={hits}  ({:.1}%)  len={}  term={:?}",
                pc,
                pct,
                spec.instrs.len(),
                spec.terminator
            );
        }
    }

    #[cfg(feature = "jit-stats")]
    pub fn dump_hot_blocks(&self, sys: &System<SYSTEM>, top_k: usize) {
        let mut entries: Vec<(u32, u64)> = self.hits.iter().map(|(&pc, &n)| (pc, n)).collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));

        if entries.is_empty() {
            tracing::info!("no dispatcher entry hits recorded");
            return;
        }

        let total: u64 = entries.iter().map(|e| e.1).sum();
        tracing::info!(
            "JIT hot blocks (top {}, total hits = {}) ===",
            top_k.min(entries.len()),
            total
        );

        for (rank, (pc, hits)) in entries.iter().take(top_k).enumerate() {
            let spec = block::discover::<SYSTEM>(sys, *pc);
            let idle_class = idle::classify::<SYSTEM>(&spec, &sys.gekko.gprs);
            let pct = (*hits as f64) * 100.0 / (total as f64);
            tracing::info!(
                "  #{rank}  pc={:08X}  hits={hits}  ({:.1}%)  len={}  idle={:?}  term={:?}",
                pc,
                pct,
                spec.instrs.len(),
                idle_class,
                spec.terminator,
            );
        }
    }

    pub fn flush(&mut self) {
        self.cache.clear();
        self.block_func_ids.clear();
        self.hle_code_sizes.clear();
        self.block_specs.clear();
        self.blocks_by_line.clear();

        for &slot_addr in self.target_slots.borrow().values() {
            unsafe {
                *(slot_addr as *mut usize) = 0;
            }
        }

        unsafe {
            let table = self.block_lookup_table_addr as *mut BlockLookupSlot;
            for i in 0..BLOCK_LOOKUP_TABLE_SIZE {
                (*table.add(i)).pc = 0;
                (*table.add(i)).entry = 0;
            }
        }
    }

    pub fn flush_with_refcount(&mut self, mmio: &mut crate::mmio::Mmio<SYSTEM>) {
        self.flush();
        mmio.clear_code_refcount();
        mmio.pending_icbi.clear();
        mmio.jit_dirty = 0;
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure(label = "ppc_jit_compile"))]
    fn compile_div2i(&mut self, pc: u32) -> BlockEntry {
        let func_id = self.func_id_for(pc);

        #[cfg(feature = "jit-stats")]
        let entry_counter_addr = self.block_entry_counter_slot(pc);

        self.ctx.clear();
        self.ctx.func.signature = self.block_sig.clone();

        let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_ctx);
        translator::register_alias_regions(&mut builder);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);
        let ctx_ptr = builder.block_params(entry)[0];

        #[cfg(feature = "jit-stats")]
        {
            let slot = builder.ins().iconst(types::I64, entry_counter_addr as i64);
            let count = builder.ins().load(types::I64, translator::vmctx_flags(), slot, 0);
            let next = builder.ins().iadd_imm(count, 1);
            builder.ins().store(translator::vmctx_flags(), next, slot, 0);
        }

        let load_gpr = |builder: &mut FunctionBuilder, index: usize| {
            builder.ins().load(
                types::I32,
                translator::vmctx_flags(),
                ctx_ptr,
                (abi::gpr_base_offset::<SYSTEM>() + index * 4) as i32,
            )
        };

        let hi_lhs = load_gpr(&mut builder, 3);
        let lo_lhs = load_gpr(&mut builder, 4);
        let hi_rhs = load_gpr(&mut builder, 5);
        let lo_rhs = load_gpr(&mut builder, 6);

        let hi_lhs = builder.ins().uextend(types::I64, hi_lhs);
        let lo_lhs = builder.ins().uextend(types::I64, lo_lhs);
        let hi_rhs = builder.ins().uextend(types::I64, hi_rhs);
        let lo_rhs = builder.ins().uextend(types::I64, lo_rhs);
        let lhs_hi = builder.ins().ishl_imm(hi_lhs, 32);
        let rhs_hi = builder.ins().ishl_imm(hi_rhs, 32);
        let lhs = builder.ins().bor(lhs_hi, lo_lhs);
        let rhs = builder.ins().bor(rhs_hi, lo_rhs);

        let zero_block = builder.create_block();
        let nonzero_block = builder.create_block();
        let overflow_block = builder.create_block();
        let divide_block = builder.create_block();
        let merge_block = builder.create_block();
        builder.append_block_param(merge_block, types::I64);

        let rhs_zero = builder.ins().icmp_imm(IntCC::Equal, rhs, 0);
        builder.ins().brif(rhs_zero, zero_block, &[], nonzero_block, &[]);

        builder.switch_to_block(zero_block);
        builder.seal_block(zero_block);
        let zero = builder.ins().iconst(types::I64, 0);
        builder.ins().jump(merge_block, &[zero.into()]);

        builder.switch_to_block(nonzero_block);
        builder.seal_block(nonzero_block);
        let lhs_min = builder.ins().icmp_imm(IntCC::Equal, lhs, i64::MIN);
        let rhs_neg_one = builder.ins().icmp_imm(IntCC::Equal, rhs, -1);
        let overflows = builder.ins().band(lhs_min, rhs_neg_one);
        builder.ins().brif(overflows, overflow_block, &[], divide_block, &[]);

        builder.switch_to_block(overflow_block);
        builder.seal_block(overflow_block);
        builder.ins().jump(merge_block, &[lhs.into()]);

        builder.switch_to_block(divide_block);
        builder.seal_block(divide_block);
        let quotient = builder.ins().sdiv(lhs, rhs);
        builder.ins().jump(merge_block, &[quotient.into()]);

        builder.switch_to_block(merge_block);
        builder.seal_block(merge_block);
        let quotient = builder.block_params(merge_block)[0];
        let quotient_hi = builder.ins().ushr_imm(quotient, 32);
        let quotient_hi = builder.ins().ireduce(types::I32, quotient_hi);
        let quotient_lo = builder.ins().ireduce(types::I32, quotient);
        builder.ins().store(
            translator::vmctx_flags(),
            quotient_hi,
            ctx_ptr,
            (abi::gpr_base_offset::<SYSTEM>() + 3 * 4) as i32,
        );
        builder.ins().store(
            translator::vmctx_flags(),
            quotient_lo,
            ctx_ptr,
            (abi::gpr_base_offset::<SYSTEM>() + 4 * 4) as i32,
        );

        let cycles_off = abi::cycles_offset::<SYSTEM>() as i32;
        let cycles = builder
            .ins()
            .load(types::I64, translator::vmctx_flags(), ctx_ptr, cycles_off);
        let cycles = builder.ins().iadd_imm(cycles, 300);
        builder
            .ins()
            .store(translator::vmctx_flags(), cycles, ctx_ptr, cycles_off);

        let lr = builder.ins().load(
            types::I32,
            translator::vmctx_flags(),
            ctx_ptr,
            abi::lr_offset::<SYSTEM>() as i32,
        );
        builder.ins().return_(&[lr]);
        builder.finalize();

        self.module
            .define_function(func_id, &mut self.ctx)
            .expect("define __div2i intrinsic");
        self.module.finalize_definitions().expect("finalize __div2i intrinsic");
        let entry = self.module.get_finalized_function(func_id) as usize;

        let slot_addr = self.target_slot_addr(pc);
        unsafe {
            *(slot_addr as *mut usize) = entry;
        }

        self.register_in_lookup_table(pc, entry);

        entry
    }

    fn compile(&mut self, spec: &block::BlockSpec, gprs: &[u32; 32]) -> BlockEntry {
        let func_id = self.func_id_for(spec.start_pc);

        let _ = self.target_slot_addr(spec.start_pc);

        #[cfg(feature = "jit-stats")]
        let entry_counter_addr = Some(self.block_entry_counter_slot(spec.start_pc));
        #[cfg(not(feature = "jit-stats"))]
        let entry_counter_addr: Option<usize> = None;

        let entry = self.compile_function(spec, gprs, func_id, entry_counter_addr);

        let slot_addr = self.target_slot_addr(spec.start_pc);
        unsafe {
            *(slot_addr as *mut usize) = entry;
        }

        self.register_in_lookup_table(spec.start_pc, entry);

        entry
    }

    fn compile_function(
        &mut self,
        spec: &block::BlockSpec,
        gprs: &[u32; 32],
        func_id: FuncId,
        entry_counter_addr: Option<usize>,
    ) -> BlockEntry {
        self.ctx.clear();
        self.ctx.func.signature = self.block_sig.clone();

        let block_lookup_table_addr = self.block_lookup_table_addr as i64;
        let chain = translator::ChainContext {
            self_pc: spec.start_pc,
            target_slots: &self.target_slots,
            block_lookup_table_addr,
        };

        translator::translate::<SYSTEM>(
            &mut self.ctx,
            &mut self.builder_ctx,
            &mut self.module,
            &self.extern_funcs,
            spec,
            gprs,
            &chain,
            entry_counter_addr,
        );

        drop(chain);

        self.module
            .define_function(func_id, &mut self.ctx)
            .expect("define block");

        self.module.finalize_definitions().expect("finalize");
        let entry = self.module.get_finalized_function(func_id) as usize;
        crate::jit::register_jit_code(
            "ppc",
            spec.start_pc,
            entry,
            self.ctx.compiled_code().unwrap().code_buffer().len(),
        );

        entry
    }
}

impl<const SYSTEM: SystemId> Default for JitEngine<SYSTEM> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "jit-stats")]
fn decode_gekko_instr(raw: u32) -> String {
    let bytes = raw.to_be_bytes();
    match disasm::gekko::GekkoInstruction::decode(&bytes) {
        Some((ins, _)) => format!("{ins}"),
        None => format!("<undecoded raw={:08X}>", raw),
    }
}

#[cfg(feature = "jit-stats")]
fn looks_like_polling_shape(spec: &block::BlockSpec, body_cap: usize) -> bool {
    if spec.terminator != block::TermKind::BranchCond {
        return false;
    }

    if spec.instrs.is_empty() || spec.instrs.len() > body_cap {
        return false;
    }

    let last_idx = spec.instrs.len() - 1;
    let raw = spec.instrs[last_idx];
    if (raw >> 26) != 16 {
        return false;
    }

    if raw & 1 != 0 {
        return false;
    }

    let bo = (raw >> 21) & 0x1F;
    if bo & 0b00100 == 0 {
        return false;
    }

    let aa = (raw >> 1) & 1 != 0;
    let bd = (((raw >> 2) & 0x3FFF) as i32) << 18 >> 18;
    let bd_bytes = bd << 2;
    let term_pc = spec.start_pc.wrapping_add((last_idx as u32) * 4);
    let target = if aa {
        bd_bytes as u32
    } else {
        term_pc.wrapping_add_signed(bd_bytes)
    };

    target == spec.start_pc
}

fn build_trampoline(
    module: &mut JITModule,
    host_cc: CallConv,
    pointer_type: cranelift_codegen::ir::Type,
    block_sig: &Signature,
) -> TrampolineFn {
    let mut tramp_sig = Signature::new(host_cc);
    tramp_sig.params.push(AbiParam::new(pointer_type));
    tramp_sig.params.push(AbiParam::new(pointer_type));
    tramp_sig.returns.push(AbiParam::new(types::I32));

    let func_id = module
        .declare_function("gecko_jit_trampoline", Linkage::Local, &tramp_sig)
        .expect("declare trampoline");

    let mut ctx = Context::new();
    ctx.func.signature = tramp_sig.clone();
    let block_sig_ref = ctx.func.import_signature(block_sig.clone());

    {
        let mut bcx_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut bcx_ctx);
        let entry = builder.create_block();

        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        let ctx_ptr = builder.block_params(entry)[0];
        let block_ptr = builder.block_params(entry)[1];
        let no_host_return = builder.ins().iconst(types::I32, u32::MAX as i64);
        let call = builder
            .ins()
            .call_indirect(block_sig_ref, block_ptr, &[ctx_ptr, no_host_return]);
        let result = builder.inst_results(call)[0];
        builder.ins().return_(&[result]);
        builder.finalize();
    }

    module.define_function(func_id, &mut ctx).expect("define trampoline");
    module.finalize_definitions().expect("finalize trampoline");

    let ptr = module.get_finalized_function(func_id);
    unsafe { core::mem::transmute::<*const u8, TrampolineFn>(ptr) }
}

#[inline(always)]
pub fn dispatch<const SYSTEM: SystemId>(
    t: &mut translator::JitTranslator,
    instr: crate::gekko::instruction::Instruction,
) {
    if SYSTEM == GC {
        self::lut::dispatch(t, instr);
    } else {
        self::lut_wii::dispatch(t, instr);
    }
}
