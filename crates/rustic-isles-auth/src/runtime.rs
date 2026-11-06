// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Runtime authorization (spec §6.6): the channel, the published manifest, the main hash an exchange code carries
//! and the artifact set a launch reports.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::errors::LauncherError;
use crate::store::Channel;
use crate::util;

/// The runtime's main DLL, which the manifest's top-level hash names.
pub const CORE_DLL: &str = "Rustic-Isles.dll";
/// The proxy DLL every approved artifact set must hold.
pub const PROXY_DLL: &str = "winmm.dll";

/// Every account runs `stable` (the tester role no longer moves a launcher).
pub fn expected_channel() -> Channel {
    Channel::Stable
}

/// `AUTH_CHANNEL_MISMATCH` unless `requested` is the channel the account must run.
pub fn assert_channel(requested: Channel) -> Result<(), LauncherError> {
    let expected = expected_channel();
    if requested != expected {
        return Err(LauncherError::new(
            "AUTH_CHANNEL_MISMATCH",
            format!(
                "Your account currently requires the {} runtime channel.",
                expected.as_str()
            ),
        ));
    }
    Ok(())
}

fn unsupported(message: impl Into<String>) -> LauncherError {
    LauncherError::new("RUNTIME_BUILD_UNSUPPORTED", message)
}

/// `<update root>/runtime/client/<channel>/windows-x86_64/latest.json`, the root `LAUNCHER_UPDATE_ROOT` as set (even
/// empty) else `updates`, made absolute against the working directory.
pub fn manifest_path(update_root: Option<&str>, channel: Channel) -> PathBuf {
    let root = Path::new(update_root.unwrap_or("updates"));
    let root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| root.to_path_buf(), |cwd| cwd.join(root))
    };
    root.join("runtime")
        .join("client")
        .join(channel.as_str())
        .join("windows-x86_64")
        .join("latest.json")
}

/// The manifest JSON, or the 409 a missing or unreadable one answers.
pub async fn load_manifest(update_root: Option<&str>, channel: Channel) -> Result<Value, LauncherError> {
    let path = manifest_path(update_root, channel);
    let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(path))
        .await
        .ok()
        .and_then(Result::ok);
    text.and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .ok_or_else(|| {
            unsupported(format!(
                "No approved {} runtime is currently published.",
                channel.as_str()
            ))
        })
}

/// JS-trimmed, lower-cased, 64 hex.
pub fn normalize_sha256(value: Option<&Value>) -> Option<String> {
    let Some(Value::String(text)) = value else {
        return None;
    };
    let normalized = service_kit::js::trim(text).to_lowercase();
    util::is_hex64(&normalized).then_some(normalized)
}

fn reported_sha256(value: Option<&Value>) -> Result<String, LauncherError> {
    normalize_sha256(value).ok_or_else(|| LauncherError::validation("runtimeSha256 must be a SHA-256 hash."))
}

/// `^\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)?$`
pub fn is_safe_version(text: &str) -> bool {
    let (core, suffix) = match text.find(['-', '+']) {
        Some(i) => (&text[..i], Some(&text[i + 1..])),
        None => (text, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let core_ok = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    let suffix_ok =
        suffix.is_none_or(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'));
    core_ok && suffix_ok
}

/// 1 to 96 of `A-Za-z0-9._-`.
pub fn is_safe_artifact_name(name: &str) -> bool {
    (1..=96).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn target_matches(manifest: &Value, target: u32) -> bool {
    let changelist = service_kit::js::number(manifest.get("targetChangelist"));
    changelist == f64::from(target)
}

/// The main hash an exchange code was minted with, against the manifest's current one.
pub fn assert_main_hash(
    manifest: &Value,
    target: u32,
    channel: Channel,
    reported: Option<&Value>,
) -> Result<String, LauncherError> {
    if !target_matches(manifest, target) {
        return Err(unsupported(
            "The published runtime does not match the supported game build.",
        ));
    }
    let Some(approved) = normalize_sha256(manifest.get("sha256")) else {
        return Err(unsupported(
            "The published runtime manifest has an invalid SHA-256 hash.",
        ));
    };
    let reported = reported_sha256(reported)?;
    if reported != approved {
        return Err(unsupported(format!(
            "The installed {} runtime is not the approved release.",
            channel.as_str()
        )));
    }
    Ok(approved)
}

/// A verified artifact set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedRuntime {
    /// The main DLL's hash
    pub runtime_sha256: String,
    /// The published manifest's version.
    pub manifest_version: String,
    /// `{lower name}:{sha256}` lines in locale order, joined by `\n`, SHA-256 hex
    pub artifact_set_sha256: String,
    /// The reported files with the manifest's names
    pub artifacts: Vec<(String, String)>,
}

/// `runtimeManifestVersion` and `runtimeArtifacts` against the manifest.
pub fn assert_artifact_set(
    manifest: &Value,
    target: u32,
    channel: Channel,
    reported_version: Option<&Value>,
    reported_artifacts: Option<&Value>,
) -> Result<ApprovedRuntime, LauncherError> {
    let version = match reported_version {
        Some(Value::String(v)) => service_kit::js::trim(v).to_owned(),
        _ => String::new(),
    };
    if !is_safe_version(&version) {
        return Err(LauncherError::validation("runtimeManifestVersion is invalid."));
    }
    let manifest_version = match manifest.get("version") {
        Some(Value::String(v)) => service_kit::js::trim(v).to_owned(),
        _ => String::new(),
    };
    if !is_safe_version(&manifest_version) || !target_matches(manifest, target) {
        return Err(unsupported(
            "The published runtime manifest is invalid or targets another game build.",
        ));
    }
    let Some(main_hash) = normalize_sha256(manifest.get("sha256")) else {
        return Err(unsupported(
            "The published runtime manifest has an invalid SHA-256 hash.",
        ));
    };
    let Some(Value::Array(extra)) = manifest.get("extraFiles") else {
        return Err(unsupported("The published runtime manifest is incomplete."));
    };
    let mut approved: Vec<(String, String)> = vec![(CORE_DLL.to_owned(), main_hash.clone())];
    for file in extra {
        let name = file.get("name").and_then(Value::as_str).unwrap_or("");
        let hash = normalize_sha256(file.get("sha256"));
        let duplicate = approved.iter().any(|(n, _)| n.eq_ignore_ascii_case(name));
        match hash {
            Some(hash) if is_safe_artifact_name(name) && !duplicate => approved.push((name.to_owned(), hash)),
            _ => {
                return Err(unsupported(
                    "The published runtime manifest has a duplicate or unsafe artifact.",
                ));
            }
        }
    }
    let has = |wanted: &str| approved.iter().any(|(n, _)| n.eq_ignore_ascii_case(wanted));
    if !has(CORE_DLL) || !has(PROXY_DLL) {
        return Err(unsupported(
            "The published runtime manifest does not contain the complete required artifact set.",
        ));
    }
    let is_core = |name: &str| name.eq_ignore_ascii_case(CORE_DLL) || name.eq_ignore_ascii_case(PROXY_DLL);
    let incomplete = || LauncherError::validation("The complete runtime artifact set is required.");
    let malformed = || LauncherError::validation("The runtime artifact set is malformed.");

    let Some(Value::Array(entries)) = reported_artifacts else {
        return Err(incomplete());
    };
    if entries.len() > approved.len() {
        return Err(incomplete());
    }
    let mut reported: Vec<(String, String)> = Vec::new();
    for entry in entries {
        let Some(name) = entry.get("name").and_then(Value::as_str) else {
            return Err(malformed());
        };
        let known = approved.iter().find(|(n, _)| n.eq_ignore_ascii_case(name));
        let repeated = reported.iter().any(|(n, _)| n.eq_ignore_ascii_case(name));
        let Some((canonical, _)) = known.filter(|_| is_safe_artifact_name(name) && !repeated) else {
            return Err(malformed());
        };
        reported.push((canonical.clone(), reported_sha256(entry.get("sha256"))?));
    }
    let core_reported = approved
        .iter()
        .filter(|(n, _)| is_core(n))
        .all(|(n, _)| reported.iter().any(|(r, _)| r == n));
    let optional_total = approved.iter().filter(|(n, _)| !is_core(n)).count();
    let optional_reported = reported.iter().filter(|(n, _)| !is_core(n)).count();
    if !core_reported || (optional_reported != 0 && optional_reported != optional_total) {
        return Err(incomplete());
    }
    let mismatch = version != manifest_version
        || reported
            .iter()
            .any(|(name, hash)| approved.iter().any(|(n, h)| n == name && h != hash));
    if mismatch {
        return Err(unsupported(format!(
            "The installed {} runtime set is not the approved release.",
            channel.as_str()
        )));
    }
    Ok(ApprovedRuntime {
        runtime_sha256: main_hash,
        manifest_version,
        artifact_set_sha256: artifact_set_digest(&reported),
        artifacts: reported,
    })
}

/// The artifact set's digest: `{lower name}:{sha256}` lines in locale order, joined by `\n`, SHA-256 hex.
pub fn artifact_set_digest(files: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by(|a, b| service_kit::collate::locale_compare(&a.0, &b.0));
    let lines: Vec<String> = sorted
        .iter()
        .map(|(name, hash)| format!("{}:{hash}", name.to_lowercase()))
        .collect();
    util::sha256_hex(&lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn h(c: char) -> String {
        c.to_string().repeat(64)
    }

    fn manifest(extra: Value) -> Value {
        json!({"version": "0.1.78", "targetChangelist": 647472, "sha256": h('b'), "extraFiles": extra})
    }

    fn core() -> Value {
        manifest(json!([{"name": "winmm.dll", "sha256": h('c')}]))
    }

    fn set(files: &[(&str, String)]) -> Value {
        Value::Array(files.iter().map(|(n, s)| json!({"name": n, "sha256": s})).collect())
    }

    fn check(manifest: &Value, version: &str, files: Value) -> Result<ApprovedRuntime, String> {
        assert_artifact_set(manifest, 647_472, Channel::Stable, Some(&json!(version)), Some(&files))
            .map_err(|e| format!("{} {}", e.status(), e.message))
    }

    #[test]
    fn channels() {
        for (text, channel) in [
            ("stable", Channel::Stable),
            ("beta", Channel::Beta),
            ("dev", Channel::Dev),
        ] {
            assert_eq!(Channel::parse(text), Some(channel));
        }
        assert_eq!(Channel::parse("preview"), None);
        assert!(assert_channel(Channel::Stable).is_ok());
        assert_eq!(assert_channel(Channel::Beta).unwrap_err().status(), 409);
        assert_eq!(
            assert_channel(Channel::Dev).unwrap_err().message,
            "Your account currently requires the stable runtime channel."
        );
    }

    #[test]
    fn main_hash() {
        let m = core();
        let upper = json!(h('B'));
        assert_eq!(
            assert_main_hash(&m, 647_472, Channel::Stable, Some(&upper)).unwrap(),
            h('b')
        );
        assert_eq!(
            assert_main_hash(&m, 647_472, Channel::Stable, Some(&json!(h('d'))))
                .unwrap_err()
                .status(),
            409
        );
        assert_eq!(
            assert_main_hash(&m, 1, Channel::Stable, Some(&upper))
                .unwrap_err()
                .status(),
            409
        );
        assert_eq!(
            assert_main_hash(&m, 647_472, Channel::Stable, Some(&json!("x")))
                .unwrap_err()
                .status(),
            400
        );
    }

    #[test]
    fn core_sets() {
        let m = core();
        let ok = check(
            &m,
            "0.1.78",
            set(&[("Rustic-Isles.dll", h('B')), ("winmm.dll", h('C'))]),
        )
        .unwrap();
        assert_eq!(
            ok.artifact_set_sha256,
            "2aa2a8904fc44b11bb0be6ee5177c8904dc0f428c3dbb6ff2201a065fa5fff20"
        );
        assert_eq!(ok.runtime_sha256, h('b'));
        let changed = set(&[("Rustic-Isles.dll", h('b')), ("winmm.dll", h('d'))]);
        assert!(check(&m, "0.1.78", changed).unwrap_err().starts_with("409"));
        let omitted = set(&[("Rustic-Isles.dll", h('b'))]);
        assert_eq!(
            check(&m, "0.1.78", omitted).unwrap_err(),
            "400 The complete runtime artifact set is required."
        );
        let unknown = set(&[
            ("Rustic-Isles.dll", h('b')),
            ("winmm.dll", h('c')),
            ("extra.dll", h('e')),
        ]);
        assert_eq!(
            check(&m, "0.1.78", unknown).unwrap_err(),
            "400 The complete runtime artifact set is required."
        );
        let renamed = set(&[("Rustic-Isles.dll", h('b')), ("other.dll", h('c'))]);
        assert_eq!(
            check(&m, "0.1.78", renamed).unwrap_err(),
            "400 The runtime artifact set is malformed."
        );
        let full = set(&[("Rustic-Isles.dll", h('b')), ("winmm.dll", h('c'))]);
        assert!(check(&m, "0.1.77", full.clone()).unwrap_err().starts_with("409"));
        assert_eq!(
            check(&m, "v1", full.clone()).unwrap_err(),
            "400 runtimeManifestVersion is invalid."
        );
        assert!(
            check(&manifest(json!([])), "0.1.78", full)
                .unwrap_err()
                .starts_with("409")
        );
    }

    #[test]
    fn optional_files_are_all_or_none() {
        let m = manifest(json!([
            {"name": "winmm.dll", "sha256": h('c')},
            {"name": "a.pak", "sha256": h('1')},
            {"name": "b.pak", "sha256": h('2')}
        ]));
        let core_only = check(
            &m,
            "0.1.78",
            set(&[("Rustic-Isles.dll", h('b')), ("winmm.dll", h('c'))]),
        )
        .unwrap();
        let all = set(&[
            ("Rustic-Isles.dll", h('b')),
            ("winmm.dll", h('c')),
            ("a.pak", h('1')),
            ("b.pak", h('2')),
        ]);
        let full = check(&m, "0.1.78", all).unwrap();
        assert_ne!(core_only.artifact_set_sha256, full.artifact_set_sha256);
        let partial = set(&[("Rustic-Isles.dll", h('b')), ("winmm.dll", h('c')), ("a.pak", h('1'))]);
        assert!(check(&m, "0.1.78", partial).unwrap_err().starts_with("400"));
        let mutated = set(&[
            ("Rustic-Isles.dll", h('b')),
            ("winmm.dll", h('c')),
            ("a.pak", h('1')),
            ("b.pak", h('3')),
        ]);
        assert!(check(&m, "0.1.78", mutated).unwrap_err().starts_with("409"));
        let unpublished = set(&[("Rustic-Isles.dll", h('b')), ("winmm.dll", h('c')), ("a.pak", h('1'))]);
        assert!(check(&core(), "0.1.78", unpublished).unwrap_err().starts_with("400"));
    }

    #[test]
    fn safe_texts() {
        assert!(is_safe_version("0.1.78"));
        assert!(is_safe_version("1.2.3-rc.1"));
        assert!(is_safe_version("1.2.3+b.7"));
        assert!(!is_safe_version("1.2"));
        assert!(!is_safe_version("1.2.3-"));
        assert!(is_safe_artifact_name("winmm.dll"));
        assert!(!is_safe_artifact_name("../x"));
        assert!(!is_safe_artifact_name(&"a".repeat(97)));
    }
}
