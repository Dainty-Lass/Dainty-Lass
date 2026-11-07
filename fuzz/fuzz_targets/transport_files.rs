// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The files and signals a transport reads (spec-rd27-rifts-transport.md §2.3, §2.9): the key=value parser, the ICE
//! material and the relay-only filter. Accepted values keep their limits, and the filter passes on no candidate that
//! is not a relay.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_rifts_transport::files::{Fields, ice_material};
use rustic_rifts_transport::filter::{CANDIDATE, DESCRIPTION, GATHERING_DONE, relay_only};

const KINDS: [&str; 3] = [DESCRIPTION, CANDIDATE, GATHERING_DONE];

fuzz_target!(|data: &[u8]| {
    let Some((first, rest)) = data.split_first() else {
        return;
    };
    let text = String::from_utf8_lossy(rest);
    if let Ok(fields) = Fields::parse(&text)
        && let Ok(material) = ice_material(&fields, &fields)
    {
        assert!(material.relays.len() <= 4);
        for relay in &material.relays {
            assert!((1..=512).contains(&relay.username.len()));
            assert!(relay.server.port != 0);
        }
        for (i, peer) in material.peers.iter().enumerate() {
            assert!(peer.peer_id != 0);
            assert!(
                material.peers[..i]
                    .iter()
                    .all(|p| p.account != peer.account && p.peer_id != peer.peer_id)
            );
        }
    }
    let kind = KINDS[usize::from(*first) % KINDS.len()];
    let out = relay_only(kind, &text);
    if kind != GATHERING_DONE {
        let candidates = out
            .split("\r\n")
            .filter(|l| l.starts_with("a=candidate:") || l.starts_with("candidate:"));
        for line in candidates {
            assert!(line.contains(" typ relay"), "a non-relay candidate: {line:?}");
        }
    }
});
