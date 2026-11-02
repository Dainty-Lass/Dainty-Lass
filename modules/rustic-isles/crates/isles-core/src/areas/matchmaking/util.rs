//! Small JavaScript behaviours the controllers lean on: `toISOString`, `Number(..)` and a clock the tests can move.

use std::sync::atomic::{AtomicI64, Ordering};

/// `Date.now()`, with an offset a test moves instead of sleeping (Node's tests mock `Date.now` the same way).
#[derive(Debug, Default)]
pub struct Clock {
    offset_ms: AtomicI64,
}

impl Clock {
    pub fn now_ms(&self) -> i64 {
        let real = crate::game_data::season_window::now_ms();
        real.saturating_add(self.offset_ms.load(Ordering::Relaxed))
    }

    /// Moves the clock forward.
    pub fn advance(&self, ms: i64) {
        self.offset_ms.fetch_add(ms, Ordering::Relaxed);
    }
}

/// `new Date(ms).toISOString()`: `2026-10-13T12:00:00.000Z`.
pub fn iso(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let in_day = ms.rem_euclid(86_400_000);
    // civil_from_days (Hinnant)
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
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000
    )
}

/// `Number(text)` for the plain decimal forms a setting holds: blank is 0, anything unparsable is `NaN`.
pub fn js_number(text: &str) -> f64 {
    let text = text.trim();
    if text.is_empty() {
        return 0.0;
    }
    let plain = text
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+' | b'e' | b'E'));
    if !plain {
        return f64::NAN;
    }
    text.parse().unwrap_or(f64::NAN)
}

/// `ReadDelayMs` (`controllers/matchmaking.ts`): a finite number above zero, floored, else the fallback.
pub fn delay_ms(value: Option<&str>, fallback: i64) -> i64 {
    let number = js_number(value.unwrap_or_default());
    if number.is_finite() && number > 0.0 {
        number.floor() as i64
    } else {
        fallback
    }
}

/// `crypto.randomInt(min, max)`: `min <= n < max`. A UUID's random bits are the entropy.
pub fn random_int(min: i64, max: i64) -> i64 {
    if max <= min {
        return min;
    }
    let span = u128::try_from(max - min).unwrap_or(1);
    let bits = crate::game_data::season_window::next_uuid().as_u128();
    min + i64::try_from(bits % span).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_print_as_to_iso_string() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(1_760_356_800_000), "2025-10-13T12:00:00.000Z");
        assert_eq!(iso(1_760_356_860_123), "2025-10-13T12:01:00.123Z");
        // a leap day, and a date before 1970
        assert_eq!(iso(1_709_164_800_000), "2024-02-29T00:00:00.000Z");
        assert_eq!(iso(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn numbers_read_like_number() {
        assert_eq!(js_number(" 12 "), 12.0);
        assert_eq!(js_number(""), 0.0);
        assert!(js_number("abc").is_nan());
        assert!(js_number("inf").is_nan());
        assert_eq!(js_number("1e3"), 1000.0);
        assert_eq!(delay_ms(Some("2500.9"), 5), 2500);
        assert_eq!(delay_ms(Some("0"), 5), 5);
        assert_eq!(delay_ms(Some("-3"), 5), 5);
        assert_eq!(delay_ms(Some("x"), 5), 5);
        assert_eq!(delay_ms(None, 5), 5);
    }

    #[test]
    fn random_ints_stay_in_range() {
        for _ in 0..200 {
            let n = random_int(10, 15);
            assert!((10..15).contains(&n));
        }
        assert_eq!(random_int(4, 4), 4);
    }

    #[test]
    fn the_clock_moves_forward() {
        let clock = Clock::default();
        let before = clock.now_ms();
        clock.advance(60_000);
        assert!(clock.now_ms() >= before + 60_000);
    }
}
