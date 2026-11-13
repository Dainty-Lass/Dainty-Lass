//! History while the server runs (M-H, manager-server review 5.2): the services keep their databases open and
//! write, and history opens, queries and reopens beside them.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use dainty_admin::history::HistorySet;
use dainty_admin::keys::{Keys, Role};
use dainty_store::{Actor, Database, DatabaseConfig, DatabaseOptions, Filter};
use diesel::connection::SimpleConnection;
use diesel::{Connection, SqliteConnection};

/// Between a service's writes: busy, but not one write lock held without a gap.
const PACE: std::time::Duration = std::time::Duration::from_millis(2);

/// The game's and the admin API's databases, open and written to as a running server does.
struct Server {
    config: DatabaseConfig,
    stop: Arc<AtomicBool>,
    writers: Vec<tokio::task::JoinHandle<()>>,
}

async fn running(dir: &std::path::Path) -> Server {
    let file = dir.join("dainty-lass.db");
    SqliteConnection::establish(&file.to_string_lossy())
        .unwrap()
        .batch_execute("CREATE TABLE accounts (id TEXT PRIMARY KEY, name TEXT NOT NULL, deleted_at BIGINT)")
        .unwrap();
    let config = DatabaseConfig::Sqlite { file };
    let game = Database::attach(&config, &DatabaseOptions::default(), &[]).unwrap();
    let keys = Keys::open(&Keys::database_for(&config), "owner-key-0123456789abcdef0123456789").unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let mut writers = Vec::new();
    {
        let stop = stop.clone();
        writers.push(tokio::spawn(async move {
            let mut n = 0u64;
            while !stop.load(Ordering::Relaxed) {
                n += 1;
                game.write(Actor::System("isles".into()), move |tx| {
                    tx.conn()
                        .batch_execute(&format!("INSERT INTO accounts (id, name) VALUES ('a{n}', 'P{n}')"))?;
                    Ok::<_, dainty_store::Error>(())
                })
                .await
                .unwrap();
                tokio::time::sleep(PACE).await;
            }
        }));
    }
    {
        let stop = stop.clone();
        writers.push(tokio::spawn(async move {
            let mut n = 0u64;
            while !stop.load(Ordering::Relaxed) {
                n += 1;
                keys.issue(&format!("Person {n}"), Role::Viewer, Actor::Host("test".into()), 1_000)
                    .await
                    .unwrap();
                tokio::time::sleep(PACE).await;
            }
        }));
    }
    // Some of each in the logs before the first query
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    Server { config, stop, writers }
}

impl Server {
    async fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        for writer in self.writers {
            writer.await.unwrap();
        }
    }
}

async fn query_once(config: &DatabaseConfig) {
    let set = HistorySet::open(config).await.unwrap();
    let page = set.query(&Filter::default(), None, 50).await.unwrap();
    assert!(!page.entries.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queries_in_a_row_while_the_server_writes() {
    let dir = tempfile::tempdir().unwrap();
    let server = running(dir.path()).await;
    for _ in 0..20 {
        query_once(&server.config).await;
    }
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_queries_while_the_server_writes() {
    let dir = tempfile::tempdir().unwrap();
    let server = running(dir.path()).await;
    for _ in 0..5 {
        let queries: Vec<_> = (0..4)
            .map(|_| {
                let config = server.config.clone();
                tokio::spawn(async move { query_once(&config).await })
            })
            .collect();
        for query in queries {
            query.await.unwrap();
        }
    }
    server.stop().await;
}
