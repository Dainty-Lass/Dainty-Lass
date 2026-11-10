// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! JavaScript's conversions and comparisons on JSON values, for ports that must answer as the Node code did.

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

/// What JavaScript's `\s` and `trim` treat as white space: Unicode white space less U+0085, plus the BOM.
pub fn is_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// `String.prototype.trim`.
pub fn trim(text: &str) -> &str {
    text.trim_matches(is_space)
}

/// `Number(value)`; a missing value is `undefined`, so NaN.
pub fn number(value: Option<&Value>) -> f64 {
    match value {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => {
            let t = trim(s);
            if t.is_empty() {
                return 0.0;
            }
            if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return f64::NAN;
                }
                return u128::from_str_radix(hex, 16).map_or_else(
                    |_| {
                        hex.bytes().fold(0.0_f64, |acc, b| {
                            let d = match b {
                                b'0'..=b'9' => b - b'0',
                                b'a'..=b'f' => b - b'a' + 10,
                                b'A'..=b'F' => b - b'A' + 10,
                                _ => 0,
                            };
                            acc * 16.0 + f64::from(d)
                        })
                    },
                    |v| v as f64,
                );
            }
            match t {
                "Infinity" | "+Infinity" => f64::INFINITY,
                "-Infinity" => f64::NEG_INFINITY,
                _ => {
                    if !t.bytes().any(|b| b.is_ascii_digit())
                        || !t
                            .bytes()
                            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'+' | b'-' | b'e' | b'E'))
                    {
                        return f64::NAN;
                    }
                    t.parse::<f64>().unwrap_or(f64::NAN)
                }
            }
        }
        Some(Value::Array(items)) => match items.as_slice() {
            [] => 0.0,
            [one] => {
                let s = string(Some(one));
                number(Some(&Value::String(s)))
            }
            _ => f64::NAN,
        },
        Some(Value::Object(_)) => f64::NAN,
    }
}

/// `String(value)`; a missing value is `"undefined"`.
pub fn string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(Value::Null) => "null".to_owned(),
        Some(Value::Bool(true)) => "true".to_owned(),
        Some(Value::Bool(false)) => "false".to_owned(),
        Some(Value::Number(n)) => n.as_f64().map_or_else(|| "NaN".to_owned(), crate::jsjson::number),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_owned(),
    }
}

/// `a === b` (arrays and objects are never equal).
pub fn strict_eq(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) | (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(ba)), Some(Value::Bool(bb))) => ba == bb,
        (Some(Value::Number(na)), Some(Value::Number(nb))) => match (na.as_f64(), nb.as_f64()) {
            (Some(fa), Some(fb)) => fa == fb,
            _ => false,
        },
        (Some(Value::String(sa)), Some(Value::String(sb))) => sa == sb,
        _ => false,
    }
}

/// `value == null`.
pub fn is_nullish(value: Option<&Value>) -> bool {
    matches!(value, None | Some(Value::Null))
}

/// JavaScript truthiness.
pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::Bool(true) | Value::Array(_) | Value::Object(_)) => true,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
    }
}

/// `typeof value === "number"`.
pub fn is_number(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Number(_)))
}

/// The value as an integer if it is a number within `Number.MAX_SAFE_INTEGER`.
pub fn safe_integer(value: Option<&Value>) -> Option<i64> {
    let f = value?.as_f64()?;
    if f.is_finite() && f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0 {
        Some(f as i64)
    } else {
        None
    }
}

/// `Date.parse(text)` in Unix milliseconds for the formats the services meet; date-only text is UTC, date-time local.
pub fn date_parse(text: &str) -> Option<i64> {
    let s = trim(text);
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    if let Ok(dt) = DateTime::parse_from_rfc2822(s) {
        return Some(dt.timestamp_millis());
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Local
                .from_local_datetime(&ndt)
                .earliest()
                .map(|dt| dt.timestamp_millis());
        }
    }
    if let Ok(nd) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return nd.and_hms_opt(0, 0, 0).map(|ndt| ndt.and_utc().timestamp_millis());
    }
    None
}

/// `Date#toISOString` of Unix milliseconds; a time chrono can't hold writes the epoch.
pub fn iso_string(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// What `encodeURIComponent` leaves as it is besides letters and digits.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// `encodeURIComponent(text)`.
pub fn encode_uri_component(text: &str) -> String {
    utf8_percent_encode(text, URI_COMPONENT).to_string()
}

/// `URLSearchParams`' encoding: `A-Za-z0-9*-._` kept, space `+`, the rest `%XX`.
pub fn search_params_encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// `decodeURIComponent(text)`; `None` where it would throw.
pub fn decode_uri_component(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = hex_val(bytes[i + 1])?;
            let lo = hex_val(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encodes_as_javascript_does() {
        assert_eq!(encode_uri_component("a b/é-_.!~*'()"), "a%20b%2F%C3%A9-_.!~*'()");
        assert_eq!(search_params_encode("a b*c-d.e_f~!"), "a+b*c-d.e_f%7E%21");
    }

    #[test]
    fn coerces_values_to_number_and_string() {
        assert!(number(None).is_nan());
        assert_eq!(number(Some(&Value::Null)), 0.0);
        assert_eq!(number(Some(&json!(true))), 1.0);
        assert_eq!(number(Some(&json!("  0x10 "))), 16.0);
        assert_eq!(number(Some(&json!(""))), 0.0);
        assert_eq!(number(Some(&json!([]))), 0.0);
        assert_eq!(number(Some(&json!(["42"]))), 42.0);
        assert!(number(Some(&json!([1, 2]))).is_nan());
        assert!(number(Some(&json!("inf"))).is_nan());
        assert_eq!(number(Some(&json!("Infinity"))), f64::INFINITY);

        assert_eq!(string(None), "undefined");
        assert_eq!(string(Some(&Value::Null)), "null");
        assert_eq!(string(Some(&json!([1, null, "a"]))), "1,,a");
        assert_eq!(string(Some(&json!({"a": 1}))), "[object Object]");
    }

    #[test]
    fn checks_strict_eq_truthy_and_safe_integer() {
        assert!(strict_eq(None, None));
        assert!(strict_eq(Some(&json!(1)), Some(&json!(1.0))));
        assert!(!strict_eq(Some(&json!([])), Some(&json!([]))));

        assert!(!truthy(None));
        assert!(!truthy(Some(&json!(0))));
        assert!(!truthy(Some(&json!(""))));
        assert!(truthy(Some(&json!([]))));

        assert_eq!(
            safe_integer(Some(&json!(9_007_199_254_740_991_i64))),
            Some(9_007_199_254_740_991)
        );
        assert_eq!(safe_integer(Some(&json!(9_007_199_254_740_992_i64))), None);
        assert_eq!(safe_integer(Some(&json!(1.5))), None);
    }

    #[test]
    fn parses_dates_and_decodes_uri_components() {
        assert_eq!(date_parse("1970-01-01"), Some(0));
        assert_eq!(date_parse("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(decode_uri_component("hello%20world%2B1"), Some("hello world+1".into()));
        assert_eq!(decode_uri_component("%E0%A4%A"), None);
        assert_eq!(decode_uri_component("%FF"), None);
    }
}
