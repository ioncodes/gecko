use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::channel::mpsc;
use futures::stream::{self, Stream, StreamExt};
use walkdir::WalkDir;

use crate::cache::{CacheEntry, FileFingerprint, LibraryCache};
use crate::game::{Format, Game, Platform};

const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub enum ScanProgress {
    Started { cached: Vec<Game>, pending: usize },
    Loaded(Box<Game>),
    Checkpoint(Box<LibraryCache>),
    Finished(Box<LibraryCache>),
    Error(String),
}

pub fn scan_library_stream(
    roots: Vec<PathBuf>,
    prior: LibraryCache,
) -> impl Stream<Item = ScanProgress> + Send + 'static {
    let (tx, rx) = mpsc::unbounded();
    tokio::spawn(self::run_scan(roots, prior, tx));
    rx
}

async fn run_scan(roots: Vec<PathBuf>, mut prior: LibraryCache, tx: mpsc::UnboundedSender<ScanProgress>) {
    let enumerated = match tokio::task::spawn_blocking(move || self::enumerate_many(&roots)).await {
        Ok(Ok(v)) => v,
        Ok(Err(err)) => {
            let _ = tx.unbounded_send(ScanProgress::Error(err));
            return;
        }
        Err(err) => {
            let _ = tx.unbounded_send(ScanProgress::Error(err.to_string()));
            return;
        }
    };

    let mut cache = LibraryCache::default();
    let mut cached_games = Vec::new();
    let mut headers = Vec::new();
    let mut banners = Vec::new();

    for (path, format, fingerprint) in enumerated {
        match prior.entries.remove(&path) {
            Some(entry) if entry.fingerprint == fingerprint => {
                if !entry.banner_scanned {
                    banners.push((path.clone(), format, fingerprint));
                }
                cached_games.push(entry.game.clone());
                cache.entries.insert(path, entry);
            }
            _ => headers.push((path, format, fingerprint)),
        }
    }

    let started = ScanProgress::Started {
        cached: cached_games,
        pending: headers.len() + banners.len(),
    };
    if tx.unbounded_send(started).is_err() {
        return;
    }

    let mut reads = stream::iter(headers)
        .map(|(path, format, fingerprint)| {
            tokio::task::spawn_blocking(move || {
                let game = self::load_header(&path, format);
                (path, format, fingerprint, game)
            })
        })
        .buffer_unordered(4);

    while let Some(result) = reads.next().await {
        let (path, format, fingerprint, game) = match result {
            Ok((path, format, fingerprint, Ok(game))) => (path, format, fingerprint, game),
            Ok((path, _, _, Err(err))) => {
                tracing::warn!(path = %path.display(), %err, "skip file");
                continue;
            }
            Err(err) => {
                tracing::warn!(?err, "header reader panicked");
                continue;
            }
        };

        let entry = CacheEntry {
            fingerprint,
            game: game.clone(),
            banner_scanned: false,
        };
        cache.entries.insert(path.clone(), entry);
        banners.push((path, format, fingerprint));

        if tx.unbounded_send(ScanProgress::Loaded(Box::new(game))).is_err() {
            return;
        }
    }

    if !self::checkpoint(&cache, &tx).await {
        return;
    }

    let mut last_checkpoint = Instant::now();
    for (path, format, fingerprint) in banners {
        if tx.is_closed() {
            return;
        }

        let path_for_task = path.clone();
        match tokio::task::spawn_blocking(move || self::load_one(&path_for_task, format)).await {
            Ok(Ok(game)) => {
                let entry = CacheEntry {
                    fingerprint,
                    game: game.clone(),
                    banner_scanned: true,
                };
                cache.entries.insert(path, entry);

                if tx.unbounded_send(ScanProgress::Loaded(Box::new(game))).is_err() {
                    return;
                }
            }
            Ok(Err(err)) => tracing::warn!(path = %path.display(), %err, "banner read failed; retaining header"),
            Err(err) => tracing::warn!(path = %path.display(), ?err, "banner reader panicked; retaining header"),
        }

        if last_checkpoint.elapsed() >= CHECKPOINT_INTERVAL {
            if !self::checkpoint(&cache, &tx).await {
                return;
            }
            last_checkpoint = Instant::now();
        }
    }

    self::save_cache(&cache).await;
    let _ = tx.unbounded_send(ScanProgress::Finished(Box::new(cache)));
}

async fn checkpoint(cache: &LibraryCache, tx: &mpsc::UnboundedSender<ScanProgress>) -> bool {
    self::save_cache(cache).await;
    tx.unbounded_send(ScanProgress::Checkpoint(Box::new(cache.clone())))
        .is_ok()
}

async fn save_cache(cache: &LibraryCache) {
    let snapshot = cache.clone();
    match tokio::task::spawn_blocking(move || crate::cache::save(&crate::cache::cache_path(), &snapshot)).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => tracing::warn!(%err, "failed to persist library cache"),
        Err(err) => tracing::warn!(%err, "library cache writer panicked"),
    }
}

fn enumerate_many(roots: &[PathBuf]) -> Result<Vec<(PathBuf, Format, FileFingerprint)>, String> {
    use std::collections::HashSet;

    let mut canonical_roots: Vec<PathBuf> = Vec::with_capacity(roots.len());
    let mut seen_roots: HashSet<PathBuf> = HashSet::new();
    for root in roots {
        if !root.exists() {
            tracing::warn!(path = %root.display(), "library path does not exist; skipping");
            continue;
        }

        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        if seen_roots.insert(canonical.clone()) {
            canonical_roots.push(canonical);
        }
    }

    let mut out = Vec::new();
    for root in &canonical_roots {
        for entry in WalkDir::new(root).max_depth(2).into_iter().filter_map(Result::ok) {
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.into_path();
            let Some(format) = Format::from_path(&path) else {
                continue;
            };
            let Some(fp) = FileFingerprint::from_path(&path) else {
                continue;
            };
            out.push((path, format, fp));
        }
    }
    Ok(out)
}

pub fn load_header(path: &Path, format: Format) -> Result<Game, String> {
    let header = crate::library_disc::read_header(path).map_err(|e| e.to_string())?;
    Ok(Game::from_metadata(path, &header, None, format))
}

pub fn load_one(path: &Path, format: Format) -> Result<Game, String> {
    let (header, banner) = crate::library_disc::read_metadata(path).map_err(|e| e.to_string())?;
    Ok(Game::from_metadata(path, &header, banner, format))
}

pub fn load_dol(path: &Path, platform: Platform) -> Result<Game, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(Game::from_dol(path, &data, platform))
}
