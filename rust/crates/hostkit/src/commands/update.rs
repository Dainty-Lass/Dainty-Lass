// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::json;

use crate::cli::{SetupArgs, UpdateArgs};
use crate::error::HostError;
use crate::hostconfig::{
    DEFAULT_MONGO_URI, HostConfig, load_host_config, node_database_config, stringify_pretty,
};
use crate::paths::Paths;
use crate::run::{RunOptions, run as run_cmd};
use crate::selfupdate::{
    Source, Wanted, extract, fetch, find_release, install, read_manifest, this_platform,
    verify_attestation,
};

pub const MONGO_MIGRATED_MARKER: &str = ".mongo-migrated";

pub const MONGO_COLLECTIONS: [&str; 15] = [
    "accounts",
    "characters",
    "inventories",
    "loadouts",
    "progression",
    "quests",
    "masteries",
    "huntpasses",
    "guilds",
    "friends",
    "mails",
    "leaderboards",
    "sanctions",
    "invites",
    "sessions",
];

pub trait UpdateEnv {
    fn say(&mut self, line: &str);
    fn err(&mut self, line: &str);
    fn platform(&self) -> Result<&'static str, HostError>;
    fn source(&self, installed: Option<&crate::selfupdate::Manifest>) -> Source;
    fn run_setup(&mut self, p: &Paths, skip_install: bool) -> Result<(), HostError>;
}

#[derive(Default)]
pub struct RealUpdateEnv;

impl UpdateEnv for RealUpdateEnv {
    fn say(&mut self, line: &str) {
        println!("{line}");
    }

    fn err(&mut self, line: &str) {
        eprintln!("{line}");
    }

    fn platform(&self) -> Result<&'static str, HostError> {
        this_platform()
    }

    fn source(&self, installed: Option<&crate::selfupdate::Manifest>) -> Source {
        Source::from_env(installed)
    }

    fn run_setup(&mut self, p: &Paths, skip_install: bool) -> Result<(), HostError> {
        let mut setup_env = crate::commands::setup::RealSetupEnv::default();
        crate::commands::setup::run_with(
            p,
            &SetupArgs {
                skip_install,
                ..SetupArgs::default()
            },
            &mut setup_env,
        )
    }
}

#[must_use]
pub fn mongo_migrated_marker(p: &Paths) -> PathBuf {
    p.workspace.join("data").join(MONGO_MIGRATED_MARKER)
}

#[must_use]
pub fn has_unmigrated_mongo_dir(p: &Paths) -> bool {
    if mongo_migrated_marker(p).is_file() {
        return false;
    }
    let Ok(entries) = fs::read_dir(&p.mongo_data) else {
        return false;
    };
    entries
        .flatten()
        .any(|e| e.file_name().to_str() != Some("mongod.lock"))
}

#[must_use]
pub fn has_external_mongo_evidence(p: &Paths, config: &HostConfig) -> bool {
    if mongo_migrated_marker(p).is_file() || config.fresh_start() {
        return false;
    }
    let has_prior_artifacts =
        p.config.join("backend.env").is_file() || p.logs.join("mongo.log").is_file();
    if !config.mongo.manage && config.mongo.uri != DEFAULT_MONGO_URI {
        let is_local_default_host = config.mongo.uri.starts_with("mongodb://127.0.0.1:27017/")
            || config.mongo.uri.starts_with("mongodb://localhost:27017/");
        if !is_local_default_host || has_prior_artifacts {
            return true;
        }
    }
    false
}

#[must_use]
pub fn unmigrated_mongo_doctor_problem(p: &Paths, config: &HostConfig) -> Option<(String, String)> {
    if cfg!(feature = "paradox") || config.fresh_start() || mongo_migrated_marker(p).is_file() {
        return None;
    }
    if has_unmigrated_mongo_dir(p) {
        return Some((
            format!(
                "unmigrated MongoDB data found in {}",
                p.mongo_data.display()
            ),
            "run reforged-host update (or reforged-host db migrate), or set [database] fresh_start = true in host.toml".to_owned(),
        ));
    }
    if has_external_mongo_evidence(p, config) {
        return Some((
            format!(
                "unmigrated external MongoDB configured ({})",
                config.mongo.uri
            ),
            "run reforged-host db migrate, or set [database] fresh_start = true in host.toml".to_owned(),
        ));
    }
    None
}

pub fn check_unmigrated_mongo_before_start(
    p: &Paths,
    config: &HostConfig,
) -> Result<(), HostError> {
    if let Some((problem, fix)) = unmigrated_mongo_doctor_problem(p, config) {
        return Err(HostError::message(format!("{problem}: {fix}")));
    }
    Ok(())
}

pub fn check_unmigrated_mongo_data(p: &Paths, config: &HostConfig) -> Result<(), HostError> {
    check_unmigrated_mongo_before_start(p, config)
}

pub fn migrate_existing_mongo_data(
    p: &Paths,
    config: &HostConfig,
    _log_file: &Path,
    say: &mut dyn FnMut(&str),
) -> Result<bool, HostError> {
    if cfg!(feature = "paradox") || config.fresh_start() || mongo_migrated_marker(p).is_file() {
        return Ok(false);
    }
    if !has_unmigrated_mongo_dir(p) && !has_external_mongo_evidence(p, config) {
        return Ok(false);
    }
    let db_cfg = node_database_config(config, p)?;
    say("Migrating existing MongoDB data to store database...");
    let (counts, mismatches) =
        crate::hooks::get().db_migrate(&config.mongo.uri, &db_cfg, false)?;
    if !mismatches.is_empty() {
        return Err(HostError::message(format!(
            "MongoDB migration verification found {} mismatch(es)",
            mismatches.len()
        )));
    }
    let marker = mongo_migrated_marker(p);
    if let Some(parent) = marker.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&marker, format!("migrated from {}\n", config.mongo.uri));
    let total: usize = counts.iter().map(|(_, n)| *n).sum();
    say(&format!("Migrated {total} record(s) from MongoDB."));
    Ok(total > 0)
}

#[must_use]
pub fn run(paths: &Paths, args: &UpdateArgs) -> ExitCode {
    let mut env = RealUpdateEnv;
    match run_with(paths, args, &mut env) {
        Ok(code) => code,
        Err(err) => {
            if args.json {
                let out = json!({
                    "ok": false,
                    "error": err.to_string(),
                });
                env.say(&stringify_pretty(&out));
            } else {
                env.err(&format!("Update failed: {err}"));
            }
            ExitCode::FAILURE
        }
    }
}

pub fn run_with(
    p: &Paths,
    args: &UpdateArgs,
    env: &mut dyn UpdateEnv,
) -> Result<ExitCode, HostError> {
    let installed = read_manifest(&p.repo)?;
    let Some(current) = installed else {
        return update_git_checkout(p, args, env);
    };

    let platform = env.platform()?;
    let source = env.source(Some(&current));
    let candidate = find_release(&source, platform, Wanted::Newest(Some(&current.version)))
        .map_err(|e| HostError::message(e.to_string()))?;

    let up_to_date = candidate.version == current.version;
    if args.check || up_to_date {
        if args.json {
            let out = json!({
                "ok": true,
                "current": current.version,
                "latest": candidate.version,
                "updateAvailable": !up_to_date,
            });
            env.say(&stringify_pretty(&out));
        } else if up_to_date {
            env.say(&format!("Server is already up to date (v{}).", current.version));
        } else {
            env.say(&format!(
                "Update available: v{} -> v{} (from {}).",
                current.version,
                candidate.version,
                source.describe()
            ));
        }
        return Ok(ExitCode::SUCCESS);
    }

    let tmp_dir = p.workspace.join("update").join("staging");
    fs::create_dir_all(&tmp_dir).map_err(|e| HostError::io(&tmp_dir, e))?;
    let download_dir = tmp_dir.join("download");
    let extract_dir = tmp_dir.join("extracted");
    let log_file = p.logs.join("update.log");

    let archive = fetch(&candidate, &download_dir).map_err(|e| HostError::message(e.to_string()))?;
    let _ = verify_attestation(&archive, &candidate, &log_file)
        .map_err(|e| HostError::message(e.to_string()))?;
    let staged = extract(&archive, &extract_dir).map_err(|e| HostError::message(e.to_string()))?;
    let (_manifest, info) =
        install(&staged, &p.repo, Some(&current)).map_err(|e| HostError::message(e.to_string()))?;
    let _ = fs::remove_dir_all(&tmp_dir);

    if !args.no_setup && p.host_toml.is_file() {
        let _ = env.run_setup(p, true);
    }

    if let Ok(Some(cfg)) = load_host_config(p) {
        let mut say_fn = |line: &str| {
            if !args.json {
                env.say(line);
            }
        };
        let _ = migrate_existing_mongo_data(p, &cfg, &log_file, &mut say_fn);
    }

    if args.json {
        let out = json!({
            "ok": true,
            "from": info.from,
            "to": info.to,
            "files": info.files,
            "removed": info.removed,
            "runtimeChanged": info.runtime_changed,
        });
        env.say(&stringify_pretty(&out));
    } else {
        env.say(&format!("Updated server to v{} ({} files).", info.to, info.files));
    }

    Ok(ExitCode::SUCCESS)
}

fn update_git_checkout(
    p: &Paths,
    args: &UpdateArgs,
    env: &mut dyn UpdateEnv,
) -> Result<ExitCode, HostError> {
    let log_file = p.logs.join("update.log");
    let opts = RunOptions {
        cwd: p.repo.clone(),
        env: None,
        log_file: Some(log_file.clone()),
        quiet: args.json,
    };
    if args.check {
        let _ = run_cmd("git", &["fetch", "--quiet"], &opts)?;
        let status = crate::run::capture(
            "git",
            &["status", "-uno", "--porcelain=v2", "--branch"],
            Some(&p.repo),
        )?
        .unwrap_or_default();
        if args.json {
            let out = json!({
                "ok": true,
                "mode": "git",
                "status": status,
            });
            env.say(&stringify_pretty(&out));
        } else {
            env.say(&format!("Git checkout status:\n{status}"));
        }
        return Ok(ExitCode::SUCCESS);
    }

    let res = run_cmd("git", &["pull", "--ff-only"], &opts)?;
    if res.code != 0 {
        return Err(HostError::message(format!(
            "git pull --ff-only failed ({})",
            res.last_line.unwrap_or_else(|| format!("exit {}", res.code))
        )));
    }

    if !args.no_setup && p.host_toml.is_file() {
        let _ = env.run_setup(p, false);
    }

    if let Ok(Some(cfg)) = load_host_config(p) {
        let mut say_fn = |line: &str| {
            if !args.json {
                env.say(line);
            }
        };
        let _ = migrate_existing_mongo_data(p, &cfg, &log_file, &mut say_fn);
    }

    if args.json {
        let out = json!({
            "ok": true,
            "mode": "git",
        });
        env.say(&stringify_pretty(&out));
    } else {
        env.say(&format!("Updated git checkout in {}.", p.repo.display()));
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn check_unmigrated_mongo_data_passes_when_empty_or_fresh_start() {
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
        assert!(check_unmigrated_mongo_before_start(&paths, &cfg).is_ok());

        // Add non-lock file in mongo_data -> fails unless marker or fresh_start
        fs::create_dir_all(&paths.mongo_data).unwrap();
        fs::write(paths.mongo_data.join("WiredTiger"), b"data").unwrap();
        if !cfg!(feature = "paradox") {
            assert!(check_unmigrated_mongo_before_start(&paths, &cfg).is_err());
        }

        // Marker clears the error
        let marker = mongo_migrated_marker(&paths);
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(&marker, "done\n").unwrap();
        assert!(check_unmigrated_mongo_before_start(&paths, &cfg).is_ok());
    }
}
