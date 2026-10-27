//! `loadoutSlots.ts`: the entitlement model (one default slot and up to five Slayer's Path unlocks) and the wire
//! counts every loadout answer carries. Both versions share it (RU3c: 1.12's RVA 0x01A6D4F0 is the same native code).

use serde_json::Value;

pub const DEFAULT_ACCOUNT_LOADOUT_SLOTS: u32 = 1;
pub const MAX_CHARACTER_LOADOUT_SLOTS: u32 = 5;
pub const MAX_TOTAL_LOADOUT_SLOTS: u32 = DEFAULT_ACCOUNT_LOADOUT_SLOTS + MAX_CHARACTER_LOADOUT_SLOTS;
pub const PLAYER_JOURNEY_LOADOUT_NODE_IDS: [&str; 5] = [
    "LoadoutSlot_01",
    "LoadoutSlot_02",
    "LoadoutSlot_03",
    "LoadoutSlot_04",
    "LoadoutSlot_05",
];

/// Node's `RangeError`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SlotRangeError(pub String);

fn validate_total(value: i64, label: &str) -> Result<u32, SlotRangeError> {
    if (i64::from(DEFAULT_ACCOUNT_LOADOUT_SLOTS)..=i64::from(MAX_TOTAL_LOADOUT_SLOTS)).contains(&value) {
        Ok(value as u32)
    } else {
        Err(SlotRangeError(format!(
            "{label} must be between {DEFAULT_ACCOUNT_LOADOUT_SLOTS} and {MAX_TOTAL_LOADOUT_SLOTS}"
        )))
    }
}

/// How many stored slots the player sees. A row without a persisted entitlement predates multi-slot support or was
/// touched by the 2026-07-22 reading of `/unlock/N`, so it has one excess slot: `stored - 1`, never below 1.
pub fn resolve_visible_total(stored: i64, persisted: Option<i64>) -> Result<u32, SlotRangeError> {
    let stored = validate_total(stored, "Stored loadout slot count")?;
    let Some(persisted) = persisted else {
        return Ok(DEFAULT_ACCOUNT_LOADOUT_SLOTS.max(stored - DEFAULT_ACCOUNT_LOADOUT_SLOTS));
    };
    let persisted = validate_total(persisted, "Persisted unlocked loadout slot count")?;
    if persisted > stored {
        return Err(SlotRangeError(format!(
            "Persisted unlocked loadout slot count {persisted} exceeds stored count {stored}"
        )));
    }
    Ok(persisted)
}

/// Unlocks are monotonic: a lower request is a stale travel replay and cannot relock a slot.
pub fn resolve_requested_total(persisted: Option<i64>, requested: i64) -> Result<u32, SlotRangeError> {
    let requested = validate_total(requested, "Requested total loadout slot count")?;
    let Some(persisted) = persisted else {
        return Ok(requested);
    };
    Ok(validate_total(persisted, "Persisted unlocked loadout slot count")?.max(requested))
}

/// The entitlement from the five `LoadoutSlot_0N` nodes: only `Unlocked` (2) counts, `LockedButClaimable` (1) does not.
pub fn resolve_player_journey_total(nodes: Option<&Value>) -> u32 {
    let unlocked = PLAYER_JOURNEY_LOADOUT_NODE_IDS
        .iter()
        .filter(|id| {
            nodes
                .and_then(|n| n.get(**id))
                .and_then(|n| n.get("node_status"))
                .and_then(Value::as_f64)
                == Some(2.0)
        })
        .count() as u32;
    DEFAULT_ACCOUNT_LOADOUT_SLOTS + unlocked
}

/// The stored active slot when it is a valid visible one, else slot zero.
pub fn resolve_active_index(stored: Option<i64>, visible_total: u32) -> Result<u32, SlotRangeError> {
    validate_total(i64::from(visible_total), "Visible loadout slot count")?;
    Ok(match stored {
        Some(index) if (0..i64::from(visible_total)).contains(&index) => index as u32,
        _ => 0,
    })
}

/// The four wire fields of every loadout response: every entitled slot is a CHARACTER slot and account slots are
/// 0/0 (`ResolveWireLoadoutSlotCounts`). `entitled` is a float because Node clamps `Math.trunc` of any number.
pub fn wire_counts(entitled: f64) -> (u32, u32, u32, u32) {
    let requested = if entitled.is_finite() { entitled.trunc() } else { 0.0 };
    let total = requested.clamp(
        f64::from(DEFAULT_ACCOUNT_LOADOUT_SLOTS),
        f64::from(MAX_TOTAL_LOADOUT_SLOTS),
    ) as u32;
    (0, 0, total, MAX_TOTAL_LOADOUT_SLOTS)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_legacy_row_exposes_one_slot_fewer() {
        assert_eq!(resolve_visible_total(3, None), Ok(2));
        assert_eq!(resolve_visible_total(2, None), Ok(1));
        assert_eq!(resolve_visible_total(1, None), Ok(1));
    }

    #[test]
    fn a_persisted_entitlement_hides_dormant_slots() {
        assert_eq!(resolve_visible_total(3, Some(2)), Ok(2));
        assert!(resolve_visible_total(2, Some(3)).is_err());
    }

    #[test]
    fn unlocks_are_monotonic() {
        assert_eq!(resolve_requested_total(None, 2), Ok(2));
        assert_eq!(resolve_requested_total(Some(3), 2), Ok(3));
        assert_eq!(resolve_requested_total(Some(5), 6), Ok(6));
        assert!(resolve_requested_total(None, 0).is_err());
    }

    #[test]
    fn the_active_slot_survives_travel_or_falls_back_to_zero() {
        assert_eq!(resolve_active_index(Some(3), 4), Ok(3));
        assert_eq!(resolve_active_index(None, 4), Ok(0));
        assert_eq!(resolve_active_index(Some(4), 4), Ok(0));
        assert_eq!(resolve_active_index(Some(-1), 4), Ok(0));
    }

    #[test]
    fn the_journey_grants_a_slot_per_unlocked_node() {
        let all = json!({
            "LoadoutSlot_01": {"node_status": 2}, "LoadoutSlot_02": {"node_status": 2},
            "LoadoutSlot_03": {"node_status": 2}, "LoadoutSlot_04": {"node_status": 2},
            "LoadoutSlot_05": {"node_status": 2}
        });
        assert_eq!(resolve_player_journey_total(Some(&all)), 6);
        let some = json!({"LoadoutSlot_01": {"node_status": 2}, "LoadoutSlot_02": {"node_status": 1},
            "LoadoutSlot_03": {"node_status": 0}});
        assert_eq!(resolve_player_journey_total(Some(&some)), 2);
        assert_eq!(resolve_player_journey_total(None), 1);
    }

    #[test]
    fn the_wire_counts_put_every_slot_in_the_character_bucket() {
        assert_eq!(wire_counts(6.0), (0, 0, 6, 6));
        assert_eq!(wire_counts(1.0), (0, 0, 1, 6));
    }

    #[test]
    fn the_native_unlock_delta_is_zero_for_every_unlock_count() {
        for unlocked in 0..=5usize {
            let nodes: serde_json::Map<String, Value> = PLAYER_JOURNEY_LOADOUT_NODE_IDS[..unlocked]
                .iter()
                .map(|id| ((*id).to_owned(), json!({"node_status": 2})))
                .collect();
            let total = resolve_player_journey_total(Some(&Value::Object(nodes)));
            let (num_account, max_account, num_char, max_char) = wire_counts(f64::from(total));
            let u = unlocked as i64;
            assert_eq!((1 + u) - i64::from(num_char), 0, "resolve delta at U={u}");
            assert_eq!(num_account + num_char, total);
            assert_eq!(max_account + max_char, 6);
            assert_eq!(i64::from(max_char) - i64::from(num_char), 5 - u);
            assert_eq!(
                (1 + u) - i64::from(total.saturating_sub(1)),
                1,
                "the old split always looped"
            );
        }
    }

    #[test]
    fn the_wire_counts_clamp_to_the_entitlement_range() {
        for bad in [0.0, -3.0, f64::NAN, f64::INFINITY] {
            assert_eq!(wire_counts(bad).2, 1, "input {bad}");
        }
        assert_eq!(wire_counts(7.0).2, 6);
        assert_eq!(wire_counts(6.9).2, 6);
    }
}
