// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The run (spec §13.2 U1-U7) against small targets built here.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use axum::Router;
use axum::routing::get;
use common::{chat, chat_server, raw_server, record, target};
use rustic_isles::replay::{ReplayScope, next_uuid, with_replay_scope};
use rustic_isles_replay::{
    DiffKind, EntryKind, NormalizerConfig, ReplayConfig, ReplayEndpoint, ReplayError, ReplayOptions, ReplayTarget,
    ReplayTargetAuth, run_replay, run_replay_cli, send_http_request, serve_ephemeral,
};
use serde_json::{Value, json};

fn config(left: ReplayTarget, right: ReplayTarget, filter: Option<&str>) -> ReplayConfig {
    ReplayConfig {
        left,
        right,
        normalizer: NormalizerConfig::default(),
        filter: filter.map(str::to_owned),
    }
}

fn live(gated: bool) -> ReplayTarget {
    ReplayTarget::router("right", target(gated), None, ReplayTargetAuth::with_test_keys("gs-k"))
}

const JSON_HEADERS: &str = "application/json";

#[tokio::test]
async fn u1_skipped_routes() {
    let records = [
        record(json!({"route": "content", "target": "/fixed"})),
        record(json!({"route": "network", "target": "/fixed"})),
        record(json!({"aborted": true, "target": "/fixed"})),
        record(json!({"route": "director", "target": "/ready"})),
    ];
    let report = run_replay(&records, &config(ReplayTarget::recorded(), live(true), None))
        .await
        .unwrap();
    assert_eq!(
        (report.total_records, report.skipped_records, report.http_replayed),
        (4, 4, 0)
    );
    assert!(report.entries.is_empty() && report.passed());
}

#[tokio::test]
async fn u2_filter_and_chat() {
    let ok = json!({"resHeaders": {"content-type": JSON_HEADERS}, "resBody": {"json": {"ok": true}}});
    let mut fixed = ok.clone();
    fixed["target"] = json!("/fixed");
    let mut chat_route = ok;
    chat_route["target"] = json!("/fixed");
    chat_route["route"] = json!("chatty");
    let characters = json!({"method": "GET", "target": "/characters/c-1",
        "resHeaders": {"content-type": JSON_HEADERS}, "resBody": {"json": {"id": "c-1", "level": 1}}});
    let records = [
        record(fixed),
        record(characters),
        chat(json!({"dir": "out", "text": "<presence/>"})),
        record(chat_route),
    ];
    let left = ReplayTarget::recorded();
    let report = run_replay(&records, &config(left.clone(), live(true), Some("/character")))
        .await
        .unwrap();
    assert_eq!(
        (
            report.http_replayed,
            report.skipped_records,
            report.chat_connections_replayed
        ),
        (1, 3, 0)
    );
    assert_eq!(report.entries[0].target, "/characters/c-1");
    assert!(report.passed(), "{}", report.to_text("t:"));

    let report = run_replay(&records, &config(left, live(true), Some("chat")))
        .await
        .unwrap();
    assert_eq!(
        (
            report.http_replayed,
            report.skipped_records,
            report.chat_connections_replayed
        ),
        (1, 2, 1)
    );
    assert_eq!(report.entries[0].route, "chatty");
    let chat_entry = &report.entries[1];
    assert_eq!(chat_entry.kind, EntryKind::Chat);
    assert_eq!(
        (chat_entry.index, chat_entry.target.as_str(), chat_entry.method.as_str()),
        (1, "chat:conn:9", "XMPP")
    );
    assert_eq!(
        (chat_entry.left_status, chat_entry.right_status, chat_entry.seq),
        (None, None, 0)
    );
    assert!(report.passed());
}

#[tokio::test]
async fn u3_lossy_differences_are_explained() {
    let records = [record(json!({"target": "/fixed", "reqBody": {"binary": 12},
        "resHeaders": {"content-type": JSON_HEADERS}, "resBody": {"json": {"ok": false}}}))];
    let report = run_replay(&records, &config(ReplayTarget::recorded(), live(true), None))
        .await
        .unwrap();
    assert_eq!(
        (report.inexact_diffed, report.unexplained_diffed, report.value_diffs),
        (1, 0, 1)
    );
    assert!(report.entries[0].inexact && report.passed());

    let exact = [record(
        json!({"target": "/fixed", "resHeaders": {"content-type": JSON_HEADERS},
        "resBody": {"json": {"ok": false}}}),
    )];
    let report = run_replay(&exact, &config(ReplayTarget::recorded(), live(true), None))
        .await
        .unwrap();
    assert_eq!(report.unexplained_diffed, 1);
    assert!(!report.passed());
}

#[tokio::test]
async fn u4_a_bad_answer_ends_the_run() {
    let addr = raw_server(b"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\n").await;
    let right = ReplayTarget::http("right", format!("http://{addr}"), ReplayTargetAuth::default());
    let records = [record(json!({"target": "/x"}))];
    let error = run_replay(&records, &config(ReplayTarget::recorded(), right, None))
        .await
        .unwrap_err();
    assert!(matches!(error, ReplayError::Transport { .. }));
    assert_eq!(
        error.to_string(),
        format!("HTTP transport error calling http://{addr}/x: malformed HTTP response (missing header terminator)")
    );

    let closed = ReplayTarget::http("right", "http://127.0.0.1:1", ReplayTargetAuth::default());
    let error = run_replay(&records, &config(ReplayTarget::recorded(), closed, None))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("HTTP transport error calling http://127.0.0.1:1/x: ")
    );
}

#[tokio::test]
async fn u5_chunked_answers() {
    let addr = raw_server(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n").await;
    let request = rustic_isles_replay::PreparedHttpRequest {
        method: "GET".into(),
        target: "/".into(),
        headers: vec![],
        body: vec![],
        inexact: false,
    };
    let answer = send_http_request(&format!("http://{addr}/ignored"), &request)
        .await
        .unwrap();
    assert_eq!((answer.status, answer.body_text.as_str()), (200, "hello"));
}

async fn seeded(seed: &str) -> String {
    with_replay_scope(ReplayScope::new(None, Some(seed)), async { next_uuid() }).await
}

#[tokio::test]
async fn u6_cross_learning_and_the_pinning_warning() {
    // The capture was taken from a pinned server, so its later records name the seeded id; the right ignores pinning
    let minted = seeded("5:1:POST:/characters").await;
    let records = [
        record(json!({"conn": 5, "seq": 1, "method": "POST", "target": "/characters", "resBody": null})),
        record(
            json!({"conn": 5, "seq": 2, "method": "GET", "target": format!("/characters/{minted}"),
            "resBody": null}),
        ),
    ];
    let left = ReplayTarget::router("left", target(true), None, ReplayTargetAuth::with_test_keys("gs-k"));
    let report = run_replay(&records, &config(left.clone(), live(false), None))
        .await
        .unwrap();
    assert!(report.passed(), "{}", report.to_text("t:"));
    assert_eq!(report.exact_matched, 2);
    assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
    assert!(report.warnings[0].contains("replay target right"));
    assert!(report.warnings[0].contains("RUSTIC_ISLES_REPLAY_HEADERS"));

    // Two gated targets mint the same ids: nothing to warn about
    let report = run_replay(&records, &config(left, live(true), None)).await.unwrap();
    assert!(report.passed() && report.warnings.is_empty(), "{:?}", report.warnings);
}

#[tokio::test]
async fn learned_ids_follow_the_recorded_answer() {
    // Recorded: the original server minted `c-recorded`; this target mints its own, and the next request follows it
    let records = [
        record(json!({"conn": 1, "seq": 1, "method": "POST", "target": "/characters",
            "resHeaders": {"content-type": JSON_HEADERS},
            "resBody": {"json": {"id": "0b9e8a56-3c3f-4d0b-9a3e-1f2d3c4b5a69", "name": "Slayer"}}})),
        record(
            json!({"conn": 1, "seq": 2, "target": "/characters/0b9e8a56-3c3f-4d0b-9a3e-1f2d3c4b5a69",
            "resHeaders": {"content-type": JSON_HEADERS},
            "resBody": {"json": {"id": "0b9e8a56-3c3f-4d0b-9a3e-1f2d3c4b5a69", "level": 1}}}),
        ),
    ];
    let report = run_replay(&records, &config(ReplayTarget::recorded(), live(true), None))
        .await
        .unwrap();
    assert!(report.passed(), "{}", report.to_text("t:"));
    assert_eq!(report.exact_matched, 2);
}

#[tokio::test]
async fn director_records_reach_the_director() {
    let director = Router::new().route("/ready", get(|| async { axum::Json(json!({"ready": true})) }));
    let right = ReplayTarget::router("right", target(true), Some(director), ReplayTargetAuth::default());
    let records = [record(json!({"route": "director", "target": "/ready",
        "resHeaders": {"content-type": JSON_HEADERS}, "resBody": {"json": {"ready": true}}}))];
    let report = run_replay(&records, &config(ReplayTarget::recorded(), right, None))
        .await
        .unwrap();
    assert_eq!(
        (report.http_replayed, report.exact_matched),
        (1, 1),
        "{}",
        report.to_text("t:")
    );
}

#[tokio::test]
async fn recorded_against_live_with_chat() {
    let chat_addr = chat_server().await;
    let server = serve_ephemeral(target(true)).await.unwrap();
    let right = ReplayTarget {
        name: "right".into(),
        endpoint: ReplayEndpoint::Http {
            base_url: server.base_url(),
            director_url: None,
            chat_addr: Some(chat_addr.to_string()),
        },
        auth: ReplayTargetAuth::with_test_keys("gs-k"),
        pin_clock: true,
    };
    let records = [
        record(json!({"target": "/fixed", "resHeaders": {"content-type": JSON_HEADERS},
            "resBody": {"json": {"ok": true}}})),
        chat(json!({"conn": 4, "event": "open"})),
        chat(json!({"conn": 4, "dir": "in", "text": "<auth mechanism='PLAIN'><plain:UID-alice></auth>"})),
        chat(json!({"conn": 4, "dir": "out", "text": "<success xmlns='urn:ietf:params:xml:ns:xmpp-sasl'/>"})),
    ];
    let report = run_replay(&records, &config(ReplayTarget::recorded(), right, None))
        .await
        .unwrap();
    server.stop().await;
    assert!(report.passed(), "{}", report.to_text("t:"));
    assert_eq!(
        (
            report.http_replayed,
            report.chat_connections_replayed,
            report.exact_matched
        ),
        (1, 1, 2)
    );

    // A frame past the cap makes the connection inexact; a side without a chat address compares the recorded frames
    let records = [
        chat(json!({"conn": 4, "dir": "in", "text": "<presence/>"})),
        chat(json!({"conn": 4, "dir": "out", "omitted": 70000})),
        chat(json!({"conn": 4, "dir": "out", "text": "<presence from='x'/>"})),
    ];
    let report = run_replay(&records, &config(ReplayTarget::recorded(), live(true), None))
        .await
        .unwrap();
    assert_eq!((report.inexact_matched, report.chat_stanza_diffs), (1, 0));
}

#[tokio::test]
async fn u7_out_report() {
    let server = serve_ephemeral(target(true)).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let corpus = dir.path().join("gateway-2026-10-22.jsonl");
    let lines = [
        json!({"kind":"http","t":"2026-10-22T17:00:00.000Z","conn":1,"seq":1,"method":"GET","target":"/fixed",
            "status":200,"resHeaders":{"content-type":JSON_HEADERS},"resBody":{"json":{"ok":false}}}),
        json!({"kind":"http","t":"2026-10-22T17:00:01.000Z","conn":1,"seq":2,"method":"GET","target":"/fixed",
            "route":"content"}),
    ];
    std::fs::write(&corpus, lines.map(|l| l.to_string()).join("\n")).unwrap();
    let out = dir.path().join("report.json");
    std::fs::write(&out, "stale").unwrap();
    let options = ReplayOptions {
        corpus: dir.path().to_path_buf(),
        right: server.base_url(),
        out_path: Some(out.clone()),
        ..ReplayOptions::default()
    };
    let report = run_replay_cli(&options).await.unwrap();
    server.stop().await;
    assert!(!report.passed());
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(!text.ends_with('\n'));
    assert!(
        text.starts_with("{\n  \"total_records\": 2,\n  \"http_replayed\": 1,"),
        "{text}"
    );
    let value: Value = serde_json::from_str(&text).unwrap();
    let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "total_records",
            "http_replayed",
            "chat_connections_replayed",
            "skipped_records",
            "exact_matched",
            "inexact_matched",
            "inexact_diffed",
            "unexplained_diffed",
            "status_diffs",
            "header_diffs",
            "value_diffs",
            "text_diffs",
            "chat_stanza_diffs",
            "duration_ms",
            "entries"
        ]
    );
    let entry = &value["entries"][0];
    let entry_keys: Vec<&String> = entry.as_object().unwrap().keys().collect();
    assert_eq!(
        entry_keys,
        [
            "index",
            "kind",
            "conn",
            "seq",
            "method",
            "target",
            "route",
            "inexact",
            "left_status",
            "right_status",
            "diffs"
        ]
    );
    assert_eq!(entry["kind"], "http");
    assert_eq!(
        entry["diffs"][0],
        json!({"kind": "ValueDiff", "path": "$.ok", "left": false, "right": true})
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&out).unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert!(matches!(report.entries[0].diffs[0], DiffKind::ValueDiff { .. }));

    let bad = ReplayOptions {
        corpus: dir.path().join("missing"),
        right: "http://127.0.0.1:1".into(),
        ..ReplayOptions::default()
    };
    let error = run_replay_cli(&bad).await.unwrap_err().to_string();
    assert!(
        error.starts_with("invalid replay configuration: I/O error at "),
        "{error}"
    );
}
