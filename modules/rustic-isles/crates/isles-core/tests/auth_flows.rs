//! Node's auth tests that need a database, on a real SQLite file (RU5 auth): the one-time code flow
//! (`controllers/emailCodes.test.ts`), the registration sweep (`launcherEmailSweep.test.ts`), the last sign-in
//! (`lastSignIn.test.ts`), the active-account guard (`RequireActiveAccount.test.ts`), registration approval
//! (`registrationApproval.test.ts`) and the middleware's game server rules (`HasParadoxBackendAuth`).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::atomic::Ordering;

use axum::body::Body;
use axum::extract::Request;
use isles_core::areas::auth::crypto::{EMAIL_CODE_TTL_MS, generate_opaque_token, hash_email_code, hash_opaque_token};
use isles_core::areas::auth::data::{store, system_actor};
use isles_core::areas::auth::email::{
    EmailCodeIssue, UNVERIFIED_REGISTRATION_TTL_MS, accept_email_code, issue_email_code,
    sweep_unverified_registrations, take_code_send_slot,
};
use isles_core::areas::auth::launcher::{
    LAST_SIGN_IN_WRITE_INTERVAL_MS, create_session_for_user, note_sign_in, refresh_session,
};
use isles_core::areas::auth::middleware::{game_auth, require_active_account};
use isles_core::areas::auth::test_support::{TestServer, TestVersion};
use isles_core::areas::auth::util::{iso_from_ms, now_ms};
use isles_core::http::Call;
use isles_core::repo::auth::{EmailCodePurpose, EmailCodeRecord, LauncherAccountRecord, LauncherDeletion};
use isles_core::routes::{Method, Params, Service};
use serde_json::{Value, json};

fn code_value(code: &str) -> Value {
    Value::String(code.to_owned())
}

async fn issue_and_read(
    server: &TestServer,
    account: &LauncherAccountRecord,
    purpose: EmailCodePurpose,
    ip: &str,
) -> String {
    let address = account.email.clone().unwrap();
    let sent = issue_email_code(&server.shared, account, &address, purpose, ip)
        .await
        .unwrap();
    assert_eq!(sent, EmailCodeIssue::Sent, "the code was mailed");
    let code = server.mail.code_for(&address).unwrap();
    assert_eq!(code.len(), 6);
    code
}

async fn fresh_account(server: &TestServer, name: &str) -> (LauncherAccountRecord, String) {
    let account = server
        .add_launcher_account_with(
            name,
            Some(&format!("{}@example.test", name.to_lowercase())),
            None,
            |a| {
                a.username_set = Some(false);
            },
        )
        .await;
    let ip = server.fresh_ip();
    (account, ip)
}

fn now_iso() -> String {
    iso_from_ms(now_ms())
}

// ---------------------------------------------------------------------------------------------- the code flow

#[tokio::test(flavor = "multi_thread")]
async fn a_code_is_mailed_and_the_store_keeps_only_its_hmac() {
    let server = TestServer::new(&[]);
    let (account, ip) = fresh_account(&server, "flow1").await;
    let code = issue_and_read(&server, &account, EmailCodePurpose::Verify, &ip).await;
    let (id, purpose) = (account.account.clone(), EmailCodePurpose::Verify);
    let stored = store(&server.shared)
        .unwrap()
        .read(move |conn, repos| repos.email_codes.peek_live(conn, &id, purpose, &iso_from_ms(0), 5))
        .await
        .unwrap();
    // peek_live at the epoch is "not yet expired": the row is there
    let stored = stored.unwrap();
    let expected = hash_email_code(&server.shared.settings, account.user_id.as_str(), purpose, &code).unwrap();
    assert_eq!(stored.code_hash, expected);
    assert_ne!(stored.code_hash, code, "the code itself is not stored");
    assert_eq!(stored.attempts, 0);
    let lifetime = isles_core::areas::auth::util::parse_ms(&stored.expires_at).unwrap()
        - isles_core::areas::auth::util::parse_ms(&stored.created_at).unwrap();
    assert_eq!(lifetime, EMAIL_CODE_TTL_MS, "valid 5 minutes from issue");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_right_code_is_accepted_once_and_parallel_uses_succeed_once() {
    let server = TestServer::new(&[]);
    let (account, ip) = fresh_account(&server, "flow2").await;
    let code = issue_and_read(&server, &account, EmailCodePurpose::Verify, &ip).await;
    let results = futures_all(&server, &account, &code).await;
    assert_eq!(results.iter().filter(|accepted| **accepted).count(), 1, "{results:?}");
    assert!(
        !accept_email_code(
            &server.shared,
            &account,
            EmailCodePurpose::Verify,
            Some(&code_value(&code)),
            &now_iso()
        )
        .await
        .unwrap()
    );
}

async fn futures_all(server: &TestServer, account: &LauncherAccountRecord, code: &str) -> Vec<bool> {
    let mut tasks = Vec::new();
    for _ in 0..3 {
        let (shared, account, code) = (server.shared.clone(), account.clone(), code.to_owned());
        tasks.push(tokio::spawn(async move {
            accept_email_code(
                &shared,
                &account,
                EmailCodePurpose::Verify,
                Some(&code_value(&code)),
                &now_iso(),
            )
            .await
            .unwrap()
        }));
    }
    let mut results = Vec::new();
    for task in tasks {
        results.push(task.await.unwrap());
    }
    results
}

#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_code_costs_no_try_and_five_wrong_ones_use_the_code_up() {
    let server = TestServer::new(&[]);
    let (account, ip) = fresh_account(&server, "flow3").await;
    let code = issue_and_read(&server, &account, EmailCodePurpose::Verify, &ip).await;
    let accept = |value: Option<Value>| {
        let (shared, account) = (server.shared.clone(), account.clone());
        async move {
            accept_email_code(&shared, &account, EmailCodePurpose::Verify, value.as_ref(), &now_iso())
                .await
                .unwrap()
        }
    };
    for bad in [Some(json!("12345")), Some(json!("abcdef")), None, Some(json!(123_456))] {
        assert!(!accept(bad).await);
    }
    let wrong = if code == "000000" { "000001" } else { "000000" };
    for attempt in 1..=5 {
        assert!(!accept(Some(json!(wrong))).await, "wrong try {attempt}");
    }
    assert!(
        !accept(Some(json!(code))).await,
        "the sixth try, with the right code, is refused"
    );

    let (other, ip) = fresh_account(&server, "flow3b").await;
    let code = issue_and_read(&server, &other, EmailCodePurpose::Verify, &ip).await;
    let wrong = if code == "000000" { "000001" } else { "000000" };
    for _ in 0..4 {
        assert!(
            !accept_email_code(
                &server.shared,
                &other,
                EmailCodePurpose::Verify,
                Some(&json!(wrong)),
                &now_iso()
            )
            .await
            .unwrap()
        );
    }
    assert!(
        accept_email_code(
            &server.shared,
            &other,
            EmailCodePurpose::Verify,
            Some(&json!(code)),
            &now_iso()
        )
        .await
        .unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_code_expires_five_minutes_after_it_was_issued() {
    let server = TestServer::new(&[]);
    let (early, ip) = fresh_account(&server, "flow4").await;
    let code = issue_and_read(&server, &early, EmailCodePurpose::Verify, &ip).await;
    let just_before = iso_from_ms(now_ms() + 4 * 60_000 + 59_000);
    assert!(
        accept_email_code(
            &server.shared,
            &early,
            EmailCodePurpose::Verify,
            Some(&json!(code)),
            &just_before
        )
        .await
        .unwrap()
    );

    let (late, ip) = fresh_account(&server, "flow4b").await;
    let code = issue_and_read(&server, &late, EmailCodePurpose::Verify, &ip).await;
    let at_expiry = iso_from_ms(now_ms() + EMAIL_CODE_TTL_MS + 1000);
    assert!(
        !accept_email_code(
            &server.shared,
            &late,
            EmailCodePurpose::Verify,
            Some(&json!(code)),
            &at_expiry
        )
        .await
        .unwrap()
    );
    assert!(
        !accept_email_code(
            &server.shared,
            &late,
            EmailCodePurpose::Verify,
            Some(&json!(code)),
            &now_iso()
        )
        .await
        .unwrap(),
        "and it is gone: the next use is refused too"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_code_replaces_the_live_one_and_purposes_are_separate() {
    let server = TestServer::new(&[]);
    let (account, _) = fresh_account(&server, "flow5").await;
    let user = account.user_id.as_str().to_owned();
    for code in ["111111", "222222"] {
        let record = EmailCodeRecord {
            account: account.account.clone(),
            purpose: EmailCodePurpose::Verify,
            code_hash: hash_email_code(&server.shared.settings, &user, EmailCodePurpose::Verify, code).unwrap(),
            attempts: 0,
            created_at: now_iso(),
            expires_at: iso_from_ms(now_ms() + EMAIL_CODE_TTL_MS),
        };
        store(&server.shared)
            .unwrap()
            .account_write(
                &account.account,
                &isles_core::areas::auth::data::player_actor(&account.account),
                move |tx, repos| repos.email_codes.replace(tx, &record),
            )
            .await
            .unwrap();
    }
    let verify = |code: &'static str, purpose| {
        let (shared, account) = (server.shared.clone(), account.clone());
        async move {
            accept_email_code(&shared, &account, purpose, Some(&json!(code)), &now_iso())
                .await
                .unwrap()
        }
    };
    assert!(
        !verify("111111", EmailCodePurpose::Verify).await,
        "the replaced code is refused"
    );
    assert!(
        !verify("222222", EmailCodePurpose::Reset).await,
        "no reset code is live"
    );
    assert!(verify("222222", EmailCodePurpose::Verify).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_are_limited_a_minute_per_purpose_and_twenty_an_hour_per_address_and_a_failed_mail_uses_the_minute() {
    let server = TestServer::new(&[]);
    let (account, ip) = fresh_account(&server, "flow6").await;
    issue_and_read(&server, &account, EmailCodePurpose::Verify, &ip).await;
    let before = server.mail.count_to(account.email.as_deref().unwrap());
    let address = account.email.clone().unwrap();
    assert_eq!(
        issue_email_code(&server.shared, &account, &address, EmailCodePurpose::Verify, &ip)
            .await
            .unwrap(),
        EmailCodeIssue::Limited
    );
    assert_eq!(server.mail.count_to(&address), before, "nothing was mailed");
    assert_eq!(
        issue_email_code(&server.shared, &account, &address, EmailCodePurpose::Reset, &ip)
            .await
            .unwrap(),
        EmailCodeIssue::Sent,
        "the reset purpose has its own minute"
    );

    let shared_ip = "198.51.100.251";
    let results: Vec<bool> = (0..21)
        .map(|n| {
            take_code_send_slot(
                &server.shared,
                &format!("account-{n}"),
                EmailCodePurpose::Verify,
                shared_ip,
            )
        })
        .collect();
    assert_eq!(
        results.iter().filter(|ok| **ok).count(),
        20,
        "twenty an hour per address"
    );
    assert!(!results[20]);

    let (failing, ip) = fresh_account(&server, "flow6b").await;
    server.mail.fail.store(true, Ordering::Relaxed);
    let address = failing.email.clone().unwrap();
    assert_eq!(
        issue_email_code(&server.shared, &failing, &address, EmailCodePurpose::Verify, &ip)
            .await
            .unwrap(),
        EmailCodeIssue::Failed
    );
    server.mail.fail.store(false, Ordering::Relaxed);
    assert_eq!(
        issue_email_code(&server.shared, &failing, &address, EmailCodePurpose::Verify, &ip)
            .await
            .unwrap(),
        EmailCodeIssue::Limited,
        "the minute is used"
    );
}

// ---------------------------------------------------------------------------------------------- the sweep

#[tokio::test(flavor = "multi_thread")]
async fn the_sweep_removes_only_stale_unverified_registrations_with_their_codes_and_sessions() {
    let server = TestServer::new(&[]);
    let now = now_ms();
    let hour = 60 * 60 * 1000;
    let pending = |ago: i64| {
        move |a: &mut LauncherAccountRecord| a.email_verification_pending_since = Some(iso_from_ms(now - ago * hour))
    };
    let stale = server
        .add_launcher_account_with("Stale", Some("stale@example.test"), None, pending(49))
        .await;
    let fresh = server
        .add_launcher_account_with("Fresh", Some("fresh@example.test"), None, pending(1))
        .await;
    let verified = server
        .add_launcher_account_with("Verified", Some("verified@example.test"), None, |a| {
            a.email_verification_pending_since = Some(iso_from_ms(now - 49 * hour));
            a.email_verified_at = Some(iso_from_ms(now - 48 * hour));
        })
        .await;
    let existing = server
        .add_launcher_account_with("Existing", Some("existing@example.test"), None, |_| {})
        .await;
    let existing_old = server
        .add_launcher_account_with("ExistingOld", Some("old@example.test"), None, |a| {
            a.created_at = iso_from_ms(now - 400 * 24 * hour);
        })
        .await;
    // Codes and a session of the stale registration: both go with it
    let record = EmailCodeRecord {
        account: stale.account.clone(),
        purpose: EmailCodePurpose::Verify,
        code_hash: hash_email_code(
            &server.shared.settings,
            stale.user_id.as_str(),
            EmailCodePurpose::Verify,
            "123456",
        )
        .unwrap(),
        attempts: 0,
        created_at: iso_from_ms(now - 49 * hour),
        expires_at: iso_from_ms(now + 60_000),
    };
    store(&server.shared)
        .unwrap()
        .write(&system_actor(), move |tx, repos| repos.email_codes.replace(tx, &record))
        .await
        .unwrap();
    let (_, refresh) = create_session_for_user(&server.shared, &stale, "device", "PC")
        .await
        .unwrap();

    assert_eq!(
        sweep_unverified_registrations(&server.shared, now, 500).await.unwrap(),
        1,
        "only the stale registration"
    );
    let find = |user: &LauncherAccountRecord| {
        let user = user.user_id.clone();
        let shared = server.shared.clone();
        async move {
            isles_core::areas::auth::data::launcher_by_user(&shared, user.as_str())
                .await
                .unwrap()
        }
    };
    assert!(find(&stale).await.is_none(), "the stale registration is gone");
    for kept in [&fresh, &verified, &existing, &existing_old] {
        assert!(find(kept).await.is_some(), "{} stays", kept.display_name);
    }
    let hash = hash_opaque_token(&refresh);
    let session = store(&server.shared)
        .unwrap()
        .read(move |conn, repos| repos.refresh_sessions.find_by_token_hash(conn, &hash))
        .await
        .unwrap();
    assert!(session.is_none(), "the session of the removed account is gone");
    assert_eq!(
        sweep_unverified_registrations(&server.shared, now, 500).await.unwrap(),
        0,
        "a second sweep removes nothing"
    );
    assert!(
        isles_core::areas::auth::data::launcher_by_email(&server.shared, "stale@example.test")
            .await
            .unwrap()
            .is_none(),
        "the address is free"
    );
    server
        .add_launcher_account("Stale2", "stale@example.test", "password-123")
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_conditional_delete_keeps_an_account_verified_after_it_was_listed() {
    let server = TestServer::new(&[]);
    let now = now_ms();
    let pending = server
        .add_launcher_account_with("LateVerify", Some("late@example.test"), None, |a| {
            a.email_verification_pending_since = Some(iso_from_ms(now - 49 * 60 * 60 * 1000));
        })
        .await;
    let cutoff = iso_from_ms(now - UNVERIFIED_REGISTRATION_TTL_MS);
    let (id, verified_at) = (pending.account.clone(), iso_from_ms(now - 1000));
    store(&server.shared)
        .unwrap()
        .write(&system_actor(), move |tx, repos| {
            repos.launcher_accounts.set_email_verified(tx, &id, &verified_at)
        })
        .await
        .unwrap();
    let id = pending.account.clone();
    let removed = store(&server.shared)
        .unwrap()
        .write(&system_actor(), move |tx, repos| {
            repos.launcher_accounts.delete_unverified_registration(tx, &id, &cutoff)
        })
        .await
        .unwrap();
    assert!(!removed, "verified meanwhile: kept");
    assert!(
        isles_core::areas::auth::data::launcher_by_user(&server.shared, pending.user_id.as_str())
            .await
            .unwrap()
            .is_some()
    );
}

// ---------------------------------------------------------------------------------------------- last sign-in

#[tokio::test(flavor = "multi_thread")]
async fn note_sign_in_writes_a_stale_or_missing_date_and_skips_one_written_within_the_interval() {
    let server = TestServer::new(&[]);
    let now = now_ms();
    let stale = server
        .add_launcher_account_with("StaleUser", Some("s@example.test"), None, |a| {
            a.last_login_at = Some("2026-08-16T10:00:00.000Z".into())
        })
        .await;
    let written = note_sign_in(&server.shared, &stale, now).await.unwrap();
    assert_eq!(written.last_login_at, Some(iso_from_ms(now)));
    let stored = isles_core::areas::auth::data::launcher_by_user(&server.shared, stale.user_id.as_str())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.last_login_at, Some(iso_from_ms(now)));
    let soon = now + LAST_SIGN_IN_WRITE_INTERVAL_MS - 1000;
    let skipped = note_sign_in(&server.shared, &written, soon).await.unwrap();
    assert_eq!(
        skipped.last_login_at,
        Some(iso_from_ms(now)),
        "a write within the interval is skipped"
    );
    let never = server
        .add_launcher_account_with("NeverUser", Some("n@example.test"), None, |_| {})
        .await;
    assert_eq!(
        note_sign_in(&server.shared, &never, now).await.unwrap().last_login_at,
        Some(iso_from_ms(now))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restored_session_updates_the_last_sign_in() {
    let server = TestServer::new(&[]);
    let account = server
        .add_launcher_account_with("Refresher", Some("r@example.test"), None, |a| {
            a.last_login_at = Some("2026-08-16T10:00:00.000Z".into())
        })
        .await;
    let (_, refresh) = create_session_for_user(&server.shared, &account, "device-1", "Test PC")
        .await
        .unwrap();
    let before = now_ms();
    refresh_session(&server.shared, Some(&json!(refresh)), "device-1", "Test PC")
        .await
        .unwrap();
    let stored = isles_core::areas::auth::data::launcher_by_user(&server.shared, account.user_id.as_str())
        .await
        .unwrap()
        .unwrap();
    let written = isles_core::areas::auth::util::parse_ms(stored.last_login_at.as_deref().unwrap()).unwrap();
    assert!(written >= before - 1000, "the stored last sign-in is the refresh time");
}

// ---------------------------------------------------------------------------------------------- RequireActiveAccount

#[tokio::test(flavor = "multi_thread")]
async fn the_active_account_guard_refuses_the_unknown_the_deactivated_and_the_unconsented() {
    let server = TestServer::new(&[]);
    let scheduled = "2026-11-13T12:00:00.000Z";
    let unknown = require_active_account(&server.shared, "not-an-account", true)
        .await
        .unwrap_err();
    assert_eq!((unknown.code.as_str(), unknown.status()), ("AUTH_UNAUTHORIZED", 401));

    let deactivated = server
        .add_launcher_account_with("Deactivated", Some("d@example.test"), None, |a| {
            a.deletion = Some(LauncherDeletion {
                requested_at: "2026-10-14T12:00:00.000Z".into(),
                scheduled_for: scheduled.into(),
            });
        })
        .await;
    for consent in [true, false] {
        let refused = require_active_account(&server.shared, deactivated.user_id.as_str(), consent)
            .await
            .unwrap_err();
        assert_eq!((refused.code.as_str(), refused.status()), ("ACCOUNT_DEACTIVATED", 403));
        assert_eq!(refused.details, vec![("scheduledFor".to_owned(), json!(scheduled))]);
    }

    let plain = server
        .add_launcher_account_with("Plain", Some("p@example.test"), None, |_| {})
        .await;
    assert!(
        require_active_account(&server.shared, plain.user_id.as_str(), true)
            .await
            .is_ok(),
        "enforcement off by default"
    );
    let enforcing = TestServer::with_store_of(&server, &[("MYSTICPARADOX_CONSENT_ENFORCE", "on")]);
    let hosting = require_active_account(&enforcing.shared, plain.user_id.as_str(), true)
        .await
        .unwrap_err();
    assert_eq!((hosting.code.as_str(), hosting.status()), ("CONSENT_REQUIRED", 403));
    assert!(
        require_active_account(&enforcing.shared, plain.user_id.as_str(), false)
            .await
            .is_ok(),
        "joining is not gated by consent"
    );
    let consenting = server
        .add_launcher_account_with("Consenting", Some("c@example.test"), None, |a| {
            a.consent = Some(isles_core::repo::auth::LauncherConsent {
                terms_version: "2026-10-14".into(),
                privacy_version: "2026-10-17".into(),
                accepted_at: now_iso(),
            });
        })
        .await;
    assert!(
        require_active_account(&enforcing.shared, consenting.user_id.as_str(), true)
            .await
            .is_ok()
    );
}

// ---------------------------------------------------------------------------------------------- registration approval

#[tokio::test(flavor = "multi_thread")]
async fn registration_approval_decides_what_a_new_account_starts_with() {
    async fn register(server: &TestServer, name: &str) -> isles_core::areas::auth::test_support::Reply {
        let body = json!({"displayName": name, "email": format!("{}@example.test", name.to_lowercase()), "password": "correct horse battery"});
        server.call("POST", "/launcher/v1/auth/register", Some(body), &[]).await
    }
    let server = TestServer::new(&[]);
    let default = register(&server, "Default").await.json();
    assert_eq!(default["approvalRequired"], json!(false));
    assert_eq!(default["account"]["approvalStatus"], json!("approved"));
    let required_server = TestServer::with_store_of(&server, &[("MYSTICPARADOX_REGISTRATION_APPROVAL", "required")]);
    let required = register(&required_server, "Required").await.json();
    assert_eq!(required["approvalRequired"], json!(true));
    assert_eq!(required["account"]["approvalStatus"], json!("pending"));
    let login = required_server
        .call(
            "POST",
            "/launcher/v1/auth/login",
            Some(json!({"email": "required@example.test", "password": "correct horse battery"})),
            &[],
        )
        .await;
    assert_eq!(
        (login.status, login.json()["error"]["code"].clone()),
        (403, json!("AUTH_APPROVAL_PENDING"))
    );
    let off_server = TestServer::with_store_of(&server, &[("MYSTICPARADOX_REGISTRATION_APPROVAL", " OFF ")]);
    assert_eq!(
        register(&off_server, "Off").await.json()["approvalRequired"],
        json!(false)
    );
    let typo = TestServer::with_store_of(&server, &[("MYSTICPARADOX_REGISTRATION_APPROVAL", "requried")]);
    assert_eq!(
        register(&typo, "Typo").await.json()["account"]["approvalStatus"],
        json!("pending"),
        "an unknown value fails closed"
    );
}

// ---------------------------------------------------------------------------------------------- the game middleware

fn call(server: &TestServer, method: &str, uri: &str, headers: &[(&str, &str)]) -> Call {
    let route = isles_core::routes::all()
        .iter()
        .find(|r| r.service == Service::Backend && r.method == Method::Get && r.path == "/character/:userId")
        .or_else(|| isles_core::routes::all().iter().find(|r| r.service == Service::Backend))
        .unwrap();
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    Call {
        shared: server.shared.clone(),
        route,
        params: Params::default(),
        request: builder.body(Body::empty()).unwrap(),
        origin_host: None,
        request_id: "test-request".into(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_game_servers_read_of_the_invalid_sentinel_is_re_pointed_on_1147_only() {
    for (version, extra, want) in [
        (TestVersion::V1147, vec![], "/loadout/mystpax//all"),
        (
            TestVersion::V1147,
            vec![("GAMESERVER_NO_PLAYER_USER_ID", " some user ")],
            "/loadout/some%20user//all",
        ),
        (TestVersion::V1120, vec![], "/loadout/INVALID//all"),
    ] {
        let server = TestServer::for_version(version, &extra);
        server.gameserver_key("gs-key").await;
        let mut read = call(
            &server,
            "GET",
            "/loadout/INVALID//all?x=1",
            &[("x-mysticparadox-gameserver-apikey", "gs-key")],
        );
        game_auth(&mut read).await.unwrap();
        assert_eq!(read.request.uri().to_string(), format!("{want}?x=1"), "{version:?}");
        // Writes are never rewritten
        let mut write = call(
            &server,
            "POST",
            "/loadout/INVALID//all",
            &[("x-mysticparadox-gameserver-apikey", "gs-key")],
        );
        game_auth(&mut write).await.unwrap();
        assert_eq!(write.request.uri().path(), "/loadout/INVALID//all");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bearer_less_game_server_call_is_attributed_to_the_configured_account_on_1147_only() {
    let key = [("x-mysticparadox-gameserver-apikey", "gs-key")];
    let server = TestServer::new(&[("GAMESERVER_NO_PLAYER_USER_ID", "configured")]);
    server.gameserver_key("gs-key").await;
    let auth = game_auth(&mut call(&server, "GET", "/tags", &key)).await.unwrap();
    assert!(auth.is_gameserver);
    assert_eq!(auth.user_id.as_deref(), Some("configured"));
    let dev = TestServer::new(&[("DEV_USER_ID", "dev-account")]);
    dev.gameserver_key("gs-key").await;
    assert_eq!(
        game_auth(&mut call(&dev, "GET", "/tags", &key))
            .await
            .unwrap()
            .user_id
            .as_deref(),
        Some("dev-account")
    );
    let none = TestServer::new(&[]);
    none.gameserver_key("gs-key").await;
    assert_eq!(
        game_auth(&mut call(&none, "GET", "/tags", &key)).await.unwrap().user_id,
        None
    );
    let old = TestServer::for_version(TestVersion::V1120, &[("GAMESERVER_NO_PLAYER_USER_ID", "configured")]);
    old.gameserver_key("gs-key").await;
    assert_eq!(
        game_auth(&mut call(&old, "GET", "/tags", &key)).await.unwrap().user_id,
        None,
        "1.12 has no user for it"
    );
    // A wrong key is 401 whatever else is sent
    let refused = game_auth(&mut call(
        &none,
        "GET",
        "/tags",
        &[("x-mysticparadox-gameserver-apikey", "nope")],
    ))
    .await
    .unwrap_err();
    assert_eq!(refused.status(), 401);
    let _ = generate_opaque_token();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_banned_launcher_accounts_game_token_is_refused_with_403() {
    let server = TestServer::new(&[]);
    let banned = server
        .add_launcher_account_with("Banned", Some("b@example.test"), None, |a| {
            a.status = isles_core::repo::auth::OperationalStatus::Banned
        })
        .await;
    let token = server.game_token(banned.user_id.as_str());
    let header = format!("Bearer {token}");
    let refused = game_auth(&mut call(
        &server,
        "GET",
        "/tags",
        &[("authorization", header.as_str())],
    ))
    .await
    .unwrap_err();
    assert_eq!(refused.status(), 403);
    // A service or dev identity is not a launcher account and keeps its token behaviour
    let dev = server.game_token("__launcher_client_credentials__");
    let header = format!("Bearer {dev}");
    assert!(
        game_auth(&mut call(
            &server,
            "GET",
            "/tags",
            &[("authorization", header.as_str())]
        ))
        .await
        .is_ok()
    );
}
