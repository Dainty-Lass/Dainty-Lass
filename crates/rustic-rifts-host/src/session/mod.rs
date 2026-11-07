// SPDX-License-Identifier: MIT
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

use rustic_rifts_seam::{HubKind, Secret, SessionId};
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
/// The switches every hosted game server starts with, after its positional arguments.
pub const DIRECTOR_SERVER_SWITCHES: [&str; 3] = ["-server", "-log", "-nullrhi"];

// Roster sizes and the host PC's memory floors.
/// The most joiners a hosted hunt takes.
pub const MAX_HUNT_PEERS: usize = 4;
/// The most joiners a hosted hub takes.
pub const MAX_HUB_PEERS: usize = 64;
/// The physical memory, in MiB, a PC needs to offer hosting.
pub const HOST_MEMORY_FLOOR_MIB: u64 = 3 * 1024;
/// The free commit charge, in MiB, below which a new hosted server is refused.
pub const HOST_COMMIT_FLOOR_MIB: u64 = 4 * 1024;

// Timers.
/// How long the game's local traffic may stand still before a joiner is taken to have left.
pub const GAME_LEFT_IDLE: Duration = Duration::from_secs(6);
/// How long a hunt's join-or-leave wait lasts before it times out.
pub const HUNT_JOIN_LEAVE_MAX: Duration = Duration::from_secs(2 * 60);
/// Within this many seconds of the relay grant's expiry, it is always refreshed.
pub const RELAY_REFRESH_MARGIN_SECONDS: i64 = 20 * 60;
/// Within this many seconds of the relay grant's expiry, an early refresh may be tried.
pub const RELAY_EARLY_REFRESH_SECONDS: i64 = 12 * 60 * 60;
/// The least time between early relay refreshes.
pub const RELAY_EARLY_REFRESH_SPACING: Duration = Duration::from_secs(30 * 60);

/// What a hosted session is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostedRole {
    /// A hunt.
    Hunt,
    /// A realm hub of this kind.
    Hub(HubKind),
}

impl HostedRole {
    /// `HUNT` or `HUB`, as the backend and the server's environment name it.
    pub const fn wire_role(self) -> &'static str {
        if let Self::Hub(_) = self { "HUB" } else { "HUNT" }
    }
}

/// Where a local session is in its life.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalState {
    /// Opening, not ready yet.
    Starting,
    /// The transport reported ready.
    Ready,
    /// Running with its game server.
    Running,
    /// Ended with this result.
    Ended(SessionResult),
}

/// One session this launcher hosts or has joined.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalSession {
    /// The realm session.
    pub session: SessionId,
    /// The session's generation.
    pub generation: u64,
    /// Hunt or hub.
    pub role: HostedRole,
    /// Whether this launcher hosts it rather than joining.
    pub hosting: bool,
    /// Where it is in its life.
    pub state: LocalState,
}

/// A host's limits for the hubs it runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HubLimits {
    /// Players per hub, 1 to 64.
    pub max_players: u8,
    /// Hub servers at once, 1 to 8.
    pub max_servers: u8,
    /// Seconds a player may idle before the hub drops them, 0 to 7200; 0 for never.
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
    /// The generation the config is for.
    pub generation: u64,
    /// This launcher's account.
    pub local_account_id: String,
    /// This launcher's peer id, as decimal text.
    pub local_peer_id: String,
    /// The host's account.
    pub host_account_id: String,
    /// The host's peer id, as decimal text.
    pub host_peer_id: String,
    /// The STUN and TURN servers.
    pub ice_servers: Vec<IceServerEntry>,
    /// The peers this side may link with.
    pub peers: Vec<IcePeerEntry>,
    /// The relay grant, if the backend sent one.
    pub relay: Option<RelayGrant>,
}

/// One ICE server entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IceServerEntry {
    /// Its `stun:` or `turn:` URLs.
    pub urls: Vec<String>,
    /// The TURN username, if any.
    pub username: Option<String>,
    /// The TURN credential, if any.
    pub credential: Option<String>,
}

/// One peer of a transport config.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IcePeerEntry {
    /// The peer's account.
    pub account_id: String,
    /// The peer's id, as decimal text.
    pub peer_id: String,
    /// The 43-character base64url key sealing frames with this peer.
    pub transport_key: String,
}

/// Every field may be missing; a missing grant is not allowed.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RelayGrant {
    /// Whether the relay may be used.
    pub allowed: bool,
    /// When the grant expires, as an ISO 8601 time.
    pub expires_at: Option<String>,
}

impl TransportConfig {
    /// When the relay grant expires, only when it is allowed.
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
    /// A joiner's link to its host.
    Joiner,
    /// A hunt host whose own game client plays.
    HuntHost,
    /// A hunt host whose own game client does not play.
    AwayHuntHost,
    /// A hub host.
    HubHost,
}

/// The two ICE files of a session folder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IceFiles {
    /// `bridge.conf`'s ICE lines.
    pub config: String,
    /// `transport.secret`: the relay credential and peer keys.
    pub secrets: Secret,
}

/// One peer event line from the transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerEvent {
    /// A peer's link connected.
    Connected {
        /// The peer's account.
        account: String,
        /// `relay` or `direct`, when the line names it.
        route: Option<String>,
    },
    /// A peer was admitted.
    Admitted {
        /// The peer's account.
        account: String,
        /// `relay` or `direct`, when the line names it.
        route: Option<String>,
    },
    /// A peer's link was lost.
    Lost {
        /// The peer's account.
        account: String,
        /// Why, when the line says.
        reason: Option<String>,
    },
    /// A peer was taken off the roster.
    Removed {
        /// The peer's account.
        account: String,
        /// Why, when the line says.
        reason: Option<String>,
        /// Whether it had been admitted.
        admitted: bool,
    },
    /// A roster add was refused.
    AddRejected {
        /// The peer's account.
        account: String,
        /// Why, when the line says.
        reason: Option<String>,
    },
}

/// One admitted peer as the hosting panel shows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerLive {
    /// `relay` or `direct`, when known.
    pub route: Option<String>,
    /// When it was admitted, as an ISO 8601 time.
    pub since: String,
}

/// What the hosting panel shows of a running session.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionLive {
    /// The hunt or hub being hosted.
    pub hunt_id: String,
    /// When it started, as an ISO 8601 time.
    pub started_at: Option<String>,
    /// Admitted peers by account.
    pub peers: BTreeMap<String, PeerLive>,
    /// When the relay grant expires, if one is allowed.
    pub relay_expires_at: Option<String>,
    /// When the host's own game client was admitted, as an ISO 8601 time.
    pub local_since: Option<String>,
}

/// One entry of a reply's `peers`; peer ids are JSON strings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerListEntry {
    /// The peer's account.
    pub account_id: String,
    /// The peer's id, as decimal text.
    pub peer_id: String,
    /// The 43-character base64url key sealing frames with this peer.
    pub transport_key: String,
}

/// A checked [`PeerListEntry`], its peer id parsed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WantedPeer {
    /// The peer's account.
    pub account_id: String,
    /// The peer's id.
    pub peer_id: u64,
    /// The 43-character base64url key sealing frames with this peer.
    pub transport_key: String,
}

/// How the host's roster must change to match the wanted peers.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct PeerDiff {
    /// Peers to add, in the wanted order.
    pub add: Vec<WantedPeer>,
    /// Accounts to remove, sorted.
    pub remove: Vec<String>,
}

/// Whether the host PC takes another hosted server.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapacityDecision {
    /// There is room.
    Accept,
    /// No room.
    Refuse {
        /// The refusal's code, `HOST_BUSY`.
        reason: &'static str,
    },
}

/// When the game's local traffic last grew, from `UDP_STATS` lines.
#[derive(Clone, Debug)]
pub struct GameTrafficWatch {
    reading: Option<u64>,
    changed: Instant,
}

/// What a hosted game server's `RUSTIC_RIFTS_` environment is built from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchEnvironmentSpec {
    /// The host runtime capability the server authenticates with.
    pub capability: Secret,
    /// Hunt or hub.
    pub role: HostedRole,
    /// The realm session.
    pub session_id: SessionId,
    /// The session's generation.
    pub generation: u64,
    /// This launch's id.
    pub launch_id: String,
    /// The pipe the server reports ready on.
    pub ready_pipe: String,
    /// The pipe the launcher sends control messages on.
    pub control_pipe: String,
    /// The pipe the server sends Guard events on.
    pub guard_pipe: String,
    /// Whether the server sends Hunt Guard events.
    pub hunt_evidence: bool,
    /// When the hunt pays its completion, such as `FIRST_DEFEAT`.
    pub guard_completion_mode: String,
    /// The server's runtime profile.
    pub runtime_profile: String,
    /// The launcher's process id.
    pub launcher_pid: u32,
    /// Players per hub; set for a hub only.
    pub max_players: Option<u8>,
    /// Hub idle limit in seconds; set for a hub only.
    pub hub_afk_seconds: Option<u32>,
    /// The trials week the hunt belongs to, if it is a trial.
    pub trials_week: Option<String>,
}

/// What hosting and joining share of a session's bootstrap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommonBootstrapConfig {
    /// The key the tickets are signed with.
    pub key_id: String,
    /// The realm session.
    pub session_id: SessionId,
    /// The party the hunt was made for.
    pub party_id: String,
    /// The party's revision.
    pub party_revision: u64,
    /// The hunt definition being played.
    pub hunt_definition_id: String,
    /// The session's generation.
    pub generation: u64,
    /// The game build.
    pub build: u32,
    /// The bridge protocol version.
    pub bridge_protocol: u8,
    /// The game's virtual port.
    pub virtual_port: u16,
    /// The host's peer id, as decimal text.
    pub host_peer_id: String,
    /// This bridge's instance id.
    pub bridge_instance_id: String,
    /// The bootstrap's nonce.
    pub metadata_nonce: String,
    /// Use TURN relays only.
    pub force_relay: bool,
    /// When the hunt pays its completion, such as `FIRST_DEFEAT`.
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

/// Everything a host session's folder and server launch are built from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostBootstrapSpec {
    /// What hosting and joining share.
    pub common: CommonBootstrapConfig,
    /// Hunt or hub.
    pub role: HostedRole,
    /// The host runtime capability.
    pub capability: Secret,
    /// The host's own ticket, for its local game client.
    pub host_ticket: Option<Secret>,
    /// The backend the transport calls.
    pub backend_url: String,
    /// The hosted game server's UDP port.
    pub server_udp_port: u16,
    /// The most joiners at once.
    pub maximum_peers: u8,
    /// Whether the host's own game client plays.
    pub local_client: bool,
    /// Whether the server sends Hunt Guard events.
    pub hunt_evidence: bool,
    /// The hunt id the server is started with.
    pub matchmaker_hunt_id: String,
    /// The map the server loads.
    pub map_path: String,
    /// The behemoth the server spawns.
    pub behemoth_path: String,
    /// The expected player count, as text.
    pub expected_players: String,
    /// The server's runtime profile.
    pub runtime_profile: String,
    /// Players per hub; for a hub only.
    pub max_players: Option<u8>,
    /// Hub idle limit in seconds; for a hub only.
    pub hub_afk_seconds: Option<u32>,
}

/// Everything a joiner session's folder is built from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JoinBootstrapSpec {
    /// What hosting and joining share.
    pub common: CommonBootstrapConfig,
    /// Hunt or hub.
    pub role: HostedRole,
    /// The joiner's admission ticket.
    pub ticket: Secret,
}
