// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What the module calls into rifts for.

use std::{
    collections::{BTreeMap, HashMap},
    hash::BuildHasher,
};

use crate::{
    error::SeamError,
    http::{BoxFuture, Json, Reply, RequestInfo, RuntimeAuth},
    id::{AccountId, PartyId, RealmId},
    store::{DeletionStep, TableSpec},
};

type Fut<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

// ---- every component ------------------------------------------------------------------------------------------

/// A read-only view of the server's settings.
pub trait Settings {
    fn get(&self, name: &str) -> Option<&str>;
}

// The two map types the module keeps its settings in.
macro_rules! settings_map {
    ($($map:ty $(where $param:ident: $bound:path)?;)*) => {$(
        impl$(<$param: $bound>)? Settings for $map {
            fn get(&self, name: &str) -> Option<&str> {
                <$map>::get(self, name).map(String::as_str)
            }
        }
    )*};
}

settings_map! {
    HashMap<String, String, S> where S: BuildHasher;
    BTreeMap<String, String>;
}

pub trait Lifecycle: Send + Sync {
    fn start(&self, role: &str) -> Result<(), SeamError>;
    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>>;
    fn startup_summary(&self, role: &str) -> Result<Option<Json>, SeamError>;
    fn runtime_safety_issues(&self, settings: &dyn Settings) -> Result<Vec<String>, SeamError>;
    fn tables(&self) -> &'static [TableSpec];
    fn account_deletion_steps(&self) -> Vec<DeletionStep>;
}

// ---- realms ---------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HubKind {
    Ramsgate,
    Training,
}

/// A matchmaking candidate as the module stores it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate(pub Json);

/// What the module knows of a hunt request when it asks rifts to route it.
#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HuntRouteOutcome {
    /// The module routes the hunt to a dedicated server as usual.
    NotHandled,
    Handled {
        result: bool,
    },
}

/// How a player-hosted candidate's leave ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateLeaveOutcome {
    Canceled,
    AlreadyCanceled,
    AlreadyEnded,
    TooLate,
    HubNotCancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateLeave {
    NotHandled,
    Handled {
        outcome: CandidateLeaveOutcome,
        state: String,
    },
}

/// Ordered name/value pairs, so the module writes them in this order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OperationsOverview {
    pub policy: Vec<(String, Json)>,
    pub sections: Vec<(String, Json)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseGate(pub Json);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthOutcome {
    /// Not a rifts credential; the module authenticates the request itself.
    NotMine,
    Authenticated(RuntimeAuth),
    Refused(Reply),
}

/// The realm a game session logs in to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealmEntry {
    pub realm: RealmId,
    pub is_host: bool,
}

pub trait RealmsHooks: Lifecycle {
    // Requests.
    fn authenticate(&self, request: RequestInfo) -> Fut<'_, AuthOutcome>;
    fn metric_class(&self, path: &str) -> Option<&'static str>;
    fn request_log_tags(&self, request: &RequestInfo) -> String;

    // Matchmaking.
    fn dedicated_hunts_disabled(&self) -> Result<bool, SeamError>;
    fn filters_offline_party_members(&self, game_mode: &str, hunt_id: Option<&str>) -> Result<bool, SeamError>;
    fn route_hunt(&self, context: HuntRouteContext) -> Fut<'_, HuntRouteOutcome>;
    fn restore_candidate(&self, player: AccountId) -> Fut<'_, Option<Candidate>>;
    fn candidate_status(&self, player: AccountId, current: Candidate) -> Fut<'_, Option<Candidate>>;
    fn cancel_candidate(&self, player: AccountId, current: Candidate) -> Fut<'_, CandidateLeave>;
    fn party_instance(&self, party: PartyId) -> Fut<'_, Option<Candidate>>;
    fn party_member_left(&self, account: AccountId) -> Fut<'_, ()>;

    // Operators and the launcher.
    fn launcher_policy_fields(&self) -> Result<Vec<(String, Json)>, SeamError>;
    fn operations_policy_fields_valid(&self, record: &Json) -> Result<bool, SeamError>;
    fn operations_overview(&self) -> Fut<'_, OperationsOverview>;
    fn release_gates(&self) -> Fut<'_, Vec<ReleaseGate>>;
}

pub trait RealmsEntry: Send + Sync {
    fn authorize_game_session_realm(&self, account: AccountId, realm: Option<Json>) -> Fut<'_, Option<RealmEntry>>;
    fn bind_realm_presence_at_login(&self, account: AccountId, realm: Option<RealmId>) -> Fut<'_, ()>;
    fn touch_presence(&self, account: AccountId) -> Fut<'_, ()>;
    fn is_member(&self, realm: RealmId, account: AccountId) -> Fut<'_, bool>;
    fn is_banned(&self, realm: RealmId, account: AccountId) -> Fut<'_, bool>;
    fn hub_room_id(&self, realm: &RealmId, kind: HubKind) -> Result<String, SeamError>;
    fn find_by_hub_room_id(&self, room: String) -> Fut<'_, Option<RealmId>>;
}

// ---- guard ----------------------------------------------------------------------------------------------------

/// One answered host route, counted by Guard.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HostRouteSample {
    pub method: String,
    pub route: String,
    pub status_code: u16,
    pub session_kind: Option<String>,
    pub hub_kind: Option<String>,
}

/// An inventory change a hosted runtime asks for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InventoryTransaction {
    pub user: AccountId,
    pub character_id: String,
    pub transaction_id: String,
    pub body: Option<Json>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

    /// For fences that need no reads, the same as [`InventoryFence::on_grant`].
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

/// The player state a hosted runtime may write.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HostWriteOptions {
    pub probe: bool,
    pub parity: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostWrite {
    pub kind: HostWriteKind,
    pub target: AccountId,
    pub summary: Json,
}

pub trait GuardHooks: Lifecycle {
    fn guard_manifest_roles(&self) -> &'static [&'static str];
    fn service_roles(&self) -> &'static [&'static str];
    fn host_policy_refuses(&self) -> Result<bool, SeamError>;
    fn note_would_refuse(&self, what: &str, detail: &str) -> Result<(), SeamError>;
    fn census(&self, sample: &HostRouteSample) -> Result<(), SeamError>;
    fn rate_limit_bucket(&self, request: RequestInfo) -> Fut<'_, Option<String>>;
    fn authorize_host_write(
        &self,
        runtime: RuntimeAuth,
        kind: HostWriteKind,
        target: AccountId,
        options: HostWriteOptions,
    ) -> Fut<'_, Option<Reply>>;
    fn record_host_write(&self, runtime: RuntimeAuth, write: HostWrite) -> Fut<'_, ()>;
    fn authorize_inventory(
        &self,
        runtime: RuntimeAuth,
        transaction: InventoryTransaction,
    ) -> Fut<'_, InventoryDecision>;
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn settings_maps() {
        let hashed = HashMap::from([(String::from("A"), String::from("1"))]);
        let sorted = BTreeMap::from([(String::from("A"), String::from("2"))]);
        let views: [&dyn Settings; 2] = [&hashed, &sorted];
        let read = |name: &str| views.map(|view| view.get(name));
        assert_eq!(read("A"), [Some("1"), Some("2")]);
        assert_eq!(read("B"), [None; 2]);
    }

    #[derive(Default)]
    struct Journal(Mutex<Vec<String>>);

    impl Journal {
        fn note(&self, line: String) {
            self.0.lock().unwrap().push(line);
        }
    }

    impl InventoryFence for Journal {
        fn on_grant(&self, grant: &Json) -> Result<(), Reply> {
            self.note(grant.0.clone());
            Ok(())
        }

        fn report(&self, phase: TransactionPhase) {
            self.note(format!("{phase:?}"));
        }
    }

    struct Empty;

    impl TransactionReads for Empty {
        fn stacked_items(&self, _: &str) -> Result<Option<Json>, SeamError> {
            Ok(None)
        }
    }

    #[test]
    fn on_grant_in_defaults_to_on_grant() {
        let fence = Journal::default();
        let grant = Json(r#"{"a":1}"#.to_owned());
        assert_eq!(fence.on_grant_in(&grant, &Empty), Ok(()));
        fence.report(TransactionPhase::Accepted);
        assert_eq!(fence.0.into_inner().unwrap(), [r#"{"a":1}"#, "Accepted"]);
    }
}
