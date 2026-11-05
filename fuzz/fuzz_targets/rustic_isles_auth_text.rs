//! What Rustic Isles' sign-in area reads as text from players and the network (spec-rustic-isles-auth §13):
//! usernames, email addresses, launcher reports and versions, runtime names and versions, encryption-key tokens,
//! client addresses, TOTP secrets and query strings. None of it may panic, and each keeps its invariant.
#![no_main]

use axum::http::{HeaderMap, HeaderValue};
use libfuzzer_sys::fuzz_target;
use rustic_isles_auth::{email, enckeys, launcher_build, policy, runtime, totp};
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let value = Value::String(text.to_string());

    if let Ok(name) = policy::validate_username_format(Some(&value)) {
        assert!((3..=16).contains(&name.len()));
        assert!(name.bytes().all(|b| b.is_ascii_alphanumeric()) && name.bytes().any(|b| b.is_ascii_alphabetic()));
        let _ = policy::check_username_allowed(&name, &policy::parse_blocklist(Some(&text)));
    }
    let _ = policy::collapse(&text);

    if email::is_valid_email(&text) {
        assert_eq!(text.matches('@').count(), 1);
    }
    let masked = email::mask_email(&text);
    if let Some(at) = text.rfind('@').filter(|&at| at > 0) {
        let units: Vec<u16> = text[..at].encode_utf16().collect();
        if units.len() > 2 {
            let middle = String::from_utf16_lossy(&units[1..units.len() - 1]);
            if !middle.contains('*') && middle.len() > 3 {
                assert!(!masked.contains(&format!("{middle}@")));
            }
        }
    }

    if let Ok(json) = serde_json::from_str::<Value>(&text) {
        if let Some(report) = launcher_build::parse_launcher_report(Some(&json)) {
            assert!(report.sha256.len() == 64 && !report.sha256.bytes().any(|b| b.is_ascii_uppercase()));
        }
    }
    let (a, b) = text.split_once(' ').unwrap_or((&text, "0.0.1"));
    let _ = launcher_build::compare_versions(a, b);
    let _ = runtime::is_safe_version(&text);
    let _ = runtime::is_safe_artifact_name(&text);

    let normalized = enckeys::normalize_token(&text);
    // One leading `bearer` and its white space go, and the result is trimmed
    assert!(!normalized.starts_with(' ') && !normalized.ends_with(' '));
    let printable = text.bytes().all(|b| b == b' ' || b.is_ascii_graphic());
    if printable && let Some(rest) = text.trim_matches(' ').strip_prefix("bearer ") {
        assert_eq!(normalized, rest.trim_matches(' '));
    }
    let _ = enckeys::fingerprint(&text);

    let mut headers = HeaderMap::new();
    if let Ok(xff) = HeaderValue::from_str(&text) {
        headers.insert("x-forwarded-for", xff);
    }
    let ip = rustic_isles_auth::client_ip_from("127.0.0.1", &headers);
    let _ = rustic_isles_auth::rate_limit_key(&ip);
    let _ = rustic_isles_auth::rate_limit_key(&text);

    let _ = totp::decode_base32(&text);
    let _ = totp::verify_totp(&text, Some(&Value::String("123456".into())), 0);
    let _ = rustic_isles_auth::query_pairs(Some(&text));
});
