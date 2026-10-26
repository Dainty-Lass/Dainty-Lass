//! In-process Director library crate (`docs/app-structure-plan.md`).
//!
//! Manages the hunt server pool and UDP port range for a given [`Edition`].

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
pub struct DirectorConfig {
    pub edition: Edition,
    pub bind_host: String,
    pub port: u16,
    pub game_address: String,
    pub udp_port_begin: u16,
    pub udp_port_end: u16,
    pub max_hunt_servers: u16,
    pub server_root_dir: Option<PathBuf>,
}

impl DirectorConfig {
    pub fn for_edition(edition: Edition) -> Self {
        let (udp_port_begin, udp_port_end) = edition.default_udp_port_range();
        Self {
            edition,
            bind_host: "127.0.0.1".into(),
            port: edition.default_director_port(),
            game_address: "127.0.0.1".into(),
            udp_port_begin,
            udp_port_end,
            max_hunt_servers: 4,
            server_root_dir: None,
        }
    }
}

#[derive(Clone)]
struct AppState {
    config: Arc<DirectorConfig>,
}

pub fn router(config: DirectorConfig) -> Router {
    let state = AppState {
        config: Arc::new(config),
    };
    Router::new()
        .route("/api/instances", get(handle_instances))
        .route("/health", get(handle_health))
        .with_state(state)
}

async fn handle_instances() -> impl IntoResponse {
    (StatusCode::OK, axum::Json(json!([])))
}

async fn handle_health(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(json!({
            "ok": true,
            "edition": state.config.edition.as_str(),
            "port": state.config.port,
            "udpPortBegin": state.config.udp_port_begin,
            "udpPortEnd": state.config.udp_port_end,
        })),
    )
}

pub async fn run(config: DirectorConfig) -> Result<(), String> {
    let name = format!("director-{}", config.edition.as_str());
    run_with_context(config, ServiceContext::standalone(name)).await
}

/// Starts the edition's Director listener and serves until `ctx` signals stop.
pub async fn run_with_context(config: DirectorConfig, ctx: ServiceContext) -> Result<(), String> {
    ctx.status()
        .report_starting(format!("starting {} director", config.edition.as_str()));
    let ip: IpAddr = config
        .bind_host
        .parse()
        .map_err(|e| format!("invalid director bind_host {}: {e}", config.bind_host))?;
    let addr = SocketAddr::new(ip, config.port);
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("could not bind director on {addr}: {e}"))?;
    let bound_port = listener.local_addr().map_or(config.port, |a| a.port());
    ctx.status().report_ready(Some(bound_port), "listening");
    tracing::info!(
        edition = config.edition.as_str(),
        port = bound_port,
        udp_begin = config.udp_port_begin,
        udp_end = config.udp_port_end,
        "director ready on {}:{}",
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
        .map_err(|e| format!("director serve error: {e}"));
    ctx.status().report_stopped("stopped");
    result
}
