// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The two in-memory limiters (spec §4.7): sliding attempt windows for the sign-in routes and the fixed per-minute
//! request budget the pipeline counts. Both are bounded (16 384 keys each), which Node's were not.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::AuthSettings;
use crate::util;

pub const MAX_TRACKED_KEYS: usize = 16_384;
const SWEEP_EVERY_MS: i64 = 60_000;
const BUDGET_WINDOW_MS: i64 = 60_000;

/// Keys with an expiry each, the one that expires first found in log time.
struct Bounded<V> {
    entries: HashMap<String, (i64, V)>,
    by_expiry: BTreeSet<(i64, String)>,
    last_sweep: i64,
}

impl<V> Bounded<V> {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            by_expiry: BTreeSet::new(),
            last_sweep: i64::MIN,
        }
    }

    fn sweep(&mut self, now: i64) {
        while let Some((expiry, key)) = self.by_expiry.first().cloned() {
            if expiry > now {
                break;
            }
            self.by_expiry.remove(&(expiry, key.clone()));
            self.entries.remove(&key);
        }
        self.last_sweep = now;
    }

    /// Sweeps at most once a minute, when the clock went back, or at the cap; then makes room for a new key.
    fn prepare(&mut self, key: &str, now: i64) {
        let at_cap = self.entries.len() >= MAX_TRACKED_KEYS && !self.entries.contains_key(key);
        if self.last_sweep == i64::MIN || now - self.last_sweep >= SWEEP_EVERY_MS || now < self.last_sweep || at_cap {
            self.sweep(now);
        }
        if self.entries.len() >= MAX_TRACKED_KEYS
            && !self.entries.contains_key(key)
            && let Some(first) = self.by_expiry.first().cloned()
        {
            self.by_expiry.remove(&first);
            self.entries.remove(&first.1);
        }
    }

    fn set_expiry(&mut self, key: &str, expiry: i64) {
        if let Some((old, _)) = self.entries.get_mut(key) {
            self.by_expiry.remove(&(*old, key.to_owned()));
            *old = expiry;
            self.by_expiry.insert((expiry, key.to_owned()));
        }
    }
}

struct Window {
    attempts: VecDeque<i64>,
}

struct Budget {
    opened: i64,
    hits: u64,
}

/// What the request budget says about one hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetVerdict {
    /// Whether this hit is refused.
    pub limited: bool,
    /// `ratelimit`: `"{limit}-in-1min"; r={remaining}; t={seconds}`
    pub rate_limit: String,
    /// `ratelimit-policy`: `"{limit}-in-1min"; q={limit}; w=60; pk=:{pk}:`
    pub rate_limit_policy: String,
    /// `retry-after` when refused
    pub retry_after: Option<String>,
}

/// Attempt windows and per-minute request budgets, in memory and bounded in keys.
pub struct RateLimiter {
    windows: Mutex<Bounded<Window>>,
    budgets: Mutex<Bounded<Budget>>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    /// A limiter that tracks nothing yet.
    pub fn new() -> Self {
        Self {
            windows: Mutex::new(Bounded::new()),
            budgets: Mutex::new(Bounded::new()),
        }
    }

    /// Whether `key` has had `max` attempts within the last `window_ms`; an allowed attempt is recorded.
    pub fn is_limited_at(&self, key: &str, max: usize, window_ms: i64, now: i64) -> bool {
        let mut windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        windows.prepare(key, now);
        if !windows.entries.contains_key(key) {
            windows.entries.insert(
                key.to_owned(),
                (
                    i64::MIN,
                    Window {
                        attempts: VecDeque::new(),
                    },
                ),
            );
            windows.by_expiry.insert((i64::MIN, key.to_owned()));
        }
        let (limited, expiry) = match windows.entries.get_mut(key) {
            Some((_, window)) => {
                // An attempt counts while now - t < window
                window.attempts.retain(|&t| now - t < window_ms && t <= now);
                let limited = window.attempts.len() >= max;
                if !limited {
                    window.attempts.push_back(now);
                }
                let last = window.attempts.back().copied().unwrap_or(now);
                (limited, last.saturating_add(window_ms))
            }
            None => (false, now),
        };
        windows.set_expiry(key, expiry);
        limited
    }

    /// One hit on `key`'s fixed one-minute budget of `limit`.
    pub fn check_request_budget_at(&self, key: &str, limit: u64, now: i64) -> BudgetVerdict {
        let mut budgets = self.budgets.lock().unwrap_or_else(PoisonError::into_inner);
        budgets.prepare(key, now);
        let fresh = match budgets.entries.get(key) {
            Some((_, budget)) => now >= budget.opened + BUDGET_WINDOW_MS || now < budget.opened,
            None => true,
        };
        if fresh {
            if let Some((old, _)) = budgets.entries.remove(key) {
                budgets.by_expiry.remove(&(old, key.to_owned()));
            }
            budgets.entries.insert(
                key.to_owned(),
                (now + BUDGET_WINDOW_MS, Budget { opened: now, hits: 0 }),
            );
            budgets.by_expiry.insert((now + BUDGET_WINDOW_MS, key.to_owned()));
        }
        let (hits, opened) = match budgets.entries.get_mut(key) {
            Some((_, budget)) => {
                budget.hits += 1;
                (budget.hits, budget.opened)
            }
            None => (1, now),
        };
        let reset_ms = (opened + BUDGET_WINDOW_MS - now).max(0);
        let seconds = (reset_ms + 999) / 1000;
        let limited = hits > limit;
        let pk = STANDARD.encode(&util::sha256_hex(key)[..12]);
        BudgetVerdict {
            limited,
            rate_limit: format!("\"{limit}-in-1min\"; r={}; t={seconds}", limit.saturating_sub(hits)),
            rate_limit_policy: format!("\"{limit}-in-1min\"; q={limit}; w=60; pk=:{pk}:"),
            retry_after: limited.then(|| seconds.to_string()),
        }
    }

    /// Keys the attempt windows track (for tests and metrics).
    pub fn tracked_windows(&self) -> usize {
        self.windows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .len()
    }

    /// Keys the request budgets track (for tests and metrics).
    pub fn tracked_budgets(&self) -> usize {
        self.budgets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .len()
    }
}

/// The request budget's settings (§10.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetConfig {
    /// False when `RUSTIC_ISLES_RATE_LIMIT` is off.
    pub enabled: bool,
    /// A client's requests per minute (default 600).
    pub client_per_minute: u64,
    /// A game server's requests per minute (default 6000).
    pub gameserver_per_minute: u64,
    /// A player host's requests per minute (default 6000).
    pub host_per_minute: u64,
}

impl BudgetConfig {
    /// The budget from the settings; an unreadable number takes the default.
    pub fn from_settings(settings: &dyn AuthSettings) -> Self {
        let enabled = !matches!(
            util::normalize_lower(settings.raw("RUSTIC_ISLES_RATE_LIMIT").unwrap_or("")).as_str(),
            "0" | "off" | "false" | "no"
        );
        Self {
            enabled,
            client_per_minute: per_minute(settings.raw("RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE"), 600),
            gameserver_per_minute: per_minute(settings.raw("RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE"), 6000),
            host_per_minute: per_minute(settings.raw("RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE"), 6000),
        }
    }
}

/// An optional `+`, then the leading digits; under 1 or unreadable falls back.
fn per_minute(raw: Option<&str>, fallback: u64) -> u64 {
    let text = dainty_service_kit::js::trim(raw.unwrap_or(""));
    let text = text.strip_prefix('+').unwrap_or(text);
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    match digits.parse::<u64>() {
        Ok(n) if n >= 1 => n,
        Err(_) if !digits.is_empty() => u64::MAX,
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_slide() {
        let limiter = RateLimiter::new();
        assert!(!limiter.is_limited_at("k", 2, 1000, 0));
        assert!(!limiter.is_limited_at("k", 2, 1000, 10));
        assert!(limiter.is_limited_at("k", 2, 1000, 20));
        assert!(limiter.is_limited_at("k", 2, 1000, 30));
        assert!(!limiter.is_limited_at("other", 2, 1000, 30));
        assert!(!limiter.is_limited_at("k", 2, 1000, 1011));
    }

    #[test]
    fn windows_are_bounded() {
        let limiter = RateLimiter::new();
        let started = std::time::Instant::now();
        for i in 0..36_384 {
            limiter.is_limited_at(&format!("key-{i}"), 5, 1000, 0);
        }
        assert_eq!(limiter.tracked_windows(), MAX_TRACKED_KEYS);
        assert!(started.elapsed().as_secs() < 5);
        limiter.is_limited_at("late", 5, 1000, 70_000);
        assert_eq!(limiter.tracked_windows(), 1);
    }

    #[test]
    fn budget_headers_name_the_key_by_digest() {
        let limiter = RateLimiter::new();
        let verdict = limiter.check_request_budget_at("client:127.0.0.1", 600, 0);
        assert_eq!(
            verdict.rate_limit_policy,
            "\"600-in-1min\"; q=600; w=60; pk=:YjU5MDczYThlN2Fj:"
        );
        assert_eq!(verdict.rate_limit, "\"600-in-1min\"; r=599; t=60");
        let verdict = limiter.check_request_budget_at("gameserver:127.0.0.1", 6000, 0);
        assert!(verdict.rate_limit_policy.ends_with("pk=:OGJjODNiMzMzMjM2:"));
    }

    #[test]
    fn budgets_count_refused_hits_and_reset() {
        let limiter = RateLimiter::new();
        assert!(!limiter.check_request_budget_at("k", 2, 0).limited);
        assert!(!limiter.check_request_budget_at("k", 2, 100).limited);
        let refused = limiter.check_request_budget_at("k", 2, 30_500);
        assert!(refused.limited);
        assert_eq!(refused.retry_after.as_deref(), Some("30"));
        assert_eq!(refused.rate_limit, "\"2-in-1min\"; r=0; t=30");
        assert!(!limiter.check_request_budget_at("k", 2, 60_000).limited);
    }

    #[test]
    fn per_minute_values_read_leading_digits() {
        assert_eq!(per_minute(Some("+120abc"), 600), 120);
        assert_eq!(per_minute(Some("0"), 600), 600);
        assert_eq!(per_minute(Some("x"), 600), 600);
        assert_eq!(per_minute(None, 6000), 6000);
    }
}
