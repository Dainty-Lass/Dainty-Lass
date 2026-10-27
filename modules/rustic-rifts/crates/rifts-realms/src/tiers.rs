//! Hosting tiers (`p2p/config/p2pPolicy.ts`): whether a hunt runs on a dedicated server or on a player's PC, and at
//! which trust tier. Tier 0 is no-reward transport (rewards are the host's tagged writes); Tier 1 decides rewards from
//! Guard's evidence and needs its own switches. Every gate fails closed.

use std::collections::BTreeSet;

use rifts_seam::Settings;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuntRoute {
    Dedicated,
    PlayerHosted,
}

/// `HuntRouteReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    HuntClassExcluded,
    HuntRuntimeIncompatible,
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
            Self::HuntClassExcluded => "HUNT_CLASS_EXCLUDED",
            Self::HuntRuntimeIncompatible => "HUNT_RUNTIME_INCOMPATIBLE",
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

/// `P2PPolicy`, read from the backend's settings.
#[derive(Debug, Clone, PartialEq)]
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
    /// 0 or 1.
    pub trust_tier: u8,
    pub tier1_rewards_enabled: bool,
    pub tier1_shadow_approved: bool,
    pub tier1_allowed_accounts: BTreeSet<String>,
    pub tier1_allowed_hunts: BTreeSet<String>,
    /// Also on without a DeployServer: a hunt that can't be player-hosted is cancelled, not sent nowhere.
    pub dedicated_hunts_disabled: bool,
    pub training_enabled: bool,
}

/// Literal `true` (trimmed, any case); `1`, `on` and `yes` fail closed.
fn strict_true(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
}

fn csv_set(value: Option<&str>) -> BTreeSet<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `Number(text)` for the decimal forms settings use; `None` for NaN.
pub(crate) fn js_number(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(0.0);
    }
    if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        #[allow(clippy::cast_precision_loss)]
        return u64::from_str_radix(hex, 16).ok().map(|n| n as f64);
    }
    match trimmed {
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    let numeric = trimmed
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
    if !numeric {
        return None;
    }
    trimmed.parse::<f64>().ok()
}

/// `ReadBoundedInteger`: the fallback for blank, fractional or out-of-range values.
pub(crate) fn bounded_integer(value: Option<&str>, fallback: i64, min: i64, max: i64) -> i64 {
    let Some(text) = value.filter(|v| !v.trim().is_empty()) else {
        return fallback;
    };
    match js_number(text) {
        #[allow(clippy::cast_possible_truncation)]
        Some(n) if n.fract() == 0.0 && n >= min as f64 && n <= max as f64 => n as i64,
        _ => fallback,
    }
}

fn bounded_percent(value: Option<&str>) -> f64 {
    let Some(text) = value.filter(|v| !v.trim().is_empty()) else {
        return 0.0;
    };
    match js_number(text) {
        Some(n) if n.is_finite() && (0.0..=100.0).contains(&n) => n,
        _ => 0.0,
    }
}

/// `DedicatedServersConfigured`: a DeployServer is set.
pub fn dedicated_servers_configured(settings: &dyn Settings) -> bool {
    !settings.get("DEPLOYSERVER_URL").unwrap_or_default().trim().is_empty()
}

impl HostingPolicy {
    /// `ReadP2PPolicy`.
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let get = |name: &str| settings.get(name);
        Self {
            enabled: strict_true(get("P2P_HUNTS_ENABLED")),
            allowed_accounts: csv_set(get("P2P_ALLOWED_ACCOUNTS")),
            allow_all_accounts: strict_true(get("P2P_ALLOW_ALL_ACCOUNTS")),
            allowed_hunts: csv_set(get("P2P_ALLOWED_HUNTS")),
            allow_all_resolvable_hunts: strict_true(get("P2P_ALLOW_ALL_RESOLVABLE_HUNTS")),
            allow_high_risk_hunt_classes: strict_true(get("P2P_ALLOW_HIGH_RISK_HUNT_CLASSES")),
            private_parties_only: get("P2P_PRIVATE_PARTIES_ONLY")
                .is_none_or(|v| !v.trim().eq_ignore_ascii_case("false")),
            max_party_size: bounded_integer(get("P2P_MAX_PARTY_SIZE"), 4, 1, 4),
            percent_rollout: bounded_percent(get("P2P_PERCENT_ROLLOUT")),
            force_sdr: strict_true(get("P2P_FORCE_SDR")),
            canary_enabled: strict_true(get("P2P_TIER0_CANARY_ENABLED")),
            // An absent, malformed or future tier stays at the no-reward tier
            trust_tier: u8::from(get("P2P_TRUST_TIER").is_some_and(|v| v.trim() == "1")),
            tier1_rewards_enabled: strict_true(get("P2P_TIER1_REWARDS_ENABLED")),
            tier1_shadow_approved: strict_true(get("P2P_TIER1_SHADOW_APPROVED")),
            tier1_allowed_accounts: csv_set(get("P2P_TIER1_ALLOWED_ACCOUNTS")),
            tier1_allowed_hunts: csv_set(get("P2P_TIER1_ALLOWED_HUNTS")),
            dedicated_hunts_disabled: strict_true(get("P2P_DISABLE_DEDICATED_HUNTS"))
                || !dedicated_servers_configured(settings),
            training_enabled: strict_true(get("P2P_TRAINING_ENABLED")),
        }
    }
}

/// The startup warning when hunts may go direct and players could see each other's address.
pub fn relay_only_warning(settings: &dyn Settings) -> Option<&'static str> {
    if !strict_true(settings.get("P2P_HUNTS_ENABLED")) || strict_true(settings.get("P2P_FORCE_SDR")) {
        return None;
    }
    Some(
        "WARNING: P2P hunts are enabled without P2P_FORCE_SDR=true: players may see each other's IP address during a \
         hunt, which the Privacy Policy says they do not. Set P2P_FORCE_SDR=true to relay all hunt traffic.",
    )
}

/// The Training Grounds' hunt id; the client asks for it with GameMode SHARED.
pub fn is_training_hunt_id(hunt_id: &str) -> bool {
    hunt_id.contains("TrainingDojo")
}

/// A Training request this policy may host on a player's PC.
pub fn is_p2p_training_request(game_mode: &str, hunt_id: &str, policy: &HostingPolicy) -> bool {
    policy.enabled && policy.training_enabled && game_mode == "SHARED" && is_training_hunt_id(hunt_id)
}

/// 1.12 sends Hunting Grounds as SHARED; the hunt allowlist stays the authority.
fn is_player_hosted_hunt(game_mode: &str, hunt_id: &str, training_enabled: bool) -> bool {
    if game_mode != "ISLAND" && game_mode != "SHARED" {
        return false;
    }
    if hunt_id.trim().is_empty() || hunt_id.contains("ReturnToRamsgate") {
        return false;
    }
    if is_training_hunt_id(hunt_id) {
        return training_enabled && game_mode == "SHARED";
    }
    true
}

/// Trials' real ids say "Arena", so both names are matched.
fn is_excluded_canary_class(hunt_id: &str) -> bool {
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
    .iter()
    .any(|class| lower.contains(class))
}

/// The 1.12 FTUE's cinematic recurses on this runtime: a measured exclusion no switch overrides.
pub fn is_known_runtime_incompatible(hunt_id: &str) -> bool {
    hunt_id.split('_').any(|segment| segment.eq_ignore_ascii_case("ftue"))
}

/// A stable bucket in [0, 100): a party keeps its canary bucket across retries. Selection, not security.
pub fn rollout_bucket(account_ids: &[String], hunt_id: &str) -> f64 {
    let roster: BTreeSet<&str> = account_ids.iter().map(String::as_str).collect();
    let stable = roster.into_iter().collect::<Vec<_>>().join(",");
    let digest = Sha256::digest(format!("{stable}\n{hunt_id}").as_bytes());
    let head = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    f64::from(head) / 4_294_967_296.0 * 100.0
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteInput {
    pub game_mode: String,
    pub hunt_id: String,
    pub account_ids: Vec<String>,
    pub party_size: i64,
    pub is_private_party: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteDecision {
    pub route: HuntRoute,
    pub reason: RouteReason,
    pub rollout_bucket: Option<f64>,
}

fn dedicated(reason: RouteReason, bucket: Option<f64>) -> RouteDecision {
    RouteDecision {
        route: HuntRoute::Dedicated,
        reason,
        rollout_bucket: bucket,
    }
}

/// `DecideHuntRoute`: the gates in Node's order; the first that fails names the reason.
pub fn decide_hunt_route(
    input: &RouteInput,
    policy: &HostingPolicy,
    player_hosted_implemented: bool,
    emergency_stop: bool,
) -> RouteDecision {
    use RouteReason as R;
    if emergency_stop {
        return dedicated(R::EmergencyStop, None);
    }
    if !policy.enabled {
        return dedicated(R::FeatureDisabled, None);
    }
    if !is_player_hosted_hunt(&input.game_mode, &input.hunt_id, policy.training_enabled) {
        return dedicated(R::NotAPlayerHostedHunt, None);
    }
    if policy.private_parties_only && !input.is_private_party {
        return dedicated(R::PrivatePartyRequired, None);
    }
    if input.party_size < 1 || input.party_size > policy.max_party_size {
        return dedicated(R::PartySizeExceeded, None);
    }
    if !policy.allow_all_accounts && policy.allowed_accounts.is_empty() {
        return dedicated(R::AccountAllowlistEmpty, None);
    }
    if !policy.allow_all_accounts && input.account_ids.iter().any(|a| !policy.allowed_accounts.contains(a)) {
        return dedicated(R::AccountNotAllowlisted, None);
    }
    if !policy.allow_all_resolvable_hunts && policy.allowed_hunts.is_empty() {
        return dedicated(R::HuntAllowlistEmpty, None);
    }
    if !policy.allow_all_resolvable_hunts && !policy.allowed_hunts.contains(&input.hunt_id) {
        return dedicated(R::HuntNotAllowlisted, None);
    }
    if is_known_runtime_incompatible(&input.hunt_id) {
        return dedicated(R::HuntRuntimeIncompatible, None);
    }
    if !policy.allow_high_risk_hunt_classes && is_excluded_canary_class(&input.hunt_id) {
        return dedicated(R::HuntClassExcluded, None);
    }
    if !policy.canary_enabled {
        return dedicated(R::CanaryDisabled, None);
    }
    if policy.trust_tier == 1 {
        if !policy.tier1_rewards_enabled {
            return dedicated(R::Tier1RewardsDisabled, None);
        }
        if !policy.tier1_shadow_approved {
            return dedicated(R::Tier1ShadowNotApproved, None);
        }
        if input
            .account_ids
            .iter()
            .any(|a| !policy.tier1_allowed_accounts.contains(a))
        {
            return dedicated(R::Tier1AccountNotAllowlisted, None);
        }
        if !policy.tier1_allowed_hunts.contains(&input.hunt_id) {
            return dedicated(R::Tier1HuntNotAllowlisted, None);
        }
    }
    if policy.percent_rollout <= 0.0 {
        return dedicated(R::RolloutDisabled, None);
    }
    let bucket = rollout_bucket(&input.account_ids, &input.hunt_id);
    if bucket >= policy.percent_rollout {
        return dedicated(R::RolloutExcluded, Some(bucket));
    }
    // A capability outside the settings, so an operator's typo can't route a client into a missing transport
    if !player_hosted_implemented {
        return dedicated(R::PlayerHostedNotImplemented, Some(bucket));
    }
    RouteDecision {
        route: HuntRoute::PlayerHosted,
        reason: R::Eligible,
        rollout_bucket: Some(bucket),
    }
}
