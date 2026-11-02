#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

//! The gateway's parity with Reforged's Node one, run in this process (the harness says how).

mod common;
#[path = "parity/harness.rs"]
mod harness;

use std::sync::Arc;

use common::{TestCert, log_lines, start_gateway};
use gateway::feed::AllowlistFeed;
use harness::Server;

async fn in_process(pairs: Vec<(String, String)>, cert: Arc<TestCert>) -> Server {
    let env = service_kit::env::Env::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let config = gateway::config::Config::from_env(&env).unwrap();
    let feed = AllowlistFeed::new(config.allowlist.as_ref().unwrap());
    let running = start_gateway(&cert, config, Some(feed.clone())).await;
    service_kit::linelog::set_level(service_kit::linelog::Level::Info);
    Server {
        port: running.port,
        log: Arc::new(|| {
            log_lines()
                .lock()
                .unwrap()
                .iter()
                .map(|(_, line)| line.clone())
                .collect()
        }),
        idle: Box::pin(async move { feed.idle().await }),
        stop: Box::pin(async move {
            running.close().await;
            drop(cert);
        }),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gateway() {
    harness::run("gateway", in_process).await;
}
