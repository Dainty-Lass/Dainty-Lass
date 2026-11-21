// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Guard's settings, read once from the env file setup writes.

use std::collections::BTreeSet;

use rifts_seam::{AccountId, Settings};

use crate::{json::js_string, ports::GuardLog};

pub(crate) const HOST_POLICY_KEY: &str = "RUSTIC_RIFTS_HOST_POLICY";
pub(crate) const HUNT_EVIDENCE_KEY: &str = "RUSTIC_RIFTS_HUNT_EVIDENCE";
pub(crate) const CENSUS_KEY: &str = "RUSTIC_RIFTS_HOST_ROUTE_CENSUS";

/// The settings Guard shares with the module's P2P section, under their original names.
mod p2p {
    pub const TRUST_TIER: &str = "P2P_TRUST_TIER";
    pub const TIER1_REWARDS: &str = "P2P_TIER1_REWARDS_ENABLED";
    pub const TIER1_SHADOW: &str = "P2P_TIER1_SHADOW_APPROVED";
    pub const TIER1_ACCOUNTS: &str = "P2P_TIER1_ALLOWED_ACCOUNTS";
    pub const TIER1_HUNTS: &str = "P2P_TIER1_ALLOWED_HUNTS";
    pub const HOST_WRITES: &str = "P2P_HOST_WRITES";
    pub const DISTRIBUTED: &str = "P2P_DISTRIBUTED_TRAFFIC_LIMITER";
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HostPolicyMode {
    #[default]
    Observe,
    Enforce,
}

impl HostPolicyMode {
    pub fn as_str(self) -> &'static str {
        if self == Self::Enforce { "enforce" } else { "observe" }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HostPolicySetting {
    pub mode: HostPolicyMode,
    /// Only for an empty value.
    pub is_default: bool,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HuntEvidenceSetting {
    pub enabled: bool,
    pub is_default: bool,
    pub warning: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TrustTier {
    #[default]
    Tier0,
    Tier1,
}

impl TrustTier {
    /// Tier 1 only for exactly `1`, so `2` or `true` stays at tier 0.
    pub fn from_setting(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("1") => Self::Tier1,
            _ => Self::Tier0,
        }
    }

    pub fn number(self) -> u8 {
        u8::from(self == Self::Tier1)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Tier1Policy {
    pub rewards_enabled: bool,
    pub shadow_approved: bool,
    pub allowed_accounts: BTreeSet<String>,
    pub allowed_hunts: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardConfig {
    pub host_policy: HostPolicySetting,
    pub hunt_evidence: HuntEvidenceSetting,
    pub trust_tier: TrustTier,
    pub tier1: Tier1Policy,
    pub census: bool,
    pub host_writes: bool,
    pub distributed_traffic: bool,
}

/// What an env file with none of Guard's settings gives.
impl Default for GuardConfig {
    fn default() -> Self {
        Self::from_settings(&std::collections::BTreeMap::new())
    }
}

/// Case-insensitive `true` only: `1`, `on` and `yes` are false.
fn strict_true(value: Option<&str>) -> bool {
    matches!(value.map(str::trim), Some(text) if text.eq_ignore_ascii_case("true"))
}

/// On unless switched off by name.
fn switch_on(value: Option<&str>) -> bool {
    let text = value.map_or("", str::trim);
    !["0", "false", "off", "no"]
        .into_iter()
        .any(|word| word.eq_ignore_ascii_case(text))
}

/// A comma list of ids; blanks dropped.
fn id_set(value: Option<&str>) -> BTreeSet<String> {
    let items = value.into_iter().flat_map(|text| text.split(','));
    items
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(String::from)
        .collect()
}

/// The setting trimmed and lower-cased; absent reads as empty.
fn word_of(value: Option<&str>) -> String {
    value.map_or_else(String::new, |text| text.trim().to_lowercase())
}

pub fn resolve_host_policy(value: Option<&str>) -> HostPolicySetting {
    let word = word_of(value);
    let mut setting = HostPolicySetting::default();
    match word.as_str() {
        "observe" => {}
        "enforce" => setting.mode = HostPolicyMode::Enforce,
        "" => setting.is_default = true,
        other => {
            let got = js_string(other);
            setting.warning = Some(format!(
                "{HOST_POLICY_KEY} must be observe or enforce (got {got}); it is treated as observe, so host policy \
                 refuses nothing"
            ));
        }
    }
    setting
}

pub fn resolve_hunt_evidence(value: Option<&str>, tier: TrustTier) -> HuntEvidenceSetting {
    let tier_default = tier == TrustTier::Tier1;
    let word = word_of(value);
    let mut setting = HuntEvidenceSetting {
        enabled: tier_default,
        ..HuntEvidenceSetting::default()
    };
    match word.as_str() {
        "on" => setting.enabled = true,
        "off" => setting.enabled = false,
        "" => setting.is_default = true,
        other => {
            let (got, applies) = (js_string(other), if tier_default { "on" } else { "off" });
            setting.warning = Some(format!(
                "{HUNT_EVIDENCE_KEY} must be on or off (got {got}); the trust tier's default applies ({applies})"
            ));
        }
    }
    setting
}

fn warn_startup(warning: Option<&String>, log: &dyn GuardLog) {
    if let Some(warning) = warning {
        log.warn(&format!("[Startup] WARNING: {warning}"));
    }
}

pub fn log_host_policy(setting: &HostPolicySetting, log: &dyn GuardLog) {
    warn_startup(setting.warning.as_ref(), log);
    let (source, effect) = match (setting.mode, setting.is_default) {
        (HostPolicyMode::Enforce, _) => ("=enforce", "refuse as before"),
        (HostPolicyMode::Observe, true) => (
            " not set (default observe)",
            "refuse nothing; would-be refusals are logged as [HostPolicyObserve]",
        ),
        (HostPolicyMode::Observe, false) => (
            "=observe",
            "refuse nothing; would-be refusals are logged as [HostPolicyObserve]",
        ),
    };
    log.info(&format!(
        "[Startup] {HOST_POLICY_KEY}{source}: host grant classes, the draft token cap and write kinds {effect}"
    ));
}

pub fn log_hunt_evidence(setting: &HuntEvidenceSetting, log: &dyn GuardLog) {
    warn_startup(setting.warning.as_ref(), log);
    let source = if setting.is_default {
        " not set (the trust tier's default)"
    } else if setting.enabled {
        "=on"
    } else {
        "=off"
    };
    let effect = if setting.enabled {
        "send Hunt Guard evidence (the host's transport posts it on its packet thread)"
    } else {
        "send no Hunt Guard evidence"
    };
    log.info(&format!("[Startup] {HUNT_EVIDENCE_KEY}{source}: hosted hunts {effect}"));
}

impl GuardConfig {
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let read = |key: &str| settings.get(key);
        let trust_tier = TrustTier::from_setting(read(p2p::TRUST_TIER));
        let tier1 = Tier1Policy {
            rewards_enabled: strict_true(read(p2p::TIER1_REWARDS)),
            shadow_approved: strict_true(read(p2p::TIER1_SHADOW)),
            allowed_accounts: id_set(read(p2p::TIER1_ACCOUNTS)),
            allowed_hunts: id_set(read(p2p::TIER1_HUNTS)),
        };
        Self {
            host_policy: resolve_host_policy(read(HOST_POLICY_KEY)),
            hunt_evidence: resolve_hunt_evidence(read(HUNT_EVIDENCE_KEY), trust_tier),
            census: switch_on(read(CENSUS_KEY)),
            host_writes: switch_on(read(p2p::HOST_WRITES)),
            distributed_traffic: strict_true(read(p2p::DISTRIBUTED)),
            trust_tier,
            tier1,
        }
    }

    pub fn host_policy_refuses(&self) -> bool {
        matches!(self.host_policy.mode, HostPolicyMode::Enforce)
    }

    /// Why tier 1 would refuse rewards for these accounts in this hunt; tier 0 never does.
    pub fn tier1_refusal(&self, accounts: &[AccountId], hunt_id: &str) -> Option<&'static str> {
        if self.trust_tier == TrustTier::Tier0 {
            return None;
        }
        let tier1 = &self.tier1;
        let gates = [
            (tier1.rewards_enabled, "TIER1_REWARDS_DISABLED"),
            (tier1.shadow_approved, "TIER1_SHADOW_NOT_APPROVED"),
            (
                accounts.iter().all(|a| tier1.allowed_accounts.contains(&a.0)),
                "TIER1_ACCOUNT_NOT_ALLOWLISTED",
            ),
            (tier1.allowed_hunts.contains(hunt_id), "TIER1_HUNT_NOT_ALLOWLISTED"),
        ];
        gates.into_iter().find_map(|(passed, code)| (!passed).then_some(code))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::ports::MemoryLog;

    fn settings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|&(key, value)| (key.into(), value.into())).collect()
    }

    const OBSERVING: &str = "host grant classes, the draft token cap and write kinds refuse nothing; would-be \
                             refusals are logged as [HostPolicyObserve]";
    const SENDING: &str = "hosted hunts send Hunt Guard evidence (the host's transport posts it on its packet thread)";

    #[test]
    fn host_policy_modes_and_warning() {
        assert_eq!(resolve_host_policy(None), resolve_host_policy(Some("  ")));
        assert!(resolve_host_policy(None).is_default);
        let observe = resolve_host_policy(Some(" Observe "));
        assert_eq!(observe, HostPolicySetting::default());
        assert_eq!(resolve_host_policy(Some("ENFORCE")).mode, HostPolicyMode::Enforce);
        let odd = resolve_host_policy(Some(" Strict "));
        assert_eq!(odd.mode, HostPolicyMode::Observe);
        let want = "RUSTIC_RIFTS_HOST_POLICY must be observe or enforce (got \"strict\"); it is treated as observe, so \
                    host policy refuses nothing";
        assert_eq!(odd.warning.as_deref(), Some(want));
    }

    #[test]
    fn evidence_follows_the_tier_unless_set() {
        let cases = [
            (None, TrustTier::Tier0, false),
            (None, TrustTier::Tier1, true),
            (Some("ON"), TrustTier::Tier0, true),
            (Some("off"), TrustTier::Tier1, false),
        ];
        for (value, tier, enabled) in cases {
            assert_eq!(
                resolve_hunt_evidence(value, tier).enabled,
                enabled,
                "{value:?} {tier:?}"
            );
        }
        let odd = resolve_hunt_evidence(Some("yes"), TrustTier::Tier1);
        assert!(odd.enabled && !odd.is_default);
        let want = "RUSTIC_RIFTS_HUNT_EVIDENCE must be on or off (got \"yes\"); the trust tier's default applies (on)";
        assert_eq!(odd.warning.as_deref(), Some(want));
        let tier0 = resolve_hunt_evidence(Some("x"), TrustTier::Tier0).warning.unwrap();
        assert!(tier0.ends_with("(off)"));
    }

    #[test]
    fn trust_tier_is_literal_one() {
        let cases = [
            (None, TrustTier::Tier0),
            (Some(" 1 "), TrustTier::Tier1),
            (Some("2"), TrustTier::Tier0),
            (Some("true"), TrustTier::Tier0),
            (Some("01"), TrustTier::Tier0),
        ];
        for (value, tier) in cases {
            assert_eq!(TrustTier::from_setting(value), tier, "{value:?}");
        }
        assert_eq!((TrustTier::Tier0.number(), TrustTier::Tier1.number()), (0, 1));
    }

    #[test]
    fn tier1_gates_in_order() {
        let accounts = [AccountId("a".into())];
        let mut config = GuardConfig::from_settings(&settings(&[("P2P_TRUST_TIER", "1")]));
        type Opener = fn(&mut Tier1Policy);
        let steps: [(&str, Opener); 4] = [
            ("TIER1_REWARDS_DISABLED", |t| t.rewards_enabled = true),
            ("TIER1_SHADOW_NOT_APPROVED", |t| t.shadow_approved = true),
            ("TIER1_ACCOUNT_NOT_ALLOWLISTED", |t| {
                t.allowed_accounts.insert("a".into());
            }),
            ("TIER1_HUNT_NOT_ALLOWLISTED", |t| {
                t.allowed_hunts.insert("h".into());
            }),
        ];
        for (refusal, open) in steps {
            assert_eq!(config.tier1_refusal(&accounts, "h"), Some(refusal));
            open(&mut config.tier1);
        }
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
        assert_eq!(
            (config.host_writes, config.census, config.distributed_traffic),
            (false, false, false)
        );
        assert_eq!(
            (config.tier1.rewards_enabled, config.tier1.shadow_approved),
            (true, false)
        );
        assert_eq!(Vec::from_iter(&config.tier1.allowed_accounts), ["a", "b"]);
        let on = GuardConfig::from_settings(&settings(&[
            ("P2P_HOST_WRITES", "maybe"),
            ("P2P_DISTRIBUTED_TRAFFIC_LIMITER", "True"),
        ]));
        assert_eq!((on.host_writes, on.census, on.distributed_traffic), (true, true, true));
        let empty = GuardConfig::default();
        assert_eq!(GuardConfig::from_settings(&settings(&[])), empty);
        assert!(empty.host_policy.is_default && !empty.host_policy_refuses());
        assert_eq!(
            (empty.census, empty.host_writes, empty.distributed_traffic),
            (true, true, false)
        );
    }

    fn startup(pairs: &[(&str, &str)]) -> Vec<String> {
        let log = MemoryLog::default();
        let config = GuardConfig::from_settings(&settings(pairs));
        log_host_policy(&config.host_policy, &log);
        log_hunt_evidence(&config.hunt_evidence, &log);
        log.lines()
    }

    #[test]
    fn startup_lines() {
        let odd = startup(&[
            ("RUSTIC_RIFTS_HOST_POLICY", "loose"),
            ("RUSTIC_RIFTS_HUNT_EVIDENCE", "maybe"),
        ]);
        assert_eq!(
            odd,
            [
                "warn [Startup] WARNING: RUSTIC_RIFTS_HOST_POLICY must be observe or enforce (got \"loose\"); it is \
                 treated as observe, so host policy refuses nothing"
                    .to_owned(),
                format!("info [Startup] RUSTIC_RIFTS_HOST_POLICY=observe: {OBSERVING}"),
                "warn [Startup] WARNING: RUSTIC_RIFTS_HUNT_EVIDENCE must be on or off (got \"maybe\"); the trust \
                 tier's default applies (off)"
                    .to_owned(),
                "info [Startup] RUSTIC_RIFTS_HUNT_EVIDENCE=off: hosted hunts send no Hunt Guard evidence".to_owned(),
            ]
        );
        assert_eq!(
            startup(&[("P2P_TRUST_TIER", "1")]),
            [
                format!("info [Startup] RUSTIC_RIFTS_HOST_POLICY not set (default observe): {OBSERVING}"),
                format!("info [Startup] RUSTIC_RIFTS_HUNT_EVIDENCE not set (the trust tier's default): {SENDING}"),
            ]
        );
        assert_eq!(
            startup(&[
                ("RUSTIC_RIFTS_HOST_POLICY", "enforce"),
                ("RUSTIC_RIFTS_HUNT_EVIDENCE", "on")
            ]),
            [
                "info [Startup] RUSTIC_RIFTS_HOST_POLICY=enforce: host grant classes, the draft token cap and write \
                 kinds refuse as before"
                    .to_owned(),
                format!("info [Startup] RUSTIC_RIFTS_HUNT_EVIDENCE=on: {SENDING}"),
            ]
        );
    }
}
