//! What Guard needs from the rest of the server that RF2's seam does not carry yet: the hunt records realms keep
//! (RF5), the operations policy and admin audit isles keeps (RU4), a clock and a log. Each is a trait here so the
//! in-memory versions run the tests; the real ones are RF5's and RU4's to supply.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use rifts_seam::{AccountId, BoxFuture, HubKind, Json, RealmId, SeamError, SessionId, StoreError, UnixMillis};

use crate::operations::RewardRollout;

/// Guard's log lines; isles routes them to its logger. Nothing logged carries a secret: capability values and the
/// shadow key never reach a line.
pub trait GuardLog: Send + Sync {
    fn info(&self, line: &str);
    fn warn(&self, line: &str);
    fn error(&self, line: &str);
}

/// Writes to standard error, until isles hands Guard its logger.
#[derive(Debug, Clone, Copy, Default)]
pub struct StderrLog;

impl GuardLog for StderrLog {
    fn info(&self, line: &str) {
        eprintln!("info {line}");
    }
    fn warn(&self, line: &str) {
        eprintln!("warn {line}");
    }
    fn error(&self, line: &str) {
        eprintln!("error {line}");
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Keeps every line as `level message`, for tests.
#[derive(Debug, Default)]
pub struct MemoryLog {
    lines: Mutex<Vec<String>>,
}

impl MemoryLog {
    pub fn lines(&self) -> Vec<String> {
        lock(&self.lines).clone()
    }
}

impl GuardLog for MemoryLog {
    fn info(&self, line: &str) {
        lock(&self.lines).push(format!("info {line}"));
    }
    fn warn(&self, line: &str) {
        lock(&self.lines).push(format!("warn {line}"));
    }
    fn error(&self, line: &str) {
        lock(&self.lines).push(format!("error {line}"));
    }
}

/// Time and fresh ids (`Date.now()`, `crypto.randomUUID()`).
pub trait Clock: Send + Sync {
    fn now(&self) -> UnixMillis;
    /// A version 4 UUID for request, audit and alert ids. These name records, they guard nothing.
    fn uuid(&self) -> String;
}

#[derive(Debug, Default)]
pub struct SystemClock {
    counter: AtomicU64,
}

impl Clock for SystemClock {
    fn now(&self) -> UnixMillis {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX));
        UnixMillis(millis)
    }

    fn uuid(&self) -> String {
        // std's RandomState keys are random per process and change per instance; no RNG crate in the workspace yet
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let mut bytes = [0u8; 16];
        for half in bytes.as_chunks_mut::<8>().0 {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u64(self.counter.fetch_add(1, Ordering::Relaxed));
            hasher.write_u128(nanos);
            half.copy_from_slice(&hasher.finish().to_le_bytes());
        }
        format_uuid(bytes)
    }
}

pub(crate) fn format_uuid(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = crate::digest::hex(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// A clock the test moves by hand; ids count up.
#[derive(Debug)]
pub struct ManualClock {
    now: AtomicU64,
    ids: AtomicU64,
}

impl ManualClock {
    pub fn at(now: UnixMillis) -> Self {
        Self {
            now: AtomicU64::new(u64::try_from(now.0).unwrap_or_default()),
            ids: AtomicU64::new(0),
        }
    }

    pub fn advance(&self, millis: u64) {
        self.now.fetch_add(millis, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> UnixMillis {
        UnixMillis(i64::try_from(self.now.load(Ordering::Relaxed)).unwrap_or(i64::MAX))
    }

    fn uuid(&self) -> String {
        let id = self.ids.fetch_add(1, Ordering::Relaxed) + 1;
        let mut bytes = [0u8; 16];
        bytes[8..].copy_from_slice(&id.to_be_bytes());
        format_uuid(bytes)
    }
}

/// `new Date(ms).toISOString()`.
pub fn iso_time(at: UnixMillis) -> String {
    let millis = at.0.rem_euclid(1000);
    let seconds = at.0.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}

/// A roster entry of a hunt (`p2pHuntSessions.roster`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub account: AccountId,
    /// `admissionState === "ADMITTED"`.
    pub admitted: bool,
}

/// What Guard reads of a hunt or hub session (`P2PHuntSessionRecord`), from realms' table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntView {
    pub session: SessionId,
    pub generation: u64,
    /// Node's state names (`RUNNING`, `DRAINING`, `SUSPECT`...).
    pub state: String,
    pub pre_suspect_state: Option<String>,
    pub host: Option<AccountId>,
    pub realm: Option<RealmId>,
    /// Set for a realm hub (`sessionKind: "REALM_HUB"`), with its kind.
    pub hub: Option<HubKind>,
    pub roster: Vec<RosterEntry>,
    /// `frozenLoadouts`: the character frozen for each member.
    pub frozen_characters: Vec<(AccountId, String)>,
}

impl HuntView {
    pub fn is_hub(&self) -> bool {
        self.hub.is_some()
    }

    pub fn session_kind(&self) -> Option<&'static str> {
        self.hub.map(|_| "REALM_HUB")
    }

    pub fn hub_kind(&self) -> Option<&'static str> {
        self.hub.map(hub_kind_name)
    }
}

pub const fn hub_kind_name(kind: HubKind) -> &'static str {
    match kind {
        HubKind::Ramsgate => "RAMSGATE",
        HubKind::Training => "TRAINING",
    }
}

/// `p2pHostCapabilities` as the rate-limit bucket reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityView {
    pub capability_id: String,
    pub expires_at: UnixMillis,
    pub revoked: bool,
}

/// Realms' hunt records, read by Guard (RF5 implements it on its store).
pub trait HuntSessions: Send + Sync {
    /// `p2pHunts.findHunt`.
    fn hunt(&self, session: &SessionId) -> Result<Option<HuntView>, StoreError>;
    /// `ListHubPresentAccounts`: the accounts ADMITTED in a hub generation (realms caches it for a second).
    fn hub_present_accounts(&self, session: &SessionId, generation: u64) -> Result<Vec<AccountId>, StoreError>;
    /// `findHostCapabilityByDigest`: by the SHA-256 hex of the secret.
    fn capability_by_digest(&self, digest: &str) -> Result<Option<CapabilityView>, StoreError>;
}

/// The operations policy as `GetP2POperationsPolicy` returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicySnapshot {
    pub version: u64,
    pub reward_rollout: RewardRollout,
    /// The whole normalized policy, as `res.json` writes it.
    pub json: Json,
}

/// One `adminAudit` row (`repos.admin.appendAudit`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub id: String,
    pub actor: String,
    pub action: String,
    pub old_state: Json,
    pub new_state: Json,
    pub reason: String,
    pub ip: String,
    pub request_id: String,
    pub created_at: String,
}

/// The `operationsPolicy` document and the admin audit, which isles owns. Node writes the policy, the audit row and
/// the alert in one transaction; until `store` has one, the in-memory version is serial.
pub trait OperationsPolicyPort: Send + Sync {
    fn policy(&self) -> BoxFuture<'_, Result<PolicySnapshot, SeamError>>;
    /// `SetRewardRollout`: writes only at `expected_version` and bumps it; `None` on a conflict.
    fn set_reward_rollout(
        &self,
        rollout: RewardRollout,
        actor: String,
        expected_version: u64,
    ) -> BoxFuture<'_, Result<Option<PolicySnapshot>, SeamError>>;
    fn append_audit(&self, entry: AuditEntry) -> BoxFuture<'_, Result<(), SeamError>>;
}

/// An operations policy and audit in memory, rendered with the module's fields only.
#[derive(Debug, Default)]
pub struct MemoryOperations {
    state: Mutex<(u64, RewardRollout, Vec<AuditEntry>)>,
}

impl MemoryOperations {
    pub fn audit(&self) -> Vec<AuditEntry> {
        lock(&self.state).2.clone()
    }

    fn snapshot(version: u64, rollout: RewardRollout) -> PolicySnapshot {
        PolicySnapshot {
            version,
            reward_rollout: rollout,
            json: Json(format!(
                r#"{{"version":{version},"p2pEmergencyStop":false,"rewardRollout":{},"coopTransport":"ICE"}}"#,
                rollout.to_json().0
            )),
        }
    }
}

fn ready<T: Send + 'static>(value: T) -> BoxFuture<'static, Result<T, SeamError>> {
    Box::pin(async move { Ok(value) })
}

impl OperationsPolicyPort for MemoryOperations {
    fn policy(&self) -> BoxFuture<'_, Result<PolicySnapshot, SeamError>> {
        let (version, rollout, _) = &*lock(&self.state);
        ready(Self::snapshot(*version, *rollout))
    }

    fn set_reward_rollout(
        &self,
        rollout: RewardRollout,
        _: String,
        expected_version: u64,
    ) -> BoxFuture<'_, Result<Option<PolicySnapshot>, SeamError>> {
        let mut state = lock(&self.state);
        if state.0 != expected_version {
            return ready(None);
        }
        state.0 += 1;
        state.1 = rollout;
        ready(Some(Self::snapshot(state.0, rollout)))
    }

    fn append_audit(&self, entry: AuditEntry) -> BoxFuture<'_, Result<(), SeamError>> {
        lock(&self.state).2.push(entry);
        ready(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_time_matches_to_iso_string() {
        assert_eq!(iso_time(UnixMillis(0)), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_time(UnixMillis(1_792_000_000_123)), "2026-10-14T17:46:40.123Z");
        assert_eq!(iso_time(UnixMillis(951_782_400_000)), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso_time(UnixMillis(-1)), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn uuids_are_version_4() {
        let clock = SystemClock::default();
        let (first, second) = (clock.uuid(), clock.uuid());
        assert_ne!(first, second);
        for id in [first, second, ManualClock::at(UnixMillis(0)).uuid()] {
            assert_eq!(id.len(), 36);
            assert_eq!(&id[14..15], "4");
            assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"), "{id}");
        }
    }
}
