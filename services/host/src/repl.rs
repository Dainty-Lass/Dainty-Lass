//! The console of `serve` (`docs/app-structure-plan.md`): the server's modules looked at, started,
//! stopped and restarted one by one, log levels changed while it runs, and a clean stop. Its lines go
//! through `tracing`, so they sit in the same table as the services' (`{service:<14} | {line}`).
//!
//! [`parse_command`] reads a line into a [`ConsoleCommand`] with no side effect; the REPL, the
//! dashboard's prompt and keys, and the `host_console` fuzz target all go through it
//! (docs/security/controller.md).

use controller::ServerController;
use service_kit::telemetry::LogFilterReloadHandle;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::watch;

/// A line longer than this is refused before it is split.
pub const MAX_LINE: usize = 512;
/// A service name: what the host kit calls a module (`legacy-content`), nothing that could reach a
/// terminal as anything but text.
pub const MAX_NAME: usize = 32;

pub const LEVELS: [&str; 6] = ["trace", "debug", "info", "warn", "error", "off"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleCommand {
    Empty,
    Status,
    Start(String),
    Stop(String),
    Restart(String),
    Invite,
    Players,
    Instances,
    Doctor,
    Backup,
    Log { service: String, level: &'static str },
    Help,
    Quit,
}

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
        "help" | "?" => alone(ConsoleCommand::Help),
        "quit" | "exit" | "shutdown" => alone(ConsoleCommand::Quit),
        other if is_service_name(other) => Err(format!("unknown command `{other}` (type `help` for the commands)")),
        _ => Err("unknown command (type `help` for the commands)".into()),
    }
}

/// Runs one command against `controller`: `(lines to show, stop the server)`.
pub async fn run_command(
    command: ConsoleCommand,
    controller: &mut ServerController,
    filter_handle: Option<&LogFilterReloadHandle>,
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
            let mut out = vec!["An invite is made for one player: reforged-host invite --for \"Alex\"".to_string()];
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
                        if comp.phase == service_kit::context::ComponentPhase::Ready {
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
        ConsoleCommand::Backup => said("The database is backed up from another terminal: reforged-host backup".into()),
        ConsoleCommand::Log { service, level } => {
            let Some(handle) = filter_handle else {
                return said("log levels cannot be changed in this console".into());
            };
            match handle.set_service_level_str(&service, level) {
                Ok(()) => said(format!("set log level for {service} to {level}")),
                Err(e) => said(format!("error: {e}")),
            }
        }
        ConsoleCommand::Help => said(
            "Commands: status | start <svc> | stop <svc> | restart <svc> | invite | players | instances | doctor | backup | log <svc> <level> | stop"
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
) -> (Vec<String>, bool) {
    match parse_command(line) {
        Ok(command) => run_command(command, controller, filter_handle).await,
        Err(why) => (vec![why], false),
    }
}

/// Reads commands from stdin until `stop`, the end of stdin, or `stop` (Ctrl+C and the other stop
/// signals, which `serve` turns into it).
pub async fn run_repl(
    controller: &mut ServerController,
    filter_handle: Option<LogFilterReloadHandle>,
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
                        let (replies, quit) = execute_command(&line, controller, filter_handle.as_ref()).await;
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
