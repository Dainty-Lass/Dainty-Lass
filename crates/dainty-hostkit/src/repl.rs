// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The console of `serve` (`docs/app-structure-plan.md`): the server's modules looked at, started,
//! stopped and restarted one by one, log levels changed while it runs, and a clean stop. Its lines go
//! through `tracing`, so they sit in the same table as the services' (`{service:<14} | {line}`).
//!
//! [`parse_command`] reads a line into a [`ConsoleCommand`] with no side effect; the REPL, the
//! dashboard's prompt and keys, and the `host_console` fuzz target all go through it
//! (docs/security/controller.md).
//!
//! `module ...` reaches the game modules and plugins `[modules]` lists (DF3): list them, load, unload or
//! restart one, ask its status, run a command it registered. `start`, `stop` and `restart` take a
//! module's id as they take a service's name. A module's answer is printed as `plain` text.

use dainty_controller::ServerController;
use dainty_service_kit::telemetry::LogFilterReloadHandle;

use crate::commands::history::{self, Job, Surface};
use crate::paths::Paths;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::watch;

/// A line longer than this is refused before it is split.
pub const MAX_LINE: usize = 512;
/// A service name: what the host kit calls a module (`legacy-content`), nothing that could reach a
/// terminal as anything but text.
pub const MAX_NAME: usize = 32;

/// The log levels `log <service> <level>` takes.
pub const LEVELS: [&str; 6] = ["trace", "debug", "info", "warn", "error", "off"];

/// A console line, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleCommand {
    /// An empty line.
    Empty,
    /// `status`: every service's state.
    Status,
    /// `start <service>`: a service or a listed module.
    Start(String),
    /// `stop <service>`: a service or a listed module.
    Stop(String),
    /// `restart <service>`: a service or a listed module.
    Restart(String),
    /// `invite`: how to make an invite, and the certificate's fingerprint.
    Invite,
    /// `players`: how many players are connected.
    Players,
    /// `instances`: how many hunts run, of the most allowed.
    Instances,
    /// `doctor`: each service's state, flagged.
    Doctor,
    /// `backup`: where to run a backup from.
    Backup,
    /// `log <service> <level>`: changes a service's log level while it runs.
    Log {
        /// The service.
        service: String,
        /// One of [`LEVELS`].
        level: &'static str,
    },
    /// `modules`: the listed modules.
    Modules,
    /// `module status <id>`.
    ModuleStatus(String),
    /// `module load <id>`.
    ModuleLoad(String),
    /// `module unload <id>`.
    ModuleUnload(String),
    /// `module restart <id>`.
    ModuleRestart(String),
    /// `module run <id> <command> [args...]`: a command the module registered.
    ModuleRun {
        /// The module's id.
        module: String,
        /// The command's name.
        command: String,
        /// Its arguments.
        args: Vec<String>,
    },
    /// `history ...` and `revert ...` (DH3b)
    History(Job),
    /// `help` or `?`.
    Help,
    /// `stop`, `quit`, `exit` or `shutdown`: stops the server.
    Quit,
}

/// Arguments to a module's command: at most this many, each at most [`MAX_ARG`] characters of text.
pub const MAX_ARGS: usize = 16;
/// The most characters one argument to a module's command may have.
pub const MAX_ARG: usize = 128;

/// A service name as the console takes it: 1 to [`MAX_NAME`] of `a-z`, `0-9` and `-`.
pub fn is_service_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn service(parts: &[&str], usage: &str) -> Result<String, String> {
    match parts {
        [_, name] if is_service_name(name) => Ok((*name).to_owned()),
        [_, _] => Err(format!(
            "not a service name (a-z, 0-9 and -, at most {MAX_NAME}): type `status` for the list"
        )),
        _ => Err(format!("usage: {usage}")),
    }
}

/// A line from a service as the console and the dashboard show it: escape sequences and other control
/// characters dropped, tabs as spaces. A Node service's output can echo what a player sent, and the
/// host's terminal must only ever print it.
pub fn plain(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\t' => out.push(' '),
            // CSI: parameters, then one final byte in @..~
            '\u{1b}' if chars.next_if_eq(&'[').is_some() => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC, DCS and the other strings: up to BEL or ST
            '\u{1b}' if chars.next_if(|c| matches!(c, ']' | 'P' | 'X' | '^' | '_')).is_some() => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            '\u{1b}' => {
                chars.next();
            }
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// The command a line names. An error is the line to show; it never repeats what was typed unless
/// that is a plain word, so nothing typed reaches the terminal as a control sequence.
pub fn parse_command(line: &str) -> Result<ConsoleCommand, String> {
    if line.len() > MAX_LINE {
        return Err(format!("a command is at most {MAX_LINE} characters"));
    }
    let parts: Vec<&str> = line.split_whitespace().collect();
    let Some(&word) = parts.first() else {
        return Ok(ConsoleCommand::Empty);
    };
    let alone = |command: ConsoleCommand| {
        if parts.len() == 1 {
            Ok(command)
        } else {
            Err(format!("`{word}` takes nothing after it"))
        }
    };
    match word {
        "status" => alone(ConsoleCommand::Status),
        "start" => service(&parts, "start <service>").map(ConsoleCommand::Start),
        // `stop` alone stops the server; with a name, that service
        "stop" if parts.len() == 1 => Ok(ConsoleCommand::Quit),
        "stop" => service(&parts, "stop <service>").map(ConsoleCommand::Stop),
        "restart" => service(&parts, "restart <service>").map(ConsoleCommand::Restart),
        "invite" => alone(ConsoleCommand::Invite),
        "players" => alone(ConsoleCommand::Players),
        "instances" => alone(ConsoleCommand::Instances),
        "doctor" => alone(ConsoleCommand::Doctor),
        "backup" => alone(ConsoleCommand::Backup),
        "log" => match parts.as_slice() {
            [_, name, level] if is_service_name(name) => LEVELS
                .into_iter()
                .find(|l| l == level)
                .map(|level| ConsoleCommand::Log {
                    service: (*name).to_owned(),
                    level,
                })
                .ok_or_else(|| format!("a level is one of {}", LEVELS.join(", "))),
            _ => Err(format!("usage: log <service> <{}>", LEVELS.join("|"))),
        },
        "modules" => alone(ConsoleCommand::Modules),
        "module" => parse_module(&parts),
        // dainty-legacy's importers, as `module run legacy import-<kind>`
        "legacy" => match parts.as_slice() {
            [_, "import", kind, args @ ..] if crate::commands::legacy::KINDS.contains(kind) => {
                let command = format!("import-{kind}");
                let mut run = vec!["module", "run", crate::commands::legacy::MODULE_ID, command.as_str()];
                run.extend_from_slice(args);
                parse_module(&run)
            }
            _ => Err(format!(
                "usage: legacy import <{}> [args]",
                crate::commands::legacy::KINDS.join("|")
            )),
        },
        "history" | "revert" => history::parse_console(word, &parts[1..]).map(ConsoleCommand::History),
        "help" | "?" => alone(ConsoleCommand::Help),
        "quit" | "exit" | "shutdown" => alone(ConsoleCommand::Quit),
        other if is_service_name(other) => Err(format!("unknown command `{other}` (type `help` for the commands)")),
        _ => Err("unknown command (type `help` for the commands)".into()),
    }
}

/// `module list|status|load|unload|restart <id>`, `module run <id> <command> [args...]`.
fn parse_module(parts: &[&str]) -> Result<ConsoleCommand, String> {
    const USAGE: &str =
        "usage: module list | module status|load|unload|restart <module> | module run <module> <command> [args]";
    let named = |name: &str, made: fn(String) -> ConsoleCommand| {
        if is_service_name(name) {
            Ok(made(name.to_owned()))
        } else {
            Err(format!(
                "not a module id (a-z, 0-9 and -, at most {MAX_NAME}): type `modules` for the list"
            ))
        }
    };
    match parts {
        [_, "list"] => Ok(ConsoleCommand::Modules),
        [_, "status", name] => named(name, ConsoleCommand::ModuleStatus),
        [_, "load", name] => named(name, ConsoleCommand::ModuleLoad),
        [_, "unload", name] => named(name, ConsoleCommand::ModuleUnload),
        [_, "restart", name] => named(name, ConsoleCommand::ModuleRestart),
        [_, "run", module, command, args @ ..] => {
            if !is_service_name(module) || !is_service_name(command) {
                return Err(format!(
                    "a module id and a command are a-z, 0-9 and -, at most {MAX_NAME}"
                ));
            }
            // Text only, so nothing typed reaches a module, or the terminal again, as a control sequence
            let text = |arg: &&str| arg.chars().count() <= MAX_ARG && !arg.chars().any(char::is_control);
            if args.len() > MAX_ARGS || !args.iter().all(text) {
                return Err(format!("at most {MAX_ARGS} arguments of {MAX_ARG} characters of text"));
            }
            Ok(ConsoleCommand::ModuleRun {
                module: (*module).to_owned(),
                command: (*command).to_owned(),
                args: args.iter().map(|a| (*a).to_owned()).collect(),
            })
        }
        _ => Err(USAGE.into()),
    }
}

/// The listed modules as the console prints them.
fn module_table(registry: &dainty_controller::ModuleRegistry) -> Vec<String> {
    let mut out = vec![format!(
        "{:<20} {:<10} {:<10} {:<12} {:<18} DETAIL",
        "MODULE", "STATE", "VERSION", "SOURCE", "PORTS"
    )];
    let modules = registry.list();
    if modules.is_empty() {
        return vec!["No modules are listed ([modules] in settings.toml).".into()];
    }
    for module in modules {
        let version = module
            .manifest
            .as_ref()
            .map_or_else(|| "-".into(), |m| m.game_version.to_string());
        let ports = module
            .ports
            .iter()
            .map(|(name, range)| {
                if range.first == range.last {
                    format!("{name}={}", range.first)
                } else {
                    format!("{name}={}-{}", range.first, range.last)
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let state = serde_json::to_value(module.state)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        out.push(format!(
            "{:<20} {:<10} {:<10} {:<12} {:<18} {}",
            module.id,
            state,
            version,
            module.source,
            if ports.is_empty() { "-".into() } else { ports },
            plain(module.detail.as_deref().unwrap_or(""))
        ));
    }
    out
}

/// A module's command, run as the console (DH1: the console has no login, so it is the host).
async fn run_module(
    registry: &dainty_controller::ModuleRegistry,
    module: &str,
    command: String,
    args: Vec<String>,
) -> Vec<String> {
    let Ok(id) = module.parse::<dainty_module_api::ModuleId>() else {
        return vec!["not a module id".into()];
    };
    let request = dainty_module_api::CommandRequest {
        actor: dainty_module_api::ActorContext::new(dainty_module_api::Actor::Console),
        name: command,
        args,
    };
    match registry.command(&id, request).await {
        Ok(output) => output.text.lines().map(plain).collect(),
        Err(error) => vec![format!("error: {}", plain(&error.to_string()))],
    }
}

/// The most lines the console prints for one command: a revert of a mass delete lists every row, and the
/// terminal command (`dainty-host revert`) is where to read all of them.
const MAX_REPLY_LINES: usize = 200;

fn capped(lines: Vec<String>) -> Vec<String> {
    let total = lines.len();
    let mut out: Vec<String> = lines.iter().take(MAX_REPLY_LINES).map(|l| plain(l)).collect();
    if total > MAX_REPLY_LINES {
        out.push(format!(
            "... {} more lines: run the same command from a terminal, `dainty-host <command>`, to read them all",
            total - MAX_REPLY_LINES
        ));
    }
    out
}

/// Runs one command against `controller`: `(lines to show, stop the server)`.
pub async fn run_command(
    command: ConsoleCommand,
    controller: &mut ServerController,
    filter_handle: Option<&LogFilterReloadHandle>,
    workspace: Option<&Paths>,
) -> (Vec<String>, bool) {
    let said = |line: String| (vec![line], false);
    match command {
        ConsoleCommand::Empty => (Vec::new(), false),
        ConsoleCommand::Status => {
            let snap = controller.snapshot();
            let mut out = vec![format!(
                "{:<20} {:<10} {:<8} {:<10} DETAIL",
                "COMPONENT", "PHASE", "PORT", "UPTIME(s)"
            )];
            for comp in &snap.components {
                let port = comp.port.map_or_else(|| "-".into(), |p| p.to_string());
                out.push(format!(
                    "{:<20} {:<10} {:<8} {:<10} {}",
                    comp.name,
                    comp.phase.as_str(),
                    port,
                    comp.uptime_secs,
                    comp.detail
                ));
            }
            out.push(format!(
                "players={} active_hunts={}/{}",
                snap.online_players, snap.active_hunts, snap.max_hunts
            ));
            (out, false)
        }
        ConsoleCommand::Start(name) => match controller.start_component(&name).await {
            Ok(()) => said(format!("started {name}")),
            Err(e) => said(format!("error: {e}")),
        },
        ConsoleCommand::Stop(name) => match controller.stop_component(&name).await {
            Ok(()) => said(format!("stopped {name}")),
            Err(e) => said(format!("error: {e}")),
        },
        ConsoleCommand::Restart(name) => match controller.restart_component(&name).await {
            Ok(()) => said(format!("restarted {name}")),
            Err(e) => said(format!("error: {e}")),
        },
        ConsoleCommand::Invite => {
            let snap = controller.snapshot();
            let mut out = vec!["An invite is made for one player: dainty-host invite --for \"Alex\"".to_string()];
            if let Some(fp) = &snap.cert_fingerprint {
                out.push(format!("Certificate SHA-256 (every invite's fp): {fp}"));
            }
            (out, false)
        }
        ConsoleCommand::Players => said(format!("Connected players: {}", controller.snapshot().online_players)),
        ConsoleCommand::Instances => {
            let snap = controller.snapshot();
            said(format!("Active hunt instances: {}/{}", snap.active_hunts, snap.max_hunts))
        }
        ConsoleCommand::Doctor => {
            let snap = controller.snapshot();
            let out = snap
                .components
                .iter()
                .map(|comp| {
                    format!(
                        "[{}] {}: {} ({})",
                        if comp.phase == dainty_service_kit::context::ComponentPhase::Ready {
                            "OK"
                        } else {
                            "WARN"
                        },
                        comp.name,
                        comp.phase.as_str(),
                        comp.detail
                    )
                })
                .collect();
            (out, false)
        }
        ConsoleCommand::Backup => said("The database is backed up from another terminal: dainty-host backup".into()),
        ConsoleCommand::Log { service, level } => {
            let Some(handle) = filter_handle else {
                return said("log levels cannot be changed in this console".into());
            };
            match handle.set_service_level_str(&service, level) {
                Ok(()) => said(format!("set log level for {service} to {level}")),
                Err(e) => said(format!("error: {e}")),
            }
        }
        ConsoleCommand::Modules
        | ConsoleCommand::ModuleStatus(_)
        | ConsoleCommand::ModuleLoad(_)
        | ConsoleCommand::ModuleUnload(_)
        | ConsoleCommand::ModuleRestart(_)
        | ConsoleCommand::ModuleRun { .. } => {
            let Some(registry) = controller.registry().cloned() else {
                return said("this server runs no modules".into());
            };
            let id = |name: &str| name.parse::<dainty_module_api::ModuleId>().map_err(|e| e.to_string());
            let result = match command {
                ConsoleCommand::ModuleStatus(name) => match id(&name) {
                    Ok(id) => match registry.info(&id) {
                        Ok(info) => {
                            let live = registry
                                .module_status(&id)
                                .await
                                .map(|s| format!(" (module says {:?})", s.phase))
                                .unwrap_or_default();
                            let state = serde_json::to_value(info.state).unwrap_or_default();
                            Ok(vec![format!(
                                "{id}: {}{live}{}",
                                state.as_str().unwrap_or_default(),
                                info.detail.map(|d| format!(": {}", plain(&d))).unwrap_or_default()
                            )])
                        }
                        Err(e) => Err(e.to_string()),
                    },
                    Err(e) => Err(e),
                },
                ConsoleCommand::ModuleLoad(name) => match id(&name) {
                    Ok(id) => registry.load(&id).await.map(|_| vec![format!("loaded {id}")]).map_err(|e| e.to_string()),
                    Err(e) => Err(e),
                },
                ConsoleCommand::ModuleUnload(name) => match id(&name) {
                    Ok(id) => registry.unload(&id).await.map(|()| vec![format!("unloaded {id}")]).map_err(|e| e.to_string()),
                    Err(e) => Err(e),
                },
                ConsoleCommand::ModuleRestart(name) => match id(&name) {
                    Ok(id) => registry
                        .restart(&id)
                        .await
                        .map(|_| vec![format!("restarted {id}")])
                        .map_err(|e| e.to_string()),
                    Err(e) => Err(e),
                },
                ConsoleCommand::ModuleRun { module, command, args } => {
                    Ok(run_module(&registry, &module, command, args).await)
                }
                _ => Ok(module_table(&registry)),
            };
            match result {
                Ok(lines) => (lines, false),
                Err(e) => said(format!("error: {}", plain(&e))),
            }
        }
        ConsoleCommand::History(job) => {
            let Some(paths) = workspace else {
                return said("history is not available in this console".into());
            };
            match history::run(paths, &job, Surface::Console).await {
                Ok(done) => (capped(done.lines), false),
                Err(why) => said(format!("error: {}", plain(&why))),
            }
        }
        ConsoleCommand::Help => said(
            "Commands: status | start <svc> | stop <svc> | restart <svc> | modules | module status|load|unload|restart <module> | module run <module> <command> [args] | legacy import <revived|offline-save|reforged> [args] | history [--actor who] [--account id] [--since time] [--limit N] | history verify | revert --actor who [--dry-run | --apply <plan id>] | invite | players | instances | doctor | backup | log <svc> <level> | stop"
                .into(),
        ),
        ConsoleCommand::Quit => (vec!["shutting down server...".into()], true),
    }
}

/// A line typed at the console: parsed, then run.
pub async fn execute_command(
    line: &str,
    controller: &mut ServerController,
    filter_handle: Option<&LogFilterReloadHandle>,
    workspace: Option<&Paths>,
) -> (Vec<String>, bool) {
    match parse_command(line) {
        Ok(command) => run_command(command, controller, filter_handle, workspace).await,
        Err(why) => (vec![why], false),
    }
}

/// Reads commands from stdin until `stop`, the end of stdin, or `stop` (Ctrl+C and the other stop
/// signals, which `serve` turns into it).
pub async fn run_repl(
    controller: &mut ServerController,
    filter_handle: Option<LogFilterReloadHandle>,
    workspace: Option<&Paths>,
    mut stop: watch::Receiver<bool>,
) {
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdin_closed = false;

    loop {
        tokio::select! {
            _ = stop.wait_for(|stopped| *stopped) => {
                tracing::info!(target: "host", "stop asked for; stopping the server");
                break;
            }
            next = lines.next_line() => {
                match next {
                    Ok(Some(line)) => {
                        let (replies, quit) = execute_command(&line, controller, filter_handle.as_ref(), workspace).await;
                        for reply in replies {
                            tracing::info!(target: "host", "{reply}");
                        }
                        if quit {
                            break;
                        }
                    }
                    Ok(None) | Err(_) => {
                        stdin_closed = true;
                        break;
                    }
                }
            }
        }
    }
    // Stdin closed: the server runs on until a stop signal
    if stdin_closed {
        let _ = stop.wait_for(|stopped| *stopped).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn the_console_drives_the_listed_modules() {
        use std::sync::Arc;
        let mut controller = ServerController::new("private", Vec::new());
        let registry = dainty_controller::ModuleRegistry::new(
            dainty_controller::RegistryOptions::standalone(),
            controller.status_registry().clone(),
        );
        registry
            .add_entry(dainty_controller::ModuleEntry {
                id: "stub".parse().unwrap(),
                source: dainty_controller::ModuleSource::CompiledIn(Arc::new(|| {
                    Arc::new(dainty_module_api::stub::StubModule::new()) as Arc<dyn dainty_module_api::GameModule>
                })),
                autoload: false,
                settings: Default::default(),
            })
            .unwrap();
        controller.set_registry(registry);
        let mut run = async |line: &str| execute_command(line, &mut controller, None, None).await.0.join("\n");
        assert!(run("modules").await.contains("not_loaded"));
        assert_eq!(run("module load stub").await, "loaded stub");
        let table = run("modules").await;
        assert!(
            table.contains("ready") && table.contains("0.0.1") && table.contains("http=65000"),
            "{table}"
        );
        assert_eq!(run("module run stub echo a b").await, "hello a b");
        assert!(run("module run stub nope").await.starts_with("error:"));
        assert!(
            run("module status stub")
                .await
                .starts_with("stub: ready (module says Ready)")
        );
        assert!(
            run("status").await.contains("stub"),
            "a module is a component in status"
        );
        assert_eq!(run("restart stub").await, "restarted stub");
        assert_eq!(run("module unload stub").await, "unloaded stub");
        assert!(run("module unload stub").await.contains("not loaded"));
        assert!(run("module load nobody").await.contains("no module nobody"));
    }

    #[test]
    fn reads_each_command_and_refuses_the_rest_without_echoing_it() {
        use ConsoleCommand::*;
        for (line, command) in [
            ("", Empty),
            ("   ", Empty),
            ("status", Status),
            ("  restart  legacy-content ", Restart("legacy-content".into())),
            ("start gateway", Start("gateway".into())),
            ("stop admin", Stop("admin".into())),
            ("stop", Quit),
            ("exit", Quit),
            ("?", Help),
            ("modules", Modules),
            ("module list", Modules),
            ("module status stub", ModuleStatus("stub".into())),
            ("module load stub", ModuleLoad("stub".into())),
            ("module unload stub", ModuleUnload("stub".into())),
            ("module restart stub", ModuleRestart("stub".into())),
            (
                "module run stub echo a b",
                ModuleRun {
                    module: "stub".into(),
                    command: "echo".into(),
                    args: vec!["a".into(), "b".into()],
                },
            ),
            (
                "legacy import revived /srv/metagame.db --dry-run",
                ModuleRun {
                    module: "legacy".into(),
                    command: "import-revived".into(),
                    args: vec!["/srv/metagame.db".into(), "--dry-run".into()],
                },
            ),
            ("history verify", History(Job::Verify)),
            (
                "revert --actor=admin:ab12cd34 --apply abc",
                History(Job::Revert {
                    select: history::Select {
                        actor: Some("admin:ab12cd34".into()),
                        ..history::Select::default()
                    },
                    apply: Some("abc".into()),
                }),
            ),
            (
                "log content debug",
                Log {
                    service: "content".into(),
                    level: "debug",
                },
            ),
        ] {
            assert_eq!(parse_command(line), Ok(command), "{line:?}");
        }
        for (line, said) in [
            ("start", "usage: start <service>"),
            ("restart Gateway", "not a service name"),
            ("restart a b", "usage: restart <service>"),
            ("status now", "takes nothing after it"),
            ("log content loud", "a level is one of"),
            ("log", "usage: log"),
            ("frobnicate", "unknown command `frobnicate`"),
            ("module", "usage: module"),
            ("module load", "usage: module"),
            ("module load Stub", "not a module id"),
            ("module run stub", "usage: module"),
            ("module run stub Echo", "a-z, 0-9"),
            ("module run stub echo a\u{1b}[2J", "of text"),
            ("legacy import metagame", "usage: legacy import"),
            ("legacy", "usage: legacy import"),
            ("modules now", "takes nothing after it"),
            ("revert --limit 5", "usage: revert"),
            ("history --limit", "usage: history"),
            ("history --actor x\u{1b}[2J", "text only"),
        ] {
            assert!(parse_command(line).unwrap_err().contains(said), "{line:?}");
        }
        // A control sequence typed at the console is never written back to the terminal
        let escape = "\u{1b}]0;owned\u{7}";
        assert!(!parse_command(escape).unwrap_err().contains('\u{1b}'));
        assert!(
            !parse_command(&format!("restart {escape}"))
                .unwrap_err()
                .contains('\u{1b}')
        );
        assert!(parse_command(&"x".repeat(MAX_LINE + 1)).is_err());
        assert_eq!(
            plain("\u{1b}[32mready\u{1b}[0m\tport 8443\u{1b}]0;title\u{7} \u{1b}]8;;x\u{1b}\\a\u{9b}\r\u{1b}c"),
            "ready port 8443 a"
        );
        assert!(parse_command(&format!("restart {}", "a".repeat(MAX_NAME + 1))).is_err());
    }
}
