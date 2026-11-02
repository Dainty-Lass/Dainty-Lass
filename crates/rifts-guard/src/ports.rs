// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What Guard reads and writes through: the hunts (realms'), the operations policy (isles'), time, ids and logs.

use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use rifts_seam::{
    AccountId, BoxFuture, CapabilityScope, HubKind, IslesCore, Json, RealmId, RuntimeAuth, SeamError, SessionId,
    StoreError, UnixMillis,
};
use serde_json::Value;

use crate::json::non_negative;
use crate::operations::RewardRollout;
use crate::store::CompletionMode;

pub trait Clock: Send + Sync {
    fn now(&self) -> UnixMillis;
    fn uuid(&self) -> String;
}

/// Version 4 text of 16 bytes.
fn uuid_text(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut out = String::with_capacity(36);
    for (i, byte) in bytes.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The system's time; ids only name records, so any random source does (std's hasher keys).
#[derive(Debug, Default)]
pub struct SystemClock {
    counter: AtomicU64,
}

impl Clock for SystemClock {
    fn now(&self) -> UnixMillis {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        UnixMillis(millis)
    }

    fn uuid(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let mut bytes = [0u8; 16];
        for (half, chunk) in bytes.chunks_mut(8).enumerate() {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u64(n);
            hasher.write_usize(half);
            chunk.copy_from_slice(&hasher.finish().to_be_bytes());
        }
        uuid_text(bytes)
    }
}

/// A clock a test sets; its ids count up from 1.
#[derive(Debug)]
pub struct ManualClock {
    now: AtomicI64,
    ids: AtomicU64,
}

impl ManualClock {
    pub fn at(now: UnixMillis) -> Self {
        Self {
            now: AtomicI64::new(now.0),
            ids: AtomicU64::new(0),
        }
    }

    pub fn advance(&self, millis: u64) {
        let step = i64::try_from(millis).unwrap_or(i64::MAX);
        self.now.fetch_add(step, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> UnixMillis {
        UnixMillis(self.now.load(Ordering::SeqCst))
    }

    fn uuid(&self) -> String {
        let n = self.ids.fetch_add(1, Ordering::SeqCst) + 1;
        let mut bytes = [0u8; 16];
        bytes[8..].copy_from_slice(&n.to_be_bytes());
        uuid_text(bytes)
    }
}

/// `new Date(ms).toISOString()`.
pub fn iso_time(at: UnixMillis) -> String {
    let days = at.0.div_euclid(86_400_000);
    let in_day = at.0.rem_euclid(86_400_000);
    // Howard Hinnant's days-to-civil
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
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000
    )
}

pub trait GuardLog: Send + Sync {
    fn info(&self, line: &str);
    fn warn(&self, line: &str);
    fn error(&self, line: &str);
}

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

/// Keeps every line as `"<level> <line>"`, for tests.
#[derive(Debug, Default)]
pub struct MemoryLog {
    lines: Mutex<Vec<String>>,
}

impl MemoryLog {
    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn push(&self, level: &str, line: &str) {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{level} {line}"));
    }
}

impl GuardLog for MemoryLog {
    fn info(&self, line: &str) {
        self.push("info", line);
    }

    fn warn(&self, line: &str) {
        self.push("warn", line);
    }

    fn error(&self, line: &str) {
        self.push("error", line);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub account: AccountId,
    pub admitted: bool,
}

/// A hosted hunt or hub as Guard sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntView {
    pub session: SessionId,
    pub generation: u64,
    pub state: String,
    pub pre_suspect_state: Option<String>,
    pub host: Option<AccountId>,
    pub realm: Option<RealmId>,
    pub hub: Option<HubKind>,
    pub roster: Vec<RosterEntry>,
    pub frozen_characters: Vec<(AccountId, String)>,
    pub completion_mode: Option<CompletionMode>,
}

impl HuntView {
    pub fn is_hub(&self) -> bool {
        self.hub.is_some()
    }

    pub fn session_kind(&self) -> Option<&'static str> {
        self.hub.map(|_| "REALM_HUB")
    }

    pub fn hub_kind(&self) -> Option<&'static str> {
        self.hub.map(|hub| match hub {
            HubKind::Ramsgate => "RAMSGATE",
            HubKind::Training => "TRAINING",
        })
    }

    /// ` hub=<KIND>` for a hub, else nothing.
    pub(crate) fn hub_tag(&self) -> String {
        self.hub_kind().map(|kind| format!(" hub={kind}")).unwrap_or_default()
    }

    /// ` realm=<id>` when the hunt has one.
    pub(crate) fn realm_tag(&self) -> String {
        self.realm
            .as_ref()
            .map(|realm| format!(" realm={realm}"))
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityView {
    pub capability_id: String,
    pub expires_at: UnixMillis,
    pub revoked: bool,
}

/// The hunts, as realms keeps them.
pub trait HuntSessions: Send + Sync {
    fn hunt(&self, session: &SessionId) -> Result<Option<HuntView>, StoreError>;
    fn hub_present_accounts(&self, session: &SessionId, generation: u64) -> Result<Vec<AccountId>, StoreError>;
    fn capability_by_digest(&self, digest: &str) -> Result<Option<CapabilityView>, StoreError>;

    fn resolve_capability(
        &self,
        digest: &str,
        session_id: &str,
        scope: CapabilityScope,
        now: UnixMillis,
    ) -> Result<Option<RuntimeAuth>, StoreError> {
        let _ = (digest, session_id, scope, now);
        Ok(None)
    }
}

/// No hunts at all: what Guard runs on with no database.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHuntSessions;

impl HuntSessions for NoHuntSessions {
    fn hunt(&self, _session: &SessionId) -> Result<Option<HuntView>, StoreError> {
        Ok(None)
    }

    fn hub_present_accounts(&self, _session: &SessionId, _generation: u64) -> Result<Vec<AccountId>, StoreError> {
        Ok(Vec::new())
    }

    fn capability_by_digest(&self, _digest: &str) -> Result<Option<CapabilityView>, StoreError> {
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicySnapshot {
    pub version: u64,
    pub reward_rollout: RewardRollout,
    pub json: Json,
}

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

type Fut<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

pub trait OperationsPolicyPort: Send + Sync {
    fn policy(&self) -> Fut<'_, PolicySnapshot>;
    /// Writes only at `expected_version` and bumps it; `None` when the version moved.
    fn set_reward_rollout(
        &self,
        rollout: RewardRollout,
        actor: String,
        expected_version: u64,
    ) -> Fut<'_, Option<PolicySnapshot>>;
    fn append_audit(&self, entry: AuditEntry) -> Fut<'_, ()>;
}

#[derive(Debug)]
struct MemoryPolicy {
    version: u64,
    rollout: RewardRollout,
    audit: Vec<AuditEntry>,
}

/// The operations policy in memory.
#[derive(Debug)]
pub struct MemoryOperations {
    state: Mutex<MemoryPolicy>,
}

impl Default for MemoryOperations {
    fn default() -> Self {
        Self {
            state: Mutex::new(MemoryPolicy {
                version: 0,
                rollout: RewardRollout::default(),
                audit: Vec::new(),
            }),
        }
    }
}

impl MemoryOperations {
    pub fn audit(&self) -> Vec<AuditEntry> {
        self.lock().audit.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MemoryPolicy> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn snapshot(state: &MemoryPolicy) -> PolicySnapshot {
        PolicySnapshot {
            version: state.version,
            reward_rollout: state.rollout,
            json: Json(format!(
                r#"{{"version":{},"p2pEmergencyStop":false,"rewardRollout":{},"coopTransport":"ICE"}}"#,
                state.version,
                state.rollout.to_json().0
            )),
        }
    }

    fn current(&self) -> PolicySnapshot {
        Self::snapshot(&self.lock())
    }
}

impl OperationsPolicyPort for MemoryOperations {
    fn policy(&self) -> Fut<'_, PolicySnapshot> {
        let snapshot = self.current();
        Box::pin(async move { Ok(snapshot) })
    }

    fn set_reward_rollout(
        &self,
        rollout: RewardRollout,
        _actor: String,
        expected_version: u64,
    ) -> Fut<'_, Option<PolicySnapshot>> {
        let mut state = self.lock();
        let answer = if state.version == expected_version {
            state.version += 1;
            state.rollout = rollout;
            Some(Self::snapshot(&state))
        } else {
            None
        };
        drop(state);
        Box::pin(async move { Ok(answer) })
    }

    fn append_audit(&self, entry: AuditEntry) -> Fut<'_, ()> {
        self.lock().audit.push(entry);
        Box::pin(async { Ok(()) })
    }
}

/// Reads isles' policy document; writes go to memory, so an update is not kept (spec-rd27-rifts-guard risk 5).
pub struct CoreOperationsPolicy {
    core: Arc<dyn IslesCore>,
    fallback: MemoryOperations,
}

impl CoreOperationsPolicy {
    pub fn new(core: Arc<dyn IslesCore>) -> Self {
        Self {
            core,
            fallback: MemoryOperations::default(),
        }
    }
}

impl OperationsPolicyPort for CoreOperationsPolicy {
    fn policy(&self) -> Fut<'_, PolicySnapshot> {
        Box::pin(async move {
            let Ok(text) = self.core.operations_policy().await else {
                return Ok(self.fallback.current());
            };
            let parsed: Option<Value> = serde_json::from_str(&text.0).ok();
            let field = |name: &str| parsed.as_ref().and_then(|value| value.get(name));
            Ok(PolicySnapshot {
                version: field("version").and_then(non_negative).unwrap_or(0),
                reward_rollout: RewardRollout::from_value(field("rewardRollout")).unwrap_or_default(),
                json: text,
            })
        })
    }

    fn set_reward_rollout(
        &self,
        rollout: RewardRollout,
        actor: String,
        expected_version: u64,
    ) -> Fut<'_, Option<PolicySnapshot>> {
        self.fallback.set_reward_rollout(rollout, actor, expected_version)
    }

    fn append_audit(&self, entry: AuditEntry) -> Fut<'_, ()> {
        self.fallback.append_audit(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::poll_once;

    #[test]
    fn iso_time_vectors() {
        assert_eq!(iso_time(UnixMillis(0)), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_time(UnixMillis(1_792_000_000_123)), "2026-10-14T17:46:40.123Z");
        assert_eq!(iso_time(UnixMillis(951_782_400_000)), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso_time(UnixMillis(-1)), "1969-12-31T23:59:59.999Z");
    }

    fn is_v4(id: &str) -> bool {
        let bytes = id.as_bytes();
        id.len() == 36
            && [8, 13, 18, 23].iter().all(|&i| bytes[i] == b'-')
            && bytes[14] == b'4'
            && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
            && id
                .bytes()
                .all(|b| b == b'-' || b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }

    #[test]
    fn uuids_are_version_4() {
        let manual = ManualClock::at(UnixMillis(5));
        assert_eq!(manual.uuid(), "00000000-0000-4000-8000-000000000001");
        assert_eq!(manual.uuid(), "00000000-0000-4000-8000-000000000002");
        let system = SystemClock::default();
        let (a, b) = (system.uuid(), system.uuid());
        assert!(is_v4(&a) && is_v4(&b), "{a} {b}");
        assert_ne!(a, b);
        manual.advance(10);
        assert_eq!(manual.now(), UnixMillis(15));
        assert!(system.now().0 > 1_700_000_000_000);
    }

    #[test]
    fn memory_log_levels() {
        let log = MemoryLog::default();
        log.info("a");
        log.warn("b");
        log.error("c");
        assert_eq!(log.lines(), ["info a", "warn b", "error c"]);
    }

    #[test]
    fn memory_operations_bumps_at_the_expected_version() {
        let ops = MemoryOperations::default();
        let policy = poll_once(ops.policy()).unwrap().unwrap();
        assert_eq!(policy.version, 0);
        assert_eq!(
            policy.json.0,
            r#"{"version":0,"p2pEmergencyStop":false,"rewardRollout":{"version":0,"tier0ShadowEnabled":true,"tier1GrantsEnabled":false},"coopTransport":"ICE"}"#
        );
        let next = RewardRollout {
            version: 1,
            ..RewardRollout::default()
        };
        assert_eq!(
            poll_once(ops.set_reward_rollout(next, "k".into(), 1)).unwrap(),
            Ok(None)
        );
        let written = poll_once(ops.set_reward_rollout(next, "k".into(), 0))
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!((written.version, written.reward_rollout), (1, next));
    }

    #[test]
    fn hunt_view_kinds() {
        let mut hunt = HuntView {
            session: "01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap(),
            generation: 1,
            state: "RUNNING".into(),
            pre_suspect_state: None,
            host: None,
            realm: None,
            hub: None,
            roster: Vec::new(),
            frozen_characters: Vec::new(),
            completion_mode: None,
        };
        assert!(!hunt.is_hub());
        assert_eq!((hunt.session_kind(), hunt.hub_kind()), (None, None));
        assert_eq!(hunt.hub_tag(), "");
        hunt.hub = Some(HubKind::Training);
        assert_eq!(
            (hunt.session_kind(), hunt.hub_kind()),
            (Some("REALM_HUB"), Some("TRAINING"))
        );
        assert_eq!(hunt.hub_tag(), " hub=TRAINING");
        hunt.hub = Some(HubKind::Ramsgate);
        assert_eq!(hunt.hub_kind(), Some("RAMSGATE"));
    }
}
