//! The game manifest (DR `src/manifest.ts`): the file list generated from the verified 1.4.4 zip by
//! DR's tools/make-game-manifest.ts. DR's launcher compiles in the same file, so what is served and
//! what it accepts are the same list; this binary does too (`CONTENT_MANIFEST` names another).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const EXPECTED_BUILD: &str = "dauntless_rel-1.4.4_Shipping_2020-10-28_20-11-15_239827";

/// DR's `data/dauntless-1.4.4.json` at 24b7624, built in.
pub const BUILT_IN: &str = include_str!("../../data/dauntless-1.4.4.json");

/// DR's `protocol::manifest::ManifestFile` (shared contract 4), here until the launcher needs it too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    /// Relative, with forward slashes
    pub path: String,
    pub size: u64,
    /// 64 lower-case hex
    pub sha256: String,
}

/// DR's `protocol::manifest::GameManifest`, serialised in Node's field order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub build: String,
    pub total_bytes: u64,
    /// Sorted by path
    pub files: Vec<ManifestFile>,
}

/// What `/content/v1/manifest` answers, as `JSON.stringify(manifest)` writes it. DR's
/// `downloadBaseUrl` (`CONTENT_DOWNLOAD_BASE_URL`) is not carried.
pub fn to_json(manifest: &Manifest) -> String {
    serde_json::to_string(manifest).unwrap_or_default()
}

/// A segment: letters, digits, "_", "-" and "."; not starting or ending with "."; at most 128.
fn is_safe_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    let edge = |b: &u8| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-';
    !bytes.is_empty()
        && bytes.len() <= 128
        && edge(&bytes[0])
        && edge(&bytes[bytes.len() - 1])
        && bytes.iter().all(|b| edge(b) || *b == b'.')
}

/// `con|prn|aux|nul|com[0-9]|lpt[0-9]`, optionally with an extension.
fn is_reserved(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit())
}

/// `IsSafeManifestPath`. Keep in sync with DR's tools/make-game-manifest.ts.
pub fn is_safe_manifest_path(value: &str) -> bool {
    !value.is_empty() && value.len() <= 400 && value.split('/').all(|s| is_safe_segment(s) && !is_reserved(s))
}

/// `ValidateManifest`: the reason it is not one we will serve from.
pub fn validate(raw: &Value) -> Result<Manifest, String> {
    let Some(obj) = raw.as_object() else {
        return Err("manifest is not an object".into());
    };
    let Some(build) = obj.get("build").and_then(Value::as_str).filter(|b| !b.is_empty()) else {
        return Err("manifest.build missing".into());
    };
    let Some(entries) = obj.get("files").and_then(Value::as_array).filter(|f| !f.is_empty()) else {
        return Err("manifest.files missing or empty".into());
    };
    let mut files = Vec::with_capacity(entries.len());
    let mut seen = HashSet::new();
    let mut total: u64 = 0;
    let mut previous = String::new();
    for entry in entries {
        let Some(entry) = entry.as_object() else {
            return Err("manifest entry is not an object".into());
        };
        let path = entry.get("path");
        let Some(path) = path.and_then(Value::as_str).filter(|p| is_safe_manifest_path(p)) else {
            return Err(format!(
                "unsafe manifest path {}",
                path.map_or_else(|| "undefined".into(), service_kit::jsjson::stringify)
            ));
        };
        let size = entry
            .get("size")
            .and_then(Value::as_f64)
            .filter(|s| s.fract() == 0.0 && *s >= 0.0 && *s <= 9_007_199_254_740_991.0)
            .ok_or_else(|| format!("bad size for {path}"))? as u64;
        let sha256 = entry
            .get("sha256")
            .and_then(Value::as_str)
            .filter(|s| s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
            .ok_or_else(|| format!("bad sha256 for {path}"))?;
        // Windows paths are case-insensitive: two entries that differ only in case would be one file
        if !seen.insert(path.to_lowercase()) {
            return Err(format!("duplicate manifest path {path}"));
        }
        if !previous.is_empty() && previous.as_str() >= path {
            return Err(format!("manifest is not sorted at {path}"));
        }
        previous = path.to_owned();
        // Saturating: a forged list of huge sizes cannot wrap the total into agreement
        total = total.saturating_add(size);
        files.push(ManifestFile {
            path: path.to_owned(),
            size,
            sha256: sha256.to_owned(),
        });
    }
    if obj.get("totalBytes").and_then(Value::as_f64) != Some(total as f64) {
        return Err(format!(
            "manifest.totalBytes is {}, the files add up to {total}",
            obj.get("totalBytes").map_or_else(
                || "undefined".into(),
                |v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                }
            )
        ));
    }
    Ok(Manifest {
        build: build.to_owned(),
        total_bytes: total,
        files,
    })
}

/// Parses manifest text (a leading BOM is allowed).
pub fn parse(text: &str) -> Result<Manifest, String> {
    let value: Value =
        serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(text)).map_err(|e| e.to_string())?;
    validate(&value)
}

/// `LoadManifest`: the named file, or the built-in one.
pub fn load(file: Option<&Path>) -> Result<Manifest, String> {
    match file {
        Some(file) => parse(&std::fs::read_to_string(file).map_err(|e| e.to_string())?),
        None => parse(BUILT_IN),
    }
}

/// Exact-match lookup. The request path is never touched after this: the file opened is always
/// built from the manifest's own path.
pub fn index(manifest: &Manifest) -> HashMap<String, ManifestFile> {
    manifest.files.iter().map(|f| (f.path.clone(), f.clone())).collect()
}

/// Where a manifest file lives under the game folder.
pub fn on_disk_path(game_dir: &Path, file: &ManifestFile) -> PathBuf {
    file.path
        .split('/')
        .fold(game_dir.to_path_buf(), |path, segment| path.join(segment))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_manifest_path_rule() {
        for good in [
            "Version.txt",
            "Archon/Content/Paks/Archon_Audio_0-WindowsClient.pak",
            "EasyAntiCheat/Licenses/CCO-1.0.txt",
            "a",
            "Engine/Binaries/Win64/debug.log",
        ] {
            assert!(is_safe_manifest_path(good), "{good}");
        }
        let long = "x".repeat(401);
        for bad in [
            "",
            "/Version.txt",
            "Version.txt/",
            "a//b",
            "../x",
            "a/../b",
            "./a",
            ".hidden",
            "trailing.",
            "a\\b",
            "C:/x",
            "a b",
            "CON",
            "nul.txt",
            "Archon/COM1/x",
            "a:b",
            &long,
            "\u{e4}.txt",
        ] {
            assert!(!is_safe_manifest_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_built_in_manifest_is_the_1_4_4_build() {
        let manifest = load(None).unwrap();
        assert_eq!(manifest.build, EXPECTED_BUILD);
        assert_eq!(manifest.total_bytes, 10_893_512_875);
        assert_eq!(
            manifest.files[0].sha256,
            "d3d41e614908d2befd518b27046d9822d6130ef12ba3504babbdb786bef9cff4"
        );
    }

    #[test]
    fn the_json_form_the_launchers_read() {
        let manifest = parse(&format!(
            r#"{{"files":[{{"sha256":"{}","size":3,"path":"a/b.pak"}}],"totalBytes":3,"build":"b"}}"#,
            "f".repeat(64)
        ))
        .unwrap();
        assert_eq!(
            to_json(&manifest),
            format!(
                r#"{{"build":"b","totalBytes":3,"files":[{{"path":"a/b.pak","size":3,"sha256":"{}"}}]}}"#,
                "f".repeat(64)
            )
        );
    }

    #[test]
    fn bad_manifests_are_refused() {
        let ok = json!({"build": "b", "totalBytes": 3, "files": [{"path": "a", "size": 1, "sha256": "0".repeat(64)}, {"path": "b", "size": 2, "sha256": "1".repeat(64)}]});
        assert!(validate(&ok).is_ok());
        let mut unsorted = ok.clone();
        unsorted["files"].as_array_mut().unwrap().reverse();
        assert!(validate(&unsorted).unwrap_err().contains("not sorted"));
        let mut total = ok.clone();
        total["totalBytes"] = json!(4);
        assert_eq!(
            validate(&total).unwrap_err(),
            "manifest.totalBytes is 4, the files add up to 3"
        );
        let mut case = ok.clone();
        case["files"][1]["path"] = json!("A");
        assert!(validate(&case).is_err());
        assert_eq!(validate(&json!([])).unwrap_err(), "manifest is not an object");
    }
}
