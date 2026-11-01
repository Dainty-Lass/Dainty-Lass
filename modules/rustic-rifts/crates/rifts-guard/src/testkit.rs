//! A Guard on in-memory parts, with a hunt table the tests edit.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rifts_seam::{
    AccountId, BoxFuture, Component, NoCore, SeamError, SessionId, StoreError, Unimplemented, UnixMillis,
};

use crate::ports::lock;
use crate::{
    CapabilityView, Guard, GuardConfig, GuardParts, HuntSessions, HuntView, ManualClock, MemoryGuardStore, MemoryLog,
    MemoryOperations,
};

#[derive(Debug, Default)]
pub struct FakeHunts {
    pub hunts: Mutex<BTreeMap<String, HuntView>>,
    pub present: Mutex<Vec<AccountId>>,
    pub capabilities: Mutex<BTreeMap<String, CapabilityView>>,
    pub capability_lookups: Mutex<u32>,
    pub failing: Mutex<bool>,
}

impl FakeHunts {
    pub fn put(&self, hunt: HuntView) {
        lock(&self.hunts).insert(hunt.session.to_string(), hunt);
    }
}

fn down() -> StoreError {
    Unimplemented::new(Component::Realms, "store down").into()
}

impl HuntSessions for FakeHunts {
    fn hunt(&self, session: &SessionId) -> Result<Option<HuntView>, StoreError> {
        Ok(lock(&self.hunts).get(&session.to_string()).cloned())
    }
    fn hub_present_accounts(&self, _: &SessionId, _: u64) -> Result<Vec<AccountId>, StoreError> {
        Ok(lock(&self.present).clone())
    }
    fn capability_by_digest(&self, digest: &str) -> Result<Option<CapabilityView>, StoreError> {
        *lock(&self.capability_lookups) += 1;
        if *lock(&self.failing) {
            return Err(down());
        }
        Ok(lock(&self.capabilities).get(digest).cloned())
    }
}

/// An isles that verifies one step-up code and holds one character's inventory.
pub struct StepUpCore {
    /// The stored `stackedItems`; `Err` is a read that failed.
    pub stacked_items: Mutex<Result<Option<String>, ()>>,
    pub replay: Mutex<bool>,
}

impl rifts_seam::IslesCore for StepUpCore {
    fn display_name(&self, account: AccountId) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        NoCore.display_name(account)
    }
    fn find_by_display_name(&self, name: String) -> BoxFuture<'_, Result<Option<AccountId>, SeamError>> {
        NoCore.find_by_display_name(name)
    }
    fn launcher_account_eligible(&self, account: AccountId) -> BoxFuture<'_, Result<bool, SeamError>> {
        NoCore.launcher_account_eligible(account)
    }
    fn party(&self, party: rifts_seam::PartyId) -> BoxFuture<'_, Result<Option<rifts_seam::Party>, SeamError>> {
        NoCore.party(party)
    }
    fn get_or_create_party(&self, account: AccountId) -> BoxFuture<'_, Result<rifts_seam::Party, SeamError>> {
        NoCore.get_or_create_party(account)
    }
    fn set_candidate(&self, account: AccountId, candidate: rifts_seam::Candidate) -> Result<(), SeamError> {
        NoCore.set_candidate(account, candidate)
    }
    fn cancel_travel(&self, hunt_id: &str, accounts: &[AccountId], reason: &str) -> Result<bool, SeamError> {
        NoCore.cancel_travel(hunt_id, accounts, reason)
    }
    fn dedicated_servers_configured(&self) -> bool {
        false
    }
    fn touch_game_session(&self, account: AccountId) -> BoxFuture<'_, Result<(), SeamError>> {
        NoCore.touch_game_session(account)
    }
    fn record_hunt_activity(
        &self,
        session: SessionId,
        activity: rifts_seam::Json,
    ) -> BoxFuture<'_, Result<(), SeamError>> {
        NoCore.record_hunt_activity(session, activity)
    }
    fn healthy_guard_session(&self, account: AccountId) -> BoxFuture<'_, Result<Option<rifts_seam::Json>, SeamError>> {
        NoCore.healthy_guard_session(account)
    }
    fn published_guard_artifacts(&self) -> BoxFuture<'_, Result<rifts_seam::Json, SeamError>> {
        NoCore.published_guard_artifacts()
    }
    fn guard_mode(&self) -> rifts_seam::GuardMode {
        rifts_seam::GuardMode::Observe
    }
    fn operations_policy(&self) -> BoxFuture<'_, Result<rifts_seam::Json, SeamError>> {
        NoCore.operations_policy()
    }
    fn verify_operations_step_up(&self, _: String, step_up: String) -> BoxFuture<'_, Result<bool, SeamError>> {
        Box::pin(async move { Ok(step_up == "123456") })
    }
    fn stacked_items(&self, _: String) -> BoxFuture<'_, Result<Option<rifts_seam::Json>, SeamError>> {
        let stored = lock(&self.stacked_items).clone();
        Box::pin(async move {
            stored
                .map(|text| text.map(rifts_seam::Json))
                .map_err(|()| SeamError::unimplemented(Component::Isles, "stacked_items"))
        })
    }
    fn accepted_inventory_replay(&self, _: rifts_seam::InventoryTransaction) -> BoxFuture<'_, Result<bool, SeamError>> {
        let replay = *lock(&self.replay);
        Box::pin(async move { Ok(replay) })
    }
}

pub struct Kit {
    pub guard: Arc<Guard>,
    pub store: Arc<MemoryGuardStore>,
    pub hunts: Arc<FakeHunts>,
    pub core: Arc<StepUpCore>,
    pub operations: Arc<MemoryOperations>,
    pub clock: Arc<ManualClock>,
    pub log: Arc<MemoryLog>,
}

pub fn kit(settings: &[(&str, &str)]) -> Kit {
    let settings: BTreeMap<String, String> = settings
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let store = Arc::new(MemoryGuardStore::default());
    let hunts = Arc::new(FakeHunts::default());
    let operations = Arc::new(MemoryOperations::default());
    let clock = Arc::new(ManualClock::at(UnixMillis(1_792_000_000_000)));
    let log = Arc::new(MemoryLog::default());
    let core = Arc::new(StepUpCore {
        stacked_items: Mutex::new(Ok(None)),
        replay: Mutex::new(false),
    });
    let guard = Arc::new(Guard::new(GuardParts {
        config: GuardConfig::from_settings(&settings),
        store: store.clone(),
        hunts: hunts.clone(),
        operations: operations.clone(),
        core: core.clone(),
        clock: clock.clone(),
        log: log.clone(),
    }));
    Kit {
        guard,
        store,
        hunts,
        core,
        operations,
        clock,
        log,
    }
}
