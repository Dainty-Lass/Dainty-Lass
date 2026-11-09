// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `dainty-host setup`: host.toml from the options and the file already there, secrets.env, the env files,
//! the gateway certificate, the firewall rules and, on Linux, Wine and the systemd units.

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
use crate::doctor::tailscale_ipv4;
use crate::error::HostError;
use crate::gamedata::{GenerateOptions, generate_game_data, seed_embedded_examples};
use crate::hostconfig::{
    Platform, ServerMode, build_env_files, linux_options, load_host_config, load_or_create_secrets, save_host_config,
    write_env_files,
};
use crate::invite::certificate_fingerprint;
use crate::linux::{ensure_wine_prefix, is_root, write_gameserver_wrapper, write_systemd_units};
use crate::mongo::{find_mongod, search_path};
use crate::paths::{Paths, game_exe_relative};
use crate::preset::{PresetApplied, apply_preset};
use crate::publish::{default_sources, ensure_server_flags, install_server_runtime, publish_runtime};
use crate::tls::generate_self_signed_cert_pem;

use self::database::database_notes;
use self::firewall::{apply_windows_firewall_rules, write_firewall_script};
use self::install::{ServiceSource, install_from_release, install_service, service_source, services_to_install};
use self::options::{host_config_from_args, performance_summary};
use self::paradox::{Layout, missing_paradox};

/// The refusal setup gives when run as root on Linux.
pub const NOT_AS_ROOT: &str =
    "dainty-host setup should not be run as root (or with sudo): run it as the user that owns this folder";

/// What setup needs of the machine, so a test can stand in for it.
pub trait SetupEnv {
    /// The OS the env files are written for.
    fn platform(&self) -> Platform;
    /// Whether setup runs as root (only checked on Linux).
    fn is_root(&self) -> Result<bool, HostError>;
    /// This PC's Tailscale IPv4 address, when Tailscale runs.
    fn tailscale_ipv4(&self) -> Result<Option<String>, HostError>;
    /// The mongod executable, when one is installed.
    fn find_mongod(&self) -> Result<Option<String>, HostError>;
    /// The executable `name` on PATH.
    fn find_on_path(&self, name: &str) -> Result<Option<String>, HostError>;
    /// The short commit of the release or checkout at `repo`; empty when unknown.
    fn source_commit(&self, repo: &Path) -> Result<String, HostError>;
    /// Applies the checkout's preset without overwriting the host's own game data.
    fn apply_preset(&self, paths: &Paths) -> Result<Option<PresetApplied>, HostError>;
    /// Installs or checks the services, printing what it does through `out`.
    fn install_services(&self, layout: &Layout, log_file: &Path, out: &mut dyn FnMut(&str)) -> Result<(), HostError>;
    /// Writes the gateway's self-signed certificate and key for `hosts` into `tls`.
    fn make_certificate(&self, layout: &Layout, hosts: &[String], tls: &Path, log_file: &Path)
    -> Result<(), HostError>;
    /// The program the firewall rules let through: the gateway's.
    fn gateway_program(&self) -> String;
    /// Whether a checkout must hold Rustic Isles' sources.
    fn require_checkout_sources(&self) -> bool {
        true
    }
    /// The environment the services will inherit, whose variables override host.toml (RD24).
    fn inherited_env(&self) -> Vec<(String, String)> {
        std::env::vars().collect()
    }
}

/// The real machine.
#[derive(Default)]
pub struct RealSetupEnv;

impl SetupEnv for RealSetupEnv {
    fn platform(&self) -> Platform {
        Platform::current()
    }

    fn is_root(&self) -> Result<bool, HostError> {
        if self.platform() == Platform::Linux {
            is_root()
        } else {
            Ok(false)
        }
    }

    fn tailscale_ipv4(&self) -> Result<Option<String>, HostError> {
        Ok(tailscale_ipv4())
    }

    fn find_mongod(&self) -> Result<Option<String>, HostError> {
        find_mongod(None)
    }

    fn find_on_path(&self, name: &str) -> Result<Option<String>, HostError> {
        let path_var = std::env::var_os("PATH");
        Ok(search_path([name], path_var.as_deref()).map(|p| p.to_string_lossy().into_owned()))
    }

    fn source_commit(&self, repo: &Path) -> Result<String, HostError> {
        Ok(crate::selfupdate::read_manifest(repo)
            .ok()
            .flatten()
            .map(|m| m.short_commit().to_owned())
            .or_else(|| {
                crate::run::capture("git", &["rev-parse", "--short=12", "HEAD"], Some(repo))
                    .ok()
                    .flatten()
            })
            .unwrap_or_default())
    }

    fn apply_preset(&self, paths: &Paths) -> Result<Option<PresetApplied>, HostError> {
        apply_preset(paths, false)
    }

    fn install_services(&self, layout: &Layout, log_file: &Path, out: &mut dyn FnMut(&str)) -> Result<(), HostError> {
        let paths = Paths::new(&layout.repo);
        match service_source(&paths)? {
            ServiceSource::Release(manifest) => {
                let platform = crate::selfupdate::this_platform()?;
                let source = crate::selfupdate::Source::from_env(Some(&manifest));
                install_from_release(&layout.repo, Some(&manifest), platform, &source, log_file, out)?;
            }
            ServiceSource::Checkout => {
                if cfg!(feature = "paradox") {
                    for svc in services_to_install(&paths) {
                        if svc.dir.join("package.json").is_file() {
                            out(&format!("- {}: npm ci + build", svc.name));
                            install_service(&svc, log_file)?;
                        }
                    }
                } else {
                    for (_, label) in install::services(layout) {
                        out(&format!("- {label}: built in"));
                    }
                }
            }
        }
        Ok(())
    }

    fn make_certificate(
        &self,
        _layout: &Layout,
        hosts: &[String],
        tls: &Path,
        _log_file: &Path,
    ) -> Result<(), HostError> {
        std::fs::create_dir_all(tls).map_err(|e| HostError::io(tls, e))?;
        let (cert_pem, key_pem) = generate_self_signed_cert_pem(hosts)?;
        let cert_file = tls.join("gateway-cert.pem");
        let key_file = tls.join("gateway-key.pem");
        std::fs::write(&cert_file, cert_pem.as_bytes()).map_err(|e| HostError::io(&cert_file, e))?;
        std::fs::write(&key_file, key_pem.as_bytes()).map_err(|e| HostError::io(&key_file, e))?;
        Ok(())
    }

    fn gateway_program(&self) -> String {
        std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "dainty-host".to_owned())
    }
}

/// Runs `dainty-host setup`, printing to stdout; an error is printed and gives a failing exit code.
#[must_use]
pub fn run(paths: &Paths, args: &SetupArgs) -> ExitCode {
    let env = RealSetupEnv;
    match run_with(paths, args, &env, &mut |line| println!("{line}")) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Runs setup, printing through `out`; fails before writing anything on a bad option.
pub fn run_with(
    p: &Paths,
    args: &SetupArgs,
    env: &dyn SetupEnv,
    out: &mut dyn FnMut(&str),
) -> Result<ExitCode, HostError> {
    if env.is_root()? {
        return Err(HostError::message(NOT_AS_ROOT));
    }

    let layout = Layout::new(p);
    let existing = load_host_config(p)?;
    let mut config = host_config_from_args(args, existing.as_ref(), env)?;

    let is_release = crate::selfupdate::read_manifest(&p.repo)?.is_some();
    if !is_release && !args.skip_install && env.require_checkout_sources() {
        let missing = missing_paradox(&layout);
        if let Some(first) = missing.first() {
            return Err(HostError::message(format!(
                "this checkout has no {first}: run git submodule update --init --recursive"
            )));
        }
    }

    std::fs::create_dir_all(&p.config).map_err(|e| HostError::io(&p.config, e))?;
    std::fs::create_dir_all(&p.logs).map_err(|e| HostError::io(&p.logs, e))?;

    let setup_log = p.logs.join("setup.log");

    match config.mode {
        ServerMode::Private => out(&format!(
            "\nPrivate server {:?} at {} (game servers on {})",
            config.name, config.host, config.game_address
        )),
        ServerMode::Public => out(&format!(
            "\nPublic server {:?} at {} (game servers on {})",
            config.name, config.host, config.game_address
        )),
    }
    out(&performance_summary(&config));

    out("\n1. Rustic Isles (modules/rustic-isles and native/ in this project)\n- in place");

    if args.skip_install {
        out("\n2. Skipped installing and building (--skip-install)");
    } else {
        out("\n2. Installing and building services");
        env.install_services(&layout, &setup_log, out)?;
    }

    if args.fresh_start {
        config.database.get_or_insert_with(Default::default).fresh_start = Some(true);
    }

    let (sec, created_secrets) = load_or_create_secrets(&p.config, secrets::new_secrets)?;
    save_host_config(p, &config)?;
    if config
        .remote
        .as_ref()
        .is_some_and(|r| r.role.as_deref() == Some("controller"))
    {
        let _ = crate::commands::remote::set_shared_config_from_host_toml(p);
    }
    if created_secrets {
        out(&format!(
            "- secrets: made (kept from now on) in {}",
            p.config.join("secrets.env").display()
        ));
    } else {
        out(&format!(
            "- secrets: kept in {}",
            p.config.join("secrets.env").display()
        ));
    }

    let platform = env.platform();
    let commit = env.source_commit(&p.repo)?;

    let mut written_env_names: Vec<String> = Vec::new();
    for dir in [
        &p.runtime_publish,
        &p.branding,
        &p.gameserver_logs,
        &p.mongo_data,
        &p.raw_game_data,
    ] {
        std::fs::create_dir_all(dir).map_err(|e| HostError::io(dir, e))?;
    }
    // The files hosts write by hand, in TOML (RD24 phase 3)
    for moved in [crate::hostfiles::ensure_news(p)?, crate::hostfiles::render_supplies(p)?]
        .into_iter()
        .flatten()
    {
        out(&moved);
    }
    let env_files = build_env_files(&config, &sec, p, &commit, platform);
    if !env_files.passthrough_refused.is_empty() {
        return Err(HostError::message(format!(
            "host.toml's [env] tables can't be applied:
  {}",
            env_files.passthrough_refused.join(
                "
  "
            )
        )));
    }
    // Each service's own loader, on the env it will start with (RD24)
    let checked = crate::hostconfig::check::check(&env_files, &env.inherited_env())?;
    for warning in &checked.warnings {
        out(&format!("- warning: {warning}"));
    }
    write_env_files(p, &env_files)?;
    for (name, entries) in env_files.files() {
        if entries.is_some() {
            written_env_names.push(format!("{name}.env"));
        }
    }
    // What a server that also ran Dauntless Revived's 1.4.4 had beside them
    for stale in [
        "metagame.env",
        "deploy.env",
        "legacy-content.env",
        "legacy-gateway.env",
        "legacy-allowlist.env",
    ] {
        let _ = std::fs::remove_file(p.config.join(stale));
    }
    out(&format!(
        "- env files: {} in {}",
        written_env_names.join(", "),
        p.config.display()
    ));

    if config.mode == ServerMode::Public {
        let tls_dir = p.cert_file.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        if p.cert_file.is_file() && p.key_file.is_file() {
            out(&format!("- gateway certificate kept in {}", p.cert_file.display()));
        } else {
            let mut hosts = vec![config.host.clone()];
            if config.game_address != config.host {
                hosts.push(config.game_address.clone());
            }
            env.make_certificate(&layout, &hosts, &tls_dir, &setup_log)?;
            out(&format!("- gateway certificate made in {}", p.cert_file.display()));
        }
        if let Ok(fp) = certificate_fingerprint(&p.cert_file) {
            out(&format!("- certificate fingerprint (goes into every invite): {fp}"));
        }
        let _ = crate::hooks::get().ensure_gateway_cert(p, &config);
    }

    let sources = default_sources(p);
    if sources.iter().all(|(_, path)| path.is_file()) {
        let _ = publish_runtime(p, Utc::now(), &sources)?;
    }

    let preset_applied = env.apply_preset(p)?;
    let _ = seed_embedded_examples(p)?;
    if preset_applied.is_none() && p.exports.is_dir() {
        let _ = generate_game_data(
            p,
            &GenerateOptions {
                examples_for_missing: true,
                log_file: setup_log.clone(),
                only: None,
            },
            &mut |_| {},
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

    if platform == Platform::Linux {
        let _ = write_gameserver_wrapper(p);
        let lnx = linux_options(&config);
        let _ = ensure_wine_prefix(p, &lnx, &setup_log);
        let _ = write_systemd_units(&config, p);
    } else if platform == Platform::Windows {
        let _ = apply_windows_firewall_rules(&config, &setup_log);
    }

    let gw_prog = env.gateway_program();
    let fw_script = write_firewall_script(p, &config, &gw_prog)?;
    if platform == Platform::Linux {
        out(&format!(
            "- firewall script written to {}",
            p.config.join("firewall.sh").display()
        ));
    } else {
        out(&format!("- firewall script written to {}", fw_script.display()));
    }

    for note in database_notes(&config, platform) {
        out(&note);
    }

    if config.game_dir.is_none() {
        out("\n! No game folder yet. Pass --game <path to Dauntless> or run dainty-host install-game.");
    }

    out("\nNext steps:\n  dainty-host start\n  dainty-host doctor shows what is still missing.");

    Ok(ExitCode::SUCCESS)
}
