//! Rustic Isles' chat rules (DD21): RU5's chat area is the rules DaintyLass's chat plugin (DN10a) asks a game module
//! for, not a chat of its own. The plugin owns the XMPP connections, rooms, whispers and presence; this crate says
//! what Mystic Paradox's `realtime/` decided that the plugin cannot know:
//!
//! - [`jid`](ChatRules::jid): a player's JID is their `userId` on the captured domain (`XMPPAuth.ts`: the SASL authcid
//!   must be the token's `userId`, so the plugin checks the JID's local part against it).
//! - [`room`](ChatRules::room): who may join a room (`RoomService.ts#authorizeJoin`): a party's members, the players
//!   matchmaking sent to a hub in a region (patch 0025, [`HubSessions`]), dev switches for the rest.
//! - [`login::authenticate`]: `authenticateSasl`'s checks on the token and the account, for the `Login` query DN10a
//!   asks DF2 for.
//! - [`headline`]: the JSON of a `party-events@` headline (`PartyOutboxDispatcher.ts`), byte for byte.
//!
//! Everything the rules read about the game goes through [`ChatSource`], which `isles-core` implements over its store.

#[cfg(test)]
mod fake;
pub mod headline;
pub mod hubs;
pub mod ids;
pub mod login;
pub mod rooms;
pub mod source;

use module_api::chat::{ChatAnswer, ChatQuery, Jid, RoomKind, RoomRules};
use module_api::{BoxFuture, ErrorKind, GlobalId, ModuleError};

pub use hubs::{HubSessions, hub_sessions};
pub use source::{ChatSettings, ChatSource, Player};

/// The XMPP domain every JID has: Paradox's captured logical domain (`XMPP_DOMAIN`), which is also the plugin's default.
pub const XMPP_DOMAIN: &str = "prod.ol.epicgames.com";

/// The chat plugin's questions, per version.
pub trait ChatRules: Send + Sync + 'static {
    /// The JID a player has in this game.
    fn jid<'a>(&'a self, source: &'a dyn ChatSource, account: &'a GlobalId) -> BoxFuture<'a, Result<Jid, ModuleError>>;

    /// What a player may do in a room.
    fn room<'a>(
        &'a self,
        source: &'a dyn ChatSource,
        room: &'a RoomKind,
        account: &'a GlobalId,
    ) -> BoxFuture<'a, Result<RoomRules, ModuleError>>;

    /// Validates a SASL login and resolves the player.
    fn login<'a>(
        &'a self,
        source: &'a dyn ChatSource,
        authcid: &'a str,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Player, ModuleError>> {
        Box::pin(async move {
            let Some(verifier) = source.verifier() else {
                return Err(ModuleError::unsupported("chat login without a token verifier"));
            };
            match login::authenticate(authcid, token, verifier, source).await {
                Ok(player) => Ok(player),
                Err(login::Refusal::Unavailable) => Err(ModuleError::new(
                    ErrorKind::Unavailable,
                    login::Refusal::Unavailable.as_str(),
                )),
                Err(refusal) => {
                    if let Ok(Some(player)) = source.foreign_login(authcid, token).await {
                        return Ok(player);
                    }
                    Err(ModuleError::new(ErrorKind::Invalid, refusal.as_str()))
                }
            }
        })
    }

    /// Accepted friends of `account`.
    fn friends<'a>(
        &'a self,
        source: &'a dyn ChatSource,
        account: &'a GlobalId,
    ) -> BoxFuture<'a, Result<Vec<GlobalId>, ModuleError>> {
        source.friends(account)
    }

    /// Whether `from` may whisper `to`.
    fn may_whisper<'a>(
        &'a self,
        _source: &'a dyn ChatSource,
        _from: &'a GlobalId,
        _to: &'a GlobalId,
    ) -> BoxFuture<'a, Result<bool, ModuleError>> {
        Box::pin(async { Ok(true) })
    }

    /// Which [`RoomKind`] a MUC room's local part names.
    fn room_of<'a>(
        &'a self,
        source: &'a dyn ChatSource,
        name: &'a str,
    ) -> BoxFuture<'a, Result<Option<RoomKind>, ModuleError>> {
        Box::pin(async move { Ok(rooms::room_kind_of(source, hub_sessions(), rooms::now_millis(), name)) })
    }

    /// One query, as `GameModule::chat` receives it.
    fn answer<'a>(
        &'a self,
        source: &'a dyn ChatSource,
        query: &'a ChatQuery,
    ) -> BoxFuture<'a, Result<ChatAnswer, ModuleError>> {
        Box::pin(async move {
            match query {
                ChatQuery::Jid { account } => self.jid(source, account).await.map(ChatAnswer::Jid),
                ChatQuery::Room { room, account } => self.room(source, room, account).await.map(ChatAnswer::Room),
                ChatQuery::Login { authcid, token } => {
                    self.login(source, authcid, token)
                        .await
                        .map(|player| ChatAnswer::Login {
                            account: player.account,
                            name: player.name,
                        })
                }
                ChatQuery::Friends { account } => self.friends(source, account).await.map(ChatAnswer::Friends),
                ChatQuery::MayWhisper { from, to } => {
                    self.may_whisper(source, from, to).await.map(ChatAnswer::MayWhisper)
                }
                ChatQuery::RoomOf { name } => self.room_of(source, name).await.map(ChatAnswer::RoomOf),
                ChatQuery::AccountOf { local } => source.account_of(local).await.map(ChatAnswer::AccountOf),
            }
        })
    }
}

/// 1.14.7's rules.
#[derive(Debug, Clone, Copy)]
pub struct IslesChat;

impl ChatRules for IslesChat {
    fn jid<'a>(&'a self, source: &'a dyn ChatSource, account: &'a GlobalId) -> BoxFuture<'a, Result<Jid, ModuleError>> {
        Box::pin(rooms::jid_of(source, account))
    }

    fn room<'a>(
        &'a self,
        source: &'a dyn ChatSource,
        room: &'a RoomKind,
        account: &'a GlobalId,
    ) -> BoxFuture<'a, Result<RoomRules, ModuleError>> {
        Box::pin(async move { rooms::rules_of(source, hub_sessions(), rooms::now_millis(), room, account).await })
    }
}
