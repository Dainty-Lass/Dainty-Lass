// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Who a download is for: the account key checked against the backend's `GetUserInfo`, and the answer cached
//! under the key's SHA-256 (the key itself is never stored or logged).

use std::collections::{BTreeMap, HashMap};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::FutureExt as _;
use futures_util::future::{BoxFuture, Shared};
use serde_json::Value;

use crate::input::{KEY_HEADER, is_plausible_key};
use crate::{Clock, client, sha256_hex};

/// How long one backend lookup may take.
pub const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest backend answer read, in bytes.
pub const MAX_ANSWER: usize = 1 << 20;
/// How long a refused key stays cached, in ms.
pub const REFUSED_TTL_MS: i64 = 30_000;
/// Most keys cached at once; the oldest goes first.
pub const MAX_ENTRIES: usize = 2000;

/// The account a key belongs to, from the backend's `GetUserInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// Account id.
    pub id: String,
    /// Display name.
    pub username: String,
    /// Whether the backend reports `IsAdmin: true`.
    pub admin: bool,
}

/// What a key check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// The key is valid for this account.
    Account(Account),
    /// The backend said no (401 or 403), or the key could never be one
    Refused,
    /// The backend could not answer; the reason goes to the log
    Unavailable(String),
}

/// One backend lookup of a key.
pub type Lookup = Arc<dyn Fn(String) -> BoxFuture<'static, Check> + Send + Sync>;

struct Entry {
    expires: i64,
    seq: u64,
    check: Check,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<String, Entry>,
    /// Insertion order, oldest first
    order: BTreeMap<u64, String>,
    next: u64,
    in_flight: HashMap<String, Shared<BoxFuture<'static, Check>>>,
}

impl Cache {
    fn insert(&mut self, hash: String, check: Check, expires: i64) {
        if let Some(old) = self.entries.remove(&hash) {
            self.order.remove(&old.seq);
        }
        let seq = self.next;
        self.next += 1;
        self.order.insert(seq, hash.clone());
        self.entries.insert(hash, Entry { expires, seq, check });
        while self.entries.len() > MAX_ENTRIES {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    fn remove(&mut self, hash: &str) {
        if let Some(old) = self.entries.remove(hash) {
            self.order.remove(&old.seq);
        }
    }
}

/// Checks account keys, caching answers under the key's SHA-256 and sharing concurrent lookups of one key.
#[derive(Clone)]
pub struct Auth {
    lookup: Lookup,
    clock: Clock,
    account_ttl_ms: i64,
    cache: Arc<Mutex<Cache>>,
}

impl Auth {
    /// Checks keys against the backend at `backend_url`.
    pub fn backend(backend_url: &str, cache_seconds: u64, clock: Clock) -> Self {
        let url = format!("{}/dainty/api/GetUserInfo", backend_url.trim_end_matches('/'));
        let lookup: Lookup = Arc::new(move |key| {
            let url = url.clone();
            Box::pin(async move { backend_lookup(&url, &key).await })
        });
        Self::new(lookup, cache_seconds, clock)
    }

    /// Checks keys with `lookup`; accounts are cached for `cache_seconds` (0 turns that off).
    pub fn new(lookup: Lookup, cache_seconds: u64, clock: Clock) -> Self {
        Self {
            lookup,
            clock,
            account_ttl_ms: i64::try_from(cache_seconds.saturating_mul(1000)).unwrap_or(i64::MAX),
            cache: Arc::default(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Checks `key`, from the cache when it holds a live answer; only accounts and refusals are cached.
    pub async fn check(&self, key: &str) -> Check {
        if !is_plausible_key(key) {
            return Check::Refused;
        }
        let hash = sha256_hex(key.as_bytes());
        let now = (self.clock)();
        let shared = {
            let mut cache = self.lock();
            match cache.entries.get(&hash) {
                Some(entry) if entry.expires > now => return entry.check.clone(),
                Some(_) => cache.remove(&hash),
                None => {}
            }
            // Concurrent checks of one key share a lookup; its task alone stores the answer
            match cache.in_flight.get(&hash) {
                Some(shared) => shared.clone(),
                None => {
                    let task = tokio::spawn(self.clone().lookup_and_store(key.to_owned(), hash.clone()));
                    let shared = async move { task.await.unwrap_or_else(|_| failed()) }.boxed().shared();
                    cache.in_flight.insert(hash, shared.clone());
                    shared
                }
            }
        };
        shared.await
    }

    async fn lookup_and_store(self, key: String, hash: String) -> Check {
        let check = AssertUnwindSafe((self.lookup)(key))
            .catch_unwind()
            .await
            .unwrap_or_else(|_| failed());
        let now = (self.clock)();
        let mut cache = self.lock();
        cache.in_flight.remove(&hash);
        match &check {
            Check::Account(_) if self.account_ttl_ms > 0 => {
                cache.insert(hash, check.clone(), now.saturating_add(self.account_ttl_ms));
            }
            Check::Refused => cache.insert(hash, check.clone(), now + REFUSED_TTL_MS),
            _ => {}
        }
        drop(cache);
        check
    }

    /// The cache's keys (key hashes), for tests.
    #[doc(hidden)]
    pub fn cached(&self) -> Vec<String> {
        self.lock().entries.keys().cloned().collect()
    }
}

fn failed() -> Check {
    Check::Unavailable("lookup failed".to_owned())
}

/// `GET <backend>/dainty/api/GetUserInfo` with the key, mapped to a [`Check`].
pub async fn backend_lookup(url: &str, key: &str) -> Check {
    let headers = [(KEY_HEADER, key), ("accept", "application/json")];
    let reply = match client::get(url, &headers, LOOKUP_TIMEOUT, MAX_ANSWER).await {
        Ok(reply) => reply,
        Err(error) => return Check::Unavailable(format!("backend unreachable: {error}")),
    };
    match reply.status {
        200 => {}
        401 | 403 => return Check::Refused,
        status => return Check::Unavailable(format!("backend answered {status}")),
    }
    let Some(Value::Object(answer)) = reply
        .body
        .ok()
        .and_then(|body| serde_json::from_slice::<Value>(&body).ok())
    else {
        return Check::Unavailable("backend answered with bad JSON".to_owned());
    };
    let id = match answer.get("UserId") {
        Some(Value::String(id)) if !id.is_empty() => id.clone(),
        _ => return Check::Unavailable("backend answer has no UserId".to_owned()),
    };
    Check::Account(Account {
        id,
        username: answer
            .get("Username")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        admin: answer.get("IsAdmin") == Some(&Value::Bool(true)),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    use super::*;

    fn account(id: &str) -> Check {
        Check::Account(Account {
            id: id.into(),
            username: "u".into(),
            admin: false,
        })
    }

    struct Fake {
        calls: Arc<AtomicUsize>,
        now: Arc<AtomicI64>,
        auth: Auth,
    }

    fn fake(ttl: u64, answer: impl Fn(&str) -> Check + Send + Sync + 'static) -> Fake {
        fake_with(ttl, Duration::from_millis(50), answer)
    }

    fn fake_with(ttl: u64, delay: Duration, answer: impl Fn(&str) -> Check + Send + Sync + 'static) -> Fake {
        let calls = Arc::new(AtomicUsize::new(0));
        let now = Arc::new(AtomicI64::new(1_000_000));
        let answer = Arc::new(answer);
        let counted = calls.clone();
        let lookup: Lookup = Arc::new(move |key| {
            counted.fetch_add(1, Ordering::SeqCst);
            let answer = answer.clone();
            Box::pin(async move {
                tokio::time::sleep(delay).await;
                answer(&key)
            })
        });
        let clock_now = now.clone();
        let auth = Auth::new(lookup, ttl, Arc::new(move || clock_now.load(Ordering::SeqCst)));
        Fake { calls, now, auth }
    }

    #[tokio::test]
    async fn one_lookup_then_cached_under_the_hash_until_the_ttl() {
        let f = fake(300, |_| account("a"));
        assert_eq!(f.auth.check("DLK_key").await, account("a"));
        assert_eq!(f.auth.check("DLK_key").await, account("a"));
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        let cached = f.auth.cached();
        assert_eq!(cached, [sha256_hex(b"DLK_key")]);
        assert!(!cached.iter().any(|k| k.contains("DLK_key")));
        f.now.fetch_add(299_999, Ordering::SeqCst);
        f.auth.check("DLK_key").await;
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        f.now.fetch_add(1, Ordering::SeqCst);
        f.auth.check("DLK_key").await;
        assert_eq!(f.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn zero_ttl_is_never_served_from_the_cache() {
        let f = fake(0, |_| account("a"));
        f.auth.check("k").await;
        f.auth.check("k").await;
        assert_eq!(f.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn unavailable_is_not_cached_and_refused_lives_30s() {
        let f = fake(300, |key| {
            if key == "down" {
                Check::Unavailable("backend answered 500".into())
            } else {
                Check::Refused
            }
        });
        f.auth.check("down").await;
        f.auth.check("down").await;
        assert_eq!(f.calls.load(Ordering::SeqCst), 2);
        assert_eq!(f.auth.check("nope").await, Check::Refused);
        f.now.fetch_add(29_999, Ordering::SeqCst);
        f.auth.check("nope").await;
        assert_eq!(f.calls.load(Ordering::SeqCst), 3);
        f.now.fetch_add(1, Ordering::SeqCst);
        f.auth.check("nope").await;
        assert_eq!(f.calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn implausible_keys_never_reach_the_backend() {
        let f = fake(300, |_| account("a"));
        for key in [String::new(), "a b".into(), "a\nb".into(), "k".repeat(257), "ä".into()] {
            assert_eq!(f.auth.check(&key).await, Check::Refused);
        }
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn concurrent_checks_share_one_lookup() {
        let f = fake(300, |_| account("a"));
        let (a, b, c) = tokio::join!(f.auth.check("k"), f.auth.check("k"), f.auth.check("k"));
        assert_eq!([a, b, c], [account("a"), account("a"), account("a")]);
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_panicking_lookup_is_unavailable_and_not_cached() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let lookup: Lookup = Arc::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { panic!("boom") })
        });
        let auth = Auth::new(lookup, 300, Arc::new(|| 0));
        assert_eq!(auth.check("k").await, Check::Unavailable("lookup failed".into()));
        assert_eq!(auth.check("k").await, Check::Unavailable("lookup failed".into()));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(auth.cached().is_empty());
    }

    #[tokio::test]
    async fn at_most_max_entries_oldest_dropped() {
        let f = fake_with(300, Duration::ZERO, account);
        for i in 0..=MAX_ENTRIES {
            f.auth.check(&format!("k{i}")).await;
        }
        let cached = f.auth.cached();
        assert_eq!(cached.len(), MAX_ENTRIES);
        assert!(!cached.contains(&sha256_hex(b"k0")));
        assert!(cached.contains(&sha256_hex(b"k1")));
    }

    async fn backend(status: u16, body: &'static str) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                assert!(request.starts_with("get /dainty/api/getuserinfo "), "{request}");
                assert!(request.contains("x-dainty-user-api-key: dlk_k"));
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        format!("http://{addr}/dainty/api/GetUserInfo")
    }

    #[tokio::test]
    async fn the_backend_mapping() {
        let cases: [(u16, &str, Check); 8] = [
            (
                200,
                r#"{"UserId":"u1","Username":"Ann","IsAdmin":true}"#,
                Check::Account(Account {
                    id: "u1".into(),
                    username: "Ann".into(),
                    admin: true,
                }),
            ),
            (
                200,
                r#"{"UserId":"u1","Username":5,"IsAdmin":"true"}"#,
                Check::Account(Account {
                    id: "u1".into(),
                    username: String::new(),
                    admin: false,
                }),
            ),
            (401, "{}", Check::Refused),
            (403, "{}", Check::Refused),
            (500, "{}", Check::Unavailable("backend answered 500".into())),
            (200, "nope", Check::Unavailable("backend answered with bad JSON".into())),
            (
                200,
                r#"{"UserId":""}"#,
                Check::Unavailable("backend answer has no UserId".into()),
            ),
            (
                200,
                r#"{"UserId":5}"#,
                Check::Unavailable("backend answer has no UserId".into()),
            ),
        ];
        for (status, body, expected) in cases {
            let url = backend(status, body).await;
            assert_eq!(backend_lookup(&url, "DLK_k").await, expected, "{status} {body}");
        }
        let free = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/dainty/api/GetUserInfo", free.local_addr().unwrap());
        drop(free);
        let Check::Unavailable(reason) = backend_lookup(&url, "DLK_k").await else {
            panic!("a free port answered")
        };
        assert!(reason.starts_with("backend unreachable: fetch failed: "), "{reason}");
    }
}
