//! 1.12.0 through the shared progression code (DD8, RU3c): what its hooks and tables change, with the expected
//! answers read from the 1.12.0 reference (`reference/1.12.0/ParadoxBackend/src/routes/progression.ts`, `system.ts`).
//! Unverified against recorded 1.12.0 traffic (DD9).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::path::PathBuf;

use isles_core::areas::progression::Ports;
use isles_core::areas::progression::testkit::{DatabaseConfig, Harness, Options, TestRequest};
use serde_json::json;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/isles-core/tests/golden/progression/game-data")
}

async fn start(dir: &tempfile::TempDir, settings: &[(&str, &str)]) -> Harness {
    let harness = Harness::start(Options {
        version: isles_1_12_0::version(),
        game_data: Some(fixture()),
        database: DatabaseConfig::Sqlite {
            file: dir.path().join("isles.db"),
        },
        settings: settings
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        ports: Ports::default(),
        guard: None,
    })
    .await
    .unwrap();
    harness.account("p1").await.unwrap();
    harness
}

fn gs(method: &str, path: &str) -> TestRequest {
    TestRequest::new(method, path).as_role("gs", None)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hunt_pass_is_the_bare_string_season19() {
    let dir = tempfile::tempdir().unwrap();
    let harness = start(&dir, &[]).await;
    let answer = harness
        .send(&TestRequest::new("GET", "/huntpass/p1").as_role("player", Some("p1")))
        .await
        .unwrap();
    assert_eq!(
        (answer.status, answer.body.as_str()),
        (200, r#"{"code":null,"message":"OK","payload":"season19"}"#)
    );
}

fn report() -> serde_json::Value {
    json!({
        "progress_tracks": [{ "progression_id": "MasteryTrack_PlayerLevel", "progress": 1 }],
        "objectives": [{ "objective_id": "AchievementObjective_Win", "value": 3, "completed_count": 1 }],
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_report_is_captured_and_answered_empty_with_mastery_persistence_off() {
    let dir = tempfile::tempdir().unwrap();
    let harness = start(&dir, &[]).await;
    let answer = harness
        .send(&gs("POST", "/progression/p1").json(&report()))
        .await
        .unwrap();
    assert_eq!(
        (answer.status, answer.body.as_str()),
        (200, r#"{"code":null,"message":"OK","payload":{}}"#)
    );
    let answer = harness
        .send(&TestRequest::new("GET", "/progression/objectives/p1").as_role("player", Some("p1")))
        .await
        .unwrap();
    assert_eq!(answer.body, r#"{"code":null,"message":"OK","payload":[]}"#);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_can_switch_mastery_on_and_objectives_are_the_three_field_rows() {
    let dir = tempfile::tempdir().unwrap();
    let harness = start(&dir, &[("MYSTICPARADOX_MASTERY_PERSISTENCE", "on")]).await;
    let answer = harness
        .send(&gs("POST", "/progression/p1").json(&report()))
        .await
        .unwrap();
    assert_eq!(answer.status, 200);
    assert_eq!(
        answer.payload()["objectives"][0]["objective_id"],
        "AchievementObjective_Win"
    );
    let answer = harness
        .send(&TestRequest::new("GET", "/progression/objectives/p1").as_role("player", Some("p1")))
        .await
        .unwrap();
    assert_eq!(
        answer.body,
        r#"{"code":null,"message":"OK","payload":[{"objective_id":"AchievementObjective_Win","value":3,"completed_count":1}]}"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_active_pass_is_season19_in_grants_and_the_default_list() {
    let dir = tempfile::tempdir().unwrap();
    let harness = start(&dir, &[]).await;
    let grant = |track: &str, id: &str| {
        gs("POST", &format!("/progression/p1/{track}/20")).header("x-mysticparadox-grant-id", id)
    };
    assert_eq!(harness.send(&grant("season19", "a")).await.unwrap().status, 200);
    // season43 is 1.14.7's pass: no approved track here
    assert_eq!(harness.send(&grant("season43", "b")).await.unwrap().status, 400);
    let answer = harness
        .send(&TestRequest::new("GET", "/progression/p1").as_role("player", Some("p1")))
        .await
        .unwrap();
    let tracks = answer.payload();
    let last = tracks.as_array().unwrap().last().unwrap();
    assert_eq!(
        (last["progression_id"].as_str(), last["progress"].as_i64()),
        (Some("season19"), Some(20))
    );
    assert!(
        !tracks
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["progression_id"] == "season43")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reforge_rules_are_off_so_a_prestige_grant_is_not_classified() {
    let dir = tempfile::tempdir().unwrap();
    let harness = start(&dir, &[]).await;
    let grant = gs("POST", "/progression/p1/PrestigeTrack_Weapon_Axe/1").header("x-mysticparadox-grant-id", "p1");
    let answer = harness.send(&grant).await.unwrap();
    assert_eq!(answer.status, 200);
    assert_eq!(answer.payload()["progress"], 1);
    // a host that switches the rules on gets the 1.14.7 refusal
    let dir = tempfile::tempdir().unwrap();
    let harness = start(&dir, &[("MYSTICPARADOX_REFORGE", "on")]).await;
    let grant = gs("POST", "/progression/p1/PrestigeTrack_Weapon_Axe/1").header("x-mysticparadox-grant-id", "p1");
    assert_eq!(harness.send(&grant).await.unwrap().status, 409);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_journey_is_served_as_stored_and_the_single_track_read_is_a_404() {
    let dir = tempfile::tempdir().unwrap();
    let harness = start(&dir, &[]).await;
    let save = json!({ "nodes": { "Gauntlet_Unlock": { "node_id": "Gauntlet_Unlock", "node_status": 2, "objectives": [] } }, "update_version": 2 });
    assert_eq!(
        harness.send(&gs("POST", "/pjm/p1").json(&save)).await.unwrap().status,
        200
    );
    let answer = harness
        .send(&TestRequest::new("GET", "/pjm/p1").as_role("player", Some("p1")))
        .await
        .unwrap();
    assert_eq!(answer.payload()["nodes"]["Gauntlet_Unlock"]["node_status"], 2);
    let answer = harness
        .send(&TestRequest::new("GET", "/progression/p1/ExperienceTrack_Weapon_Axe").as_role("player", Some("p1")))
        .await
        .unwrap();
    assert_eq!((answer.status, answer.body.as_str()), (404, ""));
    let answer = harness
        .send(&TestRequest::new("GET", "/weapon_tracker/p1/X").as_role("player", Some("p1")))
        .await
        .unwrap();
    assert_eq!(answer.status, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn escalation_is_persisted_and_written_on_both_versions() {
    let dir = tempfile::tempdir().unwrap();
    let harness = start(&dir, &[]).await;
    let write = gs("POST", "/escalation/ESC_SEASON_1/p1").json(&json!({ "escalation_level": 3, "next_level_xp": 7 }));
    assert_eq!(harness.send(&write).await.unwrap().status, 200);
    let answer = harness.send(&gs("GET", "/escalation/ESC_SEASON_1/p1")).await.unwrap();
    assert_eq!(
        answer.body,
        r#"{"code":null,"message":"OK","payload":{"escalation_level":3,"next_level_xp":7,"talents_progress":[],"unlock_progress":[],"update_version":1}}"#
    );
}
