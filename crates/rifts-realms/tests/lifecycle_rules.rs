// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The lifecycle's pure rules.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{admitted, hunt, participant};
use rifts_realms::hunt::{AdmissionState, HuntState};
use rifts_realms::lifecycle::{
    GuardRole, LIFECYCLE_STATES, PartySessionAction, classify_existing_party_session, clean_end_terminal_state,
    host_guard_still_starting, host_loss_is_clean_end, is_cancellable_forming, is_pre_active_operator_stop_state,
    mark_participant_returned_to_hub, parse_guard_presence_sessions, parse_guard_required_roles,
    participants_still_in_hunt, recovery_destination, select_recovery_participants,
    should_use_dedicated_pre_active_fallback,
};
use rifts_seam::{HubKind, UnixMillis};
use serde_json::json;

use HuntState as H;

const GUARD_ID: &str = "0190a1b2-c3d4-4e5f-8a6b-7c8d9e0f1a2b";

#[test]
fn the_scan_set_and_operator_stop_states() {
    assert_eq!(LIFECYCLE_STATES.len(), 10);
    assert!(!LIFECYCLE_STATES.contains(&H::Completed) && LIFECYCLE_STATES.contains(&H::Draining));
    for state in [H::HostSelecting, H::HostOffered, H::Starting, H::HostReady] {
        assert!(is_pre_active_operator_stop_state(state));
    }
    assert!(!is_pre_active_operator_stop_state(H::Admitting));
}

#[test]
fn recovery_participants_and_the_dedicated_rail() {
    let roster = vec![
        participant("host", true),
        admitted(participant("ready", true)),
        admitted(participant("noport", false)),
    ];
    assert_eq!(select_recovery_participants(&roster, true).len(), 3);
    let active: Vec<String> = select_recovery_participants(&roster, false)
        .into_iter()
        .map(|p| p.account_id)
        .collect();
    assert_eq!(active, ["ready"]);
    assert!(should_use_dedicated_pre_active_fallback(true, false));
    assert!(!should_use_dedicated_pre_active_fallback(true, true));
    assert!(!should_use_dedicated_pre_active_fallback(false, false));
}

#[test]
fn guard_sessions_and_roles() {
    assert_eq!(
        parse_guard_presence_sessions(&json!({ "guardSessionId": "legacy" })),
        Ok(vec![(GuardRole::Client, "legacy".to_owned())])
    );
    assert_eq!(parse_guard_presence_sessions(&json!({})), Ok(Vec::new()));
    let both = json!({ "guardSessions": [
        { "role": "client", "guardSessionId": GUARD_ID },
        { "role": "host", "guardSessionId": GUARD_ID },
    ]});
    let sessions = parse_guard_presence_sessions(&both).unwrap();
    assert_eq!(sessions.len(), 2);
    for bad in [
        json!({ "guardSessions": "x" }),
        json!({ "guardSessions": [{ "role": "client", "guardSessionId": "short" }] }),
        json!({ "guardSessions": [
            { "role": "client", "guardSessionId": GUARD_ID },
            { "role": "client", "guardSessionId": GUARD_ID },
        ]}),
    ] {
        assert_eq!(
            parse_guard_presence_sessions(&bad).unwrap_err().message,
            "The Launcher Guard session list is malformed."
        );
    }
    assert_eq!(
        parse_guard_required_roles(&json!({}), &sessions, false),
        Ok(vec![GuardRole::Client, GuardRole::Host])
    );
    let host_only = json!({ "guardRequiredRoles": ["host"] });
    assert_eq!(
        parse_guard_required_roles(&host_only, &sessions, true),
        Ok(vec![GuardRole::Host])
    );
    assert_eq!(
        parse_guard_required_roles(&host_only, &sessions, false)
            .unwrap_err()
            .code,
        "GUARD_REQUIRED"
    );
    assert_eq!(
        parse_guard_required_roles(&json!({ "guardRequiredRoles": ["host", "host"] }), &sessions, false)
            .unwrap_err()
            .message,
        "The Launcher Guard role list is malformed."
    );
}

#[test]
fn the_guard_start_grace() {
    let required = [GuardRole::Client, GuardRole::Host];
    assert!(host_guard_still_starting(
        UnixMillis(0),
        &required,
        &[GuardRole::Client],
        UnixMillis(89_999)
    ));
    assert!(!host_guard_still_starting(
        UnixMillis(0),
        &required,
        &[GuardRole::Client],
        UnixMillis(90_000)
    ));
    assert!(!host_guard_still_starting(
        UnixMillis(0),
        &required,
        &[],
        UnixMillis(10)
    ));
    assert!(!host_guard_still_starting(
        UnixMillis(0),
        &required,
        &required,
        UnixMillis(10)
    ));
}

#[test]
fn clean_ends_and_hub_returns() {
    let mut h = hunt("s");
    h.state = H::Running;
    h.roster = vec![participant("host", true), admitted(participant("p", true))];
    assert_eq!(participants_still_in_hunt(&h.roster, "host"), ["host", "p"]);
    assert_eq!(clean_end_terminal_state(&h), H::Completed);
    h.trust_tier = Some(1);
    assert_eq!(clean_end_terminal_state(&h), H::Aborted);
    assert_eq!(host_loss_is_clean_end(&h), None);
    h.trust_tier = Some(0);
    assert_eq!(host_loss_is_clean_end(&h), None);
    h.guard_encounter_state = Some("COMPLETION_REQUESTED".into());
    assert_eq!(host_loss_is_clean_end(&h), Some("TIER0_HUNT_COMPLETED"));
    h.guard_encounter_state = None;
    let (roster, first, left) = mark_participant_returned_to_hub(&h.roster, "p", UnixMillis(5), "host").unwrap();
    assert!(first);
    assert_eq!(left, ["host"]);
    let (roster, first, _) = mark_participant_returned_to_hub(&roster, "p", UnixMillis(9), "host").unwrap();
    assert!(!first);
    assert_eq!(roster[1].returned_to_hub_at, Some(UnixMillis(5)));
    let (roster, _, left) = mark_participant_returned_to_hub(&roster, "host", UnixMillis(9), "host").unwrap();
    assert!(left.is_empty());
    h.roster = roster;
    assert_eq!(host_loss_is_clean_end(&h), Some("TIER0_PARTY_LEFT"));
    assert!(mark_participant_returned_to_hub(&h.roster, "nobody", UnixMillis(1), "host").is_none());
    let pending = vec![participant("x", false)];
    assert!(mark_participant_returned_to_hub(&pending, "x", UnixMillis(1), "host").is_none());
}

#[test]
fn party_session_classification() {
    let mut h = hunt("s");
    h.roster = vec![participant("host", false), participant("p", false)];
    h.state = H::HostSelecting;
    assert_eq!(
        classify_existing_party_session(&h, "ShatteredIsles_IslandA", "p", UnixMillis(0)),
        PartySessionAction::Reuse
    );
    assert_eq!(
        classify_existing_party_session(&h, "Other", "p", UnixMillis(0)),
        PartySessionAction::Supersede("PARTY_CHANGED_HUNT")
    );
    assert_eq!(
        classify_existing_party_session(&h, "ShatteredIsles_IslandA", "new", UnixMillis(0)),
        PartySessionAction::Supersede("ROSTER_CHANGED")
    );
    h.state = H::Suspect;
    assert_eq!(
        classify_existing_party_session(&h, "ShatteredIsles_IslandA", "p", UnixMillis(0)),
        PartySessionAction::Supersede("SESSION_ENDING")
    );
    h.state = H::Running;
    h.lease_expires_at = Some(UnixMillis(10));
    assert_eq!(
        classify_existing_party_session(&h, "ShatteredIsles_IslandA", "p", UnixMillis(10)),
        PartySessionAction::Supersede("HOST_LEASE_EXPIRED")
    );
    h.lease_expires_at = Some(UnixMillis(100));
    assert_eq!(
        classify_existing_party_session(&h, "ShatteredIsles_IslandA", "new", UnixMillis(10)),
        PartySessionAction::Conflict("ROSTER_FROZEN")
    );
    // A running tier-0 hunt drains instead of being superseded.
    h.roster[1].admission_state = AdmissionState::Admitted;
    assert_eq!(
        classify_existing_party_session(&h, "ShatteredIsles_IslandA", "p", UnixMillis(10)),
        PartySessionAction::Drain("REQUESTER_LEFT_HUNT")
    );
    h.roster[1].admission_state = AdmissionState::Pending;
    assert_eq!(
        classify_existing_party_session(&h, "Other", "p", UnixMillis(10)),
        PartySessionAction::Drain("PARTY_CHANGED_HUNT")
    );
    h.hub_kind = Some(HubKind::Ramsgate);
    assert_eq!(
        classify_existing_party_session(&h, "Other", "p", UnixMillis(10)),
        PartySessionAction::Supersede("PARTY_CHANGED_HUNT")
    );
}

#[test]
fn forming_sessions_and_recovery_destinations() {
    let mut h = hunt("s");
    h.state = H::Admitting;
    h.roster = vec![participant("p", false)];
    assert!(is_cancellable_forming(&h));
    h.roster[0].admission_state = AdmissionState::Admitted;
    assert!(!is_cancellable_forming(&h));
    assert_eq!(
        recovery_destination(false, "Hunt"),
        ("CITY", "ShatteredIsles_ReturnToRamsgate".to_owned(), true)
    );
    assert_eq!(
        recovery_destination(true, "ShatteredIsles_TrainingDojo"),
        ("SHARED", "ShatteredIsles_TrainingDojo".to_owned(), true)
    );
    assert_eq!(recovery_destination(true, "Hunt"), ("ISLAND", "Hunt".to_owned(), false));
}
