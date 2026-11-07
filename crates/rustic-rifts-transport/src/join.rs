// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The joiner session: one link to the host, which it opens as the controlling side; once connected it sends its
//! ticket every 750 ms until the host answers.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use p256::ecdsa::VerifyingKey;
use rustic_rifts_admission::control;

use crate::files::Bindings;
use crate::filter::DESCRIPTION;
use crate::host::{InvalidSession, Output, SessionCounters};
use crate::link::{Link, LinkConfig, LinkState, RelayServer, Route};
use crate::router::{MessageTransport, Router};
use crate::seal::{FrameType, Side};
use crate::ticket::{self, Expected};
use crate::url::ServerAddr;

const FRAMES_PER_PUMP: usize = 64;
const RESEND: Duration = Duration::from_millis(750);

/// What a joiner session is set up with.
#[derive(Debug, Clone)]
pub struct JoinConfig {
    /// The values the session is bound to; the ticket must name the same.
    pub bindings: Bindings,
    /// This joiner's peer id.
    pub local_peer_id: u64,
    /// This joiner's account; the ticket must belong to it.
    pub local_account_id: String,
    /// The host's account; signals from anyone else are refused.
    pub host_account_id: String,
    /// The key sealing frames with the host; never printed.
    pub pair_key: [u8; 32],
    /// The admission ticket, sent until the host answers and then forgotten.
    pub ticket: String,
    /// Use TURN relays only; a direct route fails the session.
    pub force_relay: bool,
    /// The STUN server, if one is set.
    pub stun: Option<ServerAddr>,
    /// The TURN relays.
    pub relays: Vec<RelayServer>,
    /// How long the joiner waits to connect and be admitted.
    pub admission_deadline: Duration,
}

/// The joiner's side of a hunt: one link to the host and admission over it.
pub struct JoinSession {
    config: JoinConfig,
    key: VerifyingKey,
    link: Option<Link>,
    connected: bool,
    route: Route,
    deadline: Instant,
    next_request: Option<Instant>,
    admitted: bool,
    failure: Option<String>,
    router: Router,
    counters: SessionCounters,
    out: Output,
}

fn wall_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl JoinSession {
    /// Refused (`JOIN SESSION FAIL`) when an id is missing or the host's and the joiner's peer ids are the same.
    pub fn new(config: JoinConfig, key: VerifyingKey) -> Result<Self, InvalidSession> {
        let host_peer = config.bindings.host_peer_id;
        let valid = !config.host_account_id.is_empty()
            && host_peer != 0
            && config.local_peer_id != 0
            && config.local_peer_id != host_peer
            && !config.local_account_id.is_empty()
            && !config.ticket.is_empty()
            && config.bindings.virtual_port != 0;
        if !valid {
            return Err(InvalidSession);
        }
        Ok(Self {
            config,
            key,
            link: None,
            connected: false,
            route: Route::Direct,
            deadline: Instant::now(),
            next_request: None,
            admitted: false,
            failure: None,
            router: Router::default(),
            counters: SessionCounters::default(),
            out: Output::default(),
        })
    }

    /// Checks the joiner's own ticket, then opens the link; its description is the first signal out.
    pub fn start(&mut self, now: Instant) -> Result<(), String> {
        let expected = Expected {
            bindings: &self.config.bindings,
            joiner_peer: self.config.local_peer_id,
            host_peer: self.config.bindings.host_peer_id,
        };
        let verified = ticket::verify(
            &self.key,
            &self.config.ticket,
            expected,
            &HashSet::new(),
            wall_seconds(),
        )
        .map_err(|code| format!("join ticket verification failed: {code}"))?;
        if verified.account != self.config.local_account_id {
            return Err("join ticket belongs to another account".to_owned());
        }
        let link = Link::start(
            &LinkConfig {
                side: Side::Joiner,
                pair_key: self.config.pair_key.to_vec(),
                stun: self.config.stun.clone(),
                relays: self.config.relays.clone(),
                force_relay: self.config.force_relay,
            },
            None,
        )?;
        self.link = Some(link);
        self.counters.connection_attempts += 1;
        self.deadline = now + self.config.admission_deadline;
        self.forward_signals();
        Ok(())
    }

    /// What the pumps produced since the last take.
    pub fn take_output(&mut self) -> Output {
        std::mem::take(&mut self.out)
    }

    /// The counters so far.
    pub fn counters(&self) -> SessionCounters {
        self.counters
    }

    /// Whether the host admitted this joiner.
    pub fn admitted(&self) -> bool {
        self.admitted
    }

    /// Why the session failed, such as `admission_response_timeout`; `None` while it has not.
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// The link's route once connected.
    pub fn route(&self) -> Option<Route> {
        self.link.as_ref().and_then(Link::route)
    }

    fn host(&self) -> String {
        self.config.host_account_id.clone()
    }

    fn fail(&mut self, code: &str) {
        if self.failure.is_some() {
            return;
        }
        self.failure = Some(code.to_owned());
        self.counters.admission_rejected += 1;
        self.out.diagnostics.push(format!(
            "ADMISSION_REJECTED role=join peer={} code={code}",
            self.config.bindings.host_peer_id
        ));
    }

    fn forward_signals(&mut self) {
        let host = self.host();
        if let Some(link) = self.link.as_mut() {
            for (kind, data) in link.take_signals() {
                self.out.signals.push((host.clone(), kind, data));
            }
        }
    }

    /// Hands a signal from `from` to the link; anything not from the host is refused.
    /// A second description means the host replaced its link, which fails a joiner not yet admitted.
    pub fn apply_signal(&mut self, from: &str, kind: &str, data: &str) {
        let host = self.host();
        let Some(link) = self.link.as_mut().filter(|_| from == host) else {
            self.counters.signals_rejected += 1;
            return;
        };
        if kind == DESCRIPTION && link.has_remote_description() {
            // The host replaced its link.
            if !self.admitted {
                self.fail("transport_restarted");
            }
            return;
        }
        if let Err(error) = link.apply_signal(kind, data) {
            self.counters.signals_rejected += 1;
            self.out
                .diagnostics
                .push(format!("ICE_SIGNAL_REJECTED role=join reason={error}"));
        }
    }

    /// Moves the link, sends the ticket while waiting and reads the host's answer; fails the session on a timeout or refusal.
    pub fn pump(&mut self, now: Instant) {
        let host = self.host();
        let host_peer = self.config.bindings.host_peer_id;
        if let Some(link) = self.link.as_mut() {
            link.pump(now);
        }
        self.forward_signals();
        if self.failure.is_some() {
            return;
        }
        let Some(link) = self.link.as_mut() else {
            return;
        };
        let state = link.state();
        if state == LinkState::Failed {
            if self.admitted {
                self.router.detach(host_peer);
                self.counters.disconnected += 1;
                self.out
                    .events
                    .push(format!("PEER_LOST account={host} reason=link_lost"));
                self.fail("connection_closed");
            } else if self.connected {
                self.fail("connection_closed_before_admission");
            } else {
                self.fail("transport_connect_failed");
            }
            return;
        }
        if state == LinkState::Connected && !self.connected {
            self.connected = true;
            self.route = link.route().unwrap_or(Route::Direct);
            if self.config.force_relay && self.route == Route::Direct {
                self.fail("transport_not_relayed");
                return;
            }
            let route = self.route.as_str();
            self.out
                .diagnostics
                .push(format!("ICE_ROUTE role=join peer={host_peer} route={route}"));
            self.out
                .events
                .push(format!("PEER_CONNECTED account={host} route={route}"));
        }
        let Some(link) = self.link.as_mut() else {
            return;
        };
        for (kind, message) in link.receive(FRAMES_PER_PUMP) {
            match kind {
                FrameType::Gameplay if self.admitted => self.router.deliver(host_peer, message),
                FrameType::Gameplay => self.counters.pre_admission_gameplay_dropped += 1,
                _ if self.admitted => {}
                _ => match control::decode_admission_result(&message) {
                    None => self.counters.malformed_control += 1,
                    Some(result) if !result.admitted => {
                        let code = String::from_utf8_lossy(&result.code).into_owned();
                        let code = if code.is_empty() {
                            "admission_rejected".to_owned()
                        } else {
                            code
                        };
                        self.fail(&code);
                        return;
                    }
                    Some(_) => {
                        self.router.attach(host_peer);
                        self.config.ticket = String::new();
                        self.admitted = true;
                        self.counters.admitted += 1;
                        let route = self.route.as_str();
                        self.out.diagnostics.push(format!(
                            "ADMISSION_ACCEPTED role=join peer={host_peer} source=ice route={route}"
                        ));
                        self.out
                            .events
                            .push(format!("PEER_ADMITTED account={host} route={route}"));
                    }
                },
            }
        }
        if self.admitted {
            return;
        }
        if now > self.deadline {
            self.counters.authentication_timeouts += 1;
            let code = if self.connected {
                "admission_response_timeout"
            } else {
                "transport_connect_timeout"
            };
            self.fail(code);
            return;
        }
        if self.connected && self.next_request.is_none_or(|at| now >= at) {
            self.next_request = Some(now + RESEND);
            let Some(frame) = control::encode_admission_request(self.config.ticket.as_bytes()) else {
                self.fail("admission_send_failed");
                return;
            };
            if let Some(link) = self.link.as_mut() {
                link.send(FrameType::Control, &frame);
            }
        }
    }

    /// Closes the link and detaches the host peer.
    pub fn stop(&mut self) {
        if let Some(link) = self.link.as_mut() {
            link.stop();
        }
        self.router.detach(self.config.bindings.host_peer_id);
    }

    /// The joiner bridge's view of the host peer.
    pub fn network(&mut self) -> JoinNetwork<'_> {
        JoinNetwork { session: self }
    }
}

/// The joiner bridge's view: the host peer only, once admitted.
pub struct JoinNetwork<'a> {
    session: &'a mut JoinSession,
}

impl MessageTransport for JoinNetwork<'_> {
    fn send(&mut self, peer: u64, payload: &[u8]) -> bool {
        let session = &mut *self.session;
        if payload.is_empty() || peer != session.config.bindings.host_peer_id || !session.admitted {
            return false;
        }
        session
            .link
            .as_mut()
            .is_some_and(|link| link.send(FrameType::Gameplay, payload))
    }

    fn receive(&mut self, max: usize) -> Vec<(u64, Vec<u8>)> {
        self.session.router.read(max)
    }
}
