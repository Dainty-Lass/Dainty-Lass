// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The 18 rows black-box through core's `app()` (spec §17.2 "Routes"): a temp store with alice, bob and carol, the
//! fake Director online with a ready Ramsgate, every Go Now delay 40 ms and padding 1 ms.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use axum::body::Body;
use axum::extract::Request;
use http_body_util::BodyExt;
use rustic_isles::{LiveHub, Settings};
use rustic_isles_matchmaking::RAMSGATE_HUNT;
use rustic_isles_matchmaking::test_support::{FakeDirector, TEST_USER_HEADER, TestModule};
use rustic_isles_store::test_support::TempStore;
use serde_json::Value;
use tower::ServiceExt;

struct Answer {
    status: u16,
    content_type: Option<String>,
    revision: Option<String>,
    instance: Option<String>,
    body: String,
}

impl Answer {
    fn json(&self) -> Value {
        assert_eq!(
            self.content_type.as_deref(),
            Some("application/json; charset=utf-8"),
            "{}",
            self.body
        );
        serde_json::from_str(&self.body).unwrap()
    }

    fn keys(&self) -> Vec<String> {
        self.json().as_object().unwrap().keys().cloned().collect()
    }
}

struct Harness {
    module: TestModule,
    app: axum::Router,
}

impl Harness {
    async fn send(&self, method: &str, path: &str, user: Option<&str>, body: Option<&str>) -> Answer {
        let mut builder = Request::builder().method(method).uri(path).header("host", "localhost");
        if let Some(user) = user {
            builder = builder.header(TEST_USER_HEADER, user);
        }
        let body = match body {
            Some(text) => {
                builder = builder
                    .header("content-type", "application/json")
                    .header("content-length", text.len().to_string());
                Body::from(text.to_owned())
            }
            None => Body::empty(),
        };
        let response = self.app.clone().oneshot(builder.body(body).unwrap()).await.unwrap();
        let header = |name: &str| response.headers().get(name).map(|v| v.to_str().unwrap().to_owned());
        let (content_type, revision, instance) = (
            header("content-type"),
            header("x-mystic-party-revision"),
            header("x-reforged-instance-id"),
        );
        let status = response.status().as_u16();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Answer {
            status,
            content_type,
            revision,
            instance,
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        }
    }

    fn advance(&self, ms: i64) {
        self.module.state.clock().advance(ms);
    }
}

fn assert_empty_object(answer: &Answer) {
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(answer.json(), serde_json::json!({}));
}

fn settings() -> Settings {
    let mut pairs = vec![
        ("QOS_TARGET_URL", "qos.example.net"),
        ("GO_NOW_LOAD_PADDING_MAX_MS", "1"),
    ];
    for name in [
        "HUNT_GO_NOW_MIN_DELAY_MS",
        "HUNT_GO_NOW_MAX_DELAY_MS",
        "HUB_GO_NOW_MIN_DELAY_MS",
        "HUB_GO_NOW_MAX_DELAY_MS",
    ] {
        pairs.push((name, "40"));
    }
    Settings::from_pairs(pairs)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_scenario() {
    let temp = TempStore::open().unwrap();
    for user in ["alice", "bob", "carol"] {
        common::account(&temp.store, user).await;
    }
    let director = FakeDirector::online("203.0.113.10", 7777);
    director.set_ramsgate(Some(LiveHub {
        ready: true,
        host: "203.0.113.10".into(),
        port: 7777,
    }));
    let module = common::module_with(Some(temp.store.clone()), settings(), &director);
    let h = Harness {
        app: module.app(),
        module,
    };
    let (alice, bob) = (Some("alice"), Some("bob"));

    assert_empty_object(&h.send("POST", "/candidate/player/register", alice, None).await);
    assert_empty_object(&h.send("POST", "/candidate/player/alive", alice, None).await);
    assert_empty_object(&h.send("POST", "/candidate/player/alive", bob, None).await);
    let regions = h.send("GET", "/candidate/regions", alice, None).await;
    assert_eq!(
        regions.body,
        r#"{"code":200,"message":"success","payload":{"maxPingingStepTime":3,"pingCount":5,"pingFrequency":0.25,"regionUrls":["qos.example.net"]}}"#
    );
    let qos = h.send("GET", "/QoS", None, None).await;
    assert_eq!(
        (qos.status, qos.content_type.as_deref()),
        (200, Some("text/html; charset=utf-8"))
    );
    assert_eq!(qos.body, "<!DOCTYPE html><html><body>pong</body></html>");

    let unauthenticated = h.send("GET", "/candidate/status", None, None).await;
    assert_eq!((unauthenticated.status, unauthenticated.body.as_str()), (401, ""));
    let missing = h.send("GET", "/candidate/status", alice, None).await;
    assert_eq!(
        (missing.status, missing.content_type.as_deref(), missing.body.as_str()),
        (404, None, "")
    );

    // A Ramsgate join, then its status before and after the release
    let joined = h
        .send("POST", "/candidate/join", alice, Some(r#"{"gameMode":"CITY"}"#))
        .await;
    let body = joined.json();
    assert_eq!(
        joined.keys(),
        [
            "candidateId",
            "candidateStatusPeriodMillis",
            "gameMode",
            "huntId",
            "status",
            "statusReason"
        ]
    );
    assert_eq!(
        (body["gameMode"].as_str(), body["huntId"].as_str()),
        (Some("CITY"), Some(RAMSGATE_HUNT))
    );
    assert_eq!(
        (body["status"].as_str(), body["candidateStatusPeriodMillis"].as_i64()),
        (Some("MATCHING"), Some(1000))
    );
    assert_eq!(joined.instance, None);
    let alice_candidate = body["candidateId"].as_str().unwrap().to_owned();
    let before = h.send("GET", "/candidate/status", alice, None).await.json();
    assert_eq!(
        (before["status"].as_str(), before["huntId"].as_str()),
        (Some("MATCHING"), Some(RAMSGATE_HUNT))
    );
    h.advance(150);
    let after = h.send("GET", "/candidate/status", alice, None).await.json();
    assert_eq!(
        (after["status"].as_str(), after["huntId"].clone()),
        (Some("IN_PROGRESS"), Value::Null)
    );
    let hub_id = h.module.state.hub_game_session_id("203.0.113.10", 7777);
    assert_eq!(after["serverInfo"]["gameSessionId"].as_str(), Some(hub_id.as_str()));
    assert_eq!(h.module.hubs.len(), 1, "the hub session opens the region room");

    // Parties
    let created = h.send("POST", "/party", alice, Some(r#"{"buildId":"647472"}"#)).await;
    assert_eq!(created.revision.as_deref(), Some("1"));
    let party = created.json();
    assert_eq!(
        created.keys(),
        [
            "candidateId",
            "candidateState",
            "gauntletLevel",
            "leaderPlayerId",
            "partyId",
            "playerHuntId",
            "playerStates"
        ]
    );
    assert_eq!(party["candidateState"], "QUEUED_FOR_START");
    assert_eq!(party["candidateId"].as_str(), Some(alice_candidate.as_str()));
    let party_id = party["partyId"].as_str().unwrap().to_owned();
    assert!(party_id.ends_with("_NjQ3NDcy"), "{party_id}");
    assert_eq!(
        dainty_service_kit::jsjson::stringify(&party["playerStates"]),
        r#"[{"consoleSessionId":null,"displayName":"alice name","isMemberOfCandidate":true,"platform":"win","playerId":"alice"}]"#
    );
    let invite = r#"{"recipientPlayerId":"bob","buildId":"647472"}"#;
    assert_empty_object(&h.send("PUT", "/party/invite", alice, Some(invite)).await);
    assert_empty_object(&h.send("PUT", "/party/invite", alice, Some("{}")).await);
    let invites = h.send("GET", "/party/invites", bob, None).await;
    assert_eq!(
        invites.body,
        format!(
            r#"{{"invitations":[{{"partyId":"{party_id}","recipientPlayerId":"bob","sendingDisplayName":"alice name","sendingPlatform":"win","sendingPlayerId":"alice"}}]}}"#
        )
    );
    assert_empty_object(
        &h.send(
            "PUT",
            "/party/invite/accept/alice",
            bob,
            Some(r#"{"buildId":"647472"}"#),
        )
        .await,
    );
    let bobs = h.send("POST", "/party", bob, Some(r#"{"buildId":"647472"}"#)).await;
    assert_eq!(bobs.revision.as_deref(), Some("2"));
    let bobs = bobs.json();
    assert_eq!(
        (bobs["candidateId"].clone(), bobs["partyId"].as_str()),
        (Value::Null, Some(party_id.as_str()))
    );
    let flags: Vec<bool> = bobs["playerStates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["isMemberOfCandidate"].as_bool().unwrap())
        .collect();
    assert_eq!(flags, [true, false]);

    // An ISLAND join without the party id travels solo
    let island = r#"{"gameMode":"ISLAND","playerHuntId":"ShatteredIsles_IslandA"}"#;
    let solo = h.send("POST", "/candidate/join", bob, Some(island)).await.json();
    assert_eq!(solo["status"], "MATCHING");
    assert_eq!(director.launches().last().unwrap().expected_players, ["bob"]);
    let status = h.send("GET", "/candidate/status", bob, None).await.json();
    assert_eq!(
        (status["status"].as_str(), status["gameMode"].as_str()),
        (Some("MATCHING"), Some("ISLAND"))
    );
    for (method, path) in [
        ("DELETE", "/candidate/leave"),
        ("POST", "/candidate/leave"),
        ("PUT", "/candidate/leave"),
        ("DELETE", "/candidate"),
        ("DELETE", "/candidate/whatever"),
    ] {
        assert_empty_object(&h.send(method, path, bob, None).await);
    }
    let cancelled = h.send("GET", "/candidate/status", bob, None).await.json();
    assert_eq!(
        (cancelled["status"].as_str(), cancelled["statusReason"].as_str()),
        (Some("CANCELED"), Some("PLAYER_CANCELED"))
    );

    // The party's own ISLAND join
    let with_id = format!(r#"{{"gameMode":"ISLAND","playerHuntId":"ShatteredIsles_IslandA","partyId":"{party_id}"}}"#);
    let leader = h.send("POST", "/candidate/join", alice, Some(&with_id)).await.json();
    assert_eq!(director.launches().last().unwrap().expected_players, ["alice", "bob"]);
    let (a, b) = (
        h.send("GET", "/candidate/status", alice, None).await.json(),
        h.send("GET", "/candidate/status", bob, None).await.json(),
    );
    assert_eq!(
        (a["status"].as_str(), b["status"].as_str()),
        (Some("MATCHING"), Some("MATCHING"))
    );
    assert_ne!(a["candidateId"], b["candidateId"]);
    assert_eq!(a["candidateId"], leader["candidateId"]);
    let matching = h.send("POST", "/party", bob, Some("{}")).await.json();
    assert_eq!(
        (matching["candidateState"].as_str(), matching["playerHuntId"].as_str()),
        (Some("MATCHING"), Some("ShatteredIsles_IslandA"))
    );
    h.advance(150);
    let running = h.send("POST", "/party", bob, Some("{}")).await.json();
    assert_eq!(running["candidateState"], "IN_PROGRESS");
    let travelling = h.send("GET", "/candidate/status", bob, None).await.json();
    assert_eq!(travelling["status"], "IN_PROGRESS");
    assert_eq!(travelling["serverInfo"]["host"], "203.0.113.10");

    assert_empty_object(&h.send("PUT", "/party/member/promote/bob", alice, None).await);
    assert_empty_object(&h.send("DELETE", "/party/member/alice", bob, None).await);
    assert_empty_object(&h.send("DELETE", "/party/member", bob, None).await);

    let no_body = h.send("POST", "/candidate/join", alice, None).await;
    assert_eq!(
        (no_body.status, no_body.content_type.as_deref()),
        (500, Some("text/html; charset=utf-8"))
    );
    let blank = h.send("POST", "/candidate/join", Some("carol"), Some("{}")).await;
    assert_eq!(
        blank.keys(),
        [
            "candidateId",
            "candidateStatusPeriodMillis",
            "huntId",
            "status",
            "statusReason"
        ]
    );
    assert_eq!(blank.json()["huntId"], "");
    let malformed = h.send("POST", "/party", alice, Some("{")).await;
    assert_eq!(
        (malformed.status, malformed.content_type.as_deref()),
        (400, Some("text/html; charset=utf-8"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_database() {
    let director = FakeDirector::online("203.0.113.10", 7777);
    let module = common::module_with(None, settings(), &director);
    let app = module.app();
    let send = |method: &str, path: &str, body: Option<&str>| {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "localhost")
            .header(TEST_USER_HEADER, "alice");
        let body = match body {
            Some(text) => {
                builder = builder
                    .header("content-type", "application/json")
                    .header("content-length", text.len().to_string());
                Body::from(text.to_owned())
            }
            None => Body::empty(),
        };
        app.clone().oneshot(builder.body(body).unwrap())
    };
    for (method, path, body) in [
        ("POST", "/candidate/join", Some("{}")),
        ("DELETE", "/candidate/leave", None),
        ("POST", "/party", Some("{}")),
        ("GET", "/party/invites", None),
        ("PUT", "/party/invite/accept/bob", Some("{}")),
        ("DELETE", "/party/member", None),
        ("DELETE", "/party/member/bob", None),
        ("PUT", "/party/member/promote/bob", None),
        ("PUT", "/party/invite", Some(r#"{"recipientPlayerId":"bob"}"#)),
    ] {
        let response = send(method, path, body).await.unwrap();
        assert_eq!(response.status().as_u16(), 501, "{method} {path}");
    }
    let invite = send("PUT", "/party/invite", Some("{}")).await.unwrap();
    assert_eq!(invite.status().as_u16(), 200);
    let status = send("GET", "/candidate/status", None).await.unwrap();
    assert_eq!(status.status().as_u16(), 404);
}
