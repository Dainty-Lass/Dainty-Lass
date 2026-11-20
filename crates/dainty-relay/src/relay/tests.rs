// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Mutex;

use futures_util::future::BoxFuture;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

use super::*;
use crate::owner::{GameOwnerCheck, TableSource};
use crate::pinned::certificate_fingerprint;
use crate::pinned::test_tls::{acceptor, certificate};

const KEY: &str = "DLK_relaytestkey0123456789abcdefghij";
const WAIT: Duration = Duration::from_secs(5);

// ---- the fake server --------------------------------------------------------------------------------------------

type ExtraHeaders = Vec<(&'static str, String)>;

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    target: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

#[derive(Clone, Default)]
struct Fake {
    seen: Arc<Mutex<Vec<Seen>>>,
    /// Headers the next plain answer carries, for re-pointing
    next_headers: Arc<Mutex<VecDeque<ExtraHeaders>>>,
}

impl Fake {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn last(&self) -> Seen {
        self.seen().pop().expect("the server saw a request")
    }

    fn answer_next_with(&self, headers: ExtraHeaders) {
        self.next_headers.lock().unwrap().push_back(headers);
    }
}

async fn fake_answer(fake: Fake, mut request: Request<Incoming>) -> Result<Response<ResponseBody>, Infallible> {
    let target = request.uri().path_and_query().map_or("/", |pq| pq.as_str()).to_owned();
    if target.starts_with("/ws") {
        let upgrade = hyper::upgrade::on(&mut request);
        fake.seen.lock().unwrap().push(Seen {
            method: request.method().to_string(),
            target,
            headers: request.headers().clone(),
            body: Vec::new(),
        });
        tokio::spawn(async move {
            if let Ok(upgraded) = upgrade.await {
                let mut io = TokioIo::new(upgraded);
                let mut buf = [0u8; 1024];
                while let Ok(n) = io.read(&mut buf).await {
                    if n == 0 || io.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            }
        });
        let mut response = bare(StatusCode::SWITCHING_PROTOCOLS);
        let headers = response.headers_mut();
        headers.insert(UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(CONNECTION, HeaderValue::from_static("Upgrade"));
        headers.insert("sec-websocket-accept", HeaderValue::from_static("accepted"));
        return Ok(response);
    }
    if target.starts_with("/refuse-ws") {
        let mut response = Response::new(full("no upgrade for you"));
        *response.status_mut() = StatusCode::BAD_REQUEST;
        response
            .headers_mut()
            .insert("x-why", HeaderValue::from_static("policy"));
        return Ok(response);
    }
    let (parts, body) = request.into_parts();
    let body = body.collect().await.map(|b| b.to_bytes().to_vec()).unwrap_or_default();
    fake.seen.lock().unwrap().push(Seen {
        method: parts.method.to_string(),
        target: target.clone(),
        headers: parts.headers.clone(),
        body: body.clone(),
    });
    let reply = if body.is_empty() {
        format!("ok:{target}").into_bytes()
    } else {
        body
    };
    let mut response = Response::new(full(reply));
    let headers = response.headers_mut();
    headers.insert("x-reply", HeaderValue::from_static("yes"));
    headers.insert("keep-alive", HeaderValue::from_static("timeout=5"));
    if let Some(extra) = fake.next_headers.lock().unwrap().pop_front() {
        for (name, value) in extra {
            headers.insert(name, HeaderValue::from_str(&value).unwrap());
        }
    }
    if target.starts_with("/reason") {
        *response.status_mut() = StatusCode::from_u16(299).unwrap();
        response
            .extensions_mut()
            .insert(hyper::ext::ReasonPhrase::from_static(b"Quite Fine"));
    }
    Ok(response)
}

/// A server on 127.0.0.1:0; with `tls`, behind a throwaway certificate whose fingerprint is the endpoint's pin.
async fn fake_server(tls: bool) -> (Endpoint, Fake) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (cert, key) = certificate();
    let pin = tls.then(|| certificate_fingerprint(cert.as_ref()));
    let tls = tls.then(|| acceptor(cert, key));
    let fake = Fake::default();
    let state = fake.clone();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { return };
            let (tls, state) = (tls.clone(), state.clone());
            tokio::spawn(async move {
                let service = service_fn(move |request| fake_answer(state.clone(), request));
                let http = hyper::server::conn::http1::Builder::new();
                match tls {
                    Some(tls) => {
                        if let Ok(stream) = tls.accept(tcp).await {
                            let _ = http
                                .serve_connection(TokioIo::new(stream), service)
                                .with_upgrades()
                                .await;
                        }
                    }
                    None => {
                        let _ = http.serve_connection(TokioIo::new(tcp), service).with_upgrades().await;
                    }
                }
            });
        }
    });
    let endpoint = Endpoint {
        host: "127.0.0.1".to_owned(),
        port,
        pin,
    };
    (endpoint, fake)
}

// ---- the relay under test ---------------------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Heard {
    lines: Arc<Mutex<Vec<(Level, String)>>>,
    pins: Arc<Mutex<Vec<String>>>,
}

impl Heard {
    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().iter().map(|(_, l)| l.clone()).collect()
    }

    fn pins(&self) -> Vec<String> {
        self.pins.lock().unwrap().clone()
    }

    fn assert_no_key(&self) {
        for line in self.lines() {
            assert!(!line.contains(KEY), "the key reached the log: {line}");
            assert!(
                !line.contains("relaytestkey"),
                "part of the key reached the log: {line}"
            );
        }
    }
}

fn config(target: Endpoint, heard: &Heard) -> RelayConfig {
    let (lines, pins) = (Arc::clone(&heard.lines), Arc::clone(&heard.pins));
    RelayConfig {
        port: 0,
        qos_port: Some(0),
        target,
        owner: None,
        log: Arc::new(move |level, line| lines.lock().unwrap().push((level, line.to_owned()))),
        on_pin_mismatch: Arc::new(move |actual| pins.lock().unwrap().push(actual.to_owned())),
    }
}

async fn relay_for(target: Endpoint) -> (Relay, Heard) {
    let heard = Heard::default();
    let relay = Relay::start(config(target, &heard)).await.unwrap();
    (relay, heard)
}

// ---- a raw client, so every header and framing byte is the test's ------------------------------------------------

#[derive(Debug)]
struct Reply {
    status: u16,
    reason: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn parse_head(head: &str) -> (u16, String, Vec<(String, String)>) {
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap();
    let mut parts = status_line.splitn(3, ' ');
    let _version = parts.next();
    let status = parts.next().unwrap().parse().unwrap();
    let reason = parts.next().unwrap_or("").to_owned();
    let headers = lines
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (n, v) = l.split_once(':').unwrap();
            (n.trim().to_ascii_lowercase(), v.trim().to_owned())
        })
        .collect();
    (status, reason, headers)
}

fn dechunk(mut raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let line_end = raw.windows(2).position(|w| w == b"\r\n").unwrap();
        let size = usize::from_str_radix(std::str::from_utf8(&raw[..line_end]).unwrap().trim(), 16).unwrap();
        raw = &raw[line_end + 2..];
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&raw[..size]);
        raw = &raw[size + 2..];
    }
}

/// Sends `raw` from a fresh loopback socket and reads one answer.
async fn send(port: u16, raw: &str) -> Reply {
    send_bytes(TcpStream::connect(("127.0.0.1", port)).await.unwrap(), raw.as_bytes()).await
}

async fn send_bytes(mut tcp: TcpStream, raw: &[u8]) -> Reply {
    tcp.write_all(raw).await.unwrap();
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    let read = async {
        loop {
            if let Some(split) = out.windows(4).position(|w| w == b"\r\n\r\n") {
                let (status, _, headers) = parse_head(std::str::from_utf8(&out[..split]).unwrap());
                let header = |name: &str| headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());
                let body = &out[split + 4..];
                let done = status == 101
                    || match (header("content-length"), header("transfer-encoding")) {
                        (Some(len), _) => body.len() >= len.parse::<usize>().unwrap(),
                        (None, Some(_)) => body.ends_with(b"0\r\n\r\n"),
                        (None, None) => false,
                    };
                if done {
                    return;
                }
            }
            let n = tcp.read(&mut buf).await.unwrap_or(0);
            if n == 0 {
                return;
            }
            out.extend_from_slice(&buf[..n]);
        }
    };
    tokio::time::timeout(WAIT, read).await.expect("a whole answer");
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").expect("a full head");
    let (status, reason, headers) = parse_head(std::str::from_utf8(&out[..split]).unwrap());
    let rest = out[split + 4..].to_vec();
    let mut reply = Reply {
        status,
        reason,
        headers,
        body: rest,
    };
    if reply.header("transfer-encoding") == Some("chunked") {
        reply.body = dechunk(&reply.body);
    }
    reply
}

fn get(path: &str, extra: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nhost: 127.0.0.1:61100\r\n{extra}connection: close\r\n\r\n")
}

// ---- start, port and stop ---------------------------------------------------------------------------------------

#[test]
fn port_override_rule() {
    for (raw, port) in [("61101", Some(61101)), (" 1024\t", Some(1024)), ("65535", Some(65535))] {
        assert_eq!(port_override(raw), port, "{raw:?}");
    }
    for raw in [
        "", "1023", "999", "065535", "65536", "99999", "123456", "6110a", "+6110", "61 00", "0080",
    ] {
        assert_eq!(port_override(raw), None, "{raw:?}");
    }
    assert_eq!(configured_port(None), DEFAULT_PORT);
    assert_eq!(configured_port(Some("bad")), DEFAULT_PORT);
    assert_eq!(configured_port(Some("62000")), 62000);
}

#[tokio::test]
async fn starts_logs_and_stops_freeing_both_ports() {
    let (target, _) = fake_server(false).await;
    let (relay, heard) = relay_for(target).await;
    let (port, qos) = (relay.port(), relay.qos_port().unwrap());
    assert_eq!(relay.mode(), Mode::Private);
    assert_eq!(
        heard.lines(),
        vec![format!("relay listening on 127.0.0.1:{port} for 127.0.0.1 (private)")]
    );
    let counters = relay.stop().await;
    assert_eq!(counters, RelayCounters::default());
    TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    TcpListener::bind(("127.0.0.1", qos)).await.unwrap();
}

#[tokio::test]
async fn a_relay_off_its_default_port_moves_the_regions_qos_target_to_itself() {
    let (target, _) = fake_server(false).await;
    let (relay, _) = relay_for(target).await;
    let port = relay.port();
    assert_ne!(port, DEFAULT_PORT);
    let body = format!(r#"{{"regionUrls":["http://127.0.0.1:{DEFAULT_PORT}/QoS"]}}"#);
    let post = |path: &str| {
        format!(
            "POST {path} HTTP/1.1\r\nhost: localhost\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    let moved = send(port, &post("/dauntless/api/matchmaking/candidate/regions?x=1")).await;
    let want = format!(r#"{{"regionUrls":["http://127.0.0.1:{port}/QoS"]}}"#);
    assert_eq!(moved.text(), want);
    assert_eq!(moved.header("content-length"), Some(want.len().to_string().as_str()));
    // Anything else passes as it came
    assert_eq!(send(port, &post("/elsewhere")).await.text(), body);
    relay.stop().await;
}

#[tokio::test]
async fn a_busy_port_is_its_own_error_and_a_busy_qos_port_is_only_logged() {
    let (target, _) = fake_server(false).await;
    let taken = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let busy = taken.local_addr().unwrap().port();
    let heard = Heard::default();
    let err = Relay::start(RelayConfig {
        port: busy,
        ..config(target.clone(), &heard)
    })
    .await
    .err()
    .unwrap();
    assert!(matches!(err, RelayError::PortBusy(p) if p == busy), "{err:?}");

    let relay = Relay::start(RelayConfig {
        qos_port: Some(busy),
        ..config(target, &heard)
    })
    .await
    .unwrap();
    assert_eq!(relay.qos_port(), None);
    let qos_line = heard.lines.lock().unwrap().last().cloned().unwrap();
    assert_eq!(qos_line.0, Level::Info);
    assert!(
        qos_line
            .1
            .starts_with(&format!("relay QoS listener on 127.0.0.1:{busy} not started"))
    );
}

#[tokio::test]
async fn targets_are_checked_before_binding() {
    let heard = Heard::default();
    let public_without_pin = Endpoint {
        host: "203.0.113.5".to_owned(),
        port: 443,
        pin: None,
    };
    assert!(matches!(
        Relay::start(config(public_without_pin, &heard)).await,
        Err(RelayError::Target(_))
    ));
    let bad_pin = Endpoint {
        host: "203.0.113.5".to_owned(),
        port: 443,
        pin: Some("not-a-fingerprint".to_owned()),
    };
    assert!(matches!(
        Relay::start(config(bad_pin, &heard)).await,
        Err(RelayError::Target(_))
    ));
    assert!(heard.lines().is_empty());
}

// ---- forwarding ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn forwards_methods_headers_and_bodies_plain_and_pinned() {
    for tls in [false, true] {
        let (target, fake) = fake_server(tls).await;
        let (relay, heard) = relay_for(target.clone()).await;
        assert_eq!(relay.mode(), if tls { Mode::Public } else { Mode::Private });

        for method in ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"] {
            let body = if method == "GET" { "" } else { "{\"a\":1}" };
            let raw = format!(
                "{method} /api/thing?x=1 HTTP/1.1\r\nhost: localhost:61100\r\n{API_KEY_HEADER}: {KEY}\r\nx-custom: kept\r\nkeep-alive: timeout=9\r\nproxy-authorization: basic secret\r\nhttp2-settings: AAAA\r\nconnection: close, x-named\r\nx-named: dropped\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            let reply = send(relay.port(), &raw).await;
            assert_eq!(reply.status, 200, "{method} tls {tls}");
            assert_eq!(reply.header("x-reply"), Some("yes"));
            assert_eq!(reply.header("keep-alive"), None, "hop-by-hop answers are dropped");
            let seen = fake.last();
            assert_eq!(seen.method, method);
            assert_eq!(seen.target, "/api/thing?x=1");
            assert_eq!(seen.body, body.as_bytes());
            assert_eq!(seen.headers[HOST], format!("127.0.0.1:{}", target.port).as_str());
            assert_eq!(seen.headers[API_KEY_HEADER], KEY, "the key goes home");
            assert_eq!(seen.headers["x-custom"], "kept");
            for gone in [
                "keep-alive",
                "proxy-authorization",
                "http2-settings",
                "x-named",
                SESSION_TICKET_HEADER,
            ] {
                assert!(!seen.headers.contains_key(gone), "{gone} was forwarded");
            }
        }
        let counters = relay.counters();
        assert_eq!(counters.requests, 6);
        assert_eq!(counters.failures, 0);
        assert!(counters.upstream_connections >= 1);
        heard.assert_no_key();
    }
}

#[tokio::test]
async fn keep_alive_requests_share_a_pooled_upstream_connection() {
    let (target, fake) = fake_server(true).await;
    let (relay, _) = relay_for(target).await;
    let mut tcp = TcpStream::connect(("127.0.0.1", relay.port())).await.unwrap();
    for i in 0..3 {
        let raw = format!("GET /n{i} HTTP/1.1\r\nhost: localhost\r\n\r\n");
        tcp.write_all(raw.as_bytes()).await.unwrap();
        let mut buf = vec![0u8; 4096];
        let mut got = Vec::new();
        while !String::from_utf8_lossy(&got).contains(&format!("ok:/n{i}")) {
            let n = tokio::time::timeout(WAIT, tcp.read(&mut buf)).await.unwrap().unwrap();
            assert!(n > 0);
            got.extend_from_slice(&buf[..n]);
        }
    }
    assert_eq!(fake.seen().len(), 3);
    assert_eq!(relay.counters().upstream_connections, 1);
}

#[tokio::test]
async fn reason_phrases_pass_back() {
    let (target, _) = fake_server(false).await;
    let (relay, _) = relay_for(target).await;
    let reply = send(relay.port(), &get("/reason", "")).await;
    assert_eq!(reply.status, 299);
    assert_eq!(reply.reason, "Quite Fine");
}

#[tokio::test]
async fn origin_paths_pass_as_sent() {
    let (target, fake) = fake_server(false).await;
    let (relay, _) = relay_for(target).await;
    for path in [
        "/__origin/ParadoxBackend/api",
        "/__origin/a/../b?x=%2F&y",
        "/__origin/",
        "/__origin//double",
    ] {
        let reply = send(relay.port(), &get(path, "")).await;
        assert_eq!(reply.status, 200, "{path}");
        assert_eq!(fake.last().target, path);
    }
}

#[tokio::test]
async fn chunked_bodies_stay_chunked_and_other_encodings_are_refused() {
    let (target, fake) = fake_server(false).await;
    let (relay, _) = relay_for(target).await;
    let raw = "POST /chunks HTTP/1.1\r\nhost: localhost\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
    let reply = send(relay.port(), raw).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), "hello world");
    let seen = fake.last();
    assert_eq!(seen.body, b"hello world");
    assert_eq!(seen.headers[TRANSFER_ENCODING], "chunked");
    assert!(!seen.headers.contains_key(CONTENT_LENGTH));

    let before = fake.seen().len();
    let raw = "POST /gz HTTP/1.1\r\nhost: localhost\r\ntransfer-encoding: gzip, chunked\r\nconnection: close\r\n\r\n0\r\n\r\n";
    let reply = send(relay.port(), raw).await;
    assert_eq!(reply.status, 501);
    assert_eq!(reply.text(), r#"{"error":"unsupported_transfer_encoding"}"#);
    assert_eq!(reply.header("content-type"), Some("application/json"));
    assert_eq!(reply.header("cache-control"), Some("no-store"));
    assert_eq!(fake.seen().len(), before);
}

#[tokio::test]
async fn an_unreachable_server_is_a_502() {
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let target = Endpoint {
        host: "127.0.0.1".to_owned(),
        port,
        pin: None,
    };
    let (relay, heard) = relay_for(target).await;
    let reply = send(relay.port(), &get("/x", &format!("{API_KEY_HEADER}: {KEY}\r\n"))).await;
    assert_eq!(reply.status, 502);
    assert_eq!(reply.text(), r#"{"error":"upstream_unreachable"}"#);
    assert_eq!(reply.header("cache-control"), Some("no-store"));
    let upgrade = send(
        relay.port(),
        "GET /ws HTTP/1.1\r\nhost: localhost\r\nupgrade: websocket\r\nconnection: upgrade\r\n\r\n",
    )
    .await;
    assert_eq!(upgrade.status, 502);
    assert!(upgrade.body.is_empty());
    assert_eq!(relay.counters().failures, 2);
    assert!(heard.pins().is_empty());
    assert!(heard.lines().iter().any(|l| l.contains("upstream unreachable")));
    heard.assert_no_key();
}

#[tokio::test]
async fn a_pin_mismatch_is_reported_as_one() {
    let (mut target, fake) = fake_server(true).await;
    let real = target.pin.replace("0".repeat(64)).unwrap();
    let (relay, heard) = relay_for(target).await;
    let reply = send(relay.port(), &get("/x", &format!("{API_KEY_HEADER}: {KEY}\r\n"))).await;
    assert_eq!(reply.status, 502);
    assert_eq!(reply.text(), r#"{"error":"certificate_mismatch"}"#);
    assert_eq!(reply.header("content-type"), Some("application/json"));
    let upgrade = send(
        relay.port(),
        "GET /ws HTTP/1.1\r\nhost: localhost\r\nupgrade: websocket\r\nconnection: upgrade\r\n\r\n",
    )
    .await;
    assert_eq!(upgrade.status, 502);
    assert!(upgrade.body.is_empty());
    assert_eq!(heard.pins(), vec![real.clone(), real.clone()]);
    let errors: Vec<_> = heard
        .lines
        .lock()
        .unwrap()
        .iter()
        .filter(|(level, _)| *level == Level::Error)
        .map(|(_, l)| l.clone())
        .collect();
    assert_eq!(errors.len(), 2);
    assert!(errors[0].contains("certificate mismatch") && errors[0].contains(&real));
    assert!(
        fake.seen().is_empty(),
        "nothing reached a server with the wrong certificate"
    );
    assert_eq!(relay.counters().failures, 2);
    heard.assert_no_key();
}

// ---- upgrades and tunnels ---------------------------------------------------------------------------------------

const WS: &str = "GET /ws/chat HTTP/1.1\r\nhost: 127.0.0.1:61100\r\nupgrade: websocket\r\nconnection: keep-alive, Upgrade\r\nsec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-version: 13\r\nsec-fetch-mode: websocket\r\n";

async fn read_head(tcp: &mut TcpStream) -> (u16, Vec<(String, String)>) {
    let mut got = Vec::new();
    let mut byte = [0u8; 1];
    while !got.ends_with(b"\r\n\r\n") {
        let n = tokio::time::timeout(WAIT, tcp.read(&mut byte)).await.unwrap().unwrap();
        assert_eq!(n, 1, "closed before the head ended");
        got.push(byte[0]);
    }
    let (status, _, headers) = parse_head(std::str::from_utf8(&got[..got.len() - 4]).unwrap());
    (status, headers)
}

#[tokio::test]
async fn upgrades_tunnel_both_ways_plain_and_pinned_until_stopped() {
    for tls in [false, true] {
        let (target, fake) = fake_server(tls).await;
        let (relay, heard) = relay_for(target.clone()).await;
        let mut tcp = TcpStream::connect(("127.0.0.1", relay.port())).await.unwrap();
        tcp.write_all(format!("{WS}{API_KEY_HEADER}: {KEY}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let (status, headers) = read_head(&mut tcp).await;
        assert_eq!(status, 101, "tls {tls}");
        let header = |name: &str| headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
        assert_eq!(header("upgrade"), Some("websocket"));
        assert_eq!(header("connection"), Some("Upgrade"));
        assert_eq!(header("sec-websocket-accept"), Some("accepted"));

        let seen = fake.last();
        assert_eq!(seen.target, "/ws/chat");
        assert_eq!(seen.headers[UPGRADE], "websocket");
        assert_eq!(seen.headers[CONNECTION], "Upgrade");
        assert_eq!(seen.headers[HOST], format!("127.0.0.1:{}", target.port).as_str());
        assert_eq!(seen.headers[API_KEY_HEADER], KEY);
        assert_eq!(seen.headers["sec-websocket-key"], "dGhlIHNhbXBsZSBub25jZQ==");

        for message in [&b"ping"[..], b"\x81\x05hello", &[0u8; 3000]] {
            tcp.write_all(message).await.unwrap();
            let mut echoed = vec![0u8; message.len()];
            tokio::time::timeout(WAIT, tcp.read_exact(&mut echoed))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(echoed, message);
        }
        let counters = relay.counters();
        assert_eq!((counters.upgrades, counters.requests), (1, 0));

        relay.stop().await;
        let mut rest = [0u8; 16];
        let closed = tokio::time::timeout(WAIT, tcp.read(&mut rest)).await.unwrap();
        assert!(matches!(closed, Ok(0) | Err(_)), "stopping ends the tunnel");
        heard.assert_no_key();
    }
}

#[tokio::test]
async fn upgrades_never_use_the_pool() {
    let (target, _) = fake_server(false).await;
    let (relay, _) = relay_for(target).await;
    assert_eq!(send(relay.port(), &get("/warm", "")).await.status, 200);
    let opened = relay.counters().upstream_connections;
    let mut tcp = TcpStream::connect(("127.0.0.1", relay.port())).await.unwrap();
    tcp.write_all(format!("{WS}\r\n").as_bytes()).await.unwrap();
    assert_eq!(read_head(&mut tcp).await.0, 101);
    assert_eq!(relay.counters().upstream_connections, opened + 1);
}

#[tokio::test]
async fn a_refused_upgrade_passes_its_status_and_body_and_closes() {
    let (target, _) = fake_server(false).await;
    let (relay, _) = relay_for(target).await;
    let raw = "GET /refuse-ws HTTP/1.1\r\nhost: localhost\r\nupgrade: websocket\r\nconnection: Upgrade\r\n\r\n";
    let reply = send(relay.port(), raw).await;
    assert_eq!(reply.status, 400);
    assert_eq!(reply.text(), "no upgrade for you");
    assert_eq!(reply.header("x-why"), Some("policy"));
    assert_eq!(reply.header("connection"), Some("close"));
    // The server's length is not passed on: the body may be cut short
    assert!(
        reply
            .header("content-length")
            .is_none_or(|l| l == reply.body.len().to_string())
    );
}

// ---- the QoS page -----------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_qos_page_on_both_listeners() {
    let (target, fake) = fake_server(false).await;
    let (relay, _) = relay_for(target).await;
    let qos_port = relay.qos_port().unwrap();
    for (port, path) in [
        (relay.port(), "/qos"),
        (relay.port(), "/QOS?t=1"),
        (qos_port, "/"),
        (qos_port, "/qos/x"),
    ] {
        let reply = send(port, &get(path, "")).await;
        assert_eq!(reply.status, 200, "{port} {path}");
        assert_eq!(reply.text(), QOS_BODY);
        assert_eq!(reply.header("content-type"), Some("text/html"));
        assert_eq!(reply.header("cache-control"), Some("no-store"));
        assert_eq!(
            reply.header("content-length"),
            Some(QOS_BODY.len().to_string().as_str())
        );
    }
    let head = send(
        relay.port(),
        "HEAD /qos HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
    assert_eq!(head.header("content-length"), Some(QOS_BODY.len().to_string().as_str()));

    let missing = send(qos_port, &get("/api", "")).await;
    assert_eq!(missing.status, 404);
    assert_eq!(missing.text(), r#"{"error":"not_found"}"#);
    let post = send(
        qos_port,
        "POST /qos HTTP/1.1\r\nhost: localhost\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(post.status, 404);
    // The QoS listener keeps the local rules too, and a refusal there is its 404
    for raw in [
        "GET / HTTP/1.1\r\nhost: example.com\r\nconnection: close\r\n\r\n",
        "GET /QoS HTTP/1.1\r\nhost: evil.example\r\nconnection: close\r\n\r\n",
        "GET /QoS HTTP/1.1\r\nhost: localhost\r\nsec-fetch-mode: no-cors\r\nconnection: close\r\n\r\n",
        "GET /qos HTTP/1.1\r\nhost: localhost\r\norigin: https://evil.example\r\nconnection: close\r\n\r\n",
        "GET /qos HTTP/1.1\r\nhost: evil.example\r\nupgrade: websocket\r\nconnection: Upgrade\r\n\r\n",
    ] {
        let refused = send(qos_port, raw).await;
        assert_eq!(refused.status, 404, "{raw}");
        assert_eq!(refused.text(), r#"{"error":"not_found"}"#);
        assert_eq!(refused.header("content-type"), Some("application/json"));
    }
    // The game's port keeps its 403 for the same requests
    let game_port = send(
        relay.port(),
        "GET /QoS HTTP/1.1\r\nhost: localhost\r\nsec-fetch-mode: no-cors\r\nconnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(game_port.status, 403);
    assert_eq!(game_port.text(), r#"{"error":"forbidden"}"#);

    assert!(fake.seen().is_empty(), "QoS never reaches the server");
    assert_eq!(relay.counters().qos, 5);
    // "/" on the game's port is the server's
    assert_eq!(send(relay.port(), &get("/", "")).await.text(), "ok:/");
}

// ---- local refusals ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn every_local_refusal() {
    let (target, fake) = fake_server(false).await;
    let (relay, heard) = relay_for(target).await;
    let port = relay.port();
    let cases = [
        ("GET / HTTP/1.1\r\nconnection: close\r\n\r\n", "host"),
        (
            "GET / HTTP/1.1\r\nhost: evil.example\r\nconnection: close\r\n\r\n",
            "host",
        ),
        (
            "GET / HTTP/1.1\r\nhost: 127.0.0.1.nip.io\r\nconnection: close\r\n\r\n",
            "host",
        ),
        (
            "GET / HTTP/1.1\r\nhost: localhost\r\nsec-fetch-site: cross-site\r\nconnection: close\r\n\r\n",
            "browser",
        ),
        (
            "GET / HTTP/1.1\r\nhost: localhost\r\norigin: https://evil.example\r\nconnection: close\r\n\r\n",
            "origin",
        ),
        (
            "GET / HTTP/1.1\r\nhost: localhost\r\norigin: null\r\nconnection: close\r\n\r\n",
            "origin",
        ),
    ];
    for (raw, rule) in cases {
        let reply = send(
            port,
            &format!("{}{API_KEY_HEADER}: {KEY}\r\n\r\n", raw.trim_end_matches("\r\n")),
        )
        .await;
        assert_eq!(reply.status, 403, "{rule}");
        assert_eq!(reply.text(), r#"{"error":"forbidden"}"#);
        assert_eq!(reply.header("content-type"), Some("application/json"));
        assert!(heard.lines().last().unwrap().ends_with(rule), "{rule}");
    }
    // An upgrade gets a bare 403
    let raw = "GET /ws HTTP/1.1\r\nhost: evil.example\r\nupgrade: websocket\r\nconnection: Upgrade\r\n\r\n";
    let reply = send(port, raw).await;
    assert_eq!(reply.status, 403);
    assert!(reply.body.is_empty());
    assert_eq!(reply.header("content-type"), None);
    // A loopback origin and the game's own host pass
    let ok = send(port, &get("/fine", "origin: http://127.0.0.1:61100\r\n")).await;
    assert_eq!(ok.status, 200);
    assert_eq!(fake.seen().len(), 1);
    heard.assert_no_key();
}

#[tokio::test]
async fn connect_is_not_forwarded() {
    let (target, fake) = fake_server(false).await;
    let (relay, _) = relay_for(target).await;
    let reply = send(
        relay.port(),
        "CONNECT example.com:443 HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(reply.status, 405);
    assert!(fake.seen().is_empty());
}

// ---- the owner check --------------------------------------------------------------------------------------------

/// A TCP table holding whatever the test put in it.
#[derive(Clone, Default)]
struct FakeTable(Arc<Mutex<String>>);

impl TableSource for FakeTable {
    fn read(&self) -> BoxFuture<'static, Option<String>> {
        let table = self.0.lock().unwrap().clone();
        Box::pin(async move { Some(table) })
    }
}

/// A socket on a known loopback port, and the table row that names `pid` as its owner when it connects to `relay`.
fn bound_socket(pid: u32, relay: u16) -> (TcpSocket, String) {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let client = socket.local_addr().unwrap();
    let row = format!("  TCP    {client}        127.0.0.1:{relay}        ESTABLISHED     {pid}\r\n");
    (socket, row)
}

async fn connect(socket: TcpSocket, port: u16) -> TcpStream {
    socket.connect(SocketAddr::from(([127, 0, 0, 1], port))).await.unwrap()
}

#[tokio::test]
async fn the_owner_check_serves_only_the_games_connections() {
    const GAME: u32 = 4242;
    let (target, fake) = fake_server(false).await;
    let table = FakeTable::default();
    let heard = Heard::default();
    let relay = Relay::start(RelayConfig {
        owner: Some(Arc::new(GameOwnerCheck::new(|| Some(GAME), table.clone()))),
        ..config(target, &heard)
    })
    .await
    .unwrap();
    let port = relay.port();

    let (socket, row) = bound_socket(GAME, port);
    *table.0.lock().unwrap() = row;
    let reply = send_bytes(connect(socket, port).await, get("/game", "").as_bytes()).await;
    assert_eq!(reply.status, 200);
    assert_eq!(fake.last().target, "/game");

    let (socket, row) = bound_socket(GAME + 1, port);
    *table.0.lock().unwrap() = row;
    let raw = format!("GET /other HTTP/1.1\r\nhost: localhost\r\n{API_KEY_HEADER}: {KEY}\r\n\r\n");
    let reply = send_bytes(connect(socket, port).await, raw.as_bytes()).await;
    assert_eq!(reply.status, 403);
    assert_eq!(reply.text(), r#"{"error":"local_process_denied"}"#);
    assert_eq!(reply.header("connection"), Some("close"));
    assert_eq!(reply.header("cache-control"), Some("no-store"));

    // No row at all, and an upgrade from another process: a bare 403
    *table.0.lock().unwrap() = String::new();
    let raw = "GET /ws HTTP/1.1\r\nhost: localhost\r\nupgrade: websocket\r\nconnection: Upgrade\r\n\r\n";
    let reply = send(port, raw).await;
    assert_eq!(reply.status, 403);
    assert!(reply.body.is_empty());

    // The QoS listener is the game's probe target and has no owner check
    let qos = send(relay.qos_port().unwrap(), &get("/qos", "")).await;
    assert_eq!(qos.status, 200);

    // A connection that sends nothing before it closes is refused without a word
    let (socket, row) = bound_socket(GAME + 2, port);
    *table.0.lock().unwrap() = row;
    drop(connect(socket, port).await);
    let after = send(relay.qos_port().unwrap(), &get("/qos", "")).await;
    assert_eq!(after.status, 200);

    assert_eq!(fake.seen().len(), 1);
    let refused: Vec<String> = heard
        .lines()
        .into_iter()
        .filter(|l| l.contains("not the game's"))
        .collect();
    assert_eq!(refused.len(), 2, "{refused:?}");
    assert!(refused[0].contains("refused GET /other from 127.0.0.1:"), "{refused:?}");
    assert!(refused[1].contains("refused GET /ws from"), "{refused:?}");
    heard.assert_no_key();
}

/// The game's Epic token request at login, whose client row Windows listed only after the old single retry: refused,
/// the game failed sign-in and quit (launcher.log 2026-11-13 03:53:34).
#[tokio::test]
async fn the_games_epic_token_request_passes_when_its_row_comes_late() {
    const GAME: u32 = 4242;
    const TOKEN: &str = "/__origin/account-public-service-prod03.ol.epicgames.com/account/api/oauth/token";
    let (target, fake) = fake_server(false).await;
    let table = FakeTable::default();
    let heard = Heard::default();
    let relay = Relay::start(RelayConfig {
        owner: Some(Arc::new(GameOwnerCheck::new(|| Some(GAME), table.clone()))),
        ..config(target, &heard)
    })
    .await
    .unwrap();
    let port = relay.port();
    let (socket, row) = bound_socket(GAME, port);
    let late = table.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        *late.0.lock().unwrap() = row;
    });
    let body = "grant_type=exchange_code";
    let raw = format!(
        "POST {TOKEN} HTTP/1.1\r\nhost: 127.0.0.1:61100\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let reply = send_bytes(connect(socket, port).await, raw.as_bytes()).await;
    assert_eq!(reply.status, 200);
    assert_eq!(fake.last().method, "POST");
    assert_eq!(fake.last().target, TOKEN);
    assert!(heard.lines().iter().all(|l| !l.contains("not the game's")));
}

// ---- re-pointing ------------------------------------------------------------------------------------------------

fn redirect_to(node: &Endpoint, ticket: Option<&str>) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        (REDIRECT_HOST_HEADER, node.host.clone()),
        (REDIRECT_PORT_HEADER, node.port.to_string()),
    ];
    if let Some(pin) = &node.pin {
        headers.push((REDIRECT_FINGERPRINT_HEADER, pin.clone()));
    }
    if let Some(ticket) = ticket {
        headers.push((SESSION_TICKET_HEADER, ticket.to_owned()));
    }
    headers
}

fn keyed(path: &str) -> String {
    get(path, &format!("{API_KEY_HEADER}: {KEY}\r\n"))
}

#[tokio::test]
async fn a_redirect_re_points_mid_session_with_a_ticket_and_home_takes_the_key_again() {
    for tls in [false, true] {
        let (home, at_home) = fake_server(tls).await;
        let (node, at_node) = fake_server(tls).await;
        let (relay, heard) = relay_for(home.clone()).await;

        at_home.answer_next_with(redirect_to(&node, Some("ticket-one")));
        let reply = send(relay.port(), &keyed("/join")).await;
        assert_eq!(reply.status, 200);
        for hidden in [
            REDIRECT_HOST_HEADER,
            REDIRECT_PORT_HEADER,
            REDIRECT_FINGERPRINT_HEADER,
            SESSION_TICKET_HEADER,
        ] {
            assert_eq!(reply.header(hidden), None, "{hidden} reached the game");
        }
        assert!(!relay.is_home());
        assert_eq!(relay.target().port, node.port);

        // The game keeps sending its key; the node gets the ticket instead
        send(relay.port(), &keyed("/hunt")).await;
        let seen = at_node.last();
        assert_eq!(seen.target, "/hunt");
        assert_eq!(seen.headers[SESSION_TICKET_HEADER], "ticket-one");
        assert!(
            !seen.headers.contains_key(API_KEY_HEADER),
            "tls {tls}: the key left home"
        );
        assert_eq!(seen.headers[HOST], format!("127.0.0.1:{}", node.port).as_str());

        // A WebSocket opened now goes to the node as well
        let mut tcp = TcpStream::connect(("127.0.0.1", relay.port())).await.unwrap();
        tcp.write_all(format!("{WS}{API_KEY_HEADER}: {KEY}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        assert_eq!(read_head(&mut tcp).await.0, 101);
        let seen = at_node.last();
        assert_eq!(seen.headers[SESSION_TICKET_HEADER], "ticket-one");
        assert!(!seen.headers.contains_key(API_KEY_HEADER));

        relay.go_home();
        assert!(relay.is_home());
        send(relay.port(), &keyed("/back")).await;
        let seen = at_home.last();
        assert_eq!(seen.target, "/back");
        assert_eq!(seen.headers[API_KEY_HEADER], KEY);
        assert!(!seen.headers.contains_key(SESSION_TICKET_HEADER));
        assert!(
            heard
                .lines()
                .iter()
                .any(|l| l == &format!("relay re-pointed to 127.0.0.1:{}", node.port))
        );
        assert!(
            !heard.lines().iter().any(|l| l.contains("ticket-one")),
            "the ticket stays out of the log"
        );
        heard.assert_no_key();
    }
}

#[tokio::test]
async fn a_redirect_home_with_a_ticket_sends_the_ticket() {
    let (home, at_home) = fake_server(false).await;
    let (relay, _) = relay_for(home.clone()).await;
    at_home.answer_next_with(redirect_to(&home, Some("home-ticket")));
    send(relay.port(), &keyed("/a")).await;
    send(relay.port(), &keyed("/b")).await;
    let seen = at_home.last();
    assert_eq!(seen.headers[SESSION_TICKET_HEADER], "home-ticket");
    assert!(!seen.headers.contains_key(API_KEY_HEADER));
    // Without a ticket, home is home
    at_home.answer_next_with(redirect_to(&home, None));
    send(relay.port(), &keyed("/c")).await;
    send(relay.port(), &keyed("/d")).await;
    assert_eq!(at_home.last().headers[API_KEY_HEADER], KEY);
}

#[tokio::test]
async fn bad_redirects_are_logged_and_ignored() {
    let (home, at_home) = fake_server(true).await;
    let (plain_node, _) = fake_server(false).await;
    let (relay, heard) = relay_for(home.clone()).await;
    let pin = home.pin.clone().unwrap();
    let cases: Vec<Vec<(&'static str, String)>> = vec![
        // Another node without a ticket
        vec![
            (REDIRECT_HOST_HEADER, "127.0.0.1".into()),
            (REDIRECT_PORT_HEADER, "1".into()),
            (REDIRECT_FINGERPRINT_HEADER, pin.clone()),
        ],
        // A blank ticket is none
        vec![
            (REDIRECT_HOST_HEADER, "127.0.0.1".into()),
            (REDIRECT_PORT_HEADER, "1".into()),
            (REDIRECT_FINGERPRINT_HEADER, pin.clone()),
            (SESSION_TICKET_HEADER, "   ".into()),
        ],
        vec![
            (REDIRECT_HOST_HEADER, "127.0.0.1".into()),
            (REDIRECT_PORT_HEADER, "port".into()),
            (SESSION_TICKET_HEADER, "t".into()),
        ],
        vec![
            (REDIRECT_HOST_HEADER, "127.0.0.1".into()),
            (REDIRECT_PORT_HEADER, "0".into()),
            (SESSION_TICKET_HEADER, "t".into()),
        ],
        vec![
            (REDIRECT_HOST_HEADER, "127.0.0.1".into()),
            (REDIRECT_PORT_HEADER, "70000".into()),
            (SESSION_TICKET_HEADER, "t".into()),
        ],
        vec![
            (REDIRECT_PORT_HEADER, "443".into()),
            (SESSION_TICKET_HEADER, "t".into()),
        ],
        vec![
            (REDIRECT_HOST_HEADER, "bad host!".into()),
            (REDIRECT_PORT_HEADER, "443".into()),
            (REDIRECT_FINGERPRINT_HEADER, pin.clone()),
            (SESSION_TICKET_HEADER, "t".into()),
        ],
        vec![
            (REDIRECT_HOST_HEADER, "127.0.0.1".into()),
            (REDIRECT_PORT_HEADER, "443".into()),
            (REDIRECT_FINGERPRINT_HEADER, "zz".into()),
            (SESSION_TICKET_HEADER, "t".into()),
        ],
        // A pinned relay never moves to plain HTTP
        redirect_to(&plain_node, Some("t")),
    ];
    for (i, headers) in cases.into_iter().enumerate() {
        let before = heard.lines().len();
        at_home.answer_next_with(headers);
        assert_eq!(send(relay.port(), &keyed("/x")).await.status, 200);
        assert!(relay.is_home(), "case {i} re-pointed");
        let lines = heard.lines();
        assert_eq!(lines.len(), before + 1, "case {i}");
        assert!(
            lines[before].starts_with("relay ignored a redirect"),
            "case {i}: {}",
            lines[before]
        );
    }
    send(relay.port(), &keyed("/still-home")).await;
    assert_eq!(at_home.last().headers[API_KEY_HEADER], KEY);
    heard.assert_no_key();
}

#[tokio::test]
async fn repoint_needs_a_ticket_away_from_home() {
    let (home, _) = fake_server(false).await;
    let (node, _) = fake_server(false).await;
    let (relay, _) = relay_for(home).await;
    assert!(matches!(
        relay.repoint(node.clone(), None),
        Err(RepointError::TicketRequired)
    ));
    // A blank ticket is no ticket
    for blank in ["", "   ", "\t "] {
        assert!(matches!(
            relay.repoint(node.clone(), Some(blank.into())),
            Err(RepointError::TicketRequired)
        ));
        assert!(relay.is_home());
    }
    assert!(matches!(
        relay.repoint(node.clone(), Some("a\nb".into())),
        Err(RepointError::BadTicket)
    ));
    relay.repoint(node.clone(), Some("t".into())).unwrap();
    assert_eq!(relay.target(), node);
}

// ---- the key and the log -----------------------------------------------------------------------------------------

#[tokio::test]
async fn the_account_key_never_reaches_the_log() {
    let (target, _) = fake_server(true).await;
    let mut wrong = target.clone();
    wrong.pin = Some("f".repeat(64));
    for endpoint in [target, wrong] {
        let (relay, heard) = relay_for(endpoint).await;
        let key = format!("{API_KEY_HEADER}: {KEY}\r\n");
        for raw in [
            keyed("/x"),
            keyed("/qos"),
            get("/x", &format!("{key}host: evil.example\r\n")),
            format!("GET /x HTTP/1.1\r\nhost: evil.example\r\n{key}connection: close\r\n\r\n"),
            format!("GET /ws HTTP/1.1\r\nhost: localhost\r\n{key}upgrade: websocket\r\nconnection: Upgrade\r\n\r\n"),
            format!(
                "POST /x HTTP/1.1\r\nhost: localhost\r\n{key}transfer-encoding: gzip, chunked\r\nconnection: close\r\n\r\n0\r\n\r\n"
            ),
        ] {
            let _ = send(relay.port(), &raw).await;
        }
        assert!(heard.lines().len() > 1);
        heard.assert_no_key();
    }
}
