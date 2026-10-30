// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Fuzz target for remote management (`docs/security/remote-management.md`, DD27, RM1):
//! exercises `parse_enrollment_uri`, `decode_enroll_request`, `decode_enroll_answer`,
//! `decode_request`, `decode_response`, `contains_forbidden_secret_str`, and
//! `merge_managed_host_toml` on untrusted bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use remote::{
    MAX_REMOTE_WIRE, contains_forbidden_secret_bytes, contains_forbidden_secret_str,
    contains_prefixed_key_token, decode_enroll_answer, decode_enroll_request, decode_request,
    decode_response, encode_enroll_answer, encode_enroll_request, encode_request, encode_response,
    merge_managed_host_toml, parse_enrollment_uri,
};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if contains_prefixed_key_token(&text) {
        assert!(contains_forbidden_secret_str(&text));
        assert!(contains_forbidden_secret_bytes(data));
    }

    if let Ok(invite) = parse_enrollment_uri(&text) {
        assert!(invite.validate().is_ok());
        let uri = invite.to_uri().expect("valid invite serializes to URI");
        let reparsed = parse_enrollment_uri(&uri).expect("serialized URI parses back");
        assert_eq!(reparsed, invite);
    }

    if let Ok(req) = decode_enroll_request(data) {
        assert!(data.len() <= MAX_REMOTE_WIRE);
        assert!(!contains_forbidden_secret_bytes(data));
        let encoded = encode_enroll_request(&req).expect("encodes back");
        assert_eq!(decode_enroll_request(&encoded).expect("decodes back"), req);
    }

    if let Ok(ans) = decode_enroll_answer(data) {
        assert!(data.len() <= MAX_REMOTE_WIRE);
        assert!(!contains_forbidden_secret_bytes(data));
        let encoded = encode_enroll_answer(&ans).expect("encodes back");
        assert_eq!(decode_enroll_answer(&encoded).expect("decodes back"), ans);
    }

    if let Ok(req) = decode_request(data) {
        assert!(data.len() <= MAX_REMOTE_WIRE);
        assert!(!contains_forbidden_secret_bytes(data));
        let encoded = encode_request(&req).expect("encodes back");
        assert_eq!(decode_request(&encoded).expect("decodes back"), req);
    }

    if let Ok(resp) = decode_response(data) {
        assert!(data.len() <= MAX_REMOTE_WIRE);
        assert!(!contains_forbidden_secret_bytes(data));
        let encoded = encode_response(&resp).expect("encodes back");
        assert_eq!(decode_response(&encoded).expect("decodes back"), resp);
    }

    let base_local = "[server]\nhost = \"203.0.113.10\"\nmode = \"public\"\n[ports]\ngateway = 61000\n";
    if let Ok(merged) = merge_managed_host_toml(base_local, &text, None, &[]) {
        assert!(!contains_forbidden_secret_str(&merged));
        assert!(merged.contains("host = \"203.0.113.10\""));
        assert!(merged.contains("gateway = 61000"));
    }
});
