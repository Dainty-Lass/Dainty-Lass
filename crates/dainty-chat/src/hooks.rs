// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! New here (DD21, DN10): what chat asks the rest of the server. The controller (DF3) implements [`ChatHost`]:
//! who a login is (the game module checks its own token), and each game module's JID and room rules through
//! `GameModule::chat`. DR's chat asked the metagame's own tables for the same (`chat/mod.rs`'s `auth`,
//! `muc.rs`'s `may_use`).
//!
//! A module that doesn't answer chat (no chat capability, or `GameModule::chat`'s default `unsupported`)
//! gets [`default_jid`] and [`default_rules`]; any other error refuses, so a module that is down never opens
//! a room it would have closed.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use dainty_module_api::chat::{ChatAnswer, ChatQuery, Jid, RoomKind, RoomRules};
use dainty_module_api::{BoxFuture, ErrorKind, GlobalId, ModuleDriver, ModuleDriverExt, ModuleError, ModuleId};

/// A SASL PLAIN login, as the client sent it.
#[derive(Debug, Clone, Copy)]
pub struct Login<'a> {
    /// The account's name in its game: the JID local part the client will bind
    pub authcid: &'a str,
    /// The game's own token (1.14.7: the game JWT; 1.4.4: the metagame token). Never logged.
    pub token: &'a str,
    /// The XMPP domain the client opened the stream with
    pub domain: &'a str,
}

/// Who a login is, once its game module has checked the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    /// The player's global account.
    pub account: GlobalId,
    /// The game module the token belongs to, which answers this player's chat queries
    pub module: ModuleId,
    /// The account's display name now: room nicknames must carry it (`nick.rs`)
    pub name: String,
}

/// Why a login was refused; logged, never told to the client beyond SASL's `not-authorized`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginRefusal {
    /// The token's time ran out.
    Expired,
    /// The token is not one the module issued.
    BadToken,
    /// The token is good but names no account.
    NoAccount,
    /// The module could not be asked; `temporary-auth-failure`
    Unavailable,
}

impl LoginRefusal {
    /// The name logged for it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::BadToken => "bad-token",
            Self::NoAccount => "no-account",
            Self::Unavailable => "unavailable",
        }
    }
}

/// What chat needs from the host: logins, module queries, blocks and presence.
pub trait ChatHost: Send + Sync {
    /// Checks a login's token with the game it belongs to.
    fn login<'a>(&'a self, login: Login<'a>) -> BoxFuture<'a, Result<Player, LoginRefusal>>;

    /// A chat query to one game module (`GameModule::chat`).
    fn ask<'a>(&'a self, module: &'a ModuleId, query: ChatQuery) -> BoxFuture<'a, Result<ChatAnswer, ModuleError>>;

    /// Either player blocked the other: no direct message between them, and no room message from `sender`
    /// reaches `recipient` (DR: `blocked_either_way`, `blockers_among`).
    fn blocked<'a>(&'a self, _sender: &'a GlobalId, _recipient: &'a GlobalId) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }

    /// The account a JID local part belongs to when known to the host (for example from a directory or game
    /// database before the player has logged into chat).
    fn account_of<'a>(&'a self, _local: &'a str) -> BoxFuture<'a, Option<GlobalId>> {
        Box::pin(async { None })
    }

    /// Called when an account's local online presence changes on this node (`online = true` when its first
    /// session binds, `online = false` when its last session closes).
    fn presence<'a>(&'a self, _account: &'a GlobalId, _local: &'a str, _online: bool) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// The JID of a module that doesn't answer: the name the client logged in with, on its own domain.
pub fn default_jid(authcid: &str, domain: &str) -> Option<Jid> {
    Jid::new(authcid, domain, None).ok()
}

/// The rules of a module that doesn't answer: every signed-in player may join and speak in every room, as
/// DR's General and City rooms and Mystic Paradox's dev Hunt rooms are; history is kept for party and hunt
/// rooms only, as the stub module keeps it. A party or hunt room's name is its global id, which can't be
/// guessed (80 random bits).
pub fn default_rules(room: &RoomKind) -> RoomRules {
    RoomRules {
        may_join: true,
        may_speak: true,
        history: matches!(room, RoomKind::Party(_) | RoomKind::Hunt(_)),
    }
}

/// Unsupported means "use the default"; anything else is a refusal.
pub fn is_unsupported(error: &ModuleError) -> bool {
    error.kind == ErrorKind::Unsupported
}

/// Game modules' drivers by id, asked through the module protocol: the [`ChatHost::ask`] half a host can
/// use as it is. A module not held here answers unsupported.
#[derive(Default)]
pub struct Drivers {
    drivers: Mutex<BTreeMap<ModuleId, Arc<dyn ModuleDriver>>>,
}

impl Drivers {
    /// Adds a module's driver, replacing one with the same id.
    pub fn insert(&self, driver: Arc<dyn ModuleDriver>) {
        let id = driver.manifest().id.clone();
        self.drivers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, driver);
    }

    /// Drops a module's driver.
    pub fn remove(&self, module: &ModuleId) {
        self.drivers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(module);
    }

    /// The ids of the modules held.
    pub fn module_ids(&self) -> Vec<ModuleId> {
        self.drivers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    /// Asks a module a chat query; a module not held answers unsupported.
    pub async fn ask(&self, module: &ModuleId, query: ChatQuery) -> Result<ChatAnswer, ModuleError> {
        let driver = self
            .drivers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(module)
            .cloned();
        match driver {
            Some(driver) => driver.chat(query).await,
            None => Err(ModuleError::unsupported("chat")),
        }
    }
}
