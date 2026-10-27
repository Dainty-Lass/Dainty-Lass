//! Rustic Isles' chat rules (DD21): RU5's chat area became the rules DaintyLass's chat plugin (DN10a) asks a game
//! module for, instead of a chat of its own. Hooks only until DN10a lands: every answer is "unsupported", and the
//! modules don't declare the chat capability yet.
//!
//! What the rules will carry, from `realtime/` (RU3c, Cross-cutting: all `same` in both versions): a player's JID
//! from their `userId`, City rooms authorised by the `gameSessionId` `/candidate/status` serves, party events as
//! headlines from `party-events@`, an offline presence leaving the party, `REALTIME_XMPP_DEV_CITY_MUC` off. Paradox's
//! City rooms become DD21's per-region rooms.

use module_api::chat::{ChatAnswer, ChatQuery, Jid, RoomKind, RoomRules};
use module_api::{GlobalId, ModuleError};

/// The chat plugin's questions, per version.
pub trait ChatRules: Send + Sync + 'static {
    /// The JID a player has in this game.
    fn jid(&self, account: &GlobalId) -> Result<Jid, ModuleError>;

    /// What a player may do in a room.
    fn room(&self, room: &RoomKind, account: &GlobalId) -> Result<RoomRules, ModuleError>;

    /// One query, as `GameModule::chat` receives it.
    fn answer(&self, query: &ChatQuery) -> Result<ChatAnswer, ModuleError> {
        match query {
            ChatQuery::Jid { account } => self.jid(account).map(ChatAnswer::Jid),
            ChatQuery::Room { room, account } => self.room(room, account).map(ChatAnswer::Room),
        }
    }
}

/// Which version's rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatVersion {
    V1147,
    V1120,
}

/// The rules before DN10a: every question unsupported.
#[derive(Debug, Clone, Copy)]
pub struct IslesChat {
    pub version: ChatVersion,
}

impl ChatRules for IslesChat {
    fn jid(&self, _: &GlobalId) -> Result<Jid, ModuleError> {
        Err(ModuleError::unsupported("chat (DN10a)"))
    }

    fn room(&self, _: &RoomKind, _: &GlobalId) -> Result<RoomRules, ModuleError> {
        Err(ModuleError::unsupported("chat (DN10a)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use module_api::{ErrorKind, IdKind, NodeIdentity};

    #[test]
    fn every_question_is_unsupported_until_the_plugin_lands() {
        let account = GlobalId::new(IdKind::Account, &NodeIdentity::standalone().node);
        for version in [ChatVersion::V1147, ChatVersion::V1120] {
            let rules = IslesChat { version };
            let error = rules
                .answer(&ChatQuery::Jid {
                    account: account.clone(),
                })
                .unwrap_err();
            assert_eq!(error.kind, ErrorKind::Unsupported);
        }
    }
}
