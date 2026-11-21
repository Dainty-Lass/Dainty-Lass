// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Windows lends blocks of its dynamic range (49152 to 65535) to Hyper-V, WSL and Docker, and a port inside one
//! can't be bound (os error 10013); the blocks move on every reboot. Setup and start read them from netsh and move
//! each port the server listens on that falls in one, writing the move to settings.toml's `[ports]` so it stays put:
//! the private gateway to [`ALTERNATE_GATEWAY`], everything else to its default or the next port free above it.

use crate::error::HostError;
use crate::hostconfig::ports::PortsConfig;
use crate::hostconfig::{HostConfig, ServerMode};
use crate::paths::{PORTS, Paths};

/// Where a private gateway goes when Windows has lent out 61000: below the dynamic range, beside the other services.
pub const ALTERNATE_GATEWAY: u16 = 41080;

/// One port moved out of a lent range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Move {
    /// Its `[ports]` key.
    pub key: &'static str,
    /// The port Windows holds.
    pub from: u16,
    /// Where it went.
    pub to: u16,
}

impl Move {
    /// The line setup and start print.
    #[must_use]
    pub fn line(&self) -> String {
        let Move { key, from, to } = *self;
        let invites = if key == "gateway" {
            format!(": invites made before name {from}, so make new ones")
        } else {
            String::new()
        };
        format!("- ports.{key}: Windows has lent {from} out (netsh's excluded port ranges), moved to {to}{invites}")
    }
}

/// The ranges in `netsh interface ipv4 show excludedportrange protocol=tcp`'s output: each line that starts with two
/// port numbers, whatever language the headings are in.
#[must_use]
pub fn parse_excluded(text: &str) -> Vec<(u16, u16)> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let first = words.next()?.parse::<u16>().ok()?;
            let last = words.next()?.parse::<u16>().ok()?;
            (first <= last).then_some((first, last))
        })
        .collect()
}

/// The TCP ranges Windows holds now; empty elsewhere, or when netsh can't say.
#[must_use]
pub fn excluded_tcp() -> Vec<(u16, u16)> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let args = ["interface", "ipv4", "show", "excludedportrange", "protocol=tcp"];
    crate::run::capture("netsh", &args, None)
        .ok()
        .flatten()
        .map_or_else(Vec::new, |text| parse_excluded(&text))
}

fn default_of(key: &str) -> u16 {
    match key {
        "gateway" => ALTERNATE_GATEWAY,
        "backend" => PORTS.backend,
        "director" => PORTS.director,
        "qos" => PORTS.qos,
        "content" => PORTS.content,
        "admin" => PORTS.admin,
        "allowlist" => PORTS.allowlist,
        _ => PORTS.chat,
    }
}

/// The moves `config` needs so no port it listens on is inside `excluded`, applied to its `[ports]`.
pub fn move_lent_ports(config: &mut HostConfig, excluded: &[(u16, u16)]) -> Vec<Move> {
    let lent = |port: u16| excluded.iter().any(|(first, last)| (*first..=*last).contains(&port));
    let mut ports = config.ports.unwrap_or_default();
    let listened = |key: &str| match key {
        "gateway" => config.mode == ServerMode::Private,
        "allowlist" => config.mode == ServerMode::Public,
        _ => true,
    };
    let mut moves = Vec::new();
    for (key, from) in ports.named() {
        if !listened(key) || !lent(from) {
            continue;
        }
        let taken = |port: u16, ports: &PortsConfig| {
            ports.named().iter().any(|(k, p)| *k != key && *p == port)
                || [PORTS.relay, PORTS.mongo, crate::hostconfig::gateway_port(config)].contains(&port)
                || (PORTS.game_lowest..=PORTS.game_last).contains(&port)
        };
        let mut to = default_of(key);
        while lent(to) || taken(to, &ports) {
            let Some(next) = to.checked_add(1) else {
                break;
            };
            to = next;
        }
        if lent(to) || taken(to, &ports) {
            continue;
        }
        if let Some(slot) = ports.slot(key) {
            *slot = Some(to);
        }
        moves.push(Move { key, from, to });
    }
    if !moves.is_empty() {
        config.ports = Some(ports);
    }
    moves
}

/// Start's check on Windows: moves the lent ports, saves settings.toml and rewrites the env files, printing each move.
pub fn settle(paths: &Paths, config: &mut HostConfig, say: &dyn Fn(&str)) -> Result<(), HostError> {
    let moves = move_lent_ports(config, &excluded_tcp());
    if moves.is_empty() {
        return Ok(());
    }
    for m in &moves {
        say(&m.line());
    }
    crate::hostconfig::save_host_config(paths, config)?;
    let (secrets, _) =
        crate::hostconfig::load_or_create_secrets(&paths.config, crate::commands::setup::secrets::new_secrets)?;
    let commit = crate::serve::source_commit(&paths.repo);
    let files =
        crate::hostconfig::build_env_files(config, &secrets, paths, &commit, crate::hostconfig::Platform::current());
    crate::hostconfig::write_env_files(paths, &files)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    /// The maintainer's PC on 2026-11-09, headings and all.
    const NETSH: &str = "
Protocol tcp Port Exclusion Ranges

Start Port    End Port
----------    --------
      5357        5357
     50000       50059     *
     60963       61062
     63541       63640

* - Administered port exclusions.
";

    fn config(mode: &str) -> HostConfig {
        let host = if mode == "private" { "100.64.0.1" } else { "203.0.113.7" };
        HostConfig::from_json(&serde_json::json!({
            "mode": mode,
            "host": host,
            "gameAddress": host,
            "name": "Night Hunts",
            "registration": "OPEN",
            "maxHuntServers": 4
        }))
        .unwrap()
    }

    #[test]
    fn reads_netsh_in_any_language() {
        assert_eq!(
            parse_excluded(NETSH),
            [(5357, 5357), (50000, 50059), (60963, 61062), (63541, 63640)]
        );
        assert!(parse_excluded("Protokoll tcp\n\nStartport Endport\n").is_empty());
    }

    #[test]
    fn a_private_gateway_in_a_lent_range_moves_to_the_alternate_and_stays_there() {
        let mut c = config("private");
        let moves = move_lent_ports(&mut c, &parse_excluded(NETSH));
        assert_eq!(
            moves,
            [Move {
                key: "gateway",
                from: 61000,
                to: ALTERNATE_GATEWAY
            }]
        );
        assert_eq!(crate::hostconfig::gateway_port(&c), ALTERNATE_GATEWAY);
        assert!(moves[0].line().contains("make new ones"), "{}", moves[0].line());
        assert!(move_lent_ports(&mut c, &parse_excluded(NETSH)).is_empty());
        let text = c.to_toml().unwrap();
        assert!(text.contains("gateway = 41080"), "{text}");
        assert_eq!(HostConfig::from_toml(&text).unwrap().ports, c.ports);
    }

    #[test]
    fn a_service_moves_past_a_lent_range_and_the_ports_already_taken() {
        let mut c = config("public");
        c.ports = Some(PortsConfig {
            backend: Some(61001),
            director: Some(PORTS.backend),
            ..PortsConfig::default()
        });
        let moves = move_lent_ports(&mut c, &[(60963, 61062), (41001, 41001)]);
        assert_eq!(
            moves,
            [Move {
                key: "backend",
                from: 61001,
                to: 41004
            }],
            "41000 is the Director's, 41001 lent, 41002 and 41003 content's and admin's"
        );
        assert!(move_lent_ports(&mut config("public"), &parse_excluded(NETSH)).is_empty());
    }
}
