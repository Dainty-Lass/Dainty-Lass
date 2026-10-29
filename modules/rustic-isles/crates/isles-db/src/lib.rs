//! `isles-db`: RU7 + RB11 MongoDB-to-`store` migration/verification, RB13 `export-mongo` rollback,
//! and RU8 shadow replay library and CLI.

pub mod export;
pub mod migrate;
pub mod mongo;
pub mod replay;
pub mod schema;
pub mod transforms;
pub mod verify;

use std::path::PathBuf;

use module_api::NodeId;

pub use export::{ExportOptions, ExportReport, run_export_mongo, state_to_mongo_collections};
pub use migrate::{DbError, MigrateArea, MigrateOptions, MigrateReport, run_migrate};
pub use replay::{ReplayOptions, ReplayReport, run_replay_cli};
pub use verify::{CollectionVerification, VerifyReport, run_verify};

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Migrate { opts: MigrateOptions, json: bool },
    Verify { opts: MigrateOptions, json: bool },
    ExportMongo { opts: ExportOptions, json: bool },
    Replay { opts: ReplayOptions, json: bool },
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
  isles-db migrate      --from <mongodb://...> --to <sqlite-file|postgres-url> [options]
  isles-db verify       --against <mongodb://...> --to <sqlite-file|postgres-url> [options]
  isles-db export-mongo --from <sqlite-file|postgres-url> --to <mongodb://...> [options]
  isles-db replay       --corpus <path> --left <url|recorded> --right <url> [options]

Migrate / Verify / Export Options:
  --from <source>            Source URI/path (MongoDB URI for migrate/verify; SQLite/Postgres for export-mongo)
  --against <mongodb://...>  Source MongoDB URI to verify against (alias for --from on verify)
  --to <target>              Destination target (SQLite/Postgres for migrate/verify; MongoDB URI for export-mongo)
  --node <node-id>           Node ID for DD18 global account/party IDs (default: local)
  --area <area>              Migrate/verify/export only selected area(s): auth, characters,
                             inventory, progression, bounties, parties, reforged
                             (repeatable or comma-separated; default: all)
  --dry-run                  Read and transform documents without writing to --to
  --skip-invalid             Skip malformed source documents with a warning instead of failing
  --game-data-dir <dir>      Directory containing inventory_storage_policy.json for RB11
  --allow-remote-postgres    Allow non-loopback Postgres targets

Replay Options:
  --corpus <path>            Path to a .jsonl capture file or directory of .jsonl files
  --left <url|recorded>      Left target URL (e.g. http://127.0.0.1:8421) or 'recorded'
  --right <url>              Right target URL (e.g. http://127.0.0.1:61000)
  --left-director <url>      Left Director URL for route='director' requests
  --right-director <url>     Right Director URL for route='director' requests
  --left-chat <host:port>    Left XMPP chat address for kind='chat' frames
  --right-chat <host:port>   Right XMPP chat address for kind='chat' frames
  --jwt-secret <secret>      HS256 JWT secret for <token:...> on both targets
  --left-jwt-secret <sec>    HS256 JWT secret for <token:...> on left target
  --right-jwt-secret <sec>   HS256 JWT secret for <token:...> on right target
  --signing-key <b64>        RS256 AUTH_SIGNING_PRIVKEY_B64 for <token:...> on both targets
  --left-signing-key <b64>   RS256 AUTH_SIGNING_PRIVKEY_B64 for <token:...> on left target
  --right-signing-key <b64>  RS256 AUTH_SIGNING_PRIVKEY_B64 for <token:...> on right target
  --gs-key <key>             GAMESERVER_API_KEY for <gskey> on both targets
  --left-gs-key <key>        GAMESERVER_API_KEY for <gskey> on left target
  --right-gs-key <key>       GAMESERVER_API_KEY for <gskey> on right target
  --gateway-secret <secret>  x-gateway-secret header value for forwarded requests
  --key-map <json-file>      JSON file mapping <sha256> -> raw RRK_/UUK_ account keys
  --pin-clock                Send x-isles-replay-now-ms and x-isles-replay-seed headers (default)
  --no-pin-clock             Disable replay clock/seed headers
  --filter <substring>       Replay only requests matching <substring> in path or route
  --out <file.json>          Write full machine-readable JSON replay report to <file.json>

General Options:
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
    if sub == "replay" {
        return parse_replay_args(&raw[1..]);
    }
    if sub == "export-mongo" {
        return parse_export_args(&raw[1..]);
    }
    let is_migrate = match sub {
        "migrate" => true,
        "verify" => false,
        other => {
            return Err(DbError::Config(format!(
                "unknown subcommand {other:?}; expected 'migrate', 'verify', 'export-mongo', or 'replay'"
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
            "--from" | "--against" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config(format!("{arg} requires a MongoDB URI")))?;
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

fn parse_export_args(raw: &[String]) -> Result<Command, DbError> {
    let mut from_source: Option<String> = None;
    let mut to_uri: Option<String> = None;
    let mut areas = Vec::new();
    let mut dry_run = false;
    let mut allow_remote_postgres = false;
    let mut json = false;

    let mut idx = 0;
    while idx < raw.len() {
        let arg = raw[idx].as_str();
        match arg {
            "-h" | "--help" => return Ok(Command::Help),
            "--dry-run" => {
                dry_run = true;
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
                    .ok_or_else(|| DbError::Config("--from requires a SQLite path or postgres:// URL".into()))?;
                from_source = Some(val.clone());
                idx += 2;
            }
            "--to" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--to requires a MongoDB URI".into()))?;
                to_uri = Some(val.clone());
                idx += 2;
            }
            "--node" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--node requires a node ID".into()))?;
                let _ = val
                    .parse::<NodeId>()
                    .map_err(|e| DbError::Config(format!("invalid --node {val:?}: {e}")))?;
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
            other => {
                return Err(DbError::Config(format!("unexpected flag {other:?}")));
            }
        }
    }

    let from_source =
        from_source.ok_or_else(|| DbError::Config("missing required --from <sqlite-file|postgres-url>".into()))?;
    let to_uri = if dry_run && to_uri.is_none() {
        String::new()
    } else {
        to_uri.ok_or_else(|| DbError::Config("missing required --to <mongodb://...>".into()))?
    };

    Ok(Command::ExportMongo {
        opts: ExportOptions {
            from_source,
            to_uri,
            areas,
            dry_run,
            allow_remote_postgres,
        },
        json,
    })
}

fn parse_replay_args(raw: &[String]) -> Result<Command, DbError> {
    let mut corpus: Option<PathBuf> = None;
    let mut left: Option<String> = None;
    let mut right: Option<String> = None;
    let mut left_director: Option<String> = None;
    let mut right_director: Option<String> = None;
    let mut left_chat: Option<String> = None;
    let mut right_chat: Option<String> = None;
    let mut left_jwt_secret: Option<String> = None;
    let mut right_jwt_secret: Option<String> = None;
    let mut left_signing_key: Option<String> = None;
    let mut right_signing_key: Option<String> = None;
    let mut left_gs_key: Option<String> = None;
    let mut right_gs_key: Option<String> = None;
    let mut gateway_secret: Option<String> = None;
    let mut key_map_path: Option<PathBuf> = None;
    let mut pin_clock = true;
    let mut filter: Option<String> = None;
    let mut out_path: Option<PathBuf> = None;
    let mut json = false;

    let mut idx = 0;
    while idx < raw.len() {
        let arg = raw[idx].as_str();
        match arg {
            "-h" | "--help" => return Ok(Command::Help),
            "--json" => {
                json = true;
                idx += 1;
            }
            "--pin-clock" => {
                pin_clock = true;
                idx += 1;
            }
            "--no-pin-clock" => {
                pin_clock = false;
                idx += 1;
            }
            "--corpus" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--corpus requires a path".into()))?;
                corpus = Some(PathBuf::from(val));
                idx += 2;
            }
            "--left" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--left requires a URL or 'recorded'".into()))?;
                left = Some(val.clone());
                idx += 2;
            }
            "--right" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--right requires a URL".into()))?;
                right = Some(val.clone());
                idx += 2;
            }
            "--left-director" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--left-director requires a URL".into()))?;
                left_director = Some(val.clone());
                idx += 2;
            }
            "--right-director" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--right-director requires a URL".into()))?;
                right_director = Some(val.clone());
                idx += 2;
            }
            "--left-chat" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--left-chat requires a host:port".into()))?;
                left_chat = Some(val.clone());
                idx += 2;
            }
            "--right-chat" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--right-chat requires a host:port".into()))?;
                right_chat = Some(val.clone());
                idx += 2;
            }
            "--jwt-secret" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--jwt-secret requires a secret".into()))?;
                left_jwt_secret = Some(val.clone());
                right_jwt_secret = Some(val.clone());
                idx += 2;
            }
            "--left-jwt-secret" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--left-jwt-secret requires a secret".into()))?;
                left_jwt_secret = Some(val.clone());
                idx += 2;
            }
            "--right-jwt-secret" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--right-jwt-secret requires a secret".into()))?;
                right_jwt_secret = Some(val.clone());
                idx += 2;
            }
            "--signing-key" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--signing-key requires a base64 key".into()))?;
                left_signing_key = Some(val.clone());
                right_signing_key = Some(val.clone());
                idx += 2;
            }
            "--left-signing-key" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--left-signing-key requires a base64 key".into()))?;
                left_signing_key = Some(val.clone());
                idx += 2;
            }
            "--right-signing-key" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--right-signing-key requires a base64 key".into()))?;
                right_signing_key = Some(val.clone());
                idx += 2;
            }
            "--gs-key" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--gs-key requires a key".into()))?;
                left_gs_key = Some(val.clone());
                right_gs_key = Some(val.clone());
                idx += 2;
            }
            "--left-gs-key" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--left-gs-key requires a key".into()))?;
                left_gs_key = Some(val.clone());
                idx += 2;
            }
            "--right-gs-key" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--right-gs-key requires a key".into()))?;
                right_gs_key = Some(val.clone());
                idx += 2;
            }
            "--gateway-secret" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--gateway-secret requires a secret".into()))?;
                gateway_secret = Some(val.clone());
                idx += 2;
            }
            "--key-map" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--key-map requires a path".into()))?;
                key_map_path = Some(PathBuf::from(val));
                idx += 2;
            }
            "--filter" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--filter requires a substring".into()))?;
                filter = Some(val.clone());
                idx += 2;
            }
            "--out" => {
                let val = raw
                    .get(idx + 1)
                    .ok_or_else(|| DbError::Config("--out requires a file path".into()))?;
                out_path = Some(PathBuf::from(val));
                idx += 2;
            }
            other => {
                return Err(DbError::Config(format!("unexpected flag {other:?}")));
            }
        }
    }

    let corpus = corpus.ok_or_else(|| DbError::Config("missing required --corpus <path>".into()))?;
    let left = left.ok_or_else(|| DbError::Config("missing required --left <url|recorded>".into()))?;
    let right = right.ok_or_else(|| DbError::Config("missing required --right <url>".into()))?;

    Ok(Command::Replay {
        opts: ReplayOptions {
            corpus,
            left,
            right,
            left_director,
            right_director,
            left_chat,
            right_chat,
            left_jwt_secret,
            right_jwt_secret,
            left_signing_key,
            right_signing_key,
            left_gs_key,
            right_gs_key,
            gateway_secret,
            key_map_path,
            pin_clock,
            filter,
            out_path,
        },
        json,
    })
}
