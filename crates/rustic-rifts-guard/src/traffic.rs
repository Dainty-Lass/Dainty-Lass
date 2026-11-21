// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Bounds on what a host's Guard traffic may cost: per-minute windows per host and per hunt, and a global
//! in-flight budget.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use rustic_rifts_seam::UnixMillis;

use crate::{
    digest::sha256_hex,
    json::js_length,
    store::{TrafficReservation, TrafficScope},
};

/// `Number.MAX_SAFE_INTEGER`: the stored counters are JavaScript numbers.
const MAX_SAFE: u64 = (1 << 53) - 1;
const WINDOW_MS: u64 = 60_000;
const DEFAULT_LIMITER_KEYS: usize = 4096;
const MAX_KEY_UNITS: usize = 256;

/// One key's bound per fixed window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrafficLimit {
    /// Requests per window.
    pub max_requests: u64,
    /// Bytes per window.
    pub max_bytes: u64,
    /// The window's length in milliseconds.
    pub window_ms: u64,
}

impl TrafficLimit {
    fn is_valid(&self) -> bool {
        let safe = |n: u64| n != 0 && n <= MAX_SAFE;
        safe(self.max_requests) && safe(self.max_bytes) && safe(self.window_ms)
    }

    /// One charge of `bytes` can ever fit under this limit.
    fn admits(&self, bytes: u64) -> bool {
        self.is_valid() && bytes <= self.max_bytes.min(MAX_SAFE)
    }
}

/// Each host account's bound: 180 requests and 8 MiB a minute.
pub const HUNT_GUARD_HOST_TRAFFIC_LIMIT: TrafficLimit = TrafficLimit {
    max_requests: 180,
    max_bytes: 8 << 20,
    window_ms: WINDOW_MS,
};

/// Each hunt session's bound: 120 requests and 4 MiB a minute.
pub const HUNT_GUARD_SESSION_TRAFFIC_LIMIT: TrafficLimit = TrafficLimit {
    max_requests: 120,
    max_bytes: 4 << 20,
    window_ms: WINDOW_MS,
};

/// The bound on requests in flight at once, across every host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestBudgetLimit {
    /// Requests in flight at once.
    pub max_in_flight_requests: u64,
    /// Body bytes in flight at once.
    pub max_in_flight_bytes: u64,
}

/// The hunt routes' in-flight bound: 64 requests and 1 MiB.
pub const HUNT_GUARD_REQUEST_BUDGET: RequestBudgetLimit = RequestBudgetLimit {
    max_in_flight_requests: 64,
    max_in_flight_bytes: 1 << 20,
};

#[derive(Debug, Default)]
struct InFlight {
    requests: u64,
    bytes: u64,
}

type Shared = Arc<Mutex<InFlight>>;

fn held(shared: &Shared) -> MutexGuard<'_, InFlight> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Requests and bytes in flight across every host; a clone shares the counters.
#[derive(Clone, Debug)]
pub struct RequestBudget {
    limit: RequestBudgetLimit,
    in_flight: Shared,
}

/// One admitted request; dropping it gives its share back.
#[derive(Debug)]
pub struct BudgetPermit {
    bytes: u64,
    in_flight: Shared,
}

impl Drop for BudgetPermit {
    fn drop(&mut self) {
        let mut in_flight = held(&self.in_flight);
        in_flight.requests = in_flight.requests.saturating_sub(1);
        in_flight.bytes = in_flight.bytes.saturating_sub(self.bytes);
    }
}

impl RequestBudget {
    /// A budget with nothing in flight.
    pub fn new(limit: RequestBudgetLimit) -> Self {
        let in_flight = Shared::default();
        Self { limit, in_flight }
    }

    /// No queue: a request that does not fit now is refused.
    pub fn try_acquire(&self, bytes: u64) -> Option<BudgetPermit> {
        let mut in_flight = held(&self.in_flight);
        let fits = bytes <= MAX_SAFE
            && in_flight.requests < self.limit.max_in_flight_requests
            && in_flight.bytes.saturating_add(bytes) <= self.limit.max_in_flight_bytes;
        if !fits {
            return None;
        }
        in_flight.requests += 1;
        in_flight.bytes += bytes;
        let in_flight = Arc::clone(&self.in_flight);
        Some(BudgetPermit { bytes, in_flight })
    }

    /// `(requests, bytes)` in flight.
    pub fn snapshot(&self) -> (u64, u64) {
        let in_flight = held(&self.in_flight);
        (in_flight.requests, in_flight.bytes)
    }
}

#[derive(Clone, Copy, Debug)]
struct Window {
    start: i64,
    requests: u64,
    bytes: u64,
    touched: u64,
}

impl Window {
    fn fresh(start: i64, touched: u64) -> Self {
        Self {
            start,
            requests: 0,
            bytes: 0,
            touched,
        }
    }

    /// Past its length, or begun after `now` because the clock went back.
    fn expired(&self, now: i64, length_ms: u64) -> bool {
        u64::try_from(now.saturating_sub(self.start)).map_or(true, |elapsed| elapsed >= length_ms)
    }
}

/// Fixed windows per key in this process, least recently touched keys dropped past the capacity.
#[derive(Debug)]
pub struct LocalTrafficLimiter {
    max_keys: usize,
    windows: HashMap<String, Window>,
    touches: u64,
}

impl Default for LocalTrafficLimiter {
    fn default() -> Self {
        Self::new(DEFAULT_LIMITER_KEYS)
    }
}

impl LocalTrafficLimiter {
    /// A limiter keeping at most `max_keys` keys.
    pub fn new(max_keys: usize) -> Self {
        Self {
            max_keys,
            windows: HashMap::default(),
            touches: 0,
        }
    }

    /// How many keys have a window.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Whether no key has a window.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn evict_stale(&mut self) {
        while self.windows.len() > self.max_keys {
            let oldest = self
                .windows
                .iter()
                .min_by_key(|(_, window)| window.touched)
                .map(|(key, _)| key.clone());
            let Some(key) = oldest else {
                return;
            };
            self.windows.remove(&key);
        }
    }

    /// Charges one request of `bytes` to `key`'s window; `false`, charging nothing, when it does not fit.
    pub fn consume(&mut self, key: &str, bytes: u64, limit: TrafficLimit, now: UnixMillis) -> bool {
        if key.is_empty() || !limit.admits(bytes) {
            return false;
        }
        self.touches += 1;
        let touched = self.touches;
        let window = self
            .windows
            .entry(key.to_owned())
            .or_insert_with(|| Window::fresh(now.0, touched));
        if window.expired(now.0, limit.window_ms) {
            *window = Window::fresh(now.0, touched);
        }
        window.touched = touched;
        let room = window.requests < limit.max_requests && window.bytes.saturating_add(bytes) <= limit.max_bytes;
        if room {
            window.requests += 1;
            window.bytes += bytes;
            self.evict_stale();
        }
        room
    }
}

/// Which hunt route a path is, for what counts against Guard's traffic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HuntOperation {
    /// `.../event-batches`.
    EventBatch,
    /// `.../completion`.
    Completion,
    /// `.../shadow-comparisons`.
    Shadow,
    /// Any other hunt route: opening, closing and the like.
    Lifecycle,
}

impl HuntOperation {
    /// The operation a request path names.
    pub fn of(path: &str) -> Self {
        const BY_PATH: [(&str, HuntOperation); 3] = [
            ("event-batches", HuntOperation::EventBatch),
            ("completion", HuntOperation::Completion),
            ("shadow-comparisons", HuntOperation::Shadow),
        ];
        let found = BY_PATH.iter().find(|(part, _)| path.contains(part));
        found.map_or(Self::Lifecycle, |(_, operation)| *operation)
    }

    /// A hunt's lease renewals must never be starved by its own evidence.
    pub fn counts_against_guard_traffic(self) -> bool {
        !matches!(self, Self::Lifecycle)
    }
}

pub(crate) fn host_key(account: &str) -> String {
    ["guard-host:", account].concat()
}

pub(crate) fn session_key(session: &str) -> String {
    ["guard-session:", session].concat()
}

/// The store's reservation for one key's window; `None` when the key or charge is out of bounds.
pub(crate) fn reservation(
    scope: TrafficScope,
    key: &str,
    bytes: u64,
    limit: TrafficLimit,
    now: UnixMillis,
) -> Option<TrafficReservation> {
    let key_ok = !key.is_empty() && js_length(key) <= MAX_KEY_UNITS;
    if !(key_ok && limit.admits(bytes)) {
        return None;
    }
    let length = i64::try_from(limit.window_ms).ok()?;
    let start = now.0 - now.0.rem_euclid(length);
    let name = scope.as_str();
    // The key itself never reaches the store: only its hash, with the scope and window.
    let digest = sha256_hex([name, key, &start.to_string()].join("\0").as_bytes());
    Some(TrafficReservation {
        window_id: format!("{name}:{start}:{digest}"),
        window_start: UnixMillis(start),
        expires_at: UnixMillis(start.saturating_add(length.saturating_mul(2))),
        max_requests: limit.max_requests,
        max_bytes: limit.max_bytes,
        scope,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: TrafficLimit = TrafficLimit {
        max_requests: 2,
        max_bytes: 100,
        window_ms: 1000,
    };

    #[test]
    fn local_limiter_counts_requests_and_bytes() {
        let mut limiter = LocalTrafficLimiter::default();
        let invalid = TrafficLimit { window_ms: 0, ..SMALL };
        let steps = [
            ("k", 10, SMALL, 0, true),
            ("k", 10, SMALL, 10, true),
            ("k", 1, SMALL, 20, false),
            ("k", 1, SMALL, 1000, true),
            ("k", 100, SMALL, 1001, false),
            // A clock before the window's start restarts it.
            ("k", 100, SMALL, 5, true),
            ("", 1, SMALL, 0, false),
            ("j", 101, SMALL, 0, false),
            ("j", 1, invalid, 0, false),
        ];
        for (key, bytes, limit, at, accepted) in steps {
            assert_eq!(
                limiter.consume(key, bytes, limit, UnixMillis(at)),
                accepted,
                "{key} {bytes} @{at}"
            );
        }
        assert_eq!(limiter.len(), 1);
    }

    #[test]
    fn local_limiter_evicts_the_least_recently_touched() {
        let mut limiter = LocalTrafficLimiter::new(2);
        assert!(limiter.is_empty());
        for key in ["a", "b", "a", "c"] {
            assert!(limiter.consume(key, 1, SMALL, UnixMillis(0)), "{key}");
        }
        assert_eq!(limiter.len(), 2);
        // `a` survived with its two requests, `b` went.
        assert!(!limiter.consume("a", 1, SMALL, UnixMillis(1)));
        assert!(limiter.consume("b", 1, SMALL, UnixMillis(1)));
    }

    #[test]
    fn budget_releases_exactly_once() {
        let budget = RequestBudget::new(RequestBudgetLimit {
            max_in_flight_requests: 2,
            max_in_flight_bytes: 100,
        });
        let first = budget.try_acquire(60).unwrap();
        assert!(budget.try_acquire(41).is_none());
        let second = budget.clone().try_acquire(40).unwrap();
        assert_eq!(budget.snapshot(), (2, 100));
        assert!(budget.try_acquire(0).is_none());
        drop(first);
        assert_eq!(budget.snapshot(), (1, 40));
        drop(second);
        assert_eq!(budget.snapshot(), (0, 0));
        assert!(budget.try_acquire(MAX_SAFE + 1).is_none());
        assert_eq!(MAX_SAFE, 9_007_199_254_740_991);
    }

    #[test]
    fn reservation_shape() {
        let host = HUNT_GUARD_HOST_TRAFFIC_LIMIT;
        let r = reservation(TrafficScope::Host, "guard-host:acct", 10, host, UnixMillis(125_000)).unwrap();
        assert_eq!(
            (r.window_start, r.expires_at),
            (UnixMillis(120_000), UnixMillis(240_000))
        );
        let digest = sha256_hex(b"host\x00guard-host:acct\x00120000");
        assert_eq!(r.window_id, format!("host:120000:{digest}"));
        assert!(!r.window_id.contains("acct"));
        assert_eq!((r.max_requests, r.max_bytes, r.bytes), (180, 8_388_608, 10));
        assert_eq!(HUNT_GUARD_SESSION_TRAFFIC_LIMIT.max_bytes, 4_194_304);
        assert_eq!(HUNT_GUARD_REQUEST_BUDGET.max_in_flight_bytes, 1_048_576);

        let negative = reservation(TrafficScope::Session, "k", 1, SMALL, UnixMillis(-1)).unwrap();
        assert_eq!(negative.window_start, UnixMillis(-1000));
        assert!(negative.window_id.starts_with("session:-1000:"));
        let bounds = [
            ("".to_owned(), 1, false),
            ("k".repeat(257), 1, false),
            ("k".repeat(256), 1, true),
            ("k".into(), 101, false),
        ];
        for (key, bytes, made) in bounds {
            assert_eq!(
                reservation(TrafficScope::Host, &key, bytes, SMALL, UnixMillis(0)).is_some(),
                made,
                "{}",
                key.len()
            );
        }
    }

    #[test]
    fn operations() {
        let paths = [
            ("/rustic/v1/hunts/x/event-batches", HuntOperation::EventBatch),
            ("/rustic/v1/hunts/x/completion", HuntOperation::Completion),
            ("/internal/v1/hunt-guard/shadow-comparisons", HuntOperation::Shadow),
            ("/rustic/v1/hunts/x/lease", HuntOperation::Lifecycle),
        ];
        for (path, operation) in paths {
            assert_eq!(HuntOperation::of(path), operation, "{path}");
        }
        assert!(!HuntOperation::Lifecycle.counts_against_guard_traffic());
        assert!(HuntOperation::Shadow.counts_against_guard_traffic());
    }
}
