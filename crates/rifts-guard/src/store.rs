// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What Guard keeps, the store it keeps it in, and two stores without a database.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use rifts_seam::{
    AccountId, Actor, Component, HostWriteKind, Json, RealmId, SessionId, StoreError, TableSpec, Unimplemented,
    UnixMillis,
};

/// The chain head before the first batch.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The summaries' lifetime: 400 days.
pub const XP_SUMMARY_TTL_MS: u64 = 34_560_000_000;

const fn table(name: &'static str, node_collection: &'static str, keys: &'static str) -> TableSpec {
    TableSpec {
        name,
        node_collection,
        owner: Component::Guard,
        keys,
        expires_by: None,
    }
}

pub const TABLES: &[TableSpec] = &[
    table("rifts_hunt_guard", "p2pHuntSessions.guard*", "session + generation"),
    table(
        "rifts_guard_batches",
        "(the private module's evidence batches)",
        "session + generation + batch unique",
    ),
    table(
        "rifts_guard_findings",
        "(the private module's findings)",
        "session + generation + at",
    ),
    TableSpec {
        expires_by: Some("expires_at"),
        ..table(
            "rifts_traffic_windows",
            "guardTrafficWindows",
            "window id (scope, window start, hashed key)",
        )
    },
    table(
        "rifts_host_writes",
        "p2pHostWrites",
        "session + generation; target, host, realm by at",
    ),
    TableSpec {
        expires_by: Some("ttl_at"),
        ..table("rifts_hunt_xp_summaries", "p2pHuntXpSummaries", "session + member")
    },
    table(
        "rifts_operations_alerts",
        "operationsAlerts",
        "alert; status + created_at",
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompletionMode {
    FirstDefeat,
    EndOfHunt,
    NoCompletion,
}

impl CompletionMode {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "FIRST_DEFEAT" => Some(Self::FirstDefeat),
            "END_OF_HUNT" => Some(Self::EndOfHunt),
            "NO_COMPLETION" => Some(Self::NoCompletion),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FirstDefeat => "FIRST_DEFEAT",
            Self::EndOfHunt => "END_OF_HUNT",
            Self::NoCompletion => "NO_COMPLETION",
        }
    }
}

/// Where a hunt's encounter is; it only moves forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum EncounterState {
    #[default]
    Created,
    SessionStarted,
    WorldReady,
    EncounterActive,
    BehemothDefeated,
    CompletionRequested,
    Aborted,
}

impl EncounterState {
    pub const ALL: [Self; 7] = [
        Self::Created,
        Self::SessionStarted,
        Self::WorldReady,
        Self::EncounterActive,
        Self::BehemothDefeated,
        Self::CompletionRequested,
        Self::Aborted,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "CREATED",
            Self::SessionStarted => "SESSION_STARTED",
            Self::WorldReady => "WORLD_READY",
            Self::EncounterActive => "ENCOUNTER_ACTIVE",
            Self::BehemothDefeated => "BEHEMOTH_DEFEATED",
            Self::CompletionRequested => "COMPLETION_REQUESTED",
            Self::Aborted => "ABORTED",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == text)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::CompletionRequested | Self::Aborted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Disposition {
    #[default]
    ObserveClear,
    ObserveFlagged,
}

impl Disposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObserveClear => "OBSERVE_CLEAR",
            Self::ObserveFlagged => "OBSERVE_FLAGGED",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "OBSERVE_CLEAR" => Some(Self::ObserveClear),
            "OBSERVE_FLAGGED" => Some(Self::ObserveFlagged),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TrafficScope {
    Host,
    Session,
}

impl TrafficScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Session => "session",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "host" => Some(Self::Host),
            "session" => Some(Self::Session),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostWriteRecord {
    pub kind: HostWriteKind,
    pub target: AccountId,
    /// Empty when the hunt has no host.
    pub host: AccountId,
    pub session: SessionId,
    pub generation: u64,
    pub realm: Option<RealmId>,
    pub session_kind: Option<String>,
    pub hub_kind: Option<String>,
    pub summary: Json,
    pub at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficWindowRecord {
    pub window_id: String,
    pub scope: TrafficScope,
    pub window_start: UnixMillis,
    pub requests: u64,
    pub bytes: u64,
    pub expires_at: UnixMillis,
}

/// One key's charge against one window; the key itself is only in the id's hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficReservation {
    pub window_id: String,
    pub scope: TrafficScope,
    pub window_start: UnixMillis,
    pub expires_at: UnixMillis,
    pub bytes: u64,
    pub max_requests: u64,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRecord {
    pub final_sequence: u64,
    pub final_hash: String,
    pub at: UnixMillis,
}

/// One hunt generation's evidence chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainRecord {
    pub session: SessionId,
    pub generation: u64,
    pub revision: u64,
    pub completion_mode: Option<CompletionMode>,
    pub last_sequence: u64,
    pub head_hash: String,
    pub encounter_state: EncounterState,
    pub disposition: Disposition,
    pub finding_count: u64,
    pub batch_count: u64,
    pub event_count: u64,
    pub event_bytes: u64,
    pub last_monotonic_ms: Option<u64>,
    pub event_types: BTreeMap<String, u64>,
    pub has_unattributed_damage: bool,
    pub last_received_at: Option<UnixMillis>,
    pub completion: Option<CompletionRecord>,
}

impl ChainRecord {
    pub fn new(session: SessionId, generation: u64, completion_mode: Option<CompletionMode>) -> Self {
        Self {
            session,
            generation,
            revision: 0,
            completion_mode,
            last_sequence: 0,
            head_hash: GENESIS_HASH.to_owned(),
            encounter_state: EncounterState::Created,
            disposition: Disposition::ObserveClear,
            finding_count: 0,
            batch_count: 0,
            event_count: 0,
            event_bytes: 0,
            last_monotonic_ms: None,
            event_types: BTreeMap::new(),
            has_unattributed_damage: false,
            last_received_at: None,
            completion: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchRecord {
    pub session: SessionId,
    pub generation: u64,
    pub batch_id: String,
    pub previous_hash: String,
    pub batch_hash: String,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub event_count: u64,
    pub event_bytes: u64,
    /// The batch's distinct types, in byte order.
    pub event_types: Vec<String>,
    pub received_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingRecord {
    pub session: SessionId,
    pub generation: u64,
    pub sequence: Option<u64>,
    pub code: String,
    pub detail: String,
    pub at: UnixMillis,
}

/// A chain write: the new chain, the batch that moved it (if any) and the findings, stored together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainUpdate {
    pub chain: ChainRecord,
    pub batch: Option<BatchRecord>,
    pub findings: Vec<FindingRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XpSummaryRecord {
    pub session: SessionId,
    pub member: AccountId,
    pub host: AccountId,
    pub realm: Option<RealmId>,
    pub minutes: u32,
    pub grants: Json,
    pub flagged: bool,
    pub created_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertRecord {
    pub alert_id: String,
    pub code: String,
    pub severity: String,
    pub message: String,
    pub details: Json,
    pub created_at: UnixMillis,
    pub acknowledged_at: Option<UnixMillis>,
    pub acknowledged_by: Option<String>,
}

/// Guard's storage; each method is one transaction.
pub trait GuardStore: Send + Sync {
    fn record_host_write(&self, actor: &Actor, write: &HostWriteRecord) -> Result<(), StoreError>;
    /// Both windows are charged, or neither.
    fn reserve_traffic(
        &self,
        actor: &Actor,
        host: &TrafficReservation,
        session: &TrafficReservation,
    ) -> Result<bool, StoreError>;
    fn chain(&self, session: &SessionId, generation: u64) -> Result<Option<ChainRecord>, StoreError>;
    /// `false`, writing nothing, when the stored revision is not `expected_revision` or the batch id is taken.
    fn apply_chain(&self, actor: &Actor, update: &ChainUpdate, expected_revision: u64) -> Result<bool, StoreError>;
    fn batch(&self, session: &SessionId, generation: u64, batch_id: &str) -> Result<Option<BatchRecord>, StoreError>;
    fn findings(&self, session: &SessionId, generation: u64) -> Result<Vec<FindingRecord>, StoreError>;
    fn put_xp_summary(&self, actor: &Actor, summary: &XpSummaryRecord) -> Result<(), StoreError>;
    fn insert_alert(&self, actor: &Actor, alert: &AlertRecord) -> Result<(), StoreError>;
    fn acknowledge_alert(&self, actor: &Actor, alert_id: &str, at: UnixMillis) -> Result<bool, StoreError>;
}

/// Who acknowledged: the name the actor goes by.
pub(crate) fn actor_name(actor: &Actor) -> String {
    match actor {
        Actor::Account(account) => account.0.clone(),
        Actor::HostRuntime { host, .. } => host.0.clone(),
        Actor::Admin { key_id } => key_id.clone(),
        Actor::System(task) => (*task).to_owned(),
    }
}

/// A window after one more request of `bytes`, if it stays within the reservation's limits.
pub(crate) fn charged(requests: u64, bytes: u64, reservation: &TrafficReservation) -> Option<(u64, u64)> {
    let total = bytes.checked_add(reservation.bytes)?;
    (requests < reservation.max_requests && total <= reservation.max_bytes).then_some((requests + 1, total))
}

#[derive(Debug, Default)]
struct Memory {
    host_writes: Vec<HostWriteRecord>,
    windows: BTreeMap<String, TrafficWindowRecord>,
    chains: BTreeMap<(SessionId, u64), ChainRecord>,
    batches: BTreeMap<(SessionId, u64, String), BatchRecord>,
    findings: Vec<FindingRecord>,
    summaries: BTreeMap<(SessionId, AccountId), (XpSummaryRecord, UnixMillis)>,
    alerts: BTreeMap<String, AlertRecord>,
}

/// The store's semantics behind one lock.
#[derive(Debug, Default)]
pub struct MemoryGuardStore {
    inner: Mutex<Memory>,
}

impl MemoryGuardStore {
    fn lock(&self) -> MutexGuard<'_, Memory> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn host_writes(&self) -> Vec<HostWriteRecord> {
        self.lock().host_writes.clone()
    }

    pub fn windows(&self) -> Vec<TrafficWindowRecord> {
        self.lock().windows.values().cloned().collect()
    }

    pub fn alert(&self, alert_id: &str) -> Option<AlertRecord> {
        self.lock().alerts.get(alert_id).cloned()
    }
}

impl GuardStore for MemoryGuardStore {
    fn record_host_write(&self, _actor: &Actor, write: &HostWriteRecord) -> Result<(), StoreError> {
        self.lock().host_writes.push(write.clone());
        Ok(())
    }

    fn reserve_traffic(
        &self,
        _actor: &Actor,
        host: &TrafficReservation,
        session: &TrafficReservation,
    ) -> Result<bool, StoreError> {
        let mut memory = self.lock();
        memory.windows.retain(|_, window| window.expires_at > host.window_start);
        let counts = |memory: &Memory, id: &str| memory.windows.get(id).map_or((0, 0), |w| (w.requests, w.bytes));
        let Some(host_after) = ({
            let (requests, bytes) = counts(&memory, &host.window_id);
            charged(requests, bytes, host)
        }) else {
            return Ok(false);
        };
        let session_before = if session.window_id == host.window_id {
            host_after
        } else {
            counts(&memory, &session.window_id)
        };
        let Some(session_after) = charged(session_before.0, session_before.1, session) else {
            return Ok(false);
        };
        for (reservation, (requests, bytes)) in [(host, host_after), (session, session_after)] {
            let window = memory
                .windows
                .entry(reservation.window_id.clone())
                .or_insert_with(|| TrafficWindowRecord {
                    window_id: reservation.window_id.clone(),
                    scope: reservation.scope,
                    window_start: reservation.window_start,
                    requests: 0,
                    bytes: 0,
                    expires_at: reservation.expires_at,
                });
            window.requests = requests;
            window.bytes = bytes;
        }
        Ok(true)
    }

    fn chain(&self, session: &SessionId, generation: u64) -> Result<Option<ChainRecord>, StoreError> {
        Ok(self.lock().chains.get(&(session.clone(), generation)).cloned())
    }

    fn apply_chain(&self, _actor: &Actor, update: &ChainUpdate, expected_revision: u64) -> Result<bool, StoreError> {
        let mut memory = self.lock();
        let chain = &update.chain;
        let key = (chain.session.clone(), chain.generation);
        let stored = memory.chains.get(&key).map_or(0, |c| c.revision);
        if stored != expected_revision {
            return Ok(false);
        }
        if let Some(batch) = &update.batch {
            let batch_key = (batch.session.clone(), batch.generation, batch.batch_id.clone());
            if memory.batches.contains_key(&batch_key) {
                return Ok(false);
            }
            memory.batches.insert(batch_key, batch.clone());
        }
        memory.chains.insert(key, chain.clone());
        memory.findings.extend(update.findings.iter().cloned());
        Ok(true)
    }

    fn batch(&self, session: &SessionId, generation: u64, batch_id: &str) -> Result<Option<BatchRecord>, StoreError> {
        Ok(self
            .lock()
            .batches
            .get(&(session.clone(), generation, batch_id.to_owned()))
            .cloned())
    }

    fn findings(&self, session: &SessionId, generation: u64) -> Result<Vec<FindingRecord>, StoreError> {
        Ok(self
            .lock()
            .findings
            .iter()
            .filter(|f| &f.session == session && f.generation == generation)
            .cloned()
            .collect())
    }

    fn put_xp_summary(&self, _actor: &Actor, summary: &XpSummaryRecord) -> Result<(), StoreError> {
        let ttl = summary.created_at.0.saturating_add_unsigned(XP_SUMMARY_TTL_MS);
        self.lock().summaries.insert(
            (summary.session.clone(), summary.member.clone()),
            (summary.clone(), UnixMillis(ttl)),
        );
        Ok(())
    }

    fn insert_alert(&self, _actor: &Actor, alert: &AlertRecord) -> Result<(), StoreError> {
        self.lock()
            .alerts
            .entry(alert.alert_id.clone())
            .or_insert_with(|| alert.clone());
        Ok(())
    }

    fn acknowledge_alert(&self, actor: &Actor, alert_id: &str, at: UnixMillis) -> Result<bool, StoreError> {
        let mut memory = self.lock();
        match memory.alerts.get_mut(alert_id) {
            Some(alert) if alert.acknowledged_at.is_none() => {
                alert.acknowledged_at = Some(at);
                alert.acknowledged_by = Some(actor_name(actor));
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

/// No database: every call is `Unimplemented` as `store.<method>`.
#[derive(Debug, Clone, Copy, Default)]
pub struct StubGuardStore;

fn missing<T>(operation: &'static str) -> Result<T, StoreError> {
    Err(StoreError::Unimplemented(Unimplemented::new(
        Component::Guard,
        operation,
    )))
}

impl GuardStore for StubGuardStore {
    fn record_host_write(&self, _actor: &Actor, _write: &HostWriteRecord) -> Result<(), StoreError> {
        missing("store.record_host_write")
    }

    fn reserve_traffic(
        &self,
        _actor: &Actor,
        _host: &TrafficReservation,
        _session: &TrafficReservation,
    ) -> Result<bool, StoreError> {
        missing("store.reserve_traffic")
    }

    fn chain(&self, _session: &SessionId, _generation: u64) -> Result<Option<ChainRecord>, StoreError> {
        missing("store.chain")
    }

    fn apply_chain(&self, _actor: &Actor, _update: &ChainUpdate, _expected: u64) -> Result<bool, StoreError> {
        missing("store.apply_chain")
    }

    fn batch(
        &self,
        _session: &SessionId,
        _generation: u64,
        _batch_id: &str,
    ) -> Result<Option<BatchRecord>, StoreError> {
        missing("store.batch")
    }

    fn findings(&self, _session: &SessionId, _generation: u64) -> Result<Vec<FindingRecord>, StoreError> {
        missing("store.findings")
    }

    fn put_xp_summary(&self, _actor: &Actor, _summary: &XpSummaryRecord) -> Result<(), StoreError> {
        missing("store.put_xp_summary")
    }

    fn insert_alert(&self, _actor: &Actor, _alert: &AlertRecord) -> Result<(), StoreError> {
        missing("store.insert_alert")
    }

    fn acknowledge_alert(&self, _actor: &Actor, _alert_id: &str, _at: UnixMillis) -> Result<bool, StoreError> {
        missing("store.acknowledge_alert")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap()
    }

    fn reservation(id: &str, scope: TrafficScope, max_requests: u64) -> TrafficReservation {
        TrafficReservation {
            window_id: id.into(),
            scope,
            window_start: UnixMillis(60_000),
            expires_at: UnixMillis(180_000),
            bytes: 10,
            max_requests,
            max_bytes: 100,
        }
    }

    #[test]
    fn enum_texts() {
        let names: Vec<&str> = EncounterState::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            [
                "CREATED",
                "SESSION_STARTED",
                "WORLD_READY",
                "ENCOUNTER_ACTIVE",
                "BEHEMOTH_DEFEATED",
                "COMPLETION_REQUESTED",
                "ABORTED"
            ]
        );
        assert!(EncounterState::Aborted.is_terminal() && !EncounterState::BehemothDefeated.is_terminal());
        assert_eq!(
            CompletionMode::parse("NO_COMPLETION"),
            Some(CompletionMode::NoCompletion)
        );
        assert_eq!(CompletionMode::parse("no_completion"), None);
        assert_eq!(Disposition::default().as_str(), "OBSERVE_CLEAR");
        assert_eq!(TrafficScope::Session.as_str(), "session");
        assert_eq!(GENESIS_HASH.len(), 64);
    }

    #[test]
    fn memory_traffic_pair_is_all_or_nothing() {
        let store = MemoryGuardStore::default();
        let actor = Actor::System("guard");
        let host = reservation("host:a", TrafficScope::Host, 5);
        let tight = reservation("session:b", TrafficScope::Session, 1);
        assert_eq!(store.reserve_traffic(&actor, &host, &tight), Ok(true));
        assert_eq!(store.reserve_traffic(&actor, &host, &tight), Ok(false));
        let windows = store.windows();
        assert_eq!(windows.iter().map(|w| w.requests).collect::<Vec<_>>(), [1, 1]);
        // the same id sees its own first charge
        let same = MemoryGuardStore::default();
        let one = reservation("x", TrafficScope::Host, 2);
        assert_eq!(same.reserve_traffic(&actor, &one, &one), Ok(true));
        assert_eq!(same.windows()[0].requests, 2);
        assert_eq!(same.reserve_traffic(&actor, &one, &one), Ok(false));
    }

    #[test]
    fn memory_chain_revisions_and_batch_ids() {
        let store = MemoryGuardStore::default();
        let actor = Actor::System("guard");
        let mut chain = ChainRecord::new(session(), 1, None);
        chain.revision = 1;
        let update = ChainUpdate {
            chain: chain.clone(),
            batch: None,
            findings: Vec::new(),
        };
        assert_eq!(store.apply_chain(&actor, &update, 1), Ok(false));
        assert_eq!(store.apply_chain(&actor, &update, 0), Ok(true));
        assert_eq!(store.apply_chain(&actor, &update, 0), Ok(false));
        assert_eq!(store.chain(&session(), 1).unwrap(), Some(chain));
    }

    #[test]
    fn alerts_acknowledge_once() {
        let store = MemoryGuardStore::default();
        let alert = AlertRecord {
            alert_id: "alert-0001".into(),
            code: "C".into(),
            severity: "HIGH".into(),
            message: "m".into(),
            details: Json::empty_object(),
            created_at: UnixMillis(1),
            acknowledged_at: None,
            acknowledged_by: None,
        };
        store.insert_alert(&Actor::System("t"), &alert).unwrap();
        let admin = Actor::Admin { key_id: "ops".into() };
        assert_eq!(store.acknowledge_alert(&admin, "alert-0001", UnixMillis(9)), Ok(true));
        assert_eq!(store.acknowledge_alert(&admin, "alert-0001", UnixMillis(10)), Ok(false));
        assert_eq!(store.acknowledge_alert(&admin, "missing", UnixMillis(10)), Ok(false));
        let stored = store.alert("alert-0001").unwrap();
        assert_eq!(
            (stored.acknowledged_at, stored.acknowledged_by),
            (Some(UnixMillis(9)), Some("ops".into()))
        );
    }

    #[test]
    fn stub_store_names_each_method() {
        assert_eq!(
            StubGuardStore.chain(&session(), 1),
            Err(StoreError::Unimplemented(Unimplemented::new(
                Component::Guard,
                "store.chain"
            )))
        );
    }

    #[test]
    fn tables_in_order() {
        let names: Vec<&str> = TABLES.iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            [
                "rifts_hunt_guard",
                "rifts_guard_batches",
                "rifts_guard_findings",
                "rifts_traffic_windows",
                "rifts_host_writes",
                "rifts_hunt_xp_summaries",
                "rifts_operations_alerts"
            ]
        );
        assert!(TABLES.iter().all(|t| t.owner == Component::Guard));
        assert_eq!(TABLES[3].expires_by, Some("expires_at"));
        assert_eq!(TABLES[5].expires_by, Some("ttl_at"));
    }
}
