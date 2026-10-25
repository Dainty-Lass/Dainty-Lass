//! In-process game backend library crate (`docs/app-structure-plan.md`).
//!
//! Each active edition (`Edition::Refined` or `Edition::Reforged`) runs its own backend instance
//! inside the unified `ServerController` on its configured loopback port.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use protocol::edition::Edition;
use serde::{Deserialize, Serialize};
use serde_json::json;
use service_kit::context::ServiceContext;
use tokio::net::TcpListener;

const DRAIN: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    pub edition: Edition,
    pub bind_host: String,
    pub port: u16,
    pub server_name: String,
    pub server_mode: String,
    pub registration_mode: String,
    pub max_hunt_servers: u16,
    pub db_dir: Option<PathBuf>,
    pub director_url: String,
}

impl BackendConfig {
    pub fn for_edition(edition: Edition) -> Self {
        Self {
            edition,
            bind_host: "127.0.0.1".into(),
            port: edition.default_backend_port(),
            server_name: "Reforged Revival Server".into(),
            server_mode: "private".into(),
            registration_mode: "open".into(),
            max_hunt_servers: 4,
            db_dir: None,
            director_url: format!("http://127.0.0.1:{}", edition.default_director_port()),
        }
    }
}

#[derive(Clone)]
struct AppState {
    config: Arc<BackendConfig>,
}

pub fn router(config: BackendConfig) -> Router {
    let state = AppState {
        config: Arc::new(config),
    };
    Router::new()
        .route("/dauntless-status", get(handle_dauntless_status))
        .route("/health", get(handle_health))
        .route("/reforged/api/ServerStatus", get(handle_server_status))
        .with_state(state)
}

async fn handle_dauntless_status() -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(json!({
            "environment": "live",
            "status": "up",
            "service": "dauntless"
        })),
    )
}

async fn handle_health(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(json!({
            "ok": true,
            "edition": state.config.edition.as_str(),
            "port": state.config.port,
        })),
    )
}

/// The launcher's contract (`protocol::status::parse_server_status`): an answer without
/// `online: true` is an offline server. This stub lists nobody.
fn server_status_json(config: &BackendConfig) -> serde_json::Value {
    let registration = match config.registration_mode.to_ascii_lowercase().as_str() {
        "open" => "OPEN",
        "invitecode" | "invite" => "INVITECODE",
        _ => "NONE",
    };
    json!({
        "name": config.server_name,
        "online": true,
        "version": env!("CARGO_PKG_VERSION"),
        "commit": "",
        "registration": registration,
        "playersOnline": 0,
        "players": [],
        "instances": [],
        "uptimeSeconds": 0,
        "chat": false,
        "edition": config.edition.as_str(),
    })
}

async fn handle_server_status(State(state): State<AppState>) -> impl IntoResponse {
    (StatusCode::OK, axum::Json(server_status_json(&state.config)))
}

pub async fn run(config: BackendConfig) -> Result<(), String> {
    let name = format!("backend-{}", config.edition.as_str());
    run_with_context(config, ServiceContext::standalone(name)).await
}

/// Starts the edition's backend listener and serves until `ctx` signals stop.
pub async fn run_with_context(config: BackendConfig, ctx: ServiceContext) -> Result<(), String> {
    ctx.status()
        .report_starting(format!("starting {} backend", config.edition.as_str()));
    let ip: IpAddr = config
        .bind_host
        .parse()
        .map_err(|e| format!("invalid backend bind_host {}: {e}", config.bind_host))?;
    let addr = SocketAddr::new(ip, config.port);
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("could not bind backend on {addr}: {e}"))?;
    let bound_port = listener.local_addr().map_or(config.port, |a| a.port());
    ctx.status().report_ready(Some(bound_port), "listening");
    tracing::info!(
        edition = config.edition.as_str(),
        port = bound_port,
        "backend ready on {}:{}",
        config.bind_host,
        bound_port
    );
    let app = router(config);
    let mut stop_rx = ctx.stop_receiver();
    let stopped = async move {
        let _ = stop_rx.wait_for(|s| *s).await;
    };
    let result = service_kit::serve::serve(listener, app, stopped, DRAIN)
        .await
        .map_err(|e| format!("backend serve error: {e}"));
    ctx.status().report_stopped("stopped");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_status_is_online_to_the_launcher() {
        let config = BackendConfig::for_edition(Edition::Reforged);
        let status = protocol::status::parse_server_status(&server_status_json(&config)).unwrap();
        assert!(status.online);
        assert_eq!(status.name, config.server_name);
        assert_eq!(status.registration, Some(protocol::status::RegistrationMode::Open));
        assert_eq!(status.players_online, 0);
    }
}
