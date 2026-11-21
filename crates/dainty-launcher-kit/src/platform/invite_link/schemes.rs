// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Registers the invite schemes for this executable: the current user's registry on Windows, a desktop entry and
//! `xdg-mime` for an AppImage on Linux. Packaged client builds only; the shell decides that.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dainty_protocol::invite::invite_schemes;

use super::scheme_label;
use crate::platform::system::{run_hidden, system_root};

/// The handler's desktop entry, in `<data dir>/applications`.
pub const DESKTOP_FILE: &str = "dainty-lass-launcher-handler.desktop";
/// The entry older launchers wrote, removed so the two don't compete for the schemes.
pub const LEGACY_DESKTOP_FILE: &str = "reforged-revival-launcher-handler.desktop";

const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);

/// What [`register_schemes`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registered {
    /// The schemes now open this executable.
    Done,
    /// Nothing to register here: not an AppImage on Linux, or an OS without a scheme handler.
    Skipped,
}

/// One registry value under the current user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryValue {
    /// The key, from `HKCU`.
    pub key: String,
    /// The value's name; `None` is the key's default value.
    pub name: Option<&'static str>,
    /// The string written.
    pub data: String,
}

/// Registers the schemes for this process's executable (Windows) or AppImage (Linux).
pub async fn register_schemes() -> io::Result<Registered> {
    if cfg!(windows) {
        register_windows(
            &std::env::current_exe()?,
            &system_root().join("System32").join("reg.exe"),
        )
        .await?;
        return Ok(Registered::Done);
    }
    if cfg!(target_os = "linux") {
        let Some(appimage) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()) else {
            return Ok(Registered::Skipped);
        };
        let data = dirs::data_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))?;
        register_linux(Path::new(&appimage), &data, OsStr::new("xdg-mime")).await?;
        return Ok(Registered::Done);
    }
    Ok(Registered::Skipped)
}

/// The values each scheme needs; `None` when `exe` can't be written into a command line.
#[must_use]
pub fn registry_values(exe: &Path) -> Option<Vec<RegistryValue>> {
    let exe = registrable(exe)?;
    // A quote would end the quoted path early in the shell's command line.
    if exe.contains('"') {
        return None;
    }
    let mut values = Vec::new();
    for scheme in invite_schemes() {
        let key = format!(r"Software\Classes\{scheme}");
        values.push(RegistryValue {
            key: key.clone(),
            name: None,
            data: scheme_label(scheme).to_owned(),
        });
        values.push(RegistryValue {
            key: key.clone(),
            name: Some("URL Protocol"),
            data: String::new(),
        });
        values.push(RegistryValue {
            key: format!(r"{key}\shell\open\command"),
            name: None,
            data: format!("\"{exe}\" \"%1\""),
        });
    }
    Some(values)
}

/// `reg.exe`'s arguments for one value.
#[must_use]
pub fn reg_add_args(value: &RegistryValue) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["add".into(), format!(r"HKCU\{}", value.key).into()];
    match value.name {
        Some(name) => args.extend(["/v".into(), name.into()]),
        None => args.push("/ve".into()),
    }
    args.extend([
        "/t".into(),
        "REG_SZ".into(),
        "/d".into(),
        value.data.clone().into(),
        "/f".into(),
    ]);
    args
}

/// Writes every value through `reg_exe`, stopping at the first failure.
pub async fn register_windows(exe: &Path, reg_exe: &Path) -> io::Result<()> {
    let values = registry_values(exe).ok_or_else(|| bad_path(exe))?;
    for value in &values {
        run(reg_exe, &reg_add_args(value)).await?;
    }
    Ok(())
}

/// `path` quoted for a desktop entry's `Exec`; `None` when it is not absolute or holds control characters.
#[must_use]
pub fn exec_quote(path: &Path) -> Option<String> {
    let text = registrable(path)?;
    let mut quoted = String::from("\"");
    for c in text.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    // The desktop entry format reads the value once as a string (`\\`) and once as a field code (`%%`).
    Some(quoted.replace('\\', r"\\").replace('%', "%%"))
}

/// The desktop entry that opens the schemes with `appimage`.
#[must_use]
pub fn desktop_entry(appimage: &Path) -> Option<String> {
    let exec = exec_quote(appimage)?;
    let mime: String = invite_schemes()
        .iter()
        .map(|s| format!("x-scheme-handler/{s};"))
        .collect();
    Some(format!(
        "[Desktop Entry]\nType=Application\nName=Dainty Lass Launcher\nExec={exec} %u\nMimeType={mime}\n\
         NoDisplay=true\nTerminal=false\n"
    ))
}

/// Writes the desktop entry under `data_dir`, removes the old one, and makes it each scheme's default through
/// `xdg_mime`. Every scheme is tried; the first error is returned.
pub async fn register_linux(appimage: &Path, data_dir: &Path, xdg_mime: &OsStr) -> io::Result<()> {
    let entry = desktop_entry(appimage).ok_or_else(|| bad_path(appimage))?;
    let folder: PathBuf = data_dir.join("applications");
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join(DESKTOP_FILE), entry)?;
    match std::fs::remove_file(folder.join(LEGACY_DESKTOP_FILE)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut first_error = None;
    for scheme in invite_schemes() {
        let args = [
            "default".into(),
            DESKTOP_FILE.into(),
            format!("x-scheme-handler/{scheme}").into(),
        ];
        if let Err(e) = run(Path::new(xdg_mime), &args).await {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn registrable(path: &Path) -> Option<&str> {
    let text = path.to_str()?;
    (path.is_absolute() && !text.chars().any(char::is_control)).then_some(text)
}

fn bad_path(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("cannot register {}", path.display()),
    )
}

async fn run(program: &Path, args: &[OsString]) -> io::Result<()> {
    let mut command = run_hidden(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null());
    command.stderr(std::process::Stdio::null()).kill_on_drop(true);
    let status = tokio::time::timeout(COMMAND_TIMEOUT, command.status())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, format!("{} timed out", program.display())))??;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{} failed: {status}", program.display())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn absolute(unix: &str, windows: &str) -> PathBuf {
        PathBuf::from(if cfg!(windows) { windows } else { unix })
    }

    #[test]
    fn registry_values_for_each_scheme() {
        let exe = absolute("/opt/launcher", r"C:\Program Files\DaintyLass\launcher.exe");
        let values = registry_values(&exe).unwrap();
        let shown = exe.to_str().unwrap();
        let expect = |scheme: &str, label: &str| {
            vec![
                RegistryValue {
                    key: format!(r"Software\Classes\{scheme}"),
                    name: None,
                    data: label.into(),
                },
                RegistryValue {
                    key: format!(r"Software\Classes\{scheme}"),
                    name: Some("URL Protocol"),
                    data: "".into(),
                },
                RegistryValue {
                    key: format!(r"Software\Classes\{scheme}\shell\open\command"),
                    name: None,
                    data: format!("\"{shown}\" \"%1\""),
                },
            ]
        };
        let mut want = expect("dauntless", "URL:Dauntless invite");
        want.extend(expect("dainty", "URL:Dainty Lass invite"));
        assert_eq!(values, want);
        assert!(
            values.iter().all(|v| !v.key.contains("dauntless-revived")),
            "RD5: never registered"
        );
    }

    #[test]
    fn reg_arguments() {
        let default = RegistryValue {
            key: r"Software\Classes\x".into(),
            name: None,
            data: "URL:x".into(),
        };
        let args: Vec<OsString> = [
            "add",
            r"HKCU\Software\Classes\x",
            "/ve",
            "/t",
            "REG_SZ",
            "/d",
            "URL:x",
            "/f",
        ]
        .map(Into::into)
        .into();
        assert_eq!(reg_add_args(&default), args);
        let named = RegistryValue {
            key: "k".into(),
            name: Some("URL Protocol"),
            data: String::new(),
        };
        let args: Vec<OsString> = ["add", r"HKCU\k", "/v", "URL Protocol", "/t", "REG_SZ", "/d", "", "/f"]
            .map(Into::into)
            .into();
        assert_eq!(reg_add_args(&named), args);
    }

    #[test]
    fn unregistrable_paths() {
        assert_eq!(registry_values(Path::new("launcher.exe")), None);
        assert_eq!(registry_values(&absolute("/a\u{7}b", "C:\\a\u{7}b.exe")), None);
        assert_eq!(registry_values(&absolute("/a\"b", "C:\\a\"b.exe")), None);
        assert_eq!(exec_quote(Path::new("relative/app")), None);
        assert_eq!(exec_quote(&absolute("/a\nb", "C:\\a\nb")), None);
        assert_eq!(desktop_entry(Path::new("app")), None);
    }

    #[cfg(unix)]
    #[test]
    fn exec_quoting() {
        assert_eq!(exec_quote(Path::new("/opt/App Image")).unwrap(), "\"/opt/App Image\"");
        assert_eq!(exec_quote(Path::new("/a/$x`y\"z")).unwrap(), r#""/a/\\$x\\`y\\"z""#);
        assert_eq!(exec_quote(Path::new(r"/a\b")).unwrap(), r#""/a\\\\b""#);
        assert_eq!(exec_quote(Path::new("/100%")).unwrap(), "\"/100%%\"");
    }

    #[cfg(windows)]
    #[test]
    fn exec_quoting() {
        // Backslashes are path separators here, so they show the doubling twice over.
        assert_eq!(exec_quote(Path::new(r"C:\a b")).unwrap(), r#""C:\\\\a b""#);
        assert_eq!(exec_quote(Path::new(r"C:\$x%")).unwrap(), r#""C:\\\\\\$x%%""#);
    }

    #[test]
    fn desktop_entry_text() {
        let path = absolute("/home/u/DaintyLass.AppImage", r"C:\DaintyLass.AppImage");
        let exec = exec_quote(&path).unwrap();
        assert_eq!(
            desktop_entry(&path).unwrap(),
            format!(
                "[Desktop Entry]\nType=Application\nName=Dainty Lass Launcher\nExec={exec} %u\n\
                 MimeType=x-scheme-handler/dauntless;x-scheme-handler/dainty;\nNoDisplay=true\n\
                 Terminal=false\n"
            )
        );
    }

    #[tokio::test]
    async fn linux_registration_writes_the_entry_and_removes_the_old_one() {
        let data = tempfile::tempdir().unwrap();
        let apps = data.path().join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(apps.join(LEGACY_DESKTOP_FILE), "old").unwrap();
        let path = absolute("/home/u/DaintyLass.AppImage", r"C:\DaintyLass.AppImage");
        // No such tool: the entry is still written, and the failure comes back to be logged.
        let missing = data.path().join("no-xdg-mime");
        assert!(register_linux(&path, data.path(), missing.as_os_str()).await.is_err());
        assert_eq!(
            std::fs::read_to_string(apps.join(DESKTOP_FILE)).unwrap(),
            desktop_entry(&path).unwrap()
        );
        assert!(!apps.join(LEGACY_DESKTOP_FILE).exists());
    }

    #[tokio::test]
    async fn linux_registration_refuses_a_bad_path_before_writing() {
        let data = tempfile::tempdir().unwrap();
        assert!(
            register_linux(Path::new("rel"), data.path(), OsStr::new("true"))
                .await
                .is_err()
        );
        assert!(!data.path().join("applications").exists());
    }

    #[tokio::test]
    async fn windows_registration_stops_at_the_first_failure() {
        let dir = tempfile::tempdir().unwrap();
        let exe = absolute("/opt/launcher", r"C:\launcher.exe");
        assert!(register_windows(&exe, &dir.path().join("no-reg.exe")).await.is_err());
        assert!(
            register_windows(Path::new("launcher.exe"), &dir.path().join("x"))
                .await
                .is_err()
        );
    }
}
