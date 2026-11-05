//! What Node's answers look like on the wire: `JSON.stringify` text in the order Node builds its objects, ISO times
//! as `Date.toISOString` writes them, and the JavaScript parsing rules the settings and bodies are read with.

use std::fmt::Write as _;

use rifts_seam::UnixMillis;

/// `JSON.stringify` of a string: `"`, `\` and control characters escaped, nothing else (not `/`, not non-ASCII).
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

/// An object written field by field, so the order is Node's and not a map's.
#[derive(Debug, Clone, Default)]
pub struct JsonObject(String);

impl JsonObject {
    pub fn new() -> Self {
        Self(String::from("{"))
    }

    fn key(&mut self, key: &str) {
        if self.0.len() > 1 {
            self.0.push(',');
        }
        self.0.push_str(&json_string(key));
        self.0.push(':');
    }

    pub fn string(mut self, key: &str, value: &str) -> Self {
        self.key(key);
        self.0.push_str(&json_string(value));
        self
    }

    /// `undefined` fields are left out by `JSON.stringify`.
    pub fn optional_string(self, key: &str, value: Option<&str>) -> Self {
        match value {
            Some(value) => self.string(key, value),
            None => self,
        }
    }

    pub fn number(mut self, key: &str, value: impl std::fmt::Display) -> Self {
        self.key(key);
        let _ = write!(self.0, "{value}");
        self
    }

    pub fn boolean(mut self, key: &str, value: bool) -> Self {
        self.key(key);
        self.0.push_str(if value { "true" } else { "false" });
        self
    }

    /// Already-written JSON: an object, an array or a literal.
    pub fn raw(mut self, key: &str, json: &str) -> Self {
        self.key(key);
        self.0.push_str(json);
        self
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

/// `new Date(ms).toISOString()` for the years 0 to 9999.
pub fn iso_time(at: UnixMillis) -> String {
    let millis = at.0.rem_euclid(1000);
    let seconds = at.0.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        of_day / 3600,
        (of_day / 60) % 60,
        of_day % 60
    )
}

// Howard Hinnant's days-to-civil.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `String.prototype.trim`: JavaScript's white space and line terminators, which are not quite Rust's.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| {
        matches!(
            c,
            '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
                ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
        )
    })
}

pub const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// `Number.isSafeInteger`.
pub fn is_safe_integer(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER
}

/// `Number(text)` for a string: decimal with an exponent, `0x`/`0o`/`0b`, `Infinity`; blank is 0; anything else NaN.
pub fn js_number(text: &str) -> f64 {
    let text = js_trim(text);
    if text.is_empty() {
        return 0.0;
    }
    let radix = |prefix: &str, radix: u32| {
        text.get(..2)
            .filter(|head| head.eq_ignore_ascii_case(prefix))
            .map(|_| &text[2..])
            .filter(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_digit(radix)))
            .map(|digits| {
                digits.chars().fold(0f64, |acc, c| {
                    acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))
                })
            })
    };
    if let Some(value) = radix("0x", 16).or_else(|| radix("0o", 8)).or_else(|| radix("0b", 2)) {
        return value;
    }
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    if unsigned == "Infinity" {
        return if text.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    if !decimal_literal(unsigned) {
        return f64::NAN;
    }
    text.parse::<f64>().unwrap_or(f64::NAN)
}

// StrUnsignedDecimalLiteral: digits with an optional fraction, or a bare fraction, then an optional exponent.
fn decimal_literal(text: &str) -> bool {
    let (mantissa, exponent) = match text.find(['e', 'E']) {
        Some(at) => (&text[..at], Some(&text[at + 1..])),
        None => (text, None),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    let mantissa_ok = digits(whole) && digits(fraction) && !(whole.is_empty() && fraction.is_empty());
    let exponent_ok = exponent.is_none_or(|exponent| {
        let unsigned = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        !unsigned.is_empty() && digits(unsigned)
    });
    mantissa_ok && exponent_ok
}

/// `Number(value)` for a value from a JSON body (a missing field is `undefined`, NaN, and is the caller's).
pub fn js_number_value(value: &serde_json::Value) -> f64 {
    use serde_json::Value;
    match value {
        Value::Null => 0.0,
        Value::Bool(flag) => f64::from(u8::from(*flag)),
        Value::Number(number) => number.as_f64().unwrap_or(f64::NAN),
        Value::String(text) => js_number(text),
        // Through String(array): "" for [], the element's text for [x], "a,b" (NaN) for more
        Value::Array(items) => match items.as_slice() {
            [] | [Value::Null] => 0.0,
            [Value::Number(number)] => number.as_f64().unwrap_or(f64::NAN),
            [Value::String(text)] => js_number(text),
            [Value::Array(inner)] => js_number_value(&Value::Array(inner.clone())),
            _ => f64::NAN,
        },
        Value::Object(_) => f64::NAN,
    }
}

/// A JSON body field that `Number.isSafeInteger` accepts (`3`, `3.0` and `-0` do; `"3"` does not).
pub fn safe_integer(value: &serde_json::Value) -> Option<i64> {
    let number = value.as_number()?;
    if let Some(value) = number.as_i64() {
        return (value.unsigned_abs() <= MAX_SAFE_INTEGER as u64).then_some(value);
    }
    let value = number.as_f64()?;
    // In range by the check, so the cast is exact
    is_safe_integer(value).then_some(value as i64)
}

/// A random RFC 4122 version 4 UUID (`crypto.randomUUID`).
pub fn random_uuid() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format_uuid(&bytes))
}

pub fn format_uuid(bytes: &[u8; 16]) -> String {
    let hex = hex(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
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

    #[test]
    fn strings_escape_as_json_stringify() {
        assert_eq!(json_string("a\"b\\c/é\u{1}\u{8}\n"), r#""a\"b\\c/é\u0001\b\n""#);
    }

    #[test]
    fn objects_keep_their_order() {
        let json = JsonObject::new()
            .string("z", "1")
            .number("a", 2)
            .boolean("m", false)
            .optional_string("gone", None)
            .strings("urls", &["x".to_owned()])
            .finish();
        assert_eq!(json, r#"{"z":"1","a":2,"m":false,"urls":["x"]}"#);
        assert_eq!(JsonObject::new().finish(), "{}");
    }

    #[test]
    fn iso_times_match_to_iso_string() {
        assert_eq!(iso_time(UnixMillis(0)), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_time(UnixMillis(1_791_878_400_000)), "2026-10-13T08:00:00.000Z");
        assert_eq!(iso_time(UnixMillis(951_782_400_123)), "2000-02-29T00:00:00.123Z");
        assert_eq!(iso_time(UnixMillis(-1)), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn numbers_parse_as_javascript_does() {
        assert_eq!(js_number(" 12 "), 12.0);
        assert_eq!(js_number("0x10"), 16.0);
        assert_eq!(js_number("1e3"), 1000.0);
        assert_eq!(js_number("12.0"), 12.0);
        assert_eq!(js_number(".5"), 0.5);
        assert_eq!(js_number("-1"), -1.0);
        assert_eq!(js_number(""), 0.0);
        assert!(js_number("12px").is_nan());
        assert!(js_number("inf").is_nan());
        assert!(js_number("-0x10").is_nan());
        assert!(js_number(".").is_nan());
        assert_eq!(js_number("-Infinity"), f64::NEG_INFINITY);
        assert!(is_safe_integer(9_007_199_254_740_991.0));
        assert!(!is_safe_integer(9_007_199_254_740_992.0));
    }

    #[test]
    fn safe_integers_from_json() {
        let parse = |text: &str| safe_integer(&serde_json::from_str(text).unwrap());
        assert_eq!(parse("3"), Some(3));
        assert_eq!(parse("3.0"), Some(3));
        assert_eq!(parse("-4"), Some(-4));
        assert_eq!(parse("3.5"), None);
        assert_eq!(parse("\"3\""), None);
        assert_eq!(parse("9007199254740992"), None);
    }

    #[test]
    fn trim_is_javascripts() {
        assert_eq!(js_trim("\u{feff} x \u{2028}"), "x");
        assert_eq!(js_trim("\u{85}x"), "\u{85}x");
    }

    #[test]
    fn uuids_are_version_4() {
        let uuid = random_uuid().unwrap();
        assert_eq!(uuid.len(), 36);
        assert_eq!(&uuid[14..15], "4");
        assert!(matches!(&uuid[19..20], "8" | "9" | "a" | "b"));
    }
}
