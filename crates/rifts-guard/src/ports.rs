// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What Guard reads and writes through: the hunts (realms'), the operations policy (isles'), time, ids and logs.

use std::{
    collections::hash_map::RandomState,
    hash::{BuildHasher, Hasher},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use rifts_seam::{
    AccountId, BoxFuture, CapabilityScope, HubKind, IslesCore, Json, RealmId, RuntimeAuth, SeamError, SessionId,
    StoreError, UnixMillis,
};
use serde_json::Value;

use crate::{json::non_negative, operations::RewardRollout, store::CompletionMode};

// ---- time and ids ---------------------------------------------------------------------------------------------

pub trait Clock: Send + Sync {
    fn now(&self) -> UnixMillis;
    fn uuid(&self) -> String;
}

/// Version 4 text of 16 bytes: the version and variant bits are forced, the rest kept.
fn uuid_text(bytes: [u8; 16]) -> String {
    let mut bits = u128::from_be_bytes(bytes);
    bits = bits & !(0xf << 76) | 0x4 << 76;
    bits = bits & !(0x3 << 62) | 0x2 << 62;
    let hex = format!("{bits:032x}");
    [&hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]].join("-")
}

/// The system's time; ids only name records, so any random source does (std's hasher keys).
#[derive(Debug, Default)]
pub struct SystemClock {
    counter: AtomicU64,
}

impl Clock for SystemClock {
    fn now(&self) -> UnixMillis {
        let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        UnixMillis(i64::try_from(since_epoch.as_millis()).unwrap_or(i64::MAX))
    }

    fn uuid(&self) -> String {
        let serial = self.counter.fetch_add(1, Ordering::Relaxed);
        // Each half from a freshly keyed hasher, so no two ids share a key.
        let half = |which: usize| {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u64(serial);
            hasher.write_usize(which);
            hasher.finish()
        };
        let bits = u128::from(half(0)) << 64 | u128::from(half(1));
        uuid_text(bits.to_be_bytes())
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
        let UnixMillis(start) = now;
        Self {
            now: start.into(),
            ids: 0.into(),
        }
    }

    pub fn advance(&self, millis: u64) {
        self.now
            .fetch_add(millis.try_into().unwrap_or(i64::MAX), Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> UnixMillis {
        UnixMillis(self.now.load(Ordering::SeqCst))
    }

    fn uuid(&self) -> String {
        let serial = 1 + self.ids.fetch_add(1, Ordering::SeqCst);
        uuid_text(u128::from(serial).to_be_bytes())
    }
}

const DAY_MILLIS: i64 = 86_400_000;
/// Any 400 consecutive Gregorian years.
const CYCLE_DAYS: i64 = 146_097;

fn leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Year, month and day of a day count from 1970-01-01.
fn civil(days: i64) -> (i64, i64, i64) {
    let mut year = 1970 + 400 * days.div_euclid(CYCLE_DAYS);
    let mut left = days.rem_euclid(CYCLE_DAYS);
    while left >= 365 + i64::from(leap(year)) {
        left -= 365 + i64::from(leap(year));
        year += 1;
    }
    let february = 28 + i64::from(leap(year));
    let mut month = 1;
    for length in [31, february, 31, 30, 31, 30, 31, 31, 30, 31, 30] {
        if left < length {
            break;
        }
        left -= length;
        month += 1;
    }
    (year, month, left + 1)
}

/// `new Date(ms).toISOString()`.
pub fn iso_time(at: UnixMillis) -> String {
    let (year, month, day) = civil(at.0.div_euclid(DAY_MILLIS));
    let mut clock = at.0.rem_euclid(DAY_MILLIS);
    let millis = clock % 1000;
    clock /= 1000;
    let (hours, minutes, seconds) = (clock / 3600, clock / 60 % 60, clock % 60);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z")
}

// ---- logs -----------------------------------------------------------------------------------------------------

pub trait GuardLog: Send + Sync {
    fn info(&self, line: &str);
    fn warn(&self, line: &str);
    fn error(&self, line: &str);
}

// A log that writes every level the same way, prefixed by the level's name.
macro_rules! levelled_log {
    ($log:ty, |$this:ident, $text:ident| $write:expr) => {
        impl GuardLog for $log {
            fn info(&self, line: &str) {
                let ($this, $text) = (self, ["info", line].join(" "));
                $write
            }

            fn warn(&self, line: &str) {
                let ($this, $text) = (self, ["warn", line].join(" "));
                $write
            }

            fn error(&self, line: &str) {
                let ($this, $text) = (self, ["error", line].join(" "));
                $write
            }
        }
    };
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StderrLog;

levelled_log!(StderrLog, |_log, text| eprintln!("{text}"));

/// Keeps every line as `"<level> <line>"`, for tests.
#[derive(Debug, Default)]
pub struct MemoryLog {
    lines: Mutex<Vec<String>>,
}

impl MemoryLog {
    pub fn lines(&self) -> Vec<String> {
        self.kept().clone()
    }

    fn kept(&self) -> MutexGuard<'_, Vec<String>> {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

levelled_log!(MemoryLog, |log, text| log.kept().push(text));

// ---- hunts ----------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RosterEntry {
    pub account: AccountId,
    pub admitted: bool,
}

/// A hosted hunt or hub as Guard sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
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
        self.is_hub().then_some("REALM_HUB")
    }

    pub fn hub_kind(&self) -> Option<&'static str> {
        Some(if self.hub? == HubKind::Training {
            "TRAINING"
        } else {
            "RAMSGATE"
        })
    }

    /// ` hub=<KIND>` for a hub, else nothing.
    pub(crate) fn hub_tag(&self) -> String {
        let kind = self.hub_kind();
        kind.map_or_else(String::new, |kind| [" hub=", kind].concat())
    }

    /// ` realm=<id>` when the hunt has one.
    pub(crate) fn realm_tag(&self) -> String {
        let Some(realm) = &self.realm else {
            return String::new();
        };
        format!(" realm={realm}")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

    /// A store that cannot resolve capabilities knows none.
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
#[derive(Clone, Copy, Debug, Default)]
pub struct NoHuntSessions;

impl HuntSessions for NoHuntSessions {
    fn hunt(&self, _: &SessionId) -> Result<Option<HuntView>, StoreError> {
        Ok(None)
    }

    fn hub_present_accounts(&self, _: &SessionId, _: u64) -> Result<Vec<AccountId>, StoreError> {
        Ok(vec![])
    }

    fn capability_by_digest(&self, _: &str) -> Result<Option<CapabilityView>, StoreError> {
        Ok(None)
    }
}

// ---- the operations policy ------------------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicySnapshot {
    pub version: u64,
    pub reward_rollout: RewardRollout,
    pub json: Json,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

fn settled<'a, T: Send + 'a>(value: T) -> Fut<'a, T> {
    Box::pin(std::future::ready(Ok(value)))
}

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

#[derive(Debug, Default)]
struct PolicyState {
    version: u64,
    rollout: RewardRollout,
    audit: Vec<AuditEntry>,
}

impl PolicyState {
    fn snapshot(&self) -> PolicySnapshot {
        let rollout = self.rollout.to_json().0;
        let json = format!(
            r#"{{"version":{},"p2pEmergencyStop":false,"rewardRollout":{rollout},"coopTransport":"ICE"}}"#,
            self.version
        );
        PolicySnapshot {
            version: self.version,
            reward_rollout: self.rollout,
            json: Json(json),
        }
    }
}

/// The operations policy in memory.
#[derive(Debug, Default)]
pub struct MemoryOperations {
    state: Mutex<PolicyState>,
}

impl MemoryOperations {
    pub fn audit(&self) -> Vec<AuditEntry> {
        self.held().audit.clone()
    }

    fn held(&self) -> MutexGuard<'_, PolicyState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn current(&self) -> PolicySnapshot {
        self.held().snapshot()
    }
}

impl OperationsPolicyPort for MemoryOperations {
    fn policy(&self) -> Fut<'_, PolicySnapshot> {
        settled(self.current())
    }

    fn set_reward_rollout(&self, rollout: RewardRollout, _: String, expected: u64) -> Fut<'_, Option<PolicySnapshot>> {
        let written = {
            let mut state = self.held();
            (state.version == expected).then(|| {
                state.version += 1;
                state.rollout = rollout;
                state.snapshot()
            })
        };
        settled(written)
    }

    fn append_audit(&self, entry: AuditEntry) -> Fut<'_, ()> {
        self.held().audit.push(entry);
        settled(())
    }
}

/// Reads isles' policy document; writes go to memory, so an update is not kept (spec-rd27-rifts-guard risk 5).
pub struct CoreOperationsPolicy {
    core: Arc<dyn IslesCore>,
    fallback: MemoryOperations,
}

impl CoreOperationsPolicy {
    pub fn new(core: Arc<dyn IslesCore>) -> Self {
        let fallback = MemoryOperations::default();
        Self { core, fallback }
    }

    /// Isles' document as it is, with the version and rollout read out of it; unreadable parts take defaults.
    fn snapshot_of(text: Json) -> PolicySnapshot {
        let document = serde_json::from_str::<Value>(&text.0).unwrap_or(Value::Null);
        let version = document.get("version").and_then(non_negative);
        let rollout = RewardRollout::from_value(document.get("rewardRollout"));
        PolicySnapshot {
            version: version.unwrap_or(0),
            reward_rollout: rollout.unwrap_or_default(),
            json: text,
        }
    }
}

impl OperationsPolicyPort for CoreOperationsPolicy {
    fn policy(&self) -> Fut<'_, PolicySnapshot> {
        Box::pin(async move {
            let snapshot = match self.core.operations_policy().await {
                Ok(text) => Self::snapshot_of(text),
                Err(_) => self.fallback.current(),
            };
            Ok(snapshot)
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
    use rifts_seam::poll_once;

    use super::*;

    #[test]
    fn iso_time_vectors() {
        let vectors = [
            (0, "1970-01-01T00:00:00.000Z"),
            (1_792_000_000_123, "2026-10-14T17:46:40.123Z"),
            (951_782_400_000, "2000-02-29T00:00:00.000Z"),
            (-1, "1969-12-31T23:59:59.999Z"),
        ];
        for (millis, text) in vectors {
            assert_eq!(iso_time(UnixMillis(millis)), text, "{millis}");
        }
    }

    fn is_v4(id: &str) -> bool {
        let groups: Vec<&str> = id.split('-').collect();
        let lengths: Vec<usize> = groups.iter().map(|g| g.len()).collect();
        let hex = groups
            .iter()
            .all(|g| g.bytes().all(|b| b"0123456789abcdef".contains(&b)));
        lengths == [8, 4, 4, 4, 12] && hex && groups[2].starts_with('4') && groups[3].starts_with(['8', '9', 'a', 'b'])
    }

    #[test]
    fn uuids_are_version_4() {
        let manual = ManualClock::at(UnixMillis(5));
        for n in 1..=2 {
            assert_eq!(manual.uuid(), format!("00000000-0000-4000-8000-00000000000{n}"));
        }
        manual.advance(10);
        assert_eq!(manual.now(), UnixMillis(15));

        let system = SystemClock::default();
        let ids = [system.uuid(), system.uuid()];
        assert!(ids.iter().all(|id| is_v4(id)), "{ids:?}");
        assert_ne!(ids[0], ids[1]);
        assert!(system.now().0 > 1_700_000_000_000);
    }

    #[test]
    fn memory_log_levels() {
        let log = MemoryLog::default();
        let writes: [fn(&MemoryLog, &str); 3] = [|l, s| l.info(s), |l, s| l.warn(s), |l, s| l.error(s)];
        for (write, text) in writes.iter().zip(["a", "b", "c"]) {
            write(&log, text);
        }
        assert_eq!(log.lines(), ["info a", "warn b", "error c"]);
    }

    #[test]
    fn memory_operations_bumps_at_the_expected_version() {
        let ops = MemoryOperations::default();
        let policy = poll_once(ops.policy()).unwrap().unwrap();
        assert_eq!(policy.version, 0);
        let want = concat!(
            r#"{"version":0,"p2pEmergencyStop":false,"#,
            r#""rewardRollout":{"version":0,"tier0ShadowEnabled":true,"tier1GrantsEnabled":false},"#,
            r#""coopTransport":"ICE"}"#
        );
        assert_eq!(policy.json.0, want);
        let next = RewardRollout {
            version: 1,
            ..Default::default()
        };
        let stale = poll_once(ops.set_reward_rollout(next, "k".into(), 1)).unwrap();
        assert_eq!(stale, Ok(None));
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
            roster: vec![],
            frozen_characters: vec![],
            completion_mode: None,
        };
        let cases = [
            (None, false, None, None, ""),
            (
                Some(HubKind::Training),
                true,
                Some("REALM_HUB"),
                Some("TRAINING"),
                " hub=TRAINING",
            ),
            (
                Some(HubKind::Ramsgate),
                true,
                Some("REALM_HUB"),
                Some("RAMSGATE"),
                " hub=RAMSGATE",
            ),
        ];
        for (hub, is_hub, session_kind, hub_kind, tag) in cases {
            hunt.hub = hub;
            assert_eq!(hunt.is_hub(), is_hub);
            assert_eq!((hunt.session_kind(), hunt.hub_kind()), (session_kind, hub_kind));
            assert_eq!(hunt.hub_tag(), tag);
        }
    }
}
