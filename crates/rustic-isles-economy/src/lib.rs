// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Isles' economy (RD27, written clean-room from `docs/port/spec-rustic-isles-economy-inventory.md`,
//! `-progression.md` and `-bounties.md`): one crate because the three parts call each other in a cycle.
//!
//! - [`inventory`]: `POST /inventory` and its ledger, the item rules, the storage policy, the currency half of a
//!   transaction, the weapon reforge reset and banked restore;
//! - [`progression`]: XP tracks and their curves, the grant ledger, mastery, escalation, the Slayer's Path, encountered
//!   content, breadcrumbs and the session supplies refill;
//! - [`bounties`]: bounty and cooldown state, entitlements, the wallet and Platinum, the stores and the economy ledger;
//! - [`multipliers`]: the private server's test multipliers, shared by inventory and progression.
//!
//! Game-derived values (curves, store files, the storage policy) are the host's own game data, read at run time.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// A refusal is the answer itself (an axum `Response`), as in core and the other areas
#![allow(clippy::result_large_err)]

pub mod bounties;
mod caches;
mod caller;
pub mod inventory;
pub mod multipliers;
pub mod progression;
#[cfg(feature = "test-support")]
pub mod test_support;
mod tx;
mod util;

pub use bounties::{challenge_rewards, economy_ledger, entitlements, platinum, wallet};
pub use caches::EconomyCaches;
pub use util::{js_number_text, log_token, number_value};

use rustic_isles::Handlers;

/// Mounts every handler of the three parts with their default ports (the composer may call each part's own
/// `register` instead).
pub fn register(handlers: &mut Handlers) {
    inventory::register(handlers, inventory::InventoryPorts::default());
    progression::register(handlers, progression::ProgressionPorts::default());
    bounties::register(handlers);
}
