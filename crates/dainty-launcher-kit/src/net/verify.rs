// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Hashing game files against the pinned executable, with a cache keyed by size and mtime.

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use crate::core::pins::lower_hex;

pub mod cache_file;

pub use cache_file::{CacheEntry, MAX_ENTRIES, parse_cache_file, write_cache_file};

/// SHA-256 of the 1.14.7 shipping executable the launcher accepts.
pub const EXE_PIN: &str = crate::core::pins::PINNED_EXE_SHA256;

const HASH_CHUNK_BYTES: usize = 1024 * 1024;

/// Why hashing stopped.
#[derive(Debug)]
pub enum VerifyError {
    /// The caller aborted.
    Aborted,
    /// Reading the file failed.
    Io(std::io::Error),
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Aborted => write!(f, "verification aborted"),
            Self::Io(err) => write!(f, "I/O error: {err}"),
        }
    }
}

impl std::error::Error for VerifyError {}

impl From<std::io::Error> for VerifyError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// A flag that stops a running hash at its next chunk; clones share it.
#[derive(Debug, Clone, Default)]
pub struct Abort(Arc<AtomicBool>);

impl Abort {
    /// A flag not yet raised.
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// Raises the flag.
    pub fn abort(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether the flag is raised.
    pub fn aborted(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// What the game executable turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExeCheck {
    /// The pinned 1.14.7 build.
    Pinned,
    /// Some other build.
    OtherBuild,
    /// Not there.
    Missing,
}

/// The file's modification time in milliseconds since the Unix epoch, 0 when unknown.
pub fn mtime_ms(metadata: &std::fs::Metadata) -> f64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// SHA-256 of `file` as lowercase hex, read on a blocking thread in 1 MiB chunks; `on_bytes` gets each chunk's size.
pub async fn hash_file(
    file: PathBuf,
    abort: Abort,
    on_bytes: Arc<dyn Fn(u64) + Send + Sync>,
) -> Result<String, VerifyError> {
    tokio::task::spawn_blocking(move || {
        if abort.aborted() {
            return Err(VerifyError::Aborted);
        }
        let mut f = std::fs::File::open(&file)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; HASH_CHUNK_BYTES];
        loop {
            if abort.aborted() {
                return Err(VerifyError::Aborted);
            }
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            on_bytes(n as u64);
        }
        Ok(lower_hex(&hasher.finalize()))
    })
    .await
    .map_err(|e| VerifyError::Io(std::io::Error::other(e.to_string())))?
}

/// File hashes kept by path, size and mtime, so an unchanged file is not read twice.
pub struct HashCache {
    path: PathBuf,
    entries: Vec<(String, CacheEntry)>,
    dirty: bool,
}

impl HashCache {
    /// The cache saved at `file`; empty when it is missing or unreadable.
    pub fn load(file: PathBuf) -> Self {
        let entries = std::fs::read_to_string(&file)
            .map(|raw| parse_cache_file(&raw))
            .unwrap_or_default();
        Self {
            path: file,
            entries,
            dirty: false,
        }
    }

    /// The cached hash of `file`, when its size and mtime still match.
    pub fn cached(&self, file: &Path) -> Option<String> {
        let meta = std::fs::metadata(file).ok()?;
        if !meta.is_file() {
            return None;
        }
        let size = meta.len();
        let mtime = mtime_ms(&meta);
        if mtime <= 0.0 {
            return None;
        }
        let key = file.to_string_lossy();
        self.entries
            .iter()
            .rev()
            .find(|(k, _)| k == key.as_ref())
            .and_then(|(_, entry)| {
                if entry.size == size && (entry.mtime_ms - mtime).abs() < 1.0 {
                    Some(entry.sha256.clone())
                } else {
                    None
                }
            })
    }

    /// The hash of `file`, from the cache or by reading it. Cached only when the file did not change while read.
    pub async fn hash(
        &mut self,
        file: &Path,
        abort: Abort,
        on_bytes: Arc<dyn Fn(u64) + Send + Sync>,
    ) -> Result<String, VerifyError> {
        if let Some(hit) = self.cached(file) {
            return Ok(hit);
        }

        let before = std::fs::metadata(file)?;
        let before_mtime = mtime_ms(&before);
        let digest = hash_file(file.to_path_buf(), abort, on_bytes).await?;

        if let Ok(after) = std::fs::metadata(file) {
            let after_mtime = mtime_ms(&after);
            if after.len() == before.len() && after_mtime > 0.0 && (after_mtime - before_mtime).abs() < 1.0 {
                let key = file.to_string_lossy().into_owned();
                self.entries.retain(|(k, _)| k != &key);
                self.entries.push((
                    key,
                    CacheEntry {
                        size: after.len(),
                        mtime_ms: after_mtime,
                        sha256: digest.clone(),
                    },
                ));
                if self.entries.len() > MAX_ENTRIES {
                    let drop_count = self.entries.len() - MAX_ENTRIES;
                    self.entries.drain(0..drop_count);
                }
                self.dirty = true;
            }
        }

        Ok(digest)
    }

    /// Writes the cache when anything was added, through a temporary file.
    pub async fn save(&mut self) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let path = self.path.clone();
        let content = write_cache_file(&self.entries);
        tokio::task::spawn_blocking(move || {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut tmp_os = path.as_os_str().to_os_string();
            tmp_os.push(".tmp");
            let tmp_path = PathBuf::from(tmp_os);
            std::fs::write(&tmp_path, content)?;
            if std::fs::rename(&tmp_path, &path).is_err() {
                let _ = std::fs::remove_file(&path);
                std::fs::rename(&tmp_path, &path)?;
            }
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))??;
        self.dirty = false;
        Ok(())
    }
}

/// The executable's state from the cache alone; none when it would need hashing.
pub fn cached_exe_check(cache: &HashCache, exe: &Path, expected_sha256: &str) -> Option<ExeCheck> {
    if !exe.is_file() {
        return Some(ExeCheck::Missing);
    }
    let sha256 = cache.cached(exe)?;
    if sha256.eq_ignore_ascii_case(expected_sha256) {
        Some(ExeCheck::Pinned)
    } else {
        Some(ExeCheck::OtherBuild)
    }
}

/// Hashes the executable (or reads the cache) and compares it with `expected_sha256`.
pub async fn check_game_exe(
    cache: &mut HashCache,
    exe: &Path,
    expected_sha256: &str,
    abort: Abort,
    on_bytes: Arc<dyn Fn(u64) + Send + Sync>,
) -> Result<ExeCheck, VerifyError> {
    if !exe.is_file() {
        return Ok(ExeCheck::Missing);
    }
    match cache.hash(exe, abort, on_bytes).await {
        Ok(sha256) => {
            if sha256.eq_ignore_ascii_case(expected_sha256) {
                Ok(ExeCheck::Pinned)
            } else {
                Ok(ExeCheck::OtherBuild)
            }
        }
        Err(VerifyError::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => Ok(ExeCheck::Missing),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn cache_round_trip_preserves_entries_evicts_past_cap_and_drops_malformed() {
        assert!(parse_cache_file("not json").is_empty());
        assert!(parse_cache_file(r#"{"files": "bad"}"#).is_empty());
        assert!(
            parse_cache_file(
                r#"{"files": {"": {"size": 1, "mtimeMs": 1.0, "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}"#
            )
            .is_empty()
        );

        let valid_sha = "a".repeat(64);
        let mut entries = Vec::new();
        for i in 0..40 {
            entries.push((
                format!("file_{i}.bin"),
                CacheEntry {
                    size: 100 + i as u64,
                    mtime_ms: 1_700_000_000_000.0 + i as f64,
                    sha256: valid_sha.clone(),
                },
            ));
        }
        let encoded = write_cache_file(&entries);
        let parsed = parse_cache_file(&encoded);
        assert_eq!(parsed.len(), MAX_ENTRIES);
        assert_eq!(parsed[0].0, "file_8.bin");
        assert_eq!(parsed[MAX_ENTRIES - 1].0, "file_39.bin");
    }

    #[tokio::test]
    async fn hash_cache_reuses_digest_without_rereading_and_invalidates_on_change() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sample.bin");
        let cache_path = dir.path().join("verify-cache.json");
        std::fs::write(&file, b"dauntless verification bytes").unwrap();

        let mut cache = HashCache::load(cache_path.clone());
        let seen1 = Arc::new(AtomicU64::new(0));
        let seen1_cb = Arc::clone(&seen1);
        let d1 = cache
            .hash(
                &file,
                Abort::new(),
                Arc::new(move |n| {
                    seen1_cb.fetch_add(n, Ordering::SeqCst);
                }),
            )
            .await
            .unwrap();
        cache.save().await.unwrap();
        assert_eq!(seen1.load(Ordering::SeqCst), 28);

        let mut reloaded = HashCache::load(cache_path);
        assert_eq!(reloaded.cached(&file).as_deref(), Some(d1.as_str()));

        let seen2 = Arc::new(AtomicU64::new(0));
        let seen2_cb = Arc::clone(&seen2);
        let d2 = reloaded
            .hash(
                &file,
                Abort::new(),
                Arc::new(move |n| {
                    seen2_cb.fetch_add(n, Ordering::SeqCst);
                }),
            )
            .await
            .unwrap();
        assert_eq!(d1, d2);
        assert_eq!(seen2.load(Ordering::SeqCst), 0);

        std::fs::write(&file, b"dauntless verification bytes updated").unwrap();
        assert_eq!(reloaded.cached(&file), None);
        let seen3 = Arc::new(AtomicU64::new(0));
        let seen3_cb = Arc::clone(&seen3);
        let d3 = reloaded
            .hash(
                &file,
                Abort::new(),
                Arc::new(move |n| {
                    seen3_cb.fetch_add(n, Ordering::SeqCst);
                }),
            )
            .await
            .unwrap();
        assert_ne!(d1, d3);
        assert_eq!(seen3.load(Ordering::SeqCst), 36);
    }

    #[tokio::test]
    async fn check_game_exe_classifies_missing_pinned_and_other_build() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("Dauntless-Win64-Shipping.exe");
        let cache_path = dir.path().join("verify-cache.json");
        let mut cache = HashCache::load(cache_path);

        assert_eq!(cached_exe_check(&cache, &exe, EXE_PIN), Some(ExeCheck::Missing));
        assert_eq!(
            check_game_exe(&mut cache, &exe, EXE_PIN, Abort::new(), Arc::new(|_| {}))
                .await
                .unwrap(),
            ExeCheck::Missing
        );

        std::fs::write(&exe, b"exe bytes").unwrap();
        assert_eq!(cached_exe_check(&cache, &exe, EXE_PIN), None);

        let actual = hash_file(exe.clone(), Abort::new(), Arc::new(|_| {})).await.unwrap();
        assert_eq!(
            check_game_exe(&mut cache, &exe, &actual, Abort::new(), Arc::new(|_| {}))
                .await
                .unwrap(),
            ExeCheck::Pinned
        );
        assert_eq!(cached_exe_check(&cache, &exe, &actual), Some(ExeCheck::Pinned));
        assert_eq!(cached_exe_check(&cache, &exe, EXE_PIN), Some(ExeCheck::OtherBuild));
        assert_eq!(
            check_game_exe(&mut cache, &exe, EXE_PIN, Abort::new(), Arc::new(|_| {}))
                .await
                .unwrap(),
            ExeCheck::OtherBuild
        );
    }

    #[tokio::test]
    async fn stops_hashing_when_aborted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("large.bin");
        std::fs::write(&file, vec![0x42u8; 2 * HASH_CHUNK_BYTES]).unwrap();

        let abort = Abort::new();
        let abort_in_cb = abort.clone();
        let res = hash_file(
            file,
            abort,
            Arc::new(move |_| {
                abort_in_cb.abort();
            }),
        )
        .await;
        assert!(matches!(res, Err(VerifyError::Aborted)));
    }
}
