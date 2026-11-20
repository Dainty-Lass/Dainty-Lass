// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Game tickets: the launcher trades the account key for one of these just before it starts the game, and the game
//! logs in with it instead of the key (`APIKEY` mode's `exchange_code`). A ticket is single-use and lives minutes,
//! so what the game's command line shows is worthless once the game is in, and the key never enters the game.
//! Kept only in memory, by hash: a restart costs a ticket nobody can still be using.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use crate::util;

/// What every ticket starts with, so the login can tell one from an account key (`DLK_`).
pub const PREFIX: &str = "DLT_";
/// How long a ticket can be redeemed, in milliseconds: room for a slow first start (shader caches) before login.
pub const TTL_MS: i64 = 300_000;
/// Unredeemed tickets kept at most; minting past it drops the oldest.
pub const MAX_PENDING: usize = 4096;
/// The random part's length in bytes (48 hex digits, an account key's length).
const RANDOM_BYTES: usize = 24;

/// Whom a ticket logs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketHolder {
    /// The game account's userId.
    pub user_id: String,
    /// The game account's id.
    pub account_id: String,
}

#[derive(Debug)]
struct Pending {
    holder: TicketHolder,
    expires_at: i64,
}

/// The tickets minted and not yet redeemed or expired.
#[derive(Debug, Default)]
pub struct GameTickets {
    pending: Mutex<HashMap<String, Pending>>,
}

/// Whether `text` is shaped like a ticket: the prefix and 48 lowercase hex digits.
#[must_use]
pub fn is_ticket(text: &str) -> bool {
    text.strip_prefix(PREFIX).is_some_and(|rest| {
        rest.len() == RANDOM_BYTES * 2 && rest.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

impl GameTickets {
    /// A fresh ticket for `holder`, redeemable once until `now + TTL_MS`.
    pub fn mint(&self, holder: TicketHolder, now: i64) -> String {
        let ticket = format!("{PREFIX}{}", util::random_hex::<RANDOM_BYTES>());
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending.retain(|_, entry| entry.expires_at > now);
        if pending.len() >= MAX_PENDING
            && let Some(oldest) = pending
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(hash, _)| hash.clone())
        {
            pending.remove(&oldest);
        }
        pending.insert(
            util::sha256_hex(&ticket),
            Pending {
                holder,
                expires_at: now + TTL_MS,
            },
        );
        ticket
    }

    /// The ticket's holder, once: a second redemption, or one after it expired, is `None`.
    pub fn redeem(&self, ticket: &str, now: i64) -> Option<TicketHolder> {
        if !is_ticket(ticket) {
            return None;
        }
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = pending.remove(&util::sha256_hex(ticket))?;
        (entry.expires_at > now).then_some(entry.holder)
    }

    /// Drops every unredeemed ticket of `account_id` (a ban or a revoked key).
    pub fn revoke_account(&self, account_id: &str) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending.retain(|_, entry| entry.holder.account_id != account_id);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    fn holder(n: u8) -> TicketHolder {
        TicketHolder {
            user_id: format!("user-{n}"),
            account_id: format!("account-{n}"),
        }
    }

    #[test]
    fn a_ticket_logs_in_once() {
        let tickets = GameTickets::default();
        let ticket = tickets.mint(holder(1), 1_000);
        assert!(is_ticket(&ticket));
        assert_eq!(tickets.redeem(&ticket, 1_001), Some(holder(1)));
        assert_eq!(tickets.redeem(&ticket, 1_002), None);
    }

    #[test]
    fn a_ticket_expires() {
        let tickets = GameTickets::default();
        let ticket = tickets.mint(holder(1), 1_000);
        assert_eq!(tickets.redeem(&ticket, 1_000 + TTL_MS), None);
        let ticket = tickets.mint(holder(1), 1_000);
        assert_eq!(tickets.redeem(&ticket, 1_000 + TTL_MS - 1), Some(holder(1)));
    }

    #[test]
    fn only_ticket_shapes_are_looked_up() {
        let tickets = GameTickets::default();
        for text in [
            "",
            "DLT_",
            "DLK_0123",
            &format!("DLT_{}", "A".repeat(48)),
            &format!("DLT_{}", "a".repeat(49)),
        ] {
            assert!(!is_ticket(text), "{text}");
            assert_eq!(tickets.redeem(text, 0), None);
        }
    }

    #[test]
    fn revoking_drops_only_that_accounts_tickets() {
        let tickets = GameTickets::default();
        let mine = tickets.mint(holder(1), 0);
        let theirs = tickets.mint(holder(2), 0);
        tickets.revoke_account("account-1");
        assert_eq!(tickets.redeem(&mine, 1), None);
        assert_eq!(tickets.redeem(&theirs, 1), Some(holder(2)));
    }

    #[test]
    fn the_pending_set_is_bounded() {
        let tickets = GameTickets::default();
        let first = tickets.mint(holder(0), 0);
        for n in 1..=MAX_PENDING {
            tickets.mint(holder(1), i64::try_from(n).unwrap());
        }
        assert_eq!(tickets.redeem(&first, 1), None);
        assert!(tickets.pending.lock().unwrap().len() <= MAX_PENDING);
    }
}
