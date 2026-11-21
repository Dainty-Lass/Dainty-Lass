// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The host route census: which routes hosted runtimes call and how they are answered, summed per window.

use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard, PoisonError},
};

use rifts_seam::HostRouteSample;

use crate::json::{js_length, js_number, js_string};

pub const HOST_ROUTE_CENSUS_WINDOW_MS: u64 = 60_000;
pub const HOST_ROUTE_CENSUS_MAX_ROWS: usize = 200;

const NO_ROUTE: &str = "(no route)";
const METHODS: [&str; 7] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HostRouteKey {
    pub method: String,
    pub route: String,
    pub status: String,
    pub session_kind: String,
    pub hub_kind: Option<String>,
}

impl HostRouteKey {
    /// The fields joined by `|`, an absent hub kind as empty: the tie-break order.
    fn sort_text(&self) -> String {
        let hub = self.hub_kind.as_deref().unwrap_or_default();
        [self.method.as_str(), &self.route, &self.status, &self.session_kind, hub].join("|")
    }

    fn to_json(&self) -> String {
        let hub = self.hub_kind.as_deref().map(js_string);
        let fields = [
            ("method", js_string(&self.method)),
            ("route", js_string(&self.route)),
            ("status", js_string(&self.status)),
            ("sessionKind", js_string(&self.session_kind)),
            ("hubKind", hub.unwrap_or_else(|| "null".into())),
        ];
        fields
            .iter()
            .map(|(name, value)| format!(r#""{name}":{value}"#))
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostRouteRow {
    pub key: HostRouteKey,
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HostRouteSummary {
    pub window_seconds: f64,
    pub total: u64,
    pub routes: Vec<HostRouteRow>,
    pub omitted_rows: u64,
}

impl HostRouteSummary {
    pub fn to_json(&self) -> String {
        let rows: Vec<String> = self
            .routes
            .iter()
            .map(|row| format!(r#"{{{},"count":{}}}"#, row.key.to_json(), row.count))
            .collect();
        format!(
            r#"{{"windowSeconds":{},"total":{},"routes":[{}],"omittedRows":{}}}"#,
            js_number(self.window_seconds),
            self.total,
            rows.join(","),
            self.omitted_rows
        )
    }
}

pub fn status_class(code: u16) -> &'static str {
    const CLASSES: [&str; 5] = ["1xx", "2xx", "3xx", "4xx", "5xx"];
    let hundreds = usize::from(code / 100);
    match hundreds {
        1..=5 => CLASSES[hundreds - 1],
        _ => "other",
    }
}

/// Upper-case and `_`, 1 to 32 bytes; anything else is not a label.
fn kind_label(value: Option<&str>) -> Option<String> {
    let label = value?;
    let shaped = (1..=32).contains(&label.len()) && label.bytes().all(|b| b == b'_' || b.is_ascii_uppercase());
    shaped.then(|| label.to_owned())
}

pub fn normalize(sample: &HostRouteSample) -> HostRouteKey {
    let known_method = METHODS.iter().any(|m| *m == sample.method);
    let route_fits = (1..=160).contains(&js_length(&sample.route));
    HostRouteKey {
        method: if known_method {
            sample.method.clone()
        } else {
            String::from("OTHER")
        },
        route: if route_fits {
            sample.route.clone()
        } else {
            String::from(NO_ROUTE)
        },
        status: status_class(sample.status_code).into(),
        session_kind: kind_label(sample.session_kind.as_deref()).unwrap_or_else(|| "unknown".into()),
        hub_kind: kind_label(sample.hub_kind.as_deref()),
    }
}

/// The route's label: the mount's base and the matched pattern.
pub fn route_of(pattern: Option<&str>, base_url: &str) -> String {
    match pattern {
        Some(pattern) => [base_url, pattern].concat(),
        None => NO_ROUTE.into(),
    }
}

fn count_into(counts: &mut BTreeMap<HostRouteKey, u64>, sample: &HostRouteSample) {
    *counts.entry(normalize(sample)).or_default() += 1;
}

fn rows_of(counts: &BTreeMap<HostRouteKey, u64>) -> Vec<HostRouteRow> {
    let row = |(key, count): (&HostRouteKey, &u64)| HostRouteRow {
        key: key.clone(),
        count: *count,
    };
    counts.iter().map(row).collect()
}

pub fn aggregate(samples: &[HostRouteSample]) -> Vec<HostRouteRow> {
    let mut counts = BTreeMap::new();
    samples.iter().for_each(|sample| count_into(&mut counts, sample));
    rows_of(&counts)
}

/// Busiest first, ties by the key's text; at most `max_rows` rows, the rest counted as omitted.
pub fn summarize(rows: &[HostRouteRow], window_seconds: f64, max_rows: usize) -> Option<HostRouteSummary> {
    let mut kept: Vec<(String, &HostRouteRow)> = rows
        .iter()
        .filter(|row| row.count != 0)
        .map(|row| (row.key.sort_text(), row))
        .collect();
    if kept.is_empty() {
        return None;
    }
    kept.sort_by(|(a_text, a), (b_text, b)| b.count.cmp(&a.count).then_with(|| a_text.cmp(b_text)));
    let total = kept.iter().map(|(_, row)| row.count).sum();
    let omitted_rows = kept.len().saturating_sub(max_rows) as u64;
    let routes = kept.into_iter().take(max_rows).map(|(_, row)| row.clone()).collect();
    Some(HostRouteSummary {
        window_seconds,
        total,
        routes,
        omitted_rows,
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
        let counts = Mutex::default();
        Self {
            enabled,
            window_ms,
            max_rows,
            counts,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn window_ms(&self) -> u64 {
        self.window_ms
    }

    fn counts(&self) -> MutexGuard<'_, BTreeMap<HostRouteKey, u64>> {
        self.counts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn record(&self, sample: &HostRouteSample) {
        if !self.enabled {
            return;
        }
        count_into(&mut self.counts(), sample);
    }

    fn summary_of(&self, counts: &BTreeMap<HostRouteKey, u64>) -> Option<HostRouteSummary> {
        let seconds = self.window_ms as f64 / 1000.0;
        summarize(&rows_of(counts), seconds, self.max_rows)
    }

    pub fn peek(&self) -> Option<HostRouteSummary> {
        let counts = self.counts();
        self.summary_of(&counts)
    }

    /// The summary line of the window so far, if there was traffic; the counts start again.
    pub fn flush(&self) -> Option<String> {
        let window = std::mem::take(&mut *self.counts());
        let summary = self.summary_of(&window)?;
        Some(["[HostRoute] summary ", &summary.to_json()].concat())
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

    fn plain(method: &str, route: &str, status: u16) -> HostRouteSample {
        sample(method, route, status, None, None)
    }

    #[test]
    fn status_classes() {
        let pairs = [
            (99, "other"),
            (100, "1xx"),
            (204, "2xx"),
            (302, "3xx"),
            (404, "4xx"),
            (599, "5xx"),
            (600, "other"),
        ];
        for (code, class) in pairs {
            assert_eq!(status_class(code), class, "{code}");
        }
    }

    #[test]
    fn normalization() {
        let key = normalize(&sample("TRACE", "", 200, Some("realm_hub"), Some("RAMSGATE")));
        let fields = (
            key.method.as_str(),
            key.route.as_str(),
            key.session_kind.as_str(),
            key.hub_kind.as_deref(),
        );
        assert_eq!(fields, ("OTHER", "(no route)", "unknown", Some("RAMSGATE")));
        let long = "/".repeat(161);
        let too_long = normalize(&plain("get", &long, 200));
        assert_eq!(
            (too_long.route.as_str(), too_long.method.as_str()),
            ("(no route)", "OTHER")
        );
        assert_eq!(normalize(&plain("GET", &long[..160], 200)).route.len(), 160);
        let kinds = normalize(&sample("PUT", "/x", 500, Some("REALM_HUB"), Some(&"A".repeat(33))));
        assert_eq!((kinds.session_kind.as_str(), kinds.hub_kind), ("REALM_HUB", None));
        assert_eq!(route_of(Some("/inventory/:userId"), "/api"), "/api/inventory/:userId");
        assert_eq!(route_of(None, "/api"), "(no route)");
    }

    #[test]
    fn merge_sort_and_cap() {
        let samples = [
            plain("GET", "/b", 200),
            plain("GET", "/a", 200),
            plain("POST", "/c", 201),
            plain("POST", "/c", 204),
            plain("GET", "/b", 404),
        ];
        let summary = summarize(&aggregate(&samples), 60.0, 2).unwrap();
        assert_eq!((summary.total, summary.omitted_rows), (5, 2));
        let routes: Vec<(&str, u64)> = summary.routes.iter().map(|r| (r.key.route.as_str(), r.count)).collect();
        assert_eq!(routes, [("/c", 2), ("/a", 1)]);
        assert!(summarize(&[], 60.0, 2).is_none());
        let zero = HostRouteRow {
            key: normalize(&plain("GET", "/z", 200)),
            count: 0,
        };
        assert!(summarize(&[zero], 60.0, 2).is_none());
    }

    #[test]
    fn summary_line_is_exact() {
        let census = HostRouteCensus::new(true);
        let samples = [
            plain("GET", "/inventory/:userId", 200),
            sample(
                "POST",
                "/progression/:userId/:track",
                409,
                Some("REALM_HUB"),
                Some("RAMSGATE"),
            ),
            plain("GET", "/inventory/:userId", 204),
        ];
        samples.iter().for_each(|s| census.record(s));
        assert_eq!(census.peek().unwrap().total, 3);
        let want = concat!(
            r#"[HostRoute] summary {"windowSeconds":60,"total":3,"routes":["#,
            r#"{"method":"GET","route":"/inventory/:userId","status":"2xx","sessionKind":"unknown","hubKind":null,"count":2},"#,
            r#"{"method":"POST","route":"/progression/:userId/:track","status":"4xx","sessionKind":"REALM_HUB","hubKind":"RAMSGATE","count":1}"#,
            r#"],"omittedRows":0}"#
        );
        assert_eq!(census.flush().as_deref(), Some(want));
        assert_eq!(census.flush(), None);
        assert_eq!(census.window_ms(), 60_000);
    }

    #[test]
    fn disabled_counts_nothing() {
        let off = HostRouteCensus::with_window(false, 1500, 10);
        assert!(!off.enabled());
        off.record(&plain("GET", "/x", 200));
        assert!(off.peek().is_none());
        let on = HostRouteCensus::with_window(true, 1500, 10);
        on.record(&plain("GET", "/x", 200));
        assert!(on.flush().unwrap().contains(r#""windowSeconds":1.5"#));
    }
}
