// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The gateway in this process answers every recorded request as the Node one did (spec-rd27-gateway.md section 5.1;
//! the harness says how). gateway-plugin's `tests/parity.rs` runs the same scripts compiled in and as a library.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;
#[path = "parity/harness.rs"]
mod harness;

use std::sync::Arc;
use std::time::Duration;

use common::{TestCert, log_lines};
use dainty_gateway::config::Config;
use dainty_service_kit::context::{ComponentPhase, StatusRegistry};
use dainty_service_kit::env::Env;
use harness::Server;

async fn in_process(mut pairs: Vec<(String, String)>, cert: Arc<TestCert>) -> Server {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    pairs.push(("GATEWAY_PORT".into(), port.to_string()));
    let env = Env::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let config = Config::from_env(&env).unwrap();
    let capture = dainty_gateway::server::load_capture(&env).unwrap();
    let log = log_lines();
    let status = StatusRegistry::default();
    let (ctx, stop) = status.service_context("gateway");
    let running = tokio::spawn(dainty_gateway::server::run_with_context(config, capture, ctx));
    for _ in 0..300 {
        match status.snapshot().get("gateway").map(|c| c.phase) {
            Some(ComponentPhase::Ready) => break,
            Some(ComponentPhase::Failed) => panic!("the gateway failed to start"),
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    Server {
        port,
        log: Arc::new(move || log.lock().unwrap().iter().map(|(_, line)| line.clone()).collect()),
        idle: Box::pin(async {}),
        stop: Box::pin(async move {
            _ = stop.send(true);
            running.await.unwrap().unwrap();
            drop(cert);
        }),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gateway() {
    let _serial = common::serial().await;
    harness::run("gateway", in_process).await;
}
