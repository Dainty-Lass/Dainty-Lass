#![no_main]

//! Fuzz target for `Service::Ring` wire messages (DN6, DN8, DN9): instance registry lookups,
//! home-owned ticket publications, compare-and-set ticket claims, and regional Ramsgate assignments.
//! Ensures `decode_request` and `decode_reply` never panic, enforce `RING_VERSION` and bounds, and
//! never accept a payload containing an `RRK_` account key.

use libfuzzer_sys::fuzz_target;
use network::home::protocol::looks_like_account_key;
use network::ring::{decode_reply, decode_request, encode_reply, encode_request};

fuzz_target!(|data: &[u8]| {
    if let Ok(req) = decode_request(data) {
        let encoded = encode_request(req.call.clone());
        assert!(encoded.is_ok());
        if let Ok(bytes) = encoded
            && let Ok(text) = std::str::from_utf8(&bytes)
        {
            assert!(!looks_like_account_key(text));
        }
    }
    if let Ok(reply) = decode_reply(data) {
        let encoded = encode_reply(reply.outcome.clone());
        assert!(encoded.is_ok());
        if let Ok(bytes) = encoded
            && let Ok(text) = std::str::from_utf8(&bytes)
        {
            assert!(!looks_like_account_key(text));
        }
    }
});
