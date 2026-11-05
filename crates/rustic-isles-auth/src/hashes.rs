// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! HMAC hashes under `API_KEY_HASH_SECRET` (spec §5.3) and their constant-time comparison.

use aws_lc_rs::hmac;
use subtle::ConstantTimeEq as _;

use crate::util;

pub const SECRET_TOO_SHORT: &str = "API_KEY_HASH_SECRET must contain at least 32 characters.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("API_KEY_HASH_SECRET must contain at least 32 characters.")]
pub struct SecretError;

/// The JS-trimmed secret, refused under 32 UTF-16 units.
pub fn read_secret(raw: Option<&str>) -> Result<&str, SecretError> {
    let secret = service_kit::js::trim(raw.unwrap_or(""));
    if util::utf16_len(secret) < 32 {
        return Err(SecretError);
    }
    Ok(secret)
}

/// HMAC-SHA256 of the parts joined by one NUL byte, lower-case hex.
pub fn keyed_hash(secret: &str, parts: &[&str]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let mut context = hmac::Context::with_key(&key);
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            context.update(&[0]);
        }
        context.update(part.as_bytes());
    }
    util::hex(context.sign().as_ref())
}

pub fn gameserver_key_hash(secret: &str, key: &str) -> String {
    keyed_hash(secret, &["gameserver", key])
}

pub fn user_key_hash(secret: &str, key: &str) -> String {
    keyed_hash(secret, &["user", key])
}

pub fn email_code_hash(secret: &str, purpose: &str, user_id: &str, code: &str) -> String {
    keyed_hash(secret, &["email-code", purpose, user_id, code])
}

/// The erasure pseudonym of an account id.
pub fn pseudonymous_account_id(secret: &str, account_id: &str) -> String {
    keyed_hash(secret, &["account-pseudonym", account_id])
}

/// Hex-decoded constant-time equality; malformed hex, empty or unequal lengths never match.
pub fn hashes_equal(a: &str, b: &str) -> bool {
    let (Some(a), Some(b)) = (util::unhex(a), util::unhex(b)) else {
        return false;
    };
    if a.is_empty() || a.len() != b.len() {
        return false;
    }
    a.ct_eq(&b).into()
}

/// The first record matching `candidate`; every record is compared so the time does not reveal the position.
pub fn first_match<'a>(candidate: &str, records: impl IntoIterator<Item = Option<&'a str>>) -> Option<usize> {
    let mut found = None;
    for (i, record) in records.into_iter().enumerate() {
        let matched = record.is_some_and(|hash| hashes_equal(candidate, hash));
        if matched && found.is_none() {
            found = Some(i);
        }
    }
    found
}

/// `GAMESERVER_API_KEYS`: a comma list, JS-trimmed, empties dropped, de-duplicated in order.
pub fn parse_gameserver_keys(raw: Option<&str>) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for key in raw.unwrap_or("").split(',').map(service_kit::js::trim) {
        if !key.is_empty() && !keys.iter().any(|k| k == key) {
            keys.push(key.to_owned());
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-only-api-key-hash-secret-0123456789abcdef";

    #[test]
    fn hashes_match_the_spec_vectors() {
        assert_eq!(
            user_key_hash(SECRET, "RRK_x"),
            "b3a5c200029e048fe7747e1a3630111f6779d88f804a066b10dd3319f9c383ba"
        );
        assert_eq!(
            gameserver_key_hash(SECRET, "gs-key-1"),
            "4ce6fee736d315052f090e75a9093695ea4a2903b3af0d220c5545192e319043"
        );
        assert_eq!(
            email_code_hash(SECRET, "verify", "u1", "123456"),
            "2facdf75f9131ef6545a27938ee253222b949cd76c003e107835c872f7dfc246"
        );
        assert_ne!(user_key_hash(SECRET, "k"), gameserver_key_hash(SECRET, "k"));
        assert_ne!(
            email_code_hash(SECRET, "verify", "u1", "1"),
            email_code_hash(SECRET, "verify", "u2", "1")
        );
        assert_ne!(
            email_code_hash(SECRET, "verify", "u1", "1"),
            email_code_hash(SECRET, "reset", "u1", "1")
        );
        assert_ne!(
            email_code_hash(SECRET, "verify", "u1", "1"),
            email_code_hash(SECRET, "verify", "u1", "2")
        );
    }

    #[test]
    fn short_or_missing_secrets_fail() {
        assert_eq!(read_secret(None), Err(SecretError));
        assert_eq!(read_secret(Some("  short  ")), Err(SecretError));
        assert_eq!(read_secret(Some(&format!(" {SECRET} "))), Ok(SECRET));
        assert_eq!(SecretError.to_string(), SECRET_TOO_SHORT);
    }

    #[test]
    fn comparisons_are_strict() {
        assert!(hashes_equal("00ff", "00ff"));
        assert!(!hashes_equal("00ff", "00fe"));
        assert!(!hashes_equal("00ff", "00ff00"));
        assert!(!hashes_equal("", ""));
        assert!(!hashes_equal("zz", "zz"));
        assert_eq!(first_match("bb", [Some("aa"), None, Some("bb"), Some("bb")]), Some(2));
    }

    #[test]
    fn gameserver_key_lists_are_trimmed_and_deduplicated() {
        assert_eq!(
            parse_gameserver_keys(Some(" first, second ,,first,third ")),
            ["first", "second", "third"]
        );
        assert!(parse_gameserver_keys(None).is_empty());
        assert!(parse_gameserver_keys(Some("")).is_empty());
        assert!(parse_gameserver_keys(Some(" , ")).is_empty());
    }
}
