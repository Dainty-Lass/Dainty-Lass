// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! A hunt against a backend that takes 400 ms for every call but host-ready (spec §5.3 "Slow backend"): the session
//! thread never waits on it, so gameplay keeps its latency.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::time::{Duration, Instant};

use rustic_rifts_transport::{Ready, Transport};
use serde_json::Value;

use common::backend::FakeBackend;
use common::{Carrier, Client, Echo, End, Files, HostSpec, JoinSpec, Signer};

#[test]
fn a_slow_backend_does_not_slow_the_hunt() {
    let signer = Signer::new();
    let backend = FakeBackend::start();
    backend.set_delay(Duration::from_millis(400));
    let echo = Echo::start();
    let bindings = common::bindings(30, 1000);
    let hosts = Transport::new();
    let joins = Transport::new();
    let host_files = Files::new();
    let started = Instant::now();
    let host = common::open_host(
        &hosts,
        &host_files,
        &signer,
        &HostSpec {
            bindings: &bindings,
            backend_url: backend.url.clone(),
            server_port: echo.port,
            role: None,
            maximum_peers: 4,
            local_client: None,
            peers: vec![("join-account".to_owned(), 2000)],
            extra: String::new(),
            secrets_extra: String::new(),
        },
        Some(&signer.mint(&bindings, "host-account", 1000, 1000)),
    )
    .unwrap();
    assert!(matches!(hosts.wait_ready(host, 20_000), Ready::Ready(_)));
    assert!(started.elapsed() < Duration::from_secs(20));

    let join_files = Files::new();
    let join = common::open_join(
        &joins,
        &join_files,
        &signer,
        &JoinSpec {
            bindings: &bindings,
            local_peer: 2000,
            local_account: "join-account",
            host_account: "host-account",
            extra: String::new(),
            secrets_extra: String::new(),
        },
        &signer.mint(&bindings, "join-account", 2000, 1000),
    );
    let carrier = Carrier {
        pairs: vec![(
            End {
                transport: hosts.clone(),
                handle: host,
                account: "host-account".to_owned(),
            },
            End {
                transport: joins.clone(),
                handle: join,
                account: "join-account".to_owned(),
            },
        )],
    };
    let route = common::wait_ready_carrying(&joins, join, &carrier, Duration::from_secs(30)).unwrap();
    let client = Client::new(common::field(&route, "localPort").unwrap().parse().unwrap());
    assert!(client.round_trip(500, Duration::from_secs(5)).is_some());

    let traffic = Instant::now();
    let mut trips = 0;
    let mut worst = Duration::ZERO;
    while traffic.elapsed() < Duration::from_secs(5) {
        carrier.carry();
        let took = client
            .round_trip(500, Duration::from_secs(1))
            .expect("a round trip was lost");
        worst = worst.max(took);
        trips += 1;
    }
    assert!(trips >= 50, "{trips} round trips");
    assert!(worst < Duration::from_millis(200), "worst {worst:?}");
    let log = backend.log();
    assert!(log.leases.len() >= 2);
    assert!(log.leases.iter().any(|c| c.body.contains("\"admittedPeerCount\":1")));
    let mut events = Vec::new();
    common::drain_events(&hosts, host, &mut events);
    assert_eq!(events.iter().filter(|e| e.starts_with("PEER_ADMITTED")).count(), 1);

    assert!(joins.stop_session(join));
    assert!(hosts.stop_session(host));
    let log = backend.log();
    let mut chain = Vec::new();
    let mut next = 1;
    for call in &log.batches {
        let body: Value = serde_json::from_str(&call.body).unwrap();
        assert_eq!(body["firstSequence"], next);
        for event in body["events"].as_array().unwrap() {
            assert_eq!(event["sequence"], next);
            next += 1;
            chain.push(event["type"].as_str().unwrap().to_owned());
        }
    }
    assert_eq!(&chain[..2], ["SESSION_STARTED", "WORLD_READY"]);
    assert_eq!(chain.last().map(String::as_str), Some("HUNT_ABORTED"));
    assert!(chain.contains(&"PLAYER_ADMITTED".to_owned()));
}
