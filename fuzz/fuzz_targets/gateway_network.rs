// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The region report a player's launcher posts to the gateway (docs/dainty-lass-plan.md DN7, RX3;
//! docs/security/region-ping.md): a body from the network must be read whole or refused, never
//! panic, and what is read holds only region ids and round trips in range, each region once.
#![no_main]

use dainty_gateway::network::{MAX_REPORT_BYTES, MAX_REPORT_REGIONS, MAX_RTT_MS, is_region_id, parse_region_report};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some(regions) = parse_region_report(data) else {
        return;
    };
    assert!(data.len() <= MAX_REPORT_BYTES && regions.len() <= MAX_REPORT_REGIONS);
    for (at, region) in regions.iter().enumerate() {
        assert!(is_region_id(&region.region));
        assert!(region.rtt_ms.is_none_or(|ms| (0.0..=MAX_RTT_MS).contains(&ms)));
        assert!(regions[..at].iter().all(|seen| seen.region != region.region));
    }
});
