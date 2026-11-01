// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::fs::{self, File};
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::error::HostError;
use crate::hostconfig::{
    CLUSTER_CA_FILE, HostConfig, NODE_CERT_FILE, NODE_KEY_FILE, PerformanceOptions, Platform, ServerMode, gateway_port,
    is_tailscale_ipv4, linux_options, load_host_config, node_database_config, performance_options,
};
use crate::mongo::{MongoCheck, MongoProblem, ReplicaSetOptions, ensure_replica_set};
use crate::paths::{GAME_VERSION_TEXT, PINNED_EXE_SHA256, PORTS, Paths, RUNTIME_FILE_NAMES, game_exe_relative};
use crate::run::capture;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Fail,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub area: &'static str,
    pub level: Level,
    pub text: String,
    pub fix: Option<String>,
}

pub fn check(area: &'static str, level: Level, text: impl Into<String>) -> Check {
    Check {
        area,
        level,
        text: text.into(),
        fix: None,
    }
}

pub fn fix(area: &'static str, level: Level, text: impl Into<String>, fix_text: impl Into<String>) -> Check {
    Check {
        area,
        level,
        text: text.into(),
        fix: Some(fix_text.into()),
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DoctorOptions {
    pub hash_exe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuPatches {
    pub cpu_fixes: bool,
    pub frame_cap: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct GameBuild<'a> {
    pub version_text: &'a str,
    pub exe_sha256: &'a str,
    pub name: &'a str,
    pub get_it: &'a str,
}

pub const REBORN_BUILD: GameBuild<'static> = GameBuild {
    version_text: GAME_VERSION_TEXT,
    exe_sha256: PINNED_EXE_SHA256,
    name: "1.14.7",
    get_it: "run reforged-host install-game",
};

pub const SERVICES_MEMORY_MB: i64 = 1536;

pub fn tailscale_ipv4() -> Option<String> {
    let mut candidates = vec!["tailscale".to_owned()];
    if cfg!(windows) {
        let pf = crate::mongo::program_files();
        let ts = pf.join("Tailscale").join("tailscale.exe");
        if ts.is_file() {
            candidates.push(ts.to_string_lossy().into_owned());
        }
    }
    for cmd in candidates {
        if let Ok(Some(out)) = capture(&cmd, &["ip", "-4"], None) {
            for line in out.lines() {
                let ip = line.trim();
                if is_tailscale_ipv4(ip) {
                    return Some(ip.to_owned());
                }
            }
        }
    }
    None
}

pub fn port_in_use(port: u16, host: Option<&str>) -> Result<bool, HostError> {
    let host_str = host.unwrap_or("127.0.0.1");
    let ip: IpAddr = if host_str == "0.0.0.0" || host_str.trim().is_empty() {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    } else {
        host_str.parse().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
    };
    let addr = SocketAddr::new(ip, port);
    Ok(TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok())
}

pub fn port_owner_hint(port: u16) -> Option<String> {
    port_owner_hint_with(port, Platform::current(), |cmd, args| {
        capture(cmd, args, None).ok().flatten()
    })
}

pub fn port_owner_hint_for(port: u16, platform: Platform) -> String {
    match platform {
        Platform::Linux => format!("ss -tlnp 'sport = :{port}'"),
        Platform::Windows | Platform::Other => format!("netstat -ano | findstr :{port}"),
    }
}

pub fn port_owner_hint_with(
    port: u16,
    platform: Platform,
    run_cmd: impl Fn(&str, &[&str]) -> Option<String>,
) -> Option<String> {
    let needle = format!(":{port}");
    match platform {
        Platform::Windows => {
            let out = run_cmd("netstat", &["-ano", "-p", "TCP"])?;
            let mut pid = None;
            for line in out.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 5
                    && parts[0].eq_ignore_ascii_case("TCP")
                    && parts[1].ends_with(&needle)
                    && parts[3].eq_ignore_ascii_case("LISTENING")
                {
                    pid = Some(parts[4].to_owned());
                    break;
                }
            }
            let pid = pid?;
            if let Some(t_out) = run_cmd("tasklist", &["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]) {
                let first = t_out.lines().next().unwrap_or("").trim();
                if let Some(stripped) = first.strip_prefix('"')
                    && let Some((proc_name, _)) = stripped.split_once('"')
                    && !proc_name.is_empty()
                {
                    return Some(format!("{proc_name} (PID {pid})"));
                }
            }
            Some(format!("PID {pid}"))
        }
        Platform::Linux => {
            let filter = format!("sport = :{port}");
            let out = run_cmd("ss", &["-lptn", &filter])?;
            for line in out.lines() {
                if let Some(users_idx) = line.find("users:((") {
                    let rest = &line[users_idx + 8..];
                    let proc_name = rest.strip_prefix('"').and_then(|s| s.split_once('"')).map(|(n, _)| n);
                    let pid = rest.split("pid=").nth(1).and_then(|s| s.split([',', ')']).next());
                    return match (proc_name, pid) {
                        (Some(name), Some(p)) => Some(format!("{name} (PID {p})")),
                        (Some(name), None) => Some(name.to_owned()),
                        (None, Some(p)) => Some(format!("PID {p}")),
                        (None, None) => None,
                    };
                }
            }
            None
        }
        Platform::Other => None,
    }
}

pub fn wait_for_http(port: u16, path: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let step = remaining.min(Duration::from_millis(500));
        if let Ok(resp) = crate::invite::http_request("GET", port, path, &[], None, step)
            && resp.status > 0
            && resp.status < 500
        {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn sha256_file(file: &Path) -> Result<String, HostError> {
    let mut f = File::open(file)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{b:02x}");
    }
    Ok(hex)
}

pub fn node_version_ok(version: &str) -> bool {
    let trimmed = version.trim();
    let stripped = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let mut parts = stripped.split('.');
    let Some(major_str) = parts.next() else {
        return false;
    };
    let Some(minor_str) = parts.next() else {
        return false;
    };
    let Ok(major) = major_str.parse::<u32>() else {
        return false;
    };
    let minor_digits: String = minor_str.chars().take_while(|c| c.is_ascii_digit()).collect();
    let Ok(minor) = minor_digits.parse::<u32>() else {
        return false;
    };
    major > 20 || (major == 20 && minor >= 19)
}

pub fn count_function_files(sdk_dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(sdk_dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| e.path().is_file() && e.file_name().to_str().is_some_and(|n| n.ends_with("_functions.cpp")))
        .count()
}

pub fn game_checks(game_dir: Option<&str>, hash_exe: bool) -> Result<Vec<Check>, HostError> {
    game_exe_checks(game_dir, hash_exe, &REBORN_BUILD)
}

pub fn game_exe_checks(game_dir: Option<&str>, hash_exe: bool, build: &GameBuild<'_>) -> Result<Vec<Check>, HostError> {
    let area = "game";
    let mut out = Vec::new();

    let Some(dir_str) = game_dir.map(str::trim).filter(|s| !s.is_empty()) else {
        out.push(Check {
            area,
            level: Level::Fail,
            text: "no game folder configured".to_owned(),
            fix: Some(format!("{} or pass --game <dir> to setup", build.get_it)),
        });
        return Ok(out);
    };

    let dir = Path::new(dir_str);
    let exe = dir.join(game_exe_relative());
    if !exe.is_file() {
        out.push(Check {
            area,
            level: Level::Fail,
            text: format!("no game executable at {}", exe.display()),
            fix: Some(build.get_it.to_owned()),
        });
        return Ok(out);
    }

    let version_file = dir.join("Version.txt");
    let raw_version = fs::read_to_string(&version_file).unwrap_or_default();
    let text = raw_version.strip_prefix('\u{feff}').unwrap_or(&raw_version).trim();
    if text != build.version_text {
        let shown = if text.is_empty() { "nothing" } else { text };
        out.push(Check {
            area,
            level: Level::Fail,
            text: format!("Version.txt says \"{shown}\", not {}", build.version_text),
            fix: Some(build.get_it.to_owned()),
        });
        return Ok(out);
    }

    out.push(Check {
        area,
        level: Level::Ok,
        text: format!("Dauntless {} ({})", build.name, exe.display()),
        fix: None,
    });

    if hash_exe {
        let actual = sha256_file(&exe)?;
        if actual.eq_ignore_ascii_case(build.exe_sha256) {
            out.push(Check {
                area,
                level: Level::Ok,
                text: format!(
                    "the executable matches Dauntless {} (SHA-256 {})",
                    build.name, build.exe_sha256
                ),
                fix: None,
            });
        } else {
            out.push(Check {
                area,
                level: Level::Fail,
                text: format!("the executable is another build (SHA-256 {actual})"),
                fix: Some(build.get_it.to_owned()),
            });
        }
    }

    Ok(out)
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

pub fn runtime_cpu_patches(dll_path: &Path) -> Option<CpuPatches> {
    let bytes = fs::read(dll_path).ok()?;
    let walk_flag_utf16le: Vec<u8> = "LEGACY_EXPECTED_PLAYER_WALK.flag"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let cpu_fixes = contains_subslice(&bytes, &walk_flag_utf16le);
    let frame_cap = contains_subslice(&bytes, b"REFORGED_SERVER_MAX_FPS");
    Some(CpuPatches { cpu_fixes, frame_cap })
}

pub fn hunts_that_fit(total_mb: u64, perf: &PerformanceOptions) -> u64 {
    if perf.hunt_memory_mb == 0 {
        return 0;
    }
    let hunt_mb = i64::from(perf.hunt_memory_mb);
    let min_free = i64::from(perf.min_free_memory_mb);
    let overhead = SERVICES_MEMORY_MB + 2 * hunt_mb + min_free;
    let total = i64::try_from(total_mb).unwrap_or(i64::MAX);
    let available = total.saturating_sub(overhead);
    if available <= 0 {
        0
    } else {
        (available / hunt_mb) as u64
    }
}

pub fn total_memory_mb() -> Option<u64> {
    if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
        for line in meminfo.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                let kb_str = rest.split_whitespace().next()?;
                let kb = kb_str.parse::<u64>().ok()?;
                return Some(kb / 1024);
            }
        }
    }
    if cfg!(windows)
        && let Ok(Some(out)) = capture(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
            ],
            None,
        )
        && let Ok(bytes) = out.trim().parse::<u64>()
    {
        return Some(bytes / (1024 * 1024));
    }
    None
}

pub fn performance_summary_line(config: &HostConfig) -> String {
    let perf = performance_options(config);
    let fps_part = if perf.idle_fps == 0 || perf.idle_fps >= perf.server_fps {
        format!("{} fps (never lowered when empty)", perf.server_fps)
    } else {
        format!(
            "{} fps, {} fps after 15 s without players",
            perf.server_fps, perf.idle_fps
        )
    };
    let training_part = if !perf.training_on_demand {
        "Training Grounds always running".to_owned()
    } else if perf.training_idle_minutes == 0 {
        "Training Grounds on demand (never stopped when empty)".to_owned()
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
    let max_h = config.max_hunt_servers;
    let hunts_word = if max_h == 1 {
        "1 hunt".to_owned()
    } else {
        format!("{max_h} hunts")
    };
    let (first_port, last_port) = crate::hostconfig::game_ports(config);
    let guard_part = if perf.memory_guard {
        format!(
            "a new hunt only with {} MB free",
            perf.hunt_memory_mb + perf.min_free_memory_mb
        )
    } else {
        "memory guard off".to_owned()
    };
    format!(
        "game servers at {fps_part}; {training_part}; {logs_part}; at most {hunts_word} (UDP {first_port}-{last_port}); {guard_part}"
    )
}

pub fn performance_checks(
    config: &HostConfig,
    runtime_dll: Option<&Path>,
    total_mb: Option<u64>,
    _platform: Platform,
) -> Vec<Check> {
    let area = "performance";
    let mut out = Vec::new();
    let perf = performance_options(config);

    out.push(Check {
        area,
        level: Level::Info,
        text: performance_summary_line(config),
        fix: None,
    });

    if let Some(dll) = runtime_dll
        && let Some(patches) = runtime_cpu_patches(dll)
    {
        if patches.cpu_fixes && patches.frame_cap {
            out.push(Check {
                area,
                level: Level::Ok,
                text: "MysticParadox.dll includes patches 0019 (CPU fixes) and 0020 (frame cap)".to_owned(),
                fix: None,
            });
        } else {
            out.push(Check {
                area,
                level: Level::Warn,
                text: "MysticParadox.dll is missing patches 0019 (CPU fixes) and 0020 (frame cap)".to_owned(),
                fix: Some("rebuild and publish MysticParadox.dll".to_owned()),
            });
        }
    }

    if let Some(mb) = total_mb {
        let fits = hunts_that_fit(mb, &perf);
        let max_h = u64::from(config.max_hunt_servers);
        if max_h > fits {
            let gb = (mb as f64) / 1024.0;
            if perf.memory_guard {
                out.push(Check {
                    area,
                    level: Level::Info,
                    text: format!(
                        "{max_h} hunts at once may need more than this machine's {gb:.1} GB: about {fits} fit; the memory guard refuses a new hunt when memory is low"
                    ),
                    fix: None,
                });
            } else {
                out.push(Check {
                    area,
                    level: Level::Warn,
                    text: format!(
                        "{max_h} hunts at once may need more than this machine's {gb:.1} GB: about {fits} fit"
                    ),
                    fix: Some(format!("reforged-host setup --memory-guard, or --max-hunts {fits}")),
                });
            }
        }
    }

    out
}

pub fn linux_checks(p: &Paths, config: &HostConfig) -> Result<Vec<Check>, HostError> {
    let area = "linux";
    let mut out = Vec::new();
    let opts = linux_options(config);

    match crate::linux::game_runtime(&opts)? {
        Some(rt) => {
            let label = match rt.kind {
                crate::linux::RuntimeKind::Wine => "Wine",
                crate::linux::RuntimeKind::Proton => "Proton",
            };
            out.push(Check {
                area,
                level: Level::Ok,
                text: format!("{label} runtime found at {}", rt.path),
                fix: None,
            });
        }
        None => {
            out.push(Check {
                area,
                level: Level::Fail,
                text: "neither Wine nor Proton was found".to_owned(),
                fix: Some("install wine and wine64, or pass --wine / --proton to setup".to_owned()),
            });
        }
    }

    if opts.proton.is_none() {
        if crate::wine::wine_prefix_ready(&p.wine_prefix) {
            out.push(Check {
                area,
                level: Level::Ok,
                text: format!("Wine prefix ready at {}", p.wine_prefix.display()),
                fix: None,
            });
        } else {
            out.push(Check {
                area,
                level: Level::Warn,
                text: format!("Wine prefix is not initialized at {}", p.wine_prefix.display()),
                fix: Some("run reforged-host setup".to_owned()),
            });
        }
    }

    if opts.xvfb {
        if let Some(xvfb) = crate::wine::find_on_path(&["xvfb-run"])? {
            out.push(Check {
                area,
                level: Level::Ok,
                text: format!("xvfb-run found at {xvfb}"),
                fix: None,
            });
        } else {
            out.push(Check {
                area,
                level: Level::Fail,
                text: "xvfb-run not found on PATH".to_owned(),
                fix: Some("install xvfb and xauth, or pass --no-xvfb to setup".to_owned()),
            });
        }
        if let Some(xauth) = crate::wine::find_on_path(&["xauth"])? {
            out.push(Check {
                area,
                level: Level::Ok,
                text: format!("xauth found at {xauth}"),
                fix: None,
            });
        } else {
            out.push(Check {
                area,
                level: Level::Warn,
                text: "xauth not found on PATH (needed by xvfb-run)".to_owned(),
                fix: Some("install xauth".to_owned()),
            });
        }
    }

    if crate::wine::is_executable(&p.gameserver_wrapper) {
        out.push(Check {
            area,
            level: Level::Ok,
            text: format!("gameserver wrapper ready at {}", p.gameserver_wrapper.display()),
            fix: None,
        });
    } else {
        out.push(Check {
            area,
            level: Level::Fail,
            text: format!("gameserver wrapper missing at {}", p.gameserver_wrapper.display()),
            fix: Some("run reforged-host setup".to_owned()),
        });
    }

    if config.mode == ServerMode::Public {
        let nft = crate::wine::find_on_path(&["nft"])?
            .or_else(|| Path::new("/usr/sbin/nft").is_file().then(|| "/usr/sbin/nft".to_owned()))
            .or_else(|| Path::new("/sbin/nft").is_file().then(|| "/sbin/nft".to_owned()));
        if let Some(nft_bin) = nft {
            out.push(Check {
                area,
                level: Level::Ok,
                text: format!("nftables found at {nft_bin}"),
                fix: None,
            });
        } else {
            out.push(Check {
                area,
                level: Level::Fail,
                text: "nftables (nft) not found".to_owned(),
                fix: Some("install nftables".to_owned()),
            });
        }

        let gw_port = gateway_port(config);
        if gw_port < 1024
            && !crate::linux::can_bind_port(gw_port)?
            && !crate::linux::systemd_installed(crate::linux::UNIT_SERVER)
        {
            out.push(Check {
                area,
                level: Level::Warn,
                text: format!("port {gw_port} requires CAP_NET_BIND_SERVICE or the systemd unit"),
                fix: Some(format!(
                    "run sudo {}/install.sh or set public_port = 4443",
                    p.systemd.display()
                )),
            });
        }
    }

    Ok(out)
}

pub fn network_checks(p: &Paths, config: &HostConfig, _platform: Platform) -> Result<Vec<Check>, HostError> {
    let area = "network";
    let mut out = Vec::new();

    match config.mode {
        ServerMode::Private => {
            if let Some(ts_ip) = tailscale_ipv4() {
                if config.game_address == ts_ip {
                    out.push(Check {
                        area,
                        level: Level::Ok,
                        text: format!("Tailscale IPv4 {ts_ip} matches game_address"),
                        fix: None,
                    });
                } else {
                    out.push(Check {
                        area,
                        level: Level::Warn,
                        text: format!("Tailscale IPv4 is {ts_ip}, but game_address is {}", config.game_address),
                        fix: Some(format!("run reforged-host setup --game-address {ts_ip}")),
                    });
                }
            } else {
                out.push(Check {
                    area,
                    level: Level::Warn,
                    text: "Tailscale IPv4 was not detected".to_owned(),
                    fix: Some("start Tailscale and sign in".to_owned()),
                });
            }
        }
        ServerMode::Public => {
            if p.cert_file.is_file() && p.key_file.is_file() {
                match crate::invite::certificate_fingerprint(&p.cert_file) {
                    Ok(fp) => {
                        out.push(Check {
                            area,
                            level: Level::Ok,
                            text: format!("gateway TLS certificate ready (sha256:{})", &fp[..fp.len().min(16)]),
                            fix: None,
                        });
                    }
                    Err(e) => {
                        out.push(Check {
                            area,
                            level: Level::Fail,
                            text: format!("gateway TLS certificate is invalid: {e}"),
                            fix: Some("run reforged-host setup".to_owned()),
                        });
                    }
                }
            } else {
                out.push(Check {
                    area,
                    level: Level::Fail,
                    text: "gateway TLS certificate or key is missing".to_owned(),
                    fix: Some("run reforged-host setup".to_owned()),
                });
            }

            if let Some(net) = &config.network
                && !net.nodes.is_empty()
            {
                let node_cert = p.tls.join(NODE_CERT_FILE);
                let node_key = p.tls.join(NODE_KEY_FILE);
                let ca_cert = p.tls.join(CLUSTER_CA_FILE);
                if node_cert.is_file() && node_key.is_file() && ca_cert.is_file() {
                    out.push(Check {
                        area,
                        level: Level::Ok,
                        text: format!(
                            "cluster node TLS credentials present ({} peer node(s))",
                            net.nodes.len()
                        ),
                        fix: None,
                    });
                } else {
                    out.push(Check {
                        area,
                        level: Level::Fail,
                        text: "cluster node TLS certificate, key, or CA is missing in .workspace/tls".to_owned(),
                        fix: Some("run reforged-host network issue-node-cert or join".to_owned()),
                    });
                }
            }
        }
    }

    Ok(out)
}

pub fn database_check(mongo: &MongoCheck, _uri: &str, managed: bool) -> Check {
    let area = "database";
    match mongo {
        MongoCheck::Ok {
            set_name,
            primary: true,
            ..
        } => Check {
            area,
            level: Level::Ok,
            text: format!("MongoDB replica set {set_name}, primary"),
            fix: None,
        },
        MongoCheck::Ok {
            set_name,
            primary: false,
            ..
        } => Check {
            area,
            level: Level::Warn,
            text: format!("MongoDB replica set {set_name} is not primary yet"),
            fix: Some("wait a few seconds or run reforged-host setup".to_owned()),
        },
        MongoCheck::Failed {
            reason: MongoProblem::Unreachable,
            detail,
        } if managed => Check {
            area,
            level: Level::Info,
            text: format!("managed MongoDB is not running right now ({detail})"),
            fix: Some("start the server with reforged-host start".to_owned()),
        },
        MongoCheck::Failed { reason, detail } => Check {
            area,
            level: Level::Fail,
            text: format!("MongoDB check failed ({reason:?}): {detail}"),
            fix: Some("start MongoDB with --replSet rs0 or run reforged-host setup".to_owned()),
        },
    }
}

pub fn store_database_check(p: &Paths, config: &HostConfig) -> Check {
    let area = "database";
    if let Some(text) = crate::commands::update::unmigrated_mongo_problem(p, config) {
        return Check {
            area,
            level: Level::Fail,
            text,
            fix: None,
        };
    }

    let db_cfg = match node_database_config(config, p) {
        Ok(c) => c,
        Err(e) => {
            return Check {
                area,
                level: Level::Fail,
                text: format!("invalid database configuration: {e}"),
                fix: Some("run reforged-host setup".to_owned()),
            };
        }
    };

    let engine_label = if config.database.as_ref().is_some_and(|d| d.postgres()) {
        "Postgres"
    } else {
        "SQLite"
    };

    match &db_cfg {
        store::DatabaseConfig::Sqlite { file } if !file.is_file() => Check {
            area,
            level: Level::Ok,
            text: format!("SQLite store database ready ({})", file.display()),
            fix: None,
        },
        _ => match store::Database::attach(&db_cfg, &store::DatabaseOptions::default(), &[]) {
            Ok(_) => Check {
                area,
                level: Level::Ok,
                text: format!("{engine_label} store database is healthy"),
                fix: None,
            },
            Err(e) => Check {
                area,
                level: Level::Fail,
                text: format!("{engine_label} store database check failed: {e}"),
                fix: Some("run reforged-host setup".to_owned()),
            },
        },
    }
}

pub fn run_doctor(p: &Paths, opts: &DoctorOptions) -> Result<Vec<Check>, HostError> {
    let mut checks = Vec::new();
    let platform = Platform::current();

    if cfg!(feature = "paradox") {
        if let Ok(Some(ver)) = capture("node", &["--version"], None) {
            let trimmed = ver.trim();
            if node_version_ok(trimmed) {
                checks.push(Check {
                    area: "tools",
                    level: Level::Ok,
                    text: format!("Node.js {trimmed}"),
                    fix: None,
                });
            } else {
                checks.push(Check {
                    area: "tools",
                    level: Level::Fail,
                    text: format!("Node.js {trimmed} is too old (need >= 20.19)"),
                    fix: Some("install Node.js 20.19 or newer".to_owned()),
                });
            }
        } else {
            checks.push(Check {
                area: "tools",
                level: Level::Fail,
                text: "Node.js was not found on PATH".to_owned(),
                fix: Some("install Node.js 20.19 or newer".to_owned()),
            });
        }
    }

    let Some(config) = load_host_config(p)? else {
        checks.push(Check {
            area: "config",
            level: Level::Fail,
            text: "server is not configured yet (host.toml is missing)".to_owned(),
            fix: Some("run reforged-host setup".to_owned()),
        });
        return Ok(checks);
    };

    checks.push(Check {
        area: "config",
        level: Level::Ok,
        text: format!("{} server {:?} on {}", config.mode.as_str(), config.name, config.host),
        fix: None,
    });

    let mut all_rt = true;
    for name in RUNTIME_FILE_NAMES {
        if !p.runtime_publish.join(name).is_file() {
            all_rt = false;
            break;
        }
    }
    if all_rt && p.runtime_publish.join("runtime.json").is_file() {
        checks.push(Check {
            area: "runtime",
            level: Level::Ok,
            text: format!("published runtime present in {}", p.runtime_publish.display()),
            fix: None,
        });
    } else {
        checks.push(Check {
            area: "runtime",
            level: Level::Warn,
            text: format!("published runtime is missing in {}", p.runtime_publish.display()),
            fix: Some("run reforged-host publish-runtime".to_owned()),
        });
    }

    checks.extend(game_checks(config.game_dir.as_deref(), opts.hash_exe)?);

    let published_dll = p.runtime_publish.join("MysticParadox.dll");
    let dll_to_check = if published_dll.is_file() {
        Some(published_dll.as_path())
    } else if p.runtime_dll.is_file() {
        Some(p.runtime_dll.as_path())
    } else {
        None
    };
    checks.extend(performance_checks(&config, dll_to_check, total_memory_mb(), platform));

    if platform == Platform::Linux {
        checks.extend(linux_checks(p, &config)?);
    }

    checks.extend(network_checks(p, &config, platform)?);

    if cfg!(feature = "paradox") {
        let mongo = ensure_replica_set(
            &config.mongo.uri,
            &ReplicaSetOptions {
                initiate: false,
                host: None,
                timeout_ms: Some(2000),
            },
        )?;
        checks.push(database_check(&mongo, &config.mongo.uri, config.mongo.manage));
    } else {
        checks.push(store_database_check(p, &config));
    }

    if wait_for_http(PORTS.backend, "/health", Duration::from_millis(250)) {
        checks.push(Check {
            area: "services",
            level: Level::Ok,
            text: format!("backend is responding on 127.0.0.1:{}", PORTS.backend),
            fix: None,
        });
    } else {
        checks.push(Check {
            area: "services",
            level: Level::Info,
            text: "server services are not running right now".to_owned(),
            fix: None,
        });
    }

    Ok(checks)
}

pub fn print_checks(checks: &[Check], say: &mut dyn FnMut(&str)) -> (usize, usize) {
    let mut current_area: Option<&str> = None;
    let mut fails = 0usize;
    let mut warns = 0usize;

    for check in checks {
        if current_area != Some(check.area) {
            say(&format!("\n{}", check.area));
            current_area = Some(check.area);
        }
        let badge = match check.level {
            Level::Ok => "  [ ok ] ",
            Level::Warn => {
                warns += 1;
                "  [warn] "
            }
            Level::Fail => {
                fails += 1;
                "  [FAIL] "
            }
            Level::Info => "  [info] ",
        };
        say(&format!("{badge}{}", check.text));
        if check.level != Level::Ok
            && let Some(fix) = &check.fix
        {
            say(&format!("         -> {fix}"));
        }
    }

    (fails, warns)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn node_version_ok_requires_20_19_or_newer() {
        assert!(!node_version_ok("v18.20.0"));
        assert!(!node_version_ok("v20.18.9"));
        assert!(node_version_ok("v20.19.0"));
        assert!(node_version_ok("20.19.1"));
        assert!(node_version_ok("v22.1.0"));
    }

    #[test]
    fn count_function_files_counts_only_sdk_functions_cpp() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Archon_functions.cpp"), "").unwrap();
        fs::write(tmp.path().join("CoreUObject_functions.cpp"), "").unwrap();
        fs::write(tmp.path().join("Archon_classes.hpp"), "").unwrap();
        assert_eq!(count_function_files(tmp.path()), 2);
    }

    #[test]
    fn runtime_cpu_patches_detects_utf16le_and_ascii_markers() {
        let tmp = TempDir::new().unwrap();
        let dll = tmp.path().join("MysticParadox.dll");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"header");
        for unit in "LEGACY_EXPECTED_PLAYER_WALK.flag".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(b"middle REFORGED_SERVER_MAX_FPS footer");
        fs::write(&dll, &bytes).unwrap();

        let patches = runtime_cpu_patches(&dll).unwrap();
        assert_eq!(
            patches,
            CpuPatches {
                cpu_fixes: true,
                frame_cap: true
            }
        );
    }

    #[test]
    fn hunts_that_fit_respects_services_and_min_free_overhead() {
        let perf = PerformanceOptions::default(); // hunt_memory_mb=1536, min_free_memory_mb=1024
        // overhead = 1536 + 2*1536 + 1024 = 5632 MB
        assert_eq!(hunts_that_fit(4096, &perf), 0);
        assert_eq!(hunts_that_fit(5632 + 1536, &perf), 1);
        assert_eq!(hunts_that_fit(5632 + 6 * 1536, &perf), 6);
    }

    #[test]
    fn game_checks_verify_exe_and_version_txt() {
        let tmp = TempDir::new().unwrap();
        let checks = game_checks(None, false).unwrap();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].level, Level::Fail);

        let exe = tmp.path().join(game_exe_relative());
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"binary").unwrap();
        fs::write(
            tmp.path().join("Version.txt"),
            format!("\u{feff}{GAME_VERSION_TEXT}\r\n"),
        )
        .unwrap();

        let checks = game_checks(Some(&tmp.path().to_string_lossy()), false).unwrap();
        assert!(checks.iter().all(|c| c.level == Level::Ok));
    }

    #[test]
    fn port_owner_hint_parses_windows_and_linux_outputs() {
        let win = port_owner_hint_with(61000, Platform::Windows, |cmd, _| match cmd {
            "netstat" => {
                Some("  TCP    127.0.0.1:61000        0.0.0.0:0              LISTENING       4242\r\n".to_owned())
            }
            "tasklist" => Some("\"reforged-host.exe\",\"4242\",\"Console\",\"1\",\"12,000 K\"\r\n".to_owned()),
            _ => None,
        });
        assert_eq!(win.as_deref(), Some("reforged-host.exe (PID 4242)"));

        let lin = port_owner_hint_with(61000, Platform::Linux, |cmd, _| match cmd {
            "ss" => {
                Some("LISTEN 0 128 127.0.0.1:61000 0.0.0.0:* users:((\"reforged-host\",pid=9001,fd=8))\n".to_owned())
            }
            _ => None,
        });
        assert_eq!(lin.as_deref(), Some("reforged-host (PID 9001)"));
    }

    #[test]
    fn print_checks_groups_by_area_and_counts_failures_and_warnings() {
        let checks = vec![
            Check {
                area: "config",
                level: Level::Ok,
                text: "host.toml ok".to_owned(),
                fix: Some("ignored on ok".to_owned()),
            },
            Check {
                area: "game",
                level: Level::Warn,
                text: "Version.txt missing".to_owned(),
                fix: Some("restore Version.txt".to_owned()),
            },
            Check {
                area: "game",
                level: Level::Fail,
                text: "exe missing".to_owned(),
                fix: Some("run install-game".to_owned()),
            },
        ];
        let mut lines = Vec::new();
        let (fails, warns) = print_checks(&checks, &mut |l| lines.push(l.to_owned()));
        assert_eq!((fails, warns), (1, 1));
        assert_eq!(
            lines,
            vec![
                "\nconfig",
                "  [ ok ] host.toml ok",
                "\ngame",
                "  [warn] Version.txt missing",
                "         -> restore Version.txt",
                "  [FAIL] exe missing",
                "         -> run install-game",
            ]
        );
    }
}
