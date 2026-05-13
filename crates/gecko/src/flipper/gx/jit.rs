#![cfg(feature = "jit")]

pub mod attr;
pub mod builder;
pub mod runtime;
#[cfg(feature = "vtx-jit-validate")]
pub mod validate;

use cranelift_codegen::ir::Signature;
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::{Context, ir};
use cranelift_frontend::FunctionBuilderContext;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, default_libcall_names};
use rustc_hash::FxHashMap;
use std::ffi::c_void;

use crate::flipper::gx::constants::*;
use crate::flipper::gx::regs::{VatA, VatB, VatC, VcdHi, VcdLo};
use crate::host::DrawVertex;
use crate::mmio::constants::MEM2_BASE;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct VtxKey {
    pub vcd_lo: u32,
    pub vcd_hi: u32,
    pub vat_a: u32,
    pub vat_b: u32,
    pub vat_c: u32,
}

impl VtxKey {
    pub fn from_cp_regs(cp_regs: &[u32], vat_index: usize) -> Self {
        Self {
            vcd_lo: cp_regs[VCD_LO_REG],
            vcd_hi: cp_regs[VCD_HI_REG],
            vat_a: cp_regs[VATA_REG + vat_index],
            vat_b: cp_regs[VATB_REG + vat_index],
            vat_c: cp_regs[VATC_REG + vat_index],
        }
    }

    pub fn vcd_lo(&self) -> VcdLo {
        VcdLo::from_raw(self.vcd_lo)
    }

    pub fn vcd_hi(&self) -> VcdHi {
        VcdHi::from_raw(self.vcd_hi)
    }

    pub fn vat_a(&self) -> VatA {
        VatA::from_raw(self.vat_a)
    }

    pub fn vat_b(&self) -> VatB {
        VatB::from_raw(self.vat_b)
    }

    pub fn vat_c(&self) -> VatC {
        VatC::from_raw(self.vat_c)
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct ResolvedArray {
    pub host_base: *const u8,
    pub stride: u32,
    pub _pad: u32,
}

impl ResolvedArray {
    pub const fn null() -> Self {
        Self {
            host_base: std::ptr::null(),
            stride: 0,
            _pad: 0,
        }
    }
}

unsafe impl Send for ResolvedArray {}

/// 12 array slots: pos, nrm, clr0, clr1, tex0..tex7.
pub const RESOLVED_ARRAY_COUNT: usize = 12;

#[repr(C)]
#[derive(Clone, Debug)]
pub struct ResolvedArrays(pub [ResolvedArray; RESOLVED_ARRAY_COUNT]);

impl Default for ResolvedArrays {
    fn default() -> Self {
        Self([ResolvedArray::null(); RESOLVED_ARRAY_COUNT])
    }
}

#[inline]
pub fn resolve_addr(mem1: &[u8], mem2: &[u8], addr: u32) -> *const u8 {
    let addr = addr as usize;

    if addr < mem1.len() {
        unsafe { mem1.as_ptr().add(addr) }
    } else if addr >= MEM2_BASE as usize {
        let off = addr - MEM2_BASE as usize;

        if off < mem2.len() {
            unsafe { mem2.as_ptr().add(off) }
        } else {
            std::ptr::null()
        }
    } else {
        std::ptr::null()
    }
}

/// JIT'd parser entry signature. The compiled function decodes
/// `vertex_count` vertices from `fifo_ptr` straight into `out_ptr`,
/// invoking the texgen helper per vertex via `gp_ptr`.
pub type ParserFn = unsafe extern "C" fn(
    gp_ptr: *mut c_void,
    xf_mem_ptr: *const u32,
    arrays_ptr: *const ResolvedArray,
    fifo_ptr: *const u8,
    out_ptr: *mut DrawVertex,
    vertex_count: u32,
);

/// Owns the cranelift JIT module + the (key -> parser fn) cache. One
/// instance per `GraphicsProcessor`. Lazily compiles per `VtxKey` and
/// keeps the result for the lifetime of the engine.
pub struct JitVertexEngine {
    module: JITModule,
    ctx: Context,
    fn_ctx: FunctionBuilderContext,
    parser_sig: Signature,
    cache: FxHashMap<VtxKey, CompiledParser>,
    seq: u64,
    /// Last (key, parser) tuple. Consecutive draws often share a VAT
    /// config, so this saves the FxHashMap hash + lookup.
    last_hit: Option<(VtxKey, ParserFn)>,
    failed: rustc_hash::FxHashSet<VtxKey>,
    pub stats: JitStats,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct JitStats {
    pub hits: u64,
    pub compiles: u64,
    pub fallbacks: u64,
}

struct CompiledParser {
    func: ParserFn,
    #[allow(dead_code)]
    func_id: FuncId,
}

impl JitVertexEngine {
    pub fn new() -> Self {
        let mut flag_builder = settings::builder();
        flag_builder.set("use_colocated_libcalls", "false").unwrap();
        flag_builder.set("is_pic", "false").unwrap();
        flag_builder.set("opt_level", "speed").unwrap();
        flag_builder.set("preserve_frame_pointers", "true").unwrap();
        let want_verifier = if cfg!(feature = "vtx-jit-validate") {
            "true"
        } else {
            "false"
        };
        flag_builder.set("enable_verifier", want_verifier).unwrap();
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

        jit_builder.symbol(
            runtime::SYM_APPLY_TEXGENS,
            runtime::gecko_gx_jit_apply_texgens as *const u8,
        );

        let module = JITModule::new(jit_builder);
        let ptr = module.target_config().pointer_type();
        let host_cc = module.target_config().default_call_conv;

        let parser_sig = self::parser_signature(ptr, host_cc);

        Self {
            module,
            ctx: Context::new(),
            fn_ctx: FunctionBuilderContext::new(),
            parser_sig,
            cache: FxHashMap::default(),
            last_hit: None,
            failed: rustc_hash::FxHashSet::default(),
            seq: 0,
            stats: JitStats::default(),
        }
    }

    pub fn cached_keys(&self) -> Vec<crate::jit_cache::CachedVtxKey> {
        self.cache
            .keys()
            .map(|k| crate::jit_cache::CachedVtxKey {
                vcd_lo: k.vcd_lo,
                vcd_hi: k.vcd_hi,
                vat_a: k.vat_a,
                vat_b: k.vat_b,
                vat_c: k.vat_c,
            })
            .collect()
    }

    pub fn precompile_keys(&mut self, keys: &[crate::jit_cache::CachedVtxKey]) -> (usize, usize) {
        let mut compiled = 0;
        let mut skipped = 0;
        for k in keys {
            let key = VtxKey {
                vcd_lo: k.vcd_lo,
                vcd_hi: k.vcd_hi,
                vat_a: k.vat_a,
                vat_b: k.vat_b,
                vat_c: k.vat_c,
            };
            if self.cache.contains_key(&key) {
                continue;
            }
            if self.lookup_or_compile(key).is_some() {
                compiled += 1;
            } else {
                skipped += 1;
            }
        }
        (compiled, skipped)
    }

    pub fn lookup_or_compile(&mut self, key: VtxKey) -> Option<ParserFn> {
        if let Some((last_key, last_fn)) = self.last_hit {
            if last_key == key {
                self.stats.hits += 1;
                return Some(last_fn);
            }
        }
        if let Some(c) = self.cache.get(&key) {
            self.stats.hits += 1;
            self.last_hit = Some((key, c.func));
            return Some(c.func);
        }
        if self.failed.contains(&key) {
            self.stats.fallbacks += 1;
            return None;
        }
        match self.compile(key) {
            Some(c) => {
                self.stats.compiles += 1;
                let func = c.func;
                self.cache.insert(key, c);
                self.last_hit = Some((key, func));
                Some(func)
            }
            None => {
                self.failed.insert(key);
                self.stats.fallbacks += 1;
                None
            }
        }
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure(label = "vtx_jit_compile"))]
    fn compile(&mut self, key: VtxKey) -> Option<CompiledParser> {
        self.seq = self.seq.wrapping_add(1);
        let name = format!("gecko_vtx_parser_{:016x}", self.seq);
        let func_id = self
            .module
            .declare_function(&name, Linkage::Local, &self.parser_sig)
            .expect("declare vtx parser fn");

        self.ctx.clear();
        self.ctx.func.signature = self.parser_sig.clone();

        let pointer_ty = self.module.target_config().pointer_type();
        if !builder::build_parser(&mut self.ctx, &mut self.fn_ctx, &mut self.module, pointer_ty, key) {
            // Codegen refused (unsupported feature). Drop the in-progress fn.
            return None;
        }

        self.module
            .define_function(func_id, &mut self.ctx)
            .expect("define vtx parser fn");
        self.module.finalize_definitions().expect("finalize vtx jit");

        let raw = self.module.get_finalized_function(func_id);
        let func: ParserFn = unsafe { std::mem::transmute(raw) };
        Some(CompiledParser { func, func_id })
    }
}

impl Default for JitVertexEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn parser_signature(ptr: ir::Type, cc: CallConv) -> Signature {
    Signature {
        params: vec![
            ir::AbiParam::new(ptr),            // gp_ptr
            ir::AbiParam::new(ptr),            // xf_mem_ptr
            ir::AbiParam::new(ptr),            // arrays_ptr
            ir::AbiParam::new(ptr),            // fifo_ptr
            ir::AbiParam::new(ptr),            // out_ptr
            ir::AbiParam::new(ir::types::I32), // vertex_count
        ],
        returns: vec![],
        call_conv: cc,
    }
}

/// Fill a `ResolvedArrays` block from current GP state + RAM banks.
/// Called once per draw before invoking the parser. Returns false (caller
/// must fall back to interpreter) when:
///
/// 1. an indexed attribute resolves to an unmapped base address, or
/// 2. the worst-case index times stride for an indexed attribute would
///    push the read past the end of the bank the base lives in. The
///    interpreter handles those misses with a zero-fill via
///    `RamView::slice`; the JIT does pure pointer math without bounds
///    checks, so an OOB attribute would dereference unmapped memory
///    and segfault. (See `vertex.rs::fetch` for the interpreter path.)
#[cfg_attr(feature = "hotpath", hotpath::measure(label = "vtx_resolve_arrays"))]
pub fn resolve_arrays_for_draw(
    cp_regs: &[u32],
    key: &VtxKey,
    mem1: &[u8],
    mem2: &[u8],
    out: &mut ResolvedArrays,
) -> bool {
    use crate::flipper::gx::regs::AttributeType;

    let vcd_lo = key.vcd_lo();
    let vcd_hi = key.vcd_hi();
    let vat_a = key.vat_a();
    let vat_b = key.vat_b();
    let vat_c = key.vat_c();

    let resolve = |array_idx: usize, attr: AttributeType| -> ResolvedArray {
        if matches!(attr, AttributeType::Index8 | AttributeType::Index16) {
            let base = cp_regs[ARRAY_BASE_REG + array_idx];
            let stride = cp_regs[ARRAY_STRIDE_REG + array_idx];
            ResolvedArray {
                host_base: self::resolve_addr(mem1, mem2, base),
                stride,
                _pad: 0,
            }
        } else {
            ResolvedArray::null()
        }
    };

    let attr_data_size: [usize; 12] = [
        vat_a.pos_data_size(),
        vat_a.nrm_data_size(),
        vat_a.clr0_data_size(),
        vat_a.clr1_data_size(),
        vat_a.tex0_data_size(),
        vat_b.tex1_data_size(),
        vat_b.tex2_data_size(),
        vat_b.tex3_data_size(),
        vat_b.tex4_data_size(),
        vat_c.tex5_data_size(),
        vat_c.tex6_data_size(),
        vat_c.tex7_data_size(),
    ];

    let attrs_in: [AttributeType; 12] = [
        vcd_lo.position(),
        vcd_lo.normal(),
        vcd_lo.color0(),
        vcd_lo.color1(),
        vcd_hi.tex0(),
        vcd_hi.tex1(),
        vcd_hi.tex2(),
        vcd_hi.tex3(),
        vcd_hi.tex4(),
        vcd_hi.tex5(),
        vcd_hi.tex6(),
        vcd_hi.tex7(),
    ];

    let array_idx_for_slot = [
        ARRAY_POS,
        ARRAY_NRM,
        ARRAY_CLR0,
        ARRAY_CLR1,
        ARRAY_TEX0,
        ARRAY_TEX0 + 1,
        ARRAY_TEX0 + 2,
        ARRAY_TEX0 + 3,
        ARRAY_TEX0 + 4,
        ARRAY_TEX0 + 5,
        ARRAY_TEX0 + 6,
        ARRAY_TEX0 + 7,
    ];

    for slot in 0..RESOLVED_ARRAY_COUNT {
        out.0[slot] = resolve(array_idx_for_slot[slot], attrs_in[slot]);
    }

    let m1_start = mem1.as_ptr() as usize;
    let m1_end = m1_start + mem1.len();
    let m2_start = if mem2.is_empty() { 0 } else { mem2.as_ptr() as usize };
    let m2_end = if mem2.is_empty() { 0 } else { m2_start + mem2.len() };
    let bank_remaining_from = |p: *const u8| -> usize {
        let p = p as usize;
        if p == 0 {
            0
        } else if p >= m1_start && p < m1_end {
            m1_end - p
        } else if !mem2.is_empty() && p >= m2_start && p < m2_end {
            m2_end - p
        } else {
            0
        }
    };

    for slot in 0..RESOLVED_ARRAY_COUNT {
        let attr = attrs_in[slot];
        if !matches!(attr, AttributeType::Index8 | AttributeType::Index16) {
            continue;
        }
        let r = out.0[slot];
        if r.host_base.is_null() {
            return false;
        }
        let max_idx: u64 = match attr {
            AttributeType::Index8 => 255,
            AttributeType::Index16 => 65535,
            _ => 0,
        };
        let max_byte_off = max_idx
            .saturating_mul(r.stride as u64)
            .saturating_add(attr_data_size[slot] as u64);
        let remaining = bank_remaining_from(r.host_base) as u64;
        if max_byte_off > remaining {
            return false;
        }
    }
    true
}
