//! What the host kit reads before it fetches and writes game data (docs/security/host-gamedata.md): the store
//! capture pin, which names the files it downloads and where they go, and the game-data file names it makes
//! example names from. None of it may panic; a pin that passes names only plain relative paths, a full commit
//! and real SHA-256 hashes.
#![no_main]

use std::path::{Component, Path};

use dainty_hostkit::captures::parse_capture_pin;
use dainty_hostkit::gamedata::example_file_name;
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
struct Input<'a> {
    pin: &'a str,
    name: &'a str,
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

fuzz_target!(|input: Input<'_>| {
    if let Ok(pin) = parse_capture_pin(Path::new("store-captures.json"), input.pin) {
        assert!(!pin.name.is_empty() && is_lower_hex(&pin.commit, 40));
        assert!(pin.repository.starts_with("https://"));
        assert!(!pin.files.is_empty());
        for (rel, hash) in &pin.files {
            assert!(is_lower_hex(hash, 64));
            // A pinned file is written under the captures folder and nowhere else
            let under = Path::new("captures").join(rel);
            assert!(
                under.components().skip(1).all(|c| matches!(c, Component::Normal(_))),
                "{rel:?}"
            );
            assert!(!rel.contains('\\') && !rel.contains(':'), "{rel:?}");
        }
    }
    assert!(example_file_name(input.name).ends_with(".example.json"));
});
