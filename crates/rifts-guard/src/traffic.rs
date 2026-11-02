// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Bounds on what a host's Guard traffic may cost: per-minute windows per host and per hunt, and a global
//! in-flight budget.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use rifts_seam::UnixMillis;

use crate::digest::sha256_hex;
use crate::json::js_length;
use crate::store::{TrafficReservation, TrafficScope};

const MAX_SAFE: u64 = 9_007_199_254_740_991;
const WINDOW_MS: u64 = 60_000;
const DEFAULT_LIMITER_KEYS: usize = 4096;
const MAX_KEY_UNITS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrafficLimit {
    pub max_requests: u64,
    pub max_bytes: u64,
    pub window_ms: u64,
}

impl TrafficLimit {
    fn is_valid(&self) -> bool {
        [self.max_requests, self.max_bytes, self.window_ms]
            .iter()
            .all(|n| (1..=MAX_SAFE).contains(n))
    }
}

pub const HUNT_GUARD_HOST_TRAFFIC_LIMIT: TrafficLimit = TrafficLimit {
    max_requests: 180,
    max_bytes: 8_388_608,
    window_ms: WINDOW_MS,
};

pub const HUNT_GUARD_SESSION_TRAFFIC_LIMIT: TrafficLimit = TrafficLimit {
    max_requests: 120,
    max_bytes: 4_194_304,
    window_ms: WINDOW_MS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestBudgetLimit {
    pub max_in_flight_requests: u64,
    pub max_in_flight_bytes: u64,
}

pub const HUNT_GUARD_REQUEST_BUDGET: RequestBudgetLimit = RequestBudgetLimit {
    max_in_flight_requests: 64,
    max_in_flight_bytes: 1_048_576,
};

/// Requests and bytes in flight across every host; a clone shares the counters.
#[derive(Debug, Clone)]
pub struct RequestBudget {
    limit: RequestBudgetLimit,
    in_flight: Arc<Mutex<(u64, u64)>>,
}

/// One admitted request; dropping it gives its share back.
#[derive(Debug)]
pub struct BudgetPermit {
    bytes: u64,
    in_flight: Arc<Mutex<(u64, u64)>>,
}

impl Drop for BudgetPermit {
    fn drop(&mut self) {
        let mut counts = self.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
        counts.0 = counts.0.saturating_sub(1);
        counts.1 = counts.1.saturating_sub(self.bytes);
    }
}

impl RequestBudget {
    pub fn new(limit: RequestBudgetLimit) -> Self {
        Self {
            limit,
            in_flight: Arc::new(Mutex::new((0, 0))),
        }
    }

    /// No queue: a request that does not fit now is refused.
    pub fn try_acquire(&self, bytes: u64) -> Option<BudgetPermit> {
        if bytes > MAX_SAFE {
            return None;
        }
        let mut counts = self.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
        if counts.0 >= self.limit.max_in_flight_requests
            || counts.1.saturating_add(bytes) > self.limit.max_in_flight_bytes
        {
            return None;
        }
        counts.0 += 1;
        counts.1 += bytes;
        Some(BudgetPermit {
            bytes,
            in_flight: self.in_flight.clone(),
        })
    }

    /// `(requests, bytes)` in flight.
    pub fn snapshot(&self) -> (u64, u64) {
        *self.in_flight.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Debug, Clone, Copy)]
struct Window {
    start: i64,
    requests: u64,
    bytes: u64,
    touched: u64,
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
    pub fn new(max_keys: usize) -> Self {
        Self {
            max_keys,
            windows: HashMap::new(),
            touches: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn consume(&mut self, key: &str, bytes: u64, limit: TrafficLimit, now: UnixMillis) -> bool {
        if key.is_empty() || bytes > MAX_SAFE || !limit.is_valid() || bytes > limit.max_bytes {
            return false;
        }
        self.touches += 1;
        let touched = self.touches;
        let window = self.windows.entry(key.to_owned()).or_insert(Window {
            start: now.0,
            requests: 0,
            bytes: 0,
            touched,
        });
        let elapsed = now.0.saturating_sub(window.start);
        // a clock that went back also restarts the window
        if elapsed < 0 || elapsed as u64 >= limit.window_ms {
            *window = Window {
                start: now.0,
                requests: 0,
                bytes: 0,
                touched,
            };
        }
        window.touched = touched;
        if window.requests >= limit.max_requests || window.bytes.saturating_add(bytes) > limit.max_bytes {
            return false;
        }
        window.requests += 1;
        window.bytes += bytes;
        while self.windows.len() > self.max_keys {
            let Some(oldest) = self
                .windows
                .iter()
                .min_by_key(|(_, w)| w.touched)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.windows.remove(&oldest);
        }
        true
    }
}

/// Which hunt route a path is, for what counts against Guard's traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuntOperation {
    EventBatch,
    Completion,
    Shadow,
    Lifecycle,
}

impl HuntOperation {
    pub fn of(path: &str) -> Self {
        if path.contains("event-batches") {
            Self::EventBatch
        } else if path.contains("completion") {
            Self::Completion
        } else if path.contains("shadow-comparisons") {
            Self::Shadow
        } else {
            Self::Lifecycle
        }
    }

    /// A hunt's lease renewals must never be starved by its own evidence.
    pub fn counts_against_guard_traffic(self) -> bool {
        self != Self::Lifecycle
    }
}

pub(crate) fn host_key(account: &str) -> String {
    format!("guard-host:{account}")
}

pub(crate) fn session_key(session: &str) -> String {
    format!("guard-session:{session}")
}

/// The store's reservation for one key's window; `None` when the key or charge is out of bounds.
pub(crate) fn reservation(
    scope: TrafficScope,
    key: &str,
    bytes: u64,
    limit: TrafficLimit,
    now: UnixMillis,
) -> Option<TrafficReservation> {
    if key.is_empty() || js_length(key) > MAX_KEY_UNITS || bytes > MAX_SAFE || !limit.is_valid() {
        return None;
    }
    if bytes > limit.max_bytes {
        return None;
    }
    let window = i64::try_from(limit.window_ms).ok()?;
    let start = now.0.div_euclid(window) * window;
    let digest = sha256_hex(format!("{}\0{key}\0{start}", scope.as_str()).as_bytes());
    Some(TrafficReservation {
        window_id: format!("{}:{start}:{digest}", scope.as_str()),
        scope,
        window_start: UnixMillis(start),
        expires_at: UnixMillis(start.saturating_add(window.saturating_mul(2))),
        bytes,
        max_requests: limit.max_requests,
        max_bytes: limit.max_bytes,
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
        assert!(limiter.consume("k", 10, SMALL, UnixMillis(0)));
        assert!(limiter.consume("k", 10, SMALL, UnixMillis(10)));
        assert!(!limiter.consume("k", 1, SMALL, UnixMillis(20)));
        assert!(limiter.consume("k", 1, SMALL, UnixMillis(1000)));
        assert!(!limiter.consume("k", 100, SMALL, UnixMillis(1001)));
        // a clock before the window's start restarts it
        assert!(limiter.consume("k", 100, SMALL, UnixMillis(5)));
        assert!(!limiter.consume("", 1, SMALL, UnixMillis(0)));
        assert!(!limiter.consume("j", 101, SMALL, UnixMillis(0)));
        let invalid = TrafficLimit { window_ms: 0, ..SMALL };
        assert!(!limiter.consume("j", 1, invalid, UnixMillis(0)));
        assert_eq!(limiter.len(), 1);
    }

    #[test]
    fn local_limiter_evicts_the_least_recently_touched() {
        let mut limiter = LocalTrafficLimiter::new(2);
        assert!(limiter.is_empty());
        assert!(limiter.consume("a", 1, SMALL, UnixMillis(0)));
        assert!(limiter.consume("b", 1, SMALL, UnixMillis(0)));
        assert!(limiter.consume("a", 1, SMALL, UnixMillis(0)));
        assert!(limiter.consume("c", 1, SMALL, UnixMillis(0)));
        assert_eq!(limiter.len(), 2);
        // `a` survived with its two requests, `b` went
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
    }

    #[test]
    fn reservation_shape() {
        let r = reservation(
            TrafficScope::Host,
            "guard-host:acct",
            10,
            HUNT_GUARD_HOST_TRAFFIC_LIMIT,
            UnixMillis(125_000),
        )
        .unwrap();
        assert_eq!(r.window_start, UnixMillis(120_000));
        assert_eq!(r.expires_at, UnixMillis(240_000));
        let digest = sha256_hex(b"host\x00guard-host:acct\x00120000");
        assert_eq!(r.window_id, format!("host:120000:{digest}"));
        assert!(!r.window_id.contains("acct"));
        assert_eq!((r.max_requests, r.max_bytes, r.bytes), (180, 8_388_608, 10));
        let negative = reservation(TrafficScope::Session, "k", 1, SMALL, UnixMillis(-1)).unwrap();
        assert_eq!(negative.window_start, UnixMillis(-1000));
        assert!(negative.window_id.starts_with("session:-1000:"));
        assert!(reservation(TrafficScope::Host, "", 1, SMALL, UnixMillis(0)).is_none());
        assert!(reservation(TrafficScope::Host, &"k".repeat(257), 1, SMALL, UnixMillis(0)).is_none());
        assert!(reservation(TrafficScope::Host, &"k".repeat(256), 1, SMALL, UnixMillis(0)).is_some());
        assert!(reservation(TrafficScope::Host, "k", 101, SMALL, UnixMillis(0)).is_none());
    }

    #[test]
    fn operations() {
        assert_eq!(
            HuntOperation::of("/rustic/v1/hunts/x/event-batches"),
            HuntOperation::EventBatch
        );
        assert_eq!(
            HuntOperation::of("/rustic/v1/hunts/x/completion"),
            HuntOperation::Completion
        );
        assert_eq!(
            HuntOperation::of("/internal/v1/hunt-guard/shadow-comparisons"),
            HuntOperation::Shadow
        );
        assert_eq!(HuntOperation::of("/rustic/v1/hunts/x/lease"), HuntOperation::Lifecycle);
        assert!(!HuntOperation::Lifecycle.counts_against_guard_traffic());
        assert!(HuntOperation::Shadow.counts_against_guard_traffic());
    }
}
