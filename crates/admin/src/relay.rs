//! Admin actions across the network (DN11, DD16, DD23): resolving a player's home through the
//! directory, fanning out `kick`, `ban`, `timeout`, `unban`, `liftTimeout` and `revokeKey` over
//! `Service::Admin` to the home and every reachable peer, revoking session tickets everywhere on a
//! ban/timeout/kick, and persisting actions for down peers in `admin_relay_queue` via `Database::write`
//! so they are visible as `"pending on node <peer>"` and retried idempotently with the same request
//! key when the link recovers.

use std::fmt;
use std::sync::Arc;

use async_graphql::SimpleObject;
use diesel::prelude::*;
use module_api::network::{GlobalId, NodeId, OperationName};
use network_plugin::directory::{AccountRecord, Directory};
use network_plugin::home::Tickets;
use network_plugin::home::protocol::{CleanText, HomeBody, RequestKey};
use network_plugin::link::{PeerLink, PeerRoster, Service};
use network_plugin::relay::{
    AdminAuthority, AdminRelayConfig, AdminRelayRefusal, AdminRelayReply, AdminRelayRequest, AdminRelayService,
    RelayAction, decode_response, encode_request,
};
use serde_json::{Value, json};
use store::{Actor, AnyConnection, Database};

use crate::input;
use crate::log;

pub use network_plugin::relay::operations;

/// Maximum queued actions kept for a single unreachable peer.
pub const MAX_QUEUE_PER_PEER: i64 = 256;
/// Maximum queued actions kept across all peers on this node.
pub const MAX_QUEUE_TOTAL: i64 = 2048;

diesel::table! {
    // 15 columns (within Diesel's 16-column default table width)
    /// Admin actions waiting for a peer that was down (DN11).
    admin_relay_queue (id) {
        /// `<peer>:<request key>`: one entry per action and peer.
        id -> Text,
        /// The peer it waits for.
        peer -> Text,
        /// The relay's idempotency key: the peer applies it once however often it is sent.
        request_key -> Text,
        /// The relay operation, such as `admin.ban`.
        operation -> Text,
        /// The account it acts on, when the directory resolved one.
        account -> Nullable<Text>,
        /// 1 when the action ends the account's session tickets.
        revoke_tickets -> Integer,
        /// The admin key that acted: `owner` or a key id.
        actor_key -> Text,
        /// The Discord user it was for, or `host`.
        operator -> Text,
        /// The approving key, for a guarded action (DH6).
        approved_by -> Nullable<Text>,
        /// The player's name as given.
        target -> Nullable<Text>,
        /// The player's game version.
        game -> Nullable<Text>,
        /// The action's payload, as JSON.
        body -> Text,
        /// Unix milliseconds.
        created_at -> BigInt,
        /// Sends tried so far.
        attempts -> Integer,
        /// The last send's error.
        last_error -> Nullable<Text>,
    }
}

#[derive(Queryable)]
struct QueueRow {
    id: String,
    peer: String,
    request_key: String,
    operation: String,
    account: Option<String>,
    revoke_tickets: i32,
    actor_key: String,
    operator: String,
    approved_by: Option<String>,
    target: Option<String>,
    game: Option<String>,
    body: String,
    created_at: i64,
    attempts: i32,
    last_error: Option<String>,
}

/// An admin action queued for a peer that was unreachable when the action ran (DN11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPeerAction {
    /// `<peer>:<request key>`: one entry per action and peer.
    pub id: String,
    /// The peer it waits for.
    pub peer: NodeId,
    /// The relay's idempotency key.
    pub request_key: String,
    /// The relay operation, such as `admin.ban`.
    pub operation: String,
    /// The account it acts on, when the directory resolved one.
    pub account: Option<GlobalId>,
    /// Whether the action ends the account's session tickets.
    pub revoke_tickets: bool,
    /// The admin key that acted: `owner` or a key id.
    pub actor_key: String,
    /// The Discord user it was for, or `host`.
    pub operator: String,
    /// The approving key, for a guarded action (DH6).
    pub approved_by: Option<String>,
    /// The player's name as given.
    pub target: Option<String>,
    /// The player's game version.
    pub game: Option<String>,
    /// The action's payload.
    pub body: Value,
    /// Unix milliseconds.
    pub created_at: i64,
    /// Sends tried so far.
    pub attempts: i32,
    /// The last send's error.
    pub last_error: Option<String>,
    /// `"pending on node <peer>"`
    pub status: String,
}

impl fmt::Display for PendingPeerAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.status)
    }
}

/// GraphQL representation of a queued admin action waiting for a down peer (DN11).
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct PendingRelay {
    /// The queue entry's id.
    pub id: String,
    /// The peer it waits for.
    pub node: String,
    /// The relay's idempotency key.
    pub request_key: String,
    /// The relay operation, such as `admin.ban`.
    pub operation: String,
    /// The player's name as given.
    pub target: Option<String>,
    /// The player's game version.
    pub game: Option<String>,
    /// The admin key that acted: `owner` or a key id.
    pub actor_key: String,
    /// When it was queued (RFC 3339).
    pub created_at: String,
    /// Sends tried so far.
    pub attempts: i32,
    /// The last send's error.
    pub last_error: Option<String>,
    /// `"pending on node <node>"`
    pub status: String,
}

impl From<PendingPeerAction> for PendingRelay {
    fn from(item: PendingPeerAction) -> Self {
        Self {
            id: item.id,
            node: item.peer.to_string(),
            request_key: item.request_key,
            operation: item.operation,
            target: item.target,
            game: item.game,
            actor_key: item.actor_key,
            created_at: input::iso(item.created_at),
            attempts: item.attempts,
            last_error: item.last_error,
            status: item.status,
        }
    }
}

impl QueueRow {
    fn into_pending(self) -> Option<PendingPeerAction> {
        let peer: NodeId = self.peer.parse().ok()?;
        let account = match self.account {
            Some(text) => Some(text.parse().ok()?),
            None => None,
        };
        let body: Value = serde_json::from_str(&self.body).ok()?;
        let status = format!("pending on node {peer}");
        Some(PendingPeerAction {
            id: self.id,
            peer,
            request_key: self.request_key,
            operation: self.operation,
            account,
            revoke_tickets: self.revoke_tickets != 0,
            actor_key: self.actor_key,
            operator: self.operator,
            approved_by: self.approved_by,
            target: self.target,
            game: self.game,
            body,
            created_at: self.created_at,
            attempts: self.attempts,
            last_error: self.last_error,
            status,
        })
    }
}

/// The network handles the admin API uses when the server is part of a cluster (DN11).
pub trait AdminNetwork: Send + Sync {
    /// This node, when in a network.
    fn local_node(&self) -> Option<NodeId>;
    /// The link to peers.
    fn peer_link(&self) -> Option<Arc<dyn PeerLink>>;
    /// Which peers are listed and up.
    fn peer_roster(&self) -> Option<Arc<dyn PeerRoster>>;
    /// The directory, to find an account's home.
    fn directory(&self) -> Option<Arc<Directory>>;
    /// The session tickets to revoke for actions that end them.
    fn tickets(&self) -> Option<Arc<Tickets>>;
}

/// A standalone server with no peers: behaves byte-for-byte as before DN11.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoNetwork;

impl AdminNetwork for NoNetwork {
    fn local_node(&self) -> Option<NodeId> {
        None
    }

    fn peer_link(&self) -> Option<Arc<dyn PeerLink>> {
        None
    }

    fn peer_roster(&self) -> Option<Arc<dyn PeerRoster>> {
        None
    }

    fn directory(&self) -> Option<Arc<Directory>> {
        None
    }

    fn tickets(&self) -> Option<Arc<Tickets>> {
        None
    }
}

/// A fixed network configuration for tests and direct composition.
pub struct ConnectedNetwork {
    /// This node.
    pub node: NodeId,
    /// The link to peers.
    pub link: Arc<dyn PeerLink>,
    /// Which peers are listed and up.
    pub roster: Arc<dyn PeerRoster>,
    /// The directory, if this node keeps one.
    pub directory: Option<Arc<Directory>>,
    /// The session tickets, if this node makes them.
    pub tickets: Option<Arc<Tickets>>,
}

impl ConnectedNetwork {
    /// `link` is both the link and the roster.
    pub fn new<L>(
        node: NodeId,
        link: Arc<L>,
        directory: Option<Arc<Directory>>,
        tickets: Option<Arc<Tickets>>,
    ) -> Arc<Self>
    where
        L: PeerLink + PeerRoster + 'static,
    {
        let as_link: Arc<dyn PeerLink> = link.clone();
        let as_roster: Arc<dyn PeerRoster> = link;
        Arc::new(Self {
            node,
            link: as_link,
            roster: as_roster,
            directory,
            tickets,
        })
    }
}

impl AdminNetwork for ConnectedNetwork {
    fn local_node(&self) -> Option<NodeId> {
        Some(self.node.clone())
    }

    fn peer_link(&self) -> Option<Arc<dyn PeerLink>> {
        Some(self.link.clone())
    }

    fn peer_roster(&self) -> Option<Arc<dyn PeerRoster>> {
        Some(self.roster.clone())
    }

    fn directory(&self) -> Option<Arc<Directory>> {
        self.directory.clone()
    }

    fn tickets(&self) -> Option<Arc<Tickets>> {
        self.tickets.clone()
    }
}

struct DynamicRoster(Arc<dyn AdminNetwork>);

impl PeerRoster for DynamicRoster {
    fn peers(&self) -> Vec<NodeId> {
        self.0.peer_roster().map_or_else(Vec::new, |r| r.peers())
    }

    fn is_up(&self, node: &NodeId) -> bool {
        self.0.peer_roster().is_some_and(|r| r.is_up(node))
    }
}

/// Parameters for an outbound admin action to be sent to a player's home and fanned out to peers.
#[derive(Debug, Clone)]
pub struct OutboundAction {
    /// The idempotency key: resending runs the action at most once.
    pub key: RequestKey,
    /// The relay operation, such as [`operations::BAN`].
    pub operation: &'static str,
    /// The account it acts on, when the directory resolved one.
    pub account: Option<GlobalId>,
    /// Whether the action ends the account's session tickets.
    pub revoke_tickets: bool,
    /// The admin key that acted: `owner` or a key id.
    pub actor_key: String,
    /// The Discord user it was for, or `host`.
    pub operator: String,
    /// The approving key, for a guarded action (DH6).
    pub approved_by: Option<String>,
    /// The player's name as given.
    pub target: Option<String>,
    /// The player's game version.
    pub game: Option<String>,
    /// The action's payload.
    pub body: Value,
}

impl OutboundAction {
    /// An action with a fresh idempotency key; errors only when the system's random source fails.
    #[allow(
        clippy::too_many_arguments,
        reason = "each argument is one field of the relayed action"
    )]
    pub fn new(
        operation: &'static str,
        account: Option<GlobalId>,
        revoke_tickets: bool,
        actor_key: impl Into<String>,
        operator: impl Into<String>,
        approved_by: Option<String>,
        target: Option<String>,
        game: Option<String>,
        body: Value,
    ) -> Result<Self, String> {
        let key = RequestKey::generate().map_err(|e| e.to_string())?;
        Ok(Self {
            key,
            operation,
            account,
            revoke_tickets,
            actor_key: actor_key.into(),
            operator: operator.into(),
            approved_by,
            target,
            game,
            body,
        })
    }

    /// The action as it crosses the link; errors on a field the wire refuses, such as one shaped like a key.
    pub fn to_wire(&self) -> Result<RelayAction, String> {
        let operation: OperationName = self
            .operation
            .parse()
            .map_err(|e: module_api::network::OperationNameError| e.to_string())?;
        let actor_key = CleanText::new(self.actor_key.clone()).map_err(|e| e.to_string())?;
        let operator = CleanText::new(self.operator.clone()).map_err(|e| e.to_string())?;
        let approved_by = self
            .approved_by
            .as_ref()
            .map(|by| CleanText::new(by.clone()).map_err(|e| e.to_string()))
            .transpose()?;
        let body = HomeBody::new(self.body.clone()).map_err(|e| e.to_string())?;
        Ok(RelayAction {
            key: self.key.clone(),
            operation,
            account: self.account.clone(),
            revoke_tickets: self.revoke_tickets,
            actor_key,
            operator,
            approved_by,
            body,
        })
    }
}

/// Outcome of sending a [`RelayAction`] to one peer.
#[derive(Debug)]
pub enum PeerCallError {
    /// The peer was reached and refused the action (for example `PROTECTED_ACCOUNT` at the home).
    Refused {
        /// The peer's error code, if it gave one.
        code: Option<String>,
        /// What it said.
        message: String,
    },
    /// The peer was down, unreachable, or timed out: the action should be queued for retry.
    Unreachable(String),
}

/// Coordinates outbound fan-out, ticket revocation and the persistent retry queue for `Service::Admin`.
#[derive(Clone)]
pub struct AdminRelay {
    db: Database,
    network: Arc<dyn AdminNetwork>,
}

impl AdminRelay {
    /// A relay over `network`, queuing in `db`.
    pub fn new(db: Database, network: Arc<dyn AdminNetwork>) -> Self {
        Self { db, network }
    }

    /// A relay with no peers: every action stays local.
    pub fn standalone(db: Database) -> Self {
        Self::new(db, Arc::new(NoNetwork))
    }

    /// The network handles in use.
    pub fn network(&self) -> &Arc<dyn AdminNetwork> {
        &self.network
    }

    /// Whether this server is configured with network peers.
    pub fn in_network(&self) -> bool {
        self.network.peer_link().is_some()
            && self
                .network
                .peer_roster()
                .is_some_and(|roster| !roster.peers().is_empty())
    }

    /// This node, when in a network.
    pub fn local_node(&self) -> Option<NodeId> {
        self.network.local_node()
    }

    /// Builds a [`network_plugin::link::NodeService`] for `Service::Admin` backed by `authority` and this relay's network.
    pub fn service(&self, authority: Arc<dyn AdminAuthority>) -> AdminRelayService {
        let me = self
            .network
            .local_node()
            .unwrap_or_else(|| module_api::NodeIdentity::standalone().node);
        let peers = self.network.peer_roster().map_or_else(Vec::new, |r| r.peers());
        let roster: Arc<dyn PeerRoster> = Arc::new(DynamicRoster(self.network.clone()));
        AdminRelayService::new(
            me,
            peers,
            self.network.tickets(),
            authority,
            AdminRelayConfig::default(),
        )
        .with_roster(roster)
    }

    /// Looks up `username` in the network directory when one is attached.
    pub async fn find_in_directory(&self, username: &str) -> Option<AccountRecord> {
        let directory = self.network.directory()?;
        directory.find(username).await.ok().flatten()
    }

    /// Ends session tickets for `account` on this node immediately and persists the revocation
    /// row when a database is attached.
    pub async fn revoke_tickets(&self, account: &GlobalId) -> Result<(), String> {
        if let Some(tickets) = self.network.tickets()
            && let Err(error) = tickets.revoke(account).await
        {
            log::error(
                "could not persist ticket revocation",
                json!({
                    "account": account.to_string(),
                    "error": error.to_string(),
                }),
            );
            return Err(error.to_string());
        }
        Ok(())
    }

    /// Allows session tickets for `account` on this node again after an unban or lifted timeout
    /// and clears the persisted revocation row when a database is attached.
    pub async fn allow_tickets(&self, account: &GlobalId) -> Result<(), String> {
        if let Some(tickets) = self.network.tickets()
            && let Err(error) = tickets.allow(account).await
        {
            log::error(
                "could not clear ticket revocation",
                json!({
                    "account": account.to_string(),
                    "error": error.to_string(),
                }),
            );
            return Err(error.to_string());
        }
        Ok(())
    }

    /// Sends `action` to `peer` over `Service::Admin`.
    pub async fn call_peer(&self, peer: &NodeId, action: &RelayAction) -> Result<Value, PeerCallError> {
        let Some(roster) = self.network.peer_roster() else {
            return Err(PeerCallError::Unreachable("no peer roster configured".into()));
        };
        if !roster.is_up(peer) {
            return Err(PeerCallError::Unreachable(format!("{peer} is down")));
        }
        let Some(link) = self.network.peer_link() else {
            return Err(PeerCallError::Unreachable("no peer link configured".into()));
        };
        let bytes = encode_request(&AdminRelayRequest::new(action.clone())).map_err(|e| PeerCallError::Refused {
            code: None,
            message: e.to_string(),
        })?;
        let raw = link
            .call(peer, Service::Admin, bytes)
            .await
            .map_err(|e| PeerCallError::Unreachable(e.to_string()))?;
        let response = decode_response(&raw).map_err(|e| PeerCallError::Unreachable(e.to_string()))?;
        match response.reply {
            AdminRelayReply::Applied { body } => Ok(body.into_value()),
            AdminRelayReply::Refused { refusal } => match refusal {
                AdminRelayRefusal::InFlight | AdminRelayRefusal::Busy => {
                    Err(PeerCallError::Unreachable(refusal.to_string()))
                }
                AdminRelayRefusal::Failed { code, message } => Err(PeerCallError::Refused {
                    code: code.map(CleanText::into_string),
                    message: message.into_string(),
                }),
                other => Err(PeerCallError::Refused {
                    code: None,
                    message: other.to_string(),
                }),
            },
        }
    }

    /// Sends `spec` to every peer except `skip` (if `skip` was already called as the player's home).
    /// Peers that are down or fail with a link error get the action queued in `admin_relay_queue`
    /// through `Database::write`. Returns the list of peers where the action is now pending.
    pub async fn fan_out(&self, spec: &OutboundAction, skip: Option<&NodeId>, now_ms: i64) -> Vec<NodeId> {
        let Some(roster) = self.network.peer_roster() else {
            return Vec::new();
        };
        let peers: Vec<NodeId> = roster
            .peers()
            .into_iter()
            .filter(|peer| skip.is_none_or(|s| s != peer))
            .collect();
        if peers.is_empty() {
            return Vec::new();
        }
        let wire = match spec.to_wire() {
            Ok(wire) => wire,
            Err(error) => {
                log::error("could not encode admin relay action", json!({ "error": error }));
                return Vec::new();
            }
        };
        let mut pending = Vec::new();
        for peer in peers {
            match self.call_peer(&peer, &wire).await {
                Ok(_) => {}
                Err(PeerCallError::Refused { code, message }) => {
                    log::warn(
                        "peer refused relayed admin action",
                        json!({
                            "peer": peer.to_string(),
                            "operation": spec.operation,
                            "code": code,
                            "error": message,
                        }),
                    );
                }
                Err(PeerCallError::Unreachable(why)) => {
                    if let Err(error) = self.enqueue(&peer, spec, &why, now_ms).await {
                        log::error(
                            "could not queue admin action for down peer",
                            json!({
                                "peer": peer.to_string(),
                                "operation": spec.operation,
                                "error": error.to_string(),
                            }),
                        );
                    }
                    pending.push(peer);
                }
            }
        }
        pending
    }

    /// Persists `spec` in `admin_relay_queue` for `peer` via `Database::write`, bounded by
    /// [`MAX_QUEUE_PER_PEER`] and [`MAX_QUEUE_TOTAL`]. Idempotent for the same `(peer, request_key)`.
    pub async fn enqueue(
        &self,
        peer: &NodeId,
        spec: &OutboundAction,
        error: &str,
        now_ms: i64,
    ) -> Result<(), store::Error> {
        let row_id = format!("{peer}:{}", spec.key.as_str());
        let peer_str = peer.to_string();
        let request_key = spec.key.as_str().to_owned();
        let operation = spec.operation.to_owned();
        let account = spec.account.as_ref().map(ToString::to_string);
        let revoke_tickets = i32::from(spec.revoke_tickets);
        let actor_key = spec.actor_key.clone();
        let operator = spec.operator.clone();
        let approved_by = spec.approved_by.clone();
        let target = spec.target.clone();
        let game = spec.game.clone();
        let body = serde_json::to_string(&spec.body).unwrap_or_else(|_| "{}".into());
        let last_error = Some(error.chars().take(240).collect::<String>());
        let actor = Actor::Admin(actor_key.clone());

        self.db
            .write(actor, move |tx| {
                let existing: Option<String> = admin_relay_queue::table
                    .find(&row_id)
                    .select(admin_relay_queue::id)
                    .first(tx.conn())
                    .optional()?;
                if existing.is_some() {
                    diesel::update(admin_relay_queue::table.find(&row_id))
                        .set((
                            admin_relay_queue::attempts.eq(admin_relay_queue::attempts + 1),
                            admin_relay_queue::last_error.eq(last_error),
                        ))
                        .execute(tx.conn())?;
                    return Ok(());
                }

                // Enforce per-peer and total bounds by evicting the oldest row if full
                let peer_count: i64 = admin_relay_queue::table
                    .filter(admin_relay_queue::peer.eq(&peer_str))
                    .count()
                    .get_result(tx.conn())?;
                if peer_count >= MAX_QUEUE_PER_PEER
                    && let Some(oldest) = admin_relay_queue::table
                        .filter(admin_relay_queue::peer.eq(&peer_str))
                        .order((admin_relay_queue::created_at.asc(), admin_relay_queue::id.asc()))
                        .select(admin_relay_queue::id)
                        .first::<String>(tx.conn())
                        .optional()?
                {
                    diesel::delete(admin_relay_queue::table.find(oldest)).execute(tx.conn())?;
                }
                let total_count: i64 = admin_relay_queue::table.count().get_result(tx.conn())?;
                if total_count >= MAX_QUEUE_TOTAL
                    && let Some(oldest) = admin_relay_queue::table
                        .order((admin_relay_queue::created_at.asc(), admin_relay_queue::id.asc()))
                        .select(admin_relay_queue::id)
                        .first::<String>(tx.conn())
                        .optional()?
                {
                    diesel::delete(admin_relay_queue::table.find(oldest)).execute(tx.conn())?;
                }

                diesel::insert_into(admin_relay_queue::table)
                    .values((
                        admin_relay_queue::id.eq(row_id),
                        admin_relay_queue::peer.eq(peer_str),
                        admin_relay_queue::request_key.eq(request_key),
                        admin_relay_queue::operation.eq(operation),
                        admin_relay_queue::account.eq(account),
                        admin_relay_queue::revoke_tickets.eq(revoke_tickets),
                        admin_relay_queue::actor_key.eq(actor_key),
                        admin_relay_queue::operator.eq(operator),
                        admin_relay_queue::approved_by.eq(approved_by),
                        admin_relay_queue::target.eq(target),
                        admin_relay_queue::game.eq(game),
                        admin_relay_queue::body.eq(body),
                        admin_relay_queue::created_at.eq(now_ms),
                        admin_relay_queue::attempts.eq(1),
                        admin_relay_queue::last_error.eq(last_error),
                    ))
                    .execute(tx.conn())?;
                Ok(())
            })
            .await
    }

    /// Lists all queued actions waiting for unreachable peers, oldest first.
    pub async fn pending(&self) -> Result<Vec<PendingPeerAction>, store::Error> {
        let rows: Vec<QueueRow> = self
            .db
            .read(|conn: &mut AnyConnection| {
                Ok::<_, store::Error>(
                    admin_relay_queue::table
                        .order((admin_relay_queue::created_at.asc(), admin_relay_queue::id.asc()))
                        .load(conn)?,
                )
            })
            .await?;
        Ok(rows.into_iter().filter_map(QueueRow::into_pending).collect())
    }

    /// Retries queued actions for peers whose links are now up, using each row's original `request_key`.
    /// Returns the number of queued actions delivered and removed from the queue.
    pub async fn sweep(&self) -> usize {
        let Some(roster) = self.network.peer_roster() else {
            return 0;
        };
        let queued = match self.pending().await {
            Ok(queued) => queued,
            Err(error) => {
                log::error(
                    "could not read admin relay queue",
                    json!({ "error": error.to_string() }),
                );
                return 0;
            }
        };
        let mut delivered = 0;
        for item in queued {
            if !roster.is_up(&item.peer) {
                continue;
            }
            let Ok(key) = RequestKey::new(item.request_key.clone()) else {
                let _ = self.remove(&item.id).await;
                continue;
            };
            let Ok(operation) = item.operation.parse::<OperationName>() else {
                let _ = self.remove(&item.id).await;
                continue;
            };
            let Ok(actor_key) = CleanText::new(item.actor_key.clone()) else {
                let _ = self.remove(&item.id).await;
                continue;
            };
            let Ok(operator) = CleanText::new(item.operator.clone()) else {
                let _ = self.remove(&item.id).await;
                continue;
            };
            let approved_by = match &item.approved_by {
                Some(by) => CleanText::new(by.clone()).ok(),
                None => None,
            };
            let Ok(body) = HomeBody::new(item.body.clone()) else {
                let _ = self.remove(&item.id).await;
                continue;
            };
            let wire = RelayAction {
                key,
                operation,
                account: item.account.clone(),
                revoke_tickets: item.revoke_tickets,
                actor_key,
                operator,
                approved_by,
                body,
            };
            match self.call_peer(&item.peer, &wire).await {
                Ok(_) | Err(PeerCallError::Refused { .. }) => {
                    if self.remove(&item.id).await.is_ok() {
                        delivered += 1;
                    }
                }
                Err(PeerCallError::Unreachable(why)) => {
                    let _ = self.record_retry_failure(&item.id, &why).await;
                }
            }
        }
        delivered
    }

    async fn remove(&self, id: &str) -> Result<(), store::Error> {
        let id = id.to_owned();
        self.db
            .write(Actor::System("relay".into()), move |tx| {
                diesel::delete(admin_relay_queue::table.find(&id)).execute(tx.conn())?;
                Ok(())
            })
            .await
    }

    async fn record_retry_failure(&self, id: &str, why: &str) -> Result<(), store::Error> {
        let id = id.to_owned();
        let why: String = why.chars().take(240).collect();
        self.db
            .write(Actor::System("relay".into()), move |tx| {
                diesel::update(admin_relay_queue::table.find(&id))
                    .set((
                        admin_relay_queue::attempts.eq(admin_relay_queue::attempts + 1),
                        admin_relay_queue::last_error.eq(Some(why)),
                    ))
                    .execute(tx.conn())?;
                Ok(())
            })
            .await
    }
}

/// Formats `detail` with any down peers appended as `"pending on node X"`, so the audit log and
/// callers see which peers have not yet applied the action.
pub fn detail_with_pending(detail: Option<&str>, pending: &[NodeId]) -> Option<String> {
    if pending.is_empty() {
        return detail.map(str::to_owned);
    }
    let notes: Vec<String> = pending.iter().map(|peer| format!("pending on node {peer}")).collect();
    let joined = notes.join(", ");
    match detail {
        Some(text) if !text.is_empty() => Some(format!("{text} ({joined})")),
        _ => Some(joined),
    }
}
