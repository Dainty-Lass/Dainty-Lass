//! What the launcher reads back from its own files, which anything running as the player can rewrite (RD27 spec
//! §2.7.1, §2.7.2, RX3): `settings.json`, the decrypted text of a key file and the framing of an encrypted one.
#![no_main]

use launcher_kit::core::i18n::Language;
use launcher_kit::platform::keystore::format::{
    ELECTRON_V10, ELECTRON_V11, LINUX_MAGIC, NONCE_BYTES, TAG_BYTES, join_gcm, parse_text, split_cbc, split_gcm,
};
use launcher_kit::platform::keystore::slot_id;
use launcher_kit::platform::settings::{MAX_SAVED_SERVERS, Os, sanitize, to_bytes};
use libfuzzer_sys::fuzz_target;
use protocol::account::is_plausible_account_key;
use protocol::invite::{ServerMode, is_valid_fingerprint};
use serde_json::Value;

fn settings(raw: &Value) {
    for os in [Os::Windows, Os::Linux] {
        for language in Language::ALL {
            let clean = sanitize(raw, language, os);
            let saved = clean.get("savedServers").and_then(Value::as_array).map_or(0, Vec::len);
            assert!(saved <= MAX_SAVED_SERVERS);
            // What is written reads back as itself, so a rewrite never drifts
            let written: Value = serde_json::from_slice(&to_bytes(&clean)).unwrap_or(Value::Null);
            assert_eq!(sanitize(&written, language, os), clean);
        }
    }
}

fn key_text(data: &[u8]) {
    let Some(stored) = parse_text(data) else {
        return;
    };
    // Only an account key, bound to a valid certificate or none, and it writes back unchanged
    assert!(is_plausible_account_key(&stored.key));
    assert!(stored.fingerprint.as_deref().is_none_or(is_valid_fingerprint));
    assert_eq!(parse_text(stored.to_text().as_bytes()), Some(stored));
}

fn sealed(data: &[u8]) {
    for prefix in [LINUX_MAGIC, ELECTRON_V10] {
        if let Some((nonce, rest)) = split_gcm(data, prefix) {
            assert!(rest.len() >= TAG_BYTES && nonce.len() == NONCE_BYTES);
            assert_eq!(join_gcm(prefix, nonce, rest), data);
        }
    }
    if let Some((v11, body)) = split_cbc(data) {
        assert!(!body.is_empty() && body.len() % 16 == 0);
        assert_eq!(data.starts_with(ELECTRON_V11), v11);
    }
}

fuzz_target!(|data: &[u8]| {
    if let Ok(raw) = serde_json::from_slice::<Value>(data) {
        settings(&raw);
    }
    key_text(data);
    sealed(data);
    let host = String::from_utf8_lossy(data);
    for mode in [ServerMode::Private, ServerMode::Public] {
        let id = slot_id(mode, &host, 61000);
        // A slot id is a file name: 24 lower-case hex digits, whatever the host
        assert!(id.len() == 24 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    }
});
