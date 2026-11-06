// `redact_mongo_uri`, `mongo_uri_with_database`, `resolve_mongo_uri` and the marker are DaintyLass's
// `rust/crates/hostkit/src/commands/update.rs` and `db.rs` (plan `fb46a4a`), MIT, Copyright (c) 2026 Silent Spud;
// LICENSE-MIT.txt there.

//! The Node kit's MongoDB, moved into the 1.14.7 database by the host kit's `reforged-host db migrate` and checked
//! by `db verify` before anything else of the old install changes. A verified move leaves the
//! `.mongo-migrated` marker, which DaintyLass reads, and MongoDB's own files untouched.

use std::path::{Path, PathBuf};
use std::process::Command;

use store::DatabaseConfig;

use crate::error::{ImportError, Result};
use crate::reforged::layout::Layout;
use crate::report::Report;

pub const MONGO_MIGRATED_MARKER: &str = ".mongo-migrated";
const DEFAULT_MONGO_URI: &str = "mongodb://127.0.0.1:27017";

#[must_use]
pub fn marker(l: &Layout) -> PathBuf {
    l.config.join(MONGO_MIGRATED_MARKER)
}

/// The URI with its password replaced, for messages and the marker.
#[must_use]
pub fn redact_mongo_uri(uri: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return uri.to_owned();
    };
    let (authority, tail) = match rest.find(['/', '?']) {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };
    let Some((userinfo, hostpart)) = authority.rsplit_once('@') else {
        return uri.to_owned();
    };
    let redacted_user = match userinfo.split_once(':') {
        Some((user, _)) => format!("{user}:***"),
        None => "***".to_owned(),
    };
    format!("{scheme}://{redacted_user}@{hostpart}{tail}")
}

#[must_use]
pub fn mongo_uri_with_database(raw_uri: &str, db_override: Option<&str>) -> String {
    let db = db_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("reforged");
    let Some((scheme, rest)) = raw_uri.trim().split_once("://") else {
        return raw_uri.trim().to_owned();
    };
    let (before_query, query) = match rest.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (rest, None),
    };
    let (authority, path) = match before_query.split_once('/') {
        Some((a, p)) => (a, p.trim_matches('/')),
        None => (before_query, ""),
    };
    let effective_db = if path.is_empty() { db } else { path };
    match query {
        Some(q) => format!("{scheme}://{authority}/{effective_db}?{q}"),
        None => format!("{scheme}://{authority}/{effective_db}"),
    }
}

fn env_value(file: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    text.lines()
        .filter_map(|line| line.trim().split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().trim_matches(['"', '\'']).to_owned())
        .filter(|value| !value.is_empty())
}

/// The kit's MongoDB: `backend.env`'s `MONGODB_URI` and `MONGODB_DB`, else host.json's `mongo.uri`, else the
/// managed local one.
#[must_use]
pub fn resolve_mongo_uri(l: &Layout, host_json_uri: Option<&str>) -> String {
    let backend_env = l.config.join("backend.env");
    let raw = env_value(&backend_env, "MONGODB_URI")
        .or_else(|| {
            host_json_uri
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| DEFAULT_MONGO_URI.to_owned());
    mongo_uri_with_database(&raw, env_value(&backend_env, "MONGODB_DB").as_deref())
}

/// MongoDB data the kit managed, besides its lock file.
#[must_use]
pub fn has_unmigrated_mongo_dir(l: &Layout) -> bool {
    let Ok(entries) = std::fs::read_dir(&l.mongo_data) else {
        return false;
    };
    entries.flatten().any(|e| e.file_name().to_str() != Some("mongod.lock"))
}

/// The `--to` argument of `reforged-host db` for a store database (hostkit's `store_target_for_migration`).
#[must_use]
pub fn db_tool_target(config: &DatabaseConfig) -> (String, bool) {
    match config {
        DatabaseConfig::Sqlite { file } => (file.to_string_lossy().into_owned(), false),
        DatabaseConfig::Postgres(pg) => {
            let auth = if pg.password.is_empty() {
                pg.user.clone()
            } else {
                format!("{}:{}", pg.user, pg.password)
            };
            (
                format!("postgres://{auth}@{}:{}/{}", pg.host, pg.port, pg.database),
                pg.allow_remote,
            )
        }
    }
}

/// Moves a MongoDB into a 1.14.7 database and checks the result: `reforged-host db migrate` and `verify`.
pub trait MongoMigrator {
    /// Copies every document; nothing in MongoDB changes.
    fn migrate(&self, uri: &str, target: &DatabaseConfig) -> Result<()>;
    /// The collections that differ between MongoDB and the target; empty when they match.
    fn verify(&self, uri: &str, target: &DatabaseConfig) -> Result<Vec<String>>;
}

/// What `--db-tool` names when it isn't given, from `PATH`.
pub const DEFAULT_DB_TOOL: &str = "reforged-host";

/// The host kit's database tool, run as `<program> db ...` (dainty-lass's MIT `rustic-isles-db`), from a path or
/// `PATH`. It finds its server folder from the working directory or its own location.
pub struct DbTool {
    pub program: PathBuf,
}

impl DbTool {
    fn run(&self, args: &[&str]) -> Result<std::process::Output> {
        Command::new(&self.program)
            .arg("db")
            .args(args)
            .output()
            .map_err(|e| ImportError::Io(format!("{}: {e}", self.program.display())))
    }

    fn common<'a>(verb: &'a str, from: &'a str, uri: &'a str, to: &'a str, remote: bool) -> Vec<&'a str> {
        let mut args = vec![verb, from, uri, "--to", to, "--json"];
        if remote {
            args.push("--allow-remote-postgres");
        }
        args
    }
}

impl MongoMigrator for DbTool {
    fn migrate(&self, uri: &str, target: &DatabaseConfig) -> Result<()> {
        let (to, remote) = db_tool_target(target);
        let out = self.run(&Self::common("migrate", "--from", uri, &to, remote))?;
        if !out.status.success() {
            // The tool's own messages redact the URI; its exit status is what counts here
            return Err(ImportError::invalid(format!(
                "reforged-host db migrate from {} failed ({}): is MongoDB running?",
                redact_mongo_uri(uri),
                out.status
            )));
        }
        Ok(())
    }

    fn verify(&self, uri: &str, target: &DatabaseConfig) -> Result<Vec<String>> {
        let (to, remote) = db_tool_target(target);
        let out = self.run(&Self::common("verify", "--against", uri, &to, remote))?;
        let report: serde_json::Value = serde_json::from_slice(&out.stdout)
            .map_err(|_| ImportError::invalid(format!("reforged-host db verify gave no report ({})", out.status)))?;
        let differ = report["collections"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter(|c| c["ok"].as_bool() != Some(true))
                    .map(|c| c["name"].as_str().unwrap_or("?").to_owned())
                    .collect()
            })
            .unwrap_or_default();
        if !out.status.success() && Vec::<String>::is_empty(&differ) {
            return Err(ImportError::invalid(format!(
                "reforged-host db verify failed ({})",
                out.status
            )));
        }
        Ok(differ)
    }
}

/// What to do with the kit's MongoDB.
pub enum MongoPlan<'a> {
    /// Move it into this database with this migrator
    Migrate {
        target: &'a DatabaseConfig,
        migrator: &'a dyn MongoMigrator,
    },
    /// Start 1.14.7 on a new database; MongoDB is kept as it is (setup's `--fresh-start`)
    FreshStart,
    /// Nothing given: refuse when there is data to move
    Refuse,
}

/// Moves the kit's MongoDB when there is one that hasn't been moved, before anything else changes.
pub fn upgrade(
    l: &Layout,
    host_json_uri: Option<&str>,
    plan: &MongoPlan<'_>,
    dry_run: bool,
    report: &mut Report,
) -> Result<()> {
    let marker = marker(l);
    if marker.is_file() {
        report.unchanged("mongodb");
        return Ok(());
    }
    if !has_unmigrated_mongo_dir(l) {
        return Ok(());
    }
    let uri = resolve_mongo_uri(l, host_json_uri);
    let redacted = redact_mongo_uri(&uri);
    match plan {
        MongoPlan::Refuse => Err(ImportError::Usage(format!(
            "this install has MongoDB data ({redacted}): give the 1.14.7 database to move it into (--to), or \
             --fresh-start to start on a new one and keep MongoDB as it is"
        ))),
        MongoPlan::FreshStart => {
            if !dry_run {
                std::fs::create_dir_all(&l.config).map_err(|e| ImportError::io(&l.config, e))?;
                std::fs::write(&marker, "fresh-start\n").map_err(|e| ImportError::io(&marker, e))?;
            }
            report.note(format!(
                "MongoDB ({redacted}) was not moved (--fresh-start); its files are kept"
            ));
            Ok(())
        }
        MongoPlan::Migrate { target, migrator } => {
            if dry_run {
                report.note(format!(
                    "MongoDB ({redacted}) would be moved with reforged-host db migrate and verify"
                ));
                return Ok(());
            }
            migrator.migrate(&uri, target)?;
            let differ = migrator.verify(&uri, target)?;
            if !differ.is_empty() {
                return Err(ImportError::invalid(format!(
                    "reforged-host db verify found differences in [{}]: nothing else of the old install was changed, and \
                     MongoDB was not modified",
                    differ.join(", ")
                )));
            }
            std::fs::write(&marker, format!("migrated from {redacted}\n")).map_err(|e| ImportError::io(&marker, e))?;
            report.count("mongodb");
            report.note(format!("MongoDB ({redacted}) moved and verified; its files are kept"));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_get_a_database_and_lose_their_password() {
        assert_eq!(
            redact_mongo_uri("mongodb://admin:hunter2@db.example:27017/reforged?authSource=admin"),
            "mongodb://admin:***@db.example:27017/reforged?authSource=admin"
        );
        assert_eq!(
            redact_mongo_uri("mongodb://127.0.0.1:27017"),
            "mongodb://127.0.0.1:27017"
        );
        assert_eq!(
            mongo_uri_with_database("mongodb://127.0.0.1:27017", None),
            "mongodb://127.0.0.1:27017/reforged"
        );
        assert_eq!(
            mongo_uri_with_database("mongodb://h:1/?x=1", Some("other")),
            "mongodb://h:1/other?x=1"
        );
        assert_eq!(
            mongo_uri_with_database("mongodb://h:1/kept", Some("other")),
            "mongodb://h:1/kept"
        );
    }
}
