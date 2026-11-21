// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Realm sessions on the launcher: the backend and spawner ports, the session folder, the hosting and joining flows
//! and the pure rules they and the launcher's hosting panel use.

// A launch variable's full name (D3).
macro_rules! rifts_var {
    ($suffix:literal) => {
        concat!("RUSTIC_RIFTS_", $suffix)
    };
}

mod rules;

mod backend;
mod folder;
mod hosting;

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use rifts_seam::{HubKind, Secret, SessionId};
use serde::{Deserialize, Serialize};

pub use self::backend::{DefaultHostBackend, HostBackend, NoopServerSpawner, ServerSpawner};
pub use self::folder::*;
pub use self::hosting::{LocalRealmHosting, RealmHosting};
pub use self::rules::*;
use crate::{HostError, transport::SessionResult};

/// What the hosted game server is told (D3: the `RUSTIC_RIFTS_` prefix); stripped from the inherited environment.
pub const LAUNCH_VARIABLES: [&str; 17] = rules::launch_variable_names();
/// Debug switches a hosted server must never inherit.
pub const CLEARED_VARIABLES: [&str; 2] = [rifts_var!("ALLOW_DAMAGE_DEBUG"), rifts_var!("DEBUG_DAMAGE_MULTIPLIER")];
pub const DIRECTOR_SERVER_SWITCHES: [&str; 3] = ["-server", "-log", "-nullrhi"];

// Roster sizes and the host PC's memory floors.
pub const MAX_HUNT_PEERS: usize = 4;
pub const MAX_HUB_PEERS: usize = 64;
pub const HOST_MEMORY_FLOOR_MIB: u64 = 3 * 1024;
pub const HOST_COMMIT_FLOOR_MIB: u64 = 4 * 1024;

// Timers.
pub const GAME_LEFT_IDLE: Duration = Duration::from_secs(6);
pub const HUNT_JOIN_LEAVE_MAX: Duration = Duration::from_secs(2 * 60);
pub const RELAY_REFRESH_MARGIN_SECONDS: i64 = 20 * 60;
pub const RELAY_EARLY_REFRESH_SECONDS: i64 = 12 * 60 * 60;
pub const RELAY_EARLY_REFRESH_SPACING: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostedRole {
    Hunt,
    Hub(HubKind),
}

impl HostedRole {
    pub const fn wire_role(self) -> &'static str {
        if let Self::Hub(_) = self { "HUB" } else { "HUNT" }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalState {
    Starting,
    Ready,
    Running,
    Ended(SessionResult),
}

/// One session this launcher hosts or has joined.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalSession {
    pub session: SessionId,
    pub generation: u64,
    pub role: HostedRole,
    pub hosting: bool,
    pub state: LocalState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HubLimits {
    pub max_players: u8,
    pub max_servers: u8,
    pub afk_seconds: u32,
}

const DEFAULT_HUB_LIMITS: HubLimits = HubLimits {
    max_players: 16,
    max_servers: 2,
    afk_seconds: 0,
};

impl Default for HubLimits {
    fn default() -> Self {
        DEFAULT_HUB_LIMITS
    }
}

/// The backend's `transport-config` reply; a missing optional field reads as `None`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportConfig {
    pub generation: u64,
    pub local_account_id: String,
    pub local_peer_id: String,
    pub host_account_id: String,
    pub host_peer_id: String,
    pub ice_servers: Vec<IceServerEntry>,
    pub peers: Vec<IcePeerEntry>,
    pub relay: Option<RelayGrant>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IceServerEntry {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IcePeerEntry {
    pub account_id: String,
    pub peer_id: String,
    pub transport_key: String,
}

/// Every field may be missing; a missing grant is not allowed.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RelayGrant {
    pub allowed: bool,
    pub expires_at: Option<String>,
}

impl TransportConfig {
    pub fn relay_expires_at(&self) -> Option<String> {
        let grant = self.relay.as_ref()?;
        if grant.allowed { grant.expires_at.clone() } else { None }
    }

    /// The first ICE server with both a username and a credential, only when both are safe; no further look.
    pub fn relay_turn_credential(&self) -> Option<(String, Secret)> {
        let (user, credential) = self
            .ice_servers
            .iter()
            .find_map(|entry| entry.username.as_deref().zip(entry.credential.as_deref()))?;
        if safe_credential(user) && safe_credential(credential) {
            Some((user.to_owned(), Secret::new(credential.to_owned())))
        } else {
            None
        }
    }
}

/// Which session the ICE files are written for; each has its own peer list shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilesFor {
    Joiner,
    HuntHost,
    AwayHuntHost,
    HubHost,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IceFiles {
    pub config: String,
    pub secrets: Secret,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerLive {
    pub route: Option<String>,
    pub since: String,
}

/// What the hosting panel shows of a running session.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionLive {
    pub hunt_id: String,
    pub started_at: Option<String>,
    pub peers: BTreeMap<String, PeerLive>,
    pub relay_expires_at: Option<String>,
    pub local_since: Option<String>,
}

/// One entry of a reply's `peers`; peer ids are JSON strings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerListEntry {
    pub account_id: String,
    pub peer_id: String,
    pub transport_key: String,
}

/// A checked [`PeerListEntry`], its peer id parsed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WantedPeer {
    pub account_id: String,
    pub peer_id: u64,
    pub transport_key: String,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct PeerDiff {
    pub add: Vec<WantedPeer>,
    pub remove: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapacityDecision {
    Accept,
    Refuse { reason: &'static str },
}

/// When the game's local traffic last grew, from `UDP_STATS` lines.
#[derive(Clone, Debug)]
pub struct GameTrafficWatch {
    reading: Option<u64>,
    changed: Instant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

/// What hosting and joining share of a session's bootstrap.
#[derive(Clone, Debug, Eq, PartialEq)]
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
        let checked = |value: &str, maximum: usize| safe_config_value(value, maximum).map(str::to_owned);
        // Not written here, but a value the transport reads later on its own line.
        checked(&self.guard_completion_mode, 32)?;
        let lines = [
            ("key_id", checked(&self.key_id, 32)?),
            ("session_id", checked(&wire_session_id(&self.session_id), 64)?),
            ("party_id", checked(&self.party_id, 192)?),
            ("party_revision", self.party_revision.to_string()),
            ("hunt_definition_id", checked(&self.hunt_definition_id, 128)?),
            ("generation", self.generation.to_string()),
            ("build", self.build.to_string()),
            ("bridge_protocol", self.bridge_protocol.to_string()),
            ("virtual_port", self.virtual_port.to_string()),
            ("host_peer_id", checked(&self.host_peer_id, 20)?),
            ("bridge_instance_id", checked(&self.bridge_instance_id, 96)?),
            ("metadata_nonce", checked(&self.metadata_nonce, 64)?),
        ];
        Ok(lines.iter().map(|(key, value)| format!("{key}={value}\n")).collect())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JoinBootstrapSpec {
    pub common: CommonBootstrapConfig,
    pub role: HostedRole,
    pub ticket: Secret,
}
