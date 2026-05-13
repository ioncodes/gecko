pub mod abi;
pub mod block;
pub mod runtime;
pub mod translate;
pub mod translator;

#[allow(dead_code, unused_variables, non_upper_case_globals, clippy::all)]
pub mod jit_lut {
    include!(concat!(env!("OUT_DIR"), "/dsp_jit_lut.rs"));
}

use cranelift_codegen::Context;
use cranelift_codegen::ir::Signature;
use cranelift_frontend::FunctionBuilderContext;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, default_libcall_names};
use rustc_hash::FxHashMap;

use crate::system::{GC, SystemId, WII};

pub type BlockEntry = usize;

type TrampolineFn = unsafe extern "C" fn(*mut core::ffi::c_void, usize) -> u32;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct DspBlockLookupSlot {
    pub pc: u32,
    pub _pad: u32,
    pub entry: usize,
}

pub const DSP_BLOCK_LOOKUP_TABLE_SIZE: usize = 8192;

const CHAIN_DEPTH_HISTOGRAM_LEN: usize = super::DSP_JIT_CHAIN_BUDGET as usize + 2;

pub struct JitEngine<const SYSTEM: SystemId> {
    module: JITModule,
    ctx: Context,
    builder_ctx: FunctionBuilderContext,
    cache: FxHashMap<u16, BlockEntry>,
    block_func_ids: FxHashMap<u16, FuncId>,
    block_sig: Signature,
    extern_funcs: translator::ExternFuncs,
    trampoline_fn: TrampolineFn,
    block_seq: u64,
    block_lookup_table_addr: usize,
    chain_depth_total: u64,
    dispatcher_entries_total: u64,
    chain_depth_histogram: [u64; CHAIN_DEPTH_HISTOGRAM_LEN],
    #[cfg(feature = "jit-stats")]
    pub(crate) hits: FxHashMap<u16, u64>,
    pub(crate) block_specs: FxHashMap<u16, block::BlockSpec>,
    #[cfg(feature = "jit-stats")]
    block_entry_counter_ptrs: FxHashMap<u16, usize>,
}

impl<const SYSTEM: SystemId> JitEngine<SYSTEM> {
    pub fn new() -> Self {
        use cranelift_codegen::settings::{self, Configurable};

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

        struct Syms {
            cache_ext_ac: (&'static str, *const u8),
            loop_tail: (&'static str, *const u8),
            update_flags_logic: (&'static str, *const u8),
            update_flags_add: (&'static str, *const u8),
            update_flags_sub: (&'static str, *const u8),
            update_flags_ac: (&'static str, *const u8),
            read_dmem: (&'static str, *const u8),
            write_dmem: (&'static str, *const u8),
            inc_ar: (&'static str, *const u8),
            dec_ar: (&'static str, *const u8),
            increase_ar: (&'static str, *const u8),
            decrease_ar_ix: (&'static str, *const u8),
            dynamic_shift: (&'static str, *const u8),
            read_imem: (&'static str, *const u8),
            write_ac_mid_sxm: (&'static str, *const u8),
            call_stack_push: (&'static str, *const u8),
            call_stack_pop: (&'static str, *const u8),
            data_stack_pop: (&'static str, *const u8),
            read_reg_full: (&'static str, *const u8),
            write_reg_full: (&'static str, *const u8),
            loop_setup: (&'static str, *const u8),
        }
        let syms: Syms = match SYSTEM {
            GC => Syms {
                cache_ext_ac: (
                    "gecko_dsp_jit_cache_ext_ac_gc",
                    runtime::dsp_cache_ext_ac_gc as *const u8,
                ),
                loop_tail: ("gecko_dsp_jit_loop_tail_gc", runtime::dsp_loop_tail_gc as *const u8),
                update_flags_logic: (
                    "gecko_dsp_jit_uflog_gc",
                    runtime::dsp_update_flags_logic_gc as *const u8,
                ),
                update_flags_add: ("gecko_dsp_jit_ufadd_gc", runtime::dsp_update_flags_add_gc as *const u8),
                update_flags_sub: ("gecko_dsp_jit_ufsub_gc", runtime::dsp_update_flags_sub_gc as *const u8),
                update_flags_ac: ("gecko_dsp_jit_ufac_gc", runtime::dsp_update_flags_ac_gc as *const u8),
                read_dmem: ("gecko_dsp_jit_rdmem_gc", runtime::dsp_read_dmem_gc as *const u8),
                write_dmem: ("gecko_dsp_jit_wdmem_gc", runtime::dsp_write_dmem_gc as *const u8),
                inc_ar: ("gecko_dsp_jit_incar_gc", runtime::dsp_increment_ar_gc as *const u8),
                dec_ar: ("gecko_dsp_jit_decar_gc", runtime::dsp_decrement_ar_gc as *const u8),
                increase_ar: ("gecko_dsp_jit_incrar_gc", runtime::dsp_increase_ar_gc as *const u8),
                decrease_ar_ix: ("gecko_dsp_jit_decarix_gc", runtime::dsp_decrease_ar_ix_gc as *const u8),
                dynamic_shift: ("gecko_dsp_jit_dynshift_gc", runtime::dsp_dynamic_shift_gc as *const u8),
                read_imem: ("gecko_dsp_jit_rimem_gc", runtime::dsp_read_imem_gc as *const u8),
                write_ac_mid_sxm: ("gecko_dsp_jit_wamsxm_gc", runtime::dsp_write_ac_mid_sxm_gc as *const u8),
                call_stack_push: ("gecko_dsp_jit_cspush_gc", runtime::dsp_call_stack_push_gc as *const u8),
                call_stack_pop: ("gecko_dsp_jit_cspop_gc", runtime::dsp_call_stack_pop_gc as *const u8),
                data_stack_pop: ("gecko_dsp_jit_dspop_gc", runtime::dsp_data_stack_pop_gc as *const u8),
                read_reg_full: ("gecko_dsp_jit_rdregf_gc", runtime::dsp_read_reg_full_gc as *const u8),
                write_reg_full: ("gecko_dsp_jit_wrregf_gc", runtime::dsp_write_reg_full_gc as *const u8),
                loop_setup: ("gecko_dsp_jit_loopsetup_gc", runtime::dsp_loop_setup_gc as *const u8),
            },
            WII => Syms {
                cache_ext_ac: (
                    "gecko_dsp_jit_cache_ext_ac_wii",
                    runtime::dsp_cache_ext_ac_wii as *const u8,
                ),
                loop_tail: ("gecko_dsp_jit_loop_tail_wii", runtime::dsp_loop_tail_wii as *const u8),
                update_flags_logic: (
                    "gecko_dsp_jit_uflog_wii",
                    runtime::dsp_update_flags_logic_wii as *const u8,
                ),
                update_flags_add: (
                    "gecko_dsp_jit_ufadd_wii",
                    runtime::dsp_update_flags_add_wii as *const u8,
                ),
                update_flags_sub: (
                    "gecko_dsp_jit_ufsub_wii",
                    runtime::dsp_update_flags_sub_wii as *const u8,
                ),
                update_flags_ac: ("gecko_dsp_jit_ufac_wii", runtime::dsp_update_flags_ac_wii as *const u8),
                read_dmem: ("gecko_dsp_jit_rdmem_wii", runtime::dsp_read_dmem_wii as *const u8),
                write_dmem: ("gecko_dsp_jit_wdmem_wii", runtime::dsp_write_dmem_wii as *const u8),
                inc_ar: ("gecko_dsp_jit_incar_wii", runtime::dsp_increment_ar_wii as *const u8),
                dec_ar: ("gecko_dsp_jit_decar_wii", runtime::dsp_decrement_ar_wii as *const u8),
                increase_ar: ("gecko_dsp_jit_incrar_wii", runtime::dsp_increase_ar_wii as *const u8),
                decrease_ar_ix: (
                    "gecko_dsp_jit_decarix_wii",
                    runtime::dsp_decrease_ar_ix_wii as *const u8,
                ),
                dynamic_shift: (
                    "gecko_dsp_jit_dynshift_wii",
                    runtime::dsp_dynamic_shift_wii as *const u8,
                ),
                read_imem: ("gecko_dsp_jit_rimem_wii", runtime::dsp_read_imem_wii as *const u8),
                write_ac_mid_sxm: (
                    "gecko_dsp_jit_wamsxm_wii",
                    runtime::dsp_write_ac_mid_sxm_wii as *const u8,
                ),
                call_stack_push: (
                    "gecko_dsp_jit_cspush_wii",
                    runtime::dsp_call_stack_push_wii as *const u8,
                ),
                call_stack_pop: ("gecko_dsp_jit_cspop_wii", runtime::dsp_call_stack_pop_wii as *const u8),
                data_stack_pop: ("gecko_dsp_jit_dspop_wii", runtime::dsp_data_stack_pop_wii as *const u8),
                read_reg_full: ("gecko_dsp_jit_rdregf_wii", runtime::dsp_read_reg_full_wii as *const u8),
                write_reg_full: ("gecko_dsp_jit_wrregf_wii", runtime::dsp_write_reg_full_wii as *const u8),
                loop_setup: ("gecko_dsp_jit_loopsetup_wii", runtime::dsp_loop_setup_wii as *const u8),
            },
            _ => unreachable!(),
        };
        jit_builder.symbol(syms.cache_ext_ac.0, syms.cache_ext_ac.1);
        jit_builder.symbol(syms.loop_tail.0, syms.loop_tail.1);
        jit_builder.symbol(syms.update_flags_logic.0, syms.update_flags_logic.1);
        jit_builder.symbol(syms.update_flags_add.0, syms.update_flags_add.1);
        jit_builder.symbol(syms.update_flags_sub.0, syms.update_flags_sub.1);
        jit_builder.symbol(syms.update_flags_ac.0, syms.update_flags_ac.1);
        jit_builder.symbol(syms.read_dmem.0, syms.read_dmem.1);
        jit_builder.symbol(syms.write_dmem.0, syms.write_dmem.1);
        jit_builder.symbol(syms.inc_ar.0, syms.inc_ar.1);
        jit_builder.symbol(syms.dec_ar.0, syms.dec_ar.1);
        jit_builder.symbol(syms.increase_ar.0, syms.increase_ar.1);
        jit_builder.symbol(syms.decrease_ar_ix.0, syms.decrease_ar_ix.1);
        jit_builder.symbol(syms.dynamic_shift.0, syms.dynamic_shift.1);
        jit_builder.symbol(syms.read_imem.0, syms.read_imem.1);
        jit_builder.symbol(syms.write_ac_mid_sxm.0, syms.write_ac_mid_sxm.1);
        jit_builder.symbol(syms.call_stack_push.0, syms.call_stack_push.1);
        jit_builder.symbol(syms.call_stack_pop.0, syms.call_stack_pop.1);
        jit_builder.symbol(syms.data_stack_pop.0, syms.data_stack_pop.1);
        jit_builder.symbol(syms.read_reg_full.0, syms.read_reg_full.1);
        jit_builder.symbol(syms.write_reg_full.0, syms.write_reg_full.1);
        jit_builder.symbol(syms.loop_setup.0, syms.loop_setup.1);

        let mut module = JITModule::new(jit_builder);

        let pointer_type = module.target_config().pointer_type();
        let host_cc = module.target_config().default_call_conv;

        let block_sig = translator::block_signature(pointer_type);
        let void_thunk_sig = translator::void_thunk_signature(pointer_type, host_cc);
        let flags_logic_sig = translator::flags_logic_signature(pointer_type, host_cc);
        let flags_arith_sig = translator::flags_arith_signature(pointer_type, host_cc);
        let flags_ac_sig = translator::flags_ac_signature(pointer_type, host_cc);
        let dmem_read_sig = translator::dmem_read_signature(pointer_type, host_cc);
        let dmem_write_sig = translator::dmem_write_signature(pointer_type, host_cc);
        let ar_unary_sig = translator::ar_unary_signature(pointer_type, host_cc);
        let ar_binary_sig = translator::ar_binary_signature(pointer_type, host_cc);
        let dynamic_shift_sig = translator::dynamic_shift_signature(pointer_type, host_cc);

        let cache_ext_ac_id = module
            .declare_function(syms.cache_ext_ac.0, Linkage::Import, &void_thunk_sig)
            .expect("declare cache_ext_ac thunk");
        let loop_tail_id = module
            .declare_function(syms.loop_tail.0, Linkage::Import, &void_thunk_sig)
            .expect("declare loop_tail thunk");
        let update_flags_logic_id = module
            .declare_function(syms.update_flags_logic.0, Linkage::Import, &flags_logic_sig)
            .expect("declare update_flags_logic thunk");
        let update_flags_add_id = module
            .declare_function(syms.update_flags_add.0, Linkage::Import, &flags_arith_sig)
            .expect("declare update_flags_add thunk");
        let update_flags_sub_id = module
            .declare_function(syms.update_flags_sub.0, Linkage::Import, &flags_arith_sig)
            .expect("declare update_flags_sub thunk");
        let update_flags_ac_id = module
            .declare_function(syms.update_flags_ac.0, Linkage::Import, &flags_ac_sig)
            .expect("declare update_flags_ac thunk");
        let read_dmem_id = module
            .declare_function(syms.read_dmem.0, Linkage::Import, &dmem_read_sig)
            .expect("declare read_dmem thunk");
        let write_dmem_id = module
            .declare_function(syms.write_dmem.0, Linkage::Import, &dmem_write_sig)
            .expect("declare write_dmem thunk");
        let inc_ar_id = module
            .declare_function(syms.inc_ar.0, Linkage::Import, &ar_unary_sig)
            .expect("declare inc_ar thunk");
        let dec_ar_id = module
            .declare_function(syms.dec_ar.0, Linkage::Import, &ar_unary_sig)
            .expect("declare dec_ar thunk");
        let increase_ar_id = module
            .declare_function(syms.increase_ar.0, Linkage::Import, &ar_binary_sig)
            .expect("declare increase_ar thunk");
        let decrease_ar_ix_id = module
            .declare_function(syms.decrease_ar_ix.0, Linkage::Import, &ar_binary_sig)
            .expect("declare decrease_ar_ix thunk");
        let dynamic_shift_id = module
            .declare_function(syms.dynamic_shift.0, Linkage::Import, &dynamic_shift_sig)
            .expect("declare dynamic_shift thunk");
        let read_imem_id = module
            .declare_function(syms.read_imem.0, Linkage::Import, &dmem_read_sig)
            .expect("declare read_imem thunk");
        let write_ac_mid_sxm_id = module
            .declare_function(syms.write_ac_mid_sxm.0, Linkage::Import, &dmem_write_sig)
            .expect("declare write_ac_mid_sxm thunk");

        let call_stack_push_sig = {
            let mut sig = cranelift_codegen::ir::Signature::new(host_cc);
            sig.params.push(cranelift_codegen::ir::AbiParam::new(pointer_type));
            sig.params
                .push(cranelift_codegen::ir::AbiParam::new(cranelift_codegen::ir::types::I32));
            sig
        };
        let call_stack_push_id = module
            .declare_function(syms.call_stack_push.0, Linkage::Import, &call_stack_push_sig)
            .expect("declare call_stack_push thunk");

        let stack_pop_sig = translator::stack_pop_signature(pointer_type, host_cc);
        let call_stack_pop_id = module
            .declare_function(syms.call_stack_pop.0, Linkage::Import, &stack_pop_sig)
            .expect("declare call_stack_pop thunk");
        let data_stack_pop_id = module
            .declare_function(syms.data_stack_pop.0, Linkage::Import, &stack_pop_sig)
            .expect("declare data_stack_pop thunk");

        let read_reg_full_id = module
            .declare_function(syms.read_reg_full.0, Linkage::Import, &dmem_read_sig)
            .expect("declare read_reg_full thunk");

        let write_reg_full_id = module
            .declare_function(syms.write_reg_full.0, Linkage::Import, &dmem_write_sig)
            .expect("declare write_reg_full thunk");
        let loop_setup_id = module
            .declare_function(
                syms.loop_setup.0,
                Linkage::Import,
                &translator::loop_setup_signature(pointer_type, host_cc),
            )
            .expect("declare loop_setup thunk");

        let extern_funcs = translator::ExternFuncs {
            cache_ext_ac: cache_ext_ac_id,
            loop_tail: loop_tail_id,
            update_flags_logic: update_flags_logic_id,
            update_flags_add: update_flags_add_id,
            update_flags_sub: update_flags_sub_id,
            update_flags_ac: update_flags_ac_id,
            read_dmem: read_dmem_id,
            write_dmem: write_dmem_id,
            inc_ar: inc_ar_id,
            dec_ar: dec_ar_id,
            increase_ar: increase_ar_id,
            decrease_ar_ix: decrease_ar_ix_id,
            dynamic_shift: dynamic_shift_id,
            read_imem: read_imem_id,
            write_ac_mid_sxm: write_ac_mid_sxm_id,
            call_stack_push: call_stack_push_id,
            call_stack_pop: call_stack_pop_id,
            data_stack_pop: data_stack_pop_id,
            read_reg_full: read_reg_full_id,
            write_reg_full: write_reg_full_id,
            loop_setup: loop_setup_id,
        };

        let trampoline_fn = build_trampoline(&mut module, host_cc, pointer_type, &block_sig);

        let table = vec![
            DspBlockLookupSlot {
                pc: 0,
                _pad: 0,
                entry: 0
            };
            DSP_BLOCK_LOOKUP_TABLE_SIZE
        ]
        .into_boxed_slice();
        let block_lookup_table_addr = Box::leak(table).as_mut_ptr() as usize;

        Self {
            module,
            ctx: Context::new(),
            builder_ctx: FunctionBuilderContext::new(),
            cache: FxHashMap::default(),
            block_func_ids: FxHashMap::default(),
            block_sig,
            extern_funcs,
            trampoline_fn,
            block_seq: 0,
            block_lookup_table_addr,
            chain_depth_total: 0,
            dispatcher_entries_total: 0,
            chain_depth_histogram: [0; CHAIN_DEPTH_HISTOGRAM_LEN],
            #[cfg(feature = "jit-stats")]
            hits: FxHashMap::default(),
            block_specs: FxHashMap::default(),
            #[cfg(feature = "jit-stats")]
            block_entry_counter_ptrs: FxHashMap::default(),
        }
    }

    #[cfg(feature = "jit-stats")]
    fn block_entry_counter_slot(&mut self, pc: u16) -> usize {
        if let Some(&addr) = self.block_entry_counter_ptrs.get(&pc) {
            return addr;
        }
        let slot: &'static mut u64 = Box::leak(Box::new(0u64));
        let addr = slot as *mut u64 as usize;
        self.block_entry_counter_ptrs.insert(pc, addr);
        addr
    }

    pub fn record_chain_depth(&mut self, depth: u32) {
        self.chain_depth_total = self.chain_depth_total.wrapping_add(depth as u64);
        self.dispatcher_entries_total = self.dispatcher_entries_total.wrapping_add(1);
        let bucket = (depth as usize).min(self.chain_depth_histogram.len() - 1);
        self.chain_depth_histogram[bucket] = self.chain_depth_histogram[bucket].wrapping_add(1);
    }

    fn register_in_lookup_table(&self, pc: u16, entry: usize) {
        let pc_u32 = pc as u32;
        let idx = ((pc_u32 & 0xFFF) | ((pc_u32 >> 3) & 0x1000)) as usize;
        unsafe {
            let table = self.block_lookup_table_addr as *mut DspBlockLookupSlot;
            (*table.add(idx)).pc = pc_u32;
            (*table.add(idx)).entry = entry;
        }
    }

    fn clear_lookup_table(&self) {
        unsafe {
            let table = self.block_lookup_table_addr as *mut DspBlockLookupSlot;
            for i in 0..DSP_BLOCK_LOOKUP_TABLE_SIZE {
                (*table.add(i)).pc = 0;
                (*table.add(i)).entry = 0;
            }
        }
    }

    pub fn lookup_or_compile(&mut self, iram: &[u8], irom: &[u8], start_pc: u16) -> BlockEntry {
        if let Some(&entry) = self.cache.get(&start_pc) {
            #[cfg(feature = "jit-stats")]
            {
                *self.hits.entry(start_pc).or_insert(0) += 1;
            }
            return entry;
        }
        let spec = block::discover(iram, irom, start_pc);
        let entry = self.compile(&spec);
        self.cache.insert(start_pc, entry);
        #[cfg(feature = "jit-stats")]
        {
            *self.hits.entry(start_pc).or_insert(0) += 1;
        }
        self.block_specs.insert(start_pc, spec);
        entry
    }

    pub fn cached_blocks(&self) -> Vec<crate::jit_cache::CachedBlockDsp> {
        self.cache
            .keys()
            .filter_map(|&pc| {
                let spec = self.block_specs.get(&pc)?;
                Some(crate::jit_cache::CachedBlockDsp {
                    pc,
                    instr_count: spec.instrs.len() as u16,
                    hash: crate::jit_cache::hash_words(spec.instrs.iter().map(|e| e.raw)),
                })
            })
            .collect()
    }

    pub fn precompile_blocks(
        &mut self,
        iram: &[u8],
        irom: &[u8],
        blocks: &[crate::jit_cache::CachedBlockDsp],
    ) -> (usize, usize) {
        let mut compiled = 0usize;
        let mut skipped = 0usize;

        for b in blocks {
            if self.cache.contains_key(&b.pc) {
                continue;
            }

            if b.instr_count == 0 {
                skipped += 1;
                continue;
            }

            let spec = block::discover(iram, irom, b.pc);
            if spec.instrs.len() != b.instr_count as usize {
                skipped += 1;
                continue;
            }

            let actual = crate::jit_cache::hash_words(spec.instrs.iter().map(|e| e.raw));
            if actual != b.hash {
                skipped += 1;
                continue;
            }

            self.lookup_or_compile(iram, irom, b.pc);

            compiled += 1;
        }
        (compiled, skipped)
    }

    fn func_id_for(&mut self, pc: u16) -> FuncId {
        if let Some(&id) = self.block_func_ids.get(&pc) {
            return id;
        }
        self.block_seq = self.block_seq.wrapping_add(1);
        let name = format!("gecko_dsp_block_{:04x}_{}", pc, self.block_seq);
        let id = self
            .module
            .declare_function(&name, Linkage::Local, &self.block_sig)
            .expect("declare dsp block");
        self.block_func_ids.insert(pc, id);
        id
    }

    #[cfg(feature = "jit-stats")]
    pub fn dump_block_clif(&mut self, start_pc: u16, iram: &[u8], irom: &[u8]) {
        let spec = block::discover(iram, irom, start_pc);

        self.ctx.clear();
        self.ctx.func.signature = self.block_sig.clone();
        translator::translate(
            &mut self.ctx,
            &mut self.builder_ctx,
            &mut self.module,
            &self.extern_funcs,
            &spec,
            self.block_lookup_table_addr as i64,
            None,
        );
        tracing::info!(
            "CLIF for DSP block pc={:04X} (len={}, term={:?})",
            spec.start_pc,
            spec.instrs.len(),
            spec.terminator,
        );
        tracing::info!("{}", self.ctx.func.display());
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure(label = "dsp_jit_compile"))]
    fn compile(&mut self, spec: &block::BlockSpec) -> BlockEntry {
        let func_id = self.func_id_for(spec.start_pc);

        self.ctx.clear();
        self.ctx.func.signature = self.block_sig.clone();

        #[cfg(feature = "jit-stats")]
        let entry_counter_addr = Some(self.block_entry_counter_slot(spec.start_pc));
        #[cfg(not(feature = "jit-stats"))]
        let entry_counter_addr: Option<usize> = None;

        translator::translate(
            &mut self.ctx,
            &mut self.builder_ctx,
            &mut self.module,
            &self.extern_funcs,
            spec,
            self.block_lookup_table_addr as i64,
            entry_counter_addr,
        );

        self.module
            .define_function(func_id, &mut self.ctx)
            .expect("define dsp block");
        self.module.finalize_definitions().expect("finalize dsp jit");

        let entry = self.module.get_finalized_function(func_id) as usize;

        self.register_in_lookup_table(spec.start_pc, entry);

        entry
    }

    pub fn flush(&mut self) {
        self.cache.clear();
        self.block_func_ids.clear();

        self.clear_lookup_table();
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure(label = "dsp_jit_run_block"))]
    pub fn run_block(&mut self, ctx_ptr: *mut core::ffi::c_void, entry: BlockEntry) -> u16 {
        let next_pc_u32 = unsafe { (self.trampoline_fn)(ctx_ptr, entry) };
        next_pc_u32 as u16
    }

    #[cfg(feature = "jit-stats")]
    pub fn dump_hot_blocks_csv(&self, top_k: usize, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;

        let mut entries: Vec<(u16, u64, u64, usize, String, String)> = self
            .block_entry_counter_ptrs
            .iter()
            .map(|(&pc, &addr)| {
                let executions = unsafe { *(addr as *const u64) };
                let dispatch_hits = self.hits.get(&pc).copied().unwrap_or(0);
                let spec = self.block_specs.get(&pc);
                let len = spec.map(|s| s.instrs.len()).unwrap_or(0);
                let term = spec.map(|s| format!("{:?}", s.terminator)).unwrap_or_default();
                let lead = spec
                    .and_then(|s| s.instrs.first())
                    .map(|e| disasm_mnemonic(e.raw))
                    .unwrap_or_default();
                (pc, executions, dispatch_hits, len, term, lead)
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
                "rank,start_pc,executions,dispatch_hits,instr_count,cycles_estimated,terminator,lead_mnemonic"
            )?;

            for (rank, (pc, exec, hits, len, term, lead)) in entries.iter().take(top_k).enumerate() {
                let cycles = exec.saturating_mul(*len as u64);
                writeln!(
                    file,
                    "{},{:#06X},{},{},{},{},{},{}",
                    rank, pc, exec, hits, len, cycles, term, lead
                )?;
            }

            Ok(())
        })
    }

    #[cfg(feature = "jit-stats")]
    pub fn dump_hot_blocks(&self, top_k: usize) {
        let mut entries: Vec<(u16, u64)> = self.hits.iter().map(|(&pc, &n)| (pc, n)).collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));

        if entries.is_empty() {
            tracing::info!("no dispatcher-entry hits recorded");
            return;
        }

        let total_entries: u64 = entries.iter().map(|e| e.1).sum();
        let total_instrs: u64 = entries
            .iter()
            .map(|(pc, n)| {
                let len = self.block_specs.get(pc).map(|s| s.instrs.len() as u64).unwrap_or(0);
                n.saturating_mul(len)
            })
            .sum();

        tracing::info!(
            "DSP JIT hot blocks (top {} of {}, {} dispatcher entries, {} instr executions)",
            top_k.min(entries.len()),
            entries.len(),
            total_entries,
            total_instrs,
        );

        if self.dispatcher_entries_total > 0 {
            let avg_depth = self.chain_depth_total as f64 / self.dispatcher_entries_total as f64;
            let budget_bucket = super::DSP_JIT_CHAIN_BUDGET as usize;
            let saturated = self.chain_depth_histogram.get(budget_bucket).copied().unwrap_or(0);
            let zero_depth = self.chain_depth_histogram[0];
            let saturated_pct = (saturated as f64) * 100.0 / (self.dispatcher_entries_total as f64);
            let zero_pct = (zero_depth as f64) * 100.0 / (self.dispatcher_entries_total as f64);
            tracing::info!(
                "    chain link: avg depth {:.2} (over {} run_block entries, {} chained tail-calls)",
                avg_depth,
                self.dispatcher_entries_total,
                self.chain_depth_total,
            );
            tracing::info!(
                "    chain link: {:.1}% returned without chaining (depth=0), {:.1}% saturated at the budget",
                zero_pct,
                saturated_pct,
            );

            let buckets: Vec<String> = self
                .chain_depth_histogram
                .iter()
                .enumerate()
                .filter(|(_, n)| **n > 0)
                .map(|(i, n)| format!("{}:{}", i, n))
                .collect();
            tracing::info!("    chain depth histogram (non-zero): {}", buckets.join(" "));
        } else {
            tracing::info!("    chain link: no run_block entries observed yet");
        }

        let mut per_mnem: rustc_hash::FxHashMap<String, u64> = rustc_hash::FxHashMap::default();
        for (pc, hits) in entries.iter() {
            if let Some(spec) = self.block_specs.get(pc) {
                for entry in &spec.instrs {
                    let mnem = disasm_mnemonic(entry.raw);
                    *per_mnem.entry(mnem).or_insert(0) += hits;
                }
            }
        }

        for (rank, (pc, hits)) in entries.iter().take(top_k).enumerate() {
            let pct = (*hits as f64) * 100.0 / (total_entries as f64);
            let spec = self.block_specs.get(pc);
            let len = spec.map(|s| s.instrs.len()).unwrap_or(0);
            let term = spec.map(|s| format!("{:?}", s.terminator)).unwrap_or_default();

            let total_block_entries = self
                .block_entry_counter_ptrs
                .get(pc)
                .map(|&addr| unsafe { *(addr as *const u64) })
                .unwrap_or(0);
            let chain_arrivals = total_block_entries.saturating_sub(*hits);
            let chain_rate = if total_block_entries > 0 {
                (chain_arrivals as f64) * 100.0 / (total_block_entries as f64)
            } else {
                0.0
            };
            tracing::info!(
                "  #{:>3}  pc={:04X}  hits={}  ({:.1}%)  total={}  chain={:.1}%  len={}  term={}",
                rank,
                pc,
                hits,
                pct,
                total_block_entries,
                chain_rate,
                len,
                term,
            );
            if let Some(spec) = spec {
                for entry in &spec.instrs {
                    let bytes = [
                        ((entry.raw >> 8) & 0xFF) as u8,
                        (entry.raw & 0xFF) as u8,
                        ((entry.raw >> 24) & 0xFF) as u8,
                        ((entry.raw >> 16) & 0xFF) as u8,
                    ];
                    let asm = disasm::dsp::GcDspInstruction::decode(&bytes)
                        .map(|(ins, _)| format!("{ins}"))
                        .unwrap_or_else(|| "<unknown>".to_string());
                    tracing::info!("       {:04X}  {:08X}  {}", entry.pc, entry.raw, asm,);
                }
            }
        }

        let mut mnem_entries: Vec<(String, u64)> = per_mnem.into_iter().collect();
        mnem_entries.sort_by(|a, b| b.1.cmp(&a.1));
        let mnem_total: u64 = mnem_entries.iter().map(|e| e.1).sum();
        tracing::info!(
            "DSP JIT hot mnemonics (top {} of {}, {} total executions)",
            top_k.min(mnem_entries.len()),
            mnem_entries.len(),
            mnem_total,
        );
        for (rank, (mnem, n)) in mnem_entries.iter().take(top_k).enumerate() {
            let pct = (*n as f64) * 100.0 / (mnem_total.max(1) as f64);
            tracing::info!("  #{:>3}  {:6}  {:>10}  ({:.1}%)", rank, mnem, n, pct);
        }
    }
}

#[cfg(feature = "jit-stats")]
fn disasm_mnemonic(raw: u32) -> String {
    let bytes = [
        ((raw >> 8) & 0xFF) as u8,
        (raw & 0xFF) as u8,
        ((raw >> 24) & 0xFF) as u8,
        ((raw >> 16) & 0xFF) as u8,
    ];

    let Some((ins, _)) = disasm::dsp::GcDspInstruction::decode(&bytes) else {
        return "<unk>".to_string();
    };

    let s = format!("{ins}");
    s.split_whitespace().next().map(|m| m.to_string()).unwrap_or_else(|| s)
}

impl<const SYSTEM: SystemId> Default for JitEngine<SYSTEM> {
    fn default() -> Self {
        Self::new()
    }
}

fn build_trampoline(
    module: &mut JITModule,
    host_cc: cranelift_codegen::isa::CallConv,
    pointer_type: cranelift_codegen::ir::Type,
    block_sig: &Signature,
) -> TrampolineFn {
    use cranelift_codegen::Context;
    use cranelift_codegen::ir::{AbiParam, InstBuilder, types};
    use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

    let mut tramp_sig = Signature::new(host_cc);
    tramp_sig.params.push(AbiParam::new(pointer_type));
    tramp_sig.params.push(AbiParam::new(pointer_type));
    tramp_sig.returns.push(AbiParam::new(types::I32));

    let id = module
        .declare_function("gecko_dsp_jit_trampoline", Linkage::Local, &tramp_sig)
        .expect("declare dsp trampoline");

    let mut ctx = Context::new();
    ctx.func.signature = tramp_sig;

    let mut bctx = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut bctx);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        let ctx_ptr = builder.block_params(entry)[0];
        let block_ptr = builder.block_params(entry)[1];

        let sig_ref = builder.import_signature(block_sig.clone());
        let call = builder.ins().call_indirect(sig_ref, block_ptr, &[ctx_ptr]);
        let ret = builder.inst_results(call)[0];
        builder.ins().return_(&[ret]);
        builder.finalize();
    }

    module.define_function(id, &mut ctx).expect("define dsp trampoline");
    module.finalize_definitions().expect("finalize dsp trampoline");

    let raw = module.get_finalized_function(id);
    unsafe { core::mem::transmute::<*const u8, TrampolineFn>(raw) }
}
