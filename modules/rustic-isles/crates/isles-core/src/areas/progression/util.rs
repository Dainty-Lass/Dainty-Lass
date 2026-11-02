//! What the handlers share: replies as Express writes them, the JSON body as `express.json()` leaves it, and the few
//! JavaScript semantics the Node code leans on (`Number(text)`, `Number.isInteger`).

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use service_kit::json::Json;

/// `res.status(status).json(value)`.
pub fn reply(status: u16, value: Value) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(value),
    )
        .into_response()
}

/// `res.status(status).send()`: no body.
pub fn empty(status: u16) -> Response {
    StatusCode::from_u16(status)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
        .into_response()
}

/// `{ code: null, message, payload }`.
pub fn envelope(status: u16, message: &str, payload: Value) -> Response {
    reply(status, json!({ "code": null, "message": message, "payload": payload }))
}

pub fn ok(payload: Value) -> Response {
    envelope(200, "OK", payload)
}

pub fn forbidden() -> Response {
    envelope(403, "Forbidden", json!({}))
}

pub fn bad_request() -> Response {
    envelope(400, "Bad Request", json!({}))
}

pub fn conflict() -> Response {
    envelope(409, "Conflict", json!({}))
}

/// A reply a rifts hook wrote: its status and JSON text as they are.
pub fn from_hook(reply: rifts_seam::Reply) -> Response {
    (
        StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        [(header::CONTENT_TYPE, service_kit::json::JSON_UTF8)],
        reply.body.0,
    )
        .into_response()
}

/// `req.body` as `express.json()` leaves it. Express 5 leaves it `undefined` for a request it did not parse.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Undefined,
    Json(Value),
}

impl Body {
    /// `req.body?.a?.b`: `None` for undefined and for a hop that is not an object.
    pub fn get(&self, path: &[&str]) -> Option<&Value> {
        let mut value = self.json()?;
        for key in path {
            value = value.as_object()?.get(*key)?;
        }
        Some(value)
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, Self::Undefined)
    }

    pub fn json(&self) -> Option<&Value> {
        match self {
            Self::Undefined => None,
            Self::Json(value) => Some(value),
        }
    }
}

impl From<Option<Value>> for Body {
    fn from(parsed: Option<Value>) -> Self {
        parsed.map_or(Self::Undefined, Self::Json)
    }
}

/// `JSON.stringify`.
pub fn stringify(value: &Value) -> String {
    service_kit::jsjson::stringify(value)
}

/// JavaScript's `Number(text)` for a string; `NaN` when it is not a number.
pub fn js_number(text: &str) -> f64 {
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if text.is_empty() {
        return 0.0;
    }
    let radix = |digits: &str, base: u32| -> f64 {
        if digits.is_empty() || !digits.chars().all(|c| c.is_digit(base)) {
            return f64::NAN;
        }
        digits.chars().fold(0.0, |acc, c| {
            acc * f64::from(base) + f64::from(c.to_digit(base).unwrap_or(0))
        })
    };
    let lower = text.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("0x") {
        return radix(rest, 16);
    }
    if let Some(rest) = lower.strip_prefix("0o") {
        return radix(rest, 8);
    }
    if let Some(rest) = lower.strip_prefix("0b") {
        return radix(rest, 2);
    }
    let (sign, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, text.strip_prefix('+').unwrap_or(text)),
    };
    if unsigned == "Infinity" {
        return sign * f64::INFINITY;
    }
    if !is_decimal_literal(unsigned) {
        return f64::NAN;
    }
    unsigned.parse::<f64>().map_or(f64::NAN, |value| sign * value)
}

/// `digits [. digits] [e[+-]digits]` or `. digits [...]`, as `StrDecimalLiteral` has it.
fn is_decimal_literal(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    let digits = |index: &mut usize| {
        let start = *index;
        while bytes.get(*index).is_some_and(u8::is_ascii_digit) {
            *index += 1;
        }
        *index - start
    };
    let whole = digits(&mut index);
    let mut fraction = 0;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        fraction = digits(&mut index);
    }
    if whole + fraction == 0 {
        return false;
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        if digits(&mut index) == 0 {
            return false;
        }
    }
    index == bytes.len()
}

/// `Number.isInteger(value)` and the integer.
pub fn as_integer(value: f64) -> Option<i64> {
    // 2^53: the integers a double holds exactly
    (value.is_finite() && value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_992.0).then_some(value as i64)
}

/// `String(value)` for the JSON values a body can hold, enough for log lines.
pub fn log_safe(value: &str, max: usize) -> String {
    value
        .chars()
        .take(max)
        .map(|c| if ('\u{21}'..='\u{7e}').contains(&c) { c } else { '?' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_follows_javascript() {
        for (text, want) in [
            ("12", 12.0),
            (" 12 ", 12.0),
            ("", 0.0),
            ("1e3", 1000.0),
            ("+5", 5.0),
            ("-5", -5.0),
            ("0x10", 16.0),
            (".5", 0.5),
            ("5.", 5.0),
            ("Infinity", f64::INFINITY),
        ] {
            assert_eq!(js_number(text), want, "{text:?}");
        }
        for text in [
            "12abc", "1_0", "inf", "nan", "NaN", "e5", "1e", "--1", "0x", "-0x10", "1 2",
        ] {
            assert!(js_number(text).is_nan(), "{text:?}");
        }
        assert_eq!(as_integer(js_number("125")), Some(125));
        assert_eq!(as_integer(js_number("1.5")), None);
        assert_eq!(as_integer(js_number("1e400")), None);
    }

    #[test]
    fn optional_chaining_on_a_body() {
        let body = Body::Json(json!({ "a": { "b": 1 }, "c": [1] }));
        assert_eq!(body.get(&["a", "b"]), Some(&json!(1)));
        assert_eq!(body.get(&["c", "x"]), None);
        assert_eq!(Body::Undefined.get(&["a"]), None);
    }
}
