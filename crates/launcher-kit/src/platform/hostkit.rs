// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Host workspace inspection, toolchain discovery (`vswhere`, MSBuild, Cargo), Dumper-7 discovery,
//! SDK import, and logged build execution (Spec §2.3).

pub mod parse;

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, SecondsFormat, Utc};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::bridge::types::{
    GameDataState, HostBuilds, HostDump, HostExports, HostGameData, HostKit, HostMultipliers, HostSdk,
    HostServerConfig, HostServerMode, HostToolchain, HostUpstream,
};
use crate::platform::system::run_hidden;

pub use parse::{
    EXPORT_FLAGS, INVITE_LINK_PREFIXES, InviteLine, MAX_MULTIPLIER, MAX_SERVER_NAME_LEN, MIN_MULTIPLIER,
    PATCH_SENTINEL, PINNED_MP_COMMIT, SERVER_PORT, ServerConfig, ServerConfigValue, ServerMode, ServerMultipliersValue,
    UpdateCheck, UpdateDone, UpdateSummary, classify_game_data, clean_invite_link, clean_line, cut, failure_line,
    invite_line, is_patched_source, last_json, last_output_line, pack_written, parse_env_multipliers,
    parse_update_output, platform_toolset, published_version, round_multiplier, sanitize_output_line,
    server_config_toml, set_env_multipliers, update_check, update_done, update_failure, vswhere_install,
    vswhere_msbuild,
};

pub const RUNTIME_FILE_NAMES: [&str; 2] = ["Rustic-Isles.dll", "winmm.dll"];

pub const BACKEND_GAME_DATA: [&str; 7] = [
    "progression_config.json",
    "slayers_path.json",
    "slayers_path_definitions.json",
    "ladyluck_store.json",
    "platinum_store.json",
    "reward_cache_store.json",
    "inventory_storage_policy.json",
];

pub const DIRECTOR_GAME_DATA: [&str; 7] = [
    "player_hunts_table.json",
    "matchmaker_hunts_table.json",
    "arena_easy_matchmaker_hunts.json",
    "arena_hard_matchmaker_hunts.json",
    "arena_elite_matchmaker_hunts.json",
    "arena_hard_matchmaker_hunts_new.json",
    "arena_elite_matchmaker_hunts_new.json",
];

pub const DUMPER_OUTPUT_ROOT: &str = r"C:\Dumper-7";

pub const HOST_BINARY: &str = if cfg!(windows) {
    "reforged-host.exe"
} else {
    "reforged-host"
};

const EXEC_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_EXEC_BYTES: usize = 4 * 1024 * 1024;
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_DUMP_DIRS: usize = 200;

/// Canonical filesystem layout of a host workspace (submodule or flat checkout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePaths {
    pub root: PathBuf,
    pub runtime_source: PathBuf,
    pub sdk_header: PathBuf,
    pub sdk_dir: PathBuf,
    pub runtime_solution: PathBuf,
    pub runtime_dll: PathBuf,
    pub loader_dir: PathBuf,
    pub loader_dll: PathBuf,
    pub exporter_project: PathBuf,
    pub exporter_dll: PathBuf,
    pub backend_game_data: PathBuf,
    pub director_game_data: PathBuf,
    pub exports: PathBuf,
    pub publish: PathBuf,
    pub logs: PathBuf,
    pub host_toml: PathBuf,
    pub gamedata_packs: PathBuf,
}

impl WorkspacePaths {
    #[must_use]
    pub fn new(root: &Path) -> Self {
        let root = root.to_path_buf();
        let isles = root.join("modules").join("rustic-isles");
        let has_isles = isles.is_dir();

        let paradox = if has_isles {
            isles.join("reference").join("1.14.7")
        } else {
            root.join("paradox")
        };

        let runtime_source = if has_isles {
            isles.join("native").join("runtime")
        } else {
            root.join("native").join("runtime")
        };

        let exporter_dir = if has_isles {
            isles.join("native").join("catalog-exporter")
        } else {
            root.join("native").join("catalog-exporter")
        };

        let loader_dir = if root.join("native").join("runtime-loader").is_dir() {
            root.join("native").join("runtime-loader")
        } else if has_isles && isles.join("native").join("runtime-loader").is_dir() {
            isles.join("native").join("runtime-loader")
        } else {
            root.join("native").join("runtime-loader")
        };

        let msvc_loader = loader_dir
            .join("target")
            .join("x86_64-pc-windows-msvc")
            .join("release")
            .join("winmm.dll");
        let release_loader = loader_dir.join("target").join("release").join("winmm.dll");
        let loader_dll = if !msvc_loader.is_file() && release_loader.is_file() {
            release_loader
        } else {
            msvc_loader
        };

        let backend_game_data = if has_isles && isles.join("gamedata").join("backend").is_dir() {
            isles.join("gamedata").join("backend")
        } else if !has_isles
            && root.join("backend").join("data").join("GameData").is_dir()
            && !paradox.join("ParadoxBackend").join("game-data").is_dir()
        {
            root.join("backend").join("data").join("GameData")
        } else {
            paradox.join("ParadoxBackend").join("game-data")
        };

        let director_game_data = if has_isles && isles.join("gamedata").join("director").is_dir() {
            isles.join("gamedata").join("director")
        } else if !has_isles
            && root.join("director").join("data").join("GameData").is_dir()
            && !paradox.join("ParadoxDirector").join("game-data").is_dir()
        {
            root.join("director").join("data").join("GameData")
        } else {
            paradox.join("ParadoxDirector").join("game-data")
        };

        let workspace = root.join(".workspace");
        let content_runtime = root.join("content").join("data").join("runtime");
        let publish = if workspace.is_dir() || !content_runtime.is_dir() {
            workspace.join("runtime")
        } else {
            content_runtime
        };

        let sdk_header = runtime_source.join("SDK.hpp");
        let sdk_dir = runtime_source.join("SDK");
        let runtime_solution = runtime_source.join("Rustic-Isles.sln");
        let runtime_dll = runtime_source.join("x64").join("Release").join("Rustic-Isles.dll");
        let exporter_project = exporter_dir.join("CatalogExporter.vcxproj");
        let exporter_dll = exporter_dir.join("x64").join("Release").join("CatalogExporter.dll");
        let exports = paradox.join("Items_Analysis");
        let logs = workspace.join("logs");
        let config = workspace.join("config");
        let host_toml = config.join("host.toml");
        let gamedata_packs = workspace.join("gamedata-packs");

        Self {
            root,
            runtime_source,
            sdk_header,
            sdk_dir,
            runtime_solution,
            runtime_dll,
            loader_dir,
            loader_dll,
            exporter_project,
            exporter_dll,
            backend_game_data,
            director_game_data,
            exports,
            publish,
            logs,
            host_toml,
            gamedata_packs,
        }
    }

    #[must_use]
    pub fn launcher_log(&self, name: &str) -> PathBuf {
        self.logs.join(format!("launcher-{name}.log"))
    }
}

/// Detected Visual Studio / MSBuild and Rust Cargo toolchain on the host PC.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Toolchain {
    pub visual_studio: Option<String>,
    pub major_version: Option<u32>,
    pub msbuild: Option<PathBuf>,
    pub cargo: Option<PathBuf>,
}

impl Toolchain {
    #[must_use]
    pub fn view(&self) -> HostToolchain {
        HostToolchain {
            visual_studio: self.visual_studio.clone(),
            msbuild: self.msbuild.is_some(),
            cargo: self.cargo.is_some(),
        }
    }
}

/// Failure codes returned by [`import_sdk`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdkError {
    NoHeader,
    NoRuntimeSource,
    NoFunctions,
    Io,
}

impl SdkError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NoHeader => "sdk_folder",
            Self::NoRuntimeSource => "workspace",
            Self::NoFunctions => "sdk_no_functions",
            Self::Io => "sdk_copy",
        }
    }
}

/// Outcome of running an external tool via [`run_logged`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logged {
    pub code: i32,
    pub last_line: Option<String>,
}

/// Locates `reforged-host` inside `root` (`<root>/<HOST_BINARY>`, `<root>/bin/<HOST_BINARY>`,
/// or `<root>/target/release/<HOST_BINARY>`).
#[must_use]
pub fn find_host_binary(root: &Path) -> Option<PathBuf> {
    [
        root.join(HOST_BINARY),
        root.join("bin").join(HOST_BINARY),
        root.join("target").join("release").join(HOST_BINARY),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

/// Returns `true` iff `root` contains `reforged-host` and either `modules/rustic-isles` or `paradox`.
#[must_use]
pub fn is_workspace(root: &Path) -> bool {
    find_host_binary(root).is_some()
        && (root.join("modules").join("rustic-isles").is_dir() || root.join("paradox").is_dir())
}

/// Counts regular `*_functions.cpp` files in `sdk_dir`.
#[must_use]
pub fn count_function_files(sdk_dir: &Path) -> u32 {
    let Ok(entries) = std::fs::read_dir(sdk_dir) else {
        return 0;
    };
    let mut count = 0u32;
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_file() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str()
            && name.to_ascii_lowercase().ends_with("_functions.cpp")
        {
            count = count.saturating_add(1);
        }
    }
    count
}

/// Classifies game-data files in `dir` against `files` as `Missing`, `Synthetic`, or `Real`.
#[must_use]
pub fn detect_game_data(dir: &Path, files: &[&str]) -> GameDataState {
    if files.is_empty() {
        return GameDataState::Missing;
    }
    let mut contents: Vec<(&str, Vec<u8>)> = Vec::with_capacity(files.len());
    for &name in files {
        let file = dir.join(name);
        let Ok(meta) = std::fs::metadata(&file) else {
            return GameDataState::Missing;
        };
        if !meta.is_file() || meta.len() == 0 {
            return GameDataState::Missing;
        }
        let Ok(bytes) = std::fs::read(&file) else {
            return GameDataState::Missing;
        };
        if bytes.is_empty() {
            return GameDataState::Missing;
        }
        contents.push((name, bytes));
    }

    for (name, real_bytes) in &contents {
        let stem = name.strip_suffix(".json").unwrap_or(name);
        let example = dir.join(format!("{stem}.example.json"));
        if let Ok(ex_meta) = std::fs::metadata(&example)
            && ex_meta.is_file()
            && ex_meta.len() == real_bytes.len() as u64
            && let Ok(ex_bytes) = std::fs::read(&example)
            && &ex_bytes == real_bytes
        {
            return GameDataState::Synthetic;
        }
    }

    GameDataState::Real
}

/// Scans `root` (`C:\Dumper-7`) for up to 200 subdirectories containing `CppSDK/SDK.hpp`, sorted newest first.
#[must_use]
pub fn find_dumps(root: &Path) -> Vec<HostDump> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<(SystemTime, HostDump)> = Vec::new();
    for entry in entries.flatten().take(MAX_DUMP_DIRS) {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let cpp_sdk = entry.path().join("CppSDK");
        let header = cpp_sdk.join("SDK.hpp");
        let Ok(meta) = std::fs::metadata(&header) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let dt: DateTime<Utc> = mtime.into();
        found.push((
            mtime,
            HostDump {
                dir: cpp_sdk.to_string_lossy().into_owned(),
                name,
                modified: dt.to_rfc3339_opts(SecondsFormat::Millis, true),
            },
        ));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    found.into_iter().map(|(_, dump)| dump).collect()
}

/// Runs `program` with `args` (20s timeout, 4 MiB output cap) and returns stdout text when exit status is 0.
pub async fn exec_text(program: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = run_hidden(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = tokio::time::timeout(EXEC_TIMEOUT, cmd.output()).await.ok()?.ok()?;
    if !output.status.success() || output.stdout.len() > MAX_EXEC_BYTES {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Returns the standard path to `vswhere.exe` under `ProgramFiles(x86)`.
pub fn vswhere_path(env: &dyn Fn(&str) -> Option<OsString>) -> PathBuf {
    let base = env("ProgramFiles(x86)")
        .or_else(|| env("ProgramFiles"))
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(r"C:\Program Files (x86)"), PathBuf::from);
    base.join("Microsoft Visual Studio")
        .join("Installer")
        .join("vswhere.exe")
}

/// Candidate paths for the `cargo` binary (`CARGO_HOME`, `USERPROFILE`/`HOME`, and `PATH`).
pub fn cargo_candidates(env: &dyn Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let bin_name = if cfg!(windows) { "cargo.exe" } else { "cargo" };
    let mut list = Vec::new();
    let mut push_unique = |p: PathBuf| {
        if !list.contains(&p) {
            list.push(p);
        }
    };

    if let Some(cargo_home) = env("CARGO_HOME").filter(|v| !v.is_empty()) {
        push_unique(PathBuf::from(cargo_home).join("bin").join(bin_name));
    }
    for home_var in ["USERPROFILE", "HOME"] {
        if let Some(home) = env(home_var).filter(|v| !v.is_empty()) {
            push_unique(PathBuf::from(home).join(".cargo").join("bin").join(bin_name));
        }
    }
    if let Some(path_var) = env("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if !dir.as_os_str().is_empty() {
                push_unique(dir.join(bin_name));
            }
        }
    }
    list
}

/// Reads an environment variable from the current process environment.
#[must_use]
pub fn system_env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// Discovers the Visual Studio C++ toolchain (`vswhere`, MSBuild) and Rust `cargo`.
pub async fn find_toolchain() -> Toolchain {
    let cargo = cargo_candidates(&system_env).into_iter().find(|p| p.is_file());

    let mut visual_studio = None;
    let mut major_version = None;
    let mut msbuild = None;

    let vswhere = vswhere_path(&system_env);
    if vswhere.is_file() {
        if let Some(json) = exec_text(
            &vswhere,
            &[
                "-latest",
                "-products",
                "*",
                "-requires",
                "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                "-format",
                "json",
                "-utf8",
            ],
        )
        .await
            && let Some((name, major)) = vswhere_install(&json)
        {
            visual_studio = Some(name);
            major_version = major;
        }

        if visual_studio.is_some()
            && let Some(out) = exec_text(
                &vswhere,
                &[
                    "-latest",
                    "-products",
                    "*",
                    "-requires",
                    "Microsoft.Component.MSBuild",
                    "-find",
                    r"MSBuild\**\Bin\MSBuild.exe",
                    "-utf8",
                ],
            )
            .await
            && let Some(path_str) = vswhere_msbuild(&out)
        {
            let candidate = PathBuf::from(path_str);
            if candidate.is_file() {
                msbuild = Some(candidate);
            }
        }
    }

    Toolchain {
        visual_studio,
        major_version,
        msbuild,
        cargo,
    }
}

/// Queries `vswhere.exe` for any Visual Studio installation (even without the C++ workload).
pub async fn any_visual_studio() -> Option<String> {
    let vswhere = vswhere_path(&system_env);
    if !vswhere.is_file() {
        return None;
    }
    let json = exec_text(&vswhere, &["-latest", "-products", "*", "-format", "json", "-utf8"]).await?;
    vswhere_install(&json).map(|(name, _)| name)
}

/// Builds the MSBuild command-line arguments for compiling a C++ project (`Release|x64`).
#[must_use]
pub fn msbuild_args(project: &Path, toolset: &str) -> Vec<String> {
    vec![
        project.to_string_lossy().into_owned(),
        "/p:Configuration=Release".to_owned(),
        "/p:Platform=x64".to_owned(),
        format!("/p:PlatformToolset={toolset}"),
        "/m".to_owned(),
        "/nologo".to_owned(),
        "/verbosity:minimal".to_owned(),
    ]
}

fn read_bounded_string(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_CONFIG_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Reads `.workspace/config/host.toml` into [`HostServerConfig`]; a Node kit's host.json is dainty-legacy's.
#[must_use]
pub fn read_server_config(root: &Path) -> HostServerConfig {
    let paths = WorkspacePaths::new(root);
    let Some(cfg) = read_bounded_string(&paths.host_toml)
        .as_deref()
        .and_then(server_config_toml)
    else {
        return HostServerConfig {
            configured: false,
            name: None,
            mode: None,
            host: None,
            multipliers: None,
        };
    };

    let mode = match cfg.mode {
        ServerMode::Private => HostServerMode::Private,
        ServerMode::Public => HostServerMode::Public,
    };
    let multipliers = cfg.multipliers.map(|(xp, rewards)| HostMultipliers { xp, rewards });

    HostServerConfig {
        configured: true,
        name: cfg.name,
        mode: Some(mode),
        host: cfg.host,
        multipliers,
    }
}

/// Inspects `workspace`, `toolchain`, and `dump_root` to build a [`HostKit`] snapshot.
#[must_use]
pub fn detect_host_kit(workspace: Option<&Path>, toolchain: &Toolchain, dump_root: &Path) -> HostKit {
    let dumps = find_dumps(dump_root);
    let Some(ws) = workspace else {
        return HostKit {
            workspace: None,
            workspace_ok: false,
            upstream: HostUpstream {
                present: false,
                patched: false,
            },
            sdk: HostSdk {
                present: false,
                function_files: 0,
            },
            dumps,
            dumper_dll: None,
            builds: HostBuilds {
                runtime: false,
                loader: false,
                exporter: false,
                published: false,
            },
            toolchain: toolchain.view(),
            game_data: HostGameData {
                backend: GameDataState::Missing,
                director: GameDataState::Missing,
            },
            exports: HostExports {
                present: false,
                in_game: false,
            },
            server_config: HostServerConfig {
                configured: false,
                name: None,
                mode: None,
                host: None,
                multipliers: None,
            },
            requirements: Vec::new(),
            job: None,
            update: None,
            project_target: None,
        };
    };

    let paths = WorkspacePaths::new(ws);
    let workspace_ok = is_workspace(ws);
    let upstream_present = paths.runtime_source.join("dllmain.cpp").is_file();
    let sdk_functions = count_function_files(&paths.sdk_dir);
    let sdk_present = paths.sdk_header.is_file();
    let runtime_built = paths.runtime_dll.is_file();
    let loader_built = paths.loader_dll.is_file();
    let exporter_built = paths.exporter_dll.is_file();
    let runtime_published = RUNTIME_FILE_NAMES.iter().all(|name| paths.publish.join(name).is_file());
    let backend_data = detect_game_data(&paths.backend_game_data, &BACKEND_GAME_DATA);
    let director_data = detect_game_data(&paths.director_game_data, &DIRECTOR_GAME_DATA);
    let exports_present = paths.exports.is_dir();
    let server_config = read_server_config(ws);

    HostKit {
        workspace: Some(ws.to_string_lossy().into_owned()),
        workspace_ok,
        upstream: HostUpstream {
            present: upstream_present,
            patched: upstream_present,
        },
        sdk: HostSdk {
            present: sdk_present,
            function_files: sdk_functions,
        },
        dumps,
        dumper_dll: None,
        builds: HostBuilds {
            runtime: runtime_built,
            loader: loader_built,
            exporter: exporter_built,
            published: runtime_published,
        },
        toolchain: toolchain.view(),
        game_data: HostGameData {
            backend: backend_data,
            director: director_data,
        },
        exports: HostExports {
            present: exports_present,
            in_game: false,
        },
        server_config,
        requirements: Vec::new(),
        job: None,
        update: None,
        project_target: None,
    }
}

/// Recursively copies regular files and directories from `from` to `to`, skipping symlinks.
pub fn copy_tree(from: &Path, to: &Path) -> std::io::Result<u32> {
    std::fs::create_dir_all(to)?;
    let mut copied = 0u32;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            continue;
        }
        let src_path = entry.path();
        let dst_path = to.join(entry.file_name());
        if ft.is_dir() {
            copied = copied.saturating_add(copy_tree(&src_path, &dst_path)?);
        } else if ft.is_file() {
            std::fs::copy(&src_path, &dst_path)?;
            copied = copied.saturating_add(1);
        }
    }
    Ok(copied)
}

/// Imports a Dumper-7 `CppSDK` folder into `runtime_source`, returning the number of `*_functions.cpp` files.
pub fn import_sdk(dump_dir: &Path, runtime_source: &Path) -> Result<u32, SdkError> {
    let source = if dump_dir.join("SDK.hpp").is_file() {
        dump_dir.to_path_buf()
    } else if dump_dir.join("CppSDK").join("SDK.hpp").is_file() {
        dump_dir.join("CppSDK")
    } else {
        return Err(SdkError::NoHeader);
    };

    if !runtime_source.join("dllmain.cpp").is_file() {
        return Err(SdkError::NoRuntimeSource);
    }

    let sdk_sub = source.join("SDK");
    let functions = count_function_files(&sdk_sub);
    if functions == 0 {
        return Err(SdkError::NoFunctions);
    }

    let target_sdk = runtime_source.join("SDK");
    if target_sdk.exists() {
        std::fs::remove_dir_all(&target_sdk).map_err(|_| SdkError::Io)?;
    }
    copy_tree(&source, runtime_source).map_err(|_| SdkError::Io)?;

    Ok(count_function_files(&target_sdk))
}

/// Runs `program` with `args` in `cwd`, writing output to `log_file` and returning the exit code and last line.
pub async fn run_logged(program: &Path, args: &[String], cwd: &Path, log_file: &Path) -> Logged {
    if let Some(parent) = log_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut file = std::fs::File::create(log_file).ok();
    if let Some(f) = file.as_mut() {
        let _ = writeln!(f, "$ {} {}", program.display(), args.join(" "));
        let _ = writeln!(f, "(in {})\n", cwd.display());
    }

    let mut cmd = run_hidden(program);
    cmd.args(args)
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            let msg = cut(clean_line(&err.to_string()).trim(), 300);
            if let Some(f) = file.as_mut() {
                let _ = writeln!(f, "{msg}\n[exit -1]");
            }
            return Logged {
                code: -1,
                last_line: (!msg.is_empty()).then_some(msg),
            };
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    if let Some(out) = stdout {
        let tx_out = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx_out.send(line);
            }
        });
    }
    if let Some(err) = stderr {
        let tx_err = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx_err.send(line);
            }
        });
    }
    drop(tx);

    let mut last_line: Option<String> = None;
    while let Some(raw) = rx.recv().await {
        let cleaned = clean_line(&raw);
        if let Some(f) = file.as_mut() {
            let _ = writeln!(f, "{cleaned}");
        }
        let trimmed = cleaned.trim();
        if !trimmed.is_empty() {
            last_line = Some(cut(trimmed, 300));
        }
    }

    let code = match child.wait().await {
        Ok(status) => status.code().unwrap_or(-1),
        Err(_) => -1,
    };
    if let Some(f) = file.as_mut() {
        let _ = writeln!(f, "\n[exit {code}]");
    }

    Logged { code, last_line }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_detection_supports_submodule_and_flat_layouts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        assert!(!is_workspace(root));

        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("bin").join(HOST_BINARY), b"bin").unwrap();
        assert!(!is_workspace(root));

        std::fs::create_dir_all(root.join("modules").join("rustic-isles")).unwrap();
        assert!(is_workspace(root));

        let paths = WorkspacePaths::new(root);
        assert_eq!(paths.runtime_source, root.join("modules/rustic-isles/native/runtime"));
        assert_eq!(paths.host_toml, root.join(".workspace/config/host.toml"));
    }

    #[test]
    fn game_data_detection_distinguishes_missing_synthetic_and_real() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let files = &["a.json", "b.json"];
        assert_eq!(detect_game_data(dir, files), GameDataState::Missing);

        std::fs::write(dir.join("a.json"), b"{\"x\":1}").unwrap();
        std::fs::write(dir.join("b.json"), b"{\"y\":2}").unwrap();
        std::fs::write(dir.join("b.example.json"), b"{\"y\":2}").unwrap();
        assert_eq!(detect_game_data(dir, files), GameDataState::Synthetic);

        std::fs::write(dir.join("b.json"), b"{\"y\":3}").unwrap();
        assert_eq!(detect_game_data(dir, files), GameDataState::Real);
    }

    #[test]
    fn import_sdk_validates_and_copies_headers_and_functions() {
        let tmp = tempfile::tempdir().unwrap();
        let dump = tmp.path().join("dump").join("CppSDK");
        let sdk = dump.join("SDK");
        let rt = tmp.path().join("runtime");

        assert_eq!(import_sdk(&dump, &rt), Err(SdkError::NoHeader));

        std::fs::create_dir_all(&sdk).unwrap();
        std::fs::write(dump.join("SDK.hpp"), b"// header").unwrap();
        assert_eq!(import_sdk(&dump, &rt), Err(SdkError::NoRuntimeSource));

        std::fs::create_dir_all(&rt).unwrap();
        assert_eq!(import_sdk(&dump, &rt), Err(SdkError::NoRuntimeSource));
        std::fs::write(rt.join("dllmain.cpp"), b"// main").unwrap();
        assert_eq!(import_sdk(&dump, &rt), Err(SdkError::NoFunctions));

        std::fs::write(sdk.join("Archon_functions.cpp"), b"// fn").unwrap();
        std::fs::write(dump.join("PropertyFixup.hpp"), b"// fixup").unwrap();

        let count = import_sdk(dump.parent().unwrap(), &rt).unwrap();
        assert_eq!(count, 1);
        assert!(rt.join("SDK.hpp").is_file());
        assert!(rt.join("PropertyFixup.hpp").is_file());
        assert!(rt.join("SDK").join("Archon_functions.cpp").is_file());
    }

    #[test]
    fn server_config_parses_toml_and_json_and_vswhere() {
        let toml_text = r#"
[server]
name = "My Private Server"
mode = "private"

[test_multipliers]
xp = 2.5
rewards = 3
"#;
        let cfg = server_config_toml(toml_text).unwrap();
        assert_eq!(cfg.name.as_deref(), Some("My Private Server"));
        assert_eq!(cfg.mode, ServerMode::Private);
        assert_eq!(cfg.multipliers, Some((2.5, 3.0)));

        let public_toml = r#"
[server]
name = "Public Hunts"
mode = "public"
public_host = "play.example.org"

[test_multipliers]
xp = 5.0
rewards = 5.0
"#;
        let pub_cfg = server_config_toml(public_toml).unwrap();
        assert_eq!(pub_cfg.mode, ServerMode::Public);
        assert_eq!(pub_cfg.host.as_deref(), Some("play.example.org"));
        assert_eq!(pub_cfg.multipliers, None);

        let vs_json = r#"[{"displayName":"Visual Studio Build Tools 2022","installationVersion":"17.12.35527.113"}]"#;
        assert_eq!(
            vswhere_install(vs_json),
            Some(("Visual Studio Build Tools 2022".to_owned(), Some(17)))
        );
        assert_eq!(platform_toolset(Some(17)), "v143");
        assert_eq!(platform_toolset(Some(18)), "v145");
    }
}
