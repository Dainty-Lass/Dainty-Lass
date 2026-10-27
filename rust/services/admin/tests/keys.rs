//! The keys on both engines (DD15, DH2). `REFORGED_TEST_POSTGRES_URL` names a server the test may create a
//! database on (`postgres://user:pw@127.0.0.1:5432/postgres`); without it only SQLite runs.

#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use admin::keys::{Action, AuthError, Keys, Role};
use diesel::connection::SimpleConnection;
use diesel::{Connection, PgConnection};
use store::{Actor, DatabaseConfig, PostgresConfig, portable};

struct PgDatabase {
    server: PostgresConfig,
    config: PostgresConfig,
}

impl Drop for PgDatabase {
    fn drop(&mut self) {
        if let Ok(mut conn) = PgConnection::establish(&self.server.conninfo()) {
            let _ = conn.batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                portable::quote_ident(&self.config.database)
            ));
        }
    }
}

fn new_postgres() -> Option<PgDatabase> {
    let url = url::Url::parse(&std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?).expect("a postgres:// URL");
    let server = PostgresConfig {
        host: url.host_str().unwrap_or("127.0.0.1").to_owned(),
        port: url.port().unwrap_or(5432),
        user: url.username().to_owned(),
        database: url.path().trim_start_matches('/').to_owned(),
        password: url.password().unwrap_or_default().to_owned(),
        allow_remote: false,
    };
    let name = format!("admin_keys_test_{}", std::process::id());
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

async fn exercise(config: DatabaseConfig) {
    const OWNER: &str = "owner-key-0123456789abcdef0123456789";
    let keys = tokio::task::spawn_blocking(move || Keys::open(&config, OWNER))
        .await
        .unwrap()
        .unwrap();
    let host = Actor::Host("admin key issue".into());

    assert_eq!(keys.authenticate(OWNER).await, Ok(("owner".to_owned(), Role::Owner)));
    assert_eq!(keys.authenticate("nope").await, Err(AuthError::Unknown));

    let (info, key) = keys.issue("Maya", Role::Moderator, host.clone(), 1000).await.unwrap();
    assert_eq!(keys.authenticate(&key).await, Ok((info.id.clone(), Role::Moderator)));
    assert!(
        keys.issue("maya", Role::Viewer, host.clone(), 1001).await.is_err(),
        "one active key per name"
    );

    let entry = Action {
        operator: "42",
        action: "ban",
        target: Some("Bo_Slayer"),
        game: Some("1.14.7"),
        detail: Some("griefing hunts"),
    };
    keys.record(&info.id, entry, 1002).await.unwrap();

    let revoked = keys.revoke(&info.id, Actor::Admin("owner".into()), 1003).await.unwrap();
    assert_eq!(revoked.revoked_by.as_deref(), Some("admin:owner"));
    assert_eq!(keys.authenticate(&key).await, Err(AuthError::Revoked(info.id.clone())));
    assert!(keys.revoke(&info.id, host.clone(), 1004).await.is_err());
    // A name can be given again once its key is revoked
    keys.issue("Maya", Role::Viewer, host, 1005).await.unwrap();

    let listed = keys.list().await.unwrap();
    assert_eq!(
        listed.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(),
        ["REFORGED_ADMIN_API_KEY", "Maya", "Maya"]
    );

    // Every change is in the log under whoever made it, with the key's hash and never the key
    let log = keys.database().changes(0, 100).await.unwrap();
    let actors: Vec<(&str, &str, &str)> = log
        .iter()
        .filter(|e| e.table_name.starts_with("admin_"))
        .map(|e| (e.table_name.as_str(), e.op.as_str(), e.actor.as_str()))
        .collect();
    assert_eq!(
        actors,
        [
            ("admin_keys", "insert", "host:admin key issue"),
            ("admin_actions", "insert", &format!("admin:{}", info.id)),
            ("admin_keys", "update", "admin:owner"),
            ("admin_keys", "insert", "host:admin key issue"),
        ]
    );
    assert!(log.iter().all(|e| !e.after.as_deref().unwrap_or("").contains(&key)));
}

#[tokio::test(flavor = "multi_thread")]
async fn keys_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    exercise(DatabaseConfig::Sqlite {
        file: dir.path().join("nested").join("admin.db"),
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn keys_on_postgres() {
    let Some(pg) = new_postgres() else {
        eprintln!("REFORGED_TEST_POSTGRES_URL is not set: skipping the Postgres half");
        return;
    };
    exercise(DatabaseConfig::Postgres(pg.config.clone())).await;
}
