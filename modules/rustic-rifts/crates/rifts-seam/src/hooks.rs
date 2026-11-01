//! The hooks isles calls, grouped by the crate that answers them (RF1, section 2.1). Node's grouping
//! (`server`, `http`, `inventory`, `hostWrites`, `hostPolicy`, `matchmaking`, `launcher`, `operations`) is kept in
//! each method's doc. Methods are async where Node's hook is.

use crate::error::SeamError;
use crate::http::{BoxFuture, Json, Reply, RequestInfo, RuntimeAuth};
use crate::id::{AccountId, PartyId, RealmId};
use crate::store::{DeletionStep, TableSpec};

/// The settings a component reads. Node passes `process.env`; DaintyLass hands each service its env file.
pub trait Settings {
    fn get(&self, name: &str) -> Option<&str>;
}

impl<S: std::hash::BuildHasher> Settings for std::collections::HashMap<String, String, S> {
    fn get(&self, name: &str) -> Option<&str> {
        std::collections::HashMap::get(self, name).map(String::as_str)
    }
}

impl Settings for std::collections::BTreeMap<String, String> {
    fn get(&self, name: &str) -> Option<&str> {
        std::collections::BTreeMap::get(self, name).map(String::as_str)
    }
}

/// `server.*` of both hook objects.
pub trait Lifecycle: Send + Sync {
    /// `server.runtimeSafetyIssues`: the settings that make a production configuration unsafe.
    fn runtime_safety_issues(&self, settings: &dyn Settings) -> Result<Vec<String>, SeamError>;
    /// `server.startupSummary`, logged once.
    fn startup_summary(&self, role: &str) -> Result<Option<Json>, SeamError>;
    /// `server.start`: sweepers, the lifecycle monitor, the alert dispatcher.
    fn start(&self, role: &str) -> Result<(), SeamError>;
    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>>;
    /// `server.ensureDatabase`: the tables this component owns; DF7's migrations create them.
    fn tables(&self) -> &'static [TableSpec];
    /// `server.accountDeletionSteps`. Realms' own (`realms`, `realmInvites`, `realmPresence`) move behind this hook
    /// instead of sitting in the core's plan (RF1, decision 3).
    fn account_deletion_steps(&self) -> Vec<DeletionStep>;
}

/// `MatchmakingResult` as the client sees it. It is isles' type, and isles depends on this crate, so it crosses as
/// JSON text until RU4's `isles-core` and RF5 agree where it lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate(pub Json);

/// `HuntRouteContext` without its callbacks: `setCandidate` and `cancel` are [`crate::IslesCore`] methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntRouteContext {
    pub game_mode: String,
    pub hunt_id: String,
    pub player: AccountId,
    pub party: Option<PartyId>,
    pub party_members: Vec<AccountId>,
    pub party_revision: Option<u64>,
    pub build_id: Option<String>,
    /// Ramsgate, the city and the Training Dojo.
    pub is_hub: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HuntRouteOutcome {
    /// Continue with the dedicated route (the Director).
    NotHandled,
    Handled {
        result: bool,
    },
}

/// `CandidateLeaveOutcome`: what the client's Cancel did to a search rifts owns.
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

/// `OperationsOverviewExtension`: merged into the admin overview.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OperationsOverview {
    pub policy: Vec<(String, Json)>,
    pub sections: Vec<(String, Json)>,
}

/// `ReleaseGate` (`observability/releaseGates.ts`), as JSON until isles' type is ported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseGate(pub Json);

/// The outcome of `http.authenticate` for a request carrying a host capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    /// Not rifts' credential; isles' own auth goes on.
    NotMine,
    Authenticated(RuntimeAuth),
    Refused(Reply),
}

/// The `P2PExtension` hooks rifts-realms answers.
pub trait RealmsHooks: Lifecycle {
    /// `http.metricClass`.
    fn metric_class(&self, path: &str) -> Option<&'static str>;
    /// `http.requestLogTags`: extra ` key=value` tags.
    fn request_log_tags(&self, request: &RequestInfo) -> String;
    /// `http.authenticate`: a hosted game server's capability (scope `GAME_SERVER_RUNTIME`).
    fn authenticate(&self, request: RequestInfo) -> BoxFuture<'_, Result<AuthOutcome, SeamError>>;

    /// `matchmaking.dedicatedHuntsDisabled`.
    fn dedicated_hunts_disabled(&self) -> Result<bool, SeamError>;
    /// `matchmaking.filtersOfflinePartyMembers`.
    fn filters_offline_party_members(&self, game_mode: &str, hunt_id: Option<&str>) -> Result<bool, SeamError>;
    /// `matchmaking.restoreCandidate`, after a restart.
    fn restore_candidate(&self, player: AccountId) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>>;
    /// `matchmaking.candidateStatus`; `None` for a dedicated candidate.
    fn candidate_status(
        &self,
        player: AccountId,
        current: Candidate,
    ) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>>;
    /// `matchmaking.partyInstance`.
    fn party_instance(&self, party: PartyId) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>>;
    /// `matchmaking.routeHunt`.
    fn route_hunt(&self, context: HuntRouteContext) -> BoxFuture<'_, Result<HuntRouteOutcome, SeamError>>;
    /// `matchmaking.cancelCandidate`.
    fn cancel_candidate(
        &self,
        player: AccountId,
        current: Candidate,
    ) -> BoxFuture<'_, Result<CandidateLeave, SeamError>>;
    /// `matchmaking.partyMemberLeft`.
    fn party_member_left(&self, account: AccountId) -> BoxFuture<'_, Result<(), SeamError>>;

    /// `launcher.policyFields`: `p2pEmergencyStop`, `coopHunts` in `GET /launcher/v1/policy`.
    fn launcher_policy_fields(&self) -> Result<Vec<(String, Json)>, SeamError>;
    /// `operations.policyFieldsValid`.
    fn operations_policy_fields_valid(&self, record: &Json) -> Result<bool, SeamError>;
    /// `operations.overview`.
    fn operations_overview(&self) -> BoxFuture<'_, Result<OperationsOverview, SeamError>>;
    /// `releaseGates`.
    fn release_gates(&self) -> BoxFuture<'_, Result<Vec<ReleaseGate>, SeamError>>;
}

/// A realm hub's kind (`RealmHubKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HubKind {
    Ramsgate,
    Training,
}

/// `AuthorizeGameSessionRealm`'s answer: the realm the exchange code will carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmEntry {
    pub realm: RealmId,
    pub is_host: bool,
}

/// The `realms/index.ts` exports the core imports directly (RF1, section 2.1), all rifts-realms.
pub trait RealmsEntry: Send + Sync {
    /// `AuthorizeGameSessionRealm`, for `POST /launcher/v1/game-sessions`. `None` without a realm in the body.
    fn authorize_game_session_realm(
        &self,
        account: AccountId,
        realm: Option<Json>,
    ) -> BoxFuture<'_, Result<Option<RealmEntry>, SeamError>>;
    /// `BindRealmPresenceAtLogin`, at the game login that consumes the exchange code.
    fn bind_realm_presence_at_login(
        &self,
        account: AccountId,
        realm: Option<RealmId>,
    ) -> BoxFuture<'_, Result<(), SeamError>>;
    /// `GetRealmsService().touchPresence`, from the matchmaking keep-alives.
    fn touch_presence(&self, account: AccountId) -> BoxFuture<'_, Result<(), SeamError>>;
    /// `findByHubRoomId`, for chat rooms of a realm's hubs.
    fn find_by_hub_room_id(&self, room: String) -> BoxFuture<'_, Result<Option<RealmId>, SeamError>>;
    fn is_member(&self, realm: RealmId, account: AccountId) -> BoxFuture<'_, Result<bool, SeamError>>;
    fn is_banned(&self, realm: RealmId, account: AccountId) -> BoxFuture<'_, Result<bool, SeamError>>;
    /// `HubRoomId`: the UUID-shaped chat room of one hub of a realm.
    fn hub_room_id(&self, realm: &RealmId, kind: HubKind) -> Result<String, SeamError>;
}

/// `POST /inventory`'s context (`InventoryTransactionContext`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryTransaction {
    pub user: AccountId,
    pub character_id: String,
    pub transaction_id: String,
    /// The request body as JSON when available (`POST /inventory`), so Guard can check `accountId` and grant rules.
    pub body: Option<Json>,
}

/// One answered host-runtime request (`p2p/observability/hostRouteCensus.ts`). `route` is the Express pattern with its
/// mount path, never a URL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

/// What a fence reads through isles' inventory transaction (`onGrant`'s `repos` and `session`).
pub trait TransactionReads {
    /// The character's `stackedItems` as stored; `Ok(None)` when it has no inventory.
    fn stacked_items(&self, character_id: &str) -> Result<Option<Json>, SeamError>;
}

/// `ExtensionInventoryTransaction`: the grant fence isles runs inside its inventory transaction.
pub trait InventoryFence: Send + Sync {
    /// `options.onGrant` with `errorResponse`: `Err` refuses the grant with that reply.
    fn on_grant(&self, grant: &Json) -> Result<(), Reply>;
    /// `on_grant` with the transaction's own reads, which isles calls when it has them.
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

/// `HostWriteKind`: what a hosting runtime writes for a player besides inventory.
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

/// `HostWriteAuthorizeOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostWriteOptions {
    /// A quiet check among several kinds: no log, the kind always enforced.
    pub probe: bool,
    /// A dedicated server's own work for a player of its session; observe mode only.
    pub parity: bool,
}

/// `HostWriteRecordInput`. `summary` holds what changed, no secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostWrite {
    pub kind: HostWriteKind,
    pub target: AccountId,
    pub summary: Json,
}

/// The hooks rifts-guard answers: the anticheat module's, and the `P2PExtension` hooks RF1 gives Guard.
pub trait GuardHooks: Lifecycle {
    /// `server.serviceRoles`: extra `MYSTICPARADOX_SERVICE_ROLE` values (none on the stubs).
    fn service_roles(&self) -> &'static [&'static str];
    /// `launcher.guardManifestRoles`.
    fn guard_manifest_roles(&self) -> &'static [&'static str];
    /// The host route census (`p2p/observability/hostRouteCensus.ts`), recorded once the request is answered.
    fn census(&self, sample: &HostRouteSample) -> Result<(), SeamError>;
    /// `http.rateLimitBucket`: a validated capability's own bucket, or `None` for the per-address one.
    fn rate_limit_bucket(&self, request: RequestInfo) -> BoxFuture<'_, Result<Option<String>, SeamError>>;
    /// `inventory.authorizeTransaction` for a runtime rifts authenticated.
    fn authorize_inventory(
        &self,
        runtime: RuntimeAuth,
        transaction: InventoryTransaction,
    ) -> BoxFuture<'_, Result<InventoryDecision, SeamError>>;
    /// `hostWrites.authorize`: `None` lets the write through, `Some` is the reply that refuses it.
    fn authorize_host_write(
        &self,
        runtime: RuntimeAuth,
        kind: HostWriteKind,
        target: AccountId,
        options: HostWriteOptions,
    ) -> BoxFuture<'_, Result<Option<Reply>, SeamError>>;
    /// `hostWrites.record`. Node's never throws; isles logs an error here and goes on.
    fn record_host_write(&self, runtime: RuntimeAuth, write: HostWrite) -> BoxFuture<'_, Result<(), SeamError>>;
    /// `hostPolicy.refuses`: true only in enforce mode.
    fn host_policy_refuses(&self) -> Result<bool, SeamError>;
    /// `hostPolicy.noteWouldRefuse`, in observe mode.
    fn note_would_refuse(&self, what: &str, detail: &str) -> Result<(), SeamError>;
}
