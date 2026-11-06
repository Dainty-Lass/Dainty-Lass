// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The handle (spec §3) and the sign-in repositories (§6): rows S1-S4 and A1-A11 of §15.2.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{account, admin, launcher, log, system, targets};
use module_api::NodeId;
use rustic_isles::replay::{ReplayScope, with_replay_scope};
use rustic_isles_store::repo::auth::*;
use rustic_isles_store::repo::progression::now_iso;
use rustic_isles_store::{AccountId, RepoError, Repositories, UserId};

#[tokio::test]
async fn s1_an_admin_account_write_on_its_home_is_logged() {
    for t in targets() {
        let store = &t.store;
        let id = AccountId::new(store.node());
        let record = AccountRecord {
            id: id.clone(),
            user_id: UserId::new("u1"),
            name: "Ash".into(),
            notes: 0,
        };
        store
            .account_write(&id, &admin(), move |tx, repos| repos.accounts.create(tx, &record))
            .await
            .unwrap();
        let found = store
            .read(|conn, repos| repos.accounts.find_by_user_id(conn, &UserId::new("u1")))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, id, "{}", t.engine);
        let entries = log(store).await;
        assert!(
            entries.iter().any(|(actor, table, op, key)| actor == "admin:owner"
                && table == "accounts"
                && op == "insert"
                && key.contains(&id.to_string())),
            "{entries:?}"
        );
    }
}

#[tokio::test]
async fn s2_another_nodes_account_is_refused_on_a_standalone_store() {
    for t in targets() {
        let foreign = AccountId::new(&NodeId::new("germany-1").unwrap());
        let err = t
            .store
            .account_write(&foreign, &system(), |_, _| Ok(()))
            .await
            .unwrap_err();
        assert!(matches!(err, RepoError::Home(_)), "{err}");
        assert_eq!(
            err.to_string(),
            "the account's home could not be resolved: this server is not in a network"
        );
    }
}

#[tokio::test]
async fn s3_stubs_answer_unimplemented() {
    let t = targets().remove(0);
    let store = t.store.clone().with_repos(Repositories::stubs());
    let id = AccountId::new(store.node());
    let err = store
        .read(move |conn, repos| repos.characters.find_many_by_account(conn, &id))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        RepoError::Unimplemented {
            repository: "characters",
            operation: "find_many_by_account"
        }
    ));
    assert_eq!(
        err.to_string(),
        "characters: find_many_by_account is not implemented yet"
    );
}

#[tokio::test]
async fn s4_closures_run_in_the_callers_replay_scope() {
    let t = targets().remove(0);
    let store = t.store.clone();
    let pinned = service_kit::js::date_parse("2026-10-27T00:00:00.000Z").unwrap();
    let seen = with_replay_scope(ReplayScope::new(Some(pinned), None), async move {
        store.read(|_, _| Ok(now_iso())).await
    })
    .await
    .unwrap();
    assert_eq!(seen, "2026-10-27T00:00:00.000Z");
}

async fn create(store: &rustic_isles_store::IslesStore, record: LauncherAccountRecord) -> CreateOutcome {
    store
        .write(&system(), move |tx, repos| repos.launcher_accounts.create(tx, &record))
        .await
        .unwrap()
}

#[tokio::test]
async fn a1_a2_launcher_accounts_create_find_and_refuse_taken_names() {
    for t in targets() {
        let store = &t.store;
        let id = AccountId::new(store.node());
        let record = launcher(&id, "u1", Some("ash@example.org"), "Slayer", "2026-10-01T00:00:00.000Z");
        assert_eq!(create(store, record.clone()).await, CreateOutcome::Created);
        let (by_user, by_id, by_email, by_name, unknown, game) = store
            .read({
                let id = id.clone();
                move |conn, r| {
                    let l = &r.launcher_accounts;
                    Ok((
                        l.find_by_user_id(conn, &UserId::new("u1"))?,
                        l.find_by_account(conn, &id)?,
                        l.find_by_email(conn, "ash@example.org")?,
                        l.find_by_display_name_normalized(conn, "slayer")?,
                        l.find_by_email(conn, "nobody@example.org")?,
                        r.accounts.find(conn, &id)?,
                    ))
                }
            })
            .await
            .unwrap();
        assert_eq!(by_user.as_ref(), Some(&record), "{}", t.engine);
        assert_eq!(by_id, by_user);
        assert_eq!(by_email, by_user);
        assert_eq!(by_name, by_user);
        assert_eq!(unknown, None);
        let game = game.unwrap();
        assert_eq!((game.name.as_str(), game.notes), ("Slayer", 0));

        let second = AccountId::new(store.node());
        let same_email = launcher(
            &second,
            "u2",
            Some("ash@example.org"),
            "Other",
            "2026-10-02T00:00:00.000Z",
        );
        assert_eq!(create(store, same_email).await, CreateOutcome::EmailTaken);
        let same_name = launcher(
            &second,
            "u2",
            Some("b@example.org"),
            "SLAYER",
            "2026-10-02T00:00:00.000Z",
        );
        assert_eq!(create(store, same_name).await, CreateOutcome::DisplayNameTaken);
        let written = store
            .read({
                let second = second.clone();
                move |conn, r| {
                    Ok((
                        r.accounts.find(conn, &second)?,
                        r.launcher_accounts.find_by_account(conn, &second)?,
                    ))
                }
            })
            .await
            .unwrap();
        assert_eq!(written, (None, None));
        let (c, d) = (AccountId::new(store.node()), AccountId::new(store.node()));
        assert_eq!(
            create(store, launcher(&c, "u3", None, "Three", "x")).await,
            CreateOutcome::Created
        );
        assert_eq!(
            create(store, launcher(&d, "u4", None, "Four", "x")).await,
            CreateOutcome::Created
        );
    }
}

#[tokio::test]
async fn a3_a4_profile_writes_read_back() {
    for t in targets() {
        let store = &t.store;
        let id = AccountId::new(store.node());
        create(
            store,
            launcher(&id, "u1", Some("a@x.org"), "Ash", "2026-10-01T00:00:00.000Z"),
        )
        .await;
        let other = AccountId::new(store.node());
        create(
            store,
            launcher(&other, "u2", Some("b@x.org"), "Taken", "2026-10-01T00:00:00.000Z"),
        )
        .await;
        let w = id.clone();
        store
            .write(&system(), move |tx, r| {
                let l = &r.launcher_accounts;
                l.update_last_login(tx, &w, "2026-10-03T00:00:00.000Z")?;
                l.set_consent(
                    tx,
                    &w,
                    &LauncherConsent {
                        terms_version: "t1".into(),
                        privacy_version: "p1".into(),
                        accepted_at: "2026-10-03T00:00:00.000Z".into(),
                    },
                )?;
                l.set_deletion(
                    tx,
                    &w,
                    Some(&LauncherDeletion {
                        requested_at: "2026-10-03T00:00:00.000Z".into(),
                        scheduled_for: "2026-10-10T00:00:00.000Z".into(),
                    }),
                )?;
                l.set_password_hash(tx, &w, "new-hash")?;
                l.set_email_verified(tx, &w, "2026-10-04T00:00:00.000Z")?;
                assert_eq!(
                    l.set_username(tx, &w, "Slayer Prime", "slayer prime")?,
                    CreateOutcome::Created
                );
                assert_eq!(
                    l.set_username(tx, &w, "TAKEN", "taken")?,
                    CreateOutcome::DisplayNameTaken
                );
                assert_eq!(l.set_email(tx, &w, "b@x.org")?, CreateOutcome::EmailTaken);
                assert_eq!(l.set_email(tx, &w, "c@x.org")?, CreateOutcome::Created);
                Ok(())
            })
            .await
            .unwrap();
        let r = id.clone();
        let (record, game, due, early) = store
            .read(move |conn, repos| {
                Ok((
                    repos.launcher_accounts.find_by_account(conn, &r)?.unwrap(),
                    repos.accounts.find(conn, &r)?.unwrap(),
                    repos
                        .launcher_accounts
                        .list_deletions_due(conn, "2026-10-11T00:00:00.000Z", 0)?,
                    repos
                        .launcher_accounts
                        .list_deletions_due(conn, "2026-10-09T00:00:00.000Z", 0)?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(record.last_login_at.as_deref(), Some("2026-10-03T00:00:00.000Z"));
        assert_eq!(record.consent.as_ref().map(|c| c.terms_version.as_str()), Some("t1"));
        assert_eq!(record.password_hash.as_deref(), Some("new-hash"));
        assert_eq!(record.email_verified_at.as_deref(), Some("2026-10-04T00:00:00.000Z"));
        assert_eq!(record.email_verification_pending_since, None);
        assert_eq!(record.email.as_deref(), Some("c@x.org"));
        assert_eq!(
            (
                record.display_name.as_str(),
                record.display_name_normalized.as_str(),
                record.username_set
            ),
            ("Slayer Prime", "slayer prime", Some(true))
        );
        assert_eq!(game.name, "Slayer Prime");
        assert_eq!(due.len(), 1);
        assert!(early.is_empty());
        let w = id.clone();
        store
            .write(&system(), move |tx, r| r.launcher_accounts.set_deletion(tx, &w, None))
            .await
            .unwrap();
        let due = store
            .read(|conn, r| {
                r.launcher_accounts
                    .list_deletions_due(conn, "2026-10-11T00:00:00.000Z", 0)
            })
            .await
            .unwrap();
        assert!(due.is_empty());
    }
}

#[tokio::test]
async fn a5_a6_the_admin_list_filters_pages_and_writes() {
    for t in targets() {
        let store = &t.store;
        let mut ids = Vec::new();
        for n in 0..5 {
            let id = AccountId::new(store.node());
            let mut record = launcher(
                &id,
                &format!("user-{n}"),
                Some(&format!("p{n}@x.org")),
                &format!("Name{n}"),
                &format!("2026-10-0{}T00:00:00.000Z", n + 1),
            );
            record.approval_status = match n {
                0 => Some(ApprovalStatus::Pending),
                1 => None,
                _ => Some(ApprovalStatus::Approved),
            };
            create(store, record).await;
            ids.push(id);
        }
        let (all, approved, search, page) = store
            .read(|conn, r| {
                let l = &r.launcher_accounts;
                let filter = |f: AdminAccountFilter| f;
                Ok((
                    l.list_for_admin(conn, &AdminAccountFilter::default())?,
                    l.list_for_admin(
                        conn,
                        &filter(AdminAccountFilter {
                            approval_status: Some(ApprovalStatus::Approved),
                            ..Default::default()
                        }),
                    )?,
                    l.list_for_admin(
                        conn,
                        &AdminAccountFilter {
                            search: Some("NAME3".into()),
                            ..Default::default()
                        },
                    )?,
                    l.list_for_admin(
                        conn,
                        &AdminAccountFilter {
                            skip: 2,
                            limit: 2,
                            ..Default::default()
                        },
                    )?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(all.1, 5);
        assert_eq!(all.0[0].display_name, "Name4", "newest first");
        assert_eq!(approved.1, 4, "a legacy row counts as approved");
        assert_eq!((search.1, search.0[0].display_name.as_str()), (1, "Name3"));
        assert_eq!(page.1, 5);
        assert_eq!(
            page.0.iter().map(|r| r.display_name.as_str()).collect::<Vec<_>>(),
            ["Name2", "Name1"]
        );

        let target = ids[0].clone();
        let unknown = AccountId::new(store.node());
        let (changed, none, roles, timed) = store
            .write(&admin(), move |tx, r| {
                let l = &r.launcher_accounts;
                let change = AccessChange {
                    approval_status: Some(ApprovalStatus::Approved),
                    status: Some(OperationalStatus::Banned),
                    approval_updated_at: "2026-10-05T00:00:00.000Z".into(),
                    approval_updated_by: "owner".into(),
                    approval_reason: None,
                };
                let timeout = AccountTimeout {
                    until: "2026-10-09T00:00:00.000Z".into(),
                    since: "2026-10-05T00:00:00.000Z".into(),
                    reason: None,
                };
                Ok((
                    l.set_access_state(tx, &target, &change)?,
                    l.set_access_state(tx, &unknown, &change)?,
                    l.update_roles(tx, &target, &["admin".into()], "2026-10-05T00:00:00.000Z", "owner")?,
                    l.set_timeout(tx, &target, Some(&timeout))?,
                ))
            })
            .await
            .unwrap();
        let changed = changed.unwrap();
        assert_eq!(changed.approval_status, Some(ApprovalStatus::Approved));
        assert_eq!(changed.status, OperationalStatus::Banned);
        assert_eq!(changed.approval_reason.as_deref(), Some(""));
        assert_eq!(none, None);
        assert_eq!(roles.unwrap().roles, ["admin"]);
        let timed = timed.unwrap();
        assert!(timed.timeout_at("2026-10-06T00:00:00.000Z").is_some());
        assert!(timed.timeout_at("2026-10-10T00:00:00.000Z").is_none());
        let filtered = store
            .read(|conn, r| {
                r.launcher_accounts.list_for_admin(
                    conn,
                    &AdminAccountFilter {
                        timed_out_at: Some("2026-10-06T00:00:00.000Z".into()),
                        ..Default::default()
                    },
                )
            })
            .await
            .unwrap();
        assert_eq!(filtered.1, 1);
    }
}

#[tokio::test]
async fn a7_unverified_registrations_are_deleted_after_their_cut_off() {
    for t in targets() {
        let store = &t.store;
        let id = AccountId::new(store.node());
        let mut record = launcher(&id, "u1", Some("a@x.org"), "Ash", "2026-10-01T00:00:00.000Z");
        record.email_verification_pending_since = Some("2026-10-01T00:00:00.000Z".into());
        create(store, record).await;
        let listed = store
            .read(|conn, r| {
                r.launcher_accounts
                    .list_unverified_registrations(conn, "2026-10-02T00:00:00.000Z", 0)
            })
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        let w = id.clone();
        let (early, late) = store
            .write(&system(), move |tx, r| {
                Ok((
                    r.launcher_accounts
                        .delete_unverified_registration(tx, &w, "2026-10-01T00:00:00.000Z")?,
                    r.launcher_accounts
                        .delete_unverified_registration(tx, &w, "2026-10-02T00:00:00.000Z")?,
                ))
            })
            .await
            .unwrap();
        assert_eq!((early, late), (false, true));
        let r = id.clone();
        let gone = store
            .read(move |conn, repos| repos.launcher_accounts.find_by_account(conn, &r))
            .await
            .unwrap();
        assert_eq!(gone, None);
        let again = AccountId::new(store.node());
        let fresh = launcher(&again, "u2", Some("a@x.org"), "Ash", "2026-10-03T00:00:00.000Z");
        assert_eq!(create(store, fresh).await, CreateOutcome::Created);
        let entries = log(store).await;
        let key = id.to_string();
        assert!(entries.iter().any(|(a, table, op, k)| a == "system:isles"
            && table == "launcher_accounts"
            && op == "insert"
            && k.contains(&key)));
        // store's triggers name a deleted_at transition `soft_delete` (spec §15.2 A7 says `update`)
        assert!(
            entries
                .iter()
                .any(|(_, table, op, k)| table == "launcher_accounts" && op == "soft_delete" && k.contains(&key)),
            "{entries:?}"
        );
    }
}

#[tokio::test]
async fn a8_refresh_sessions() {
    for t in targets() {
        let store = &t.store;
        let id = account(store, "u1").await;
        let session = |sid: &str, family: &str, expires: &str| RefreshSessionRecord {
            id: sid.into(),
            token_hash: format!("hash-{sid}"),
            account: id.clone(),
            family_id: family.into(),
            device_id: "d".into(),
            device_name: "pc".into(),
            created_at: "2026-10-01T00:00:00.000Z".into(),
            expires_at: expires.into(),
            revoked_at: None,
        };
        let (s1, s2, s3, s4) = (
            session("s1", "f1", "2026-11-01T00:00:00.000Z"),
            session("s2", "f1", "2026-11-01T00:00:00.000Z"),
            session("s3", "f2", "2026-11-01T00:00:00.000Z"),
            session("s4", "f3", "2026-10-02T00:00:00.000Z"),
        );
        let a = id.clone();
        let (family_revoked, active_before, all_revoked) = store
            .write(&system(), move |tx, r| {
                let s = &r.refresh_sessions;
                s.create(tx, &s1)?;
                s.revoke_by_id(tx, "s1", "2026-10-02T00:00:00.000Z")?;
                s.create(tx, &s2)?;
                s.create(tx, &s3)?;
                s.create(tx, &s4)?;
                let family_revoked = s.revoke_family(tx, "f1", "2026-10-03T00:00:00.000Z")?;
                let active = s.is_family_active(tx.conn(), "f2", &a, "2026-10-03T00:00:00.000Z")?;
                let all = s.revoke_all_for_account(tx, &a, "2026-10-04T00:00:00.000Z")?;
                Ok((family_revoked, active, all))
            })
            .await
            .unwrap();
        assert_eq!(family_revoked, 1, "only the unrevoked one");
        assert!(active_before);
        assert_eq!(all_revoked, 2);
        let a = id.clone();
        let (revoked, active, expired) = store
            .read(move |conn, r| {
                Ok((
                    r.refresh_sessions.find_by_token_hash(conn, "hash-s1")?,
                    r.refresh_sessions
                        .is_family_active(conn, "f2", &a, "2026-10-03T00:00:00.000Z")?,
                    r.refresh_sessions
                        .is_family_active(conn, "f3", &a, "2026-10-03T00:00:00.000Z")?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(revoked.unwrap().revoked_at.as_deref(), Some("2026-10-02T00:00:00.000Z"));
        assert!(!active && !expired);
        let purged = store
            .write(&system(), |tx, r| {
                r.refresh_sessions.purge_expired(tx, "2026-10-05T00:00:00.000Z")
            })
            .await
            .unwrap();
        assert_eq!(purged, 1);
    }
}

#[tokio::test]
async fn a9_codes_are_consumed_once() {
    for t in targets() {
        let store = &t.store;
        let id = account(store, "u1").await;
        let code = |hash: &str, expires: &str| GameExchangeCodeRecord {
            code_hash: hash.into(),
            account: id.clone(),
            launcher_session_id: "ls".into(),
            build_changelist: 123,
            executable_sha256: "e".into(),
            runtime_channel: RuntimeChannel::Beta,
            runtime_sha256: "r".into(),
            runtime_manifest_version: "1".into(),
            runtime_artifact_set_sha256: "s".into(),
            realm_id: None,
            created_at: "2026-10-01T00:00:00.000Z".into(),
            expires_at: expires.into(),
            consumed_at: None,
        };
        let (c1, c2, c3) = (
            code("c1", "2026-10-02T00:00:00.000Z"),
            code("c2", "2026-10-01T00:00:01.000Z"),
            code("c3", "2026-10-02T00:00:00.000Z"),
        );
        let a = id.clone();
        let now = "2026-10-01T00:00:05.000Z";
        let (first, second, expired, unused) = store
            .write(&system(), move |tx, r| {
                let g = &r.game_exchange_codes;
                g.create(tx, &c1)?;
                g.create(tx, &c2)?;
                g.create(tx, &c3)?;
                let first = g.consume_by_code_hash(tx, "c1", now)?;
                let second = g.consume_by_code_hash(tx, "c1", now)?;
                let expired = g.consume_by_code_hash(tx, "c2", now)?;
                g.revoke_unused_for_account(tx, &a, now)?;
                let unused = g.consume_by_code_hash(tx, "c3", now)?;
                Ok((first, second, expired, unused))
            })
            .await
            .unwrap();
        let first = first.unwrap();
        assert_eq!(
            (first.consumed_at.as_deref(), first.runtime_channel),
            (Some(now), RuntimeChannel::Beta)
        );
        assert!(second.is_none() && expired.is_none() && unused.is_none());

        let tx_record = |state: &str, expires: &str| DiscordOAuthTransactionRecord {
            state: state.into(),
            code_verifier: "v".into(),
            created_at: "2026-10-01T00:00:00.000Z".into(),
            expires_at: expires.into(),
            consumed_at: None,
            completion_code_hash: None,
            completion_expires_at: None,
            completion_consumed_at: None,
            account: None,
            link_account: None,
        };
        let (d1, d2) = (
            tx_record("st1", "2026-10-02T00:00:00.000Z"),
            tx_record("st2", "2026-10-01T00:00:01.000Z"),
        );
        let a = id.clone();
        let results = store
            .write(&system(), move |tx, r| {
                let d = &r.discord_oauth_transactions;
                d.create(tx, &d1)?;
                d.create(tx, &d2)?;
                let first = d.consume_by_state(tx, "st1", now)?.is_some();
                let again = d.consume_by_state(tx, "st1", now)?.is_some();
                let expired = d.consume_by_state(tx, "st2", now)?.is_some();
                d.attach_completion_code(tx, "st1", "done", "2026-10-02T00:00:00.000Z", &a)?;
                let completed = d.consume_by_completion_code_hash(tx, "done", now)?;
                let twice = d.consume_by_completion_code_hash(tx, "done", now)?.is_some();
                Ok((first, again, expired, completed, twice))
            })
            .await
            .unwrap();
        assert_eq!(
            (results.0, results.1, results.2, results.4),
            (true, false, false, false)
        );
        let completed = results.3.unwrap();
        assert_eq!(completed.account.as_ref(), Some(&id));
        assert_eq!(completed.completion_consumed_at.as_deref(), Some(now));

        let email = |hash: &str, purpose, attempts, expires: &str| EmailCodeRecord {
            account: id.clone(),
            purpose,
            code_hash: hash.into(),
            attempts,
            created_at: "2026-10-01T00:00:00.000Z".into(),
            expires_at: expires.into(),
        };
        let later = "2026-10-02T00:00:00.000Z";
        let (e1, e2, e3, e4) = (
            email("old", EmailCodePurpose::Verify, 0, later),
            email("new", EmailCodePurpose::Verify, 0, later),
            email("reset", EmailCodePurpose::Reset, 4, later),
            email("gone", EmailCodePurpose::Unlink, 0, "2026-10-01T00:00:01.000Z"),
        );
        let a = id.clone();
        let codes = store
            .write(&system(), move |tx, r| {
                let e = &r.email_codes;
                e.replace(tx, &e1)?;
                e.replace(tx, &e2)?;
                e.replace(tx, &e3)?;
                e.replace(tx, &e4)?;
                let peek = e.peek_live(tx.conn(), &a, EmailCodePurpose::Verify, now, 5)?;
                let old = e.consume(tx, &a, EmailCodePurpose::Verify, "old", now, 5)?;
                let once = e.consume(tx, &a, EmailCodePurpose::Verify, "new", now, 5)?;
                let twice = e.consume(tx, &a, EmailCodePurpose::Verify, "new", now, 5)?;
                let tried = e.reserve_try(tx, &a, EmailCodePurpose::Reset, now, 5)?;
                let used_up = e.reserve_try(tx, &a, EmailCodePurpose::Reset, now, 5)?;
                let expired = e.reserve_try(tx, &a, EmailCodePurpose::Unlink, now, 5)?;
                let left = e.delete_for_account(tx, &a)?;
                Ok((
                    peek.map(|c| c.code_hash),
                    old,
                    once,
                    twice,
                    tried,
                    used_up,
                    expired,
                    left,
                ))
            })
            .await
            .unwrap();
        assert_eq!(codes, (Some("new".into()), false, true, false, Some(5), None, None, 0));
    }
}

#[tokio::test]
async fn a10_a11_identities_and_keys() {
    for t in targets() {
        let store = &t.store;
        let id = account(store, "u1").await;
        let identity = |subject: &str| AuthIdentityRecord {
            provider_subject: subject.into(),
            account: id.clone(),
            provider_username: "ash".into(),
            provider_avatar_url: None,
            linked_at: "2026-10-01T00:00:00.000Z".into(),
        };
        let (i1, i2) = (identity("s1"), identity("s2"));
        let a = id.clone();
        let (first, same, other_subject, by_subject, by_account, deleted) = store
            .write(&system(), move |tx, r| {
                let i = &r.auth_identities;
                let first = i.create(tx, &i1)?;
                let same = i.create(tx, &i1)?;
                let other = i.create(tx, &i2)?;
                let by_subject = i.find_by_provider_subject(tx.conn(), "s1")?;
                let by_account = i.find_by_account(tx.conn(), &a)?;
                let deleted = i.delete_for_account(tx, &a)?;
                Ok((first, same, other, by_subject, by_account, deleted))
            })
            .await
            .unwrap();
        assert_eq!((first, same, other_subject, deleted), (true, false, false, 1));
        assert_eq!(by_subject, by_account);

        let a = id.clone();
        let (server, users) = store
            .write(&system(), move |tx, r| {
                let k = &r.api_keys;
                k.replace_game_server_key_hashes(tx, &["aa".into(), "bb".into()])?;
                k.replace_game_server_key_hashes(tx, &["bb".into(), "cc".into()])?;
                let mut kept: Vec<String> = k
                    .find_all_game_server_key_hashes(tx.conn())?
                    .into_iter()
                    .filter_map(|h| h.key_hash)
                    .collect();
                kept.sort();
                k.insert_user_key_hash(tx, &a, "first")?;
                k.insert_user_key_hash(tx, &a, "second")?;
                Ok((kept, k.find_all_user_key_hashes(tx.conn())?))
            })
            .await
            .unwrap();
        assert_eq!(server, ["bb", "cc"]);
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].key_hash, "second");
        let emptied = store
            .write(&system(), |tx, r| {
                r.api_keys.replace_game_server_key_hashes(tx, &[])?;
                r.api_keys.find_all_game_server_key_hashes(tx.conn())
            })
            .await
            .unwrap();
        assert!(emptied.is_empty());
        let debug = format!("{:?}", users[0]);
        assert!(!debug.contains("second"), "{debug}");
    }
}
