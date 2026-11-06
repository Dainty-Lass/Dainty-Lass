// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `dainty-host db migrate | verify | export-mongo | replay`: MongoDB-to-SQLite/Postgres
//! migration, verification, and shadow replay commands.

use std::path::Path;
use std::process::ExitCode;

use crate::Outcome;
use crate::cli::{DbArgs, DbCommand, DbExportMongoArgs, DbMigrateArgs, DbReplayArgs, DbVerifyArgs};
use crate::commands::update::{
    mongo_migrated_marker, mongo_uri_with_database, redact_uri_password, store_target_for_migration,
};
use crate::error::HostError;
use crate::hostconfig::{
    DATABASE_FILE, DEFAULT_MONGO_URI, HostConfig, load_host_config, node_database_config, read_env_file,
};
use crate::paths::Paths;

/// Runs `dainty-host db <subcommand>` printing to stdout.
pub fn run(paths: &Paths, args: &DbArgs) -> Outcome {
    run_with(paths, args, &mut |line| println!("{line}"))
}

/// Runs `dainty-host db <subcommand>` sending output lines to `out`.
pub fn run_with(paths: &Paths, args: &DbArgs, out: &mut dyn FnMut(&str)) -> Outcome {
    if let Some(res) = crate::hooks::get().run_db_command(paths, args, out) {
        return res;
    }
    match &args.command {
        DbCommand::Migrate(m) => migrate(paths, m, out),
        DbCommand::Verify(v) => verify(paths, v, out),
        DbCommand::ExportMongo(e) => export_mongo(paths, e, out),
        DbCommand::Replay(r) => replay(paths, r, out),
    }
}

/// Resolves the MongoDB URI from `--from` / `--against` / `--to`, or defaults from `backend.env` and `host.toml`.
pub fn resolve_mongo_uri(paths: &Paths, config: Option<&HostConfig>, explicit: Option<&str>) -> String {
    let backend_env = read_env_file(&paths.config.join("backend.env")).unwrap_or_default();
    if let Some(raw) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return mongo_uri_with_database(raw, backend_env.get("MONGODB_DB"));
    }
    let raw = backend_env
        .get("MONGODB_URI")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| config.map(|c| c.mongo.uri.trim()).filter(|s| !s.is_empty()))
        .unwrap_or(DEFAULT_MONGO_URI);
    mongo_uri_with_database(raw, backend_env.get("MONGODB_DB"))
}

/// Resolves the SQLite or Postgres store target and `allow_remote_postgres` from `--to` / `--from`, or defaults from `host.toml` and `secrets.env`.
pub fn resolve_store_target(
    paths: &Paths,
    config: Option<&HostConfig>,
    explicit: Option<&str>,
    explicit_allow_remote: bool,
) -> Result<(String, bool), HostError> {
    if let Some(raw) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        let cfg_allow = config
            .and_then(|c| c.database.as_ref())
            .and_then(|d| d.allow_remote)
            .unwrap_or(false);
        return Ok((raw.to_owned(), explicit_allow_remote || cfg_allow));
    }
    if let Some(cfg) = config {
        let (target, allow_remote, _) = store_target_for_migration(paths, cfg)?;
        return Ok((target, explicit_allow_remote || allow_remote));
    }
    let sqlite_file = paths.config.join(DATABASE_FILE);
    Ok((sqlite_file.to_string_lossy().into_owned(), explicit_allow_remote))
}

/// Extracts the password from a `scheme://user:password@host...` URI, if present and non-empty.
fn extract_uri_password(uri: &str) -> Option<&str> {
    let (_scheme, rest) = uri.split_once("://")?;
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, _tail) = rest.split_at(end);
    let (userinfo, _hostport) = authority.rsplit_once('@')?;
    let (_user, password) = userinfo.split_once(':')?;
    (!password.is_empty()).then_some(password)
}

/// Redacts any credentials from `text` so passwords in `postgres://` or `mongodb://` URLs never appear in output or errors.
pub fn redact_credentials_in_text(text: &str, uris: &[&str]) -> String {
    let mut out = text.to_owned();
    for uri in uris {
        let redacted = redact_uri_password(uri);
        if redacted != *uri {
            out = out.replace(uri, &redacted);
        }
        if let Some(pw) = extract_uri_password(uri) {
            out = out.replace(pw, "***");
        }
    }
    out
}

fn resolve_db_config(
    p: &Paths,
    config: Option<&HostConfig>,
    to: Option<&str>,
) -> Result<store::DatabaseConfig, HostError> {
    if let Some(target) = to {
        if target.starts_with("postgres://") || target.starts_with("postgresql://") {
            let parsed = url::Url::parse(target).map_err(|e| HostError::message(format!("--to {target}: {e}")))?;
            let host = parsed.host_str().unwrap_or("127.0.0.1").to_owned();
            let port = parsed.port().unwrap_or(5432);
            let user = if parsed.username().is_empty() {
                "dainty".to_owned()
            } else {
                parsed.username().to_owned()
            };
            let password = parsed.password().unwrap_or_default().to_owned();
            let database = parsed.path().trim_start_matches('/').to_owned();
            let database = if database.is_empty() {
                "dainty".to_owned()
            } else {
                database
            };
            return Ok(store::DatabaseConfig::Postgres(store::PostgresConfig {
                host,
                port,
                user,
                password,
                database,
                allow_remote: true,
            }));
        }
        return Ok(store::DatabaseConfig::Sqlite { file: target.into() });
    }
    if let Some(c) = config {
        node_database_config(c, p)
    } else {
        Ok(store::DatabaseConfig::Sqlite {
            file: p.config.join(DATABASE_FILE),
        })
    }
}

fn migrate(p: &Paths, args: &DbMigrateArgs, out: &mut dyn FnMut(&str)) -> Outcome {
    let config = load_host_config(p)?;
    let from = resolve_mongo_uri(p, config.as_ref(), args.from.as_deref());
    let db_cfg = resolve_db_config(p, config.as_ref(), args.to.as_deref())?;
    let (counts, mismatches) = crate::hooks::get().db_migrate(&from, &db_cfg, args.dry_run)?;
    let total: usize = counts.iter().map(|(_, n)| *n).sum();
    for (name, n) in &counts {
        out(&format!("  {name}: {n} record(s)"));
    }
    if !mismatches.is_empty() {
        return Err(HostError::message(format!(
            "Verification found {} mismatch(es)",
            mismatches.len()
        )));
    }
    if args.dry_run {
        out(&format!(
            "Dry run verified {total} record(s) across {} collection(s).",
            counts.len()
        ));
    } else {
        let marker = mongo_migrated_marker(p);
        if let Some(parent) = marker.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&marker, format!("migrated from {from}\n"));
        out(&format!(
            "Migrated and verified {total} record(s) across {} collection(s).",
            counts.len()
        ));
    }
    Ok(ExitCode::SUCCESS)
}

fn verify(p: &Paths, args: &DbVerifyArgs, out: &mut dyn FnMut(&str)) -> Outcome {
    let config = load_host_config(p)?;
    let from = resolve_mongo_uri(p, config.as_ref(), args.against.as_deref());
    let db_cfg = resolve_db_config(p, config.as_ref(), args.to.as_deref())?;
    let mismatches = crate::hooks::get().db_verify(&from, &db_cfg)?;
    if mismatches.is_empty() {
        out("Database verification passed: all collections match.");
        Ok(ExitCode::SUCCESS)
    } else {
        let names: Vec<&str> = mismatches.iter().map(|(c, _)| c.as_str()).collect();
        Err(HostError::message(format!(
            "db verify failed: differences in [{}]",
            names.join(", ")
        )))
    }
}

fn export_mongo(p: &Paths, args: &DbExportMongoArgs, out: &mut dyn FnMut(&str)) -> Outcome {
    let config = load_host_config(p)?;
    let from = resolve_mongo_uri(p, config.as_ref(), args.from.as_deref());
    let target_dir = args.to.as_deref().unwrap_or(".");
    let counts = crate::hooks::get().db_export_mongo(&from, Path::new(target_dir))?;
    let total: usize = counts.iter().map(|(_, n)| *n).sum();
    for (name, n) in &counts {
        out(&format!("  {name}: {n} document(s)"));
    }
    out(&format!(
        "Exported {total} document(s) across {} collection(s) to {target_dir}.",
        counts.len()
    ));
    Ok(ExitCode::SUCCESS)
}

fn replay(_p: &Paths, _args: &DbReplayArgs, _out: &mut dyn FnMut(&str)) -> Outcome {
    Err(HostError::message("shadow replay is not registered in this build"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]
mod tests {
    use super::*;
    use crate::hostconfig::{DatabaseSettings, FileForm, HostConfig, MongoConfig, Registration, ServerMode};

    #[test]
    fn defaults_source_and_target_from_host_toml_and_never_prints_passwords() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(
            paths.config.join("secrets.env"),
            "DAINTY_DB_PASSWORD=\"pgSuperSecret42\"\n",
        )
        .unwrap();

        let cfg = HostConfig {
            version: 1,
            mode: ServerMode::Private,
            name: "Test Server".into(),
            host: "100.64.0.1".into(),
            game_address: "100.64.0.1".into(),
            public_port: None,
            game_dir: None,
            registration: Registration::Invitecode,
            share_url: None,
            dashboard_url: None,
            max_hunt_servers: 3,
            mongo: MongoConfig {
                uri: "mongodb://admin:mongoSecret99@127.0.0.1:27017/reforged?replicaSet=rs0".into(),
                manage: false,
                mongod: None,
            },
            performance: None,
            multipliers: None,
            linux: None,
            database: Some(DatabaseSettings {
                engine: Some("postgres".into()),
                postgres_host: Some("127.0.0.1".into()),
                postgres_port: Some(5432),
                postgres_user: Some("dainty".into()),
                postgres_database: Some("dainty".into()),
                allow_remote: Some(false),
                fresh_start: None,
            }),
            guard: None,
            history: None,
            switches: None,
            modules: None,
            services: None,
            env: None,
            network: None,
            remote: None,
            split: None,
            logs: None,
            file: FileForm::default(),
        };
        let mongo_uri = resolve_mongo_uri(&paths, Some(&cfg), None);
        assert_eq!(
            mongo_uri,
            "mongodb://admin:mongoSecret99@127.0.0.1:27017/reforged?replicaSet=rs0"
        );
        let (store_target, allow_remote) = resolve_store_target(&paths, Some(&cfg), None, false).unwrap();
        assert_eq!(store_target, "postgres://dainty:pgSuperSecret42@127.0.0.1:5432/dainty");
        assert!(!allow_remote);

        let err_text = format!("failed connecting to {store_target} or {mongo_uri} (pw=pgSuperSecret42)");
        let cleaned = redact_credentials_in_text(&err_text, &[&store_target, &mongo_uri]);
        assert!(!cleaned.contains("pgSuperSecret42"));
        assert!(!cleaned.contains("mongoSecret99"));
        assert!(cleaned.contains("postgres://dainty:***@127.0.0.1:5432/dainty"));
        assert!(cleaned.contains("mongodb://admin:***@127.0.0.1:27017/reforged?replicaSet=rs0"));
    }
}
