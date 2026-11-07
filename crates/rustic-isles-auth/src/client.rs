// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The client address (spec §4.6): `x-forwarded-for` is believed only from a loopback peer, the gateway in front.

use std::net::{IpAddr, Ipv6Addr};

use axum::extract::{ConnectInfo, Request};
use axum::http::HeaderMap;
use dainty_service_kit::serve::ConnectionInfo;

use crate::util;

/// The peer as Node prints it (`::ffff:`-prefixed for a mapped IPv4), `unknown` without one.
pub fn peer_text(request: &Request) -> String {
    request
        .extensions()
        .get::<ConnectInfo<ConnectionInfo>>()
        .map_or_else(|| "unknown".to_owned(), |info| info.0.peer.ip().to_string())
}

/// The request's client address, as [`client_ip_from`] reads it from the peer and headers.
pub fn client_ip(request: &Request) -> String {
    client_ip_from(&peer_text(request), request.headers())
}

/// A loopback peer speaks for a client: the right-most non-loopback `x-forwarded-for` entry, else the peer.
pub fn client_ip_from(peer: &str, headers: &HeaderMap) -> String {
    if !util::is_loopback_text(peer) {
        return peer.to_owned();
    }
    let entries: Vec<String> = headers
        .get_all("x-forwarded-for")
        .iter()
        .flat_map(|value| {
            String::from_utf8_lossy(value.as_bytes())
                .split(',')
                .map(|entry| entry.trim().to_owned())
                .collect::<Vec<_>>()
        })
        .filter(|entry| !entry.is_empty() && entry.len() <= 64)
        .collect();
    entries
        .into_iter()
        .rev()
        .find(|entry| !util::is_loopback_text(entry))
        .unwrap_or_else(|| peer.to_owned())
}

/// `express-rate-limit` 8's `ipKeyGenerator(ip, 56)`: IPv4 as it is, IPv6 as its `/56` network.
pub fn rate_limit_key(ip: &str) -> String {
    match ip.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.to_string(),
        Ok(IpAddr::V6(v6)) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.to_string();
            }
            // `::a.b.c.d` (IPv4-compatible) written with a dot is the IPv4 too
            if ip.contains('.')
                && let Some(v4) = v6.to_ipv4()
            {
                return v4.to_string();
            }
            let mask = !0u128 << (128 - 56);
            format!("{}/56", Ipv6Addr::from_bits(v6.to_bits() & mask))
        }
        Err(_) => ip.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn xff(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn forwarded_addresses_count_only_from_loopback() {
        assert_eq!(client_ip_from("8.8.8.8", &xff("1.2.3.4")), "8.8.8.8");
        assert_eq!(client_ip_from("127.0.0.1", &xff("1.2.3.4, 5.6.7.8")), "5.6.7.8");
        assert_eq!(client_ip_from("127.0.0.1", &xff("1.2.3.4, 127.0.0.2")), "1.2.3.4");
        assert_eq!(client_ip_from("::1", &HeaderMap::new()), "::1");
        assert_eq!(client_ip_from("::ffff:127.0.0.1", &xff("9.9.9.9")), "9.9.9.9");
        assert_eq!(
            client_ip_from("127.0.0.1", &xff(&format!("{}, ", "9".repeat(65)))),
            "127.0.0.1"
        );
    }

    #[test]
    fn rate_limit_keys_group_ipv6_by_56() {
        for same in ["127.0.0.1", "198.51.100.10", "unknown"] {
            assert_eq!(rate_limit_key(same), same);
        }
        assert_eq!(rate_limit_key("::1"), "::/56");
        assert_eq!(rate_limit_key("::ffff:127.0.0.1"), "127.0.0.1");
        assert_eq!(rate_limit_key("2001:db8:abcd:ef12:1:2:3:4"), "2001:db8:abcd:ef00::/56");
        assert_eq!(rate_limit_key("2001:db8:abcd:ef99::1"), "2001:db8:abcd:ef00::/56");
        assert_eq!(rate_limit_key("fe80::1ff:fe23:4567:890a"), "fe80::/56");
        assert_eq!(
            rate_limit_key("2001:0db8:85a3:0000:0000:8a2e:0370:7334"),
            "2001:db8:85a3::/56"
        );
    }
}
