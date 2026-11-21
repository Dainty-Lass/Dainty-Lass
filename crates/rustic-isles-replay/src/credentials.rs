// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Per-side credentials, minted tokens and placeholder substitution (spec §4).

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeyPair;
use aws_lc_rs::signature::RSA_PKCS1_SHA256;
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use regex::{Captures, Regex};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{ReplayError, pattern};

/// The subject a bare `<token>` is minted for (§3.5; the stale host doc names another, §15 item 1).
pub const ANON_SUBJECT: &str = "UID-replay-anon";
/// The HS256 secret of a side with neither a signing key nor a secret (§15 item 6).
pub const DEFAULT_HS256_SECRET: &str = "replay-default-jwt-secret-key-32b";
/// 2100-01-01: a replayed token never expires during a run (§15 item 5).
const EXP_FLOOR_S: i64 = 4_102_444_800;
const TTL_S: i64 = 86_400;

/// One side's credentials (§4.1). `Debug` says which secrets are set, never their values (D2).
#[derive(Clone, Default)]
pub struct ReplayTargetAuth {
    /// HS256 secret for minted tokens; a fixed default when none.
    pub jwt_secret: Option<String>,
    /// Standard base64 of a PEM RSA private key (`AUTH_SIGNING_PRIVKEY_B64`), PKCS#1 or PKCS#8
    pub signing_key_b64: Option<String>,
    /// Game-server key that fills `<gskey>`.
    pub gs_key: Option<String>,
    /// Sent as `x-gateway-secret` on every request when set (§6.2).
    pub gateway_secret: Option<String>,
    /// SHA-256 hex of a raw account key → the key
    pub key_map: BTreeMap<String, String>,
    /// Fill an unmapped `<key:...>` with `DLK_replay_` and the hash's first 24 characters instead of leaving it
    /// inexact.
    pub deterministic_fallback_keys: bool,
}

impl std::fmt::Debug for ReplayTargetAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayTargetAuth")
            .field("jwt_secret", &self.jwt_secret.is_some())
            .field("signing_key_b64", &self.signing_key_b64.is_some())
            .field("gs_key", &self.gs_key.is_some())
            .field("gateway_secret", &self.gateway_secret.is_some())
            .field("key_map_entries", &self.key_map.len())
            .field("deterministic_fallback_keys", &self.deterministic_fallback_keys)
            .finish()
    }
}

impl ReplayTargetAuth {
    /// A token for `subject` at `now_ms`: RS256 with the signing key, else HS256 (§4.2).
    pub fn mint_token(&self, subject: &str, now_ms: i64) -> Result<String, ReplayError> {
        match &self.signing_key_b64 {
            Some(key) => mint_rs256_jwt(subject, key, now_ms),
            None => Ok(mint_hs256_jwt(
                subject,
                self.jwt_secret.as_deref().unwrap_or(DEFAULT_HS256_SECRET),
                now_ms,
            )),
        }
    }

    /// Maps the key's SHA-256 hex to it and answers the hex (§4.4).
    pub fn register_account_key(&mut self, raw: &str) -> String {
        let hex = sha256_hex(raw.as_bytes());
        self.key_map.insert(hex.clone(), raw.to_owned());
        hex
    }

    /// Auth's throwaway RS256 key, `gs_key`, no HS256 or gateway secret, fallback keys on (§11.2).
    #[cfg(feature = "test-support")]
    pub fn with_test_keys(gs_key: impl Into<String>) -> Self {
        let (private, _) = rustic_isles_auth::test_support::throwaway_signing_keys();
        Self {
            signing_key_b64: Some(private),
            gs_key: Some(gs_key.into()),
            deterministic_fallback_keys: true,
            ..Self::default()
        }
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn times(now_ms: i64) -> (i64, i64) {
    let iat = now_ms.div_euclid(1000);
    (iat, iat.saturating_add(TTL_S).max(EXP_FLOOR_S))
}

fn encode_part(value: &Value) -> String {
    URL_SAFE_NO_PAD.encode(dainty_service_kit::jsjson::stringify(value))
}

/// The HS256 token of §4.2 (claims `userId`, `sub`, `iat`, `exp`, `iss`, `aud`).
pub fn mint_hs256_jwt(subject: &str, secret: &str, now_ms: i64) -> String {
    let (iat, exp) = times(now_ms);
    let mut claims = Map::new();
    claims.insert("userId".into(), subject.into());
    claims.insert("sub".into(), subject.into());
    claims.insert("iat".into(), iat.into());
    claims.insert("exp".into(), exp.into());
    claims.insert("iss".into(), "rusticisles".into());
    claims.insert("aud".into(), "rustic-isles-game".into());
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#),
        encode_part(&Value::Object(claims))
    );
    let key = aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256, secret.as_bytes());
    let tag = aws_lc_rs::hmac::sign(&key, input.as_bytes());
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(tag.as_ref()))
}

// Parsing an RSA key is costly and a run mints a token per placeholder, so the last key read is kept, by its hash
type CachedKey = Option<(String, Arc<KeyPair>)>;
static KEY_CACHE: LazyLock<Mutex<CachedKey>> = LazyLock::new(|| Mutex::new(None));

fn token_error(message: String) -> ReplayError {
    ReplayError::Token(message)
}

fn read_signing_key(privkey_b64: &str) -> Result<Arc<KeyPair>, ReplayError> {
    let id = sha256_hex(privkey_b64.as_bytes());
    let mut cache = KEY_CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((cached, pair)) = cache.as_ref()
        && *cached == id
    {
        return Ok(Arc::clone(pair));
    }
    let cleaned: String = privkey_b64.chars().filter(|c| !c.is_whitespace()).collect();
    let pem = STANDARD
        .decode(cleaned)
        .map_err(|e| token_error(format!("invalid base64 signing key: {e}")))?;
    let pem = String::from_utf8_lossy(&pem);
    let pair = pem_private_key(&pem).map_err(|m| token_error(format!("invalid RSA PEM signing key: {m}")))?;
    let pair = Arc::new(pair);
    *cache = Some((id, Arc::clone(&pair)));
    Ok(pair)
}

fn pem_private_key(pem: &str) -> Result<KeyPair, String> {
    let mut rest = pem;
    let mut last_error = String::from("no PRIVATE KEY or RSA PRIVATE KEY block");
    while let Some(start) = rest.find("-----BEGIN ") {
        let after = &rest[start + "-----BEGIN ".len()..];
        let Some(label_len) = after.find("-----") else {
            break;
        };
        let label = &after[..label_len];
        let body = &after[label_len + 5..];
        let end = format!("-----END {label}-----");
        let Some(body_len) = body.find(&end) else {
            break;
        };
        let text: String = body[..body_len].chars().filter(|c| !c.is_whitespace()).collect();
        rest = &body[body_len + end.len()..];
        let parse = match label {
            "PRIVATE KEY" => KeyPair::from_pkcs8,
            "RSA PRIVATE KEY" => KeyPair::from_der,
            _ => continue,
        };
        match STANDARD.decode(text) {
            Ok(der) => match parse(&der) {
                Ok(pair) => return Ok(pair),
                Err(e) => last_error = e.to_string(),
            },
            Err(e) => last_error = e.to_string(),
        }
    }
    Err(last_error)
}

/// The RS256 game account token of auth §8 (claims `userId`, `iat`, `exp`, `aud`, `iss`).
pub fn mint_rs256_jwt(subject: &str, privkey_b64: &str, now_ms: i64) -> Result<String, ReplayError> {
    let pair = read_signing_key(privkey_b64)?;
    let (iat, exp) = times(now_ms);
    let mut claims = Map::new();
    claims.insert("userId".into(), subject.into());
    claims.insert("iat".into(), iat.into());
    claims.insert("exp".into(), exp.into());
    claims.insert("aud".into(), "paradox-backend".into());
    claims.insert("iss".into(), "paradox-backend".into());
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","typ":"JWT"}"#),
        encode_part(&Value::Object(claims))
    );
    let mut signature = vec![0u8; pair.public_modulus_len()];
    pair.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        input.as_bytes(),
        &mut signature,
    )
    .map_err(|e| token_error(format!("RS256 signing failed: {e}")))?;
    Ok(format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

/// A substituted value and whether it is only an approximation of what the client sent (§4.3).
#[derive(Debug, Clone, PartialEq)]
pub struct Substituted<T> {
    /// The substituted value.
    pub value: T,
    /// Whether a placeholder could not be filled exactly.
    pub inexact: bool,
}

static TOKEN_SUBJECT: LazyLock<Regex> = LazyLock::new(|| pattern(r"<token:([^>]+)>"));
static KEY: LazyLock<Regex> = LazyLock::new(|| pattern(r"<key:([0-9A-Fa-f]{64})>"));
static PLAIN: LazyLock<Regex> = LazyLock::new(|| pattern(r"<plain:([^>]+)>"));

/// §4.3's six steps, each on the result of the previous.
pub fn substitute_str(text: &str, auth: &ReplayTargetAuth, now_ms: i64) -> Substituted<String> {
    let mut inexact = text.contains("<redacted>");
    let mut out = text.to_owned();

    if out.contains("<token:") {
        out = TOKEN_SUBJECT
            .replace_all(&out, |c: &Captures| match auth.mint_token(&c[1], now_ms) {
                Ok(token) => token,
                Err(_) => {
                    inexact = true;
                    c[0].to_owned()
                }
            })
            .into_owned();
    }
    if out.contains("<token>") {
        match auth.mint_token(ANON_SUBJECT, now_ms) {
            Ok(token) => out = out.replace("<token>", &token),
            Err(_) => inexact = true,
        }
    }
    if out.contains("<key:") {
        out = KEY
            .replace_all(&out, |c: &Captures| {
                let hex = c[1].to_ascii_lowercase();
                if let Some(raw) = auth.key_map.get(&hex) {
                    raw.clone()
                } else if auth.deterministic_fallback_keys {
                    format!("DLK_replay_{}", &hex[..24])
                } else {
                    inexact = true;
                    c[0].to_owned()
                }
            })
            .into_owned();
    }
    if out.contains("<gskey>") {
        match &auth.gs_key {
            Some(key) => out = out.replace("<gskey>", key),
            None => inexact = true,
        }
    }
    if out.contains("<plain:") {
        out = PLAIN
            .replace_all(&out, |c: &Captures| match auth.mint_token(&c[1], now_ms) {
                Ok(token) => STANDARD.encode(format!("\0{}\0{token}", &c[1])),
                Err(_) => {
                    inexact = true;
                    c[0].to_owned()
                }
            })
            .into_owned();
    }
    Substituted { value: out, inexact }
}

/// Every string of a JSON value (not object keys), recursively.
pub fn substitute_value(value: &Value, auth: &ReplayTargetAuth, now_ms: i64) -> Substituted<Value> {
    let mut inexact = false;
    let value = substitute_walk(value, auth, now_ms, &mut inexact);
    Substituted { value, inexact }
}

fn substitute_walk(value: &Value, auth: &ReplayTargetAuth, now_ms: i64, inexact: &mut bool) -> Value {
    match value {
        Value::String(s) => {
            let sub = substitute_str(s, auth, now_ms);
            *inexact |= sub.inexact;
            Value::String(sub.value)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| substitute_walk(v, auth, now_ms, inexact))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), substitute_walk(v, auth, now_ms, inexact)))
                .collect(),
        ),
        other => other.clone(),
    }
}
