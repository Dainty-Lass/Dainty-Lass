//! The auth area wired into the Reforged API (RU5): the launcher authentication it registers, the registration flow
//! that writes the game account and the launcher account together, the admin sign-in's checks, and the periodic
//! erasure. All on a real SQLite file through the real pipeline.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use isles_core::areas::auth::crypto::hash_password;
use isles_core::areas::auth::test_support::TestServer;
use isles_core::areas::auth::util::{iso_from_ms, now_ms};
use isles_core::areas::auth::wiring::{self, AdminSignIn};
use isles_core::areas::reforged_api::admin::AdminCredentials;
use isles_core::areas::reforged_api::launcher_auth::has_launcher_auth;
use isles_core::repo::auth::LauncherDeletion;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn the_registered_authentication_admits_a_launcher_session_and_refuses_the_rest() {
    let server = TestServer::new(&[]);
    wiring::register_seams();
    let account = server
        .add_launcher_account("Seamer", "seamer@example.test", "password-123")
        .await;
    let (access, _) = server.launcher_session(&account).await;
    let caller = has_launcher_auth(&server.shared, Some(&format!("Bearer {access}")))
        .await
        .unwrap();
    assert_eq!(caller.account, account.account);
    assert_eq!(caller.user_id, account.user_id.to_string());
    assert_eq!(caller.record.display_name, "Seamer");
    let bad = has_launcher_auth(&server.shared, Some("Bearer not-a-token"))
        .await
        .unwrap_err();
    assert_eq!(
        (bad.status, bad.code.as_str()),
        (401, "AUTH_UNAUTHORIZED"),
        "{}",
        bad.message
    );
    assert_eq!(bad.message, "Your session has expired. Please sign in again.");
}

/// reforged-api's `Register` writes the game account, then the launcher account, then the key. The launcher account's
/// create must not insert the game account a second time.
#[tokio::test(flavor = "multi_thread")]
async fn registering_through_the_reforged_api_writes_each_row_once() {
    let server = TestServer::new(&[("REGISTRATION_MODE", "OPEN")]);
    let register = server
        .call(
            "POST",
            "/reforged/api/Register",
            Some(json!({ "Username": "Newcomer" })),
            &[],
        )
        .await;
    assert_eq!(register.status, 200, "{}", register.body);
    let key = register.json()["UUK"].as_str().unwrap().to_owned();
    assert!(key.starts_with("RRK_"));
    let again = server
        .call(
            "POST",
            "/reforged/api/Register",
            Some(json!({ "Username": "newcomer" })),
            &[],
        )
        .await;
    assert_eq!(again.status, 409, "the name is taken, whatever its case");

    let (launchers, accounts, keys) = server
        .store()
        .read(|conn, repos| {
            let launcher = repos
                .launcher_accounts
                .find_by_display_name_normalized(conn, "newcomer")?;
            let account = match &launcher {
                Some(l) => repos.accounts.find(conn, &l.account)?,
                None => None,
            };
            let keys = repos.api_keys.find_all_user_key_hashes(conn)?;
            Ok((launcher, account, keys))
        })
        .await
        .unwrap();
    let launcher = launchers.expect("the launcher account");
    let account = accounts.expect("the game account");
    assert_eq!(account.user_id, launcher.user_id);
    assert_eq!(account.name, "Newcomer");
    assert_eq!(keys.iter().filter(|k| k.account == launcher.account).count(), 1);

    // The key the player was given finds the account (one hash on both sides)
    let info = server
        .call(
            "GET",
            "/reforged/api/GetUserInfo",
            None,
            &[("x-reforged-user-api-key", key.as_str())],
        )
        .await;
    assert_eq!(info.status, 200, "{}", info.body);
}

#[test]
fn the_admin_sign_in_checks_the_password_hash_and_the_authenticator_code() {
    let hash = hash_password("a long password").unwrap();
    assert!(AdminSignIn.verify_password(Some(&hash), "a long password"));
    assert!(!AdminSignIn.verify_password(Some(&hash), "another one"));
    assert!(!AdminSignIn.verify_password(None, "a long password"));
    assert!(!AdminSignIn.verify_totp("GEZDGNBVGY3TQOJQ", Some(&json!(123_456))));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_erasure_job_erases_what_is_due_once() {
    let server = TestServer::new(&[]);
    let due = server
        .add_launcher_account_with("Leaving", Some("leaving@example.test"), Some("password-123"), |a| {
            a.deletion = Some(LauncherDeletion {
                requested_at: "2026-09-01T00:00:00.000Z".into(),
                scheduled_for: "2026-10-01T00:00:00.000Z".into(),
            });
        })
        .await;
    let held = server
        .add_launcher_account_with("Holding", Some("holding@example.test"), Some("password-123"), |a| {
            a.deletion = Some(LauncherDeletion {
                requested_at: iso_from_ms(now_ms()),
                scheduled_for: iso_from_ms(now_ms() + 30 * 86_400_000),
            });
        })
        .await;
    let now = iso_from_ms(now_ms());
    assert_eq!(wiring::run_erasures(&server.shared, &now).await.unwrap(), 1);
    assert_eq!(wiring::run_erasures(&server.shared, &now).await.unwrap(), 0);
    let (gone, kept) = server
        .store()
        .read(move |conn, repos| {
            Ok((
                repos.launcher_accounts.find_by_account(conn, &due.account)?,
                repos.launcher_accounts.find_by_account(conn, &held.account)?,
            ))
        })
        .await
        .unwrap();
    assert!(gone.is_none());
    assert!(kept.is_some());
}
