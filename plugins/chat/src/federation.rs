// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Chat federation between nodes over `Service::Chat` (DD21, DN10b).
//!
//! On a server outside a network, chat runs with [`LocalOnly`]: every account and room is this node's, and
//! nothing leaves it.
//!
//! In a network, [`ChatFederation`] implements [`Federation`] (called by [`crate::server::Chat`]) and
//! [`network::NodeService`] (attached to `Service::Chat` on the node link):
//! - **Presence and account resolution:** when a player binds or ends their last session on a node, the node
//!   informs the player's home (`account.owner()`) and peers over `Service::Chat`. A whisper to a JID local
//!   part unknown locally resolves the account through peers.
//! - **Direct messages:** every whisper gets a unique `mid`, is stored at both players' homes (`from.owner()`
//!   and `to.owner()`) with [`History::add_direct_once`], and is routed through the recipient's home to the
//!   node(s) where the recipient is currently online. If the recipient is offline, the message waits at
//!   `to.owner()` (`delivered_at = None`) and is delivered once at their next login (pulled from `to.owner()`
//!   when logging in on another node), deduplicated by `mid`.
//! - **Rooms and room history:** `RoomKind::Network` is owned by the cluster's designated leader node;
//!   `RoomKind::Region(r)` is owned by the designated node for region `r`; `RoomKind::Party(id)` and
//!   `RoomKind::Hunt(id)` are owned by `id.owner()`. Nodes with local occupants subscribe at the room's owner
//!   on join (pulling bounded room history from the owner when the room's rules keep history) and unsubscribe
//!   when the last local occupant leaves. Room messages go to the owner and fan out to subscribed nodes,
//!   deduplicated by `mid`.
//! - **Mutes:** mutes and unmutes are recorded at the account's home (`account.owner()`), pushed to peers so
//!   live sessions follow them at once, and consulted at login when an account logs into another node.
//! - **Outbox (`chat_outbox`):** calls to a peer that is unreachable are persisted in `chat_outbox` and
//!   retried on `tick` so a partitioned node catches up when the link heals.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use module_api::chat::RoomKind;
use module_api::network::{NodeId, Region};
use module_api::{BoxFuture, GlobalId, NodeIdentity};
use network::link::MAX_MESSAGE;
use network::{LinkError, NodeService, PeerLink, Service};
use serde::{Deserialize, Serialize};
use store::Actor;
use tokio::sync::{mpsc, oneshot};

use crate::history::{DirectMessage, History, Mute, NewDirect, NewRoom, RoomMessage};
use crate::hooks::ChatHost;
use crate::muc::MAX_NICK_BYTES;
use crate::naming::room_key;
use crate::server::{BODY_LIMIT, Clock, Event, is_authcid_shape};
use crate::xmpp::{DEFAULT_DOMAIN, has_control_character};

pub const CHAT_FEDERATION_VERSION: u32 = 1;
pub const MAX_CHAT_WIRE: usize = MAX_MESSAGE;
pub const MAX_HISTORY_PULL: u32 = 100;
const MAX_MID_LEN: usize = 128;
const MAX_STANZA_ID_LEN: usize = 128;
const MAX_REASON_LEN: usize = 512;
const MAX_ACTOR_LEN: usize = 256;
const MAX_ACK_MIDS: usize = 200;
const MAX_OUTBOX_ROWS: i64 = 4096;
const OUTBOX_BATCH: i64 = 64;

/// One message, as another node needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Envelope {
    /// A direct message. DD21 stores it at both players' homes and routes it to the recipient's node.
    Direct {
        #[serde(default)]
        mid: String,
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
        #[serde(default)]
        mid: String,
        room: RoomKind,
        from: GlobalId,
        nick: String,
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        at: i64,
        #[serde(default)]
        history: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectWire {
    pub mid: String,
    pub from: GlobalId,
    pub from_local: String,
    pub to: GlobalId,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stanza_id: Option<String>,
    pub at: i64,
    pub delivered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomWire {
    pub mid: String,
    pub room: RoomKind,
    pub from: GlobalId,
    pub nick: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stanza_id: Option<String>,
    pub at: i64,
    pub history: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MuteWire {
    pub account: GlobalId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub by: String,
    pub at: i64,
}

impl MuteWire {
    pub fn from_mute(account: GlobalId, mute: &Mute) -> Self {
        Self {
            account,
            until: mute.until,
            reason: mute.reason.clone(),
            by: mute.by.clone(),
            at: mute.at,
        }
    }

    pub fn into_mute(self) -> Mute {
        Mute {
            account: self.account.to_string(),
            until: self.until,
            reason: self.reason,
            by: self.by,
            at: self.at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatOp {
    Resolve {
        local: String,
    },
    Presence {
        account: GlobalId,
        local: String,
        online: bool,
    },
    StoreDirect {
        message: DirectWire,
    },
    RouteDirect {
        message: DirectWire,
    },
    DeliverDirect {
        message: DirectWire,
    },
    PullWaiting {
        account: GlobalId,
        limit: u32,
    },
    AckWaiting {
        account: GlobalId,
        mids: Vec<String>,
        at: i64,
    },
    RoomJoin {
        room: RoomKind,
        history_limit: u32,
    },
    RoomLeave {
        room: RoomKind,
    },
    RoomPost {
        message: RoomWire,
    },
    RoomFanout {
        message: RoomWire,
    },
    GetMute {
        account: GlobalId,
    },
    SetMute {
        account: GlobalId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mute: Option<MuteWire>,
        actor: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatRequest {
    pub version: u32,
    pub op: ChatOp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatReply {
    Ack,
    Resolved {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<GlobalId>,
    },
    Direct {
        delivered: bool,
    },
    Waiting {
        messages: Vec<DirectWire>,
    },
    RoomHistory {
        messages: Vec<RoomWire>,
    },
    Mute {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mute: Option<MuteWire>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatResponse {
    pub version: u32,
    pub reply: ChatReply,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChatWireError {
    #[error("chat federation message is {0} bytes; max is {MAX_CHAT_WIRE}")]
    TooLarge(usize),
    #[error("invalid JSON: {0}")]
    Json(String),
    #[error("unsupported version {0}")]
    Version(u32),
    #[error("invalid field: {0}")]
    Field(String),
}

fn check_mid(mid: &str) -> Result<(), ChatWireError> {
    if mid.is_empty() || mid.len() > MAX_MID_LEN || mid.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(ChatWireError::Field(
            "mid must be 1..=128 non-whitespace printable chars".into(),
        ));
    }
    Ok(())
}

fn check_stanza_id(id: &Option<String>) -> Result<(), ChatWireError> {
    if let Some(id) = id
        && (id.is_empty() || id.encode_utf16().count() > MAX_STANZA_ID_LEN || has_control_character(id))
    {
        return Err(ChatWireError::Field("invalid stanza_id".into()));
    }
    Ok(())
}

fn check_body(body: &str) -> Result<(), ChatWireError> {
    let chars = body.chars().count();
    if !(1..=BODY_LIMIT).contains(&chars) {
        return Err(ChatWireError::Field(format!("body must be 1..={BODY_LIMIT} chars")));
    }
    Ok(())
}

fn check_direct_wire(msg: &DirectWire) -> Result<(), ChatWireError> {
    check_mid(&msg.mid)?;
    if !is_authcid_shape(&msg.from_local) {
        return Err(ChatWireError::Field("from_local is not a valid JID local part".into()));
    }
    check_body(&msg.body)?;
    check_stanza_id(&msg.stanza_id)?;
    if msg.at < 0 {
        return Err(ChatWireError::Field("at must be non-negative".into()));
    }
    Ok(())
}

fn check_room_wire(msg: &RoomWire) -> Result<(), ChatWireError> {
    check_mid(&msg.mid)?;
    if msg.nick.is_empty() || msg.nick.len() > MAX_NICK_BYTES || has_control_character(&msg.nick) {
        return Err(ChatWireError::Field("invalid room nick".into()));
    }
    check_body(&msg.body)?;
    check_stanza_id(&msg.stanza_id)?;
    if msg.at < 0 {
        return Err(ChatWireError::Field("at must be non-negative".into()));
    }
    Ok(())
}

fn check_mute_wire(mute: &MuteWire) -> Result<(), ChatWireError> {
    if let Some(reason) = &mute.reason
        && (reason.chars().count() > MAX_REASON_LEN || has_control_character(reason))
    {
        return Err(ChatWireError::Field("invalid mute reason".into()));
    }
    if mute.by.is_empty() || mute.by.len() > MAX_ACTOR_LEN || has_control_character(&mute.by) {
        return Err(ChatWireError::Field("invalid mute actor".into()));
    }
    if mute.at < 0 {
        return Err(ChatWireError::Field("mute at must be non-negative".into()));
    }
    Ok(())
}

pub fn validate_envelope(envelope: &Envelope) -> Result<(), ChatWireError> {
    match envelope {
        Envelope::Direct {
            mid,
            from_local,
            body,
            id,
            at,
            ..
        } => {
            if !mid.is_empty() {
                check_mid(mid)?;
            }
            if !is_authcid_shape(from_local) {
                return Err(ChatWireError::Field("from_local is not a valid JID local part".into()));
            }
            check_body(body)?;
            check_stanza_id(id)?;
            if *at < 0 {
                return Err(ChatWireError::Field("at must be non-negative".into()));
            }
            Ok(())
        }
        Envelope::Room {
            mid,
            nick,
            body,
            id,
            at,
            ..
        } => {
            if !mid.is_empty() {
                check_mid(mid)?;
            }
            if nick.is_empty() || nick.len() > MAX_NICK_BYTES || has_control_character(nick) {
                return Err(ChatWireError::Field("invalid room nick".into()));
            }
            check_body(body)?;
            check_stanza_id(id)?;
            if *at < 0 {
                return Err(ChatWireError::Field("at must be non-negative".into()));
            }
            Ok(())
        }
    }
}

pub fn validate_op(op: &ChatOp) -> Result<(), ChatWireError> {
    match op {
        ChatOp::Resolve { local } => {
            if !is_authcid_shape(local) {
                return Err(ChatWireError::Field("local is not a valid JID local part".into()));
            }
            Ok(())
        }
        ChatOp::Presence { local, .. } => {
            if !is_authcid_shape(local) {
                return Err(ChatWireError::Field("local is not a valid JID local part".into()));
            }
            Ok(())
        }
        ChatOp::StoreDirect { message } | ChatOp::RouteDirect { message } | ChatOp::DeliverDirect { message } => {
            check_direct_wire(message)
        }
        ChatOp::PullWaiting { limit, .. } => {
            if *limit == 0 || *limit > MAX_HISTORY_PULL {
                return Err(ChatWireError::Field(format!(
                    "PullWaiting limit must be 1..={MAX_HISTORY_PULL}"
                )));
            }
            Ok(())
        }
        ChatOp::AckWaiting { mids, at, .. } => {
            if mids.is_empty() || mids.len() > MAX_ACK_MIDS {
                return Err(ChatWireError::Field("AckWaiting mids out of bounds".into()));
            }
            for mid in mids {
                check_mid(mid)?;
            }
            if *at < 0 {
                return Err(ChatWireError::Field("at must be non-negative".into()));
            }
            Ok(())
        }
        ChatOp::RoomJoin { history_limit, .. } => {
            if *history_limit > MAX_HISTORY_PULL {
                return Err(ChatWireError::Field(format!(
                    "RoomJoin history_limit must be <={MAX_HISTORY_PULL}"
                )));
            }
            Ok(())
        }
        ChatOp::RoomLeave { .. } | ChatOp::GetMute { .. } => Ok(()),
        ChatOp::RoomPost { message } | ChatOp::RoomFanout { message } => check_room_wire(message),
        ChatOp::SetMute { mute, actor, .. } => {
            if actor.is_empty() || actor.len() > MAX_ACTOR_LEN || has_control_character(actor) {
                return Err(ChatWireError::Field("invalid actor".into()));
            }
            if let Some(m) = mute {
                check_mute_wire(m)?;
            }
            Ok(())
        }
    }
}

pub fn validate_reply(reply: &ChatReply) -> Result<(), ChatWireError> {
    match reply {
        ChatReply::Ack | ChatReply::Resolved { .. } | ChatReply::Direct { .. } => Ok(()),
        ChatReply::Waiting { messages } => {
            if messages.len() > MAX_HISTORY_PULL as usize {
                return Err(ChatWireError::Field("too many waiting messages".into()));
            }
            for m in messages {
                check_direct_wire(m)?;
            }
            Ok(())
        }
        ChatReply::RoomHistory { messages } => {
            if messages.len() > MAX_HISTORY_PULL as usize {
                return Err(ChatWireError::Field("too many room history messages".into()));
            }
            for m in messages {
                check_room_wire(m)?;
            }
            Ok(())
        }
        ChatReply::Mute { mute } => {
            if let Some(m) = mute {
                check_mute_wire(m)?;
            }
            Ok(())
        }
    }
}

pub fn encode_request(op: &ChatOp) -> Result<Vec<u8>, ChatWireError> {
    validate_op(op)?;
    let req = ChatRequest {
        version: CHAT_FEDERATION_VERSION,
        op: op.clone(),
    };
    let bytes = serde_json::to_vec(&req).map_err(|e| ChatWireError::Json(e.to_string()))?;
    if bytes.len() > MAX_CHAT_WIRE {
        return Err(ChatWireError::TooLarge(bytes.len()));
    }
    Ok(bytes)
}

pub fn decode_request(bytes: &[u8]) -> Result<ChatRequest, ChatWireError> {
    if bytes.len() > MAX_CHAT_WIRE {
        return Err(ChatWireError::TooLarge(bytes.len()));
    }
    let req: ChatRequest = serde_json::from_slice(bytes).map_err(|e| ChatWireError::Json(e.to_string()))?;
    if req.version != CHAT_FEDERATION_VERSION {
        return Err(ChatWireError::Version(req.version));
    }
    validate_op(&req.op)?;
    Ok(req)
}

pub fn encode_response(reply: &ChatReply) -> Result<Vec<u8>, ChatWireError> {
    validate_reply(reply)?;
    let resp = ChatResponse {
        version: CHAT_FEDERATION_VERSION,
        reply: reply.clone(),
    };
    let bytes = serde_json::to_vec(&resp).map_err(|e| ChatWireError::Json(e.to_string()))?;
    if bytes.len() > MAX_CHAT_WIRE {
        return Err(ChatWireError::TooLarge(bytes.len()));
    }
    Ok(bytes)
}

pub fn decode_response(bytes: &[u8]) -> Result<ChatResponse, ChatWireError> {
    if bytes.len() > MAX_CHAT_WIRE {
        return Err(ChatWireError::TooLarge(bytes.len()));
    }
    let resp: ChatResponse = serde_json::from_slice(bytes).map_err(|e| ChatWireError::Json(e.to_string()))?;
    if resp.version != CHAT_FEDERATION_VERSION {
        return Err(ChatWireError::Version(resp.version));
    }
    validate_reply(&resp.reply)?;
    Ok(resp)
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FederationError {
    #[error("the node that owns it did not answer: {0}")]
    Unreachable(String),
}

pub trait Federation: Send + Sync {
    /// Whether this server federates chat with peer nodes (`false` for [`LocalOnly`]).
    fn is_federated(&self) -> bool {
        false
    }

    /// Attaches the local chat runtime when [`crate::ChatPlugin::start`] boots.
    fn attach(
        &self,
        _events: mpsc::UnboundedSender<Event>,
        _history: History,
        _host: Arc<dyn ChatHost>,
        _clock: Clock,
    ) {
    }

    /// The account a JID local part names when no session or record on this node knows it (an account homed
    /// elsewhere). On one node: nobody.
    fn resolve<'a>(&'a self, local: &'a str) -> BoxFuture<'a, Option<GlobalId>>;

    /// A message handled here, for the nodes that must also hold or deliver it.
    fn publish(&self, envelope: Envelope) -> BoxFuture<'_, Result<(), FederationError>>;

    /// Records at the account's home whether the account has a bound session on this node.
    fn presence<'a>(&'a self, _account: &'a GlobalId, _local: &'a str, _online: bool) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Pulls waiting direct messages from `account.owner()` when `account` binds on another node.
    fn pull_waiting<'a>(&'a self, _account: &'a GlobalId, _limit: i64) -> BoxFuture<'a, Vec<DirectWire>> {
        Box::pin(async { Vec::new() })
    }

    /// Marks pulled waiting direct messages delivered at `account.owner()`.
    fn ack_waiting<'a>(&'a self, _account: &'a GlobalId, _mids: Vec<String>, _at: i64) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Subscribes this node to `room` at the room's owner and pulls up to `history_limit` recent messages.
    fn room_joined<'a>(&'a self, _room: &'a RoomKind, _history_limit: i64) -> BoxFuture<'a, Vec<RoomWire>> {
        Box::pin(async { Vec::new() })
    }

    /// Unsubscribes this node from `room` at the room's owner when the last local occupant leaves.
    fn room_left<'a>(&'a self, _room: &'a RoomKind) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Consults `account.owner()` for an active mute when `account` logs in on another node.
    fn mute_of<'a>(&'a self, _account: &'a GlobalId) -> BoxFuture<'a, Option<Mute>> {
        Box::pin(async { None })
    }

    /// Propagates a mute or unmute to `account.owner()` and peer nodes.
    fn set_mute<'a>(&'a self, _account: &'a GlobalId, _mute: Option<Mute>, _actor: Actor) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Flushes due calls from `chat_outbox` on tick.
    fn tick(&self, _now: i64) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
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

#[derive(Clone)]
struct AttachedLocal {
    events: mpsc::UnboundedSender<Event>,
    history: History,
    host: Arc<dyn ChatHost>,
    clock: Clock,
}

#[derive(Clone)]
pub struct ChatFederationConfig {
    pub node: NodeIdentity,
    pub leader: Option<NodeId>,
    pub peers: Vec<(NodeId, Region)>,
}

impl ChatFederationConfig {
    pub fn new(node: NodeIdentity, peers: impl IntoIterator<Item = (NodeId, Region)>) -> Self {
        Self {
            node,
            leader: None,
            peers: peers.into_iter().collect(),
        }
    }

    pub fn with_leader(mut self, leader: NodeId) -> Self {
        self.leader = Some(leader);
        self
    }
}

/// Chat federation over [`PeerLink`] (`Service::Chat`, DD21, DN10b).
pub struct ChatFederation {
    node: NodeIdentity,
    leader: NodeId,
    /// Every node in the cluster (including this node), ordered by `NodeId`.
    all_nodes: BTreeMap<NodeId, Region>,
    peers: BTreeSet<NodeId>,
    link: Arc<dyn PeerLink>,
    attached: Mutex<Option<AttachedLocal>>,
    /// At `account.owner()`: which nodes currently hold bound sessions of `account`.
    presence: Mutex<BTreeMap<GlobalId, BTreeSet<NodeId>>>,
    /// At `room_owner(room)`: which peer nodes currently hold local occupants in `room_key(room)`.
    room_members: Mutex<BTreeMap<String, BTreeSet<NodeId>>>,
}

impl ChatFederation {
    pub fn new(config: ChatFederationConfig, link: Arc<dyn PeerLink>) -> Self {
        let mut all_nodes = BTreeMap::new();
        let mut peers = BTreeSet::new();
        all_nodes.insert(config.node.node.clone(), config.node.region.clone());
        for (id, region) in config.peers {
            if id != config.node.node {
                peers.insert(id.clone());
                all_nodes.insert(id, region);
            }
        }
        let leader = config
            .leader
            .or_else(|| all_nodes.keys().next().cloned())
            .unwrap_or_else(|| config.node.node.clone());
        Self {
            node: config.node,
            leader,
            all_nodes,
            peers,
            link,
            attached: Mutex::new(None),
            presence: Mutex::new(BTreeMap::new()),
            room_members: Mutex::new(BTreeMap::new()),
        }
    }

    /// Designated owner node for `kind` (DD21):
    /// - `RoomKind::Network`: the cluster leader
    /// - `RoomKind::Region(r)`: the first node in region `r`, or the leader if none
    /// - `RoomKind::Party(id)` / `RoomKind::Hunt(id)`: `id.owner()`
    pub fn room_owner(&self, kind: &RoomKind) -> NodeId {
        match kind {
            RoomKind::Network => self.leader.clone(),
            RoomKind::Region(region) => self
                .all_nodes
                .iter()
                .find(|(_, r)| *r == region)
                .map(|(n, _)| n.clone())
                .unwrap_or_else(|| self.leader.clone()),
            RoomKind::Party(id) | RoomKind::Hunt(id) => id.owner().clone(),
        }
    }

    fn local(&self) -> Option<AttachedLocal> {
        self.attached.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    async fn call_peer(&self, peer: &NodeId, op: &ChatOp) -> Result<ChatReply, LinkError> {
        let body = encode_request(op).map_err(|e| LinkError::Protocol {
            node: self.node.node.clone(),
            reason: e.to_string(),
        })?;
        let resp_bytes = self.link.call(peer, Service::Chat, body).await?;
        let resp = decode_response(&resp_bytes).map_err(|e| LinkError::Protocol {
            node: peer.clone(),
            reason: e.to_string(),
        })?;
        Ok(resp.reply)
    }

    async fn enqueue_outbox(&self, peer: &NodeId, op: &ChatOp) {
        let Some(local) = self.local() else {
            return;
        };
        let Ok(bytes) = encode_request(op) else {
            return;
        };
        let Ok(json) = String::from_utf8(bytes) else {
            return;
        };
        let now = (local.clock)();
        if local.history.outbox_len().await.unwrap_or(0) >= MAX_OUTBOX_ROWS {
            tracing::warn!("chat: federation outbox full, dropping call to {peer}");
            return;
        }
        if let Err(e) = local.history.outbox_add(peer.as_str(), &json, now).await {
            tracing::warn!("chat: could not queue federation outbox call to {peer} ({e})");
        }
    }

    async fn call_or_queue(&self, peer: &NodeId, op: ChatOp) -> Option<ChatReply> {
        match self.call_peer(peer, &op).await {
            Ok(reply) => Some(reply),
            Err(LinkError::Unreachable { .. } | LinkError::NoService { .. }) => {
                self.enqueue_outbox(peer, &op).await;
                None
            }
            Err(e) => {
                tracing::warn!("chat: federation call to {peer} failed ({e})");
                None
            }
        }
    }

    fn record_presence_here(&self, account: &GlobalId, node: &NodeId, online: bool) {
        let mut guard = self.presence.lock().unwrap_or_else(PoisonError::into_inner);
        if online {
            guard.entry(account.clone()).or_default().insert(node.clone());
        } else if let Some(nodes) = guard.get_mut(account) {
            nodes.remove(node);
            if nodes.is_empty() {
                guard.remove(account);
            }
        }
    }

    fn present_nodes(&self, account: &GlobalId) -> Vec<NodeId> {
        self.presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(account)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    async fn deliver_direct_locally(&self, message: &DirectWire) -> bool {
        let Some(local) = self.local() else {
            return false;
        };
        let (tx, rx) = oneshot::channel();
        if local
            .events
            .send(Event::RemoteDirect {
                message: message.clone(),
                reply: tx,
            })
            .is_err()
        {
            return false;
        }
        rx.await.unwrap_or(false)
    }

    async fn deliver_room_locally(&self, message: RoomWire) {
        let Some(local) = self.local() else {
            return;
        };
        let _ = local.events.send(Event::RemoteRoom { message });
    }

    /// Handles a DM at `to.owner()` (or when `this_node == to.owner()`): stores the DM in `to.owner()`'s
    /// history before delivering locally or to `to`'s present peer node(s), then marks it delivered.
    async fn route_direct_at_home(&self, message: DirectWire, check_local: bool) -> bool {
        let Some(local) = self.local() else {
            return false;
        };
        if let Ok(Some(existing)) = local.history.direct_by_mid(&message.mid).await
            && existing.delivered_at.is_some()
        {
            return true;
        }
        let now = (local.clock)();
        let _ = local
            .history
            .add_direct_once(NewDirect {
                sender: message.from.to_string(),
                sender_local: message.from_local.clone(),
                recipient: message.to.to_string(),
                body: message.body.clone(),
                stanza_id: message.stanza_id.clone(),
                at: message.at,
                delivered_at: message.delivered.then_some(now),
                mid: Some(message.mid.clone()),
            })
            .await;
        let mut delivered = message.delivered;
        if !delivered && check_local && self.deliver_direct_locally(&message).await {
            delivered = true;
        }
        if !delivered {
            let targets = self.present_nodes(&message.to);
            for peer in targets {
                if peer == self.node.node {
                    continue;
                }
                let op = ChatOp::DeliverDirect {
                    message: DirectWire {
                        delivered: true,
                        ..message.clone()
                    },
                };
                match self.call_peer(&peer, &op).await {
                    Ok(ChatReply::Direct { delivered: true }) => {
                        delivered = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(LinkError::Unreachable { .. } | LinkError::NoService { .. }) => {
                        self.enqueue_outbox(&peer, &op).await;
                    }
                    Err(_) => {}
                }
            }
        }
        if delivered {
            let _ = local
                .history
                .mark_delivered_mids(&message.to.to_string(), vec![message.mid], now)
                .await;
        }
        delivered
    }

    async fn handle_op(&self, from: &NodeId, op: ChatOp) -> ChatReply {
        let Some(local) = self.local() else {
            return ChatReply::Ack;
        };
        match op {
            ChatOp::Resolve { local: name } => {
                if let Ok(Some(text)) = local.history.account_of(&name).await
                    && let Ok(id) = text.parse::<GlobalId>()
                {
                    return ChatReply::Resolved { account: Some(id) };
                }
                let account = local.host.account_of(&name).await;
                ChatReply::Resolved { account }
            }
            ChatOp::Presence {
                account,
                local: name,
                online,
            } => {
                let now = (local.clock)();
                let _ = local
                    .history
                    .remember_account(&name, &account.to_string(), "federated", DEFAULT_DOMAIN, now)
                    .await;
                if account.owner() == &self.node.node {
                    self.record_presence_here(&account, from, online);
                }
                ChatReply::Ack
            }
            ChatOp::StoreDirect { message } => {
                let now = (local.clock)();
                let _ = local
                    .history
                    .add_direct_once(NewDirect {
                        sender: message.from.to_string(),
                        sender_local: message.from_local,
                        recipient: message.to.to_string(),
                        body: message.body,
                        stanza_id: message.stanza_id,
                        at: message.at,
                        delivered_at: Some(now),
                        mid: Some(message.mid),
                    })
                    .await;
                ChatReply::Ack
            }
            ChatOp::RouteDirect { message } => {
                let delivered = self.route_direct_at_home(message, true).await;
                ChatReply::Direct { delivered }
            }
            ChatOp::DeliverDirect { message } => {
                let delivered = self.deliver_direct_locally(&message).await;
                if delivered {
                    let now = (local.clock)();
                    let _ = local
                        .history
                        .add_direct_once(NewDirect {
                            sender: message.from.to_string(),
                            sender_local: message.from_local,
                            recipient: message.to.to_string(),
                            body: message.body,
                            stanza_id: message.stanza_id,
                            at: message.at,
                            delivered_at: Some(now),
                            mid: Some(message.mid),
                        })
                        .await;
                }
                ChatReply::Direct { delivered }
            }
            ChatOp::PullWaiting { account, limit } => {
                let rows = local
                    .history
                    .pending_for(&account.to_string(), i64::from(limit))
                    .await
                    .unwrap_or_default();
                let messages = rows.into_iter().filter_map(direct_row_to_wire).collect();
                ChatReply::Waiting { messages }
            }
            ChatOp::AckWaiting { account, mids, at } => {
                let _ = local.history.mark_delivered_mids(&account.to_string(), mids, at).await;
                ChatReply::Ack
            }
            ChatOp::RoomJoin { room, history_limit } => {
                let key = room_key(&room);
                self.room_members
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .entry(key.clone())
                    .or_default()
                    .insert(from.clone());
                let messages = if history_limit > 0 {
                    let rows = local
                        .history
                        .room_history(&key, i64::from(history_limit))
                        .await
                        .unwrap_or_default();
                    rows.into_iter().filter_map(|r| room_row_to_wire(&room, r)).collect()
                } else {
                    Vec::new()
                };
                ChatReply::RoomHistory { messages }
            }
            ChatOp::RoomLeave { room } => {
                let key = room_key(&room);
                let mut guard = self.room_members.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(set) = guard.get_mut(&key) {
                    set.remove(from);
                    if set.is_empty() {
                        guard.remove(&key);
                    }
                }
                ChatReply::Ack
            }
            ChatOp::RoomPost { message } => {
                let key = room_key(&message.room);
                self.room_members
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .entry(key.clone())
                    .or_default()
                    .insert(from.clone());
                if message.history {
                    let inserted = local
                        .history
                        .add_room_once(NewRoom {
                            room: key.clone(),
                            sender: message.from.to_string(),
                            nick: message.nick.clone(),
                            body: message.body.clone(),
                            stanza_id: message.stanza_id.clone(),
                            mid: Some(message.mid.clone()),
                            at: message.at,
                        })
                        .await
                        .unwrap_or(false);
                    if !inserted {
                        return ChatReply::Ack;
                    }
                }
                self.deliver_room_locally(message.clone()).await;
                let subscribers: Vec<NodeId> = self
                    .room_members
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&key)
                    .map(|s| {
                        s.iter()
                            .filter(|n| *n != from && *n != &self.node.node)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                for peer in subscribers {
                    let _ = self
                        .call_or_queue(
                            &peer,
                            ChatOp::RoomFanout {
                                message: message.clone(),
                            },
                        )
                        .await;
                }
                ChatReply::Ack
            }
            ChatOp::RoomFanout { message } => {
                self.deliver_room_locally(message).await;
                ChatReply::Ack
            }
            ChatOp::GetMute { account } => {
                let now = (local.clock)();
                let mute = local
                    .history
                    .mute_of(&account.to_string())
                    .await
                    .ok()
                    .flatten()
                    .filter(|m| m.holds(now))
                    .map(|m| MuteWire::from_mute(account, &m));
                ChatReply::Mute { mute }
            }
            ChatOp::SetMute { account, mute, actor } => {
                let parsed_actor = parse_actor(&actor);
                let live_mute = match mute {
                    Some(wire) => {
                        let m = wire.into_mute();
                        let _ = local.history.set_mute(m.clone(), parsed_actor).await;
                        Some(m)
                    }
                    None => {
                        let _ = local.history.clear_mute(&account.to_string(), parsed_actor).await;
                        None
                    }
                };
                let _ = local.events.send(Event::Muted {
                    account,
                    mute: live_mute,
                });
                ChatReply::Ack
            }
        }
    }
}

fn direct_row_to_wire(row: DirectMessage) -> Option<DirectWire> {
    Some(DirectWire {
        mid: row.mid.unwrap_or_else(|| format!("legacy-{}", row.id)),
        from: row.sender.parse().ok()?,
        from_local: row.sender_local,
        to: row.recipient.parse().ok()?,
        body: row.body,
        stanza_id: row.stanza_id,
        at: row.at,
        delivered: row.delivered_at.is_some(),
    })
}

fn room_row_to_wire(room: &RoomKind, row: RoomMessage) -> Option<RoomWire> {
    Some(RoomWire {
        mid: row.mid.unwrap_or_else(|| format!("legacy-{}", row.id)),
        room: room.clone(),
        from: row.sender.parse().ok()?,
        nick: row.nick,
        body: row.body,
        stanza_id: row.stanza_id,
        at: row.at,
        history: true,
    })
}

fn parse_actor(text: &str) -> Actor {
    if let Some(rest) = text.strip_prefix("admin:") {
        Actor::Admin(rest.to_owned())
    } else if let Some(rest) = text.strip_prefix("player:") {
        Actor::Player(rest.to_owned())
    } else if let Some(rest) = text.strip_prefix("system:") {
        Actor::System(rest.to_owned())
    } else {
        Actor::System(text.to_owned())
    }
}

impl NodeService for ChatFederation {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if service != Service::Chat {
                return Err(LinkError::NoService {
                    node: self.node.node.clone(),
                    service,
                });
            }
            if !self.peers.contains(from) {
                return Err(LinkError::Refused {
                    node: self.node.node.clone(),
                    reason: format!("{from} is not a peer of {}", self.node.node),
                });
            }
            let req = decode_request(&body).map_err(|e| LinkError::Protocol {
                node: from.clone(),
                reason: e.to_string(),
            })?;
            let reply = self.handle_op(from, req.op).await;
            encode_response(&reply).map_err(|e| LinkError::Protocol {
                node: self.node.node.clone(),
                reason: e.to_string(),
            })
        })
    }
}

impl Federation for ChatFederation {
    fn is_federated(&self) -> bool {
        !self.peers.is_empty()
    }

    fn attach(&self, events: mpsc::UnboundedSender<Event>, history: History, host: Arc<dyn ChatHost>, clock: Clock) {
        *self.attached.lock().unwrap_or_else(PoisonError::into_inner) = Some(AttachedLocal {
            events,
            history,
            host,
            clock,
        });
    }

    fn resolve<'a>(&'a self, local: &'a str) -> BoxFuture<'a, Option<GlobalId>> {
        Box::pin(async move {
            if !is_authcid_shape(local) {
                return None;
            }
            if let Some(att) = self.local()
                && let Some(id) = att.host.account_of(local).await
            {
                return Some(id);
            }
            let op = ChatOp::Resolve {
                local: local.to_owned(),
            };
            for peer in &self.peers {
                if let Ok(ChatReply::Resolved { account: Some(id) }) = self.call_peer(peer, &op).await {
                    if let Some(att) = self.local() {
                        let now = (att.clock)();
                        let _ = att
                            .history
                            .remember_account(local, &id.to_string(), "federated", DEFAULT_DOMAIN, now)
                            .await;
                    }
                    return Some(id);
                }
            }
            None
        })
    }

    fn publish(&self, envelope: Envelope) -> BoxFuture<'_, Result<(), FederationError>> {
        Box::pin(async move {
            match envelope {
                Envelope::Direct {
                    mid,
                    from,
                    from_local,
                    to,
                    body,
                    id,
                    at,
                    delivered,
                } => {
                    let mid = if mid.is_empty() {
                        format!("dm-{}", crate::server::random_hex(16))
                    } else {
                        mid
                    };
                    let wire = DirectWire {
                        mid: mid.clone(),
                        from: from.clone(),
                        from_local,
                        to: to.clone(),
                        body,
                        stanza_id: id,
                        at,
                        delivered,
                    };
                    let sender_home = from.owner();
                    let recipient_home = to.owner();

                    if sender_home != &self.node.node && sender_home != recipient_home {
                        let _ = self
                            .call_or_queue(
                                sender_home,
                                ChatOp::StoreDirect {
                                    message: DirectWire {
                                        delivered: true,
                                        ..wire.clone()
                                    },
                                },
                            )
                            .await;
                    }

                    if recipient_home == &self.node.node {
                        if !delivered {
                            let _ = self.route_direct_at_home(wire, false).await;
                        }
                    } else if delivered {
                        let _ = self
                            .call_or_queue(
                                recipient_home,
                                ChatOp::StoreDirect {
                                    message: DirectWire {
                                        delivered: true,
                                        ..wire
                                    },
                                },
                            )
                            .await;
                    } else {
                        let _ = self
                            .call_or_queue(recipient_home, ChatOp::RouteDirect { message: wire })
                            .await;
                    }
                    Ok(())
                }
                Envelope::Room {
                    mid,
                    room,
                    from,
                    nick,
                    body,
                    id,
                    at,
                    history,
                } => {
                    let mid = if mid.is_empty() {
                        format!("rm-{}", crate::server::random_hex(16))
                    } else {
                        mid
                    };
                    let wire = RoomWire {
                        mid,
                        room: room.clone(),
                        from,
                        nick,
                        body,
                        stanza_id: id,
                        at,
                        history,
                    };
                    let owner = self.room_owner(&room);
                    if owner == self.node.node {
                        let key = room_key(&room);
                        let subscribers: Vec<NodeId> = self
                            .room_members
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .get(&key)
                            .map(|s| s.iter().filter(|n| *n != &self.node.node).cloned().collect())
                            .unwrap_or_default();
                        for peer in subscribers {
                            let _ = self
                                .call_or_queue(&peer, ChatOp::RoomFanout { message: wire.clone() })
                                .await;
                        }
                    } else {
                        let _ = self.call_or_queue(&owner, ChatOp::RoomPost { message: wire }).await;
                    }
                    Ok(())
                }
            }
        })
    }

    fn presence<'a>(&'a self, account: &'a GlobalId, local: &'a str, online: bool) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if !is_authcid_shape(local) {
                return;
            }
            if account.owner() == &self.node.node {
                self.record_presence_here(account, &self.node.node, online);
            }
            let op = ChatOp::Presence {
                account: account.clone(),
                local: local.to_owned(),
                online,
            };
            if account.owner() != &self.node.node {
                let _ = self.call_or_queue(account.owner(), op.clone()).await;
            }
            if online {
                for peer in &self.peers {
                    if peer != account.owner() {
                        let _ = self.call_peer(peer, &op).await;
                    }
                }
            }
        })
    }

    fn pull_waiting<'a>(&'a self, account: &'a GlobalId, limit: i64) -> BoxFuture<'a, Vec<DirectWire>> {
        Box::pin(async move {
            let home = account.owner();
            if home == &self.node.node {
                return Vec::new();
            }
            let bounded = u32::try_from(limit.clamp(1, i64::from(MAX_HISTORY_PULL))).unwrap_or(MAX_HISTORY_PULL);
            match self
                .call_peer(
                    home,
                    &ChatOp::PullWaiting {
                        account: account.clone(),
                        limit: bounded,
                    },
                )
                .await
            {
                Ok(ChatReply::Waiting { messages }) => messages,
                _ => Vec::new(),
            }
        })
    }

    fn ack_waiting<'a>(&'a self, account: &'a GlobalId, mids: Vec<String>, at: i64) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let home = account.owner();
            if home == &self.node.node || mids.is_empty() {
                return;
            }
            let _ = self
                .call_or_queue(
                    home,
                    ChatOp::AckWaiting {
                        account: account.clone(),
                        mids,
                        at,
                    },
                )
                .await;
        })
    }

    fn room_joined<'a>(&'a self, room: &'a RoomKind, history_limit: i64) -> BoxFuture<'a, Vec<RoomWire>> {
        Box::pin(async move {
            let owner = self.room_owner(room);
            if owner == self.node.node {
                return Vec::new();
            }
            let bounded =
                u32::try_from(history_limit.clamp(0, i64::from(MAX_HISTORY_PULL))).unwrap_or(MAX_HISTORY_PULL);
            let op = ChatOp::RoomJoin {
                room: room.clone(),
                history_limit: bounded,
            };
            match self.call_or_queue(&owner, op).await {
                Some(ChatReply::RoomHistory { messages }) => messages,
                _ => Vec::new(),
            }
        })
    }

    fn room_left<'a>(&'a self, room: &'a RoomKind) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let owner = self.room_owner(room);
            if owner == self.node.node {
                return;
            }
            let _ = self.call_peer(&owner, &ChatOp::RoomLeave { room: room.clone() }).await;
        })
    }

    fn mute_of<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Option<Mute>> {
        Box::pin(async move {
            let home = account.owner();
            if home == &self.node.node {
                return None;
            }
            match self
                .call_peer(
                    home,
                    &ChatOp::GetMute {
                        account: account.clone(),
                    },
                )
                .await
            {
                Ok(ChatReply::Mute { mute: Some(wire) }) => {
                    let m = wire.into_mute();
                    if let Some(local) = self.local() {
                        let actor = parse_actor(&m.by);
                        let _ = local.history.set_mute(m.clone(), actor).await;
                    }
                    Some(m)
                }
                _ => None,
            }
        })
    }

    fn set_mute<'a>(&'a self, account: &'a GlobalId, mute: Option<Mute>, actor: Actor) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let wire = mute.as_ref().map(|m| MuteWire::from_mute(account.clone(), m));
            let op = ChatOp::SetMute {
                account: account.clone(),
                mute: wire,
                actor: actor.as_text(),
            };
            for peer in &self.peers {
                let _ = self.call_or_queue(peer, op.clone()).await;
            }
        })
    }

    fn tick(&self, now: i64) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let Some(local) = self.local() else {
                return;
            };
            let rows = local.history.outbox_due(now, OUTBOX_BATCH).await.unwrap_or_default();
            for row in rows {
                let Ok(peer) = row.peer.parse::<NodeId>() else {
                    let _ = local.history.outbox_done(row.id).await;
                    continue;
                };
                let Ok(req) = decode_request(row.call.as_bytes()) else {
                    let _ = local.history.outbox_done(row.id).await;
                    continue;
                };
                match self.call_peer(&peer, &req.op).await {
                    Ok(reply) => {
                        if let (
                            ChatOp::DeliverDirect { message } | ChatOp::RouteDirect { message },
                            ChatReply::Direct { delivered: true },
                        ) = (&req.op, &reply)
                        {
                            let _ = local
                                .history
                                .mark_delivered_mids(&message.to.to_string(), vec![message.mid.clone()], now)
                                .await;
                        }
                        if let ChatReply::RoomHistory { messages } = reply {
                            for msg in messages {
                                self.deliver_room_locally(msg).await;
                            }
                        }
                        let _ = local.history.outbox_done(row.id).await;
                    }
                    Err(LinkError::Unreachable { .. } | LinkError::NoService { .. }) => {
                        let _ = local.history.outbox_retry(row.id, row.tries + 1, now).await;
                    }
                    Err(_) => {
                        let _ = local.history.outbox_done(row.id).await;
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use module_api::IdKind;

    use super::*;

    #[test]
    fn wire_round_trip_and_bounds() {
        let oregon: NodeId = "oregon-1".parse().unwrap();
        let frankfurt: NodeId = "frankfurt-1".parse().unwrap();
        let ada = GlobalId::new(IdKind::Account, &oregon);
        let bo = GlobalId::new(IdKind::Account, &frankfurt);

        let dm = DirectWire {
            mid: "dm-0123456789abcdef".into(),
            from: ada.clone(),
            from_local: "ada".into(),
            to: bo,
            body: "hello across continents".into(),
            stanza_id: Some("w1".into()),
            at: 1_761_500_000_000,
            delivered: false,
        };
        let op = ChatOp::RouteDirect { message: dm };
        let bytes = encode_request(&op).unwrap();
        let decoded = decode_request(&bytes).unwrap();
        assert_eq!(decoded.version, CHAT_FEDERATION_VERSION);
        assert_eq!(decoded.op, op);

        let reply = ChatReply::Direct { delivered: true };
        let resp_bytes = encode_response(&reply).unwrap();
        let decoded_resp = decode_response(&resp_bytes).unwrap();
        assert_eq!(decoded_resp.reply, reply);

        let bad_local = ChatOp::Resolve {
            local: "bad@jid".into(),
        };
        assert!(encode_request(&bad_local).is_err());

        let oversized_body = DirectWire {
            mid: "dm-1".into(),
            from: ada.clone(),
            from_local: "ada".into(),
            to: ada,
            body: "x".repeat(BODY_LIMIT + 1),
            stanza_id: None,
            at: 1,
            delivered: false,
        };
        assert!(
            encode_request(&ChatOp::StoreDirect {
                message: oversized_body
            })
            .is_err()
        );
    }
}
