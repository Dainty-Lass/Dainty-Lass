//! Legacy servers (the `legacy` feature; RD5 reversed): Dauntless Revived's 1.4.4 stack, set up and run
//! by the same kit. In place of Mystic Paradox's backend, Director and MongoDB, a legacy server runs DR's
//! metagame (its own SQLite database) and DR's deploy server, which starts the 1.4.4 game servers as DR's
//! deploy kits did, the internal server (`native/internal-server` in legacy-revived) loaded by the runtime loader
//! (`deploy/unix-common/generate-config.mjs`, `deploy/windows-server/Stack.ps1` at DR 24b7624). The content server,
//! the allowlist and the gateway are the reborn ones, told `legacy` in their env files.
//!
//! Each DR variable keeps its name with the service in front in its env file: the metagame's `PORT` is
//! `METAGAME_PORT`, the deploy server's `DEPLOY_PORT`, so a stale one in the shell never reaches them
//! ([`crate::services`]). The metagame and the deploy server are legacy-revived's own binaries since DS6,
//! built in `modules/legacy-revived/rust`, and read DR's names: [`dr_environment`] hands them the file
//! with the prefix taken off, until RV1 puts them on `module-api`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use aws_lc_rs::rand::{SecureRandom, SystemRandom};

use crate::commands::start::binaries::{FrontService, Programs};
use crate::commands::start::{Planned, env_file, require_env_files, say, spec_from};
use crate::doctor::{Check, GameBuild, Level, check, fix, game_exe_checks, port_in_use as port_taken, wait_for_http};
use crate::error::HostError;
use crate::hostconfig::read_env_file;
use crate::hostconfig::{
    Edition, EnvEntries, HostConfig, LEGACY_RULE_NAME, Platform, Secrets, ServerMode, build_env_files,
    legacy_game_ports, legacy_gateway_port, linux_options, path_text, write_named_env_files, write_secret_file,
};
use crate::paths::{BOTH_LEGACY_PORTS, LegacyPorts, PORTS, Paths, game_exe_relative};
use crate::services::ServiceSpec;
use crate::services::{Ready, service_environment};

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
    let deploy = Programs::find_rust(p).rust_bin("deploy").unwrap_or_else(|| {
        crate::linux::host_binary(p).with_file_name(format!("deploy{}", std::env::consts::EXE_SUFFIX))
    });
    let script = [
        "#!/bin/sh".to_owned(),
        "# DaintyLass: DR's deploy server starts this for each 1.4.4 game server. Written by `reforged-host setup`."
            .to_owned(),
        format!(
            "exec {} run-game \"$@\"",
            crate::linux::sh_quote(&deploy.to_string_lossy())
        ),
        String::new(),
    ]
    .join("\n");
    let target = run_game_wrapper(p);
    std::fs::create_dir_all(&p.bin).map_err(|e| HostError::io(&p.bin, e))?;
    std::fs::write(&target, script).map_err(|e| HostError::io(&target, e))?;
    crate::linux::set_mode(&target, 0o755)?;
    Ok(target)
}

/// Where the 1.4.4 stack runs: alone, on the reborn stack's ports and env file names as it always has, or
/// beside the reborn stack (`edition = "both"`, RD26) on [`BOTH_LEGACY_PORTS`], its content server, gateway
/// and allowlist under `legacy-` names, its game servers from `legacy_game_dir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyLayout {
    pub ports: LegacyPorts,
    /// The 1.4.4 gateway's port, public or private
    pub gateway_port: u16,
    /// The Dauntless 1.4.4 folder
    pub game_dir: Option<String>,
    both: bool,
}

impl LegacyLayout {
    pub fn of(c: &HostConfig) -> Self {
        let both = c.edition == Edition::Both;
        let (game_first, game_last) = legacy_game_ports(c);
        let ports = if both {
            BOTH_LEGACY_PORTS
        } else {
            LegacyPorts {
                metagame: METAGAME_PORT,
                deploy: DEPLOY_PORT,
                content: PORTS.content,
                allowlist: PORTS.allowlist,
                private_gateway: PORTS.private_gateway,
                chat: CHAT_PORT,
                game_first,
                game_last,
            }
        };
        Self {
            ports,
            gateway_port: legacy_gateway_port(c),
            game_dir: if both {
                c.legacy_game_dir.clone()
            } else {
                c.game_dir.clone()
            },
            both,
        }
    }

    /// A front service's name, for its env file and its module: `content`, or `legacy-content` beside reborn.
    pub fn name(&self, service: &str) -> String {
        if self.both {
            format!("legacy-{service}")
        } else {
            service.to_owned()
        }
    }

    fn game_ports_text(&self, separator: &str) -> String {
        format!("{}{separator}{}", self.ports.game_first, self.ports.game_last)
    }
}

/// The 1.4.4 stack's env files: metagame, deploy, content, gateway and (public) allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyEnvFiles {
    pub metagame: EnvEntries,
    pub deploy: EnvEntries,
    pub content: EnvEntries,
    pub gateway: EnvEntries,
    pub allowlist: Option<EnvEntries>,
    layout: LegacyLayout,
}

impl LegacyEnvFiles {
    /// `(name, entries)` in the order they are written; `None` removes a file. A 1.4.4-only server removes
    /// a reborn server's files; beside reborn nothing is removed.
    pub fn files(&self) -> Vec<(String, Option<&EnvEntries>)> {
        let mut files = vec![
            ("metagame".to_owned(), Some(&self.metagame)),
            ("deploy".to_owned(), Some(&self.deploy)),
            (self.layout.name("content"), Some(&self.content)),
            (self.layout.name("gateway"), Some(&self.gateway)),
            (self.layout.name("allowlist"), self.allowlist.as_ref()),
        ];
        if !self.layout.both {
            files.extend(["backend", "director", "admin"].map(|n| (n.to_owned(), None)));
        }
        files
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
    let layout = LegacyLayout::of(c);
    let ports = layout.ports;
    // The gateway's and the allowlist's files are the reborn ones, with the game named, the metagame's
    // chat in place of the backend's realtime listener, and (beside reborn) the 1.4.4 stack's own ports.
    let reborn = build_env_files(c, s, p, source_commit, platform);

    let metagame = vec![
        ("METAGAME_PORT", ports.metagame.to_string()),
        // Plain HTTP on this machine only: the gateway is the front door in both modes.
        ("METAGAME_BIND_HOST", "127.0.0.1".into()),
        ("METAGAME_NODE_ENV", "production".into()),
        ("METAGAME_AUTH_MODE", "APIKEY".into()),
        ("METAGAME_AUTH_SIGNING_PRIVKEY_B64", s.jwt_private_key_b64.clone()),
        ("METAGAME_AUTH_SIGNING_PUBKEY_B64", s.jwt_public_key_b64.clone()),
        ("METAGAME_REGISTRATION_MODE", c.registration.as_str().into()),
        ("METAGAME_DB_FILENAME", path_text(&database(p))),
        ("METAGAME_MATCHMAKING_MODE", "DEPLOYSERVER".into()),
        ("METAGAME_DEPLOYSERVER_URL", format!("127.0.0.1:{}", ports.deploy)),
        // The launcher answers the game's QoS ping itself, on its relay, as for reborn.
        ("METAGAME_QOS_TARGET_URL", format!("{}/QoS", local(PORTS.relay))),
        ("METAGAME_TARGET_CHANGELIST", GAME_CHANGELIST.to_string()),
        ("METAGAME_SERVER_NAME", c.name.clone()),
        ("METAGAME_SOURCE_URL", SOURCE_URL.into()),
        ("METAGAME_GIT_COMMIT", source_commit.into()),
        ("METAGAME_CONTENT_PORT", ports.content.to_string()),
        ("METAGAME_BODY_LOG_FILE", path_text(&p.logs.join("metagame-bodies.log"))),
        ("METAGAME_LOG_BODIES", "0".into()),
        ("METAGAME_CHAT", "1".into()),
        ("METAGAME_CHAT_PORT", ports.chat.to_string()),
        ("METAGAME_CHAT_BIND_HOST", "127.0.0.1".into()),
        // DR sent it in public mode only; Reforged's gateway proxies in both.
        ("METAGAME_GATEWAY_SECRET", s.gateway_secret.clone()),
    ];

    let lx = linux_options(c);
    let exe = layout
        .game_dir
        .as_ref()
        .map(|dir| Path::new(dir).join(game_exe_relative()));
    let exe_text = exe.as_deref().map(path_text).unwrap_or_default();
    let mut deploy: Vec<(&str, String)> = vec![
        ("DEPLOY_NODE_ENV", "production".into()),
        ("DEPLOY_PORT", ports.deploy.to_string()),
        ("DEPLOY_BIND_HOST", "127.0.0.1".into()),
        ("DEPLOY_MY_IP", c.game_address.clone()),
        // Ramsgate on the last port, the Training Dojo on the one before, hunts below (DR's layout).
        ("DEPLOY_PORT_RANGE_BEGIN", ports.game_first.to_string()),
        ("DEPLOY_PORT_RANGE_END", ports.game_last.to_string()),
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
        ("DEPLOY_METAGAME_URL", local(ports.metagame)),
        // DR's default is relative to the working folder.
        ("DEPLOY_DRAIN_FILE", path_text(&p.config.join("drain.flag"))),
    ]);

    let mut content = vec![
        ("PORT", ports.content.to_string()),
        ("BIND_HOST", "127.0.0.1".into()),
        ("BACKEND_URL", local(ports.metagame)),
        ("CONTENT_GAME", "legacy".into()),
    ];
    // The 1.4.4 files players download (DR's content server).
    if let Some(dir) = &layout.game_dir {
        content.push(("CONTENT_GAME_DIR", dir.clone()));
    }
    content.extend([
        ("CONTENT_BRANDING_DIR", path_text(&p.branding)),
        ("CONTENT_NEWS_FILE", path_text(&p.news_file)),
    ]);

    let mut gateway: EnvEntries = Vec::new();
    for (key, value) in reborn.gateway {
        match key.as_str() {
            "GATEWAY_WS_URL" => gateway.push((key, local(ports.chat))),
            "GATEWAY_BACKEND_URL" => {
                gateway.push(("GATEWAY_GAME".into(), "legacy".into()));
                gateway.push(("GATEWAY_METAGAME_URL".into(), local(ports.metagame)));
                gateway.push((key, value));
            }
            "GATEWAY_PORT" => gateway.push((key, layout.gateway_port.to_string())),
            "GATEWAY_CONTENT_URL" => gateway.push((key, local(ports.content))),
            "ALLOWLIST_URL" => gateway.push((key, local(ports.allowlist))),
            _ => gateway.push((key, value)),
        }
    }

    // Beside reborn the second helper owns its own table, rule, state and ports (D3)
    let allowlist = reborn.allowlist.map(|entries| {
        let mut out: EnvEntries = Vec::new();
        for (key, value) in entries {
            let value = match key.as_str() {
                "ALLOWLIST_PORT" => ports.allowlist.to_string(),
                "ALLOWLIST_PORTS" => layout.game_ports_text("-"),
                "ALLOWLIST_NFT_TABLE" if layout.both => "reforged_revival_legacy".into(),
                "ALLOWLIST_AUDIT_LOG" if layout.both => path_text(&p.logs.join("allowlist-legacy-audit.log")),
                "ALLOWLIST_STATE_FILE" if layout.both => path_text(&p.config.join("allowlist-legacy-state.json")),
                _ => value,
            };
            out.push((key, value));
        }
        if layout.both {
            out.push(("ALLOWLIST_RULE_NAME".into(), LEGACY_RULE_NAME.into()));
        }
        out
    });

    LegacyEnvFiles {
        metagame: owned(metagame),
        deploy: owned(deploy),
        content: owned(content),
        gateway,
        allowlist,
        layout,
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
    let named = files.files();
    let borrowed: Vec<(&str, Option<&EnvEntries>)> = named.iter().map(|(n, e)| (n.as_str(), *e)).collect();
    let mut written = write_named_env_files(p, &borrowed)?;
    let key = gameserver_key_file(p);
    write_secret_file(&key, &format!("{}\n", s.gameserver_key))?;
    written.push(key);
    Ok(written)
}

/// The env files the 1.4.4 stack runs from, for `start`'s check and a first `serve`.
pub fn env_file_names(c: &HostConfig) -> Vec<String> {
    let layout = LegacyLayout::of(c);
    let mut names = vec![
        "metagame".to_owned(),
        "deploy".to_owned(),
        layout.name("content"),
        layout.name("gateway"),
    ];
    if c.mode == ServerMode::Public {
        names.push(layout.name("allowlist"));
    }
    names
}

// ------------------------------------------------------------------ start

/// legacy-revived's metagame and deploy server read DR's names (`PORT`): `<name>.env`'s settings with
/// the service's prefix (`METAGAME_`) taken off, for their environment in place of `--env-file`.
pub fn dr_environment(paths: &Paths, name: &str) -> Result<Vec<(String, String)>, HostError> {
    let prefix = format!("{}_", name.to_ascii_uppercase());
    let settings = read_env_file(&env_file(paths, name))?;
    Ok(settings
        .iter()
        .map(|(k, v)| {
            let k = k.strip_prefix(prefix.as_str()).filter(|k| !k.is_empty()).unwrap_or(k);
            (k.to_owned(), v.to_owned())
        })
        .collect())
}

/// The metagame's command line: `metagame <args>` with the system's variables from the shell and its
/// settings by DR's names.
fn metagame(programs: &Programs, paths: &Paths, args: &[&str]) -> Result<(), HostError> {
    let bin = programs.rust_bin("metagame").ok_or_else(|| missing_bin("metagame"))?;
    let options = crate::run::RunOptions {
        cwd: paths.workspace.clone(),
        env: Some(service_environment(
            std::env::vars(),
            std::iter::empty::<(&str, &str)>(),
            &dr_environment(paths, "metagame")?,
        )),
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
        "a legacy server needs the {name} service: the gateway, content server and allowlist inside this binary (cargo build --release -p server-console --features host/legacy), the metagame and deploy server next to it or in modules/legacy-revived/rust/target (cargo build --release -p metagame -p deploy there)"
    ))
}

/// The 1.4.4 stack after its database is ready, in the order it starts: the metagame, the allowlist
/// (public, unless systemd runs it: its table closes the game ports before the deploy server starts any),
/// the deploy server, the content server, the gateway last. Each its own process: the metagame and the
/// deploy server are binaries of their own, the rest this binary's services (RD26).
pub fn legacy_service_plan(
    paths: &Paths,
    config: &HostConfig,
    external_allowlist: bool,
    programs: &Programs,
) -> Result<Vec<Planned>, HostError> {
    let layout = LegacyLayout::of(config);
    let ports = layout.ports;
    let minute = Duration::from_secs(60);
    let bin = |name: &str| programs.rust_bin(name).ok_or_else(|| missing_bin(name));
    let serve = |name: &str, command: PathBuf, ready: Ready| {
        Ok::<_, HostError>(ServiceSpec {
            name: name.into(),
            cwd: paths.workspace.clone(),
            command,
            args: vec!["serve".into()],
            env_file: None,
            extra_env: dr_environment(paths, name)?,
            ready: Some(ready),
        })
    };
    let front = |service: FrontService| {
        let name = layout.name(service.bin_name());
        let file = env_file(paths, &name);
        let launch = programs
            .launch_rust(service, paths, &file)
            .ok_or_else(|| missing_bin(service.bin_name()))?;
        Ok::<_, HostError>((name, launch, file))
    };
    let mut plan = vec![Planned {
        spec: serve(
            "metagame",
            bin("metagame")?,
            Ready::http(ports.metagame, "/dauntless-status"),
        )?,
        ready_timeout: Duration::from_secs(120),
        required: true,
    }];
    if config.mode == ServerMode::Public && !external_allowlist {
        let (name, launch, file) = front(FrontService::Allowlist)?;
        plan.push(Planned {
            spec: spec_from(&name, launch, file, Ready::http(ports.allowlist, "/status")),
            ready_timeout: minute,
            required: true,
        });
    }
    // Without DR's deploy server the server runs, but no game server starts.
    match programs.rust_bin("deploy") {
        Some(deploy) => plan.push(Planned {
            spec: serve("deploy", deploy, Ready::http(ports.deploy, "/gameservers"))?,
            ready_timeout: minute,
            required: false,
        }),
        None => say(
            "! No deploy bin (DR's deploy server): the 1.4.4 game servers cannot start. Players can sign in, not play.",
        ),
    }
    let (name, launch, file) = front(FrontService::Content)?;
    plan.push(Planned {
        spec: spec_from(&name, launch, file, Ready::http(ports.content, "/content/v1/news")),
        ready_timeout: minute,
        required: false,
    });
    let gateway_ready = match config.mode {
        ServerMode::Public => Ready::tcp("127.0.0.1", layout.gateway_port),
        ServerMode::Private => Ready::tcp(&config.game_address, layout.gateway_port),
    };
    let (name, launch, file) = front(FrontService::Gateway)?;
    plan.push(Planned {
        spec: spec_from(&name, launch, file, gateway_ready),
        ready_timeout: minute,
        required: true,
    });
    Ok(plan)
}

/// Before the 1.4.4 stack starts: its env files there, its database migrated and the game-server key
/// registered (as DR's install did), and a missing game folder said.
pub fn prepare(paths: &Paths, config: &HostConfig) -> Result<(), HostError> {
    let names = env_file_names(config);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    require_env_files(paths, &names)?;
    if LegacyLayout::of(config).game_dir.is_none() {
        say(if config.edition == Edition::Both {
            "! No 1.4.4 game folder: its game servers cannot start (reforged-host setup --legacy-game <your Dauntless 1.4.4 folder>)."
        } else {
            "! No game folder: the game servers cannot start (reforged-host setup --game <your Dauntless 1.4.4 folder>)."
        });
    }
    let programs = Programs::find_rust(paths);
    metagame(&programs, paths, &["migrate"])?;
    let db = database(paths).display().to_string();
    let key = gameserver_key_file(paths).display().to_string();
    metagame(&programs, paths, &["db", "gs-key", &db, &key])?;
    Ok(())
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
            "build it in modules/legacy-revived/rust (cargo build --release -p metagame -p deploy; docs/hosting.md, Legacy servers)",
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
            "build it in modules/legacy-revived/rust (cargo build --release -p metagame -p deploy; docs/hosting.md, Legacy servers)",
        ),
    });
    let layout = LegacyLayout::of(config);
    let names = env_file_names(config);
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
        layout.game_dir.as_deref(),
        hash_exe,
        &GameBuild {
            version_text: GAME_VERSION_TEXT,
            exe_sha256: PINNED_EXE_SHA256,
            name: "1.4.4",
            get_it: "use the verified 1.4.4 build Dauntless Revived's kits check (DR's docs)",
        },
    )?);
    if let Some(win64) = layout
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
                    "put winmm.dll (native/runtime-loader) and UndauntedInternalServer.dll (modules/legacy-revived/native/internal-server) there, and a {LOADER_INI} with the line {}",
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
        ("metagame", layout.ports.metagame, "/dauntless-status"),
        ("deploy server", layout.ports.deploy, "/gameservers"),
        ("1.4.4 content server", layout.ports.content, "/content/v1/news"),
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
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(
            paths.config.join("metagame.env"),
            "METAGAME_PORT=61000\nMETAGAME_DB_FILENAME=/w/undaunted.db\n",
        )
        .unwrap();
        std::fs::write(
            paths.config.join("deploy.env"),
            "DEPLOY_DR_GAME_EXE=/g/x.exe\nDEPLOY_METAGAME_URL=u\n",
        )
        .unwrap();
        let all = ["metagame", "deploy", "content", "allowlist", "gateway"];
        let plan =
            legacy_service_plan(&paths, &config(ServerMode::Public), false, &programs(dir.path(), &all)).unwrap();
        assert_eq!(names(&plan), ["metagame", "allowlist", "deploy", "content", "gateway"]);
        for p in &plan {
            assert_eq!(
                p.spec.env_file, None,
                "a Rust bin reads its file itself, or is given DR's names"
            );
            if ["metagame", "deploy"].contains(&p.spec.name.as_str()) {
                assert_eq!(p.spec.args, ["serve"], "{}", p.spec.name);
                continue;
            }
            let file = paths.config.join(format!("{}.env", p.spec.name)).display().to_string();
            assert_eq!(p.spec.args.last(), Some(&file), "{}", p.spec.name);
        }
        let env = |p: &Planned| {
            let mut env = p.spec.extra_env.clone();
            env.sort();
            env
        };
        let pair = |k: &str, v: &str| (k.to_owned(), v.to_owned());
        assert_eq!(
            env(&plan[0]),
            [pair("DB_FILENAME", "/w/undaunted.db"), pair("PORT", "61000")]
        );
        assert_eq!(
            env(&plan[2]),
            [pair("DR_GAME_EXE", "/g/x.exe"), pair("METAGAME_URL", "u")]
        );
        assert_eq!(plan[0].spec.ready, Some(Ready::http(61000, "/dauntless-status")));
        assert!(plan[0].required);
        assert_eq!(plan[1].spec.ready, Some(Ready::http(61005, "/status")));
        assert_eq!(plan[2].spec.ready, Some(Ready::http(61001, "/gameservers")));
        assert!(!plan[2].required);
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
            error
                .to_string()
                .starts_with("a legacy server needs the metagame service"),
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

    fn both(mode: ServerMode) -> HostConfig {
        let mut toml = String::from("version = 1\nedition = \"both\"\nname = \"Two Hunts\"\n");
        toml.push_str(match mode {
            ServerMode::Private => {
                "mode = \"private\"\nhost = \"100.101.102.103\"\ngame_address = \"100.101.102.103\"\n"
            }
            ServerMode::Public => "mode = \"public\"\nhost = \"two.example.net\"\ngame_address = \"203.0.113.7\"\n",
        });
        toml.push_str("game_dir = \"/games/1.14.7\"\nlegacy_game_dir = \"/games/1.4.4\"\n");
        toml.push_str("registration = \"INVITECODE\"\nmax_hunt_servers = 3\n[mongo]\nuri = \"mongodb://127.0.0.1:27017\"\nmanage = false\n");
        HostConfig::from_toml(&toml).unwrap()
    }

    #[test]
    fn beside_reborn_the_1_4_4_stack_has_its_own_ports_files_and_firewall_rule() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let c = both(ServerMode::Public);
        let files = build_legacy_env_files(&c, &Secrets::default(), &paths, "abc", Platform::Linux);
        let names: Vec<String> = files.files().into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            [
                "metagame",
                "deploy",
                "legacy-content",
                "legacy-gateway",
                "legacy-allowlist"
            ],
            "nothing of reborn's removed"
        );
        let get = |entries: &EnvEntries, key: &str| entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        assert_eq!(get(&files.metagame, "METAGAME_PORT").as_deref(), Some("61010"));
        assert_eq!(get(&files.deploy, "DEPLOY_PORT").as_deref(), Some("61011"));
        assert_eq!(get(&files.deploy, "DEPLOY_PORT_RANGE_BEGIN").as_deref(), Some("8780"));
        assert_eq!(get(&files.deploy, "DEPLOY_PORT_RANGE_END").as_deref(), Some("8787"));
        assert!(get(&files.deploy, "DEPLOY_DR_GAME_EXE").is_some_and(|e| e.contains("1.4.4")));
        assert_eq!(get(&files.content, "PORT").as_deref(), Some("61012"));
        assert_eq!(get(&files.content, "CONTENT_GAME_DIR").as_deref(), Some("/games/1.4.4"));
        assert_eq!(get(&files.gateway, "GATEWAY_PORT").as_deref(), Some("4443"));
        assert_eq!(
            get(&files.gateway, "GATEWAY_CONTENT_URL").as_deref(),
            Some("http://127.0.0.1:61012")
        );
        assert_eq!(
            get(&files.gateway, "GATEWAY_METAGAME_URL").as_deref(),
            Some("http://127.0.0.1:61010")
        );
        assert_eq!(
            get(&files.gateway, "ALLOWLIST_URL").as_deref(),
            Some("http://127.0.0.1:61015")
        );
        let allowlist = files.allowlist.as_ref().unwrap();
        assert_eq!(get(allowlist, "ALLOWLIST_PORT").as_deref(), Some("61015"));
        assert_eq!(get(allowlist, "ALLOWLIST_PORTS").as_deref(), Some("8780-8787"));
        assert_eq!(
            get(allowlist, "ALLOWLIST_NFT_TABLE").as_deref(),
            Some("reforged_revival_legacy")
        );
        assert_eq!(get(allowlist, "ALLOWLIST_RULE_NAME").as_deref(), Some(LEGACY_RULE_NAME));
        assert!(get(allowlist, "ALLOWLIST_STATE_FILE").is_some_and(|f| f.contains("allowlist-legacy-state")));

        let alone = build_legacy_env_files(
            &config(ServerMode::Public),
            &Secrets::default(),
            &paths,
            "abc",
            Platform::Linux,
        );
        assert_eq!(
            get(&alone.gateway, "GATEWAY_PORT").as_deref(),
            Some("443"),
            "a 1.4.4-only server as before"
        );
        assert_eq!(get(alone.allowlist.as_ref().unwrap(), "ALLOWLIST_RULE_NAME"), None);
        assert_eq!(
            get(alone.allowlist.as_ref().unwrap(), "ALLOWLIST_NFT_TABLE").as_deref(),
            Some("reforged_revival")
        );
    }

    #[test]
    fn beside_reborn_the_plan_runs_the_legacy_named_services_on_their_ports() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let all = ["metagame", "deploy"];
        let plan = legacy_service_plan(&paths, &both(ServerMode::Public), false, &programs(dir.path(), &all)).unwrap();
        assert_eq!(
            names(&plan),
            [
                "metagame",
                "legacy-allowlist",
                "deploy",
                "legacy-content",
                "legacy-gateway"
            ]
        );
        assert_eq!(plan[0].spec.ready, Some(Ready::http(61010, "/dauntless-status")));
        assert_eq!(plan[4].spec.ready, Some(Ready::tcp("127.0.0.1", 4443)));
        let file = paths.config.join("legacy-gateway.env").display().to_string();
        assert_eq!(plan[4].spec.args.last(), Some(&file));
        let private =
            legacy_service_plan(&paths, &both(ServerMode::Private), false, &programs(dir.path(), &all)).unwrap();
        assert_eq!(
            private.last().unwrap().spec.ready,
            Some(Ready::tcp("100.101.102.103", 61010))
        );
    }
}
