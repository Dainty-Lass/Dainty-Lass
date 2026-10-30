//! The host kit (`reforged-host`): thin AGPL shell wiring `hostkit` (MIT) to the compiled-in
//! services (`gateway`, `allowlist`, `content`, `admin`, `isles-1-14-7`), `isles-db` migration,
//! `isles-core` / `isles-director` generators, and the `legacy` 1.4.4 stack.

pub use hostkit::{
    HostError, Outcome, captures, cli, envfile, epicinstall, error, gamepack, guarded, hooks, hostconfig, hosttoml,
    invite, linux, mongo, paths, preset, progression, publish, repl, selfupdate, services, tls, tui, wine,
    with_admin_env,
};

pub mod run {
    pub use hostkit::run::*;
}

// Dauntless Revived's 1.4.4 servers (RD5 reversed); without the feature, refusals with the same signatures
#[cfg(feature = "legacy")]
pub mod legacy;
#[cfg(not(feature = "legacy"))]
#[path = "legacy/off.rs"]
pub mod legacy;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser as _;
use controller::modules;

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
];

struct ServicesHostHooks;

static SERVICES_HOST_HOOKS: ServicesHostHooks = ServicesHostHooks;

#[inline]
pub fn ensure_hooks() {
    hostkit::hooks::set_hooks(&SERVICES_HOST_HOOKS);
}

#[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
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
    match generator {
        GeneratorKind::ProgressionConfig => {
            isles_core::gamedata::generate_progression_config(&p.raw_game_data, &p.backend_game_data, true)
        }
        GeneratorKind::HuntTables => {
            isles_director::hunt_tables::import_hunt_tables(&isles_director::hunt_tables::ImportHuntTablesOptions {
                export_dir: p.exports.join("hunts_1_14_7"),
                vendor_dir: p.director_game_data.clone(),
                repo_root: Some(p.repo.clone()),
                apply: true,
                update_existing: false,
                verbose: false,
                backup_stamp: None,
            })
            .map(|report| report.output)
        }
        GeneratorKind::SlayersPath => isles_core::gamedata::generate_slayers_path(
            &p.exports.join("slayers_path_1_14_7").join("player_journey_nodes.jsonl"),
            &p.backend_game_data.join("slayers_path.json"),
            true,
        ),
        GeneratorKind::SlayersPathDefinitions => isles_core::gamedata::generate_slayers_path_definitions(
            &p.exports.join("slayers_path_1_14_7").join("player_journey_nodes.jsonl"),
            &p.backend_game_data.join("slayers_path_definitions.json"),
            "1_14_7",
            true,
        ),
        GeneratorKind::RewardCache => {
            let catalog = p.exports.join("catalog_1_14_7.jsonl");
            let out = p.backend_game_data.join("reward_cache_store.json");
            let a = isles_core::gamedata::generate_reward_cache_store(&catalog, &out)?;
            let b = isles_core::gamedata::generate_inventory_storage_policy(&catalog, None, &p.backend_game_data)?;
            Ok(format!("{a}\n{b}"))
        }
        GeneratorKind::LadyLuckGenerate => {
            isles_core::gamedata::generate_ladyluck_store(&p.raw_game_data, &p.backend_game_data, true)
        }
        GeneratorKind::LadyLuckEnrich => {
            let catalog = p.exports.join("catalog_1_14_7.jsonl");
            let live = p
                .store_captures
                .join("Store")
                .join("Product")
                .join("Skus")
                .join("ladyluckstore.json");
            let store = p.backend_game_data.join("ladyluck_store.json");
            let a = isles_core::gamedata::enrich_ladyluck_store(&catalog, &live, &store)?;
            let b = isles_core::gamedata::generate_inventory_storage_policy(&catalog, None, &p.backend_game_data)?;
            Ok(format!("{a}\n{b}"))
        }
        GeneratorKind::PlatinumAndStoragePolicy => {
            let catalog = isles_core::gamedata::resolve_platinum_export_file(&p.exports, "catalog");
            let store_images = isles_core::gamedata::resolve_platinum_export_file(&p.exports, "store_item_images");
            let captures = p.store_captures.join("Store").join("Product").join("Skus");
            let a = isles_core::gamedata::generate_platinum_store(
                &catalog,
                &store_images,
                &captures,
                &p.backend_game_data,
            )?;
            let policy_catalog = p.exports.join("catalog_1_14_7.jsonl");
            let b =
                isles_core::gamedata::generate_inventory_storage_policy(&policy_catalog, None, &p.backend_game_data)?;
            Ok(format!("{a}\n{b}"))
        }
    }
}

#[cfg(not(feature = "isles-1-14-7"))]
fn execute_rust_generator(_p: &paths::Paths, generator: hostkit::gamedata::GeneratorKind) -> Result<String, String> {
    Err(format!(
        "{} is unavailable: this binary was built without the isles-1-14-7 feature",
        generator.label()
    ))
}

#[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
mod db_impl {
    use std::path::PathBuf;
    use std::process::ExitCode;

    use crate::Outcome;
    use crate::cli::{DbArgs, DbCommand, DbExportMongoArgs, DbMigrateArgs, DbVerifyArgs};
    use crate::error::HostError;
    use crate::hostconfig::{HostConfig, load_host_config};
    use crate::paths::Paths;
    use hostkit::commands::db::{redact_credentials_in_text, resolve_mongo_uri, resolve_store_target};
    use hostkit::commands::update::{MONGO_MIGRATED_MARKER, redact_uri_password};

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
}

impl hooks::HostHooks for ServicesHostHooks {
    fn built_in_services(&self) -> &'static [&'static str] {
        BUILT_IN_SERVICES
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

    fn mongo_has_documents(&self, uri: &str) -> Result<bool, String> {
        #[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
        {
            let mut client = isles_db::mongo::MongoClient::connect(uri).map_err(|e| e.to_string())?;
            let primary = [
                "accounts",
                "characters",
                "inventories",
                "loadouts",
                "reforgedInvites",
                "wallets",
            ];
            for col in primary {
                if client.collection_has_documents(col).unwrap_or(false) {
                    return Ok(true);
                }
            }
            let all_areas: std::collections::BTreeSet<isles_db::MigrateArea> =
                isles_db::MigrateArea::ALL.into_iter().collect();
            let empty_state = isles_db::migrate::TargetState::default();
            for col in isles_db::state_to_mongo_collections(&empty_state, &all_areas).into_keys() {
                if !primary.contains(&col.as_str()) && client.collection_has_documents(&col).unwrap_or(false) {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        #[cfg(not(any(feature = "isles-1-14-7", feature = "paradox")))]
        {
            let _ = uri;
            Err("MongoDB support is not compiled in".to_owned())
        }
    }

    fn db_migrate(
        &self,
        source_uri: &str,
        db_cfg: &store::DatabaseConfig,
        dry_run: bool,
    ) -> Result<(Vec<(String, usize)>, Vec<(String, String)>), HostError> {
        #[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
        {
            let (to_target, allow_remote_postgres) = db_cfg_to_target(db_cfg);
            let opts = isles_db::MigrateOptions {
                from_uri: source_uri.to_owned(),
                to_target,
                node: "local"
                    .parse::<module_api::NodeId>()
                    .map_err(|e| HostError::message(e.to_string()))?,
                dry_run,
                areas: Vec::new(),
                skip_invalid: false,
                game_data_dir: None,
                allow_remote_postgres,
            };
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| HostError::message(format!("could not start runtime: {e}")))?;
            let report = runtime
                .block_on(isles_db::run_migrate(&opts))
                .map_err(|e| HostError::message(e.to_string()))?;
            let counts: Vec<(String, usize)> = report.counts.into_iter().collect();
            Ok((counts, Vec::new()))
        }
        #[cfg(not(any(feature = "isles-1-14-7", feature = "paradox")))]
        {
            let _ = (source_uri, db_cfg, dry_run);
            Err(HostError::message("database migration is not compiled in"))
        }
    }

    fn db_verify(&self, source_uri: &str, db_cfg: &store::DatabaseConfig) -> Result<Vec<(String, String)>, HostError> {
        #[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
        {
            let (to_target, allow_remote_postgres) = db_cfg_to_target(db_cfg);
            let opts = isles_db::MigrateOptions {
                from_uri: source_uri.to_owned(),
                to_target,
                node: "local"
                    .parse::<module_api::NodeId>()
                    .map_err(|e| HostError::message(e.to_string()))?,
                dry_run: false,
                areas: Vec::new(),
                skip_invalid: false,
                game_data_dir: None,
                allow_remote_postgres,
            };
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| HostError::message(format!("could not start runtime: {e}")))?;
            match runtime.block_on(isles_db::run_verify(&opts)) {
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
                Err(isles_db::DbError::VerificationFailed(n)) => {
                    Ok(vec![(format!("{n} collection(s)"), "verification failed".to_owned())])
                }
                Err(e) => Err(HostError::message(e.to_string())),
            }
        }
        #[cfg(not(any(feature = "isles-1-14-7", feature = "paradox")))]
        {
            let _ = (source_uri, db_cfg);
            Err(HostError::message("database verification is not compiled in"))
        }
    }

    fn db_export_mongo(&self, uri: &str, out: &Path) -> Result<Vec<(String, usize)>, HostError> {
        #[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
        {
            let opts = isles_db::ExportOptions {
                from_source: out.to_string_lossy().into_owned(),
                to_uri: uri.to_owned(),
                areas: Vec::new(),
                dry_run: false,
                allow_remote_postgres: false,
            };
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| HostError::message(format!("could not start runtime: {e}")))?;
            let report = runtime
                .block_on(isles_db::run_export_mongo(&opts))
                .map_err(|e| HostError::message(e.to_string()))?;
            Ok(report.counts.into_iter().collect())
        }
        #[cfg(not(any(feature = "isles-1-14-7", feature = "paradox")))]
        {
            let _ = (uri, out);
            Err(HostError::message("MongoDB export is not compiled in"))
        }
    }

    fn run_db_command(&self, paths: &paths::Paths, args: &cli::DbArgs, out: &mut dyn FnMut(&str)) -> Option<Outcome> {
        #[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
        {
            Some(db_impl::run_with(paths, args, out))
        }
        #[cfg(not(any(feature = "isles-1-14-7", feature = "paradox")))]
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
        #[cfg(feature = "allowlist")]
        {
            let helper = modules::allowlist_slot();
            controller.add(
                modules::allowlist(
                    &hostkit::serve::module_env(paths, "allowlist")?,
                    helper.clone(),
                    Some(registry.clone()),
                )
                .map_err(HostError::message)?,
            );
            Ok(Some(Box::new(helper)))
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
        #[cfg(any(feature = "gateway", feature = "allowlist"))]
        let slot: Option<modules::AllowlistSlot> =
            slot.and_then(|s| s.downcast::<modules::AllowlistSlot>().ok().map(|b| *b));
        #[cfg(not(any(feature = "gateway", feature = "allowlist")))]
        let _ = slot;

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
            let game = config
                .edition
                .versions()
                .first()
                .cloned()
                .unwrap_or_else(protocol::invite::reforged_version);
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
                    allowlist: slot.clone(),
                    replication_interval: Duration::from_secs(5),
                    peer_log_dir,
                    ship_to_peer,
                })
                .map_err(HostError::message)?,
            );
        }

        #[cfg(feature = "gateway")]
        if config.edition.runs_reborn() && in_process_gateway {
            controller.add(
                modules::gateway(
                    &hostkit::serve::module_env(paths, "gateway")?,
                    slot.clone(),
                    Some(registry.clone()),
                )
                .map_err(HostError::message)?,
            );
        }
        #[cfg(not(feature = "gateway"))]
        let _ = (controller, paths, config, in_process_gateway);

        #[cfg(any(feature = "gateway", feature = "allowlist"))]
        let _ = &slot;
        #[cfg(feature = "allowlist")]
        if public && !guarded {
            modules::allowlist_absent(registry);
        }
        #[cfg(not(feature = "allowlist"))]
        let _ = (registry, public, guarded);
        Ok(())
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
        controller::modules::game_modules()
    }

    fn run_service(&self, name: &str, args: &[String]) -> Option<ExitCode> {
        let argv = std::iter::once(name.to_owned()).chain(args.iter().cloned());
        match name {
            #[cfg(feature = "gateway")]
            "gateway" => Some(gateway::cli::main_from(argv)),
            #[cfg(feature = "allowlist")]
            "allowlist" => Some(gateway::allowlist::cli::main_from(argv)),
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
            _ => {
                drop(argv);
                None
            }
        }
    }

    fn legacy_write_env_files(
        &self,
        c: &hostconfig::HostConfig,
        s: &hostconfig::Secrets,
        p: &paths::Paths,
        source_commit: &str,
        platform: hostconfig::Platform,
    ) -> Result<Vec<PathBuf>, HostError> {
        crate::legacy::write_env_files(c, s, p, source_commit, platform)
    }

    fn legacy_env_file_names(&self, c: &hostconfig::HostConfig) -> Vec<String> {
        crate::legacy::env_file_names(c)
    }

    fn legacy_service_plan(
        &self,
        paths: &paths::Paths,
        config: &hostconfig::HostConfig,
        external_allowlist: bool,
        programs: &hostkit::commands::start::binaries::Programs,
    ) -> Result<Vec<hostkit::commands::start::Planned>, HostError> {
        crate::legacy::legacy_service_plan(paths, config, external_allowlist, programs)
    }

    fn legacy_prepare(&self, paths: &paths::Paths, config: &hostconfig::HostConfig) -> Result<(), HostError> {
        crate::legacy::prepare(paths, config)
    }

    fn legacy_create_invite_code(&self, paths: &paths::Paths) -> Result<String, HostError> {
        crate::legacy::create_invite_code(paths)
    }

    fn legacy_doctor_checks(
        &self,
        p: &paths::Paths,
        config: &hostconfig::HostConfig,
        hash_exe: bool,
    ) -> Result<Vec<hostkit::doctor::Check>, HostError> {
        crate::legacy::doctor_checks(p, config, hash_exe)
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

#[cfg(feature = "isles-1-14-7")]
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
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        } else if !p.config.join(hostkit::commands::update::MONGO_MIGRATED_MARKER).exists()
            && hostkit::commands::update::has_managed_mongo_data(p)
        {
            eprintln!(
                "Unmigrated MongoDB data found in {}. Run `reforged-host setup` to migrate existing player data before starting the server, or pass `reforged-host setup --fresh-start` (or `fresh_start = true` under `[database]` in host.toml) to start with an empty database without modifying MongoDB.",
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

    let mut spec = isles_1_14_7::spec();
    spec.services = match name {
        "backend" => vec![std::sync::Arc::new(isles_core::module::BackendService)],
        "director" => vec![std::sync::Arc::new(isles_director::DirectorService::new(
            isles_1_14_7::DIRECTOR,
        ))],
        _ => Vec::new(),
    };
    let module = isles_core::module::IslesModule::new(spec);
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

        pub fn migrate_existing_mongo_data(
            p: &crate::paths::Paths,
            config: &crate::hostconfig::HostConfig,
            log_file: &std::path::Path,
            out: &mut dyn FnMut(&str),
        ) -> Result<bool, crate::HostError> {
            crate::ensure_hooks();
            hostkit::commands::update::migrate_existing_mongo_data(p, config, log_file, out)
        }

        pub fn migrate_existing_mongo_data_for_setup(
            p: &crate::paths::Paths,
            config: &crate::hostconfig::HostConfig,
            log_file: &std::path::Path,
            out: &mut dyn FnMut(&str),
        ) -> Result<bool, crate::HostError> {
            crate::ensure_hooks();
            hostkit::commands::update::migrate_existing_mongo_data_for_setup(p, config, log_file, out)
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

    #[cfg(any(feature = "isles-1-14-7", feature = "paradox"))]
    #[test]
    fn db_migrate_verify_and_export_mongo_work_with_host_toml_defaults() {
        use crate::cli::{DbArgs, DbCommand, DbExportMongoArgs, DbMigrateArgs, DbVerifyArgs};
        use crate::hostconfig::{
            DatabaseSettings, Edition, FileForm, HostConfig, MongoConfig, Registration, ServerMode, save_host_config,
        };
        use isles_db::mongo::MockMongoServer;
        use serde_json::{Map, Value, json};
        use std::collections::BTreeMap;

        ensure_hooks();
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
        let paths = paths::Paths::new(tmp.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        let cfg = HostConfig {
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
        use hostconfig::{DEFAULT_MONGO_URI, Edition, FileForm, HostConfig, MongoConfig, Registration, ServerMode};

        ensure_hooks();
        let dir = tempfile::tempdir().unwrap();
        let paths = paths::Paths::new(dir.path());
        let c = HostConfig {
            version: 1,
            mode: ServerMode::Private,
            edition: Edition::Reborn,
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
            legacy_game_dir: None,
            legacy_public_port: None,
            performance: None,
            linux: None,
            database: None,
            guard: None,
            history: None,
            modules: None,
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
    }

    #[cfg(all(feature = "isles-1-14-7", not(feature = "paradox")))]
    #[tokio::test(flavor = "multi_thread")]
    async fn compose_starts_rust_1_14_7_backend_and_director_without_node_or_mongo() {
        use hostconfig::{DEFAULT_MONGO_URI, Edition, FileForm, HostConfig, MongoConfig, Registration, ServerMode};

        ensure_hooks();
        let dir = tempfile::tempdir().unwrap();
        let paths = paths::Paths::new(dir.path());
        let c = HostConfig {
            version: 1,
            mode: ServerMode::Private,
            edition: Edition::Reborn,
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
            legacy_game_dir: None,
            legacy_public_port: None,
            performance: None,
            linux: None,
            database: None,
            guard: None,
            history: None,
            modules: None,
            network: None,
            remote: None,
            switches: Default::default(),
            file: FileForm::default(),
        };
        serve::ensure_server_files(&paths, &c).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("..");
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
        assert_eq!(controller.module_names(), ["content", "admin", "gateway"]);
        controller.start_all().await.unwrap();

        let registry = controller.registry().unwrap();
        let isles = registry.info(&"isles-1-14-7".parse().unwrap()).unwrap();
        assert_eq!(isles.state, controller::ModuleState::Ready, "{:?}", isles.detail);
        assert!(isles.autoload);
        if let Ok(isles_112) = registry.info(&"isles-1-12-0".parse().unwrap()) {
            assert!(!isles_112.autoload);
            assert_eq!(isles_112.state, controller::ModuleState::NotLoaded);
        }

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
