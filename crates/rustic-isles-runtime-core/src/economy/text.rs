//! R15's log lines. The tags and wording are the AGPL's, so a host's searches keep working (call 4).

/// A refused `DAINTY_KEEP_SUPPLY_STACK_CAPS`.
pub fn caps_invalid(value: &str) -> String {
    format!("[SupplyCap] DAINTY_KEEP_SUPPLY_STACK_CAPS='{value}' invalid (default used)")
}

/// A refused `DAINTY_CELL_FUSION_MINUTES`.
pub fn fusion_invalid(value: &str) -> String {
    format!("[CellFusion] DAINTY_CELL_FUSION_MINUTES='{value}' invalid (default used)")
}

/// The caps are kept.
pub const CAPS_KEPT: &str = "[SupplyCap] DAINTY_KEEP_SUPPLY_STACK_CAPS=true: the game's stack caps are kept";

/// The game's fusion times are kept.
pub const FUSION_KEPT: &str = "[CellFusion] the game's own cell fusion times are kept";

/// One catalog walk's writes; `list` holds the first 8 as `<id>(<old cap>)`.
pub fn caps_removed(count: usize, list: &[String]) -> String {
    format!(
        "[SupplyCap] removed the stack cap of {count} supply item(s): {}",
        list.join(",")
    )
}

/// One table walk's writes; `list` holds the first 8 as `<row>(<old minutes>min)`.
pub fn fusion_set(changed: usize, rows: usize, table: &str, minutes: u32, list: &[String]) -> String {
    format!(
        "[CellFusion] set {changed}/{rows} fusion recipe(s) of {table} to {minutes} min; before: {}",
        list.join(",")
    )
}

/// A pointer as the AGPL logged it: decimal, `null` for 0.
pub fn pointer(ptr: usize) -> String {
    if ptr == 0 { "null".to_owned() } else { ptr.to_string() }
}

/// A CRC as `0x` and upper-case hex without leading zeros.
pub fn crc(value: u32) -> String {
    format!("0x{value:X}")
}

/// Slots that know their CRC but not their item, once per loadout.
pub fn waiting(loadout: usize, waiting: usize) -> String {
    format!(
        "[LoadoutSlotRepair] loadout={} {waiting} item slot(s) have an item CRC but no item id, and their inventory \
         is missing or still loading",
        pointer(loadout)
    )
}

/// The slot notification could not be found.
pub const ON_REP_MISSING: &str = "[LoadoutSlotRepair] ArchonLoadoutItemSlot.OnRep_ItemInstanceIdChanged not found";

/// One slot's outcome.
pub fn slot(data_index: usize, name: &str, value: u32, resolved: bool, attempt: u32) -> String {
    format!(
        "[LoadoutSlotRepair] loadout slot {data_index} {name} crc={} {} (attempt {attempt})",
        crc(value),
        if resolved { "resolved" } else { "still unknown" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines() {
        assert_eq!(
            caps_removed(2, &["Tonic_A(30)".into(), "Tonic_B(30)".into()]),
            "[SupplyCap] removed the stack cap of 2 supply item(s): Tonic_A(30),Tonic_B(30)"
        );
        assert_eq!(
            fusion_set(
                2,
                3,
                "DT_CellExchange",
                30,
                &["Row_A(60min)".into(), "Row_B(0min)".into()]
            ),
            "[CellFusion] set 2/3 fusion recipe(s) of DT_CellExchange to 30 min; before: Row_A(60min),Row_B(0min)"
        );
        assert_eq!(
            waiting(1234, 2),
            "[LoadoutSlotRepair] loadout=1234 2 item slot(s) have an item CRC but no item id, and their inventory \
             is missing or still loading"
        );
        assert_eq!(pointer(0), "null");
        assert_eq!(
            slot(1, "weapon", 0x1A_2B3C, true, 1),
            "[LoadoutSlotRepair] loadout slot 1 weapon crc=0x1A2B3C resolved (attempt 1)"
        );
        assert_eq!(
            slot(0, "lantern", 0xF, false, 3),
            "[LoadoutSlotRepair] loadout slot 0 lantern crc=0xF still unknown (attempt 3)"
        );
        assert_eq!(crc(0x00AB), "0xAB");
        assert_eq!(
            CAPS_KEPT,
            "[SupplyCap] DAINTY_KEEP_SUPPLY_STACK_CAPS=true: the game's stack caps are kept"
        );
    }
}
