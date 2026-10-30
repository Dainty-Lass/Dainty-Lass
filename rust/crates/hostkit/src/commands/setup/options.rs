// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::io::{self, IsTerminal, Write};
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::cli::SetupArgs;
use crate::doctor::tailscale_ipv4;
use crate::error::HostError;
use crate::hostconfig::{
    DEFAULT_MONGO_URI, Edition, HostConfig, Platform, Registration, ServerMode, is_dns_name,
    is_ipv4, is_public_ipv4, is_tailscale_ipv4, is_valid_share_url, linux_options,
    multipliers_on, performance_options,
};
use crate::paths::{GAME_VERSION_TEXT, MAX_HUNTS, PORTS, game_exe_relative};

pub const DEFAULT_HOST: &str = "100.64.0.1";
pub const DEFAULT_NAME: &str = "Dauntless Reforged Revival";
pub const DEFAULT_MAX_HUNT_SERVERS: u32 = 3;
pub const HOST_CONFIG_VERSION: u32 = 1;

pub trait SetupPromptEnv {
    fn is_tty(&self) -> bool;
    fn platform(&self) -> Platform;
    fn tailscale_ip(&self) -> Option<String>;
    fn detected_game_dir(&self) -> Option<String>;
    fn ask(&self, label: &str, default_val: &str) -> Result<String, HostError>;
}

#[derive(Debug, Clone)]
pub struct RealSetupPromptEnv {
    pub is_tty: bool,
    pub platform: Platform,
    pub tailscale_ip: Option<String>,
    pub detected_game_dir: Option<String>,
}

impl Default for RealSetupPromptEnv {
    fn default() -> Self {
        let is_tty = io::stdin().is_terminal() && io::stdout().is_terminal();
        Self {
            is_tty,
            platform: Platform::current(),
            tailscale_ip: tailscale_ipv4(),
            detected_game_dir: detect_default_game_dir(),
        }
    }
}

impl SetupPromptEnv for RealSetupPromptEnv {
    fn is_tty(&self) -> bool {
        self.is_tty
    }

    fn platform(&self) -> Platform {
        self.platform
    }

    fn tailscale_ip(&self) -> Option<String> {
        self.tailscale_ip.clone()
    }

    fn detected_game_dir(&self) -> Option<String> {
        self.detected_game_dir.clone()
    }

    fn ask(&self, label: &str, default_val: &str) -> Result<String, HostError> {
        prompt_stdio(label, default_val)
    }
}

fn parse_u32_flag(flag: &str, raw: &str) -> Result<u32, HostError> {
    raw.trim()
        .parse::<u32>()
        .map_err(|_| HostError::message(format!("{flag} must be a non-negative integer (got {raw:?})")))
}

fn parse_u16_port_flag(flag: &str, raw: &str) -> Result<u16, HostError> {
    let val = raw
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|&n| n >= 1)
        .ok_or_else(|| {
            HostError::message(format!("{flag} must be a port between 1 and 65535 (got {raw:?})"))
        })?;
    Ok(val)
}

fn parse_f64_flag(flag: &str, raw: &str) -> Result<f64, HostError> {
    let val = raw
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n > 0.0 && *n <= 1000.0)
        .ok_or_else(|| {
            HostError::message(format!(
                "{flag} must be a number greater than 0 and at most 1000 (got {raw:?})"
            ))
        })?;
    Ok(val)
}

pub fn check_unexpected_args(args: &SetupArgs) -> Result<(), HostError> {
    if let Some(first) = args.unexpected.first() {
        return Err(HostError::message(format!(
            "unexpected argument to setup: {first}"
        )));
    }
    Ok(())
}

fn is_valid_host_for_mode(mode: ServerMode, host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    match mode {
        ServerMode::Private => is_tailscale_ipv4(&h) || (is_dns_name(&h) && h.ends_with(".ts.net")),
        ServerMode::Public => is_public_ipv4(&h) || is_dns_name(&h),
    }
}

pub fn resolve_options(
    args: &SetupArgs,
    existing: Option<&HostConfig>,
    env: &dyn SetupPromptEnv,
) -> Result<HostConfig, HostError> {
    check_unexpected_args(args)?;

    let interactive = env.is_tty();

    let default_mode = existing
        .map(|c| c.mode.as_str().to_owned())
        .unwrap_or_else(|| "private".to_owned());
    let mode_raw = if let Some(m) = &args.mode {
        m.trim().to_ascii_lowercase()
    } else if interactive {
        let ans = env.ask("Server mode (private/public)", &default_mode)?;
        if ans.trim().is_empty() {
            default_mode
        } else {
            ans.trim().to_ascii_lowercase()
        }
    } else {
        default_mode
    };
    let mode = ServerMode::parse(&mode_raw).ok_or_else(|| {
        HostError::message(format!(
            "--mode must be \"private\" or \"public\" (got {mode_raw:?})"
        ))
    })?;
    let is_public = mode == ServerMode::Public;

    let default_edition = existing
        .map(|c| c.edition.as_str().to_owned())
        .unwrap_or_else(|| "reborn".to_owned());
    let edition_raw = if let Some(ed) = &args.edition {
        ed.trim().to_ascii_lowercase()
    } else if interactive {
        let ans = env.ask("Edition (reborn/legacy/both)", &default_edition)?;
        if ans.trim().is_empty() {
            default_edition
        } else {
            ans.trim().to_ascii_lowercase()
        }
    } else {
        default_edition
    };
    let edition = Edition::parse(&edition_raw).ok_or_else(|| {
        HostError::message(format!(
            "--edition must be \"reborn\", \"legacy\", or \"both\" (got {edition_raw:?})"
        ))
    })?;

    let ts_ip = env.tailscale_ip();
    let fallback_host = if is_public {
        existing
            .filter(|c| c.mode == ServerMode::Public)
            .map(|c| c.host.clone())
            .unwrap_or_else(|| "example.com".to_owned())
    } else {
        ts_ip
            .clone()
            .or_else(|| {
                existing
                    .filter(|c| c.mode == ServerMode::Private)
                    .map(|c| c.host.clone())
            })
            .unwrap_or_else(|| DEFAULT_HOST.to_owned())
    };

    let host = if let Some(h) = &args.host {
        h.trim().to_owned()
    } else if interactive {
        let ans = env.ask("Server host or IPv4", &fallback_host)?;
        if ans.trim().is_empty() {
            fallback_host
        } else {
            ans.trim().to_owned()
        }
    } else {
        fallback_host
    };
    if !is_valid_host_for_mode(mode, &host) {
        return Err(HostError::message(format!("invalid --host for {mode} mode: {host:?}")));
    }

    let default_game_address = existing
        .map(|c| {
            if c.game_address == c.host && is_ipv4(&host) {
                host.clone()
            } else {
                c.game_address.clone()
            }
        })
        .or_else(|| is_ipv4(&host).then(|| host.clone()))
        .or(ts_ip)
        .unwrap_or_else(|| DEFAULT_HOST.to_owned());

    let game_address = if let Some(ga) = &args.game_address {
        ga.trim().to_owned()
    } else if interactive && (!is_ipv4(&host) || is_public) {
        let ans = env.ask("Game server IPv4 address", &default_game_address)?;
        if ans.trim().is_empty() {
            default_game_address
        } else {
            ans.trim().to_owned()
        }
    } else if is_ipv4(&host) && existing.is_none() {
        host.clone()
    } else {
        default_game_address
    };
    if !is_ipv4(&game_address) {
        return Err(HostError::message(format!(
            "--game-address must be an IPv4 address (got {game_address:?})"
        )));
    }

    let default_name = existing
        .map(|c| c.name.clone())
        .unwrap_or_else(|| DEFAULT_NAME.to_owned());
    let name = if let Some(n) = &args.name {
        n.trim().to_owned()
    } else if interactive {
        let ans = env.ask("Server display name", &default_name)?;
        if ans.trim().is_empty() {
            default_name
        } else {
            ans.trim().to_owned()
        }
    } else {
        default_name
    };

    let default_game = existing
        .and_then(|c| c.game_dir.clone())
        .or_else(|| env.detected_game_dir())
        .unwrap_or_default();
    let game_dir_str = if let Some(g) = &args.game {
        g.trim().to_owned()
    } else if interactive && edition.runs_reborn() {
        env.ask("Dauntless 1.14.7 game folder (optional)", &default_game)?
            .trim()
            .to_owned()
    } else {
        default_game
    };

    let default_legacy_game = existing
        .and_then(|c| c.legacy_game_dir.clone())
        .unwrap_or_default();
    let legacy_game_dir_str = if let Some(lg) = &args.legacy_game {
        lg.trim().to_owned()
    } else if interactive && edition.runs_legacy() {
        env.ask("Dauntless 1.4.4 legacy game folder (optional)", &default_legacy_game)?
            .trim()
            .to_owned()
    } else {
        default_legacy_game
    };

    let default_reg = existing
        .map(|c| c.registration.as_str().to_owned())
        .unwrap_or_else(|| "INVITECODE".to_owned());
    let reg_str = if let Some(r) = &args.registration {
        r.trim().to_ascii_uppercase()
    } else if interactive {
        let ans = env.ask("Registration (OPEN, INVITECODE, or NONE)", &default_reg)?;
        if ans.trim().is_empty() {
            default_reg
        } else {
            ans.trim().to_ascii_uppercase()
        }
    } else {
        default_reg
    };
    let registration = Registration::parse(&reg_str).ok_or_else(|| {
        HostError::message(format!(
            "--registration must be OPEN, INVITECODE, or NONE (got {reg_str:?})"
        ))
    })?;

    let share_url = if is_public {
        None
    } else if let Some(sh) = &args.share {
        let trimmed = sh.trim();
        if trimmed.is_empty() {
            None
        } else {
            if !is_valid_share_url(trimmed) {
                return Err(HostError::message(
                    "--share must be a https://login.tailscale.com/... link",
                ));
            }
            Some(trimmed.to_owned())
        }
    } else {
        existing.and_then(|c| c.share_url.clone())
    };

    let mongo_uri = args
        .mongo_uri
        .as_ref()
        .map(|s| s.trim().to_owned())
        .or_else(|| existing.map(|c| c.mongo.uri.clone()))
        .unwrap_or_else(|| DEFAULT_MONGO_URI.to_owned());

    let manage_mongo = if args.no_manage_mongo {
        false
    } else if args.manage_mongo {
        true
    } else if let Some(c) = existing {
        c.mongo.manage
    } else {
        true
    };

    let mongod = if let Some(m) = &args.mongod {
        let trimmed = m.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    } else {
        existing.and_then(|c| c.mongo.mongod.clone())
    };

    let max_hunt_servers = if let Some(mh) = &args.max_hunts {
        let parsed = parse_u32_flag("--max-hunts", mh)?;
        if !(1..=MAX_HUNTS).contains(&parsed) {
            return Err(HostError::message(format!(
                "--max-hunts must be an integer between 1 and {MAX_HUNTS} (got {parsed})"
            )));
        }
        parsed
    } else if let Some(c) = existing {
        c.max_hunt_servers
    } else {
        DEFAULT_MAX_HUNT_SERVERS
    };

    let public_port = if let Some(pp) = &args.public_port {
        let port = parse_u16_port_flag("--public-port", pp)?;
        Some(port)
    } else {
        existing.and_then(|c| c.public_port)
    };

    let legacy_public_port = if let Some(lpp) = &args.legacy_public_port {
        let port = parse_u16_port_flag("--legacy-public-port", lpp)?;
        Some(port)
    } else {
        existing.and_then(|c| c.legacy_public_port)
    };
    let _ = PORTS;

    let mut perf = existing.map(performance_options).unwrap_or_default();
    if let Some(raw) = &args.server_fps {
        perf.server_fps = parse_u32_flag("--server-fps", raw)?;
    }
    if let Some(raw) = &args.idle_fps {
        perf.idle_fps = parse_u32_flag("--idle-fps", raw)?;
    }
    if args.training_on_demand {
        perf.training_on_demand = true;
    }
    if args.training_at_boot {
        perf.training_on_demand = false;
    }
    if let Some(raw) = &args.training_idle_minutes {
        perf.training_idle_minutes = parse_u32_flag("--training-idle-minutes", raw)?;
    }
    if args.verbose_game_logs {
        perf.verbose_game_logs = true;
    }
    if args.quiet_game_logs {
        perf.verbose_game_logs = false;
    }
    if args.memory_guard {
        perf.memory_guard = true;
    }
    if args.no_memory_guard {
        perf.memory_guard = false;
    }
    if let Some(raw) = &args.min_free_ram {
        perf.min_free_memory_mb = parse_u32_flag("--min-free-ram", raw)?;
    }
    if let Some(raw) = &args.hunt_ram {
        perf.hunt_memory_mb = parse_u32_flag("--hunt-ram", raw)?;
    }

    let mut doc = Map::new();
    doc.insert("version".into(), json!(HOST_CONFIG_VERSION));
    doc.insert("mode".into(), json!(mode.as_str()));
    doc.insert("edition".into(), json!(edition.as_str()));
    doc.insert("host".into(), json!(host));
    doc.insert("gameAddress".into(), json!(game_address));
    doc.insert("name".into(), json!(name));
    if !game_dir_str.is_empty() {
        doc.insert("gameDir".into(), json!(game_dir_str));
    }
    if !legacy_game_dir_str.is_empty() {
        doc.insert("legacyGameDir".into(), json!(legacy_game_dir_str));
    }
    doc.insert("registration".into(), json!(registration.as_str()));
    if let Some(share) = share_url {
        doc.insert("shareUrl".into(), json!(share));
    }
    if let Some(pp) = public_port {
        doc.insert("publicPort".into(), json!(pp));
    }
    if let Some(lpp) = legacy_public_port {
        doc.insert("legacyPublicPort".into(), json!(lpp));
    }

    let mut mongo_obj = Map::new();
    mongo_obj.insert("uri".into(), json!(mongo_uri));
    mongo_obj.insert("manage".into(), json!(manage_mongo));
    if let Some(m) = mongod {
        mongo_obj.insert("mongod".into(), json!(m));
    }
    doc.insert("mongo".into(), Value::Object(mongo_obj));
    doc.insert("maxHuntServers".into(), json!(max_hunt_servers));

    if !args.no_multipliers {
        let prev_mult = existing.and_then(|c| c.multipliers.as_ref());
        let xp = match &args.xp_multiplier {
            Some(raw) => Some(parse_f64_flag("--xp-multiplier", raw)?),
            None => prev_mult.map(|m| m.xp),
        };
        let rewards = match &args.reward_multiplier {
            Some(raw) => Some(parse_f64_flag("--reward-multiplier", raw)?),
            None => prev_mult.map(|m| m.rewards),
        };
        if xp.is_some() || rewards.is_some() {
            let xp_val = xp.unwrap_or(1.0);
            let rewards_val = rewards.unwrap_or(1.0);
            if (xp_val - 1.0).abs() > f64::EPSILON || (rewards_val - 1.0).abs() > f64::EPSILON {
                doc.insert(
                    "multipliers".into(),
                    json!({
                        "xp": xp_val,
                        "rewards": rewards_val,
                    }),
                );
            }
        }
    }

    doc.insert(
        "performance".into(),
        json!({
            "serverFps": perf.server_fps,
            "idleFps": perf.idle_fps,
            "trainingOnDemand": perf.training_on_demand,
            "trainingIdleMinutes": perf.training_idle_minutes,
            "verboseGameLogs": perf.verbose_game_logs,
            "memoryGuard": perf.memory_guard,
            "minFreeMemoryMb": perf.min_free_memory_mb,
            "huntMemoryMb": perf.hunt_memory_mb,
        }),
    );

    let has_linux_flags = args.wine.is_some()
        || args.proton.is_some()
        || args.no_proton
        || args.xvfb
        || args.no_xvfb
        || args.firewall_interface.is_some()
        || args.no_firewall_interface;
    if env.platform() == Platform::Linux
        || has_linux_flags
        || existing.and_then(|c| c.linux.as_ref()).is_some()
    {
        let mut lnx = existing.map(linux_options).unwrap_or_default();
        if let Some(w) = &args.wine {
            let trimmed = w.trim();
            lnx.wine = (!trimmed.is_empty()).then(|| trimmed.to_owned());
        }
        if args.no_proton {
            lnx.proton = None;
        } else if let Some(pr) = &args.proton {
            let trimmed = pr.trim();
            lnx.proton = (!trimmed.is_empty()).then(|| trimmed.to_owned());
        }
        if args.xvfb {
            lnx.xvfb = true;
        }
        if args.no_xvfb {
            lnx.xvfb = false;
        }
        if args.no_firewall_interface {
            lnx.firewall_interface = None;
        } else if let Some(iface) = &args.firewall_interface {
            let trimmed = iface.trim();
            lnx.firewall_interface = (!trimmed.is_empty()).then(|| trimmed.to_owned());
        }
        let mut lnx_obj = Map::new();
        if let Some(w) = lnx.wine {
            lnx_obj.insert("wine".into(), json!(w));
        }
        if let Some(pr) = lnx.proton {
            lnx_obj.insert("proton".into(), json!(pr));
        }
        lnx_obj.insert("xvfb".into(), json!(lnx.xvfb));
        if let Some(iface) = lnx.firewall_interface {
            lnx_obj.insert("firewallInterface".into(), json!(iface));
        }
        doc.insert("linux".into(), Value::Object(lnx_obj));
    }

    if args.fresh_start || existing.and_then(|c| c.database.as_ref()).is_some() {
        let prev_db = existing.and_then(|c| c.database.as_ref());
        let mut db_obj = Map::new();
        if let Some(engine) = prev_db.and_then(|d| d.engine.as_ref()) {
            db_obj.insert("engine".into(), json!(engine));
        }
        if let Some(h) = prev_db.and_then(|d| d.postgres_host.as_ref()) {
            db_obj.insert("postgresHost".into(), json!(h));
        }
        if let Some(p) = prev_db.and_then(|d| d.postgres_port) {
            db_obj.insert("postgresPort".into(), json!(p));
        }
        if let Some(u) = prev_db.and_then(|d| d.postgres_user.as_ref()) {
            db_obj.insert("postgresUser".into(), json!(u));
        }
        if let Some(db) = prev_db.and_then(|d| d.postgres_database.as_ref()) {
            db_obj.insert("postgresDatabase".into(), json!(db));
        }
        if let Some(ar) = prev_db.and_then(|d| d.allow_remote) {
            db_obj.insert("allowRemote".into(), json!(ar));
        }
        let fs_val = if args.fresh_start {
            Some(true)
        } else {
            prev_db.and_then(|d| d.fresh_start)
        };
        if let Some(fs) = fs_val {
            db_obj.insert("freshStart".into(), json!(fs));
        }
        if !db_obj.is_empty() {
            doc.insert("database".into(), Value::Object(db_obj));
        }
    }

    let parsed = HostConfig::from_json(&Value::Object(doc))?;
    let _ = multipliers_on(&parsed);
    Ok(parsed.written_over(existing))
}

pub fn prompt_stdio(label: &str, default_val: &str) -> Result<String, HostError> {
    let mut stdout = io::stdout();
    if default_val.is_empty() {
        write!(stdout, "{label}: ")?;
    } else {
        write!(stdout, "{label} [{default_val}]: ")?;
    }
    stdout.flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        Ok(default_val.to_owned())
    } else {
        Ok(trimmed.to_owned())
    }
}

pub fn detect_default_game_dir() -> Option<String> {
    let candidates = [
        r"C:\Program Files\Epic Games\Dauntless",
        r"D:\Program Files\Epic Games\Dauntless",
        r"D:\Games\Dauntless",
        r"C:\Games\Dauntless",
    ];
    for c in candidates {
        let p = Path::new(c);
        if p.join(game_exe_relative()).is_file() {
            let ver_file = p.join("Version.txt");
            if !ver_file.is_file()
                || std::fs::read_to_string(&ver_file)
                    .ok()
                    .is_some_and(|v| v.contains(GAME_VERSION_TEXT))
            {
                return Some(c.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn builds_default_private_config_without_tty() {
        let args = SetupArgs::default();
        let env = RealSetupPromptEnv {
            is_tty: false,
            platform: Platform::Windows,
            tailscale_ip: Some("100.101.102.103".to_owned()),
            detected_game_dir: None,
        };
        let cfg = resolve_options(&args, None, &env).unwrap();
        assert_eq!(cfg.mode, ServerMode::Private);
        assert_eq!(cfg.edition, Edition::Reborn);
        assert_eq!(cfg.host, "100.101.102.103");
        assert_eq!(cfg.game_address, "100.101.102.103");
        assert_eq!(cfg.registration, Registration::Invitecode);
    }

    #[test]
    fn public_mode_validates_host_and_game_address() {
        let args = SetupArgs {
            mode: Some("public".to_owned()),
            host: Some("play.example.com".to_owned()),
            game_address: Some("203.0.113.10".to_owned()),
            max_hunts: Some("5".to_owned()),
            ..SetupArgs::default()
        };
        let env = RealSetupPromptEnv {
            is_tty: false,
            platform: Platform::Linux,
            tailscale_ip: None,
            detected_game_dir: None,
        };
        let cfg = resolve_options(&args, None, &env).unwrap();
        assert_eq!(cfg.mode, ServerMode::Public);
        assert_eq!(cfg.host, "play.example.com");
        assert_eq!(cfg.game_address, "203.0.113.10");
        assert_eq!(cfg.max_hunt_servers, 5);
        assert!(cfg.multipliers.is_none());
        assert!(cfg.linux.is_some());
    }
}
