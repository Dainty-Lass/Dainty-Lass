// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The `AuthStore` adapter (spec §12): row U1 of §15.2.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{player, system, targets};
use module_api::NodeId;
use rustic_isles_auth::{
    AuthStore, Channel, CreateOutcome, EmailCode, ErasureCounts, ExchangeCode, GameAccount, Identity, LauncherAccount,
    NewLauncherAccount, Purpose, RefreshSession, Status,
};
use rustic_isles_store::{AccountId, IslesAuthStore};

fn launcher(id: &str, user: &str, email: &str, name: &str) -> LauncherAccount {
    LauncherAccount {
        account_id: id.into(),
        user_id: user.into(),
        email: Some(email.into()),
        display_name: name.into(),
        display_name_normalized: name.to_lowercase(),
        password_hash: Some("hash".into()),
        status: Status::Active,
        approval: None,
        roles: vec![],
        roles_updated_at: None,
        created_at: "2026-10-01T00:00:00.000Z".into(),
        last_login_at: None,
        username_set: Some(false),
        consent: None,
        deletion: None,
        email_verified_at: None,
        email_verification_pending_since: Some("2026-10-01T00:00:00.000Z".into()),
        timeout: None,
    }
}

fn session(id: &str, account: &str, family: &str) -> RefreshSession {
    RefreshSession {
        id: id.into(),
        token_hash: format!("t-{id}"),
        account_id: account.into(),
        family_id: family.into(),
        device_id: "d".into(),
        device_name: "pc".into(),
        created_at: "2026-10-01T00:00:00.000Z".into(),
        expires_at: "2026-11-01T00:00:00.000Z".into(),
        revoked_at: None,
    }
}

#[tokio::test]
async fn u1_the_adapter_follows_the_store_rules() {
    for t in targets() {
        let auth = IslesAuthStore::new(t.store.clone());
        let id = AccountId::new(t.store.node());
        let text = id.to_string();
        let me = player(&id);

        let new = NewLauncherAccount {
            account: launcher(&text, "u1", "a@x.org", "Ash"),
            game_name: "AshGame".into(),
        };
        assert!(matches!(
            auth.create_launcher_account(&system(), &new).await.unwrap(),
            CreateOutcome::Created(_)
        ));
        assert_eq!(
            auth.create_launcher_account(&system(), &new).await.unwrap(),
            CreateOutcome::EmailTaken
        );
        let game = auth.game_account_by_id(&text).await.unwrap().unwrap();
        assert_eq!(game.name, "AshGame");
        assert_eq!(auth.game_account_by_id("not an id").await.unwrap(), None);
        assert_eq!(
            auth.launcher_account_by_email("a@x.org")
                .await
                .unwrap()
                .map(|a| a.account_id),
            Some(text.clone())
        );
        assert!(
            !auth
                .ensure_game_account(
                    &system(),
                    &GameAccount {
                        id: text.clone(),
                        user_id: "u1".into(),
                        name: "x".into(),
                        notes: 0
                    }
                )
                .await
                .unwrap()
        );

        // Player writes of a node-local account succeed; another node's account is refused
        auth.set_last_login(&me, &text, "2026-10-02T00:00:00.000Z")
            .await
            .unwrap();
        assert!(auth.set_username(&me, &text, "Slayer", "slayer").await.unwrap());
        let foreign = AccountId::new(&NodeId::new("germany-1").unwrap());
        let err = auth
            .set_last_login(&player(&foreign), &foreign.to_string(), "x")
            .await
            .unwrap_err();
        assert!(err.message.contains("could not be resolved"), "{err}");
        assert!(auth.set_last_login(&system(), "garbage", "x").await.is_err());

        // Sessions and their counts
        auth.create_session(&me, &session("s1", &text, "f1")).await.unwrap();
        auth.rotate_session(&me, "s1", "2026-10-02T00:00:00.000Z", &session("s2", &text, "f1"))
            .await
            .unwrap();
        auth.create_session(&me, &session("s3", &text, "f2")).await.unwrap();
        assert_eq!(
            auth.revoke_family(&me, "f1", "2026-10-03T00:00:00.000Z").await.unwrap(),
            1
        );
        assert!(
            auth.is_family_active("f2", &text, "2026-10-03T00:00:00.000Z")
                .await
                .unwrap()
        );
        assert_eq!(
            auth.revoke_all_sessions(&me, &text, "2026-10-03T00:00:00.000Z")
                .await
                .unwrap(),
            1
        );
        assert!(
            auth.session_by_token_hash("t-s1")
                .await
                .unwrap()
                .unwrap()
                .revoked_at
                .is_some()
        );

        // Email code tries: the fifth removes the code and still answers 5
        let code = EmailCode {
            account_id: text.clone(),
            purpose: Purpose::Verify,
            code_hash: "h".into(),
            attempts: 3,
            created_at: "2026-10-01T00:00:00.000Z".into(),
            expires_at: "2026-11-01T00:00:00.000Z".into(),
        };
        auth.replace_email_code(&me, &code).await.unwrap();
        let now = "2026-10-02T00:00:00.000Z";
        assert_eq!(
            auth.count_email_code_try(&me, &text, Purpose::Verify, now, 5)
                .await
                .unwrap(),
            Some(4)
        );
        assert_eq!(
            auth.count_email_code_try(&me, &text, Purpose::Verify, now, 5)
                .await
                .unwrap(),
            Some(5)
        );
        assert_eq!(auth.live_email_code(&text, Purpose::Verify, now).await.unwrap(), None);
        assert_eq!(
            auth.count_email_code_try(&me, &text, Purpose::Verify, now, 5)
                .await
                .unwrap(),
            None
        );
        auth.replace_email_code(
            &me,
            &EmailCode {
                attempts: 0,
                ..code.clone()
            },
        )
        .await
        .unwrap();
        assert!(
            auth.consume_email_code(&me, &text, Purpose::Verify, "h", now)
                .await
                .unwrap()
        );

        // Exchange codes, Discord, identities, keys
        let exchange = ExchangeCode {
            code_hash: "ec".into(),
            account_id: text.clone(),
            launcher_session_id: "ls".into(),
            build_changelist: 7,
            executable_sha256: "e".into(),
            runtime_channel: Channel::Stable,
            runtime_sha256: "r".into(),
            runtime_manifest_version: "m".into(),
            runtime_artifact_set_sha256: "a".into(),
            realm_id: None,
            created_at: "c".into(),
            expires_at: "2026-11-01T00:00:00.000Z".into(),
            consumed_at: None,
        };
        assert!(
            auth.create_exchange_code(
                &me,
                &ExchangeCode {
                    build_changelist: -1,
                    ..exchange.clone()
                }
            )
            .await
            .is_err()
        );
        auth.create_exchange_code(&me, &exchange).await.unwrap();
        assert_eq!(
            auth.consume_exchange_code(&me, "ec", now)
                .await
                .unwrap()
                .unwrap()
                .build_changelist,
            7
        );
        assert!(auth.consume_exchange_code(&me, "ec", now).await.unwrap().is_none());
        let identity = Identity {
            provider_subject: "d1".into(),
            account_id: text.clone(),
            provider_username: "ash".into(),
            provider_avatar_url: None,
            linked_at: "l".into(),
        };
        assert!(auth.create_identity(&me, &identity).await.unwrap());
        assert_eq!(auth.identity_by_account(&text).await.unwrap(), Some(identity));
        auth.replace_gameserver_keys(&system(), &["k1".into()]).await.unwrap();
        assert_eq!(auth.gameserver_key_hashes().await.unwrap(), ["k1"]);
        auth.health().await.unwrap();

        // The erasure's counts, in order
        let counts = auth.erase_account(&me, &text).await.unwrap();
        assert_eq!(
            counts,
            ErasureCounts {
                email_codes: 0,
                game_exchange_codes: 1,
                discord_oauth_transactions: 0,
                auth_identities: 1,
                user_api_keys: 0,
                friendships: 0,
                other_areas: 0,
                accounts: 1,
            }
        );
        assert_eq!(auth.launcher_account_by_id(&text).await.unwrap(), None);
        assert!(auth.session_by_token_hash("t-s1").await.unwrap().is_none());

        // An unverified registration with no cut-off
        let other = AccountId::new(t.store.node());
        let pending = NewLauncherAccount {
            account: launcher(&other.to_string(), "u2", "b@x.org", "Bee"),
            game_name: "Bee".into(),
        };
        auth.create_launcher_account(&system(), &pending).await.unwrap();
        assert!(
            auth.delete_unverified_registration(&system(), &other.to_string())
                .await
                .unwrap()
        );
        assert!(
            !auth
                .delete_unverified_registration(&system(), &other.to_string())
                .await
                .unwrap()
        );
    }
}
