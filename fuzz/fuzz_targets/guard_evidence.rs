// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The evidence a host's transport posts to Guard: the body parser, the batch and completion readers and the chain
//! they move (RX3, docs/security/rifts-guard.md).
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_rifts_guard::ChainRecord;
use rustic_rifts_guard::evidence::{
    CompletionRequest, MAX_BATCH_EVENT_BYTES, MAX_BATCH_EVENTS, apply_batch, apply_completion, parse_batch,
    parse_completion,
};
use rustic_rifts_guard::json::parse_p2p_body;
use rustic_rifts_seam::{AccountId, SessionId, UnixMillis};

fn is_hex64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn guard_code(error: &rustic_rifts_guard::GuardError) -> bool {
    error.code().is_some_and(|code| code.starts_with("GUARD_"))
}

fuzz_target!(|data: &[u8]| {
    let headers = [("content-type".to_owned(), "application/json".to_owned())];
    let Ok(body) = parse_p2p_body(&headers, data) else {
        return;
    };
    let _ = parse_completion(body.as_ref());
    let Ok(batch) = parse_batch(body.as_ref()) else {
        return;
    };
    let count = batch.events.len() as u64;
    assert!((1..=MAX_BATCH_EVENTS as u64).contains(&count));
    assert!(batch.first_sequence >= 1);
    assert_eq!(batch.last_sequence, batch.first_sequence + count - 1);
    assert!(batch.event_bytes <= MAX_BATCH_EVENT_BYTES);
    assert!(is_hex64(&batch.batch_hash));
    for (i, pair) in batch.events.windows(2).enumerate() {
        assert_eq!(pair[1].sequence, pair[0].sequence + 1, "event {i}");
        assert!(pair[1].monotonic_ms >= pair[0].monotonic_ms);
    }
    assert_eq!(
        parse_batch(body.as_ref()).map(|b| b.batch_hash),
        Ok(batch.batch_hash.clone())
    );

    let Ok(session) = "01ARZ3NDEKTSV4RRFFQ69G5FAV@fuzz".parse::<SessionId>() else {
        return;
    };
    let mut chain = ChainRecord::new(session, 1, None);
    chain.last_sequence = batch.first_sequence - 1;
    chain.head_hash = batch.previous_hash.clone();
    let roster = [AccountId("host".to_owned())];
    let update = match apply_batch(&chain, &batch, &roster, UnixMillis(1)) {
        Ok(update) => update,
        Err(refusal) => {
            assert!(guard_code(&refusal.error));
            return;
        }
    };
    let next = &update.chain;
    assert_eq!(next.last_sequence, batch.last_sequence);
    assert_eq!(next.head_hash, batch.batch_hash);
    assert_eq!(next.revision, chain.revision + 1);
    assert_eq!(next.event_count, count);
    assert_eq!(next.finding_count, update.findings.len() as u64);
    assert_eq!(
        next.disposition.as_str() == "OBSERVE_FLAGGED",
        !update.findings.is_empty()
    );

    let request = CompletionRequest {
        final_sequence: next.last_sequence,
        final_hash: next.head_hash.clone(),
    };
    match apply_completion(next, &request, UnixMillis(2)) {
        Ok(done) => assert!(done.is_some_and(|d| d.chain.completion.is_some())),
        Err(error) => assert!(guard_code(&error)),
    }
});
