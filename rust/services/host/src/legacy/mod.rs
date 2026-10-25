//! Legacy servers (the `legacy` feature; RD5 reversed): Dauntless Revived's 1.4.4 stack, set up and run
//! by the same kit. In place of Mystic Paradox's backend, Director and MongoDB, a legacy server runs DR's
//! metagame (its own SQLite database) and DR's deploy server, which starts the 1.4.4 game servers as DR's
//! deploy kits did, the internal server (`native/internal-server`) loaded by the runtime loader (`deploy/unix-common/
//! generate-config.mjs`, `deploy/windows-server/Stack.ps1` at DR 24b7624). The content server, the
//! allowlist and the gateway are the reborn ones, told `legacy` in their env files.
//!
//! Each DR variable keeps its name with the service in front: the metagame's `PORT` is `METAGAME_PORT`,
//! the deploy server's `DEPLOY_PORT`, so a stale one in the shell never reaches them ([`crate::services`]).

use std::path::{Path, PathBuf};
use std::time::Duration;

use aws_lc_rs::rand::{SecureRandom, SystemRandom};

use crate::Outcome;
use crate::commands::start::binaries::{FrontService, Programs};
use crate::commands::start::control::{Control, controlled_by_stdin};
use crate::commands::start::{
    Planned, Started, close_game_ports, console_printer, env_file, finish, host_checks, linux_steps, port_in_use,
    require_env_files, say, spec_from, start_services, up_line,
};
use crate::doctor::{Check, GameBuild, Level, check, fix, game_exe_checks, port_in_use as port_taken, wait_for_http};
use crate::error::HostError;
use crate::hostconfig::{
    EnvEntries, HostConfig, Platform, Secrets, ServerMode, build_env_files, game_ports, linux_options, path_text,
    write_named_env_files, write_secret_file,
};
use crate::paths::{PORTS, Paths, game_exe_relative};
use crate::services::{Ready, Supervisor, service_environment};

/// The metagame answers where the backend does: the launcher probes 61000 (`start`'s control contract).
pub const METAGAME_PORT: u16 = PORTS.backend;
/// DR's deploy server, where the Director listens on a reborn server.
pub const DEPLOY_PORT: u16 = PORTS.director;
/// The metagame's XMPP chat (DR's `CHAT_PORT`), behind the gateway's `GATEWAY_WS_URL`.
pub const CHAT_PORT: u16 = 61099;
pub const GAME_CHANGELIST: u32 = 239_827;
pub const GAME_VERSION_TEXT: &str = "dauntless_rel-1.4.4_Shipping_2020-10-28_20-11-15_239827";
/// DR's pin of the 1.4.4 executable (`DauntlessServer.Common.ps1`).
pub const PINNED_EXE_SHA256: &str = "d3d41e614908d2befd518b27046d9822d6130ef12ba3504babbdb786bef9cff4";
/// Beside the game executable: the runtime loader and the internal server its ini names (no DR dxgi.dll).
pub const SERVER_DLLS: [&str; 2] = ["winmm.dll", "UndauntedInternalServer.dll"];
pub const LOADER_INI: &str = "mystic_loader.ini";

const SOURCE_URL: &str = "https://github.com/FreezaMotoqueiro/Dauntless-Reforged-Revival";

/// The metagame's database, in the workspace with the rest of the server's state.
pub fn database(p: &Paths) -> PathBuf {
    p.workspace.join("metagame").join("undaunted.db")
}

/// The game-server key the metagame registers (`metagame db gs-key`) and the deploy server sends.
pub fn gameserver_key_file(p: &Paths) -> PathBuf {
    p.config.join("gameserver.key")
}

/// Linux: what the deploy server starts for each game server, `deploy run-game` (DR linked it as
/// `dr-run-game`).
pub fn run_game_wrapper(p: &Paths) -> PathBuf {
    p.bin.join("reforged-legacy-gameserver")
}

fn write_run_game_wrapper(p: &Paths) -> Result<PathBuf, HostError> {
    let host = crate::linux::host_binary(p);
    let deploy = host.with_file_name(format!("deploy{}", std::env::consts::EXE_SUFFIX));
    let script = [
        "#!/bin/sh".to_owned(),
        "# Dauntless Reforged Revival: DR's deploy server starts this for each 1.4.4 game server. Written by `reforged-host setup`.".to_owned(),
        format!("exec {} run-game \"$@\"", crate::linux::sh_quote(&deploy.to_string_lossy())),
        String::new(),
    ]
    .join("
");
    let target = run_game_wrapper(p);
    std::fs::create_dir_all(&p.bin).map_err(|e| HostError::io(&p.bin, e))?;
    std::fs::write(&target, script).map_err(|e| HostError::io(&target, e))?;
    crate::linux::set_mode(&target, 0o755)?;
    Ok(target)
}

/// A legacy server's env files: metagame, deploy, content, gateway and (public) allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyEnvFiles {
    pub metagame: EnvEntries,
    pub deploy: EnvEntries,
    pub content: EnvEntries,
    pub gateway: EnvEntries,
    pub allowlist: Option<EnvEntries>,
}

impl LegacyEnvFiles {
    /// `(name, entries)` in the order they are written; `None` removes a reborn server's file.
    pub fn files(&self) -> [(&'static str, Option<&EnvEntries>); 8] {
        [
            ("metagame", Some(&self.metagame)),
            ("deploy", Some(&self.deploy)),
            ("content", Some(&self.content)),
            ("gateway", Some(&self.gateway)),
            ("allowlist", self.allowlist.as_ref()),
            ("backend", None),
            ("director", None),
            ("admin", None),
        ]
    }
}

fn owned(entries: Vec<(&str, String)>) -> EnvEntries {
    entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

fn local(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

pub fn build_legacy_env_files(
    c: &HostConfig,
    s: &Secrets,
    p: &Paths,
    source_commit: &str,
    platform: Platform,
) -> LegacyEnvFiles {
    let linux = platform == Platform::Linux;
    // The gateway's and the allowlist's files are the reborn ones, with the game named and the metagame's
    // chat in place of the backend's realtime listener.
    let reborn = build_env_files(c, s, p, source_commit, platform);

    let metagame = vec![
        ("METAGAME_PORT", METAGAME_PORT.to_string()),
        // Plain HTTP on this machine only: the gateway is the front door in both modes.
        ("METAGAME_BIND_HOST", "127.0.0.1".into()),
        ("METAGAME_NODE_ENV", "production".into()),
        ("METAGAME_AUTH_MODE", "APIKEY".into()),
        ("METAGAME_AUTH_SIGNING_PRIVKEY_B64", s.jwt_private_key_b64.clone()),
        ("METAGAME_AUTH_SIGNING_PUBKEY_B64", s.jwt_public_key_b64.clone()),
        ("METAGAME_REGISTRATION_MODE", c.registration.as_str().into()),
        ("METAGAME_DB_FILENAME", path_text(&database(p))),
        ("METAGAME_MATCHMAKING_MODE", "DEPLOYSERVER".into()),
        ("METAGAME_DEPLOYSERVER_URL", format!("127.0.0.1:{DEPLOY_PORT}")),
        // The launcher answers the game's QoS ping itself, on its relay, as for reborn.
        ("METAGAME_QOS_TARGET_URL", format!("{}/QoS", local(PORTS.relay))),
        ("METAGAME_TARGET_CHANGELIST", GAME_CHANGELIST.to_string()),
        ("METAGAME_SERVER_NAME", c.name.clone()),
        ("METAGAME_SOURCE_URL", SOURCE_URL.into()),
        ("METAGAME_GIT_COMMIT", source_commit.into()),
        ("METAGAME_CONTENT_PORT", PORTS.content.to_string()),
        ("METAGAME_BODY_LOG_FILE", path_text(&p.logs.join("metagame-bodies.log"))),
        ("METAGAME_LOG_BODIES", "0".into()),
        ("METAGAME_CHAT", "1".into()),
        ("METAGAME_CHAT_PORT", CHAT_PORT.to_string()),
        ("METAGAME_CHAT_BIND_HOST", "127.0.0.1".into()),
        // DR sent it in public mode only; Reforged's gateway proxies in both.
        ("METAGAME_GATEWAY_SECRET", s.gateway_secret.clone()),
    ];

    let lx = linux_options(c);
    let exe = c.game_dir.as_ref().map(|dir| Path::new(dir).join(game_exe_relative()));
    let exe_text = exe.as_deref().map(path_text).unwrap_or_default();
    let (first, last) = game_ports(c);
    let mut deploy: Vec<(&str, String)> = vec![
        ("DEPLOY_NODE_ENV", "production".into()),
        ("DEPLOY_PORT", DEPLOY_PORT.to_string()),
        ("DEPLOY_BIND_HOST", "127.0.0.1".into()),
        ("DEPLOY_MY_IP", c.game_address.clone()),
        // Ramsgate on the last port, the Training Dojo on the one before, hunts below (DR's layout).
        ("DEPLOY_PORT_RANGE_BEGIN", first.to_string()),
        ("DEPLOY_PORT_RANGE_END", last.to_string()),
        // Linux: DR's own launcher (`deploy run-game`), which runs the exe through Wine or Proton with winmm=n,b.
        (
            "DEPLOY_GAMESERVER_BINARY_PATH",
            if linux {
                path_text(&run_game_wrapper(p))
            } else {
                exe_text.clone()
            },
        ),
    ];
    // run-game's settings, which the deploy server hands it under DR's names (DR_GAME_EXE ...).
    if linux {
        deploy.push(("DEPLOY_DR_GAME_EXE", exe_text));
        if let Some(wine) = &lx.wine {
            deploy.push(("DEPLOY_DR_WINE_BINARY", wine.clone()));
        }
        if let Some(proton) = &lx.proton {
            deploy.push(("DEPLOY_DR_PROTON_BINARY", proton.clone()));
        }
        deploy.extend([
            ("DEPLOY_DR_WINEPREFIX", path_text(&p.wine_prefix)),
            ("DEPLOY_DR_PROTON_COMPAT_DATA", path_text(&p.proton_data)),
            ("DEPLOY_DR_XVFB", if lx.xvfb { "1" } else { "0" }.into()),
        ]);
    }
    deploy.extend([
        ("DEPLOY_METAGAME_API_KEY", s.gameserver_key.clone()),
        ("DEPLOY_SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP", "10".into()),
        ("DEPLOY_RAMSGATE_POOL", "1".into()),
        ("DEPLOY_ENABLE_DOJO", "0".into()),
        ("DEPLOY_GAMESERVER_LOG_DIR", path_text(&p.gameserver_logs)),
        ("DEPLOY_METAGAME_URL", local(METAGAME_PORT)),
        // DR's default is relative to the working folder.
        ("DEPLOY_DRAIN_FILE", path_text(&p.config.join("drain.flag"))),
    ]);

    let mut content = vec![
        ("PORT", PORTS.content.to_string()),
        ("BIND_HOST", "127.0.0.1".into()),
        ("BACKEND_URL", local(METAGAME_PORT)),
        ("CONTENT_GAME", "legacy".into()),
    ];
    // The 1.4.4 files players download (DR's content server).
    if let Some(dir) = &c.game_dir {
        content.push(("CONTENT_GAME_DIR", dir.clone()));
    }
    content.extend([
        ("CONTENT_BRANDING_DIR", path_text(&p.branding)),
        ("CONTENT_NEWS_FILE", path_text(&p.news_file)),
    ]);

    let mut gateway: EnvEntries = Vec::new();
    for (key, value) in reborn.gateway {
        match key.as_str() {
            "GATEWAY_WS_URL" => gateway.push((key, local(CHAT_PORT))),
            "GATEWAY_BACKEND_URL" => {
                gateway.push(("GATEWAY_GAME".into(), "legacy".into()));
                gateway.push(("GATEWAY_METAGAME_URL".into(), local(METAGAME_PORT)));
                gateway.push((key, value));
            }
            _ => gateway.push((key, value)),
        }
    }

    LegacyEnvFiles {
        metagame: owned(metagame),
        deploy: owned(deploy),
        content: owned(content),
        gateway,
        allowlist: reborn.allowlist,
    }
}

/// Setup's step: the env files and the game-server key file, owner-only. Returns the files written.
pub fn write_env_files(
    c: &HostConfig,
    s: &Secrets,
    p: &Paths,
    source_commit: &str,
    platform: Platform,
) -> Result<Vec<PathBuf>, HostError> {
    for dir in [&p.branding, &p.gameserver_logs] {
        std::fs::create_dir_all(dir).map_err(|e| HostError::io(dir, e))?;
    }
    if let Some(dir) = database(p).parent() {
        std::fs::create_dir_all(dir).map_err(|e| HostError::io(dir, e))?;
    }
    if platform == Platform::Linux {
        write_run_game_wrapper(p)?;
    }
    let files = build_legacy_env_files(c, s, p, source_commit, platform);
    let mut written = write_named_env_files(p, &files.files())?;
    let key = gameserver_key_file(p);
    write_secret_file(&key, &format!("{}\n", s.gameserver_key))?;
    written.push(key);
    Ok(written)
}

// ------------------------------------------------------------------ start

/// The metagame's command line: `metagame <args>` with only the system's variables from the shell.
fn metagame(programs: &Programs, paths: &Paths, args: &[&str]) -> Result<(), HostError> {
    let bin = programs.rust_bin("metagame").ok_or_else(|| missing_bin("metagame"))?;
    let options = crate::run::RunOptions {
        cwd: paths.workspace.clone(),
        env: Some(service_environment(std::env::vars(), [], &[])),
        log_file: Some(paths.logs.join("metagame.log")),
        quiet: true,
    };
    let r = crate::run::run(&bin.display().to_string(), args, &options)?;
    if r.code != 0 {
        return Err(HostError::message(format!(
            "metagame {}: {}",
            args.first().copied().unwrap_or_default(),
            r.last_line.unwrap_or_else(|| format!("exit code {}", r.code))
        )));
    }
    Ok(())
}

fn missing_bin(name: &str) -> HostError {
    HostError::message(format!(
        "a legacy server needs the {name} service built with the legacy feature: the gateway, content server and allowlist inside this binary, the metagame next to it or in rust/target (cargo build --release -p server-console -p metagame --features host/legacy)"
    ))
}

/// The services after the metagame's database is ready, in the order they start: metagame, deploy
/// server, content, the allowlist (public, unless systemd runs it), the gateway last. All Rust bins.
pub fn legacy_service_plan(
    paths: &Paths,
    config: &HostConfig,
    external_allowlist: bool,
    programs: &Programs,
) -> Result<Vec<Planned>, HostError> {
    let minute = Duration::from_secs(60);
    let bin = |name: &str| programs.rust_bin(name).ok_or_else(|| missing_bin(name));
    let serve = |name: &str, command: PathBuf| crate::commands::start::binaries::Launch {
        command,
        args: vec![
            "serve".into(),
            "--env-file".into(),
            env_file(paths, name).display().to_string(),
        ],
        cwd: paths.workspace.clone(),
        env_file_in_args: true,
    };
    let front = |service: FrontService| {
        programs
            .launch_rust(service, paths, &env_file(paths, service.bin_name()))
            .ok_or_else(|| missing_bin(service.bin_name()))
    };
    let mut plan = vec![Planned {
        spec: spec_from(
            "metagame",
            serve("metagame", bin("metagame")?),
            env_file(paths, "metagame"),
            Ready::http(METAGAME_PORT, "/dauntless-status"),
        ),
        ready_timeout: Duration::from_secs(120),
        required: true,
    }];
    // DR's deploy server is not in this repository yet: without it the server runs, but no game server starts.
    match programs.rust_bin("deploy") {
        Some(deploy) => plan.push(Planned {
            spec: spec_from(
                "deploy",
                serve("deploy", deploy),
                env_file(paths, "deploy"),
                Ready::http(DEPLOY_PORT, "/gameservers"),
            ),
            ready_timeout: minute,
            required: false,
        }),
        None => {
            say("! No deploy bin (DR's deploy server): the game servers cannot start. Players can sign in, not play.")
        }
    }
    plan.push(Planned {
        spec: spec_from(
            "content",
            front(FrontService::Content)?,
            env_file(paths, "content"),
            Ready::http(PORTS.content, "/content/v1/news"),
        ),
        ready_timeout: minute,
        required: false,
    });
    if config.mode == ServerMode::Public && !external_allowlist {
        plan.push(Planned {
            spec: spec_from(
                "allowlist",
                front(FrontService::Allowlist)?,
                env_file(paths, "allowlist"),
                Ready::http(PORTS.allowlist, "/status"),
            ),
            ready_timeout: minute,
            required: false,
        });
    }
    let gateway_ready = match config.mode {
        ServerMode::Public => Ready::tcp("127.0.0.1", crate::hostconfig::gateway_port(config)),
        ServerMode::Private => Ready::tcp(&config.game_address, PORTS.private_gateway),
    };
    plan.push(Planned {
        spec: spec_from(
            "gateway",
            front(FrontService::Gateway)?,
            env_file(paths, "gateway"),
            gateway_ready,
        ),
        ready_timeout: minute,
        required: true,
    });
    Ok(plan)
}

/// `start` for a legacy server: the reborn start's checks of this PC and its supervisor, with the
/// metagame's database migrated and its game-server key registered first, as DR's install did.
pub fn start(paths: &Paths, config: &HostConfig, external_allowlist: bool) -> Outcome {
    let public = config.mode == ServerMode::Public;
    let external_allowlist = external_allowlist && public;
    let mut names = vec!["metagame", "deploy", "content", "gateway"];
    if public {
        names.push("allowlist");
    }
    require_env_files(paths, &names)?;
    host_checks(paths, config, external_allowlist)?;
    if config.game_dir.is_none() {
        say(
            "! No game folder: the game servers cannot start (reforged-host setup --game <your Dauntless 1.4.4 folder>).",
        );
    }
    let programs = Programs::find_rust(paths);
    let metagame_env = env_file(paths, "metagame").display().to_string();
    metagame(&programs, paths, &["migrate", "--env-file", &metagame_env])?;
    let db = database(paths).display().to_string();
    let key = gameserver_key_file(paths).display().to_string();
    metagame(&programs, paths, &["db", "gs-key", &db, &key])?;
    let plan = legacy_service_plan(paths, config, external_allowlist, &programs)?;

    let controlled = controlled_by_stdin();
    let control = Control::begin(controlled)?;
    let supervisor = Supervisor::new(&paths.logs, console_printer()).with_cancel(control.flag());
    let started = (|| {
        if cfg!(target_os = "linux") && !linux_steps(paths, config, &programs, external_allowlist, &control)? {
            return Ok(Started::Stopped);
        }
        start_services(&supervisor, &control, plan, Some(config), &port_in_use)
    })();
    let started = match started {
        Ok(started) => started,
        Err(error) => {
            supervisor.stop_all();
            return Err(error);
        }
    };
    let close_ports = public && !external_allowlist;
    finish(started, &supervisor, &control, controlled, &up_line(config), || {
        if close_ports {
            close_game_ports(paths, &programs);
        }
    })
}

// ------------------------------------------------------------------ invite

/// A new one-use invite code in the metagame's database (`metagame db add-invite`, DR's kits' `New-Invite`).
/// DR's codes are 20 hex characters.
pub fn create_invite_code(paths: &Paths) -> Result<String, HostError> {
    let mut bytes = [0u8; 10];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| HostError::message("could not make a random invite code"))?;
    let code: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let db = database(paths).display().to_string();
    metagame(&Programs::find_rust(paths), paths, &["db", "add-invite", &db, &code])?;
    Ok(code)
}

// ------------------------------------------------------------------ doctor

/// `doctor` for a legacy server: the bins, the env files, the 1.4.4 game folder with the internal
/// server beside it, the metagame's database and what is running.
pub fn doctor_checks(p: &Paths, config: &HostConfig, hash_exe: bool) -> Result<Vec<Check>, HostError> {
    let mut checks = vec![check(
        "config",
        Level::Ok,
        format!(
            "legacy (1.4.4) {} server \"{}\" at {}, game servers on {}",
            config.mode.as_str(),
            config.name,
            config.host,
            config.game_address
        ),
    )];
    let programs = Programs::find_rust(p);
    checks.push(match programs.rust_bin("metagame") {
        Some(bin) => check("services", Level::Ok, format!("metagame: {}", bin.display())),
        None => fix(
            "services",
            Level::Fail,
            "no metagame bin",
            "build it with the legacy feature (docs/hosting.md, Legacy servers), or use a server release that has it",
        ),
    });
    for service in [FrontService::Gateway, FrontService::Content, FrontService::Allowlist] {
        let name = service.bin_name();
        checks.push(if service.built_in() {
            check(
                "services",
                Level::Ok,
                format!("{name}: built into {}", programs.host.display()),
            )
        } else {
            fix(
                "services",
                Level::Fail,
                format!("this binary has no {name} service"),
                "use a server release, or build it with its default features (docs/hosting.md, Legacy servers)",
            )
        });
    }
    checks.push(match programs.rust_bin("deploy") {
        Some(bin) => check("services", Level::Ok, format!("deploy: {}", bin.display())),
        None => fix(
            "services",
            Level::Fail,
            "no deploy bin (DR's deploy server): the game servers cannot start",
            "DR's deploy server is not ported to this repository yet (docs/hosting.md, Legacy servers)",
        ),
    });
    let mut names = vec!["metagame", "deploy", "content", "gateway"];
    if config.mode == ServerMode::Public {
        names.push("allowlist");
    }
    let mut missing: Vec<String> = names
        .iter()
        .filter(|n| !p.config.join(format!("{n}.env")).exists())
        .map(|n| format!("{n}.env"))
        .collect();
    for file in [p.config.join(crate::hostconfig::SECRET_FILE), gameserver_key_file(p)] {
        if !file.exists() {
            missing.push(
                file.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
        }
    }
    checks.push(if missing.is_empty() {
        check("config", Level::Ok, "env files and secrets written")
    } else {
        fix(
            "config",
            Level::Fail,
            format!("missing {}", missing.join(", ")),
            "reforged-host setup",
        )
    });
    checks.extend(game_exe_checks(
        config.game_dir.as_deref(),
        hash_exe,
        &GameBuild {
            version_text: GAME_VERSION_TEXT,
            exe_sha256: PINNED_EXE_SHA256,
            name: "1.4.4",
            get_it: "use the verified 1.4.4 build Dauntless Revived's kits check (DR's docs)",
        },
    )?);
    if let Some(win64) = config
        .game_dir
        .as_deref()
        .and_then(|dir| Path::new(dir).join(game_exe_relative()).parent().map(Path::to_path_buf))
        .filter(|dir| dir.exists())
    {
        let mut absent: Vec<&str> = SERVER_DLLS
            .iter()
            .copied()
            .filter(|n| !win64.join(n).exists())
            .collect();
        let names_server = std::fs::read_to_string(win64.join(LOADER_INI))
            .is_ok_and(|ini| ini.lines().any(|l| l.trim().eq_ignore_ascii_case(SERVER_DLLS[1])));
        if !names_server {
            absent.push(LOADER_INI);
        }
        checks.push(if absent.is_empty() {
            check(
                "game",
                Level::Ok,
                "the runtime loader loads the internal server beside the game executable",
            )
        } else {
            fix(
                "game",
                Level::Fail,
                format!(
                    "no {} in {} (or {LOADER_INI} does not name {})",
                    absent.join(", "),
                    win64.display(),
                    SERVER_DLLS[1]
                ),
                format!(
                    "put winmm.dll (native/runtime-loader) and UndauntedInternalServer.dll (native/internal-server) there, and a {LOADER_INI} with the line {}",
                    SERVER_DLLS[1]
                ),
            )
        });
    }
    checks.push(if database(p).exists() {
        check(
            "database",
            Level::Ok,
            format!("metagame database {}", database(p).display()),
        )
    } else {
        check(
            "database",
            Level::Info,
            format!("no metagame database yet: start makes it ({})", database(p).display()),
        )
    });
    for (name, port, path) in [
        ("metagame", METAGAME_PORT, "/dauntless-status"),
        ("deploy server", DEPLOY_PORT, "/gameservers"),
        ("content server", PORTS.content, "/content/v1/news"),
    ] {
        let answering = port_taken(port, None)? && wait_for_http(port, path, Duration::from_millis(1500));
        checks.push(check(
            "running",
            Level::Info,
            format!(
                "{name} {} (127.0.0.1:{port})",
                if answering { "is running" } else { "is not running" }
            ),
        ));
    }
    Ok(checks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn programs(dir: &Path, bins: &[&str]) -> Programs {
        let bin_dir = dir.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        for name in bins {
            std::fs::write(bin_dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)), "").unwrap();
        }
        Programs {
            node: PathBuf::from("node"),
            host: bin_dir.join("reforged-host"),
            rust_dirs: vec![bin_dir],
        }
    }

    fn config(mode: ServerMode) -> HostConfig {
        let mut toml = String::from("version = 1\nedition = \"legacy\"\nname = \"Old Hunts\"\n");
        toml.push_str(match mode {
            ServerMode::Private => {
                "mode = \"private\"\nhost = \"100.101.102.103\"\ngame_address = \"100.101.102.103\"\n"
            }
            ServerMode::Public => "mode = \"public\"\nhost = \"old.example.net\"\ngame_address = \"203.0.113.7\"\n",
        });
        toml.push_str("registration = \"INVITECODE\"\nmax_hunt_servers = 3\n[mongo]\nuri = \"mongodb://127.0.0.1:27017\"\nmanage = false\n");
        HostConfig::from_toml(&toml).unwrap()
    }

    fn names(plan: &[Planned]) -> Vec<&str> {
        plan.iter().map(|p| p.spec.name.as_str()).collect()
    }

    #[test]
    fn runs_the_metagame_first_and_the_gateway_last_every_one_a_rust_bin_with_its_env_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let all = ["metagame", "deploy", "content", "allowlist", "gateway"];
        let plan =
            legacy_service_plan(&paths, &config(ServerMode::Public), false, &programs(dir.path(), &all)).unwrap();
        assert_eq!(names(&plan), all);
        for p in &plan {
            let file = paths.config.join(format!("{}.env", p.spec.name)).display().to_string();
            assert_eq!(p.spec.args.last(), Some(&file), "{}", p.spec.name);
            assert_eq!(p.spec.env_file, None, "a Rust bin reads its file itself");
        }
        assert_eq!(plan[0].spec.args[0], "serve");
        assert_eq!(plan[0].spec.ready, Some(Ready::http(61000, "/dauntless-status")));
        assert!(plan[0].required);
        assert_eq!(plan[1].spec.ready, Some(Ready::http(61001, "/gameservers")));
        assert!(!plan[1].required);
        assert_eq!(plan[4].spec.ready, Some(Ready::tcp("127.0.0.1", 443)));

        let private =
            legacy_service_plan(&paths, &config(ServerMode::Private), false, &programs(dir.path(), &all)).unwrap();
        assert_eq!(names(&private), ["metagame", "deploy", "content", "gateway"]);
        assert_eq!(private[3].spec.ready, Some(Ready::tcp("100.101.102.103", 61000)));
    }

    #[test]
    fn needs_the_legacy_bins_but_runs_without_the_deploy_server() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let c = config(ServerMode::Private);
        let error = legacy_service_plan(&paths, &c, false, &programs(dir.path(), &["content", "gateway"])).unwrap_err();
        assert!(
            error.to_string().starts_with("a legacy server needs the metagame bin"),
            "{error}"
        );
        let plan = legacy_service_plan(
            &paths,
            &c,
            false,
            &programs(dir.path(), &["metagame", "content", "gateway"]),
        )
        .unwrap();
        assert_eq!(names(&plan), ["metagame", "content", "gateway"]);
    }
}
