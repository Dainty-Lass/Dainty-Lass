//! Guard (RF6): observe mode on stubs, since Mystic's anticheat module is private (DD14).
//!
//! Ported from `p2p/antiCheat.ts` (the stub side), `p2p/security/hostWrites.ts`, `hubAuthorization.ts`,
//! `p2pInventoryTransactions.ts`, `hostPolicyMode.ts`, `guardTraffic*.ts`, `config/huntEvidence.ts`, the host route
//! census and the operations alerts (RF1). [`Guard`] answers [`rifts_seam::GuardHooks`] and the six routes; [`Stub`]
//! still answers [`rifts_seam::Unimplemented`] everywhere for builds that leave Guard out.
//!
//! The evidence chain intake is rifts' own ([`evidence`]): with `MYSTICPARADOX_HUNT_EVIDENCE` off (the default at trust
//! tier 0) the routes answer `HUNT_EVIDENCE_NOT_COLLECTED` exactly as Node does without the module; with it on, Guard
//! takes the transport's batches, checks the chain and records what it would refuse, never refusing a hunt for it.
//!
//! Launcher Guard (`/launcher/v1/guard/*`, `/admin/v1/guard/*`) and the Guard manifests stay isles' (RF1, decision 1):
//! every official Play uses them, so a 1.14.7 module built without rifts must keep them. Realms read them through
//! [`rifts_seam::IslesCore`], and `MYSTICPARADOX_GUARD_MODE` goes with them.

pub mod census;
mod config;
mod digest;
mod errors;
pub mod evidence;
pub mod json;
mod operations;
pub mod ports;
mod routes;
pub mod sql;
pub mod store;
mod traffic;
mod writes;

use std::collections::VecDeque;
use std::collections::hash_map::HashMap;
use std::sync::{Arc, Mutex};

pub use census::{HostRouteCensus, HostRouteRow, HostRouteSample, HostRouteSummary};
pub use config::{
    GuardConfig, HostPolicyMode, HostPolicySetting, HuntEvidenceSetting, Tier1Policy, TrustTier, log_host_policy,
    log_hunt_evidence, resolve_host_policy, resolve_hunt_evidence,
};
pub use digest::sha256_hex;
pub use errors::{ERROR_STATUSES, GuardError, status_of};
pub use operations::{DEFAULT_REWARD_ROLLOUT, RewardRollout};
pub use ports::{
    AuditEntry, CapabilityView, Clock, GuardLog, HuntSessions, HuntView, ManualClock, MemoryLog, MemoryOperations,
    OperationsPolicyPort, PolicySnapshot, RosterEntry, StderrLog, SystemClock, iso_time,
};
pub use routes::{GuardRoutes, ROUTES, envelope, stub_routes};
pub use sql::{
    POSTGRES_MIGRATIONS, SCHEMA, SQLITE_MIGRATIONS, SqlGuardStore, XP_SUMMARY_TTL_MS, block_on_store, store_actor,
};
pub use store::{
    AlertRecord, BatchRecord, ChainRecord, ChainUpdate, CompletionMode, CompletionRecord, Disposition, EncounterState,
    FindingRecord, GENESIS_HASH, GuardStore, HostWriteRecord, MemoryGuardStore, StubGuardStore, TABLES,
    TrafficReservation, TrafficScope, TrafficWindowRecord, XpSummaryRecord,
};
pub use traffic::{
    HUNT_GUARD_HOST_TRAFFIC_LIMIT, HUNT_GUARD_REQUEST_BUDGET, HUNT_GUARD_SESSION_TRAFFIC_LIMIT, HuntOperation,
    LocalTrafficLimiter, RequestBudget, TrafficLimit,
};

use rifts_seam::{
    AccountId, Actor, BoxFuture, Component, DeletionAction, DeletionStep, GuardHooks, HOST_CAPABILITY_HEADER,
    HostWrite, HostWriteKind, HostWriteOptions, InventoryDecision, InventoryFence, InventoryTransaction, IslesCore,
    Json, Lifecycle, Reply, RequestInfo, RuntimeAuth, SeamError, Secret, SessionId, Settings, TableSpec,
    TransactionPhase, stub,
};
use serde_json::Value;

use crate::errors::{fenced, not_collected, store_to_seam};
use crate::json::js_slice;
use crate::ports::lock;
pub use writes::{host_write_denial, inventory_body_denial, inventory_denial, kind_name, loot_refusal, write_refusal};

/// `HuntVerdict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HuntVerdict {
    pub progression_applied: bool,
    pub rewards_applied: bool,
}

impl HuntVerdict {
    /// What observe mode decides: nothing is applied from evidence.
    pub const OBSERVED: Self = Self {
        progression_applied: false,
        rewards_applied: false,
    };

    pub fn to_json(&self) -> Json {
        Json(format!(
            r#"{{"progressionApplied":{},"rewardsApplied":{}}}"#,
            self.progression_applied, self.rewards_applied
        ))
    }
}

/// `RewardPolicyStatus`: never ready without the anticheat module, so a Tier 1 hunt routes to a dedicated server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewardPolicyStatus {
    Ready { version: String, digest: String },
    NotReady { reason: &'static str },
}

/// The anticheat module's `hunts.*`: the evidence chain intake, in observe mode.
pub trait EvidenceIntake: Send + Sync {
    /// `POST .../event-batches`.
    fn submit_evidence(&self, runtime: RuntimeAuth, batch: Json) -> BoxFuture<'_, Result<Json, GuardError>>;
    /// `POST .../completion`.
    fn complete(&self, runtime: RuntimeAuth, request: Json) -> BoxFuture<'_, Result<Json, GuardError>>;
    /// `hunts.result`, shown by the hunt status route.
    fn verdict(&self, session: SessionId, generation: u64) -> BoxFuture<'_, Result<Option<HuntVerdict>, GuardError>>;
    /// `hunts.rewardReceipt`, for `GET .../reward/self`.
    fn reward_receipt(
        &self,
        session: SessionId,
        generation: u64,
        account: AccountId,
    ) -> BoxFuture<'_, Result<Option<Json>, GuardError>>;
    /// `POST /internal/v1/hunt-guard/shadow-comparisons`; the key is `x-hunt-guard-shadow-key`.
    fn record_shadow_comparison(
        &self,
        key: Option<Secret>,
        comparison: Json,
    ) -> BoxFuture<'_, Result<Json, GuardError>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficDecision {
    Accepted,
    /// `GUARD_TRAFFIC_RATE_LIMITED`.
    RateLimited,
    /// `GUARD_TRAFFIC_UNAVAILABLE`: the distributed windows could not be read; never a local fallback.
    Unavailable,
}

/// The Guard traffic limits, local or distributed (`P2P_DISTRIBUTED_TRAFFIC_LIMITER`).
pub trait TrafficLimits: Send + Sync {
    /// `EnforceP2PGuardTraffic`: one request of `bytes` against the host's and the hunt session's windows.
    fn charge(
        &self,
        host: AccountId,
        session: SessionId,
        bytes: u64,
    ) -> BoxFuture<'_, Result<TrafficDecision, GuardError>>;
}

/// Guard's admin operations routes. Replies are Node's own shapes, not the launcher envelope.
pub trait GuardOperations: Send + Sync {
    /// `UpdateRewardRollout` (Tier 1 grants).
    fn set_reward_rollout(
        &self,
        admin_key: String,
        body: Option<Value>,
        ip: String,
    ) -> BoxFuture<'_, Result<Reply, GuardError>>;
    /// `AcknowledgeOperationsAlert`.
    fn acknowledge_alert(
        &self,
        admin_key: String,
        alert_id: String,
        body: Option<Value>,
        ip: String,
    ) -> BoxFuture<'_, Result<Reply, GuardError>>;
}

/// `launcher.guardManifestRoles`: the artifacts realms add to the signed manifest.
pub const GUARD_MANIFEST_ROLES: &[&str] = &["TRANSPORT_MODULE", "TRANSPORT_RUNTIME", "ADMISSION_KEY"];

const COMPONENT: Component = Component::Guard;
const RATE_LIMIT_BUCKET_CACHE_MS: i64 = 10_000;
const RATE_LIMIT_BUCKET_CACHE_MAX: usize = 1024;
/// A store write lost to a concurrent one is retried this many times.
const CHAIN_ATTEMPTS: usize = 3;

/// What [`Guard`] is built from.
pub struct GuardParts {
    pub config: GuardConfig,
    pub store: Arc<dyn GuardStore>,
    pub hunts: Arc<dyn HuntSessions>,
    pub operations: Arc<dyn OperationsPolicyPort>,
    pub core: Arc<dyn IslesCore>,
    pub clock: Arc<dyn Clock>,
    pub log: Arc<dyn GuardLog>,
}

#[derive(Debug, Default)]
struct BucketCache {
    entries: HashMap<String, (Option<String>, i64)>,
    order: VecDeque<String>,
}

/// Guard in observe mode.
pub struct Guard {
    config: GuardConfig,
    store: Arc<dyn GuardStore>,
    hunts: Arc<dyn HuntSessions>,
    operations: Arc<dyn OperationsPolicyPort>,
    core: Arc<dyn IslesCore>,
    clock: Arc<dyn Clock>,
    log: Arc<dyn GuardLog>,
    local_traffic: Mutex<LocalTrafficLimiter>,
    budget: RequestBudget,
    census: Mutex<HostRouteCensus>,
    buckets: Mutex<BucketCache>,
}

const RUNTIME: Actor = Actor::System("guard");

fn host_runtime_actor(runtime: &RuntimeAuth) -> Actor {
    Actor::HostRuntime {
        session: runtime.session.clone(),
        host: runtime.host_account.clone(),
    }
}

/// `OPAQUE_CAPABILITY`: 32 random bytes in base64url.
fn opaque_capability(raw: &str) -> bool {
    raw.len() == 43 && raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl Guard {
    pub fn new(parts: GuardParts) -> Self {
        let census = HostRouteCensus::new(parts.config.census);
        Self {
            config: parts.config,
            store: parts.store,
            hunts: parts.hunts,
            operations: parts.operations,
            core: parts.core,
            clock: parts.clock,
            log: parts.log,
            local_traffic: Mutex::new(LocalTrafficLimiter::default()),
            budget: RequestBudget::new(HUNT_GUARD_REQUEST_BUDGET),
            census: Mutex::new(census),
            buckets: Mutex::new(BucketCache::default()),
        }
    }

    pub fn config(&self) -> &GuardConfig {
        &self.config
    }

    /// The in-flight Guard request budget the routes charge (`CentralGuardRequestBudget`).
    pub fn request_budget(&self) -> &RequestBudget {
        &self.budget
    }

    /// The census count of one answered host-runtime request. The seam's `census` hook sees the request before it is
    /// answered, so isles calls this when the response finishes.
    pub fn record_census(&self, sample: &HostRouteSample) {
        lock(&self.census).record(sample);
    }

    /// Ends the census window: logs its summary line when there was traffic. Isles' sweeper calls it every
    /// [`census::HOST_ROUTE_CENSUS_WINDOW_MS`].
    pub fn flush_census(&self) {
        if let Some(line) = lock(&self.census).flush() {
            self.log.info(&line);
        }
    }

    /// `rewards.policyStatus`: never ready on the stubs.
    pub fn reward_policy_status(&self, _hunt_id: &str, _expected_version: Option<&str>) -> RewardPolicyStatus {
        RewardPolicyStatus::NotReady {
            reason: "ANTICHEAT_MODULE_ABSENT",
        }
    }

    /// The chain row realms writes when it creates a hunt generation (`guardSeed` stays realms'; the zero head,
    /// sequence 0, `CREATED`, `OBSERVE_CLEAR` and the completion mode are Guard's).
    pub fn open_chain(
        &self,
        session: SessionId,
        generation: u64,
        completion_mode: Option<&str>,
    ) -> Result<(), GuardError> {
        if self.store.chain(&session, generation)?.is_some() {
            return Ok(());
        }
        let chain = ChainRecord::new(session, generation, completion_mode.and_then(CompletionMode::parse));
        let update = ChainUpdate {
            chain: ChainRecord { revision: 1, ..chain },
            batch: None,
            findings: Vec::new(),
        };
        self.store.apply_chain(&RUNTIME, &update, 0)?;
        Ok(())
    }

    /// The Guard fields of `GetHuntStatus` for one generation.
    pub fn chain_status(&self, session: &SessionId, generation: u64) -> Result<Option<ChainRecord>, GuardError> {
        Ok(self.store.chain(session, generation)?)
    }

    /// The hunt a runtime's capability belongs to, or the fence.
    fn runtime_hunt(&self, runtime: &RuntimeAuth) -> Result<HuntView, GuardError> {
        let hunt = self.hunts.hunt(&runtime.session)?.ok_or_else(fenced)?;
        if hunt.generation != runtime.generation || hunt.host.as_ref() != Some(&runtime.host_account) {
            return Err(fenced());
        }
        Ok(hunt)
    }

    fn note_finding(&self, finding: &FindingRecord) {
        self.log.warn(&format!(
            "[HuntGuardObserve] would refuse {} session={} generation={} sequence={}{}",
            finding.code,
            finding.session,
            finding.generation,
            finding
                .sequence
                .map_or_else(|| "-".to_owned(), |sequence| sequence.to_string()),
            if finding.detail.is_empty() {
                String::new()
            } else {
                format!(" {}", finding.detail)
            }
        ));
    }

    /// `hunts.submitEvidence` on a parsed body.
    pub fn submit_parsed(&self, runtime: &RuntimeAuth, body: Option<&Value>) -> Result<Json, GuardError> {
        if !self.config.hunt_evidence.enabled {
            return Err(not_collected());
        }
        let hunt = self.runtime_hunt(runtime)?;
        if hunt.is_hub() {
            return Err(not_collected());
        }
        let batch = evidence::parse_batch(body).map_err(evidence::BatchProblem::into_error)?;
        let roster: Vec<AccountId> = hunt.roster.iter().map(|entry| entry.account.clone()).collect();
        let actor = host_runtime_actor(runtime);
        for _ in 0..CHAIN_ATTEMPTS {
            let now = self.clock.now();
            let chain = self
                .store
                .chain(&runtime.session, runtime.generation)?
                .unwrap_or_else(|| ChainRecord::new(runtime.session.clone(), runtime.generation, None));
            if let Some(stored) = self
                .store
                .batch(&runtime.session, runtime.generation, &batch.batch_id)?
            {
                // The transport resends a batch it got no answer for: the same batch is taken again, quietly
                if stored.batch_hash == batch.batch_hash {
                    return Ok(Json(evidence::duplicate_reply(&chain, &stored)));
                }
                return Err(GuardError::refused(
                    "GUARD_BATCH_REPLAY",
                    "This Guard batch id was already used for other evidence.",
                ));
            }
            match evidence::apply_batch(&chain, &batch, &roster, now) {
                Ok(update) => {
                    if self.store.apply_chain(&actor, &update, chain.revision)? {
                        for finding in &update.findings {
                            self.note_finding(finding);
                        }
                        return Ok(Json(evidence::batch_reply(&update.chain)));
                    }
                }
                Err(refusal) => {
                    if let Some((code, detail)) = refusal.finding {
                        let update = evidence::finding_update(&chain, code, detail, now);
                        if self.store.apply_chain(&actor, &update, chain.revision)? {
                            for finding in &update.findings {
                                self.note_finding(finding);
                            }
                        }
                    }
                    return Err(refusal.error);
                }
            }
        }
        Err(GuardError::refused(
            "GUARD_CHAIN_CONFLICT",
            "The Guard batch does not continue this hunt generation's chain.",
        ))
    }

    /// `hunts.complete` on a parsed body.
    pub fn complete_parsed(&self, runtime: &RuntimeAuth, body: Option<&Value>) -> Result<Json, GuardError> {
        if !self.config.hunt_evidence.enabled {
            return Err(not_collected());
        }
        let hunt = self.runtime_hunt(runtime)?;
        if hunt.is_hub() {
            return Err(not_collected());
        }
        let request = evidence::parse_completion(body)?;
        let actor = host_runtime_actor(runtime);
        for _ in 0..CHAIN_ATTEMPTS {
            let Some(chain) = self.store.chain(&runtime.session, runtime.generation)? else {
                return Err(GuardError::refused(
                    "GUARD_COMPLETION_INVALID",
                    "The Guard chain has not requested completion.",
                ));
            };
            match evidence::apply_completion(&chain, &request, self.clock.now())? {
                None => return Ok(Json(evidence::completion_reply(&chain))),
                Some(update) => {
                    if self.store.apply_chain(&actor, &update, chain.revision)? {
                        let done = &update.chain;
                        self.log.info(&format!(
                            "[HuntGuardObserve] completion session={} generation={} lastSequence={} findings={} \
                             disposition={}",
                            done.session,
                            done.generation,
                            done.last_sequence,
                            done.finding_count,
                            done.disposition.as_str()
                        ));
                        return Ok(Json(evidence::completion_reply(done)));
                    }
                }
            }
        }
        Err(GuardError::refused(
            "GUARD_FINALIZATION_BUSY",
            "The Guard completion is busy; retry shortly.",
        ))
    }

    /// `GetSelfHuntReward`: a member of the hunt is told no rewards are granted here, as Node without the module.
    pub fn self_reward(&self, account: &AccountId, session: &str) -> Result<Json, GuardError> {
        let not_found = || GuardError::refused("P2P_SESSION_NOT_FOUND", "Hunt reward receipt not found.");
        let session: SessionId = session.parse().map_err(|_| not_found())?;
        let hunt = self.hunts.hunt(&session)?.ok_or_else(not_found)?;
        if !hunt.roster.iter().any(|entry| &entry.account == account) {
            return Err(not_found());
        }
        Err(GuardError::refused(
            "HUNT_REWARDS_UNAVAILABLE",
            "This server grants no player-hosted hunt rewards.",
        ))
    }

    fn charge_now(&self, host: &AccountId, session: &SessionId, bytes: u64) -> Result<TrafficDecision, GuardError> {
        let host_key = format!("guard-host:{host}");
        let session_key = format!("guard-session:{session}");
        let now = self.clock.now();
        if self.config.distributed_traffic {
            let host_window = traffic::reservation(
                TrafficScope::Host,
                &host_key,
                bytes,
                &HUNT_GUARD_HOST_TRAFFIC_LIMIT,
                now,
            );
            let session_window = traffic::reservation(
                TrafficScope::Session,
                &session_key,
                bytes,
                &HUNT_GUARD_SESSION_TRAFFIC_LIMIT,
                now,
            );
            let (Some(host_window), Some(session_window)) = (host_window, session_window) else {
                return Ok(TrafficDecision::RateLimited);
            };
            // Any storage failure is a bounded 503, never a silent local fallback
            return Ok(
                match self.store.reserve_traffic(&RUNTIME, &host_window, &session_window) {
                    Ok(true) => TrafficDecision::Accepted,
                    Ok(false) => TrafficDecision::RateLimited,
                    Err(_) => TrafficDecision::Unavailable,
                },
            );
        }
        let mut limiter = lock(&self.local_traffic);
        if !limiter.consume(&host_key, bytes, &HUNT_GUARD_HOST_TRAFFIC_LIMIT, now)
            || !limiter.consume(&session_key, bytes, &HUNT_GUARD_SESSION_TRAFFIC_LIMIT, now)
        {
            return Ok(TrafficDecision::RateLimited);
        }
        Ok(TrafficDecision::Accepted)
    }

    fn resolve_bucket(&self, raw: &str) -> Option<String> {
        if !opaque_capability(raw) {
            return None;
        }
        let digest = sha256_hex(raw.as_bytes());
        let now = self.clock.now().0;
        let mut cache = lock(&self.buckets);
        if let Some((bucket, at)) = cache.entries.get(&digest)
            && now - at < RATE_LIMIT_BUCKET_CACHE_MS
        {
            return bucket.clone();
        }
        let bucket = match self.hunts.capability_by_digest(&digest) {
            Ok(Some(capability)) if !capability.revoked && capability.expires_at.0 > now => {
                Some(format!("host:{}", capability.capability_id))
            }
            _ => None,
        };
        if cache.entries.len() >= RATE_LIMIT_BUCKET_CACHE_MAX
            && let Some(oldest) = cache.order.pop_front()
        {
            cache.entries.remove(&oldest);
        }
        if cache.entries.insert(digest.clone(), (bucket.clone(), now)).is_none() {
            cache.order.push_back(digest);
        }
        bucket
    }

    fn hub_presence(&self, hunt: &HuntView) -> Result<Option<Vec<AccountId>>, SeamError> {
        if !hunt.is_hub() {
            return Ok(None);
        }
        self.hunts
            .hub_present_accounts(&hunt.session, hunt.generation)
            .map(Some)
            .map_err(store_to_seam)
    }

    fn authorize_write_now(
        &self,
        runtime: &RuntimeAuth,
        kind: HostWriteKind,
        target: &AccountId,
        options: HostWriteOptions,
    ) -> Result<Option<Reply>, SeamError> {
        let name = kind_name(kind);
        let quiet = options.probe || options.parity;
        if !self.config.host_writes {
            if !quiet {
                self.log.warn(&format!(
                    "[HostWrite] refused kind={name} target={target} reason=HOST_WRITES_OFF"
                ));
            }
            return Ok(Some(write_refusal("Host writes are switched off.")));
        }
        // Dedicated parity is observe mode's only, never enforce's
        if options.parity && self.config.host_policy_refuses() {
            return Ok(Some(write_refusal(
                "This host may not do a dedicated server's work here.",
            )));
        }
        let hunt = self.hunts.hunt(&runtime.session).map_err(store_to_seam)?;
        let presence = match &hunt {
            Some(view) if view.session == runtime.session => self.hub_presence(view)?,
            _ => None,
        };
        let presence = presence.as_deref();
        let denied = "This host may not write this player's state.";
        if options.parity {
            if host_write_denial(runtime, hunt.as_ref(), target, Some(kind), presence, false).is_some() {
                return Ok(Some(write_refusal(denied)));
            }
            return Ok(None);
        }
        let hub_tag = hunt
            .as_ref()
            .and_then(HuntView::hub_kind)
            .map_or_else(String::new, |hub| format!(" hub={hub}"));
        let mut denial = host_write_denial(runtime, hunt.as_ref(), target, Some(kind), presence, true);
        if let Some(reason) = denial
            && reason.is_policy()
            && !options.probe
            && !self.config.host_policy_refuses()
        {
            let identity = host_write_denial(runtime, hunt.as_ref(), target, Some(kind), presence, false);
            if identity.is_none() {
                self.note_would_refuse_now(
                    &format!("write kind={name}"),
                    &format!(
                        "target={target} session={} reason={}{hub_tag}",
                        runtime.session,
                        reason.as_str()
                    ),
                );
                return Ok(None);
            }
            denial = identity;
        }
        if let Some(reason) = denial {
            if !options.probe {
                self.log.warn(&format!(
                    "[HostWrite] refused kind={name} target={target} session={} reason={}{hub_tag}",
                    runtime.session,
                    reason.as_str()
                ));
            }
            return Ok(Some(write_refusal(denied)));
        }
        Ok(None)
    }

    fn note_would_refuse_now(&self, what: &str, detail: &str) {
        self.log
            .warn(&format!("[HostPolicyObserve] would refuse {what} {detail}"));
    }

    fn record_write_now(&self, runtime: &RuntimeAuth, write: &HostWrite) -> Result<(), SeamError> {
        let name = kind_name(write.kind);
        // Node carries the authorized session on the request; here it is read again from the runtime's own
        let hunt = match self.hunts.hunt(&runtime.session) {
            Ok(Some(hunt)) if hunt.generation == runtime.generation => hunt,
            Ok(_) => {
                self.log.warn(&format!(
                    "[HostWrite] record without authorize kind={name} target={}",
                    write.target
                ));
                return Ok(());
            }
            Err(error) => return Err(store_to_seam(error)),
        };
        let host = hunt.host.clone().unwrap_or_else(|| AccountId(String::new()));
        self.log.info(&format!(
            "[HostWrite] kind={name} target={} host={host} session={} generation={}{}{} summary={}",
            write.target,
            hunt.session,
            hunt.generation,
            hunt.realm
                .as_ref()
                .map_or_else(String::new, |realm| format!(" realm={realm}")),
            hunt.hub_kind().map_or_else(String::new, |hub| format!(" hub={hub}")),
            js_slice(&write.summary.0, 400)
        ));
        let record = HostWriteRecord {
            kind: write.kind,
            target: write.target.clone(),
            host,
            session: hunt.session.clone(),
            generation: hunt.generation,
            realm: hunt.realm.clone(),
            session_kind: hunt.session_kind().map(str::to_owned),
            hub_kind: hunt.hub_kind().map(str::to_owned),
            summary: write.summary.clone(),
            at: self.clock.now(),
        };
        if let Err(error) = self.store.record_host_write(&host_runtime_actor(runtime), &record) {
            self.log.error(&format!(
                "[HostWrite] tag not stored kind={name} target={} session={}: {error}",
                write.target, hunt.session
            ));
        }
        Ok(())
    }

    fn authorize_inventory_now(
        &self,
        runtime: &RuntimeAuth,
        transaction: &InventoryTransaction,
    ) -> Result<InventoryDecision, SeamError> {
        let hunt = self.hunts.hunt(&runtime.session).map_err(store_to_seam)?;
        let presence = match &hunt {
            Some(view) => self.hub_presence(view)?,
            None => None,
        };
        let parsed_body = transaction.body.as_ref().and_then(parse_json_text);
        if let Some(denial) = inventory_body_denial(
            runtime,
            hunt.as_ref(),
            &transaction.user,
            &transaction.character_id,
            parsed_body.as_ref(),
            presence.as_deref(),
        ) {
            self.log.warn(&format!(
                "[P2PInventory] rejected code={denial} account={} session={} generation={}",
                transaction.user, runtime.session, runtime.generation
            ));
            return Ok(InventoryDecision::Refused(loot_refusal("P2P_LOOT_GRANT_DENIED")));
        }
        let mut tags = format!(
            "account={} character={} session={} generation={} transaction={}",
            transaction.user, transaction.character_id, runtime.session, runtime.generation, transaction.transaction_id
        );
        if let Some(hunt) = &hunt {
            if let Some(realm) = &hunt.realm {
                tags.push_str(&format!(" realm={realm}"));
            }
            if let Some(hub) = hunt.hub_kind() {
                tags.push_str(&format!(" hub={hub}"));
            }
        }
        Ok(InventoryDecision::Authorized(Box::new(HuntFence {
            hunts: Arc::clone(&self.hunts),
            log: Arc::clone(&self.log),
            runtime: runtime.clone(),
            transaction: transaction.clone(),
            tags,
        })))
    }
}

/// The checks repeated inside the inventory transaction: a member who left between the first check and the commit
/// is seen.
struct HuntFence {
    hunts: Arc<dyn HuntSessions>,
    log: Arc<dyn GuardLog>,
    runtime: RuntimeAuth,
    transaction: InventoryTransaction,
    tags: String,
}

impl InventoryFence for HuntFence {
    fn on_grant(&self, grant: &Json) -> Result<(), Reply> {
        let refuse = |code: &str| {
            self.log
                .warn(&format!("[P2PInventory] transaction denied code={code} {}", self.tags));
            Err(loot_refusal(code))
        };
        let Ok(hunt) = self.hunts.hunt(&self.runtime.session) else {
            return refuse("P2P_LOOT_GRANT_DENIED");
        };
        let presence = match &hunt {
            Some(view) if view.is_hub() => match self.hunts.hub_present_accounts(&view.session, view.generation) {
                Ok(present) => Some(present),
                Err(_) => return refuse("P2P_LOOT_GRANT_DENIED"),
            },
            _ => None,
        };
        let parsed_body = self
            .transaction
            .body
            .as_ref()
            .and_then(parse_json_text)
            .or_else(|| parse_json_text(grant));
        match inventory_body_denial(
            &self.runtime,
            hunt.as_ref(),
            &self.transaction.user,
            &self.transaction.character_id,
            parsed_body.as_ref(),
            presence.as_deref(),
        ) {
            Some(denial) => refuse(&format!("P2P_LOOT_GRANT_{denial}")),
            None => Ok(()),
        }
    }

    fn report(&self, phase: TransactionPhase) {
        match phase {
            TransactionPhase::Applying => self.log.info(&format!("[P2PInventory] applying {}", self.tags)),
            TransactionPhase::Accepted => self
                .log
                .info(&format!("[P2PInventory] transaction accepted {}", self.tags)),
            TransactionPhase::Failed => self
                .log
                .error(&format!("[P2PInventory] transaction failed {}", self.tags)),
        }
    }
}

fn ready<T: Send + 'static, E: Send + 'static>(value: Result<T, E>) -> BoxFuture<'static, Result<T, E>> {
    Box::pin(async move { value })
}

fn parse_json_text(text: &Json) -> Option<Value> {
    serde_json::from_str(&text.0).ok()
}

impl Lifecycle for Guard {
    /// The anticheat stub adds none; the distributed limiter's rule is realms' `P2PRuntimeSafetyIssues`.
    fn runtime_safety_issues(&self, _: &dyn Settings) -> Result<Vec<String>, SeamError> {
        Ok(Vec::new())
    }

    fn startup_summary(&self, _: &str) -> Result<Option<Json>, SeamError> {
        Ok(Some(Json(format!(
            r#"{{"antiCheatLoaded":false,"hostPolicy":"{}","huntEvidence":{},"trustTier":{}}}"#,
            self.config.host_policy.mode.as_str(),
            self.config.hunt_evidence.enabled,
            self.config.trust_tier.number()
        ))))
    }

    fn start(&self, _: &str) -> Result<(), SeamError> {
        log_host_policy(&self.config.host_policy, self.log.as_ref());
        log_hunt_evidence(&self.config.hunt_evidence, self.log.as_ref());
        if self.config.host_policy_refuses() {
            // RF6 gap: the seam's inventory hook carries no body, so the grant-class and draft-token rules wait on RU4
            self.log.warn(
                "[Startup] WARNING: MYSTICPARADOX_HOST_POLICY=enforce refuses host write kinds only; host grant \
                 classes and the draft token cap are not checked by rifts-guard yet",
            );
        }
        Ok(())
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        self.flush_census();
        ready(Ok(()))
    }

    fn tables(&self) -> &'static [TableSpec] {
        TABLES
    }

    /// `p2pHostWrites` about the account or written by it, and its XP summaries (`accountDeletionSteps.ts`).
    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        vec![
            DeletionStep {
                table: "rifts_host_writes",
                account_column: "target",
                action: DeletionAction::Delete,
            },
            DeletionStep {
                table: "rifts_host_writes",
                account_column: "host",
                action: DeletionAction::Delete,
            },
            DeletionStep {
                table: "rifts_hunt_xp_summaries",
                account_column: "member",
                action: DeletionAction::Delete,
            },
        ]
    }
}

impl GuardHooks for Guard {
    fn service_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        GUARD_MANIFEST_ROLES
    }
    /// Records one answered host-runtime request in the host route census.
    fn census(&self, sample: &HostRouteSample) -> Result<(), SeamError> {
        self.record_census(sample);
        Ok(())
    }
    fn rate_limit_bucket(&self, request: RequestInfo) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        let bucket = request
            .header(HOST_CAPABILITY_HEADER)
            .and_then(|raw| self.resolve_bucket(raw));
        ready(Ok(bucket))
    }
    fn authorize_inventory(
        &self,
        runtime: RuntimeAuth,
        transaction: InventoryTransaction,
    ) -> BoxFuture<'_, Result<InventoryDecision, SeamError>> {
        ready(self.authorize_inventory_now(&runtime, &transaction))
    }
    fn authorize_host_write(
        &self,
        runtime: RuntimeAuth,
        kind: HostWriteKind,
        target: AccountId,
        options: HostWriteOptions,
    ) -> BoxFuture<'_, Result<Option<Reply>, SeamError>> {
        ready(self.authorize_write_now(&runtime, kind, &target, options))
    }
    fn record_host_write(&self, runtime: RuntimeAuth, write: HostWrite) -> BoxFuture<'_, Result<(), SeamError>> {
        ready(self.record_write_now(&runtime, &write))
    }
    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        Ok(self.config.host_policy_refuses())
    }
    fn note_would_refuse(&self, what: &str, detail: &str) -> Result<(), SeamError> {
        self.note_would_refuse_now(what, detail);
        Ok(())
    }
}

impl EvidenceIntake for Guard {
    fn submit_evidence(&self, runtime: RuntimeAuth, batch: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        ready(self.submit_parsed(&runtime, parse_json_text(&batch).as_ref()))
    }
    fn complete(&self, runtime: RuntimeAuth, request: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        ready(self.complete_parsed(&runtime, parse_json_text(&request).as_ref()))
    }
    fn verdict(&self, session: SessionId, generation: u64) -> BoxFuture<'_, Result<Option<HuntVerdict>, GuardError>> {
        let verdict = self
            .store
            .chain(&session, generation)
            .map(|chain| chain.and_then(|chain| chain.completion).map(|_| HuntVerdict::OBSERVED))
            .map_err(GuardError::from);
        ready(verdict)
    }
    /// No rewards are granted on the stubs, so there is never a receipt.
    fn reward_receipt(&self, _: SessionId, _: u64, _: AccountId) -> BoxFuture<'_, Result<Option<Json>, GuardError>> {
        ready(Ok(None))
    }
    /// The shadow comparison is the private module's; the key is never read, compared or logged here.
    fn record_shadow_comparison(&self, _: Option<Secret>, _: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        ready(Err(not_collected()))
    }
}

impl TrafficLimits for Guard {
    fn charge(
        &self,
        host: AccountId,
        session: SessionId,
        bytes: u64,
    ) -> BoxFuture<'_, Result<TrafficDecision, GuardError>> {
        ready(self.charge_now(&host, &session, bytes))
    }
}

impl GuardOperations for Guard {
    fn set_reward_rollout(
        &self,
        admin_key: String,
        body: Option<Value>,
        ip: String,
    ) -> BoxFuture<'_, Result<Reply, GuardError>> {
        Box::pin(async move {
            let Some(request) = operations::parse_rollout_request(body.as_ref()) else {
                return Ok(operations::rollout_validation_error());
            };
            let request_id = self.clock.uuid();
            let before = self.operations.policy().await?;
            if before.version != request.expected_version {
                return Ok(operations::policy_conflict(&before.json));
            }
            if request.rollout.version <= before.reward_rollout.version {
                return Ok(operations::rollout_conflict(&before.json));
            }
            let codes = operations::weakening_codes(&before.reward_rollout, &request.rollout);
            if !codes.is_empty() {
                let stepped_up = match &request.break_glass_totp {
                    Some(code) => {
                        self.core
                            .verify_operations_step_up(admin_key.clone(), code.clone())
                            .await?
                    }
                    None => false,
                };
                if !stepped_up {
                    self.operations
                        .append_audit(AuditEntry {
                            id: self.clock.uuid(),
                            actor: admin_key.clone(),
                            action: "operations.policy.weakening_rejected".to_owned(),
                            old_state: before.json.clone(),
                            new_state: Json(format!(
                                r#"{{"rewardRollout":{},"weakeningCodes":{}}}"#,
                                request.rollout.to_json().0,
                                operations::codes_json(&codes)
                            )),
                            reason: request.reason.clone(),
                            ip,
                            request_id,
                            created_at: iso_time(self.clock.now()),
                        })
                        .await?;
                    return Ok(operations::step_up_required(&before.json, &codes));
                }
            }
            let Some(after) = self
                .operations
                .set_reward_rollout(request.rollout, admin_key.clone(), request.expected_version)
                .await?
            else {
                return Ok(operations::policy_conflict(&before.json));
            };
            self.operations
                .append_audit(AuditEntry {
                    id: self.clock.uuid(),
                    actor: admin_key,
                    action: "operations.reward_rollout.update".to_owned(),
                    old_state: before.json,
                    new_state: operations::with_weakening_codes(&after.json, &codes),
                    reason: request.reason,
                    ip,
                    request_id: request_id.clone(),
                    created_at: iso_time(self.clock.now()),
                })
                .await?;
            Ok(operations::rollout_updated(&after.json, &request_id))
        })
    }

    fn acknowledge_alert(
        &self,
        admin_key: String,
        alert_id: String,
        body: Option<Value>,
        ip: String,
    ) -> BoxFuture<'_, Result<Reply, GuardError>> {
        Box::pin(async move {
            let Some((alert_id, reason)) = operations::parse_ack(&alert_id, body.as_ref()) else {
                return Ok(operations::ack_validation_error());
            };
            let request_id = self.clock.uuid();
            let at = self.clock.now();
            let acknowledged_at = iso_time(at);
            if !self
                .store
                .acknowledge_alert(&operations::admin_actor(&admin_key), &alert_id, at)?
            {
                return Ok(operations::alert_not_open());
            }
            let alert = json::js_string(&alert_id);
            self.operations
                .append_audit(AuditEntry {
                    id: self.clock.uuid(),
                    actor: admin_key,
                    action: "operations.alert.acknowledge".to_owned(),
                    old_state: Json(format!(r#"{{"alertId":{alert},"status":"OPEN"}}"#)),
                    new_state: Json(format!(
                        r#"{{"alertId":{alert},"status":"ACKNOWLEDGED","acknowledgedAt":"{acknowledged_at}"}}"#
                    )),
                    reason,
                    ip,
                    request_id: request_id.clone(),
                    created_at: acknowledged_at.clone(),
                })
                .await?;
            Ok(operations::alert_acknowledged(&alert_id, &acknowledged_at, &request_id))
        })
    }
}

/// Every operation answers [`rifts_seam::Unimplemented`] (RD18): for a module built with Guard left out.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

fn refuse<T>(operation: &'static str) -> Result<T, SeamError> {
    Err(SeamError::unimplemented(COMPONENT, operation))
}

impl Lifecycle for Stub {
    fn runtime_safety_issues(&self, _: &dyn Settings) -> Result<Vec<String>, SeamError> {
        refuse("runtime_safety_issues")
    }
    fn startup_summary(&self, _: &str) -> Result<Option<Json>, SeamError> {
        refuse("startup_summary")
    }
    fn start(&self, _: &str) -> Result<(), SeamError> {
        refuse("start")
    }
    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        stub(COMPONENT, "stop")
    }
    fn tables(&self) -> &'static [TableSpec] {
        TABLES
    }
    // A stub stores nothing, so it has nothing to delete
    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        Vec::new()
    }
}

impl GuardHooks for Stub {
    fn service_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        GUARD_MANIFEST_ROLES
    }
    fn census(&self, _: &HostRouteSample) -> Result<(), SeamError> {
        refuse("census")
    }
    fn rate_limit_bucket(&self, _: RequestInfo) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        stub(COMPONENT, "rate_limit_bucket")
    }
    fn authorize_inventory(
        &self,
        _: RuntimeAuth,
        _: InventoryTransaction,
    ) -> BoxFuture<'_, Result<InventoryDecision, SeamError>> {
        stub(COMPONENT, "authorize_inventory")
    }
    fn authorize_host_write(
        &self,
        _: RuntimeAuth,
        _: HostWriteKind,
        _: AccountId,
        _: HostWriteOptions,
    ) -> BoxFuture<'_, Result<Option<Reply>, SeamError>> {
        stub(COMPONENT, "authorize_host_write")
    }
    fn record_host_write(&self, _: RuntimeAuth, _: HostWrite) -> BoxFuture<'_, Result<(), SeamError>> {
        stub(COMPONENT, "record_host_write")
    }
    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        refuse("host_policy_refuses")
    }
    fn note_would_refuse(&self, _: &str, _: &str) -> Result<(), SeamError> {
        refuse("note_would_refuse")
    }
}

impl EvidenceIntake for Stub {
    fn submit_evidence(&self, _: RuntimeAuth, _: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        stub(COMPONENT, "submit_evidence")
    }
    fn complete(&self, _: RuntimeAuth, _: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        stub(COMPONENT, "complete")
    }
    fn verdict(&self, _: SessionId, _: u64) -> BoxFuture<'_, Result<Option<HuntVerdict>, GuardError>> {
        stub(COMPONENT, "verdict")
    }
    fn reward_receipt(&self, _: SessionId, _: u64, _: AccountId) -> BoxFuture<'_, Result<Option<Json>, GuardError>> {
        stub(COMPONENT, "reward_receipt")
    }
    fn record_shadow_comparison(&self, _: Option<Secret>, _: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        stub(COMPONENT, "record_shadow_comparison")
    }
}

impl TrafficLimits for Stub {
    fn charge(&self, _: AccountId, _: SessionId, _: u64) -> BoxFuture<'_, Result<TrafficDecision, GuardError>> {
        stub(COMPONENT, "charge")
    }
}

impl GuardOperations for Stub {
    fn set_reward_rollout(&self, _: String, _: Option<Value>, _: String) -> BoxFuture<'_, Result<Reply, GuardError>> {
        stub(COMPONENT, "set_reward_rollout")
    }
    fn acknowledge_alert(
        &self,
        _: String,
        _: String,
        _: Option<Value>,
        _: String,
    ) -> BoxFuture<'_, Result<Reply, GuardError>> {
        stub(COMPONENT, "acknowledge_alert")
    }
}

#[cfg(test)]
pub(crate) mod testkit;

#[cfg(test)]
mod tests;
