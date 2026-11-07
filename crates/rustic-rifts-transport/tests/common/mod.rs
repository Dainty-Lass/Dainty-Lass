// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Test helpers: a ticket signer, the session files, a fake backend, a UDP echo server, a game client and a signal
//! carrier between sessions.

#![allow(dead_code, reason = "shared by several test binaries, each using only some")]

pub mod backend;
pub mod turn_server;

use std::collections::HashSet;
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use p256::ecdsa::SigningKey;
use rustic_rifts_admission::KeyId;
use rustic_rifts_admission::ticket::{Es256Tickets, Jti, SessionUuid, TicketBindings, TicketClaims};
use rustic_rifts_seam::{AccountId, PeerId};
use rustic_rifts_transport::files::Bindings;
use rustic_rifts_transport::{Handle, Ready, Transport};

/// Each host gets a pipe name of its own; a pipe is the first and only instance of its name.
static PIPES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub const KEY_ID: &str = "test-key";
pub const PAIR_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
pub const CAPABILITY: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";

pub fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// An ES256 key pair, its public half written as the raw X‖Y file.
pub struct Signer {
    pub tickets: Es256Tickets,
}

impl Signer {
    pub fn new() -> Self {
        let mut seed = [0_u8; 32];
        for (i, byte) in seed.iter_mut().enumerate() {
            *byte = (i as u8).wrapping_mul(37).wrapping_add(11);
        }
        let signing = SigningKey::from_slice(&seed).unwrap();
        let verifying = *signing.verifying_key();
        Self {
            tickets: Es256Tickets {
                key_id: KeyId(KEY_ID.to_owned()),
                signing: Some(signing),
                verifying,
                default_lifetime: 120.0,
            },
        }
    }

    pub fn public_xy(&self) -> Vec<u8> {
        rustic_rifts_admission::keys::public_xy(&self.tickets.verifying)
            .0
            .to_vec()
    }

    pub fn write_key(&self, dir: &Path) -> PathBuf {
        let path = dir.join("p2p-admission-public.xy");
        std::fs::write(&path, self.public_xy()).unwrap();
        path
    }

    /// A ticket issued now, valid 120 s.
    pub fn mint(&self, bindings: &Bindings, account: &str, joiner: u64, host: u64) -> String {
        let now = now_seconds();
        self.mint_at(bindings, account, joiner, host, now, now + 120)
    }

    pub fn mint_at(
        &self,
        bindings: &Bindings,
        account: &str,
        joiner: u64,
        host: u64,
        issued: u64,
        expires: u64,
    ) -> String {
        let claims = TicketClaims {
            key_id: KeyId(bindings.key_id.clone()),
            jti: Jti::random().unwrap(),
            bindings: TicketBindings {
                account: AccountId(account.to_owned()),
                joiner_peer: PeerId(joiner),
                host_peer: PeerId(host),
                session: SessionUuid::from_str(&bindings.session_id).unwrap(),
                generation: bindings.generation,
                party_id: bindings.party_id.clone(),
                party_revision: bindings.party_revision,
                hunt_definition_id: bindings.hunt_definition_id.clone(),
                build: bindings.build,
                bridge_protocol: bindings.bridge_protocol,
                virtual_port: bindings.virtual_port,
            },
            issued_at: issued,
            not_before: issued,
            expires_at: expires,
        };
        self.tickets.sign(claims).unwrap().text.expose().to_owned()
    }
}

pub fn bindings(session: u32, host_peer: u64) -> Bindings {
    Bindings {
        key_id: KEY_ID.to_owned(),
        session_id: format!("3f1c2a4e-1b2c-4d3e-8f40-{session:012x}"),
        party_id: "party-1".to_owned(),
        hunt_definition_id: "hunt:def/1".to_owned(),
        bridge_instance_id: format!("bridge-{session}"),
        party_revision: 1,
        host_peer_id: host_peer,
        generation: 1,
        build: 7,
        bridge_protocol: 3,
        virtual_port: 7,
    }
}

pub fn binding_lines(b: &Bindings) -> String {
    format!(
        "key_id={}\nsession_id={}\nparty_id={}\nhunt_definition_id={}\nbridge_instance_id={}\nparty_revision={}\n\
         host_peer_id={}\ngeneration={}\nbuild={}\nbridge_protocol={}\nvirtual_port={}\n",
        b.key_id,
        b.session_id,
        b.party_id,
        b.hunt_definition_id,
        b.bridge_instance_id,
        b.party_revision,
        b.host_peer_id,
        b.generation,
        b.build,
        b.bridge_protocol,
        b.virtual_port
    )
}

/// The files of one session in a directory of its own.
pub struct Files {
    pub dir: tempfile::TempDir,
}

impl Files {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    pub fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
}

pub struct HostSpec<'a> {
    pub bindings: &'a Bindings,
    pub backend_url: String,
    pub server_port: u16,
    pub role: Option<&'a str>,
    pub maximum_peers: usize,
    pub local_client: Option<&'a str>,
    pub peers: Vec<(String, u64)>,
    pub extra: String,
    pub secrets_extra: String,
}

impl HostSpec<'_> {
    pub fn config(&self) -> String {
        let mut text = binding_lines(self.bindings);
        text.push_str(&format!(
            "backend_url={}\nserver_udp_port={}\nmaximum_peers={}\n",
            self.backend_url, self.server_port, self.maximum_peers
        ));
        if let Some(role) = self.role {
            text.push_str(&format!("session_role={role}\n"));
        }
        if let Some(local) = self.local_client {
            text.push_str(&format!("host_local_client={local}\n"));
        }
        if self.role != Some("HUB") {
            text.push_str(&format!(
                "guard_pipe_name=RusticHuntGuard.events.test-{}-{}\nexpected_server_pid={}\n",
                std::process::id(),
                PIPES.fetch_add(1, Ordering::SeqCst),
                std::process::id()
            ));
        }
        text.push_str(&self.extra);
        text
    }

    pub fn secrets(&self) -> String {
        let mut text: String = self
            .peers
            .iter()
            .map(|(account, id)| format!("peer={account},{id},{PAIR_KEY}\n"))
            .collect();
        text.push_str(&self.secrets_extra);
        text
    }
}

/// Opens a host session from its files.
pub fn open_host(
    transport: &Transport,
    files: &Files,
    signer: &Signer,
    spec: &HostSpec<'_>,
    host_ticket: Option<&str>,
) -> Result<Handle, String> {
    let key = signer.write_key(files.dir.path());
    let config = files.write("bridge.conf", &spec.config());
    let secrets = files.write("transport.secret", &spec.secrets());
    let capability = files.write("capability.secret", CAPABILITY);
    let ticket = host_ticket.map(|t| files.write("ticket.secret", t));
    transport.host_open(&key, &config, &capability, ticket.as_deref(), &secrets)
}

pub struct JoinSpec<'a> {
    pub bindings: &'a Bindings,
    pub local_peer: u64,
    pub local_account: &'a str,
    pub host_account: &'a str,
    pub extra: String,
    pub secrets_extra: String,
}

pub fn open_join(transport: &Transport, files: &Files, signer: &Signer, spec: &JoinSpec<'_>, ticket: &str) -> Handle {
    let key = signer.write_key(files.dir.path());
    let mut config = binding_lines(spec.bindings);
    config.push_str(&format!(
        "local_udp_port=0\nlocal_peer_id={}\nlocal_account_id={}\nhost_account_id={}\n",
        spec.local_peer, spec.local_account, spec.host_account
    ));
    config.push_str(&spec.extra);
    let config = files.write("bridge.conf", &config);
    let secrets = files.write(
        "transport.secret",
        &format!(
            "peer={},{},{PAIR_KEY}\n{}",
            spec.host_account, spec.bindings.host_peer_id, spec.secrets_extra
        ),
    );
    let ticket = files.write("ticket.secret", ticket);
    transport.join_open(&key, &config, &ticket, &secrets).unwrap()
}

/// One end of a signal path.
#[derive(Clone)]
pub struct End {
    pub transport: Transport,
    pub handle: Handle,
    pub account: String,
}

/// Moves `ReadSignalH` output into the other end's `SubmitSignalH`, by the `to` account.
pub struct Carrier {
    pub pairs: Vec<(End, End)>,
}

impl Carrier {
    pub fn carry(&self) {
        let mut ends: Vec<&End> = Vec::new();
        for (a, b) in &self.pairs {
            for end in [a, b] {
                if !ends.iter().any(|e| same(e, end)) {
                    ends.push(end);
                }
            }
        }
        for from in ends {
            while let Some(signal) = from.transport.read_signal(from.handle) {
                let target = self.pairs.iter().find_map(|(a, b)| {
                    if same(a, from) && b.account == signal.to {
                        Some(b)
                    } else if same(b, from) && a.account == signal.to {
                        Some(a)
                    } else {
                        None
                    }
                });
                if let Some(to) = target {
                    to.transport
                        .submit_signal(to.handle, &from.account, &signal.kind, &signal.data);
                }
            }
        }
    }
}

fn same(a: &End, b: &End) -> bool {
    a.handle == b.handle && a.transport.same_as(&b.transport)
}

/// Reads diagnostics into `seen` and returns them too.
pub fn drain_diagnostics(transport: &Transport, handle: Handle, seen: &mut Vec<String>) {
    while let Some(line) = transport.read_diagnostic(handle) {
        seen.push(line);
    }
}

pub fn drain_events(transport: &Transport, handle: Handle, seen: &mut Vec<String>) {
    while let Some(line) = transport.read_peer_event(handle) {
        seen.push(line);
    }
}

/// Waits for readiness while carrying signals.
pub fn wait_ready_carrying(
    transport: &Transport,
    handle: Handle,
    carrier: &Carrier,
    limit: Duration,
) -> Option<String> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        carrier.carry();
        match transport.wait_ready(handle, 20) {
            Ready::Ready(line) => return Some(line),
            Ready::Ended => return None,
            Ready::TimedOut => {}
        }
    }
    None
}

/// The value of `key=` in a line of space-separated fields.
pub fn field(line: &str, key: &str) -> Option<String> {
    line.split(' ')
        .find_map(|token| token.strip_prefix(&format!("{key}=")).map(str::to_owned))
}

/// A UDP echo server on 127.0.0.1 that remembers the source ports it saw.
pub struct Echo {
    pub port: u16,
    pub sources: Arc<Mutex<HashSet<SocketAddr>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Echo {
    pub fn start() -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let port = socket.local_addr().unwrap().port();
        let sources: Arc<Mutex<HashSet<SocketAddr>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_sources, thread_stop) = (sources.clone(), stop.clone());
        let thread = thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            while !thread_stop.load(Ordering::SeqCst) {
                if let Ok((size, from)) = socket.recv_from(&mut buffer) {
                    thread_sources.lock().unwrap().insert(from);
                    let _ = socket.send_to(&buffer[..size], from);
                }
            }
        });
        Self {
            port,
            sources,
            stop,
            thread: Some(thread),
        }
    }

    pub fn source_count(&self) -> usize {
        self.sources.lock().unwrap().len()
    }
}

impl Drop for Echo {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A game client: sends to the bridge and waits for the echo.
pub struct Client {
    pub socket: UdpSocket,
    pub target: u16,
}

impl Client {
    pub fn new(target: u16) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        Self { socket, target }
    }

    /// Sends until the echo comes back or `limit` passes; the time of the successful round trip.
    pub fn round_trip(&self, size: usize, limit: Duration) -> Option<Duration> {
        let payload: Vec<u8> = (0..size).map(|i| (i * 7 % 256) as u8).collect();
        let deadline = Instant::now() + limit;
        let mut buffer = [0_u8; 4096];
        while Instant::now() < deadline {
            let sent = Instant::now();
            self.socket.send_to(&payload, ("127.0.0.1", self.target)).ok()?;
            let wait_until = sent + Duration::from_millis(300);
            while Instant::now() < wait_until {
                if let Ok((n, _)) = self.socket.recv_from(&mut buffer)
                    && buffer[..n] == payload[..]
                {
                    return Some(sent.elapsed());
                }
            }
        }
        None
    }
}
