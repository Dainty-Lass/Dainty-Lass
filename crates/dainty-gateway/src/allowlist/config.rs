// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The helper's settings. It runs with the most rights of any server part, so it listens on loopback only, takes
//! only its own `ALLOWLIST_*` names, and checks every value that could reach a firewall.

use std::path::PathBuf;
use std::time::Duration;

use dainty_service_kit::env::Env;

use super::nft::{NFT_TABLE, check_interface, check_nft_binary, check_nft_table, default_nft};
pub use super::ports::check_ports;
use crate::config::{absolute, check_secret, optional, whole};
use crate::json_string;

/// Which firewall the helper drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallKind {
    /// Windows Firewall, through its COM API.
    Windows,
    /// Linux nftables, through the `nft` binary.
    Nftables,
}

impl FirewallKind {
    /// The name `ALLOWLIST_FIREWALL` takes.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Nftables => "nftables",
        }
    }

    /// `/status`'s backend name.
    pub fn backend_name(self) -> &'static str {
        match self {
            Self::Windows => "windows-firewall",
            Self::Nftables => "linux-nftables",
        }
    }
}

/// The helper's settings, from its `ALLOWLIST_*` variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowlistConfig {
    /// Only log what would change (`ALLOWLIST_DRY_RUN=1`); the variable must be set to `1` or `0`.
    pub dry_run: bool,
    /// Loopback address to listen on, `127.0.0.1` or `::1`.
    pub bind: String,
    /// The firewall to drive.
    pub firewall: FirewallKind,
    /// Absolute path of the `nft` binary.
    pub nft: String,
    /// nftables table the helper owns.
    pub nft_table: String,
    /// Network interface the nftables rules match, if any.
    pub interface: Option<String>,
    /// Port to listen on; 61005 by default.
    pub port: u16,
    /// Shared secret the gateway sends.
    pub secret: String,
    /// Admit private and loopback addresses too (`ALLOWLIST_ALLOW_PRIVATE=1`).
    pub allow_private: bool,
    /// How long an admitted address stays; 10 minutes by default.
    pub ttl: Duration,
    /// Shortest time between two firewall changes; 3 s by default.
    pub min_interval: Duration,
    /// Most addresses admitted at once; 256 by default.
    pub max_entries: usize,
    /// `ALLOWLIST_PORTS`, normalised
    pub ports: String,
    /// Audit log file, absolute.
    pub audit_log: PathBuf,
    /// State file restored at start, absolute.
    pub state_file: PathBuf,
    /// Windows Firewall rule display name, starting `Dainty Lass `.
    pub rule_name: String,
}

const RULE_NAME_PREFIXES: [&str; 2] = ["Dainty Lass ", "Dauntless Reforged Revival "];

/// The Windows Firewall rule this helper owns: `Dainty Lass ...` (or the legacy prefix), plain characters only.
pub fn check_rule_name(name: &str) -> Result<String, String> {
    let ok = RULE_NAME_PREFIXES.iter().any(|p| name.starts_with(p))
        && name.len() <= 80
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'(' | b')' | b'-' | b'.' | b'_'));
    if ok {
        Ok(name.to_owned())
    } else {
        Err(format!(
            "ALLOWLIST_RULE_NAME must start with \"Dainty Lass \" (or legacy \"Dauntless Reforged Revival \") and hold only letters, digits, spaces and ()-._, at most 80 (got {})",
            json_string(name)
        ))
    }
}

impl AllowlistConfig {
    /// Reads and checks every setting; the error names the first bad one.
    pub fn from_env(env: &Env) -> Result<Self, String> {
        let dry_run = match optional(env, "ALLOWLIST_DRY_RUN") {
            Some("1") => true,
            Some("0") => false,
            _ => {
                return Err("ALLOWLIST_DRY_RUN must be set: 1 only logs what it would do (any test machine), 0 really changes the firewall (the server)".to_owned());
            }
        };
        let bind = optional(env, "ALLOWLIST_BIND").unwrap_or("127.0.0.1").to_owned();
        if bind != "127.0.0.1" && bind != "::1" {
            return Err("ALLOWLIST_BIND must be 127.0.0.1 or ::1: the helper runs with administrator rights and only the gateway on this machine may reach it".to_owned());
        }
        let firewall = match optional(env, "ALLOWLIST_FIREWALL").unwrap_or("windows") {
            "windows" => FirewallKind::Windows,
            "nftables" => FirewallKind::Nftables,
            other => {
                return Err(format!(
                    "ALLOWLIST_FIREWALL must be windows (Windows Firewall) or nftables (Linux), not {}",
                    json_string(other)
                ));
            }
        };
        let nft = check_nft_binary(&optional(env, "ALLOWLIST_NFT").map_or_else(default_nft, str::to_owned))?;
        let nft_table = check_nft_table(optional(env, "ALLOWLIST_NFT_TABLE").unwrap_or(NFT_TABLE))?;
        let interface = optional(env, "ALLOWLIST_INTERFACE").map(check_interface).transpose()?;
        let port = whole(env, "ALLOWLIST_PORT", 61005, 1, 65535)? as u16;
        let secret = check_secret(env.get("ALLOWLIST_SECRET"), "ALLOWLIST_SECRET")?;
        let allow_private = optional(env, "ALLOWLIST_ALLOW_PRIVATE") == Some("1");
        let ttl = Duration::from_secs(whole(env, "ALLOWLIST_TTL_SECONDS", 600, 1, 86_400)?);
        let min_interval = Duration::from_millis(whole(env, "ALLOWLIST_MIN_INTERVAL_MS", 3000, 50, 600_000)?);
        let max_entries = whole(env, "ALLOWLIST_MAX_ENTRIES", 256, 1, 1000)? as usize;
        let ports = check_ports(optional(env, "ALLOWLIST_PORTS").unwrap_or("8770-8777"))?;
        let audit_log = absolute(optional(env, "ALLOWLIST_AUDIT_LOG").unwrap_or("allowlist-audit.log"));
        let state_file = absolute(optional(env, "ALLOWLIST_STATE_FILE").unwrap_or("allowlist-state.json"));
        let rule_name =
            check_rule_name(optional(env, "ALLOWLIST_RULE_NAME").unwrap_or(super::firewall::RULE_DISPLAY_NAME))?;
        Ok(Self {
            dry_run,
            bind,
            firewall,
            nft,
            nft_table,
            interface,
            port,
            secret,
            allow_private,
            ttl,
            min_interval,
            max_entries,
            ports,
            audit_log,
            state_file,
            rule_name,
        })
    }

    /// `<bind>:<port>`, with brackets for IPv6.
    pub fn listen_address(&self) -> String {
        crate::config::Endpoint::new(self.bind.clone(), self.port).authority()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn config(pairs: &[(&str, &str)]) -> Result<AllowlistConfig, String> {
        let mut all = vec![("ALLOWLIST_DRY_RUN", "1"), ("ALLOWLIST_SECRET", SECRET)];
        all.extend_from_slice(pairs);
        AllowlistConfig::from_env(&Env::from_pairs(all))
    }

    #[test]
    fn defaults() {
        let c = config(&[]).unwrap();
        assert!(c.dry_run && !c.allow_private);
        assert_eq!(
            (c.bind.as_str(), c.port, c.firewall),
            ("127.0.0.1", 61005, FirewallKind::Windows)
        );
        assert_eq!(
            (c.ttl.as_secs(), c.min_interval.as_millis(), c.max_entries),
            (600, 3000, 256)
        );
        assert_eq!(c.ports, "8770-8777");
        assert_eq!(c.nft_table, "reforged_revival");
        assert_eq!(c.rule_name, "Dainty Lass game ports (allowlist)");
        assert!(c.audit_log.is_absolute() && c.state_file.ends_with("allowlist-state.json"));
        assert!(c.nft.starts_with('/'));
    }

    #[test]
    fn errors() {
        let dry = AllowlistConfig::from_env(&Env::from_pairs([("ALLOWLIST_SECRET", SECRET)])).unwrap_err();
        assert!(dry.starts_with("ALLOWLIST_DRY_RUN must be set"));
        assert!(
            config(&[("ALLOWLIST_DRY_RUN", "true")])
                .unwrap_err()
                .starts_with("ALLOWLIST_DRY_RUN must be set")
        );
        assert!(
            config(&[("ALLOWLIST_BIND", "localhost")])
                .unwrap_err()
                .starts_with("ALLOWLIST_BIND must be")
        );
        assert!(config(&[("ALLOWLIST_BIND", "::1")]).is_ok());
        assert_eq!(
            config(&[("ALLOWLIST_FIREWALL", "pf")]).unwrap_err(),
            "ALLOWLIST_FIREWALL must be windows (Windows Firewall) or nftables (Linux), not \"pf\""
        );
        assert_eq!(
            config(&[("ALLOWLIST_NFT", "nft")]).unwrap_err(),
            "ALLOWLIST_NFT must be the absolute path of nft (the helper never looks it up in PATH)"
        );
        assert!(config(&[("ALLOWLIST_NFT_TABLE", "1t")]).is_err());
        assert!(config(&[("ALLOWLIST_INTERFACE", "eth0:1")]).is_err());
        assert_eq!(
            config(&[("ALLOWLIST_SECRET", "")]).unwrap_err(),
            "ALLOWLIST_SECRET is not set"
        );
        assert_eq!(
            config(&[("ALLOWLIST_TTL_SECONDS", "0")]).unwrap_err(),
            "ALLOWLIST_TTL_SECONDS must be a whole number from 1 to 86400"
        );
        assert!(config(&[("ALLOWLIST_PORTS", "8770,8771")]).is_err());
        assert!(config(&[("ALLOWLIST_POWERSHELL", "x")]).is_ok());
    }

    #[test]
    fn rule_names() {
        assert!(check_rule_name("Dainty Lass 1.4.4 game ports (allowlist)").is_ok());
        assert!(check_rule_name("Dauntless Reforged Revival game ports").is_ok());
        let long = format!("Dainty Lass {}", "a".repeat(70));
        for bad in [
            "Dainty Lass",
            "Dainty Lass *",
            "Dainty Lass \"x\"",
            "Dainty Lass a\nb",
            "Dainty Lass \u{f6}",
            long.as_str(),
        ] {
            assert!(check_rule_name(bad).is_err(), "{bad:?}");
        }
    }
}
