// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The host session: its roster of joiners, one link per joiner, admission over the link and the backend consume.
//! The pair key proves who is on a link; the ticket proves the backend let that account in, once.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use p256::ecdsa::VerifyingKey;
use rustic_rifts_admission::control::{self, AdmissionResult};

use crate::backend::{BackendClient, ConsumeAnswer, Lane};
use crate::files::{Bindings, PeerEntry};
use crate::filter::DESCRIPTION;
use crate::link::{Link, LinkConfig, LinkState, RelayServer, Route, candidate_type_name};
use crate::router::{MessageTransport, Router};
use crate::seal::{FrameType, Side};
use crate::text::{is_account, truncate};
use crate::ticket::{self, Expected};
use crate::url::ServerAddr;

const MAX_HELD_ACCOUNTS: usize = 64;
const MAX_HELD_PER_ACCOUNT: usize = 16;
const FRAMES_PER_PUMP: usize = 64;
const REJECT_LINGER: Duration = Duration::from_secs(1);
const GAMEPLAY_PAYLOAD: &str = "{\"transport\":\"ice\"}";

/// The host session's timeouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// How long a joiner's signal is held while its link is not open.
    pub held_signal_ttl: Duration,
    /// How long a connected joiner has to be admitted.
    pub admission_deadline: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            held_signal_ttl: Duration::from_secs(20),
            admission_deadline: Duration::from_secs(60),
        }
    }
}

/// `HOST SESSION FAIL` / `JOIN SESSION FAIL: invalid session configuration`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidSession;

/// What a session counted, reported in its stats lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionCounters {
    /// Links opened to joiners.
    pub connection_attempts: u64,
    /// Joiners admitted.
    pub admitted: u64,
    /// Admission requests refused.
    pub admission_rejected: u64,
    /// Joiners that connected but never asked to be admitted in time.
    pub authentication_timeouts: u64,
    /// Control frames that were not an admission request.
    pub malformed_control: u64,
    /// Gameplay frames dropped from a joiner not yet admitted.
    pub pre_admission_gameplay_dropped: u64,
    /// Admitted joiners whose link closed.
    pub disconnected: u64,
    /// Signals refused: unknown, malformed or over a holding limit.
    pub signals_rejected: u64,
}

/// What a pump produced for the run to route: lines, signals for the launcher, Guard events, freed bridge slots.
#[derive(Debug, Default)]
pub struct Output {
    /// Diagnostic lines.
    pub diagnostics: Vec<String>,
    /// Peer event lines for the launcher.
    pub events: Vec<String>,
    /// Signals for the launcher, as `(account, kind, data)`.
    pub signals: Vec<(String, String, String)>,
    /// Guard events, as `(type, actor account, payload JSON)`.
    pub guard: Vec<(String, String, String)>,
    /// Peer ids whose bridge slot is free again.
    pub disconnected: Vec<u64>,
}

/// What a host session is set up with.
#[derive(Debug, Clone)]
pub struct HostConfig {
    /// The values the session is bound to.
    pub bindings: Bindings,
    /// The most joiners at once.
    pub maximum_peers: usize,
    /// Use TURN relays only.
    pub force_relay: bool,
    /// The STUN server, if one is set.
    pub stun: Option<ServerAddr>,
    /// The TURN relays.
    pub relays: Vec<RelayServer>,
    /// The session's timeouts.
    pub timing: Timing,
}

struct Slot {
    link: Link,
    index: u64,
    connected: bool,
    deadline: Instant,
    route: Route,
    admitted: bool,
    admission_id: String,
    result_frame: Vec<u8>,
    closing_at: Option<Instant>,
    consume_token: Option<u64>,
}

struct Peer {
    account: String,
    peer_id: u64,
    pair_key: [u8; 32],
    slot: Option<Slot>,
}

struct Held {
    signals: Vec<(Instant, String, String)>,
    full_reported: bool,
}

struct Pending {
    account: String,
    link_index: u64,
    jti: [u8; 16],
}

struct Notice {
    admission_id: String,
    request_id: Option<String>,
    peer_id: u64,
    link_index: u64,
}

type Mailbox = Arc<Mutex<Vec<(u64, ConsumeAnswer)>>>;

/// The host's side of a hunt: a link per joiner, admission over it and the backend consume.
pub struct HostSession {
    config: HostConfig,
    key: VerifyingKey,
    backend: Option<Arc<BackendClient>>,
    lane: Option<Lane>,
    peers: Vec<Peer>,
    held: BTreeMap<String, Held>,
    next_link: u64,
    next_token: u64,
    router: Router,
    consumed: HashSet<[u8; 16]>,
    pending: HashMap<u64, Pending>,
    answers: Mailbox,
    notices: VecDeque<Notice>,
    counters: SessionCounters,
    out: Output,
}

fn wall_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl HostSession {
    /// Refused (`HOST SESSION FAIL`) without a host peer id or virtual port, or with a bad or excess starting peer.
    pub fn new(
        config: HostConfig,
        key: VerifyingKey,
        backend: Option<Arc<BackendClient>>,
        lane: Option<Lane>,
        peers: &[PeerEntry],
    ) -> Result<Self, InvalidSession> {
        if config.bindings.host_peer_id == 0 || config.bindings.virtual_port == 0 || peers.len() > config.maximum_peers
        {
            return Err(InvalidSession);
        }
        let mut session = Self {
            config,
            key,
            backend,
            lane,
            peers: Vec::new(),
            held: BTreeMap::new(),
            next_link: 1,
            next_token: 1,
            router: Router::default(),
            consumed: HashSet::new(),
            pending: HashMap::new(),
            answers: Mailbox::default(),
            notices: VecDeque::new(),
            counters: SessionCounters::default(),
            out: Output::default(),
        };
        for peer in peers {
            session
                .validate(&peer.account, peer.peer_id)
                .map_err(|_| InvalidSession)?;
            session.peers.push(Peer {
                account: peer.account.clone(),
                peer_id: peer.peer_id,
                pair_key: peer.pair_key,
                slot: None,
            });
        }
        Ok(session)
    }

    /// What the pumps produced since the last take.
    pub fn take_output(&mut self) -> Output {
        std::mem::take(&mut self.out)
    }

    /// The counters so far.
    pub fn counters(&self) -> SessionCounters {
        self.counters
    }

    /// How many joiners are admitted now.
    pub fn admitted_count(&self) -> usize {
        self.peers
            .iter()
            .filter(|p| p.slot.as_ref().is_some_and(|s| s.admitted))
            .count()
    }

    /// Every rostered joiner as `(account, peer id)`.
    pub fn roster(&self) -> Vec<(String, u64)> {
        self.peers.iter().map(|p| (p.account.clone(), p.peer_id)).collect()
    }

    /// The most joiners at once.
    pub fn limit(&self) -> usize {
        self.config.maximum_peers
    }

    fn diag(&mut self, line: String) {
        self.out.diagnostics.push(line);
    }

    fn validate(&self, account: &str, peer_id: u64) -> Result<(), &'static str> {
        if !is_account(account) || peer_id == 0 || peer_id == self.config.bindings.host_peer_id {
            return Err("invalid_peer");
        }
        if self.peers.iter().any(|p| p.account == account) {
            return Err("duplicate_account");
        }
        if self.peers.iter().any(|p| p.peer_id == peer_id) {
            return Err("duplicate_peer");
        }
        Ok(())
    }

    /// Adds a joiner to the roster; refused as `invalid_peer`, `duplicate_account`, `duplicate_peer` or `full`.
    pub fn add_peer(&mut self, account: &str, peer_id: u64, pair_key: [u8; 32]) -> Result<(), &'static str> {
        self.validate(account, peer_id)?;
        if self.peers.len() >= self.config.maximum_peers {
            return Err("full");
        }
        self.peers.push(Peer {
            account: account.to_owned(),
            peer_id,
            pair_key,
            slot: None,
        });
        self.diag(format!("ROSTER_ADD peer={peer_id} roster={}", self.peers.len()));
        self.replay_held(account, peer_id);
        Ok(())
    }

    fn replay_held(&mut self, account: &str, peer_id: u64) {
        let Some(held) = self.held.remove(account) else {
            return;
        };
        let now = Instant::now();
        let ttl = self.config.timing.held_signal_ttl;
        let mut replayed = 0;
        for (at, kind, data) in held.signals {
            if now.saturating_duration_since(at) >= ttl {
                self.counters.signals_rejected += 1;
                continue;
            }
            replayed += 1;
            self.apply_signal(account, &kind, &data);
        }
        if replayed > 0 {
            self.diag(format!("ICE_SIGNAL_REPLAYED role=host peer={peer_id} count={replayed}"));
        }
    }

    /// `false` for an account not on the roster.
    pub fn remove_peer(&mut self, account: &str, reason: &str) -> bool {
        self.held.remove(account);
        let Some(index) = self.peers.iter().position(|p| p.account == account) else {
            return false;
        };
        let token: String = truncate(reason, 48)
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"_-.".contains(&b) {
                    char::from(b)
                } else {
                    '_'
                }
            })
            .collect();
        let token = if token.is_empty() { "removed".to_owned() } else { token };
        let admitted = self.peers[index].slot.as_ref().is_some_and(|s| s.admitted);
        self.close(index, true, &token);
        let mut peer = self.peers.remove(index);
        peer.pair_key = [0; 32];
        self.out.events.push(format!(
            "PEER_REMOVED account={account} reason={token} admitted={}",
            u8::from(admitted)
        ));
        self.diag(format!(
            "ROSTER_REMOVE peer={} roster={}",
            peer.peer_id,
            self.peers.len()
        ));
        true
    }

    /// A signal from the launcher, already checked for kind and size.
    pub fn apply_signal(&mut self, from: &str, kind: &str, data: &str) {
        let Some(index) = self.peers.iter().position(|p| p.account == from) else {
            self.hold(from, kind, data);
            return;
        };
        if kind == DESCRIPTION {
            if self.peers[index].slot.is_some() {
                // The joiner started over.
                self.close(index, true, "restarted");
            }
            self.open_link(index, data);
            return;
        }
        let peer_id = self.peers[index].peer_id;
        let Some(slot) = self.peers[index].slot.as_mut() else {
            self.counters.signals_rejected += 1;
            return;
        };
        if let Err(error) = slot.link.apply_signal(kind, data) {
            self.counters.signals_rejected += 1;
            self.diag(format!("ICE_SIGNAL_REJECTED role=host peer={peer_id} reason={error}"));
        }
    }

    fn hold(&mut self, from: &str, kind: &str, data: &str) {
        if !self.held.contains_key(from) && self.held.len() >= MAX_HELD_ACCOUNTS {
            self.counters.signals_rejected += 1;
            self.diag("ICE_SIGNAL_REJECTED role=host reason=unknown_peer".to_owned());
            return;
        }
        let entry = self.held.entry(from.to_owned()).or_insert_with(|| Held {
            signals: Vec::new(),
            full_reported: false,
        });
        if entry.signals.len() >= MAX_HELD_PER_ACCOUNT {
            self.counters.signals_rejected += 1;
            if !entry.full_reported {
                entry.full_reported = true;
                self.out
                    .diagnostics
                    .push("ICE_SIGNAL_REJECTED role=host reason=held_signals_full".to_owned());
            }
            return;
        }
        if entry.signals.is_empty() {
            self.out
                .diagnostics
                .push("ICE_SIGNAL_HELD role=host reason=unknown_peer".to_owned());
        }
        entry.signals.push((Instant::now(), kind.to_owned(), data.to_owned()));
    }

    fn expire_held(&mut self, now: Instant) {
        let ttl = self.config.timing.held_signal_ttl;
        let mut lines = Vec::new();
        for held in self.held.values_mut() {
            let before = held.signals.len();
            held.signals
                .retain(|(at, _, _)| now.saturating_duration_since(*at) < ttl);
            let expired = before - held.signals.len();
            if expired > 0 {
                self.counters.signals_rejected += expired as u64;
                lines.push(format!(
                    "ICE_SIGNAL_EXPIRED role=host reason=unknown_peer count={expired}"
                ));
            }
        }
        self.held.retain(|_, h| !h.signals.is_empty());
        self.out.diagnostics.extend(lines);
    }

    fn open_link(&mut self, index: usize, description: &str) {
        self.counters.connection_attempts += 1;
        let peer = &self.peers[index];
        let config = LinkConfig {
            side: Side::Host,
            pair_key: peer.pair_key.to_vec(),
            stun: self.config.stun.clone(),
            relays: self.config.relays.clone(),
            force_relay: self.config.force_relay,
        };
        match Link::start(&config, Some(description)) {
            Ok(link) => {
                let link_index = self.next_link;
                self.next_link += 1;
                self.peers[index].slot = Some(Slot {
                    link,
                    index: link_index,
                    connected: false,
                    deadline: Instant::now(),
                    route: Route::Direct,
                    admitted: false,
                    admission_id: String::new(),
                    result_frame: Vec::new(),
                    closing_at: None,
                    consume_token: None,
                });
            }
            Err(error) => {
                self.counters.signals_rejected += 1;
                let peer_id = self.peers[index].peer_id;
                self.diag(format!("ICE_LINK_FAILED role=host peer={peer_id} reason={error}"));
            }
        }
    }

    fn close(&mut self, index: usize, tell_backend: bool, reason: &str) {
        let peer = &mut self.peers[index];
        let Some(mut slot) = peer.slot.take() else {
            return;
        };
        if slot.admitted {
            self.router.detach(peer.peer_id);
            self.out.disconnected.push(peer.peer_id);
            if tell_backend && !slot.admission_id.is_empty() {
                self.notices.push_back(Notice {
                    admission_id: slot.admission_id.clone(),
                    request_id: None,
                    peer_id: peer.peer_id,
                    link_index: slot.index,
                });
            }
            self.counters.disconnected += 1;
            self.out.guard.push((
                "PLAYER_LOST".to_owned(),
                peer.account.clone(),
                GAMEPLAY_PAYLOAD.to_owned(),
            ));
            self.out.events.push(format!(
                "PEER_LOST account={} reason={}",
                peer.account,
                truncate(reason, 48)
            ));
        }
        slot.link.stop();
    }

    fn reject(&mut self, index: usize, code: &str) {
        let peer_id = self.peers[index].peer_id;
        let Some(slot) = self.peers[index].slot.as_mut() else {
            return;
        };
        let result = AdmissionResult {
            admitted: false,
            code: code.as_bytes().to_vec(),
            admission_id: Vec::new(),
        };
        if let Some(frame) = control::encode_admission_result(&result) {
            slot.link.send(FrameType::Control, &frame);
        }
        // The result gets a moment to leave before the link goes.
        slot.closing_at = Some(Instant::now() + REJECT_LINGER);
        self.counters.admission_rejected += 1;
        self.diag(format!(
            "ADMISSION_REJECTED role=host peer={peer_id} code={}",
            truncate(code, 64)
        ));
    }

    fn admit(&mut self, index: usize, ticket_text: &[u8]) {
        let peer_id = self.peers[index].peer_id;
        let Some(slot) = self.peers[index].slot.as_ref() else {
            return;
        };
        if self.config.force_relay && slot.link.route() != Some(Route::Relay) {
            let (local, remote) = slot.link.selected_types().map_or(("unknown", "unknown"), |(l, r)| {
                (candidate_type_name(l), candidate_type_name(r))
            });
            self.diag(format!(
                "ICE_ROUTE_REFUSED role=host peer={peer_id} local={local} remote={remote}"
            ));
            self.reject(index, "transport_not_relayed");
            return;
        }
        let link_index = slot.index;
        let text = String::from_utf8_lossy(ticket_text).into_owned();
        let expected = Expected {
            bindings: &self.config.bindings,
            joiner_peer: peer_id,
            host_peer: self.config.bindings.host_peer_id,
        };
        let verified = match ticket::verify(&self.key, &text, expected, &self.consumed, wall_seconds()) {
            Ok(verified) => verified,
            Err(code) => {
                self.reject(index, code);
                return;
            }
        };
        if verified.account != self.peers[index].account {
            self.reject(index, "wrong_account");
            return;
        }
        let generation = self.config.bindings.generation;
        let handle = format!("ice{link_index}");
        let request_id = format!("adm-{generation}-ice{link_index}");
        let bridge = self.config.bindings.bridge_instance_id.clone();
        let Some(backend) = self.backend.clone() else {
            self.reject(index, "central_consume_failed");
            return;
        };
        if let Some(lane) = &self.lane {
            let token = self.next_token;
            self.next_token += 1;
            let mailbox = self.answers.clone();
            let posted = lane.post(move |client| {
                let answer = client.consume(&text, peer_id, &bridge, &handle, &request_id);
                if let Ok(mut answers) = mailbox.lock() {
                    answers.push((token, answer));
                }
            });
            if !posted {
                self.reject(index, "central_consume_busy");
                return;
            }
            self.pending.insert(
                token,
                Pending {
                    account: self.peers[index].account.clone(),
                    link_index,
                    jti: verified.jti,
                },
            );
            if let Some(slot) = self.peers[index].slot.as_mut() {
                slot.consume_token = Some(token);
            }
        } else {
            let answer = backend.consume(&text, peer_id, &bridge, &handle, &request_id);
            self.complete(index, &answer, verified.jti);
        }
    }

    fn complete(&mut self, index: usize, answer: &ConsumeAnswer, jti: [u8; 16]) {
        if answer.code != "ok" || answer.admission_id.len() != 64 {
            let code = if answer.code.is_empty() || answer.code == "ok" {
                "central_consume_failed"
            } else {
                answer.code.as_str()
            };
            self.reject(index, code);
            return;
        }
        let generation = self.config.bindings.generation;
        let peer = &self.peers[index];
        let (account, peer_id) = (peer.account.clone(), peer.peer_id);
        let Some(link_index) = peer.slot.as_ref().map(|s| s.index) else {
            return;
        };
        if self.consumed.contains(&jti) {
            self.notices.push_back(Notice {
                admission_id: answer.admission_id.clone(),
                request_id: Some(format!("disc-replay-{generation}-ice{link_index}")),
                peer_id,
                link_index,
            });
            self.reject(index, "local_replay");
            return;
        }
        self.consumed.insert(jti);
        let result = AdmissionResult {
            admitted: true,
            code: b"ok".to_vec(),
            admission_id: answer.admission_id.as_bytes().to_vec(),
        };
        let frame = control::encode_admission_result(&result).unwrap_or_default();
        self.router.attach(peer_id);
        let Some(slot) = self.peers[index].slot.as_mut() else {
            return;
        };
        slot.admitted = true;
        slot.admission_id = answer.admission_id.clone();
        slot.result_frame = frame.clone();
        slot.link.send(FrameType::Control, &frame);
        let route = slot.route.as_str();
        self.counters.admitted += 1;
        self.diag(format!(
            "ADMISSION_ACCEPTED role=host peer={peer_id} source=ice route={route}"
        ));
        self.out
            .events
            .push(format!("PEER_ADMITTED account={account} route={route}"));
        self.out
            .guard
            .push(("PLAYER_ADMITTED".to_owned(), account, GAMEPLAY_PAYLOAD.to_owned()));
    }

    fn settle(&mut self) {
        let answers: Vec<(u64, ConsumeAnswer)> = self
            .answers
            .lock()
            .map(|mut a| a.drain(..).collect())
            .unwrap_or_default();
        for (token, answer) in answers {
            let Some(pending) = self.pending.remove(&token) else {
                continue;
            };
            let live = self.peers.iter().position(|p| {
                p.account == pending.account
                    && p.slot.as_ref().is_some_and(|s| {
                        s.index == pending.link_index && s.consume_token == Some(token) && s.closing_at.is_none()
                    })
            });
            if let Some(index) = live {
                if let Some(slot) = self.peers[index].slot.as_mut() {
                    slot.consume_token = None;
                }
                self.complete(index, &answer, pending.jti);
            } else if answer.code == "ok" && answer.admission_id.len() == 64 {
                // The link went while the backend admitted it: undo that admission.
                self.consumed.insert(pending.jti);
                let peer_id = self
                    .peers
                    .iter()
                    .find(|p| p.account == pending.account)
                    .map_or(0, |p| p.peer_id);
                self.notices.push_back(Notice {
                    admission_id: answer.admission_id,
                    request_id: None,
                    peer_id,
                    link_index: pending.link_index,
                });
            }
        }
    }

    fn flush_notices(&mut self) {
        let generation = self.config.bindings.generation;
        while let Some(notice) = self.notices.front() {
            let request_id = notice
                .request_id
                .clone()
                .unwrap_or_else(|| format!("disc-{generation}-{}-{}", notice.peer_id, notice.link_index));
            let admission_id = notice.admission_id.clone();
            if let Some(lane) = &self.lane {
                if !lane.post(move |client| client.disconnect(&admission_id, &request_id)) {
                    return;
                }
            } else if let Some(backend) = &self.backend {
                backend.disconnect(&admission_id, &request_id);
            }
            self.notices.pop_front();
        }
    }

    /// Settles consumes that came back and sends what is left in place (shutdown, after the lane drained).
    pub fn finish_in_place(&mut self) {
        self.settle();
        self.lane = None;
        self.flush_notices();
    }

    /// Takes the backend lane, so it can be drained off the session.
    pub fn take_lane(&mut self) -> Option<Lane> {
        self.lane.take()
    }

    /// Puts a backend lane back, or none.
    pub fn set_lane(&mut self, lane: Option<Lane>) {
        self.lane = lane;
    }

    /// Moves every link, admits or rejects joiners, and closes failed or timed-out ones.
    pub fn pump(&mut self, now: Instant) {
        self.settle();
        self.expire_held(now);
        let deadline_after = self.config.timing.admission_deadline;
        for index in 0..self.peers.len() {
            let (account, peer_id) = (self.peers[index].account.clone(), self.peers[index].peer_id);
            let Some(slot) = self.peers[index].slot.as_mut() else {
                continue;
            };
            slot.link.pump(now);
            for (kind, data) in slot.link.take_signals() {
                self.out.signals.push((account.clone(), kind, data));
            }
            if let Some(at) = slot.closing_at {
                if now >= at {
                    self.close(index, false, "rejected");
                }
                continue;
            }
            let state = slot.link.state();
            if state == LinkState::Connected && !slot.connected {
                slot.connected = true;
                slot.deadline = now + deadline_after;
                slot.route = slot.link.route().unwrap_or(Route::Direct);
                let route = slot.route.as_str();
                self.out
                    .diagnostics
                    .push(format!("ICE_ROUTE role=host peer={peer_id} route={route}"));
                self.out
                    .events
                    .push(format!("PEER_CONNECTED account={account} route={route}"));
            }
            if state == LinkState::Failed {
                let reason = if slot.connected { "link_lost" } else { "connect_failed" };
                self.close(index, true, reason);
                continue;
            }
            if !slot.connected {
                continue;
            }
            let frames = slot.link.receive(FRAMES_PER_PUMP);
            for (kind, message) in frames {
                let Some(slot) = self.peers[index].slot.as_mut() else {
                    break;
                };
                match kind {
                    FrameType::Gameplay if slot.admitted => self.router.deliver(peer_id, message),
                    FrameType::Gameplay => self.counters.pre_admission_gameplay_dropped += 1,
                    _ => {
                        let Some(ticket) = control::decode_admission_request(&message) else {
                            self.counters.malformed_control += 1;
                            continue;
                        };
                        if slot.admitted {
                            let frame = slot.result_frame.clone();
                            slot.link.send(FrameType::Control, &frame);
                        } else if slot.closing_at.is_none() && slot.consume_token.is_none() {
                            let ticket = ticket.to_vec();
                            self.admit(index, &ticket);
                        }
                    }
                }
            }
            if let Some(slot) = self.peers[index].slot.as_ref()
                && !slot.admitted
                && slot.closing_at.is_none()
                && now > slot.deadline
            {
                self.counters.authentication_timeouts += 1;
                self.reject(index, "admission_request_timeout");
            }
        }
        self.flush_notices();
    }

    /// Closes every link, telling the backend, and flushes the notices.
    pub fn stop(&mut self) {
        for index in 0..self.peers.len() {
            self.close(index, true, "host_stopped");
        }
        self.held.clear();
        self.flush_notices();
    }

    /// The remote bridge's view of the admitted joiners.
    pub fn network(&mut self) -> HostNetwork<'_> {
        HostNetwork { session: self }
    }
}

/// The remote bridge's view: send to an admitted peer's link, read what admitted peers sent.
pub struct HostNetwork<'a> {
    session: &'a mut HostSession,
}

impl MessageTransport for HostNetwork<'_> {
    fn send(&mut self, peer: u64, payload: &[u8]) -> bool {
        if payload.is_empty() || !self.session.router.is_attached(peer) {
            return false;
        }
        self.session
            .peers
            .iter_mut()
            .find(|p| p.peer_id == peer)
            .and_then(|p| p.slot.as_mut())
            .is_some_and(|slot| slot.admitted && slot.link.send(FrameType::Gameplay, payload))
    }

    fn receive(&mut self, max: usize) -> Vec<(u64, Vec<u8>)> {
        self.session.router.read(max)
    }
}

impl Drop for HostSession {
    fn drop(&mut self) {
        for peer in &mut self.peers {
            peer.pair_key = [0; 32];
        }
    }
}
