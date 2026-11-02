// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Guard's settings, read once from the env file setup writes.

use std::collections::BTreeSet;

use rifts_seam::{AccountId, Settings};

use crate::json::js_string;
use crate::ports::GuardLog;

pub(crate) const HOST_POLICY_KEY: &str = "RUSTIC_RIFTS_HOST_POLICY";
pub(crate) const HUNT_EVIDENCE_KEY: &str = "RUSTIC_RIFTS_HUNT_EVIDENCE";
pub(crate) const CENSUS_KEY: &str = "RUSTIC_RIFTS_HOST_ROUTE_CENSUS";
const TRUST_TIER_KEY: &str = "P2P_TRUST_TIER";
const TIER1_REWARDS_KEY: &str = "P2P_TIER1_REWARDS_ENABLED";
const TIER1_SHADOW_KEY: &str = "P2P_TIER1_SHADOW_APPROVED";
const TIER1_ACCOUNTS_KEY: &str = "P2P_TIER1_ALLOWED_ACCOUNTS";
const TIER1_HUNTS_KEY: &str = "P2P_TIER1_ALLOWED_HUNTS";
const HOST_WRITES_KEY: &str = "P2P_HOST_WRITES";
const DISTRIBUTED_KEY: &str = "P2P_DISTRIBUTED_TRAFFIC_LIMITER";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostPolicyMode {
    #[default]
    Observe,
    Enforce,
}

impl HostPolicyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Enforce => "enforce",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostPolicySetting {
    pub mode: HostPolicyMode,
    /// Only for an empty value.
    pub is_default: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HuntEvidenceSetting {
    pub enabled: bool,
    pub is_default: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrustTier {
    #[default]
    Tier0,
    Tier1,
}

impl TrustTier {
    /// Tier 1 only for exactly `1`, so `2` or `true` stays at tier 0.
    pub fn from_setting(value: Option<&str>) -> Self {
        if value.map(str::trim) == Some("1") {
            Self::Tier1
        } else {
            Self::Tier0
        }
    }

    pub fn number(self) -> u8 {
        match self {
            Self::Tier0 => 0,
            Self::Tier1 => 1,
        }
    }
}

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
    pub census: bool,
    pub host_writes: bool,
    pub distributed_traffic: bool,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self {
            host_policy: HostPolicySetting {
                is_default: true,
                ..HostPolicySetting::default()
            },
            hunt_evidence: resolve_hunt_evidence(None, TrustTier::Tier0),
            trust_tier: TrustTier::Tier0,
            tier1: Tier1Policy::default(),
            census: true,
            host_writes: true,
            distributed_traffic: false,
        }
    }
}

/// Case-insensitive `true` only: `1`, `on` and `yes` are false.
fn strict_true(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
}

/// On unless switched off by name.
fn switch_on(value: Option<&str>) -> bool {
    let value = value.unwrap_or("").trim();
    !["0", "false", "off", "no"]
        .iter()
        .any(|off| value.eq_ignore_ascii_case(off))
}

fn id_set(value: Option<&str>) -> BTreeSet<String> {
    value
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn resolve_host_policy(value: Option<&str>) -> HostPolicySetting {
    let value = value.unwrap_or("").trim().to_lowercase();
    match value.as_str() {
        "" => HostPolicySetting {
            mode: HostPolicyMode::Observe,
            is_default: true,
            warning: None,
        },
        "observe" => HostPolicySetting::default(),
        "enforce" => HostPolicySetting {
            mode: HostPolicyMode::Enforce,
            ..HostPolicySetting::default()
        },
        other => HostPolicySetting {
            mode: HostPolicyMode::Observe,
            is_default: false,
            warning: Some(format!(
                "{HOST_POLICY_KEY} must be observe or enforce (got {}); it is treated as observe, so host policy refuses nothing",
                js_string(other)
            )),
        },
    }
}

pub fn resolve_hunt_evidence(value: Option<&str>, tier: TrustTier) -> HuntEvidenceSetting {
    let tier_default = tier == TrustTier::Tier1;
    let value = value.unwrap_or("").trim().to_lowercase();
    match value.as_str() {
        "" => HuntEvidenceSetting {
            enabled: tier_default,
            is_default: true,
            warning: None,
        },
        "on" | "off" => HuntEvidenceSetting {
            enabled: value == "on",
            is_default: false,
            warning: None,
        },
        other => HuntEvidenceSetting {
            enabled: tier_default,
            is_default: false,
            warning: Some(format!(
                "{HUNT_EVIDENCE_KEY} must be on or off (got {}); the trust tier's default applies ({})",
                js_string(other),
                if tier_default { "on" } else { "off" }
            )),
        },
    }
}

pub fn log_host_policy(setting: &HostPolicySetting, log: &dyn GuardLog) {
    if let Some(warning) = &setting.warning {
        log.warn(&format!("[Startup] WARNING: {warning}"));
    }
    match setting.mode {
        HostPolicyMode::Observe => {
            let source = if setting.is_default {
                " not set (default observe)"
            } else {
                "=observe"
            };
            log.info(&format!(
                "[Startup] {HOST_POLICY_KEY}{source}: host grant classes, the draft token cap and write kinds refuse nothing; would-be refusals are logged as [HostPolicyObserve]"
            ));
        }
        HostPolicyMode::Enforce => log.info(&format!(
            "[Startup] {HOST_POLICY_KEY}=enforce: host grant classes, the draft token cap and write kinds refuse as before"
        )),
    }
}

pub fn log_hunt_evidence(setting: &HuntEvidenceSetting, log: &dyn GuardLog) {
    if let Some(warning) = &setting.warning {
        log.warn(&format!("[Startup] WARNING: {warning}"));
    }
    let source = match (setting.is_default, setting.enabled) {
        (true, _) => " not set (the trust tier's default)",
        (false, true) => "=on",
        (false, false) => "=off",
    };
    if setting.enabled {
        log.info(&format!(
            "[Startup] {HUNT_EVIDENCE_KEY}{source}: hosted hunts send Hunt Guard evidence (the host's transport posts it on its packet thread)"
        ));
    } else {
        log.info(&format!(
            "[Startup] {HUNT_EVIDENCE_KEY}{source}: hosted hunts send no Hunt Guard evidence"
        ));
    }
}

impl GuardConfig {
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let trust_tier = TrustTier::from_setting(settings.get(TRUST_TIER_KEY));
        Self {
            host_policy: resolve_host_policy(settings.get(HOST_POLICY_KEY)),
            hunt_evidence: resolve_hunt_evidence(settings.get(HUNT_EVIDENCE_KEY), trust_tier),
            trust_tier,
            tier1: Tier1Policy {
                rewards_enabled: strict_true(settings.get(TIER1_REWARDS_KEY)),
                shadow_approved: strict_true(settings.get(TIER1_SHADOW_KEY)),
                allowed_accounts: id_set(settings.get(TIER1_ACCOUNTS_KEY)),
                allowed_hunts: id_set(settings.get(TIER1_HUNTS_KEY)),
            },
            census: switch_on(settings.get(CENSUS_KEY)),
            host_writes: switch_on(settings.get(HOST_WRITES_KEY)),
            distributed_traffic: strict_true(settings.get(DISTRIBUTED_KEY)),
        }
    }

    pub fn host_policy_refuses(&self) -> bool {
        self.host_policy.mode == HostPolicyMode::Enforce
    }

    /// Why tier 1 would refuse rewards for these accounts in this hunt; tier 0 never does.
    pub fn tier1_refusal(&self, accounts: &[AccountId], hunt_id: &str) -> Option<&'static str> {
        if self.trust_tier != TrustTier::Tier1 {
            return None;
        }
        if !self.tier1.rewards_enabled {
            Some("TIER1_REWARDS_DISABLED")
        } else if !self.tier1.shadow_approved {
            Some("TIER1_SHADOW_NOT_APPROVED")
        } else if accounts.iter().any(|a| !self.tier1.allowed_accounts.contains(&a.0)) {
            Some("TIER1_ACCOUNT_NOT_ALLOWLISTED")
        } else if !self.tier1.allowed_hunts.contains(hunt_id) {
            Some("TIER1_HUNT_NOT_ALLOWLISTED")
        } else {
            None
        }
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

    #[test]
    fn host_policy_modes_and_warning() {
        assert_eq!(resolve_host_policy(None), resolve_host_policy(Some("  ")));
        assert!(resolve_host_policy(None).is_default);
        let observe = resolve_host_policy(Some(" Observe "));
        assert_eq!(
            (observe.mode, observe.is_default, observe.warning),
            (HostPolicyMode::Observe, false, None)
        );
        assert_eq!(resolve_host_policy(Some("ENFORCE")).mode, HostPolicyMode::Enforce);
        let odd = resolve_host_policy(Some(" Strict "));
        assert_eq!(odd.mode, HostPolicyMode::Observe);
        assert_eq!(
            odd.warning.as_deref(),
            Some(
                "RUSTIC_RIFTS_HOST_POLICY must be observe or enforce (got \"strict\"); it is treated as observe, so host policy refuses nothing"
            )
        );
    }

    #[test]
    fn evidence_follows_the_tier_unless_set() {
        assert!(!resolve_hunt_evidence(None, TrustTier::Tier0).enabled);
        assert!(resolve_hunt_evidence(None, TrustTier::Tier1).enabled);
        assert!(resolve_hunt_evidence(Some("ON"), TrustTier::Tier0).enabled);
        assert!(!resolve_hunt_evidence(Some("off"), TrustTier::Tier1).enabled);
        let odd = resolve_hunt_evidence(Some("yes"), TrustTier::Tier1);
        assert!(odd.enabled && !odd.is_default);
        assert_eq!(
            odd.warning.as_deref(),
            Some("RUSTIC_RIFTS_HUNT_EVIDENCE must be on or off (got \"yes\"); the trust tier's default applies (on)")
        );
        assert!(
            resolve_hunt_evidence(Some("x"), TrustTier::Tier0)
                .warning
                .unwrap()
                .ends_with("(off)")
        );
    }

    #[test]
    fn trust_tier_is_literal_one() {
        for (value, tier) in [
            (None, TrustTier::Tier0),
            (Some(" 1 "), TrustTier::Tier1),
            (Some("2"), TrustTier::Tier0),
            (Some("true"), TrustTier::Tier0),
            (Some("01"), TrustTier::Tier0),
        ] {
            assert_eq!(TrustTier::from_setting(value), tier, "{value:?}");
        }
        assert_eq!(TrustTier::Tier1.number(), 1);
    }

    #[test]
    fn tier1_gates_in_order() {
        let accounts = [AccountId("a".into())];
        let mut config = GuardConfig::from_settings(&settings(&[("P2P_TRUST_TIER", "1")]));
        assert_eq!(config.tier1_refusal(&accounts, "h"), Some("TIER1_REWARDS_DISABLED"));
        config.tier1.rewards_enabled = true;
        assert_eq!(config.tier1_refusal(&accounts, "h"), Some("TIER1_SHADOW_NOT_APPROVED"));
        config.tier1.shadow_approved = true;
        assert_eq!(
            config.tier1_refusal(&accounts, "h"),
            Some("TIER1_ACCOUNT_NOT_ALLOWLISTED")
        );
        config.tier1.allowed_accounts.insert("a".into());
        assert_eq!(config.tier1_refusal(&accounts, "h"), Some("TIER1_HUNT_NOT_ALLOWLISTED"));
        config.tier1.allowed_hunts.insert("h".into());
        assert_eq!(config.tier1_refusal(&accounts, "h"), None);
        assert_eq!(GuardConfig::default().tier1_refusal(&accounts, "h"), None);
    }

    #[test]
    fn switches() {
        let config = GuardConfig::from_settings(&settings(&[
            ("P2P_HOST_WRITES", " OFF "),
            ("RUSTIC_RIFTS_HOST_ROUTE_CENSUS", "No"),
            ("P2P_DISTRIBUTED_TRAFFIC_LIMITER", "1"),
            ("P2P_TIER1_REWARDS_ENABLED", " TRUE "),
            ("P2P_TIER1_SHADOW_APPROVED", "yes"),
            ("P2P_TIER1_ALLOWED_ACCOUNTS", " a, ,b,a "),
        ]));
        assert!(!config.host_writes && !config.census && !config.distributed_traffic);
        assert!(config.tier1.rewards_enabled && !config.tier1.shadow_approved);
        assert_eq!(config.tier1.allowed_accounts.iter().collect::<Vec<_>>(), ["a", "b"]);
        let on = GuardConfig::from_settings(&settings(&[
            ("P2P_HOST_WRITES", "maybe"),
            ("P2P_DISTRIBUTED_TRAFFIC_LIMITER", "True"),
        ]));
        assert!(on.host_writes && on.census && on.distributed_traffic);
        assert_eq!(GuardConfig::from_settings(&settings(&[])), GuardConfig::default());
        assert!(!GuardConfig::default().host_policy_refuses());
    }

    #[test]
    fn startup_lines() {
        let log = MemoryLog::default();
        let config = GuardConfig::from_settings(&settings(&[
            ("RUSTIC_RIFTS_HOST_POLICY", "loose"),
            ("RUSTIC_RIFTS_HUNT_EVIDENCE", "maybe"),
        ]));
        log_host_policy(&config.host_policy, &log);
        log_hunt_evidence(&config.hunt_evidence, &log);
        assert_eq!(
            log.lines(),
            [
                "warn [Startup] WARNING: RUSTIC_RIFTS_HOST_POLICY must be observe or enforce (got \"loose\"); it is treated as observe, so host policy refuses nothing",
                "info [Startup] RUSTIC_RIFTS_HOST_POLICY=observe: host grant classes, the draft token cap and write kinds refuse nothing; would-be refusals are logged as [HostPolicyObserve]",
                "warn [Startup] WARNING: RUSTIC_RIFTS_HUNT_EVIDENCE must be on or off (got \"maybe\"); the trust tier's default applies (off)",
                "info [Startup] RUSTIC_RIFTS_HUNT_EVIDENCE=off: hosted hunts send no Hunt Guard evidence",
            ]
        );
        let log = MemoryLog::default();
        let config = GuardConfig::from_settings(&settings(&[("P2P_TRUST_TIER", "1")]));
        log_host_policy(&config.host_policy, &log);
        log_hunt_evidence(&config.hunt_evidence, &log);
        assert_eq!(
            log.lines(),
            [
                "info [Startup] RUSTIC_RIFTS_HOST_POLICY not set (default observe): host grant classes, the draft token cap and write kinds refuse nothing; would-be refusals are logged as [HostPolicyObserve]",
                "info [Startup] RUSTIC_RIFTS_HUNT_EVIDENCE not set (the trust tier's default): hosted hunts send Hunt Guard evidence (the host's transport posts it on its packet thread)",
            ]
        );
        let log = MemoryLog::default();
        let config = GuardConfig::from_settings(&settings(&[
            ("RUSTIC_RIFTS_HOST_POLICY", "enforce"),
            ("RUSTIC_RIFTS_HUNT_EVIDENCE", "on"),
        ]));
        log_host_policy(&config.host_policy, &log);
        log_hunt_evidence(&config.hunt_evidence, &log);
        assert_eq!(
            log.lines(),
            [
                "info [Startup] RUSTIC_RIFTS_HOST_POLICY=enforce: host grant classes, the draft token cap and write kinds refuse as before",
                "info [Startup] RUSTIC_RIFTS_HUNT_EVIDENCE=on: hosted hunts send Hunt Guard evidence (the host's transport posts it on its packet thread)",
            ]
        );
    }
}
