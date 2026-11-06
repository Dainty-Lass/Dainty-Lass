// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What a game version supplies (core spec §5.2, §5.3): its tables, the hooks areas ask, and its share of the
//! route table. The values are wire facts the version crate holds.

use std::sync::Arc;

use serde_json::Value;

use crate::routes::RouteSet;
use crate::settings::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BountyItemGrant {
    pub item_id: &'static str,
    pub amount: i64,
    /// ISO texts
    pub start: &'static str,
    pub end: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeasonalCoin {
    pub catalog_id: &'static str,
    pub max_per_claim: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaskedJourneyNode {
    pub node: &'static str,
    /// The switch that unmasks it
    pub switch: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StarterManifest {
    pub bootstrap_version: &'static str,
    pub stacked_item_ids: &'static [&'static str],
    /// The quick-emoji wheel's texts (nine for 1.14.7)
    pub quick_emoji_wheel: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub struct VersionTables {
    pub target_changelist: u32,
    pub game_version: &'static str,
    pub active_huntpass_id: &'static str,
    pub recent_huntpass_seasons: &'static [&'static str],
    pub patchnotes_release_version: &'static str,
    pub bounty_item_grant_data: &'static [BountyItemGrant],
    pub store_sections: &'static [&'static str],
    pub seasonal_coin: Option<SeasonalCoin>,
    pub masked_journey_nodes: &'static [MaskedJourneyNode],
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
    pub status: u16,
    pub body: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectiveRow {
    pub objective_id: String,
    pub value: f64,
    pub completed_count: f64,
    pub created: String,
    pub last_modified: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entitlement {
    pub name: String,
    pub duration: f64,
    pub activated: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    pub game_mode: GameMode,
    pub hunt_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressionConfigRule {
    AsIs,
    RollingHuntpassWindows,
}

/// The areas ask, the version answers.
pub trait VersionHooks: Send + Sync {
    fn no_player_policy(&self) -> NoPlayerPolicy;
    fn weapons_at_base_level(&self, settings: &Settings) -> bool;
    fn reforge_rules(&self) -> bool;
    fn mastery_persistence(&self) -> bool;
    fn huntpass_payload(&self, id: &str) -> Value;
    fn objective_row(&self, user_id: &str, row: &ObjectiveRow) -> Value;
    fn progression_config(&self) -> ProgressionConfigRule;
    fn survey_config(&self) -> StubReply;
    fn entitlements_payload(&self, list: &[Entitlement]) -> Value;
    fn candidate_destination(&self, hunt_id: &str, realm_hub: bool) -> Destination;
}

#[derive(Clone)]
pub struct IslesVersion {
    pub hooks: Arc<dyn VersionHooks>,
    pub tables: &'static VersionTables,
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
