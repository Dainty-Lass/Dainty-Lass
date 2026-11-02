//! Guard's settings (RF1, section 5): `hostPolicyMode.ts`, `huntEvidence.ts`, the trust tier and Tier 1 gates of
//! `p2pPolicy.ts`, and the switches of the census, host writes and the distributed traffic limiter.

use std::collections::BTreeSet;

use rifts_seam::{AccountId, Settings};

use crate::json::js_string;
use crate::ports::GuardLog;

pub const HOST_POLICY_ENV: &str = "MYSTICPARADOX_HOST_POLICY";
pub const HUNT_EVIDENCE_ENV: &str = "MYSTICPARADOX_HUNT_EVIDENCE";

/// `MYSTICPARADOX_HOST_POLICY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostPolicyMode {
    #[default]
    Observe,
    Enforce,
}

impl HostPolicyMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Enforce => "enforce",
        }
    }
}

/// `P2P_TRUST_TIER` (RF1, decision 4 puts it here): 0, or 1 for evidence and Tier 1 rewards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrustTier {
    #[default]
    Tier0,
    Tier1,
}

impl TrustTier {
    /// Only a literal `1` (trimmed) raises it: an absent, malformed or future value stays at the no-reward tier.
    pub fn from_setting(value: Option<&str>) -> Self {
        if value.map(str::trim) == Some("1") {
            Self::Tier1
        } else {
            Self::Tier0
        }
    }

    pub const fn number(self) -> u8 {
        match self {
            Self::Tier0 => 0,
            Self::Tier1 => 1,
        }
    }
}

/// `HostPolicySetting`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPolicySetting {
    pub mode: HostPolicyMode,
    pub is_default: bool,
    pub warning: Option<String>,
}

/// `ResolveHostPolicyMode`: observe and enforce, trimmed and in any case; anything else observes, with a warning.
pub fn resolve_host_policy(settings: &dyn Settings) -> HostPolicySetting {
    let raw = settings.get(HOST_POLICY_ENV).unwrap_or_default().trim().to_lowercase();
    let (mode, is_default, warning) = match raw.as_str() {
        "" => (HostPolicyMode::Observe, true, None),
        "observe" => (HostPolicyMode::Observe, false, None),
        "enforce" => (HostPolicyMode::Enforce, false, None),
        _ => (
            HostPolicyMode::Observe,
            false,
            Some(format!(
                "{HOST_POLICY_ENV} must be observe or enforce (got {}); it is treated as observe, so host policy \
                 refuses nothing",
                js_string(&raw)
            )),
        ),
    };
    HostPolicySetting {
        mode,
        is_default,
        warning,
    }
}

/// `LogHostPolicyMode`.
pub fn log_host_policy(setting: &HostPolicySetting, log: &dyn GuardLog) {
    if let Some(warning) = &setting.warning {
        log.warn(&format!("[Startup] WARNING: {warning}"));
    }
    match setting.mode {
        HostPolicyMode::Observe => log.info(&format!(
            "[Startup] {HOST_POLICY_ENV}{}: host grant classes, the draft token cap and write kinds refuse nothing; \
             would-be refusals are logged as [HostPolicyObserve]",
            if setting.is_default {
                " not set (default observe)"
            } else {
                "=observe"
            }
        )),
        HostPolicyMode::Enforce => log.info(&format!(
            "[Startup] {HOST_POLICY_ENV}=enforce: host grant classes, the draft token cap and write kinds refuse \
             as before"
        )),
    }
}

/// `HuntEvidenceSetting`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntEvidenceSetting {
    pub enabled: bool,
    pub is_default: bool,
    pub warning: Option<String>,
}

/// `ResolveHuntEvidence`: on or off (trimmed, any case); otherwise on at trust tier 1 and off at 0, since the host's
/// transport posts the evidence on its packet thread and every hosted hunt lagged (`huntEvidence.ts`).
pub fn resolve_hunt_evidence(settings: &dyn Settings) -> HuntEvidenceSetting {
    let tier_default = TrustTier::from_setting(settings.get("P2P_TRUST_TIER")) == TrustTier::Tier1;
    let raw = settings
        .get(HUNT_EVIDENCE_ENV)
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    match raw.as_str() {
        "" => HuntEvidenceSetting {
            enabled: tier_default,
            is_default: true,
            warning: None,
        },
        "on" => HuntEvidenceSetting {
            enabled: true,
            is_default: false,
            warning: None,
        },
        "off" => HuntEvidenceSetting {
            enabled: false,
            is_default: false,
            warning: None,
        },
        _ => HuntEvidenceSetting {
            enabled: tier_default,
            is_default: false,
            warning: Some(format!(
                "{HUNT_EVIDENCE_ENV} must be on or off (got {}); the trust tier's default applies ({})",
                js_string(&raw),
                if tier_default { "on" } else { "off" }
            )),
        },
    }
}

/// `LogHuntEvidence`.
pub fn log_hunt_evidence(setting: &HuntEvidenceSetting, log: &dyn GuardLog) {
    if let Some(warning) = &setting.warning {
        log.warn(&format!("[Startup] WARNING: {warning}"));
    }
    let source = if setting.is_default {
        " not set (the trust tier's default)"
    } else {
        ""
    };
    if setting.enabled {
        let source = if source.is_empty() { "=on" } else { source };
        log.info(&format!(
            "[Startup] {HUNT_EVIDENCE_ENV}{source}: hosted hunts send Hunt Guard evidence (the host's transport \
             posts it on its packet thread)"
        ));
    } else {
        let source = if source.is_empty() { "=off" } else { source };
        log.info(&format!(
            "[Startup] {HUNT_EVIDENCE_ENV}{source}: hosted hunts send no Hunt Guard evidence"
        ));
    }
}

/// On unless the value is `0`, `false`, `off` or `no` (trimmed, any case).
fn switch_on(value: Option<&str>) -> bool {
    let value = value.unwrap_or_default().trim();
    !["0", "false", "off", "no"]
        .iter()
        .any(|off| value.eq_ignore_ascii_case(off))
}

/// `IsStrictTrue`: `true` (trimmed, any case) only; 1, on and yes fail closed.
fn strict_true(value: Option<&str>) -> bool {
    value.is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
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

/// The Tier 1 gates of `ReadP2PPolicy`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tier1Policy {
    pub rewards_enabled: bool,
    pub shadow_approved: bool,
    pub allowed_accounts: BTreeSet<String>,
    pub allowed_hunts: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardConfig {
    pub host_policy: HostPolicySetting,
    pub hunt_evidence: HuntEvidenceSetting,
    pub trust_tier: TrustTier,
    pub tier1: Tier1Policy,
    /// `MYSTICPARADOX_HOST_ROUTE_CENSUS`.
    pub census: bool,
    /// `P2P_HOST_WRITES`: the kill switch for host writes besides inventory.
    pub host_writes: bool,
    /// `P2P_DISTRIBUTED_TRAFFIC_LIMITER`.
    pub distributed_traffic: bool,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self::from_settings(&std::collections::BTreeMap::new())
    }
}

impl GuardConfig {
    pub fn from_settings(settings: &dyn Settings) -> Self {
        Self {
            host_policy: resolve_host_policy(settings),
            hunt_evidence: resolve_hunt_evidence(settings),
            trust_tier: TrustTier::from_setting(settings.get("P2P_TRUST_TIER")),
            tier1: Tier1Policy {
                rewards_enabled: strict_true(settings.get("P2P_TIER1_REWARDS_ENABLED")),
                shadow_approved: strict_true(settings.get("P2P_TIER1_SHADOW_APPROVED")),
                allowed_accounts: csv_set(settings.get("P2P_TIER1_ALLOWED_ACCOUNTS")),
                allowed_hunts: csv_set(settings.get("P2P_TIER1_ALLOWED_HUNTS")),
            },
            census: switch_on(settings.get("MYSTICPARADOX_HOST_ROUTE_CENSUS")),
            host_writes: switch_on(settings.get("P2P_HOST_WRITES")),
            distributed_traffic: strict_true(settings.get("P2P_DISTRIBUTED_TRAFFIC_LIMITER")),
        }
    }

    /// `HostPolicyRefuses`: only in enforce mode.
    pub fn host_policy_refuses(&self) -> bool {
        self.host_policy.mode == HostPolicyMode::Enforce
    }

    /// The Tier 1 slice of `DecideHuntRoute`: why a hunt stays dedicated at trust tier 1, or `None`. Realms runs the
    /// gates before and after it.
    pub fn tier1_refusal(&self, accounts: &[AccountId], hunt_id: &str) -> Option<&'static str> {
        if self.trust_tier != TrustTier::Tier1 {
            return None;
        }
        if !self.tier1.rewards_enabled {
            return Some("TIER1_REWARDS_DISABLED");
        }
        if !self.tier1.shadow_approved {
            return Some("TIER1_SHADOW_NOT_APPROVED");
        }
        if accounts
            .iter()
            .any(|account| !self.tier1.allowed_accounts.contains(&account.0))
        {
            return Some("TIER1_ACCOUNT_NOT_ALLOWLISTED");
        }
        if !self.tier1.allowed_hunts.contains(hunt_id) {
            return Some("TIER1_HUNT_NOT_ALLOWLISTED");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::MemoryLog;
    use std::collections::BTreeMap;

    fn settings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    // hostPolicyObserve.gamedata.test.ts: "the mode: unset and observe let policy refusals through, enforce refuses,
    // anything else is observe with a warning"
    #[test]
    fn host_policy_mode() {
        let unset = resolve_host_policy(&settings(&[]));
        assert_eq!(
            (unset.mode, unset.is_default, unset.warning),
            (HostPolicyMode::Observe, true, None)
        );
        let observe = resolve_host_policy(&settings(&[(HOST_POLICY_ENV, " Observe ")]));
        assert_eq!((observe.mode, observe.is_default), (HostPolicyMode::Observe, false));
        let enforce = resolve_host_policy(&settings(&[(HOST_POLICY_ENV, "ENFORCE")]));
        assert_eq!(enforce.mode, HostPolicyMode::Enforce);
        let odd = resolve_host_policy(&settings(&[(HOST_POLICY_ENV, "strict")]));
        assert_eq!(odd.mode, HostPolicyMode::Observe);
        assert!(
            odd.warning
                .unwrap()
                .contains("must be observe or enforce (got \"strict\")")
        );
        assert!(GuardConfig::from_settings(&settings(&[(HOST_POLICY_ENV, "enforce")])).host_policy_refuses());
        assert!(!GuardConfig::from_settings(&settings(&[(HOST_POLICY_ENV, "strict")])).host_policy_refuses());
    }

    #[test]
    fn host_policy_startup_line() {
        let log = MemoryLog::default();
        log_host_policy(&resolve_host_policy(&settings(&[])), &log);
        log_host_policy(&resolve_host_policy(&settings(&[(HOST_POLICY_ENV, "maybe")])), &log);
        log_host_policy(&resolve_host_policy(&settings(&[(HOST_POLICY_ENV, "enforce")])), &log);
        let lines = log.lines();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("info [Startup] MYSTICPARADOX_HOST_POLICY not set (default observe): "));
        assert!(lines[1].starts_with("warn [Startup] WARNING: MYSTICPARADOX_HOST_POLICY must be observe or enforce"));
        assert!(lines[2].starts_with("info [Startup] MYSTICPARADOX_HOST_POLICY=observe: "));
        assert!(lines[3].starts_with("info [Startup] MYSTICPARADOX_HOST_POLICY=enforce: "));
    }

    // huntEvidence.test.ts
    #[test]
    fn hunt_evidence_is_off_by_default_at_tier_0_and_on_at_tier_1() {
        let resolve = |pairs: &[(&str, &str)]| {
            let setting = resolve_hunt_evidence(&settings(pairs));
            (setting.enabled, setting.is_default)
        };
        assert_eq!(resolve(&[]), (false, true));
        assert_eq!(resolve(&[(HUNT_EVIDENCE_ENV, "  ")]), (false, true));
        assert_eq!(resolve(&[("P2P_TRUST_TIER", "0")]), (false, true));
        assert_eq!(resolve(&[("P2P_TRUST_TIER", "1")]), (true, true));
    }

    #[test]
    fn hunt_evidence_on_and_off_are_explicit_at_either_tier() {
        let enabled = |pairs: &[(&str, &str)]| resolve_hunt_evidence(&settings(pairs)).enabled;
        assert!(enabled(&[(HUNT_EVIDENCE_ENV, "on")]));
        assert!(enabled(&[(HUNT_EVIDENCE_ENV, " ON ")]));
        assert!(!enabled(&[(HUNT_EVIDENCE_ENV, "off"), ("P2P_TRUST_TIER", "1")]));
        assert!(!enabled(&[(HUNT_EVIDENCE_ENV, "Off")]));
    }

    #[test]
    fn hunt_evidence_unrecognised_value_keeps_the_tier_default_and_says_so() {
        let tier0 = resolve_hunt_evidence(&settings(&[(HUNT_EVIDENCE_ENV, "true")]));
        assert!(!tier0.enabled);
        let warning = tier0.warning.unwrap();
        assert!(warning.contains("must be on or off (got \"true\")") && warning.ends_with("(off)"));
        let tier1 = resolve_hunt_evidence(&settings(&[(HUNT_EVIDENCE_ENV, "1"), ("P2P_TRUST_TIER", "1")]));
        assert!(tier1.enabled);
        assert!(tier1.warning.unwrap().ends_with("(on)"));
    }

    #[test]
    fn hunt_evidence_startup_line() {
        let log = MemoryLog::default();
        log_hunt_evidence(&resolve_hunt_evidence(&settings(&[])), &log);
        log_hunt_evidence(&resolve_hunt_evidence(&settings(&[(HUNT_EVIDENCE_ENV, "on")])), &log);
        log_hunt_evidence(&resolve_hunt_evidence(&settings(&[(HUNT_EVIDENCE_ENV, "maybe")])), &log);
        let lines = log.lines();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("info [Startup] MYSTICPARADOX_HUNT_EVIDENCE not set "));
        assert!(lines[0].ends_with("send no Hunt Guard evidence"));
        assert!(
            lines[1]
                .starts_with("info [Startup] MYSTICPARADOX_HUNT_EVIDENCE=on: hosted hunts send Hunt Guard evidence")
        );
        assert!(lines[2].starts_with("warn [Startup] WARNING: MYSTICPARADOX_HUNT_EVIDENCE must be on or off"));
        assert!(lines[3].ends_with("send no Hunt Guard evidence"));
    }

    // p2pPolicy.test.ts: "trust tier defaults to zero and recognizes only explicit tier one"
    #[test]
    fn trust_tier_recognises_only_an_explicit_one() {
        assert_eq!(TrustTier::from_setting(None), TrustTier::Tier0);
        assert_eq!(TrustTier::from_setting(Some(" 1 ")), TrustTier::Tier1);
        assert_eq!(TrustTier::from_setting(Some("2")), TrustTier::Tier0);
        assert_eq!(TrustTier::from_setting(Some("true")), TrustTier::Tier0);
    }

    // p2pPolicy.test.ts: "Tier 1 requires separate reward, shadow, account, and hunt gates"
    #[test]
    fn tier1_requires_separate_gates() {
        let accounts = [AccountId("a".to_owned()), AccountId("b".to_owned())];
        let mut pairs = vec![("P2P_TRUST_TIER", "1")];
        let refusal =
            |pairs: &[(&str, &str)]| GuardConfig::from_settings(&settings(pairs)).tier1_refusal(&accounts, "h");
        assert_eq!(refusal(&pairs), Some("TIER1_REWARDS_DISABLED"));
        pairs.push(("P2P_TIER1_REWARDS_ENABLED", "true"));
        assert_eq!(refusal(&pairs), Some("TIER1_SHADOW_NOT_APPROVED"));
        pairs.push(("P2P_TIER1_SHADOW_APPROVED", "true"));
        assert_eq!(refusal(&pairs), Some("TIER1_ACCOUNT_NOT_ALLOWLISTED"));
        pairs.push(("P2P_TIER1_ALLOWED_ACCOUNTS", "a, b"));
        assert_eq!(refusal(&pairs), Some("TIER1_HUNT_NOT_ALLOWLISTED"));
        pairs.push(("P2P_TIER1_ALLOWED_HUNTS", "h"));
        assert_eq!(refusal(&pairs), None);
        assert_eq!(
            refusal(&[("P2P_TIER1_REWARDS_ENABLED", "1")]),
            None,
            "tier 0 has no Tier 1 gates"
        );
    }

    #[test]
    fn switches() {
        let config = GuardConfig::default();
        assert!(config.census && config.host_writes && !config.distributed_traffic);
        // hostWrites.test.ts: "P2P_HOST_WRITES=off switches host writes off; anything else keeps them on"
        for (value, on) in [
            ("on", true),
            ("off", false),
            ("0", false),
            (" No ", false),
            ("yes", true),
        ] {
            assert_eq!(
                GuardConfig::from_settings(&settings(&[("P2P_HOST_WRITES", value)])).host_writes,
                on
            );
        }
        // hostRouteCensus.test.ts: "MYSTICPARADOX_HOST_ROUTE_CENSUS: on unless it is off, 0, false or no"
        for (value, on) in [
            ("on", true),
            ("off", false),
            (" OFF ", false),
            ("0", false),
            ("false", false),
            ("no", false),
        ] {
            assert_eq!(
                GuardConfig::from_settings(&settings(&[("MYSTICPARADOX_HOST_ROUTE_CENSUS", value)])).census,
                on
            );
        }
        // guardTrafficDistributed.test.ts: "distributed Guard limiter configuration is literal-true only"
        for (value, on) in [("1", false), (" TRUE ", true), ("true", true)] {
            assert_eq!(
                GuardConfig::from_settings(&settings(&[("P2P_DISTRIBUTED_TRAFFIC_LIMITER", value)]))
                    .distributed_traffic,
                on
            );
        }
    }
}
