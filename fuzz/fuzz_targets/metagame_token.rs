//! The legacy metagame's session tokens and recovery blobs (DR's `auth_token`; DR M9, M20, M22, X9): whatever a
//! client sends as a bearer token or a recovery blob must be refused cleanly, never accepted and never a panic.
//! Only a token signed with the server's own key verifies, so random input never does. The log redaction runs
//! on the same text: no account key may come out of it.
#![no_main]

use std::sync::LazyLock;

use libfuzzer_sys::fuzz_target;
use metagame::crypto::{TokenKeys, open_recovery};
use metagame::redact::redact;
use serde_json::Value;

static FIXTURE: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../services/metagame/tests/fixtures/node-crypto.json")).unwrap_or(Value::Null)
});

static KEYS: LazyLock<Option<TokenKeys>> =
    LazyLock::new(|| TokenKeys::from_base64_pem(FIXTURE["privB64"].as_str()?, FIXTURE["pubB64"].as_str()?).ok());

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Some(keys) = KEYS.as_ref() {
        assert!(keys.verify(&text).is_err(), "an unsigned token must never verify");
        let _ = keys.verify_detailed(&text);
    }
    if let Some(signing) = FIXTURE["privB64"].as_str() {
        assert!(
            open_recovery(signing, "UID-fuzz", &text).is_err(),
            "a forged recovery blob must never open"
        );
    }
    let redacted = redact(&text);
    for prefix in ["RRK_", "UUK_"] {
        let whole_key = redacted.match_indices(prefix).any(|(at, _)| {
            let hex = &redacted.as_bytes()[at + 4..];
            hex.len() >= 48
                && hex[..48].iter().all(u8::is_ascii_hexdigit)
                && hex.get(48).is_none_or(|b| !b.is_ascii_hexdigit())
        });
        assert!(!whole_key, "an account key survived the redaction");
    }
});
