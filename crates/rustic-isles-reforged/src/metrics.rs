// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The module's metrics registry and its Prometheus text 0.0.4 exposition (public spec §4.4).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

const INVALID_NAME: &str = "rustic_isles_invalid_metric";
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
const BUCKETS: [f64; 7] = [5.0, 25.0, 100.0, 250.0, 500.0, 1000.0, 5000.0];
const MAX_TOTAL: f64 = 9_007_199_254_740_991.0;

type Labels = Vec<(String, String)>;

/// A section another area renders itself: `(name, help, value)` gauges; the raw gauges it names are not repeated.
#[derive(Clone)]
pub struct Section {
    pub render: Arc<dyn Fn() -> Vec<(String, String, f64)> + Send + Sync>,
    pub hidden_gauges: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct Histogram {
    buckets: [u64; 7],
    sum: f64,
    count: u64,
}

#[derive(Default)]
struct Registry {
    counters: BTreeMap<String, (String, Labels, f64)>,
    gauges: BTreeMap<String, (String, Labels, f64)>,
    histograms: BTreeMap<String, (String, Labels, Histogram)>,
    sections: Vec<Section>,
}

/// One per module.
#[derive(Default)]
pub struct Metrics(Mutex<Registry>);

impl std::fmt::Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Metrics")
    }
}

/// `[a-z_:][a-z0-9_:]*`, at most 128 bytes.
pub fn is_metric_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    name.len() <= 128
        && (first.is_ascii_lowercase() || first == b'_' || first == b':')
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b':')
}

fn is_label_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    name.len() <= 32
        && (first.is_ascii_lowercase() || first == b'_')
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && ALLOWED_LABELS.contains(&name)
}

fn is_label_value(value: &str) -> bool {
    (1..=48).contains(&value.chars().count())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

/// The kept labels, sorted by name, at most 4.
pub fn clean_labels(labels: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut kept: Vec<(String, String)> = Vec::new();
    for (name, value) in labels {
        if is_label_name(name) && is_label_value(value) && !kept.iter().any(|(n, _)| n == name) {
            kept.push(((*name).to_owned(), (*value).to_owned()));
        }
    }
    kept.sort_by(|a, b| a.0.cmp(&b.0));
    kept.truncate(4);
    kept
}

fn key(name: &str, labels: &Labels) -> String {
    let inner: Vec<String> = labels.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
    format!("{name}|{{{}}}", inner.join(","))
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

fn label_text(labels: &[(String, String)]) -> String {
    labels
        .iter()
        .map(|(k, v)| format!("{k}=\"{}\"", escape(v)))
        .collect::<Vec<_>>()
        .join(",")
}

fn sample(name: &str, labels: &[(String, String)], value: f64) -> String {
    if labels.is_empty() {
        format!("{name} {}", number(value))
    } else {
        format!("{name}{{{}}} {}", label_text(labels), number(value))
    }
}

/// JavaScript number text; a non-finite value writes `0`.
pub fn number(value: f64) -> String {
    if value.is_finite() {
        service_kit::jsjson::number(value)
    } else {
        "0".to_owned()
    }
}

fn metric_name(name: &str) -> &str {
    if is_metric_name(name) { name } else { INVALID_NAME }
}

impl Metrics {
    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn increment(&self, name: &str, amount: f64, labels: &[(&str, &str)]) {
        if !amount.is_finite() || amount <= 0.0 {
            return;
        }
        let name = metric_name(name);
        let labels = clean_labels(labels);
        let k = key(name, &labels);
        let mut registry = self.registry();
        let entry = registry
            .counters
            .entry(k)
            .or_insert_with(|| (name.to_owned(), labels, 0.0));
        entry.2 = (entry.2 + amount).min(MAX_TOTAL);
    }

    pub fn set(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
        if !value.is_finite() {
            return;
        }
        let name = metric_name(name);
        let labels = clean_labels(labels);
        let k = key(name, &labels);
        self.registry().gauges.insert(k, (name.to_owned(), labels, value));
    }

    pub fn adjust(&self, name: &str, delta: f64, labels: &[(&str, &str)]) {
        if !delta.is_finite() {
            return;
        }
        let name = metric_name(name);
        let labels = clean_labels(labels);
        let k = key(name, &labels);
        let mut registry = self.registry();
        let entry = registry
            .gauges
            .entry(k)
            .or_insert_with(|| (name.to_owned(), labels, 0.0));
        entry.2 = (entry.2 + delta).max(0.0);
    }

    pub fn observe(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
        if !value.is_finite() || value < 0.0 {
            return;
        }
        let name = metric_name(name);
        let labels = clean_labels(labels);
        let k = key(name, &labels);
        let mut registry = self.registry();
        let entry = registry
            .histograms
            .entry(k)
            .or_insert_with(|| (name.to_owned(), labels, Histogram::default()));
        for (i, bound) in BUCKETS.iter().enumerate() {
            if value <= *bound {
                entry.2.buckets[i] += 1;
            }
        }
        entry.2.sum += value;
        entry.2.count += 1;
    }

    pub fn register_section(&self, section: Section) {
        self.registry().sections.push(section);
    }

    /// The whole exposition, one `\n` at the end.
    pub fn render(&self, role: &str, ready: bool, changed_at_seconds: f64) -> String {
        let registry = self.registry();
        let mut lines = vec![
            "# HELP rustic_isles_info Process identity and configured role.".to_owned(),
            "# TYPE rustic_isles_info gauge".to_owned(),
            format!("rustic_isles_info{{role=\"{}\",service=\"metagame\"}} 1", escape(role)),
            "# HELP rustic_isles_service_ready Whether the service has completed its role startup.".to_owned(),
            "# TYPE rustic_isles_service_ready gauge".to_owned(),
            format!("rustic_isles_service_ready {}", u8::from(ready)),
            "# HELP rustic_isles_service_readiness_changed_timestamp_seconds Unix time of the last readiness \
             transition."
                .to_owned(),
            "# TYPE rustic_isles_service_readiness_changed_timestamp_seconds gauge".to_owned(),
            format!(
                "rustic_isles_service_readiness_changed_timestamp_seconds {}",
                number(changed_at_seconds)
            ),
        ];
        let mut hidden = BTreeSet::new();
        for section in &registry.sections {
            hidden.extend(section.hidden_gauges.iter().cloned());
            for (name, help, value) in (section.render)() {
                lines.push(format!("# HELP {name} {help}"));
                lines.push(format!("# TYPE {name} gauge"));
                lines.push(format!("{name} {}", number(value)));
            }
        }
        let names: BTreeSet<&String> = registry.counters.values().map(|(n, _, _)| n).collect();
        for name in names {
            lines.push(format!("# HELP {name} Rustic Isles counter."));
            lines.push(format!("# TYPE {name} counter"));
        }
        for (name, labels, value) in registry.counters.values() {
            lines.push(sample(name, labels, *value));
        }
        for (name, labels, value) in registry.gauges.values() {
            if hidden.contains(name) {
                continue;
            }
            lines.push(format!("# HELP {name} Rustic Isles gauge."));
            lines.push(format!("# TYPE {name} gauge"));
            lines.push(sample(name, labels, *value));
        }
        for (name, labels, histogram) in registry.histograms.values() {
            lines.push(format!("# HELP {name} Rustic Isles request latency histogram."));
            lines.push(format!("# TYPE {name} histogram"));
            let base = label_text(labels);
            let with_le = |le: &str| {
                if base.is_empty() {
                    format!("le=\"{le}\"")
                } else {
                    format!("{base},le=\"{le}\"")
                }
            };
            for (i, bound) in BUCKETS.iter().enumerate() {
                lines.push(format!(
                    "{name}_bucket{{{}}} {}",
                    with_le(&number(*bound)),
                    histogram.buckets[i]
                ));
            }
            lines.push(format!("{name}_bucket{{{}}} {}", with_le("+Inf"), histogram.count));
            lines.push(sample(&format!("{name}_sum"), labels, histogram.sum));
            lines.push(format!(
                "{} {}",
                if base.is_empty() {
                    format!("{name}_count")
                } else {
                    format!("{name}_count{{{base}}}")
                },
                histogram.count
            ));
        }
        let mut text = lines.join("\n");
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
