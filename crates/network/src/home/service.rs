//! The home's side of the home API (DN5): the node an account is homed on answering other nodes over
//! `Service::Home`.
//!
//! The caller is the node the link authenticated (`from`), never a name in the message. It gets nothing
//! without a ticket this node signed for *it*: the key check (`Check`), a new ticket (`Exchange`) and every
//! operation (`Run`) all verify the ticket against the caller first, so a compromised node can act only for
//! the players whose home handed it a live ticket, for the length of that ticket and its renewals, and only
//! until the home ends their session (`Tickets::revoke`).
//!
//! An operation is a game module's: the plugin finds the module's [`HomeBackend`] in the [`HomeRouter`] and
//! hands it the operation name and JSON body without reading them. A key makes a request run once however
//! often it arrives (a lost answer is resent, not redone): the answer is kept for a while, bounded.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use module_api::network::{GlobalId, NodeId, SessionTicket};
use module_api::{
    BoxFuture, GameVersion, HomeBackend, HomeInvocation, ModuleDriver, ModuleDriverExt, ModuleError, ModuleId,
    NodeIdentity,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use super::protocol::{
    DecodeError, HomeBody, HomeCall, HomeRefusal, HomeReply, HomeRequestCall, HomeResponse, RequestKey, SessionInfo,
    decode_request, encode_response,
};
use super::ticket::{Claims, Expect, TicketError, Tickets};
use crate::link::{LinkError, NodeService, Service};

/// Where an account's home is recorded: the directory (DN4). A trait so the home API can be tested, and a
/// server outside a network can have none.
pub trait HomeLocator: Send + Sync {
    /// The node the account is homed on, none if the directory doesn't know it. Errors are text a log may
    /// show.
    fn home_of<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<NodeId>, String>>;
}

#[cfg(feature = "directory")]
impl HomeLocator for crate::directory::Directory {
    fn home_of<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<NodeId>, String>> {
        Box::pin(async move {
            let found = crate::directory::Directory::home_of(self, account)
                .await
                .map_err(|error| error.to_string())?;
            if found.is_some() {
                return Ok(found);
            }
            let _ = self.sync().await;
            crate::directory::Directory::home_of(self, account)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

/// The game modules that answer for their accounts at this node, by module id (DD3).
#[derive(Default)]
pub struct HomeRouter {
    backends: RwLock<BTreeMap<ModuleId, Arc<dyn HomeBackend>>>,
}

impl HomeRouter {
    /// Answers for `module` from now on, replacing an earlier backend of the same module (a reload).
    pub fn register(&self, module: ModuleId, backend: Arc<dyn HomeBackend>) {
        self.backends
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(module, backend);
    }

    /// Stops answering for `module`.
    pub fn unregister(&self, module: &ModuleId) {
        self.backends
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(module);
    }

    /// The backend answering for `module`, if one is registered.
    pub fn get(&self, module: &ModuleId) -> Option<Arc<dyn HomeBackend>> {
        self.backends
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(module)
            .cloned()
    }
}

/// A loaded module as a [`HomeBackend`]: each operation is the module's `__home` command
/// ([`HomeInvocation::to_command`]), answered in the command's `data`. This is how a module as its own
/// process answers, and a compiled-in one that doesn't implement the trait itself.
pub struct DriverBackend {
    driver: Arc<dyn ModuleDriver>,
    game: GameVersion,
}

impl DriverBackend {
    /// Answers for `driver`'s module and game version.
    pub fn new(driver: Arc<dyn ModuleDriver>) -> Self {
        let game = driver.manifest().game_version.clone();
        Self { driver, game }
    }
}

impl HomeBackend for DriverBackend {
    fn game(&self) -> &GameVersion {
        &self.game
    }

    fn run<'a>(&'a self, invocation: HomeInvocation) -> BoxFuture<'a, Result<Value, ModuleError>> {
        Box::pin(async move {
            let output = self.driver.command(invocation.to_command()).await?;
            Ok(output.data.unwrap_or(Value::Null))
        })
    }
}

/// Limits on how a [`HomeService`] runs calls.
#[derive(Debug, Clone, Copy)]
pub struct HomeConfig {
    /// How long a request waits for the module before it is told to ask again
    pub call_timeout: Duration,
    /// Operations running at once; past it the home answers busy
    pub max_in_flight: usize,
    /// Answers kept for repeats, by count and by bytes
    pub dedup_entries: usize,
    /// The byte half of that limit.
    pub dedup_bytes: usize,
}

impl Default for HomeConfig {
    fn default() -> Self {
        Self {
            call_timeout: Duration::from_secs(20),
            max_in_flight: 64,
            dedup_entries: 1024,
            dedup_bytes: 8 << 20,
        }
    }
}

enum Slot {
    Running {
        since: Instant,
        digest: [u8; 32],
    },
    Done {
        digest: [u8; 32],
        body: Value,
        bytes: usize,
    },
}

enum Begin {
    Fresh,
    Running,
    Done(Value),
    /// The key was used for another request
    Reused,
}

#[derive(Default)]
struct Dedup {
    slots: HashMap<(NodeId, RequestKey), Slot>,
    done: VecDeque<(NodeId, RequestKey)>,
    bytes: usize,
}

impl Dedup {
    fn begin(&mut self, id: &(NodeId, RequestKey), digest: [u8; 32], stale: Duration) -> Begin {
        match self.slots.get(id) {
            Some(Slot::Done { digest: have, body, .. }) => {
                if *have == digest {
                    Begin::Done(body.clone())
                } else {
                    Begin::Reused
                }
            }
            Some(Slot::Running { since, digest: have }) if since.elapsed() < stale => {
                if *have == digest {
                    Begin::Running
                } else {
                    Begin::Reused
                }
            }
            // Nothing, or a request that never finished (its module hung): run it afresh
            _ => {
                self.slots.insert(
                    id.clone(),
                    Slot::Running {
                        since: Instant::now(),
                        digest,
                    },
                );
                Begin::Fresh
            }
        }
    }

    fn finish(&mut self, id: &(NodeId, RequestKey), result: Option<(Value, [u8; 32])>, entries: usize, cap: usize) {
        match result {
            None => {
                if matches!(self.slots.get(id), Some(Slot::Running { .. })) {
                    self.slots.remove(id);
                }
            }
            Some((body, digest)) => {
                let bytes = serde_json::to_vec(&body).map_or(0, |b| b.len()) + 128;
                self.slots.insert(id.clone(), Slot::Done { digest, body, bytes });
                self.done.push_back(id.clone());
                self.bytes += bytes;
                while self.done.len() > entries || self.bytes > cap {
                    let Some(old) = self.done.pop_front() else { break };
                    if let Some(Slot::Done { bytes, .. }) = self.slots.remove(&old) {
                        self.bytes = self.bytes.saturating_sub(bytes);
                    }
                }
            }
        }
    }
}

struct Inner {
    me: NodeIdentity,
    tickets: Arc<Tickets>,
    router: Arc<HomeRouter>,
    locator: Arc<dyn HomeLocator>,
    peers: BTreeSet<NodeId>,
    config: HomeConfig,
    dedup: Arc<Mutex<Dedup>>,
    permits: Arc<Semaphore>,
}

/// The answering side of `Service::Home`.
#[derive(Clone)]
pub struct HomeService {
    inner: Arc<Inner>,
}

fn digest_of(call: &HomeCall) -> [u8; 32] {
    let mut hash = Sha256::new();
    let account = call.account.to_string();
    let parts: [&[u8]; 3] = [
        call.module.as_str().as_bytes(),
        call.operation.as_str().as_bytes(),
        account.as_bytes(),
    ];
    for part in parts {
        hash.update(part);
        hash.update([0]);
    }
    hash.update(serde_json::to_vec(call.body.value()).unwrap_or_default());
    hash.finalize().into()
}

impl HomeService {
    /// `peers` are the nodes this one lists: the only audiences it makes tickets for besides itself.
    pub fn new(
        me: NodeIdentity,
        tickets: Arc<Tickets>,
        router: Arc<HomeRouter>,
        locator: Arc<dyn HomeLocator>,
        peers: impl IntoIterator<Item = NodeId>,
        config: HomeConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                permits: Arc::new(Semaphore::new(config.max_in_flight)),
                me,
                tickets,
                router,
                locator,
                peers: peers.into_iter().collect(),
                config,
                dedup: Arc::new(Mutex::new(Dedup::default())),
            }),
        }
    }

    /// The tickets this home makes and checks.
    pub fn tickets(&self) -> &Arc<Tickets> {
        &self.inner.tickets
    }

    /// The modules answering here.
    pub fn router(&self) -> &Arc<HomeRouter> {
        &self.inner.router
    }

    /// Answers one request from `from`, a peer as the link authenticated it.
    pub async fn handle(&self, from: &NodeId, call: HomeRequestCall) -> HomeReply {
        match self.inner.answer(from, call).await {
            Ok(reply) => reply,
            Err(refusal) => HomeReply::Refused(refusal),
        }
    }
}

impl Inner {
    /// A ticket this node made for `from`, still good, optionally for `game`.
    fn own_ticket(
        &self,
        from: &NodeId,
        ticket: &SessionTicket,
        game: Option<&GameVersion>,
    ) -> Result<Claims, HomeRefusal> {
        self.tickets
            .verify(
                ticket,
                &Expect {
                    issuer: &self.me.node,
                    audience: from,
                    game,
                    account: None,
                },
            )
            .map_err(HomeRefusal::Ticket)
    }

    async fn answer(&self, from: &NodeId, call: HomeRequestCall) -> Result<HomeReply, HomeRefusal> {
        match call {
            HomeRequestCall::PublicKeys => Ok(HomeReply::PublicKeys {
                node: self.me.node.clone(),
                keys: self.tickets.public_keys(),
            }),
            HomeRequestCall::Check { ticket } => {
                let claims = self.own_ticket(from, &ticket, None)?;
                Ok(HomeReply::Session(SessionInfo {
                    account: claims.sub,
                    expires_at: claims.exp,
                    since: claims.since,
                }))
            }
            HomeRequestCall::Exchange { ticket, audience } => {
                let claims = self.own_ticket(from, &ticket, None)?;
                if audience != self.me.node && !self.peers.contains(&audience) {
                    return Err(HomeRefusal::BadRequest);
                }
                // The login's age carries over, so a session can be handed on and renewed but not forever
                let renewed = self
                    .tickets
                    .issue(&claims.sub, &audience, &claims.game, Some(claims.since))
                    .map_err(HomeRefusal::Ticket)?;
                Ok(HomeReply::Ticket(renewed))
            }
            HomeRequestCall::Run(call) => self.run(from, *call).await,
        }
    }

    async fn run(&self, from: &NodeId, call: HomeCall) -> Result<HomeReply, HomeRefusal> {
        if call.account != call.ticket.account {
            return Err(HomeRefusal::BadRequest);
        }
        let backend = self.router.get(&call.module).ok_or(HomeRefusal::NoModule)?;
        let claims = self.own_ticket(from, &call.ticket, Some(backend.game()))?;
        if claims.sub != call.account {
            return Err(HomeRefusal::Ticket(TicketError::WrongAccount));
        }
        match self.locator.home_of(&call.account).await {
            Ok(Some(home)) if home == self.me.node => {}
            Ok(_) => return Err(HomeRefusal::NotHome),
            Err(error) => {
                tracing::warn!(peer = %from, %error, "the directory could not place an account for a home call");
                return Err(HomeRefusal::Unavailable);
            }
        }
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| HomeRefusal::Busy)?;
        let id = (from.clone(), call.key.clone());
        let digest = digest_of(&call);
        let begin =
            self.dedup
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .begin(&id, digest, self.config.call_timeout * 5);
        match begin {
            Begin::Done(body) => {
                return HomeBody::new(body)
                    .map(|body| HomeReply::Done { body })
                    .map_err(|_| HomeRefusal::Unavailable);
            }
            Begin::Running => return Err(HomeRefusal::InFlight),
            Begin::Reused => return Err(HomeRefusal::BadRequest),
            Begin::Fresh => {}
        }
        tracing::debug!(
            peer = %from,
            module = %call.module,
            operation = %call.operation,
            account = %call.account,
            "running an operation for another node"
        );
        let invocation = HomeInvocation {
            from: from.clone(),
            account: call.account.clone(),
            actor: call.actor.actor().clone(),
            operation: call.operation.clone(),
            body: call.body.into_value(),
        };
        let kept_in = Arc::clone(&self.dedup);
        let (entries, cap) = (self.config.dedup_entries, self.config.dedup_bytes);
        // Its own task: a request whose link drops, or that outwaits its caller, still finishes and is kept
        let task = tokio::spawn(async move {
            let _permit = permit;
            let result = backend.run(invocation).await;
            let kept = match &result {
                Ok(body) => Some((body.clone(), digest)),
                Err(_) => None,
            };
            kept_in
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .finish(&id, kept, entries, cap);
            result
        });
        let result = match tokio::time::timeout(self.config.call_timeout, task).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                tracing::error!(peer = %from, %error, "a module's home operation panicked");
                return Err(HomeRefusal::Unavailable);
            }
            Err(_) => return Err(HomeRefusal::InFlight),
        };
        match result {
            Ok(body) => match HomeBody::new(body) {
                Ok(body) => Ok(HomeReply::Done { body }),
                Err(error) => {
                    tracing::error!(peer = %from, %error, "a module's home answer can't cross a node link");
                    Err(HomeRefusal::TooLarge)
                }
            },
            Err(error) => Err(HomeRefusal::Failed {
                kind: error.kind,
                message: super::protocol::CleanText::lossy(&error.message),
            }),
        }
    }
}

impl NodeService for HomeService {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if service != Service::Home {
                return Err(LinkError::NoService {
                    node: self.inner.me.node.clone(),
                    service,
                });
            }
            let reply = match decode_request(&body) {
                Ok(request) => self.handle(from, request.call).await,
                Err(DecodeError::Version(_)) => HomeReply::Refused(HomeRefusal::UnsupportedVersion {
                    supported: super::protocol::HOME_VERSION,
                }),
                Err(DecodeError::TooLarge) => HomeReply::Refused(HomeRefusal::TooLarge),
                Err(_) => HomeReply::Refused(HomeRefusal::Malformed),
            };
            encode_response(&HomeResponse::new(reply)).map_err(|_| LinkError::TooLarge)
        })
    }
}
