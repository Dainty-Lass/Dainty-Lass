//! `admin serve`: the timeouts file, the schema, a listener per bind address, and the sweep that
//! lifts timeouts as they run out.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::net::TcpListener;

use crate::app::{AppState, router};
use crate::config::Config;
use crate::log;
use crate::modules::AdminPlugin;
use crate::schema::{Admin, build};
use crate::timeouts::Timeouts;
use crate::upstream::{Clock, Upstream};

/// How often due timeouts are lifted: a timeout ends at most this late.
pub const SWEEP_EVERY: Duration = Duration::from_secs(30);
const DRAIN: Duration = Duration::from_secs(5);

pub fn prepare(config: &Config, clock: Clock) -> Result<(AppState, Arc<Admin>), String> {
    prepare_with_modules(config, clock, Arc::new(AdminPlugin::none()))
}

/// As `prepare`, with the game modules the controller attached to `modules` (DF4c).
pub fn prepare_with_modules(
    config: &Config,
    clock: Clock,
    modules: Arc<AdminPlugin>,
) -> Result<(AppState, Arc<Admin>), String> {
    let admin = Arc::new(Admin {
        upstream: Upstream::new(
            &config.backend_url,
            &config.director_url,
            &config.admin_key,
            &config.operator_secret,
            clock.clone(),
        ),
        modules,
        timeouts: Timeouts::load(&config.timeouts_file)?,
        max_hunt_servers: config.max_hunt_servers,
        server_mode: config.server_mode,
        clock,
    });
    let state = AppState {
        schema: build(admin.clone()),
        api_key: config.api_key.as_str().into(),
    };
    Ok((state, admin))
}

pub async fn run(config: Config) -> Result<(), String> {
    run_with_context(config, service_kit::context::ServiceContext::standalone("admin")).await
}

/// In-process entry point driven by `ServerController` (`docs/app-structure-plan.md`).
pub async fn run_with_context(config: Config, ctx: service_kit::context::ServiceContext) -> Result<(), String> {
    run_with_modules(config, ctx, Arc::new(AdminPlugin::none())).await
}

/// As `run_with_context`, sending kick, ban and lookup to the modules attached to `modules`: the
/// controller registers the same plugin with its core plugins (DF3).
pub async fn run_with_modules(
    config: Config,
    mut ctx: service_kit::context::ServiceContext,
    modules: Arc<AdminPlugin>,
) -> Result<(), String> {
    ctx.status().report_starting("preparing admin API");
    let (state, admin) = prepare_with_modules(&config, Arc::new(crate::now_ms), modules)?;
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let mut servers = Vec::new();
    for host in &config.bind_hosts {
        let ip = Config::bind_address(host)
            .ok_or_else(|| format!("Could not listen on {host}:{}: not an address", config.port))?;
        let listener = TcpListener::bind(SocketAddr::new(ip, config.port))
            .await
            .map_err(|e| format!("Could not listen on {host}:{}: {e}", config.port))?;
        let mut stop = stop_rx.clone();
        let app = router(state.clone());
        servers.push(tokio::spawn(async move {
            let stopped = async move {
                let _ = stop.wait_for(|s| *s).await;
            };
            service_kit::serve::serve(listener, app, stopped, DRAIN).await
        }));
    }
    let sweeper = tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        loop {
            tick.tick().await;
            admin.sweep().await;
        }
    });
    ctx.status().report_ready(Some(config.port), "listening");
    log::info(
        "admin API ready",
        json!({
            "listen": config.bind_hosts.iter().map(|h| format!("{h}:{}", config.port)).collect::<Vec<_>>(),
            "backend": config.backend_url,
            "director": config.director_url,
            "timeouts": config.timeouts_file.display().to_string(),
        }),
    );
    ctx.wait_stop().await;
    ctx.status().report_stopping("stopping");
    log::info("stopping", json!({"signal": "stop"}));
    sweeper.abort();
    let _ = stop_tx.send(true);
    for server in servers {
        if let Ok(Err(error)) = server.await {
            log::warn("listener stopped with an error", json!({"error": error.to_string()}));
        }
    }
    ctx.status().report_stopped("stopped");
    Ok(())
}
