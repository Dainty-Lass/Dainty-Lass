// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What Guard keeps, the store it keeps it in, and two stores without a database.

use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard, PoisonError},
};

use rifts_seam::{
    AccountId, Actor, Component, HostWriteKind, Json, RealmId, SessionId, StoreError, TableSpec, Unimplemented,
    UnixMillis,
};

/// The chain head before the first batch.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The summaries' lifetime: 400 days.
pub const XP_SUMMARY_TTL_MS: u64 = 400 * 24 * 60 * 60 * 1000;

const fn table(name: &'static str, node_collection: &'static str, keys: &'static str) -> TableSpec {
    TableSpec {
        name,
        node_collection,
        owner: Component::Guard,
        keys,
        expires_by: None,
    }
}

/// A table whose rows a sweeper drops once `column` has passed.
const fn swept(spec: TableSpec, column: &'static str) -> TableSpec {
    TableSpec {
        expires_by: Some(column),
        ..spec
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
    swept(
        table(
            "rifts_traffic_windows",
            "guardTrafficWindows",
            "window id (scope, window start, hashed key)",
        ),
        "expires_at",
    ),
    table(
        "rifts_host_writes",
        "p2pHostWrites",
        "session + generation; target, host, realm by at",
    ),
    swept(
        table("rifts_hunt_xp_summaries", "p2pHuntXpSummaries", "session + member"),
        "ttl_at",
    ),
    table(
        "rifts_operations_alerts",
        "operationsAlerts",
        "alert; status + created_at",
    ),
];

// Each stored enum's text, both ways, from one list.
macro_rules! stored_as {
    ($name:ident: $($variant:ident = $text:literal),+ $(,)?) => {
        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }

            pub fn parse(text: &str) -> Option<Self> {
                [$(Self::$variant),+].into_iter().find(|value| value.as_str() == text)
            }
        }
    };
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CompletionMode {
    FirstDefeat,
    EndOfHunt,
    NoCompletion,
}

stored_as!(CompletionMode: FirstDefeat = "FIRST_DEFEAT", EndOfHunt = "END_OF_HUNT", NoCompletion = "NO_COMPLETION");

/// Where a hunt's encounter is; it only moves forward.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
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

stored_as!(EncounterState:
    Created = "CREATED",
    SessionStarted = "SESSION_STARTED",
    WorldReady = "WORLD_READY",
    EncounterActive = "ENCOUNTER_ACTIVE",
    BehemothDefeated = "BEHEMOTH_DEFEATED",
    CompletionRequested = "COMPLETION_REQUESTED",
    Aborted = "ABORTED",
);

impl EncounterState {
    /// In the order an encounter moves through them.
    pub const ALL: [Self; 7] = {
        use EncounterState::*;
        [
            Created,
            SessionStarted,
            WorldReady,
            EncounterActive,
            BehemothDefeated,
            CompletionRequested,
            Aborted,
        ]
    };

    pub fn is_terminal(self) -> bool {
        self >= Self::CompletionRequested
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Disposition {
    #[default]
    ObserveClear,
    ObserveFlagged,
}

stored_as!(Disposition: ObserveClear = "OBSERVE_CLEAR", ObserveFlagged = "OBSERVE_FLAGGED");

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TrafficScope {
    Host,
    Session,
}

stored_as!(TrafficScope: Host = "host", Session = "session");

/// One player-state write a hosted runtime made, as Guard records it.
#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrafficWindowRecord {
    pub window_id: String,
    pub scope: TrafficScope,
    pub window_start: UnixMillis,
    pub requests: u64,
    pub bytes: u64,
    pub expires_at: UnixMillis,
}

/// One key's charge against one window; the key itself is only in the id's hash.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrafficReservation {
    pub window_id: String,
    pub scope: TrafficScope,
    pub window_start: UnixMillis,
    pub expires_at: UnixMillis,
    pub bytes: u64,
    pub max_requests: u64,
    pub max_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionRecord {
    pub final_sequence: u64,
    pub final_hash: String,
    pub at: UnixMillis,
}

/// One hunt generation's evidence chain.
#[derive(Clone, Debug, Eq, PartialEq)]
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
    /// A chain at genesis: no batch, no finding, nothing counted.
    pub fn new(session: SessionId, generation: u64, completion_mode: Option<CompletionMode>) -> Self {
        Self {
            session,
            generation,
            completion_mode,
            head_hash: GENESIS_HASH.into(),
            encounter_state: EncounterState::default(),
            disposition: Disposition::default(),
            event_types: BTreeMap::default(),
            has_unattributed_damage: false,
            revision: 0,
            last_sequence: 0,
            finding_count: 0,
            batch_count: 0,
            event_count: 0,
            event_bytes: 0,
            last_monotonic_ms: None,
            last_received_at: None,
            completion: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FindingRecord {
    pub session: SessionId,
    pub generation: u64,
    pub sequence: Option<u64>,
    pub code: String,
    pub detail: String,
    pub at: UnixMillis,
}

/// A chain write: the new chain, the batch that moved it (if any) and the findings, stored together.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainUpdate {
    pub chain: ChainRecord,
    pub batch: Option<BatchRecord>,
    pub findings: Vec<FindingRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
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
    // Evidence chains.
    fn chain(&self, session: &SessionId, generation: u64) -> Result<Option<ChainRecord>, StoreError>;
    /// `false`, writing nothing, when the stored revision is not `expected_revision` or the batch id is taken.
    fn apply_chain(&self, actor: &Actor, update: &ChainUpdate, expected_revision: u64) -> Result<bool, StoreError>;
    fn batch(&self, session: &SessionId, generation: u64, batch_id: &str) -> Result<Option<BatchRecord>, StoreError>;
    fn findings(&self, session: &SessionId, generation: u64) -> Result<Vec<FindingRecord>, StoreError>;

    // Traffic and host writes.
    /// Both windows are charged, or neither.
    fn reserve_traffic(
        &self,
        actor: &Actor,
        host: &TrafficReservation,
        session: &TrafficReservation,
    ) -> Result<bool, StoreError>;
    fn record_host_write(&self, actor: &Actor, write: &HostWriteRecord) -> Result<(), StoreError>;
    fn put_xp_summary(&self, actor: &Actor, summary: &XpSummaryRecord) -> Result<(), StoreError>;

    // Operator alerts.
    fn insert_alert(&self, actor: &Actor, alert: &AlertRecord) -> Result<(), StoreError>;
    fn acknowledge_alert(&self, actor: &Actor, alert_id: &str, at: UnixMillis) -> Result<bool, StoreError>;
}

/// Who acknowledged: the name the actor goes by.
pub(crate) fn actor_name(actor: &Actor) -> String {
    let name: &str = match actor {
        Actor::Admin { key_id } => key_id,
        Actor::Account(account) | Actor::HostRuntime { host: account, .. } => &account.0,
        Actor::System(task) => task,
    };
    name.to_owned()
}

/// A window after one more request of `bytes`, if it stays within the reservation's limits.
pub(crate) fn charged(requests: u64, bytes: u64, reservation: &TrafficReservation) -> Option<(u64, u64)> {
    if requests >= reservation.max_requests {
        return None;
    }
    let total = bytes
        .checked_add(reservation.bytes)
        .filter(|total| *total <= reservation.max_bytes)?;
    Some((requests + 1, total))
}

type ChainKey = (SessionId, u64);

#[derive(Debug, Default)]
struct Tables {
    host_writes: Vec<HostWriteRecord>,
    windows: BTreeMap<String, TrafficWindowRecord>,
    chains: BTreeMap<ChainKey, ChainRecord>,
    batches: BTreeMap<(ChainKey, String), BatchRecord>,
    findings: Vec<FindingRecord>,
    summaries: BTreeMap<(SessionId, AccountId), (XpSummaryRecord, UnixMillis)>,
    alerts: BTreeMap<String, AlertRecord>,
}

impl Tables {
    fn window_counts(&self, id: &str) -> (u64, u64) {
        self.windows
            .get(id)
            .map_or((0, 0), |window| (window.requests, window.bytes))
    }

    fn set_window(&mut self, reservation: &TrafficReservation, (requests, bytes): (u64, u64)) {
        let window = self
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
        (window.requests, window.bytes) = (requests, bytes);
    }
}

/// The store's semantics behind one lock.
#[derive(Debug, Default)]
pub struct MemoryGuardStore {
    inner: Mutex<Tables>,
}

impl MemoryGuardStore {
    fn tables(&self) -> MutexGuard<'_, Tables> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn host_writes(&self) -> Vec<HostWriteRecord> {
        self.tables().host_writes.clone()
    }

    pub fn windows(&self) -> Vec<TrafficWindowRecord> {
        let tables = self.tables();
        tables.windows.values().cloned().collect()
    }

    pub fn alert(&self, alert_id: &str) -> Option<AlertRecord> {
        let tables = self.tables();
        tables.alerts.get(alert_id).cloned()
    }
}

impl GuardStore for MemoryGuardStore {
    fn chain(&self, session: &SessionId, generation: u64) -> Result<Option<ChainRecord>, StoreError> {
        let key = (session.clone(), generation);
        Ok(self.tables().chains.get(&key).cloned())
    }

    fn apply_chain(&self, _: &Actor, update: &ChainUpdate, expected_revision: u64) -> Result<bool, StoreError> {
        let mut tables = self.tables();
        let key = (update.chain.session.clone(), update.chain.generation);
        let stored_revision = tables.chains.get(&key).map(|chain| chain.revision).unwrap_or_default();
        let batch_key = update
            .batch
            .as_ref()
            .map(|batch| ((batch.session.clone(), batch.generation), batch.batch_id.clone()));
        let batch_taken = batch_key.as_ref().is_some_and(|key| tables.batches.contains_key(key));
        if stored_revision != expected_revision || batch_taken {
            return Ok(false);
        }
        if let (Some(key), Some(batch)) = (batch_key, &update.batch) {
            tables.batches.insert(key, batch.clone());
        }
        tables.chains.insert(key, update.chain.clone());
        tables.findings.extend_from_slice(&update.findings);
        Ok(true)
    }

    fn batch(&self, session: &SessionId, generation: u64, batch_id: &str) -> Result<Option<BatchRecord>, StoreError> {
        let key = ((session.clone(), generation), batch_id.to_owned());
        Ok(self.tables().batches.get(&key).cloned())
    }

    fn findings(&self, session: &SessionId, generation: u64) -> Result<Vec<FindingRecord>, StoreError> {
        let tables = self.tables();
        let of_chain = tables
            .findings
            .iter()
            .filter(|f| f.generation == generation && f.session == *session);
        Ok(of_chain.cloned().collect())
    }

    fn reserve_traffic(
        &self,
        _: &Actor,
        host: &TrafficReservation,
        session: &TrafficReservation,
    ) -> Result<bool, StoreError> {
        let mut tables = self.tables();
        tables.windows.retain(|_, window| window.expires_at > host.window_start);
        let (requests, bytes) = tables.window_counts(&host.window_id);
        let Some(host_after) = charged(requests, bytes, host) else {
            return Ok(false);
        };
        // One window for both scopes sees the host's charge first.
        let (requests, bytes) = if session.window_id == host.window_id {
            host_after
        } else {
            tables.window_counts(&session.window_id)
        };
        let Some(session_after) = charged(requests, bytes, session) else {
            return Ok(false);
        };
        tables.set_window(host, host_after);
        tables.set_window(session, session_after);
        Ok(true)
    }

    fn record_host_write(&self, _: &Actor, write: &HostWriteRecord) -> Result<(), StoreError> {
        self.tables().host_writes.push(write.clone());
        Ok(())
    }

    fn put_xp_summary(&self, _: &Actor, summary: &XpSummaryRecord) -> Result<(), StoreError> {
        let expires = UnixMillis(summary.created_at.0.saturating_add_unsigned(XP_SUMMARY_TTL_MS));
        let key = (summary.session.clone(), summary.member.clone());
        self.tables().summaries.insert(key, (summary.clone(), expires));
        Ok(())
    }

    fn insert_alert(&self, _: &Actor, alert: &AlertRecord) -> Result<(), StoreError> {
        // The first alert under an id is kept; a repeat changes nothing.
        let mut tables = self.tables();
        if !tables.alerts.contains_key(&alert.alert_id) {
            tables.alerts.insert(alert.alert_id.clone(), alert.clone());
        }
        Ok(())
    }

    fn acknowledge_alert(&self, actor: &Actor, alert_id: &str, at: UnixMillis) -> Result<bool, StoreError> {
        let mut tables = self.tables();
        let Some(alert) = tables
            .alerts
            .get_mut(alert_id)
            .filter(|alert| alert.acknowledged_at.is_none())
        else {
            return Ok(false);
        };
        alert.acknowledged_at = Some(at);
        alert.acknowledged_by = Some(actor_name(actor));
        Ok(true)
    }
}

/// No database: every call is `Unimplemented` as `store.<method>`.
#[derive(Clone, Copy, Debug, Default)]
pub struct StubGuardStore;

// Each method refuses as `store.<its name>`.
macro_rules! no_database {
    ($($name:ident($($arg:ty),*) -> $out:ty;)*) => {$(
        fn $name(&self, $(_: $arg),*) -> Result<$out, StoreError> {
            Err(Unimplemented::new(Component::Guard, concat!("store.", stringify!($name))).into())
        }
    )*};
}

impl GuardStore for StubGuardStore {
    no_database! {
        chain(&SessionId, u64) -> Option<ChainRecord>;
        apply_chain(&Actor, &ChainUpdate, u64) -> bool;
        batch(&SessionId, u64, &str) -> Option<BatchRecord>;
        findings(&SessionId, u64) -> Vec<FindingRecord>;
        reserve_traffic(&Actor, &TrafficReservation, &TrafficReservation) -> bool;
        record_host_write(&Actor, &HostWriteRecord) -> ();
        put_xp_summary(&Actor, &XpSummaryRecord) -> ();
        insert_alert(&Actor, &AlertRecord) -> ();
        acknowledge_alert(&Actor, &str, UnixMillis) -> bool;
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

    const SYSTEM: Actor = Actor::System("guard");

    #[test]
    fn enum_texts() {
        let names = EncounterState::ALL.map(EncounterState::as_str);
        let want = [
            "CREATED",
            "SESSION_STARTED",
            "WORLD_READY",
            "ENCOUNTER_ACTIVE",
            "BEHEMOTH_DEFEATED",
            "COMPLETION_REQUESTED",
            "ABORTED",
        ];
        assert_eq!(names, want);
        for (state, text) in EncounterState::ALL.into_iter().zip(want) {
            assert_eq!(EncounterState::parse(text), Some(state));
        }
        assert!(EncounterState::Aborted.is_terminal());
        assert!(!EncounterState::BehemothDefeated.is_terminal());
        let modes = [
            ("NO_COMPLETION", Some(CompletionMode::NoCompletion)),
            ("no_completion", None),
        ];
        for (text, mode) in modes {
            assert_eq!(CompletionMode::parse(text), mode);
        }
        assert_eq!(Disposition::default().as_str(), "OBSERVE_CLEAR");
        assert_eq!(TrafficScope::Session.as_str(), "session");
        assert_eq!(GENESIS_HASH.len(), 64);
        assert_eq!(XP_SUMMARY_TTL_MS, 34_560_000_000);
    }

    #[test]
    fn memory_traffic_pair_is_all_or_nothing() {
        let store = MemoryGuardStore::default();
        let host = reservation("host:a", TrafficScope::Host, 5);
        let tight = reservation("session:b", TrafficScope::Session, 1);
        let attempts = [true, false].map(|_| store.reserve_traffic(&SYSTEM, &host, &tight));
        assert_eq!(attempts, [Ok(true), Ok(false)]);
        let requests: Vec<u64> = store.windows().iter().map(|w| w.requests).collect();
        assert_eq!(requests, [1, 1]);

        // The same id sees its own first charge.
        let shared = MemoryGuardStore::default();
        let both = reservation("x", TrafficScope::Host, 2);
        assert_eq!(shared.reserve_traffic(&SYSTEM, &both, &both), Ok(true));
        assert_eq!(shared.windows()[0].requests, 2);
        assert_eq!(shared.reserve_traffic(&SYSTEM, &both, &both), Ok(false));
    }

    #[test]
    fn memory_chain_revisions_and_batch_ids() {
        let store = MemoryGuardStore::default();
        let chain = ChainRecord {
            revision: 1,
            ..ChainRecord::new(session(), 1, None)
        };
        let update = ChainUpdate {
            chain: chain.clone(),
            batch: None,
            findings: vec![],
        };
        let answers = [1, 0, 0].map(|expected| store.apply_chain(&SYSTEM, &update, expected));
        assert_eq!(answers, [Ok(false), Ok(true), Ok(false)]);
        assert_eq!(store.chain(&session(), 1), Ok(Some(chain)));
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
        let tries = [
            ("alert-0001", 9, true),
            ("alert-0001", 10, false),
            ("missing", 10, false),
        ];
        for (id, at, acknowledged) in tries {
            assert_eq!(
                store.acknowledge_alert(&admin, id, UnixMillis(at)),
                Ok(acknowledged),
                "{id} {at}"
            );
        }
        let stored = store.alert("alert-0001").unwrap();
        assert_eq!(stored.acknowledged_at, Some(UnixMillis(9)));
        assert_eq!(stored.acknowledged_by.as_deref(), Some("ops"));
    }

    #[test]
    fn stub_store_names_each_method() {
        let missing = Unimplemented::new(Component::Guard, "store.chain");
        assert_eq!(
            StubGuardStore.chain(&session(), 1),
            Err(StoreError::Unimplemented(missing))
        );
    }

    #[test]
    fn tables_in_order() {
        let expected = [
            ("rifts_hunt_guard", None),
            ("rifts_guard_batches", None),
            ("rifts_guard_findings", None),
            ("rifts_traffic_windows", Some("expires_at")),
            ("rifts_host_writes", None),
            ("rifts_hunt_xp_summaries", Some("ttl_at")),
            ("rifts_operations_alerts", None),
        ];
        let listed: Vec<(&str, Option<&str>)> = TABLES.iter().map(|t| (t.name, t.expires_by)).collect();
        assert_eq!(listed, expected);
        assert!(TABLES.iter().all(|t| t.owner == Component::Guard));
    }
}
