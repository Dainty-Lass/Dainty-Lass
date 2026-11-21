// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! A transport's ready line as the launcher reads it (spec-rd27-rifts-host.md §6.2.1 `parse_ice_ready`), with every
//! state and flag. An accepted line is a loopback ICE route whose port matches the local client flag.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_rifts_host::parse_ice_ready;

fuzz_target!(|data: &[u8]| {
    let Some((flags, rest)) = data.split_first() else {
        return;
    };
    let line = String::from_utf8_lossy(rest);
    let state = if flags & 1 == 0 { "HOST_READY" } else { "ROUTE_READY" };
    let force_relay = flags & 2 != 0;
    let local_client = flags & 4 != 0;
    if let Ok(port) = parse_ice_ready(&line, state, force_relay, local_client) {
        assert_eq!(port != 0, local_client);
        assert!(line.split_whitespace().next() == Some(state));
        assert!(line.split_whitespace().any(|t| t == "localHost=127.0.0.1"));
        assert!(line.split_whitespace().any(|t| t == "transport=ice"));
    }
});
