//! Node links (DD16): how one node calls another. DN3 carries them over mutual TLS on host-only gateway
//! routes, one route per [`Service`]; until then [`crate::inprocess`] carries them between nodes in one
//! process, and [`NoPeers`] is the link of a server outside a network.
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

/// What a call is for: one host-only route each on DN3's links.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    /// The directory: registrations through the leader, and the log replicas pull (DN4)
    Directory,
}

impl Service {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
        }
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
