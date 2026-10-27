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
}

impl Service {
    pub const ALL: [Service; 1] = [Self::Directory];
}

impl Service {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
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
