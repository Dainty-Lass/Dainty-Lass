// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The two ports a hosting flow calls out through: the backend (the launcher's HTTP client, which holds the account
//! key) and the spawner that starts the hosted game server.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use rifts_seam::{AccountId, GlobalId, HubKind, NodeId, Secret, SessionId, Ulid};

use super::rules::wire_session_id;
use super::{
    CommonBootstrapConfig, HostBootstrapSpec, HostedRole, HubLimits, IcePeerEntry, IceServerEntry, JoinBootstrapSpec,
    PeerListEntry, RelayGrant, TransportConfig,
};
use crate::HostError;
use crate::files::validate_hub_limits as validate_limits;
use crate::transport::Signal;

/// Synchronous; the launcher's implementation blocks on its own runtime.
pub trait HostBackend: Send + Sync {
    fn host_bootstrap(
        &self,
        account: &AccountId,
        session: &SessionId,
    ) -> Result<(HostBootstrapSpec, TransportConfig), HostError>;
    fn join_bootstrap(
        &self,
        account: &AccountId,
        session: &SessionId,
    ) -> Result<(JoinBootstrapSpec, TransportConfig), HostError>;
    fn start_hub(
        &self,
        account: &AccountId,
        kind: HubKind,
        limits: &HubLimits,
    ) -> Result<(HostBootstrapSpec, TransportConfig), HostError>;
    fn stop_hub(&self, account: &AccountId, kind: HubKind) -> Result<Option<SessionId>, HostError>;
    fn route_ready(
        &self,
        account: &AccountId,
        session: &SessionId,
        generation: u64,
        local_port: u16,
    ) -> Result<(), HostError>;

    fn supports_signalling(&self) -> bool {
        false
    }

    fn poll_peers(
        &self,
        _account: &AccountId,
        _session: &SessionId,
        _generation: u64,
        _role: HostedRole,
    ) -> Result<Vec<PeerListEntry>, HostError> {
        Ok(Vec::new())
    }

    fn send_signal(
        &self,
        _account: &AccountId,
        _session: &SessionId,
        _generation: u64,
        _signal: &Signal,
    ) -> Result<(), HostError> {
        Ok(())
    }

    /// Incoming signals with the sender; their `to_account` is not read.
    fn poll_signals(
        &self,
        _account: &AccountId,
        _session: &SessionId,
        _generation: u64,
    ) -> Result<Vec<(AccountId, Signal)>, HostError> {
        Ok(Vec::new())
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A deterministic local backend: the launcher's fallback with no server configured, and its tests'.
#[derive(Debug, Default)]
pub struct DefaultHostBackend {
    hubs: Mutex<HashMap<(AccountId, HubKind), SessionId>>,
    routes: Mutex<HashMap<SessionId, u16>>,
}

const LOCAL_HOST_PEER: &str = "100001";
const LOCAL_JOIN_PEER: &str = "200002";
const LOCAL_HOST_ACCOUNT: &str = "host-account-1";
const LOCAL_BACKEND_URL: &str = "https://127.0.0.1:8443";
// 2026-10-14, so every hub id the local backend makes sorts together.
const HUB_ID_MILLIS: u64 = 1_792_000_000_000;

fn a_run(prefix: &str, count: usize) -> String {
    format!("{prefix}{}", "A".repeat(count))
}

impl DefaultHostBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn recorded_route(&self, session: &SessionId) -> Option<u16> {
        locked(&self.routes).get(session).copied()
    }

    fn common(session: &SessionId, hunt_definition_id: &str, completion: &str) -> CommonBootstrapConfig {
        let wire = wire_session_id(session);
        CommonBootstrapConfig {
            key_id: "local-key-1".to_owned(),
            session_id: session.clone(),
            party_id: format!("party-{wire}"),
            party_revision: 1,
            hunt_definition_id: hunt_definition_id.to_owned(),
            generation: 1,
            build: 647_472,
            bridge_protocol: 1,
            virtual_port: 1,
            host_peer_id: LOCAL_HOST_PEER.to_owned(),
            bridge_instance_id: format!("bridge-{wire}"),
            metadata_nonce: "0123456789abcdef0123456789abcdef".to_owned(),
            force_relay: false,
            guard_completion_mode: completion.to_owned(),
        }
    }

    fn transport_config(account: &AccountId, joiner: bool) -> TransportConfig {
        TransportConfig {
            generation: 1,
            local_account_id: account.0.clone(),
            local_peer_id: if joiner { LOCAL_JOIN_PEER } else { LOCAL_HOST_PEER }.to_owned(),
            host_account_id: if joiner {
                LOCAL_HOST_ACCOUNT.to_owned()
            } else {
                account.0.clone()
            },
            host_peer_id: LOCAL_HOST_PEER.to_owned(),
            ice_servers: vec![
                IceServerEntry {
                    urls: vec!["stun:stun.cloudflare.com:3478".to_owned()],
                    username: None,
                    credential: None,
                },
                IceServerEntry {
                    urls: vec!["turn:127.0.0.1:3478?transport=udp".to_owned()],
                    username: Some("1700000000:local".to_owned()),
                    credential: Some("local-turn-secret".to_owned()),
                },
            ],
            peers: if joiner {
                vec![IcePeerEntry {
                    account_id: LOCAL_HOST_ACCOUNT.to_owned(),
                    peer_id: LOCAL_HOST_PEER.to_owned(),
                    transport_key: "A".repeat(43),
                }]
            } else {
                Vec::new()
            },
            relay: Some(RelayGrant {
                allowed: true,
                expires_at: Some("2026-10-28T00:00:00Z".to_owned()),
            }),
        }
    }

    fn hub_session(account: &AccountId, kind: HubKind) -> Result<SessionId, HostError> {
        let k: u128 = match kind {
            HubKind::Ramsgate => 1,
            HubKind::Training => 2,
        };
        let h = account
            .0
            .bytes()
            .fold(0_u64, |h, b| h.wrapping_mul(131).wrapping_add(u64::from(b)));
        let node = NodeId::new("local").map_err(|e| HostError::InvalidInput(e.to_string()))?;
        Ok(SessionId(GlobalId {
            ulid: Ulid::from_parts(HUB_ID_MILLIS, (k << 64) | u128::from(h)),
            node,
        }))
    }
}

impl HostBackend for DefaultHostBackend {
    fn host_bootstrap(
        &self,
        account: &AccountId,
        session: &SessionId,
    ) -> Result<(HostBootstrapSpec, TransportConfig), HostError> {
        let spec = HostBootstrapSpec {
            common: Self::common(session, "Hunt_Default", "FIRST_DEFEAT"),
            role: HostedRole::Hunt,
            capability: Secret::new(a_run("cap_", 39)),
            host_ticket: Some(Secret::new("rpt1.ticket.signature".to_owned())),
            backend_url: LOCAL_BACKEND_URL.to_owned(),
            server_udp_port: 8791,
            maximum_peers: 4,
            local_client: true,
            hunt_evidence: false,
            matchmaker_hunt_id: "Arena_MatchmakerHunt_Easy_001".to_owned(),
            map_path: "/Game/Maps/HuntMap".to_owned(),
            behemoth_path: "Behemoth_Gnasher".to_owned(),
            expected_players: "1".to_owned(),
            runtime_profile: "production".to_owned(),
            max_players: None,
            hub_afk_seconds: None,
        };
        Ok((spec, Self::transport_config(account, false)))
    }

    fn join_bootstrap(
        &self,
        account: &AccountId,
        session: &SessionId,
    ) -> Result<(JoinBootstrapSpec, TransportConfig), HostError> {
        let spec = JoinBootstrapSpec {
            common: Self::common(session, "Hunt_Default", "FIRST_DEFEAT"),
            role: HostedRole::Hunt,
            ticket: Secret::new("rpt1.joiner_ticket.signature".to_owned()),
        };
        Ok((spec, Self::transport_config(account, true)))
    }

    fn start_hub(
        &self,
        account: &AccountId,
        kind: HubKind,
        limits: &HubLimits,
    ) -> Result<(HostBootstrapSpec, TransportConfig), HostError> {
        validate_limits(limits)?;
        let session = Self::hub_session(account, kind)?;
        locked(&self.hubs).insert((account.clone(), kind), session.clone());
        let spec = HostBootstrapSpec {
            common: Self::common(&session, "ShatteredIsles_ReturnToRamsgate", "NO_COMPLETION"),
            role: HostedRole::Hub(kind),
            capability: Secret::new(a_run("cap_HUB_", 35)),
            host_ticket: Some(Secret::new("rpt1.hub_host_ticket.signature".to_owned())),
            backend_url: LOCAL_BACKEND_URL.to_owned(),
            server_udp_port: 8792,
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
        Ok((spec, Self::transport_config(account, false)))
    }

    fn stop_hub(&self, account: &AccountId, kind: HubKind) -> Result<Option<SessionId>, HostError> {
        Ok(locked(&self.hubs).remove(&(account.clone(), kind)))
    }

    fn route_ready(
        &self,
        _account: &AccountId,
        session: &SessionId,
        _generation: u64,
        local_port: u16,
    ) -> Result<(), HostError> {
        locked(&self.routes).insert(session.clone(), local_port);
        Ok(())
    }
}

/// Starts a process with exactly the environment given and nothing else (RD9: no injection).
pub trait ServerSpawner: Send + Sync {
    fn spawn(
        &self,
        program: &Path,
        arguments: &[String],
        working_dir: &Path,
        environment: &BTreeMap<String, String>,
    ) -> Result<u32, HostError>;
    fn stop(&self, pid: u32);
}

type Launch = (Vec<String>, BTreeMap<String, String>);

/// Records the launch and starts nothing; what the launcher ships until hosted servers run for real.
#[derive(Debug, Default)]
pub struct NoopServerSpawner {
    last: Mutex<Option<Launch>>,
}

const NOOP_PID: u32 = 4242;

impl NoopServerSpawner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last_environment(&self) -> Option<BTreeMap<String, String>> {
        locked(&self.last).as_ref().map(|(_, environment)| environment.clone())
    }

    pub fn last_arguments(&self) -> Option<Vec<String>> {
        locked(&self.last).as_ref().map(|(arguments, _)| arguments.clone())
    }
}

impl ServerSpawner for NoopServerSpawner {
    fn spawn(
        &self,
        _program: &Path,
        arguments: &[String],
        _working_dir: &Path,
        environment: &BTreeMap<String, String>,
    ) -> Result<u32, HostError> {
        *locked(&self.last) = Some((arguments.to_vec(), environment.clone()));
        Ok(NOOP_PID)
    }

    fn stop(&self, _pid: u32) {}
}
