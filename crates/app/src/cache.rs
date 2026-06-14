use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::config;
use crate::game::Game;

const CACHE_VERSION: u32 = 1;
const CACHE_DIR: &str = "cache";
const CACHE_FILE: &str = "library.bin";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFingerprint {
    pub size: u64,
    pub mtime_secs: u64,
}

impl FileFingerprint {
    pub fn from_path(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        let mtime_secs = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Some(Self {
            size: meta.len(),
            mtime_secs,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub fingerprint: FileFingerprint,
    pub game: Game,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryCache {
    pub version: u32,
    pub entries: HashMap<PathBuf, CacheEntry>,
}

impl Default for LibraryCache {
    fn default() -> Self {
        Self {
            version: CACHE_VERSION,
            entries: HashMap::new(),
        }
    }
}

pub fn cache_path() -> PathBuf {
    config::exe_relative(Path::new(CACHE_DIR).join(CACHE_FILE))
}

pub fn load(path: &Path) -> LibraryCache {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return LibraryCache::default(),
        Err(err) => {
            tracing::warn!(?err, path = %path.display(), "failed to read cache; starting fresh");
            return LibraryCache::default();
        }
    };

    match bincode::serde::decode_from_slice::<LibraryCache, _>(&bytes, bincode::config::standard()) {
        Ok((mut cache, _)) if cache.version == CACHE_VERSION => {
            for entry in cache.entries.values_mut() {
                entry.game.rehydrate_keys();
            }
            cache
        }
        Ok((cache, _)) => {
            tracing::info!(
                got = cache.version,
                want = CACHE_VERSION,
                "cache version mismatch; discarding"
            );
            LibraryCache::default()
        }
        Err(err) => {
            tracing::warn!(?err, "failed to decode cache; starting fresh");
            LibraryCache::default()
        }
    }
}

pub fn save(path: &Path, cache: &LibraryCache) -> anyhow::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let bytes = bincode::serde::encode_to_vec(cache, bincode::config::standard())?;
    std::fs::write(path, bytes)?;
    Ok(())
}
