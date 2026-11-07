// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Rifts' Guard in observe mode: the Hunt Guard evidence chain a host's transport posts, whom a hosted
//! runtime may write player state for, the item classes and the daily draft token a host may grant, the bounds on
//! Guard traffic, the host route census and two admin operations routes. Nothing here grants or withholds a
//! reward: the chain is observed, and every would-be refusal of the host policy is logged unless it enforces.

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")
)]

pub mod census;
pub mod evidence;
pub mod grants;
pub mod json;
pub mod ports;
pub mod sql;
pub mod store;

mod config;
mod digest;
mod errors;
mod fence;
mod operations;
mod routes;
mod traffic;
mod writes;

#[cfg(test)]
mod testkit;
#[cfg(test)]
mod tests;

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use rustic_rifts_seam::{
    AccountId, Actor, BoxFuture, Component, DeletionAction, DeletionStep, GuardHooks, HOST_CAPABILITY_HEADER,
    HostWrite, HostWriteKind, HostWriteOptions, InventoryDecision, InventoryTransaction, IslesCore, Json, Lifecycle,
    Reply, RequestInfo, RuntimeAuth, SeamError, Secret, SessionId, Settings, StoreError, TableSpec, Unimplemented,
    UnixMillis,
};
use serde_json::Value;

pub use crate::{
    census::{HostRouteCensus, HostRouteRow, HostRouteSummary},
    config::{
        GuardConfig, HostPolicyMode, HostPolicySetting, HuntEvidenceSetting, Tier1Policy, TrustTier, log_host_policy,
        log_hunt_evidence, resolve_host_policy, resolve_hunt_evidence,
    },
    digest::sha256_hex,
    errors::{ERROR_STATUSES, GuardError, envelope, status_of},
    operations::{DEFAULT_REWARD_ROLLOUT, RewardRollout},
    ports::{
        AuditEntry, CapabilityView, Clock, CoreOperationsPolicy, GuardLog, HuntSessions, HuntView, ManualClock,
        MemoryLog, MemoryOperations, NoHuntSessions, OperationsPolicyPort, PolicySnapshot, RosterEntry, StderrLog,
        SystemClock, iso_time,
    },
    routes::{GuardRoutes, ROUTES, stub_routes},
    sql::{POSTGRES_MIGRATIONS, SCHEMA, SQLITE_MIGRATIONS, SqlGuardStore, block_on_store, store_actor},
    store::{
        AlertRecord, BatchRecord, ChainRecord, ChainUpdate, CompletionMode, CompletionRecord, Disposition,
        EncounterState, FindingRecord, GENESIS_HASH, GuardStore, HostWriteRecord, MemoryGuardStore, StubGuardStore,
        TABLES, TrafficReservation, TrafficScope, TrafficWindowRecord, XP_SUMMARY_TTL_MS, XpSummaryRecord,
    },
    traffic::{
        BudgetPermit, HUNT_GUARD_HOST_TRAFFIC_LIMIT, HUNT_GUARD_REQUEST_BUDGET, HUNT_GUARD_SESSION_TRAFFIC_LIMIT,
        HuntOperation, LocalTrafficLimiter, RequestBudget, RequestBudgetLimit, TrafficLimit,
    },
    writes::{
        WriteDenial, host_write_denial, inventory_body_denial, inventory_denial, kind_name, loot_refusal, write_refusal,
    },
};
pub use rustic_rifts_seam::HostRouteSample;

use crate::{
    evidence::{
        BatchProblem, CONFLICT, EvidenceBatch, NOT_REQUESTED, apply_batch, apply_completion, batch_reply,
        completion_reply, finding_update, parse_batch, parse_completion,
    },
    fence::GuardFence,
    grants::{
        Allowances, DraftPremiumDecision, GrantSession, NoGameData, body_adds_host_draft_premium, decide_draft_premium,
        grant_denial, stacked_balance_of,
    },
    json::js_slice,
    operations::Operations,
    traffic::{host_key, reservation, session_key},
};

/// The runtime files a Guard manifest names.
pub const GUARD_MANIFEST_ROLES: &[&str] = &["TRANSPORT_MODULE", "TRANSPORT_RUNTIME", "ADMISSION_KEY"];

/// Guard's own writes: chains it opens and traffic it charges.
const GUARD_ACTOR: Actor = Actor::System("guard");
/// Store races a chain write retries before giving up.
const ATTEMPTS: usize = 3;
/// The rate-limit bucket cache: entries kept, and how long one answers.
const BUCKET_CACHE_ENTRIES: usize = 1024;
const BUCKET_CACHE_MS: i64 = 10_000;

/// Guard's own refusals, beside the route gates and the evidence module's.
#[derive(Clone, Copy)]
enum Refuse {
    NotCollected,
    Fenced,
    Replay,
    Busy,
    Conflict,
    NotRequested,
    NoReceipt,
    NoRewards,
}

impl Refuse {
    fn error(self) -> GuardError {
        let (code, text) = match self {
            Self::NotCollected => (
                "HUNT_EVIDENCE_NOT_COLLECTED",
                "This server does not collect hunt evidence.",
            ),
            Self::Fenced => (
                "P2P_GENERATION_FENCED",
                "The hunt generation no longer belongs to this host.",
            ),
            Self::Replay => (
                "GUARD_BATCH_REPLAY",
                "This Guard batch id was already used for other evidence.",
            ),
            Self::Busy => (
                "GUARD_FINALIZATION_BUSY",
                "The Guard completion is busy; retry shortly.",
            ),
            Self::Conflict => ("GUARD_CHAIN_CONFLICT", CONFLICT),
            Self::NotRequested => ("GUARD_COMPLETION_INVALID", NOT_REQUESTED),
            Self::NoReceipt => ("P2P_SESSION_NOT_FOUND", "Hunt reward receipt not found."),
            Self::NoRewards => (
                "HUNT_REWARDS_UNAVAILABLE",
                "This server grants no player-hosted hunt rewards.",
            ),
        };
        GuardError::refused(code, text)
    }
}

/// What Guard says a completed hunt earned: in observe mode, nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HuntVerdict {
    /// Whether the hunt's progression was applied; always false in observe mode.
    pub progression_applied: bool,
    /// Whether the hunt's rewards were applied; always false in observe mode.
    pub rewards_applied: bool,
}

impl HuntVerdict {
    /// Nothing applied, the only verdict observe mode gives.
    pub const OBSERVED: Self = Self {
        progression_applied: false,
        rewards_applied: false,
    };

    /// The verdict as `{"progressionApplied":..,"rewardsApplied":..}`.
    pub fn to_json(&self) -> Json {
        let Self {
            progression_applied: progression,
            rewards_applied: rewards,
        } = self;
        Json(format!(
            r#"{{"progressionApplied":{progression},"rewardsApplied":{rewards}}}"#
        ))
    }
}

/// Whether a hunt has a reward policy Guard could grant against.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RewardPolicyStatus {
    /// A policy is loaded.
    Ready {
        /// The policy's version.
        version: String,
        /// The policy's digest.
        digest: String,
    },
    /// No policy is loaded.
    NotReady {
        /// Why, such as `ANTICHEAT_MODULE_ABSENT`.
        reason: &'static str,
    },
}

/// The Hunt Guard evidence routes a host's transport and the launcher call.
pub trait EvidenceIntake: Send + Sync {
    /// Appends one evidence batch to the runtime's chain and answers with its new head.
    fn submit_evidence(&self, runtime: RuntimeAuth, body: Json) -> BoxFuture<'_, Result<Json, GuardError>>;
    /// Closes the runtime's chain at its final sequence and hash.
    fn complete(&self, runtime: RuntimeAuth, body: Json) -> BoxFuture<'_, Result<Json, GuardError>>;
    /// What a completed generation earned, `None` before completion.
    fn verdict(&self, session: SessionId, generation: u64) -> BoxFuture<'_, Result<Option<HuntVerdict>, GuardError>>;
    /// A member's reward receipt for a generation, `None` when there is none.
    fn reward_receipt(
        &self,
        session: SessionId,
        generation: u64,
        account: AccountId,
    ) -> BoxFuture<'_, Result<Option<Json>, GuardError>>;
    /// Records a shadow reward comparison, keyed by the shadow key.
    fn record_shadow_comparison(&self, key: Option<Secret>, body: Json) -> BoxFuture<'_, Result<Json, GuardError>>;
}

/// The answer to a charge against the Guard traffic limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrafficDecision {
    /// Within both the host's and the session's windows.
    Accepted,
    /// Over a window's request or byte bound.
    RateLimited,
    /// The shared store could not be reached.
    Unavailable,
}

/// Bounds the Guard traffic each host and session may send.
pub trait TrafficLimits: Send + Sync {
    /// Charges one request of `bytes` to the host's and the session's windows.
    fn charge(
        &self,
        host: AccountId,
        session: SessionId,
        bytes: u64,
    ) -> BoxFuture<'_, Result<TrafficDecision, GuardError>>;
}

/// The admin operations routes.
pub trait GuardOperations: Send + Sync {
    /// Sets the reward rollout flags behind the admin key, auditing the change from `ip`.
    fn set_reward_rollout(
        &self,
        admin_key: String,
        body: Option<Value>,
        ip: String,
    ) -> BoxFuture<'_, Result<Reply, GuardError>>;
    /// Acknowledges an operator alert behind the admin key, auditing it from `ip`.
    fn acknowledge_alert(
        &self,
        admin_key: String,
        alert_id: String,
        body: Option<Value>,
        ip: String,
    ) -> BoxFuture<'_, Result<Reply, GuardError>>;
}

/// What Guard is built from.
pub struct GuardParts {
    /// Guard's settings.
    pub config: GuardConfig,
    /// Chains, traffic windows, host writes and alerts.
    pub store: Arc<dyn GuardStore>,
    /// The hunt sessions Guard checks runtimes against.
    pub hunts: Arc<dyn HuntSessions>,
    /// The operations policy and its audit log.
    pub operations: Arc<dyn OperationsPolicyPort>,
    /// The Isles core Guard answers for.
    pub core: Arc<dyn IslesCore>,
    /// The time and new record ids.
    pub clock: Arc<dyn Clock>,
    /// Where Guard's lines go.
    pub log: Arc<dyn GuardLog>,
}

/// One cached rate-limit bucket answer, by the capability's digest.
struct Bucket {
    digest: String,
    at: UnixMillis,
    answer: Option<String>,
}

/// Rustic Rifts' Guard: the evidence chain, host policy, traffic bounds and census over one store.
pub struct Guard {
    parts: GuardParts,
    census: HostRouteCensus,
    budget: RequestBudget,
    local: Mutex<LocalTrafficLimiter>,
    buckets: Mutex<VecDeque<Bucket>>,
}

/// 43 characters of base64url: the hosted runtime's capability token.
pub(crate) fn capability_shaped(value: &str) -> bool {
    let base64url = |b: u8| b.is_ascii_alphanumeric() || b"-_".contains(&b);
    value.len() == 43 && value.bytes().all(base64url)
}

fn seam(error: StoreError) -> SeamError {
    GuardError::from(error).into()
}

fn held<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Guard {
    /// A Guard over these parts; the census switch comes from the config.
    pub fn new(parts: GuardParts) -> Self {
        Self {
            census: HostRouteCensus::new(parts.config.census),
            budget: RequestBudget::new(HUNT_GUARD_REQUEST_BUDGET),
            local: Mutex::default(),
            buckets: Mutex::default(),
            parts,
        }
    }

    /// The settings Guard was built with.
    pub fn config(&self) -> &GuardConfig {
        &self.parts.config
    }

    /// The in-flight request bound the hunt routes share.
    pub fn request_budget(&self) -> &RequestBudget {
        &self.budget
    }

    pub(crate) fn log(&self) -> &dyn GuardLog {
        &*self.parts.log
    }

    pub(crate) fn hunts(&self) -> &dyn HuntSessions {
        &*self.parts.hunts
    }

    pub(crate) fn now(&self) -> UnixMillis {
        self.parts.clock.now()
    }

    fn store(&self) -> &dyn GuardStore {
        &*self.parts.store
    }

    fn refuses(&self) -> bool {
        self.parts.config.host_policy_refuses()
    }

    /// Counts one hosted runtime's route call in the census.
    pub fn record_census(&self, sample: &HostRouteSample) {
        self.census.record(sample);
    }

    /// Logs the census window's summary line, if it saw traffic, and starts a new window.
    pub fn flush_census(&self) {
        let Some(line) = self.census.flush() else {
            return;
        };
        self.log().info(&line);
    }

    /// No reward module is loaded, so no hunt has a reward policy.
    pub fn reward_policy_status(&self, hunt_id: &str, expected_version: Option<&str>) -> RewardPolicyStatus {
        let _ = (hunt_id, expected_version);
        RewardPolicyStatus::NotReady {
            reason: "ANTICHEAT_MODULE_ABSENT",
        }
    }

    /// Writes a fresh chain unless one exists; losing the race to another writer is fine.
    pub fn open_chain(
        &self,
        session: SessionId,
        generation: u64,
        completion_mode: Option<&str>,
    ) -> Result<(), GuardError> {
        if self.store().chain(&session, generation)?.is_none() {
            let mode = completion_mode.and_then(CompletionMode::parse);
            let fresh = ChainUpdate {
                chain: ChainRecord {
                    revision: 1,
                    ..ChainRecord::new(session, generation, mode)
                },
                batch: None,
                findings: vec![],
            };
            self.store().apply_chain(&GUARD_ACTOR, &fresh, 0)?;
        }
        Ok(())
    }

    /// The chain of this hunt generation as stored, `None` when none was opened.
    pub fn chain_status(&self, session: &SessionId, generation: u64) -> Result<Option<ChainRecord>, GuardError> {
        self.store().chain(session, generation).map_err(GuardError::from)
    }

    /// The hunt the runtime may post evidence for: this generation, this host, and not a hub.
    fn evidence_gate(&self, runtime: &RuntimeAuth) -> Result<HuntView, GuardError> {
        if !self.parts.config.hunt_evidence.enabled {
            return Err(Refuse::NotCollected.error());
        }
        let ours = |hunt: &HuntView| {
            hunt.generation == runtime.generation && hunt.host.as_ref() == Some(&runtime.host_account)
        };
        match self.hunts().hunt(&runtime.session)?.filter(ours) {
            None => Err(Refuse::Fenced.error()),
            Some(hunt) if hunt.is_hub() => Err(Refuse::NotCollected.error()),
            Some(hunt) => Ok(hunt),
        }
    }

    fn runtime_actor(runtime: &RuntimeAuth) -> Actor {
        let (session, host) = (runtime.session.clone(), runtime.host_account.clone());
        Actor::HostRuntime { session, host }
    }

    fn log_findings(&self, findings: &[FindingRecord]) {
        for finding in findings {
            let sequence = finding.sequence.map(|s| s.to_string()).unwrap_or_else(|| "-".into());
            let detail = if finding.detail.is_empty() {
                String::new()
            } else {
                format!(" {}", finding.detail)
            };
            let (code, session, generation) = (&finding.code, &finding.session, finding.generation);
            self.log().warn(&format!(
                "[HuntGuardObserve] would refuse {code} session={session} generation={generation} \
                 sequence={sequence}{detail}"
            ));
        }
    }

    /// One try at moving the chain by `batch`; `None` when another writer moved it first.
    fn try_batch(
        &self,
        actor: &Actor,
        runtime: &RuntimeAuth,
        batch: &EvidenceBatch,
        roster: &[AccountId],
    ) -> Result<Option<Json>, GuardError> {
        let (session, generation) = (&runtime.session, runtime.generation);
        let chain = match self.store().chain(session, generation)? {
            Some(chain) => chain,
            None => ChainRecord::new(session.clone(), generation, None),
        };
        // A batch id seen before answers as it did, if it carries the same evidence.
        if let Some(stored) = self.store().batch(session, generation, &batch.batch_id)? {
            return if stored.batch_hash == batch.batch_hash {
                let reply = batch_reply(
                    session,
                    generation,
                    &stored.batch_hash,
                    stored.last_sequence,
                    &chain,
                    true,
                );
                Ok(Some(reply))
            } else {
                Err(Refuse::Replay.error())
            };
        }
        let now = self.now();
        let refusal = match apply_batch(&chain, batch, roster, now) {
            Ok(update) => {
                if !self.store().apply_chain(actor, &update, chain.revision)? {
                    return Ok(None);
                }
                self.log_findings(&update.findings);
                let next = &update.chain;
                return Ok(Some(batch_reply(
                    session,
                    generation,
                    &next.head_hash,
                    next.last_sequence,
                    next,
                    false,
                )));
            }
            Err(refusal) => refusal,
        };
        if let Some((code, detail)) = &refusal.finding {
            let noted = finding_update(&chain, code, detail, now);
            if self.store().apply_chain(actor, &noted, chain.revision)? {
                self.log_findings(&noted.findings);
            }
        }
        Err(refusal.error)
    }

    /// Appends a parsed batch body to the runtime's chain, retrying a lost race a few times.
    pub fn submit_parsed(&self, runtime: &RuntimeAuth, body: Option<&Value>) -> Result<Json, GuardError> {
        let hunt = self.evidence_gate(runtime)?;
        let batch = parse_batch(body).map_err(BatchProblem::into_error)?;
        let roster: Vec<AccountId> = hunt.roster.into_iter().map(|entry| entry.account).collect();
        let actor = Self::runtime_actor(runtime);
        for _ in 0..ATTEMPTS {
            if let Some(reply) = self.try_batch(&actor, runtime, &batch, &roster)? {
                return Ok(reply);
            }
        }
        Err(Refuse::Conflict.error())
    }

    /// Completes the runtime's chain from a parsed body, retrying a lost race a few times.
    pub fn complete_parsed(&self, runtime: &RuntimeAuth, body: Option<&Value>) -> Result<Json, GuardError> {
        self.evidence_gate(runtime)?;
        let request = parse_completion(body)?;
        let actor = Self::runtime_actor(runtime);
        for _ in 0..ATTEMPTS {
            let chain = self
                .store()
                .chain(&runtime.session, runtime.generation)?
                .ok_or_else(|| Refuse::NotRequested.error())?;
            let Some(update) = apply_completion(&chain, &request, self.now())? else {
                return Ok(completion_reply(&chain));
            };
            if !self.store().apply_chain(&actor, &update, chain.revision)? {
                continue;
            }
            let done = &update.chain;
            let line = format!(
                "[HuntGuardObserve] completion session={} generation={} lastSequence={} findings={} disposition={}",
                done.session,
                done.generation,
                done.last_sequence,
                done.finding_count,
                done.disposition.as_str()
            );
            self.log().info(&line);
            return Ok(completion_reply(done));
        }
        Err(Refuse::Busy.error())
    }

    /// The launcher's own reward for a hunt: Guard grants none, but only a member learns that.
    pub fn self_reward(&self, account: &AccountId, session: &str) -> Result<Json, GuardError> {
        let session: SessionId = session.parse().map_err(|_| Refuse::NoReceipt.error())?;
        let member = self
            .hunts()
            .hunt(&session)?
            .is_some_and(|hunt| hunt.roster.iter().any(|e| e.account == *account));
        Err(if member { Refuse::NoRewards } else { Refuse::NoReceipt }.error())
    }

    fn verdict_now(&self, session: &SessionId, generation: u64) -> Result<Option<HuntVerdict>, GuardError> {
        let chain = self.store().chain(session, generation)?;
        let completed = chain.is_some_and(|chain| chain.completion.is_some());
        Ok(completed.then_some(HuntVerdict::OBSERVED))
    }

    /// Both keys in this process's windows; a session refusal keeps the host's charge (spec-rd27-rifts-guard risk 6).
    fn charge_local(&self, host_key: &str, session_key: &str, bytes: u64, now: UnixMillis) -> TrafficDecision {
        let mut local = held(&self.local);
        let host_ok = local.consume(host_key, bytes, HUNT_GUARD_HOST_TRAFFIC_LIMIT, now);
        if host_ok && local.consume(session_key, bytes, HUNT_GUARD_SESSION_TRAFFIC_LIMIT, now) {
            TrafficDecision::Accepted
        } else {
            TrafficDecision::RateLimited
        }
    }

    /// Both keys in the store's windows, charged together or not at all.
    fn charge_shared(&self, host_key: &str, session_key: &str, bytes: u64, now: UnixMillis) -> TrafficDecision {
        let windows = (
            reservation(TrafficScope::Host, host_key, bytes, HUNT_GUARD_HOST_TRAFFIC_LIMIT, now),
            reservation(
                TrafficScope::Session,
                session_key,
                bytes,
                HUNT_GUARD_SESSION_TRAFFIC_LIMIT,
                now,
            ),
        );
        let (Some(host), Some(session)) = windows else {
            return TrafficDecision::RateLimited;
        };
        match self.store().reserve_traffic(&GUARD_ACTOR, &host, &session) {
            Err(_) => TrafficDecision::Unavailable,
            Ok(false) => TrafficDecision::RateLimited,
            Ok(true) => TrafficDecision::Accepted,
        }
    }

    pub(crate) fn charge_now(&self, host: &AccountId, session: &SessionId, bytes: u64) -> TrafficDecision {
        let now = self.now();
        let keys = (host_key(&host.0), session_key(&session.to_string()));
        if self.parts.config.distributed_traffic {
            self.charge_shared(&keys.0, &keys.1, bytes, now)
        } else {
            self.charge_local(&keys.0, &keys.1, bytes, now)
        }
    }

    fn cached_bucket(&self, digest: &str, now: UnixMillis) -> Option<Option<String>> {
        let fresh = |bucket: &&Bucket| bucket.digest == digest && now.0.saturating_sub(bucket.at.0) < BUCKET_CACHE_MS;
        held(&self.buckets)
            .iter()
            .find(fresh)
            .map(|bucket| bucket.answer.clone())
    }

    fn remember_bucket(&self, digest: String, now: UnixMillis, answer: Option<String>) {
        let mut buckets = held(&self.buckets);
        match buckets.iter_mut().find(|bucket| bucket.digest == digest) {
            Some(known) => (known.at, known.answer) = (now, answer),
            None => {
                if buckets.len() >= BUCKET_CACHE_ENTRIES {
                    buckets.pop_front();
                }
                buckets.push_back(Bucket {
                    digest,
                    at: now,
                    answer,
                });
            }
        }
    }

    /// `host:<capability id>` for a live capability, cached briefly by the token's digest.
    fn bucket_now(&self, request: &RequestInfo) -> Option<String> {
        let token = request
            .header(HOST_CAPABILITY_HEADER)
            .filter(|t| capability_shaped(t))?;
        let digest = sha256_hex(token.as_bytes());
        let now = self.now();
        if let Some(answer) = self.cached_bucket(&digest, now) {
            return answer;
        }
        let live = self.hunts().capability_by_digest(&digest).ok().flatten();
        let answer = live
            .filter(|capability| !capability.revoked && capability.expires_at > now)
            .map(|capability| format!("host:{}", capability.capability_id));
        self.remember_bucket(digest, now, answer.clone());
        answer
    }

    /// Who is in a hub now; a hunt has no presence list.
    fn hub_presence(&self, hunt: Option<&HuntView>) -> Result<Option<Vec<AccountId>>, SeamError> {
        match hunt {
            Some(hub) if hub.is_hub() => {
                let present = self.hunts().hub_present_accounts(&hub.session, hub.generation);
                present.map(Some).map_err(seam)
            }
            _ => Ok(None),
        }
    }

    fn authorize_write_now(
        &self,
        runtime: &RuntimeAuth,
        kind: HostWriteKind,
        target: &AccountId,
        options: HostWriteOptions,
    ) -> Result<Option<Reply>, SeamError> {
        const STATE: &str = "This host may not write this player's state.";
        let HostWriteOptions { probe, parity } = options;
        let name = kind_name(kind);
        if !self.parts.config.host_writes {
            if !(probe || parity) {
                self.log().warn(&format!(
                    "[HostWrite] refused kind={name} target={target} reason=HOST_WRITES_OFF"
                ));
            }
            return Ok(Some(write_refusal("Host writes are switched off.")));
        }
        let enforce = self.refuses();
        if parity && enforce {
            return Ok(Some(write_refusal(
                "This host may not do a dedicated server's work here.",
            )));
        }
        let hunt = self.hunts().hunt(&runtime.session).map_err(seam)?;
        let presence = self.hub_presence(hunt.as_ref())?;
        let denial = |check_kind: bool| {
            host_write_denial(
                runtime,
                hunt.as_ref(),
                target,
                Some(kind),
                presence.as_deref(),
                check_kind,
            )
        };
        if parity {
            return Ok(denial(false).map(|_| write_refusal(STATE)));
        }
        let hub_tag = hunt.as_ref().map(HuntView::hub_tag).unwrap_or_default();
        let session = &runtime.session;
        let mut reason = denial(true);
        // Observing, a refusal by kind alone is logged and let through.
        let observed = reason.filter(|r| r.is_policy() && !probe && !enforce);
        if let Some(policy) = observed {
            reason = denial(false);
            if reason.is_none() {
                let policy = policy.as_str();
                self.log().warn(&format!(
                    "[HostPolicyObserve] would refuse write kind={name} target={target} session={session} \
                     reason={policy}{hub_tag}"
                ));
            }
        }
        let Some(reason) = reason else {
            return Ok(None);
        };
        if !probe {
            let reason = reason.as_str();
            self.log().warn(&format!(
                "[HostWrite] refused kind={name} target={target} session={session} reason={reason}{hub_tag}"
            ));
        }
        Ok(Some(write_refusal(STATE)))
    }

    fn record_write_now(&self, runtime: &RuntimeAuth, write: &HostWrite) -> Result<(), SeamError> {
        let (name, target) = (kind_name(write.kind), &write.target);
        let hunt = self.hunts().hunt(&runtime.session).map_err(seam)?;
        let Some(hunt) = hunt.filter(|hunt| hunt.generation == runtime.generation) else {
            self.log().warn(&format!(
                "[HostWrite] record without authorize kind={name} target={target}"
            ));
            return Ok(());
        };
        let host = hunt.host.clone().unwrap_or(AccountId(String::new()));
        let (realm_tag, hub_tag) = (hunt.realm_tag(), hunt.hub_tag());
        let summary = js_slice(&write.summary.0, 400);
        self.log().info(&format!(
            "[HostWrite] kind={name} target={target} host={host} session={} generation={}{realm_tag}{hub_tag} \
             summary={summary}",
            hunt.session, hunt.generation
        ));
        let record = HostWriteRecord {
            session_kind: hunt.session_kind().map(String::from),
            hub_kind: hunt.hub_kind().map(String::from),
            kind: write.kind,
            target: target.clone(),
            host,
            session: hunt.session,
            generation: hunt.generation,
            realm: hunt.realm,
            summary: write.summary.clone(),
            at: self.now(),
        };
        let stored = self.store().record_host_write(&Self::runtime_actor(runtime), &record);
        if let Err(error) = stored {
            let session = &runtime.session;
            self.log().error(&format!(
                "[HostWrite] tag not stored kind={name} target={target} session={session}: {error}"
            ));
        }
        Ok(())
    }

    /// The draft token: whether this is an accepted replay, and the decision read before the transaction.
    async fn draft_premium(
        &self,
        transaction: &InventoryTransaction,
        body: Option<&Value>,
    ) -> (bool, Option<DraftPremiumDecision>) {
        let core = &self.parts.core;
        if core
            .accepted_inventory_replay(transaction.clone())
            .await
            .unwrap_or(false)
        {
            return (true, None);
        }
        let stored = core.stacked_items(transaction.character_id.clone()).await;
        let balance = stored
            .ok()
            .and_then(|items| stacked_balance_of(items.as_ref().map(|j| j.0.as_str()), grants::HOST_DRAFT_PREMIUM_ID));
        (false, body.and_then(|body| decide_draft_premium(body, balance)))
    }

    /// The item classes a host may not grant; observing, they are logged and let through.
    fn grant_check(
        &self,
        hunt: Option<&HuntView>,
        body: &Value,
        allowances: Allowances,
        early: Option<DraftPremiumDecision>,
        tags: impl FnOnce(&str) -> String,
    ) -> Option<Reply> {
        let session = GrantSession {
            no_completion: hunt.is_some_and(|h| h.completion_mode == Some(CompletionMode::NoCompletion)),
            realm_hub: hunt.is_some_and(HuntView::is_hub),
        };
        let catalog = self.parts.core.host_grant_catalog();
        let catalog: &dyn rustic_rifts_seam::HostGrantCatalog = catalog.as_deref().unwrap_or(&NoGameData);
        let denial = grant_denial(session, body, catalog, allowances)?;
        let draft = match early {
            Some(decision) if !decision.allowed => format!(" draftPremium={}", decision.reason),
            _ => String::new(),
        };
        let tags = tags(&format!(
            "classes={} denied={}{draft}",
            denial.classes.join(","),
            denial.summary
        ));
        if !self.refuses() {
            self.log()
                .warn(&format!("[HostPolicyObserve] would refuse grant {tags}"));
            return None;
        }
        self.log().warn(&format!("[HostGrantDeny] {tags}"));
        let classes: Vec<String> = denial.classes.iter().map(|class| json::js_string(class)).collect();
        let body = format!(
            r#"{{"code":"P2P_HOST_GRANT_DENIED","message":"A player host may not grant this item class.","payload":{{"classes":[{}]}}}}"#,
            classes.join(",")
        );
        Some(Reply {
            status: 403,
            body: Json(body),
        })
    }

    async fn authorize_inventory_now(
        &self,
        runtime: RuntimeAuth,
        transaction: InventoryTransaction,
    ) -> Result<InventoryDecision, SeamError> {
        let hunt = self.hunts().hunt(&runtime.session).map_err(seam)?;
        let presence = self.hub_presence(hunt.as_ref())?;
        let body = transaction.body.as_ref().and_then(parse_json);
        let user = &transaction.user;
        let (session, generation) = (&runtime.session, runtime.generation);
        let refused_body = inventory_body_denial(
            &runtime,
            hunt.as_ref(),
            user,
            &transaction.character_id,
            body.as_ref(),
            presence.as_deref(),
        );
        if let Some(reason) = refused_body {
            self.log().warn(&format!(
                "[P2PInventory] rejected code={reason} account={user} session={session} generation={generation}"
            ));
            return Ok(InventoryDecision::Refused(loot_refusal("P2P_LOOT_GRANT_DENIED")));
        }

        let hub_tag = hunt.as_ref().map(HuntView::hub_tag).unwrap_or_default();
        let adds_draft = body.as_ref().is_some_and(body_adds_host_draft_premium);
        let (replay, early) = if adds_draft {
            self.draft_premium(&transaction, body.as_ref()).await
        } else {
            (false, None)
        };
        if let Some(body) = &body {
            let allowances = Allowances {
                draft_premium: replay || early.is_some_and(|d| d.allowed),
            };
            let transaction_id = &transaction.transaction_id;
            let tags = |denied: &str| {
                format!(
                    "account={user} session={session} generation={generation} transaction={transaction_id} \
                     {denied}{hub_tag}"
                )
            };
            if let Some(reply) = self.grant_check(hunt.as_ref(), body, allowances, early, tags) {
                return Ok(InventoryDecision::Refused(reply));
            }
        }

        let realm_tag = hunt.as_ref().map(HuntView::realm_tag).unwrap_or_default();
        let tags = format!(
            "account={user} character={} session={session} generation={generation} transaction={}{realm_tag}{hub_tag}",
            transaction.character_id, transaction.transaction_id
        );
        let fence = GuardFence {
            hunts: Arc::clone(&self.parts.hunts),
            log: Arc::clone(&self.parts.log),
            clock: Arc::clone(&self.parts.clock),
            enforce: self.refuses(),
            carries_draft: adds_draft && !replay,
            early,
            body,
            tags,
            runtime,
            transaction,
            allowed: Mutex::new(None),
        };
        Ok(InventoryDecision::Authorized(Box::new(fence)))
    }

    fn operations(&self) -> Operations<'_> {
        let GuardParts {
            operations,
            core,
            store,
            clock,
            ..
        } = &self.parts;
        Operations {
            policy: &**operations,
            core: &**core,
            store: &**store,
            clock: &**clock,
        }
    }
}

fn parse_json(body: &Json) -> Option<Value> {
    serde_json::from_str(&body.0).ok()
}

fn ready<'a, T: Send + 'a, E: Send + 'a>(answer: Result<T, E>) -> BoxFuture<'a, Result<T, E>> {
    Box::pin(std::future::ready(answer))
}

/// Runs `work` when first polled, as an `async` block would.
fn deferred<'a, T: Send + 'a>(work: impl FnOnce() -> T + Send + 'a) -> BoxFuture<'a, T> {
    Box::pin(async move { work() })
}

impl Lifecycle for Guard {
    fn runtime_safety_issues(&self, _: &dyn Settings) -> Result<Vec<String>, SeamError> {
        Ok(vec![])
    }

    fn startup_summary(&self, _: &str) -> Result<Option<Json>, SeamError> {
        let config = &self.parts.config;
        let (policy, evidence, tier) = (
            config.host_policy.mode.as_str(),
            config.hunt_evidence.enabled,
            config.trust_tier.number(),
        );
        let summary = format!(
            r#"{{"antiCheatLoaded":false,"hostPolicy":"{policy}","huntEvidence":{evidence},"trustTier":{tier}}}"#
        );
        Ok(Some(Json(summary)))
    }

    fn start(&self, _: &str) -> Result<(), SeamError> {
        let config = &self.parts.config;
        log_host_policy(&config.host_policy, self.log());
        log_hunt_evidence(&config.hunt_evidence, self.log());
        Ok(())
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        self.flush_census();
        ready(Ok(()))
    }

    fn tables(&self) -> &'static [TableSpec] {
        TABLES
    }

    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        const ACCOUNT_COLUMNS: [(&str, &str); 3] = [
            ("rifts_host_writes", "target"),
            ("rifts_host_writes", "host"),
            ("rifts_hunt_xp_summaries", "member"),
        ];
        let delete = |(table, account_column): (&'static str, &'static str)| DeletionStep {
            table,
            account_column,
            action: DeletionAction::Delete,
        };
        ACCOUNT_COLUMNS.into_iter().map(delete).collect()
    }
}

impl GuardHooks for Guard {
    fn service_roles(&self) -> &'static [&'static str] {
        &[]
    }

    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        GUARD_MANIFEST_ROLES
    }

    fn census(&self, sample: &HostRouteSample) -> Result<(), SeamError> {
        self.record_census(sample);
        Ok(())
    }

    fn rate_limit_bucket(&self, request: RequestInfo) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        deferred(move || Ok(self.bucket_now(&request)))
    }

    fn authorize_inventory(
        &self,
        runtime: RuntimeAuth,
        transaction: InventoryTransaction,
    ) -> BoxFuture<'_, Result<InventoryDecision, SeamError>> {
        Box::pin(self.authorize_inventory_now(runtime, transaction))
    }

    fn authorize_host_write(
        &self,
        runtime: RuntimeAuth,
        kind: HostWriteKind,
        target: AccountId,
        options: HostWriteOptions,
    ) -> BoxFuture<'_, Result<Option<Reply>, SeamError>> {
        deferred(move || self.authorize_write_now(&runtime, kind, &target, options))
    }

    fn record_host_write(&self, runtime: RuntimeAuth, write: HostWrite) -> BoxFuture<'_, Result<(), SeamError>> {
        deferred(move || self.record_write_now(&runtime, &write))
    }

    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        Ok(self.refuses())
    }

    fn note_would_refuse(&self, what: &str, detail: &str) -> Result<(), SeamError> {
        self.log()
            .warn(&format!("[HostPolicyObserve] would refuse {what} {detail}"));
        Ok(())
    }
}

impl EvidenceIntake for Guard {
    fn submit_evidence(&self, runtime: RuntimeAuth, body: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        deferred(move || self.submit_parsed(&runtime, parse_json(&body).as_ref()))
    }

    fn complete(&self, runtime: RuntimeAuth, body: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        deferred(move || self.complete_parsed(&runtime, parse_json(&body).as_ref()))
    }

    fn verdict(&self, session: SessionId, generation: u64) -> BoxFuture<'_, Result<Option<HuntVerdict>, GuardError>> {
        deferred(move || self.verdict_now(&session, generation))
    }

    fn reward_receipt(&self, _: SessionId, _: u64, _: AccountId) -> BoxFuture<'_, Result<Option<Json>, GuardError>> {
        ready(Ok(None))
    }

    /// The shadow key is never read or logged: nothing here compares.
    fn record_shadow_comparison(&self, _: Option<Secret>, _: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        ready(Err(Refuse::NotCollected.error()))
    }
}

impl TrafficLimits for Guard {
    fn charge(
        &self,
        host: AccountId,
        session: SessionId,
        bytes: u64,
    ) -> BoxFuture<'_, Result<TrafficDecision, GuardError>> {
        deferred(move || Ok(self.charge_now(&host, &session, bytes)))
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
            self.operations()
                .set_reward_rollout(&admin_key, body.as_ref(), &ip)
                .await
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
            let operations = self.operations();
            operations
                .acknowledge_alert(&admin_key, &alert_id, body.as_ref(), &ip)
                .await
        })
    }
}

/// Guard with nothing behind it: every operation is `Unimplemented` under its method name.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stub;

// Each listed method answers `Unimplemented` under its own name, at once or as a ready future, with the error type
// its trait uses.
macro_rules! unwritten {
    (now $error:ty: $($name:ident($($arg:ty),*) -> $out:ty;)*) => {$(
        fn $name(&self, $(_: $arg),*) -> Result<$out, $error> {
            Err(Unimplemented::new(Component::Guard, stringify!($name)).into())
        }
    )*};
    (later $error:ty: $($name:ident($($arg:ty),*) -> $out:ty;)*) => {$(
        fn $name(&self, $(_: $arg),*) -> BoxFuture<'_, Result<$out, $error>> {
            rustic_rifts_seam::stub(Component::Guard, stringify!($name))
        }
    )*};
}

impl Lifecycle for Stub {
    unwritten! { now SeamError:
        runtime_safety_issues(&dyn Settings) -> Vec<String>;
        startup_summary(&str) -> Option<Json>;
        start(&str) -> ();
    }

    unwritten! { later SeamError:
        stop() -> ();
    }

    fn tables(&self) -> &'static [TableSpec] {
        TABLES
    }

    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        vec![]
    }
}

impl GuardHooks for Stub {
    unwritten! { now SeamError:
        census(&HostRouteSample) -> ();
        host_policy_refuses() -> bool;
        note_would_refuse(&str, &str) -> ();
    }

    unwritten! { later SeamError:
        rate_limit_bucket(RequestInfo) -> Option<String>;
        authorize_inventory(RuntimeAuth, InventoryTransaction) -> InventoryDecision;
        authorize_host_write(RuntimeAuth, HostWriteKind, AccountId, HostWriteOptions) -> Option<Reply>;
        record_host_write(RuntimeAuth, HostWrite) -> ();
    }

    fn service_roles(&self) -> &'static [&'static str] {
        &[]
    }

    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        GUARD_MANIFEST_ROLES
    }
}

impl EvidenceIntake for Stub {
    unwritten! { later GuardError:
        submit_evidence(RuntimeAuth, Json) -> Json;
        complete(RuntimeAuth, Json) -> Json;
        verdict(SessionId, u64) -> Option<HuntVerdict>;
        reward_receipt(SessionId, u64, AccountId) -> Option<Json>;
        record_shadow_comparison(Option<Secret>, Json) -> Json;
    }
}

impl TrafficLimits for Stub {
    unwritten! { later GuardError:
        charge(AccountId, SessionId, u64) -> TrafficDecision;
    }
}

impl GuardOperations for Stub {
    unwritten! { later GuardError:
        set_reward_rollout(String, Option<Value>, String) -> Reply;
        acknowledge_alert(String, String, Option<Value>, String) -> Reply;
    }
}
