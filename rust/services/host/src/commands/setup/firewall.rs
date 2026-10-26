//! `.workspace/config/firewall.ps1` (`firewallScript()` in `cli.ts`), byte for byte as the Node kit
//! writes it: the Windows Firewall rules a host adds once, as administrator.

use std::path::Path;

use crate::hostconfig::{HostConfig, ServerMode, game_ports_text, gateway_port};
use crate::paths::{PORTS, game_exe_relative};

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// `program`: what listens on the gateway's port (the Node kit's `node.exe`).
pub fn firewall_script(c: &HostConfig, program: &str) -> String {
    let exe = c
        .game_dir
        .as_deref()
        .map(|dir| Path::new(dir).join(game_exe_relative()).to_string_lossy().into_owned());
    let exe = exe.as_deref().map_or_else(|| "$null".to_owned(), quote);
    let ports = game_ports_text(c, "-");
    let mut lines: Vec<String> = [
        "# Dauntless Reforged Revival: Windows Firewall rules for this server. Written by `npm run setup`.",
        "# Run once in PowerShell as administrator:  powershell -ExecutionPolicy Bypass -File firewall.ps1",
        "$ErrorActionPreference = 'Stop'",
        "function Add-Rule($Name, $Display, $Protocol, $Port, $Program, $Remote, $Enabled) {",
        "    if (Get-NetFirewallRule -Name $Name -ErrorAction SilentlyContinue) { Write-Host \"$Display already exists\"; return }",
        "    $Params = @{ Name = $Name; DisplayName = $Display; Direction = 'Inbound'; Action = 'Allow'; Protocol = $Protocol; LocalPort = $Port; Profile = 'Any'; Enabled = $Enabled }",
        "    if ($Program) { $Params.Program = $Program }",
        "    if ($Remote) { $Params.RemoteAddress = $Remote }",
        "    New-NetFirewallRule @Params | Out-Null",
        "    Write-Host \"added $Display\"",
        "}",
    ]
    .map(str::to_owned)
    .to_vec();
    let program = quote(program);
    if c.mode == ServerMode::Public {
        let port = gateway_port(c);
        lines.push(format!(
            "Add-Rule 'ReforgedRevival-Gateway' 'Dauntless Reforged Revival gateway (TCP {port})' 'TCP' '{port}' {program} $null 'True'"
        ));
        lines.push(
            "# The game ports start closed: the allowlist helper opens them only for players who logged in.".into(),
        );
        lines.push(format!(
            "Add-Rule 'ReforgedRevival-GamePorts-Allowlist' 'Dauntless Reforged Revival game ports (allowlist)' 'UDP' '{ports}' {exe} $null 'False'"
        ));
    } else {
        let port = PORTS.private_gateway;
        lines.push("# Only from Tailscale addresses: nobody else can reach these.".into());
        lines.push(format!(
            "Add-Rule 'ReforgedRevival-Gateway-Tailnet' 'Dauntless Reforged Revival gateway (TCP {port}, Tailscale)' 'TCP' '{port}' {program} '100.64.0.0/10' 'True'"
        ));
        lines.push(format!(
            "Add-Rule 'ReforgedRevival-GamePorts-Tailnet' 'Dauntless Reforged Revival game ports (Tailscale)' 'UDP' '{ports}' {exe} '100.64.0.0/10' 'True'"
        ));
    }
    // A server running both games: the 1.4.4 gateway, and its game ports for its own allowlist's rule (RD26)
    if c.edition == crate::hostconfig::Edition::Both {
        let port = crate::hostconfig::legacy_gateway_port(c);
        let (first, last) = crate::hostconfig::legacy_game_ports(c);
        let legacy_exe = c
            .legacy_game_dir
            .as_deref()
            .map(|dir| quote(&Path::new(dir).join(game_exe_relative()).to_string_lossy()))
            .unwrap_or_else(|| "$null".to_owned());
        if c.mode == ServerMode::Public {
            lines.push(format!(
                "Add-Rule 'ReforgedRevival-Gateway-Legacy' 'Dauntless Reforged Revival 1.4.4 gateway (TCP {port})' 'TCP' '{port}' {program} $null 'True'"
            ));
            lines.push(format!(
                "Add-Rule 'ReforgedRevival-GamePorts-Allowlist-Legacy' '{}' 'UDP' '{first}-{last}' {legacy_exe} $null 'False'",
                crate::hostconfig::LEGACY_RULE_NAME
            ));
        } else {
            lines.push(format!(
                "Add-Rule 'ReforgedRevival-Gateway-Legacy-Tailnet' 'Dauntless Reforged Revival 1.4.4 gateway (TCP {port}, Tailscale)' 'TCP' '{port}' {program} '100.64.0.0/10' 'True'"
            ));
            lines.push(format!(
                "Add-Rule 'ReforgedRevival-GamePorts-Legacy-Tailnet' 'Dauntless Reforged Revival 1.4.4 game ports (Tailscale)' 'UDP' '{first}-{last}' {legacy_exe} '100.64.0.0/10' 'True'"
            ));
        }
    }
    lines.join("\r\n") + "\r\n"
}
