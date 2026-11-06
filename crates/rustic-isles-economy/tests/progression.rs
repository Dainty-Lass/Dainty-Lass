// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The progression routes (progression spec §13), black-box through core's `app()`.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{Harness, NOW_MS, Options, PASS, Who, settings};
use rifts_seam::HostWriteKind;
use rustic_isles_auth::{GameAccount, GameLogin};
use rustic_isles_economy::progression::{ProgressionPorts, SessionSupplies};
use serde_json::{Value, json};

const GS: Who = Who::Gameserver;
const AXE: &str = "ExperienceTrack_Weapon_Axe";

fn grant_uri(user: &str, track: &str, amount: &str) -> String {
    format!("/progression/{user}/{track}/{amount}")
}

async fn grant(h: &Harness, who: &Who, user: &str, track: &str, amount: &str, id: &str) -> common::Answer {
    h.send_with(
        "POST",
        &grant_uri(user, track, amount),
        who,
        None,
        &[("x-rustic-isles-grant-id", id)],
    )
    .await
}

#[tokio::test]
async fn reads() {
    let h = Harness::bare().await;
    h.character("p1").await;
    let pass = h.get("/huntpass/p1", &Who::Player("p1")).await.ok();
    assert_eq!(
        pass,
        json!({"code": null, "message": "OK", "payload": {"huntpass_id": PASS, "season": PASS}})
    );
    let rows = h.get("/progression/p1", &Who::Player("p1")).await.ok()["payload"].clone();
    assert_eq!(rows.as_array().unwrap().len(), 24);
    assert_eq!(rows[0]["progression_id"], "MasteryTrack_PlayerLevel");
    assert_eq!(rows[23]["progression_id"], PASS);
    assert_eq!(rows[0]["confirmed_date"], rustic_isles::season::to_iso_string(NOW_MS));
    let sentinel = h.get("/progression/INVALID", &GS).await.ok()["payload"].clone();
    assert_eq!(sentinel.as_array().unwrap().len(), 24);
    h.get("/progression/other", &Who::Player("p1"))
        .await
        .assert(403, r#"{"code":null,"message":"Forbidden","payload":{}}"#);
    h.get("/progression/p1", &Who::Nobody).await.assert_empty(401);
    h.get("/progression/config/x", &Who::Nobody).await.assert_empty(404);
    let one = h
        .get("/progression/p1/ExperienceTrack_PlayerLevel", &Who::Player("p1"))
        .await
        .ok();
    assert_eq!(one["payload"]["progress"], 0);
    let long = "x".repeat(129);
    assert_eq!(
        h.get(&format!("/progression/p1/{long}"), &GS).await.ok()["payload"]["progression_id"],
        long.as_str()
    );
    assert_eq!(
        h.get("/progression/objectives/INVALID", &GS).await.ok()["payload"],
        json!([])
    );
    h.get("/weapon_tracker/p1/x", &Who::Player("p1"))
        .await
        .assert(200, r#"{"code":null,"message":"OK","payload":{"TrackersInfo":[]}}"#);
    assert_eq!(h.get("/weapon_tracker/p2/x", &Who::Player("p1")).await.status, 403);
    let tracked = h
        .get("/progression/tracked_objectives/p1", &Who::Player("p1"))
        .await
        .ok();
    assert_eq!(tracked["payload"]["current_set"], "quest_slayer_links");
    h.post(
        "/progression/tracked_objectives/p1",
        &Who::Player("p1"),
        &json!({"x": 1}),
    )
    .await
    .assert(200, r#"{"code":null,"message":"OK","payload":null}"#);
    h.set_progress("p1", "Custom_Track", 7).await;
    let rows = h.get("/progression/p1", &GS).await.ok()["payload"].clone();
    assert_eq!(rows[24]["progression_id"], "Custom_Track");
}

#[tokio::test]
async fn grants() {
    let h = Harness::bare().await;
    h.character("p1").await;
    h.character("p2").await;
    let first = grant(&h, &GS, "p1", "ExperienceTrack_PlayerLevel", "100", "g1")
        .await
        .ok();
    assert_eq!(
        (
            first["payload"]["progress"].clone(),
            first["payload"]["replayed"].clone()
        ),
        (json!(100), json!(false))
    );
    let again = grant(&h, &GS, "p1", "ExperienceTrack_PlayerLevel", "100", "g1")
        .await
        .ok();
    assert_eq!(
        (
            again["payload"]["progress"].clone(),
            again["payload"]["replayed"].clone()
        ),
        (json!(100), json!(true))
    );
    let conflict = r#"{"code":null,"message":"Conflict","payload":{}}"#;
    grant(&h, &GS, "p1", "ExperienceTrack_PlayerLevel", "50", "g1")
        .await
        .assert(409, conflict);
    grant(&h, &GS, "p2", "ExperienceTrack_PlayerLevel", "100", "g1")
        .await
        .assert(409, conflict);
    let minted = h
        .send("POST", &grant_uri("p1", "ExperienceTrack_PlayerLevel", "5"), &GS, None)
        .await
        .ok();
    assert!(minted["payload"]["grant_id"].as_str().unwrap().starts_with("gs-"));
    h.send(
        "POST",
        &grant_uri("p1", "ExperienceTrack_PlayerLevel", "5"),
        &Who::Player("p1"),
        None,
    )
    .await
    .assert(403, r#"{"code":null,"message":"Forbidden","payload":{}}"#);
    let bad = r#"{"code":null,"message":"Bad Request","payload":{}}"#;
    for id in ["-x", "a b", &"a".repeat(129)] {
        grant(&h, &GS, "p1", "ExperienceTrack_PlayerLevel", "5", id)
            .await
            .assert(400, bad);
    }
    let mixed = h
        .send_with(
            "POST",
            &grant_uri("p1", "ExperienceTrack_PlayerLevel", "5"),
            &GS,
            Some(r#"{"grant_id":"b"}"#),
            &[("x-rustic-isles-grant-id", "a")],
        )
        .await;
    mixed.assert(400, bad);
    let body = h
        .send(
            "POST",
            &grant_uri("p1", "ExperienceTrack_PlayerLevel", "5"),
            &GS,
            Some(r#"{"grantId":"body-1"}"#),
        )
        .await
        .ok();
    assert_eq!(body["payload"]["grant_id"], "body-1");
    let key = h
        .send_with(
            "POST",
            &grant_uri("p1", "ExperienceTrack_PlayerLevel", "5"),
            &GS,
            None,
            &[("idempotency-key", "key-1")],
        )
        .await
        .ok();
    assert_eq!(key["payload"]["grant_id"], "key-1");
    for amount in ["0", "abc", "1.5", "100001"] {
        grant(
            &h,
            &GS,
            "p1",
            "ExperienceTrack_PlayerLevel",
            amount,
            &format!("a-{amount}"),
        )
        .await
        .assert(400, bad);
    }
    let thousand = grant(&h, &GS, "p1", AXE, "1e3", "e3").await.ok();
    assert_eq!(thousand["payload"]["progress"], 1000);
    grant(&h, &GS, "p1", AXE, "100000", "max").await.ok();
    grant(&h, &GS, "INVALID", AXE, "5", "inv").await.assert(400, bad);
    for track in ["Linked_Slayer_Slot_1", "No_Curve_Track", &"x".repeat(129)] {
        grant(&h, &GS, "p1", track, "5", "unapproved").await.assert(400, bad);
    }
    grant(&h, &GS, "p1", PASS, "5", "pass").await.ok();
    grant(&h, &GS, "p1", "PrestigeTrack_Weapon_Axe", "1", "prestige")
        .await
        .assert(409, conflict);
}

#[tokio::test]
async fn test_multiplier() {
    let on = Harness::new(Options {
        settings: settings(&[
            ("DAINTY_SERVER_MODE", "private"),
            ("DAINTY_SERVER_HOST", "100.101.102.103"),
            ("DAINTY_XP_MULTIPLIER", "5"),
        ]),
        ..Options::default()
    })
    .await;
    on.character("p1").await;
    assert_eq!(
        grant(&on, &GS, "p1", AXE, "125", "m1").await.ok()["payload"]["progress"],
        625
    );
    let off = Harness::new(Options {
        settings: settings(&[
            ("DAINTY_SERVER_MODE", "private"),
            ("DAINTY_SERVER_HOST", "203.0.113.7"),
            ("DAINTY_XP_MULTIPLIER", "5"),
        ]),
        ..Options::default()
    })
    .await;
    off.character("p1").await;
    assert_eq!(
        grant(&off, &GS, "p1", AXE, "125", "m1").await.ok()["payload"]["progress"],
        125
    );
}

#[tokio::test]
async fn confirm_and_reforge() {
    let h = Harness::bare().await;
    h.character("p1").await;
    let confirm = |track: &str, rank: &str| format!("/progression/p1/{track}/{rank}/confirm/public");
    h.set_progress("p1", AXE, 1500).await;
    let ok = h.send("POST", &confirm(AXE, "1"), &GS, None).await.ok();
    assert_eq!(ok["payload"]["confirmed_fremium_rank"], 1);
    h.send("POST", &confirm(AXE, "0"), &GS, None).await.ok();
    assert_eq!(h.track("p1", AXE).await.unwrap().confirmed_fremium_rank, 1);
    h.send("POST", &confirm(AXE, "2"), &GS, None).await.assert(
        409,
        r#"{"code":null,"message":"Rejected: insufficient_xp","payload":{}}"#,
    );
    h.send("POST", &confirm("ExperienceTrack_PlayerLevel", "4"), &GS, None)
        .await
        .assert(
            400,
            r#"{"code":null,"message":"Rejected: rank_out_of_range","payload":{}}"#,
        );
    h.send("POST", &confirm("Nope", "1"), &GS, None)
        .await
        .assert(400, r#"{"code":null,"message":"Rejected: unknown_track","payload":{}}"#);
    for rank in ["x", "1.5"] {
        assert_eq!(h.send("POST", &confirm(AXE, rank), &GS, None).await.status, 400);
    }
    // A mastery rank one short syncs; two short does not
    h.send("POST", &confirm("MasteryTrack_Behemoth", "1"), &GS, None)
        .await
        .ok();
    assert_eq!(h.track("p1", "MasteryTrack_Behemoth").await.unwrap().progress, 1);
    assert_eq!(
        h.send("POST", &confirm("MasteryTrack_Behemoth", "3"), &GS, None)
            .await
            .status,
        409
    );
    // An open reforge allows one +1
    h.set_progress("p1", AXE, 3000).await;
    h.send("DELETE", &format!("/progression/p1/{AXE}"), &GS, None)
        .await
        .ok();
    grant(&h, &GS, "p1", "PrestigeTrack_Weapon_Axe", "1", "plus-1")
        .await
        .ok();
    assert_eq!(
        grant(&h, &GS, "p1", "PrestigeTrack_Weapon_Axe", "1", "plus-2")
            .await
            .status,
        409
    );
    let replay = grant(&h, &GS, "p1", "PrestigeTrack_Weapon_Axe", "1", "plus-1")
        .await
        .ok();
    assert_eq!(replay["payload"]["replayed"], true);
    // The banked restore of the remaining amount closes it (the +1 is done)
    grant(&h, &GS, "p1", AXE, "1500", "restore").await.ok();
    assert!(!h.track("p1", AXE).await.unwrap().reforge.unwrap().open);
    // The 15-minute duplicate window is judged on the system clock (spec §15 item 3), and the reset's stamp is the
    // replay clock's, long past here: the retry is an ordinary grant
    assert_eq!(grant(&h, &GS, "p1", AXE, "1500", "restore-2").await.status, 200);
}

#[tokio::test]
async fn stale_repair_on_load() {
    let h = Harness::bare().await;
    h.character("p1").await;
    h.set_progress("p1", AXE, 3200).await;
    h.send("DELETE", &format!("/progression/p1/{AXE}"), &GS, None)
        .await
        .ok();
    let later = (NOW_MS + 180_000).to_string();
    let extra = [("x-isles-replay-now-ms", later.as_str())];
    let load = || h.send_with("GET", "/progression/p1", &GS, None, &extra);
    load().await.ok();
    assert_eq!(h.track("p1", AXE).await.unwrap().progress, 200);
    assert_eq!(h.track("p1", "PrestigeTrack_Weapon_Axe").await.unwrap().progress, 1);
    load().await.ok();
    assert_eq!(h.track("p1", AXE).await.unwrap().progress, 200);
}

#[tokio::test]
async fn reports() {
    let h = Harness::bare().await;
    h.character("p1").await;
    let sword = "MasteryObjective_Weapon_Sword_Generic_Kills1";
    let report = json!({
        "objectives": [{"objective_id": sword, "value": 3, "completed_count": 1}],
        "progress_tracks": [{"progression_id": "MasteryTrack_PlayerLevel", "progress": 1}, {"progression_id": "MasteryTrack_Weapon_Sword", "progress": 1}],
    });
    let first = h.post("/progression/p1", &GS, &report).await.ok()["payload"].clone();
    let tracks: Vec<_> = first["progress_tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["progression_id"].clone(), t["progress"].clone()))
        .collect();
    assert_eq!(
        tracks,
        vec![
            (json!("MasteryTrack_PlayerLevel"), json!(1)),
            (json!("MasteryTrack_Weapon_Sword"), json!(1))
        ]
    );
    assert_eq!(
        first["objectives"][0]["created_date"],
        rustic_isles::season::to_iso_string(NOW_MS)
    );
    h.post("/progression/p1", &GS, &report).await.ok();
    assert_eq!(h.track("p1", "MasteryTrack_PlayerLevel").await.unwrap().progress, 1);
    h.post("/progression/p1", &Who::Player("p1"), &report)
        .await
        .assert(200, r#"{"code":null,"message":"OK","payload":{}}"#);
    let bad = r#"{"code":null,"message":"Bad Request","payload":{}}"#;
    h.post("/progression/p1", &GS, &json!({"progress_tracks": 5}))
        .await
        .assert(400, bad);
    h.post(
        "/progression/p1",
        &GS,
        &json!({"objectives": [{"objective_id": "x", "value": -1}]}),
    )
    .await
    .assert(400, bad);
    h.post(
        "/progression/p1",
        &GS,
        &json!({"objectives": [{"objective_id": "a b", "value": 1}]}),
    )
    .await
    .assert(400, bad);
    h.post("/progression/INVALID", &GS, &report)
        .await
        .assert(200, r#"{"code":null,"message":"OK","payload":{}}"#);
    let rows = h.get("/progression/objectives/p1", &Who::Player("p1")).await.ok()["payload"].clone();
    assert_eq!(rows[0]["objective_id"], sword);
}

#[tokio::test]
async fn escalation() {
    let tuning = json!({"seasons": {"s1": {
        "maxLevel": 10, "xpToReachLevel": [10, 20, 30, 40, 50, 60, 70, 80, 90, 100],
        "unlocks": {"r1": {"level": 2}}, "talents": {"a": {"pointsToUnlock": 0, "pointCosts": [1]}},
    }}});
    let h = Harness::new(Options {
        files: vec![("escalation_tuning.json", tuning)],
        ..Options::default()
    })
    .await;
    h.character("player-1").await;
    h.character("p2").await;
    let zero = h.get("/escalation/s1/player-1", &Who::Player("player-1")).await.ok();
    assert_eq!(
        zero["payload"],
        json!({"escalation_level": 0, "next_level_xp": 0, "talents_progress": [], "unlock_progress": [], "update_version": 0})
    );
    h.get("/escalation/s1/player-1", &Who::Player("p2"))
        .await
        .assert_empty(403);
    h.post(
        "/escalation/s1/player-1",
        &GS,
        &json!({"escalation_level": 9, "next_level_xp": 50}),
    )
    .await
    .ok();
    let lower = h
        .post("/escalation/s1/player-1", &GS, &json!({"escalation_level": 4}))
        .await
        .ok();
    assert_eq!(
        (
            lower["payload"]["escalation_level"].clone(),
            lower["payload"]["update_version"].clone()
        ),
        (json!(9), json!(2))
    );
    h.post(
        "/escalation/s1/player-1",
        &Who::Player("player-1"),
        &json!({"next_level_xp": 1}),
    )
    .await
    .ok();
    h.post("/escalation/s1/player-1", &Who::Player("p2"), &json!({}))
        .await
        .assert_empty(403);
    let host = Who::Host("cap-ok");
    h.post("/escalation/unknown/player-1", &host, &json!({})).await.assert(
        403,
        r#"{"code":"ESCALATION_SEASON_UNKNOWN","message":"This escalation season is not known.","payload":null}"#,
    );
    let host_read = h.get("/escalation/s1/player-1", &host).await.ok();
    assert_eq!(host_read["payload"]["escalation_level"], 9);
    h.post("/escalation/s1/player-1", &host, &json!({"escalation_level": 3}))
        .await
        .ok();
    let tags = h.guard.tags();
    let (kind, _, summary) = tags.last().unwrap();
    assert_eq!(*kind, HostWriteKind::Escalation);
    assert_eq!(
        (
            summary["season"].clone(),
            summary["levelAfter"].clone(),
            summary["levelXpRegressionIgnored"].clone()
        ),
        (json!("s1"), json!(9), json!(true))
    );
}

#[tokio::test]
async fn encountered_and_breadcrumbs() {
    let h = Harness::bare().await;
    let c = h.character("p1").await;
    let other = h.character("p2").await;
    let me = Who::Player("p1");
    h.post(
        &format!("/encountered-content/{c}"),
        &me,
        &json!({"content_type": 3, "content_id": "a"}),
    )
    .await
    .ok();
    h.post(
        &format!("/encountered-content/{c}"),
        &me,
        &json!({"content_type": 3, "content_id": "a"}),
    )
    .await
    .ok();
    h.post(
        &format!("/encountered-content/{c}"),
        &me,
        &json!({"content_type": 1, "content_id": "b"}),
    )
    .await
    .ok();
    h.post(&format!("/encountered-content/{c}"), &me, &json!({"content_id": "b"}))
        .await
        .ok();
    let three = h.get(&format!("/encountered-content/{c}/3"), &me).await.ok();
    assert_eq!(
        three["payload"],
        json!({"content_types": [{"content": ["a", "a"], "content_type": 3}], "success": true})
    );
    assert_eq!(
        h.get(&format!("/encountered-content/{c}/9"), &me).await.ok()["payload"]["content_types"],
        json!([])
    );
    assert_eq!(
        h.get(&format!("/encountered-content/{c}/abc"), &me).await.ok()["payload"]["content_types"],
        json!([])
    );
    let query = h
        .post(
            &format!("/encountered-content/query/{c}"),
            &me,
            &json!({"content_types": [3, 1, "1"]}),
        )
        .await
        .ok();
    assert_eq!(
        query["payload"]["content_types"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["content_type"].clone())
            .collect::<Vec<_>>(),
        [json!(1), json!(3)]
    );
    h.send("POST", &format!("/encountered-content/query/{c}"), &me, None)
        .await
        .assert_empty(500);
    assert_eq!(
        h.post(&format!("/encountered-content/{c}/query"), &me, &json!({}))
            .await
            .ok()["payload"]["content_types"],
        json!([])
    );
    assert_eq!(
        h.get(&format!("/encountered-content/{other}/3"), &me).await.json()["code"],
        "forbidden"
    );
    h.post(
        &format!("/encountered-content/{other}"),
        &me,
        &json!({"content_type": 1, "content_id": "x"}),
    )
    .await
    .assert_empty(403);
    let crumbs = h.get(&format!("/breadcrumbs/{c}"), &me).await.ok();
    assert_eq!(crumbs["payload"], json!({"breadcrumbs": [], "updateVersion": 0}));
    let wrote = h
        .post(&format!("/breadcrumbs/{c}"), &me, &json!({"breadcrumbs": [{"x": 1}]}))
        .await
        .ok();
    assert_eq!(wrote["payload"], json!({"breadcrumbs": [{"x": 1}]}));
    let read = h.get(&format!("/breadcrumbs/{c}"), &me).await.ok();
    assert_eq!(read["payload"], json!({"breadcrumbs": [{"x": 1}], "updateVersion": 0}));
    h.get(&format!("/breadcrumbs/{other}"), &me).await.assert_empty(403);
}

#[tokio::test]
async fn journey() {
    let defs = json!({
        "Start": {"childNodes": ["Gauntlet_Unlock", "Reward_Node"], "rewards": ["None"]},
        "Reward_Node": {"childNodes": ["None"], "rewards": ["PART_REWARD"]},
        "Gauntlet_Unlock": {"childNodes": []},
    });
    let file =
        json!({"code": null, "message": "OK", "payload": {"nodes": {"Start": {"node_id": "Start", "node_status": 0}}}});
    let h = Harness::new(Options {
        files: vec![
            ("slayers_path_definitions.json", defs),
            ("slayers_path.json", file.clone()),
        ],
        ..Options::default()
    })
    .await;
    let c = h.character("p1").await;
    assert_eq!(h.get("/pjm", &Who::Player("p1")).await.ok(), file);
    let save = json!({"nodes": {"Start": {"node_status": 2}, "Gauntlet_Unlock": {"node_status": 2}, "Reward_Node": {"node_status": 2}}, "update_version": 2});
    let saved = h.post("/pjm/p1", &GS, &save).await.ok();
    assert_eq!(saved["payload"]["nodes"]["Gauntlet_Unlock"]["node_status"], 0);
    assert_eq!(saved["payload"]["update_version"], 2);
    // The reward node granted its item once, with the stable ids
    let items = h.instanced(&c).await;
    assert_eq!(
        items
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["catalogId"] == "PART_REWARD")
            .count(),
        1
    );
    let read = h.get("/pjm/p1", &Who::Player("p1")).await.ok();
    assert_eq!(read["payload"]["nodes"]["Gauntlet_Unlock"]["node_status"], 0);
    // A stale save keeps the stored nodes and version; the masked echo never relocks
    let stale = json!({"nodes": {"Start": {"node_status": 0}, "New": {"node_status": 2}}, "update_version": 1});
    let kept = h.post("/pjm/p1", &GS, &stale).await.ok();
    assert_eq!(
        (
            kept["payload"]["update_version"].clone(),
            kept["payload"]["nodes"]["Start"]["node_status"].clone()
        ),
        (json!(2), json!(2))
    );
    let echo = json!({"nodes": {"Gauntlet_Unlock": {"node_status": 0}}, "update_version": 3});
    h.post("/pjm/p1", &GS, &echo).await.ok();
    let account = h.account_id("p1").await;
    let row = h
        .store
        .read(move |c, r| r.player_journey.find(c, &account))
        .await
        .unwrap()
        .unwrap();
    assert!(row.nodes.contains("\"Gauntlet_Unlock\":{\"node_status\":2}"));
    assert_eq!(h.post("/pjm/p1", &Who::Player("p1"), &echo).await.status, 403);
    // A host may unlock only what the graph allows
    let host = h
        .post(
            "/pjm/p1",
            &Who::Host("cap-ok"),
            &json!({"nodes": {"Ghost": {"node_status": 2}}, "update_version": 9}),
        )
        .await
        .ok();
    assert_eq!(host["payload"]["nodes"]["Ghost"]["node_status"], 0);
    let tags = h.guard.tags();
    assert_eq!(tags.last().unwrap().2["stripped"], json!(["Ghost:unknown-node"]));
}

#[tokio::test]
async fn host_limits() {
    let h = Harness::bare().await;
    h.character("player-1").await;
    let host = Who::Host("cap-ok");
    grant(&h, &host, "player-1", AXE, "1000", "h1").await.ok();
    grant(&h, &host, "player-1", AXE, "1001", "h2").await.assert(
        403,
        r#"{"code":"HOST_WRITE_LIMIT","message":"This write is beyond what a hosted server may report.","payload":null}"#,
    );
    assert_eq!(
        grant(&h, &host, "player-1", "MasteryTrack_Behemoth", "1", "h3")
            .await
            .status,
        403
    );
    assert_eq!(
        grant(&h, &host, "player-1", "PrestigeTrack_Weapon_Axe", "1", "h4")
            .await
            .status,
        409
    );
    assert_eq!(grant(&h, &host, "player-1", PASS, "201", "h5").await.status, 403);
    *h.guard.observe.lock().unwrap() = true;
    grant(&h, &host, "player-1", AXE, "1001", "h6").await.ok();
    assert_eq!(h.guard.notes.lock().unwrap()[0].0, "xp");
    *h.guard.observe.lock().unwrap() = false;
    h.guard.probe_refusals.lock().unwrap().push(HostWriteKind::Xp);
    grant(&h, &host, "player-1", PASS, "10", "h7").await.ok();
    let tags = h.guard.tags();
    assert_eq!(tags.last().unwrap().0, HostWriteKind::HuntPassXp);
    // Filling the player track, then granting again
    h.guard.probe_refusals.lock().unwrap().clear();
    for i in 0..3 {
        grant(
            &h,
            &host,
            "player-1",
            "ExperienceTrack_PlayerLevel",
            "1000",
            &format!("fill-{i}"),
        )
        .await
        .ok();
    }
    assert_eq!(
        grant(&h, &host, "player-1", "ExperienceTrack_PlayerLevel", "1", "over")
            .await
            .status,
        403
    );
    assert_eq!(
        grant(&h, &host, "player-1", "ExperienceTrack_PlayerLevel", "1000", "fill-2")
            .await
            .ok()["payload"]["replayed"],
        true
    );
    *h.guard.fail_authorize.lock().unwrap() = true;
    grant(&h, &host, "player-1", AXE, "1", "h8").await.assert_empty(500);
}

#[tokio::test]
async fn session_supplies() {
    let h = Harness::bare().await;
    let c = h.character("p1").await;
    let file = h.data.path().join("supplies.json");
    std::fs::write(&file, r#"{"accounts":{"p1":{"STACK_STARTER":5,"MAT_TEST":3}}}"#).unwrap();
    let shared = {
        // The setting points at the file; a new module reads it
        let h2 = Harness::new(Options {
            settings: settings(&[("DAINTY_SESSION_SUPPLIES_FILE", file.to_str().unwrap())]),
            ..Options::default()
        })
        .await;
        let c2 = h2.character("p1").await;
        let account = h2.account("p1").await.unwrap();
        let seam = SessionSupplies::new(h2.shared.clone(), ProgressionPorts::default());
        let login = GameAccount {
            id: account.id.to_string(),
            user_id: "p1".into(),
            name: "p1".into(),
            notes: 0,
        };
        seam.session_supplies(&login).await;
        let stacked = h2.stacked(&c2).await;
        assert!(
            stacked
                .as_array()
                .unwrap()
                .contains(&json!({"catalogId": "MAT_TEST", "quantity": 3}))
        );
        assert!(
            stacked
                .as_array()
                .unwrap()
                .contains(&json!({"catalogId": "STACK_STARTER", "quantity": 5}))
        );
        h2.shared.clone()
    };
    drop(shared);
    // Without the setting nothing happens
    let before: Value = h.stacked(&c).await;
    let account = h.account("p1").await.unwrap();
    SessionSupplies::new(h.shared.clone(), ProgressionPorts::default())
        .session_supplies(&GameAccount {
            id: account.id.to_string(),
            user_id: "p1".into(),
            name: "p1".into(),
            notes: 0,
        })
        .await;
    assert_eq!(h.stacked(&c).await, before);
}
