// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A store on a temporary SQLite file, for tests here and in the area crates (spec §13.2).

use std::sync::Arc;

use module_api::NodeIdentity;
use module_api::network::StandaloneHome;
use store::{DatabaseConfig, DatabaseOptions};

use crate::{IslesStore, RepoResult};

/// The store and the folder its file lives in (removed when dropped).
pub struct TempStore {
    /// The opened store.
    pub store: IslesStore,
    /// The temporary folder holding the database file.
    pub dir: tempfile::TempDir,
}

impl TempStore {
    /// A standalone node (`local`) whose home owns every account minted on it.
    pub fn open() -> RepoResult<Self> {
        Self::open_as(NodeIdentity::standalone())
    }

    /// A store in a fresh temporary folder, as node `identity`.
    pub fn open_as(identity: NodeIdentity) -> RepoResult<Self> {
        let dir = tempfile::tempdir()
            .map_err(|e| crate::RepoError::Store(store::Error::Config(format!("temporary folder: {e}"))))?;
        let config = DatabaseConfig::Sqlite {
            file: dir.path().join("isles.db"),
        };
        let home = Arc::new(StandaloneHome {
            identity: identity.clone(),
        });
        let store = IslesStore::open(&config, &DatabaseOptions::default(), home, &identity)?;
        Ok(Self { store, dir })
    }
}
