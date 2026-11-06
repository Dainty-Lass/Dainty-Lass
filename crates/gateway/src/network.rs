// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The network's public routes (DN7, DD19), answered by the gateway itself: the list of the network's
//! nodes the launcher pings, the cheap answer it times, and the stub that takes its ordered regions.
//! A server outside a network lists no nodes, which is how a network with no peers answers too (plan
//! section 6). The shapes here are the launcher's (`net/regionping/parse.rs`) until DN2's
//! `crates/module-api` gives both one definition.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

/// Lists the network's nodes for the launcher.
pub const NODES_PATH: &str = "/network/v1/nodes";
/// Answers a ping, for measuring round trips.
pub const PING_PATH: &str = "/network/v1/ping";
/// Takes a player's region round-trip report.
pub const REGIONS_PATH: &str = "/network/v1/regions";
/// Redeems a remote-management enrollment code.
pub const ENROLL_PATH: &str = "/__node/enroll";
/// Header carrying a home-signed session ticket when the launcher is connected to or reporting to a node (DN5, DN6, DN8).
pub const SESSION_TICKET_HEADER: &str = "x-dainty-session-ticket";

/// Optional handler on a controller gateway that redeems a one-time enrollment token (`POST /__node/enroll`, DD27, RM2).
pub type EnrollHandler = Arc<dyn Fn(Vec<u8>) -> Pin<Box<dyn Future<Output = (u16, Vec<u8>)> + Send>> + Send + Sync>;

/// Optional sink on the gateway that verifies the caller's account key or session ticket and stores the
/// ordered [`RegionRtt`] report in the network plugin (`RingNode`, DN8). Returns `true` when accepted.
pub type RegionReportHandler = Arc<
    dyn Fn(Option<String>, Option<String>, Vec<RegionRtt>) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync,
>;

/// The launcher reads at most this many nodes; a list longer than that is the operator's mistake
pub const MAX_NODES: usize = 16;
/// A report is a few dozen bytes per region
pub const MAX_REPORT_BYTES: usize = 4096;
/// Most regions one report may list.
pub const MAX_REPORT_REGIONS: usize = 32;
/// A minute: anything slower is no answer
pub const MAX_RTT_MS: f64 = 60_000.0;

/// One node as the launcher reaches it: its region, its gateway, and the certificate the launcher pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkNode {
    /// Region id.
    pub region: String,
    /// Gateway host name or address.
    pub host: String,
    /// Gateway port.
    pub port: u16,
    /// SHA-256 of the gateway's certificate, the launcher's pin.
    pub fingerprint: String,
}

/// A region id: lower-case letters, digits and inner hyphens, 1 to 32 long (`oregon`, `new-york`).
pub fn is_region_id(text: &str) -> bool {
    (1..=32).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !text.starts_with('-')
        && !text.ends_with('-')
}

/// `GET /network/v1/nodes`: `{"nodes":[...]}`, empty outside a network.
pub fn nodes_body(nodes: &[NetworkNode]) -> String {
    #[derive(Serialize)]
    struct Nodes<'a> {
        nodes: &'a [NetworkNode],
    }
    serde_json::to_string(&Nodes {
        nodes: &nodes[..nodes.len().min(MAX_NODES)],
    })
    .unwrap_or_else(|_| "{\"nodes\":[]}".into())
}

/// One region of a player's report, nearest first; `None` when the launcher got no answer from it.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionRtt {
    /// Region id.
    pub region: String,
    /// Round trip in milliseconds.
    pub rtt_ms: Option<f64>,
}

/// `POST /network/v1/regions`: `{"regions":[{"region":"oregon","rttMs":42.5},...]}`, exactly those
/// fields, each region once. Anything else refuses the whole report.
pub fn parse_region_report(body: &[u8]) -> Option<Vec<RegionRtt>> {
    if body.len() > MAX_REPORT_BYTES {
        return None;
    }
    let value: Value = serde_json::from_slice(body).ok()?;
    let report = value.as_object()?;
    if report.len() != 1 {
        return None;
    }
    let list = report.get("regions")?.as_array()?;
    if list.len() > MAX_REPORT_REGIONS {
        return None;
    }
    let mut regions: Vec<RegionRtt> = Vec::with_capacity(list.len());
    for entry in list {
        let entry = entry.as_object()?;
        if entry.len() != 2 {
            return None;
        }
        let region = entry.get("region")?.as_str().filter(|id| is_region_id(id))?;
        let rtt_ms = match entry.get("rttMs")? {
            Value::Null => None,
            value => Some(value.as_f64().filter(|ms| (0.0..=MAX_RTT_MS).contains(ms))?),
        };
        if regions.iter().any(|seen| seen.region == region) {
            return None;
        }
        regions.push(RegionRtt {
            region: region.to_owned(),
            rtt_ms,
        });
    }
    Some(regions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_ids() {
        for good in ["oregon", "new-york", "uk", "de-2", "a", &"a".repeat(32)] {
            assert!(is_region_id(good), "{good}");
        }
        for bad in ["", "-uk", "uk-", "New-York", "new york", "uk_2", "é", &"a".repeat(33)] {
            assert!(!is_region_id(bad), "{bad}");
        }
    }

    #[test]
    fn no_network_lists_no_nodes() {
        assert_eq!(nodes_body(&[]), r#"{"nodes":[]}"#);
        let node = NetworkNode {
            region: "oregon".into(),
            host: "or.example.org".into(),
            port: 443,
            fingerprint: "ab".repeat(32),
        };
        assert_eq!(
            nodes_body(std::slice::from_ref(&node)),
            format!(
                r#"{{"nodes":[{{"region":"oregon","host":"or.example.org","port":443,"fingerprint":"{}"}}]}}"#,
                "ab".repeat(32)
            )
        );
        let many = vec![node; MAX_NODES + 4];
        let body: Value = serde_json::from_str(&nodes_body(&many)).unwrap();
        assert_eq!(body["nodes"].as_array().unwrap().len(), MAX_NODES);
    }

    #[test]
    fn a_report_is_read_whole_or_refused() {
        let report = br#"{"regions":[{"region":"uk","rttMs":12.5},{"region":"oregon","rttMs":140},{"region":"de","rttMs":null}]}"#;
        assert_eq!(
            parse_region_report(report),
            Some(vec![
                RegionRtt {
                    region: "uk".into(),
                    rtt_ms: Some(12.5)
                },
                RegionRtt {
                    region: "oregon".into(),
                    rtt_ms: Some(140.0)
                },
                RegionRtt {
                    region: "de".into(),
                    rtt_ms: None
                },
            ])
        );
        assert_eq!(parse_region_report(br#"{"regions":[]}"#), Some(Vec::new()));
        for bad in [
            &br#"{}"#[..],
            br#"[]"#,
            br#"{"regions":{}}"#,
            br#"{"regions":[],"extra":1}"#,
            br#"{"regions":[{"region":"uk"}]}"#,
            br#"{"regions":[{"region":"uk","rttMs":1,"x":2}]}"#,
            br#"{"regions":[{"region":"UK","rttMs":1}]}"#,
            br#"{"regions":[{"region":"uk","rttMs":-1}]}"#,
            br#"{"regions":[{"region":"uk","rttMs":60001}]}"#,
            br#"{"regions":[{"region":"uk","rttMs":"1"}]}"#,
            br#"{"regions":[{"region":"uk","rttMs":1},{"region":"uk","rttMs":2}]}"#,
            b"not json",
        ] {
            assert_eq!(parse_region_report(bad), None, "{}", String::from_utf8_lossy(bad));
        }
        let too_many: Vec<String> = (0..=MAX_REPORT_REGIONS)
            .map(|n| format!(r#"{{"region":"r{n}","rttMs":1}}"#))
            .collect();
        let body = format!(r#"{{"regions":[{}]}}"#, too_many.join(","));
        assert_eq!(parse_region_report(body.as_bytes()), None);
    }
}
