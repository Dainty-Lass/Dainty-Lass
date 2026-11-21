// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Single IP addresses, strictly: no range, CIDR, zone id, leading zero or white space. What the rate limits key
//! on and what the allowlist helper lets near a firewall rule.

use std::net::SocketAddr;

/// What kind of address an IP is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpClass {
    /// `0.0.0.0` or `::`.
    Unspecified,
    /// Loopback.
    Loopback,
    /// Link-local.
    LinkLocal,
    /// Multicast.
    Multicast,
    /// Reserved ranges, or IPv6 outside global unicast.
    Reserved,
    /// Private networks, CGNAT, benchmarking and unique-local.
    Private,
    /// Anything else.
    Public,
}

/// One strictly parsed IP address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedIp {
    /// 4 or 6; an IPv4-mapped IPv6 address is 4
    pub version: u8,
    /// Its canonical text.
    pub canonical: String,
    /// What kind it is.
    pub class: IpClass,
    /// 4 bytes for IPv4, 16 for IPv6
    pub bytes: Vec<u8>,
}

fn parse_v4(text: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = text.split('.');
    for slot in &mut out {
        let part = parts.next()?;
        let ok = !part.is_empty()
            && part.len() <= 3
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'));
        if !ok {
            return None;
        }
        *slot = part.parse::<u8>().ok()?;
    }
    parts.next().is_none().then_some(out)
}

fn hex_group(group: &str) -> Option<u16> {
    if group.is_empty() || group.len() > 4 || !group.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u16::from_str_radix(group, 16).ok()
}

/// Words of one side of a `::`; the last group may be a dotted IPv4 tail when `tail` allows it.
fn words(side: &str, tail: bool) -> Option<Vec<u16>> {
    if side.is_empty() {
        return Some(Vec::new());
    }
    let groups: Vec<&str> = side.split(':').collect();
    let mut out = Vec::with_capacity(groups.len() + 1);
    for (index, group) in groups.iter().enumerate() {
        if group.contains('.') {
            if !tail || index + 1 != groups.len() {
                return None;
            }
            let v4 = parse_v4(group)?;
            out.push(u16::from_be_bytes([v4[0], v4[1]]));
            out.push(u16::from_be_bytes([v4[2], v4[3]]));
        } else {
            out.push(hex_group(group)?);
        }
    }
    Some(out)
}

fn parse_v6(text: &str) -> Option<[u16; 8]> {
    if !(2..=45).contains(&text.len()) || !text.bytes().all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.') {
        return None;
    }
    let mut out = [0u16; 8];
    match text.matches("::").count() {
        0 => {
            let all = words(text, true)?;
            if all.len() != 8 {
                return None;
            }
            out.copy_from_slice(&all);
        }
        1 => {
            let (left, right) = text.split_once("::")?;
            let left = words(left, false)?;
            let right = words(right, true)?;
            if left.len() + right.len() > 7 {
                return None;
            }
            out[..left.len()].copy_from_slice(&left);
            out[8 - right.len()..].copy_from_slice(&right);
        }
        _ => return None,
    }
    Some(out)
}

/// RFC 5952: lower case, no leading zeros, the longest run of two or more zero words (the first on a tie) as `::`.
pub fn format_v6(words: &[u16; 8]) -> String {
    let (mut best, mut best_len) = (usize::MAX, 1usize);
    let mut index = 0;
    while index < 8 {
        if words[index] == 0 {
            let start = index;
            while index < 8 && words[index] == 0 {
                index += 1;
            }
            if index - start > best_len {
                best = start;
                best_len = index - start;
            }
        } else {
            index += 1;
        }
    }
    let join = |part: &[u16]| part.iter().map(|w| format!("{w:x}")).collect::<Vec<_>>().join(":");
    if best == usize::MAX {
        return join(words);
    }
    format!("{}::{}", join(&words[..best]), join(&words[best + best_len..]))
}

fn class_v4(b: [u8; 4]) -> IpClass {
    match b {
        [0, ..] => IpClass::Unspecified,
        [127, ..] => IpClass::Loopback,
        [169, 254, ..] => IpClass::LinkLocal,
        [224..=239, ..] => IpClass::Multicast,
        [240..=255, ..] | [192, 0, 0, _] => IpClass::Reserved,
        [10, ..] | [192, 168, ..] => IpClass::Private,
        [172, x, ..] if (16..=31).contains(&x) => IpClass::Private,
        [100, x, ..] if (64..=127).contains(&x) => IpClass::Private,
        [198, 18 | 19, ..] => IpClass::Private,
        _ => IpClass::Public,
    }
}

fn class_v6(w: &[u16; 8]) -> IpClass {
    if w.iter().all(|&x| x == 0) {
        IpClass::Unspecified
    } else if w[..7].iter().all(|&x| x == 0) && w[7] == 1 {
        IpClass::Loopback
    } else if w[0] & 0xff00 == 0xff00 {
        IpClass::Multicast
    } else if w[0] & 0xffc0 == 0xfe80 {
        IpClass::LinkLocal
    } else if w[0] & 0xfe00 == 0xfc00 || w[0] & 0xffc0 == 0xfec0 {
        IpClass::Private
    } else if w[0] & 0xe000 != 0x2000 {
        IpClass::Reserved
    } else {
        IpClass::Public
    }
}

fn from_v4(b: [u8; 4]) -> ParsedIp {
    ParsedIp {
        version: 4,
        canonical: format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]),
        class: class_v4(b),
        bytes: b.to_vec(),
    }
}

/// One address, strictly; `::ffff:a.b.c.d` (a dual-stack peer) is that IPv4 address.
pub fn parse_ip(text: &str) -> Option<ParsedIp> {
    if let Some(v4) = parse_v4(text) {
        return Some(from_v4(v4));
    }
    let w = parse_v6(text)?;
    if w[..5].iter().all(|&x| x == 0) && w[5] == 0xffff {
        let [a, b] = w[6].to_be_bytes();
        let [c, d] = w[7].to_be_bytes();
        return Some(from_v4([a, b, c, d]));
    }
    Some(ParsedIp {
        version: 6,
        canonical: format_v6(&w),
        class: class_v6(&w),
        bytes: w.iter().flat_map(|x| x.to_be_bytes()).collect(),
    })
}

/// The socket's remote address canonicalized, or `unknown`.
pub fn peer_address(addr: Option<SocketAddr>) -> String {
    addr.and_then(|a| parse_ip(&a.ip().to_string()))
        .map_or_else(|| "unknown".to_owned(), |p| p.canonical)
}

/// IPv4 address, or the IPv6 /64; text that does not parse is its own key.
pub fn rate_limit_key(ip: &str) -> String {
    match parse_ip(ip) {
        Some(p) if p.version == 4 => p.canonical,
        Some(p) => {
            let mut w = [0u16; 8];
            for (i, slot) in w.iter_mut().take(4).enumerate() {
                *slot = u16::from_be_bytes([p.bytes[i * 2], p.bytes[i * 2 + 1]]);
            }
            format!("{}/64", format_v6(&w))
        }
        None => ip.to_owned(),
    }
}

/// The allowlist helper's judgement of one value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowlistCheck {
    /// The address may be admitted.
    Allowed(ParsedIp),
    /// `not_a_string`, `range_not_allowed`, `not_an_ip`, `unspecified`, `multicast`, `reserved` or `private`
    Refused(&'static str),
}

const KEYWORDS: [&str; 9] = [
    "any",
    "localsubnet",
    "dns",
    "dhcp",
    "wins",
    "defaultgateway",
    "internet",
    "intranet",
    "localhost",
];

/// Whether `value` may go in the firewall rule: one public address (or private, loopback and link-local too when
/// `allow_private`), never a range or a firewall keyword.
pub fn check_allowlist_ip(value: Option<&str>, allow_private: bool) -> AllowlistCheck {
    let Some(text) = value else {
        return AllowlistCheck::Refused("not_a_string");
    };
    if text.contains(['/', ',', '-', '*']) || KEYWORDS.iter().any(|k| text.eq_ignore_ascii_case(k)) {
        return AllowlistCheck::Refused("range_not_allowed");
    }
    let Some(parsed) = parse_ip(text) else {
        return AllowlistCheck::Refused("not_an_ip");
    };
    match parsed.class {
        IpClass::Unspecified => AllowlistCheck::Refused("unspecified"),
        IpClass::Multicast => AllowlistCheck::Refused("multicast"),
        IpClass::Reserved => AllowlistCheck::Refused("reserved"),
        IpClass::Private | IpClass::Loopback | IpClass::LinkLocal if !allow_private => {
            AllowlistCheck::Refused("private")
        }
        _ => AllowlistCheck::Allowed(parsed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_ipv4() {
        assert_eq!(parse_ip("203.0.113.7").unwrap().canonical, "203.0.113.7");
        for bad in [
            "010.0.0.1",
            "1.2.3",
            "256.1.1.1",
            " 1.2.3.4",
            "0x7f.0.0.1",
            "1..2.3",
            "1.2.3.4.5",
            "",
        ] {
            assert!(parse_ip(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn strict_ipv6_and_canonical_form() {
        assert_eq!(parse_ip("2001:0DB8:0:0:0:0:0:0001").unwrap().canonical, "2001:db8::1");
        assert_eq!(parse_ip("2001:db8:0:0:1:0:0:1").unwrap().canonical, "2001:db8::1:0:0:1");
        assert_eq!(parse_ip("::").unwrap().canonical, "::");
        assert_eq!(parse_ip("::1").unwrap().class, IpClass::Loopback);
        assert_eq!(parse_ip("1:0:0:2:0:0:0:3").unwrap().canonical, "1:0:0:2::3");
        assert_eq!(parse_ip("1:2:3:4:5:6:7:0").unwrap().canonical, "1:2:3:4:5:6:7:0");
        for bad in [
            "fe80::1%eth0",
            "2001:db8::/64",
            "2001:db8:::1",
            ":1::",
            "1:2:3:4:5:6:7:8:9",
            "1:2:3:4:5:6:7:8::",
            "g::1",
            "[::1]",
            "::ffff:010.0.0.1",
        ] {
            assert!(parse_ip(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn mapped_addresses_are_ipv4() {
        let p = parse_ip("::ffff:203.0.113.9").unwrap();
        assert_eq!((p.version, p.canonical.as_str()), (4, "203.0.113.9"));
        assert_eq!(parse_ip("::ffff:cb00:7109").unwrap().canonical, "203.0.113.9");
    }

    #[test]
    fn classes() {
        let class = |t: &str| parse_ip(t).unwrap().class;
        assert_eq!(class("0.1.2.3"), IpClass::Unspecified);
        assert_eq!(class("127.0.0.5"), IpClass::Loopback);
        assert_eq!(class("169.254.1.1"), IpClass::LinkLocal);
        assert_eq!(class("224.0.0.1"), IpClass::Multicast);
        assert_eq!(class("255.255.255.255"), IpClass::Reserved);
        assert_eq!(class("192.0.0.8"), IpClass::Reserved);
        for private in [
            "10.1.1.1",
            "172.16.0.1",
            "172.31.9.9",
            "192.168.1.1",
            "100.64.0.1",
            "198.19.0.1",
        ] {
            assert_eq!(class(private), IpClass::Private, "{private}");
        }
        assert_eq!(class("172.32.0.1"), IpClass::Public);
        assert_eq!(class("ff02::1"), IpClass::Multicast);
        assert_eq!(class("fe80::1"), IpClass::LinkLocal);
        assert_eq!(class("fd00::1"), IpClass::Private);
        assert_eq!(class("fec0::1"), IpClass::Private);
        assert_eq!(class("64:ff9b::1.2.3.4"), IpClass::Reserved);
        assert_eq!(class("::1.2.3.4"), IpClass::Reserved);
        assert_eq!(class("100::1"), IpClass::Reserved);
        assert_eq!(class("2001:db8::1"), IpClass::Public);
    }

    #[test]
    fn rate_keys() {
        assert_eq!(rate_limit_key("2001:db8:1:2:aaaa::1"), "2001:db8:1:2::/64");
        assert_eq!(rate_limit_key("203.0.113.7"), "203.0.113.7");
        assert_eq!(rate_limit_key("unknown"), "unknown");
        assert_eq!(peer_address(None), "unknown");
        assert_eq!(peer_address(Some("[::ffff:127.0.0.1]:5".parse().unwrap())), "127.0.0.1");
    }

    #[test]
    fn allowlist_check() {
        let refused = |v: Option<&str>, p| match check_allowlist_ip(v, p) {
            AllowlistCheck::Refused(r) => r,
            AllowlistCheck::Allowed(_) => "allowed",
        };
        assert_eq!(refused(None, true), "not_a_string");
        for range in [
            "1.2.3.4/32",
            "1.2.3.4,5.6.7.8",
            "1.2.3.4-1.2.3.9",
            "*",
            "Any",
            "LocalSubnet",
            "localhost",
        ] {
            assert_eq!(refused(Some(range), true), "range_not_allowed", "{range}");
        }
        assert_eq!(refused(Some("nope"), true), "not_an_ip");
        assert_eq!(refused(Some("0.0.0.0"), true), "unspecified");
        assert_eq!(refused(Some("224.0.0.1"), true), "multicast");
        assert_eq!(refused(Some("240.0.0.1"), true), "reserved");
        assert_eq!(refused(Some("10.0.0.1"), false), "private");
        assert_eq!(refused(Some("127.0.0.1"), false), "private");
        assert_eq!(refused(Some("10.0.0.1"), true), "allowed");
        assert_eq!(refused(Some("203.0.113.7"), false), "allowed");
    }
}
