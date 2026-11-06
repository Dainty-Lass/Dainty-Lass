// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The pure rules: id and value checks, the ICE files, ready and peer lines, the roster, the hosted server's launch,
//! and the timers and limits of the hosting panel.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    ops::RangeInclusive,
    path::Path,
    time::{Duration, Instant},
};

use rifts_seam::{Secret, SessionId};

use super::*;
use crate::HostError;

// ---- ids and values -------------------------------------------------------------------------------------------

/// The ULID's 128 bits as a dashed 8-4-4-4-12 hex id; the node is not part of it.
pub fn wire_session_id(session: &SessionId) -> String {
    let mut out = String::with_capacity(36);
    for (at, digit) in format!("{:032x}", session.0.ulid.0).chars().enumerate() {
        if matches!(at, 8 | 12 | 16 | 20) {
            out.push('-');
        }
        out.push(digit);
    }
    out
}

/// The value if it is 1 to `maximum` bytes on one line; otherwise the backend's data is refused as unsafe.
pub fn safe_config_value(value: &str, maximum: usize) -> Result<&str, HostError> {
    let one_line = !value.contains(['\r', '\n', '\0']);
    ((1..=maximum).contains(&value.len()) && one_line)
        .then_some(value)
        .ok_or_else(|| HostError::InvalidInput("the backend returned unsafe player-host bootstrap data".to_owned()))
}

/// Length in `lengths` and every byte passing `byte`.
fn shaped(text: &str, lengths: RangeInclusive<usize>, byte: impl Fn(u8) -> bool) -> bool {
    lengths.contains(&text.len()) && text.bytes().all(byte)
}

fn printable(text: &str) -> bool {
    text.bytes().all(|b| b.is_ascii_graphic())
}

/// 1 to 64 ASCII alphanumerics, `-`, `_` or `:`.
pub fn safe_account(text: &str) -> bool {
    shaped(text, 1..=64, |b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
}

/// A nonzero decimal u64 with no leading zero.
pub fn safe_peer_id(text: &str) -> bool {
    // Decimal with no leading zero that still fits a u64.
    shaped(text, 1..=20, |b| b.is_ascii_digit()) && !text.starts_with('0') && text.parse::<u64>().is_ok()
}

/// 43 base64url characters: a 32-byte pair key.
pub fn safe_key(text: &str) -> bool {
    shaped(text, 43..=43, |b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

/// A `stun:`, `turn:` or `turns:` URL of at most 256 printable bytes and no comma.
pub fn safe_url(text: &str) -> bool {
    let scheme_ok = text.starts_with("stun:") || text.starts_with("turn:") || text.starts_with("turns:");
    scheme_ok && text.len() <= 256 && printable(text) && !text.contains(',')
}

/// 1 to 512 printable ASCII bytes.
pub fn safe_credential(text: &str) -> bool {
    (1..=512).contains(&text.len()) && printable(text)
}

// ---- ICE files ------------------------------------------------------------------------------------------------

fn unsafe_transport_config() -> HostError {
    HostError::InvalidInput("the backend returned an unsafe co-op transport configuration".to_owned())
}

/// The STUN and TURN URLs to write and the one TURN credential, from the first server that has one.
#[derive(Default)]
struct IceUrls<'a> {
    stun: Vec<&'a str>,
    turn: Vec<&'a str>,
    credential: Option<(&'a str, &'a str)>,
}

impl<'a> IceUrls<'a> {
    fn gather(servers: &'a [IceServerEntry]) -> Result<Self, HostError> {
        let mut urls = Self::default();
        for server in servers {
            if server.urls.iter().any(|u| !safe_url(u)) {
                return Err(unsafe_transport_config());
            }
            let with = |prefix: &'a str| {
                server
                    .urls
                    .iter()
                    .map(String::as_str)
                    .filter(move |u| u.starts_with(prefix))
            };
            let Some(pair) = server.username.as_deref().zip(server.credential.as_deref()) else {
                urls.stun.extend(with("stun:"));
                continue;
            };
            if !(safe_credential(pair.0) && safe_credential(pair.1)) {
                return Err(unsafe_transport_config());
            }
            if urls.credential.is_none() {
                urls.credential = Some(pair);
                urls.turn.extend(with("turn"));
            }
        }
        Ok(urls)
    }
}

/// The peer list's shape for who the files are for; a hub host takes its peers later, so any list fits.
fn peers_fit(config: &TransportConfig, files_for: FilesFor) -> bool {
    let peers = &config.peers;
    let entries_ok = peers
        .iter()
        .all(|p| safe_account(&p.account_id) && safe_peer_id(&p.peer_id) && safe_key(&p.transport_key));
    let none_local = || peers.iter().all(|p| p.account_id != config.local_account_id);
    entries_ok
        && match files_for {
            FilesFor::Joiner => {
                peers.len() == 1
                    && peers[0].account_id == config.host_account_id
                    && peers[0].peer_id == config.host_peer_id
            }
            FilesFor::HuntHost => peers.len() <= 3 && none_local(),
            FilesFor::AwayHuntHost => peers.len() <= 4 && none_local(),
            FilesFor::HubHost => true,
        }
}

fn push_line(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push('=');
    out.push_str(value);
    out.push('\n');
}

/// The session's ICE files from the backend's transport config; refused when an id is unsafe, the host peer id
/// differs or the peers do not fit `files_for`. A hub host gets no peer lines.
pub fn ice_files(config: &TransportConfig, files_for: FilesFor, host_peer_id: &str) -> Result<IceFiles, HostError> {
    let accounts_ok = safe_account(&config.local_account_id) && safe_account(&config.host_account_id);
    let peers_ok = safe_peer_id(&config.local_peer_id) && safe_peer_id(&config.host_peer_id);
    if !(accounts_ok && peers_ok && config.host_peer_id == host_peer_id) {
        return Err(unsafe_transport_config());
    }
    let urls = IceUrls::gather(&config.ice_servers)?;
    let hub = files_for == FilesFor::HubHost;
    if !hub && !peers_fit(config, files_for) {
        return Err(unsafe_transport_config());
    }

    let mut public = String::new();
    for (key, list) in [("ice_stun_urls", &urls.stun), ("ice_turn_urls", &urls.turn)] {
        if !list.is_empty() {
            push_line(&mut public, key, &list.join(","));
        }
    }
    push_line(&mut public, "local_account_id", &config.local_account_id);
    if files_for == FilesFor::Joiner {
        push_line(&mut public, "local_peer_id", &config.local_peer_id);
        push_line(&mut public, "host_account_id", &config.host_account_id);
    }

    let mut secret = String::new();
    if let (Some((user, credential)), false) = (urls.credential, urls.turn.is_empty()) {
        push_line(&mut secret, "turn_username", user);
        push_line(&mut secret, "turn_credential", credential);
    }
    for peer in config.peers.iter().filter(|_| !hub) {
        let entry = [peer.account_id.as_str(), &peer.peer_id, &peer.transport_key].join(",");
        push_line(&mut secret, "peer", &entry);
    }
    Ok(IceFiles {
        config: public,
        secrets: Secret::new(secret),
    })
}

// ---- transport lines ------------------------------------------------------------------------------------------

/// `key=value` tokens; `None` on an empty side or a key given twice.
fn route_fields<'a>(tokens: impl Iterator<Item = &'a str>) -> Option<HashMap<&'a str, &'a str>> {
    let mut fields = HashMap::new();
    for token in tokens {
        let (key, value) = token.split_once('=')?;
        if key.is_empty() || value.is_empty() || fields.insert(key, value).is_some() {
            return None;
        }
    }
    Some(fields)
}

/// The local port from a `HOST_READY` or `ROUTE_READY` line, after checking it is the loopback ICE route asked for.
pub fn parse_ice_ready(
    line: &str,
    expected_state: &str,
    force_relay: bool,
    local_client: bool,
) -> Result<u16, HostError> {
    let refused = |text: &str| HostError::Transport(format!("the co-op transport module {text}"));
    let mut tokens = line.split_whitespace();
    if tokens.next() != Some(expected_state) {
        return Err(refused("returned an unexpected readiness state"));
    }
    let fields = route_fields(tokens).ok_or_else(|| refused("returned a malformed local route"))?;
    let is = |key: &str, wanted: &str| fields.get(key) == Some(&wanted);
    if !(is("localHost", "127.0.0.1") && is("transport", "ice")) {
        return Err(refused("did not return a verified loopback route"));
    }
    let policy_ok = is("routePolicy", if force_relay { "RELAY_ONLY" } else { "AUTOMATIC" });
    let relayed_when_forced = !force_relay || expected_state != "ROUTE_READY" || is("route", "relay");
    if !(policy_ok && relayed_when_forced) {
        return Err(refused("returned a route outside the requested transport policy"));
    }
    // The port is 0 exactly when no local client is to connect.
    fields
        .get("localPort")
        .and_then(|p| p.parse::<u16>().ok())
        .filter(|port| (*port != 0) == local_client)
        .ok_or_else(|| refused("returned an invalid local route"))
}

/// The peer event a transport line reports, `None` for any other line.
pub fn parse_peer_event(line: &str) -> Option<PeerEvent> {
    let mut tokens = line.split_whitespace();
    let kind = tokens.next()?;
    let fields: HashMap<&str, &str> = tokens.filter_map(|t| t.split_once('=')).collect();
    let owned = |key: &str| fields.get(key).map(|v| (*v).to_owned());
    let account = owned("account")?;
    let (route, reason) = (owned("route"), owned("reason"));
    let admitted = fields.get("admitted") == Some(&"1");
    match kind {
        "PEER_CONNECTED" => Some(PeerEvent::Connected { account, route }),
        "PEER_ADMITTED" => Some(PeerEvent::Admitted { account, route }),
        "PEER_LOST" => Some(PeerEvent::Lost { account, reason }),
        "PEER_REMOVED" => Some(PeerEvent::Removed {
            account,
            reason,
            admitted,
        }),
        "PEER_ADD_REJECTED" => Some(PeerEvent::AddRejected { account, reason }),
        _ => None,
    }
}

/// Whether the line admits the host's own local game client.
pub fn is_local_admission(line: &str) -> bool {
    let mut tokens = line.split_whitespace();
    if tokens.next() != Some("ADMISSION_ACCEPTED") {
        return false;
    }
    let (mut as_host, mut from_local) = (false, false);
    for token in tokens {
        as_host |= token == "role=host";
        from_local |= token == "source=local";
    }
    as_host && from_local
}

/// Applies a peer event to the panel's view: an admission adds the peer, a loss or removal drops it.
pub fn apply_peer_event(live: &mut SessionLive, event: &PeerEvent, now_iso: &str) {
    if let PeerEvent::Admitted { account, route } = event {
        let entry = PeerLive {
            route: route.clone(),
            since: now_iso.to_owned(),
        };
        live.peers.insert(account.clone(), entry);
    } else if let PeerEvent::Lost { account, .. } | PeerEvent::Removed { account, .. } = event {
        live.peers.remove(account);
    }
}

/// Applies a transport line to the panel's view, if it is a peer event or the local admission.
pub fn apply_peer_line(live: &mut SessionLive, line: &str, now_iso: &str) {
    if is_local_admission(line) {
        live.local_since = Some(now_iso.to_owned());
        return;
    }
    if let Some(event) = parse_peer_event(line) {
        apply_peer_event(live, &event, now_iso);
    }
}

// ---- roster ---------------------------------------------------------------------------------------------------

fn checked_peer(entry: &PeerListEntry, host_account_id: Option<&str>) -> Option<WantedPeer> {
    let not_host = host_account_id != Some(entry.account_id.as_str());
    let fields_ok = safe_account(&entry.account_id) && safe_key(&entry.transport_key) && safe_peer_id(&entry.peer_id);
    if !(fields_ok && not_host) {
        return None;
    }
    Some(WantedPeer {
        account_id: entry.account_id.clone(),
        peer_id: entry.peer_id.parse().ok()?,
        transport_key: entry.transport_key.clone(),
    })
}

/// The peers worth linking, in order, and how many entries were refused.
pub fn wanted_peers(entries: &[PeerListEntry], host_account_id: Option<&str>) -> (Vec<WantedPeer>, usize) {
    let mut refused = 0;
    let mut seen = HashSet::new();
    let wanted = entries
        .iter()
        .take(MAX_HUB_PEERS)
        .filter_map(|entry| match checked_peer(entry, host_account_id) {
            // A repeat of an account already taken is dropped, not counted.
            Some(peer) => seen.insert(peer.account_id.clone()).then_some(peer),
            None => {
                refused += 1;
                None
            }
        })
        .collect();
    (wanted, refused)
}

/// The adds and removes that turn the linked accounts into the wanted roster.
pub fn peer_diff(linked: &HashSet<String>, wanted: &[WantedPeer]) -> PeerDiff {
    let wanted_ids: HashSet<&str> = wanted.iter().map(|w| w.account_id.as_str()).collect();
    let remove: BTreeSet<&String> = linked.iter().filter(|a| !wanted_ids.contains(a.as_str())).collect();
    PeerDiff {
        add: wanted
            .iter()
            .filter(|w| !linked.contains(&w.account_id))
            .cloned()
            .collect(),
        remove: remove.into_iter().cloned().collect(),
    }
}

// ---- the hosted server's launch -------------------------------------------------------------------------------

/// The week of a Trials matchmaker hunt id, `Arena_MatchmakerHunt_<Easy|Hard|Elite>_<three digits>`.
pub fn trials_week(matchmaker_hunt_id: &str) -> Option<&str> {
    let rest = matchmaker_hunt_id.strip_prefix("Arena_MatchmakerHunt_")?;
    let week = ["Easy_", "Hard_", "Elite_"]
        .iter()
        .find_map(|tier| rest.strip_prefix(tier))?;
    shaped(week, 3..=3, |b| b.is_ascii_digit()).then_some(week)
}

/// The hosted server's command line; the runtime checks the leading `-` and the loopback multihome.
pub fn host_server_arguments(
    server_udp_port: u16,
    map_path: &str,
    behemoth_path: &str,
    matchmaker_hunt_id: &str,
    expected_players: &str,
) -> Vec<String> {
    let port = server_udp_port.to_string();
    let loopback = format!("127.0.0.1:{port}");
    let positional = [
        "-",
        &port,
        map_path,
        behemoth_path,
        matchmaker_hunt_id,
        expected_players,
        &loopback,
    ];
    positional
        .into_iter()
        .chain(DIRECTOR_SERVER_SWITCHES)
        .chain(["-MULTIHOME=127.0.0.1"])
        .map(str::to_owned)
        .collect()
}

/// One argument as the Microsoft C runtime reads it back: backslashes double only before a quote.
fn push_quoted(out: &mut String, argument: &str) {
    let wrap = argument.is_empty() || argument.contains([' ', '\t']);
    let mut pending = 0_usize;
    let flush = |out: &mut String, count: usize| out.extend(std::iter::repeat_n('\\', count));
    if wrap {
        out.push('"');
    }
    for c in argument.chars() {
        if c == '\\' {
            pending += 1;
            continue;
        }
        let quote = c == '"';
        flush(out, if quote { 2 * pending + 1 } else { pending });
        out.push(c);
        pending = 0;
    }
    flush(out, if wrap { 2 * pending } else { pending });
    if wrap {
        out.push('"');
    }
}

/// Quotes as the Microsoft C runtime splits (`CommandLineToArgvW`'s inverse).
pub fn windows_command_line(program: &Path, arguments: &[String]) -> String {
    let mut out = format!("\"{}\"", program.to_string_lossy());
    for argument in arguments {
        out.push(' ');
        push_quoted(&mut out, argument);
    }
    out
}

type LaunchValue = fn(&LaunchEnvironmentSpec) -> Option<String>;

fn hub_only<T: ToString>(spec: &LaunchEnvironmentSpec, value: Option<T>) -> Option<String> {
    value
        .filter(|_| matches!(spec.role, HostedRole::Hub(_)))
        .map(|v| v.to_string())
}

// Each launch variable and its value; `None` leaves it unset. Also the order of `LAUNCH_VARIABLES`.
const LAUNCH: [(&str, LaunchValue); 17] = [
    (rifts_var!("AUTH_MODE"), |_| Some("HUNT_CAPABILITY".to_owned())),
    (rifts_var!("BIND_ADDRESS"), |_| Some("127.0.0.1".to_owned())),
    (rifts_var!("HOST_RUNTIME_CAPABILITY"), |s| {
        Some(s.capability.expose().to_owned())
    }),
    (rifts_var!("HOSTED_ROLE"), |s| Some(s.role.wire_role().to_owned())),
    (rifts_var!("SESSION_ID"), |s| Some(wire_session_id(&s.session_id))),
    (rifts_var!("SESSION_GENERATION"), |s| Some(s.generation.to_string())),
    (rifts_var!("GAMESERVER_LAUNCH_ID"), |s| Some(s.launch_id.clone())),
    (rifts_var!("READY_PIPE"), |s| Some(s.ready_pipe.clone())),
    (rifts_var!("CONTROL_PIPE"), |s| Some(s.control_pipe.clone())),
    (rifts_var!("GUARD_PIPE"), |s| Some(s.guard_pipe.clone())),
    (rifts_var!("HUNT_GUARD_EVENTS"), |s| {
        Some(u8::from(s.hunt_evidence).to_string())
    }),
    (rifts_var!("HUNT_GUARD_COMPLETION_MODE"), |s| {
        Some(s.guard_completion_mode.clone())
    }),
    (rifts_var!("RUNTIME_PROFILE"), |s| Some(s.runtime_profile.clone())),
    (rifts_var!("LAUNCHER_PID"), |s| Some(s.launcher_pid.to_string())),
    (rifts_var!("MAX_PLAYERS"), |s| hub_only(s, s.max_players)),
    (rifts_var!("HUB_AFK_SECONDS"), |s| hub_only(s, s.hub_afk_seconds)),
    (rifts_var!("TRIALS_WEEK"), |s| s.trials_week.clone()),
];

pub(super) const fn launch_variable_names() -> [&'static str; 17] {
    let mut names = [""; 17];
    let mut at = 0;
    while at < LAUNCH.len() {
        names[at] = LAUNCH[at].0;
        at += 1;
    }
    names
}

/// The hosted server's environment: the inherited one without our variables, in any letter case, then ours from `spec`.
pub fn build_launch_environment(
    spec: &LaunchEnvironmentSpec,
    inherited: impl IntoIterator<Item = (String, String)>,
) -> BTreeMap<String, String> {
    // Windows names are case-insensitive, so a differently cased copy of ours is dropped too.
    let ours = |key: &str| {
        let mut names = CLEARED_VARIABLES.iter().chain(&LAUNCH_VARIABLES);
        names.any(|name| name.eq_ignore_ascii_case(key))
    };
    let set = LAUNCH
        .iter()
        .filter_map(|(name, value)| value(spec).map(|v| ((*name).to_owned(), v)));
    inherited.into_iter().filter(|(key, _)| !ours(key)).chain(set).collect()
}

// ---- the central log ------------------------------------------------------------------------------------------

const LOG_LINE_MAX: usize = 2048;
const SECRET_WORDS: [&str; 8] = [
    "ticket",
    "capability",
    "token",
    "challenge",
    "secret",
    "signature",
    "password",
    "auth_password",
];

fn redacted(token: &str) -> String {
    let lower = token.to_ascii_lowercase();
    if !SECRET_WORDS.iter().any(|word| lower.contains(word)) {
        return token.to_owned();
    }
    token
        .split_once('=')
        .map_or_else(|| "[redacted]".to_owned(), |(key, _)| format!("{key}=[redacted]"))
}

/// A transport line made safe for the central log: any token naming a secret loses its value.
pub fn sanitize_central_log_line(line: &str) -> String {
    let mut out = String::new();
    for (at, token) in line.split_whitespace().enumerate() {
        if at > 0 {
            out.push(' ');
        }
        out += &redacted(token);
        if out.len() >= LOG_LINE_MAX {
            return out.chars().take(LOG_LINE_MAX).collect();
        }
    }
    out
}

// ---- traffic, relay and the hosting panel's timers ------------------------------------------------------------

/// The `localRx=` count of a `UDP_STATS` line.
pub fn udp_stats_local_rx(line: &str) -> Option<u64> {
    let reading = line.split_whitespace().find_map(|t| t.strip_prefix("localRx="));
    reading.filter(|_| line.contains("UDP_STATS"))?.parse().ok()
}

impl GameTrafficWatch {
    /// A watch whose traffic last changed at `now`.
    pub fn new(now: Instant) -> Self {
        Self {
            reading: None,
            changed: now,
        }
    }

    /// Takes a `UDP_STATS` line; the watch restarts when the local count grew.
    pub fn observe(&mut self, line: &str, now: Instant) {
        if let Some(reading) = udp_stats_local_rx(line) {
            let grew = self.reading.is_none_or(|previous| reading > previous);
            self.reading = Some(reading);
            if grew {
                self.restart(now);
            }
        }
    }

    /// Treats the traffic as changed at `now`.
    pub fn restart(&mut self, now: Instant) {
        self.changed = now;
    }

    /// How long the local traffic has stood still.
    pub fn idle(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.changed)
    }
}

/// `game_left` once the game's traffic stood still long enough, `timeout` once the wait ran out, else `None`.
pub fn hunt_join_leave_outcome(asked_at: Instant, traffic: &GameTrafficWatch, now: Instant) -> Option<&'static str> {
    let waited = now.saturating_duration_since(asked_at);
    [
        (traffic.idle(now) >= GAME_LEFT_IDLE, "game_left"),
        (waited >= HUNT_JOIN_LEAVE_MAX, "timeout"),
    ]
    .into_iter()
    .find_map(|(due, outcome)| due.then_some(outcome))
}

/// Whether to refresh the relay grant now: inside the margin always, inside the early window once `RELAY_EARLY_REFRESH_SPACING` has passed since the last one.
pub fn relay_refresh_due(seconds_left: Option<i64>, since_last: Option<Duration>) -> bool {
    seconds_left.is_some_and(|left| {
        let spaced = since_last.is_none_or(|s| s >= RELAY_EARLY_REFRESH_SPACING);
        left <= RELAY_REFRESH_MARGIN_SECONDS || (spaced && left <= RELAY_EARLY_REFRESH_SECONDS)
    })
}

/// Whether the relay grant has 5 minutes or less left, so the session must restart.
pub fn relay_restart_due(seconds_left: Option<i64>) -> bool {
    matches!(seconds_left, Some(left) if left <= 300)
}

/// The default hub server limit for a PC with this much memory, in GB: 1 to 6.
pub fn default_max_servers(physical_gb: u64) -> u8 {
    // Two servers per 3 GB above the first 4, from 1 to 6.
    let spare = physical_gb.saturating_sub(4);
    let servers = if physical_gb < 4 {
        1
    } else {
        (spare * 2 / 3).clamp(1, 6)
    };
    u8::try_from(servers).unwrap_or(6)
}

/// Accepts another server only above the commit floor and under the server limit.
pub fn capacity_decision(commit_mib: u64, active_servers: u8, max_servers: u8) -> CapacityDecision {
    let room = commit_mib >= HOST_COMMIT_FLOOR_MIB && active_servers < max_servers;
    if room {
        CapacityDecision::Accept
    } else {
        CapacityDecision::Refuse { reason: "HOST_BUSY" }
    }
}

const RESTART_WINDOW: Duration = Duration::from_secs(600);
const BACKOFF_SECONDS: [u64; 3] = [5, 15, 60];

/// The wait before a restart after this many recent ones: 5, 15, then 60 seconds.
pub fn restart_backoff(attempts_in_window: usize) -> Duration {
    let step = attempts_in_window.min(BACKOFF_SECONDS.len() - 1);
    Duration::from_secs(BACKOFF_SECONDS[step])
}

/// Restart attempts within the last 10 minutes.
pub fn attempts_in_window(attempts: &[Instant], now: Instant) -> usize {
    let recent = |at: &&Instant| now.saturating_duration_since(**at) < RESTART_WINDOW;
    attempts.iter().filter(recent).count()
}

/// Whether fewer than 5 restarts were tried within the last 10 minutes.
pub fn restart_allowed(attempts: &[Instant], now: Instant) -> bool {
    attempts_in_window(attempts, now) < 5
}

/// When the hub became empty, `None` while anyone is present.
pub fn next_idle_since(previous: Option<Instant>, present: usize, now: Instant) -> Option<Instant> {
    (present == 0).then(|| previous.unwrap_or(now))
}

/// Whether the Training hub has been empty for 5 minutes.
pub fn training_should_stop(idle_since: Option<Instant>, now: Instant) -> bool {
    const TRAINING_IDLE: Duration = Duration::from_secs(5 * 60);
    matches!(idle_since, Some(since) if now.saturating_duration_since(since) >= TRAINING_IDLE)
}

/// Whether an empty hub has been up more than 20 hours and should restart.
pub fn scheduled_restart_due(uptime: Duration, present: bool) -> bool {
    const SCHEDULED_UPTIME: Duration = Duration::from_secs(20 * 60 * 60);
    uptime > SCHEDULED_UPTIME && !present
}
