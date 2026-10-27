//! What progression takes from other areas, as traits with stand-ins, so this file's area builds and tests alone.
//! The integration pass passes the real ones to [`super::register_with`]; each is one Node call named below.

use std::sync::Arc;

use module_api::BoxFuture;

use crate::AccountId;
use crate::http::Shared;

use super::journey::{JourneyInventory, UnwiredInventory};

/// The activity monitor's recorders (`activity/activityMonitor.ts`: `RecordXpActivity`, `RecordReforgeActivity`,
/// `RecordSlayersPathUnlocksActivity`), reforged-api's. Monitoring only: they never fail a request.
pub trait Activity: Send + Sync {
    fn xp(&self, _user_id: &str, _progression_id: &str, _amount: i64) {}
    fn reforge(&self, _user_id: &str) {}
    /// `source` is `host` or `gameserver`
    fn slayers_path_unlocks(&self, _user_id: &str, _node_ids: &[String], _source: &str) {}
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoActivity;

impl Activity for NoActivity {}

/// `EnsureStarterBootstrapRecords` (characters-loadouts' `starterManifest.ts`), run before a session refill.
pub trait StarterBootstrap: Send + Sync {
    fn ensure<'a>(&'a self, shared: &'a Shared, account: &'a AccountId, character_id: &'a str) -> BoxFuture<'a, ()>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoStarterBootstrap;

impl StarterBootstrap for NoStarterBootstrap {
    fn ensure<'a>(&'a self, _: &'a Shared, _: &'a AccountId, _: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// `IsCharacterOwnedByUser`'s lookup: whether a character row belongs to the account (characters-loadouts' table).
pub trait Characters: Send + Sync {
    fn owns<'a>(&'a self, shared: &'a Shared, account: &'a AccountId, character_id: &'a str) -> BoxFuture<'a, bool>;
}

/// The characters repository as the characters-loadouts area lands it; a lookup that fails reads as not owned.
#[derive(Debug, Clone, Copy, Default)]
pub struct RepoCharacters;

impl Characters for RepoCharacters {
    fn owns<'a>(&'a self, shared: &'a Shared, account: &'a AccountId, character_id: &'a str) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            let Some(store) = &shared.store else { return false };
            let (account, character) = (account.clone(), character_id.to_owned());
            match store
                .read(move |conn, repos| repos.characters.find(conn, &character, &account))
                .await
            {
                Ok(found) => found.is_some(),
                Err(error) => {
                    tracing::error!("progression: character lookup failed: {error}");
                    false
                }
            }
        })
    }
}

#[derive(Clone)]
pub struct Ports {
    pub characters: Arc<dyn Characters>,
    pub inventory: Arc<dyn JourneyInventory>,
    pub activity: Arc<dyn Activity>,
    pub starter: Arc<dyn StarterBootstrap>,
}

impl Default for Ports {
    fn default() -> Self {
        Self {
            characters: Arc::new(RepoCharacters),
            inventory: Arc::new(UnwiredInventory),
            activity: Arc::new(NoActivity),
            starter: Arc::new(NoStarterBootstrap),
        }
    }
}
