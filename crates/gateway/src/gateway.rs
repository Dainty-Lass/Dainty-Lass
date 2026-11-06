// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The listener: admission, TLS (split by ALPN when node links are on), the connection timers, the policy, the
//! rate and body limits, proxying with the secret set, WebSocket tunnels, node links, enrollment, the network
//! routes, the feed, the access log and the capture.

use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http::header::{CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, HeaderName, HeaderValue, TRANSFER_ENCODING};
use http::{HeaderMap, Method, Request, Response, StatusCode};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt as _, Empty, Full};
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo};
use module_api::network::NodeId;
use network_plugin::cluster::NODE_ALPN;
use network_plugin::endpoint::{Head, Reply};
use network_plugin::link::MAX_MESSAGE;
use network_plugin::wire::{self, ErrorBody};
use serde_json::{Map, Value, json};
use service_kit::js;
use service_kit::linelog::{self, Throttle};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::time::{Instant, Sleep};

use crate::capture::{BodyTap, Capture, CaptureConn, ChatTap, HttpRecord};
use crate::cert::CertInfo;
use crate::config::{Config, Endpoint};
use crate::feed::AllowlistFeed;
use crate::guard::{Guard, Phases};
use crate::ip::{peer_address, rate_limit_key};
use crate::modules::ModuleRoutes;
use crate::network::RegionReportHandler;
use crate::nodelink::NodeLinks;
use crate::policy::{Decision, FeedKind, LimitClass, NetworkRoute, Upstream, has_bearer};
use crate::ratelimit::{RateLimiter, Take};
use crate::redact::{redact_target, redact_user_agent};

type GwBody = UnsyncBoxBody<Bytes, io::Error>;

/// The public certificate's server settings and what it says about itself.
#[derive(Clone)]
pub struct Tls {
    /// Server settings with the certificate and key.
    pub config: Arc<rustls::ServerConfig>,
    /// The certificate's details.
    pub info: CertInfo,
}

/// Open connections and WebSockets.
pub struct Stats {
    connections: watch::Sender<usize>,
    per_key: Mutex<HashMap<String, usize>>,
    websockets: AtomicUsize,
}

impl Stats {
    fn new() -> Self {
        Self {
            connections: watch::channel(0).0,
            per_key: Mutex::default(),
            websockets: AtomicUsize::new(0),
        }
    }

    /// Connections open now.
    pub fn connections(&self) -> usize {
        *self.connections.borrow()
    }

    /// WebSockets open now.
    pub fn websockets(&self) -> usize {
        self.websockets.load(Ordering::Relaxed)
    }
}

struct Limiters {
    general: RateLimiter,
    content: RateLimiter,
    register: RateLimiter,
    token: RateLimiter,
    ws: RateLimiter,
    connect: RateLimiter,
}

impl Limiters {
    fn class(&self, class: LimitClass) -> &RateLimiter {
        match class {
            LimitClass::General => &self.general,
            LimitClass::Content => &self.content,
            LimitClass::Register => &self.register,
            LimitClass::Token => &self.token,
            LimitClass::Ws => &self.ws,
        }
    }

    fn sweep(&self) {
        for l in [
            &self.general,
            &self.content,
            &self.register,
            &self.token,
            &self.ws,
            &self.connect,
        ] {
            l.sweep();
        }
    }
}

struct Shared {
    config: Config,
    tls: Option<Tls>,
    routes: Arc<ModuleRoutes>,
    feed: AllowlistFeed,
    capture: Option<Capture>,
    limiters: Limiters,
    stats: Stats,
    closing: watch::Sender<bool>,
    abort: watch::Sender<bool>,
    node_links: RwLock<Option<Arc<NodeLinks>>>,
    region: RwLock<Option<RegionReportHandler>>,
    client: Client<HttpConnector, UpBody>,
    throttle: Throttle,
}

/// The gateway. Cheap to clone.
#[derive(Clone)]
pub struct Gateway(Arc<Shared>);

/// What one connection's requests share.
#[derive(Clone)]
struct Conn {
    ip: String,
    key: String,
    phases: Phases,
    capture: Option<CaptureConn>,
    node: Option<NodeId>,
}

/// Request headers without the hop-by-hop ones and the client's address and origin claims, then the forwarded
/// address and protocol and the gateway secret, set rather than appended.
pub fn forward_headers(headers: &HeaderMap, ip: &str, tls: bool, secret: &str, upgrade: bool) -> HeaderMap {
    const CLAIMS: [&str; 13] = [
        "x-forwarded-for",
        "x-forwarded-proto",
        "x-forwarded-host",
        "x-forwarded-port",
        "x-forwarded-prefix",
        "forwarded",
        "x-real-ip",
        "x-client-ip",
        "true-client-ip",
        "cf-connecting-ip",
        "x-dainty-gateway",
        // A client's copy of the old name goes too, so no upstream ever sees one
        "x-reforged-gateway",
        "expect",
    ];
    let mut out = proxy::end_to_end(headers, |name| CLAIMS.contains(&name));
    let set = |out: &mut HeaderMap, name: &'static str, value: &str| {
        if let Ok(value) = HeaderValue::from_str(value) {
            out.insert(HeaderName::from_static(name), value);
        }
    };
    set(&mut out, "x-forwarded-for", ip);
    set(&mut out, "x-forwarded-proto", if tls { "https" } else { "http" });
    set(&mut out, "x-dainty-gateway", secret);
    if upgrade {
        set(&mut out, "connection", "Upgrade");
        set(&mut out, "upgrade", "websocket");
    }
    out
}

fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "timed out")
}

fn full(bytes: impl Into<Bytes>) -> GwBody {
    Full::new(bytes.into()).map_err(|never| match never {}).boxed_unsync()
}

/// A body that declared something: `transfer-encoding`, or a `content-length` above 0 read as a number.
fn has_body(headers: &HeaderMap) -> bool {
    headers.contains_key(TRANSFER_ENCODING)
        || headers
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| js::number(Some(&Value::String(v.to_owned()))) > 0.0)
}

fn declared_length(headers: &HeaderMap) -> Option<f64> {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .map(|v| js::number(Some(&Value::String(v.to_owned()))))
}

/// `<CODE>` of `upstream_<CODE>`: the first I/O error in the chain decides.
fn error_code(error: &(dyn std::error::Error + 'static)) -> &'static str {
    let mut current = Some(error);
    while let Some(e) = current {
        if let Some(io) = e.downcast_ref::<io::Error>() {
            return match io.kind() {
                io::ErrorKind::ConnectionRefused => "ECONNREFUSED",
                io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::UnexpectedEof => "ECONNRESET",
                io::ErrorKind::TimedOut => "ETIMEDOUT",
                io::ErrorKind::AddrNotAvailable => "EADDRNOTAVAIL",
                _ => "error",
            };
        }
        if let Some(h) = e.downcast_ref::<hyper::Error>()
            && (h.is_incomplete_message() || h.is_closed())
        {
            return "ECONNRESET";
        }
        current = e.source();
    }
    "error"
}

/// The access line and the capture's record, written once when the answer ends or is dropped.
struct Access {
    gateway: Gateway,
    conn: Conn,
    method: String,
    target: String,
    route: &'static str,
    metric: &'static str,
    status: u16,
    bytes_in: Arc<AtomicU64>,
    started: std::time::Instant,
    reason: Option<String>,
    ua: Option<String>,
    /// Only bytes from an upstream count as out
    upstream_bytes: bool,
    declared: Option<u64>,
    capture: Option<CaptureParts>,
}

struct CaptureParts {
    req_headers: HeaderMap,
    req_body: Arc<Mutex<BodyTap>>,
    res_headers: HeaderMap,
    res_body: Arc<Mutex<BodyTap>>,
}

impl Access {
    fn write(self, bytes_out: u64, ended: bool) {
        let ms = self.started.elapsed().as_millis() as u64;
        let bytes_out = if self.upstream_bytes { bytes_out } else { 0 };
        let aborted = !ended && self.declared.is_none_or(|d| bytes_out < d);
        let mut line = Map::new();
        line.insert("ip".into(), json!(self.conn.ip));
        line.insert("method".into(), json!(self.method));
        line.insert("target".into(), json!(redact_target(&self.target)));
        line.insert("route".into(), json!(self.route));
        line.insert("status".into(), json!(self.status));
        line.insert("bytesIn".into(), json!(self.bytes_in.load(Ordering::Relaxed)));
        line.insert("bytesOut".into(), json!(bytes_out));
        line.insert("ms".into(), json!(ms));
        if let Some(reason) = &self.reason {
            line.insert("reason".into(), json!(reason));
        }
        if let Some(ua) = &self.ua {
            line.insert("ua".into(), json!(ua));
        }
        if aborted {
            line.insert("aborted".into(), json!(true));
        }
        linelog::info("request", Value::Object(line));
        service_kit::metrics::http_request("gateway", self.metric, &self.method, self.status, self.started);
        if let (Some(conn), Some(parts)) = (&self.conn.capture, &self.capture) {
            let lock = |t: &Arc<Mutex<BodyTap>>| t.lock().unwrap_or_else(PoisonError::into_inner).clone();
            conn.http(&HttpRecord {
                ip: &self.conn.ip,
                method: &self.method,
                target: &self.target,
                route: self.route,
                req_headers: &parts.req_headers,
                req_body: &lock(&parts.req_body),
                status: self.status,
                res_headers: &parts.res_headers,
                res_body: &lock(&parts.res_body),
                ms,
                aborted,
            });
        }
        self.conn.phases.answer_done();
        drop(self.gateway);
    }
}

/// The answer's body, counted; on its end or drop the access line goes out.
struct Tracked {
    inner: GwBody,
    out: u64,
    access: Option<Access>,
    tap: Option<Arc<Mutex<BodyTap>>>,
}

impl Body for Tracked {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let polled = Pin::new(&mut self.inner).poll_frame(cx);
        match &polled {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    self.out += data.len() as u64;
                    if let Some(tap) = &self.tap {
                        tap.lock().unwrap_or_else(PoisonError::into_inner).push(data);
                    }
                }
            }
            Poll::Ready(None) => {
                if let Some(access) = self.access.take() {
                    access.write(self.out, true);
                }
            }
            _ => {}
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        if let Some(access) = self.access.take() {
            let ended = self.inner.is_end_stream();
            access.write(self.out, ended);
        }
    }
}

/// How an exchange's request body ended.
#[derive(Default)]
struct Exchange {
    bytes_in: Arc<AtomicU64>,
    over_cap: AtomicBool,
    timed_out: AtomicBool,
    failed: AtomicBool,
}

/// The client's body on its way upstream: counted, capped and timed.
pub struct UpBody {
    inner: Option<Incoming>,
    cap: u64,
    sleep: Pin<Box<Sleep>>,
    exchange: Arc<Exchange>,
    tap: Option<Arc<Mutex<BodyTap>>>,
}

impl Body for UpBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let this = &mut *self;
        let Some(inner) = this.inner.as_mut() else {
            return Poll::Ready(None);
        };
        match Pin::new(inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    let total =
                        this.exchange.bytes_in.fetch_add(data.len() as u64, Ordering::Relaxed) + data.len() as u64;
                    if let Some(tap) = &this.tap {
                        tap.lock().unwrap_or_else(PoisonError::into_inner).push(data);
                    }
                    if total > this.cap {
                        this.exchange.over_cap.store(true, Ordering::Relaxed);
                        this.inner = None;
                        return Poll::Ready(Some(Err(io::Error::other("body over the cap"))));
                    }
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(e))) => {
                this.exchange.failed.store(true, Ordering::Relaxed);
                this.inner = None;
                Poll::Ready(Some(Err(io::Error::other(e))))
            }
            Poll::Ready(None) => {
                this.inner = None;
                Poll::Ready(None)
            }
            Poll::Pending => {
                if this.sleep.as_mut().poll(cx).is_ready() {
                    this.exchange.timed_out.store(true, Ordering::Relaxed);
                    this.inner = None;
                    return Poll::Ready(Some(Err(timed_out())));
                }
                Poll::Pending
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.as_ref().is_none_or(Body::is_end_stream)
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.as_ref().map(Body::size_hint).unwrap_or_default()
    }
}

/// An upstream answer's body that fails when no frame comes for the upstream idle time.
struct IdleBody {
    inner: Incoming,
    idle: Duration,
    sleep: Pin<Box<Sleep>>,
}

impl Body for IdleBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(frame) => {
                let next = Instant::now() + this.idle;
                this.sleep.as_mut().reset(next);
                Poll::Ready(frame.map(|f| f.map_err(io::Error::other)))
            }
            Poll::Pending => {
                if this.sleep.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(Some(Err(io::Error::new(io::ErrorKind::TimedOut, "upstream timed out"))));
                }
                Poll::Pending
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

enum Limited {
    TooLarge,
    Failed,
    Clock,
}

/// A body read whole, up to `limit` bytes, within `within`.
async fn read_limited(body: Incoming, limit: usize, within: Duration, count: &AtomicU64) -> Result<Vec<u8>, Limited> {
    let read = async {
        let mut body = body;
        let mut out = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|_| Limited::Failed)?;
            if let Some(data) = frame.data_ref() {
                count.fetch_add(data.len() as u64, Ordering::Relaxed);
                if out.len() + data.len() > limit {
                    return Err(Limited::TooLarge);
                }
                out.extend_from_slice(data);
            }
        }
        Ok(out)
    };
    tokio::time::timeout(within, read).await.unwrap_or(Err(Limited::Clock))
}

fn is_upgrade(req: &Request<Incoming>) -> bool {
    req.headers().contains_key(http::header::UPGRADE) && proxy::connection_tokens(req.headers()).contains("upgrade")
}

/// Admitted connections are counted until dropped.
struct Admitted {
    gateway: Gateway,
    key: String,
}

impl Drop for Admitted {
    fn drop(&mut self) {
        let stats = &self.gateway.0.stats;
        {
            let mut per = stats.per_key.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(n) = per.get_mut(&self.key) {
                *n -= 1;
                if *n == 0 {
                    per.remove(&self.key);
                }
            }
        }
        stats.connections.send_modify(|n| *n = n.saturating_sub(1));
    }
}

impl Gateway {
    /// A gateway that routes to the stand-in for the configured game.
    pub fn new(
        config: Config,
        tls: Option<Tls>,
        feed: Option<AllowlistFeed>,
        capture: Option<Capture>,
    ) -> Result<Arc<Self>, String> {
        let routes = Arc::new(ModuleRoutes::standalone(&config)?);
        Self::with_routes(config, tls, feed, capture, routes)
    }

    /// `feed` `None` is no feed; node links are loaded when the config names them (section 9.1).
    pub fn with_routes(
        config: Config,
        tls: Option<Tls>,
        feed: Option<AllowlistFeed>,
        capture: Option<Capture>,
        routes: Arc<ModuleRoutes>,
    ) -> Result<Arc<Self>, String> {
        let feed = feed.unwrap_or_else(|| AllowlistFeed::new(None));
        match (config.tls, &tls) {
            (true, None) => return Err("the TLS gateway needs its certificate and key".to_owned()),
            (false, Some(_)) => {
                return Err("a private gateway (GATEWAY_TLS=0) serves plain HTTP and takes no certificate".to_owned());
            }
            _ => {}
        }
        let rates = config.limits.rates;
        let mut connector = HttpConnector::new();
        connector.set_nodelay(true);
        let client = Client::builder(TokioExecutor::new())
            .pool_max_idle_per_host(64)
            .pool_idle_timeout(Duration::from_secs(30))
            .build(connector);
        let gateway = Arc::new(Self(Arc::new(Shared {
            limiters: Limiters {
                general: RateLimiter::new(rates.general),
                content: RateLimiter::new(rates.content),
                register: RateLimiter::new(rates.register),
                token: RateLimiter::new(rates.token),
                ws: RateLimiter::new(rates.ws),
                connect: RateLimiter::new(rates.connect),
            },
            config,
            tls,
            routes,
            feed,
            capture,
            stats: Stats::new(),
            closing: watch::channel(false).0,
            abort: watch::channel(false).0,
            node_links: RwLock::default(),
            region: RwLock::default(),
            client,
            throttle: Throttle::new(60_000),
        })));
        if let Some(links) = &gateway.0.config.node_links {
            gateway.set_node_links(Some(Arc::new(NodeLinks::load(links)?)));
        }
        Ok(gateway)
    }

    /// Its settings.
    pub fn config(&self) -> &Config {
        &self.0.config
    }

    /// The public certificate's details; `None` in private mode.
    pub fn certificate(&self) -> Option<&CertInfo> {
        self.0.tls.as_ref().map(|t| &t.info)
    }

    /// Connection counts.
    pub fn stats(&self) -> &Stats {
        &self.0.stats
    }

    /// The module routes.
    pub fn routes(&self) -> &Arc<ModuleRoutes> {
        &self.0.routes
    }

    /// The allowlist feed.
    pub fn feed(&self) -> &AllowlistFeed {
        &self.0.feed
    }

    /// Sets what takes `POST /network/v1/regions` reports.
    pub fn set_region_handler(&self, handler: RegionReportHandler) {
        *self.0.region.write().unwrap_or_else(PoisonError::into_inner) = Some(handler);
    }

    /// The node links, when loaded.
    pub fn node_links(&self) -> Option<Arc<NodeLinks>> {
        self.0.node_links.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Node links on the public listener (public mode only).
    pub fn set_node_links(&self, links: Option<Arc<NodeLinks>>) {
        *self.0.node_links.write().unwrap_or_else(PoisonError::into_inner) = links;
    }

    /// Whether it is shutting down.
    pub fn is_closing(&self) -> bool {
        *self.0.closing.borrow()
    }

    fn warn_throttled(&self, key: &str, msg: &str, fields: Value) {
        if self.0.throttle.allow(key, crate::now_ms()).is_some() {
            linelog::warn(msg, fields);
        }
    }

    /// Serves until `stop` turns true.
    pub async fn serve(self: Arc<Self>, listener: TcpListener, mut stop: watch::Receiver<bool>) {
        self.serve_until(listener, async move {
            while !*stop.borrow_and_update() {
                if stop.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        })
        .await;
    }

    /// Serves until `stop`, then shuts down gracefully within the grace, and lets the feed send what it queued.
    pub async fn serve_until(&self, listener: TcpListener, stop: impl Future<Output = ()>) {
        let sweeper = {
            let gateway = self.clone();
            tokio::spawn(async move {
                let mut every = tokio::time::interval(Duration::from_secs(60));
                every.tick().await;
                loop {
                    every.tick().await;
                    gateway.0.limiters.sweep();
                }
            })
        };
        tokio::pin!(stop);
        loop {
            tokio::select! {
                () = &mut stop => break,
                accepted = listener.accept() => {
                    if let Ok((stream, addr)) = accepted {
                        self.admit(stream, addr);
                    }
                }
            }
        }
        drop(listener);
        self.0.closing.send_replace(true);
        let grace = self.0.config.timeouts.shutdown_grace;
        let began = Instant::now();
        let mut open = self.0.stats.connections.subscribe();
        _ = tokio::time::timeout(grace, open.wait_for(|n| *n == 0)).await;
        self.0.abort.send_replace(true);
        let left = (grace + Duration::from_secs(5)).saturating_sub(began.elapsed());
        _ = tokio::time::timeout(left, async {
            _ = open.wait_for(|n| *n == 0).await;
            self.0.feed.idle().await;
        })
        .await;
        sweeper.abort();
    }

    /// Section 3.1: admitted or dropped before any byte is read.
    fn admit(&self, stream: TcpStream, addr: SocketAddr) {
        if self.is_closing() {
            return;
        }
        let ip = peer_address(Some(addr));
        let key = rate_limit_key(&ip);
        let limits = &self.0.config.limits;
        let stats = &self.0.stats;
        if stats.connections() >= limits.max_connections {
            self.warn_throttled(
                "drop",
                "connection refused: the gateway is at its connection limit",
                json!({ "ip": ip, "max": limits.max_connections }),
            );
            return;
        }
        {
            let per = stats.per_key.lock().unwrap_or_else(PoisonError::into_inner);
            let open = per.get(&key).copied().unwrap_or(0);
            if open >= limits.max_connections_per_ip {
                drop(per);
                self.warn_throttled(
                    &format!("perip:{key}"),
                    "connection refused: too many open connections from one address",
                    json!({ "ip": ip, "open": open }),
                );
                return;
            }
        }
        if let Take::Limited { .. } = self.0.limiters.connect.take(&key) {
            self.warn_throttled(
                &format!("connrate:{key}"),
                "connection refused: too many new connections from one address",
                json!({ "ip": ip }),
            );
            return;
        }
        *stats
            .per_key
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(key.clone())
            .or_insert(0) += 1;
        stats.connections.send_modify(|n| *n += 1);
        let admitted = Admitted {
            gateway: self.clone(),
            key: key.clone(),
        };
        let gateway = self.clone();
        tokio::spawn(async move {
            gateway.connection(stream, ip, key).await;
            drop(admitted);
        });
    }

    async fn connection(&self, stream: TcpStream, ip: String, key: String) {
        _ = stream.set_nodelay(true);
        let Some(tls) = self.0.tls.clone() else {
            self.serve_http(stream, ip, key, None).await;
            return;
        };
        let links = self.node_links();
        let handshake = async {
            let start = tokio_rustls::LazyConfigAcceptor::new(rustls::server::Acceptor::default(), stream).await?;
            let node = links.as_ref().is_some_and(|_| {
                start
                    .client_hello()
                    .alpn()
                    .is_some_and(|mut names| names.any(|n| n == NODE_ALPN))
            });
            let config = match (&links, node) {
                (Some(links), true) => links.server_config().clone(),
                _ => tls.config.clone(),
            };
            let stream = start.into_stream(config).await?;
            Ok::<_, io::Error>((stream, node))
        };
        let timeout = self.0.config.timeouts.handshake;
        let (stream, node) = match tokio::time::timeout(timeout, handshake).await {
            Ok(Ok(done)) => done,
            failed => {
                let error = match failed {
                    Ok(Err(e)) => e.to_string(),
                    _ => "timed out".to_owned(),
                };
                if self.0.throttle.allow(&format!("tls:{ip}"), crate::now_ms()).is_some() {
                    linelog::debug("tls handshake failed", json!({ "ip": ip, "error": error }));
                }
                return;
            }
        };
        if !node {
            self.serve_http(stream, ip, key, None).await;
            return;
        }
        let caller = crate::nodelink::caller(stream.get_ref().1.peer_certificates()).map(|caller| caller.node);
        match caller {
            Some(caller) => self.serve_http(stream, ip, key, Some(caller)).await,
            None => self.warn_throttled(
                &format!("nodetls:{ip}"),
                "node link refused at the handshake",
                json!({ "ip": ip, "error": "its certificate names no node" }),
            ),
        }
    }

    async fn serve_http<S>(&self, stream: S, ip: String, key: String, node: Option<NodeId>)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let phases = Phases::new(&self.0.config.timeouts);
        let conn = Conn {
            ip: ip.clone(),
            key,
            phases: phases.clone(),
            capture: self
                .0
                .capture
                .as_ref()
                .filter(|_| node.is_none())
                .map(Capture::connection),
            node,
        };
        let gateway = self.clone();
        let service = hyper::service::service_fn(move |req| {
            let gateway = gateway.clone();
            let conn = conn.clone();
            async move { Ok::<_, Infallible>(gateway.handle(req, conn).await) }
        });
        let mut builder = hyper::server::conn::http1::Builder::new();
        builder
            .max_headers(self.0.config.limits.max_headers)
            .max_buf_size(16 * 1024)
            .header_read_timeout(None);
        let connection = builder
            .serve_connection(TokioIo::new(Guard::new(stream, phases.clone())), service)
            .with_upgrades();
        tokio::pin!(connection);
        let mut closing = self.0.closing.subscribe();
        let mut abort = self.0.abort.subscribe();
        let mut graceful = false;
        let result = loop {
            let awaiting = phases.is_awaiting_head();
            // A backstop: a waiting head is the stream's to end (with its 408 and linger)
            let watchdog = phases.deadline()
                + if awaiting {
                    Duration::from_secs(3)
                } else {
                    Duration::ZERO
                };
            tokio::select! {
                result = &mut connection => break result,
                _ = closing.wait_for(|c| *c), if !graceful => {
                    graceful = true;
                    connection.as_mut().graceful_shutdown();
                }
                _ = abort.wait_for(|a| *a) => return,
                () = tokio::time::sleep_until(watchdog) => {
                    let awaiting = phases.is_awaiting_head();
                    let deadline = phases.deadline() + if awaiting { Duration::from_secs(3) } else { Duration::ZERO };
                    if Instant::now() >= deadline {
                        return;
                    }
                }
            }
        };
        let (status, reason) = if phases.sent_408() {
            (408, "ERR_HTTP_REQUEST_TIMEOUT")
        } else {
            match &result {
                Err(e) if e.is_parse_too_large() => (431, "HPE_HEADER_OVERFLOW"),
                Err(e) if e.is_parse() => (400, "client_error"),
                _ => return,
            }
        };
        if self
            .0
            .throttle
            .allow(&format!("clienterror:{ip}"), crate::now_ms())
            .is_some()
        {
            linelog::info(
                "request",
                json!({ "ip": ip, "route": "gateway", "status": status, "reason": reason }),
            );
        }
    }

    fn access(&self, conn: &Conn, req: &Request<Incoming>, started: std::time::Instant) -> Access {
        Access {
            gateway: self.clone(),
            conn: conn.clone(),
            method: req.method().as_str().to_owned(),
            target: raw_target(req.uri()),
            route: "gateway",
            metric: "refused",
            status: 0,
            bytes_in: Arc::default(),
            started,
            reason: None,
            ua: req
                .headers()
                .get(http::header::USER_AGENT)
                .map(|v| redact_user_agent(v.as_bytes())),
            upstream_bytes: false,
            declared: None,
            capture: conn.capture.as_ref().filter(|_| !is_upgrade(req)).map(|_| {
                let cap = self.0.capture.as_ref().map_or(0, Capture::max_body);
                CaptureParts {
                    req_headers: req.headers().clone(),
                    req_body: Arc::new(Mutex::new(BodyTap::new(cap))),
                    res_headers: HeaderMap::new(),
                    res_body: Arc::new(Mutex::new(BodyTap::new(cap))),
                }
            }),
        }
    }

    /// An answer of the gateway's own, with its access line.
    fn answer(&self, mut access: Access, response: Response<GwBody>, reason: Option<String>) -> Response<GwBody> {
        access.status = response.status().as_u16();
        access.reason = reason;
        let declared = response.body().size_hint().exact();
        access.declared = declared;
        let (parts, body) = response.into_parts();
        if let Some(capture) = &mut access.capture
            && capture.res_headers.is_empty()
        {
            capture.res_headers = parts.headers.clone();
        }
        let tap = access.capture.as_ref().map(|c| c.res_body.clone());
        Response::from_parts(
            parts,
            Tracked {
                inner: body,
                out: 0,
                access: Some(access),
                tap,
            }
            .boxed_unsync(),
        )
    }

    /// `{"error":"<code>"}`: closes the connection rather than read a body it refused.
    fn refusal(&self, status: u16, code: &str, close: bool, retry_after: Option<u64>) -> Response<GwBody> {
        let body = format!("{{\"error\":\"{code}\"}}");
        let mut response = Response::new(full(body.clone()));
        *response.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let h = response.headers_mut();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        h.insert(CONTENT_LENGTH, HeaderValue::from(body.len()));
        h.insert(http::header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if let Some(seconds) = retry_after {
            h.insert(http::header::RETRY_AFTER, HeaderValue::from(seconds));
        }
        if close || self.is_closing() {
            h.insert(CONNECTION, HeaderValue::from_static("close"));
        }
        response
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "each argument is part of one refusal: its status, code, log reason and connection handling"
    )]
    fn refuse(
        &self,
        access: Access,
        status: u16,
        code: &str,
        reason: &str,
        close: bool,
        retry_after: Option<u64>,
    ) -> Response<GwBody> {
        let response = self.refusal(status, code, close, retry_after);
        self.answer(access, response, Some(reason.to_owned()))
    }

    async fn handle(&self, req: Request<Incoming>, conn: Conn) -> Response<GwBody> {
        conn.phases.request_started();
        let started = std::time::Instant::now();
        if let Some(node) = conn.node.clone() {
            return self.node_request(req, conn, node, started).await;
        }
        if is_upgrade(&req) {
            return self.upgrade(req, conn, started).await;
        }
        if req.method() == Method::POST
            && raw_target(req.uri()) == "/__node/enroll"
            && let Some(handler) = self.node_links().and_then(|l| l.enroll_handler())
        {
            return self.enroll(req, conn, started, handler).await;
        }
        self.request(req, conn, started).await
    }

    fn take(&self, class: LimitClass, key: &str) -> Option<u64> {
        match self.0.limiters.class(class).take(key) {
            Take::Allowed => None,
            Take::Limited { retry_after } => Some(retry_after),
        }
    }

    /// Sections 6 and 7: one request on a public or private connection.
    async fn request(&self, req: Request<Incoming>, conn: Conn, started: std::time::Instant) -> Response<GwBody> {
        let mut access = self.access(&conn, &req, started);
        let close = has_body(req.headers());
        let config = &self.0.config;
        let routed = self.0.routes.route(
            &config.version.to_string(),
            &config.version,
            req.method().as_str(),
            &access.target,
            req.headers(),
            false,
        );
        let class = routed.decision.class();
        if let Some(retry) = self.take(class, &conn.key) {
            let reason = format!("rate_limited:{}", class.as_str());
            return self.refuse(access, 429, "rate_limited", &reason, close, Some(retry));
        }
        let (upstream, feed) = match routed.decision {
            Decision::Reject {
                status, error, reason, ..
            } => return self.refuse(access, status, error, reason, close, None),
            Decision::Network { route, .. } => return self.network(req, access, route).await,
            Decision::Proxy { upstream, feed, .. } => (upstream, feed),
        };
        let endpoint = match upstream {
            Upstream::Content => Some(config.content.clone()),
            Upstream::Ws => routed.endpoint.or_else(|| Some(config.ws.clone())),
            Upstream::Backend => routed.endpoint,
        };
        let Some(endpoint) = endpoint else {
            return self.refuse(access, 502, "bad_gateway", "no_upstream", close, None);
        };
        if declared_length(req.headers()).is_some_and(|n| n > config.limits.max_body_bytes as f64) {
            return self.refuse(access, 413, "body_too_large", "body_too_large", close, None);
        }
        let Ok(uri) = format!("http://{}{}", endpoint.authority(), access.target).parse::<http::Uri>() else {
            return self.refuse(access, 400, "bad_request", "target_not_origin_form", close, None);
        };
        let feeds = match feed {
            Some(FeedKind::Token) => true,
            Some(FeedKind::Heartbeat) => has_bearer(req.headers()),
            None => false,
        };
        let exchange = Arc::new(Exchange::default());
        access.bytes_in = exchange.bytes_in.clone();
        let req_tap = access.capture.as_ref().map(|c| c.req_body.clone());
        let headers = forward_headers(req.headers(), &conn.ip, config.tls, &config.secret, false);
        let method = req.method().clone();
        let body = UpBody {
            inner: Some(req.into_body()),
            cap: config.limits.max_body_bytes,
            sleep: Box::pin(tokio::time::sleep(config.timeouts.request)),
            exchange: exchange.clone(),
            tap: req_tap,
        };
        let mut upstream_req = Request::new(body);
        *upstream_req.method_mut() = method;
        *upstream_req.uri_mut() = uri;
        *upstream_req.headers_mut() = headers;
        let answered = tokio::time::timeout(config.timeouts.upstream_idle, self.0.client.request(upstream_req)).await;
        let response = match answered {
            Ok(Ok(response)) => response,
            failed => {
                if exchange.over_cap.load(Ordering::Relaxed) {
                    return self.refuse(access, 413, "body_too_large", "body_too_large", true, None);
                }
                if exchange.timed_out.load(Ordering::Relaxed) {
                    return self.refuse(access, 408, "request_timeout", "ERR_HTTP_REQUEST_TIMEOUT", true, None);
                }
                access.route = upstream.route();
                return match failed {
                    Err(_) => self.refuse(access, 504, "upstream_timeout", "upstream_timeout", close, None),
                    Ok(Err(e)) => {
                        let reason = format!("upstream_{}", error_code(&e));
                        self.refuse(access, 502, "bad_gateway", &reason, close, None)
                    }
                    Ok(Ok(_)) => unreachable!("handled above"),
                };
            }
        };
        access.route = upstream.route();
        access.metric = upstream.route();
        access.upstream_bytes = true;
        if feeds && response.status().is_success() {
            self.0.feed.report(&conn.ip);
        }
        let (mut parts, body) = response.into_parts();
        if let Some(capture) = &mut access.capture {
            capture.res_headers = parts.headers.clone();
        }
        parts.headers = proxy::end_to_end(&parts.headers, |n| n == "x-powered-by" || n == "x-dainty-gateway");
        if self.is_closing() {
            parts.headers.insert(CONNECTION, HeaderValue::from_static("close"));
        }
        let idle = config.timeouts.upstream_idle;
        let body = IdleBody {
            inner: body,
            idle,
            sleep: Box::pin(tokio::time::sleep(idle)),
        };
        self.answer(access, Response::from_parts(parts, body.boxed_unsync()), None)
    }

    /// Section 9.3: answered here, never proxied.
    async fn network(&self, req: Request<Incoming>, mut access: Access, route: NetworkRoute) -> Response<GwBody> {
        access.route = "network";
        access.metric = "network";
        let close = has_body(req.headers());
        let no_store = |mut r: Response<GwBody>, status: u16| {
            *r.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
            r.headers_mut()
                .insert(http::header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            if self.is_closing() {
                r.headers_mut().insert(CONNECTION, HeaderValue::from_static("close"));
            }
            r
        };
        match route {
            NetworkRoute::Nodes => {
                let body = json!({ "nodes": self.0.config.network }).to_string();
                let mut r = Response::new(full(body));
                r.headers_mut()
                    .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
                self.answer(access, no_store(r, 200), None)
            }
            NetworkRoute::Ping => self.answer(access, no_store(Response::new(full(Bytes::new())), 204), None),
            NetworkRoute::Regions => {
                let header = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
                let api_key =
                    header("x-dainty-user-api-key").filter(|k| protocol::account::is_plausible_account_key(k));
                let ticket = req
                    .headers()
                    .get("x-dainty-session-ticket")
                    .filter(|v| (1..=4096).contains(&v.len()))
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                if api_key.is_none() && ticket.is_none() {
                    let r = self.refusal(401, "unauthorized", close, None);
                    return self.answer(access, r, Some("no_account_key".into()));
                }
                let count = access.bytes_in.clone();
                let body = match read_limited(req.into_body(), 4096, self.0.config.timeouts.request, &count).await {
                    Ok(body) => body,
                    Err(Limited::TooLarge) => {
                        return self.refuse(access, 413, "body_too_large", "body_too_large", true, None);
                    }
                    Err(Limited::Failed) => return self.refuse(access, 400, "bad_request", "body_error", true, None),
                    Err(Limited::Clock) => {
                        return self.refuse(access, 408, "request_timeout", "ERR_HTTP_REQUEST_TIMEOUT", true, None);
                    }
                };
                let Some(regions) = crate::network::parse_region_report(&body) else {
                    return self.refuse(access, 400, "bad_request", "bad_region_report", false, None);
                };
                let handler = self.0.region.read().unwrap_or_else(PoisonError::into_inner).clone();
                if let Some(handler) = handler {
                    let accepted = handler(api_key, ticket, regions).await;
                    if !accepted {
                        let r = self.refusal(401, "unauthorized", false, None);
                        return self.answer(access, r, Some("invalid_credentials".into()));
                    }
                }
                self.answer(access, no_store(Response::new(full(Bytes::new())), 204), None)
            }
        }
    }

    /// Section 9.2.
    async fn enroll(
        &self,
        req: Request<Incoming>,
        conn: Conn,
        started: std::time::Instant,
        handler: crate::network::EnrollHandler,
    ) -> Response<GwBody> {
        let mut access = self.access(&conn, &req, started);
        access.route = "enroll";
        access.metric = "enroll";
        let close = has_body(req.headers());
        if let Some(retry) = self.take(LimitClass::Register, &conn.key) {
            return self.refuse(access, 429, "rate_limited", "rate_limited:register", close, Some(retry));
        }
        let count = access.bytes_in.clone();
        let body = match read_limited(req.into_body(), 64 * 1024, self.0.config.timeouts.request, &count).await {
            Ok(body) => body,
            Err(Limited::TooLarge) => return self.refuse(access, 413, "body_too_large", "body_too_large", true, None),
            Err(_) => return self.refuse(access, 400, "bad_request", "body_error", true, None),
        };
        let (status, body) = handler(body).await;
        self.json_reply(access, status, body, None, false)
    }

    /// A JSON answer: `content-type`, `content-length`, `cache-control: no-store`, `connection: close` while closing.
    fn json_reply(
        &self,
        access: Access,
        status: u16,
        body: Vec<u8>,
        time_ms: Option<i64>,
        close: bool,
    ) -> Response<GwBody> {
        let len = body.len();
        let mut r = Response::new(full(body));
        *r.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
        let h = r.headers_mut();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        h.insert(CONTENT_LENGTH, HeaderValue::from(len));
        h.insert(http::header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if let Some(ms) = time_ms {
            h.insert(HeaderName::from_static(wire::TIME_HEADER), HeaderValue::from(ms));
        }
        if close || self.is_closing() {
            h.insert(CONNECTION, HeaderValue::from_static("close"));
        }
        self.answer(access, r, None)
    }

    /// Section 9.1: a request on a node connection. No policy, no rate limit, no capture.
    async fn node_request(
        &self,
        req: Request<Incoming>,
        conn: Conn,
        from: NodeId,
        started: std::time::Instant,
    ) -> Response<GwBody> {
        let mut access = self.access(&conn, &req, started);
        access.route = "node";
        access.metric = "node";
        access.ua = None;
        let with_reason = |mut access: Access, reply: &Reply| {
            if reply.status != 200 {
                access.reason = ErrorBody::decode(&reply.body).map(|b| b.error);
            }
            access
        };
        let Some(links) = self.node_links() else {
            let body = ErrorBody::new(wire::code::NOT_FOUND).encode();
            access.reason = Some(wire::code::NOT_FOUND.to_owned());
            return self.json_reply(access, 404, body, None, false);
        };
        let endpoint = links.endpoint();
        let had_body = has_body(req.headers());
        let times: Vec<&HeaderValue> = req.headers().get_all(wire::TIME_HEADER).iter().collect();
        let time = match times.as_slice() {
            [one] => one.to_str().ok(),
            _ => None,
        };
        let content_length = req
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let target = access.target.clone();
        let head = Head {
            method: req.method().as_str(),
            target: &target,
            content_length,
            time,
        };
        let admitted = endpoint.admit(&from, &head);
        let target = match admitted {
            Ok(target) => target,
            Err(reply) => {
                let access = with_reason(access, &reply);
                return self.json_reply(access, reply.status, reply.body, Some(reply.time_ms), had_body);
            }
        };
        let count = access.bytes_in.clone();
        let body = match read_limited(req.into_body(), MAX_MESSAGE, self.0.config.timeouts.request, &count).await {
            Ok(body) => body,
            Err(failed) => {
                let (status, code) = match failed {
                    Limited::TooLarge => (413, wire::code::TOO_LARGE),
                    _ => (400, wire::code::BAD_REQUEST),
                };
                access.reason = Some(code.to_owned());
                let now = endpoint.clock().now_ms();
                return self.json_reply(access, status, ErrorBody::new(code).encode(), Some(now), true);
            }
        };
        let reply = endpoint.serve(&from, target, body).await;
        let access = with_reason(access, &reply);
        self.json_reply(access, reply.status, reply.body, Some(reply.time_ms), false)
    }

    /// Section 8: a WebSocket upgrade, joined byte for byte once both sides switched.
    async fn upgrade(&self, mut req: Request<Incoming>, conn: Conn, started: std::time::Instant) -> Response<GwBody> {
        let mut access = self.access(&conn, &req, started);
        if self.is_closing() {
            return self.refuse(access, 503, "shutting_down", "shutting_down", true, None);
        }
        let config = &self.0.config;
        let routed = self.0.routes.route(
            &config.version.to_string(),
            &config.version,
            req.method().as_str(),
            &access.target,
            req.headers(),
            true,
        );
        if let Some(retry) = self.take(LimitClass::Ws, &conn.key) {
            return self.refuse(access, 429, "rate_limited", "rate_limited:ws", true, Some(retry));
        }
        match routed.decision {
            Decision::Reject {
                status, error, reason, ..
            } => return self.refuse(access, status, error, reason, true, None),
            Decision::Network { .. } => {
                return self.refuse(access, 400, "bad_request", "unsupported_upgrade", true, None);
            }
            Decision::Proxy { .. } => {}
        }
        let endpoint: Endpoint = routed.endpoint.unwrap_or_else(|| config.ws.clone());
        access.route = "ws";
        access.metric = "ws";
        let headers = forward_headers(req.headers(), &conn.ip, config.tls, &config.secret, true);
        let mut upstream_req = Request::new(Empty::<Bytes>::new());
        *upstream_req.method_mut() = req.method().clone();
        let Ok(uri) = access.target.parse::<http::Uri>() else {
            return self.refuse(access, 400, "bad_request", "target_not_origin_form", true, None);
        };
        *upstream_req.uri_mut() = uri;
        *upstream_req.headers_mut() = headers;
        let authority = endpoint.authority();
        let exchange = async {
            let stream = TcpStream::connect(&authority).await?;
            _ = stream.set_nodelay(true);
            let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
                .await
                .map_err(io::Error::other)?;
            tokio::spawn(connection.with_upgrades());
            sender.send_request(upstream_req).await.map_err(io::Error::other)
        };
        let mut response = match tokio::time::timeout(config.timeouts.headers, exchange).await {
            Err(_) => return self.refuse(access, 504, "upstream_timeout", "upstream_timeout", true, None),
            Ok(Err(e)) => {
                let reason = format!("upstream_{}", error_code(&e));
                return self.refuse(access, 502, "bad_gateway", &reason, true, None);
            }
            Ok(Ok(response)) => response,
        };
        let status = response.status();
        if status != StatusCode::SWITCHING_PROTOCOLS {
            let mut headers = proxy::end_to_end(response.headers(), |n| {
                n == "x-powered-by" || n == "x-dainty-gateway" || n == "content-length"
            });
            headers.insert(CONNECTION, HeaderValue::from_static("close"));
            let mut kept = Vec::new();
            let mut body = response.into_body();
            while let Some(Ok(frame)) = body.frame().await {
                if let Some(data) = frame.data_ref() {
                    let room = (64 * 1024usize).saturating_sub(kept.len()).min(data.len());
                    kept.extend_from_slice(&data[..room]);
                }
            }
            headers.insert(CONTENT_LENGTH, HeaderValue::from(kept.len()));
            access.upstream_bytes = true;
            let mut r = Response::new(full(kept));
            *r.status_mut() = status;
            *r.headers_mut() = headers;
            return self.answer(access, r, None);
        }
        let mut headers = response.headers().clone();
        headers.remove("x-powered-by");
        headers.remove("x-dainty-gateway");
        let upstream_upgrade = hyper::upgrade::on(&mut response);
        let client_upgrade = hyper::upgrade::on(&mut req);
        access.status = 101;
        let target = access.target.clone();
        access.write(0, true);
        conn.phases.request_started();
        let gateway = self.clone();
        let phases = conn.phases.clone();
        let ip = conn.ip.clone();
        let chat = conn.capture.clone();
        tokio::spawn(async move {
            let (Ok(client), Ok(upstream)) = (client_upgrade.await, upstream_upgrade.await) else {
                return;
            };
            phases.upgraded();
            let stats = &gateway.0.stats;
            stats.websockets.fetch_add(1, Ordering::Relaxed);
            let begun = std::time::Instant::now();
            let idle = gateway.0.config.timeouts.ws_idle;
            let mut closing = gateway.0.closing.subscribe();
            let client = TokioIo::new(client);
            let upstream = TokioIo::new(upstream);
            let joined = async {
                match &chat {
                    Some(chat) => {
                        chat.chat_open(&ip, &target);
                        let counts = proxy::tunnel(ChatTap::new(client, chat.clone()), upstream, idle).await;
                        chat.chat_close();
                        counts
                    }
                    None => proxy::tunnel(client, upstream, idle).await,
                }
            };
            let (bytes_in, bytes_out) = tokio::select! {
                counts = joined => counts,
                _ = closing.wait_for(|c| *c) => (0, 0),
            };
            stats.websockets.fetch_sub(1, Ordering::Relaxed);
            linelog::info(
                "websocket closed",
                json!({ "ip": ip, "target": redact_target(&target), "bytesIn": bytes_in, "bytesOut": bytes_out,
                    "ms": begun.elapsed().as_millis() as u64 }),
            );
        });
        let mut r = Response::new(full(Bytes::new()));
        *r.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
        *r.headers_mut() = headers;
        r
    }
}

/// The request target as sent: hyper's path drops an absolute form's scheme and host, which policy must refuse
/// (section 4.3 step 1) and the access log must show.
fn raw_target(uri: &http::Uri) -> String {
    match uri.path_and_query() {
        Some(p) if uri.scheme().is_none() && uri.authority().is_none() => p.as_str().to_owned(),
        _ => uri.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_judged_as_sent() {
        for (sent, logged) in [
            ("/dainty/api/Register?x=1", "/dainty/api/Register?x=1"),
            ("//", "//"),
            ("*", "*"),
            (
                "http://evil.example/dainty/api/CreateInvite",
                "http://evil.example/dainty/api/CreateInvite",
            ),
            (
                "https://evil.example:443/__node/enroll",
                "https://evil.example:443/__node/enroll",
            ),
            ("evil.example:443", "evil.example:443"),
        ] {
            let uri: http::Uri = sent.parse().unwrap();
            let target = raw_target(&uri);
            assert_eq!(target, logged);
            let refused = !sent.starts_with('/');
            let decision = crate::policy::classify("GET", &target, &HeaderMap::new(), false);
            assert_eq!(
                matches!(
                    decision,
                    Decision::Reject {
                        reason: "target_not_origin_form",
                        ..
                    }
                ),
                refused,
                "{sent}"
            );
        }
    }

    #[test]
    fn forwarded_headers_drop_claims_and_set_ours() {
        let mut h = HeaderMap::new();
        for (name, value) in [
            ("x-forwarded-for", "1.1.1.1"),
            ("forwarded", "for=x"),
            ("x-real-ip", "2.2.2.2"),
            ("x-dainty-gateway", "fake"),
            ("expect", "100-continue"),
            ("connection", "keep-alive, x-hidden"),
            ("x-hidden", "1"),
            ("upgrade", "websocket"),
            ("authorization", "Bearer t"),
            ("host", "play.example.org"),
        ] {
            h.append(name, HeaderValue::from_static(value));
        }
        let out = forward_headers(&h, "203.0.113.7", true, "the-secret", false);
        let names: Vec<&str> = out.keys().map(HeaderName::as_str).collect();
        assert_eq!(
            names,
            [
                "authorization",
                "host",
                "x-forwarded-for",
                "x-forwarded-proto",
                "x-dainty-gateway"
            ]
        );
        assert_eq!(out["x-forwarded-for"], "203.0.113.7");
        assert_eq!(out["x-forwarded-proto"], "https");
        assert_eq!(out["x-dainty-gateway"], "the-secret");
        let up = forward_headers(&h, "::1", false, "s", true);
        assert_eq!(up["x-forwarded-proto"], "http");
        assert_eq!(
            (up["connection"].to_str().unwrap(), up["upgrade"].to_str().unwrap()),
            ("Upgrade", "websocket")
        );
    }

    #[test]
    fn error_codes_take_the_first_io_error() {
        let refused = io::Error::new(io::ErrorKind::ConnectionRefused, "no");
        assert_eq!(error_code(&refused), "ECONNREFUSED");
        let wrapped = io::Error::other(io::Error::new(io::ErrorKind::ConnectionRefused, "no"));
        assert_eq!(error_code(&wrapped), "error");
        assert_eq!(error_code(&io::Error::from(io::ErrorKind::UnexpectedEof)), "ECONNRESET");
        assert_eq!(error_code(&io::Error::from(io::ErrorKind::TimedOut)), "ETIMEDOUT");
        assert_eq!(
            error_code(&io::Error::from(io::ErrorKind::AddrNotAvailable)),
            "EADDRNOTAVAIL"
        );
        #[derive(Debug)]
        struct Plain;
        impl std::fmt::Display for Plain {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("plain")
            }
        }
        impl std::error::Error for Plain {}
        assert_eq!(error_code(&Plain), "error");
    }

    #[test]
    fn bodies_are_declared_by_either_header() {
        let mut h = HeaderMap::new();
        assert!(!has_body(&h));
        h.insert(CONTENT_LENGTH, HeaderValue::from_static("0"));
        assert!(!has_body(&h));
        h.insert(CONTENT_LENGTH, HeaderValue::from_static("12"));
        assert!(has_body(&h));
        let mut t = HeaderMap::new();
        t.insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
        assert!(has_body(&t));
    }
}
