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
            Ok(file) => Some(Mutex::new(file)),
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
