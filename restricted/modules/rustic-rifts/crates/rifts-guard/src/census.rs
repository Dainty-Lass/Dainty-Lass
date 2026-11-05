//! The host route census (`p2p/observability/hostRouteCensus.ts`): which routes a player-host runtime calls, counted
//! by method, route pattern, status class and session kind, one `[HostRoute] summary` line per window with traffic.
//! It only observes. The window's timer is isles' sweeper: it calls [`HostRouteCensus::flush`] each window.

use std::collections::BTreeMap;

use crate::json::{js_number, js_string};

pub const HOST_ROUTE_CENSUS_WINDOW_MS: u64 = 60_000;
pub const HOST_ROUTE_CENSUS_MAX_ROWS: usize = 200;
const KNOWN_METHODS: &[&str] = &["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
const MAX_ROUTE_LENGTH: usize = 160;
const NO_ROUTE: &str = "(no route)";

/// One answered host request. `route` is the Express pattern with its mount path, never a URL.
pub use rifts_seam::HostRouteSample;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HostRouteKey {
    pub method: String,
    pub route: String,
    pub status: String,
    pub session_kind: String,
    pub hub_kind: Option<String>,
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
    pub omitted_rows: usize,
}

/// `HostRouteStatusClass`.
pub fn status_class(status_code: u16) -> String {
    if (100..=599).contains(&status_code) {
        format!("{}xx", status_code / 100)
    } else {
        "other".to_owned()
    }
}

fn kind_label(kind: Option<&str>) -> Option<String> {
    kind.filter(|kind| (1..=32).contains(&kind.len()) && kind.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'))
        .map(str::to_owned)
}

/// `NormalizeHostRouteSample`: a bounded label set.
pub fn normalize(sample: &HostRouteSample) -> HostRouteKey {
    let route_length = sample.route.encode_utf16().count();
    HostRouteKey {
        method: if KNOWN_METHODS.contains(&sample.method.as_str()) {
            sample.method.clone()
        } else {
            "OTHER".to_owned()
        },
        route: if (1..=MAX_ROUTE_LENGTH).contains(&route_length) {
            sample.route.clone()
        } else {
            NO_ROUTE.to_owned()
        },
        status: status_class(sample.status_code),
        session_kind: kind_label(sample.session_kind.as_deref()).unwrap_or_else(|| "unknown".to_owned()),
        hub_kind: kind_label(sample.hub_kind.as_deref()),
    }
}

/// `HostRouteOf`: the pattern with the router's mount path; `None` when no route answered.
pub fn route_of(pattern: Option<&str>, base_url: &str) -> String {
    pattern.map_or_else(|| NO_ROUTE.to_owned(), |pattern| format!("{base_url}{pattern}"))
}

fn row_key(key: &HostRouteKey) -> String {
    [
        key.method.as_str(),
        key.route.as_str(),
        key.status.as_str(),
        key.session_kind.as_str(),
        key.hub_kind.as_deref().unwrap_or_default(),
    ]
    .join("|")
}

/// `AggregateHostRouteSamples`.
pub fn aggregate<'a>(samples: impl IntoIterator<Item = &'a HostRouteSample>) -> Vec<HostRouteRow> {
    let mut counts: BTreeMap<HostRouteKey, u64> = BTreeMap::new();
    for sample in samples {
        *counts.entry(normalize(sample)).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|(key, count)| HostRouteRow { key, count })
        .collect()
}

/// `SummarizeHostRoutes`: most requests first, ties by key (byte order, where Node uses `localeCompare`), capped.
pub fn summarize(rows: &[HostRouteRow], window_seconds: f64, max_rows: usize) -> Option<HostRouteSummary> {
    let mut counted: Vec<HostRouteRow> = rows.iter().filter(|row| row.count > 0).cloned().collect();
    if counted.is_empty() {
        return None;
    }
    counted.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| row_key(&left.key).cmp(&row_key(&right.key)))
    });
    let total = counted.iter().map(|row| row.count).sum();
    let omitted_rows = counted.len().saturating_sub(max_rows);
    counted.truncate(max_rows);
    Some(HostRouteSummary {
        window_seconds,
        total,
        routes: counted,
        omitted_rows,
    })
}

impl HostRouteSummary {
    /// `JSON.stringify(summary)`.
    pub fn to_json(&self) -> String {
        let routes: Vec<String> = self
            .routes
            .iter()
            .map(|row| {
                format!(
                    r#"{{"method":{},"route":{},"status":{},"sessionKind":{},"hubKind":{},"count":{}}}"#,
                    js_string(&row.key.method),
                    js_string(&row.key.route),
                    js_string(&row.key.status),
                    js_string(&row.key.session_kind),
                    row.key.hub_kind.as_deref().map_or_else(|| "null".to_owned(), js_string),
                    row.count
                )
            })
            .collect();
        format!(
            r#"{{"windowSeconds":{},"total":{},"routes":[{}],"omittedRows":{}}}"#,
            js_number(self.window_seconds),
            self.total,
            routes.join(","),
            self.omitted_rows
        )
    }
}

/// `HostRouteCensus` without its timer.
#[derive(Debug)]
pub struct HostRouteCensus {
    enabled: bool,
    window_ms: u64,
    max_rows: usize,
    counts: BTreeMap<HostRouteKey, u64>,
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
            counts: BTreeMap::new(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn window_ms(&self) -> u64 {
        self.window_ms
    }

    pub fn record(&mut self, sample: &HostRouteSample) {
        if self.enabled {
            *self.counts.entry(normalize(sample)).or_insert(0) += 1;
        }
    }

    /// The window so far, without clearing it.
    pub fn peek(&self) -> Option<HostRouteSummary> {
        let rows: Vec<HostRouteRow> = self
            .counts
            .iter()
            .map(|(key, count)| HostRouteRow {
                key: key.clone(),
                count: *count,
            })
            .collect();
        summarize(&rows, self.window_ms as f64 / 1000.0, self.max_rows)
    }

    /// The window's line, when there was traffic, and a fresh window.
    pub fn flush(&mut self) -> Option<String> {
        let summary = self.peek();
        self.counts.clear();
        summary.map(|summary| format!("[HostRoute] summary {}", summary.to_json()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(method: &str, route: &str, status: u16) -> HostRouteSample {
        HostRouteSample {
            method: method.to_owned(),
            route: route.to_owned(),
            status_code: status,
            session_kind: None,
            hub_kind: None,
        }
    }

    fn hub_sample(method: &str, route: &str, status: u16, hub: &str) -> HostRouteSample {
        HostRouteSample {
            session_kind: Some("REALM_HUB".to_owned()),
            hub_kind: Some(hub.to_owned()),
            ..sample(method, route, status)
        }
    }

    // hostRouteCensus.test.ts
    #[test]
    fn status_classes() {
        assert_eq!(status_class(200), "2xx");
        assert_eq!(status_class(409), "4xx");
        assert_eq!(status_class(503), "5xx");
        assert_eq!(status_class(0), "other");
        assert_eq!(status_class(99), "other");
        assert_eq!(status_class(600), "other");
    }

    #[test]
    fn a_sample_is_counted_as_a_bounded_label_set() {
        let key = normalize(&hub_sample("PROPFIND", "/x", 200, "RAMSGATE"));
        assert_eq!(
            (
                key.method.as_str(),
                key.route.as_str(),
                key.status.as_str(),
                key.session_kind.as_str()
            ),
            ("OTHER", "/x", "2xx", "REALM_HUB")
        );
        assert_eq!(key.hub_kind.as_deref(), Some("RAMSGATE"));
        let empty = normalize(&sample("GET", "", 200));
        assert_eq!(
            (empty.route.as_str(), empty.session_kind.as_str(), empty.hub_kind),
            ("(no route)", "unknown", None)
        );
        let odd = HostRouteSample {
            session_kind: Some("player 123/abc".to_owned()),
            ..sample("GET", "/y", 200)
        };
        assert_eq!(normalize(&odd).session_kind, "unknown");
        assert_eq!(normalize(&sample("GET", &"/".repeat(161), 200)).route, "(no route)");
    }

    #[test]
    fn the_route_label_is_the_pattern_with_its_mount_path() {
        assert_eq!(route_of(Some("/inventory/:userId"), ""), "/inventory/:userId");
        assert_eq!(
            route_of(Some("/:huntId/lease"), "/mystic/v1/hunts"),
            "/mystic/v1/hunts/:huntId/lease"
        );
        assert_eq!(route_of(None, ""), "(no route)");
    }

    #[test]
    fn samples_merge_and_the_summary_sorts_and_caps() {
        let samples = [
            sample("GET", "/a", 200),
            sample("GET", "/a", 204),
            sample("GET", "/a", 404),
            hub_sample("POST", "/a", 200, "TRAINING"),
        ];
        let rows = aggregate(samples.iter());
        assert_eq!(rows.len(), 3);
        let summary = summarize(&rows, 60.0, HOST_ROUTE_CENSUS_MAX_ROWS).unwrap();
        assert_eq!(
            summary.routes.iter().map(|row| row.count).collect::<Vec<_>>(),
            [2, 1, 1]
        );
        assert_eq!((summary.total, summary.omitted_rows), (4, 0));
        assert!(summarize(&[], 60.0, 10).is_none(), "an idle window has no summary");
        let many: Vec<HostRouteRow> = (1..=5)
            .map(|count| HostRouteRow {
                key: normalize(&sample("GET", &format!("/r{count}"), 200)),
                count,
            })
            .collect();
        let capped = summarize(&many, 60.0, 3).unwrap();
        assert_eq!(capped.routes.iter().map(|row| row.count).collect::<Vec<_>>(), [5, 4, 3]);
        assert_eq!((capped.omitted_rows, capped.total), (2, 15));
    }

    #[test]
    fn ties_are_ordered_by_key() {
        let rows = aggregate([sample("POST", "/b", 200), sample("GET", "/a", 200)].iter());
        let summary = summarize(&rows, 60.0, 10).unwrap();
        assert_eq!(
            summary
                .routes
                .iter()
                .map(|row| row.key.method.as_str())
                .collect::<Vec<_>>(),
            ["GET", "POST"]
        );
    }

    #[test]
    fn one_summary_line_per_window_with_traffic() {
        let mut census = HostRouteCensus::new(true);
        census.record(&sample("GET", "/inventory/:userId", 200));
        census.record(&sample("GET", "/inventory/:userId", 200));
        census.record(&hub_sample("POST", "/progression/:userId/:track", 409, "RAMSGATE"));
        assert_eq!(
            census.flush().unwrap(),
            concat!(
                r#"[HostRoute] summary {"windowSeconds":60,"total":3,"routes":["#,
                r#"{"method":"GET","route":"/inventory/:userId","status":"2xx","sessionKind":"unknown","#,
                r#""hubKind":null,"count":2},"#,
                r#"{"method":"POST","route":"/progression/:userId/:track","status":"4xx","sessionKind":"REALM_HUB","#,
                r#""hubKind":"RAMSGATE","count":1}"#,
                r#"],"omittedRows":0}"#
            )
        );
        assert_eq!(census.flush(), None, "an idle window logs nothing");
        census.record(&sample("GET", "/x", 200));
        assert!(census.flush().unwrap().contains(r#""total":1,"#));
    }

    #[test]
    fn peek_reads_the_window_without_clearing_it() {
        let mut census = HostRouteCensus::new(true);
        census.record(&sample("GET", "/x", 200));
        assert_eq!(census.peek().unwrap().total, 1);
        assert_eq!(census.peek().unwrap().total, 1);
    }

    #[test]
    fn disabled_counts_nothing() {
        let mut census = HostRouteCensus::new(false);
        census.record(&sample("GET", "/x", 200));
        assert!(census.peek().is_none());
        assert!(census.flush().is_none());
    }
}
