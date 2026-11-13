// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Runtime DLL publishing into `.workspace/runtime/`. The director injects the runtime from there into every game
//! server (RD9 as of 2026-11-13), so nothing is copied beside the server executable any more.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::HostError;
use crate::paths::{GAME_CHANGELIST, Paths, RUNTIME_FILE_NAMES};
use dainty_protocol::manifest::{LEGACY_RUNTIME_DLL, LOADER_DLL, RUNTIME_DLL};

/// What follows a DLL's name when it is moved aside because it was loaded; such files are swept on the next install.
pub const ASIDE_MARK: &str = ".rr-old-";
/// Flag files the server's game folder must hold for the runtime.
pub const SERVER_RUNTIME_FLAGS: [&str; 1] = ["CALL_HUNTPASS_QUERY.flag"];

/// One published runtime file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeFile {
    /// Its file name.
    pub name: String,
    /// Its size in bytes.
    pub size: u64,
    /// Its SHA-256, lowercase hex.
    pub sha256: String,
}

/// `.workspace/runtime/runtime.json`: what was last published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedRuntime {
    /// The publish's version: its date and a hash of its files.
    pub version: String,
    /// The game build it is for.
    pub changelist: u32,
    /// The files published.
    pub files: Vec<RuntimeFile>,
}

impl PublishedRuntime {
    /// The runtime.json text.
    pub fn to_json(&self) -> String {
        let files: Vec<Value> = self
            .files
            .iter()
            .map(|f| {
                json!({
                    "name": f.name,
                    "size": f.size,
                    "sha256": f.sha256,
                })
            })
            .collect();
        let value = json!({
            "version": self.version,
            "build": {
                "changelist": self.changelist,
            },
            "files": files,
        });
        let mut text = crate::hostconfig::stringify_pretty(&value);
        text.push('\n');
        text
    }
}

/// How a file was put in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replaced {
    /// Renamed over the old one.
    Replaced,
    /// The old one could not be replaced (it was loaded), so it was moved aside first.
    InUse,
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// `YYYYMMDD-` and the first 8 hex digits of the SHA-256 of the files' hashes.
pub fn runtime_version(now: DateTime<Utc>, hashes: &[&str]) -> String {
    let combined = hashes.join(":");
    let digest = sha256_bytes(combined.as_bytes());
    let short = &digest[..8];
    format!("{}-{short}", now.format("%Y%m%d"))
}

/// The file's SHA-256, lowercase hex.
pub fn sha256_file(file: &Path) -> Result<String, HostError> {
    let bytes = std::fs::read(file).map_err(|e| HostError::io(file, e))?;
    Ok(sha256_bytes(&bytes))
}

/// Renames `tmp` over `target`; a loaded `target` is moved aside first, and put back if the rename still fails.
pub fn replace_loaded_file(tmp: &Path, target: &Path) -> Result<Replaced, HostError> {
    match std::fs::rename(tmp, target) {
        Ok(()) => Ok(Replaced::Replaced),
        Err(first_err) => {
            if !target.exists() {
                return Err(HostError::io(target, first_err));
            }
            let millis = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let aside = PathBuf::from(format!("{}{ASIDE_MARK}{millis}", target.display()));
            std::fs::rename(target, &aside).map_err(|e| HostError::io(target, e))?;
            if let Err(e) = std::fs::rename(tmp, target) {
                let _ = std::fs::rename(&aside, target);
                return Err(HostError::io(target, e));
            }
            let _ = std::fs::remove_file(&aside);
            Ok(Replaced::InUse)
        }
    }
}

/// Where the build puts the runtime DLL and the loader.
pub fn default_sources(p: &Paths) -> [(&'static str, PathBuf); 2] {
    [(RUNTIME_DLL, p.runtime_dll.clone()), (LOADER_DLL, p.loader_dll.clone())]
}

/// Copies the runtime files from `sources` into `.workspace/runtime` and writes runtime.json; fails when the runtime
/// is not built. The loader is optional: only Proton players who load the runtime through it need it in the feed.
pub fn publish_runtime(
    p: &Paths,
    now: DateTime<Utc>,
    sources: &[(&str, PathBuf)],
) -> Result<PublishedRuntime, HostError> {
    for (name, src) in sources {
        if *name != LOADER_DLL && !src.is_file() {
            return Err(HostError::message(format!(
                "{name} is not built yet ({})",
                src.display()
            )));
        }
    }
    let sources: Vec<&(&str, PathBuf)> = sources.iter().filter(|(_, src)| src.is_file()).collect();

    std::fs::create_dir_all(&p.runtime_publish).map_err(|e| HostError::io(&p.runtime_publish, e))?;

    let mut files = Vec::with_capacity(sources.len());
    for (name, src) in sources {
        let dest = p.runtime_publish.join(name);
        let tmp = p.runtime_publish.join(format!("{name}.tmp"));
        if src != &dest {
            std::fs::copy(src, &tmp).map_err(|e| HostError::io(&tmp, e))?;
            let _ = replace_loaded_file(&tmp, &dest)?;
        }
        let meta = std::fs::metadata(&dest).map_err(|e| HostError::io(&dest, e))?;
        let sha256 = sha256_file(&dest)?;
        files.push(RuntimeFile {
            name: (*name).to_owned(),
            size: meta.len(),
            sha256,
        });
    }

    let hash_refs: Vec<&str> = files.iter().map(|f| f.sha256.as_str()).collect();
    let version = runtime_version(now, &hash_refs);
    let published = PublishedRuntime {
        version,
        changelist: GAME_CHANGELIST,
        files,
    };

    let manifest_tmp = p.runtime_publish.join("runtime.json.tmp");
    let manifest_dest = p.runtime_publish.join("runtime.json");
    std::fs::write(&manifest_tmp, published.to_json()).map_err(|e| HostError::io(&manifest_tmp, e))?;
    let _ = replace_loaded_file(&manifest_tmp, &manifest_dest)?;
    // The runtime under its name from before the Rustic Isles rename, published by an older kit (DX4)
    let _ = std::fs::remove_file(p.runtime_publish.join(LEGACY_RUNTIME_DLL));
    // A loader published before and not built now would be served with no line in runtime.json
    if !published.files.iter().any(|f| f.name == LOADER_DLL) {
        let _ = std::fs::remove_file(p.runtime_publish.join(LOADER_DLL));
    }

    Ok(published)
}

fn sweep_aside_files(win64_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(win64_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if RUNTIME_FILE_NAMES
            .iter()
            .any(|dll| name_str.starts_with(&format!("{dll}{ASIDE_MARK}")))
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Retires what an older kit installed beside the server's game executable in `win64_dir`: its moved-aside files,
/// and its `winmm.dll` when that is the published loader byte for byte. The director injects the runtime now, and a
/// loader there would load a second one into every server and refuse the launcher's PLAY from the same folder. A
/// `winmm.dll` someone else put there is left, and the director names it at start. True when a loader was removed.
pub fn retire_server_loader(p: &Paths, win64_dir: &Path) -> bool {
    sweep_aside_files(win64_dir);
    let installed = win64_dir.join(LOADER_DLL);
    let published = p.runtime_publish.join(LOADER_DLL);
    let ours =
        installed.is_file() && matches!((sha256_file(&installed), sha256_file(&published)), (Ok(a), Ok(b)) if a == b);
    ours && std::fs::remove_file(&installed).is_ok()
}

/// Creates the flag files the game folder lacks; returns the ones made.
pub fn ensure_server_flags(win64_dir: &Path) -> Result<Vec<String>, HostError> {
    let mut created = Vec::new();
    for flag in SERVER_RUNTIME_FLAGS {
        let file = win64_dir.join(flag);
        if !file.exists() {
            std::fs::write(&file, "").map_err(|e| HostError::io(&file, e))?;
            created.push(flag.to_owned());
        }
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use chrono::TimeZone;

    use super::*;

    #[test]
    fn publishes_runtime_retires_the_server_loader_and_writes_flags() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        std::fs::create_dir_all(paths.runtime_dll.parent().unwrap()).unwrap();
        std::fs::create_dir_all(paths.loader_dll.parent().unwrap()).unwrap();
        std::fs::write(&paths.runtime_dll, b"runtime-bytes").unwrap();
        std::fs::write(&paths.loader_dll, b"loader-bytes").unwrap();

        let now = Utc.with_ymd_and_hms(2026, 4, 17, 12, 0, 0).unwrap();
        let sources = default_sources(&paths);
        std::fs::create_dir_all(&paths.runtime_publish).unwrap();
        std::fs::write(paths.runtime_publish.join("MysticParadox.dll"), b"an older kit's").unwrap();
        let published = publish_runtime(&paths, now, &sources).unwrap();
        assert!(!paths.runtime_publish.join("MysticParadox.dll").exists());
        assert!(published.version.starts_with("20260417-"));
        assert_eq!(published.changelist, GAME_CHANGELIST);
        assert_eq!(published.files.len(), 2);
        assert!(paths.runtime_publish.join("runtime.json").is_file());

        let win64 = tmp.path().join("Win64");
        std::fs::create_dir_all(&win64).unwrap();
        std::fs::write(win64.join("Rustic-Isles.dll.rr-old-123"), b"stale").unwrap();

        std::fs::write(win64.join("winmm.dll"), b"loader-bytes").unwrap();
        assert!(retire_server_loader(&paths, &win64));
        assert!(!win64.join("Rustic-Isles.dll.rr-old-123").exists());
        assert!(!win64.join("winmm.dll").exists());
        assert!(!win64.join("Rustic-Isles.dll").exists());
        std::fs::write(win64.join("winmm.dll"), b"someone else's").unwrap();
        assert!(!retire_server_loader(&paths, &win64));
        assert!(win64.join("winmm.dll").is_file());

        let flags = ensure_server_flags(&win64).unwrap();
        assert_eq!(flags, vec!["CALL_HUNTPASS_QUERY.flag".to_owned()]);
        assert!(ensure_server_flags(&win64).unwrap().is_empty());
    }
}

#[cfg(test)]
mod loader_tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn the_loader_is_optional_and_the_runtime_is_not() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        assert!(publish_runtime(&paths, Utc::now(), &default_sources(&paths)).is_err());
        std::fs::create_dir_all(paths.runtime_dll.parent().unwrap()).unwrap();
        std::fs::write(&paths.runtime_dll, b"runtime-bytes").unwrap();
        std::fs::create_dir_all(&paths.runtime_publish).unwrap();
        std::fs::write(paths.runtime_publish.join(LOADER_DLL), b"an older publish").unwrap();
        let published = publish_runtime(&paths, Utc::now(), &default_sources(&paths)).unwrap();
        let names: Vec<&str> = published.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, [RUNTIME_DLL]);
        assert!(!paths.runtime_publish.join(LOADER_DLL).exists());
    }
}
