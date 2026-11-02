// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The game runtime the host published to `CONTENT_RUNTIME_DIR`: `runtime.json` and the files it lists, each
//! checked against its size and SHA-256 before anything is offered.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::UNIX_EPOCH;

use protocol::manifest::{LEGACY_RUNTIME_DLL, RUNTIME_FILE_NAMES, RuntimeFile, RuntimeManifest, runtime_file_slot};
use serde_json::json;
use sha2::Digest as _;

use crate::input::parse_runtime_json;
use crate::{Clock, log, sha256_hex};

pub const MAX_INDEX_BYTES: u64 = 64 * 1024;
pub const RECHECK_MS: i64 = 10_000;
pub const READ_CHUNK: usize = 256 * 1024;
const INDEX: &str = "runtime.json";

/// A runtime that checked out: the manifest, the body players are sent, and its ETag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub manifest: RuntimeManifest,
    pub body: String,
    pub etag: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Nothing published (404 `no_runtime`)
    None(String),
    /// Published, but it does not check out (503 `runtime_invalid`)
    Invalid(String),
    Ok(Arc<Published>),
}

impl Status {
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::None(reason) | Self::Invalid(reason) => Some(reason),
            Self::Ok(_) => None,
        }
    }

    /// The listed entry for `requested`'s slot, so the pre-DX4 name and the new one find the same file.
    pub fn listed(&self, requested: &str) -> Option<&RuntimeFile> {
        let Self::Ok(published) = self else {
            return None;
        };
        let slot = runtime_file_slot(requested)?;
        published
            .manifest
            .files
            .iter()
            .find(|file| runtime_file_slot(&file.name) == Some(slot))
    }
}

/// Existence, length and modification time of the files a publish touches.
type Signature = Vec<Option<(u64, u128)>>;

struct State {
    status: Status,
    checked_at: Option<i64>,
    signature: Option<Signature>,
}

/// The server's own runtime: checked at most every [`RECHECK_MS`], and reloaded only when a file changed.
pub struct Store {
    dir: Option<PathBuf>,
    clock: Clock,
    state: Mutex<State>,
}

impl Store {
    pub fn new(dir: Option<PathBuf>, clock: Clock) -> Self {
        Self {
            dir,
            clock,
            state: Mutex::new(State {
                status: Status::None("not checked yet".to_owned()),
                checked_at: None,
                signature: None,
            }),
        }
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The current status, checking the folder again when it is due. Blocking: it may hash the files.
    pub fn status(&self) -> Status {
        let Some(dir) = &self.dir else {
            return Status::None("CONTENT_RUNTIME_DIR is not set".to_owned());
        };
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let now = (self.clock)();
        if state.checked_at.is_some_and(|at| now - at < RECHECK_MS) {
            return state.status.clone();
        }
        state.checked_at = Some(now);
        let signature = signature(dir);
        if state.signature.as_ref() == Some(&signature) {
            return state.status.clone();
        }
        state.signature = Some(signature);
        let status = load(dir);
        report(dir, &state.status, &status);
        state.status = status.clone();
        status
    }

    /// Makes the next request load the folder again (a served file failed to open or changed).
    pub fn forget(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.checked_at = None;
        state.signature = None;
    }

    /// The runtime of a version: the first of `folders` that is a directory under the runtime folder, checked
    /// now (no cache, no log); none when there is no such folder.
    pub fn version_status(&self, folders: &[String]) -> Option<Status> {
        let dir = self.dir.as_ref()?;
        folders
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_dir())
            .map(|path| load(&path))
    }

    /// [`Self::version_status`], else the server's own.
    pub fn status_for(&self, folders: Option<&[String]>) -> (Status, Option<PathBuf>) {
        if let Some(folders) = folders
            && let Some(dir) = &self.dir
            && let Some(path) = folders.iter().map(|name| dir.join(name)).find(|path| path.is_dir())
        {
            let status = load(&path);
            return (status, Some(path));
        }
        (self.status(), self.dir.clone())
    }
}

fn report(dir: &Path, before: &Status, after: &Status) {
    match (before, after) {
        (Status::Ok(old), Status::Ok(new)) if old.manifest.version == new.manifest.version => {}
        (_, Status::Ok(new)) => {
            let files: Vec<_> = new
                .manifest
                .files
                .iter()
                .map(|f| json!({"name": f.name, "size": f.size, "sha256": f.sha256}))
                .collect();
            log::info(
                "runtime ready",
                json!({"version": new.manifest.version, "files": files}),
            );
        }
        (old, new) => {
            if old.reason() == new.reason() && !matches!(old, Status::Ok(_)) {
                return;
            }
            let fields = json!({"dir": dir.display().to_string(), "reason": new.reason()});
            if matches!(new, Status::Invalid(_)) {
                log::warn("no runtime is offered to players", fields);
            } else {
                log::info("no runtime is offered to players", fields);
            }
        }
    }
}

fn signature(dir: &Path) -> Signature {
    [INDEX, RUNTIME_FILE_NAMES[0], RUNTIME_FILE_NAMES[1], LEGACY_RUNTIME_DLL]
        .iter()
        .map(|name| {
            let meta = fs::metadata(dir.join(name)).ok()?;
            let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
            Some((meta.len(), modified))
        })
        .collect()
}

/// Checks a runtime folder: `runtime.json`, then every listed file's type, size and SHA-256.
pub fn load(dir: &Path) -> Status {
    let index = dir.join(INDEX);
    let too_big = || Status::Invalid("runtime.json is not a regular file under 64 KB".to_owned());
    let missing = || Status::None("no runtime.json yet (the host has not published a runtime)".to_owned());
    match fs::metadata(&index) {
        Err(_) => return missing(),
        Ok(meta) if !meta.is_file() || meta.len() > MAX_INDEX_BYTES => return too_big(),
        Ok(_) => {}
    }
    let Ok(bytes) = fs::read(&index) else {
        return missing();
    };
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return too_big();
    }
    let manifest = match parse_runtime_json(&bytes) {
        Ok(manifest) => manifest,
        Err(error) => return Status::Invalid(error),
    };
    for file in &manifest.files {
        if let Err(reason) = check_file(dir, file) {
            return Status::Invalid(reason);
        }
    }
    let body = manifest.to_json();
    let etag = format!("\"{}\"", sha256_hex(body.as_bytes()));
    Status::Ok(Arc::new(Published { manifest, body, etag }))
}

fn check_file(dir: &Path, file: &RuntimeFile) -> Result<(), String> {
    let name = &file.name;
    let path = dir.join(name);
    // Not followed: a link in the runtime folder is not a file the host published
    let meta = fs::symlink_metadata(&path).map_err(|_| format!("{name} is missing"))?;
    if !meta.file_type().is_file() {
        return Err(format!("{name} is not a regular file"));
    }
    if meta.len() != file.size {
        return Err(format!(
            "{name} is {} bytes, runtime.json says {}",
            meta.len(),
            file.size
        ));
    }
    let hash = hash_file(&path).map_err(|_| format!("{name} is missing"))?;
    if hash != file.sha256 {
        return Err(format!("{name} does not match its sha256 in runtime.json"));
    }
    Ok(())
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; READ_CHUNK];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(crate::hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;

    fn publish(dir: &Path, files: &[(&str, &[u8])]) {
        let listed: Vec<_> = files
            .iter()
            .map(|(name, bytes)| json!({"name": name, "size": bytes.len(), "sha256": sha256_hex(bytes)}))
            .collect();
        for (name, bytes) in files {
            fs::write(dir.join(name), bytes).unwrap();
        }
        fs::write(dir.join(INDEX), json!({"version": "v1", "files": listed}).to_string()).unwrap();
    }

    fn store(dir: &Path) -> (Store, Arc<AtomicI64>) {
        let now = Arc::new(AtomicI64::new(1_000_000));
        let clock = now.clone();
        (
            Store::new(Some(dir.to_owned()), Arc::new(move || clock.load(Ordering::SeqCst))),
            now,
        )
    }

    #[test]
    fn t5_none_then_ok_then_invalid_after_the_recheck() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            Store::new(None, Arc::new(|| 0)).status(),
            Status::None("CONTENT_RUNTIME_DIR is not set".into())
        );
        let (store, now) = store(tmp.path());
        assert!(matches!(store.status(), Status::None(_)));
        publish(tmp.path(), &[("Rustic-Isles.dll", b"dll"), ("winmm.dll", b"winmm")]);
        // Within 10 s the cached status stands
        now.fetch_add(9_999, Ordering::SeqCst);
        assert!(matches!(store.status(), Status::None(_)));
        store.forget();
        let status = store.status();
        assert!(matches!(status, Status::Ok(_)));
        assert_eq!(status.listed("winmm.dll").unwrap().name, "winmm.dll");
        assert!(status.listed("WINMM.DLL").is_none() && status.listed("runtime.json").is_none());
        fs::write(tmp.path().join("winmm.dll"), b"WINMM").unwrap();
        now.fetch_add(10_000, Ordering::SeqCst);
        let Status::Invalid(reason) = store.status() else {
            panic!("a wrong hash checked out")
        };
        assert!(reason.contains("winmm.dll"), "{reason}");
        // A new publish is picked up after 10 s
        publish(tmp.path(), &[("Rustic-Isles.dll", b"dll2"), ("winmm.dll", b"winmm2")]);
        now.fetch_add(10_000, Ordering::SeqCst);
        assert!(matches!(store.status(), Status::Ok(_)));
    }

    #[test]
    fn t6_the_pre_dx4_name_serves_both_names() {
        let tmp = tempfile::tempdir().unwrap();
        publish(tmp.path(), &[("MysticParadox.dll", b"dll"), ("winmm.dll", b"w")]);
        let status = load(tmp.path());
        let Status::Ok(published) = &status else {
            panic!("{status:?}")
        };
        assert_eq!(published.manifest.files[0].name, "MysticParadox.dll");
        assert_eq!(status.listed("Rustic-Isles.dll").unwrap().name, "MysticParadox.dll");
        assert_eq!(status.listed("MysticParadox.dll").unwrap().name, "MysticParadox.dll");
        assert!(status.listed("mysticparadox.dll").is_none());
    }

    #[test]
    fn t7_index_size_and_folders() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(INDEX), vec![b' '; 64 * 1024 + 1]).unwrap();
        assert_eq!(
            load(tmp.path()),
            Status::Invalid("runtime.json is not a regular file under 64 KB".into())
        );
        publish(tmp.path(), &[("Rustic-Isles.dll", b"dll"), ("winmm.dll", b"w")]);
        fs::remove_file(tmp.path().join("winmm.dll")).unwrap();
        assert_eq!(load(tmp.path()), Status::Invalid("winmm.dll is missing".into()));
        fs::create_dir(tmp.path().join("winmm.dll")).unwrap();
        assert_eq!(
            load(tmp.path()),
            Status::Invalid("winmm.dll is not a regular file".into())
        );
        fs::remove_dir(tmp.path().join("winmm.dll")).unwrap();
        fs::write(tmp.path().join("winmm.dll"), b"ww").unwrap();
        assert_eq!(
            load(tmp.path()),
            Status::Invalid("winmm.dll is 2 bytes, runtime.json says 1".into())
        );
        fs::write(tmp.path().join("winmm.dll"), b"x").unwrap();
        assert_eq!(
            load(tmp.path()),
            Status::Invalid("winmm.dll does not match its sha256 in runtime.json".into())
        );
    }

    #[test]
    fn versioned_folders_are_checked_now() {
        let tmp = tempfile::tempdir().unwrap();
        let (store, _) = store(tmp.path());
        assert!(store.version_status(&["revived".into()]).is_none());
        fs::create_dir(tmp.path().join("revived")).unwrap();
        publish(
            &tmp.path().join("revived"),
            &[("Rustic-Isles.dll", b"a"), ("winmm.dll", b"b")],
        );
        assert!(matches!(
            store.version_status(&[".x".into(), "1.4.4".into(), "revived".into()]),
            Some(Status::Ok(_))
        ));
    }
}
