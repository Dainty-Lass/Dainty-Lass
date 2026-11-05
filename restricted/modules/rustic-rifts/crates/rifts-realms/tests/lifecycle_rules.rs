//! `p2p/controllers/p2pHostLifecycle.test.ts`: the lifecycle's pure rules. The matchmaker's helpers it also checks
//! (`GetHubGameSessionId`, `PlayerHostedSetupFailureReason`, `DedicatedFallbackAction`) are isles' and left out.
#![allow(clippy::unwrap_used)]

mod common;

use common::{hunt_session, participant};
use rifts_realms::hunt::AdmissionState::{Admitted, Issued, Lost, Pending};
use rifts_realms::hunt::{HuntSession, HuntState};
use rifts_realms::lifecycle::{
    GuardRole, HOST_END_STATES, HOST_GUARD_START_GRACE_MS, LIFECYCLE_STATES, PartySessionAction, SUPERSEDE_STATES,
    classify_existing_party_session, clean_end_terminal_state, host_guard_still_starting, host_loss_is_clean_end,
    is_pre_active_operator_stop_state, mark_participant_returned_to_hub, parse_guard_presence_sessions,
    parse_guard_required_roles, participants_still_in_hunt, recovery_destination, select_recovery_participants,
    should_use_dedicated_pre_active_fallback,
};
use rifts_realms::time::parse_iso;
use rifts_seam::{HubKind, UnixMillis};
use serde_json::json;

use HuntState::*;

const NOW: UnixMillis = UnixMillis(1_792_000_000_000);

fn ids(roster: &[rifts_realms::hunt::Participant]) -> Vec<&str> {
    roster.iter().map(|p| p.account_id.as_str()).collect()
}

#[test]
fn the_lifecycle_scan_includes_host_selection_for_restart_recovery() {
    assert!(LIFECYCLE_STATES.contains(&HostSelecting));
}

#[test]
fn operator_stop_routes_offered_and_starting_hunts_through_pre_active_recovery() {
    for state in [HostSelecting, HostOffered, Starting, HostReady] {
        assert!(is_pre_active_operator_stop_state(state), "{state:?}");
    }
    for state in [Admitting, Running, Suspect, Aborting, Recovering] {
        assert!(!is_pre_active_operator_stop_state(state), "{state:?}");
    }
}

#[test]
fn active_recovery_excludes_a_joiner_that_never_completed_the_route() {
    let roster = [
        participant("host", Admitted, true),
        participant("joiner", Issued, false),
    ];
    assert_eq!(ids(&select_recovery_participants(&roster, false)), ["host"]);
}

#[test]
fn active_recovery_includes_every_admitted_participant_with_a_published_route() {
    let roster = [
        participant("host", Admitted, true),
        participant("joiner", Admitted, true),
    ];
    assert_eq!(ids(&select_recovery_participants(&roster, false)), ["host", "joiner"]);
}

#[test]
fn pre_active_dedicated_fallback_still_includes_the_complete_roster() {
    let roster = [
        participant("host", Pending, false),
        participant("joiner", Pending, false),
    ];
    assert_eq!(ids(&select_recovery_participants(&roster, true)), ["host", "joiner"]);
}

#[test]
fn the_dedicated_hunt_rail_cancels_pre_active_failure_but_keeps_active_recovery() {
    // The policy's dedicated_hunts_disabled covers both the rail and a missing DeployServer (tiers.rs tests)
    assert!(!should_use_dedicated_pre_active_fallback(true, true));
    assert!(!should_use_dedicated_pre_active_fallback(false, true));
    assert!(should_use_dedicated_pre_active_fallback(true, false));
    assert!(!should_use_dedicated_pre_active_fallback(false, false));
}

#[test]
fn launcher_guard_presence_keeps_client_and_host_sessions_independently_bound() {
    let body = json!({
        "guardSessions": [
            { "role": "client", "guardSessionId": "11111111-1111-4111-8111-111111111111" },
            { "role": "host", "guardSessionId": "22222222-2222-4222-8222-222222222222" }
        ],
        "guardRequiredRoles": ["client", "host"]
    });
    let sessions = parse_guard_presence_sessions(&body).unwrap();
    let roles: Vec<GuardRole> = sessions.iter().map(|(role, _)| *role).collect();
    assert_eq!(roles, [GuardRole::Client, GuardRole::Host]);
    assert_eq!(
        parse_guard_required_roles(&body, &sessions, false).unwrap(),
        [GuardRole::Client, GuardRole::Host]
    );
    assert!(parse_guard_required_roles(&json!({ "guardRequiredRoles": ["host"] }), &sessions, false).is_err());
    let twice = json!({ "guardSessions": [
        { "role": "client", "guardSessionId": "11111111-1111-4111-8111-111111111111" },
        { "role": "client", "guardSessionId": "22222222-2222-4222-8222-222222222222" }
    ] });
    assert!(parse_guard_presence_sessions(&twice).is_err());
    // A realm host the server confirmed may stand on its host role alone
    let host_only = json!({ "guardRequiredRoles": ["host"] });
    assert_eq!(
        parse_guard_required_roles(&host_only, &sessions, true).unwrap(),
        [GuardRole::Host]
    );
}

fn session() -> HuntSession {
    let mut s = hunt_session("session-1", Running);
    s.generation = 2;
    s.state_version = 9;
    s.party_revision = 4;
    s.trust_tier = Some(0);
    s.roster = vec![
        participant("host", Admitted, true),
        participant("joiner", Admitted, true),
    ];
    s.lease_expires_at = Some(UnixMillis(NOW.0 + 10_000));
    s.guard_encounter_state = Some("ENCOUNTER_ACTIVE".to_owned());
    s
}

fn with(change: impl FnOnce(&mut HuntSession)) -> HuntSession {
    let mut s = session();
    change(&mut s);
    s
}

fn suspect_after_running(s: &mut HuntSession) {
    s.state = Suspect;
    s.pre_suspect_state = Some(Running);
}

#[test]
fn a_tier_0_session_that_ran_ends_completed_otherwise_aborted() {
    assert_eq!(clean_end_terminal_state(&session()), Completed);
    assert_eq!(clean_end_terminal_state(&with(suspect_after_running)), Completed);
    assert_eq!(clean_end_terminal_state(&with(|s| s.state = Admitting)), Aborted);
    assert_eq!(clean_end_terminal_state(&with(|s| s.state = HostSelecting)), Aborted);
    assert_eq!(clean_end_terminal_state(&with(|s| s.trust_tier = Some(1))), Aborted);
}

#[test]
fn losing_the_host_after_a_guard_completion_or_after_everyone_left_is_a_clean_end() {
    let done = |s: &mut HuntSession| s.guard_encounter_state = Some("COMPLETION_REQUESTED".to_owned());
    assert_eq!(
        host_loss_is_clean_end(&session()),
        None,
        "players still in the hunt: failure path"
    );
    assert_eq!(host_loss_is_clean_end(&with(done)), Some("TIER0_HUNT_COMPLETED"));
    assert_eq!(
        host_loss_is_clean_end(&with(|s| {
            suspect_after_running(s);
            done(s);
        })),
        Some("TIER0_HUNT_COMPLETED")
    );
    let left = with(|s| {
        let mut joiner = participant("joiner", Admitted, true);
        joiner.returned_to_hub_at = parse_iso("2026-10-11T10:30:00.000Z");
        s.roster = vec![participant("host", Lost, true), joiner];
    });
    assert_eq!(host_loss_is_clean_end(&left), Some("TIER0_PARTY_LEFT"));
    // Nobody was ever admitted: a host start failure, not a clean end
    let unstarted = with(|s| {
        s.state = Admitting;
        s.roster = vec![participant("host", Pending, false)];
    });
    assert_eq!(host_loss_is_clean_end(&unstarted), None);
    assert_eq!(
        host_loss_is_clean_end(&with(|s| {
            s.trust_tier = Some(1);
            done(s);
        })),
        None
    );
    assert_eq!(
        host_loss_is_clean_end(&with(|s| {
            s.state = Recovering;
            done(s);
        })),
        None
    );
}

#[test]
fn hub_returns_are_recorded_per_admitted_participant_and_the_last_one_out_empties_the_hunt() {
    let roster = vec![
        participant("host", Admitted, true),
        participant("joiner", Admitted, true),
        participant("never", Issued, false),
    ];
    assert_eq!(participants_still_in_hunt(&roster, None), ["host", "joiner"]);
    let at = |text: &str| parse_iso(text).unwrap();
    let (first, changed, still) =
        mark_participant_returned_to_hub(&roster, "joiner", at("2026-10-11T10:30:00.000Z"), None).unwrap();
    assert!(changed);
    assert_eq!(still, ["host"]);
    assert_eq!(
        first
            .iter()
            .find(|p| p.account_id == "joiner")
            .unwrap()
            .returned_to_hub_at,
        Some(at("2026-10-11T10:30:00.000Z"))
    );
    let (_, repeat_changed, _) =
        mark_participant_returned_to_hub(&first, "joiner", at("2026-10-11T10:31:00.000Z"), None).unwrap();
    assert!(!repeat_changed);
    let (_, _, last) = mark_participant_returned_to_hub(&first, "host", at("2026-10-11T10:32:00.000Z"), None).unwrap();
    assert!(last.is_empty());
    // A participant that never joined, or an outsider, records nothing
    assert!(mark_participant_returned_to_hub(&roster, "never", at("2026-10-11T10:30:00.000Z"), None).is_none());
    assert!(mark_participant_returned_to_hub(&roster, "outsider", at("2026-10-11T10:30:00.000Z"), None).is_none());
}

#[test]
fn the_host_stays_in_its_own_hunt_until_it_leaves() {
    let roster = vec![
        participant("host", Pending, false),
        participant("joiner", Admitted, true),
    ];
    let at = |text: &str| parse_iso(text).unwrap();
    assert_eq!(participants_still_in_hunt(&roster, Some("host")), ["host", "joiner"]);
    let (joiner_left, _, still) =
        mark_participant_returned_to_hub(&roster, "joiner", at("2026-10-11T10:30:00.000Z"), Some("host")).unwrap();
    assert_eq!(still, ["host"], "the joiner leaving must not end the host's hunt");
    let (host_left, _, empty) =
        mark_participant_returned_to_hub(&joiner_left, "host", at("2026-10-11T10:31:00.000Z"), Some("host")).unwrap();
    assert!(empty.is_empty());
    assert_eq!(host_loss_is_clean_end(&with(|s| s.roster = joiner_left.clone())), None);
    assert_eq!(
        host_loss_is_clean_end(&with(|s| s.roster = host_left.clone())),
        Some("TIER0_PARTY_LEFT")
    );
    assert!(participants_still_in_hunt(&[participant("host", Lost, true)], Some("host")).is_empty());
}

fn classify(s: &HuntSession, hunt: &str, requester: &str) -> PartySessionAction {
    classify_existing_party_session(s, hunt, requester, NOW)
}

#[test]
fn a_partys_new_request_reuses_only_a_forming_session_and_supersedes_finished_or_ending_ones() {
    use PartySessionAction::{Conflict, Drain, Reuse, Supersede};
    let forming = with(|s| {
        s.state = HostOffered;
        s.lease_expires_at = None;
        s.roster = vec![
            participant("host", Pending, false),
            participant("joiner", Pending, false),
        ];
    });
    assert_eq!(classify(&forming, "ShatteredIsles_IslandA", "joiner"), Reuse);
    assert_eq!(
        classify(&forming, "ShatteredIsles_IslandB", "joiner"),
        Supersede("PARTY_CHANGED_HUNT")
    );
    assert_eq!(
        classify(&forming, "ShatteredIsles_IslandA", "newcomer"),
        Supersede("ROSTER_CHANGED")
    );
    let finished = with(|s| s.guard_encounter_state = Some("COMPLETION_REQUESTED".to_owned()));
    assert!(matches!(
        classify(&finished, "ShatteredIsles_IslandA", "host"),
        Drain("HUNT_FINISHED") | Supersede("HUNT_FINISHED")
    ));
    assert_eq!(
        classify(&with(suspect_after_running), "ShatteredIsles_IslandA", "host"),
        Supersede("SESSION_ENDING")
    );
    assert_eq!(
        classify(&with(|s| s.state = Aborting), "ShatteredIsles_IslandB", "host"),
        Supersede("SESSION_ENDING")
    );
    let expired = with(|s| s.lease_expires_at = Some(UnixMillis(NOW.0 - 1_000)));
    assert_eq!(
        classify(&expired, "ShatteredIsles_IslandA", "host"),
        Supersede("HOST_LEASE_EXPIRED")
    );
    // A running Tier 0 hunt the requester left is drained, not fenced
    assert_eq!(
        classify(&session(), "ShatteredIsles_IslandA", "joiner"),
        Drain("REQUESTER_LEFT_HUNT")
    );
    // A member who joined after the roster froze cannot end the others' running hunt
    assert_eq!(
        classify(&session(), "ShatteredIsles_IslandA", "newcomer"),
        Conflict("ROSTER_FROZEN")
    );
}

#[test]
fn a_running_tier_0_hunt_the_requester_left_is_drained_and_every_other_superseded_session_ends() {
    use PartySessionAction::{Drain, Supersede};
    // The host of a running hunt asking for one again has left its own server too
    assert_eq!(
        classify(&session(), "ShatteredIsles_IslandA", "host"),
        Drain("REQUESTER_LEFT_HUNT")
    );
    // A Tier 1 hunt is never drained: its completion belongs to Guard
    assert_eq!(
        classify(&with(|s| s.trust_tier = Some(1)), "ShatteredIsles_IslandA", "joiner"),
        Supersede("REQUESTER_LEFT_HUNT")
    );
    // A hunt still starting is superseded
    for state in [Admitting, HostReady] {
        assert_eq!(
            classify(&with(|s| s.state = state), "ShatteredIsles_IslandA", "joiner"),
            Supersede("REQUESTER_LEFT_HUNT"),
            "{state:?}"
        );
    }
    // A realm hub is never drained
    assert_eq!(
        classify(
            &with(|s| s.hub_kind = Some(HubKind::Ramsgate)),
            "ShatteredIsles_IslandA",
            "joiner"
        ),
        Supersede("REQUESTER_LEFT_HUNT")
    );
    let finished = |s: &mut HuntSession| s.guard_encounter_state = Some("COMPLETION_REQUESTED".to_owned());
    assert_eq!(
        classify(&with(finished), "ShatteredIsles_IslandA", "joiner"),
        Drain("HUNT_FINISHED")
    );
    assert_eq!(
        classify(
            &with(|s| {
                finished(s);
                s.trust_tier = Some(1);
            }),
            "ShatteredIsles_IslandA",
            "joiner"
        ),
        Supersede("HUNT_FINISHED")
    );
    // A party that changed hunt while its running hunt still has players drains it as well
    assert_eq!(
        classify(&session(), "ShatteredIsles_IslandB", "joiner2"),
        Drain("PARTY_CHANGED_HUNT")
    );
    assert_eq!(
        classify(&with(suspect_after_running), "ShatteredIsles_IslandA", "joiner"),
        Supersede("SESSION_ENDING")
    );
    let expired = with(|s| s.lease_expires_at = Some(UnixMillis(NOW.0 - 1_000)));
    assert_eq!(
        classify(&expired, "ShatteredIsles_IslandA", "joiner"),
        Supersede("HOST_LEASE_EXPIRED")
    );
}

#[test]
fn a_draining_hunt_is_scanned_ends_completed_and_is_no_supersede_or_host_end_state() {
    assert!(LIFECYCLE_STATES.contains(&Draining));
    assert_eq!(clean_end_terminal_state(&with(|s| s.state = Draining)), Completed);
    assert_eq!(
        clean_end_terminal_state(&with(|s| {
            s.state = Draining;
            s.trust_tier = Some(1);
        })),
        Aborted
    );
    assert!(
        !SUPERSEDE_STATES.contains(&Draining),
        "a new request never supersedes a drain again"
    );
    assert!(
        !HOST_END_STATES.contains(&Draining),
        "the host's end is handled by its own drain branch"
    );
}

#[test]
fn recovery_goes_to_ramsgate_once_active_and_to_the_training_hub_for_unstarted_training() {
    assert_eq!(
        recovery_destination(false, "ShatteredIsles_IslandA"),
        ("CITY", "ShatteredIsles_ReturnToRamsgate".to_owned(), true)
    );
    assert_eq!(
        recovery_destination(true, "ShatteredIsles_IslandA"),
        ("ISLAND", "ShatteredIsles_IslandA".to_owned(), false)
    );
    assert_eq!(
        recovery_destination(true, "ShatteredIsles_TrainingDojo"),
        ("SHARED", "ShatteredIsles_TrainingDojo".to_owned(), true)
    );
}

#[test]
fn a_just_started_host_whose_first_guard_verdict_is_pending_is_not_recovered() {
    let created = parse_iso("2026-10-13T13:15:38.762Z").unwrap();
    let at = |ms: i64| UnixMillis(created.0 + ms);
    let both = [GuardRole::Client, GuardRole::Host];
    let client = [GuardRole::Client];
    assert!(host_guard_still_starting(created, &both, &client, at(15_000)));
    assert!(!host_guard_still_starting(
        created,
        &both,
        &client,
        at(HOST_GUARD_START_GRACE_MS)
    ));
    // An unverified client is never excused, nor is a host role reported with no client verdict
    assert!(!host_guard_still_starting(created, &both, &[], at(15_000)));
    assert!(!host_guard_still_starting(created, &client, &[], at(15_000)));
    // Nothing missing, or a clock before the creation: no grace
    assert!(!host_guard_still_starting(created, &both, &both, at(15_000)));
    assert!(!host_guard_still_starting(created, &both, &client, at(-1_000)));
}
