// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The in-process transport behind the trait (spec §8.2 item 4, rewritten for D1), and a hub hosted and joined end to
//! end through two `LocalRealmHosting`s over the real transport, a fake backend and a signal relay.

#![cfg(feature = "in-process")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::path::Path;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use p256::ecdsa::SigningKey;
use rifts_admission::KeyId;
use rifts_admission::ticket::{Es256Tickets, Jti, SessionUuid, TicketBindings, TicketClaims};
use rifts_host::{
    CommonBootstrapConfig, Handle, HostBackend, HostBootstrapSpec, HostError, HostOpen, HostedRole, HubLimits,
    IcePeerEntry, InProcessTransport, JoinBootstrapSpec, JoinOpen, LocalRealmHosting, LocalState,
    NativeTransportLoader, NoopServerSpawner, PeerListEntry, ReadyState, RealmHosting, SessionResult, Signal,
    TransportAbi, TransportConfig, TransportLoader, sha256_file, wire_session_id,
};
use rifts_seam::{AccountId, GlobalId, HubKind, NodeId, PeerId, Secret, SessionId, Ulid, poll_once};

const PAIR_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const CAPABILITY: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";
const HOST: &str = "host-account";
const JOINER: &str = "join-account";

#[test]
fn the_loader_hands_out_the_linked_transport() {
    let transport = NativeTransportLoader.load(Path::new("nowhere.dll"), [0; 32]).unwrap();
    assert_eq!(transport.abi_version().unwrap(), 3);
    assert_eq!(
        transport.peer_id(),
        Err(HostError::Transport(
            "transport peer id is 0 (not initialized)".to_owned()
        ))
    );
    assert!(!transport.networking_ready().unwrap());
    transport.initialize().unwrap();
    assert_ne!(transport.peer_id().unwrap(), PeerId(0));
    assert!(transport.networking_ready().unwrap());

    let unknown = Handle(99);
    assert_eq!(transport.wait_ready(unknown, 5).unwrap(), ReadyState::Ended);
    assert!(!transport.session_running(unknown).unwrap());
    assert_eq!(
        transport.last_session_result(unknown).unwrap(),
        SessionResult::UnknownHandle
    );
    assert_eq!(
        transport.stop_session(unknown),
        Err(HostError::NotFound("unknown session handle 99".to_owned()))
    );
    assert_eq!(transport.read_signal(unknown).unwrap(), None);
    assert_eq!(transport.read_peer_event(unknown).unwrap(), None);
    assert_eq!(transport.read_diagnostic(unknown).unwrap(), None);
    let account = AccountId("peer-acct-1".to_owned());
    assert_eq!(
        transport.submit_signal(unknown, &account, "DESCRIPTION", "v=0\r\n"),
        Err(HostError::Transport("transport refused incoming signal".to_owned()))
    );
    assert_eq!(
        transport.submit_signal(unknown, &account, "DESCRIPTION", "a\0b"),
        Err(HostError::InvalidInput("data contains an interior NUL byte".to_owned()))
    );
    let key = Secret::new(PAIR_KEY.to_owned());
    assert_eq!(
        transport.add_peer(unknown, &account, PeerId(5), &key),
        Err(HostError::Transport("transport refused peer addition".to_owned()))
    );
    assert_eq!(
        transport.remove_peer(unknown, &account, "roster"),
        Err(HostError::Transport("transport refused peer removal".to_owned()))
    );
    assert_eq!(transport.update_relay_credential(unknown, "u", &key), Ok(()));
    assert_eq!(
        transport.update_relay_credential(unknown, "u\0", &key),
        Err(HostError::InvalidInput(
            "username contains an interior NUL byte".to_owned()
        ))
    );

    let dir = tempfile::tempdir().unwrap();
    let file = |name: &str| dir.path().join(name);
    let handle = transport
        .host_open(&HostOpen {
            public_key: file("k"),
            config: file("c"),
            capability: file("cap"),
            host_ticket: None,
            secrets: file("s"),
        })
        .unwrap();
    assert!(handle.0 > 0);
    assert_eq!(transport.wait_ready(handle, 10_000).unwrap(), ReadyState::Ended);
    assert_eq!(transport.last_session_result(handle).unwrap(), SessionResult::BadInput);
    let mut diagnostics = Vec::new();
    while let Some(line) = transport.read_diagnostic(handle).unwrap() {
        diagnostics.push(line);
    }
    assert!(
        diagnostics.iter().any(|l| l.starts_with("HOST CONFIG FAIL")),
        "{diagnostics:?}"
    );
    transport.stop_session(handle).unwrap();
    assert!(
        transport
            .join_open(&JoinOpen {
                public_key: file("k"),
                config: file("c"),
                ticket: Path::new("").to_owned(),
                secrets: file("s"),
            })
            .is_err()
    );
    transport.shutdown().unwrap();
}

#[test]
fn file_digests() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.bin");
    std::fs::write(&path, b"abc").unwrap();
    assert_eq!(
        sha256_file(&path).unwrap()[..4],
        [0xba, 0x78, 0x16, 0xbf],
        "SHA-256 of abc"
    );
    std::fs::write(&path, b"").unwrap();
    assert!(matches!(sha256_file(&path), Err(HostError::InvalidInput(_))));
    assert!(matches!(sha256_file(dir.path()), Err(HostError::InvalidInput(_))));
    assert!(matches!(sha256_file(&dir.path().join("none")), Err(HostError::Io(_))));
}

/// A backend on 127.0.0.1 over plain HTTP, which the transport allows on loopback: host-ready, consume, lease and
/// disconnect, each answered as a healthy backend would.
struct HttpBackend {
    url: String,
    consumes: Arc<AtomicUsize>,
}

fn answer(stream: TcpStream, consumes: &AtomicUsize) {
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    let path = first.split_whitespace().nth(1).unwrap_or_default().to_owned();
    let mut length = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let (status, text) = if path.ends_with("/tickets/consume") {
        let n = consumes.fetch_add(1, Ordering::SeqCst) + 1;
        (
            "200 OK",
            format!("{{\"admitted\":true,\"admissionId\":\"{n:064x}\",\"idempotent\":false}}"),
        )
    } else if path.ends_with("/host-ready") {
        ("200 OK", "{\"ready\":true}".to_owned())
    } else if path.ends_with("/lease") {
        ("200 OK", "{\"renewed\":true,\"state\":\"RUNNING\"}".to_owned())
    } else if path.ends_with("/disconnect") {
        ("200 OK", "{}".to_owned())
    } else {
        ("404 Not Found", "{}".to_owned())
    };
    let mut stream = reader.into_inner();
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
        text.len()
    );
}

impl HttpBackend {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let consumes = Arc::new(AtomicUsize::new(0));
        let counter = consumes.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let counter = counter.clone();
                thread::spawn(move || answer(stream, &counter));
            }
        });
        Self { url, consumes }
    }
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// The realm backend both launchers share: it mints tickets, lists the joiner on the hub's roster and carries each
/// signal to the account it names.
struct RelayBackend {
    tickets: Es256Tickets,
    url: String,
    session: SessionId,
    server_port: u16,
    inbox: Mutex<HashMap<String, Vec<(AccountId, Signal)>>>,
    routes: Mutex<Vec<(String, u16)>>,
}

impl RelayBackend {
    fn common(&self) -> CommonBootstrapConfig {
        CommonBootstrapConfig {
            key_id: "test-key".to_owned(),
            session_id: self.session.clone(),
            party_id: "party-1".to_owned(),
            party_revision: 1,
            hunt_definition_id: "hunt:def/1".to_owned(),
            generation: 1,
            build: 7,
            bridge_protocol: 3,
            virtual_port: 7,
            host_peer_id: "1000".to_owned(),
            bridge_instance_id: "bridge-1".to_owned(),
            metadata_nonce: "nonce-1".to_owned(),
            force_relay: false,
            guard_completion_mode: "NO_COMPLETION".to_owned(),
        }
    }

    fn mint(&self, account: &str, joiner: u64) -> Secret {
        let now = now_seconds();
        let claims = TicketClaims {
            key_id: KeyId("test-key".to_owned()),
            jti: Jti::random().unwrap(),
            bindings: TicketBindings {
                account: AccountId(account.to_owned()),
                joiner_peer: PeerId(joiner),
                host_peer: PeerId(1000),
                session: SessionUuid::from_str(&wire_session_id(&self.session)).unwrap(),
                generation: 1,
                party_id: "party-1".to_owned(),
                party_revision: 1,
                hunt_definition_id: "hunt:def/1".to_owned(),
                build: 7,
                bridge_protocol: 3,
                virtual_port: 7,
            },
            issued_at: now,
            not_before: now,
            expires_at: now + 120,
        };
        self.tickets.sign(claims).unwrap().text
    }

    fn config(local: &str, local_peer: &str, peers: Vec<IcePeerEntry>) -> TransportConfig {
        TransportConfig {
            generation: 1,
            local_account_id: local.to_owned(),
            local_peer_id: local_peer.to_owned(),
            host_account_id: HOST.to_owned(),
            host_peer_id: "1000".to_owned(),
            ice_servers: Vec::new(),
            peers,
            relay: None,
        }
    }
}

impl HostBackend for RelayBackend {
    fn host_bootstrap(
        &self,
        _a: &AccountId,
        _s: &SessionId,
    ) -> Result<(HostBootstrapSpec, TransportConfig), HostError> {
        Err(HostError::NotFound("hubs only".to_owned()))
    }

    fn join_bootstrap(
        &self,
        account: &AccountId,
        _session: &SessionId,
    ) -> Result<(JoinBootstrapSpec, TransportConfig), HostError> {
        let spec = JoinBootstrapSpec {
            common: self.common(),
            role: HostedRole::Hub(HubKind::Ramsgate),
            ticket: self.mint(&account.0, 2000),
        };
        let host = IcePeerEntry {
            account_id: HOST.to_owned(),
            peer_id: "1000".to_owned(),
            transport_key: PAIR_KEY.to_owned(),
        };
        Ok((spec, Self::config(&account.0, "2000", vec![host])))
    }

    fn start_hub(
        &self,
        account: &AccountId,
        kind: HubKind,
        limits: &HubLimits,
    ) -> Result<(HostBootstrapSpec, TransportConfig), HostError> {
        let spec = HostBootstrapSpec {
            common: self.common(),
            role: HostedRole::Hub(kind),
            capability: Secret::new(CAPABILITY.to_owned()),
            host_ticket: Some(self.mint(&account.0, 1000)),
            backend_url: self.url.clone(),
            server_udp_port: self.server_port,
            maximum_peers: limits.max_players,
            local_client: true,
            hunt_evidence: false,
            matchmaker_hunt_id: "ShatteredIsles_ReturnToRamsgate".to_owned(),
            map_path: "/Game/Maps/Ramsgate".to_owned(),
            behemoth_path: "None".to_owned(),
            expected_players: limits.max_players.to_string(),
            runtime_profile: "production".to_owned(),
            max_players: Some(limits.max_players),
            hub_afk_seconds: Some(limits.afk_seconds),
        };
        Ok((spec, Self::config(&account.0, "1000", Vec::new())))
    }

    fn stop_hub(&self, _account: &AccountId, _kind: HubKind) -> Result<Option<SessionId>, HostError> {
        Ok(Some(self.session.clone()))
    }

    fn route_ready(&self, account: &AccountId, _s: &SessionId, _g: u64, port: u16) -> Result<(), HostError> {
        self.routes.lock().unwrap().push((account.0.clone(), port));
        Ok(())
    }

    fn supports_signalling(&self) -> bool {
        true
    }

    fn poll_peers(
        &self,
        _account: &AccountId,
        _session: &SessionId,
        _generation: u64,
        role: HostedRole,
    ) -> Result<Vec<PeerListEntry>, HostError> {
        assert_eq!(role, HostedRole::Hub(HubKind::Ramsgate));
        Ok(vec![PeerListEntry {
            account_id: JOINER.to_owned(),
            peer_id: "2000".to_owned(),
            transport_key: PAIR_KEY.to_owned(),
        }])
    }

    fn send_signal(&self, account: &AccountId, _s: &SessionId, _g: u64, signal: &Signal) -> Result<(), HostError> {
        let incoming = Signal {
            to_account: AccountId(String::new()),
            ..signal.clone()
        };
        self.inbox
            .lock()
            .unwrap()
            .entry(signal.to_account.0.clone())
            .or_default()
            .push((account.clone(), incoming));
        Ok(())
    }

    fn poll_signals(
        &self,
        account: &AccountId,
        _s: &SessionId,
        _g: u64,
    ) -> Result<Vec<(AccountId, Signal)>, HostError> {
        Ok(self.inbox.lock().unwrap().remove(&account.0).unwrap_or_default())
    }
}

#[test]
fn a_hub_hosted_and_joined_through_the_real_transport() {
    let mut seed = [0_u8; 32];
    for (i, byte) in seed.iter_mut().enumerate() {
        *byte = (i as u8).wrapping_mul(29).wrapping_add(3);
    }
    let signing = SigningKey::from_slice(&seed).unwrap();
    let verifying = *signing.verifying_key();
    let game = tempfile::tempdir().unwrap();
    std::fs::write(
        game.path().join(rifts_host::ADMISSION_PUBLIC_KEY_FILE),
        rifts_admission::keys::public_xy(&verifying).0,
    )
    .unwrap();

    // The ticket's session must be a UUID (version 4, variant 8 here).
    let uuid = 0x3f1c_2a4e_1b2c_4d3e_8f40_0000_0000_0001_u128;
    let session = SessionId(GlobalId {
        ulid: Ulid(uuid),
        node: NodeId::new("local").unwrap(),
    });
    let http = HttpBackend::start();
    let game_server = UdpSocket::bind("127.0.0.1:0").unwrap();
    let backend = Arc::new(RelayBackend {
        tickets: Es256Tickets {
            key_id: KeyId("test-key".to_owned()),
            signing: Some(signing),
            verifying,
            default_lifetime: 120.0,
        },
        url: http.url.clone(),
        session: session.clone(),
        server_port: game_server.local_addr().unwrap().port(),
        inbox: Mutex::default(),
        routes: Mutex::default(),
    });

    let launcher = |data: &Path| {
        let transport = Arc::new(InProcessTransport::new());
        transport.initialize().unwrap();
        LocalRealmHosting::with_backend_and_spawner(
            data,
            game.path(),
            transport,
            backend.clone(),
            Arc::new(NoopServerSpawner::new()),
        )
        .with_wait_ready_timeout_ms(30_000)
    };
    let host_data = tempfile::tempdir().unwrap();
    let join_data = tempfile::tempdir().unwrap();
    let host = launcher(host_data.path());
    let joiner = launcher(join_data.path());
    assert_eq!(poll_once(host.coop_available()).unwrap(), Ok(true));

    let limits = HubLimits {
        max_players: 4,
        max_servers: 1,
        afk_seconds: 0,
    };
    let hub = poll_once(host.start_hub(AccountId(HOST.to_owned()), HubKind::Ramsgate, limits))
        .unwrap()
        .unwrap();
    assert_eq!(hub.state, LocalState::Running);
    assert_ne!(host.session_local_port(&session).unwrap(), 0);

    let joined = poll_once(joiner.join_hunt(AccountId(JOINER.to_owned()), session.clone()))
        .unwrap()
        .unwrap();
    assert!(!joined.hosting);
    assert_eq!(joined.role, HostedRole::Hub(HubKind::Ramsgate));
    let route = joiner.session_local_port(&session).unwrap();
    assert_ne!(route, 0);
    let routes = backend.routes.lock().unwrap().clone();
    assert!(routes.iter().any(|(account, _)| account == HOST));
    assert!(routes.contains(&(JOINER.to_owned(), route)));
    assert_eq!(
        http.consumes.load(Ordering::SeqCst),
        2,
        "the host's own ticket and the joiner's"
    );

    let sessions = poll_once(host.sessions()).unwrap().unwrap();
    assert_eq!(sessions[0].state, LocalState::Running);
    let host_dir = host.session_directory(&session).unwrap();
    let join_dir = joiner.session_directory(&session).unwrap();
    poll_once(joiner.stop(session.clone())).unwrap().unwrap();
    poll_once(host.stop_hub(AccountId(HOST.to_owned()), HubKind::Ramsgate))
        .unwrap()
        .unwrap();
    assert!(!host_dir.exists() && !join_dir.exists());
}
