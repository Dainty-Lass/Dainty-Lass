// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Tokens, keys, codes, the background jobs and start-up, from spec §12.3.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{GS_KEY, Harness};
use rustic_isles_auth::test_support::{MapSettings, MemoryStore};
use rustic_isles_auth::{Auth, AuthConfig, AuthStore, Deletion, ErasureOutcome, Seams, StartError, StorePurchase};
use serde_json::json;

#[tokio::test]
async fn tokens_keep_their_kinds_apart() {
    let t = Harness::new().await;
    let game = t.auth.sign_game_token("u1").unwrap();
    let claims = t.auth.validate_game_token(&game).unwrap();
    assert_eq!(claims["aud"], json!("paradox-backend"));
    assert_eq!(claims["userId"], json!("u1"));
    let keys: Vec<&String> = claims.keys().collect();
    assert_eq!(keys, ["userId", "iat", "exp", "aud", "iss"]);
    let launcher = t.auth.sign_launcher_token("u1", "fam").unwrap();
    assert_eq!(
        t.auth.validate_launcher_token(&launcher).unwrap(),
        ("u1".into(), "fam".into())
    );
    assert!(t.auth.validate_game_token(&launcher).is_err());
    assert!(t.auth.validate_launcher_token(&game).is_err());
    assert!(t.auth.validate_game_token("garbage").is_err());
    let purchase = StorePurchase {
        user_id: "u1".into(),
        character_id: "c1".into(),
        sku_id: "sku".into(),
        store_tag: Some("tag".into()),
    };
    let store = t.auth.sign_store_token(&purchase).unwrap();
    assert_eq!(t.auth.validate_store_token(&store).unwrap(), purchase);
    assert!(t.auth.validate_game_token(&store).is_err());
    assert!(t.auth.validate_launcher_token(&store).is_err());
    assert!(t.auth.validate_store_token(&game).is_err());
    t.settings.set("AUTH_SIGNING_PRIVKEY_B64", "@@@");
    assert!(matches!(
        t.auth.sign_game_token("u1"),
        Err(rustic_isles_auth::TokenError::Keys("not base64"))
    ));
}

#[tokio::test]
async fn gameserver_keys_synchronize() {
    let t = Harness::new().await;
    assert_eq!(t.store().gameserver_hashes().len(), 1);
    assert_eq!(t.auth.synchronize_gameserver_keys(Some(" , ")).await.unwrap(), 0);
    assert_eq!(
        t.store().gameserver_hashes().len(),
        1,
        "a blank configuration keeps the stored keys"
    );
    assert_eq!(t.auth.synchronize_gameserver_keys(Some("gs-key-2")).await.unwrap(), 1);
    let stored = t.store().gameserver_hashes();
    assert_eq!(stored.len(), 1);
    assert!(stored[0].len() == 64 && stored[0].bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(
        t.get("/tags", &[("x-rustic-isles-gameserver-apikey", GS_KEY)])
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        t.get("/tags", &[("x-rustic-isles-gameserver-key", "gs-key-2")])
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn email_codes_are_single_use_and_counted() {
    let t = Harness::new().await;
    t.smtp_on();
    t.settings.set("RUSTIC_ISLES_EMAIL_VERIFICATION", "required");
    t.register(Some("Slayer1"), "slayer1@example.test").await;
    let code = t.mail.last_code("slayer1@example.test").unwrap();
    let verify = |code: &str| {
        let body = json!({"email": "slayer1@example.test", "password": "correct horse", "code": code});
        async { t.post("/launcher/v1/auth/email/verify", &[], body).await }
    };
    let wrong = if code == "000000" { "111111" } else { "000000" };
    for _ in 0..4 {
        assert_eq!(verify(wrong).await.status, StatusCode::BAD_REQUEST);
    }
    assert_eq!(t.store().email_codes()[0].attempts, 4);
    assert_eq!(verify("12ab").await.status, StatusCode::BAD_REQUEST);
    assert_eq!(t.store().email_codes()[0].attempts, 4, "a malformed code costs no try");
    t.advance(4 * 60_000 + 59_000);
    assert_eq!(
        verify(&code).await.status,
        StatusCode::OK,
        "four wrong then right, at 4 min 59 s"
    );
}

#[tokio::test]
async fn five_wrong_tries_use_a_code_up_and_codes_expire() {
    let t = Harness::new().await;
    t.smtp_on();
    t.settings.set("RUSTIC_ISLES_EMAIL_VERIFICATION", "required");
    t.register(Some("Slayer1"), "slayer1@example.test").await;
    let code = t.mail.last_code("slayer1@example.test").unwrap();
    let verify = |code: &str| {
        let body = json!({"email": "slayer1@example.test", "password": "correct horse", "code": code});
        async { t.post("/launcher/v1/auth/email/verify", &[], body).await }
    };
    let wrong = if code == "000000" { "111111" } else { "000000" };
    for _ in 0..5 {
        verify(wrong).await;
    }
    assert!(t.store().email_codes().is_empty());
    assert_eq!(verify(&code).await.status, StatusCode::BAD_REQUEST);

    t.advance(61_000);
    assert_eq!(
        t.login("slayer1@example.test", "correct horse").await.code(),
        "AUTH_EMAIL_UNVERIFIED"
    );
    let fresh = t.mail.last_code("slayer1@example.test").unwrap();
    t.advance(5 * 60_000);
    assert_eq!(
        verify(&fresh).await.status,
        StatusCode::BAD_REQUEST,
        "refused after 5 minutes"
    );
    assert!(t.store().email_codes().is_empty(), "and gone");
}

#[tokio::test]
async fn mail_slots() {
    let t = Harness::new().await;
    t.smtp_on();
    t.settings.set("RUSTIC_ISLES_EMAIL_VERIFICATION", "required");
    t.register(Some("Slayer1"), "slayer1@example.test").await;
    assert_eq!(t.mail.messages().len(), 1);
    t.login("slayer1@example.test", "correct horse").await;
    assert_eq!(t.mail.messages().len(), 1, "one per minute per purpose");
    t.post(
        "/launcher/v1/auth/password/forgot",
        &[],
        json!({"email": "slayer1@example.test"}),
    )
    .await;
    assert!(t.mail.wait_for(2).await, "reset has its own minute");
    t.mail.fail(true);
    t.advance(61_000);
    t.login("slayer1@example.test", "correct horse").await;
    t.mail.fail(false);
    t.login("slayer1@example.test", "correct horse").await;
    assert_eq!(t.mail.messages().len(), 2, "a failed mail uses the minute");
}

#[tokio::test]
async fn the_sweep_removes_stale_unverified_registrations() {
    let t = Harness::new().await;
    t.smtp_on();
    t.settings.set("RUSTIC_ISLES_EMAIL_VERIFICATION", "required");
    t.register(Some("Stale1"), "stale@example.test").await;
    let kept = t.register(Some("Kept1"), "kept@example.test").await.json()["account"]["userId"]
        .as_str()
        .unwrap()
        .to_owned();
    t.store()
        .update_account(&kept, |a| a.email_verified_at = Some("2026-01-01T00:00:00.000Z".into()));
    t.advance(49 * 3600 * 1000);
    t.register(Some("Fresh1"), "fresh@example.test").await;
    assert_eq!(t.auth.sweep_unverified_registrations(t.now(), 500).await.unwrap(), 1);
    assert!(
        t.store()
            .launcher_account_by_email("stale@example.test")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        t.store()
            .launcher_account_by_email("kept@example.test")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        t.store()
            .launcher_account_by_email("fresh@example.test")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        t.register(Some("Stale1"), "stale@example.test").await.status,
        StatusCode::ACCEPTED
    );
}

#[tokio::test]
async fn erasure() {
    let t = Harness::new().await;
    let (_, _, user_id) = t.account("Slayer1", "slayer1@example.test").await;
    let (_, _, other) = t.account("Slayer2", "slayer2@example.test").await;
    let account = t.store().launcher_account_by_user_id(&user_id).await.unwrap().unwrap();
    let hash = rustic_isles_auth::hashes::user_key_hash(common::SECRET, "RRK_gone");
    t.store().insert_user_key(&account.account_id, &hash);
    let early = Deletion {
        requested_at: rustic_isles_auth::iso(t.now()),
        scheduled_for: rustic_isles_auth::iso(t.now() + 1000),
    };
    t.store().update_account(&user_id, |a| a.deletion = Some(early));
    let account = t.store().launcher_account_by_user_id(&user_id).await.unwrap().unwrap();
    assert_eq!(
        t.auth.erase_account(&account, true).await.unwrap_err().code,
        "DELETION_HOLD_EXPIRED"
    );
    t.advance(2000);
    assert_eq!(
        t.auth
            .due_deletions(&rustic_isles_auth::iso(t.now()), 0)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        t.auth.erase_account(&account, false).await.unwrap(),
        ErasureOutcome::DryRun
    );
    let ErasureOutcome::Erased(counts) = t.auth.erase_account(&account, true).await.unwrap() else {
        panic!("not erased");
    };
    assert_eq!((counts.user_api_keys, counts.accounts), (1, 1));
    assert_eq!(
        serde_json::to_value(&counts)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        [
            "emailCodes",
            "gameExchangeCodes",
            "discordOAuthTransactions",
            "authIdentities",
            "userApiKeys",
            "friendships",
            "otherAreas",
            "accounts"
        ]
    );
    assert!(t.store().launcher_account_by_user_id(&user_id).await.unwrap().is_none());
    assert!(t.store().game_account_by_user_id(&user_id).await.unwrap().is_none());
    assert_eq!(
        t.get("/launcher/v1/me", &[("x-reforged-user-api-key", "RRK_gone")])
            .await
            .message(),
        "Sign in required."
    );
    assert!(t.store().launcher_account_by_user_id(&other).await.unwrap().is_some());
    assert_eq!(
        t.register(Some("Slayer1"), "slayer1@example.test").await.status,
        StatusCode::ACCEPTED
    );
}

struct Busy;
impl rustic_isles_auth::Erasure for Busy {
    fn blockers<'a>(
        &'a self,
        _account: &'a rustic_isles_auth::LauncherAccount,
    ) -> module_api::BoxFuture<'a, Result<Vec<String>, String>> {
        Box::pin(async { Ok(vec!["online".to_owned()]) })
    }
}

#[tokio::test]
async fn blockers_keep_the_account() {
    let (private, public) = rustic_isles_auth::test_support::throwaway_signing_keys();
    let settings = Arc::new(MapSettings::new(&[
        ("API_KEY_HASH_SECRET", common::SECRET),
        ("AUTH_SIGNING_PRIVKEY_B64", &private),
        ("AUTH_SIGNING_PUBKEY_B64", &public),
    ]));
    let seams = Seams {
        erasure: Arc::new(Busy),
        ..Seams::default()
    };
    let auth = Auth::new(
        AuthConfig::for_1_14_7(module_api::NodeIdentity::standalone()),
        settings,
        Some(MemoryStore::new()),
        seams,
    );
    let mut account = rustic_isles_auth::LauncherAccount {
        account_id: module_api::GlobalId::new(
            module_api::IdKind::Account,
            &module_api::NodeIdentity::standalone().node,
        )
        .to_string(),
        user_id: "u1".into(),
        email: None,
        display_name: "Fox".into(),
        display_name_normalized: "fox".into(),
        password_hash: None,
        status: rustic_isles_auth::Status::Active,
        approval: None,
        roles: vec![],
        roles_updated_at: None,
        created_at: "2026-01-01T00:00:00.000Z".into(),
        last_login_at: None,
        username_set: None,
        consent: None,
        deletion: None,
        email_verified_at: None,
        email_verification_pending_since: None,
        timeout: None,
    };
    assert_eq!(auth.erase_account(&account, true).await.unwrap_err().code, "INTERNAL");
    account.deletion = Some(Deletion {
        requested_at: "2026-01-01T00:00:00.000Z".into(),
        scheduled_for: "2026-01-31T00:00:00.000Z".into(),
    });
    assert_eq!(
        auth.erase_account(&account, true).await.unwrap(),
        ErasureOutcome::Blocked(vec!["online".into()])
    );
}

#[tokio::test]
async fn start_up_refuses_bad_secrets_and_missing_smtp() {
    let (private, public) = rustic_isles_auth::test_support::throwaway_signing_keys();
    let make = |pairs: &[(&str, &str)]| {
        let settings = Arc::new(MapSettings::new(pairs));
        Auth::new(
            AuthConfig::for_1_14_7(module_api::NodeIdentity::standalone()),
            settings,
            Some(MemoryStore::new()),
            Seams::default(),
        )
    };
    let short = make(&[("API_KEY_HASH_SECRET", "short")]);
    assert_eq!(
        short.start().await.unwrap_err().to_string(),
        "API_KEY_HASH_SECRET must contain at least 32 characters."
    );
    let no_keys = make(&[("API_KEY_HASH_SECRET", common::SECRET)]);
    assert!(matches!(no_keys.start().await.unwrap_err(), StartError::Keys(_)));
    let smtp = make(&[
        ("API_KEY_HASH_SECRET", common::SECRET),
        ("AUTH_SIGNING_PRIVKEY_B64", &private),
        ("AUTH_SIGNING_PUBKEY_B64", &public),
        ("RUSTIC_ISLES_EMAIL_VERIFICATION", "required"),
        ("RUSTIC_ISLES_SMTP_HOST", "smtp.example.test"),
    ]);
    assert_eq!(
        smtp.start().await.unwrap_err().to_string(),
        "RUSTIC_ISLES_EMAIL_VERIFICATION=required needs working SMTP settings (RUSTIC_ISLES_SMTP_USER, \
         RUSTIC_ISLES_SMTP_PASSWORD not set or invalid; the required keys are RUSTIC_ISLES_SMTP_HOST, \
         RUSTIC_ISLES_SMTP_USER and RUSTIC_ISLES_SMTP_PASSWORD)"
    );
    let without_store: Arc<Auth<MemoryStore>> = Auth::new(
        AuthConfig::for_1_14_7(module_api::NodeIdentity::standalone()),
        Arc::new(MapSettings::default()),
        None,
        Seams::default(),
    );
    assert!(without_store.start().await.unwrap().is_empty());
}

#[tokio::test]
async fn an_arc_dyn_store_is_a_store() {
    let store: Arc<dyn AuthStore> = Arc::new(MemoryStore::new());
    let auth = Auth::new(
        AuthConfig::for_1_14_7(module_api::NodeIdentity::standalone()),
        Arc::new(MapSettings::default()),
        Some(store),
        Seams::default(),
    );
    assert!(auth.store_ref().unwrap().health().await.is_ok());
}

#[test]
fn every_route_is_listed_once() {
    let routes = rustic_isles_auth::routes();
    assert_eq!(routes.len(), 49);
    let mut seen = std::collections::HashSet::new();
    for route in routes {
        assert!(
            seen.insert((route.method.clone(), route.path)),
            "{} {}",
            route.method,
            route.path
        );
    }
}
