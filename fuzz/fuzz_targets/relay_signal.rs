// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The ICE signals a launcher sends through the relay (spec-rd27-rifts-admission-relay.md §4.5.4): an accepted signal
//! is bounded printable SDP, and the relay-only filter passes on only lines the sender wrote, never a host or
//! reflexive candidate.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_rifts_relay::rules::{MAX_SIGNAL_BYTES, SignalKind, filter_relay_only, parse_ice_signal};
use rustic_rifts_relay::wire::js_trim;

const KINDS: [SignalKind; 3] = [
    SignalKind::Description,
    SignalKind::Candidate,
    SignalKind::GatheringDone,
];

/// The lines as the filter sees them: split on LF, one CR dropped, trimmed, blanks gone.
fn sent_lines(data: &str) -> Vec<&str> {
    data.split('\n')
        .map(|piece| js_trim(piece.strip_suffix('\r').unwrap_or(piece)))
        .filter(|line| !line.is_empty())
        .collect()
}

fn is_candidate(line: &str) -> bool {
    line.strip_prefix("a=").unwrap_or(line).starts_with("candidate:")
}

fn check_filter(kind: SignalKind, data: &str) {
    let Some(out) = filter_relay_only(kind, data) else {
        return;
    };
    if kind == SignalKind::GatheringDone {
        assert!(out.is_empty(), "gathering-done data passed on: {out:?}");
        return;
    }
    let sent = sent_lines(data);
    let separator = if data.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = if kind == SignalKind::Candidate {
        vec![out.as_str()]
    } else {
        out.split(separator).collect()
    };
    for line in &lines {
        assert!(sent.contains(line), "a line the sender did not send: {line:?}");
        if is_candidate(line) {
            assert!(line.contains(" typ relay"), "a non-relay candidate: {line:?}");
        }
    }
    if kind == SignalKind::Candidate {
        assert!(is_candidate(&out));
    }
}

fuzz_target!(|data: &[u8]| {
    if let Ok(body) = serde_json::from_slice::<serde_json::Value>(data)
        && let Some(signal) = parse_ice_signal(&body)
    {
        assert!(signal.generation >= 0);
        assert!(signal.data.len() <= MAX_SIGNAL_BYTES);
        assert!(
            signal
                .data
                .bytes()
                .all(|b| (0x20..=0x7e).contains(&b) || b == b'\r' || b == b'\n')
        );
        check_filter(signal.kind, &signal.data);
    }
    let Some((first, rest)) = data.split_first() else {
        return;
    };
    let text = String::from_utf8_lossy(rest);
    check_filter(KINDS[usize::from(*first) % KINDS.len()], &text);
});
