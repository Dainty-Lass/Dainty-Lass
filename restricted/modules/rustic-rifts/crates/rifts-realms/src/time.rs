//! Node keeps every realm time as `toISOString()` text; the tables keep Unix milliseconds and the views write the text.

use std::time::{SystemTime, UNIX_EPOCH};

use rifts_seam::UnixMillis;

pub fn now() -> UnixMillis {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX));
    UnixMillis(millis)
}

/// `new Date(ms).toISOString()`, for years 0 to 9999.
pub fn iso(at: UnixMillis) -> String {
    let millis = at.0.rem_euclid(1000);
    let seconds = at.0.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}

/// The `toISOString()` shape only; `None` for anything else.
pub fn parse_iso(text: &str) -> Option<UnixMillis> {
    let b = text.as_bytes();
    if b.len() != 24 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    if b[19] != b'.' || b[23] != b'Z' {
        return None;
    }
    let number = |from: usize, to: usize| -> Option<i64> {
        let digits = text.get(from..to)?;
        if !digits.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok()
    };
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second, millis) = (number(11, 13)?, number(14, 16)?, number(17, 19)?, number(20, 23)?);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) || hour > 23 || minute > 59 {
        return None;
    }
    if second > 59 {
        return None;
    }
    // Howard Hinnant's days_from_civil
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(UnixMillis(
        ((days * 86_400 + hour * 3600 + minute * 60 + second) * 1000) + millis,
    ))
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_round_trips_as_to_iso_string() {
        for (ms, text) in [
            (0, "1970-01-01T00:00:00.000Z"),
            (1_792_000_000_123, "2026-10-14T17:46:40.123Z"),
            (951_782_400_000, "2000-02-29T00:00:00.000Z"),
            (-1, "1969-12-31T23:59:59.999Z"),
        ] {
            assert_eq!(iso(UnixMillis(ms)), text);
            assert_eq!(parse_iso(text), Some(UnixMillis(ms)));
        }
        for bad in [
            "2026-02-30T00:00:00.000Z",
            "2026-10-14T17:46:40Z",
            "2026-10-14 17:46:40.123Z",
            "",
        ] {
            assert_eq!(parse_iso(bad), None, "{bad}");
        }
    }
}
