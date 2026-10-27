//! MPC1 admission control frames (RF4): what a peer sends inside a sealed control frame, the joiner's admission
//! request and the host's result (`src/control_codec.cpp`). It checks that a frame that decodes is exactly the frame
//! its contents encode to, and that an admission always carries a 64-byte id.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rifts_admission::control::{
    MAX_CONTROL_FRAME, decode_admission_request, decode_admission_result, encode_admission_request,
    encode_admission_result,
};

fuzz_target!(|data: &[u8]| {
    if let Some(ticket) = decode_admission_request(data) {
        assert!(!ticket.is_empty() && ticket.len() <= 4096 && data.len() <= MAX_CONTROL_FRAME);
        assert_eq!(encode_admission_request(ticket).as_deref(), Some(data));
    }
    if let Some(result) = decode_admission_result(data) {
        assert!((1..=64).contains(&result.code.len()) && result.admission_id.len() <= 64);
        assert!(!result.admitted || result.admission_id.len() == 64);
        assert_eq!(encode_admission_result(&result).as_deref(), Some(data));
    }
});
