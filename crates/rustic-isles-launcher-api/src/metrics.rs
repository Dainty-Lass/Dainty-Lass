// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The module's metrics registry and its Prometheus text 0.0.4 exposition (public spec §4.4).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

const FALLBACK_NAME: &str = "rustic_isles_invalid_metric";
const LABEL_NAMES: &[&str] = &[
    "class",
    "code",
    "le",
    "method",
    "operation",
    "result",
    "role",
    "service",
    "status",
];
const MAX_LABELS: usize = 4;
const LATENCY_BOUNDS: [f64; 7] = [5.0, 25.0, 100.0, 250.0, 500.0, 1000.0, 5000.0];
/// 2^53 - 1, JavaScript's largest safe integer.
const COUNTER_CEILING: f64 = 9_007_199_254_740_991.0;

/// A section another area renders itself: `(name, help, value)` gauges; the raw gauges it names are not repeated.
#[derive(Clone)]
pub struct Section {
    /// Renders the section's gauges.
    pub render: Arc<dyn Fn() -> Vec<(String, String, f64)> + Send + Sync>,
    /// Raw gauge names the section replaces.
    pub hidden_gauges: Vec<String>,
}

/// One series: a metric name and its kept labels. Ordered by its sort text, the order samples are written in.
#[derive(Clone, PartialEq, Eq)]
struct Series {
    metric: String,
    labels: Vec<(String, String)>,
}

impl Series {
    fn new(name: &str, labels: &[(&str, &str)]) -> Self {
        let metric = if is_metric_name(name) { name } else { FALLBACK_NAME };
        Self {
            metric: metric.to_owned(),
            labels: clean_labels(labels),
        }
    }

    /// `name|{k="v",...}` with Rust's `{:?}` quoting (§4.4's sample order).
    fn sort_text(&self) -> String {
        let inner: Vec<String> = self.labels.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
        format!("{}|{{{}}}", self.metric, inner.join(","))
    }
}

impl Ord for Series {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sort_text().as_bytes().cmp(other.sort_text().as_bytes())
    }
}

impl PartialOrd for Series {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Default)]
struct Latency {
    per_bound: [u64; LATENCY_BOUNDS.len()],
    observations: u64,
    total: f64,
}

#[derive(Default)]
struct Store {
    counters: BTreeMap<Series, f64>,
    gauges: BTreeMap<Series, f64>,
    latencies: BTreeMap<Series, Latency>,
    sections: Vec<Section>,
}

/// One per module.
#[derive(Default)]
pub struct Metrics(Mutex<Store>);

impl std::fmt::Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Metrics { .. }")
    }
}

fn plain_name(text: &str, max: usize, first: fn(u8) -> bool, rest: fn(u8) -> bool) -> bool {
    match text.as_bytes() {
        [] => false,
        [head, tail @ ..] => text.len() <= max && first(*head) && tail.iter().all(|b| rest(*b)),
    }
}

/// `[a-z_:][a-z0-9_:]*`, at most 128 bytes.
pub fn is_metric_name(name: &str) -> bool {
    plain_name(
        name,
        128,
        |b| b.is_ascii_lowercase() || b == b'_' || b == b':',
        |b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b':',
    )
}

fn keeps_label(name: &str, value: &str) -> bool {
    let name_ok = LABEL_NAMES.contains(&name)
        && plain_name(
            name,
            32,
            |b| b.is_ascii_lowercase() || b == b'_',
            |b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_',
        );
    let value_ok = (1..=48).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'));
    name_ok && value_ok
}

/// The kept labels, sorted by name, at most 4.
pub fn clean_labels(labels: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut kept: Vec<(String, String)> = Vec::new();
    for (name, value) in labels {
        if keeps_label(name, value) && !kept.iter().any(|(n, _)| n == name) {
            kept.push(((*name).to_owned(), (*value).to_owned()));
        }
    }
    kept.sort_by(|a, b| a.0.cmp(&b.0));
    kept.truncate(MAX_LABELS);
    kept
}

/// JavaScript number text; a non-finite value writes `0`.
pub fn number(value: f64) -> String {
    if value.is_finite() {
        service_kit::jsjson::number(value)
    } else {
        "0".to_owned()
    }
}

fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

fn line(out: &mut Vec<String>, metric: &str, labels: &[(String, String)], value: f64) {
    if labels.is_empty() {
        out.push(format!("{metric} {}", number(value)));
    } else {
        let pairs: Vec<String> = labels.iter().map(|(k, v)| format!("{k}=\"{}\"", quote(v))).collect();
        out.push(format!("{metric}{{{}}} {}", pairs.join(","), number(value)));
    }
}

fn headed(out: &mut Vec<String>, metric: &str, help: &str, kind: &str) {
    out.push(format!("# HELP {metric} {help}"));
    out.push(format!("# TYPE {metric} {kind}"));
}

impl Metrics {
    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds to a counter; a non-positive or non-finite amount is ignored, and the total stops at 2^53 - 1.
    pub fn increment(&self, name: &str, amount: f64, labels: &[(&str, &str)]) {
        if !(amount.is_finite() && amount > 0.0) {
            return;
        }
        let mut store = self.store();
        let total = store.counters.entry(Series::new(name, labels)).or_insert(0.0);
        *total = (*total + amount).min(COUNTER_CEILING);
    }

    /// Sets a gauge; a non-finite value is ignored.
    pub fn set(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
        if value.is_finite() {
            self.store().gauges.insert(Series::new(name, labels), value);
        }
    }

    /// Moves a gauge, never below 0; a non-finite delta is ignored.
    pub fn adjust(&self, name: &str, delta: f64, labels: &[(&str, &str)]) {
        if !delta.is_finite() {
            return;
        }
        let mut store = self.store();
        let gauge = store.gauges.entry(Series::new(name, labels)).or_insert(0.0);
        *gauge = (*gauge + delta).max(0.0);
    }

    /// Records a latency in the histogram; a negative or non-finite value is ignored.
    pub fn observe(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
        if !(value.is_finite() && value >= 0.0) {
            return;
        }
        let mut store = self.store();
        let latency = store.latencies.entry(Series::new(name, labels)).or_default();
        for (count, bound) in latency.per_bound.iter_mut().zip(LATENCY_BOUNDS) {
            if value <= bound {
                *count += 1;
            }
        }
        latency.observations += 1;
        latency.total += value;
    }

    /// Adds a section rendered with the rest.
    pub fn register_section(&self, section: Section) {
        self.store().sections.push(section);
    }

    /// The Prometheus text, with the identity, readiness and every series.
    pub fn render(&self, role: &str, ready: bool, changed_at_seconds: f64) -> String {
        let store = self.store();
        let mut out = Vec::new();
        headed(
            &mut out,
            "rustic_isles_info",
            "Process identity and configured role.",
            "gauge",
        );
        let identity = [
            ("role".to_owned(), role.to_owned()),
            ("service".to_owned(), "metagame".to_owned()),
        ];
        line(&mut out, "rustic_isles_info", &identity, 1.0);
        headed(
            &mut out,
            "rustic_isles_service_ready",
            "Whether the service has completed its role startup.",
            "gauge",
        );
        line(
            &mut out,
            "rustic_isles_service_ready",
            &[],
            if ready { 1.0 } else { 0.0 },
        );
        let changed = "rustic_isles_service_readiness_changed_timestamp_seconds";
        headed(
            &mut out,
            changed,
            "Unix time of the last readiness transition.",
            "gauge",
        );
        line(&mut out, changed, &[], changed_at_seconds);

        let mut skipped: Vec<&str> = Vec::new();
        for section in &store.sections {
            for (metric, help, value) in (section.render)() {
                headed(&mut out, &metric, &help, "gauge");
                line(&mut out, &metric, &[], value);
            }
            skipped.extend(section.hidden_gauges.iter().map(String::as_str));
        }

        let mut counter_names: Vec<&str> = store.counters.keys().map(|s| s.metric.as_str()).collect();
        counter_names.sort_unstable();
        counter_names.dedup();
        for metric in counter_names {
            headed(&mut out, metric, "Rustic Isles counter.", "counter");
        }
        for (series, total) in &store.counters {
            line(&mut out, &series.metric, &series.labels, *total);
        }

        for (series, value) in store
            .gauges
            .iter()
            .filter(|(s, _)| !skipped.contains(&s.metric.as_str()))
        {
            headed(&mut out, &series.metric, "Rustic Isles gauge.", "gauge");
            line(&mut out, &series.metric, &series.labels, *value);
        }

        for (series, latency) in &store.latencies {
            headed(
                &mut out,
                &series.metric,
                "Rustic Isles request latency histogram.",
                "histogram",
            );
            let bucket = format!("{}_bucket", series.metric);
            let bounded = |le: String| {
                let mut labels = series.labels.clone();
                labels.push(("le".to_owned(), le));
                labels
            };
            for (bound, count) in LATENCY_BOUNDS.iter().zip(latency.per_bound) {
                line(&mut out, &bucket, &bounded(number(*bound)), count as f64);
            }
            line(
                &mut out,
                &bucket,
                &bounded("+Inf".to_owned()),
                latency.observations as f64,
            );
            line(
                &mut out,
                &format!("{}_sum", series.metric),
                &series.labels,
                latency.total,
            );
            line(
                &mut out,
                &format!("{}_count", series.metric),
                &series.labels,
                latency.observations as f64,
            );
        }

        let mut text = out.join("\n");
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_rules() {
        let m = Metrics::default();
        let kept = clean_labels(&[
            ("zeta", "x"),
            ("method", "GET"),
            ("class", "bad value"),
            ("status", "2xx"),
        ]);
        assert_eq!(
            kept,
            vec![("method".into(), "GET".into()), ("status".into(), "2xx".into())]
        );
        m.increment("t_total", 1.0, &[("method", "GET"), ("status", "2xx")]);
        m.increment("t_total", 2.0, &[("status", "2xx"), ("method", "GET")]);
        m.increment("Bad Name", 1.0, &[]);
        m.adjust("g", -3.0, &[]);
        m.observe("h_ms", 30.0, &[("class", "game")]);
        let text = m.render("combined", true, 12.5);
        assert!(text.starts_with("# HELP rustic_isles_info Process identity and configured role.\n"));
        assert!(text.contains("rustic_isles_info{role=\"combined\",service=\"metagame\"} 1\n"));
        assert!(text.contains("rustic_isles_service_ready 1\n"));
        assert!(text.contains("rustic_isles_service_readiness_changed_timestamp_seconds 12.5\n"));
        assert!(text.contains("t_total{method=\"GET\",status=\"2xx\"} 3\n"));
        assert!(text.contains("# TYPE t_total counter\n"));
        assert!(text.contains("rustic_isles_invalid_metric 1\n"));
        assert!(text.contains("\ng 0\n"));
        assert!(text.contains("h_ms_bucket{class=\"game\",le=\"25\"} 0\n"));
        assert!(text.contains("h_ms_bucket{class=\"game\",le=\"100\"} 1\n"));
        assert!(text.contains("h_ms_bucket{class=\"game\",le=\"+Inf\"} 1\n"));
        assert!(text.contains("h_ms_sum{class=\"game\"} 30\n"));
        assert!(text.contains("h_ms_count{class=\"game\"} 1\n"));
        assert!(text.ends_with("1\n") && !text.ends_with("\n\n"));
        // Every counter's HELP/TYPE comes before any counter sample
        let help = text.find("# HELP t_total").unwrap();
        let first_sample = text.find("rustic_isles_invalid_metric 1").unwrap();
        assert!(help < first_sample);
    }

    #[test]
    fn ignored_values() {
        let m = Metrics::default();
        m.increment("a", 0.0, &[]);
        m.increment("a", f64::NAN, &[]);
        m.set("b", f64::INFINITY, &[]);
        m.observe("c", -1.0, &[]);
        let text = m.render("api", false, 0.0);
        assert!(!text.contains("\na ") && !text.contains("\nb ") && !text.contains("c_bucket"));
        assert!(text.contains("rustic_isles_service_ready 0\n"));
    }
}
