// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The area's start-ups (spec §8, §9): the QoS pong listener, then the party outbox dispatcher.

use std::sync::Arc;
use std::time::Duration;

use axum::response::IntoResponse;
use http::{StatusCode, header};
use module_api::BoxFuture;
use rustic_isles::{Shared, Started, StartedListener, Startup};
use rustic_isles_store::IslesStore;

use crate::outbox::{ContextSink, run_outbox};
use crate::rules::qos_plan;
use crate::state::MatchmakingState;

/// The pong every region ping expects.
pub const QOS_PONG: &str = "<!DOCTYPE html><html><body>pong</body></html>";
/// 1.14.7's `qos` port when the config assigns none.
pub const DEFAULT_QOS_BIND_PORT: u16 = 61_010;
const QOS_DRAIN: Duration = Duration::from_secs(1);

/// The client pings each `regionUrls` entry over plain HTTP on TCP and expects the pong on every path; with nothing
/// answering every region reads `9999ms Invalid`. A game module opens no public listener, so this one is loopback
/// and the gateway decides whether to publish it.
pub struct QosListener {
    /// The loopback port the listener binds.
    pub bind_port: u16,
}

impl Default for QosListener {
    fn default() -> Self {
        Self {
            bind_port: DEFAULT_QOS_BIND_PORT,
        }
    }
}

fn qos_app() -> axum::Router {
    axum::Router::new()
        .fallback(|| async { (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], QOS_PONG).into_response() })
}

impl Startup for QosListener {
    fn start<'a>(&'a self, shared: &'a Arc<Shared>) -> BoxFuture<'a, Result<Started, String>> {
        Box::pin(async move {
            if let Some(state) = shared.slot::<MatchmakingState>() {
                state.bind_shared(shared);
            }
            let default = shared.version().tables.switch_default("qos_port");
            let plan = qos_plan(shared.settings().raw("QOS_PORT"), default);
            let Some(public) = plan.port else {
                match plan.problem {
                    Some(problem) => tracing::warn!("QoS listener not started: {problem}"),
                    None => tracing::info!(
                        "QoS latency-test listener off (QOS_PORT=off); the relay or a proxy answers the region pings"
                    ),
                }
                return Ok(Started::default());
            };
            // The game is playable without a latency measurement
            let listener = match tokio::net::TcpListener::bind(("127.0.0.1", self.bind_port)).await {
                Ok(listener) => listener,
                Err(e) => {
                    tracing::warn!("QoS listener could not start: {e}");
                    return Ok(Started::default());
                }
            };
            let address = match listener.local_addr() {
                Ok(address) => address,
                Err(e) => {
                    tracing::warn!("QoS listener could not start: {e}");
                    return Ok(Started::default());
                }
            };
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let task = tokio::spawn(async move {
                let signal = async move {
                    let _ = stopped.await;
                };
                if let Err(e) = service_kit::serve::serve(listener, qos_app(), signal, QOS_DRAIN).await {
                    tracing::warn!("QoS listener failed: {e}");
                }
            });
            tracing::info!(
                "QoS latency-test listener on {address} for public port {public} (answers pong for region pings)"
            );
            Ok(Started {
                listeners: vec![StartedListener {
                    addresses: vec![
                        ("qos".to_owned(), address.to_string()),
                        ("qos_public_port".to_owned(), public.to_string()),
                    ],
                    stop: Box::new(move || {
                        let _ = stop.send(());
                    }),
                    task,
                }],
                ..Started::default()
            })
        })
    }
}

/// Starts only with a store.
pub struct OutboxDispatcher;

impl Startup for OutboxDispatcher {
    fn start<'a>(&'a self, shared: &'a Arc<Shared>) -> BoxFuture<'a, Result<Started, String>> {
        Box::pin(async move {
            let Some(store) = shared.slot::<IslesStore>() else {
                return Ok(Started::default());
            };
            let sink = Arc::new(ContextSink(shared.context().cloned()));
            let state = shared.slot::<MatchmakingState>();
            let clock = move || state.as_ref().map_or_else(rustic_isles::replay::now_ms, |s| s.now_ms());
            let task = tokio::spawn(run_outbox(IslesStore::clone(&store), sink, clock));
            Ok(Started {
                tasks: vec![task],
                ..Started::default()
            })
        })
    }
}

/// QoS (on 1.14.7's default `qos` port), then the outbox.
pub fn startups() -> Vec<Arc<dyn Startup>> {
    startups_on(DEFAULT_QOS_BIND_PORT)
}

/// The same with the `qos` port the config assigned (0 in tests).
pub fn startups_on(qos_port: u16) -> Vec<Arc<dyn Startup>> {
    vec![
        Arc::new(QosListener { bind_port: qos_port }),
        Arc::new(OutboxDispatcher),
    ]
}
