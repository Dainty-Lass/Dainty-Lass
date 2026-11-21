// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Pinned store-capture manifest parser and git-backed fetcher.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::HostError;
use crate::paths::Paths;
use crate::run::{RunOptions, run};

const DEFAULT_CAPTURE_NAME: &str = "DauntlessEndpointDocumentation";

/// `store-captures.json`: the store captures pinned to one commit of a git repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturePin {
    /// The capture set's name, also the bare repository's under `.workspace/captures`.
    pub name: String,
    /// The repository's https URL.
    pub repository: String,
    /// The pinned commit, 40 lowercase hex digits.
    pub commit: String,
    /// Each file as (path in the repository, its SHA-256 in lowercase hex).
    pub files: Vec<(String, String)>,
}

/// What [`ensure_captures`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureFetch {
    /// Every file was there already with its pinned SHA-256.
    AlreadyReady,
    /// The files were fetched.
    Fetched,
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn normalize_lf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn is_lower_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_valid_repository(repo: &str) -> bool {
    let repo = repo.trim();
    if repo.starts_with("https://") || repo.starts_with("file://") || repo.starts_with('/') {
        return repo.len() > 1 && !repo.chars().any(char::is_control);
    }
    let bytes = repo.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
        && !repo.chars().any(char::is_control)
}

fn is_valid_rel_path(path: &str) -> bool {
    if path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
    {
        return false;
    }
    path.split('/')
        .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn is_valid_pin_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn target_path(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for segment in rel.split('/') {
        out.push(segment);
    }
    out
}

/// Reads and checks the pin at `file`.
pub fn read_capture_pin(file: &Path) -> Result<CapturePin, HostError> {
    let text = std::fs::read_to_string(file).map_err(|e| HostError::io(file, e))?;
    parse_capture_pin(file, &text)
}

/// Parses and checks a pin: its name, a URL, a full commit id and every file's path and SHA-256.
pub fn parse_capture_pin(file: &Path, text: &str) -> Result<CapturePin, HostError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let value: Value =
        serde_json::from_str(text).map_err(|e| HostError::message(format!("{}: {e}", file.display())))?;
    let Some(obj) = value.as_object() else {
        return Err(HostError::message(format!(
            "{}: expected a JSON object",
            file.display()
        )));
    };

    let name = match obj.get("name").and_then(Value::as_str) {
        Some(n) if is_valid_pin_name(n) => n.to_owned(),
        Some(_) => {
            return Err(HostError::message(format!("{}: invalid pin name", file.display())));
        }
        None => DEFAULT_CAPTURE_NAME.to_owned(),
    };

    let Some(repository) = obj.get("repository").and_then(Value::as_str) else {
        return Err(HostError::message(format!(
            "{}: repository must be an https URL",
            file.display()
        )));
    };
    if !is_valid_repository(repository) {
        return Err(HostError::message(format!(
            "{}: repository must be an https URL",
            file.display()
        )));
    }

    let Some(commit) = obj.get("commit").and_then(Value::as_str) else {
        return Err(HostError::message(format!(
            "{}: commit must be a full 40-character commit id",
            file.display()
        )));
    };
    if !is_lower_hex(commit, 40) {
        return Err(HostError::message(format!(
            "{}: commit must be a full 40-character commit id",
            file.display()
        )));
    }

    let Some(files_obj) = obj.get("files").and_then(Value::as_object) else {
        return Err(HostError::message(format!(
            "{}: files must map plain paths to SHA-256 hashes",
            file.display()
        )));
    };
    let mut files = Vec::with_capacity(files_obj.len());
    for (rel, val) in files_obj {
        let Some(hash) = val.as_str() else {
            return Err(HostError::message(format!(
                "{}: files must map plain paths to SHA-256 hashes",
                file.display()
            )));
        };
        if !is_valid_rel_path(rel) || !is_lower_hex(hash, 64) {
            return Err(HostError::message(format!(
                "{}: files must map plain paths to SHA-256 hashes",
                file.display()
            )));
        }
        files.push((rel.clone(), hash.to_owned()));
    }
    if files.is_empty() {
        return Err(HostError::message(format!("{}: no files", file.display())));
    }

    Ok(CapturePin {
        name,
        repository: repository.trim().to_owned(),
        commit: commit.to_owned(),
        files,
    })
}

fn file_matches_sha256(path: &Path, expected: &str) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    if sha256_hex(&bytes) == expected {
        return true;
    }
    let normalized = normalize_lf(&bytes);
    sha256_hex(&normalized) == expected
}

/// The pinned files missing or not matching their SHA-256.
pub fn missing_captures(p: &Paths, pin: &CapturePin) -> Vec<String> {
    pin.files
        .iter()
        .filter(|(rel, sha)| !file_matches_sha256(&target_path(&p.store_captures, rel), sha))
        .map(|(rel, _)| rel.clone())
        .collect()
}

fn git_show_blob(git_dir: &Path, commit: &str, rel: &str) -> Result<Vec<u8>, HostError> {
    let spec = format!("{commit}:{rel}");
    let mut cmd = Command::new("git");
    cmd.args(["--git-dir", &git_dir.to_string_lossy(), "show", &spec]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let output = cmd
        .output()
        .map_err(|e| HostError::message(format!("could not run git show {spec}: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.lines().last().unwrap_or("git show failed").trim();
        return Err(HostError::message(format!(
            "capture commit {commit} has no {rel} ({detail})"
        )));
    }
    Ok(output.stdout)
}

/// Fetches the pinned commit with git into a bare repository under `.workspace/captures` and writes every pinned
/// file whose SHA-256 checks out; fails when one does not. Logs git's output to `log_file`.
pub fn ensure_captures(p: &Paths, pin: &CapturePin, log_file: Option<&Path>) -> Result<CaptureFetch, HostError> {
    if missing_captures(p, pin).is_empty() {
        return Ok(CaptureFetch::AlreadyReady);
    }

    let captures_root = p.workspace.join("captures");
    std::fs::create_dir_all(&captures_root).map_err(|e| HostError::io(&captures_root, e))?;
    let git_dir = captures_root.join(format!("{}.git", pin.name));
    let git_dir_str = git_dir.to_string_lossy().into_owned();
    let run_opts = RunOptions {
        cwd: captures_root,
        env: None,
        log_file: log_file.map(Path::to_path_buf),
        quiet: true,
    };

    if !git_dir.join("HEAD").is_file() {
        let init = run("git", &["init", "--bare", "--quiet", &git_dir_str], &run_opts)?;
        if init.code != 0 {
            return Err(HostError::message(format!(
                "could not initialize capture repository: {}",
                init.last_line.unwrap_or_else(|| format!("exit {}", init.code))
            )));
        }
    }

    let fetch = run(
        "git",
        &[
            "--git-dir",
            &git_dir_str,
            "-c",
            "protocol.version=2",
            "fetch",
            "--quiet",
            "--depth",
            "1",
            &pin.repository,
            &pin.commit,
        ],
        &run_opts,
    )?;
    if fetch.code != 0 {
        return Err(HostError::message(format!(
            "could not fetch store captures from {} ({})",
            pin.repository,
            fetch.last_line.unwrap_or_else(|| format!("exit {}", fetch.code))
        )));
    }

    let mut staged: Vec<(PathBuf, Vec<u8>)> = Vec::with_capacity(pin.files.len());
    for (rel, expected_sha) in &pin.files {
        let raw = git_show_blob(&git_dir, &pin.commit, rel)?;
        let raw_sha = sha256_hex(&raw);
        let verified_bytes = if raw_sha == *expected_sha {
            raw
        } else {
            let lf = normalize_lf(&raw);
            let lf_sha = sha256_hex(&lf);
            if lf_sha == *expected_sha {
                lf
            } else {
                return Err(HostError::message(format!(
                    "store capture {rel} does not match its pinned SHA-256 (expected {expected_sha}, got {raw_sha})"
                )));
            }
        };
        staged.push((target_path(&p.store_captures, rel), verified_bytes));
    }

    for (dest, bytes) in staged {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
        }
        std::fs::write(&dest, &bytes).map_err(|e| HostError::io(&dest, e))?;
    }

    Ok(CaptureFetch::Fetched)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn parses_valid_pin_and_rejects_bad_fields() {
        let file = Path::new("store-captures.json");
        let hash = "a".repeat(64);
        let commit = "b".repeat(40);
        let valid = format!(
            r#"{{
                "repository": "https://example.com/captures.git",
                "commit": "{commit}",
                "files": {{
                    "Store/ladyluck.json": "{hash}"
                }}
            }}"#
        );
        let pin = parse_capture_pin(file, &valid).unwrap();
        assert_eq!(pin.name, "DauntlessEndpointDocumentation");
        assert_eq!(pin.repository, "https://example.com/captures.git");
        assert_eq!(pin.commit, commit);
        assert_eq!(pin.files, vec![("Store/ladyluck.json".to_owned(), hash.clone())]);

        let bad_repo =
            format!(r#"{{"repository":"http://example.com/x.git","commit":"{commit}","files":{{"a.json":"{hash}"}}}}"#);
        assert!(
            parse_capture_pin(file, &bad_repo)
                .unwrap_err()
                .to_string()
                .contains("repository must be an https URL")
        );

        let bad_commit =
            format!(r#"{{"repository":"https://example.com/x.git","commit":"1234","files":{{"a.json":"{hash}"}}}}"#);
        assert!(
            parse_capture_pin(file, &bad_commit)
                .unwrap_err()
                .to_string()
                .contains("commit must be a full 40-character commit id")
        );

        let bad_path = format!(
            r#"{{"repository":"https://example.com/x.git","commit":"{commit}","files":{{"../a.json":"{hash}"}}}}"#
        );
        assert!(
            parse_capture_pin(file, &bad_path)
                .unwrap_err()
                .to_string()
                .contains("files must map plain paths to SHA-256 hashes")
        );

        let empty_files = format!(r#"{{"repository":"https://example.com/x.git","commit":"{commit}","files":{{}}}}"#);
        assert!(
            parse_capture_pin(file, &empty_files)
                .unwrap_err()
                .to_string()
                .contains("no files")
        );
    }

    #[test]
    fn missing_captures_checks_presence_and_sha256() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        let content = b"{\"hello\":\"world\"}\n";
        let sha = sha256_hex(content);
        let pin = CapturePin {
            name: "test".into(),
            repository: "https://example.com/repo.git".into(),
            commit: "0".repeat(40),
            files: vec![("sub/item.json".into(), sha)],
        };
        assert_eq!(missing_captures(&paths, &pin), vec!["sub/item.json"]);

        let dest = paths.store_captures.join("sub").join("item.json");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"wrong").unwrap();
        assert_eq!(missing_captures(&paths, &pin), vec!["sub/item.json"]);

        std::fs::write(&dest, content).unwrap();
        assert!(missing_captures(&paths, &pin).is_empty());
        assert_eq!(ensure_captures(&paths, &pin, None).unwrap(), CaptureFetch::AlreadyReady);
    }
}
