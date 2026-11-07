// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The allowlist helper over HTTP, in dry run: every status, the nft script and the Windows rule after a login
//! and a logout, state across a restart, `--close-ports`, the platform checks, modules' ports, and the gateway's
//! feed straight into an in-process helper (spec-rd27-gateway.md 5.2).
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use dainty_gateway::allowlist::config::AllowlistConfig;
use dainty_gateway::allowlist::helper::{Helper, HelperSink, applier, applier_for, close_ports_once};
use dainty_module_api::PortRange;
use dainty_service_kit::env::Env;
use serde_json::{Value, json};

fn settings(dir: &std::path::Path, port: u16, extra: &[(&str, &str)]) -> AllowlistConfig {
    let audit = dir.join("audit.log").display().to_string();
    let state = dir.join("state.json").display().to_string();
    let port = port.to_string();
    let mut pairs = vec![
        ("ALLOWLIST_DRY_RUN", "1"),
        ("ALLOWLIST_SECRET", HELPER_SECRET),
        ("ALLOWLIST_FIREWALL", "nftables"),
        ("ALLOWLIST_PORT", port.as_str()),
        ("ALLOWLIST_AUDIT_LOG", audit.as_str()),
        ("ALLOWLIST_STATE_FILE", state.as_str()),
        ("ALLOWLIST_MIN_INTERVAL_MS", "50"),
    ];
    pairs.extend_from_slice(extra);
    AllowlistConfig::from_env(&Env::from_pairs(pairs)).unwrap()
}

fn audit(dir: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("audit.log"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

async fn post(port: u16, secret: Option<&str>, body: &[u8]) -> Answer {
    let mut headers = vec![("content-type", "application/json")];
    if let Some(secret) = secret {
        headers.push(("x-allowlist-secret", secret));
    }
    request(port, None, "POST", "/allow", &headers, body).await
}

#[tokio::test(flavor = "multi_thread")]
async fn every_status_of_the_endpoint() {
    let _serial = serial().await;
    let dir = tempfile::tempdir().unwrap();
    let port = closed_port().await;
    let config = settings(dir.path(), port, &[("ALLOWLIST_MAX_ENTRIES", "1")]);
    let helper = Helper::new(config.clone(), applier_for(&config)).unwrap();
    let running = helper.start().await.unwrap();
    let s = Some(HELPER_SECRET);

    let missing = request(port, None, "GET", "/nope", &[], b"").await;
    assert_eq!(
        (missing.status, missing.text().as_str()),
        (404, r#"{"error":"not_found"}"#)
    );
    let method = request(port, None, "GET", "/allow", &[], b"").await;
    assert_eq!((method.status, method.header("allow")), (405, Some("POST")));
    let method = request(port, None, "POST", "/status", &[], b"").await;
    assert_eq!((method.status, method.header("allow")), (405, Some("GET")));
    let unauthorized = post(
        port,
        Some("wrong-secret-0123456789abcdef01234"),
        br#"{"ip":"203.0.113.7"}"#,
    )
    .await;
    assert_eq!(
        (unauthorized.status, unauthorized.header("connection")),
        (401, Some("close"))
    );
    assert_eq!(post(port, None, br#"{"ip":"203.0.113.7"}"#).await.status, 401);
    let big = post(port, s, &[b' '; 1025]).await;
    assert_eq!(
        (big.status, big.text().as_str()),
        (413, r#"{"error":"body_too_large"}"#)
    );
    let not_json = post(port, s, b"nope").await;
    assert_eq!(
        (not_json.status, not_json.text().as_str()),
        (400, r#"{"error":"bad_request"}"#)
    );
    let private = post(port, s, br#"{"ip":"10.0.0.1"}"#).await;
    assert_eq!(private.text(), r#"{"error":"invalid_ip","reason":"private"}"#);
    let number = post(port, s, br#"{"ip":5}"#).await;
    assert_eq!(number.json()["reason"], "not_a_string");
    let added = post(port, s, br#"{"ip":"::ffff:203.0.113.7"}"#).await;
    assert_eq!(added.text(), r#"{"ip":"203.0.113.7","added":true,"ttlSeconds":600}"#);
    assert_eq!(added.header("cache-control"), Some("no-store"));
    let again = post(port, s, br#"{"ip":"203.0.113.7"}"#).await;
    assert_eq!(again.json()["added"], false);
    let full = post(port, s, br#"{"ip":"198.51.100.2"}"#).await;
    assert_eq!(
        (full.status, full.text().as_str()),
        (503, r#"{"error":"allowlist_full"}"#)
    );
    let status = request(
        port,
        None,
        "GET",
        "/status",
        &[("x-allowlist-secret", HELPER_SECRET)],
        b"",
    )
    .await;
    let status = status.json();
    assert_eq!(status["backend"], "linux-nftables");
    assert_eq!(status["ports"], "8770-8777");
    assert_eq!(status["entries"][0]["ip"], "203.0.113.7");
    assert!(!status.to_string().contains(HELPER_SECRET));
    helper.close(running).await.unwrap();
    let events: Vec<String> = audit(dir.path())
        .iter()
        .map(|e| e["event"].as_str().unwrap().to_owned())
        .collect();
    for event in ["start", "unauthorized", "refused", "added", "full", "stop"] {
        assert!(events.iter().any(|e| e == event), "{event} in {events:?}");
    }
    assert!(
        !std::fs::read_to_string(dir.path().join("audit.log"))
            .unwrap()
            .contains(HELPER_SECRET)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_and_a_logout_in_the_script_and_the_windows_rule() {
    let _serial = serial().await;
    for firewall in ["nftables", "windows"] {
        let dir = tempfile::tempdir().unwrap();
        let port = closed_port().await;
        let config = settings(
            dir.path(),
            port,
            &[("ALLOWLIST_FIREWALL", firewall), ("ALLOWLIST_TTL_SECONDS", "1")],
        );
        let helper = Helper::new(config.clone(), applier_for(&config)).unwrap();
        let running = helper.start().await.unwrap();
        assert!(helper.allow_ip("203.0.113.7"));
        assert!(helper.sync().settled(Duration::from_secs(5)).await);
        tokio::time::sleep(Duration::from_millis(1300)).await;
        assert!(helper.sync().settled(Duration::from_secs(5)).await);
        helper.close(running).await.unwrap();
        let dry: Vec<Value> = audit(dir.path())
            .into_iter()
            .filter(|e| e["event"] == "dry_run")
            .collect();
        let opened = dry.iter().find(|e| e["enabled"] == true).unwrap();
        let closed = dry.last().unwrap();
        assert_eq!(closed["enabled"], false);
        if firewall == "nftables" {
            assert_eq!(opened["rule"], "table inet reforged_revival");
            assert!(
                opened["script"]
                    .as_str()
                    .unwrap()
                    .contains("\t\telements = { 203.0.113.7 }\n")
            );
            assert!(!closed["script"].as_str().unwrap().contains("elements"));
        } else {
            assert_eq!(opened["windows"]["remoteAddresses"], "203.0.113.7");
            assert_eq!(closed["windows"], json!({"enabled": false}));
        }
        let events: Vec<Value> = audit(dir.path());
        assert!(
            events
                .iter()
                .any(|e| e["event"] == "expired" && e["ip"] == "203.0.113.7")
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_saves_and_start_restores() {
    let _serial = serial().await;
    let dir = tempfile::tempdir().unwrap();
    let config = settings(dir.path(), closed_port().await, &[]);
    let helper = Helper::new(config.clone(), applier_for(&config)).unwrap();
    let running = helper.start().await.unwrap();
    assert!(helper.allow_ip("203.0.113.7"));
    assert_eq!(helper.close(running).await.unwrap(), 1);
    let config = settings(dir.path(), closed_port().await, &[]);
    let again = Helper::new(config.clone(), applier_for(&config)).unwrap();
    let running = again.start().await.unwrap();
    assert_eq!(again.list(), ["203.0.113.7"]);
    let start = audit(dir.path()).into_iter().rfind(|e| e["event"] == "start").unwrap();
    assert_eq!(start["restored"], 1);
    again.close(running).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn close_ports_and_the_platform_checks() {
    let _serial = serial().await;
    for firewall in ["nftables", "windows"] {
        let dir = tempfile::tempdir().unwrap();
        let config = settings(dir.path(), 61005, &[("ALLOWLIST_FIREWALL", firewall)]);
        close_ports_once(&config).await.unwrap();
        let events = audit(dir.path());
        assert_eq!(events.last().unwrap()["event"], "closed_ports");
        assert_eq!(events[0]["enabled"], false);
    }
    let dir = tempfile::tempdir().unwrap();
    let foreign = if cfg!(windows) { "nftables" } else { "windows" };
    let real = settings(
        dir.path(),
        61005,
        &[("ALLOWLIST_DRY_RUN", "0"), ("ALLOWLIST_FIREWALL", foreign)],
    );
    let error = applier(&real).await.err().unwrap();
    let wanted = if cfg!(windows) {
        "ALLOWLIST_FIREWALL=nftables needs Linux"
    } else {
        "ALLOWLIST_FIREWALL=windows needs Windows (Windows Firewall); on Linux use ALLOWLIST_FIREWALL=nftables"
    };
    assert_eq!(error, wanted);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_modules_ports_join_and_leave_the_rule() {
    let _serial = serial().await;
    let dir = tempfile::tempdir().unwrap();
    let config = settings(dir.path(), closed_port().await, &[]);
    let helper = Helper::new(config.clone(), applier_for(&config)).unwrap();
    let running = helper.start().await.unwrap();
    assert!(helper.allow_ip("203.0.113.7"));
    let realms: dainty_module_api::ModuleId = "realms".parse().unwrap();
    helper
        .guard()
        .attach(
            &realms,
            &[PortRange {
                first: 64000,
                last: 64009,
            }],
        )
        .unwrap();
    assert!(helper.sync().settled(Duration::from_secs(5)).await);
    assert!(helper.is_admitted("203.0.113.7", 64005));
    assert_eq!(helper.status()["modulePorts"], json!({"realms": ["64000-64009"]}));
    helper.guard().detach(&realms);
    assert!(helper.sync().settled(Duration::from_secs(5)).await);
    assert!(!helper.is_admitted("203.0.113.7", 64005));
    helper.close(running).await.unwrap();
    let events = audit(dir.path());
    let changed: Vec<&Value> = events.iter().filter(|e| e["event"] == "ports_changed").collect();
    assert_eq!(changed[0]["ports"], "8770-8777,64000-64009");
    assert_eq!(changed[1]["ports"], "8770-8777");
    let scripts: Vec<&str> = events
        .iter()
        .filter(|e| e["event"] == "dry_run")
        .filter_map(|e| e["script"].as_str())
        .collect();
    assert!(
        scripts
            .iter()
            .any(|s| s.contains("udp dport { 8770-8777, 64000-64009 } drop"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_logged_in_players_ports_open_through_the_gateway() {
    let _serial = serial().await;
    let dir = tempfile::tempdir().unwrap();
    let config = settings(dir.path(), closed_port().await, &[("ALLOWLIST_ALLOW_PRIVATE", "1")]);
    let helper = Helper::new(config.clone(), applier_for(&config)).unwrap();
    let running = helper.start().await.unwrap();
    let backend = upstream("backend").await;
    let env = env(None, backend.port, backend.port, None, &[]);
    let gw_config = dainty_gateway::config::Config::from_env(&env).unwrap();
    let feed = dainty_gateway::feed::AllowlistFeed::from_in_process(HelperSink(Arc::downgrade(&helper)).into_sink());
    let gw = dainty_gateway::gateway::Gateway::new(gw_config, None, Some(feed), None).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(async move {
        gw.serve_until(listener, async move {
            _ = rx.await;
        })
        .await;
    });
    let local = std::net::Ipv4Addr::new(127, 0, 0, 71);
    let a = request_from(port, local, None, "POST", "/account/api/oauth/token", &[], b"").await;
    assert_eq!(a.status, 200);
    assert!(helper.is_admitted("127.0.0.71", 8770));
    assert!(!helper.is_admitted("127.0.0.72", 8770));
    _ = tx.send(());
    _ = serving.await;
    helper.close(running).await.unwrap();
}
