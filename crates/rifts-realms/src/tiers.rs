// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Hosting and trust tiers: the player-hosted policy, whether a hunt is routed to a player's PC, and the relay
//! warning. Tier 0 is no-reward transport; tier 1 decides rewards from Guard's evidence. Every gate fails closed.

use std::collections::BTreeSet;

use rifts_seam::Settings;
use sha2::{Digest, Sha256};

fn csv_set(settings: &dyn Settings, name: &str) -> BTreeSet<String> {
    settings
        .get(name)
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn dedicated_servers_configured(settings: &dyn Settings) -> bool {
    settings
        .get("DEPLOYSERVER_URL")
        .is_some_and(|url| !url.trim().is_empty())
}

impl HostingPolicy {
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let strict = |name: &str| strict_true(settings, name);
        let max_party_size = settings
            .get("P2P_MAX_PARTY_SIZE")
            .and_then(js_number)
            .filter(|n| n.fract() == 0.0 && (1.0..=4.0).contains(n))
            .map_or(4, |n| n as i64);
        let percent_rollout = settings
            .get("P2P_PERCENT_ROLLOUT")
            .and_then(js_number)
            .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
            .unwrap_or(0.0);
        Self {
            enabled: strict("P2P_HUNTS_ENABLED"),
            allowed_accounts: csv_set(settings, "P2P_ALLOWED_ACCOUNTS"),
            allow_all_accounts: strict("P2P_ALLOW_ALL_ACCOUNTS"),
            allowed_hunts: csv_set(settings, "P2P_ALLOWED_HUNTS"),
            allow_all_resolvable_hunts: strict("P2P_ALLOW_ALL_RESOLVABLE_HUNTS"),
            allow_high_risk_hunt_classes: strict("P2P_ALLOW_HIGH_RISK_HUNT_CLASSES"),
            private_parties_only: !settings
                .get("P2P_PRIVATE_PARTIES_ONLY")
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("false")),
            max_party_size,
            percent_rollout,
            force_sdr: strict("P2P_FORCE_SDR"),
            canary_enabled: strict("P2P_TIER0_CANARY_ENABLED"),
            trust_tier: u8::from(settings.get("P2P_TRUST_TIER").is_some_and(|v| v.trim() == "1")),
            tier1_rewards_enabled: strict("P2P_TIER1_REWARDS_ENABLED"),
            tier1_shadow_approved: strict("P2P_TIER1_SHADOW_APPROVED"),
            tier1_allowed_accounts: csv_set(settings, "P2P_TIER1_ALLOWED_ACCOUNTS"),
            tier1_allowed_hunts: csv_set(settings, "P2P_TIER1_ALLOWED_HUNTS"),
            dedicated_hunts_disabled: strict("P2P_DISABLE_DEDICATED_HUNTS") || !dedicated_servers_configured(settings),
            training_enabled: strict("P2P_TRAINING_ENABLED"),
        }
    }
}

pub fn is_training_hunt_id(hunt_id: &str) -> bool {
    hunt_id.contains("TrainingDojo")
}

pub fn is_p2p_training_request(game_mode: &str, hunt_id: &str, policy: &HostingPolicy) -> bool {
    policy.enabled && policy.training_enabled && game_mode == "SHARED" && is_training_hunt_id(hunt_id)
}

fn is_player_hosted_shape(game_mode: &str, hunt_id: &str, policy: &HostingPolicy) -> bool {
    if !matches!(game_mode, "ISLAND" | "SHARED") {
        return false;
    }
    if hunt_id.trim().is_empty() || hunt_id.contains("ReturnToRamsgate") {
        return false;
    }
    !is_training_hunt_id(hunt_id) || (policy.training_enabled && game_mode == "SHARED")
}

const HIGH_RISK_CLASSES: [&str; 7] = [
    "trial",
    "arena",
    "leaderboard",
    "gauntlet",
    "escalation",
    "race",
    "competitive",
];

pub fn is_high_risk_hunt(hunt_id: &str) -> bool {
    let lower = hunt_id.to_lowercase();
    HIGH_RISK_CLASSES.iter().any(|class| lower.contains(class))
}

/// The first-time-user hunts do not run on the hosted runtime.
pub fn is_known_runtime_incompatible(hunt_id: &str) -> bool {
    hunt_id.split('_').any(|segment| segment.eq_ignore_ascii_case("ftue"))
}

/// In [0, 100): the same party and hunt always land in the same bucket, whatever the order of its accounts.
pub fn rollout_bucket(account_ids: &[String], hunt_id: &str) -> f64 {
    let mut ids: Vec<&str> = account_ids.iter().map(String::as_str).collect();
    ids.sort_unstable();
    ids.dedup();
    let digest = Sha256::digest(format!("{}\n{hunt_id}", ids.join(",")).as_bytes());
    let first = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    f64::from(first) / 4_294_967_296.0 * 100.0
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteInput {
    pub game_mode: String,
    pub hunt_id: String,
    pub account_ids: Vec<String>,
    pub party_size: i64,
    pub is_private_party: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HuntRoute {
    Dedicated,
    PlayerHosted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteReason {
    EmergencyStop,
    FeatureDisabled,
    NotAPlayerHostedHunt,
    PrivatePartyRequired,
    PartySizeExceeded,
    AccountAllowlistEmpty,
    AccountNotAllowlisted,
    HuntAllowlistEmpty,
    HuntNotAllowlisted,
    HuntRuntimeIncompatible,
    HuntClassExcluded,
    CanaryDisabled,
    Tier1RewardsDisabled,
    Tier1ShadowNotApproved,
    Tier1AccountNotAllowlisted,
    Tier1HuntNotAllowlisted,
    RolloutDisabled,
    RolloutExcluded,
    PlayerHostedNotImplemented,
    Eligible,
}

impl RouteReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EmergencyStop => "P2P_EMERGENCY_STOP",
            Self::FeatureDisabled => "FEATURE_DISABLED",
            Self::NotAPlayerHostedHunt => "NOT_A_PLAYER_HOSTED_HUNT",
            Self::PrivatePartyRequired => "PRIVATE_PARTY_REQUIRED",
            Self::PartySizeExceeded => "PARTY_SIZE_EXCEEDED",
            Self::AccountAllowlistEmpty => "ACCOUNT_ALLOWLIST_EMPTY",
            Self::AccountNotAllowlisted => "ACCOUNT_NOT_ALLOWLISTED",
            Self::HuntAllowlistEmpty => "HUNT_ALLOWLIST_EMPTY",
            Self::HuntNotAllowlisted => "HUNT_NOT_ALLOWLISTED",
            Self::HuntRuntimeIncompatible => "HUNT_RUNTIME_INCOMPATIBLE",
            Self::HuntClassExcluded => "HUNT_CLASS_EXCLUDED",
            Self::CanaryDisabled => "CANARY_DISABLED",
            Self::Tier1RewardsDisabled => "TIER1_REWARDS_DISABLED",
            Self::Tier1ShadowNotApproved => "TIER1_SHADOW_NOT_APPROVED",
            Self::Tier1AccountNotAllowlisted => "TIER1_ACCOUNT_NOT_ALLOWLISTED",
            Self::Tier1HuntNotAllowlisted => "TIER1_HUNT_NOT_ALLOWLISTED",
            Self::RolloutDisabled => "ROLLOUT_DISABLED",
            Self::RolloutExcluded => "ROLLOUT_EXCLUDED",
            Self::PlayerHostedNotImplemented => "PLAYER_HOSTED_NOT_IMPLEMENTED",
            Self::Eligible => "ELIGIBLE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteDecision {
    pub route: HuntRoute,
    pub reason: RouteReason,
    pub rollout_bucket: Option<f64>,
}

fn dedicated(reason: RouteReason) -> RouteDecision {
    RouteDecision {
        route: HuntRoute::Dedicated,
        reason,
        rollout_bucket: None,
    }
}

/// The gates in order; the first that fails sends the hunt to a dedicated server.
pub fn decide_hunt_route(
    input: &RouteInput,
    policy: &HostingPolicy,
    player_hosted_implemented: bool,
    emergency_stop: bool,
) -> RouteDecision {
    use RouteReason as R;
    if emergency_stop {
        return dedicated(R::EmergencyStop);
    }
    if !policy.enabled {
        return dedicated(R::FeatureDisabled);
    }
    if !is_player_hosted_shape(&input.game_mode, &input.hunt_id, policy) {
        return dedicated(R::NotAPlayerHostedHunt);
    }
    if policy.private_parties_only && !input.is_private_party {
        return dedicated(R::PrivatePartyRequired);
    }
    if input.party_size < 1 || input.party_size > policy.max_party_size {
        return dedicated(R::PartySizeExceeded);
    }
    if !policy.allow_all_accounts {
        if policy.allowed_accounts.is_empty() {
            return dedicated(R::AccountAllowlistEmpty);
        }
        if input.account_ids.iter().any(|a| !policy.allowed_accounts.contains(a)) {
            return dedicated(R::AccountNotAllowlisted);
        }
    }
    if !policy.allow_all_resolvable_hunts {
        if policy.allowed_hunts.is_empty() {
            return dedicated(R::HuntAllowlistEmpty);
        }
        if !policy.allowed_hunts.contains(&input.hunt_id) {
            return dedicated(R::HuntNotAllowlisted);
        }
    }
    if is_known_runtime_incompatible(&input.hunt_id) {
        return dedicated(R::HuntRuntimeIncompatible);
    }
    if is_high_risk_hunt(&input.hunt_id) && !policy.allow_high_risk_hunt_classes {
        return dedicated(R::HuntClassExcluded);
    }
    if !policy.canary_enabled {
        return dedicated(R::CanaryDisabled);
    }
    if policy.trust_tier == 1 {
        if !policy.tier1_rewards_enabled {
            return dedicated(R::Tier1RewardsDisabled);
        }
        if !policy.tier1_shadow_approved {
            return dedicated(R::Tier1ShadowNotApproved);
        }
        if input
            .account_ids
            .iter()
            .any(|a| !policy.tier1_allowed_accounts.contains(a))
        {
            return dedicated(R::Tier1AccountNotAllowlisted);
        }
        if !policy.tier1_allowed_hunts.contains(&input.hunt_id) {
            return dedicated(R::Tier1HuntNotAllowlisted);
        }
    }
    if policy.percent_rollout <= 0.0 {
        return dedicated(R::RolloutDisabled);
    }
    let bucket = rollout_bucket(&input.account_ids, &input.hunt_id);
    let with_bucket = |route, reason| RouteDecision {
        route,
        reason,
        rollout_bucket: Some(bucket),
    };
    if bucket >= policy.percent_rollout {
        return with_bucket(HuntRoute::Dedicated, R::RolloutExcluded);
    }
    if !player_hosted_implemented {
        return with_bucket(HuntRoute::Dedicated, R::PlayerHostedNotImplemented);
    }
    with_bucket(HuntRoute::PlayerHosted, R::Eligible)
}

/// The player-hosted policy (Node's `ReadP2PPolicy`). Every gate fails closed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HostingPolicy {
    pub enabled: bool,
    pub allowed_accounts: BTreeSet<String>,
    pub allow_all_accounts: bool,
    pub allowed_hunts: BTreeSet<String>,
    pub allow_all_resolvable_hunts: bool,
    pub allow_high_risk_hunt_classes: bool,
    pub private_parties_only: bool,
    pub max_party_size: i64,
    pub percent_rollout: f64,
    pub force_sdr: bool,
    pub canary_enabled: bool,
    /// 0 is no-reward transport; 1 decides rewards from Guard's evidence.
    pub trust_tier: u8,
    pub tier1_rewards_enabled: bool,
    pub tier1_shadow_approved: bool,
    pub tier1_allowed_accounts: BTreeSet<String>,
    pub tier1_allowed_hunts: BTreeSet<String>,
    pub dedicated_hunts_disabled: bool,
    pub training_enabled: bool,
}

/// `Number(text)` as JavaScript reads it; `None` is NaN.
pub(crate) fn js_number(text: &str) -> Option<f64> {
    let text = crate::realm::js_trim(text);
    if text.is_empty() {
        return Some(0.0);
    }
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        if hex.is_empty() {
            return None;
        }
        return hex
            .chars()
            .try_fold(0f64, |acc, c| c.to_digit(16).map(|d| acc * 16.0 + f64::from(d)));
    }
    match text {
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    let allowed = |b: u8| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-');
    if !text.bytes().all(allowed) {
        return None;
    }
    text.parse::<f64>().ok().filter(|n| !n.is_nan())
}

fn strict_true(settings: &dyn Settings, name: &str) -> bool {
    settings
        .get(name)
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
}

/// Hunt traffic not forced through the relay can show players each other's addresses.
pub fn relay_only_warning(settings: &dyn Settings) -> Option<String> {
    (strict_true(settings, "P2P_HUNTS_ENABLED") && !strict_true(settings, "P2P_FORCE_SDR")).then(|| {
        "WARNING: P2P hunts are enabled without P2P_FORCE_SDR=true: players may see each other's IP address \
         during a hunt, which the Privacy Policy says they do not. Set P2P_FORCE_SDR=true to relay all hunt traffic."
            .to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn numbers_as_javascript_reads_them() {
        for (text, value) in [
            ("", Some(0.0)),
            ("  ", Some(0.0)),
            ("12", Some(12.0)),
            (" 1.5 ", Some(1.5)),
            ("-1", Some(-1.0)),
            ("1e3", Some(1000.0)),
            ("0x1f", Some(31.0)),
            ("0X10", Some(16.0)),
            ("Infinity", Some(f64::INFINITY)),
            ("-Infinity", Some(f64::NEG_INFINITY)),
            ("0x", None),
            ("0xg", None),
            ("abc", None),
            ("NaN", None),
            ("1,000", None),
            ("--1", None),
        ] {
            assert_eq!(js_number(text), value, "{text:?}");
        }
    }

    #[test]
    fn relay_warning_only_without_force_sdr() {
        let with = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
        };
        let warning = relay_only_warning(&with(&[("P2P_HUNTS_ENABLED", " TRUE ")])).unwrap();
        assert!(warning.starts_with("WARNING: P2P hunts are enabled without P2P_FORCE_SDR=true"));
        assert!(!warning.contains('\n'));
        assert_eq!(
            relay_only_warning(&with(&[("P2P_HUNTS_ENABLED", "true"), ("P2P_FORCE_SDR", "true")])),
            None
        );
        assert_eq!(relay_only_warning(&with(&[("P2P_HUNTS_ENABLED", "1")])), None);
        assert_eq!(relay_only_warning(&with(&[])), None);
    }
}
