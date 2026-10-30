// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Utc;

use crate::cli::GamedataArgs;
use crate::error::HostError;
use crate::gamedata::{
    BACKEND_GAME_DATA, DIRECTOR_GAME_DATA, GenerateOptions, copy_supplied_sources,
    generate_game_data, seed_embedded_examples, use_examples,
};
use crate::gamepack::{default_pack_file, export_game_data_pack, import_game_data_pack};
use crate::paths::Paths;
use crate::progression::{SourceWritten, write_progression_source};

pub fn run(paths: &Paths, args: &GamedataArgs) -> Result<ExitCode, HostError> {
    run_with(paths, args, &mut |line| println!("{line}"))
}

pub fn run_with(
    paths: &Paths,
    args: &GamedataArgs,
    out: &mut dyn FnMut(&str),
) -> Result<ExitCode, HostError> {
    if let Some(ref import_path) = args.import {
        let pack_file = Path::new(import_path.trim());
        let imported = import_game_data_pack(paths, pack_file, Utc::now())?;
        out(&format!(
            "Imported {} file(s) from {} ({} unchanged, {} backed up).",
            imported.written.len(),
            pack_file.display(),
            imported.unchanged.len(),
            imported.backups.len()
        ));
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(ref export_arg) = args.export {
        let now = Utc::now();
        let out_file = resolve_export_path(paths, export_arg, now);
        let exported = export_game_data_pack(paths, &out_file, now)?;
        out(&format!(
            "Exported {} file(s) to {}.",
            exported.included.len(),
            exported.file.display()
        ));
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(ref prog_arg) = args.progression {
        let trimmed = prog_arg.trim();
        let exports_dir = if trimmed.is_empty() {
            paths.exports.clone()
        } else {
            PathBuf::from(trimmed)
        };
        let out_file = paths.raw_game_data.join("progression_config.source.json");
        match write_progression_source(&exports_dir, &out_file)? {
            SourceWritten::Written { written, derived } => {
                out(&format!(
                    "Wrote {written} progression path(s) ({derived} derived prestige track(s)) to {}.",
                    out_file.display()
                ));
            }
            SourceWritten::Supplied => {
                out(&format!(
                    "Keeping existing supplied progression source at {}.",
                    out_file.display()
                ));
            }
            SourceWritten::NoExport => {
                return Err(HostError::message(format!(
                    "no progression export found in {}",
                    exports_dir.display()
                )));
            }
        }
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(ref src_dir_raw) = args.example_sources {
        let src_dir = Path::new(src_dir_raw.trim());
        let copied = copy_supplied_sources(paths, src_dir)?;
        out(&format!(
            "Copied {} supplied source file(s) into {}.",
            copied.len(),
            paths.raw_game_data.display()
        ));
        return Ok(ExitCode::SUCCESS);
    }

    if args.examples {
        let mut copied = use_examples(&paths.backend_game_data, &BACKEND_GAME_DATA)?;
        copied.extend(use_examples(&paths.director_game_data, &DIRECTOR_GAME_DATA)?);
        let seeded = seed_embedded_examples(paths)?;
        for s in seeded {
            if !copied.contains(&s) {
                copied.push(s);
            }
        }
        out(&format!("Seeded {} example game-data file(s).", copied.len()));
        return Ok(ExitCode::SUCCESS);
    }

    out(&format!("Generating game data from {}...", paths.exports.display()));
    let log_file = paths.logs.join("gamedata.log");
    let generated = generate_game_data(
        paths,
        &GenerateOptions {
            examples_for_missing: true,
            log_file,
            only: None,
        },
        out,
    )?;
    if !generated.ok {
        return Ok(ExitCode::FAILURE);
    }
    out(&format!(
        "Generated {} step(s) ({} fallback example(s)).",
        generated.ran.len(),
        generated.examples.len()
    ));
    Ok(ExitCode::SUCCESS)
}

fn resolve_export_path(paths: &Paths, raw: &str, now: chrono::DateTime<Utc>) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "default" {
        default_pack_file(paths, now)
    } else {
        PathBuf::from(trimmed)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn examples_flag_seeds_backend_and_director_game_data() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        let args = GamedataArgs {
            examples: true,
            ..GamedataArgs::default()
        };
        let code = run(&paths, &args).unwrap();
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(paths.backend_game_data.join("progression_config.json").is_file());
        assert!(paths.director_game_data.join("player_hunts_table.json").is_file());
    }
}
