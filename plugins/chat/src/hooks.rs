// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! CLEAN-ROOM STUB (RD27): stands in for the Category A `hooks.rs`, which the integration pass copies in unchanged.
//! Its API is the clean-room rewrite's reading of spec-rd27-chat §1.4, §2.8.3 and §3.4; nothing here is the real
//! file. Where the real signatures differ, the Category B callers change to fit, never the A file.

use std::collections::BTreeMap;
use std::sync::Arc;

use module_api::chat::{ChatAnswer, ChatQuery, Jid, RoomKind, RoomRules};
use module_api::{BoxFuture, ErrorKind, GlobalId, ModuleDriver, ModuleError, ModuleId};

/// A SASL PLAIN login as chat hands it to its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub authcid: String,
    pub token: String,
    pub domain: String,
}

/// Whom a login named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    pub account: GlobalId,
    pub module: ModuleId,
    /// The display name at login
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginRefusal {
    Expired,
    BadToken,
    NoAccount,
    Unavailable,
}

impl LoginRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::BadToken => "bad-token",
            Self::NoAccount => "no-account",
            Self::Unavailable => "unavailable",
        }
    }
}

/// What chat asks of the server it runs in.
pub trait ChatHost: Send + Sync {
    fn login(&self, login: Login) -> BoxFuture<'_, Result<Player, LoginRefusal>>;

    fn ask<'a>(&'a self, module: &'a ModuleId, query: ChatQuery) -> BoxFuture<'a, Result<ChatAnswer, ModuleError>>;

    fn blocked<'a>(&'a self, _sender: &'a GlobalId, _recipient: &'a GlobalId) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }

    fn account_of<'a>(&'a self, _local: &'a str) -> BoxFuture<'a, Option<GlobalId>> {
        Box::pin(async { None })
    }

    fn presence<'a>(&'a self, _account: &'a GlobalId, _local: &'a str, _online: bool) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

pub fn default_jid(authcid: &str, domain: &str) -> Option<Jid> {
    Jid::new(authcid, domain, None).ok()
}

pub fn default_rules(kind: &RoomKind) -> RoomRules {
    RoomRules {
        may_join: true,
        may_speak: true,
        history: matches!(kind, RoomKind::Party(_) | RoomKind::Hunt(_)),
    }
}

pub fn is_unsupported(error: &ModuleError) -> bool {
    error.kind == ErrorKind::Unsupported
}

/// The loaded modules' drivers chat asks.
#[derive(Default, Clone)]
pub struct Drivers {
    drivers: BTreeMap<ModuleId, Arc<dyn ModuleDriver>>,
}

impl Drivers {
    pub fn insert(&mut self, id: ModuleId, driver: Arc<dyn ModuleDriver>) {
        self.drivers.insert(id, driver);
    }

    pub fn remove(&mut self, id: &ModuleId) {
        self.drivers.remove(id);
    }

    pub fn module_ids(&self) -> Vec<ModuleId> {
        self.drivers.keys().cloned().collect()
    }
}
