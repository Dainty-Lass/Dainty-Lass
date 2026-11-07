// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The command line (spec §3) and its output (§12.4). The texts name `isles-db`, restricted's binary, while it
//! is the front (§17 item 11).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;

use dainty_module_api::NodeId;
use serde::Serialize;

#[cfg(feature = "export-mongo")]
use crate::run_export_mongo;
use crate::{
    Area, DbError, ExportOptions, ExportReport, LegacyStarter, MigrateOptions, MigrateReport, VerifyReport,
    run_migrate, verify_report,
};

const USAGE: &str = "\
usage:
  isles-db migrate --from <mongodb://...> --to <sqlite-file|postgres-url> [options]
  isles-db verify --against <mongodb://...> --to <sqlite-file|postgres-url> [options]
  isles-db export-mongo --from <sqlite-file|postgres-url> --to <mongodb://...> [options]
  isles-db replay ...  (the shadow replay harness's own options)

options:
  --from, --against <uri>      the MongoDB source (migrate, verify)
  --to <target>                the store: a SQLite file, sqlite://<file> or postgres://...
  --node <id>                  the node derived ids belong to (default local)
  --area <a,b>                 auth, characters, inventory, progression, bounties, parties, reforged (repeatable)
  --game-data-dir <dir>        where inventory_storage_policy.json is looked for
  --dry-run                    read and transform only; write nothing
  --skip-invalid               skip a document missing a required field instead of stopping
  --allow-remote-postgres      accept a Postgres host off this machine (the connection is plaintext)
  --json                       print the report as JSON
  -h, --help                   this text";

/// A parsed command line.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `migrate`: MongoDB into the store; `json` prints the report as JSON.
    Migrate {
        /// The command's options.
        opts: MigrateOptions,
        /// Print the report as JSON (`--json`).
        json: bool,
    },
    /// `verify`: the store compared with MongoDB table by table.
    Verify {
        /// The command's options.
        opts: MigrateOptions,
        /// Print the report as JSON (`--json`).
        json: bool,
    },
    /// `export-mongo`: the store written back into MongoDB (RB13).
    ExportMongo {
        /// The command's options.
        opts: ExportOptions,
        /// Print the report as JSON (`--json`).
        json: bool,
    },
    /// `-h`/`--help`, or no command: print the usage.
    Help,
}

fn config(text: impl Into<String>) -> DbError {
    DbError::Config(text.into())
}

/// The next argument, whatever it is (so `--to --dry-run` targets the text `--dry-run`).
fn value(args: &mut impl Iterator<Item = String>, missing: String) -> Result<String, DbError> {
    args.next().ok_or_else(|| config(missing))
}

fn add_areas(areas: &mut BTreeSet<Area>, list: &str) -> Result<(), DbError> {
    for item in list.split(',').filter(|i| !i.trim().is_empty()) {
        areas.insert(Area::parse(item)?);
    }
    Ok(())
}

fn node(text: Option<String>) -> Result<NodeId, DbError> {
    let text = text.unwrap_or_else(|| "local".into());
    text.parse::<NodeId>()
        .map_err(|e| config(format!("invalid --node {text:?}: {e}")))
}

/// The arguments after the program name.
pub fn parse_args(args: impl IntoIterator<Item = String>, starter: LegacyStarter) -> Result<Command, DbError> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Ok(Command::Help);
    };
    match command.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "migrate" | "verify" => parse_migrate(&command, args, starter),
        "export-mongo" => parse_export(args),
        "replay" => Err(DbError::Replay(args.collect())),
        other => Err(config(format!(
            "unknown subcommand {other:?}; expected 'migrate', 'verify', 'export-mongo', or 'replay'"
        ))),
    }
}

fn parse_migrate(
    command: &str,
    mut args: impl Iterator<Item = String>,
    starter: LegacyStarter,
) -> Result<Command, DbError> {
    let (mut from, mut to, mut node_text, mut dir) = (None, None, None, None);
    let mut areas = BTreeSet::new();
    let (mut dry_run, mut skip_invalid, mut allow_remote, mut json) = (false, false, false, false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--from" | "--against" => from = Some(value(&mut args, format!("{arg} requires a MongoDB URI"))?),
            "--to" => {
                to = Some(value(
                    &mut args,
                    "--to requires a target path or postgres:// URL".into(),
                )?)
            }
            "--node" => node_text = Some(value(&mut args, "--node requires a node ID".into())?),
            "--area" => add_areas(&mut areas, &value(&mut args, "--area requires an area name".into())?)?,
            "--game-data-dir" => {
                dir = Some(PathBuf::from(value(
                    &mut args,
                    "--game-data-dir requires a path".into(),
                )?))
            }
            "--dry-run" => dry_run = true,
            "--skip-invalid" => skip_invalid = true,
            "--allow-remote-postgres" => allow_remote = true,
            "--json" => json = true,
            "-h" | "--help" => return Ok(Command::Help),
            other => return Err(config(format!("unexpected flag {other:?}"))),
        }
    }
    let from = from.ok_or_else(|| config("missing required --from <mongodb://...>"))?;
    let to = match to {
        Some(to) => to,
        None if dry_run => String::new(),
        None => return Err(config("missing required --to <sqlite-file|postgres-url>")),
    };
    let opts = MigrateOptions {
        from_uri: from,
        to_target: to,
        node: node(node_text)?,
        dry_run,
        areas,
        skip_invalid,
        game_data_dir: dir,
        allow_remote_postgres: allow_remote,
        starter,
    };
    Ok(if command == "verify" {
        Command::Verify { opts, json }
    } else {
        Command::Migrate { opts, json }
    })
}

fn parse_export(mut args: impl Iterator<Item = String>) -> Result<Command, DbError> {
    let (mut from, mut to, mut node_text) = (None, None, None);
    let mut areas = BTreeSet::new();
    let (mut dry_run, mut allow_remote, mut json) = (false, false, false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--from" => {
                from = Some(value(
                    &mut args,
                    "--from requires a SQLite path or postgres:// URL".into(),
                )?)
            }
            "--to" => to = Some(value(&mut args, "--to requires a MongoDB URI".into())?),
            "--node" => node_text = Some(value(&mut args, "--node requires a node ID".into())?),
            "--area" => add_areas(&mut areas, &value(&mut args, "--area requires an area name".into())?)?,
            "--dry-run" => dry_run = true,
            "--allow-remote-postgres" => allow_remote = true,
            "--json" => json = true,
            "-h" | "--help" => return Ok(Command::Help),
            other => return Err(config(format!("unexpected flag {other:?}"))),
        }
    }
    let from = from.ok_or_else(|| config("missing required --from <sqlite-file|postgres-url>"))?;
    let to = match to {
        Some(to) => to,
        None if dry_run => String::new(),
        None => return Err(config("missing required --to <mongodb://...>")),
    };
    // Validated, then unused: export writes Node's ids (P3)
    node(node_text)?;
    Ok(Command::ExportMongo {
        opts: ExportOptions {
            from_source: from,
            to_uri: to,
            areas,
            dry_run,
            allow_remote_postgres: allow_remote,
        },
        json,
    })
}

fn print_json(report: &impl Serialize) {
    match serde_json::to_string_pretty(report) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("error: {e}"),
    }
}

/// Prints `migrate`'s report to stdout, as text or as JSON.
pub fn print_migrate_report(report: &MigrateReport, json: bool) {
    if json {
        return print_json(report);
    }
    println!(
        "isles-db migrate ({}):",
        if report.dry_run { "dry-run" } else { "migrated" }
    );
    for (table, n) in report.counts.iter().filter(|(_, n)| **n > 0) {
        println!("  {table}: {n}");
    }
    println!(
        "  rb11 transforms: approvals={}, inventory_arrays={}, starter_instances={}, loadout_slots={}",
        report.rb11_launcher_approvals_backfilled,
        report.rb11_inventory_arrays_transformed,
        report.rb11_starter_instances_repaired,
        report.rb11_loadout_slots_reconciled
    );
    if report.skipped_invalid > 0 {
        println!("  skipped_invalid: {}", report.skipped_invalid);
    }
}

/// A failed report prints too, with `[DIFF]` lines and their mismatches (coordinator item 4).
pub fn print_verify_report(report: &VerifyReport, json: bool) {
    if json {
        return print_json(report);
    }
    let failed = report.collections.iter().filter(|c| !c.ok).count();
    if failed == 0 {
        println!(
            "isles-db verify: OK ({} collections verified)",
            report.collections.len()
        );
    } else {
        println!(
            "isles-db verify: FAILED ({failed} of {} collections did not match)",
            report.collections.len()
        );
    }
    for c in &report.collections {
        let short = c.actual_sha256.get(..16).unwrap_or(&c.actual_sha256);
        if c.ok {
            println!(
                "  [OK] {} (area={}, count={}, sha256={short})",
                c.name,
                c.area.as_str(),
                c.actual_count
            );
        } else {
            println!(
                "  [DIFF] {} (area={}, expected={}, actual={})",
                c.name,
                c.area.as_str(),
                c.expected_count,
                c.actual_count
            );
            for m in &c.mismatches {
                println!("    {m}");
            }
        }
    }
}

/// Prints `export-mongo`'s report to stdout, as text or as JSON.
pub fn print_export_report(report: &ExportReport, json: bool) {
    if json {
        return print_json(report);
    }
    println!(
        "isles-db export-mongo ({}):",
        if report.dry_run { "dry-run" } else { "exported" }
    );
    for (collection, n) in report.counts.iter().filter(|(_, n)| **n > 0) {
        println!("  {collection}: {n}");
    }
}

fn failed(command: &str, error: &DbError) -> ExitCode {
    eprintln!("isles-db {command} failed: {error}");
    ExitCode::from(1)
}

/// The front: parse, run, print; 0 done, 1 a command failed, 2 the arguments were wrong.
pub fn main_with(args: impl IntoIterator<Item = String>, starter: LegacyStarter) -> ExitCode {
    let command = match parse_args(args, starter) {
        Ok(Command::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(command) => command,
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    runtime.block_on(async move {
        match command {
            Command::Migrate { opts, json } => match run_migrate(&opts).await {
                Ok(report) => {
                    print_migrate_report(&report, json);
                    ExitCode::SUCCESS
                }
                Err(e) => failed("migrate", &e),
            },
            Command::Verify { opts, json } => match verify_report(&opts).await {
                Ok(report) => {
                    print_verify_report(&report, json);
                    match report.collections.iter().filter(|c| !c.ok).count() {
                        0 => ExitCode::SUCCESS,
                        n => failed("verify", &DbError::VerificationFailed(n)),
                    }
                }
                Err(e) => failed("verify", &e),
            },
            #[cfg(feature = "export-mongo")]
            Command::ExportMongo { opts, json } => match run_export_mongo(&opts).await {
                Ok(report) => {
                    print_export_report(&report, json);
                    ExitCode::SUCCESS
                }
                Err(e) => failed("export-mongo", &e),
            },
            #[cfg(not(feature = "export-mongo"))]
            Command::ExportMongo { .. } => failed(
                "export-mongo",
                &DbError::Config("this build has no export-mongo (RB15)".into()),
            ),
            Command::Help => ExitCode::SUCCESS,
        }
    })
}
