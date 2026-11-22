// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `runtime.json` the Director reads before every game server start (docs/security/director-injection.md): any
//! text parses or is refused without a panic, and what parses is the runtime's lowercase SHA-256 (R21).
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_director::runtime::manifest_hash;

fuzz_target!(|text: &str| {
    if let Ok(sha256) = manifest_hash(text) {
        assert!(sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    }
});
