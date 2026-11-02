// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The pure rules: id and value checks, the ICE files, ready and peer lines, the roster, the hosted server's launch,
//! and the timers and limits of the hosting panel.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use rifts_seam::{Secret, SessionId};

use super::{
    CLEARED_VARIABLES, CapacityDecision, DIRECTOR_SERVER_SWITCHES, FilesFor, GAME_LEFT_IDLE, GameTrafficWatch,
    HOST_COMMIT_FLOOR_MIB, HUNT_JOIN_LEAVE_MAX, HostedRole, IceFiles, LAUNCH_VARIABLES, LaunchEnvironmentSpec,
    MAX_HUB_PEERS, PeerDiff, PeerEvent, PeerListEntry, PeerLive, RELAY_EARLY_REFRESH_SECONDS,
    RELAY_EARLY_REFRESH_SPACING, RELAY_REFRESH_MARGIN_SECONDS, SessionLive, TransportConfig, WantedPeer,
};
use crate::HostError;

/// The ULID's 128 bits as a dashed 8-4-4-4-12 hex id; the node is not part of it.
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
    let fine = !value.is_empty() && value.len() <= maximum && !value.contains(['\r', '\n', '\0']);
    if fine {
        Ok(value)
    } else {
        Err(HostError::InvalidInput(
            "the backend returned unsafe player-host bootstrap data".to_owned(),
        ))
    }
}

pub fn safe_account(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':'))
}

pub fn safe_peer_id(text: &str) -> bool {
    (1..=20).contains(&text.len())
        && text.bytes().all(|b| b.is_ascii_digit())
        && !text.starts_with('0')
        && text.parse::<u64>().is_ok()
}

pub fn safe_key(text: &str) -> bool {
    text.len() == 43
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn printable(text: &str) -> bool {
    text.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

pub fn safe_url(text: &str) -> bool {
    text.len() <= 256
        && ["stun:", "turn:", "turns:"].iter().any(|p| text.starts_with(p))
        && printable(text)
        && !text.contains(',')
}

pub fn safe_credential(text: &str) -> bool {
    (1..=512).contains(&text.len()) && printable(text)
}

pub fn ice_files(config: &TransportConfig, files_for: FilesFor, host_peer_id: &str) -> Result<IceFiles, HostError> {
    let unsafe_config =
        || HostError::InvalidInput("the backend returned an unsafe co-op transport configuration".to_owned());
    let joiner = files_for == FilesFor::Joiner;
    let hub = files_for == FilesFor::HubHost;
    let ids_ok = safe_account(&config.local_account_id)
        && safe_account(&config.host_account_id)
        && safe_peer_id(&config.local_peer_id)
        && safe_peer_id(&config.host_peer_id)
        && config.host_peer_id == host_peer_id;
    if !ids_ok {
        return Err(unsafe_config());
    }

    let mut stun: Vec<&str> = Vec::new();
    let mut turn: Vec<&str> = Vec::new();
    let mut credential: Option<(&str, &str)> = None;
    for server in &config.ice_servers {
        if !server.urls.iter().all(|u| safe_url(u)) {
            return Err(unsafe_config());
        }
        match (server.username.as_deref(), server.credential.as_deref()) {
            (Some(user), Some(secret)) => {
                if !safe_credential(user) || !safe_credential(secret) {
                    return Err(unsafe_config());
                }
                if credential.is_none() {
                    credential = Some((user, secret));
                    turn.extend(server.urls.iter().filter(|u| u.starts_with("turn")).map(String::as_str));
                }
            }
            _ => stun.extend(
                server
                    .urls
                    .iter()
                    .filter(|u| u.starts_with("stun:"))
                    .map(String::as_str),
            ),
        }
    }

    if !hub {
        let entries_ok = config
            .peers
            .iter()
            .all(|p| safe_account(&p.account_id) && safe_peer_id(&p.peer_id) && safe_key(&p.transport_key));
        let shape_ok = match files_for {
            FilesFor::Joiner => {
                matches!(config.peers.as_slice(), [only] if only.account_id == config.host_account_id
                    && only.peer_id == config.host_peer_id)
            }
            FilesFor::HuntHost | FilesFor::AwayHuntHost => {
                let most = if files_for == FilesFor::HuntHost { 3 } else { 4 };
                config.peers.len() <= most && config.peers.iter().all(|p| p.account_id != config.local_account_id)
            }
            FilesFor::HubHost => true,
        };
        if !entries_ok || !shape_ok {
            return Err(unsafe_config());
        }
    }

    let mut text = String::new();
    if !stun.is_empty() {
        text.push_str(&format!("ice_stun_urls={}\n", stun.join(",")));
    }
    if !turn.is_empty() {
        text.push_str(&format!("ice_turn_urls={}\n", turn.join(",")));
    }
    text.push_str(&format!("local_account_id={}\n", config.local_account_id));
    if joiner {
        text.push_str(&format!(
            "local_peer_id={}\nhost_account_id={}\n",
            config.local_peer_id, config.host_account_id
        ));
    }

    let mut secrets = String::new();
    if let Some((user, secret)) = credential.filter(|_| !turn.is_empty()) {
        secrets.push_str(&format!("turn_username={user}\nturn_credential={secret}\n"));
    }
    if !hub {
        for peer in &config.peers {
            secrets.push_str(&format!(
                "peer={},{},{}\n",
                peer.account_id, peer.peer_id, peer.transport_key
            ));
        }
    }
    Ok(IceFiles {
        config: text,
        secrets: Secret::new(secrets),
    })
}

/// The local port from a `HOST_READY` or `ROUTE_READY` line, after checking it is the loopback ICE route asked for.
pub fn parse_ice_ready(
    line: &str,
    expected_state: &str,
    force_relay: bool,
    local_client: bool,
) -> Result<u16, HostError> {
    let fail = |text: &str| Err(HostError::Transport(text.to_owned()));
    let mut tokens = line.split_whitespace();
    if tokens.next() != Some(expected_state) {
        return fail("the co-op transport module returned an unexpected readiness state");
    }
    let mut fields: HashMap<&str, &str> = HashMap::new();
    for token in tokens {
        let pair = token.split_once('=').filter(|(k, v)| !k.is_empty() && !v.is_empty());
        let Some((key, value)) = pair else {
            return fail("the co-op transport module returned a malformed local route");
        };
        if fields.insert(key, value).is_some() {
            return fail("the co-op transport module returned a malformed local route");
        }
    }
    if fields.get("localHost") != Some(&"127.0.0.1") || fields.get("transport") != Some(&"ice") {
        return fail("the co-op transport module did not return a verified loopback route");
    }
    let policy = if force_relay { "RELAY_ONLY" } else { "AUTOMATIC" };
    let relay_route_ok = !(force_relay && expected_state == "ROUTE_READY") || fields.get("route") == Some(&"relay");
    if fields.get("routePolicy") != Some(&policy) || !relay_route_ok {
        return fail("the co-op transport module returned a route outside the requested transport policy");
    }
    match fields.get("localPort").and_then(|p| p.parse::<u16>().ok()) {
        Some(port) if (port != 0) == local_client => Ok(port),
        _ => fail("the co-op transport module returned an invalid local route"),
    }
}

pub fn parse_peer_event(line: &str) -> Option<PeerEvent> {
    let mut tokens = line.split_whitespace();
    let kind = tokens.next()?;
    let mut fields: HashMap<&str, &str> = HashMap::new();
    for (key, value) in tokens.filter_map(|t| t.split_once('=')) {
        fields.insert(key, value);
    }
    let account = (*fields.get("account")?).to_owned();
    let field = |key: &str| fields.get(key).map(|v| (*v).to_owned());
    Some(match kind {
        "PEER_CONNECTED" => PeerEvent::Connected {
            account,
            route: field("route"),
        },
        "PEER_ADMITTED" => PeerEvent::Admitted {
            account,
            route: field("route"),
        },
        "PEER_LOST" => PeerEvent::Lost {
            account,
            reason: field("reason"),
        },
        "PEER_REMOVED" => PeerEvent::Removed {
            account,
            reason: field("reason"),
            admitted: fields.get("admitted") == Some(&"1"),
        },
        "PEER_ADD_REJECTED" => PeerEvent::AddRejected {
            account,
            reason: field("reason"),
        },
        _ => return None,
    })
}

pub fn is_local_admission(line: &str) -> bool {
    let mut tokens = line.split_whitespace();
    tokens.next() == Some("ADMISSION_ACCEPTED") && {
        let rest: Vec<&str> = tokens.collect();
        rest.contains(&"role=host") && rest.contains(&"source=local")
    }
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
    } else if let Some(event) = parse_peer_event(line) {
        apply_peer_event(live, &event, now_iso);
    }
}

/// The peers worth linking, in order, and how many entries were refused.
pub fn wanted_peers(entries: &[PeerListEntry], host_account_id: Option<&str>) -> (Vec<WantedPeer>, usize) {
    let mut wanted: Vec<WantedPeer> = Vec::new();
    let mut taken: HashSet<&str> = HashSet::new();
    let mut refused = 0;
    for entry in entries.iter().take(MAX_HUB_PEERS) {
        let peer_id = entry
            .peer_id
            .parse::<u64>()
            .ok()
            .filter(|_| safe_peer_id(&entry.peer_id));
        let fine = safe_account(&entry.account_id)
            && safe_key(&entry.transport_key)
            && host_account_id != Some(entry.account_id.as_str());
        let (Some(peer_id), true) = (peer_id, fine) else {
            refused += 1;
            continue;
        };
        if !taken.insert(entry.account_id.as_str()) {
            continue;
        }
        wanted.push(WantedPeer {
            account_id: entry.account_id.clone(),
            peer_id,
            transport_key: entry.transport_key.clone(),
        });
    }
    (wanted, refused)
}

pub fn peer_diff(linked: &HashSet<String>, wanted: &[WantedPeer]) -> PeerDiff {
    let add = wanted
        .iter()
        .filter(|w| !linked.contains(&w.account_id))
        .cloned()
        .collect();
    let mut remove: Vec<String> = linked
        .iter()
        .filter(|a| !wanted.iter().any(|w| &w.account_id == *a))
        .cloned()
        .collect();
    remove.sort();
    PeerDiff { add, remove }
}

/// The week of a Trials matchmaker hunt id, `Arena_MatchmakerHunt_<Easy|Hard|Elite>_<three digits>`.
pub fn trials_week(matchmaker_hunt_id: &str) -> Option<&str> {
    let rest = matchmaker_hunt_id.strip_prefix("Arena_MatchmakerHunt_")?;
    let (tier, week) = rest.split_once('_')?;
    let tier_ok = matches!(tier, "Easy" | "Hard" | "Elite");
    (tier_ok && week.len() == 3 && week.bytes().all(|b| b.is_ascii_digit())).then_some(week)
}

/// The hosted server's command line; the runtime checks the leading `-` and the loopback multihome.
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

fn push_backslashes(out: &mut String, count: usize) {
    out.extend(std::iter::repeat_n('\\', count));
}

/// Quotes as the Microsoft C runtime splits (`CommandLineToArgvW`'s inverse).
pub fn windows_command_line(program: &Path, arguments: &[String]) -> String {
    let mut out = format!("\"{}\"", program.to_string_lossy());
    for argument in arguments {
        out.push(' ');
        let quoted = argument.is_empty() || argument.contains([' ', '\t']);
        if quoted {
            out.push('"');
        }
        let mut backslashes = 0;
        for c in argument.chars() {
            match c {
                '\\' => backslashes += 1,
                '"' => {
                    push_backslashes(&mut out, backslashes * 2 + 1);
                    out.push('"');
                    backslashes = 0;
                }
                other => {
                    push_backslashes(&mut out, backslashes);
                    out.push(other);
                    backslashes = 0;
                }
            }
        }
        if quoted {
            push_backslashes(&mut out, backslashes * 2);
            out.push('"');
        } else {
            push_backslashes(&mut out, backslashes);
        }
    }
    out
}

pub fn build_launch_environment(
    spec: &LaunchEnvironmentSpec,
    inherited: impl IntoIterator<Item = (String, String)>,
) -> BTreeMap<String, String> {
    let ours = |key: &str| {
        CLEARED_VARIABLES
            .iter()
            .chain(LAUNCH_VARIABLES.iter())
            .any(|name| name.eq_ignore_ascii_case(key))
    };
    let mut environment: BTreeMap<String, String> = inherited.into_iter().filter(|(k, _)| !ours(k)).collect();
    let mut set = |key: &str, value: String| {
        environment.insert(key.to_owned(), value);
    };
    set("RUSTIC_RIFTS_AUTH_MODE", "HUNT_CAPABILITY".to_owned());
    set("RUSTIC_RIFTS_BIND_ADDRESS", "127.0.0.1".to_owned());
    set(
        "RUSTIC_RIFTS_HOST_RUNTIME_CAPABILITY",
        spec.capability.expose().to_owned(),
    );
    set("RUSTIC_RIFTS_HOSTED_ROLE", spec.role.wire_role().to_owned());
    set("RUSTIC_RIFTS_SESSION_ID", wire_session_id(&spec.session_id));
    set("RUSTIC_RIFTS_SESSION_GENERATION", spec.generation.to_string());
    set("RUSTIC_RIFTS_GAMESERVER_LAUNCH_ID", spec.launch_id.clone());
    set("RUSTIC_RIFTS_READY_PIPE", spec.ready_pipe.clone());
    set("RUSTIC_RIFTS_CONTROL_PIPE", spec.control_pipe.clone());
    set("RUSTIC_RIFTS_GUARD_PIPE", spec.guard_pipe.clone());
    set(
        "RUSTIC_RIFTS_HUNT_GUARD_EVENTS",
        if spec.hunt_evidence { "1" } else { "0" }.to_owned(),
    );
    set(
        "RUSTIC_RIFTS_HUNT_GUARD_COMPLETION_MODE",
        spec.guard_completion_mode.clone(),
    );
    set("RUSTIC_RIFTS_RUNTIME_PROFILE", spec.runtime_profile.clone());
    set("RUSTIC_RIFTS_LAUNCHER_PID", spec.launcher_pid.to_string());
    if matches!(spec.role, HostedRole::Hub(_)) {
        if let Some(players) = spec.max_players {
            set("RUSTIC_RIFTS_MAX_PLAYERS", players.to_string());
        }
        if let Some(afk) = spec.hub_afk_seconds {
            set("RUSTIC_RIFTS_HUB_AFK_SECONDS", afk.to_string());
        }
    }
    if let Some(week) = &spec.trials_week {
        set("RUSTIC_RIFTS_TRIALS_WEEK", week.clone());
    }
    environment
}

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

/// A transport line made safe for the central log: any token naming a secret loses its value.
pub fn sanitize_central_log_line(line: &str) -> String {
    let mut out = String::new();
    for token in line.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        let lower = token.to_ascii_lowercase();
        if SECRET_WORDS.iter().any(|w| lower.contains(w)) {
            match token.split_once('=') {
                Some((key, _)) => {
                    out.push_str(key);
                    out.push_str("=[redacted]");
                }
                None => out.push_str("[redacted]"),
            }
        } else {
            out.push_str(token);
        }
        if out.len() >= LOG_LINE_MAX {
            out = out.chars().take(LOG_LINE_MAX).collect();
            break;
        }
    }
    out
}

pub fn udp_stats_local_rx(line: &str) -> Option<u64> {
    if !line.contains("UDP_STATS") {
        return None;
    }
    line.split_whitespace()
        .find_map(|t| t.strip_prefix("localRx="))?
        .parse()
        .ok()
}

impl GameTrafficWatch {
    pub fn new(now: Instant) -> Self {
        Self {
            reading: None,
            changed: now,
        }
    }

    pub fn observe(&mut self, line: &str, now: Instant) {
        let Some(reading) = udp_stats_local_rx(line) else {
            return;
        };
        if self.reading.is_none_or(|previous| reading > previous) {
            self.changed = now;
        }
        self.reading = Some(reading);
    }

    pub fn restart(&mut self, now: Instant) {
        self.changed = now;
    }

    pub fn idle(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.changed)
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

pub fn relay_refresh_due(seconds_left: Option<i64>, since_last: Option<Duration>) -> bool {
    let Some(left) = seconds_left else {
        return false;
    };
    left <= RELAY_REFRESH_MARGIN_SECONDS
        || (left <= RELAY_EARLY_REFRESH_SECONDS && since_last.is_none_or(|s| s >= RELAY_EARLY_REFRESH_SPACING))
}

pub fn relay_restart_due(seconds_left: Option<i64>) -> bool {
    seconds_left.is_some_and(|left| left <= 300)
}

pub fn default_max_servers(physical_gb: u64) -> u8 {
    if physical_gb < 4 {
        return 1;
    }
    let servers = (2 * (physical_gb - 4) / 3).clamp(1, 6);
    u8::try_from(servers).unwrap_or(6)
}

pub fn capacity_decision(commit_mib: u64, active_servers: u8, max_servers: u8) -> CapacityDecision {
    if commit_mib < HOST_COMMIT_FLOOR_MIB || active_servers >= max_servers {
        CapacityDecision::Refuse { reason: "HOST_BUSY" }
    } else {
        CapacityDecision::Accept
    }
}

pub fn restart_backoff(attempts_in_window: usize) -> Duration {
    Duration::from_secs(match attempts_in_window {
        0 => 5,
        1 => 15,
        _ => 60,
    })
}

const RESTART_WINDOW: Duration = Duration::from_secs(600);

pub fn attempts_in_window(attempts: &[Instant], now: Instant) -> usize {
    attempts
        .iter()
        .filter(|a| now.saturating_duration_since(**a) < RESTART_WINDOW)
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
    idle_since.is_some_and(|since| now.saturating_duration_since(since) >= Duration::from_secs(300))
}

pub fn scheduled_restart_due(uptime: Duration, present: bool) -> bool {
    !present && uptime > Duration::from_secs(72_000)
}
