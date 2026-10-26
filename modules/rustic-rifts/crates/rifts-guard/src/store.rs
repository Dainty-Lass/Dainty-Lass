//! Guard's tables (RF1, section 7), shaped for `store` (see `rifts_seam`'s store notes). The private module's
//! evidence, verdict and receipt collections are not in the tree; `rifts_guard_batches` and `rifts_guard_findings` are
//! RF6's own, for the evidence chain intake. [`MemoryGuardStore`] holds them until RF5 puts them on `store` (DF7).

use std::collections::BTreeMap;
use std::sync::Mutex;

use rifts_seam::{
    AccountId, Actor, Component, HostWriteKind, Json, RealmId, SessionId, StoreError, TableSpec, Unimplemented,
    UnixMillis,
};

use crate::ports::lock;

pub const TABLES: &[TableSpec] = &[
    TableSpec {
        name: "rifts_hunt_guard",
        node_collection: "p2pHuntSessions.guard*",
        owner: Component::Guard,
        keys: "session + generation",
        expires_by: None,
    },
    TableSpec {
        name: "rifts_guard_batches",
        node_collection: "(the private module's evidence batches)",
        owner: Component::Guard,
        keys: "session + generation + batch unique",
        expires_by: None,
    },
    TableSpec {
        name: "rifts_guard_findings",
        node_collection: "(the private module's findings)",
        owner: Component::Guard,
        keys: "session + generation + at",
        expires_by: None,
    },
    TableSpec {
        name: "rifts_traffic_windows",
        node_collection: "guardTrafficWindows",
        owner: Component::Guard,
        keys: "window id (scope, window start, hashed key)",
        expires_by: Some("expires_at"),
    },
    TableSpec {
        name: "rifts_host_writes",
        node_collection: "p2pHostWrites",
        owner: Component::Guard,
        keys: "session + generation; target, host, realm by at",
        expires_by: None,
    },
    TableSpec {
        name: "rifts_hunt_xp_summaries",
        node_collection: "p2pHuntXpSummaries",
        owner: Component::Guard,
        keys: "session + member",
        expires_by: Some("ttl_at"),
    },
    TableSpec {
        name: "rifts_operations_alerts",
        node_collection: "operationsAlerts",
        owner: Component::Guard,
        keys: "alert; status + created_at",
        expires_by: None,
    },
];

/// `p2pHostWrites`: one accepted host write, for monitoring and reversal. Realm fields only for a realm session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostWriteRecord {
    pub kind: HostWriteKind,
    pub target: AccountId,
    pub host: AccountId,
    pub session: SessionId,
    pub generation: u64,
    pub realm: Option<RealmId>,
    pub session_kind: Option<String>,
    pub hub_kind: Option<String>,
    pub summary: Json,
    pub at: UnixMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TrafficScope {
    Host,
    Session,
}

impl TrafficScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Session => "session",
        }
    }
}

/// One fixed-window reservation (`ReserveWindow`): accepted while the window has a request and the bytes left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficReservation {
    /// `scope:windowStart:sha256(scope\0key\0windowStart)`: the key itself is never stored.
    pub window_id: String,
    pub scope: TrafficScope,
    pub window_start: UnixMillis,
    pub expires_at: UnixMillis,
    pub bytes: u64,
    pub max_requests: u64,
    pub max_bytes: u64,
}

/// `guardTrafficWindows`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficWindowRecord {
    pub window_id: String,
    pub scope: TrafficScope,
    pub window_start: UnixMillis,
    pub requests: u64,
    pub bytes: u64,
    pub expires_at: UnixMillis,
}

/// `guardEncounterState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
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
    pub const fn as_str(self) -> &'static str {
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

    /// No more evidence after these.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::CompletionRequested | Self::Aborted)
    }
}

/// `guardDisposition`. Observe mode never quarantines: a finding flags the hunt and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Disposition {
    #[default]
    ObserveClear,
    ObserveFlagged,
}

impl Disposition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ObserveClear => "OBSERVE_CLEAR",
            Self::ObserveFlagged => "OBSERVE_FLAGGED",
        }
    }
}

/// `guardCompletionMode`, frozen into the hunt when realms creates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// The accepted completion of a chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRecord {
    pub final_sequence: u64,
    pub final_hash: String,
    pub at: UnixMillis,
}

/// `p2pHuntSessions.guard*` of one generation: the evidence chain's head and counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainRecord {
    pub session: SessionId,
    pub generation: u64,
    /// Bumped on every write; a write names the revision it read.
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

/// `guardHeadHash` before the first batch.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

impl ChainRecord {
    /// A generation's chain before its first batch (`guardLastSequence: 0`, the zero head, `CREATED`, clear).
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

/// One accepted evidence batch, without its payloads.
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
    pub event_types: Vec<String>,
    pub received_at: UnixMillis,
}

/// What observe mode would have refused, recorded instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingRecord {
    pub session: SessionId,
    pub generation: u64,
    pub sequence: Option<u64>,
    pub code: String,
    pub detail: String,
    pub at: UnixMillis,
}

/// One write to a chain: its new head (revision already bumped), the batch that moved it, and the findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainUpdate {
    pub chain: ChainRecord,
    pub batch: Option<BatchRecord>,
    pub findings: Vec<FindingRecord>,
}

/// `p2pHuntXpSummaries`, about 400 days.
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

/// `operationsAlerts`; never purged.
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

pub trait GuardStore: Send + Sync {
    fn record_host_write(&self, actor: &Actor, write: &HostWriteRecord) -> Result<(), StoreError>;
    /// `ConsumeDistributedGuardTrafficPair`: the host window, then the session window, in one transaction. Both are
    /// charged or neither is (a session refusal must not use host capacity); expired windows may be purged.
    fn reserve_traffic(
        &self,
        actor: &Actor,
        host: &TrafficReservation,
        session: &TrafficReservation,
    ) -> Result<bool, StoreError>;
    fn chain(&self, session: &SessionId, generation: u64) -> Result<Option<ChainRecord>, StoreError>;
    /// Writes the update in one transaction when the stored revision is `expected_revision` (0: no row yet).
    fn apply_chain(&self, actor: &Actor, update: &ChainUpdate, expected_revision: u64) -> Result<bool, StoreError>;
    fn batch(&self, session: &SessionId, generation: u64, batch_id: &str) -> Result<Option<BatchRecord>, StoreError>;
    fn findings(&self, session: &SessionId, generation: u64) -> Result<Vec<FindingRecord>, StoreError>;
    fn put_xp_summary(&self, actor: &Actor, summary: &XpSummaryRecord) -> Result<(), StoreError>;
    fn insert_alert(&self, actor: &Actor, alert: &AlertRecord) -> Result<(), StoreError>;
    /// Acknowledges an open alert; false when it is missing or already acknowledged.
    fn acknowledge_alert(&self, actor: &Actor, alert_id: &str, at: UnixMillis) -> Result<bool, StoreError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StubGuardStore;

fn unimplemented<T>(operation: &'static str) -> Result<T, StoreError> {
    Err(Unimplemented::new(Component::Guard, operation).into())
}

impl GuardStore for StubGuardStore {
    fn record_host_write(&self, _: &Actor, _: &HostWriteRecord) -> Result<(), StoreError> {
        unimplemented("store.record_host_write")
    }
    fn reserve_traffic(&self, _: &Actor, _: &TrafficReservation, _: &TrafficReservation) -> Result<bool, StoreError> {
        unimplemented("store.reserve_traffic")
    }
    fn chain(&self, _: &SessionId, _: u64) -> Result<Option<ChainRecord>, StoreError> {
        unimplemented("store.chain")
    }
    fn apply_chain(&self, _: &Actor, _: &ChainUpdate, _: u64) -> Result<bool, StoreError> {
        unimplemented("store.apply_chain")
    }
    fn batch(&self, _: &SessionId, _: u64, _: &str) -> Result<Option<BatchRecord>, StoreError> {
        unimplemented("store.batch")
    }
    fn findings(&self, _: &SessionId, _: u64) -> Result<Vec<FindingRecord>, StoreError> {
        unimplemented("store.findings")
    }
    fn put_xp_summary(&self, _: &Actor, _: &XpSummaryRecord) -> Result<(), StoreError> {
        unimplemented("store.put_xp_summary")
    }
    fn insert_alert(&self, _: &Actor, _: &AlertRecord) -> Result<(), StoreError> {
        unimplemented("store.insert_alert")
    }
    fn acknowledge_alert(&self, _: &Actor, _: &str, _: UnixMillis) -> Result<bool, StoreError> {
        unimplemented("store.acknowledge_alert")
    }
}

type ChainKey = (String, u64);

#[derive(Debug, Default)]
struct Tables {
    host_writes: Vec<HostWriteRecord>,
    windows: BTreeMap<String, TrafficWindowRecord>,
    chains: BTreeMap<ChainKey, ChainRecord>,
    batches: BTreeMap<(String, u64, String), BatchRecord>,
    findings: Vec<FindingRecord>,
    xp_summaries: BTreeMap<(String, String), XpSummaryRecord>,
    alerts: BTreeMap<String, AlertRecord>,
}

/// Guard's tables in memory, serial behind one lock: each method is one transaction.
#[derive(Debug, Default)]
pub struct MemoryGuardStore {
    tables: Mutex<Tables>,
}

impl MemoryGuardStore {
    pub fn host_writes(&self) -> Vec<HostWriteRecord> {
        lock(&self.tables).host_writes.clone()
    }

    pub fn windows(&self) -> Vec<TrafficWindowRecord> {
        lock(&self.tables).windows.values().cloned().collect()
    }

    pub fn alert(&self, alert_id: &str) -> Option<AlertRecord> {
        lock(&self.tables).alerts.get(alert_id).cloned()
    }
}

fn chain_key(session: &SessionId, generation: u64) -> ChainKey {
    (session.to_string(), generation)
}

/// `ReserveWindow` on one window: charged when a request and the bytes are left.
fn reserve(windows: &mut BTreeMap<String, TrafficWindowRecord>, reservation: &TrafficReservation) -> bool {
    let window = windows
        .entry(reservation.window_id.clone())
        .or_insert_with(|| TrafficWindowRecord {
            window_id: reservation.window_id.clone(),
            scope: reservation.scope,
            window_start: reservation.window_start,
            requests: 0,
            bytes: 0,
            expires_at: reservation.expires_at,
        });
    let accepted = window.requests < reservation.max_requests
        && window.bytes.saturating_add(reservation.bytes) <= reservation.max_bytes;
    if accepted {
        window.requests += 1;
        window.bytes += reservation.bytes;
    }
    accepted
}

impl GuardStore for MemoryGuardStore {
    fn record_host_write(&self, _: &Actor, write: &HostWriteRecord) -> Result<(), StoreError> {
        lock(&self.tables).host_writes.push(write.clone());
        Ok(())
    }

    fn reserve_traffic(
        &self,
        _: &Actor,
        host: &TrafficReservation,
        session: &TrafficReservation,
    ) -> Result<bool, StoreError> {
        let mut tables = lock(&self.tables);
        let now = host.window_start;
        tables.windows.retain(|_, window| window.expires_at > now);
        // Work on a copy so a refusal rolls both windows back, as the aborted transaction does
        let mut windows = tables.windows.clone();
        let accepted = reserve(&mut windows, host) && reserve(&mut windows, session);
        if accepted {
            tables.windows = windows;
        }
        Ok(accepted)
    }

    fn chain(&self, session: &SessionId, generation: u64) -> Result<Option<ChainRecord>, StoreError> {
        Ok(lock(&self.tables).chains.get(&chain_key(session, generation)).cloned())
    }

    fn apply_chain(&self, _: &Actor, update: &ChainUpdate, expected_revision: u64) -> Result<bool, StoreError> {
        let mut tables = lock(&self.tables);
        let key = chain_key(&update.chain.session, update.chain.generation);
        let stored = tables.chains.get(&key).map_or(0, |chain| chain.revision);
        if stored != expected_revision {
            return Ok(false);
        }
        if let Some(batch) = &update.batch {
            let batch_key = (key.0.clone(), key.1, batch.batch_id.clone());
            if tables.batches.contains_key(&batch_key) {
                return Ok(false);
            }
            tables.batches.insert(batch_key, batch.clone());
        }
        tables.findings.extend(update.findings.iter().cloned());
        tables.chains.insert(key, update.chain.clone());
        Ok(true)
    }

    fn batch(&self, session: &SessionId, generation: u64, batch_id: &str) -> Result<Option<BatchRecord>, StoreError> {
        let key = (session.to_string(), generation, batch_id.to_owned());
        Ok(lock(&self.tables).batches.get(&key).cloned())
    }

    fn findings(&self, session: &SessionId, generation: u64) -> Result<Vec<FindingRecord>, StoreError> {
        Ok(lock(&self.tables)
            .findings
            .iter()
            .filter(|finding| &finding.session == session && finding.generation == generation)
            .cloned()
            .collect())
    }

    fn put_xp_summary(&self, _: &Actor, summary: &XpSummaryRecord) -> Result<(), StoreError> {
        let key = (summary.session.to_string(), summary.member.0.clone());
        lock(&self.tables).xp_summaries.insert(key, summary.clone());
        Ok(())
    }

    fn insert_alert(&self, _: &Actor, alert: &AlertRecord) -> Result<(), StoreError> {
        // $setOnInsert: a retried enqueue keeps the first alert
        lock(&self.tables)
            .alerts
            .entry(alert.alert_id.clone())
            .or_insert_with(|| alert.clone());
        Ok(())
    }

    fn acknowledge_alert(&self, actor: &Actor, alert_id: &str, at: UnixMillis) -> Result<bool, StoreError> {
        let mut tables = lock(&self.tables);
        let Some(alert) = tables.alerts.get_mut(alert_id) else {
            return Ok(false);
        };
        if alert.acknowledged_at.is_some() {
            return Ok(false);
        }
        alert.acknowledged_at = Some(at);
        alert.acknowledged_by = Some(match actor {
            Actor::Admin { key_id } => key_id.clone(),
            Actor::Account(account) => account.0.clone(),
            Actor::HostRuntime { host, .. } => host.0.clone(),
            Actor::System(name) => (*name).to_owned(),
        });
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reservation(id: &str, scope: TrafficScope, max_requests: u64) -> TrafficReservation {
        TrafficReservation {
            window_id: id.to_owned(),
            scope,
            window_start: UnixMillis(60_000),
            expires_at: UnixMillis(180_000),
            bytes: 1,
            max_requests,
            max_bytes: 10,
        }
    }

    #[test]
    fn stub_store_refuses() {
        let error = StubGuardStore
            .acknowledge_alert(&Actor::System("test"), "a", UnixMillis(0))
            .unwrap_err();
        assert_eq!(
            error,
            StoreError::Unimplemented(Unimplemented::new(Component::Guard, "store.acknowledge_alert"))
        );
    }

    // guardTrafficDistributed.integration.test.ts: "actual distributed Guard implementation rolls back a rejected
    // session reservation", on the in-memory store
    #[test]
    fn a_rejected_session_reservation_rolls_the_host_back() {
        let store = MemoryGuardStore::default();
        let actor = Actor::System("test");
        let host = reservation("host-a", TrafficScope::Host, 2);
        let session_a = reservation("session-a", TrafficScope::Session, 1);
        let session_b = reservation("session-b", TrafficScope::Session, 1);
        assert!(store.reserve_traffic(&actor, &host, &session_a).unwrap());
        assert!(!store.reserve_traffic(&actor, &host, &session_a).unwrap());
        assert!(store.reserve_traffic(&actor, &host, &session_b).unwrap());
        let windows = store.windows();
        let host_window = windows
            .iter()
            .find(|window| window.scope == TrafficScope::Host)
            .unwrap();
        assert_eq!((host_window.requests, host_window.bytes), (2, 2));
        assert_eq!(
            windows
                .iter()
                .filter(|window| window.scope == TrafficScope::Session)
                .count(),
            2
        );
    }

    #[test]
    fn alerts_are_acknowledged_once() {
        let store = MemoryGuardStore::default();
        let actor = Actor::Admin {
            key_id: "admin-1".to_owned(),
        };
        let alert = AlertRecord {
            alert_id: "alert-0001".to_owned(),
            code: "REWARD_INVARIANT".to_owned(),
            severity: "critical".to_owned(),
            message: "m".to_owned(),
            details: Json::empty_object(),
            created_at: UnixMillis(1),
            acknowledged_at: None,
            acknowledged_by: None,
        };
        store.insert_alert(&actor, &alert).unwrap();
        assert!(store.acknowledge_alert(&actor, "alert-0001", UnixMillis(5)).unwrap());
        assert!(!store.acknowledge_alert(&actor, "alert-0001", UnixMillis(6)).unwrap());
        assert!(!store.acknowledge_alert(&actor, "missing", UnixMillis(6)).unwrap());
        let stored = store.alert("alert-0001").unwrap();
        assert_eq!(stored.acknowledged_at, Some(UnixMillis(5)));
        assert_eq!(stored.acknowledged_by.as_deref(), Some("admin-1"));
    }
}
