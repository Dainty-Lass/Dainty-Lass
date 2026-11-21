// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Fixtures written from the spec (§13.1: nothing copied): capture lines, a small pinned target and raw servers.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "shared by several test binaries, each using only some; a panic is how a test fails"
)]

use std::net::SocketAddr;

use axum::Router;
use axum::extract::Path;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use rustic_isles::replay::{ReplayScope, next_uuid, random_uuid, with_replay_scope};
use rustic_isles_replay::{CaptureRecord, HttpRecord, parse_capture_line};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const UUID_A: &str = "0b9e8a56-3c3f-4d0b-9a3e-1f2d3c4b5a69";
pub const UUID_B: &str = "1c8f7b45-2d2e-4c1a-8b2d-0e1c2b3a4968";

/// An HTTP record from a capture line built over these defaults.
pub fn http(fields: Value) -> HttpRecord {
    let mut line = json!({"kind": "http", "t": "2026-10-22T17:00:00.000Z", "conn": 1, "seq": 1,
        "method": "GET", "target": "/x", "route": "backend", "status": 200});
    for (k, v) in fields.as_object().unwrap() {
        line[k] = v.clone();
    }
    match parse_capture_line(&line.to_string()) {
        Some(CaptureRecord::Http(r)) => r,
        other => panic!("not an http record: {other:?}"),
    }
}

pub fn record(fields: Value) -> CaptureRecord {
    CaptureRecord::Http(http(fields))
}

pub fn chat(fields: Value) -> CaptureRecord {
    let mut line = json!({"kind": "chat", "t": "2026-10-22T17:00:00.000Z", "conn": 9});
    for (k, v) in fields.as_object().unwrap() {
        line[k] = v.clone();
    }
    parse_capture_line(&line.to_string()).unwrap()
}

/// A target with core's replay scope: `gated` is `RUSTIC_ISLES_REPLAY_HEADERS` on. `POST /characters` mints an id,
/// `GET /characters/{id}` names it back, `GET /fixed` answers the same bytes every time.
pub fn target(gated: bool) -> Router {
    let scope = move |headers: &HeaderMap| {
        if gated {
            ReplayScope::from_headers(headers)
        } else {
            ReplayScope::new(None, None)
        }
    };
    Router::new()
        .route(
            "/characters",
            post(move |headers: HeaderMap| async move {
                let id = with_replay_scope(scope(&headers), async { next_uuid() }).await;
                axum::Json(json!({"id": id, "name": "Slayer"}))
            }),
        )
        .route(
            "/characters/{id}",
            get(|Path(id): Path<String>| async move { axum::Json(json!({"id": id, "level": 1})) }),
        )
        .route(
            "/random",
            get(|| async { axum::Json(json!({"session": random_uuid()})) }),
        )
        .route("/fixed", get(|| async { axum::Json(json!({"ok": true})) }))
        .route("/empty", post(|| async { axum::http::StatusCode::NO_CONTENT }))
}

/// A server answering every connection with `answer` and closing.
pub async fn raw_server(answer: &'static [u8]) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = vec![0u8; 65536];
            let _ = stream.read(&mut buf).await;
            let _ = stream.write_all(answer).await;
            let _ = stream.shutdown().await;
        }
    });
    addr
}

/// A plain XMPP-over-TCP mock: `<success .../>` to an `<auth ...>`, the frame echoed otherwise.
pub async fn chat_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 {
                        return;
                    }
                    let frame = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let answer = if frame.starts_with("<auth") {
                        "<success xmlns='urn:ietf:params:xml:ns:xmpp-sasl'/>".to_owned()
                    } else {
                        frame
                    };
                    if stream.write_all(answer.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    addr
}
