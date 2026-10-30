// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

pub mod database;
pub mod firewall;
pub mod install;
pub mod options;
pub mod paradox;
pub mod secrets;

use std::path::Path;
use std::process::ExitCode;

use chrono::Utc;

use crate::cli::SetupArgs;
use crate::error::HostError;
use crate::gamedata::{GenerateOptions, generate_game_data, seed_embedded_examples};
use crate::hostconfig::{
    HostConfig, Platform, ServerMode, build_env_files, check_edition, linux_options,
    load_host_config, load_or_create_secrets, save_host_config, write_env_files,
};
use crate::linux::{ensure_wine_prefix, is_root, write_gameserver_wrapper, write_systemd_units};
use crate::mongo::find_mongod;
use crate::paths::{Paths, game_exe_relative};
use crate::preset::apply_preset;
use crate::publish::{default_sources, ensure_server_flags, install_server_runtime, publish_runtime};
use crate::tls::write_gateway_cert_if_missing;

use self::database::database_notes;
use self::firewall::{apply_windows_firewall_rules, write_firewall_script};
use self::install::{ServiceSource, install_service, repair_release, service_source, services_to_install};
use self::options::{RealSetupPromptEnv, SetupPromptEnv, resolve_options};
use self::paradox::{Layout, migrate_old_workspace};

pub const NOT_AS_ROOT: &str =
    "reforged-host setup should not be run as root (or with sudo): run it as the user that owns this folder";

pub trait SetupEnv: SetupPromptEnv {
    fn is_root(&self) -> Result<bool, HostError>;
    fn say(&mut self, line: &str);
    fn find_mongod(&self, hint: Option<&str>) -> Result<Option<String>, HostError>;
    fn source_commit(&self, repo: &Path) -> String;
    fn install_services(&mut self, p: &Paths, config: &HostConfig, log_file: &Path) -> Result<(), HostError>;
}

#[derive(Default)]
pub struct RealSetupEnv {
    pub prompt: RealSetupPromptEnv,
}

impl SetupPromptEnv for RealSetupEnv {
    fn is_tty(&self) -> bool {
        self.prompt.is_tty()
    }

    fn platform(&self) -> Platform {
        self.prompt.platform()
    }

    fn tailscale_ip(&self) -> Option<String> {
        self.prompt.tailscale_ip()
    }

    fn detected_game_dir(&self) -> Option<String> {
        self.prompt.detected_game_dir()
    }

    fn ask(&self, label: &str, default_val: &str) -> Result<String, HostError> {
        self.prompt.ask(label, default_val)
    }
}

impl SetupEnv for RealSetupEnv {
    fn is_root(&self) -> Result<bool, HostError> {
        if self.platform() == Platform::Linux {
            is_root()
        } else {
            Ok(false)
        }
    }

    fn say(&mut self, line: &str) {
        println!("{line}");
    }

    fn find_mongod(&self, hint: Option<&str>) -> Result<Option<String>, HostError> {
        find_mongod(hint)
    }

    fn source_commit(&self, repo: &Path) -> String {
        crate::selfupdate::read_manifest(repo)
            .ok()
            .flatten()
            .map(|m| m.short_commit().to_owned())
            .or_else(|| {
                crate::run::capture("git", &["rev-parse", "--short=12", "HEAD"], Some(repo))
                    .ok()
                    .flatten()
            })
            .unwrap_or_default()
    }

    fn install_services(&mut self, p: &Paths, config: &HostConfig, log_file: &Path) -> Result<(), HostError> {
        match service_source(p)? {
            ServiceSource::Release(manifest) => {
                let _ = repair_release(p, &manifest, log_file);
            }
            ServiceSource::Checkout => {
                if cfg!(feature = "paradox") && config.edition.runs_reborn() {
                    for svc in services_to_install(p) {
                        if svc.dir.join("package.json").is_file() {
                            install_service(&svc, log_file)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[must_use]
pub fn run(paths: &Paths, args: &SetupArgs) -> ExitCode {
    let mut env = RealSetupEnv::default();
    match run_with(paths, args, &mut env) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Setup failed: {err}");
            ExitCode::FAILURE
        }
    }
}

pub fn run_with(
    p: &Paths,
    args: &SetupArgs,
    env: &mut dyn SetupEnv,
) -> Result<(), HostError> {
    if env.is_root()? {
        return Err(HostError::message(NOT_AS_ROOT));
    }

    std::fs::create_dir_all(&p.config).map_err(|e| HostError::io(&p.config, e))?;
    std::fs::create_dir_all(&p.logs).map_err(|e| HostError::io(&p.logs, e))?;

    let layout = Layout::new(p);
    let moved = migrate_old_workspace(&layout, &p.repo)?;
    for item in moved {
        env.say(&format!("- migrated legacy folder: {item}"));
    }

    let existing = load_host_config(p)?;
    let mut config = resolve_options(args, existing.as_ref(), env)?;
    check_edition(&config)?;

    if cfg!(feature = "paradox")
        && config.mongo.manage
        && config.mongo.mongod.is_none()
        && let Ok(Some(found)) = env.find_mongod(None)
    {
        config.mongo.mongod = Some(found);
    }

    let (sec, created_secrets) = load_or_create_secrets(&p.config, secrets::new_secrets)?;
    let saved_config = save_host_config(p, &config)?;

    let platform = env.platform();
    let commit = env.source_commit(&p.repo);

    if config.edition.runs_reborn() {
        for dir in [
            &p.runtime_publish,
            &p.branding,
            &p.gameserver_logs,
            &p.mongo_data,
        ] {
            std::fs::create_dir_all(dir).map_err(|e| HostError::io(dir, e))?;
        }
        let env_files = build_env_files(&config, &sec, p, &commit, platform);
        write_env_files(p, &env_files)?;
    }

    if config.edition.runs_legacy() {
        crate::legacy::write_env_files(&config, &sec, p, &commit, platform)?;
    }

    if config.mode == ServerMode::Public {
        let created_cert = write_gateway_cert_if_missing(p, &config)?;
        if created_cert {
            env.say(&format!(
                "- generated gateway TLS certificate in {}",
                p.cert_file.display()
            ));
        }
        let _ = crate::hooks::get().ensure_gateway_cert(p, &config);
    }

    let setup_log = p.logs.join("setup.log");
    if !args.skip_install {
        env.install_services(p, &config, &setup_log)?;
    }

    if config.edition.runs_reborn() {
        let sources = default_sources(p);
        if sources.iter().all(|(_, path)| path.is_file()) {
            let _ = publish_runtime(p, Utc::now(), &sources)?;
        }

        let preset_applied = apply_preset(p, false)?;
        let seeded = seed_embedded_examples(p)?;
        if !seeded.is_empty() {
            env.say(&format!("- seeded {} example game-data file(s)", seeded.len()));
        }
        if preset_applied.is_none() && p.exports.is_dir() {
            let _ = generate_game_data(
                p,
                &GenerateOptions {
                    examples_for_missing: true,
                    log_file: Some(setup_log.clone()),
                },
                |_| {},
            );
        }

        if let Some(ref game_dir) = config.game_dir {
            let exe = Path::new(game_dir).join(game_exe_relative());
            if let Some(win64_dir) = exe.parent().filter(|d| d.is_dir()) {
                let _ = install_server_runtime(p, win64_dir);
                let _ = ensure_server_flags(win64_dir);
            }
        }
        let _ = crate::hooks::get().install_director_runner(p);

        if !cfg!(feature = "paradox") {
            let mut say_fn = |line: &str| env.say(line);
            let _ = crate::commands::update::migrate_existing_mongo_data(
                p,
                &config,
                &setup_log,
                &mut say_fn,
            );
        }
    }

    if platform == Platform::Linux {
        let _ = write_gameserver_wrapper(p);
        let lnx = linux_options(&config);
        let _ = ensure_wine_prefix(p, &lnx, &setup_log);
        let _ = write_systemd_units(&config, p);
    } else if platform == Platform::Windows {
        let _ = apply_windows_firewall_rules(&config, &setup_log);
    }

    let fw_script = write_firewall_script(p, &config)?;
    env.say(&format!("- firewall script written to {}", fw_script.display()));

    for note in database_notes(&config, platform) {
        env.say(&note);
    }

    env.say(&format!("- configuration saved to {}", p.host_toml.display()));
    if created_secrets {
        env.say(&format!(
            "- generated secrets in {}",
            p.config.join("secrets.env").display()
        ));
    }
    if !saved_config {
        env.say("- host configuration unchanged");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn setup_creates_host_toml_secrets_and_env_files() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        let args = SetupArgs {
            mode: Some("private".to_owned()),
            host: Some("100.64.0.1".to_owned()),
            game_address: Some("100.64.0.1".to_owned()),
            skip_install: true,
            ..SetupArgs::default()
        };

        let mut env = RealSetupEnv {
            prompt: RealSetupPromptEnv {
                is_tty: false,
                platform: Platform::Windows,
                tailscale_ip: Some("100.64.0.1".to_owned()),
                detected_game_dir: None,
            },
        };
        run_with(&paths, &args, &mut env).unwrap();
        assert!(paths.host_toml.is_file());
        assert!(paths.config.join("secrets.env").is_file());
        assert!(paths.config.join("backend.env").is_file());
        assert!(paths.config.join("gateway.env").is_file());
    }
}
