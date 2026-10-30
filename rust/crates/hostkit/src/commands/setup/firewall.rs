// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::HostError;
use crate::hostconfig::{
    Edition, HostConfig, ServerMode, game_ports, gateway_port, legacy_game_ports,
    legacy_gateway_port,
};
use crate::paths::Paths;
use crate::run::{RunOptions, run};

pub const ALLOWLIST_RULE_NAME: &str = "Reforged Host Allowlist";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FirewallRules {
    pub applied: Vec<String>,
    pub failed: Vec<String>,
}

#[must_use]
pub fn firewall_script(config: &HostConfig) -> String {
    let mut lines = vec![
        "# Written by reforged-host setup. Run in an elevated PowerShell prompt.".to_owned(),
        "$ErrorActionPreference = 'Stop'".to_owned(),
    ];

    if config.edition.runs_reborn() {
        let gw = gateway_port(config);
        let (first, last) = game_ports(config);
        lines.push(format!(
            "netsh advfirewall firewall delete rule name=\"Reforged Gateway TCP {gw}\" >$null 2>&1"
        ));
        lines.push(format!(
            "netsh advfirewall firewall add rule name=\"Reforged Gateway TCP {gw}\" dir=in action=allow protocol=TCP localport={gw}"
        ));
        lines.push(format!(
            "netsh advfirewall firewall delete rule name=\"Reforged Game UDP {first}-{last}\" >$null 2>&1"
        ));
        lines.push(format!(
            "netsh advfirewall firewall add rule name=\"Reforged Game UDP {first}-{last}\" dir=in action=allow protocol=UDP localport={first}-{last}"
        ));
    }

    if config.edition.runs_legacy() {
        let gw = legacy_gateway_port(config);
        let (first, last) = legacy_game_ports(config);
        lines.push(format!(
            "netsh advfirewall firewall delete rule name=\"Reforged Legacy Gateway TCP {gw}\" >$null 2>&1"
        ));
        lines.push(format!(
            "netsh advfirewall firewall add rule name=\"Reforged Legacy Gateway TCP {gw}\" dir=in action=allow protocol=TCP localport={gw}"
        ));
        lines.push(format!(
            "netsh advfirewall firewall delete rule name=\"Reforged Legacy Game UDP {first}-{last}\" >$null 2>&1"
        ));
        lines.push(format!(
            "netsh advfirewall firewall add rule name=\"Reforged Legacy Game UDP {first}-{last}\" dir=in action=allow protocol=UDP localport={first}-{last}"
        ));
    }

    if config.mode == ServerMode::Public {
        lines.push(format!(
            "# Inbound UDP allowlist rule ({ALLOWLIST_RULE_NAME}) is managed dynamically by reforged-host allowlist."
        ));
    }

    let _ = Edition::Both;
    format!("{}\n", lines.join("\n"))
}

pub fn apply_windows_firewall_rules(
    config: &HostConfig,
    log_file: &Path,
) -> Result<FirewallRules, HostError> {
    let mut rules_to_apply: Vec<(String, String, String)> = Vec::new();
    if config.edition.runs_reborn() {
        let gw = gateway_port(config);
        let (first, last) = game_ports(config);
        rules_to_apply.push((
            format!("Reforged Gateway TCP {gw}"),
            "TCP".to_owned(),
            gw.to_string(),
        ));
        rules_to_apply.push((
            format!("Reforged Game UDP {first}-{last}"),
            "UDP".to_owned(),
            format!("{first}-{last}"),
        ));
    }
    if config.edition.runs_legacy() {
        let gw = legacy_gateway_port(config);
        let (first, last) = legacy_game_ports(config);
        rules_to_apply.push((
            format!("Reforged Legacy Gateway TCP {gw}"),
            "TCP".to_owned(),
            gw.to_string(),
        ));
        rules_to_apply.push((
            format!("Reforged Legacy Game UDP {first}-{last}"),
            "UDP".to_owned(),
            format!("{first}-{last}"),
        ));
    }

    let opts = RunOptions {
        cwd: log_file
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
        env: None,
        log_file: Some(log_file.to_path_buf()),
        quiet: true,
    };

    let mut out = FirewallRules::default();
    for (name, proto, port_spec) in rules_to_apply {
        let name_arg = format!("name={name}");
        let proto_arg = format!("protocol={proto}");
        let port_arg = format!("localport={port_spec}");
        let _ = run(
            "netsh",
            &[
                "advfirewall",
                "firewall",
                "delete",
                "rule",
                &name_arg,
            ],
            &opts,
        );
        match run(
            "netsh",
            &[
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &name_arg,
                "dir=in",
                "action=allow",
                &proto_arg,
                &port_arg,
            ],
            &opts,
        ) {
            Ok(res) if res.code == 0 => out.applied.push(name),
            _ => out.failed.push(name),
        }
    }
    Ok(out)
}

pub fn write_firewall_script(p: &Paths, config: &HostConfig) -> Result<PathBuf, HostError> {
    fs::create_dir_all(&p.config).map_err(|e| HostError::io(&p.config, e))?;
    if cfg!(windows) {
        let path = p.config.join("firewall-rules.ps1");
        let text = firewall_script(config);
        fs::write(&path, text.as_bytes()).map_err(|e| HostError::io(&path, e))?;
        Ok(path)
    } else {
        let path = p.config.join("firewall-rules.sh");
        let text = crate::linux::linux_firewall_script(config)?;
        fs::write(&path, text.as_bytes()).map_err(|e| HostError::io(&path, e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path)
                .map_err(|e| HostError::io(&path, e))?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).map_err(|e| HostError::io(&path, e))?;
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use serde_json::json;

    #[test]
    fn firewall_script_includes_gateway_and_game_ports() {
        let cfg = HostConfig::from_json(&json!({
            "mode": "public",
            "edition": "both",
            "host": "play.example.com",
            "gameAddress": "203.0.113.10",
            "name": "Public Hunts",
            "registration": "INVITECODE",
            "maxHuntServers": 4
        }))
        .unwrap();
        let script = firewall_script(&cfg);
        assert!(script.contains("localport=443"));
        assert!(script.contains("localport=8770-8777"));
        assert!(script.contains("localport=4443"));
        assert!(script.contains("localport=8780-8787"));
        assert!(script.contains(ALLOWLIST_RULE_NAME));
    }

    #[test]
    fn write_firewall_script_writes_config_file() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        let cfg = HostConfig::from_json(&json!({
            "mode": "private",
            "host": "100.64.0.1",
            "gameAddress": "100.64.0.1",
            "name": "Private Hunts",
            "registration": "INVITECODE",
            "maxHuntServers": 4
        }))
        .unwrap();
        let written = write_firewall_script(&paths, &cfg).unwrap();
        assert!(written.is_file());
    }
}
