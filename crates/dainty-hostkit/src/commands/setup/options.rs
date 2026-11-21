// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Setup's options turned into a [`HostConfig`], each one left out keeping the file's value.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::cli::SetupArgs;
use crate::commands::setup::SetupEnv;
use crate::error::HostError;
use crate::hostconfig::discord::DiscordConfig;
use crate::hostconfig::{
    DEFAULT_MONGO_URI, DEFAULT_PERFORMANCE, HostConfig, LinuxOptions, Multipliers, PerformanceOptions, Platform,
    Registration, ServerMode, game_ports, is_dns_name, is_ipv4, is_public_ipv4, is_tailscale_ipv4, is_valid_share_url,
    linux_options, performance_options,
};
use crate::paths::{GAME_VERSION_TEXT, MAX_HUNTS, game_exe_relative};

/// The server's name on a first setup.
pub const DEFAULT_NAME: &str = "Dainty Lass server";
/// The most hunts at once on a first setup.
pub const DEFAULT_MAX_HUNT_SERVERS: u32 = 3;
/// The settings.toml version setup writes.
pub const HOST_CONFIG_VERSION: u32 = 1;

/// Fails on words setup did not expect, with a hint to quote values with spaces.
pub fn check_unexpected_args(args: &SetupArgs) -> Result<(), HostError> {
    if let Some(first) = args.unexpected.first() {
        return Err(HostError::message(format!(
            "unexpected {first:?}: put a value with spaces in quotes (for example, --game \"D:\\Games\\My Dauntless\")"
        )));
    }
    Ok(())
}

/// The public port settings.toml keeps: `None` for 443 or a private server; fails on a bad `--public-port`.
pub fn public_port_from_args(
    args: &SetupArgs,
    mode: ServerMode,
    existing: Option<&HostConfig>,
) -> Result<Option<f64>, HostError> {
    if let Some(ref raw) = args.public_port {
        if mode != ServerMode::Public {
            return Err(HostError::message("--public-port is for public servers only"));
        }
        let port = raw
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|&p| p >= 1)
            .ok_or_else(|| HostError::message(format!("--public-port must be a TCP port number (got {raw})")))?;
        if port == 443 {
            Ok(None)
        } else {
            Ok(Some(f64::from(port)))
        }
    } else if mode == ServerMode::Public {
        Ok(existing.and_then(|c| c.public_port).map(f64::from))
    } else {
        Ok(None)
    }
}

fn parse_ram_mb(flag: &str, raw: &str, min_mb: u32) -> Result<u32, HostError> {
    let trimmed = raw.trim();
    let num_part = trimmed
        .strip_suffix("MB")
        .or_else(|| trimmed.strip_suffix("mb"))
        .unwrap_or(trimmed)
        .trim();
    let mb = num_part
        .parse::<u32>()
        .ok()
        .filter(|&n| n >= min_mb && n <= 1_048_576)
        .ok_or_else(|| {
            HostError::message(format!(
                "{flag} must be a whole number of megabytes, at least {min_mb} (got {raw})"
            ))
        })?;
    Ok(mb)
}

/// The game-server options: `existing` (or the defaults) with the options given laid over; fails on a bad value.
pub fn performance_from_args(
    args: &SetupArgs,
    existing: Option<PerformanceOptions>,
) -> Result<PerformanceOptions, HostError> {
    let mut perf = existing.unwrap_or(DEFAULT_PERFORMANCE);

    if let Some(ref raw) = args.server_fps {
        let fps = raw
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|&n| n == 0 || (10..=240).contains(&n))
            .ok_or_else(|| {
                HostError::message(format!(
                    "--server-fps must be 0 (no cap) or 10 to 240 frames per second (got {raw})"
                ))
            })?;
        perf.server_fps = fps;
    }

    if let Some(ref raw) = args.idle_fps {
        let fps = raw.trim().parse::<u32>().ok().filter(|&n| n <= 240).ok_or_else(|| {
            HostError::message(format!(
                "--idle-fps must be 0 (the same as --server-fps) or 1 to 240 frames per second (got {raw})"
            ))
        })?;
        perf.idle_fps = fps;
    }

    if args.training_on_demand {
        perf.training_on_demand = true;
    }
    if args.training_at_boot {
        perf.training_on_demand = false;
    }

    if let Some(ref raw) = args.training_idle_minutes {
        let mins = raw.trim().parse::<u32>().ok().filter(|&n| n <= 1440).ok_or_else(|| {
            HostError::message(format!(
                "--training-idle-minutes must be 0 (keep them running) or 1 to 1440 minutes (got {raw})"
            ))
        })?;
        perf.training_idle_minutes = mins;
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

    if let Some(ref raw) = args.min_free_ram {
        perf.min_free_memory_mb = parse_ram_mb("--min-free-ram", raw, 0)?;
    }

    if let Some(ref raw) = args.hunt_ram {
        perf.hunt_memory_mb = parse_ram_mb("--hunt-ram", raw, 256)?;
    }

    Ok(perf)
}

/// The game-server settings in one line, as setup prints them.
#[must_use]
pub fn performance_summary(c: &HostConfig) -> String {
    let perf = performance_options(c);
    let fps_part = match (perf.server_fps, perf.idle_fps) {
        (0, 0) => "uncapped fps, no idle cap".to_owned(),
        (0, idle) => format!("uncapped fps, {idle} fps after 15 s without players"),
        (fps, 0) => format!("{fps} fps, no idle cap"),
        (fps, idle) => format!("{fps} fps, {idle} fps after 15 s without players"),
    };
    let training_part = if !perf.training_on_demand {
        "Training Grounds at boot".to_owned()
    } else if perf.training_idle_minutes == 0 {
        "Training Grounds on demand, kept running".to_owned()
    } else {
        format!(
            "Training Grounds on demand, stopped after {} min without players",
            perf.training_idle_minutes
        )
    };
    let logs_part = if perf.verbose_game_logs {
        "verbose game logs"
    } else {
        "quiet game logs"
    };
    let (first, last) = game_ports(c);
    let mem_part = if perf.memory_guard {
        format!(
            "a new hunt only with {} MB free",
            perf.min_free_memory_mb + perf.hunt_memory_mb
        )
    } else {
        "no memory guard".to_owned()
    };
    format!(
        "Game servers: {fps_part}; {training_part}; {logs_part}; at most {} hunts at once (UDP {first}-{last}), {mem_part}",
        c.max_hunt_servers
    )
}

fn is_valid_iface(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 15
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The Linux options: `existing` (or the defaults) with the options given laid over; fails on a bad value.
pub fn linux_from_args(
    args: &SetupArgs,
    existing: Option<&LinuxOptions>,
    find_on_path: &dyn Fn(&str) -> Result<Option<String>, HostError>,
) -> Result<LinuxOptions, HostError> {
    let mut lnx = existing.cloned().unwrap_or(LinuxOptions {
        wine: None,
        proton: None,
        xvfb: true,
        firewall_interface: None,
    });

    if let Some(ref raw) = args.wine {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            lnx.wine = None;
        } else if trimmed.contains('/') || trimmed.contains('\\') {
            lnx.wine = Some(trimmed.to_owned());
        } else if let Some(found) = find_on_path(trimmed)? {
            lnx.wine = Some(found);
        } else {
            return Err(HostError::message(format!("--wine {trimmed:?} was not found on PATH")));
        }
    }

    if args.no_proton {
        lnx.proton = None;
    } else if let Some(ref raw) = args.proton {
        let trimmed = raw.trim();
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
    } else if let Some(ref raw) = args.firewall_interface {
        let trimmed = raw.trim();
        if !is_valid_iface(trimmed) {
            return Err(HostError::message(format!(
                "--firewall-interface must be a network interface name like eth0 (got {raw})"
            )));
        }
        lnx.firewall_interface = Some(trimmed.to_owned());
    }

    Ok(lnx)
}

fn parse_game_dir(flag: &str, ver: &str, raw: &str) -> Result<String, HostError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('^') {
        return Err(HostError::message(format!(
            "{flag} needs the full path of the Dauntless {ver} folder, like \"D:\\Games\\Dauntless\" (got {raw})"
        )));
    }
    let bytes = trimmed.as_bytes();
    let is_win_abs = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let is_posix_abs = trimmed.starts_with('/');

    if !is_win_abs && !is_posix_abs {
        return Err(HostError::message(format!(
            "{flag} needs the full path of the Dauntless {ver} folder, like \"D:\\Games\\Dauntless\" (got {raw})"
        )));
    }

    if cfg!(windows) && is_posix_abs {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("C:\\"));
        let cwd_str = cwd.to_string_lossy();
        let drive = if cwd_str.len() >= 2 && cwd_str.as_bytes()[1] == b':' {
            &cwd_str[..2]
        } else {
            "C:"
        };
        let rest = trimmed.replace('/', "\\");
        Ok(format!("{drive}{rest}"))
    } else {
        Ok(trimmed.to_owned())
    }
}

fn parse_multiplier_arg(flag: &str, raw: &str) -> Result<f64, HostError> {
    let trimmed = raw.trim();
    let num_str = trimmed
        .strip_prefix('x')
        .or_else(|| trimmed.strip_prefix('X'))
        .unwrap_or(trimmed);
    let val = num_str
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && (1.0..=100.0).contains(n))
        .ok_or_else(|| HostError::message(format!("{flag} must be a number from 1 to 100 (got {raw})")))?;
    Ok(val)
}

/// The config setup writes: the options given over `existing`, checked as settings.toml is.
pub fn host_config_from_args(
    args: &SetupArgs,
    existing: Option<&HostConfig>,
    env: &dyn SetupEnv,
) -> Result<HostConfig, HostError> {
    check_unexpected_args(args)?;

    let mode = if let Some(ref m) = args.mode {
        ServerMode::parse(m.trim()).ok_or_else(|| {
            HostError::message(
                "say which kind of server: --mode private (friends over Tailscale) or --mode public (a server on the internet)",
            )
        })?
    } else if let Some(c) = existing {
        c.mode
    } else {
        return Err(HostError::message(
            "say which kind of server: --mode private (friends over Tailscale) or --mode public (a server on the internet)",
        ));
    };

    let ts_ip = env.tailscale_ipv4()?;
    let (host, game_address) = match mode {
        ServerMode::Private => {
            let h = if let Some(ref raw_h) = args.host {
                raw_h.trim().to_ascii_lowercase()
            } else if let Some(ip) = ts_ip.clone() {
                ip
            } else if let Some(c) = existing.filter(|c| c.mode == ServerMode::Private) {
                c.host.clone()
            } else {
                return Err(HostError::message(
                    "Tailscale is not running here, so this PC's Tailscale address is unknown. Start Tailscale, or pass --host <your Tailscale IP>",
                ));
            };
            if !is_tailscale_ipv4(&h) && !(is_dns_name(&h) && h.ends_with(".ts.net")) {
                return Err(HostError::message(format!(
                    "private mode needs a Tailscale 100.64.0.0/10 address or .ts.net name (got {h})"
                )));
            }
            let ga = if let Some(ref raw_ga) = args.game_address {
                raw_ga.trim().to_owned()
            } else if is_ipv4(&h) {
                h.clone()
            } else if let Some(ip) = ts_ip {
                ip
            } else if let Some(c) = existing.filter(|c| c.mode == ServerMode::Private) {
                c.game_address.clone()
            } else {
                return Err(HostError::message(
                    "the host is a DNS name: pass --game-address <your Tailscale IPv4> as well",
                ));
            };
            (h, ga)
        }
        ServerMode::Public => {
            let h = if let Some(ref raw_h) = args.host {
                raw_h.trim().to_ascii_lowercase()
            } else if let Some(c) = existing.filter(|c| c.mode == ServerMode::Public) {
                c.host.clone()
            } else {
                return Err(HostError::message(
                    "public mode needs --host <public IPv4 or domain name>",
                ));
            };
            if !is_public_ipv4(&h) && !is_dns_name(&h) {
                return Err(HostError::message(format!(
                    "public mode needs a public IPv4 address or DNS name (got {h})"
                )));
            }
            let ga = if let Some(ref raw_ga) = args.game_address {
                raw_ga.trim().to_owned()
            } else if is_public_ipv4(&h) {
                h.clone()
            } else if let Some(c) = existing.filter(|c| c.mode == ServerMode::Public && is_public_ipv4(&c.game_address))
            {
                c.game_address.clone()
            } else {
                return Err(HostError::message(
                    "the host is a DNS name: pass --game-address <the server's public IPv4> as well (the game servers announce an address, not a name)",
                ));
            };
            if !is_public_ipv4(&ga) {
                return Err(HostError::message(format!(
                    "gameAddress must be a public IPv4 address (got {ga})"
                )));
            }
            (h, ga)
        }
    };

    let name = if let Some(ref n) = args.name {
        n.trim().to_owned()
    } else if let Some(c) = existing {
        c.name.clone()
    } else {
        DEFAULT_NAME.to_owned()
    };

    let game_dir = if let Some(ref g) = args.game {
        Some(parse_game_dir("--game", "1.14.7", g)?)
    } else {
        existing.and_then(|c| c.game_dir.clone())
    };

    let registration = if let Some(ref r) = args.registration {
        Registration::parse(&r.trim().to_ascii_uppercase())
            .ok_or_else(|| HostError::message("registration must be INVITECODE, OPEN or NONE"))?
    } else if let Some(c) = existing {
        c.registration
    } else {
        Registration::Invitecode
    };

    let share_url = if mode == ServerMode::Public {
        None
    } else if let Some(ref sh) = args.share {
        let trimmed = sh.trim();
        if trimmed.is_empty() {
            None
        } else {
            if !is_valid_share_url(trimmed) {
                return Err(HostError::message(
                    "--share must be a https://login.tailscale.com/admin/invite/... link",
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
    } else if args.mongo_uri.is_some() && mongo_uri != DEFAULT_MONGO_URI {
        false
    } else if let Some(c) = existing {
        c.mongo.manage
    } else {
        true
    };

    let mongod = if !manage_mongo {
        None
    } else if let Some(ref m) = args.mongod {
        let trimmed = m.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    } else if let Some(existing_mongod) = existing.and_then(|c| c.mongo.mongod.clone()) {
        Some(existing_mongod)
    } else {
        env.find_mongod()?
    };

    let max_hunt_servers = if let Some(ref mh) = args.max_hunts {
        mh.trim()
            .parse::<u32>()
            .ok()
            .filter(|&n| (1..=MAX_HUNTS).contains(&n))
            .ok_or_else(|| {
                HostError::message("maxHuntServers must be 1 to 10 (UDP 8770-8777 up to 6 hunts, 8766-8777 at 10)")
            })?
    } else if let Some(c) = existing {
        c.max_hunt_servers
    } else {
        DEFAULT_MAX_HUNT_SERVERS
    };

    let public_port = public_port_from_args(args, mode, existing)?.map(|p| p as u16);

    if (args.xp_multiplier.is_some() || args.reward_multiplier.is_some()) && mode != ServerMode::Private {
        return Err(HostError::message(
            "XP and reward multipliers are for private (Tailscale) test servers only",
        ));
    }

    let multipliers = if mode != ServerMode::Private || args.no_multipliers {
        None
    } else {
        let prev = existing.and_then(|c| c.multipliers);
        let xp = match &args.xp_multiplier {
            Some(raw) => Some(parse_multiplier_arg("--xp-multiplier", raw)?),
            None => prev.map(|m| m.xp),
        };
        let rewards = match &args.reward_multiplier {
            Some(raw) => Some(parse_multiplier_arg("--reward-multiplier", raw)?),
            None => prev.map(|m| m.rewards),
        };
        if xp.is_some() || rewards.is_some() {
            let m = Multipliers {
                xp: xp.unwrap_or(1.0),
                rewards: rewards.unwrap_or(1.0),
            };
            if (m.xp - 1.0).abs() < f64::EPSILON && (m.rewards - 1.0).abs() < f64::EPSILON {
                None
            } else {
                Some(m)
            }
        } else {
            None
        }
    };

    let perf = performance_from_args(args, existing.and_then(|c| c.performance))?;

    let find_fn = |name: &str| env.find_on_path(name);
    let linux = if env.platform() == Platform::Linux {
        Some(linux_from_args(
            args,
            existing.and_then(|c| c.linux.as_ref()),
            &find_fn,
        )?)
    } else if let Some(prev_lnx) = existing.and_then(|c| c.linux.as_ref()) {
        Some(linux_from_args(args, Some(prev_lnx), &find_fn)?)
    } else {
        None
    };

    let mut doc = Map::new();
    doc.insert("version".into(), json!(HOST_CONFIG_VERSION));
    doc.insert("mode".into(), json!(mode.as_str()));
    doc.insert("host".into(), json!(host));
    doc.insert("gameAddress".into(), json!(game_address));
    doc.insert("name".into(), json!(name));
    doc.insert("gameDir".into(), json!(game_dir));
    doc.insert("registration".into(), json!(registration.as_str()));
    doc.insert("shareUrl".into(), json!(share_url));

    let mut mongo_obj = Map::new();
    mongo_obj.insert("uri".into(), json!(mongo_uri));
    mongo_obj.insert("manage".into(), json!(manage_mongo));
    mongo_obj.insert("mongod".into(), json!(mongod));
    doc.insert("mongo".into(), Value::Object(mongo_obj));

    doc.insert("maxHuntServers".into(), json!(max_hunt_servers));
    if let Some(pp) = public_port {
        doc.insert("publicPort".into(), json!(pp));
    }
    if let Some(m) = multipliers {
        doc.insert(
            "multipliers".into(),
            json!({
                "xp": m.xp,
                "rewards": m.rewards,
            }),
        );
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
    if let Some(lnx) = linux {
        doc.insert(
            "linux".into(),
            json!({
                "wine": lnx.wine,
                "proton": lnx.proton,
                "xvfb": lnx.xvfb,
                "firewallInterface": lnx.firewall_interface,
            }),
        );
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

    let mut parsed = HostConfig::from_json(&Value::Object(doc))?.written_over(existing);
    let _ = linux_options(&parsed);
    parsed.discord = discord_from_args(args, parsed.discord.take())?;
    Ok(parsed)
}

/// `[discord]` with setup's options over settings.toml's; untouched when none is given. A token turns the bot on.
pub fn discord_from_args(
    args: &SetupArgs,
    existing: Option<DiscordConfig>,
) -> Result<Option<DiscordConfig>, HostError> {
    let given = args.discord_bot_token
        || args.no_discord
        || !args.discord_guilds.is_empty()
        || args.discord_application_id.is_some()
        || args.discord_dashboard_url.is_some();
    if !given {
        return Ok(existing);
    }
    if args.discord_bot_token && args.no_discord {
        return Err(HostError::message(
            "--discord-bot-token turns the bot on and --no-discord off: pass one",
        ));
    }
    let mut table = existing.unwrap_or_default().to_json();
    if let Value::Object(map) = &mut table {
        if args.discord_bot_token {
            map.insert("enabled".into(), json!(true));
        }
        if args.no_discord {
            map.insert("enabled".into(), json!(false));
        }
        if !args.discord_guilds.is_empty() {
            map.insert("guildIds".into(), json!(args.discord_guilds));
        }
        if let Some(id) = &args.discord_application_id {
            map.insert("applicationId".into(), json!(id));
        }
        if let Some(url) = &args.discord_dashboard_url {
            map.insert("dashboardUrl".into(), json!(url));
        }
    }
    DiscordConfig::from_json(&table).map(Some)
}

/// The first common Dauntless folder that holds the pinned 1.14.7 build.
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
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn discord_options_lay_over_settings_toml() {
        let none = SetupArgs::default();
        assert_eq!(discord_from_args(&none, None).unwrap(), None);
        let kept = DiscordConfig {
            enabled: true,
            guild_ids: vec!["5".into()],
            ..DiscordConfig::default()
        };
        assert_eq!(
            discord_from_args(&none, Some(kept.clone())).unwrap(),
            Some(kept.clone())
        );

        let token = SetupArgs {
            discord_bot_token: true,
            discord_guilds: vec!["7".into(), "8".into()],
            ..SetupArgs::default()
        };
        let on = discord_from_args(&token, None).unwrap().unwrap();
        assert!(on.enabled);
        assert_eq!(on.guild_ids, ["7", "8"]);

        let off = SetupArgs {
            no_discord: true,
            ..SetupArgs::default()
        };
        let off = discord_from_args(&off, Some(kept)).unwrap().unwrap();
        assert!(!off.enabled);
        assert_eq!(off.guild_ids, ["5"], "turning it off keeps the rest");

        let both = SetupArgs {
            discord_bot_token: true,
            no_discord: true,
            ..SetupArgs::default()
        };
        assert!(discord_from_args(&both, None).is_err());
        let bad = SetupArgs {
            discord_guilds: vec!["guild".into()],
            ..SetupArgs::default()
        };
        assert!(discord_from_args(&bad, None).is_err());
    }
}
