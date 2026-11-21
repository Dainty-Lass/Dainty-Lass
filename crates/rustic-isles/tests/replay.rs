// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The process-wide test pin against the replay scope (core spec §9.1), in a binary of its own because the pin is
//! process-wide.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use rustic_isles::replay::{
    ReplayScope, is_replay_pinned, now_ms, pin_now_for_tests, replay_now_ms, with_replay_scope,
};

#[tokio::test]
async fn a_scope_time_overrides_the_test_pin() {
    pin_now_for_tests(Some(5));
    assert_eq!(now_ms(), 5);
    assert_eq!(replay_now_ms(), None);
    assert!(is_replay_pinned());
    let inside = with_replay_scope(ReplayScope::new(Some(9), None), async { (now_ms(), replay_now_ms()) }).await;
    assert_eq!(inside, (9, Some(9)));
    let seeded = with_replay_scope(ReplayScope::new(None, Some("s")), async { (now_ms(), replay_now_ms()) }).await;
    assert_eq!(seeded, (5, None));
    pin_now_for_tests(None);
    assert!(!is_replay_pinned());
    assert!(now_ms() > 1_700_000_000_000);
}
