//! The host kit (`reforged-host`): thin AGPL shell wiring `hostkit` (MIT) to the compiled-in
//! services (`gateway`, `allowlist`, `content`, `admin`, and `isles-1-14-7` from dainty-lass's
//! `rustic-isles-1-14-7`), the `rustic-isles-db` and `rustic-isles-replay` tools, the
//! `rustic-isles-gamedata` / `rustic-isles-director` generators, and the `legacy` 1.4.4 stack.

pub use hostkit::{
    HostError, Outcome, captures, cli, envfile, epicinstall, error, gamepack, guarded, hooks, hostconfig, hosttoml,
    invite, linux, mongo, paths, preset, progression, publish, repl, selfupdate, services, tls, tui, wine,
    with_admin_env,
};

pub mod compose;
#[cfg(feature = "db-tools")]
pub mod version_data;

pub mod run {
    pub use hostkit::run::*;
}

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use crate::compose as modules;
use clap::Parser as _;

/// Which services this binary runs on its own (`reforged-host <service>`).
pub const BUILT_IN_SERVICES: &[&str] = &[
    #[cfg(feature = "gateway")]
    "gateway",
    #[cfg(feature = "allowlist")]
    "allowlist",
    #[cfg(feature = "content")]
    "content",
    #[cfg(feature = "admin")]
    "admin",
    #[cfg(feature = "isles-1-14-7")]
    "backend",
    #[cfg(feature = "isles-1-14-7")]
    "director",
    "remote-agent",
];

struct ServicesHostHooks;

static SERVICES_HOST_HOOKS: ServicesHostHooks = ServicesHostHooks;

#[inline]
pub fn ensure_hooks() {
    hostkit::hooks::set_hooks(&SERVICES_HOST_HOOKS);
}

#[cfg(feature = "db-tools")]
fn db_cfg_to_target(db_cfg: &store::DatabaseConfig) -> (String, bool) {
    match db_cfg {
        store::DatabaseConfig::Sqlite { file } => (file.to_string_lossy().into_owned(), false),
        store::DatabaseConfig::Postgres(pg) => {
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

#[cfg(feature = "isles-1-14-7")]
fn execute_rust_generator(p: &paths::Paths, generator: hostkit::gamedata::GeneratorKind) -> Result<String, String> {
    use hostkit::gamedata::GeneratorKind;
    use rustic_isles_gamedata::generators::{BackendStep, GeneratorPaths};
    let step = match generator {
        GeneratorKind::HuntTables => {
            return rustic_isles_director::hunt_tables::import_hunt_tables(
                &rustic_isles_director::hunt_tables::ImportHuntTablesOptions {
                    export_dir: p.exports.join("hunts_1_14_7"),
                    vendor_dir: p.director_game_data.clone(),
                    repo_root: Some(p.repo.clone()),
                    apply: true,
                    update_existing: false,
                    verbose: false,
                    backup_stamp: None,
                },
            )
            .map(|report| report.output);
        }
        GeneratorKind::ProgressionConfig => BackendStep::ProgressionConfig,
        GeneratorKind::SlayersPath => BackendStep::SlayersPath,
        GeneratorKind::SlayersPathDefinitions => BackendStep::SlayersPathDefinitions,
        GeneratorKind::RewardCache => BackendStep::RewardCache,
        GeneratorKind::LadyLuckGenerate => BackendStep::LadyLuckGenerate,
        GeneratorKind::LadyLuckEnrich => BackendStep::LadyLuckEnrich,
        GeneratorKind::PlatinumAndStoragePolicy => BackendStep::PlatinumAndStoragePolicy,
    };
    // Beside the files it generates, for the module as a library or child (spec-rustic-isles-1-14-7 decision 7)
    version_data::install(&p.backend_game_data).map_err(|e| {
        format!(
            "could not install {} in {}: {e}",
            rustic_isles_1_14_7::DATA_FILE,
            p.backend_game_data.display()
        )
    })?;
    let inputs = version_data::load()?
        .generator_inputs()
        .ok_or_else(|| "the compiled-in version_data.json has no rewardCachePlan or storageTokenSeeds".to_owned())?;
    let paths = GeneratorPaths {
        raw_game_data: p.raw_game_data.clone(),
        backend_game_data: p.backend_game_data.clone(),
        exports: p.exports.clone(),
        store_captures: p.store_captures.clone(),
    };
    rustic_isles_gamedata::generators::run(step, &paths, &inputs)
}

#[cfg(not(feature = "isles-1-14-7"))]
fn execute_rust_generator(_p: &paths::Paths, generator: hostkit::gamedata::GeneratorKind) -> Result<String, String> {
    Err(format!(
        "{} is unavailable: this binary was built without the isles-1-14-7 feature",
        generator.label()
    ))
}

#[cfg(feature = "db-tools")]
mod db_impl {
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::process::ExitCode;

    use crate::Outcome;
    use crate::cli::{DbArgs, DbCommand, DbExportMongoArgs, DbMigrateArgs, DbReplayArgs, DbVerifyArgs};
    use crate::error::HostError;
    use crate::hostconfig::{HostConfig, load_host_config, read_env_file};
    use crate::paths::Paths;
    use hostkit::commands::db::{redact_credentials_in_text, resolve_mongo_uri, resolve_store_target};
    use hostkit::commands::update::{MONGO_MIGRATED_MARKER, redact_uri_password};

    fn parse_areas(raw_areas: &[String]) -> Result<BTreeSet<rustic_isles_db::Area>, HostError> {
        let mut areas = BTreeSet::new();
        for item in raw_areas {
            for part in item.split(',') {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    let area = rustic_isles_db::Area::parse(trimmed).map_err(|e| HostError::message(e.to_string()))?;
                    areas.insert(area);
                }
            }
        }
        Ok(areas)
    }

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

    fn resolve_game_data_dir(paths: &Paths, explicit: Option<&str>) -> Option<PathBuf> {
        if let Some(dir) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
            return Some(PathBuf::from(dir));
        }
        paths
            .backend_game_data
            .is_dir()
            .then(|| paths.backend_game_data.clone())
    }

    pub fn run_with(paths: &Paths, args: &DbArgs, out: &mut dyn FnMut(&str)) -> Outcome {
        let config = load_host_config(paths).ok().flatten();
        match &args.command {
            DbCommand::Migrate(m) => run_migrate_cmd(paths, config.as_ref(), m, out),
            DbCommand::Verify(v) => run_verify_cmd(paths, config.as_ref(), v, out),
            DbCommand::ExportMongo(e) => run_export_cmd(paths, config.as_ref(), e, out),
            DbCommand::Replay(r) => run_replay_cmd(paths, r, out),
        }
    }

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

        let opts = rustic_isles_db::MigrateOptions {
            from_uri: from_uri.clone(),
            to_target: to_target.clone(),
            node,
            dry_run: args.dry_run,
            areas,
            skip_invalid: args.skip_invalid,
            game_data_dir,
            allow_remote_postgres,
            starter: crate::version_data::legacy_starter().map_err(HostError::message)?,
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| HostError::message(format!("could not start runtime for db migrate: {e}")))?;

        let report = runtime.block_on(rustic_isles_db::run_migrate(&opts)).map_err(|e| {
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

        let opts = rustic_isles_db::MigrateOptions {
            from_uri: from_uri.clone(),
            to_target: to_target.clone(),
            node,
            dry_run: false,
            areas,
            skip_invalid: args.skip_invalid,
            game_data_dir,
            allow_remote_postgres,
            starter: crate::version_data::legacy_starter().map_err(HostError::message)?,
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| HostError::message(format!("could not start runtime for db verify: {e}")))?;

        let report = runtime.block_on(rustic_isles_db::run_verify(&opts)).map_err(|e| {
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

        let opts = rustic_isles_db::ExportOptions {
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

        let report = runtime
            .block_on(rustic_isles_db::run_export_mongo(&opts))
            .map_err(|e| {
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

    fn run_replay_cmd(paths: &Paths, args: &DbReplayArgs, out: &mut dyn FnMut(&str)) -> Outcome {
        let secrets_env = read_env_file(&paths.config.join("secrets.env")).unwrap_or_default();
        let default_signing_key = secrets_env
            .get("AUTH_SIGNING_PRIVKEY_B64")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let default_jwt_secret = secrets_env
            .get("JWT_SECRET")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let default_gs_key = secrets_env
            .get("GAMESERVER_API_KEY")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let default_gateway_secret = secrets_env
            .get("GATEWAY_SECRET")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        let left_signing_key = args
            .left_signing_key
            .clone()
            .or_else(|| args.signing_key.clone())
            .or_else(|| default_signing_key.clone());
        let right_signing_key = args
            .right_signing_key
            .clone()
            .or_else(|| args.signing_key.clone())
            .or(default_signing_key);

        let left_jwt_secret = args
            .left_jwt_secret
            .clone()
            .or_else(|| args.jwt_secret.clone())
            .or_else(|| default_jwt_secret.clone());
        let right_jwt_secret = args
            .right_jwt_secret
            .clone()
            .or_else(|| args.jwt_secret.clone())
            .or(default_jwt_secret);

        let left_gs_key = args
            .left_gs_key
            .clone()
            .or_else(|| args.gs_key.clone())
            .or_else(|| default_gs_key.clone());
        let right_gs_key = args
            .right_gs_key
            .clone()
            .or_else(|| args.gs_key.clone())
            .or(default_gs_key);

        let gateway_secret = args.gateway_secret.clone().or(default_gateway_secret);
        let pin_clock = !args.no_pin_clock || args.pin_clock;

        let opts = rustic_isles_replay::ReplayOptions {
            corpus: PathBuf::from(&args.corpus),
            left: args.left.clone(),
            right: args.right.clone(),
            left_director: args.left_director.clone(),
            right_director: args.right_director.clone(),
            left_chat: args.left_chat.clone(),
            right_chat: args.right_chat.clone(),
            left_jwt_secret,
            right_jwt_secret,
            left_signing_key,
            right_signing_key,
            left_gs_key,
            right_gs_key,
            gateway_secret,
            key_map_path: args.key_map.as_ref().map(PathBuf::from),
            pin_clock,
            filter: args.filter.clone(),
            out_path: args.out.as_ref().map(PathBuf::from),
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| HostError::message(format!("could not start runtime for db replay: {e}")))?;

        let report = runtime
            .block_on(rustic_isles_replay::run_replay_cli(&opts))
            .map_err(|e| HostError::message(format!("db replay failed: {e}")))?;

        if args.json {
            if let Ok(s) = serde_json::to_string_pretty(&report) {
                out(&s);
            }
        } else {
            let status = if report.passed() { "PASS" } else { "DIFF" };
            out(&format!(
                "reforged-host db replay: {status} (records={}, http={}, chat_conns={}, skipped={}, exact_matched={}, inexact_matched={}, inexact_diffed={}, unexplained_diffed={})",
                report.total_records,
                report.http_replayed,
                report.chat_connections_replayed,
                report.skipped_records,
                report.exact_matched,
                report.inexact_matched,
                report.inexact_diffed,
                report.unexplained_diffed,
            ));
            out(&format!(
                "  diffs by kind: status={}, header={}, value={}, text={}, chat_stanza={}",
                report.status_diffs,
                report.header_diffs,
                report.value_diffs,
                report.text_diffs,
                report.chat_stanza_diffs,
            ));
            for entry in &report.entries {
                if !entry.diffs.is_empty() {
                    let tag = if entry.inexact { "INEXACT" } else { "DIFF" };
                    out(&format!(
                        "  [{tag}] #{} {} {} (conn={}, seq={}, route={}): {} diff(s)",
                        entry.index,
                        entry.method,
                        entry.target,
                        entry.conn,
                        entry.seq,
                        entry.route,
                        entry.diffs.len()
                    ));
                    for d in &entry.diffs {
                        out(&format!("    - {d:?}"));
                    }
                }
            }
        }

        if report.passed() {
            Ok(ExitCode::SUCCESS)
        } else {
            Ok(ExitCode::from(2))
        }
    }
}

impl hooks::HostHooks for ServicesHostHooks {
    fn built_in_services(&self) -> &'static [&'static str] {
        BUILT_IN_SERVICES
    }

    // setup's and start's install step: version_data.json for the 1.14.7 module loaded as a library (RD27); the
    // compiled-in one is handed the values instead
    fn install_director_runner(&self, paths: &paths::Paths) -> Result<(), HostError> {
        #[cfg(feature = "isles-1-14-7")]
        if paths.backend_game_data.is_dir() {
            version_data::install(&paths.backend_game_data).map_err(|e| {
                HostError::message(format!("could not install {}: {e}", rustic_isles_1_14_7::DATA_FILE))
            })?;
        }
        #[cfg(not(feature = "isles-1-14-7"))]
        let _ = paths;
        Ok(())
    }

    fn run_generator(
        &self,
        paths: &paths::Paths,
        step: &hostkit::gamedata::Step<'_>,
    ) -> Result<run::RunResult, HostError> {
        match execute_rust_generator(paths, step.generator) {
            Ok(output) => {
                let last_line = output
                    .lines()
                    .map(str::trim)
                    .rfind(|l| !l.is_empty())
                    .map(|l| l.chars().take(300).collect());
                Ok(run::RunResult {
                    code: 0,
                    last_line,
                    output,
                })
            }
            Err(error) => {
                let last_line = error
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .map(|l| l.chars().take(300).collect());
                Ok(run::RunResult {
                    code: 1,
                    last_line,
                    output: error,
                })
            }
        }
    }

    fn db_migrate(
        &self,
        source_uri: &str,
        db_cfg: &store::DatabaseConfig,
        dry_run: bool,
    ) -> Result<(Vec<(String, usize)>, Vec<(String, String)>), HostError> {
        #[cfg(feature = "db-tools")]
        {
            let (to_target, allow_remote_postgres) = db_cfg_to_target(db_cfg);
            let opts = rustic_isles_db::MigrateOptions {
                from_uri: source_uri.to_owned(),
                to_target,
                node: "local"
                    .parse::<module_api::NodeId>()
                    .map_err(|e| HostError::message(e.to_string()))?,
                dry_run,
                areas: Default::default(),
                skip_invalid: false,
                game_data_dir: None,
                allow_remote_postgres,
                starter: version_data::legacy_starter().map_err(HostError::message)?,
            };
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| HostError::message(format!("could not start runtime: {e}")))?;
            let report = runtime
                .block_on(rustic_isles_db::run_migrate(&opts))
                .map_err(|e| HostError::message(e.to_string()))?;
            let counts: Vec<(String, usize)> = report.counts.into_iter().collect();
            Ok((counts, Vec::new()))
        }
        #[cfg(not(feature = "db-tools"))]
        {
            let _ = (source_uri, db_cfg, dry_run);
            Err(HostError::message("database migration is not compiled in"))
        }
    }

    fn db_verify(&self, source_uri: &str, db_cfg: &store::DatabaseConfig) -> Result<Vec<(String, String)>, HostError> {
        #[cfg(feature = "db-tools")]
        {
            let (to_target, allow_remote_postgres) = db_cfg_to_target(db_cfg);
            let opts = rustic_isles_db::MigrateOptions {
                from_uri: source_uri.to_owned(),
                to_target,
                node: "local"
                    .parse::<module_api::NodeId>()
                    .map_err(|e| HostError::message(e.to_string()))?,
                dry_run: false,
                areas: Default::default(),
                skip_invalid: false,
                game_data_dir: None,
                allow_remote_postgres,
                starter: version_data::legacy_starter().map_err(HostError::message)?,
            };
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| HostError::message(format!("could not start runtime: {e}")))?;
            match runtime.block_on(rustic_isles_db::run_verify(&opts)) {
                Ok(report) => {
                    let mismatches = report
                        .collections
                        .into_iter()
                        .filter(|c| !c.ok)
                        .map(|c| {
                            (
                                c.name,
                                format!(
                                    "expected count={} sha256={}, actual count={} sha256={}",
                                    c.expected_count, c.expected_sha256, c.actual_count, c.actual_sha256
                                ),
                            )
                        })
                        .collect();
                    Ok(mismatches)
                }
                Err(rustic_isles_db::DbError::VerificationFailed(n)) => {
                    Ok(vec![(format!("{n} collection(s)"), "verification failed".to_owned())])
                }
                Err(e) => Err(HostError::message(e.to_string())),
            }
        }
        #[cfg(not(feature = "db-tools"))]
        {
            let _ = (source_uri, db_cfg);
            Err(HostError::message("database verification is not compiled in"))
        }
    }

    fn db_export_mongo(&self, uri: &str, out: &Path) -> Result<Vec<(String, usize)>, HostError> {
        #[cfg(feature = "db-tools")]
        {
            let opts = rustic_isles_db::ExportOptions {
                from_source: out.to_string_lossy().into_owned(),
                to_uri: uri.to_owned(),
                areas: Default::default(),
                dry_run: false,
                allow_remote_postgres: false,
            };
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| HostError::message(format!("could not start runtime: {e}")))?;
            let report = runtime
                .block_on(rustic_isles_db::run_export_mongo(&opts))
                .map_err(|e| HostError::message(e.to_string()))?;
            Ok(report.counts.into_iter().collect())
        }
        #[cfg(not(feature = "db-tools"))]
        {
            let _ = (uri, out);
            Err(HostError::message("MongoDB export is not compiled in"))
        }
    }

    fn run_db_command(&self, paths: &paths::Paths, args: &cli::DbArgs, out: &mut dyn FnMut(&str)) -> Option<Outcome> {
        #[cfg(feature = "db-tools")]
        {
            Some(db_impl::run_with(paths, args, out))
        }
        #[cfg(not(feature = "db-tools"))]
        {
            let _ = (paths, args, out);
            Some(Err(HostError::message(
                "this build does not include the Rustic Isles database tools (rebuild with --features isles-1-14-7)",
            )))
        }
    }

    fn compose_in_process_allowlist(
        &self,
        controller: &mut controller::ServerController,
        paths: &paths::Paths,
        registry: &controller::ModuleRegistry,
    ) -> Result<Option<hooks::AnySlot>, HostError> {
        // It runs inside the gateway service (compose_in_process_network_and_gateway), as in the gateway library
        #[cfg(feature = "allowlist")]
        {
            let _ = (controller, registry);
            let mode =
                modules::allowlist(&hostkit::serve::module_env(paths, "allowlist")?).map_err(HostError::message)?;
            Ok(Some(Box::new(mode)))
        }
        #[cfg(not(feature = "allowlist"))]
        {
            let _ = (controller, paths, registry);
            Ok(None)
        }
    }

    fn compose_in_process_content(
        &self,
        controller: &mut controller::ServerController,
        paths: &paths::Paths,
        registry: &controller::ModuleRegistry,
    ) -> Result<(), HostError> {
        #[cfg(feature = "content")]
        {
            controller.add(
                modules::content(&hostkit::serve::module_env(paths, "content")?, Some(registry.clone()))
                    .map_err(HostError::message)?,
            );
        }
        #[cfg(not(feature = "content"))]
        let _ = (controller, paths, registry);
        Ok(())
    }

    fn compose_in_process_network_and_gateway(
        &self,
        controller: &mut controller::ServerController,
        paths: &paths::Paths,
        config: &hostconfig::HostConfig,
        registry: &controller::ModuleRegistry,
        slot: Option<hooks::AnySlot>,
        in_process_gateway: bool,
        public: bool,
        guarded: bool,
    ) -> Result<(), HostError> {
        // A service host.toml loads as a library is hostkit's to compose (compose_service_library)
        let gateway_library = hostkit::serve::library_configured(config, "gateway");
        let chat_library = hostkit::serve::library_configured(config, "chat");

        #[cfg(feature = "gateway")]
        let gateway = if in_process_gateway && !gateway_library {
            use controller::service_library::gateway::AllowlistMode;
            let in_process = slot.and_then(|s| s.downcast::<AllowlistMode>().ok().map(|b| *b));
            let allowlist = match in_process {
                Some(mode) => mode,
                // Nothing here guards UDP ranges: they are refused, or held to the external helper's (DF4d)
                None if public && !guarded && cfg!(feature = "allowlist") => {
                    if cfg!(feature = "paradox") {
                        AllowlistMode::Absent
                    } else {
                        let (first, last) = hostconfig::game_ports(config);
                        AllowlistMode::External {
                            base: module_api::PortRange { first, last },
                        }
                    }
                }
                None => AllowlistMode::Off,
            };
            Some(
                modules::gateway(
                    &hostkit::serve::module_env(paths, "gateway")?,
                    allowlist,
                    Some(registry.clone()),
                )
                .map_err(HostError::message)?,
            )
        } else {
            None
        };
        #[cfg(not(feature = "gateway"))]
        let _ = (slot, public, guarded, gateway_library);

        #[cfg(feature = "gateway")]
        if let Some(network) = &config.network
            && !network.nodes.is_empty()
        {
            let cert = paths.tls.join(hostconfig::NODE_CERT_FILE);
            let key = paths.tls.join(hostconfig::NODE_KEY_FILE);
            let ca = paths.tls.join(hostconfig::CLUSTER_CA_FILE);
            let credentials = network_plugin::cluster::NodeCredentials::load(&cert, &key, &ca)
                .map_err(|e| HostError::message(format!("network credentials in {}: {e}", paths.tls.display())))?;
            let said = credentials.identity();
            if said.node != network.node || said.region != network.region {
                return Err(HostError::message(format!(
                    "the node certificate in {} is for {} in {}, but host.toml says {} in {}",
                    paths.tls.display(),
                    said.node,
                    said.region,
                    network.node,
                    network.region
                )));
            }
            std::fs::create_dir_all(&paths.tls).map_err(|e| HostError::io(&paths.tls, e))?;
            let key_path = paths.tls.join(hostconfig::SESSION_KEY_FILE);
            let signing_key = network_plugin::home::SigningKey::load_or_create(&key_path)
                .map_err(|e| HostError::message(format!("network session key {}: {e}", key_path.display())))?;
            let database = hostconfig::node_database_config(config, paths)?;
            let game = protocol::invite::reforged_version();
            let peer_log_dir = Some(paths.workspace.join("peer-logs"));
            let ship_to_peer = config
                .history
                .as_ref()
                .and_then(|h| h.ship_to_peer.clone())
                .map(|peer| (peer, paths.config.join("history-peer-state.json")));
            controller.add(
                modules::network(modules::NetworkOptions {
                    node: said.clone(),
                    leader: network.effective_leader(),
                    peers: network.nodes.clone(),
                    credentials,
                    signing_key,
                    database,
                    game,
                    registry: registry.clone(),
                    allowlist: gateway.as_ref().map(|(_, handle)| handle.clone()),
                    replication_interval: Duration::from_secs(5),
                    peer_log_dir,
                    ship_to_peer,
                    wrong_node_policy: network.wrong_node_policy,
                })
                .map_err(HostError::message)?,
            );
        }

        #[cfg(feature = "gateway")]
        if let Some((module, _)) = gateway {
            controller.add(module);
        }
        #[cfg(all(feature = "chat", not(feature = "paradox")))]
        if in_process_gateway && !chat_library {
            let node = config
                .network
                .as_ref()
                .map(|n| module_api::NodeIdentity {
                    node: n.node.clone(),
                    region: n.region.clone(),
                })
                .unwrap_or_else(module_api::NodeIdentity::standalone);
            let database = hostconfig::node_database_config(config, paths)?;
            let network =
                config
                    .network
                    .as_ref()
                    .filter(|n| !n.nodes.is_empty())
                    .map(|n| modules::ChatNetworkOptions {
                        leader: n.effective_leader(),
                        peers: n.nodes.iter().map(|p| (p.id.clone(), p.region.clone())).collect(),
                    });
            controller.add(
                modules::chat(modules::ChatOptions {
                    env: hostkit::serve::module_env(paths, "gateway")?,
                    node,
                    database,
                    registry: registry.clone(),
                    network,
                })
                .map_err(HostError::message)?,
            );
        }
        #[cfg(not(feature = "gateway"))]
        let _ = (controller, paths, config, in_process_gateway);
        #[cfg(not(all(feature = "chat", not(feature = "paradox"))))]
        let _ = chat_library;

        #[cfg(not(feature = "gateway"))]
        let _ = registry;
        Ok(())
    }

    fn compose_service_library(
        &self,
        controller: &mut controller::ServerController,
        paths: &paths::Paths,
        config: &hostconfig::HostConfig,
        registry: &controller::ModuleRegistry,
        library: controller::ServiceLibrary,
    ) -> Result<(), HostError> {
        // hostkit gives the gateway and chat libraries their own core side, as the MIT core does (RD27)
        hostkit::serve::add_service_library(controller, paths, config, registry, library)
    }

    fn ensure_gateway_cert(&self, paths: &paths::Paths, config: &hostconfig::HostConfig) -> Result<(), HostError> {
        #[cfg(feature = "gateway")]
        if config.mode == hostconfig::ServerMode::Public && !paths.cert_file.exists() {
            gateway::cert::load_or_generate(&paths.cert_file, &paths.key_file, std::slice::from_ref(&config.host))
                .map_err(HostError::message)?;
            hostkit::commands::start::say(&format!(
                "- gateway certificate made in {} (fingerprint {})",
                paths.tls.display(),
                crate::invite::certificate_fingerprint(&paths.cert_file)?
            ));
        }
        #[cfg(not(feature = "gateway"))]
        let _ = (paths, config);
        Ok(())
    }

    fn game_modules(&self) -> Vec<controller::ModuleFactory> {
        crate::compose::game_modules()
    }

    fn run_service(&self, name: &str, args: &[String]) -> Option<ExitCode> {
        let argv = std::iter::once(name.to_owned()).chain(args.iter().cloned());
        match name {
            #[cfg(feature = "gateway")]
            "gateway" => {
                drop(argv);
                Some(gateway_service_main(args))
            }
            #[cfg(feature = "allowlist")]
            "allowlist" => Some(gateway::allowlist::cli::main_from(argv.collect())),
            #[cfg(feature = "content")]
            "content" => Some(content::cli::main_from(argv)),
            #[cfg(feature = "admin")]
            "admin" => Some(admin::cli::main_from(argv)),
            #[cfg(feature = "isles-1-14-7")]
            "backend" => {
                drop(argv);
                Some(isles_service_main("backend", args))
            }
            #[cfg(feature = "isles-1-14-7")]
            "director" => {
                drop(argv);
                Some(isles_service_main("director", args))
            }
            "remote-agent" => {
                drop(argv);
                Some(remote_agent_service_main(args))
            }
            _ => {
                drop(argv);
                None
            }
        }
    }
}

#[cfg(feature = "isles-1-14-7")]
struct StandaloneLink {
    home: module_api::network::StandaloneHome,
}

#[cfg(feature = "isles-1-14-7")]
impl module_api::HostLink for StandaloneLink {
    fn call<'a>(
        &'a self,
        module: &'a module_api::ModuleId,
        call: module_api::HostCall,
    ) -> module_api::BoxFuture<'a, Result<module_api::HostAnswer, module_api::ModuleError>> {
        Box::pin(async move {
            match call {
                module_api::HostCall::Emit(_) => Ok(module_api::HostAnswer::Done),
                module_api::HostCall::Log(record) => {
                    println!("{module} | {:?}: {}", record.level, record.message);
                    Ok(module_api::HostAnswer::Done)
                }
                module_api::HostCall::Home(call) => Ok(module_api::HostAnswer::Home(
                    module_api::network::dispatch_home(&self.home, call).await,
                )),
            }
        })
    }
}

fn parse_service_env_file(
    name: &str,
    args: &[String],
    paths: Option<&paths::Paths>,
) -> Result<Option<PathBuf>, String> {
    let mut env_file: Option<PathBuf> = None;
    let mut idx = 0;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if arg == "serve" && idx == 0 {
            idx += 1;
            continue;
        }
        if arg == "--env-file" {
            idx += 1;
            let Some(val) = args.get(idx) else {
                return Err("--env-file requires a path".into());
            };
            env_file = Some(PathBuf::from(val));
            idx += 1;
        } else if let Some(val) = arg.strip_prefix("--env-file=") {
            env_file = Some(PathBuf::from(val));
            idx += 1;
        } else {
            return Err(format!("unexpected argument: {arg}"));
        }
    }
    if env_file.is_none() {
        if let Some(p) = paths {
            let candidate = p.config.join(format!("{name}.env"));
            if candidate.is_file() {
                env_file = Some(candidate);
            }
        }
        if env_file.is_none() {
            let dot_env = PathBuf::from(".env");
            if dot_env.is_file() {
                env_file = Some(dot_env);
            }
        }
    }
    Ok(env_file)
}

#[cfg(feature = "gateway")]
fn gateway_service_main(args: &[String]) -> ExitCode {
    let argv: Vec<String> = std::iter::once("gateway".to_owned())
        .chain(args.iter().cloned())
        .collect();
    if args.first().map(String::as_str) == Some("make-cert")
        || args
            .iter()
            .any(|a| matches!(a.as_str(), "--help" | "-h" | "--version" | "-V"))
    {
        return gateway::cli::main_from(argv);
    }
    let repo_paths = paths::locate_repo_root().ok().map(paths::Paths::new);
    let Ok(env_file) = parse_service_env_file("gateway", args, repo_paths.as_ref()) else {
        return gateway::cli::main_from(argv);
    };
    let paths = env_file
        .as_deref()
        .and_then(|file| {
            let config_dir = file.parent()?;
            if config_dir.file_name().is_some_and(|n| n == "config") {
                let ws_dir = config_dir.parent()?;
                if ws_dir.file_name().is_some_and(|n| n == ".workspace") {
                    return ws_dir.parent().map(paths::Paths::new);
                }
            }
            None
        })
        .or(repo_paths);
    let Some(p) = paths else {
        return gateway::cli::main_from(argv);
    };
    if std::env::var("REFORGED_NETWORK_FULL_NODE").as_deref() == Ok("1")
        && let Ok(Some(hc)) = hostconfig::load_host_config(&p)
    {
        return run_network_full_node(&p, &hc);
    }
    let state_file = hostkit::commands::remote::controller_state_path(&p);
    let ca_file = hostkit::commands::remote::controller_ca_path(&p);
    if !state_file.is_file() || !ca_file.is_file() {
        return gateway::cli::main_from(argv);
    }
    let Ok(state) = remote::RemoteControllerState::load(&state_file) else {
        return gateway::cli::main_from(argv);
    };
    let Ok(ca) = network_plugin::cluster::ClusterCa::read(&ca_file) else {
        return gateway::cli::main_from(argv);
    };
    let env = match service_kit::env::Env::load(env_file.as_deref()) {
        Ok(e) => e,
        Err(error) => {
            service_kit::linelog::error(&format!("Configuration: {error}"), serde_json::json!({}));
            return ExitCode::FAILURE;
        }
    };
    let config = match gateway::config::Config::from_env(&env) {
        Ok(c) => c,
        Err(error) => {
            service_kit::linelog::error(&format!("Configuration: {error}"), serde_json::json!({}));
            return ExitCode::FAILURE;
        }
    };
    let capture = match gateway::server::load_capture(&env) {
        Ok(cap) => cap,
        Err(error) => {
            service_kit::linelog::error(&error, serde_json::json!({}));
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(error) => {
            service_kit::linelog::error(&error.to_string(), serde_json::json!({}));
            return ExitCode::FAILURE;
        }
    };
    let slot: gateway::server::NodeLinkSlot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let slot_bg = slot.clone();
    let state_arc = std::sync::Arc::new(std::sync::Mutex::new(state));
    let ca_arc = std::sync::Arc::new(ca);
    let res = runtime.block_on(async move {
        tokio::spawn(async move {
            let links = loop {
                if let Some(l) = slot_bg
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
                {
                    break l;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
            let enrolled: Vec<module_api::network::NodeId> = {
                let g = state_arc.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                g.installs
                    .values()
                    .filter(|r| !r.revoked)
                    .map(|r| r.node.clone())
                    .collect()
            };
            for node in enrolled {
                links.endpoint().add_peer(node);
            }
            let enroll_state = state_arc.clone();
            let enroll_file = state_file.clone();
            let enroll_ca = ca_arc.clone();
            let enroll_links = links.clone();
            links.set_enroll_handler(Some(std::sync::Arc::new(move |body: Vec<u8>| {
                let enroll_state = enroll_state.clone();
                let enroll_file = enroll_file.clone();
                let enroll_ca = enroll_ca.clone();
                let enroll_links = enroll_links.clone();
                Box::pin(async move {
                    let req = match remote::decode_enroll_request(&body) {
                        Ok(r) => r,
                        Err(e) => {
                            let err = serde_json::json!({ "error": e.to_string() });
                            return (400, err.to_string().into_bytes());
                        }
                    };
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    let answer_res = {
                        let mut g = enroll_state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        if enroll_file.is_file()
                            && let Ok(fresh) = remote::RemoteControllerState::load(&enroll_file)
                        {
                            *g = fresh;
                        }
                        let res = g.redeem_enrollment(&enroll_ca, &req, now_ms);
                        if res.is_ok() {
                            let _ = g.save(&enroll_file);
                        }
                        res
                    };
                    match answer_res {
                        Ok(ans) => {
                            enroll_links.endpoint().add_peer(req.node);
                            match remote::encode_enroll_answer(&ans) {
                                Ok(bytes) => (200, bytes),
                                Err(e) => {
                                    let err = serde_json::json!({ "error": e.to_string() });
                                    (500, err.to_string().into_bytes())
                                }
                            }
                        }
                        Err(e) => {
                            let status = match e {
                                remote::RemoteError::InvalidEnrollmentToken
                                | remote::RemoteError::EnrollmentTokenUsed
                                | remote::RemoteError::EnrollmentTokenExpired => 403,
                                _ => 400,
                            };
                            let err = serde_json::json!({ "error": e.to_string() });
                            (status, err.to_string().into_bytes())
                        }
                    }
                })
            })));
            let svc = std::sync::Arc::new(remote::RemoteControllerService::new(state_arc, Some(state_file)));
            links.endpoint().attach(svc);
        });
        let ctx = service_kit::context::ServiceContext::standalone("gateway");
        gateway::server::run_with_network(config, capture, None, ctx, None, Some(slot), None).await
    });
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            service_kit::linelog::error(&error, serde_json::json!({}));
            ExitCode::FAILURE
        }
    }
}

#[cfg(feature = "gateway")]
fn run_network_full_node(p: &paths::Paths, hc: &hostconfig::HostConfig) -> ExitCode {
    ensure_hooks();
    let _ = hostkit::gamedata::seed_embedded_examples(p);
    let _ = std::fs::create_dir_all(&p.backend_game_data);
    let progression_config_json = serde_json::json!({
        "code": null,
        "message": "OK",
        "payload": {
            "paths": [
                {
                    "progression_id": "ExperienceTrack_PlayerLevel",
                    "requirements": [
                        { "rank_id": 1, "xp_required": 0 },
                        { "rank_id": 20, "xp_required": 10000 }
                    ],
                    "prestige": { "xp_per_level": 1000 }
                }
            ]
        }
    });
    if let Ok(formatted) = serde_json::to_string_pretty(&progression_config_json) {
        let _ = std::fs::write(p.backend_game_data.join("progression_config.json"), formatted);
    }
    let _ = std::fs::create_dir_all(&p.director_game_data);
    let player_hunts_json = serde_json::json!([
        {
            "Type": "DataTable",
            "Name": "player_hunts_table",
            "Properties": {
                "RowStruct": {
                    "ObjectName": "ScriptStruct'PlayerHuntTableData'",
                    "ObjectPath": "/Script/Example"
                }
            },
            "Rows": {
                "Example_Hunt_Easy": {
                    "HuntName": { "SourceString": "Example Hunt", "LocalizedString": "Example Hunt" },
                    "HuntDescription": { "SourceString": "Synthetic placeholder hunt.", "LocalizedString": "Synthetic placeholder hunt." },
                    "MatchmakerHuntIDs": [ { "RowName": "Example_MatchmakerHunt" } ]
                },
                "CR19_PlayerHunt_Example": {
                    "HuntName": { "SourceString": "Example Ring Hunt", "LocalizedString": "Example Ring Hunt" },
                    "HuntDescription": { "SourceString": "Synthetic matchmaking ring hunt.", "LocalizedString": "Synthetic matchmaking ring hunt." },
                    "MatchmakerHuntIDs": [ { "RowName": "Example_MatchmakerHunt" } ]
                }
            }
        }
    ]);
    if let Ok(formatted) = serde_json::to_string_pretty(&player_hunts_json) {
        let _ = std::fs::write(p.director_game_data.join("player_hunts_table.json"), formatted);
    }
    let matchmaker_hunts_json = serde_json::json!([
        {
            "Type": "DataTable",
            "Name": "matchmaker_hunts_table",
            "Rows": {
                "Example_MatchmakerHunt": {
                    "MapList": [
                        {
                            "MapAssetName": "/Game/Maps/islands/island_example.island_example"
                        }
                    ],
                    "SpecificBehemoth": {
                        "BehemothAsset": {
                            "AssetPathName": "None"
                        }
                    }
                }
            }
        }
    ]);
    if let Ok(formatted) = serde_json::to_string_pretty(&matchmaker_hunts_json) {
        let _ = std::fs::write(p.director_game_data.join("matchmaker_hunts_table.json"), formatted);
    }
    if let Some(parent) = p.gameserver_wrapper.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let wrapper_script = "#!/bin/sh\necho \"RUSTIC_ISLES_GAMESERVER_READY launchId=${RUSTIC_ISLES_GAMESERVER_LAUNCH_ID} port=$2\"\nexec sleep 3600\n";
    let _ = std::fs::write(&p.gameserver_wrapper, wrapper_script);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p.gameserver_wrapper, std::fs::Permissions::from_mode(0o755));
    }
    if let Err(error) = serve::ensure_server_files(p, hc) {
        service_kit::linelog::error(&error.to_string(), serde_json::json!({}));
        return ExitCode::FAILURE;
    }
    let director_env = p.config.join("director.env");
    if let Ok(raw) = std::fs::read_to_string(&director_env) {
        let updated = raw
            .replace("GAMESERVER_MEMORY_GUARD=\"1\"", "GAMESERVER_MEMORY_GUARD=0")
            .replace("GAMESERVER_MEMORY_GUARD=1", "GAMESERVER_MEMORY_GUARD=0")
            .replace(
                "SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP=\"3\"",
                "SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP=0",
            )
            .replace(
                "SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP=3",
                "SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP=0",
            );
        let _ = std::fs::write(&director_env, updated);
    }
    let allowlist_env = p.config.join("allowlist.env");
    if let Ok(raw) = std::fs::read_to_string(&allowlist_env) {
        let mut updated = raw
            .replace("ALLOWLIST_DRY_RUN=\"0\"", "ALLOWLIST_DRY_RUN=1")
            .replace("ALLOWLIST_DRY_RUN=0", "ALLOWLIST_DRY_RUN=1");
        if !updated.contains("ALLOWLIST_ALLOW_PRIVATE") {
            updated.push_str("\nALLOWLIST_ALLOW_PRIVATE=1\n");
        }
        let _ = std::fs::write(&allowlist_env, updated);
    }
    let mut controller = match serve::compose(p, hc, false, serve::table_printer()) {
        Ok(c) => c,
        Err(error) => {
            service_kit::linelog::error(&error.to_string(), serde_json::json!({}));
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(error) => {
            service_kit::linelog::error(&error.to_string(), serde_json::json!({}));
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async move {
        if let Err(error) = controller.start_all().await {
            let _ = controller.shutdown().await;
            service_kit::linelog::error(&error.to_string(), serde_json::json!({}));
            return ExitCode::FAILURE;
        }
        if let Some(reg) = controller.registry() {
            for info in reg.list() {
                if info.autoload && info.state == controller::ModuleState::Failed {
                    let detail = info.detail.unwrap_or_else(|| "module load failed".into());
                    service_kit::linelog::error(&detail, serde_json::json!({ "module": info.id.as_str() }));
                }
            }
        }
        service_kit::serve::stop_requested().await;
        controller.shutdown().await;
        ExitCode::SUCCESS
    })
}

fn remote_agent_service_main(args: &[String]) -> ExitCode {
    ensure_hooks();
    let repo_paths = paths::locate_repo_root().ok().map(paths::Paths::new);
    let env_file = parse_service_env_file("remote-agent", args, repo_paths.as_ref())
        .ok()
        .flatten();
    let paths = env_file
        .as_deref()
        .and_then(|file| {
            let config_dir = file.parent()?;
            if config_dir.file_name().is_some_and(|n| n == "config") {
                let ws_dir = config_dir.parent()?;
                if ws_dir.file_name().is_some_and(|n| n == ".workspace") {
                    return ws_dir.parent().map(paths::Paths::new);
                }
            }
            None
        })
        .or(repo_paths);
    let Some(p) = paths else {
        eprintln!("Configuration: could not locate server workspace");
        return ExitCode::FAILURE;
    };
    let sync_args = cli::RemoteSyncArgs {
        continuous: true,
        interval_secs: None,
        json: false,
    };
    match hostkit::commands::remote::sync_with_controller(&p, &sync_args) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(feature = "isles-1-14-7")]
fn isles_service_main(name: &'static str, args: &[String]) -> ExitCode {
    use module_api::GameModule as _;

    ensure_hooks();
    let repo_paths = paths::locate_repo_root().ok().map(paths::Paths::new);
    let env_file = match parse_service_env_file(name, args, repo_paths.as_ref()) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("Configuration: {error}");
            return ExitCode::FAILURE;
        }
    };
    let paths = env_file
        .as_deref()
        .and_then(|file| {
            let config_dir = file.parent()?;
            if config_dir.file_name().is_some_and(|n| n == "config") {
                let ws_dir = config_dir.parent()?;
                if ws_dir.file_name().is_some_and(|n| n == ".workspace") {
                    return ws_dir.parent().map(paths::Paths::new);
                }
            }
            None
        })
        .or(repo_paths);
    let host_config = paths
        .as_ref()
        .and_then(|p| hostconfig::load_host_config(p).ok().flatten());
    if name == "backend"
        && let Some(p) = &paths
    {
        if let Some(hc) = &host_config {
            if let Err(error) = hostkit::commands::update::check_unmigrated_mongo_data(p, hc) {
                eprintln!("Unmigrated MongoDB data: {error}");
                return ExitCode::FAILURE;
            }
        } else if !p.config.join(hostkit::commands::update::MONGO_MIGRATED_MARKER).exists()
            && hostkit::commands::update::has_unmigrated_mongo_dir(p)
        {
            eprintln!(
                "Unmigrated MongoDB data found in {}: move it with `reforged-host db migrate` and `db verify` (or `dainty-legacy import reforged <checkout>`), or pass `reforged-host setup --fresh-start` to start with an empty database without modifying MongoDB.",
                p.mongo_data.display()
            );
            return ExitCode::FAILURE;
        }
    }
    let identity =
        host_config
            .as_ref()
            .and_then(|c| c.network.as_ref())
            .map_or_else(module_api::NodeIdentity::standalone, |n| module_api::NodeIdentity {
                node: n.node.clone(),
                region: n.region.clone(),
            });

    let mut spec =
        match version_data::load().and_then(|data| rustic_isles_1_14_7::spec_with(data).map_err(|e| e.message)) {
            Ok(spec) => spec,
            Err(error) => {
                eprintln!("Configuration: {error}");
                return ExitCode::FAILURE;
            }
        };
    spec.services = match name {
        "backend" => vec![std::sync::Arc::new(rustic_isles::BackendService)],
        "director" => vec![std::sync::Arc::new(rustic_isles_director::DirectorService::new(
            rustic_isles_1_14_7::DIRECTOR,
        ))],
        _ => Vec::new(),
    };
    let module = match rustic_isles::IslesModule::new(spec) {
        Ok(module) => module,
        Err(error) => {
            eprintln!("Configuration: {error}");
            return ExitCode::FAILURE;
        }
    };
    let manifest = module.manifest();
    let mut registrar = module_api::Registrar::new(manifest.clone());
    if let Err(error) = module.register(&mut registrar) {
        eprintln!("Configuration: {error}");
        return ExitCode::FAILURE;
    }
    let registration = match registrar.finish() {
        Ok(reg) => reg,
        Err(error) => {
            eprintln!("Configuration: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut config = module_api::ModuleConfig::defaults(&manifest, &registration);
    if let Some(file) = &env_file {
        config.sections.entry("backend".to_owned()).or_default().insert(
            "env_file".to_owned(),
            serde_json::Value::String(file.display().to_string()),
        );
        if let Ok(entries) = hostconfig::read_env_file(file) {
            if let Some(port) = entries.get("PORT").and_then(|v| v.parse::<u16>().ok()) {
                config
                    .ports
                    .insert(name.to_owned(), module_api::PortRange::single(port));
            }
            if let (Some(first), Some(last)) = (
                entries.get("PORT_RANGE_BEGIN").and_then(|v| v.parse::<u16>().ok()),
                entries.get("PORT_RANGE_END").and_then(|v| v.parse::<u16>().ok()),
            ) {
                config
                    .ports
                    .insert("game".to_owned(), module_api::PortRange { first, last });
            }
        }
    }
    if let Some(p) = &paths {
        let game_data = config.sections.entry("game_data".to_owned()).or_default();
        if p.backend_game_data.is_dir() {
            game_data.insert(
                "backend_dir".to_owned(),
                serde_json::Value::String(p.backend_game_data.display().to_string()),
            );
        }
        if p.director_game_data.is_dir() {
            game_data.insert(
                "director_dir".to_owned(),
                serde_json::Value::String(p.director_game_data.display().to_string()),
            );
        }
    }
    if let Some(switches) = host_config.as_ref().and_then(|c| c.switches.as_ref()) {
        let section = config.sections.entry("switches".to_owned()).or_default();
        for (key, value) in switches.module_switches() {
            section.insert(key.to_owned(), value);
        }
    }
    let ctx = module_api::ModuleContext::new(
        manifest.id,
        identity.clone(),
        std::sync::Arc::new(StandaloneLink {
            home: module_api::network::StandaloneHome { identity },
        }),
    );
    let _ = service_kit::telemetry::init_table_stdout_subscriber(tracing::level_filters::LevelFilter::INFO);
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async move {
        if let Err(error) = module.start(ctx.clone(), config).await {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        service_kit::serve::stop_requested().await;
        if let Err(error) = module.stop(ctx).await {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        ExitCode::SUCCESS
    })
}

pub mod doctor {
    pub use hostkit::doctor::*;

    pub fn run_doctor(p: &crate::paths::Paths, opts: &DoctorOptions) -> Result<Vec<Check>, crate::HostError> {
        crate::ensure_hooks();
        hostkit::doctor::run_doctor(p, opts)
    }

    pub fn store_database_check(p: &crate::paths::Paths, config: &crate::hostconfig::HostConfig) -> Check {
        crate::ensure_hooks();
        hostkit::doctor::store_database_check(p, config)
    }
}

pub mod gamedata {
    pub use hostkit::gamedata::*;

    pub fn run_generator(
        p: &crate::paths::Paths,
        step: &Step<'_>,
        log_file: &std::path::Path,
    ) -> Result<crate::run::RunResult, crate::HostError> {
        crate::ensure_hooks();
        hostkit::gamedata::run_generator(p, step, log_file)
    }

    pub fn generate_game_data(
        p: &crate::paths::Paths,
        opts: &GenerateOptions,
        print: &mut dyn FnMut(&str),
    ) -> Result<Generated, crate::HostError> {
        crate::ensure_hooks();
        hostkit::gamedata::generate_game_data(p, opts, print)
    }

    pub fn generate_game_data_with(
        p: &crate::paths::Paths,
        opts: &GenerateOptions,
        print: &mut dyn FnMut(&str),
        hooks: &Generators<'_>,
    ) -> Result<Generated, crate::HostError> {
        crate::ensure_hooks();
        hostkit::gamedata::generate_game_data_with(p, opts, print, hooks)
    }
}

pub mod serve {
    pub use hostkit::serve::*;

    pub fn compose(
        paths: &crate::paths::Paths,
        config: &crate::hostconfig::HostConfig,
        external_allowlist: bool,
        printer: crate::services::Printer,
    ) -> Result<controller::ServerController, crate::HostError> {
        crate::ensure_hooks();
        hostkit::serve::compose(paths, config, external_allowlist, printer)
    }

    pub fn new_registry(
        config: &crate::hostconfig::HostConfig,
        controller: &controller::ServerController,
    ) -> controller::ModuleRegistry {
        crate::ensure_hooks();
        hostkit::serve::new_registry(config, controller)
    }

    pub fn ensure_server_files(
        paths: &crate::paths::Paths,
        config: &crate::hostconfig::HostConfig,
    ) -> Result<Vec<std::path::PathBuf>, crate::HostError> {
        crate::ensure_hooks();
        hostkit::serve::ensure_server_files(paths, config)
    }

    pub fn run(paths: &crate::paths::Paths, options: &ServeOptions) -> crate::Outcome {
        crate::ensure_hooks();
        hostkit::serve::run(paths, options)
    }

    pub fn prepare_embedded(
        paths: &crate::paths::Paths,
        printer: crate::services::Printer,
    ) -> Result<controller::ServerController, String> {
        crate::ensure_hooks();
        hostkit::serve::prepare_embedded(paths, printer)
    }

    pub fn minimal_main() -> std::process::ExitCode {
        crate::ensure_hooks();
        hostkit::serve::minimal_main()
    }
}

pub mod commands {
    pub use hostkit::commands::{backup, history, install_game, invite, multipliers, network, preset, remote};

    pub mod gamedata {
        pub fn run(paths: &crate::paths::Paths, args: &crate::cli::GamedataArgs) -> crate::Outcome {
            crate::ensure_hooks();
            hostkit::commands::gamedata::run(paths, args)
        }

        pub fn run_with(
            paths: &crate::paths::Paths,
            args: &crate::cli::GamedataArgs,
            out: &mut dyn FnMut(&str),
        ) -> crate::Outcome {
            crate::ensure_hooks();
            hostkit::commands::gamedata::run_with(paths, args, out)
        }
    }

    pub mod publish_runtime {
        pub fn run(paths: &crate::paths::Paths, args: &crate::cli::PublishRuntimeArgs) -> crate::Outcome {
            crate::ensure_hooks();
            hostkit::commands::publish_runtime::run(paths, args)
        }

        pub fn run_with(
            paths: &crate::paths::Paths,
            args: &crate::cli::PublishRuntimeArgs,
            out: &mut dyn FnMut(&str),
        ) -> crate::Outcome {
            crate::ensure_hooks();
            hostkit::commands::publish_runtime::run_with(paths, args, out)
        }
    }

    pub fn dispatch(command: &crate::cli::Command, paths: &crate::paths::Paths) -> crate::Outcome {
        crate::ensure_hooks();
        hostkit::commands::dispatch(command, paths)
    }

    pub mod db {
        pub use hostkit::commands::db::*;

        pub fn run(paths: &crate::paths::Paths, args: &crate::cli::DbArgs) -> crate::Outcome {
            crate::ensure_hooks();
            hostkit::commands::db::run(paths, args)
        }

        pub fn run_with(
            paths: &crate::paths::Paths,
            args: &crate::cli::DbArgs,
            out: &mut dyn FnMut(&str),
        ) -> crate::Outcome {
            crate::ensure_hooks();
            hostkit::commands::db::run_with(paths, args, out)
        }
    }

    pub mod doctor {
        pub use hostkit::commands::doctor::*;

        pub fn run(paths: &crate::paths::Paths, args: &crate::cli::DoctorArgs) -> std::process::ExitCode {
            crate::ensure_hooks();
            hostkit::commands::doctor::run(paths, args)
        }
    }

    pub mod setup {
        pub use hostkit::commands::setup::*;

        pub fn run(paths: &crate::paths::Paths, args: &crate::cli::SetupArgs) -> std::process::ExitCode {
            crate::ensure_hooks();
            hostkit::commands::setup::run(paths, args)
        }

        pub fn run_with(
            p: &crate::paths::Paths,
            args: &crate::cli::SetupArgs,
            env: &dyn SetupEnv,
            out: &mut dyn FnMut(&str),
        ) -> Result<std::process::ExitCode, crate::HostError> {
            crate::ensure_hooks();
            hostkit::commands::setup::run_with(p, args, env, out)
        }
    }

    pub mod start {
        pub use hostkit::commands::start::*;

        pub fn run(paths: &crate::paths::Paths) -> crate::Outcome {
            crate::ensure_hooks();
            hostkit::commands::start::run(paths)
        }

        pub mod binaries {
            pub use hostkit::commands::start::binaries::*;
        }
    }

    pub mod update {
        pub use hostkit::commands::update::*;

        pub fn run(paths: &crate::paths::Paths, args: &crate::cli::UpdateArgs) -> std::process::ExitCode {
            crate::ensure_hooks();
            hostkit::commands::update::run(paths, args)
        }

        pub fn run_with(
            p: &crate::paths::Paths,
            args: &crate::cli::UpdateArgs,
            env: &dyn UpdateEnv,
            out: &mut dyn FnMut(&str),
        ) -> Result<std::process::ExitCode, crate::HostError> {
            crate::ensure_hooks();
            hostkit::commands::update::run_with(p, args, env, out)
        }

        pub fn check_unmigrated_mongo_data(
            p: &crate::paths::Paths,
            config: &crate::hostconfig::HostConfig,
        ) -> Result<(), crate::HostError> {
            crate::ensure_hooks();
            hostkit::commands::update::check_unmigrated_mongo_data(p, config)
        }

        pub fn inspect_mongo_migration(p: &crate::paths::Paths) -> MongoMigrationStatus {
            crate::ensure_hooks();
            hostkit::commands::update::inspect_mongo_migration(p)
        }
    }
}

pub fn run(cli: &cli::Cli) -> Outcome {
    ensure_hooks();
    hostkit::run(cli)
}

pub fn service_main(name: &str, args: &[String]) -> ExitCode {
    ensure_hooks();
    hostkit::service_main(name, args)
}

pub fn host_main() -> ExitCode {
    ensure_hooks();
    let cli = cli::Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(error) => {
            eprint!("\nError: {error}\n");
            ExitCode::FAILURE
        }
    }
}

pub fn console_main() -> ExitCode {
    ensure_hooks();
    hostkit::console_main()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[cfg(feature = "isles-1-14-7")]
    #[test]
    fn built_in_services_include_backend_and_director() {
        ensure_hooks();
        assert!(BUILT_IN_SERVICES.contains(&"backend"));
        assert!(BUILT_IN_SERVICES.contains(&"director"));
        let cli = cli::Cli::try_parse_from([
            "reforged-host",
            "backend",
            "serve",
            "--env-file",
            "/opt/reforged/.workspace/config/backend.env",
        ])
        .unwrap();
        assert_eq!(cli.command.name(), "backend");
        let cli = cli::Cli::try_parse_from([
            "reforged-host",
            "director",
            "serve",
            "--env-file=/opt/reforged/.workspace/config/director.env",
        ])
        .unwrap();
        assert_eq!(cli.command.name(), "director");
        assert_eq!(
            parse_service_env_file(
                "backend",
                &[
                    "serve".into(),
                    "--env-file".into(),
                    "/opt/reforged/.workspace/config/backend.env".into()
                ],
                None
            )
            .unwrap()
            .as_deref(),
            Some(std::path::Path::new("/opt/reforged/.workspace/config/backend.env"))
        );
        assert!(parse_service_env_file("backend", &["--env-file".into()], None).is_err());
    }

    #[cfg(feature = "db-tools")]
    #[test]
    fn db_migrate_verify_and_export_mongo_work_with_host_toml_defaults() {
        use crate::cli::{DbArgs, DbCommand, DbExportMongoArgs, DbMigrateArgs, DbVerifyArgs};
        use crate::hostconfig::{
            DatabaseSettings, FileForm, HostConfig, MongoConfig, Registration, ServerMode, save_host_config,
        };
        use rustic_isles_db::mongo::MockMongoServer;
        use serde_json::json;

        ensure_hooks();
        let mock = MockMongoServer::start(50).unwrap();
        mock.seed(
            "accounts",
            vec![json!({
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
            })],
        );
        let mongo_uri = mock.uri("reforged");

        let tmp = tempfile::tempdir().unwrap();
        let paths = paths::Paths::new(tmp.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        let cfg = HostConfig {
            version: 1,
            mode: ServerMode::Private,
            name: "Test Server".into(),
            host: "100.64.0.1".into(),
            game_address: "100.64.0.1".into(),
            public_port: None,
            dashboard_url: None,
            game_dir: None,
            registration: Registration::Invitecode,
            share_url: None,
            max_hunt_servers: 3,
            mongo: MongoConfig {
                uri: mongo_uri,
                manage: false,
                mongod: None,
            },
            performance: None,
            multipliers: None,
            linux: None,
            database: Some(DatabaseSettings {
                engine: Some("sqlite".into()),
                ..DatabaseSettings::default()
            }),
            guard: None,
            history: None,
            switches: None,
            modules: None,
            services: None,
            env: None,
            network: None,
            remote: None,
            file: FileForm::default(),
        };
        save_host_config(&paths, &cfg).unwrap();

        let mut lines = Vec::new();
        commands::db::run_with(
            &paths,
            &DbArgs {
                command: DbCommand::Migrate(DbMigrateArgs::default()),
            },
            &mut |l| lines.push(l.to_owned()),
        )
        .unwrap();
        assert!(lines.iter().any(|l| l.contains("accounts: 1")));

        lines.clear();
        commands::db::run_with(
            &paths,
            &DbArgs {
                command: DbCommand::Verify(DbVerifyArgs::default()),
            },
            &mut |l| lines.push(l.to_owned()),
        )
        .unwrap();
        assert!(lines.iter().any(|l| l.contains("reforged-host db verify: OK")));
        assert!(paths.config.join(commands::update::MONGO_MIGRATED_MARKER).exists());

        lines.clear();
        commands::db::run_with(
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

    #[tokio::test(flavor = "multi_thread")]
    async fn compose_runs_gateway_and_content_in_process_without_node() {
        use commands::start::binaries::{FrontService, rust_ported};
        use hostconfig::{DEFAULT_MONGO_URI, FileForm, HostConfig, MongoConfig, Registration, ServerMode};

        ensure_hooks();
        let dir = tempfile::tempdir().unwrap();
        let paths = paths::Paths::new(dir.path());
        let c = HostConfig {
            version: 1,
            mode: ServerMode::Private,
            host: "127.0.0.1".into(),
            game_address: "127.0.0.1".into(),
            name: "Friday Hunts".into(),
            game_dir: None,
            registration: Registration::parse("INVITECODE").unwrap(),
            share_url: None,
            mongo: MongoConfig {
                uri: DEFAULT_MONGO_URI.into(),
                manage: true,
                mongod: None,
            },
            max_hunt_servers: 3,
            multipliers: None,
            public_port: None,
            dashboard_url: None,
            performance: None,
            linux: None,
            database: None,
            guard: None,
            history: None,
            modules: None,
            services: None,
            env: None,
            network: None,
            remote: None,
            switches: Default::default(),
            file: FileForm::default(),
        };
        serve::ensure_server_files(&paths, &c).unwrap();
        let content_port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let gateway_port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let content_env = commands::start::env_file(&paths, "content");
        let content_text = std::fs::read_to_string(&content_env)
            .unwrap()
            .replace("PORT=61002", &format!("PORT={content_port}"));
        std::fs::write(&content_env, content_text).unwrap();
        let gateway_env = commands::start::env_file(&paths, "gateway");
        let gateway_text = std::fs::read_to_string(&gateway_env)
            .unwrap()
            .replace("GATEWAY_PORT=61000", &format!("GATEWAY_PORT={gateway_port}"));
        std::fs::write(&gateway_env, gateway_text).unwrap();

        let mut controller = serve::compose(&paths, &c, false, serve::table_printer()).unwrap();
        controller.start_component("content").await.unwrap();
        controller.start_component("gateway").await.unwrap();
        assert!(rust_ported(FrontService::Gateway));
        assert!(rust_ported(FrontService::Content));
        controller.shutdown().await;

        // [services.<name>] takes a service out of the binary: the library runs in its place, pinned by its SHA-256
        let digest = "ab".repeat(32);
        let doc = hostkit::hosttoml::document(&format!(
            "[services.content]\nlibrary = \"plugins/content_plugin.dll\"\nsha256 = \"{digest}\"\n\
             [services.gateway]\nlibrary = \"plugins/gateway_plugin.dll\"\nsha256 = \"{digest}\"\n\
             [services.chat]\nlibrary = \"plugins/chat_plugin.dll\"\nsha256 = \"{digest}\"\n"
        ))
        .unwrap();
        let mut libraries = c.clone();
        libraries.services = Some(hostconfig::services::from_json(&doc["services"]).unwrap());
        let mut controller = serve::compose(&paths, &libraries, false, serve::table_printer()).unwrap();
        let names = controller.module_names();
        for name in ["content", "gateway", "chat"] {
            assert_eq!(names.iter().filter(|n| *n == name).count(), 1, "{names:?}");
        }
        let refused = controller.start_component("content").await.unwrap_err().to_string();
        assert!(refused.contains("content_plugin.dll"), "{refused}");
        controller.shutdown().await;
    }

    #[cfg(all(feature = "isles-1-14-7", not(feature = "paradox")))]
    #[tokio::test(flavor = "multi_thread")]
    async fn compose_starts_rust_1_14_7_backend_and_director_without_node_or_mongo() {
        use hostconfig::{DEFAULT_MONGO_URI, FileForm, HostConfig, MongoConfig, Registration, ServerMode};

        ensure_hooks();
        let dir = tempfile::tempdir().unwrap();
        let paths = paths::Paths::new(dir.path());
        let c = HostConfig {
            version: 1,
            mode: ServerMode::Private,
            host: "127.0.0.1".into(),
            game_address: "127.0.0.1".into(),
            name: "Friday Hunts".into(),
            game_dir: None,
            registration: Registration::parse("INVITECODE").unwrap(),
            share_url: None,
            mongo: MongoConfig {
                uri: DEFAULT_MONGO_URI.into(),
                manage: true,
                mongod: None,
            },
            max_hunt_servers: 3,
            multipliers: None,
            public_port: None,
            dashboard_url: None,
            performance: None,
            linux: None,
            database: None,
            guard: None,
            history: None,
            modules: None,
            services: None,
            env: None,
            network: None,
            remote: None,
            switches: Default::default(),
            file: FileForm::default(),
        };
        serve::ensure_server_files(&paths, &c).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        let repo_paths = paths::Paths::new(&repo);
        std::fs::create_dir_all(&paths.backend_game_data).unwrap();
        std::fs::create_dir_all(&paths.director_game_data).unwrap();
        for f in gamedata::BACKEND_GAME_DATA {
            let src = gamepack::example_of(&repo_paths.backend_game_data.join(f));
            std::fs::copy(&src, paths.backend_game_data.join(f)).unwrap();
        }
        for f in gamedata::DIRECTOR_GAME_DATA {
            let src = gamepack::example_of(&repo_paths.director_game_data.join(f));
            std::fs::copy(&src, paths.director_game_data.join(f)).unwrap();
        }

        let content_port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let admin_port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let gateway_port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let content_env = commands::start::env_file(&paths, "content");
        let content_text = std::fs::read_to_string(&content_env)
            .unwrap()
            .replace("PORT=61002", &format!("PORT={content_port}"));
        std::fs::write(&content_env, content_text).unwrap();
        let admin_env = commands::start::env_file(&paths, "admin");
        let admin_text = std::fs::read_to_string(&admin_env)
            .unwrap()
            .replace("PORT=61003", &format!("PORT={admin_port}"));
        std::fs::write(&admin_env, admin_text).unwrap();
        let gateway_env = commands::start::env_file(&paths, "gateway");
        let gateway_text = std::fs::read_to_string(&gateway_env)
            .unwrap()
            .replace("GATEWAY_PORT=61000", &format!("GATEWAY_PORT={gateway_port}"));
        std::fs::write(&gateway_env, gateway_text).unwrap();

        let mut controller = serve::compose(&paths, &c, false, serve::table_printer()).unwrap();
        assert_eq!(controller.module_names(), ["content", "admin", "gateway", "chat"]);
        controller.start_all().await.unwrap();

        let registry = controller.registry().unwrap();
        let isles = registry.info(&"isles-1-14-7".parse().unwrap()).unwrap();
        assert_eq!(isles.state, controller::ModuleState::Ready, "{:?}", isles.detail);
        assert!(isles.autoload);

        let reply = invite::http_request(
            "GET",
            gateway_port,
            "/dauntless-status",
            &[("X-Dauntless-Game-Version", "1.14.7")],
            None,
            Duration::from_secs(5),
        )
        .unwrap();
        let body = &reply.body;
        assert_eq!(reply.status, 200, "{body}");
        assert!(body.contains("\"show-status\""), "{body}");
        assert!(paths.config.join(hostconfig::DATABASE_FILE).is_file());
        let (secrets, _) =
            hostconfig::load_or_create_secrets(&paths.config, commands::setup::secrets::new_secrets).unwrap();
        let invites = invite::list_invite_codes(paths::PORTS.backend, &secrets.admin_key).unwrap();
        assert!(invites.is_empty(), "{invites:?}");

        controller.shutdown().await;
    }
}
