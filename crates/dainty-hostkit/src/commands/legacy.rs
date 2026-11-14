// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `dainty-host legacy import <kind> [args]`: dainty-legacy's importers for old installs (Revived 1.4.4
//! servers and offline saves, Reforged Node kits). It runs its own process, never linked in, so its AGPL
//! code stays out of this crate; while the server runs, the console's `legacy import` reaches the same
//! importer as `module run legacy import-<kind>`.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::Outcome;
use crate::cli::LegacyArgs;
use crate::error::HostError;
use crate::hostconfig::load_host_config;
use crate::paths::Paths;

/// The external module id dainty-legacy is pinned under in `[[modules.external]]`.
pub const MODULE_ID: &str = "legacy";

/// The kinds `legacy import` takes.
pub const KINDS: [&str; 3] = ["revived", "offline-save", "reforged"];

/// Runs dainty-legacy's `import <kind>` in the server folder and returns its exit code.
pub fn run(paths: &Paths, args: &LegacyArgs) -> Outcome {
    let crate::cli::LegacyCommand::Import(import) = &args.command;
    if !KINDS.contains(&import.kind.as_str()) {
        return Err(HostError::message(format!(
            "unknown kind {:?}: one of {}",
            import.kind,
            KINDS.join(", ")
        )));
    }
    let program = program(paths, import.program.as_deref())?;
    let status = Command::new(&program)
        .arg("import")
        .arg(&import.kind)
        .args(&import.args)
        .current_dir(&paths.repo)
        .status()
        .map_err(|e| HostError::io(&program, e))?;
    Ok(match status.code() {
        Some(0) => ExitCode::SUCCESS,
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => ExitCode::FAILURE,
    })
}

/// `--program` as given, else the `legacy` module settings.toml pins, checked against its SHA-256 as a load is (DD5).
fn program(paths: &Paths, explicit: Option<&Path>) -> Result<PathBuf, HostError> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let pinned = load_host_config(paths)?
        .and_then(|c| c.modules)
        .and_then(|m| m.external.into_iter().find(|e| e.id.as_str() == MODULE_ID));
    let Some(entry) = pinned else {
        // Before the first setup (a Node kit's upgrade) there is no settings.toml to pin it in
        return Err(HostError::message(format!(
            "no [[modules.external]] with id = \"{MODULE_ID}\" in settings.toml: pass --program <path to dainty-legacy>"
        )));
    };
    let path = paths.repo.join(&entry.path);
    let actual = crate::doctor::sha256_file(&path)?;
    if actual != entry.sha256.to_string() {
        return Err(HostError::message(format!(
            "{} does not match the SHA-256 settings.toml pins for {MODULE_ID}",
            path.display()
        )));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;
    use crate::cli::{LegacyCommand, LegacyImportArgs};
    use tempfile::TempDir;

    fn args(kind: &str, program: Option<&str>) -> LegacyArgs {
        LegacyArgs {
            command: LegacyCommand::Import(LegacyImportArgs {
                kind: kind.into(),
                program: program.map(PathBuf::from),
                args: Vec::new(),
            }),
        }
    }

    #[test]
    fn refuses_an_unknown_kind_and_an_unpinned_program() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        let err = run(&paths, &args("metagame", None)).unwrap_err().to_string();
        assert!(err.contains("revived, offline-save, reforged"), "{err}");
        let err = run(&paths, &args("revived", None)).unwrap_err().to_string();
        assert!(err.contains("--program"), "{err}");
    }

    #[test]
    fn a_node_kit_is_refused_until_imported() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(paths.config.join("host.json"), "{}").unwrap();
        let err = load_host_config(&paths).unwrap_err().to_string();
        assert!(err.contains("dainty-legacy import reforged"), "{err}");
    }
}
