//! What the MIT game runtime reads from its environment and command line (spec-runtime-r1 4, 6; RX3): the backend
//! address (`RUSTIC_ISLES_BACKEND_URL` / `DAINTY_BACKEND_URL`) and a server's launch arguments, which pick the port,
//! the map URL the engine loads and the player roster. Also every URL the game hands HTTP-1 (spec-runtime-r2 4, 5):
//! the split, the redirect and the grant path. And the engine command line CL-1 serves (spec-runtime-r4 3).
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_runtime_core::config::auth::AuthMode;
use rustic_isles_runtime_core::config::backend::parse_backend_url;
use rustic_isles_runtime_core::config::launch::{Mode, parse_launch};
use rustic_isles_runtime_core::config::roster::{expected_player_count, map_url};
use rustic_isles_runtime_core::http::{
    MAX_REPLACEMENT_UNITS, Redirect, grant_path, is_redirect_host, is_under, redirect, split_url,
};
use rustic_isles_runtime_core::login::cmdline::{build_served_line, is_ip_literal};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Ok(url) = parse_backend_url(&text) {
        // Accepted only as scheme://host[:port]: lower-case host, a real port, and the API base reads back the same
        assert!(url.port != 0);
        assert_eq!(url.host, url.host.to_ascii_lowercase());
        assert!(!url.host.is_empty() && !url.host.contains(['/', '?', '#', '@', '\\']));
        assert_eq!(parse_backend_url(&url.api_base()), Ok(url.clone()));
        assert!(!url.chat_address()[6..].contains(':') || url.host.starts_with('['));
    }
    // A game URL: a split host is non-empty with no path, query or port, its tail a path; a rewrite stays under the
    // base
    let url: Vec<u16> = text.encode_utf16().collect();
    if let Some((host, tail)) = split_url(&url) {
        assert!(!host.is_empty() && !host.iter().any(|&unit| [0x2F, 0x3F, 0x23, 0x3A].contains(&unit)));
        assert_eq!(tail.first(), Some(&0x2F));
    }
    let base: Vec<u16> = "https://backend.invalid".encode_utf16().collect();
    match redirect(&base, &url) {
        Redirect::To { host, replacement } => {
            assert!(is_redirect_host(&host));
            assert!(replacement.len() < MAX_REPLACEMENT_UNITS);
            assert!(is_under(&replacement, &base));
        }
        Redirect::TooLong => assert!(url.len() > 1000),
        Redirect::Unchanged => {}
    }
    if let Some(path) = grant_path(&url) {
        assert!(String::from_utf16_lossy(&path).starts_with("/progression/"));
    }
    // Launch arguments: NUL separates argv entries; the whole input is also the command line
    let line: Vec<u16> = text.encode_utf16().collect();
    let args: Vec<Vec<u16>> = text.split('\0').map(|arg| arg.encode_utf16().collect()).collect();
    for auth in [AuthMode::GlobalGameserver, AuthMode::PlayerHost, AuthMode::Invalid] {
        if let Ok(launch) = parse_launch(&args, &line, auth) {
            assert!(launch.port != 0);
            assert_eq!(launch.mode == Mode::Full, args.len() > 8);
            assert!(launch.api_key.is_none() || auth == AuthMode::GlobalGameserver);
            let url = map_url(&launch, 4, true);
            assert!(url.starts_with(&launch.map_path));
            assert!(String::from_utf16_lossy(&url).ends_with("?MaxPlayers=4?listen"));
            assert!(expected_player_count(launch.expected_players.as_deref(), 4) <= 4);
        }
    }
    // CL-1 (R4 3): the served line keeps the engine's as a prefix, and a published MULTIHOME is an IP literal
    for server in [false, true] {
        let served = build_served_line(&line, &args, &line, server, true);
        assert!(served.line.starts_with(&line));
        if let Some(multihome) = &served.multihome {
            assert!(is_ip_literal(&multihome.encode_utf16().collect::<Vec<u16>>()));
        }
        assert!(!served.log_line().contains(['\r', '\n']));
    }
});
