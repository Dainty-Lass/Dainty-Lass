//! `Date.now()` and `new Date().toISOString()`.

pub fn now_ms() -> i64 {
    crate::game_data::season_window::now_ms()
}

/// `new Date(ms).toISOString()` (years 0000 to 9999).
pub fn iso(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let in_day = ms.rem_euclid(86_400_000);
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000
    )
}

pub fn iso_now() -> String {
    iso(now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_matches_to_iso_string() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        // new Date(Date.UTC(2026, 9, 11, 7, 0, 0, 123)).toISOString()
        assert_eq!(iso(1_791_702_000_123), "2026-10-11T07:00:00.123Z");
        assert_eq!(iso(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso(-1), "1969-12-31T23:59:59.999Z");
    }
}
