#![no_main]

//! Fuzzes the peer log shipping (`Service::LogShip`, DH7) wire codec and object key parser
//! (`dainty_network::logship::{decode_put_request, encode_put_request, decode_put_response, encode_put_response, parse_peer_object_key}`):
//! - Neither decoder may panic on any byte slice, regardless of length or UTF-8 validity.
//! - Every decoded `LogShipPutRequest` must round-trip identically through `encode_put_request` -> `decode_put_request`,
//!   and if `decode_body()` succeeds, the decoded body must be at most `MAX_LOG_SHIP_OBJECT_BYTES`.
//! - Every decoded `LogShipPutResponse` must round-trip identically through `encode_put_response` -> `decode_put_response`.
//! - `parse_peer_object_key` must never panic on any key string and must only accept keys starting with `<from>/log/`
//!   or `<from>/snapshot/` with a 64-character lowercase hex SHA-256 digest and no path traversal components.

use libfuzzer_sys::fuzz_target;
use dainty_module_api::network::NodeId;
use dainty_network::logship::{
    LOG_SHIP_VERSION, MAX_LOG_SHIP_OBJECT_BYTES, MAX_LOG_SHIP_WIRE, decode_put_request, decode_put_response,
    encode_put_request, encode_put_response, parse_peer_object_key,
};

fuzz_target!(|data: &[u8]| {
    if let Ok(req) = decode_put_request(data) {
        assert!(data.len() <= MAX_LOG_SHIP_WIRE);
        let wire = encode_put_request(&req).expect("accepted request must encode");
        let again = decode_put_request(&wire).expect("encoded request must decode");
        assert_eq!(again, req);
        if let Ok(body) = req.decode_body() {
            assert!(body.len() <= MAX_LOG_SHIP_OBJECT_BYTES);
        }
        let _ = req.version == LOG_SHIP_VERSION;
    }

    if let Ok(resp) = decode_put_response(data) {
        assert!(data.len() <= MAX_LOG_SHIP_WIRE);
        let wire = encode_put_response(&resp).expect("accepted response must encode");
        let again = decode_put_response(&wire).expect("encoded response must decode");
        assert_eq!(again, resp);
    }

    if let Ok(text) = std::str::from_utf8(data) {
        let origin: NodeId = "oregon-1".parse().expect("valid static node id");
        if let Some((rel, expected_sha256)) = parse_peer_object_key(&origin, text) {
            assert!(rel.starts_with("log/") || rel.starts_with("snapshot/"));
            assert!(!rel.contains('\\'));
            assert!(!rel.contains(".."));
            assert_eq!(expected_sha256.len(), 64);
            assert!(
                expected_sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            );
        }
    }
});
