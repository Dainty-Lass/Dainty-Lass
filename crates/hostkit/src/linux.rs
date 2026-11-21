// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::HostError;
use crate::hostconfig::{HostConfig, LinuxOptions, ServerMode, game_ports, gateway_port};
use crate::paths::Paths;
use crate::run::{RunOptions, capture, run};

pub const UNIT_SERVER: &str = "reforged.service";
pub const UNIT_ALLOWLIST: &str = "reforged-allowlist.service";
pub const CAP_NET_BIND_SERVICE: u32 = 10;
pub const CAP_NET_ADMIN: u32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeKind {
    Wine,
    Proton,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRuntime {
    pub kind: RuntimeKind,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinePrefix {
    pub ok: bool,
    pub created: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitContext {
    pub user: String,
    pub host_bin: PathBuf,
}

pub fn effective_capabilities(status_text: &str) -> u64 {
    for line in status_text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("CapEff:") {
            let hex = rest.trim();
            if let Ok(mask) = u64::from_str_radix(hex, 16) {
                return mask;
            }
        }
    }
    0
}

pub fn has_capability(bit: u32, status_text: Option<&str>) -> bool {
    if bit >= 64 {
        return false;
    }
    let owned;
    let text = match status_text {
        Some(s) => s,
        None => {
            let Ok(s) = fs::read_to_string("/proc/self/status") else {
                return false;
            };
            owned = s;
            &owned
        }
    };
    (effective_capabilities(text) & (1u64 << bit)) != 0
}

fn parse_effective_uid(status_text: &str) -> Option<u32> {
    for line in status_text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Uid:") {
            let mut parts = rest.split_whitespace();
            let _real = parts.next()?;
            let eff = parts.next()?;
            return eff.parse::<u32>().ok();
        }
    }
    None
}

pub fn is_root() -> Result<bool, HostError> {
    if let Ok(status) = fs::read_to_string("/proc/self/status")
        && let Some(euid) = parse_effective_uid(&status)
    {
        return Ok(euid == 0);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(meta) = fs::metadata("/proc/self") {
            return Ok(meta.uid() == 0);
        }
    }
    if let Ok(Some(uid_str)) = capture("id", &["-u"], None)
        && let Ok(uid) = uid_str.trim().parse::<u32>()
    {
        return Ok(uid == 0);
    }
    Ok(false)
}

pub fn can_bind_port_with(port: u16, root: bool, status: Option<&str>, unprivileged_start: Option<&str>) -> bool {
    if port >= 1024 || root {
        return true;
    }
    if let Some(raw) = unprivileged_start
        && let Ok(start) = raw.trim().parse::<u32>()
        && u32::from(port) >= start
    {
        return true;
    }
    has_capability(CAP_NET_BIND_SERVICE, status)
}

pub fn can_bind_port(port: u16) -> Result<bool, HostError> {
    if port >= 1024 {
        return Ok(true);
    }
    let root = is_root()?;
    let status = fs::read_to_string("/proc/self/status").ok();
    let unpriv = fs::read_to_string("/proc/sys/net/ipv4/ip_unprivileged_port_start").ok();
    Ok(can_bind_port_with(port, root, status.as_deref(), unpriv.as_deref()))
}

pub fn can_change_firewall() -> Result<bool, HostError> {
    Ok(is_root()? || has_capability(CAP_NET_ADMIN, None))
}

pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn gameserver_wrapper_script(host_bin: &Path) -> String {
    let quoted = sh_quote(&host_bin.to_string_lossy());
    format!(
        "#!/bin/sh\n# Written by reforged-host setup. Launches Dauntless-Win64-Shipping.exe under Wine or Proton.\nexec {quoted} gameserver \"$@\"\n"
    )
}

pub fn write_gameserver_wrapper(paths: &Paths) -> Result<PathBuf, HostError> {
    let host_bin = resolve_host_bin(paths)?;
    fs::create_dir_all(&paths.bin)?;
    let script = gameserver_wrapper_script(&host_bin);
    fs::write(&paths.gameserver_wrapper, script.as_bytes())?;
    set_executable_mode(&paths.gameserver_wrapper)?;
    Ok(paths.gameserver_wrapper.clone())
}

fn set_executable_mode(path: &Path) -> Result<(), HostError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

pub fn game_runtime(options: &LinuxOptions) -> Result<Option<GameRuntime>, HostError> {
    if let Some(proton) = options.proton.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if crate::wine::is_executable(Path::new(proton)) {
            return Ok(Some(GameRuntime {
                kind: RuntimeKind::Proton,
                path: proton.to_owned(),
            }));
        }
        return Ok(None);
    }
    if let Some(wine) = options.wine.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if crate::wine::is_executable(Path::new(wine)) {
            return Ok(Some(GameRuntime {
                kind: RuntimeKind::Wine,
                path: wine.to_owned(),
            }));
        }
        return Ok(None);
    }
    let path_var = std::env::var("PATH").unwrap_or_default();
    Ok(crate::wine::find_wine(&path_var)?.map(|path| GameRuntime {
        kind: RuntimeKind::Wine,
        path,
    }))
}

pub fn ensure_wine_prefix(paths: &Paths, options: &LinuxOptions, log_file: &Path) -> Result<WinePrefix, HostError> {
    if options.proton.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        fs::create_dir_all(&paths.proton_data)?;
        return Ok(WinePrefix {
            ok: true,
            created: false,
            detail: Some("Proton manages its own prefix".to_owned()),
        });
    }
    if crate::wine::wine_prefix_ready(&paths.wine_prefix) {
        return Ok(WinePrefix {
            ok: true,
            created: false,
            detail: None,
        });
    }
    let Some(runtime) = game_runtime(options)? else {
        return Ok(WinePrefix {
            ok: false,
            created: false,
            detail: Some("Wine is not installed".to_owned()),
        });
    };
    fs::create_dir_all(&paths.wine_prefix)?;

    let wine_path = Path::new(&runtime.path);
    let sibling_wineboot = wine_path.parent().map(|dir| dir.join("wineboot"));
    let (cmd, args): (String, Vec<&str>) =
        if let Some(wb) = sibling_wineboot.as_ref().filter(|p| crate::wine::is_executable(p)) {
            (wb.to_string_lossy().into_owned(), vec!["--init"])
        } else if let Some(wb) = crate::wine::find_on_path(&["wineboot"])? {
            (wb, vec!["--init"])
        } else {
            (runtime.path.clone(), vec!["wineboot", "--init"])
        };

    let mut env: Vec<(String, String)> = std::env::vars().collect();
    env.push((
        "WINEPREFIX".to_owned(),
        paths.wine_prefix.to_string_lossy().into_owned(),
    ));
    env.push(("WINEDEBUG".to_owned(), "-all".to_owned()));
    env.push(("WINEDLLOVERRIDES".to_owned(), "mscoree=d;mshtml=d".to_owned()));

    let res = run(
        &cmd,
        &args,
        &RunOptions {
            cwd: paths.repo.clone(),
            env: Some(env),
            log_file: Some(log_file.to_path_buf()),
            quiet: true,
        },
    )?;

    let ready = res.code == 0 && crate::wine::wine_prefix_ready(&paths.wine_prefix);
    Ok(WinePrefix {
        ok: ready,
        created: ready,
        detail: if ready {
            None
        } else {
            res.last_line
                .or_else(|| Some(format!("wineboot exited with code {}", res.code)))
        },
    })
}

pub fn unit_word(s: &str) -> Result<String, HostError> {
    let trimmed = s.trim();
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(HostError::message(format!("unsafe systemd unit word: {s:?}")));
    }
    Ok(trimmed.to_owned())
}

pub fn unit_path(s: &str) -> Result<String, HostError> {
    let trimmed = s.trim();
    if trimmed.is_empty()
        || (!trimmed.starts_with('/') && !Path::new(trimmed).is_absolute())
        || trimmed.chars().any(|c| c.is_control() || matches!(c, '"' | '\'' | ';'))
    {
        return Err(HostError::message(format!("unsafe systemd unit path: {s:?}")));
    }
    Ok(trimmed.replace('%', "%%"))
}

pub fn host_binary(paths: &Paths) -> PathBuf {
    resolve_host_bin(paths).unwrap_or_else(|_| paths.repo.join("reforged-host"))
}

pub fn set_mode(_path: &Path, _mode: u32) -> Result<(), HostError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(_path, fs::Permissions::from_mode(_mode)).map_err(|e| HostError::io(_path, e))?;
    }
    Ok(())
}

fn resolve_host_bin(paths: &Paths) -> Result<PathBuf, HostError> {
    let candidates = [
        paths.repo.join("bin").join("reforged-host"),
        paths.repo.join("reforged-host"),
        paths.repo.join("target").join("release").join("reforged-host"),
    ];
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
    }
    std::env::current_exe().map_err(HostError::from)
}

pub fn unit_context(paths: &Paths) -> Result<UnitContext, HostError> {
    let user = std::env::var("SUDO_USER")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty() && s != "root")
        .or_else(|| {
            std::env::var("USER")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            std::env::var("LOGNAME")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            capture("id", &["-un"], None)
                .ok()
                .flatten()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "reforged".to_owned());

    let host_bin = resolve_host_bin(paths)?;
    Ok(UnitContext { user, host_bin })
}

pub fn systemd_units(c: &HostConfig, p: &Paths, ctx: &UnitContext) -> Result<Vec<(String, String)>, HostError> {
    let user = unit_word(&ctx.user)?;
    if user == "root" {
        return Err(HostError::message(
            "systemd units must run as an ordinary user, not root",
        ));
    }
    let repo_dir = unit_path(&p.repo.to_string_lossy())?;
    let host_bin = unit_path(&ctx.host_bin.to_string_lossy())?;

    let mut units = Vec::new();
    let mut service_names = Vec::new();

    if c.mode == ServerMode::Public {
        let allowlist_env = unit_path(&p.config.join("allowlist.env").to_string_lossy())?;
        let allowlist_unit = format!(
            "[Unit]\n\
             Description=DaintyLass UDP allowlist helper\n\
             After=network-online.target\n\
             Wants=network-online.target\n\
             \n\
             [Service]\n\
             Type=simple\n\
             User={user}\n\
             WorkingDirectory={repo_dir}\n\
             EnvironmentFile={allowlist_env}\n\
             ExecStart={host_bin} allowlist\n\
             AmbientCapabilities=CAP_NET_ADMIN\n\
             CapabilityBoundingSet=CAP_NET_ADMIN\n\
             NoNewPrivileges=true\n\
             Restart=on-failure\n\
             RestartSec=2\n\
             \n\
             [Install]\n\
             WantedBy=multi-user.target\n"
        );
        units.push((UNIT_ALLOWLIST.to_owned(), allowlist_unit));
        service_names.push(UNIT_ALLOWLIST);
    }

    let mut after = vec!["network-online.target"];
    after.extend_from_slice(&service_names);
    let after_line = after.join(" ");

    let needs_low_port = c.mode == ServerMode::Public && gateway_port(c) < 1024;
    let cap_lines = if needs_low_port {
        "AmbientCapabilities=CAP_NET_BIND_SERVICE\nCapabilityBoundingSet=CAP_NET_BIND_SERVICE\n"
    } else {
        ""
    };

    let env_line = if c.mode == ServerMode::Public {
        "Environment=REFORGED_ALLOWLIST=external\n"
    } else {
        ""
    };

    let server_unit = format!(
        "[Unit]\n\
         Description=DaintyLass game server ({name})\n\
         After={after_line}\n\
         Wants={after_line}\n\
         \n\
         [Service]\n\
         Type=simple\n\
         User={user}\n\
         WorkingDirectory={repo_dir}\n\
         {env_line}ExecStart={host_bin} start\n\
         {cap_lines}NoNewPrivileges=true\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        name = c.name
    );
    units.push((UNIT_SERVER.to_owned(), server_unit));
    service_names.push(UNIT_SERVER);

    let systemd_dir = sh_quote(&p.systemd.to_string_lossy());
    let mut install_lines = vec![
        "#!/bin/sh".to_owned(),
        "set -eu".to_owned(),
        format!("UNIT_DIR={systemd_dir}"),
    ];
    for svc in &service_names {
        install_lines.push(format!("install -m 0644 \"$UNIT_DIR/{svc}\" /etc/systemd/system/{svc}"));
    }
    install_lines.push("systemctl daemon-reload".to_owned());
    for svc in &service_names {
        install_lines.push(format!("systemctl enable --now {svc}"));
    }
    let install_sh = format!("{}\n", install_lines.join("\n"));
    units.push(("install.sh".to_owned(), install_sh));

    Ok(units)
}

pub fn write_systemd_units(config: &HostConfig, paths: &Paths) -> Result<Vec<PathBuf>, HostError> {
    let ctx = unit_context(paths)?;
    write_systemd_units_with(config, paths, &ctx)
}

pub fn write_systemd_units_with(
    config: &HostConfig,
    paths: &Paths,
    ctx: &UnitContext,
) -> Result<Vec<PathBuf>, HostError> {
    fs::create_dir_all(&paths.systemd)?;
    let units = systemd_units(config, paths, ctx)?;
    let mut written = Vec::with_capacity(units.len());
    for (name, content) in units {
        let dest = paths.systemd.join(&name);
        fs::write(&dest, content.as_bytes())?;
        if name.ends_with(".sh") {
            set_executable_mode(&dest)?;
        }
        written.push(dest);
    }
    Ok(written)
}

pub fn systemd_installed(unit: &str) -> bool {
    Path::new("/etc/systemd/system").join(unit).is_file()
        || Path::new("/lib/systemd/system").join(unit).is_file()
        || Path::new("/usr/lib/systemd/system").join(unit).is_file()
}

pub fn linux_firewall_script(c: &HostConfig) -> Result<String, HostError> {
    let mut lines = vec![
        "#!/bin/sh".to_owned(),
        "# Written by reforged-host setup. Configures ufw for the server's ports.".to_owned(),
        "set -eu".to_owned(),
    ];

    let gw = gateway_port(c);
    let (first, last) = game_ports(c);
    match c.mode {
        ServerMode::Public => {
            lines.push(format!("ufw allow {gw}/tcp"));
            lines.push(format!("ufw allow {first}:{last}/udp"));
        }
        ServerMode::Private => {
            lines.push(format!("ufw allow in on tailscale0 to any port {gw} proto tcp"));
            lines.push(format!(
                "ufw allow in on tailscale0 to any port {first}:{last} proto udp"
            ));
        }
    }

    Ok(format!("{}\n", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use serde_json::json;

    #[test]
    fn parses_effective_capabilities_from_proc_status() {
        let status =
            "Name:\treforged-host\nCapInh:\t0000000000000000\nCapPrm:\t0000000000001400\nCapEff:\t0000000000001400\n";
        assert_eq!(effective_capabilities(status), 0x1400);
        assert!(has_capability(CAP_NET_BIND_SERVICE, Some(status)));
        assert!(has_capability(CAP_NET_ADMIN, Some(status)));
        assert!(!has_capability(0, Some(status)));
    }

    #[test]
    fn can_bind_port_checks_root_sysctl_and_cap() {
        assert!(can_bind_port_with(61000, false, None, None));
        assert!(can_bind_port_with(443, true, None, None));
        assert!(can_bind_port_with(443, false, None, Some("80\n")));
        assert!(!can_bind_port_with(443, false, None, Some("1024\n")));
        let status_with_cap = "CapEff:\t0000000000000400\n";
        assert!(can_bind_port_with(443, false, Some(status_with_cap), Some("1024\n")));
    }

    #[test]
    fn sh_quote_and_wrapper_script_escape_single_quotes() {
        assert_eq!(sh_quote("/opt/dainty's/bin"), "'/opt/dainty'\\''s/bin'");
        let script = gameserver_wrapper_script(Path::new("/srv/dauntless/bin/reforged-host"));
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains("exec '/srv/dauntless/bin/reforged-host' gameserver \"$@\""));
    }

    #[test]
    fn systemd_units_reject_root_and_include_allowlist_for_public() {
        let cfg = HostConfig::from_json(&json!({
            "mode": "public",
            "host": "play.example.org",
            "gameAddress": "203.0.113.10",
            "name": "Linux Hunts",
            "registration": "INVITECODE",
            "maxHuntServers": 4
        }))
        .unwrap();
        let paths = Paths::new("/srv/server");
        let bad_ctx = UnitContext {
            user: "root".to_owned(),
            host_bin: PathBuf::from("/srv/server/bin/reforged-host"),
        };
        assert!(systemd_units(&cfg, &paths, &bad_ctx).is_err());

        let ctx = UnitContext {
            user: "reforged".to_owned(),
            host_bin: PathBuf::from("/srv/server/bin/reforged-host"),
        };
        let units = systemd_units(&cfg, &paths, &ctx).unwrap();
        let names: Vec<&str> = units.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec![UNIT_ALLOWLIST, UNIT_SERVER, "install.sh"]);
        let server_unit = &units[1].1;
        assert!(server_unit.contains("AmbientCapabilities=CAP_NET_BIND_SERVICE"));
        assert!(server_unit.contains("Environment=REFORGED_ALLOWLIST=external\n"));
    }

    #[test]
    fn linux_firewall_script_public_and_private() {
        let pub_cfg = HostConfig::from_json(&json!({
            "mode": "public",
            "host": "play.example.org",
            "gameAddress": "203.0.113.10",
            "name": "Public Hunts",
            "registration": "INVITECODE",
            "maxHuntServers": 4
        }))
        .unwrap();
        let pub_script = linux_firewall_script(&pub_cfg).unwrap();
        assert!(pub_script.contains("ufw allow 443/tcp"));
        assert!(pub_script.contains("ufw allow 8770:8777/udp"));

        let priv_cfg = HostConfig::from_json(&json!({
            "mode": "private",
            "host": "100.64.0.1",
            "gameAddress": "100.64.0.1",
            "name": "Private Hunts",
            "registration": "INVITECODE",
            "maxHuntServers": 4
        }))
        .unwrap();
        let priv_script = linux_firewall_script(&priv_cfg).unwrap();
        assert!(priv_script.contains("ufw allow in on tailscale0 to any port 61000 proto tcp"));
        assert!(priv_script.contains("ufw allow in on tailscale0 to any port 8770:8777 proto udp"));
    }
}
