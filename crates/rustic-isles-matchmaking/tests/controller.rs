// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The controller (spec §6, §17.2 "Controller"): a bare module with the fake Director, the clock moved, never slept.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::sync::Arc;

use common::{Activity, RecordingHome, account, ctx, module_with, solo, with_party};
use rustic_isles::{LiveHub, Settings};
use rustic_isles_matchmaking::test_support::{FakeDirector, FakeLaunch, TestModule};
use rustic_isles_matchmaking::{CandidateSource, JoinRequest, MatchCandidate, Parties, RAMSGATE_HUNT};
use rustic_isles_store::test_support::TempStore;
use serde_json::Value;

const ISLAND: &str = "ShatteredIsles_IslandA";
const CR19: &str = "CR19_PlayerHunt_X";
/// Past any default Go Now window.
const PAST_RELEASE: i64 = 50_000;

fn director() -> Arc<FakeDirector> {
    FakeDirector::online("203.0.113.10", 7777)
}

fn module() -> (TestModule, Arc<FakeDirector>) {
    let d = director();
    (module_with(None, Settings::default(), &d), d)
}

async fn leave(module: &TestModule, player: &str) -> (String, String) {
    let done = ctx(module, None).cancel_for_leave(player).await.unwrap();
    (done.outcome, done.state)
}

fn reason(module: &TestModule, player: &str) -> Option<String> {
    module.state.candidate(player).and_then(|c| c.status_reason)
}

#[tokio::test]
async fn a_solo_hunt_cancelled_before_its_release() {
    let (m, d) = module();
    let c = ctx(&m, None);
    assert!(c.join(&solo("a", Some("ISLAND"), Some(ISLAND))).await.unwrap());
    let searched = m.state.candidate("a").unwrap().candidate_id;
    // Ready on the server but inside its Go Now window: "go-now" by §6.4 step 6
    assert_eq!(leave(&m, "a").await, ("canceled".into(), "go-now".into()));
    let status = c.status_answer("a", c.poll("a").await.unwrap().as_ref()).await;
    let body = status.body.unwrap();
    assert_eq!(
        (body["status"].as_str(), body["statusReason"].as_str()),
        (Some("CANCELED"), Some("PLAYER_CANCELED"))
    );
    assert!(body.get("retryAfterSeconds").is_none());
    // A cancel keeps the search's id
    assert_eq!(body["candidateId"].as_str(), Some(searched.as_str()));
    assert_eq!(leave(&m, "a").await, ("already-canceled".into(), "canceled".into()));
    assert_eq!(d.launches().len(), 1);
    assert!(c.join(&solo("a", Some("ISLAND"), Some(ISLAND))).await.unwrap());
    assert_eq!(d.launches().len(), 2, "a re-join launches a fresh hunt");
    let again = c
        .status_answer("a", c.poll("a").await.unwrap().as_ref())
        .await
        .body
        .unwrap();
    assert_eq!(again["status"], "MATCHING");
}

#[tokio::test]
async fn a_party_hunt_cancelled_by_its_leader() {
    let temp = TempStore::open().unwrap();
    for user in ["alice", "bob"] {
        account(&temp.store, user).await;
    }
    let d = director();
    let m = module_with(Some(temp.store.clone()), Settings::default(), &d);
    let c = ctx(&m, None);
    let parties = Parties {
        store: Some(&temp.store),
        node: &m.shared.node().node,
        request_id: None,
        now: m.state.now_ms(),
    };
    let party = parties.get_or_create("alice", "647472").await.unwrap();
    parties.invite("alice", "bob", "647472", "Alice", "win").await.unwrap();
    let joined = parties.accept("bob", "alice", "647472").await.unwrap().unwrap();
    assert_eq!(joined.members.len(), 2);
    let join = with_party(
        "alice",
        "ISLAND",
        Some(ISLAND),
        &party.party_id,
        &["alice", "bob"],
        Some(party.id.clone()),
    );
    assert!(c.join(&join).await.unwrap());
    let (a, b) = (m.state.candidate("alice").unwrap(), m.state.candidate("bob").unwrap());
    assert_eq!(a.game_session_id, b.game_session_id);
    assert_ne!(a.candidate_id, b.candidate_id);
    assert_eq!(leave(&m, "alice").await, ("canceled".into(), "go-now".into()));
    assert_eq!(reason(&m, "alice").as_deref(), Some("PLAYER_CANCELED"));
    assert_eq!(reason(&m, "bob").as_deref(), Some("PARTY_MEMBER_CANCELED"));
    assert!(c.party_instance(&party.party_id, None).await.unwrap().is_none());
    // The leader searches again and the member follows the new instance
    assert!(c.join(&join).await.unwrap());
    let (a2, b2) = (m.state.candidate("alice").unwrap(), m.state.candidate("bob").unwrap());
    assert!(!a2.canceled && !b2.canceled);
    assert_eq!(a2.game_session_id, b2.game_session_id);
    assert_ne!(a2.game_session_id, a.game_session_id);
    assert_eq!(d.launches().len(), 2);
}

#[tokio::test]
async fn a_leave_after_the_release_is_too_late() {
    let (m, _) = module();
    let c = ctx(&m, None);
    c.join(&solo("a", Some("ISLAND"), Some(ISLAND))).await.unwrap();
    m.state.clock().advance(PAST_RELEASE);
    let before = m.state.candidate("a").unwrap();
    assert_eq!(leave(&m, "a").await, ("too-late".into(), "ready".into()));
    assert_eq!(m.state.candidate("a").unwrap(), before);
}

#[tokio::test]
async fn hubs_queues_and_nothing() {
    let (m, _) = module();
    let c = ctx(&m, None);
    assert_eq!(leave(&m, "nobody").await, ("no-candidate".into(), "none".into()));
    c.join(&solo("h", Some("CITY"), None)).await.unwrap();
    assert_eq!(leave(&m, "h").await, ("hub-not-cancelled".into(), "hub".into()));
    c.join(&solo("q", Some("ISLAND"), Some(CR19))).await.unwrap();
    assert_eq!(leave(&m, "q").await, ("canceled".into(), "queued".into()));
    // The queue emptied and went; a later join queues again
    c.join(&solo("q", Some("ISLAND"), Some(CR19))).await.unwrap();
    assert_eq!(leave(&m, "q").await, ("canceled".into(), "queued".into()));
}

#[tokio::test]
async fn four_players_fill_a_queue_in_nodes_order() {
    let (m, d) = module();
    let c = ctx(&m, None);
    for p in ["a", "b", "c"] {
        assert!(c.join(&solo(p, Some("ISLAND"), Some(CR19))).await.unwrap());
    }
    assert!(d.launches().is_empty());
    assert!(c.join(&solo("d", Some("ISLAND"), Some(CR19))).await.unwrap());
    assert_eq!(d.launches()[0].expected_players, ["a", "b", "c", "d"]);
    let first = m.state.candidate("a").unwrap();
    for p in ["a", "b", "c"] {
        let candidate = m.state.candidate(p).unwrap();
        assert!(candidate.ready);
        assert_eq!(
            (candidate.host.as_str(), candidate.ready_at),
            ("203.0.113.10", first.ready_at)
        );
    }
    // The fourth's candidate was made after the launch and waits on a queue that no longer exists
    let fourth = m.state.candidate("d").unwrap();
    assert!(!fourth.ready && !fourth.canceled);
    m.state.clock().advance(30_000);
    assert!(!c.poll("d").await.unwrap().unwrap().ready);
    assert_eq!(d.launches().len(), 1);
}

#[tokio::test]
async fn three_players_then_twenty_seconds() {
    let (m, d) = module();
    let c = ctx(&m, None);
    for p in ["a", "b", "c"] {
        c.join(&solo(p, Some("ISLAND"), Some(CR19))).await.unwrap();
    }
    m.state.clock().advance(19_000);
    c.poll("a").await.unwrap();
    assert!(d.launches().is_empty(), "inside the wait");
    m.state.clock().advance(2_000);
    c.poll("a").await.unwrap();
    let release = m.state.candidate("a").unwrap().ready_at;
    for p in ["a", "b", "c"] {
        let candidate = m.state.candidate(p).unwrap();
        assert!(candidate.ready);
        assert_eq!(candidate.ready_at, release);
    }
}

#[tokio::test]
async fn without_a_director() {
    let m = TestModule::new(None, Settings::default());
    let c = ctx(&m, None);
    assert!(c.join(&solo("a", Some("CITY"), None)).await.unwrap());
    let a = m.state.candidate("a").unwrap();
    assert_eq!(
        (a.canceled, a.status_reason.as_deref(), a.hunt_id.as_str()),
        (true, Some("NO_DEDICATED_SERVERS"), RAMSGATE_HUNT)
    );
    let party = with_party("a", "ISLAND", Some(ISLAND), "p", &["a", "b"], None);
    assert!(c.join(&party).await.unwrap());
    for p in ["a", "b"] {
        assert_eq!(reason(&m, p).as_deref(), Some("NO_DEDICATED_SERVERS"));
    }
    let cached = MatchCandidate {
        ready: true,
        host: "198.51.100.9".into(),
        port: 7777,
        ..MatchCandidate::matching(RAMSGATE_HUNT, "c")
    };
    m.state.set_candidate("r", cached);
    let body = c
        .status_answer("r", c.poll("r").await.unwrap().as_ref())
        .await
        .body
        .unwrap();
    assert_eq!(
        (body["status"].as_str(), body["gameMode"].as_str()),
        (Some("MATCHING"), Some("CITY"))
    );
    assert_eq!(m.state.hub_game_session_id("198.51.100.9", 7777).len(), 36);
}

#[tokio::test]
async fn a_director_that_refuses_or_throws() {
    let (m, d) = module();
    let c = ctx(&m, None);
    d.set_answer(FakeLaunch::Refused(None));
    assert!(!c.join(&solo("a", Some("ISLAND"), Some(ISLAND))).await.unwrap());
    d.set_answer(FakeLaunch::Fails("boom".into()));
    assert_eq!(
        c.join(&solo("a", Some("ISLAND"), Some(ISLAND))).await,
        Err("boom".into())
    );
}

#[tokio::test]
async fn a_failed_pop_clears_the_launching_mark() {
    let (m, d) = module();
    let c = ctx(&m, None);
    for p in ["a", "b", "c"] {
        c.join(&solo(p, Some("ISLAND"), Some(CR19))).await.unwrap();
    }
    d.set_answer(FakeLaunch::Fails("down".into()));
    assert!(c.join(&solo("d", Some("ISLAND"), Some(CR19))).await.is_err());
    // Not stuck resolved: the next join is taken (a departure, spec §15 item 4)
    d.set_answer(FakeLaunch::Online {
        host: "203.0.113.10".into(),
        port: 7777,
    });
    assert!(c.join(&solo("e", Some("ISLAND"), Some(CR19))).await.unwrap());
    assert!(m.state.candidate("a").unwrap().ready);
}

#[tokio::test]
async fn matchmaking_modes() {
    for mode in ["DISABLED", "", "deployserver", "SOMETHING"] {
        let d = director();
        let m = module_with(None, Settings::from_pairs([("MATCHMAKING_MODE", mode)]), &d);
        assert!(
            !ctx(&m, None)
                .join(&solo("a", Some("ISLAND"), Some(ISLAND)))
                .await
                .unwrap(),
            "{mode}"
        );
    }
}

#[tokio::test]
async fn gauntlet_switch() {
    let (m, d) = module();
    let c = ctx(&m, None);
    let party = with_party("a", "ISLAND", Some("Gauntlet_Season_1"), "p", &["a", "b"], None);
    assert!(c.join(&party).await.unwrap());
    for p in ["a", "b"] {
        assert_eq!(reason(&m, p).as_deref(), Some("GAUNTLET_DISABLED"));
    }
    assert!(d.launches().is_empty());
    let on = module_with(None, Settings::from_pairs([("RUSTIC_ISLES_GAUNTLET", " On ")]), &d);
    let c = ctx(&on, None);
    c.join(&solo("a", Some("ISLAND"), Some("CR19_Gauntlet_X")))
        .await
        .unwrap();
    assert_eq!(leave(&on, "a").await.1, "queued");
    c.join(&solo("b", Some("ISLAND"), Some("CR19_PlayerHunt_Arena_Elite")))
        .await
        .unwrap();
    assert_eq!(leave(&on, "b").await.1, "queued");
    // The level the client chose goes to the Director with the launch
    let picked = JoinRequest {
        gauntlet_level: Some(10_000_003),
        ..with_party("c", "ISLAND", Some("CR19_PlayerHunt_Gauntlet"), "q", &["c", "d"], None)
    };
    assert!(c.join(&picked).await.unwrap());
    let launched = d
        .launches()
        .into_iter()
        .find(|l| l.hunt_id == "CR19_PlayerHunt_Gauntlet")
        .unwrap();
    assert_eq!(launched.gauntlet_level, Some(10_000_003));
}

#[tokio::test]
async fn a_party_returning_to_the_hub() {
    let (m, d) = module();
    let activity = Arc::new(Activity::default());
    m.state.set_hunt_activity(activity.clone());
    let c = ctx(&m, None);
    let join = with_party("a", "CITY", None, "p", &["a", "b"], None);
    assert!(c.join(&join).await.unwrap());
    let (a, b) = (m.state.candidate("a").unwrap(), m.state.candidate("b").unwrap());
    assert_ne!(a.candidate_id, b.candidate_id);
    assert_eq!(a.game_session_id, b.game_session_id);
    assert_eq!(a.game_session_id, m.state.hub_game_session_id("203.0.113.10", 7777));
    let instance = c.party_instance("p", None).await.unwrap().unwrap();
    assert_eq!(instance.hunt_id, RAMSGATE_HUNT);
    assert_eq!(d.launches()[0].expected_players, ["a", "b"]);
    assert_eq!(d.launches()[0].game_mode.as_deref(), Some("CITY"));
    assert!(activity.starts.lock().unwrap().is_empty());
    assert_eq!(
        activity.ends.lock().unwrap()[0],
        (vec!["a".to_owned(), "b".to_owned()], "RETURNED".to_owned())
    );
}

#[tokio::test]
async fn a_party_hunt_reuses_its_instance_for_two_minutes() {
    let (m, d) = module();
    let activity = Arc::new(Activity::default());
    m.state.set_hunt_activity(activity.clone());
    let c = ctx(&m, None);
    let join = with_party("a", "ISLAND", Some(ISLAND), "p", &["a", "b"], None);
    c.join(&join).await.unwrap();
    let first = m.state.candidate("a").unwrap();
    let hunt_ref = activity.starts.lock().unwrap()[0].2.clone();
    // A margin for the real clock under the offset
    m.state.clock().advance(118_000);
    c.join(&join).await.unwrap();
    let again = m.state.candidate("a").unwrap();
    assert_eq!(again.game_session_id, first.game_session_id);
    assert_ne!(again.candidate_id, first.candidate_id);
    assert_eq!(d.launches().len(), 1);
    assert_eq!(
        activity.starts.lock().unwrap()[1],
        (vec!["a".to_owned()], ISLAND.to_owned(), hunt_ref)
    );
    m.state.clock().advance(2_000);
    c.join(&join).await.unwrap();
    assert_eq!(d.launches().len(), 2);
}

fn status_of(answer: &rustic_isles_matchmaking::status::StatusAnswer) -> &Value {
    answer.body.as_ref().unwrap()
}

#[tokio::test]
async fn status_answers() {
    let (m, _) = module();
    let c = ctx(&m, None);
    let none = c.status_answer("x", None).await;
    assert_eq!((none.status, none.body.is_none()), (404, true));
    c.join(&solo("q", Some("ISLAND"), Some(CR19))).await.unwrap();
    let queued = c.status_answer("q", c.poll("q").await.unwrap().as_ref()).await;
    let body = status_of(&queued);
    assert_eq!(
        dainty_service_kit::jsjson::stringify(body).replace(body["candidateId"].as_str().unwrap(), "ID"),
        r#"{"candidateId":"ID","candidateStatusPeriodMillis":1000,"gameMode":"ISLAND","huntId":"CR19_PlayerHunt_X","playerStates":{"q":{}},"status":"MATCHING","statusDuration":0,"statusReason":null}"#
    );
    m.state.cancel_hunt(ISLAND, &["w".into()], "WHY");
    let cancelled = c.status_answer("w", c.poll("w").await.unwrap().as_ref()).await;
    let body = status_of(&cancelled);
    assert_eq!(
        (body["status"].as_str(), body["statusReason"].as_str()),
        (Some("CANCELED"), Some("WHY"))
    );
    assert_eq!(body["huntId"], Value::Null);
    assert!(body.get("retryAfterSeconds").is_none());
    m.state.cancel_hunt(ISLAND, &["f".into()], "REALM_FULL");
    let full = c.status_answer("f", c.poll("f").await.unwrap().as_ref()).await;
    assert_eq!(status_of(&full)["retryAfterSeconds"], 30);
    let keys: Vec<&String> = status_of(&full).as_object().unwrap().keys().collect();
    assert_eq!(keys.last().map(|k| k.as_str()), Some("retryAfterSeconds"));
}

#[tokio::test]
async fn ramsgate_is_verified_against_the_director() {
    let (m, d) = module();
    let c = ctx(&m, None);
    let cached = MatchCandidate {
        ready: true,
        host: "198.51.100.9".into(),
        port: 7777,
        game_session_id: "hub-room".into(),
        ..MatchCandidate::matching(RAMSGATE_HUNT, "c1")
    };
    m.state.set_candidate("r", cached);
    let held = c.status_answer("r", c.poll("r").await.unwrap().as_ref()).await;
    let body = status_of(&held);
    assert_eq!(body["candidateStatusPeriodMillis"], 1000);
    assert_eq!(
        (body["gameMode"].as_str(), body["status"].as_str()),
        (Some("CITY"), Some("MATCHING"))
    );
    assert_eq!(body["huntId"], Value::Null);
    assert_eq!(d.hub_asks(), 1);
    d.set_ramsgate(Some(LiveHub {
        ready: true,
        host: "198.51.100.9".into(),
        port: 8791,
    }));
    let served = c.status_answer("r", c.poll("r").await.unwrap().as_ref()).await;
    let body = status_of(&served);
    assert_eq!(body["status"], "IN_PROGRESS");
    assert_eq!(body["candidateStatusPeriodMillis"], 10_000);
    assert_eq!(
        dainty_service_kit::jsjson::stringify(&body["serverInfo"]),
        r#"{"buildId":"647472_1.14.7_shipping","gameSessionId":"hub-room","host":"198.51.100.9","port":8791}"#
    );
    let keys: Vec<&str> = body.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "candidateId",
            "candidateStatusPeriodMillis",
            "gameMode",
            "huntId",
            "playerStates",
            "serverInfo",
            "status",
            "statusDuration",
            "statusReason"
        ]
    );
    // A local client travels to loopback
    let local = ctx(&m, Some("::ffff:198.51.100.9"));
    let body = local
        .status_answer("r", local.poll("r").await.unwrap().as_ref())
        .await
        .body
        .unwrap();
    assert_eq!(body["serverInfo"]["host"], "127.0.0.1");
}

#[tokio::test]
async fn realm_hubs() {
    let (m, d) = module();
    let c = ctx(&m, None);
    let realm = |hunt: &str, ready: bool| MatchCandidate {
        ready,
        host: "198.51.100.9".into(),
        port: 7777,
        source: Some(CandidateSource::RealmHub),
        ..MatchCandidate::matching(hunt, "c")
    };
    m.state.set_candidate("r", realm(RAMSGATE_HUNT, true));
    let body = c
        .status_answer("r", c.poll("r").await.unwrap().as_ref())
        .await
        .body
        .unwrap();
    assert_eq!(body["status"], "IN_PROGRESS");
    assert_eq!(d.hub_asks(), 0);
    m.state.set_candidate("t", realm("Arena_TrainingDojo_01", false));
    let waiting = c
        .status_answer("t", c.poll("t").await.unwrap().as_ref())
        .await
        .body
        .unwrap();
    assert_eq!(
        (waiting["gameMode"].as_str(), waiting["huntId"].as_str()),
        (Some("SHARED"), Some("Arena_TrainingDojo_01"))
    );
    m.state.set_candidate("t", realm("Arena_TrainingDojo_01", true));
    let running = c
        .status_answer("t", c.poll("t").await.unwrap().as_ref())
        .await
        .body
        .unwrap();
    assert_eq!(
        (running["gameMode"].as_str(), running["huntId"].as_str()),
        (Some("SHARED"), Some("Arena_TrainingDojo_01"))
    );
    m.state.set_candidate(
        "x",
        MatchCandidate {
            canceled: true,
            status_reason: Some("REALM_CLOSED".into()),
            ..realm("Arena_TrainingDojo_01", false)
        },
    );
    let cancelled = c
        .status_answer("x", c.poll("x").await.unwrap().as_ref())
        .await
        .body
        .unwrap();
    assert_eq!(cancelled["statusReason"], "REALM_CLOSED");
    assert!(cancelled.get("retryAfterSeconds").is_none());
}

#[tokio::test]
async fn the_poll_period_follows_the_release() {
    let (m, _) = module();
    let c = ctx(&m, None);
    c.join(&solo("a", Some("ISLAND"), Some(ISLAND))).await.unwrap();
    let now = m.state.now_ms();
    let candidate = c.poll("a").await.unwrap().unwrap();
    assert_eq!(candidate.status_period_millis(now), 1000);
    m.state.clock().advance(PAST_RELEASE);
    let later = c.poll("a").await.unwrap().unwrap();
    assert_eq!(later.status_period_millis(m.state.now_ms()), 10_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn ring_calls_carry_the_client_address() {
    let temp = TempStore::open().unwrap();
    account(&temp.store, "alice").await;
    let d = director();
    let m = module_with(Some(temp.store.clone()), Settings::default(), &d);
    let home = RecordingHome::new();
    m.state.set_home(home.clone());
    ctx(&m, Some("198.51.100.4"))
        .join(&solo("alice", Some("ISLAND"), Some(CR19)))
        .await
        .unwrap();
    assert_eq!(home.calls(), [format!("submit {CR19} 198.51.100.4")]);
    ctx(&m, Some("198.51.100.4"))
        .join(&solo("alice", Some("CITY"), None))
        .await
        .unwrap();
    assert_eq!(home.calls()[1], "assign_ramsgate 198.51.100.4");
    let before = home.calls().len();
    ctx(&m, None)
        .join(&solo("alice", Some("ISLAND"), Some("CR19_Other")))
        .await
        .unwrap();
    ctx(&m, None).join(&solo("alice", Some("CITY"), None)).await.unwrap();
    assert_eq!(home.calls().len(), before, "{:?}", home.calls());
}
