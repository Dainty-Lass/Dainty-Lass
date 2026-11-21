// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What the library's tests share: the library cargo built, run by the core as `host` would, and its log.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "shared by several test binaries, each using only some; a panic is how a test fails"
)]

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use dainty_controller::ServiceLibrary;
use dainty_module_api::CoreCapability;
use dainty_module_api::host::file_sha256;
use dainty_service_kit::context::{ComponentPhase, StatusRegistry};
use serde_json::Value;

/// The library cargo built beside this test (target/<profile>/), pinned as settings.toml would.
pub async fn library() -> ServiceLibrary {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().and_then(|deps| deps.parent()).unwrap();
    let path = profile.join(format!(
        "{}dainty_gateway_plugin{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    // A cdylib is not built for its own tests, and a nested cargo would wait on this one's lock
    assert!(
        path.is_file(),
        "{} is missing: run `cargo build -p dainty-gateway-plugin` before the tests (CI does)",
        path.display()
    );
    ServiceLibrary {
        name: "gateway",
        sha256: file_sha256(&path).await.unwrap(),
        path,
        serves: &[CoreCapability::Routes, CoreCapability::UdpPorts],
    }
}

/// The library's linelog lines, as the core logs them (`service` events whose source is `linelog`).
pub fn library_log() -> Arc<Mutex<Vec<Value>>> {
    static LINES: OnceLock<Arc<Mutex<Vec<Value>>>> = OnceLock::new();
    LINES
        .get_or_init(|| {
            use tracing_subscriber::layer::SubscriberExt as _;
            let lines: Arc<Mutex<Vec<Value>>> = Arc::default();
            let subscriber = tracing_subscriber::registry().with(Lines(lines.clone()));
            tracing::subscriber::set_global_default(subscriber).unwrap();
            lines
        })
        .clone()
}

struct Lines(Arc<Mutex<Vec<Value>>>);

#[derive(Default)]
struct Fields {
    source: String,
    message: String,
}

impl tracing::field::Visit for Fields {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "source" {
            value.clone_into(&mut self.source);
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = format!("{value:?}"),
            "source" => self.source = format!("{value:?}"),
            _ => {}
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Lines {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().target() != "service" {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        if fields.source == "linelog"
            && let Ok(line) = serde_json::from_str::<Value>(&fields.message)
        {
            self.0.lock().unwrap().push(line);
        }
    }
}

/// Waits for `name` in `status` to be ready; panics on a failure.
pub async fn ready(status: &StatusRegistry, name: &str) {
    for _ in 0..300 {
        let phase = status.snapshot().get(name).map(|c| (c.phase, c.last_error.clone()));
        match phase {
            Some((ComponentPhase::Ready, _)) => return,
            Some((ComponentPhase::Failed, error)) => panic!("{name} failed: {error:?}"),
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    panic!("{name} was not ready within 15 s");
}

pub async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..100 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("never: {what}");
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
