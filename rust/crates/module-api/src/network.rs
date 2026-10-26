//! The network's seams (DN2, DD16-DD18): who this node is, ids that are the same on every node, and the
//! home API another node calls for an account it doesn't own. Stubs: a server outside a network is a node
//! with no peers ([`NodeIdentity::standalone`], [`StandaloneHome`]), and DN3-DN5 fill the rest.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::ids::name_type;
use crate::module::BoxFuture;

name_type!(
    /// A node's name in the cluster, as its `host.toml` gives it (`[network]`, DF3).
    NodeId,
    "a node id"
);

name_type!(
    /// The region a node serves, such as `oregon` or `germany`.
    Region,
    "a region"
);

/// Who this server is in the network.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeIdentity {
    pub node: NodeId,
    pub region: Region,
}

impl NodeIdentity {
    /// A server that isn't in a network: one node named `local` in region `local`, which works exactly as
    /// a node with no peers (section 6).
    pub fn standalone() -> Self {
        Self {
            node: NodeId(String::from("local")),
            region: Region(String::from("local")),
        }
    }
}

/// What a [`GlobalId`] names (DD18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdKind {
    Account,
    Hunt,
    HubInstance,
    Party,
    Ticket,
}

impl IdKind {
    pub const ALL: [IdKind; 5] = [Self::Account, Self::Hunt, Self::HubInstance, Self::Party, Self::Ticket];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Hunt => "hunt",
            Self::HubInstance => "hub_instance",
            Self::Party => "party",
            Self::Ticket => "ticket",
        }
    }
}

/// A network-wide id: a ULID tagged with its kind and the node that owns it (DD18), written
/// `hunt:01J...@oregon-1`. Every node resolves one to its owner without asking anyone.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GlobalId {
    kind: IdKind,
    ulid: Ulid,
    node: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a global id is <kind>:<ULID>@<node>, such as hunt:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@oregon-1")]
pub struct GlobalIdError;

impl GlobalId {
    /// A new id owned by `node`.
    pub fn new(kind: IdKind, node: &NodeId) -> Self {
        Self {
            kind,
            ulid: Ulid::generate(),
            node: node.clone(),
        }
    }

    pub fn from_parts(kind: IdKind, ulid: Ulid, node: NodeId) -> Self {
        Self { kind, ulid, node }
    }

    pub fn kind(&self) -> IdKind {
        self.kind
    }

    pub fn ulid(&self) -> Ulid {
        self.ulid
    }

    /// The node that owns what this names.
    pub fn owner(&self) -> &NodeId {
        &self.node
    }
}

impl FromStr for GlobalId {
    type Err = GlobalIdError;

    fn from_str(text: &str) -> Result<Self, GlobalIdError> {
        let (kind, rest) = text.split_once(':').ok_or(GlobalIdError)?;
        let (ulid, node) = rest.split_once('@').ok_or(GlobalIdError)?;
        let kind = IdKind::ALL
            .into_iter()
            .find(|k| k.as_str() == kind)
            .ok_or(GlobalIdError)?;
        let parsed = Ulid::from_string(ulid).map_err(|_| GlobalIdError)?;
        // Only the canonical spelling, so an id has one text and compares as text everywhere
        if parsed.to_string() != ulid {
            return Err(GlobalIdError);
        }
        let node = NodeId::new(node).map_err(|_| GlobalIdError)?;
        Ok(Self {
            kind,
            ulid: parsed,
            node,
        })
    }
}

impl TryFrom<String> for GlobalId {
    type Error = GlobalIdError;

    fn try_from(text: String) -> Result<Self, GlobalIdError> {
        text.parse()
    }
}

impl From<GlobalId> for String {
    fn from(id: GlobalId) -> String {
        id.to_string()
    }
}

impl fmt::Display for GlobalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}@{}", self.kind.as_str(), self.ulid, self.node)
    }
}

/// A short-lived ticket an account's home signs so another node can act for the player (DD17); the
/// account key itself never leaves its home. Opaque here: DN5 decides its form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionTicket {
    pub account: GlobalId,
    pub audience: NodeId,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum HomeError {
    /// This server has no peers, so only accounts it owns resolve
    #[error("this server is not in a network")]
    NotInNetwork,
    #[error("no node owns that account")]
    UnknownAccount,
    #[error("the home node did not answer: {0}")]
    Unreachable(String),
}

/// A call to an account's home, as it crosses the module pipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HomeCall {
    ResolveHome { account: GlobalId },
    SessionTicket { account: GlobalId, audience: NodeId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HomeAnswer {
    Home(NodeIdentity),
    Ticket(SessionTicket),
}

/// The home API (DD17): what a node asks the home of an account it doesn't own. The network plugin
/// implements it over node links (DN5); modules reach it through `ModuleContext`.
pub trait HomeApi: Send + Sync {
    /// The node an account is homed on.
    fn resolve_home<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<NodeIdentity, HomeError>>;

    /// A ticket from the account's home that lets `audience` act for the player.
    fn session_ticket<'a>(
        &'a self,
        account: &'a GlobalId,
        audience: &'a NodeId,
    ) -> BoxFuture<'a, Result<SessionTicket, HomeError>>;
}

/// Answers a [`HomeCall`] from a [`HomeApi`], for the host side of the pipe.
pub async fn dispatch_home(api: &dyn HomeApi, call: HomeCall) -> Result<HomeAnswer, HomeError> {
    match call {
        HomeCall::ResolveHome { account } => api.resolve_home(&account).await.map(HomeAnswer::Home),
        HomeCall::SessionTicket { account, audience } => {
            api.session_ticket(&account, &audience).await.map(HomeAnswer::Ticket)
        }
    }
}

/// The home API of a server with no peers: every account it owns is homed here, every other is
/// unknown, and no ticket is issued (stub until DN5).
#[derive(Debug, Clone)]
pub struct StandaloneHome {
    pub identity: NodeIdentity,
}

impl HomeApi for StandaloneHome {
    fn resolve_home<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<NodeIdentity, HomeError>> {
        Box::pin(async move {
            if account.kind() != IdKind::Account {
                return Err(HomeError::UnknownAccount);
            }
            if account.owner() == &self.identity.node {
                Ok(self.identity.clone())
            } else {
                Err(HomeError::NotInNetwork)
            }
        })
    }

    fn session_ticket<'a>(
        &'a self,
        _account: &'a GlobalId,
        _audience: &'a NodeId,
    ) -> BoxFuture<'a, Result<SessionTicket, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_ids_round_trip_and_name_their_owner() {
        let node = NodeId::new("oregon-1").unwrap();
        for kind in IdKind::ALL {
            let id = GlobalId::new(kind, &node);
            let text = id.to_string();
            assert!(text.starts_with(kind.as_str()) && text.ends_with("@oregon-1"), "{text}");
            let back: GlobalId = text.parse().unwrap();
            assert_eq!(back, id);
            assert_eq!(back.owner(), &node);
            assert_eq!(back.kind(), kind);
            assert_eq!(
                serde_json::from_str::<GlobalId>(&serde_json::to_string(&id).unwrap()).unwrap(),
                id
            );
        }
        let a = GlobalId::new(IdKind::Hunt, &node);
        let b = GlobalId::new(IdKind::Hunt, &node);
        assert_ne!(a, b);
    }

    #[test]
    fn global_ids_have_one_spelling() {
        let good = "hunt:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@oregon-1";
        assert_eq!(good.parse::<GlobalId>().unwrap().to_string(), good);
        for bad in [
            "hunt:01j9z3n5c8q0v2x4y6w8t0r2p4@oregon-1",
            "hunt:01J9Z3N5C8Q0V2X4Y6W8T0R2P@oregon-1",
            "hunt:81J9Z3N5C8Q0V2X4Y6W8T0R2P4@oregon-1",
            "raid:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@oregon-1",
            "hunt:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@Oregon",
            "hunt:01J9Z3N5C8Q0V2X4Y6W8T0R2P4",
            "hunt01J9Z3N5C8Q0V2X4Y6W8T0R2P4@oregon-1",
            "hunt:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@oregon-1@x",
            "",
        ] {
            assert!(bad.parse::<GlobalId>().is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn a_standalone_home_owns_its_own_accounts_only() {
        let home = StandaloneHome {
            identity: NodeIdentity::standalone(),
        };
        let ours = GlobalId::new(IdKind::Account, &home.identity.node);
        let theirs = GlobalId::new(IdKind::Account, &NodeId::new("germany-1").unwrap());
        let hunt = GlobalId::new(IdKind::Hunt, &home.identity.node);
        assert_eq!(home.resolve_home(&ours).await, Ok(NodeIdentity::standalone()));
        assert_eq!(home.resolve_home(&theirs).await, Err(HomeError::NotInNetwork));
        assert_eq!(home.resolve_home(&hunt).await, Err(HomeError::UnknownAccount));
        let audience = NodeId::new("germany-1").unwrap();
        assert_eq!(
            home.session_ticket(&ours, &audience).await,
            Err(HomeError::NotInNetwork)
        );
        let answer = dispatch_home(&home, HomeCall::ResolveHome { account: ours }).await;
        assert_eq!(answer, Ok(HomeAnswer::Home(NodeIdentity::standalone())));
    }
}
