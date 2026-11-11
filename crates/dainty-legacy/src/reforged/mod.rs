// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Reforged Node-kit upgrade (spec §11): `host.json` to `host.toml`, MongoDB into the 1.14.7 database,
//! the old folders' contents to where Dainty Lass looks, and `secrets.env` checked. Each step finishes before
//! the next, so a failure leaves the rest as it was.

pub mod hostjson;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use dainty_store::DatabaseConfig;
use serde::Deserialize;

use crate::args::ImportArgs;
use crate::data::{self, Reforged};
use crate::envfile;
use crate::error::ImportError;
use crate::report::Report;

/// The largest `host.json` read.
pub const MAX_HOST_JSON: u64 = 262_144;
/// Written once MongoDB was moved or left behind on purpose (the host kit's `MONGO_MIGRATED_MARKER`).
const MARKER: &str = ".mongo-migrated";
const DEFAULT_MONGO: &str = "mongodb://127.0.0.1:27017";

/// The checkout's files.
struct Layout {
    root: PathBuf,
    config: PathBuf,
    host_json: PathBuf,
    host_toml: PathBuf,
    mongo_data: PathBuf,
}

impl Layout {
    fn new(root: &Path) -> Self {
        let workspace = root.join(".workspace");
        let config = workspace.join("config");
        Self {
            root: root.to_path_buf(),
            host_json: config.join("host.json"),
            host_toml: config.join("host.toml"),
            mongo_data: workspace.join("mongodb"),
            config,
        }
    }
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> ImportError + '_ {
    move |e| ImportError::io(path, e)
}

/// Runs the upgrade of the checkout `args.source`.
pub(crate) async fn run(args: &ImportArgs) -> Result<Report, ImportError> {
    let args = args.clone();
    tokio::task::spawn_blocking(move || upgrade(&args))
        .await
        .map_err(|_| ImportError::Io("the upgrade's task stopped".into()))?
}

fn upgrade(args: &ImportArgs) -> Result<Report, ImportError> {
    let data = data::reforged()?;
    let layout = Layout::new(&args.source);
    let mut report = Report::new("reforged", args.dry_run);

    // 1. host.json, bounded
    let json_text = if layout.host_json.exists() {
        let meta = std::fs::metadata(&layout.host_json).map_err(io(&layout.host_json))?;
        if !meta.is_file() || meta.len() > MAX_HOST_JSON {
            return Err(ImportError::Invalid(format!(
                "{}: not a host.json of at most {MAX_HOST_JSON} bytes",
                layout.host_json.display()
            )));
        }
        let bytes = std::fs::read(&layout.host_json).map_err(io(&layout.host_json))?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    } else {
        None
    };
    let toml_exists = layout.host_toml.exists();
    if json_text.is_none() && !toml_exists {
        return Err(ImportError::Invalid(format!(
            "{}: no .workspace/config/host.json or host.toml, so not a Reforged install",
            layout.root.display()
        )));
    }

    // 2. Converted before anything moves, so a bad file changes nothing
    let converted = match (&json_text, toml_exists) {
        (Some(text), false) => Some(hostjson::convert(text, data).map_err(ImportError::Invalid)?),
        _ => None,
    };
    let host_json: Option<serde_json::Value> = json_text.as_deref().and_then(|t| serde_json::from_str(t).ok());

    mongodb(args, &layout, host_json.as_ref(), &mut report)?;
    folders(args, &layout, data, &mut report)?;
    secrets(args, &layout, data, &mut report)?;

    // 6. host.toml
    if let Some(converted) = converted {
        if !args.dry_run {
            let partial = layout.config.join("host.toml.partial");
            std::fs::write(&partial, &converted.toml).map_err(io(&partial))?;
            std::fs::rename(&partial, &layout.host_toml).map_err(io(&layout.host_toml))?;
            let bak = layout.config.join("host.json.bak");
            if bak.exists() {
                std::fs::remove_file(&bak).map_err(io(&bak))?;
            }
            std::fs::rename(&layout.host_json, &bak).map_err(io(&layout.host_json))?;
        }
        report.count("host.toml");
        report.note("host.json is now host.toml; the old file is kept as host.json.bak");
        if !converted.dropped.is_empty() {
            report.note(format!(
                "left out of host.toml (1.4.4, 1.12 or unknown): {}",
                converted.dropped.join(", ")
            ));
        }
        if converted.legacy_only {
            report.note(
                "this server ran only Dauntless 1.4.4: set game_dir in host.toml to a 1.14.7 install before setup",
            );
        }
    } else {
        report.same("host.toml");
        if json_text.is_some() {
            report.note("host.toml is already there, so host.json was left as it is: delete it once you've checked");
        }
    }

    // 7.
    for old in &data.old_folders {
        if layout.root.join(old).exists() {
            report.note(format!(
                "{old} is no longer used: delete it once you've checked what moved"
            ));
        }
    }
    report.note("next: run dainty-host setup, which checks host.toml and writes the services' settings");
    Ok(report)
}

/// The URI with its user and password hidden: `user:pass@` is `user:***@`, a bare `user@` `***@`.
pub fn redact(uri: &str) -> String {
    let (scheme, rest) = uri.split_once("://").map_or(("", uri), |(s, r)| (s, r));
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let Some(at) = authority.rfind('@') else {
        return uri.to_owned();
    };
    let (userinfo, host) = authority.split_at(at);
    let hidden = match userinfo.split_once(':') {
        Some((user, _)) => format!("{user}:***"),
        None => String::from("***"),
    };
    let scheme = if scheme.is_empty() {
        String::new()
    } else {
        format!("{scheme}://")
    };
    format!("{scheme}{hidden}{host}{tail}")
}

/// The URI with `database` as its path when it names none.
fn with_database(uri: &str, fallback: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return uri.to_owned();
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let path_end = tail.find(['?', '#']).unwrap_or(tail.len());
    let (path, query) = tail.split_at(path_end);
    if path.trim_start_matches('/').is_empty() {
        format!("{scheme}://{authority}/{fallback}{query}")
    } else {
        uri.to_owned()
    }
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// `db migrate`'s `--to`, and whether it needs `--allow-remote-postgres`.
fn store_target(config: &DatabaseConfig) -> (String, bool) {
    match config {
        DatabaseConfig::Sqlite { file } => (file.to_string_lossy().into_owned(), false),
        DatabaseConfig::Postgres(pg) => {
            let password = if pg.password.is_empty() {
                String::new()
            } else {
                format!(":{}", encode(&pg.password))
            };
            let host = if pg.host.contains(':') {
                format!("[{}]", pg.host)
            } else {
                pg.host.clone()
            };
            (
                format!(
                    "postgres://{}{password}@{host}:{}/{}",
                    encode(&pg.user),
                    pg.port,
                    encode(&pg.database)
                ),
                pg.allow_remote,
            )
        }
    }
}

#[derive(Deserialize)]
struct VerifyReport {
    collections: Vec<VerifyCollection>,
}

#[derive(Deserialize)]
struct VerifyCollection {
    name: String,
    ok: bool,
}

/// 3. MongoDB (§11.5).
fn mongodb(
    args: &ImportArgs,
    layout: &Layout,
    host_json: Option<&serde_json::Value>,
    report: &mut Report,
) -> Result<(), ImportError> {
    let marker = layout.config.join(MARKER);
    if marker.exists() {
        report.same("mongodb");
        return Ok(());
    }
    let has_data = match std::fs::read_dir(&layout.mongo_data) {
        Ok(entries) => entries.filter_map(Result::ok).any(|e| e.file_name() != "mongod.lock"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(ImportError::io(&layout.mongo_data, e)),
    };
    if !has_data {
        return Ok(());
    }
    let backend = envfile::read_optional(&layout.config.join("backend.env"))?.unwrap_or_default();
    let nonempty = |v: Option<&String>| v.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let uri = nonempty(backend.get("MONGODB_URI"))
        .or_else(|| {
            host_json
                .and_then(|j| j.get("mongo"))
                .and_then(|m| m.get("uri"))
                .and_then(serde_json::Value::as_str)
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_MONGO.to_owned());
    let database = nonempty(backend.get("MONGODB_DB")).unwrap_or_else(|| String::from("reforged"));
    let uri = with_database(&uri, &database);
    let shown = redact(&uri);

    let Some(target) = &args.target else {
        if args.fresh_start {
            if !args.dry_run {
                std::fs::write(&marker, "fresh-start\n").map_err(io(&marker))?;
            }
            report.note(format!(
                "MongoDB ({shown}) was not moved (--fresh-start); its files are kept"
            ));
            return Ok(());
        }
        return Err(ImportError::Usage(format!(
            "this install has MongoDB data ({shown}): give the 1.14.7 database to move it into (--to), or \
             --fresh-start to start on a new one and keep MongoDB as it is"
        )));
    };
    if args.dry_run {
        report.note(format!(
            "MongoDB ({shown}) would be moved with dainty-host db migrate and verify"
        ));
        return Ok(());
    }
    let (to, allow_remote) = store_target(&envfile::database(target)?);
    let program = args.db_tool.clone().unwrap_or_else(|| PathBuf::from("dainty-host"));
    let tool = |verb: &str, from_flag: &str| {
        let mut command = Command::new(&program);
        command
            .args(["db", verb, from_flag, &uri, "--to", &to, "--json"])
            .stdin(Stdio::null())
            .stderr(Stdio::inherit());
        if allow_remote {
            command.arg("--allow-remote-postgres");
        }
        // Output, not status: stdout must never reach the module's frame pipe
        command.output().map_err(io(&program))
    };
    let migrate = tool("migrate", "--from")?;
    if !migrate.status.success() {
        return Err(ImportError::Io(format!(
            "dainty-host db migrate from {shown} failed ({}): is MongoDB running?",
            migrate.status
        )));
    }
    let verify = tool("verify", "--against")?;
    let parsed: VerifyReport = serde_json::from_slice(&verify.stdout)
        .map_err(|_| ImportError::Io(format!("dainty-host db verify gave no report ({})", verify.status)))?;
    let differ: Vec<&str> = parsed
        .collections
        .iter()
        .filter(|c| !c.ok)
        .map(|c| c.name.as_str())
        .collect();
    if !differ.is_empty() {
        return Err(ImportError::Io(format!(
            "dainty-host db verify found differences in [{}]: nothing else of the old install was changed, and \
             MongoDB was not modified",
            differ.join(", ")
        )));
    }
    if !verify.status.success() {
        return Err(ImportError::Io(format!(
            "dainty-host db verify failed ({})",
            verify.status
        )));
    }
    std::fs::write(&marker, format!("migrated from {shown}\n")).map_err(io(&marker))?;
    report.count("mongodb");
    report.note(format!("MongoDB ({shown}) moved and verified; its files are kept"));
    Ok(())
}

/// 4. Folders (§11.3): moved, never overwritten or deleted.
fn folders(args: &ImportArgs, layout: &Layout, data: &Reforged, report: &mut Report) -> Result<(), ImportError> {
    // On a dry run, what a real run would have put in place by now
    let mut planned: BTreeSet<PathBuf> = BTreeSet::new();
    for (from, to) in &data.moves {
        let pairs: Vec<(String, String)> = if from.contains("<sdk>") {
            data.sdk_files
                .iter()
                .map(|f| (from.replace("<sdk>", f), to.replace("<sdk>", f)))
                .collect()
        } else {
            vec![(from.clone(), to.clone())]
        };
        for (from, to) in pairs {
            let source = layout.root.join(&from);
            let destination = layout.root.join(&to);
            if std::fs::symlink_metadata(&source).is_err() || source == destination {
                continue;
            }
            if destination.exists() || planned.contains(&destination) {
                report.skip("folder", &from, format!("{to} is already there"));
                continue;
            }
            if args.dry_run {
                planned.insert(destination);
            } else {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent).map_err(io(parent))?;
                }
                std::fs::rename(&source, &destination).map_err(io(&source))?;
            }
            report.count("folders moved");
            report.note(format!("moved {from} -> {to}"));
        }
    }
    Ok(())
}

/// 5. `secrets.env` (§11.4): carried untouched; only which names have a value is read.
fn secrets(args: &ImportArgs, layout: &Layout, data: &Reforged, report: &mut Report) -> Result<(), ImportError> {
    let path = layout.config.join("secrets.env");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            report.note("there is no secrets.env: dainty-host setup makes a new one, which logs every player out");
            return Ok(());
        }
        Err(e) => return Err(ImportError::io(&path, e)),
    };
    let names = envfile::secret_names(&String::from_utf8_lossy(&bytes));
    drop(bytes);
    report.same("secrets.env");
    let mut old = Vec::new();
    let mut lacking = Vec::new();
    for key in &data.secret_keys {
        if names.contains(key) {
            continue;
        }
        match data.secret_renames.get(key) {
            Some(old_name) if names.contains(old_name) => old.push(old_name.as_str()),
            _ => lacking.push(key.as_str()),
        }
    }
    if !old.is_empty() {
        report.note(format!(
            "secrets.env keeps the old names {}; dainty-host setup moves each to its new name, same value",
            old.join(", ")
        ));
    }
    if !lacking.is_empty() {
        report.note(format!(
            "secrets.env is kept; dainty-host setup adds what it lacks: {}",
            lacking.join(", ")
        ));
    }
    #[cfg(unix)]
    if !args.dry_run {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(io(&path))?;
    }
    #[cfg(not(unix))]
    let _ = args;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_and_names_the_database() {
        assert_eq!(redact("mongodb://u:p@h:1/db"), "mongodb://u:***@h:1/db");
        assert_eq!(redact("mongodb://u@h"), "mongodb://***@h");
        assert_eq!(redact("mongodb://h:27017"), "mongodb://h:27017");
        assert_eq!(
            with_database("mongodb://h:27017", "reforged"),
            "mongodb://h:27017/reforged"
        );
        assert_eq!(
            with_database("mongodb://h:27017/?replicaSet=a", "x"),
            "mongodb://h:27017/x?replicaSet=a"
        );
        assert_eq!(with_database("mongodb://h/live", "x"), "mongodb://h/live");
    }

    #[test]
    fn postgres_targets_are_urls() {
        let config = DatabaseConfig::Postgres(dainty_store::PostgresConfig {
            host: "db.lan".into(),
            port: 5433,
            user: "dainty".into(),
            database: "isles".into(),
            password: "p@ss word".into(),
            allow_remote: true,
        });
        assert_eq!(
            store_target(&config),
            ("postgres://dainty:p%40ss%20word@db.lan:5433/isles".into(), true)
        );
    }
}
