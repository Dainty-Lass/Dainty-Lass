// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The local relay (spec §2.6.4): while the game runs it listens on loopback and carries the game's HTTP, WebSocket
//! upgrades and their tunnels to the server, over TLS pinned to the invite's certificate in public mode or plain
//! HTTP to a private-mode host. The account key passes through it, so it serves only the game (`local`, and the
//! owner check of [`crate::net::relay_owner`]) and sends the key only to its own server.

pub mod local;

#[cfg(test)]
mod tests;

use std::convert::Infallible;
use std::fmt;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::header::{
    CACHE_CONTROL, CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, HOST, HeaderName, HeaderValue,
    TRANSFER_ENCODING, UPGRADE,
};
use hyper::service::service_fn;
use hyper::{HeaderMap, Method, Request, Response, StatusCode, Uri, Version};
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

use crate::net::connector::{Connector, pin_mismatch};
use crate::net::http::{Endpoint, HttpError, check_endpoint, host_header};
use crate::net::relay_owner::OwnerCheck;
use crate::platform::log::{Level, describe_error};

/// The port the game is pointed at unless [`PORT_VAR`] overrides it.
pub const DEFAULT_PORT: u16 = 61100;
/// Where the game's port goes when Windows has lent [`DEFAULT_PORT`] (or the override) out to Hyper-V, WSL or Docker:
/// below the dynamic range Windows lends from.
pub const ALTERNATE_PORTS: std::ops::RangeInclusive<u16> = 41100..=41109;
/// The game's QoS probe port.
pub const DEFAULT_QOS_PORT: u16 = 80;
/// The server's QoS target names the relay at its default port; a relay elsewhere rewrites it on the way back.
const REGIONS_PATH: &str = "/candidate/regions";
const MAX_REGIONS_BYTES: usize = 64 * 1024;
/// Overrides the relay port, when [`port_override`] accepts it.
pub const PORT_VAR: &str = "DAINTY_RELAY_PORT";
/// The game's account key header, sent only to the home server.
pub const API_KEY_HEADER: &str = "x-dainty-user-api-key";
/// The session ticket that stands in for the key on another node (DN5).
pub const SESSION_TICKET_HEADER: &str = "x-dainty-session-ticket";
/// A response re-pointing the relay names the node's host here...
pub const REDIRECT_HOST_HEADER: &str = "x-dainty-redirect-host";
/// ...its port here...
pub const REDIRECT_PORT_HEADER: &str = "x-dainty-redirect-port";
/// ...and, for a public node, its certificate's SHA-256 here.
pub const REDIRECT_FINGERPRINT_HEADER: &str = "x-dainty-redirect-fingerprint";
/// The QoS page's body.
pub const QOS_BODY: &str = "<!DOCTYPE html><html><body>pong</body></html>";

const BUFFER_BYTES: usize = 64 * 1024;
const POOL_IDLE: Duration = Duration::from_secs(60);
const POOL_MAX_IDLE: usize = 8;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TUNNEL_IDLE: Duration = Duration::from_secs(24 * 60 * 60);
const REFUSED_UPGRADE_BODY_BYTES: usize = 64 * 1024;
const REFUSED_UPGRADE_BODY_WAIT: Duration = Duration::from_secs(10);
const MAX_TICKET_BYTES: usize = 8 * 1024;
// Lets the pooled client keep its absolute-URI pool key; the connector ignores it and the path is sent as is
const UPSTREAM_ORIGIN: &str = "http://relay.upstream";

/// A port override: trimmed, 4 or 5 digits without a leading zero, 1024 to 65535. Anything else is ignored.
#[must_use]
pub fn port_override(raw: &str) -> Option<u16> {
    let raw = raw.trim();
    if !(4..=5).contains(&raw.len()) || raw.starts_with('0') || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse::<u16>().ok().filter(|port| *port >= 1024)
}

/// The relay port from the override variable's value, else [`DEFAULT_PORT`].
#[must_use]
pub fn configured_port(var: Option<&str>) -> u16 {
    var.and_then(port_override).unwrap_or(DEFAULT_PORT)
}

/// Public (pinned TLS) or private (plain HTTP to a private-mode host).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Pinned TLS to the gateway.
    Public,
    /// Plain HTTP over Tailscale or loopback.
    Private,
}

impl Mode {
    /// The word logged for it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

/// Where relay lines go; the caller prefixes and writes them. No line carries a header value.
pub type LogSink = Arc<dyn Fn(Level, &str) + Send + Sync>;
/// Hears the fingerprint a server presented when it did not match the pin.
pub type PinSink = Arc<dyn Fn(&str) + Send + Sync>;

/// What [`Relay::start`] needs.
pub struct RelayConfig {
    /// The game's port on 127.0.0.1.
    pub port: u16,
    /// The QoS-only listener's port, when one is wanted.
    pub qos_port: Option<u16>,
    /// The home server: its pin, when present, makes the relay public.
    pub target: Endpoint,
    /// Decides per connection on the game's port; `None` lets any loopback client through the local rules.
    pub owner: Option<Arc<dyn OwnerCheck>>,
    /// Relay lines.
    pub log: LogSink,
    /// A pin mismatch upstream.
    pub on_pin_mismatch: PinSink,
}

/// Why the relay did not start.
#[derive(Debug)]
pub enum RelayError {
    /// The port is in use or not ours to bind.
    PortBusy(u16),
    /// Another bind failure.
    Listen {
        /// The port.
        port: u16,
        /// What failed.
        error: io::Error,
    },
    /// The target is refused as the endpoint rules refuse it (a public host without a pin, among others).
    Target(HttpError),
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PortBusy(port) => write!(f, "port {port} is busy"),
            Self::Listen { port, error } => write!(f, "could not listen on port {port}: {error}"),
            Self::Target(error) => write!(f, "refused target: {error}"),
        }
    }
}

impl std::error::Error for RelayError {}

/// Why [`Relay::repoint`] refused.
#[derive(Debug)]
pub enum RepointError {
    /// The node is refused as the endpoint rules refuse it, or is plain HTTP while home is pinned.
    Endpoint(HttpError),
    /// Only home may be reached without a session ticket.
    TicketRequired,
    /// The ticket is not a header value.
    BadTicket,
}

impl fmt::Display for RepointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Endpoint(error) => write!(f, "refused node: {error}"),
            Self::TicketRequired => f.write_str("another node needs a session ticket"),
            Self::BadTicket => f.write_str("the session ticket is not a header value"),
        }
    }
}

impl std::error::Error for RepointError {}

/// The relay's counts so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelayCounters {
    /// Plain requests forwarded.
    pub requests: u64,
    /// Upgrades forwarded.
    pub upgrades: u64,
    /// Requests and upgrades that failed upstream.
    pub failures: u64,
    /// QoS pages answered, on either listener.
    pub qos: u64,
    /// Connections opened to the server.
    pub upstream_connections: u64,
}

type ResponseBody = BoxBody<Bytes, hyper::Error>;

/// One upstream: home or another node, with its own pool.
struct Route {
    endpoint: Endpoint,
    ticket: Option<HeaderValue>,
    home: bool,
    host: HeaderValue,
    connector: Connector,
    client: Client<Connector, Incoming>,
}

#[derive(Default)]
struct Counts {
    requests: AtomicU64,
    upgrades: AtomicU64,
    failures: AtomicU64,
    qos: AtomicU64,
}

struct Shared {
    home: Endpoint,
    route: RwLock<Arc<Route>>,
    connectors: Mutex<Vec<Connector>>,
    counts: Counts,
    owner: Option<Arc<dyn OwnerCheck>>,
    log: LogSink,
    on_pin_mismatch: PinSink,
    cancel: CancellationToken,
    /// The address the server's QoS target names and the one this relay holds, when they differ
    qos_moved: Option<(String, String)>,
}

/// A running relay. Dropping it stops it too, without waiting.
pub struct Relay {
    shared: Arc<Shared>,
    port: u16,
    qos_port: Option<u16>,
    tasks: Vec<JoinHandle<()>>,
}

impl Relay {
    /// Binds 127.0.0.1:`port` (and the QoS port, best effort) and starts serving on the current tokio runtime.
    pub async fn start(config: RelayConfig) -> Result<Self, RelayError> {
        let pin = check_endpoint(&config.target).map_err(RelayError::Target)?;
        let home = Endpoint { pin, ..config.target };
        let route = Route::new(home.clone(), None, true).map_err(RelayError::Target)?;
        let listener = bind_game_port(config.port).await?;
        let port = listener.local_addr().map_or(config.port, |addr| addr.port());
        let qos_moved =
            (port != DEFAULT_PORT).then(|| (format!("127.0.0.1:{DEFAULT_PORT}/"), format!("127.0.0.1:{port}/")));

        let mode = if home.pin.is_some() {
            Mode::Public
        } else {
            Mode::Private
        };
        let shared = Arc::new(Shared {
            connectors: Mutex::new(vec![route.connector.clone()]),
            route: RwLock::new(Arc::new(route)),
            home,
            counts: Counts::default(),
            owner: config.owner,
            log: config.log,
            on_pin_mismatch: config.on_pin_mismatch,
            cancel: CancellationToken::new(),
            qos_moved,
        });
        shared.log(
            Level::Info,
            &format!(
                "relay listening on 127.0.0.1:{port} for {} ({})",
                shared.home.host,
                mode.as_str()
            ),
        );

        let mut tasks = vec![tokio::spawn(accept_loop(listener, Arc::clone(&shared), false))];
        let mut qos_port = None;
        if let Some(wanted) = config.qos_port {
            match bind(wanted).await {
                Ok(qos) => {
                    qos_port = Some(qos.local_addr().map_or(wanted, |addr| addr.port()));
                    tasks.push(tokio::spawn(accept_loop(qos, Arc::clone(&shared), true)));
                }
                Err(error) => shared.log(
                    Level::Info,
                    &format!("relay QoS listener on 127.0.0.1:{wanted} not started: {error}"),
                ),
            }
        }
        Ok(Self {
            shared,
            port,
            qos_port,
            tasks,
        })
    }

    /// The game's port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The QoS listener's port, when it started.
    #[must_use]
    pub fn qos_port(&self) -> Option<u16> {
        self.qos_port
    }

    /// Public or private, from home's pin.
    #[must_use]
    pub fn mode(&self) -> Mode {
        if self.shared.home.pin.is_some() {
            Mode::Public
        } else {
            Mode::Private
        }
    }

    /// The node requests go to now.
    #[must_use]
    pub fn target(&self) -> Endpoint {
        self.shared.current().endpoint.clone()
    }

    /// Whether requests go home, with the account key.
    #[must_use]
    pub fn is_home(&self) -> bool {
        let route = self.shared.current();
        route.home && route.ticket.is_none()
    }

    /// The counts so far.
    #[must_use]
    pub fn counters(&self) -> RelayCounters {
        self.shared.counters()
    }

    /// Sends later requests to `node` with `ticket` in place of the account key. Home may go without one.
    pub fn repoint(&self, node: Endpoint, ticket: Option<String>) -> Result<(), RepointError> {
        self.shared.repoint(node, ticket)
    }

    /// Sends later requests home, with the account key again.
    pub fn go_home(&self) {
        if let Ok(route) = Route::new(self.shared.home.clone(), None, true) {
            self.shared.install(route);
        }
    }

    /// Stops every listener, connection and tunnel; both ports are free when this returns.
    pub async fn stop(mut self) -> RelayCounters {
        self.shared.cancel.cancel();
        for task in self.tasks.drain(..) {
            let _ = task.await;
        }
        self.shared.counters()
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.shared.cancel.cancel();
    }
}

impl Route {
    fn new(endpoint: Endpoint, ticket: Option<HeaderValue>, home: bool) -> Result<Self, HttpError> {
        let connector = Connector::new(&endpoint, CONNECT_TIMEOUT)?;
        let host = HeaderValue::from_str(&host_header(&endpoint)).map_err(|e| HttpError::Protocol(e.to_string()))?;
        let client = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(POOL_IDLE)
            .pool_max_idle_per_host(POOL_MAX_IDLE)
            .pool_timer(TokioTimer::new())
            .set_host(false)
            .build(connector.clone());
        Ok(Self {
            endpoint,
            ticket,
            home,
            host,
            connector,
            client,
        })
    }

    /// The game's headers made fit for this route: hop-by-hop gone, our `host`, and away from home (or with a
    /// ticket) the ticket instead of the key.
    fn headers(&self, incoming: &HeaderMap) -> HeaderMap {
        let ticket = HeaderName::from_static(SESSION_TICKET_HEADER);
        let mut headers = dainty_proxy::end_to_end(incoming, |name| {
            name == "host" || name == "http2-settings" || name == SESSION_TICKET_HEADER
        });
        if !self.home || self.ticket.is_some() {
            headers.remove(API_KEY_HEADER);
            if let Some(value) = &self.ticket {
                headers.insert(ticket, value.clone());
            }
        }
        headers.insert(HOST, self.host.clone());
        headers
    }
}

impl Shared {
    fn log(&self, level: Level, line: &str) {
        (self.log)(level, line);
    }

    fn current(&self) -> Arc<Route> {
        Arc::clone(&self.route.read().unwrap_or_else(PoisonError::into_inner))
    }

    fn install(&self, route: Route) {
        self.connectors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(route.connector.clone());
        *self.route.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(route);
    }

    fn counters(&self) -> RelayCounters {
        let opened = self
            .connectors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|c| c.connections_opened() as u64)
            .sum();
        RelayCounters {
            requests: self.counts.requests.load(Ordering::Relaxed),
            upgrades: self.counts.upgrades.load(Ordering::Relaxed),
            failures: self.counts.failures.load(Ordering::Relaxed),
            qos: self.counts.qos.load(Ordering::Relaxed),
            upstream_connections: opened,
        }
    }

    fn is_home(&self, node: &Endpoint) -> bool {
        node.host.eq_ignore_ascii_case(&self.home.host)
            && node.port == self.home.port
            && node.pin.as_ref().is_none_or(|pin| Some(pin) == self.home.pin.as_ref())
    }

    fn repoint(&self, node: Endpoint, ticket: Option<String>) -> Result<(), RepointError> {
        let ticket = match ticket.filter(|t| !t.trim().is_empty()) {
            Some(t) if t.len() > MAX_TICKET_BYTES => return Err(RepointError::BadTicket),
            Some(t) => Some(HeaderValue::from_str(&t).map_err(|_| RepointError::BadTicket)?),
            None => None,
        };
        let home = self.is_home(&node);
        if !home && ticket.is_none() {
            return Err(RepointError::TicketRequired);
        }
        let node = if home { self.home.clone() } else { node };
        // A pinned relay never falls back to plain HTTP: the ticket would travel unpinned
        if self.home.pin.is_some() && node.pin.is_none() {
            return Err(RepointError::Endpoint(HttpError::Protocol(
                "node has no pin".to_owned(),
            )));
        }
        let pin = check_endpoint(&node).map_err(RepointError::Endpoint)?;
        let route = Route::new(Endpoint { pin, ..node }, ticket, home).map_err(RepointError::Endpoint)?;
        self.install(route);
        Ok(())
    }

    /// Re-points on a response naming another node (DN6); bad headers are logged and ignored.
    fn follow_redirect(&self, headers: &HeaderMap) {
        let host = headers.get(REDIRECT_HOST_HEADER);
        let port = headers.get(REDIRECT_PORT_HEADER);
        if host.is_none() && port.is_none() {
            return;
        }
        fn text(value: Option<&HeaderValue>) -> Option<&str> {
            value.and_then(|v| v.to_str().ok()).map(str::trim)
        }
        let port = text(port)
            .filter(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|p| p.parse::<u16>().ok())
            .filter(|p| *p != 0);
        let (Some(host), Some(port)) = (text(host).filter(|h| !h.is_empty()), port) else {
            self.log(Level::Warn, "relay ignored a redirect: bad host or port");
            return;
        };
        let fingerprint = match headers.get(REDIRECT_FINGERPRINT_HEADER) {
            None => None,
            Some(value) => match value.to_str() {
                Ok(fp) => Some(fp.trim().to_owned()),
                Err(_) => {
                    self.log(Level::Warn, "relay ignored a redirect: bad fingerprint");
                    return;
                }
            },
        };
        let ticket = match headers.get(SESSION_TICKET_HEADER).map(HeaderValue::to_str) {
            None => None,
            Some(Ok(t)) => Some(t.to_owned()),
            Some(Err(_)) => {
                self.log(Level::Warn, "relay ignored a redirect: bad session ticket");
                return;
            }
        };
        let node = Endpoint {
            host: host.to_owned(),
            port,
            pin: fingerprint,
        };
        let described = format!("{}:{}", node.host, node.port);
        match self.repoint(node, ticket) {
            Ok(()) => self.log(Level::Info, &format!("relay re-pointed to {described}")),
            Err(error) => self.log(
                Level::Warn,
                &format!("relay ignored a redirect to {described}: {error}"),
            ),
        }
    }

    fn upstream_failed(&self, error: &(dyn std::error::Error + 'static), what: &str) -> bool {
        self.counts.failures.fetch_add(1, Ordering::Relaxed);
        if let Some(actual) = pin_mismatch(error) {
            (self.on_pin_mismatch)(&actual);
            self.log(
                Level::Error,
                &format!("relay {what}: certificate mismatch, the server presented {actual}"),
            );
            true
        } else {
            self.log(
                Level::Warn,
                &format!("relay {what}: upstream unreachable: {}", describe_error(error)),
            );
            false
        }
    }
}

/// The game's port: `port`, or, when Windows has lent it out (bind refused with os error 10013), the first of
/// [`ALTERNATE_PORTS`] that binds.
async fn bind_game_port(port: u16) -> Result<TcpListener, RelayError> {
    match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
        Ok(listener) => Ok(listener),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            for alternate in ALTERNATE_PORTS {
                if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, alternate)).await {
                    return Ok(listener);
                }
            }
            Err(RelayError::PortBusy(port))
        }
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => Err(RelayError::PortBusy(port)),
        Err(error) => Err(RelayError::Listen { port, error }),
    }
}

async fn bind(port: u16) -> Result<TcpListener, RelayError> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|error| match error.kind() {
            io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied => RelayError::PortBusy(port),
            _ => RelayError::Listen { port, error },
        })
}

async fn accept_loop(listener: TcpListener, shared: Arc<Shared>, qos_only: bool) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            () = shared.cancel.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((tcp, peer)) => {
                    connections.spawn(serve(tcp, peer, Arc::clone(&shared), qos_only));
                }
                // Out of handles, most likely: let some close before trying again
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
}

async fn serve(tcp: TcpStream, peer: SocketAddr, shared: Arc<Shared>, qos_only: bool) {
    let _ = tcp.set_nodelay(true);
    let allowed = match (&shared.owner, qos_only, tcp.local_addr()) {
        (None, _, _) | (Some(_), true, _) => true,
        (Some(owner), false, Ok(local)) => owner.allows(peer, local).await,
        (Some(_), false, Err(_)) => false,
    };
    if !allowed {
        shared.log(
            Level::Warn,
            &format!("relay refused a connection from {peer}: not the game's"),
        );
    }
    let service_shared = Arc::clone(&shared);
    let service = service_fn(move |request| {
        let shared = Arc::clone(&service_shared);
        async move { Ok::<_, Infallible>(handle(&shared, request, peer, allowed, qos_only).await) }
    });
    let connection = hyper::server::conn::http1::Builder::new()
        .keep_alive(true)
        .max_buf_size(BUFFER_BYTES)
        .serve_connection(TokioIo::new(tcp), service)
        .with_upgrades();
    tokio::select! {
        _ = connection => {}
        () = shared.cancel.cancelled() => {}
    }
}

fn is_upgrade(headers: &HeaderMap) -> bool {
    headers.contains_key(UPGRADE) && dainty_proxy::connection_tokens(headers).contains("upgrade")
}

async fn handle(
    shared: &Arc<Shared>,
    request: Request<Incoming>,
    peer: SocketAddr,
    allowed: bool,
    qos_only: bool,
) -> Response<ResponseBody> {
    let upgrade = is_upgrade(request.headers());
    if !allowed {
        if upgrade {
            return bare(StatusCode::FORBIDDEN);
        }
        let mut response = json_error(StatusCode::FORBIDDEN, "local_process_denied");
        response
            .headers_mut()
            .insert(CONNECTION, HeaderValue::from_static("close"));
        return response;
    }
    if let Err(refusal) = local::check(peer.ip(), request.headers(), upgrade) {
        shared.log(
            Level::Warn,
            &format!("relay refused a request from {peer}: {}", refusal.as_str()),
        );
        // The QoS listener answers nothing but its page, so a refusal there looks like any unknown path
        return if qos_only {
            json_error(StatusCode::NOT_FOUND, "not_found")
        } else if upgrade {
            bare(StatusCode::FORBIDDEN)
        } else {
            json_error(StatusCode::FORBIDDEN, "forbidden")
        };
    }
    let target = request.uri().path_and_query().map_or("/", |pq| pq.as_str()).to_owned();
    let probe = matches!(*request.method(), Method::GET | Method::HEAD);
    if qos_only {
        return if probe && (target == "/" || local::is_qos_target(&target)) {
            qos(shared)
        } else {
            json_error(StatusCode::NOT_FOUND, "not_found")
        };
    }
    if probe && local::is_qos_target(&target) {
        return qos(shared);
    }
    // An authority-form target has no path to forward, and a tunnel is only ever an upgrade here
    if request.method() == Method::CONNECT {
        return json_error(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed");
    }
    if upgrade {
        forward_upgrade(shared, request, &target).await
    } else {
        forward(shared, request, &target).await
    }
}

async fn forward(shared: &Arc<Shared>, request: Request<Incoming>, target: &str) -> Response<ResponseBody> {
    let mut encodings = request.headers().get_all(TRANSFER_ENCODING).iter().peekable();
    let chunked = encodings.peek().is_some();
    if chunked && !encodings.all(|v| v.to_str().is_ok_and(|v| v.trim().eq_ignore_ascii_case("chunked"))) {
        return json_error(StatusCode::NOT_IMPLEMENTED, "unsupported_transfer_encoding");
    }
    shared.counts.requests.fetch_add(1, Ordering::Relaxed);
    let route = shared.current();
    let (parts, body) = request.into_parts();
    let mut headers = route.headers(&parts.headers);
    if chunked {
        // Never both framings upstream: that is how requests get smuggled
        headers.remove(CONTENT_LENGTH);
        headers.insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
    }
    let Ok(uri) = format!("{UPSTREAM_ORIGIN}{target}").parse::<Uri>() else {
        return json_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    let mut upstream = Request::new(body);
    *upstream.method_mut() = parts.method;
    *upstream.uri_mut() = uri;
    *upstream.version_mut() = Version::HTTP_11;
    *upstream.headers_mut() = headers;

    match route.client.request(upstream).await {
        Ok(answer) => {
            shared.follow_redirect(answer.headers());
            let (parts, body) = answer.into_parts();
            let mut headers = answer_headers(&parts.headers);
            let body = match &shared.qos_moved {
                Some(moved) if is_regions(target) && !headers.contains_key(CONTENT_ENCODING) => {
                    let text = qos_moved(body, moved).await;
                    headers.remove(TRANSFER_ENCODING);
                    headers.insert(CONTENT_LENGTH, HeaderValue::from(text.len()));
                    full(text)
                }
                _ => body.boxed(),
            };
            let mut response = Response::new(body);
            *response.status_mut() = parts.status;
            *response.headers_mut() = headers;
            copy_reason(&parts.extensions, response.extensions_mut());
            response
        }
        Err(error) => {
            if shared.upstream_failed(&error, "request") {
                json_error(StatusCode::BAD_GATEWAY, "certificate_mismatch")
            } else {
                json_error(StatusCode::BAD_GATEWAY, "upstream_unreachable")
            }
        }
    }
}

async fn forward_upgrade(shared: &Arc<Shared>, mut request: Request<Incoming>, target: &str) -> Response<ResponseBody> {
    shared.counts.upgrades.fetch_add(1, Ordering::Relaxed);
    let route = shared.current();
    let downstream = hyper::upgrade::on(&mut request);
    let (parts, body) = request.into_parts();
    let mut headers = route.headers(&parts.headers);
    for value in parts.headers.get_all(UPGRADE) {
        headers.append(UPGRADE, value.clone());
    }
    headers.insert(CONNECTION, HeaderValue::from_static("Upgrade"));
    let Ok(uri) = target.parse::<Uri>() else {
        return bare(StatusCode::BAD_REQUEST);
    };

    let mut connector = route.connector.clone();
    let stream = match tower::Service::call(&mut connector, Uri::from_static(UPSTREAM_ORIGIN)).await {
        Ok(stream) => stream,
        Err(error) => {
            shared.upstream_failed(&error, "upgrade");
            return bare(StatusCode::BAD_GATEWAY);
        }
    };
    let (mut sender, connection) = match hyper::client::conn::http1::Builder::new()
        .handshake::<_, Incoming>(stream)
        .await
    {
        Ok(pair) => pair,
        Err(error) => {
            shared.upstream_failed(&error, "upgrade");
            return bare(StatusCode::BAD_GATEWAY);
        }
    };
    let cancel = shared.cancel.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = connection.with_upgrades() => {}
            () = cancel.cancelled() => {}
        }
    });

    let mut upstream = Request::new(body);
    *upstream.method_mut() = parts.method;
    *upstream.uri_mut() = uri;
    *upstream.version_mut() = Version::HTTP_11;
    *upstream.headers_mut() = headers;
    let mut answer = match sender.send_request(upstream).await {
        Ok(answer) => answer,
        Err(error) => {
            shared.upstream_failed(&error, "upgrade");
            return bare(StatusCode::BAD_GATEWAY);
        }
    };
    shared.follow_redirect(answer.headers());

    if answer.status() == StatusCode::SWITCHING_PROTOCOLS {
        let mut response = bare(StatusCode::SWITCHING_PROTOCOLS);
        let headers = response.headers_mut();
        *headers = answer_headers(answer.headers());
        for name in [CONNECTION, UPGRADE] {
            for value in answer.headers().get_all(&name) {
                headers.append(name.clone(), value.clone());
            }
        }
        let upstream = hyper::upgrade::on(&mut answer);
        let cancel = shared.cancel.clone();
        tokio::spawn(async move {
            let (Ok(up), Ok(down)) = tokio::join!(upstream, downstream) else {
                return;
            };
            tokio::select! {
                _ = dainty_proxy::tunnel(TokioIo::new(down), TokioIo::new(up), TUNNEL_IDLE) => {}
                () = cancel.cancelled() => {}
            }
        });
        return response;
    }

    let (parts, body) = answer.into_parts();
    let body = read_prefix(body, REFUSED_UPGRADE_BODY_BYTES).await;
    let mut response = Response::new(full(body));
    *response.status_mut() = parts.status;
    let headers = response.headers_mut();
    *headers = answer_headers(&parts.headers);
    headers.remove(CONTENT_LENGTH);
    headers.insert(CONNECTION, HeaderValue::from_static("close"));
    copy_reason(&parts.extensions, response.extensions_mut());
    response
}

fn is_regions(target: &str) -> bool {
    target
        .split('?')
        .next()
        .is_some_and(|path| path.ends_with(REGIONS_PATH))
}

/// The regions answer with its QoS target moved to this relay's port (`moved` is from, to).
async fn qos_moved(body: Incoming, moved: &(String, String)) -> Bytes {
    let read = http_body_util::Limited::new(body, MAX_REGIONS_BYTES).collect().await;
    let Ok(collected) = read else {
        return Bytes::new();
    };
    let bytes = collected.to_bytes();
    match std::str::from_utf8(&bytes) {
        Ok(text) => Bytes::from(text.replace(&moved.0, &moved.1)),
        Err(_) => bytes,
    }
}

/// Up to `limit` bytes of `body`, whatever arrives within the wait.
async fn read_prefix(mut body: Incoming, limit: usize) -> Bytes {
    let mut out = Vec::new();
    let read = async {
        while out.len() < limit {
            let Some(Ok(frame)) = body.frame().await else { break };
            if let Ok(data) = frame.into_data() {
                out.extend_from_slice(&data[..data.len().min(limit - out.len())]);
            }
        }
    };
    let _ = tokio::time::timeout(REFUSED_UPGRADE_BODY_WAIT, read).await;
    Bytes::from(out)
}

/// A server's headers as passed back: end-to-end only, and never the re-pointing ones, which carry a ticket meant
/// for the relay.
fn answer_headers(headers: &HeaderMap) -> HeaderMap {
    dainty_proxy::end_to_end(headers, |name| {
        matches!(
            name,
            REDIRECT_HOST_HEADER | REDIRECT_PORT_HEADER | REDIRECT_FINGERPRINT_HEADER | SESSION_TICKET_HEADER
        )
    })
}

fn copy_reason(from: &hyper::http::Extensions, to: &mut hyper::http::Extensions) {
    if let Some(reason) = from.get::<hyper::ext::ReasonPhrase>() {
        to.insert(reason.clone());
    }
}

fn full(bytes: impl Into<Bytes>) -> ResponseBody {
    Full::new(bytes.into()).map_err(|never| match never {}).boxed()
}

fn bare(status: StatusCode) -> Response<ResponseBody> {
    let mut response = Response::new(Empty::new().map_err(|never| match never {}).boxed());
    *response.status_mut() = status;
    response
}

fn json_error(status: StatusCode, code: &str) -> Response<ResponseBody> {
    let body = format!("{{\"error\":\"{code}\"}}");
    let mut response = Response::new(full(body));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn qos(shared: &Shared) -> Response<ResponseBody> {
    shared.counts.qos.fetch_add(1, Ordering::Relaxed);
    let mut response = Response::new(full(QOS_BODY));
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/html"));
    headers.insert(CONTENT_LENGTH, HeaderValue::from(QOS_BODY.len()));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
