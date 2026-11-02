//! The gateway built as a library, run by the core as `host` would (`controller::service_library::gateway`),
//! answers every recorded script as the Node one did (RD27; the harness says how). `cargo build -p gateway-plugin`
//! first.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

#[path = "../../gateway/tests/common/mod.rs"]
mod common;
#[path = "../../gateway/tests/parity/harness.rs"]
mod harness;
mod support;

use std::sync::Arc;

use common::TestCert;
use controller::service_library::gateway::{AllowlistMode, GatewayOptions, gateway_library};
use harness::Server;
use service_kit::context::StatusRegistry;
use service_kit::env::Env;

async fn through_library(mut pairs: Vec<(String, String)>, cert: Arc<TestCert>) -> Server {
    let log = support::library_log();
    // GATEWAY_PORT defaults to 443: a free one, found first
    let port = support::free_port();
    pairs.push(("GATEWAY_PORT".into(), port.to_string()));
    let env = Env::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let options = GatewayOptions {
        allowlist: AllowlistMode::Off,
        ws_port: None,
    };
    let (module, _) = gateway_library(support::library().await, env, None, options);
    let status = StatusRegistry::default();
    let (ctx, stop) = status.service_context("gateway");
    let running = tokio::spawn((module.start)(ctx));
    support::ready(&status, "gateway").await;
    Server {
        port,
        log: Arc::new(move || log.lock().unwrap().clone()),
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
    harness::run("gateway", through_library).await;
}
