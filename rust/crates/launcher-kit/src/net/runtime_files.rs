// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

pub mod manifest;

#[cfg(test)]
mod tests;

pub use manifest::{MAX_SAFE_INTEGER, RUNTIME_MARKER_FILE, parse_runtime_manifest};
pub use protocol::manifest::{
    DEFAULT_CHANGELIST, MAX_RUNTIME_FILE_BYTES, RUNTIME_BUILD_NAME, RUNTIME_FILE_NAMES, RUNTIME_MANIFEST_PATH,
    RuntimeBuild, RuntimeFile, RuntimeManifest,
};

use crate::bridge::types::{RuntimeFileInfo, RuntimeState};
use crate::core::pins::{DISK_MARGIN_BYTES, sha256_hex};
use crate::net::http::to_file::{FileRequestError, FileRequestOptions, request_to_file};
use crate::net::http::{Endpoint, HttpError, RequestOptions, parse_json_body, request};
use crate::net::verify::{Abort, VerifyError, hash_file};
use crate::platform::system::free_bytes;

pub const RUNTIME_FILES_BASE_PATH: &str = "/content/v1/runtime/files";
pub const DOWNLOAD_SUFFIX: &str = ".rr-download";
pub const ASIDE_MARK: &str = ".rr-old-";

const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(10);
const MANIFEST_MAX_BYTES: usize = 64 * 1024;
const MAX_TRUSTED_SETS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeManifestResult {
    Ok(RuntimeManifest),
    /// Server returned HTTP 404 (no runtime offered).
    None,
    /// Non-200/404 status, invalid JSON, or failed manifest schema check.
    BadAnswer,
    /// TLS certificate fingerprint did not match the endpoint's pin.
    CertMismatch,
    /// Network timeout or connection failure.
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeErrorCode {
    KeyRejected,
    NotFound,
    Different,
    DownloadFailed,
    CertMismatch,
    DiskError,
    InUse,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    pub code: RuntimeErrorCode,
    pub file: Option<String>,
    pub message: String,
}

impl RuntimeError {
    pub fn new(code: RuntimeErrorCode, file: Option<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            file,
            message: message.into(),
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.file {
            Some(file) => write!(f, "{:?} ({file}): {}", self.code, self.message),
            None => write!(f, "{:?}: {}", self.code, self.message),
        }
    }
}

impl std::error::Error for RuntimeError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProgress {
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub files_done: usize,
    pub files_total: usize,
    pub current_file: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeInstallResult {
    pub replaced_in_use: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replaced {
    Replaced,
    InUse,
}

pub trait ReplaceOps {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn remove(&self, file: &Path) -> std::io::Result<()>;
}

pub struct DiskOps;

impl ReplaceOps for DiskOps {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }

    fn remove(&self, file: &Path) -> std::io::Result<()> {
        std::fs::remove_file(file)
    }
}

pub struct InstallRequest<'a> {
    pub endpoint: &'a Endpoint,
    pub headers: Vec<(&'static str, String)>,
    pub manifest: &'a RuntimeManifest,
    pub win64_dir: &'a Path,
    pub cancel: Option<CancellationToken>,
    pub on_progress: Option<&'a (dyn Fn(RuntimeProgress) + Send + Sync)>,
}

pub fn file_infos(files: &[RuntimeFile]) -> Vec<RuntimeFileInfo> {
    files
        .iter()
        .map(|f| RuntimeFileInfo {
            name: f.name.clone(),
            size: f.size,
            sha256: f.sha256.clone(),
        })
        .collect()
}

pub fn runtime_set_id(files: &[RuntimeFile]) -> String {
    let mut sorted = files.to_vec();
    sorted.sort_by(|a, b| a.name.encode_utf16().cmp(b.name.encode_utf16()));
    let lines: Vec<String> = sorted
        .into_iter()
        .map(|f| format!("{}:{}:{}", f.name, f.size, f.sha256.to_ascii_lowercase()))
        .collect();
    sha256_hex(lines.join("\n").as_bytes())
}

pub fn is_trusted(trusted: &[String], files: &[RuntimeFile]) -> bool {
    !files.is_empty() && trusted.contains(&runtime_set_id(files))
}

pub fn accept(trusted: &mut Vec<String>, files: &[RuntimeFile]) -> String {
    let id = runtime_set_id(files);
    trusted.retain(|existing| existing != &id);
    trusted.push(id.clone());
    if trusted.len() > MAX_TRUSTED_SETS {
        let excess = trusted.len() - MAX_TRUSTED_SETS;
        trusted.drain(0..excess);
    }
    id
}

pub async fn fetch_runtime_manifest(endpoint: &Endpoint) -> RuntimeManifestResult {
    let options = RequestOptions::get(MANIFEST_TIMEOUT, MANIFEST_MAX_BYTES);
    match request(endpoint, RUNTIME_MANIFEST_PATH, options).await {
        Ok(res) => match res.status {
            200 => match parse_json_body(&res).and_then(|v| parse_runtime_manifest(&v)) {
                Some(manifest) => RuntimeManifestResult::Ok(manifest),
                None => RuntimeManifestResult::BadAnswer,
            },
            404 => RuntimeManifestResult::None,
            _ => RuntimeManifestResult::BadAnswer,
        },
        Err(HttpError::Pin { .. }) => RuntimeManifestResult::CertMismatch,
        Err(HttpError::TooLarge) => RuntimeManifestResult::BadAnswer,
        Err(_) => RuntimeManifestResult::Unreachable,
    }
}

pub async fn installed_runtime(win64_dir: &Path) -> Vec<(&'static str, Option<String>)> {
    let mut out = Vec::with_capacity(RUNTIME_FILE_NAMES.len());
    for name in RUNTIME_FILE_NAMES {
        let path = win64_dir.join(name);
        let valid_meta = std::fs::metadata(&path)
            .map(|m| m.is_file() && m.len() > 0)
            .unwrap_or(false);
        if !valid_meta {
            out.push((name, None));
            continue;
        }
        let digest = hash_file(path, Abort::new(), Arc::new(|_| {})).await.ok();
        out.push((name, digest));
    }
    out
}

pub async fn is_complete_runtime_dir(dir: &Path) -> bool {
    installed_runtime(dir).await.iter().all(|(_, hash)| hash.is_some())
}

pub async fn check_runtime(
    win64_dir: &Path,
    offered: Option<&RuntimeManifest>,
    trusted: bool,
    local_dir: Option<&Path>,
) -> (RuntimeState, bool) {
    if let Some(dir) = local_dir {
        return (RuntimeState::Local, is_complete_runtime_dir(dir).await);
    }

    let installed = installed_runtime(win64_dir).await;
    let present = installed.iter().filter(|(_, hash)| hash.is_some()).count();

    match offered {
        None => {
            if present == RUNTIME_FILE_NAMES.len() {
                (RuntimeState::Ok, true)
            } else {
                (RuntimeState::Unavailable, false)
            }
        }
        Some(manifest) => {
            let matches = manifest.files.iter().all(|f| {
                installed
                    .iter()
                    .any(|(n, h)| *n == f.name && h.as_deref() == Some(f.sha256.as_str()))
            });
            if matches {
                (RuntimeState::Ok, true)
            } else if !trusted {
                (RuntimeState::Untrusted, false)
            } else if present == 0 {
                (RuntimeState::Missing, false)
            } else {
                (RuntimeState::Outdated, false)
            }
        }
    }
}

pub async fn unblock(file: &Path) {
    #[cfg(windows)]
    {
        let mut ads = file.as_os_str().to_os_string();
        ads.push(":Zone.Identifier");
        let _ = tokio::fs::remove_file(PathBuf::from(ads)).await;
    }
    #[cfg(not(windows))]
    {
        let _ = file;
    }
}

pub fn replace_file(tmp: &Path, target: &Path, ops: &dyn ReplaceOps) -> std::io::Result<Replaced> {
    if ops.rename(tmp, target).is_ok() {
        return Ok(Replaced::Replaced);
    }

    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut aside_os = target.as_os_str().to_os_string();
    aside_os.push(format!("{ASIDE_MARK}{millis}"));
    let aside = PathBuf::from(aside_os);

    ops.rename(target, &aside)?;

    if let Err(err) = ops.rename(tmp, target) {
        let _ = ops.rename(&aside, target);
        return Err(err);
    }

    let _ = ops.remove(&aside);
    Ok(Replaced::InUse)
}

pub fn sweep_aside(win64_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(win64_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name_str) = file_name.to_str() else {
            continue;
        };
        let should_remove = RUNTIME_FILE_NAMES.iter().any(|rt| {
            let download_name = format!("{rt}{DOWNLOAD_SUFFIX}");
            let aside_prefix = format!("{rt}{ASIDE_MARK}");
            name_str == download_name || name_str.starts_with(&aside_prefix)
        });
        if should_remove {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn now_epoch_millis_iso() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{millis}")
}

fn write_runtime_marker(win64_dir: &Path, version: &str, source: &str, files: &[RuntimeFile]) -> std::io::Result<()> {
    let mut files_map = Map::new();
    for f in files {
        files_map.insert(
            f.name.clone(),
            json!({
                "size": f.size,
                "sha256": f.sha256,
            }),
        );
    }
    let doc = json!({
        "version": version,
        "source": source,
        "installedAt": now_epoch_millis_iso(),
        "files": Value::Object(files_map),
    });
    let text = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_owned());
    std::fs::write(win64_dir.join(RUNTIME_MARKER_FILE), text)
}

pub async fn install_runtime(request: InstallRequest<'_>) -> Result<RuntimeInstallResult, RuntimeError> {
    let InstallRequest {
        endpoint,
        headers,
        manifest,
        win64_dir,
        cancel,
        on_progress,
    } = request;

    let is_dir = std::fs::metadata(win64_dir).map(|m| m.is_dir()).unwrap_or(false);
    if !is_dir {
        return Err(RuntimeError::new(
            RuntimeErrorCode::DiskError,
            None,
            "Game Win64 directory does not exist",
        ));
    }

    sweep_aside(win64_dir);

    let current = installed_runtime(win64_dir).await;
    let needed: Vec<&RuntimeFile> = manifest
        .files
        .iter()
        .filter(|f| {
            !current
                .iter()
                .any(|(name, hash)| *name == f.name && hash.as_deref() == Some(f.sha256.as_str()))
        })
        .collect();

    let total_bytes: u64 = needed.iter().map(|f| f.size).sum();
    let files_total = needed.len();

    if let Some(free) = free_bytes(win64_dir)
        && free < total_bytes.saturating_add(DISK_MARGIN_BYTES)
    {
        return Err(RuntimeError::new(
            RuntimeErrorCode::DiskError,
            None,
            "Not enough disk space",
        ));
    }

    if let Some(cb) = on_progress {
        cb(RuntimeProgress {
            done_bytes: 0,
            total_bytes,
            files_done: 0,
            files_total,
            current_file: None,
        });
    }

    let mut completed_bytes: u64 = 0;
    let mut replaced_in_use: Vec<String> = Vec::new();
    let ops = DiskOps;

    for (files_done, file) in needed.iter().enumerate() {
        if cancel.as_ref().is_some_and(CancellationToken::is_cancelled) {
            return Err(RuntimeError::new(
                RuntimeErrorCode::Cancelled,
                Some(file.name.clone()),
                "Cancelled",
            ));
        }

        if let Some(cb) = on_progress {
            cb(RuntimeProgress {
                done_bytes: completed_bytes,
                total_bytes,
                files_done,
                files_total,
                current_file: Some(file.name.clone()),
            });
        }

        let target = win64_dir.join(&file.name);
        let tmp = win64_dir.join(format!("{}{DOWNLOAD_SUFFIX}", file.name));
        let path_and_query = format!("{RUNTIME_FILES_BASE_PATH}/{}", file.name);

        let status = {
            let mut file_bytes: u64 = 0;
            let file_name = file.name.clone();
            let base_done = completed_bytes;
            let mut on_chunk = |n: u64| {
                file_bytes = file_bytes.saturating_add(n);
                if let Some(cb) = on_progress {
                    cb(RuntimeProgress {
                        done_bytes: base_done.saturating_add(file_bytes),
                        total_bytes,
                        files_done,
                        files_total,
                        current_file: Some(file_name.clone()),
                    });
                }
            };

            let opts = FileRequestOptions {
                headers: headers.clone(),
                idle_timeout: DOWNLOAD_IDLE_TIMEOUT,
                max_bytes: file.size,
                cancel: cancel.clone(),
                on_bytes: Some(&mut on_chunk),
            };

            match request_to_file(endpoint, &path_and_query, &tmp, opts).await {
                Ok(status) => status,
                Err(err) => {
                    let _ = std::fs::remove_file(&tmp);
                    let code = match err {
                        FileRequestError::Http(HttpError::Aborted) => RuntimeErrorCode::Cancelled,
                        FileRequestError::Http(HttpError::Pin { .. }) => RuntimeErrorCode::CertMismatch,
                        FileRequestError::Http(HttpError::TooLarge) => RuntimeErrorCode::Different,
                        FileRequestError::Disk(_) => RuntimeErrorCode::DiskError,
                        FileRequestError::Http(_) => RuntimeErrorCode::DownloadFailed,
                    };
                    return Err(RuntimeError::new(code, Some(file.name.clone()), err.to_string()));
                }
            }
        };

        if status != 200 {
            let _ = std::fs::remove_file(&tmp);
            let code = match status {
                401 | 403 => RuntimeErrorCode::KeyRejected,
                404 => RuntimeErrorCode::NotFound,
                503 => RuntimeErrorCode::Different,
                _ => RuntimeErrorCode::DownloadFailed,
            };
            return Err(RuntimeError::new(
                code,
                Some(file.name.clone()),
                format!("HTTP {status}"),
            ));
        }

        let abort = Abort::new();
        if cancel.as_ref().is_some_and(CancellationToken::is_cancelled) {
            abort.abort();
        }

        let actual_size = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
        let actual_hash = match hash_file(tmp.clone(), abort, Arc::new(|_| {})).await {
            Ok(h) => h,
            Err(VerifyError::Aborted) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(RuntimeError::new(
                    RuntimeErrorCode::Cancelled,
                    Some(file.name.clone()),
                    "Cancelled",
                ));
            }
            Err(VerifyError::Io(e)) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(RuntimeError::new(
                    RuntimeErrorCode::DiskError,
                    Some(file.name.clone()),
                    e.to_string(),
                ));
            }
        };

        if actual_size != file.size || !actual_hash.eq_ignore_ascii_case(&file.sha256) {
            let _ = std::fs::remove_file(&tmp);
            return Err(RuntimeError::new(
                RuntimeErrorCode::Different,
                Some(file.name.clone()),
                "Downloaded file digest or size did not match manifest",
            ));
        }

        unblock(&tmp).await;

        match replace_file(&tmp, &target, &ops) {
            Ok(Replaced::Replaced) => {}
            Ok(Replaced::InUse) => {
                replaced_in_use.push(file.name.clone());
            }
            Err(err) => {
                let _ = std::fs::remove_file(&tmp);
                let code = if err.kind() == std::io::ErrorKind::PermissionDenied {
                    RuntimeErrorCode::InUse
                } else {
                    RuntimeErrorCode::DiskError
                };
                return Err(RuntimeError::new(code, Some(file.name.clone()), err.to_string()));
            }
        }

        completed_bytes = completed_bytes.saturating_add(file.size);
        let done_count = files_done + 1;
        if let Some(cb) = on_progress {
            cb(RuntimeProgress {
                done_bytes: completed_bytes,
                total_bytes,
                files_done: done_count,
                files_total,
                current_file: Some(file.name.clone()),
            });
        }
    }

    let source = format!("{}:{}", endpoint.host, endpoint.port);
    let _ = write_runtime_marker(win64_dir, &manifest.version, &source, &manifest.files);

    Ok(RuntimeInstallResult { replaced_in_use })
}

pub async fn install_local_runtime(local_dir: &Path, win64_dir: &Path) -> Result<RuntimeInstallResult, RuntimeError> {
    let is_dir = std::fs::metadata(win64_dir).map(|m| m.is_dir()).unwrap_or(false);
    if !is_dir {
        return Err(RuntimeError::new(
            RuntimeErrorCode::DiskError,
            None,
            "Game Win64 directory does not exist",
        ));
    }

    sweep_aside(win64_dir);

    for name in RUNTIME_FILE_NAMES {
        let src = local_dir.join(name);
        let ok = std::fs::metadata(&src)
            .map(|m| m.is_file() && m.len() > 0)
            .unwrap_or(false);
        if !ok {
            return Err(RuntimeError::new(
                RuntimeErrorCode::NotFound,
                Some((*name).to_owned()),
                format!("Missing local runtime file {name}"),
            ));
        }
    }

    let mut replaced_in_use = Vec::new();
    let mut local_files = Vec::with_capacity(RUNTIME_FILE_NAMES.len());
    let ops = DiskOps;

    for name in RUNTIME_FILE_NAMES {
        let src = local_dir.join(name);
        let target = win64_dir.join(name);
        let tmp = win64_dir.join(format!("{name}{DOWNLOAD_SUFFIX}"));

        std::fs::copy(&src, &tmp)
            .map_err(|err| RuntimeError::new(RuntimeErrorCode::DiskError, Some((*name).to_owned()), err.to_string()))?;

        unblock(&tmp).await;

        let size = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
        let sha256 = hash_file(tmp.clone(), Abort::new(), Arc::new(|_| {}))
            .await
            .unwrap_or_default();

        match replace_file(&tmp, &target, &ops) {
            Ok(Replaced::Replaced) => {}
            Ok(Replaced::InUse) => {
                replaced_in_use.push((*name).to_owned());
            }
            Err(err) => {
                let _ = std::fs::remove_file(&tmp);
                let code = if err.kind() == std::io::ErrorKind::PermissionDenied {
                    RuntimeErrorCode::InUse
                } else {
                    RuntimeErrorCode::DiskError
                };
                return Err(RuntimeError::new(code, Some((*name).to_owned()), err.to_string()));
            }
        }

        local_files.push(RuntimeFile {
            name: (*name).to_owned(),
            size,
            sha256,
        });
    }

    let source = local_dir.to_string_lossy();
    let _ = write_runtime_marker(win64_dir, "local", &source, &local_files);

    Ok(RuntimeInstallResult { replaced_in_use })
}
