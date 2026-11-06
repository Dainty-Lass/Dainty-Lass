//! What Rustic Isles' shadow replay harness reads (spec-rustic-isles-replay §14 item 8): arbitrary text as capture
//! lines and their bodies, as a target's raw HTTP answer (chunked included), as a key map, through the normalizer,
//! header and chat stanza normalization, and as two answers compared. Nothing panics, a parsed body survives its wire
//! form, and an answer compared with itself has no differences.
#![no_main]

use std::collections::BTreeMap;

use libfuzzer_sys::fuzz_target;
use rustic_isles_replay::{
    CaptureBody, CaptureRecord, IdNormalizer, NormalizerConfig, ReplayHttpResponse, compare_chat_stanzas,
    compare_http_responses, key_map_from_json, normalize_chat_stanza, normalize_headers, parse_capture_line,
    parse_capture_str, parse_http_response,
};
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);

    for record in parse_capture_str(&text) {
        if let CaptureRecord::Http(http) = &record {
            let wire = http.req_body.to_wire();
            assert_eq!(CaptureBody::from_wire(Some(&wire)), http.req_body);
            let _ = ReplayHttpResponse::from_recorded(http);
        }
    }
    let _ = parse_capture_line(&text);

    if let Ok(answer) = parse_http_response(data) {
        let mut left = IdNormalizer::new(NormalizerConfig::default());
        let mut right = IdNormalizer::new(NormalizerConfig::default());
        assert!(compare_http_responses(&answer, &answer, &mut left, &mut right).is_empty());
    }

    let mut normalizer = IdNormalizer::default();
    let _ = normalizer.normalize_str(&text, None);
    let _ = normalizer.normalize_str(&text, Some("instanceId"));
    let _ = normalizer.normalize_raw_text(&text);
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        let _ = normalizer.normalize_value(&value, None);
        let _ = key_map_from_json(&value);
    }
    let headers: BTreeMap<String, String> = text
        .lines()
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.to_owned(), v.to_owned()))
        .collect();
    let _ = normalize_headers(&headers, &mut normalizer);
    let _ = normalize_chat_stanza(&text);

    let mid = (text.len() / 2..=text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(text.len());
    let (a, b) = text.split_at(mid);
    let left = ReplayHttpResponse {
        status: 200,
        headers: BTreeMap::new(),
        body_text: a.to_owned(),
    };
    let right = ReplayHttpResponse {
        status: 200,
        headers: BTreeMap::new(),
        body_text: b.to_owned(),
    };
    let mut l = IdNormalizer::default();
    let mut r = IdNormalizer::default();
    let _ = compare_http_responses(&left, &right, &mut l, &mut r);
    let _ = compare_chat_stanzas(1, &[a.to_owned()], &[b.to_owned()]);
});
