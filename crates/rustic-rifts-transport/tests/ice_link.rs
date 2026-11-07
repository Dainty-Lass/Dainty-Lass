// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Two links connecting through the signal texts alone: direct, and relay-only through a TURN server on 127.0.0.2.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::time::{Duration, Instant};

use rustic_rifts_transport::link::{Link, LinkConfig, LinkState, RelayServer, Route};
use rustic_rifts_transport::seal::{FrameType, Side};

fn config(side: Side, relays: Vec<RelayServer>, force_relay: bool) -> LinkConfig {
    LinkConfig {
        side,
        pair_key: vec![9; 32],
        stun: None,
        relays,
        force_relay,
    }
}

/// Moves signals both ways until both links are connected.
fn connect(joiner_config: &LinkConfig, host_config: &LinkConfig) -> (Link, Link) {
    let mut joiner = Link::start(joiner_config, None).unwrap();
    let first = joiner.take_signals();
    assert_eq!(first[0].0, "DESCRIPTION");
    let mut host = Link::start(host_config, Some(&first[0].1)).unwrap();
    for (kind, data) in &first[1..] {
        host.apply_signal(kind, data).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let now = Instant::now();
        joiner.pump(now);
        host.pump(now);
        for (kind, data) in joiner.take_signals() {
            host.apply_signal(&kind, &data).unwrap();
        }
        for (kind, data) in host.take_signals() {
            joiner.apply_signal(&kind, &data).unwrap();
        }
        if joiner.state() == LinkState::Connected && host.state() == LinkState::Connected {
            return (joiner, host);
        }
        assert_ne!(joiner.state(), LinkState::Failed);
        assert_ne!(host.state(), LinkState::Failed);
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!(
        "links did not connect: joiner {:?}, host {:?}",
        joiner.state(),
        host.state()
    );
}

fn round_trip(joiner: &mut Link, host: &mut Link, size: usize) {
    let payload: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    assert!(joiner.send(FrameType::Gameplay, &payload));
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let now = Instant::now();
        joiner.pump(now);
        host.pump(now);
        let got = host.receive(64);
        if let Some((kind, message)) = got.into_iter().next() {
            assert_eq!(kind, FrameType::Gameplay);
            assert_eq!(message, payload);
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("{size} bytes did not arrive");
}

#[test]
fn direct() {
    let (mut joiner, mut host) = connect(
        &config(Side::Joiner, Vec::new(), false),
        &config(Side::Host, Vec::new(), false),
    );
    assert_eq!(joiner.route(), Some(Route::Direct));
    for size in [1, 64, 512, 1200, 1400, 2000] {
        round_trip(&mut joiner, &mut host, size);
    }
    assert_eq!(host.counters().frames_rejected, 0);
}

#[test]
fn relay_only_through_local_turn() {
    let server = common::turn_server::TurnServer::start();
    let (mut joiner, mut host) = connect(
        &config(Side::Joiner, vec![server.relay()], true),
        &config(Side::Host, vec![server.relay()], true),
    );
    assert_eq!(joiner.route(), Some(Route::Relay));
    assert_eq!(host.route(), Some(Route::Relay));
    for size in [1, 64, 512, 1200, 1400, 2000] {
        round_trip(&mut joiner, &mut host, size);
    }
    assert_eq!(host.counters().frames_rejected, 0);
    assert_eq!(joiner.counters().frames_rejected, 0);
}

/// Sends `count` tagged messages ~3 ms apart and returns how many never arrived.
fn burst(joiner: &mut Link, host: &mut Link, count: u16, size: usize) -> usize {
    let mut seen = std::collections::HashSet::new();
    let take = |host: &mut Link, seen: &mut std::collections::HashSet<u16>| {
        for (_, message) in host.receive(64) {
            assert_eq!(message.len(), size);
            seen.insert(u16::from_be_bytes([message[0], message[1]]));
        }
    };
    for n in 0..count {
        let mut payload = vec![0xa5; size];
        payload[..2].copy_from_slice(&n.to_be_bytes());
        joiner.send(FrameType::Gameplay, &payload);
        let until = Instant::now() + Duration::from_millis(3);
        while Instant::now() < until {
            let now = Instant::now();
            joiner.pump(now);
            host.pump(now);
            take(host, &mut seen);
        }
    }
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until {
        let now = Instant::now();
        joiner.pump(now);
        host.pump(now);
        take(host, &mut seen);
    }
    usize::from(count) - seen.len()
}

#[test]
fn bursts_direct() {
    let (mut joiner, mut host) = connect(
        &config(Side::Joiner, Vec::new(), false),
        &config(Side::Host, Vec::new(), false),
    );
    assert!(burst(&mut joiner, &mut host, 300, 600) <= 3);
    assert!(burst(&mut joiner, &mut host, 100, 1800) <= 3);
    assert_eq!(host.counters().frames_rejected, 0);
}

#[test]
fn bursts_relay_local() {
    let server = common::turn_server::TurnServer::start();
    let (mut joiner, mut host) = connect(
        &config(Side::Joiner, vec![server.relay()], true),
        &config(Side::Host, vec![server.relay()], true),
    );
    assert!(burst(&mut joiner, &mut host, 300, 600) <= 3);
    assert!(burst(&mut joiner, &mut host, 100, 1800) <= 3);
    assert_eq!(host.counters().frames_rejected, 0);
}

/// Against a real relay: `RIFTS_RELAY_FILE` names a file with `ice_stun_urls`, `ice_turn_urls`, `turn_username` and
/// `turn_credential` (as `rustic-rifts-relay`'s `mint-relay-file` example writes). Nothing secret is printed.
#[test]
#[ignore = "needs a real relay's credentials in RIFTS_RELAY_FILE and network access"]
fn relay_file() {
    use rtc_ice::candidate::CandidateType;

    let path = std::env::var("RIFTS_RELAY_FILE").expect("RIFTS_RELAY_FILE");
    let text = std::fs::read_to_string(path).unwrap();
    let fields = rustic_rifts_transport::files::Fields::parse(&text).unwrap();
    let material = rustic_rifts_transport::files::ice_material(&fields, &fields).unwrap();
    assert!(!material.relays.is_empty());
    let joiner = LinkConfig {
        stun: material.stun.clone(),
        ..config(Side::Joiner, material.relays.clone(), true)
    };
    let host = LinkConfig {
        stun: material.stun.clone(),
        ..config(Side::Host, material.relays.clone(), true)
    };
    let (mut joiner, mut host) = connect(&joiner, &host);
    assert_eq!(joiner.selected_types().map(|(l, _)| l), Some(CandidateType::Relay));
    assert_eq!(host.selected_types().map(|(l, _)| l), Some(CandidateType::Relay));
    for size in [1, 64, 512, 1200, 1400, 2000] {
        round_trip(&mut joiner, &mut host, size);
    }
}
