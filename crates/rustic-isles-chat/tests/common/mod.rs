// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Accounts on a temporary store, and a home that says some accounts live elsewhere.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    dead_code,
    reason = "shared by several test binaries, each using only some; a panic is how a test fails"
)]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use module_api::network::HomeError;
use module_api::{Actor, ActorContext, BoxFuture, GlobalId, HomeApi, NodeId, NodeIdentity, Region};
use rustic_isles_store::repo::auth::{AccountRecord, LauncherAccountRecord, OperationalStatus};
use rustic_isles_store::{AccountId, IslesStore, UserId};
use store::{DatabaseConfig, DatabaseOptions};

pub fn system() -> ActorContext {
    ActorContext::new(Actor::System)
}

/// A game account on this node whose name is its userId.
pub async fn account(store: &IslesStore, user_id: &str) -> AccountId {
    let id = AccountId::new(store.node());
    let record = AccountRecord {
        id: id.clone(),
        user_id: UserId::new(user_id),
        name: format!("{user_id} name"),
        notes: 0,
    };
    store
        .write(&system(), move |tx, repos| repos.accounts.create(tx, &record))
        .await
        .unwrap();
    id
}

/// A launcher account (and its game account) in good standing; `change` adjusts it first.
pub async fn launcher(
    store: &IslesStore,
    user_id: &str,
    name: &str,
    change: impl FnOnce(&mut LauncherAccountRecord),
) -> AccountId {
    let id = AccountId::new(store.node());
    let mut record = LauncherAccountRecord {
        account: id.clone(),
        user_id: UserId::new(user_id),
        email: None,
        display_name_normalized: name.to_lowercase(),
        display_name: name.to_owned(),
        password_hash: None,
        status: OperationalStatus::Active,
        approval_status: None,
        approval_updated_at: None,
        approval_updated_by: None,
        approval_reason: None,
        roles: vec!["player".into()],
        roles_updated_at: None,
        roles_updated_by: None,
        created_at: "2026-10-01T00:00:00.000Z".into(),
        last_login_at: None,
        username_set: Some(true),
        consent: None,
        deletion: None,
        email_verified_at: None,
        email_verification_pending_since: None,
        timeout: None,
    };
    change(&mut record);
    store
        .write(&system(), move |tx, repos| repos.launcher_accounts.create(tx, &record))
        .await
        .unwrap();
    id
}

/// Every account is homed here except those marked away, which resolve to `elsewhere`.
pub struct SplitHome {
    pub here: NodeIdentity,
    pub away: Mutex<HashSet<GlobalId>>,
}

impl SplitHome {
    pub fn new(here: NodeIdentity) -> Arc<Self> {
        Arc::new(Self {
            here,
            away: Mutex::new(HashSet::new()),
        })
    }

    pub fn send_away(&self, account: &AccountId) {
        self.away.lock().unwrap().insert(account.global().clone());
    }
}

impl HomeApi for SplitHome {
    fn resolve_home<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<NodeIdentity, HomeError>> {
        Box::pin(async move {
            if self.away.lock().unwrap().contains(account) {
                return Ok(NodeIdentity {
                    node: NodeId::new("elsewhere").unwrap(),
                    region: Region::new("germany").unwrap(),
                });
            }
            Ok(self.here.clone())
        })
    }

    fn session_ticket<'a>(
        &'a self,
        _account: &'a GlobalId,
        _audience: &'a NodeId,
    ) -> BoxFuture<'a, Result<module_api::network::SessionTicket, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }
}

/// A store on a temporary file with a [`SplitHome`].
pub struct SplitStore {
    pub store: IslesStore,
    pub home: Arc<SplitHome>,
    _dir: tempfile::TempDir,
}

pub fn split_store() -> SplitStore {
    let dir = tempfile::tempdir().unwrap();
    let identity = NodeIdentity::standalone();
    let home = SplitHome::new(identity.clone());
    let config = DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    };
    let store = IslesStore::open(&config, &DatabaseOptions::default(), home.clone(), &identity).unwrap();
    SplitStore { store, home, _dir: dir }
}
