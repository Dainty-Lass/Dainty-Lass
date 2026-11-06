// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What a game version supplies (core spec §5.2, §5.3): its tables, the hooks areas ask, and its share of the
//! route table. The values are wire facts the version crate holds.

use std::sync::Arc;

use serde_json::Value;

use crate::routes::RouteSet;
use crate::settings::Settings;

/// A bounty item grant the client reads, valid between two dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BountyItemGrant {
    /// The item granted.
    pub item_id: &'static str,
    /// How many are granted.
    pub amount: i64,
    /// ISO texts
    pub start: &'static str,
    /// The end of the window, ISO text.
    pub end: &'static str,
}

/// The seasonal coin and its claim cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeasonalCoin {
    /// The coin's catalog id.
    pub catalog_id: &'static str,
    /// Most coins one claim grants.
    pub max_per_claim: i64,
}

/// A Slayer's Journey node hidden unless a switch is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaskedJourneyNode {
    /// The node's id.
    pub node: &'static str,
    /// The switch that unmasks it
    pub switch: &'static str,
}

/// What a new account starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StarterManifest {
    /// The starter version stored with existing rows; must stay byte for byte.
    pub bootstrap_version: &'static str,
    /// Items written to a new account's stacked items, in order.
    pub stacked_item_ids: &'static [&'static str],
    /// The quick-emoji wheel's texts (nine for 1.14.7)
    pub quick_emoji_wheel: &'static [&'static str],
}

/// A game version's wire facts (§5.2).
#[derive(Debug, Clone, Copy)]
pub struct VersionTables {
    /// The client's build changelist.
    pub target_changelist: u32,
    /// The client's game version text, such as `1.14.7`.
    pub game_version: &'static str,
    /// The Hunt Pass season the client runs.
    pub active_huntpass_id: &'static str,
    /// Recent Hunt Pass season ids.
    pub recent_huntpass_seasons: &'static [&'static str],
    /// The patch notes' release version; empty for 1.14.7.
    pub patchnotes_release_version: &'static str,
    /// The bounty item grants the client reads.
    pub bounty_item_grant_data: &'static [BountyItemGrant],
    /// Store section tags the client asks for.
    pub store_sections: &'static [&'static str],
    /// The seasonal coin, none when the version has none.
    pub seasonal_coin: Option<SeasonalCoin>,
    /// Journey nodes hidden behind switches.
    pub masked_journey_nodes: &'static [MaskedJourneyNode],
    /// What a new account starts with.
    pub starter_manifest: StarterManifest,
    /// `(switch key, default)`; a switch without one defaults to empty text
    pub switch_defaults: &'static [(&'static str, &'static str)],
    /// The `/gamesession/epic` payload for a session token (a plain function, as auth's config takes it)
    pub session_payload: fn(&str) -> Value,
}

impl VersionTables {
    /// `<changelist>_<game version>_shipping`
    pub fn build_id(&self) -> String {
        format!("{}_{}_shipping", self.target_changelist, self.game_version)
    }

    /// The default of a `[switches]` key, empty when it has none.
    pub fn switch_default(&self, key: &str) -> &'static str {
        self.switch_defaults
            .iter()
            .find(|(k, _)| *k == key)
            .map_or("", |(_, v)| v)
    }
}

/// Who a bearer-less game server call is, and what the `INVALID` sentinel becomes (`HasParadoxBackendAuth`'s
/// chain).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoPlayerPolicy {
    /// No account stands in for a bearer-less call.
    Off,
    /// `GAMESERVER_NO_PLAYER_USER_ID`, else `DEV_USER_ID`
    ConfiguredAccount,
}

impl NoPlayerPolicy {
    /// The `INVALID` sentinel's account: the first set of the two (a set-but-empty value is taken), else
    /// `mystpax`, trimmed.
    pub fn path_account(self, settings: &Settings) -> Option<String> {
        match self {
            Self::Off => None,
            Self::ConfiguredAccount => {
                let raw = settings
                    .raw("GAMESERVER_NO_PLAYER_USER_ID")
                    .or_else(|| settings.raw("DEV_USER_ID"))
                    .unwrap_or("mystpax");
                Some(raw.trim().to_owned())
            }
        }
    }

    /// The same two settings with no fallback; none when unset or blank.
    pub fn bearerless_account(self, settings: &Settings) -> Option<String> {
        match self {
            Self::Off => None,
            Self::ConfiguredAccount => settings
                .raw("GAMESERVER_NO_PLAYER_USER_ID")
                .or_else(|| settings.raw("DEV_USER_ID"))
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned),
        }
    }

    /// The same policy as the auth crate's type.
    pub fn for_auth(self) -> rustic_isles_auth::NoPlayerPolicy {
        match self {
            Self::Off => rustic_isles_auth::NoPlayerPolicy::Off,
            Self::ConfiguredAccount => rustic_isles_auth::NoPlayerPolicy::ConfiguredAccount,
        }
    }
}

/// A canned answer: a status and maybe a JSON body.
#[derive(Debug, Clone, PartialEq)]
pub struct StubReply {
    /// The HTTP status.
    pub status: u16,
    /// The JSON body; none answers the bare status.
    pub body: Option<Value>,
}

/// One objective's progress as the store holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectiveRow {
    /// The objective's id.
    pub objective_id: String,
    /// Progress toward the objective.
    pub value: f64,
    /// Times completed.
    pub completed_count: f64,
    /// Created, ISO text.
    pub created: String,
    /// Last modified, ISO text.
    pub last_modified: String,
}

/// An entitlement as the store holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entitlement {
    /// Its name.
    pub name: String,
    /// How long it lasts.
    pub duration: f64,
    /// When it was activated, ISO text; none when it was not.
    pub activated: Option<String>,
}

/// How the client travels to a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameMode {
    /// `ISLAND`: a hunt.
    Island,
    /// `CITY`: Ramsgate.
    City,
    /// `SHARED`: a shared hub, such as a realm's Training Grounds.
    Shared,
}

impl GameMode {
    /// The mode's wire text.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Island => "ISLAND",
            Self::City => "CITY",
            Self::Shared => "SHARED",
        }
    }
}

/// Where a matchmaking candidate sends the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    /// How the client travels.
    pub game_mode: GameMode,
    /// The hunt id the client gets; none for Ramsgate.
    pub hunt_id: Option<String>,
}

/// How `GET /progression/config` treats the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressionConfigRule {
    /// Served as it is.
    AsIs,
    /// Hunt Pass windows widened so recent seasons stay open (§8.4).
    RollingHuntpassWindows,
}

/// The areas ask, the version answers.
pub trait VersionHooks: Send + Sync {
    /// Who a bearer-less game server call is (§5.3).
    fn no_player_policy(&self) -> NoPlayerPolicy;
    /// Whether weapons are reset to their base level.
    fn weapons_at_base_level(&self, settings: &Settings) -> bool;
    /// Whether the reforge rules apply.
    fn reforge_rules(&self) -> bool;
    /// Whether a game server's mastery report may be saved.
    fn mastery_persistence(&self) -> bool;
    /// The Hunt Pass payload for a season id.
    fn huntpass_payload(&self, id: &str) -> Value;
    /// One objective's row as the client reads it.
    fn objective_row(&self, user_id: &str, row: &ObjectiveRow) -> Value;
    /// How the progression config is served.
    fn progression_config(&self) -> ProgressionConfigRule;
    /// The survey config answer.
    fn survey_config(&self) -> StubReply;
    /// The entitlements answer.
    fn entitlements_payload(&self, list: &[Entitlement]) -> Value;
    /// Where a candidate for this hunt sends the client; `realm_hub` when it is a realm's hub.
    fn candidate_destination(&self, hunt_id: &str, realm_hub: bool) -> Destination;
}

/// A game version as areas see it: hooks, tables and route set.
#[derive(Clone)]
pub struct IslesVersion {
    /// The version's answers to the areas.
    pub hooks: Arc<dyn VersionHooks>,
    /// The version's wire facts.
    pub tables: &'static VersionTables,
    /// The version's share of the route table.
    pub routes: RouteSet,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_player_accounts() {
        let p = NoPlayerPolicy::ConfiguredAccount;
        let s = Settings::default();
        assert_eq!(
            (p.path_account(&s), p.bearerless_account(&s)),
            (Some("mystpax".into()), None)
        );
        let s = Settings::from_pairs([("DEV_USER_ID", " dev ")]);
        assert_eq!(
            (p.path_account(&s), p.bearerless_account(&s)),
            (Some("dev".into()), Some("dev".into()))
        );
        let s = Settings::from_pairs([("GAMESERVER_NO_PLAYER_USER_ID", ""), ("DEV_USER_ID", "dev")]);
        assert_eq!(
            (p.path_account(&s), p.bearerless_account(&s)),
            (Some(String::new()), None)
        );
        let s = Settings::from_pairs([("DEV_USER_ID", "dev")]);
        let off = NoPlayerPolicy::Off;
        assert_eq!((off.path_account(&s), off.bearerless_account(&s)), (None, None));
    }
}
