// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `dainty-host publish-runtime`: the built runtime DLLs published into `.workspace/runtime`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Utc;

use crate::cli::PublishRuntimeArgs;
use crate::error::HostError;
use crate::hostconfig::load_host_config;
use crate::paths::{MIT_RUNTIME_BUILD_NAME, Paths, RUNTIME_FILE_NAMES, game_exe_relative};
use crate::publish::{default_sources, ensure_server_flags, publish_runtime, retire_server_loader};
use dainty_protocol::manifest::MIT_RUNTIME_DLL;

/// Runs `dainty-host publish-runtime`, printing to stdout.
pub fn run(paths: &Paths, args: &PublishRuntimeArgs) -> Result<ExitCode, HostError> {
    run_with(paths, args, &mut |line| println!("{line}"))
}

/// Publishes the runtime DLLs from `--from` or the build's outputs, printing through `out`.
pub fn run_with(paths: &Paths, args: &PublishRuntimeArgs, out: &mut dyn FnMut(&str)) -> Result<ExitCode, HostError> {
    let sources: Vec<(&str, PathBuf)> = match args.from.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(dir_str) => {
            let dir = Path::new(dir_str);
            RUNTIME_FILE_NAMES
                .iter()
                .map(|&name| (name, from_dir(dir, name)))
                .collect()
        }
        None => default_sources(paths).to_vec(),
    };

    let published = publish_runtime(paths, Utc::now(), &sources)?;
    out(&format!(
        "Published runtime {} (changelist {}) to {}:",
        published.version,
        published.changelist,
        paths.runtime_publish.display()
    ));
    for f in &published.files {
        out(&format!("  {} ({} bytes, sha256:{})", f.name, f.size, &f.sha256[..16]));
    }

    if let Ok(Some(cfg)) = load_host_config(paths)
        && let Some(ref game_dir) = cfg.game_dir
    {
        let exe = Path::new(game_dir).join(game_exe_relative());
        if let Some(win64_dir) = exe.parent().filter(|d| d.is_dir()) {
            if retire_server_loader(paths, win64_dir) {
                out(&format!(
                    "Removed the runtime loader (winmm.dll) from {}: the Director injects the runtime now",
                    win64_dir.display()
                ));
            }
            let _ = ensure_server_flags(win64_dir)?;
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// `name` in `dir`; the MIT runtime also under the name cargo builds it with.
fn from_dir(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    if name == MIT_RUNTIME_DLL && !path.is_file() {
        return dir.join(MIT_RUNTIME_BUILD_NAME);
    }
    path
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn publish_runtime_from_custom_folder_succeeds() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        let custom = tmp.path().join("built");
        std::fs::create_dir_all(&custom).unwrap();
        for name in RUNTIME_FILE_NAMES {
            std::fs::write(custom.join(name), b"dll-bytes").unwrap();
        }

        let args = PublishRuntimeArgs {
            from: Some(custom.to_string_lossy().into_owned()),
        };
        let code = run(&paths, &args).unwrap();
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(paths.runtime_publish.join("runtime.json").is_file());
    }

    #[test]
    fn a_folder_with_the_mit_build_name_publishes_it_renamed() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        let custom = tmp.path().join("built");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(custom.join("Rustic-Isles.dll"), b"agpl").unwrap();
        std::fs::write(custom.join("winmm.dll"), b"loader").unwrap();
        std::fs::write(custom.join(MIT_RUNTIME_BUILD_NAME), b"mit").unwrap();
        let args = PublishRuntimeArgs {
            from: Some(custom.to_string_lossy().into_owned()),
        };
        run_with(&paths, &args, &mut |_| {}).unwrap();
        assert_eq!(
            std::fs::read(paths.runtime_publish.join(MIT_RUNTIME_DLL)).unwrap(),
            b"mit"
        );
        assert!(!paths.runtime_publish.join(MIT_RUNTIME_BUILD_NAME).exists());
        // A retired runtime in the folder is never published (spec-runtime-r21 H-4)
        assert!(!paths.runtime_publish.join("Rustic-Isles.dll").exists());
    }
}
