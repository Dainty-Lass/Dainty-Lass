// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Passwords (spec §5.4): Argon2id PHC strings, and a dummy verification when there is no hash so a miss takes as
//! long as a wrong password.

use std::sync::OnceLock;

use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier as _, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

use crate::util;

fn hasher() -> Result<Argon2<'static>, argon2::Error> {
    Ok(Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(19_456, 2, 1, Some(32))?,
    ))
}

fn hash_with_salt(password: &str, salt: &[u8]) -> Option<String> {
    let salt = SaltString::encode_b64(salt).ok()?;
    let hash = hasher().ok()?.hash_password(password.as_bytes(), &salt).ok()?;
    Some(hash.to_string())
}

/// The Argon2 hasher refused its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the password could not be hashed")]
pub struct HashError;

/// `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>` with a fresh 16-byte salt. Slow: call it off the async threads.
pub fn hash_password(password: &str) -> Result<String, HashError> {
    hash_with_salt(password, &util::random_bytes::<16>()).ok_or(HashError)
}

fn dummy_hash() -> Option<&'static str> {
    static DUMMY: OnceLock<Option<String>> = OnceLock::new();
    DUMMY
        .get_or_init(|| hash_with_salt("no account has this password", &[7u8; 16]))
        .as_deref()
}

/// Any well-formed Argon2 PHC string, with its own parameters; a malformed hash is false, and no hash verifies the
/// dummy so the time matches. Slow: call it off the async threads.
pub fn verify_password(hash: Option<&str>, password: &str) -> bool {
    let (target, real) = match hash {
        Some(hash) => (Some(hash), true),
        None => (dummy_hash(), false),
    };
    let Some(target) = target else {
        return false;
    };
    let Ok(parsed) = PasswordHash::new(target) else {
        return false;
    };
    let matched = Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok();
    matched && real
}

/// `hash_password` on the blocking pool.
pub async fn hash_password_blocking(password: String) -> Result<String, HashError> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|_| HashError)?
}

/// `verify_password` on the blocking pool.
pub async fn verify_password_blocking(hash: Option<String>, password: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(hash.as_deref(), &password))
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_verify_and_bad_input_is_false() {
        let hash = hash_password("correct horse").unwrap();
        assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(verify_password(Some(&hash), "correct horse"));
        assert!(!verify_password(Some(&hash), "wrong horse"));
        assert!(!verify_password(Some("not a hash"), "correct horse"));
        assert!(!verify_password(None, "correct horse"));
        assert!(!verify_password(None, "no account has this password"));
    }
}
