//! `reforged-host gamedata`: the Node kit's `gamedata()` in `cli.ts`, with [`crate::gamedata`],
//! [`crate::gamepack`], [`crate::progression`] and [`crate::captures`].
//!
//! `--export [file]` and `--import <file>` move a game data pack; `--examples` copies the shipped
//! examples where data is missing; otherwise the generators run from the host's export
//! (`--progression`: only the progression config). Every generator runs in-process through
//! [`crate::gamedata::run_generator`] (RB11).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Utc;

use crate::Outcome;
use crate::cli::GamedataArgs;
use crate::error::HostError;
use crate::gamedata::{
    BACKEND_GAME_DATA, DIRECTOR_GAME_DATA, GenerateOptions, game_data_state, generate_game_data, use_examples,
};
use crate::gamepack::{default_pack_file, export_game_data_pack, import_game_data_pack};
use crate::paths::Paths;

/// A file named on the command line, relative to where the command was run.
pub(crate) fn from_cwd(file: &str) -> Result<PathBuf, HostError> {
    let cwd = std::env::current_dir().map_err(|e| HostError::io(".", e))?;
    Ok(cwd.join(file))
}

pub(crate) fn not_set_up(paths: &Paths) -> Result<(), HostError> {
    if paths.backend_dir.exists() || paths.host_toml.exists() || paths.host_json.exists() {
        Ok(())
    } else {
        Err(HostError::message("not set up yet: run reforged-host setup first"))
    }
}

/// The two `backend: real` / `Director: missing (...)` lines.
pub(crate) fn print_states_with(paths: &Paths, out: &mut dyn FnMut(&str)) -> Result<(), HostError> {
    let sides: [(&str, &Path, &[&str]); 2] = [
        ("backend", &paths.backend_game_data, &BACKEND_GAME_DATA),
        ("Director", &paths.director_game_data, &DIRECTOR_GAME_DATA),
    ];
    for (label, dir, files) in sides {
        let g = game_data_state(dir, files)?;
        let missing = if g.missing.is_empty() {
            String::new()
        } else {
            format!(" (missing {})", g.missing.join(", "))
        };
        let synthetic = if g.synthetic.is_empty() {
            String::new()
        } else {
            format!(" (example data in {})", g.synthetic.join(", "))
        };
        out(&format!("{label}: {}{missing}{synthetic}", g.state.as_str()));
    }
    Ok(())
}

pub(crate) fn log_file(paths: &Paths) -> PathBuf {
    paths.logs.join("gamedata.log")
}

pub fn run(paths: &Paths, args: &GamedataArgs) -> Outcome {
    run_with(paths, args, &mut |line| println!("{line}"))
}

pub fn run_with(paths: &Paths, args: &GamedataArgs, out: &mut dyn FnMut(&str)) -> Outcome {
    not_set_up(paths)?;
    if let Some(file) = &args.export {
        let out_file = match file {
            Some(file) => from_cwd(file)?,
            None => default_pack_file(paths, Utc::now()),
        };
        let r = export_game_data_pack(paths, &out_file, Utc::now())?;
        out(&format!("Game data pack written: {}", r.file.display()));
        let left = if r.left.is_empty() {
            String::new()
        } else {
            format!("; not in it (missing, or still the example): {}", r.left.join(", "))
        };
        out(&format!("  {} files{left}", r.included.len()));
        out("Keep it for your own servers: it is Phoenix Labs' game data from your game, not for publishing.");
        out("On a new setup: reforged-host gamedata --import <this file>");
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(file) = &args.import {
        let Some(file) = file else {
            return Err(HostError::message(
                "--import needs the pack file: reforged-host gamedata --import <file>",
            ));
        };
        let r = import_game_data_pack(paths, &from_cwd(file)?, Utc::now())?;
        let backups = if r.backups.is_empty() {
            String::new()
        } else {
            format!(" ({} replaced files kept as .bak)", r.backups.len())
        };
        out(&format!(
            "Game data pack imported: {} files written, {} already the same{backups}",
            r.written.len(),
            r.unchanged.len()
        ));
        print_states_with(paths, out)?;
        return Ok(ExitCode::SUCCESS);
    }
    if args.examples {
        if !paths.backend_game_data.exists() || !paths.director_game_data.exists() {
            crate::gamedata::seed_embedded_examples(paths)?;
        }
        let mut copied = use_examples(&paths.backend_game_data, &BACKEND_GAME_DATA)?;
        copied.extend(use_examples(&paths.director_game_data, &DIRECTOR_GAME_DATA)?);
        if copied.is_empty() {
            out("Nothing was missing.");
        } else {
            out(&format!("Copied the example data for {}.", copied.join(", ")));
            out(
                "Example data lets the services start for a test. It is not playable: export the real data from your game (docs/sdk-and-game-data.md).",
            );
        }
        return Ok(ExitCode::SUCCESS);
    }
    out("Generating the game data from your export (Items_Analysis):");
    let opts = GenerateOptions {
        examples_for_missing: args.example_sources,
        log_file: log_file(paths),
        // --progression: only the progression config (from the progression export), nothing else rewritten.
        only: args.progression.then(|| vec!["Progression config".to_owned()]),
    };
    let r = generate_game_data(paths, &opts, out)?;
    print_states_with(paths, out)?;
    Ok(if r.ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

#[cfg(test)]
mod tests {
    use crate::error::HostError;

    #[test]
    fn refuses_before_setup() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let paths = crate::paths::Paths::new(dir.path());
        let error = super::run(&paths, &Default::default()).unwrap_err();
        assert!(
            matches!(&error, HostError::Message(m) if m.starts_with("not set up yet")),
            "{error}"
        );
    }
}
