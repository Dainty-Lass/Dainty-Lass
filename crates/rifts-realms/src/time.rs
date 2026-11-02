// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Time as the launcher reads it: Unix milliseconds inside, `Date.prototype.toISOString` text outside.

use std::time::{SystemTime, UNIX_EPOCH};

pub use rifts_seam::UnixMillis;

/// An instant moved by a duration; durations stay plain milliseconds.
pub(crate) trait Later {
    fn saturating_add(self, ms: i64) -> UnixMillis;
}

impl Later for UnixMillis {
    fn saturating_add(self, ms: i64) -> UnixMillis {
        UnixMillis(self.0.saturating_add(ms))
    }
}

const DAY_MS: i64 = 86_400_000;

pub fn now() -> UnixMillis {
    UnixMillis(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX)),
    )
}

/// `new Date(at).toISOString()` for years 0000-9999.
pub fn iso(at: UnixMillis) -> String {
    rifts_guard::iso_time(at)
}

/// An instant in stored JSON: the bare millisecond number.
pub(crate) mod millis {
    use super::UnixMillis;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(at: &UnixMillis, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(at.0)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<UnixMillis, D::Error> {
        i64::deserialize(deserializer).map(UnixMillis)
    }
}

/// `millis` for an optional instant.
pub(crate) mod opt_millis {
    use super::UnixMillis;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(at: &Option<UnixMillis>, serializer: S) -> Result<S::Ok, S::Error> {
        match at {
            Some(at) => serializer.serialize_some(&at.0),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<UnixMillis>, D::Error> {
        Option::<i64>::deserialize(deserializer).map(|at| at.map(UnixMillis))
    }
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let shifted = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * shifted + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Only `YYYY-MM-DDTHH:MM:SS.mmmZ` with a real date in it.
pub fn parse_iso(text: &str) -> Option<UnixMillis> {
    let bytes = text.as_bytes();
    if bytes.len() != 24 {
        return None;
    }
    for (index, byte) in bytes.iter().enumerate() {
        let expected = match index {
            4 | 7 => Some(b'-'),
            10 => Some(b'T'),
            13 | 16 => Some(b':'),
            19 => Some(b'.'),
            23 => Some(b'Z'),
            _ => None,
        };
        match expected {
            Some(separator) if *byte != separator => return None,
            None if !byte.is_ascii_digit() => return None,
            _ => {}
        }
    }
    let number = |from: usize, to: usize| {
        bytes[from..to]
            .iter()
            .fold(0i64, |acc, digit| acc * 10 + i64::from(digit - b'0'))
    };
    let (year, month, day) = (number(0, 4), number(5, 7), number(8, 10));
    let (hour, minute, second, millis) = (number(11, 13), number(14, 16), number(17, 19), number(20, 23));
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let in_day = ((hour * 60 + minute) * 60 + second) * 1000 + millis;
    Some(UnixMillis(days_from_civil(year, month, day) * DAY_MS + in_day))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_both_ways() {
        for (at, text) in [
            (0, "1970-01-01T00:00:00.000Z"),
            (1_792_000_000_123, "2026-10-14T17:46:40.123Z"),
            (951_782_400_000, "2000-02-29T00:00:00.000Z"),
            (-1, "1969-12-31T23:59:59.999Z"),
        ] {
            assert_eq!(iso(UnixMillis(at)), text);
            assert_eq!(parse_iso(text), Some(UnixMillis(at)), "{text}");
        }
    }

    #[test]
    fn only_the_one_shape() {
        for bad in [
            "2026-10-14T17:46:40Z",
            "2026-10-14 17:46:40.123Z",
            "2026-02-30T00:00:00.000Z",
            "2026-13-01T00:00:00.000Z",
            "2026-10-14T24:00:00.000Z",
            "2026-10-14T17:60:00.000Z",
            "2026-10-14T17:46:60.000Z",
            "1900-02-29T00:00:00.000Z",
            "2026-10-14T17:46:40.123z",
            "+026-10-14T17:46:40.123Z",
            "",
        ] {
            assert_eq!(parse_iso(bad), None, "{bad}");
        }
        assert!(parse_iso("2024-02-29T00:00:00.000Z").is_some());
        assert!(now().0 > 1_700_000_000_000);
    }
}
