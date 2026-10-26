//! What a host's transport posts to Guard (RF6): the body of `POST /mystic/v1/hunts/:sessionId/event-batches` and
//! `/completion`, through the P2P parser, the batch and completion readers and the chain they are applied to. It
//! checks that an accepted batch is bounded, gap-free and moves the chain to exactly its own hash and sequence.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rifts_guard::evidence::{
    MAX_BATCH_EVENT_BYTES, MAX_BATCH_EVENTS, apply_batch, apply_completion, parse_batch, parse_completion,
};
use rifts_guard::json::parse_p2p_body;
use rifts_guard::{ChainRecord, Disposition};
use rifts_seam::{AccountId, SessionId, UnixMillis};

fuzz_target!(|data: &[u8]| {
    let headers = [("content-type".to_owned(), "application/json".to_owned())];
    let Ok(body) = parse_p2p_body(&headers, data) else {
        return;
    };
    let completion = parse_completion(body.as_ref());
    let Ok(batch) = parse_batch(body.as_ref()) else {
        return;
    };
    let count = batch.events.len() as u64;
    assert!((1..=MAX_BATCH_EVENTS as u64).contains(&count));
    assert!(batch.first_sequence >= 1);
    assert_eq!(batch.last_sequence, batch.first_sequence + count - 1);
    assert!(batch.event_bytes <= MAX_BATCH_EVENT_BYTES);
    assert!(batch.batch_hash.len() == 64 && batch.batch_hash.bytes().all(|b| b.is_ascii_hexdigit()));
    for pair in batch.events.windows(2) {
        assert_eq!(pair[1].sequence, pair[0].sequence + 1);
        assert!(pair[1].monotonic_ms >= pair[0].monotonic_ms);
    }
    assert_eq!(parse_batch(body.as_ref()).map(|again| again.batch_hash), Ok(batch.batch_hash.clone()));

    let Ok(session) = "01ARZ3NDEKTSV4RRFFQ69G5FAV@fuzz".parse::<SessionId>() else {
        return;
    };
    let mut chain = ChainRecord::new(session, 1, None);
    chain.last_sequence = batch.first_sequence - 1;
    chain.head_hash = batch.previous_hash.clone();
    let roster = [AccountId("host".to_owned())];
    match apply_batch(&chain, &batch, &roster, UnixMillis(0)) {
        Ok(update) => {
            assert_eq!(update.chain.last_sequence, batch.last_sequence);
            assert_eq!(update.chain.head_hash, batch.batch_hash);
            assert_eq!(update.chain.revision, chain.revision + 1);
            assert_eq!(update.chain.event_count, count);
            assert_eq!(update.chain.finding_count, update.findings.len() as u64);
            assert_eq!(update.chain.disposition == Disposition::ObserveFlagged, !update.findings.is_empty());
            if let Ok(request) = completion {
                let _ = apply_completion(&update.chain, &request, UnixMillis(1));
            }
        }
        Err(refusal) => assert!(refusal.error.code().is_some_and(|code| code.starts_with("GUARD_"))),
    }
});
