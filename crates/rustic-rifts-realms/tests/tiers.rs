// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The hosting policy and hunt routing: every gate fails closed, and each on its own.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::BTreeMap;

use rustic_rifts_realms::tiers::{
    HostingPolicy, HuntRoute, RouteInput, RouteReason, decide_hunt_route, is_known_runtime_incompatible,
    is_p2p_training_request, relay_only_warning, rollout_bucket,
};

fn settings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

/// Every gate open, at 100 % rollout.
fn open() -> HostingPolicy {
    HostingPolicy::from_settings(&settings(&[
        ("P2P_HUNTS_ENABLED", "true"),
        ("P2P_ALLOW_ALL_ACCOUNTS", "true"),
        ("P2P_ALLOW_ALL_RESOLVABLE_HUNTS", "true"),
        ("P2P_TIER0_CANARY_ENABLED", "true"),
        ("P2P_PERCENT_ROLLOUT", "100"),
        ("DEPLOYSERVER_URL", "http://deploy"),
    ]))
}

fn input() -> RouteInput {
    RouteInput {
        game_mode: "ISLAND".into(),
        hunt_id: "Hunt_Ice".into(),
        account_ids: vec!["a".into(), "b".into()],
        party_size: 2,
        is_private_party: true,
    }
}

fn reason(policy: &HostingPolicy, input: &RouteInput) -> RouteReason {
    decide_hunt_route(input, policy, true, false).reason
}

#[test]
fn defaults_fail_closed() {
    let policy = HostingPolicy::from_settings(&settings(&[]));
    assert!(!policy.enabled && policy.private_parties_only && policy.dedicated_hunts_disabled);
    assert_eq!(
        (policy.max_party_size, policy.percent_rollout, policy.trust_tier),
        (4, 0.0, 0)
    );
    assert_eq!(reason(&policy, &input()), RouteReason::FeatureDisabled);
    for value in ["1", "on", "yes"] {
        assert!(!HostingPolicy::from_settings(&settings(&[("P2P_HUNTS_ENABLED", value)])).enabled);
    }
    let parsed = HostingPolicy::from_settings(&settings(&[
        ("P2P_PRIVATE_PARTIES_ONLY", " FALSE "),
        ("P2P_MAX_PARTY_SIZE", "5"),
        ("P2P_PERCENT_ROLLOUT", "150"),
        ("P2P_TRUST_TIER", " 1 "),
        ("P2P_ALLOWED_ACCOUNTS", " a, ,b "),
    ]));
    assert!(!parsed.private_parties_only);
    assert_eq!(
        (parsed.max_party_size, parsed.percent_rollout, parsed.trust_tier),
        (4, 0.0, 1)
    );
    assert_eq!(parsed.allowed_accounts.len(), 2);
}

#[test]
fn every_gate_in_order() {
    let policy = open();
    let decision = decide_hunt_route(&input(), &policy, true, false);
    assert_eq!(
        (decision.route, decision.reason),
        (HuntRoute::PlayerHosted, RouteReason::Eligible)
    );
    assert!(decision.rollout_bucket.is_some());
    assert_eq!(
        decide_hunt_route(&input(), &policy, true, true).reason,
        RouteReason::EmergencyStop
    );
    assert_eq!(
        decide_hunt_route(&input(), &policy, false, false).reason,
        RouteReason::PlayerHostedNotImplemented
    );
    let with = |f: fn(&mut RouteInput)| {
        let mut i = input();
        f(&mut i);
        reason(&policy, &i)
    };
    assert_eq!(with(|i| i.game_mode = "CITY".into()), RouteReason::NotAPlayerHostedHunt);
    assert_eq!(
        with(|i| i.hunt_id = "ShatteredIsles_ReturnToRamsgate".into()),
        RouteReason::NotAPlayerHostedHunt
    );
    assert_eq!(with(|i| i.is_private_party = false), RouteReason::PrivatePartyRequired);
    assert_eq!(with(|i| i.party_size = 5), RouteReason::PartySizeExceeded);
    assert_eq!(with(|i| i.party_size = 0), RouteReason::PartySizeExceeded);
    assert_eq!(
        with(|i| i.hunt_id = "Hunt_FTUE_1".into()),
        RouteReason::HuntRuntimeIncompatible
    );
    assert_eq!(
        with(|i| i.hunt_id = "Hunt_Trials_1".into()),
        RouteReason::HuntClassExcluded
    );
    let mut p = open();
    p.allow_all_accounts = false;
    assert_eq!(reason(&p, &input()), RouteReason::AccountAllowlistEmpty);
    p.allowed_accounts.insert("a".into());
    assert_eq!(reason(&p, &input()), RouteReason::AccountNotAllowlisted);
    let mut p = open();
    p.allow_all_resolvable_hunts = false;
    assert_eq!(reason(&p, &input()), RouteReason::HuntAllowlistEmpty);
    p.allowed_hunts.insert("Other".into());
    assert_eq!(reason(&p, &input()), RouteReason::HuntNotAllowlisted);
    let mut p = open();
    p.allow_high_risk_hunt_classes = true;
    let mut trial = input();
    trial.hunt_id = "Hunt_Trials_1".into();
    assert_eq!(reason(&p, &trial), RouteReason::Eligible);
    p.canary_enabled = false;
    assert_eq!(reason(&p, &input()), RouteReason::CanaryDisabled);
    let mut p = open();
    p.percent_rollout = 0.0;
    assert_eq!(reason(&p, &input()), RouteReason::RolloutDisabled);
    p.percent_rollout = 0.0001;
    let excluded = decide_hunt_route(&input(), &p, true, false);
    assert_eq!(excluded.reason, RouteReason::RolloutExcluded);
    assert!(excluded.rollout_bucket.is_some());
}

#[test]
fn tier_one_needs_its_own_switches() {
    let mut p = open();
    p.trust_tier = 1;
    assert_eq!(reason(&p, &input()), RouteReason::Tier1RewardsDisabled);
    p.tier1_rewards_enabled = true;
    assert_eq!(reason(&p, &input()), RouteReason::Tier1ShadowNotApproved);
    p.tier1_shadow_approved = true;
    assert_eq!(reason(&p, &input()), RouteReason::Tier1AccountNotAllowlisted);
    p.tier1_allowed_accounts = ["a".to_owned(), "b".to_owned()].into();
    assert_eq!(reason(&p, &input()), RouteReason::Tier1HuntNotAllowlisted);
    p.tier1_allowed_hunts.insert("Hunt_Ice".into());
    assert_eq!(reason(&p, &input()), RouteReason::Eligible);
    assert_eq!(
        RouteReason::Tier1HuntNotAllowlisted.as_str(),
        "TIER1_HUNT_NOT_ALLOWLISTED"
    );
}

#[test]
fn training_ftue_and_the_bucket() {
    let mut p = open();
    let mut training = input();
    training.game_mode = "SHARED".into();
    training.hunt_id = "ShatteredIsles_TrainingDojo".into();
    assert_eq!(reason(&p, &training), RouteReason::NotAPlayerHostedHunt);
    assert!(!is_p2p_training_request("SHARED", "ShatteredIsles_TrainingDojo", &p));
    p.training_enabled = true;
    assert_eq!(reason(&p, &training), RouteReason::Eligible);
    assert!(is_p2p_training_request("SHARED", "ShatteredIsles_TrainingDojo", &p));
    assert!(is_known_runtime_incompatible("a_ftue_b") && !is_known_runtime_incompatible("aftue"));
    let bucket = rollout_bucket(&["player-b".into(), "player-a".into()], "hunt");
    assert!((bucket - 99.409_532_197_751_1).abs() < 1e-9, "{bucket}");
    let again = rollout_bucket(&["player-a".into(), "player-b".into(), "player-a".into()], "hunt");
    assert_eq!(bucket, again);
}

#[test]
fn the_dedicated_rail_and_the_relay_warning() {
    let configured = HostingPolicy::from_settings(&settings(&[("DEPLOYSERVER_URL", "x")]));
    assert!(!configured.dedicated_hunts_disabled);
    let disabled = HostingPolicy::from_settings(&settings(&[
        ("DEPLOYSERVER_URL", "x"),
        ("P2P_DISABLE_DEDICATED_HUNTS", "true"),
    ]));
    assert!(disabled.dedicated_hunts_disabled);
    assert!(relay_only_warning(&settings(&[("P2P_HUNTS_ENABLED", "true")])).is_some());
    assert!(relay_only_warning(&settings(&[("P2P_HUNTS_ENABLED", "true"), ("P2P_FORCE_SDR", "TRUE")])).is_none());
}
