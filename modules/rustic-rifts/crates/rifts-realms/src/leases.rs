//! Leases and the lifecycle monitor (`RenewHostLease`, `RecordParticipantObservation`, `P2PHuntLifecycleMonitor`,
//! `TickRealmHub`, `EndRealmHub`): the host renews its session every couple of seconds; the monitor moves on every
//! session whose offer, startup, lease or suspect window ran out. Host selection is [`crate::selection`]'s half.

use std::sync::Arc;

use rifts_seam::{AccountId, Actor, Refusal, Settings, UnixMillis};
use serde::Serialize;
use serde_json::{Value, json};

use crate::RealmsError;
use crate::db::RealmsDb;
use crate::host_policy::EligibilityPolicy;
use crate::hub::{HubTick, RealmHubPolicy, decide_hub_tick, hub_end_counts_as_host_failure, is_hub_live};
use crate::hunt::{HostCapabilityRecord, HuntSession, HuntState, HuntTransition, ObservationRecord, TransitionActor};
use crate::ids::random_uuid;
use crate::lifecycle::{
    DRAIN_COMPLETION_REASON, HOST_END_STATES, LIFECYCLE_STATES, Recovery, end_session, expire_offer,
    host_loss_is_clean_end, is_pre_active_operator_stop_state, mark_suspect, recover_from_suspect,
    retry_pre_active_host, string_in,
};
use crate::ports::{Clock, GuardSessions, OperationsView, RealmsLog};
use crate::realm::{HostingPatch, HubState, Patch};
use crate::repo::hub::RealmHubRepository;
use crate::repo::hunts::{HuntRepository, NewControlMessage};
use crate::repo::realms::RealmsRepository;
use crate::tiers::HostingPolicy;
use crate::time::iso;

use HuntState::{Aborting, Admitting, Draining, HostOffered, HostReady, HostSelecting, Recovering, Running, Starting, Suspect};

const LIFECYCLE_SCAN_LIMIT: u32 = 100;
pub(crate) const MAX_OBSERVATION_AGE_MS: i64 = 15_000;
const HUNT_ADMITTED_PEER_LIMIT: i64 = 4;
const HUB_ADMITTED_PEER_LIMIT: i64 = 64;
const CONTROL_MESSAGE_LIFETIME_MS: i64 = 10 * 60_000;

pub(crate) fn refused(code: &str, message: &str) -> RealmsError {
    RealmsError::Refused(Refusal {
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

fn fenced(message: &str) -> RealmsError {
    refused("P2P_GENERATION_FENCED", message)
}

/// `NumberIn`.
pub(crate) fn number_in(value: Option<&Value>, min: f64, max: f64) -> Option<f64> {
    value?.as_f64().filter(|n| n.is_finite() && *n >= min && *n <= max)
}

/// `IntegerIn`.
pub(crate) fn integer_in(value: Option<&Value>, min: f64, max: f64) -> Option<i64> {
    #[allow(clippy::cast_possible_truncation)]
    number_in(value, min, max).filter(|n| n.fract() == 0.0).map(|n| n as i64)
}

/// The settings the lifecycle reads, taken once.
#[derive(Debug, Clone, PartialEq)]
pub struct HostingSettings {
    pub policy: HostingPolicy,
    pub eligibility: EligibilityPolicy,
    pub realm_hub: RealmHubPolicy,
    /// `MYSTICPARADOX_REALMS_HOSTING`.
    pub realm_hosting: bool,
}

impl HostingSettings {
    pub fn from_settings(settings: &dyn Settings) -> Self {
        Self {
            policy: HostingPolicy::from_settings(settings),
            eligibility: EligibilityPolicy::from_settings(settings),
            realm_hub: RealmHubPolicy::from_settings(settings),
            realm_hosting: crate::hub::realm_hosting_enabled(settings),
        }
    }
}

/// What the hosting side runs on.
pub struct HostingParts {
    pub db: RealmsDb,
    pub settings: HostingSettings,
    pub clock: Arc<dyn Clock>,
    pub operations: Arc<dyn OperationsView>,
    pub guard: Arc<dyn GuardSessions>,
    pub recovery: Arc<dyn Recovery>,
    /// The artifact hashes a host must match, from the feed (`ReadP2PArtifactPolicy`); `Err` leaves hosts unverified.
    pub expected_artifacts: Arc<dyn Fn() -> Result<crate::hunt::ArtifactHashes, String> + Send + Sync>,
    pub log: Arc<dyn RealmsLog>,
}

/// The host's side of a session: leases, observations, host selection and the monitor.
pub struct Hosting {
    pub(crate) hunts: HuntRepository,
    pub(crate) hub: RealmHubRepository,
    pub(crate) realms: RealmsRepository,
    pub(crate) settings: HostingSettings,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) operations: Arc<dyn OperationsView>,
    pub(crate) guard: Arc<dyn GuardSessions>,
    pub(crate) recovery: Arc<dyn Recovery>,
    pub(crate) expected_artifacts: Arc<dyn Fn() -> Result<crate::hunt::ArtifactHashes, String> + Send + Sync>,
    pub(crate) log: Arc<dyn RealmsLog>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseReply {
    pub renewed: bool,
    pub session_id: String,
    pub generation: i64,
    pub lease_expires_at: String,
    pub state: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationReply {
    pub accepted: bool,
    pub session_id: String,
    pub generation: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored: Option<bool>,
    pub state: &'static str,
}

/// What one monitor pass did to a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SweepStep {
    Suspect { reason: String },
    Recovered,
    OfferExpired,
    RetriedHost { reason: String },
    Ended { reason: String, state: HuntState },
    Selected,
    /// Handed to isles' recovery.
    Recovery { reason: String, before_active: bool },
    RecoveryAdvanced,
    HubEntriesExpired(usize),
}

fn host_actor(capability: &HostCapabilityRecord) -> Actor {
    Actor::Account(AccountId(capability.host_account_id.clone()))
}

const SYSTEM: Actor = Actor::System("realms-lifecycle");

impl Hosting {
    pub fn new(parts: HostingParts) -> Self {
        Self {
            hunts: HuntRepository::new(parts.db.clone()),
            hub: RealmHubRepository::new(parts.db.clone()),
            realms: RealmsRepository::new(parts.db),
            settings: parts.settings,
            clock: parts.clock,
            operations: parts.operations,
            guard: parts.guard,
            recovery: parts.recovery,
            expected_artifacts: parts.expected_artifacts,
            log: parts.log,
        }
    }

    pub(crate) fn now(&self) -> UnixMillis {
        self.clock.now()
    }

    pub(crate) async fn emergency_stop(&self) -> Result<bool, RealmsError> {
        self.operations.emergency_stop().await.map_err(RealmsError::Database)
    }

    /// `RenewHostLease` with the capability `RequireHostCapability(LEASE_HEARTBEAT)` checked.
    pub async fn renew(&self, capability: &HostCapabilityRecord, body: &Value) -> Result<LeaseReply, RealmsError> {
        let malformed = || refused("P2P_TICKET_INVALID", "The host heartbeat is malformed.");
        let sequence = integer_in(body.get("leaseSequence"), 1.0, 9_007_199_254_740_991.0).ok_or_else(malformed)?;
        let (Some(bridge_alive), Some(server_ready)) = (
            body.get("bridgeAlive").and_then(Value::as_bool),
            body.get("serverReady").and_then(Value::as_bool),
        ) else {
            return Err(malformed());
        };
        #[allow(clippy::cast_precision_loss)]
        let peers = integer_in(body.get("admittedPeerCount"), 0.0, HUB_ADMITTED_PEER_LIMIT as f64);
        if peers.is_none() || integer_in(body.get("queuePressure"), 0.0, 100.0).is_none() {
            return Err(malformed());
        }
        // Only a realm hub may report more than a hunt's four admitted peers
        if peers.is_some_and(|n| n > HUNT_ADMITTED_PEER_LIMIT)
            && !self
                .hunts
                .find_hunt(&capability.session_id)
                .await?
                .is_some_and(|h| h.is_realm_hub())
        {
            return Err(malformed());
        }
        if !bridge_alive || !server_ready {
            self.host_process_down(capability).await?;
            return Err(fenced("The host process reported a fatal failure."));
        }
        let now = self.now();
        let lease_expires_at = UnixMillis(now.0 + self.settings.eligibility.lease_lifetime_ms);
        if self.emergency_stop().await? {
            return Err(refused(
                "P2P_EMERGENCY_STOP",
                "Player-hosted hunt operations are temporarily stopped.",
            ));
        }
        let renewal = crate::repo::hunts::LeaseRenewal {
            session_id: capability.session_id.clone(),
            generation: capability.generation,
            host_steam_id: capability.host_steam_id.clone(),
            bridge_instance_id: capability.bridge_instance_id.clone(),
            lease_sequence: sequence,
            heartbeat_at: now,
            lease_expires_at,
        };
        let capability_id = capability.capability_id.clone();
        let hunt = self
            .hunts
            .transaction(&host_actor(capability), move |tx| {
                if !tx.advance_capability_sequence(&capability_id, sequence)? {
                    return Err(fenced("The lease sequence is stale."));
                }
                tx.renew_host_lease(&renewal)?
                    .ok_or_else(|| fenced("The hunt generation no longer accepts heartbeats."))
            })
            .await?;
        Ok(LeaseReply {
            renewed: true,
            session_id: hunt.session_id,
            generation: hunt.generation,
            lease_expires_at: iso(lease_expires_at),
            state: hunt.state.as_str(),
        })
    }

    /// The host's process is gone: a hub ends ABORTED, a drained hunt or a finished one ends cleanly, anything else
    /// goes to recovery.
    async fn host_process_down(&self, capability: &HostCapabilityRecord) -> Result<(), RealmsError> {
        let current = self.hunts.find_hunt(&capability.session_id).await?;
        let host = Some(capability.host_account_id.as_str());
        let actor = host_actor(capability);
        match current {
            Some(hunt) if hunt.is_realm_hub() => {
                self.end_hub(&hunt.session_id, capability.generation, "HOST_PROCESS_UNHEALTHY", HuntState::Aborted, TransitionActor::Host, host)
                    .await?;
            }
            Some(hunt) if hunt.state == Draining && hunt.generation == capability.generation => {
                let reason = hunt.completion_reason.clone().unwrap_or_else(|| DRAIN_COMPLETION_REASON.to_owned());
                self.end_hunt(&actor, &hunt, &reason, &[Draining], TransitionActor::Host, host, true).await?;
            }
            Some(hunt) if hunt.generation == capability.generation && host_loss_is_clean_end(&hunt).is_some() => {
                let reason = format!("{}_HOST_STOPPED", host_loss_is_clean_end(&hunt).unwrap_or_default());
                self.end_hunt(&actor, &hunt, &reason, HOST_END_STATES, TransitionActor::Host, host, true).await?;
            }
            _ => {
                self.recovery
                    .prepare(capability.session_id.clone(), capability.generation, "HOST_PROCESS_UNHEALTHY".to_owned(), false)
                    .await?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn end_hunt(
        &self,
        actor: &Actor,
        hunt: &HuntSession,
        reason: &str,
        allowed: &[HuntState],
        transition_actor: TransitionActor,
        transition_account: Option<&str>,
        fence: bool,
    ) -> Result<Option<HuntSession>, RealmsError> {
        let (id, generation, reason_text, allowed) =
            (hunt.session_id.clone(), hunt.generation, reason.to_owned(), allowed.to_vec());
        let account = transition_account.map(str::to_owned);
        let now = self.now();
        let ended = self
            .hunts
            .transaction(actor, move |tx| {
                end_session(tx, &id, generation, &reason_text, &allowed, transition_actor, account.as_deref(), fence, now)
            })
            .await?;
        if let Some(record) = &ended {
            self.log.info(&format!(
                "[P2P] session {} generation={} ended cleanly state={} reason={reason}",
                record.session_id,
                generation,
                record.state.as_str()
            ));
        }
        Ok(ended)
    }

    /// `EndRealmHub` (a hub never released): revoke, close, its members LOST and told, its host fenced, the host's
    /// reliability, then the realm's hosting record mirrored. The Ramsgate hub's realm hunts are isles' recovery.
    pub async fn end_hub(
        &self,
        session_id: &str,
        generation: i64,
        reason: &str,
        terminal: HuntState,
        actor_type: TransitionActor,
        actor_account: Option<&str>,
    ) -> Result<Option<HuntSession>, RealmsError> {
        let now = self.now();
        let (id, reason_text, account) = (session_id.to_owned(), reason.to_owned(), actor_account.map(str::to_owned));
        let actor = account.clone().map_or(SYSTEM, |a| Actor::Account(AccountId(a)));
        let ended = self
            .hunts
            .transaction(&actor, move |tx| {
                let Some(hub) = tx.find_hunt(&id)? else {
                    return Ok::<_, RealmsError>(None);
                };
                if !hub.is_realm_hub() || hub.generation != generation || !is_hub_live(hub.state) {
                    return Ok(None);
                }
                tx.revoke_generation(&id, generation, now)?;
                let reason_set = reason_text.clone();
                let Some(closed) = tx.compare_and_set(&id, generation, &[hub.state], hub.state_version, move |h| {
                    h.state = terminal;
                    h.terminal_at = Some(now);
                    h.completion_reason = Some(reason_set.clone());
                    if terminal == HuntState::Aborted {
                        h.failure_reason = Some(reason_set);
                    }
                    h.active_party_key = None;
                    h.lease_expires_at = None;
                    h.suspect_since = None;
                    h.suspect_deadline = None;
                    h.pre_suspect_state = None;
                    h.offer_expires_at = None;
                    h.startup_deadline = None;
                })?
                else {
                    return Ok(None);
                };
                tx.append_transition(&HuntTransition {
                    transition_id: String::new(),
                    session_id: id.clone(),
                    generation,
                    from_state: hub.state,
                    to_state: terminal,
                    state_version: closed.state_version,
                    reason: reason_text.clone(),
                    actor_type: actor_type.as_str().to_owned(),
                    actor_account_id: account.clone(),
                    at: now,
                })?;
                let rows = tx.end_hub_presence(&id, generation, now, crate::hub::presence_purge_at(now))?;
                let expires = UnixMillis(now.0 + CONTROL_MESSAGE_LIFETIME_MS);
                let kind = hub.hub_kind.map(crate::realm::hub_kind_name);
                for row in rows.iter().filter(|r| r.session_id == id && r.account_id != hub.host_account_id) {
                    tx.enqueue_control(&NewControlMessage {
                        account_id: row.account_id.clone(),
                        message_type: "REALM_SESSION_LEAVE".to_owned(),
                        session_id: Some(id.clone()),
                        generation: Some(generation),
                        payload: json!({ "sessionId": id, "generation": generation, "reason": reason_text,
                            "hubKind": kind, "realmId": hub.realm_id }),
                        created_at: now,
                        expires_at: expires,
                    })?;
                }
                tx.enqueue_control(&NewControlMessage {
                    account_id: hub.host_account_id.clone(),
                    message_type: "FENCE_NOW".to_owned(),
                    session_id: Some(id.clone()),
                    generation: Some(generation),
                    payload: json!({ "reason": reason_text, "recoveryAction": "NONE",
                        "cleanEnd": terminal == HuntState::Completed }),
                    created_at: now,
                    expires_at: expires,
                })?;
                let add = if terminal == HuntState::Completed {
                    Some(crate::hunt::ReliabilityIncrements {
                        clean_completions: 1,
                        ..Default::default()
                    })
                } else if hub_end_counts_as_host_failure(&reason_text) {
                    // No cooldown: the launcher restarts a dead hub within seconds
                    Some(if reason_text == "HUB_STARTUP_TIMEOUT" {
                        crate::hunt::ReliabilityIncrements {
                            starts_failed: 1,
                            ..Default::default()
                        }
                    } else {
                        crate::hunt::ReliabilityIncrements {
                            unexpected_failures: 1,
                            ..Default::default()
                        }
                    })
                } else {
                    None
                };
                if let Some(add) = add {
                    tx.increment_reliability(&hub.host_account_id, add, now, None)?;
                }
                Ok(Some(closed))
            })
            .await?;
        if let Some(hub) = &ended {
            self.log.info(&format!(
                "[RealmHub] hub {} ended state={} reason={reason}",
                hub.session_id,
                hub.state.as_str()
            ));
            self.mirror_hub(hub, HubState::None).await;
            if hub.hub_kind == Some(rifts_seam::HubKind::Ramsgate)
                && let Some(realm) = &hub.realm_id
                && let Err(error) = self.realms.mark_offline(&SYSTEM, realm, now).await
            {
                self.log.warn(&format!("[RealmHub] could not mark realm {realm} offline: {error}"));
            }
        }
        Ok(ended)
    }

    /// `MirrorWithCount`: the hub's state and the realm's live servers into its hosting record; a failure is logged.
    pub(crate) async fn mirror_hub(&self, hub: &HuntSession, state: HubState) {
        let Some(realm) = &hub.realm_id else {
            return;
        };
        let active = self.hunts.count_active_realm_servers(realm).await.ok().map(|n| n.min(32));
        let mut patch = if hub.hub_kind == Some(rifts_seam::HubKind::Training) {
            HostingPatch {
                training_state: Some(state),
                ..HostingPatch::default()
            }
        } else {
            HostingPatch {
                hub_state: Some(state),
                hub_session_id: if state == HubState::None {
                    Patch::Remove
                } else {
                    Patch::Set(hub.session_id.clone())
                },
                ..HostingPatch::default()
            }
        };
        patch.active_servers = active;
        if let Err(error) = self.realms.patch_hosting(&SYSTEM, realm, &patch, self.now()).await {
            self.log.warn(&format!("[RealmHub] could not mirror hub state into realm {realm}: {error}"));
        }
    }

    /// `RecordParticipantObservation`.
    pub async fn observe(&self, account: &str, session_id: &str, body: &Value) -> Result<ObservationReply, RealmsError> {
        let not_bound = || fenced("The observation is not bound to the active roster generation.");
        let generation = body.get("generation").and_then(Value::as_i64);
        let Some(hunt) = self.hunts.find_hunt(session_id).await? else {
            return Err(not_bound());
        };
        let accepted = |stored: Option<bool>, hunt: &HuntSession| ObservationReply {
            accepted: true,
            session_id: session_id.to_owned(),
            generation: hunt.generation,
            stored,
            state: hunt.state.as_str(),
        };
        // A hub's health is its host's lease: an observation from someone it knows is accepted and dropped
        if hunt.is_realm_hub() {
            let known = hunt.host_account_id == account || self.hub.find_presence(session_id, account).await?.is_some();
            if !known || generation != Some(hunt.generation) {
                return Err(fenced("The observation is not bound to the active hub generation."));
            }
            return Ok(accepted(Some(false), &hunt));
        }
        let rostered = hunt.participant(account).is_some();
        // An ended or drained hunt keeps no observations; a member's report is answered with its state
        if matches!(hunt.state, HuntState::Completed | HuntState::Aborted | Draining) {
            if generation != Some(hunt.generation) || !rostered {
                return Err(not_bound());
            }
            return Ok(accepted(Some(false), &hunt));
        }
        let Some(participant) = hunt.participant(account).cloned() else {
            return Err(not_bound());
        };
        if generation != Some(hunt.generation) {
            return Err(not_bound());
        }
        let malformed = || refused("P2P_TICKET_INVALID", "The participant observation is malformed or stale.");
        let observed = string_in(body.get("observedAt"), 40)
            .as_deref()
            .and_then(crate::time::parse_iso)
            .ok_or_else(malformed)?;
        let route = body.get("routeKind").and_then(Value::as_str).unwrap_or_default().to_owned();
        let metrics = body.get("metricsAvailable").and_then(Value::as_bool);
        let absent = |key: &str| body.get(key).is_none_or(Value::is_null);
        let metrics_valid = match metrics {
            Some(true) => {
                route != "UNKNOWN"
                    && integer_in(body.get("lastPacketAgeMs"), 0.0, 600_000.0).is_some()
                    && number_in(body.get("pingMs"), 0.0, 60_000.0).is_some()
                    && number_in(body.get("jitterMs"), 0.0, 60_000.0).is_some()
                    && number_in(body.get("packetLossPercent"), 0.0, 100.0).is_some()
            }
            _ => {
                route == "UNKNOWN"
                    && absent("lastPacketAgeMs")
                    && absent("pingMs")
                    && absent("jitterMs")
                    && absent("packetLossPercent")
            }
        };
        let (Some(reachable), Some(alive), Some(metrics)) = (
            body.get("hostReachable").and_then(Value::as_bool),
            body.get("localBridgeAlive").and_then(Value::as_bool),
            metrics,
        ) else {
            return Err(malformed());
        };
        if (self.now().0 - observed.0).abs() > MAX_OBSERVATION_AGE_MS
            || !["DIRECT", "RELAY", "CONNECTING", "UNKNOWN"].contains(&route.as_str())
            || !metrics_valid
        {
            return Err(malformed());
        }
        #[allow(clippy::cast_possible_truncation)]
        let whole = |key: &str| metrics.then(|| body.get(key).and_then(Value::as_f64).map(|n| n.round() as i64)).flatten();
        let observation = ObservationRecord {
            observation_id: random_uuid(),
            session_id: session_id.to_owned(),
            generation: hunt.generation,
            account_id: account.to_owned(),
            steam_id: participant.steam_id,
            host_reachable: reachable,
            local_bridge_alive: alive,
            metrics_available: metrics,
            last_packet_age_ms: whole("lastPacketAgeMs"),
            route_kind: route,
            ping_ms: whole("pingMs"),
            jitter_ms: whole("jitterMs"),
            packet_loss_percent: metrics.then(|| body.get("packetLossPercent").and_then(Value::as_f64)).flatten(),
            observed_at: observed,
        };
        if self.emergency_stop().await? {
            return Err(refused(
                "P2P_EMERGENCY_STOP",
                "Player-hosted hunt operations are temporarily stopped.",
            ));
        }
        let (id, generation, who) = (session_id.to_owned(), hunt.generation, account.to_owned());
        self.hunts
            .transaction(&Actor::Account(AccountId(account.to_owned())), move |tx| {
                let current = tx.find_hunt(&id)?;
                if !current.is_some_and(|c| c.generation == generation && c.participant(&who).is_some()) {
                    return Err(fenced("The observation is not bound to the active roster generation."));
                }
                tx.record_observation(&observation)?;
                Ok(())
            })
            .await?;
        Ok(accepted(None, &hunt))
    }

    /// One monitor pass (`P2PHuntLifecycleMonitor.tick`): every session in a lifecycle state, oldest first. One failing
    /// session does not stop the others; its error is logged.
    pub async fn sweep(&self) -> Result<Vec<(String, SweepStep)>, RealmsError> {
        let hunts = self.hunts.list_lifecycle_hunts(LIFECYCLE_STATES, LIFECYCLE_SCAN_LIMIT).await?;
        let stop = self.emergency_stop().await?;
        let mut steps = Vec::new();
        for hunt in hunts {
            let id = hunt.session_id.clone();
            match self.tick(hunt, stop).await {
                Ok(Some(step)) => steps.push((id, step)),
                Ok(None) => {}
                Err(error) => self.log.warn(&format!("[P2P Lifecycle] session {id} tick failed: {error}")),
            }
        }
        Ok(steps)
    }

    async fn tick(&self, hunt: HuntSession, stop: bool) -> Result<Option<SweepStep>, RealmsError> {
        let now = self.now();
        if hunt.is_realm_hub() {
            return self.tick_hub(&hunt, now, stop).await;
        }
        if hunt.state == Draining {
            return self.tick_draining(&hunt, now, stop).await;
        }
        if stop && ![Aborting, Recovering].contains(&hunt.state) {
            let before = is_pre_active_operator_stop_state(hunt.state);
            return self.hand_to_recovery(&hunt, "OPERATOR_P2P_STOP", before).await;
        }
        match hunt.state {
            HostSelecting => {
                self.begin_host_selection(&hunt.session_id).await?;
                Ok(Some(SweepStep::Selected))
            }
            HostOffered if hunt.offer_expires_at.is_some_and(|at| at <= now) => {
                let expired = self.hunts.transaction(&SYSTEM, move |tx| expire_offer(tx, &hunt, now)).await?;
                if let Some(next) = expired {
                    self.begin_host_selection(&next.session_id).await?;
                    return Ok(Some(SweepStep::OfferExpired));
                }
                Ok(None)
            }
            Starting if hunt.startup_deadline.is_some_and(|at| at <= now) => self.retry(&hunt, "HOST_STARTUP_TIMEOUT").await,
            HostReady | Admitting | Running => self.tick_running(&hunt, now).await,
            Suspect => {
                if hunt.lease_expires_at.is_some_and(|at| at > now) && hunt.pre_suspect_state.is_some() {
                    let recovered = self.hunts.transaction(&SYSTEM, move |tx| recover_from_suspect(tx, &hunt)).await?;
                    return Ok(recovered.map(|_| SweepStep::Recovered));
                }
                if !hunt.suspect_deadline.is_some_and(|at| at <= now) {
                    return Ok(None);
                }
                if let Some(clean) = host_loss_is_clean_end(&hunt) {
                    return self.clean_end(&hunt, &format!("{clean}_LEASE_RELEASED")).await;
                }
                if hunt.pre_suspect_state == Some(HostReady) {
                    return self.retry(&hunt, "HOST_FAILED_BEFORE_RELEASE").await;
                }
                let reason = hunt.failure_reason.clone().unwrap_or_else(|| "HOST_FAILED".to_owned());
                self.hand_to_recovery(&hunt, &reason, false).await
            }
            Recovering if hunt.recovery_deadline.is_some_and(|at| at <= now) => {
                self.recovery.advance(hunt).await?;
                Ok(Some(SweepStep::RecoveryAdvanced))
            }
            Aborting if hunt.recovery_retry_at.is_none_or(|at| at <= now) => {
                self.recovery.advance(hunt).await?;
                Ok(Some(SweepStep::RecoveryAdvanced))
            }
            _ => Ok(None),
        }
    }

    async fn tick_running(&self, hunt: &HuntSession, now: UnixMillis) -> Result<Option<SweepStep>, RealmsError> {
        let lapsed = hunt.lease_expires_at.is_some_and(|at| at <= now);
        // The host's lease ending after the hunt was completed or left is its clean exit
        if lapsed && let Some(clean) = host_loss_is_clean_end(hunt) {
            return self.clean_end(hunt, &format!("{clean}_LEASE_RELEASED")).await;
        }
        let observations = self
            .hunts
            .list_recent_participant_observations(&hunt.session_id, hunt.generation, UnixMillis(now.0 - MAX_OBSERVATION_AGE_MS))
            .await?;
        let mut failing: Vec<&str> = observations
            .iter()
            .filter(|o| !o.host_reachable || !o.local_bridge_alive)
            .map(|o| o.account_id.as_str())
            .collect();
        failing.sort_unstable();
        failing.dedup();
        if lapsed || failing.len() >= hunt.roster.len().min(2) {
            let reason = if failing.len() >= 2 {
                "PARTICIPANT_FAILURE_QUORUM"
            } else {
                "HOST_LEASE_MISSED"
            };
            let suspect_ms = self.settings.eligibility.suspect_lifetime_ms;
            let hunt = hunt.clone();
            let marked = self
                .hunts
                .transaction(&SYSTEM, move |tx| mark_suspect(tx, &hunt, reason, suspect_ms, now))
                .await?;
            return Ok(marked.map(|_| SweepStep::Suspect {
                reason: reason.to_owned(),
            }));
        }
        Ok(None)
    }

    /// A drained hunt never goes SUSPECT and never recovers: its players are on their way to another hunt.
    async fn tick_draining(&self, hunt: &HuntSession, now: UnixMillis, stop: bool) -> Result<Option<SweepStep>, RealmsError> {
        let reason = hunt.completion_reason.clone().unwrap_or_else(|| DRAIN_COMPLETION_REASON.to_owned());
        let lapsed = hunt
            .lease_expires_at
            .is_none_or(|at| at.0 + self.settings.eligibility.suspect_lifetime_ms <= now.0);
        let fence = if lapsed && !stop {
            false
        } else if stop || hunt.drain_deadline.is_some_and(|at| at <= now) {
            true
        } else {
            return Ok(None);
        };
        let ended = self
            .end_hunt(&SYSTEM, hunt, &reason, &[Draining], TransitionActor::System, None, fence)
            .await?;
        Ok(ended.map(|h| SweepStep::Ended { reason, state: h.state }))
    }

    async fn tick_hub(&self, hub: &HuntSession, now: UnixMillis, stop: bool) -> Result<Option<SweepStep>, RealmsError> {
        let enabled = self.settings.realm_hosting && self.settings.policy.enabled;
        match decide_hub_tick(hub, now, stop, enabled) {
            HubTick::End { reason } => {
                let ended = self
                    .end_hub(&hub.session_id, hub.generation, reason, HuntState::Aborted, TransitionActor::System, None)
                    .await?;
                Ok(ended.map(|h| SweepStep::Ended {
                    reason: reason.to_owned(),
                    state: h.state,
                }))
            }
            HubTick::Suspect { reason } => {
                let suspect_ms = self.settings.eligibility.suspect_lifetime_ms;
                let hub = hub.clone();
                let marked = self
                    .hunts
                    .transaction(&SYSTEM, move |tx| mark_suspect(tx, &hub, reason, suspect_ms, now))
                    .await?;
                Ok(marked.map(|_| SweepStep::Suspect {
                    reason: reason.to_owned(),
                }))
            }
            HubTick::Recover => {
                let hub = hub.clone();
                let recovered = self
                    .hunts
                    .transaction(&SYSTEM, move |tx| {
                        let next = recover_from_suspect(tx, &hub)?;
                        if let Some(next) = &next {
                            tx.append_transition(&HuntTransition {
                                transition_id: String::new(),
                                session_id: hub.session_id.clone(),
                                generation: hub.generation,
                                from_state: Suspect,
                                to_state: next.state,
                                state_version: next.state_version,
                                reason: "LEASE_RECOVERED".to_owned(),
                                actor_type: TransitionActor::System.as_str().to_owned(),
                                actor_account_id: None,
                                at: now,
                            })?;
                        }
                        Ok::<_, RealmsError>(next)
                    })
                    .await?;
                Ok(recovered.map(|_| SweepStep::Recovered))
            }
            HubTick::None if hub.state == Running => {
                let expired = self
                    .hub
                    .expire_stale(&SYSTEM, Some(&hub.session_id), now, crate::hub::presence_purge_at(now))
                    .await?;
                Ok((!expired.is_empty()).then_some(SweepStep::HubEntriesExpired(expired.len())))
            }
            HubTick::None => Ok(None),
        }
    }

    async fn clean_end(&self, hunt: &HuntSession, reason: &str) -> Result<Option<SweepStep>, RealmsError> {
        let ended = self
            .end_hunt(&SYSTEM, hunt, reason, HOST_END_STATES, TransitionActor::System, None, true)
            .await?;
        Ok(ended.map(|h| SweepStep::Ended {
            reason: reason.to_owned(),
            state: h.state,
        }))
    }

    async fn retry(&self, hunt: &HuntSession, reason: &'static str) -> Result<Option<SweepStep>, RealmsError> {
        let now = self.now();
        let snapshot = hunt.clone();
        let retried = self
            .hunts
            .transaction(&SYSTEM, move |tx| retry_pre_active_host(tx, &snapshot, reason, now))
            .await?;
        let Some(next) = retried else {
            return Ok(None);
        };
        self.begin_host_selection(&next.session_id).await?;
        Ok(Some(SweepStep::RetriedHost {
            reason: reason.to_owned(),
        }))
    }

    pub(crate) async fn hand_to_recovery(
        &self,
        hunt: &HuntSession,
        reason: &str,
        before_active: bool,
    ) -> Result<Option<SweepStep>, RealmsError> {
        self.recovery
            .prepare(hunt.session_id.clone(), hunt.generation, reason.to_owned(), before_active)
            .await?;
        Ok(Some(SweepStep::Recovery {
            reason: reason.to_owned(),
            before_active,
        }))
    }
}
