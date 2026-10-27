//! Who may host (`p2p/config/p2pHostPolicy.ts`): the artifact hashes a host must match, the eligibility rules and
//! the score host selection ranks by (lower is better).

use rifts_seam::{Settings, UnixMillis};
use serde_json::Value;

use crate::hunt::{ArtifactHashes, HostPresenceRecord, HostReliabilityRecord, ScoreBreakdown};
use crate::tiers::js_number;

/// Dauntless 1.14.7, CL 647472: `Dauntless-Win64-Shipping.exe` (121,407,264 bytes), checked 11 Oct 2026.
pub const KNOWN_GAME_EXE_SHA256: &[(i64, &str)] =
    &[(647_472, "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b")];

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// What the update feed serves for `P2P_RUNTIME_CHANNEL` (isles reads it; RU4's seam).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArtifactSources {
    /// The client runtime feed's `latest.json`.
    pub runtime_manifest: Option<Value>,
    /// Role and SHA-256 of each artifact of the signed Guard manifest.
    pub guard_artifacts: Option<Vec<(String, String)>>,
}

/// `Number(TARGET_CHANGELIST)` when it is a safe integer.
fn target_changelist(settings: &dyn Settings) -> Option<i64> {
    let n = js_number(settings.get("TARGET_CHANGELIST")?)?;
    #[allow(clippy::cast_possible_truncation)]
    (n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0).then_some(n as i64)
}

fn feed_changelist_matches(manifest: &Value, settings: &dyn Settings) -> bool {
    match (target_changelist(settings), manifest.get("targetChangelist")) {
        (None, _) | (_, None) => true,
        (Some(expected), Some(actual)) => actual.as_i64() == Some(expected),
    }
}

fn feed_runtime_hash(sources: &ArtifactSources, settings: &dyn Settings) -> Option<String> {
    let manifest = sources.runtime_manifest.as_ref()?;
    if !feed_changelist_matches(manifest, settings) {
        return None;
    }
    manifest
        .get("sha256")
        .and_then(Value::as_str)
        .filter(|h| is_sha256(h))
        .map(str::to_lowercase)
}

fn feed_extra_file_hash(sources: &ArtifactSources, settings: &dyn Settings, file: &str) -> Option<String> {
    let manifest = sources.runtime_manifest.as_ref()?;
    if !feed_changelist_matches(manifest, settings) {
        return None;
    }
    manifest
        .get("extraFiles")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("name").and_then(Value::as_str).is_some_and(|n| n.eq_ignore_ascii_case(file)))?
        .get("sha256")
        .and_then(Value::as_str)
        .filter(|h| is_sha256(h))
        .map(str::to_lowercase)
}

fn guard_role_hash(sources: &ArtifactSources, role: &str) -> Option<String> {
    sources
        .guard_artifacts
        .as_ref()?
        .iter()
        .find(|(r, _)| r == role)
        .map(|(_, hash)| hash.as_str())
        .filter(|h| is_sha256(h))
        .map(str::to_lowercase)
}

/// The artifact hashes and the pins that differ from the feed (Node logs each once: the pin wins).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactPolicy {
    pub hashes: ArtifactHashes,
    pub stale_pins: Vec<String>,
}

/// The first set name wins (later ones are older spellings); a blank value is unset; then the feed's value.
fn required_hash(
    settings: &dyn Settings,
    derived: Option<String>,
    stale: &mut Vec<String>,
    names: &[&str],
) -> Result<String, String> {
    let missing = || format!("Missing or invalid {}.", names[0]);
    let Some((set, value)) = names
        .iter()
        .find_map(|name| settings.get(name).map(str::trim).filter(|v| !v.is_empty()).map(|v| (*name, v)))
    else {
        return derived.ok_or_else(missing);
    };
    if !is_sha256(value) {
        return Err(missing());
    }
    let pinned = value.to_lowercase();
    if let Some(derived) = derived.filter(|d| *d != pinned) {
        stale.push(format!(
            "[P2PArtifacts] {set} pins {}... but the feed serves {}...; the pin wins, so hosts on the feed's file are \
             refused (HOST_RUNTIME_UNVERIFIED). Remove the pin to follow the feed.",
            &pinned[..12],
            &derived[..12]
        ));
    }
    Ok(pinned)
}

/// `ReadP2PArtifactPolicy`: per artifact, the setting, else the feed, else (the exe) the changelist's known hash.
pub fn read_artifact_policy(settings: &dyn Settings, sources: &ArtifactSources) -> Result<ArtifactPolicy, String> {
    let mut stale = Vec::new();
    let known_exe = target_changelist(settings).and_then(|cl| {
        KNOWN_GAME_EXE_SHA256
            .iter()
            .find(|(known, _)| *known == cl)
            .map(|(_, hash)| (*hash).to_owned())
    });
    let hashes = ArtifactHashes {
        game_exe: required_hash(settings, known_exe, &mut stale, &["P2P_GAME_EXE_SHA256"])?,
        internal_server: required_hash(
            settings,
            feed_runtime_hash(sources, settings),
            &mut stale,
            &["P2P_INTERNAL_SERVER_SHA256"],
        )?,
        winmm_proxy: required_hash(
            settings,
            feed_extra_file_hash(sources, settings, "winmm.dll"),
            &mut stale,
            &["P2P_WINMM_SHA256"],
        )?,
        steam_bridge: required_hash(
            settings,
            guard_role_hash(sources, "TRANSPORT_MODULE"),
            &mut stale,
            &["P2P_TRANSPORT_MODULE_SHA256", "P2P_STEAM_BRIDGE_SHA256"],
        )?,
        // The ICE transport has no separate runtime: launchers report the module for both
        steam_runtime: required_hash(
            settings,
            guard_role_hash(sources, "TRANSPORT_RUNTIME").or_else(|| guard_role_hash(sources, "TRANSPORT_MODULE")),
            &mut stale,
            &["P2P_TRANSPORT_RUNTIME_SHA256", "P2P_STEAM_RUNTIME_SHA256"],
        )?,
        admission_public_key: required_hash(
            settings,
            guard_role_hash(sources, "ADMISSION_KEY"),
            &mut stale,
            &["P2P_ADMISSION_PUBLIC_XY_SHA256"],
        )?,
    };
    Ok(ArtifactPolicy {
        hashes,
        stale_pins: stale,
    })
}

pub fn artifact_hashes_match(actual: &ArtifactHashes, expected: &ArtifactHashes) -> bool {
    [
        (&actual.game_exe, &expected.game_exe),
        (&actual.internal_server, &expected.internal_server),
        (&actual.winmm_proxy, &expected.winmm_proxy),
        (&actual.steam_bridge, &expected.steam_bridge),
        (&actual.steam_runtime, &expected.steam_runtime),
        (&actual.admission_public_key, &expected.admission_public_key),
    ]
    .iter()
    .all(|(a, e)| is_sha256(a) && a.eq_ignore_ascii_case(e))
}

/// `VersionAtLeast` (`security/launcherVersion.ts`): semver, prerelease below its release.
pub fn version_at_least(actual: &str, minimum: &str) -> bool {
    fn parse(text: &str) -> Option<([u128; 3], Option<Vec<String>>)> {
        let text = text.trim();
        let core_end = text.find(['-', '+']).unwrap_or(text.len());
        let (core, rest) = text.split_at(core_end);
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
            return None;
        }
        let numbers = [parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?];
        let ident_ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
        let (pre, build) = match rest.strip_prefix('-') {
            Some(after) => match after.split_once('+') {
                Some((pre, build)) => (Some(pre), Some(build)),
                None => (Some(after), None),
            },
            None => (None, rest.strip_prefix('+')),
        };
        if !rest.is_empty() && !rest.starts_with('-') && !rest.starts_with('+') {
            return None;
        }
        if pre.is_some_and(|p| !ident_ok(p)) || build.is_some_and(|b| !ident_ok(b)) {
            return None;
        }
        Some((numbers, pre.map(|p| p.split('.').map(str::to_owned).collect())))
    }
    let (Some(left), Some(right)) = (parse(actual), parse(minimum)) else {
        return false;
    };
    for index in 0..3 {
        if left.0[index] != right.0[index] {
            return left.0[index] > right.0[index];
        }
    }
    let (left, right) = match (left.1, right.1) {
        (None, _) => return true,
        (Some(_), None) => return false,
        (Some(l), Some(r)) => (l, r),
    };
    for index in 0..left.len().max(right.len()) {
        let (Some(a), Some(b)) = (left.get(index), right.get(index)) else {
            return left.get(index).is_none();
        };
        if a == b {
            continue;
        }
        let numeric = |s: &str| s.bytes().all(|c| c.is_ascii_digit());
        return match (numeric(a), numeric(b)) {
            (true, true) => a.parse::<u128>().unwrap_or(0) > b.parse::<u128>().unwrap_or(0),
            (true, false) => false,
            (false, true) => true,
            (false, false) => a > b,
        };
    }
    true
}

/// `P2PHostEligibilityPolicy`.
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

/// `BoundedNumber`: `Number(value)` within the range, else the fallback (an unset value is NaN).
fn bounded(settings: &dyn Settings, name: &str, fallback: f64, min: f64, max: f64) -> f64 {
    match settings.get(name).and_then(js_number) {
        Some(n) if n.is_finite() && n >= min && n <= max => n,
        _ => fallback,
    }
}

#[allow(clippy::cast_possible_truncation)]
fn bounded_ms(settings: &dyn Settings, name: &str, fallback: f64, min: f64, max: f64) -> i64 {
    bounded(settings, name, fallback, min, max) as i64
}

impl EligibilityPolicy {
    /// `ReadP2PHostEligibilityPolicy`.
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let version = settings.get("P2P_HOST_MIN_LAUNCHER_VERSION").map(str::trim).unwrap_or_default();
        let version_ok = {
            let parts: Vec<&str> = version.split('.').collect();
            parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        };
        Self {
            minimum_launcher_version: if version_ok { version.to_owned() } else { "0.1.35".to_owned() },
            minimum_free_memory_mib: bounded(settings, "P2P_HOST_MIN_FREE_MEMORY_MIB", 4096.0, 1024.0, 262_144.0),
            minimum_commit_headroom_mib: bounded(settings, "P2P_HOST_MIN_COMMIT_HEADROOM_MIB", 6144.0, 1024.0, 262_144.0),
            minimum_logical_cpu_count: bounded(settings, "P2P_HOST_MIN_LOGICAL_CPUS", 4.0, 2.0, 256.0),
            maximum_cpu_load_percent: bounded(settings, "P2P_HOST_MAX_CPU_PERCENT", 85.0, 10.0, 100.0),
            maximum_worst_peer_rtt_ms: bounded(settings, "P2P_HOST_MAX_WORST_RTT_MS", 250.0, 20.0, 2000.0),
            maximum_jitter_ms: bounded(settings, "P2P_HOST_MAX_JITTER_MS", 80.0, 1.0, 1000.0),
            maximum_packet_loss_percent: bounded(settings, "P2P_HOST_MAX_PACKET_LOSS_PERCENT", 10.0, 0.0, 100.0),
            presence_maximum_age_ms: bounded_ms(settings, "P2P_HOST_PRESENCE_MAX_AGE_MS", 30_000.0, 5_000.0, 300_000.0),
            offer_lifetime_ms: bounded_ms(settings, "P2P_HOST_OFFER_LIFETIME_MS", 15_000.0, 5_000.0, 120_000.0),
            startup_lifetime_ms: bounded_ms(settings, "P2P_HOST_STARTUP_LIFETIME_MS", 120_000.0, 30_000.0, 600_000.0),
            lease_lifetime_ms: bounded_ms(settings, "P2P_HOST_LEASE_LIFETIME_MS", 10_000.0, 4_000.0, 60_000.0),
            suspect_lifetime_ms: bounded_ms(settings, "P2P_HOST_SUSPECT_LIFETIME_MS", 15_000.0, 5_000.0, 120_000.0),
            rejoin_lifetime_ms: bounded_ms(settings, "P2P_REJOIN_LIFETIME_MS", 60_000.0, 10_000.0, 300_000.0),
            maximum_reconnects: bounded(settings, "P2P_MAX_RECONNECTS", 3.0, 1.0, 10.0).floor() as i64,
        }
    }
}

/// `P2P_HUNT_DRAIN_GRACE_MS`: no cap unless 30 s to 10 min.
pub fn hunt_drain_cap_ms(settings: &dyn Settings) -> Option<i64> {
    let raw = settings.get("P2P_HUNT_DRAIN_GRACE_MS").map(str::trim).unwrap_or_default();
    if raw.is_empty() {
        return None;
    }
    let value = js_number(raw)?;
    #[allow(clippy::cast_possible_truncation)]
    (value.is_finite() && (30_000.0..=600_000.0).contains(&value)).then(|| value.floor() as i64)
}

/// `HostIneligibilityReason`: the first rule a host fails, or `None`.
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
    if reliability.and_then(|r| r.cooldown_until).is_some_and(|until| until > now) {
        return Some("HOST_COOLDOWN");
    }
    None
}

fn clamp_risk(value: f64) -> f64 {
    value.clamp(0.0, 100.0)
}

/// `ScoreHost`: seven risks of 0 to 100, weighted; rounded to three places.
pub fn score_host(
    presence: &HostPresenceRecord,
    reliability: Option<&HostReliabilityRecord>,
    policy: &EligibilityPolicy,
    active_host_sessions: i64,
) -> (f64, ScoreBreakdown) {
    let (succeeded, failed, unexpected) = reliability.map_or((0, 0, 0), |r| {
        (r.starts_succeeded, r.starts_failed, r.unexpected_failures)
    });
    let attempts = succeeded + failed + unexpected;
    #[allow(clippy::cast_precision_loss)]
    let failure_rate = if attempts == 0 {
        25.0
    } else {
        100.0 * (failed + unexpected) as f64 / attempts as f64
    };
    let integrity = if !presence.artifacts_verified {
        100.0
    } else if presence.guard_verified == Some(true) {
        presence.guard_risk_score.unwrap_or(0.0)
    } else {
        25.0
    };
    let breakdown = ScoreBreakdown {
        latency_risk: clamp_risk(100.0 * presence.worst_peer_rtt_ms / policy.maximum_worst_peer_rtt_ms),
        network_risk: clamp_risk(
            50.0 * presence.packet_loss_percent / policy.maximum_packet_loss_percent.max(1.0)
                + 50.0 * presence.jitter_ms / policy.maximum_jitter_ms,
        ),
        memory_risk: clamp_risk(100.0 * policy.minimum_commit_headroom_mib / presence.commit_headroom_mib.max(1.0)),
        cpu_risk: clamp_risk(presence.cpu_load_percent),
        stability_risk: clamp_risk(failure_rate),
        host_load_risk: if active_host_sessions > 0 { 100.0 } else { 0.0 },
        integrity_risk: integrity,
    };
    let score = 0.35 * breakdown.latency_risk
        + 0.20 * breakdown.network_risk
        + 0.15 * breakdown.memory_risk
        + 0.10 * breakdown.cpu_risk
        + 0.10 * breakdown.stability_risk
        + 0.05 * breakdown.host_load_risk
        + 0.05 * breakdown.integrity_risk;
    ((score * 1000.0).round() / 1000.0, breakdown)
}
