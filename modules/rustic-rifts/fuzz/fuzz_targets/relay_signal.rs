//! ICE signals (RF4): the body of `POST /launcher/v1/p2p/hunts/:sessionId/signal`, read by `parse_ice_signal`, and
//! the relay-only filter it goes through under `P2P_FORCE_SDR`. It checks that an accepted signal is bounded
//! printable SDP and that the filter never lets an address through: no candidate but a relay one, no line the
//! sender did not send.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rifts_relay::rules::{MAX_SIGNAL_BYTES, SignalKind, filter_relay_only, parse_ice_signal};
use rifts_relay::wire::js_trim;

fuzz_target!(|data: &[u8]| {
    let Ok(body) = serde_json::from_slice::<serde_json::Value>(data) else {
        return;
    };
    let Some(signal) = parse_ice_signal(&body) else {
        return;
    };
    assert!(signal.generation >= 0 && signal.data.len() <= MAX_SIGNAL_BYTES);
    assert!(
        signal
            .data
            .bytes()
            .all(|b| (0x20..=0x7e).contains(&b) || b == b'\r' || b == b'\n')
    );
    let Some(kept) = filter_relay_only(signal.kind, &signal.data) else {
        return;
    };
    if signal.kind == SignalKind::GatheringDone {
        assert_eq!(kept, signal.data);
        return;
    }
    // Lines as the filter reads them: split at LF, one CR dropped, JavaScript's trim
    let sent: Vec<&str> = signal
        .data
        .split('\n')
        .map(|line| js_trim(line.strip_suffix('\r').unwrap_or(line)))
        .collect();
    let separator = if signal.data.contains("\r\n") { "\r\n" } else { "\n" };
    for line in kept.split(separator) {
        assert!(sent.contains(&line), "{line:?} was not sent");
        let candidate = line.strip_prefix("a=").unwrap_or(line).starts_with("candidate:");
        assert!(
            !candidate || line.contains(" typ relay"),
            "{line:?} is not a relay candidate"
        );
        if signal.kind == SignalKind::Candidate {
            assert!(candidate);
        }
    }
});
