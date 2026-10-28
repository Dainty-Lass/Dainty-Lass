//! What the auth code shares with itself: `Date#toISOString`, Express's
//! `req.ip` under `trust proxy: loopback` (`config/proxyTrust.ts`) and the JSON body parser's rules.

use std::net::IpAddr;

use axum::extract::{ConnectInfo, Request};
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::Value;
use service_kit::serve::ConnectionInfo;

pub fn now_ms() -> i64 {
    crate::game_data::season_window::replay_now_ms().unwrap_or_else(|| Utc::now().timestamp_millis())
}

/// `new Date(ms).toISOString()`.
pub fn iso_from_ms(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn iso_now() -> String {
    iso_from_ms(now_ms())
}

/// `Date.parse` of an ISO text, in milliseconds; `None` for NaN.
pub fn parse_ms(text: &str) -> Option<i64> {
    service_kit::js::date_parse(text)
}

// ------------------------------------------------------------------ client address

fn is_loopback(address: &str) -> bool {
    let lower = address.to_ascii_lowercase();
    if lower == "::1" {
        return true;
    }
    let v4 = lower.strip_prefix("::ffff:").unwrap_or(&lower);
    let mut parts = v4.split('.');
    parts.next() == Some("127")
        && parts.clone().count() == 3
        && parts.all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// `ForwardedClientAddress` (`config/proxyTrust.ts`): a loopback peer may speak for a client, by the rightmost
/// `X-Forwarded-For` entry that is not loopback; any other peer is its own address.
pub fn forwarded_client_address(peer: &str, forwarded_for: &[&str]) -> String {
    if !is_loopback(peer) {
        return peer.to_owned();
    }
    let hops: Vec<&str> = forwarded_for
        .iter()
        .flat_map(|header| header.split(','))
        .map(str::trim)
        .filter(|hop| !hop.is_empty() && hop.len() <= 64)
        .collect();
    hops.iter()
        .rev()
        .find(|hop| !is_loopback(hop))
        .map_or_else(|| peer.to_owned(), |hop| (*hop).to_owned())
}

/// The peer's address, as the connection layer saw it.
pub fn peer_ip(request: &Request) -> Option<IpAddr> {
    request
        .extensions()
        .get::<ConnectInfo<ConnectionInfo>>()
        .map(|info| info.0.peer.ip())
}

/// `req.ip ?? req.socket.remoteAddress ?? "unknown"`.
pub fn client_ip(request: &Request) -> String {
    let Some(peer) = peer_ip(request) else {
        return "unknown".to_owned();
    };
    let forwarded: Vec<&str> = request
        .headers()
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    forwarded_client_address(&peer.to_string(), &forwarded)
}

/// `ipKeyGenerator(ip, 56)` (`express-rate-limit` 8.7.0): converts IPv4-mapped IPv6 addresses to IPv4 and groups other
/// IPv6 addresses by their `/56` network prefix.
pub fn ip_key_generator(ip: &str) -> String {
    let trimmed = ip.trim();
    if let Some(rest) = trimmed
        .strip_prefix("::ffff:")
        .or_else(|| trimmed.strip_prefix("::FFFF:"))
        && let Ok(v4) = rest.parse::<std::net::Ipv4Addr>()
    {
        return v4.to_string();
    }
    if let Ok(v6) = trimmed.parse::<std::net::Ipv6Addr>() {
        if let Some(v4) = v6.to_ipv4_mapped() {
            return v4.to_string();
        }
        if trimmed.contains('.')
            && let Some(v4) = v6.to_ipv4()
        {
            return v4.to_string();
        }
        let masked = std::net::Ipv6Addr::from(u128::from(v6) & (!0u128 << (128 - 56)));
        return format!("{masked}/56");
    }
    trimmed.to_owned()
}

// ------------------------------------------------------------------ bodies

pub use crate::http::{BODY_LIMIT, read_body};

/// `req.body?.name`: the field of an object body.
pub fn field<'a>(body: Option<&'a Value>, name: &str) -> Option<&'a Value> {
    body.and_then(|value| value.get(name))
}

/// `typeof value === "string"`.
pub fn as_str(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str)
}

/// Every value of a query name, decoded, in order. A name that repeats is an array in `qs`, which is not a string.
pub fn query_values(query: Option<&str>, name: &str) -> Vec<String> {
    query
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode_query(key) == name).then(|| decode_query(value))
        })
        .collect()
}

/// A single query value as `Option<Option<String>>`: `None` absent, `Some(None)` repeated (an array), `Some(Some(v))`.
pub fn query_param(query: Option<&str>, name: &str) -> Option<Option<String>> {
    let mut found = query_values(query, name);
    match found.len() {
        0 => None,
        1 => Some(found.pop()),
        _ => Some(None),
    }
}

/// A query component as `qs` decodes it: `+` is a space, and text that is not valid percent-encoding stays as it is.
pub fn decode_query(text: &str) -> String {
    let spaced = text.replace('+', " ");
    match percent_encoding::percent_decode_str(&spaced).decode_utf8() {
        Ok(decoded) => decoded.into_owned(),
        Err(_) => spaced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_text_is_javascripts() {
        assert_eq!(iso_from_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_from_ms(1_790_000_000_123), "2026-09-21T14:13:20.123Z");
    }

    #[test]
    fn only_a_loopback_peer_speaks_for_a_client() {
        assert_eq!(forwarded_client_address("8.8.8.8", &["1.2.3.4"]), "8.8.8.8");
        assert_eq!(forwarded_client_address("127.0.0.1", &["1.2.3.4, 5.6.7.8"]), "5.6.7.8");
        assert_eq!(
            forwarded_client_address("127.0.0.1", &["1.2.3.4, 127.0.0.2"]),
            "1.2.3.4"
        );
        assert_eq!(forwarded_client_address("::1", &[]), "::1");
        assert_eq!(forwarded_client_address("::ffff:127.0.0.1", &["9.9.9.9"]), "9.9.9.9");
    }

    #[test]
    fn query_parameters_are_strings_only_when_single() {
        assert_eq!(
            query_param(Some("name=Fox%20X"), "name"),
            Some(Some("Fox X".to_owned()))
        );
        assert_eq!(query_param(Some("a=1&name=x&name=y"), "name"), Some(None));
        assert_eq!(query_param(Some("a=1"), "name"), None);
    }

    #[test]
    fn ip_key_generator_matches_express_rate_limit_v8() {
        assert_eq!(ip_key_generator("127.0.0.1"), "127.0.0.1");
        assert_eq!(ip_key_generator("198.51.100.10"), "198.51.100.10");
        assert_eq!(ip_key_generator("unknown"), "unknown");
        assert_eq!(ip_key_generator("::1"), "::/56");
        assert_eq!(ip_key_generator("::ffff:127.0.0.1"), "127.0.0.1");
        assert_eq!(ip_key_generator("::ffff:198.51.100.10"), "198.51.100.10");
        assert_eq!(
            ip_key_generator("2001:db8:abcd:ef12:1:2:3:4"),
            "2001:db8:abcd:ef00::/56"
        );
        assert_eq!(ip_key_generator("2001:db8:abcd:ef99::1"), "2001:db8:abcd:ef00::/56");
        assert_eq!(ip_key_generator("fe80::1ff:fe23:4567:890a"), "fe80::/56");
        assert_eq!(
            ip_key_generator("2001:0db8:85a3:0000:0000:8a2e:0370:7334"),
            "2001:db8:85a3::/56"
        );
    }
}
