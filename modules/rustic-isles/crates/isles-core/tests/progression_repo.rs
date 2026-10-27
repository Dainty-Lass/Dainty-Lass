//! The progression repositories on `store` (DD15), ported from `persistence/sqlite/repositories.progression.test.ts`:
//! tracks and their guards, the reforge compare-and-set, prestige claims, objectives, the exactly-once grant ledger,
//! escalation's versioned save, the Slayer's Path revision and the character-scoped rows. SQLite always; Postgres when
//! `REFORGED_TEST_POSTGRES_URL` names a server the tests may create databases on, skipped otherwise.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use diesel::Connection as _;
use diesel::connection::SimpleConnection;
use isles_core::db::IslesStore;
use isles_core::repo::progression::{
    BreadcrumbsRecord, EncounteredContentRecord, EscalationProgressRecord, PlayerJourneyRecord, ProgressionGrantRecord,
    WeaponReforge,
};
use isles_core::repo::{RepoError, RepoResult};
use isles_core::{AccountId, UserId};
use module_api::network::StandaloneHome;
use module_api::{Actor, ActorContext, NodeIdentity};
use serde_json::json;
use store::{DatabaseConfig, DatabaseOptions, PostgresConfig};

const TRACK: &str = "ExperienceTrack_Weapon_Axe";

fn actor() -> ActorContext {
    ActorContext::new(Actor::Admin { key: "owner".into() })
}

fn open(config: &DatabaseConfig) -> IslesStore {
    let node = NodeIdentity::standalone();
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    IslesStore::open(config, &DatabaseOptions::default(), home, &node).unwrap()
}

fn postgres_server() -> Option<PostgresConfig> {
    let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
    let rest = url
        .strip_prefix("postgres://")
        .expect("REFORGED_TEST_POSTGRES_URL is a postgres:// URL");
    let (login, location) = rest.rsplit_once('@')?;
    let (user, password) = login.split_once(':').unwrap_or((login, ""));
    let (hostport, database) = location.split_once('/').unwrap_or((location, "postgres"));
    let (host, port) = hostport.split_once(':').unwrap_or((hostport, "5432"));
    Some(PostgresConfig {
        host: host.to_owned(),
        port: port.parse().unwrap(),
        user: user.to_owned(),
        database: database.to_owned(),
        password: password.to_owned(),
        allow_remote: false,
    })
}

static NEXT: AtomicU32 = AtomicU32::new(0);

struct Throwaway {
    server: PostgresConfig,
    name: String,
}

impl Throwaway {
    fn create(server: PostgresConfig) -> Self {
        let name = format!(
            "isles_prog_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        diesel::PgConnection::establish(&server.conninfo())
            .expect("REFORGED_TEST_POSTGRES_URL is reachable")
            .batch_execute(&format!("CREATE DATABASE \"{name}\""))
            .unwrap();
        Self { server, name }
    }

    fn config(&self) -> DatabaseConfig {
        DatabaseConfig::Postgres(PostgresConfig {
            database: self.name.clone(),
            ..self.server.clone()
        })
    }
}

impl Drop for Throwaway {
    fn drop(&mut self) {
        if let Ok(mut admin) = diesel::PgConnection::establish(&self.server.conninfo()) {
            let _ = admin.batch_execute(&format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", self.name));
        }
    }
}

/// The checks on a fresh database of each engine.
async fn on_each_engine<F, Fut>(check: F)
where
    F: Fn(IslesStore) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let dir = tempfile::tempdir().unwrap();
    check(open(&DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    }))
    .await;
    if let Some(server) = postgres_server() {
        let database = Throwaway::create(server);
        check(open(&database.config())).await;
    }
}

async fn account(store: &IslesStore, user: &str) -> AccountId {
    let id = AccountId::new(store.node());
    let record = isles_core::repo::auth::AccountRecord {
        id: id.clone(),
        user_id: UserId(user.to_owned()),
        name: user.to_owned(),
        notes: 0,
    };
    store
        .account_write(&id, &actor(), move |tx, repos| repos.accounts.create(tx, &record))
        .await
        .unwrap();
    id
}

fn reforge(id: &str) -> WeaponReforge {
    WeaponReforge {
        reforge_id: id.into(),
        open: true,
        prior_progress: 5000,
        cap: 4000,
        banked: 1000,
        banked_restored: 0,
        prestige_granted: false,
        opened_at: "2026-10-13T12:00:00.000Z".into(),
        closed_at: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tracks_accumulate_count_versions_and_keep_their_order() {
    on_each_engine(|store| async move {
        let u1 = account(&store, "u1").await;
        let u2 = account(&store, "u2").await;
        let (a, b) = (u1.clone(), u2.clone());
        let read = |id: AccountId, track: &'static str| {
            let store = store.clone();
            async move {
                store
                    .read(move |conn, repos| repos.progression_tracks.get(conn, &id, track))
                    .await
                    .unwrap()
            }
        };
        assert_eq!(read(u1.clone(), TRACK).await, None);
        let first = store
            .account_write(&u1, &actor(), {
                let a = a.clone();
                move |tx, repos| repos.progression_tracks.increment(tx, &a, TRACK, 250)
            })
            .await
            .unwrap();
        assert_eq!((first.progress, first.update_version), (250, 1));
        assert_eq!((first.confirmed_fremium_rank, first.confirmed_premium_rank), (0, 0));
        assert!(first.reforge.is_none() && first.prestige_claimed.is_none());
        assert!(!first.created_at.is_empty() && !first.updated_at.is_empty());
        for _ in 0..25 {
            let a = a.clone();
            store
                .account_write(&u1, &actor(), move |tx, repos| {
                    repos.progression_tracks.increment(tx, &a, TRACK, 2)
                })
                .await
                .unwrap();
        }
        let after = read(u1.clone(), TRACK).await.unwrap();
        assert_eq!(
            (after.progress, after.update_version, &after.created_at),
            (300, 26, &first.created_at)
        );
        store
            .account_write(&u1, &actor(), move |tx, repos| {
                repos.progression_tracks.increment(tx, &a, "Other", 1)
            })
            .await
            .unwrap();
        store
            .account_write(&u2, &actor(), move |tx, repos| {
                repos.progression_tracks.increment(tx, &b, TRACK, 1)
            })
            .await
            .unwrap();
        let ids: Vec<String> = store
            .read({
                let u1 = u1.clone();
                move |conn, repos| repos.progression_tracks.get_all_for_account(conn, &u1)
            })
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.progression_id)
            .collect();
        assert_eq!(ids, [TRACK, "Other"]);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn max_guards_never_regress_and_spend_never_goes_negative() {
    on_each_engine(|store| async move {
        let u1 = account(&store, "u1").await;
        let owner = u1.clone();
        let (progress, rank, spent) = store
            .account_write(&u1, &actor(), move |tx, repos| -> RepoResult<_> {
                let t = &repos.progression_tracks;
                assert_eq!(t.set_progress_if_greater(tx, &owner, "Mastery_A", 5)?.progress, 5);
                assert_eq!(t.set_progress_if_greater(tx, &owner, "Mastery_A", 3)?.progress, 5);
                let higher = t.set_progress_if_greater(tx, &owner, "Mastery_A", 9)?;
                assert_eq!((higher.progress, higher.update_version), (9, 3));
                let ranked = t.set_confirmed_fremium_rank(tx, &owner, "Rank_B", 4)?;
                assert_eq!((ranked.confirmed_fremium_rank, ranked.progress), (4, 0));
                assert_eq!(
                    t.set_confirmed_fremium_rank(tx, &owner, "Rank_B", 2)?
                        .confirmed_fremium_rank,
                    4
                );
                t.increment(tx, &owner, "Banked", 100)?;
                let spent = (
                    t.spend(tx, &owner, "Banked", 60)?.map(|r| r.progress),
                    t.spend(tx, &owner, "Banked", 41)?.map(|r| r.progress),
                    t.spend(tx, &owner, "Nothing", 1)?.map(|r| r.progress),
                    t.get(tx.conn(), &owner, "Banked")?.map(|r| r.progress),
                );
                Ok((higher.progress, ranked.confirmed_fremium_rank, spent))
            })
            .await
            .unwrap();
        assert_eq!((progress, rank), (9, 4));
        assert_eq!(spent, (Some(40), None, None, Some(40)));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_that_fails_rolls_back_with_everything_it_did() {
    on_each_engine(|store| async move {
        let u1 = account(&store, "u1").await;
        let owner = u1.clone();
        let failed = store
            .account_write(&u1, &actor(), move |tx, repos| -> RepoResult<()> {
                repos.progression_tracks.increment(tx, &owner, "SkuRankSkip", 5)?;
                Err(RepoError::Data("crash".into()))
            })
            .await;
        assert!(failed.is_err());
        let owner = u1.clone();
        let held = store
            .read(move |conn, repos| repos.progression_tracks.get(conn, &owner, "SkuRankSkip"))
            .await
            .unwrap();
        assert_eq!(held, None);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_reforge_reset_needs_the_exact_state_and_the_follow_ups_apply_once_each() {
    on_each_engine(|store| async move {
        let u1 = account(&store, "u1").await;
        let owner = u1.clone();
        store
            .account_write(&u1, &actor(), move |tx, repos| -> RepoResult<()> {
                let t = &repos.progression_tracks;
                let before = t.increment(tx, &owner, TRACK, 5000)?;
                t.set_confirmed_fremium_rank(tx, &owner, TRACK, 7)?;
                let current = t.get(tx.conn(), &owner, TRACK)?.unwrap();
                assert!(
                    t.begin_reforge(tx, &owner, TRACK, 4999, current.update_version, &reforge("rf-1"))?
                        .is_none()
                );
                assert!(
                    t.begin_reforge(tx, &owner, TRACK, 5000, before.update_version, &reforge("rf-1"))?
                        .is_none()
                );
                assert!(
                    t.begin_reforge(tx, &owner, "Missing", 0, 0, &reforge("rf-1"))?
                        .is_none()
                );
                let reset = t
                    .begin_reforge(tx, &owner, TRACK, 5000, current.update_version, &reforge("rf-1"))?
                    .unwrap();
                assert_eq!((reset.progress, reset.confirmed_fremium_rank), (0, 0));
                assert_eq!(reset.update_version, current.update_version + 1);
                assert_eq!(reset.updated_at, "2026-10-13T12:00:00.000Z");
                assert_eq!(reset.reforge, Some(reforge("rf-1")));
                assert!(
                    t.begin_reforge(tx, &owner, TRACK, 0, reset.update_version, &reforge("rf-2"))?
                        .is_none()
                );

                let mut once = |expected, set| t.update_reforge(tx, &owner, TRACK, "rf-1", &expected, &set);
                assert!(once(
                    json!({ "prestigeGranted": false }),
                    json!({ "prestigeGranted": true })
                )?);
                assert!(!once(
                    json!({ "prestigeGranted": false }),
                    json!({ "prestigeGranted": true })
                )?);
                assert!(!once(json!({}), json!({ "prestigeGranted": true }))?);
                assert!(!t.update_reforge(
                    tx,
                    &owner,
                    TRACK,
                    "other",
                    &json!({}),
                    &json!({ "bankedRestored": 1000 })
                )?);
                assert!(!t.update_reforge(
                    tx,
                    &owner,
                    "Missing",
                    "rf-1",
                    &json!({}),
                    &json!({ "bankedRestored": 1 })
                )?);
                assert!(t.update_reforge(
                    tx,
                    &owner,
                    TRACK,
                    "rf-1",
                    &json!({ "bankedRestored": 0 }),
                    &json!({ "bankedRestored": 1000, "status": "closed", "closedAt": "2026-10-13T12:05:00.000Z" })
                )?);
                let closed = t.get(tx.conn(), &owner, TRACK)?.unwrap().reforge.unwrap();
                assert!(!closed.open && closed.prestige_granted);
                assert_eq!(
                    (closed.banked_restored, closed.closed_at.as_deref()),
                    (1000, Some("2026-10-13T12:05:00.000Z"))
                );
                // a closed reforge no longer blocks the next one
                let next = t.get(tx.conn(), &owner, TRACK)?.unwrap();
                assert!(
                    t.begin_reforge(tx, &owner, TRACK, next.progress, next.update_version, &reforge("rf-2"))?
                        .is_some()
                );
                Ok(())
            })
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn prestige_levels_are_claimed_once_with_enough_progress() {
    on_each_engine(|store| async move {
        let u1 = account(&store, "u1").await;
        let owner = u1.clone();
        store
            .account_write(&u1, &actor(), move |tx, repos| -> RepoResult<()> {
                let t = &repos.progression_tracks;
                let id = "PrestigeTrack_Weapon_Sword";
                t.increment(tx, &owner, id, 3)?;
                assert!(!t.claim_prestige_levels(tx, &owner, id, 0, 2, 4)?);
                assert!(t.claim_prestige_levels(tx, &owner, id, 0, 2, 2)?);
                assert!(!t.claim_prestige_levels(tx, &owner, id, 0, 2, 2)?);
                assert!(!t.claim_prestige_levels(tx, &owner, id, 1, 1, 2)?);
                assert!(t.claim_prestige_levels(tx, &owner, id, 2, 1, 3)?);
                let track = t.get(tx.conn(), &owner, id)?.unwrap();
                assert_eq!((track.prestige_claimed, track.progress), (Some(3), 3));
                assert!(!t.claim_prestige_levels(tx, &owner, "Nope", 0, 1, 0)?);
                Ok(())
            })
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn objectives_merge_with_max_guards_in_the_order_they_arrived() {
    on_each_engine(|store| async move {
        let u1 = account(&store, "u1").await;
        let owner = u1.clone();
        let rows = store
            .account_write(&u1, &actor(), move |tx, repos| -> RepoResult<_> {
                let t = &repos.progression_tracks;
                let first = t.set_objective_if_greater(tx, &owner, "Zed", 5, 1)?;
                assert_eq!((first.value, first.completed_count), (5, 1));
                let lower = t.set_objective_if_greater(tx, &owner, "Zed", 3, 0)?;
                assert_eq!((lower.value, lower.completed_count), (5, 1));
                t.merge_objectives(
                    tx,
                    &owner,
                    &[("Alpha".into(), 2, 0), ("Zed".into(), 9, 1), ("Mid".into(), 1, 1)],
                )?;
                t.append_objective_event(tx, &owner, r#"{"objectives":[]}"#, "2026-10-13T12:00:00.000Z")?;
                let named = t.get_objectives(tx.conn(), &owner, &["Mid".into(), "Nothing".into()])?;
                assert_eq!(named.len(), 1);
                assert!(t.get_objectives(tx.conn(), &owner, &[])?.is_empty());
                t.get_all_objectives_for_account(tx.conn(), &owner)
            })
            .await
            .unwrap();
        let shown: Vec<_> = rows
            .iter()
            .map(|r| (r.objective_id.as_str(), r.value, r.completed_count))
            .collect();
        assert_eq!(shown, [("Zed", 9, 1), ("Alpha", 2, 0), ("Mid", 1, 1)]);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_grant_ledger_is_exactly_once() {
    on_each_engine(|store| async move {
        let u1 = account(&store, "u1").await;
        let owner = u1.clone();
        let outcome = store
            .account_write(&u1, &actor(), move |tx, repos| -> RepoResult<_> {
                let result = repos.progression_tracks.increment(tx, &owner, TRACK, 98)?;
                let record = ProgressionGrantRecord {
                    grant_id: "g1".into(),
                    account: owner.clone(),
                    progression_id: TRACK.into(),
                    amount: 98,
                    request_hash: "abc".into(),
                    result: result.clone(),
                    created_at: "2026-10-13T12:00:00.000Z".into(),
                };
                repos.progression_grants.insert_applied(tx, &record)?;
                let found = repos.progression_grants.find_by_grant_id(tx.conn(), "g1")?.unwrap();
                assert_eq!((found.amount, found.request_hash.as_str()), (98, "abc"));
                assert_eq!(found.result.progress, 98);
                assert_eq!(found.result.reforge, None);
                assert_eq!(
                    repos
                        .progression_grants
                        .list_recent_for_account(tx.conn(), &owner, 5)?
                        .len(),
                    1
                );
                Ok(record)
            })
            .await
            .unwrap();
        // a second insert of the same grant id fails, and its transaction takes the XP back with it
        let (owner, record) = (u1.clone(), outcome);
        let again = store
            .account_write(&u1, &actor(), move |tx, repos| -> RepoResult<()> {
                repos.progression_tracks.increment(tx, &owner, TRACK, 98)?;
                repos.progression_grants.insert_applied(tx, &record)
            })
            .await;
        assert!(again.is_err());
        let owner = u1.clone();
        let held = store
            .read(move |conn, repos| repos.progression_tracks.get(conn, &owner, TRACK))
            .await
            .unwrap();
        assert_eq!(held.unwrap().progress, 98);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn escalation_saves_insert_only_or_replace_at_the_expected_version() {
    on_each_engine(|store| async move {
        let u1 = account(&store, "u1").await;
        let owner = u1.clone();
        store
            .account_write(&u1, &actor(), move |tx, repos| -> RepoResult<()> {
                let e = &repos.escalation_progress;
                let record = |version, level| EscalationProgressRecord {
                    account: owner.clone(),
                    season_id: "ESC_SEASON_6".into(),
                    escalation_level: level,
                    next_level_xp: 10,
                    talents_progress: vec![json!({ "talent_id": "T", "rank": 1 })],
                    unlock_progress: vec![],
                    update_version: version,
                    created_at: "t0".into(),
                    updated_at: "t1".into(),
                };
                assert_eq!(e.save_if_version(tx, &record(1, 1), None)?, Some(record(1, 1)));
                assert_eq!(
                    e.save_if_version(tx, &record(1, 2), None)?,
                    None,
                    "insert-only when the row exists"
                );
                assert_eq!(e.save_if_version(tx, &record(2, 3), Some(7))?, None, "a stale version");
                assert_eq!(e.save_if_version(tx, &record(2, 3), Some(1))?, Some(record(2, 3)));
                assert_eq!(e.get(tx.conn(), &owner, "ESC_SEASON_6")?, Some(record(2, 3)));
                assert_eq!(e.get(tx.conn(), &owner, "ESC_SEASON_7")?, None);
                assert_eq!(
                    e.save_if_version(tx, &record(1, 1), Some(1))?
                        .map(|r| r.escalation_level),
                    None
                );
                Ok(())
            })
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_journey_updates_only_at_its_revision_and_character_rows_belong_to_their_account() {
    on_each_engine(|store| async move {
        let u1 = account(&store, "u1").await;
        let u2 = account(&store, "u2").await;
        let (a, b) = (u1.clone(), u2.clone());
        store
            .account_write(&u1, &actor(), move |tx, repos| -> RepoResult<()> {
                let j = &repos.player_journey;
                let record = PlayerJourneyRecord {
                    account: a.clone(),
                    nodes: "{}".into(),
                    update_version: 2,
                    revision: 0,
                };
                assert!(j.create_if_absent(tx, &record)?);
                assert!(!j.create_if_absent(tx, &record)?);
                assert!(!j.update_if_revision(tx, &a, r#"{"A":1}"#, 3, 5)?);
                assert!(j.update_if_revision(tx, &a, r#"{"A":1}"#, 3, 0)?);
                assert!(!j.update_if_revision(tx, &a, r#"{"A":2}"#, 4, 0)?);
                let held = j.find(tx.conn(), &a)?.unwrap();
                assert_eq!(
                    (held.nodes.as_str(), held.update_version, held.revision),
                    (r#"{"A":1}"#, 3, 1)
                );
                assert!(j.find(tx.conn(), &b)?.is_none());

                let crumbs = &repos.breadcrumbs;
                crumbs.create(
                    tx,
                    &BreadcrumbsRecord {
                        character_id: "c1".into(),
                        account: a.clone(),
                        breadcrumbs: "[]".into(),
                        update_version: 0,
                    },
                )?;
                crumbs.update(tx, "c1", &a, r#"["x"]"#, 4)?;
                assert_eq!(
                    crumbs
                        .find(tx.conn(), "c1", &a)?
                        .map(|c| (c.breadcrumbs, c.update_version)),
                    Some((r#"["x"]"#.to_owned(), 4))
                );
                assert!(
                    crumbs.find(tx.conn(), "c1", &b)?.is_none(),
                    "another account's character is not found"
                );
                crumbs.update(tx, "c1", &b, "[]", 9)?;
                assert_eq!(crumbs.find(tx.conn(), "c1", &a)?.map(|c| c.update_version), Some(4));

                let seen = &repos.encountered_content;
                seen.create(
                    tx,
                    &EncounteredContentRecord {
                        character_id: "c1".into(),
                        account: a.clone(),
                        encountered_content: "[]".into(),
                    },
                )?;
                seen.update_content(tx, "c1", &a, r#"[{"content":"X","category":2}]"#)?;
                assert_eq!(
                    seen.find(tx.conn(), "c1", &a)?.map(|c| c.encountered_content),
                    Some(r#"[{"content":"X","category":2}]"#.to_owned())
                );
                assert!(seen.find(tx.conn(), "c1", &b)?.is_none());
                Ok(())
            })
            .await
            .unwrap();
        let log = store.database().changes(0, 1000).await.unwrap();
        assert!(
            log.iter()
                .any(|e| e.table_name == "player_journey" && e.actor == "admin:owner")
        );
    })
    .await;
}
