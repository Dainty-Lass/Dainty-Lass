//! The content server built as a library, run by the core as `host` would (`controller::service_library`), answers
//! every recorded script as the Node one did (RD27; the harness says how).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

#[path = "../../content/tests/parity/harness.rs"]
mod harness;

use std::path::PathBuf;
use std::time::Duration;

use controller::{ServiceLibrary, service_library};
use harness::Server;
use module_api::CoreCapability;
use module_api::host::file_sha256;
use service_kit::context::{ComponentPhase, StatusRegistry};
use service_kit::env::Env;

/// The library cargo built beside this test (target/<profile>/), before running it.
fn library() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().and_then(|deps| deps.parent()).unwrap();
    let name = format!(
        "{}content_plugin{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    profile.join(name)
}

async fn through_library(mut pairs: Vec<(String, String)>) -> Server {
    let path = library();
    // A cdylib is not built for its own tests, and a nested cargo would wait on this one's lock
    assert!(
        path.is_file(),
        "{} is missing: run `cargo build -p content-plugin` before the tests (CI does)",
        path.display()
    );
    // PORT refuses 0: a free one, found first
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    pairs.push(("PORT".into(), port.to_string()));
    let env = Env::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let library = ServiceLibrary {
        name: "content",
        sha256: file_sha256(&path).await.unwrap(),
        path,
        serves: &[CoreCapability::ContentFiles],
    };
    let module = service_library(library, env, None);
    let status = StatusRegistry::default();
    let (ctx, stop) = status.service_context("content");
    let running = tokio::spawn((module.start)(ctx));
    for _ in 0..200 {
        let phase = status
            .snapshot()
            .get("content")
            .map(|c| (c.phase, c.last_error.clone()));
        match phase {
            Some((ComponentPhase::Ready, _)) => break,
            Some((ComponentPhase::Failed, error)) => panic!("content failed: {error:?}"),
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    assert_eq!(
        status.snapshot().get("content").map(|c| c.phase),
        Some(ComponentPhase::Ready),
        "content ready"
    );
    Server {
        port,
        stop: Box::pin(async move {
            let _ = stop.send(true);
            running.await.unwrap().unwrap();
        }),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn content() {
    harness::run("content", through_library).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn content_none() {
    harness::run("content-none", through_library).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn content_invalid() {
    harness::run("content-invalid", through_library).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn content_index() {
    harness::run("content-index", through_library).await;
}
