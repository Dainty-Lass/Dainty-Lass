// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Loadout slot rules (spec §7.3, `loadoutSlots.ts`).

use serde_json::Value;

/// The slots a new account starts with.
pub const DEFAULT_ACCOUNT_LOADOUT_SLOTS: u32 = 1;
/// The most slots the Slayer's Path can unlock.
pub const MAX_CHARACTER_LOADOUT_SLOTS: u32 = 5;
/// The most slots a character's stored row can hold.
pub const MAX_TOTAL_LOADOUT_SLOTS: u32 = 6;
/// The Slayer's Path nodes that each unlock one slot.
pub const PLAYER_JOURNEY_LOADOUT_NODE_IDS: [&str; 5] = [
    "LoadoutSlot_01",
    "LoadoutSlot_02",
    "LoadoutSlot_03",
    "LoadoutSlot_04",
    "LoadoutSlot_05",
];

/// A slot count out of range; the text says which.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SlotRangeError(pub String);

fn valid(total: i64, label: &str) -> Result<u32, SlotRangeError> {
    match u32::try_from(total) {
        Ok(n) if (1..=MAX_TOTAL_LOADOUT_SLOTS).contains(&n) => Ok(n),
        _ => Err(SlotRangeError(format!("{label} must be between 1 and 6"))),
    }
}

const PERSISTED: &str = "Persisted unlocked loadout slot count";

/// The slots a character sees: a legacy row (no persisted count) has one excess stored slot.
pub fn resolve_visible_total(stored: i64, persisted: Option<i64>) -> Result<u32, SlotRangeError> {
    let stored = valid(stored, "Stored loadout slot count")?;
    let Some(persisted) = persisted else {
        return Ok(stored.saturating_sub(1).max(1));
    };
    let persisted = valid(persisted, PERSISTED)?;
    if persisted > stored {
        return Err(SlotRangeError(format!(
            "{PERSISTED} {persisted} exceeds stored count {stored}"
        )));
    }
    Ok(persisted)
}

/// Unlocks are monotonic: never below what is persisted.
pub fn resolve_requested_total(persisted: Option<i64>, requested: i64) -> Result<u32, SlotRangeError> {
    let requested = valid(requested, "Requested total loadout slot count")?;
    match persisted {
        None => Ok(requested),
        Some(p) => Ok(valid(p, PERSISTED)?.max(requested)),
    }
}

/// 1 + the loadout nodes at `node_status` 2 (`Unlocked`); 1 (`LockedButClaimable`) does not count.
pub fn resolve_player_journey_total(nodes: Option<&Value>) -> u32 {
    let unlocked = PLAYER_JOURNEY_LOADOUT_NODE_IDS
        .iter()
        .filter(|id| {
            nodes
                .and_then(|n| n.get(**id))
                .and_then(|node| node.get("node_status"))
                .and_then(Value::as_f64)
                == Some(2.0)
        })
        .count();
    DEFAULT_ACCOUNT_LOADOUT_SLOTS + u32::try_from(unlocked).unwrap_or(0)
}

/// The stored index while it is a visible slot, else 0.
pub fn resolve_active_index(stored: Option<i64>, visible: u32) -> Result<u32, SlotRangeError> {
    let visible = valid(i64::from(visible), "Visible loadout slot count")?;
    Ok(stored
        .and_then(|s| u32::try_from(s).ok())
        .filter(|s| *s < visible)
        .unwrap_or(0))
}

/// The client's four counts for an entitlement: `(0, 0, total, 6)`, the total truncated into 1..6.
pub fn wire_counts(entitled: f64) -> (u32, u32, u32, u32) {
    let entitled = if entitled.is_finite() { entitled.trunc() } else { 0.0 };
    let total = entitled.clamp(1.0, f64::from(MAX_TOTAL_LOADOUT_SLOTS));
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 1..6 above, so the cast is exact"
    )]
    let total = total as u32;
    (0, 0, total, MAX_TOTAL_LOADOUT_SLOTS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn visible_totals() {
        assert_eq!(resolve_visible_total(3, None), Ok(2));
        assert_eq!(resolve_visible_total(2, None), Ok(1));
        assert_eq!(resolve_visible_total(1, None), Ok(1));
        assert_eq!(resolve_visible_total(3, Some(2)), Ok(2));
        assert_eq!(
            resolve_visible_total(2, Some(3)).unwrap_err().to_string(),
            "Persisted unlocked loadout slot count 3 exceeds stored count 2"
        );
        assert_eq!(
            resolve_visible_total(7, None).unwrap_err().to_string(),
            "Stored loadout slot count must be between 1 and 6"
        );
        assert_eq!(
            resolve_visible_total(3, Some(0)).unwrap_err().to_string(),
            "Persisted unlocked loadout slot count must be between 1 and 6"
        );
    }

    #[test]
    fn requested_totals() {
        assert_eq!(resolve_requested_total(None, 2), Ok(2));
        assert_eq!(resolve_requested_total(Some(3), 2), Ok(3));
        assert_eq!(resolve_requested_total(Some(5), 6), Ok(6));
        assert_eq!(
            resolve_requested_total(None, 0).unwrap_err().to_string(),
            "Requested total loadout slot count must be between 1 and 6"
        );
    }

    #[test]
    fn active_indexes() {
        assert_eq!(resolve_active_index(Some(3), 4), Ok(3));
        assert_eq!(resolve_active_index(None, 4), Ok(0));
        assert_eq!(resolve_active_index(Some(4), 4), Ok(0));
        assert_eq!(resolve_active_index(Some(-1), 4), Ok(0));
        assert_eq!(
            resolve_active_index(Some(0), 0).unwrap_err().to_string(),
            "Visible loadout slot count must be between 1 and 6"
        );
    }

    #[test]
    fn journey_totals() {
        let mut nodes = serde_json::Map::new();
        for id in PLAYER_JOURNEY_LOADOUT_NODE_IDS {
            nodes.insert(id.into(), json!({"node_status": 2}));
        }
        assert_eq!(resolve_player_journey_total(Some(&Value::Object(nodes))), 6);
        let mixed = json!({
            "LoadoutSlot_01": {"node_status": 2},
            "LoadoutSlot_02": {"node_status": 1},
            "LoadoutSlot_03": {"node_status": 0},
            "LoadoutSlot_04": {"node_status": "2"},
        });
        assert_eq!(resolve_player_journey_total(Some(&mixed)), 2);
        assert_eq!(resolve_player_journey_total(None), 1);
        for unlocked in 0..=5 {
            let mut nodes = serde_json::Map::new();
            for id in &PLAYER_JOURNEY_LOADOUT_NODE_IDS[..unlocked] {
                nodes.insert((*id).into(), json!({"node_status": 2}));
            }
            let total = resolve_player_journey_total(Some(&Value::Object(nodes)));
            assert_eq!(total as usize, 1 + unlocked);
        }
    }

    #[test]
    fn wire() {
        assert_eq!(wire_counts(6.0), (0, 0, 6, 6));
        assert_eq!(wire_counts(1.0), (0, 0, 1, 6));
        for low in [0.0, -3.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(wire_counts(low).2, 1, "{low}");
        }
        assert_eq!(wire_counts(7.0).2, 6);
        assert_eq!(wire_counts(6.9).2, 6);
        assert_eq!(wire_counts(2.7).2, 2);
    }
}
