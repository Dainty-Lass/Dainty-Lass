// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! STUN and TURN server URLs. TURN is over UDP only: `turns:` and TCP are refused, since the relay path is the one the
//! backend's relay-only filter and the TURN client both expect.

const DEFAULT_PORT: u16 = 3478;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerAddr {
    pub host: String,
    pub port: u16,
}

fn host_port(rest: &str) -> Option<ServerAddr> {
    let (host, port) = match rest.rfind(':') {
        Some(at) => {
            let digits = &rest[at + 1..];
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let port: u16 = digits.parse().ok()?;
            if port == 0 {
                return None;
            }
            (&rest[..at], port)
        }
        None => (rest, DEFAULT_PORT),
    };
    let host_ok = (1..=253).contains(&host.len())
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    host_ok.then(|| ServerAddr {
        host: host.to_owned(),
        port,
    })
}

/// `stun:<host>[:<port>]`.
pub fn parse_stun(url: &str) -> Option<ServerAddr> {
    host_port(url.strip_prefix("stun:")?)
}

/// `turn:<host>[:<port>]`, optionally followed by exactly `?transport=udp`.
pub fn parse_turn_udp(url: &str) -> Option<ServerAddr> {
    let rest = url.strip_prefix("turn:")?;
    let rest = match rest.find('?') {
        Some(at) => {
            if &rest[at..] != "?transport=udp" {
                return None;
            }
            &rest[..at]
        }
        None => rest,
    };
    host_port(rest)
}

/// The list rule of the config files: split on `,`, trim spaces, drop empty items.
pub fn split_list(value: &str) -> Vec<&str> {
    value
        .split(',')
        .map(|item| item.trim_matches(' '))
        .filter(|item| !item.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            parse_stun("stun:stun.cloudflare.com:3478"),
            Some(ServerAddr {
                host: "stun.cloudflare.com".to_owned(),
                port: 3478
            })
        );
        assert_eq!(parse_stun("stun:example.org").map(|s| s.port), Some(3478));
        assert_eq!(
            parse_turn_udp("turn:turn.cloudflare.com:3478?transport=udp").map(|s| s.port),
            Some(3478)
        );
        assert_eq!(
            parse_turn_udp("turn:turn.cloudflare.com:443").map(|s| s.port),
            Some(443)
        );
        assert_eq!(parse_turn_udp("turn:turn.cloudflare.com:3478?transport=tcp"), None);
        assert_eq!(parse_turn_udp("turns:turn.cloudflare.com:5349"), None);
        assert_eq!(parse_stun("stun:bad host:1"), None);
        assert_eq!(parse_stun("stun:host:0"), None);
        assert_eq!(parse_stun("stun:host:70000"), None);
        assert_eq!(parse_stun("stun:[::1]:3478"), None);
        assert_eq!(split_list(" a , ,b,"), vec!["a", "b"]);
    }
}
