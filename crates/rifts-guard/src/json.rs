// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The hunt routes' body parser and the JavaScript text rules the replies, hashes and limits are measured in.

use std::fmt::Write as _;

use serde_json::Value;

/// Express's `json({ limit: "72kb" })` on the hunt routes.
pub const P2P_MAX_HTTP_BODY_BYTES: usize = 73_728;

const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Why a hunt route's body was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyError {
    /// Over the routes' body limit.
    TooLarge,
    /// Not strict JSON, or not JSON at all.
    Invalid,
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find_map(|(key, value)| (key == name).then_some(value.as_str()))
}

/// The hunt routes' strict JSON body: `None` when the request carries no JSON body at all.
pub fn parse_p2p_body(headers: &[(String, String)], body: &[u8]) -> Result<Option<Value>, BodyError> {
    let media = header(headers, "content-type").map(|value| value.split(';').next().unwrap_or_default().trim());
    if !media.is_some_and(|media| media.eq_ignore_ascii_case("application/json")) {
        return Ok(None);
    }
    // Express reads a body only when the request says it has one.
    let length_given = header(headers, "content-length").is_some_and(|value| value.trim().parse::<u64>().is_ok());
    let has_body = !body.is_empty() || length_given || header(headers, "transfer-encoding").is_some();
    match body.len() {
        _ if !has_body => Ok(None),
        n if n > P2P_MAX_HTTP_BODY_BYTES => Err(BodyError::TooLarge),
        0 => Ok(Some(Value::Object(serde_json::Map::new()))),
        _ => {
            let text = String::from_utf8_lossy(body);
            // Strict: only an object or an array is a body.
            let opening = text.trim_start_matches([' ', '\t', '\n', '\r']).bytes().next();
            if !matches!(opening, Some(b'{' | b'[')) {
                return Err(BodyError::Invalid);
            }
            serde_json::from_str(&text).map(Some).map_err(|_| BodyError::Invalid)
        }
    }
}

/// The admin routes take any body leniently: nothing usable is no body.
pub fn parse_admin_body(bytes: &[u8]) -> Option<Value> {
    if bytes.is_empty() {
        return None;
    }
    serde_json::from_slice(bytes).ok()
}

/// `JSON.stringify` of a string.
pub fn js_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    push_js_string(&mut out, text);
    out
}

pub(crate) fn push_js_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        let short = match c {
            '"' | '\\' => Some(c),
            '\u{8}' => Some('b'),
            '\u{c}' => Some('f'),
            '\n' => Some('n'),
            '\r' => Some('r'),
            '\t' => Some('t'),
            _ => None,
        };
        if let Some(letter) = short {
            out.extend(['\\', letter]);
        } else if c < ' ' {
            let _ = write!(out, "\\u{:04x}", u32::from(c));
        } else {
            out.push(c);
        }
    }
    out.push('"');
}

/// ECMAScript `Number::toString`, with `null` for what JSON cannot hold.
pub fn js_number(value: f64) -> String {
    if !value.is_finite() {
        return "null".into();
    }
    if value == 0.0 {
        return "0".into();
    }
    // `{:e}` gives the shortest round-trip digits; only the layout differs from JavaScript's.
    let shortest = format!("{:e}", value.abs());
    let (mantissa, power) = shortest.split_once('e').unwrap_or((&shortest, "0"));
    let digits = mantissa.replace('.', "");
    let count = digits.len() as i64;
    // The decimal point sits after `point` digits.
    let point = power.parse::<i64>().unwrap_or(0) + 1;
    let sign = if value < 0.0 { "-" } else { "" };
    let zeros = |n: i64| "0".repeat(n.max(0) as usize);
    let body = if (count..=21).contains(&point) {
        format!("{digits}{}", zeros(point - count))
    } else if (1..=21).contains(&point) {
        let (whole, fraction) = digits.split_at(point as usize);
        format!("{whole}.{fraction}")
    } else if (-5..=0).contains(&point) {
        format!("0.{}{digits}", zeros(-point))
    } else {
        let (first, rest) = digits.split_at(1);
        let fraction = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let exponent = point - 1;
        format!(
            "{first}{fraction}e{}{}",
            if exponent < 0 { '-' } else { '+' },
            exponent.abs()
        )
    };
    [sign, &body].concat()
}

/// `JSON.stringify` of a parsed value: keys in document order, every number through its `f64`.
pub fn stringify(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

fn write_value(out: &mut String, value: &Value) {
    // Items or members between brackets, comma-separated.
    fn list<T>(
        out: &mut String,
        open: char,
        close: char,
        items: impl Iterator<Item = T>,
        mut each: impl FnMut(&mut String, T),
    ) {
        out.push(open);
        for (at, item) in items.enumerate() {
            if at != 0 {
                out.push(',');
            }
            each(out, item);
        }
        out.push(close);
    }
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&js_number(number.as_f64().unwrap_or(f64::NAN))),
        Value::String(text) => push_js_string(out, text),
        Value::Array(items) => list(out, '[', ']', items.iter(), write_value),
        Value::Object(members) => list(out, '{', '}', members.iter(), |out, (key, item)| {
            push_js_string(out, key);
            out.push(':');
            write_value(out, item);
        }),
    }
}

/// The byte length of `JSON.stringify(body)`; no body counts as `null`.
pub fn stringified_len(value: Option<&Value>) -> usize {
    value.map_or(4, |v| stringify(v).len())
}

/// `Number.isSafeInteger` of a JSON value.
pub fn safe_integer(value: &Value) -> Option<i64> {
    value.as_f64().and_then(js_safe_integer)
}

/// `Number.isSafeInteger` of a number.
pub fn js_safe_integer(value: f64) -> Option<i64> {
    (value.is_finite() && value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER).then_some(value as i64)
}

/// A non-negative safe integer as `u64`.
pub(crate) fn non_negative(value: &Value) -> Option<u64> {
    safe_integer(value).and_then(|n| u64::try_from(n).ok())
}

/// The length in UTF-16 units.
pub fn js_length(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// At most the first `units` UTF-16 units, never splitting a character.
pub fn js_slice(text: &str, units: usize) -> &str {
    let mut budget = units;
    let cut = text
        .char_indices()
        .find(|(_, c)| match budget.checked_sub(c.len_utf16()) {
            Some(left) => {
                budget = left;
                false
            }
            None => true,
        });
    cut.map_or(text, |(at, _)| &text[..at])
}

// ECMAScript's WhiteSpace and LineTerminator: Rust's `is_whitespace` would also take U+0085.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// `String.prototype.trim`.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

/// JavaScript's `Number(value)` of a JSON value.
pub fn js_to_number(value: &Value) -> f64 {
    match value {
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => string_to_number(s),
        Value::Array(_) => string_to_number(&js_to_text(value)),
        Value::Object(_) => f64::NAN,
    }
}

/// `String(value)` for what `Number` of an array goes through.
fn js_to_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => js_number(number.as_f64().unwrap_or(f64::NAN)),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => "null".into(),
        Value::Object(_) => "[object Object]".into(),
        // Array.prototype.join writes null elements as empty.
        Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|item| {
                    if item.is_null() {
                        String::new()
                    } else {
                        js_to_text(item)
                    }
                })
                .collect();
            parts.join(",")
        }
    }
}

fn string_to_number(text: &str) -> f64 {
    const RADIX_PREFIXES: [(&str, u32); 6] = [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)];
    let text = js_trim(text);
    if text.is_empty() {
        return 0.0;
    }
    if let Some((digits, radix)) = RADIX_PREFIXES
        .iter()
        .find_map(|(p, r)| text.strip_prefix(p).map(|d| (d, *r)))
    {
        let values: Option<Vec<u32>> = digits.chars().map(|c| c.to_digit(radix)).collect();
        return match values {
            Some(values) if !values.is_empty() => values
                .into_iter()
                .fold(0.0, |acc, d| acc * f64::from(radix) + f64::from(d)),
            _ => f64::NAN,
        };
    }
    let (sign, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, text.strip_prefix('+').unwrap_or(text)),
    };
    let magnitude = match unsigned {
        "Infinity" => f64::INFINITY,
        literal if decimal_literal(literal) => literal.parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    };
    sign * magnitude
}

/// `StrUnsignedDecimalLiteral`: digits with at most one point and an optional exponent.
fn decimal_literal(text: &str) -> bool {
    let (mantissa, exponent) = match text.find(['e', 'E']) {
        Some(at) => (&text[..at], Some(&text[at + 1..])),
        None => (text, None),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || !digits(fraction) || whole.len() + fraction.len() == 0 {
        return false;
    }
    match exponent {
        None => true,
        Some(exp) => {
            let exp = exp.strip_prefix(['+', '-']).unwrap_or(exp);
            !exp.is_empty() && digits(exp)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    fn json_type() -> Vec<(String, String)> {
        headers(&[("content-type", "application/json; charset=utf-8")])
    }

    #[test]
    fn body_parser_takes_objects_and_arrays_only() {
        assert_eq!(parse_p2p_body(&json_type(), b" \n{\"a\":1}"), Ok(Some(json!({"a": 1}))));
        assert_eq!(parse_p2p_body(&json_type(), b"[1]"), Ok(Some(json!([1]))));
        for bad in [&b"1"[..], b"\"x\"", b"null", b"{\"a\":", b"{} x", b"\xef\xbb\xbf{}"] {
            assert_eq!(parse_p2p_body(&json_type(), bad), Err(BodyError::Invalid), "{bad:?}");
        }
    }

    #[test]
    fn body_parser_bounds_and_absence() {
        let big = format!("[{}]", "1,".repeat(40_000));
        assert_eq!(parse_p2p_body(&json_type(), big.as_bytes()), Err(BodyError::TooLarge));
        let exact = format!("[\"{}\"]", "a".repeat(P2P_MAX_HTTP_BODY_BYTES - 4));
        assert!(parse_p2p_body(&json_type(), exact.as_bytes()).unwrap().is_some());
        let over = format!("[\"{}\"]", "a".repeat(P2P_MAX_HTTP_BODY_BYTES - 3));
        assert_eq!(parse_p2p_body(&json_type(), over.as_bytes()), Err(BodyError::TooLarge));
        assert_eq!(parse_p2p_body(&json_type(), b""), Ok(None));
        let declared = headers(&[("content-type", "application/json"), ("content-length", " 0 ")]);
        assert_eq!(parse_p2p_body(&declared, b""), Ok(Some(json!({}))));
        let chunked = headers(&[("content-type", "Application/JSON"), ("transfer-encoding", "chunked")]);
        assert_eq!(parse_p2p_body(&chunked, b""), Ok(Some(json!({}))));
        assert_eq!(
            parse_p2p_body(&headers(&[("content-type", "text/plain")]), b"{]"),
            Ok(None)
        );
        assert_eq!(parse_p2p_body(&[], b"{]"), Ok(None));
        assert_eq!(
            parse_p2p_body(&headers(&[("Content-Type", "application/json")]), b"{]"),
            Ok(None)
        );
    }

    #[test]
    fn duplicate_keys_keep_first_place_and_last_value() {
        let value = parse_p2p_body(&json_type(), br#"{"b":1,"a":2,"b":3}"#)
            .unwrap()
            .unwrap();
        assert_eq!(stringify(&value), r#"{"b":3,"a":2}"#);
    }

    #[test]
    fn admin_body_is_lenient() {
        assert_eq!(parse_admin_body(b""), None);
        assert_eq!(parse_admin_body(b"{nope"), None);
        assert_eq!(parse_admin_body(b"7"), Some(json!(7)));
    }

    #[test]
    fn numbers_as_javascript_writes_them() {
        let cases: [(f64, &str); 14] = [
            (1e21, "1e+21"),
            (1e-7, "1e-7"),
            (123_456_789_012_345_680_000.0, "123456789012345680000"),
            (9_007_199_254_740_993.0, "9007199254740992"),
            (-0.0, "0"),
            (0.000_001, "0.000001"),
            (1.5, "1.5"),
            (-2.25e-10, "-2.25e-10"),
            (1.234_5e30, "1.2345e+30"),
            (100.0, "100"),
            (0.1, "0.1"),
            (f64::NAN, "null"),
            (f64::INFINITY, "null"),
            (5e-324, "5e-324"),
        ];
        for (value, text) in cases {
            assert_eq!(js_number(value), text, "{value:e}");
        }
        let parsed: Value = serde_json::from_str("[1e21, 9007199254740993, 1.0, -0]").unwrap();
        assert_eq!(stringify(&parsed), "[1e+21,9007199254740992,1,0]");
    }

    #[test]
    fn strings_as_json_stringify_writes_them() {
        assert_eq!(
            js_string("a\"\\\u{8}\u{c}\n\r\t\u{1}\u{1f}\u{7f}\u{2028}é"),
            "\"a\\\"\\\\\\b\\f\\n\\r\\t\\u0001\\u001f\u{7f}\u{2028}é\""
        );
        assert_eq!(stringified_len(None), 4);
        assert_eq!(
            stringified_len(Some(&json!({"é": [true, null]}))),
            "{\"é\":[true,null]}".len()
        );
    }

    #[test]
    fn utf16_measures() {
        assert_eq!(js_length("a😀"), 3);
        assert_eq!(js_slice("a😀b", 2), "a");
        assert_eq!(js_slice("a😀b", 3), "a😀");
        assert_eq!(js_slice("ab", 9), "ab");
        assert_eq!(js_trim("\u{feff}\u{a0} x \n\u{2028}"), "x");
    }

    #[test]
    fn number_conversions() {
        let cases: [(Value, f64); 10] = [
            (json!(" 0x10 "), 16.0),
            (json!(""), 0.0),
            (json!("4"), 4.0),
            (json!(null), 0.0),
            (json!(true), 1.0),
            (json!([3]), 3.0),
            (json!([]), 0.0),
            (json!("-1e3"), -1000.0),
            (json!(".5"), 0.5),
            (json!("-Infinity"), f64::NEG_INFINITY),
        ];
        for (value, number) in cases {
            assert_eq!(js_to_number(&value), number, "{value}");
        }
        for nan in [
            json!("1x"),
            json!({}),
            json!([1, 2]),
            json!("inf"),
            json!("0x"),
            json!("1_0"),
            json!("."),
        ] {
            assert!(js_to_number(&nan).is_nan(), "{nan}");
        }
        assert_eq!(js_safe_integer(js_to_number(&json!("1.5"))), None);
        assert_eq!(js_safe_integer(9_007_199_254_740_991.0), Some(9_007_199_254_740_991));
        assert_eq!(js_safe_integer(9_007_199_254_740_992.0), None);
        assert_eq!(safe_integer(&json!("1")), None);
        assert_eq!(safe_integer(&json!(-3)), Some(-3));
        assert_eq!(non_negative(&json!(-3)), None);
    }
}
