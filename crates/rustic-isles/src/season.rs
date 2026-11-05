// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Hunt Pass season window (core spec §8.4), shared by game data, progression, bounties and cooldowns.

use chrono::{DateTime, Datelike, Timelike};

/// `2024-09-26T17:00:00.000Z`, a Thursday
pub const ANCHOR_MS: i64 = 1_727_370_000_000;
/// 84 days
pub const LENGTH_MS: i64 = 84 * 24 * 60 * 60 * 1000;

/// `[start, end)` of the window holding `t`; before the anchor the cycles run backwards.
pub fn active_season_window_ms(t: i64) -> (i64, i64) {
    let start = ANCHOR_MS + (t - ANCHOR_MS).div_euclid(LENGTH_MS) * LENGTH_MS;
    (start, start + LENGTH_MS)
}

/// The window as `IsoUtc` texts.
pub fn active_season_window(t: i64) -> (String, String) {
    let (start, end) = active_season_window_ms(t);
    (iso_utc(start), iso_utc(end))
}

/// `Date#toISOString`: four-digit years, `+YYYYYY` / `-YYYYYY` outside 0000-9999.
pub fn to_iso_string(ms: i64) -> String {
    let Some(at) = DateTime::from_timestamp_millis(ms) else {
        return String::from("Invalid Date");
    };
    let year = at.year();
    let year = if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else if year < 0 {
        format!("-{:06}", -i64::from(year))
    } else {
        format!("+{year:06}")
    };
    format!(
        "{year}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.month(),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.timestamp_subsec_millis()
    )
}

/// The ISO text with a whole-second `.000Z` written `+00:00`; milliseconds keep `Z`.
pub fn iso_utc(ms: i64) -> String {
    let iso = to_iso_string(ms);
    match iso.strip_suffix(".000Z") {
        Some(whole) => format!("{whole}+00:00"),
        None => iso,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(text: &str) -> i64 {
        DateTime::parse_from_rfc3339(text).unwrap().timestamp_millis()
    }

    #[test]
    fn windows() {
        let window = |t: &str| active_season_window(ms(t));
        let summer = (
            "2026-07-30T17:00:00+00:00".to_owned(),
            "2026-10-22T17:00:00+00:00".to_owned(),
        );
        assert_eq!(window("2026-10-22T16:59:59.999Z"), summer);
        assert_eq!(window("2026-10-13T18:00:00.000Z"), summer);
        let autumn = active_season_window_ms(ms("2026-10-22T17:00:00.000Z"));
        assert_eq!(
            autumn,
            (ms("2026-10-22T17:00:00Z"), ms("2026-10-22T17:00:00Z") + LENGTH_MS)
        );
        assert_eq!(
            active_season_window_ms(ANCHOR_MS - 1),
            (ANCHOR_MS - LENGTH_MS, ANCHOR_MS)
        );
        let anchor = DateTime::from_timestamp_millis(ANCHOR_MS).unwrap();
        assert_eq!(anchor.weekday(), chrono::Weekday::Thu);
        assert_eq!(to_iso_string(ANCHOR_MS), "2024-09-26T17:00:00.000Z");
    }

    #[test]
    fn iso_texts() {
        assert_eq!(to_iso_string(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(to_iso_string(-1), "1969-12-31T23:59:59.999Z");
        let leap = ms("2024-02-29T23:59:59.999Z");
        assert_eq!(to_iso_string(leap), "2024-02-29T23:59:59.999Z");
        assert_eq!(iso_utc(ms("2026-10-22T17:00:00.250Z")), "2026-10-22T17:00:00.250Z");
        assert_eq!(to_iso_string(253_402_300_800_000), "+010000-01-01T00:00:00.000Z");
    }
}
