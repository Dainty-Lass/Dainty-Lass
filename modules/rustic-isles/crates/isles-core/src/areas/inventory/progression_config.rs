//! The two things this area reads from `progression_config.json` (the file `GET /progression/config` serves): a
//! track's level-20 cap, which a reforge resets at, and which reward currencies are prestige-gated. The rank curves
//! themselves are the progression area's (`controllers/progression.ts`'s `TrackCurves`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;
use service_kit::js;

use super::storage_policy::{PolicyError, read_game_data};

#[derive(Debug, Clone, Default)]
pub struct ProgressionConfig {
    /// `TrackCapXp`: the cumulative XP of the track's top regular rank
    caps: HashMap<String, f64>,
    /// `IsPrestigeGatedReward`: catalog ids a prestige track pays out per level
    gated: HashSet<String>,
}

type Cache = Mutex<HashMap<PathBuf, Arc<ProgressionConfig>>>;

impl ProgressionConfig {
    pub fn from_document(document: &Value) -> Self {
        let paths = document
            .pointer("/payload/paths")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let mut caps = HashMap::new();
        let mut gated = HashSet::new();
        for path in paths {
            let Some(id) = path
                .get("progression_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let requirements = path
                .get("requirements")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice);
            if !requirements.is_empty() {
                // The running sum over the requirements by rank is its total at the last rank
                let cap: f64 = requirements
                    .iter()
                    .map(|requirement| {
                        let xp = js::number(requirement.get("xp_required"));
                        if xp.is_nan() { 0.0 } else { xp }
                    })
                    .sum();
                caps.insert(id.to_owned(), cap);
            }
            let per_level = js::number(path.pointer("/prestige/xp_per_level"));
            if per_level.is_finite() && per_level > 0.0 {
                let rewards = path
                    .pointer("/prestige/free_rewards/stacked_items")
                    .and_then(Value::as_array)
                    .map_or(&[][..], Vec::as_slice);
                for reward in rewards {
                    let quantity = js::number(reward.get("quantity"));
                    if let Some(catalog_id) = reward.get("catalog_id").and_then(Value::as_str)
                        && quantity.is_finite()
                        && quantity > 0.0
                    {
                        gated.insert(catalog_id.to_owned());
                    }
                }
            }
        }
        Self { caps, gated }
    }

    pub fn load(path: &Path) -> Result<Arc<Self>, PolicyError> {
        static CACHE: OnceLock<Cache> = OnceLock::new();
        let cache = CACHE.get_or_init(Mutex::default);
        if let Some(config) = cache.lock().ok().and_then(|cache| cache.get(path).cloned()) {
            return Ok(config);
        }
        let config = Arc::new(Self::from_document(&read_game_data(path)?));
        if let Ok(mut cache) = cache.lock() {
            cache.insert(path.to_path_buf(), config.clone());
        }
        Ok(config)
    }

    /// `TrackCapXp`.
    pub fn track_cap_xp(&self, progression_id: &str) -> Option<i64> {
        self.caps.get(progression_id).map(|cap| *cap as i64)
    }

    /// `IsPrestigeGatedReward`.
    pub fn is_prestige_gated_reward(&self, catalog_id: &str) -> bool {
        self.gated.contains(catalog_id)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn config() -> ProgressionConfig {
        ProgressionConfig::from_document(&json!({"payload": {"paths": [
            {"progression_id": "ExperienceTrack_Weapon_Axe",
             "requirements": [{"rank_id": 2, "xp_required": 775}, {"rank_id": 1, "xp_required": 27000}]},
            {"progression_id": "PrestigeTrack_Weapon_Axe",
             "requirements": [{"rank_id": 1, "xp_required": 0}],
             "prestige": {"xp_per_level": 1, "free_rewards": {"stacked_items": [
                 {"catalog_id": "CURRENCY_PJM_PRESTIGE_FILLED", "quantity": 1}]}}},
            {"progression_id": "empty", "requirements": []},
            {"requirements": [{"rank_id": 1, "xp_required": 5}]}
        ]}}))
    }

    #[test]
    fn caps_are_the_cumulative_xp_of_the_top_rank() {
        let config = config();
        assert_eq!(config.track_cap_xp("ExperienceTrack_Weapon_Axe"), Some(27775));
        assert_eq!(config.track_cap_xp("PrestigeTrack_Weapon_Axe"), Some(0));
        assert_eq!(config.track_cap_xp("empty"), None);
        assert_eq!(config.track_cap_xp("nope"), None);
    }

    #[test]
    fn gated_rewards_are_what_a_prestige_track_pays() {
        let config = config();
        assert!(config.is_prestige_gated_reward("CURRENCY_PJM_PRESTIGE_FILLED"));
        assert!(!config.is_prestige_gated_reward("CURRENCY_PJM_PRESTIGE_EMPTY"));
    }
}
