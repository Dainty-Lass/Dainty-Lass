//! RU3c's tables: the same code, a per-version value (DD8; "what RU4's seams must expose", 3 and 6). Each version
//! module fills one [`VersionTables`]; the game data root and the Director's tables are [`crate::game_data`]'s and
//! `isles-director`'s.

use crate::settings::SharedSwitch;

/// One `item_grant_data` window of `/game_tuning/bounty_game_data` (`tuning.ts`'s `BountyGrantWindows`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemGrantWindow {
    pub item_id: &'static str,
    pub amount: u32,
    /// ISO 8601, as `Date.toISOString()` writes it
    pub start: &'static str,
    pub end: &'static str,
}

/// The season-challenge coin (`POST /inventory`, the Reward Cache, prestige payouts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeasonalCoin {
    pub catalog_id: &'static str,
    /// The most one season-challenge claim grants
    pub max_per_claim: u32,
}

/// A Slayer's Path node served `Locked` while its switch is off; the stored status is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaskedJourneyNode {
    pub node: &'static str,
    /// The switch that unmasks it
    pub unless: &'static str,
}

/// What a new account is seeded with (`controllers/starterManifest.ts`). The instanced items and the loadout are
/// fixed; these are the module's parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StarterManifest {
    /// `BOOTSTRAP_VERSION`: which seeding an account got, so a newer one reconciles it
    pub bootstrap_version: &'static str,
    /// One of each
    pub stacked: &'static [&'static str],
    /// The default quick-emoji wheel (`DEFAULT_QUICK_EMOJIS`), when the version's catalog has the emojis
    pub quick_emojis: Option<&'static [&'static str]>,
}

/// The version's default for a shared switch; the host's setting wins ([`crate::settings`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchDefault {
    pub switch: SharedSwitch,
    pub value: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionTables {
    /// `TARGET_CHANGELIST`: oauth, the launcher's game sessions, status and runtime feeds, `LastChangelist`,
    /// `buildId`, the Director's runtime updater
    pub target_changelist: u32,
    /// `GAME_VERSION`: `serverInfo.buildId` is `<changelist>_<version>_shipping`
    pub game_version: &'static str,
    /// `ACTIVE_HUNTPASS_ID` (`GAMESERVER_HUNTPASS_ID` overrides it on 1.14.7)
    pub active_huntpass_id: &'static str,
    /// `RECENT_HUNTPASS_SEASONS` (`GAMESERVER_RECENT_SEASONS` overrides it)
    pub recent_huntpass_seasons: &'static [&'static str],
    /// `release_version` of `/patchnotes`
    pub patchnotes_release_version: &'static str,
    pub bounty_item_grant_data: &'static [ItemGrantWindow],
    /// The store sections served (`/product/skus/public`, `/token`, `/notification`), as RU3c lists them
    pub store_sections: &'static [&'static str],
    pub seasonal_coin: Option<SeasonalCoin>,
    pub masked_journey_nodes: &'static [MaskedJourneyNode],
    pub starter_manifest: StarterManifest,
    /// `CONFIG_DEFAULTS` and the switches whose default follows a hook
    pub switch_defaults: &'static [SwitchDefault],
}

impl VersionTables {
    /// `serverInfo.buildId`.
    pub fn build_id(&self) -> String {
        format!("{}_{}_shipping", self.target_changelist, self.game_version)
    }

    pub fn switch_default(&self, switch: SharedSwitch) -> Option<&'static str> {
        self.switch_defaults
            .iter()
            .find(|default| default.switch == switch)
            .map(|default| default.value)
    }
}
