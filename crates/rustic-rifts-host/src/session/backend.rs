// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The two ports a hosting flow calls out through: the backend (the launcher's HTTP client, which holds the account
//! key) and the spawner that starts the hosted game server.

use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::{Mutex, MutexGuard, PoisonError},
};

use rustic_rifts_seam::{AccountId, GlobalId, HubKind, NodeId, Secret, SessionId, Ulid};

use super::{
    CommonBootstrapConfig, HostBootstrapSpec, HostedRole, HubLimits, IcePeerEntry, IceServerEntry, JoinBootstrapSpec,
    PeerListEntry, RelayGrant, TransportConfig, rules::wire_session_id,
};
use crate::{HostError, files::validate_hub_limits, transport::Signal};

type Hosted = (HostBootstrapSpec, TransportConfig);

/// Synchronous; the launcher's implementation blocks on its own runtime.
///
/// Signalling is optional: a backend without it keeps the defaults, which list no peers and drop every signal.
pub trait HostBackend: Send + Sync {
    /// The host bootstrap and transport config for hosting this hunt.
    fn host_bootstrap(&self, account: &AccountId, session: &SessionId) -> Result<Hosted, HostError>;

    /// The joiner bootstrap and transport config for joining this hunt.
    fn join_bootstrap(
        &self,
        account: &AccountId,
        session: &SessionId,
    ) -> Result<(JoinBootstrapSpec, TransportConfig), HostError>;

    /// Starts a hub on the backend and returns its host bootstrap and transport config.
    fn start_hub(&self, account: &AccountId, kind: HubKind, limits: &HubLimits) -> Result<Hosted, HostError>;

    /// Stops this account's hub of this kind; the session it named, if any.
    fn stop_hub(&self, account: &AccountId, kind: HubKind) -> Result<Option<SessionId>, HostError>;

    /// Tells the backend the session's local route is ready on `port`.
    fn route_ready(
        &self,
        account: &AccountId,
        session: &SessionId,
        generation: u64,
        port: u16,
    ) -> Result<(), HostError>;

    /// Whether the backend relays peers and signals; the default says no.
    fn supports_signalling(&self) -> bool {
        false
    }

    /// The peers the session should link with now; the default lists none.
    fn poll_peers(
        &self,
        account: &AccountId,
        session: &SessionId,
        generation: u64,
        role: HostedRole,
    ) -> Result<Vec<PeerListEntry>, HostError> {
        let _ = (account, session, generation, role);
        Ok(vec![])
    }

    /// Relays one signal to a peer; the default drops it.
    fn send_signal(
        &self,
        account: &AccountId,
        session: &SessionId,
        generation: u64,
        signal: &Signal,
    ) -> Result<(), HostError> {
        let _ = (account, session, generation, signal);
        Ok(())
    }

    /// Incoming signals with the sender; their `to_account` is not read.
    fn poll_signals(
        &self,
        account: &AccountId,
        session: &SessionId,
        generation: u64,
    ) -> Result<Vec<(AccountId, Signal)>, HostError> {
        let _ = (account, session, generation);
        Ok(vec![])
    }
}

/// A poisoned lock still holds consistent maps: every critical section is one insert or remove.
fn held<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A deterministic local backend: the launcher's fallback with no server configured, and its tests'.
#[derive(Debug, Default)]
pub struct DefaultHostBackend {
    hub_sessions: Mutex<HashMap<(AccountId, HubKind), SessionId>>,
    ready_ports: Mutex<HashMap<SessionId, u16>>,
}

mod local {
    pub const HOST_PEER: &str = "100001";
    pub const JOIN_PEER: &str = "200002";
    pub const HOST_ACCOUNT: &str = "host-account-1";
    pub const BACKEND_URL: &str = "https://127.0.0.1:8443";
    // 2026-10-14, so every hub id the local backend makes sorts together.
    pub const HUB_ID_MILLIS: u64 = 1_792_000_000_000;
}

/// What differs between the local backend's hunt and hub hosts.
struct HostTemplate {
    hunt_definition_id: &'static str,
    completion: &'static str,
    capability_prefix: &'static str,
    capability_run: usize,
    host_ticket: &'static str,
    server_udp_port: u16,
    matchmaker_hunt_id: &'static str,
    map_path: &'static str,
    behemoth_path: &'static str,
}

const HUNT_HOST: HostTemplate = HostTemplate {
    hunt_definition_id: "Hunt_Default",
    completion: "FIRST_DEFEAT",
    capability_prefix: "cap_",
    capability_run: 39,
    host_ticket: "rpt1.ticket.signature",
    server_udp_port: 8791,
    matchmaker_hunt_id: "Arena_MatchmakerHunt_Easy_001",
    map_path: "/Game/Maps/HuntMap",
    behemoth_path: "Behemoth_Gnasher",
};

const HUB_HOST: HostTemplate = HostTemplate {
    hunt_definition_id: "ShatteredIsles_ReturnToRamsgate",
    completion: "NO_COMPLETION",
    capability_prefix: "cap_HUB_",
    capability_run: 35,
    host_ticket: "rpt1.hub_host_ticket.signature",
    server_udp_port: 8792,
    matchmaker_hunt_id: "ShatteredIsles_ReturnToRamsgate",
    map_path: "/Game/Maps/Ramsgate",
    behemoth_path: "None",
};

impl HostTemplate {
    fn common(&self, session: &SessionId) -> CommonBootstrapConfig {
        let wire = wire_session_id(session);
        CommonBootstrapConfig {
            key_id: "local-key-1".into(),
            session_id: session.clone(),
            party_id: format!("party-{wire}"),
            party_revision: 1,
            hunt_definition_id: self.hunt_definition_id.into(),
            generation: 1,
            build: 647_472,
            bridge_protocol: 1,
            virtual_port: 1,
            host_peer_id: local::HOST_PEER.into(),
            bridge_instance_id: format!("bridge-{wire}"),
            metadata_nonce: "0123456789abcdef".repeat(2),
            force_relay: false,
            guard_completion_mode: self.completion.into(),
        }
    }

    /// A hunt host when `hub` is `None`; a hub host sizes itself from the limits.
    fn host(&self, session: &SessionId, hub: Option<(HubKind, &HubLimits)>) -> HostBootstrapSpec {
        let players = hub.map(|(_, limits)| limits.max_players);
        HostBootstrapSpec {
            common: self.common(session),
            role: hub.map_or(HostedRole::Hunt, |(kind, _)| HostedRole::Hub(kind)),
            capability: Secret::new(self.capability_prefix.to_owned() + &"A".repeat(self.capability_run)),
            host_ticket: Some(Secret::new(self.host_ticket.into())),
            backend_url: local::BACKEND_URL.into(),
            server_udp_port: self.server_udp_port,
            maximum_peers: players.unwrap_or(4),
            local_client: true,
            hunt_evidence: false,
            matchmaker_hunt_id: self.matchmaker_hunt_id.into(),
            map_path: self.map_path.into(),
            behemoth_path: self.behemoth_path.into(),
            expected_players: players.unwrap_or(1).to_string(),
            runtime_profile: "production".into(),
            max_players: players,
            hub_afk_seconds: hub.map(|(_, limits)| limits.afk_seconds),
        }
    }
}

fn ice_server(url: &str, turn: Option<(&str, &str)>) -> IceServerEntry {
    IceServerEntry {
        urls: vec![url.into()],
        username: turn.map(|(user, _)| user.into()),
        credential: turn.map(|(_, secret)| secret.into()),
    }
}

/// The joiner links to the host alone; a host starts with nobody.
fn local_transport_config(account: &AccountId, joiner: bool) -> TransportConfig {
    let host_account = if joiner {
        local::HOST_ACCOUNT.to_owned()
    } else {
        account.0.clone()
    };
    let peers = joiner.then(|| IcePeerEntry {
        account_id: local::HOST_ACCOUNT.into(),
        peer_id: local::HOST_PEER.into(),
        transport_key: "A".repeat(43),
    });
    TransportConfig {
        generation: 1,
        local_account_id: account.0.clone(),
        local_peer_id: (if joiner { local::JOIN_PEER } else { local::HOST_PEER }).into(),
        host_account_id: host_account,
        host_peer_id: local::HOST_PEER.into(),
        ice_servers: vec![
            ice_server("stun:stun.cloudflare.com:3478", None),
            ice_server(
                "turn:127.0.0.1:3478?transport=udp",
                Some(("1700000000:local", "local-turn-secret")),
            ),
        ],
        peers: peers.into_iter().collect(),
        relay: Some(RelayGrant {
            allowed: true,
            expires_at: Some("2026-10-28T00:00:00Z".into()),
        }),
    }
}

/// The same account and kind always get the same hub id.
fn local_hub_session(account: &AccountId, kind: HubKind) -> Result<SessionId, HostError> {
    let kind_bits: u128 = if kind == HubKind::Training { 2 } else { 1 };
    let mut hash = 0_u64;
    for byte in account.0.bytes() {
        hash = hash.wrapping_mul(131).wrapping_add(byte.into());
    }
    let ulid = Ulid::from_parts(local::HUB_ID_MILLIS, kind_bits << 64 | u128::from(hash));
    let node = NodeId::new("local").map_err(|e| HostError::InvalidInput(e.to_string()))?;
    Ok(SessionId(GlobalId { ulid, node }))
}

impl DefaultHostBackend {
    /// A backend with no hubs and no routes recorded.
    pub fn new() -> Self {
        Default::default()
    }

    /// The port a session last reported ready on.
    pub fn recorded_route(&self, session: &SessionId) -> Option<u16> {
        held(&self.ready_ports).get(session).copied()
    }
}

impl HostBackend for DefaultHostBackend {
    fn host_bootstrap(&self, account: &AccountId, session: &SessionId) -> Result<Hosted, HostError> {
        Ok((HUNT_HOST.host(session, None), local_transport_config(account, false)))
    }

    fn join_bootstrap(
        &self,
        account: &AccountId,
        session: &SessionId,
    ) -> Result<(JoinBootstrapSpec, TransportConfig), HostError> {
        let join = JoinBootstrapSpec {
            common: HUNT_HOST.common(session),
            role: HostedRole::Hunt,
            ticket: Secret::new("rpt1.joiner_ticket.signature".into()),
        };
        Ok((join, local_transport_config(account, true)))
    }

    fn start_hub(&self, account: &AccountId, kind: HubKind, limits: &HubLimits) -> Result<Hosted, HostError> {
        validate_hub_limits(limits)?;
        let session = local_hub_session(account, kind)?;
        let spec = HUB_HOST.host(&session, Some((kind, limits)));
        held(&self.hub_sessions).insert((account.clone(), kind), session);
        Ok((spec, local_transport_config(account, false)))
    }

    fn stop_hub(&self, account: &AccountId, kind: HubKind) -> Result<Option<SessionId>, HostError> {
        let key = (account.clone(), kind);
        Ok(held(&self.hub_sessions).remove(&key))
    }

    fn route_ready(&self, _: &AccountId, session: &SessionId, _: u64, port: u16) -> Result<(), HostError> {
        held(&self.ready_ports).insert(session.clone(), port);
        Ok(())
    }
}

/// Starts the hosted server with exactly the environment given; the launcher's injects only into it (RD9).
pub trait ServerSpawner: Send + Sync {
    /// Starts `program` and returns its process id.
    fn spawn(
        &self,
        program: &Path,
        arguments: &[String],
        working_dir: &Path,
        environment: &BTreeMap<String, String>,
    ) -> Result<u32, HostError>;

    /// Waits until the server started under `pid` reports ready on its ready pipe; the default has no pipe.
    fn wait_ready(&self, pid: u32) -> Result<(), HostError> {
        let _ = pid;
        Ok(())
    }

    /// Stops the process started under `pid`: fenced, or ended when the fence fails.
    fn stop(&self, pid: u32);
}

/// Records the launch and starts nothing; what the launcher ships until hosted servers run for real.
#[derive(Debug, Default)]
pub struct NoopServerSpawner {
    last: Mutex<Option<Launch>>,
}

#[derive(Clone, Debug)]
struct Launch {
    arguments: Vec<String>,
    environment: BTreeMap<String, String>,
}

impl NoopServerSpawner {
    // The pid every pretend launch reports.
    const PID: u32 = 4242;

    /// A spawner with nothing recorded.
    pub fn new() -> Self {
        Self::default()
    }

    /// The environment of the last launch.
    pub fn last_environment(&self) -> Option<BTreeMap<String, String>> {
        held(&self.last).clone().map(|launch| launch.environment)
    }

    /// The arguments of the last launch.
    pub fn last_arguments(&self) -> Option<Vec<String>> {
        held(&self.last).clone().map(|launch| launch.arguments)
    }
}

impl ServerSpawner for NoopServerSpawner {
    fn spawn(
        &self,
        _: &Path,
        arguments: &[String],
        _: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<u32, HostError> {
        let launch = Launch {
            arguments: arguments.to_vec(),
            environment: env.clone(),
        };
        held(&self.last).replace(launch);
        Ok(Self::PID)
    }

    fn stop(&self, _: u32) {}
}
