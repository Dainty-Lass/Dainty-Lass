//! What Launcher Guard reads (spec-rustic-isles-reforged-public §19): arbitrary JSON as a Guard report, a heartbeat's
//! signals and a manifest envelope (fixed test keys, a fixed time), and text pairs for `version_at_least`. Nothing
//! panics; a parsed report holds at most 32 modules with lower-case hashes; a verified envelope has 1 to 128
//! artifacts and the asked changelist; the risk stays 0 to 100.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_reforged::guard::{Signals, explain_continuity, explain_signals, version_at_least};
use rustic_isles_reforged::guard_manifest::{Trusted, verify_envelope};

const NOW: i64 = 1_792_000_000_000;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (a, b) = text.split_once('\n').unwrap_or((&text, ""));
    let _ = version_at_least(a, b);
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) else {
        return;
    };
    if let Some(signals) = Signals::parse(&value) {
        let modules = signals.modules.as_deref().unwrap_or(&[]);
        assert!(modules.len() <= 32);
        assert!(modules.iter().all(|m| {
            m.sha256
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        }));
        let (score, _) = explain_signals(&signals, 1 << 20, 1 << 16);
        let (more, _) = explain_continuity(Some(&signals), &signals, 3);
        assert!((0..=100).contains(&score) && (score + more).min(100) <= 100);
    }
    if let Some(inner) = value.get("signals").and_then(Signals::parse) {
        let _ = explain_signals(&inner, 0, 0);
    }
    let trusted = Trusted::with_keys([7u8; 32], [9u8; 32]);
    if let Ok(payload) = verify_envelope(&value, 647_472, &["EXTRA"], NOW, &trusted) {
        assert!((1..=128).contains(&payload.artifacts.len()));
        assert_eq!(payload.changelist, 647_472);
    }
});
