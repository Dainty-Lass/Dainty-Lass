//! Trials rotation and week (R1 9), R18's inputs. The Director computes the same week for the hunts it launches.

use crate::config::numbers::strtol;
use crate::env::Raw;

/// Minutes per rotation.
pub const ROTATION_VARIABLE: &str = "TRIALS_ROTATION_MINUTES";
/// Its raw read's cap.
pub const ROTATION_CAP: usize = 31;
/// The week an arena hunt launched with.
pub const WEEK_VARIABLE: &str = "RUSTIC_ISLES_TRIALS_WEEK";
/// Its raw read's cap.
pub const WEEK_CAP: usize = 15;
/// Seven days.
pub const DEFAULT_ROTATION_MINUTES: i64 = 10_080;
/// Rows in the arena schedule.
pub const WEEKS: i64 = 181;
/// The row name prefix.
pub const ROW_PREFIX: &str = "Scheduled_Arena_Hunt_";
/// .NET ticks at the Unix epoch.
pub const UNIX_EPOCH_TICKS: i64 = 621_355_968_000_000_000;
/// .NET ticks at the `FILETIME` epoch (1601).
pub const FILETIME_EPOCH_TICKS: i64 = 504_911_232_000_000_000;
/// Ticks per minute.
pub const TICKS_PER_MINUTE: i64 = 600_000_000;

/// The rotation in minutes: 1 to 525 600, else 10 080.
pub fn rotation_minutes(raw: &Raw) -> i64 {
    match raw {
        Raw::Value(text) => match strtol(text) {
            Some(value) if (1..=525_600).contains(&value) => i64::from(value),
            _ => DEFAULT_ROTATION_MINUTES,
        },
        Raw::Unset | Raw::TooLong(_) => DEFAULT_ROTATION_MINUTES,
    }
}

/// The pinned week: 1 to 181, else none.
pub fn trials_week(raw: &Raw) -> Option<u32> {
    match raw {
        Raw::Value(text) => strtol(text)
            .filter(|week| (1..=181).contains(week))
            .and_then(|week| u32::try_from(week).ok()),
        Raw::Unset | Raw::TooLong(_) => None,
    }
}

/// `parse_week_row`: exactly the prefix and three digits, 1 to 181.
pub fn parse_week_row(name: &str) -> Option<u32> {
    let digits = name.strip_prefix(ROW_PREFIX)?;
    if digits.len() != 3 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok().filter(|week| (1..=181).contains(week))
}

/// The row name for a week.
pub fn week_row(week: u32) -> String {
    format!("{ROW_PREFIX}{week:03}")
}

/// A `FILETIME` (100 ns since 1601) in .NET ticks.
pub fn filetime_to_ticks(filetime: u64) -> i64 {
    i64::try_from(filetime)
        .unwrap_or(i64::MAX)
        .saturating_add(FILETIME_EPOCH_TICKS)
}

/// Where `now` falls: the bucket, its week and the bucket's start, all in .NET ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rotation {
    /// Rotations since the Unix epoch.
    pub bucket: i64,
    /// 1 to 181.
    pub week: u32,
    /// The bucket's first tick.
    pub bucket_start: i64,
}

/// The current week for `now` (.NET ticks) and a rotation of `minutes`.
pub fn rotation_at(now: i64, minutes: i64) -> Rotation {
    let rotation = minutes.max(1) * TICKS_PER_MINUTE;
    let bucket = (now - UNIX_EPOCH_TICKS).div_euclid(rotation);
    let week = u32::try_from(bucket.rem_euclid(WEEKS) + 1).unwrap_or(1);
    Rotation {
        bucket,
        week,
        bucket_start: UNIX_EPOCH_TICKS + bucket * rotation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(text: Option<&str>, cap: usize) -> Raw {
        Raw::from_text(text, cap)
    }

    #[test]
    fn rotation_setting() {
        assert_eq!(rotation_minutes(&raw(Some("0"), ROTATION_CAP)), 10_080);
        assert_eq!(rotation_minutes(&raw(Some("1"), ROTATION_CAP)), 1);
        assert_eq!(rotation_minutes(&raw(Some("525600"), ROTATION_CAP)), 525_600);
        assert_eq!(rotation_minutes(&raw(Some("525601"), ROTATION_CAP)), 10_080);
        assert_eq!(rotation_minutes(&raw(None, ROTATION_CAP)), 10_080);
        assert_eq!(rotation_minutes(&raw(Some(&"1".repeat(32)), ROTATION_CAP)), 10_080);
    }

    #[test]
    fn week_setting() {
        assert_eq!(trials_week(&raw(Some("0"), WEEK_CAP)), None);
        assert_eq!(trials_week(&raw(Some("1"), WEEK_CAP)), Some(1));
        assert_eq!(trials_week(&raw(Some("181"), WEEK_CAP)), Some(181));
        assert_eq!(trials_week(&raw(Some("182"), WEEK_CAP)), None);
        assert_eq!(trials_week(&raw(None, WEEK_CAP)), None);
    }

    #[test]
    fn rows() {
        assert_eq!(parse_week_row("Scheduled_Arena_Hunt_001"), Some(1));
        assert_eq!(parse_week_row("Scheduled_Arena_Hunt_181"), Some(181));
        assert_eq!(parse_week_row("Scheduled_Arena_Hunt_000"), None);
        assert_eq!(parse_week_row("Scheduled_Arena_Hunt_182"), None);
        assert_eq!(parse_week_row("Scheduled_Arena_Hunt_1"), None);
        assert_eq!(parse_week_row("Scheduled_Arena_Hunt_0011"), None);
        assert_eq!(week_row(7), "Scheduled_Arena_Hunt_007");
    }

    #[test]
    fn buckets() {
        let epoch = rotation_at(UNIX_EPOCH_TICKS, 10_080);
        assert_eq!(
            epoch,
            Rotation {
                bucket: 0,
                week: 1,
                bucket_start: UNIX_EPOCH_TICKS
            }
        );
        let now = UNIX_EPOCH_TICKS + 1_794_441_600 * 10_000_000;
        let weekly = rotation_at(now, 10_080);
        assert_eq!((weekly.bucket, weekly.week), (2_967, 72));
        let fast = rotation_at(now, 30);
        assert_eq!((fast.bucket, fast.week), (996_912, 146));
        // FILETIME of the Unix epoch is 116444736000000000
        assert_eq!(filetime_to_ticks(116_444_736_000_000_000), UNIX_EPOCH_TICKS);
    }
}
