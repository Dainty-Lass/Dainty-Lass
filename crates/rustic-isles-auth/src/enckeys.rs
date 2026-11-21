// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The hub's network encryption keys (spec §5.6): issued to a player, fetched by the hub. In memory only.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use crate::util;

/// How long an issued key can be fetched: 30 minutes.
pub const KEY_TTL_MS: i64 = 30 * 60 * 1000;

/// A key issued to a player for the hub's network encryption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedKey {
    /// The key: 32 random bytes as hex.
    pub key: String,
    /// The nonce: 32 random bytes as base64url.
    pub nonce: String,
    /// The token as the player sent it
    pub token: String,
    /// When it was issued, in Unix milliseconds.
    pub at: i64,
    /// The account it was issued to, when the caller named one.
    pub account: Option<String>,
}

/// A lookup's key and why it may be doubtful.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyLookup {
    /// The key found, if any.
    pub key: Option<IssuedKey>,
    /// The most recent key answered a token nothing was issued under (it may be another player's)
    pub unknown_token: bool,
    /// A key was found but its account isn't in the caller's roster
    pub roster_rejected: bool,
}

#[derive(Default)]
struct Keys {
    by_candidate: HashMap<String, IssuedKey>,
    by_token: HashMap<String, IssuedKey>,
    recent: Option<IssuedKey>,
}

/// The issued keys, by candidate and by token, plus the most recent.
#[derive(Default)]
pub struct EncryptionKeys {
    keys: Mutex<Keys>,
}

/// JS-trim, drop a leading `bearer` (any case) followed by white space, trim again.
pub fn normalize_token(token: &str) -> String {
    let trimmed = dainty_service_kit::js::trim(token);
    let bytes = trimmed.as_bytes();
    if bytes.len() > 6 && trimmed[..6].eq_ignore_ascii_case("bearer") {
        let rest = &trimmed[6..];
        if rest.starts_with(dainty_service_kit::js::is_space) {
            return dainty_service_kit::js::trim(rest).to_owned();
        }
    }
    trimmed.to_owned()
}

/// The first 12 hex of the normalized token's SHA-256, `none` for an empty one.
pub fn fingerprint(token: &str) -> String {
    let normalized = normalize_token(token);
    if normalized.is_empty() {
        return "none".to_owned();
    }
    util::sha256_hex(&normalized)[..12].to_owned()
}

fn live(key: &IssuedKey, now: i64) -> bool {
    now - key.at <= KEY_TTL_MS
}

impl EncryptionKeys {
    /// A fresh key, kept under `candidate` and the normalized token (each when non-empty) and as the most recent.
    pub fn issue(&self, candidate: &str, token: &str, account: Option<&str>, now: i64) -> IssuedKey {
        let issued = IssuedKey {
            key: util::random_hex::<32>(),
            nonce: util::random_base64url::<32>(),
            token: token.to_owned(),
            at: now,
            account: account.filter(|a| !a.is_empty()).map(str::to_owned),
        };
        let mut keys = self.keys.lock().unwrap_or_else(PoisonError::into_inner);
        if !candidate.is_empty() {
            keys.by_candidate.insert(candidate.to_owned(), issued.clone());
        }
        let normalized = normalize_token(token);
        if !normalized.is_empty() {
            keys.by_token.insert(normalized, issued.clone());
        }
        keys.recent = Some(issued.clone());
        issued
    }

    /// Without a roster: candidate, token, else the most recent. With one: candidate then token, only roster
    /// accounts, never the fallback.
    pub fn lookup(&self, candidate: &str, token: &str, roster: Option<&[String]>, now: i64) -> KeyLookup {
        let mut keys = self.keys.lock().unwrap_or_else(PoisonError::into_inner);
        keys.by_candidate.retain(|_, k| live(k, now));
        keys.by_token.retain(|_, k| live(k, now));
        let normalized = normalize_token(token);
        let found: Vec<IssuedKey> = [
            (!candidate.is_empty())
                .then(|| keys.by_candidate.get(candidate).cloned())
                .flatten(),
            (!normalized.is_empty())
                .then(|| keys.by_token.get(&normalized).cloned())
                .flatten(),
        ]
        .into_iter()
        .flatten()
        .collect();
        let mut result = KeyLookup::default();
        match roster {
            Some(roster) => {
                for key in found {
                    let rostered = key.account.as_ref().is_some_and(|a| roster.iter().any(|r| r == a));
                    if rostered {
                        result.key = Some(key);
                        return result;
                    }
                    result.roster_rejected = true;
                }
                result.unknown_token = !normalized.is_empty() && !keys.by_token.contains_key(&normalized);
            }
            None => {
                if let Some(key) = found.into_iter().next() {
                    result.key = Some(key);
                    return result;
                }
                if let Some(recent) = keys.recent.clone().filter(|k| live(k, now)) {
                    result.unknown_token = !normalized.is_empty();
                    result.key = Some(recent);
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_normalize() {
        assert_eq!(
            normalize_token("  bearer eyJleader.payload.sig "),
            "eyJleader.payload.sig"
        );
        assert_eq!(normalize_token("BEARER eyJleader.payload.sig"), "eyJleader.payload.sig");
        assert_eq!(normalize_token("Bearer  abc "), "abc");
        assert_eq!(normalize_token("bearerxyz"), "bearerxyz");
        assert_eq!(fingerprint("eyJleader.payload.sig"), "ee5827e0de42");
        assert_eq!(fingerprint(""), "none");
        assert!(!"eyJleader.payload.sig".contains(&fingerprint("eyJleader.payload.sig")));
    }

    #[test]
    fn lookups_without_a_roster() {
        let keys = EncryptionKeys::default();
        let a = keys.issue("", "BEARER eyJleader.payload.sig", Some("acc-a"), 0);
        let b = keys.issue("", "token-b", Some("acc-b"), 0);
        assert_eq!(keys.lookup("", "eyJleader.payload.sig", None, 1).key, Some(a.clone()));
        assert_eq!(keys.lookup("", "  bearer eyJleader.payload.sig ", None, 1).key, Some(a));
        assert_eq!(keys.lookup("", "token-b", None, 1).key, Some(b.clone()));
        let c = keys.issue("cand", "token-c", Some("acc-c"), 2);
        assert_eq!(keys.lookup("cand", "token-b", None, 3).key, Some(c.clone()));
        let b2 = keys.issue("", "token-b", Some("acc-b"), 4);
        assert_eq!(keys.lookup("", "token-b", None, 5).key, Some(b2.clone()));
        let unknown = keys.lookup("", "nothing", None, 5);
        assert_eq!((unknown.key, unknown.unknown_token), (Some(b2), true));
        assert_eq!(keys.lookup("", "token-b", None, 5 + KEY_TTL_MS).key, None);
        let _ = c;
    }

    #[test]
    fn roster_lookups_serve_only_the_roster() {
        let keys = EncryptionKeys::default();
        let a = keys.issue("cand-a", "token-a", Some("acc-a"), 0);
        keys.issue("cand-x", "token-x", None, 0);
        let roster = vec!["acc-a".to_owned()];
        assert_eq!(keys.lookup("cand-a", "", Some(&roster), 1).key, Some(a.clone()));
        assert_eq!(keys.lookup("", "token-a", Some(&roster), 1).key, Some(a));
        let rejected = keys.lookup("cand-x", "token-x", Some(&roster), 1);
        assert_eq!((rejected.key, rejected.roster_rejected), (None, true));
        let unknown = keys.lookup("", "nothing", Some(&roster), 1);
        assert_eq!((unknown.key, unknown.unknown_token), (None, true));
        assert_eq!(keys.lookup("cand-a", "token-a", Some(&[]), 1).key, None);
    }
}
