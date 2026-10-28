//! What progression takes from other areas, as traits with stand-ins, so this file's area builds and tests alone.
//! The integration pass passes the real ones to [`super::register_with`]; each is one Node call named below.

use std::sync::Arc;

use module_api::BoxFuture;

use crate::AccountId;
use crate::http::Shared;

use super::journey::{JourneyInventory, RepoJourneyInventory};

/// The activity monitor's recorders (`activity/activityMonitor.ts`: `RecordXpActivity`, `RecordReforgeActivity`,
/// `RecordSlayersPathUnlocksActivity`), reforged-api's. Monitoring only: they never fail a request.
pub trait Activity: Send + Sync {
    fn xp(&self, _shared: &Shared, _user_id: &str, _progression_id: &str, _amount: i64) {}
    fn reforge(&self, _shared: &Shared, _user_id: &str) {}
    /// `source` is `host` or `gameserver`
    fn slayers_path_unlocks(&self, _shared: &Shared, _user_id: &str, _node_ids: &[String], _source: &str) {}
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoActivity;

impl Activity for NoActivity {}

#[derive(Debug, Clone, Copy, Default)]
pub struct ActivityMonitor;

impl Activity for ActivityMonitor {
    fn xp(&self, shared: &Shared, user_id: &str, progression_id: &str, amount: i64) {
        crate::areas::reforged_api::activity::record_xp_grant(shared, user_id, progression_id, amount);
    }

    fn reforge(&self, shared: &Shared, user_id: &str) {
        crate::areas::reforged_api::activity::record_reforge(shared, user_id);
    }

    fn slayers_path_unlocks(&self, shared: &Shared, user_id: &str, node_ids: &[String], source: &str) {
        crate::areas::reforged_api::activity::record_slayers_path_unlocks(shared, user_id, node_ids, source);
    }
}

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

#[derive(Debug, Clone, Copy, Default)]
pub struct CharactersStarterBootstrap;

impl StarterBootstrap for CharactersStarterBootstrap {
    fn ensure<'a>(&'a self, shared: &'a Shared, account: &'a AccountId, character_id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(store) = &shared.store else { return };
            let actor = module_api::ActorContext::new(module_api::Actor::Player {
                account: account.global().clone(),
            });
            let owner = account.clone();
            let cid = character_id.to_owned();
            let tables = shared.version.tables;
            let _ = store
                .account_write(account, &actor, move |tx, repos| {
                    let user = repos
                        .accounts
                        .find(tx.conn(), &owner)?
                        .map(|a| a.user_id.0)
                        .unwrap_or_else(|| owner.to_string());
                    let _ = crate::areas::characters::starter::ensure_in_transaction(
                        tx, repos, &tables, &owner, &user, &cid,
                    );
                    Ok(())
                })
                .await;
        })
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
            inventory: Arc::new(RepoJourneyInventory),
            activity: Arc::new(ActivityMonitor),
            starter: Arc::new(CharactersStarterBootstrap),
        }
    }
}
