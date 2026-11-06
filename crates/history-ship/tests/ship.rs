//! Shipping and restore against an in-memory object-locked bucket (DH4's Verify): restore to a minute before
//! a delete, on a copy, into either engine; with shipping stopped, the lag alert fires. SQLite always;
//! Postgres too when `REFORGED_TEST_POSTGRES_URL` names a server the tests may create databases on, as in
//! `store`'s tests.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Binary, Double, Integer, Nullable, Text};
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use history_ship::layout::{SegmentName, encode_segment};
use history_ship::{
    Bucket, BucketRead, LagAlert, LagLimits, MemoryBucket, RestoreOptions, ShipSettings, Shipper, lag, restore_to,
};
use store::PgConnection;
use store::history::{Head, entry_hash};
use store::{
    Actor, ChangeEntry, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema, PostgresConfig, portable, transfer,
};

const V1_SQLITE: EmbeddedMigrations = embed_migrations!("tests/db/v1/sqlite");
const V1_POSTGRES: EmbeddedMigrations = embed_migrations!("tests/db/v1/postgres");
const V2_SQLITE: EmbeddedMigrations = embed_migrations!("tests/db/v2/sqlite");
const V2_POSTGRES: EmbeddedMigrations = embed_migrations!("tests/db/v2/postgres");
const V1: DatabaseSchema = DatabaseSchema {
    sqlite: || V1_SQLITE,
    postgres: || V1_POSTGRES,
    soft_delete: &["accounts"],
};
const V2: DatabaseSchema = DatabaseSchema {
    sqlite: || V2_SQLITE,
    postgres: || V2_POSTGRES,
    soft_delete: &["accounts"],
};

const MINUTE: i64 = 60_000;
/// 2026-10-26T00:00:00Z
const T0: i64 = 1_761_436_800_000;

// ------------------------------------------------------------------ the engines

struct PgDatabase {
    server: PostgresConfig,
    config: PostgresConfig,
}

impl Drop for PgDatabase {
    fn drop(&mut self) {
        if let Ok(mut admin) = PgConnection::establish(&self.server.conninfo()) {
            let _ = admin.batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                portable::quote_ident(&self.config.database)
            ));
        }
    }
}

static NEXT: AtomicU32 = AtomicU32::new(0);

fn new_postgres() -> Option<PgDatabase> {
    let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
    let url = url::Url::parse(&url).expect("REFORGED_TEST_POSTGRES_URL is a postgres:// URL");
    let server = PostgresConfig {
        host: url.host_str().unwrap_or("127.0.0.1").to_owned(),
        port: url.port().unwrap_or(5432),
        user: url.username().to_owned(),
        database: url.path().trim_start_matches('/').to_owned(),
        password: url.password().unwrap_or_default().to_owned(),
        allow_remote: false,
    };
    let name = format!(
        "ship_test_{}_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    );
    PgConnection::establish(&server.conninfo())
        .expect("REFORGED_TEST_POSTGRES_URL is reachable")
        .batch_execute(&format!("CREATE DATABASE {}", portable::quote_ident(&name)))
        .unwrap();
    let config = PostgresConfig {
        database: name,
        ..server.clone()
    };
    Some(PgDatabase { server, config })
}

enum Target {
    Sqlite(tempfile::TempDir),
    Postgres(PgDatabase),
}

impl Target {
    fn config(&self) -> DatabaseConfig {
        match self {
            Self::Sqlite(dir) => DatabaseConfig::Sqlite {
                file: dir.path().join("game.db"),
            },
            Self::Postgres(pg) => DatabaseConfig::Postgres(pg.config.clone()),
        }
    }

    fn config_path(&self) -> String {
        match self {
            Self::Sqlite(dir) => dir.path().join("game.db").to_string_lossy().into_owned(),
            Self::Postgres(_) => String::new(),
        }
    }

    fn config_file_exists(&self) -> bool {
        match self {
            Self::Sqlite(dir) => dir.path().join("game.db").exists(),
            Self::Postgres(_) => false,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "sqlite",
            Self::Postgres(_) => "postgres",
        }
    }
}

fn sqlite() -> Target {
    Target::Sqlite(tempfile::tempdir().unwrap())
}

fn engines() -> Vec<Target> {
    let mut out = vec![sqlite()];
    match new_postgres() {
        Some(pg) => out.push(Target::Postgres(pg)),
        None => eprintln!("REFORGED_TEST_POSTGRES_URL is not set: Postgres skipped"),
    }
    out
}

// ------------------------------------------------------------------ clocks and writes

/// Each test moves its own clock: the database's `at`, the shipper's times and the bucket's `last_modified`.
macro_rules! clock {
    ($cell:ident, $read:ident) => {
        static $cell: AtomicI64 = AtomicI64::new(T0);
        fn $read() -> i64 {
            $cell.load(Ordering::SeqCst)
        }
    };
}

fn open(target: &Target, schema: &DatabaseSchema, clock: fn() -> i64) -> Database {
    let options = DatabaseOptions {
        clock,
        ..DatabaseOptions::default()
    };
    Database::open(&target.config(), &options, schema).unwrap()
}

async fn exec(db: &Database, sql: &'static str, binds: Vec<Bind>) {
    db.write(Actor::Player("1".into()), move |tx| {
        let mut query = diesel::sql_query(sql).into_boxed::<store::MultiBackend>();
        for bind in binds {
            query = match bind {
                Bind::Int(n) => query.bind::<BigInt, _>(n),
                Bind::Small(n) => query.bind::<Integer, _>(n),
                Bind::Text(t) => query.bind::<Text, _>(t),
                Bind::Real(r) => query.bind::<Nullable<Double>, _>(r),
                Bind::Blob(b) => query.bind::<Binary, _>(b),
            };
        }
        query.execute(tx.conn())?;
        Ok::<_, store::Error>(())
    })
    .await
    .unwrap();
}

enum Bind {
    Int(i64),
    Small(i32),
    Text(String),
    Real(Option<f64>),
    Blob(Vec<u8>),
}

fn text(s: &str) -> Bind {
    Bind::Text(s.into())
}

type Dump = BTreeMap<String, Vec<serde_json::Value>>;

fn settings(dir: &tempfile::TempDir) -> ShipSettings {
    let mut settings = ShipSettings::new("srv-1/", dir.path().join("ship-state.json"));
    settings.snapshot_every = std::time::Duration::from_secs(15 * 60);
    settings
}

/// The story every restore test reads back. Returns the database as it was a minute before the delete, at
/// the end, and the delete's time.
async fn story(
    source: &Target,
    bucket: &MemoryBucket,
    state: &tempfile::TempDir,
    cell: &AtomicI64,
    clock: fn() -> i64,
) -> (Dump, Dump, i64) {
    cell.store(T0, Ordering::SeqCst);
    let db = open(source, &V1, clock);
    exec(
        &db,
        "INSERT INTO accounts (id, name, is_admin, score) VALUES (1, 'alice', TRUE, $1)",
        vec![Bind::Real(Some(2.5))],
    )
    .await;
    exec(
        &db,
        "INSERT INTO accounts (id, name, avatar) VALUES (2, 'bob', $1)",
        vec![Bind::Blob(vec![0, 255, 16])],
    )
    .await;
    for (slot, item) in [(1, "sword"), (2, "shield")] {
        exec(
            &db,
            "INSERT INTO items (account_id, slot, name) VALUES (2, $1, $2)",
            vec![Bind::Small(slot), text(item)],
        )
        .await;
    }
    let mut shipper = Shipper::new(db.clone(), bucket.clone(), settings(state)).unwrap();
    shipper.tick().await.unwrap(); // the first snapshot

    cell.store(T0 + 10 * MINUTE, Ordering::SeqCst);
    exec(
        &db,
        "UPDATE accounts SET score = $1 WHERE id = 1",
        vec![Bind::Real(Some(7.25))],
    )
    .await;
    shipper.tick().await.unwrap();

    cell.store(T0 + 16 * MINUTE, Ordering::SeqCst);
    shipper.tick().await.unwrap(); // the second snapshot, the one the restore starts from
    drop(shipper);
    drop(db);

    // An upgrade: the migration is logged where it ran, and its data change rides on it.
    cell.store(T0 + 20 * MINUTE, Ordering::SeqCst);
    let db = open(source, &V2, clock);
    exec(
        &db,
        "INSERT INTO notes (account_id, body) VALUES (2, $1)",
        vec![text("bob's note")],
    )
    .await;
    exec(
        &db,
        "UPDATE accounts SET deleted_at = $1 WHERE id = 1",
        vec![Bind::Int(T0 + 20 * MINUTE)],
    )
    .await;
    let mut shipper = Shipper::new(db.clone(), bucket.clone(), settings(state)).unwrap();
    shipper.tick().await.unwrap();

    cell.store(T0 + 29 * MINUTE, Ordering::SeqCst);
    let before_delete = transfer::dump(&source.config()).unwrap();

    // The delete a partner makes: bob, his items by cascade, his note.
    cell.store(T0 + 30 * MINUTE, Ordering::SeqCst);
    let delete_at = T0 + 30 * MINUTE;
    db.write(Actor::Admin("partner".into()), |tx| {
        tx.conn()
            .batch_execute("DELETE FROM notes; DELETE FROM accounts WHERE id = 2;")?;
        Ok::<_, store::Error>(())
    })
    .await
    .unwrap();
    shipper.tick().await.unwrap();

    cell.store(T0 + 33 * MINUTE, Ordering::SeqCst);
    shipper.tick().await.unwrap(); // a third snapshot, after the delete
    cell.store(T0 + 35 * MINUTE, Ordering::SeqCst);
    exec(&db, "INSERT INTO accounts (id, name) VALUES (3, 'carol')", vec![]).await;
    cell.store(T0 + 36 * MINUTE, Ordering::SeqCst);
    shipper.tick().await.unwrap();
    let end = transfer::dump(&source.config()).unwrap();
    (before_delete, end, delete_at)
}

clock!(RESTORE_CELL, restore_clock);

#[tokio::test(flavor = "multi_thread")]
async fn restore_to_a_minute_before_a_delete_on_either_engine() {
    for source in engines() {
        let bucket = MemoryBucket::new(restore_clock);
        let state = tempfile::tempdir().unwrap();
        let (before_delete, end, delete_at) = story(&source, &bucket, &state, &RESTORE_CELL, restore_clock).await;
        let accounts = |dump: &Dump| {
            let mut names: Vec<_> = dump["accounts"].iter().map(|r| r["name"].to_string()).collect();
            names.sort();
            names
        };
        assert_eq!(accounts(&before_delete), ["\"alice\"", "\"bob\""]);
        assert_eq!(accounts(&end), ["\"alice\"", "\"carol\""]);

        for target in engines() {
            let scratch = tempfile::tempdir().unwrap();
            let outcome = restore_to(
                &bucket,
                "srv-1/",
                delete_at - MINUTE,
                &target.config(),
                &V2,
                &DatabaseOptions::default(),
                &RestoreOptions::new(scratch.path()),
            )
            .await
            .unwrap();
            let what = format!("{} to {}", source.name(), target.name());
            assert!(
                outcome
                    .snapshot
                    .contains(&format!("/snapshot/{:013}-", T0 + 16 * MINUTE)),
                "{what}: {outcome:?}"
            );
            assert!(outcome.report.replayed >= 3, "{what}: {outcome:?}");
            assert!(outcome.report.last_at.unwrap() < delete_at - MINUTE, "{what}");
            assert_eq!(transfer::dump(&target.config()).unwrap(), before_delete, "{what}");
            // The restored database is a working one: its writes are logged again.
            let db = open(&target, &V2, restore_clock);
            exec(&db, "UPDATE accounts SET name = 'robert' WHERE id = 2", vec![]).await;
            assert_eq!(db.changes(outcome.report.last_id, 10).await.unwrap().len(), 1, "{what}");
        }

        // And to the end, through the cascade and from the third snapshot.
        let target = sqlite();
        let scratch = tempfile::tempdir().unwrap();
        let outcome = restore_to(
            &bucket,
            "srv-1/",
            T0 + 40 * MINUTE,
            &target.config(),
            &V2,
            &DatabaseOptions::default(),
            &RestoreOptions::new(scratch.path()),
        )
        .await
        .unwrap();
        assert!(
            outcome
                .snapshot
                .contains(&format!("/snapshot/{:013}-", T0 + 33 * MINUTE)),
            "{outcome:?}"
        );
        assert_eq!(
            transfer::dump(&target.config()).unwrap(),
            end,
            "{} to the end",
            source.name()
        );
    }
}

clock!(LAG_CELL, lag_clock);

#[tokio::test(flavor = "multi_thread")]
async fn with_shipping_stopped_the_lag_alert_fires() {
    let source = sqlite();
    let bucket = MemoryBucket::new(lag_clock);
    let state = tempfile::tempdir().unwrap();
    let db = open(&source, &V1, lag_clock);
    exec(&db, "INSERT INTO accounts (id, name) VALUES (1, 'alice')", vec![]).await;
    let settings = settings(&state);
    let limits = LagLimits::default();
    let mut shipper = Shipper::new(db.clone(), bucket.clone(), settings.clone()).unwrap();
    shipper.tick().await.unwrap();
    let fresh = lag(&db, &settings.state_file, &limits).await.unwrap();
    assert!(!fresh.behind(), "{fresh:?}");
    assert_eq!(fresh.last_shipped_id, fresh.newest_id);

    // Shipping stops (the process is killed); writes go on.
    drop(shipper);
    LAG_CELL.store(T0 + MINUTE, Ordering::SeqCst);
    exec(&db, "INSERT INTO accounts (id, name) VALUES (2, 'bob')", vec![]).await;
    let soon = lag(&db, &settings.state_file, &limits).await.unwrap();
    assert!(!soon.behind(), "within the limit: {soon:?}");
    LAG_CELL.store(T0 + 7 * MINUTE, Ordering::SeqCst);
    let late = lag(&db, &settings.state_file, &limits).await.unwrap();
    assert!(late.behind());
    assert!(
        late.alerts
            .iter()
            .any(|a| matches!(a, LagAlert::Unshipped { since, .. } if *since == T0 + MINUTE))
    );
    assert!(late.alerts.iter().any(|a| matches!(a, LagAlert::Silent { .. })));

    // A shipper that runs but can't reach the bucket: still behind, and says why.
    bucket.set_offline(true);
    let mut shipper = Shipper::new(db.clone(), bucket.clone(), settings.clone()).unwrap();
    assert!(shipper.tick().await.is_err());
    let failing = lag(&db, &settings.state_file, &limits).await.unwrap();
    assert!(failing.behind());
    assert!(failing.alerts.iter().any(|a| matches!(a, LagAlert::Failing { .. })));
    assert!(!failing.alerts.iter().any(|a| matches!(a, LagAlert::Silent { .. })));

    // Back online, it catches up and the alert clears.
    bucket.set_offline(false);
    shipper.tick().await.unwrap();
    let caught_up = lag(&db, &settings.state_file, &limits).await.unwrap();
    assert!(!caught_up.behind(), "{caught_up:?}");
}

clock!(TAMPER_CELL, tamper_clock);

async fn segment_of(bucket: &MemoryBucket, key: &str) -> (SegmentName, Vec<u8>) {
    (
        SegmentName::parse("srv-1/", key).unwrap(),
        bucket.get(key).await.unwrap(),
    )
}

/// The entries sealed again from the first one's `prev_hash`, as someone with the database file could.
fn rehash(entries: &mut [ChangeEntry]) {
    let mut prev = entries[0].prev_hash.clone().unwrap();
    for entry in entries {
        entry.prev_hash = Some(prev.clone());
        entry.hash = Some(entry_hash(&prev, entry));
        prev = entry.hash.clone().unwrap();
    }
}

/// Another bucket holding the same objects, for a test to plant things in.
async fn copy(bucket: &MemoryBucket) -> MemoryBucket {
    let out = MemoryBucket::new(tamper_clock);
    for object in bucket.list("", None).await.unwrap() {
        out.put_at(
            &object.key,
            bucket.get(&object.key).await.unwrap(),
            object.last_modified,
        );
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn planted_objects_and_holes_are_named_not_trusted() {
    let source = sqlite();
    let bucket = MemoryBucket::new(tamper_clock);
    let state = tempfile::tempdir().unwrap();
    let (before_delete, _, delete_at) = story(&source, &bucket, &state, &TAMPER_CELL, tamper_clock).await;
    let restore = |bucket: MemoryBucket, options: RestoreOptions| async move {
        let target = sqlite();
        let result = restore_to(
            &bucket,
            "srv-1/",
            delete_at - MINUTE,
            &target.config(),
            &V2,
            &DatabaseOptions::default(),
            &options,
        )
        .await;
        (result, target)
    };
    let scratch = tempfile::tempdir().unwrap();
    let (clean, _) = restore(bucket.clone(), RestoreOptions::new(scratch.path())).await;
    let used = clean.unwrap().segments;
    assert_eq!(used.len(), 2, "{used:?}");
    let objects = bucket.list("srv-1/log/", None).await.unwrap();
    let segment = |key: &str| {
        let info = objects.iter().find(|o| o.key == key).unwrap().clone();
        (SegmentName::parse("srv-1/", key).unwrap(), info)
    };

    // A planted segment that rewrites an entry the real one holds, dated back: the two disagree.
    let (real, info) = &segment(&used[0]);
    let body = bucket.get(&info.key).await.unwrap();
    let mut entries = history_ship::layout::decode_segment(real, &body).unwrap();
    entries[0].actor = "player:someone-else".into();
    rehash(&mut entries);
    let (fake, fake_body) = encode_segment(real.after, &entries).unwrap();
    let planted = copy(&bucket).await;
    planted.put_at(&fake.key("srv-1/"), fake_body, info.last_modified);
    let (result, _) = restore(planted.clone(), RestoreOptions::new(scratch.path())).await;
    let error = result.unwrap_err().to_string();
    assert!(error.contains("disagree"), "{error}");
    let mut skip = RestoreOptions::new(scratch.path());
    skip.skip.push(fake.key("srv-1/"));
    let (result, target) = restore(planted, skip).await;
    result.unwrap();
    assert_eq!(transfer::dump(&target.config()).unwrap(), before_delete);

    // An entry edited in a segment, its hash left as it was: the restore fails naming the entry and the object.
    let mut edited = history_ship::layout::decode_segment(real, &body).unwrap();
    let victim = edited[1].id;
    edited[1].after = Some(r#"{"id":1,"name":"edited"}"#.into());
    let (forged, forged_body) = encode_segment(real.after, &edited).unwrap();
    let swapped_in = copy(&bucket).await;
    swapped_in.forget(&info.key);
    swapped_in.put_at(&forged.key("srv-1/"), forged_body, info.last_modified);
    let (result, target) = restore(swapped_in, RestoreOptions::new(scratch.path())).await;
    let error = result.unwrap_err().to_string();
    assert!(
        error.contains(&forged.key("srv-1/"))
            && error.contains(&format!("entry {victim} was changed after it was written")),
        "{error}"
    );
    assert!(!target.config_file_exists(), "a failed restore leaves no database");

    // A segment rewritten consistently from an entry on (hashes recomputed) leaves the next one not following it.
    let mut rewritten = history_ship::layout::decode_segment(real, &body).unwrap();
    rewritten[0].actor = "player:someone-else".into();
    rehash(&mut rewritten);
    let (fake, fake_body) = encode_segment(real.after, &rewritten).unwrap();
    let rewritten_in = copy(&bucket).await;
    rewritten_in.forget(&info.key);
    rewritten_in.put_at(&fake.key("srv-1/"), fake_body, info.last_modified);
    let (result, _) = restore(rewritten_in, RestoreOptions::new(scratch.path())).await;
    let error = result.unwrap_err().to_string();
    assert!(error.contains("doesn't follow entry"), "{error}");

    // The newest segment, rewritten consistently, shows only against a head the owner kept off the server.
    let end = T0 + 40 * MINUTE;
    let to_end = |bucket: MemoryBucket, options: RestoreOptions| {
        let scratch = scratch.path().to_owned();
        async move {
            let target = sqlite();
            let result = restore_to(
                &bucket,
                "srv-1/",
                end,
                &target.config(),
                &V2,
                &DatabaseOptions::default(),
                &RestoreOptions { scratch, ..options },
            )
            .await;
            (result, target)
        }
    };
    let newest = bucket.list("srv-1/log/", None).await.unwrap().pop().unwrap();
    let (tail_name, _) = segment_of(&bucket, &newest.key).await;
    let tail_body = bucket.get(&newest.key).await.unwrap();
    let mut tail = history_ship::layout::decode_segment(&tail_name, &tail_body).unwrap();
    let last = tail.last().unwrap();
    let head = Head {
        id: last.id,
        hash: last.hash.clone().unwrap(),
    };
    tail[0].actor = "player:someone-else".into();
    rehash(&mut tail);
    let (fake_tail, fake_tail_body) = encode_segment(tail_name.after, &tail).unwrap();
    let tail_in = copy(&bucket).await;
    tail_in.forget(&newest.key);
    tail_in.put_at(&fake_tail.key("srv-1/"), fake_tail_body, newest.last_modified);
    let (unwitnessed, _) = to_end(tail_in.clone(), RestoreOptions::new(scratch.path())).await;
    unwitnessed.unwrap(); // the chain alone can't tell
    let mut witnessed = RestoreOptions::new(scratch.path());
    witnessed.trusted_head = Some(head.clone());
    let (result, _) = to_end(tail_in, witnessed).await;
    let error = result.unwrap_err().to_string();
    assert!(
        error.contains(&format!("entry {} isn't the trusted head any more", head.id)),
        "{error}"
    );
    let mut honest = RestoreOptions::new(scratch.path());
    honest.trusted_head = Some(head);
    to_end(bucket.clone(), honest).await.0.unwrap();

    // A snapshot planted after the fact under an earlier name is outside the trust window.
    let late = copy(&bucket).await;
    let snapshots = late.list("srv-1/snapshot/", None).await.unwrap();
    let newest = snapshots.last().unwrap();
    let mut stolen = history_ship::layout::SnapshotName::parse("srv-1/", &newest.key).unwrap();
    stolen.taken_at = delete_at - 2 * MINUTE;
    late.put_at(
        &stolen.key("srv-1/"),
        late.get(&newest.key).await.unwrap(),
        T0 + 24 * 60 * MINUTE,
    );
    let (result, target) = restore(late, RestoreOptions::new(scratch.path())).await;
    let outcome = result.unwrap();
    assert!(outcome.ignored.contains(&stolen.key("srv-1/")), "{outcome:?}");
    assert_eq!(transfer::dump(&target.config()).unwrap(), before_delete);

    // A segment missing from the middle is a gap, not a quiet stop.
    let holed = copy(&bucket).await;
    holed.forget(&used[1]);
    let (result, _) = restore(holed, RestoreOptions::new(scratch.path())).await;
    let error = result.unwrap_err().to_string();
    assert!(error.contains("gap"), "{error}");

    // A body swapped under its name.
    let swapped = MemoryBucket::new(tamper_clock);
    for object in bucket.list("", None).await.unwrap() {
        let mut body = bucket.get(&object.key).await.unwrap();
        if object.key.contains("/snapshot/") {
            body.push(0);
        }
        swapped.put_at(&object.key, body, object.last_modified);
    }
    let (result, _) = restore(swapped, RestoreOptions::new(scratch.path())).await;
    assert!(result.unwrap_err().to_string().contains("hash"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_only_ever_puts() {
    /// A bucket that fails the test if anything but a put reaches it.
    struct WriteOnly(MemoryBucket);
    impl Bucket for WriteOnly {
        async fn put(&self, key: &str, body: Vec<u8>, content_type: &'static str) -> Result<(), history_ship::Error> {
            self.0.put(key, body, content_type).await
        }
    }
    let source = sqlite();
    let db = open(&source, &V1, store_now);
    exec(&db, "INSERT INTO accounts (id, name) VALUES (1, 'alice')", vec![]).await;
    let state = tempfile::tempdir().unwrap();
    let memory = MemoryBucket::new(store_now);
    let mut shipper = Shipper::new(db.clone(), WriteOnly(memory.clone()), settings(&state)).unwrap();
    shipper.tick().await.unwrap();
    // A re-shipped key (a lost state file) keeps the version before it, as object lock does.
    std::fs::remove_file(state.path().join("ship-state.json")).unwrap();
    let mut shipper = Shipper::new(db, WriteOnly(memory.clone()), settings(&state)).unwrap();
    shipper.ship_log().await.unwrap();
    let keys = memory.list("srv-1/log/", None).await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(memory.versions(&keys[0].key).len(), 2);
}

clock!(BREAK_CELL, break_clock);

#[tokio::test(flavor = "multi_thread")]
async fn a_local_log_that_no_longer_chains_is_not_shipped_on_and_alerts() {
    let source = sqlite();
    let bucket = MemoryBucket::new(break_clock);
    let state = tempfile::tempdir().unwrap();
    let db = open(&source, &V1, break_clock);
    exec(&db, "INSERT INTO accounts (id, name) VALUES (1, 'alice')", vec![]).await;
    let settings = settings(&state);
    let mut shipper = Shipper::new(db.clone(), bucket.clone(), settings.clone()).unwrap();
    shipper.tick().await.unwrap();
    let shipped = shipper.state().last_shipped_id;
    assert!(shipper.state().last_shipped_hash.is_some());

    // Someone with the file edits the newest entry (dropping the update guard first) and writes on.
    exec(&db, "INSERT INTO accounts (id, name) VALUES (2, 'bob')", vec![]).await;
    let mut raw = SqliteConnection::establish(&source.config_path()).unwrap();
    raw.batch_execute(&format!(
        "DROP TRIGGER \"__store_change_log_no_update\"; \
         UPDATE \"__store_change_log\" SET \"after\" = '{{\"id\":2,\"name\":\"eve\"}}' WHERE id = {};",
        shipped + 1
    ))
    .unwrap();
    drop(raw);
    let before = bucket.list("", None).await.unwrap().len();
    let error = shipper.tick().await.unwrap_err().to_string();
    assert!(
        error.contains(&format!("entry {} was changed after it was written", shipped + 1)),
        "{error}"
    );
    assert_eq!(
        bucket.list("", None).await.unwrap().len(),
        before,
        "nothing was shipped on"
    );
    let lag = lag(&db, &settings.state_file, &LagLimits::default()).await.unwrap();
    assert!(lag.behind());
    assert!(
        lag.alerts.iter().any(|a| matches!(a, LagAlert::HistoryBroken { .. })),
        "{lag:?}"
    );
}

fn store_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
