//! The asking side of the home API (DN5): [`NetworkedHome`] is the [`HomeApi`] of a node in a network. It
//! finds an account's home in the directory, makes tickets for accounts homed here, checks tickets as the
//! node they are for, and runs a module's operations on accounts homed elsewhere over `Service::Home`.
//!
//! A node acts for a foreign account only with a ticket its home made for it. Admission (`admit`) is where
//! one arrives: a player turns up at this node with a ticket, this node checks it offline against the home's
//! public key and keeps it, renewing it with the home before it expires for as long as the node needs the
//! player (a hunt), and the node's later operations on that account present it. A node with no peers has no
//! use for any of this and keeps [`module_api::network::StandaloneHome`].

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use module_api::network::{GlobalId, HomeApi, HomeError, IdKind, NodeId, Region, SessionTicket};
use module_api::{BoxFuture, GameVersion, HomeForward, NodeIdentity};
use serde_json::Value;

use super::protocol::{
    DecodeError, HomeBody, HomeCall, HomeRefusal, HomeReply, HomeRequest, HomeRequestCall, RequestKey, SessionInfo,
    WireActor, decode_response, encode_request,
};
use super::service::HomeLocator;
use super::ticket::{Expect, TicketError, Tickets};
use crate::link::{LinkError, PeerLink, Service};

/// Sessions a node holds tickets for at once.
pub const MAX_HELD: usize = 4096;

#[derive(Debug, Clone)]
pub struct NetworkedConfig {
    /// The game version this network serves (DD16: 1.14.7 at first): tickets made here are for it
    pub game: GameVersion,
    /// Times a request is sent when the home can't be reached or is busy, with the same key
    pub attempts: u32,
    pub retry_after: Duration,
    /// A held ticket with less than this left is renewed before it is used
    pub renew_within: Duration,
    /// The least time between two fetches of one node's keys
    pub refetch_keys_after: Duration,
}

impl NetworkedConfig {
    pub fn new(game: GameVersion) -> Self {
        Self {
            game,
            attempts: 3,
            retry_after: Duration::from_millis(200),
            renew_within: Duration::from_secs(60),
            refetch_keys_after: Duration::from_secs(10),
        }
    }
}

#[derive(Clone)]
struct Held {
    ticket: SessionTicket,
    exp: i64,
}

pub struct NetworkedHome {
    me: NodeIdentity,
    regions: BTreeMap<NodeId, Region>,
    link: Arc<dyn PeerLink>,
    locator: Arc<dyn HomeLocator>,
    tickets: Arc<Tickets>,
    config: NetworkedConfig,
    held: Mutex<HashMap<GlobalId, Held>>,
    fetched: Mutex<HashMap<NodeId, i64>>,
}

fn denied(error: impl std::fmt::Display) -> HomeError {
    HomeError::Denied(error.to_string())
}

fn ticket_denied(error: TicketError) -> HomeError {
    denied(error)
}

fn unreachable(error: impl std::fmt::Display) -> HomeError {
    HomeError::Unreachable(error.to_string())
}

/// Why a call to a home failed: it refused, or the call could not be made or understood.
enum Failure {
    Refused(HomeRefusal),
    Error(HomeError),
}

impl Failure {
    /// The home no longer stands behind the ticket that was sent
    fn ends_ticket(&self) -> bool {
        matches!(self, Self::Refused(HomeRefusal::Ticket(_)))
    }
}

impl From<Failure> for HomeError {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Refused(refusal) => from_refusal(refusal),
            Failure::Error(error) => error,
        }
    }
}

impl From<HomeError> for Failure {
    fn from(error: HomeError) -> Self {
        Self::Error(error)
    }
}

/// Why a home refused, as a module sees it.
fn from_refusal(refusal: HomeRefusal) -> HomeError {
    match refusal {
        HomeRefusal::Failed { kind, message } => {
            HomeError::Module(module_api::ModuleError::new(kind, message.as_str()))
        }
        HomeRefusal::InFlight | HomeRefusal::Busy | HomeRefusal::Unavailable => unreachable(refusal),
        other => denied(other),
    }
}

impl NetworkedHome {
    /// `peers` are the nodes of `[[network.nodes]]` with their regions, for answering where an account is homed.
    pub fn new(
        me: NodeIdentity,
        peers: impl IntoIterator<Item = (NodeId, Region)>,
        link: Arc<dyn PeerLink>,
        locator: Arc<dyn HomeLocator>,
        tickets: Arc<Tickets>,
        config: NetworkedConfig,
    ) -> Self {
        let mut regions: BTreeMap<NodeId, Region> = peers.into_iter().collect();
        regions.insert(me.node.clone(), me.region.clone());
        Self {
            me,
            regions,
            link,
            locator,
            tickets,
            config,
            held: Mutex::new(HashMap::new()),
            fetched: Mutex::new(HashMap::new()),
        }
    }

    pub fn tickets(&self) -> &Arc<Tickets> {
        &self.tickets
    }

    /// The session tickets held right now, for tests and `status`.
    pub fn held_len(&self) -> usize {
        self.held.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    async fn home_node(&self, account: &GlobalId) -> Result<NodeId, HomeError> {
        if account.kind() != IdKind::Account {
            return Err(HomeError::UnknownAccount);
        }
        match self.locator.home_of(account).await {
            Ok(Some(node)) => Ok(node),
            Ok(None) => Err(HomeError::UnknownAccount),
            Err(error) => Err(unreachable(error)),
        }
    }

    /// One request to `node`'s home service, and its answer as a refusal or the reply.
    async fn call(&self, node: &NodeId, call: HomeRequestCall) -> Result<HomeReply, Failure> {
        let body = encode_request(&HomeRequest::new(call)).map_err(denied)?;
        let answer = self
            .link
            .call(node, Service::Home, body)
            .await
            .map_err(|error| match error {
                LinkError::TooLarge => denied(error),
                other => unreachable(other),
            })?;
        let response = decode_response(&answer).map_err(|error| match error {
            DecodeError::Version(version) => unreachable(format!("{node} speaks home protocol {version}")),
            _ => unreachable(format!("{node} broke the home protocol")),
        })?;
        match response.reply {
            HomeReply::Refused(refusal) => Err(Failure::Refused(refusal)),
            reply => Ok(reply),
        }
    }

    /// Asks `node` for its public keys and keeps them: what a ticket from it is checked against. A node is
    /// asked at most once in a while, so a ticket with a key nobody has can't make this node ask forever.
    async fn fetch_keys(&self, node: &NodeId) -> Result<(), HomeError> {
        let now = self.tickets.clock().now_ms();
        {
            let mut fetched = self.fetched.lock().unwrap_or_else(PoisonError::into_inner);
            let wait = i64::try_from(self.config.refetch_keys_after.as_millis()).unwrap_or(i64::MAX);
            if fetched.get(node).is_some_and(|at| now.saturating_sub(*at) < wait) {
                return Err(denied(TicketError::UnknownKey));
            }
            fetched.insert(node.clone(), now);
        }
        match self.call(node, HomeRequestCall::PublicKeys).await? {
            HomeReply::PublicKeys { node: told, keys } if &told == node => {
                if self.tickets.learn(node, keys) {
                    Ok(())
                } else {
                    Err(denied(TicketError::UnknownKey))
                }
            }
            _ => Err(unreachable(format!(
                "{node} answered a key request with something else"
            ))),
        }
    }

    fn keep(&self, ticket: SessionTicket, exp: i64) {
        let now = self.tickets.clock().now_ms();
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if held.len() >= MAX_HELD {
            held.retain(|_, h| h.exp > now);
        }
        if held.len() >= MAX_HELD && !held.contains_key(&ticket.account) {
            // Drop the one closest to expiry rather than refuse a player
            if let Some(oldest) = held.iter().min_by_key(|(_, h)| h.exp).map(|(k, _)| k.clone()) {
                held.remove(&oldest);
            }
        }
        held.insert(ticket.account.clone(), Held { ticket, exp });
    }

    fn drop_held(&self, account: &GlobalId) {
        self.held.lock().unwrap_or_else(PoisonError::into_inner).remove(account);
    }

    /// Whether this node holds a session for `account` (admitted here and not expired).
    pub fn holds(&self, account: &GlobalId) -> bool {
        let now = self.tickets.clock().now_ms();
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(account)
            .is_some_and(|h| h.exp > now)
    }

    /// The ticket this node holds for `account`, renewed with the home if it is about to expire.
    async fn usable_ticket(&self, account: &GlobalId, home: &NodeId) -> Result<SessionTicket, HomeError> {
        let held = self
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(account)
            .cloned()
            .ok_or_else(|| denied("this node holds no session for that account"))?;
        let now = self.tickets.clock().now_ms();
        let soon = i64::try_from(self.config.renew_within.as_millis()).unwrap_or(i64::MAX);
        if held.exp.saturating_sub(now) > soon {
            return Ok(held.ticket);
        }
        let asked = self
            .call(
                home,
                HomeRequestCall::Exchange {
                    ticket: held.ticket.clone(),
                    audience: self.me.node.clone(),
                },
            )
            .await;
        match asked {
            Ok(HomeReply::Ticket(ticket)) => {
                let claims = self
                    .tickets
                    .verify(
                        &ticket,
                        &Expect {
                            issuer: home,
                            audience: &self.me.node,
                            game: Some(&self.config.game),
                            account: Some(account),
                        },
                    )
                    .map_err(ticket_denied)?;
                self.keep(ticket.clone(), claims.exp);
                Ok(ticket)
            }
            Ok(_) => Err(unreachable(format!("{home} answered a renewal with something else"))),
            // Whatever kept the home from renewing it, a ticket that is still good is still good
            Err(_) if held.exp > now => Ok(held.ticket),
            Err(failure) => {
                if failure.ends_ticket() {
                    self.drop_held(account);
                }
                Err(failure.into())
            }
        }
    }

    /// Asks the home whether it still stands behind the session this node holds for `account`: the check that
    /// sees an ended session, which admission (offline) can't.
    pub async fn check_session(&self, account: &GlobalId) -> Result<SessionInfo, HomeError> {
        let home = self.home_node(account).await?;
        if home == self.me.node {
            return Err(denied("this node is the account's home"));
        }
        let ticket = self.usable_ticket(account, &home).await?;
        match self.call(&home, HomeRequestCall::Check { ticket }).await {
            Ok(HomeReply::Session(info)) => Ok(info),
            Ok(_) => Err(unreachable(format!(
                "{home} answered a session check with something else"
            ))),
            Err(failure) => {
                if failure.ends_ticket() {
                    self.drop_held(account);
                }
                Err(failure.into())
            }
        }
    }

    /// A ticket for `audience` for the network's game, which is `session_ticket`'s.
    pub async fn ticket_for(&self, account: &GlobalId, audience: &NodeId) -> Result<SessionTicket, HomeError> {
        let home = self.home_node(account).await?;
        if home == self.me.node {
            return self
                .tickets
                .issue(account, audience, &self.config.game, None)
                .map_err(ticket_denied);
        }
        let ticket = self.usable_ticket(account, &home).await?;
        match self
            .call(
                &home,
                HomeRequestCall::Exchange {
                    ticket,
                    audience: audience.clone(),
                },
            )
            .await?
        {
            HomeReply::Ticket(made) => {
                self.tickets
                    .verify(
                        &made,
                        &Expect {
                            issuer: &home,
                            audience,
                            game: Some(&self.config.game),
                            account: Some(account),
                        },
                    )
                    .map_err(ticket_denied)?;
                Ok(made)
            }
            _ => Err(unreachable(format!("{home} answered an exchange with something else"))),
        }
    }

    async fn run_forward(&self, forward: &HomeForward) -> Result<Value, HomeError> {
        let home = self.home_node(&forward.account).await?;
        if home == self.me.node {
            return Err(denied("this node is the account's home"));
        }
        let key = match &forward.key {
            Some(key) => RequestKey::new(key.clone()).map_err(denied)?,
            None => RequestKey::generate().map_err(denied)?,
        };
        let actor = WireActor::new(forward.actor.clone()).map_err(denied)?;
        let body = HomeBody::new(forward.body.clone()).map_err(denied)?;
        let mut last = unreachable("no attempt was made");
        for attempt in 0..self.config.attempts.max(1) {
            if attempt > 0 {
                tokio::time::sleep(self.config.retry_after * attempt).await;
            }
            let ticket = self.usable_ticket(&forward.account, &home).await?;
            let call = HomeCall {
                ticket,
                module: forward.module.clone(),
                operation: forward.operation.clone(),
                account: forward.account.clone(),
                actor: actor.clone(),
                key: key.clone(),
                body: body.clone(),
            };
            match self.call(&home, HomeRequestCall::Run(Box::new(call))).await {
                Ok(HomeReply::Done { body }) => return Ok(body.into_value()),
                Ok(_) => return Err(unreachable(format!("{home} answered an operation with something else"))),
                // The home couldn't be reached, or is busy with this very request: ask again, same key
                Err(Failure::Error(error @ HomeError::Unreachable(_))) => last = error,
                Err(Failure::Refused(HomeRefusal::InFlight | HomeRefusal::Busy | HomeRefusal::Unavailable)) => {
                    last = unreachable("the home is busy");
                }
                Err(failure) => {
                    if failure.ends_ticket() {
                        self.drop_held(&forward.account);
                    }
                    return Err(failure.into());
                }
            }
        }
        Err(last)
    }
}

impl HomeApi for NetworkedHome {
    fn resolve_home<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<NodeIdentity, HomeError>> {
        Box::pin(async move {
            let node = self.home_node(account).await?;
            let region = self
                .regions
                .get(&node)
                .ok_or_else(|| unreachable(format!("{node} is not a node this one lists")))?;
            Ok(NodeIdentity {
                node,
                region: region.clone(),
            })
        })
    }

    fn session_ticket<'a>(
        &'a self,
        account: &'a GlobalId,
        audience: &'a NodeId,
    ) -> BoxFuture<'a, Result<SessionTicket, HomeError>> {
        Box::pin(self.ticket_for(account, audience))
    }

    fn forward<'a>(&'a self, call: &'a HomeForward) -> BoxFuture<'a, Result<Value, HomeError>> {
        Box::pin(self.run_forward(call))
    }

    fn admit<'a>(
        &'a self,
        ticket: &'a SessionTicket,
        game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<GlobalId, HomeError>> {
        Box::pin(async move {
            // Unverified: only to find where to look. Nothing below trusts it until the signature is checked
            let claims = self.tickets.peek(ticket).map_err(ticket_denied)?;
            if claims.aud != self.me.node {
                return Err(ticket_denied(TicketError::WrongAudience));
            }
            let home = self.home_node(&claims.sub).await?;
            if home != claims.iss {
                return Err(ticket_denied(TicketError::WrongIssuer));
            }
            if !self.tickets.knows_key(&home, &claims.kid) {
                self.fetch_keys(&home).await?;
            }
            let claims = self
                .tickets
                .redeem(
                    ticket,
                    &Expect {
                        issuer: &home,
                        audience: &self.me.node,
                        game: Some(game),
                        account: None,
                    },
                )
                .map_err(ticket_denied)?;
            // A ticket made here for a player homed here is checked, not kept: there is no one to forward to
            if home != self.me.node {
                self.keep(ticket.clone(), claims.exp);
            }
            Ok(claims.sub)
        })
    }
}
