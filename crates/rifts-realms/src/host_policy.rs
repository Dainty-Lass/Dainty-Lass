// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Who may host: the artifact hashes a host must run, the launcher floor, the eligibility checks and the score that
//! ranks eligible hosts (lower is better).

use rifts_seam::{Settings, UnixMillis};
use serde_json::Value;

use crate::hunt::{ArtifactHashes, HostPresenceRecord, HostReliabilityRecord, ScoreBreakdown};
use crate::tiers::js_number;

/// Dauntless 1.14.7's `Dauntless-Win64-Shipping.exe` (121,407,264 bytes).
pub const KNOWN_GAME_EXE_SHA256: &[(i64, &str)] = &[(
    647_472,
    "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b",
)];

const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// What the server knows the hosts should run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArtifactSources {
    /// The client runtime feed's `latest.json`.
    pub runtime_manifest: Option<Value>,
    /// Guard's published (role, sha256) pairs.
    pub guard_artifacts: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArtifactPolicy {
    pub hashes: ArtifactHashes,
    /// One operator warning per pin that disagrees with the feed.
    pub stale_pins: Vec<String>,
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn feed_hash(value: Option<&str>) -> Option<String> {
    value.filter(|v| is_sha256(v)).map(str::to_ascii_lowercase)
}

fn safe_integer(number: f64) -> Option<i64> {
    (number.is_finite() && number.fract() == 0.0 && number.abs() <= MAX_SAFE_INTEGER).then_some(number as i64)
}

fn target_changelist(settings: &dyn Settings) -> Option<i64> {
    let text = settings.get("TARGET_CHANGELIST")?;
    if text.trim().is_empty() {
        return None;
    }
    js_number(text).and_then(safe_integer)
}

fn manifest_changelist(manifest: &Value) -> Option<Option<i64>> {
    let value = manifest.get("targetChangelist")?;
    let number = match value {
        Value::Number(n) => n.as_f64(),
        Value::String(text) => js_number(text),
        _ => None,
    };
    Some(number.and_then(safe_integer))
}

/// The six hashes as the feeds give them, before any pin.
fn feed_hashes(settings: &dyn Settings, sources: &ArtifactSources) -> [Option<String>; 6] {
    let target = target_changelist(settings);
    let manifest = sources.runtime_manifest.as_ref().filter(|manifest| match target {
        None => true,
        Some(target) => match manifest_changelist(manifest) {
            None => true,
            Some(changelist) => changelist == Some(target),
        },
    });
    let internal_server = manifest.and_then(|m| feed_hash(m.get("sha256").and_then(Value::as_str)));
    let winmm = manifest.and_then(|m| {
        let files = m.get("extraFiles")?.as_array()?;
        let entry = files.iter().find(|f| {
            f.get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case("winmm.dll"))
        })?;
        feed_hash(entry.get("sha256").and_then(Value::as_str))
    });
    let role = |name: &str| {
        sources
            .guard_artifacts
            .as_ref()
            .and_then(|roles| roles.iter().find(|(role, _)| role == name))
            .and_then(|(_, sha)| feed_hash(Some(sha)))
    };
    let module = role("TRANSPORT_MODULE");
    // The ICE transport has no runtime of its own.
    let runtime = role("TRANSPORT_RUNTIME").or_else(|| module.clone());
    let game_exe = target.and_then(|target| {
        KNOWN_GAME_EXE_SHA256
            .iter()
            .find(|(changelist, _)| *changelist == target)
            .map(|(_, sha)| (*sha).to_owned())
    });
    [game_exe, internal_server, winmm, module, runtime, role("ADMISSION_KEY")]
}

/// Per artifact, its settings in order: the first one set wins; later names are older spellings.
const PIN_SETTINGS: [&[&str]; 6] = [
    &["P2P_GAME_EXE_SHA256"],
    &["P2P_INTERNAL_SERVER_SHA256"],
    &["P2P_WINMM_SHA256"],
    &["P2P_TRANSPORT_MODULE_SHA256", "P2P_STEAM_BRIDGE_SHA256"],
    &["P2P_TRANSPORT_RUNTIME_SHA256", "P2P_STEAM_RUNTIME_SHA256"],
    &["P2P_ADMISSION_PUBLIC_XY_SHA256"],
];

fn twelve(text: &str) -> &str {
    text.get(..12).unwrap_or(text)
}

pub fn read_artifact_policy(settings: &dyn Settings, sources: &ArtifactSources) -> Result<ArtifactPolicy, String> {
    let feeds = feed_hashes(settings, sources);
    let mut values: Vec<String> = Vec::with_capacity(6);
    let mut stale_pins = Vec::new();
    for (names, feed) in PIN_SETTINGS.iter().zip(feeds) {
        let missing = || format!("Missing or invalid {}.", names[0]);
        let pin = names
            .iter()
            .find_map(|name| settings.get(name).filter(|v| !v.trim().is_empty()).map(|v| (*name, v)));
        let value = match (pin, feed) {
            (Some((name, pin)), feed) => {
                let pin = pin.trim();
                if !is_sha256(pin) {
                    return Err(missing());
                }
                let pin = pin.to_ascii_lowercase();
                if let Some(feed) = feed.filter(|feed| *feed != pin) {
                    stale_pins.push(format!(
                        "[P2PArtifacts] {name} pins {}... but the feed serves {}...; the pin wins, so hosts on the \
                         feed's file are refused (HOST_RUNTIME_UNVERIFIED). Remove the pin to follow the feed.",
                        twelve(&pin),
                        twelve(&feed)
                    ));
                }
                pin
            }
            (None, Some(feed)) => feed,
            (None, None) => return Err(missing()),
        };
        values.push(value);
    }
    let mut values = values.into_iter();
    let mut next = || values.next().unwrap_or_default();
    Ok(ArtifactPolicy {
        hashes: ArtifactHashes {
            game_exe: next(),
            internal_server: next(),
            winmm_proxy: next(),
            steam_bridge: next(),
            steam_runtime: next(),
            admission_public_key: next(),
        },
        stale_pins,
    })
}

fn fields(hashes: &ArtifactHashes) -> [&str; 6] {
    [
        &hashes.game_exe,
        &hashes.internal_server,
        &hashes.winmm_proxy,
        &hashes.steam_bridge,
        &hashes.steam_runtime,
        &hashes.admission_public_key,
    ]
}

pub fn artifact_hashes_match(actual: &ArtifactHashes, expected: &ArtifactHashes) -> bool {
    fields(actual)
        .iter()
        .zip(fields(expected))
        .all(|(a, e)| is_sha256(a) && a.eq_ignore_ascii_case(e))
}

struct Semver<'a> {
    core: [u64; 3],
    prerelease: Option<&'a str>,
}

fn identifier_ok(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

fn parse_semver(text: &str) -> Option<Semver<'_>> {
    let text = text.trim();
    let (rest, build) = match text.split_once('+') {
        Some((rest, build)) => (rest, Some(build)),
        None => (text, None),
    };
    if build.is_some_and(|b| !identifier_ok(b)) {
        return None;
    }
    let (core, prerelease) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (rest, None),
    };
    if prerelease.is_some_and(|p| !identifier_ok(p)) {
        return None;
    }
    let mut parts = core.split('.');
    let mut numbers = [0u64; 3];
    for slot in &mut numbers {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(Semver {
        core: numbers,
        prerelease,
    })
}

fn prerelease_at_least(actual: &str, minimum: &str) -> bool {
    use std::cmp::Ordering;
    let mut a = actual.split('.');
    let mut m = minimum.split('.');
    loop {
        match (a.next(), m.next()) {
            (None, None) => return true,
            (None, Some(_)) => return false,
            (Some(_), None) => return true,
            (Some(x), Some(y)) => {
                let numeric = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
                let order = match (numeric(x), numeric(y)) {
                    (true, true) => {
                        let (x, y) = (
                            x.parse::<u128>().unwrap_or(u128::MAX),
                            y.parse::<u128>().unwrap_or(u128::MAX),
                        );
                        x.cmp(&y)
                    }
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    (false, false) => x.as_bytes().cmp(y.as_bytes()),
                };
                match order {
                    Ordering::Equal => {}
                    Ordering::Greater => return true,
                    Ordering::Less => return false,
                }
            }
        }
    }
}

/// Semantic versions; build metadata is ignored and anything unparsable is false.
pub fn version_at_least(actual: &str, minimum: &str) -> bool {
    let (Some(a), Some(m)) = (parse_semver(actual), parse_semver(minimum)) else {
        return false;
    };
    if a.core != m.core {
        return a.core > m.core;
    }
    match (a.prerelease, m.prerelease) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(a), Some(m)) => prerelease_at_least(a, m),
    }
}

/// The host checks' thresholds (Node's `ReadP2PHostEligibilityPolicy`).
#[derive(Debug, Clone, PartialEq)]
pub struct EligibilityPolicy {
    pub minimum_launcher_version: String,
    pub minimum_free_memory_mib: f64,
    pub minimum_commit_headroom_mib: f64,
    pub minimum_logical_cpu_count: f64,
    pub maximum_cpu_load_percent: f64,
    pub maximum_worst_peer_rtt_ms: f64,
    pub maximum_jitter_ms: f64,
    pub maximum_packet_loss_percent: f64,
    pub presence_maximum_age_ms: i64,
    pub offer_lifetime_ms: i64,
    pub startup_lifetime_ms: i64,
    pub lease_lifetime_ms: i64,
    pub suspect_lifetime_ms: i64,
    pub rejoin_lifetime_ms: i64,
    pub maximum_reconnects: i64,
}

impl Default for EligibilityPolicy {
    fn default() -> Self {
        Self::from_settings(&std::collections::BTreeMap::new())
    }
}

/// `js_number` finite and within range; anything else, unset included, is the default.
fn bounded(settings: &dyn Settings, name: &str, default: f64, min: f64, max: f64) -> f64 {
    settings
        .get(name)
        .and_then(js_number)
        .filter(|n| n.is_finite() && (min..=max).contains(n))
        .unwrap_or(default)
}

fn three_digit_runs(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

impl EligibilityPolicy {
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let b = |name: &str, default: f64, min: f64, max: f64| bounded(settings, name, default, min, max);
        let ms = |name: &str, default: f64, min: f64, max: f64| b(name, default, min, max).trunc() as i64;
        let minimum_launcher_version = settings
            .get("P2P_HOST_MIN_LAUNCHER_VERSION")
            .map(str::trim)
            .filter(|v| three_digit_runs(v))
            .unwrap_or("0.1.35")
            .to_owned();
        Self {
            minimum_launcher_version,
            minimum_free_memory_mib: b("P2P_HOST_MIN_FREE_MEMORY_MIB", 4096.0, 1024.0, 262_144.0),
            minimum_commit_headroom_mib: b("P2P_HOST_MIN_COMMIT_HEADROOM_MIB", 6144.0, 1024.0, 262_144.0),
            minimum_logical_cpu_count: b("P2P_HOST_MIN_LOGICAL_CPUS", 4.0, 2.0, 256.0),
            maximum_cpu_load_percent: b("P2P_HOST_MAX_CPU_PERCENT", 85.0, 10.0, 100.0),
            maximum_worst_peer_rtt_ms: b("P2P_HOST_MAX_WORST_RTT_MS", 250.0, 20.0, 2000.0),
            maximum_jitter_ms: b("P2P_HOST_MAX_JITTER_MS", 80.0, 1.0, 1000.0),
            maximum_packet_loss_percent: b("P2P_HOST_MAX_PACKET_LOSS_PERCENT", 10.0, 0.0, 100.0),
            presence_maximum_age_ms: ms("P2P_HOST_PRESENCE_MAX_AGE_MS", 30_000.0, 5_000.0, 300_000.0),
            offer_lifetime_ms: ms("P2P_HOST_OFFER_LIFETIME_MS", 15_000.0, 5_000.0, 120_000.0),
            startup_lifetime_ms: ms("P2P_HOST_STARTUP_LIFETIME_MS", 120_000.0, 30_000.0, 600_000.0),
            lease_lifetime_ms: ms("P2P_HOST_LEASE_LIFETIME_MS", 10_000.0, 4_000.0, 60_000.0),
            suspect_lifetime_ms: ms("P2P_HOST_SUSPECT_LIFETIME_MS", 15_000.0, 5_000.0, 120_000.0),
            rejoin_lifetime_ms: ms("P2P_REJOIN_LIFETIME_MS", 60_000.0, 10_000.0, 300_000.0),
            maximum_reconnects: b("P2P_MAX_RECONNECTS", 3.0, 1.0, 10.0).floor() as i64,
        }
    }
}

/// How long a drained hunt may keep serving; `None` is no cap.
pub fn hunt_drain_cap_ms(settings: &dyn Settings) -> Option<i64> {
    let text = settings.get("P2P_HUNT_DRAIN_GRACE_MS")?;
    if text.trim().is_empty() {
        return None;
    }
    js_number(text)
        .filter(|n| n.is_finite() && (30_000.0..=600_000.0).contains(n))
        .map(|n| n.floor() as i64)
}

/// The first reason the host may not host, or `None`.
pub fn host_ineligibility_reason(
    presence: &HostPresenceRecord,
    reliability: Option<&HostReliabilityRecord>,
    active_host_sessions: i64,
    policy: &EligibilityPolicy,
    guard_required: bool,
    now: UnixMillis,
) -> Option<&'static str> {
    if !presence.consent_to_host {
        return Some("HOST_CONSENT_REQUIRED");
    }
    if !version_at_least(&presence.launcher_version, &policy.minimum_launcher_version) {
        return Some("HOST_LAUNCHER_OUTDATED");
    }
    if !presence.artifacts_verified || !presence.runtime_supported || !presence.steam_runtime_ready {
        return Some("HOST_RUNTIME_UNVERIFIED");
    }
    if guard_required && presence.guard_verified != Some(true) {
        return Some("HOST_GUARD_UNVERIFIED");
    }
    if presence.expires_at <= now || now.0 - presence.observed_at.0 > policy.presence_maximum_age_ms {
        return Some("HOST_PRESENCE_STALE");
    }
    if presence.free_physical_memory_mib < policy.minimum_free_memory_mib
        || presence.commit_headroom_mib < policy.minimum_commit_headroom_mib
    {
        return Some("HOST_MEMORY_LOW");
    }
    if presence.logical_cpu_count < policy.minimum_logical_cpu_count
        || presence.cpu_load_percent > policy.maximum_cpu_load_percent
        || presence.power_saver
    {
        return Some("HOST_CPU_UNSUITABLE");
    }
    if presence.worst_peer_rtt_ms > policy.maximum_worst_peer_rtt_ms
        || presence.jitter_ms > policy.maximum_jitter_ms
        || presence.packet_loss_percent > policy.maximum_packet_loss_percent
    {
        return Some("HOST_NETWORK_UNSUITABLE");
    }
    if active_host_sessions > 0 {
        return Some("HOST_ALREADY_ACTIVE");
    }
    if reliability
        .and_then(|r| r.cooldown_until)
        .is_some_and(|until| until > now)
    {
        return Some("HOST_COOLDOWN");
    }
    None
}

fn clamp_risk(value: f64) -> f64 {
    value.clamp(0.0, 100.0)
}

/// Lower is better; rounded to three decimals.
pub fn score_host(
    presence: &HostPresenceRecord,
    reliability: Option<&HostReliabilityRecord>,
    policy: &EligibilityPolicy,
    active_host_sessions: i64,
) -> (f64, ScoreBreakdown) {
    let latency = clamp_risk(100.0 * presence.worst_peer_rtt_ms / policy.maximum_worst_peer_rtt_ms);
    let network = clamp_risk(
        50.0 * presence.packet_loss_percent / policy.maximum_packet_loss_percent.max(1.0)
            + 50.0 * presence.jitter_ms / policy.maximum_jitter_ms,
    );
    let memory = clamp_risk(100.0 * policy.minimum_commit_headroom_mib / presence.commit_headroom_mib.max(1.0));
    let cpu = clamp_risk(presence.cpu_load_percent);
    let (succeeded, failed, unexpected) = reliability.map_or((0, 0, 0), |r| {
        (r.starts_succeeded, r.starts_failed, r.unexpected_failures)
    });
    let attempts = succeeded + failed + unexpected;
    let stability = if attempts == 0 {
        25.0
    } else {
        clamp_risk(100.0 * (failed + unexpected) as f64 / attempts as f64)
    };
    let host_load = if active_host_sessions > 0 { 100.0 } else { 0.0 };
    let integrity = if !presence.artifacts_verified {
        100.0
    } else if presence.guard_verified == Some(true) {
        presence.guard_risk_score.unwrap_or(0.0)
    } else {
        25.0
    };
    let total = 0.35 * latency
        + 0.20 * network
        + 0.15 * memory
        + 0.10 * cpu
        + 0.10 * stability
        + 0.05 * host_load
        + 0.05 * integrity;
    let breakdown = ScoreBreakdown {
        latency_risk: latency,
        network_risk: network,
        memory_risk: memory,
        cpu_risk: cpu,
        stability_risk: stability,
        host_load_risk: host_load,
        integrity_risk: integrity,
    };
    ((total * 1000.0).round() / 1000.0, breakdown)
}
