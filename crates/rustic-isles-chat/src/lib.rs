// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Isles' chat (RD27, written clean-room from `docs/port/spec-rustic-isles-chat.md`): the answers a 1.14.7
//! module gives the chat plugin through `dainty_module_api::chat`, the hub-session registry that opens region rooms, the
//! party headline body, the `chat.*` home operations, and the chat area's 12 backend routes. It never depends on the
//! `chat` plugin crate: a game module talks to chat only through `module-api`.

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")
)]
#![allow(
    clippy::result_large_err,
    reason = "a refusal is the answer itself (an axum `Response`), as in core and rustic-isles-auth, so it is not boxed"
)]

pub mod headline;
mod home;
pub mod hubs;
pub mod ids;
mod login;
pub mod reset;
mod routes;
mod rules;
mod source;

use dainty_module_api::{BoxFuture, GlobalId, ModuleError, NodeIdentity};
use rustic_isles::Settings;
use serde_json::{Map, Value};

pub use dainty_module_api::chat::{ChatAnswer, ChatQuery, Jid, RoomKind, RoomRules};
pub use home::home_operation;
pub use hubs::{HUB_SESSION_TTL_MS, HubSessions, MAX_ACCOUNTS};
pub use login::{Refusal, authenticate};
pub use routes::{
    Clock, SLAYER_LINK_MAX_ACCOUNTS, include_pending, register, register_with, slayer_link_availability,
    slayer_link_status,
};
pub use rules::{ChatRules, IslesChat, room_kind_of};
pub use source::StoreChat;

/// The captured logical XMPP domain.
pub const XMPP_DOMAIN: &str = "prod.ol.epicgames.com";
/// Who party headlines come from.
pub const HEADLINE_FROM: &str = "party-events@prod.ol.epicgames.com";
/// The launcher client-credentials grant's userId (auth §3.2): never a chat player.
pub const CLIENT_CREDENTIALS_USER_ID: &str = "__launcher_client_credentials__";

/// The two dev switches (spec §8), read once when the source is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatSettings {
    /// Every region room open to anyone
    pub dev_city_muc: bool,
    /// Every hunt room open to anyone (no account-to-hunt mapping exists)
    pub dev_hunt_muc: bool,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            dev_city_muc: false,
            dev_hunt_muc: true,
        }
    }
}

impl ChatSettings {
    /// City on only for `true` (RU3c; Node's was on unless `false`), hunt off only for `false` (Node's).
    pub fn from_settings(settings: &Settings) -> Self {
        let read = |name: &str| settings.raw(name).map(dainty_service_kit::js::trim);
        Self {
            dev_city_muc: read("REALTIME_XMPP_DEV_CITY_MUC") == Some("true"),
            dev_hunt_muc: read("REALTIME_XMPP_DEV_HUNT_MUC") != Some("false"),
        }
    }
}

/// A chat player: the account, the userId the client sees, and `accounts.name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    /// The account the player logged in as.
    pub account: GlobalId,
    /// The game token's `userId`.
    pub user_id: String,
    /// The display name, from `accounts.name`.
    pub name: String,
}

/// A game token that did not verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidToken;

/// The game token check (auth §5.1).
pub trait TokenCheck: Send + Sync {
    /// The token's claims when it is a valid game token.
    fn validate(&self, token: &str) -> Result<Map<String, Value>, InvalidToken>;
}

/// What the rules read (spec §5.1). Every lookup answers `Unsupported` without a database, `Invalid` for an id of
/// another kind where an account is needed and `Unavailable` for a store failure.
pub trait ChatSource: Send + Sync {
    /// The node this module serves, which names its rooms and JIDs.
    fn node(&self) -> &NodeIdentity;

    /// The `REALTIME_XMPP_*` switches.
    fn settings(&self) -> ChatSettings;

    /// None without a store.
    fn token_check(&self) -> Option<&dyn TokenCheck>;

    /// The live game account of that id, or the account's home's answer when it lives elsewhere.
    fn player_by_account<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<Player>, ModuleError>>;

    /// The live game account with that userId, when its launcher account (if any) is eligible for play.
    fn player_by_user_id<'a>(&'a self, user_id: &'a str) -> BoxFuture<'a, Result<Option<Player>, ModuleError>>;

    /// A JID local part's account (§5.8): a userId, else an eligible launcher account's normalized name.
    fn account_of<'a>(&'a self, local: &'a str) -> BoxFuture<'a, Result<Option<GlobalId>, ModuleError>>;

    /// A player homed on another node signing in here (§5.5); errors are none.
    fn cross_node_login<'a>(&'a self, authcid: &'a str, token: &'a str) -> BoxFuture<'a, Option<Player>>;

    /// The client-facing id of the account's party.
    fn party_of<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<String>, ModuleError>>;

    /// The other ends of the account's `ACCEPTED` edges, in list order.
    fn friends<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Vec<GlobalId>, ModuleError>>;

    /// One step of the account's `/reset` at `now`; `Unsupported` where saves can't be reset.
    fn save_reset<'a>(
        &'a self,
        _account: &'a GlobalId,
        _step: reset::ResetStep,
        _now: i64,
    ) -> BoxFuture<'a, Result<reset::ResetAnswer, ModuleError>> {
        Box::pin(async {
            Err(ModuleError::new(
                dainty_module_api::ErrorKind::Unsupported,
                "no save resets",
            ))
        })
    }
}
