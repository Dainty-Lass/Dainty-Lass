#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

//! The content server's parity with the Node one, run in this process (the harness says how).

#[path = "parity/harness.rs"]
mod harness;

use harness::Server;
use service_kit::env::Env;

async fn in_process(pairs: Vec<(String, String)>) -> Server {
    let env = Env::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let config = content::config::Config::from_env(&env).unwrap();
    content::log::set_sink(Some(Box::new(|_, _| {})));
    let app = content::server::prepare(&config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopping) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(content::server::serve(listener, app, stopping));
    Server {
        port,
        stop: Box::pin(async move {
            let _ = stop.send(true);
            let _ = server.await;
        }),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn content() {
    harness::run("content", in_process).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn content_none() {
    harness::run("content-none", in_process).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn content_invalid() {
    harness::run("content-invalid", in_process).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn content_index() {
    harness::run("content-index", in_process).await;
}
