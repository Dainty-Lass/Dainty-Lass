//! The two settings (section 3, RT-11), read once at a feature's first run. The flag files the AGPL read are not
//! (departure 1).

use super::text;
use crate::config::numbers::strtol;

/// Keeps the game's supply stack caps: `true` or `false`.
pub const KEEP_SUPPLY_STACK_CAPS: &str = "DAINTY_KEEP_SUPPLY_STACK_CAPS";
/// Cell fusion minutes, 0 to 10 080; 0 keeps the game's own (departure 2: the AGPL's `CELL_FUSION_MINUTES`).
pub const CELL_FUSION_MINUTES: &str = "DAINTY_CELL_FUSION_MINUTES";
/// The `env_value` cap both are read with.
pub const MAX_UNITS: usize = 15;
/// The default fusion time.
pub const DEFAULT_FUSION_MINUTES: u32 = 30;
/// Seven days.
pub const MAX_FUSION_MINUTES: u32 = 10_080;
/// `FTimespan` ticks (100 ns) in a minute.
pub const TICKS_PER_MINUTE: i64 = 600_000_000;

/// What the environment held: `set` when the variable is present and non-empty, `value` what `env_value` returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Read<'a> {
    /// Present and non-empty.
    pub set: bool,
    /// `env_value`'s answer (`None` when it refused the value).
    pub value: Option<&'a str>,
}

impl<'a> Read<'a> {
    /// Unset.
    pub const UNSET: Read<'static> = Read {
        set: false,
        value: None,
    };

    /// A set variable whose value `env_value` returned.
    pub fn value(value: &'a str) -> Self {
        Read {
            set: true,
            value: Some(value),
        }
    }
}

/// Whether the caps are kept, and the note for a refused value.
pub fn keep_supply_caps(read: Read) -> (bool, Option<String>) {
    match read.value {
        Some("true") => (true, None),
        Some("false") => (false, None),
        _ if read.set => (false, Some(text::caps_invalid(read.value.unwrap_or("")))),
        _ => (false, None),
    }
}

/// The fusion minutes (0 keeps the game's), and the note for a refused value.
pub fn fusion_minutes(read: Read) -> (u32, Option<String>) {
    let parsed = read
        .value
        .and_then(strtol)
        .and_then(|minutes| u32::try_from(minutes).ok())
        .filter(|&minutes| minutes <= MAX_FUSION_MINUTES);
    match parsed {
        Some(minutes) => (minutes, None),
        None if read.set => (
            DEFAULT_FUSION_MINUTES,
            Some(text::fusion_invalid(read.value.unwrap_or(""))),
        ),
        None => (DEFAULT_FUSION_MINUTES, None),
    }
}

/// The `TimeRequired` every recipe gets, `None` for 0 (the game's own times are kept).
pub fn target_ticks(minutes: u32) -> Option<i64> {
    (minutes > 0).then(|| i64::from(minutes) * TICKS_PER_MINUTE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::check_env_value;

    /// What the DLL's reads give for a raw value: set when non-empty, then R0's `env_value` rule.
    fn env(raw: Option<&str>) -> (bool, Option<String>) {
        let units: Option<Vec<u16>> = raw.map(|text| text.encode_utf16().collect());
        let set = raw.is_some_and(|text| !text.is_empty());
        (set, check_env_value(units.as_deref(), MAX_UNITS))
    }

    fn caps(raw: Option<&str>) -> (bool, Option<String>) {
        let (set, value) = env(raw);
        keep_supply_caps(Read {
            set,
            value: value.as_deref(),
        })
    }

    fn minutes(raw: Option<&str>) -> (u32, Option<String>) {
        let (set, value) = env(raw);
        fusion_minutes(Read {
            set,
            value: value.as_deref(),
        })
    }

    #[test]
    fn supply_caps_setting() {
        assert_eq!(caps(None), (false, None));
        assert_eq!(caps(Some("true")), (true, None));
        assert_eq!(caps(Some("false")), (false, None));
        let note = |text: &str| {
            Some(format!(
                "[SupplyCap] DAINTY_KEEP_SUPPLY_STACK_CAPS='{text}' invalid (default used)"
            ))
        };
        assert_eq!(caps(Some("TRUE")), (false, note("TRUE")));
        assert_eq!(caps(Some("1")), (false, note("1")));
        assert_eq!(
            caps(Some("abcdefghijklmnop")),
            (false, note("")),
            "16 units: env_value refuses"
        );
        assert_eq!(caps(Some("")), (false, None));
    }

    #[test]
    fn fusion_setting() {
        let note = |text: &str| {
            Some(format!(
                "[CellFusion] DAINTY_CELL_FUSION_MINUTES='{text}' invalid (default used)"
            ))
        };
        assert_eq!(minutes(None), (30, None));
        assert_eq!(minutes(Some("0")), (0, None));
        assert_eq!(minutes(Some("1")), (1, None));
        assert_eq!(minutes(Some("10080")), (10_080, None));
        assert_eq!(minutes(Some("10081")), (30, note("10081")));
        assert_eq!(minutes(Some("-1")), (30, note("-1")));
        assert_eq!(minutes(Some(" 30")), (30, None));
        assert_eq!(minutes(Some("30 ")), (30, note("30 ")));
        assert_eq!(minutes(Some("x")), (30, note("x")));
        assert_eq!(target_ticks(30), Some(18_000_000_000));
        assert_eq!(target_ticks(10_080), Some(6_048_000_000_000));
        assert_eq!(target_ticks(0), None);
    }
}
