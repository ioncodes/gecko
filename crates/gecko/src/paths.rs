use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static BASE: OnceLock<PathBuf> = OnceLock::new();

pub fn set_base(dir: impl Into<PathBuf>) {
    let _ = BASE.set(dir.into());
}

pub fn base() -> &'static Path {
    BASE.get_or_init(|| {
        if let Some(custom) = std::env::var_os("GECKO_DATA_DIR") {
            return PathBuf::from(custom);
        }

        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    })
}

pub fn resolve(rel: impl AsRef<Path>) -> PathBuf {
    self::base().join(rel)
}

pub fn cache(rel: impl AsRef<Path>) -> PathBuf {
    self::resolve(Path::new("cache").join(rel))
}
