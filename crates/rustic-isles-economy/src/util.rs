// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! JavaScript value habits and the answers the three parts share.

use axum::response::{IntoResponse, Response};
use http::{StatusCode, header};
use serde_json::{Map, Number, Value};
use service_kit::json::JSON_UTF8;
use service_kit::{js, jsjson};
use sha2::{Digest, Sha256};

pub(crate) const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

/// A JS number as JSON: a whole number below 2^53 in magnitude is an integer, anything else the float; NaN and the
/// infinities are `null`, as `JSON.stringify` writes them.
pub fn number_value(n: f64) -> Value {
    if !n.is_finite() {
        return Value::Null;
    }
    if n.fract() == 0.0 && n.abs() <= MAX_SAFE {
        // Whole and inside 2^53, so the cast is exact; -0 becomes 0 as JSON.stringify writes it
        #[allow(clippy::cast_possible_truncation)]
        return Value::from(n as i64);
    }
    Number::from_f64(n).map_or(Value::Null, Value::Number)
}

/// `Number(text)` as JavaScript reads a string: also `0o` and `0b` prefixes, which `js::number` does not.
pub fn js_number_text(text: &str) -> f64 {
    let t = js::trim(text);
    for (prefix, radix) in [("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(digits) = t.strip_prefix(prefix) {
            if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
                return f64::NAN;
            }
            return digits.chars().fold(0.0, |acc, c| {
                acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))
            });
        }
    }
    js::number(Some(&Value::String(text.to_owned())))
}

/// `Number(x)` of a JSON value.
pub(crate) fn num(value: Option<&Value>) -> f64 {
    js::number(value)
}

/// `Number(x)`, NaN read as 0.
pub(crate) fn num0(value: Option<&Value>) -> f64 {
    let n = num(value);
    if n.is_nan() { 0.0 } else { n }
}

/// A finite whole number at most 2^53 - 1 in magnitude.
pub(crate) fn is_whole(n: f64) -> bool {
    n.is_finite() && n.fract() == 0.0 && n.abs() <= MAX_SAFE
}

/// An `i64` from a number already known to be whole and in range.
pub(crate) fn whole(n: f64) -> i64 {
    // Callers check `is_whole` (or truncate a bounded value) first
    #[allow(clippy::cast_possible_truncation)]
    let out = n as i64;
    out
}

/// `Math.trunc` into an `i64`, saturating at the ends (NaN is 0).
pub(crate) fn trunc_i64(n: f64) -> i64 {
    if n.is_nan() {
        return 0;
    }
    // `as` saturates for floats, which is the bound wanted here
    #[allow(clippy::cast_possible_truncation)]
    let out = n.trunc() as i64;
    out
}

pub(crate) fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The first `n` UTF-16 units of `text` (a split surrogate pair is dropped).
pub(crate) fn first_units(text: &str, n: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let width = c.len_utf16();
        if used + width > n {
            break;
        }
        used += width;
        out.push(c);
    }
    out
}

pub(crate) fn first_chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

pub(crate) fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    out
}

/// The first `n` hex digits of a SHA-256, upper-cased.
pub(crate) fn sha_upper(text: &str, n: usize) -> String {
    sha256_hex(text)[..n].to_ascii_uppercase()
}

pub(crate) fn iso(ms: i64) -> String {
    rustic_isles::season::to_iso_string(ms)
}

pub(crate) fn now_iso() -> String {
    iso(rustic_isles::replay::now_ms())
}

pub(crate) fn system_now_ms() -> i64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
}

/// Plain when it is 1 to 128 bytes of ASCII letters, digits and `_ . : -`, else its first 64 characters as a JSON
/// string; client text in a log line can never start a new one.
pub(crate) fn log_token(text: &str) -> String {
    let plain = (1..=128).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'));
    if plain {
        text.to_owned()
    } else {
        jsjson::stringify(&Value::String(first_chars(text, 64)))
    }
}

/// The first 128 characters with everything outside `!`..`~` as `?` (progression's log habit).
pub(crate) fn printable(text: &str, n: usize) -> String {
    text.chars()
        .take(n)
        .map(|c| if ('!'..='~').contains(&c) { c } else { '?' })
        .collect()
}

pub(crate) fn obj(pairs: Vec<(&str, Value)>) -> Value {
    let mut map = Map::new();
    for (k, v) in pairs {
        map.insert(k.to_owned(), v);
    }
    Value::Object(map)
}

pub(crate) fn text_of(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str)
}

pub(crate) fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    text_of(value).filter(|s| !s.is_empty())
}

// ------------------------------------------------------------------ answers

pub(crate) fn empty(status: StatusCode) -> Response {
    status.into_response()
}

pub(crate) fn page500() -> Response {
    service_kit::body::error_page(StatusCode::INTERNAL_SERVER_ERROR)
}

pub(crate) fn json_text(status: StatusCode, body: String) -> Response {
    (status, [(header::CONTENT_TYPE, JSON_UTF8)], body).into_response()
}

pub(crate) fn json(status: StatusCode, value: &Value) -> Response {
    json_text(status, jsjson::stringify(value))
}

pub(crate) fn ok(value: &Value) -> Response {
    json(StatusCode::OK, value)
}

/// Express's `res.json(undefined)`: 200, the JSON type, no bytes.
pub(crate) fn empty_json() -> Response {
    json_text(StatusCode::OK, String::new())
}

/// `{"code":null,"message":..,"payload":..}`
pub(crate) fn envelope(status: StatusCode, message: &str, payload: Value) -> Response {
    json(
        status,
        &obj(vec![
            ("code", Value::Null),
            ("message", Value::from(message)),
            ("payload", payload),
        ]),
    )
}

pub(crate) fn refusal(status: StatusCode, message: &str) -> Response {
    envelope(status, message, Value::Object(Map::new()))
}

pub(crate) fn coded(status: StatusCode, code: &str, message: &str) -> Response {
    json(
        status,
        &obj(vec![
            ("code", Value::from(code)),
            ("message", Value::from(message)),
            ("payload", Value::Null),
        ]),
    )
}

/// `{"error":{"code":"NOT_IMPLEMENTED","message":..,"requestId":..}}`, 501.
pub(crate) fn not_implemented(message: &str, request_id: &str) -> Response {
    let body = obj(vec![(
        "error",
        obj(vec![
            ("code", Value::from("NOT_IMPLEMENTED")),
            ("message", Value::from(message)),
            ("requestId", Value::from(request_id)),
        ]),
    )]);
    json(StatusCode::NOT_IMPLEMENTED, &body)
}

/// A rifts hook's reply, answered as it is.
pub(crate) fn reply(reply: &rifts_seam::Reply) -> Response {
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    json_text(status, reply.body.0.clone())
}

pub(crate) fn status(code: u16) -> StatusCode {
    StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_as_javascript_reads_text() {
        for (text, want) in [
            ("0x5", 5.0),
            (" 2 ", 2.0),
            ("1e1", 10.0),
            ("0o17", 15.0),
            ("0b11", 3.0),
            ("", 0.0),
        ] {
            assert_eq!(js_number_text(text), want, "{text}");
        }
        for text in ["12abc", "1_0", "inf", "NaN", "-0x10", "1 2", "0b2"] {
            assert!(js_number_text(text).is_nan(), "{text}");
        }
        assert_eq!(js_number_text(".5"), 0.5);
        assert_eq!(js_number_text("-Infinity"), f64::NEG_INFINITY);
    }

    #[test]
    fn number_values() {
        assert_eq!(number_value(5.0), Value::from(5));
        assert_eq!(number_value(-0.0), Value::from(0));
        assert_eq!(number_value(1.5), Value::from(1.5));
        assert_eq!(number_value(f64::NAN), Value::Null);
    }

    #[test]
    fn log_tokens() {
        assert_eq!(log_token("player-1"), "player-1");
        assert_eq!(log_token("a\nb"), "\"a\\nb\"");
        assert_eq!(log_token(""), "\"\"");
    }
}
