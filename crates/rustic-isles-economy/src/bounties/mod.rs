// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Bounties and the stores (bounties spec): bounty and cooldown state, entitlements, the wallet and Platinum, the
//! store sections and purchases, and the economy ledger monitor.

pub mod challenge_rewards;
pub mod economy_ledger;
pub mod platinum;
pub mod state;
pub mod wallet;

use rustic_isles::Handlers;

/// Mounts the 17 handlers.
pub fn register(handlers: &mut Handlers) {
    let _ = handlers;
}
