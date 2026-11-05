//! An HTTP/1.1 answer as Rustic Isles' Director client reads it (spec-rustic-isles-matchmaking §10, §16): never panics,
//! never reads past the input, and a de-chunked body is never longer than the input.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_matchmaking::read_answer;

fuzz_target!(|data: &[u8]| {
    if let Ok((_, body)) = read_answer(data) {
        assert!(body.len() <= data.len());
    }
});
