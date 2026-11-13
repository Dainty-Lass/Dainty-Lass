//! The `runtime.json` the Director reads before every game server start (docs/security/director-injection.md): any
//! text parses or is refused without a panic, and what parses is one lowercase SHA-256.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_director::runtime::manifest_sha256;

fuzz_target!(|text: &str| {
    if let Ok(sha256) = manifest_sha256(text) {
        assert!(sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    }
});
