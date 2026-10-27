//! `isles-db`: RU7 + RB11 MongoDB-to-`store` migration and verification library and CLI.

pub mod migrate;
pub mod mongo;
pub mod schema;
pub mod transforms;
pub mod verify;

use std::path::PathBuf;

use module_api::NodeId;

pub use migrate::{DbError, MigrateArea, MigrateOptions, MigrateReport, run_migrate};
pub use verify::{CollectionVerification, VerifyReport, run_verify};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Migrate { opts: MigrateOptions, json: bool },
    Verify { opts: MigrateOptions, json: bool },
    Help,
}

impl MigrateOptions {
    // Allow PartialEq/Eq on Command via manual comparison
}

impl PartialEq for MigrateOptions {
    fn eq(&self, other: &Self) -> bool {
        self.from_uri == other.from_uri
            && self.to_target == other.to_target
            && self.node == other.node
            && self.dry_run == other.dry_run
            && self.areas == other.areas
            && self.skip_invalid == other.skip_invalid
            && self.game_data_dir == other.game_data_dir
            && self.allow_remote_postgres == other.allow_remote_postgres
    }
}

impl Eq for MigrateOptions {}

pub const USAGE: &str = "\
Usage:
  isles-db migrate --from <mongodb://...> --to <sqlite-file|postgres-url> [options]
  isles-db verify  --from <mongodb://...> --to <sqlite-file|postgres-url> [options]

Options:
  --from <uri>               Source MongoDB connection URI (read-only)
  --to <target>              Destination SQLite file path or postgres:// URL
  --node <node-id>           Node ID for DD18 global account/party IDs (default: local)
  --area <area>              Migrate/verify only selected area(s): auth, characters,
                             inventory, progression, bounties, parties, reforged
                             (repeatable or comma-separated; default: all)
  --dry-run                  Read and transform MongoDB documents without writing to --to
  --skip-invalid             Skip malformed source documents with a warning instead of failing
  --game-data-dir <dir>      Directory containing inventory_storage_policy.json for RB11
  --allow-remote-postgres    Allow non-loopback Postgres targets
  --json                     Print machine-readable JSON report to stdout
  -h, --help                 Print this help message
";

pub fn parse_args<I, S>(args: I) -> Result<Command, DbError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let raw: Vec<String> = args.into_iter().map(|s| s.as_ref().to_owned()).collect();
    let Some(sub) = raw.first().map(String::as_str) else {
        return Ok(Command::Help);
    };
    if matches!(sub, "-h" | "--help" | "help") {
        return Ok(Command::Help);
    }
    let is_migrate = match sub {
        "migrate" => true,
        "verify" => false,
        other => {
            return Err(DbError::Config(format!(
                "unknown subcommand {other:?}; expected 'migrate' or 'verify'"
            )));
        }
    };

    let mut from_uri: Option<String> = None;
    let mut to_target: Option<String> = None;
    let mut node_str = "local".to_owned();
    let mut dry_run = false;
    let mut areas = Vec::new();
    let mut skip_invalid = false;
    let mut game_data_dir: Option<PathBuf> = None;
    let mut allow_remote_postgres = false;
    let mut json = false;

    let mut idx = 1;
    while idx < raw.len() {
        let arg = raw[idx].as_str();
        match arg {
            "-h" | "--help" => return Ok(Command::Help),
            "--dry-run" => {
                dry_run = true;
                idx += 1;
            }
            "--skip-invalid" => {
                skip_invalid = true;
                idx += 1;
            }
            "--allow-remote-postgres" => {
                allow_remote_postgres = true;
                idx += 1;
            }
            "--json" => {
                json = true;
                idx += 1;
            }
            "--from" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--from requires a MongoDB URI".into()))?;
                from_uri = Some(val.clone());
                idx += 2;
            }
            "--to" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--to requires a target path or postgres:// URL".into()))?;
                to_target = Some(val.clone());
                idx += 2;
            }
            "--node" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--node requires a node ID".into()))?;
                node_str = val.clone();
                idx += 2;
            }
            "--area" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--area requires an area name".into()))?;
                for part in val.split(',') {
                    if !part.trim().is_empty() {
                        areas.push(MigrateArea::parse(part)?);
                    }
                }
                idx += 2;
            }
            "--game-data-dir" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--game-data-dir requires a path".into()))?;
                game_data_dir = Some(PathBuf::from(val));
                idx += 2;
            }
            other => {
                return Err(DbError::Config(format!("unexpected flag {other:?}")));
            }
        }
    }

    let from_uri = from_uri.ok_or_else(|| DbError::Config("missing required --from <mongodb://...>".into()))?;
    let to_target = if dry_run && to_target.is_none() {
        String::new()
    } else {
        to_target.ok_or_else(|| DbError::Config("missing required --to <sqlite-file|postgres-url>".into()))?
    };
    let node = node_str
        .parse::<NodeId>()
        .map_err(|e| DbError::Config(format!("invalid --node {node_str:?}: {e}")))?;

    let opts = MigrateOptions {
        from_uri,
        to_target,
        node,
        dry_run,
        areas,
        skip_invalid,
        game_data_dir,
        allow_remote_postgres,
    };

    if is_migrate {
        Ok(Command::Migrate { opts, json })
    } else {
        Ok(Command::Verify { opts, json })
    }
}
