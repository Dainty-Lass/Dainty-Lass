// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What the gateway's integration tests share: a test certificate, configs, a gateway on a free port, pinning
//! HTTPS and raw clients with a chosen local address, log capture, and fake upstreams and helper.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use bytes::Bytes;
use gateway::config::Config;
use gateway::feed::AllowlistFeed;
use gateway::gateway::Gateway;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt as _, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use service_kit::env::Env;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpSocket};

pub const SECRET: &str = "gw-secret-0123456789abcdef0123456789";
pub const HELPER_SECRET: &str = "al-secret-0123456789abcdef0123456789";

/// Tests that read the shared log, or set it, take this.
pub async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

type Log = Arc<Mutex<Vec<(service_kit::linelog::Level, Value)>>>;

fn log() -> &'static Log {
    static LOG: OnceLock<Log> = OnceLock::new();
    LOG.get_or_init(|| {
        service_kit::linelog::set_level(service_kit::linelog::Level::Debug);
        service_kit::linelog::set_sink(Some(Box::new(|line, level| {
            let line = serde_json::from_str(line).unwrap_or_else(|_| Value::String(line.to_owned()));
            log().lock().unwrap().push((level, line));
        })));
        Log::default()
    })
}

pub fn log_len() -> usize {
    log().lock().unwrap().len()
}

/// This process's linelog lines with their levels, as they are written.
pub fn log_lines() -> Log {
    log().clone()
}

pub fn log_texts() -> Vec<String> {
    log().lock().unwrap().iter().map(|(_, line)| line.to_string()).collect()
}

pub fn log_since(at: usize) -> Vec<Value> {
    log().lock().unwrap()[at..].iter().map(|(_, l)| l.clone()).collect()
}

/// The access lines since `at`.
pub fn requests_since(at: usize) -> Vec<Value> {
    log_since(at).into_iter().filter(|l| l["msg"] == "request").collect()
}

/// A certificate for 127.0.0.1 and localhost, and its fingerprint.
pub struct TestCert {
    pub dir: tempfile::TempDir,
    pub cert_file: PathBuf,
    pub key_file: PathBuf,
    pub fingerprint: String,
}

impl TestCert {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (cert_file, key_file) = (dir.path().join("gateway-cert.pem"), dir.path().join("gateway-key.pem"));
        let (loaded, _) = gateway::cert::load_or_generate(&cert_file, &key_file, &[]).unwrap();
        Self {
            dir,
            cert_file,
            key_file,
            fingerprint: loaded.info.fingerprint,
        }
    }
}

/// Settings for a test gateway: loopback, the secret, the feed to `helper` (or off), and `extra` on top.
pub fn env(cert: Option<&TestCert>, backend: u16, content: u16, helper: Option<u16>, extra: &[(&str, String)]) -> Env {
    let mut pairs: Vec<(String, String)> = vec![
        ("GATEWAY_BIND".into(), "127.0.0.1".into()),
        ("GATEWAY_PORT".into(), "1".into()),
        ("GATEWAY_SECRET".into(), SECRET.into()),
        ("GATEWAY_BACKEND_URL".into(), format!("http://127.0.0.1:{backend}")),
        ("GATEWAY_WS_URL".into(), format!("http://127.0.0.1:{backend}")),
        ("GATEWAY_CONTENT_URL".into(), format!("http://127.0.0.1:{content}")),
    ];
    match cert {
        Some(c) => {
            pairs.push(("GATEWAY_CERT".into(), c.cert_file.display().to_string()));
            pairs.push(("GATEWAY_KEY".into(), c.key_file.display().to_string()));
        }
        None => pairs.push(("GATEWAY_TLS".into(), "0".into())),
    }
    match helper {
        Some(port) => {
            pairs.push(("ALLOWLIST_URL".into(), format!("http://127.0.0.1:{port}")));
            pairs.push(("ALLOWLIST_SECRET".into(), HELPER_SECRET.into()));
        }
        None => pairs.push(("GATEWAY_ALLOWLIST".into(), "0".into())),
    }
    for (k, v) in extra {
        pairs.push(((*k).to_owned(), v.clone()));
    }
    Env::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
}

/// A gateway serving on a free loopback port until stopped.
pub struct Running {
    pub port: u16,
    pub gateway: Arc<Gateway>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Running {
    pub async fn close(self) {
        self.stop().await;
    }

    pub async fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            _ = stop.send(());
        }
        _ = tokio::time::timeout(Duration::from_secs(30), &mut self.task).await;
    }
}

pub async fn start(env: &Env) -> Running {
    let config = Config::from_env(env).unwrap();
    let tls = gateway::server::load_tls(&config).unwrap();
    let capture = gateway::server::load_capture(env).unwrap();
    let feed = AllowlistFeed::new(config.allowlist.as_ref());
    let gateway = Gateway::new(config, tls, Some(feed), capture).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let serving = gateway.clone();
    let task = tokio::spawn(async move {
        serving
            .serve_until(listener, async move {
                _ = rx.await;
            })
            .await;
    });
    Running {
        port,
        gateway,
        stop: Some(tx),
        task,
    }
}

// ---- clients ----

#[derive(Debug)]
struct Pinned(String);

impl rustls::client::danger::ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let seen: String = Sha256::digest(end_entity).iter().map(|b| format!("{b:02x}")).collect();
        if seen == self.0 {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!("fingerprint {seen}")))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::aws_lc_rs::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::aws_lc_rs::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub trait Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Stream for T {}

/// A connection from `local` (any 127.0.0.x), TLS pinned to `fingerprint` when given.
pub async fn connect(port: u16, local: Ipv4Addr, fingerprint: Option<&str>) -> Box<dyn Stream> {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind(SocketAddr::from((local, 0))).unwrap();
    let tcp = socket.connect(SocketAddr::from(([127, 0, 0, 1], port))).await.unwrap();
    match fingerprint {
        None => Box::new(tcp),
        Some(fp) => {
            let config =
                rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
                    .with_safe_default_protocol_versions()
                    .unwrap()
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(Pinned(fp.to_owned())))
                    .with_no_client_auth();
            let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
            let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
            Box::new(connector.connect(name, tcp).await.unwrap())
        }
    }
}

#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn all(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// One answer from raw bytes: the head, then a body by length, chunks, or to the end.
pub fn parse(raw: &[u8]) -> Option<(Answer, usize)> {
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
    let mut answer = Answer {
        status,
        headers,
        body: Vec::new(),
    };
    let rest = &raw[end..];
    if status == 101 || status == 100 || status == 204 || status == 304 {
        return Some((answer, end));
    }
    if let Some(len) = answer.header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        if rest.len() < len {
            return None;
        }
        answer.body = rest[..len].to_vec();
        return Some((answer, end + len));
    }
    if answer
        .header("transfer-encoding")
        .is_some_and(|v| v.contains("chunked"))
    {
        let mut at = 0;
        loop {
            let line_end = rest[at..].windows(2).position(|w| w == b"\r\n")? + at;
            let size = usize::from_str_radix(String::from_utf8_lossy(&rest[at..line_end]).trim(), 16).ok()?;
            at = line_end + 2;
            if size == 0 {
                return Some((answer, end + at + 2));
            }
            answer.body.extend_from_slice(rest.get(at..at + size)?);
            at += size + 2;
        }
    }
    answer.body = rest.to_vec();
    Some((answer, raw.len()))
}

/// Sends `request` and reads until the connection closes (or 10 s).
pub async fn exchange(
    port: u16,
    local: Ipv4Addr,
    fingerprint: Option<&str>,
    request: &[u8],
) -> (Option<Answer>, Vec<u8>) {
    let mut stream = connect(port, local, fingerprint).await;
    // A dropped connection may refuse the write
    _ = stream.write_all(request).await;
    let mut raw = Vec::new();
    _ = tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut raw)).await;
    (parse(&raw).map(|(a, _)| a), raw)
}

/// `GET`/`POST` with `Connection: close`, from 127.0.0.1.
pub async fn request(
    port: u16,
    fingerprint: Option<&str>,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Answer {
    request_from(port, Ipv4Addr::LOCALHOST, fingerprint, method, target, headers, body).await
}

#[allow(clippy::too_many_arguments)]
pub async fn request_from(
    port: u16,
    local: Ipv4Addr,
    fingerprint: Option<&str>,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Answer {
    let mut raw = format!("{method} {target} HTTP/1.1\r\nhost: play.example.org\r\nconnection: close\r\n");
    for (n, v) in headers {
        raw.push_str(&format!("{n}: {v}\r\n"));
    }
    if !body.is_empty() || method == "POST" {
        raw.push_str(&format!("content-length: {}\r\n", body.len()));
    }
    raw.push_str("\r\n");
    let mut bytes = raw.into_bytes();
    bytes.extend_from_slice(body);
    let (answer, raw) = exchange(port, local, fingerprint, &bytes).await;
    answer.unwrap_or_else(|| panic!("no answer: {:?}", String::from_utf8_lossy(&raw)))
}

// ---- fake upstreams ----

pub struct Upstream {
    pub port: u16,
    pub seen: Arc<Mutex<Vec<Value>>>,
}

/// One request an upstream saw.
pub struct Seen {
    pub url: String,
    pub headers: std::collections::HashMap<String, String>,
}

impl Upstream {
    pub fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    pub fn last(&self) -> Seen {
        let last = self.seen.lock().unwrap().last().cloned().unwrap();
        let headers = last["headers"].as_object().cloned().unwrap_or_default();
        Seen {
            url: last["url"].as_str().unwrap_or_default().to_owned(),
            headers: headers
                .into_iter()
                .map(|(k, v)| (k, v.as_str().unwrap_or_default().to_owned()))
                .collect(),
        }
    }
}

fn accept_key(key: &str) -> String {
    use sha1::Digest as _;
    let digest = sha1::Sha1::digest(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes());
    base64::engine::general_purpose::STANDARD.encode(digest)
}

/// Unmasked server frames of the echo.
fn text_frame(text: &str) -> Vec<u8> {
    let mut out = vec![0x81, text.len() as u8];
    out.extend_from_slice(text.as_bytes());
    out
}

/// A masked client text frame.
pub fn client_frame(text: &str) -> Vec<u8> {
    let mask = [1u8, 2, 3, 4];
    let mut out = vec![0x81, 0x80 | text.len() as u8];
    out.extend_from_slice(&mask);
    out.extend(text.bytes().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

async fn echo_frames(io: hyper::upgrade::Upgraded) {
    let mut io = TokioIo::new(io);
    let mut head = [0u8; 2];
    while io.read_exact(&mut head).await.is_ok() {
        let len = usize::from(head[1] & 0x7f);
        let mut mask = [0u8; 4];
        if head[1] & 0x80 != 0 && io.read_exact(&mut mask).await.is_err() {
            return;
        }
        let mut payload = vec![0u8; len];
        if io.read_exact(&mut payload).await.is_err() {
            return;
        }
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
        if head[0] & 0x0f == 8 {
            return;
        }
        let reply = format!("echo:{}", String::from_utf8_lossy(&payload));
        if io.write_all(&text_frame(&reply)).await.is_err() {
            return;
        }
    }
}

type Body = http_body_util::combinators::UnsyncBoxBody<Bytes, Infallible>;

async fn upstream_answer(
    name: &'static str,
    seen: Arc<Mutex<Vec<Value>>>,
    mut req: Request<Incoming>,
) -> Response<Body> {
    let path = req.uri().path().to_owned();
    let url = req.uri().to_string();
    let mut headers = serde_json::Map::new();
    for (n, v) in req.headers() {
        if n != "connection" {
            headers.insert(n.as_str().to_owned(), json!(String::from_utf8_lossy(v.as_bytes())));
        }
    }
    if req.headers().contains_key("upgrade") {
        seen.lock()
            .unwrap()
            .push(json!({ "upstream": name, "url": url, "headers": headers }));
        if path == "/refuse" {
            let mut r = Response::new(Full::new(Bytes::from_static(b"no room")).boxed_unsync());
            *r.status_mut() = StatusCode::NOT_FOUND;
            r.headers_mut().insert("content-type", "text/plain".parse().unwrap());
            return r;
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
        h.insert("connection", "Upgrade".parse().unwrap());
        h.insert("sec-websocket-accept", accept_key(&key).parse().unwrap());
        h.insert("x-powered-by", "Express".parse().unwrap());
        if let Some(p) = protocol {
            h.insert("sec-websocket-protocol", p);
        }
        return r;
    }
    let method = req.method().to_string();
    let body = req
        .into_body()
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    if path == "/hang" {
        std::future::pending::<()>().await;
    }
    if path == "/slow" {
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
    let record = json!({
        "upstream": name, "method": method, "url": url, "headers": headers, "bodyLength": body.len(),
        "bodySha": Sha256::digest(&body).iter().map(|b| format!("{b:02x}")).collect::<String>(),
    });
    seen.lock().unwrap().push(record.clone());
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
        h.insert("x-powered-by", "Express".parse().unwrap());
        h.insert("x-reforged-gateway", "leak".parse().unwrap());
        return r;
    }
    if path == "/heartbeat" && method == "POST" {
        let mut r = Response::new(Full::new(Bytes::from_static(b"20000")).boxed_unsync());
        r.headers_mut().insert("content-type", "text/plain".parse().unwrap());
        return r;
    }
    let mut r = Response::new(Full::new(Bytes::from(record.to_string())).boxed_unsync());
    r.headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    r
}

/// A backend or content server that echoes what reached it.
pub async fn upstream(name: &'static str) -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let record = seen.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let seen = record.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |req| {
                    let seen = seen.clone();
                    async move { Ok::<_, Infallible>(upstream_answer(name, seen, req).await) }
                });
                _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .with_upgrades()
                    .await;
            });
        }
    });
    Upstream { port, seen }
}

/// A port nothing listens on.
pub async fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

/// A helper that records the addresses it was sent.
pub async fn fake_helper(status: u16) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let seen = record.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |req: Request<Incoming>| {
                    let seen = seen.clone();
                    async move {
                        let secret_ok = req
                            .headers()
                            .get("x-allowlist-secret")
                            .is_some_and(|v| v == HELPER_SECRET);
                        let body = req
                            .into_body()
                            .collect()
                            .await
                            .map(|b| b.to_bytes())
                            .unwrap_or_default();
                        let value: Value = serde_json::from_slice(&body).unwrap_or_default();
                        if secret_ok {
                            seen.lock()
                                .unwrap()
                                .push(value["ip"].as_str().unwrap_or_default().to_owned());
                        }
                        let mut r = Response::new(Full::new(Bytes::from_static(b"{}")));
                        *r.status_mut() = StatusCode::from_u16(status).unwrap();
                        Ok::<_, Infallible>(r)
                    }
                });
                _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    (port, seen)
}

mod compat;
#[allow(unused_imports)] // not every test calls the moved tests' helpers
pub use compat::*;
