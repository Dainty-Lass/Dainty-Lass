//! What the legacy deploy server reads before it starts a 1.4.4 game process (RX3, `docs/security/deploy.md`):
//! the caller's address and proxy headers, the matchmaking body the metagame sends (whose fields become the
//! game's arguments), a worker's encoded argument list, and the occupancy lines the game writes.
#![no_main]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use arbitrary::Arbitrary;
use deploy::app::{is_direct_loopback, is_loopback_address};
use deploy::kit::decode_worker_payload;
use deploy::matchmaker::check_request;
use deploy::occupancy::parse_native_occupancy;
use deploy::remote::Region;
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

#[derive(Arbitrary, Debug)]
struct Input<'a> {
    v6: bool,
    ip: [u8; 16],
    peer_text: Option<&'a str>,
    headers: Vec<(&'a str, &'a str)>,
    body: &'a [u8],
    payload: &'a str,
    occupancy: &'a str,
}

fn starts_a_game_path(text: &str) -> bool {
    text.starts_with("/Game/")
}

fuzz_target!(|input: Input<'_>| {
    let ip = if input.v6 {
        IpAddr::V6(Ipv6Addr::from(input.ip))
    } else {
        IpAddr::V4(Ipv4Addr::new(input.ip[0], input.ip[1], input.ip[2], input.ip[3]))
    };
    let mut headers = http::HeaderMap::new();
    for (name, value) in &input.headers {
        if let (Ok(name), Ok(value)) = (
            http::HeaderName::from_bytes(name.as_bytes()),
            http::HeaderValue::from_str(value),
        ) {
            headers.append(name, value);
        }
    }
    // Only a loopback peer with no proxy's marks may start anything
    for peer in [Some(ip.to_string()), input.peer_text.map(str::to_owned)] {
        if is_direct_loopback(peer.as_deref(), &headers) {
            assert!(is_loopback_address(peer.as_deref()));
            for name in ["x-forwarded-for", "forwarded", "x-real-ip", "via", "x-dauntless-gateway", "x-reforged-gateway"] {
                assert!(!headers.contains_key(name));
            }
        }
    }
    if is_loopback_address(Some(&ip.to_string())) {
        let mapped = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
            ip => ip,
        };
        assert!(mapped.is_loopback());
    }

    // The matchmaking body, as app.rs reads it: what passes may become the game's argv
    let body: Value = serde_json::from_slice(input.body).unwrap_or(Value::Null);
    let body = if body.is_object() { body } else { serde_json::json!({}) };
    if let Some(region) = body["Region"].as_str().and_then(Region::parse) {
        assert_eq!(region.as_str(), body["Region"].as_str().unwrap_or_default());
    }
    let field = |name: &str| body.get(name).filter(|v| !v.is_null());
    if check_request(field("GameMode"), field("GameArgs"), field("HuntId"), field("ExpectedPlayers")).is_none() {
        if let Some(hunt) = field("HuntId") {
            let hunt = hunt.as_str().expect("a string hunt id");
            assert!(hunt.len() <= 128 && hunt.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'+'));
        }
        if body["GameMode"] == "ISLAND"
            && let Some(args) = field("GameArgs").and_then(Value::as_str)
            && !service_kit::js::trim(args).is_empty()
        {
            // No argument of its own: the map and the behemoth are game paths, never a `-flag`
            let parts: Vec<&str> = args.split('?').collect();
            assert!(starts_a_game_path(parts[0]));
            let behemoth = parts.get(2).and_then(|p| p.split('=').nth(1)).expect("a behemoth");
            assert!(behemoth.is_empty() || starts_a_game_path(behemoth));
            assert!(!args.contains(' ') && !args.contains('\0'));
        }
        if let Some(players) = field("ExpectedPlayers") {
            let players = players.as_array().expect("a list");
            assert!(players.len() <= 16);
            for id in players {
                // `uid:hunt,uid:hunt` in one argument: an id can't add a player or end the list
                let id = id.as_str().expect("a string id");
                assert!(!id.is_empty() && !id.contains([',', ':', ' ', '\0']));
            }
        }
    }

    if let Ok(args) = decode_worker_payload(input.payload) {
        assert!((8..=64).contains(&args.len()));
        assert!(args.iter().all(|a| !a.contains('\0') && a.encode_utf16().count() <= 4096));
    }

    if parse_native_occupancy(input.occupancy).is_some() {
        assert!(input.occupancy.contains("net driver=IpNetDriver"));
    }
});
