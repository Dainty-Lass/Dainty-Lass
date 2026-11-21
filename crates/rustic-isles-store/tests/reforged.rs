// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The Reforged API's repositories, the erasure and the TTL purge (spec §11.3).
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{account, admin, launcher, system, targets};
use rustic_isles_store::repo::characters::CharacterRecord;
use rustic_isles_store::repo::chat::{FriendEdgeRecord, FriendshipStatus};
use rustic_isles_store::repo::reforged::*;
use rustic_isles_store::{AccountId, UserId};
use serde_json::json;

fn audit(id: &str, target: Option<&str>, at: &str) -> AdminAuditRecord {
    AdminAuditRecord {
        id: id.into(),
        actor_user_id: "owner".into(),
        target_user_id: target.map(str::to_owned),
        action: "player.update".into(),
        old_state: Some(json!({"a": 1})),
        new_state: None,
        reason: None,
        ip: "127.0.0.1".into(),
        request_id: "r".into(),
        created_at: at.into(),
    }
}

#[tokio::test]
async fn admin_sessions_audit_and_policy() {
    for t in targets() {
        let store = &t.store;
        let a = account(store, "ua").await;
        let session = AdminSessionRecord {
            id: "s1".into(),
            token_hash: "secret-hash".into(),
            account: a.clone(),
            csrf_token: "secret-csrf".into(),
            created_at: "2026-10-01T00:00:00.000Z".into(),
            expires_at: "2026-10-02T00:00:00.000Z".into(),
            ip: "127.0.0.1".into(),
            user_agent: "ua".into(),
            revoked_at: None,
        };
        let debug = format!("{session:?}");
        assert!(!debug.contains("secret"), "{debug}");
        let s = session.clone();
        let (active, revoked, page, policy_ok, policy_stale, policy) = store
            .write(&admin(), move |tx, r| {
                let ad = &r.admin;
                ad.create_session(tx, &s)?;
                let active =
                    ad.find_active_session_by_token_hash(tx.conn(), "secret-hash", "2026-10-01T12:00:00.000Z")?;
                ad.revoke_session(tx, "s1", "2026-10-01T13:00:00.000Z")?;
                let revoked =
                    ad.find_active_session_by_token_hash(tx.conn(), "secret-hash", "2026-10-01T14:00:00.000Z")?;
                for (n, target) in [(1, Some("u1")), (2, None), (3, Some("u1"))] {
                    ad.append_audit(tx, &audit(&format!("a{n}"), target, "2026-10-01T00:00:00.000Z"))?;
                }
                let page = ad.list_audit(tx.conn(), Some("u1"), 0, 1)?;
                let policy = ad.read_operations_policy(tx.conn())?.unwrap();
                let mut next = policy.clone();
                next.guard_enforcement = "OBSERVE".into();
                let ok = ad.update_operations_policy(tx, 0, &next)?;
                let stale = ad.update_operations_policy(tx, 0, &next)?;
                Ok((
                    active,
                    revoked,
                    page,
                    ok,
                    stale,
                    ad.read_operations_policy(tx.conn())?.unwrap(),
                ))
            })
            .await
            .unwrap();
        assert_eq!(active, Some(session));
        assert!(revoked.is_none());
        assert_eq!(
            (page.1, page.0[0].id.as_str()),
            (2, "a3"),
            "same time: seq descending ({})",
            t.engine
        );
        assert_eq!(page.0[0].old_state, Some(json!({"a": 1})));
        assert!(policy_ok && !policy_stale);
        assert_eq!((policy.guard_enforcement.as_str(), policy.version), ("OBSERVE", 1));
        assert_eq!(policy.diagnostics_profile, "PRODUCTION");
    }
}

#[tokio::test]
async fn notices_and_invites() {
    for t in targets() {
        let store = &t.store;
        let notices = LauncherNoticesRecord {
            version: 1,
            updated_at: None,
            updated_by: Some("owner".into()),
            announcements: vec![json!({"t": "hi"})],
            tags: vec![],
        };
        let n = notices.clone();
        let out = store
            .write(&system(), move |tx, r| {
                let l = &r.launcher_notices;
                let empty = l.get(tx.conn())?;
                let wrong_first = l.replace_if_version(tx, &n, 3)?;
                let first = l.replace_if_version(tx, &n, 0)?;
                let mut next = n.clone();
                next.version = 2;
                let stale = l.replace_if_version(tx, &next, 0)?;
                let replaced = l.replace_if_version(tx, &next, 1)?;
                Ok((empty, wrong_first, first, stale, replaced, l.get(tx.conn())?))
            })
            .await
            .unwrap();
        assert_eq!((out.0, out.1, out.2, out.3, out.4), (None, false, true, false, true));
        let stored = out.5.unwrap();
        assert_eq!((stored.version, &stored.announcements), (2, &notices.announcements));
        assert!(decode_notice_list("{}").is_empty());

        let code = |c: &str, uses, infinite, at: &str| InviteRecord {
            code: c.into(),
            uses_remaining: uses,
            infinite_uses: infinite,
            note: None,
            created_at: at.into(),
            expires_at: None,
        };
        const NOW: &str = "2026-11-08T12:00:00.000Z";
        let out = store
            .write(&system(), move |tx, r| {
                let i = &r.invites;
                i.insert_if_absent(tx, &code("one", 1, false, "2026-10-01T00:00:00.000Z"))?;
                i.insert_if_absent(tx, &code("inf", 0, true, "2026-10-02T00:00:00.000Z"))?;
                let spent = i.spend(tx, "one", NOW)?;
                let empty = i.spend(tx, "one", NOW)?;
                let unlimited = i.spend(tx, "inf", NOW)?;
                let unknown = i.spend(tx, "nope", NOW)?;
                let listed: Vec<String> = i.list(tx.conn(), 10)?.into_iter().map(|c| c.code).collect();
                Ok((
                    spent,
                    empty,
                    unlimited,
                    unknown,
                    listed,
                    i.delete(tx, "one")?,
                    i.delete(tx, "one")?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(
            out,
            (true, false, true, false, vec!["inf".into(), "one".into()], true, false)
        );
        // SQLite keeps the transaction usable after a refused insert; Postgres spoils it (spec §16 item 14)
        if t.engine == "sqlite" {
            let taken = store
                .write(&system(), move |tx, r| {
                    r.invites
                        .insert_if_absent(tx, &code("inf", 1, false, "2026-10-03T00:00:00.000Z"))
                })
                .await
                .unwrap();
            assert!(!taken);
        }

        // D4: a code expires at its `expires_at`; one without never does
        let dated = move |c: &str, at: &str| InviteRecord {
            expires_at: Some(at.into()),
            ..code(c, 5, true, "2026-11-08T00:00:00.000Z")
        };
        let out = store
            .write(&system(), move |tx, r| {
                let i = &r.invites;
                i.insert_if_absent(tx, &dated("late", "2026-11-08T12:00:00.001Z"))?;
                i.insert_if_absent(tx, &dated("gone", NOW))?;
                let late = i.spend(tx, "late", NOW)?;
                let gone = i.spend(tx, "gone", NOW)?;
                let after = i.spend(tx, "late", "2026-11-09T00:00:00.000Z")?;
                let kept = i.list(tx.conn(), 10)?.into_iter().find(|c| c.code == "late");
                Ok((late, gone, after, kept.and_then(|c| c.expires_at)))
            })
            .await
            .unwrap();
        assert_eq!(out, (true, false, false, Some("2026-11-08T12:00:00.001Z".into())));
    }
}

#[tokio::test]
async fn activity_counters_hunts_and_flags() {
    for t in targets() {
        let store = &t.store;
        let (a, op) = (account(store, "ua").await, account(store, "operator").await);
        let (w, wop) = (a.clone(), op.clone());
        let out = store
            .write(&system(), move |tx, r| {
                let ac = &r.activity;
                ac.add_to_daily(tx, &w, "2026-10-01", &[("hunts.completed".into(), 1.0)], 10)?;
                let doc = ac.add_to_daily(
                    tx,
                    &w,
                    "2026-10-01",
                    &[("hunts.completed".into(), 2.0), ("zero".into(), 0.0)],
                    10,
                )?;
                let fields = ac.set_daily_fields(tx, &w, "2026-10-02", &json!({"note": "x"}), 10)?;
                let rows = ac.find_daily_rows(tx.conn(), &w, "2026-10-01", "2026-10-02", 10)?;
                let all = ac.find_daily_rows_in_range(tx.conn(), "2026-10-01", "2026-10-01", 10)?;
                let hunt = ActivityHuntRecord {
                    id: "h1".into(),
                    account: w.clone(),
                    host: true,
                    kind: HuntKind::Dedicated,
                    hunt_id: "hid".into(),
                    hunt_ref: "ref".into(),
                    started_at: "2026-10-01T10:00:00.000Z".into(),
                    ended_at: None,
                    end_reason: None,
                    result: None,
                    kills: 0,
                    ttl_at: 10,
                };
                let first = ac.insert_hunt_if_absent(tx, &hunt)?;
                let twice = ac.insert_hunt_if_absent(tx, &hunt)?;
                ac.add_hunt_kills(tx, "h1", 3)?;
                let open = ac.find_open_dedicated_hunts(tx.conn(), &w)?.len();
                let covering = ac
                    .find_dedicated_hunts_covering(
                        tx.conn(),
                        &w,
                        "2026-10-01T11:00:00.000Z",
                        "2026-10-01T09:00:00.000Z",
                        "2026-10-01T10:30:00.000Z",
                    )?
                    .len();
                let closed = ac.close_hunt_if_open(
                    tx,
                    "h1",
                    "2026-10-01T12:00:00.000Z",
                    "done",
                    Some(HuntResult::Completed),
                )?;
                let closed_again = ac.close_hunt_if_open(tx, "h1", "x", "x", None)?;
                let hunt = ac.find_hunt(tx.conn(), "h1")?.unwrap();
                let flag = ActivityFlagRecord {
                    id: "f1".into(),
                    kind: "k".into(),
                    severity: "high".into(),
                    account: w.clone(),
                    day: "2026-10-01".into(),
                    detail: json!({"why": 1}),
                    acknowledged: false,
                    created_at: "2026-10-01T00:00:00.000Z".into(),
                    acknowledged_at: None,
                    acknowledged_by: None,
                    note: None,
                    ttl_at: 10,
                };
                ac.insert_flag_if_absent(tx, &flag)?;
                let unacked = ac.list_flags(tx.conn(), Some(false), 0, 0)?.1;
                let acked = ac
                    .acknowledge_flag(tx, "f1", &wop, "2026-10-02T00:00:00.000Z", Some("ok"))?
                    .unwrap();
                let missing = ac.acknowledge_flag(tx, "nope", &wop, "x", None)?;
                Ok((
                    doc,
                    fields,
                    rows,
                    all,
                    first,
                    twice,
                    open,
                    covering,
                    closed,
                    closed_again,
                    hunt,
                    unacked,
                    acked,
                    missing,
                ))
            })
            .await
            .unwrap();
        assert_eq!(out.0.to_string(), r#"{"day":"2026-10-01","hunts":{"completed":3.0}}"#);
        assert_eq!(out.1.to_string(), r#"{"day":"2026-10-02","note":"x"}"#);
        assert_eq!(out.2.len(), 2);
        assert_eq!(out.3.len(), 1);
        assert_eq!(
            (out.4, out.5, out.6, out.7, out.8, out.9),
            (true, false, 1, 1, true, false)
        );
        assert_eq!((out.10.kills, out.10.result), (3, Some(HuntResult::Completed)));
        assert_eq!(out.11, 1);
        assert_eq!(
            (
                out.12.acknowledged,
                out.12.acknowledged_by.as_deref(),
                out.12.note.as_deref()
            ),
            (true, Some("operator"), Some("ok"))
        );
        assert!(out.13.is_none());
    }
}

#[tokio::test]
async fn launcher_guard_sessions_and_findings() {
    for t in targets() {
        let store = &t.store;
        let a = account(store, "ua").await;
        let session = LauncherGuardSessionRecord {
            guard_session_id: "g1".into(),
            account: a.clone(),
            role: GuardRole::Client,
            public_key_base64: "k".into(),
            manifest_id: "m".into(),
            manifest_sequence: u64::MAX,
            channel: "stable".into(),
            launcher_version: "1.0".into(),
            challenge: "c0".into(),
            last_sequence: 0,
            status: GuardStatus::Challenged,
            risk_score: 0.0,
            created_at: "2026-10-01T00:00:00.000Z".into(),
            expires_at: "2026-10-01T01:00:00.000Z".into(),
            last_heartbeat_at: None,
            last_signals: None,
            reasons: vec![],
            history: vec![],
        };
        assert!(!format!("{session:?}").contains("c0"));
        let beat = |seq: u64, challenge: &str, next: &str, at: &str| GuardHeartbeat {
            guard_session_id: "g1".into(),
            account: a.clone(),
            expected_challenge: challenge.into(),
            sequence: seq,
            next_challenge: next.into(),
            status: GuardStatus::Healthy,
            risk_score: 0.5,
            heartbeat_at: at.into(),
            expires_at: "2026-10-01T02:00:00.000Z".into(),
            signals: json!({"s": seq}),
            reasons: vec!["r".into()],
            history_entry: json!({"seq": seq}),
        };
        let (b1, b_replay, b_wrong, b2) = (
            beat(1, "c0", "c1", "2026-10-01T00:10:00.000Z"),
            beat(1, "c1", "c2", "2026-10-01T00:11:00.000Z"),
            beat(2, "zz", "c2", "2026-10-01T00:11:00.000Z"),
            beat(2, "c1", "c2", "2026-10-01T00:12:00.000Z"),
        );
        let w = a.clone();
        let out = store
            .write(&system(), move |tx, r| {
                let g = &r.launcher_guard;
                g.create(tx, &session)?;
                let first = g.advance(tx, &b1)?;
                let replay = g.advance(tx, &b_replay)?;
                let wrong = g.advance(tx, &b_wrong)?;
                let second = g.advance(tx, &b2)?.unwrap();
                let healthy =
                    g.find_healthy_for_account(tx.conn(), &w, "g1", GuardRole::Client, "2026-10-01T00:12:30.000Z")?;
                let late =
                    g.find_healthy_for_account(tx.conn(), &w, "g1", GuardRole::Client, "2026-10-01T00:13:00.000Z")?;
                let bad_now = g.find_healthy_for_account(tx.conn(), &w, "g1", GuardRole::Client, "not a time");
                let listed = g.list_sessions(tx.conn(), Some(GuardStatus::Healthy), Some(0.5), 0, 0)?;
                g.revoke_for_account(tx, &w, GuardRole::Client, "2026-10-01T00:20:00.000Z")?;
                let revoked = g.find(tx.conn(), "g1")?.unwrap();
                Ok((
                    first.is_some(),
                    replay,
                    wrong,
                    second,
                    healthy.is_some(),
                    late.is_some(),
                    bad_now.is_err(),
                    listed,
                    revoked,
                ))
            })
            .await
            .unwrap();
        assert!(out.0 && out.1.is_none() && out.2.is_none());
        let second = out.3;
        assert_eq!((second.last_sequence, second.challenge.as_str()), (2, "c2"));
        assert_eq!(second.history, vec![json!({"seq": 2}), json!({"seq": 1})]);
        assert_eq!(
            second.manifest_sequence,
            i64::MAX as u64,
            "a u64 above i64::MAX is stored as i64::MAX"
        );
        assert_eq!(second.last_signals, Some(json!({"s": 2})));
        assert!(out.4 && !out.5 && out.6);
        assert_eq!(out.7.1, 1);
        assert!(out.7.0[0].history.is_empty(), "lists drop histories");
        assert_eq!(
            (out.8.status, out.8.expires_at.as_str()),
            (GuardStatus::Revoked, "2026-10-01T00:20:00.000Z")
        );

        let sighting = |account: &AccountId, new_in_session: bool| GuardSighting {
            file_name: "x.dll".into(),
            sha256: "ab".into(),
            signer: "Some_Signer".into(),
            size: 10,
            account: account.clone(),
            seen_at: "2026-10-01T00:00:00.000Z".into(),
            new_in_session,
        };
        let b = account(store, "ub").await;
        let (s1, s2, s3) = (sighting(&a, true), sighting(&a, false), sighting(&b, true));
        let found = store
            .write(&system(), move |tx, r| {
                let g = &r.launcher_guard;
                g.record_finding_sighting(tx, &s1)?;
                g.record_finding_sighting(tx, &s2)?;
                g.record_finding_sighting(tx, &s3)?;
                Ok((
                    g.list_findings(tx.conn(), Some("some_s"), 0, 0)?,
                    g.list_findings(tx.conn(), Some("some%"), 0, 0)?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(found.0.1, 1);
        assert_eq!((found.0.0[0].session_count, found.0.0[0].account_count), (2, 2));
        assert_eq!(found.1.1, 0, "% is escaped");
    }
}

#[tokio::test]
async fn delete_player_data_erases_every_area_and_purge_drops_ttl_rows() {
    for t in targets() {
        let store = &t.store;
        let a = AccountId::new(store.node());
        let other = account(store, "other").await;
        let record = launcher(&a, "ua", Some("a@x.org"), "Ash", "2026-10-01T00:00:00.000Z");
        let o = other.clone();
        let w = a.clone();
        store
            .write(&system(), move |tx, r| {
                r.launcher_accounts.create(tx, &record)?;
                r.characters.create(
                    tx,
                    &CharacterRecord {
                        character_id: "c1".into(),
                        account: w.clone(),
                        created_date: "d".into(),
                        last_modified_date: "d".into(),
                        name: "n".into(),
                        update_version: 0,
                        data: "{}".into(),
                    },
                )?;
                r.progression_tracks.increment(tx, &w, "xp", 1)?;
                r.friendships.upsert(
                    tx,
                    &FriendEdgeRecord {
                        owner: o.clone(),
                        other: w.clone(),
                        other_user_id: UserId::new("ua"),
                        status: FriendshipStatus::Accepted,
                        direction: None,
                        created: "c".into(),
                        favorite: false,
                    },
                )?;
                r.activity.add_to_daily(tx, &w, "2026-10-01", &[("x".into(), 1.0)], 5)?;
                r.launcher_guard.record_finding_sighting(
                    tx,
                    &GuardSighting {
                        file_name: "f".into(),
                        sha256: "s".into(),
                        signer: "x".into(),
                        size: 1,
                        account: w.clone(),
                        seen_at: "2026-10-01T00:00:00.000Z".into(),
                        new_in_session: true,
                    },
                )?;
                r.parties.get_or_create_solo(
                    tx,
                    &w,
                    "b",
                    &rustic_isles_store::repo::parties::NewParty {
                        id: dainty_module_api::GlobalId::new(
                            dainty_module_api::IdKind::Party,
                            &dainty_module_api::NodeId::new("local").unwrap(),
                        ),
                        party_id: "p".into(),
                    },
                    "now",
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let w = a.clone();
        let deletion = store
            .account_write(&a, &admin(), move |tx, r| {
                r.admin
                    .delete_player_data(tx, &w, "ua", &audit("del", Some("ua"), "2026-10-01T00:00:00.000Z"))
            })
            .await
            .unwrap();
        let counts: Vec<(&str, u64)> = deletion.deleted_counts.iter().map(|(n, c)| (n.as_str(), *c)).collect();
        let names: Vec<&str> = counts.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            [
                "accounts",
                "launcherAccounts",
                "adminSessions",
                "launcherGuardSessions",
                "guardFindings",
                "playerActivityDaily",
                "activityHunts",
                "slayersPathUnlocks",
                "activityFlags",
                "characters",
                "loadouts",
                "inventories",
                "inventoryTransactions",
                "breadcrumbs",
                "encounteredContent",
                "progressionTracks",
                "progressionObjectives",
                "progressionObjectiveEvents",
                "progressionGrants",
                "escalationProgress",
                "playerJourney",
                "bountyStates",
                "cooldownStates",
                "entitlements",
                "walletBalances",
                "wallets",
                "economyRollups",
                "friends",
                "partyInvites",
                "parties",
            ]
        );
        let get = |name: &str| counts.iter().find(|(n, _)| *n == name).map(|(_, c)| *c);
        assert_eq!(
            (
                get("accounts"),
                get("launcherAccounts"),
                get("guardFindings"),
                get("playerActivityDaily")
            ),
            (Some(1), Some(1), Some(1), Some(1))
        );
        assert_eq!(
            (get("characters"), get("progressionTracks"), get("friends")),
            (Some(1), Some(1), Some(1))
        );
        assert_eq!(
            get("parties"),
            Some(2),
            "a membership and the emptied party ({})",
            t.engine
        );
        assert_eq!(deletion.character_count, 1);
        let r = a.clone();
        let (audit_total, gone) = store
            .read(move |conn, repos| {
                Ok((
                    repos.admin.list_audit(conn, Some("ua"), 0, 0)?.1,
                    repos.accounts.find(conn, &r)?,
                ))
            })
            .await
            .unwrap();
        assert_eq!((audit_total, gone), (1, None));

        let w = other.clone();
        let purged = store
            .write(&system(), move |tx, r| {
                r.activity.add_to_daily(tx, &w, "2026-10-01", &[("x".into(), 1.0)], 5)?;
                r.economy_rollups
                    .add_to_rollup(tx, &w, "d", "c", &[("x".into(), 1.0)], 100)?;
                purge_expired_ttl_rows(tx, 5)
            })
            .await
            .unwrap();
        assert_eq!(purged, 1, "only ttl_at <= now");
    }
}
