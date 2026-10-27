//! `FriendshipRepository` on both engines (DD15): `repositories.social.test.ts`'s friendship test, the edge order
//! Node listed them in, and the account deletion sweep. The Postgres half runs when `REFORGED_TEST_POSTGRES_URL`
//! names a server (`postgres://user:pw@127.0.0.1:5432/postgres`); it skips otherwise.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::Arc;

use diesel::Connection as _;
use diesel::PgConnection;
use diesel::connection::SimpleConnection;
use isles_core::db::IslesStore;
use isles_core::repo::auth::AccountRecord;
use isles_core::repo::chat::{FriendEdgeRecord, FriendshipDirection, FriendshipStatus};
use isles_core::{AccountId, UserId};
use module_api::network::StandaloneHome;
use module_api::{Actor, ActorContext, NodeIdentity};
use store::{DatabaseConfig, DatabaseOptions, PostgresConfig};

const T0: &str = "2026-10-01T00:00:00.000Z";
const T1: &str = "2026-10-02T00:00:00.000Z";

fn actor() -> ActorContext {
    ActorContext::new(Actor::Admin { key: "owner".into() })
}

fn open(config: &DatabaseConfig) -> IslesStore {
    let node = NodeIdentity::standalone();
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    IslesStore::open(config, &DatabaseOptions::default(), home, &node).unwrap()
}

async fn account(store: &IslesStore, name: &str) -> AccountRecord {
    let record = AccountRecord {
        id: AccountId::new(&NodeIdentity::standalone().node),
        user_id: UserId::random(),
        name: name.into(),
        notes: 0,
    };
    let created = record.clone();
    store
        .account_write(&record.id, &actor(), move |tx, repos| {
            repos.accounts.create(tx, &created)
        })
        .await
        .unwrap();
    record
}

fn edge(
    owner: &AccountRecord,
    other: &AccountRecord,
    status: FriendshipStatus,
    direction: Option<FriendshipDirection>,
    created: &str,
) -> FriendEdgeRecord {
    FriendEdgeRecord {
        owner: owner.id.clone(),
        other: other.id.clone(),
        other_user_id: other.user_id.clone(),
        status,
        direction,
        created: created.into(),
        favorite: false,
    }
}

async fn upsert(store: &IslesStore, edge: FriendEdgeRecord) {
    store
        .write(&actor(), move |tx, repos| repos.friendships.upsert(tx, &edge))
        .await
        .unwrap();
}

async fn list(store: &IslesStore, owner: &AccountRecord) -> Vec<FriendEdgeRecord> {
    let id = owner.id.clone();
    store
        .read(move |conn, repos| repos.friendships.list_for_owner(conn, &id))
        .await
        .unwrap()
}

async fn find(store: &IslesStore, owner: &AccountRecord, other: &AccountRecord) -> Option<FriendEdgeRecord> {
    let (a, b) = (owner.id.clone(), other.id.clone());
    store
        .read(move |conn, repos| repos.friendships.find(conn, &a, &b))
        .await
        .unwrap()
}

/// `friendships: one directed edge per pair and direction, upsert replaces, remove is one-sided`
async fn the_friendship_graph(store: IslesStore) {
    use FriendshipDirection::{Inbound, Outbound};
    use FriendshipStatus::{Accepted, Blocked, Pending};
    let (a, b, c) = (
        account(&store, "a").await,
        account(&store, "b").await,
        account(&store, "c").await,
    );
    assert!(list(&store, &a).await.is_empty());
    assert_eq!(find(&store, &a, &b).await, None);

    upsert(&store, edge(&a, &b, Pending, Some(Outbound), T0)).await;
    upsert(&store, edge(&b, &a, Pending, Some(Inbound), T0)).await;
    upsert(&store, edge(&a, &c, Blocked, None, T1)).await;
    let found = find(&store, &a, &b).await.unwrap();
    assert_eq!(
        (found.status, found.direction, found.created.as_str(), found.favorite),
        (Pending, Some(Outbound), T0, false)
    );
    assert_eq!(find(&store, &a, &c).await.unwrap().direction, None);

    // The edge keeps its place when it is replaced, as Node's did
    let mut accepted = edge(&a, &b, Accepted, None, T0);
    accepted.favorite = true;
    upsert(&store, accepted).await;
    upsert(&store, edge(&b, &a, Accepted, None, T0)).await;
    let found = find(&store, &a, &b).await.unwrap();
    assert_eq!((found.status, found.direction, found.favorite), (Accepted, None, true));
    let others: Vec<UserId> = list(&store, &a).await.into_iter().map(|e| e.other_user_id).collect();
    assert_eq!(others, vec![b.user_id.clone(), c.user_id.clone()]);
    assert_eq!(list(&store, &b).await.len(), 1);

    let (owner, other) = (a.id.clone(), b.id.clone());
    store
        .write(&actor(), move |tx, repos| repos.friendships.remove(tx, &owner, &other))
        .await
        .unwrap();
    assert_eq!(find(&store, &a, &b).await, None);
    assert!(find(&store, &b, &a).await.is_some(), "the other direction stays");
    let (owner, other) = (a.id.clone(), c.id.clone());
    store
        .write(&actor(), move |tx, repos| repos.friendships.remove(tx, &owner, &other))
        .await
        .unwrap();

    // An account's deletion drops its edges in either direction
    upsert(&store, edge(&a, &c, Pending, Some(Outbound), T0)).await;
    upsert(&store, edge(&c, &a, Pending, Some(Inbound), T0)).await;
    let id = a.id.clone();
    let removed = store
        .write(&actor(), move |tx, repos| {
            repos.friendships.remove_all_for_account(tx, &id)
        })
        .await
        .unwrap();
    assert_eq!(removed, 3, "a->c, c->a and b->a");
    assert!(list(&store, &b).await.is_empty() && list(&store, &c).await.is_empty());

    let log = store.database().changes(0, 1000).await.unwrap();
    assert!(
        log.iter()
            .any(|entry| entry.table_name == "friend_edges" && entry.actor == "admin:owner"),
        "the change log has the edges"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn friendships_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    });
    the_friendship_graph(store).await;
}

struct PgDatabase {
    admin: PostgresConfig,
    config: PostgresConfig,
}

impl Drop for PgDatabase {
    fn drop(&mut self) {
        if let Ok(mut conn) = PgConnection::establish(&self.admin.conninfo()) {
            let _ = conn.batch_execute(&format!(
                "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
                self.config.database
            ));
        }
    }
}

fn throwaway_postgres() -> Option<PgDatabase> {
    let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
    let rest = url.strip_prefix("postgres://")?;
    let (login, location) = rest.split_once('@')?;
    let (user, password) = login.split_once(':').unwrap_or((login, ""));
    let (hostport, database) = location.split_once('/')?;
    let (host, port) = hostport.split_once(':').unwrap_or((hostport, "5432"));
    let admin = PostgresConfig {
        host: host.into(),
        port: port.parse().ok()?,
        user: user.into(),
        database: database.into(),
        password: password.into(),
        allow_remote: false,
    };
    let name = format!("isles_friends_{}", std::process::id());
    PgConnection::establish(&admin.conninfo())
        .expect("REFORGED_TEST_POSTGRES_URL is reachable")
        .batch_execute(&format!("CREATE DATABASE \"{name}\""))
        .unwrap();
    let config = PostgresConfig {
        database: name,
        ..admin.clone()
    };
    Some(PgDatabase { admin, config })
}

#[tokio::test(flavor = "multi_thread")]
async fn friendships_on_postgres() {
    let Some(pg) = throwaway_postgres() else {
        eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the Postgres half is skipped");
        return;
    };
    let store = open(&DatabaseConfig::Postgres(pg.config.clone()));
    the_friendship_graph(store).await;
}
