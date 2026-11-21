// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Work counted per module: a named task, or a router's requests, polls under a `tokio-metrics` monitor of that
//! name, and `/metrics` reports each monitor and the runtime (`dainty_task_*`, `dainty_runtime_*`). Under the
//! `tokio-console` feature the task carries the name in the console too (docs/tokio-console.md).

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Mutex, OnceLock, PoisonError};

use axum::Router;
use tokio::task::JoinHandle;
use tokio_metrics::TaskMonitor;
use tower::util::MapFutureLayer;

use crate::metrics as m;

#[cfg(all(feature = "tokio-console", not(tokio_unstable)))]
compile_error!("the tokio-console feature needs RUSTFLAGS=\"--cfg tokio_unstable\" (docs/tokio-console.md)");

static MONITORS: OnceLock<Mutex<BTreeMap<String, TaskMonitor>>> = OnceLock::new();

fn monitors() -> &'static Mutex<BTreeMap<String, TaskMonitor>> {
    MONITORS.get_or_init(|| {
        m::collector("tasks", || {
            report();
            runtime();
        });
        Mutex::default()
    })
}

/// The monitor for `name`, made on first use and kept for the process, so its totals only go up.
pub fn monitor(name: &str) -> TaskMonitor {
    monitors()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(name.to_owned())
        .or_default()
        .clone()
}

/// Spawns `future` on the current runtime as `tokio::spawn` does, counted under `name` and named so where tokio
/// can name tasks.
pub fn spawn_named<F>(name: &str, future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let future = monitor(name).instrument(future);
    #[cfg(tokio_unstable)]
    {
        // Never Err today; an error would leave nothing to fall back to, the future being moved
        match tokio::task::Builder::new().name(name).spawn(future) {
            Ok(handle) => handle,
            Err(_) => unreachable!(),
        }
    }
    #[cfg(not(tokio_unstable))]
    {
        tokio::spawn(future)
    }
}

/// `app` with every request's handling counted under `name`. axum spawns the connection tasks itself, so this
/// is how a module's requests reach its monitor.
pub fn monitored(name: &str, app: Router) -> Router {
    let monitor = monitor(name);
    app.layer(MapFutureLayer::new(move |future| monitor.instrument(future)))
}

fn report() {
    let monitors: Vec<(String, TaskMonitor)> = monitors()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(name, monitor)| (name.clone(), monitor.clone()))
        .collect();
    for (name, monitor) in monitors {
        let t = monitor.cumulative();
        let labels = [("task", name.as_str())];
        m::total(
            "dainty_task_started_total",
            "Futures a module ran: its named tasks and each request it handled",
            &labels,
            t.instrumented_count as f64,
        );
        m::gauge(
            "dainty_task_alive",
            "A module's futures not yet finished",
            &labels,
            t.instrumented_count.saturating_sub(t.dropped_count) as f64,
        );
        m::total(
            "dainty_task_polls_total",
            "Times a module's futures were polled",
            &labels,
            t.total_poll_count as f64,
        );
        m::total(
            "dainty_task_poll_seconds_total",
            "Time spent polling a module's futures: the CPU it took on the runtime",
            &labels,
            t.total_poll_duration.as_secs_f64(),
        );
        m::total(
            "dainty_task_slow_polls_total",
            "Polls of a module's futures that took over 50 microseconds, blocking a runtime worker",
            &labels,
            t.total_slow_poll_count as f64,
        );
        m::total(
            "dainty_task_scheduled_seconds_total",
            "Time a module's woken tasks waited for a runtime worker",
            &labels,
            t.total_scheduled_duration.as_secs_f64(),
        );
    }
}

fn runtime() {
    // A scrape is served on the runtime; a render from elsewhere (a test) has none to read
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let rt = handle.metrics();
    let workers = rt.num_workers();
    m::gauge(
        "dainty_runtime_workers",
        "Worker threads of the server's async runtime",
        &[],
        workers as f64,
    );
    m::gauge(
        "dainty_runtime_alive_tasks",
        "Tasks alive on the runtime, every module's",
        &[],
        rt.num_alive_tasks() as f64,
    );
    m::gauge(
        "dainty_runtime_global_queue_depth",
        "Tasks waiting in the runtime's shared queue: a steady rise means the workers can't keep up",
        &[],
        rt.global_queue_depth() as f64,
    );
    let busy: f64 = (0..workers)
        .map(|w| rt.worker_total_busy_duration(w).as_secs_f64())
        .sum();
    m::total(
        "dainty_runtime_busy_seconds_total",
        "Time the runtime's workers spent running tasks, summed over workers",
        &[],
        busy,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_named_task_and_its_requests_reach_the_metrics() {
        spawn_named("task-metrics-test", async { tokio::task::yield_now().await })
            .await
            .unwrap();
        let app = monitored(
            "task-metrics-test",
            Router::new().route("/", axum::routing::get(|| async { "ok" })),
        );
        let response = tower::ServiceExt::oneshot(app, axum::http::Request::new(axum::body::Body::empty()))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let text = m::render();
        assert!(
            text.contains("dainty_task_started_total{task=\"task-metrics-test\"} 2\n"),
            "{text}"
        );
        assert!(
            text.contains("dainty_task_alive{task=\"task-metrics-test\"} 0\n"),
            "{text}"
        );
        assert!(text.contains("# TYPE dainty_task_polls_total counter\n"), "{text}");
        assert!(text.contains("dainty_runtime_workers "), "{text}");
    }
}
