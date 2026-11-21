// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `dainty-host preset`: the checkout's game-data preset shown, created from this server's game data, or applied.

use std::path::PathBuf;
use std::process::ExitCode;

use chrono::Utc;

use crate::cli::PresetArgs;
use crate::error::HostError;
use crate::gamedata::{GenerateOptions, generate_game_data};
use crate::hostconfig::load_host_config;
use crate::paths::{Paths, game_exe_relative};
use crate::preset::{CreateOptions, apply_preset, create_preset, read_preset_manifest};

/// Runs `dainty-host preset`: applies, creates or shows the preset, printing to stdout.
pub fn run(paths: &Paths, args: &PresetArgs) -> Result<ExitCode, HostError> {
    if let Some(ref apply_arg) = args.apply {
        let target_paths = resolve_target_paths(paths, apply_arg);
        match apply_preset(&target_paths, args.force)? {
            Some(applied) => {
                println!(
                    "Applied preset {:?}: {} written, {} unchanged, {} kept, {} backed up.",
                    applied.name,
                    applied.written.len(),
                    applied.unchanged.len(),
                    applied.kept.len(),
                    applied.backups.len()
                );
            }
            None => {
                println!("No preset found in {}.", target_paths.preset.display());
            }
        }
        return Ok(ExitCode::SUCCESS);
    }

    if args.create.is_some() || args.generate || args.name.is_some() {
        if args.generate {
            let log_file = paths.logs.join("gamedata.log");
            let _ = generate_game_data(
                paths,
                &GenerateOptions {
                    examples_for_missing: false,
                    log_file,
                    only: None,
                },
                &mut |line| println!("{line}"),
            )?;
        }

        let name = args
            .name
            .as_deref()
            .or(args.create.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "true")
            .unwrap_or("Custom")
            .to_owned();

        let executable = load_host_config(paths)
            .ok()
            .flatten()
            .and_then(|c| c.game_dir)
            .map(|gd| PathBuf::from(gd).join(game_exe_relative()))
            .filter(|p| p.is_file());

        let created = create_preset(
            paths,
            &CreateOptions {
                name,
                executable,
                executable_sha256: None,
            },
            Utc::now(),
        )?;
        println!(
            "Created preset {:?} in {} ({} file(s), {} excluded).",
            created.manifest.name,
            paths.preset.display(),
            created.manifest.files.len(),
            created.excluded.len()
        );
        return Ok(ExitCode::SUCCESS);
    }

    match read_preset_manifest(paths)? {
        Some(manifest) => {
            println!("Preset: {} (game {})", manifest.name, manifest.game);
            println!("Created: {}", manifest.created);
            println!("Files: {}", manifest.files.len());
        }
        None => {
            println!("No preset found in {}.", paths.preset.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn resolve_target_paths(paths: &Paths, raw: &str) -> Paths {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "default" || trimmed == "true" {
        return paths.clone();
    }
    let candidate = PathBuf::from(trimmed);
    if candidate.join("preset.json").is_file() {
        let mut p = paths.clone();
        p.preset = candidate;
        p
    } else {
        paths.clone()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn show_preset_succeeds_when_none_exists() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        let args = PresetArgs::default();
        let code = run(&paths, &args).unwrap();
        assert_eq!(code, ExitCode::SUCCESS);
    }
}
