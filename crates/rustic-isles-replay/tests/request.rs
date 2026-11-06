// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Request preparation (spec §13.2 Q1-Q7) and the recorded answer (§6.5).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::http;
use rustic_isles_replay::{IdMap, ReplayHttpResponse, ReplayTargetAuth, prepare_http_request};
use serde_json::json;

const RAW_KEY: &str = "RRK_aabbccddeeff00112233445566778899aabbccddeeff0011";

fn side() -> (ReplayTargetAuth, String) {
    let mut auth = ReplayTargetAuth {
        gs_key: Some("gs-k".into()),
        gateway_secret: Some("gw".into()),
        ..ReplayTargetAuth::default()
    };
    let hex = auth.register_account_key(RAW_KEY);
    (auth, hex)
}

fn oauth(target: &str, hex: &str) -> rustic_isles_replay::HttpRecord {
    http(json!({
        "t": "2026-10-22T17:00:00.000Z", "conn": 3, "seq": 2, "ip": "10.0.0.5", "method": "POST", "target": target,
        "reqHeaders": {"x-rustic-isles-gameserver-apikey": "<gskey>", "authorization": "basic <redacted>"},
        "reqBody": {"form": [["grant_type", "exchange_code"], ["exchange_code", format!("<key:{hex}>")]]},
    }))
}

#[test]
fn q1_oauth_exchange() {
    let (auth, hex) = side();
    let req = prepare_http_request(&oauth("/account/api/oauth/token", &hex), &auth, &IdMap::new(), true);
    assert!(!req.inexact);
    assert_eq!(
        String::from_utf8(req.body.clone()).unwrap(),
        format!("grant_type=exchange_code&exchange_code={RAW_KEY}")
    );
    let headers: Vec<(&str, &str)> = req.headers.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    assert_eq!(
        headers,
        [
            ("x-rustic-isles-gameserver-apikey", "gs-k"),
            ("authorization", "basic <redacted>"),
            ("content-type", "application/x-www-form-urlencoded"),
            ("x-forwarded-for", "10.0.0.5"),
            ("x-gateway-secret", "gw"),
            ("x-isles-replay-now-ms", "1792688400000"),
            ("x-isles-replay-seed", "3:2:POST:/account/api/oauth/token"),
        ]
    );
}

#[test]
fn q2_basic_redacted_elsewhere_is_inexact() {
    let (auth, hex) = side();
    let req = prepare_http_request(&oauth("/account/api/other", &hex), &auth, &IdMap::new(), true);
    assert!(req.inexact);
}

#[test]
fn q3_game_login_rewrite() {
    let (auth, _) = side();
    let rec = http(json!({"target": "/game/login?x=1", "conn": 1, "seq": 7}));
    let req = prepare_http_request(&rec, &auth, &IdMap::new(), true);
    assert_eq!(req.target, "/login?x=1");
    assert_eq!(req.header("x-isles-replay-seed"), Some("1:7:GET:/game/login?x=1"));
    let bare = prepare_http_request(&http(json!({"target": "/game/login"})), &auth, &IdMap::new(), true);
    assert_eq!(bare.target, "/login");
    let other_route = http(json!({"target": "/game/login", "route": "ws"}));
    assert_eq!(
        prepare_http_request(&other_route, &auth, &IdMap::new(), true).target,
        "/game/login"
    );
    let longer = http(json!({"target": "/game/loginx"}));
    assert_eq!(
        prepare_http_request(&longer, &auth, &IdMap::new(), true).target,
        "/game/loginx"
    );
}

#[test]
fn q4_hop_headers_dropped() {
    let (auth, _) = side();
    let rec = http(
        json!({"reqHeaders": {"Host": "a", "Content-Length": "9", "Connection": "keep-alive",
        "Transfer-Encoding": "chunked", "Keep-Alive": "5", "X-Forwarded-For": "1.2.3.4", "Accept": ["a", "b"],
        "X-Num": 5, "x-gateway-secret": "old"}}),
    );
    let req = prepare_http_request(&rec, &auth, &IdMap::new(), false);
    let headers: Vec<(&str, &str)> = req.headers.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    assert_eq!(
        headers,
        [
            ("x-forwarded-for", "1.2.3.4"),
            ("accept", "a, b"),
            ("x-num", "5"),
            ("x-gateway-secret", "gw")
        ]
    );
}

#[test]
fn q5_form_encoding() {
    let (auth, _) = side();
    let rec = http(
        json!({"method": "POST", "reqHeaders": {"Content-Type": "application/x-www-form-urlencoded"},
        "reqBody": {"form": [["k y", "a b+c/é"], ["s", "-._*~"]]}}),
    );
    let req = prepare_http_request(&rec, &auth, &IdMap::new(), false);
    assert_eq!(
        String::from_utf8(req.body.clone()).unwrap(),
        "k%20y=a%20b%2Bc%2F%C3%A9&s=-._*%7E"
    );
    assert_eq!(req.headers.iter().filter(|(n, _)| n == "content-type").count(), 1);
}

#[test]
fn q6_no_pinning_no_replay_headers() {
    let (auth, _) = side();
    let req = prepare_http_request(&http(json!({})), &auth, &IdMap::new(), false);
    assert!(req.headers.iter().all(|(n, _)| !n.starts_with("x-isles-replay-")));
}

#[test]
fn q7_learned_ids_in_the_target_not_the_seed() {
    let (auth, _) = side();
    let map: IdMap = [("65f1a2b3c4d5e6f7a8b9c0d1".into(), "999aaa000bbb111ccc222d44".into())].into();
    let rec = http(
        json!({"target": "/loadout/u/65f1a2b3c4d5e6f7a8b9c0d1/all", "conn": 2, "seq": 4,
        "method": "PUT", "reqBody": {"json": {"characterId": "65f1a2b3c4d5e6f7a8b9c0d1", "n": 1.5}}}),
    );
    let req = prepare_http_request(&rec, &auth, &map, true);
    assert_eq!(req.target, "/loadout/u/999aaa000bbb111ccc222d44/all");
    assert_eq!(
        req.header("x-isles-replay-seed"),
        Some("2:4:PUT:/loadout/u/65f1a2b3c4d5e6f7a8b9c0d1/all")
    );
    assert_eq!(
        String::from_utf8(req.body.clone()).unwrap(),
        r#"{"characterId":"999aaa000bbb111ccc222d44","n":1.5}"#
    );
    assert_eq!(req.header("content-type"), Some("application/json"));
}

#[test]
fn unreadable_time_and_lossy_bodies() {
    let (auth, _) = side();
    let rec = http(json!({"t": "soon", "reqBody": {"binary": 3}, "ip": ""}));
    let req = prepare_http_request(&rec, &auth, &IdMap::new(), true);
    assert!(req.inexact && req.body.is_empty());
    assert_eq!(req.header("x-isles-replay-now-ms"), Some("1793016000000"));
    assert_eq!(req.header("x-forwarded-for"), None);
    let text = http(json!({"reqBody": {"text": "a=<gskey>"}}));
    assert_eq!(prepare_http_request(&text, &auth, &IdMap::new(), false).body, b"a=gs-k");
}

#[test]
fn recorded_answers() {
    let rec = http(
        json!({"status": 201, "resHeaders": {"Content-Type": "application/json", "Set": ["a", "b"]},
        "resBody": {"json": {"b": 1, "a": "x"}}}),
    );
    let answer = ReplayHttpResponse::from_recorded(&rec);
    assert_eq!(answer.status, 201);
    assert_eq!(answer.headers["content-type"], "application/json");
    assert_eq!(answer.headers["set"], "a, b");
    assert_eq!(answer.body_text, r#"{"b":1,"a":"x"}"#);
    let form = http(json!({"resBody": {"form": [["a b", "c"]]}}));
    assert_eq!(ReplayHttpResponse::from_recorded(&form).body_text, "a b=c");
}

#[test]
fn k7_prepared_debug_hides_values() {
    let (auth, hex) = side();
    let req = prepare_http_request(&oauth("/account/api/oauth/token", &hex), &auth, &IdMap::new(), true);
    let shown = format!("{req:?}");
    for secret in ["gs-k", "gw", RAW_KEY, "exchange_code", "10.0.0.5"] {
        assert!(!shown.contains(secret), "{secret} in {shown}");
    }
}
