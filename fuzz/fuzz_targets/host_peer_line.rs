// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Peer event and diagnostic lines from the transport applied to the launcher's live view (spec-rd27-rifts-host.md
//! §6.2.1 `apply_peer_line`). Each line is applied once; only an admission adds a peer.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_rifts_host::{PeerEvent, SessionLive, apply_peer_line, parse_peer_event};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut live = SessionLive::default();
    for line in text.lines() {
        let before = live.peers.len();
        apply_peer_line(&mut live, line, "now");
        // The parser splits on white space, so a line led by a space admits too
        let admitted = matches!(parse_peer_event(line), Some(PeerEvent::Admitted { .. }));
        assert!(live.peers.len() <= before + usize::from(admitted));
    }
});
