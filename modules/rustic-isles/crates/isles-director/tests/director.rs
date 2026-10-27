#![allow(clippy::unwrap_used, clippy::expect_used)] // tests

//! RU6's verify, on both versions' real `DIRECTOR` configs: Ramsgate, the Training Grounds and a hunt start
//! and stop through the HTTP API with a fake game server (`isles-fake-gameserver`) and the real launcher,
//! so the process tree handling runs for real on this OS. No game and no SDK.

mod common;

use std::time::Duration;

use common::{Harness, V1120, V1147, Version, bounded, send, start, start_with, udp_free};
use serde_json::{Value, json};

fn strings(value: &Value) -> Vec<&str> {
    value.as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect()
}

/// Boots the hubs and waits as `Startup` does.
async fn boot(d: &Harness) {
    d.boot().await;
}

async fn serves_the_hubs_and_hunts(version: Version) {
    let d = start(version, &[]).await;
    let (begin, end) = (d.begin, d.end);
    // Before the boot: not ready, nothing to list, no hub.
    let health = d.get("/health/ready").await;
    assert_eq!(
        (health.status, health.text.as_str()),
        (503, r#"{"ok":false,"service":"deployserver"}"#)
    );
    assert_eq!(health.header("content-type"), Some("application/json; charset=utf-8"));
    assert_eq!(
        d.get("/api/hub-status").await.text,
        r#"{"ramsgate":null,"trainingDojo":null}"#
    );
    assert_eq!(d.get("/api/instances").await.text, "[]");
    let city = d.matchmake(json!({"GameMode": "CITY"})).await;
    assert_eq!(
        (city.status, city.text.as_str()),
        (500, r#"{"error":"matchmaking_failed"}"#)
    );

    boot(&d).await;
    assert_eq!(
        d.get("/HEALTH/Ready/").await.text,
        r#"{"ok":true,"service":"deployserver"}"#,
        "Express matching"
    );
    let status = d.get("/api/hub-status").await.json();
    assert!(
        !status["ramsgate"].is_null(),
        "Ramsgate is down after the boot
{}",
        d.diagnostics().await
    );
    assert_eq!(status["ramsgate"]["port"], end);
    assert_eq!(status["ramsgate"]["host"], "100.101.102.103");
    assert_eq!(status["trainingDojo"]["port"], end - 1);
    assert_eq!(status["ramsgate"]["ready"], true);
    assert!(status["ramsgate"]["processId"].as_u64().is_some_and(|pid| pid > 0));

    // The hubs' command lines: the map and the NO_ placeholders, then the version's switches.
    let ramsgate = d.args(end).await;
    let args = strings(&ramsgate["args"]);
    assert_eq!(
        args[..7],
        [
            "gameserver-key",
            &end.to_string(),
            "/Game/Maps/ramsgate/ramsgate_01_persistent",
            "NO_BEHEMOTH",
            "NO_MM_HUNTID",
            "NO_EXPECTED_PLAYERS",
            &format!("100.101.102.103:{end}")
        ]
    );
    let switches = &args[7..];
    if version.name == "1.14.7" {
        assert_eq!(
            switches,
            [
                "-EpicPortal",
                "-server",
                "-nullrhi",
                "-warp",
                "-nosound",
                "-RepDriverEnable",
                "-LogCmds=\"LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off\""
            ]
        );
    } else {
        assert_eq!(
            switches,
            [
                "-EpicPortal",
                "-server",
                "-nullrhi",
                "-warp",
                "-RepDriverEnable",
                "-LogCmds=\"LogPhoenixCharacter VeryVerbose, LogOnline Verbose\""
            ]
        );
    }
    assert_eq!(ramsgate["inherited"], "30", "the game servers inherit the env file");
    assert_eq!(
        ramsgate["launchId"].as_str().map(str::len),
        Some(36),
        "MYSTICPARADOX_GAMESERVER_LAUNCH_ID"
    );

    let city = d.matchmake(json!({"GameMode": "CITY"})).await;
    assert_eq!(
        (city.status, city.text.as_str()),
        (200, format!(r#"{{"host":"100.101.102.103","port":{end}}}"#).as_str())
    );
    assert!(
        city.header("x-mysticparadox-request-id")
            .is_some_and(|id| id.len() == 36)
    );
    let dojo = d
        .matchmake(json!({"GameMode": "SHARED", "HuntId": "ShatteredIsles_TrainingDojo"}))
        .await;
    assert_eq!(dojo.json()["port"], end - 1);

    // ISLAND from travel arguments: the oldest free hunt port.
    let island = d
        .matchmake(json!({
            "GameMode": "ISLAND",
            "GameArgs": "/Game/Maps/test/x?MonsterClass=/Game/Monsters/a/fulgar_bp.fulgar_bp_C?HuntID=Arena_MatchmakerHunt_Hard_042",
            "HuntId": "Plain_Hunt",
            "ExpectedPlayers": ["acc-1"]
        }))
        .await;
    assert_eq!(island.text, format!(r#"{{"host":"100.101.102.103","port":{begin}}}"#));
    let hunt = d.args(begin).await;
    let args = &hunt["args"];
    assert_eq!(args[3], "/Game/Monsters/a/fulgar_bp.fulgar_bp_C");
    assert_eq!(args[4], "Arena_MatchmakerHunt_Hard_042");
    assert_eq!(args[5], "acc-1:Plain_Hunt");
    assert_eq!(hunt["trialsWeek"], "042", "an arena hunt keeps its week");
    assert!(d.logs.join(format!("port{begin}_fulgar_bp.log")).is_file());

    // SHARED through the manifest.
    let shared = d
        .matchmake(json!({"GameMode": "SHARED", "HuntId": "Boss_Hunt", "ExpectedPlayers": ["acc-2", "acc-3"]}))
        .await;
    assert_eq!(shared.json()["port"], begin + 1);
    let args = &d.args(begin + 1).await["args"];
    assert_eq!(args[2], "/Game/Maps/test/boss");
    assert_eq!(args[3], "/Game/Monsters/test/boss_bp.boss_bp_C");
    assert_eq!(args[4], "MM_Boss");
    assert_eq!(args[5], "acc-2:Boss_Hunt,acc-3:Boss_Hunt");

    // The cap (2): a third hunt fails, nothing is started.
    let full = d
        .matchmake(json!({"GameMode": "SHARED", "HuntId": "Plain_Hunt", "ExpectedPlayers": ["acc-4"]}))
        .await;
    assert_eq!(full.status, 500);
    assert!(!d.logs.read_dir().unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(&format!("port{}_", begin + 2))
    }));

    let instances = d.get("/api/instances").await.json();
    let ids: Vec<&str> = instances
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            format!("city-{end}"),
            format!("dojo-{}", end - 1),
            format!("hunt-{begin}"),
            format!("hunt-{}", begin + 1)
        ]
    );
    let boss = &instances[3];
    assert_eq!(boss["players"], 2);
    assert_eq!(boss["maxPlayers"], 4);
    assert_eq!(boss["playerIds"], json!(["acc-2", "acc-3"]));
    assert_eq!(boss["behemoth"], "/Game/Monsters/test/boss_bp.boss_bp_C");
    assert_eq!(instances[0]["maxPlayers"], 0);

    // A hunt that ends gives its port back at once, and the next hunt can start.
    d.stop(begin);
    d.until("the hunt's exit", |dir| {
        dir.gameservers().iter().all(|s| s.port != begin)
    })
    .await;
    let next = d
        .matchmake(json!({"GameMode": "SHARED", "HuntId": "Plain_Hunt", "ExpectedPlayers": ["acc-5"]}))
        .await;
    assert_eq!(next.json()["port"], begin + 2, "FIFO: the freed port is reused last");
    let log = d.log_of(begin, "=== Gameserver process exited").await;
    assert!(log.starts_with("=== Gameserver launch "));
    assert!(
        log.contains("MYSTICPARADOX_GAMESERVER_READY"),
        "the output is copied to the log"
    );
    assert!(
        log.contains(&format!(
            "=== Gameserver process exited port={begin} label=fulgar_bp pid="
        )),
        "{log}"
    );

    let unsupported = d.matchmake(json!({"GameMode": "RAID"})).await;
    assert_eq!(unsupported.status, 500);
    let not_json = send(
        d.addr,
        "POST",
        "/api/matchmaker/handle-matchmaking-for-player",
        &[],
        "GameMode=CITY",
    )
    .await;
    assert_eq!(not_json.status, 500, "no JSON content type: req.body is undefined");

    // Stopping the Director ends every game server and frees every port.
    d.shutdown().await;
    assert!(d.director.gameservers().is_empty());
    for port in d.begin..=d.end {
        until_udp_free(port).await;
    }
}

async fn until_udp_free(port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !udp_free(port) {
        assert!(tokio::time::Instant::now() < deadline, "UDP port {port} is still held");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ramsgate_the_training_grounds_and_a_hunt_start_and_stop_on_1_14_7() {
    bounded(async {
        serves_the_hubs_and_hunts(V1147).await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ramsgate_the_training_grounds_and_a_hunt_start_and_stop_on_1_12_0() {
    bounded(async {
        serves_the_hubs_and_hunts(V1120).await;
    })
    .await;
}

async fn answers_express_404s_and_request_ids(version: Version) {
    let d = start(version, &[]).await;
    let missing = d.get("/nope?x=1").await;
    assert_eq!(missing.status, 404);
    assert!(missing.text.contains("<pre>Cannot GET /nope</pre>"), "{}", missing.text);
    assert_eq!(missing.header("content-type"), Some("text/html; charset=utf-8"));
    assert!(missing.header("x-mysticparadox-request-id").is_some());
    let chosen = send(d.addr, "GET", "/nope", &[("x-request-id", "client-id-0001")], "").await;
    assert_eq!(chosen.header("x-mysticparadox-request-id"), Some("client-id-0001"));
    let unsafe_id = send(d.addr, "GET", "/nope", &[("x-mysticparadox-request-id", "short")], "").await;
    assert_ne!(unsafe_id.header("x-mysticparadox-request-id"), Some("short"));
    let wrong_method = d.post("/api/instances", json!({})).await;
    assert_eq!(wrong_method.status, 404);
    assert!(wrong_method.text.contains("Cannot POST /api/instances"));
    let head = send(d.addr, "HEAD", "/api/instances", &[], "").await;
    assert_eq!((head.status, head.text.as_str()), (200, ""));
    let broken = send(
        d.addr,
        "POST",
        "/api/matchmaker/handle-matchmaking-for-player",
        &[("Content-Type", "application/json")],
        "{oops",
    )
    .await;
    assert_eq!(broken.status, 400);
    let too_big = send(
        d.addr,
        "POST",
        "/api/matchmaker/handle-matchmaking-for-player",
        &[("Content-Type", "application/json")],
        &format!(r#"{{"GameMode":"{}"}}"#, "x".repeat(101 * 1024)),
    )
    .await;
    assert_eq!(too_big.status, 413, "express.json's 100 kB");
}

#[tokio::test(flavor = "multi_thread")]
async fn answers_express_404s_and_request_ids_on_both_versions() {
    bounded(async {
        answers_express_404s_and_request_ids(V1147).await;
        answers_express_404s_and_request_ids(V1120).await;
    })
    .await;
}

async fn failed_starts_stay_failures(version: Version) {
    let d = start(
        version,
        &[
            ("RAMSGATE_HUB", "off"),
            ("TRAINING_HUB", "off"),
            ("GAMESERVER_READY_TIMEOUT_MS", "1500"),
        ],
    )
    .await;
    boot(&d).await;
    // A game server that dies at once: its exit line comes after its pipes closed (DS5's write after end)
    let crash = d
        .matchmake(json!({"GameMode": "ISLAND", "GameArgs": "/Game/Maps/crash"}))
        .await;
    assert_eq!(crash.status, 500);
    let silent = d
        .matchmake(json!({"GameMode": "ISLAND", "GameArgs": "/Game/Maps/silent"}))
        .await;
    assert_eq!(silent.status, 500, "no readiness within GAMESERVER_READY_TIMEOUT_MS");
    // Both ports come back: the crash's at once, the silent one's once its stopped process exits.
    d.until("the ports back", |dir| dir.gameservers().is_empty()).await;
    let log = d.log_of(d.begin, "=== Gameserver process exited").await;
    assert!(log.contains("code=3"), "{log}");
    // The Director is still serving, and the next hunt starts
    let ok = d
        .matchmake(json!({"GameMode": "ISLAND", "GameArgs": "/Game/Maps/test/ok"}))
        .await;
    assert_eq!(ok.status, 200, "{}", ok.text);
    let off = d.matchmake(json!({"GameMode": "CITY"})).await;
    assert_eq!(off.status, 500, "RAMSGATE_HUB=off");
    // TRAINING_HUB=off: the first request starts the Training Grounds and waits for them.
    let dojo = d
        .matchmake(json!({"GameMode": "SHARED", "HuntId": "ShatteredIsles_TrainingDojo"}))
        .await;
    assert_eq!(dojo.json()["port"], d.end - 1, "{}", dojo.text);
    d.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_starts_stay_failures_on_both_versions() {
    bounded(async {
        failed_starts_stay_failures(V1147).await;
        failed_starts_stay_failures(V1120).await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_executable_is_a_failed_matchmaking_and_gives_the_port_back() {
    bounded(async {
        let d = start(V1147, &[("GAMESERVER_BINARY_PATH", "/nonexistent/game-server")]).await;
        d.boot().await;
        for _ in 0..2 {
            let reply = d
                .matchmake(json!({"GameMode": "ISLAND", "GameArgs": "/Game/Maps/test/ok"}))
                .await;
            assert_eq!(reply.status, 500);
        }
        assert!(d.director.gameservers().is_empty());
        assert_eq!(d.get("/api/instances").await.text, "[]");
        // The hubs are retrying on their backoff; stopping ends that
        d.shutdown().await;
    })
    .await;
}

async fn a_dead_hub_is_refused_then_restarted(version: Version) {
    let d = start_with(version, &[("TRAINING_HUB", "off")], |config| {
        config.hub_backoff_ms = Some(vec![100]);
    })
    .await;
    boot(&d).await;
    let first = d.director.ramsgate.current().unwrap();
    d.stop(d.end);
    d.until("Ramsgate's exit", |dir| dir.ramsgate.current().is_none()).await;
    assert_eq!(d.get("/api/hub-status").await.json()["ramsgate"], Value::Null);
    assert_eq!(
        d.matchmake(json!({"GameMode": "CITY"})).await.status,
        500,
        "never a dead hub's address"
    );
    // An early exit backs off, then the supervisor starts it again by itself.
    d.until("Ramsgate's restart", |dir| {
        dir.ramsgate
            .current()
            .is_some_and(|c| !std::sync::Arc::ptr_eq(&c, &first))
    })
    .await;
    assert_eq!(d.matchmake(json!({"GameMode": "CITY"})).await.json()["port"], d.end);
    d.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dead_hub_is_refused_then_restarted_on_both_versions() {
    bounded(async {
        a_dead_hub_is_refused_then_restarted(V1147).await;
        a_dead_hub_is_refused_then_restarted(V1120).await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_registry_survives_a_server_that_dies_after_ready() {
    bounded(async {
        let d = start(V1147, &[("RAMSGATE_HUB", "off"), ("TRAINING_HUB", "off")]).await;
        boot(&d).await;
        let steady = d
            .matchmake(json!({"GameMode": "ISLAND", "GameArgs": "/Game/Maps/test/steady"}))
            .await;
        assert_eq!(steady.status, 200);
        let steady_port = steady.json()["port"].as_u64().unwrap() as u16;
        let flaky = d
            .matchmake(json!({"GameMode": "ISLAND", "GameArgs": "/Game/Maps/flaky"}))
            .await;
        assert_eq!(flaky.status, 200, "ready first, then it dies");
        let flaky_port = flaky.json()["port"].as_u64().unwrap() as u16;
        d.until("the flaky server's exit", |dir| {
            dir.gameservers().iter().all(|s| s.port != flaky_port)
        })
        .await;
        let log = d.log_of(flaky_port, "=== Gameserver process exited").await;
        assert!(log.contains("code=5"), "{log}");
        // Only the steady one is listed, the port is free again, and the Director answers
        let instances = d.get("/api/instances").await.json();
        assert_eq!(instances.as_array().unwrap().len(), 1);
        assert_eq!(instances[0]["id"], format!("hunt-{steady_port}"));
        let again = d
            .matchmake(json!({"GameMode": "ISLAND", "GameArgs": "/Game/Maps/test/after"}))
            .await;
        assert_eq!(again.status, 200);
        d.shutdown().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chatty_server_never_blocks_on_its_pipe() {
    bounded(async {
        // The 2026-07-19 freeze: nothing read stdout after the readiness marker and the server blocked on a full pipe
        let d = start(V1147, &[("RAMSGATE_HUB", "off"), ("TRAINING_HUB", "off")]).await;
        boot(&d).await;
        let reply = d
            .matchmake(json!({"GameMode": "ISLAND", "GameArgs": "/Game/Maps/chatty"}))
            .await;
        assert_eq!(reply.status, 200);
        let port = reply.json()["port"].as_u64().unwrap() as u16;
        let log = d.log_of(port, "CHATTY_DONE").await;
        assert!(log.len() > 3 * 1024 * 1024, "{} bytes", log.len());
        d.shutdown().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_ends_the_whole_process_tree() {
    bounded(async {
        let d = start(V1147, &[("RAMSGATE_HUB", "off"), ("TRAINING_HUB", "off")]).await;
        boot(&d).await;
        let reply = d
            .matchmake(json!({"GameMode": "ISLAND", "GameArgs": "/Game/Maps/grandchild"}))
            .await;
        assert_eq!(reply.status, 200);
        let port = reply.json()["port"].as_u64().unwrap() as u16;
        let log = d.log_of(port, "FAKE_GRANDCHILD_UDP=").await;
        let held: u16 = log
            .lines()
            .find_map(|l| l.strip_prefix("FAKE_GRANDCHILD_UDP="))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(
            !udp_free(port) && !udp_free(held),
            "both the server and its child listen"
        );
        d.shutdown().await;
        until_udp_free(port).await;
        until_udp_free(held).await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_exits_takes_what_it_started_with_it() {
    bounded(async {
        let d = start(V1120, &[("RAMSGATE_HUB", "off"), ("TRAINING_HUB", "off")]).await;
        boot(&d).await;
        let reply = d
            .matchmake(json!({"GameMode": "ISLAND", "GameArgs": "/Game/Maps/grandchild"}))
            .await;
        let port = reply.json()["port"].as_u64().unwrap() as u16;
        let log = d.log_of(port, "FAKE_GRANDCHILD_UDP=").await;
        let held: u16 = log
            .lines()
            .find_map(|l| l.strip_prefix("FAKE_GRANDCHILD_UDP="))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        d.stop(port);
        d.until("the exit", |dir| dir.gameservers().is_empty()).await;
        until_udp_free(held).await;
        d.shutdown().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_public_hunt_is_reused_when_the_host_allows_it() {
    bounded(async {
        let d = start(
            V1147,
            &[
                ("RAMSGATE_HUB", "off"),
                ("TRAINING_HUB", "off"),
                ("ENABLE_PUBLIC_HUNT_REUSE", "1"),
            ],
        )
        .await;
        boot(&d).await;
        let first = d
            .matchmake(json!({"GameMode": "SHARED", "HuntId": "Plain_Hunt", "ExpectedPlayers": ["acc-1"]}))
            .await;
        let second = d
            .matchmake(json!({"GameMode": "SHARED", "HuntId": "Plain_Hunt", "ExpectedPlayers": ["acc-2"]}))
            .await;
        assert_eq!(first.json()["port"], second.json()["port"], "P2 joins P1's server");
        let instances = d.get("/api/instances").await.json();
        assert_eq!(instances[0]["playerIds"], json!(["acc-1", "acc-2"]));
        d.shutdown().await;
    })
    .await;
}
