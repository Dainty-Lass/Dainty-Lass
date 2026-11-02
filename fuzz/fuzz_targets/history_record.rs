//! Change-log entries read back during a restore (DD23, DH4, DH5, DX2): one record per line, from storage the
//! server doesn't control. Nothing panics, an accepted record writes back as itself, a break moves the chain
//! nowhere, and an accepted record resealed onto the chain's head is taken.
#![no_main]

use libfuzzer_sys::fuzz_target;
use store::history::{Chain, GENESIS, entry_hash, record};

fuzz_target!(|data: &[u8]| {
    let mut chain = Chain::genesis();
    for line in data.split(|b| *b == b'\n') {
        let Ok(entry) = record::decode(line) else {
            continue;
        };
        let written = record::encode(&entry).expect("a decoded record is sealed");
        assert_eq!(record::decode(written.as_bytes()).expect("and reads back"), entry);

        let before = chain.head();
        if let Err(broken) = chain.check(&entry) {
            assert_eq!(chain.head(), before);
            assert_eq!(broken.id(), entry.id);
        }

        // The same entry, sealed as the write path would onto the current head.
        let mut resealed = entry.clone();
        let prev = chain.head().map_or(GENESIS.to_owned(), |h| h.hash);
        resealed.hash = Some(entry_hash(&prev, &resealed));
        resealed.prev_hash = Some(prev);
        let next = chain.head().is_none_or(|h| resealed.id > h.id);
        assert_eq!(chain.clone().check(&resealed).is_ok(), next);
    }
});
