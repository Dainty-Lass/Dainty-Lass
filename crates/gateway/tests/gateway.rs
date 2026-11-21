// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The gateway end to end: TLS, routing, the secret, the policy, the feed, the access log, WebSockets, private
//! mode and the capture (spec-rd27-gateway.md section 5.2).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::net::Ipv4Addr;
use std::time::Duration;

use common::*;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

#[tokio::test(flavor = "multi_thread")]
async fn proxies_with_the_secret_set_and_the_policy_applied() {
    let _serial = serial().await;
    let cert = TestCert::new();
    let backend = upstream("backend").await;
    let content = upstream("content").await;
    let (helper, sent) = fake_helper(200).await;
    let gw = start(&env(Some(&cert), backend.port, content.port, Some(helper), &[])).await;
    let fp = Some(cert.fingerprint.as_str());
    let at = log_len();

    // Backend routing, path and query untouched; claims replaced; the user's key passes byte for byte
    let key = "RRK_abcdefgh12345678";
    let a = request(
        gw.port,
        fp,
        "GET",
        "/inventory/x?b=1&a=%20",
        &[
            ("x-forwarded-for", "6.6.6.6"),
            ("x-reforged-gateway", "fake"),
            ("x-reforged-user-api-key", key),
        ],
        b"",
    )
    .await;
    assert_eq!(a.status, 200);
    let seen = a.json();
    assert_eq!(seen["upstream"], "backend");
    assert_eq!(seen["url"], "/inventory/x?b=1&a=%20");
    assert_eq!(seen["headers"]["x-forwarded-for"], "127.0.0.1");
    assert_eq!(seen["headers"]["x-forwarded-proto"], "https");
    assert_eq!(seen["headers"]["x-reforged-gateway"], SECRET);
    assert_eq!(seen["headers"]["x-reforged-user-api-key"], key);

    // Content
    let c = request(gw.port, fp, "GET", "/content/a.json", &[], b"").await;
    assert_eq!(c.json()["upstream"], "content");

    // The game-server key is refused and never forwarded; the admin API never reached
    let before = backend.seen.lock().unwrap().len();
    let k = request(
        gw.port,
        fp,
        "GET",
        "/x",
        &[("x-rustic-isles-gameserver-apikey", "k")],
        b"",
    )
    .await;
    assert_eq!((k.status, k.text().as_str()), (403, r#"{"error":"forbidden"}"#));
    assert_eq!(k.header("cache-control"), Some("no-store"));
    let admin = request(gw.port, fp, "POST", "/reforged/api/CreateInvite", &[], b"{}").await;
    assert_eq!(admin.status, 403);
    assert_eq!(admin.header("connection"), Some("close"));
    assert_eq!(backend.seen.lock().unwrap().len(), before);
    let public = request(gw.port, fp, "GET", "/reforged/api/ServerStatus", &[], b"").await;
    assert_eq!(public.status, 200);

    // Chunked answers stay chunked with both cookies; hop-by-hop and leaky headers dropped
    let s = request(gw.port, fp, "GET", "/status/201", &[], b"").await;
    assert_eq!((s.status, s.text().as_str()), (201, "status 201"));
    assert_eq!(s.header("transfer-encoding"), Some("chunked"));
    assert_eq!(s.all("set-cookie"), ["a=1", "b=2"]);
    assert!(s.header("x-powered-by").is_none() && s.header("x-reforged-gateway").is_none());

    // The feed: tokens always, heartbeats only with a bearer, only 2xx
    let local = |n| Ipv4Addr::new(127, 0, 0, n);
    request_from(
        gw.port,
        local(51),
        fp,
        "POST",
        "/account/api/oauth/token",
        &[],
        b"grant_type=x",
    )
    .await;
    request_from(
        gw.port,
        local(52),
        fp,
        "POST",
        "/heartbeat",
        &[("authorization", "Bearer anything")],
        b"",
    )
    .await;
    request_from(gw.port, local(53), fp, "POST", "/heartbeat", &[], b"").await;
    gw.gateway.feed().idle().await;
    let mut fed = sent.lock().unwrap().clone();
    fed.sort();
    assert_eq!(fed, ["127.0.0.51", "127.0.0.52"]);

    // One access line per request, never a secret or the user's key
    let lines = requests_since(at);
    assert_eq!(lines.len(), 9, "{lines:#?}");
    let text = log_texts().join("\n");
    assert!(!text.contains(SECRET) && !text.contains(HELPER_SECRET) && !text.contains(key));
    let refused = lines.iter().find(|l| l["status"] == 403).unwrap();
    assert_eq!(refused["route"], "gateway");
    assert_eq!(refused["reason"], "gameserver_key_header");
    let keys: Vec<&str> = lines[0].as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys[..11],
        [
            "t", "level", "msg", "ip", "method", "target", "route", "status", "bytesIn", "bytesOut", "ms"
        ]
    );
    gw.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tls_is_required_and_a_down_upstream_is_502() {
    let _serial = serial().await;
    let cert = TestCert::new();
    let down = closed_port().await;
    let gw = start(&env(Some(&cert), down, down, None, &[])).await;
    let a = request(gw.port, Some(&cert.fingerprint), "GET", "/x", &[], b"").await;
    assert_eq!((a.status, a.text().as_str()), (502, r#"{"error":"bad_gateway"}"#));
    let at = log_len();
    let b = request(gw.port, Some(&cert.fingerprint), "GET", "/y", &[], b"").await;
    assert_eq!(b.status, 502);
    let line = &requests_since(at)[0];
    assert_eq!(
        (line["route"].as_str(), line["reason"].as_str()),
        (Some("backend"), Some("upstream_ECONNREFUSED"))
    );
    // Plain HTTP to the TLS port gets nothing usable
    let (answer, _) = exchange(gw.port, Ipv4Addr::LOCALHOST, None, b"GET / HTTP/1.1\r\nhost: x\r\n\r\n").await;
    assert!(answer.is_none());
    gw.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_upgrades_are_joined_byte_for_byte() {
    let _serial = serial().await;
    let cert = TestCert::new();
    let backend = upstream("backend").await;
    let gw = start(&env(Some(&cert), backend.port, backend.port, None, &[])).await;
    let at = log_len();
    let mut s = connect(gw.port, Ipv4Addr::LOCALHOST, Some(&cert.fingerprint)).await;
    s.write_all(
        b"GET // HTTP/1.1\r\nhost: x\r\nconnection: Upgrade\r\nupgrade: websocket\r\nsec-websocket-version: 13\r\nsec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-protocol: xmpp\r\n\r\n",
    )
    .await
    .unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8; 1];
        s.read_exact(&mut b).await.unwrap();
        head.push(b[0]);
    }
    let (answer, _) = parse(&head).unwrap();
    assert_eq!(answer.status, 101);
    assert_eq!(
        answer.header("sec-websocket-accept"),
        Some("s3pPLMBiTxaQ9kYGzzhZRbK+xOo=")
    );
    assert_eq!(answer.header("sec-websocket-protocol"), Some("xmpp"));
    assert!(answer.header("x-powered-by").is_none());
    s.write_all(&client_frame("hi")).await.unwrap();
    let mut frame = [0u8; 9];
    s.read_exact(&mut frame).await.unwrap();
    assert_eq!(&frame[2..], b"echo:hi");
    let seen = backend.seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(seen["headers"]["x-reforged-gateway"], SECRET);
    assert_eq!(seen["headers"]["upgrade"], "websocket");
    let line = &requests_since(at)[0];
    assert_eq!(
        (line["status"].as_i64(), line["route"].as_str()),
        (Some(101), Some("ws"))
    );
    drop(s);

    // A refused upgrade is passed on, closed
    let refused = request(
        gw.port,
        Some(&cert.fingerprint),
        "GET",
        "/refuse",
        &[("upgrade", "websocket"), ("connection", "Upgrade")],
        b"",
    )
    .await;
    assert_eq!((refused.status, refused.text().as_str()), (404, "no room"));
    assert_eq!(refused.header("content-length"), Some("7"));
    assert_eq!(refused.header("connection"), Some("close"));
    gw.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn private_mode_serves_plain_http_with_the_same_rules() {
    let _serial = serial().await;
    let backend = upstream("backend").await;
    let gw = start(&env(None, backend.port, backend.port, None, &[])).await;
    let a = request(
        gw.port,
        None,
        "GET",
        "/__origin/gamedev-prod.steelyard.ca/party",
        &[],
        b"",
    )
    .await;
    assert_eq!(a.json()["url"], "/__origin/gamedev-prod.steelyard.ca/party");
    assert_eq!(a.json()["headers"]["x-forwarded-proto"], "http");
    let inner_admin = request(gw.port, None, "GET", "/__origin/x.example/admin/users", &[], b"").await;
    assert_eq!(inner_admin.status, 403);
    let health = request(gw.port, None, "GET", "/health/live", &[], b"").await;
    assert_eq!(health.status, 403);
    let bad = request(gw.port, None, "GET", "/%zz", &[], b"").await;
    assert_eq!((bad.status, bad.text().as_str()), (400, r#"{"error":"bad_path"}"#));
    let trace = request(gw.port, None, "TRACE", "/heartbeat", &[], b"").await;
    assert_eq!(trace.status, 405);
    let nodes = request(gw.port, None, "GET", "/network/v1/nodes", &[], b"").await;
    assert_eq!((nodes.status, nodes.text().as_str()), (200, r#"{"nodes":[]}"#));
    let ping = request(gw.port, None, "GET", "/network/v1/ping", &[], b"").await;
    assert_eq!(ping.status, 204);
    let regions = request(gw.port, None, "POST", "/network/v1/regions", &[], b"{}").await;
    assert_eq!(
        (regions.status, regions.text().as_str()),
        (401, r#"{"error":"unauthorized"}"#)
    );
    let node = request(gw.port, None, "GET", "/__node/ping", &[], b"").await;
    assert_eq!(node.status, 403);
    gw.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_capture_records_requests_redacted() {
    let _serial = serial().await;
    let backend = upstream("backend").await;
    let dir = tempfile::tempdir().unwrap();
    let extra = [("CAPTURE_DIR", dir.path().display().to_string())];
    let gw = start(&env(None, backend.port, backend.port, None, &extra)).await;
    let key = "RRK_abcdefgh12345678";
    request(
        gw.port,
        None,
        "POST",
        "/login",
        &[("content-type", "application/json"), ("x-reforged-user-api-key", key)],
        br#"{"Username":"Alex","Password":"hunter2"}"#,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    gw.stop().await;
    let file = std::fs::read_dir(dir.path()).unwrap().next().unwrap().unwrap().path();
    let text = std::fs::read_to_string(file).unwrap();
    assert!(
        !text.contains("hunter2") && !text.contains(key) && !text.contains(SECRET),
        "{text}"
    );
    let record: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(record["src"], "gateway");
    assert_eq!(record["kind"], "http");
    assert_eq!(record["reqBody"]["json"]["Password"], "<redacted>");
    assert_eq!(record["reqBody"]["json"]["Username"], "Alex");
    assert!(
        record["reqHeaders"]["x-reforged-user-api-key"]
            .as_str()
            .unwrap()
            .starts_with("<key:")
    );
    assert_eq!(record["status"], 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_settings_and_a_foreign_key_are_refused() {
    let cert = TestCert::new();
    let other = TestCert::new();
    let env = service_kit::env::Env::from_pairs([
        ("GATEWAY_SECRET", SECRET),
        ("GATEWAY_ALLOWLIST", "0"),
        ("GATEWAY_CERT", cert.cert_file.to_str().unwrap()),
        ("GATEWAY_KEY", other.key_file.to_str().unwrap()),
    ]);
    let config = gateway::config::Config::from_env(&env).unwrap();
    let error = gateway::server::load_tls(&config).err().unwrap();
    assert!(error.ends_with("the key does not belong to the certificate"), "{error}");
    let private = gateway::config::Config::from_env(&service_kit::env::Env::from_pairs([
        ("GATEWAY_SECRET", SECRET),
        ("GATEWAY_ALLOWLIST", "0"),
        ("GATEWAY_TLS", "0"),
        ("GATEWAY_BIND", "127.0.0.1"),
    ]))
    .unwrap();
    let feed = gateway::feed::AllowlistFeed::new(None);
    let tls = gateway::server::load_tls(&config.clone()).err();
    assert!(tls.is_some());
    assert_eq!(
        gateway::gateway::Gateway::new(config, None, Some(feed.clone()), None)
            .err()
            .unwrap(),
        "the TLS gateway needs its certificate and key"
    );
    assert!(gateway::gateway::Gateway::new(private, None, Some(feed), None).is_ok());
}
