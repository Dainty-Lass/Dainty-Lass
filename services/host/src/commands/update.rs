//! `reforged-host update`: the Node kit's `update()` in `cli.ts`, with [`crate::selfupdate`].
//!
//! It changed meaning (RH5, section 6): from a `git` fast-forward of the checkout to the server
//! release (RD8). It looks for the newest release for this platform, downloads it, checks it against
//! `SHA256SUMS.txt` and its build provenance (with `gh`, when it is there), installs it over this
//! folder and runs setup again with the new `reforged-host`. A Node-kit host (a git checkout with no
//! `release.json`) moves over the same way: run a release's `reforged-host` from the checkout's
//! folder once.
//!
//! `--check` only compares, `--json` ends with one JSON line in the Node kit's shape (the launcher's
//! Host page reads `behind`, then `updated`, `to`, `files` and `runtimeChanged`, or `error`), and
//! `--no-setup` stops after installing.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::Outcome;
use crate::cli::UpdateArgs;
use crate::error::HostError;
#[cfg(all(feature = "isles-1-14-7", not(feature = "paradox")))]
use crate::hostconfig::load_host_config;
use crate::hostconfig::{DATABASE_FILE, HostConfig, SECRET_FILE, has_host_config, read_env_file};
use crate::paths::Paths;
use crate::selfupdate::{
    self, Attestation, Candidate, Manifest, Source, UpdateError, UpdateResult, Wanted, find_release, installing_marker,
    read_manifest,
};

/// Marker written after `update` migrates and verifies a server's MongoDB data into `store` (RU9/RB14).
pub const MONGO_MIGRATED_MARKER: &str = ".mongo-migrated";

/// What `update` asks of this PC: where releases are, the attestation check, data migration and setup.
pub trait UpdateEnv {
    fn platform(&self) -> Result<&'static str, HostError>;
    fn source(&self, installed: Option<&Manifest>) -> Source;
    fn verify_attestation(&self, archive: &Path, candidate: &Candidate, log_file: &Path) -> UpdateResult<Attestation>;
    /// `git rev-parse --short=12 HEAD` of a checkout with no release, or "".
    fn checkout_commit(&self, root: &Path) -> String;
    /// Runs `program setup` in `root`; returns its exit code.
    fn run_setup(&self, program: &Path, root: &Path) -> Result<i32, HostError>;
    /// Migrates and verifies existing MongoDB data into the configured `store` before switching
    /// to the Rust backend (RU9/RB14). Defaults to [`migrate_existing_mongo_data`].
    fn migrate_mongo_data(&self, paths: &Paths, out: &mut dyn FnMut(&str)) -> Result<(), HostError> {
        migrate_existing_mongo_data(paths, out)
    }
}

pub struct System;

impl UpdateEnv for System {
    fn platform(&self) -> Result<&'static str, HostError> {
        selfupdate::this_platform()
    }

    fn source(&self, installed: Option<&Manifest>) -> Source {
        Source::from_env(installed)
    }

    fn verify_attestation(&self, archive: &Path, candidate: &Candidate, log_file: &Path) -> UpdateResult<Attestation> {
        selfupdate::verify_attestation(archive, candidate, log_file)
    }

    fn checkout_commit(&self, root: &Path) -> String {
        if !root.join(".git").exists() {
            return String::new();
        }
        crate::run::capture("git", &["rev-parse", "--short=12", "HEAD"], Some(root))
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    fn run_setup(&self, program: &Path, root: &Path) -> Result<i32, HostError> {
        let status = std::process::Command::new(program)
            .arg("setup")
            .current_dir(root)
            .status()
            .map_err(|e| HostError::io(program, e))?;
        Ok(status.code().unwrap_or(1))
    }
}

pub fn run(paths: &Paths, args: &UpdateArgs) -> Outcome {
    run_with(paths, args, &System, &mut |line| println!("{line}"))
}

/// The server's `reforged-host` once the release is installed.
pub fn release_program(root: &Path) -> PathBuf {
    root.join("bin")
        .join(format!("reforged-host{}", std::env::consts::EXE_SUFFIX))
}

fn up_to_date(installed: Option<&Manifest>, interrupted: bool, platform: &str, candidate: &Candidate) -> bool {
    let Some(installed) = installed else { return false };
    if interrupted || installed.platform != platform {
        return false;
    }
    match (
        selfupdate::Version::parse(&installed.version),
        selfupdate::Version::parse(&candidate.version),
    ) {
        (Some(have), Some(offered)) => offered <= have,
        _ => installed.version == candidate.version,
    }
}

/// How the server is described: its release, or the checkout's commit.
fn head(installed: Option<&Manifest>, commit: &str) -> String {
    match installed {
        Some(m) => m.version.clone(),
        None => commit.to_owned(),
    }
}

pub fn run_with(paths: &Paths, args: &UpdateArgs, env: &dyn UpdateEnv, out: &mut dyn FnMut(&str)) -> Outcome {
    match update(paths, args, env, out) {
        Ok(code) => Ok(code),
        Err(UpdateError::Problem(problem)) => {
            if args.json {
                out(&json!({ "error": problem.code(), "detail": problem.detail() }).to_string());
            }
            Err(HostError::message(problem.message()))
        }
        Err(UpdateError::Host(e)) => Err(e),
    }
}

fn update(paths: &Paths, args: &UpdateArgs, env: &dyn UpdateEnv, out: &mut dyn FnMut(&str)) -> UpdateResult<ExitCode> {
    let root = &paths.repo;
    let platform = env.platform()?;
    let installed = read_manifest(root)?;
    let interrupted = installing_marker(root).exists();
    let commit = if installed.is_none() {
        env.checkout_commit(root)
    } else {
        String::new()
    };
    let source = env.source(installed.as_ref());
    let current = installed.as_ref().map(|m| m.version.as_str());
    let candidate = find_release(&source, platform, Wanted::Newest(current))?;
    let fresh = up_to_date(installed.as_ref(), interrupted, platform, &candidate);
    let summary = |updated: Option<Value>| {
        let mut v = json!({
            "branch": "server release",
            "upstream": source.describe(),
            "head": head(installed.as_ref(), &commit),
            "remoteHead": candidate.version,
            "behind": if fresh { 0 } else { 1 },
            "ahead": 0,
            "platform": platform,
            "installed": installed.as_ref().map(|m| m.version.clone()),
            "available": candidate.version,
        });
        if let (Some(extra), Some(map)) = (updated, v.as_object_mut()) {
            for (k, val) in extra.as_object().into_iter().flatten() {
                map.insert(k.clone(), val.clone());
            }
        }
        v
    };
    let describe = || match (&installed, fresh) {
        (Some(m), true) => format!("Up to date: servers {} ({platform}).", m.version),
        (Some(m), false) if interrupted => format!(
            "The last update stopped before it was complete: servers {} ({platform}) install again.",
            candidate.version
        ),
        (Some(m), false) => format!(
            "Servers {} are out for {platform} ({} -> {}).",
            candidate.version, m.version, candidate.version
        ),
        (None, _) => format!(
            "This server runs from a git checkout{}: update moves it to the server release {} ({platform}).",
            if commit.is_empty() {
                String::new()
            } else {
                format!(" ({commit})")
            },
            candidate.version
        ),
    };

    if args.check {
        out(&describe());
        if args.json {
            out(&summary(None).to_string());
        }
        return Ok(ExitCode::SUCCESS);
    }

    out(&format!("Looking for the server release at {}...", source.describe()));
    if fresh {
        out(&describe());
        if args.json {
            let same = head(installed.as_ref(), &commit);
            out(&summary(Some(json!({ "updated": false, "from": same, "to": same, "files": 0, "runtimeChanged": false, "hostKitDeps": false }))).to_string());
        }
        return Ok(ExitCode::SUCCESS);
    }
    out(&describe());

    let work = paths.workspace.join("update");
    let log_file = paths.logs.join("update.log");
    out(&format!("- downloading {}", candidate.archive));
    let archive = selfupdate::fetch(&candidate, &work)?;
    out(&format!(
        "- {}: SHA-256 checked against {}",
        candidate.archive,
        selfupdate::SUMS_FILE
    ));
    match env.verify_attestation(&archive, &candidate, &log_file)? {
        Attestation::Verified => out("- build provenance: verified (gh attestation verify)"),
        Attestation::Skipped(why) => out(&format!("- build provenance: not checked ({why})")),
    }
    let staged = selfupdate::extract(&archive, &work.join("staging"))?;
    if let Err(e) = env.migrate_mongo_data(paths, out) {
        let _ = std::fs::remove_dir_all(work.join("staging"));
        let _ = std::fs::remove_file(&archive);
        return Err(e.into());
    }
    let (manifest, report) = selfupdate::install(&staged, root, installed.as_ref())?;
    let _ = std::fs::remove_dir_all(work.join("staging"));
    let _ = std::fs::remove_file(&archive);

    let from = report.from.clone().unwrap_or_else(|| head(None, &commit));
    out(&format!(
        "Updated to servers {}{} ({} file{} written{}).",
        report.to,
        if from.is_empty() {
            String::new()
        } else {
            format!(" from {from}")
        },
        report.files,
        if report.files == 1 { "" } else { "s" },
        if report.removed > 0 {
            format!(", {} removed", report.removed)
        } else {
            String::new()
        }
    ));
    match report.runtime_changed {
        Some(true) => out(
            "The game runtime changed: build MysticParadox.dll again and publish it (reforged-host publish-runtime) for the players.",
        ),
        None => out(
            "If the game runtime's source changed since your build, build MysticParadox.dll again and publish it (reforged-host publish-runtime).",
        ),
        Some(false) => {}
    }
    let program = release_program(root);
    if let Ok(me) = std::env::current_exe()
        && me.parent() != program.parent()
    {
        out(&format!(
            "From now on run {} ({} is not needed any more).",
            program.display(),
            me.display()
        ));
    }
    if args.json {
        out(&summary(Some(json!({
            "updated": true,
            "from": from,
            "to": manifest.version,
            "commit": manifest.short_commit(),
            "files": report.files,
            "removed": report.removed,
            "runtimeChanged": report.runtime_changed == Some(true),
            "hostKitDeps": false,
            "behind": 0,
            "head": manifest.version,
        })))
        .to_string());
    }
    if args.no_setup {
        return Ok(ExitCode::SUCCESS);
    }
    if !has_host_config(paths) {
        out("Now run reforged-host setup.");
        return Ok(ExitCode::SUCCESS);
    }
    out("\nRunning setup again with this server's settings (reforged-host setup)...");
    let code = env.run_setup(&program, root)?;
    if code != 0 {
        return Err(HostError::message(format!("setup failed: exit {code}")).into());
    }
    Ok(ExitCode::SUCCESS)
}

/// Target store (`SQLite` path or `postgres://` URL) and `allow_remote_postgres` for `isles-db`.
pub fn store_target_for_migration(
    paths: &Paths,
    config: &HostConfig,
) -> Result<(String, bool, Option<PathBuf>), HostError> {
    let d = config.database.clone().unwrap_or_default();
    if !d.postgres() {
        let sqlite_file = paths.config.join(DATABASE_FILE);
        return Ok((sqlite_file.to_string_lossy().into_owned(), false, Some(sqlite_file)));
    }
    let existing = read_env_file(&paths.config.join(SECRET_FILE))?;
    let password = existing.get("REFORGED_DB_PASSWORD").unwrap_or_default().to_owned();
    let host = d.postgres_host.unwrap_or_else(|| "127.0.0.1".into());
    let port = d.postgres_port.unwrap_or(5432);
    let user = d.postgres_user.unwrap_or_else(|| "dainty".into());
    let database = d.postgres_database.unwrap_or_else(|| "dainty".into());
    let allow_remote = d.allow_remote == Some(true);
    let url = if password.is_empty() {
        format!("postgres://{user}@{host}:{port}/{database}")
    } else {
        format!("postgres://{user}:{password}@{host}:{port}/{database}")
    };
    Ok((url, allow_remote, None))
}

/// Redacts any password in a `scheme://user:password@host...` URI so credentials are never printed or logged.
pub fn redact_uri_password(uri: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return uri.to_owned();
    };
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let Some((userinfo, hostport)) = authority.rsplit_once('@') else {
        return uri.to_owned();
    };
    let Some((user, _password)) = userinfo.split_once(':') else {
        return uri.to_owned();
    };
    format!("{scheme}://{user}:***@{hostport}{tail}")
}

/// Whether `.workspace/mongodb` exists and holds at least one entry.
pub fn has_managed_mongo_data(paths: &Paths) -> bool {
    std::fs::read_dir(&paths.mongo_data)
        .ok()
        .is_some_and(|mut entries| entries.next().is_some())
}

#[cfg(all(feature = "isles-1-14-7", not(feature = "paradox")))]
pub fn mongo_has_documents(client: &mut isles_db::mongo::MongoClient) -> bool {
    let primary = [
        "accounts",
        "characters",
        "inventories",
        "loadouts",
        "reforgedInvites",
        "wallets",
    ];
    for col in primary {
        if client.find_all(col).is_ok_and(|docs| !docs.is_empty()) {
            return true;
        }
    }
    let all_areas: std::collections::BTreeSet<isles_db::MigrateArea> = isles_db::MigrateArea::ALL.into_iter().collect();
    let empty_state = isles_db::migrate::TargetState::default();
    for col in isles_db::state_to_mongo_collections(&empty_state, &all_areas).into_keys() {
        if !primary.contains(&col.as_str()) && client.find_all(&col).is_ok_and(|docs| !docs.is_empty()) {
            return true;
        }
    }
    false
}

#[cfg(all(feature = "isles-1-14-7", not(feature = "paradox")))]
fn should_probe_mongo(paths: &Paths, config: &HostConfig, raw_uri: &str, has_managed_mongo_dir: bool) -> bool {
    if has_managed_mongo_dir || !config.mongo.manage || raw_uri != crate::hostconfig::DEFAULT_MONGO_URI {
        return true;
    }
    if !paths.host_json.exists() && !paths.host_toml.exists() && !paths.config.join("backend.env").exists() {
        return false;
    }
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], crate::paths::PORTS.mongo)),
        std::time::Duration::from_millis(50),
    )
    .is_ok()
}

#[cfg(all(feature = "isles-1-14-7", not(feature = "paradox")))]
fn remove_new_sqlite_files(sqlite_existed: bool, sqlite_path: Option<&Path>) {
    if !sqlite_existed && let Some(p) = sqlite_path {
        let _ = std::fs::remove_file(p);
        let base = p.to_string_lossy();
        let _ = std::fs::remove_file(format!("{base}-wal"));
        let _ = std::fs::remove_file(format!("{base}-shm"));
    }
}

#[cfg(all(feature = "isles-1-14-7", not(feature = "paradox")))]
pub fn migrate_existing_mongo_data(paths: &Paths, out: &mut dyn FnMut(&str)) -> Result<(), HostError> {
    let config = match load_host_config(paths) {
        Ok(Some(c)) => c,
        Ok(None) | Err(_) => return Ok(()),
    };
    migrate_existing_mongo_data_with_config(paths, &config, out)
}

#[cfg(all(feature = "isles-1-14-7", not(feature = "paradox")))]
pub fn migrate_existing_mongo_data_with_config(
    paths: &Paths,
    config: &HostConfig,
    out: &mut dyn FnMut(&str),
) -> Result<(), HostError> {
    if !config.edition.runs_reborn() {
        return Ok(());
    }
    let marker = paths.config.join(MONGO_MIGRATED_MARKER);
    if marker.exists() {
        return Ok(());
    }
    if config.fresh_start() {
        if let Some(parent) = marker.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&marker, "fresh start (opted out of MongoDB migration)\n");
        out(
            "- skipping MongoDB migration (--fresh-start / database.fresh_start): starting on a fresh database (MongoDB data kept intact)",
        );
        return Ok(());
    }
    let backend_env = read_env_file(&paths.config.join("backend.env"))?;
    let raw_uri = backend_env.get("MONGODB_URI").unwrap_or(&config.mongo.uri).trim();
    if raw_uri.is_empty() {
        return Ok(());
    }
    let from_uri = mongo_uri_with_database(raw_uri, backend_env.get("MONGODB_DB"));
    let display_uri = redact_uri_password(&from_uri);
    let has_managed_mongo_dir = has_managed_mongo_data(paths);
    if !should_probe_mongo(paths, config, raw_uri, has_managed_mongo_dir) {
        return Ok(());
    }
    let probe = isles_db::mongo::MongoClient::connect(&from_uri);
    let mut client = match probe {
        Ok(client) => client,
        Err(e) => {
            if has_managed_mongo_dir {
                return Err(HostError::message(format!(
                    "MongoDB data was found in {}, but MongoDB is not reachable at {display_uri} ({e}): start mongod and run reforged-host setup so your data can be migrated and verified before switching to the Rust backend, or run reforged-host setup --fresh-start (or set fresh_start = true under [database] in host.toml) to start on an empty database; the server was left on its current setup and MongoDB data was not modified",
                    paths.mongo_data.display()
                )));
            }
            return Ok(());
        }
    };
    let has_docs = has_managed_mongo_dir || mongo_has_documents(&mut client);
    if !has_docs {
        return Ok(());
    }

    let _ = std::fs::create_dir_all(&paths.config);
    let (to_target, allow_remote_postgres, sqlite_path) = store_target_for_migration(paths, config)?;
    let sqlite_existed = sqlite_path.as_ref().is_some_and(|p| p.exists());
    let node_str = config.network.as_ref().map(|n| n.node.as_str()).unwrap_or("local");
    let node = node_str
        .parse::<module_api::NodeId>()
        .or_else(|_| "local".parse::<module_api::NodeId>())
        .map_err(|e| HostError::message(format!("invalid node id: {e}")))?;
    let game_data_dir = paths
        .backend_game_data
        .is_dir()
        .then(|| paths.backend_game_data.clone());
    let opts = isles_db::MigrateOptions {
        from_uri: from_uri.clone(),
        to_target: to_target.clone(),
        node,
        dry_run: false,
        areas: Vec::new(),
        skip_invalid: false,
        game_data_dir,
        allow_remote_postgres,
    };

    out(&format!(
        "- migrating existing MongoDB data from {display_uri} into the configured store..."
    ));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| HostError::message(format!("could not start runtime for migration: {e}")))?;

    if let Err(e) = runtime.block_on(isles_db::run_migrate(&opts)) {
        remove_new_sqlite_files(sqlite_existed, sqlite_path.as_deref());
        return Err(HostError::message(format!(
            "MongoDB migration failed ({e}): refusing to switch to the Rust backend; the server stays on its previous setup and MongoDB data was not modified"
        )));
    }

    out("- verifying migrated store against MongoDB (isles-db verify)...");
    match runtime.block_on(isles_db::run_verify(&opts)) {
        Ok(report) if report.ok => {
            out(&format!(
                "- MongoDB migration verified ({} collections checked, 0 differences; MongoDB data kept intact)",
                report.collections.len()
            ));
            if let Some(parent) = marker.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&marker, format!("migrated from {display_uri}\n"));
            Ok(())
        }
        Ok(report) => {
            remove_new_sqlite_files(sqlite_existed, sqlite_path.as_deref());
            let failed: Vec<String> = report
                .collections
                .iter()
                .filter(|c| !c.ok)
                .map(|c| c.name.clone())
                .collect();
            Err(HostError::message(format!(
                "isles-db verify reported differences in [{}]: refusing to switch to the Rust backend; the server stays on its previous setup and MongoDB data was not modified",
                failed.join(", ")
            )))
        }
        Err(e) => {
            remove_new_sqlite_files(sqlite_existed, sqlite_path.as_deref());
            Err(HostError::message(format!(
                "isles-db verify reported a difference ({e}): refusing to switch to the Rust backend; the server stays on its previous setup and MongoDB data was not modified"
            )))
        }
    }
}

/// Refuses to start the Rust backend while unmigrated MongoDB data exists (M1).
#[cfg(all(feature = "isles-1-14-7", not(feature = "paradox")))]
pub fn check_unmigrated_mongo_data(paths: &Paths, config: &HostConfig) -> Result<(), HostError> {
    if !config.edition.runs_reborn() {
        return Ok(());
    }
    let marker = paths.config.join(MONGO_MIGRATED_MARKER);
    if marker.exists() {
        return Ok(());
    }
    if config.fresh_start() {
        if let Some(parent) = marker.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&marker, "fresh start (opted out of MongoDB migration)\n");
        return Ok(());
    }
    if has_managed_mongo_data(paths) {
        return Err(HostError::message(format!(
            "unmigrated MongoDB data was found in {}: run reforged-host setup (with mongod running) to migrate and verify your player data before starting the Rust backend, or run reforged-host setup --fresh-start (or set fresh_start = true under [database] in host.toml) to start on an empty database",
            paths.mongo_data.display()
        )));
    }
    let backend_env = read_env_file(&paths.config.join("backend.env"))?;
    let raw_uri = backend_env.get("MONGODB_URI").unwrap_or(&config.mongo.uri).trim();
    if raw_uri.is_empty() || !should_probe_mongo(paths, config, raw_uri, false) {
        return Ok(());
    }
    let from_uri = mongo_uri_with_database(raw_uri, backend_env.get("MONGODB_DB"));
    let display_uri = redact_uri_password(&from_uri);
    if let Ok(mut client) = isles_db::mongo::MongoClient::connect(&from_uri)
        && mongo_has_documents(&mut client)
    {
        return Err(HostError::message(format!(
            "unmigrated MongoDB data was found at {display_uri}: run reforged-host setup to migrate and verify your player data before starting the Rust backend, or run reforged-host setup --fresh-start (or set fresh_start = true under [database] in host.toml) to start on an empty database"
        )));
    }
    Ok(())
}

#[cfg(not(all(feature = "isles-1-14-7", not(feature = "paradox"))))]
pub fn migrate_existing_mongo_data(_paths: &Paths, _out: &mut dyn FnMut(&str)) -> Result<(), HostError> {
    Ok(())
}

#[cfg(not(all(feature = "isles-1-14-7", not(feature = "paradox"))))]
pub fn migrate_existing_mongo_data_with_config(
    _paths: &Paths,
    _config: &HostConfig,
    _out: &mut dyn FnMut(&str),
) -> Result<(), HostError> {
    Ok(())
}

#[cfg(not(all(feature = "isles-1-14-7", not(feature = "paradox"))))]
pub fn check_unmigrated_mongo_data(_paths: &Paths, _config: &HostConfig) -> Result<(), HostError> {
    Ok(())
}

/// Ensures `uri` names the database from `MONGODB_DB` (default `reforged`) when `uri` has no `/db` path.
pub fn mongo_uri_with_database(uri: &str, mongodb_db: Option<&str>) -> String {
    let db = mongodb_db
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("reforged");
    let Some(rest) = uri.strip_prefix("mongodb://") else {
        return uri.to_owned();
    };
    let (before_query, query) = match rest.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (rest, None),
    };
    if let Some((authority, path)) = before_query.split_once('/') {
        if !path.trim_matches('/').is_empty() {
            return uri.to_owned();
        }
        return match query {
            Some(q) => format!("mongodb://{authority}/{db}?{q}"),
            None => format!("mongodb://{authority}/{db}"),
        };
    }
    match query {
        Some(q) => format!("mongodb://{before_query}/{db}?{q}"),
        None => format!("mongodb://{before_query}/{db}"),
    }
}
