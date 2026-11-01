// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};

use crate::cli::InviteArgs;
use crate::error::HostError;
use crate::hostconfig::{
    HostConfig, Registration, SECRET_FILE, ServerMode, gateway_port, load_host_config, read_env_file, stringify_pretty,
};
use crate::invite::{
    CodeOptions, GameVersion, certificate_fingerprint, create_invite_code, invite_for_game, list_invite_codes,
    parse_game_version, reforged_version, served_fingerprint,
};
use crate::paths::{PORTS, Paths};

pub fn run(paths: &Paths, args: &InviteArgs) -> Result<ExitCode, HostError> {
    run_with(paths, args, PORTS.backend, &mut |l| println!("{l}"))
}

pub fn run_with(
    paths: &Paths,
    args: &InviteArgs,
    backend_port: u16,
    out: &mut dyn FnMut(&str),
) -> Result<ExitCode, HostError> {
    let config =
        load_host_config(paths)?.ok_or_else(|| HostError::message("not set up yet: run reforged-host setup first"))?;

    let target_game = resolve_game_version(args.game.as_deref())?;

    if args.list {
        return run_list(paths, args.json, backend_port, out);
    }

    let uses_val = parse_uses_arg(args)?;

    let code = resolve_or_create_reborn_code(paths, &config, args, uses_val, backend_port)?;

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
    let for_whom = args
        .for_whom
        .as_deref()
        .map(service_kit::js::trim)
        .filter(|s| !s.is_empty());

    if args.json {
        let mut obj = serde_json::Map::new();
        obj.insert("link".into(), Value::String(url));
        obj.insert(
            "for".into(),
            for_whom.map_or(Value::Null, |s| Value::String(s.to_owned())),
        );
        obj.insert(
            "uses".into(),
            if args.unlimited {
                Value::Null
            } else {
                Value::Number((uses_val as i64).into())
            },
        );
        obj.insert(
            "shareUrl".into(),
            config.share_url.clone().map_or(Value::Null, Value::String),
        );
        out(&service_kit::jsjson::stringify(&Value::Object(obj)));
    } else {
        let uses_label = if args.unlimited {
            "unlimited uses".to_owned()
        } else if uses_val as u64 == 1 {
            "1 use".to_owned()
        } else {
            format!("{} uses", uses_val as u64)
        };
        let header = match for_whom {
            Some(who) => format!("\nInvite for {who} ({uses_label}):\n"),
            None => format!("\nInvite ({uses_label}):\n"),
        };
        out(&header);
        out(&format!("  {url}\n"));
        out(
            "Send the whole line. They paste it into the DaintyLass launcher (or click it once the launcher is installed).",
        );
        if config.mode == ServerMode::Private {
            out(
                "They also need Tailscale, and access to this PC: share it from https://login.tailscale.com/admin/machines.",
            );
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn resolve_game_version(game_arg: Option<&str>) -> Result<GameVersion, HostError> {
    let Some(raw) = game_arg.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(reforged_version());
    };
    parse_game_version(raw).ok_or_else(|| {
        HostError::message(format!(
            "unknown --game version {raw:?} (expected \"1.14.7\" / \"reforged\")"
        ))
    })
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

fn parse_uses_arg(args: &InviteArgs) -> Result<f64, HostError> {
    if args.unlimited {
        return Ok(1.0);
    }
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
    Ok(f64::from(n))
}

fn resolve_or_create_reborn_code(
    paths: &Paths,
    config: &HostConfig,
    args: &InviteArgs,
    uses: f64,
    backend_port: u16,
) -> Result<String, HostError> {
    match config.registration {
        Registration::Open => Ok("OPEN".to_owned()),
        Registration::None => Err(HostError::message(
            "registration is disabled (NONE) in host.toml; change registration to INVITECODE or OPEN",
        )),
        Registration::Invitecode => {
            let admin_key = read_admin_key(paths)?;
            create_invite_code(
                &admin_key,
                &CodeOptions {
                    uses,
                    infinite: args.unlimited,
                    note: args.for_whom.clone(),
                },
                backend_port,
            )
        }
    }
}

fn run_list(paths: &Paths, as_json: bool, backend_port: u16, out: &mut dyn FnMut(&str)) -> Result<ExitCode, HostError> {
    let admin_key = read_admin_key(paths)?;
    let invites = list_invite_codes(backend_port, &admin_key)?;
    if as_json {
        out(&stringify_pretty(&json!({ "invites": invites })));
        return Ok(ExitCode::SUCCESS);
    }
    if invites.is_empty() {
        out("No invite codes found.");
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
            out(&format!("  {code}  uses: {uses}/unlimited{note}"));
        } else {
            out(&format!("  {code}  uses: {uses}/{max_uses}{note}"));
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
    fn resolve_game_version_takes_1_14_7_only() {
        assert!(resolve_game_version(Some("1.14.7")).is_ok());
        assert!(resolve_game_version(Some("1.4.4")).is_err());
    }
}
