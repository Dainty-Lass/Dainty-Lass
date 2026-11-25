// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `dainty-host db migrate | verify | replay`: an old install's MongoDB moved into the store and checked
//! (dainty-legacy's `import reforged` runs them), and the shadow replay.

use std::path::PathBuf;
use std::process::ExitCode;

use crate::Outcome;
use crate::cli::{DbArgs, DbCommand, DbMigrateArgs, DbReplayArgs, DbVerifyArgs};
use crate::error::HostError;
use crate::hostconfig::{DATABASE_FILE, HostConfig, load_host_config, node_database_config};
use crate::paths::Paths;

/// The MongoDB a Node kit ran, when `--from` names none.
pub const DEFAULT_MONGO_URI: &str = "mongodb://127.0.0.1:27017";

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
        DbCommand::Replay(r) => replay(paths, r, out),
    }
}

/// The MongoDB URI `--from` or `--against` gives, else [`DEFAULT_MONGO_URI`], with a database (`reforged` when it
/// names none).
#[must_use]
pub fn resolve_mongo_uri(explicit: Option<&str>) -> String {
    let raw = explicit
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_MONGO_URI);
    mongo_uri_with_database(raw, None)
}

/// `uri` with the password in its user info replaced, for printing.
#[must_use]
pub fn redact_uri_password(uri: &str) -> String {
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

/// `raw_uri` with a database in its path: the one it names, else `db_override`, else `reforged`.
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

/// The store a migration writes to: its target (a SQLite file or a postgres:// URL with the password),
/// whether a remote Postgres is allowed, and the SQLite file when it is one.
pub fn store_target_for_migration(
    paths: &Paths,
    config: &HostConfig,
) -> Result<(String, bool, Option<PathBuf>), HostError> {
    match node_database_config(config, paths)? {
        dainty_store::DatabaseConfig::Sqlite { file } => {
            let target = file.to_string_lossy().into_owned();
            Ok((target, false, Some(file)))
        }
        dainty_store::DatabaseConfig::Postgres(pg) => {
            let auth = if pg.password.is_empty() {
                pg.user
            } else {
                format!("{}:{}", pg.user, pg.password)
            };
            let target = format!("postgres://{auth}@{}:{}/{}", pg.host, pg.port, pg.database);
            Ok((target, pg.allow_remote, None))
        }
    }
}

/// Resolves the SQLite or Postgres store target and `allow_remote_postgres` from `--to` / `--from`, or defaults from `settings.toml` and `secrets.env`.
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
) -> Result<dainty_store::DatabaseConfig, HostError> {
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
            return Ok(dainty_store::DatabaseConfig::Postgres(dainty_store::PostgresConfig {
                host,
                port,
                user,
                password,
                database,
                allow_remote: true,
            }));
        }
        return Ok(dainty_store::DatabaseConfig::Sqlite { file: target.into() });
    }
    if let Some(c) = config {
        node_database_config(c, p)
    } else {
        Ok(dainty_store::DatabaseConfig::Sqlite {
            file: p.config.join(DATABASE_FILE),
        })
    }
}

fn migrate(p: &Paths, args: &DbMigrateArgs, out: &mut dyn FnMut(&str)) -> Outcome {
    let config = load_host_config(p)?;
    let from = resolve_mongo_uri(args.from.as_deref());
    let db_cfg = resolve_db_config(p, config.as_ref(), args.to.as_deref())?;
    let (counts, mismatches) = crate::hooks::get().db_migrate(p, &from, &db_cfg, args.dry_run)?;
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
        out(&format!(
            "Migrated and verified {total} record(s) across {} collection(s).",
            counts.len()
        ));
    }
    Ok(ExitCode::SUCCESS)
}

fn verify(p: &Paths, args: &DbVerifyArgs, out: &mut dyn FnMut(&str)) -> Outcome {
    let config = load_host_config(p)?;
    let from = resolve_mongo_uri(args.against.as_deref());
    let db_cfg = resolve_db_config(p, config.as_ref(), args.to.as_deref())?;
    let mismatches = crate::hooks::get().db_verify(p, &from, &db_cfg)?;
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

fn replay(_p: &Paths, _args: &DbReplayArgs, _out: &mut dyn FnMut(&str)) -> Outcome {
    Err(HostError::message("shadow replay is not registered in this build"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]
mod tests {
    use super::*;
    use crate::hostconfig::{DatabaseSettings, FileForm, HostConfig, Registration, ServerMode};

    #[test]
    fn defaults_the_target_from_settings_toml_and_never_prints_passwords() {
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
            version_data: None,
            version_facts: None,
            max_hunt_servers: 3,
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
            gateway: None,
            content: None,
            allowlist: None,
            backend: None,
            director: None,
            network: None,
            remote: None,
            split: None,
            logs: None,
            events: None,
            ports: None,
            discord: None,
            oauth: None,
            file: FileForm::default(),
        };
        assert_eq!(resolve_mongo_uri(None), "mongodb://127.0.0.1:27017/reforged");
        let mongo_uri = resolve_mongo_uri(Some("mongodb://admin:mongoSecret99@127.0.0.1:27017/?replicaSet=rs0"));
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
