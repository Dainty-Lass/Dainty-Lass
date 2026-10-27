//! The auth repositories on both engines (DD15): `repositories.accounts.test.ts` and `repositories.emailCodes.test.ts` for
//! launcher accounts, sessions, exchange codes, OAuth transactions, email codes and API keys, through `store`'s write path
//! and its change log. The Postgres half runs when `REFORGED_TEST_POSTGRES_URL` names a server
//! (`postgres://user:pw@127.0.0.1:5432/postgres`); it skips otherwise.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::Arc;

use diesel::Connection as _;
use diesel::PgConnection;
use diesel::connection::SimpleConnection;
use isles_core::db::IslesStore;
use isles_core::repo::auth::{
    AccessChange, AdminAccountFilter, ApprovalStatus, AuthIdentityRecord, CreateOutcome, DiscordOAuthTransactionRecord,
    EmailCodePurpose, EmailCodeRecord, GameExchangeCodeRecord, LauncherAccountRecord, LauncherConsent,
    LauncherDeletion, OperationalStatus, RefreshSessionRecord, RuntimeChannel,
};
use isles_core::{AccountId, UserId};
use module_api::network::StandaloneHome;
use module_api::{Actor, ActorContext, NodeIdentity};
use store::{DatabaseConfig, DatabaseOptions, PostgresConfig};

const T0: &str = "2026-10-01T00:00:00.000Z";
const T1: &str = "2026-10-02T00:00:00.000Z";
const T2: &str = "2026-10-03T00:00:00.000Z";

fn actor() -> ActorContext {
    ActorContext::new(Actor::Admin { key: "owner".into() })
}

fn open(config: &DatabaseConfig) -> IslesStore {
    let node = NodeIdentity::standalone();
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    IslesStore::open(config, &DatabaseOptions::default(), home, &node).unwrap()
}

fn launcher(name: &str, email: Option<&str>, created: &str) -> LauncherAccountRecord {
    LauncherAccountRecord {
        account: AccountId::new(&NodeIdentity::standalone().node),
        user_id: UserId::random(),
        email: email.map(str::to_owned),
        display_name_normalized: name.to_lowercase(),
        display_name: name.to_owned(),
        password_hash: Some("$argon2id$hash".into()),
        status: OperationalStatus::Active,
        approval_status: Some(ApprovalStatus::Approved),
        approval_updated_at: None,
        approval_updated_by: None,
        approval_reason: None,
        roles: vec!["player".into()],
        roles_updated_at: None,
        roles_updated_by: None,
        created_at: created.to_owned(),
        last_login_at: None,
        username_set: Some(true),
        consent: None,
        deletion: None,
        email_verified_at: None,
        email_verification_pending_since: None,
    }
}

async fn create(store: &IslesStore, record: &LauncherAccountRecord) -> CreateOutcome {
    let record = record.clone();
    store
        .write(&actor(), move |tx, repos| repos.launcher_accounts.create(tx, &record))
        .await
        .unwrap()
}

async fn by_user(store: &IslesStore, record: &LauncherAccountRecord) -> Option<LauncherAccountRecord> {
    let user = record.user_id.clone();
    store
        .read(move |conn, repos| repos.launcher_accounts.find_by_user_id(conn, &user))
        .await
        .unwrap()
}

async fn launcher_accounts(store: IslesStore) {
    let slayer = launcher("Slayer", Some("slayer@example.test"), T0);
    assert_eq!(create(&store, &slayer).await, CreateOutcome::Created);
    // The record round trips, the game account is made beside it, and both are found by the client's id
    assert_eq!(by_user(&store, &slayer).await, Some(slayer.clone()));
    let user = slayer.user_id.clone();
    let game = store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (game.id.clone(), game.name.as_str(), game.notes),
        (slayer.account.clone(), "Slayer", 0)
    );

    // Unique among live rows: the address and the name, in any case
    let same_email = launcher("Other", Some("slayer@example.test"), T0);
    assert_eq!(create(&store, &same_email).await, CreateOutcome::EmailTaken);
    let same_name = launcher("SLAYER", Some("other@example.test"), T0);
    assert_eq!(create(&store, &same_name).await, CreateOutcome::DisplayNameTaken);
    assert!(
        by_user(&store, &same_name).await.is_none(),
        "a refused create writes nothing"
    );
    let user = same_name.user_id.clone();
    assert!(
        store
            .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
            .await
            .unwrap()
            .is_none()
    );
    let no_email = launcher("Discorder", None, T1);
    assert_eq!(create(&store, &no_email).await, CreateOutcome::Created);
    let another = launcher("Discorder2", None, T1);
    assert_eq!(
        create(&store, &another).await,
        CreateOutcome::Created,
        "accounts without an address do not collide"
    );

    let id = slayer.account.clone();
    let found = store
        .read(move |conn, repos| {
            Ok((
                repos.launcher_accounts.find_by_email(conn, "slayer@example.test")?,
                repos
                    .launcher_accounts
                    .find_by_display_name_normalized(conn, "slayer")?,
                repos.launcher_accounts.find_by_account(conn, &id)?,
                repos.launcher_accounts.find_by_email(conn, "nobody@example.test")?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(found.0.as_ref(), Some(&slayer));
    assert_eq!(found.1.as_ref(), Some(&slayer));
    assert_eq!(found.2.as_ref(), Some(&slayer));
    assert_eq!(found.3, None);

    // Writes: last sign-in, consent, deletion, verification, password, username
    let id = slayer.account.clone();
    let consent = LauncherConsent {
        terms_version: "2026-10-14".into(),
        privacy_version: "2026-10-17".into(),
        accepted_at: T1.into(),
    };
    let deletion = LauncherDeletion {
        requested_at: T1.into(),
        scheduled_for: T2.into(),
    };
    let (c, d) = (consent.clone(), deletion.clone());
    store
        .write(&actor(), move |tx, repos| {
            repos.launcher_accounts.update_last_login(tx, &id, T1)?;
            repos.launcher_accounts.set_consent(tx, &id, &c)?;
            repos.launcher_accounts.set_deletion(tx, &id, Some(&d))?;
            repos.launcher_accounts.set_password_hash(tx, &id, "$argon2id$new")?;
            repos.launcher_accounts.set_email_verified(tx, &id, T1)
        })
        .await
        .unwrap();
    let after = by_user(&store, &slayer).await.unwrap();
    assert_eq!(after.last_login_at.as_deref(), Some(T1));
    assert_eq!(after.consent, Some(consent));
    assert_eq!(after.deletion, Some(deletion));
    assert_eq!(
        (after.password_hash.as_deref(), after.email_verified_at.as_deref()),
        (Some("$argon2id$new"), Some(T1))
    );
    let now = T2.to_owned();
    let due = store
        .read(move |conn, repos| repos.launcher_accounts.list_deletions_due(conn, &now, 10))
        .await
        .unwrap();
    assert_eq!(due.len(), 1);
    let id = slayer.account.clone();
    store
        .write(&actor(), move |tx, repos| {
            repos.launcher_accounts.set_deletion(tx, &id, None)
        })
        .await
        .unwrap();
    assert_eq!(by_user(&store, &slayer).await.unwrap().deletion, None);

    let id = slayer.account.clone();
    let outcome = store
        .write(&actor(), move |tx, repos| {
            repos
                .launcher_accounts
                .set_username(tx, &id, "Slayer Prime", "slayer prime")
        })
        .await
        .unwrap();
    assert_eq!(outcome, CreateOutcome::Created);
    let renamed = by_user(&store, &slayer).await.unwrap();
    assert_eq!(
        (renamed.display_name.as_str(), renamed.username_set),
        ("Slayer Prime", Some(true))
    );
    let user = slayer.user_id.clone();
    let game = store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(game.name, "Slayer Prime", "the game account's name follows");
    let id = no_email.account.clone();
    let taken = store
        .write(&actor(), move |tx, repos| {
            repos
                .launcher_accounts
                .set_username(tx, &id, "slayer prime", "slayer prime")
        })
        .await
        .unwrap();
    assert_eq!(taken, CreateOutcome::DisplayNameTaken);

    // The admin list: newest first, filtered, searched, paged; legacy rows without an approval count as approved
    let mut legacy = launcher("Legacy", Some("legacy@example.test"), T2);
    legacy.approval_status = None;
    create(&store, &legacy).await;
    let mut pending = launcher("Waiting", Some("waiting@example.test"), "2026-10-04T00:00:00.000Z");
    pending.approval_status = Some(ApprovalStatus::Pending);
    create(&store, &pending).await;
    let list = |filter: AdminAccountFilter| {
        let store = store.clone();
        async move {
            store
                .read(move |conn, repos| repos.launcher_accounts.list_for_admin(conn, &filter))
                .await
                .unwrap()
        }
    };
    let all = AdminAccountFilter {
        approval_status: None,
        status: None,
        search: None,
        skip: 0,
        limit: 0,
    };
    let (accounts, total) = list(all.clone()).await;
    assert_eq!(total, 5);
    assert_eq!(
        accounts.first().map(|a| a.display_name.as_str()),
        Some("Waiting"),
        "newest first"
    );
    let (approved, total) = list(AdminAccountFilter {
        approval_status: Some(ApprovalStatus::Approved),
        ..all.clone()
    })
    .await;
    assert_eq!(total, 4, "{approved:?}");
    assert!(approved.iter().any(|a| a.display_name == "Legacy"));
    let (found, total) = list(AdminAccountFilter {
        search: Some("PRIME".into()),
        ..all.clone()
    })
    .await;
    assert_eq!((found.len(), total), (1, 1));
    let (page, total) = list(AdminAccountFilter {
        skip: 1,
        limit: 2,
        ..all
    })
    .await;
    assert_eq!((page.len(), total), (2, 5));

    // Access state and roles
    let id = pending.account.clone();
    let change = AccessChange {
        approval_status: Some(ApprovalStatus::Approved),
        status: Some(OperationalStatus::Banned),
        approval_updated_at: T2.into(),
        approval_updated_by: "admin-1".into(),
        approval_reason: None,
    };
    let updated = store
        .write(&actor(), move |tx, repos| {
            repos.launcher_accounts.set_access_state(tx, &id, &change)
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            updated.approval_status,
            updated.status,
            updated.approval_reason.as_deref()
        ),
        (Some(ApprovalStatus::Approved), OperationalStatus::Banned, Some(""))
    );
    let id = pending.account.clone();
    let roles = vec!["player".to_owned(), "tester".to_owned()];
    let with_roles = store
        .write(&actor(), move |tx, repos| {
            repos.launcher_accounts.update_roles(tx, &id, &roles, T2, "admin-1")
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (with_roles.roles.len(), with_roles.roles_updated_by.as_deref()),
        (2, Some("admin-1"))
    );
    let ghost = AccountId::new(&NodeIdentity::standalone().node);
    let none = store
        .write(&actor(), move |tx, repos| {
            repos.launcher_accounts.update_roles(tx, &ghost, &[], T2, "x")
        })
        .await
        .unwrap();
    assert!(none.is_none());

    // An unverified registration is soft-deleted only while it still is one, and frees its address and name
    let mut registration = launcher("Registrant", Some("registrant@example.test"), T0);
    registration.email_verification_pending_since = Some(T0.into());
    create(&store, &registration).await;
    let listed = store
        .read(|conn, repos| repos.launcher_accounts.list_unverified_registrations(conn, T1, 10))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    let id = registration.account.clone();
    assert!(
        !store
            .write(&actor(), move |tx, repos| repos
                .launcher_accounts
                .delete_unverified_registration(tx, &id, T0))
            .await
            .unwrap(),
        "not yet past the cut-off"
    );
    let id = registration.account.clone();
    assert!(
        store
            .write(&actor(), move |tx, repos| repos
                .launcher_accounts
                .delete_unverified_registration(tx, &id, T1))
            .await
            .unwrap()
    );
    assert!(by_user(&store, &registration).await.is_none());
    assert_eq!(
        create(&store, &launcher("Registrant", Some("registrant@example.test"), T2)).await,
        CreateOutcome::Created
    );

    let log = store.database().changes(0, 10_000).await.unwrap();
    assert!(
        log.iter()
            .any(|e| e.table_name == "launcher_accounts" && e.op == "insert" && e.actor == "admin:owner")
    );
    assert!(
        log.iter()
            .any(|e| e.table_name == "launcher_accounts" && e.op == "update"),
        "the soft delete is a logged update"
    );
}

async fn sessions_and_codes(store: IslesStore) {
    let account = launcher("Sessions", Some("sessions@example.test"), T0);
    create(&store, &account).await;
    let id = account.account.clone();
    let session = |n: u32, family: &str, expires: &str| RefreshSessionRecord {
        id: format!("session-{n}"),
        token_hash: format!("hash-{n}"),
        account: id.clone(),
        family_id: family.into(),
        device_id: "device".into(),
        device_name: "PC".into(),
        created_at: T0.into(),
        expires_at: expires.into(),
        revoked_at: None,
    };
    let (s1, s2, s3) = (
        session(1, "fam-a", T2),
        session(2, "fam-a", T2),
        session(3, "fam-b", T0),
    );
    let rows = [s1.clone(), s2.clone(), s3.clone()];
    store
        .write(&actor(), move |tx, repos| {
            rows.iter().try_for_each(|s| repos.refresh_sessions.create(tx, s))
        })
        .await
        .unwrap();
    let (a, now) = (account.account.clone(), T1.to_owned());
    let active = store
        .read(move |conn, repos| {
            Ok((
                repos.refresh_sessions.is_family_active(conn, "fam-a", &a, &now)?,
                repos.refresh_sessions.is_family_active(conn, "fam-b", &a, &now)?,
                repos.refresh_sessions.is_family_active(
                    conn,
                    "fam-a",
                    &AccountId::new(&NodeIdentity::standalone().node),
                    &now,
                )?,
                repos.refresh_sessions.find_by_token_hash(conn, "hash-2")?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(
        (active.0, active.1, active.2),
        (true, false, false),
        "live, expired, another account's"
    );
    assert_eq!(active.3, Some(s2.clone()));
    store
        .write(&actor(), move |tx, repos| {
            repos.refresh_sessions.revoke_by_id(tx, "session-1", T1)
        })
        .await
        .unwrap();
    let hash = store
        .read(|conn, repos| repos.refresh_sessions.find_by_token_hash(conn, "hash-1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hash.revoked_at.as_deref(), Some(T1));
    store
        .write(&actor(), move |tx, repos| {
            repos.refresh_sessions.revoke_family(tx, "fam-a", T2)
        })
        .await
        .unwrap();
    let (a, now) = (account.account.clone(), T1.to_owned());
    assert!(
        !store
            .read(move |conn, repos| repos.refresh_sessions.is_family_active(conn, "fam-a", &a, &now))
            .await
            .unwrap()
    );
    let still = store
        .read(|conn, repos| repos.refresh_sessions.find_by_token_hash(conn, "hash-1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        still.revoked_at.as_deref(),
        Some(T1),
        "an already revoked row keeps its time"
    );
    let purged = store
        .write(&actor(), move |tx, repos| repos.refresh_sessions.purge_expired(tx, T1))
        .await
        .unwrap();
    assert_eq!(purged, 1, "the expired family-b session");

    // A game exchange code is consumed once, before it expires
    let code = |hash: &str, expires: &str| GameExchangeCodeRecord {
        code_hash: hash.into(),
        account: account.account.clone(),
        launcher_session_id: "fam-a".into(),
        build_changelist: 647_472,
        executable_sha256: "a".repeat(64),
        runtime_channel: RuntimeChannel::Stable,
        runtime_sha256: "b".repeat(64),
        runtime_manifest_version: "0.1.78".into(),
        runtime_artifact_set_sha256: "c".repeat(64),
        realm_id: None,
        created_at: T0.into(),
        expires_at: expires.into(),
        consumed_at: None,
    };
    let (fresh, old, revoked) = (code("code-1", T2), code("code-2", T0), code("code-3", T2));
    store
        .write(&actor(), move |tx, repos| {
            repos.game_exchange_codes.create(tx, &fresh)?;
            repos.game_exchange_codes.create(tx, &old)?;
            repos.game_exchange_codes.create(tx, &revoked)
        })
        .await
        .unwrap();
    let consume = |hash: &'static str, now: &'static str| {
        let store = store.clone();
        async move {
            store
                .write(&actor(), move |tx, repos| {
                    repos.game_exchange_codes.consume_by_code_hash(tx, hash, now)
                })
                .await
                .unwrap()
        }
    };
    assert!(consume("code-2", T1).await.is_none(), "expired");
    assert!(consume("nope", T1).await.is_none());
    let got = consume("code-1", T1).await.unwrap();
    assert_eq!(
        (got.build_changelist, got.runtime_channel, got.consumed_at.as_deref()),
        (647_472, RuntimeChannel::Stable, Some(T1))
    );
    assert!(consume("code-1", T1).await.is_none(), "once");
    let a = account.account.clone();
    store
        .write(&actor(), move |tx, repos| {
            repos.game_exchange_codes.revoke_unused_for_account(tx, &a, T1)
        })
        .await
        .unwrap();
    assert!(consume("code-3", T1).await.is_none(), "revoked");

    // Discord OAuth transactions: the state is consumed once, then the completion code once
    let transaction = DiscordOAuthTransactionRecord {
        state: "state-1".into(),
        code_verifier: "verifier".into(),
        created_at: T0.into(),
        expires_at: T2.into(),
        consumed_at: None,
        completion_code_hash: None,
        completion_expires_at: None,
        completion_consumed_at: None,
        account: None,
        link_account: Some(account.account.clone()),
    };
    store
        .write(&actor(), move |tx, repos| {
            repos.discord_oauth_transactions.create(tx, &transaction)
        })
        .await
        .unwrap();
    let state = |now: &'static str| {
        let store = store.clone();
        async move {
            store
                .write(&actor(), move |tx, repos| {
                    repos.discord_oauth_transactions.consume_by_state(tx, "state-1", now)
                })
                .await
                .unwrap()
        }
    };
    let consumed = state(T1).await.unwrap();
    assert_eq!(
        (consumed.code_verifier.as_str(), consumed.link_account.clone()),
        ("verifier", Some(account.account.clone()))
    );
    assert!(state(T1).await.is_none(), "the state is single use");
    let a = account.account.clone();
    store
        .write(&actor(), move |tx, repos| {
            repos
                .discord_oauth_transactions
                .attach_completion_code(tx, "state-1", "completion-hash", T2, &a)
        })
        .await
        .unwrap();
    let completion = |now: &'static str| {
        let store = store.clone();
        async move {
            store
                .write(&actor(), move |tx, repos| {
                    repos
                        .discord_oauth_transactions
                        .consume_by_completion_code_hash(tx, "completion-hash", now)
                })
                .await
                .unwrap()
        }
    };
    assert!(completion("2026-10-04T00:00:00.000Z").await.is_none(), "expired");
    assert_eq!(completion(T1).await.unwrap().account, Some(account.account.clone()));
    assert!(completion(T1).await.is_none());

    // Email codes: one row per account and purpose; tries are counted; the used-up row goes
    let record = |hash: &str, attempts: u32| EmailCodeRecord {
        account: account.account.clone(),
        purpose: EmailCodePurpose::Verify,
        code_hash: hash.into(),
        attempts,
        created_at: T0.into(),
        expires_at: T2.into(),
    };
    let first = record("one", 0);
    store
        .write(&actor(), move |tx, repos| repos.email_codes.replace(tx, &first))
        .await
        .unwrap();
    let second = record("two", 0);
    store
        .write(&actor(), move |tx, repos| repos.email_codes.replace(tx, &second))
        .await
        .unwrap();
    let a = account.account.clone();
    let live = store
        .read(move |conn, repos| repos.email_codes.peek_live(conn, &a, EmailCodePurpose::Verify, T1, 5))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(live.code_hash, "two", "a new code replaces the last");
    let tries = |expected: Option<u32>| {
        let (store, a) = (store.clone(), account.account.clone());
        async move {
            let counted = store
                .write(&actor(), move |tx, repos| {
                    repos.email_codes.reserve_try(tx, &a, EmailCodePurpose::Verify, T1, 5)
                })
                .await
                .unwrap();
            assert_eq!(counted.map(|c| c.attempts), expected);
        }
    };
    for n in 1..=5 {
        tries(Some(n)).await;
    }
    tries(None).await;
    let a = account.account.clone();
    assert!(
        store
            .read(move |conn, repos| repos.email_codes.peek_live(conn, &a, EmailCodePurpose::Verify, T1, 5))
            .await
            .unwrap()
            .is_none(),
        "used up, and gone"
    );
    let third = record("three", 0);
    store
        .write(&actor(), move |tx, repos| repos.email_codes.replace(tx, &third))
        .await
        .unwrap();
    let a = account.account.clone();
    assert!(
        !store
            .write(&actor(), move |tx, repos| repos.email_codes.consume(
                tx,
                &a,
                EmailCodePurpose::Verify,
                "wrong",
                T1,
                5
            ))
            .await
            .unwrap()
    );
    let a = account.account.clone();
    assert!(
        store
            .write(&actor(), move |tx, repos| repos.email_codes.consume(
                tx,
                &a,
                EmailCodePurpose::Verify,
                "three",
                T1,
                5
            ))
            .await
            .unwrap()
    );
    let a = account.account.clone();
    assert!(
        !store
            .write(&actor(), move |tx, repos| repos.email_codes.remove(
                tx,
                &a,
                EmailCodePurpose::Verify,
                "three"
            ))
            .await
            .unwrap()
    );

    // Discord links: one per Discord user and per account
    let identity = |subject: &str| AuthIdentityRecord {
        provider_subject: subject.into(),
        account: account.account.clone(),
        provider_username: "user".into(),
        provider_avatar_url: None,
        linked_at: T0.into(),
    };
    let first = identity("9000");
    assert!(
        store
            .write(&actor(), move |tx, repos| repos.auth_identities.create(tx, &first))
            .await
            .unwrap()
    );
    let again = identity("9000");
    assert!(
        !store
            .write(&actor(), move |tx, repos| repos.auth_identities.create(tx, &again))
            .await
            .unwrap()
    );
    let another = identity("9001");
    assert!(
        !store
            .write(&actor(), move |tx, repos| repos.auth_identities.create(tx, &another))
            .await
            .unwrap(),
        "one link per account"
    );
    let a = account.account.clone();
    let (by_subject, by_account) = store
        .read(move |conn, repos| {
            Ok((
                repos.auth_identities.find_by_provider_subject(conn, "9000")?,
                repos.auth_identities.find_by_account(conn, &a)?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(by_subject, by_account);
    let a = account.account.clone();
    assert_eq!(
        store
            .write(&actor(), move |tx, repos| repos
                .auth_identities
                .delete_for_account(tx, &a))
            .await
            .unwrap(),
        1
    );

    // API keys: the game server set is replaced, a user has one key
    store
        .write(&actor(), |tx, repos| {
            repos
                .api_keys
                .replace_game_server_key_hashes(tx, &["aa".into(), "bb".into()])
        })
        .await
        .unwrap();
    store
        .write(&actor(), |tx, repos| {
            repos
                .api_keys
                .replace_game_server_key_hashes(tx, &["bb".into(), "cc".into()])
        })
        .await
        .unwrap();
    let mut hashes: Vec<_> = store
        .read(|conn, repos| repos.api_keys.find_all_game_server_key_hashes(conn))
        .await
        .unwrap()
        .into_iter()
        .filter_map(|k| k.key_hash)
        .collect();
    hashes.sort();
    assert_eq!(hashes, ["bb", "cc"]);
    store
        .write(&actor(), |tx, repos| {
            repos.api_keys.replace_game_server_key_hashes(tx, &[])
        })
        .await
        .unwrap();
    assert!(
        store
            .read(|conn, repos| repos.api_keys.find_all_game_server_key_hashes(conn))
            .await
            .unwrap()
            .is_empty()
    );
    for hash in ["first", "second"] {
        let a = account.account.clone();
        store
            .write(&actor(), move |tx, repos| {
                repos.api_keys.insert_user_key_hash(tx, &a, hash)
            })
            .await
            .unwrap();
    }
    let keys = store
        .read(|conn, repos| repos.api_keys.find_all_user_key_hashes(conn))
        .await
        .unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].key_hash, "second");
}

#[tokio::test(flavor = "multi_thread")]
async fn launcher_accounts_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    launcher_accounts(open(&DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    }))
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_codes_and_keys_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    sessions_and_codes(open(&DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    }))
    .await;
}

struct PgDatabase {
    admin: PostgresConfig,
    config: PostgresConfig,
}

impl Drop for PgDatabase {
    fn drop(&mut self) {
        if let Ok(mut conn) = PgConnection::establish(&self.admin.conninfo()) {
            let _ = conn.batch_execute(&format!(
                "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
                self.config.database
            ));
        }
    }
}

fn throwaway_postgres(label: &str) -> Option<PgDatabase> {
    let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
    let rest = url.strip_prefix("postgres://")?;
    let (login, location) = rest.split_once('@')?;
    let (user, password) = login.split_once(':').unwrap_or((login, ""));
    let (hostport, database) = location.split_once('/')?;
    let (host, port) = hostport.split_once(':').unwrap_or((hostport, "5432"));
    let admin = PostgresConfig {
        host: host.into(),
        port: port.parse().ok()?,
        user: user.into(),
        database: database.into(),
        password: password.into(),
        allow_remote: false,
    };
    let name = format!("isles_auth_{label}_{}", std::process::id());
    PgConnection::establish(&admin.conninfo())
        .expect("REFORGED_TEST_POSTGRES_URL is reachable")
        .batch_execute(&format!("CREATE DATABASE \"{name}\""))
        .unwrap();
    let config = PostgresConfig {
        database: name,
        ..admin.clone()
    };
    Some(PgDatabase { admin, config })
}

#[tokio::test(flavor = "multi_thread")]
async fn launcher_accounts_on_postgres() {
    let Some(pg) = throwaway_postgres("accounts") else {
        eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the Postgres half is skipped");
        return;
    };
    launcher_accounts(open(&DatabaseConfig::Postgres(pg.config.clone()))).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_codes_and_keys_on_postgres() {
    let Some(pg) = throwaway_postgres("codes") else {
        eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the Postgres half is skipped");
        return;
    };
    sessions_and_codes(open(&DatabaseConfig::Postgres(pg.config.clone()))).await;
}
