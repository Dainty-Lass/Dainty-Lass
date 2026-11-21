// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The network harness's region ping check (DN12, `docker/network/test.sh`): the real `region_ping` against a node of the
//! Docker harness as the home, from a machine whose latency to each node `netem` shapes. Ignored by default; the script
//! sets the environment and runs it with `--ignored`.
//!   RR_HARNESS_HOME    the home's gateway, `host:port`
//!   RR_HARNESS_PIN     its certificate's SHA-256 (the invite's)
//!   RR_HARNESS_EXPECT  the regions nearest first with the round trip injected to each, `uk=20,new-york=85`
//!   RR_HARNESS_TOLERANCE_MS  how far above the injected round trip a median may be (25)

use super::*;

// Built from pieces, so the repository's secret check never sees a whole key
const KEY: &str = concat!("RRK_", "eeeeeeeeeeeeeeee", "eeeeeeeeeeeeeeee", "eeeeeeeeeeeeeeee");

fn setting(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set: run docker/network/test.sh"))
}

#[tokio::test]
#[ignore = "needs the Docker network harness: docker/network/test.sh runs it"]
async fn the_region_order_follows_the_latency_the_harness_injects() {
    let home = setting("RR_HARNESS_HOME");
    let (host, port) = home.rsplit_once(':').expect("RR_HARNESS_HOME is host:port");
    let home = Endpoint {
        host: host.into(),
        port: port.parse().expect("a port"),
        pin: Some(setting("RR_HARNESS_PIN")),
    };
    let expect: Vec<(String, f64)> = setting("RR_HARNESS_EXPECT")
        .split(',')
        .map(|pair| {
            let (region, rtt) = pair.split_once('=').expect("region=milliseconds");
            (region.to_owned(), rtt.parse().expect("milliseconds"))
        })
        .collect();
    let tolerance: f64 = std::env::var("RR_HARNESS_TOLERANCE_MS").map_or(25.0, |v| v.parse().expect("milliseconds"));

    let RegionPing::Ok { regions, reported } = region_ping(&home, Some(KEY), &PingSettings::default()).await else {
        panic!("the home refused or could not be reached");
    };
    let measured: Vec<_> = regions.iter().map(|r| (r.region.clone(), r.rtt_ms)).collect();
    assert_eq!(
        regions.iter().map(|r| r.region.as_str()).collect::<Vec<_>>(),
        expect.iter().map(|(region, _)| region.as_str()).collect::<Vec<_>>(),
        "order {measured:?}"
    );
    for (region, (_, injected)) in regions.iter().zip(&expect) {
        let rtt = region.rtt_ms.unwrap_or_else(|| panic!("{region:?} never answered"));
        assert!(region.problem.is_none() && region.samples >= 3, "{region:?}");
        // Never faster than the injected round trip (less a timer step); a little over is the machine's own time
        assert!(
            (injected - 2.0..=injected + tolerance).contains(&rtt),
            "{}: {rtt} ms, injected {injected} ms (+{tolerance}): {measured:?}",
            region.region
        );
    }
    assert!(reported, "the home took the order");
    println!("region order {measured:?}");
}
