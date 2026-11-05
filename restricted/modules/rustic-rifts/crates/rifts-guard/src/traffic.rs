//! The Guard traffic bounds (`guardTraffic.ts`, `guardTrafficDistributed.ts`, `EnforceP2PGuardTraffic`): a fixed
//! window per host and per hunt session, local or in the store, and the in-flight request budget.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rifts_seam::UnixMillis;

use crate::digest::sha256_hex;
use crate::ports::lock;
use crate::store::{TrafficReservation, TrafficScope};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// `GuardTrafficLimit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrafficLimit {
    pub max_requests: u64,
    pub max_bytes: u64,
    pub window_ms: u64,
}

pub const HUNT_GUARD_TRAFFIC_WINDOW_MS: u64 = 60_000;
pub const HUNT_GUARD_HOST_TRAFFIC_LIMIT: TrafficLimit = TrafficLimit {
    max_requests: 180,
    max_bytes: 8 * 1024 * 1024,
    window_ms: HUNT_GUARD_TRAFFIC_WINDOW_MS,
};
pub const HUNT_GUARD_SESSION_TRAFFIC_LIMIT: TrafficLimit = TrafficLimit {
    max_requests: 120,
    max_bytes: 4 * 1024 * 1024,
    window_ms: HUNT_GUARD_TRAFFIC_WINDOW_MS,
};

fn valid_limit(limit: &TrafficLimit) -> bool {
    (1..=MAX_SAFE_INTEGER).contains(&limit.max_requests)
        && (1..=MAX_SAFE_INTEGER).contains(&limit.max_bytes)
        && (1..=MAX_SAFE_INTEGER).contains(&limit.window_ms)
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    started_at: i64,
    requests: u64,
    bytes: u64,
    touched_at: i64,
}

/// `GuardTrafficLimiter`: the process-local fallback, fail-closed, with old keys evicted past `max_keys`.
#[derive(Debug)]
pub struct LocalTrafficLimiter {
    buckets: HashMap<String, Bucket>,
    max_keys: usize,
}

impl Default for LocalTrafficLimiter {
    fn default() -> Self {
        Self::new(4096)
    }
}

impl LocalTrafficLimiter {
    pub fn new(max_keys: usize) -> Self {
        Self {
            buckets: HashMap::new(),
            max_keys,
        }
    }

    pub fn consume(&mut self, key: &str, bytes: u64, limit: &TrafficLimit, now: UnixMillis) -> bool {
        if key.is_empty() || bytes > MAX_SAFE_INTEGER || !valid_limit(limit) || bytes > limit.max_bytes {
            return false;
        }
        let current = now.0;
        let window = i64::try_from(limit.window_ms).unwrap_or(i64::MAX);
        let mut bucket = match self.buckets.get(key) {
            Some(bucket) if current - bucket.started_at < window && current >= bucket.started_at => *bucket,
            _ => Bucket {
                started_at: current,
                requests: 0,
                bytes: 0,
                touched_at: current,
            },
        };
        bucket.touched_at = current;
        if bucket.requests >= limit.max_requests || bucket.bytes + bytes > limit.max_bytes {
            self.buckets.insert(key.to_owned(), bucket);
            return false;
        }
        bucket.requests += 1;
        bucket.bytes += bytes;
        self.buckets.insert(key.to_owned(), bucket);
        self.evict_if_needed();
        true
    }

    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    fn evict_if_needed(&mut self) {
        while self.buckets.len() > self.max_keys {
            let oldest = self
                .buckets
                .iter()
                .min_by_key(|(_, bucket)| bucket.touched_at)
                .map(|(key, _)| key.clone());
            match oldest {
                Some(key) => self.buckets.remove(&key),
                None => break,
            };
        }
    }
}

/// `GuardRequestBudgetLimit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestBudgetLimit {
    pub max_in_flight_requests: u64,
    pub max_in_flight_bytes: u64,
}

pub const HUNT_GUARD_REQUEST_BUDGET: RequestBudgetLimit = RequestBudgetLimit {
    max_in_flight_requests: 64,
    max_in_flight_bytes: 1024 * 1024,
};

/// `GuardRequestBudget`: bounds the request work in flight, with no queue. The permit releases on drop, once.
#[derive(Debug, Clone)]
pub struct RequestBudget {
    limit: RequestBudgetLimit,
    in_flight: Arc<Mutex<(u64, u64)>>,
}

impl RequestBudget {
    pub fn new(limit: RequestBudgetLimit) -> Self {
        Self {
            limit,
            in_flight: Arc::new(Mutex::new((0, 0))),
        }
    }

    pub fn try_acquire(&self, bytes: u64) -> Option<BudgetPermit> {
        let mut in_flight = lock(&self.in_flight);
        if bytes > MAX_SAFE_INTEGER
            || in_flight.0 >= self.limit.max_in_flight_requests
            || in_flight.1 + bytes > self.limit.max_in_flight_bytes
        {
            return None;
        }
        in_flight.0 += 1;
        in_flight.1 += bytes;
        Some(BudgetPermit {
            in_flight: Arc::clone(&self.in_flight),
            bytes,
        })
    }

    /// (requests, bytes) in flight.
    pub fn snapshot(&self) -> (u64, u64) {
        *lock(&self.in_flight)
    }
}

#[derive(Debug)]
pub struct BudgetPermit {
    in_flight: Arc<Mutex<(u64, u64)>>,
    bytes: u64,
}

impl Drop for BudgetPermit {
    fn drop(&mut self) {
        let mut in_flight = lock(&self.in_flight);
        in_flight.0 = in_flight.0.saturating_sub(1);
        in_flight.1 = in_flight.1.saturating_sub(self.bytes);
    }
}

/// `WindowStart`.
pub fn window_start(now: UnixMillis, window_ms: u64) -> UnixMillis {
    let window = i64::try_from(window_ms).unwrap_or(i64::MAX).max(1);
    UnixMillis(now.0.div_euclid(window) * window)
}

/// `WindowId`: the key only as a digest.
pub fn window_id(scope: TrafficScope, key: &str, start: UnixMillis) -> String {
    let digest = sha256_hex(format!("{}\0{key}\0{}", scope.as_str(), start.0).as_bytes());
    format!("{}:{}:{digest}", scope.as_str(), start.0)
}

/// `ValidateReservation`.
pub fn validate_reservation(key: &str, bytes: u64, limit: &TrafficLimit) -> bool {
    !key.is_empty()
        && key.chars().map(char::len_utf16).sum::<usize>() <= 256
        && bytes <= MAX_SAFE_INTEGER
        && valid_limit(limit)
        && bytes <= limit.max_bytes
}

/// The reservation `ReserveWindow` makes for one key; `None` when it fails validation (a refusal, as in Node).
pub fn reservation(
    scope: TrafficScope,
    key: &str,
    bytes: u64,
    limit: &TrafficLimit,
    now: UnixMillis,
) -> Option<TrafficReservation> {
    if !validate_reservation(key, bytes, limit) {
        return None;
    }
    let start = window_start(now, limit.window_ms);
    let window = i64::try_from(limit.window_ms).unwrap_or(i64::MAX);
    Some(TrafficReservation {
        window_id: window_id(scope, key, start),
        scope,
        window_start: start,
        expires_at: UnixMillis(start.0.saturating_add(window.saturating_mul(2))),
        bytes,
        max_requests: limit.max_requests,
        max_bytes: limit.max_bytes,
    })
}

/// Which `Handle()` operation a path is (`P2PHuntOperationOf`).
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

    /// `CountsAgainstGuardTraffic`: only Guard traffic. A busy hunt's batches once used up the windows and its lease
    /// renewals got 429 with them, so the hunt was fenced under its players.
    pub const fn counts_against_guard_traffic(self) -> bool {
        !matches!(self, Self::Lifecycle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // guardTraffic.test.ts
    #[test]
    fn limiter_bounds_requests_and_bytes_then_resets_its_window() {
        let mut limiter = LocalTrafficLimiter::new(2);
        let limit = TrafficLimit {
            max_requests: 2,
            max_bytes: 10,
            window_ms: 1_000,
        };
        let at = UnixMillis(1_000);
        assert!(limiter.consume("host:a", 6, &limit, at));
        assert!(!limiter.consume("host:a", 5, &limit, at));
        assert!(limiter.consume("host:a", 4, &limit, at));
        assert!(!limiter.consume("host:a", 1, &limit, at));
        assert!(limiter.consume("host:a", 10, &limit, UnixMillis(2_000)));
    }

    #[test]
    fn limiter_evicts_old_keys_at_its_bound() {
        let mut limiter = LocalTrafficLimiter::new(2);
        let limit = TrafficLimit {
            max_requests: 10,
            max_bytes: 100,
            window_ms: 60_000,
        };
        for key in ["a", "b", "c"] {
            assert!(limiter.consume(key, 1, &limit, UnixMillis(1)));
        }
        assert_eq!(limiter.len(), 2);
    }

    #[test]
    fn request_budget_rejects_aggregate_bytes_and_releases_exactly_once() {
        let budget = RequestBudget::new(RequestBudgetLimit {
            max_in_flight_requests: 2,
            max_in_flight_bytes: 10,
        });
        let first = budget.try_acquire(6).unwrap();
        assert!(budget.try_acquire(5).is_none());
        let second = budget.try_acquire(4).unwrap();
        assert!(budget.try_acquire(1).is_none());
        assert_eq!(budget.snapshot(), (2, 10));
        drop(first);
        assert_eq!(budget.snapshot(), (1, 4));
        drop(second);
        assert_eq!(budget.snapshot(), (0, 0));
    }

    // guardTrafficDistributed.test.ts: "distributed Guard windows are stable, bounded and key-hashed"
    #[test]
    fn distributed_windows_are_stable_bounded_and_key_hashed() {
        let limit = TrafficLimit {
            max_requests: 10,
            max_bytes: 100,
            window_ms: 60_000,
        };
        let start = window_start(UnixMillis(61_234), limit.window_ms);
        assert_eq!(start, UnixMillis(60_000));
        let id = window_id(TrafficScope::Host, "account-secret", start);
        let (prefix, digest) = id.split_at("host:60000:".len());
        assert_eq!(prefix, "host:60000:");
        assert!(digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(!id.contains("account-secret"));
        assert!(validate_reservation("a", 100, &limit));
        assert!(!validate_reservation("a", 101, &limit));
        assert!(!validate_reservation("", 1, &limit));
        assert!(!validate_reservation(&"k".repeat(257), 1, &limit));
    }

    // p2pGuardTraffic.test.ts: "only Guard traffic counts against the Guard traffic windows; the lease and the other
    // lifecycle calls do not"
    #[test]
    fn only_guard_traffic_counts_against_the_windows() {
        let session = "/mystic/v1/hunts/11111111-1111-4111-8111-111111111111";
        assert_eq!(
            HuntOperation::of(&format!("{session}/event-batches")),
            HuntOperation::EventBatch
        );
        assert_eq!(
            HuntOperation::of(&format!("{session}/completion")),
            HuntOperation::Completion
        );
        assert_eq!(
            HuntOperation::of("/internal/v1/hunt-guard/shadow-comparisons"),
            HuntOperation::Shadow
        );
        for lifecycle in [
            "lease",
            "host-ready",
            "tickets/consume",
            "abort",
            "end",
            "peers/abc/disconnect",
        ] {
            let operation = HuntOperation::of(&format!("{session}/{lifecycle}"));
            assert_eq!(operation, HuntOperation::Lifecycle, "{lifecycle}");
            assert!(!operation.counts_against_guard_traffic(), "{lifecycle}");
        }
        for operation in [
            HuntOperation::EventBatch,
            HuntOperation::Completion,
            HuntOperation::Shadow,
        ] {
            assert!(operation.counts_against_guard_traffic());
        }
    }
}
