// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `dainty-host start`'s pieces: the checks before the server starts, the database, the plan of front services
//! and their supervised start, and the stop.

pub mod binaries;
pub mod control;

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::Outcome;
use crate::doctor::{
    Level, REBORN_BUILD, game_exe_checks, linux_checks, network_checks, port_in_use as raw_port_in_use,
    port_owner_hint, store_database_check, unfinished_install_check,
};
use crate::error::HostError;
use crate::gamedata::{BACKEND_GAME_DATA, DIRECTOR_GAME_DATA, GameDataKind, game_data_state, seed_embedded_examples};
use crate::hostconfig::{HostConfig, Platform, ServerMode, gateway_port, linux_options, load_host_config};
use crate::linux::{UNIT_ALLOWLIST, ensure_wine_prefix, systemd_installed, write_gameserver_wrapper};
use crate::mongo::{MongoCheck, ReplicaSetOptions, ensure_replica_set, find_mongod, mongod_args};
use crate::paths::{PORTS, Paths, game_exe_relative};
use crate::preset::apply_preset;
use crate::publish::{ensure_server_flags, retire_server_loader};
use crate::services::{Printer, Ready, ServiceSpec, Supervisor, check_ready};
use std::sync::Arc;

use self::binaries::{FrontService, Launch, Programs};
use self::control::Control;

/// Prints each service's output line as `name | line` on stdout.
#[must_use]
pub fn console_printer() -> Printer {
    Arc::new(|name: &str, line: &str| {
        println!("{name:<8} | {line}");
    })
}

/// One service to start, and how long it may take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// What to run.
    pub spec: ServiceSpec,
    /// How long it may take to be ready.
    pub ready_timeout: Duration,
    /// A start that is not ready in time stops the server; otherwise it runs on without it.
    pub required: bool,
}

/// [`Planned`] under the name the plan's callers use.
pub type PlannedService = Planned;

/// Where `start` writes what it says.
pub trait StartEnv {
    /// Writes one line.
    fn say(&mut self, line: &str);
}

/// Prints `line` on stdout.
pub fn say(line: &str) {
    println!("{line}");
}

/// The service's env file, `<name>.env` in the config folder.
#[must_use]
pub fn env_file(paths: &Paths, name: &str) -> PathBuf {
    paths.config.join(format!("{name}.env"))
}

/// Fails, naming them, when any of the services' env files is missing.
pub fn require_env_files(paths: &Paths, names: &[&str]) -> Result<(), HostError> {
    let missing: Vec<String> = names
        .iter()
        .filter(|n| !env_file(paths, n).is_file())
        .map(|n| format!("{n}.env"))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(HostError::message(format!(
            "missing {} in {}: run dainty-host setup first",
            missing.join(", "),
            paths.config.display()
        )))
    }
}

/// The spec that runs `launch` as `name`, given its env file unless `launch` passes it as an argument.
#[must_use]
pub fn spec_from(name: &str, launch: Launch, env_file_path: PathBuf, ready: Ready) -> ServiceSpec {
    ServiceSpec {
        name: name.to_owned(),
        cwd: launch.cwd,
        command: launch.command,
        args: launch.args,
        env_file: (!launch.env_file_in_args).then_some(env_file_path),
        extra_env: Vec::new(),
        ready: Some(ready),
    }
}

/// Whether something already answers on the service's port.
pub fn port_in_use(ready: &Ready) -> Result<bool, HostError> {
    Ok(check_ready(ready) || raw_port_in_use(ready.port(), Some(ready.host()))?)
}

/// The workspace's settings.toml; an error says to run setup, or to migrate the MongoDB data first.
pub fn load_config(paths: &Paths) -> Result<HostConfig, HostError> {
    if let Some(cfg) = load_host_config(paths)? {
        return Ok(cfg);
    }
    if !cfg!(feature = "paradox")
        && !crate::commands::update::mongo_migrated_marker(paths).is_file()
        && crate::commands::update::has_unmigrated_mongo_dir(paths)
    {
        return Err(HostError::message(format!(
            "unmigrated MongoDB data was found in {}: move it with `dainty-host db migrate` and `db verify` (or `dainty-legacy import reforged <checkout>`), or pass --fresh-start to setup to start on a fresh database",
            paths.mongo_data.display()
        )));
    }
    Err(HostError::message(
        "server is not configured yet: run dainty-host setup first",
    ))
}

/// The line printed once the server is up.
#[must_use]
pub fn up_line(config: &HostConfig) -> String {
    let port = gateway_port(config);
    format!(
        "Server {:?} is up ({} mode, gateway {}:{}).",
        config.name,
        config.mode.as_str(),
        config.host,
        port
    )
}

/// The services to start, in order: the allowlist on a public server (unless it runs as its own unit), the Node
/// backend and Director in a `paradox` build, the content server, then the gateway.
pub fn service_plan(
    p: &Paths,
    config: &HostConfig,
    programs: &Programs,
    external_allowlist: bool,
) -> Result<Vec<PlannedService>, HostError> {
    let own = crate::hostconfig::service_ports(config);
    let mut plan = Vec::new();
    let public = config.mode == ServerMode::Public;

    if public && !external_allowlist {
        let file = env_file(p, FrontService::Allowlist.bin_name());
        let launch = programs.launch(FrontService::Allowlist, p, &file);
        plan.push(Planned {
            spec: spec_from(
                FrontService::Allowlist.bin_name(),
                launch,
                file,
                Ready::http(own.allowlist, "/status"),
            ),
            ready_timeout: Duration::from_secs(60),
            required: true,
        });
    }

    if cfg!(feature = "paradox") {
        plan.push(Planned {
            spec: ServiceSpec {
                name: "backend".into(),
                cwd: p.backend_dir.clone(),
                command: programs.node.clone(),
                args: vec![Path::new("build").join("server.js").display().to_string()],
                env_file: Some(env_file(p, "backend")),
                extra_env: Vec::new(),
                ready: Some(Ready::http(own.backend, "/dauntless-status")),
            },
            ready_timeout: Duration::from_secs(120),
            required: true,
        });
        plan.push(Planned {
            spec: ServiceSpec {
                name: "director".into(),
                cwd: p.director_dir.clone(),
                command: programs.node.clone(),
                args: vec![Path::new("dist").join("server.js").display().to_string()],
                env_file: Some(env_file(p, "director")),
                extra_env: Vec::new(),
                ready: Some(Ready::http(own.director, "/api/instances")),
            },
            ready_timeout: Duration::from_secs(60),
            required: false,
        });
    }

    let content_file = env_file(p, FrontService::Content.bin_name());
    let content_launch = programs.launch(FrontService::Content, p, &content_file);
    plan.push(Planned {
        spec: spec_from(
            FrontService::Content.bin_name(),
            content_launch,
            content_file,
            Ready::http(own.content, "/content/v1/news"),
        ),
        ready_timeout: Duration::from_secs(60),
        required: false,
    });

    let gw_ready = match config.mode {
        ServerMode::Public => Ready::tcp("127.0.0.1", gateway_port(config)),
        ServerMode::Private => Ready::tcp(&config.game_address, crate::hostconfig::gateway_port(config)),
    };
    let gw_file = env_file(p, FrontService::Gateway.bin_name());
    let gw_launch = programs.launch(FrontService::Gateway, p, &gw_file);
    plan.push(Planned {
        spec: spec_from(FrontService::Gateway.bin_name(), gw_launch, gw_file, gw_ready),
        ready_timeout: Duration::from_secs(60),
        required: true,
    });

    Ok(plan)
}

/// Prepares the database before the services start; `false` stops the start.
pub type PrepareDbFn<'a> = &'a dyn Fn(&Supervisor) -> Result<bool, HostError>;
/// Whether something already holds a service's port.
pub type PortTakenFn<'a> = &'a dyn Fn(&Ready) -> Result<bool, HostError>;

/// Prepares the database, then starts each planned service in order; `false` when a stop was asked meanwhile.
/// Fails when a port is taken or a required service is not ready in time.
pub fn start_services(
    supervisor: &Supervisor,
    control: &Control,
    plan: Vec<PlannedService>,
    prepare_db: Option<PrepareDbFn<'_>>,
    port_taken: PortTakenFn<'_>,
) -> Result<bool, HostError> {
    if let Some(prep) = prepare_db
        && !prep(supervisor)?
    {
        return Ok(false);
    }
    for item in plan {
        if control.asked() {
            return Ok(false);
        }
        if let Some(ready) = &item.spec.ready
            && port_taken(ready)?
        {
            return Err(HostError::message(format!(
                "port {} ({}) is already in use",
                ready.port(),
                item.spec.name
            )));
        }
        let ok = supervisor.start(item.spec.clone(), item.ready_timeout)?;
        if !ok && item.required {
            if control.asked() {
                return Ok(false);
            }
            return Err(HostError::message(format!(
                "{} did not start within {}s",
                item.spec.name,
                item.ready_timeout.as_secs()
            )));
        }
    }
    Ok(!control.asked())
}

/// Prints that the server is up and waits for a stop when `started`, then stops every service and runs `after_stop`.
pub fn finish(
    started: bool,
    supervisor: &Supervisor,
    control: &Control,
    controlled: bool,
    up: &str,
    after_stop: impl FnOnce(),
) -> Outcome {
    if started && !control.asked() {
        println!("{up}");
        if controlled {
            println!("{}", control::SERVER_READY_LINE);
        } else {
            println!("Ctrl+C stops everything.");
        }
        let _ = std::io::Write::flush(&mut std::io::stdout());
        control.wait();
    }
    println!("Stopping...");
    supervisor.stop_all();
    after_stop();
    println!("Stopped.");
    Ok(std::process::ExitCode::SUCCESS)
}

/// The doctor's network and Linux checks; fails on the first failing one.
pub fn host_checks(paths: &Paths, config: &HostConfig, external_allowlist: bool) -> Result<(), HostError> {
    let platform = Platform::current();
    let mut checks = network_checks(paths, config, platform)?;
    if platform == Platform::Linux {
        let mut lchecks = linux_checks(paths, config)?;
        if external_allowlist {
            lchecks.retain(|c| !c.text.contains("nftables"));
        }
        checks.extend(lchecks);
    }

    let gw_port = gateway_port(config);
    if raw_port_in_use(gw_port, Some(&config.host))? {
        let owner = port_owner_hint(gw_port).map(|o| format!(" by {o}")).unwrap_or_default();
        return Err(HostError::message(format!(
            "gateway port {gw_port} is already in use{owner}; stop that program and start again"
        )));
    }

    let failed: Vec<String> = checks
        .into_iter()
        .filter(|c| c.level == Level::Fail)
        .map(|c| match c.fix {
            Some(fix) => format!("{} ({fix})", c.text),
            None => c.text,
        })
        .collect();

    if !failed.is_empty() {
        return Err(HostError::message(format!(
            "preflight check failed:\n  - {}",
            failed.join("\n  - ")
        )));
    }
    Ok(())
}

/// [`preflight_with`] with the allowlist run by `start`.
pub fn preflight(paths: &Paths, config: &HostConfig) -> Result<(), HostError> {
    preflight_with(paths, config, false)
}

/// What runs before the services: the MongoDB data check, the host checks, the preset and game data,
/// the game folder with its runtime, and the store check. Fails on what would stop the server from running.
pub fn preflight_with(paths: &Paths, config: &HostConfig, external_allowlist: bool) -> Result<(), HostError> {
    if !cfg!(feature = "paradox") {
        crate::commands::update::check_unmigrated_mongo_data(paths, config)?;
    }
    host_checks(paths, config, external_allowlist)?;

    let _ = apply_preset(paths, false)?;
    let backend_state = game_data_state(&paths.backend_game_data, &BACKEND_GAME_DATA)?;
    let director_state = game_data_state(&paths.director_game_data, &DIRECTOR_GAME_DATA)?;
    if backend_state.state == GameDataKind::Missing || director_state.state == GameDataKind::Missing {
        let _ = seed_embedded_examples(paths)?;
    }

    if let Some(ref game_dir) = config.game_dir {
        let mut gchecks = game_exe_checks(Some(game_dir), false, &REBORN_BUILD)?;
        let epic_data = crate::epicinstall::Files::of(paths).data;
        gchecks.extend(unfinished_install_check(&epic_data, Path::new(game_dir), &REBORN_BUILD));
        if let Some(fail) = gchecks.iter().find(|c| c.level == Level::Fail) {
            return Err(HostError::message(fail.text.clone()));
        }
        let exe = Path::new(game_dir).join(game_exe_relative());
        if let Some(win64_dir) = exe.parent().filter(|d| d.is_dir()) {
            let _ = retire_server_loader(paths, win64_dir);
            let _ = ensure_server_flags(win64_dir);
        }
    }

    if !cfg!(feature = "paradox") {
        let db_check = store_database_check(paths, config);
        if db_check.level == Level::Fail {
            return Err(HostError::message(db_check.text));
        }
    }

    let _ = crate::hooks::get().install_director_runner(paths);
    Ok(())
}

/// Brings up the managed MongoDB's replica set, starting mongod under `supervisor` when it does not answer.
/// `false` when `cancel` stopped it.
pub fn prepare_database(
    paths: &Paths,
    config: &HostConfig,
    supervisor: &Supervisor,
    cancel: &dyn Fn() -> bool,
) -> Result<bool, HostError> {
    if cancel() {
        return Ok(false);
    }

    let initial = ensure_replica_set(
        &config.mongo.uri,
        &ReplicaSetOptions {
            initiate: true,
            host: None,
            timeout_ms: Some(2000),
        },
    )?;
    if matches!(initial, MongoCheck::Ok { primary: true, .. }) {
        return Ok(true);
    }

    if !config.mongo.manage {
        return match initial {
            MongoCheck::Ok { primary: false, .. } => {
                Err(HostError::message("external MongoDB replica set is not primary yet"))
            }
            MongoCheck::Failed { detail, .. } => Err(HostError::message(format!(
                "could not connect to external MongoDB ({}): {detail}",
                config.mongo.uri
            ))),
            MongoCheck::Ok { primary: true, .. } => Ok(true),
        };
    }

    let mongod_bin = find_mongod(config.mongo.mongod.as_deref())?.ok_or_else(|| {
        HostError::message("mongod was not found: install MongoDB Community Server or pass --mongod <path> to setup")
    })?;

    std::fs::create_dir_all(&paths.mongo_data).map_err(|e| HostError::io(&paths.mongo_data, e))?;
    std::fs::create_dir_all(&paths.logs).map_err(|e| HostError::io(&paths.logs, e))?;
    let mongo_log = paths.logs.join("mongo.log");
    let args = mongod_args(&paths.mongo_data, &mongo_log, PORTS.mongo)?;

    let spec = ServiceSpec {
        name: "mongod".to_owned(),
        cwd: paths.workspace.clone(),
        command: PathBuf::from(mongod_bin),
        args,
        env_file: None,
        extra_env: Vec::new(),
        ready: Some(Ready::tcp("127.0.0.1", PORTS.mongo)),
    };

    let started = supervisor.start(spec, Duration::from_secs(30))?;
    if !started {
        if cancel() {
            return Ok(false);
        }
        return Err(HostError::message(format!(
            "mongod did not start on port {} within 30s (see {})",
            PORTS.mongo,
            mongo_log.display()
        )));
    }

    let after = ensure_replica_set(
        &config.mongo.uri,
        &ReplicaSetOptions {
            initiate: true,
            host: None,
            timeout_ms: Some(15_000),
        },
    )?;
    match after {
        MongoCheck::Ok { primary: true, .. } => Ok(true),
        MongoCheck::Ok { primary: false, .. } => Err(HostError::message(
            "MongoDB replica set rs0 did not become primary within 15s",
        )),
        MongoCheck::Failed { detail, .. } => Err(HostError::message(format!(
            "MongoDB replica set initialization failed: {detail}"
        ))),
    }
}

/// Writes the game server wrapper if it is missing and makes the Wine prefix.
pub fn linux_wine(paths: &Paths, config: &HostConfig) -> Result<(), HostError> {
    if !crate::wine::is_executable(&paths.gameserver_wrapper) {
        write_gameserver_wrapper(paths)?;
    }
    let lnx = linux_options(config);
    let log_file = paths.logs.join("wine.log");
    let _ = ensure_wine_prefix(paths, &lnx, &log_file)?;
    Ok(())
}

/// Waits up to 30 s for the allowlist systemd unit to answer on a public server; `false` when a stop was asked.
pub fn linux_external_allowlist(control: &Control, config: &HostConfig) -> Result<bool, HostError> {
    let own = crate::hostconfig::service_ports(config);
    if config.mode != ServerMode::Public {
        return Ok(true);
    }
    let ready = Ready::http(own.allowlist, "/status");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if control.asked() {
            return Ok(false);
        }
        if check_ready(&ready) {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let hint = if systemd_installed(UNIT_ALLOWLIST) {
        format!("sudo systemctl start {UNIT_ALLOWLIST}")
    } else {
        "install and start the systemd allowlist unit (see .workspace/systemd/install.sh)".to_owned()
    };
    Err(HostError::message(format!(
        "external allowlist service did not answer on 127.0.0.1:{} ({hint})",
        own.allowlist
    )))
}

/// Runs `dainty-host start`: the server without a console.
pub fn run(paths: &Paths) -> Outcome {
    crate::serve::run(paths, &crate::serve::ServeOptions::start())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn env_file_and_up_line_format_expected_values() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        assert_eq!(env_file(&paths, "gateway"), paths.config.join("gateway.env"));

        let cfg = HostConfig::from_json(&json!({
            "mode": "private",
            "host": "100.64.0.1",
            "gameAddress": "100.64.0.1",
            "name": "Night Hunts",
            "registration": "OPEN",
            "maxHuntServers": 4
        }))
        .unwrap();
        let line = up_line(&cfg);
        assert!(line.contains("Night Hunts"));
        assert!(line.contains("100.64.0.1:61000"));
    }

    #[test]
    fn load_config_errors_when_unconfigured() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        assert!(load_config(&paths).is_err());
    }

    #[test]
    fn load_config_refuses_unmigrated_mongo_before_requiring_settings_toml() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        std::fs::create_dir_all(&paths.mongo_data).unwrap();
        std::fs::write(paths.mongo_data.join("WiredTiger"), "wt").unwrap();
        let err = load_config(&paths).unwrap_err().to_string();
        if !cfg!(feature = "paradox") {
            assert!(
                err.contains("unmigrated MongoDB data") && err.contains("dainty-legacy import reforged"),
                "{err}"
            );
        }
    }
}
