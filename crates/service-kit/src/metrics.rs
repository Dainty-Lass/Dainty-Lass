// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Process-wide metrics in Prometheus' text format (0.0.4). Every service runs in one process (RD26), so one
//! registry covers them all; the admin service serves it at `/metrics`. Collectors registered with
//! [`collector`] refresh their gauges just before each render, so a scrape reads current values and nothing
//! polls in between.
//!
//! Label values are free, so callers keep them few: a service or route class, a status class, never a player
//! id or a path.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;

/// Milliseconds, as the backend's own histograms bucket them (`observability/metrics.ts`).
pub const BUCKETS_MS: [f64; 10] = [5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1_000.0, 2_500.0, 5_000.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Counter,
    Gauge,
    Histogram,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

type Labels = Vec<(&'static str, String)>;

#[derive(Debug, Clone)]
enum Value {
    Number(f64),
    Histogram {
        buckets: [u64; BUCKETS_MS.len()],
        sum: f64,
        count: u64,
    },
}

struct Family {
    kind: Kind,
    help: &'static str,
    series: BTreeMap<Labels, Value>,
}

type Collector = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Registry {
    families: BTreeMap<&'static str, Family>,
    collectors: BTreeMap<String, Collector>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

fn with<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    f(&mut registry().lock().unwrap_or_else(PoisonError::into_inner))
}

fn labels(given: &[(&'static str, &str)]) -> Labels {
    let mut out: Labels = given.iter().map(|(k, v)| (*k, (*v).to_owned())).collect();
    out.sort();
    out
}

fn family<'a>(registry: &'a mut Registry, name: &'static str, kind: Kind, help: &'static str) -> &'a mut Family {
    registry.families.entry(name).or_insert_with(|| Family {
        kind,
        help,
        series: BTreeMap::new(),
    })
}

/// Adds to a counter.
pub fn count(name: &'static str, help: &'static str, given: &[(&'static str, &str)], by: u64) {
    with(|r| {
        let series = &mut family(r, name, Kind::Counter, help).series;
        let value = series.entry(labels(given)).or_insert(Value::Number(0.0));
        if let Value::Number(n) = value {
            *n += by as f64;
        }
    });
}

/// Sets a gauge.
pub fn gauge(name: &'static str, help: &'static str, given: &[(&'static str, &str)], value: f64) {
    with(|r| {
        family(r, name, Kind::Gauge, help)
            .series
            .insert(labels(given), Value::Number(value));
    });
}

/// Drops every series of a gauge, for a collector that sets the whole family again (processes come and go).
pub fn clear(name: &'static str) {
    with(|r| {
        if let Some(family) = r.families.get_mut(name) {
            family.series.clear();
        }
    });
}

/// Records one observation in milliseconds.
pub fn observe_ms(name: &'static str, help: &'static str, given: &[(&'static str, &str)], ms: f64) {
    with(|r| {
        let series = &mut family(r, name, Kind::Histogram, help).series;
        let value = series.entry(labels(given)).or_insert(Value::Histogram {
            buckets: [0; BUCKETS_MS.len()],
            sum: 0.0,
            count: 0,
        });
        if let Value::Histogram { buckets, sum, count } = value {
            for (bucket, bound) in buckets.iter_mut().zip(BUCKETS_MS) {
                if ms <= bound {
                    *bucket += 1;
                }
            }
            *sum += ms;
            *count += 1;
        }
    });
}

/// Runs `refresh` before every render, under `key` (a second registration under the key replaces the first).
pub fn collector(key: impl Into<String>, refresh: impl Fn() + Send + Sync + 'static) {
    with(|r| r.collectors.insert(key.into(), Arc::new(refresh)));
}

pub fn remove_collector(key: &str) {
    with(|r| r.collectors.remove(key));
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

fn label_text(labels: &Labels, extra: Option<(&str, String)>) -> String {
    let mut parts: Vec<String> = labels.iter().map(|(k, v)| format!("{k}=\"{}\"", escape(v))).collect();
    if let Some((k, v)) = extra {
        parts.push(format!("{k}=\"{v}\""));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", parts.join(","))
    }
}

fn number(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Everything, after the collectors ran, in Prometheus' text format.
pub fn render() -> String {
    // Collectors run outside the lock: they set gauges themselves
    let collectors: Vec<Collector> = with(|r| r.collectors.values().cloned().collect());
    for refresh in collectors {
        refresh();
    }
    with(|r| {
        let mut out = String::new();
        for (name, family) in &r.families {
            if family.series.is_empty() {
                continue;
            }
            let _ = writeln!(out, "# HELP {name} {}", family.help);
            let _ = writeln!(out, "# TYPE {name} {}", family.kind.as_str());
            for (labels, value) in &family.series {
                match value {
                    Value::Number(n) => {
                        let _ = writeln!(out, "{name}{} {}", label_text(labels, None), number(*n));
                    }
                    Value::Histogram { buckets, sum, count } => {
                        for (bucket, bound) in buckets.iter().zip(BUCKETS_MS) {
                            let le = Some(("le", number(bound)));
                            let _ = writeln!(out, "{name}_bucket{} {bucket}", label_text(labels, le));
                        }
                        let inf = Some(("le", "+Inf".to_owned()));
                        let _ = writeln!(out, "{name}_bucket{} {count}", label_text(labels, inf));
                        let _ = writeln!(out, "{name}_sum{} {}", label_text(labels, None), number(*sum));
                        let _ = writeln!(out, "{name}_count{} {count}", label_text(labels, None));
                    }
                }
            }
        }
        out
    })
}

/// `2xx`, `4xx`...: a status as a label without one series per code.
pub fn status_class(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    }
}

/// One HTTP request a service answered: `dainty_http_requests_total` and `dainty_http_request_duration_ms`
/// (time to the response's head).
pub fn http_request(service: &str, route: &str, method: &str, status: u16, started: Instant) {
    let method = match method {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" => method,
        _ => "other",
    };
    let given = [
        ("service", service),
        ("route", route),
        ("method", method),
        ("status", status_class(status)),
    ];
    count(
        "dainty_http_requests_total",
        "HTTP requests answered, by service, route class, method and status class",
        &given,
        1,
    );
    observe_ms(
        "dainty_http_request_duration_ms",
        "Time to the response head in milliseconds, by service and route class",
        &given[..2],
        started.elapsed().as_secs_f64() * 1000.0,
    );
}

/// Records every request a router answers as `service` (route class `all`):
/// `.layer(axum::middleware::from_fn_with_state("admin", metrics::track))`.
pub async fn track(
    axum::extract::State(service): axum::extract::State<&'static str>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let started = Instant::now();
    let method = request.method().as_str().to_owned();
    let response = next.run(request).await;
    http_request(service, "all", &method, response.status().as_u16(), started);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_counters_gauges_and_histograms() {
        count("test_render_total", "Things", &[("b", "2"), ("a", "1")], 2);
        count("test_render_total", "Things", &[("a", "1"), ("b", "2")], 1);
        gauge("test_render_gauge", "A level", &[("name", "x\"y")], 1.5);
        observe_ms("test_render_ms", "Time", &[], 7.0);
        observe_ms("test_render_ms", "Time", &[], 700.0);
        let text = render();
        assert!(
            text.contains("# TYPE test_render_total counter\ntest_render_total{a=\"1\",b=\"2\"} 3\n"),
            "{text}"
        );
        assert!(text.contains("test_render_gauge{name=\"x\\\"y\"} 1.5\n"), "{text}");
        assert!(text.contains("test_render_ms_bucket{le=\"5\"} 0\n"), "{text}");
        assert!(text.contains("test_render_ms_bucket{le=\"10\"} 1\n"), "{text}");
        assert!(text.contains("test_render_ms_bucket{le=\"1000\"} 2\n"), "{text}");
        assert!(text.contains("test_render_ms_bucket{le=\"+Inf\"} 2\n"), "{text}");
        assert!(
            text.contains("test_render_ms_sum 707\ntest_render_ms_count 2\n"),
            "{text}"
        );
    }

    #[test]
    fn collectors_run_before_a_render_and_clear_drops_gone_series() {
        collector("test-collector", || {
            gauge("test_collected", "Set late", &[("pid", "1")], 4.0)
        });
        assert!(render().contains("test_collected{pid=\"1\"} 4\n"));
        remove_collector("test-collector");
        clear("test_collected");
        assert!(!render().contains("test_collected"));
    }

    #[tokio::test]
    async fn the_layer_counts_a_routers_requests() {
        use tower::ServiceExt;
        let app = axum::Router::new()
            .route("/", axum::routing::get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state("test-layer", track));
        let request = axum::http::Request::get("/").body(axum::body::Body::empty()).unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), 200);
        let text = render();
        assert!(
            text.contains(
                "dainty_http_requests_total{method=\"GET\",route=\"all\",service=\"test-layer\",status=\"2xx\"} 1\n"
            ),
            "{text}"
        );
    }
}
