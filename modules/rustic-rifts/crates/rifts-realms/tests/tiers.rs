//! `p2p/config/p2pPolicy.test.ts`: the hosting tiers.
#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use rifts_realms::tiers::{
    HostingPolicy, HuntRoute, RouteDecision, RouteInput, RouteReason, decide_hunt_route, is_p2p_training_request,
    relay_only_warning, rollout_bucket,
};

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

fn policy(pairs: &[(&str, &str)]) -> HostingPolicy {
    HostingPolicy::from_settings(&env(pairs))
}

fn base() -> RouteInput {
    RouteInput {
        game_mode: "ISLAND".to_owned(),
        hunt_id: "ShatteredIsles_IslandA".to_owned(),
        account_ids: vec!["player-a".to_owned(), "player-b".to_owned()],
        party_size: 2,
        is_private_party: true,
    }
}

fn with_hunt(hunt: &str) -> RouteInput {
    RouteInput {
        hunt_id: hunt.to_owned(),
        ..base()
    }
}

fn reason(input: &RouteInput, policy: &HostingPolicy) -> RouteReason {
    decide_hunt_route(input, policy, true, false).reason
}

fn route(input: &RouteInput, policy: &HostingPolicy) -> HuntRoute {
    decide_hunt_route(input, policy, true, false).route
}

const ALLOWLISTED: &[(&str, &str)] = &[
    ("P2P_HUNTS_ENABLED", "true"),
    ("P2P_ALLOWED_ACCOUNTS", "player-a,player-b"),
    ("P2P_ALLOWED_HUNTS", "ShatteredIsles_IslandA"),
    ("P2P_PERCENT_ROLLOUT", "100"),
    ("P2P_TIER0_CANARY_ENABLED", "true"),
];

#[test]
fn p2p_policy_fails_closed_when_all_environment_values_are_absent() {
    let p = policy(&[]);
    assert!(!p.enabled);
    assert!(p.allowed_accounts.is_empty());
    assert!(!p.allow_all_accounts);
    assert!(p.allowed_hunts.is_empty());
    assert!(!p.allow_all_resolvable_hunts);
    assert!(!p.allow_high_risk_hunt_classes);
    assert!(p.private_parties_only);
    assert_eq!(p.max_party_size, 4);
    assert_eq!(p.percent_rollout, 0.0);
    assert!(!p.force_sdr);
    assert!(!p.canary_enabled);
    assert_eq!(p.trust_tier, 0);
    assert!(!p.tier1_rewards_enabled);
    assert!(!p.tier1_shadow_approved);
    // No DeployServer here: there is no dedicated route to take
    assert!(p.dedicated_hunts_disabled);
    assert_eq!(
        decide_hunt_route(&base(), &p, false, false),
        RouteDecision {
            route: HuntRoute::Dedicated,
            reason: RouteReason::FeatureDisabled,
            rollout_bucket: None
        }
    );
}

#[test]
fn only_literal_true_enables_the_master_feature_flag() {
    for value in ["1", "yes", "on", "TRUE-ish"] {
        assert!(!policy(&[("P2P_HUNTS_ENABLED", value)]).enabled);
    }
    assert!(policy(&[("P2P_HUNTS_ENABLED", "true")]).enabled);
    assert!(policy(&[("P2P_HUNTS_ENABLED", " TRUE ")]).enabled);
}

#[test]
fn durable_emergency_stop_has_a_distinct_reason_without_mutating_environment_policy() {
    let p = policy(ALLOWLISTED);
    assert!(p.enabled);
    assert_eq!(
        decide_hunt_route(&base(), &p, true, true),
        RouteDecision {
            route: HuntRoute::Dedicated,
            reason: RouteReason::EmergencyStop,
            rollout_bucket: None
        }
    );
    assert_eq!(route(&base(), &p), HuntRoute::PlayerHosted);
}

#[test]
fn enabled_policy_still_requires_explicit_account_and_hunt_allowlists() {
    let empty = policy(&[("P2P_HUNTS_ENABLED", "true"), ("P2P_PERCENT_ROLLOUT", "100")]);
    assert_eq!(reason(&base(), &empty), RouteReason::AccountAllowlistEmpty);
    let account_only = policy(&[
        ("P2P_HUNTS_ENABLED", "true"),
        ("P2P_PERCENT_ROLLOUT", "100"),
        ("P2P_TIER0_CANARY_ENABLED", "true"),
        ("P2P_ALLOWED_ACCOUNTS", "player-a,player-b"),
    ]);
    assert_eq!(reason(&base(), &account_only), RouteReason::HuntAllowlistEmpty);
}

#[test]
fn phase_0_capability_gate_keeps_a_fully_allowlisted_hunt_dedicated() {
    let p = policy(&[
        ("P2P_HUNTS_ENABLED", "true"),
        ("P2P_ALLOWED_ACCOUNTS", "player-a, player-b"),
        ("P2P_ALLOWED_HUNTS", "ShatteredIsles_IslandA"),
        ("P2P_PERCENT_ROLLOUT", "100"),
        ("P2P_TIER0_CANARY_ENABLED", "true"),
        ("P2P_FORCE_SDR", "true"),
    ]);
    let decision = decide_hunt_route(&base(), &p, false, false);
    assert_eq!(decision.route, HuntRoute::Dedicated);
    assert_eq!(decision.reason, RouteReason::PlayerHostedNotImplemented);
    assert!(decision.rollout_bucket.is_some());
}

#[test]
fn future_capability_can_select_player_hosting_only_after_every_gate_passes() {
    let p = policy(ALLOWLISTED);
    assert_eq!(route(&base(), &p), HuntRoute::PlayerHosted);
    let shared = RouteInput {
        game_mode: "SHARED".to_owned(),
        ..base()
    };
    assert_eq!(route(&shared, &p), HuntRoute::PlayerHosted);
    let city = RouteInput {
        game_mode: "CITY".to_owned(),
        ..base()
    };
    assert_eq!(reason(&city, &p), RouteReason::NotAPlayerHostedHunt);
}

#[test]
fn private_party_party_size_account_and_hunt_gates_fail_independently() {
    let mut pairs = ALLOWLISTED.to_vec();
    pairs.push(("P2P_MAX_PARTY_SIZE", "2"));
    let p = policy(&pairs);
    let public = RouteInput {
        is_private_party: false,
        ..base()
    };
    assert_eq!(reason(&public, &p), RouteReason::PrivatePartyRequired);
    let three = RouteInput {
        party_size: 3,
        ..base()
    };
    assert_eq!(reason(&three, &p), RouteReason::PartySizeExceeded);
    let stranger = RouteInput {
        account_ids: vec!["player-a".to_owned(), "stranger".to_owned()],
        ..base()
    };
    assert_eq!(reason(&stranger, &p), RouteReason::AccountNotAllowlisted);
    assert_eq!(reason(&with_hunt("OtherHunt"), &p), RouteReason::HuntNotAllowlisted);
}

#[test]
fn rollout_bucket_is_stable_across_roster_ordering_and_bounded_below_100() {
    let ids = |x: &[&str]| x.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let first = rollout_bucket(&ids(&["player-b", "player-a"]), "hunt");
    let second = rollout_bucket(&ids(&["player-a", "player-b", "player-a"]), "hunt");
    assert_eq!(first, second);
    assert!((0.0..100.0).contains(&first));
}

#[test]
fn invalid_numeric_policy_values_fail_closed() {
    let p = policy(&[("P2P_MAX_PARTY_SIZE", "99"), ("P2P_PERCENT_ROLLOUT", "NaN")]);
    assert_eq!(p.max_party_size, 4);
    assert_eq!(p.percent_rollout, 0.0);
}

#[test]
fn player_hosting_needs_a_separate_explicit_tier_0_canary_switch() {
    let p = policy(&ALLOWLISTED[..4]);
    assert_eq!(reason(&base(), &p), RouteReason::CanaryDisabled);
}

#[test]
fn trust_tier_defaults_to_zero_and_recognizes_only_explicit_tier_one() {
    assert_eq!(policy(&[("P2P_TRUST_TIER", "2")]).trust_tier, 0);
    assert_eq!(policy(&[("P2P_TRUST_TIER", "garbage")]).trust_tier, 0);
    assert_eq!(policy(&[("P2P_TRUST_TIER", "1")]).trust_tier, 1);
}

#[test]
fn competitive_and_high_risk_hunt_classes_stay_dedicated_even_when_allowlisted() {
    let p = policy(&[
        ("P2P_HUNTS_ENABLED", "true"),
        ("P2P_TIER0_CANARY_ENABLED", "true"),
        ("P2P_ALLOWED_ACCOUNTS", "player-a,player-b"),
        ("P2P_ALLOWED_HUNTS", "Trials_Leaderboard"),
        ("P2P_PERCENT_ROLLOUT", "100"),
    ]);
    assert_eq!(
        reason(&with_hunt("Trials_Leaderboard"), &p),
        RouteReason::HuntClassExcluded
    );
}

const ALL_RESOLVABLE: &[(&str, &str)] = &[
    ("P2P_HUNTS_ENABLED", "true"),
    ("P2P_TIER0_CANARY_ENABLED", "true"),
    ("P2P_ALLOWED_ACCOUNTS", "player-a,player-b"),
    ("P2P_ALLOW_ALL_RESOLVABLE_HUNTS", "true"),
    ("P2P_PERCENT_ROLLOUT", "100"),
];

#[test]
fn all_resolvable_rollout_still_requires_explicit_accounts_and_capability_gates() {
    let p = policy(ALL_RESOLVABLE);
    assert!(p.allowed_hunts.is_empty());
    assert_eq!(route(&with_hunt("ShatteredIsles_IslandU"), &p), HuntRoute::PlayerHosted);
    let stranger = RouteInput {
        account_ids: vec!["player-a".to_owned(), "stranger".to_owned()],
        ..base()
    };
    assert_eq!(reason(&stranger, &p), RouteReason::AccountNotAllowlisted);
    assert_eq!(
        decide_hunt_route(&base(), &p, false, false).reason,
        RouteReason::PlayerHostedNotImplemented
    );
}

#[test]
fn automatic_primary_rollout_can_explicitly_admit_all_accounts() {
    let p = policy(&[
        ("P2P_HUNTS_ENABLED", "true"),
        ("P2P_TIER0_CANARY_ENABLED", "true"),
        ("P2P_ALLOW_ALL_ACCOUNTS", "true"),
        ("P2P_ALLOW_ALL_RESOLVABLE_HUNTS", "true"),
        ("P2P_PERCENT_ROLLOUT", "100"),
    ]);
    assert!(p.allow_all_accounts);
    let newcomers = RouteInput {
        account_ids: vec!["new-player".to_owned(), "other-player".to_owned()],
        ..base()
    };
    assert_eq!(route(&newcomers, &p), HuntRoute::PlayerHosted);
}

#[test]
fn dedicated_hunt_disable_rail_recognizes_only_literal_true() {
    let deploy = ("DEPLOYSERVER_URL", "deployserver.invalid:8080");
    assert!(!policy(&[deploy]).dedicated_hunts_disabled);
    for value in ["1", "yes", "on", "TRUE-ish"] {
        assert!(!policy(&[deploy, ("P2P_DISABLE_DEDICATED_HUNTS", value)]).dedicated_hunts_disabled);
    }
    assert!(policy(&[deploy, ("P2P_DISABLE_DEDICATED_HUNTS", " true ")]).dedicated_hunts_disabled);
}

#[test]
fn no_deploy_server_disables_the_dedicated_route_whatever_the_rail_says() {
    for unconfigured in [
        &[][..],
        &[("DEPLOYSERVER_URL", "")][..],
        &[("DEPLOYSERVER_URL", "   ")][..],
    ] {
        assert!(policy(unconfigured).dedicated_hunts_disabled);
        let mut pairs = unconfigured.to_vec();
        pairs.push(("P2P_DISABLE_DEDICATED_HUNTS", "false"));
        assert!(policy(&pairs).dedicated_hunts_disabled);
    }
    assert!(!policy(&[("DEPLOYSERVER_URL", "deployserver.invalid:8080")]).dedicated_hunts_disabled);
}

#[test]
fn high_risk_classes_require_a_second_explicit_switch() {
    let trials = with_hunt("CR19_PlayerHunt_Arena_Hard");
    assert_eq!(reason(&trials, &policy(ALL_RESOLVABLE)), RouteReason::HuntClassExcluded);
    let mut pairs = ALL_RESOLVABLE.to_vec();
    pairs.push(("P2P_ALLOW_HIGH_RISK_HUNT_CLASSES", "true"));
    assert_eq!(route(&trials, &policy(&pairs)), HuntRoute::PlayerHosted);
}

#[test]
fn known_ftue_recursion_stays_excluded_even_during_the_broad_rollout() {
    let mut pairs = ALL_RESOLVABLE.to_vec();
    pairs.push(("P2P_ALLOW_HIGH_RISK_HUNT_CLASSES", "true"));
    let ftue = with_hunt("CR19_PlayerHunt_FTUE_Pursuit_Beta_Beaver");
    assert_eq!(reason(&ftue, &policy(&pairs)), RouteReason::HuntRuntimeIncompatible);
}

#[test]
fn tier_1_requires_separate_reward_shadow_account_and_hunt_gates() {
    let mut pairs = ALLOWLISTED.to_vec();
    pairs.push(("P2P_TRUST_TIER", "1"));
    assert_eq!(reason(&base(), &policy(&pairs)), RouteReason::Tier1RewardsDisabled);
    pairs.push(("P2P_TIER1_REWARDS_ENABLED", "true"));
    assert_eq!(reason(&base(), &policy(&pairs)), RouteReason::Tier1ShadowNotApproved);
    pairs.push(("P2P_TIER1_SHADOW_APPROVED", "true"));
    assert_eq!(
        reason(&base(), &policy(&pairs)),
        RouteReason::Tier1AccountNotAllowlisted
    );
    pairs.push(("P2P_TIER1_ALLOWED_ACCOUNTS", "player-a,player-b"));
    assert_eq!(reason(&base(), &policy(&pairs)), RouteReason::Tier1HuntNotAllowlisted);
    pairs.push(("P2P_TIER1_ALLOWED_HUNTS", "ShatteredIsles_IslandA"));
    assert_eq!(route(&base(), &policy(&pairs)), HuntRoute::PlayerHosted);
}

#[test]
fn training_grounds_stay_dedicated_unless_p2p_training_enabled_is_literally_true() {
    let training = RouteInput {
        game_mode: "SHARED".to_owned(),
        hunt_id: "ShatteredIsles_TrainingDojo".to_owned(),
        ..base()
    };
    assert!(!policy(&[]).training_enabled);
    for value in [None, Some("1"), Some("yes"), Some("on")] {
        let mut pairs = ALL_RESOLVABLE.to_vec();
        if let Some(value) = value {
            pairs.push(("P2P_TRAINING_ENABLED", value));
        }
        let p = policy(&pairs);
        assert!(!p.training_enabled);
        assert_eq!(reason(&training, &p), RouteReason::NotAPlayerHostedHunt);
        assert!(!is_p2p_training_request("SHARED", "ShatteredIsles_TrainingDojo", &p));
    }
    let mut pairs = ALL_RESOLVABLE.to_vec();
    pairs.push(("P2P_TRAINING_ENABLED", "true"));
    let enabled = policy(&pairs);
    assert_eq!(route(&training, &enabled), HuntRoute::PlayerHosted);
    assert!(is_p2p_training_request(
        "SHARED",
        "ShatteredIsles_TrainingDojo",
        &enabled
    ));
    // Only the SHARED request the client sends; Ramsgate never
    let island = RouteInput {
        game_mode: "ISLAND".to_owned(),
        ..training.clone()
    };
    assert_eq!(reason(&island, &enabled), RouteReason::NotAPlayerHostedHunt);
    let ramsgate = RouteInput {
        hunt_id: "ShatteredIsles_ReturnToRamsgate".to_owned(),
        ..training.clone()
    };
    assert_eq!(reason(&ramsgate, &enabled), RouteReason::NotAPlayerHostedHunt);
    assert!(!is_p2p_training_request("SHARED", "ShatteredIsles_IslandA", &enabled));
    // The master switch still wins
    let mut off = ALL_RESOLVABLE.to_vec();
    off.retain(|(k, _)| *k != "P2P_HUNTS_ENABLED");
    off.extend([("P2P_HUNTS_ENABLED", "false"), ("P2P_TRAINING_ENABLED", "true")]);
    assert!(!is_p2p_training_request(
        "SHARED",
        "ShatteredIsles_TrainingDojo",
        &policy(&off)
    ));
    // The hunt allowlist still applies when the broad switch is off
    let mut allowlisted = ALL_RESOLVABLE.to_vec();
    allowlisted.retain(|(k, _)| *k != "P2P_ALLOW_ALL_RESOLVABLE_HUNTS");
    allowlisted.extend([
        ("P2P_ALLOW_ALL_RESOLVABLE_HUNTS", "false"),
        ("P2P_ALLOWED_HUNTS", "ShatteredIsles_IslandA"),
        ("P2P_TRAINING_ENABLED", "true"),
    ]);
    assert_eq!(
        reason(&training, &policy(&allowlisted)),
        RouteReason::HuntNotAllowlisted
    );
}

// relayOnlyWarning.test.ts
#[test]
fn the_relay_only_warning_shows_only_with_hunts_on_and_relay_off() {
    assert_eq!(relay_only_warning(&env(&[])), None);
    assert!(relay_only_warning(&env(&[("P2P_HUNTS_ENABLED", "true")])).is_some());
    assert_eq!(
        relay_only_warning(&env(&[("P2P_HUNTS_ENABLED", "true"), ("P2P_FORCE_SDR", "true")])),
        None
    );
}
