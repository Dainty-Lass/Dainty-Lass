//! What the chat service and the core say to each other beyond the service protocol (RD27): what crossed in-process
//! as `ChatHost` and the chat drivers hook. Chat keeps its sessions, rooms, history and federation; the core keeps
//! the game modules' drivers, the home API, the directory, the event bus and the node link, and answers for them.
//!
//! Every request carries an id the asking side chose; the answer names it. Each side holds at most
//! [`MAX_PENDING`] unanswered requests and waits [`REPLY_TIMEOUT`] for one.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use module_api::chat::{ChatAnswer, ChatQuery, PresenceEvent};
use module_api::network::{NodeId, Region};
use module_api::{BoxFuture, ErrorKind, GlobalId, ModuleError, ModuleId, NodeIdentity};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::FromService;

/// Unanswered requests one side holds; past it a request is refused as unavailable.
pub const MAX_PENDING: usize = 256;
/// How long a side waits for an answer before giving up on it.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// A JID local part, as chat's `is_authcid_shape` bounds it, with room to spare
pub const MAX_LOCAL: usize = 256;
/// A game token (1.14.7: a JWT)
pub const MAX_TOKEN: usize = 16 * 1024;
/// Longest [`PeerError`] reason carried, in bytes.
pub const MAX_REASON: usize = 512;
/// Most peers a [`ChatNetwork`] may list.
pub const MAX_PEERS: usize = 1024;

/// Who this node is and, in a network, who leads it and its peers (DN10b): what `ChatOptions` gave in-process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatSetup {
    /// This node.
    pub node: NodeIdentity,
    /// The network it is in; `None` for a server on its own.
    pub network: Option<ChatNetwork>,
}

/// The network a node is in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatNetwork {
    /// The node that leads it.
    pub leader: NodeId,
    /// Every peer and its region.
    pub peers: Vec<(NodeId, Region)>,
}

/// The core to chat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToChat {
    /// Before `Start`, once per instance
    Setup {
        /// This node and its network.
        setup: Box<ChatSetup>,
    },
    /// A game module with the chat capability started, or is gone: whom [`FromChat::Ask`] reaches
    Loaded {
        /// The module that started.
        module: ModuleId,
    },
    /// A game module with the chat capability is gone.
    Unloaded {
        /// The module that is gone.
        module: ModuleId,
    },
    /// The answer to [`FromChat::Ask`].
    Answer {
        /// Id of the [`FromChat::Ask`] it answers.
        id: u64,
        /// What the module answered.
        result: Result<ChatAnswer, ModuleError>,
    },
    /// The display name the account's home gave, none when it refused
    HomeLoggedIn {
        /// Id of the [`FromChat::HomeLogin`] it answers.
        id: u64,
        /// Display name; `None` when the home refused.
        name: Option<String>,
    },
    /// The answer to [`FromChat::FindAccount`].
    Account {
        /// Id of the [`FromChat::FindAccount`] it answers.
        id: u64,
        /// The account; `None` when the directory holds none.
        account: Option<GlobalId>,
    },
    /// The answer to [`FromChat::PeerCall`].
    PeerAnswer {
        /// Id of the [`FromChat::PeerCall`] it answers.
        id: u64,
        /// The peer's answer, or why there is none.
        result: Result<serde_json::Value, PeerError>,
    },
    /// A peer's call to this node's `Service::Chat`, as the node link authenticated `from`
    PeerServe {
        /// Id the answer goes back under, in [`FromChat::PeerServed`].
        id: u64,
        /// The calling node.
        from: NodeId,
        /// Request body.
        body: serde_json::Value,
    },
}

/// Chat to the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum FromChat {
    /// `GameModule::chat` on one loaded module
    Ask {
        /// Id the answer comes back under.
        id: u64,
        /// The module asked.
        module: ModuleId,
        /// The query.
        query: ChatQuery,
    },
    /// A login whose account is homed elsewhere: `chat.login` run at its home through `module`
    HomeLogin {
        /// Id the answer comes back under.
        id: u64,
        /// Module that reaches the account's home.
        module: ModuleId,
        /// The account logging in.
        account: GlobalId,
        /// JID local part the client logged in with, at most [`MAX_LOCAL`] bytes.
        authcid: String,
        /// The client's game token, at most [`MAX_TOKEN`] bytes.
        token: String,
    },
    /// The account a JID local part is in the network's directory
    FindAccount {
        /// Id the answer comes back under.
        id: u64,
        /// The local part, at most [`MAX_LOCAL`] bytes.
        local: String,
    },
    /// Emitted as `chat.presence`
    Presence {
        /// The presence change.
        event: PresenceEvent,
    },
    /// A call to `to`'s `Service::Chat`
    PeerCall {
        /// Id the answer comes back under.
        id: u64,
        /// The node called.
        to: NodeId,
        /// Request body.
        body: serde_json::Value,
    },
    /// Chat's answer to [`ToChat::PeerServe`].
    PeerServed {
        /// Id of the [`ToChat::PeerServe`] it answers.
        id: u64,
        /// The answer, or why there is none.
        result: Result<serde_json::Value, PeerError>,
    },
}

impl FromChat {
    /// Whether its strings are within bounds; the core refuses one that isn't.
    pub fn in_bounds(&self) -> bool {
        match self {
            Self::HomeLogin { authcid, token, .. } => authcid.len() <= MAX_LOCAL && token.len() <= MAX_TOKEN,
            Self::FindAccount { local, .. } => local.len() <= MAX_LOCAL,
            Self::Presence { event } => event.user_id.len() <= MAX_LOCAL,
            Self::PeerServed { result: Err(error), .. } => error.in_bounds(),
            Self::Ask { .. } | Self::PeerCall { .. } | Self::PeerServed { .. } => true,
        }
    }
}

/// A node-link error (`network::LinkError`), naming no node: the side that converts it back knows which.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PeerError {
    /// This server is not in a network.
    NoPeers {},
    /// The node is not a peer of this one.
    UnknownPeer {},
    /// The peer did not answer.
    Unreachable {
        /// Why, at most [`MAX_REASON`] bytes.
        reason: String,
    },
    /// The message is over the node link's limit.
    TooLarge {},
    /// The peer does not serve chat.
    NoService {},
    /// The peer's clock is too far off this node's (DN3).
    ClockSkew {
        /// Its clock minus ours, in ms.
        skew_ms: i64,
    },
    /// The peer refused this node.
    Refused {
        /// Why, at most [`MAX_REASON`] bytes.
        reason: String,
    },
    /// What answered isn't that peer on this cluster.
    Untrusted {
        /// Why, at most [`MAX_REASON`] bytes.
        reason: String,
    },
    /// The peer answered something a node link doesn't say.
    Protocol {
        /// What was wrong, at most [`MAX_REASON`] bytes.
        reason: String,
    },
}

impl PeerError {
    fn in_bounds(&self) -> bool {
        match self {
            Self::Unreachable { reason }
            | Self::Refused { reason }
            | Self::Untrusted { reason }
            | Self::Protocol { reason } => reason.len() <= MAX_REASON,
            _ => true,
        }
    }

    /// The same, with its reason cut to [`MAX_REASON`].
    pub fn bounded(self) -> Self {
        let cut = |mut reason: String| {
            if reason.len() > MAX_REASON {
                let mut end = MAX_REASON;
                while !reason.is_char_boundary(end) {
                    end -= 1;
                }
                reason.truncate(end);
            }
            reason
        };
        match self {
            Self::Unreachable { reason } => Self::Unreachable { reason: cut(reason) },
            Self::Refused { reason } => Self::Refused { reason: cut(reason) },
            Self::Untrusted { reason } => Self::Untrusted { reason: cut(reason) },
            Self::Protocol { reason } => Self::Protocol { reason: cut(reason) },
            other => other,
        }
    }
}

/// What chat serves its peers with: `Service::Chat`'s `NodeService`.
pub trait PeerServer: Send + Sync {
    /// Answers `from`'s call with `body`.
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        body: serde_json::Value,
    ) -> BoxFuture<'a, Result<serde_json::Value, PeerError>>;
}

/// The library side: chat's handle on the core, one per open instance ([`crate::Service::chat_link`]).
#[derive(Clone)]
pub struct ChatLink {
    inner: Arc<Inner>,
}

struct Inner {
    /// Taken at close, so a link the service keeps doesn't hold the instance's writer open
    out: Mutex<Option<mpsc::UnboundedSender<FromService>>>,
    next: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<ToChat>>>,
    setup: Mutex<Option<ChatSetup>>,
    loaded: Mutex<BTreeSet<ModuleId>>,
    server: Mutex<Option<Arc<dyn PeerServer>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn unavailable(what: &str) -> ModuleError {
    ModuleError::new(ErrorKind::Unavailable, format!("the core did not answer {what}"))
}

impl ChatLink {
    pub(crate) fn new(out: mpsc::UnboundedSender<FromService>) -> Self {
        Self {
            inner: Arc::new(Inner {
                out: Mutex::new(Some(out)),
                next: AtomicU64::new(1),
                pending: Mutex::default(),
                setup: Mutex::default(),
                loaded: Mutex::default(),
                server: Mutex::default(),
            }),
        }
    }

    /// What the core sent before `Start`; none from a core that runs chat outside it.
    pub fn setup(&self) -> Option<ChatSetup> {
        lock(&self.inner.setup).clone()
    }

    /// The loaded game modules with the chat capability.
    pub fn loaded(&self) -> Vec<ModuleId> {
        lock(&self.inner.loaded).iter().cloned().collect()
    }

    fn send(&self, message: FromChat) {
        if let Some(out) = lock(&self.inner.out).as_ref() {
            _ = out.send(FromService::Chat(message));
        }
    }

    /// Sends the request `make` builds with a fresh id and waits for its answer.
    async fn request(&self, make: impl FnOnce(u64) -> FromChat) -> Option<ToChat> {
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = lock(&self.inner.pending);
            if pending.len() >= MAX_PENDING {
                return None;
            }
            pending.insert(id, tx);
        }
        self.send(make(id));
        let answer = tokio::time::timeout(REPLY_TIMEOUT, rx).await;
        lock(&self.inner.pending).remove(&id);
        answer.ok()?.ok()
    }

    /// Runs `query` on `module`'s chat hook; fails as unavailable when the core does not answer in time.
    pub async fn ask(&self, module: &ModuleId, query: ChatQuery) -> Result<ChatAnswer, ModuleError> {
        let module = module.clone();
        match self.request(|id| FromChat::Ask { id, module, query }).await {
            Some(ToChat::Answer { result, .. }) => result,
            _ => Err(unavailable("a chat query")),
        }
    }

    /// Logs in at the account's home through `module`; returns the display name, or `None` when refused, too long or unanswered.
    pub async fn home_login(
        &self,
        module: &ModuleId,
        account: &GlobalId,
        authcid: &str,
        token: &str,
    ) -> Option<String> {
        if authcid.len() > MAX_LOCAL || token.len() > MAX_TOKEN {
            return None;
        }
        let (module, account) = (module.clone(), account.clone());
        let (authcid, token) = (authcid.to_owned(), token.to_owned());
        let request = |id| FromChat::HomeLogin {
            id,
            module,
            account,
            authcid,
            token,
        };
        match self.request(request).await {
            Some(ToChat::HomeLoggedIn { name, .. }) => name,
            _ => None,
        }
    }

    /// The account a JID local part names in the network's directory; `None` when unknown, too long or unanswered.
    pub async fn find_account(&self, local: &str) -> Option<GlobalId> {
        if local.len() > MAX_LOCAL {
            return None;
        }
        let local = local.to_owned();
        match self.request(|id| FromChat::FindAccount { id, local }).await {
            Some(ToChat::Account { account, .. }) => account,
            _ => None,
        }
    }

    /// Emits `event` as `chat.presence`; dropped when its user id is over [`MAX_LOCAL`].
    pub fn presence(&self, event: PresenceEvent) {
        if event.user_id.len() <= MAX_LOCAL {
            self.send(FromChat::Presence { event });
        }
    }

    /// Calls `to`'s chat service with `body`; an unanswered call comes back as [`PeerError::Unreachable`].
    pub async fn peer_call(&self, to: &NodeId, body: serde_json::Value) -> Result<serde_json::Value, PeerError> {
        let to = to.clone();
        match self.request(|id| FromChat::PeerCall { id, to, body }).await {
            Some(ToChat::PeerAnswer { result, .. }) => result,
            _ => Err(PeerError::Unreachable {
                reason: String::from("the core did not answer"),
            }),
        }
    }

    /// What answers peers' calls from now on; none answers `NoService`.
    pub fn serve_peers(&self, server: Option<Arc<dyn PeerServer>>) {
        *lock(&self.inner.server) = server;
    }

    /// A message from the core, from the instance's read loop.
    pub(crate) fn deliver(&self, message: ToChat) {
        match message {
            ToChat::Setup { setup } => *lock(&self.inner.setup) = Some(*setup),
            ToChat::Loaded { module } => _ = lock(&self.inner.loaded).insert(module),
            ToChat::Unloaded { module } => _ = lock(&self.inner.loaded).remove(&module),
            ToChat::PeerServe { id, from, body } => {
                let link = self.clone();
                tokio::spawn(async move {
                    let server = lock(&link.inner.server).clone();
                    let result = match server {
                        Some(server) => server.serve(&from, body).await.map_err(PeerError::bounded),
                        None => Err(PeerError::NoService {}),
                    };
                    link.send(FromChat::PeerServed { id, result });
                });
            }
            ToChat::Answer { id, .. }
            | ToChat::HomeLoggedIn { id, .. }
            | ToChat::Account { id, .. }
            | ToChat::PeerAnswer { id, .. } => {
                if let Some(tx) = lock(&self.inner.pending).remove(&id) {
                    _ = tx.send(message);
                }
            }
        }
    }

    /// The instance closed: nothing waits on an answer that can't come.
    pub(crate) fn close(&self) {
        lock(&self.inner.out).take();
        lock(&self.inner.pending).clear();
        lock(&self.inner.server).take();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]
    use super::*;

    #[test]
    fn chat_messages_are_tagged_and_refuse_unknown_fields() {
        let ask = FromChat::Ask {
            id: 3,
            module: "stub".parse().unwrap(),
            query: ChatQuery::AccountOf { local: "a".into() },
        };
        let text = serde_json::to_string(&FromService::Chat(ask.clone())).unwrap();
        assert_eq!(
            text,
            r#"{"type":"chat","op":"ask","id":3,"module":"stub","query":{"account_of":{"local":"a"}}}"#
        );
        assert_eq!(
            serde_json::from_str::<FromService>(&text).unwrap(),
            FromService::Chat(ask)
        );
        let loaded = r#"{"type":"chat","op":"loaded","module":"stub"}"#;
        assert!(serde_json::from_str::<crate::ToService>(loaded).is_ok());
        let extra = r#"{"type":"chat","op":"loaded","module":"stub","driver":1}"#;
        assert!(serde_json::from_str::<crate::ToService>(extra).is_err());
        let unknown = r#"{"type":"chat","op":"emit","name":"x"}"#;
        assert!(serde_json::from_str::<FromService>(unknown).is_err());
        let error = PeerError::Unreachable {
            reason: "é".repeat(400),
        }
        .bounded();
        assert!(error.in_bounds());
    }

    #[test]
    fn oversized_strings_are_out_of_bounds() {
        let find = FromChat::FindAccount {
            id: 1,
            local: "a".repeat(MAX_LOCAL + 1),
        };
        assert!(!find.in_bounds());
        assert!(
            FromChat::FindAccount {
                id: 1,
                local: "a".into()
            }
            .in_bounds()
        );
    }
}
