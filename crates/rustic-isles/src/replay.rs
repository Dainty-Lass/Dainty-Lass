// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The replay-aware clock and id source (core spec §9.1). A request's scope reaches its async work through a
//! task-local and the closures a store hands to blocking threads through [`ReplayScope::enter`].

use std::cell::RefCell;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

/// The header pinning a request's clock: Unix ms or any text `Date.parse` reads.
pub const REPLAY_NOW_HEADER: &str = "x-isles-replay-now-ms";
/// The header seeding a request's generated ids.
pub const REPLAY_SEED_HEADER: &str = "x-isles-replay-seed";

/// A request's pinned clock and id seed, set by the shadow replay harness (§9.1).
#[derive(Debug, Clone, Default)]
pub struct ReplayScope {
    now: Option<i64>,
    seed: Option<[u8; 32]>,
    issued: Arc<AtomicU64>,
}

tokio::task_local! {
    static TASK_SCOPE: ReplayScope;
}

thread_local! {
    static THREAD_SCOPE: RefCell<Option<ReplayScope>> = const { RefCell::new(None) };
}

// Process-wide on purpose: a test pins the clock for everything it drives
static TEST_PIN: Mutex<Option<i64>> = Mutex::new(None);

impl ReplayScope {
    /// `now`: the header's time; `seed`: the header's seed text (empty is none).
    pub fn new(now: Option<i64>, seed: Option<&str>) -> Self {
        let seed = seed.filter(|s| !s.is_empty()).map(|text| {
            let mut hasher = Sha256::new();
            hasher.update(b"isles-replay-seed:");
            hasher.update(text.as_bytes());
            hasher.finalize().into()
        });
        Self {
            now,
            seed,
            issued: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Reads the two headers (trimmed): an integer of ms or any text `Date.parse` reads, and a seed.
    pub fn from_headers(headers: &http::HeaderMap) -> Self {
        let text = |name: &str| {
            headers
                .get(name)
                .map(|v| String::from_utf8_lossy(v.as_bytes()).trim().to_owned())
        };
        let now = text(REPLAY_NOW_HEADER).and_then(|t| parse_now(&t));
        Self::new(now, text(REPLAY_SEED_HEADER).as_deref())
    }

    /// Whether the clock or the ids are pinned.
    pub fn is_active(&self) -> bool {
        self.now.is_some() || self.seed.is_some()
    }

    /// The pinned time, Unix ms.
    pub fn now(&self) -> Option<i64> {
        self.now
    }

    /// Runs `f` with this scope in force on the current thread (a blocking closure, say).
    pub fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        let previous = THREAD_SCOPE.with(|cell| cell.replace(Some(self.clone())));
        struct Restore(Option<ReplayScope>);
        impl Drop for Restore {
            fn drop(&mut self) {
                let previous = self.0.take();
                THREAD_SCOPE.with(|cell| *cell.borrow_mut() = previous);
            }
        }
        let _restore = Restore(previous);
        f()
    }

    fn next_seeded(&self) -> Option<String> {
        let key = self.seed?;
        let n = self.issued.fetch_add(1, Ordering::Relaxed);
        let mut hasher = Sha256::new();
        hasher.update(key);
        hasher.update(n.to_le_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        Some(format_uuid(bytes))
    }
}

fn parse_now(text: &str) -> Option<i64> {
    if text.is_empty() {
        return None;
    }
    let digits = text.strip_prefix('-').unwrap_or(text);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        return text.parse().ok();
    }
    service_kit::js::date_parse(text)
}

/// Runs `future` with `scope` in force for every await inside it.
pub fn with_replay_scope<F: Future>(scope: ReplayScope, future: F) -> impl Future<Output = F::Output> {
    TASK_SCOPE.scope(scope, future)
}

/// The current task's or thread's scope, none when nothing is pinned.
pub fn current_replay_scope() -> Option<ReplayScope> {
    let task = TASK_SCOPE.try_with(Clone::clone).ok();
    task.or_else(|| THREAD_SCOPE.with(|cell| cell.borrow().clone()))
        .filter(ReplayScope::is_active)
}

fn test_pin() -> Option<i64> {
    *TEST_PIN.lock().unwrap_or_else(PoisonError::into_inner)
}

#[doc(hidden)]
pub fn pin_now_for_tests(now: Option<i64>) {
    *TEST_PIN.lock().unwrap_or_else(PoisonError::into_inner) = now;
}

/// Unix ms: the scope's time, else a test pin, else the system's.
pub fn now_ms() -> i64 {
    replay_now_ms()
        .or_else(test_pin)
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
}

/// The scope's time only, so a pinned test does not backdate live token expiries.
pub fn replay_now_ms() -> Option<i64> {
    current_replay_scope().and_then(|s| s.now)
}

/// Whether the clock is pinned by a replay header or a test.
pub fn is_replay_pinned() -> bool {
    replay_now_ms().is_some() || test_pin().is_some()
}

/// A UUID v4 text: the scope's next seeded one, else random.
pub fn next_uuid() -> String {
    current_replay_scope()
        .and_then(|s| s.next_seeded())
        .unwrap_or_else(random_uuid)
}

/// A random v4, never seeded (the request id fallback).
pub fn random_uuid() -> String {
    format_uuid(rand::random())
}

fn format_uuid(mut b: [u8; 16]) -> String {
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_v4(id: &str) -> bool {
        id.len() == 36 && id.as_bytes()[14] == b'4' && matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b')
    }

    #[tokio::test]
    async fn seeded_ids_repeat_per_request() {
        let run = || with_replay_scope(ReplayScope::new(None, Some("s1")), async { (next_uuid(), next_uuid()) });
        let (a, b) = run().await;
        assert_eq!(run().await, (a.clone(), b.clone()));
        assert_ne!(a, b);
        assert!(is_v4(&a) && is_v4(&b));
        let other = with_replay_scope(ReplayScope::new(None, Some("s2")), async { next_uuid() }).await;
        assert_ne!(other, a);
        assert!(is_v4(&next_uuid()));
    }

    #[tokio::test]
    async fn blocking_closures_see_the_scope() {
        let scope = ReplayScope::new(Some(42), None);
        let inside = with_replay_scope(scope, async {
            let scope = current_replay_scope().unwrap();
            tokio::task::spawn_blocking(move || scope.enter(now_ms)).await.unwrap()
        })
        .await;
        assert_eq!(inside, 42);
        assert!(current_replay_scope().is_none());
    }

    #[test]
    fn headers_parse() {
        let mut headers = http::HeaderMap::new();
        headers.insert(REPLAY_NOW_HEADER, " 1000 ".parse().unwrap());
        headers.insert(REPLAY_SEED_HEADER, "   ".parse().unwrap());
        let scope = ReplayScope::from_headers(&headers);
        assert_eq!(scope.now(), Some(1000));
        assert!(scope.seed.is_none());
        headers.insert(REPLAY_NOW_HEADER, "2026-10-22T17:00:00.000Z".parse().unwrap());
        assert_eq!(ReplayScope::from_headers(&headers).now(), Some(1_792_688_400_000));
        headers.insert(REPLAY_NOW_HEADER, "soon".parse().unwrap());
        assert!(!ReplayScope::from_headers(&headers).is_active());
    }
}
