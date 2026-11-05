// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Characters, loadouts, friendships, inventory, progression and the economy (spec §7-§11.1): rows C1-C4, F1,
//! I1-I3, P1-P9 and B1-B4 of §15.2.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{account, log, player, system, targets};
use rustic_isles_store::repo::bounties::*;
use rustic_isles_store::repo::characters::*;
use rustic_isles_store::repo::chat::*;
use rustic_isles_store::repo::inventory::*;
use rustic_isles_store::repo::progression::*;
use rustic_isles_store::{AccountId, RepoError, UserId};
use serde_json::json;
use store::KeyValue;

fn character(id: &str, account: &AccountId) -> CharacterRecord {
    CharacterRecord {
        character_id: id.into(),
        account: account.clone(),
        created_date: "2026-10-01T00:00:00.000Z".into(),
        last_modified_date: "2026-10-01T00:00:00.000Z".into(),
        name: id.into(),
        update_version: 1,
        data: r#"{"a":1}"#.into(),
    }
}

#[tokio::test]
async fn c1_c4_characters_keep_order_and_owner() {
    for t in targets() {
        let store = &t.store;
        let (a, b) = (account(store, "a").await, account(store, "b").await);
        let (ca1, cb, ca2) = (character("a1", &a), character("b1", &b), character("a2", &a));
        let (wa, wb) = (a.clone(), b.clone());
        store
            .account_write(&a, &player(&a), move |tx, r| {
                r.characters.create(tx, &ca1)?;
                r.characters.create(tx, &cb)?;
                r.characters.create(tx, &ca2)?;
                r.characters.update_data_conditional(tx, "a1", &wa, r#"{"a":2}"#, 1)?;
                r.characters.update_data_conditional(tx, "a2", &wa, r#"{"a":3}"#, 2)?;
                let _ = wb;
                Ok(())
            })
            .await
            .unwrap();
        let (ra, rb) = (a.clone(), b.clone());
        let (mine, theirs, found) = store
            .read(move |conn, r| {
                Ok((
                    r.characters.find_many_by_account(conn, &ra)?,
                    r.characters.find(conn, "a1", &rb)?,
                    r.characters.find(conn, "a2", &ra)?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(
            mine.iter().map(|c| c.character_id.as_str()).collect::<Vec<_>>(),
            ["a1", "a2"]
        );
        assert_eq!(mine[0].data, r#"{"a":1}"#, "not a higher version");
        assert!(theirs.is_none());
        let found = found.unwrap();
        assert_eq!((found.data.as_str(), found.update_version), (r#"{"a":3}"#, 2));
        let player_text = format!("player:{a}");
        assert!(
            log(store)
                .await
                .iter()
                .any(|(actor, table, _, _)| actor == &player_text && table == "characters")
        );

        let key = [("character_id", KeyValue::Text("a1".into()))];
        let k2 = key.clone();
        store
            .write(&system(), move |tx, _| Ok(tx.soft_delete("characters", &key)?))
            .await
            .unwrap();
        let ra = a.clone();
        assert_eq!(
            store
                .read(move |conn, r| r.characters.find_many_by_account(conn, &ra))
                .await
                .unwrap()
                .len(),
            1
        );
        store
            .write(&system(), move |tx, _| Ok(tx.restore("characters", &k2)?))
            .await
            .unwrap();
        let ra = a.clone();
        assert_eq!(
            store
                .read(move |conn, r| r.characters.find_many_by_account(conn, &ra))
                .await
                .unwrap()
                .len(),
            2
        );
    }
}

#[tokio::test]
async fn c2_c3_loadouts_are_revision_guarded() {
    for t in targets() {
        let store = &t.store;
        let a = account(store, "a").await;
        let record = |id: &str, slots: usize, unlocked: Option<u32>| LoadoutRecord {
            character_id: id.into(),
            account: a.clone(),
            loadouts: serde_json::to_string(&vec![json!({"n": 0}); slots]).unwrap(),
            persistent: "{}".into(),
            active_index: None,
            unlocked_total_slots: unlocked,
            revision: None,
            bootstrap_version: None,
        };
        let (fresh, legacy) = (record("c1", 3, Some(2)), record("c2", 3, None));
        let w = a.clone();
        let results = store
            .write(&system(), move |tx, r| {
                let l = &r.loadouts;
                l.create(tx, &fresh)?;
                l.create(tx, &legacy)?;
                Ok((
                    l.replace_slot_if_revision_matches(tx, "c1", &w, 1, r#"{"n":1}"#, 0)?,
                    l.replace_slot_if_revision_matches(tx, "c1", &w, 1, r#"{"n":9}"#, 0)?,
                    l.replace_slot_if_revision_matches(tx, "c1", &w, 2, r#"{"n":2}"#, 1)?,
                    l.replace_all_and_entitlement_if_revision_matches(tx, "c1", &w, "[]", 1, 1)?,
                    l.replace_all_and_entitlement_if_revision_matches(tx, "c1", &w, "[1,2]", 3, 1)?,
                    l.replace_all_and_entitlement_if_revision_matches(tx, "c1", &w, "[1,2,3]", 3, 1)?,
                    l.update_active_index_if_revision_matches(tx, "c1", &w, 0, 99)?,
                    l.update_active_index_if_revision_matches(tx, "c1", &w, 2, 2)?,
                    l.update_persistent_if_revision_matches(tx, "c1", &w, r#"{"p":1}"#, 3)?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(results, (true, false, false, false, false, true, true, true, true));
        let r = a.clone();
        let stored = store
            .read(move |conn, repos| repos.loadouts.find(conn, "c1", &r))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (stored.revision, stored.active_index, stored.unlocked_total_slots),
            (Some(4), Some(2), Some(3))
        );
        assert_eq!(stored.loadouts, "[1,2,3]");
        assert_eq!(stored.persistent, r#"{"p":1}"#);

        let w = a.clone();
        let legacy = store
            .write(&system(), move |tx, r| {
                // A row from before the entitlement column: null slots, revision 0
                rustic_isles_store_test_sql(
                    tx,
                    "UPDATE loadouts SET unlocked_total_slots = NULL WHERE character_id = 'c2'",
                )?;
                r.loadouts.replace_slot_if_revision_matches(tx, "c2", &w, 1, "{}", 0)
            })
            .await
            .unwrap();
        assert!(
            legacy,
            "a legacy row of 3 shows 2 slots and stays writable at revision 0"
        );
    }
}

fn rustic_isles_store_test_sql(tx: &mut store::Tx<'_>, text: &str) -> rustic_isles_store::RepoResult<()> {
    use diesel::RunQueryDsl;
    diesel::sql_query(text).execute(tx.conn())?;
    Ok(())
}

#[tokio::test]
async fn f1_friend_edges() {
    for t in targets() {
        let store = &t.store;
        let (a, b, c) = (
            account(store, "a").await,
            account(store, "b").await,
            account(store, "c").await,
        );
        let edge = |owner: &AccountId, other: &AccountId, status| FriendEdgeRecord {
            owner: owner.clone(),
            other: other.clone(),
            other_user_id: UserId::new("x"),
            status,
            direction: Some(FriendshipDirection::Outbound),
            created: "2026-10-01T00:00:00.000Z".into(),
            favorite: false,
        };
        let (ab, ac, ba) = (
            edge(&a, &b, FriendshipStatus::Pending),
            edge(&a, &c, FriendshipStatus::Accepted),
            edge(&b, &a, FriendshipStatus::Pending),
        );
        let (wa, wb) = (a.clone(), b.clone());
        let (listed, one_sided, after_all) = store
            .write(&system(), move |tx, r| {
                let f = &r.friendships;
                f.upsert(tx, &ab)?;
                f.upsert(tx, &ac)?;
                f.upsert(tx, &ba)?;
                let mut accepted = ab.clone();
                accepted.status = FriendshipStatus::Accepted;
                accepted.favorite = true;
                f.upsert(tx, &accepted)?;
                let listed = f.list_for_owner(tx.conn(), &wa)?;
                f.remove(tx, &wa, &wb)?;
                let one_sided = (f.find(tx.conn(), &wa, &wb)?, f.find(tx.conn(), &wb, &wa)?.is_some());
                f.upsert(tx, &accepted)?;
                let removed = f.remove_all_for_account(tx, &wb)?;
                Ok((listed, one_sided, (removed, f.list_for_owner(tx.conn(), &wa)?.len())))
            })
            .await
            .unwrap();
        assert_eq!(listed[0].other, b, "an update keeps its list position ({})", t.engine);
        assert_eq!(
            (listed[0].status, listed[0].favorite),
            (FriendshipStatus::Accepted, true)
        );
        assert_eq!(one_sided, (None, true));
        assert_eq!(after_all, (2, 1));
    }
}

#[tokio::test]
async fn i1_i3_inventory_and_its_ledger() {
    for t in targets() {
        let store = &t.store;
        let a = account(store, "a").await;
        let inventory = InventoryRecord {
            character_id: "c1".into(),
            account: Some(a.clone()),
            instanced_items: "[]".into(),
            stacked_items: "[]".into(),
            revision: None,
            bootstrap_version: None,
        };
        let (updated, stale) = store
            .account_write(&a, &player(&a), move |tx, r| {
                r.inventories.create(tx, &inventory)?;
                Ok((
                    r.inventories
                        .update_both_if_revision_matches(tx, "c1", "[1]", "[2]", 0)?,
                    r.inventories
                        .update_both_if_revision_matches(tx, "c1", "[3]", "[4]", 0)?,
                ))
            })
            .await
            .unwrap();
        let updated = updated.unwrap();
        assert_eq!((updated.instanced_items.as_str(), updated.revision), ("[1]", Some(1)));
        assert!(stale.is_none());
        let player_text = format!("player:{a}");
        assert!(
            log(store)
                .await
                .iter()
                .any(|(actor, table, _, _)| actor == &player_text && table == "inventories")
        );

        let key = [("character_id", KeyValue::Text("c1".into()))];
        let k2 = key.clone();
        store
            .write(&system(), move |tx, _| Ok(tx.soft_delete("inventories", &key)?))
            .await
            .unwrap();
        assert!(
            store
                .read(|c, r| r.inventories.find_by_character_id(c, "c1"))
                .await
                .unwrap()
                .is_none()
        );
        store
            .write(&system(), move |tx, _| Ok(tx.restore("inventories", &k2)?))
            .await
            .unwrap();
        assert!(
            store
                .read(|c, r| r.inventories.find_by_character_id(c, "c1"))
                .await
                .unwrap()
                .is_some()
        );

        let w = a.clone();
        let (begun, existing, record, recent) = store
            .write(&system(), move |tx, r| {
                let l = &r.inventory_transactions;
                let k1 = LedgerKey {
                    account: &w,
                    character_id: "c1",
                    transaction_id: "t1",
                };
                let k2 = LedgerKey {
                    account: &w,
                    character_id: "c1",
                    transaction_id: "t2",
                };
                let begun = l.try_begin(tx, k1, Some("h"), "2026-10-01T00:00:00.000Z")?;
                let existing = l.try_begin(tx, k1, Some("h"), "2026-10-01T00:00:00.000Z")?;
                l.try_begin(tx, k2, None, "2026-10-02T00:00:00.000Z")?;
                let extra = json!({"economySource": "STORE", "economySessionId": 7, "status": "x", "sku": "s1"});
                l.complete(tx, k1, &json!({"ok": true}), Some(&extra), "2026-10-01T00:00:01.000Z")?;
                Ok((
                    begun,
                    existing,
                    l.find_record(tx.conn(), k1)?,
                    l.list_recent_for_account(tx.conn(), &w, 10)?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(begun, TryBegin::Begun);
        assert!(matches!(existing, TryBegin::Existing(_)));
        let record = record.unwrap();
        assert_eq!(record.status, LedgerStatus::Completed);
        assert_eq!(record.result, Some(json!({"ok": true})));
        assert_eq!(record.economy_source.as_deref(), Some("STORE"));
        assert_eq!(record.economy_session_id, None, "a non-string is dropped");
        assert_eq!(record.extra, Some(json!({"sku": "s1"})));
        assert_eq!(
            serde_json::to_string(&recent).unwrap(),
            r#"[{"transactionId":"t2","characterId":"c1","status":"pending","createdAt":"2026-10-02T00:00:00.000Z"},{"transactionId":"t1","characterId":"c1","status":"completed","createdAt":"2026-10-01T00:00:00.000Z","economySource":"STORE","completedAt":"2026-10-01T00:00:01.000Z"}]"#
        );
    }
}

fn reforge(id: &str, open: bool) -> WeaponReforge {
    WeaponReforge {
        reforge_id: id.into(),
        open,
        prior_progress: 50,
        cap: 100,
        banked: 5,
        banked_restored: 0,
        prestige_granted: false,
        opened_at: "2026-10-05T00:00:00.000Z".into(),
        closed_at: None,
    }
}

#[tokio::test]
async fn p1_p6_tracks_and_objectives() {
    for t in targets() {
        let store = &t.store;
        let a = account(store, "a").await;
        let w = a.clone();
        let out = store
            .write(&system(), move |tx, r| {
                let p = &r.progression_tracks;
                p.increment(tx, &w, "weapon", 10)?;
                p.increment(tx, &w, "hammer", 1)?;
                let t = p.increment(tx, &w, "weapon", 5)?;
                let max = p.set_progress_if_greater(tx, &w, "weapon", 3)?;
                let rank = p.set_confirmed_fremium_rank(tx, &w, "weapon", 2)?;
                let lower = p.set_confirmed_fremium_rank(tx, &w, "weapon", 1)?;
                let over = p.spend(tx, &w, "weapon", 100)?;
                let spent = p.spend(tx, &w, "weapon", 15)?;
                let order: Vec<String> = p
                    .get_all_for_account(tx.conn(), &w)?
                    .into_iter()
                    .map(|t| t.progression_id)
                    .collect();
                Ok((t, max, rank, lower, over, spent, order))
            })
            .await
            .unwrap();
        let (t1, max, rank, lower, over, spent, order) = out;
        assert_eq!((t1.progress, t1.update_version), (15, 2));
        assert_eq!(
            (max.progress, max.update_version),
            (15, 3),
            "never regresses, still counts"
        );
        assert_eq!((rank.confirmed_fremium_rank, lower.confirmed_fremium_rank), (2, 2));
        assert!(over.is_none());
        assert_eq!(spent.unwrap().progress, 0);
        assert_eq!(order, ["weapon", "hammer"]);

        // P3: a failing closure rolls back
        let w = a.clone();
        let err = store
            .write(&system(), move |tx, r| {
                r.progression_tracks.increment(tx, &w, "weapon", 1000)?;
                Err::<(), _>(RepoError::Data("stop".into()))
            })
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "stored data: stop");
        let r = a.clone();
        assert_eq!(
            store
                .read(move |c, repos| repos.progression_tracks.get(c, &r, "weapon"))
                .await
                .unwrap()
                .unwrap()
                .progress,
            0
        );

        // P4, P5
        let w = a.clone();
        let (begun, wrong, again, applied, repeat, claimed, twice, poor) = store
            .write(&system(), move |tx, r| {
                let p = &r.progression_tracks;
                p.increment(tx, &w, "sword", 50)?;
                let current = p.get(tx.conn(), &w, "sword")?.unwrap();
                let wrong = p.begin_reforge(tx, &w, "sword", 49, current.update_version, &reforge("r1", true))?;
                let begun = p.begin_reforge(tx, &w, "sword", 50, current.update_version, &reforge("r1", true))?;
                let after = p.get(tx.conn(), &w, "sword")?.unwrap();
                let again = p.begin_reforge(tx, &w, "sword", 0, after.update_version, &reforge("r2", true))?;
                let expected = json!({"status": "open", "bankedRestored": 0, "absent": null});
                let set = json!({"bankedRestored": 5});
                let applied = p.update_reforge(tx, &w, "sword", "r1", &expected, &set)?;
                let repeat = p.update_reforge(tx, &w, "sword", "r1", &json!({}), &set)?;
                p.increment(tx, &w, "sword", 30)?;
                let claimed = p.claim_prestige_levels(tx, &w, "sword", 0, 2, 30)?;
                let twice = p.claim_prestige_levels(tx, &w, "sword", 0, 2, 30)?;
                let poor = p.claim_prestige_levels(tx, &w, "sword", 2, 1, 31)?;
                Ok((begun, wrong, again, applied, repeat, claimed, twice, poor))
            })
            .await
            .unwrap();
        assert!(wrong.is_none() && again.is_none());
        let begun = begun.unwrap();
        assert_eq!(
            (begun.progress, begun.updated_at.as_str()),
            (0, "2026-10-05T00:00:00.000Z")
        );
        assert_eq!(begun.reforge, Some(reforge("r1", true)));
        assert_eq!((applied, repeat), (true, false));
        assert_eq!((claimed, twice, poor), (true, false, false));
        let r = a.clone();
        let sword = store
            .read(move |c, repos| repos.progression_tracks.get(c, &r, "sword"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (sword.prestige_claimed, sword.reforge.unwrap().banked_restored),
            (Some(2), 5)
        );

        // P6
        let w = a.clone();
        let objectives = store
            .write(&system(), move |tx, r| {
                let p = &r.progression_tracks;
                p.merge_objectives(tx, &w, &[("o2".into(), 5, 1), ("o1".into(), 3, 0)])?;
                p.merge_objectives(tx, &w, &[("o2".into(), 4, 2)])?;
                let merged = p.set_objective_if_greater(tx, &w, "o1", 1, 1)?;
                p.append_objective_event(tx, &w, "{}", "2026-10-01T00:00:00.000Z")?;
                let none = p.get_objectives(tx.conn(), &w, &[])?;
                Ok((p.get_all_objectives_for_account(tx.conn(), &w)?, merged, none))
            })
            .await
            .unwrap();
        let ids: Vec<_> = objectives
            .0
            .iter()
            .map(|o| (o.objective_id.as_str(), o.value, o.completed_count))
            .collect();
        assert_eq!(ids, [("o2", 5, 2), ("o1", 3, 1)]);
        assert_eq!((objectives.1.value, objectives.1.completed_count), (3, 1));
        assert!(objectives.2.is_empty());
    }
}

#[tokio::test]
async fn p7_p9_grants_escalation_journey_breadcrumbs() {
    for t in targets() {
        let store = &t.store;
        let (a, b) = (account(store, "a").await, account(store, "b").await);
        let w = a.clone();
        let track = store
            .write(&system(), move |tx, r| r.progression_tracks.increment(tx, &w, "xp", 7))
            .await
            .unwrap();
        let grant = ProgressionGrantRecord {
            grant_id: "g1".into(),
            account: a.clone(),
            progression_id: "xp".into(),
            amount: 7,
            request_hash: "h".into(),
            result: track.clone(),
            created_at: "2026-10-01T00:00:00.000Z".into(),
        };
        let g = grant.clone();
        store
            .write(&system(), move |tx, r| r.progression_grants.insert_applied(tx, &g))
            .await
            .unwrap();
        let g = grant.clone();
        assert!(
            store
                .write(&system(), move |tx, r| r.progression_grants.insert_applied(tx, &g))
                .await
                .is_err()
        );
        let r = a.clone();
        let (found, recent) = store
            .read(move |c, repos| {
                Ok((
                    repos.progression_grants.find_by_grant_id(c, "g1")?,
                    repos.progression_grants.list_recent_for_account(c, &r, 5)?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(found.as_ref(), Some(&grant));
        assert_eq!(recent.len(), 1);

        let escalation = EscalationProgressRecord {
            account: a.clone(),
            season_id: "s1".into(),
            escalation_level: 1,
            next_level_xp: 100,
            talents_progress: vec![json!({"t": 1})],
            unlock_progress: vec![],
            update_version: 1,
            created_at: "c".into(),
            updated_at: "u".into(),
        };
        let e = escalation.clone();
        let (first, second, stale, replaced) = store
            .write(&system(), move |tx, r| {
                let s = &r.escalation_progress;
                let first = s.save_if_version(tx, &e, None)?;
                let second = s.save_if_version(tx, &e, None)?;
                let mut next = e.clone();
                next.update_version = 2;
                let stale = s.save_if_version(tx, &next, Some(5))?;
                let replaced = s.save_if_version(tx, &next, Some(1))?;
                Ok((first.is_some(), second.is_some(), stale.is_some(), replaced.is_some()))
            })
            .await
            .unwrap();
        assert_eq!((first, second, stale, replaced), (true, false, false, true));

        let journey = PlayerJourneyRecord {
            account: a.clone(),
            nodes: "{}".into(),
            update_version: 0,
            revision: 0,
        };
        let (wa, wb) = (a.clone(), b.clone());
        let out = store
            .write(&system(), move |tx, r| {
                let j = &r.player_journey;
                let created = j.create_if_absent(tx, &journey)?;
                let twice = j.create_if_absent(tx, &journey)?;
                let at0 = j.update_if_revision(tx, &wa, r#"{"x":1}"#, 1, 0)?;
                let stale = j.update_if_revision(tx, &wa, r#"{"x":2}"#, 2, 0)?;
                r.breadcrumbs.create(
                    tx,
                    &BreadcrumbsRecord {
                        character_id: "c1".into(),
                        account: wa.clone(),
                        breadcrumbs: "[]".into(),
                        update_version: 0,
                    },
                )?;
                r.breadcrumbs.update(tx, "c1", &wa, "[1]", 1)?;
                r.encountered_content.create(
                    tx,
                    &EncounteredContentRecord {
                        character_id: "c1".into(),
                        account: wa.clone(),
                        encountered_content: "{}".into(),
                    },
                )?;
                r.encountered_content.update_content(tx, "c1", &wa, r#"{"e":1}"#)?;
                Ok((
                    created,
                    twice,
                    at0,
                    stale,
                    j.find(tx.conn(), &wa)?.map(|j| j.revision),
                    r.breadcrumbs.find(tx.conn(), "c1", &wa)?.map(|b| b.breadcrumbs),
                    r.breadcrumbs.find(tx.conn(), "c1", &wb)?,
                    r.encountered_content.find(tx.conn(), "c1", &wb)?,
                    r.encountered_content
                        .find(tx.conn(), "c1", &wa)?
                        .map(|e| e.encountered_content),
                ))
            })
            .await
            .unwrap();
        assert_eq!(
            out,
            (
                true,
                false,
                true,
                false,
                Some(1),
                Some("[1]".into()),
                None,
                None,
                Some(r#"{"e":1}"#.into())
            )
        );
    }
}

#[tokio::test]
async fn b1_b4_bounties_wallets_entitlements_rollups() {
    for t in targets() {
        let store = &t.store;
        let a = account(store, "a").await;
        let state = BountyStateRecord {
            account: a.clone(),
            bounties: vec![StoredBounty {
                group: BountyGroup::Daily,
                bounty: json!({"id": "b1"}),
                claimed_at: Some("2026-10-01T00:00:00.000Z".into()),
                reward_credit: Some(RewardCredit {
                    catalog_id: "CURRENCY".into(),
                    amount: 5.0,
                    transaction_id: "t".into(),
                    at: "x".into(),
                }),
            }],
            draft_data: DraftData {
                base: Some(json!([])),
                daily: None,
                weekly: None,
            },
            daily_reward_credited_on: None,
            daily_premium_draft_granted_on: None,
            update_version: 1,
            created_at: "c".into(),
            updated_at: "u".into(),
        };
        let s = state.clone();
        let w = a.clone();
        let (first, second, stale, saved, read) = store
            .write(&system(), move |tx, r| {
                let b = &r.bounty_states;
                let first = b.save_if_version(tx, &s, None)?;
                let second = b.save_if_version(tx, &s, None)?;
                let mut next = s.clone();
                next.update_version = 2;
                let stale = b.save_if_version(tx, &next, Some(7))?;
                let saved = b.save_if_version(tx, &next, Some(1))?;
                Ok((first.is_some(), second, stale, saved.is_some(), b.get(tx.conn(), &w)?))
            })
            .await
            .unwrap();
        assert!(first && second.is_none() && stale.is_none() && saved);
        let read = read.unwrap();
        assert_eq!((read.update_version, &read.bounties), (2, &state.bounties));
        assert_eq!(read.draft_data, state.draft_data);

        let w = a.clone();
        let cooldown = store
            .write(&system(), move |tx, r| {
                let c = CooldownStateRecord {
                    account: w.clone(),
                    entries: vec![CooldownEntry {
                        id: "c".into(),
                        started_date: "s".into(),
                        updated_at: "u".into(),
                    }],
                    update_version: 0,
                    created_at: "c".into(),
                    updated_at: "u".into(),
                };
                r.cooldown_states.save_if_version(tx, &c, None)?;
                r.cooldown_states.get(tx.conn(), &w)
            })
            .await
            .unwrap();
        assert_eq!(cooldown.unwrap().entries[0].id, "c");

        // B2
        let w = a.clone();
        let wallet = store
            .write(&system(), move |tx, r| {
                let ws = &r.wallets;
                let none = ws.increment_balance(tx, &w, "CURRENCY_RAMS", 5.0)?;
                let made = ws.find(tx.conn(), &w)?;
                ws.create(
                    tx,
                    &WalletRecord {
                        account: w.clone(),
                        balances: vec![("CURRENCY_RAMS".into(), 5000.0), ("CURRENCY_B".into(), 1.0)],
                        bootstrap_version: Some("v1".into()),
                    },
                )?;
                let kept = ws.create_if_missing(
                    tx,
                    &WalletRecord {
                        account: w.clone(),
                        balances: vec![("CURRENCY_RAMS".into(), 1.0)],
                        bootstrap_version: None,
                    },
                )?;
                let debited = ws.increment_balance(tx, &w, "CURRENCY_RAMS", -4000.0)?;
                let over = ws.increment_balance(tx, &w, "CURRENCY_RAMS", -1001.0)?;
                let credited = ws.increment_balance(tx, &w, "CURRENCY_NEW", 3.0)?;
                Ok((none, made, kept, debited, over, credited))
            })
            .await
            .unwrap();
        assert!(wallet.0.is_none() && wallet.1.is_none());
        assert_eq!(wallet.2.balance("CURRENCY_RAMS"), Some(5000.0));
        assert_eq!(wallet.3.unwrap().balance("CURRENCY_RAMS"), Some(1000.0));
        assert!(wallet.4.is_none());
        let ids: Vec<String> = wallet.5.unwrap().balances.into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, ["CURRENCY_RAMS", "CURRENCY_B", "CURRENCY_NEW"]);
        let w = a.clone();
        let bad = store
            .write(&system(), move |tx, r| {
                r.wallets.increment_balance(tx, &w, "bad.id", 1.0)
            })
            .await
            .unwrap_err();
        assert_eq!(bad.to_string(), "stored data: unsafe balance catalogId \"bad.id\"");

        // B3
        let entry = |name: &str| EntitlementEntry {
            name: name.into(),
            duration: -1.0,
            activated_date: None,
            granted_at: Some("g".into()),
            source_sku_id: None,
        };
        let w = a.clone();
        let (e1, e2, e3, held) = store
            .write(&system(), move |tx, r| {
                let e = &r.entitlements;
                Ok((
                    e.grant_if_missing(tx, &w, &entry("exchange_slot_2"))?,
                    e.grant_if_missing(tx, &w, &entry("exchange_slot_2"))?,
                    e.grant_if_missing(tx, &w, &entry("another"))?,
                    e.find(tx.conn(), &w)?,
                ))
            })
            .await
            .unwrap();
        assert_eq!((e1, e2, e3), (true, false, true));
        let names: Vec<_> = held.unwrap().entitlements.into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["another", "exchange_slot_2"]);
        let w = a.clone();
        let err = store
            .write(&system(), move |tx, r| {
                r.entitlements.grant_if_missing(tx, &w, &entry("bad name"))
            })
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "stored data: unsafe entitlement name \"bad name\"");

        // B4
        let w = a.clone();
        let doc = store
            .write(&system(), move |tx, r| {
                let e = &r.economy_rollups;
                e.add_to_rollup(
                    tx,
                    &w,
                    "2026-10-01",
                    "RAMS",
                    &[("credited".into(), 100.0), ("bySource.STORE.debited".into(), 40.0)],
                    1,
                )?;
                e.add_to_rollup(
                    tx,
                    &w,
                    "2026-10-01",
                    "RAMS",
                    &[
                        ("credited".into(), 5.0),
                        ("bySource.STORE.debited".into(), 2.0),
                        ("bySource.QUEST.credited".into(), 1.0),
                    ],
                    2,
                )
            })
            .await
            .unwrap();
        assert_eq!(
            doc.to_string(),
            r#"{"bySource":{"QUEST":{"credited":1.0},"STORE":{"debited":42.0}},"credited":105.0}"#
        );
    }
}
