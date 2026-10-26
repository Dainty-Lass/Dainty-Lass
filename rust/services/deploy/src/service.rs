//! The deploy server itself, as `deploy serve` runs it.

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use service_kit::env::Env;

use crate::gameservers::Deploy;
use crate::matchmaker::Matchmaker;
use crate::process::RealPlatform;
use crate::settings::Settings;

/// Serves until a stop request (`shutdown` on stdin, Ctrl+C / Ctrl+Break or SIGTERM), with the settings
/// in `env_file` (the environment wins over it).
pub fn serve(env_file: Option<&Path>) -> ExitCode {
    let env = match Env::load(env_file) {
        Ok(env) => env,
        Err(error) => {
            eprintln!("Configuration: {error}");
            return ExitCode::FAILURE;
        }
    };
    let settings = Settings::from_env(&env);
    let level = match service_kit::log::parse_level(settings.get("LOG_LEVEL").as_deref()) {
        Ok(level) => level,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    service_kit::log::init(level, !settings.is("NODE_ENV", "production"));
    if settings.is("SERVER_ROLE", "database-only") {
        eprintln!("Deploy service disabled on database-only node");
        return ExitCode::FAILURE;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(run(settings))
}

async fn run(settings: Settings) -> ExitCode {
    match run_with_context(settings, service_kit::context::ServiceContext::standalone("deploy")).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// In-process entry point driven by `ServerController` (`docs/app-structure-plan.md`).
pub async fn run_with_context(settings: Settings, mut ctx: service_kit::context::ServiceContext) -> Result<(), String> {
    ctx.status().report_starting("preparing deploy server");
    let port = settings
        .number("PORT")
        .filter(|p| p.fract() == 0.0 && (0.0..=65535.0).contains(p))
        .map(|p| p as u16);
    let host = settings.non_empty("BIND_HOST").unwrap_or_else(|| "127.0.0.1".into());
    let (Some(port), Ok(ip)) = (port, host.parse::<IpAddr>()) else {
        let msg = format!(
            "Could not listen on {host}:{:?}: not an address and port",
            settings.get("PORT")
        );
        service_kit::fatal!("{msg}");
        return Err(msg);
    };
    // It starts processes for whoever calls it: never off loopback (app.rs refuses other peers too)
    if !ip.is_loopback() {
        let msg = format!("Could not listen on {host}:{port}: DEPLOY_BIND_HOST must be a loopback address");
        service_kit::fatal!("{msg}");
        return Err(msg);
    }
    let deploy = match Deploy::new(settings, Arc::new(RealPlatform)) {
        Ok(deploy) => deploy,
        Err(error) => {
            let msg = format!("Configuration: {error}");
            service_kit::fatal!("{msg}");
            return Err(msg);
        }
    };
    deploy.cpu_admission.start_sampling();
    let listener = match tokio::net::TcpListener::bind(SocketAddr::new(ip, port)).await {
        Ok(listener) => listener,
        Err(error) => {
            let msg = format!("Could not listen on {host}:{port}: {error}");
            service_kit::fatal!("{msg}");
            return Err(msg);
        }
    };
    let bound_port = listener.local_addr().map_or(port, |a| a.port());
    let matchmaker = Matchmaker::new(deploy.clone());
    let app = crate::app::router(matchmaker).into_make_service_with_connect_info::<SocketAddr>();
    let (stop, stopping) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(async move {
        let mut stopping = stopping;
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = stopping.wait_for(|s| *s).await;
            })
            .await;
    });

    let startup = {
        let deploy = deploy.clone();
        tokio::spawn(async move { deploy.startup_and_report_failure().await })
    };
    let watchdog = {
        let deploy = deploy.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_secs(15));
            every.tick().await;
            loop {
                every.tick().await;
                deploy.run_watchdog().await;
            }
        })
    };
    ctx.status().report_ready(Some(bound_port), "listening");
    tracing::info!("Dauntless Revived deploy server (legacy) on port {bound_port}");
    tracing::info!(
        "Ramsgate and Dojo liveness check before handing them out: {}",
        if deploy.persistent_world_liveness() {
            "on"
        } else {
            "off (DEPLOY_PERSISTENT_WORLD_LIVENESS=0)"
        }
    );
    tracing::info!("Clear Skies, Slayer.");

    ctx.wait_stop().await;
    ctx.status().report_stopping("stopping");
    tracing::info!("stopping: ending the game servers this deploy server started");
    watchdog.abort();
    let _ = stop.send(true);
    deploy.shutdown().await;
    let _ = startup.await;
    let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
    ctx.status().report_stopped("stopped");
    if deploy.startup_failed() {
        Err("deploy startup failed".into())
    } else {
        Ok(())
    }
}
