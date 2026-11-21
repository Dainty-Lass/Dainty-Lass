// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Utc;

use crate::cli::PublishRuntimeArgs;
use crate::error::HostError;
use crate::hostconfig::load_host_config;
use crate::paths::{Paths, RUNTIME_FILE_NAMES, game_exe_relative};
use crate::publish::{default_sources, ensure_server_flags, install_server_runtime, publish_runtime};

pub fn run(paths: &Paths, args: &PublishRuntimeArgs) -> Result<ExitCode, HostError> {
    run_with(paths, args, &mut |line| println!("{line}"))
}

pub fn run_with(paths: &Paths, args: &PublishRuntimeArgs, out: &mut dyn FnMut(&str)) -> Result<ExitCode, HostError> {
    let sources: Vec<(&str, PathBuf)> = match args.from.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(dir_str) => {
            let dir = Path::new(dir_str);
            RUNTIME_FILE_NAMES.iter().map(|&name| (name, dir.join(name))).collect()
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
            let installed = install_server_runtime(paths, win64_dir);
            let _ = ensure_server_flags(win64_dir)?;
            if !installed.installed.is_empty() {
                out(&format!(
                    "Installed into {}: {}",
                    win64_dir.display(),
                    installed.installed.join(", ")
                ));
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

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
}
