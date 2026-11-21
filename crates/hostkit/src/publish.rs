// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Runtime DLL publishing into `.workspace/runtime/` and installation beside the server executable.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::HostError;
use crate::paths::{GAME_CHANGELIST, Paths, RUNTIME_FILE_NAMES};
use protocol::manifest::{LEGACY_RUNTIME_DLL, RUNTIME_DLL};

pub const ASIDE_MARK: &str = ".rr-old-";
pub const SERVER_RUNTIME_FLAGS: [&str; 1] = ["CALL_HUNTPASS_QUERY.flag"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedRuntime {
    pub version: String,
    pub changelist: u32,
    pub files: Vec<RuntimeFile>,
}

impl PublishedRuntime {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replaced {
    Replaced,
    InUse,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstalledRuntime {
    pub installed: Vec<String>,
    pub unchanged: Vec<String>,
    pub in_use: Vec<String>,
    pub failed: Vec<(String, String)>,
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

pub fn runtime_version(now: DateTime<Utc>, hashes: &[&str]) -> String {
    let combined = hashes.join(":");
    let digest = sha256_bytes(combined.as_bytes());
    let short = &digest[..8];
    format!("{}-{short}", now.format("%Y%m%d"))
}

pub fn sha256_file(file: &Path) -> Result<String, HostError> {
    let bytes = std::fs::read(file).map_err(|e| HostError::io(file, e))?;
    Ok(sha256_bytes(&bytes))
}

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

pub fn default_sources(p: &Paths) -> [(&'static str, PathBuf); 2] {
    [
        (RUNTIME_DLL, p.runtime_dll.clone()),
        ("winmm.dll", p.loader_dll.clone()),
    ]
}

pub fn publish_runtime(
    p: &Paths,
    now: DateTime<Utc>,
    sources: &[(&str, PathBuf)],
) -> Result<PublishedRuntime, HostError> {
    for (name, src) in sources {
        if !src.is_file() {
            return Err(HostError::message(format!(
                "{name} is not built yet ({})",
                src.display()
            )));
        }
    }

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

pub fn install_server_runtime(p: &Paths, win64_dir: &Path) -> InstalledRuntime {
    let mut out = InstalledRuntime::default();
    sweep_aside_files(win64_dir);

    for name in RUNTIME_FILE_NAMES {
        let src = p.runtime_publish.join(name);
        if !src.is_file() {
            out.failed.push((
                name.to_owned(),
                format!("{name} is not in {}", p.runtime_publish.display()),
            ));
            continue;
        }
        let target = win64_dir.join(name);
        if target.is_file()
            && let (Ok(src_sha), Ok(dst_sha)) = (sha256_file(&src), sha256_file(&target))
            && src_sha == dst_sha
        {
            out.unchanged.push(name.to_owned());
            continue;
        }
        let tmp = win64_dir.join(format!("{name}.tmp"));
        if let Err(e) = std::fs::copy(&src, &tmp) {
            out.failed.push((name.to_owned(), e.to_string()));
            continue;
        }
        if name == RUNTIME_DLL {
            // An older kit's copy under the old name would be loaded instead if it were ever the newer file (DX4)
            let _ = std::fs::remove_file(win64_dir.join(LEGACY_RUNTIME_DLL));
        }
        match replace_loaded_file(&tmp, &target) {
            Ok(Replaced::Replaced) => {
                out.installed.push(name.to_owned());
            }
            Ok(Replaced::InUse) => {
                out.installed.push(name.to_owned());
                out.in_use.push(name.to_owned());
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                out.failed.push((name.to_owned(), e.to_string()));
            }
        }
    }

    out
}

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
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use chrono::TimeZone;

    use super::*;

    #[test]
    fn publishes_runtime_installs_into_win64_and_writes_flags() {
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
        std::fs::write(win64.join("MysticParadox.dll"), b"an older kit's").unwrap();

        let installed = install_server_runtime(&paths, &win64);
        assert_eq!(
            installed.installed,
            vec!["Rustic-Isles.dll".to_owned(), "winmm.dll".to_owned()]
        );
        assert!(installed.failed.is_empty());
        assert!(!win64.join("Rustic-Isles.dll.rr-old-123").exists());
        assert!(!win64.join("MysticParadox.dll").exists());

        let installed_again = install_server_runtime(&paths, &win64);
        assert_eq!(
            installed_again.unchanged,
            vec!["Rustic-Isles.dll".to_owned(), "winmm.dll".to_owned()]
        );
        assert!(installed_again.installed.is_empty());

        let flags = ensure_server_flags(&win64).unwrap();
        assert_eq!(flags, vec!["CALL_HUNTPASS_QUERY.flag".to_owned()]);
        assert!(ensure_server_flags(&win64).unwrap().is_empty());
    }
}
