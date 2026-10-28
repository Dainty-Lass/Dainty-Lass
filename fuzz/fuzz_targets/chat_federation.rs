#![no_main]

//! Fuzzes the chat federation (`Service::Chat`, DN10b) wire codec and envelope validator
//! (`chat::federation::{decode_request, encode_request, decode_response, encode_response, validate_envelope, Envelope}`):
//! - Neither decoder nor validator may panic on any byte slice, regardless of length or UTF-8 validity.
//! - Every decoded `ChatRequest` must round-trip identically through `encode_request` -> `decode_request`
//!   and be at most `MAX_CHAT_WIRE` bytes.
//! - Every decoded `ChatResponse` must round-trip identically through `encode_response` -> `decode_response`
//!   and be at most `MAX_CHAT_WIRE` bytes.
//! - Every `Envelope` accepted by `validate_envelope` must obey the stanza and body bounds (`BODY_LIMIT`,
//!   `MAX_NICK_BYTES`, `is_authcid_shape`, no control characters in identifiers).

use chat::federation::{
    CHAT_FEDERATION_VERSION, Envelope, MAX_CHAT_WIRE, decode_request, decode_response, encode_request, encode_response,
    validate_envelope,
};
use chat::muc::MAX_NICK_BYTES;
use chat::server::{BODY_LIMIT, is_authcid_shape};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(req) = decode_request(data) {
        assert!(data.len() <= MAX_CHAT_WIRE);
        assert_eq!(req.version, CHAT_FEDERATION_VERSION);
        let wire = encode_request(&req.op).expect("accepted request must encode");
        let again = decode_request(&wire).expect("encoded request must decode");
        assert_eq!(again, req);
    }

    if let Ok(resp) = decode_response(data) {
        assert!(data.len() <= MAX_CHAT_WIRE);
        assert_eq!(resp.version, CHAT_FEDERATION_VERSION);
        let wire = encode_response(&resp.reply).expect("accepted response must encode");
        let again = decode_response(&wire).expect("encoded response must decode");
        assert_eq!(again, resp);
    }

    if data.len() <= MAX_CHAT_WIRE
        && let Ok(env) = serde_json::from_slice::<Envelope>(data)
        && validate_envelope(&env).is_ok()
    {
        match &env {
            Envelope::Direct {
                mid, from_local, body, ..
            } => {
                assert!(!mid.is_empty() && mid.len() <= 128);
                assert!(is_authcid_shape(from_local));
                assert!(body.len() <= BODY_LIMIT);
            }
            Envelope::Room { mid, nick, body, .. } => {
                assert!(!mid.is_empty() && mid.len() <= 128);
                assert!(!nick.is_empty() && nick.len() <= MAX_NICK_BYTES);
                assert!(body.len() <= BODY_LIMIT);
            }
        }
    }
});
