// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! CLEAN-ROOM STUB (RD27): stands in for the Category A `federation.rs` (DN10b, chat between nodes), which the
//! integration pass copies in unchanged. Its API is the rewrite's reading of spec-rd27-chat §1.4, §2.9-§2.13 and
//! §3.1. `ChatFederation` here federates nothing: it answers as `LocalOnly` does.

use std::sync::Arc;

use module_api::chat::RoomKind;
use module_api::network::{NodeId, Region};
use module_api::{BoxFuture, GlobalId, NodeIdentity};
use network::{LinkError, NodeService, PeerLink, Service};
use store::Actor;
use tokio::sync::mpsc;

use crate::history::{History, Mute};
use crate::hooks::ChatHost;
use crate::server::{Clock, Event};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectWire {
    pub mid: String,
    pub from: GlobalId,
    pub from_local: String,
    pub to: GlobalId,
    pub body: String,
    pub stanza_id: Option<String>,
    pub at: i64,
    pub delivered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomWire {
    pub mid: String,
    pub room: RoomKind,
    pub from: GlobalId,
    pub nick: String,
    pub body: String,
    pub stanza_id: Option<String>,
    pub at: i64,
    pub history: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MuteWire {
    pub account: String,
    pub until: Option<i64>,
    pub reason: Option<String>,
    pub by: String,
    pub at: i64,
}

/// What a node publishes to the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Envelope {
    Direct {
        mid: String,
        from: GlobalId,
        from_local: String,
        to: GlobalId,
        body: String,
        id: Option<String>,
        at: i64,
        delivered: bool,
    },
    Room {
        mid: String,
        room: RoomKind,
        from: GlobalId,
        nick: String,
        body: String,
        id: Option<String>,
        at: i64,
        history: bool,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum FederationError {
    #[error("the node that owns it did not answer: {0}")]
    Unreachable(String),
}

/// Chat between nodes, as the chat task sees it.
pub trait Federation: Send + Sync {
    fn attach(&self, events: mpsc::Sender<Event>, history: History, host: Arc<dyn ChatHost>, clock: Clock);
    fn is_federated(&self) -> bool;
    fn resolve<'a>(&'a self, local: &'a str) -> BoxFuture<'a, Option<GlobalId>>;
    fn presence<'a>(&'a self, account: &'a GlobalId, local: &'a str, online: bool) -> BoxFuture<'a, ()>;
    fn mute_of<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Option<Mute>>;
    fn set_mute<'a>(&'a self, account: &'a GlobalId, mute: Option<Mute>, actor: Actor) -> BoxFuture<'a, ()>;
    fn publish(&self, envelope: Envelope) -> BoxFuture<'_, Result<(), FederationError>>;
    fn pull_waiting<'a>(&'a self, account: &'a GlobalId, limit: i64) -> BoxFuture<'a, Vec<DirectWire>>;
    fn ack_waiting<'a>(&'a self, account: &'a GlobalId, mids: Vec<String>, at: i64) -> BoxFuture<'a, ()>;
    fn room_joined<'a>(&'a self, room: &'a RoomKind, history_limit: i64) -> BoxFuture<'a, Vec<RoomWire>>;
    fn room_left<'a>(&'a self, room: &'a RoomKind) -> BoxFuture<'a, ()>;
    fn tick(&self, now: i64) -> BoxFuture<'_, ()>;
}

/// One node, no peers.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalOnly;

impl Federation for LocalOnly {
    fn attach(&self, _: mpsc::Sender<Event>, _: History, _: Arc<dyn ChatHost>, _: Clock) {}
    fn is_federated(&self) -> bool {
        false
    }
    fn resolve<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Option<GlobalId>> {
        Box::pin(async { None })
    }
    fn presence<'a>(&'a self, _: &'a GlobalId, _: &'a str, _: bool) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn mute_of<'a>(&'a self, _: &'a GlobalId) -> BoxFuture<'a, Option<Mute>> {
        Box::pin(async { None })
    }
    fn set_mute<'a>(&'a self, _: &'a GlobalId, _: Option<Mute>, _: Actor) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn publish(&self, _: Envelope) -> BoxFuture<'_, Result<(), FederationError>> {
        Box::pin(async { Ok(()) })
    }
    fn pull_waiting<'a>(&'a self, _: &'a GlobalId, _: i64) -> BoxFuture<'a, Vec<DirectWire>> {
        Box::pin(async { Vec::new() })
    }
    fn ack_waiting<'a>(&'a self, _: &'a GlobalId, _: Vec<String>, _: i64) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn room_joined<'a>(&'a self, _: &'a RoomKind, _: i64) -> BoxFuture<'a, Vec<RoomWire>> {
        Box::pin(async { Vec::new() })
    }
    fn room_left<'a>(&'a self, _: &'a RoomKind) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn tick(&self, _: i64) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

#[derive(Debug, Clone)]
pub struct ChatFederationConfig {
    pub node: NodeIdentity,
    pub leader: Option<NodeId>,
    pub peers: Vec<(NodeId, Region)>,
}

impl ChatFederationConfig {
    pub fn new(node: NodeIdentity) -> Self {
        Self {
            node,
            leader: None,
            peers: Vec::new(),
        }
    }

    pub fn with_leader(mut self, leader: NodeId, peers: Vec<(NodeId, Region)>) -> Self {
        self.leader = Some(leader);
        self.peers = peers;
        self
    }
}

/// The stub federates nothing.
pub struct ChatFederation {
    _config: ChatFederationConfig,
    _link: Arc<dyn PeerLink>,
}

impl ChatFederation {
    pub fn new(config: ChatFederationConfig, link: Arc<dyn PeerLink>) -> Arc<Self> {
        Arc::new(Self {
            _config: config,
            _link: link,
        })
    }
}

impl NodeService for ChatFederation {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        _: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            Err(LinkError::NoService {
                node: from.clone(),
                service,
            })
        })
    }
}

impl Federation for ChatFederation {
    fn attach(&self, events: mpsc::Sender<Event>, history: History, host: Arc<dyn ChatHost>, clock: Clock) {
        LocalOnly.attach(events, history, host, clock);
    }
    fn is_federated(&self) -> bool {
        false
    }
    fn resolve<'a>(&'a self, local: &'a str) -> BoxFuture<'a, Option<GlobalId>> {
        LocalOnly.resolve(local)
    }
    fn presence<'a>(&'a self, _: &'a GlobalId, _: &'a str, _: bool) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn mute_of<'a>(&'a self, _: &'a GlobalId) -> BoxFuture<'a, Option<Mute>> {
        Box::pin(async { None })
    }
    fn set_mute<'a>(&'a self, _: &'a GlobalId, _: Option<Mute>, _: Actor) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn publish(&self, _: Envelope) -> BoxFuture<'_, Result<(), FederationError>> {
        Box::pin(async { Ok(()) })
    }
    fn pull_waiting<'a>(&'a self, _: &'a GlobalId, _: i64) -> BoxFuture<'a, Vec<DirectWire>> {
        Box::pin(async { Vec::new() })
    }
    fn ack_waiting<'a>(&'a self, _: &'a GlobalId, _: Vec<String>, _: i64) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn room_joined<'a>(&'a self, _: &'a RoomKind, _: i64) -> BoxFuture<'a, Vec<RoomWire>> {
        Box::pin(async { Vec::new() })
    }
    fn room_left<'a>(&'a self, _: &'a RoomKind) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn tick(&self, _: i64) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}
