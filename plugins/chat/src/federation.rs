//! New here: the seam DN10b fills (DD21: nodes federated server to server, as XMPP does). On one node chat
//! runs with [`LocalOnly`]: every account and room is this node's, and nothing leaves it.
//!
//! What crosses is an [`Envelope`], the meaning of one message, never the client's XML: the receiving node
//! writes its own stanzas for its own sessions, as this one does. Chat calls the seam after it has handled a
//! message here (stored it, delivered it to local sessions), so DN10b adds routing without changing what a
//! node does on its own. Delivering an envelope that arrives from another node is DN10b's to add, through
//! the chat task like any other event.

use module_api::chat::RoomKind;
use module_api::{BoxFuture, GlobalId};
use serde::{Deserialize, Serialize};

/// One message, as another node needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Envelope {
    /// A direct message. DD21 stores it at both players' homes and routes it to the recipient's node.
    Direct {
        from: GlobalId,
        /// The sender's JID local part, which the recipient's client shows
        from_local: String,
        to: GlobalId,
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// Unix milliseconds
        at: i64,
        /// Whether a session on this node took it
        delivered: bool,
    },
    /// A room message. DD21 keeps a room's history with its owner and replicates it to the nodes that read it.
    Room {
        room: RoomKind,
        from: GlobalId,
        nick: String,
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        at: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FederationError {
    #[error("the node that owns it did not answer: {0}")]
    Unreachable(String),
}

pub trait Federation: Send + Sync {
    /// The account a JID local part names when no session or record on this node knows it (an account homed
    /// elsewhere). On one node: nobody.
    fn resolve<'a>(&'a self, local: &'a str) -> BoxFuture<'a, Option<GlobalId>>;

    /// A message handled here, for the nodes that must also hold or deliver it.
    fn publish(&self, envelope: Envelope) -> BoxFuture<'_, Result<(), FederationError>>;
}

/// A server with no peers (section 6: a node with no peers works as before).
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalOnly;

impl Federation for LocalOnly {
    fn resolve<'a>(&'a self, _local: &'a str) -> BoxFuture<'a, Option<GlobalId>> {
        Box::pin(async { None })
    }

    fn publish(&self, _envelope: Envelope) -> BoxFuture<'_, Result<(), FederationError>> {
        Box::pin(async { Ok(()) })
    }
}
