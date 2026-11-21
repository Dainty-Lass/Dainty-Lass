// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Download slots: so many streams per account and so many in all, held until the reply body ends.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// Why a download slot was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The account's own streams are full (429)
    Account,
    /// The server's are (503)
    Total,
}

impl Refusal {
    /// The log and reply reason code.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Account => "account_limit",
            Self::Total => "total_limit",
        }
    }
}

#[derive(Default)]
struct Counts {
    total: usize,
    accounts: HashMap<String, usize>,
}

/// Counts of open download streams, per account and in all; cheap to clone.
#[derive(Clone)]
pub struct Slots {
    per_account: usize,
    total: usize,
    counts: Arc<Mutex<Counts>>,
}

/// One stream; dropping it gives the slot back.
pub struct Slot {
    account: String,
    counts: Arc<Mutex<Counts>>,
}

impl Slots {
    /// Allows `per_account` streams per account and `total` in all.
    pub fn new(per_account: usize, total: usize) -> Self {
        Self {
            per_account,
            total,
            counts: Arc::default(),
        }
    }

    /// Takes a slot for `account`, or says which limit is full.
    pub fn acquire(&self, account: &str) -> Result<Slot, Refusal> {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        if counts.accounts.get(account).copied().unwrap_or(0) >= self.per_account {
            return Err(Refusal::Account);
        }
        if counts.total >= self.total {
            return Err(Refusal::Total);
        }
        counts.total += 1;
        *counts.accounts.entry(account.to_owned()).or_default() += 1;
        Ok(Slot {
            account: account.to_owned(),
            counts: self.counts.clone(),
        })
    }

    /// Streams open now: in all, and for `account`.
    pub fn open(&self, account: &str) -> (usize, usize) {
        let counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        (counts.total, counts.accounts.get(account).copied().unwrap_or(0))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        counts.total = counts.total.saturating_sub(1);
        if let Some(n) = counts.accounts.get_mut(&self.account) {
            *n -= 1;
            if *n == 0 {
                counts.accounts.remove(&self.account);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]
    use super::*;

    #[test]
    fn per_account_limit() {
        let slots = Slots::new(2, 10);
        let a1 = slots.acquire("a").unwrap();
        let _a2 = slots.acquire("a").unwrap();
        assert_eq!(slots.acquire("a").err(), Some(Refusal::Account));
        let _b = slots.acquire("b").unwrap();
        drop(a1);
        let _a3 = slots.acquire("a").unwrap();
        assert_eq!(slots.open("a"), (3, 2));
    }

    #[test]
    fn server_wide_limit_checked_after_the_account() {
        let slots = Slots::new(1, 2);
        let a = slots.acquire("a").unwrap();
        let _b = slots.acquire("b").unwrap();
        assert_eq!(slots.acquire("c").err(), Some(Refusal::Total));
        assert_eq!(slots.acquire("a").err(), Some(Refusal::Account));
        drop(a);
        assert_eq!(slots.open("a"), (1, 0));
        assert!(slots.acquire("c").is_ok());
        assert_eq!(Refusal::Total.reason(), "total_limit");
    }
}
