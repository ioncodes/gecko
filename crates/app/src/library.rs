use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

use futures::channel::mpsc;
use futures::stream::Stream;
use walkdir::WalkDir;

use crate::cache::{CacheEntry, FileFingerprint, LibraryCache};
use crate::game::{Format, Game, Platform};

#[derive(Debug, Clone)]
pub enum ScanProgress {
    Started { cached: Vec<Game>, pending: usize },
    Loaded(Box<Game>),
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

async fn run_scan(roots: Vec<PathBuf>, prior: LibraryCache, tx: mpsc::UnboundedSender<ScanProgress>) {
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
    let mut cached_games: Vec<Game> = Vec::new();
    let mut todo: Vec<(PathBuf, Format, FileFingerprint)> = Vec::new();

    for (path, format, fp) in enumerated {
        match prior.entries.get(&path) {
            Some(entry) if entry.fingerprint == fp => {
                let game = entry.game.clone();
                cache.entries.insert(
                    path.clone(),
                    CacheEntry {
                        fingerprint: fp,
                        game: game.clone(),
                    },
                );
                cached_games.push(game);
            }
            _ => todo.push((path, format, fp)),
        }
    }

    let _ = tx.unbounded_send(ScanProgress::Started {
        cached: cached_games,
        pending: todo.len(),
    });

    for (path, format, fp) in todo {
        let path_for_task = path.clone();
        let join = tokio::task::spawn_blocking(move || {
            tracing::info!(path = %path_for_task.display(), "scanning");
            self::load_one(&path_for_task, format).map(|game| (path_for_task, fp, game))
        })
        .await;

        let (path, fp, game) = match join {
            Ok(Ok(triple)) => triple,
            Ok(Err(err)) => {
                tracing::warn!(path = %path.display(), %err, "skip file");
                continue;
            }
            Err(err) => {
                tracing::warn!(?err, path = %path.display(), "scanner task panicked");
                continue;
            }
        };

        cache.entries.insert(
            path,
            CacheEntry {
                fingerprint: fp,
                game: game.clone(),
            },
        );
        if tx.unbounded_send(ScanProgress::Loaded(Box::new(game))).is_err() {
            return;
        }
    }

    let _ = tx.unbounded_send(ScanProgress::Finished(Box::new(cache)));
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

pub fn load_one(path: &Path, format: Format) -> Result<Game, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let dvd = image::load_dvd(data);
        Game::from_dvd(path, dvd.as_ref(), format)
    }));
    result.map_err(|_| "image::load_dvd panicked".to_owned())
}

pub fn load_dol(path: &Path, platform: Platform) -> Result<Game, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(Game::from_dol(path, &data, platform))
}
