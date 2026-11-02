// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Realm sessions on the launcher: the backend and spawner ports, the session folder, the hosting and joining flows
//! and the pure rules they and the launcher's hosting panel use.

mod backend;
mod folder;
mod hosting;
mod rules;

use std::collections::BTreeMap;
use std::time::Duration;

use rifts_seam::{HubKind, Secret, SessionId};
use serde::{Deserialize, Serialize};

pub use self::backend::{DefaultHostBackend, HostBackend, NoopServerSpawner, ServerSpawner};
pub use self::folder::{cleanup_directory, create_session_directory, write_session_file_0600};
pub use self::hosting::{LocalRealmHosting, RealmHosting};
pub use self::rules::{
    apply_peer_event, apply_peer_line, attempts_in_window, build_launch_environment, capacity_decision,
    default_max_servers, host_server_arguments, hunt_join_leave_outcome, ice_files, is_local_admission,
    next_idle_since, parse_ice_ready, parse_peer_event, peer_diff, relay_refresh_due, relay_restart_due,
    restart_allowed, restart_backoff, safe_account, safe_config_value, safe_credential, safe_key, safe_peer_id,
    safe_url, sanitize_central_log_line, scheduled_restart_due, training_should_stop, trials_week, udp_stats_local_rx,
    wanted_peers, windows_command_line, wire_session_id,
};
use crate::HostError;
use crate::transport::SessionResult;

/// What the hosted game server is told (D3: the `RUSTIC_RIFTS_` prefix); stripped from the inherited environment.
pub const LAUNCH_VARIABLES: [&str; 17] = [
    "RUSTIC_RIFTS_AUTH_MODE",
    "RUSTIC_RIFTS_BIND_ADDRESS",
    "RUSTIC_RIFTS_HOST_RUNTIME_CAPABILITY",
    "RUSTIC_RIFTS_HOSTED_ROLE",
    "RUSTIC_RIFTS_SESSION_ID",
    "RUSTIC_RIFTS_SESSION_GENERATION",
    "RUSTIC_RIFTS_GAMESERVER_LAUNCH_ID",
    "RUSTIC_RIFTS_READY_PIPE",
    "RUSTIC_RIFTS_CONTROL_PIPE",
    "RUSTIC_RIFTS_GUARD_PIPE",
    "RUSTIC_RIFTS_HUNT_GUARD_EVENTS",
    "RUSTIC_RIFTS_HUNT_GUARD_COMPLETION_MODE",
    "RUSTIC_RIFTS_RUNTIME_PROFILE",
    "RUSTIC_RIFTS_LAUNCHER_PID",
    "RUSTIC_RIFTS_MAX_PLAYERS",
    "RUSTIC_RIFTS_HUB_AFK_SECONDS",
    "RUSTIC_RIFTS_TRIALS_WEEK",
];
/// Debug switches a hosted server must never inherit.
pub const CLEARED_VARIABLES: [&str; 2] = [
    "RUSTIC_RIFTS_ALLOW_DAMAGE_DEBUG",
    "RUSTIC_RIFTS_DEBUG_DAMAGE_MULTIPLIER",
];
pub const DIRECTOR_SERVER_SWITCHES: [&str; 3] = ["-server", "-log", "-nullrhi"];
pub const MAX_HUB_PEERS: usize = 64;
pub const MAX_HUNT_PEERS: usize = 4;
pub const HOST_COMMIT_FLOOR_MIB: u64 = 4096;
pub const HOST_MEMORY_FLOOR_MIB: u64 = 3072;
pub const GAME_LEFT_IDLE: Duration = Duration::from_secs(6);
pub const HUNT_JOIN_LEAVE_MAX: Duration = Duration::from_secs(120);
pub const RELAY_REFRESH_MARGIN_SECONDS: i64 = 1200;
pub const RELAY_EARLY_REFRESH_SECONDS: i64 = 43200;
pub const RELAY_EARLY_REFRESH_SPACING: Duration = Duration::from_secs(1800);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedRole {
    Hunt,
    Hub(HubKind),
}

impl HostedRole {
    pub const fn wire_role(self) -> &'static str {
        match self {
            Self::Hunt => "HUNT",
            Self::Hub(_) => "HUB",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalState {
    Starting,
    Ready,
    Running,
    Ended(SessionResult),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSession {
    pub session: SessionId,
    pub generation: u64,
    pub role: HostedRole,
    pub hosting: bool,
    pub state: LocalState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubLimits {
    pub max_players: u8,
    pub max_servers: u8,
    pub afk_seconds: u32,
}

impl Default for HubLimits {
    fn default() -> Self {
        Self {
            max_players: 16,
            max_servers: 2,
            afk_seconds: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IceServerEntry {
    pub urls: Vec<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub credential: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IcePeerEntry {
    pub account_id: String,
    pub peer_id: String,
    pub transport_key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayGrant {
    #[serde(default)]
    pub allowed: bool,
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// The backend's `transport-config` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportConfig {
    pub generation: u64,
    pub local_account_id: String,
    pub local_peer_id: String,
    pub host_account_id: String,
    pub host_peer_id: String,
    pub ice_servers: Vec<IceServerEntry>,
    pub peers: Vec<IcePeerEntry>,
    #[serde(default)]
    pub relay: Option<RelayGrant>,
}

impl TransportConfig {
    pub fn relay_expires_at(&self) -> Option<String> {
        self.relay
            .as_ref()
            .filter(|r| r.allowed)
            .and_then(|r| r.expires_at.clone())
    }

    /// The first ICE server with both a username and a credential, only when both are safe; no further look.
    pub fn relay_turn_credential(&self) -> Option<(String, Secret)> {
        let server = self
            .ice_servers
            .iter()
            .find(|s| s.username.is_some() && s.credential.is_some())?;
        let (user, credential) = (server.username.as_deref()?, server.credential.as_deref()?);
        (safe_credential(user) && safe_credential(credential))
            .then(|| (user.to_owned(), Secret::new(credential.to_owned())))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilesFor {
    Joiner,
    HuntHost,
    AwayHuntHost,
    HubHost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceFiles {
    pub config: String,
    pub secrets: Secret,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerEvent {
    Connected {
        account: String,
        route: Option<String>,
    },
    Admitted {
        account: String,
        route: Option<String>,
    },
    Lost {
        account: String,
        reason: Option<String>,
    },
    Removed {
        account: String,
        reason: Option<String>,
        admitted: bool,
    },
    AddRejected {
        account: String,
        reason: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerLive {
    pub route: Option<String>,
    pub since: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionLive {
    pub hunt_id: String,
    pub started_at: Option<String>,
    pub peers: BTreeMap<String, PeerLive>,
    pub relay_expires_at: Option<String>,
    pub local_since: Option<String>,
}

/// One entry of a reply's `peers`; peer ids are JSON strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerListEntry {
    pub account_id: String,
    pub peer_id: String,
    pub transport_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WantedPeer {
    pub account_id: String,
    pub peer_id: u64,
    pub transport_key: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PeerDiff {
    pub add: Vec<WantedPeer>,
    pub remove: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityDecision {
    Accept,
    Refuse { reason: &'static str },
}

/// When the game's local traffic last grew, from `UDP_STATS` lines.
#[derive(Debug, Clone)]
pub struct GameTrafficWatch {
    reading: Option<u64>,
    changed: std::time::Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchEnvironmentSpec {
    pub capability: Secret,
    pub role: HostedRole,
    pub session_id: SessionId,
    pub generation: u64,
    pub launch_id: String,
    pub ready_pipe: String,
    pub control_pipe: String,
    pub guard_pipe: String,
    pub hunt_evidence: bool,
    pub guard_completion_mode: String,
    pub runtime_profile: String,
    pub launcher_pid: u32,
    pub max_players: Option<u8>,
    pub hub_afk_seconds: Option<u32>,
    pub trials_week: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommonBootstrapConfig {
    pub key_id: String,
    pub session_id: SessionId,
    pub party_id: String,
    pub party_revision: u64,
    pub hunt_definition_id: String,
    pub generation: u64,
    pub build: u32,
    pub bridge_protocol: u8,
    pub virtual_port: u16,
    pub host_peer_id: String,
    pub bridge_instance_id: String,
    pub metadata_nonce: String,
    pub force_relay: bool,
    pub guard_completion_mode: String,
}

impl CommonBootstrapConfig {
    /// The first twelve lines of every `bridge.conf`, after checking each value that could break a line.
    pub fn to_config_lines(&self) -> Result<String, HostError> {
        let wire = wire_session_id(&self.session_id);
        let key_id = safe_config_value(&self.key_id, 32)?;
        let session = safe_config_value(&wire, 64)?;
        let party = safe_config_value(&self.party_id, 192)?;
        let hunt = safe_config_value(&self.hunt_definition_id, 128)?;
        safe_config_value(&self.guard_completion_mode, 32)?;
        let host_peer = safe_config_value(&self.host_peer_id, 20)?;
        let bridge = safe_config_value(&self.bridge_instance_id, 96)?;
        let nonce = safe_config_value(&self.metadata_nonce, 64)?;
        Ok(format!(
            "key_id={key_id}\nsession_id={session}\nparty_id={party}\nparty_revision={}\nhunt_definition_id={hunt}\n\
             generation={}\nbuild={}\nbridge_protocol={}\nvirtual_port={}\nhost_peer_id={host_peer}\n\
             bridge_instance_id={bridge}\nmetadata_nonce={nonce}\n",
            self.party_revision, self.generation, self.build, self.bridge_protocol, self.virtual_port,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostBootstrapSpec {
    pub common: CommonBootstrapConfig,
    pub role: HostedRole,
    pub capability: Secret,
    pub host_ticket: Option<Secret>,
    pub backend_url: String,
    pub server_udp_port: u16,
    pub maximum_peers: u8,
    pub local_client: bool,
    pub hunt_evidence: bool,
    pub matchmaker_hunt_id: String,
    pub map_path: String,
    pub behemoth_path: String,
    pub expected_players: String,
    pub runtime_profile: String,
    pub max_players: Option<u8>,
    pub hub_afk_seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinBootstrapSpec {
    pub common: CommonBootstrapConfig,
    pub role: HostedRole,
    pub ticket: Secret,
}
