// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The Guard pipe's `RGE1` messages from the hosted game server (spec-rd27-rifts-transport.md §3.4): an accepted
//! event has a bounded type and actor and a braced payload.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rifts_transport::pipe::parse_message;

fuzz_target!(|data: &[u8]| {
    if let Some(event) = parse_message(data) {
        assert!((1..=64).contains(&event.kind.len()));
        assert!(event.actor.len() <= 96);
        assert!((1..=4096).contains(&event.payload.len()));
        assert!(event.payload.starts_with('{') && event.payload.ends_with('}'));
        assert!(!event.kind.contains('\t') && !event.actor.contains('\t'));
    }
});
