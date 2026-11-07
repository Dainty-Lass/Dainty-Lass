// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Sessions through the `Transport` API (spec §5.3 "Module sessions" and the API checks of "abi" that still apply
//! without the C ABI): handles, files, config rules, the session limit, a hub and a hunt side by side, teardown.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use rustic_rifts_transport::{Handle, Ready, Transport};

use common::backend::FakeBackend;
use common::{Carrier, Client, Echo, End, Files, HostSpec, JoinSpec, PAIR_KEY, Signer};

fn spec<'a>(
    bindings: &'a rustic_rifts_transport::files::Bindings,
    backend: &FakeBackend,
    server_port: u16,
) -> HostSpec<'a> {
    HostSpec {
        bindings,
        backend_url: backend.url.clone(),
        server_port,
        role: None,
        maximum_peers: 4,
        local_client: None,
        peers: Vec::new(),
        extra: String::new(),
        secrets_extra: String::new(),
    }
}

fn diagnostics(transport: &Transport, handle: Handle) -> Vec<String> {
    let mut lines = Vec::new();
    common::drain_diagnostics(transport, handle, &mut lines);
    lines
}

fn events(transport: &Transport, handle: Handle) -> Vec<String> {
    let mut lines = Vec::new();
    common::drain_events(transport, handle, &mut lines);
    lines
}

#[test]
fn api_basics() {
    let transport = Transport::new();
    assert_eq!(transport.abi_version(), 3);
    assert_eq!(transport.peer_id(), 0);
    assert!(!transport.networking_ready());
    transport.initialize().unwrap();
    let id = transport.peer_id();
    assert_ne!(id, 0);
    transport.initialize().unwrap();
    assert_eq!(transport.peer_id(), id);
    assert!(transport.networking_ready());

    let unknown = Handle(999);
    assert_eq!(transport.wait_ready(unknown, 10), Ready::Ended);
    assert!(!transport.session_running(unknown));
    assert!(!transport.stop_session(unknown));
    assert_eq!(transport.last_session_result(unknown), -1);
    assert!(transport.read_signal(unknown).is_none());
    assert!(transport.read_diagnostic(unknown).is_none());
    assert!(transport.read_peer_event(unknown).is_none());
    assert!(!transport.submit_signal(unknown, "acc", "GATHERING_DONE", "-"));
    assert!(!transport.add_peer(unknown, "acc", 5, PAIR_KEY));
    assert!(!transport.remove_peer(unknown, "acc", None));

    let empty = Path::new("");
    let some = Path::new("x");
    assert!(
        transport
            .host_open(empty, some, some, None, some)
            .unwrap_err()
            .contains("missing")
    );
    assert!(
        transport
            .join_open(some, some, empty, some)
            .unwrap_err()
            .contains("missing")
    );

    let files = Files::new();
    let handle = transport
        .host_open(
            &files.path("k"),
            &files.path("c"),
            &files.path("cap"),
            None,
            &files.path("s"),
        )
        .unwrap();
    assert_eq!(transport.wait_ready(handle, 10_000), Ready::Ended);
    assert!(
        diagnostics(&transport, handle)
            .iter()
            .any(|l| l.starts_with("HOST CONFIG FAIL"))
    );
    assert_eq!(transport.last_session_result(handle), 2);
    assert!(transport.stop_session(handle));

    let signer = Signer::new();
    let bindings = common::bindings(1, 1000);
    let ticket = signer.mint(&bindings, "join-account", 2000, 1000);
    let join_files = Files::new();
    let join = common::open_join(
        &transport,
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
        &ticket,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let signal = loop {
        if let Some(signal) = transport.read_signal(join) {
            break signal;
        }
        assert!(Instant::now() < deadline, "{:?}", diagnostics(&transport, join));
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        (signal.to.as_str(), signal.kind.as_str()),
        ("host-account", "DESCRIPTION")
    );
    assert!(transport.session_running(join));
    assert_eq!(transport.last_session_result(join), 0);
    assert!(transport.submit_signal(join, "host-account", "GATHERING_DONE", "-"));
    assert!(!transport.submit_signal(join, "bad account", "GATHERING_DONE", "-"));
    assert!(!transport.submit_signal(join, "host-account", "BOGUS", "-"));
    assert!(!transport.submit_signal(join, "host-account", "CANDIDATE", "a\u{1}b"));
    assert!(!transport.add_peer(join, "member-1", 7, PAIR_KEY));
    assert!(!transport.remove_peer(join, "host-account", None));
    assert!(transport.stop_session(join));
    assert_eq!(transport.last_session_result(join), 0);
    assert!(!transport.session_running(join));

    let again = common::open_join(
        &transport,
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
        &ticket,
    );
    assert!(again.0 > join.0, "handles are never reused");
    transport.shutdown();
    assert!(!transport.session_running(again));
}

/// Opens a host, waits for its end or readiness, then stops it.
fn outcome(
    transport: &Transport,
    signer: &Signer,
    spec: &HostSpec<'_>,
    ticket: Option<&str>,
) -> Result<String, String> {
    let files = Files::new();
    let handle = common::open_host(transport, &files, signer, spec, ticket)?;
    let ready = transport.wait_ready(handle, 20_000);
    let lines = diagnostics(transport, handle);
    transport.stop_session(handle);
    match ready {
        Ready::Ready(line) => Ok(line),
        _ => Err(lines.join("\n")),
    }
}

#[test]
fn config_rules() {
    let transport = Transport::new();
    let signer = Signer::new();
    let backend = FakeBackend::start();
    let bindings = common::bindings(1, 1000);
    let ticket = signer.mint(&bindings, "host-account", 1000, 1000);

    let hub = |peers: usize| HostSpec {
        role: Some("HUB"),
        local_client: Some("0"),
        maximum_peers: peers,
        ..spec(&bindings, &backend, 9)
    };
    let line = outcome(&transport, &signer, &hub(64), None).unwrap();
    assert!(line.starts_with("HOST_READY session="), "{line}");
    assert!(line.contains("localPort=0 "));
    assert!(line.contains("routePolicy=AUTOMATIC relayReady=0 peers=0"));
    assert!(
        outcome(&transport, &signer, &hub(65), None)
            .unwrap_err()
            .contains("HOST CONFIG FAIL")
    );
    assert!(
        outcome(&transport, &signer, &hub(4), Some(&ticket)).is_ok(),
        "a hub ignores a ticket"
    );

    let hunt = |peers: usize, local: Option<&'static str>, role: Option<&'static str>| HostSpec {
        role,
        local_client: local,
        maximum_peers: peers,
        ..spec(&bindings, &backend, 9)
    };
    let line = outcome(&transport, &signer, &hunt(4, None, Some("HUNT")), Some(&ticket)).unwrap();
    assert_ne!(common::field(&line, "localPort").unwrap(), "0");
    assert!(outcome(&transport, &signer, &hunt(5, None, Some("HUNT")), Some(&ticket)).is_err());
    assert!(outcome(&transport, &signer, &hunt(5, Some("0"), Some("HUNT")), None).is_err());
    assert!(outcome(&transport, &signer, &hunt(4, Some("0"), Some("HUNT")), None).is_ok());
    assert!(
        outcome(&transport, &signer, &hunt(4, None, Some("BOGUS")), Some(&ticket))
            .unwrap_err()
            .contains("HOST CONFIG FAIL: invalid session_role")
    );
    assert!(
        outcome(&transport, &signer, &hunt(4, Some("2"), None), Some(&ticket))
            .unwrap_err()
            .contains("HOST CONFIG FAIL: invalid host_local_client")
    );
    assert!(
        outcome(&transport, &signer, &hunt(4, None, None), None)
            .unwrap_err()
            .contains("a host ticket is required unless host_local_client=0")
    );
    let ticket_for_hunt = signer.mint(&bindings, "host-account", 1000, 1000);
    assert!(outcome(&transport, &signer, &hunt(4, None, None), Some(&ticket_for_hunt)).is_ok());
    assert!(outcome(&transport, &signer, &hunt(5, None, None), Some(&ticket_for_hunt)).is_err());

    let log = backend.log();
    assert!(log.host_ready.iter().all(|c| c.request_id == "ready-1"));
    assert!(log.consumes.iter().all(|c| c.request_id == "host-local-1"));
    assert!(
        log.consumes
            .iter()
            .all(|c| c.body.contains("\"connectionHandle\":\"localhost\""))
    );
}

#[test]
fn at_most_eight_sessions() {
    let transport = Transport::new();
    let signer = Signer::new();
    let backend = FakeBackend::start();
    let bindings = common::bindings(1, 1000);
    let spec = HostSpec {
        role: Some("HUB"),
        local_client: Some("0"),
        ..spec(&bindings, &backend, 9)
    };
    let files: Vec<Files> = (0..9).map(|_| Files::new()).collect();
    let handles: Vec<Handle> = files[..8]
        .iter()
        .map(|f| common::open_host(&transport, f, &signer, &spec, None).unwrap())
        .collect();
    for handle in &handles {
        assert!(matches!(transport.wait_ready(*handle, 20_000), Ready::Ready(_)));
    }
    let refused = common::open_host(&transport, &files[8], &signer, &spec, None).unwrap_err();
    assert!(refused.contains("At most 8"), "{refused}");
    assert!(transport.stop_session(handles[0]));
    assert!(
        transport.read_diagnostic(handles[0]).is_some(),
        "an ended session stays readable"
    );
    let ninth = common::open_host(&transport, &files[8], &signer, &spec, None).unwrap();
    assert!(transport.session_running(ninth));
    transport.shutdown();
}

#[test]
fn a_hub_and_a_hunt_side_by_side() {
    let hosts = Transport::new();
    let joins = Transport::new();
    let signer = Signer::new();
    let hub_backend = FakeBackend::start();
    let hunt_backend = FakeBackend::start();
    let hub_echo = Echo::start();
    let hunt_echo = Echo::start();
    let hub_bindings = common::bindings(1, 1000);
    let hunt_bindings = common::bindings(2, 1001);

    let hub_files = Files::new();
    let hub = common::open_host(
        &hosts,
        &hub_files,
        &signer,
        &HostSpec {
            role: Some("HUB"),
            local_client: Some("0"),
            maximum_peers: 6,
            ..spec(&hub_bindings, &hub_backend, hub_echo.port)
        },
        None,
    )
    .unwrap();
    let hunt_files = Files::new();
    let hunt_ticket = signer.mint(&hunt_bindings, "host-account", 1001, 1001);
    let hunt = common::open_host(
        &hosts,
        &hunt_files,
        &signer,
        &HostSpec {
            role: Some("HUNT"),
            peers: vec![("join-account".to_owned(), 2000)],
            ..spec(&hunt_bindings, &hunt_backend, hunt_echo.port)
        },
        Some(&hunt_ticket),
    )
    .unwrap();
    assert_ne!(hub, hunt);
    let Ready::Ready(hub_line) = hosts.wait_ready(hub, 20_000) else {
        panic!("{:?}", diagnostics(&hosts, hub));
    };
    let Ready::Ready(hunt_line) = hosts.wait_ready(hunt, 20_000) else {
        panic!("{:?}", diagnostics(&hosts, hunt));
    };
    assert!(hub_line.contains("localPort=0 "));
    let hunt_local = common::field(&hunt_line, "localPort").unwrap();
    assert_ne!(hunt_local, "0");
    assert_eq!(hub_backend.log().host_ready.len(), 1);
    assert_eq!(hunt_backend.log().host_ready.len(), 1);
    assert!(hosts.add_peer(hub, "join-account", 2000, PAIR_KEY));
    assert!(
        !hosts.add_peer(hunt, "join-account", 2000, PAIR_KEY),
        "already on the roster"
    );
    assert!(!hosts.add_peer(hub, "bad account!", 2001, PAIR_KEY));

    let join_spec = |bindings, host_account| JoinSpec {
        bindings,
        local_peer: 2000,
        local_account: "join-account",
        host_account,
        extra: String::new(),
        secrets_extra: String::new(),
    };
    let hub_join_files = Files::new();
    let hub_join = common::open_join(
        &joins,
        &hub_join_files,
        &signer,
        &join_spec(&hub_bindings, "hub-account"),
        &signer.mint(&hub_bindings, "join-account", 2000, 1000),
    );
    let hunt_join_files = Files::new();
    let hunt_join = common::open_join(
        &joins,
        &hunt_join_files,
        &signer,
        &join_spec(&hunt_bindings, "host-account"),
        &signer.mint(&hunt_bindings, "join-account", 2000, 1001),
    );
    let end = |transport: &Transport, handle, account: &str| End {
        transport: transport.clone(),
        handle,
        account: account.to_owned(),
    };
    let carrier = Carrier {
        pairs: vec![
            (end(&hosts, hub, "hub-account"), end(&joins, hub_join, "join-account")),
            (
                end(&hosts, hunt, "host-account"),
                end(&joins, hunt_join, "join-account"),
            ),
        ],
    };
    let mut hub_route = None;
    let mut hunt_route = None;
    let deadline = Instant::now() + Duration::from_secs(40);
    while (hub_route.is_none() || hunt_route.is_none()) && Instant::now() < deadline {
        carrier.carry();
        if let Ready::Ready(line) = joins.wait_ready(hub_join, 5) {
            hub_route = Some(line);
        }
        if let Ready::Ready(line) = joins.wait_ready(hunt_join, 5) {
            hunt_route = Some(line);
        }
    }
    let hub_route = hub_route.unwrap_or_else(|| panic!("{:?}", diagnostics(&joins, hub_join)));
    let hunt_route = hunt_route.unwrap_or_else(|| panic!("{:?}", diagnostics(&joins, hunt_join)));
    assert!(hub_route.starts_with("ROUTE_READY localHost=127.0.0.1 localPort="));
    assert!(hub_route.contains(&format!("session={}", hub_bindings.session_id)));
    assert!(hunt_route.contains(&format!("session={}", hunt_bindings.session_id)));

    let round_trips = |route: &str| {
        let client = Client::new(common::field(route, "localPort").unwrap().parse().unwrap());
        for size in [700, 900] {
            let start = Instant::now();
            let mut done = None;
            while done.is_none() && start.elapsed() < Duration::from_secs(10) {
                carrier.carry();
                done = client.round_trip(size, Duration::from_millis(300));
            }
            assert!(done.is_some(), "{size} bytes did not come back");
        }
    };
    round_trips(&hub_route);
    round_trips(&hunt_route);
    assert_eq!(hub_echo.source_count(), 1);
    assert_eq!(hunt_echo.source_count(), 1);

    let hub_events = events(&hosts, hub);
    let hunt_events = events(&hosts, hunt);
    assert_eq!(
        hub_events
            .iter()
            .filter(|e| e.starts_with("PEER_ADMITTED account=join-account"))
            .count(),
        1
    );
    assert_eq!(
        hunt_events
            .iter()
            .filter(|e| e.starts_with("PEER_ADMITTED account=join-account"))
            .count(),
        1
    );
    assert_eq!(hub_backend.log().consumes.len(), 1);
    let hunt_consumes = hunt_backend.log().consumes;
    assert_eq!(hunt_consumes.len(), 2);
    assert!(
        hunt_consumes
            .iter()
            .any(|c| c.body.contains("\"connectionHandle\":\"localhost\"")
                && c.body.contains("\"authenticatedSteamId\":\"1001\""))
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !hub_backend
        .log()
        .leases
        .iter()
        .any(|c| c.body.contains("\"admittedPeerCount\":1"))
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        hub_backend
            .log()
            .leases
            .iter()
            .any(|c| c.body.contains("\"admittedPeerCount\":1"))
    );

    assert!(!joins.add_peer(hub_join, "member-1", 9, PAIR_KEY));
    assert!(!hosts.remove_peer(hub, "nobody", None));
    assert!(!hosts.remove_peer(Handle(999), "join-account", None));

    assert!(joins.stop_session(hub_join));
    assert!(joins.stop_session(hunt_join));
    assert!(hosts.stop_session(hub));
    assert!(hosts.stop_session(hunt));
    assert_eq!(hosts.last_session_result(hub), 0);
    assert_eq!(hosts.last_session_result(hunt), 0);
    assert!(
        events(&hosts, hub)
            .iter()
            .any(|e| e == "PEER_LOST account=join-account reason=host_stopped")
    );
    assert!(
        events(&hosts, hunt)
            .iter()
            .any(|e| e == "PEER_LOST account=join-account reason=host_stopped")
    );
    assert_eq!(
        hub_backend.log().disconnects.len() + hunt_backend.log().disconnects.len(),
        2
    );
}
