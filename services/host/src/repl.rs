//! The console of `serve` (`docs/app-structure-plan.md`): the server's modules looked at, started,
//! stopped and restarted one by one, log levels changed while it runs, and a clean stop. Its lines go
//! through `tracing`, so they sit in the same table as the services' (`{service:<14} | {line}`).

use controller::ServerController;
use service_kit::telemetry::LogFilterReloadHandle;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::watch;

/// Executes a single operator command against `controller` and returns `(output_lines, should_quit)`.
pub async fn execute_command(
    line: &str,
    controller: &mut ServerController,
    filter_handle: Option<&LogFilterReloadHandle>,
) -> (Vec<String>, bool) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return (Vec::new(), false);
    }
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    match parts[0] {
        "status" => {
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
        "start" => {
            let Some(&svc) = parts.get(1) else {
                return (vec!["Usage: start <service>".into()], false);
            };
            match controller.start_component(svc).await {
                Ok(()) => (vec![format!("started {svc}")], false),
                Err(e) => (vec![format!("error: {e}")], false),
            }
        }
        "stop" if parts.len() > 1 => {
            let svc = parts[1];
            match controller.stop_component(svc).await {
                Ok(()) => (vec![format!("stopped {svc}")], false),
                Err(e) => (vec![format!("error: {e}")], false),
            }
        }
        "restart" => {
            let Some(&svc) = parts.get(1) else {
                return (vec!["Usage: restart <service>".into()], false);
            };
            match controller.restart_component(svc).await {
                Ok(()) => (vec![format!("restarted {svc}")], false),
                Err(e) => (vec![format!("error: {e}")], false),
            }
        }
        "invite" => {
            let snap = controller.snapshot();
            let mut out = vec!["An invite is made for one player: reforged-host invite --for \"Alex\"".to_string()];
            if let Some(fp) = &snap.cert_fingerprint {
                out.push(format!("Certificate SHA-256 (every invite's fp): {fp}"));
            }
            (out, false)
        }
        "players" => {
            let snap = controller.snapshot();
            (vec![format!("Connected players: {}", snap.online_players)], false)
        }
        "instances" => {
            let snap = controller.snapshot();
            (
                vec![format!("Active hunt instances: {}/{}", snap.active_hunts, snap.max_hunts)],
                false,
            )
        }
        "doctor" => {
            let snap = controller.snapshot();
            let mut out = Vec::new();
            for comp in &snap.components {
                out.push(format!(
                    "[{}] {}: {} ({})",
                    if comp.phase == service_kit::context::ComponentPhase::Ready {
                        "OK"
                    } else {
                        "WARN"
                    },
                    comp.name,
                    comp.phase.as_str(),
                    comp.detail
                ));
            }
            (out, false)
        }
        "backup" => (
            vec!["The database is backed up from another terminal: reforged-host backup".into()],
            false,
        ),
        "log" => {
            let (Some(&svc), Some(&level)) = (parts.get(1), parts.get(2)) else {
                return (vec!["Usage: log <service> <trace|debug|info|warn|error>".into()], false);
            };
            let Some(handle) = filter_handle else {
                return (vec!["dynamic log filter handle unavailable".into()], false);
            };
            match handle.set_service_level_str(svc, level) {
                Ok(()) => (vec![format!("set log level for {svc} to {level}")], false),
                Err(e) => (vec![format!("error: {e}")], false),
            }
        }
        "help" | "?" => (
            vec![
                "Commands: status | start <svc> | stop <svc> | restart <svc> | invite | players | instances | doctor | backup | log <svc> <level> | stop".into(),
            ],
            false,
        ),
        "stop" | "quit" | "exit" | "shutdown" => (vec!["shutting down server...".into()], true),
        other => (
            vec![format!("unknown command `{other}` (type `help` for available commands)")],
            false,
        ),
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
