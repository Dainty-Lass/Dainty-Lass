// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Proton and Wine on Linux: finding one, its prefix and environment, and installing Wine (spec §2.7.13).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::platform::log;

/// A Proton to use: a path (`~/` expanded) or a name on `PATH`.
pub const PROTON_VAR: &str = "DAUNTLESS_REVIVED_PROTON";
/// A Wine to use: a path (`~/` expanded) or a name on `PATH`.
pub const WINE_VAR: &str = "DAUNTLESS_REVIVED_WINE";
/// `0` turns off installing Wine when none is found.
pub const AUTO_INSTALL_VAR: &str = "DAUNTLESS_REVIVED_AUTO_INSTALL";
/// The override that makes the runtime loader's `dxgi.dll` beside the game load before Wine's.
pub const DXGI_OVERRIDE: &str = "dxgi=n,b";

const PREFIX_INIT_TIMEOUT: Duration = Duration::from_secs(120);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const SYSTEM_DIRS: [&str; 5] = ["/usr/bin", "/usr/sbin", "/bin", "/sbin", "/usr/local/bin"];

/// The environment and folders the runtimes are looked up in.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// `HOME`, or the OS's home folder.
    pub home: Option<PathBuf>,
    /// `PATH`'s folders, in order.
    pub path: Vec<PathBuf>,
    /// Every environment variable, sorted.
    pub vars: BTreeMap<String, String>,
    /// The process runs as root, so package managers need no `pkexec`.
    pub is_root: bool,
    /// Where package managers and `pkexec` are looked for after `PATH`.
    pub system_dirs: Vec<PathBuf>,
}

impl Env {
    /// A variable's value.
    #[must_use]
    pub fn var(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }

    fn non_empty(&self, name: &str) -> Option<&str> {
        self.var(name).filter(|v| !v.is_empty())
    }

    /// The first executable file named `binary` in `PATH`.
    #[must_use]
    pub fn find_in_path(&self, binary: &str) -> Option<PathBuf> {
        self.path.iter().map(|dir| dir.join(binary)).find(|c| is_executable(c))
    }

    fn expand_home(&self, value: &str) -> PathBuf {
        match (value.strip_prefix("~/"), &self.home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => PathBuf::from(value),
        }
    }

    /// A configured runner: a path when it names a folder, else a name looked up in `PATH`.
    fn configured(&self, name: &str) -> Option<(PathBuf, bool)> {
        let value = self.non_empty(name)?.trim();
        if Path::new(value).components().count() > 1 {
            let path = self.expand_home(value);
            is_executable(&path).then_some((path, false))
        } else {
            self.find_in_path(value).map(|p| (p, true))
        }
    }
}

/// The current process's environment.
#[must_use]
pub fn current_env() -> Env {
    let vars: BTreeMap<String, String> = std::env::vars().collect();
    let home = vars
        .get("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir);
    let path = std::env::var_os("PATH")
        .map(|raw| {
            std::env::split_paths(&raw)
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    Env {
        home,
        path,
        vars,
        is_root: effective_uid_is_root(),
        system_dirs: SYSTEM_DIRS.iter().map(PathBuf::from).collect(),
    }
}

// The effective uid without `unsafe`: the second field of `/proc/self/status`'s `Uid:` line.
fn effective_uid_is_root() -> bool {
    std::fs::read_to_string("/proc/self/status").is_ok_and(|status| {
        status
            .lines()
            .find_map(|l| l.strip_prefix("Uid:"))
            .and_then(|ids| ids.split_whitespace().nth(1))
            == Some("0")
    })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Natural, case-insensitive order: `Proton 10.0` after `Proton 9.0`.
#[must_use]
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(d) = it.next_if(char::is_ascii_digit) {
                        digits.push(d);
                    }
                    digits.trim_start_matches('0').to_owned()
                };
                let (da, db) = (take(&mut a), take(&mut b));
                let order = da.len().cmp(&db.len()).then_with(|| da.cmp(&db));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Subfolders of `dir`, newest name first.
fn newest_first(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort_by(|a, b| natural_cmp(&folder_name(b), &folder_name(a)));
    dirs
}

/// The family of a runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// Valve's Proton, run through its `proton` script.
    Proton,
    /// Plain Wine.
    Wine,
}

/// The runner family.
pub type RunnerKind = RuntimeKind;

/// A runner found on this PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRuntime {
    /// Which family.
    pub kind: RuntimeKind,
    /// The executable to run: the `proton` script, or `wine64`/`wine`.
    pub binary: PathBuf,
    /// `Proton (<folder>)`, `Wine (<folder>)` or `Wine (system)`.
    pub label: String,
    /// The Steam install a Proton belongs to.
    pub steam_root: Option<PathBuf>,
}

/// Steam's install folders, native then Flatpak.
#[must_use]
pub fn steam_roots(home: &Path) -> [PathBuf; 5] {
    let flatpak = home.join(".var").join("app").join("com.valvesoftware.Steam");
    [
        home.join(".local").join("share").join("Steam"),
        home.join(".steam").join("root"),
        home.join(".steam").join("steam"),
        flatpak.join(".local").join("share").join("Steam"),
        flatpak.join("data").join("Steam"),
    ]
}

/// The Steam install above a `proton` script: the parent of `compatibilitytools.d` or of `steamapps/common`.
#[must_use]
pub fn steam_root_of(proton: &Path) -> Option<PathBuf> {
    let mut ancestors = proton.ancestors().skip(1);
    while let Some(dir) = ancestors.next() {
        match dir.file_name().and_then(|n| n.to_str()) {
            Some("compatibilitytools.d") => return dir.parent().map(Path::to_path_buf),
            Some("common") if dir.parent().is_some_and(|p| p.ends_with("steamapps")) => {
                return ancestors.nth(1).map(Path::to_path_buf);
            }
            _ => {}
        }
    }
    None
}

fn proton(binary: PathBuf, steam_root: Option<PathBuf>) -> DetectedRuntime {
    let label = format!("Proton ({})", binary.parent().map(folder_name).unwrap_or_default());
    DetectedRuntime {
        kind: RuntimeKind::Proton,
        binary,
        label,
        steam_root,
    }
}

fn wine(binary: PathBuf, system: bool) -> DetectedRuntime {
    let label = if system {
        "Wine (system)".to_owned()
    } else {
        // `<runner>/bin/wine` is named after the runner.
        let parent = binary.parent();
        let named = match parent {
            Some(p) if p.ends_with("bin") => p.parent(),
            other => other,
        };
        format!("Wine ({})", named.map(folder_name).unwrap_or_default())
    };
    DetectedRuntime {
        kind: RuntimeKind::Wine,
        binary,
        label,
        steam_root: None,
    }
}

fn search_proton(env: &Env, searched: &mut Vec<PathBuf>) -> Option<DetectedRuntime> {
    if let Some((binary, _)) = env.configured(PROTON_VAR) {
        let root = steam_root_of(&binary);
        return Some(proton(binary, root));
    }
    let home = env.home.as_deref()?;
    let mut found: Vec<(String, PathBuf, PathBuf)> = Vec::new();
    for root in steam_roots(home) {
        let tools = root.join("compatibilitytools.d");
        let common = root.join("steamapps").join("common");
        searched.push(tools.clone());
        searched.push(common.clone());
        let in_common = newest_first(&common).into_iter().filter(|dir| {
            let name = folder_name(dir).to_lowercase();
            name.starts_with("proton") || name.starts_with("ge-proton")
        });
        for dir in newest_first(&tools).into_iter().chain(in_common) {
            let script = dir.join("proton");
            if script.is_file() {
                found.push((folder_name(&dir), script, root.clone()));
            }
        }
    }
    found.sort_by(|a, b| natural_cmp(&b.0, &a.0));
    found
        .into_iter()
        .next()
        .map(|(_, script, root)| proton(script, Some(root)))
}

fn search_wine(env: &Env, searched: &mut Vec<PathBuf>) -> Option<DetectedRuntime> {
    if let Some((binary, on_path)) = env.configured(WINE_VAR) {
        return Some(wine(binary, on_path));
    }
    for name in ["wine64", "wine"] {
        searched.extend(env.path.iter().map(|dir| dir.join(name)));
        if let Some(binary) = env.find_in_path(name) {
            return Some(wine(binary, true));
        }
    }
    let runners = env.home.as_deref()?.join(".local/share/lutris/runners/wine");
    searched.push(runners.clone());
    newest_first(&runners).into_iter().find_map(|runner| {
        ["wine64", "wine"]
            .into_iter()
            .map(|name| runner.join("bin").join(name))
            .find(|c| is_executable(c))
            .map(|binary| wine(binary, false))
    })
}

fn detect_with_searched(env: &Env) -> (Option<DetectedRuntime>, Vec<PathBuf>) {
    let mut searched = Vec::new();
    let only_wine = env.non_empty(WINE_VAR).is_some() && env.non_empty(PROTON_VAR).is_none();
    let found = (!only_wine)
        .then(|| search_proton(env, &mut searched))
        .flatten()
        .or_else(|| search_wine(env, &mut searched));
    (found, searched)
}

/// Proton unless only a Wine is configured, else Wine.
#[must_use]
pub fn detect_linux_runtime(env: &Env) -> Option<DetectedRuntime> {
    detect_with_searched(env).0
}

/// `existing`'s entries without a `dxgi=` one, then [`DXGI_OVERRIDE`].
#[must_use]
pub fn merge_dll_overrides(existing: Option<&str>) -> String {
    let mut kept: Vec<&str> = existing
        .unwrap_or("")
        .split(';')
        .map(str::trim)
        .filter(|e| !e.is_empty() && !e.to_ascii_lowercase().starts_with("dxgi="))
        .collect();
    kept.push(DXGI_OVERRIDE);
    kept.join(";")
}

/// On a Wayland session without `XAUTHORITY`, the newest `.mutter-Xwaylandauth.*` in `XDG_RUNTIME_DIR`, so the
/// game's X client can reach XWayland.
#[must_use]
pub fn xwayland_authority(env: &Env) -> Option<PathBuf> {
    let wayland = env.non_empty("WAYLAND_DISPLAY").is_some() || env.var("XDG_SESSION_TYPE") == Some("wayland");
    if !wayland || env.non_empty("XAUTHORITY").is_some() {
        return None;
    }
    let runtime = env.non_empty("XDG_RUNTIME_DIR")?;
    std::fs::read_dir(runtime)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".mutter-Xwaylandauth."))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(time, _)| *time)
        .map(|(_, path)| path)
}

/// A prefix user name: `\/:*?"<>|` become `_`, at most 64 characters, `wineuser` when empty.
#[must_use]
pub fn sanitize_user(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .take(64)
        .collect();
    if clean.is_empty() { "wineuser".to_owned() } else { clean }
}

/// Everything needed to start the game through a runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedLaunch {
    /// The runner executable.
    pub program: PathBuf,
    /// Arguments before the game executable (`run` for Proton).
    pub args_before_exe: Vec<OsString>,
    /// The runner's variables, added to the game's environment.
    pub env: BTreeMap<String, String>,
    /// Which family.
    pub kind: RuntimeKind,
    /// The Wine prefix.
    pub prefix: PathBuf,
    /// Name shown to the player.
    pub label: String,
    /// The game's config folder inside the prefix.
    pub config_dir: PathBuf,
}

/// A launch ready to run.
pub type LaunchRuntime = PreparedLaunch;

/// Neither Proton nor Wine was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeMissing {
    /// Every place searched, for the error.
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

/// The launch for `runtime`, with its prefix under `<user_data>/compat`. Creates nothing.
#[must_use]
pub fn launch_for(env: &Env, user_data: &Path, runtime: DetectedRuntime) -> PreparedLaunch {
    let compat = user_data.join("compat");
    let mut vars = BTreeMap::new();
    vars.insert(
        "WINEDLLOVERRIDES".to_owned(),
        merge_dll_overrides(env.var("WINEDLLOVERRIDES")),
    );
    if let Some(auth) = xwayland_authority(env) {
        vars.insert("XAUTHORITY".to_owned(), auth.to_string_lossy().into_owned());
    }
    let (prefix, user, args) = match runtime.kind {
        RuntimeKind::Proton => {
            let data = compat.join("proton");
            vars.insert("STEAM_COMPAT_DATA_PATH".to_owned(), data.to_string_lossy().into_owned());
            if let Some(root) = &runtime.steam_root {
                vars.insert(
                    "STEAM_COMPAT_CLIENT_INSTALL_PATH".to_owned(),
                    root.to_string_lossy().into_owned(),
                );
            }
            vars.insert("STEAM_COMPAT_APP_ID".to_owned(), "0".to_owned());
            vars.insert("PROTON_LOG".to_owned(), "0".to_owned());
            (data.join("pfx"), "steamuser".to_owned(), vec![OsString::from("run")])
        }
        RuntimeKind::Wine => {
            let prefix = compat.join("wine");
            vars.insert("WINEPREFIX".to_owned(), prefix.to_string_lossy().into_owned());
            if env.var("WINEDEBUG").is_none() {
                vars.insert("WINEDEBUG".to_owned(), "-all".to_owned());
            }
            let user = env.non_empty("USER").or_else(|| env.non_empty("LOGNAME")).unwrap_or("");
            (prefix, sanitize_user(user), Vec::new())
        }
    };
    let config_dir = prefix
        .join("drive_c/users")
        .join(user)
        .join("AppData/Local/Archon/Saved/Config/WindowsClient");
    PreparedLaunch {
        program: runtime.binary,
        args_before_exe: args,
        env: vars,
        kind: runtime.kind,
        prefix,
        label: runtime.label,
        config_dir,
    }
}

/// Finds a runner and builds its launch, without installing or touching the prefix.
pub fn prepare_linux_launch(env: &Env, user_data: &Path) -> Result<PreparedLaunch, RuntimeMissing> {
    let (found, searched) = detect_with_searched(env);
    found
        .map(|runtime| launch_for(env, user_data, runtime))
        .ok_or(RuntimeMissing { searched })
}

/// How [`prepare`] may act.
#[derive(Debug, Clone, Copy, Default)]
pub struct PrepareOptions {
    /// A development build never installs packages.
    pub development_build: bool,
}

/// Finds a runner (installing Wine when none is found and auto-install is on), then initialises its prefix
/// unless `drive_c` exists. A failed initialisation only warns.
pub async fn prepare(env: &Env, user_data: &Path, options: PrepareOptions) -> Result<PreparedLaunch, RuntimeMissing> {
    let (mut found, mut searched) = detect_with_searched(env);
    let auto_install = !options.development_build && env.var(AUTO_INSTALL_VAR) != Some("0");
    if found.is_none()
        && auto_install
        && let Some(plan) = install_plan(env, DependencyKind::Wine)
    {
        log::info(format!(
            "No Proton or Wine found; installing Wine: {}",
            plan.display_command
        ));
        if let Err(e) = run_install(&plan, &env.vars).await {
            log::warn(format!("Installing Wine failed: {e}"));
        }
        (found, searched) = detect_with_searched(env);
    }
    let runtime = found.ok_or(RuntimeMissing { searched })?;
    let launch = launch_for(env, user_data, runtime);
    if !launch.prefix.join("drive_c").is_dir()
        && let Err(e) = init_prefix(env, &launch).await
    {
        log::warn(format!("Initialising the {} prefix failed: {e}", launch.label));
    }
    Ok(launch)
}

async fn init_prefix(env: &Env, launch: &PreparedLaunch) -> Result<(), String> {
    std::fs::create_dir_all(&launch.prefix).map_err(|e| e.to_string())?;
    let (program, args): (PathBuf, &[&str]) = match launch.kind {
        RuntimeKind::Proton => (launch.program.clone(), &["run", "cmd", "/c", "exit"]),
        RuntimeKind::Wine => {
            let beside = launch.program.parent().map(|d| d.join("wineboot"));
            match beside
                .filter(|p| is_executable(p))
                .or_else(|| env.find_in_path("wineboot"))
            {
                Some(wineboot) => (wineboot, &["-u"]),
                None => (launch.program.clone(), &["wineboot", "-u"]),
            }
        }
    };
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(args)
        .env_clear()
        .envs(&env.vars)
        .envs(&launch.env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let status = tokio::time::timeout(PREFIX_INIT_TIMEOUT, cmd.status())
        .await
        .map_err(|_| "timed out".to_owned())?
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited with {status}", program.display()))
    }
}

/// What a package-manager plan installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    /// Wine, to run the game.
    Wine,
    /// A Secret Service (GNOME Keyring and libsecret), to keep the account key.
    SecureStorage,
}

/// An install command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    /// The package manager, or `pkexec` running it.
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
    /// The command as logged.
    pub display_command: String,
    /// The command runs through `pkexec`.
    pub needs_privilege_escalation: bool,
}

fn manager_args(manager: &str, kind: DependencyKind) -> Vec<&'static str> {
    let (flags, wine, storage): (&[&str], &[&str], &[&str]) = match manager {
        "apt-get" => (&["install", "-y"], &["wine64"], &["gnome-keyring", "libsecret-1-0"]),
        "dnf" => (&["install", "-y"], &["wine"], &["gnome-keyring", "libsecret"]),
        "zypper" => (
            &["--non-interactive", "install"],
            &["wine"],
            &["gnome-keyring", "libsecret"],
        ),
        "pacman" => (
            &["-S", "--needed", "--noconfirm"],
            &["wine"],
            &["gnome-keyring", "libsecret"],
        ),
        _ => (&["-Sy"], &["wine"], &["gnome-keyring", "libsecret"]),
    };
    let packages = match kind {
        DependencyKind::Wine => wine,
        DependencyKind::SecureStorage => storage,
    };
    flags.iter().chain(packages).copied().collect()
}

fn find_system_tool(env: &Env, name: &str) -> Option<PathBuf> {
    env.find_in_path(name).or_else(|| {
        env.system_dirs
            .iter()
            .map(|dir| dir.join(name))
            .find(|c| is_executable(c))
    })
}

/// The install command for `kind`: the first package manager found, run directly as root, else through `pkexec`
/// (none → no plan).
#[must_use]
pub fn install_plan(env: &Env, kind: DependencyKind) -> Option<InstallPlan> {
    let (manager, path) = ["apt-get", "dnf", "zypper", "pacman", "xbps-install"]
        .into_iter()
        .find_map(|name| find_system_tool(env, name).map(|p| (name, p)))?;
    let args: Vec<String> = manager_args(manager, kind).into_iter().map(str::to_owned).collect();
    let shown = format!("{} {}", path.display(), args.join(" "));
    if env.is_root {
        return Some(InstallPlan {
            program: path,
            args,
            display_command: shown,
            needs_privilege_escalation: false,
        });
    }
    let pkexec = find_system_tool(env, "pkexec")?;
    let mut escalated = vec![path.to_string_lossy().into_owned()];
    escalated.extend(args);
    Some(InstallPlan {
        display_command: format!("{} {shown}", pkexec.display()),
        program: pkexec,
        args: escalated,
        needs_privilege_escalation: true,
    })
}

/// Runs `plan` with the environment cleared to `env`, for at most 15 minutes.
pub async fn run_install(plan: &InstallPlan, env: &BTreeMap<String, String>) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new(&plan.program);
    cmd.args(&plan.args)
        .env_clear()
        .envs(env)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(INSTALL_TIMEOUT, cmd.output())
        .await
        .map_err(|_| format!("{} timed out", plan.display_command))?
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    match stderr.lines().rfind(|l| !l.trim().is_empty()) {
        Some(detail) => Err(detail.trim().to_owned()),
        None => Err(format!("{} exited with {}", plan.display_command, output.status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_exe(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn env_with(home: Option<&Path>, path: Vec<PathBuf>, vars: &[(&str, &str)]) -> Env {
        Env {
            home: home.map(Path::to_path_buf),
            path,
            vars: vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
            is_root: false,
            system_dirs: Vec::new(),
        }
    }

    #[test]
    fn dll_overrides_replace_dxgi() {
        assert_eq!(merge_dll_overrides(None), "dxgi=n,b");
        assert_eq!(merge_dll_overrides(Some("")), "dxgi=n,b");
        assert_eq!(
            merge_dll_overrides(Some("d3d11=n;winmm=n,b")),
            "d3d11=n;winmm=n,b;dxgi=n,b"
        );
        assert_eq!(
            merge_dll_overrides(Some("dxgi=b; d3d11=n ;;DXGI=n")),
            "d3d11=n;dxgi=n,b"
        );
    }

    #[test]
    fn natural_order_is_numeric_and_case_insensitive() {
        let mut names = vec![
            "Proton 9.0",
            "proton 10.0",
            "GE-Proton9-2",
            "GE-Proton10-1",
            "Proton 9.0 (Beta)",
        ];
        names.sort_by(|a, b| natural_cmp(b, a));
        assert_eq!(
            names,
            [
                "proton 10.0",
                "Proton 9.0 (Beta)",
                "Proton 9.0",
                "GE-Proton10-1",
                "GE-Proton9-2"
            ]
        );
        assert_eq!(natural_cmp("a007", "A7"), Ordering::Equal);
    }

    #[test]
    fn newest_proton_across_steam_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let steam = home.join(".local/share/Steam");
        make_exe(&steam.join("steamapps/common/Proton 9.0/proton"));
        make_exe(&steam.join("steamapps/common/Half-Life/proton"));
        let flatpak = home.join(".var/app/com.valvesoftware.Steam/data/Steam");
        make_exe(&flatpak.join("compatibilitytools.d/GE-Proton10-1/proton"));
        make_exe(&steam.join("steamapps/common/Proton 10.0/proton"));

        let found = detect_linux_runtime(&env_with(Some(home), Vec::new(), &[])).unwrap();
        assert_eq!(found.kind, RuntimeKind::Proton);
        assert_eq!(found.label, "Proton (Proton 10.0)");
        assert_eq!(found.steam_root.as_deref(), Some(steam.as_path()));
        assert_eq!(
            steam_root_of(&flatpak.join("compatibilitytools.d/GE-Proton10-1/proton")),
            Some(flatpak)
        );
    }

    #[test]
    fn proton_launch_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let steam = tmp.path().join(".steam").join("root");
        make_exe(&steam.join("compatibilitytools.d/GE-Proton10-1/proton"));
        let env = env_with(Some(tmp.path()), Vec::new(), &[("WINEDLLOVERRIDES", "dxgi=b;d3d11=n")]);
        let ud = tmp.path().join("ud");
        let launch = prepare_linux_launch(&env, &ud).unwrap();
        assert_eq!(launch.args_before_exe, [OsString::from("run")]);
        assert_eq!(launch.prefix, ud.join("compat/proton/pfx"));
        assert_eq!(
            launch.config_dir,
            launch
                .prefix
                .join("drive_c/users/steamuser/AppData/Local/Archon/Saved/Config/WindowsClient")
        );
        let expected: BTreeMap<String, String> = [
            ("PROTON_LOG", "0".to_owned()),
            ("STEAM_COMPAT_APP_ID", "0".to_owned()),
            ("STEAM_COMPAT_CLIENT_INSTALL_PATH", steam.to_string_lossy().into_owned()),
            (
                "STEAM_COMPAT_DATA_PATH",
                ud.join("compat").join("proton").to_string_lossy().into_owned(),
            ),
            ("WINEDLLOVERRIDES", "d3d11=n;dxgi=n,b".to_owned()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        assert_eq!(launch.env, expected);
    }

    #[test]
    fn configured_wine_alone_skips_proton() {
        let tmp = tempfile::tempdir().unwrap();
        make_exe(&tmp.path().join(".local/share/Steam/steamapps/common/Proton 9.0/proton"));
        let wine_bin = tmp.path().join("opt/wine-staging/bin/wine");
        make_exe(&wine_bin);
        let env = env_with(
            Some(tmp.path()),
            Vec::new(),
            &[(WINE_VAR, "~/opt/wine-staging/bin/wine")],
        );
        let found = detect_linux_runtime(&env).unwrap();
        assert_eq!(
            (found.kind, found.binary, found.label.as_str()),
            (RuntimeKind::Wine, wine_bin, "Wine (wine-staging)")
        );

        // With a Proton configured as well, Proton wins.
        let proton = tmp.path().join("p/proton");
        make_exe(&proton);
        let mut env = env;
        env.vars
            .insert(PROTON_VAR.to_owned(), proton.to_string_lossy().into_owned());
        assert_eq!(detect_linux_runtime(&env).unwrap().binary, proton);
    }

    #[test]
    fn wine_from_path_then_lutris() {
        let tmp = tempfile::tempdir().unwrap();
        let lutris = tmp.path().join(".local/share/lutris/runners/wine");
        make_exe(&lutris.join("lutris-7.2/bin/wine"));
        make_exe(&lutris.join("lutris-GE-8.10/bin/wine64"));
        let env = env_with(Some(tmp.path()), Vec::new(), &[("USER", "a:b/c")]);
        let found = detect_linux_runtime(&env).unwrap();
        assert_eq!(found.label, "Wine (lutris-GE-8.10)");
        let launch = launch_for(&env, Path::new("/ud"), found);
        assert_eq!(launch.env.get("WINEDEBUG").map(String::as_str), Some("-all"));
        assert_eq!(launch.prefix, Path::new("/ud/compat/wine"));
        assert!(
            launch
                .config_dir
                .starts_with("/ud/compat/wine/drive_c/users/a_b_c/AppData")
        );

        let bin = tmp.path().join("bin");
        make_exe(&bin.join("wine"));
        let env = env_with(Some(tmp.path()), vec![bin.clone()], &[("WINEDEBUG", "")]);
        let found = detect_linux_runtime(&env).unwrap();
        assert_eq!(
            (found.binary, found.label.as_str()),
            (bin.join("wine"), "Wine (system)")
        );
        let launch = prepare_linux_launch(&env, Path::new("/ud")).unwrap();
        assert!(!launch.env.contains_key("WINEDEBUG"));
    }

    #[test]
    fn nothing_found_lists_searched_places() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = prepare_linux_launch(&env_with(Some(tmp.path()), Vec::new(), &[]), tmp.path()).unwrap_err();
        assert!(missing.searched.len() >= 10);
    }

    #[test]
    fn user_names_are_sanitised() {
        assert_eq!(sanitize_user(r#"a\b/c:d*e?f"g<h>i|j"#), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize_user(""), "wineuser");
        assert_eq!(sanitize_user(&"x".repeat(80)).len(), 64);
    }

    #[test]
    fn xwayland_authority_only_on_wayland_without_xauthority() {
        let tmp = tempfile::tempdir().unwrap();
        let run = tmp.path().to_string_lossy().into_owned();
        std::fs::write(tmp.path().join(".mutter-Xwaylandauth.ABC"), "").unwrap();
        std::fs::write(tmp.path().join("other"), "").unwrap();
        let env = env_with(
            None,
            Vec::new(),
            &[("WAYLAND_DISPLAY", "wayland-0"), ("XDG_RUNTIME_DIR", &run)],
        );
        assert_eq!(
            xwayland_authority(&env),
            Some(tmp.path().join(".mutter-Xwaylandauth.ABC"))
        );
        let env = env_with(
            None,
            Vec::new(),
            &[
                ("WAYLAND_DISPLAY", "wayland-0"),
                ("XDG_RUNTIME_DIR", &run),
                ("XAUTHORITY", "/x"),
            ],
        );
        assert_eq!(xwayland_authority(&env), None);
        assert_eq!(
            xwayland_authority(&env_with(None, Vec::new(), &[("XDG_RUNTIME_DIR", &run)])),
            None
        );
    }

    #[test]
    fn install_plans_by_manager_and_privilege() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        make_exe(&bin.join("zypper"));
        make_exe(&bin.join("pacman"));
        let mut env = env_with(None, vec![bin.clone()], &[]);
        env.is_root = true;
        let plan = install_plan(&env, DependencyKind::Wine).unwrap();
        assert_eq!(plan.program, bin.join("zypper"));
        assert_eq!(plan.args, ["--non-interactive", "install", "wine"]);
        assert!(!plan.needs_privilege_escalation);

        make_exe(&bin.join("apt-get"));
        make_exe(&bin.join("pkexec"));
        env.is_root = false;
        let plan = install_plan(&env, DependencyKind::SecureStorage).unwrap();
        assert_eq!(plan.program, bin.join("pkexec"));
        assert_eq!(
            plan.args,
            [
                bin.join("apt-get").to_string_lossy().as_ref(),
                "install",
                "-y",
                "gnome-keyring",
                "libsecret-1-0"
            ]
        );
        assert!(plan.needs_privilege_escalation);
        assert_eq!(
            manager_args("pacman", DependencyKind::Wine),
            ["-S", "--needed", "--noconfirm", "wine"]
        );
        assert_eq!(
            manager_args("xbps-install", DependencyKind::SecureStorage),
            ["-Sy", "gnome-keyring", "libsecret"]
        );
        assert_eq!(manager_args("dnf", DependencyKind::Wine), ["install", "-y", "wine"]);
    }
}
