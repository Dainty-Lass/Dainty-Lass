// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `dainty-host backup`: a copy of the server's SQLite database in `.workspace/backups/`. A Postgres database is
//! the host's to back up with its own tools. The command has no options yet; an output folder (`--out`) would go
//! through the integration pass (RH1 owns the `clap` tree).

use std::process::ExitCode;

use crate::Outcome;
use crate::commands::setup::database::{Database, database};
use crate::error::HostError;
use crate::hostconfig::load_host_config;
use crate::paths::Paths;

/// What backup needs from this PC.
pub trait BackupEnv {
    /// The archive's time stamp: `20261022-153000`.
    fn stamp(&self) -> String;
}

/// [`BackupEnv`] on this PC.
pub struct System;

impl BackupEnv for System {
    fn stamp(&self) -> String {
        chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
    }
}

/// Runs `dainty-host backup`, printing to stdout.
pub fn run(paths: &Paths) -> Outcome {
    run_with(paths, &System, &mut |line| println!("{line}"))
}

/// Backs up the server's SQLite database into `.workspace/backups`, printing through `out`.
pub fn run_with(paths: &Paths, env: &dyn BackupEnv, out: &mut dyn FnMut(&str)) -> Outcome {
    let config =
        load_host_config(paths)?.ok_or_else(|| HostError::message("not set up yet: run dainty-host setup first"))?;
    if database(&config) == Database::Postgres {
        return Err(HostError::message(
            "this server's database is Postgres: back it up with pg_dump (the database settings.toml's [database] names)",
        ));
    }
    let db_file = paths.config.join(crate::hostconfig::DATABASE_FILE);
    if !db_file.is_file() {
        return Err(HostError::message(format!(
            "no SQLite database at {} yet: start the server once so the backend creates it, then run backup again",
            db_file.display()
        )));
    }
    let dir = paths.workspace.join("backups");
    std::fs::create_dir_all(&dir).map_err(|e| HostError::io(&dir, e))?;
    let file = dir.join(format!("sqlite-{}.db", env.stamp()));
    let bytes = std::fs::copy(&db_file, &file).map_err(|e| HostError::io(&file, e))?;
    out(&format!("Backup written: {} ({bytes} bytes)", file.display()));
    out(&format!(
        "Restore it with the server stopped: copy \"{}\" over \"{}\"",
        file.display(),
        db_file.display()
    ));
    Ok(ExitCode::SUCCESS)
}
