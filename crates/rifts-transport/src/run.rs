// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The host and join runs: what a session thread does from reading its files to its last disconnect notice.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::backend::{BackendClient, Lane, LeaseAnswer, LeaseOutcome, LeaseTracker};
use crate::bridge::{BridgeCounters, HostLocalBridge, HostRemoteBridge, JoinerBridge};
use crate::files::{self, CAPABILITY_LIMIT, CONFIG_LIMIT, Fields, IceMaterial, Reader, TICKET_LIMIT};
use crate::guard::{HUNT_ABORTED, Publisher, PublisherConfig};
use crate::host::{HostConfig, HostSession, Output, SessionCounters, Timing};
use crate::join::{JoinConfig, JoinSession};
use crate::pipe::GuardPipe;
use crate::session::{SessionIo, Signal, result};
use crate::ticket::{self, Expected};

const STATS_EVERY: Duration = Duration::from_secs(2);
const LEASE_EVERY: Duration = Duration::from_secs(2);
const LEASE_LANE: usize = 2;
const WORK_LANE: usize = 256;
const DRAIN_LIMIT: Duration = Duration::from_secs(3);
const REMOTE_PEERS_MAX_HUB: u64 = 64;
const REMOTE_PEERS_MAX_HUNT: u64 = 4;

#[derive(Debug, Clone)]
pub struct HostPaths {
    pub public_key: PathBuf,
    pub config: PathBuf,
    pub capability: PathBuf,
    pub host_ticket: Option<PathBuf>,
    pub secrets: PathBuf,
}

#[derive(Debug, Clone)]
pub struct JoinPaths {
    pub public_key: PathBuf,
    pub config: PathBuf,
    pub ticket: PathBuf,
    pub secrets: PathBuf,
}

fn route_policy(force_relay: bool) -> &'static str {
    if force_relay { "RELAY_ONLY" } else { "AUTOMATIC" }
}

fn wall_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn stats_line(role: &str, path: &str, bridge: &BridgeCounters, session: &SessionCounters, guard: (u64, u64)) -> String {
    format!(
        "UDP_STATS role={role} path={path} {} admitted={} rejected={} disconnected={} signalsRejected={} \
         guardPublishFailures={} guardEvidenceLoss={}",
        bridge.stats_fields(),
        session.admitted,
        session.admission_rejected,
        session.disconnected,
        session.signals_rejected,
        guard.0,
        guard.1,
    )
}

fn read_pair(
    config: &Path,
    secrets: &Path,
    allow_empty_secrets: bool,
) -> Result<(Fields, Fields, IceMaterial), String> {
    let config = Fields::parse(&files::read_input(config, CONFIG_LIMIT, false)?)?;
    let secrets = Fields::parse(&files::read_input(secrets, CONFIG_LIMIT, allow_empty_secrets)?)?;
    let material = files::ice_material(&config, &secrets)?;
    Ok((config, secrets, material))
}

struct Guard<'a> {
    io: SessionIo<'a>,
    publisher: Option<Publisher>,
    evidence: bool,
    fenced_warned: bool,
}

impl Guard<'_> {
    /// The session's own Guard calls (spec §3.3.6).
    fn publish(&self, kind: &str, actor: &str, payload: &str) -> Result<(), &'static str> {
        if !self.evidence {
            return Ok(());
        }
        let Some(publisher) = &self.publisher else {
            return Err("no Guard event publisher");
        };
        if publisher.enqueue(kind, actor, payload) {
            Ok(())
        } else {
            Err("the Guard event queue did not take the event")
        }
    }

    fn publish_or_warn(&self, kind: &str, actor: &str, payload: &str) {
        if let Err(error) = self.publish(kind, actor, payload) {
            self.io.diagnostic(&format!("HUNT GUARD OBSERVE WARN: {error}"));
        }
    }

    fn counters(&self) -> (u64, u64) {
        self.publisher.as_ref().map_or((0, 0), |p| {
            let stats = p.stats();
            (stats.publish_failures, stats.evidence_loss)
        })
    }
}

fn route_output(io: SessionIo<'_>, output: Output, guard: &Guard<'_>, remote: Option<&mut HostRemoteBridge>) {
    for line in output.diagnostics {
        io.diagnostic(&line);
    }
    for line in output.events {
        io.event(&line);
    }
    for (to, kind, data) in output.signals {
        io.signal(Signal { to, kind, data });
    }
    for (kind, actor, payload) in output.guard {
        guard.publish_or_warn(&kind, &actor, &payload);
    }
    if let Some(remote) = remote {
        for peer in output.disconnected {
            remote.disconnect(peer);
        }
    }
}

/// The host run (spec §2.5); returns the result code.
pub fn host(io: SessionIo<'_>, paths: &HostPaths, runtime: Option<&tokio::runtime::Handle>, timing: Timing) -> i32 {
    let fail = |line: String, code: i32| {
        io.diagnostic(&line);
        code
    };
    let (config, _secrets, material) = match read_pair(&paths.config, &paths.secrets, true) {
        Ok(read) => read,
        Err(error) => return fail(format!("HOST CONFIG FAIL: {error}"), result::BAD_INPUT),
    };
    let hub = match config.get("session_role").unwrap_or("HUNT") {
        "HUNT" => false,
        "HUB" => true,
        _ => return fail("HOST CONFIG FAIL: invalid session_role".to_owned(), result::BAD_INPUT),
    };
    let local_client = match config.get("host_local_client").unwrap_or("1") {
        "1" => true,
        "0" => false,
        _ => {
            return fail(
                "HOST CONFIG FAIL: invalid host_local_client".to_owned(),
                result::BAD_INPUT,
            );
        }
    };

    let mut reader = Reader::new(&config);
    let bindings = files::read_bindings(&mut reader);
    let backend_url = reader.text("backend_url", 512, b":/._-");
    let server_port = reader.number("server_udp_port", 1, 65535) as u16;
    let max = if hub {
        REMOTE_PEERS_MAX_HUB
    } else {
        REMOTE_PEERS_MAX_HUNT
    };
    let maximum_peers = reader.number("maximum_peers", 1, max) as usize;
    let pipe_required = !hub || config.has("guard_pipe_name") || config.has("expected_server_pid");
    let (pipe_name, server_pid) = if pipe_required {
        (
            reader.text("guard_pipe_name", 96, b"._-"),
            reader.number("expected_server_pid", 1, u64::from(u32::MAX)) as u32,
        )
    } else {
        (String::new(), 0)
    };
    let evidence = if hub {
        files::boolean(&config, "hunt_evidence")
    } else {
        config.get("hunt_evidence").is_none_or(|v| v == "true")
    };
    let force_relay = files::boolean(&config, "force_relay");
    let capability = files::read_input(&paths.capability, CAPABILITY_LIMIT, false).unwrap_or_else(|error| {
        reader.fail(error);
        String::new()
    });
    let mut host_ticket = String::new();
    if local_client {
        match paths.host_ticket.as_ref().filter(|p| !p.as_os_str().is_empty()) {
            None => reader.fail("a host ticket is required unless host_local_client=0".to_owned()),
            Some(path) => match files::read_input(path, TICKET_LIMIT, false) {
                Ok(text) => host_ticket = text,
                Err(error) => reader.fail(error),
            },
        }
    }
    if let Err(error) = reader.finish() {
        return fail(format!("HOST CONFIG FAIL: {error}"), result::BAD_INPUT);
    }
    if material.peers.len() > maximum_peers {
        return fail(
            "HOST CONFIG FAIL: more transport peers than maximum_peers".to_owned(),
            result::BAD_INPUT,
        );
    }
    if force_relay && !material.peers.is_empty() && material.relays.is_empty() {
        io.failure("host:relay_unavailable");
        return result::SETUP;
    }
    let key = match ticket::load_public_key(&paths.public_key) {
        Ok(key) => key,
        Err(error) => return fail(format!("HOST KEY FAIL: {error}"), result::SETUP),
    };
    let Some(runtime) = runtime else {
        return fail(
            "HOST BACKEND FAIL: admission backend connection setup failed".to_owned(),
            result::SETUP,
        );
    };
    let client = match BackendClient::new(&backend_url, &capability, &bindings.session_id, runtime.clone()) {
        Ok(client) => Arc::new(client),
        Err(error) => return fail(format!("HOST BACKEND FAIL: {error}"), result::SETUP),
    };
    drop(capability);
    let mut lease_lane = Lane::start(client.clone(), LEASE_LANE);
    let work_lane = Lane::start(client.clone(), WORK_LANE);
    let sink_shared: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink_lines = sink_shared.clone();
    let publisher = evidence.then(|| {
        Publisher::start(
            client.clone(),
            bindings.generation,
            PublisherConfig::default(),
            Arc::new(move |line| {
                if let Ok(mut lines) = sink_lines.lock() {
                    lines.push(line);
                }
            }),
        )
    });
    let guard = Guard {
        io,
        publisher,
        evidence,
        fenced_warned: false,
    };
    let mut guard = guard;
    let flush_sink = |io: SessionIo<'_>| {
        if let Ok(mut lines) = sink_shared.lock() {
            for line in lines.drain(..) {
                io.diagnostic(&line);
            }
        }
    };

    let host_config = HostConfig {
        bindings: bindings.clone(),
        maximum_peers,
        force_relay,
        stun: material.stun.clone(),
        relays: material.relays.clone(),
        timing,
    };
    let mut session = match HostSession::new(host_config, key, Some(client.clone()), None, &material.peers) {
        Ok(session) => session,
        Err(_) => {
            return fail(
                "HOST SESSION FAIL: invalid session configuration".to_owned(),
                result::SETUP,
            );
        }
    };
    let pipe = if pipe_required {
        match GuardPipe::create(&pipe_name, server_pid, runtime) {
            Ok(pipe) => Some(pipe),
            Err(error) => return fail(format!("HOST GUARD PIPE FAIL: {error}"), result::SETUP),
        }
    } else {
        None
    };
    let mut remote = HostRemoteBridge::new(server_port, maximum_peers);
    let generation = bindings.generation;
    if let Err(error) = client.host_ready(
        bindings.virtual_port,
        &bindings.bridge_instance_id,
        &format!("ready-{generation}"),
    ) {
        return fail(format!("HOST BACKEND FAIL: {error}"), result::SETUP);
    }
    let mut host_account = String::new();
    if local_client {
        let expected = Expected {
            bindings: &bindings,
            joiner_peer: bindings.host_peer_id,
            host_peer: bindings.host_peer_id,
        };
        let verified = match ticket::verify(&key, &host_ticket, expected, &Default::default(), wall_seconds()) {
            Ok(verified) => verified,
            Err(code) => {
                io.failure(&format!("host:{code}"));
                return result::REFUSED_ADMISSION;
            }
        };
        let answer = client.consume(
            &host_ticket,
            bindings.host_peer_id,
            &bindings.bridge_instance_id,
            "localhost",
            &format!("host-local-{generation}"),
        );
        host_ticket = String::new();
        if answer.code != "ok" {
            let code = if answer.code.is_empty() {
                "central_consume_failed"
            } else {
                &answer.code
            };
            io.failure(&format!("host:{code}"));
            return result::REFUSED_ADMISSION;
        }
        io.diagnostic(&format!(
            "ADMISSION_ACCEPTED role=host peer={} source=local",
            bindings.host_peer_id
        ));
        host_account = verified.account;
    }
    drop(host_ticket);
    session.set_lane(Some(work_lane));
    if let Err(error) = guard.publish("SESSION_STARTED", &host_account, "{\"source\":\"ice_transport\"}") {
        io.diagnostic(&format!("HUNT GUARD OBSERVE WARN: {error}"));
    } else if let Err(error) = guard.publish("WORLD_READY", &host_account, "{\"source\":\"launcher_ready_pipe\"}") {
        io.diagnostic(&format!("HUNT GUARD OBSERVE WARN: {error}"));
    }
    let mut local = if local_client {
        match HostLocalBridge::new(server_port) {
            Ok(bridge) => Some(bridge),
            Err(_) => {
                io.failure("exception");
                return result::SETUP;
            }
        }
    } else {
        None
    };
    io.report_roster(maximum_peers, &session.roster());
    io.diagnostic(&format!(
        "ICE_READY role=host stun={} turn={} relayOnly={} peers={}",
        u8::from(material.stun.is_some()),
        material.relays.len(),
        u8::from(force_relay),
        material.peers.len()
    ));
    io.set_ready(format!(
        "HOST_READY session={} generation={generation} transport=ice localHost=127.0.0.1 localPort={} routePolicy={} \
         relayReady={} peers={}",
        bindings.session_id,
        local.as_ref().map_or(0, HostLocalBridge::port),
        route_policy(force_relay),
        u8::from(!material.relays.is_empty()),
        material.peers.len()
    ));

    let started = Instant::now();
    let mut tracker = LeaseTracker::new(started);
    let lease_mailbox: Arc<Mutex<Option<LeaseAnswer>>> = Arc::default();
    let mut lease_in_flight = false;
    let mut lease_waiting_warned = false;
    let mut lease_sequence = 0_u64;
    let mut next_lease = started;
    let mut next_stats = started;
    let mut lease_failed = false;

    while !io.stop_requested() {
        let now = Instant::now();
        for command in io.take_commands() {
            match command {
                crate::session::RosterCommand::Add {
                    account,
                    peer_id,
                    pair_key,
                } => {
                    if let Err(code) = session.add_peer(&account, peer_id, pair_key) {
                        io.diagnostic(&format!("ROSTER_ADD_REJECTED reason={code}"));
                        io.event(&format!("PEER_ADD_REJECTED account={account} reason={code}"));
                        if code != "duplicate_account" {
                            io.forget_peer(&account);
                        }
                    }
                }
                crate::session::RosterCommand::Remove { account, reason } => {
                    if !session.remove_peer(&account, &reason) {
                        io.diagnostic("ROSTER_REMOVE_IGNORED reason=unknown_account");
                    }
                }
            }
            let output = session.take_output();
            route_output(io, output, &guard, Some(&mut remote));
        }
        for signal in io.take_inbox() {
            session.apply_signal(&signal.from, &signal.kind, &signal.data);
        }
        session.pump(now);
        let output = session.take_output();
        route_output(io, output, &guard, Some(&mut remote));
        if let Some(pipe) = &pipe {
            let (events, dropped) = pipe.take();
            let mut taken = true;
            if dropped > 0 {
                taken &= guard
                    .publish(
                        "EVIDENCE_LOSS",
                        "",
                        &format!("{{\"droppedEvents\":{dropped},\"reason\":\"bridge_queue_overflow\"}}"),
                    )
                    .is_ok();
            }
            let mut last_error = None;
            for event in events {
                if let Err(error) = guard.publish(&event.kind, &event.actor, &event.payload) {
                    taken = false;
                    last_error = Some(error);
                }
            }
            if !taken && !guard.fenced_warned {
                guard.fenced_warned = true;
                let error = last_error.unwrap_or("the Guard event queue did not take the event");
                io.diagnostic(&format!(
                    "HUNT GUARD OBSERVE WARN: {error} (completion fenced until this hunt ends)"
                ));
            }
        }
        remote.pump(&mut session.network());
        if let Some(local) = local.as_mut() {
            let mut lines = Vec::new();
            local.pump(&mut lines);
            for line in lines {
                io.diagnostic(&line);
            }
        }
        flush_sink(io);
        if now >= next_stats {
            next_stats = now + STATS_EVERY;
            let counters = session.counters();
            if let Some(local) = &local {
                io.diagnostic(&stats_line(
                    "host",
                    "local",
                    &local.counters,
                    &counters,
                    guard.counters(),
                ));
            }
            io.diagnostic(&stats_line(
                "host",
                "remote",
                &remote.counters,
                &counters,
                guard.counters(),
            ));
        }

        let answer = lease_mailbox.lock().ok().and_then(|mut a| a.take());
        if let Some(answer) = answer {
            lease_in_flight = false;
            lease_waiting_warned = false;
            let failed = tracker.record(answer.outcome, now);
            let since = tracker.seconds_since_renewal(now);
            if failed {
                if answer.outcome == LeaseOutcome::Rejected {
                    io.diagnostic(&format!(
                        "HOST LEASE FAIL: the backend refused the lease ({})",
                        answer.error
                    ));
                } else {
                    io.diagnostic(&format!(
                        "HOST LEASE FAIL: central authority unavailable for {since} s ({})",
                        answer.error
                    ));
                }
                io.failure("host:lease_failed");
                lease_failed = true;
                break;
            }
            match answer.outcome {
                LeaseOutcome::Renewed => {}
                LeaseOutcome::Rejected => io.diagnostic(&format!("HOST LEASE WARN: {}", answer.error)),
                LeaseOutcome::Unavailable => io.diagnostic(&format!(
                    "HOST LEASE WARN: {} (waiting; last renewal {since} s ago)",
                    answer.error
                )),
            }
        }
        if now >= next_lease {
            next_lease = now + LEASE_EVERY;
            if lease_in_flight {
                let failed = tracker.record(LeaseOutcome::Unavailable, now);
                let since = tracker.seconds_since_renewal(now);
                if failed {
                    io.diagnostic(&format!(
                        "HOST LEASE FAIL: central authority unavailable for {since} s (no answer to the renewal)"
                    ));
                    io.failure("host:lease_failed");
                    lease_failed = true;
                    break;
                }
                if !lease_waiting_warned {
                    lease_waiting_warned = true;
                    io.diagnostic(&format!(
                        "HOST LEASE WARN: the last renewal has not been answered yet (last renewal {since} s ago)"
                    ));
                }
            } else {
                lease_sequence += 1;
                let sequence = lease_sequence;
                let admitted = u32::try_from(session.admitted_count()).unwrap_or(u32::MAX);
                let mailbox = lease_mailbox.clone();
                let request_id = format!("lease-{generation}-{sequence}");
                lease_in_flight = lease_lane.post(move |client| {
                    let answer = client.lease(sequence, admitted, 0, &request_id);
                    if let Ok(mut slot) = mailbox.lock() {
                        *slot = Some(answer);
                    }
                });
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    session.stop();
    let output = session.take_output();
    route_output(io, output, &guard, Some(&mut remote));
    if let Some(publisher) = guard.publisher.as_ref()
        && !publisher.stats().completed
    {
        let reason = if lease_failed { "LEASE_FAILED" } else { "HOST_STOPPED" };
        guard.publish_or_warn(HUNT_ABORTED, &host_account, &format!("{{\"reason\":\"{reason}\"}}"));
    }
    if let Some(publisher) = guard.publisher.as_mut() {
        publisher.flush_and_stop(DRAIN_LIMIT);
    }
    flush_sink(io);
    if let Some(mut lane) = session.take_lane() {
        lane.drain_and_stop(DRAIN_LIMIT);
    }
    lease_lane.stop();
    session.finish_in_place();
    if lease_failed {
        result::LEASE_FAILED
    } else {
        result::CLEAN
    }
}

/// The join run (spec §2.6); returns the result code.
pub fn join(io: SessionIo<'_>, paths: &JoinPaths, timing: Timing) -> i32 {
    let fail = |line: String, code: i32| {
        io.diagnostic(&line);
        code
    };
    let (config, _secrets, material) = match read_pair(&paths.config, &paths.secrets, false) {
        Ok(read) => read,
        Err(error) => return fail(format!("JOIN CONFIG FAIL: {error}"), result::BAD_INPUT),
    };
    let mut reader = Reader::new(&config);
    let bindings = files::read_bindings(&mut reader);
    let local_port = reader.number("local_udp_port", 0, 65535) as u16;
    let local_peer_id = reader.number("local_peer_id", 1, u64::MAX);
    let local_account = reader.text("local_account_id", 64, b"-");
    let host_account = reader.text("host_account_id", 64, b"-");
    let force_relay = files::boolean(&config, "force_relay");
    let ticket_text = files::read_input(&paths.ticket, TICKET_LIMIT, false).unwrap_or_else(|error| {
        reader.fail(error);
        String::new()
    });
    if let Err(error) = reader.finish() {
        return fail(format!("JOIN CONFIG FAIL: {error}"), result::BAD_INPUT);
    }
    let host_entry = match material.peers.as_slice() {
        [only] if only.account == host_account && only.peer_id == bindings.host_peer_id => only.clone(),
        _ => {
            return fail(
                "JOIN CONFIG FAIL: the transport secrets must name exactly the host".to_owned(),
                result::BAD_INPUT,
            );
        }
    };
    if force_relay && material.relays.is_empty() {
        io.failure("join:relay_unavailable");
        return result::SETUP;
    }
    let key = match ticket::load_public_key(&paths.public_key) {
        Ok(key) => key,
        Err(error) => return fail(format!("JOIN KEY FAIL: {error}"), result::SETUP),
    };
    let generation = bindings.generation;
    let session_id = bindings.session_id.clone();
    let host_peer = bindings.host_peer_id;
    let join_config = JoinConfig {
        bindings,
        local_peer_id,
        local_account_id: local_account,
        host_account_id: host_account,
        pair_key: host_entry.pair_key,
        ticket: ticket_text,
        force_relay,
        stun: material.stun.clone(),
        relays: material.relays.clone(),
        admission_deadline: timing.admission_deadline,
    };
    let mut session = match JoinSession::new(join_config, key) {
        Ok(session) => session,
        Err(_) => {
            return fail(
                "JOIN SESSION FAIL: invalid session configuration".to_owned(),
                result::SETUP,
            );
        }
    };
    if let Err(error) = session.start(Instant::now()) {
        io.diagnostic(&format!("JOIN SESSION FAIL: {error}"));
        io.failure("join:transport_start_failed");
        return result::SETUP;
    }
    let route_out = |session: &mut JoinSession| {
        let output = session.take_output();
        for line in output.diagnostics {
            io.diagnostic(&line);
        }
        for line in output.events {
            io.event(&line);
        }
        for (to, kind, data) in output.signals {
            io.signal(Signal { to, kind, data });
        }
    };
    io.diagnostic(&format!(
        "ICE_READY role=join stun={} turn={} relayOnly={} peers=1",
        u8::from(material.stun.is_some()),
        material.relays.len(),
        u8::from(force_relay)
    ));
    route_out(&mut session);
    let mut bridge = match JoinerBridge::bind(local_port, host_peer) {
        Ok(bridge) => bridge,
        Err(_) => {
            io.failure("exception");
            return result::SETUP;
        }
    };
    let finish = |session: &mut JoinSession, code: i32| {
        session.stop();
        code
    };
    loop {
        if io.stop_requested() {
            route_out(&mut session);
            return finish(&mut session, result::CLEAN);
        }
        for signal in io.take_inbox() {
            session.apply_signal(&signal.from, &signal.kind, &signal.data);
        }
        session.pump(Instant::now());
        route_out(&mut session);
        if let Some(code) = session.failure() {
            io.failure(&format!("join:{code}"));
            return finish(&mut session, result::REFUSED_ADMISSION);
        }
        if session.admitted() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let route = session.route().map_or("direct", crate::link::Route::as_str);
    io.set_ready(format!(
        "ROUTE_READY localHost=127.0.0.1 localPort={} session={session_id} generation={generation} transport=ice \
         routePolicy={} route={route}",
        bridge.port(),
        route_policy(force_relay)
    ));
    let mut next_stats = Instant::now();
    while !io.stop_requested() {
        for signal in io.take_inbox() {
            session.apply_signal(&signal.from, &signal.kind, &signal.data);
        }
        let now = Instant::now();
        session.pump(now);
        bridge.pump(&mut session.network());
        route_out(&mut session);
        if let Some(code) = session.failure() {
            io.failure(&format!("join:{code}"));
            return finish(&mut session, result::LEASE_FAILED);
        }
        if now >= next_stats {
            next_stats = now + STATS_EVERY;
            io.diagnostic(&stats_line(
                "join",
                "remote",
                &bridge.counters,
                &session.counters(),
                (0, 0),
            ));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    finish(&mut session, result::CLEAN)
}
