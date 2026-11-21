// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! HTTP parity with the Node server (spec section 3.1): each script's replies must equal the recorded ones. A
//! failure means the Rust side is wrong; the recordings can't be redone (the Node server is gone).

#[path = "parity/harness.rs"]
mod harness;

use content::config::Config;
use harness::Server;
use service_kit::env::Env;
use tokio::net::TcpListener;

async fn launch(env: Vec<(String, String)>) -> Server {
    let config = Config::from_env(&Env::from_pairs(env.iter().map(|(k, v)| (k.as_str(), v.as_str()))))
        .unwrap_or_else(|e| panic!("{e}"));
    let app = content::server::prepare(&config);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap_or_else(|e| panic!("{e}"));
    let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let serving = tokio::spawn(content::server::serve(listener, app, stopped));
    Server {
        port,
        stop: Box::pin(async move {
            _ = stop.send(true);
            _ = serving.await;
        }),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn content() {
    harness::run("content", launch).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn content_none() {
    harness::run("content-none", launch).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn content_invalid() {
    harness::run("content-invalid", launch).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn content_index() {
    harness::run("content-index", launch).await;
}

/// A script written for this rewrite from the spec alone, so the harness itself is exercised.
#[tokio::test(flavor = "multi_thread")]
async fn spec_selftest() {
    harness::run("spec-selftest", launch).await;
}
