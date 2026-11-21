// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Hosting and trust tiers: the player-hosted policy, whether a hunt is routed to a player's PC, and the relay
//! warning. Tier 0 is no-reward transport; tier 1 decides rewards from Guard's evidence. Every gate fails closed.

use std::collections::BTreeSet;

use rustic_rifts_seam::Settings;
use sha2::{Digest as _, Sha256};

/// Declares the policy struct and its reader together, so each field sits beside the setting it comes from.
macro_rules! hosting_policy {
    (|$s:ident| { $( $(#[$doc:meta])* $field:ident: $ty:ty = $read:expr ),* $(,)? }) => {
        /// The player-hosted policy (Node's `ReadP2PPolicy`). Every gate fails closed.
        #[derive(Debug, Clone, PartialEq, Default)]
        pub struct HostingPolicy { $( $(#[$doc])* pub $field: $ty, )* }

        impl HostingPolicy {
            /// Reads every gate from the settings; a missing or invalid value is the closed default.
            pub fn from_settings($s: &dyn Settings) -> Self {
                Self { $( $field: $read, )* }
            }
        }
    };
}

hosting_policy!(|s| {
    /// Whether player-hosted hunts are on.
    enabled: bool = flag(s, "P2P_HUNTS_ENABLED"),
    /// Accounts allowed to play hosted hunts.
    allowed_accounts: BTreeSet<String> = list(s, "P2P_ALLOWED_ACCOUNTS"),
    /// Whether every account is allowed.
    allow_all_accounts: bool = flag(s, "P2P_ALLOW_ALL_ACCOUNTS"),
    /// Hunt ids allowed to be hosted.
    allowed_hunts: BTreeSet<String> = list(s, "P2P_ALLOWED_HUNTS"),
    /// Whether every hunt with a manifest is allowed.
    allow_all_resolvable_hunts: bool = flag(s, "P2P_ALLOW_ALL_RESOLVABLE_HUNTS"),
    /// Whether trials, arenas and other competitive classes may be hosted.
    allow_high_risk_hunt_classes: bool = flag(s, "P2P_ALLOW_HIGH_RISK_HUNT_CLASSES"),
    /// Whether only private parties are hosted; only an explicit `false` opens public parties.
    private_parties_only: bool = !trimmed(s, "P2P_PRIVATE_PARTIES_ONLY").is_some_and(|v| v.eq_ignore_ascii_case("false")),
    /// The largest party a hosted hunt takes, 1 to 4.
    max_party_size: i64 = numeric(s, "P2P_MAX_PARTY_SIZE", |n| n.fract() == 0.0 && (1.0..=4.0).contains(&n))
        .map_or(4, |n| n as i64),
    /// The share of parties that get player hosting, 0 to 100.
    percent_rollout: f64 = numeric(s, "P2P_PERCENT_ROLLOUT", |n| n.is_finite() && (0.0..=100.0).contains(&n))
        .unwrap_or_default(),
    /// Whether hunt traffic must go through the relay.
    force_sdr: bool = flag(s, "P2P_FORCE_SDR"),
    /// Whether the tier-0 canary is on.
    canary_enabled: bool = flag(s, "P2P_TIER0_CANARY_ENABLED"),
    /// 0 is no-reward transport; 1 decides rewards from Guard's evidence.
    trust_tier: u8 = if trimmed(s, "P2P_TRUST_TIER") == Some("1") { 1 } else { 0 },
    /// Whether tier 1 may grant rewards.
    tier1_rewards_enabled: bool = flag(s, "P2P_TIER1_REWARDS_ENABLED"),
    /// Whether tier 1's shadow comparison was approved.
    tier1_shadow_approved: bool = flag(s, "P2P_TIER1_SHADOW_APPROVED"),
    /// Accounts allowed at tier 1.
    tier1_allowed_accounts: BTreeSet<String> = list(s, "P2P_TIER1_ALLOWED_ACCOUNTS"),
    /// Hunt ids allowed at tier 1.
    tier1_allowed_hunts: BTreeSet<String> = list(s, "P2P_TIER1_ALLOWED_HUNTS"),
    /// Whether no dedicated servers take hunts, either by the setting or because none is configured.
    dedicated_hunts_disabled: bool = flag(s, "P2P_DISABLE_DEDICATED_HUNTS") || !dedicated_servers_configured(s),
    /// Whether the training dojo may be hosted.
    training_enabled: bool = flag(s, "P2P_TRAINING_ENABLED"),
});

fn trimmed<'a>(settings: &'a dyn Settings, name: &str) -> Option<&'a str> {
    settings.get(name).map(str::trim)
}

fn flag(settings: &dyn Settings, name: &str) -> bool {
    matches!(trimmed(settings, name), Some(v) if v.eq_ignore_ascii_case("true"))
}

fn list(settings: &dyn Settings, name: &str) -> BTreeSet<String> {
    let Some(raw) = settings.get(name) else {
        return BTreeSet::new();
    };
    raw.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(String::from)
        .collect()
}

fn numeric(settings: &dyn Settings, name: &str, valid: impl Fn(f64) -> bool) -> Option<f64> {
    settings.get(name).and_then(js_number).filter(|n| valid(*n))
}

/// Whether `DEPLOYSERVER_URL` names a dedicated server deployer.
pub fn dedicated_servers_configured(settings: &dyn Settings) -> bool {
    trimmed(settings, "DEPLOYSERVER_URL").is_some_and(|url| !url.is_empty())
}

/// Whether the hunt id is the training dojo.
pub fn is_training_hunt_id(hunt_id: &str) -> bool {
    hunt_id.contains("TrainingDojo")
}

/// Whether this is a dojo request player hosting takes: `SHARED` mode, with hosting and training on.
pub fn is_p2p_training_request(game_mode: &str, hunt_id: &str, policy: &HostingPolicy) -> bool {
    game_mode == "SHARED" && policy.enabled && policy.training_enabled && is_training_hunt_id(hunt_id)
}

fn is_player_hosted_shape(game_mode: &str, hunt_id: &str, policy: &HostingPolicy) -> bool {
    let mode_ok = game_mode == "ISLAND" || game_mode == "SHARED";
    let hunt_ok = !hunt_id.trim().is_empty() && !hunt_id.contains("ReturnToRamsgate");
    let training_ok = !is_training_hunt_id(hunt_id) || (game_mode == "SHARED" && policy.training_enabled);
    mode_ok && hunt_ok && training_ok
}

/// Whether the hunt is a trial, arena, gauntlet, escalation, race or other competitive class.
pub fn is_high_risk_hunt(hunt_id: &str) -> bool {
    let lower = hunt_id.to_lowercase();
    [
        "trial",
        "arena",
        "leaderboard",
        "gauntlet",
        "escalation",
        "race",
        "competitive",
    ]
    .into_iter()
    .any(|class| lower.contains(class))
}

/// The first-time-user hunts do not run on the hosted runtime.
pub fn is_known_runtime_incompatible(hunt_id: &str) -> bool {
    hunt_id.split('_').any(|part| part.eq_ignore_ascii_case("ftue"))
}

/// In [0, 100): the same party and hunt always land in the same bucket, whatever the order of its accounts.
pub fn rollout_bucket(account_ids: &[String], hunt_id: &str) -> f64 {
    let party: BTreeSet<&str> = account_ids.iter().map(String::as_str).collect();
    let mut hasher = Sha256::new();
    hasher.update(party.into_iter().collect::<Vec<_>>().join(","));
    hasher.update(b"\n");
    hasher.update(hunt_id);
    let digest = hasher.finalize();
    let mut head = [0u8; 4];
    head.copy_from_slice(&digest[..4]);
    f64::from(u32::from_be_bytes(head)) * 100.0 / 4_294_967_296.0
}

/// What a hunt request is routed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteInput {
    /// The game mode asked for.
    pub game_mode: String,
    /// The hunt id asked for.
    pub hunt_id: String,
    /// Every account that would play.
    pub account_ids: Vec<String>,
    /// How many play.
    pub party_size: i64,
    /// Whether the party is private.
    pub is_private_party: bool,
}

/// Where a hunt is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HuntRoute {
    /// A dedicated server.
    Dedicated,
    /// A player's PC.
    PlayerHosted,
}

/// One table gives the variants, in order, and their wire codes.
macro_rules! route_reasons {
    ($( $variant:ident = $code:literal ),* $(,)?) => {
        /// Why a hunt went where it did: the gate that sent it to a dedicated server, or [`RouteReason::Eligible`].
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[allow(missing_docs, reason = "the reason table takes no attributes; each variant is its wire code")]
        pub enum RouteReason { $( $variant, )* }

        impl RouteReason {
            /// The reason's wire code.
            pub const fn as_str(self) -> &'static str {
                match self { $( Self::$variant => $code, )* }
            }
        }
    };
}

route_reasons! {
    EmergencyStop = "P2P_EMERGENCY_STOP", FeatureDisabled = "FEATURE_DISABLED",
    NotAPlayerHostedHunt = "NOT_A_PLAYER_HOSTED_HUNT", PrivatePartyRequired = "PRIVATE_PARTY_REQUIRED",
    PartySizeExceeded = "PARTY_SIZE_EXCEEDED", AccountAllowlistEmpty = "ACCOUNT_ALLOWLIST_EMPTY",
    AccountNotAllowlisted = "ACCOUNT_NOT_ALLOWLISTED", HuntAllowlistEmpty = "HUNT_ALLOWLIST_EMPTY",
    HuntNotAllowlisted = "HUNT_NOT_ALLOWLISTED", HuntRuntimeIncompatible = "HUNT_RUNTIME_INCOMPATIBLE",
    HuntClassExcluded = "HUNT_CLASS_EXCLUDED", CanaryDisabled = "CANARY_DISABLED",
    Tier1RewardsDisabled = "TIER1_REWARDS_DISABLED", Tier1ShadowNotApproved = "TIER1_SHADOW_NOT_APPROVED",
    Tier1AccountNotAllowlisted = "TIER1_ACCOUNT_NOT_ALLOWLISTED", Tier1HuntNotAllowlisted = "TIER1_HUNT_NOT_ALLOWLISTED",
    RolloutDisabled = "ROLLOUT_DISABLED", RolloutExcluded = "ROLLOUT_EXCLUDED",
    PlayerHostedNotImplemented = "PLAYER_HOSTED_NOT_IMPLEMENTED", Eligible = "ELIGIBLE",
}

/// Where a hunt goes, and why.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteDecision {
    /// Where it goes.
    pub route: HuntRoute,
    /// The gate that decided, or [`RouteReason::Eligible`].
    pub reason: RouteReason,
    /// The party's rollout bucket, 0 to 100, once every gate passed.
    pub rollout_bucket: Option<f64>,
}

type Gate = fn(&RouteInput, &HostingPolicy) -> bool;

fn outside(set: &BTreeSet<String>, accounts: &[String]) -> bool {
    accounts.iter().any(|id| !set.contains(id))
}

/// The policy gates in the order they are checked; a gate that returns true sends the hunt to a dedicated server.
const GATES: &[(Gate, RouteReason)] = {
    use RouteReason as R;
    &[
        (|_, p| !p.enabled, R::FeatureDisabled),
        (
            |i, p| !is_player_hosted_shape(&i.game_mode, &i.hunt_id, p),
            R::NotAPlayerHostedHunt,
        ),
        (
            |i, p| p.private_parties_only && !i.is_private_party,
            R::PrivatePartyRequired,
        ),
        (
            |i, p| !(1..=p.max_party_size).contains(&i.party_size),
            R::PartySizeExceeded,
        ),
        (
            |_, p| !p.allow_all_accounts && p.allowed_accounts.is_empty(),
            R::AccountAllowlistEmpty,
        ),
        (
            |i, p| !p.allow_all_accounts && outside(&p.allowed_accounts, &i.account_ids),
            R::AccountNotAllowlisted,
        ),
        (
            |_, p| !p.allow_all_resolvable_hunts && p.allowed_hunts.is_empty(),
            R::HuntAllowlistEmpty,
        ),
        (
            |i, p| !p.allow_all_resolvable_hunts && !p.allowed_hunts.contains(&i.hunt_id),
            R::HuntNotAllowlisted,
        ),
        (
            |i, _| is_known_runtime_incompatible(&i.hunt_id),
            R::HuntRuntimeIncompatible,
        ),
        (
            |i, p| !p.allow_high_risk_hunt_classes && is_high_risk_hunt(&i.hunt_id),
            R::HuntClassExcluded,
        ),
        (|_, p| !p.canary_enabled, R::CanaryDisabled),
        (
            |_, p| p.trust_tier == 1 && !p.tier1_rewards_enabled,
            R::Tier1RewardsDisabled,
        ),
        (
            |_, p| p.trust_tier == 1 && !p.tier1_shadow_approved,
            R::Tier1ShadowNotApproved,
        ),
        (
            |i, p| p.trust_tier == 1 && outside(&p.tier1_allowed_accounts, &i.account_ids),
            R::Tier1AccountNotAllowlisted,
        ),
        (
            |i, p| p.trust_tier == 1 && !p.tier1_allowed_hunts.contains(&i.hunt_id),
            R::Tier1HuntNotAllowlisted,
        ),
        (|_, p| p.percent_rollout <= 0.0, R::RolloutDisabled),
    ]
};

/// The gates in order; the first that fails sends the hunt to a dedicated server.
pub fn decide_hunt_route(
    input: &RouteInput,
    policy: &HostingPolicy,
    player_hosted_implemented: bool,
    emergency_stop: bool,
) -> RouteDecision {
    let decision = |route, reason, rollout_bucket| RouteDecision {
        route,
        reason,
        rollout_bucket,
    };
    let refused = if emergency_stop {
        Some(RouteReason::EmergencyStop)
    } else {
        GATES
            .iter()
            .find(|(gate, _)| gate(input, policy))
            .map(|(_, reason)| *reason)
    };
    if let Some(reason) = refused {
        return decision(HuntRoute::Dedicated, reason, None);
    }
    let bucket = rollout_bucket(&input.account_ids, &input.hunt_id);
    let (route, reason) = match (bucket < policy.percent_rollout, player_hosted_implemented) {
        (false, _) => (HuntRoute::Dedicated, RouteReason::RolloutExcluded),
        (true, false) => (HuntRoute::Dedicated, RouteReason::PlayerHostedNotImplemented),
        (true, true) => (HuntRoute::PlayerHosted, RouteReason::Eligible),
    };
    decision(route, reason, Some(bucket))
}

/// `Number(text)` as JavaScript reads it; `None` is NaN.
pub(crate) fn js_number(text: &str) -> Option<f64> {
    let text = crate::realm::js_trim(text);
    let hex = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"));
    match (text, hex) {
        ("", _) => Some(0.0),
        (_, Some("")) => None,
        (_, Some(digits)) => digits
            .chars()
            .try_fold(0f64, |n, c| Some(n * 16.0 + f64::from(c.to_digit(16)?))),
        ("Infinity" | "+Infinity", None) => Some(f64::INFINITY),
        ("-Infinity", None) => Some(f64::NEG_INFINITY),
        (decimal, None) => {
            let plain = decimal.bytes().all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b));
            plain
                .then(|| decimal.parse::<f64>().ok())
                .flatten()
                .filter(|n| !n.is_nan())
        }
    }
}

/// Hunt traffic not forced through the relay can show players each other's addresses.
pub fn relay_only_warning(settings: &dyn Settings) -> Option<String> {
    if !flag(settings, "P2P_HUNTS_ENABLED") || flag(settings, "P2P_FORCE_SDR") {
        return None;
    }
    Some(String::from(concat!(
        "WARNING: P2P hunts are enabled without P2P_FORCE_SDR=true: players may see each other's IP address ",
        "during a hunt, which the Privacy Policy says they do not. Set P2P_FORCE_SDR=true to relay all hunt traffic."
    )))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{js_number, relay_only_warning};

    const NUMBERS: &[(&str, Option<f64>)] = &[
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
    ];

    #[test]
    fn numbers_as_javascript_reads_them() {
        for &(text, expected) in NUMBERS {
            assert_eq!(js_number(text), expected, "{text:?}");
        }
    }

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|&(k, v)| (k.to_owned(), v.to_owned())).collect()
    }

    #[test]
    fn relay_warning_only_without_force_sdr() {
        let warning = relay_only_warning(&env(&[("P2P_HUNTS_ENABLED", " TRUE ")])).unwrap();
        assert!(warning.starts_with("WARNING: P2P hunts are enabled without P2P_FORCE_SDR=true"));
        assert!(!warning.contains('\n'));
        for quiet in [
            &[("P2P_HUNTS_ENABLED", "true"), ("P2P_FORCE_SDR", "true")][..],
            &[("P2P_HUNTS_ENABLED", "1")],
            &[],
        ] {
            assert_eq!(relay_only_warning(&env(quiet)), None);
        }
    }
}
