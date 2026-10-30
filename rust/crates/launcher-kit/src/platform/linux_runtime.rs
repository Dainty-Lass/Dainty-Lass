// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Linux Wine/Proton/ULWGL runtime discovery, launch environment preparation, and package helpers (Spec §3.3.1).

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Snapshot of environment variables and filesystem roots used to probe Linux runtimes.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub home: Option<PathBuf>,
    pub path: Vec<PathBuf>,
    pub flatpak: bool,
    pub vars: HashMap<String, String>,
}

impl Env {
    /// Looks up an environment variable in the snapshot.
    #[must_use]
    pub fn var(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }

    /// Searches `self.path` for an executable file named `binary`.
    #[must_use]
    pub fn find_in_path(&self, binary: &str) -> Option<PathBuf> {
        self.path
            .iter()
            .map(|dir| dir.join(binary))
            .find(|candidate| candidate.is_file())
    }
}

/// Captures the current process environment into an [`Env`].
#[must_use]
pub fn current_env() -> Env {
    let vars: HashMap<String, String> = std::env::vars().collect();
    let home = vars
        .get("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir);
    let path = std::env::var_os("PATH")
        .map(|raw| std::env::split_paths(&raw).collect())
        .unwrap_or_default();
    let flatpak = Path::new("/.flatpak-info").exists() || vars.contains_key("FLATPAK_ID");
    Env {
        home,
        path,
        flatpak,
        vars,
    }
}

/// Compatibility runner family used to launch the Windows game executable on Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Proton,
    Umu,
    Wine,
}

pub type RunnerKind = RuntimeKind;

/// A discovered Linux compatibility runtime on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRuntime {
    pub kind: RuntimeKind,
    pub binary: PathBuf,
    pub label: String,
    pub steam_root: Option<PathBuf>,
}

/// Prepared command, prefix, and environment variables for launching the game on Linux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedLaunch {
    pub program: PathBuf,
    pub args_before_exe: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub kind: RuntimeKind,
    pub prefix: PathBuf,
    pub label: String,
}

pub type LaunchRuntime = PreparedLaunch;

/// Returned when no suitable Proton, ULWGL, or Wine runtime could be found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeMissing {
    pub searched: Vec<PathBuf>,
}

impl std::fmt::Display for RuntimeMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "no Linux compatibility runtime (Proton or Wine) found after checking {} location(s)",
            self.searched.len()
        )
    }
}

impl std::error::Error for RuntimeMissing {}

/// Merges `winmm=n,b` into an existing `WINEDLLOVERRIDES` value so the runtime loader `winmm.dll`
/// beside the game executable is loaded before Wine's built-in `winmm`.
#[must_use]
pub fn merge_dll_overrides(existing: Option<&str>) -> String {
    let Some(raw) = existing.map(str::trim).filter(|s| !s.is_empty()) else {
        return "winmm=n,b".to_owned();
    };
    let parts: Vec<&str> = raw
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();
    if parts.contains(&"winmm=n,b") {
        return parts.join(";");
    }
    let mut kept: Vec<&str> = parts
        .into_iter()
        .filter(|entry| {
            let key = entry.split('=').next().unwrap_or("").trim();
            !key.eq_ignore_ascii_case("winmm")
        })
        .collect();
    kept.push("winmm=n,b");
    kept.join(";")
}

fn resolve_proton_candidate(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    let script = path.join("proton");
    script.is_file().then_some(script)
}

fn steam_roots(home: &Path) -> [PathBuf; 4] {
    [
        home.join(".local").join("share").join("Steam"),
        home.join(".steam").join("steam"),
        home.join(".steam").join("root"),
        home.join(".var")
            .join("app")
            .join("com.valvesoftware.Steam")
            .join(".local")
            .join("share")
            .join("Steam"),
    ]
}

fn scan_proton_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    dirs
}

/// Searches `env` for an installed Proton, ULWGL (`umu-run`), or Wine runtime.
#[must_use]
pub fn detect_linux_runtime(env: &Env) -> Option<DetectedRuntime> {
    detect_with_searched(env).0
}

fn detect_with_searched(env: &Env) -> (Option<DetectedRuntime>, Vec<PathBuf>) {
    let mut searched = Vec::new();

    for var in ["DAUNTLESS_PROTON", "PROTON_PATH"] {
        if let Some(val) = env.var(var).map(str::trim).filter(|s| !s.is_empty()) {
            let candidate = PathBuf::from(val);
            searched.push(candidate.clone());
            if let Some(binary) = resolve_proton_candidate(&candidate) {
                let label = binary
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("Proton")
                    .to_owned();
                return (
                    Some(DetectedRuntime {
                        kind: RuntimeKind::Proton,
                        binary,
                        label,
                        steam_root: env.home.as_deref().map(|h| h.join(".local/share/Steam")),
                    }),
                    searched,
                );
            }
        }
    }

    if let Some(val) = env.var("DAUNTLESS_WINE").map(str::trim).filter(|s| !s.is_empty()) {
        let candidate = PathBuf::from(val);
        searched.push(candidate.clone());
        if candidate.is_file() {
            return (
                Some(DetectedRuntime {
                    kind: RuntimeKind::Wine,
                    binary: candidate,
                    label: "Wine".to_owned(),
                    steam_root: None,
                }),
                searched,
            );
        }
    }

    if let Some(umu) = env.find_in_path("umu-run") {
        return (
            Some(DetectedRuntime {
                kind: RuntimeKind::Umu,
                binary: umu,
                label: "UMU".to_owned(),
                steam_root: None,
            }),
            searched,
        );
    }

    if let Some(home) = &env.home {
        let heroic_roots = [
            home.join(".config").join("heroic").join("tools"),
            home.join(".var")
                .join("app")
                .join("com.heroicgameslauncher.hgl")
                .join("config")
                .join("heroic")
                .join("tools"),
        ];
        for tools_root in &heroic_roots {
            let proton_root = tools_root.join("proton");
            searched.push(proton_root.clone());
            for dir in scan_proton_dirs(&proton_root) {
                if let Some(binary) = resolve_proton_candidate(&dir) {
                    let label = dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("Heroic Proton")
                        .to_owned();
                    return (
                        Some(DetectedRuntime {
                            kind: RuntimeKind::Proton,
                            binary,
                            label,
                            steam_root: Some(home.join(".local").join("share").join("Steam")),
                        }),
                        searched,
                    );
                }
            }
            let wine_root = tools_root.join("wine");
            searched.push(wine_root.clone());
            for dir in scan_proton_dirs(&wine_root) {
                for bin_name in ["wine64", "wine"] {
                    let candidate = dir.join("bin").join(bin_name);
                    if candidate.is_file() {
                        let label = dir
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("Heroic Wine")
                            .to_owned();
                        return (
                            Some(DetectedRuntime {
                                kind: RuntimeKind::Wine,
                                binary: candidate,
                                label,
                                steam_root: None,
                            }),
                            searched,
                        );
                    }
                }
            }
        }

        for steam_root in steam_roots(home) {
            let custom_dir = steam_root.join("compatibilitytools.d");
            searched.push(custom_dir.clone());
            for dir in scan_proton_dirs(&custom_dir) {
                if let Some(binary) = resolve_proton_candidate(&dir) {
                    let label = dir.file_name().and_then(|n| n.to_str()).unwrap_or("Proton").to_owned();
                    return (
                        Some(DetectedRuntime {
                            kind: RuntimeKind::Proton,
                            binary,
                            label,
                            steam_root: Some(steam_root),
                        }),
                        searched,
                    );
                }
            }

            let common_dir = steam_root.join("steamapps").join("common");
            searched.push(common_dir.clone());
            for dir in scan_proton_dirs(&common_dir) {
                let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !name.starts_with("Proton") {
                    continue;
                }
                if let Some(binary) = resolve_proton_candidate(&dir) {
                    return (
                        Some(DetectedRuntime {
                            kind: RuntimeKind::Proton,
                            binary,
                            label: name.to_owned(),
                            steam_root: Some(steam_root),
                        }),
                        searched,
                    );
                }
            }
        }
    }

    for bin_name in ["wine64", "wine"] {
        for dir in &env.path {
            searched.push(dir.join(bin_name));
        }
        if let Some(binary) = env.find_in_path(bin_name) {
            return (
                Some(DetectedRuntime {
                    kind: RuntimeKind::Wine,
                    binary,
                    label: bin_name.to_owned(),
                    steam_root: None,
                }),
                searched,
            );
        }
    }

    (None, searched)
}

/// Prepares the wrapper command, arguments, prefix directory, and environment variables for launching `exe` on Linux.
pub fn prepare_linux_launch(env: &Env, data_dir: &Path, _exe: &Path) -> Result<PreparedLaunch, RuntimeMissing> {
    let (detected, searched) = detect_with_searched(env);
    let Some(runtime) = detected else {
        return Err(RuntimeMissing { searched });
    };

    let dll_overrides = merge_dll_overrides(env.var("WINEDLLOVERRIDES"));
    match runtime.kind {
        RuntimeKind::Proton => {
            let prefix = env
                .var("STEAM_COMPAT_DATA_PATH")
                .filter(|s| !s.trim().is_empty())
                .map_or_else(|| data_dir.join("compatdata"), PathBuf::from);
            let _ = std::fs::create_dir_all(&prefix);
            let steam_install = runtime
                .steam_root
                .or_else(|| env.home.as_ref().map(|h| h.join(".local/share/Steam")))
                .unwrap_or_else(|| data_dir.to_path_buf());
            let launch_env = vec![
                (
                    OsString::from("STEAM_COMPAT_DATA_PATH"),
                    prefix.as_os_str().to_os_string(),
                ),
                (
                    OsString::from("STEAM_COMPAT_CLIENT_INSTALL_PATH"),
                    steam_install.into_os_string(),
                ),
                (OsString::from("SteamAppId"), OsString::from("0")),
                (OsString::from("SteamGameId"), OsString::from("0")),
                (OsString::from("WINEDLLOVERRIDES"), OsString::from(dll_overrides)),
            ];
            Ok(PreparedLaunch {
                program: runtime.binary,
                args_before_exe: vec![OsString::from("waitforexitandrun")],
                env: launch_env,
                kind: RuntimeKind::Proton,
                prefix,
                label: runtime.label,
            })
        }
        RuntimeKind::Umu => {
            let prefix = env
                .var("WINEPREFIX")
                .filter(|s| !s.trim().is_empty())
                .map_or_else(|| data_dir.join("wineprefix"), PathBuf::from);
            let _ = std::fs::create_dir_all(&prefix);
            let launch_env = vec![
                (OsString::from("WINEPREFIX"), prefix.as_os_str().to_os_string()),
                (OsString::from("GAMEID"), OsString::from("umu-default")),
                (OsString::from("WINEDLLOVERRIDES"), OsString::from(dll_overrides)),
            ];
            Ok(PreparedLaunch {
                program: runtime.binary,
                args_before_exe: Vec::new(),
                env: launch_env,
                kind: RuntimeKind::Umu,
                prefix,
                label: runtime.label,
            })
        }
        RuntimeKind::Wine => {
            let prefix = env
                .var("WINEPREFIX")
                .filter(|s| !s.trim().is_empty())
                .map_or_else(|| data_dir.join("wineprefix"), PathBuf::from);
            let _ = std::fs::create_dir_all(&prefix);
            let launch_env = vec![
                (OsString::from("WINEPREFIX"), prefix.as_os_str().to_os_string()),
                (OsString::from("WINEDLLOVERRIDES"), OsString::from(dll_overrides)),
            ];
            Ok(PreparedLaunch {
                program: runtime.binary,
                args_before_exe: Vec::new(),
                env: launch_env,
                kind: RuntimeKind::Wine,
                prefix,
                label: runtime.label,
            })
        }
    }
}

/// System dependency categories that can be installed via the distribution package manager on Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    Wine,
    Tailscale,
    SecretTool,
}

/// Planned command for installing a system package on Linux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub display_command: String,
    pub needs_privilege_escalation: bool,
}

fn package_for(manager: &str, kind: DependencyKind) -> &'static str {
    match (manager, kind) {
        (_, DependencyKind::Wine) => "wine",
        (_, DependencyKind::Tailscale) => "tailscale",
        ("apt-get", DependencyKind::SecretTool) => "libsecret-tools",
        ("pacman" | "apk", DependencyKind::SecretTool) => "libsecret",
        (_, DependencyKind::SecretTool) => "libsecret",
    }
}

/// Builds an [`InstallPlan`] for `kind` using the package manager discovered in `env`.
#[must_use]
pub fn install_plan(env: &Env, kind: DependencyKind) -> Option<InstallPlan> {
    if env.flatpak {
        return None;
    }
    let managers: [(&str, &[&str]); 5] = [
        ("apt-get", &["install", "-y"]),
        ("dnf", &["install", "-y"]),
        ("pacman", &["-S", "--noconfirm"]),
        ("zypper", &["install", "-y"]),
        ("apk", &["add"]),
    ];

    let (mgr_name, mgr_path, mgr_flags) = managers
        .into_iter()
        .find_map(|(name, flags)| env.find_in_path(name).map(|path| (name, path, flags)))?;

    let pkg = package_for(mgr_name, kind);
    let mut base_args: Vec<String> = mgr_flags.iter().map(|s| (*s).to_owned()).collect();
    base_args.push(pkg.to_owned());

    let is_root = env.var("USER") == Some("root") || env.var("EUID") == Some("0");
    if is_root {
        let display_command = format!("{} {}", mgr_name, base_args.join(" "));
        return Some(InstallPlan {
            program: mgr_path,
            args: base_args,
            display_command,
            needs_privilege_escalation: false,
        });
    }

    let escalator = env.find_in_path("pkexec").or_else(|| env.find_in_path("sudo"))?;
    let esc_name = escalator.file_name().and_then(|n| n.to_str()).unwrap_or("sudo");
    let mut args = Vec::with_capacity(base_args.len() + 1);
    args.push(mgr_path.to_string_lossy().into_owned());
    args.extend(base_args.iter().cloned());
    let display_command = format!("{esc_name} {mgr_name} {}", base_args.join(" "));
    Some(InstallPlan {
        program: escalator,
        args,
        display_command,
        needs_privilege_escalation: true,
    })
}

/// Runs an [`InstallPlan`] as a child process and returns an error string if it exits non-zero.
pub async fn run_install(plan: &InstallPlan) -> Result<(), String> {
    let output = tokio::process::Command::new(&plan.program)
        .args(&plan.args)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("");
        if detail.is_empty() {
            Err(format!("{} exited with {}", plan.display_command, output.status))
        } else {
            Err(detail.trim().to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dll_overrides_append_winmm_and_preserve_existing_entries() {
        assert_eq!(merge_dll_overrides(None), "winmm=n,b");
        assert_eq!(merge_dll_overrides(Some("")), "winmm=n,b");
        assert_eq!(merge_dll_overrides(Some("d3d11=n;dxgi=n")), "d3d11=n;dxgi=n;winmm=n,b");
        assert_eq!(merge_dll_overrides(Some("winmm=b;dxgi=n")), "dxgi=n;winmm=n,b");
    }

    #[test]
    fn detects_proton_in_steam_and_prepares_waitforexitandrun_launch() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let proton_dir = home
            .join(".local/share/Steam/steamapps/common")
            .join("Proton - Experimental");
        std::fs::create_dir_all(&proton_dir).unwrap();
        let proton_bin = proton_dir.join("proton");
        std::fs::write(&proton_bin, "#!/bin/sh\n").unwrap();

        let env = Env {
            home: Some(home.clone()),
            path: Vec::new(),
            flatpak: false,
            vars: HashMap::new(),
        };
        let detected = detect_linux_runtime(&env).unwrap();
        assert_eq!(detected.kind, RuntimeKind::Proton);
        assert_eq!(detected.binary, proton_bin);

        let data_dir = tmp.path().join("userdata");
        let exe = tmp.path().join("Dauntless-Win64-Shipping.exe");
        let prepared = prepare_linux_launch(&env, &data_dir, &exe).unwrap();
        assert_eq!(prepared.kind, RuntimeKind::Proton);
        assert_eq!(prepared.args_before_exe, vec![OsString::from("waitforexitandrun")]);
        assert!(
            prepared
                .env
                .iter()
                .any(|(k, v)| { k == "WINEDLLOVERRIDES" && v == "winmm=n,b" })
        );
    }

    #[test]
    fn detects_wine_on_path_and_builds_install_plan() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let wine = bin.join("wine64");
        let apt = bin.join("apt-get");
        let pkexec = bin.join("pkexec");
        std::fs::write(&wine, "").unwrap();
        std::fs::write(&apt, "").unwrap();
        std::fs::write(&pkexec, "").unwrap();

        let env = Env {
            home: None,
            path: vec![bin],
            flatpak: false,
            vars: HashMap::new(),
        };
        let detected = detect_linux_runtime(&env).unwrap();
        assert_eq!(detected.kind, RuntimeKind::Wine);
        assert_eq!(detected.binary, wine);

        let plan = install_plan(&env, DependencyKind::SecretTool).unwrap();
        assert_eq!(plan.program, pkexec);
        assert!(plan.needs_privilege_escalation);
        assert!(plan.display_command.contains("apt-get install -y libsecret-tools"));
    }
}
