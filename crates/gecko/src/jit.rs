#[cfg(feature = "jit")]
pub mod arena;
pub mod cache;

#[cfg(feature = "jit")]
pub(crate) fn register_jit_code(kind: &str, pc: u32, address: usize, size: usize) {
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};
    static MAP: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    let map = MAP.get_or_init(|| {
        let path = std::env::var_os("GECKO_JIT_MAP")?;

        match std::fs::File::create(path) {
            Ok(mut file) => {
                use crate::gekko::jit::abi;
                use crate::system::GC;

                let _ = writeln!(file,
                    "# gecko_ctx_offsets {{\"LUT\":{},\"RC\":{},\"DIRTY\":{},\"CYC\":{},\"DL\":{},\"CR\":{},\"XER\":{},\"GPR\":{},\"FPR_END\":{}}}",
                    abi::fastmem_lut_ptr_offset::<GC>(), abi::code_refcount_ptr_offset::<GC>(),
                    abi::jit_dirty_offset::<GC>(), abi::cycles_offset::<GC>(), abi::next_deadline_offset::<GC>(),
                    abi::cr_offset::<GC>(), abi::xer_offset::<GC>(), abi::gpr_base_offset::<GC>(),
                    abi::fpr_base_offset::<GC>() + 32 * 16);
                Some(Mutex::new(file))
            },
            Err(err) => {
                tracing::warn!(%err, "failed to create JIT symbol map");

                None
            }
        }
    });

    if let Some(map) = map {
        if let Ok(mut file) = map.lock() {
            let _ = writeln!(file, "{address:x} {size:x} {kind}_{pc:08x}");
        }
    }
}
