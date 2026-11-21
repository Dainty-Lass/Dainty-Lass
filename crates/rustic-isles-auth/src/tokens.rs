// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! RS256 tokens (spec §5.1): the game token, the launcher access token and the store purchase token, each with
//! its own audience so one kind is never accepted as another.

use std::sync::Arc;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeyPair;
use aws_lc_rs::signature::{RSA_PKCS1_2048_8192_SHA256, RSA_PKCS1_SHA256, UnparsedPublicKey};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::{Map, Value};

pub const GAME_ISSUER: &str = "paradox-backend";
pub const GAME_AUDIENCE: &str = "paradox-backend";
pub const LAUNCHER_AUDIENCE: &str = "rustic-isles-launcher";
pub const STORE_ISSUER: &str = "rustic-isles-store";
pub const STORE_AUDIENCE: &str = "rustic-isles-store-purchase";

pub const GAME_TTL_SECONDS: i64 = 24 * 60 * 60;
pub const LAUNCHER_TTL_SECONDS: i64 = 12 * 60;
pub const STORE_TTL_SECONDS: i64 = 10 * 60;

const HEADER: &str = r#"{"alg":"RS256","typ":"JWT"}"#;
/// Bigger is not a token this crate signed (a 8192-bit signature is under 1.4 KB)
const MAX_TOKEN_LEN: usize = 16 * 1024;

/// Why a token could not be signed or validated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    /// The signing keys can't be read: `not base64`, `no RSA private key` or `no RSA public key`
    #[error("the token signing keys can't be read: {0}")]
    Keys(&'static str),
    /// The signer refused.
    #[error("the token could not be signed")]
    Sign,
    /// Malformed, wrongly signed, expired, or not this kind of token.
    #[error("invalid token")]
    Invalid,
}

/// A store purchase token's claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorePurchase {
    /// The buyer's userId.
    pub user_id: String,
    /// The character the purchase is for.
    pub character_id: String,
    /// The SKU bought.
    pub sku_id: String,
    /// The store the SKU was listed in, when the client named one.
    pub store_tag: Option<String>,
}

pub struct TokenKeys {
    private: KeyPair,
    public_der: Vec<u8>,
}

impl std::fmt::Debug for TokenKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenKeys(..)")
    }
}

impl TokenKeys {
    /// Reads `AUTH_SIGNING_PRIVKEY_B64` and `AUTH_SIGNING_PUBKEY_B64`: standard base64 of a PEM each.
    pub fn read(private_b64: &str, public_b64: &str) -> Result<Self, TokenError> {
        let private_pem = decode_pem_text(private_b64)?;
        let public_pem = decode_pem_text(public_b64)?;
        let private = pem_blocks(&private_pem)
            .into_iter()
            .find_map(|(label, der)| match label.as_str() {
                "PRIVATE KEY" => KeyPair::from_pkcs8(&der).ok(),
                "RSA PRIVATE KEY" => KeyPair::from_der(&der).ok(),
                _ => None,
            })
            .ok_or(TokenError::Keys("no RSA private key"))?;
        let public_der = pem_blocks(&public_pem)
            .into_iter()
            .find_map(|(label, der)| {
                let wanted = label == "PUBLIC KEY" || label == "RSA PUBLIC KEY";
                (wanted && aws_lc_rs::rsa::PublicKey::from_der(&der).is_ok()).then_some(der)
            })
            .ok_or(TokenError::Keys("no RSA public key"))?;
        Ok(Self { private, public_der })
    }

    fn sign(&self, claims: &Map<String, Value>) -> Result<String, TokenError> {
        let payload = dainty_service_kit::jsjson::stringify(&Value::Object(claims.clone()));
        let input = format!("{}.{}", URL_SAFE_NO_PAD.encode(HEADER), URL_SAFE_NO_PAD.encode(payload));
        let mut signature = vec![0u8; self.private.public_modulus_len()];
        self.private
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                input.as_bytes(),
                &mut signature,
            )
            .map_err(|_| TokenError::Sign)?;
        Ok(format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature)))
    }

    /// RS256 only, signature, `iss` and `aud` accepted, `exp` not passed and `nbf` reached at `now_s` (no leeway).
    fn verify(&self, token: &str, issuer: &str, audience: &str, now_s: i64) -> Result<Map<String, Value>, TokenError> {
        if token.len() > MAX_TOKEN_LEN {
            return Err(TokenError::Invalid);
        }
        let mut parts = token.split('.');
        let (Some(header), Some(payload), Some(signature), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(TokenError::Invalid);
        };
        let header: Value = decode_json(header)?;
        if header.get("alg").and_then(Value::as_str) != Some("RS256") {
            return Err(TokenError::Invalid);
        }
        let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| TokenError::Invalid)?;
        let input_len = token.len() - signature_len(token);
        UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, &self.public_der)
            .verify(&token.as_bytes()[..input_len], &signature)
            .map_err(|_| TokenError::Invalid)?;
        let Value::Object(claims) = decode_json(payload)? else {
            return Err(TokenError::Invalid);
        };
        if claims.get("iss").and_then(Value::as_str) != Some(issuer) {
            return Err(TokenError::Invalid);
        }
        let audience_ok = match claims.get("aud") {
            Some(Value::String(aud)) => aud == audience,
            Some(Value::Array(list)) => list.iter().any(|aud| aud.as_str() == Some(audience)),
            _ => false,
        };
        if !audience_ok {
            return Err(TokenError::Invalid);
        }
        let Some(exp) = claims.get("exp").and_then(Value::as_f64) else {
            return Err(TokenError::Invalid);
        };
        if now_s as f64 >= exp {
            return Err(TokenError::Invalid);
        }
        if let Some(nbf) = claims.get("nbf") {
            match nbf.as_f64() {
                Some(nbf) if nbf <= now_s as f64 => {}
                _ => return Err(TokenError::Invalid),
            }
        }
        Ok(claims)
    }
}

// The input is everything before the last dot
fn signature_len(token: &str) -> usize {
    token.rfind('.').map_or(token.len(), |dot| token.len() - dot)
}

fn decode_json(part: &str) -> Result<Value, TokenError> {
    let bytes = URL_SAFE_NO_PAD.decode(part).map_err(|_| TokenError::Invalid)?;
    serde_json::from_slice(&bytes).map_err(|_| TokenError::Invalid)
}

fn decode_pem_text(b64: &str) -> Result<String, TokenError> {
    let cleaned: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = STANDARD.decode(cleaned).map_err(|_| TokenError::Keys("not base64"))?;
    String::from_utf8(bytes).map_err(|_| TokenError::Keys("not base64"))
}

/// Every `-----BEGIN <label>-----` block with its decoded body.
fn pem_blocks(pem: &str) -> Vec<(String, Vec<u8>)> {
    let mut blocks = Vec::new();
    let mut rest = pem;
    while let Some(start) = rest.find("-----BEGIN ") {
        let after = &rest[start + 11..];
        let Some(label_end) = after.find("-----") else {
            break;
        };
        let label = after[..label_end].to_owned();
        let body_start = &after[label_end + 5..];
        let end_marker = format!("-----END {label}-----");
        let Some(body_end) = body_start.find(&end_marker) else {
            break;
        };
        let body: String = body_start[..body_end].chars().filter(|c| !c.is_whitespace()).collect();
        if let Ok(der) = STANDARD.decode(body) {
            blocks.push((label, der));
        }
        rest = &body_start[body_end + end_marker.len()..];
    }
    blocks
}

pub fn claims_base(user_id: &str) -> Map<String, Value> {
    let mut claims = Map::new();
    claims.insert("userId".into(), Value::String(user_id.to_owned()));
    claims
}

fn finish(mut claims: Map<String, Value>, now_s: i64, ttl: i64, audience: &str, issuer: &str) -> Map<String, Value> {
    claims.insert("iat".into(), Value::from(now_s));
    claims.insert("exp".into(), Value::from(now_s + ttl));
    claims.insert("aud".into(), Value::String(audience.to_owned()));
    claims.insert("iss".into(), Value::String(issuer.to_owned()));
    claims
}

pub fn sign_game(keys: &TokenKeys, user_id: &str, now_s: i64) -> Result<String, TokenError> {
    keys.sign(&finish(
        claims_base(user_id),
        now_s,
        GAME_TTL_SECONDS,
        GAME_AUDIENCE,
        GAME_ISSUER,
    ))
}

pub fn sign_launcher(keys: &TokenKeys, user_id: &str, sid: &str, now_s: i64) -> Result<String, TokenError> {
    let mut claims = claims_base(user_id);
    claims.insert("sid".into(), Value::String(sid.to_owned()));
    keys.sign(&finish(
        claims,
        now_s,
        LAUNCHER_TTL_SECONDS,
        LAUNCHER_AUDIENCE,
        GAME_ISSUER,
    ))
}

pub fn sign_store(keys: &TokenKeys, purchase: &StorePurchase, now_s: i64) -> Result<String, TokenError> {
    let mut claims = claims_base(&purchase.user_id);
    claims.insert("characterId".into(), Value::String(purchase.character_id.clone()));
    claims.insert("skuId".into(), Value::String(purchase.sku_id.clone()));
    if let Some(tag) = &purchase.store_tag {
        claims.insert("storeTag".into(), Value::String(tag.clone()));
    }
    keys.sign(&finish(claims, now_s, STORE_TTL_SECONDS, STORE_AUDIENCE, STORE_ISSUER))
}

pub fn validate_game(keys: &TokenKeys, token: &str, now_s: i64) -> Result<Map<String, Value>, TokenError> {
    keys.verify(token, GAME_ISSUER, GAME_AUDIENCE, now_s)
}

pub fn validate_launcher(keys: &TokenKeys, token: &str, now_s: i64) -> Result<(String, String), TokenError> {
    let claims = keys.verify(token, GAME_ISSUER, LAUNCHER_AUDIENCE, now_s)?;
    match (claims.get("userId"), claims.get("sid")) {
        (Some(Value::String(user)), Some(Value::String(sid))) => Ok((user.clone(), sid.clone())),
        _ => Err(TokenError::Invalid),
    }
}

pub fn validate_store(keys: &TokenKeys, token: &str, now_s: i64) -> Result<StorePurchase, TokenError> {
    let claims = keys.verify(token, STORE_ISSUER, STORE_AUDIENCE, now_s)?;
    let text = |name: &str| match claims.get(name) {
        Some(Value::String(s)) => Ok(s.clone()),
        _ => Err(TokenError::Invalid),
    };
    let store_tag = match claims.get("storeTag") {
        None => None,
        Some(Value::String(tag)) => Some(tag.clone()),
        Some(_) => return Err(TokenError::Invalid),
    };
    Ok(StorePurchase {
        user_id: text("userId")?,
        character_id: text("characterId")?,
        sku_id: text("skuId")?,
        store_tag,
    })
}

/// The parsed pair, kept until either setting changes (parsed once per distinct pair).
#[derive(Default)]
pub struct KeyCache {
    cached: std::sync::Mutex<Option<(String, String, Arc<TokenKeys>)>>,
}

impl KeyCache {
    pub fn get(&self, private_b64: &str, public_b64: &str) -> Result<Arc<TokenKeys>, TokenError> {
        let mut cached = self.cached.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((private, public, keys)) = cached.as_ref()
            && private == private_b64
            && public == public_b64
        {
            return Ok(Arc::clone(keys));
        }
        let keys = Arc::new(TokenKeys::read(private_b64, public_b64)?);
        *cached = Some((private_b64.to_owned(), public_b64.to_owned(), Arc::clone(&keys)));
        Ok(keys)
    }
}
