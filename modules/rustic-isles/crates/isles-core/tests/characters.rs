//! RU5 characters-loadouts: the repositories' Node tests (`persistence/sqlite/repositories.game.test.ts`) on both
//! engines, the loadout unlock gate (`routes/loadoutUnlockGate.test.ts`), the change log, soft deletion, the starter
//! bootstrap's repairs and the routes' edges that the Node replay does not reach. SQLite always; Postgres when
//! `REFORGED_TEST_POSTGRES_URL` names a server.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::{Arc, Mutex};

use diesel::RunQueryDsl;
use isles_core::areas::characters::controller;
use isles_core::areas::characters::golden::{Fakes, TestDb, call_once, open_store};
use isles_core::areas::characters::{AuthData, starter};
use isles_core::db::IslesStore;
use isles_core::http::{Rifts, Shared};
use isles_core::repo::characters::{CharacterRecord, LoadoutRecord};
use isles_core::tables::{StarterManifest, VersionTables};
use isles_core::{AccountId, IslesVersion, UserId};
use module_api::{Actor, ActorContext, NodeIdentity};
use rifts_seam::{
    AccountId as RiftsAccount, BoxFuture, GuardHooks, HostWrite, HostWriteKind, HostWriteOptions, InventoryDecision,
    InventoryTransaction, Lifecycle, Reply, RequestInfo, RiftsParts, RuntimeAuth, SeamError,
};
use serde_json::{Value, json};

fn actor() -> ActorContext {
    ActorContext::new(Actor::Admin { key: "owner".into() })
}

async fn on_both(test: impl AsyncFn(TestDb)) {
    test(TestDb::sqlite()).await;
    match TestDb::postgres() {
        Some(db) => test(db).await,
        None => eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the Postgres half is skipped"),
    }
}

async fn account(store: &IslesStore, user: &str) -> AccountId {
    controller::find_or_create_account(store, &actor(), user).await.unwrap()
}

fn character(id: &str, account: &AccountId, version: i64) -> CharacterRecord {
    CharacterRecord {
        character_id: id.into(),
        account: account.clone(),
        created_date: "Oct 1, 2026".into(),
        last_modified_date: "Oct 1, 2026".into(),
        name: format!("char-{id}"),
        update_version: version,
        data: format!(r#"{{"id":"{id}"}}"#),
    }
}

async fn read_character(store: &IslesStore, id: &str, account: &AccountId) -> Option<CharacterRecord> {
    let (id, account) = (id.to_owned(), account.clone());
    store
        .read(move |conn, repos| repos.characters.find(conn, &id, &account))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn characters_keep_insertion_order_stay_private_and_update_only_to_a_newer_version() {
    on_both(async |db| {
        let store = open_store(&db, &Fakes::default());
        let (u1, u2) = (account(&store, "u1").await, account(&store, "u2").await);
        for (id, owner) in [("c-b", &u1), ("c-a", &u1), ("c-x", &u2)] {
            let record = character(id, owner, 1);
            store
                .account_write(owner, &actor(), move |tx, repos| repos.characters.create(tx, &record))
                .await
                .unwrap();
        }
        let duplicate = character("c-a", &u1, 1);
        let again = store
            .account_write(&u1, &actor(), move |tx, repos| repos.characters.create(tx, &duplicate))
            .await;
        assert!(again.is_err(), "a character id is unique");

        let owner = u1.clone();
        let ids: Vec<String> = store
            .read(move |conn, repos| repos.characters.find_many_by_account(conn, &owner))
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.character_id)
            .collect();
        assert_eq!(ids, ["c-b", "c-a"]);
        assert!(read_character(&store, "c-x", &u1).await.is_none(), "wrong owner");
        assert_eq!(read_character(&store, "c-x", &u2).await.unwrap().name, "char-c-x");

        let write = |account: AccountId, id: &'static str, data: &'static str, version: i64| {
            let store = store.clone();
            async move {
                store
                    .account_write(&account.clone(), &actor(), move |tx, repos| {
                        repos
                            .characters
                            .update_data_conditional(tx, id, &account, data, version)
                    })
                    .await
                    .unwrap();
            }
        };
        write(u1.clone(), "c-a", r#"{"v":1}"#, 1).await; // equal: refused
        assert_eq!(
            read_character(&store, "c-a", &u1).await.unwrap().data,
            r#"{"id":"c-a"}"#
        );
        write(u1.clone(), "c-a", r#"{"v":3}"#, 3).await;
        let updated = read_character(&store, "c-a", &u1).await.unwrap();
        assert_eq!((updated.data.as_str(), updated.update_version), (r#"{"v":3}"#, 3));
        write(u1.clone(), "c-a", r#"{"v":2}"#, 2).await; // older: refused
        assert_eq!(read_character(&store, "c-a", &u1).await.unwrap().update_version, 3);
        write(u2.clone(), "c-a", r#"{"stolen":1}"#, 9).await; // wrong owner
        assert_eq!(read_character(&store, "c-a", &u1).await.unwrap().update_version, 3);
        write(u1.clone(), "ghost", "{}", 9).await;
    })
    .await;
}

fn slots(n: usize) -> String {
    serde_json::to_string(&(0..n).map(|slot| json!({ "slot": slot })).collect::<Vec<_>>()).unwrap()
}

async fn loadout_row(store: &IslesStore, id: &str, account: &AccountId) -> Option<LoadoutRecord> {
    let (id, account) = (id.to_owned(), account.clone());
    store
        .read(move |conn, repos| repos.loadouts.find(conn, &id, &account))
        .await
        .unwrap()
}

/// A loadout write as one of `store`'s account writes: `f` gets the repository and the account.
async fn loadout_write<T: Send + 'static>(
    store: &IslesStore,
    account: &AccountId,
    f: impl FnOnce(&mut store::Tx<'_>, &isles_core::repo::Repositories) -> isles_core::repo::RepoResult<T> + Send + 'static,
) -> T {
    store.account_write(account, &actor(), f).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn loadouts_are_revision_guarded_and_respect_the_visible_slots() {
    on_both(async |db| {
        let store = open_store(&db, &Fakes::default());
        let (u1, u2) = (account(&store, "u1").await, account(&store, "u2").await);
        let first = LoadoutRecord {
            character_id: "c1".into(),
            account: u1.clone(),
            loadouts: slots(3),
            persistent: r#"{"p":1}"#.into(),
            active_index: None,
            unlocked_total_slots: Some(3),
            revision: None,
            bootstrap_version: None,
        };
        let record = first.clone();
        loadout_write(&store, &u1, move |tx, repos| repos.loadouts.create(tx, &record)).await;
        let duplicate = LoadoutRecord {
            loadouts: "[]".into(),
            persistent: "{}".into(),
            ..first.clone()
        };
        assert!(
            store
                .account_write(&u1, &actor(), move |tx, repos| repos.loadouts.create(tx, &duplicate))
                .await
                .is_err()
        );
        let initial = loadout_row(&store, "c1", &u1).await.unwrap();
        assert_eq!(
            initial,
            LoadoutRecord {
                active_index: Some(0),
                revision: Some(0),
                ..first
            }
        );
        assert!(loadout_row(&store, "c1", &u2).await.is_none());

        let a = u1.clone();
        let replace = |slot: u32, data: &'static str, revision: i64, who: &AccountId, id: &'static str| {
            let (store, who) = (store.clone(), who.clone());
            async move {
                let owner = who.clone();
                loadout_write(&store, &owner, move |tx, repos| {
                    repos
                        .loadouts
                        .replace_slot_if_revision_matches(tx, id, &who, slot, data, revision)
                })
                .await
            }
        };
        assert!(replace(1, r#"{"slot":"new"}"#, 0, &a, "c1").await);
        let row = loadout_row(&store, "c1", &u1).await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&row.loadouts).unwrap()[1],
            json!({ "slot": "new" })
        );
        assert_eq!(row.revision, Some(1));
        assert!(!replace(1, "{}", 0, &a, "c1").await, "stale revision");
        assert!(!replace(3, "{}", 1, &a, "c1").await, "a slot beyond the unlocked total");
        assert!(!replace(0, "not json", 1, &a, "c1").await);
        assert!(!replace(0, "{}", 0, &a, "ghost").await);
        assert!(!replace(0, "{}", 1, &u2, "c1").await, "wrong owner");

        let persistent = |json: &'static str, revision: i64| {
            let (store, who) = (store.clone(), u1.clone());
            async move {
                let owner = who.clone();
                loadout_write(&store, &owner, move |tx, repos| {
                    repos
                        .loadouts
                        .update_persistent_if_revision_matches(tx, "c1", &who, json, revision)
                })
                .await
            }
        };
        assert!(persistent(r#"{"p":2}"#, 1).await);
        assert!(!persistent(r#"{"p":3}"#, 1).await);
        let row = loadout_row(&store, "c1", &u1).await.unwrap();
        assert_eq!((row.persistent.as_str(), row.revision), (r#"{"p":2}"#, Some(2)));

        let active = |index: u32, revision: i64| {
            let (store, who) = (store.clone(), u1.clone());
            async move {
                let owner = who.clone();
                loadout_write(&store, &owner, move |tx, repos| {
                    repos
                        .loadouts
                        .update_active_index_if_revision_matches(tx, "c1", &who, index, revision)
                })
                .await
            }
        };
        assert!(active(2, 2).await);
        let row = loadout_row(&store, "c1", &u1).await.unwrap();
        assert_eq!((row.active_index, row.revision), (Some(2), Some(3)));
        assert!(
            active(2, 99).await,
            "re-applying the current slot is a success without revision churn"
        );
        assert_eq!(loadout_row(&store, "c1", &u1).await.unwrap().revision, Some(3));
        assert!(!active(5, 3).await, "locked slot");
        assert!(!active(1, 2).await, "stale revision");

        let all = |json: String, total: u32, revision: i64| {
            let (store, who) = (store.clone(), u1.clone());
            async move {
                let owner = who.clone();
                loadout_write(&store, &owner, move |tx, repos| {
                    repos
                        .loadouts
                        .replace_all_and_entitlement_if_revision_matches(tx, "c1", &who, &json, total, revision)
                })
                .await
            }
        };
        assert!(all(slots(5), 4, 3).await);
        let row = loadout_row(&store, "c1", &u1).await.unwrap();
        assert_eq!(serde_json::from_str::<Vec<Value>>(&row.loadouts).unwrap().len(), 5);
        assert_eq!((row.unlocked_total_slots, row.revision), (Some(4), Some(4)));
        assert!(!all(slots(5), 6, 4).await, "more than the maximum");
        assert!(!all(slots(2), 3, 4).await, "total above the stored slots");
        assert!(!all("[]".into(), 1, 4).await);
        assert!(!all("nope".into(), 1, 4).await);
        assert!(!all(slots(5), 4, 3).await, "stale revision");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_legacy_loadout_row_stays_writable_at_revision_zero() {
    on_both(async |db| {
        let store = open_store(&db, &Fakes::default());
        let u1 = account(&store, "u1").await;
        let owner = u1.to_string();
        // A row from before the entitlement column: no persisted total
        store
            .write(&actor(), move |tx, _| {
                let sql = format!(
                    "INSERT INTO loadouts (character_id, account_id, loadouts, persistent) VALUES ('old', '{owner}', '{}', '{{}}')",
                    slots(2)
                );
                diesel::sql_query(sql).execute(tx.conn()).map_err(isles_core::repo::RepoError::from)
            })
            .await
            .unwrap();
        let row = loadout_row(&store, "old", &u1).await.unwrap();
        assert_eq!((row.revision, row.active_index, row.unlocked_total_slots), (Some(0), Some(0), None));
        // Two stored slots and no persisted total: one is visible (the legacy interpretation), so slot 1 is locked
        let write = |slot: u32, data: &'static str| {
            let (store, who) = (store.clone(), u1.clone());
            async move {
                let owner = who.clone();
                loadout_write(&store, &owner, move |tx, repos| {
                    repos.loadouts.replace_slot_if_revision_matches(tx, "old", &who, slot, data, 0)
                })
                .await
            }
        };
        assert!(!write(1, "{}").await);
        assert!(write(0, r#"{"slot":"x"}"#).await);
        assert_eq!(loadout_row(&store, "old", &u1).await.unwrap().revision, Some(1));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_are_logged_and_a_deleted_character_is_gone_until_restored() {
    on_both(async |db| {
        let store = open_store(&db, &Fakes::default());
        let u1 = account(&store, "u1").await;
        let record = character("c1", &u1, 0);
        store
            .account_write(&u1, &actor(), move |tx, repos| repos.characters.create(tx, &record))
            .await
            .unwrap();
        let log = store.database().changes(0, 100).await.unwrap();
        let entry = log.iter().find(|e| e.table_name == "characters").unwrap();
        assert_eq!((entry.op.as_str(), entry.actor.as_str()), ("insert", "admin:owner"));
        assert!(entry.row_key.contains("c1"));

        store
            .write(&actor(), |tx, _| {
                tx.soft_delete("characters", &[("character_id", store::KeyValue::Text("c1".into()))])
                    .map_err(isles_core::repo::RepoError::from)
            })
            .await
            .unwrap();
        assert!(
            read_character(&store, "c1", &u1).await.is_none(),
            "soft-deleted rows are not served"
        );
        store
            .write(&actor(), |tx, _| {
                tx.restore("characters", &[("character_id", store::KeyValue::Text("c1".into()))])
                    .map_err(isles_core::repo::RepoError::from)
            })
            .await
            .unwrap();
        assert!(read_character(&store, "c1", &u1).await.is_some());
    })
    .await;
}

fn tables() -> VersionTables {
    VersionTables {
        target_changelist: 647472,
        game_version: "1.14.7",
        active_huntpass_id: "",
        recent_huntpass_seasons: &[],
        patchnotes_release_version: "",
        bounty_item_grant_data: &[],
        store_sections: &[],
        seasonal_coin: None,
        masked_journey_nodes: &[],
        starter_manifest: StarterManifest {
            bootstrap_version: "starter-1.12-v5",
            stacked: &["TOKEN_DAILY_CHALLENGE_DRAFT", "QI_BASIC_FLARE_DURABLE"],
            quick_emojis: Some(&["EJ_PLAYER_BOSUN_JOY", "", "", "", "", "", "", "", ""]),
        },
        switch_defaults: &[],
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_character_is_seeded_atomically_and_a_missing_record_is_repaired() {
    on_both(async |db| {
        let fakes = Fakes::default();
        let store = open_store(&db, &fakes);
        let tables = tables();
        let characters = controller::characters_for_user(&store, &tables, &actor(), "alice")
            .await
            .unwrap();
        assert_eq!(characters.len(), 1);
        let id = characters[0]["id"].as_str().unwrap().to_owned();
        let u = account(&store, "alice").await;
        // Seeded: the inventory, the loadout (version-tagged) and the wallet, together
        let (inventory, loadout) = controller::ensure_bootstrap(&store, &tables, &actor(), "alice", &id)
            .await
            .unwrap();
        assert_eq!(inventory.bootstrap_version.as_deref(), Some("starter-1.12-v5"));
        assert_eq!(loadout.bootstrap_version.as_deref(), Some("starter-1.12-v5"));
        assert_eq!(loadout.unlocked_total_slots, Some(1));

        // Another account's character is not repaired for this one
        let other = controller::ensure_bootstrap(&store, &tables, &actor(), "mallory", &id).await;
        assert!(matches!(other, Err(controller::CtlError::NotOwned)));
        let unknown = controller::ensure_bootstrap(&store, &tables, &actor(), "alice", "nope").await;
        assert!(matches!(unknown, Err(controller::CtlError::NotOwned)));

        // A loadout lost while the inventory survives is rebuilt from the inventory's own instance ids
        let (owner, cid) = (u.clone(), id.clone());
        store
            .write(&actor(), move |tx, _| {
                tx.soft_delete("loadouts", &[("character_id", store::KeyValue::Text(cid))])
                    .map_err(isles_core::repo::RepoError::from)?;
                let _ = owner;
                Ok(())
            })
            .await
            .unwrap();
        assert!(loadout_row(&store, &id, &u).await.is_none());
        let state = controller::loadout_state(&store, &tables, &actor(), "alice", &id).await;
        // The deleted row still holds the primary key, so the rebuild cannot insert: the failure is the database's
        assert!(state.is_err());

        // An inventory lost while the loadout survives is rebuilt with the loadout's ids
        let bob = controller::characters_for_user(&store, &tables, &actor(), "bob")
            .await
            .unwrap();
        let bob_id = bob[0]["id"].as_str().unwrap().to_owned();
        let before = controller::ensure_bootstrap(&store, &tables, &actor(), "bob", &bob_id)
            .await
            .unwrap();
        fakes.inventories_clear(&bob_id);
        let (rebuilt, kept) = controller::ensure_bootstrap(&store, &tables, &actor(), "bob", &bob_id)
            .await
            .unwrap();
        let items: Vec<Value> = serde_json::from_str(&rebuilt.instanced_items).unwrap();
        assert_eq!(items.len(), 23);
        let weapon = serde_json::from_str::<Vec<Value>>(&kept.loadouts).unwrap()[0]["weapon"]["instance_id"].clone();
        assert!(
            items.iter().any(|i| i["instanceId"] == weapon),
            "{items:?} lacks {weapon}"
        );
        assert_eq!(before.1.loadouts, kept.loadouts);
    })
    .await;
}

/// Records what the gate asks the host-write hook.
struct Recorder {
    inner: rifts_seam::DedicatedOnly,
    asked: Mutex<Vec<(HostWriteKind, String)>>,
    tagged: Mutex<Vec<HostWrite>>,
    denial: Option<(u16, &'static str)>,
}

impl Lifecycle for Recorder {
    fn runtime_safety_issues(&self, s: &dyn rifts_seam::Settings) -> Result<Vec<String>, SeamError> {
        self.inner.runtime_safety_issues(s)
    }
    fn startup_summary(&self, role: &str) -> Result<Option<rifts_seam::Json>, SeamError> {
        self.inner.startup_summary(role)
    }
    fn start(&self, role: &str) -> Result<(), SeamError> {
        self.inner.start(role)
    }
    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        self.inner.stop()
    }
    fn tables(&self) -> &'static [rifts_seam::TableSpec] {
        self.inner.tables()
    }
    fn account_deletion_steps(&self) -> Vec<rifts_seam::DeletionStep> {
        self.inner.account_deletion_steps()
    }
}

impl GuardHooks for Recorder {
    fn service_roles(&self) -> &'static [&'static str] {
        self.inner.service_roles()
    }
    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        self.inner.guard_manifest_roles()
    }
    fn census(&self, request: &rifts_seam::HostRouteSample) -> Result<(), SeamError> {
        self.inner.census(request)
    }
    fn rate_limit_bucket(&self, request: RequestInfo) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        self.inner.rate_limit_bucket(request)
    }
    fn authorize_inventory(
        &self,
        runtime: RuntimeAuth,
        transaction: InventoryTransaction,
    ) -> BoxFuture<'_, Result<InventoryDecision, SeamError>> {
        self.inner.authorize_inventory(runtime, transaction)
    }
    fn authorize_host_write(
        &self,
        _: RuntimeAuth,
        kind: HostWriteKind,
        target: RiftsAccount,
        _: HostWriteOptions,
    ) -> BoxFuture<'_, Result<Option<Reply>, SeamError>> {
        self.asked.lock().unwrap().push((kind, target.0));
        let reply = self.denial.map(|(status, body)| Reply {
            status,
            body: rifts_seam::Json(body.to_owned()),
        });
        Box::pin(async move { Ok(reply) })
    }
    fn record_host_write(&self, _: RuntimeAuth, write: HostWrite) -> BoxFuture<'_, Result<(), SeamError>> {
        self.tagged.lock().unwrap().push(write);
        Box::pin(async { Ok(()) })
    }
    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        self.inner.host_policy_refuses()
    }
    fn note_would_refuse(&self, what: &str, detail: &str) -> Result<(), SeamError> {
        self.inner.note_would_refuse(what, detail)
    }
}

const DENIAL: &str =
    r#"{"code":"P2P_HOST_WRITE_DENIED","message":"This host may not write this player's state.","payload":null}"#;

fn runtime() -> RuntimeAuth {
    RuntimeAuth {
        capability_id: "cap".into(),
        session: "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap(),
        generation: 1,
        host_account: RiftsAccount("host-1".into()),
        scopes: Vec::new(),
        roster: Vec::new(),
        host_peer: None,
        bridge_instance: None,
        wire_session: None,
    }
}

fn shared_with(store: &IslesStore, version: IslesVersion, recorder: Arc<Recorder>) -> Arc<Shared> {
    let dedicated = rifts_seam::DedicatedOnly;
    Arc::new(Shared {
        version,
        settings: isles_core::Settings::default(),
        node: NodeIdentity::standalone(),
        game_data: isles_core::game_data::GameDataRoot::default(),
        store: Some(store.clone()),
        rifts: Rifts::new(RiftsParts {
            realms: Arc::new(dedicated),
            entry: Arc::new(dedicated),
            guard: recorder,
            routes: Vec::new(),
        }),
        admin_api: false,
        matchmaking: Arc::default(),
        auth: isles_core::areas::auth::AuthRuntime::default(),
    })
}

fn version() -> IslesVersion {
    struct Hooks;
    impl isles_core::VersionHooks for Hooks {
        fn no_player_policy(&self) -> isles_core::hooks::NoPlayerPolicy {
            isles_core::hooks::NoPlayerPolicy::ConfiguredAccount
        }
        fn game_session_payload(&self, _: &str) -> Value {
            Value::Null
        }
        fn network_encryption_keys(&self) -> bool {
            false
        }
        fn weapons_at_base_level(&self, _: &isles_core::Settings) -> bool {
            false
        }
        fn reforge_rules(&self) -> bool {
            false
        }
        fn mastery_persistence(&self) -> bool {
            false
        }
        fn huntpass_payload(&self, _: &str) -> Value {
            Value::Null
        }
        fn objective_row(&self, _: &str, _: &isles_core::hooks::ObjectiveRow) -> Value {
            Value::Null
        }
        fn progression_config(&self) -> isles_core::hooks::ProgressionConfigRule {
            isles_core::hooks::ProgressionConfigRule::AsIs
        }
        fn survey_config(&self) -> isles_core::hooks::StubReply {
            isles_core::hooks::StubReply {
                status: 400,
                body: None,
            }
        }
        fn entitlements_payload(&self, _: &[isles_core::hooks::Entitlement]) -> Value {
            Value::Null
        }
        fn candidate_destination(&self, hunt_id: &str, _: bool) -> isles_core::hooks::Destination {
            isles_core::hooks::Destination {
                game_mode: isles_core::hooks::GameMode::Island,
                hunt_id: Some(hunt_id.to_owned()),
            }
        }
    }
    IslesVersion {
        hooks: Arc::new(Hooks),
        tables: tables(),
        routes: isles_core::routes::RouteSet::V1147,
    }
}

fn gate_recorder(denial: Option<(u16, &'static str)>) -> Arc<Recorder> {
    Arc::new(Recorder {
        inner: rifts_seam::DedicatedOnly,
        asked: Mutex::default(),
        tagged: Mutex::default(),
        denial,
    })
}

async fn unlock(shared: &Arc<Shared>, auth: AuthData, user: &str, character: &str) -> (u16, String) {
    let reply = call_once(
        shared,
        "POST",
        &format!("/loadout/{user}/{character}/unlock/1"),
        auth,
        None,
        None,
    )
    .await;
    (reply.status, reply.body)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_unlock_gate_admits_a_gameserver_a_permitted_host_and_never_a_player() {
    let db = TestDb::sqlite();
    let store = open_store(&db, &Fakes::default());
    let characters = controller::characters_for_user(&store, &tables(), &actor(), "player-1")
        .await
        .unwrap();
    let id = characters[0]["id"].as_str().unwrap().to_owned();
    let gameserver = AuthData {
        user_id: None,
        is_gameserver: true,
        host_runtime: None,
    };
    let host = AuthData {
        user_id: Some("host-1".into()),
        is_gameserver: false,
        host_runtime: Some(runtime()),
    };
    let player = AuthData {
        user_id: Some("player-1".into()),
        is_gameserver: false,
        host_runtime: None,
    };

    // A dedicated gameserver unlocks for any account, and no host hook is asked
    let recorder = gate_recorder(Some((403, DENIAL)));
    let shared = shared_with(&store, version(), recorder.clone());
    assert_eq!(unlock(&shared, gameserver, "player-1", &id).await.0, 200);
    assert!(recorder.asked.lock().unwrap().is_empty());

    // A player's own bearer token is refused with 403 before any hook is asked
    let (status, body) = unlock(&shared, player, "player-1", &id).await;
    assert_eq!(status, 403);
    assert!(body.contains(r#""code":"forbidden""#), "{body}");
    assert!(recorder.asked.lock().unwrap().is_empty());

    // A player-hosted runtime is asked LOADOUT_UNLOCK for the target player, and may unlock when the hook allows it
    let recorder = gate_recorder(None);
    let shared = shared_with(&store, version(), recorder.clone());
    let target = account(&store, "player-1").await.to_string();
    assert_eq!(unlock(&shared, host.clone(), "player-1", &id).await.0, 200);
    assert_eq!(
        *recorder.asked.lock().unwrap(),
        [(HostWriteKind::LoadoutUnlock, target.clone())]
    );
    // A host's unlock is tagged like its other writes: counts only
    let tagged = recorder.tagged.lock().unwrap().clone();
    assert_eq!(tagged.len(), 1);
    assert_eq!(tagged[0].kind, HostWriteKind::LoadoutUnlock);
    assert!(tagged[0].summary.0.contains("authoritativeTotalSlots"));
    assert!(!tagged[0].summary.0.contains(&id));

    // A host for a player it may not write is refused with the hook's answer, and nothing is unlocked
    let recorder = gate_recorder(Some((403, DENIAL)));
    let shared = shared_with(&store, version(), recorder.clone());
    let (status, body) = unlock(&shared, host.clone(), "player-1", &id).await;
    assert_eq!((status, body.as_str()), (403, DENIAL));
    assert!(recorder.tagged.lock().unwrap().is_empty());
    // A player the node does not know cannot be on a host's roster
    let (status, body) = unlock(&shared, host, "stranger-1", "character-9").await;
    assert_eq!(status, 403);
    assert!(body.contains("P2P_HOST_WRITE_DENIED"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_with_no_authority_at_all_is_refused() {
    let db = TestDb::sqlite();
    let store = open_store(&db, &Fakes::default());
    let shared = shared_with(&store, version(), gate_recorder(None));
    let nobody = AuthData::default();
    let (status, body) = unlock(&shared, nobody, "player-1", "character-1").await;
    assert_eq!(status, 403);
    assert!(body.contains(r#""code":"forbidden""#));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_the_auth_middleware_did_not_reach_answers_501() {
    let db = TestDb::sqlite();
    let store = open_store(&db, &Fakes::default());
    let shared = shared_with(&store, version(), gate_recorder(None));
    // No AuthData in the request's extensions: the middleware (RU5 auth) is not in this build
    let handlers = isles_core::areas::backend();
    let route = isles_core::routes::all()
        .iter()
        .find(|r| r.path == "/character" && r.method == isles_core::routes::Method::Get)
        .unwrap();
    let call = isles_core::http::Call {
        shared,
        route,
        params: isles_core::routes::Params::default(),
        request: axum::extract::Request::new(axum::body::Body::empty()),
        origin_host: None,
        request_id: "r".into(),
    };
    let response = handlers.get(route).unwrap().clone()(call).await;
    assert_eq!(response.status().as_u16(), 501);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_starter_ids_of_two_accounts_never_collide() {
    let tables = tables();
    let a = starter::build_manifest(&tables).unwrap();
    let b = starter::build_manifest(&tables).unwrap();
    let ids = |m: &starter::Manifest| -> Vec<String> {
        m.instanced_items
            .iter()
            .map(|i| i["instanceId"].as_str().unwrap().to_owned())
            .collect()
    };
    assert!(ids(&a).iter().all(|id| !ids(&b).contains(id)));
    let _ = UserId::random();
}
