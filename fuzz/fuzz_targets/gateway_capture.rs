// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The traffic capture's redaction (RD19, RD27): bodies, headers, targets and chat frames never panic, no
//! account key from the input survives, and the chat frame reader gives the same messages however the bytes are
//! split.
#![no_main]

use gateway::capture::{
    FrameReader, redact_capture_body, redact_capture_headers, redact_capture_target, redact_chat_frame,
};
use http::{HeaderMap, HeaderValue};
use libfuzzer_sys::fuzz_target;

/// Every account key in `text`, whole.
fn keys(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for prefix in ["RRK_", "UUK_"] {
        let mut rest = text;
        while let Some(at) = rest.find(prefix) {
            let len = rest[at + 4..].bytes().take_while(u8::is_ascii_alphanumeric).count();
            if len > 0 {
                found.push(rest[at..at + 4 + len].to_owned());
            }
            rest = &rest[at + 4..];
        }
    }
    found
}

fuzz_target!(|input: (Vec<u8>, String, u8)| {
    let (bytes, content_type, split) = input;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mut outputs = vec![
        redact_capture_body(&bytes, Some(&content_type), bytes.len() as u64, 1 << 20).to_string(),
        redact_capture_target(&text),
        redact_chat_frame(&text),
    ];
    if let Ok(value) = HeaderValue::from_bytes(&bytes) {
        let mut headers = HeaderMap::new();
        headers.insert("x-reforged-user-api-key", value.clone());
        headers.insert("authorization", value);
        outputs.push(redact_capture_headers(&headers).to_string());
    }
    for key in keys(&text) {
        for out in &outputs {
            assert!(!out.contains(&key), "{key} survived in {out}");
        }
    }
    let mut whole = FrameReader::new(64);
    let all = whole.push(&bytes);
    let mut pieces = FrameReader::new(64);
    let parts: Vec<_> = bytes
        .chunks(usize::from(split.max(1)))
        .flat_map(|c| pieces.push(c))
        .collect();
    assert_eq!(all, parts);
});
