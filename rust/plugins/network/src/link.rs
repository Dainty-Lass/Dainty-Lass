//! Node links (DD16): how one node calls another. [`crate::tls`] carries them over mutual TLS on the
//! gateways' host-only `/__node` routes, one route per [`Service`]; [`crate::inprocess`] carries them
//! between nodes in one process for tests, and [`NoPeers`] is the link of a server outside a network.
//!
//! A call is one request body and one answer body, both JSON a [`Service`] defines. The link
//! authenticates the caller and hands its [`NodeId`] to the [`NodeService`]; services trust that name, not
//! anything inside the body.

use module_api::BoxFuture;
use module_api::network::NodeId;
use serde::{Deserialize, Serialize};

/// The most bytes one request or answer may hold, either way. A link refuses a bigger body before it
/// is parsed.
pub const MAX_MESSAGE: usize = 1 << 20;

/// How far apart two nodes' clocks may be before a link between them is refused (DN3). Tickets and the
/// directory's timestamps assume nodes agree to well within this.
pub const MAX_CLOCK_SKEW: std::time::Duration = std::time::Duration::from_secs(30);

/// What a call is for: one host-only route each on DN3's links.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    /// The directory: registrations through the leader, and the log replicas pull (DN4)
    Directory,
    /// The home API: session tickets, and operations on an account at its home (DN5)
    Home,
    /// Operator actions relayed between nodes: an admin's kick or ban applied on every node (DN11)
    Admin,
    /// A node's change log shipped to a peer, which keeps it (DD23 layer 2, DH7)
    LogShip,
    /// Chat federation: direct messages, room fan-out, presence and mutes between nodes (DN10b)
    Chat,
}

impl Service {
    pub const ALL: [Service; 5] = [Self::Directory, Self::Home, Self::Admin, Self::LogShip, Self::Chat];
}

impl Service {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::Home => "home",
            Self::Admin => "admin",
            Self::LogShip => "log_ship",
            Self::Chat => "chat",
        }
    }

    /// The service a route name stands for: exactly its [`as_str`](Self::as_str), nothing folded.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|service| service.as_str() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    /// This server is not in a network: it has no one to call
    #[error("this server is not in a network")]
    NoPeers,
    #[error("{0} is not a peer of this node")]
    UnknownPeer(NodeId),
    #[error("{node} did not answer: {reason}")]
    Unreachable { node: NodeId, reason: String },
    #[error("a node-link message is over {MAX_MESSAGE} bytes")]
    TooLarge,
    #[error("{node} does not serve {}", .service.as_str())]
    NoService { node: NodeId, service: Service },
    /// The peer's clock and this node's differ by more than [`MAX_CLOCK_SKEW`] (DN3): its minus ours, in ms
    #[error("{node}'s clock is {skew_ms} ms off this node's (the most allowed is {} ms)", MAX_CLOCK_SKEW.as_millis())]
    ClockSkew { node: NodeId, skew_ms: i64 },
    /// The peer refused this node: it doesn't list it as a peer, or didn't accept its certificate
    #[error("{node} refused this node: {reason}")]
    Refused { node: NodeId, reason: String },
    /// What answered at the peer's address isn't that peer on this cluster: a certificate from another CA, or
    /// another node's name
    #[error("what answered for {node} is not it: {reason}")]
    Untrusted { node: NodeId, reason: String },
    /// The peer answered something a node link doesn't say
    #[error("{node} broke the node-link protocol: {reason}")]
    Protocol { node: NodeId, reason: String },
}

/// The calling side of a node link.
pub trait PeerLink: Send + Sync {
    /// Sends `body` to `to`'s `service` and waits for the answer.
    fn call<'a>(&'a self, to: &'a NodeId, service: Service, body: Vec<u8>)
    -> BoxFuture<'a, Result<Vec<u8>, LinkError>>;
}

/// The answering side: what a link hands a call it received. `from` is the caller as the link
/// authenticated it (DN3: the node certificate's name).
pub trait NodeService: Send + Sync {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>>;
}

/// Who the peers of this node are and whether each is reachable now, from the link's own health checks. A
/// service that must reach every node (DN11) skips the ones that are down and queues for them.
pub trait PeerRoster: Send + Sync {
    /// Every peer this node lists, not itself.
    fn peers(&self) -> Vec<NodeId>;

    /// Whether `node` is worth calling now: a peer never tried counts as up, so its first call is made.
    fn is_up(&self, node: &NodeId) -> bool;
}

impl PeerRoster for NoPeers {
    fn peers(&self) -> Vec<NodeId> {
        Vec::new()
    }

    fn is_up(&self, _node: &NodeId) -> bool {
        false
    }
}

/// The link of a server that isn't in a network (section 6: it works as a node with no peers).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPeers;

impl PeerLink for NoPeers {
    fn call<'a>(
        &'a self,
        _to: &'a NodeId,
        _service: Service,
        _body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async { Err(LinkError::NoPeers) })
    }
}

/// A node's clock in unix milliseconds. It can be set wrong on purpose (`skew_by`), which is how the tests
/// and an operator's drill make a node whose clock has drifted (DN3).
#[derive(Debug, Default)]
pub struct Clock {
    offset_ms: std::sync::atomic::AtomicI64,
}

impl Clock {
    pub fn now_ms(&self) -> i64 {
        let system = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX));
        system.saturating_add(self.offset_ms.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// From now on, the clock reads `ms` milliseconds ahead of the system's (behind, if negative).
    pub fn skew_by(&self, ms: i64) {
        self.offset_ms.store(ms, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The services one node answers, by [`Service`]: the endpoint attaches one [`NodeService`], so a node with a
/// directory and a home API puts both in a mux (DN5). A call for a service nobody was given is `NoService`.
pub struct ServiceMux {
    node: NodeId,
    services: std::collections::BTreeMap<&'static str, std::sync::Arc<dyn NodeService>>,
}

impl ServiceMux {
    pub fn new(node: NodeId) -> Self {
        Self {
            node,
            services: std::collections::BTreeMap::new(),
        }
    }

    pub fn with(mut self, service: Service, handler: std::sync::Arc<dyn NodeService>) -> Self {
        self.services.insert(service.as_str(), handler);
        self
    }
}

impl NodeService for ServiceMux {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            match self.services.get(service.as_str()) {
                Some(handler) => handler.serve(from, service, body).await,
                None => Err(LinkError::NoService {
                    node: self.node.clone(),
                    service,
                }),
            }
        })
    }
}
