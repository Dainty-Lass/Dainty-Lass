// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The host route census: which routes hosted runtimes call and how they are answered, summed per window.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Mutex, PoisonError};

use rifts_seam::HostRouteSample;

use crate::json::{js_length, js_number, js_string};

pub const HOST_ROUTE_CENSUS_WINDOW_MS: u64 = 60_000;
pub const HOST_ROUTE_CENSUS_MAX_ROWS: usize = 200;

const NO_ROUTE: &str = "(no route)";
const METHODS: [&str; 7] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostRouteKey {
    pub method: String,
    pub route: String,
    pub status: String,
    pub session_kind: String,
    pub hub_kind: Option<String>,
}

impl HostRouteKey {
    fn sort_text(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.method,
            self.route,
            self.status,
            self.session_kind,
            self.hub_kind.as_deref().unwrap_or("")
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRouteRow {
    pub key: HostRouteKey,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostRouteSummary {
    pub window_seconds: f64,
    pub total: u64,
    pub routes: Vec<HostRouteRow>,
    pub omitted_rows: u64,
}

impl HostRouteSummary {
    pub fn to_json(&self) -> String {
        let mut out = format!(
            r#"{{"windowSeconds":{},"total":{},"routes":["#,
            js_number(self.window_seconds),
            self.total
        );
        for (i, row) in self.routes.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let key = &row.key;
            let _ = write!(
                out,
                r#"{{"method":{},"route":{},"status":{},"sessionKind":{},"hubKind":{},"count":{}}}"#,
                js_string(&key.method),
                js_string(&key.route),
                js_string(&key.status),
                js_string(&key.session_kind),
                key.hub_kind.as_deref().map_or_else(|| "null".to_owned(), js_string),
                row.count
            );
        }
        let _ = write!(out, r#"],"omittedRows":{}}}"#, self.omitted_rows);
        out
    }
}

pub fn status_class(code: u16) -> &'static str {
    match code {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "other",
    }
}

fn kind_label(value: Option<&str>) -> Option<String> {
    value
        .filter(|v| (1..=32).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'))
        .map(str::to_owned)
}

pub fn normalize(sample: &HostRouteSample) -> HostRouteKey {
    let method = if METHODS.contains(&sample.method.as_str()) {
        sample.method.clone()
    } else {
        "OTHER".to_owned()
    };
    let route = if (1..=160).contains(&js_length(&sample.route)) {
        sample.route.clone()
    } else {
        NO_ROUTE.to_owned()
    };
    HostRouteKey {
        method,
        route,
        status: status_class(sample.status_code).to_owned(),
        session_kind: kind_label(sample.session_kind.as_deref()).unwrap_or_else(|| "unknown".to_owned()),
        hub_kind: kind_label(sample.hub_kind.as_deref()),
    }
}

/// The route's label: the mount's base and the matched pattern.
pub fn route_of(pattern: Option<&str>, base_url: &str) -> String {
    pattern.map_or_else(|| NO_ROUTE.to_owned(), |p| format!("{base_url}{p}"))
}

pub fn aggregate(samples: &[HostRouteSample]) -> Vec<HostRouteRow> {
    let mut counts: BTreeMap<HostRouteKey, u64> = BTreeMap::new();
    for sample in samples {
        *counts.entry(normalize(sample)).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|(key, count)| HostRouteRow { key, count })
        .collect()
}

/// Busiest first, ties by the key's text; at most `max_rows` rows, the rest counted as omitted.
pub fn summarize(rows: &[HostRouteRow], window_seconds: f64, max_rows: usize) -> Option<HostRouteSummary> {
    let mut rows: Vec<HostRouteRow> = rows.iter().filter(|row| row.count > 0).cloned().collect();
    if rows.is_empty() {
        return None;
    }
    rows.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.key.sort_text().cmp(&b.key.sort_text()))
    });
    let total = rows.iter().map(|row| row.count).sum();
    let omitted = rows.len().saturating_sub(max_rows) as u64;
    rows.truncate(max_rows);
    Some(HostRouteSummary {
        window_seconds,
        total,
        routes: rows,
        omitted_rows: omitted,
    })
}

#[derive(Debug)]
pub struct HostRouteCensus {
    enabled: bool,
    window_ms: u64,
    max_rows: usize,
    counts: Mutex<BTreeMap<HostRouteKey, u64>>,
}

impl HostRouteCensus {
    pub fn new(enabled: bool) -> Self {
        Self::with_window(enabled, HOST_ROUTE_CENSUS_WINDOW_MS, HOST_ROUTE_CENSUS_MAX_ROWS)
    }

    pub fn with_window(enabled: bool, window_ms: u64, max_rows: usize) -> Self {
        Self {
            enabled,
            window_ms,
            max_rows,
            counts: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn window_ms(&self) -> u64 {
        self.window_ms
    }

    pub fn record(&self, sample: &HostRouteSample) {
        if self.enabled {
            let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
            *counts.entry(normalize(sample)).or_insert(0) += 1;
        }
    }

    fn summary_of(&self, counts: &BTreeMap<HostRouteKey, u64>) -> Option<HostRouteSummary> {
        let rows: Vec<HostRouteRow> = counts
            .iter()
            .map(|(key, count)| HostRouteRow {
                key: key.clone(),
                count: *count,
            })
            .collect();
        summarize(&rows, self.window_ms as f64 / 1000.0, self.max_rows)
    }

    pub fn peek(&self) -> Option<HostRouteSummary> {
        self.summary_of(&self.counts.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The summary line of the window so far, if there was traffic; the counts start again.
    pub fn flush(&self) -> Option<String> {
        let counts = std::mem::take(&mut *self.counts.lock().unwrap_or_else(PoisonError::into_inner));
        self.summary_of(&counts)
            .map(|summary| format!("[HostRoute] summary {}", summary.to_json()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(method: &str, route: &str, status: u16, session: Option<&str>, hub: Option<&str>) -> HostRouteSample {
        HostRouteSample {
            method: method.into(),
            route: route.into(),
            status_code: status,
            session_kind: session.map(Into::into),
            hub_kind: hub.map(Into::into),
        }
    }

    #[test]
    fn status_classes() {
        let classes: Vec<&str> = [99, 100, 204, 302, 404, 599, 600].map(status_class).to_vec();
        assert_eq!(classes, ["other", "1xx", "2xx", "3xx", "4xx", "5xx", "other"]);
    }

    #[test]
    fn normalization() {
        let key = normalize(&sample("TRACE", "", 200, Some("realm_hub"), Some("RAMSGATE")));
        assert_eq!(key.method, "OTHER");
        assert_eq!(key.route, "(no route)");
        assert_eq!(key.session_kind, "unknown");
        assert_eq!(key.hub_kind.as_deref(), Some("RAMSGATE"));
        let long = "/".repeat(161);
        assert_eq!(normalize(&sample("get", &long, 200, None, None)).route, "(no route)");
        assert_eq!(normalize(&sample("get", &long, 200, None, None)).method, "OTHER");
        assert_eq!(
            normalize(&sample("GET", &long[..160], 200, None, None)).route.len(),
            160
        );
        let kinds = normalize(&sample("PUT", "/x", 500, Some("REALM_HUB"), Some(&"A".repeat(33))));
        assert_eq!((kinds.session_kind.as_str(), kinds.hub_kind), ("REALM_HUB", None));
        assert_eq!(route_of(Some("/inventory/:userId"), "/api"), "/api/inventory/:userId");
        assert_eq!(route_of(None, "/api"), "(no route)");
    }

    #[test]
    fn merge_sort_and_cap() {
        let samples = [
            sample("GET", "/b", 200, None, None),
            sample("GET", "/a", 200, None, None),
            sample("POST", "/c", 201, None, None),
            sample("POST", "/c", 204, None, None),
            sample("GET", "/b", 404, None, None),
        ];
        let summary = summarize(&aggregate(&samples), 60.0, 2).unwrap();
        assert_eq!(summary.total, 5);
        assert_eq!(summary.omitted_rows, 2);
        let routes: Vec<(&str, u64)> = summary.routes.iter().map(|r| (r.key.route.as_str(), r.count)).collect();
        assert_eq!(routes, [("/c", 2), ("/a", 1)]);
        assert!(summarize(&[], 60.0, 2).is_none());
        let zero = [HostRouteRow {
            key: normalize(&sample("GET", "/z", 200, None, None)),
            count: 0,
        }];
        assert!(summarize(&zero, 60.0, 2).is_none());
    }

    #[test]
    fn summary_line_is_exact() {
        let census = HostRouteCensus::new(true);
        census.record(&sample("GET", "/inventory/:userId", 200, None, None));
        census.record(&sample(
            "POST",
            "/progression/:userId/:track",
            409,
            Some("REALM_HUB"),
            Some("RAMSGATE"),
        ));
        census.record(&sample("GET", "/inventory/:userId", 204, None, None));
        assert_eq!(census.peek().unwrap().total, 3);
        assert_eq!(
            census.flush().unwrap(),
            r#"[HostRoute] summary {"windowSeconds":60,"total":3,"routes":[{"method":"GET","route":"/inventory/:userId","status":"2xx","sessionKind":"unknown","hubKind":null,"count":2},{"method":"POST","route":"/progression/:userId/:track","status":"4xx","sessionKind":"REALM_HUB","hubKind":"RAMSGATE","count":1}],"omittedRows":0}"#
        );
        assert_eq!(census.flush(), None);
        assert_eq!(census.window_ms(), 60_000);
    }

    #[test]
    fn disabled_counts_nothing() {
        let census = HostRouteCensus::with_window(false, 1500, 10);
        assert!(!census.enabled());
        census.record(&sample("GET", "/x", 200, None, None));
        assert!(census.peek().is_none());
        let on = HostRouteCensus::with_window(true, 1500, 10);
        on.record(&sample("GET", "/x", 200, None, None));
        assert!(on.flush().unwrap().contains(r#""windowSeconds":1.5"#));
    }
}
