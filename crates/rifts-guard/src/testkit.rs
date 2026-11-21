// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Fixtures for the unit tests: one hunt and one hub, fake hunts and core, and a Guard over memory.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rifts_seam::{
    AccountId, BoxFuture, Candidate, CapabilityScope, GuardMode, HubKind, InventoryTransaction, IslesCore, Json, Party,
    PartyId, RuntimeAuth, SeamError, SessionId, StoreError, UnixMillis,
};

use crate::ports::{CapabilityView, HuntSessions, HuntView, ManualClock, MemoryLog, MemoryOperations, RosterEntry};
use crate::store::{GuardStore, MemoryGuardStore};
use crate::{Guard, GuardConfig, GuardParts};

pub const SESSION: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV@eu-1";
pub const HUB_SESSION: &str = "01BX5ZZKBKACTAV9WEVGEMMVRZ@eu-1";
pub const REALM: &str = "01BX5ZZKBKACTAV9WEVGEMMVS0@eu-1";
/// A capability token's shape: 43 characters of base64url.
pub const TOKEN: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123-_x";
pub const NOW: i64 = 1_792_000_000_000;

pub fn account(name: &str) -> AccountId {
    AccountId(name.to_owned())
}

pub fn session() -> SessionId {
    SESSION.parse().unwrap()
}

pub fn runtime() -> RuntimeAuth {
    RuntimeAuth {
        capability_id: "cap-1".into(),
        session: session(),
        generation: 1,
        host_account: account("host"),
        scopes: vec![CapabilityScope::EventsWrite, CapabilityScope::CompleteRequest],
        roster: Vec::new(),
        host_peer: None,
        bridge_instance: None,
        wire_session: None,
    }
}

/// A running hunt: the host and `member` admitted, `pending` not; only `member` froze a character.
pub fn hunt() -> HuntView {
    HuntView {
        session: session(),
        generation: 1,
        state: "RUNNING".into(),
        pre_suspect_state: None,
        host: Some(account("host")),
        realm: None,
        hub: None,
        roster: ["host", "member", "pending"]
            .iter()
            .map(|name| RosterEntry {
                account: account(name),
                admitted: *name != "pending",
            })
            .collect(),
        frozen_characters: vec![(account("member"), "char-m".into())],
        completion_mode: None,
    }
}

pub fn hub(kind: HubKind) -> HuntView {
    HuntView {
        session: session(),
        realm: Some(REALM.parse().unwrap()),
        hub: Some(kind),
        roster: Vec::new(),
        frozen_characters: Vec::new(),
        ..hunt()
    }
}

#[derive(Default)]
pub struct FakeHunts {
    pub hunts: Mutex<HashMap<SessionId, HuntView>>,
    pub presence: Mutex<Vec<AccountId>>,
    pub capabilities: Mutex<HashMap<String, CapabilityView>>,
    pub runtimes: Mutex<HashMap<String, RuntimeAuth>>,
    pub lookups: AtomicUsize,
    pub failing: AtomicBool,
}

impl FakeHunts {
    pub fn put(&self, hunt: HuntView) {
        self.hunts.lock().unwrap().insert(hunt.session.clone(), hunt);
    }

    pub fn set_presence(&self, accounts: &[&str]) {
        *self.presence.lock().unwrap() = accounts.iter().map(|a| account(a)).collect();
    }

    fn check(&self) -> Result<(), StoreError> {
        if self.failing.load(Ordering::SeqCst) {
            Err(StoreError::Database("hunts are down".into()))
        } else {
            Ok(())
        }
    }
}

impl HuntSessions for FakeHunts {
    fn hunt(&self, session: &SessionId) -> Result<Option<HuntView>, StoreError> {
        self.check()?;
        Ok(self.hunts.lock().unwrap().get(session).cloned())
    }

    fn hub_present_accounts(&self, _session: &SessionId, _generation: u64) -> Result<Vec<AccountId>, StoreError> {
        self.check()?;
        Ok(self.presence.lock().unwrap().clone())
    }

    fn capability_by_digest(&self, digest: &str) -> Result<Option<CapabilityView>, StoreError> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        self.check()?;
        Ok(self.capabilities.lock().unwrap().get(digest).cloned())
    }

    fn resolve_capability(
        &self,
        digest: &str,
        session_id: &str,
        _scope: CapabilityScope,
        _now: UnixMillis,
    ) -> Result<Option<RuntimeAuth>, StoreError> {
        self.check()?;
        Ok(self
            .runtimes
            .lock()
            .unwrap()
            .get(digest)
            .filter(|r| r.session.to_string() == session_id)
            .cloned())
    }
}

type Fut<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

fn ready<T: Send + 'static>(value: Result<T, SeamError>) -> Fut<'static, T> {
    Box::pin(std::future::ready(value))
}

fn unused<T: Send + 'static>(name: &'static str) -> Fut<'static, T> {
    rifts_seam::stub(rifts_seam::Component::Isles, name)
}

/// The parts of isles Guard asks: stacked items, replays, the policy and its step-up.
pub struct FakeCore {
    pub stacked: Mutex<Result<Option<Json>, SeamError>>,
    pub replay: AtomicBool,
    pub policy: Mutex<Result<Json, SeamError>>,
    pub step_up: AtomicBool,
}

impl Default for FakeCore {
    fn default() -> Self {
        Self {
            stacked: Mutex::new(Ok(None)),
            replay: AtomicBool::new(false),
            policy: Mutex::new(Err(SeamError::unimplemented(
                rifts_seam::Component::Isles,
                "operations_policy",
            ))),
            step_up: AtomicBool::new(false),
        }
    }
}

impl IslesCore for FakeCore {
    fn display_name(&self, _account: AccountId) -> Fut<'_, Option<String>> {
        unused("display_name")
    }

    fn find_by_display_name(&self, _name: String) -> Fut<'_, Option<AccountId>> {
        unused("find_by_display_name")
    }

    fn launcher_account_eligible(&self, _account: AccountId) -> Fut<'_, bool> {
        unused("launcher_account_eligible")
    }

    fn party(&self, _party: PartyId) -> Fut<'_, Option<Party>> {
        unused("party")
    }

    fn get_or_create_party(&self, _account: AccountId) -> Fut<'_, Party> {
        unused("get_or_create_party")
    }

    fn set_candidate(&self, _account: AccountId, _candidate: Candidate) -> Result<(), SeamError> {
        Ok(())
    }

    fn cancel_travel(&self, _hunt_id: &str, _accounts: &[AccountId], _reason: &str) -> Result<bool, SeamError> {
        Ok(false)
    }

    fn dedicated_servers_configured(&self) -> bool {
        false
    }

    fn touch_game_session(&self, _account: AccountId) -> Fut<'_, ()> {
        unused("touch_game_session")
    }

    fn record_hunt_activity(&self, _session: SessionId, _activity: Json) -> Fut<'_, ()> {
        unused("record_hunt_activity")
    }

    fn healthy_guard_session(&self, _account: AccountId) -> Fut<'_, Option<Json>> {
        unused("healthy_guard_session")
    }

    fn published_guard_artifacts(&self) -> Fut<'_, Json> {
        unused("published_guard_artifacts")
    }

    fn guard_mode(&self) -> GuardMode {
        GuardMode::Observe
    }

    fn operations_policy(&self) -> Fut<'_, Json> {
        ready(self.policy.lock().unwrap().clone())
    }

    fn verify_operations_step_up(&self, _key_id: String, _step_up: String) -> Fut<'_, bool> {
        ready(Ok(self.step_up.load(Ordering::SeqCst)))
    }

    fn stacked_items(&self, _character_id: String) -> Fut<'_, Option<Json>> {
        ready(self.stacked.lock().unwrap().clone())
    }

    fn accepted_inventory_replay(&self, _transaction: InventoryTransaction) -> Fut<'_, bool> {
        ready(Ok(self.replay.load(Ordering::SeqCst)))
    }
}

pub struct Kit {
    pub guard: Arc<Guard>,
    pub store: Arc<MemoryGuardStore>,
    pub hunts: Arc<FakeHunts>,
    pub core: Arc<FakeCore>,
    pub operations: Arc<MemoryOperations>,
    pub clock: Arc<ManualClock>,
    pub log: Arc<MemoryLog>,
}

impl Kit {
    pub fn lines(&self) -> Vec<String> {
        self.log.lines()
    }
}

pub fn settings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

/// Guard over memory with these settings, the hunt of `hunt()` in place.
pub fn kit(pairs: &[(&str, &str)]) -> Kit {
    kit_on(pairs, None)
}

/// The same over another store (the memory one still built, but unused).
pub fn kit_on(pairs: &[(&str, &str)], store: Option<Arc<dyn GuardStore>>) -> Kit {
    let memory = Arc::new(MemoryGuardStore::default());
    let hunts = Arc::new(FakeHunts::default());
    hunts.put(hunt());
    let core = Arc::new(FakeCore::default());
    let operations = Arc::new(MemoryOperations::default());
    let clock = Arc::new(ManualClock::at(UnixMillis(NOW)));
    let log = Arc::new(MemoryLog::default());
    let guard = Guard::new(GuardParts {
        config: GuardConfig::from_settings(&settings(pairs)),
        store: store.unwrap_or_else(|| memory.clone()),
        hunts: hunts.clone(),
        operations: operations.clone(),
        core: core.clone(),
        clock: clock.clone(),
        log: log.clone(),
    });
    Kit {
        guard: Arc::new(guard),
        store: memory,
        hunts,
        core,
        operations,
        clock,
        log,
    }
}
