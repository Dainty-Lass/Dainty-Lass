//! What the shipper and a restore need from a bucket: the server only [`Bucket::put`]s; the owner's
//! credentials also [`BucketRead::list`] and [`BucketRead::get`]. [`MemoryBucket`] keeps every version
//! of every key, as an object-locked bucket does, for the tests.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::Error;

/// One object as a listing names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectInfo {
    pub key: String,
    /// Unix milliseconds by the bucket's clock: when it received the current version. The server can't
    /// set it, which is what lets a restore distrust objects written after the moment it restores to.
    pub last_modified: i64,
    pub size: u64,
}

/// Write-only: what the server's credentials allow.
pub trait Bucket: Send + Sync {
    /// Stores `body` under `key`. Object lock keeps an earlier version of the key if there was one.
    fn put(
        &self,
        key: &str,
        body: Vec<u8>,
        content_type: &'static str,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}

/// The owner's credentials: list and read.
pub trait BucketRead: Send + Sync {
    /// Every key under `prefix` after `start_after`, in key order, across pages.
    fn list(
        &self,
        prefix: &str,
        start_after: Option<&str>,
    ) -> impl Future<Output = Result<Vec<ObjectInfo>, Error>> + Send;
    /// The current version of `key`.
    fn get(&self, key: &str) -> impl Future<Output = Result<Vec<u8>, Error>> + Send;
}

impl<B: Bucket> Bucket for &B {
    fn put(
        &self,
        key: &str,
        body: Vec<u8>,
        content_type: &'static str,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        (**self).put(key, body, content_type)
    }
}

impl<B: BucketRead> BucketRead for &B {
    fn list(
        &self,
        prefix: &str,
        start_after: Option<&str>,
    ) -> impl Future<Output = Result<Vec<ObjectInfo>, Error>> + Send {
        (**self).list(prefix, start_after)
    }

    fn get(&self, key: &str) -> impl Future<Output = Result<Vec<u8>, Error>> + Send {
        (**self).get(key)
    }
}

type Versions = BTreeMap<String, Vec<(Vec<u8>, i64)>>;

/// An in-memory bucket with object lock: a put never replaces a version, a get returns the newest.
/// Clones share the objects.
#[derive(Clone)]
pub struct MemoryBucket {
    objects: Arc<Mutex<Versions>>,
    clock: fn() -> i64,
    offline: Arc<AtomicBool>,
}

impl MemoryBucket {
    /// `clock` stamps each put's `last_modified`, as the bucket's own clock would.
    pub fn new(clock: fn() -> i64) -> Self {
        Self {
            objects: Arc::new(Mutex::new(BTreeMap::new())),
            clock,
            offline: Arc::new(AtomicBool::new(false)),
        }
    }

    /// While set, every call fails as an unreachable bucket would.
    pub fn set_offline(&self, offline: bool) {
        self.offline.store(offline, Ordering::SeqCst);
    }

    /// Every version of `key`, oldest first.
    pub fn versions(&self, key: &str) -> Vec<Vec<u8>> {
        self.lock()
            .get(key)
            .map(|v| v.iter().map(|(body, _)| body.clone()).collect())
            .unwrap_or_default()
    }

    /// Stores a version with a chosen `last_modified`: what an object written later looks like.
    pub fn put_at(&self, key: &str, body: Vec<u8>, last_modified: i64) {
        self.lock()
            .entry(key.to_owned())
            .or_default()
            .push((body, last_modified));
    }

    /// Drops `key`, as if it never arrived: no locked bucket allows it, a test of a hole needs it.
    pub fn forget(&self, key: &str) {
        self.lock().remove(key);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Versions> {
        self.objects.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn check(&self) -> Result<(), Error> {
        if self.offline.load(Ordering::SeqCst) {
            Err(Error::Bucket("the bucket can't be reached".into()))
        } else {
            Ok(())
        }
    }
}

impl Bucket for MemoryBucket {
    async fn put(&self, key: &str, body: Vec<u8>, _content_type: &'static str) -> Result<(), Error> {
        self.check()?;
        let now = (self.clock)();
        self.put_at(key, body, now);
        Ok(())
    }
}

impl BucketRead for MemoryBucket {
    async fn list(&self, prefix: &str, start_after: Option<&str>) -> Result<Vec<ObjectInfo>, Error> {
        self.check()?;
        Ok(self
            .lock()
            .iter()
            .filter(|(key, _)| key.starts_with(prefix) && start_after.is_none_or(|s| key.as_str() > s))
            .filter_map(|(key, versions)| {
                versions.last().map(|(body, at)| ObjectInfo {
                    key: key.clone(),
                    last_modified: *at,
                    size: body.len() as u64,
                })
            })
            .collect())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        self.check()?;
        self.lock()
            .get(key)
            .and_then(|v| v.last())
            .map(|(body, _)| body.clone())
            .ok_or_else(|| Error::Bucket(format!("no object {key}")))
    }
}
