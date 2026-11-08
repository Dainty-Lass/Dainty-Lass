// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The parity harness (spec section 3.1): builds a script's fixture folders and fake backend, starts a server
//! through `launch`, sends the script's raw requests, and compares the replies with the ones recorded from the
//! Node server. `content-plugin`'s test includes this file by `#[path]`, so it finds the fixtures from either
//! crate and needs only `serde_json`, `service-kit`, `sha2`, `tempfile` and `tokio`.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "shared by several test binaries, each using only some; a panic is how a test fails"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use serde_json::{Map, Value, json};
use sha2::Digest as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

/// A server the harness runs a script against: where it listens, and what stops it.
pub struct Server {
    pub port: u16,
    pub stop: Pin<Box<dyn Future<Output = ()> + Send>>,
}

/// Every file the harness writes gets this modification time, so the branding ETags match the recording.
pub const FIXTURE_MTIME_SECS: u64 = 1_700_000_000;
/// The headers a reply is compared on.
pub const COMPARED_HEADERS: [&str; 11] = [
    "content-type",
    "content-length",
    "content-range",
    "etag",
    "accept-ranges",
    "cache-control",
    "retry-after",
    "allow",
    "access-control-allow-origin",
    "cross-origin-resource-policy",
    "x-content-type-options",
];
const MAX_TEXT_BODY: usize = 4096;

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../dainty-content/tests/parity")
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    // Recorded from Node under the old names (dainty_protocol::renamed)
    let text = dainty_protocol::renamed::renamed(text.trim_start_matches('\u{feff}'));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().fold(String::new(), |mut out, b| {
        _ = write!(out, "{b:02x}");
        out
    })
}

/// A file's contents: `{"text"}`, `{"hex"}` or `{"pattern": n}` (`(i*7+3) & 0xff`).
fn contents(spec: &Value) -> Vec<u8> {
    if let Some(text) = spec.get("text").and_then(Value::as_str) {
        return text.as_bytes().to_vec();
    }
    if let Some(hex) = spec.get("hex").and_then(Value::as_str) {
        return (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("hex"))
            .collect();
    }
    if let Some(n) = spec.get("pattern").and_then(Value::as_u64) {
        return (0..n).map(|i| ((i * 7 + 3) & 0xff) as u8).collect();
    }
    if let Some(text) = spec.as_str() {
        return text.as_bytes().to_vec();
    }
    panic!("a file spec is text, hex or pattern: {spec}")
}

/// `files` as `(name, contents)` in the script's order: an object of name to spec, or a list of
/// `{"name", ...spec}` or `[name, spec]`.
fn file_list(files: Option<&Value>) -> Vec<(String, Vec<u8>)> {
    match files {
        Some(Value::Object(map)) => map.iter().map(|(name, spec)| (name.clone(), contents(spec))).collect(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Array(pair) => (pair[0].as_str().expect("name").to_owned(), contents(&pair[1])),
                Value::Object(entry) => {
                    let name = entry["name"].as_str().expect("name").to_owned();
                    let spec = entry
                        .get("spec")
                        .cloned()
                        .unwrap_or_else(|| Value::Object(entry.clone()));
                    (name, contents(&spec))
                }
                other => panic!("a file entry: {other}"),
            })
            .collect(),
        None | Some(Value::Null) => Vec::new(),
        Some(other) => panic!("files: {other}"),
    }
}

fn write_file(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(UNIX_EPOCH + Duration::from_secs(FIXTURE_MTIME_SECS))
        .unwrap();
}

/// An object as `JSON.stringify` writes it, a string as is, with each `$sha:<file>` replaced by its SHA-256.
fn text_of(value: &Value, hashes: &[(String, String)]) -> String {
    let mut text = match value {
        Value::String(text) => text.clone(),
        other => dainty_service_kit::jsjson::stringify(other),
    };
    let mut hashes: Vec<_> = hashes.iter().collect();
    hashes.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    for (name, hash) in hashes {
        text = text.replace(&format!("$sha:{name}"), hash);
    }
    text
}

/// The fake backend: a key in `keys` gets its status and body, anything else 401 `{}`.
async fn fake_backend(keys: Map<String, Value>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let keys = Arc::new(keys);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let keys = keys.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buf = [0u8; 4096];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&head).into_owned();
                let mut lines = text.split("\r\n");
                let target = lines
                    .next()
                    .unwrap_or_default()
                    .split(' ')
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                let key = lines.find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("x-dainty-user-api-key")
                        .then(|| value.trim().to_owned())
                });
                let answer = key
                    .filter(|_| target == "/dainty/api/GetUserInfo")
                    .and_then(|key| keys.get(&key).cloned());
                let (status, body) = match answer {
                    Some(answer) => (
                        answer["status"].as_u64().unwrap_or(200),
                        match &answer["body"] {
                            Value::String(text) => text.clone(),
                            other => serde_json::to_string(other).unwrap(),
                        },
                    ),
                    None => (401, "{}".to_owned()),
                };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                _ = socket.write_all(reply.as_bytes()).await;
                _ = socket.shutdown().await;
            });
        }
    });
    format!("http://{addr}")
}

/// One raw request on a new connection, read to the end of the stream.
async fn send(port: u16, method: &str, path: &str, headers: &[(String, String)]) -> Vec<u8> {
    let mut request = format!("{method} {path} HTTP/1.1\r\nhost: content\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("connection: close\r\n\r\n");
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    socket.write_all(request.as_bytes()).await.unwrap();
    let mut reply = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), socket.read_to_end(&mut reply))
        .await
        .expect("the reply ends")
        .ok();
    reply
}

struct Parsed {
    status: u64,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn parse_reply(raw: &[u8]) -> Parsed {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("a reply head");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let mut body = raw[split + 4..].to_vec();
    if headers
        .iter()
        .any(|(n, v)| n == "transfer-encoding" && v.eq_ignore_ascii_case("chunked"))
    {
        body = dechunk(&body);
    }
    Parsed { status, headers, body }
}

fn dechunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(end) = data.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&data[..end]).trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        out.extend_from_slice(&data[end + 2..end + 2 + size]);
        data = &data[end + 2 + size + 2..];
    }
    out
}

fn body_text(body: &[u8]) -> String {
    match std::str::from_utf8(body) {
        Ok(text) if body.len() <= MAX_TEXT_BODY && !body.iter().any(|&b| b <= 0x08) => text.to_owned(),
        _ => format!("<{} bytes sha256 {}>", body.len(), sha256_hex(body)),
    }
}

/// A recorded or actual reply, compared on what the script format names.
#[derive(Debug, PartialEq)]
struct Compared {
    request: String,
    status: u64,
    headers: BTreeMap<String, String>,
    body: String,
}

fn compared(entry: &Value) -> Compared {
    let headers = match &entry["headers"] {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                (
                    k.to_ascii_lowercase(),
                    v.as_str().map_or_else(|| v.to_string(), str::to_owned),
                )
            })
            .collect(),
        Value::Array(pairs) => pairs
            .iter()
            .map(|p| {
                (
                    p[0].as_str().unwrap().to_ascii_lowercase(),
                    p[1].as_str().unwrap().to_owned(),
                )
            })
            .collect(),
        _ => BTreeMap::new(),
    };
    let body = entry["body"].as_str().unwrap_or_default().to_owned();
    // A hashed body compares whatever the case of its hex
    let body = if body.starts_with('<') && body.contains(" bytes sha256 ") {
        body.to_ascii_lowercase()
    } else {
        body
    };
    Compared {
        request: entry["request"].as_str().unwrap_or_default().to_owned(),
        status: entry["status"].as_u64().unwrap_or_default(),
        headers,
        body,
    }
}

/// Runs script `name` against the server `launch` starts on the script's env, and compares the replies.
pub async fn run<F: Future<Output = Server>>(name: &str, launch: impl FnOnce(Vec<(String, String)>) -> F) {
    let dir = fixtures();
    let script = read_json(&dir.join(format!("{name}.requests.json")));
    let expected = read_json(&dir.join(format!("{name}.expected.json")));
    let actual = play(&script, launch).await;
    let recorded = expected["results"].as_array().expect("results").clone();
    let recorded: Vec<Compared> = recorded.iter().map(compared).collect();
    let actual: Vec<Compared> = actual.iter().map(compared).collect();
    for (i, (want, got)) in recorded.iter().zip(&actual).enumerate() {
        assert_eq!(got, want, "{name}: step {i}");
    }
    assert_eq!(actual.len(), recorded.len(), "{name}: number of replies");
}

/// Plays a script and returns its replies in the recording's shape.
pub async fn play<F: Future<Output = Server>>(
    script: &Value,
    launch: impl FnOnce(Vec<(String, String)>) -> F,
) -> Vec<Value> {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mut env: Vec<(String, String)> = Vec::new();
    let runtime_dir = root.join("runtime");
    let runtime = script.get("runtime").filter(|r| !r.is_null());
    if let Some(runtime) = runtime {
        std::fs::create_dir_all(&runtime_dir).unwrap();
        let files = file_list(runtime.get("files"));
        let hashes: Vec<(String, String)> = files.iter().map(|(n, b)| (n.clone(), sha256_hex(b))).collect();
        for (name, bytes) in &files {
            write_file(&runtime_dir.join(name), bytes);
        }
        let index = match runtime.get("index").filter(|i| !i.is_null()) {
            Some(index) => text_of(index, &hashes),
            None => {
                let listed: Vec<Value> = files
                    .iter()
                    .map(|(name, bytes)| json!({"name": name, "size": bytes.len(), "sha256": sha256_hex(bytes)}))
                    .collect();
                serde_json::to_string(&json!({
                    "version": "20261014-1a2b3c4d",
                    "build": {"changelist": 647472},
                    "files": listed,
                }))
                .unwrap()
            }
        };
        write_file(&runtime_dir.join("runtime.json"), index.as_bytes());
        env.push(("CONTENT_RUNTIME_DIR".into(), runtime_dir.display().to_string()));
    }
    if let Some(branding) = script.get("branding").filter(|b| !b.is_null()) {
        let art = root.join("art");
        std::fs::create_dir_all(&art).unwrap();
        for (name, bytes) in file_list(branding.get("files")) {
            write_file(&art.join(name), &bytes);
        }
        if let Some(config) = branding.get("json").filter(|j| !j.is_null()) {
            write_file(&art.join("branding.json"), text_of(config, &[]).as_bytes());
        }
        env.push(("CONTENT_BRANDING_DIR".into(), art.display().to_string()));
    }
    if let Some(news) = script.get("news").filter(|n| !n.is_null()) {
        let file = root.join("news.json");
        write_file(&file, text_of(news, &[]).as_bytes());
        env.push(("CONTENT_NEWS_FILE".into(), file.display().to_string()));
    }
    let keys = script
        .get("keys")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    env.push(("BACKEND_URL".into(), fake_backend(keys).await));
    if let Some(Value::Object(extra)) = script.get("env") {
        for (name, value) in extra {
            let value = value.as_str().map_or_else(|| value.to_string(), str::to_owned);
            env.push((name.clone(), value));
        }
    }
    let server = launch(env).await;
    let mut results = Vec::new();
    let mut last_etag: Option<String> = None;
    for step in script["requests"].as_array().expect("requests") {
        if let Some(name) = step.get("write").and_then(Value::as_str) {
            write_file(&runtime_dir.join(name), &contents(&step["spec"]));
            continue;
        }
        let method = step.get("method").and_then(Value::as_str).unwrap_or("GET");
        let path = step["path"].as_str().expect("path");
        let headers: Vec<(String, String)> = step
            .get("headers")
            .and_then(Value::as_array)
            .map(|pairs| {
                pairs
                    .iter()
                    .map(|pair| {
                        let value = pair[1].as_str().unwrap_or_default();
                        let value = match &last_etag {
                            Some(etag) => value.replacen("$etag", etag, 1),
                            None => value.to_owned(),
                        };
                        (pair[0].as_str().unwrap_or_default().to_owned(), value)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let reply = parse_reply(&send(server.port, method, path, &headers).await);
        if let Some((_, etag)) = reply.headers.iter().find(|(n, _)| n == "etag") {
            last_etag = Some(etag.clone());
        }
        let kept: Map<String, Value> = reply
            .headers
            .iter()
            .filter(|(name, _)| COMPARED_HEADERS.contains(&name.as_str()))
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect();
        results.push(json!({
            "request": format!("{method} {path}"),
            "status": reply.status,
            "headers": kept,
            "body": body_text(&reply.body),
        }));
    }
    server.stop.await;
    results
}
