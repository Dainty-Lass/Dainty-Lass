// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Body caps, rate buckets, connection limits, the timers and graceful shutdown (spec-rd27-gateway.md 5.2).
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use common::*;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

fn limits() -> Vec<(&'static str, String)> {
    vec![
        ("GATEWAY_MAX_BODY_BYTES", "4096".into()),
        ("GATEWAY_RATE_TOKEN", "2,1".into()),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn body_caps() {
    let _serial = serial().await;
    let backend = upstream("backend").await;
    let gw = start(&env(None, backend.port, backend.port, None, &limits())).await;
    let before = backend.seen.lock().unwrap().len();
    let over = request(gw.port, None, "POST", "/character", &[], &[b'x'; 4097]).await;
    assert_eq!(
        (over.status, over.text().as_str()),
        (413, r#"{"error":"body_too_large"}"#)
    );
    assert_eq!(over.header("connection"), Some("close"));
    assert_eq!(backend.seen.lock().unwrap().len(), before);
    let exact = request(gw.port, None, "POST", "/character", &[], &[b'x'; 4096]).await;
    assert_eq!((exact.status, exact.json()["bodyLength"].as_u64()), (200, Some(4096)));

    // Chunked past the cap: cut, 413, the crossing chunk counted and nothing after it
    let at = log_len();
    let mut raw = b"POST /character HTTP/1.1\r\nhost: x\r\ntransfer-encoding: chunked\r\n\r\n".to_vec();
    for _ in 0..6 {
        raw.extend_from_slice(b"400\r\n");
        raw.extend_from_slice(&[b'y'; 1024]);
        raw.extend_from_slice(b"\r\n");
    }
    raw.extend_from_slice(b"0\r\n\r\n");
    let (answer, _) = exchange(gw.port, Ipv4Addr::LOCALHOST, None, &raw).await;
    assert_eq!(answer.unwrap().status, 413);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let line = requests_since(at).into_iter().find(|l| l["status"] == 413).unwrap();
    assert_eq!(line["bytesIn"], 5120);

    // Expect: 100-continue over the cap gets no 100
    let mut s = connect(gw.port, Ipv4Addr::LOCALHOST, None).await;
    s.write_all(b"POST /character HTTP/1.1\r\nhost: x\r\nexpect: 100-continue\r\ncontent-length: 5000\r\n\r\n")
        .await
        .unwrap();
    let mut got = Vec::new();
    _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut got)).await;
    assert!(
        String::from_utf8_lossy(&got).starts_with("HTTP/1.1 413"),
        "{}",
        String::from_utf8_lossy(&got)
    );

    // Under it: the 100, then the answer; `expect` not forwarded
    let mut s = connect(gw.port, Ipv4Addr::LOCALHOST, None).await;
    s.write_all(b"POST /character HTTP/1.1\r\nhost: x\r\nconnection: close\r\nexpect: 100-continue\r\ncontent-length: 2\r\n\r\n")
        .await
        .unwrap();
    let mut first = [0u8; 25];
    s.read_exact(&mut first).await.unwrap();
    assert!(first.starts_with(b"HTTP/1.1 100 Continue"));
    s.write_all(b"ok").await.unwrap();
    let mut rest = Vec::new();
    s.read_to_end(&mut rest).await.unwrap();
    let text = String::from_utf8_lossy(&rest);
    let start = text.find("HTTP/1.1 200").unwrap();
    let (answer, _) = parse(&rest[start..]).unwrap();
    assert!(answer.json()["headers"].get("expect").is_none());
    gw.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rate_buckets() {
    let _serial = serial().await;
    let backend = upstream("backend").await;
    let gw = start(&env(None, backend.port, backend.port, None, &limits())).await;
    for _ in 0..2 {
        let a = request(gw.port, None, "POST", "/account/api/oauth/token", &[], b"").await;
        assert_eq!(a.status, 200);
    }
    let third = request(gw.port, None, "POST", "/account/api/oauth/token", &[], b"").await;
    assert_eq!(
        (third.status, third.text().as_str()),
        (429, r#"{"error":"rate_limited"}"#)
    );
    assert_eq!(third.header("retry-after"), Some("60"));
    // Another class still has its tokens
    assert_eq!(request(gw.port, None, "GET", "/x", &[], b"").await.status, 200);
    gw.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_per_address_and_new_connection_rate() {
    let _serial = serial().await;
    let backend = upstream("backend").await;
    let extra = [
        ("GATEWAY_MAX_CONNECTIONS_PER_IP", "2".to_owned()),
        ("GATEWAY_RATE_CONNECT", "8,60".to_owned()),
    ];
    let gw = start(&env(None, backend.port, backend.port, None, &extra)).await;
    let local = Ipv4Addr::new(127, 0, 0, 61);
    let held: Vec<_> = futures_util::future::join_all((0..2).map(|_| connect(gw.port, local, None))).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (third, _) = exchange(
        gw.port,
        local,
        None,
        b"GET /x HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n",
    )
    .await;
    assert!(third.is_none(), "a third connection from one address is dropped");
    drop(held);
    tokio::time::sleep(Duration::from_millis(200)).await;
    // A fresh address: 8 of 10 new connections at 8,60
    let other = Ipv4Addr::new(127, 0, 0, 62);
    let mut answered = 0;
    for _ in 0..10 {
        let (a, _) = exchange(
            gw.port,
            other,
            None,
            b"GET /x HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n",
        )
        .await;
        answered += usize::from(a.is_some());
    }
    assert_eq!(answered, 8);
    gw.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_silent_connection_gets_the_raw_408() {
    let _serial = serial().await;
    let backend = upstream("backend").await;
    let extra = [("GATEWAY_HEADERS_TIMEOUT_MS", "1000".to_owned())];
    let gw = start(&env(None, backend.port, backend.port, None, &extra)).await;
    let mut s = connect(gw.port, Ipv4Addr::LOCALHOST, None).await;
    let began = Instant::now();
    let mut got = Vec::new();
    _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut got)).await;
    assert_eq!(got, dainty_gateway::guard::TIMEOUT_408);
    let took = began.elapsed();
    assert!(
        took >= Duration::from_millis(900) && took < Duration::from_millis(2500),
        "{took:?}"
    );
    gw.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn graceful_shutdown_finishes_in_flight_requests_with_close() {
    let _serial = serial().await;
    let backend = upstream("backend").await;
    let extra = [("GATEWAY_SHUTDOWN_GRACE_MS", "5000".to_owned())];
    let gw = start(&env(None, backend.port, backend.port, None, &extra)).await;
    let port = gw.port;
    let slow = tokio::spawn(async move {
        let mut s = connect(port, Ipv4Addr::LOCALHOST, None).await;
        s.write_all(b"GET /slow HTTP/1.1\r\nhost: x\r\n\r\n").await.unwrap();
        let mut got = Vec::new();
        _ = tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut got)).await;
        parse(&got).map(|(a, _)| a)
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let began = Instant::now();
    gw.stop().await;
    let answer = slow.await.unwrap().unwrap();
    assert_eq!(answer.status, 200);
    assert_eq!(answer.header("connection"), Some("close"));
    assert!(began.elapsed() < Duration::from_secs(5));
    // New connections are refused
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    assert!(socket.connect(([127, 0, 0, 1], port).into()).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hanging_request_is_cut_at_the_grace() {
    let _serial = serial().await;
    let backend = upstream("backend").await;
    let extra = [("GATEWAY_SHUTDOWN_GRACE_MS", "1000".to_owned())];
    let gw = start(&env(None, backend.port, backend.port, None, &extra)).await;
    let port = gw.port;
    let hang = tokio::spawn(async move {
        let mut s = connect(port, Ipv4Addr::LOCALHOST, None).await;
        s.write_all(b"GET /hang HTTP/1.1\r\nhost: x\r\n\r\n").await.unwrap();
        let mut got = Vec::new();
        _ = tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut got)).await;
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let began = Instant::now();
    gw.stop().await;
    let took = began.elapsed();
    assert!(
        took >= Duration::from_millis(900) && took < Duration::from_secs(4),
        "{took:?}"
    );
    _ = hang.await;
}
