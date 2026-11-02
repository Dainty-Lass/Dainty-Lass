// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What a peer sends over a link (spec-rd27-rifts-transport.md §2.10, §2.11): sealed frames opened with a fixed key,
//! then fragment reassembly. Nothing opens without the tag, and a reassembled message is at most four pieces.
#![no_main]

use std::time::{Duration, Instant};

use libfuzzer_sys::fuzz_target;
use rifts_transport::fragment::{MAX_PIECES, PIECE_BYTES, Piece, Reassembler};
use rifts_transport::seal::{FrameType, PairCipher, Side};

fuzz_target!(|data: &[u8]| {
    let Ok(cipher) = PairCipher::new(&[7; 32], Side::Host) else {
        return;
    };
    let mut reassembler = Reassembler::default();
    let start = Instant::now();
    for (index, chunk) in data.split(|b| *b == 0xff).enumerate() {
        // Forging a tag is out of reach, so an opened frame means a bug in the window or the checks.
        assert!(cipher.open(chunk).is_none());
        let now = start + Duration::from_millis(index as u64 * 100);
        if let Piece::Complete(kind, message) = reassembler.add(chunk, now) {
            assert!(matches!(kind, FrameType::Control | FrameType::Gameplay));
            assert!(message.len() <= MAX_PIECES * data.len().max(PIECE_BYTES));
        }
    }
});
