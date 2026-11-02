// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The JavaScript rules the launcher and the transport already depend on: `JSON.stringify`'s escaping and field
//! order, `Date.toISOString`, `Number()` and `String.prototype.trim`.

use std::fmt::{Display, Write as _};

use rand::TryRng as _;
use rifts_seam::UnixMillis;
use serde_json::Value;

pub const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// A JSON string literal as `JSON.stringify` writes it.
pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON object written field by field, in call order (the order is part of the wire contract).
#[derive(Debug, Clone, Default)]
pub struct JsonObject(String);

impl JsonObject {
    pub fn new() -> Self {
        Self(String::from("{"))
    }

    fn key(mut self, key: &str) -> Self {
        if !self.0.ends_with('{') {
            self.0.push(',');
        }
        self.0.push_str(&json_string(key));
        self.0.push(':');
        self
    }

    pub fn string(self, key: &str, value: &str) -> Self {
        let mut this = self.key(key);
        this.0.push_str(&json_string(value));
        this
    }

    pub fn optional_string(self, key: &str, value: Option<&str>) -> Self {
        match value {
            Some(value) => self.string(key, value),
            None => self,
        }
    }

    pub fn number(self, key: &str, value: impl Display) -> Self {
        let mut this = self.key(key);
        let _ = write!(this.0, "{value}");
        this
    }

    pub fn boolean(self, key: &str, value: bool) -> Self {
        self.raw(key, if value { "true" } else { "false" })
    }

    pub fn raw(self, key: &str, json_text: &str) -> Self {
        let mut this = self.key(key);
        this.0.push_str(json_text);
        this
    }

    pub fn strings(self, key: &str, values: &[String]) -> Self {
        let array = json_array(values.iter().map(|value| json_string(value)));
        self.raw(key, &array)
    }

    pub fn finish(mut self) -> String {
        self.0.push('}');
        self.0
    }
}

pub fn json_array(items: impl IntoIterator<Item = String>) -> String {
    let mut out = String::from("[");
    for (index, item) in items.into_iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&item);
    }
    out.push(']');
    out
}

/// `Date.prototype.toISOString` for years 0 to 9999.
pub fn iso_time(at: UnixMillis) -> String {
    let millis = at.0.rem_euclid(1000);
    let seconds = at.0.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_date(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date, counting in 400-year eras that start on 1 March.
fn civil_date(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let of_era = shifted.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * of_year + 2) / 153;
    let day = of_year - (153 * march_month + 2) / 5 + 1;
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// What JavaScript's `trim` removes; U+0085 is not among them.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

pub fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

pub fn is_safe_integer(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER
}

/// `Number(text)`.
pub fn js_number(text: &str) -> f64 {
    let text = js_trim(text);
    if text.is_empty() {
        return 0.0;
    }
    if let Some(value) = radix_literal(text) {
        return value;
    }
    match text {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    if !is_decimal_literal(text) {
        return f64::NAN;
    }
    text.parse::<f64>().unwrap_or(f64::NAN)
}

/// `0x`, `0o` and `0b` literals; `Some(NaN)` for a prefix with no valid digits.
fn radix_literal(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'0' {
        return None;
    }
    let radix = match bytes[1] {
        b'x' | b'X' => 16,
        b'o' | b'O' => 8,
        b'b' | b'B' => 2,
        _ => return None,
    };
    let digits = &text[2..];
    if digits.is_empty() {
        return Some(f64::NAN);
    }
    let mut value = 0.0_f64;
    for c in digits.chars() {
        match c.to_digit(radix) {
            Some(digit) => value = value * f64::from(radix) + f64::from(digit),
            None => return Some(f64::NAN),
        }
    }
    Some(value)
}

/// An optional sign, digits with an optional fraction (or a bare fraction), and an optional exponent.
fn is_decimal_literal(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        at += 1;
    }
    let integer = count_digits(&bytes[at..]);
    at += integer;
    let mut fraction = 0;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        fraction = count_digits(&bytes[at..]);
        at += fraction;
    }
    if integer == 0 && fraction == 0 {
        return false;
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let exponent = count_digits(&bytes[at..]);
        if exponent == 0 {
            return false;
        }
        at += exponent;
    }
    at == bytes.len()
}

fn count_digits(bytes: &[u8]) -> usize {
    bytes.iter().take_while(|b| b.is_ascii_digit()).count()
}

/// `Number(value)` for a parsed JSON value.
pub fn js_number_value(value: &Value) -> f64 {
    match value {
        Value::Null => 0.0,
        Value::Bool(flag) => f64::from(u8::from(*flag)),
        Value::Number(number) => number.as_f64().unwrap_or(f64::NAN),
        Value::String(text) => js_number(text),
        // An array goes through its string form, which only a lone element (or none) keeps numeric.
        Value::Array(items) => match items.as_slice() {
            [] | [Value::Null] => 0.0,
            [only @ (Value::Number(_) | Value::String(_) | Value::Array(_))] => js_number_value(only),
            _ => f64::NAN,
        },
        Value::Object(_) => f64::NAN,
    }
}

/// `Number.isSafeInteger` for JSON numbers; strings never count.
pub fn safe_integer(value: &Value) -> Option<i64> {
    let Value::Number(number) = value else {
        return None;
    };
    if let Some(integer) = number.as_i64() {
        return (integer.unsigned_abs() <= 9_007_199_254_740_991).then_some(integer);
    }
    if number.is_u64() {
        return None;
    }
    let float = number.as_f64()?;
    // Bounded by MAX_SAFE_INTEGER, so the cast is exact.
    #[allow(clippy::cast_possible_truncation)]
    is_safe_integer(float).then_some(float as i64)
}

/// A version 4 UUID from the system's random source (`crypto.randomUUID`).
pub fn random_uuid() -> Result<String, rand::rngs::SysError> {
    let mut bytes = [0_u8; 16];
    rand::rngs::SysRng.try_fill_bytes(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format_uuid(&bytes))
}

pub fn format_uuid(bytes: &[u8; 16]) -> String {
    let text = hex(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &text[0..8],
        &text[8..12],
        &text[12..16],
        &text[16..20],
        &text[20..32]
    )
}

pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_string_matches_stringify() {
        assert_eq!(json_string("a\"b\\c/é\u{1}\u{8}\n"), "\"a\\\"b\\\\c/é\\u0001\\b\\n\"");
        assert_eq!(json_string("\u{c}\t\r\u{1f}"), "\"\\f\\t\\r\\u001f\"");
    }

    #[test]
    fn objects_keep_call_order() {
        let text = JsonObject::new()
            .string("b", "x")
            .number("a", 2)
            .boolean("c", false)
            .optional_string("skip", None)
            .optional_string("d", Some("y"))
            .raw("e", "{}")
            .strings("f", &["g".to_owned(), "h\"".to_owned()])
            .finish();
        assert_eq!(text, r#"{"b":"x","a":2,"c":false,"d":"y","e":{},"f":["g","h\""]}"#);
        assert_eq!(JsonObject::new().finish(), "{}");
        assert_eq!(json_array(Vec::new()), "[]");
        assert_eq!(json_array(["1".to_owned(), "\"a\"".to_owned()]), r#"[1,"a"]"#);
    }

    #[test]
    fn iso_times() {
        assert_eq!(iso_time(UnixMillis(0)), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_time(UnixMillis(1_791_878_400_000)), "2026-10-13T08:00:00.000Z");
        assert_eq!(iso_time(UnixMillis(951_782_400_123)), "2000-02-29T00:00:00.123Z");
        assert_eq!(iso_time(UnixMillis(-1)), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn numbers_follow_javascript() {
        assert_eq!(js_number(""), 0.0);
        assert_eq!(js_number("  \u{feff}"), 0.0);
        assert_eq!(js_number("1e2"), 100.0);
        assert_eq!(js_number("0x12c"), 300.0);
        assert_eq!(js_number("0X1f"), 31.0);
        assert_eq!(js_number("0o17"), 15.0);
        assert_eq!(js_number("0b101"), 5.0);
        assert_eq!(js_number(".5"), 0.5);
        assert_eq!(js_number("1."), 1.0);
        assert_eq!(js_number(" -2.5E-1 "), -0.25);
        assert_eq!(js_number("+7"), 7.0);
        assert_eq!(js_number("-Infinity"), f64::NEG_INFINITY);
        assert_eq!(js_number("+Infinity"), f64::INFINITY);
        for nan in [
            "-0x10", ".", "inf", "12px", "0x", "0xg", "1e", "--1", "NaN", "1_000", "infinity",
        ] {
            assert!(js_number(nan).is_nan(), "{nan}");
        }
    }

    #[test]
    fn number_of_json_values() {
        assert_eq!(js_number_value(&json!(null)), 0.0);
        assert_eq!(js_number_value(&json!(true)), 1.0);
        assert_eq!(js_number_value(&json!(" 5 ")), 5.0);
        assert_eq!(js_number_value(&json!([])), 0.0);
        assert_eq!(js_number_value(&json!([null])), 0.0);
        assert_eq!(js_number_value(&json!([4])), 4.0);
        assert_eq!(js_number_value(&json!(["6"])), 6.0);
        assert_eq!(js_number_value(&json!([[8]])), 8.0);
        assert!(js_number_value(&json!([1, 2])).is_nan());
        assert!(js_number_value(&json!([true])).is_nan());
        assert!(js_number_value(&json!({})).is_nan());
    }

    #[test]
    fn safe_integers() {
        assert_eq!(safe_integer(&json!(3)), Some(3));
        assert_eq!(safe_integer(&json!(3.0)), Some(3));
        assert_eq!(safe_integer(&json!(-4)), Some(-4));
        assert_eq!(
            safe_integer(&json!(9_007_199_254_740_991_i64)),
            Some(9_007_199_254_740_991)
        );
        assert_eq!(safe_integer(&json!(3.5)), None);
        assert_eq!(safe_integer(&json!("3")), None);
        assert_eq!(safe_integer(&json!(9_007_199_254_740_992_i64)), None);
        assert_eq!(safe_integer(&json!(u64::MAX)), None);
        assert!(is_safe_integer(MAX_SAFE_INTEGER));
        assert!(!is_safe_integer(f64::NAN));
    }

    #[test]
    fn trims_javascript_space() {
        assert_eq!(js_trim("\u{feff}\u{2028} a b \u{3000}"), "a b");
        assert_eq!(js_trim("\u{85}a"), "\u{85}a");
    }

    #[test]
    fn uuids() {
        let uuid = random_uuid().unwrap();
        let bytes = uuid.as_bytes();
        assert_eq!(uuid.len(), 36);
        assert_eq!(bytes[14], b'4');
        assert!(matches!(bytes[19], b'8' | b'9' | b'a' | b'b'));
        assert!(
            uuid.chars()
                .all(|c| c == '-' || c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
        assert_ne!(random_uuid().unwrap(), uuid);
        let fixed: [u8; 16] = core::array::from_fn(|i| u8::try_from(i * 17).unwrap());
        assert_eq!(format_uuid(&fixed), "00112233-4455-6677-8899-aabbccddeeff");
        assert_eq!(hex(&[0, 0xab]), "00ab");
    }
}
