//! Shipping a node's change log and database snapshots to a peer region's node over `Service::LogShip`
//! (DH7, DD23 layer 2; docs/security/{history,network}.md).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use history_ship::layout::{SegmentName, encode_segment};
use history_ship::{Bucket, BucketRead, LagAlert, LagLimits, RestoreOptions, ShipSettings, Shipper, lag, restore_to};
use module_api::network::NodeId;
use network::directory::SCHEMA;
use network::inprocess::InProcessNetwork;
use network::link::{Clock, NodeService, PeerLink, Service, ServiceMux};
use network::logship::{
    LOG_SHIP_VERSION, LogShipPutRequest, LogShipRefusal, LogShipReply, PeerBucket, PeerLogStore, decode_put_response,
    encode_put_request,
};
use store::history::{Head, entry_hash};
use store::{Actor, ChangeEntry, Database, DatabaseConfig, DatabaseOptions, transfer};

const MINUTE: i64 = 60_000;
const T0: i64 = 1_761_500_000_000;

fn id(text: &str) -> NodeId {
    text.parse().unwrap()
}

static CLOCK_1: AtomicI64 = AtomicI64::new(T0);
static CLOCK_2: AtomicI64 = AtomicI64::new(T0);
static CLOCK_3: AtomicI64 = AtomicI64::new(T0);

fn clock_1() -> i64 {
    CLOCK_1.load(Ordering::SeqCst)
}

fn clock_2() -> i64 {
    CLOCK_2.load(Ordering::SeqCst)
}

fn clock_3() -> i64 {
    CLOCK_3.load(Ordering::SeqCst)
}

fn synced_clock(cell: &AtomicI64) -> Arc<Clock> {
    let clock = Arc::new(Clock::default());
    let now = Clock::default().now_ms();
    clock.skew_by(cell.load(Ordering::SeqCst).saturating_sub(now));
    clock
}

fn set_time(cell: &AtomicI64, at: i64, receiver_clock: &Clock) {
    cell.store(at, Ordering::SeqCst);
    let sys = Clock::default().now_ms();
    receiver_clock.skew_by(at.saturating_sub(sys));
}

async fn write_entry(db: &Database, actor: &str, id_str: &str, name: &str, home: &str) {
    use diesel::connection::SimpleConnection;
    let sql = format!(
        "INSERT INTO network_directory_accounts (account, home, username, username_key, seq) \
         VALUES ('{id_str}', '{home}', '{name}', '{}', (SELECT COALESCE(MAX(seq), 0) + 1 FROM network_directory_accounts))",
        name.to_ascii_lowercase(),
    );
    db.write(Actor::Admin(actor.into()), move |tx| {
        tx.conn().batch_execute(&sql)?;
        Ok::<_, store::Error>(())
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_destroyed_nodes_database_restores_from_its_peers_folder_with_chain_and_trusted_head_verified() {
    let oregon = id("oregon-1");
    let uk = id("uk-1");

    let net = InProcessNetwork::new();
    let uk_dir = tempfile::tempdir().unwrap();
    let uk_clock = synced_clock(&CLOCK_1);
    set_time(&CLOCK_1, T0, &uk_clock);

    let uk_store = Arc::new(PeerLogStore::new(
        uk_dir.path().join("peer-logs"),
        [oregon.clone()],
        uk_clock.clone(),
    ));
    let uk_mux: Arc<dyn NodeService> = Arc::new(ServiceMux::new(uk.clone()).with(Service::LogShip, uk_store.clone()));
    net.attach(&uk, &uk_mux);

    let oregon_db_dir = tempfile::tempdir().unwrap();
    let oregon_db_cfg = DatabaseConfig::Sqlite {
        file: oregon_db_dir.path().join("oregon.db"),
    };
    let db = Database::open(
        &oregon_db_cfg,
        &DatabaseOptions {
            clock: clock_1,
            ..DatabaseOptions::default()
        },
        &SCHEMA,
    )
    .unwrap();

    write_entry(&db, "owner", "acc-1", "Alice", "oregon-1").await;
    write_entry(&db, "owner", "acc-2", "Bob", "oregon-1").await;

    let oregon_state_dir = tempfile::tempdir().unwrap();
    let mut settings = ShipSettings::new(
        PeerBucket::prefix_for(&oregon),
        oregon_state_dir.path().join("ship-peer-state.json"),
    );
    settings.snapshot_every = Duration::from_secs(15 * 60);

    let peer_bucket = PeerBucket::new(uk.clone(), Arc::new(net.link(&oregon)));
    let mut shipper = Shipper::new(db.clone(), peer_bucket.clone(), settings.clone()).unwrap();
    shipper.tick().await.unwrap(); // initial snapshot + log segment

    set_time(&CLOCK_1, T0 + 5 * MINUTE, &uk_clock);
    write_entry(&db, "owner", "acc-3", "Carol", "oregon-1").await;
    shipper.tick().await.unwrap();

    let expected_dump = transfer::dump(&oregon_db_cfg).unwrap();
    let trusted_head = Head {
        id: shipper.state().last_shipped_id,
        hash: shipper.state().last_shipped_hash.clone().unwrap(),
    };

    // Simulate Oregon's server and database being completely destroyed.
    drop(shipper);
    drop(db);
    drop(oregon_db_dir);

    // Restore Oregon's database from UK's peer-logs folder into a brand-new SQLite database,
    // verifying both the hash chain and the off-server trusted_head.
    let restored_dir = tempfile::tempdir().unwrap();
    let restored_cfg = DatabaseConfig::Sqlite {
        file: restored_dir.path().join("restored.db"),
    };
    let scratch = tempfile::tempdir().unwrap();
    let mut restore_opts = RestoreOptions::new(scratch.path());
    restore_opts.trusted_head = Some(trusted_head.clone());

    let outcome = restore_to(
        uk_store.as_ref(),
        &PeerBucket::prefix_for(&oregon),
        T0 + 6 * MINUTE,
        &restored_cfg,
        &SCHEMA,
        &DatabaseOptions::default(),
        &restore_opts,
    )
    .await
    .unwrap();

    assert!(outcome.snapshot.starts_with("oregon-1/snapshot/"));
    assert!(!outcome.segments.is_empty());
    assert_eq!(transfer::dump(&restored_cfg).unwrap(), expected_dump);

    // A wrong trusted_head fails the restore.
    let bad_dir = tempfile::tempdir().unwrap();
    let bad_cfg = DatabaseConfig::Sqlite {
        file: bad_dir.path().join("bad.db"),
    };
    let mut bad_opts = RestoreOptions::new(scratch.path());
    bad_opts.trusted_head = Some(Head {
        id: trusted_head.id,
        hash: "0".repeat(64),
    });
    let err = restore_to(
        uk_store.as_ref(),
        &PeerBucket::prefix_for(&oregon),
        T0 + 6 * MINUTE,
        &bad_cfg,
        &SCHEMA,
        &DatabaseOptions::default(),
        &bad_opts,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        err.contains(&format!("entry {} isn't the trusted head any more", trusted_head.id)),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tampered_segment_is_named_and_peer_enforces_prefix_and_append_only() {
    let oregon = id("oregon-1");
    let uk = id("uk-1");
    let germany = id("germany-1");

    let net = InProcessNetwork::new();
    let uk_dir = tempfile::tempdir().unwrap();
    let uk_clock = synced_clock(&CLOCK_2);
    set_time(&CLOCK_2, T0, &uk_clock);

    let uk_store = Arc::new(PeerLogStore::new(
        uk_dir.path().join("peer-logs"),
        [oregon.clone()],
        uk_clock.clone(),
    ));
    let uk_mux: Arc<dyn NodeService> = Arc::new(ServiceMux::new(uk.clone()).with(Service::LogShip, uk_store.clone()));
    net.attach(&uk, &uk_mux);

    // 1. Unlisted peer (`germany-1`) is refused with UnknownPeer.
    let germany_link = net.link(&germany);
    let req = LogShipPutRequest::new(
        format!(
            "germany-1/log/0000000000000000000-0000000000000000001-{}.jsonl.gz",
            "a".repeat(64)
        ),
        "application/gzip",
        b"test",
    )
    .unwrap();
    let raw = germany_link
        .call(&uk, Service::LogShip, encode_put_request(&req).unwrap())
        .await
        .unwrap();
    assert_eq!(
        decode_put_response(&raw).unwrap().reply,
        LogShipReply::Refused {
            refusal: LogShipRefusal::UnknownPeer
        }
    );

    // 2. Listed peer (`oregon-1`) trying to write under another node's prefix (`uk-1/...`) is refused.
    let oregon_link = net.link(&oregon);
    let wrong_prefix = LogShipPutRequest::new(
        format!(
            "uk-1/log/0000000000000000000-0000000000000000001-{}.jsonl.gz",
            "a".repeat(64)
        ),
        "application/gzip",
        b"test",
    )
    .unwrap();
    let raw = oregon_link
        .call(&uk, Service::LogShip, encode_put_request(&wrong_prefix).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        decode_put_response(&raw).unwrap().reply,
        LogShipReply::Refused {
            refusal: LogShipRefusal::WrongPrefix { .. }
        }
    ));

    // 3. Listed peer trying to upload a body whose SHA-256 does not match its key is refused.
    let bad_hash = LogShipPutRequest::new(
        format!(
            "oregon-1/log/0000000000000000000-0000000000000000001-{}.jsonl.gz",
            "a".repeat(64)
        ),
        "application/gzip",
        b"not-that-sha256",
    )
    .unwrap();
    let raw = oregon_link
        .call(&uk, Service::LogShip, encode_put_request(&bad_hash).unwrap())
        .await
        .unwrap();
    assert_eq!(
        decode_put_response(&raw).unwrap().reply,
        LogShipReply::Refused {
            refusal: LogShipRefusal::HashMismatch
        }
    );

    // 4. Ship a real snapshot and segment, then tamper with the segment in the peer folder (or plant an
    //    entry edited in-place with its old hash) and verify restore names both the object key and entry id.
    let oregon_db_dir = tempfile::tempdir().unwrap();
    let oregon_db_cfg = DatabaseConfig::Sqlite {
        file: oregon_db_dir.path().join("oregon.db"),
    };
    let db = Database::open(
        &oregon_db_cfg,
        &DatabaseOptions {
            clock: clock_2,
            ..DatabaseOptions::default()
        },
        &SCHEMA,
    )
    .unwrap();
    write_entry(&db, "owner", "acc-1", "Alice", "oregon-1").await;

    let oregon_state_dir = tempfile::tempdir().unwrap();
    let settings = ShipSettings::new(
        PeerBucket::prefix_for(&oregon),
        oregon_state_dir.path().join("ship-peer-state.json"),
    );
    let peer_bucket = PeerBucket::new(uk.clone(), Arc::new(net.link(&oregon)));
    let mut shipper = Shipper::new(db.clone(), peer_bucket.clone(), settings).unwrap();
    shipper.tick().await.unwrap();

    set_time(&CLOCK_2, T0 + 2 * MINUTE, &uk_clock);
    write_entry(&db, "owner", "acc-2", "Bob", "oregon-1").await;
    write_entry(&db, "owner", "acc-3", "Carol", "oregon-1").await;
    shipper.ship_log().await.unwrap();

    let listed = uk_store.list("oregon-1/log/", None).await.unwrap();
    let last_seg = listed.last().unwrap().clone();

    // Re-putting the exact same content-addressed segment is idempotent (`created: false`) and keeps the
    // original `received_at`.
    let seg_bytes = uk_store.get(&last_seg.key).await.unwrap();
    set_time(&CLOCK_2, T0 + 9 * MINUTE, &uk_clock);
    let again = LogShipPutRequest::new(last_seg.key.clone(), "application/gzip", &seg_bytes).unwrap();
    let raw = oregon_link
        .call(&uk, Service::LogShip, encode_put_request(&again).unwrap())
        .await
        .unwrap();
    assert_eq!(
        decode_put_response(&raw).unwrap().reply,
        LogShipReply::Stored {
            created: false,
            received_at: last_seg.last_modified,
        }
    );

    // Replace the segment on disk with a forged segment where an entry's `after` was edited without
    // updating its hash (matching the key's outer SHA-256 so the chain check inside decode_segment fires).
    let parsed = SegmentName::parse("oregon-1/", &last_seg.key).unwrap();
    let mut entries: Vec<ChangeEntry> = history_ship::layout::decode_segment(&parsed, &seg_bytes).unwrap();
    let edited_id = entries[0].id;
    entries[0].after = Some(r#"{"account_id":"acc-2","tampered":true}"#.into());
    let (forged_name, forged_body) = encode_segment(parsed.after, &entries).unwrap();
    let forged_key = forged_name.key("oregon-1/");

    // Remove the honest segment file in the test folder and ship the forged segment over the link.
    let honest_rel = last_seg.key.strip_prefix("oregon-1/").unwrap();
    std::fs::remove_file(uk_store.origin_dir(&oregon).join(honest_rel)).unwrap();
    peer_bucket
        .put(&forged_key, forged_body, "application/gzip")
        .await
        .unwrap();

    let target_dir = tempfile::tempdir().unwrap();
    let target_cfg = DatabaseConfig::Sqlite {
        file: target_dir.path().join("restored.db"),
    };
    let scratch = tempfile::tempdir().unwrap();
    let err = restore_to(
        uk_store.as_ref(),
        "oregon-1/",
        T0 + 10 * MINUTE,
        &target_cfg,
        &SCHEMA,
        &DatabaseOptions::default(),
        &RestoreOptions::new(scratch.path()),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        err.contains(&forged_key) && err.contains(&format!("entry {edited_id} was changed after it was written")),
        "{err}"
    );
    let _ = LOG_SHIP_VERSION;
    let _ = entry_hash;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_down_peer_makes_lag_alert_fire_and_catch_up_works() {
    let oregon = id("oregon-1");
    let uk = id("uk-1");

    let net = InProcessNetwork::new();
    let uk_dir = tempfile::tempdir().unwrap();
    let uk_clock = synced_clock(&CLOCK_3);
    set_time(&CLOCK_3, T0, &uk_clock);

    let uk_store = Arc::new(PeerLogStore::new(
        uk_dir.path().join("peer-logs"),
        [oregon.clone()],
        uk_clock.clone(),
    ));
    let uk_mux: Arc<dyn NodeService> = Arc::new(ServiceMux::new(uk.clone()).with(Service::LogShip, uk_store.clone()));
    net.attach(&uk, &uk_mux);

    let oregon_db_dir = tempfile::tempdir().unwrap();
    let oregon_db_cfg = DatabaseConfig::Sqlite {
        file: oregon_db_dir.path().join("oregon.db"),
    };
    let db = Database::open(
        &oregon_db_cfg,
        &DatabaseOptions {
            clock: clock_3,
            ..DatabaseOptions::default()
        },
        &SCHEMA,
    )
    .unwrap();
    write_entry(&db, "owner", "acc-1", "Alice", "oregon-1").await;

    let oregon_state_dir = tempfile::tempdir().unwrap();
    let settings = ShipSettings::new(
        PeerBucket::prefix_for(&oregon),
        oregon_state_dir.path().join("ship-peer-state.json"),
    );
    let limits = LagLimits::default();
    let peer_bucket = PeerBucket::new(uk.clone(), Arc::new(net.link(&oregon)));
    let mut shipper = Shipper::new(db.clone(), peer_bucket, settings.clone()).unwrap();
    shipper.tick().await.unwrap();

    let fresh = lag(&db, &settings.state_file, &limits).await.unwrap();
    assert!(!fresh.behind(), "{fresh:?}");

    // Cut the link to `uk-1`, write a new change on `oregon-1`, and advance past `max_unshipped` (5 minutes).
    net.cut(&oregon, &uk);
    set_time(&CLOCK_3, T0 + MINUTE, &uk_clock);
    write_entry(&db, "owner", "acc-2", "Bob", "oregon-1").await;

    set_time(&CLOCK_3, T0 + 7 * MINUTE, &uk_clock);
    assert!(shipper.tick().await.is_err());

    let failing = lag(&db, &settings.state_file, &limits).await.unwrap();
    assert!(failing.behind(), "{failing:?}");
    assert!(
        failing
            .alerts
            .iter()
            .any(|a| matches!(a, LagAlert::Unshipped { since, .. } if *since == T0 + MINUTE)),
        "{failing:?}"
    );
    assert!(
        failing.alerts.iter().any(|a| matches!(a, LagAlert::Failing { .. })),
        "{failing:?}"
    );

    // Heal the link; the next tick catches up, clears the lag alerts, and UK's folder can restore the full state.
    net.heal();
    shipper.tick().await.unwrap();
    let caught_up = lag(&db, &settings.state_file, &limits).await.unwrap();
    assert!(!caught_up.behind(), "{caught_up:?}");

    let restored_dir = tempfile::tempdir().unwrap();
    let restored_cfg = DatabaseConfig::Sqlite {
        file: restored_dir.path().join("restored.db"),
    };
    let scratch = tempfile::tempdir().unwrap();
    restore_to(
        uk_store.as_ref(),
        "oregon-1/",
        T0 + 8 * MINUTE,
        &restored_cfg,
        &SCHEMA,
        &DatabaseOptions::default(),
        &RestoreOptions::new(scratch.path()),
    )
    .await
    .unwrap();
    assert_eq!(
        transfer::dump(&restored_cfg).unwrap(),
        transfer::dump(&oregon_db_cfg).unwrap()
    );
}
