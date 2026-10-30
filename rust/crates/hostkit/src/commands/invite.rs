// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::process::ExitCode;
use std::time::Duration;

use serde_json::json;

use crate::cli::InviteArgs;
use crate::error::HostError;
use crate::hostconfig::{
    HostConfig, Registration, SECRET_FILE, ServerMode, gateway_port, load_host_config,
    read_env_file, stringify_pretty,
};
use crate::invite::{
    CodeOptions, GameVersion, certificate_fingerprint, create_invite_code, invite_for_game,
    list_invite_codes, parse_game_version, revived_version, served_fingerprint,
};
use crate::paths::{PORTS, Paths};

pub fn run(paths: &Paths, args: &InviteArgs) -> Result<ExitCode, HostError> {
    let config = load_host_config(paths)?.ok_or_else(|| {
        HostError::message("server is not configured yet: run reforged-host setup first")
    })?;

    let target_game = resolve_game_version(&config, args.game.as_deref())?;

    if args.list {
        return run_list(paths, args.json);
    }

    let code = if target_game == revived_version() {
        crate::legacy::create_invite_code(paths)?
    } else {
        resolve_or_create_reborn_code(paths, &config, args)?
    };

    let fp = if config.mode == ServerMode::Public {
        let disk_fp = certificate_fingerprint(&paths.cert_file)?;
        let gw_port = gateway_port(&config);
        if let Some(live_fp) = served_fingerprint(gw_port, &config.host, Duration::from_millis(1500))
            && !live_fp.eq_ignore_ascii_case(&disk_fp)
        {
            return Err(HostError::message(format!(
                "gateway on {}:{} is serving certificate {live_fp}, which does not match {} ({disk_fp}); restart the server so it loads the current certificate",
                config.host,
                gw_port,
                paths.cert_file.display()
            )));
        }
        Some(disk_fp)
    } else {
        None
    };

    let url = invite_for_game(&config, &code, fp.as_deref(), &target_game)?;

    if args.json {
        let out = json!({
            "invite": url,
            "code": code,
            "game": target_game.to_string(),
            "mode": config.mode.as_str(),
            "host": config.host,
            "port": gateway_port(&config),
            "name": config.name,
            "fp": fp,
        });
        println!("{}", stringify_pretty(&out));
    } else {
        println!("Invite for {} ({target_game}):", config.name);
        println!("  {url}");
    }

    Ok(ExitCode::SUCCESS)
}

fn resolve_game_version(
    config: &HostConfig,
    game_arg: Option<&str>,
) -> Result<GameVersion, HostError> {
    let allowed = config.edition.versions();
    let Some(raw) = game_arg.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(config.edition.primary());
    };
    let parsed = parse_game_version(raw).ok_or_else(|| {
        HostError::message(format!(
            "unknown --game version {raw:?} (expected \"1.14.7\" / \"reforged\" or \"1.4.4\" / \"revived\")"
        ))
    })?;
    if !allowed.contains(&parsed) {
        return Err(HostError::message(format!(
            "this server is configured for edition {:?}, which does not run Dauntless {parsed}",
            config.edition.as_str()
        )));
    }
    Ok(parsed)
}

fn read_admin_key(paths: &Paths) -> Result<String, HostError> {
    let secrets_path = paths.config.join(SECRET_FILE);
    if secrets_path.is_file() {
        let env = read_env_file(&secrets_path)?;
        if let Some(key) = env.get("REFORGED_ADMIN_KEY").filter(|s| !s.trim().is_empty()) {
            return Ok(key.to_string());
        }
    }
    let backend_env_path = paths.config.join("backend.env");
    if backend_env_path.is_file() {
        let env = read_env_file(&backend_env_path)?;
        if let Some(key) = env.get("REFORGED_ADMIN_KEY").filter(|s| !s.trim().is_empty()) {
            return Ok(key.to_string());
        }
    }
    Err(HostError::message(
        "REFORGED_ADMIN_KEY is missing from .workspace/config/secrets.env (run reforged-host setup)",
    ))
}

fn resolve_or_create_reborn_code(
    paths: &Paths,
    config: &HostConfig,
    args: &InviteArgs,
) -> Result<String, HostError> {
    match config.registration {
        Registration::Open => Ok("OPEN".to_owned()),
        Registration::None => Err(HostError::message(
            "registration is disabled (NONE) in host.toml; change registration to INVITECODE or OPEN",
        )),
        Registration::Invitecode => {
            let uses = if args.unlimited {
                1.0
            } else {
                let n = args
                    .uses
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|&u| (1..=10_000).contains(&u))
                    .ok_or_else(|| {
                        HostError::message(format!(
                            "--uses must be an integer between 1 and 10000 (got {:?})",
                            args.uses
                        ))
                    })?;
                f64::from(n)
            };
            let admin_key = read_admin_key(paths)?;
            create_invite_code(
                PORTS.backend,
                &admin_key,
                &CodeOptions {
                    uses,
                    infinite: args.unlimited,
                    note: args.note.clone(),
                },
            )
        }
    }
}

fn run_list(paths: &Paths, as_json: bool) -> Result<ExitCode, HostError> {
    let admin_key = read_admin_key(paths)?;
    let invites = list_invite_codes(PORTS.backend, &admin_key)?;
    if as_json {
        println!("{}", stringify_pretty(&json!({ "invites": invites })));
        return Ok(ExitCode::SUCCESS);
    }
    if invites.is_empty() {
        println!("No invite codes found.");
        return Ok(ExitCode::SUCCESS);
    }
    for item in invites {
        let code = item.get("code").and_then(|v| v.as_str()).unwrap_or("?");
        let uses = item.get("uses").and_then(|v| v.as_u64()).unwrap_or(0);
        let max_uses = item.get("maxUses").and_then(|v| v.as_u64()).unwrap_or(0);
        let infinite = item.get("infinite").and_then(|v| v.as_bool()).unwrap_or(false);
        let note = item
            .get("note")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|n| format!(" ({n})"))
            .unwrap_or_default();
        if infinite {
            println!("  {code}  uses: {uses}/unlimited{note}");
        } else {
            println!("  {code}  uses: {uses}/{max_uses}{note}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::hostconfig::save_host_config;
    use tempfile::TempDir;

    #[test]
    fn open_registration_formats_invite_without_backend_call() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        let cfg = HostConfig::from_json(&json!({
            "mode": "private",
            "host": "100.64.0.1",
            "gameAddress": "100.64.0.1",
            "name": "Open Hunts",
            "registration": "OPEN",
            "maxHuntServers": 4
        }))
        .unwrap();
        save_host_config(&paths, &cfg).unwrap();

        let args = InviteArgs::default();
        let code = run(&paths, &args).unwrap();
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn resolve_game_version_checks_edition() {
        let cfg = HostConfig::from_json(&json!({
            "mode": "private",
            "edition": "reborn",
            "host": "100.64.0.1",
            "gameAddress": "100.64.0.1",
            "name": "Reborn Only",
            "registration": "OPEN",
            "maxHuntServers": 4
        }))
        .unwrap();
        assert!(resolve_game_version(&cfg, Some("1.14.7")).is_ok());
        assert!(resolve_game_version(&cfg, Some("1.4.4")).is_err());
    }
}
