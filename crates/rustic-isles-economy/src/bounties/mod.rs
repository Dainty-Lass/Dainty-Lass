// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Bounties and the stores (bounties spec): bounty and cooldown state, entitlements, the wallet and Platinum, the
//! store sections and purchases, and the economy ledger monitor.

pub mod challenge_rewards;
pub mod cooldowns;
pub mod economy_ledger;
pub mod entitlements;
pub mod platinum;
mod routes;
pub mod state;
pub mod store;
pub mod wallet;

use std::sync::Arc;

use module_api::{BoxFuture, GlobalId};
use rustic_isles::Shared;
use rustic_isles_store::{AccountId, IslesStore};
use serde_json::Value;

pub use routes::{register, routes};
pub use store::{PurchasePlan, SkuGrants};

/// Matchmaking's view of accounts homed on other nodes (it fills the slot).
pub trait ForeignAccounts: Send + Sync {
    /// The account `user_id` names when its owner node is not this one.
    fn foreign_account<'a>(&'a self, shared: &'a Shared, user_id: &'a str) -> BoxFuture<'a, Option<GlobalId>>;
}

/// The Reforged API's claim counter (fire and forget).
pub trait ClaimActivity: Send + Sync {
    fn challenges_claimed(&self, user_id: &str, count: usize);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoForeignAccounts;

impl ForeignAccounts for NoForeignAccounts {
    fn foreign_account<'a>(&'a self, _: &'a Shared, _: &'a str) -> BoxFuture<'a, Option<GlobalId>> {
        Box::pin(async { None })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoClaimActivity;

impl ClaimActivity for NoClaimActivity {
    fn challenges_claimed(&self, _: &str, _: usize) {}
}

/// This part's slot; the composer fills it from matchmaking and the Reforged API.
#[derive(Clone)]
pub struct BountiesSeams {
    pub foreign: Arc<dyn ForeignAccounts>,
    pub activity: Arc<dyn ClaimActivity>,
}

impl Default for BountiesSeams {
    fn default() -> Self {
        Self {
            foreign: Arc::new(NoForeignAccounts),
            activity: Arc::new(NoClaimActivity),
        }
    }
}

/// A daily claim's ledger row: the character it was made under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyClaim {
    pub character_id: Option<String>,
}

/// `FindClaimInInventoryLedger`: this window's claim among the account's characters, else among its 500 most
/// recent ledger rows. A store failure is an error text (the callers fail closed).
pub async fn find_claim(
    store: &IslesStore,
    account: &AccountId,
    transaction_id: &str,
) -> Result<Option<DailyClaim>, String> {
    let (id, tx) = (account.clone(), transaction_id.to_owned());
    store
        .read(move |conn, repos| {
            for character in repos.characters.find_many_by_account(conn, &id)? {
                let key = rustic_isles_store::repo::inventory::LedgerKey {
                    account: &id,
                    character_id: &character.character_id,
                    transaction_id: &tx,
                };
                if repos.inventory_transactions.find_record(conn, key)?.is_some() {
                    return Ok(Some(DailyClaim {
                        character_id: Some(character.character_id),
                    }));
                }
            }
            let recent = repos.inventory_transactions.list_recent_for_account(conn, &id, 500)?;
            Ok(recent
                .iter()
                .find(|row| row.get("transactionId").and_then(Value::as_str) == Some(tx.as_str()))
                .map(|row| DailyClaim {
                    character_id: row.get("characterId").and_then(Value::as_str).map(str::to_owned),
                }))
        })
        .await
        .map_err(|e| e.to_string())
}
