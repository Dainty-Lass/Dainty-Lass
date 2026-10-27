//! What the Reforged API area's handlers share: Express's replies and request reading, the clock, hashes, a
//! sliding-window limiter (`security/rateLimit.ts`'s `IsRateLimited`). Local to this area on purpose; whatever
//! another area ports of the same helpers is reconciled at integration.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Request};
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use serde_json::Value;
use service_kit::body::{BodyError, error_page, parse_json};
use service_kit::json::{JSON_UTF8, Json};
use service_kit::serve::ConnectionInfo;
use sha2::{Digest, Sha256};

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// `new Date(ms).toISOString()`.
pub fn iso(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rest = ms.rem_euclid(86_400_000);
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1000 % 60,
        rest % 1000
    )
}

pub fn now_iso() -> String {
    iso(now_ms())
}

/// `Date.parse`, `None` where it is NaN.
pub fn date_parse(text: &str) -> Option<i64> {
    service_kit::js::date_parse(text)
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `crypto.createHmac("sha256", key)` over the concatenated parts.
pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|b| b ^ 0x36));
    for part in parts {
        inner.update(part);
    }
    let mut outer = Sha256::new();
    outer.update(block.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().to_vec()
}

/// Constant-time equality of two byte strings (`crypto.timingSafeEqual` after a length check).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `crypto.randomBytes(n)`, from v4 UUIDs (the OS generator).
pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n + 16);
    while out.len() < n {
        out.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    out.truncate(n);
    out
}

/// `crypto.randomInt(bound)`, without modulo bias.
pub fn random_int(bound: u32) -> u32 {
    let limit = u32::MAX - (u32::MAX % bound);
    loop {
        let bytes = random_bytes(4);
        let n = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if n < limit {
            return n % bound;
        }
    }
}

/// `Buffer.from(text, "base64")`: lenient like Node's (skips what isn't base64, accepts url-safe).
pub fn base64_decode(text: &str) -> Vec<u8> {
    let cleaned: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '-' | '_'))
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    let engine = base64::engine::general_purpose::STANDARD_NO_PAD;
    // A dangling single character carries no byte
    let usable = cleaned.len() - usize::from(cleaned.len() % 4 == 1);
    engine.decode(&cleaned[..usable]).unwrap_or_default()
}

pub fn base64_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

// ---------------------------------------------------------------- replies

/// `res.status(status).json(value)`.
pub fn json(status: u16, value: &Value) -> Response {
    let mut response = Json(value).into_response();
    *response.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    response
}

/// `res.status(status).send()`: no body.
pub fn empty(status: u16) -> Response {
    StatusCode::from_u16(status)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
        .into_response()
}

/// `res.sendStatus(status)`: the status text as a plain body (none for 204 and 304).
pub fn send_status(status: u16) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    if matches!(status, 204 | 304) {
        return code.into_response();
    }
    let text = code.canonical_reason().unwrap_or("");
    let mut response = (code, text.to_owned()).into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    response
}

pub fn text(status: u16, content_type: &'static str, body: impl Into<Body>) -> Response {
    let mut response = Response::new(body.into());
    *response.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

pub fn set_header(response: &mut Response, name: &'static str, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().insert(HeaderName::from_static(name), value);
    }
}

/// `res.redirect(status, url)` for a client that doesn't ask for HTML.
pub fn redirect(status: u16, url: &str) -> Response {
    let mut response = text(
        status,
        "text/plain; charset=utf-8",
        format!("Found. Redirecting to {url}"),
    );
    set_header(&mut response, "location", url);
    set_header(&mut response, "vary", "Accept");
    response
}

pub const NO_CACHE: &str = "no-cache, no-store, must-revalidate";

// ---------------------------------------------------------------- requests

/// A request read the way Express's parsers leave it: head, query, and either the parsed JSON (`req.body`,
/// `None` for another content type), or raw bytes.
pub struct Req {
    pub method: String,
    pub path: String,
    pub headers: HeaderMap,
    pub query: Vec<(String, String)>,
    pub peer: Option<ConnectionInfo>,
    pub body: Option<Value>,
    pub raw: Option<Vec<u8>>,
}

pub enum BodyKind {
    None,
    /// `express.json({limit})`
    Json(usize),
    /// `express.raw({type: "application/octet-stream", limit})`
    Raw(usize),
}

fn content_type_is(headers: &HeaderMap, wanted: &str) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case(wanted))
}

fn declared_length(headers: &HeaderMap) -> Option<usize> {
    headers.get(CONTENT_LENGTH)?.to_str().ok()?.trim().parse().ok()
}

/// Percent-decodes a query component (`+` is a space).
pub fn decode_component(text: &str) -> String {
    let spaced = text.replace('+', " ");
    percent_encoding::percent_decode_str(&spaced)
        .decode_utf8_lossy()
        .into_owned()
}

impl Req {
    /// The request's head and its body, unread: auth middleware runs before a route's own body parser.
    pub fn head(request: Request) -> (Self, Body) {
        let (parts, body) = request.into_parts();
        let query = parts
            .uri
            .query()
            .unwrap_or_default()
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                (decode_component(name), decode_component(value))
            })
            .collect();
        let peer = parts.extensions.get::<ConnectInfo<ConnectionInfo>>().map(|info| info.0);
        let req = Self {
            method: parts.method.as_str().to_owned(),
            path: parts.uri.path().to_owned(),
            headers: parts.headers,
            query,
            peer,
            body: None,
            raw: None,
        };
        (req, body)
    }

    /// Reads `body` the way `kind`'s parser does; the `Err` is the answer Express's error page gives.
    pub async fn load(&mut self, body: Body, kind: BodyKind) -> Result<(), Response> {
        match kind {
            BodyKind::None => {}
            BodyKind::Json(limit) if content_type_is(&self.headers, "application/json") => {
                let bytes = read_limited(body, limit, declared_length(&self.headers)).await?;
                let text = std::str::from_utf8(&bytes).map_err(|_| error_page(StatusCode::BAD_REQUEST))?;
                self.body = Some(parse_json(text).map_err(|e: BodyError| error_page(e.status()))?);
            }
            BodyKind::Raw(limit) if content_type_is(&self.headers, "application/octet-stream") => {
                self.raw = Some(read_limited(body, limit, declared_length(&self.headers)).await?);
            }
            _ => {}
        }
        Ok(())
    }

    /// Reads `request`; the `Err` is the answer Express's error page gives.
    pub async fn read(request: Request, kind: BodyKind) -> Result<Self, Response> {
        let (mut req, body) = Self::head(request);
        req.load(body, kind).await?;
        Ok(req)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)?.to_str().ok()
    }

    /// The first value of a query name (`req.query.x` when it is a string).
    pub fn query_one(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(key, _)| key == name).map(|(_, v)| v.as_str())
    }

    /// `typeof req.query.x === "string"`: false when the name repeats (an array).
    pub fn query_string(&self, name: &str) -> Option<&str> {
        let mut values = self.query.iter().filter(|(key, _)| key == name);
        let first = values.next()?;
        values.next().is_none().then_some(first.1.as_str())
    }

    pub fn peer_ip(&self) -> Option<IpAddr> {
        self.peer.map(|info| info.peer.ip())
    }

    pub fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer.map(|info| info.peer)
    }

    /// The socket peer as `req.socket.remoteAddress` writes it.
    pub fn remote_address(&self) -> Option<String> {
        self.peer_ip().map(|ip| ip.to_string())
    }

    /// `req.ip` with `trust proxy = loopback` (`config/proxyTrust.ts`).
    pub fn ip(&self) -> String {
        let Some(peer) = self.remote_address() else {
            return "unknown".to_owned();
        };
        if !is_loopback_address(&peer) {
            return peer;
        }
        let forwarded = self.header("x-forwarded-for").unwrap_or_default();
        forwarded
            .split(',')
            .map(str::trim)
            .filter(|hop| !hop.is_empty() && hop.len() <= 64)
            .rev()
            .find(|hop| !is_loopback_address(hop))
            .map_or(peer, str::to_owned)
    }

    /// `Body(req)` of the Reforged API: the parsed body when it is an object (arrays count), else empty.
    pub fn fields(&self) -> serde_json::Map<String, Value> {
        match &self.body {
            Some(Value::Object(map)) => map.clone(),
            _ => serde_json::Map::new(),
        }
    }
}

async fn read_limited(body: Body, limit: usize, declared: Option<usize>) -> Result<Vec<u8>, Response> {
    if declared.is_some_and(|len| len > limit) {
        return Err(error_page(StatusCode::PAYLOAD_TOO_LARGE));
    }
    to_bytes(body, limit)
        .await
        .map(|b| b.to_vec())
        .map_err(|_| error_page(StatusCode::PAYLOAD_TOO_LARGE))
}

/// `127.x.x.x`, `::1` and `::ffff:127.x.x.x` (`config/proxyTrust.ts`).
pub fn is_loopback_address(address: &str) -> bool {
    let v4 = |text: &str| {
        let parts: Vec<&str> = text.split('.').collect();
        parts.len() == 4
            && parts[0] == "127"
            && parts[1..]
                .iter()
                .all(|p| (1..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))
    };
    address == "::1" || v4(address) || address.strip_prefix("::ffff:").is_some_and(v4)
}

/// The peer addresses `IsLoopbackAddress` (releaseGates.ts) accepts.
pub fn is_loopback_peer(address: Option<&str>) -> bool {
    matches!(address, Some("127.0.0.1" | "::1" | "::ffff:127.0.0.1"))
}

pub fn header_value(response: &Response, name: &str) -> Option<String> {
    response.headers().get(name)?.to_str().ok().map(str::to_owned)
}

pub fn json_content_type() -> HeaderValue {
    HeaderValue::from_static(JSON_UTF8)
}

pub fn cache_control(response: &mut Response, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().insert(header::CACHE_CONTROL, value);
    }
}

// ---------------------------------------------------------------- IsRateLimited

static WINDOWS: Mutex<Option<HashMap<String, Vec<i64>>>> = Mutex::new(None);

/// `IsRateLimited(key, max, windowMs)`: a sliding window of attempts, counted when not refused.
pub fn is_rate_limited(key: &str, max_attempts: usize, window_ms: i64) -> bool {
    is_rate_limited_at(key, max_attempts, window_ms, now_ms())
}

pub fn is_rate_limited_at(key: &str, max_attempts: usize, window_ms: i64, now: i64) -> bool {
    let mut guard = WINDOWS.lock().unwrap_or_else(PoisonError::into_inner);
    let windows = guard.get_or_insert_with(HashMap::new);
    let attempts = windows.entry(key.to_owned()).or_default();
    attempts.retain(|t| now - t < window_ms);
    if attempts.len() >= max_attempts {
        return true;
    }
    attempts.push(now);
    false
}

/// `Number(text)` for the settings that are whole numbers; `None` when it is not one.
pub fn parse_integer(text: &str) -> Option<i64> {
    let n = service_kit::js::number(Some(&Value::String(text.to_owned())));
    (n.fract() == 0.0 && n.is_finite()).then_some(n as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_matches_to_iso_string() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(1_792_454_400_123), "2026-10-20T00:00:00.123Z");
        assert_eq!(iso(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(date_parse(&iso(1_700_000_000_999)), Some(1_700_000_000_999));
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        let mac = hmac_sha256(b"Jefe", &[b"what do ya want ", b"for nothing?"]);
        assert_eq!(
            hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        let long = hmac_sha256(
            &[0xaa; 131],
            &[b"Test Using Larger Than Block-Size Key - Hash Key First"],
        );
        assert_eq!(
            hex(&long),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn base64_reads_as_node_does() {
        assert_eq!(base64_decode("aGk="), b"hi");
        assert_eq!(base64_decode("aGk"), b"hi");
        assert_eq!(base64_decode("a-_"), base64_decode("a+/"));
        assert!(base64_decode("").is_empty());
    }

    #[test]
    fn the_window_slides() {
        assert!(!is_rate_limited_at("util-test", 2, 1000, 10));
        assert!(!is_rate_limited_at("util-test", 2, 1000, 20));
        assert!(is_rate_limited_at("util-test", 2, 1000, 30));
        assert!(!is_rate_limited_at("util-test", 2, 1000, 1015));
    }

    #[test]
    fn the_client_address_follows_the_trust_rule() {
        assert!(is_loopback_address("127.0.0.1") && is_loopback_address("::ffff:127.0.0.9"));
        assert!(!is_loopback_address("10.0.0.1") && !is_loopback_address("::2"));
    }
}
