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
}

pub struct JitEngine<const SYSTEM: SystemId> {
    module: JITModule,
    ctx: Context,
    builder_ctx: FunctionBuilderContext,
    cache: FxHashMap<u32, BlockEntry>,
    block_func_ids: FxHashMap<u32, FuncId>,
    pending_chain_slots: FxHashMap<u32, Vec<usize>>,
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
    dump_pc: Option<u32>,
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
        ] {
            jit_builder.symbol(name, addr);
        }

        let mut module = JITModule::new(jit_builder);

        let pointer_type = module.target_config().pointer_type();
        let call_conv = module.target_config().default_call_conv;

        let mut block_sig = Signature::new(CallConv::Tail);
        block_sig.params.push(AbiParam::new(pointer_type));
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
            pending_chain_slots: FxHashMap::default(),
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
            dump_pc: std::env::var("GECKO_DUMP_PC")
                .ok()
                .and_then(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()),
        }
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

    pub fn cached_blocks(&self) -> Vec<crate::jit_cache::CachedBlockPpc> {
        self.cache
            .keys()
            .filter_map(|&pc| {
                let spec = self.block_specs.get(&pc)?;
                Some(crate::jit_cache::CachedBlockPpc {
                    pc,
                    instr_count: spec.instrs.len() as u16,
                    hash: crate::jit_cache::hash_words(spec.instrs.iter().copied()),
                })
            })
            .collect()
    }

    pub fn precompile_blocks(
        &mut self,
        sys: &mut System<SYSTEM>,
        blocks: &[crate::jit_cache::CachedBlockPpc],
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

            let actual = crate::jit_cache::hash_words(buf.into_iter());
            if actual != b.hash {
                skipped += 1;
                continue;
            }

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

        let spec = block::discover::<SYSTEM>(sys, pc);
        let gprs_snapshot = sys.gekko.gprs;

        let entry = self.compile(&spec, &gprs_snapshot);
        self.cache.insert(pc, entry);
        self.block_specs.insert(pc, spec);

        entry
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
        self.pending_chain_slots.clear();

        unsafe {
            let table = self.block_lookup_table_addr as *mut BlockLookupSlot;
            for i in 0..BLOCK_LOOKUP_TABLE_SIZE {
                (*table.add(i)).pc = 0;
                (*table.add(i)).entry = 0;
            }
        }
    }

    fn compile(&mut self, spec: &block::BlockSpec, gprs: &[u32; 32]) -> BlockEntry {
        let func_id = self.func_id_for(spec.start_pc);

        self.ctx.clear();
        self.ctx.func.signature = self.block_sig.clone();

        let self_slot: &'static mut usize = Box::leak(Box::new(0usize));
        let self_slot_addr = self_slot as *mut usize as i64;

        let mut pending_for_this_block: Vec<(u32, usize)> = Vec::new();

        #[cfg(feature = "jit-stats")]
        let entry_counter_addr = Some(self.block_entry_counter_slot(spec.start_pc));
        #[cfg(not(feature = "jit-stats"))]
        let entry_counter_addr: Option<usize> = None;

        let chain = translator::ChainContext {
            self_pc: spec.start_pc,
            self_func_id: func_id,
            self_slot_addr,
            compiled_func_ids: &self.block_func_ids,
            compiled_cache: &self.cache,
            pending: std::cell::RefCell::new(&mut pending_for_this_block),
            block_lookup_table_addr: self.block_lookup_table_addr as i64,
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
        for (target_pc, slot_addr) in pending_for_this_block.drain(..) {
            self.pending_chain_slots.entry(target_pc).or_default().push(slot_addr);
        }

        let want_dump = self.dump_pc == Some(spec.start_pc);
        if want_dump {
            self.ctx.set_disasm(true);
        }

        self.module
            .define_function(func_id, &mut self.ctx)
            .expect("define block");

        if want_dump {
            if let Some(cc) = self.ctx.compiled_code() {
                let path = format!("./profile-dumps/jit-disasm-{:08x}.txt", spec.start_pc);
                let mut s = String::new();

                use std::fmt::Write as _;

                let _ = writeln!(
                    s,
                    "; PPC block at {:#010x}, len={} instrs, terminator={:?}",
                    spec.start_pc,
                    spec.instrs.len(),
                    spec.terminator
                );

                for (i, &raw) in spec.instrs.iter().enumerate() {
                    let pc = spec.pc_of(i);
                    let _ = writeln!(s, ";   {:08x}  {:08x}", pc, raw);
                }

                let _ = writeln!(s, "; --- cranelift vcode ---");
                if let Some(vcode) = cc.vcode.as_ref() {
                    s.push_str(vcode);
                }

                let _ = writeln!(s, "\n; --- emitted bytes ({} bytes) ---", cc.code_buffer().len());
                for chunk in cc.code_buffer().chunks(16) {
                    s.push_str("; ");
                    for b in chunk {
                        let _ = write!(s, "{:02x} ", b);
                    }
                    s.push('\n');
                }

                let _ = std::fs::write(&path, s);
                tracing::info!(pc = format_args!("{:#010x}", spec.start_pc).to_string(), %path, "dumped JIT disasm");
            }
        }

        self.module.finalize_definitions().expect("finalize");
        let entry = self.module.get_finalized_function(func_id) as usize;

        unsafe {
            *(self_slot_addr as *mut usize) = entry;
        }

        if let Some(slots) = self.pending_chain_slots.remove(&spec.start_pc) {
            for slot_addr in slots {
                unsafe {
                    *(slot_addr as *mut usize) = entry;
                }
            }
        }

        self.register_in_lookup_table(spec.start_pc, entry);

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
        let call = builder.ins().call_indirect(block_sig_ref, block_ptr, &[ctx_ptr]);
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
