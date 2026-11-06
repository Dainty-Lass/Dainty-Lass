// SPDX-License-Identifier: MIT
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
    /// The setting's value, or `None` when it is unset.
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

/// What every rifts component does at startup and shutdown, and the storage it brings.
pub trait Lifecycle: Send + Sync {
    /// Starts the component's background work for the server role given.
    fn start(&self, role: &str) -> Result<(), SeamError>;
    /// Stops the component, flushing anything it buffers.
    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>>;
    /// The component's line for the startup log, or `None` to add nothing.
    fn startup_summary(&self, role: &str) -> Result<Option<Json>, SeamError>;
    /// Warnings about unsafe settings, each one a line for the host's log.
    fn runtime_safety_issues(&self, settings: &dyn Settings) -> Result<Vec<String>, SeamError>;
    /// The tables the component owns.
    fn tables(&self) -> &'static [TableSpec];
    /// What deleting an account removes from the component's tables.
    fn account_deletion_steps(&self) -> Vec<DeletionStep>;
}

// ---- realms ---------------------------------------------------------------------------------------------------

/// Which hub a realm's room id names.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HubKind {
    /// The realm's Ramsgate hub.
    Ramsgate,
    /// The realm's training hub.
    Training,
}

/// A matchmaking candidate as the module stores it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate(pub Json);

/// What the module knows of a hunt request when it asks rifts to route it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HuntRouteContext {
    /// The game mode the player asked for.
    pub game_mode: String,
    /// The hunt (map) id the player asked for; empty when the client sent none.
    pub hunt_id: String,
    /// The player who asked.
    pub player: AccountId,
    /// The player's party, if any.
    pub party: Option<PartyId>,
    /// Every member who would join the hunt, the player included.
    pub party_members: Vec<AccountId>,
    /// The party's revision when the request was made.
    pub party_revision: Option<u64>,
    /// The client's build id, when it sent one.
    pub build_id: Option<String>,
    /// Whether the request is for a hub rather than a hunt.
    pub is_hub: bool,
}

/// Whether rifts took a hunt request over.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HuntRouteOutcome {
    /// The module routes the hunt to a dedicated server as usual.
    NotHandled,
    /// Rifts routed the hunt to a player host.
    Handled {
        /// What the module answers the client.
        result: bool,
    },
}

/// How a player-hosted candidate's leave ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateLeaveOutcome {
    /// The candidate was canceled now.
    Canceled,
    /// The candidate had already been canceled.
    AlreadyCanceled,
    /// The hunt had already ended.
    AlreadyEnded,
    /// The hunt had already started, so the candidate stays.
    TooLate,
    /// A hub candidate, which a leave does not cancel.
    HubNotCancelled,
}

/// Whether rifts handled a candidate's leave.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateLeave {
    /// The candidate is not a player-hosted one; the module cancels it itself.
    NotHandled,
    /// Rifts handled the leave.
    Handled {
        /// How the leave ended.
        outcome: CandidateLeaveOutcome,
        /// The candidate's state after the leave, as the module reports it.
        state: String,
    },
}

/// Ordered name/value pairs, so the module writes them in this order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OperationsOverview {
    /// Policy fields for the operations page.
    pub policy: Vec<(String, Json)>,
    /// Named sections for the operations page.
    pub sections: Vec<(String, Json)>,
}

/// One release gate for the operations page, as JSON.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseGate(pub Json);

/// How rifts answered a request's authentication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthOutcome {
    /// Not a rifts credential; the module authenticates the request itself.
    NotMine,
    /// A hosted runtime's capability, checked.
    Authenticated(RuntimeAuth),
    /// A rifts credential that failed; the module sends this reply.
    Refused(Reply),
}

/// The realm a game session logs in to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealmEntry {
    /// The realm the session logs in to.
    pub realm: RealmId,
    /// Whether the player hosts that realm.
    pub is_host: bool,
}

/// What the module asks the realms component about requests, matchmaking and operations.
pub trait RealmsHooks: Lifecycle {
    // Requests.
    /// Checks a host capability on the request; [`AuthOutcome::NotMine`] when it carries none.
    fn authenticate(&self, request: RequestInfo) -> Fut<'_, AuthOutcome>;
    /// The metrics class for a request path, or `None` when rifts does not own it.
    fn metric_class(&self, path: &str) -> Option<&'static str>;
    /// Extra tags for the request's log line; empty for none.
    fn request_log_tags(&self, request: &RequestInfo) -> String;

    // Matchmaking.
    /// Whether the server hosts no dedicated hunts, leaving them to player hosts.
    fn dedicated_hunts_disabled(&self) -> Result<bool, SeamError>;
    /// Whether matchmaking drops offline party members for this mode and hunt.
    fn filters_offline_party_members(&self, game_mode: &str, hunt_id: Option<&str>) -> Result<bool, SeamError>;
    /// Offers a hunt request to player hosting before the module routes it to a dedicated server.
    fn route_hunt(&self, context: HuntRouteContext) -> Fut<'_, HuntRouteOutcome>;
    /// The player's player-hosted matchmaking candidate after a restart, if any.
    fn restore_candidate(&self, player: AccountId) -> Fut<'_, Option<Candidate>>;
    /// The player's candidate updated from player hosting, or `None` to keep the module's own.
    fn candidate_status(&self, player: AccountId, current: Candidate) -> Fut<'_, Option<Candidate>>;
    /// Leaves a player-hosted candidate.
    fn cancel_candidate(&self, player: AccountId, current: Candidate) -> Fut<'_, CandidateLeave>;
    /// The candidate of the instance the party is in, if player-hosted.
    fn party_instance(&self, party: PartyId) -> Fut<'_, Option<Candidate>>;
    /// Tells rifts a member left their party.
    fn party_member_left(&self, account: AccountId) -> Fut<'_, ()>;

    // Operators and the launcher.
    /// Fields added to the launcher's policy answer, in order.
    fn launcher_policy_fields(&self) -> Result<Vec<(String, Json)>, SeamError>;
    /// Whether an operations policy record's rifts fields are valid.
    fn operations_policy_fields_valid(&self, record: &Json) -> Result<bool, SeamError>;
    /// What rifts adds to the operations overview.
    fn operations_overview(&self) -> Fut<'_, OperationsOverview>;
    /// The release gates rifts reports.
    fn release_gates(&self) -> Fut<'_, Vec<ReleaseGate>>;
}

/// What the module asks the realms component when a game session logs in and joins a hub.
pub trait RealmsEntry: Send + Sync {
    /// Checks the realm a game session asks for; `None` for the ordinary login without one.
    fn authorize_game_session_realm(&self, account: AccountId, realm: Option<Json>) -> Fut<'_, Option<RealmEntry>>;
    /// Records which realm, if any, the player is present in after login.
    fn bind_realm_presence_at_login(&self, account: AccountId, realm: Option<RealmId>) -> Fut<'_, ()>;
    /// Refreshes the player's presence timestamp.
    fn touch_presence(&self, account: AccountId) -> Fut<'_, ()>;
    /// Whether the account is a member of the realm.
    fn is_member(&self, realm: RealmId, account: AccountId) -> Fut<'_, bool>;
    /// Whether the account is banned from the realm.
    fn is_banned(&self, realm: RealmId, account: AccountId) -> Fut<'_, bool>;
    /// The game room id of the realm's hub of this kind; an error while realms are off.
    fn hub_room_id(&self, realm: &RealmId, kind: HubKind) -> Result<String, SeamError>;
    /// The realm whose hub has this room id.
    fn find_by_hub_room_id(&self, room: String) -> Fut<'_, Option<RealmId>>;
}

// ---- guard ----------------------------------------------------------------------------------------------------

/// One answered host route, counted by Guard.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HostRouteSample {
    /// The HTTP method.
    pub method: String,
    /// The route pattern that answered.
    pub route: String,
    /// The HTTP status sent.
    pub status_code: u16,
    /// The hosted session's kind, when the caller was a runtime.
    pub session_kind: Option<String>,
    /// The hub kind, when the session was a hub.
    pub hub_kind: Option<String>,
}

/// An inventory change a hosted runtime asks for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InventoryTransaction {
    /// The account whose inventory changes.
    pub user: AccountId,
    /// The character whose inventory changes.
    pub character_id: String,
    /// The client's id for the transaction.
    pub transaction_id: String,
    /// The request body, if any.
    pub body: Option<Json>,
}

/// How an authorized inventory transaction ended, reported to its fence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionPhase {
    /// The transaction started applying its grants.
    Applying,
    /// The transaction committed.
    Accepted,
    /// The transaction failed and rolled back.
    Failed,
}

/// Reads inside the inventory transaction a fence is checking.
pub trait TransactionReads {
    /// The character's stacked items as the transaction sees them.
    fn stacked_items(&self, character_id: &str) -> Result<Option<Json>, SeamError>;
}

/// Checks each grant of an authorized inventory transaction and hears how it ended.
pub trait InventoryFence: Send + Sync {
    /// Checks one grant; an `Err` refuses the transaction with that reply.
    fn on_grant(&self, grant: &Json) -> Result<(), Reply>;

    /// For fences that need no reads, the same as [`InventoryFence::on_grant`].
    fn on_grant_in(&self, grant: &Json, reads: &dyn TransactionReads) -> Result<(), Reply> {
        let _ = reads;
        self.on_grant(grant)
    }

    /// Hears how the transaction ended.
    fn report(&self, phase: TransactionPhase);
}

/// Guard's answer to an inventory transaction.
pub enum InventoryDecision {
    /// Allowed, with the fence that checks each grant.
    Authorized(Box<dyn InventoryFence>),
    /// Refused with this reply.
    Refused(Reply),
}

/// The player state a hosted runtime may write.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HostWriteKind {
    /// Hunt XP.
    Xp,
    /// Weapon or other mastery progress.
    Mastery,
    /// Objective progress.
    Objectives,
    /// Escalation progress.
    Escalation,
    /// A cooldown.
    Cooldown,
    /// Bounties.
    Bounty,
    /// The Slayer's Path.
    SlayersPath,
    /// Reforging.
    Reforge,
    /// A rank confirmation.
    RankConfirm,
    /// A loadout unlock.
    LoadoutUnlock,
    /// Hunt pass XP.
    HuntPassXp,
    /// An item save.
    ItemSave,
}

/// How Guard checks a host write.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HostWriteOptions {
    /// A dry run: nothing is logged when the write is refused.
    pub probe: bool,
    /// The write is a dedicated server's work done by the host, refused outright while the host policy enforces.
    pub parity: bool,
}

/// One accepted host write, recorded for the audit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostWrite {
    /// What kind of state was written.
    pub kind: HostWriteKind,
    /// The player whose state was written.
    pub target: AccountId,
    /// A summary of the write, as JSON.
    pub summary: Json,
}

/// What the module asks Guard about hosted runtimes and the writes they make.
pub trait GuardHooks: Lifecycle {
    /// The roles listed in Guard's manifest.
    fn guard_manifest_roles(&self) -> &'static [&'static str];
    /// The server roles Guard runs as a service in; empty when it runs in-process only.
    fn service_roles(&self) -> &'static [&'static str];
    /// Whether the host policy enforces rather than observes.
    fn host_policy_refuses(&self) -> Result<bool, SeamError>;
    /// Logs what an enforcing host policy would have refused.
    fn note_would_refuse(&self, what: &str, detail: &str) -> Result<(), SeamError>;
    /// Counts one answered host route.
    fn census(&self, sample: &HostRouteSample) -> Result<(), SeamError>;
    /// The rate-limit bucket for a request carrying a live host capability, or `None` for the default bucket.
    fn rate_limit_bucket(&self, request: RequestInfo) -> Fut<'_, Option<String>>;
    /// Checks a hosted runtime's write to a player's state; `Some` is the refusal to send.
    fn authorize_host_write(
        &self,
        runtime: RuntimeAuth,
        kind: HostWriteKind,
        target: AccountId,
        options: HostWriteOptions,
    ) -> Fut<'_, Option<Reply>>;
    /// Records an accepted host write.
    fn record_host_write(&self, runtime: RuntimeAuth, write: HostWrite) -> Fut<'_, ()>;
    /// Decides whether a hosted runtime may run an inventory transaction.
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
