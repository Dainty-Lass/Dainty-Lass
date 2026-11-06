// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The parity harness (spec-rd27-gateway.md section 5.1): every recorded request on its own TLS connection to a
//! gateway in front of fake upstreams and a fake helper, compared with what the Node gateway answered: the status, the
//! listed headers, the body and the access lines (less `t` and `ms`), then the addresses the helper was sent. How the
//! gateway runs is the caller's (`run`'s `start`). Written at integration from the spec and the fixtures
//! (docs/port/clean-room/gateway.md section 7).
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "shared by several test binaries, each using only some; a panic is how a test fails"
)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::Future;
use std::net::Ipv4Addr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt as _, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use crate::common::{TestCert, connect};

const SECRET: &str = "parity-gateway-secret-0123456789abcdef0123456789abcdef";
const HELPER_SECRET: &str = "parity-allowlist-secret-0123456789abcdef0123456789abcdef";

/// The headers the fixtures recorded, when present.
const HEADERS: [&str; 12] = [
    "content-type",
    "content-length",
    "transfer-encoding",
    "cache-control",
    "connection",
    "retry-after",
    "x-powered-by",
    "x-dainty-gateway",
    "set-cookie",
    "upgrade",
    "sec-websocket-accept",
    "sec-websocket-protocol",
];

/// A gateway the harness drives.
pub struct Server {
    pub port: u16,
    /// Every log line so far, as JSON
    pub log: Arc<dyn Fn() -> Vec<Value> + Send + Sync>,
    /// Resolves once the feed has nothing on its way (or at once, when the caller can't see the feed)
    pub idle: Pin<Box<dyn Future<Output = ()> + Send>>,
    pub stop: Pin<Box<dyn Future<Output = ()> + Send>>,
}

type Body = http_body_util::combinators::UnsyncBoxBody<Bytes, Infallible>;

fn full(status: u16, content_type: &str, body: impl Into<Bytes>) -> Response<Body> {
    let mut r = Response::new(Full::new(body.into()).boxed_unsync());
    *r.status_mut() = StatusCode::from_u16(status).unwrap();
    r.headers_mut().insert("content-type", content_type.parse().unwrap());
    r
}

fn json_text(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

/// `{upstream, method, url, headers (without connection), bodyLength, bodySha}`, in that order.
fn echo_record(name: &str, method: &str, url: &str, headers: &http::HeaderMap, body: &[u8]) -> String {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for (n, v) in headers {
        if n == "connection" {
            continue;
        }
        let value = String::from_utf8_lossy(v.as_bytes()).into_owned();
        // Named as the Node recording names them, so the echo's length matches it too (protocol::renamed)
        seen.entry(protocol::renamed::as_recorded(n.as_str()))
            .and_modify(|old| {
                old.push_str(", ");
                old.push_str(&value);
            })
            .or_insert(value);
    }
    let sha: String = Sha256::digest(body).iter().map(|b| format!("{b:02x}")).collect();
    format!(
        r#"{{"upstream":{},"method":{},"url":{},"headers":{},"bodyLength":{},"bodySha":{}}}"#,
        json_text(name),
        json_text(method),
        json_text(&protocol::renamed::as_recorded(url)),
        serde_json::to_string(&seen).unwrap(),
        body.len(),
        json_text(&sha)
    )
}

fn accept_key(key: &str) -> String {
    use sha1::Digest as _;
    let digest = sha1::Sha1::digest(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes());
    base64::engine::general_purpose::STANDARD.encode(digest)
}

/// Echoes what the client sends as `echo:<bytes>`: the recorded byte counts (in 5, out 10 for `hello`) are of the
/// text alone, so the recorder sent it unframed.
async fn echo_frames(io: hyper::upgrade::Upgraded) {
    let mut io = TokioIo::new(io);
    let mut buf = [0u8; 4096];
    while let Ok(n) = io.read(&mut buf).await {
        if n == 0 {
            return;
        }
        let mut reply = b"echo:".to_vec();
        reply.extend_from_slice(&buf[..n]);
        if io.write_all(&reply).await.is_err() {
            return;
        }
    }
}

/// The backend, content server and chat listener the fixtures were recorded against.
async fn upstream_answer(name: &'static str, mut req: Request<Incoming>) -> Response<Body> {
    let path = req.uri().path().to_owned();
    if req.headers().contains_key("upgrade") {
        if path == "/refuse" {
            return full(404, "text/plain", "no room");
        }
        let key = req
            .headers()
            .get("sec-websocket-key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let protocol = req.headers().get("sec-websocket-protocol").cloned();
        let upgrade = hyper::upgrade::on(&mut req);
        tokio::spawn(async move {
            if let Ok(io) = upgrade.await {
                echo_frames(io).await;
            }
        });
        let mut r = Response::new(Full::new(Bytes::new()).boxed_unsync());
        *r.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
        let h = r.headers_mut();
        h.insert("upgrade", "websocket".parse().unwrap());
        h.insert("connection", "upgrade".parse().unwrap());
        h.insert("sec-websocket-accept", accept_key(&key).parse().unwrap());
        if let Some(p) = protocol {
            h.insert("sec-websocket-protocol", p);
        }
        return r;
    }
    let method = req.method().to_string();
    let url = req.uri().to_string();
    let headers = req.headers().clone();
    let body = req
        .into_body()
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    if let Some(n) = path.strip_prefix("/status/").and_then(|n| n.parse::<u16>().ok()) {
        let frames = futures_util::stream::iter(vec![Ok::<_, Infallible>(Frame::data(Bytes::from(format!(
            "status {n}"
        ))))]);
        let mut r = Response::new(StreamBody::new(frames).boxed_unsync());
        *r.status_mut() = StatusCode::from_u16(n).unwrap();
        let h = r.headers_mut();
        h.insert("content-type", "text/plain".parse().unwrap());
        h.append("set-cookie", "a=1".parse().unwrap());
        h.append("set-cookie", "b=2".parse().unwrap());
        return r;
    }
    match (method.as_str(), path.as_str()) {
        ("POST", "/heartbeat") => {
            let bearer = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("Bearer "));
            return if bearer {
                full(200, "text/plain", "20000")
            } else {
                full(401, "text/plain", "")
            };
        }
        ("POST", "/account/api/oauth/token") => return full(200, "application/json", r#"{"access_token":"x"}"#),
        ("POST", "/game/login" | "/login") => return full(200, "application/json", "{}"),
        _ => {}
    }
    full(
        200,
        "application/json",
        echo_record(name, &method, &url, &headers, &body),
    )
}

async fn serve_fake<F, Fut>(answer: F) -> u16
where
    F: Fn(Request<Incoming>) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Response<Body>> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let answer = answer.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |req| {
                    let answer = answer.clone();
                    async move { Ok::<_, Infallible>(answer(req).await) }
                });
                _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .with_upgrades()
                    .await;
            });
        }
    });
    port
}

/// A helper that records the addresses `POST /allow` brought with its secret.
async fn fake_helper() -> (u16, Arc<Mutex<Vec<String>>>) {
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    let port = serve_fake(move |req: Request<Incoming>| {
        let record = record.clone();
        async move {
            let secret = req
                .headers()
                .get("x-allowlist-secret")
                .is_some_and(|v| v == HELPER_SECRET);
            let allow = req.method() == http::Method::POST && req.uri().path() == "/allow";
            let body = req
                .into_body()
                .collect()
                .await
                .map(|b| b.to_bytes())
                .unwrap_or_default();
            if !(secret && allow) {
                return full(403, "application/json", r#"{"error":"forbidden"}"#);
            }
            let value: Value = serde_json::from_slice(&body).unwrap_or_default();
            record
                .lock()
                .unwrap()
                .push(value["ip"].as_str().unwrap_or_default().to_owned());
            full(200, "application/json", r#"{"ok":true}"#)
        }
    })
    .await;
    (port, seen)
}

/// One answer read from the wire.
struct Read {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// The head, then the body by length, chunks or to the end; a HEAD answer has none.
fn read_answer(raw: &[u8], head_request: bool) -> Option<(Read, usize)> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = String::from_utf8_lossy(&raw[..end]);
    let mut lines = head.split("\r\n");
    let status: u16 = lines.next()?.split(' ').nth(1)?.parse().ok()?;
    let headers: Vec<(String, String)> = lines
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            l.split_once(':')
                .map(|(n, v)| (n.to_ascii_lowercase(), v.trim().to_owned()))
        })
        .collect();
    let find = |name: &str| headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());
    let rest = &raw[end..];
    let mut read = Read {
        status,
        headers: headers.clone(),
        body: Vec::new(),
    };
    if head_request || matches!(status, 101 | 204 | 304) {
        return Some((read, end));
    }
    if let Some(len) = find("content-length").and_then(|v| v.parse::<usize>().ok()) {
        read.body = rest.get(..len)?.to_vec();
        return Some((read, end + len));
    }
    if find("transfer-encoding").is_some_and(|v| v.contains("chunked")) {
        let mut at = 0;
        loop {
            let line_end = rest[at..].windows(2).position(|w| w == b"\r\n")? + at;
            let size = usize::from_str_radix(String::from_utf8_lossy(&rest[at..line_end]).trim(), 16).ok()?;
            at = line_end + 2;
            if size == 0 {
                return Some((read, end + at));
            }
            read.body.extend_from_slice(rest.get(at..at + size)?);
            at += size + 2;
        }
    }
    read.body = rest.to_vec();
    Some((read, raw.len()))
}

/// The fixture's view of one answer: the listed headers (a repeated one joined by `, `) and the body as JSON when it
/// parses, else as text.
fn observed(label: &str, read: &Read, body: &[u8], log: Vec<Value>) -> Value {
    let mut headers = serde_json::Map::new();
    for name in HEADERS {
        let values: Vec<&str> = read
            .headers
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect();
        if !values.is_empty() {
            headers.insert(name.to_owned(), Value::String(values.join(", ")));
        }
    }
    let text = String::from_utf8_lossy(body).into_owned();
    let body = serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text));
    serde_json::json!({ "request": label, "status": read.status, "headers": headers, "body": body, "log": log })
}

/// The lines the fixtures hold, without the fields that vary from run to run.
fn recorded_lines(lines: &[Value]) -> Vec<Value> {
    lines
        .iter()
        .filter(|l| matches!(l["msg"].as_str(), Some("request" | "websocket closed")))
        .map(|l| {
            let mut l = l.clone();
            if let Value::Object(map) = &mut l {
                map.remove("t");
                map.remove("ms");
            }
            l
        })
        .collect()
}

/// Waits until `wanted` recorded lines arrived after `at` (at most 3 s), and returns those there are.
async fn lines_after(server: &Server, at: usize, wanted: usize) -> Vec<Value> {
    for _ in 0..60 {
        let lines = recorded_lines(&(server.log)()[at..]);
        if lines.len() >= wanted {
            tokio::time::sleep(Duration::from_millis(30)).await;
            return recorded_lines(&(server.log)()[at..]);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    recorded_lines(&(server.log)()[at..])
}

/// Sends one recorded request and reads its answer.
async fn exchange(server: &Server, cert: &TestCert, request: &Value, wanted_lines: usize) -> Value {
    let label = request["label"].as_str().unwrap();
    let raw = request["raw"].as_str().unwrap();
    let local: Ipv4Addr = request["local"]
        .as_str()
        .map_or(Ipv4Addr::LOCALHOST, |l| l.parse().unwrap());
    let head_request = raw.starts_with("HEAD ");
    let at = (server.log)().len();
    let mut stream = connect(server.port, local, Some(&cert.fingerprint)).await;
    // The gateway may answer before the body and close
    _ = stream.write_all(raw.as_bytes()).await;
    let mut got = Vec::new();
    let mut buf = [0u8; 16384];
    let answer = loop {
        if let Some((read, used)) = read_answer(&got, head_request)
            && (read.status != 101 || got.len() >= used)
        {
            break Some((read, used));
        }
        match tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buf)).await {
            Ok(Ok(n)) if n > 0 => got.extend_from_slice(&buf[..n]),
            _ => break read_answer(&got, head_request),
        }
    };
    let Some((read, used)) = answer else {
        return serde_json::json!({ "request": label, "status": 0, "raw": String::from_utf8_lossy(&got) });
    };
    let mut body = read.body.clone();
    if read.status == 101 {
        let mut rest = got[used..].to_vec();
        if let Some(text) = request["send"].as_str() {
            _ = stream.write_all(text.as_bytes()).await;
            let wanted = "echo:".len() + text.len();
            while rest.len() < wanted {
                match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await {
                    Ok(Ok(n)) if n > 0 => rest.extend_from_slice(&buf[..n]),
                    _ => break,
                }
            }
            body = rest;
        }
        _ = stream.shutdown().await;
        drop(stream);
    } else {
        // Read to the end, so the access line is written
        _ = tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut got)).await;
    }
    let log = lines_after(server, at, wanted_lines).await;
    observed(label, &read, &body, log)
}

fn load(name: &str, kind: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../gateway/tests/parity")
        .join(format!("{name}.{kind}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    // Recorded from Node under the old names (protocol::renamed): the client sends the new ones, and the echo
    // upstream reports them as recorded, so the answers compare as they are
    let text = if kind == "requests" {
        protocol::renamed::renamed(&text)
    } else {
        text
    };
    serde_json::from_str(&text).unwrap()
}

/// Runs every request of `<name>.requests.json` against the gateway `start` brings up with the given settings and
/// certificate, and fails with every difference from `<name>.expected.json`.
pub async fn run<F, Fut>(name: &str, start: F)
where
    F: FnOnce(Vec<(String, String)>, Arc<TestCert>) -> Fut,
    Fut: Future<Output = Server>,
{
    let requests = load(name, "requests");
    let expected = load(name, "expected");
    let expected = expected["results"].as_array().unwrap();
    let cert = Arc::new(TestCert::new());
    let backend = serve_fake(|req| upstream_answer("backend", req)).await;
    let content = serve_fake(|req| upstream_answer("content", req)).await;
    let ws = serve_fake(|req| upstream_answer("ws", req)).await;
    let (helper, sent) = fake_helper().await;
    let mut pairs: Vec<(String, String)> = vec![
        ("GATEWAY_BIND".into(), "127.0.0.1".into()),
        ("GATEWAY_SECRET".into(), SECRET.into()),
        ("GATEWAY_CERT".into(), cert.cert_file.display().to_string()),
        ("GATEWAY_KEY".into(), cert.key_file.display().to_string()),
        ("GATEWAY_BACKEND_URL".into(), format!("http://127.0.0.1:{backend}")),
        ("GATEWAY_CONTENT_URL".into(), format!("http://127.0.0.1:{content}")),
        ("GATEWAY_WS_URL".into(), format!("http://127.0.0.1:{ws}")),
        ("ALLOWLIST_URL".into(), format!("http://127.0.0.1:{helper}")),
        ("ALLOWLIST_SECRET".into(), HELPER_SECRET.into()),
    ];
    for (k, v) in requests["env"].as_object().into_iter().flatten() {
        pairs.push((k.clone(), v.as_str().unwrap().to_owned()));
    }
    let mut server = start(pairs, cert.clone()).await;
    let mut differences = Vec::new();
    for (request, wanted) in requests["requests"].as_array().unwrap().iter().zip(expected) {
        // The label and the gateway's own log say what it was sent, under the new names
        let mut wanted = wanted.clone();
        for field in ["request", "log"] {
            wanted[field] = serde_json::from_str(&protocol::renamed::renamed(&wanted[field].to_string())).unwrap();
        }
        let wanted = &wanted;
        let wanted_lines = wanted["log"].as_array().map_or(0, Vec::len);
        let got = exchange(&server, &cert, request, wanted_lines).await;
        if &got != wanted {
            differences.push(format!(
                "{}\n  expected {}\n  got      {}",
                request["label"].as_str().unwrap_or_default(),
                wanted,
                got
            ));
        }
    }
    (&mut server.idle).await;
    let feed = expected.iter().find(|e| e["request"] == "allowlist feed");
    if let Some(feed) = feed {
        let wanted = feed["body"].as_array().map_or(0, Vec::len);
        let mut addresses = Vec::new();
        for _ in 0..100 {
            addresses = sent.lock().unwrap().clone();
            addresses.sort();
            addresses.dedup();
            if addresses.len() >= wanted {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if feed["body"] != serde_json::json!(addresses) {
            differences.push(format!(
                "allowlist feed\n  expected {}\n  got      {addresses:?}",
                feed["body"]
            ));
        }
    }
    server.stop.await;
    assert!(
        differences.is_empty(),
        "{name}: {} of {} differ:\n{}",
        differences.len(),
        expected.len(),
        differences.join("\n")
    );
}
