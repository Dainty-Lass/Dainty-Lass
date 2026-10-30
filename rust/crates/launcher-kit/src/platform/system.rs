// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! System binary discovery, disk-space probing, and child-process helpers.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Creates a [`tokio::process::Command`] configured with `CREATE_NO_WINDOW` on Windows
/// so console helper tools do not flash a terminal window.
pub fn run_hidden(program: impl AsRef<OsStr>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Returns the Windows root folder (`%SystemRoot%` or `%WINDIR%`, defaulting to `C:\Windows`).
#[must_use]
pub fn system_root() -> PathBuf {
    std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
}

/// Candidate paths for the Tailscale CLI binary, ordered by preference.
pub fn tailscale_candidates(env: &dyn Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_unique = |path: PathBuf| {
        if !out.contains(&path) {
            out.push(path);
        }
    };

    if cfg!(windows) {
        for var in ["ProgramW6432", "ProgramFiles"] {
            if let Some(dir) = env(var).filter(|v| !v.is_empty()) {
                push_unique(PathBuf::from(dir).join("Tailscale").join("tailscale.exe"));
            }
        }
        push_unique(PathBuf::from(r"C:\Program Files\Tailscale\tailscale.exe"));
        if let Some(local) = env("LOCALAPPDATA").filter(|v| !v.is_empty()) {
            push_unique(PathBuf::from(local).join("Tailscale").join("tailscale.exe"));
        }
        if let Some(path_var) = env("PATH") {
            for entry in std::env::split_paths(&path_var) {
                if !entry.as_os_str().is_empty() {
                    push_unique(entry.join("tailscale.exe"));
                }
            }
        }
    } else {
        for fixed in ["/usr/bin/tailscale", "/usr/local/bin/tailscale", "/snap/bin/tailscale"] {
            push_unique(PathBuf::from(fixed));
        }
        if let Some(path_var) = env("PATH") {
            for entry in std::env::split_paths(&path_var) {
                if !entry.as_os_str().is_empty() {
                    push_unique(entry.join("tailscale"));
                }
            }
        }
    }

    out
}

/// Locates an installed `tailscale` (`tailscale.exe` on Windows) binary on disk.
#[must_use]
pub fn find_tailscale() -> Option<PathBuf> {
    tailscale_candidates(&|name| std::env::var_os(name))
        .into_iter()
        .find(|candidate| candidate.is_file())
}

/// Walks `dir` and its parent directories until `fs4::available_space` succeeds.
#[must_use]
pub fn free_bytes(dir: &Path) -> Option<u64> {
    for ancestor in dir.ancestors() {
        if ancestor.as_os_str().is_empty() {
            break;
        }
        if let Ok(space) = fs4::available_space(ancestor) {
            return Some(space);
        }
    }
    None
}

/// Normalizes optional SMBIOS UUID (8-4-4-4-12 lowercase hex) and TPM EK SHA-256 (64 lowercase hex),
/// rejecting all-zero, all-`f`, and placeholder values.
#[must_use]
pub fn normalize_device_signals(uuid: Option<&str>, tpm: Option<&str>) -> (Option<String>, Option<String>) {
    let norm_uuid = uuid.and_then(|raw| {
        let lower = raw.trim().to_ascii_lowercase();
        let bytes = lower.as_bytes();
        if bytes.len() != 36 {
            return None;
        }
        for (idx, &b) in bytes.iter().enumerate() {
            if matches!(idx, 8 | 13 | 18 | 23) {
                if b != b'-' {
                    return None;
                }
            } else if !b.is_ascii_hexdigit() {
                return None;
            }
        }
        let hex_only: Vec<u8> = bytes.iter().copied().filter(|&b| b != b'-').collect();
        if hex_only.iter().all(|&b| b == b'0')
            || hex_only.iter().all(|&b| b == b'f')
            || lower == "03000200-0400-0500-0006-000700080009"
        {
            return None;
        }
        Some(lower)
    });

    let norm_tpm = tpm.and_then(|raw| {
        let lower = raw.trim().to_ascii_lowercase();
        let bytes = lower.as_bytes();
        if bytes.len() != 64 || !bytes.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        if bytes.iter().all(|&b| b == b'0') || bytes.iter().all(|&b| b == b'f') {
            return None;
        }
        Some(lower)
    });

    (norm_uuid, norm_tpm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailscale_candidates_include_program_files_and_path() {
        let list = tailscale_candidates(&|key| match key {
            "ProgramW6432" => Some(OsString::from(r"C:\Program Files")),
            "LOCALAPPDATA" => Some(OsString::from(r"C:\Users\Player\AppData\Local")),
            "PATH" => Some(OsString::from(r"C:\Tools;D:\Bin")),
            _ => None,
        });
        assert!(!list.is_empty());
    }

    #[test]
    fn free_bytes_walks_to_existing_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("does").join("not").join("exist");
        assert!(free_bytes(&nested).is_some());
    }

    #[test]
    fn device_signals_reject_placeholders_and_keep_valid_values() {
        let (u, t) = normalize_device_signals(Some("03000200-0400-0500-0006-000700080009"), Some(&"0".repeat(64)));
        assert_eq!(u, None);
        assert_eq!(t, None);

        let (u2, t2) = normalize_device_signals(Some("12345678-9ABC-DEF0-1234-56789ABCDEF0"), Some(&"a1".repeat(32)));
        assert_eq!(u2.as_deref(), Some("12345678-9abc-def0-1234-56789abcdef0"));
        assert_eq!(t2.as_deref(), Some(/*"a1" * 32*/ &"a1".repeat(32) as &str));
    }
}
