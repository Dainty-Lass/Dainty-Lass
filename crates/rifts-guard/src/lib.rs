// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Rifts' Guard in observe mode: the Hunt Guard evidence chain a host's transport posts, whom a hosted
//! runtime may write player state for, the item classes and the daily draft token a host may grant, the bounds on
//! Guard traffic, the host route census and two admin operations routes. Nothing here grants or withholds a
//! reward: the chain is observed, and every would-be refusal of the host policy is logged unless it enforces.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

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

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use rifts_seam::{
    AccountId, Actor, BoxFuture, Component, DeletionAction, DeletionStep, GuardHooks, HOST_CAPABILITY_HEADER,
    HostWrite, HostWriteKind, HostWriteOptions, InventoryDecision, InventoryTransaction, IslesCore, Json, Lifecycle,
    Reply, RequestInfo, RuntimeAuth, SeamError, Secret, SessionId, Settings, StoreError, TableSpec, Unimplemented,
    UnixMillis,
};
use serde_json::Value;

pub use crate::census::{HostRouteCensus, HostRouteRow, HostRouteSummary};
pub use crate::config::{
    GuardConfig, HostPolicyMode, HostPolicySetting, HuntEvidenceSetting, Tier1Policy, TrustTier, log_host_policy,
    log_hunt_evidence, resolve_host_policy, resolve_hunt_evidence,
};
pub use crate::digest::sha256_hex;
pub use crate::errors::{ERROR_STATUSES, GuardError, envelope, status_of};
pub use crate::operations::{DEFAULT_REWARD_ROLLOUT, RewardRollout};
pub use crate::ports::{
    AuditEntry, CapabilityView, Clock, CoreOperationsPolicy, GuardLog, HuntSessions, HuntView, ManualClock, MemoryLog,
    MemoryOperations, NoHuntSessions, OperationsPolicyPort, PolicySnapshot, RosterEntry, StderrLog, SystemClock,
    iso_time,
};
pub use crate::routes::{GuardRoutes, ROUTES, stub_routes};
pub use crate::sql::{POSTGRES_MIGRATIONS, SCHEMA, SQLITE_MIGRATIONS, SqlGuardStore, block_on_store, store_actor};
pub use crate::store::{
    AlertRecord, BatchRecord, ChainRecord, ChainUpdate, CompletionMode, CompletionRecord, Disposition, EncounterState,
    FindingRecord, GENESIS_HASH, GuardStore, HostWriteRecord, MemoryGuardStore, StubGuardStore, TABLES,
    TrafficReservation, TrafficScope, TrafficWindowRecord, XP_SUMMARY_TTL_MS, XpSummaryRecord,
};
pub use crate::traffic::{
    BudgetPermit, HUNT_GUARD_HOST_TRAFFIC_LIMIT, HUNT_GUARD_REQUEST_BUDGET, HUNT_GUARD_SESSION_TRAFFIC_LIMIT,
    HuntOperation, LocalTrafficLimiter, RequestBudget, RequestBudgetLimit, TrafficLimit,
};
pub use crate::writes::{
    WriteDenial, host_write_denial, inventory_body_denial, inventory_denial, kind_name, loot_refusal, write_refusal,
};
pub use rifts_seam::HostRouteSample;

use crate::evidence::{
    BatchProblem, CONFLICT, NOT_REQUESTED, apply_batch, apply_completion, batch_reply, completion_reply,
    finding_update, parse_batch, parse_completion,
};
use crate::fence::GuardFence;
use crate::grants::{
    Allowances, DraftPremiumDecision, GrantSession, NoGameData, body_adds_host_draft_premium, decide_draft_premium,
    grant_denial, stacked_balance_of,
};
use crate::json::js_slice;
use crate::operations::Operations;
use crate::traffic::{host_key, reservation, session_key};

/// The runtime files a Guard manifest names.
pub const GUARD_MANIFEST_ROLES: &[&str] = &["TRANSPORT_MODULE", "TRANSPORT_RUNTIME", "ADMISSION_KEY"];

const NOT_COLLECTED: &str = "This server does not collect hunt evidence.";
const FENCED: &str = "The hunt generation no longer belongs to this host.";
const REPLAY: &str = "This Guard batch id was already used for other evidence.";
const BUSY: &str = "The Guard completion is busy; retry shortly.";
const ATTEMPTS: usize = 3;
const BUCKET_CACHE_ENTRIES: usize = 1024;
const BUCKET_CACHE_MS: i64 = 10_000;

/// What Guard says a completed hunt earned: in observe mode, nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HuntVerdict {
    pub progression_applied: bool,
    pub rewards_applied: bool,
}

impl HuntVerdict {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewardPolicyStatus {
    Ready { version: String, digest: String },
    NotReady { reason: &'static str },
}

pub trait EvidenceIntake: Send + Sync {
    fn submit_evidence(&self, runtime: RuntimeAuth, body: Json) -> BoxFuture<'_, Result<Json, GuardError>>;
    fn complete(&self, runtime: RuntimeAuth, body: Json) -> BoxFuture<'_, Result<Json, GuardError>>;
    fn verdict(&self, session: SessionId, generation: u64) -> BoxFuture<'_, Result<Option<HuntVerdict>, GuardError>>;
    fn reward_receipt(
        &self,
        session: SessionId,
        generation: u64,
        account: AccountId,
    ) -> BoxFuture<'_, Result<Option<Json>, GuardError>>;
    fn record_shadow_comparison(&self, key: Option<Secret>, body: Json) -> BoxFuture<'_, Result<Json, GuardError>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficDecision {
    Accepted,
    RateLimited,
    Unavailable,
}

pub trait TrafficLimits: Send + Sync {
    fn charge(
        &self,
        host: AccountId,
        session: SessionId,
        bytes: u64,
    ) -> BoxFuture<'_, Result<TrafficDecision, GuardError>>;
}

pub trait GuardOperations: Send + Sync {
    fn set_reward_rollout(
        &self,
        admin_key: String,
        body: Option<Value>,
        ip: String,
    ) -> BoxFuture<'_, Result<Reply, GuardError>>;
    fn acknowledge_alert(
        &self,
        admin_key: String,
        alert_id: String,
        body: Option<Value>,
        ip: String,
    ) -> BoxFuture<'_, Result<Reply, GuardError>>;
}

pub struct GuardParts {
    pub config: GuardConfig,
    pub store: Arc<dyn GuardStore>,
    pub hunts: Arc<dyn HuntSessions>,
    pub operations: Arc<dyn OperationsPolicyPort>,
    pub core: Arc<dyn IslesCore>,
    pub clock: Arc<dyn Clock>,
    pub log: Arc<dyn GuardLog>,
}

/// One cached rate-limit bucket answer, by the capability's digest.
struct Bucket {
    digest: String,
    at: UnixMillis,
    answer: Option<String>,
}

pub struct Guard {
    config: GuardConfig,
    store: Arc<dyn GuardStore>,
    hunts: Arc<dyn HuntSessions>,
    operations: Arc<dyn OperationsPolicyPort>,
    core: Arc<dyn IslesCore>,
    clock: Arc<dyn Clock>,
    log: Arc<dyn GuardLog>,
    census: HostRouteCensus,
    budget: RequestBudget,
    local: Mutex<LocalTrafficLimiter>,
    buckets: Mutex<VecDeque<Bucket>>,
}

fn refused(code: &str, message: &str) -> GuardError {
    GuardError::refused(code, message)
}

/// 43 characters of base64url: the hosted runtime's capability token.
pub(crate) fn capability_shaped(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn seam(error: StoreError) -> SeamError {
    SeamError::from(GuardError::from(error))
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
            census,
            budget: RequestBudget::new(HUNT_GUARD_REQUEST_BUDGET),
            local: Mutex::new(LocalTrafficLimiter::default()),
            buckets: Mutex::new(VecDeque::new()),
        }
    }

    pub fn config(&self) -> &GuardConfig {
        &self.config
    }

    pub fn request_budget(&self) -> &RequestBudget {
        &self.budget
    }

    pub(crate) fn log(&self) -> &dyn GuardLog {
        self.log.as_ref()
    }

    pub(crate) fn hunts(&self) -> &dyn HuntSessions {
        self.hunts.as_ref()
    }

    pub(crate) fn now(&self) -> UnixMillis {
        self.clock.now()
    }

    pub fn record_census(&self, sample: &HostRouteSample) {
        self.census.record(sample);
    }

    pub fn flush_census(&self) {
        if let Some(line) = self.census.flush() {
            self.log.info(&line);
        }
    }

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
        if self.store.chain(&session, generation)?.is_some() {
            return Ok(());
        }
        let mut chain = ChainRecord::new(session, generation, completion_mode.and_then(CompletionMode::parse));
        chain.revision = 1;
        let update = ChainUpdate {
            chain,
            batch: None,
            findings: Vec::new(),
        };
        self.store.apply_chain(&Actor::System("guard"), &update, 0)?;
        Ok(())
    }

    pub fn chain_status(&self, session: &SessionId, generation: u64) -> Result<Option<ChainRecord>, GuardError> {
        Ok(self.store.chain(session, generation)?)
    }

    /// The hunt the runtime may post evidence for.
    fn evidence_gate(&self, runtime: &RuntimeAuth) -> Result<HuntView, GuardError> {
        if !self.config.hunt_evidence.enabled {
            return Err(refused("HUNT_EVIDENCE_NOT_COLLECTED", NOT_COLLECTED));
        }
        let hunt = self
            .hunts
            .hunt(&runtime.session)?
            .filter(|h| h.generation == runtime.generation && h.host.as_ref() == Some(&runtime.host_account))
            .ok_or_else(|| refused("P2P_GENERATION_FENCED", FENCED))?;
        if hunt.is_hub() {
            return Err(refused("HUNT_EVIDENCE_NOT_COLLECTED", NOT_COLLECTED));
        }
        Ok(hunt)
    }

    fn runtime_actor(runtime: &RuntimeAuth) -> Actor {
        Actor::HostRuntime {
            session: runtime.session.clone(),
            host: runtime.host_account.clone(),
        }
    }

    fn log_finding(&self, finding: &FindingRecord) {
        let sequence = finding.sequence.map_or_else(|| "-".to_owned(), |s| s.to_string());
        let detail = if finding.detail.is_empty() {
            String::new()
        } else {
            format!(" {}", finding.detail)
        };
        self.log.warn(&format!(
            "[HuntGuardObserve] would refuse {} session={} generation={} sequence={sequence}{detail}",
            finding.code, finding.session, finding.generation
        ));
    }

    pub fn submit_parsed(&self, runtime: &RuntimeAuth, body: Option<&Value>) -> Result<Json, GuardError> {
        let hunt = self.evidence_gate(runtime)?;
        let batch = parse_batch(body).map_err(BatchProblem::into_error)?;
        let roster: Vec<AccountId> = hunt.roster.iter().map(|entry| entry.account.clone()).collect();
        let actor = Self::runtime_actor(runtime);
        let (session, generation) = (&runtime.session, runtime.generation);
        for _ in 0..ATTEMPTS {
            let chain = self
                .store
                .chain(session, generation)?
                .unwrap_or_else(|| ChainRecord::new(session.clone(), generation, None));
            if let Some(stored) = self.store.batch(session, generation, &batch.batch_id)? {
                if stored.batch_hash != batch.batch_hash {
                    return Err(refused("GUARD_BATCH_REPLAY", REPLAY));
                }
                return Ok(batch_reply(
                    session,
                    generation,
                    &stored.batch_hash,
                    stored.last_sequence,
                    &chain,
                    true,
                ));
            }
            let now = self.clock.now();
            match apply_batch(&chain, &batch, &roster, now) {
                Ok(update) => {
                    if self.store.apply_chain(&actor, &update, chain.revision)? {
                        for finding in &update.findings {
                            self.log_finding(finding);
                        }
                        let next = &update.chain;
                        return Ok(batch_reply(
                            session,
                            generation,
                            &next.head_hash,
                            next.last_sequence,
                            next,
                            false,
                        ));
                    }
                }
                Err(refusal) => {
                    if let Some((code, detail)) = &refusal.finding {
                        let update = finding_update(&chain, code, detail, now);
                        if self.store.apply_chain(&actor, &update, chain.revision)? {
                            for finding in &update.findings {
                                self.log_finding(finding);
                            }
                        }
                    }
                    return Err(refusal.error);
                }
            }
        }
        Err(refused("GUARD_CHAIN_CONFLICT", CONFLICT))
    }

    pub fn complete_parsed(&self, runtime: &RuntimeAuth, body: Option<&Value>) -> Result<Json, GuardError> {
        self.evidence_gate(runtime)?;
        let request = parse_completion(body)?;
        let actor = Self::runtime_actor(runtime);
        for _ in 0..ATTEMPTS {
            let Some(chain) = self.store.chain(&runtime.session, runtime.generation)? else {
                return Err(refused("GUARD_COMPLETION_INVALID", NOT_REQUESTED));
            };
            let Some(update) = apply_completion(&chain, &request, self.clock.now())? else {
                return Ok(completion_reply(&chain));
            };
            if self.store.apply_chain(&actor, &update, chain.revision)? {
                let done = &update.chain;
                self.log.info(&format!(
                    "[HuntGuardObserve] completion session={} generation={} lastSequence={} findings={} disposition={}",
                    done.session,
                    done.generation,
                    done.last_sequence,
                    done.finding_count,
                    done.disposition.as_str()
                ));
                return Ok(completion_reply(done));
            }
        }
        Err(refused("GUARD_FINALIZATION_BUSY", BUSY))
    }

    /// The launcher's own reward for a hunt: Guard grants none, but only a member learns that.
    pub fn self_reward(&self, account: &AccountId, session: &str) -> Result<Json, GuardError> {
        let not_found = || refused("P2P_SESSION_NOT_FOUND", "Hunt reward receipt not found.");
        let Ok(session) = session.parse::<SessionId>() else {
            return Err(not_found());
        };
        let hunt = self.hunts.hunt(&session)?.ok_or_else(not_found)?;
        if !hunt.roster.iter().any(|entry| &entry.account == account) {
            return Err(not_found());
        }
        Err(refused(
            "HUNT_REWARDS_UNAVAILABLE",
            "This server grants no player-hosted hunt rewards.",
        ))
    }

    fn verdict_now(&self, session: &SessionId, generation: u64) -> Result<Option<HuntVerdict>, GuardError> {
        Ok(self
            .store
            .chain(session, generation)?
            .filter(|chain| chain.completion.is_some())
            .map(|_| HuntVerdict::OBSERVED))
    }

    pub(crate) fn charge_now(&self, host: &AccountId, session: &SessionId, bytes: u64) -> TrafficDecision {
        let now = self.clock.now();
        let (host_key, session_key) = (host_key(&host.0), session_key(&session.to_string()));
        if !self.config.distributed_traffic {
            let mut local = self.local.lock().unwrap_or_else(PoisonError::into_inner);
            // a session refusal keeps the host's charge (spec-rd27-rifts-guard risk 6)
            let accepted = local.consume(&host_key, bytes, HUNT_GUARD_HOST_TRAFFIC_LIMIT, now)
                && local.consume(&session_key, bytes, HUNT_GUARD_SESSION_TRAFFIC_LIMIT, now);
            return if accepted {
                TrafficDecision::Accepted
            } else {
                TrafficDecision::RateLimited
            };
        }
        let host_window = reservation(TrafficScope::Host, &host_key, bytes, HUNT_GUARD_HOST_TRAFFIC_LIMIT, now);
        let session_window = reservation(
            TrafficScope::Session,
            &session_key,
            bytes,
            HUNT_GUARD_SESSION_TRAFFIC_LIMIT,
            now,
        );
        let (Some(host_window), Some(session_window)) = (host_window, session_window) else {
            return TrafficDecision::RateLimited;
        };
        match self
            .store
            .reserve_traffic(&Actor::System("guard"), &host_window, &session_window)
        {
            Ok(true) => TrafficDecision::Accepted,
            Ok(false) => TrafficDecision::RateLimited,
            Err(_) => TrafficDecision::Unavailable,
        }
    }

    fn bucket_now(&self, request: &RequestInfo) -> Option<String> {
        let value = request
            .header(HOST_CAPABILITY_HEADER)
            .filter(|v| capability_shaped(v))?;
        let digest = sha256_hex(value.as_bytes());
        let now = self.clock.now();
        {
            let buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(hit) = buckets
                .iter()
                .find(|b| b.digest == digest && now.0.saturating_sub(b.at.0) < BUCKET_CACHE_MS)
            {
                return hit.answer.clone();
            }
        }
        let answer = match self.hunts.capability_by_digest(&digest) {
            Ok(Some(capability)) if !capability.revoked && capability.expires_at > now => {
                Some(format!("host:{}", capability.capability_id))
            }
            _ => None,
        };
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = buckets.iter_mut().find(|b| b.digest == digest) {
            existing.at = now;
            existing.answer = answer.clone();
        } else {
            if buckets.len() >= BUCKET_CACHE_ENTRIES {
                buckets.pop_front();
            }
            buckets.push_back(Bucket {
                digest,
                at: now,
                answer: answer.clone(),
            });
        }
        answer
    }

    fn hub_presence(&self, hunt: &HuntView) -> Result<Option<Vec<AccountId>>, StoreError> {
        if hunt.is_hub() {
            self.hunts
                .hub_present_accounts(&hunt.session, hunt.generation)
                .map(Some)
        } else {
            Ok(None)
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
        let name = kind_name(kind);
        if !self.config.host_writes {
            if !options.probe && !options.parity {
                self.log.warn(&format!(
                    "[HostWrite] refused kind={name} target={target} reason=HOST_WRITES_OFF"
                ));
            }
            return Ok(Some(write_refusal("Host writes are switched off.")));
        }
        let enforce = self.config.host_policy_refuses();
        if options.parity && enforce {
            return Ok(Some(write_refusal(
                "This host may not do a dedicated server's work here.",
            )));
        }
        let hunt = self.hunts.hunt(&runtime.session).map_err(seam)?;
        let presence = match &hunt {
            Some(h) => self.hub_presence(h).map_err(seam)?,
            None => None,
        };
        let denial = |check_kind| {
            host_write_denial(
                runtime,
                hunt.as_ref(),
                target,
                Some(kind),
                presence.as_deref(),
                check_kind,
            )
        };
        if options.parity {
            return Ok(denial(false).map(|_| write_refusal(STATE)));
        }
        let hub_tag = hunt.as_ref().map(HuntView::hub_tag).unwrap_or_default();
        let mut reason = denial(true);
        if let Some(policy) = reason.filter(|r| r.is_policy())
            && !options.probe
            && !enforce
        {
            reason = denial(false);
            if reason.is_none() {
                self.log.warn(&format!(
                    "[HostPolicyObserve] would refuse write kind={name} target={target} session={} reason={}{hub_tag}",
                    runtime.session,
                    policy.as_str()
                ));
            }
        }
        let Some(reason) = reason else {
            return Ok(None);
        };
        if !options.probe {
            self.log.warn(&format!(
                "[HostWrite] refused kind={name} target={target} session={} reason={}{hub_tag}",
                runtime.session,
                reason.as_str()
            ));
        }
        Ok(Some(write_refusal(STATE)))
    }

    fn record_write_now(&self, runtime: &RuntimeAuth, write: &HostWrite) -> Result<(), SeamError> {
        let name = kind_name(write.kind);
        let target = &write.target;
        let Some(hunt) = self
            .hunts
            .hunt(&runtime.session)
            .map_err(seam)?
            .filter(|h| h.generation == runtime.generation)
        else {
            self.log.warn(&format!(
                "[HostWrite] record without authorize kind={name} target={target}"
            ));
            return Ok(());
        };
        let host = hunt.host.clone().unwrap_or_else(|| AccountId(String::new()));
        let summary = js_slice(&write.summary.0, 400);
        self.log.info(&format!(
            "[HostWrite] kind={name} target={target} host={host} session={} generation={}{}{} summary={summary}",
            hunt.session,
            hunt.generation,
            hunt.realm_tag(),
            hunt.hub_tag()
        ));
        let record = HostWriteRecord {
            kind: write.kind,
            target: target.clone(),
            host,
            session: hunt.session.clone(),
            generation: hunt.generation,
            realm: hunt.realm.clone(),
            session_kind: hunt.session_kind().map(str::to_owned),
            hub_kind: hunt.hub_kind().map(str::to_owned),
            summary: write.summary.clone(),
            at: self.clock.now(),
        };
        if let Err(error) = self.store.record_host_write(&Self::runtime_actor(runtime), &record) {
            self.log.error(&format!(
                "[HostWrite] tag not stored kind={name} target={target} session={}: {error}",
                runtime.session
            ));
        }
        Ok(())
    }

    async fn authorize_inventory_now(
        &self,
        runtime: RuntimeAuth,
        transaction: InventoryTransaction,
    ) -> Result<InventoryDecision, SeamError> {
        let hunt = self.hunts.hunt(&runtime.session).map_err(seam)?;
        let presence = match &hunt {
            Some(h) => self.hub_presence(h).map_err(seam)?,
            None => None,
        };
        let body: Option<Value> = transaction
            .body
            .as_ref()
            .and_then(|json| serde_json::from_str(&json.0).ok());
        let user = &transaction.user;
        if let Some(reason) = inventory_body_denial(
            &runtime,
            hunt.as_ref(),
            user,
            &transaction.character_id,
            body.as_ref(),
            presence.as_deref(),
        ) {
            self.log.warn(&format!(
                "[P2PInventory] rejected code={reason} account={user} session={} generation={}",
                runtime.session, runtime.generation
            ));
            return Ok(InventoryDecision::Refused(loot_refusal("P2P_LOOT_GRANT_DENIED")));
        }
        let hub_tag = hunt.as_ref().map(HuntView::hub_tag).unwrap_or_default();
        let adds_draft = body.as_ref().is_some_and(body_adds_host_draft_premium);
        let mut replay = false;
        let mut early: Option<DraftPremiumDecision> = None;
        if adds_draft {
            replay = self
                .core
                .accepted_inventory_replay(transaction.clone())
                .await
                .unwrap_or(false);
            if !replay {
                let balance = match self.core.stacked_items(transaction.character_id.clone()).await {
                    Ok(stored) => {
                        stacked_balance_of(stored.as_ref().map(|j| j.0.as_str()), grants::HOST_DRAFT_PREMIUM_ID)
                    }
                    Err(_) => None,
                };
                early = body.as_ref().and_then(|b| decide_draft_premium(b, balance));
            }
        }
        if let Some(body) = &body {
            let session = GrantSession {
                no_completion: hunt
                    .as_ref()
                    .is_some_and(|h| h.completion_mode == Some(CompletionMode::NoCompletion)),
                realm_hub: hunt.as_ref().is_some_and(HuntView::is_hub),
            };
            let catalog = self.core.host_grant_catalog();
            let catalog: &dyn rifts_seam::HostGrantCatalog = match &catalog {
                Some(catalog) => catalog.as_ref(),
                None => &NoGameData,
            };
            let allowances = Allowances {
                draft_premium: replay || early.is_some_and(|d| d.allowed),
            };
            if let Some(denial) = grant_denial(session, body, catalog, allowances) {
                let draft = early
                    .filter(|d| !d.allowed)
                    .map(|d| format!(" draftPremium={}", d.reason))
                    .unwrap_or_default();
                let tags = format!(
                    "account={user} session={} generation={} transaction={} classes={} denied={}{draft}{hub_tag}",
                    runtime.session,
                    runtime.generation,
                    transaction.transaction_id,
                    denial.classes.join(","),
                    denial.summary
                );
                if !self.config.host_policy_refuses() {
                    self.log.warn(&format!("[HostPolicyObserve] would refuse grant {tags}"));
                } else {
                    self.log.warn(&format!("[HostGrantDeny] {tags}"));
                    let classes: Vec<String> = denial.classes.iter().map(|c| json::js_string(c)).collect();
                    return Ok(InventoryDecision::Refused(Reply {
                        status: 403,
                        body: Json(format!(
                            r#"{{"code":"P2P_HOST_GRANT_DENIED","message":"A player host may not grant this item class.","payload":{{"classes":[{}]}}}}"#,
                            classes.join(",")
                        )),
                    }));
                }
            }
        }
        let realm_tag = hunt.as_ref().map(HuntView::realm_tag).unwrap_or_default();
        let tags = format!(
            "account={user} character={} session={} generation={} transaction={}{realm_tag}{hub_tag}",
            transaction.character_id, runtime.session, runtime.generation, transaction.transaction_id
        );
        let fence = GuardFence {
            hunts: self.hunts.clone(),
            log: self.log.clone(),
            clock: self.clock.clone(),
            enforce: self.config.host_policy_refuses(),
            runtime,
            carries_draft: adds_draft && !replay,
            early,
            body,
            transaction,
            tags,
            allowed: Mutex::new(None),
        };
        Ok(InventoryDecision::Authorized(Box::new(fence)))
    }

    fn operations(&self) -> Operations<'_> {
        Operations {
            policy: self.operations.as_ref(),
            core: self.core.as_ref(),
            store: self.store.as_ref(),
            clock: self.clock.as_ref(),
        }
    }
}

fn parse_json(body: &Json) -> Option<Value> {
    serde_json::from_str(&body.0).ok()
}

impl Lifecycle for Guard {
    fn runtime_safety_issues(&self, _settings: &dyn Settings) -> Result<Vec<String>, SeamError> {
        Ok(Vec::new())
    }

    fn startup_summary(&self, _role: &str) -> Result<Option<Json>, SeamError> {
        Ok(Some(Json(format!(
            r#"{{"antiCheatLoaded":false,"hostPolicy":"{}","huntEvidence":{},"trustTier":{}}}"#,
            self.config.host_policy.mode.as_str(),
            self.config.hunt_evidence.enabled,
            self.config.trust_tier.number()
        ))))
    }

    fn start(&self, _role: &str) -> Result<(), SeamError> {
        log_host_policy(&self.config.host_policy, self.log.as_ref());
        log_hunt_evidence(&self.config.hunt_evidence, self.log.as_ref());
        Ok(())
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        self.flush_census();
        Box::pin(async { Ok(()) })
    }

    fn tables(&self) -> &'static [TableSpec] {
        TABLES
    }

    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        [
            ("rifts_host_writes", "target"),
            ("rifts_host_writes", "host"),
            ("rifts_hunt_xp_summaries", "member"),
        ]
        .into_iter()
        .map(|(table, account_column)| DeletionStep {
            table,
            account_column,
            action: DeletionAction::Delete,
        })
        .collect()
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
        Box::pin(async move { Ok(self.bucket_now(&request)) })
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
        Box::pin(async move { self.authorize_write_now(&runtime, kind, &target, options) })
    }

    fn record_host_write(&self, runtime: RuntimeAuth, write: HostWrite) -> BoxFuture<'_, Result<(), SeamError>> {
        Box::pin(async move { self.record_write_now(&runtime, &write) })
    }

    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        Ok(self.config.host_policy_refuses())
    }

    fn note_would_refuse(&self, what: &str, detail: &str) -> Result<(), SeamError> {
        self.log
            .warn(&format!("[HostPolicyObserve] would refuse {what} {detail}"));
        Ok(())
    }
}

impl EvidenceIntake for Guard {
    fn submit_evidence(&self, runtime: RuntimeAuth, body: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        Box::pin(async move { self.submit_parsed(&runtime, parse_json(&body).as_ref()) })
    }

    fn complete(&self, runtime: RuntimeAuth, body: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        Box::pin(async move { self.complete_parsed(&runtime, parse_json(&body).as_ref()) })
    }

    fn verdict(&self, session: SessionId, generation: u64) -> BoxFuture<'_, Result<Option<HuntVerdict>, GuardError>> {
        Box::pin(async move { self.verdict_now(&session, generation) })
    }

    fn reward_receipt(
        &self,
        _session: SessionId,
        _generation: u64,
        _account: AccountId,
    ) -> BoxFuture<'_, Result<Option<Json>, GuardError>> {
        Box::pin(async { Ok(None) })
    }

    /// The shadow key is never read or logged: nothing here compares.
    fn record_shadow_comparison(&self, _key: Option<Secret>, _body: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        Box::pin(async { Err(refused("HUNT_EVIDENCE_NOT_COLLECTED", NOT_COLLECTED)) })
    }
}

impl TrafficLimits for Guard {
    fn charge(
        &self,
        host: AccountId,
        session: SessionId,
        bytes: u64,
    ) -> BoxFuture<'_, Result<TrafficDecision, GuardError>> {
        Box::pin(async move { Ok(self.charge_now(&host, &session, bytes)) })
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
            self.operations()
                .acknowledge_alert(&admin_key, &alert_id, body.as_ref(), &ip)
                .await
        })
    }
}

/// Guard with nothing behind it: every operation is `Unimplemented` under its method name.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

fn missing(operation: &'static str) -> Unimplemented {
    Unimplemented::new(Component::Guard, operation)
}

fn seam_stub<T: Send + 'static>(operation: &'static str) -> BoxFuture<'static, Result<T, SeamError>> {
    rifts_seam::stub(Component::Guard, operation)
}

fn guard_stub<T: Send + 'static>(operation: &'static str) -> BoxFuture<'static, Result<T, GuardError>> {
    rifts_seam::stub(Component::Guard, operation)
}

impl Lifecycle for Stub {
    fn runtime_safety_issues(&self, _settings: &dyn Settings) -> Result<Vec<String>, SeamError> {
        Err(missing("runtime_safety_issues").into())
    }

    fn startup_summary(&self, _role: &str) -> Result<Option<Json>, SeamError> {
        Err(missing("startup_summary").into())
    }

    fn start(&self, _role: &str) -> Result<(), SeamError> {
        Err(missing("start").into())
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        seam_stub("stop")
    }

    fn tables(&self) -> &'static [TableSpec] {
        TABLES
    }

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

    fn census(&self, _sample: &HostRouteSample) -> Result<(), SeamError> {
        Err(missing("census").into())
    }

    fn rate_limit_bucket(&self, _request: RequestInfo) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        seam_stub("rate_limit_bucket")
    }

    fn authorize_inventory(
        &self,
        _runtime: RuntimeAuth,
        _transaction: InventoryTransaction,
    ) -> BoxFuture<'_, Result<InventoryDecision, SeamError>> {
        seam_stub("authorize_inventory")
    }

    fn authorize_host_write(
        &self,
        _runtime: RuntimeAuth,
        _kind: HostWriteKind,
        _target: AccountId,
        _options: HostWriteOptions,
    ) -> BoxFuture<'_, Result<Option<Reply>, SeamError>> {
        seam_stub("authorize_host_write")
    }

    fn record_host_write(&self, _runtime: RuntimeAuth, _write: HostWrite) -> BoxFuture<'_, Result<(), SeamError>> {
        seam_stub("record_host_write")
    }

    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        Err(missing("host_policy_refuses").into())
    }

    fn note_would_refuse(&self, _what: &str, _detail: &str) -> Result<(), SeamError> {
        Err(missing("note_would_refuse").into())
    }
}

impl EvidenceIntake for Stub {
    fn submit_evidence(&self, _runtime: RuntimeAuth, _body: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        guard_stub("submit_evidence")
    }

    fn complete(&self, _runtime: RuntimeAuth, _body: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        guard_stub("complete")
    }

    fn verdict(&self, _session: SessionId, _generation: u64) -> BoxFuture<'_, Result<Option<HuntVerdict>, GuardError>> {
        guard_stub("verdict")
    }

    fn reward_receipt(
        &self,
        _session: SessionId,
        _generation: u64,
        _account: AccountId,
    ) -> BoxFuture<'_, Result<Option<Json>, GuardError>> {
        guard_stub("reward_receipt")
    }

    fn record_shadow_comparison(&self, _key: Option<Secret>, _body: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        guard_stub("record_shadow_comparison")
    }
}

impl TrafficLimits for Stub {
    fn charge(
        &self,
        _host: AccountId,
        _session: SessionId,
        _bytes: u64,
    ) -> BoxFuture<'_, Result<TrafficDecision, GuardError>> {
        guard_stub("charge")
    }
}

impl GuardOperations for Stub {
    fn set_reward_rollout(
        &self,
        _admin_key: String,
        _body: Option<Value>,
        _ip: String,
    ) -> BoxFuture<'_, Result<Reply, GuardError>> {
        guard_stub("set_reward_rollout")
    }

    fn acknowledge_alert(
        &self,
        _admin_key: String,
        _alert_id: String,
        _body: Option<Value>,
        _ip: String,
    ) -> BoxFuture<'_, Result<Reply, GuardError>> {
        guard_stub("acknowledge_alert")
    }
}
