// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The JavaScript rules the launcher and the transport already depend on: `JSON.stringify`'s escaping and field
//! order, `Date.toISOString`, `Number()` and `String.prototype.trim`.

use std::fmt::{Display, Write as _};

use rand::TryRng as _;
use rifts_seam::UnixMillis;
use serde_json::Value;

/// `Number.MAX_SAFE_INTEGER`.
pub const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// The characters `JSON.stringify` writes with a short escape.
const SHORT_ESCAPES: [(char, &str); 7] = [
    ('"', "\\\""),
    ('\\', "\\\\"),
    ('\u{8}', "\\b"),
    ('\u{c}', "\\f"),
    ('\n', "\\n"),
    ('\r', "\\r"),
    ('\t', "\\t"),
];

/// A JSON string literal as `JSON.stringify` writes it.
pub fn json_string(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for c in text.chars() {
        if let Some((_, escape)) = SHORT_ESCAPES.iter().find(|(plain, _)| *plain == c) {
            quoted.push_str(escape);
        } else if c < ' ' {
            // Other control characters as `\u00XX`, lower-case hex.
            let _ = write!(quoted, "\\u{:04x}", c as u32);
        } else {
            quoted.push(c);
        }
    }
    quoted.push('"');
    quoted
}

/// A JSON object written field by field, in call order (the order is part of the wire contract).
#[derive(Clone, Debug, Default)]
pub struct JsonObject(String);

impl JsonObject {
    /// An empty object to add fields to, in order.
    pub fn new() -> Self {
        Self("{".to_owned())
    }

    /// Appends `"key":` and then `value`, already JSON.
    fn field(mut self, key: &str, value: &str) -> Self {
        let first = self.0.ends_with('{');
        if !first {
            self.0.push(',');
        }
        self.0 += &json_string(key);
        self.0.push(':');
        self.0 += value;
        self
    }

    /// Adds a string field, escaped as `JSON.stringify` does.
    pub fn string(self, key: &str, value: &str) -> Self {
        self.field(key, &json_string(value))
    }

    /// Adds a string field when there is a value; nothing for `None`.
    pub fn optional_string(self, key: &str, value: Option<&str>) -> Self {
        let Some(text) = value else {
            return self;
        };
        self.string(key, text)
    }

    /// Adds a number field, written as its `Display`.
    pub fn number(self, key: &str, value: impl Display) -> Self {
        self.field(key, &value.to_string())
    }

    /// Adds a boolean field.
    pub fn boolean(self, key: &str, value: bool) -> Self {
        self.field(key, &value.to_string())
    }

    /// Adds a field whose value is already JSON text.
    pub fn raw(self, key: &str, json_text: &str) -> Self {
        self.field(key, json_text)
    }

    /// Adds an array of strings.
    pub fn strings(self, key: &str, values: &[String]) -> Self {
        let quoted = values.iter().map(|value| json_string(value));
        self.field(key, &json_array(quoted))
    }

    /// The finished JSON text.
    pub fn finish(self) -> String {
        let Self(mut text) = self;
        text.push('}');
        text
    }
}

/// A JSON array of items that are already JSON text.
pub fn json_array(items: impl IntoIterator<Item = String>) -> String {
    let joined = items.into_iter().collect::<Vec<_>>().join(",");
    format!("[{joined}]")
}

/// `Date.prototype.toISOString` for years 0 to 9999.
pub fn iso_time(at: UnixMillis) -> String {
    const DAY_MS: i64 = 86_400_000;
    let (days, in_day) = (at.0.div_euclid(DAY_MS), at.0.rem_euclid(DAY_MS));
    let (year, month, day) = civil_date(days);
    let (hours, minutes) = (in_day / 3_600_000, in_day / 60_000 % 60);
    let (seconds, millis) = (in_day / 1000 % 60, in_day % 1000);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z")
}

/// Where each month starts in a year counted from 1 March, so February, with the leap day, comes last.
const MARCH_YEAR_MONTHS: [i64; 12] = [0, 31, 61, 92, 122, 153, 184, 214, 245, 275, 306, 337];

/// Days since 1970-01-01 to a proleptic Gregorian date: whole 400-, 100-, 4- and 1-year spans from 0000-03-01.
fn civil_date(days: i64) -> (i64, i64, i64) {
    let since = days + 719_468;
    let eras = since.div_euclid(146_097);
    let mut left = since.rem_euclid(146_097);
    // The last century, the last leap cycle's year and the last year of an era each hold the leap day.
    let centuries = (left / 36_524).min(3);
    left -= centuries * 36_524;
    let cycles = left / 1461;
    left -= cycles * 1461;
    let years = (left / 365).min(3);
    left -= years * 365;
    let month_index = MARCH_YEAR_MONTHS.iter().rposition(|&start| start <= left).unwrap_or(0);
    let day = left - MARCH_YEAR_MONTHS[month_index] + 1;
    // Index 0 is March, 10 and 11 are the next year's January and February.
    let month = (month_index as i64 + 2) % 12 + 1;
    let year = eras * 400 + centuries * 100 + cycles * 4 + years + i64::from(month <= 2);
    (year, month, day)
}

/// What JavaScript's `trim` removes; U+0085 is not among them.
fn is_js_space(c: char) -> bool {
    const SINGLE: &[char] = &[
        '\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{a0}', '\u{1680}', '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}',
        '\u{3000}', '\u{feff}',
    ];
    ('\u{2000}'..='\u{200a}').contains(&c) || SINGLE.contains(&c)
}

/// `String.prototype.trim`: strips JavaScript's whitespace, not only Rust's.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

/// `Number.isSafeInteger`.
pub fn is_safe_integer(value: f64) -> bool {
    value.is_finite() && value.trunc() == value && value.abs() <= MAX_SAFE_INTEGER
}

/// `Number(text)`.
pub fn js_number(text: &str) -> f64 {
    let text = js_trim(text);
    match text {
        "" => 0.0,
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ => radix_literal(text)
            .or_else(|| is_decimal_literal(text).then(|| text.parse().unwrap_or(f64::NAN)))
            .unwrap_or(f64::NAN),
    }
}

/// `0x`, `0o` and `0b` literals; `Some(NaN)` for a prefix with no valid digits.
fn radix_literal(text: &str) -> Option<f64> {
    let mut chars = text.chars();
    if chars.next() != Some('0') {
        return None;
    }
    let radix = match chars.next()?.to_ascii_lowercase() {
        'x' => 16,
        'o' => 8,
        'b' => 2,
        _ => return None,
    };
    let digits = chars.as_str();
    let value = digits.chars().try_fold(0.0, |total: f64, c| {
        Some(total * f64::from(radix) + f64::from(c.to_digit(radix)?))
    });
    Some(value.filter(|_| !digits.is_empty()).unwrap_or(f64::NAN))
}

/// An optional sign, digits with an optional fraction (or a bare fraction), and an optional exponent.
fn is_decimal_literal(text: &str) -> bool {
    let all_digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(at) => (&unsigned[..at], Some(&unsigned[at + 1..])),
        None => (unsigned, None),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mantissa_ok = all_digits(whole) && all_digits(fraction) && !(whole.is_empty() && fraction.is_empty());
    let exponent_ok = exponent.is_none_or(|power| {
        let power = power.strip_prefix(['+', '-']).unwrap_or(power);
        !power.is_empty() && all_digits(power)
    });
    mantissa_ok && exponent_ok
}

/// `Number(value)` for a parsed JSON value.
pub fn js_number_value(value: &Value) -> f64 {
    match value {
        Value::Number(number) => number.as_f64().unwrap_or(f64::NAN),
        Value::String(text) => js_number(text),
        Value::Bool(true) => 1.0,
        Value::Bool(false) | Value::Null => 0.0,
        Value::Object(_) => f64::NAN,
        // An array goes through its string form, which only a lone element (or none) keeps numeric.
        Value::Array(items) => match &items[..] {
            [] | [Value::Null] => 0.0,
            [only] if matches!(only, Value::Number(_) | Value::String(_) | Value::Array(_)) => js_number_value(only),
            _ => f64::NAN,
        },
    }
}

/// `Number.isSafeInteger` for JSON numbers; strings never count.
pub fn safe_integer(value: &Value) -> Option<i64> {
    let number = value.as_number()?;
    match (number.as_i64(), number.is_u64()) {
        (Some(integer), _) => (integer.unsigned_abs() < 1 << 53).then_some(integer),
        (None, true) => None,
        // Bounded by MAX_SAFE_INTEGER, so the cast is exact.
        (None, false) => number
            .as_f64()
            .filter(|&float| is_safe_integer(float))
            .map(|float| float as i64),
    }
}

/// A version 4 UUID from the system's random source (`crypto.randomUUID`).
pub fn random_uuid() -> Result<String, rand::rngs::SysError> {
    let mut bytes = [0_u8; 16];
    rand::rngs::SysRng.try_fill_bytes(&mut bytes)?;
    // RFC 4122: version 4 in the high nibble of byte 6, variant 10 in the top bits of byte 8.
    bytes[6] = 0x40 | (bytes[6] & 0x0f);
    bytes[8] = 0x80 | (bytes[8] & 0x3f);
    Ok(format_uuid(&bytes))
}

/// The 16 bytes as a lower-case hyphenated UUID.
pub fn format_uuid(bytes: &[u8; 16]) -> String {
    let mut text = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            text.push('-');
        }
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Lower-case hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|&b| [b >> 4, b & 15])
        .filter_map(|n| char::from_digit(u32::from(n), 16))
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn json_string_matches_stringify() {
        let cases = [
            ("a\"b\\c/é\u{1}\u{8}\n", "\"a\\\"b\\\\c/é\\u0001\\b\\n\""),
            ("\u{c}\t\r\u{1f}", "\"\\f\\t\\r\\u001f\""),
        ];
        for (plain, quoted) in cases {
            assert_eq!(json_string(plain), quoted);
        }
    }

    #[test]
    fn objects_keep_call_order() {
        let built = JsonObject::new()
            .string("b", "x")
            .number("a", 2)
            .boolean("c", false)
            .optional_string("skip", None)
            .optional_string("d", Some("y"))
            .raw("e", "{}")
            .strings("f", &["g".to_owned(), "h\"".to_owned()])
            .finish();
        assert_eq!(built, r#"{"b":"x","a":2,"c":false,"d":"y","e":{},"f":["g","h\""]}"#);
        assert_eq!(JsonObject::new().finish(), "{}");
        assert_eq!(json_array(Vec::new()), "[]");
        assert_eq!(json_array(["1".to_owned(), "\"a\"".to_owned()]), r#"[1,"a"]"#);
    }

    #[test]
    fn iso_times() {
        let cases = [
            (0, "1970-01-01T00:00:00.000Z"),
            (1_791_878_400_000, "2026-10-13T08:00:00.000Z"),
            (951_782_400_123, "2000-02-29T00:00:00.123Z"),
            (-1, "1969-12-31T23:59:59.999Z"),
        ];
        for (millis, text) in cases {
            assert_eq!(iso_time(UnixMillis(millis)), text);
        }
    }

    #[test]
    fn numbers_follow_javascript() {
        let exact = [
            ("", 0.0),
            ("  \u{feff}", 0.0),
            ("1e2", 100.0),
            ("0x12c", 300.0),
            ("0X1f", 31.0),
            ("0o17", 15.0),
            ("0b101", 5.0),
            (".5", 0.5),
            ("1.", 1.0),
            (" -2.5E-1 ", -0.25),
            ("+7", 7.0),
            ("-Infinity", f64::NEG_INFINITY),
            ("+Infinity", f64::INFINITY),
        ];
        for (text, number) in exact {
            assert_eq!(js_number(text), number, "{text:?}");
        }
        for nan in [
            "-0x10", ".", "inf", "12px", "0x", "0xg", "1e", "--1", "NaN", "1_000", "infinity",
        ] {
            assert!(js_number(nan).is_nan(), "{nan}");
        }
    }

    #[test]
    fn number_of_json_values() {
        let numeric = [
            (json!(null), 0.0),
            (json!(true), 1.0),
            (json!(" 5 "), 5.0),
            (json!([]), 0.0),
            (json!([null]), 0.0),
            (json!([4]), 4.0),
            (json!(["6"]), 6.0),
            (json!([[8]]), 8.0),
        ];
        for (value, number) in numeric {
            assert_eq!(js_number_value(&value), number, "{value}");
        }
        for value in [json!([1, 2]), json!([true]), json!({})] {
            assert!(js_number_value(&value).is_nan(), "{value}");
        }
    }

    #[test]
    fn safe_integers() {
        let cases = [
            (json!(3), Some(3)),
            (json!(3.0), Some(3)),
            (json!(-4), Some(-4)),
            (json!(9_007_199_254_740_991_i64), Some(9_007_199_254_740_991)),
            (json!(3.5), None),
            (json!("3"), None),
            (json!(9_007_199_254_740_992_i64), None),
            (json!(u64::MAX), None),
        ];
        for (value, expected) in cases {
            assert_eq!(safe_integer(&value), expected, "{value}");
        }
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
        assert_eq!(uuid.len(), 36);
        assert_eq!(&uuid[14..15], "4");
        assert!("89ab".contains(&uuid[19..20]));
        assert!(
            uuid.chars()
                .all(|c| c == '-' || (c.is_ascii_hexdigit() && !c.is_ascii_uppercase()))
        );
        assert_ne!(random_uuid().unwrap(), uuid);
        let counting: [u8; 16] = std::array::from_fn(|i| (i * 17) as u8);
        assert_eq!(format_uuid(&counting), "00112233-4455-6677-8899-aabbccddeeff");
        assert_eq!(hex(&[0, 0xab]), "00ab");
    }
}
