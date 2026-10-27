//! `observability/metrics.ts`: counters, gauges and latency histograms in Prometheus text, one registry per process
//! (Node's module-level maps). The pipeline feeds the `mysticparadox_http_*` series; areas add their own.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

const BUCKETS: [f64; 7] = [5.0, 25.0, 100.0, 250.0, 500.0, 1000.0, 5000.0];
const ALLOWED_LABELS: [&str; 9] = [
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

type Labels = Vec<(String, String)>;

#[derive(Default)]
struct Histogram {
    /// One per bucket, then +Inf
    buckets: [u64; 8],
    sum: f64,
    count: u64,
}

#[derive(Default)]
struct Registry {
    counters: BTreeMap<String, (String, Labels, f64)>,
    gauges: BTreeMap<String, (String, Labels, f64)>,
    histograms: BTreeMap<String, (String, Labels, Histogram)>,
}

static REGISTRY: Mutex<Option<Registry>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Registry) -> T) -> T {
    let mut guard = REGISTRY.lock().unwrap_or_else(PoisonError::into_inner);
    f(guard.get_or_insert_with(Registry::default))
}

fn safe_name(name: &str) -> String {
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_' || c == ':');
    let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == ':');
    if first_ok && rest_ok && name.len() <= 128 {
        name.to_owned()
    } else {
        "mysticparadox_invalid_metric".to_owned()
    }
}

fn label_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn label_value_ok(value: &str) -> bool {
    (1..=48).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

fn safe_labels(labels: &[(&str, &str)]) -> Labels {
    let mut kept: Labels = labels
        .iter()
        .filter(|(name, value)| ALLOWED_LABELS.contains(name) && label_name_ok(name) && label_value_ok(value))
        .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
        .collect();
    kept.sort_by(|a, b| a.0.cmp(&b.0));
    kept.truncate(MAX_LABELS);
    kept
}

fn encode(name: &str, labels: &Labels) -> String {
    let body: Vec<String> = labels.iter().map(|(n, v)| format!("{n}={v:?}")).collect();
    format!("{name}|{{{}}}", body.join(","))
}

fn escaped(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

fn labels_text(labels: &Labels) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = labels.iter().map(|(n, v)| format!("{n}=\"{}\"", escaped(v))).collect();
    format!("{{{}}}", parts.join(","))
}

fn number(value: f64) -> String {
    service_kit::jsjson::number(if value.is_finite() { value } else { 0.0 })
}

pub fn increment(name: &str, amount: f64, labels: &[(&str, &str)]) {
    if !amount.is_finite() || amount <= 0.0 {
        return;
    }
    let (name, labels) = (safe_name(name), safe_labels(labels));
    let key = encode(&name, &labels);
    with(|r| {
        let entry = r.counters.entry(key).or_insert((name, labels, 0.0));
        entry.2 = (entry.2 + amount).min(9_007_199_254_740_991.0);
    });
}

pub fn set(name: &str, value: f64, labels: &[(&str, &str)]) {
    if !value.is_finite() {
        return;
    }
    let (name, labels) = (safe_name(name), safe_labels(labels));
    let key = encode(&name, &labels);
    with(|r| {
        r.gauges.insert(key, (name, labels, value));
    });
}

pub fn adjust(name: &str, delta: f64, labels: &[(&str, &str)]) {
    if !delta.is_finite() {
        return;
    }
    let (name, labels) = (safe_name(name), safe_labels(labels));
    let key = encode(&name, &labels);
    with(|r| {
        let entry = r.gauges.entry(key).or_insert((name, labels, 0.0));
        entry.2 = (entry.2 + delta).max(0.0);
    });
}

pub fn observe(name: &str, value: f64, labels: &[(&str, &str)]) {
    if !value.is_finite() || value < 0.0 {
        return;
    }
    let (name, labels) = (safe_name(name), safe_labels(labels));
    let key = encode(&name, &labels);
    with(|r| {
        let entry = r.histograms.entry(key).or_insert((name, labels, Histogram::default()));
        let h = &mut entry.2;
        h.count += 1;
        h.sum += value;
        for (index, bound) in BUCKETS.iter().enumerate() {
            if value <= *bound {
                h.buckets[index] += 1;
            }
        }
        h.buckets[7] += 1;
    });
}

/// A derived gauge: name, help, value.
pub type Derived = (String, String, f64);

/// `MetricsSection`: gauges derived at render time, and the raw gauges they render themselves.
/// Reads a raw gauge by name.
pub type ReadGauge<'a> = &'a dyn Fn(&str) -> Option<f64>;

pub struct Section {
    pub render: fn(now_ms: i64, read_gauge: ReadGauge<'_>) -> Vec<Derived>,
    pub hidden_gauges: &'static [&'static str],
}

static SECTIONS: Mutex<Vec<Section>> = Mutex::new(Vec::new());

pub fn register_section(section: Section) {
    SECTIONS.lock().unwrap_or_else(PoisonError::into_inner).push(section);
}

/// What `RenderMetrics` writes; the readiness comes from the caller.
pub fn render(now_ms: i64, role: &str, ready: bool, changed_at_seconds: f64) -> String {
    let mut lines: Vec<String> = Vec::new();
    let line = |name: &str, labels: &Labels, value: f64| format!("{name}{} {}", labels_text(labels), number(value));
    lines.push("# HELP mysticparadox_info Process identity and configured role.".into());
    lines.push("# TYPE mysticparadox_info gauge".into());
    let info: Labels = safe_labels(&[("service", "metagame"), ("role", role)]);
    lines.push(line("mysticparadox_info", &info, 1.0));
    lines.push("# HELP mysticparadox_service_ready Whether the service has completed its role startup.".into());
    lines.push("# TYPE mysticparadox_service_ready gauge".into());
    lines.push(line(
        "mysticparadox_service_ready",
        &Vec::new(),
        f64::from(u8::from(ready)),
    ));
    lines.push(
        "# HELP mysticparadox_service_readiness_changed_timestamp_seconds Unix time of the last readiness transition."
            .into(),
    );
    lines.push("# TYPE mysticparadox_service_readiness_changed_timestamp_seconds gauge".into());
    lines.push(line(
        "mysticparadox_service_readiness_changed_timestamp_seconds",
        &Vec::new(),
        changed_at_seconds,
    ));

    let sections = SECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    with(|r| {
        let read = |name: &str| r.gauges.get(&encode(&safe_name(name), &Vec::new())).map(|g| g.2);
        for section in sections.iter() {
            for (name, help, value) in (section.render)(now_ms, &read) {
                lines.push(format!("# HELP {name} {help}"));
                lines.push(format!("# TYPE {name} gauge"));
                lines.push(line(&name, &Vec::new(), value));
            }
        }
        let mut names: Vec<&str> = r.counters.values().map(|c| c.0.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        for name in names {
            lines.push(format!("# HELP {name} Mystic Paradox counter."));
            lines.push(format!("# TYPE {name} counter"));
        }
        for (name, labels, value) in r.counters.values() {
            lines.push(line(name, labels, *value));
        }
        for (name, labels, value) in r.gauges.values() {
            if sections.iter().any(|s| s.hidden_gauges.contains(&name.as_str())) {
                continue;
            }
            lines.push(format!("# HELP {name} Mystic Paradox gauge."));
            lines.push(format!("# TYPE {name} gauge"));
            lines.push(line(name, labels, *value));
        }
        for (name, labels, h) in r.histograms.values() {
            lines.push(format!("# HELP {name} Mystic Paradox request latency histogram."));
            lines.push(format!("# TYPE {name} histogram"));
            for index in 0..8 {
                let bound = BUCKETS.get(index).map_or_else(|| "+Inf".to_owned(), |b| number(*b));
                let mut with_le = labels.clone();
                with_le.push(("le".to_owned(), bound));
                lines.push(format!("{name}_bucket{} {}", labels_text(&with_le), h.buckets[index]));
            }
            lines.push(line(&format!("{name}_sum"), labels, h.sum));
            lines.push(line(&format!("{name}_count"), labels, h.count as f64));
        }
    });
    format!("{}\n", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_filtered_like_node() {
        let labels = safe_labels(&[
            ("zeta", "x"),
            ("method", "GET"),
            ("class", "bad value"),
            ("status", "2xx"),
        ]);
        assert_eq!(
            labels,
            vec![("method".into(), "GET".into()), ("status".into(), "2xx".into())]
        );
        assert_eq!(labels_text(&labels), r#"{method="GET",status="2xx"}"#);
        assert_eq!(safe_name("Bad Name"), "mysticparadox_invalid_metric");
    }

    #[test]
    fn it_renders_what_the_pipeline_counted() {
        // Names no other test uses: the registry is process-wide
        increment("mp_test_requests_total", 1.0, &[("class", "game"), ("method", "GET")]);
        increment("mp_test_requests_total", 2.0, &[("class", "game"), ("method", "GET")]);
        adjust("mp_test_inflight", -3.0, &[]);
        observe("mp_test_duration_ms", 30.0, &[("class", "game")]);
        let text = render(0, "combined", true, 12.5);
        assert!(text.starts_with("# HELP mysticparadox_info Process identity and configured role.\n"));
        assert!(text.contains("mysticparadox_info{role=\"combined\",service=\"metagame\"} 1\n"));
        assert!(text.contains("mysticparadox_service_readiness_changed_timestamp_seconds 12.5\n"));
        assert!(text.contains("mp_test_requests_total{class=\"game\",method=\"GET\"} 3\n"));
        assert!(text.contains("# TYPE mp_test_requests_total counter\n"));
        assert!(text.contains("mp_test_inflight 0\n"), "gauges stop at 0");
        assert!(text.contains("mp_test_duration_ms_bucket{class=\"game\",le=\"25\"} 0\n"));
        assert!(text.contains("mp_test_duration_ms_bucket{class=\"game\",le=\"100\"} 1\n"));
        assert!(text.contains("mp_test_duration_ms_bucket{class=\"game\",le=\"+Inf\"} 1\n"));
        assert!(text.contains("mp_test_duration_ms_sum{class=\"game\"} 30\n"));
        assert!(text.ends_with('\n') && !text.ends_with("\n\n"));
    }
}
