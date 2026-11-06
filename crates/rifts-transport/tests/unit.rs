// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The unit-level scenarios of the transport spec (§5.1): crypto, parsing, the lease, held signals and the roster,
//! the lane, the late consume and the local bridge.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::collections::HashSet;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rifts_transport::backend::{Answer, BackendClient, Lane, LeaseOutcome, LeaseTracker, classify_lease};
use rifts_transport::bridge::{HostLocalBridge, HostRemoteBridge, JoinerBridge};
use rifts_transport::files::{Fields, ice_material};
use rifts_transport::filter::{CANDIDATE, DESCRIPTION, relay_only};
use rifts_transport::host::{HostConfig, HostSession, Timing};
use rifts_transport::join::{JoinConfig, JoinSession};
use rifts_transport::link::{Link, LinkConfig};
use rifts_transport::seal::{FrameType, PairCipher, Side, decode_pair_key};
use rifts_transport::ticket::{self, Expected};
use rifts_transport::url::{parse_stun, parse_turn_udp};

use common::backend::FakeBackend;
use common::{CAPABILITY, PAIR_KEY, Signer};

#[test]
fn crypto() {
    let key = decode_pair_key(PAIR_KEY).unwrap();
    assert_eq!(decode_pair_key(&PAIR_KEY[..42]), None);
    assert_eq!(decode_pair_key(&format!("{}+", &PAIR_KEY[..42])), None);
    // The last character's spare bits must be zero.
    assert_eq!(decode_pair_key(&format!("{}B", &PAIR_KEY[..42])), None);

    let host = PairCipher::new(&key, Side::Host).unwrap();
    let joiner = PairCipher::new(&key, Side::Joiner).unwrap();
    let payload = vec![0x5a; 1200];
    let frame = host.seal(FrameType::Gameplay, &payload).unwrap();
    assert_eq!(frame.len(), 1200 + 28);
    assert_eq!(&frame[..3], b"RX\x01");
    assert_eq!(joiner.open(&frame), Some((FrameType::Gameplay, payload.clone())));
    assert_eq!(joiner.open(&frame), None, "replayed");
    let fresh = host.seal(FrameType::Gameplay, &payload).unwrap();
    assert_eq!(host.open(&fresh), None, "own direction");
    let mut flipped = fresh.clone();
    flipped[20] ^= 1;
    assert_eq!(joiner.open(&flipped), None, "ciphertext");
    let mut retyped = fresh.clone();
    retyped[3] = FrameType::Control as u8;
    assert_eq!(joiner.open(&retyped), None, "type");
    let other = PairCipher::new(&[1; 32], Side::Joiner).unwrap();
    assert_eq!(other.open(&fresh), None, "key");
    assert!(PairCipher::new(&[1; 31], Side::Host).is_err());
    assert!(host.seal(FrameType::Gameplay, &[]).is_none());
    assert!(host.seal(FrameType::Gameplay, &vec![0; 65537]).is_none());

    let host = PairCipher::new(&key, Side::Host).unwrap();
    let joiner = PairCipher::new(&key, Side::Joiner).unwrap();
    let frames: Vec<Vec<u8>> = (0..1100)
        .map(|_| host.seal(FrameType::Gameplay, b"x").unwrap())
        .collect();
    assert!(joiner.open(&frames[1099]).is_some());
    assert!(joiner.open(&frames[499]).is_some());
    assert!(joiner.open(&frames[9]).is_none(), "outside the window");
    assert!(joiner.open(&frames[499]).is_none(), "seen");

    let firsts: HashSet<u64> = (0..8)
        .map(|_| PairCipher::new(&key, Side::Host).unwrap().next_counter())
        .collect();
    assert_eq!(firsts.len(), 8);
    assert!(!firsts.contains(&0));
    let host = PairCipher::new(&key, Side::Host).unwrap();
    assert!(host.next_counter() > 1);
    let frame = host.seal(FrameType::Control, b"hello").unwrap();
    assert!(PairCipher::new(&key, Side::Joiner).unwrap().open(&frame).is_some());
}

#[test]
fn parsing() {
    let stun = parse_stun("stun:stun.cloudflare.com:3478").unwrap();
    assert_eq!((stun.host.as_str(), stun.port), ("stun.cloudflare.com", 3478));
    assert_eq!(
        parse_turn_udp("turn:turn.cloudflare.com:3478?transport=udp")
            .unwrap()
            .port,
        3478
    );
    assert_eq!(parse_turn_udp("turn:turn.cloudflare.com:443").unwrap().port, 443);
    assert!(parse_turn_udp("turn:turn.cloudflare.com:3478?transport=tcp").is_none());
    assert!(parse_turn_udp("turns:turn.cloudflare.com:5349").is_none());
    assert!(parse_stun("stun:bad host:1").is_none());

    let description = "a=ice-ufrag:abcd\r\na=ice-pwd:0123456789abcdefghijkl\r\n\
                       a=candidate:1 1 udp 2130706431 192.168.1.20 50000 typ host\r\n\
                       a=candidate:2 1 udp 16777215 203.0.113.7 60000 typ relay raddr 0.0.0.0 rport 0\r\n";
    let filtered = relay_only(DESCRIPTION, description);
    assert!(filtered.contains("a=ice-ufrag:abcd"));
    assert!(filtered.contains("typ relay"));
    assert!(!filtered.contains("typ host"));
    assert_eq!(
        relay_only(
            CANDIDATE,
            "a=candidate:3 1 udp 1694498815 198.51.100.4 40000 typ srflx raddr 10.0.0.2 rport 40000"
        ),
        ""
    );

    let config = format!(
        "ice_stun_urls=stun:stun.cloudflare.com:3478\n\
         ice_turn_urls=turn:turn.cloudflare.com:3478?transport=udp,turns:turn.cloudflare.com:5349?transport=tcp,\
         turn:turn.cloudflare.com:443?transport=udp\n\
         turn_username=user-1\nturn_credential=credential-1\n\
         peer=member-0,11,{PAIR_KEY}\npeer=member-1,12,{PAIR_KEY}\n"
    );
    let fields = Fields::parse(&config).unwrap();
    let material = ice_material(&fields, &fields).unwrap();
    assert_eq!(material.relays.len(), 2);
    assert_eq!(material.peers.len(), 2);
    assert_eq!(material.stun.unwrap().port, 3478);
    let short = Fields::parse("peer=member-0,11,AAAA").unwrap();
    assert!(ice_material(&short, &short).is_err());
    let duplicate = Fields::parse(&format!("peer=member-0,11,{PAIR_KEY}\npeer=member-0,13,{PAIR_KEY}")).unwrap();
    assert!(ice_material(&duplicate, &duplicate).is_err());
}

#[test]
fn ticket_checks() {
    let signer = Signer::new();
    let dir = tempfile::tempdir().unwrap();
    let key = ticket::load_public_key(&signer.write_key(dir.path())).unwrap();
    let bindings = common::bindings(1, 1000);
    let expected = Expected {
        bindings: &bindings,
        joiner_peer: 2000,
        host_peer: 1000,
    };
    let now = common::now_seconds();
    let none = HashSet::new();
    let good = signer.mint(&bindings, "join-account", 2000, 1000);
    let verified = ticket::verify(&key, &good, expected, &none, now).unwrap();
    assert_eq!(verified.account, "join-account");
    let mut consumed = HashSet::new();
    consumed.insert(verified.jti);
    assert_eq!(ticket::verify(&key, &good, expected, &consumed, now), Err("replay"));
    assert_eq!(ticket::verify(&key, "rpt1.a", expected, &none, now), Err("malformed"));
    let mut bent = good.clone().into_bytes();
    let last = bent.len() - 2;
    bent[last] = if bent[last] == b'A' { b'B' } else { b'A' };
    assert_eq!(
        ticket::verify(&key, std::str::from_utf8(&bent).unwrap(), expected, &none, now),
        Err("bad_signature")
    );
    let expired = signer.mint_at(&bindings, "join-account", 2000, 1000, now - 300, now - 200);
    assert_eq!(ticket::verify(&key, &expired, expected, &none, now), Err("expired"));
    let early = signer.mint_at(&bindings, "join-account", 2000, 1000, now + 100, now + 200);
    assert_eq!(ticket::verify(&key, &early, expected, &none, now), Err("not_yet_valid"));
    let long = signer.mint_at(&bindings, "join-account", 2000, 1000, now, now + 181);
    assert_eq!(ticket::verify(&key, &long, expected, &none, now), Err("bad_lifetime"));
    let wrong_joiner = Expected {
        joiner_peer: 2001,
        ..expected
    };
    assert_eq!(
        ticket::verify(&key, &good, wrong_joiner, &none, now),
        Err("wrong_joiner")
    );
    let mut other_session = bindings.clone();
    other_session.session_id = common::bindings(2, 1000).session_id;
    let wrong_session = Expected {
        bindings: &other_session,
        ..expected
    };
    assert_eq!(
        ticket::verify(&key, &good, wrong_session, &none, now),
        Err("wrong_session")
    );
    let mut other_build = bindings.clone();
    other_build.build = 8;
    let wrong_build = Expected {
        bindings: &other_build,
        ..expected
    };
    assert_eq!(ticket::verify(&key, &good, wrong_build, &none, now), Err("wrong_build"));
}

/// A signature with s above n/2 still verifies (umbrella spec §7 item 5).
#[test]
fn high_s_signatures_pass() {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let signer = Signer::new();
    let dir = tempfile::tempdir().unwrap();
    let key = ticket::load_public_key(&signer.write_key(dir.path())).unwrap();
    let bindings = common::bindings(1, 1000);
    let good = signer.mint(&bindings, "join-account", 2000, 1000);
    let parts: Vec<&str> = good.split('.').collect();
    let signature = p256::ecdsa::Signature::from_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap()).unwrap();
    let low = signature.normalize_s();
    let (r, s) = low.split_scalars();
    let flipped = p256::ecdsa::Signature::from_scalars(r, -s).unwrap();
    assert_ne!(flipped, low);
    let token = format!(
        "{}.{}.{}",
        parts[0],
        parts[1],
        URL_SAFE_NO_PAD.encode(flipped.to_bytes())
    );
    let expected = Expected {
        bindings: &bindings,
        joiner_peer: 2000,
        host_peer: 1000,
    };
    assert!(ticket::verify(&key, &token, expected, &HashSet::new(), common::now_seconds()).is_ok());
}

#[test]
fn lease() {
    let answer = |status: u16, body: &str| {
        classify_lease(Ok(Answer {
            status,
            body: body.to_owned(),
        }))
        .outcome
    };
    assert_eq!(
        answer(200, "{\"renewed\":true,\"state\":\"RUNNING\"}"),
        LeaseOutcome::Renewed
    );
    assert_eq!(answer(200, "{\"renewed\":false}"), LeaseOutcome::Rejected);
    assert_eq!(classify_lease(Err("x".to_owned())).outcome, LeaseOutcome::Unavailable);
    for status in [500, 502, 503, 504, 408, 429] {
        assert_eq!(
            answer(status, "<html>busy</html>"),
            LeaseOutcome::Unavailable,
            "{status}"
        );
    }
    assert_eq!(answer(503, "{\"code\":\"P2P_EMERGENCY_STOP\"}"), LeaseOutcome::Rejected);
    for status in [400, 401, 403, 404, 409, 410] {
        assert_eq!(answer(status, "{}"), LeaseOutcome::Rejected, "{status}");
    }

    let start = Instant::now();
    let at = |s: u64| start + Duration::from_secs(s);
    let mut tracker = LeaseTracker::new(start);
    assert!(!tracker.record(LeaseOutcome::Unavailable, at(2)));
    assert!(!tracker.record(LeaseOutcome::Unavailable, at(4)));
    assert!(!tracker.record(LeaseOutcome::Unavailable, at(6)));
    assert!(!tracker.record(LeaseOutcome::Renewed, at(8)));
    assert!(!tracker.record(LeaseOutcome::Unavailable, at(8 + 29)));
    assert!(tracker.record(LeaseOutcome::Unavailable, at(8 + 30)));

    let mut tracker = LeaseTracker::new(start);
    assert!(!tracker.record(LeaseOutcome::Rejected, at(1)));
    assert!(tracker.record(LeaseOutcome::Rejected, at(2)));
    let mut tracker = LeaseTracker::new(start);
    assert!(!tracker.record(LeaseOutcome::Rejected, at(1)));
    assert!(!tracker.record(LeaseOutcome::Renewed, at(2)));
    assert!(!tracker.record(LeaseOutcome::Rejected, at(3)));
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn lane() {
    let backend = FakeBackend::start();
    let runtime = runtime();
    let client = Arc::new(BackendClient::new(&backend.url, CAPABILITY, "s", runtime.handle().clone()).unwrap());
    let mut lane = Lane::start(client, 3);
    let order: Arc<Mutex<Vec<u32>>> = Arc::default();
    let started = Instant::now();
    for n in 1..=3 {
        let order = order.clone();
        assert!(lane.post(move |_| {
            std::thread::sleep(Duration::from_millis(150));
            order.lock().unwrap().push(n);
        }));
    }
    assert!(started.elapsed() < Duration::from_millis(50));
    assert!(!lane.post(|_| {}), "a fourth job is over capacity");
    lane.drain_and_stop(Duration::from_secs(3));
    assert_eq!(*order.lock().unwrap(), vec![1, 2, 3]);
    assert!(!lane.post(|_| {}), "a stopped lane refuses");
}

fn host_config(bindings: rifts_transport::files::Bindings, maximum_peers: usize, timing: Timing) -> HostConfig {
    HostConfig {
        bindings,
        maximum_peers,
        force_relay: false,
        stun: None,
        relays: Vec::new(),
        timing,
    }
}

fn has(lines: &[String], text: &str) -> usize {
    lines.iter().filter(|l| l.contains(text)).count()
}

#[test]
fn held_signals_and_roster() {
    let signer = Signer::new();
    let key = signer.tickets.verifying;
    let timing = Timing {
        held_signal_ttl: Duration::from_millis(400),
        ..Timing::default()
    };
    let mut host = HostSession::new(host_config(common::bindings(1, 1000), 8, timing), key, None, None, &[]).unwrap();
    let pair = decode_pair_key(PAIR_KEY).unwrap();
    let joiner = Link::start(
        &LinkConfig {
            side: Side::Joiner,
            pair_key: pair.to_vec(),
            stun: None,
            relays: Vec::new(),
            force_relay: false,
        },
        None,
    )
    .unwrap();
    let mut joiner = joiner;
    let description = joiner.take_signals().remove(0).1;

    host.apply_signal("joiner-a", DESCRIPTION, &description);
    host.apply_signal("joiner-a", "GATHERING_DONE", "-");
    host.apply_signal("joiner-a", "GATHERING_DONE", "-");
    host.pump(Instant::now());
    let out = host.take_output();
    assert_eq!(
        has(&out.diagnostics, "ICE_SIGNAL_HELD role=host reason=unknown_peer"),
        1
    );
    assert_eq!(host.counters().signals_rejected, 0);
    assert!(out.signals.is_empty());

    host.add_peer("joiner-a", 3001, pair).unwrap();
    host.pump(Instant::now());
    let out = host.take_output();
    assert_eq!(
        has(&out.diagnostics, "ICE_SIGNAL_REPLAYED role=host peer=3001 count=3"),
        1
    );
    assert!(
        out.signals
            .iter()
            .any(|(to, kind, _)| to == "joiner-a" && kind == DESCRIPTION)
    );

    for _ in 0..20 {
        host.apply_signal("joiner-b", CANDIDATE, "a=candidate:1 1 udp 1 127.0.0.1 9 typ host");
    }
    let out = host.take_output();
    assert_eq!(host.counters().signals_rejected, 4);
    assert_eq!(has(&out.diagnostics, "held_signals_full"), 1);
    std::thread::sleep(Duration::from_millis(500));
    host.pump(Instant::now());
    let out = host.take_output();
    assert_eq!(
        has(
            &out.diagnostics,
            "ICE_SIGNAL_EXPIRED role=host reason=unknown_peer count=16"
        ),
        1
    );
    assert_eq!(host.counters().signals_rejected, 20);
    host.add_peer("joiner-b", 3002, pair).unwrap();
    assert_eq!(has(&host.take_output().diagnostics, "ICE_SIGNAL_REPLAYED"), 0);

    for n in 0..70 {
        host.apply_signal(&format!("acct-{n}"), "GATHERING_DONE", "-");
    }
    assert_eq!(host.counters().signals_rejected, 26);

    assert!(!host.remove_peer("nobody", "left"));
    assert_eq!(host.add_peer("joiner-a", 3999, pair), Err("duplicate_account"));
    assert_eq!(host.add_peer("member-x", 3001, pair), Err("duplicate_peer"));
    assert_eq!(host.add_peer("member-y", 1000, pair), Err("invalid_peer"));
    for n in 0..6 {
        host.add_peer(&format!("member-{n}"), 4000 + n, pair).unwrap();
    }
    assert_eq!(host.add_peer("member-6", 4006, pair), Err("full"));
    host.take_output();
    assert!(host.remove_peer("member-0", "left"));
    let out = host.take_output();
    assert_eq!(
        out.events,
        vec!["PEER_REMOVED account=member-0 reason=left admitted=0".to_owned()]
    );
    host.add_peer("member-0", 4000, pair).unwrap();
}

/// Pumps a host and a joiner session, carrying signals between them.
fn pump_pair(host: &mut HostSession, joiner: &mut JoinSession, host_account: &str, joiner_account: &str) {
    let now = Instant::now();
    host.pump(now);
    joiner.pump(now);
    for (to, kind, data) in host.take_output().signals {
        assert_eq!(to, joiner_account);
        joiner.apply_signal(host_account, &kind, &data);
    }
    for (to, kind, data) in joiner.take_output().signals {
        assert_eq!(to, host_account);
        host.apply_signal(joiner_account, &kind, &data);
    }
}

fn join_config(bindings: &rifts_transport::files::Bindings, account: &str, peer: u64, ticket: String) -> JoinConfig {
    JoinConfig {
        bindings: bindings.clone(),
        local_peer_id: peer,
        local_account_id: account.to_owned(),
        host_account_id: "host-account".to_owned(),
        pair_key: decode_pair_key(PAIR_KEY).unwrap(),
        ticket,
        force_relay: false,
        stun: None,
        relays: Vec::new(),
        admission_deadline: Duration::from_secs(60),
    }
}

#[test]
fn late_consume() {
    let backend = FakeBackend::start();
    backend.set_delay(Duration::from_millis(1500));
    let runtime = runtime();
    let signer = Signer::new();
    let bindings = common::bindings(1, 1000);
    let client =
        Arc::new(BackendClient::new(&backend.url, CAPABILITY, &bindings.session_id, runtime.handle().clone()).unwrap());
    let timing = Timing {
        admission_deadline: Duration::from_secs(1),
        ..Timing::default()
    };
    let lane = Lane::start(client.clone(), 8);
    let pair = decode_pair_key(PAIR_KEY).unwrap();
    let mut host = HostSession::new(
        host_config(bindings.clone(), 4, timing),
        signer.tickets.verifying,
        Some(client),
        Some(lane),
        &[],
    )
    .unwrap();
    host.add_peer("join-account", 2000, pair).unwrap();
    let ticket = signer.mint(&bindings, "join-account", 2000, 1000);
    let mut joiner = JoinSession::new(
        join_config(&bindings, "join-account", 2000, ticket),
        signer.tickets.verifying,
    )
    .unwrap();
    joiner.start(Instant::now()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline && backend.log().disconnects.is_empty() {
        pump_pair(&mut host, &mut joiner, "host-account", "join-account");
        std::thread::sleep(Duration::from_millis(2));
    }
    std::thread::sleep(Duration::from_millis(300));
    let log = backend.log();
    assert_eq!(log.disconnects.len(), 1);
    assert_eq!(log.consumes.len(), 1);
    assert!(!joiner.admitted());
    assert_eq!(host.admitted_count(), 0);
}

#[test]
fn session_admits_once_and_carries_datagrams() {
    let backend = FakeBackend::start();
    let runtime = runtime();
    let signer = Signer::new();
    let bindings = common::bindings(1, 1000);
    let client =
        Arc::new(BackendClient::new(&backend.url, CAPABILITY, &bindings.session_id, runtime.handle().clone()).unwrap());
    let pair = decode_pair_key(PAIR_KEY).unwrap();
    let echo = common::Echo::start();
    let mut host = HostSession::new(
        host_config(bindings.clone(), 4, Timing::default()),
        signer.tickets.verifying,
        Some(client),
        None,
        &[],
    )
    .unwrap();
    host.add_peer("join-account", 2000, pair).unwrap();
    let ticket = signer.mint(&bindings, "join-account", 2000, 1000);
    let mut joiner = JoinSession::new(
        join_config(&bindings, "join-account", 2000, ticket),
        signer.tickets.verifying,
    )
    .unwrap();
    joiner.start(Instant::now()).unwrap();
    let mut remote = HostRemoteBridge::new(echo.port, 4);
    let mut bridge = JoinerBridge::bind(0, 1000).unwrap();
    let game = common::Client::new(bridge.port());

    let deadline = Instant::now() + Duration::from_secs(30);
    while !joiner.admitted() && Instant::now() < deadline {
        pump_pair(&mut host, &mut joiner, "host-account", "join-account");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(joiner.admitted());
    // Requests repeat every 750 ms; the host consumes once.
    let until = Instant::now() + Duration::from_millis(1200);
    let mut echoed = false;
    let payload = vec![7_u8; 900];
    let mut buffer = [0_u8; 2048];
    game.socket.set_nonblocking(true).unwrap();
    let mut next_send = Instant::now();
    while Instant::now() < until || !echoed {
        assert!(Instant::now() < deadline);
        if !echoed && Instant::now() >= next_send {
            next_send = Instant::now() + Duration::from_millis(200);
            game.socket.send_to(&payload, ("127.0.0.1", bridge.port())).unwrap();
        }
        pump_pair(&mut host, &mut joiner, "host-account", "join-account");
        bridge.pump(&mut joiner.network());
        remote.pump(&mut host.network());
        if let Ok((n, _)) = game.socket.recv_from(&mut buffer) {
            echoed |= buffer[..n] == payload[..];
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(host.admitted_count(), 1);
    let log = backend.log();
    assert_eq!(log.consumes.len(), 1);
    assert!(log.consumes[0].body.contains("\"authenticatedSteamId\":\"2000\""));
    assert!(log.consumes[0].body.contains("\"connectionHandle\":\"ice1\""));
    assert_eq!(log.consumes[0].request_id, "adm-1-ice1");
    assert_eq!(log.consumes[0].user_agent, "RusticTransport/1");
    assert_eq!(log.consumes[0].capability, CAPABILITY);
}

#[test]
fn ticket_for_another_account_is_refused() {
    let backend = FakeBackend::start();
    let runtime = runtime();
    let signer = Signer::new();
    let bindings = common::bindings(1, 1000);
    let client =
        Arc::new(BackendClient::new(&backend.url, CAPABILITY, &bindings.session_id, runtime.handle().clone()).unwrap());
    let pair = decode_pair_key(PAIR_KEY).unwrap();
    let mut host = HostSession::new(
        host_config(bindings.clone(), 4, Timing::default()),
        signer.tickets.verifying,
        Some(client),
        None,
        &[],
    )
    .unwrap();
    host.add_peer("join-account", 2000, pair).unwrap();
    let ticket = signer.mint(&bindings, "other-account", 2000, 1000);
    let mut joiner = JoinSession::new(
        join_config(&bindings, "other-account", 2000, ticket),
        signer.tickets.verifying,
    )
    .unwrap();
    joiner.start(Instant::now()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while joiner.failure().is_none() && Instant::now() < deadline {
        let now = Instant::now();
        host.pump(now);
        joiner.pump(now);
        for (_, kind, data) in host.take_output().signals {
            joiner.apply_signal("host-account", &kind, &data);
        }
        // The joiner's launcher speaks for the roster's account.
        for (_, kind, data) in joiner.take_output().signals {
            host.apply_signal("join-account", &kind, &data);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(joiner.failure(), Some("wrong_account"));
    assert!(!joiner.admitted());
    assert!(backend.log().consumes.is_empty());
}

#[test]
fn local_bridge_repins_after_silence() {
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    server.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
    let mut bridge = HostLocalBridge::new(server.local_addr().unwrap().port()).unwrap();
    let first = UdpSocket::bind("127.0.0.1:0").unwrap();
    let second = UdpSocket::bind("127.0.0.1:0").unwrap();
    for socket in [&first, &second] {
        socket.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
    }
    let mut lines = Vec::new();
    let mut buffer = [0_u8; 64];
    let target = ("127.0.0.1", bridge.port());

    first.send_to(b"one", target).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    bridge.pump(&mut lines);
    let (_, first_source) = server.recv_from(&mut buffer).unwrap();
    second.send_to(b"two", target).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    bridge.pump(&mut lines);
    assert_eq!(bridge.counters.unexpected_local_sources_dropped, 1);

    std::thread::sleep(Duration::from_millis(2100));
    second.send_to(b"two", target).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    bridge.pump(&mut lines);
    let (_, second_source) = server.recv_from(&mut buffer).unwrap();
    assert_ne!(first_source, second_source);
    assert!(lines.iter().any(|l| l == "LOCAL_CLIENT_REPINNED role=host path=local"));

    server.send_to(b"reply", second_source).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    bridge.pump(&mut lines);
    let (n, _) = second.recv_from(&mut buffer).unwrap();
    assert_eq!(&buffer[..n], b"reply");
    assert!(first.recv_from(&mut buffer).is_err());
}
