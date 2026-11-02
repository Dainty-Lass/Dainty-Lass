// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The control frames a host's transport reads from a joiner (spec-rd27-rifts-admission-relay.md §3.4): whatever
//! decodes is within the frame's bounds and re-encodes to the same bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rifts_admission::control::{
    MAX_CONTROL_FRAME, decode_admission_request, decode_admission_result, encode_admission_request,
    encode_admission_result,
};

fuzz_target!(|data: &[u8]| {
    if let Some(ticket) = decode_admission_request(data) {
        assert!((1..=4096).contains(&ticket.len()));
        assert!(data.len() <= MAX_CONTROL_FRAME);
        assert_eq!(encode_admission_request(ticket).as_deref(), Some(data));
    }
    if let Some(result) = decode_admission_result(data) {
        assert!((1..=64).contains(&result.code.len()));
        assert!(result.admission_id.len() <= 64);
        assert!(!result.admitted || result.admission_id.len() == 64);
        assert_eq!(encode_admission_result(&result).as_deref(), Some(data));
    }
});
