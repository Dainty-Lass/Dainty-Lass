//! Interactive stdin REPL for `server-console*` binaries (`docs/app-structure-plan.md`).
//!
//! Runs alongside the unified `TableStdoutLayer` (`{service:<14} | {line}`) so an operator can
//! inspect status, restart individual in-process components, generate invite links, adjust per-service
//! log levels via `LogFilterReloadHandle`, trigger backups, or shut down cleanly.

use controller::ServerController;
use service_kit::telemetry::LogFilterReloadHandle;
use tokio::io::{AsyncBufReadExt, BufReader};

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
        "invite" => (
            vec![
                format!("Invite URL: {}", controller.invite_url()),
                format!("Cert SHA-256 (fp): {}", controller.cert_fingerprint()),
            ],
            false,
        ),
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
        "backup" => {
            let db_dir = &controller.provisioned().db_dir;
            let backup_dir = controller.config().workspace_dir.join("backups");
            if let Err(e) = std::fs::create_dir_all(&backup_dir) {
                return (vec![format!("backup failed: {e}")], false);
            }
            let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
            let marker = backup_dir.join(format!("backup-{stamp}.json"));
            let meta = serde_json::json!({
                "timestamp": stamp.to_string(),
                "dbDir": db_dir.display().to_string(),
            });
            match std::fs::write(&marker, meta.to_string()) {
                Ok(()) => (vec![format!("backup recorded at {}", marker.display())], false),
                Err(e) => (vec![format!("backup failed: {e}")], false),
            }
        }
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

/// Runs the interactive stdin loop alongside Ctrl+C signal handling until shutdown is requested.
pub async fn run_repl(controller: &mut ServerController, filter_handle: Option<LogFilterReloadHandle>) {
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                tracing::info!(target: "host", "received Ctrl+C; stopping server");
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
                        // Stdin closed (detached mode): wait on Ctrl+C / stop signal.
                        service_kit::serve::stop_requested().await;
                        break;
                    }
                }
            }
        }
    }
}
