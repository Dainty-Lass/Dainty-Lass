//! What a MongoDB server sends during SCRAM-SHA-256 (spec-rustic-isles-db §4.6, §15): arbitrary server-first and
//! server-final payloads. Nothing panics, a server-first is refused unless it carries the client's nonce, a
//! salt and an iteration count within the cap, and a server-final is refused unless its signature matches.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_db::mongo::scram::{MAX_ITERATIONS, Scram};

/// The last `<prefix>` attribute, as the client reads them.
fn attribute<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.split(',').rev().find_map(|p| p.strip_prefix(prefix))
}

fuzz_target!(|data: &[u8]| {
    let rest = data.get(1..).unwrap_or_default();
    let (first, last) = rest.split_at(usize::from(data.first().copied().unwrap_or(0)).min(rest.len()));
    let mut scram = Scram::new("user", "pencil", "fuzz".into());
    let text = std::str::from_utf8(first).unwrap_or_default();
    let iterations = attribute(text, "i=").and_then(|i| i.parse::<u32>().ok());
    // Counts the cap allows are run in full; keep an input fast by skipping the large ones
    if iterations.is_some_and(|i| (65..=MAX_ITERATIONS).contains(&i)) {
        return;
    }
    match scram.client_final(first) {
        Ok(_) => {
            assert!(iterations.is_some_and(|i| (1..=MAX_ITERATIONS).contains(&i)));
            assert!(attribute(text, "r=").is_some_and(|n| n.starts_with("fuzz")));
            // A random server-final carries the right signature only by a SHA-256 collision
            assert!(scram.verify_server_final(last).is_err());
        }
        Err(_) => assert!(scram.verify_server_final(last).is_err()),
    }
});
