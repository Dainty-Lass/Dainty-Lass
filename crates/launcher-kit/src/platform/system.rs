// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! System binary discovery, disk-space probing, device signals and child-process helpers (spec §2.7.14).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::AsyncReadExt as _;

/// Creates a [`tokio::process::Command`] configured with `CREATE_NO_WINDOW` on Windows
/// so console helper tools do not flash a terminal window.
pub fn run_hidden(program: impl AsRef<OsStr>) -> tokio::process::Command {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = tokio::process::Command::new(program);
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(windows))]
    {
        tokio::process::Command::new(program)
    }
}

/// Returns the Windows root folder (`%SystemRoot%` or `%WINDIR%`, defaulting to `C:\Windows`).
#[must_use]
pub fn system_root() -> PathBuf {
    std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
}

/// Candidate paths for the Tailscale CLI binary on this OS, ordered by preference.
pub fn tailscale_candidates(env: &dyn Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    tailscale_candidates_for(cfg!(windows), env)
}

/// Candidate paths for the Tailscale CLI binary on Windows (`windows`) or Linux, ordered by preference.
pub fn tailscale_candidates_for(windows: bool, env: &dyn Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_unique = |path: PathBuf| {
        if !out.contains(&path) {
            out.push(path);
        }
    };
    let set = |var: &str| env(var).filter(|v| !v.is_empty());
    // Split by hand so a Windows `PATH` reads the same on any host; quoted entries are legal there.
    let path_dirs = |sep: char| -> Vec<PathBuf> {
        set("PATH")
            .map(|raw| {
                raw.to_string_lossy()
                    .split(sep)
                    .map(|entry| entry.trim().replace('"', ""))
                    .filter(|entry| !entry.is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default()
    };

    if windows {
        let exe = |dir: PathBuf| dir.join("Tailscale").join("tailscale.exe");
        for var in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(dir) = set(var) {
                push_unique(exe(PathBuf::from(dir)));
            }
        }
        push_unique(exe(PathBuf::from(r"C:\Program Files")));
        if let Some(local) = set("LOCALAPPDATA") {
            push_unique(exe(PathBuf::from(local)));
        }
        for dir in path_dirs(';') {
            push_unique(dir.join("tailscale.exe"));
        }
    } else {
        for fixed in ["/usr/bin", "/usr/local/bin", "/snap/bin"] {
            push_unique(Path::new(fixed).join("tailscale"));
        }
        for dir in path_dirs(':') {
            push_unique(dir.join("tailscale"));
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

const SIGNAL_TIMEOUT: Duration = Duration::from_secs(4);
const SIGNAL_OUTPUT_LIMIT: u64 = 4 * 1024;
const UUID_QUERY: &str = "(Get-CimInstance -ClassName Win32_ComputerSystemProduct).UUID";
const TPM_QUERY: &str = "(Get-TpmEndorsementKeyInfo -HashAlgorithm Sha256).PublicKeyHash";

/// Runs a helper with null stdin and stderr, killed on drop: its stdout as UTF-8 when it exits successfully
/// within `timeout` having printed at most `limit` bytes.
pub async fn capture_hidden(mut cmd: tokio::process::Command, timeout: Duration, limit: u64) -> Option<String> {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let run = async {
        let mut child = cmd.spawn().ok()?;
        let mut out = Vec::new();
        child.stdout.take()?.take(limit + 1).read_to_end(&mut out).await.ok()?;
        let status = child.wait().await.ok()?;
        (status.success() && out.len() as u64 <= limit).then_some(out)
    };
    let out = tokio::time::timeout(timeout, run).await.ok()??;
    String::from_utf8(out).ok()
}

async fn powershell(query: &str) -> Option<String> {
    let exe = system_root()
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let mut cmd = run_hidden(exe);
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", query]);
    capture_hidden(cmd, SIGNAL_TIMEOUT, SIGNAL_OUTPUT_LIMIT).await
}

/// This PC's SMBIOS UUID and TPM EK SHA-256, normalised by [`normalize_device_signals`]; both None off Windows.
pub async fn read_device_signals() -> (Option<String>, Option<String>) {
    if !cfg!(windows) {
        return (None, None);
    }
    let (uuid, tpm) = tokio::join!(powershell(UUID_QUERY), powershell(TPM_QUERY));
    normalize_device_signals(uuid.as_deref(), tpm.as_deref())
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
    fn windows_tailscale_candidates_in_order() {
        let list = tailscale_candidates_for(true, &|key| match key {
            "ProgramW6432" | "ProgramFiles" => Some(OsString::from(r"C:\Program Files")),
            "ProgramFiles(x86)" => Some(OsString::from(r"C:\Program Files (x86)")),
            "LOCALAPPDATA" => Some(OsString::from(r"C:\Users\P\AppData\Local")),
            "PATH" => Some(OsString::from(r#"C:\Tools;;"D:\Quoted Bin";C:\Tools"#)),
            _ => None,
        });
        let exe = |dir: &str| PathBuf::from(dir).join("Tailscale").join("tailscale.exe");
        assert_eq!(
            list,
            [
                exe(r"C:\Program Files"),
                exe(r"C:\Program Files (x86)"),
                exe(r"C:\Users\P\AppData\Local"),
                PathBuf::from(r"C:\Tools").join("tailscale.exe"),
                PathBuf::from(r"D:\Quoted Bin").join("tailscale.exe"),
            ]
        );
    }

    #[test]
    fn linux_tailscale_candidates_in_order() {
        let list = tailscale_candidates_for(false, &|key| {
            (key == "PATH").then(|| OsString::from("/opt/ts:/usr/bin:"))
        });
        let expected: Vec<PathBuf> = [
            Path::new("/usr/bin").join("tailscale"),
            Path::new("/usr/local/bin").join("tailscale"),
            Path::new("/snap/bin").join("tailscale"),
            Path::new("/opt/ts").join("tailscale"),
        ]
        .into();
        assert_eq!(list, expected);
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
        assert_eq!(t2, Some("a1".repeat(32)));

        let (u3, t3) = normalize_device_signals(Some("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF"), Some("abc"));
        assert_eq!((u3, t3), (None, None));
        let (u4, _) = normalize_device_signals(Some(" 12345678-9abc-def0-1234-56789abcdef0\r\n"), None);
        assert!(u4.is_some());
    }

    #[tokio::test]
    async fn capture_hidden_limits_output() {
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("cmd", &["/C", "echo hello"])
        } else {
            ("sh", &["-c", "echo hello"])
        };
        let mut cmd = run_hidden(program);
        cmd.args(args);
        assert_eq!(
            capture_hidden(cmd, Duration::from_secs(10), 64)
                .await
                .as_deref()
                .map(str::trim),
            Some("hello")
        );
        let mut cmd = run_hidden(program);
        cmd.args(args);
        assert_eq!(capture_hidden(cmd, Duration::from_secs(10), 3).await, None);
    }
}
