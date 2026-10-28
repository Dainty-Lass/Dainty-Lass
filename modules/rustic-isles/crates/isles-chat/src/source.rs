//! What the rules read about the game. `isles-core` implements it over its store; the tests over a table in memory.

use module_api::{BoxFuture, GlobalId, ModuleError, NodeIdentity};

/// A player as chat knows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    pub account: GlobalId,
    /// Node's `userId`: what the client calls the account, and the JID's local part
    pub user_id: String,
    /// The display name
    pub name: String,
}

/// The two dev switches of `RoomService.ts`, read from the module's env file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatSettings {
    /// `REALTIME_XMPP_DEV_CITY_MUC`: any signed-in player may join any region room. Off unless `true`: RU3c's
    /// default, where Node's own was on unless `false` and production had to switch it off (`runtimeSafety.ts`).
    pub dev_city_muc: bool,
    /// `REALTIME_XMPP_DEV_HUNT_MUC`: any signed-in player may join any Hunt room. On unless `false`, as Node's: no
    /// account-to-hunt mapping exists to check a Hunt room against.
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

pub trait ChatSource: Send + Sync {
    fn node(&self) -> &NodeIdentity;

    fn settings(&self) -> ChatSettings;

    /// What the game's token check says about a token (`TokenVerifier`), or `None` when not configured.
    fn verifier(&self) -> Option<&dyn crate::login::TokenVerifier> {
        None
    }

    /// The account's player, `None` for one that does not exist.
    fn player<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<Player>, ModuleError>>;

    /// The account a `userId` names, `None` unless it exists and may sign in (`IsAccountEligible`).
    fn player_by_user<'a>(&'a self, user_id: &'a str) -> BoxFuture<'a, Result<Option<Player>, ModuleError>>;

    /// The party the account is in (`GetPartyForPlayer(..).partyId`), as the party area writes the id.
    fn party_of<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<String>, ModuleError>>;

    /// The account's accepted friends (`PresenceService`'s `acceptedFriendIds`).
    fn friends<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Vec<GlobalId>, ModuleError>>;
}
