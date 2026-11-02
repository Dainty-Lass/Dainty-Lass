// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! PLACEHOLDER for a Category A file (DN7). The real `network.rs` lives in dainty-restricted and moves here
//! unchanged at integration (docs/port/clean-room/gateway.md); the clean-room implementer never saw it. This
//! stand-in holds only what the rewritten gateway calls, from spec-rd27-gateway-front.md section 9.3 and the
//! launcher's report body (`launcher-kit`'s `regionping`).

use std::sync::Arc;

use module_api::BoxFuture;
use serde_json::Value;
use service_plugin::gateway::RegionRtt;

/// A shape-checked `POST /network/v1/regions`.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionReport {
    pub api_key: Option<String>,
    pub ticket: Option<String>,
    pub regions: Vec<RegionRtt>,
}

/// Whether the core accepted a report's credentials.
pub type RegionHandler = Arc<dyn Fn(RegionReport) -> BoxFuture<'static, bool> + Send + Sync>;

const MAX_REGIONS: usize = 64;

/// `{"regions":[{"region":"uk","rttMs":5.0},...]}`, each round trip a number or null.
pub fn parse_region_report(body: &[u8]) -> Option<Vec<RegionRtt>> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let regions = value.as_object()?.get("regions")?.as_array()?;
    if regions.len() > MAX_REGIONS {
        return None;
    }
    regions
        .iter()
        .map(|entry| {
            let entry = entry.as_object()?;
            let region = entry.get("region")?.as_str().filter(|r| (1..=64).contains(&r.len()))?;
            let rtt_ms = match entry.get("rttMs") {
                None | Some(Value::Null) => None,
                Some(Value::Number(n)) => Some(n.as_f64().filter(|v| v.is_finite() && *v >= 0.0)?),
                Some(_) => return None,
            };
            Some(RegionRtt {
                region: region.to_owned(),
                rtt_ms,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_shape() {
        let report = parse_region_report(br#"{"regions":[{"region":"uk","rttMs":5.0},{"region":"at","rttMs":null}]}"#);
        assert_eq!(report.map(|r| r.len()), Some(2));
        assert_eq!(parse_region_report(b"[]"), None);
        assert_eq!(parse_region_report(br#"{"regions":[{"region":1}]}"#), None);
    }
}
