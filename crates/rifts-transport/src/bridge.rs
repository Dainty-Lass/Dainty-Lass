// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The UDP side the game sees: IPv4 sockets on 127.0.0.1 only, non-blocking, pumped by the session thread.

use std::collections::BTreeMap;
use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::router::MessageTransport;

pub const MAX_DATAGRAM: usize = 2048;
pub const PER_PUMP: usize = 64;
const REPIN_SILENCE: Duration = Duration::from_secs(2);
const MAX_ERROR_REPORTS: u32 = 8;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BridgeCounters {
    pub local_datagrams_received: u64,
    pub local_datagrams_sent: u64,
    pub transport_messages_received: u64,
    pub transport_messages_sent: u64,
    pub oversized_datagrams_dropped: u64,
    pub unexpected_local_sources_dropped: u64,
    pub unknown_peers_dropped: u64,
    pub transport_backpressure_drops: u64,
    pub socket_errors: u64,
}

impl BridgeCounters {
    /// The ten bridge fields of a `UDP_STATS` line, in order.
    pub fn stats_fields(&self) -> String {
        format!(
            "localRx={} localTx={} transportRx={} transportTx={} oversized={} foreign={} unknownPeers={} \
             backpressure={} socketErrors={}",
            self.local_datagrams_received,
            self.local_datagrams_sent,
            self.transport_messages_received,
            self.transport_messages_sent,
            self.oversized_datagrams_dropped,
            self.unexpected_local_sources_dropped,
            self.unknown_peers_dropped,
            self.transport_backpressure_drops,
            self.socket_errors,
        )
    }
}

fn bind_loopback(port: u16) -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, port))?;
    socket.set_nonblocking(true)?;
    Ok(socket)
}

fn connected_to_server(port: u16) -> io::Result<UdpSocket> {
    let socket = bind_loopback(0)?;
    socket.connect((Ipv4Addr::LOCALHOST, port))?;
    Ok(socket)
}

fn is_loopback_v4(addr: SocketAddr) -> bool {
    addr.ip() == Ipv4Addr::LOCALHOST
}

enum Received {
    Datagram(usize, SocketAddr),
    Empty,
    Done,
    Error(io::Error),
}

fn receive_from(socket: &UdpSocket, buffer: &mut [u8]) -> Received {
    match socket.recv_from(buffer) {
        Ok((0, _)) => Received::Empty,
        Ok((size, from)) => Received::Datagram(size, from),
        Err(e) if e.kind() == ErrorKind::WouldBlock => Received::Done,
        Err(e) => Received::Error(e),
    }
}

/// The joiner's bridge: the game client talks to it, and it talks to the host peer.
pub struct JoinerBridge {
    socket: UdpSocket,
    host_peer: u64,
    pinned: Option<SocketAddr>,
    pub counters: BridgeCounters,
}

impl JoinerBridge {
    pub fn bind(port: u16, host_peer: u64) -> io::Result<Self> {
        Ok(Self {
            socket: bind_loopback(port)?,
            host_peer,
            pinned: None,
            counters: BridgeCounters::default(),
        })
    }

    pub fn port(&self) -> u16 {
        self.socket.local_addr().map_or(0, |a| a.port())
    }

    pub fn pump(&mut self, network: &mut dyn MessageTransport) {
        let mut buffer = [0_u8; MAX_DATAGRAM + 1];
        for _ in 0..PER_PUMP {
            match receive_from(&self.socket, &mut buffer) {
                Received::Datagram(size, from) => {
                    self.counters.local_datagrams_received += 1;
                    if size > MAX_DATAGRAM {
                        self.counters.oversized_datagrams_dropped += 1;
                        continue;
                    }
                    match self.pinned {
                        None if is_loopback_v4(from) => self.pinned = Some(from),
                        Some(pinned) if pinned == from => {}
                        _ => {
                            self.counters.unexpected_local_sources_dropped += 1;
                            continue;
                        }
                    }
                    if network.send(self.host_peer, &buffer[..size]) {
                        self.counters.transport_messages_sent += 1;
                    } else {
                        self.counters.transport_backpressure_drops += 1;
                    }
                }
                Received::Empty => {}
                Received::Done => break,
                Received::Error(_) => {
                    self.counters.socket_errors += 1;
                    break;
                }
            }
        }
        for (peer, message) in network.receive(PER_PUMP) {
            self.counters.transport_messages_received += 1;
            if peer != self.host_peer {
                self.counters.unknown_peers_dropped += 1;
            } else if message.is_empty() || message.len() > MAX_DATAGRAM {
                self.counters.oversized_datagrams_dropped += 1;
            } else if let Some(client) = self.pinned {
                match self.socket.send_to(&message, client) {
                    Ok(sent) if sent == message.len() => self.counters.local_datagrams_sent += 1,
                    _ => self.counters.socket_errors += 1,
                }
            } else {
                self.counters.unexpected_local_sources_dropped += 1;
            }
        }
    }
}

/// The host's bridge for joiners: one socket per joiner, so the game server sees one source port each.
pub struct HostRemoteBridge {
    server_port: u16,
    max_peers: usize,
    sockets: BTreeMap<u64, UdpSocket>,
    pub counters: BridgeCounters,
}

impl HostRemoteBridge {
    pub fn new(server_port: u16, max_peers: usize) -> Self {
        Self {
            server_port,
            max_peers,
            sockets: BTreeMap::new(),
            counters: BridgeCounters::default(),
        }
    }

    pub fn disconnect(&mut self, peer: u64) {
        self.sockets.remove(&peer);
    }

    pub fn slots(&self) -> usize {
        self.sockets.len()
    }

    pub fn pump(&mut self, network: &mut dyn MessageTransport) {
        for (peer, message) in network.receive(PER_PUMP) {
            self.counters.transport_messages_received += 1;
            if message.is_empty() || message.len() > MAX_DATAGRAM {
                self.counters.oversized_datagrams_dropped += 1;
                continue;
            }
            if peer == 0 || (!self.sockets.contains_key(&peer) && self.sockets.len() >= self.max_peers) {
                self.counters.unknown_peers_dropped += 1;
                continue;
            }
            if !self.sockets.contains_key(&peer) {
                match connected_to_server(self.server_port) {
                    Ok(socket) => {
                        self.sockets.insert(peer, socket);
                    }
                    Err(_) => {
                        self.counters.socket_errors += 1;
                        self.counters.unknown_peers_dropped += 1;
                        continue;
                    }
                }
            }
            let sent = self.sockets.get(&peer).map(|s| s.send(&message));
            match sent {
                Some(Ok(size)) if size == message.len() => self.counters.local_datagrams_sent += 1,
                _ => self.counters.socket_errors += 1,
            }
        }
        let mut budget = PER_PUMP;
        let mut buffer = [0_u8; MAX_DATAGRAM + 1];
        let mut outgoing = Vec::new();
        for (peer, socket) in &self.sockets {
            while budget > 0 {
                match socket.recv(&mut buffer) {
                    Ok(0) => budget -= 1,
                    Ok(size) => {
                        budget -= 1;
                        self.counters.local_datagrams_received += 1;
                        if size > MAX_DATAGRAM {
                            self.counters.oversized_datagrams_dropped += 1;
                        } else {
                            outgoing.push((*peer, buffer[..size].to_vec()));
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => {
                        self.counters.socket_errors += 1;
                        break;
                    }
                }
            }
        }
        for (peer, message) in outgoing {
            if network.send(peer, &message) {
                self.counters.transport_messages_sent += 1;
            } else {
                self.counters.transport_backpressure_drops += 1;
            }
        }
    }
}

/// The host's own game client, on the same machine as the game server.
pub struct HostLocalBridge {
    client: UdpSocket,
    server: UdpSocket,
    server_port: u16,
    pinned: Option<SocketAddr>,
    last_heard: Instant,
    reports: u32,
    last_report: Option<(&'static str, i32)>,
    pub counters: BridgeCounters,
}

impl HostLocalBridge {
    pub fn new(server_port: u16) -> io::Result<Self> {
        Ok(Self {
            client: bind_loopback(0)?,
            server: connected_to_server(server_port)?,
            server_port,
            pinned: None,
            last_heard: Instant::now(),
            reports: 0,
            last_report: None,
            counters: BridgeCounters::default(),
        })
    }

    pub fn port(&self) -> u16 {
        self.client.local_addr().map_or(0, |a| a.port())
    }

    fn socket_error(&mut self, op: &'static str, error: &io::Error, diagnostics: &mut Vec<String>) {
        self.counters.socket_errors += 1;
        let code = error.raw_os_error().unwrap_or(0);
        if self.reports >= MAX_ERROR_REPORTS || self.last_report == Some((op, code)) {
            return;
        }
        self.reports += 1;
        self.last_report = Some((op, code));
        diagnostics.push(format!("HOST_UDP_SOCKET_ERROR path=local op={op} wsa={code}"));
    }

    pub fn pump(&mut self, diagnostics: &mut Vec<String>) {
        let mut buffer = [0_u8; MAX_DATAGRAM + 1];
        for _ in 0..PER_PUMP {
            match receive_from(&self.client, &mut buffer) {
                Received::Datagram(size, from) => {
                    self.counters.local_datagrams_received += 1;
                    if size > MAX_DATAGRAM {
                        self.counters.oversized_datagrams_dropped += 1;
                        continue;
                    }
                    let now = Instant::now();
                    match self.pinned {
                        None if is_loopback_v4(from) => self.pinned = Some(from),
                        Some(pinned) if pinned == from => {}
                        Some(_)
                            if is_loopback_v4(from)
                                && now.saturating_duration_since(self.last_heard) >= REPIN_SILENCE =>
                        {
                            // A restarted client gets a fresh server-side source port, so the server sees a new
                            // connection rather than the old one's state.
                            let Ok(fresh) = connected_to_server(self.server_port) else {
                                self.counters.unexpected_local_sources_dropped += 1;
                                continue;
                            };
                            self.server = fresh;
                            self.pinned = Some(from);
                            diagnostics.push("LOCAL_CLIENT_REPINNED role=host path=local".to_owned());
                        }
                        _ => {
                            self.counters.unexpected_local_sources_dropped += 1;
                            continue;
                        }
                    }
                    self.last_heard = now;
                    match self.server.send(&buffer[..size]) {
                        Ok(sent) if sent == size => self.counters.transport_messages_sent += 1,
                        Ok(_) => self.counters.socket_errors += 1,
                        Err(e) => self.socket_error("client_send", &e, diagnostics),
                    }
                }
                Received::Empty => {}
                Received::Done => break,
                Received::Error(e) => {
                    self.socket_error("client_recvfrom", &e, diagnostics);
                    break;
                }
            }
        }
        for _ in 0..PER_PUMP {
            match self.server.recv(&mut buffer) {
                Ok(0) => {}
                Ok(size) => {
                    self.counters.transport_messages_received += 1;
                    let Some(client) = self.pinned else {
                        continue;
                    };
                    if size > MAX_DATAGRAM {
                        self.counters.oversized_datagrams_dropped += 1;
                        continue;
                    }
                    match self.client.send_to(&buffer[..size], client) {
                        Ok(sent) if sent == size => self.counters.local_datagrams_sent += 1,
                        Ok(_) => self.counters.socket_errors += 1,
                        Err(e) => self.socket_error("server_sendto", &e, diagnostics),
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => {
                    self.socket_error("server_recv", &e, diagnostics);
                    break;
                }
            }
        }
    }
}
