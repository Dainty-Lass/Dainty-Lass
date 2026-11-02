// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What reaches the allowlist helper, which runs as root or administrator (RX3, RD27): the `/allow` body and its
//! `Content-Length`, the state file, and the settings that reach a script. None may panic; an accepted address is
//! exactly one script element and one Windows remote address; a port text, table or interface name that passes
//! is one token; a guard attach either succeeds with the text grown or leaves it unchanged.
#![no_main]

use gateway::allowlist::firewall::{WindowsChange, plan, windows_change};
use gateway::allowlist::guard::Guard;
use gateway::allowlist::helper::{allow_body, declared_length};
use gateway::allowlist::nft::{NftSettings, check_interface, check_nft_table, nft_script};
use gateway::allowlist::ports::{check_port_list, check_ports};
use gateway::allowlist::state::AllowlistState;
use gateway::ip::{AllowlistCheck, check_allowlist_ip};
use http::{HeaderMap, HeaderValue};
use libfuzzer_sys::fuzz_target;
use module_api::PortRange;

fn one_token(text: &str) -> bool {
    !text.is_empty() && !text.contains(|c: char| c.is_whitespace() || matches!(c, '"' | ';' | '{' | '}' | '#'))
}

fuzz_target!(|input: (Vec<u8>, String, Vec<(u16, u16)>)| {
    let (body, setting, ranges) = input;
    if let Some(Some(value)) = allow_body(&body)
        && let AllowlistCheck::Allowed(ip) = check_allowlist_ip(value.as_str(), true)
    {
        let rule = plan(std::slice::from_ref(&ip.canonical), "8770-8777").expect("an accepted address plans");
        let settings = NftSettings {
            table: "t".into(),
            interface: None,
        };
        let script = nft_script(&rule, &settings).expect("a planned rule scripts");
        // 3 table lines, 2 sets of 3 lines, 8 chain lines, one elements line
        assert_eq!(script.lines().count(), 3 + 6 + 8 + 1);
        assert_eq!(script.matches(&format!(" {} ", ip.canonical)).count(), 1);
        match windows_change(&rule, "DaintyLass x") {
            WindowsChange::Open(open) => assert_eq!(open.remote_addresses, ip.canonical),
            WindowsChange::Close => unreachable!("an address opens the rule"),
        }
    }
    if let Ok(value) = HeaderValue::from_bytes(&body) {
        let mut headers = HeaderMap::new();
        headers.insert("content-length", value);
        let _ = declared_length(&headers);
    }
    if let Ok(saved) = serde_json::from_slice(&body) {
        let mut state = AllowlistState::new(600_000, 256);
        state.restore(&saved, 1, false);
        assert!(state.len() <= 256);
    }
    let checked = [
        check_ports(&setting),
        check_port_list(&setting),
        check_nft_table(&setting),
        check_interface(&setting),
    ];
    for name in checked.into_iter().flatten() {
        assert!(one_token(&name), "{name:?}");
    }
    if let (Ok(guard), Ok(module)) = (Guard::new("8770-8777"), "m".parse::<module_api::ModuleId>()) {
        let before = guard.text();
        let ranges: Vec<PortRange> = ranges.iter().map(|&(first, last)| PortRange { first, last }).collect();
        match guard.attach(&module, &ranges) {
            Ok(()) => assert!(ranges.iter().all(|r| guard.covers_port(r.first) && guard.covers_port(r.last))),
            Err(_) => assert_eq!(guard.text(), before),
        }
    }
});
