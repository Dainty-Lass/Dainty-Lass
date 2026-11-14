// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `dainty-host update`: a newer release found, checked and installed in place, then setup rerun; and the check that
//! stops a start or update while pre-cutover MongoDB data is not migrated.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::json;

use crate::cli::UpdateArgs;
use crate::error::HostError;
use crate::hostconfig::{HostConfig, load_host_config, node_database_config};
use crate::paths::Paths;
use crate::run::{RunOptions, run as run_cmd};
use crate::selfupdate::{
    Attestation, Candidate, Manifest, SUMS_FILE, Source, UpdateError, UpdateResult, Wanted, extract, fetch,
    find_release, install, installing_marker, read_manifest, this_platform, verify_attestation,
};

/// The marker file written by dainty-legacy's `import reforged` when it moves a Node kit's MongoDB into the store.
pub const MONGO_MIGRATED_MARKER: &str = ".mongo-migrated";

/// What `update` needs of the machine, so a test can stand in for it.
pub trait UpdateEnv {
    /// This machine's release platform; an error on one with no releases.
    fn platform(&self) -> Result<&'static str, HostError>;
    /// Where releases are looked for.
    fn source(&self, installed: Option<&Manifest>) -> Source;
    /// Checks the archive's build provenance, logging to `log`.
    fn verify_attestation(&self, archive: &Path, candidate: &Candidate, log: &Path) -> UpdateResult<Attestation>;
    /// The checkout's short commit, or `unknown`.
    fn checkout_commit(&self, root: &Path) -> String;
    /// Runs the new `program`'s `setup --skip-install` in `root`; returns its exit code.
    fn run_setup(&self, program: &Path, root: &Path) -> Result<i32, HostError>;
}

/// The real machine.
#[derive(Default)]
pub struct RealUpdateEnv;

impl UpdateEnv for RealUpdateEnv {
    fn platform(&self) -> Result<&'static str, HostError> {
        this_platform()
    }

    fn source(&self, installed: Option<&Manifest>) -> Source {
        Source::from_env(installed)
    }

    fn verify_attestation(&self, archive: &Path, candidate: &Candidate, log: &Path) -> UpdateResult<Attestation> {
        verify_attestation(archive, candidate, log)
    }

    fn checkout_commit(&self, root: &Path) -> String {
        crate::run::capture("git", &["rev-parse", "--short=12", "HEAD"], Some(root))
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".into())
    }

    fn run_setup(&self, program: &Path, root: &Path) -> Result<i32, HostError> {
        let res = run_cmd(
            &program.to_string_lossy(),
            &["setup", "--skip-install"],
            &RunOptions {
                cwd: root.to_path_buf(),
                env: None,
                log_file: None,
                quiet: false,
            },
        )?;
        Ok(res.code)
    }
}

/// Where the MongoDB-migrated marker is.
#[must_use]
pub fn mongo_migrated_marker(p: &Paths) -> PathBuf {
    p.config.join(MONGO_MIGRATED_MARKER)
}

/// `uri` with the password in its user info replaced, for printing.
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

/// `uri` with its password redacted, as [`redact_mongo_uri`] does.
#[must_use]
pub fn redact_uri_password(uri: &str) -> String {
    redact_mongo_uri(uri)
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

/// Whether this server's pre-cutover MongoDB data was moved into the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MongoMigrationStatus {
    /// There is no MongoDB data to move.
    None,
    /// It was moved.
    Migrated,
    /// The host chose to start on an empty store instead.
    Skipped,
    /// MongoDB data is there and was not moved.
    Unmigrated {
        /// The folder the data is in.
        source: String,
    },
}

/// Whether the marker records a fresh start rather than a migration.
#[must_use]
pub fn is_fresh_start_marker(marker: &Path) -> bool {
    fs::read_to_string(marker)
        .ok()
        .is_some_and(|s| s.starts_with("fresh-start"))
}

/// Records that the host chose to start on an empty store.
pub fn confirm_fresh_start(p: &Paths) -> Result<(), HostError> {
    let marker = mongo_migrated_marker(p);
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
    }
    fs::write(&marker, "fresh-start\n").map_err(|e| HostError::io(&marker, e))
}

/// The migration's state, from the marker, settings.toml and the managed MongoDB's data folder.
#[must_use]
pub fn inspect_mongo_migration(p: &Paths) -> MongoMigrationStatus {
    let marker = mongo_migrated_marker(p);
    if marker.is_file() {
        if is_fresh_start_marker(&marker) {
            return MongoMigrationStatus::Skipped;
        }
        return MongoMigrationStatus::Migrated;
    }
    if load_host_config(p).ok().flatten().is_some_and(|c| c.fresh_start()) {
        return MongoMigrationStatus::Skipped;
    }
    if has_unmigrated_mongo_dir(p) {
        return MongoMigrationStatus::Unmigrated {
            source: p.mongo_data.to_string_lossy().into_owned(),
        };
    }
    MongoMigrationStatus::None
}

/// Whether the managed MongoDB's data folder holds anything but its lock file.
#[must_use]
pub fn has_unmigrated_mongo_dir(p: &Paths) -> bool {
    let Ok(entries) = fs::read_dir(&p.mongo_data) else {
        return false;
    };
    entries.flatten().any(|e| e.file_name().to_str() != Some("mongod.lock"))
}

/// The error a start or update gives while MongoDB data is not migrated; `None` when there is nothing to move.
#[must_use]
pub fn unmigrated_mongo_problem(p: &Paths, config: &HostConfig) -> Option<String> {
    if cfg!(feature = "paradox") || config.fresh_start() || mongo_migrated_marker(p).is_file() {
        return None;
    }
    // An install already on settings.toml never meets dainty-legacy's host.json check, so it is caught here
    has_unmigrated_mongo_dir(p).then(|| {
        format!(
            "unmigrated MongoDB data was found in {}: move it with `dainty-host db migrate` and `db verify` (or `dainty-legacy import reforged <checkout>`), or pass --fresh-start to setup to start on a fresh database",
            p.mongo_data.display()
        )
    })
}

/// Fails with [`unmigrated_mongo_problem`]'s text when there is one.
pub fn check_unmigrated_mongo_data(p: &Paths, config: &HostConfig) -> Result<(), HostError> {
    match unmigrated_mongo_problem(p, config) {
        Some(problem) => Err(HostError::message(problem)),
        None => Ok(()),
    }
}

/// Runs `dainty-host update`, printing to stdout; an error is printed and gives a failing exit code.
#[must_use]
pub fn run(paths: &Paths, args: &UpdateArgs) -> ExitCode {
    let env = RealUpdateEnv;
    match run_with(paths, args, &env, &mut |line| println!("{line}")) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Finds the release, downloads and checks it, installs it and reruns setup unless `--no-setup`, printing through
/// `out`.
/// With `--check` it only says whether a newer release is out.
pub fn run_with(
    p: &Paths,
    args: &UpdateArgs,
    env: &dyn UpdateEnv,
    out: &mut dyn FnMut(&str),
) -> Result<ExitCode, HostError> {
    let installed = read_manifest(&p.repo)?;
    let interrupted = installing_marker(&p.repo).is_file();
    let platform = env.platform()?;
    let source = env.source(installed.as_ref());
    let installed_ver = if interrupted {
        None
    } else {
        installed.as_ref().map(|m| m.version.as_str())
    };

    let map_update_err = |err: UpdateError, staging: Option<&Path>, out: &mut dyn FnMut(&str)| -> HostError {
        if let Some(s) = staging {
            let _ = fs::remove_dir_all(s);
        }
        match err {
            UpdateError::Problem(prob) => {
                if args.json {
                    let line = json!({
                        "updated": false,
                        "error": prob.code(),
                        "message": prob.message(),
                    });
                    out(&line.to_string());
                }
                HostError::message(prob.message())
            }
            UpdateError::Host(he) => he,
        }
    };

    let candidate = match find_release(&source, platform, Wanted::Newest(installed_ver)) {
        Ok(c) => c,
        Err(e) => return Err(map_update_err(e, None, out)),
    };

    if args.check {
        if interrupted {
            out(&format!(
                "The last update stopped before it was complete: run dainty-host update to finish installing servers {} ({platform}).",
                candidate.version
            ));
            if args.json {
                out(&json!({
                    "behind": 1,
                    "head": installed.as_ref().map(|m| m.version.clone()),
                    "target": candidate.version,
                })
                .to_string());
            }
            return Ok(ExitCode::SUCCESS);
        }
        if let Some(current) = &installed {
            let up_to_date = candidate.version == current.version;
            if up_to_date {
                out(&format!("Up to date: servers {} ({platform}).", current.version));
            } else {
                out(&format!(
                    "Update available: servers {} -> {} ({platform}).",
                    current.version, candidate.version
                ));
            }
            if args.json {
                out(&json!({
                    "behind": usize::from(!up_to_date),
                    "head": current.version,
                    "target": candidate.version,
                })
                .to_string());
            }
            return Ok(ExitCode::SUCCESS);
        }
        let commit = env.checkout_commit(&p.repo);
        out(&format!(
            "This server runs from a git checkout ({commit}): update moves it to the server release {} ({platform}).",
            candidate.version
        ));
        if args.json {
            out(&json!({
                "behind": 1,
                "head": commit,
                "target": candidate.version,
            })
            .to_string());
        }
        return Ok(ExitCode::SUCCESS);
    }

    if !interrupted
        && let Some(current) = &installed
        && candidate.version == current.version
    {
        out(&format!("Up to date: servers {} ({platform}).", current.version));
        if args.json {
            out(&json!({
                "updated": false,
                "to": current.version,
            })
            .to_string());
        }
        return Ok(ExitCode::SUCCESS);
    }

    let update_dir = p.workspace.join("update");
    let staging_dir = update_dir.join("staging");
    let _ = fs::remove_dir_all(&staging_dir);
    fs::create_dir_all(&update_dir).map_err(|e| HostError::io(&update_dir, e))?;
    let log_file = p.logs.join("update.log");

    let archive = match fetch(&candidate, &update_dir) {
        Ok(a) => a,
        Err(e) => return Err(map_update_err(e, Some(&staging_dir), out)),
    };
    if let Err(e) = env.verify_attestation(&archive, &candidate, &log_file) {
        let _ = fs::remove_file(&archive);
        let _ = fs::remove_file(update_dir.join(SUMS_FILE));
        return Err(map_update_err(e, Some(&staging_dir), out));
    }
    let staged = match extract(&archive, &staging_dir) {
        Ok(s) => s,
        Err(e) => {
            let _ = fs::remove_file(&archive);
            let _ = fs::remove_file(update_dir.join(SUMS_FILE));
            return Err(map_update_err(e, Some(&staging_dir), out));
        }
    };

    let prev_for_install = if interrupted { None } else { installed.as_ref() };
    let (_manifest, info) = match install(&staged, &p.repo, prev_for_install) {
        Ok(r) => r,
        Err(e) => {
            let _ = fs::remove_file(&archive);
            let _ = fs::remove_file(update_dir.join(SUMS_FILE));
            return Err(map_update_err(e, Some(&staging_dir), out));
        }
    };
    let _ = fs::remove_file(&archive);
    let _ = fs::remove_file(update_dir.join(SUMS_FILE));
    let _ = fs::remove_dir_all(&staging_dir);

    if let Some(from) = &info.from {
        out(&format!(
            "Updated to servers {} from {from} ({} files written, {} removed).",
            info.to, info.files, info.removed
        ));
    } else {
        out(&format!(
            "Updated to servers {} ({} files written).",
            info.to, info.files
        ));
    }
    if info.runtime_changed == Some(true) {
        out("The game runtime changed: players should update their game patch before joining.");
    }
    if args.json {
        out(&json!({
            "updated": true,
            "from": info.from,
            "to": info.to,
            "files": info.files,
            "removed": info.removed,
            "runtimeChanged": info.runtime_changed,
        })
        .to_string());
    }

    if !args.no_setup {
        let configured = p.settings_toml.is_file();
        if configured {
            let host_bin = p
                .repo
                .join("bin")
                .join(format!("dainty-host{}", std::env::consts::EXE_SUFFIX));
            let code = env.run_setup(&host_bin, &p.repo)?;
            if code != 0 {
                return Err(HostError::message(format!("dainty-host setup exited with code {code}")));
            }
        } else {
            out("Now run dainty-host setup.");
        }
    }

    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn check_unmigrated_mongo_data_passes_when_empty_or_marked() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        let cfg = HostConfig::from_json(&json!({
            "mode": "private",
            "host": "100.64.0.1",
            "gameAddress": "100.64.0.1",
            "name": "Test",
            "registration": "INVITECODE",
            "maxHuntServers": 4
        }))
        .unwrap();
        assert!(check_unmigrated_mongo_data(&paths, &cfg).is_ok());

        fs::create_dir_all(&paths.mongo_data).unwrap();
        fs::write(paths.mongo_data.join("WiredTiger"), b"data").unwrap();
        if !cfg!(feature = "paradox") {
            let err = check_unmigrated_mongo_data(&paths, &cfg).unwrap_err().to_string();
            assert!(err.contains("dainty-legacy import reforged"), "{err}");
        }

        let marker = mongo_migrated_marker(&paths);
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(&marker, "migrated from mongodb://127.0.0.1:27017/reforged\n").unwrap();
        assert!(check_unmigrated_mongo_data(&paths, &cfg).is_ok());
    }
}
