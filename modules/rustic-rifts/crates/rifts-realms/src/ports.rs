//! What realms take from the server around them that RF2's seam doesn't carry yet: time, ids, the rate limiter, the
//! accounts' names, a log and the event sink. Isles supplies the real ones (RU4); tests use the plain ones here.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

use rifts_seam::{BoxFuture, UnixMillis};

use crate::realm::RealmEvent;

pub trait Clock: Send + Sync {
    fn now(&self) -> UnixMillis;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> UnixMillis {
        crate::time::now()
    }
}

/// A clock a test sets by hand.
#[derive(Debug)]
pub struct ManualClock(AtomicI64);

impl ManualClock {
    pub fn at(now: UnixMillis) -> Self {
        Self(AtomicI64::new(now.0))
    }

    pub fn set(&self, now: UnixMillis) {
        self.0.store(now.0, Ordering::Relaxed);
    }

    pub fn advance(&self, millis: i64) {
        self.0.fetch_add(millis, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> UnixMillis {
        UnixMillis(self.0.load(Ordering::Relaxed))
    }
}

/// `IsRateLimited(key, max, windowMs)`: isles' per-key budget.
pub trait RateLimiter: Send + Sync {
    fn is_limited(&self, key: &str, max: u32, window_ms: i64) -> bool;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoRateLimit;

impl RateLimiter for NoRateLimit {
    fn is_limited(&self, _: &str, _: u32, _: i64) -> bool {
        false
    }
}

/// Fresh realm ids and invite codes (`crypto.randomUUID`, `GenerateInviteCode`).
pub trait IdSource: Send + Sync {
    fn realm_id(&self) -> String;
    fn invite_code(&self) -> String;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RandomIds;

impl IdSource for RandomIds {
    fn realm_id(&self) -> String {
        crate::ids::random_uuid()
    }

    fn invite_code(&self) -> String {
        crate::realm::generate_invite_code()
    }
}

/// The accounts' display names and usernames (isles' `accounts` collection, `findByDisplayNameNormalized`).
pub trait Accounts: Send + Sync {
    /// Best effort: an unknown account has no entry.
    fn display_names(&self, account_ids: Vec<String>) -> BoxFuture<'_, Result<BTreeMap<String, String>, String>>;
    /// The account whose launcher username this is (trimmed text, matched case-insensitively), if any.
    fn resolve_username(&self, username: String) -> BoxFuture<'_, Result<Option<String>, String>>;
}

/// Names from a table, for tests.
#[derive(Debug, Clone, Default)]
pub struct FixedAccounts {
    pub names: BTreeMap<String, String>,
    /// Lower-case username to account id.
    pub usernames: BTreeMap<String, String>,
}

impl Accounts for FixedAccounts {
    fn display_names(&self, account_ids: Vec<String>) -> BoxFuture<'_, Result<BTreeMap<String, String>, String>> {
        let names = account_ids
            .into_iter()
            .filter_map(|id| self.names.get(&id).map(|name| (id, name.clone())))
            .collect();
        Box::pin(async move { Ok(names) })
    }

    fn resolve_username(&self, username: String) -> BoxFuture<'_, Result<Option<String>, String>> {
        let found = self.usernames.get(&username.to_lowercase()).cloned();
        Box::pin(async move { Ok(found) })
    }
}

/// What the directory tells the p2p side (`RealmEvent`); a failure is logged, never the request's.
pub trait RealmEventSink: Send + Sync {
    fn emit(&self, event: RealmEvent) -> BoxFuture<'_, Result<(), String>>;
}

/// Keeps every event, for tests.
#[derive(Debug, Default)]
pub struct RecordedEvents(Mutex<Vec<RealmEvent>>);

impl RecordedEvents {
    pub fn take(&self) -> Vec<RealmEvent> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    pub fn all(&self) -> Vec<RealmEvent> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}

impl RealmEventSink for RecordedEvents {
    fn emit(&self, event: RealmEvent) -> BoxFuture<'_, Result<(), String>> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(event);
        Box::pin(async { Ok(()) })
    }
}

/// Realms' log lines; nothing logged carries a secret or an address.
pub trait RealmsLog: Send + Sync {
    fn info(&self, line: &str);
    fn warn(&self, line: &str);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StderrLog;

impl RealmsLog for StderrLog {
    fn info(&self, line: &str) {
        eprintln!("info {line}");
    }

    fn warn(&self, line: &str) {
        eprintln!("warn {line}");
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct QuietLog;

impl RealmsLog for QuietLog {
    fn info(&self, _: &str) {}
    fn warn(&self, _: &str) {}
}

/// A healthy Launcher Guard session (`launcherGuard.findHealthyForAccount`), which isles keeps (RF1, decision 1).
#[derive(Debug, Clone, PartialEq)]
pub struct HealthyGuard {
    pub role: crate::lifecycle::GuardRole,
    pub guard_session_id: String,
    pub manifest_sequence: i64,
    pub last_sequence: i64,
    pub risk_score: f64,
}

pub trait GuardSessions: Send + Sync {
    fn find_healthy(
        &self,
        account: String,
        guard_session_id: String,
        role: crate::lifecycle::GuardRole,
    ) -> BoxFuture<'_, Result<Option<HealthyGuard>, String>>;
}

/// No Launcher Guard: nothing verifies (observe mode refuses nothing for it).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoGuardSessions;

impl GuardSessions for NoGuardSessions {
    fn find_healthy(
        &self,
        _: String,
        _: String,
        _: crate::lifecycle::GuardRole,
    ) -> BoxFuture<'_, Result<Option<HealthyGuard>, String>> {
        Box::pin(async { Ok(None) })
    }
}

/// What realms reads of isles' operations policy (`GetP2POperationsPolicy`) and its Guard mode.
pub trait OperationsView: Send + Sync {
    fn emergency_stop(&self) -> BoxFuture<'_, Result<bool, String>>;
    /// `coopTransport === "ICE"` (the default).
    fn ice_transport(&self) -> bool;
    /// `GuardVerdictRefuses`: enforce mode with the Guard rule on.
    fn guard_refuses(&self) -> bool;
}

/// The defaults: no emergency stop, ICE, observe mode.
#[derive(Debug, Default)]
pub struct DefaultOperations {
    pub stopped: std::sync::atomic::AtomicBool,
}

impl OperationsView for DefaultOperations {
    fn emergency_stop(&self) -> BoxFuture<'_, Result<bool, String>> {
        let stopped = self.stopped.load(Ordering::Relaxed);
        Box::pin(async move { Ok(stopped) })
    }

    fn ice_transport(&self) -> bool {
        true
    }

    fn guard_refuses(&self) -> bool {
        false
    }
}
