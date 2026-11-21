// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Time as the launcher reads it: Unix milliseconds inside, `Date.prototype.toISOString` text outside.

use std::time::{SystemTime, UNIX_EPOCH};

pub use rustic_rifts_seam::UnixMillis;

/// An instant moved by a duration; durations stay plain milliseconds.
pub(crate) trait Later {
    fn saturating_add(self, ms: i64) -> UnixMillis;
}

impl Later for UnixMillis {
    fn saturating_add(self, ms: i64) -> UnixMillis {
        UnixMillis(self.0.saturating_add(ms))
    }
}

/// The system time, in milliseconds since the Unix epoch; 0 before it.
pub fn now() -> UnixMillis {
    UnixMillis(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX)),
    )
}

/// `new Date(at).toISOString()` for years 0000-9999.
pub fn iso(at: UnixMillis) -> String {
    rustic_rifts_guard::iso_time(at)
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

/// Only `YYYY-MM-DDTHH:MM:SS.mmmZ` with a real date in it.
pub fn parse_iso(text: &str) -> Option<UnixMillis> {
    // Four digits of year, no sign: chrono's %Y would take one
    if text.len() != 24 || !text.bytes().take(4).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let at = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.3fZ").ok()?;
    // chrono reads second 60 as a leap second; JavaScript refuses it
    (at.and_utc().timestamp_subsec_nanos() < 1_000_000_000).then(|| UnixMillis(at.and_utc().timestamp_millis()))
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
