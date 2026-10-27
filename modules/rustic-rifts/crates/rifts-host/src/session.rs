//! The launcher's host and join surface (`ParadoxLauncher/src-tauri/src/p2p/`, RF1 section 3), behind DaintyLass's
//! realms feature (DD12). The launcher calls it from its bridge commands (DF8); the account key never reaches it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rifts_seam::{AccountId, BoxFuture, GlobalId, HubKind, NodeId, Secret, SessionId, Ulid};
use serde::{Deserialize, Serialize};

use crate::HostError;
use crate::files::{ADMISSION_PUBLIC_KEY_FILE, DISABLE_COOP_FLAG_FILE, SESSION_FILES, validate_hub_limits};
use crate::transport::{ABI_VERSION, Handle, HostOpen, JoinOpen, ReadyState, SessionResult, TransportAbi};

/// What the launcher gives the hosted game server, checked by the runtime's `ReadLaunch`. A change here is a change
/// to rustic-isles' runtime too (RF1, decision 5).
pub const LAUNCH_VARIABLES: [&str; 17] = [
    "MYSTICPARADOX_AUTH_MODE",
    "MYSTICPARADOX_BIND_ADDRESS",
    "MYSTICPARADOX_HOST_RUNTIME_CAPABILITY",
    "MYSTICPARADOX_HOSTED_ROLE",
    "MYSTICPARADOX_SESSION_ID",
    "MYSTICPARADOX_SESSION_GENERATION",
    "MYSTICPARADOX_GAMESERVER_LAUNCH_ID",
    "MYSTICPARADOX_READY_PIPE",
    "MYSTICPARADOX_CONTROL_PIPE",
    "MYSTICPARADOX_GUARD_PIPE",
    "MYSTICPARADOX_HUNT_GUARD_EVENTS",
    "MYSTICPARADOX_HUNT_GUARD_COMPLETION_MODE",
    "MYSTICPARADOX_RUNTIME_PROFILE",
    "MYSTICPARADOX_LAUNCHER_PID",
    "MYSTICPARADOX_MAX_PLAYERS",
    "MYSTICPARADOX_HUB_AFK_SECONDS",
    "MYSTICPARADOX_TRIALS_WEEK",
];

/// Removed from the environment the hosted server inherits.
pub const CLEARED_VARIABLES: [&str; 2] = [
    "MYSTICPARADOX_ALLOW_DAMAGE_DEBUG",
    "MYSTICPARADOX_DEBUG_DAMAGE_MULTIPLIER",
];

/// Standard switches passed to a player-hosted 1.14.7 server (`ParadoxLauncher/src-tauri/src/p2p/session.rs`).
pub const DIRECTOR_SERVER_SWITCHES: [&str; 3] = ["-server", "-log", "-nullrhi"];

/// Maximum remote peers on a hub vs a hunt (`p2p/session/hub.rs`).
pub const MAX_HUB_PEERS: usize = 64;
pub const MAX_HUNT_PEERS: usize = 4;

/// Commit available floor (MiB) before one more hosted server starts (`p2p/realm.rs`).
pub const HOST_COMMIT_FLOOR_MIB: u64 = 4096;
/// Free physical memory warning floor (MiB) (`p2p/realm.rs`).
pub const HOST_MEMORY_FLOOR_MIB: u64 = 3072;

/// `MYSTICPARADOX_HOSTED_ROLE`.
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

/// One session this PC hosts or joined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSession {
    pub session: SessionId,
    pub generation: u64,
    pub role: HostedRole,
    pub hosting: bool,
    pub state: LocalState,
}

/// `realm-host-limits.json`, per hub server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubLimits {
    /// 1 to 64.
    pub max_players: u8,
    pub max_servers: u8,
    /// 0 to 7200.
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

pub trait RealmHosting: Send + Sync {
    /// The co-op switch (`p2p/mod.rs`): off with `DISABLE_COOP_TRANSPORT.flag` or an unusable transport.
    fn coop_available(&self) -> BoxFuture<'_, Result<bool, HostError>>;
    /// Host bootstrap, session folder, transport `HostOpen`, then the game server with [`LAUNCH_VARIABLES`].
    fn host_hunt(&self, account: AccountId, session: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>>;
    /// Join bootstrap, transport config, `JoinOpen`; the game connects to the local port.
    fn join_hunt(&self, account: AccountId, session: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>>;
    fn start_hub(
        &self,
        account: AccountId,
        kind: HubKind,
        limits: HubLimits,
    ) -> BoxFuture<'_, Result<LocalSession, HostError>>;
    fn stop_hub(&self, account: AccountId, kind: HubKind) -> BoxFuture<'_, Result<(), HostError>>;
    /// Ends one session and removes its folder (`session.rs:cleanup_directory`).
    fn stop(&self, session: SessionId) -> BoxFuture<'_, Result<(), HostError>>;
    /// The hosting panel's view.
    fn sessions(&self) -> BoxFuture<'_, Result<Vec<LocalSession>, HostError>>;
}

// ---- pure rules ported from ParadoxLauncher/src-tauri/src/p2p/ ------------------------------------------------------

/// Formats a [`SessionId`] as the 36-character lower-case UUID wire form expected by the runtime and `bridge.conf`.
pub fn wire_session_id(session: &SessionId) -> String {
    let hex = format!("{:032x}", session.0.ulid.0);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

pub fn safe_config_value(value: &str, maximum: usize) -> Result<&str, HostError> {
    if value.is_empty() || value.len() > maximum || value.contains(['\r', '\n', '\0']) {
        return Err(HostError::InvalidInput(
            "the backend returned unsafe player-host bootstrap data".to_owned(),
        ));
    }
    Ok(value)
}

pub fn safe_account(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
}

pub fn safe_peer_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 20
        && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok()
}

pub fn safe_key(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

pub fn safe_url(value: &str) -> bool {
    value.len() <= 256
        && (value.starts_with("stun:") || value.starts_with("turn:") || value.starts_with("turns:"))
        && value.bytes().all(|byte| byte > 0x20 && byte < 0x7f && byte != b',')
}

pub fn safe_credential(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && value.bytes().all(|byte| byte > 0x20 && byte < 0x7f)
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

/// The backend's `transport-config` answer (`p2p/session/ice.rs`).
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
            .filter(|relay| relay.allowed)
            .and_then(|relay| relay.expires_at.clone())
    }

    pub fn relay_turn_credential(&self) -> Option<(String, Secret)> {
        let server = self
            .ice_servers
            .iter()
            .find(|server| server.username.is_some() && server.credential.is_some())?;
        let (user, secret) = (server.username.as_deref()?, server.credential.as_deref()?);
        (safe_credential(user) && safe_credential(secret)).then(|| (user.to_owned(), Secret::new(secret.to_owned())))
    }
}

/// Who the ICE config and secret files are built for (`p2p/session/ice.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilesFor {
    Joiner,
    HuntHost,
    AwayHuntHost,
    HubHost,
}

/// Config additions (non-secret) and `transport.secret` contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceFiles {
    pub config: String,
    pub secrets: Secret,
}

/// Builds the ICE lines for `bridge.conf` and `transport.secret` (`p2p/session/ice.rs:ice_files`).
pub fn ice_files(config: &TransportConfig, files_for: FilesFor, host_peer_id: &str) -> Result<IceFiles, HostError> {
    let host = files_for != FilesFor::Joiner;
    let hub = files_for == FilesFor::HubHost;
    let invalid = || HostError::InvalidInput("the backend returned an unsafe co-op transport configuration".to_owned());
    if !safe_account(&config.local_account_id)
        || !safe_peer_id(&config.local_peer_id)
        || !safe_account(&config.host_account_id)
        || !safe_peer_id(&config.host_peer_id)
        || config.host_peer_id != host_peer_id
    {
        return Err(invalid());
    }
    let mut stun = Vec::new();
    let mut turn = Vec::new();
    let mut credential: Option<(&str, &str)> = None;
    for server in &config.ice_servers {
        if !server.urls.iter().all(|url| safe_url(url)) {
            return Err(invalid());
        }
        match (&server.username, &server.credential) {
            (Some(user), Some(secret)) => {
                if !safe_credential(user) || !safe_credential(secret) {
                    return Err(invalid());
                }
                if credential.is_none() {
                    credential = Some((user, secret));
                    turn.extend(server.urls.iter().filter(|url| url.starts_with("turn")).cloned());
                }
            }
            _ => stun.extend(server.urls.iter().filter(|url| url.starts_with("stun:")).cloned()),
        }
    }
    let peers_valid = hub
        || config
            .peers
            .iter()
            .all(|peer| safe_account(&peer.account_id) && safe_peer_id(&peer.peer_id) && safe_key(&peer.transport_key));
    let pairs_valid = if hub {
        true
    } else if host {
        let joiners = if files_for == FilesFor::AwayHuntHost { 4 } else { 3 };
        config.peers.len() <= joiners
            && config
                .peers
                .iter()
                .all(|peer| peer.account_id != config.local_account_id)
    } else {
        config.peers.len() == 1
            && config.peers[0].account_id == config.host_account_id
            && config.peers[0].peer_id == config.host_peer_id
    };
    if !peers_valid || !pairs_valid {
        return Err(invalid());
    }
    let mut text = String::new();
    if !stun.is_empty() {
        text.push_str(&format!("ice_stun_urls={}\n", stun.join(",")));
    }
    if !turn.is_empty() {
        text.push_str(&format!("ice_turn_urls={}\n", turn.join(",")));
    }
    text.push_str(&format!("local_account_id={}\n", config.local_account_id));
    if !host {
        text.push_str(&format!(
            "local_peer_id={}\nhost_account_id={}\n",
            config.local_peer_id, config.host_account_id
        ));
    }
    let mut secrets = String::new();
    if let Some((user, secret)) = credential.filter(|_| !turn.is_empty()) {
        secrets.push_str(&format!("turn_username={user}\nturn_credential={secret}\n"));
    }
    for peer in config.peers.iter().filter(|_| !hub) {
        secrets.push_str(&format!(
            "peer={},{},{}\n",
            peer.account_id, peer.peer_id, peer.transport_key
        ));
    }
    Ok(IceFiles {
        config: text,
        secrets: Secret::new(secrets),
    })
}

/// Parses a `HOST_READY` or `ROUTE_READY` line (`p2p/session/ice.rs:parse_ice_ready`).
pub fn parse_ice_ready(
    line: &str,
    expected_state: &str,
    force_relay: bool,
    local_client: bool,
) -> Result<u16, HostError> {
    let mut tokens = line.split_whitespace();
    if tokens.next() != Some(expected_state) {
        return Err(HostError::Transport(
            "the co-op transport module returned an unexpected readiness state".to_owned(),
        ));
    }
    let mut fields = HashMap::new();
    for token in tokens {
        let Some((key, value)) = token.split_once('=') else {
            return Err(HostError::Transport(
                "the co-op transport module returned a malformed local route".to_owned(),
            ));
        };
        if key.is_empty() || value.is_empty() || fields.insert(key, value).is_some() {
            return Err(HostError::Transport(
                "the co-op transport module returned a malformed local route".to_owned(),
            ));
        }
    }
    if fields.get("localHost") != Some(&"127.0.0.1") || fields.get("transport") != Some(&"ice") {
        return Err(HostError::Transport(
            "the co-op transport module did not return a verified loopback route".to_owned(),
        ));
    }
    let expected_policy = if force_relay { "RELAY_ONLY" } else { "AUTOMATIC" };
    if fields.get("routePolicy") != Some(&expected_policy)
        || (force_relay && expected_state == "ROUTE_READY" && fields.get("route") != Some(&"relay"))
    {
        return Err(HostError::Transport(
            "the co-op transport module returned a route outside the requested transport policy".to_owned(),
        ));
    }
    fields
        .get("localPort")
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| (*port != 0) == local_client)
        .ok_or_else(|| HostError::Transport("the co-op transport module returned an invalid local route".to_owned()))
}

/// A line of `ReadPeerEventH` (`p2p/session/hub.rs`).
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

pub fn parse_peer_event(line: &str) -> Option<PeerEvent> {
    let mut tokens = line.split_whitespace();
    let kind = tokens.next()?;
    let fields: HashMap<&str, &str> = tokens.filter_map(|token| token.split_once('=')).collect();
    let account = (*fields.get("account")?).to_owned();
    let text = |key: &str| fields.get(key).map(|value| (*value).to_owned());
    Some(match kind {
        "PEER_CONNECTED" => PeerEvent::Connected {
            account,
            route: text("route"),
        },
        "PEER_ADMITTED" => PeerEvent::Admitted {
            account,
            route: text("route"),
        },
        "PEER_LOST" => PeerEvent::Lost {
            account,
            reason: text("reason"),
        },
        "PEER_REMOVED" => PeerEvent::Removed {
            account,
            reason: text("reason"),
            admitted: fields.get("admitted") == Some(&"1"),
        },
        "PEER_ADD_REJECTED" => PeerEvent::AddRejected {
            account,
            reason: text("reason"),
        },
        _ => return None,
    })
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

pub fn is_local_admission(line: &str) -> bool {
    let mut tokens = line.split_whitespace();
    if tokens.next() != Some("ADMISSION_ACCEPTED") {
        return false;
    }
    let fields: Vec<&str> = tokens.collect();
    fields.contains(&"role=host") && fields.contains(&"source=local")
}

pub fn apply_peer_event(live: &mut SessionLive, event: &PeerEvent, now_iso: &str) {
    match event {
        PeerEvent::Admitted { account, route } => {
            live.peers.insert(
                account.clone(),
                PeerLive {
                    route: route.clone(),
                    since: now_iso.to_owned(),
                },
            );
        }
        PeerEvent::Lost { account, .. } | PeerEvent::Removed { account, .. } => {
            live.peers.remove(account);
        }
        PeerEvent::Connected { .. } | PeerEvent::AddRejected { .. } => {}
    }
}

pub fn apply_peer_line(live: &mut SessionLive, line: &str, now_iso: &str) {
    if is_local_admission(line) {
        live.local_since = Some(now_iso.to_owned());
        return;
    }
    if let Some(event) = parse_peer_event(line) {
        apply_peer_event(live, &event, now_iso);
    }
}

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

pub fn wanted_peers(entries: &[PeerListEntry], host_account_id: Option<&str>) -> (Vec<WantedPeer>, usize) {
    let mut wanted: Vec<WantedPeer> = Vec::new();
    let mut refused = 0;
    for entry in entries.iter().take(MAX_HUB_PEERS) {
        let peer_id = entry
            .peer_id
            .parse::<u64>()
            .ok()
            .filter(|_| safe_peer_id(&entry.peer_id));
        match peer_id {
            Some(peer_id)
                if safe_account(&entry.account_id)
                    && safe_key(&entry.transport_key)
                    && Some(entry.account_id.as_str()) != host_account_id =>
            {
                if !wanted.iter().any(|other| other.account_id == entry.account_id) {
                    wanted.push(WantedPeer {
                        account_id: entry.account_id.clone(),
                        peer_id,
                        transport_key: entry.transport_key.clone(),
                    });
                }
            }
            _ => refused += 1,
        }
    }
    (wanted, refused)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PeerDiff {
    pub add: Vec<WantedPeer>,
    pub remove: Vec<String>,
}

pub fn peer_diff(linked: &HashSet<String>, wanted: &[WantedPeer]) -> PeerDiff {
    let add = wanted
        .iter()
        .filter(|peer| !linked.contains(&peer.account_id))
        .cloned()
        .collect();
    let mut remove: Vec<String> = linked
        .iter()
        .filter(|account| !wanted.iter().any(|peer| &peer.account_id == *account))
        .cloned()
        .collect();
    remove.sort();
    PeerDiff { add, remove }
}

/// The Trials week pinned from a matchmaker hunt id (`p2p/session.rs:trials_week`).
pub fn trials_week(matchmaker_hunt_id: &str) -> Option<&str> {
    let week = ["Easy", "Hard", "Elite"].iter().find_map(|tier| {
        matchmaker_hunt_id
            .strip_prefix("Arena_MatchmakerHunt_")?
            .strip_prefix(tier)?
            .strip_prefix('_')
    })?;
    (week.len() == 3 && week.bytes().all(|byte| byte.is_ascii_digit())).then_some(week)
}

pub fn host_server_arguments(
    server_udp_port: u16,
    map_path: &str,
    behemoth_path: &str,
    matchmaker_hunt_id: &str,
    expected_players: &str,
) -> Vec<String> {
    let mut arguments = vec![
        "-".to_owned(),
        server_udp_port.to_string(),
        map_path.to_owned(),
        behemoth_path.to_owned(),
        matchmaker_hunt_id.to_owned(),
        expected_players.to_owned(),
        format!("127.0.0.1:{server_udp_port}"),
    ];
    arguments.extend(DIRECTOR_SERVER_SWITCHES.iter().map(|s| (*s).to_owned()));
    arguments.push("-MULTIHOME=127.0.0.1".to_owned());
    arguments
}

pub fn windows_command_line(program: &Path, arguments: &[String]) -> String {
    fn append(line: &mut String, argument: &str, force_quotes: bool) {
        let quote = force_quotes || argument.is_empty() || argument.contains([' ', '\t']);
        if quote {
            line.push('"');
        }
        let mut backslashes = 0usize;
        for character in argument.chars() {
            if character == '\\' {
                backslashes += 1;
            } else {
                if character == '"' {
                    line.extend(std::iter::repeat_n('\\', backslashes + 1));
                }
                backslashes = 0;
            }
            line.push(character);
        }
        if quote {
            line.extend(std::iter::repeat_n('\\', backslashes));
            line.push('"');
        }
    }
    let mut line = String::new();
    append(&mut line, &program.to_string_lossy(), true);
    for argument in arguments {
        line.push(' ');
        append(&mut line, argument, false);
    }
    line
}

pub fn sanitize_central_log_line(line: &str) -> String {
    let mut sanitized = String::with_capacity(line.len().min(2048));
    for (index, part) in line.split_whitespace().enumerate() {
        if index > 0 {
            sanitized.push(' ');
        }
        let lower = part.to_ascii_lowercase();
        let sensitive = [
            "ticket",
            "capability",
            "token",
            "challenge",
            "secret",
            "signature",
            "password",
            "auth_password",
        ]
        .iter()
        .any(|needle| lower.contains(needle));
        if sensitive {
            if let Some((key, _)) = part.split_once('=') {
                sanitized.push_str(key);
                sanitized.push_str("=[redacted]");
            } else {
                sanitized.push_str("[redacted]");
            }
        } else {
            sanitized.push_str(part);
        }
        if sanitized.len() >= 2048 {
            sanitized = sanitized.chars().take(2048).collect();
            break;
        }
    }
    sanitized
}

/// Inputs to build the hosted game server's environment (`LAUNCH_VARIABLES` and `CLEARED_VARIABLES`).
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

/// Builds the hosted server's environment: removes [`CLEARED_VARIABLES`] from `inherited` (case-insensitively) and
/// applies [`LAUNCH_VARIABLES`] (`p2p/session/host.rs`).
pub fn build_launch_environment(
    spec: &LaunchEnvironmentSpec,
    inherited: impl IntoIterator<Item = (String, String)>,
) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = inherited
        .into_iter()
        .filter(|(key, _)| {
            !CLEARED_VARIABLES
                .iter()
                .chain(LAUNCH_VARIABLES.iter())
                .any(|name| key.eq_ignore_ascii_case(name))
        })
        .collect();

    env.insert("MYSTICPARADOX_AUTH_MODE".to_owned(), "HUNT_CAPABILITY".to_owned());
    env.insert("MYSTICPARADOX_BIND_ADDRESS".to_owned(), "127.0.0.1".to_owned());
    env.insert(
        "MYSTICPARADOX_HOST_RUNTIME_CAPABILITY".to_owned(),
        spec.capability.expose().to_owned(),
    );
    env.insert("MYSTICPARADOX_HOSTED_ROLE".to_owned(), spec.role.wire_role().to_owned());
    let wire_session = wire_session_id(&spec.session_id);
    env.insert("MYSTICPARADOX_SESSION_ID".to_owned(), wire_session);
    env.insert(
        "MYSTICPARADOX_SESSION_GENERATION".to_owned(),
        spec.generation.to_string(),
    );
    env.insert("MYSTICPARADOX_GAMESERVER_LAUNCH_ID".to_owned(), spec.launch_id.clone());
    env.insert("MYSTICPARADOX_READY_PIPE".to_owned(), spec.ready_pipe.clone());
    env.insert("MYSTICPARADOX_CONTROL_PIPE".to_owned(), spec.control_pipe.clone());
    env.insert("MYSTICPARADOX_GUARD_PIPE".to_owned(), spec.guard_pipe.clone());
    env.insert(
        "MYSTICPARADOX_HUNT_GUARD_EVENTS".to_owned(),
        if spec.hunt_evidence { "1" } else { "0" }.to_owned(),
    );
    env.insert(
        "MYSTICPARADOX_HUNT_GUARD_COMPLETION_MODE".to_owned(),
        spec.guard_completion_mode.clone(),
    );
    env.insert("MYSTICPARADOX_RUNTIME_PROFILE".to_owned(), spec.runtime_profile.clone());
    env.insert("MYSTICPARADOX_LAUNCHER_PID".to_owned(), spec.launcher_pid.to_string());
    if matches!(spec.role, HostedRole::Hub(_)) {
        if let Some(max_players) = spec.max_players {
            env.insert("MYSTICPARADOX_MAX_PLAYERS".to_owned(), max_players.to_string());
        }
        if let Some(afk) = spec.hub_afk_seconds {
            env.insert("MYSTICPARADOX_HUB_AFK_SECONDS".to_owned(), afk.to_string());
        }
    }
    if let Some(week) = spec.trials_week.as_deref() {
        env.insert("MYSTICPARADOX_TRIALS_WEEK".to_owned(), week.to_owned());
    }
    env
}

pub const GAME_LEFT_IDLE: Duration = Duration::from_secs(6);
pub const HUNT_JOIN_LEAVE_MAX: Duration = Duration::from_secs(120);

pub fn udp_stats_local_rx(line: &str) -> Option<u64> {
    if !line.contains("UDP_STATS") {
        return None;
    }
    line.split_whitespace()
        .find_map(|field| field.strip_prefix("localRx="))
        .and_then(|value| value.parse::<u64>().ok())
}

#[derive(Debug, Clone)]
pub struct GameTrafficWatch {
    local_rx: Option<u64>,
    last_change: Instant,
}

impl GameTrafficWatch {
    pub fn new(now: Instant) -> Self {
        Self {
            local_rx: None,
            last_change: now,
        }
    }

    pub fn observe(&mut self, line: &str, now: Instant) {
        if let Some(rx) = udp_stats_local_rx(line) {
            if self.local_rx.is_none_or(|prev| rx > prev) {
                self.last_change = now;
            }
            self.local_rx = Some(rx);
        }
    }

    pub fn restart(&mut self, now: Instant) {
        self.last_change = now;
    }

    pub fn idle(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_change)
    }
}

pub fn hunt_join_leave_outcome(asked_at: Instant, traffic: &GameTrafficWatch, now: Instant) -> Option<&'static str> {
    if traffic.idle(now) >= GAME_LEFT_IDLE {
        Some("game_left")
    } else if now.saturating_duration_since(asked_at) >= HUNT_JOIN_LEAVE_MAX {
        Some("timeout")
    } else {
        None
    }
}

pub const RELAY_REFRESH_MARGIN_SECONDS: i64 = 20 * 60;
pub const RELAY_EARLY_REFRESH_SECONDS: i64 = 12 * 3600;
pub const RELAY_EARLY_REFRESH_SPACING: Duration = Duration::from_secs(30 * 60);

pub fn relay_refresh_due(seconds_left: Option<i64>, since_last: Option<Duration>) -> bool {
    let Some(left) = seconds_left else {
        return false;
    };
    left <= RELAY_REFRESH_MARGIN_SECONDS
        || (left <= RELAY_EARLY_REFRESH_SECONDS
            && since_last.is_none_or(|elapsed| elapsed >= RELAY_EARLY_REFRESH_SPACING))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityDecision {
    Accept,
    Refuse { reason: &'static str },
}

pub fn default_max_servers(physical_gb: u64) -> u8 {
    if physical_gb < 4 {
        return 1;
    }
    (2 * (physical_gb - 4) / 3).clamp(1, 6) as u8
}

pub fn capacity_decision(commit_mib: u64, active_servers: u8, max_servers: u8) -> CapacityDecision {
    if commit_mib < HOST_COMMIT_FLOOR_MIB || active_servers >= max_servers {
        CapacityDecision::Refuse { reason: "HOST_BUSY" }
    } else {
        CapacityDecision::Accept
    }
}

pub fn restart_backoff(attempts_in_window: usize) -> Duration {
    match attempts_in_window {
        0 => Duration::from_secs(5),
        1 => Duration::from_secs(15),
        _ => Duration::from_secs(60),
    }
}

pub fn attempts_in_window(attempts: &[Instant], now: Instant) -> usize {
    attempts
        .iter()
        .filter(|at| now.saturating_duration_since(**at) < Duration::from_secs(10 * 60))
        .count()
}

pub fn restart_allowed(attempts: &[Instant], now: Instant) -> bool {
    attempts_in_window(attempts, now) < 5
}

pub fn next_idle_since(previous: Option<Instant>, present: usize, now: Instant) -> Option<Instant> {
    if present > 0 {
        None
    } else {
        Some(previous.unwrap_or(now))
    }
}

pub fn training_should_stop(idle_since: Option<Instant>, now: Instant) -> bool {
    idle_since.is_some_and(|since| now.saturating_duration_since(since) >= Duration::from_secs(5 * 60))
}

pub fn scheduled_restart_due(uptime: Duration, present: bool) -> bool {
    !present && uptime > Duration::from_secs(20 * 3600)
}

pub fn relay_restart_due(seconds_left: Option<i64>) -> bool {
    seconds_left.is_some_and(|left| left <= 5 * 60)
}

// ---- session files (`p2p-sessions/` created 0600 and deleted at session end) ---------------------------------------

fn random_hex(bytes: usize) -> Result<String, HostError> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|_| HostError::Io("could not generate session randomness".to_owned()))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Creates `<app_local_data>/p2p-sessions/<sessionId>-<generation>-<16 hex>/` with owner-only permissions.
pub fn create_session_directory(
    app_local_data: &Path,
    session_id: &SessionId,
    generation: u64,
) -> Result<PathBuf, HostError> {
    let wire = wire_session_id(session_id);
    safe_config_value(&wire, 64)?;
    let root = app_local_data.join("p2p-sessions");
    fs::create_dir_all(&root).map_err(|err| HostError::Io(format!("could not create {}: {err}", root.display())))?;
    let suffix = random_hex(8)?;
    let dir = root.join(format!("{wire}-{generation}-{suffix}"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|err| HostError::Io(format!("could not create {}: {err}", dir.display())))?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(&dir).map_err(|err| HostError::Io(format!("could not create {}: {err}", dir.display())))?;
    }
    Ok(dir)
}

/// Writes one session file (`bridge.conf`, `transport.secret`, `capability.secret`, `ticket.secret`) with `create_new`
/// and `0600` owner-only permissions.
pub fn write_session_file_0600(path: &Path, contents: &[u8]) -> Result<(), HostError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|err| HostError::Io(format!("could not create {}: {err}", path.display())))?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|err| HostError::Io(format!("could not write {}: {err}", path.display())))
}

/// Removes the four [`SESSION_FILES`] and the session directory itself (`p2p/session.rs:cleanup_directory`).
pub fn cleanup_directory(path: &Path) {
    for name in SESSION_FILES {
        let _ = fs::remove_file(path.join(name));
    }
    let _ = fs::remove_dir(path);
}

struct SessionCleanup {
    path: PathBuf,
    committed: bool,
}

impl SessionCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, committed: false }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for SessionCleanup {
    fn drop(&mut self) {
        if !self.committed {
            cleanup_directory(&self.path);
        }
    }
}

// ---- bootstrap specs and RealmHosting implementation ---------------------------------------------------------------

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
    pub fn to_config_lines(&self) -> Result<String, HostError> {
        let wire_session = wire_session_id(&self.session_id);
        for (value, maximum) in [
            (self.key_id.as_str(), 32),
            (wire_session.as_str(), 64),
            (self.party_id.as_str(), 192),
            (self.hunt_definition_id.as_str(), 128),
            (self.guard_completion_mode.as_str(), 32),
            (self.host_peer_id.as_str(), 20),
            (self.bridge_instance_id.as_str(), 96),
            (self.metadata_nonce.as_str(), 64),
        ] {
            safe_config_value(value, maximum)?;
        }
        Ok(format!(
            "key_id={}\nsession_id={}\nparty_id={}\nparty_revision={}\nhunt_definition_id={}\ngeneration={}\nbuild={}\nbridge_protocol={}\nvirtual_port={}\nhost_peer_id={}\nbridge_instance_id={}\nmetadata_nonce={}\n",
            self.key_id,
            wire_session,
            self.party_id,
            self.party_revision,
            self.hunt_definition_id,
            self.generation,
            self.build,
            self.bridge_protocol,
            self.virtual_port,
            self.host_peer_id,
            self.bridge_instance_id,
            self.metadata_nonce
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

/// Port for fetching session bootstrap and transport configurations without exposing the account's key to `rifts-host`.
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
}

/// Deterministic local [`HostBackend`] for single-PC hosting, the debug launcher, and unit tests.
#[derive(Debug, Default)]
pub struct DefaultHostBackend {
    hubs: Mutex<HashMap<(AccountId, HubKind), SessionId>>,
    routes: Mutex<HashMap<SessionId, u16>>,
}

impl DefaultHostBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn recorded_route(&self, session: &SessionId) -> Option<u16> {
        self.routes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session)
            .copied()
    }
}

fn default_common(session: &SessionId, generation: u64, hunt_def: &str, completion: &str) -> CommonBootstrapConfig {
    let wire = wire_session_id(session);
    CommonBootstrapConfig {
        key_id: "local-key-1".to_owned(),
        session_id: session.clone(),
        party_id: format!("party-{wire}"),
        party_revision: 1,
        hunt_definition_id: hunt_def.to_owned(),
        generation,
        build: 647_472,
        bridge_protocol: 1,
        virtual_port: 1,
        host_peer_id: "100001".to_owned(),
        bridge_instance_id: format!("bridge-{wire}"),
        metadata_nonce: "0123456789abcdef0123456789abcdef".to_owned(),
        force_relay: false,
        guard_completion_mode: completion.to_owned(),
    }
}

fn default_transport_config(account: &AccountId, generation: u64, joiner: bool) -> TransportConfig {
    let host_account = if joiner {
        "host-account-1".to_owned()
    } else {
        account.0.clone()
    };
    let local_peer = if joiner { "200002" } else { "100001" };
    let peers = if joiner {
        vec![IcePeerEntry {
            account_id: host_account.clone(),
            peer_id: "100001".to_owned(),
            transport_key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned(),
        }]
    } else {
        Vec::new()
    };
    TransportConfig {
        generation,
        local_account_id: account.0.clone(),
        local_peer_id: local_peer.to_owned(),
        host_account_id: host_account,
        host_peer_id: "100001".to_owned(),
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
        peers,
        relay: Some(RelayGrant {
            allowed: true,
            expires_at: Some("2026-10-28T00:00:00Z".to_owned()),
        }),
    }
}

fn hub_session_id(kind: HubKind, account: &AccountId) -> Result<SessionId, HostError> {
    let kind_bits: u128 = match kind {
        HubKind::Ramsgate => 1,
        HubKind::Training => 2,
    };
    let acct_bits = account
        .0
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(131).wrapping_add(u64::from(b)));
    let ulid = Ulid::from_parts(1_792_000_000_000, (kind_bits << 64) | u128::from(acct_bits));
    let node = NodeId::new("local").map_err(|err| HostError::InvalidInput(err.to_string()))?;
    Ok(SessionId(GlobalId { ulid, node }))
}

impl HostBackend for DefaultHostBackend {
    fn host_bootstrap(
        &self,
        account: &AccountId,
        session: &SessionId,
    ) -> Result<(HostBootstrapSpec, TransportConfig), HostError> {
        let generation = 1;
        let common = default_common(session, generation, "Hunt_Default", "FIRST_DEFEAT");
        let spec = HostBootstrapSpec {
            common,
            role: HostedRole::Hunt,
            capability: Secret::new("cap_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned()),
            host_ticket: Some(Secret::new("mpt1.ticket.signature".to_owned())),
            backend_url: "https://127.0.0.1:8443".to_owned(),
            server_udp_port: 8791,
            maximum_peers: 4,
            local_client: true,
            hunt_evidence: false,
            matchmaker_hunt_id: "Arena_MatchmakerHunt_Easy_001".to_owned(),
            map_path: "/Game/Maps/ HuntMap".replace(' ', ""),
            behemoth_path: "Behemoth_Gnasher".to_owned(),
            expected_players: "1".to_owned(),
            runtime_profile: "production".to_owned(),
            max_players: None,
            hub_afk_seconds: None,
        };
        Ok((spec, default_transport_config(account, generation, false)))
    }

    fn join_bootstrap(
        &self,
        account: &AccountId,
        session: &SessionId,
    ) -> Result<(JoinBootstrapSpec, TransportConfig), HostError> {
        let generation = 1;
        let common = default_common(session, generation, "Hunt_Default", "FIRST_DEFEAT");
        let spec = JoinBootstrapSpec {
            common,
            role: HostedRole::Hunt,
            ticket: Secret::new("mpt1.joiner_ticket.signature".to_owned()),
        };
        Ok((spec, default_transport_config(account, generation, true)))
    }

    fn start_hub(
        &self,
        account: &AccountId,
        kind: HubKind,
        limits: &HubLimits,
    ) -> Result<(HostBootstrapSpec, TransportConfig), HostError> {
        validate_hub_limits(limits)?;
        let generation = 1;
        let session = hub_session_id(kind, account)?;
        self.hubs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert((account.clone(), kind), session.clone());
        let common = default_common(&session, generation, "ShatteredIsles_ReturnToRamsgate", "NO_COMPLETION");
        let spec = HostBootstrapSpec {
            common,
            role: HostedRole::Hub(kind),
            capability: Secret::new("cap_HUB_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned()),
            host_ticket: Some(Secret::new("mpt1.hub_host_ticket.signature".to_owned())),
            backend_url: "https://127.0.0.1:8443".to_owned(),
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
        Ok((spec, default_transport_config(account, generation, false)))
    }

    fn stop_hub(&self, account: &AccountId, kind: HubKind) -> Result<Option<SessionId>, HostError> {
        Ok(self
            .hubs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&(account.clone(), kind)))
    }

    fn route_ready(
        &self,
        _account: &AccountId,
        session: &SessionId,
        _generation: u64,
        local_port: u16,
    ) -> Result<(), HostError> {
        self.routes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(session.clone(), local_port);
        Ok(())
    }
}

/// Port for spawning the local player-host server process (RD9: never injects a DLL; configures via [`LAUNCH_VARIABLES`]).
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

/// Default [`ServerSpawner`] that records the launch command and environment without starting a child process.
#[derive(Debug, Default)]
pub struct NoopServerSpawner {
    last_env: Mutex<Option<BTreeMap<String, String>>>,
    last_args: Mutex<Option<Vec<String>>>,
}

impl NoopServerSpawner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last_environment(&self) -> Option<BTreeMap<String, String>> {
        self.last_env.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn last_arguments(&self) -> Option<Vec<String>> {
        self.last_args.lock().unwrap_or_else(|p| p.into_inner()).clone()
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
        *self.last_args.lock().unwrap_or_else(|p| p.into_inner()) = Some(arguments.to_vec());
        *self.last_env.lock().unwrap_or_else(|p| p.into_inner()) = Some(environment.clone());
        Ok(4242)
    }

    fn stop(&self, _pid: u32) {}
}

#[derive(Debug, Clone)]
struct ManagedSession {
    local: LocalSession,
    account: AccountId,
    handle: Handle,
    session_dir: PathBuf,
    server_pid: Option<u32>,
    local_port: u16,
    launch_env: BTreeMap<String, String>,
}

/// Concrete [`RealmHosting`] implementation managing session directories, `0600` secret files, [`LAUNCH_VARIABLES`]
/// construction, and [`TransportAbi`] sessions.
pub struct LocalRealmHosting {
    app_local_data: PathBuf,
    game_dir: PathBuf,
    transport: Arc<dyn TransportAbi>,
    backend: Arc<dyn HostBackend>,
    spawner: Arc<dyn ServerSpawner>,
    wait_ready_timeout_ms: u32,
    sessions: Mutex<BTreeMap<SessionId, ManagedSession>>,
}

impl LocalRealmHosting {
    pub fn new(
        app_local_data: impl Into<PathBuf>,
        game_dir: impl Into<PathBuf>,
        transport: Arc<dyn TransportAbi>,
    ) -> Self {
        Self {
            app_local_data: app_local_data.into(),
            game_dir: game_dir.into(),
            transport,
            backend: Arc::new(DefaultHostBackend::new()),
            spawner: Arc::new(NoopServerSpawner::new()),
            wait_ready_timeout_ms: 5_000,
            sessions: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn with_backend_and_spawner(
        app_local_data: impl Into<PathBuf>,
        game_dir: impl Into<PathBuf>,
        transport: Arc<dyn TransportAbi>,
        backend: Arc<dyn HostBackend>,
        spawner: Arc<dyn ServerSpawner>,
    ) -> Self {
        Self {
            app_local_data: app_local_data.into(),
            game_dir: game_dir.into(),
            transport,
            backend,
            spawner,
            wait_ready_timeout_ms: 5_000,
            sessions: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn session_directory(&self, session: &SessionId) -> Option<PathBuf> {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session)
            .map(|s| s.session_dir.clone())
    }

    pub fn session_launch_environment(&self, session: &SessionId) -> Option<BTreeMap<String, String>> {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session)
            .map(|s| s.launch_env.clone())
    }

    pub fn session_local_port(&self, session: &SessionId) -> Option<u16> {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session)
            .map(|s| s.local_port)
    }

    fn coop_available_sync(&self) -> Result<bool, HostError> {
        if self.app_local_data.join(DISABLE_COOP_FLAG_FILE).exists()
            || self
                .app_local_data
                .join("MysticParadox")
                .join(DISABLE_COOP_FLAG_FILE)
                .exists()
            || self.game_dir.join(DISABLE_COOP_FLAG_FILE).exists()
        {
            return Ok(false);
        }
        let version = match self.transport.abi_version() {
            Ok(v) => v,
            Err(_) => return Ok(false),
        };
        if version != ABI_VERSION {
            return Ok(false);
        }
        match self.transport.networking_ready() {
            Ok(ready) => Ok(ready),
            Err(_) => Ok(false),
        }
    }

    fn require_coop(&self) -> Result<(), HostError> {
        if !self.coop_available_sync()? {
            return Err(HostError::Transport(
                "co-op transport is disabled or not ready".to_owned(),
            ));
        }
        Ok(())
    }

    fn start_hosted_session(
        &self,
        account: AccountId,
        spec: HostBootstrapSpec,
        transport_config: TransportConfig,
    ) -> Result<LocalSession, HostError> {
        self.require_coop()?;
        let session = spec.common.session_id.clone();
        if self
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(&session)
        {
            return Err(HostError::Busy(format!("session {session} is already running")));
        }

        let hub_session = matches!(spec.role, HostedRole::Hub(_));
        let files_for = if hub_session {
            FilesFor::HubHost
        } else if spec.local_client {
            FilesFor::HuntHost
        } else {
            FilesFor::AwayHuntHost
        };
        let ice = ice_files(&transport_config, files_for, &spec.common.host_peer_id)?;

        let session_dir = create_session_directory(&self.app_local_data, &session, spec.common.generation)?;
        let mut cleanup = SessionCleanup::new(session_dir.clone());

        let suffix = random_hex(16)?;
        let ready_pipe = format!(r"\\.\pipe\MysticInternalServer.ready.{suffix}");
        let control_pipe = format!(r"\\.\pipe\MysticInternalServer.control.{suffix}");
        let guard_pipe_token = format!("MysticHuntGuard.events.{suffix}");
        let guard_pipe = format!(r"\\.\pipe\{guard_pipe_token}");
        let launch_id = format!("host-{suffix}");

        let env_spec = LaunchEnvironmentSpec {
            capability: spec.capability.clone(),
            role: spec.role,
            session_id: session.clone(),
            generation: spec.common.generation,
            launch_id,
            ready_pipe,
            control_pipe,
            guard_pipe,
            hunt_evidence: spec.hunt_evidence,
            guard_completion_mode: spec.common.guard_completion_mode.clone(),
            runtime_profile: spec.runtime_profile.clone(),
            launcher_pid: std::process::id(),
            max_players: spec.max_players,
            hub_afk_seconds: spec.hub_afk_seconds,
            trials_week: trials_week(&spec.matchmaker_hunt_id).map(str::to_owned),
        };
        let launch_env = build_launch_environment(&env_spec, std::env::vars());
        let server_args = host_server_arguments(
            spec.server_udp_port,
            &spec.map_path,
            &spec.behemoth_path,
            &spec.matchmaker_hunt_id,
            &spec.expected_players,
        );
        let game_exe = self.game_dir.join("Dauntless.exe");
        let server_pid = self
            .spawner
            .spawn(&game_exe, &server_args, &self.game_dir, &launch_env)?;

        let mut config = spec.common.to_config_lines()?;
        config.push_str(&format!(
            "backend_url={}\nserver_udp_port={}\nmaximum_peers={}\nforce_relay={}\nguard_pipe_name={}\nhunt_evidence={}\nexpected_server_pid={}\n",
            safe_config_value(&spec.backend_url, 512)?,
            spec.server_udp_port,
            spec.maximum_peers,
            spec.common.force_relay,
            guard_pipe_token,
            spec.hunt_evidence,
            server_pid
        ));
        if hub_session {
            config.push_str("session_role=HUB\nhost_local_client=1\n");
        } else if !spec.local_client {
            config.push_str("host_local_client=0\n");
        }
        config.push_str(&ice.config);

        let config_path = session_dir.join("bridge.conf");
        let secrets_path = session_dir.join("transport.secret");
        let capability_path = session_dir.join("capability.secret");
        let ticket_path = session_dir.join("ticket.secret");

        if let Err(err) = (|| -> Result<(), HostError> {
            write_session_file_0600(&config_path, config.as_bytes())?;
            write_session_file_0600(&secrets_path, ice.secrets.expose().as_bytes())?;
            write_session_file_0600(&capability_path, spec.capability.expose().as_bytes())?;
            let ticket_bytes = spec.host_ticket.as_ref().map_or(&b""[..], |t| t.expose().as_bytes());
            write_session_file_0600(&ticket_path, ticket_bytes)?;
            Ok(())
        })() {
            self.spawner.stop(server_pid);
            return Err(err);
        }

        let public_key = self.game_dir.join(ADMISSION_PUBLIC_KEY_FILE);
        let open = HostOpen {
            public_key,
            config: config_path,
            capability: capability_path,
            host_ticket: spec.local_client.then_some(ticket_path),
            secrets: secrets_path,
        };
        let handle = match self.transport.host_open(&open) {
            Ok(h) => h,
            Err(err) => {
                self.spawner.stop(server_pid);
                return Err(err);
            }
        };

        let local_port = match self.transport.wait_ready(handle, self.wait_ready_timeout_ms) {
            Ok(ReadyState::Ready(line)) => {
                if line.starts_with("HOST_READY ") {
                    match parse_ice_ready(&line, "HOST_READY", spec.common.force_relay, spec.local_client) {
                        Ok(port) => port,
                        Err(err) => {
                            let _ = self.transport.stop_session(handle);
                            self.spawner.stop(server_pid);
                            return Err(err);
                        }
                    }
                } else if spec.local_client {
                    spec.server_udp_port
                } else {
                    0
                }
            }
            Ok(ReadyState::TimedOut) => {
                let _ = self.transport.stop_session(handle);
                self.spawner.stop(server_pid);
                return Err(HostError::Transport(
                    "transport timed out waiting for HOST_READY".to_owned(),
                ));
            }
            Ok(ReadyState::Ended) => {
                let _ = self.transport.stop_session(handle);
                self.spawner.stop(server_pid);
                return Err(HostError::Transport(
                    "transport session ended before HOST_READY".to_owned(),
                ));
            }
            Err(err) => {
                let _ = self.transport.stop_session(handle);
                self.spawner.stop(server_pid);
                return Err(err);
            }
        };

        if spec.local_client
            && let Err(err) = self
                .backend
                .route_ready(&account, &session, spec.common.generation, local_port)
        {
            let _ = self.transport.stop_session(handle);
            self.spawner.stop(server_pid);
            return Err(err);
        }

        let local = LocalSession {
            session: session.clone(),
            generation: spec.common.generation,
            role: spec.role,
            hosting: true,
            state: LocalState::Running,
        };
        self.sessions.lock().unwrap_or_else(|p| p.into_inner()).insert(
            session,
            ManagedSession {
                local: local.clone(),
                account,
                handle,
                session_dir,
                server_pid: Some(server_pid),
                local_port,
                launch_env,
            },
        );
        cleanup.commit();
        Ok(local)
    }

    fn start_joined_session(
        &self,
        account: AccountId,
        spec: JoinBootstrapSpec,
        transport_config: TransportConfig,
    ) -> Result<LocalSession, HostError> {
        self.require_coop()?;
        let session = spec.common.session_id.clone();
        if self
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(&session)
        {
            return Err(HostError::Busy(format!("session {session} is already running")));
        }

        let ice = ice_files(&transport_config, FilesFor::Joiner, &spec.common.host_peer_id)?;
        let session_dir = create_session_directory(&self.app_local_data, &session, spec.common.generation)?;
        let mut cleanup = SessionCleanup::new(session_dir.clone());

        let mut config = spec.common.to_config_lines()?;
        config.push_str(&format!(
            "local_udp_port=0\nforce_relay={}\n{}",
            spec.common.force_relay, ice.config
        ));

        let config_path = session_dir.join("bridge.conf");
        let secrets_path = session_dir.join("transport.secret");
        let capability_path = session_dir.join("capability.secret");
        let ticket_path = session_dir.join("ticket.secret");

        write_session_file_0600(&config_path, config.as_bytes())?;
        write_session_file_0600(&secrets_path, ice.secrets.expose().as_bytes())?;
        write_session_file_0600(&capability_path, b"")?;
        write_session_file_0600(&ticket_path, spec.ticket.expose().as_bytes())?;

        let public_key = self.game_dir.join(ADMISSION_PUBLIC_KEY_FILE);
        let open = JoinOpen {
            public_key,
            config: config_path,
            ticket: ticket_path,
            secrets: secrets_path,
        };
        let handle = self.transport.join_open(&open)?;
        let local_port = match self.transport.wait_ready(handle, self.wait_ready_timeout_ms) {
            Ok(ReadyState::Ready(line)) => {
                if line.starts_with("ROUTE_READY ") {
                    match parse_ice_ready(&line, "ROUTE_READY", spec.common.force_relay, true) {
                        Ok(port) => port,
                        Err(err) => {
                            let _ = self.transport.stop_session(handle);
                            return Err(err);
                        }
                    }
                } else {
                    29_001
                }
            }
            Ok(ReadyState::TimedOut) => {
                let _ = self.transport.stop_session(handle);
                return Err(HostError::Transport(
                    "transport timed out waiting for ROUTE_READY".to_owned(),
                ));
            }
            Ok(ReadyState::Ended) => {
                let _ = self.transport.stop_session(handle);
                return Err(HostError::Transport(
                    "transport session ended before ROUTE_READY".to_owned(),
                ));
            }
            Err(err) => {
                let _ = self.transport.stop_session(handle);
                return Err(err);
            }
        };

        if let Err(err) = self
            .backend
            .route_ready(&account, &session, spec.common.generation, local_port)
        {
            let _ = self.transport.stop_session(handle);
            return Err(err);
        }

        let local = LocalSession {
            session: session.clone(),
            generation: spec.common.generation,
            role: spec.role,
            hosting: false,
            state: LocalState::Running,
        };
        self.sessions.lock().unwrap_or_else(|p| p.into_inner()).insert(
            session,
            ManagedSession {
                local: local.clone(),
                account,
                handle,
                session_dir,
                server_pid: None,
                local_port,
                launch_env: BTreeMap::new(),
            },
        );
        cleanup.commit();
        Ok(local)
    }

    fn stop_managed(&self, managed: ManagedSession) -> Result<(), HostError> {
        let stop_res = self.transport.stop_session(managed.handle);
        if let Some(pid) = managed.server_pid {
            self.spawner.stop(pid);
        }
        cleanup_directory(&managed.session_dir);
        stop_res
    }
}

impl RealmHosting for LocalRealmHosting {
    fn coop_available(&self) -> BoxFuture<'_, Result<bool, HostError>> {
        let res = self.coop_available_sync();
        Box::pin(async move { res })
    }

    fn host_hunt(&self, account: AccountId, session: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        let res = self
            .backend
            .host_bootstrap(&account, &session)
            .and_then(|(spec, cfg)| self.start_hosted_session(account, spec, cfg));
        Box::pin(async move { res })
    }

    fn join_hunt(&self, account: AccountId, session: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        let res = self
            .backend
            .join_bootstrap(&account, &session)
            .and_then(|(spec, cfg)| self.start_joined_session(account, spec, cfg));
        Box::pin(async move { res })
    }

    fn start_hub(
        &self,
        account: AccountId,
        kind: HubKind,
        limits: HubLimits,
    ) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        let res = self
            .backend
            .start_hub(&account, kind, &limits)
            .and_then(|(spec, cfg)| self.start_hosted_session(account, spec, cfg));
        Box::pin(async move { res })
    }

    fn stop_hub(&self, account: AccountId, kind: HubKind) -> BoxFuture<'_, Result<(), HostError>> {
        let res = (|| -> Result<(), HostError> {
            let backend_session = self.backend.stop_hub(&account, kind)?;
            let key = {
                let sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
                backend_session.filter(|s| sessions.contains_key(s)).or_else(|| {
                    sessions
                        .iter()
                        .find(|(_, m)| m.account == account && m.local.role == HostedRole::Hub(kind))
                        .map(|(k, _)| k.clone())
                })
            };
            let Some(key) = key else {
                return Err(HostError::NotFound("no running hub session for that kind".to_owned()));
            };
            let managed = self
                .sessions
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&key)
                .ok_or_else(|| HostError::NotFound("hub session already stopped".to_owned()))?;
            self.stop_managed(managed)
        })();
        Box::pin(async move { res })
    }

    fn stop(&self, session: SessionId) -> BoxFuture<'_, Result<(), HostError>> {
        let res = (|| -> Result<(), HostError> {
            let managed = self
                .sessions
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&session)
                .ok_or_else(|| HostError::NotFound(format!("session {session} is not running")))?;
            self.stop_managed(managed)
        })();
        Box::pin(async move { res })
    }

    fn sessions(&self) -> BoxFuture<'_, Result<Vec<LocalSession>, HostError>> {
        let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        let mut out = Vec::with_capacity(sessions.len());
        for managed in sessions.values_mut() {
            if matches!(
                managed.local.state,
                LocalState::Running | LocalState::Ready | LocalState::Starting
            ) && !self.transport.session_running(managed.handle).unwrap_or(false)
            {
                let code = self
                    .transport
                    .last_session_result(managed.handle)
                    .unwrap_or(SessionResult::Clean);
                managed.local.state = LocalState::Ended(code);
                cleanup_directory(&managed.session_dir);
            }
            out.push(managed.local.clone());
        }
        drop(sessions);
        Box::pin(async move { Ok(out) })
    }
}
