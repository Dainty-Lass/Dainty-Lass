// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What the module calls into rifts for.

use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;

use crate::error::SeamError;
use crate::http::{BoxFuture, Json, Reply, RequestInfo, RuntimeAuth};
use crate::id::{AccountId, PartyId, RealmId};
use crate::store::{DeletionStep, TableSpec};

type Fut<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

/// A read-only view of the server's settings.
pub trait Settings {
    fn get(&self, name: &str) -> Option<&str>;
}

impl<S: BuildHasher> Settings for HashMap<String, String, S> {
    fn get(&self, name: &str) -> Option<&str> {
        HashMap::get(self, name).map(String::as_str)
    }
}

impl Settings for BTreeMap<String, String> {
    fn get(&self, name: &str) -> Option<&str> {
        BTreeMap::get(self, name).map(String::as_str)
    }
}

pub trait Lifecycle: Send + Sync {
    fn runtime_safety_issues(&self, settings: &dyn Settings) -> Result<Vec<String>, SeamError>;
    fn startup_summary(&self, role: &str) -> Result<Option<Json>, SeamError>;
    fn start(&self, role: &str) -> Result<(), SeamError>;
    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>>;
    fn tables(&self) -> &'static [TableSpec];
    fn account_deletion_steps(&self) -> Vec<DeletionStep>;
}

/// A matchmaking candidate as the module stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate(pub Json);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntRouteContext {
    pub game_mode: String,
    pub hunt_id: String,
    pub player: AccountId,
    pub party: Option<PartyId>,
    pub party_members: Vec<AccountId>,
    pub party_revision: Option<u64>,
    pub build_id: Option<String>,
    pub is_hub: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HuntRouteOutcome {
    /// The module routes the hunt to a dedicated server as usual.
    NotHandled,
    Handled {
        result: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateLeaveOutcome {
    Canceled,
    AlreadyCanceled,
    AlreadyEnded,
    TooLate,
    HubNotCancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateLeave {
    NotHandled,
    Handled {
        outcome: CandidateLeaveOutcome,
        state: String,
    },
}

/// Ordered name/value pairs, so the module writes them in this order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OperationsOverview {
    pub policy: Vec<(String, Json)>,
    pub sections: Vec<(String, Json)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseGate(pub Json);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    /// Not a rifts credential; the module authenticates the request itself.
    NotMine,
    Authenticated(RuntimeAuth),
    Refused(Reply),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HubKind {
    Ramsgate,
    Training,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmEntry {
    pub realm: RealmId,
    pub is_host: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryTransaction {
    pub user: AccountId,
    pub character_id: String,
    pub transaction_id: String,
    pub body: Option<Json>,
}

/// One answered host route, counted by Guard.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostRouteSample {
    pub method: String,
    pub route: String,
    pub status_code: u16,
    pub session_kind: Option<String>,
    pub hub_kind: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionPhase {
    Applying,
    Accepted,
    Failed,
}

/// Reads inside the inventory transaction a fence is checking.
pub trait TransactionReads {
    fn stacked_items(&self, character_id: &str) -> Result<Option<Json>, SeamError>;
}

/// Checks each grant of an authorized inventory transaction and hears how it ended.
pub trait InventoryFence: Send + Sync {
    fn on_grant(&self, grant: &Json) -> Result<(), Reply>;

    fn on_grant_in(&self, grant: &Json, reads: &dyn TransactionReads) -> Result<(), Reply> {
        let _ = reads;
        self.on_grant(grant)
    }

    fn report(&self, phase: TransactionPhase);
}

pub enum InventoryDecision {
    Authorized(Box<dyn InventoryFence>),
    Refused(Reply),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostWriteKind {
    Xp,
    Mastery,
    Objectives,
    Escalation,
    Cooldown,
    Bounty,
    SlayersPath,
    Reforge,
    RankConfirm,
    LoadoutUnlock,
    HuntPassXp,
    ItemSave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostWriteOptions {
    pub probe: bool,
    pub parity: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostWrite {
    pub kind: HostWriteKind,
    pub target: AccountId,
    pub summary: Json,
}

pub trait RealmsHooks: Lifecycle {
    fn metric_class(&self, path: &str) -> Option<&'static str>;
    fn request_log_tags(&self, request: &RequestInfo) -> String;
    fn authenticate(&self, request: RequestInfo) -> Fut<'_, AuthOutcome>;
    fn dedicated_hunts_disabled(&self) -> Result<bool, SeamError>;
    fn filters_offline_party_members(&self, game_mode: &str, hunt_id: Option<&str>) -> Result<bool, SeamError>;
    fn restore_candidate(&self, player: AccountId) -> Fut<'_, Option<Candidate>>;
    fn candidate_status(&self, player: AccountId, current: Candidate) -> Fut<'_, Option<Candidate>>;
    fn party_instance(&self, party: PartyId) -> Fut<'_, Option<Candidate>>;
    fn route_hunt(&self, context: HuntRouteContext) -> Fut<'_, HuntRouteOutcome>;
    fn cancel_candidate(&self, player: AccountId, current: Candidate) -> Fut<'_, CandidateLeave>;
    fn party_member_left(&self, account: AccountId) -> Fut<'_, ()>;
    fn launcher_policy_fields(&self) -> Result<Vec<(String, Json)>, SeamError>;
    fn operations_policy_fields_valid(&self, record: &Json) -> Result<bool, SeamError>;
    fn operations_overview(&self) -> Fut<'_, OperationsOverview>;
    fn release_gates(&self) -> Fut<'_, Vec<ReleaseGate>>;
}

pub trait RealmsEntry: Send + Sync {
    fn authorize_game_session_realm(&self, account: AccountId, realm: Option<Json>) -> Fut<'_, Option<RealmEntry>>;
    fn bind_realm_presence_at_login(&self, account: AccountId, realm: Option<RealmId>) -> Fut<'_, ()>;
    fn touch_presence(&self, account: AccountId) -> Fut<'_, ()>;
    fn find_by_hub_room_id(&self, room: String) -> Fut<'_, Option<RealmId>>;
    fn is_member(&self, realm: RealmId, account: AccountId) -> Fut<'_, bool>;
    fn is_banned(&self, realm: RealmId, account: AccountId) -> Fut<'_, bool>;
    fn hub_room_id(&self, realm: &RealmId, kind: HubKind) -> Result<String, SeamError>;
}

pub trait GuardHooks: Lifecycle {
    fn service_roles(&self) -> &'static [&'static str];
    fn guard_manifest_roles(&self) -> &'static [&'static str];
    fn census(&self, sample: &HostRouteSample) -> Result<(), SeamError>;
    fn rate_limit_bucket(&self, request: RequestInfo) -> Fut<'_, Option<String>>;
    fn authorize_inventory(
        &self,
        runtime: RuntimeAuth,
        transaction: InventoryTransaction,
    ) -> Fut<'_, InventoryDecision>;
    fn authorize_host_write(
        &self,
        runtime: RuntimeAuth,
        kind: HostWriteKind,
        target: AccountId,
        options: HostWriteOptions,
    ) -> Fut<'_, Option<Reply>>;
    fn record_host_write(&self, runtime: RuntimeAuth, write: HostWrite) -> Fut<'_, ()>;
    fn host_policy_refuses(&self) -> Result<bool, SeamError>;
    fn note_would_refuse(&self, what: &str, detail: &str) -> Result<(), SeamError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn settings_maps() {
        let mut hashed: HashMap<String, String> = HashMap::new();
        hashed.insert("A".into(), "1".into());
        let tree: BTreeMap<String, String> = [("A".to_owned(), "2".to_owned())].into();
        let views: [&dyn Settings; 2] = [&hashed, &tree];
        assert_eq!(views.map(|s| s.get("A")), [Some("1"), Some("2")]);
        assert_eq!(views.map(|s| s.get("B")), [None, None]);
    }

    struct Recorder(Mutex<Vec<String>>);

    impl InventoryFence for Recorder {
        fn on_grant(&self, grant: &Json) -> Result<(), Reply> {
            self.0.lock().unwrap().push(grant.0.clone());
            Ok(())
        }

        fn report(&self, phase: TransactionPhase) {
            self.0.lock().unwrap().push(format!("{phase:?}"));
        }
    }

    struct NoReads;

    impl TransactionReads for NoReads {
        fn stacked_items(&self, _character_id: &str) -> Result<Option<Json>, SeamError> {
            Ok(None)
        }
    }

    #[test]
    fn on_grant_in_defaults_to_on_grant() {
        let fence = Recorder(Mutex::new(Vec::new()));
        fence.on_grant_in(&Json("{\"a\":1}".into()), &NoReads).unwrap();
        fence.report(TransactionPhase::Accepted);
        assert_eq!(*fence.0.lock().unwrap(), ["{\"a\":1}", "Accepted"]);
    }
}
