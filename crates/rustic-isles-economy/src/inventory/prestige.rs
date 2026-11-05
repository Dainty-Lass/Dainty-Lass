// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Prestige sources (spec §9.3): which reward currencies a prestige track pays per level, read from
//! `progression_config.json`, so the prestige gate can hold a game server to the levels it banked.

use std::sync::Arc;

use rustic_isles::Shared;
use serde_json::Value;

use crate::util::num;

pub const PROGRESSION_CONFIG_FILE: &str = "progression_config.json";

/// One track paying `reward_per_level` of a catalog id per prestige level past `cap`.
#[derive(Debug, Clone, PartialEq)]
pub struct PrestigeRewardSource {
    pub catalog_id: String,
    pub progression_id: String,
    pub xp_per_level: f64,
    pub reward_per_level: f64,
    pub cap: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProgressionConfig {
    sources: Vec<PrestigeRewardSource>,
}

fn finite_or_zero(n: f64) -> f64 {
    if n.is_finite() { n } else { 0.0 }
}

impl ProgressionConfig {
    /// `{"payload":{"paths":[..]}}`: every path with a prestige level size and its free stacked rewards.
    pub fn from_document(document: &Value) -> Self {
        let mut sources = Vec::new();
        let paths = document
            .get("payload")
            .and_then(|p| p.get("paths"))
            .and_then(Value::as_array);
        for path in paths.into_iter().flatten() {
            let Some(id) = path
                .get("progression_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let prestige = path.get("prestige");
            let per_level = num(prestige.and_then(|p| p.get("xp_per_level")));
            if !(per_level.is_finite() && per_level > 0.0) {
                continue;
            }
            let cap: f64 = path
                .get("requirements")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|r| finite_or_zero(num(r.get("xp_required"))))
                .sum();
            let rewards = prestige
                .and_then(|p| p.get("free_rewards"))
                .and_then(|r| r.get("stacked_items"))
                .and_then(Value::as_array);
            for reward in rewards.into_iter().flatten() {
                let Some(catalog_id) = reward.get("catalog_id").and_then(Value::as_str) else {
                    continue;
                };
                let quantity = num(reward.get("quantity"));
                if quantity.is_finite() && quantity > 0.0 {
                    sources.push(PrestigeRewardSource {
                        catalog_id: catalog_id.to_owned(),
                        progression_id: id.to_owned(),
                        xp_per_level: per_level,
                        reward_per_level: quantity,
                        cap,
                    });
                }
            }
        }
        Self { sources }
    }

    /// Loaded once and kept for the module's life; a failed load answers "no sources" and is tried again.
    pub fn load(shared: &Shared) -> Arc<ProgressionConfig> {
        crate::caches::cached(
            shared,
            "inventory.prestige",
            || match crate::inventory::storage_policy::read_game_data(shared, PROGRESSION_CONFIG_FILE) {
                Ok(document) => (Arc::new(Self::from_document(&document)), true),
                Err(_) => (Arc::new(Self::default()), false),
            },
        )
    }

    pub fn is_prestige_gated_reward(&self, catalog_id: &str) -> bool {
        self.sources.iter().any(|s| s.catalog_id == catalog_id)
    }

    pub fn prestige_sources(&self, catalog_id: &str) -> Vec<&PrestigeRewardSource> {
        self.sources.iter().filter(|s| s.catalog_id == catalog_id).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

/// `IsPrestigeGatedReward` over the module's config.
pub fn is_prestige_gated_reward(shared: &Shared, catalog_id: &str) -> bool {
    ProgressionConfig::load(shared).is_prestige_gated_reward(catalog_id)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn sources() {
        let config = ProgressionConfig::from_document(&json!({"payload": {"paths": [
            {"progression_id": "ExperienceTrack_Weapon_Axe", "requirements": [{"xp_required": 100}, {"xp_required": "x"}, {"xp_required": 50}],
             "prestige": {"xp_per_level": 1000, "free_rewards": {"stacked_items": [{"catalog_id": "CURRENCY_PJM_PRESTIGE_FILLED", "quantity": 1}, {"catalog_id": "Z", "quantity": 0}]}}},
            {"progression_id": "ExperienceTrack_PlayerLevel", "requirements": [{"xp_required": 10}]},
            {"progression_id": "", "prestige": {"xp_per_level": 5}},
        ]}}));
        assert!(config.is_prestige_gated_reward("CURRENCY_PJM_PRESTIGE_FILLED"));
        assert!(!config.is_prestige_gated_reward("CURRENCY_PJM_PRESTIGE_EMPTY"));
        assert!(!config.is_prestige_gated_reward("Z"));
        let s = &config.prestige_sources("CURRENCY_PJM_PRESTIGE_FILLED")[0];
        assert_eq!((s.cap, s.xp_per_level, s.reward_per_level), (150.0, 1000.0, 1.0));
    }
}
