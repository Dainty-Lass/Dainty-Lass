// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Reading a source field that may be absent, null, text, a number, a boolean, an array or an object (spec §5).
//! Node stored the same field several ways over its versions; each reader accepts them all.

use chrono::{DateTime, Datelike as _};
use serde_json::{Map, Value};

pub(crate) type Doc = Map<String, Value>;

/// Wherever a row needs a time and the source has none.
pub const DEFAULT_TIME: &str = "2026-01-01T00:00:00.000Z";

/// The first of `keys` present in the document, whatever its value: a present `null` hides the later keys (P19).
pub(crate) fn pick<'a>(doc: &'a Doc, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| doc.get(*k))
}

/// `Date#toISOString` of epoch milliseconds; out of range reads as the epoch.
pub fn iso_ms(ms: i64) -> String {
    let Some(at) = DateTime::from_timestamp_millis(ms) else {
        return "1970-01-01T00:00:00.000Z".into();
    };
    let year = at.year();
    let rest = at.format("-%m-%dT%H:%M:%S%.3fZ");
    // JavaScript's expanded years outside 0..=9999
    if (0..=9999).contains(&year) {
        format!("{year:04}{rest}")
    } else if year < 0 {
        format!("-{:06}{rest}", year.unsigned_abs())
    } else {
        format!("+{year:06}{rest}")
    }
}

fn rfc3339_ms(text: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(text).ok().map(|t| t.timestamp_millis())
}

fn number_text(n: &serde_json::Number) -> String {
    n.to_string()
}

/// Text or none: non-empty text, or a number's JSON text.
pub(crate) fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(number_text(n)),
        _ => None,
    }
}

/// Text or default: any text (`""` included), or a number's JSON text.
pub(crate) fn text_or(v: Option<&Value>, default: &str) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => number_text(n),
        _ => default.to_owned(),
    }
}

fn number_int(n: &serde_json::Number) -> i64 {
    if let Some(i) = n.as_i64() {
        i
    } else if n.as_u64().is_some() {
        i64::MAX
    } else {
        // `as` truncates toward zero and saturates
        n.as_f64().map_or(0, |f| f as i64)
    }
}

pub(crate) fn int_or(v: Option<&Value>, default: i64) -> i64 {
    match v {
        Some(Value::Number(n)) => number_int(n),
        Some(Value::String(s)) => s.parse::<i64>().unwrap_or(default),
        Some(Value::Bool(b)) => i64::from(*b),
        _ => default,
    }
}

pub(crate) fn float_or(v: Option<&Value>, default: f64) -> f64 {
    match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(default),
        // Rust's parser takes `inf` and `NaN` (P13)
        Some(Value::String(s)) => s.parse::<f64>().unwrap_or(default),
        _ => default,
    }
}

pub(crate) fn bool_or(v: Option<&Value>, default: bool) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => i != 0,
            (None, Some(u)) => u != 0,
            // A fractional number has no integer value
            _ => false,
        },
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true") || s == "1",
        _ => default,
    }
}

/// Time or none, as `toISOString` text where it can be read as a time; other text kept as it came.
pub(crate) fn time(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                None
            } else if let Some(ms) = rfc3339_ms(s) {
                Some(iso_ms(ms))
            } else if let Ok(ms) = s.parse::<i64>() {
                Some(iso_ms(ms))
            } else {
                Some(s.to_owned())
            }
        }
        Value::Number(n) => {
            let n = number_int(n);
            // Below 10^10 it is seconds (Node wrote both)
            let ms = if n.unsigned_abs() < 10_000_000_000 {
                n.saturating_mul(1000)
            } else {
                n
            };
            Some(iso_ms(ms))
        }
        Value::Object(o) => o.get("$date").and_then(|d| time(Some(d))),
        _ => None,
    }
}

pub(crate) fn time_or(v: Option<&Value>, default: &str) -> String {
    time(v).unwrap_or_else(|| default.to_owned())
}

/// Epoch milliseconds or default; no seconds heuristic here.
pub(crate) fn ms_or(v: Option<&Value>, default: i64) -> i64 {
    match v {
        Some(Value::Number(n)) => number_int(n),
        Some(Value::String(s)) => rfc3339_ms(s).or_else(|| s.parse::<i64>().ok()).unwrap_or(default),
        Some(Value::Object(o)) => o.get("$date").map_or(default, |d| ms_or(Some(d), default)),
        _ => default,
    }
}

/// A JSON field stored natively or as JSON text, as a value.
pub(crate) fn value_or(v: Option<&Value>, default: Value) -> Value {
    match v {
        None | Some(Value::Null) => default,
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(default),
        Some(other) => other.clone(),
    }
}

/// The same field as JSON text: a string that parses is kept verbatim, spacing included (P16).
pub(crate) fn json_text_or(v: Option<&Value>, default: &str) -> String {
    match v {
        None | Some(Value::Null) => default.to_owned(),
        Some(Value::String(s)) => {
            if serde_json::from_str::<Value>(s).is_ok() {
                s.clone()
            } else {
                default.to_owned()
            }
        }
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn t(v: Value) -> Option<String> {
        time(Some(&v))
    }

    #[test]
    fn times() {
        assert_eq!(t(json!("2026-03-01T10:00:00Z")).unwrap(), "2026-03-01T10:00:00.000Z");
        assert_eq!(t(json!(1_700_000_000)).unwrap(), "2023-11-14T22:13:20.000Z");
        assert_eq!(t(json!(1_700_000_000_000_i64)).unwrap(), "2023-11-14T22:13:20.000Z");
        assert_eq!(t(json!("1700000000000")).unwrap(), "2023-11-14T22:13:20.000Z");
        assert_eq!(t(json!(" yesterday ")).unwrap(), "yesterday");
        assert_eq!(
            t(json!({"$date": "2026-03-01T10:00:00Z"})).unwrap(),
            "2026-03-01T10:00:00.000Z"
        );
        assert_eq!(t(json!({"other": 1})), None);
        assert_eq!(t(json!("  ")), None);
        assert_eq!(t(json!(i64::MAX)).unwrap(), "1970-01-01T00:00:00.000Z");
        assert_eq!(time_or(None, DEFAULT_TIME), DEFAULT_TIME);
        assert_eq!(ms_or(Some(&json!("2026-03-01T10:00:00Z")), 0), 1_772_359_200_000);
        assert_eq!(ms_or(Some(&json!(1_700_000_000)), 0), 1_700_000_000);
        assert_eq!(ms_or(Some(&json!("x")), 7), 7);
    }

    #[test]
    fn numbers_and_booleans() {
        let i = |v: Value| int_or(Some(&v), -1);
        assert_eq!(i(json!("12")), 12);
        assert_eq!(i(json!("1.5")), -1);
        assert_eq!(i(json!(2.9)), 2);
        assert_eq!(i(json!(-2.9)), -2);
        assert_eq!(i(json!(true)), 1);
        assert_eq!(i(json!(1e300)), i64::MAX);
        let b = |v: Value| bool_or(Some(&v), true);
        assert!(b(json!("TRUE")));
        assert!(!b(json!("no")));
        assert!(b(json!(null)));
        assert!(!b(json!(1.5)));
        assert!(b(json!(3)));
        assert!(float_or(Some(&json!("inf")), 0.0).is_infinite());
    }

    #[test]
    fn text_readers() {
        assert_eq!(text(Some(&json!(123))).unwrap(), "123");
        assert_eq!(text(Some(&json!(1.5))).unwrap(), "1.5");
        assert_eq!(text(Some(&json!(" "))).unwrap(), " ");
        assert_eq!(text(Some(&json!(""))), None);
        assert_eq!(text(Some(&json!(true))), None);
        assert_eq!(text_or(Some(&json!("")), "d"), "");
        assert_eq!(json_text_or(Some(&json!("{ \"a\": 1 }")), "{}"), "{ \"a\": 1 }");
        assert_eq!(json_text_or(Some(&json!("plain")), "{}"), "{}");
        assert_eq!(json_text_or(Some(&json!({"a": [1]})), "{}"), r#"{"a":[1]}"#);
        assert_eq!(value_or(Some(&json!("[1,2]")), json!([])), json!([1, 2]));
        let doc = json!({"userId": null, "_id": "u1"}).as_object().cloned().unwrap();
        assert_eq!(text(pick(&doc, &["userId", "_id"])), None);
    }
}
