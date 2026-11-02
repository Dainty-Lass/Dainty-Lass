// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Chat's database on both engines (DD15). `REFORGED_TEST_POSTGRES_URL` names a server the test may create a
//! database on, as `store`'s tests read it; without it the Postgres half is skipped.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use chat::History;
use chat::history::{Mute, NewDirect};
use diesel::connection::SimpleConnection;
use diesel::{Connection, PgConnection};
use store::{Actor, DatabaseConfig, PostgresConfig, portable};

/// A throwaway Postgres database, dropped with the guard.
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
    let name = format!("chat_test_{}_{}", std::process::id(), chat::server::random_hex(4));
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
    let history = tokio::task::spawn_blocking(move || History::open(&config))
        .await
        .unwrap()
        .unwrap();
    let (ada, bo) = (
        "account:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@local",
        "account:01J9Z3N5C8Q0V2X4Y6W8T0R2P5@local",
    );

    history.remember_account("ada", ada, "isles", "dom", 1).await.unwrap();
    history.remember_account("ada", ada, "isles", "dom", 2).await.unwrap();
    assert_eq!(history.account_of("ada").await.unwrap().as_deref(), Some(ada));
    assert_eq!(history.account_of("bo").await.unwrap(), None);

    for (n, body) in ["one", "two", "three"].into_iter().enumerate() {
        history
            .add_room_message("party:x", ada, "Ada:ada:r", body, Some("m"), n as i64)
            .await
            .unwrap();
    }
    let last: Vec<String> = history
        .room_history("party:x", 2)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect();
    assert_eq!(last, ["two", "three"]);

    let message = |body: &str, delivered_at| NewDirect {
        sender: ada.into(),
        sender_local: "ada".into(),
        recipient: bo.into(),
        body: body.into(),
        stanza_id: None,
        at: 10,
        delivered_at,
        mid: None,
    };
    history.add_direct(message("seen", Some(10))).await.unwrap();
    let waiting = history.add_direct(message("later", None)).await.unwrap();
    let pending = history.pending_for(bo, 10).await.unwrap();
    assert_eq!(pending.iter().map(|m| m.id).collect::<Vec<_>>(), [waiting]);
    history.mark_delivered(bo, vec![waiting], 20).await.unwrap();
    assert!(history.pending_for(bo, 10).await.unwrap().is_empty());
    assert_eq!(history.direct_between(bo, ada, 10).await.unwrap().len(), 2);

    let admin = Actor::Admin("mod".into());
    let mute = Mute {
        account: ada.into(),
        until: Some(100),
        reason: None,
        by: admin.as_text(),
        at: 5,
    };
    history.set_mute(mute.clone(), admin.clone()).await.unwrap();
    let stored = history.mute_of(ada).await.unwrap().unwrap();
    assert_eq!(stored, mute);
    assert!(stored.holds(99) && !stored.holds(100));
    assert!(history.clear_mute(ada, admin.clone()).await.unwrap());
    assert!(!history.clear_mute(ada, admin).await.unwrap());

    // Every write is in the change log, under who made it (DD23)
    let changes = history.database().changes(0, 1000).await.unwrap();
    assert!(
        changes
            .iter()
            .any(|c| c.table_name == "chat_mutes" && c.actor == "admin:mod")
    );
    assert!(
        changes
            .iter()
            .any(|c| c.table_name == "chat_room_messages" && c.actor == format!("player:{ada}"))
    );
}

#[tokio::test]
async fn history_works_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let game = DatabaseConfig::Sqlite {
        file: dir.path().join("game.db"),
    };
    let config = History::beside(&game);
    assert_eq!(
        config,
        DatabaseConfig::Sqlite {
            file: dir.path().join("chat.db")
        }
    );
    exercise(config).await;
}

#[tokio::test]
async fn history_works_on_postgres() {
    let Some(pg) = new_postgres() else {
        eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the Postgres half is skipped");
        return;
    };
    exercise(DatabaseConfig::Postgres(pg.config.clone())).await;
}
