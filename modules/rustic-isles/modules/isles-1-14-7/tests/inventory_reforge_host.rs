//! A realm host's runtime through rustic-rifts' guard hooks (the `P2PExtension` of Node): the grant fence inside the
//! inventory transaction, the item-save authorization and the reforge's host writes. The guard here records what isles
//! asks it and answers as told; rustic-rifts' own guard is RF6's.
#![cfg(feature = "realms")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::{Arc, Mutex};

use isles_core::Settings;
use isles_core::areas::inventory::testkit::{Harness, SCENARIOS, sqlite_config};
use isles_core::repo::inventory::LedgerKey;
use rifts_seam::{
    AccountId, BoxFuture, DedicatedOnly, DeletionStep, GuardHooks, HostWrite, HostWriteKind, HostWriteOptions,
    InventoryDecision, InventoryFence, InventoryTransaction, Json, Lifecycle, Reply, Rifts, RiftsParts, RouteHandler,
    RuntimeAuth, SeamError, TableSpec, TransactionPhase,
};
use serde_json::{Value, json};

fn seed() -> Value {
    serde_json::from_str::<Value>(SCENARIOS).unwrap()["seed"].clone()
}

fn settings() -> Settings {
    Settings::from_pairs([("GAMESERVER_NO_PLAYER_USER_ID", "u1")])
}

fn gameserver(user: &str) -> Vec<(&'static str, String)> {
    vec![
        ("x-test-auth", json!({"IsGameserver": true, "userId": user}).to_string()),
        ("content-type", "application/json".into()),
    ]
}

#[derive(Default)]
struct Seen(Mutex<Vec<String>>);

impl Seen {
    fn push(&self, line: String) {
        self.0.lock().unwrap().push(line);
    }

    fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// A rustic-rifts guard that records what isles asks and answers as told.
struct Guard {
    seen: Arc<Seen>,
    /// `authorize_host_write` refuses this target
    deny_target: Option<&'static str>,
    /// The fence refuses the grant with this reply
    fence_refuses: Option<Reply>,
}

struct Fence {
    seen: Arc<Seen>,
    refuses: Option<Reply>,
}

impl InventoryFence for Fence {
    fn on_grant(&self, grant: &Json) -> Result<(), Reply> {
        self.seen.push(format!("grant {}", grant.0));
        self.refuses.clone().map_or(Ok(()), Err)
    }

    fn report(&self, phase: TransactionPhase) {
        self.seen.push(format!("report {phase:?}"));
    }
}

impl Lifecycle for Guard {
    fn runtime_safety_issues(&self, _: &dyn rifts_seam::Settings) -> Result<Vec<String>, SeamError> {
        Ok(Vec::new())
    }
    fn startup_summary(&self, _: &str) -> Result<Option<Json>, SeamError> {
        Ok(None)
    }
    fn start(&self, _: &str) -> Result<(), SeamError> {
        Ok(())
    }
    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        Box::pin(async { Ok(()) })
    }
    fn tables(&self) -> &'static [TableSpec] {
        &[]
    }
    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        Vec::new()
    }
}

impl GuardHooks for Guard {
    fn service_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn census(&self, _: &rifts_seam::RequestInfo) -> Result<(), SeamError> {
        Ok(())
    }
    fn rate_limit_bucket(&self, _: rifts_seam::RequestInfo) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        Box::pin(async { Ok(None) })
    }
    fn authorize_inventory(
        &self,
        _: RuntimeAuth,
        transaction: InventoryTransaction,
    ) -> BoxFuture<'_, Result<InventoryDecision, SeamError>> {
        self.seen.push(format!(
            "authorize_inventory {} {} {}",
            transaction.user, transaction.character_id, transaction.transaction_id
        ));
        let fence = Fence {
            seen: self.seen.clone(),
            refuses: self.fence_refuses.clone(),
        };
        Box::pin(async move { Ok(InventoryDecision::Authorized(Box::new(fence))) })
    }
    fn authorize_host_write(
        &self,
        _: RuntimeAuth,
        kind: HostWriteKind,
        target: AccountId,
        _: HostWriteOptions,
    ) -> BoxFuture<'_, Result<Option<Reply>, SeamError>> {
        self.seen.push(format!("authorize_host_write {kind:?} {target}"));
        let denied = self.deny_target == Some(target.0.as_str());
        Box::pin(async move {
            Ok(denied.then(|| Reply {
                status: 403,
                body: Json(r#"{"code":"HOST_DENIED","message":"no","payload":null}"#.into()),
            }))
        })
    }
    fn record_host_write(&self, _: RuntimeAuth, write: HostWrite) -> BoxFuture<'_, Result<(), SeamError>> {
        self.seen.push(format!(
            "record_host_write {:?} {} {}",
            write.kind, write.target, write.summary.0
        ));
        Box::pin(async { Ok(()) })
    }
    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        Ok(false)
    }
    fn note_would_refuse(&self, _: &str, _: &str) -> Result<(), SeamError> {
        Ok(())
    }
}

/// The module's own stub routes with this guard in place of rustic-guard's stub.
fn rifts(guard: Guard) -> Rifts {
    let routes: Vec<Arc<dyn RouteHandler>> = vec![
        Arc::new(rifts_admission::stub_routes()),
        Arc::new(rifts_relay::stub_routes()),
        Arc::new(rifts_realms::stub_routes()),
        Arc::new(rifts_guard::stub_routes()),
    ];
    Rifts::new(RiftsParts {
        realms: Arc::new(DedicatedOnly),
        entry: Arc::new(DedicatedOnly),
        guard: Arc::new(guard),
        routes,
    })
}

fn host(user: Option<&str>) -> Vec<(&'static str, String)> {
    let mut auth = json!({"host": true});
    if let Some(user) = user {
        auth["userId"] = json!(user);
    }
    vec![
        ("x-test-auth", auth.to_string()),
        ("content-type", "application/json".into()),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hosts_transaction_runs_through_the_fence_inside_the_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Seen::default());
    let guard = Guard {
        seen: seen.clone(),
        deny_target: None,
        fence_refuses: None,
    };
    let harness = Harness::start_with(
        &sqlite_config(dir.path()),
        isles_1_14_7::version(),
        settings(),
        &seed(),
        rifts(guard),
    )
    .await;
    // a host may grant: the module's fence authorizes it
    let transaction = json!({"characterId": "c1", "transactionId": "h-1", "source": "x",
        "addStackedItems": [{"catalogId": "MAT_A", "quantity": 1}]});
    let reply = harness
        .send("POST", "/inventory", &host(Some("u1")), Some(&transaction.to_string()))
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    let lines = seen.all();
    assert_eq!(lines[0], "authorize_inventory u1 c1 h-1");
    assert_eq!(lines[1], "report Applying");
    // the fence sees the whole request body, as the module's own hook read req.body
    assert!(
        lines[2].starts_with("grant {") && lines[2].contains(r#""source":"x""#),
        "{lines:?}"
    );
    assert_eq!(lines[3], "report Accepted");

    // the fence refuses: its reply is the answer and nothing is stored
    let seen = Arc::new(Seen::default());
    let guard = Guard {
        seen: seen.clone(),
        deny_target: None,
        fence_refuses: Some(Reply {
            status: 403,
            body: Json(r#"{"code":"HOST_GRANT","message":"not that","payload":null}"#.into()),
        }),
    };
    let dir = tempfile::tempdir().unwrap();
    let harness = Harness::start_with(
        &sqlite_config(dir.path()),
        isles_1_14_7::version(),
        settings(),
        &seed(),
        rifts(guard),
    )
    .await;
    let reply = harness
        .send("POST", "/inventory", &host(Some("u1")), Some(&transaction.to_string()))
        .await;
    assert_eq!(
        (reply.status, reply.body.as_str()),
        (403, r#"{"code":"HOST_GRANT","message":"not that","payload":null}"#)
    );
    let read = harness.send("GET", "/inventory/u1/c1", &gameserver("u1"), None).await;
    assert!(
        read.body.contains(r#"{"catalogId":"MAT_A","quantity":4}"#),
        "{}",
        read.body
    );
    let ledger = {
        let (repos, account) = (harness.repos.clone(), harness.accounts["u1"].clone());
        harness
            .store
            .read(move |conn, _| {
                repos.inventory_transactions.find_record(
                    conn,
                    LedgerKey {
                        account: &account,
                        character_id: "c1",
                        transaction_id: "h-1",
                    },
                )
            })
            .await
            .unwrap()
    };
    assert_eq!(ledger, None, "a refused grant is rolled back with its ledger row");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hosts_item_save_and_reforge_are_authorized_and_tagged() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Seen::default());
    let guard = Guard {
        seen: seen.clone(),
        deny_target: Some("u2"),
        fence_refuses: None,
    };
    let harness = Harness::start_with(
        &sqlite_config(dir.path()),
        isles_1_14_7::version(),
        settings(),
        &seed(),
        rifts(guard),
    )
    .await;
    let item = json!({"characterId": "c1", "instanceId": "I-AXE", "catalogId": "WP_AX_BEGINNER", "itemData": "d", "updateVersion": 50});
    let reply = harness
        .send(
            "POST",
            "/inventory/instanceditem",
            &host(Some("u1")),
            Some(&item.to_string()),
        )
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    assert_eq!(
        seen.all(),
        [
            "authorize_host_write ItemSave u1".to_owned(),
            r#"record_host_write ItemSave u1 {"instanceId":"I-AXE","catalogId":"WP_AX_BEGINNER","updateVersion":50,"itemDataBytes":1}"#.to_owned()
        ]
    );
    // a refused host write answers the module's reply
    let reply = harness
        .send(
            "POST",
            "/inventory/instanceditem",
            &host(Some("u2")),
            Some(&item.to_string()),
        )
        .await;
    assert_eq!(
        (reply.status, reply.body.as_str()),
        (403, r#"{"code":"HOST_DENIED","message":"no","payload":null}"#)
    );

    // the reforge: reset and restore, each authorized as REFORGE and the applied ones tagged
    let reset = harness
        .send(
            "DELETE",
            "/progression/u1/ExperienceTrack_Weapon_Axe",
            &host(Some("u1")),
            None,
        )
        .await;
    assert_eq!(reset.status, 200, "{reset:?}");
    let _ = harness
        .send(
            "DELETE",
            "/progression/u1/ExperienceTrack_Weapon_Axe",
            &host(Some("u1")),
            None,
        )
        .await;
    let restore = harness
        .send(
            "POST",
            "/progression/purchase/u1/ExperienceTrack_Weapon_Axe/3500",
            &host(Some("u1")),
            None,
        )
        .await;
    assert_eq!(restore.status, 200, "{restore:?}");
    let lines: Vec<String> = seen.all().into_iter().skip(3).collect();
    assert_eq!(lines[0], "authorize_host_write Reforge u1");
    assert!(
        lines[1].starts_with(
            "record_host_write Reforge u1 {\"route\":\"delete\",\"track\":\"ExperienceTrack_Weapon_Axe\",\"reforgeId\":"
        ),
        "{lines:?}"
    );
    assert!(
        lines[1].ends_with(r#""priorProgress":31275,"cap":27775,"banked":3500}"#),
        "{lines:?}"
    );
    // the second DELETE replays an open reforge: authorized, not tagged again
    assert_eq!(lines[2], "authorize_host_write Reforge u1");
    assert_eq!(lines[3], "authorize_host_write Reforge u1");
    assert!(lines[4].starts_with(r#"record_host_write Reforge u1 {"route":"purchase","track":"ExperienceTrack_Weapon_Axe","amount":3500,"grantId":"reforge-"#), "{lines:?}");
    assert_eq!(lines.len(), 5);
}
