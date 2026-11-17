// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Director end to end over the real launcher and the fake game server (spec §14.2). Every test is bounded
//! and kills every tree however it ends (the rig's drop).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::time::Duration;

use common::{FAKE, MY_IP, rig, send, udp_free, wait_for};
use serde_json::{Value, json};

const BOUND: Duration = Duration::from_secs(180);

async fn bounded(test: impl Future<Output = ()>) {
    tokio::time::timeout(BOUND, test)
        .await
        .expect("the test took longer than 180 s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn before_boot_and_http() {
    bounded(async {
        let r = rig(&[], |_| {}).await;
        let ready = r.get("/health/ready").await;
        assert_eq!(
            (ready.status, ready.body.as_str()),
            (503, r#"{"ok":false,"service":"deployserver"}"#)
        );
        assert_eq!(ready.header("content-type"), Some("application/json; charset=utf-8"));
        assert_eq!(
            r.get("/api/hub-status").await.body,
            r#"{"ramsgate":null,"trainingDojo":null}"#
        );
        assert_eq!(r.get("/api/instances").await.body, "[]");
        let city = r.matchmake(json!({"GameMode": "CITY"})).await;
        assert_eq!(
            (city.status, city.body.as_str()),
            (500, r#"{"error":"matchmaking_failed"}"#)
        );
        assert_eq!(
            r.matchmake(json!({"GameMode": "RAID", "HuntId": "x"})).await.status,
            500
        );
        // A form body with no content type is no body
        let form = send(
            r.address,
            "POST",
            "/api/matchmaker/handle-matchmaking-for-player",
            &[],
            Some(b"GameMode=CITY"),
        )
        .await;
        assert_eq!(form.status, 500);

        let nope = r.get("/nope?x=1").await;
        assert_eq!(nope.status, 404);
        assert!(nope.body.contains("<pre>Cannot GET /nope</pre>"), "{}", nope.body);
        assert_eq!(nope.header("content-type"), Some("text/html; charset=utf-8"));
        assert_eq!(nope.header("x-rustic-isles-request-id").map(str::len), Some(36));
        let kept = send(
            r.address,
            "GET",
            "/api/instances",
            &[("x-rustic-isles-request-id", "client-id-0001")],
            None,
        )
        .await;
        assert_eq!(kept.header("x-rustic-isles-request-id"), Some("client-id-0001"));
        let short = send(
            r.address,
            "GET",
            "/api/instances",
            &[("x-rustic-isles-request-id", "short")],
            None,
        )
        .await;
        assert_eq!(short.header("x-rustic-isles-request-id").map(str::len), Some(36));
        let post = send(r.address, "POST", "/api/instances", &[], None).await;
        assert_eq!(post.status, 404);
        assert!(post.body.contains("Cannot POST /api/instances"));
        let head = send(r.address, "HEAD", "/api/instances", &[], None).await;
        assert_eq!((head.status, head.body.as_str()), (200, ""));
        let json_type = [("content-type", "application/json")];
        let path = "/api/matchmaker/handle-matchmaking-for-player";
        assert_eq!(
            send(r.address, "POST", path, &json_type, Some(b"{oops")).await.status,
            400
        );
        let big = format!("{{\"x\":\"{}\"}}", "a".repeat(101 * 1024));
        assert_eq!(
            send(r.address, "POST", path, &json_type, Some(big.as_bytes()))
                .await
                .status,
            413
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn boot_matchmaking_instances_and_ports() {
    bounded(async {
        let r = rig(&[], |_| {}).await;
        let (first, last) = (r.range.first, r.range.last);
        r.director.startup().await;
        let ready = r.get("/HEALTH/Ready/").await;
        assert_eq!((ready.status, ready.body.as_str()), (200, r#"{"ok":true,"service":"deployserver"}"#));
        let hubs = r.get("/api/hub-status").await.json();
        assert_eq!(hubs["ramsgate"]["port"], last);
        assert_eq!(hubs["ramsgate"]["host"], MY_IP);
        assert_eq!(hubs["ramsgate"]["ready"], true);
        assert!(hubs["ramsgate"]["processId"].is_u64());
        assert_eq!(hubs["trainingDojo"]["port"], last - 1);
        let args = r.fake_args(last, "ramsgate");
        let port = last.to_string();
        let address = format!("{MY_IP}:{last}");
        let expect: Vec<&str> = vec![
            "gameserver-key",
            &port,
            "/Game/Maps/ramsgate/ramsgate_01_persistent",
            "NO_BEHEMOTH",
            "NO_MM_HUNTID",
            "NO_EXPECTED_PLAYERS",
            &address,
            "-EpicPortal",
            "-server",
            "-nullrhi",
            "-warp",
            "-nosound",
            "-RepDriverEnable",
            "-LogCmds=\"LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off\"",
        ];
        assert_eq!(args["args"], json!(expect));
        assert_eq!(args["inherited"], "30");
        assert_eq!(args["launchId"].as_str().map(str::len), Some(36));
        assert_eq!(args["trialsWeek"], Value::Null);

        let city = r.matchmake(json!({"GameMode": "CITY"})).await;
        assert_eq!(city.body, format!(r#"{{"host":"{MY_IP}","port":{last}}}"#));
        assert_eq!(city.header("x-rustic-isles-request-id").map(str::len), Some(36));
        let dojo = r.matchmake(json!({"GameMode": "SHARED", "HuntId": "ShatteredIsles_TrainingDojo"})).await;
        assert_eq!(dojo.json()["port"], last - 1);

        let travel = "/Game/Maps/test/x?MonsterClass=/Game/Monsters/a/fulgar_bp.fulgar_bp_C?HuntID=Arena_MatchmakerHunt_Hard_042";
        let island = r
            .matchmake(json!({"GameMode": "ISLAND", "GameArgs": travel, "HuntId": "Plain_Hunt", "ExpectedPlayers": ["acc-1"]}))
            .await;
        assert_eq!(island.json()["port"], first);
        let args = r.fake_args(first, "fulgar_bp");
        assert_eq!(args["args"][2], "/Game/Maps/test/x");
        assert_eq!(args["args"][3], "/Game/Monsters/a/fulgar_bp.fulgar_bp_C");
        assert_eq!(args["args"][4], "Arena_MatchmakerHunt_Hard_042");
        assert_eq!(args["args"][5], "acc-1:Plain_Hunt");
        assert_eq!(args["trialsWeek"], "042");

        let boss = r
            .matchmake(json!({"GameMode": "SHARED", "HuntId": "Boss_Hunt", "ExpectedPlayers": ["acc-2", "acc-3"]}))
            .await;
        assert_eq!(boss.json()["port"], first + 1);
        let args = r.fake_args(first + 1, "boss_bp");
        assert_eq!(args["args"][2], "/Game/Maps/test/boss");
        assert_eq!(args["args"][3], "/Game/Monsters/test/boss_bp.boss_bp_C");
        assert_eq!(args["args"][4], "MM_Boss");
        assert_eq!(args["args"][5], "acc-2:Boss_Hunt,acc-3:Boss_Hunt");
        let third = r
            .matchmake(json!({"GameMode": "SHARED", "HuntId": "Plain_Hunt", "ExpectedPlayers": ["acc-4"]}))
            .await;
        assert_eq!(third.status, 500);
        assert!(!r.logs().join(format!("port{}_hunt.log", first + 2)).exists());

        let instances = r.get("/api/instances").await.json();
        let ids: Vec<&str> = instances.as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap()).collect();
        assert_eq!(
            ids,
            [format!("city-{last}"), format!("dojo-{}", last - 1), format!("hunt-{first}"), format!("hunt-{}", first + 1)]
        );
        let boss = &instances[3];
        assert_eq!(boss["players"], 2);
        assert_eq!(boss["maxPlayers"], 4);
        assert_eq!(boss["playerIds"], json!(["acc-2", "acc-3"]));
        assert_eq!(boss["behemoth"], "/Game/Monsters/test/boss_bp.boss_bp_C");
        assert_eq!(instances[0]["maxPlayers"], 0);
        assert!(instances[0].get("processId").is_none());

        // A stopped hunt leaves the list, and the next hunt takes the oldest free port (FIFO)
        r.stop_server(first);
        assert!(wait_for(60, || !r.listed_ports().contains(&first)).await);
        let next = r
            .matchmake(json!({"GameMode": "SHARED", "HuntId": "Plain_Hunt", "ExpectedPlayers": ["acc-5"]}))
            .await;
        assert_eq!(next.json()["port"], first + 2);
        assert!(wait_for(10, || r.log(first, "fulgar_bp").contains("=== Gameserver process exited")).await);
        let log = r.log(first, "fulgar_bp");
        assert!(log.starts_with("=== Gameserver launch "), "{log}");
        assert!(log.contains("RUSTIC_ISLES_GAMESERVER_READY launchId="));
        assert!(log.contains(&format!("=== Gameserver process exited port={first} label=fulgar_bp pid=")));

        r.director.shutdown().await;
        assert!(r.director.gameservers().is_empty());
        assert_eq!(r.director.port_counts().1, 0);
        for port in r.range.first..=r.range.last {
            assert!(wait_for(10, || udp_free(port)).await, "port {port} still held");
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_starts_and_training_on_demand() {
    bounded(async {
        let extra = [
            ("RAMSGATE_HUB", "off"),
            ("TRAINING_HUB", "off"),
            ("GAMESERVER_READY_TIMEOUT_MS", "1500"),
        ];
        let r = rig(&extra, |_| {}).await;
        let first = r.range.first;
        r.director.startup().await;
        assert!(r.director.gameservers().is_empty());
        let island = |map: &str| json!({"GameMode": "ISLAND", "GameArgs": map});
        assert_eq!(r.matchmake(island("/Game/Maps/crash")).await.status, 500);
        assert!(wait_for(10, || r.log(first, "hunt").contains("code=3")).await);
        assert_eq!(r.matchmake(island("/Game/Maps/silent")).await.status, 500);
        assert!(wait_for(30, || r.director.port_counts() == (6, 0)).await);
        assert_eq!(r.matchmake(island("/Game/Maps/test/ok")).await.status, 200);
        assert_eq!(r.matchmake(json!({"GameMode": "CITY"})).await.status, 500);
        let dojo = r
            .matchmake(json!({"GameMode": "SHARED", "HuntId": "ShatteredIsles_TrainingDojo"}))
            .await;
        assert_eq!(dojo.json()["port"], r.range.last - 1);
        r.director.shutdown().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_missing_executable() {
    bounded(async {
        let missing = std::env::temp_dir().join("rustic-isles-no-such-gameserver.exe");
        let missing = missing.display().to_string();
        let extra = [("GAMESERVER_BINARY_PATH", missing.as_str())];
        let r = rig(&extra, |c| c.hub_backoff_ms = Some(vec![100])).await;
        r.director.startup().await;
        for _ in 0..2 {
            let hunt = json!({"GameMode": "ISLAND", "GameArgs": "/Game/Maps/test/x"});
            assert_eq!(r.matchmake(hunt).await.status, 500);
        }
        assert!(r.director.gameservers().is_empty());
        assert_eq!(r.get("/api/instances").await.body, "[]");
        tokio::time::timeout(Duration::from_secs(30), r.director.shutdown())
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dead_hub_is_restarted() {
    bounded(async {
        let r = rig(&[("TRAINING_HUB", "off")], |c| c.hub_backoff_ms = Some(vec![100])).await;
        r.director.startup().await;
        let old = r.director.ramsgate().current().unwrap();
        old.terminate().unwrap();
        assert!(wait_for(30, || r.director.ramsgate().current().as_ref() != Some(&old)).await);
        // Down, or already back on a new process
        let restarted = wait_for(60, || r.director.ramsgate().current().is_some_and(|c| c.alive())).await;
        assert!(restarted);
        let city = r.matchmake(json!({"GameMode": "CITY"})).await;
        assert_eq!(city.json()["port"], r.range.last);
        r.director.shutdown().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_ready_pipes_and_trees() {
    bounded(async {
        let extra = [
            ("RAMSGATE_HUB", "off"),
            ("TRAINING_HUB", "off"),
            ("MAX_CONCURRENT_HUNT_SERVERS", "4"),
        ];
        let r = rig(&extra, |_| {}).await;
        let first = r.range.first;
        r.director.startup().await;
        let island = |map: &str| json!({"GameMode": "ISLAND", "GameArgs": map});
        assert_eq!(r.matchmake(island("/Game/Maps/test/steady")).await.status, 200);
        assert_eq!(r.matchmake(island("/Game/Maps/flaky")).await.status, 200);
        assert!(wait_for(30, || r.log(first + 1, "hunt").contains("code=5")).await);
        assert!(wait_for(10, || r.listed_ports() == [first]).await);

        assert_eq!(r.matchmake(island("/Game/Maps/chatty")).await.status, 200);
        let chatty = r.director.gameservers().last().unwrap().clone();
        assert!(wait_for(60, || r.log(chatty.port, "hunt").contains("CHATTY_DONE")).await);
        assert!(r.log(chatty.port, "hunt").len() > 3 * 1024 * 1024);

        assert_eq!(r.matchmake(island("/Game/Maps/grandchild")).await.status, 200);
        let tree = r.director.gameservers().last().unwrap().clone();
        let held = wait_for(20, || r.log(tree.port, "hunt").contains("FAKE_GRANDCHILD_UDP=")).await;
        assert!(held);
        let log = r.log(tree.port, "hunt");
        let grandchild: u16 = log
            .split("FAKE_GRANDCHILD_UDP=")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(!udp_free(tree.port) && !udp_free(grandchild));
        // A stop of the server alone frees the grandchild's port too
        tree.terminate().unwrap();
        assert!(wait_for(30, || udp_free(tree.port) && udp_free(grandchild)).await);
        r.director.shutdown().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_hunt_reuse() {
    bounded(async {
        let extra = [
            ("RAMSGATE_HUB", "off"),
            ("TRAINING_HUB", "off"),
            ("ENABLE_PUBLIC_HUNT_REUSE", "1"),
        ];
        let r = rig(&extra, |_| {}).await;
        r.director.startup().await;
        let a = r
            .matchmake(json!({"GameMode": "SHARED", "HuntId": "Plain_Hunt", "ExpectedPlayers": ["acc-1"]}))
            .await;
        let b = r
            .matchmake(json!({"GameMode": "SHARED", "HuntId": "Plain_Hunt", "ExpectedPlayers": ["acc-2"]}))
            .await;
        assert_eq!(a.json()["port"], b.json()["port"]);
        let instances = r.get("/api/instances").await.json();
        assert_eq!(instances[0]["playerIds"], json!(["acc-1", "acc-2"]));
        r.director.shutdown().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_kick_reaches_every_server_as_a_file_beside_the_game() {
    bounded(async {
        let extra = [("RAMSGATE_HUB", "off"), ("TRAINING_HUB", "off")];
        let r = rig(&extra, |_| {}).await;
        r.director.startup().await;
        let island = json!({"GameMode": "ISLAND", "GameArgs": "/Game/Maps/test/ok"});
        assert_eq!(r.matchmake(island).await.status, 200);
        let port = *r.listed_ports().first().unwrap();
        let file = std::path::Path::new(FAKE)
            .parent()
            .unwrap()
            .join(format!("rustic_isles_kick_port{port}.txt"));
        _ = std::fs::remove_file(&file);
        assert_eq!(r.director.kick("9c65cf9f-kick"), 1);
        assert_eq!(r.director.kick("has space"), 0);
        let text = std::fs::read_to_string(&file).unwrap();
        _ = std::fs::remove_file(&file);
        let (at, id) = text.trim_end().split_once(' ').unwrap();
        assert!(at.parse::<u64>().is_ok());
        assert_eq!((id, text.lines().count()), ("9c65cf9f-kick", 1));
        r.director.shutdown().await;
    })
    .await;
}
