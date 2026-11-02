//! What a game module supplies on the wire (DD8): RU3c's hooks, one trait every area reaches through
//! [`crate::IslesVersion`], the auth middleware included. The 1.14.7 module answers them; the tests' fakes answer
//! the other side where a hook picks a code path.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::settings::Settings;

/// How a game server's call that names no player is resolved (`HasParadoxBackendAuth`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoPlayerPolicy {
    /// `INVALID` stays as it is and a bearer-less call has no user (empty data, 400 or `[]`)
    Off,
    /// `INVALID` path segments of GET and HEAD, and the user of a bearer-less call, become the configured
    /// account; an empty `characterId` or `accountId` resolves to that account's first character
    ConfiguredAccount,
}

impl NoPlayerPolicy {
    /// The account an `INVALID` path segment becomes: `GAMESERVER_NO_PLAYER_USER_ID ?? DEV_USER_ID ?? "mystpax"`.
    pub fn path_account(self, settings: &Settings) -> Option<String> {
        match self {
            Self::Off => None,
            Self::ConfiguredAccount => Some(
                settings
                    .raw("GAMESERVER_NO_PLAYER_USER_ID")
                    .or_else(|| settings.raw("DEV_USER_ID"))
                    .unwrap_or("mystpax")
                    .trim()
                    .to_owned(),
            ),
        }
    }

    /// The user of a bearer-less game server call: the same chain with no fallback, `None` when it is blank.
    pub fn bearerless_account(self, settings: &Settings) -> Option<String> {
        match self {
            Self::Off => None,
            Self::ConfiguredAccount => settings
                .raw("GAMESERVER_NO_PLAYER_USER_ID")
                .or_else(|| settings.raw("DEV_USER_ID"))
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_owned),
        }
    }
}

/// A fixed answer: a status and a JSON body, or none (Express's empty `send()`).
#[derive(Debug, Clone, PartialEq)]
pub struct StubReply {
    pub status: u16,
    pub body: Option<Value>,
}

/// One objective row as `/progression/objectives/:userId` stores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectiveRow {
    pub objective_id: String,
    pub value: i64,
    pub completed_count: i64,
    pub created_date: String,
    pub last_modified_date: String,
}

/// An entitlement as storage holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entitlement {
    pub name: String,
    pub duration: f64,
    pub activated_date: Option<String>,
}

/// `gameMode` in `/candidate/status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    Island,
    City,
    Shared,
}

impl GameMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Island => "ISLAND",
            Self::City => "CITY",
            Self::Shared => "SHARED",
        }
    }
}

/// Where `/candidate/status` sends the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    pub game_mode: GameMode,
    /// `null` on the wire when `None`
    pub hunt_id: Option<String>,
}

/// What `GET /progression/config` does to `progression_config.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressionConfigRule {
    /// The file as it is
    AsIs,
    /// 1.14.7 (`seasonWindow.ts`): the active pass on a rolling 12-week window, every other pass ended by
    /// 2024-12-03, `RECENT_HUNTPASS_SEASONS` added with 2099 windows; `GAMESERVER_WIDEN_HUNTPASS_WINDOWS=0` is `AsIs`
    RollingHuntpassWindows,
}

/// RU3c's hooks. The three booleans are code paths, not settings: where Node also has a switch, the boolean is that
/// switch's version default ([`crate::tables::VersionTables::switch_defaults`]).
pub trait VersionHooks: Send + Sync + 'static {
    fn no_player_policy(&self) -> NoPlayerPolicy;

    /// `PUT /gamesession/epic` (and 1.14.7's `/gamesession/epiceos`).
    fn game_session_payload(&self, token: &str) -> Value;

    /// Weapons served at `CurrentLevel` 0 (Reforged 0036: the 1.14.7 paks have no per-level power tables).
    fn weapons_at_base_level(&self, settings: &Settings) -> bool;

    /// Prestige only from an open reforge, banked restores, stale-reforge repair, the reforge routes.
    fn reforge_rules(&self) -> bool;

    /// A game server's report updates mastery and the answer carries it; without, `payload: {}`.
    fn mastery_persistence(&self) -> bool;

    /// `GET /huntpass/:userId`'s payload.
    fn huntpass_payload(&self, id: &str) -> Value;

    /// One row of `GET /progression/objectives/:userId`.
    fn objective_row(&self, user_id: &str, row: &ObjectiveRow) -> Value;

    fn progression_config(&self) -> ProgressionConfigRule;

    /// `GET /survey/config`.
    fn survey_config(&self) -> StubReply;

    /// `GET /entitlementsv2`.
    fn entitlements_payload(&self, list: &[Entitlement]) -> Value;

    /// `GET /candidate/status`'s destination for a candidate's hunt; `realm_hub` when a realm hub answers it.
    fn candidate_destination(&self, hunt_id: &str, realm_hub: bool) -> Destination;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_no_player_account_follows_nodes_chain() {
        let policy = NoPlayerPolicy::ConfiguredAccount;
        assert_eq!(policy.path_account(&Settings::default()).as_deref(), Some("mystpax"));
        assert_eq!(policy.bearerless_account(&Settings::default()), None);
        let dev = Settings::from_pairs([("DEV_USER_ID", " dev ")]);
        assert_eq!(policy.path_account(&dev).as_deref(), Some("dev"));
        // `??` keeps an empty value: it doesn't fall through to DEV_USER_ID
        let blank = Settings::from_pairs([("GAMESERVER_NO_PLAYER_USER_ID", ""), ("DEV_USER_ID", "dev")]);
        assert_eq!(policy.path_account(&blank).as_deref(), Some(""));
        assert_eq!(policy.bearerless_account(&blank), None);
        assert_eq!(NoPlayerPolicy::Off.path_account(&dev), None);
    }
}
