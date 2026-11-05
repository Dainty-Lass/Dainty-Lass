// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Progression (progression spec): XP tracks and their curves, the grant ledger, the rank confirm, the objective
//! report and mastery, escalation, the Slayer's Path, encountered content, breadcrumbs and the session supplies.

pub mod escalation;
pub mod journey;
pub mod mastery;
pub mod reforge;
mod routes;
pub mod supplies;
pub mod tracks;

use std::sync::Arc;

use module_api::{Actor, ActorContext, BoxFuture};
use rustic_isles::Shared;
use rustic_isles_store::{AccountId, IslesStore};

use crate::inventory::{InventoryPorts, NoRing, RingInstances};

pub use routes::{SessionSupplies, register, routes};
pub use tracks::{curves, resolve_track_id};

/// The Reforged API's activity counters (monitoring only, never failing a request).
pub trait Activity: Send + Sync {
    fn xp(&self, _user_id: &str, _track: &str, _amount: f64) {}
    fn slayers_path_unlocks(&self, _user_id: &str, _node_keys: &[String], _source: &str) {}
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoActivity;

impl Activity for NoActivity {}

/// The characters area's two checks this part needs (spec §9.4).
pub trait CharacterChecks: Send + Sync {
    /// Whether `character` is a live character of `account`; a lookup failure reads false.
    fn owns<'a>(&'a self, store: &'a IslesStore, account: &'a AccountId, character: &'a str) -> BoxFuture<'a, bool>;
    /// The starter records ensured in one account write; never fails.
    fn ensure_starter<'a>(
        &'a self,
        shared: &'a Arc<Shared>,
        store: &'a IslesStore,
        ports: &'a InventoryPorts,
        account: &'a AccountId,
        user_id: &'a str,
        character: &'a str,
    ) -> BoxFuture<'a, ()>;
}

/// The store's characters and inventory's starter bootstrap.
#[derive(Debug, Default, Clone, Copy)]
pub struct StoreCharacters;

impl CharacterChecks for StoreCharacters {
    fn owns<'a>(&'a self, store: &'a IslesStore, account: &'a AccountId, character: &'a str) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            let (id, c) = (account.clone(), character.to_owned());
            match store
                .read(move |conn, repos| repos.characters.find(conn, &c, &id))
                .await
            {
                Ok(found) => found.is_some(),
                Err(e) => {
                    tracing::error!("progression: character lookup failed: {e}");
                    false
                }
            }
        })
    }

    fn ensure_starter<'a>(
        &'a self,
        shared: &'a Arc<Shared>,
        store: &'a IslesStore,
        ports: &'a InventoryPorts,
        account: &'a AccountId,
        user_id: &'a str,
        character: &'a str,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let actor = ActorContext::new(Actor::Player {
                account: account.global().clone(),
            });
            let (shared, starter, id) = (Arc::clone(shared), Arc::clone(&ports.starter), account.clone());
            let (user, c) = (user_id.to_owned(), character.to_owned());
            let _ = store
                .account_write(account, &actor, move |tx, repos| {
                    starter.ensure(tx, repos, &shared, &id, &user, &c).map(|_| ())
                })
                .await;
        })
    }
}

#[derive(Clone)]
pub struct ProgressionPorts {
    pub characters: Arc<dyn CharacterChecks>,
    pub activity: Arc<dyn Activity>,
    pub ring: Arc<dyn RingInstances>,
    pub inventory: InventoryPorts,
}

impl Default for ProgressionPorts {
    fn default() -> Self {
        Self {
            characters: Arc::new(StoreCharacters),
            activity: Arc::new(NoActivity),
            ring: Arc::new(NoRing),
            inventory: InventoryPorts::default(),
        }
    }
}
