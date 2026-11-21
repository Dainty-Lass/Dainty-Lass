// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The answers, request readers and text rules both halves share (public spec §2, admin spec §2).

use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use dainty_service_kit::json::JSON_UTF8;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use http_body_util::BodyExt as _;
use rand::RngExt as _;
use serde_json::{Map, Value};
use subtle::ConstantTimeEq as _;

/// 2^53 - 1, JavaScript's largest safe integer.
pub const MAX_SAFE: f64 = 9_007_199_254_740_991.0;
/// A minute in ms.
pub const MINUTE_MS: i64 = 60_000;
/// An hour in ms.
pub const HOUR_MS: i64 = 60 * MINUTE_MS;
/// A day in ms.
pub const DAY_MS: i64 = 24 * HOUR_MS;

// ------------------------------------------------------------------ answers

/// `JSON.stringify`'s text with the JSON content type.
pub fn json(status: StatusCode, value: &Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, JSON_UTF8)],
        dainty_service_kit::jsjson::stringify(value),
    )
        .into_response()
}

/// A 200 with the JSON text.
pub fn ok(value: &Value) -> Response {
    json(StatusCode::OK, value)
}

/// A status, no body, no content type.
pub fn empty(status: StatusCode) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    response
}

/// Express's `res.sendStatus(n)`.
pub fn status_text(status: StatusCode) -> Response {
    if matches!(status.as_u16(), 204 | 304) {
        return empty(status);
    }
    let reason = status.canonical_reason().unwrap_or("Unknown");
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        reason.to_owned(),
    )
        .into_response()
}

/// Express's HTML error page for the status.
pub fn error_page(status: StatusCode) -> Response {
    dainty_service_kit::body::error_page(status)
}

/// `{"error":{"code","message","requestId"}}` with the status auth's table gives the code.
pub fn launcher_error(code: &str, message: &str) -> Response {
    rustic_isles_auth::LauncherError::new(code, message).response(&random_uuid())
}

/// The admin half's `{"error":{"code","message"}}`.
pub fn admin_error(status: StatusCode, code: &str, message: &str) -> Response {
    json(status, &error_object(code, message))
}

/// `{"error":{"code","message"}}`.
pub fn error_object(code: &str, message: &str) -> Value {
    object(vec![(
        "error",
        object(vec![("code", Value::from(code)), ("message", Value::from(message))]),
    )])
}

/// An object in the order given.
pub fn object(pairs: Vec<(&str, Value)>) -> Value {
    let mut map = Map::new();
    for (k, v) in pairs {
        map.insert(k.to_owned(), v);
    }
    Value::Object(map)
}

/// Sets a header; a value that is not a valid header is skipped.
pub fn set_header(response: &mut Response, name: &'static str, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().insert(name, value);
    }
}

/// [`set_header`], returning the response.
pub fn with_header(mut response: Response, name: &'static str, value: &str) -> Response {
    set_header(&mut response, name, value);
    response
}

/// A JS number as JSON (a whole number within 2^53 an integer).
pub fn number(n: f64) -> Value {
    rustic_isles_economy::number_value(n)
}

// ------------------------------------------------------------------ time and ids

/// Core's replay-aware clock.
pub fn now_ms() -> i64 {
    rustic_isles::replay::now_ms()
}

/// The system clock, for the admin half's trust decisions.
pub fn system_now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Unix ms as `Date.toISOString` writes it.
pub use rustic_isles_auth::iso;

/// Random bytes from the thread-local generator.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    rand::rng().fill(&mut bytes[..]);
    bytes
}

/// Random bytes as lower-case hex.
pub fn random_hex<const N: usize>() -> String {
    hex::encode(random_bytes::<N>())
}

/// Random bytes as unpadded base64url.
pub fn random_base64url<const N: usize>() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<N>())
}

/// A UUID v4 from the system's randomness, never the replay seed.
pub fn random_uuid() -> String {
    rustic_isles::replay::random_uuid()
}

// ------------------------------------------------------------------ bytes

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// `Buffer.from(text, "hex")`: pairs decoded until the first that is not hex.
pub fn hex_lenient(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for pair in text.as_bytes().as_chunks::<2>().0 {
        match (hex_digit(pair[0]), hex_digit(pair[1])) {
            (Some(a), Some(b)) => out.push((a << 4) | b),
            _ => break,
        }
    }
    out
}

/// Whether the text is exactly `len` hex digits.
pub fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The SHA-256 digest of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

/// SHA-256 as lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(sha256(bytes))
}

/// HMAC-SHA256 under `key`.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
    let key = aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256, key);
    aws_lc_rs::hmac::sign(&key, message).as_ref().to_vec()
}

/// Constant time for equal lengths; unequal lengths never match.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

/// `Buffer.from(text, "base64")`: characters outside the alphabet dropped, `-` and `_` read as `+` and `/`, a
/// dangling last character dropped; text that still does not decode gives no bytes.
pub fn lenient_base64(text: &str) -> Vec<u8> {
    let mut clean: String = text
        .chars()
        .filter_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' => Some(c),
            '-' => Some('+'),
            '_' => Some('/'),
            _ => None,
        })
        .collect();
    if clean.len() % 4 == 1 {
        clean.pop();
    }
    STANDARD_NO_PAD.decode(clean.as_bytes()).unwrap_or_default()
}

/// Standard base64 with padding.
pub fn standard_base64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

// ------------------------------------------------------------------ text

/// Length in UTF-16 units, as JavaScript counts it.
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The first `n` Unicode scalars.
pub fn take_chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// The first `n` UTF-16 units (a surrogate pair is never split).
pub fn take_utf16(text: &str, n: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        used += c.len_utf16();
        if used > n {
            break;
        }
        out.push(c);
    }
    out
}

/// U+0000 to U+001F and U+007F to U+009F.
pub fn is_control(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}')
}

/// The text with control characters replaced by spaces.
pub fn controls_to_spaces(text: &str) -> String {
    text.chars().map(|c| if is_control(c) { ' ' } else { c }).collect()
}

/// JavaScript's `String.prototype.trim`.
pub fn trim(text: &str) -> &str {
    dainty_service_kit::js::trim(text)
}

/// A non-empty text of ASCII letters, digits, `.`, `_`, `-`, and neither `.` nor `..`.
pub fn is_segment(text: &str) -> bool {
    !text.is_empty()
        && text != "."
        && text != ".."
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `path.basename`: trailing `/` and `\` trimmed, then the text after the last of either.
pub fn basename(text: &str) -> &str {
    let trimmed = text.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(at) => &trimmed[at + 1..],
        None => trimmed,
    }
}

pub use dainty_service_kit::js::{encode_uri_component, search_params_encode as form_encode};

/// A field of the body when it is an object.
pub fn field<'a>(body: Option<&'a Value>, name: &str) -> Option<&'a Value> {
    match body {
        Some(Value::Object(map)) => map.get(name),
        _ => None,
    }
}

/// A string field of the body when it is an object.
pub fn str_field<'a>(body: Option<&'a Value>, name: &str) -> Option<&'a str> {
    field(body, name).and_then(Value::as_str)
}

// ------------------------------------------------------------------ requests

/// A header's first value as text.
pub fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?;
    value.to_str().ok().map(str::to_owned)
}

/// A header's first value, lossily.
pub fn header_lossy(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
}

/// The query as Express's `qs` reads it: pairs split on `&`, each at its first `=`, `+` as space, percent-decoded
/// leniently.
pub fn query_pairs(query: Option<&str>) -> Vec<(String, String)> {
    rustic_isles_auth::query_pairs(query)
}

/// The value of a name given exactly once.
pub fn single<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    let mut found = pairs.iter().filter(|(k, _)| k == name);
    let first = found.next()?;
    if found.next().is_some() {
        return None;
    }
    Some(first.1.as_str())
}

/// Whether a name is given more than once.
pub fn repeated(pairs: &[(String, String)], name: &str) -> bool {
    pairs.iter().filter(|(k, _)| k == name).count() > 1
}

/// Every value of a name joined with `,` (`String(array)`).
pub fn joined(pairs: &[(String, String)], name: &str) -> Option<String> {
    let values: Vec<&str> = pairs
        .iter()
        .filter(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .collect();
    (!values.is_empty()).then(|| values.join(","))
}

/// The client address (auth §4.6).
pub fn client_ip(request: &Request) -> String {
    rustic_isles_auth::client_ip(request)
}

/// The socket peer's IP as text, `unknown` without one.
pub fn peer_text(request: &Request) -> String {
    peer_ip(request).map_or_else(|| "unknown".to_owned(), |ip| ip.to_string())
}

/// The socket peer's IP.
pub fn peer_ip(request: &Request) -> Option<std::net::IpAddr> {
    request
        .extensions()
        .get::<axum::extract::ConnectInfo<dainty_service_kit::serve::ConnectionInfo>>()
        .map(|info| info.0.peer.ip())
}

/// The parsed JSON (or form) body under core's rules; the error page on a failure.
pub async fn read_json(request: &mut Request) -> Result<Option<Value>, Response> {
    rustic_isles::read_body(request).await
}

/// `application/octet-stream` (essence, any case) up to `limit`: over it is the 413 page; any other type no body.
pub async fn read_octets(request: &mut Request, limit: usize) -> Result<Option<Bytes>, Response> {
    let essence = header_lossy(request.headers(), "content-type")
        .map(|t| t.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
        .unwrap_or_default();
    if essence != "application/octet-stream" {
        return Ok(None);
    }
    let declared = header_lossy(request.headers(), "content-length").and_then(|v| v.trim().parse::<u64>().ok());
    if declared.is_some_and(|n| n > limit as u64) {
        return Err(error_page(StatusCode::PAYLOAD_TOO_LARGE));
    }
    // Only the body is taken: the head, and the peer in its extensions, stay for the later checks
    let body = std::mem::take(request.body_mut());
    match http_body_util::Limited::new(body, limit).collect().await {
        Ok(collected) => Ok(Some(collected.to_bytes())),
        Err(e) if e.downcast_ref::<http_body_util::LengthLimitError>().is_some() => {
            Err(error_page(StatusCode::PAYLOAD_TOO_LARGE))
        }
        Err(_) => Err(error_page(StatusCode::BAD_REQUEST)),
    }
}

/// The module's store, when one is composed.
pub fn store_of(shared: &rustic_isles::Shared) -> Option<std::sync::Arc<rustic_isles_store::IslesStore>> {
    shared.slot::<rustic_isles_store::IslesStore>()
}

/// Core's 501 envelope for the call's row.
pub fn no_store(call_route: &rustic_isles::RouteDef, request_id: &str) -> Response {
    rustic_isles::not_implemented(call_route, request_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_rules() {
        assert!(is_segment("a.b_c-1") && !is_segment("..") && !is_segment("a/b") && !is_segment(""));
        assert_eq!(basename("a/b\\c//"), "c");
        assert_eq!(basename("x"), "x");
        assert_eq!(encode_uri_component("a b/é"), "a%20b%2F%C3%A9");
        assert_eq!(lenient_base64("e30="), b"{}");
        assert_eq!(lenient_base64("AAAA"), vec![0, 0, 0]);
        assert_eq!(lenient_base64("-_8"), vec![0xfb, 0xff]);
        assert_eq!(lenient_base64("A"), Vec::<u8>::new());
        assert_eq!(hex_lenient("0aZZ0b"), vec![10]);
        assert_eq!(take_utf16("a😀b", 2), "a");
        assert_eq!(controls_to_spaces("a\u{0}b\u{85}"), "a b ");
        let pairs = query_pairs(Some("a=1&a=2&b=x+y"));
        assert_eq!(single(&pairs, "a"), None);
        assert_eq!(joined(&pairs, "a").as_deref(), Some("1,2"));
        assert_eq!(single(&pairs, "b"), Some("x y"));
    }
}
