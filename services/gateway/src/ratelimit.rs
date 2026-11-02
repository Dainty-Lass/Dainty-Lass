// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Token buckets per rate key (an IPv4 address or an IPv6 /64), one limiter per class.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use service_kit::js;

const MAX_KEYS: usize = 100_000;
const MAX_VALUE: f64 = 1_000_000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BucketSpec {
    pub burst: f64,
    pub per_minute: f64,
}

/// `"<burst>,<per minute>"`, as `GATEWAY_RATE_*` take it.
pub fn parse_bucket_spec(text: &str, name: &str) -> Result<BucketSpec, String> {
    let shape = || format!("{name} must look like \"<burst>,<per minute>\", for example \"300,180\"");
    let (burst, rate) = text.split_once(',').ok_or_else(shape)?;
    let (burst, rate) = (js::trim(burst), js::trim(rate));
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let rate_ok = match rate.split_once('.') {
        Some((whole, frac)) => digits(whole) && digits(frac),
        None => digits(rate),
    };
    if !digits(burst) || !rate_ok {
        return Err(shape());
    }
    let burst: f64 = burst.parse().map_err(|_| shape())?;
    let per_minute: f64 = rate.parse().map_err(|_| shape())?;
    if !(1.0..=MAX_VALUE).contains(&burst) || per_minute > MAX_VALUE {
        return Err(format!(
            "{name}: burst must be 1 to 1000000 and the rate at most 1000000 per minute"
        ));
    }
    Ok(BucketSpec { burst, per_minute })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Take {
    Allowed,
    /// Seconds until a token is back
    Limited {
        retry_after: u64,
    },
}

struct Bucket {
    tokens: f64,
    at_ms: f64,
    order: u64,
}

#[derive(Default)]
struct Buckets {
    by_key: HashMap<String, Bucket>,
    /// Keys in the order they first came
    order: BTreeMap<u64, String>,
    next: u64,
}

pub struct RateLimiter {
    spec: BucketSpec,
    start: Instant,
    buckets: Mutex<Buckets>,
}

impl RateLimiter {
    pub fn new(spec: BucketSpec) -> Self {
        Self {
            spec,
            start: Instant::now(),
            buckets: Mutex::default(),
        }
    }

    pub fn spec(&self) -> BucketSpec {
        self.spec
    }

    fn now_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    fn refilled(&self, bucket: &Bucket, now_ms: f64) -> f64 {
        let gained = (now_ms - bucket.at_ms).max(0.0) / 60_000.0 * self.spec.per_minute;
        (bucket.tokens + gained).min(self.spec.burst)
    }

    pub fn take(&self, key: &str) -> Take {
        self.take_at(key, self.now_ms())
    }

    /// One token for `key` at `now_ms` (milliseconds on the limiter's own clock).
    pub fn take_at(&self, key: &str, now_ms: f64) -> Take {
        let mut guard = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if !guard.by_key.contains_key(key) {
            if guard.by_key.len() >= MAX_KEYS {
                self.sweep_locked(&mut guard, now_ms);
            }
            if guard.by_key.len() >= MAX_KEYS
                && let Some((_, oldest)) = guard.order.pop_first()
            {
                guard.by_key.remove(&oldest);
            }
            let order = guard.next;
            guard.next += 1;
            guard.order.insert(order, key.to_owned());
            guard.by_key.insert(
                key.to_owned(),
                Bucket {
                    tokens: self.spec.burst,
                    at_ms: now_ms,
                    order,
                },
            );
        }
        let Some(bucket) = guard.by_key.get_mut(key) else {
            return Take::Allowed;
        };
        let tokens =
            (bucket.tokens + (now_ms - bucket.at_ms).max(0.0) / 60_000.0 * self.spec.per_minute).min(self.spec.burst);
        bucket.at_ms = now_ms;
        if tokens >= 1.0 {
            bucket.tokens = tokens - 1.0;
            return Take::Allowed;
        }
        bucket.tokens = tokens;
        if self.spec.per_minute <= 0.0 {
            return Take::Limited { retry_after: 3600 };
        }
        let seconds = (1.0 - tokens) / (self.spec.per_minute / 60.0) - 1e-6;
        Take::Limited {
            retry_after: (seconds.ceil() as u64).max(1),
        }
    }

    fn sweep_locked(&self, guard: &mut Buckets, now_ms: f64) {
        let full: Vec<(String, u64)> = guard
            .by_key
            .iter()
            .filter(|(_, b)| self.refilled(b, now_ms) >= self.spec.burst)
            .map(|(k, b)| (k.clone(), b.order))
            .collect();
        for (key, order) in full {
            guard.by_key.remove(&key);
            guard.order.remove(&order);
        }
    }

    /// Forgets the buckets that refilled completely (every 60 s).
    pub fn sweep(&self) {
        self.sweep_at(self.now_ms());
    }

    pub fn sweep_at(&self, now_ms: f64) {
        let mut guard = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        self.sweep_locked(&mut guard, now_ms);
    }

    pub fn len(&self) -> usize {
        self.buckets.lock().unwrap_or_else(PoisonError::into_inner).by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_text() {
        assert_eq!(
            parse_bucket_spec("300,180", "R").unwrap(),
            BucketSpec {
                burst: 300.0,
                per_minute: 180.0
            }
        );
        assert_eq!(parse_bucket_spec(" 5 , 0.2 ", "R").unwrap().per_minute, 0.2);
        let shape = "R must look like \"<burst>,<per minute>\", for example \"300,180\"";
        for bad in ["", "5", "-1,2", "a,b", "5,1,2", "5,.2", "5,2.", "1e3,1"] {
            assert_eq!(parse_bucket_spec(bad, "R").unwrap_err(), shape, "{bad}");
        }
        assert_eq!(
            parse_bucket_spec("0,1", "R").unwrap_err(),
            "R: burst must be 1 to 1000000 and the rate at most 1000000 per minute"
        );
        assert!(parse_bucket_spec("1,1000001", "R").is_err());
    }

    #[test]
    fn burst_three_at_sixty() {
        let l = RateLimiter::new(BucketSpec {
            burst: 3.0,
            per_minute: 60.0,
        });
        for _ in 0..3 {
            assert_eq!(l.take_at("a", 0.0), Take::Allowed);
        }
        assert_eq!(l.take_at("a", 0.0), Take::Limited { retry_after: 1 });
        assert_eq!(l.take_at("a", 1000.0), Take::Allowed);
        assert!(matches!(l.take_at("a", 1000.0), Take::Limited { .. }));
        assert_eq!(l.take_at("a", 2500.0), Take::Allowed);
        assert_eq!(l.take_at("b", 2500.0), Take::Allowed);
    }

    #[test]
    fn slow_bucket_is_kept_until_full() {
        let l = RateLimiter::new(BucketSpec {
            burst: 1.0,
            per_minute: 0.2,
        });
        assert_eq!(l.take_at("a", 0.0), Take::Allowed);
        assert_eq!(l.take_at("a", 0.0), Take::Limited { retry_after: 300 });
        l.sweep_at(299_000.0);
        assert_eq!(l.len(), 1);
        l.sweep_at(300_100.0);
        assert_eq!(l.len(), 0);
    }

    #[test]
    fn never_refilling_bucket_says_an_hour() {
        let l = RateLimiter::new(BucketSpec {
            burst: 1.0,
            per_minute: 0.0,
        });
        assert_eq!(l.take_at("a", 0.0), Take::Allowed);
        assert_eq!(l.take_at("a", 10.0), Take::Limited { retry_after: 3600 });
    }

    #[test]
    fn token_bucket_of_the_parity_fixture() {
        let l = RateLimiter::new(BucketSpec {
            burst: 2.0,
            per_minute: 1.0,
        });
        assert_eq!(l.take_at("a", 0.0), Take::Allowed);
        assert_eq!(l.take_at("a", 5.0), Take::Allowed);
        assert_eq!(l.take_at("a", 10.0), Take::Limited { retry_after: 60 });
    }
}
