//! RU5 bounties-store repositories on `store`'s two engines (DD15): the optimistic version check, the guarded wallet
//! increment, idempotent entitlements and the rollup's counters. Postgres runs when `REFORGED_TEST_POSTGRES_URL` names a
//! server.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use isles_core::AccountId;
use isles_core::areas::bounties_store::harness::{DatabaseConfig, postgres_target};
use isles_core::areas::bounties_store::wallet::{WalletError, add_currency, spend_platinum, starter_wallet};
use isles_core::db::IslesStore;
use isles_core::repo::bounties::{BountyStateRecord, DraftData, EntitlementEntry, WalletRecord};
use isles_core::repo::{RepoError, Repositories};
use module_api::network::StandaloneHome;
use module_api::{Actor, ActorContext, NodeIdentity};
use store::{DatabaseOptions, KeyValue};

fn open(config: &DatabaseConfig) -> IslesStore {
    let node = NodeIdentity::standalone();
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    IslesStore::open(config, &DatabaseOptions::default(), home, &node).unwrap()
}

fn actor() -> ActorContext {
    ActorContext::new(Actor::System)
}

async fn each_engine(run: impl AsyncFn(IslesStore)) {
    let dir = tempfile::tempdir().unwrap();
    run(open(&DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    }))
    .await;
    if let Some(pg) = postgres_target() {
        run(open(&pg.config)).await;
    }
}

fn account() -> AccountId {
    AccountId::new(&NodeIdentity::standalone().node)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_versioned_save_inserts_once_and_updates_only_the_version_it_expects() {
    each_engine(async |store| {
        let id = account();
        let mut record = BountyStateRecord {
            account: id.clone(),
            bounties: vec![],
            draft_data: DraftData::default(),
            daily_reward_credited_on: None,
            daily_premium_draft_granted_on: None,
            update_version: 1,
            created_at: "2026-10-10T00:00:00.000Z".into(),
            updated_at: "2026-10-10T00:00:00.000Z".into(),
        };
        let (a, r) = (id.clone(), record.clone());
        let outcomes = store
            .account_write(&id, &actor(), move |tx, repos| {
                let first = repos.bounty_states.save_if_version(tx, &r, None)?;
                // an insert-only save finds the row now
                let second = repos.bounty_states.save_if_version(tx, &r, None)?;
                let mut next = r.clone();
                next.update_version = 2;
                let stale = repos.bounty_states.save_if_version(tx, &next, Some(7))?;
                let ok = repos.bounty_states.save_if_version(tx, &next, Some(1))?;
                let again = repos.bounty_states.save_if_version(tx, &next, Some(1))?;
                let _ = &a;
                Ok((
                    first.is_some(),
                    second.is_some(),
                    stale.is_some(),
                    ok.is_some(),
                    again.is_some(),
                ))
            })
            .await
            .unwrap();
        assert_eq!(outcomes, (true, false, false, true, false));
        record.update_version = 2;
        let a = id.clone();
        let stored = store
            .read(move |c, r| r.bounty_states.get(c, &a))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.update_version, 2);

        // a deleted account's state is soft-deleted (DD23): invisible to the reads, there to revert
        let (a, key) = (id.clone(), id.to_string());
        store
            .account_write(&id, &actor(), move |tx, _| {
                assert!(tx.soft_delete("bounty_states", &[("account_id", KeyValue::Text(key))])?);
                Ok(())
            })
            .await
            .unwrap();
        assert!(
            store
                .read(move |c, r| r.bounty_states.get(c, &a))
                .await
                .unwrap()
                .is_none()
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_debit_never_goes_below_zero_and_a_missing_wallet_is_not_created_by_it() {
    each_engine(async |store| {
        let id = account();
        let a = id.clone();
        let result = store
            .account_write(&id, &actor(), move |tx, repos: &Repositories| {
                // no wallet yet: the guarded increment answers None
                let none = repos.wallets.increment_balance(tx, &a, "CURRENCY_NOTES", 5.0)?;
                assert!(none.is_none());
                let created = repos.wallets.create_if_missing(tx, &starter_wallet(&a))?;
                assert_eq!(created.balance("CURRENCY_NOTES"), Some(5000.0));
                // an existing wallet is adopted, never reset
                let mut other = starter_wallet(&a);
                other.balances = vec![("CURRENCY_NOTES".into(), 1.0)];
                let adopted = repos.wallets.create_if_missing(tx, &other)?;
                assert_eq!(adopted.balance("CURRENCY_NOTES"), Some(5000.0));
                assert_eq!(add_currency(repos, tx, &a, "CURRENCY_NOTES", -4000.0).unwrap(), 1000.0);
                let over = add_currency(repos, tx, &a, "CURRENCY_NOTES", -1001.0);
                assert!(matches!(over, Err(WalletError::Insufficient { current, .. }) if current == 1000.0));
                // a debit of a currency never held is insufficient, not a negative balance
                assert!(matches!(
                    add_currency(repos, tx, &a, "CURRENCY_S19_COIN", -1.0),
                    Err(WalletError::Insufficient { .. })
                ));
                // a credit appends the new key at the end, keeping the order of the others
                add_currency(repos, tx, &a, "CURRENCY_S19_COIN", 3.0).unwrap();
                add_currency(repos, tx, &a, "CURRENCY_NOTES", 0.5).unwrap();
                let order: Vec<String> = repos
                    .wallets
                    .find(tx.conn(), &a)?
                    .unwrap()
                    .balances
                    .into_iter()
                    .map(|b| b.0)
                    .collect();
                assert_eq!(order.last().map(String::as_str), Some("CURRENCY_S19_COIN"));
                assert_eq!(order[0], "CURRENCY_NOTES");
                // an unsafe key never reaches storage
                assert!(matches!(
                    add_currency(repos, tx, &a, "a.b", 1.0),
                    Err(WalletError::UnsafeCatalogId(_))
                ));
                // platinum spends the free buckets first
                add_currency(repos, tx, &a, "CURRENCY_PLATINUM_UNIV", 30.0).unwrap();
                add_currency(repos, tx, &a, "CURRENCY_PLATINUM_EPIC", 100.0).unwrap();
                let touched = spend_platinum(repos, tx, &a, 50.0).unwrap();
                assert_eq!(touched[0], ("CURRENCY_PLATINUM_UNIV".to_owned(), 0.0));
                assert_eq!(touched.last().unwrap(), &("CURRENCY_PLATINUM".to_owned(), 80.0));
                assert!(spend_platinum(repos, tx, &a, 81.0).is_err());
                Ok(())
            })
            .await;
        result.unwrap();
        let _ = WalletRecord {
            account: id,
            balances: vec![],
            bootstrap_version: None,
        };
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_entitlement_is_granted_once_and_an_unsafe_name_never() {
    each_engine(async |store| {
        let id = account();
        let a = id.clone();
        let entry = |name: &str, duration: f64| EntitlementEntry {
            name: name.into(),
            duration,
            activated_date: None,
            granted_at: None,
            source_sku_id: None,
        };
        store
            .account_write(&id, &actor(), move |tx, repos| {
                assert!(
                    repos
                        .entitlements
                        .grant_if_missing(tx, &a, &entry("exchange_slot_2", 0.0))?
                );
                assert!(
                    !repos
                        .entitlements
                        .grant_if_missing(tx, &a, &entry("exchange_slot_2", 5.0))?
                );
                assert!(repos.entitlements.grant_if_missing(
                    tx,
                    &a,
                    &entry("gauntlet-season01_top_100", 2_400_000.0)
                )?);
                assert!(matches!(
                    repos.entitlements.grant_if_missing(tx, &a, &entry("bad name", 0.0)),
                    Err(RepoError::Data(_))
                ));
                let held = repos.entitlements.find(tx.conn(), &a)?.unwrap();
                assert_eq!(held.entitlements.len(), 2);
                assert_eq!(
                    held.entitlements
                        .iter()
                        .find(|e| e.name == "exchange_slot_2")
                        .unwrap()
                        .duration,
                    0.0
                );
                Ok(())
            })
            .await
            .unwrap();
        let a = id.clone();
        assert!(
            store
                .read(move |c, r| r.entitlements.find(c, &a))
                .await
                .unwrap()
                .is_some()
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rollup_adds_its_counters_and_returns_the_day_as_one_document() {
    each_engine(async |store| {
        let id = account();
        let a = id.clone();
        let doc = store
            .account_write(&id, &actor(), move |tx, repos| {
                let inc = |path: &str, v: f64| (path.to_owned(), v);
                repos.economy_rollups.add_to_rollup(
                    tx,
                    &a,
                    "2026-10-10",
                    "CURRENCY_PLATINUM",
                    &[inc("credited", 100.0), inc("bySource.STORE.debited", 40.0)],
                    1_900_000_000_000,
                )?;
                repos.economy_rollups.add_to_rollup(
                    tx,
                    &a,
                    "2026-10-10",
                    "CURRENCY_PLATINUM",
                    &[
                        inc("credited", 5.0),
                        inc("bySource.STORE.debited", 2.0),
                        inc("bySource.QUEST.credited", 1.0),
                    ],
                    1_900_000_000_001,
                )
            })
            .await
            .unwrap();
        assert_eq!(doc["credited"], 105.0);
        assert_eq!(doc["bySource"]["STORE"]["debited"], 42.0);
        assert_eq!(doc["bySource"]["QUEST"]["credited"], 1.0);
    })
    .await;
}
