// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Who may host: the artifact hashes a host must run, the launcher floor, the eligibility checks and the score that
//! ranks eligible hosts (lower is better).

use std::cmp::Ordering;

use rifts_seam::{Settings, UnixMillis};
use serde_json::Value;

use crate::hunt::{ArtifactHashes, HostPresenceRecord, HostReliabilityRecord, ScoreBreakdown};
use crate::tiers::js_number;

/// Dauntless 1.14.7's `Dauntless-Win64-Shipping.exe` (121,407,264 bytes).
pub const KNOWN_GAME_EXE_SHA256: &[(i64, &str)] = &[(
    647_472,
    "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b",
)];

/// What the server knows the hosts should run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArtifactSources {
    /// The client runtime feed's `latest.json`.
    pub runtime_manifest: Option<Value>,
    /// Guard's published (role, sha256) pairs.
    pub guard_artifacts: Option<Vec<(String, String)>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArtifactPolicy {
    pub hashes: ArtifactHashes,
    /// One operator warning per pin that disagrees with the feed.
    pub stale_pins: Vec<String>,
}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// A feed's hash, lower case, or nothing when it is not a SHA-256.
fn normalized(hash: &str) -> Option<String> {
    is_sha256(hash).then(|| hash.to_ascii_lowercase())
}

/// A JavaScript safe integer, or nothing.
fn safe_integer(number: f64) -> Option<i64> {
    const LIMIT: f64 = 9_007_199_254_740_991.0;
    if number.is_finite() && number.trunc() == number && number.abs() <= LIMIT {
        Some(number as i64)
    } else {
        None
    }
}

fn setting_text<'a>(settings: &'a dyn Settings, name: &str) -> Option<&'a str> {
    settings.get(name).filter(|text| !text.trim().is_empty())
}

/// The runtime manifest counts unless both it and the server name a changelist and they differ.
fn manifest_applies(manifest: &Value, target: Option<i64>) -> bool {
    let (Some(target), Some(declared)) = (target, manifest.get("targetChangelist")) else {
        return true;
    };
    let number = if let Some(text) = declared.as_str() {
        js_number(text)
    } else {
        declared.as_f64()
    };
    number.and_then(safe_integer) == Some(target)
}

fn manifest_winmm(manifest: &Value) -> Option<String> {
    let is_winmm = |file: &&Value| {
        file["name"]
            .as_str()
            .is_some_and(|n| n.eq_ignore_ascii_case("winmm.dll"))
    };
    let entry = manifest.get("extraFiles")?.as_array()?.iter().find(is_winmm)?;
    normalized(entry.get("sha256")?.as_str()?)
}

/// The six hashes as the feeds give them, before any pin, in `ArtifactHashes` field order.
fn feed_hashes(settings: &dyn Settings, sources: &ArtifactSources) -> [Option<String>; 6] {
    let target = setting_text(settings, "TARGET_CHANGELIST")
        .and_then(js_number)
        .and_then(safe_integer);
    let manifest = sources
        .runtime_manifest
        .as_ref()
        .filter(|m| manifest_applies(m, target));
    let guard = |wanted: &str| {
        let published = sources.guard_artifacts.as_deref().unwrap_or_default();
        let (_, sha) = published.iter().find(|(role, _)| role == wanted)?;
        normalized(sha)
    };
    let game_exe = KNOWN_GAME_EXE_SHA256
        .iter()
        .find(|&&(changelist, _)| Some(changelist) == target)
        .map(|&(_, sha)| String::from(sha));
    let internal_server = manifest.and_then(|m| normalized(m.get("sha256")?.as_str()?));
    let module = guard("TRANSPORT_MODULE");
    // The ICE transport has no runtime of its own.
    let runtime = guard("TRANSPORT_RUNTIME").or(module.clone());
    [
        game_exe,
        internal_server,
        manifest.and_then(manifest_winmm),
        module,
        runtime,
        guard("ADMISSION_KEY"),
    ]
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

fn stale_pin_warning(setting: &str, pinned: &str, served: &str) -> String {
    let short = |hash: &str| hash.chars().take(12).collect::<String>();
    format!(
        "[P2PArtifacts] {setting} pins {}... but the feed serves {}...; the pin wins, so hosts on the feed's file are \
         refused (HOST_RUNTIME_UNVERIFIED). Remove the pin to follow the feed.",
        short(pinned),
        short(served),
    )
}

/// One artifact's hash: a valid pin beats the feed, an invalid pin or nothing at all is an error.
fn resolve(settings: &dyn Settings, names: &[&str], feed: Option<String>, stale: &mut Vec<String>) -> Option<String> {
    let Some((setting, raw)) = names
        .iter()
        .find_map(|&name| Some((name, setting_text(settings, name)?)))
    else {
        return feed;
    };
    let pinned = normalized(raw.trim())?;
    if let Some(served) = feed.filter(|served| *served != pinned) {
        stale.push(stale_pin_warning(setting, &pinned, &served));
    }
    Some(pinned)
}

pub fn read_artifact_policy(settings: &dyn Settings, sources: &ArtifactSources) -> Result<ArtifactPolicy, String> {
    let mut policy = ArtifactPolicy::default();
    let feeds = feed_hashes(settings, sources);
    for ((names, feed), slot) in PIN_SETTINGS.into_iter().zip(feeds).zip(slots(&mut policy.hashes)) {
        *slot = resolve(settings, names, feed, &mut policy.stale_pins)
            .ok_or_else(|| format!("Missing or invalid {}.", names[0]))?;
    }
    Ok(policy)
}

fn slots(h: &mut ArtifactHashes) -> [&mut String; 6] {
    let ArtifactHashes {
        game_exe,
        internal_server,
        winmm_proxy,
        steam_bridge,
        steam_runtime,
        admission_public_key,
    } = h;
    [
        game_exe,
        internal_server,
        winmm_proxy,
        steam_bridge,
        steam_runtime,
        admission_public_key,
    ]
}

fn values(h: &ArtifactHashes) -> [&String; 6] {
    let ArtifactHashes {
        game_exe,
        internal_server,
        winmm_proxy,
        steam_bridge,
        steam_runtime,
        admission_public_key,
    } = h;
    [
        game_exe,
        internal_server,
        winmm_proxy,
        steam_bridge,
        steam_runtime,
        admission_public_key,
    ]
}

pub fn artifact_hashes_match(actual: &ArtifactHashes, expected: &ArtifactHashes) -> bool {
    std::iter::zip(values(actual), values(expected)).all(|(got, want)| is_sha256(got) && got.eq_ignore_ascii_case(want))
}

/// `major.minor.patch[-pre][+build]`, split; build metadata only has to be well formed.
fn semver_parts(text: &str) -> Option<([u64; 3], Option<&str>)> {
    let well_formed =
        |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b));
    let (version, build) = text
        .trim()
        .split_once('+')
        .map_or((text.trim(), None), |(v, b)| (v, Some(b)));
    let (numbers, pre) = version.split_once('-').map_or((version, None), |(n, p)| (n, Some(p)));
    if !build.into_iter().chain(pre).all(well_formed) {
        return None;
    }
    let pieces: Vec<&str> = numbers.split('.').collect();
    let [major, minor, patch] = pieces.as_slice() else {
        return None;
    };
    let number = |piece: &str| {
        if all_digits(piece) {
            piece.parse::<u64>().ok()
        } else {
            None
        }
    };
    Some(([number(major)?, number(minor)?, number(patch)?], pre))
}

/// Dot-separated identifiers: numbers below words, numbers by value, words by bytes, and a longer list after
/// its prefix.
fn compare_prerelease(left: &str, right: &str) -> Ordering {
    let mut left = left.split('.');
    let mut right = right.split('.');
    loop {
        let (l, r) = match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(l), Some(r)) => (l, r),
        };
        // Numbers sort below words; overlong numbers saturate.
        let value = |id: &str| id.parse::<u128>().unwrap_or(u128::MAX);
        let order = all_digits(r).cmp(&all_digits(l)).then_with(|| {
            if all_digits(l) {
                value(l).cmp(&value(r))
            } else {
                l.as_bytes().cmp(r.as_bytes())
            }
        });
        if order.is_ne() {
            return order;
        }
    }
}

/// Semantic versions; build metadata is ignored and anything unparsable is false.
pub fn version_at_least(actual: &str, minimum: &str) -> bool {
    let (Some((have, have_pre)), Some((need, need_pre))) = (semver_parts(actual), semver_parts(minimum)) else {
        return false;
    };
    match have.cmp(&need) {
        Ordering::Equal => match (have_pre, need_pre) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(have), Some(need)) => compare_prerelease(have, need).is_ge(),
        },
        newer_or_older => newer_or_older.is_gt(),
    }
}

/// Declares the thresholds struct and its reader together: each field beside its setting, default and range.
macro_rules! eligibility_policy {
    (|$s:ident| { $( $field:ident: $ty:ty = $read:expr ),* $(,)? }) => {
        /// The host checks' thresholds (Node's `ReadP2PHostEligibilityPolicy`).
        #[derive(Clone, Debug, PartialEq)]
        pub struct EligibilityPolicy { $( pub $field: $ty, )* }

        impl EligibilityPolicy {
            pub fn from_settings($s: &dyn Settings) -> Self {
                Self { $( $field: $read, )* }
            }
        }
    };
}

eligibility_policy!(|s| {
    minimum_launcher_version: String = launcher_floor(s),
    minimum_free_memory_mib: f64 = within(s, "P2P_HOST_MIN_FREE_MEMORY_MIB", 4096.0, 1024.0..=262_144.0),
    minimum_commit_headroom_mib: f64 = within(s, "P2P_HOST_MIN_COMMIT_HEADROOM_MIB", 6144.0, 1024.0..=262_144.0),
    minimum_logical_cpu_count: f64 = within(s, "P2P_HOST_MIN_LOGICAL_CPUS", 4.0, 2.0..=256.0),
    maximum_cpu_load_percent: f64 = within(s, "P2P_HOST_MAX_CPU_PERCENT", 85.0, 10.0..=100.0),
    maximum_worst_peer_rtt_ms: f64 = within(s, "P2P_HOST_MAX_WORST_RTT_MS", 250.0, 20.0..=2000.0),
    maximum_jitter_ms: f64 = within(s, "P2P_HOST_MAX_JITTER_MS", 80.0, 1.0..=1000.0),
    maximum_packet_loss_percent: f64 = within(s, "P2P_HOST_MAX_PACKET_LOSS_PERCENT", 10.0, 0.0..=100.0),
    presence_maximum_age_ms: i64 = within(s, "P2P_HOST_PRESENCE_MAX_AGE_MS", 30e3, 5e3..=300e3).trunc() as i64,
    offer_lifetime_ms: i64 = within(s, "P2P_HOST_OFFER_LIFETIME_MS", 15e3, 5e3..=120e3).trunc() as i64,
    startup_lifetime_ms: i64 = within(s, "P2P_HOST_STARTUP_LIFETIME_MS", 120e3, 30e3..=600e3).trunc() as i64,
    lease_lifetime_ms: i64 = within(s, "P2P_HOST_LEASE_LIFETIME_MS", 10e3, 4e3..=60e3).trunc() as i64,
    suspect_lifetime_ms: i64 = within(s, "P2P_HOST_SUSPECT_LIFETIME_MS", 15e3, 5e3..=120e3).trunc() as i64,
    rejoin_lifetime_ms: i64 = within(s, "P2P_REJOIN_LIFETIME_MS", 60e3, 10e3..=300e3).trunc() as i64,
    maximum_reconnects: i64 = within(s, "P2P_MAX_RECONNECTS", 3.0, 1.0..=10.0).floor() as i64,
});

impl Default for EligibilityPolicy {
    fn default() -> Self {
        Self::from_settings(&std::collections::BTreeMap::<String, String>::new())
    }
}

/// A finite `js_number` inside `range`; anything else, unset included, is `default`.
fn within(settings: &dyn Settings, name: &str, default: f64, range: std::ops::RangeInclusive<f64>) -> f64 {
    match settings.get(name).and_then(js_number) {
        Some(n) if n.is_finite() && range.contains(&n) => n,
        _ => default,
    }
}

/// Three runs of digits, or the default floor.
fn launcher_floor(settings: &dyn Settings) -> String {
    let configured = settings.get("P2P_HOST_MIN_LAUNCHER_VERSION").map(str::trim);
    let valid = configured.filter(|v| v.split('.').count() == 3 && v.split('.').all(all_digits));
    valid.unwrap_or("0.1.35").into()
}

/// How long a drained hunt may keep serving; `None` is no cap.
pub fn hunt_drain_cap_ms(settings: &dyn Settings) -> Option<i64> {
    let grace = js_number(setting_text(settings, "P2P_HUNT_DRAIN_GRACE_MS")?)?;
    (grace.is_finite() && (30_000.0..=600_000.0).contains(&grace)).then(|| grace.floor() as i64)
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
    let p = presence;
    let age = now.0 - p.observed_at.0;
    let cooling = reliability
        .and_then(|r| r.cooldown_until)
        .is_some_and(|until| until > now);
    // Checked in this order; the first failure is the reason.
    let failures = [
        ("HOST_CONSENT_REQUIRED", !p.consent_to_host),
        (
            "HOST_LAUNCHER_OUTDATED",
            !version_at_least(&p.launcher_version, &policy.minimum_launcher_version),
        ),
        (
            "HOST_RUNTIME_UNVERIFIED",
            !(p.artifacts_verified && p.runtime_supported && p.steam_runtime_ready),
        ),
        (
            "HOST_GUARD_UNVERIFIED",
            guard_required && p.guard_verified != Some(true),
        ),
        (
            "HOST_PRESENCE_STALE",
            p.expires_at <= now || age > policy.presence_maximum_age_ms,
        ),
        (
            "HOST_MEMORY_LOW",
            p.free_physical_memory_mib < policy.minimum_free_memory_mib
                || p.commit_headroom_mib < policy.minimum_commit_headroom_mib,
        ),
        (
            "HOST_CPU_UNSUITABLE",
            p.power_saver
                || p.logical_cpu_count < policy.minimum_logical_cpu_count
                || p.cpu_load_percent > policy.maximum_cpu_load_percent,
        ),
        (
            "HOST_NETWORK_UNSUITABLE",
            [
                (p.worst_peer_rtt_ms, policy.maximum_worst_peer_rtt_ms),
                (p.jitter_ms, policy.maximum_jitter_ms),
                (p.packet_loss_percent, policy.maximum_packet_loss_percent),
            ]
            .iter()
            .any(|(measured, ceiling)| measured > ceiling),
        ),
        ("HOST_ALREADY_ACTIVE", active_host_sessions > 0),
        ("HOST_COOLDOWN", cooling),
    ];
    failures.into_iter().find_map(|(code, failed)| failed.then_some(code))
}

/// Each risk's share of the score, in `ScoreBreakdown` field order.
const WEIGHTS: [f64; 7] = [0.35, 0.20, 0.15, 0.10, 0.10, 0.05, 0.05];

/// Lower is better; rounded to three decimals.
pub fn score_host(
    presence: &HostPresenceRecord,
    reliability: Option<&HostReliabilityRecord>,
    policy: &EligibilityPolicy,
    active_host_sessions: i64,
) -> (f64, ScoreBreakdown) {
    let risk = |value: f64| value.clamp(0.0, 100.0);
    let p = presence;
    let outcomes = reliability.map(|r| [r.starts_succeeded, r.starts_failed, r.unexpected_failures]);
    let [ok, failed, unexpected] = outcomes.unwrap_or_default();
    let tries = ok + failed + unexpected;
    let risks = [
        risk(100.0 * p.worst_peer_rtt_ms / policy.maximum_worst_peer_rtt_ms),
        risk(
            50.0 * p.packet_loss_percent / policy.maximum_packet_loss_percent.max(1.0)
                + 50.0 * p.jitter_ms / policy.maximum_jitter_ms,
        ),
        risk(100.0 * policy.minimum_commit_headroom_mib / p.commit_headroom_mib.max(1.0)),
        risk(p.cpu_load_percent),
        // A host never tried is a mild unknown.
        if tries == 0 {
            25.0
        } else {
            risk(100.0 * (failed + unexpected) as f64 / tries as f64)
        },
        if active_host_sessions > 0 { 100.0 } else { 0.0 },
        match (p.artifacts_verified, p.guard_verified) {
            (false, _) => 100.0,
            (true, Some(true)) => p.guard_risk_score.unwrap_or(0.0),
            (true, _) => 25.0,
        },
    ];
    let total: f64 = WEIGHTS.iter().zip(risks).map(|(weight, value)| weight * value).sum();
    let [
        latency_risk,
        network_risk,
        memory_risk,
        cpu_risk,
        stability_risk,
        host_load_risk,
        integrity_risk,
    ] = risks;
    let breakdown = ScoreBreakdown {
        latency_risk,
        network_risk,
        memory_risk,
        cpu_risk,
        stability_risk,
        host_load_risk,
        integrity_risk,
    };
    ((total * 1000.0).round() / 1000.0, breakdown)
}
