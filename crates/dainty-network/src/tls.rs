//! Node links over mutual TLS (DN3): [`TlsPeerLink`] calls a peer's gateway on its `/__node/<service>`
//! route. The connection is TLS 1.3 under the cluster CA with both sides' node certificates, and the peer
//! must be the node `[[network.nodes]]` says answers at that address: a certificate from another CA, or
//! one that names another node, is refused before a byte of the request is sent.
//!
//! Each call checks the clocks (the peer's `x-node-time` against ours, the bound is
//! [`crate::link::MAX_CLOCK_SKEW`]), and every call and every periodic ping updates the
//! peer's [`PeerHealth`]. Connections are kept for the next call and dropped when they close, so a peer
//! that restarts is reached again on the next call, with no one doing anything.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use bytes::Bytes;
use dainty_module_api::BoxFuture;
use dainty_module_api::NodeIdentity;
use dainty_module_api::network::{NodeId, Region};
use http_body_util::{BodyExt, Full, Limited};
use hyper::client::conn::http1::{self, SendRequest};
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use serde::Serialize;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::cluster::{self, NODE_ALPN, NodeCredentials, Peer};
use crate::link::{Clock, LinkError, MAX_CLOCK_SKEW, MAX_MESSAGE, PeerLink, PeerRoster, Service};
use crate::wire::{self, PingAnswer, Target};

/// How a [`TlsPeerLink`] reaches its peers.
pub struct TlsLinkConfig {
    /// This node's certificate, key and the cluster CA.
    pub credentials: NodeCredentials,
    /// The peers, as `[[network.nodes]]` lists them.
    pub peers: Vec<Peer>,
    /// Reaching a peer and finishing the TLS handshake
    pub connect_timeout: Duration,
    /// One call, connecting included
    pub request_timeout: Duration,
    /// How often each peer is pinged.
    pub ping_interval: Duration,
    /// How far a peer's clock may differ before its calls are refused.
    pub max_skew: Duration,
    /// The clock pings and calls are stamped with.
    pub clock: Arc<Clock>,
}

impl TlsLinkConfig {
    /// 5 s to connect, 15 s per call, a ping every 10 s and the standard skew bound.
    pub fn new(credentials: NodeCredentials, peers: Vec<Peer>) -> Self {
        Self {
            credentials,
            peers,
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(15),
            ping_interval: Duration::from_secs(10),
            max_skew: MAX_CLOCK_SKEW,
            clock: Arc::new(Clock::default()),
        }
    }
}

/// What this node knows of a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerState {
    /// Not called or pinged yet
    Unknown,
    /// The last call or ping was answered.
    Up,
    /// Not reachable, or no answer in time
    Down,
    /// Reachable, but its clock and ours differ by more than the bound: its calls are refused
    ClockSkew,
    /// It refused this node: it doesn't list it, or doesn't accept its certificate
    Refused,
    /// What answered isn't that peer on this cluster
    Untrusted,
}

/// What this node knows of one peer, for `status` and the roster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PeerHealth {
    /// The peer.
    pub node: NodeId,
    /// Its region.
    pub region: Region,
    /// Up, down or refused.
    pub state: PeerState,
    /// The last ping's round trip
    pub rtt_ms: Option<u64>,
    /// The peer's clock minus ours, at the last answer
    pub skew_ms: Option<i64>,
    /// Unix ms of the last answer that counted as up
    pub last_ok_ms: Option<i64>,
    /// Unix ms the peer entered this state
    pub since_ms: i64,
    /// Failures in a row
    pub failures: u32,
    /// The last failure, if any.
    pub last_error: Option<String>,
}

/// A ping that was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingReport {
    /// The round trip.
    pub rtt: Duration,
    /// The peer's clock minus ours, in milliseconds.
    pub skew_ms: i64,
}

type Sender = SendRequest<Full<Bytes>>;

struct Inner {
    me: NodeIdentity,
    peers: Vec<Peer>,
    connector: TlsConnector,
    connect_timeout: Duration,
    request_timeout: Duration,
    ping_interval: Duration,
    max_skew: Duration,
    clock: Arc<Clock>,
    idle: Mutex<HashMap<NodeId, Sender>>,
    health: Mutex<HashMap<NodeId, PeerHealth>>,
}

/// The calling side of a node link over mutual TLS; cheap to clone.
#[derive(Clone)]
pub struct TlsPeerLink {
    inner: Arc<Inner>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A failure's cause: TLS errors say whether the peer's certificate or ours was the trouble.
fn classify(node: &NodeId, error: &(dyn std::error::Error + 'static)) -> LinkError {
    let mut source = Some(error);
    while let Some(current) = source {
        // io::Error's `source` skips the error it wraps, which is where tokio-rustls puts TLS's
        let tls = current.downcast_ref::<rustls::Error>().or_else(|| {
            let inner = current.downcast_ref::<std::io::Error>()?.get_ref()?;
            inner.downcast_ref::<rustls::Error>()
        });
        if let Some(tls) = tls {
            let reason = tls.to_string();
            return match tls {
                rustls::Error::InvalidCertificate(_) | rustls::Error::NoCertificatesPresented => LinkError::Untrusted {
                    node: node.clone(),
                    reason,
                },
                rustls::Error::AlertReceived(_) => LinkError::Refused {
                    node: node.clone(),
                    reason,
                },
                _ => LinkError::Unreachable {
                    node: node.clone(),
                    reason,
                },
            };
        }
        source = current.source();
    }
    LinkError::Unreachable {
        node: node.clone(),
        reason: error.to_string(),
    }
}

fn unreachable(node: &NodeId, reason: impl Into<String>) -> LinkError {
    LinkError::Unreachable {
        node: node.clone(),
        reason: reason.into(),
    }
}

impl TlsPeerLink {
    /// Checks the peer list against this node's own name and builds the TLS client; no connection is made yet.
    pub fn new(config: TlsLinkConfig) -> Result<Self, cluster::CertError> {
        let me = config.credentials.identity().clone();
        cluster::check_peers(&me.node, &config.peers)?;
        let connector = TlsConnector::from(config.credentials.client_config()?);
        let now = config.clock.now_ms();
        let health = config
            .peers
            .iter()
            .map(|peer| {
                let unknown = PeerHealth {
                    node: peer.id.clone(),
                    region: peer.region.clone(),
                    state: PeerState::Unknown,
                    rtt_ms: None,
                    skew_ms: None,
                    last_ok_ms: None,
                    since_ms: now,
                    failures: 0,
                    last_error: None,
                };
                (peer.id.clone(), unknown)
            })
            .collect();
        Ok(Self {
            inner: Arc::new(Inner {
                me,
                peers: config.peers,
                connector,
                connect_timeout: config.connect_timeout,
                request_timeout: config.request_timeout,
                ping_interval: config.ping_interval,
                max_skew: config.max_skew,
                clock: config.clock,
                idle: Mutex::default(),
                health: Mutex::new(health),
            }),
        })
    }

    /// This node, as its certificate names it.
    pub fn identity(&self) -> &NodeIdentity {
        &self.inner.me
    }

    /// Every peer's health, in the order `[[network.nodes]]` lists them.
    pub fn health(&self) -> Vec<PeerHealth> {
        let health = lock(&self.inner.health);
        self.inner
            .peers
            .iter()
            .filter_map(|p| health.get(&p.id).cloned())
            .collect()
    }

    /// `node`'s health, if it is a listed peer.
    pub fn peer_health(&self, node: &NodeId) -> Option<PeerHealth> {
        lock(&self.inner.health).get(node).cloned()
    }

    fn peer(&self, node: &NodeId) -> Result<&Peer, LinkError> {
        self.inner
            .peers
            .iter()
            .find(|peer| &peer.id == node)
            .ok_or_else(|| LinkError::UnknownPeer(node.clone()))
    }

    /// A connection to `peer`: the one kept from the last call if it is still open, else a new one.
    async fn sender(&self, peer: &Peer) -> Result<(Sender, bool), LinkError> {
        let kept = lock(&self.inner.idle).remove(&peer.id);
        if let Some(mut sender) = kept
            && sender.ready().await.is_ok()
        {
            return Ok((sender, false));
        }
        Ok((self.connect(peer).await?, true))
    }
}

impl TlsPeerLink {
    async fn connect(&self, peer: &Peer) -> Result<Sender, LinkError> {
        let inner = &self.inner;
        let node = &peer.id;
        let timed_out = || unreachable(node, "the connection timed out");
        let tcp = tokio::time::timeout(inner.connect_timeout, TcpStream::connect(peer.dial()))
            .await
            .map_err(|_| timed_out())?
            .map_err(|e| unreachable(node, e.to_string()))?;
        let _ = tcp.set_nodelay(true);
        let name = ServerName::try_from(peer.dial().0.to_owned()).map_err(|e| unreachable(node, e.to_string()))?;
        let tls = tokio::time::timeout(inner.connect_timeout, inner.connector.connect(name, tcp))
            .await
            .map_err(|_| timed_out())?
            .map_err(|e| classify(node, &e))?;
        let (_, session) = tls.get_ref();
        if session.alpn_protocol() != Some(NODE_ALPN) {
            return Err(LinkError::Untrusted {
                node: node.clone(),
                reason: "it does not speak node links".into(),
            });
        }
        let untrusted = |reason: String| LinkError::Untrusted {
            node: node.clone(),
            reason,
        };
        let identity = cluster::peer_identity(session.peer_certificates().unwrap_or_default())
            .map_err(|e| untrusted(e.to_string()))?;
        if identity.node != peer.id || identity.region != peer.region {
            return Err(untrusted(format!(
                "its certificate says it is {} in {}, not {} in {}",
                identity.node, identity.region, peer.id, peer.region
            )));
        }
        let (sender, connection) = http1::handshake(TokioIo::new(tls))
            .await
            .map_err(|e| classify(node, &e))?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(sender)
    }

    fn request(&self, peer: &Peer, method: Method, target: Target, body: Vec<u8>) -> Request<Full<Bytes>> {
        let (dial_host, dial_port) = peer.dial();
        let host = if dial_host.contains(':') {
            format!("[{dial_host}]:{dial_port}")
        } else {
            format!("{dial_host}:{dial_port}")
        };
        let mut request = Request::builder()
            .method(method)
            .uri(target.path())
            .header("host", host)
            .header(wire::TIME_HEADER, self.inner.clock.now_ms().to_string());
        if matches!(target, Target::Service(_)) {
            request = request
                .header("content-type", "application/json")
                .header("content-length", body.len());
        }
        request
            .body(Full::new(Bytes::from(body)))
            .unwrap_or_else(|_| Request::new(Full::new(Bytes::new())))
    }
}

impl TlsPeerLink {
    /// One request and its whole answer: the status, the peer's clock and the body (at most [`MAX_MESSAGE`]).
    async fn exchange(&self, peer: &Peer, method: Method, target: Target, body: Vec<u8>) -> Result<Bytes, LinkError> {
        let node = &peer.id;
        for attempt in 0..2 {
            let (mut sender, fresh) = self.sender(peer).await?;
            let request = self.request(peer, method.clone(), target, body.clone());
            let response = match sender.try_send_request(request).await {
                Ok(response) => response,
                // A kept connection that closed under us: nothing was sent, so try a fresh one
                Err(e) if !fresh && attempt == 0 && e.message().is_some() => continue,
                Err(e) => return Err(classify(node, e.error())),
            };
            let (parts, incoming) = response.into_parts();
            let collected = Limited::new(incoming, MAX_MESSAGE).collect().await.map_err(|e| {
                if e.is::<http_body_util::LengthLimitError>() {
                    LinkError::TooLarge
                } else {
                    classify(node, &*e)
                }
            })?;
            let answer = collected.to_bytes();
            let closes = parts
                .headers
                .get("connection")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.eq_ignore_ascii_case("close"));
            if !closes {
                lock(&self.inner.idle).insert(node.clone(), sender);
            }
            let service = match target {
                Target::Service(service) => Some(service),
                Target::Ping => None,
            };
            if parts.status != StatusCode::OK {
                return Err(wire::link_error(node, service, parts.status.as_u16(), &answer));
            }
            let theirs = parts
                .headers
                .get(wire::TIME_HEADER)
                .and_then(|v| v.to_str().ok())
                .and_then(wire::parse_time)
                .ok_or_else(|| LinkError::Protocol {
                    node: node.clone(),
                    reason: "its answer carries no clock".into(),
                })?;
            let skew = wire::skew_ms(self.inner.clock.now_ms(), theirs);
            if !wire::within(skew, self.inner.max_skew) {
                return Err(LinkError::ClockSkew {
                    node: node.clone(),
                    skew_ms: skew,
                });
            }
            return Ok(answer);
        }
        Err(unreachable(node, "the connection kept closing"))
    }

    async fn timed(&self, peer: &Peer, method: Method, target: Target, body: Vec<u8>) -> Result<Bytes, LinkError> {
        // A tunnelled hunt launch waits for its game server, as the Director's own HTTP client does
        let limit = match target {
            Target::Service(Service::Tunnel) => crate::tunnel::CALL_TIMEOUT.max(self.inner.request_timeout),
            _ => self.inner.request_timeout,
        };
        tokio::time::timeout(limit, self.exchange(peer, method, target, body))
            .await
            .unwrap_or_else(|_| Err(unreachable(&peer.id, "no answer in time")))
    }

    /// Asks `to` who it is and what time it has, and records the answer in its health.
    pub async fn ping(&self, to: &NodeId) -> Result<PingReport, LinkError> {
        let peer = self.peer(to)?;
        let (started, sent_ms) = (Instant::now(), self.inner.clock.now_ms());
        let result = self
            .timed(peer, Method::GET, Target::Ping, Vec::new())
            .await
            .and_then(|body| {
                let rtt = started.elapsed();
                let answer = PingAnswer::decode(&body).map_err(|reason| LinkError::Protocol {
                    node: peer.id.clone(),
                    reason,
                })?;
                if answer.node != peer.id || answer.region != peer.region {
                    return Err(LinkError::Untrusted {
                        node: peer.id.clone(),
                        reason: format!("it answers as {} in {}", answer.node, answer.region),
                    });
                }
                // Its clock against ours at the middle of the round trip
                let middle = sent_ms.saturating_add(i64::try_from(rtt.as_millis() / 2).unwrap_or(0));
                let skew_ms = wire::skew_ms(middle, answer.time_ms);
                if !wire::within(skew_ms, self.inner.max_skew) {
                    return Err(LinkError::ClockSkew {
                        node: peer.id.clone(),
                        skew_ms,
                    });
                }
                Ok(PingReport { rtt, skew_ms })
            });
        self.record(&peer.id, result.as_ref().ok().copied(), result.as_ref().err());
        result
    }
}

impl TlsPeerLink {
    /// Updates a peer's health with how a call or ping went. A node that answered with a refusal about the
    /// call itself (no such service, too large, odd answer) is up; the link worked.
    fn record(&self, node: &NodeId, ping: Option<PingReport>, error: Option<&LinkError>) {
        let now = self.inner.clock.now_ms();
        let mut health = lock(&self.inner.health);
        let Some(entry) = health.get_mut(node) else {
            return;
        };
        let (state, detail) = match error {
            None => (PeerState::Up, None),
            Some(e @ LinkError::Unreachable { .. }) => (PeerState::Down, Some(e)),
            Some(e @ LinkError::ClockSkew { .. }) => (PeerState::ClockSkew, Some(e)),
            Some(e @ LinkError::Refused { .. }) => (PeerState::Refused, Some(e)),
            Some(e @ LinkError::Untrusted { .. }) => (PeerState::Untrusted, Some(e)),
            Some(_) => (PeerState::Up, None),
        };
        let before = entry.state;
        if before != state {
            entry.state = state;
            entry.since_ms = now;
        }
        if let Some(report) = ping {
            entry.rtt_ms = Some(u64::try_from(report.rtt.as_millis()).unwrap_or(u64::MAX));
            entry.skew_ms = Some(report.skew_ms);
        }
        if let Some(LinkError::ClockSkew { skew_ms, .. }) = detail {
            entry.skew_ms = Some(*skew_ms);
        }
        match detail {
            None => {
                entry.failures = 0;
                entry.last_ok_ms = Some(now);
                entry.last_error = None;
            }
            Some(e) => {
                entry.failures = entry.failures.saturating_add(1);
                entry.last_error = Some(e.to_string());
            }
        }
        if before != state {
            match state {
                PeerState::Up if before != PeerState::Unknown => {
                    tracing::info!(peer = %node, "node link is up again");
                }
                PeerState::Up | PeerState::Unknown => {}
                _ => {
                    tracing::warn!(peer = %node, state = ?state, error = entry.last_error.as_deref(), "node link is not healthy")
                }
            }
        }
    }

    /// Pings every peer now and then until the returned task is dropped.
    pub fn spawn_health_checks(&self) -> HealthTask {
        let link = self.clone();
        HealthTask(tokio::spawn(async move {
            let mut every = tokio::time::interval(link.inner.ping_interval);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                every.tick().await;
                let mut pings = tokio::task::JoinSet::new();
                for peer in &link.inner.peers {
                    let (link, id) = (link.clone(), peer.id.clone());
                    pings.spawn(async move {
                        let _ = link.ping(&id).await;
                    });
                }
                while pings.join_next().await.is_some() {}
            }
        }))
    }
}

/// The periodic pings; they stop when this is dropped.
pub struct HealthTask(tokio::task::JoinHandle<()>);

impl Drop for HealthTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The peers `[[network.nodes]]` lists. One that has never been called is tried; one the last call found down,
/// refused, untrusted or off by more than the clock bound waits for the health checks to see it recover.
impl PeerRoster for TlsPeerLink {
    fn peers(&self) -> Vec<NodeId> {
        self.inner.peers.iter().map(|peer| peer.id.clone()).collect()
    }

    fn is_up(&self, node: &NodeId) -> bool {
        matches!(
            self.peer_health(node).map(|health| health.state),
            Some(PeerState::Up | PeerState::Unknown)
        )
    }
}

impl PeerLink for TlsPeerLink {
    fn call<'a>(
        &'a self,
        to: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if body.len() > MAX_MESSAGE {
                return Err(LinkError::TooLarge);
            }
            let peer = self.peer(to)?;
            let result = self
                .timed(peer, Method::POST, Target::Service(service), body)
                .await
                .map(|answer| answer.to_vec());
            self.record(to, None, result.as_ref().err());
            result
        })
    }
}
