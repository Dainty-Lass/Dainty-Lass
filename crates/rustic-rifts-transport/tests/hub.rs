// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! A realm hub with six joiners and a changing roster (spec §5.3 "hub"), direct and relay-only through a TURN
//! server on 127.0.0.2, so that only relay pairs exist.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::collections::HashSet;
use std::time::{Duration, Instant};

use rustic_rifts_transport::{Handle, Ready, Transport};

use common::backend::FakeBackend;
use common::turn_server::{CREDENTIAL, TurnServer, USERNAME};
use common::{Carrier, Client, Echo, End, Files, HostSpec, JoinSpec, PAIR_KEY, Signer};

const HOST: &str = "hub-account";

struct Hub {
    hosts: Transport,
    joins: Transport,
    hub: Handle,
    signer: Signer,
    bindings: rustic_rifts_transport::files::Bindings,
    relay_lines: Option<(String, String)>,
    carrier: Carrier,
    events: Vec<String>,
    diagnostics: Vec<String>,
    files: Vec<Files>,
}

impl Hub {
    fn pump(&mut self) {
        self.carrier.carry();
        common::drain_events(&self.hosts, self.hub, &mut self.events);
        common::drain_diagnostics(&self.hosts, self.hub, &mut self.diagnostics);
    }

    fn start_join(&mut self, n: u64) -> Handle {
        let files = Files::new();
        let account = format!("member-{n}");
        let (extra, secrets_extra) = self.relay_lines.clone().unwrap_or_default();
        let handle = common::open_join(
            &self.joins,
            &files,
            &self.signer,
            &JoinSpec {
                bindings: &self.bindings,
                local_peer: 3000 + n,
                local_account: &account,
                host_account: HOST,
                extra,
                secrets_extra,
            },
            &self.signer.mint(&self.bindings, &account, 3000 + n, 1000),
        );
        self.files.push(files);
        let host = End {
            transport: self.hosts.clone(),
            handle: self.hub,
            account: HOST.to_owned(),
        };
        let join = End {
            transport: self.joins.clone(),
            handle,
            account,
        };
        self.carrier.pairs.push((host, join));
        handle
    }

    fn drop_join(&mut self, handle: Handle) {
        assert!(self.joins.stop_session(handle));
        self.carrier.pairs.retain(|(_, join)| join.handle != handle);
    }

    fn wait_route(&mut self, handle: Handle) -> String {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            self.pump();
            match self.joins.wait_ready(handle, 5) {
                Ready::Ready(line) => return line,
                Ready::Ended => break,
                Ready::TimedOut => {}
            }
        }
        let mut lines = Vec::new();
        common::drain_diagnostics(&self.joins, handle, &mut lines);
        panic!("join did not get ready: {lines:#?}\nhost: {:#?}", self.diagnostics);
    }

    fn echo(&mut self, route: &str) {
        let client = Client::new(common::field(route, "localPort").unwrap().parse().unwrap());
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            self.pump();
            if client.round_trip(600, Duration::from_millis(200)).is_some() {
                return;
            }
        }
        panic!("no echo through {route}");
    }

    fn count(&self, text: &str) -> usize {
        self.events.iter().filter(|e| e.contains(text)).count()
    }

    fn wait_event(&mut self, text: &str, at_least: usize) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while self.count(text) < at_least && Instant::now() < deadline {
            self.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(self.count(text) >= at_least, "{text}: {:#?}", self.events);
    }
}

fn hub(relay: Option<&TurnServer>) {
    let signer = Signer::new();
    let backend = FakeBackend::start();
    let echo = Echo::start();
    let bindings = common::bindings(if relay.is_some() { 21 } else { 20 }, 1000);
    let relay_lines = relay.map(|server| {
        (
            format!("force_relay=true\nice_turn_urls={}\n", server.url()),
            format!("turn_username={USERNAME}\nturn_credential={CREDENTIAL}\n"),
        )
    });
    let (extra, secrets_extra) = relay_lines.clone().unwrap_or_default();
    let hosts = Transport::new();
    let host_files = Files::new();
    let hub = common::open_host(
        &hosts,
        &host_files,
        &signer,
        &HostSpec {
            bindings: &bindings,
            backend_url: backend.url.clone(),
            server_port: echo.port,
            role: Some("HUB"),
            maximum_peers: 6,
            local_client: Some("0"),
            peers: Vec::new(),
            extra,
            secrets_extra,
        },
        None,
    )
    .unwrap();
    let Ready::Ready(ready) = hosts.wait_ready(hub, 20_000) else {
        let mut lines = Vec::new();
        common::drain_diagnostics(&hosts, hub, &mut lines);
        panic!("{lines:#?}");
    };
    if relay.is_some() {
        assert!(ready.contains("routePolicy=RELAY_ONLY relayReady=1"), "{ready}");
    }
    let mut rig = Hub {
        hosts: hosts.clone(),
        joins: Transport::new(),
        hub,
        signer,
        bindings,
        relay_lines,
        carrier: Carrier { pairs: Vec::new() },
        events: Vec::new(),
        diagnostics: Vec::new(),
        files: vec![host_files],
    };

    // 0. The host's own id is refused by the session; an add and a remove in one burst leave the roster as it was.
    assert!(hosts.add_peer(hub, "self-account", 1000, PAIR_KEY));
    rig.wait_event("PEER_ADD_REJECTED account=self-account reason=invalid_peer", 1);
    assert!(!hosts.remove_peer(hub, "self-account", None));
    assert!(hosts.add_peer(hub, "burst-account", 2999, PAIR_KEY));
    assert!(hosts.remove_peer(hub, "burst-account", Some("left")));
    rig.wait_event("PEER_REMOVED account=burst-account reason=left admitted=0", 1);

    // 1. Six joiners, one after another.
    let mut joins = Vec::new();
    let mut routes = Vec::new();
    for n in 0..6 {
        assert!(hosts.add_peer(hub, &format!("member-{n}"), 3000 + n, PAIR_KEY));
        let handle = rig.start_join(n);
        let route = rig.wait_route(handle);
        rig.echo(&route);
        joins.push(handle);
        routes.push(route);
    }
    for n in 0..6 {
        assert_eq!(rig.count(&format!("PEER_CONNECTED account=member-{n} ")), 1);
        assert_eq!(rig.count(&format!("PEER_ADMITTED account=member-{n} ")), 1);
    }
    if relay.is_some() {
        assert!(rig.count("route=relay") >= 12);
        assert_eq!(rig.count("route=direct"), 0);
        assert!(routes.iter().all(|r| r.contains("routePolicy=RELAY_ONLY route=relay")));
    }
    assert_eq!(echo.source_count(), 6);
    assert_eq!(backend.log().consumes.len(), 6);

    // 2. Full, and duplicates.
    assert!(!hosts.add_peer(hub, "member-9", 3009, PAIR_KEY));

    // 3. A kick.
    let disconnects_before = backend.log().disconnects.len();
    assert!(hosts.remove_peer(hub, "member-3", Some("kicked")));
    rig.wait_event("PEER_REMOVED account=member-3 reason=kicked admitted=1", 1);
    let deadline = Instant::now() + Duration::from_secs(5);
    while backend.log().disconnects.len() < disconnects_before + 1 && Instant::now() < deadline {
        rig.pump();
    }
    assert_eq!(backend.log().disconnects.len(), disconnects_before + 1);
    assert!(!hosts.remove_peer(hub, "member-3", None));
    assert!(!hosts.add_peer(hub, "member-0", 3010, PAIR_KEY), "duplicate account");
    assert!(!hosts.add_peer(hub, "member-10", 3000, PAIR_KEY), "duplicate peer id");
    rig.drop_join(joins[3]);

    // 4. A joiner that starts before its add is held, then replayed.
    let late = rig.start_join(6);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !rig
        .diagnostics
        .iter()
        .any(|l| l == "ICE_SIGNAL_HELD role=host reason=unknown_peer")
        && Instant::now() < deadline
    {
        rig.pump();
    }
    assert!(
        rig.diagnostics
            .iter()
            .any(|l| l == "ICE_SIGNAL_HELD role=host reason=unknown_peer")
    );
    assert_eq!(rig.joins.wait_ready(late, 200), Ready::TimedOut);
    assert!(hosts.add_peer(hub, "member-6", 3006, PAIR_KEY));
    let route = rig.wait_route(late);
    rig.echo(&route);
    assert_eq!(
        rig.diagnostics
            .iter()
            .filter(|l| l.starts_with("ICE_SIGNAL_REPLAYED role=host peer=3006 "))
            .count(),
        1
    );
    assert_eq!(echo.source_count(), 7);

    // 5. A member leaves, is removed, comes back over a fresh link.
    rig.drop_join(joins[2]);
    assert!(hosts.remove_peer(hub, "member-2", Some("left")));
    rig.wait_event("PEER_REMOVED account=member-2", 1);
    assert!(hosts.add_peer(hub, "member-2", 3002, PAIR_KEY));
    let back = rig.start_join(2);
    let route = rig.wait_route(back);
    rig.echo(&route);
    assert_eq!(rig.count("PEER_ADMITTED account=member-2 "), 2);

    // 6. A member restarts without a remove.
    rig.drop_join(joins[5]);
    let restarted = rig.start_join(5);
    let route = rig.wait_route(restarted);
    rig.echo(&route);
    assert_eq!(rig.count("PEER_LOST account=member-5 reason=restarted"), 1);
    assert_eq!(rig.count("PEER_ADMITTED account=member-5 "), 2);

    // 7. Every consume and disconnect is its own.
    let log = backend.log();
    assert_eq!(log.consumes.len(), 9);
    let ids: HashSet<&str> = log.consumes.iter().map(|c| c.request_id.as_str()).collect();
    assert_eq!(ids.len(), 9);
    let handles: HashSet<String> = log
        .consumes
        .iter()
        .map(|c| c.body.split("connectionHandle\":\"").nth(1).unwrap().to_owned())
        .collect();
    assert_eq!(handles.len(), 9);
    assert_eq!(
        log.consumes
            .iter()
            .filter(|c| c.body.contains("\"authenticatedSteamId\":\"3002\""))
            .count(),
        2
    );
    assert!(
        log.consumes
            .iter()
            .all(|c| !c.body.contains("\"authenticatedSteamId\":\"1000\""))
    );
    let disconnect_ids: HashSet<&str> = log.disconnects.iter().map(|c| c.request_id.as_str()).collect();
    assert!(disconnect_ids.len() >= 2);
    assert_eq!(disconnect_ids.len(), log.disconnects.len());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !backend
        .log()
        .leases
        .iter()
        .any(|c| c.body.contains("\"admittedPeerCount\":6"))
        && Instant::now() < deadline
    {
        rig.pump();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        backend
            .log()
            .leases
            .iter()
            .any(|c| c.body.contains("\"admittedPeerCount\":6"))
    );

    // 8. Stop.
    assert!(hosts.stop_session(hub));
    assert_eq!(hosts.last_session_result(hub), 0);
    common::drain_events(&hosts, hub, &mut rig.events);
    assert_eq!(rig.count("reason=host_stopped"), 6);
    assert!(backend.log().disconnects.len() >= disconnects_before + 7);
    rig.joins.shutdown();
}

#[test]
fn hub_direct() {
    hub(None);
}

#[test]
fn hub_relay_local() {
    let server = TurnServer::start();
    hub(Some(&server));
}
