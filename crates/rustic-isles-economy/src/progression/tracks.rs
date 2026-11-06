// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rank curves from `progression_config.json`, the track lists and the grant ids (progression spec §4.1 to §4.3).

use std::collections::HashMap;
use std::sync::Arc;

use rustic_isles::Shared;
use serde_json::Value;

use crate::util::{num, sha256_hex, trunc_i64};

/// The largest XP one grant may carry.
pub const MAX_PROGRESSION_GRANT_AMOUNT: f64 = 100_000.0;
/// UE4's userId for a call with no logged-in player.
pub const NO_PLAYER: &str = "INVALID";
/// The weapons with experience and prestige tracks.
pub const WEAPONS: [&str; 7] = [
    "Axe",
    "ChainBlades",
    "Hammer",
    "Repeaters",
    "Spear",
    "Strikers",
    "Sword",
];
/// The weapon order of the default progression list (Sword before Spear, as the game lists them)
const LIST_WEAPONS: [&str; 7] = [
    "Axe",
    "ChainBlades",
    "Hammer",
    "Repeaters",
    "Sword",
    "Spear",
    "Strikers",
];

/// `GET /progression/:userId`'s 23 known ids, in their order.
pub const DEFAULT_PROGRESSION_IDS: [&str; 23] = [
    "MasteryTrack_PlayerLevel",
    "MasteryTrack_Weapon_Axe",
    "MasteryTrack_Weapon_ChainBlades",
    "MasteryTrack_Weapon_Hammer",
    "MasteryTrack_Weapon_Repeaters",
    "MasteryTrack_Weapon_Sword",
    "MasteryTrack_Weapon_Spear",
    "MasteryTrack_Weapon_Strikers",
    "ExperienceTrack_PlayerLevel",
    "ExperienceTrack_Weapon_Axe",
    "ExperienceTrack_Weapon_ChainBlades",
    "ExperienceTrack_Weapon_Hammer",
    "ExperienceTrack_Weapon_Repeaters",
    "ExperienceTrack_Weapon_Sword",
    "ExperienceTrack_Weapon_Spear",
    "ExperienceTrack_Weapon_Strikers",
    "PrestigeTrack_Weapon_Axe",
    "PrestigeTrack_Weapon_ChainBlades",
    "PrestigeTrack_Weapon_Hammer",
    "PrestigeTrack_Weapon_Repeaters",
    "PrestigeTrack_Weapon_Sword",
    "PrestigeTrack_Weapon_Spear",
    "PrestigeTrack_Weapon_Strikers",
];

/// One track's cumulative XP per rank.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackCurve {
    /// `(rank, cumulative xp)`, sorted by rank, each rank once
    cumulative: Vec<(i64, f64)>,
    max_rank: i64,
    prestige_xp_per_level: Option<f64>,
}

impl TrackCurve {
    /// From `requirements` `[{rank_id, xp_required}]` (per-rank increments) and the prestige level size.
    pub fn build(requirements: &[Value], prestige_xp_per_level: Option<f64>) -> Self {
        let mut pairs: Vec<(i64, f64)> = requirements
            .iter()
            .map(|r| {
                let xp = num(r.get("xp_required"));
                (trunc_i64(num(r.get("rank_id"))), if xp.is_nan() { 0.0 } else { xp })
            })
            .collect();
        pairs.sort_by_key(|(rank, _)| *rank);
        let mut cumulative: Vec<(i64, f64)> = Vec::new();
        let mut total = 0.0;
        for (rank, xp) in &pairs {
            total += xp;
            match cumulative.iter_mut().find(|(r, _)| r == rank) {
                Some(entry) => entry.1 = total,
                None => cumulative.push((*rank, total)),
            }
        }
        let max_rank = pairs.last().map_or(0, |(rank, _)| *rank);
        Self {
            cumulative,
            max_rank,
            prestige_xp_per_level,
        }
    }

    /// The track's highest rank.
    pub fn max_rank(&self) -> i64 {
        self.max_rank
    }

    fn cumulative_of(&self, rank: i64) -> Option<f64> {
        self.cumulative.iter().find(|(r, _)| *r == rank).map(|(_, xp)| *xp)
    }

    /// The cumulative XP of the top rank.
    pub fn cap_xp(&self) -> f64 {
        self.cumulative_of(self.max_rank).unwrap_or(0.0)
    }

    /// The cumulative XP rank `rank` takes; a rank inside the curve without an entry takes 0 (spec §15 item 7).
    pub fn required_xp_for_rank(&self, rank: i64) -> Option<f64> {
        if rank < 0 {
            return None;
        }
        if rank <= self.max_rank {
            return Some(self.cumulative_of(rank).unwrap_or(0.0));
        }
        let per_level = self.prestige_xp_per_level?;
        if rank > self.max_rank.saturating_add(100_000) {
            return None;
        }
        #[allow(clippy::cast_precision_loss, reason = "bounded by the check above")]
        let levels = (rank - self.max_rank) as f64;
        Some(self.cap_xp() + levels * per_level)
    }

    /// The largest rank whose cumulative XP is at most `progress`, at least 0.
    pub fn rank_for_progress(&self, progress: f64) -> Option<i64> {
        if !progress.is_finite() {
            return None;
        }
        let best = self
            .cumulative
            .iter()
            .filter(|(_, xp)| *xp <= progress)
            .map(|(rank, _)| *rank)
            .max()
            .unwrap_or(0);
        Some(best.max(0))
    }
}

/// Every track's XP curve from `progression_config.json`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Curves {
    tracks: HashMap<String, TrackCurve>,
}

impl Curves {
    /// The curves of `{"payload":{"paths":[..]}}`; a path without an id or requirements is skipped.
    pub fn from_document(document: &Value) -> Self {
        let mut tracks = HashMap::new();
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
            let Some(requirements) = path
                .get("requirements")
                .and_then(Value::as_array)
                .filter(|r| !r.is_empty())
            else {
                continue;
            };
            let per_level = num(path.get("prestige").and_then(|p| p.get("xp_per_level")));
            let prestige = (per_level.is_finite() && per_level > 0.0).then_some(per_level);
            tracks.insert(id.to_owned(), TrackCurve::build(requirements, prestige));
        }
        Self { tracks }
    }

    /// The track's curve.
    pub fn get(&self, track: &str) -> Option<&TrackCurve> {
        self.tracks.get(track)
    }

    /// Whether the track has a curve.
    pub fn has(&self, track: &str) -> bool {
        self.tracks.contains_key(track)
    }

    /// The rank `progress` XP reaches on the track; None for a track with no curve.
    pub fn rank_for_progress(&self, track: &str, progress: f64) -> Option<i64> {
        self.get(track)?.rank_for_progress(progress)
    }
}

/// The module's curves, built once; a missing or bad file is "no curves" for the module's life (spec §15 item 8).
pub fn curves(shared: &Shared) -> Arc<Curves> {
    crate::caches::cached(shared, "progression.curves", || {
        let root = shared.game_data();
        if root.backend_file("progression_config.json").is_none() {
            return (Arc::new(Curves::default()), false);
        }
        match root.try_load_backend("progression_config.json") {
            Ok(Some(document)) => (Arc::new(Curves::from_document(&document)), true),
            Ok(None) => {
                tracing::error!("progression_config.json: the file is missing");
                (Arc::new(Curves::default()), true)
            }
            Err(e) => {
                tracing::error!("progression_config.json: {e}");
                (Arc::new(Curves::default()), true)
            }
        }
    })
}

/// The version's active Hunt Pass id.
pub fn active_pass(shared: &Shared) -> &'static str {
    shared.version().tables.active_huntpass_id
}

/// The Hunt Pass id the progression config serves.
pub fn served_pass(shared: &Shared) -> String {
    rustic_isles_gamedata::progression_config::served_huntpass_id(shared.version().tables, shared.settings())
}

fn starts_with_ci(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len() && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// Whether the id names a Hunt Pass (`season`, `eventpass_` or `d24_`, in any case).
pub fn has_huntpass_shape(id: &str) -> bool {
    ["season", "eventpass_", "d24_"].iter().any(|p| starts_with_ci(id, p))
}

/// Whether XP to the id goes to the active or the served Hunt Pass.
pub fn is_huntpass_xp_track(shared: &Shared, id: &str) -> bool {
    id == active_pass(shared) || id == served_pass(shared)
}

/// The 24 ids the XP route grants to without the Hunt Pass rule.
pub fn is_on_xp_allowlist(id: &str) -> bool {
    if matches!(
        id,
        "MasteryTrack_PlayerLevel" | "MasteryTrack_Behemoth" | "ExperienceTrack_PlayerLevel"
    ) {
        return true;
    }
    [
        "MasteryTrack_Weapon_",
        "ExperienceTrack_Weapon_",
        "PrestigeTrack_Weapon_",
    ]
    .iter()
    .any(|p| id.strip_prefix(p).is_some_and(|w| WEAPONS.contains(&w)))
}

/// Why `id` is not an approved XP track, or none when it is.
pub fn xp_track_refusal(shared: &Shared, id: &str) -> Option<String> {
    let curves = curves(shared);
    let served = served_pass(shared);
    let listed = is_on_xp_allowlist(id) || id == active_pass(shared) || (id == served && has_huntpass_shape(&served));
    if listed && curves.has(id) {
        return None;
    }
    if id
        .strip_prefix("Linked_Slayer_Slot_")
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
    {
        return Some("Slayer Link track (the hunt pass also_progress target) - deliberately not approved".into());
    }
    if !curves.has(id) {
        return Some("no rank curve in progression_config.json".into());
    }
    if has_huntpass_shape(id) {
        return Some(format!("hunt pass that is not served (served={served})"));
    }
    Some("not on the XP allowlist".into())
}

/// Whether the XP route may grant to the track.
pub fn is_approved_xp_track(shared: &Shared, id: &str) -> bool {
    xp_track_refusal(shared, id).is_none()
}

/// `selected_huntpass` is the active pass; another id is itself when it has a curve.
pub fn resolve_track_id(shared: &Shared, id: &str) -> Option<String> {
    if id == "selected_huntpass" {
        return Some(active_pass(shared).to_owned());
    }
    curves(shared).has(id).then(|| id.to_owned())
}

/// The track's cap (none without a curve).
pub fn track_cap_xp(shared: &Shared, id: &str) -> Option<f64> {
    curves(shared).get(id).map(TrackCurve::cap_xp)
}

/// Whether the id is a weapon's experience track, which can be reforged.
pub fn is_reforgeable_track(id: &str) -> bool {
    id.strip_prefix("ExperienceTrack_Weapon_")
        .is_some_and(|w| WEAPONS.contains(&w))
}

/// Whether the id is a weapon's prestige track.
pub fn is_prestige_track(id: &str) -> bool {
    id.strip_prefix("PrestigeTrack_Weapon_")
        .is_some_and(|w| WEAPONS.contains(&w))
}

/// `PrestigeTrack_Weapon_<W>` maps to `ExperienceTrack_Weapon_<W>`.
pub fn experience_track_for_prestige(id: &str) -> Option<String> {
    is_prestige_track(id).then(|| id.replacen("PrestigeTrack_", "ExperienceTrack_", 1))
}

/// The prestige track paired with a weapon's experience track.
pub fn prestige_track_for(experience_track: &str) -> String {
    experience_track.replacen("ExperienceTrack_", "PrestigeTrack_", 1)
}

/// Lowercase hex SHA-256 of `{"version":1,"userId":..,"progressionId":..,"amount":..}`.
pub fn grant_request_hash(user_id: &str, progression_id: &str, amount: f64) -> String {
    let body = crate::util::obj(vec![
        ("version", Value::from(1)),
        ("userId", Value::from(user_id)),
        ("progressionId", Value::from(progression_id)),
        ("amount", crate::util::number_value(amount)),
    ]);
    sha256_hex(&service_kit::jsjson::stringify(&body))
}

/// `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`
pub fn is_valid_grant_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

/// The default list's weapons in the game's order (for tests that rebuild it).
pub fn list_weapons() -> [&'static str; 7] {
    LIST_WEAPONS
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn worked_example() {
        let curve = TrackCurve::build(
            &[
                json!({"rank_id": 2, "xp_required": 20}),
                json!({"rank_id": 1, "xp_required": 10}),
                json!({"rank_id": 3, "xp_required": 5}),
            ],
            Some(100.0),
        );
        assert_eq!((curve.max_rank(), curve.cap_xp()), (3, 35.0));
        assert_eq!(curve.required_xp_for_rank(2), Some(30.0));
        assert_eq!(curve.required_xp_for_rank(0), Some(0.0));
        assert_eq!(curve.required_xp_for_rank(5), Some(235.0));
        assert_eq!(curve.required_xp_for_rank(-1), None);
        assert_eq!(curve.required_xp_for_rank(100_004), None);
        assert_eq!(curve.rank_for_progress(31.0), Some(2));
        assert_eq!(curve.rank_for_progress(5.0), Some(0));
        let no_prestige = TrackCurve::build(&[json!({"rank_id": 1, "xp_required": 10})], None);
        assert_eq!(no_prestige.required_xp_for_rank(2), None);
    }

    #[test]
    fn duplicate_ranks() {
        let curve = TrackCurve::build(
            &[
                json!({"rank_id": 1, "xp_required": 10}),
                json!({"rank_id": 1, "xp_required": 5}),
                json!({"rank_id": "x", "xp_required": 1}),
                json!({"rank_id": 2.7, "xp_required": "3"}),
            ],
            None,
        );
        // Sorted: 0 (NaN rank) 1, 1:10, 1:5, 2:3 -> cumulative 0:1, 1:16, 2:19
        assert_eq!(curve.required_xp_for_rank(1), Some(16.0));
        assert_eq!(curve.cap_xp(), 19.0);
    }

    #[test]
    fn grant_ids_and_tracks() {
        assert!(is_valid_grant_id("g1"));
        assert!(is_valid_grant_id("reforge-01ARZ-banked"));
        for bad in ["", "-x", "a b", &"a".repeat(129)] {
            assert!(!is_valid_grant_id(bad), "{bad}");
        }
        assert!(is_valid_grant_id(&"a".repeat(128)));
        assert!(is_reforgeable_track("ExperienceTrack_Weapon_Axe"));
        assert!(!is_reforgeable_track("ExperienceTrack_PlayerLevel"));
        assert_eq!(
            experience_track_for_prestige("PrestigeTrack_Weapon_Sword").as_deref(),
            Some("ExperienceTrack_Weapon_Sword")
        );
        assert_eq!(
            prestige_track_for("ExperienceTrack_Weapon_Sword"),
            "PrestigeTrack_Weapon_Sword"
        );
        assert!(is_on_xp_allowlist("MasteryTrack_Behemoth"));
        assert!(!is_on_xp_allowlist("MasteryTrack_Weapon_Bow"));
        let mut all: Vec<String> = vec!["MasteryTrack_PlayerLevel".into()];
        all.extend(list_weapons().iter().map(|w| format!("MasteryTrack_Weapon_{w}")));
        all.push("ExperienceTrack_PlayerLevel".into());
        all.extend(list_weapons().iter().map(|w| format!("ExperienceTrack_Weapon_{w}")));
        all.extend(list_weapons().iter().map(|w| format!("PrestigeTrack_Weapon_{w}")));
        assert_eq!(all, DEFAULT_PROGRESSION_IDS);
        assert_eq!(
            grant_request_hash("u", "p", 5.0),
            sha256_hex(r#"{"version":1,"userId":"u","progressionId":"p","amount":5}"#)
        );
    }
}
