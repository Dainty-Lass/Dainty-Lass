// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The hidden `dainty-host gameserver` command on Linux: Dauntless-Win64-Shipping.exe run under Wine or Proton,
//! and under xvfb-run when there is no display, from the variables the Director's env file sets. The Director names
//! the runtime to load in [`ENV_INJECT_DLL`]; the game then starts through the `dainty-inject.exe` helper, which
//! loads it into the game suspended inside the prefix (RD9 as of 2026-11-13).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::HostError;

/// The game executable's absolute path.
pub const ENV_EXE: &str = "DAINTY_GAME_EXE";
/// The Wine executable; found on PATH when unset.
pub const ENV_WINE: &str = "DAINTY_WINE";
/// The Proton executable; when set, Proton runs the game instead of Wine.
pub const ENV_PROTON: &str = "DAINTY_PROTON";
/// The Wine prefix's absolute path.
pub const ENV_PREFIX: &str = "DAINTY_WINEPREFIX";
/// Proton's compat data folder, an absolute path.
pub const ENV_PROTON_DATA: &str = "DAINTY_PROTON_DATA";
/// `0` runs the game without xvfb-run.
pub const ENV_XVFB: &str = "DAINTY_XVFB";
/// The xvfb-run executable; found on PATH when unset.
pub const ENV_XVFB_RUN: &str = "DAINTY_XVFB_RUN";
/// The runtime DLL the Director asks for, a host path (`rustic_isles_director::runtime`).
pub const ENV_INJECT_DLL: &str = "DAINTY_INJECT_DLL";
/// Its SHA-256, which the helper checks through a held handle before loading it.
pub const ENV_INJECT_SHA256: &str = "DAINTY_INJECT_SHA256";
/// The pre-R21 Director's second runtime: never read, and dropped from the game's environment (spec-runtime-r21 H-2).
pub const ENV_INJECT_MIT_DLL: &str = "DAINTY_INJECT_MIT_DLL";
/// Its SHA-256, dropped the same way.
pub const ENV_INJECT_MIT_SHA256: &str = "DAINTY_INJECT_MIT_SHA256";
/// The helper, beside `dainty-host` in a release.
pub use dainty_inject::helper::HELPER_EXE as INJECT_HELPER;

/// How a game server is launched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    /// The program run.
    pub command: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// The folder it runs in: the executable's.
    pub cwd: PathBuf,
    /// Its whole environment.
    pub env: BTreeMap<String, String>,
    /// What runs it, as the log says: `Wine`, `Proton (<build>)`, or either followed by ` on xvfb-run`.
    pub runtime: String,
}

fn is_abs_path(s: &str) -> bool {
    s.starts_with('/') || Path::new(s).is_absolute()
}

fn split_path_entries(path_var: &str) -> Vec<PathBuf> {
    if path_var.trim().is_empty() {
        return Vec::new();
    }
    if path_var.contains(';') {
        return path_var
            .split(';')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();
    }
    let raw: Vec<&str> = path_var.split(':').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let cur = raw[i];
        if cur.len() == 1
            && cur.as_bytes()[0].is_ascii_alphabetic()
            && i + 1 < raw.len()
            && (raw[i + 1].starts_with('\\') || raw[i + 1].starts_with('/'))
        {
            out.push(PathBuf::from(format!("{cur}:{}", raw[i + 1])));
            i += 2;
        } else {
            if !cur.is_empty() {
                out.push(PathBuf::from(cur));
            }
            i += 1;
        }
    }
    out
}

/// Whether `file` is a file this user may run (any file off Unix).
pub fn is_executable(file: &Path) -> bool {
    let Ok(meta) = fs::metadata(file) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        (meta.permissions().mode() & 0o111) != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The first of `names` that is executable in one of `path_var`'s folders.
pub fn find_on_path_in(names: &[&str], path_var: &str) -> Option<String> {
    let dirs = split_path_entries(path_var);
    for &name in names {
        if name.is_empty() {
            continue;
        }
        for dir in &dirs {
            let candidate = dir.join(name);
            if is_executable(&candidate) {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// The first of `names` on this process's PATH.
pub fn find_on_path(names: &[&str]) -> Result<Option<String>, HostError> {
    let path_var = std::env::var("PATH").unwrap_or_default();
    Ok(find_on_path_in(names, &path_var))
}

/// `wine64` or `wine` on `path_var`.
pub fn find_wine(path_var: &str) -> Result<Option<String>, HostError> {
    Ok(find_on_path_in(&["wine64", "wine"], path_var))
}

/// `existing`'s `WINEDLLOVERRIDES` with `winmm`, `mscoree` and `mshtml` set as the runtime needs. `winmm` is Wine's
/// own: a loader left beside the exe would load a second runtime beside the injected one.
pub fn dll_overrides(existing: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(raw) = existing {
        for item in raw.split(';') {
            let trimmed = item.trim();
            if trimmed.is_empty() {
                continue;
            }
            let key = trimmed
                .split_once('=')
                .map_or(trimmed, |(k, _)| k.trim())
                .to_ascii_lowercase();
            if key == "winmm" || key == "mscoree" || key == "mshtml" {
                continue;
            }
            parts.push(trimmed.to_owned());
        }
    }
    parts.push("winmm=b".to_owned());
    parts.push("mscoree=d".to_owned());
    parts.push("mshtml=d".to_owned());
    parts.join(";")
}

fn infer_steam_client_install_path(proton: &Path) -> Option<String> {
    let build_dir = proton.parent()?;
    let parent = build_dir.parent()?;
    let parent_name = parent.file_name()?.to_str()?;
    if parent_name == "compatibilitytools.d" {
        return parent.parent().map(|p| p.to_string_lossy().into_owned());
    }
    if parent_name == "common" {
        let grand = parent.parent()?;
        if grand.file_name()?.to_str()? == "steamapps" {
            return grand.parent().map(|p| p.to_string_lossy().into_owned());
        }
    }
    None
}

/// What Wine or Proton runs: the game itself, or `helper` with the runtime to load into it.
fn wine_program(
    exe: &str,
    game_args: &[String],
    env: &BTreeMap<String, String>,
    helper: &Path,
) -> Result<Vec<String>, HostError> {
    let named = |key: &str| env.get(key).map(|s| s.trim()).filter(|s| !s.is_empty());
    let pair = |dll_var: &str, sha_var: &str| match (named(dll_var), named(sha_var)) {
        (None, None) => Ok(None),
        (Some(dll), Some(sha256)) => Ok(Some((dll, sha256))),
        _ => Err(HostError::message(format!(
            "{dll_var} and {sha_var} are set together or not at all"
        ))),
    };
    // The pre-R21 second runtime's variables are ignored: the Director sets only these (spec-runtime-r21 H-2)
    let Some(runtime) = pair(ENV_INJECT_DLL, ENV_INJECT_SHA256)? else {
        return Ok(std::iter::once(exe.to_owned())
            .chain(game_args.iter().cloned())
            .collect());
    };
    if !helper.is_file() {
        return Err(HostError::message(format!(
            "{} is missing, so the game server can't load its runtime: reinstall the server release",
            helper.display()
        )));
    }
    let wine = |path: &str| {
        dainty_inject::helper::wine_path(Path::new(path))
            .ok_or_else(|| HostError::message(format!("not an absolute path: {path}")))
    };
    let game_args: Vec<std::ffi::OsString> = game_args.iter().map(Into::into).collect();
    let (dll, sha256) = runtime;
    let dlls = [(wine(dll)?, sha256.to_owned())];
    let line = dainty_inject::helper::command_line(&dlls, &wine(exe)?, &game_args);
    Ok(std::iter::once(helper.to_string_lossy().into_owned())
        .chain(line.into_iter().map(|a| a.to_string_lossy().into_owned()))
        .collect())
}

/// The helper beside this `dainty-host`.
pub fn inject_helper() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(INJECT_HELPER)))
        .unwrap_or_else(|| PathBuf::from(INJECT_HELPER))
}

/// The launch for the game with `game_args`, from the `DAINTY_*` variables in `env`, through `helper` when the
/// Director named a runtime. Fails when a variable is missing or not an absolute path, or a program is not found.
pub fn build_launch(
    game_args: &[String],
    env: &BTreeMap<String, String>,
    helper: &Path,
) -> Result<LaunchSpec, HostError> {
    let exe = env
        .get(ENV_EXE)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HostError::message(format!("{ENV_EXE} is not set")))?;
    if !is_abs_path(exe) {
        return Err(HostError::message(format!("{ENV_EXE} must be an absolute path: {exe}")));
    }
    let exe_path = PathBuf::from(exe);
    if !exe_path.is_file() {
        return Err(HostError::message(format!("game executable does not exist: {exe}")));
    }
    let cwd = exe_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));

    let mut out_env = env.clone();
    for var in [
        ENV_INJECT_DLL,
        ENV_INJECT_SHA256,
        ENV_INJECT_MIT_DLL,
        ENV_INJECT_MIT_SHA256,
    ] {
        out_env.remove(var);
    }
    let program = wine_program(exe, game_args, env, helper)?;
    let overrides = dll_overrides(out_env.get("WINEDLLOVERRIDES").map(String::as_str));
    out_env.insert("WINEDLLOVERRIDES".to_owned(), overrides);

    let path_var = env.get("PATH").map(String::as_str).unwrap_or("");

    let (mut command, mut args, mut runtime) =
        if let Some(proton) = env.get(ENV_PROTON).map(|s| s.trim()).filter(|s| !s.is_empty()) {
            if !is_abs_path(proton) {
                return Err(HostError::message(format!(
                    "{ENV_PROTON} must be an absolute path: {proton}"
                )));
            }
            let proton_path = Path::new(proton);
            if !is_executable(proton_path) {
                return Err(HostError::message(format!("Proton script is not executable: {proton}")));
            }
            let proton_data = env
                .get(ENV_PROTON_DATA)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| HostError::message(format!("{ENV_PROTON_DATA} is not set")))?;
            if !is_abs_path(proton_data) {
                return Err(HostError::message(format!(
                    "{ENV_PROTON_DATA} must be an absolute path: {proton_data}"
                )));
            }
            out_env.insert("STEAM_COMPAT_DATA_PATH".to_owned(), proton_data.to_owned());
            if !out_env
                .get("STEAM_COMPAT_CLIENT_INSTALL_PATH")
                .is_some_and(|v| !v.is_empty())
                && let Some(steam_dir) = infer_steam_client_install_path(proton_path)
            {
                out_env.insert("STEAM_COMPAT_CLIENT_INSTALL_PATH".to_owned(), steam_dir);
            }
            out_env.insert("STEAM_COMPAT_APP_ID".to_owned(), "0".to_owned());
            out_env.insert("PROTON_LOG".to_owned(), "0".to_owned());

            let build_name = proton_path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("Proton");
            let launch_args = std::iter::once("run".to_owned()).chain(program).collect();
            (proton.to_owned(), launch_args, format!("Proton ({build_name})"))
        } else {
            let wine_bin = if let Some(configured) = env.get(ENV_WINE).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                if !is_abs_path(configured) {
                    return Err(HostError::message(format!(
                        "{ENV_WINE} must be an absolute path: {configured}"
                    )));
                }
                if !is_executable(Path::new(configured)) {
                    return Err(HostError::message(format!(
                        "Wine binary is not executable: {configured}"
                    )));
                }
                configured.to_owned()
            } else {
                find_wine(path_var)?.ok_or_else(|| HostError::message("could not find wine64 or wine on PATH"))?
            };

            let prefix = env
                .get(ENV_PREFIX)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| HostError::message(format!("{ENV_PREFIX} is not set")))?;
            if !is_abs_path(prefix) {
                return Err(HostError::message(format!(
                    "{ENV_PREFIX} must be an absolute path: {prefix}"
                )));
            }
            out_env.insert("WINEPREFIX".to_owned(), prefix.to_owned());
            if !out_env.get("WINEDEBUG").is_some_and(|v| !v.is_empty()) {
                out_env.insert("WINEDEBUG".to_owned(), "-all".to_owned());
            }

            (wine_bin, program, "Wine".to_owned())
        };

    let xvfb_enabled = env.get(ENV_XVFB).map(String::as_str) != Some("0");
    let has_display = env.get("DISPLAY").map(|s| !s.trim().is_empty()).unwrap_or(false);

    if xvfb_enabled && !has_display {
        let xvfb_run = if let Some(configured) = env.get(ENV_XVFB_RUN).map(|s| s.trim()).filter(|s| !s.is_empty()) {
            if !is_abs_path(configured) || !is_executable(Path::new(configured)) {
                return Err(HostError::message(format!("xvfb-run is not executable: {configured}")));
            }
            configured.to_owned()
        } else {
            find_on_path_in(&["xvfb-run"], path_var).ok_or_else(|| {
                HostError::message("xvfb-run not found on PATH (install xvfb or set linux.xvfb = false)")
            })?
        };
        let mut wrapped_args = Vec::with_capacity(4 + args.len());
        wrapped_args.push("-a".to_owned());
        wrapped_args.push("-s".to_owned());
        wrapped_args.push("-screen 0 1024x768x24".to_owned());
        wrapped_args.push(command);
        wrapped_args.extend(args);
        command = xvfb_run;
        args = wrapped_args;
        runtime = format!("{runtime} on xvfb-run");
    }

    Ok(LaunchSpec {
        command,
        args,
        cwd,
        env: out_env,
        runtime,
    })
}

/// Whether `prefix` holds an initialized Wine prefix.
pub fn wine_prefix_ready(prefix: &Path) -> bool {
    prefix.join("system.reg").is_file() && prefix.join("drive_c").is_dir()
}

/// Launches the game server with `args` and waits for it; returns its exit code.
pub fn run_game_server(args: &[String]) -> Result<i32, HostError> {
    let env: BTreeMap<String, String> = std::env::vars().collect();
    let spec = build_launch(args, &env, &inject_helper())?;
    if let Some(prefix) = spec.env.get("WINEPREFIX") {
        fs::create_dir_all(prefix)?;
    }
    if let Some(proton_data) = spec.env.get("STEAM_COMPAT_DATA_PATH") {
        fs::create_dir_all(proton_data)?;
    }
    let status = Command::new(&spec.command)
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .envs(&spec.env)
        .status()
        .map_err(|e| HostError::message(format!("failed to start {}: {e}", spec.command)))?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use super::*;
    use tempfile::TempDir;

    fn write_executable(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).unwrap();
        }
    }

    #[test]
    fn dll_overrides_replaces_existing_and_appends() {
        assert_eq!(dll_overrides(None), "winmm=b;mscoree=d;mshtml=d");
        assert_eq!(dll_overrides(Some("")), "winmm=b;mscoree=d;mshtml=d");
        assert_eq!(
            dll_overrides(Some("d3d11=n;winmm=n,b;mscoree=n;dxgi=n")),
            "d3d11=n;dxgi=n;winmm=b;mscoree=d;mshtml=d"
        );
    }

    #[test]
    fn find_wine_prefers_wine64() {
        let tmp = TempDir::new().unwrap();
        let bin1 = tmp.path().join("bin1");
        let bin2 = tmp.path().join("bin2");
        write_executable(&bin1.join("wine"));
        write_executable(&bin2.join("wine64"));

        let path_var = format!("{}:{}", bin1.display(), bin2.display());
        let found = find_wine(&path_var).unwrap().unwrap();
        assert!(found.ends_with("wine64"));
    }

    #[test]
    fn wine_prefix_ready_checks_system_reg_and_drive_c() {
        let tmp = TempDir::new().unwrap();
        assert!(!wine_prefix_ready(tmp.path()));
        fs::write(tmp.path().join("system.reg"), "WINE REGISTRY Version 2\n").unwrap();
        assert!(!wine_prefix_ready(tmp.path()));
        fs::create_dir_all(tmp.path().join("drive_c")).unwrap();
        assert!(wine_prefix_ready(tmp.path()));
    }

    #[test]
    fn build_launch_wine_and_xvfb() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe");
        write_executable(&exe);
        let wine = tmp.path().join("bin/wine64");
        write_executable(&wine);
        let xvfb = tmp.path().join("bin/xvfb-run");
        write_executable(&xvfb);
        let prefix = tmp.path().join("wineprefix");

        let mut env = BTreeMap::new();
        env.insert(ENV_EXE.to_owned(), exe.to_string_lossy().into_owned());
        env.insert(ENV_WINE.to_owned(), wine.to_string_lossy().into_owned());
        env.insert(ENV_PREFIX.to_owned(), prefix.to_string_lossy().into_owned());
        env.insert(ENV_XVFB_RUN.to_owned(), xvfb.to_string_lossy().into_owned());

        let spec = build_launch(&["-log".to_owned(), "-port=8770".to_owned()], &env, Path::new("/none")).unwrap();
        assert_eq!(spec.command, xvfb.to_string_lossy());
        assert_eq!(
            spec.args,
            vec![
                "-a".to_owned(),
                "-s".to_owned(),
                "-screen 0 1024x768x24".to_owned(),
                wine.to_string_lossy().into_owned(),
                exe.to_string_lossy().into_owned(),
                "-log".to_owned(),
                "-port=8770".to_owned(),
            ]
        );
        assert_eq!(spec.runtime, "Wine on xvfb-run");
        assert_eq!(
            spec.env.get("WINEPREFIX").map(String::as_str),
            Some(prefix.to_string_lossy().as_ref())
        );
        assert_eq!(spec.env.get("WINEDEBUG").map(String::as_str), Some("-all"));
        assert_eq!(
            spec.env.get("WINEDLLOVERRIDES").map(String::as_str),
            Some("winmm=b;mscoree=d;mshtml=d")
        );
    }

    #[test]
    fn build_launch_proton_infers_steam_root() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("game/Dauntless-Win64-Shipping.exe");
        write_executable(&exe);
        let steam_root = tmp.path().join("steam");
        let proton = steam_root.join("compatibilitytools.d/GE-Proton9-20/proton");
        write_executable(&proton);
        let proton_data = tmp.path().join("proton-data");

        let mut env = BTreeMap::new();
        env.insert(ENV_EXE.to_owned(), exe.to_string_lossy().into_owned());
        env.insert(ENV_PROTON.to_owned(), proton.to_string_lossy().into_owned());
        env.insert(ENV_PROTON_DATA.to_owned(), proton_data.to_string_lossy().into_owned());
        env.insert(ENV_XVFB.to_owned(), "0".to_owned());

        let spec = build_launch(&["-log".to_owned()], &env, Path::new("/none")).unwrap();
        assert_eq!(spec.command, proton.to_string_lossy());
        assert_eq!(
            spec.args,
            vec!["run".to_owned(), exe.to_string_lossy().into_owned(), "-log".to_owned()]
        );
        assert_eq!(spec.runtime, "Proton (GE-Proton9-20)");
        assert_eq!(
            spec.env.get("STEAM_COMPAT_CLIENT_INSTALL_PATH").map(String::as_str),
            Some(steam_root.to_string_lossy().as_ref())
        );
        assert_eq!(spec.env.get("STEAM_COMPAT_APP_ID").map(String::as_str), Some("0"));
        assert_eq!(spec.env.get("PROTON_LOG").map(String::as_str), Some("0"));
    }

    #[test]
    fn the_runtime_goes_in_through_the_helper() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("Win64/Dauntless-Win64-Shipping.exe");
        write_executable(&exe);
        let wine = tmp.path().join("bin/wine64");
        write_executable(&wine);
        let helper = tmp.path().join("bin/dainty-inject.exe");
        let sha = "a".repeat(64);
        let mut env = BTreeMap::new();
        env.insert(ENV_EXE.to_owned(), exe.to_string_lossy().into_owned());
        env.insert(ENV_WINE.to_owned(), wine.to_string_lossy().into_owned());
        env.insert(ENV_PREFIX.to_owned(), "/srv/.workspace/wine".to_owned());
        env.insert(ENV_XVFB.to_owned(), "0".to_owned());
        env.insert(
            ENV_INJECT_DLL.to_owned(),
            "/srv/.workspace/runtime/rustic-isles-runtime.dll".to_owned(),
        );
        env.insert(ENV_INJECT_SHA256.to_owned(), sha.clone());

        let missing = build_launch(&["-server".to_owned()], &env, &helper)
            .unwrap_err()
            .to_string();
        assert!(missing.contains("dainty-inject.exe"), "{missing}");

        std::fs::write(&helper, b"MZ").unwrap();
        // The wrapper runs on Linux; Wine's Z: drive needs a Unix path
        if cfg!(unix) {
            let spec = build_launch(&["-server".to_owned()], &env, &helper).unwrap();
            let exe_wine = format!("Z:{}", exe.to_string_lossy().replace('/', "\\"));
            assert_eq!(
                spec.args,
                [
                    helper.to_string_lossy().into_owned(),
                    "--dll".to_owned(),
                    "Z:\\srv\\.workspace\\runtime\\rustic-isles-runtime.dll".to_owned(),
                    sha.clone(),
                    "--".to_owned(),
                    exe_wine,
                    "-server".to_owned(),
                ]
            );
            assert!(!spec.env.contains_key(ENV_INJECT_DLL));

            // A pre-R21 Director's second runtime is ignored, and never reaches the game (spec-runtime-r21 H-2)
            env.insert(ENV_INJECT_MIT_DLL.to_owned(), "/srv/old/Rustic-Isles.dll".to_owned());
            env.insert(ENV_INJECT_MIT_SHA256.to_owned(), "b".repeat(64));
            let again = build_launch(&["-server".to_owned()], &env, &helper).unwrap();
            assert_eq!(again.args, spec.args);
            assert!(!again.env.contains_key(ENV_INJECT_MIT_DLL) && !again.env.contains_key(ENV_INJECT_MIT_SHA256));
            env.remove(ENV_INJECT_MIT_DLL);
            env.remove(ENV_INJECT_MIT_SHA256);
        }

        env.remove(ENV_INJECT_SHA256);
        assert!(build_launch(&[], &env, &helper).is_err());
        env.remove(ENV_INJECT_DLL);
        // Alone, the retired variables start the game as if none were set
        env.insert(ENV_INJECT_MIT_DLL.to_owned(), "/r/rustic-isles-runtime.dll".to_owned());
        env.insert(ENV_INJECT_MIT_SHA256.to_owned(), sha.clone());
        assert!(build_launch(&[], &env, &helper).is_ok());
    }
}
