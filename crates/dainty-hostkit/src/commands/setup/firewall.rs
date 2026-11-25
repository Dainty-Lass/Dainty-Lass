// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Windows Firewall rules for the gateway's port and the game ports, applied with netsh and written as a script.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::HostError;
use crate::hostconfig::{HostConfig, ServerMode, game_ports, gateway_port};
use crate::paths::{Paths, game_exe_relative};
use crate::run::{RunOptions, run};

/// The public server's game-port rule, made disabled: the allowlist opens the ports only for players who logged in.
pub const ALLOWLIST_RULE_NAME: &str = "DaintyLass-GamePorts-Allowlist";

/// What [`apply_windows_firewall_rules`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FirewallRules {
    /// The rules added.
    pub applied: Vec<String>,
    /// The rules netsh refused.
    pub failed: Vec<String>,
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The PowerShell script that adds the server's inbound rules, letting `gateway_program` through.
#[must_use]
pub fn firewall_script(config: &HostConfig, gateway_program: &str) -> String {
    let mut lines = vec![
        "# Dainty Lass: Windows Firewall rules for this server. Written by dainty-host setup.".to_owned(),
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

    let gw = gateway_port(config);
    let (first, last) = game_ports(config);
    if config.mode == ServerMode::Private {
        lines.push("# Only from Tailscale addresses: nobody else can reach these.".to_owned());
        lines.push(format!(
            "Add-Rule 'DaintyLass-Gateway-Tailnet' 'Dainty Lass gateway (TCP {gw}, Tailscale)' 'TCP' '{gw}' {gw_prog} '100.64.0.0/10' 'True'"
        ));
        lines.push(format!(
            "Add-Rule 'DaintyLass-GamePorts-Tailnet' 'Dainty Lass game ports (Tailscale)' 'UDP' '{first}-{last}' $null '100.64.0.0/10' 'True'"
        ));
    } else {
        lines.push(format!(
            "Add-Rule 'DaintyLass-Gateway' 'Dainty Lass gateway (TCP {gw})' 'TCP' '{gw}' {gw_prog} $null 'True'"
        ));
        lines.push(
            "# The game ports start closed: the allowlist helper opens them only for players who logged in.".to_owned(),
        );
        lines.push(format!(
            "Add-Rule '{ALLOWLIST_RULE_NAME}' 'Dainty Lass game ports (allowlist)' 'UDP' '{first}-{last}' $null $null 'False'"
        ));
    }

    if let Some(ref gd) = config.game_dir {
        let exe = Path::new(gd).join(game_exe_relative());
        let exe_q = ps_quote(&exe.to_string_lossy());
        lines.push(format!(
            "Add-Rule 'DaintyLass-GameServer' 'Dainty Lass game server' 'UDP' '{first}-{last}' {exe_q} $null 'True'"
        ));
    }

    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    out
}

/// Adds the inbound rules for the gateway's TCP port and the game ports with netsh, logging to `log_file`.
pub fn apply_windows_firewall_rules(config: &HostConfig, log_file: &Path) -> Result<FirewallRules, HostError> {
    let gw = gateway_port(config);
    let (first, last) = game_ports(config);
    let rules_to_apply = [
        (
            format!("Dainty Lass Gateway TCP {gw}"),
            "TCP".to_owned(),
            gw.to_string(),
        ),
        (
            format!("Dainty Lass Game UDP {first}-{last}"),
            "UDP".to_owned(),
            format!("{first}-{last}"),
        ),
    ];

    let opts = RunOptions {
        cwd: log_file.parent().unwrap_or_else(|| Path::new(".")).to_path_buf(),
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
            &["advfirewall", "firewall", "delete", "rule", &name_arg],
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

/// Writes the script as `firewall.ps1` in the config folder; returns its path.
pub fn write_firewall_script(p: &Paths, config: &HostConfig, gateway_program: &str) -> Result<PathBuf, HostError> {
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
