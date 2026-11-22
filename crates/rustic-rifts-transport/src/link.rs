// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! One ICE link to one peer, driven by the session thread: the link owns its sockets, the sans-IO agent and the
//! TURN clients, and nothing here takes a lock across a call into them (the reason the agent is sans-IO).
//!
//! The joiner is the controlling side and speaks first; the host creates its link from the joiner's description, so
//! it is controlled and no role conflict arises.

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::BytesMut;
use rtc_ice::agent::agent_config::AgentConfig;
use rtc_ice::candidate::candidate_host::CandidateHostConfig;
use rtc_ice::candidate::candidate_relay::CandidateRelayConfig;
use rtc_ice::candidate::candidate_server_reflexive::CandidateServerReflexiveConfig;
use rtc_ice::candidate::{Candidate, CandidateConfig, CandidateType, unmarshal_candidate};
use rtc_ice::mdns::MulticastDnsMode;
use rtc_ice::network_type::NetworkType;
use rtc_ice::state::ConnectionState;
use rtc_ice::{Agent, Event};
use rtc_shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use rtc_turn::client::{Client, ClientConfig, Event as TurnEvent};
use sansio::Protocol as _;

use crate::filter::{self, CANDIDATE, DESCRIPTION, GATHERING_DONE};
use crate::fragment::{self, Piece, Reassembler};
use crate::seal::{FrameType, PairCipher, Side};
use crate::url::ServerAddr;

/// The largest signal a link accepts, in bytes.
pub const MAX_SIGNAL_BYTES: usize = 4096;
const MAX_OUTGOING_SIGNALS: usize = 64;
const MAX_EARLY_CANDIDATES: usize = 32;
const MAX_RECEIVED: usize = 512;
const RECEIVE_BUFFER: usize = 2048;
const READS_PER_PUMP: usize = 256;
/// Gathering is reported done once every server answered or this long has passed.
const GATHER_LIMIT: Duration = Duration::from_secs(10);
const ICE_OPTIONS: &str = "ice2,trickle";

/// One TURN relay and its credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayServer {
    /// The relay's host and port.
    pub server: ServerAddr,
    /// The TURN username.
    pub username: String,
    /// The TURN credential; never printed.
    pub credential: String,
}

/// What a link is set up with.
#[derive(Debug, Clone)]
pub struct LinkConfig {
    /// Which end of the pair this link is, which picks the seal's direction keys.
    pub side: Side,
    /// The 32-byte key sealing every frame.
    pub pair_key: Vec<u8>,
    /// The STUN server, if one is set.
    pub stun: Option<ServerAddr>,
    /// The TURN relays.
    pub relays: Vec<RelayServer>,
    /// Offer and accept relay candidates only.
    pub force_relay: bool,
}

/// Where a link is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// Created, not started.
    New,
    /// Waiting for the peer's description.
    Gathering,
    /// Both descriptions set; checking candidate pairs.
    Connecting,
    /// A pair was selected; frames may flow.
    Connected,
    /// ICE failed; the link carries nothing more.
    Failed,
    /// Stopped by its owner.
    Closed,
}

/// How a connected link reaches its peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Through a TURN relay at either end.
    Relay,
    /// Host or reflexive candidates at both ends.
    Direct,
}

impl Route {
    /// `relay` or `direct`, as diagnostics name it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Relay => "relay",
            Self::Direct => "direct",
        }
    }
}

/// The SDP name of a candidate type, such as `srflx`.
pub fn candidate_type_name(kind: CandidateType) -> &'static str {
    match kind {
        CandidateType::Host => "host",
        CandidateType::ServerReflexive => "srflx",
        CandidateType::PeerReflexive => "prflx",
        CandidateType::Relay => "relay",
        _ => "unknown",
    }
}

/// What a link failed to send or deliver.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkCounters {
    /// Frames that could not be sealed or sent.
    pub send_failures: u64,
    /// Frames that failed to open or were not valid pieces.
    pub frames_rejected: u64,
    /// Messages dropped with the receive queue full, partial messages evicted included.
    pub queue_drops: u64,
}

struct ServerClient {
    client: Client,
    server: SocketAddr,
    socket: usize,
    relayed: Option<SocketAddr>,
    done: bool,
}

/// The IPv4 addresses host candidates are gathered on: every interface's, loopback included, link-local left out.
fn local_addresses() -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = rtc_shared::ifaces::ifaces()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|iface| match iface.addr.map(|a| a.ip()) {
            Some(IpAddr::V4(ip)) if !ip.is_unspecified() && !ip.is_link_local() && !ip.is_multicast() => Some(ip),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    if !out.contains(&Ipv4Addr::LOCALHOST) {
        out.push(Ipv4Addr::LOCALHOST);
    }
    out
}

fn resolve(server: &ServerAddr) -> Option<SocketAddr> {
    use std::net::ToSocketAddrs as _;
    (server.host.as_str(), server.port)
        .to_socket_addrs()
        .ok()?
        .find(SocketAddr::is_ipv4)
}

fn is_stun(packet: &[u8]) -> bool {
    packet.len() >= 20 && packet[0] & 0xc0 == 0 && packet[4..8] == [0x21, 0x12, 0xa4, 0x42]
}

fn tagged(now: Instant, local: SocketAddr, peer: SocketAddr, data: &[u8]) -> TaggedBytesMut {
    TaggedBytesMut {
        now,
        transport: TransportContext {
            local_addr: local,
            peer_addr: peer,
            ecn: None,
            transport_protocol: TransportProtocol::UDP,
        },
        message: BytesMut::from(data),
    }
}

struct RemoteDescription {
    ufrag: String,
    pwd: String,
    candidates: Vec<String>,
}

fn parse_description(text: &str) -> Option<RemoteDescription> {
    let mut ufrag = None;
    let mut pwd = None;
    let mut candidates = Vec::new();
    for line in text.split('\n').map(|l| l.trim_end_matches('\r').trim()) {
        if let Some(value) = line.strip_prefix("a=ice-ufrag:") {
            ufrag = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("a=ice-pwd:") {
            pwd = Some(value.to_owned());
        } else if line.starts_with("a=candidate:") || line.starts_with("candidate:") {
            candidates.push(line.to_owned());
        }
    }
    let (ufrag, pwd) = (ufrag?, pwd?);
    (!ufrag.is_empty() && !pwd.is_empty()).then_some(RemoteDescription { ufrag, pwd, candidates })
}

fn parse_candidate(line: &str) -> Option<Candidate> {
    let body = line
        .trim()
        .strip_prefix("a=candidate:")
        .or_else(|| line.trim().strip_prefix("candidate:"))?;
    unmarshal_candidate(body).ok()
}

/// One ICE link to one peer, carrying sealed frames once connected.
pub struct Link {
    side: Side,
    force_relay: bool,
    cipher: PairCipher,
    agent: Agent,
    sockets: Vec<UdpSocket>,
    stun: Option<ServerClient>,
    relays: Vec<ServerClient>,
    state: LinkState,
    agent_connected: bool,
    remote_set: bool,
    early_candidates: Vec<String>,
    early_done: bool,
    remote_addrs: Vec<SocketAddr>,
    outgoing: VecDeque<(String, String)>,
    gathering_reported: bool,
    started: Instant,
    reassembler: Reassembler,
    received: VecDeque<(FrameType, Vec<u8>)>,
    next_message: u16,
    counters: LinkCounters,
}

impl Link {
    /// The host passes the joiner's description; the joiner passes none and waits for the host's.
    pub fn start(config: &LinkConfig, remote_description: Option<&str>) -> Result<Self, String> {
        let cipher = PairCipher::new(&config.pair_key, config.side)?;
        if config.force_relay && config.relays.is_empty() {
            return Err("relay-only needs a relay server".to_owned());
        }
        let now = Instant::now();
        let sockets: Vec<UdpSocket> = local_addresses()
            .into_iter()
            .filter_map(|ip| UdpSocket::bind((ip, 0)).ok())
            .filter(|s| s.set_nonblocking(true).is_ok())
            .collect();
        if sockets.is_empty() {
            return Err("ICE agent creation failed".to_owned());
        }
        let provider = rtc_ice::crypto::default_provider().map_err(|_| "ICE agent creation failed".to_owned())?;
        let agent_config = AgentConfig {
            is_controlling: config.side == Side::Joiner,
            multicast_dns_mode: MulticastDnsMode::Disabled,
            network_types: vec![NetworkType::Udp4],
            candidate_types: vec![
                CandidateType::Host,
                CandidateType::ServerReflexive,
                CandidateType::Relay,
            ],
            // RFC 7675's 30 s consent loss
            failed_timeout: Some(Duration::from_secs(30)),
            ..AgentConfig::default()
        };
        let agent =
            Agent::new(now, Arc::new(agent_config), provider.clone()).map_err(|_| "ICE agent creation failed")?;

        let mut link = Self {
            side: config.side,
            force_relay: config.force_relay,
            cipher,
            agent,
            sockets,
            stun: None,
            relays: Vec::new(),
            state: LinkState::New,
            agent_connected: false,
            remote_set: false,
            early_candidates: Vec::new(),
            early_done: false,
            remote_addrs: Vec::new(),
            outgoing: VecDeque::new(),
            gathering_reported: false,
            started: now,
            reassembler: Reassembler::default(),
            received: VecDeque::new(),
            next_message: 0,
            counters: LinkCounters::default(),
        };

        if let Some(text) = remote_description {
            let text = if link.force_relay {
                filter::relay_only(DESCRIPTION, text)
            } else {
                text.to_owned()
            };
            if text.is_empty() || text.len() > MAX_SIGNAL_BYTES || link.set_remote_description(&text, now).is_err() {
                return Err("remote description rejected".to_owned());
            }
        }

        let local = link.local_description();
        let local = if link.force_relay {
            filter::relay_only(DESCRIPTION, &local)
        } else {
            local
        };
        if local.is_empty() {
            return Err("ICE local description is empty".to_owned());
        }
        link.push_signal(DESCRIPTION, local);
        if !link.gather(now, &provider, config) {
            return Err("ICE candidate gathering failed to start".to_owned());
        }
        link.state = if link.remote_set {
            LinkState::Connecting
        } else {
            LinkState::Gathering
        };
        link.flush(now);
        Ok(link)
    }

    fn local_description(&self) -> String {
        let credentials = self.agent.get_local_credentials();
        format!(
            "a=ice-ufrag:{}\r\na=ice-pwd:{}\r\na=ice-options:{ICE_OPTIONS}\r\n",
            credentials.ufrag, credentials.pwd
        )
    }

    fn push_signal(&mut self, kind: &str, data: String) {
        if self.outgoing.len() < MAX_OUTGOING_SIGNALS {
            self.outgoing.push_back((kind.to_owned(), data));
        }
    }

    fn emit_candidate(&mut self, candidate: &Candidate) {
        let line = format!("a=candidate:{}", candidate.marshal());
        let line = if self.force_relay {
            filter::relay_only(CANDIDATE, &line)
        } else {
            line
        };
        if !line.is_empty() {
            self.push_signal(CANDIDATE, line);
        }
    }

    fn socket_for(&self, server: SocketAddr) -> usize {
        let wanted_loopback = server.ip().is_loopback();
        self.sockets
            .iter()
            .position(|s| s.local_addr().is_ok_and(|a| a.ip().is_loopback() == wanted_loopback))
            .unwrap_or(0)
    }

    fn local_addr(&self, socket: usize) -> Option<SocketAddr> {
        self.sockets.get(socket)?.local_addr().ok()
    }

    fn server_client(
        &self,
        server: SocketAddr,
        socket: usize,
        relay: Option<&RelayServer>,
        provider: &Arc<dyn rtc_ice::crypto::RTCCryptoProvider>,
    ) -> Option<Client> {
        let local_addr = self.local_addr(socket)?;
        let config = match relay {
            Some(relay) => ClientConfig {
                turn_serv_addr: server.to_string(),
                local_addr,
                transport_protocol: TransportProtocol::UDP,
                username: relay.username.clone(),
                password: relay.credential.clone(),
                ..ClientConfig::default()
            },
            None => ClientConfig {
                stun_serv_addr: server.to_string(),
                local_addr,
                transport_protocol: TransportProtocol::UDP,
                ..ClientConfig::default()
            },
        };
        Client::new(config, provider.clone()).ok()
    }

    fn gather(
        &mut self,
        now: Instant,
        provider: &Arc<dyn rtc_ice::crypto::RTCCryptoProvider>,
        config: &LinkConfig,
    ) -> bool {
        // A relay-only link never uses a host or reflexive path, so it never offers one to the agent either.
        if !self.force_relay {
            let hosts: Vec<SocketAddr> = self.sockets.iter().filter_map(|s| s.local_addr().ok()).collect();
            for addr in hosts {
                let candidate = CandidateHostConfig {
                    base_config: CandidateConfig {
                        network: "udp".to_owned(),
                        address: addr.ip().to_string(),
                        port: addr.port(),
                        component: 1,
                        ..CandidateConfig::default()
                    },
                    ..CandidateHostConfig::default()
                }
                .new_candidate_host();
                if let Ok(candidate) = candidate
                    && self.agent.add_local_candidate(candidate.clone()).unwrap_or(false)
                {
                    self.emit_candidate(&candidate);
                }
            }
            if let Some(addr) = config.stun.as_ref().and_then(resolve) {
                let socket = self.socket_for(addr);
                if let Some(mut client) = self.server_client(addr, socket, None, provider)
                    && client.send_binding_request(now).is_ok()
                {
                    self.stun = Some(ServerClient {
                        client,
                        server: addr,
                        socket,
                        relayed: None,
                        done: false,
                    });
                }
            }
        }
        for relay in &config.relays {
            let Some(addr) = resolve(&relay.server) else {
                continue;
            };
            let socket = self.socket_for(addr);
            if let Some(mut client) = self.server_client(addr, socket, Some(relay), provider)
                && client.allocate(now).is_ok()
            {
                self.relays.push(ServerClient {
                    client,
                    server: addr,
                    socket,
                    relayed: None,
                    done: false,
                });
            }
        }
        !(self.force_relay && self.relays.is_empty())
    }

    fn set_remote_description(&mut self, text: &str, now: Instant) -> Result<(), ()> {
        let description = parse_description(text).ok_or(())?;
        self.agent
            .start_connectivity_checks(now, self.side == Side::Joiner, description.ufrag, description.pwd)
            .map_err(|_| ())?;
        self.remote_set = true;
        for line in description.candidates {
            let _ = self.add_remote(&line, now);
        }
        for line in std::mem::take(&mut self.early_candidates) {
            let _ = self.add_remote(&line, now);
        }
        if self.state == LinkState::Gathering {
            self.state = LinkState::Connecting;
        }
        Ok(())
    }

    fn add_remote(&mut self, line: &str, now: Instant) -> Result<(), ()> {
        let candidate = parse_candidate(line).ok_or(())?;
        let addr = candidate.addr();
        self.agent.add_remote_candidate(candidate).map_err(|_| ())?;
        if !self.remote_addrs.contains(&addr) {
            self.remote_addrs.push(addr);
            self.permit(addr, now);
        }
        Ok(())
    }

    /// TURN relays drop traffic from and to a peer without a permission.
    fn permit(&mut self, peer: SocketAddr, now: Instant) {
        for relay in &mut self.relays {
            if let Some(relayed) = relay.relayed {
                let _ = relay
                    .client
                    .relay(relayed)
                    .and_then(|mut handle| handle.create_permission(now, peer));
            }
        }
    }

    /// Applies one signal from the peer (spec §2.10.2).
    pub fn apply_signal(&mut self, kind: &str, data: &str) -> Result<(), String> {
        if self.state == LinkState::Closed {
            return Err("link is closed".to_owned());
        }
        if data.len() > MAX_SIGNAL_BYTES {
            return Err("signal too large".to_owned());
        }
        let data = if self.force_relay && kind != GATHERING_DONE {
            filter::relay_only(kind, data)
        } else {
            data.to_owned()
        };
        let now = Instant::now();
        let result = match kind {
            DESCRIPTION => {
                if self.remote_set {
                    Err("remote description already set".to_owned())
                } else if self.force_relay && data.is_empty() {
                    // Kept as observed: a description with nothing left is refused with no text (spec §7).
                    Err(String::new())
                } else {
                    self.set_remote_description(&data, now)
                        .map_err(|()| "remote description rejected".to_owned())
                }
            }
            CANDIDATE => {
                if self.force_relay && data.is_empty() {
                    Ok(())
                } else if !self.remote_set {
                    if self.early_candidates.len() >= MAX_EARLY_CANDIDATES {
                        Err("too many early candidates".to_owned())
                    } else {
                        self.early_candidates.push(data);
                        Ok(())
                    }
                } else {
                    self.add_remote(&data, now)
                        .map_err(|()| "remote candidate rejected".to_owned())
                }
            }
            GATHERING_DONE => {
                if !self.remote_set {
                    self.early_done = true;
                }
                Ok(())
            }
            _ => Err("unknown signal kind".to_owned()),
        };
        self.flush(now);
        result
    }

    /// Signals for the peer, oldest first.
    pub fn take_signals(&mut self) -> Vec<(String, String)> {
        self.outgoing.drain(..).collect()
    }

    /// Whether the peer's description was applied.
    pub fn has_remote_description(&self) -> bool {
        self.remote_set
    }

    /// Reads every socket, runs due timers and sends what the agent and clients produced.
    pub fn pump(&mut self, now: Instant) {
        if self.state == LinkState::Closed {
            return;
        }
        let mut buffer = [0_u8; RECEIVE_BUFFER * 2];
        for socket in 0..self.sockets.len() {
            for _ in 0..READS_PER_PUMP {
                let received = self.sockets[socket].recv_from(&mut buffer);
                match received {
                    Ok((size, from)) => {
                        let packet = buffer[..size].to_vec();
                        self.dispatch(socket, from, &packet, now);
                    }
                    // WouldBlock ends the round; an ICMP unreachable shows up here on Windows and is skipped too.
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
        }
        if self.agent.poll_timeout().is_some_and(|due| due <= now) {
            let _ = self.agent.handle_timeout(now);
        }
        for client in self.relays.iter_mut().chain(self.stun.iter_mut()) {
            if client.client.poll_timeout().is_some_and(|due| due <= now) {
                let _ = client.client.handle_timeout(now);
            }
        }
        self.process_client_events(now);
        self.process_agent_events();
        let servers_done = self.stun.as_ref().is_none_or(|s| s.done) && self.relays.iter().all(|r| r.done);
        if !self.gathering_reported && (servers_done || now.saturating_duration_since(self.started) >= GATHER_LIMIT) {
            self.gathering_reported = true;
            self.push_signal(GATHERING_DONE, "-".to_owned());
        }
        self.flush(now);
    }

    fn dispatch(&mut self, socket: usize, from: SocketAddr, packet: &[u8], now: Instant) {
        let Some(local) = self.local_addr(socket) else {
            return;
        };
        let server = self
            .relays
            .iter_mut()
            .chain(self.stun.iter_mut())
            .find(|c| c.server == from && c.socket == socket);
        if let Some(server) = server {
            let _ = server.client.handle_read(tagged(now, local, from, packet));
            self.process_client_events(now);
            return;
        }
        self.inbound(local, from, packet, now);
    }

    fn inbound(&mut self, local: SocketAddr, from: SocketAddr, packet: &[u8], now: Instant) {
        if is_stun(packet) {
            let _ = self.agent.handle_read(tagged(now, local, from, packet));
            return;
        }
        let context = TransportContext {
            local_addr: local,
            peer_addr: from,
            ecn: None,
            transport_protocol: TransportProtocol::UDP,
        };
        if self.agent.is_valid_non_stun_traffic(now, context) {
            self.on_frame(packet, now);
        }
    }

    fn process_client_events(&mut self, now: Instant) {
        let mut data = Vec::new();
        let mut allocated = Vec::new();
        for (index, relay) in self.relays.iter_mut().enumerate() {
            while let Some(event) = relay.client.poll_event() {
                match event {
                    TurnEvent::AllocateResponse(_, relayed) if relay.relayed.is_none() => {
                        relay.relayed = Some(relayed);
                        relay.done = true;
                        allocated.push(index);
                    }
                    TurnEvent::AllocateError(..) | TurnEvent::TransactionTimeout(_) => relay.done = true,
                    TurnEvent::DataIndicationOrChannelData(_, peer, bytes) => {
                        if let Some(relayed) = relay.relayed {
                            data.push((relayed, peer, bytes.to_vec()));
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut reflexive = None;
        if let Some(stun) = self.stun.as_mut() {
            while let Some(event) = stun.client.poll_event() {
                match event {
                    TurnEvent::BindingResponse(_, mapped) if !stun.done => {
                        stun.done = true;
                        reflexive = Some((mapped, stun.socket));
                    }
                    TurnEvent::BindingError(..) | TurnEvent::TransactionTimeout(_) => stun.done = true,
                    _ => {}
                }
            }
        }
        for index in allocated {
            self.add_relay_candidate(index, now);
        }
        if let Some((mapped, socket)) = reflexive
            && let Some(base) = self.local_addr(socket)
        {
            let candidate = CandidateServerReflexiveConfig {
                base_config: CandidateConfig {
                    network: "udp".to_owned(),
                    address: mapped.ip().to_string(),
                    port: mapped.port(),
                    component: 1,
                    ..CandidateConfig::default()
                },
                rel_addr: base.ip().to_string(),
                rel_port: base.port(),
                url: None,
            }
            .new_candidate_server_reflexive();
            if let Ok(candidate) = candidate
                && self.agent.add_local_candidate(candidate.clone()).unwrap_or(false)
            {
                self.emit_candidate(&candidate);
            }
        }
        for (relayed, peer, bytes) in data {
            self.inbound(relayed, peer, &bytes, now);
        }
    }

    fn add_relay_candidate(&mut self, index: usize, now: Instant) {
        let Some(relayed) = self.relays.get(index).and_then(|r| r.relayed) else {
            return;
        };
        // No raddr: the relay candidate is what a relay-only peer sees, and it must not carry our address.
        let candidate = CandidateRelayConfig {
            base_config: CandidateConfig {
                network: "udp".to_owned(),
                address: relayed.ip().to_string(),
                port: relayed.port(),
                component: 1,
                ..CandidateConfig::default()
            },
            rel_addr: "0.0.0.0".to_owned(),
            rel_port: 0,
            url: None,
        }
        .new_candidate_relay();
        if let Ok(candidate) = candidate
            && self.agent.add_local_candidate(candidate.clone()).unwrap_or(false)
        {
            self.emit_candidate(&candidate);
        }
        let peers = self.remote_addrs.clone();
        if let Some(relay) = self.relays.get_mut(index) {
            for peer in peers {
                let _ = relay
                    .client
                    .relay(relayed)
                    .and_then(|mut handle| handle.create_permission(now, peer));
            }
        }
    }

    fn process_agent_events(&mut self) {
        while let Some(event) = self.agent.poll_event() {
            if let Event::ConnectionStateChange(state) = event.event {
                match state {
                    ConnectionState::Connected | ConnectionState::Completed => self.agent_connected = true,
                    ConnectionState::Failed if self.state != LinkState::Closed => self.state = LinkState::Failed,
                    _ => {}
                }
            }
        }
        let usable = !matches!(self.state, LinkState::Failed | LinkState::Closed);
        if usable && self.agent_connected && self.agent.get_selected_candidate_pair().is_some() {
            self.state = LinkState::Connected;
        }
    }

    fn flush(&mut self, now: Instant) {
        while let Some(message) = self.agent.poll_write() {
            let local = message.transport.local_addr;
            let peer = message.transport.peer_addr;
            self.send_from(local, peer, &message.message, now);
        }
        for client in self.relays.iter_mut().chain(self.stun.iter_mut()) {
            while let Some(transmit) = client.client.poll_write() {
                if let Some(socket) = self.sockets.get(client.socket) {
                    let _ = socket.send_to(&transmit.message, transmit.transport.peer_addr);
                }
            }
        }
    }

    /// Sends from a local candidate: through its allocation when it is a relay, else from its socket.
    fn send_from(&mut self, local: SocketAddr, peer: SocketAddr, data: &[u8], now: Instant) -> bool {
        if let Some(relay) = self.relays.iter_mut().find(|r| r.relayed == Some(local)) {
            return relay
                .client
                .relay(local)
                .and_then(|mut handle| handle.send_to(now, data, peer))
                .is_ok();
        }
        self.sockets
            .iter()
            .find(|s| s.local_addr().is_ok_and(|a| a == local))
            .is_some_and(|s| s.send_to(data, peer).is_ok_and(|sent| sent == data.len()))
    }

    fn on_frame(&mut self, frame: &[u8], now: Instant) {
        let Some((kind, plain)) = self.cipher.open(frame) else {
            self.counters.frames_rejected += 1;
            return;
        };
        let message = match kind {
            FrameType::Fragment => match self.reassembler.add(&plain, now) {
                Piece::Rejected => {
                    self.counters.frames_rejected += 1;
                    return;
                }
                Piece::Pending => return,
                Piece::Complete(inner, message) => (inner, message),
            },
            other => (other, plain),
        };
        if self.received.len() >= MAX_RECEIVED {
            self.counters.queue_drops += 1;
            return;
        }
        self.received.push_back(message);
    }

    fn transmit(&mut self, frame: &[u8], now: Instant) -> bool {
        let Some((local, remote)) = self.agent.get_selected_candidate_pair() else {
            return false;
        };
        let from = if local.candidate_type() == CandidateType::Relay {
            local.addr()
        } else {
            local.base_addr()
        };
        let to = remote.addr();
        self.send_from(from, to, frame, now)
    }

    /// Seals and sends one message, split into fragments over 1100 bytes.
    pub fn send(&mut self, kind: FrameType, payload: &[u8]) -> bool {
        let sent = self.try_send(kind, payload);
        if !sent {
            self.counters.send_failures += 1;
        }
        sent
    }

    fn try_send(&mut self, kind: FrameType, payload: &[u8]) -> bool {
        if self.state != LinkState::Connected || kind == FrameType::Fragment || payload.is_empty() {
            return false;
        }
        let now = Instant::now();
        let sent = if payload.len() <= fragment::PIECE_BYTES {
            self.cipher
                .seal(kind, payload)
                .is_some_and(|frame| self.transmit(&frame, now))
        } else {
            let id = self.next_message;
            self.next_message = self.next_message.wrapping_add(1);
            let Some(pieces) = fragment::split(kind, id, payload) else {
                return false;
            };
            let mut all = true;
            for piece in pieces {
                let ok = self
                    .cipher
                    .seal(FrameType::Fragment, &piece)
                    .is_some_and(|frame| self.transmit(&frame, now));
                all &= ok;
            }
            all
        };
        self.flush(now);
        sent
    }

    /// Up to `max` delivered messages, oldest first.
    pub fn receive(&mut self, max: usize) -> Vec<(FrameType, Vec<u8>)> {
        let count = max.min(self.received.len());
        self.received.drain(..count).collect()
    }

    /// Where the link is in its life.
    pub fn state(&self) -> LinkState {
        self.state
    }

    /// The selected pair's local and remote candidate types.
    pub fn selected_types(&self) -> Option<(CandidateType, CandidateType)> {
        self.agent
            .get_selected_candidate_pair()
            .map(|(local, remote)| (local.candidate_type(), remote.candidate_type()))
    }

    /// Relayed when either end of the selected pair is a relay candidate.
    pub fn route(&self) -> Option<Route> {
        self.selected_types().map(|(local, remote)| {
            if local == CandidateType::Relay || remote == CandidateType::Relay {
                Route::Relay
            } else {
                Route::Direct
            }
        })
    }

    /// What the link failed to send or deliver so far.
    pub fn counters(&self) -> LinkCounters {
        LinkCounters {
            queue_drops: self.counters.queue_drops + self.reassembler.queue_drops,
            ..self.counters
        }
    }

    /// Closes the TURN allocations and sockets; the link carries nothing more.
    pub fn stop(&mut self) {
        if self.state == LinkState::Closed {
            return;
        }
        let now = Instant::now();
        for relay in &mut self.relays {
            if let Some(relayed) = relay.relayed {
                let _ = relay.client.relay(relayed).and_then(|mut handle| handle.close(now));
            }
        }
        self.flush(now);
        let _ = self.agent.close();
        self.state = LinkState::Closed;
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop();
    }
}
