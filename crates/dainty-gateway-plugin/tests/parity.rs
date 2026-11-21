// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The gateway service, run by the core as `host` would (`dainty_controller::service_library::gateway`), answers every
//! recorded script as the Node one did (RD27; the harness says how), compiled in and loaded as a library alike.
//! `cargo build -p dainty-gateway-plugin` first.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

#[path = "../../dainty-gateway/tests/common/mod.rs"]
mod common;
#[path = "../../dainty-gateway/tests/parity/harness.rs"]
mod harness;
mod support;

use std::sync::Arc;

use common::{TestCert, log_lines};
use dainty_controller::Module;
use dainty_controller::service_library::gateway::{
    AllowlistMode, GatewayOptions, gateway_compiled_in, gateway_library,
};
use dainty_service_kit::context::StatusRegistry;
use dainty_service_kit::env::Env;
use harness::Server;

fn options() -> GatewayOptions {
    GatewayOptions {
        allowlist: AllowlistMode::Off,
        ws_port: None,
    }
}

/// GATEWAY_PORT defaults to 443: a free one, found first, and the env with it.
fn with_port(mut pairs: Vec<(String, String)>) -> (u16, Env) {
    let port = support::free_port();
    pairs.push(("GATEWAY_PORT".into(), port.to_string()));
    (
        port,
        Env::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str()))),
    )
}

/// The same service compiled in, as `host::compose::gateway` runs it: its linelog is this process's.
async fn compiled_in(pairs: Vec<(String, String)>, cert: Arc<TestCert>) -> Server {
    let (port, env) = with_port(pairs);
    let (module, _) = gateway_compiled_in(dainty_gateway::service::GatewayService::default, env, None, options());
    let log = log_lines();
    let log = Arc::new(move || log.lock().unwrap().iter().map(|(_, line)| line.clone()).collect());
    run(module, port, log, cert).await
}

async fn through_library(pairs: Vec<(String, String)>, cert: Arc<TestCert>) -> Server {
    let (port, env) = with_port(pairs);
    let (module, _) = gateway_library(support::library().await, env, None, options());
    let log = support::library_log();
    run(module, port, Arc::new(move || log.lock().unwrap().clone()), cert).await
}

/// Runs `module` until it is ready, as the controller would.
async fn run(
    module: Module,
    port: u16,
    log: Arc<dyn Fn() -> Vec<serde_json::Value> + Send + Sync>,
    cert: Arc<TestCert>,
) -> Server {
    let status = StatusRegistry::default();
    let (ctx, stop) = status.service_context("gateway");
    let running = tokio::spawn((module.start)(ctx));
    support::ready(&status, "gateway").await;
    Server {
        port,
        log,
        idle: Box::pin(async {}),
        stop: Box::pin(async move {
            let _ = stop.send(true);
            running.await.unwrap().unwrap();
            drop(cert);
        }),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gateway() {
    harness::run("gateway", compiled_in).await;
    harness::run("gateway", through_library).await;
}
