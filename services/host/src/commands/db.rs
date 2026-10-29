//! `reforged-host db`: `migrate`, `verify` and `export-mongo` (RB4, RB13, RU7, RU9), calling
//! `isles-db`'s library with source and target defaulted from `host.toml` and `secrets.env`.

use std::path::PathBuf;
use std::process::ExitCode;

use crate::Outcome;
use crate::cli::{DbArgs, DbCommand, DbExportMongoArgs, DbMigrateArgs, DbVerifyArgs};
use crate::commands::update::{
    MONGO_MIGRATED_MARKER, mongo_uri_with_database, redact_uri_password, store_target_for_migration,
};
use crate::error::HostError;
use crate::hostconfig::{DATABASE_FILE, DEFAULT_MONGO_URI, HostConfig, load_host_config, read_env_file};
use crate::paths::Paths;

/// Runs `reforged-host db <subcommand>` printing to stdout.
pub fn run(paths: &Paths, args: &DbArgs) -> Outcome {
    run_with(paths, args, &mut |line| println!("{line}"))
}

/// Runs `reforged-host db <subcommand>` sending output lines to `out`.
#[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
pub fn run_with(paths: &Paths, args: &DbArgs, out: &mut dyn FnMut(&str)) -> Outcome {
    let config = load_host_config(paths).ok().flatten();
    match &args.command {
        DbCommand::Migrate(m) => run_migrate_cmd(paths, config.as_ref(), m, out),
        DbCommand::Verify(v) => run_verify_cmd(paths, config.as_ref(), v, out),
        DbCommand::ExportMongo(e) => run_export_cmd(paths, config.as_ref(), e, out),
    }
}

#[cfg(not(any(feature = "isles-1-14-7", feature = "paradox")))]
pub fn run_with(_paths: &Paths, _args: &DbArgs, _out: &mut dyn FnMut(&str)) -> Outcome {
    Err(HostError::message(
        "this build does not include the Rustic Isles database tools (rebuild with --features isles-1-14-7)",
    ))
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

#[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
fn parse_areas(raw_areas: &[String]) -> Result<Vec<isles_db::MigrateArea>, HostError> {
    let mut areas = Vec::new();
    for item in raw_areas {
        for part in item.split(',') {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                let area = isles_db::MigrateArea::parse(trimmed).map_err(|e| HostError::message(e.to_string()))?;
                areas.push(area);
            }
        }
    }
    Ok(areas)
}

#[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
fn resolve_node(config: Option<&HostConfig>, explicit: Option<&str>) -> Result<module_api::NodeId, HostError> {
    let raw = explicit
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            config
                .and_then(|c| c.network.as_ref())
                .map(|n| n.node.as_str().trim())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("local");
    raw.parse::<module_api::NodeId>()
        .map_err(|e| HostError::message(format!("invalid --node {raw:?}: {e}")))
}

#[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
fn resolve_game_data_dir(paths: &Paths, explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(dir) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    paths
        .backend_game_data
        .is_dir()
        .then(|| paths.backend_game_data.clone())
}

#[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
fn run_migrate_cmd(
    paths: &Paths,
    config: Option<&HostConfig>,
    args: &DbMigrateArgs,
    out: &mut dyn FnMut(&str),
) -> Outcome {
    let from_uri = resolve_mongo_uri(paths, config, args.from.as_deref());
    let (to_target, allow_remote_postgres) =
        resolve_store_target(paths, config, args.to.as_deref(), args.allow_remote_postgres)?;
    let node = resolve_node(config, args.node.as_deref())?;
    let areas = parse_areas(&args.area)?;
    let game_data_dir = resolve_game_data_dir(paths, args.game_data_dir.as_deref());

    if !args.dry_run
        && !to_target.starts_with("postgres://")
        && !to_target.starts_with("postgresql://")
        && let Some(parent) = std::path::Path::new(&to_target).parent()
    {
        let _ = std::fs::create_dir_all(parent);
    }

    let opts = isles_db::MigrateOptions {
        from_uri: from_uri.clone(),
        to_target: to_target.clone(),
        node,
        dry_run: args.dry_run,
        areas,
        skip_invalid: args.skip_invalid,
        game_data_dir,
        allow_remote_postgres,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| HostError::message(format!("could not start runtime for db migrate: {e}")))?;

    let report = runtime.block_on(isles_db::run_migrate(&opts)).map_err(|e| {
        let msg = redact_credentials_in_text(&e.to_string(), &[&from_uri, &to_target]);
        HostError::message(format!("db migrate failed: {msg}"))
    })?;

    if args.json {
        if let Ok(s) = serde_json::to_string_pretty(&report) {
            out(&s);
        }
    } else {
        let mode = if report.dry_run { "dry-run" } else { "migrated" };
        let display_from = redact_uri_password(&from_uri);
        let display_to = redact_uri_password(&to_target);
        out(&format!(
            "reforged-host db migrate ({mode}, from {display_from} to {display_to}):"
        ));
        for (table, count) in &report.counts {
            if *count > 0 {
                out(&format!("  {table}: {count}"));
            }
        }
        out(&format!(
            "  rb11 transforms: approvals={}, inventory_arrays={}, starter_instances={}, loadout_slots={}",
            report.rb11_launcher_approvals_backfilled,
            report.rb11_inventory_arrays_transformed,
            report.rb11_starter_instances_repaired,
            report.rb11_loadout_slots_reconciled
        ));
        if report.skipped_invalid > 0 {
            out(&format!("  skipped_invalid: {}", report.skipped_invalid));
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
fn run_verify_cmd(
    paths: &Paths,
    config: Option<&HostConfig>,
    args: &DbVerifyArgs,
    out: &mut dyn FnMut(&str),
) -> Outcome {
    let from_uri = resolve_mongo_uri(paths, config, args.against.as_deref());
    let (to_target, allow_remote_postgres) =
        resolve_store_target(paths, config, args.to.as_deref(), args.allow_remote_postgres)?;
    let node = resolve_node(config, args.node.as_deref())?;
    let areas = parse_areas(&args.area)?;
    let game_data_dir = resolve_game_data_dir(paths, args.game_data_dir.as_deref());
    let all_areas = areas.is_empty();

    let opts = isles_db::MigrateOptions {
        from_uri: from_uri.clone(),
        to_target: to_target.clone(),
        node,
        dry_run: false,
        areas,
        skip_invalid: args.skip_invalid,
        game_data_dir,
        allow_remote_postgres,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| HostError::message(format!("could not start runtime for db verify: {e}")))?;

    let report = runtime.block_on(isles_db::run_verify(&opts)).map_err(|e| {
        let msg = redact_credentials_in_text(&e.to_string(), &[&from_uri, &to_target]);
        HostError::message(format!("db verify failed: {msg}"))
    })?;

    let display_against = redact_uri_password(&from_uri);
    let display_to = redact_uri_password(&to_target);
    if args.json {
        if let Ok(s) = serde_json::to_string_pretty(&report) {
            out(&s);
        }
    } else if report.ok {
        out(&format!(
            "reforged-host db verify: OK ({} collections verified against {display_against} for {display_to})",
            report.collections.len()
        ));
        for col in &report.collections {
            out(&format!(
                "  [OK] {} (area={}, count={}, sha256={})",
                col.name,
                col.area.as_str(),
                col.actual_count,
                &col.actual_sha256[..16]
            ));
        }
    }

    if !report.ok {
        let failed: Vec<String> = report
            .collections
            .iter()
            .filter(|c| !c.ok)
            .map(|c| c.name.clone())
            .collect();
        return Err(HostError::message(format!(
            "db verify failed: differences in [{}]",
            failed.join(", ")
        )));
    }

    if all_areas {
        let marker = paths.config.join(MONGO_MIGRATED_MARKER);
        if !marker.exists() {
            if let Some(parent) = marker.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&marker, format!("verified against {display_against}\n"));
        }
    }

    Ok(ExitCode::SUCCESS)
}

#[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
fn run_export_cmd(
    paths: &Paths,
    config: Option<&HostConfig>,
    args: &DbExportMongoArgs,
    out: &mut dyn FnMut(&str),
) -> Outcome {
    let (from_source, allow_remote_postgres) =
        resolve_store_target(paths, config, args.from.as_deref(), args.allow_remote_postgres)?;
    let to_uri = resolve_mongo_uri(paths, config, args.to.as_deref());
    let areas = parse_areas(&args.area)?;

    let opts = isles_db::ExportOptions {
        from_source: from_source.clone(),
        to_uri: to_uri.clone(),
        areas,
        dry_run: args.dry_run,
        allow_remote_postgres,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| HostError::message(format!("could not start runtime for db export-mongo: {e}")))?;

    let report = runtime.block_on(isles_db::run_export_mongo(&opts)).map_err(|e| {
        let msg = redact_credentials_in_text(&e.to_string(), &[&from_source, &to_uri]);
        HostError::message(format!("db export-mongo failed: {msg}"))
    })?;

    if args.json {
        if let Ok(s) = serde_json::to_string_pretty(&report) {
            out(&s);
        }
    } else {
        let mode = if report.dry_run { "dry-run" } else { "exported" };
        let display_from = redact_uri_password(&from_source);
        let display_to = redact_uri_password(&to_uri);
        out(&format!(
            "reforged-host db export-mongo ({mode}, from {display_from} to {display_to}):"
        ));
        for (collection, count) in &report.counts {
            if *count > 0 {
                out(&format!("  {collection}: {count}"));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::hostconfig::{
        DatabaseSettings, Edition, FileForm, HostConfig, MongoConfig, Registration, ServerMode, save_host_config,
    };

    fn sample_config() -> HostConfig {
        HostConfig {
            version: 1,
            mode: ServerMode::Private,
            edition: Edition::Reborn,
            name: "Test Server".into(),
            host: "100.64.0.1".into(),
            game_address: "100.64.0.1".into(),
            public_port: None,
            legacy_public_port: None,
            game_dir: None,
            legacy_game_dir: None,
            registration: Registration::Invitecode,
            share_url: None,
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
            network: None,
            file: FileForm::default(),
        }
    }

    #[test]
    fn defaults_source_and_target_from_host_toml_and_never_prints_passwords() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(
            paths.config.join("secrets.env"),
            "REFORGED_DB_PASSWORD=\"pgSuperSecret42\"\n",
        )
        .unwrap();

        let cfg = sample_config();
        let mongo_uri = resolve_mongo_uri(&paths, Some(&cfg), None);
        assert_eq!(
            mongo_uri,
            "mongodb://admin:mongoSecret99@127.0.0.1:27017/reforged?replicaSet=rs0"
        );
        let (store_target, allow_remote) = resolve_store_target(&paths, Some(&cfg), None, false).unwrap();
        assert_eq!(store_target, "postgres://dainty:pgSuperSecret42@127.0.0.1:5432/dainty");
        assert!(!allow_remote);

        let redacted_mongo = redact_uri_password(&mongo_uri);
        let redacted_pg = redact_uri_password(&store_target);
        assert_eq!(
            redacted_mongo,
            "mongodb://admin:***@127.0.0.1:27017/reforged?replicaSet=rs0"
        );
        assert_eq!(redacted_pg, "postgres://dainty:***@127.0.0.1:5432/dainty");
        assert!(!redacted_mongo.contains("mongoSecret99"));
        assert!(!redacted_pg.contains("pgSuperSecret42"));

        let err_text = format!("failed connecting to {store_target} or {mongo_uri} (pw=pgSuperSecret42)");
        let cleaned = redact_credentials_in_text(&err_text, &[&store_target, &mongo_uri]);
        assert!(!cleaned.contains("pgSuperSecret42"));
        assert!(!cleaned.contains("mongoSecret99"));
        assert!(cleaned.contains("postgres://dainty:***@127.0.0.1:5432/dainty"));
        assert!(cleaned.contains("mongodb://admin:***@127.0.0.1:27017/reforged?replicaSet=rs0"));
    }

    #[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
    #[test]
    fn db_migrate_verify_and_export_mongo_work_with_host_toml_defaults() {
        use isles_db::mongo::MockMongoServer;
        use serde_json::{Map, Value, json};
        use std::collections::BTreeMap;

        let mut cols: BTreeMap<String, Vec<Map<String, Value>>> = BTreeMap::new();
        if let Value::Object(acc) = json!({
            "_id": "user-m3-01",
            "userId": "user-m3-01",
            "name": "SlayerM3",
            "displayName": "SlayerM3",
            "displayNameNormalized": "slayerm3",
            "status": "active",
            "approvalStatus": "approved",
            "roles": ["player"],
            "usernameSet": 1,
            "createdAt": "2026-03-01T10:00:00.000Z"
        }) {
            cols.insert("accounts".into(), vec![acc]);
        }
        let mock = MockMongoServer::start(cols, 50).unwrap();
        let mongo_uri = mock.uri("reforged");

        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        let mut cfg = sample_config();
        cfg.mongo.uri = mongo_uri.clone();
        cfg.database = Some(DatabaseSettings {
            engine: Some("sqlite".into()),
            ..DatabaseSettings::default()
        });
        save_host_config(&paths, &cfg).unwrap();

        let mut lines = Vec::new();
        run_with(
            &paths,
            &DbArgs {
                command: DbCommand::Migrate(DbMigrateArgs::default()),
            },
            &mut |l| lines.push(l.to_owned()),
        )
        .unwrap();
        assert!(lines.iter().any(|l| l.contains("accounts: 1")));

        lines.clear();
        run_with(
            &paths,
            &DbArgs {
                command: DbCommand::Verify(DbVerifyArgs::default()),
            },
            &mut |l| lines.push(l.to_owned()),
        )
        .unwrap();
        assert!(lines.iter().any(|l| l.contains("reforged-host db verify: OK")));
        assert!(paths.config.join(MONGO_MIGRATED_MARKER).exists());

        lines.clear();
        run_with(
            &paths,
            &DbArgs {
                command: DbCommand::ExportMongo(DbExportMongoArgs {
                    dry_run: true,
                    ..DbExportMongoArgs::default()
                }),
            },
            &mut |l| lines.push(l.to_owned()),
        )
        .unwrap();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("reforged-host db export-mongo (dry-run"))
        );
        assert!(lines.iter().any(|l| l.contains("accounts: 1")));
    }
}
