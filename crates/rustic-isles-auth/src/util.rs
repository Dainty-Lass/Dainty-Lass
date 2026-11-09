// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Text, time, randomness and HTTP helpers shared by every part of the area (spec §2).

use std::net::IpAddr;

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dainty_service_kit::body::{BodyError, ExpressBody};
use http_body_util::BodyExt as _;
use rand::RngExt as _;
use serde_json::{Map, Value};

/// The JSON body limit of the area's routes (Express's `express.json` default).
pub const BODY_LIMIT: usize = 100 * 1024;

/// `Date#toISOString`; a time chrono can't hold writes the epoch (§2.1).
pub fn iso(ms: i64) -> String {
    let at = chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default();
    at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn hex(bytes: &[u8]) -> String {
    ::hex::encode(bytes)
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn unhex(text: &str) -> Option<Vec<u8>> {
    ::hex::decode(text).ok()
}

pub fn is_hex64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    rand::rng().fill(&mut bytes[..]);
    bytes
}

pub fn random_hex<const N: usize>() -> String {
    hex(&random_bytes::<N>())
}

pub fn random_base64url<const N: usize>() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<N>())
}

/// `crypto.randomUUID()`.
pub fn random_uuid() -> String {
    uuid::Builder::from_random_bytes(random_bytes::<16>())
        .into_uuid()
        .to_string()
}

/// Six uniform digits, leading zeros kept (§7.3).
pub fn random_code() -> String {
    // Rejection keeps the digits uniform: 4 294 000 000 is the largest multiple of 10^6 under 2^32
    loop {
        let n = u32::from_le_bytes(random_bytes::<4>());
        if n < 4_294_000_000 {
            return format!("{:06}", n % 1_000_000);
        }
    }
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

pub fn sha256_hex(text: &str) -> String {
    hex(&sha256(text.as_bytes()))
}

pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// JS trim, then lower-cased (§2.1).
pub fn normalize_lower(text: &str) -> String {
    dainty_service_kit::js::trim(text).to_lowercase()
}

/// A field of `req.body` (`body?.name`): absent for an array or scalar body.
pub fn field<'a>(body: Option<&'a Value>, name: &str) -> Option<&'a Value> {
    match body {
        Some(Value::Object(map)) => map.get(name),
        _ => None,
    }
}

pub fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    match value {
        Some(Value::String(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// A header as Node reads it: `authorization` keeps its first value, any other repeat is joined with `, `.
pub fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut values = headers
        .get_all(name)
        .iter()
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    if name.eq_ignore_ascii_case("authorization") {
        return values.next();
    }
    let all: Vec<String> = values.collect();
    if all.is_empty() { None } else { Some(all.join(", ")) }
}

/// Drops the first `n` bytes of a header value (Node's `slice(7)` on a Latin-1 header).
pub fn drop_bytes(text: &str, n: usize) -> String {
    let bytes = text.as_bytes();
    if bytes.len() <= n {
        return String::new();
    }
    String::from_utf8_lossy(&bytes[n..]).into_owned()
}

/// `qs`-decoded query values: `+` is space, invalid percent-encoding is left as it is.
pub fn query_pairs(query: Option<&str>) -> Vec<(String, String)> {
    let Some(query) = query else {
        return Vec::new();
    };
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            let key = qs_decode(k);
            (!key.is_empty()).then(|| (key, qs_decode(v)))
        })
        .collect()
}

pub fn qs_decode(text: &str) -> String {
    let spaced = text.replace('+', " ");
    dainty_service_kit::js::decode_uri_component(&spaced).unwrap_or_else(|| lenient_decode(&spaced))
}

// qs decodes what it can and leaves a bad escape as written
fn lenient_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

/// What `req.query.name` is: absent, one string, or an array for a repeated name.
pub enum QueryValue {
    Absent,
    One(String),
    Many,
}

pub fn query_value(pairs: &[(String, String)], name: &str) -> QueryValue {
    let values: Vec<String> = pairs
        .iter()
        .filter(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
        .collect();
    match values.len() {
        0 => QueryValue::Absent,
        1 => values.into_iter().next().map_or(QueryValue::Absent, QueryValue::One),
        _ => QueryValue::Many,
    }
}

pub fn empty(status: StatusCode) -> Response {
    status.into_response()
}

pub fn page(status: StatusCode) -> Response {
    dainty_service_kit::body::error_page(status)
}

pub fn json(status: StatusCode, value: Value) -> Response {
    let mut response = dainty_service_kit::json::Json(value).into_response();
    *response.status_mut() = status;
    response
}

pub fn ok(value: Value) -> Response {
    json(StatusCode::OK, value)
}

pub fn object(pairs: Vec<(&str, Value)>) -> Value {
    let mut map = Map::new();
    for (k, v) in pairs {
        map.insert(k.to_owned(), v);
    }
    Value::Object(map)
}

/// `req.body` (§2.2): the pipeline's parse when it ran, else this crate's under the same rules.
pub async fn read_body(request: axum::extract::Request) -> Result<Option<Value>, Response> {
    if let Some(ExpressBody(body)) = request.extensions().get::<ExpressBody>() {
        return Ok(body.clone());
    }
    let (parts, body) = request.into_parts();
    let kind = match body_kind(&parts.headers) {
        Ok(Some(kind)) => kind,
        Ok(None) => return Ok(None),
        Err(err) => return Err(err.into_response()),
    };
    let has_body =
        parts.headers.contains_key(header::TRANSFER_ENCODING) || parts.headers.contains_key(header::CONTENT_LENGTH);
    if !has_body {
        return Ok(None);
    }
    if let Some(encoding) = parts.headers.get(header::CONTENT_ENCODING) {
        let encoding = String::from_utf8_lossy(encoding.as_bytes()).trim().to_ascii_lowercase();
        if !encoding.is_empty() && encoding != "identity" {
            return Err(BodyError::UnsupportedEncoding.into_response());
        }
    }
    let bytes = match http_body_util::Limited::new(body, BODY_LIMIT).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            let too_large = err.downcast_ref::<http_body_util::LengthLimitError>().is_some();
            let error = if too_large {
                BodyError::TooLarge
            } else {
                BodyError::Aborted
            };
            return Err(error.into_response());
        }
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Err(BodyError::Malformed.into_response());
    };
    let parsed = match kind {
        Kind::Json => {
            let text = text.trim_start_matches([' ', '\t', '\n', '\r']);
            dainty_service_kit::body::parse_json(text.strip_prefix('\u{feff}').unwrap_or(text))
        }
        Kind::Form => dainty_service_kit::body::parse_form(text),
    };
    parsed.map(Some).map_err(IntoResponse::into_response)
}

#[derive(Clone, Copy)]
enum Kind {
    Json,
    Form,
}

fn body_kind(headers: &HeaderMap) -> Result<Option<Kind>, BodyError> {
    let Some(raw) = headers.get(header::CONTENT_TYPE) else {
        return Ok(None);
    };
    let text = String::from_utf8_lossy(raw.as_bytes());
    let mut parts = text.split(';');
    let essence = parts.next().unwrap_or("").trim();
    let kind = if essence.eq_ignore_ascii_case("application/json") {
        Kind::Json
    } else if essence.eq_ignore_ascii_case("application/x-www-form-urlencoded") {
        Kind::Form
    } else {
        return Ok(None);
    };
    for param in parts {
        if let Some((name, value)) = param.split_once('=')
            && name.trim().eq_ignore_ascii_case("charset")
            && !value.trim().trim_matches('"').eq_ignore_ascii_case("utf-8")
        {
            return Err(BodyError::UnsupportedCharset);
        }
    }
    Ok(Some(kind))
}

pub use dainty_service_kit::js::encode_uri_component;

/// Whether an address is loopback as §4.6 reads it: `::1`, or dotted `127.x.x.x`, optionally `::ffff:`-prefixed.
pub fn is_loopback_text(text: &str) -> bool {
    if text == "::1" {
        return true;
    }
    let v4 = text.strip_prefix("::ffff:").unwrap_or(text);
    matches!(v4.parse::<IpAddr>(), Ok(IpAddr::V4(a)) if a.octets()[0] == 127)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_iso_text() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(1_790_000_000_123), "2026-09-21T14:13:20.123Z");
        assert_eq!(iso(i64::MAX), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn query_values_decode_as_qs() {
        let pairs = query_pairs(Some("name=Fox%20X&a=1&a=2&b=%zz+c"));
        assert!(matches!(query_value(&pairs, "name"), QueryValue::One(ref v) if v == "Fox X"));
        assert!(matches!(query_value(&pairs, "a"), QueryValue::Many));
        assert!(matches!(query_value(&pairs, "none"), QueryValue::Absent));
        assert!(matches!(query_value(&pairs, "b"), QueryValue::One(ref v) if v == "%zz c"));
    }

    #[test]
    fn uuids_and_codes_have_their_shape() {
        let id = random_uuid();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        let code = random_code();
        assert!(code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
