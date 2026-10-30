// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::HostError;
use crate::hostconfig::{
    Edition, HostConfig, LEGACY_RULE_NAME, ServerMode, game_ports, gateway_port,
    legacy_game_ports, legacy_gateway_port,
};
use crate::paths::{Paths, game_exe_relative};
use crate::run::{RunOptions, run};

pub const ALLOWLIST_RULE_NAME: &str = "ReforgedRevival-GamePorts-Allowlist";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FirewallRules {
    pub applied: Vec<String>,
    pub failed: Vec<String>,
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[must_use]
pub fn firewall_script(config: &HostConfig, gateway_program: &str) -> String {
    let mut lines = vec![
        "# DaintyLass: Windows Firewall rules for this server. Written by `npm run setup`.".to_owned(),
        "# Run once in PowerShell as administrator:  powershell -ExecutionPolicy Bypass -File firewall.ps1".to_owned(),
        "$ErrorActionPreference = 'Stop'".to_owned(),
        "function Add-Rule($Name, $Display, $Protocol, $Port, $Program, $Remote, $Enabled) {".to_owned(),
        "    if (Get-NetFirewallRule -Name $Name -ErrorAction SilentlyContinue) { Write-Host \"$Display already exists\"; return }".to_owned(),
        "    $Params = @{ Name = $Name; DisplayName = $Display; Direction = 'Inbound'; Action = 'Allow'; Protocol = $Protocol; LocalPort = $Port; Profile = 'Any'; Enabled = $Enabled }".to_owned(),
        "    if ($Program) { $Params.Program = $Program }".to_owned(),
        "    if ($Remote) { $Params.RemoteAddress = $Remote }".to_owned(),
        "    New-NetFirewallRule @Params | Out-Null".to_owned(),
        "    Write-Host \"added $Display\"".to_owned(),
        "}".to_owned(),
    ];

    let gw_prog = ps_quote(gateway_program);

    if config.mode == ServerMode::Private {
        lines.push("# Only from Tailscale addresses: nobody else can reach these.".to_owned());
        if config.edition.runs_reborn() || config.edition == Edition::Legacy {
            let gw = if config.edition == Edition::Legacy {
                legacy_gateway_port(config)
            } else {
                gateway_port(config)
            };
            let (first, last) = if config.edition == Edition::Legacy {
                legacy_game_ports(config)
            } else {
                game_ports(config)
            };
            lines.push(format!(
                "Add-Rule 'ReforgedRevival-Gateway-Tailnet' 'Dauntless Reforged Revival gateway (TCP {gw}, Tailscale)' 'TCP' '{gw}' {gw_prog} '100.64.0.0/10' 'True'"
            ));
            lines.push(format!(
                "Add-Rule 'ReforgedRevival-GamePorts-Tailnet' 'Dauntless Reforged Revival game ports (Tailscale)' 'UDP' '{first}-{last}' $null '100.64.0.0/10' 'True'"
            ));
        }
        if config.edition == Edition::Both {
            let gw = legacy_gateway_port(config);
            let (first, last) = legacy_game_ports(config);
            lines.push(format!(
                "Add-Rule 'ReforgedRevival-Gateway-Legacy' 'Dauntless Reforged Revival legacy gateway (TCP {gw}, Tailscale)' 'TCP' '{gw}' {gw_prog} '100.64.0.0/10' 'True'"
            ));
            lines.push(format!(
                "Add-Rule 'ReforgedRevival-GamePorts-Legacy-Tailnet' 'Dauntless Reforged Revival legacy game ports (Tailscale)' 'UDP' '{first}-{last}' $null '100.64.0.0/10' 'True'"
            ));
        }
    } else {
        if config.edition.runs_reborn() || config.edition == Edition::Legacy {
            let gw = if config.edition == Edition::Legacy {
                legacy_gateway_port(config)
            } else {
                gateway_port(config)
            };
            let (first, last) = if config.edition == Edition::Legacy {
                legacy_game_ports(config)
            } else {
                game_ports(config)
            };
            lines.push(format!(
                "Add-Rule 'ReforgedRevival-Gateway' 'Dauntless Reforged Revival gateway (TCP {gw})' 'TCP' '{gw}' {gw_prog} $null 'True'"
            ));
            lines.push("# The game ports start closed: the allowlist helper opens them only for players who logged in.".to_owned());
            lines.push(format!(
                "Add-Rule '{ALLOWLIST_RULE_NAME}' 'Dauntless Reforged Revival game ports (allowlist)' 'UDP' '{first}-{last}' $null $null 'False'"
            ));
        }
        if config.edition == Edition::Both {
            let gw = legacy_gateway_port(config);
            let (first, last) = legacy_game_ports(config);
            lines.push(format!(
                "Add-Rule 'ReforgedRevival-Gateway-Legacy' 'Dauntless Reforged Revival legacy gateway (TCP {gw})' 'TCP' '{gw}' {gw_prog} $null 'True'"
            ));
            lines.push(format!(
                "Add-Rule '{LEGACY_RULE_NAME}' 'Dauntless Reforged Revival legacy game ports (allowlist)' 'UDP' '{first}-{last}' $null $null 'False'"
            ));
        }
    }

    if let Some(ref gd) = config.game_dir {
        let exe = Path::new(gd).join(game_exe_relative());
        let exe_q = ps_quote(&exe.to_string_lossy());
        let (first, last) = game_ports(config);
        lines.push(format!(
            "Add-Rule 'ReforgedRevival-GameServer' 'Dauntless Reforged Revival game server' 'UDP' '{first}-{last}' {exe_q} $null 'True'"
        ));
    }

    if let Some(ref lgd) = config.legacy_game_dir {
        let exe = Path::new(lgd).join(game_exe_relative());
        let exe_q = ps_quote(&exe.to_string_lossy());
        let (first, last) = legacy_game_ports(config);
        lines.push(format!(
            "Add-Rule 'ReforgedRevival-GameServer-Legacy' 'Dauntless Reforged Revival legacy game server' 'UDP' '{first}-{last}' {exe_q} $null 'True'"
        ));
    }

    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    out
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

pub fn write_firewall_script(
    p: &Paths,
    config: &HostConfig,
    gateway_program: &str,
) -> Result<PathBuf, HostError> {
    fs::create_dir_all(&p.config).map_err(|e| HostError::io(&p.config, e))?;
    let ps_path = p.config.join("firewall.ps1");
    let ps_text = firewall_script(config, gateway_program);
    fs::write(&ps_path, ps_text.as_bytes()).map_err(|e| HostError::io(&ps_path, e))?;

    if !cfg!(windows) {
        let sh_path = p.config.join("firewall.sh");
        let sh_text = crate::linux::linux_firewall_script(config)?;
        fs::write(&sh_path, sh_text.as_bytes()).map_err(|e| HostError::io(&sh_path, e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&sh_path)
                .map_err(|e| HostError::io(&sh_path, e))?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&sh_path, perms).map_err(|e| HostError::io(&sh_path, e))?;
        }
    }
    Ok(ps_path)
}
