//! Shipping and restore against a real S3 server: opt-in, with `REFORGED_TEST_S3` naming a throwaway MinIO
//! on this machine and its root credentials (`http://user:password@127.0.0.1:9000`). The test makes its own
//! bucket with object lock and a one-day GOVERNANCE retention. Skipped otherwise.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::atomic::{AtomicI64, Ordering};

use diesel::connection::SimpleConnection;
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use history_ship::{BucketRead, LagLimits, RestoreOptions, S3Bucket, S3Config, ShipSettings, Shipper, lag, restore_to};
use store::{Actor, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema};

const SQLITE: EmbeddedMigrations = embed_migrations!("tests/db/v1/sqlite");
const POSTGRES: EmbeddedMigrations = embed_migrations!("tests/db/v1/postgres");
const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE,
    postgres: || POSTGRES,
    soft_delete: &["accounts"],
};
const MINUTE: i64 = 60_000;

static CELL: AtomicI64 = AtomicI64::new(0);
fn clock() -> i64 {
    CELL.load(Ordering::SeqCst)
}

fn now() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn minio() -> Option<S3Config> {
    let Ok(url) = std::env::var("REFORGED_TEST_S3") else {
        eprintln!("REFORGED_TEST_S3 is not set: the MinIO test is skipped");
        return None;
    };
    let url = url::Url::parse(&url).expect("REFORGED_TEST_S3 is http://user:password@127.0.0.1:port");
    Some(S3Config {
        endpoint: format!(
            "{}://{}:{}",
            url.scheme(),
            url.host_str().unwrap(),
            url.port().unwrap_or(9000)
        ),
        region: "us-east-1".into(),
        bucket: format!("dh4-{}-{}", std::process::id(), now()),
        path_style: true,
        access_key_id: url.username().to_owned(),
        secret_access_key: url.password().unwrap_or_default().to_owned(),
    })
}

async fn write(db: &Database, sql: &'static str) {
    db.write(Actor::Admin("owner".into()), move |tx| {
        tx.conn().batch_execute(sql)?;
        Ok::<_, store::Error>(())
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn ships_to_and_restores_from_an_object_locked_minio_bucket() {
    let Some(config) = minio() else { return };
    let bucket = S3Bucket::new(config).unwrap();
    bucket.create_locked("GOVERNANCE", 1).await.unwrap();

    // Times near the bucket's own, which a restore checks objects against.
    let start = now();
    CELL.store(start, Ordering::SeqCst);
    let dir = tempfile::tempdir().unwrap();
    let source = DatabaseConfig::Sqlite {
        file: dir.path().join("game.db"),
    };
    let options = DatabaseOptions {
        clock,
        ..DatabaseOptions::default()
    };
    let db = Database::open(&source, &options, &SCHEMA).unwrap();
    write(&db, "INSERT INTO accounts (id, name) VALUES (1, 'alice'), (2, 'bob');").await;
    write(
        &db,
        "INSERT INTO items (account_id, slot, name) VALUES (2, 1, 'sword');",
    )
    .await;
    let settings = ShipSettings::new("node-a/", dir.path().join("ship-state.json"));
    let mut shipper = Shipper::new(db.clone(), bucket.clone(), settings.clone()).unwrap();
    shipper.tick().await.unwrap();

    CELL.store(start + 10 * MINUTE, Ordering::SeqCst);
    write(&db, "DELETE FROM accounts WHERE id = 2;").await;
    shipper.tick().await.unwrap();
    let shipped = lag(&db, &settings.state_file, &LagLimits::default()).await.unwrap();
    assert!(!shipped.behind(), "{shipped:?}");

    let listed = bucket.list("node-a/", None).await.unwrap();
    assert!(
        listed.iter().any(|o| o.key.starts_with("node-a/snapshot/")),
        "{listed:?}"
    );
    assert!(
        listed.iter().filter(|o| o.key.starts_with("node-a/log/")).count() >= 2,
        "{listed:?}"
    );

    for (to, bob) in [(start + 9 * MINUTE, true), (start + 11 * MINUTE, false)] {
        let target_dir = tempfile::tempdir().unwrap();
        let target = DatabaseConfig::Sqlite {
            file: target_dir.path().join("restored.db"),
        };
        let outcome = restore_to(
            &bucket,
            "node-a/",
            to,
            &target,
            &SCHEMA,
            &DatabaseOptions::default(),
            &RestoreOptions::new(target_dir.path()),
        )
        .await
        .unwrap();
        let dump = store::transfer::dump(&target).unwrap();
        let names: Vec<_> = dump["accounts"].iter().map(|r| r["name"].clone()).collect();
        assert_eq!(names.contains(&serde_json::json!("bob")), bob, "{outcome:?}");
        assert_eq!(dump["items"].len(), usize::from(bob));
    }
}
