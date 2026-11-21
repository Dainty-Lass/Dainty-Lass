// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Which requests count as the game's own: loopback peers naming a loopback host, never a browser page (spec
//! §2.6.4). A web page can reach 127.0.0.1 too, so the Host, `sec-fetch-*` and Origin rules keep it out.

use std::net::{IpAddr, Ipv4Addr};

use hyper::HeaderMap;
use hyper::header::{HOST, ORIGIN};

/// Why a request was refused, in the order the rules are tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The peer is not a loopback address.
    NotLocal,
    /// The Host header is missing, unreadable or not a loopback name.
    Host,
    /// A browser's fetch metadata on a plain request.
    Browser,
    /// The Origin is unreadable or not a loopback origin.
    Origin,
}

impl Refusal {
    /// The word logged for it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotLocal => "not_local",
            Self::Host => "host",
            Self::Browser => "browser",
            Self::Origin => "origin",
        }
    }
}

/// A loopback peer, IPv4-mapped IPv6 included.
#[must_use]
pub fn is_loopback_peer(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
    }
}

/// `localhost`, `::1` or a dotted `127.x.y.z`; nothing that needs a resolver.
#[must_use]
pub fn is_loopback_name(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host == "::1"
        || host.parse::<Ipv4Addr>().is_ok_and(|v4| v4.octets()[0] == 127)
}

/// The host part of `host[:port]` or `[v6][:port]`, brackets removed; `None` when the shape is wrong.
#[must_use]
pub fn authority_host(authority: &str) -> Option<&str> {
    let port_ok = |port: &str| !port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit());
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        return match after.strip_prefix(':') {
            None if after.is_empty() => Some(host),
            Some(port) if port_ok(port) => Some(host),
            _ => None,
        };
    }
    match authority.matches(':').count() {
        0 => Some(authority),
        1 => {
            let (host, port) = authority.split_once(':')?;
            port_ok(port).then_some(host)
        }
        // An unbracketed IPv6 address carries no port
        _ => Some(authority),
    }
}

/// A Host header value naming a loopback host.
#[must_use]
pub fn host_is_local(value: &str) -> bool {
    authority_host(value).is_some_and(is_loopback_name)
}

/// An Origin (scheme and port optional, one trailing `/` allowed) whose host is loopback.
#[must_use]
pub fn origin_is_local(value: &str) -> bool {
    let rest = match value.split_once("://") {
        Some((scheme, rest)) => {
            let scheme_ok = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'));
            if !scheme_ok {
                return false;
            }
            rest
        }
        None => value,
    };
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    // A path, query or user part means this is not an origin
    if authority.is_empty() || authority.contains(['/', '?', '#', '@']) {
        return false;
    }
    host_is_local(authority)
}

/// The local-request rules, first match wins. `upgrade` requests skip the `sec-fetch-*` rule: the game's own
/// WebSocket client may send them.
pub fn check(peer: IpAddr, headers: &HeaderMap, upgrade: bool) -> Result<(), Refusal> {
    if !is_loopback_peer(peer) {
        return Err(Refusal::NotLocal);
    }
    let mut hosts = headers.get_all(HOST).iter();
    let host = match (hosts.next(), hosts.next()) {
        (Some(one), None) => one.to_str().ok(),
        _ => None,
    };
    if !host.is_some_and(host_is_local) {
        return Err(Refusal::Host);
    }
    if !upgrade && headers.keys().any(|name| name.as_str().starts_with("sec-fetch-")) {
        return Err(Refusal::Browser);
    }
    let mut origins = headers.get_all(ORIGIN).iter();
    match (origins.next(), origins.next()) {
        (None, _) => Ok(()),
        (Some(one), None) if one.to_str().is_ok_and(origin_is_local) => Ok(()),
        _ => Err(Refusal::Origin),
    }
}

/// `^/qos(?:[/?]|$)`, ignoring case.
#[must_use]
pub fn is_qos_target(target: &str) -> bool {
    let Some((head, rest)) = target.split_at_checked(4) else {
        return false;
    };
    head.eq_ignore_ascii_case("/qos") && (rest.is_empty() || rest.starts_with(['/', '?']))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use hyper::header::{HeaderName, HeaderValue};

    use super::*;

    fn headers(pairs: &[(&str, &[u8])]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_bytes(value).unwrap(),
            );
        }
        map
    }

    const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    #[test]
    fn peers() {
        assert!(is_loopback_peer(LOOPBACK));
        assert!(is_loopback_peer("127.9.8.7".parse().unwrap()));
        assert!(is_loopback_peer("::1".parse().unwrap()));
        assert!(is_loopback_peer("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!is_loopback_peer("::ffff:10.0.0.1".parse().unwrap()));
        assert!(!is_loopback_peer("192.168.1.2".parse().unwrap()));
        assert_eq!(
            check("10.0.0.1".parse().unwrap(), &headers(&[("host", b"localhost")]), false),
            Err(Refusal::NotLocal)
        );
    }

    #[test]
    fn host_names() {
        for good in [
            "localhost",
            "LocalHost:61100",
            "127.0.0.1",
            "127.1.2.3:80",
            "[::1]",
            "[::1]:61100",
            "::1",
        ] {
            assert!(host_is_local(good), "{good}");
        }
        for bad in [
            "",
            "localhost.",
            "evil.localhost",
            "127.0.0.01",
            "127.1",
            "128.0.0.1",
            "0.0.0.0",
            "[::2]",
            "[::1",
            "[::1]x",
            "localhost:",
            "localhost:abc",
            "localhost:123456",
            "example.com",
            "::2",
        ] {
            assert!(!host_is_local(bad), "{bad}");
        }
    }

    #[test]
    fn host_rule() {
        assert_eq!(check(LOOPBACK, &headers(&[]), false), Err(Refusal::Host));
        assert_eq!(
            check(LOOPBACK, &headers(&[("host", b"example.com")]), false),
            Err(Refusal::Host)
        );
        assert_eq!(
            check(LOOPBACK, &headers(&[("host", b"local\xffhost")]), false),
            Err(Refusal::Host)
        );
        assert_eq!(
            check(
                LOOPBACK,
                &headers(&[("host", b"localhost"), ("host", b"localhost")]),
                false
            ),
            Err(Refusal::Host)
        );
        assert_eq!(
            check(LOOPBACK, &headers(&[("host", b"127.0.0.1:61100")]), false),
            Ok(())
        );
    }

    #[test]
    fn browser_rule_spares_upgrades() {
        let browser = headers(&[("host", b"127.0.0.1"), ("sec-fetch-mode", b"cors")]);
        assert_eq!(check(LOOPBACK, &browser, false), Err(Refusal::Browser));
        assert_eq!(check(LOOPBACK, &browser, true), Ok(()));
    }

    #[test]
    fn origins() {
        for good in [
            "http://localhost",
            "http://127.0.0.1:61100",
            "https://[::1]:443/",
            "localhost",
            "127.0.0.1:5000/",
            "ws://LOCALHOST",
        ] {
            assert!(origin_is_local(good), "{good}");
        }
        for bad in [
            "null",
            "",
            "http://",
            "http://example.com",
            "http://localhost.evil.com",
            "http://localhost/path",
            "http://user@localhost",
            "http://localhost//",
            "1http://localhost",
            "http://localhost?x",
        ] {
            assert!(!origin_is_local(bad), "{bad}");
        }
        let with = |origin: &[u8]| headers(&[("host", b"localhost"), ("origin", origin)]);
        assert_eq!(check(LOOPBACK, &with(b"http://localhost"), false), Ok(()));
        assert_eq!(
            check(LOOPBACK, &with(b"https://example.com"), true),
            Err(Refusal::Origin)
        );
        assert_eq!(
            check(LOOPBACK, &with(b"http://local\xfehost"), false),
            Err(Refusal::Origin)
        );
    }

    #[test]
    fn rules_apply_in_order() {
        // Every rule fails; the first is reported
        let all = headers(&[("sec-fetch-site", b"cross-site"), ("origin", b"https://example.com")]);
        assert_eq!(check("8.8.8.8".parse().unwrap(), &all, false), Err(Refusal::NotLocal));
        assert_eq!(check(LOOPBACK, &all, false), Err(Refusal::Host));
    }

    #[test]
    fn qos_targets() {
        for yes in ["/qos", "/QoS", "/qos/", "/qos?x=1", "/qos/ping", "/QOS?"] {
            assert!(is_qos_target(yes), "{yes}");
        }
        for no in [
            "/",
            "/qo",
            "/qosx",
            "/api/qos",
            "qos",
            "/q\u{f6}s",
            "",
            "/qos#x",
            "/\u{f6}",
        ] {
            assert!(!is_qos_target(no), "{no}");
        }
    }
}
