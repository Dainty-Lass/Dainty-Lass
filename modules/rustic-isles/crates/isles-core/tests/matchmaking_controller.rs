//! `controllers/matchmaking.ts` and `controllers/candidateStatus.ts` ported: the Node tests `matchmakingLeave.test.ts`,
//! `matchmakingNoDeployServer.test.ts`, `matchmakingHuntPolicy.test.ts` (its dedicated-route half) and
//! `candidateStatus.test.ts`. Node mocks `fetch` and `Date.now`; here the Director is a scripted fake and the clock is
//! the state's.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::Arc;

use isles_core::areas::matchmaking::controller::{
    JoinRequest, LeaveOutcome, LeaveParty, LeavePartyLookup, MatchResult, Matchmaker, NO_DEDICATED_SERVERS_REASON,
};
use isles_core::areas::matchmaking::deploy::{DedicatedServers, Unconfigured};
use isles_core::areas::matchmaking::state::{MatchmakingResult, Source};
use isles_core::areas::matchmaking::status::{REALM_FULL_RETRY_AFTER_SECONDS, candidate_status_answer};
use isles_core::areas::matchmaking::testkit::{FakeDeploy, shared};
use isles_core::http::Shared;
use serde_json::{Value, json};

const USER: &str = "leader-account-0001";
const MEMBER_B: &str = "member-account-0002";
const MEMBER_C: &str = "member-account-0003";
const HUB_USER: &str = "hub-account-0001";
const ISLAND: &str = "ShatteredIsles_IslandA";
const RAMSGATE: &str = "ShatteredIsles_ReturnToRamsgate";
const TRAINING: &str = "ShatteredIsles_TrainingDojo";
const PARTY_ID: &str = "party-cancel-0001";

fn module(deploy: Arc<dyn DedicatedServers>) -> Shared {
    shared(&[], deploy)
}

fn join(mode: &str, hunt: &str, player: &str) -> JoinRequest {
    JoinRequest {
        game_mode: Some(mode.to_owned()),
        game_args: Some(String::new()),
        hunt_id: Some(hunt.to_owned()),
        player: player.to_owned(),
        ..JoinRequest::default()
    }
}

fn party_join(hunt: &str, player: &str, party: &str, members: &[&str]) -> JoinRequest {
    JoinRequest {
        party_id: Some(party.to_owned()),
        party_members: Some(members.iter().map(|m| (*m).to_owned()).collect()),
        party_revision: Some(1),
        build_id: Some("build".to_owned()),
        ..join("ISLAND", hunt, player)
    }
}

fn matchmaker(shared: &Shared) -> Matchmaker<'_> {
    Matchmaker {
        shared,
        request_id: None,
    }
}

/// GET /candidate/status: the body the route answers for this account (`None`: no candidate, the route answers empty).
async fn status_body(shared: &Shared, user: &str) -> Option<Value> {
    let result = matchmaker(shared).check_and_update_queue_status(user).await.unwrap();
    candidate_status_answer(shared, user, result.as_ref(), None).await.body
}

fn solo<'a>() -> LeavePartyLookup<'a> {
    &|_| Box::pin(async { MatchResult::Ok(None) })
}

fn party_of_all<'a>() -> LeavePartyLookup<'a> {
    &|_| {
        Box::pin(async {
            MatchResult::Ok(Some(LeaveParty {
                party_id: PARTY_ID.to_owned(),
                members: vec![USER.to_owned(), MEMBER_B.to_owned(), MEMBER_C.to_owned()],
            }))
        })
    }
}

#[tokio::test]
async fn a_solo_search_cancelled_before_its_travel_is_canceled_a_second_press_is_a_no_op_and_a_re_join_searches_again()
{
    let deploy = FakeDeploy::online("203.0.113.10", 7777);
    let shared = module(deploy.clone());
    let mm = matchmaker(&shared);
    assert!(
        mm.handle_player_matchmaking(join("ISLAND", ISLAND, USER))
            .await
            .unwrap()
    );
    let matching = status_body(&shared, USER).await.unwrap();
    assert_eq!(matching["status"], "MATCHING");
    let candidate = matching["candidateId"].as_str().unwrap().to_owned();

    let first = mm.cancel_matchmaking_for_leave(USER, solo()).await.unwrap();
    assert_eq!(
        (first.outcome, first.state.as_str(), first.candidate_id.as_deref()),
        (LeaveOutcome::Canceled, "go-now", Some(candidate.as_str()))
    );
    let canceled = status_body(&shared, USER).await.unwrap();
    assert_eq!(canceled["status"], "CANCELED");
    assert_eq!(canceled["statusReason"], "PLAYER_CANCELED");
    assert_eq!(
        canceled["candidateId"],
        candidate.as_str(),
        "the candidate keeps its id"
    );

    let second = mm.cancel_matchmaking_for_leave(USER, solo()).await.unwrap();
    assert_eq!(
        (second.outcome, second.state.as_str(), second.candidate_id.as_deref()),
        (LeaveOutcome::AlreadyCanceled, "canceled", Some(candidate.as_str()))
    );
    assert_eq!(
        status_body(&shared, USER).await.unwrap()["statusReason"],
        "PLAYER_CANCELED"
    );

    assert!(
        mm.handle_player_matchmaking(join("ISLAND", ISLAND, USER))
            .await
            .unwrap(),
        "a join after the cancel is accepted"
    );
    assert_eq!(status_body(&shared, USER).await.unwrap()["status"], "MATCHING");
    assert_eq!(deploy.launch_count(), 2, "the re-join launches a fresh hunt");
}

#[tokio::test]
async fn a_partys_forming_hunt_is_cancelled_for_every_member() {
    let deploy = FakeDeploy::online("203.0.113.10", 7777);
    let shared = module(deploy.clone());
    let mm = matchmaker(&shared);
    let members = [USER, MEMBER_B, MEMBER_C];
    for member in members {
        assert!(
            mm.handle_player_matchmaking(party_join(ISLAND, member, PARTY_ID, &members))
                .await
                .unwrap()
        );
    }
    assert_eq!(deploy.launch_count(), 1, "the party shares one hunt server");
    assert_eq!(status_body(&shared, MEMBER_B).await.unwrap()["status"], "MATCHING");

    let candidate = status_body(&shared, USER).await.unwrap()["candidateId"].clone();
    let answer = mm.cancel_matchmaking_for_leave(USER, party_of_all()).await.unwrap();
    assert_eq!(
        (answer.outcome, answer.state.as_str()),
        (LeaveOutcome::Canceled, "go-now")
    );
    assert_eq!(answer.candidate_id.as_deref(), candidate.as_str());
    assert_eq!(
        status_body(&shared, USER).await.unwrap()["statusReason"],
        "PLAYER_CANCELED"
    );
    for member in [MEMBER_B, MEMBER_C] {
        let body = status_body(&shared, member).await.unwrap();
        assert_eq!(body["status"], "CANCELED");
        assert_eq!(body["statusReason"], "PARTY_MEMBER_CANCELED");
    }
    assert_eq!(
        mm.party_instance(PARTY_ID, None).await.unwrap(),
        None,
        "the party no longer shows a hunt"
    );

    // The leader searches again: a new shared instance, and the members who were cancelled follow it in
    assert!(
        mm.handle_player_matchmaking(party_join(ISLAND, USER, PARTY_ID, &members))
            .await
            .unwrap()
    );
    assert_eq!(deploy.launch_count(), 2, "a fresh hunt server, not the cancelled one");
    assert_eq!(status_body(&shared, USER).await.unwrap()["status"], "MATCHING");
    assert!(
        mm.handle_player_matchmaking(party_join(ISLAND, MEMBER_B, PARTY_ID, &members))
            .await
            .unwrap()
    );
    assert_eq!(
        status_body(&shared, MEMBER_B).await.unwrap()["status"],
        "MATCHING",
        "a follower of the new hunt is not still cancelled"
    );
    assert_eq!(deploy.launch_count(), 2);
}

#[tokio::test]
async fn a_search_already_told_to_travel_is_left_alone() {
    let shared = module(FakeDeploy::online("203.0.113.10", 7777));
    let mm = matchmaker(&shared);
    assert!(
        mm.handle_player_matchmaking(join("ISLAND", ISLAND, USER))
            .await
            .unwrap()
    );
    shared.matchmaking.clock.advance(60_000);
    assert_eq!(status_body(&shared, USER).await.unwrap()["status"], "IN_PROGRESS");

    let outcome = mm.cancel_matchmaking_for_leave(USER, solo()).await.unwrap();
    assert_eq!(
        (outcome.outcome, outcome.state.as_str()),
        (LeaveOutcome::TooLate, "ready")
    );
    assert_eq!(status_body(&shared, USER).await.unwrap()["status"], "IN_PROGRESS");
}

#[tokio::test]
async fn a_hub_destination_is_never_cancelled() {
    let deploy = FakeDeploy::online("203.0.113.10", 7777);
    deploy.set_ramsgate(true, "203.0.113.10", 7777);
    let shared = module(deploy);
    let mm = matchmaker(&shared);
    assert!(mm.handle_player_matchmaking(join("CITY", "", HUB_USER)).await.unwrap());
    let matching = status_body(&shared, HUB_USER).await.unwrap();
    assert_eq!(matching["status"], "MATCHING");

    let outcome = mm.cancel_matchmaking_for_leave(HUB_USER, solo()).await.unwrap();
    assert_eq!(
        (outcome.outcome, outcome.state.as_str(), outcome.candidate_id.as_deref()),
        (LeaveOutcome::HubNotCancelled, "hub", matching["candidateId"].as_str())
    );
    assert_eq!(status_body(&shared, HUB_USER).await.unwrap()["status"], "MATCHING");

    shared.matchmaking.clock.advance(60_000);
    assert_eq!(status_body(&shared, HUB_USER).await.unwrap()["status"], "IN_PROGRESS");
    assert_eq!(
        mm.cancel_matchmaking_for_leave(HUB_USER, solo()).await.unwrap().outcome,
        LeaveOutcome::HubNotCancelled
    );
    assert_eq!(
        status_body(&shared, HUB_USER).await.unwrap()["status"],
        "IN_PROGRESS",
        "still travelling to Ramsgate"
    );
}

#[tokio::test]
async fn a_queued_search_leaves_its_queue_on_cancel_and_a_join_after_it_queues_again() {
    let deploy = FakeDeploy::online("203.0.113.10", 7777);
    let shared = module(deploy.clone());
    let mm = matchmaker(&shared);
    let queued = "CR19_PlayerHunt_Cancel_Test";
    assert!(
        mm.handle_player_matchmaking(join("ISLAND", queued, USER))
            .await
            .unwrap()
    );
    assert_eq!(status_body(&shared, USER).await.unwrap()["status"], "MATCHING");

    let outcome = mm.cancel_matchmaking_for_leave(USER, solo()).await.unwrap();
    assert_eq!(
        (outcome.outcome, outcome.state.as_str()),
        (LeaveOutcome::Canceled, "queued")
    );
    assert_eq!(
        status_body(&shared, USER).await.unwrap()["statusReason"],
        "PLAYER_CANCELED"
    );
    assert_eq!(deploy.launch_count(), 0, "nothing was launched for the queue");

    assert!(
        mm.handle_player_matchmaking(join("ISLAND", queued, USER))
            .await
            .unwrap()
    );
    assert_eq!(status_body(&shared, USER).await.unwrap()["status"], "MATCHING");
}

#[tokio::test]
async fn a_leave_by_a_player_with_no_search_answers_no_candidate() {
    let shared = module(FakeDeploy::online("203.0.113.10", 7777));
    let answer = matchmaker(&shared)
        .cancel_matchmaking_for_leave("nobody-account-0001", solo())
        .await
        .unwrap();
    assert_eq!(
        (answer.outcome, answer.state.as_str(), answer.candidate_id),
        (LeaveOutcome::NoCandidate, "none", None)
    );
}

#[tokio::test]
async fn the_dedicated_only_build_leaves_a_search_to_the_shared_cancel() {
    // Rifts::dedicated_only: the module's candidate hooks are inert
    let shared = module(FakeDeploy::online("203.0.113.10", 7777));
    assert!(!shared.rifts.loaded());
    let mm = matchmaker(&shared);
    assert!(
        mm.handle_player_matchmaking(join("ISLAND", ISLAND, USER))
            .await
            .unwrap()
    );
    let answer = mm.cancel_matchmaking_for_leave(USER, solo()).await.unwrap();
    assert_eq!(answer.outcome, LeaveOutcome::Canceled);
}

#[tokio::test]
async fn a_full_queue_launches_and_the_waiting_players_travel_together() {
    let deploy = FakeDeploy::online("203.0.113.10", 7777);
    let shared = module(deploy.clone());
    let mm = matchmaker(&shared);
    let hunt = "CR19_PlayerHunt_Four";
    for player in ["a", "b", "c"] {
        assert!(
            mm.handle_player_matchmaking(join("ISLAND", hunt, player))
                .await
                .unwrap()
        );
    }
    assert_eq!(deploy.launch_count(), 0);
    // The queue waits 20 s for a fourth player, then the next poll launches it
    shared.matchmaking.clock.advance(21_000);
    assert_eq!(status_body(&shared, "a").await.unwrap()["status"], "MATCHING");
    assert_eq!(deploy.launch_count(), 1);
    let launch = deploy.launches.lock().unwrap()[0].clone();
    assert_eq!(launch.expected_players, ["a", "b", "c"]);
    assert_eq!(launch.game_mode.as_deref(), Some("ISLAND"));
    assert_eq!(launch.game_args.as_deref(), Some(""));
    shared.matchmaking.clock.advance(60_000);
    for player in ["a", "b", "c"] {
        let body = status_body(&shared, player).await.unwrap();
        assert_eq!(body["status"], "IN_PROGRESS", "{player}");
        assert_eq!(body["serverInfo"]["port"], 7777);
    }
}

#[tokio::test]
async fn the_fourth_player_of_a_queue_pops_it_before_getting_a_candidate_as_node_does() {
    let deploy = FakeDeploy::online("203.0.113.10", 7777);
    let shared = module(deploy.clone());
    let mm = matchmaker(&shared);
    let hunt = "CR19_PlayerHunt_Four";
    for player in ["a", "b", "c", "d"] {
        assert!(
            mm.handle_player_matchmaking(join("ISLAND", hunt, player))
                .await
                .unwrap()
        );
    }
    assert_eq!(deploy.launch_count(), 1);
    shared.matchmaking.clock.advance(60_000);
    for player in ["a", "b", "c"] {
        assert_eq!(status_body(&shared, player).await.unwrap()["status"], "IN_PROGRESS");
    }
    // The queue was popped before the fourth player's candidate existed, so it never hears of the server
    assert_eq!(status_body(&shared, "d").await.unwrap()["status"], "MATCHING");
}

// ---- no DeployServer (matchmakingNoDeployServer.test.ts) ----

#[tokio::test]
async fn the_hub_status_is_not_asked_for_and_the_hubs_are_unavailable_without_a_deployserver() {
    let shared = module(Arc::new(Unconfigured));
    let hubs = shared.matchmaking.clone();
    assert_eq!(hubs.hub_game_session_id("h", 1).len(), 36);
    let candidate = {
        // a Ramsgate candidate that was cached: the live check says the hub is not verified
        let mut result = MatchmakingResult::matching(RAMSGATE, "c1".into());
        result.ready = true;
        result.host = "198.51.100.9".into();
        result.port = 8790;
        result
    };
    let answer = candidate_status_answer(&shared, USER, Some(&candidate), None).await;
    assert_eq!(answer.body.unwrap()["status"], "MATCHING");
}

#[tokio::test]
async fn a_ramsgate_request_with_no_deployserver_is_cancelled_with_no_dedicated_servers() {
    let shared = module(Arc::new(Unconfigured));
    let mm = matchmaker(&shared);
    assert!(
        mm.handle_player_matchmaking(join("CITY", RAMSGATE, "solo-account-0001"))
            .await
            .unwrap()
    );
    let candidate = mm.player_candidate("solo-account-0001").await.unwrap().unwrap();
    assert!(candidate.canceled, "the candidate is cancelled, not left waiting");
    assert_eq!(candidate.status_reason.as_deref(), Some(NO_DEDICATED_SERVERS_REASON));
}

#[tokio::test]
async fn a_partys_dedicated_hunt_with_no_deployserver_cancels_every_member() {
    let shared = module(Arc::new(Unconfigured));
    let mm = matchmaker(&shared);
    let members = ["party-leader-0001", "party-follower-0001"];
    assert!(
        mm.handle_player_matchmaking(party_join(ISLAND, members[0], "party-no-deployserver-1", &members))
            .await
            .unwrap()
    );
    for member in members {
        let candidate = mm.player_candidate(member).await.unwrap().unwrap();
        assert!(candidate.canceled, "{member} is cancelled");
        assert_eq!(
            candidate.status_reason.as_deref(),
            Some(NO_DEDICATED_SERVERS_REASON),
            "{member}"
        );
    }
}

// ---- the join's refusals ----

#[tokio::test]
async fn a_launch_the_director_refuses_is_a_rejected_join_and_one_that_throws_fails_the_request() {
    let shared = module(FakeDeploy::failing());
    assert!(
        !matchmaker(&shared)
            .handle_player_matchmaking(join("ISLAND", ISLAND, USER))
            .await
            .unwrap()
    );
    let shared = module(FakeDeploy::throwing());
    assert!(
        matchmaker(&shared)
            .handle_player_matchmaking(join("ISLAND", ISLAND, USER))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn matchmaking_mode_must_be_deployserver() {
    for mode in ["DISABLED", "", "deployserver", "SOMETHING"] {
        let shared = shared(&[("MATCHMAKING_MODE", mode)], FakeDeploy::online("h", 1));
        assert!(
            !matchmaker(&shared)
                .handle_player_matchmaking(join("ISLAND", ISLAND, USER))
                .await
                .unwrap(),
            "{mode:?}"
        );
    }
}

// ---- Gauntlet (matchmakingHuntPolicy.test.ts) ----

#[tokio::test]
async fn the_dedicated_route_cancels_a_gauntlet_request_with_gauntlet_disabled_for_the_requester_and_the_party() {
    let deploy = FakeDeploy::online("203.0.113.10", 7777);
    let shared = module(deploy.clone());
    let mm = matchmaker(&shared);
    assert!(
        mm.handle_player_matchmaking(join("ISLAND", "CR19_PlayerHunt_Gauntlet", "core-solo-player"))
            .await
            .unwrap()
    );
    let solo_result = mm
        .check_and_update_queue_status("core-solo-player")
        .await
        .unwrap()
        .unwrap();
    assert!(solo_result.canceled);
    assert_eq!(solo_result.status_reason.as_deref(), Some("GAUNTLET_DISABLED"));
    assert!(!solo_result.ready);
    assert_eq!(solo_result.hunt_id, "CR19_PlayerHunt_Gauntlet");

    let members = ["core-leader", "core-member"];
    assert!(
        mm.handle_player_matchmaking(party_join(
            "CR19_MatchmakerHunt_Gauntlet",
            members[0],
            "core-party",
            &members
        ))
        .await
        .unwrap()
    );
    for player in members {
        let result = mm.check_and_update_queue_status(player).await.unwrap().unwrap();
        assert_eq!(result.status_reason.as_deref(), Some("GAUNTLET_DISABLED"), "{player}");
    }
    assert_eq!(deploy.launch_count(), 0);
}

#[tokio::test]
async fn the_dedicated_route_queues_a_gauntlet_request_with_the_switch_on_and_other_hunts_are_not_refused_by_name() {
    let shared = shared(&[("MYSTICPARADOX_GAUNTLET", "on")], FakeDeploy::online("h", 1));
    let mm = matchmaker(&shared);
    assert!(
        mm.handle_player_matchmaking(join("ISLAND", "CR19_PlayerHunt_Gauntlet", "core-gauntlet-on"))
            .await
            .unwrap()
    );
    let queued = mm
        .check_and_update_queue_status("core-gauntlet-on")
        .await
        .unwrap()
        .unwrap();
    assert!(!queued.canceled);
    assert_eq!(queued.status_reason, None);

    assert!(
        mm.handle_player_matchmaking(join("ISLAND", "CR19_PlayerHunt_Arena_Elite", "core-arena"))
            .await
            .unwrap()
    );
    assert!(
        !mm.check_and_update_queue_status("core-arena")
            .await
            .unwrap()
            .unwrap()
            .canceled
    );
}

#[tokio::test]
async fn a_party_returning_to_a_hub_fans_the_destination_out_to_every_member() {
    let deploy = FakeDeploy::online("203.0.113.9", 8790);
    let shared = module(deploy.clone());
    let mm = matchmaker(&shared);
    let members = ["lead", "m1"];
    let request = JoinRequest {
        game_mode: Some("CITY".into()),
        ..party_join(RAMSGATE, "lead", "p1", &members)
    };
    assert!(mm.handle_player_matchmaking(request).await.unwrap());
    let (lead, member) = (
        shared.matchmaking.candidate("lead").unwrap(),
        shared.matchmaking.candidate("m1").unwrap(),
    );
    assert_ne!(
        lead.candidate_id, member.candidate_id,
        "each client consumes its own candidate"
    );
    assert_eq!(
        lead.game_session_id, member.game_session_id,
        "the hub's room id is shared"
    );
    assert_eq!(
        lead.game_session_id,
        shared.matchmaking.hub_game_session_id("203.0.113.9", 8790)
    );
    assert!(shared.matchmaking.is_hub_game_session_id(&lead.game_session_id));
    assert_eq!(mm.party_instance("p1", None).await.unwrap().unwrap().hunt_id, RAMSGATE);
    // Hubs launch with the whole party expected, and a hub is no dedicated hunt
    assert_eq!(deploy.launches.lock().unwrap()[0].expected_players, ["lead", "m1"]);
}

// ---- GET /candidate/status (candidateStatus.test.ts) ----

fn candidate(change: impl FnOnce(&mut MatchmakingResult)) -> MatchmakingResult {
    let mut result = MatchmakingResult::matching(ISLAND, "candidate-1".into());
    result.game_session_id = "game-session-1".into();
    change(&mut result);
    result
}

#[tokio::test]
async fn a_missing_candidate_is_a_404_with_no_body() {
    let shared = module(FakeDeploy::online("h", 1));
    let answer = candidate_status_answer(&shared, USER, None, None).await;
    assert_eq!((answer.status, answer.body), (404, None));
}

#[tokio::test]
async fn a_dedicated_hunt_candidate_that_is_still_matching_keeps_its_hunt_id_and_island() {
    let deploy = FakeDeploy::online("h", 1);
    let shared = module(deploy.clone());
    let answer = candidate_status_answer(&shared, USER, Some(&candidate(|_| {})), Some("203.0.113.7")).await;
    assert_eq!(answer.status, 200);
    let body = answer.body.unwrap();
    assert_eq!(
        body,
        json!({
            "candidateId": "candidate-1",
            "candidateStatusPeriodMillis": 1000,
            "gameMode": "ISLAND",
            "huntId": ISLAND,
            "playerStates": { USER: {} },
            "status": "MATCHING",
            "statusDuration": 0,
            "statusReason": null
        })
    );
    assert_eq!(
        body.to_string(),
        format!(
            r#"{{"candidateId":"candidate-1","candidateStatusPeriodMillis":1000,"gameMode":"ISLAND","huntId":"{ISLAND}","playerStates":{{"{USER}":{{}}}},"status":"MATCHING","statusDuration":0,"statusReason":null}}"#
        ),
        "key order is JSON.stringify's"
    );
    assert_eq!(*deploy.hub_asks.lock().unwrap(), 0);
}

#[tokio::test]
async fn a_cancelled_hunt_candidate_answers_canceled_with_its_reason_and_no_retry_hint() {
    let shared = module(FakeDeploy::online("h", 1));
    let canceled = candidate(|c| {
        c.canceled = true;
        c.status_reason = Some("P2P_REQUIRED_ACCOUNT_NOT_ALLOWLISTED".into());
    });
    let body = candidate_status_answer(&shared, USER, Some(&canceled), None)
        .await
        .body
        .unwrap();
    assert_eq!(body["status"], "CANCELED");
    assert_eq!(body["statusReason"], "P2P_REQUIRED_ACCOUNT_NOT_ALLOWLISTED");
    assert_eq!(body["huntId"], Value::Null);
    assert!(body.get("retryAfterSeconds").is_none());
    let unreasoned = candidate(|c| c.canceled = true);
    let body = candidate_status_answer(&shared, USER, Some(&unreasoned), None)
        .await
        .body
        .unwrap();
    assert_eq!(body["statusReason"], "HOST_ROUTE_FAILED");
}

#[tokio::test]
async fn a_hunt_cancelled_for_its_realms_capacity_carries_the_30_s_retry_hint() {
    let shared = module(FakeDeploy::online("h", 1));
    let full = candidate(|c| {
        c.canceled = true;
        c.status_reason = Some("REALM_FULL".into());
    });
    let body = candidate_status_answer(&shared, USER, Some(&full), None)
        .await
        .body
        .unwrap();
    assert_eq!(body["statusReason"], "REALM_FULL");
    assert_eq!(body["retryAfterSeconds"], REALM_FULL_RETRY_AFTER_SECONDS);
    assert_eq!(REALM_FULL_RETRY_AFTER_SECONDS, 30);
    assert_eq!(
        body.to_string().rsplit_once(",\"retryAfterSeconds\"").unwrap().1,
        ":30}",
        "the hint is last"
    );
}

#[tokio::test]
async fn a_dedicated_ramsgate_travel_is_verified_against_the_live_hub() {
    let stale = FakeDeploy::online("h", 1);
    stale.set_ramsgate(false, "", 0);
    let shared = module(stale.clone());
    let ramsgate = candidate(|c| {
        c.hunt_id = RAMSGATE.into();
        c.ready = true;
        c.host = "198.51.100.9".into();
        c.port = 8790;
    });
    let holding = candidate_status_answer(&shared, USER, Some(&ramsgate), None)
        .await
        .body
        .unwrap();
    assert_eq!(holding["status"], "MATCHING");
    assert_eq!(holding["candidateStatusPeriodMillis"], 1000);
    assert_eq!(holding["gameMode"], "CITY");
    assert_eq!(holding["huntId"], Value::Null);
    assert_eq!(*stale.hub_asks.lock().unwrap(), 1);

    let live = FakeDeploy::online("h", 1);
    live.set_ramsgate(true, "198.51.100.9", 8791);
    let shared = module(live);
    let ramsgate = candidate(|c| {
        c.hunt_id = RAMSGATE.into();
        c.ready = true;
        c.host = "198.51.100.9".into();
        c.port = 8790;
        c.game_session_id = "hub-room".into();
    });
    let body = candidate_status_answer(&shared, USER, Some(&ramsgate), Some("203.0.113.7"))
        .await
        .body
        .unwrap();
    assert_eq!(body["status"], "IN_PROGRESS");
    assert_eq!(body["gameMode"], "CITY");
    assert_eq!(body["huntId"], Value::Null);
    assert_eq!(
        body["serverInfo"],
        json!({ "buildId": "647472_1.14.7_shipping", "gameSessionId": "hub-room", "host": "198.51.100.9", "port": 8791 })
    );
    assert_eq!(body["candidateStatusPeriodMillis"], 10_000);
}

#[tokio::test]
async fn a_client_behind_the_hubs_public_address_is_told_to_use_loopback() {
    let live = FakeDeploy::online("h", 1);
    live.set_ramsgate(true, "198.51.100.9", 8790);
    let shared = module(live);
    let ramsgate = candidate(|c| {
        c.hunt_id = RAMSGATE.into();
        c.ready = true;
        c.host = "198.51.100.9".into();
        c.port = 8790;
    });
    for (ip, host) in [
        (Some("198.51.100.9"), "127.0.0.1"),
        (Some("::ffff:198.51.100.9"), "127.0.0.1"),
        (Some("127.0.0.1"), "127.0.0.1"),
        (Some("::1"), "127.0.0.1"),
        (Some("203.0.113.7"), "198.51.100.9"),
        (None, "198.51.100.9"),
    ] {
        let body = candidate_status_answer(&shared, USER, Some(&ramsgate), ip)
            .await
            .body
            .unwrap();
        assert_eq!(body["serverInfo"]["host"], host, "{ip:?}");
    }
}

fn realm(change: impl FnOnce(&mut MatchmakingResult)) -> MatchmakingResult {
    candidate(|c| {
        c.source = Some(Source::RealmHub);
        c.source_session_id = Some("hub-session".into());
        change(c);
    })
}

#[tokio::test]
async fn a_realms_ramsgate_candidate_is_never_checked_against_the_deployserver() {
    let deploy = FakeDeploy::online("h", 1);
    let shared = module(deploy.clone());
    let running = realm(|c| {
        c.hunt_id = RAMSGATE.into();
        c.game_session_id = "5f1c0e7a-3d2b-4a6c-9e8f-0123456789ab".into();
        c.ready = true;
        c.host = "127.0.0.1".into();
        c.port = 56000;
    });
    let body = candidate_status_answer(&shared, USER, Some(&running), Some("203.0.113.7"))
        .await
        .body
        .unwrap();
    assert_eq!(
        *deploy.hub_asks.lock().unwrap(),
        0,
        "the hub's live check is not asked for a realm hub"
    );
    assert_eq!(body["status"], "IN_PROGRESS");
    assert_eq!(body["gameMode"], "CITY");
    assert_eq!(body["huntId"], Value::Null);
    assert_eq!(
        body["serverInfo"]["gameSessionId"],
        "5f1c0e7a-3d2b-4a6c-9e8f-0123456789ab"
    );
    assert_eq!(body["serverInfo"]["host"], "127.0.0.1");
    assert_eq!(body["serverInfo"]["port"], 56000);

    let waiting = realm(|c| c.hunt_id = RAMSGATE.into());
    let body = candidate_status_answer(&shared, USER, Some(&waiting), None)
        .await
        .body
        .unwrap();
    assert_eq!(body["status"], "MATCHING");
    assert_eq!(body["gameMode"], "CITY");
    assert_eq!(body["huntId"], Value::Null, "as it will be when it runs");
}

#[tokio::test]
async fn a_realms_training_hub_is_shared_with_the_training_hunt_id_waiting_or_running() {
    let shared = module(FakeDeploy::online("h", 1));
    let waiting = realm(|c| c.hunt_id = TRAINING.into());
    let body = candidate_status_answer(&shared, USER, Some(&waiting), None)
        .await
        .body
        .unwrap();
    assert_eq!(
        (
            body["status"].as_str(),
            body["gameMode"].as_str(),
            body["huntId"].as_str()
        ),
        (Some("MATCHING"), Some("SHARED"), Some(TRAINING))
    );

    let running = realm(|c| {
        c.hunt_id = TRAINING.into();
        c.ready = true;
        c.host = "127.0.0.1".into();
        c.port = 56001;
        c.game_session_id = "training-room".into();
    });
    let body = candidate_status_answer(&shared, USER, Some(&running), None)
        .await
        .body
        .unwrap();
    assert_eq!(
        (
            body["status"].as_str(),
            body["gameMode"].as_str(),
            body["huntId"].as_str()
        ),
        (Some("IN_PROGRESS"), Some("SHARED"), Some(TRAINING))
    );
}

#[tokio::test]
async fn a_realm_hub_cancelled_with_its_reason_says_so_without_a_retry_hint() {
    let shared = module(FakeDeploy::online("h", 1));
    let canceled = realm(|c| {
        c.hunt_id = TRAINING.into();
        c.canceled = true;
        c.status_reason = Some("REALM_HUB_START_TIMEOUT".into());
    });
    let body = candidate_status_answer(&shared, USER, Some(&canceled), None)
        .await
        .body
        .unwrap();
    assert_eq!(body["status"], "CANCELED");
    assert_eq!(body["statusReason"], "REALM_HUB_START_TIMEOUT");
    assert_eq!(body["gameMode"], "SHARED");
    assert_eq!(body["huntId"], Value::Null);
    assert!(body.get("retryAfterSeconds").is_none());
}

#[tokio::test]
async fn the_poll_period_is_one_second_while_matching_and_in_the_release_window_then_ten() {
    let shared = module(FakeDeploy::online("h", 1));
    let now = shared.matchmaking.clock.now_ms();
    assert_eq!(candidate(|c| c.ready = false).status_period_millis(now), 1000);
    assert_eq!(
        candidate(|c| {
            c.ready = true;
            c.ready_at = now + 5000;
        })
        .status_period_millis(now),
        1000
    );
    assert_eq!(
        candidate(|c| {
            c.ready = true;
            c.ready_at = 0;
        })
        .status_period_millis(now),
        10_000
    );
}

/// DN8: the ring gets the address the player connects from, which the hosting node's allowlist admits; a join
/// without one stays on this node.
#[tokio::test(flavor = "multi_thread")]
async fn the_ring_is_given_the_players_address_and_skipped_without_one() {
    use module_api::module::{HostCall, ModuleContext};
    use module_api::network::{HomeCall, IdKind};
    use module_api::{GlobalId, NodeIdentity};

    let host = module_api::stub::recording_host();
    let node = NodeIdentity::standalone();
    let context = ModuleContext::new("isles-1-14-7".parse().unwrap(), node.clone(), host.clone());
    let dir = tempfile::tempdir().unwrap();
    let config = store::DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    };
    let home = Arc::new(isles_core::db::ContextHome(context));
    let opened = isles_core::db::IslesStore::open(&config, &store::DatabaseOptions::default(), home, &node).unwrap();
    let mut shared = module(FakeDeploy::online("203.0.113.9", 8790));
    shared.store = Some(opened);
    let mm = matchmaker(&shared);
    let ring_ips = || {
        host.calls()
            .into_iter()
            .filter_map(|(_, call)| match call {
                HostCall::Home(HomeCall::RingSubmitTicket { client_ip, .. }) => Some(("ticket", client_ip)),
                HostCall::Home(HomeCall::RingAssignRamsgate { client_ip, .. }) => Some(("ramsgate", client_ip)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    let hunter = GlobalId::new(IdKind::Account, &node.node).to_string();
    let mut request = join("ISLAND", "CR19_PlayerHunt_Four", &hunter);
    request.client_ip = Some("198.51.100.7".to_owned());
    assert!(mm.handle_player_matchmaking(request).await.unwrap());

    let hub = GlobalId::new(IdKind::Account, &node.node).to_string();
    let mut request = join("CITY", RAMSGATE, &hub);
    request.client_ip = Some("198.51.100.8".to_owned());
    assert!(mm.handle_player_matchmaking(request).await.unwrap());

    let silent = GlobalId::new(IdKind::Account, &node.node).to_string();
    assert!(
        mm.handle_player_matchmaking(join("ISLAND", "CR19_PlayerHunt_Four", &silent))
            .await
            .unwrap()
    );

    assert_eq!(
        ring_ips(),
        [
            ("ticket", "198.51.100.7".to_owned()),
            ("ramsgate", "198.51.100.8".to_owned())
        ]
    );
}
