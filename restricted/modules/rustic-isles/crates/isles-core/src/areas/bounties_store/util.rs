//! JavaScript's clock and value habits the Node code leans on: `Date.toISOString`, `Number(x) || 0`, plain objects.

use serde_json::{Map, Value};
use service_kit::js;

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// `Date.now()`, which the golden tests pin.
pub fn now_ms() -> i64 {
    crate::game_data::season_window::now_ms()
}

/// `new Date(ms).toISOString()`.
pub fn iso(ms: i64) -> String {
    crate::game_data::season_window::to_iso_string(ms)
}

pub fn now_iso() -> String {
    iso(now_ms())
}

/// `Date.parse(text)`; `None` is NaN.
pub fn parse_ms(text: &str) -> Option<i64> {
    js::date_parse(text)
}

/// `Date.parse(text)` where a NaN compares false against everything.
pub fn parse_ms_f64(text: &str) -> f64 {
    parse_ms(text).map_or(f64::NAN, |ms| ms as f64)
}

/// `Number(x) || 0`.
pub fn number_or_zero(value: Option<&Value>) -> f64 {
    let n = js::number(value);
    if n.is_nan() || n == 0.0 { 0.0 } else { n }
}

pub fn is_plain_object(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Object(_)))
}

/// `.length` of a JavaScript string: UTF-16 code units.
pub fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

pub fn object(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}

/// A JavaScript number as JSON: whole numbers without a fraction.
pub fn num(value: f64) -> Value {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        Value::from(value as i64)
    } else {
        serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_matches_to_iso_string() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(1_788_966_000_123), "2026-09-09T15:00:00.123Z");
        assert_eq!(iso(-1), "1969-12-31T23:59:59.999Z");
        assert_eq!(iso(4_070_908_800_000), "2099-01-01T00:00:00.000Z");
        for text in [
            "2024-02-29T12:34:56.789Z",
            "2026-10-22T17:00:00.000Z",
            "2000-12-31T23:59:59.999Z",
        ] {
            assert_eq!(iso(parse_ms(text).unwrap()), text);
        }
    }

    #[test]
    fn number_or_zero_is_js() {
        assert_eq!(number_or_zero(Some(&Value::from("x"))), 0.0);
        assert_eq!(number_or_zero(Some(&Value::from("7"))), 7.0);
        assert_eq!(number_or_zero(None), 0.0);
    }
}
