// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Substitution and minted tokens (spec §13.2 S1-S10).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use rustic_isles_auth::test_support::{MapSettings, MemoryStore, throwaway_signing_keys};
use rustic_isles_auth::{Auth, AuthConfig, Seams};
use rustic_isles_replay::{
    IdNormalizer, NormalizerConfig, ReplayTargetAuth, mint_hs256_jwt, mint_rs256_jwt, normalize_jwts_in_str,
    substitute_str, substitute_value,
};
use serde_json::{Value, json};

const NOW: i64 = 1_792_688_400_000;
const RAW_KEY: &str = "RRK_00112233445566778899aabbccddeeff0011223344556677";

fn hs_side() -> (ReplayTargetAuth, String) {
    let mut auth = ReplayTargetAuth {
        jwt_secret: Some("an-hs256-secret-for-the-tests-0001".into()),
        gs_key: Some("gs-k".into()),
        ..ReplayTargetAuth::default()
    };
    let hex = auth.register_account_key(RAW_KEY);
    (auth, hex)
}

fn claims(token: &str) -> Value {
    let part = token.split('.').nth(1).unwrap();
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
}

#[test]
fn s1_s2_tokens() {
    let (auth, _) = hs_side();
    let s1 = substitute_str("Bearer <token:UID-alice>", &auth, NOW);
    assert!(!s1.inexact);
    assert!(s1.value.starts_with("Bearer eyJ"));
    assert_eq!(normalize_jwts_in_str(&s1.value), "Bearer <jwt:UID-alice>");
    let s2 = substitute_str("Bearer <token> <token>", &auth, NOW);
    assert!(!s2.inexact);
    assert_eq!(
        normalize_jwts_in_str(&s2.value),
        "Bearer <jwt:UID-replay-anon> <jwt:UID-replay-anon>"
    );
}

#[test]
fn s3_s4_keys() {
    let (mut auth, hex) = hs_side();
    let s3 = substitute_str(&format!("code=<key:{}>", hex.to_ascii_uppercase()), &auth, NOW);
    assert_eq!(
        (s3.value.as_str(), s3.inexact),
        (format!("code={RAW_KEY}").as_str(), false)
    );
    let unknown = format!("<key:{}>", "f".repeat(64));
    let off = substitute_str(&unknown, &auth, NOW);
    assert_eq!((off.value.as_str(), off.inexact), (unknown.as_str(), true));
    auth.deterministic_fallback_keys = true;
    let on = substitute_str(&unknown, &auth, NOW);
    assert_eq!(
        (on.value.as_str(), on.inexact),
        ("RRK_replay_ffffffffffffffffffffffff", false)
    );
}

#[test]
fn s5_game_server_key() {
    let (mut auth, _) = hs_side();
    let with = substitute_str("<gskey>", &auth, NOW);
    assert_eq!((with.value.as_str(), with.inexact), ("gs-k", false));
    auth.gs_key = None;
    let without = substitute_str("<gskey>", &auth, NOW);
    assert_eq!((without.value.as_str(), without.inexact), ("<gskey>", true));
}

#[test]
fn s6_sasl_plain() {
    let (auth, _) = hs_side();
    let s6 = substitute_str("<auth><plain:UID-alice></auth>", &auth, NOW);
    assert!(!s6.inexact);
    let b64 = s6
        .value
        .strip_prefix("<auth>")
        .unwrap()
        .strip_suffix("</auth>")
        .unwrap();
    let decoded = String::from_utf8(STANDARD.decode(b64).unwrap()).unwrap();
    let parts: Vec<&str> = decoded.split('\0').collect();
    assert_eq!(parts.len(), 3);
    assert_eq!((parts[0], parts[1]), ("", "UID-alice"));
    assert_eq!(normalize_jwts_in_str(parts[2]), "<jwt:UID-alice>");
}

#[test]
fn s7_redacted_json() {
    let (auth, _) = hs_side();
    let sub = substitute_value(
        &json!({"secret": "<redacted>", "n": 1, "list": ["<gskey>"]}),
        &auth,
        NOW,
    );
    assert!(sub.inexact);
    assert_eq!(sub.value, json!({"secret": "<redacted>", "n": 1, "list": ["gs-k"]}));
    let keys = substitute_value(&json!({"<gskey>": "x"}), &auth, NOW);
    assert_eq!(keys.value, json!({"<gskey>": "x"}));
}

#[tokio::test]
async fn s8_rs256_is_accepted_by_the_game_token_check() {
    let auth = ReplayTargetAuth::with_test_keys("gs-k");
    let now = chrono_now_ms();
    let s8 = substitute_str("Bearer <token:UID-bob>", &auth, now);
    assert!(!s8.inexact);
    assert_eq!(normalize_jwts_in_str(&s8.value), "Bearer <jwt:UID-bob>");

    let (private, public) = throwaway_signing_keys();
    let settings = MapSettings::new(&[
        ("AUTH_SIGNING_PRIVKEY_B64", &private),
        ("AUTH_SIGNING_PUBKEY_B64", &public),
    ]);
    let target = Auth::<MemoryStore>::new(
        AuthConfig::for_1_14_7(module_api::NodeIdentity::standalone()),
        Arc::new(settings),
        None,
        Seams::default(),
    );
    let token = s8.value.strip_prefix("Bearer ").unwrap();
    let claims = target.validate_game_token(token).unwrap();
    assert_eq!(claims["userId"], "UID-bob");
    // A token minted for a capture's old time is still accepted (exp floor, §15 item 5)
    let old = auth.mint_token("UID-bob", 1_600_000_000_000).unwrap();
    assert!(target.validate_game_token(&old).is_ok());
}

fn chrono_now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

#[test]
fn s8_pkcs1_keys_are_read_too() {
    // The PKCS#8 throwaway key's inner RSAPrivateKey, re-wrapped as an `RSA PRIVATE KEY` PEM
    let (private, _) = throwaway_signing_keys();
    let pem = String::from_utf8(STANDARD.decode(private).unwrap()).unwrap();
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    let der = STANDARD.decode(body).unwrap();
    let pkcs1 = pkcs8_inner(&der);
    let pkcs1_pem = format!(
        "-----BEGIN RSA PRIVATE KEY-----\n{}\n-----END RSA PRIVATE KEY-----\n",
        STANDARD.encode(pkcs1)
    );
    let token = mint_rs256_jwt("UID-c", &STANDARD.encode(pkcs1_pem), NOW).unwrap();
    assert_eq!(normalize_jwts_in_str(&token), "<jwt:UID-c>");
}

/// The OCTET STRING privateKey of a PKCS#8 PrivateKeyInfo (version, algorithm, privateKey).
fn pkcs8_inner(der: &[u8]) -> Vec<u8> {
    fn header(der: &[u8], at: usize) -> (usize, usize) {
        let first = der[at + 1] as usize;
        if first < 0x80 {
            return (at + 2, first);
        }
        let n = first & 0x7f;
        let len = der[at + 2..at + 2 + n]
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | *b as usize);
        (at + 2 + n, len)
    }
    let (mut at, _) = header(der, 0);
    for _ in 0..2 {
        let (start, len) = header(der, at);
        at = start + len;
    }
    let (start, len) = header(der, at);
    der[start..start + len].to_vec()
}

#[test]
fn s9_bad_signing_key_is_inexact_not_an_error() {
    let auth = ReplayTargetAuth {
        signing_key_b64: Some("not base64".into()),
        ..ReplayTargetAuth::default()
    };
    let s9 = substitute_str("Bearer <token:UID-a> <plain:UID-a> <token>", &auth, NOW);
    assert!(s9.inexact);
    assert_eq!(s9.value, "Bearer <token:UID-a> <plain:UID-a> <token>");
    let error = mint_rs256_jwt("UID-a", "not base64", NOW).unwrap_err().to_string();
    assert!(
        error.starts_with("token signing error: invalid base64 signing key: "),
        "{error}"
    );
    let error = mint_rs256_jwt("UID-a", &STANDARD.encode("no pem here"), NOW)
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("token signing error: invalid RSA PEM signing key: "),
        "{error}"
    );
}

#[test]
fn s10_times() {
    let token = mint_hs256_jwt("UID-a", "s", NOW);
    let c = claims(&token);
    assert_eq!(
        (c["iat"].as_i64(), c["exp"].as_i64()),
        (Some(1_792_688_400), Some(4_102_444_800))
    );
    assert_eq!(
        c,
        json!({"userId": "UID-a", "sub": "UID-a", "iat": 1_792_688_400, "exp": 4_102_444_800_i64,
            "iss": "rusticisles", "aud": "rustic-isles-game"})
    );
    assert_eq!(
        String::from_utf8(URL_SAFE_NO_PAD.decode(token.split('.').next().unwrap()).unwrap()).unwrap(),
        r#"{"alg":"HS256","typ":"JWT"}"#
    );
    // Floor division: a negative time rounds down
    assert_eq!(claims(&mint_hs256_jwt("u", "s", -1))["iat"], -1);
    let late = claims(&mint_hs256_jwt("u", "s", 5_000_000_000_000));
    assert_eq!(late["exp"], 5_000_000_000_i64 + 86_400);
    let rs = claims(&ReplayTargetAuth::with_test_keys("g").mint_token("UID-r", NOW).unwrap());
    assert_eq!(
        rs,
        json!({"userId": "UID-r", "iat": 1_792_688_400, "exp": 4_102_444_800_i64,
            "aud": "paradox-backend", "iss": "paradox-backend"})
    );
}

#[test]
fn n1_hs256_and_rs256_normalize_alike() {
    let hs = mint_hs256_jwt("UID-player1", "s", NOW);
    let rs = ReplayTargetAuth::with_test_keys("g")
        .mint_token("UID-player1", NOW)
        .unwrap();
    let mut left = IdNormalizer::new(NormalizerConfig::default());
    let mut right = IdNormalizer::new(NormalizerConfig::default());
    assert_eq!(left.normalize_str(&hs, None), "<jwt:UID-player1>");
    assert_eq!(right.normalize_str(&rs, None), "<jwt:UID-player1>");
}

#[test]
fn debug_hides_credentials() {
    let (auth, _) = hs_side();
    let shown = format!("{:?}", ReplayTargetAuth::with_test_keys("gs-secret-value"));
    assert!(!shown.contains("gs-secret-value") && !shown.contains("LS0t"), "{shown}");
    let shown = format!("{auth:?}");
    assert!(
        !shown.contains("an-hs256-secret") && !shown.contains(RAW_KEY) && !shown.contains("gs-k"),
        "{shown}"
    );
}
