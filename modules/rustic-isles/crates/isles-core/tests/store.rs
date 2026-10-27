//! The isles database on `store` (DD15) with its change log (DD23): an account written through the home check (DN2)
//! under a global id (DD18), logged with its actor, and refused on a node that isn't its home. SQLite here; the
//! Postgres half runs the same migrations once DF7's CI matrix covers this repository.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::Arc;

use isles_core::db::IslesStore;
use isles_core::repo::auth::AccountRecord;
use isles_core::repo::characters::CharacterRepository;
use isles_core::repo::{RepoError, Stub};
use isles_core::{AccountId, UserId};
use module_api::network::StandaloneHome;
use module_api::{Actor, ActorContext, NodeId, NodeIdentity};
use store::{DatabaseConfig, DatabaseOptions};

fn open(dir: &tempfile::TempDir) -> IslesStore {
    let config = DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    };
    let node = NodeIdentity::standalone();
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    IslesStore::open(&config, &DatabaseOptions::default(), home, &node).unwrap()
}

fn admin() -> ActorContext {
    ActorContext::new(Actor::Admin { key: "owner".into() })
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_is_written_at_its_home_and_logged() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let account = AccountRecord {
        id: AccountId::new(&NodeIdentity::standalone().node),
        user_id: UserId::random(),
        name: "Fox".into(),
        notes: 0,
    };
    let record = account.clone();
    store
        .account_write(&account.id, &admin(), move |tx, repos| {
            repos.accounts.create(tx, &record)
        })
        .await
        .unwrap();
    let user = account.user_id.clone();
    let found = store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
        .await
        .unwrap();
    assert_eq!(found, Some(account.clone()));

    let log = store.database().changes(0, 100).await.unwrap();
    let entry = log.iter().find(|e| e.table_name == "accounts").unwrap();
    assert_eq!((entry.op.as_str(), entry.actor.as_str()), ("insert", "admin:owner"));
    assert!(entry.row_key.contains(&account.id.to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn another_nodes_account_is_not_written_here() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let theirs = AccountId::new(&"germany-1".parse::<NodeId>().unwrap());
    let result = store.account_write(&theirs, &admin(), |_, _| Ok(())).await;
    assert!(matches!(result, Err(RepoError::Home(_))), "{result:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn unported_repositories_answer_unimplemented() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let id = AccountId::new(&NodeIdentity::standalone().node);
    let result = store
        .read(move |conn, _| CharacterRepository::find_many_by_account(&Stub, conn, &id))
        .await;
    assert!(matches!(
        result,
        Err(RepoError::Unimplemented {
            repository: "characters",
            operation: "find_many_by_account"
        })
    ));
}
