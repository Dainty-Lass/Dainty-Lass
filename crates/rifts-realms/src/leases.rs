// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `Hosting`, the facade the session routes call; the host lease, participant observations, the end of a hub and
//! the lifecycle monitor's sweep. Nothing in the crate schedules the sweep: the caller drives it.

use std::sync::Arc;

use module_api::NodeId;
use rifts_admission::{AdmissionStore, Es256Tickets, LinkStatus, PeerLinkRecord};
use rifts_seam::{AccountId, Actor, HubKind, IslesCore, PeerId, Refusal, Settings, UnixMillis};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::RealmsError;
use crate::capabilities::HostCapabilities;
use crate::db::RealmsDb;
use crate::host_policy::EligibilityPolicy;
use crate::hub::{
    HubTick, RealmHubPolicy, decide_hub_tick, hub_end_counts_as_host_failure, is_hub_live, presence_purge_at,
    realm_hosting_enabled,
};
use crate::hunt::{
    ArtifactHashes, HostCapabilityRecord, HuntSession, HuntState, ObservationRecord, ReliabilityIncrements,
    TransitionActor, hub_kind_name,
};
use crate::ids::{random_uuid, seam_session_id};
use crate::input::whole_number;
use crate::lifecycle::{
    CONTROL_LIFE_MS, DRAIN_COMPLETION_REASON, HOST_END_STATES, LIFECYCLE_STATES, Recovery, end_session, expire_offer,
    host_loss_is_clean_end, is_pre_active_operator_stop_state, mark_suspect, queue, recover_from_suspect,
    retry_pre_active_host, transition,
};
use crate::ports::{Clock, GuardSessions, OperationsView, RealmsLog};
use crate::realm::{HostingPatch, HubState, Patch};
use crate::repo::hub::RealmHubRepository;
use crate::repo::hunts::{HuntRepository, LeaseRenewal};
use crate::repo::realms::RealmsRepository;
use crate::tiers::HostingPolicy;
use crate::time::{Later as _, iso, parse_iso};

use HuntState as H;

const MONITOR_BATCH: u32 = 100;
const OBSERVATION_WINDOW_MS: i64 = 15_000;
const HUNT_MAX_PEERS: i64 = 4;
const HUB_MAX_PEERS: i64 = 64;
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
const ACTIVE_SERVERS_CAP: i64 = 32;

pub(crate) const LIFECYCLE_ACTOR: Actor = Actor::System("realms-lifecycle");

/// What hosting reads from settings; isles may change `policy` and `realm_hosting` after reading it.
#[derive(Debug, Clone, PartialEq)]
pub struct HostingSettings {
    pub policy: HostingPolicy,
    pub eligibility: EligibilityPolicy,
    pub realm_hub: RealmHubPolicy,
    pub realm_hosting: bool,
}

impl HostingSettings {
    pub fn from_settings(settings: &dyn Settings) -> Self {
        Self {
            policy: HostingPolicy::from_settings(settings),
            eligibility: EligibilityPolicy::from_settings(settings),
            realm_hub: RealmHubPolicy::from_settings(settings),
            realm_hosting: realm_hosting_enabled(settings),
        }
    }
}

/// What a server that issues bootstraps and tickets wires in.
#[derive(Clone)]
pub struct HostingRuntime {
    pub capabilities: Arc<HostCapabilities>,
    pub admission_store: Arc<dyn AdmissionStore>,
    pub tickets: Arc<Es256Tickets>,
    pub core: Arc<dyn IslesCore>,
    pub backend_url: String,
    pub host_server_udp_port: u16,
    pub realms_enabled: bool,
    pub node: NodeId,
}

pub struct HostingParts {
    pub db: RealmsDb,
    pub settings: HostingSettings,
    pub clock: Arc<dyn Clock>,
    pub operations: Arc<dyn OperationsView>,
    pub guard: Arc<dyn GuardSessions>,
    pub recovery: Arc<dyn Recovery>,
    pub expected_artifacts: Arc<dyn Fn() -> Result<ArtifactHashes, String> + Send + Sync>,
    pub log: Arc<dyn RealmsLog>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SweepStep {
    Suspect { reason: String },
    Recovered,
    OfferExpired,
    RetriedHost { reason: String },
    Ended { reason: String, state: HuntState },
    Selected,
    Recovery { reason: String, before_active: bool },
    RecoveryAdvanced,
    HubEntriesExpired(usize),
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

pub struct Hosting {
    pub(crate) parts: HostingParts,
    pub(crate) runtime: Option<HostingRuntime>,
    pub(crate) hunts: HuntRepository,
    pub(crate) hub: RealmHubRepository,
    pub(crate) realms: RealmsRepository,
}

pub(crate) fn refused(code: &str, message: &str) -> RealmsError {
    RealmsError::Refused(Refusal {
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

pub(crate) fn as_account(account: &str) -> Actor {
    Actor::Account(AccountId(account.to_owned()))
}

/// A JSON number, finite and within range.
pub(crate) fn number_in(value: Option<&Value>, min: f64, max: f64) -> Option<f64> {
    value?.as_f64().filter(|n| n.is_finite() && (min..=max).contains(n))
}

/// That, and a whole value.
pub(crate) fn integer_in(value: Option<&Value>, min: i64, max: i64) -> Option<i64> {
    whole_number(value?).filter(|n| (min..=max).contains(n))
}

/// A string of 1 to `max` UTF-16 units without NUL.
pub(crate) fn short_text(value: Option<&Value>, max: usize) -> Option<&str> {
    let text = value?.as_str()?;
    let units = text.encode_utf16().count();
    ((1..=max).contains(&units) && !text.contains('\0')).then_some(text)
}

impl Hosting {
    pub fn new(parts: HostingParts) -> Hosting {
        Hosting {
            hunts: HuntRepository::new(parts.db.clone()),
            hub: RealmHubRepository::new(parts.db.clone()),
            realms: RealmsRepository::new(parts.db.clone()),
            parts,
            runtime: None,
        }
    }

    #[must_use]
    pub fn with_runtime(self, runtime: HostingRuntime) -> Self {
        Self {
            runtime: Some(runtime),
            ..self
        }
    }

    pub(crate) fn now(&self) -> UnixMillis {
        self.parts.clock.now()
    }

    pub(crate) fn settings(&self) -> &HostingSettings {
        &self.parts.settings
    }

    pub(crate) async fn emergency_stop(&self) -> Result<bool, RealmsError> {
        self.parts
            .operations
            .emergency_stop()
            .await
            .map_err(RealmsError::Database)
    }

    pub(crate) fn node(&self) -> &NodeId {
        self.parts.db.node()
    }

    /// Drop everything up to the first `:`; a seam id is read as its wire form. Looked up by that, then the raw text.
    pub(crate) async fn find_by_path(&self, raw: &str) -> Result<Option<HuntSession>, RealmsError> {
        let rest = raw.split_once(':').map_or(raw, |(_, rest)| rest);
        let wire = rest
            .parse::<rifts_seam::SessionId>()
            .map_or_else(|_| rest.to_owned(), |seam| crate::ids::wire_session_id(&seam));
        if let Some(hunt) = self.hunts.find_hunt(&wire).await? {
            return Ok(Some(hunt));
        }
        if wire != raw {
            return Ok(self.hunts.find_hunt(raw).await?);
        }
        Ok(None)
    }

    pub(crate) fn invalidate_hub(&self, session_id: &str) {
        if let Some(runtime) = &self.runtime {
            let cache = runtime.capabilities.hub_presence();
            cache.invalidate(session_id);
            if let Some(seam) = seam_session_id(session_id, self.node()) {
                cache.invalidate(&seam.to_string());
            }
        }
    }

    /// The account's link to a peer id, verified; the result is ignored.
    pub(crate) fn ensure_verified_link(&self, account: &str, peer: &str, revision: u64, method: &str) {
        let (Some(runtime), Ok(peer)) = (&self.runtime, peer.parse::<u64>()) else {
            return;
        };
        let link = PeerLinkRecord {
            account: AccountId(account.to_owned()),
            peer: PeerId(peer),
            status: LinkStatus::Verified,
            revision,
            verified_at: self.now(),
            verification_method: method.to_owned(),
        };
        let _ = runtime.admission_store.put_peer_link(&as_account(account), &link);
    }

    /// Node's `RenewHostLease`; the route has checked the capability for `LEASE_HEARTBEAT`.
    pub async fn renew(
        &self,
        capability: &HostCapabilityRecord,
        body: &Map<String, Value>,
    ) -> Result<LeaseReply, RealmsError> {
        let malformed = || refused("P2P_TICKET_INVALID", "The host heartbeat is malformed.");
        let sequence = integer_in(body.get("leaseSequence"), 1, MAX_SAFE_INTEGER as i64).ok_or_else(malformed)?;
        let bridge_alive = body.get("bridgeAlive").and_then(Value::as_bool).ok_or_else(malformed)?;
        let server_ready = body.get("serverReady").and_then(Value::as_bool).ok_or_else(malformed)?;
        let peers = integer_in(body.get("admittedPeerCount"), 0, HUB_MAX_PEERS).ok_or_else(malformed)?;
        integer_in(body.get("queuePressure"), 0, 100).ok_or_else(malformed)?;
        let hunt = self.hunts.find_hunt(&capability.session_id).await?;
        let hub = hunt.as_ref().is_some_and(HuntSession::is_realm_hub);
        if peers > HUNT_MAX_PEERS && !hub {
            return Err(malformed());
        }
        if !bridge_alive || !server_ready {
            if let Some(hunt) = hunt {
                self.host_process_down(capability, hunt).await?;
            }
            return Err(refused(
                "P2P_GENERATION_FENCED",
                "The host process reported a fatal failure.",
            ));
        }
        if self.emergency_stop().await? {
            return Err(refused(
                "P2P_EMERGENCY_STOP",
                "Player-hosted hunt operations are temporarily stopped.",
            ));
        }
        let now = self.now();
        let renewal = LeaseRenewal {
            session_id: capability.session_id.clone(),
            generation: capability.generation,
            host_steam_id: capability.host_steam_id.clone(),
            bridge_instance_id: capability.bridge_instance_id.clone(),
            lease_sequence: sequence,
            heartbeat_at: now,
            lease_expires_at: now.saturating_add(self.settings().eligibility.lease_lifetime_ms),
        };
        let capability_id = capability.capability_id.clone();
        let renewed = self
            .hunts
            .transaction(&as_account(&capability.host_account_id), move |tx| {
                if !tx.advance_capability_sequence(&capability_id, sequence)? {
                    return Err(refused("P2P_GENERATION_FENCED", "The lease sequence is stale."));
                }
                tx.renew_host_lease(&renewal)?.ok_or_else(|| {
                    refused(
                        "P2P_GENERATION_FENCED",
                        "The hunt generation no longer accepts heartbeats.",
                    )
                })
            })
            .await?;
        Ok(LeaseReply {
            renewed: true,
            session_id: renewed.session_id.clone(),
            generation: renewed.generation,
            lease_expires_at: iso(renewed.lease_expires_at.unwrap_or(now)),
            state: renewed.state.as_str(),
        })
    }

    async fn host_process_down(&self, capability: &HostCapabilityRecord, hunt: HuntSession) -> Result<(), RealmsError> {
        let host = capability.host_account_id.clone();
        if hunt.is_realm_hub() {
            self.end_hub(
                &hunt.session_id,
                capability.generation,
                "HOST_PROCESS_UNHEALTHY",
                H::Aborted,
                TransitionActor::Host,
                Some(&host),
            )
            .await?;
            return Ok(());
        }
        let same_generation = hunt.generation == capability.generation;
        if same_generation && hunt.state == H::Draining {
            let reason = hunt
                .completion_reason
                .clone()
                .unwrap_or_else(|| DRAIN_COMPLETION_REASON.to_owned());
            let actor = as_account(&host);
            self.end_hunt(
                &actor,
                &hunt,
                &reason,
                &[H::Draining],
                TransitionActor::Host,
                Some(&host),
                true,
            )
            .await?;
            return Ok(());
        }
        if same_generation && let Some(clean) = host_loss_is_clean_end(&hunt) {
            let actor = as_account(&host);
            let reason = format!("{clean}_HOST_STOPPED");
            self.end_hunt(
                &actor,
                &hunt,
                &reason,
                &HOST_END_STATES,
                TransitionActor::Host,
                Some(&host),
                true,
            )
            .await?;
            return Ok(());
        }
        self.parts
            .recovery
            .prepare(
                hunt.session_id.clone(),
                capability.generation,
                "HOST_PROCESS_UNHEALTHY".to_owned(),
                false,
            )
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn end_hunt(
        &self,
        actor: &Actor,
        hunt: &HuntSession,
        reason: &str,
        allowed: &[HuntState],
        transition_actor: TransitionActor,
        account: Option<&str>,
        fence: bool,
    ) -> Result<Option<HuntSession>, RealmsError> {
        let now = self.now();
        let (id, generation) = (hunt.session_id.clone(), hunt.generation);
        let (reason_owned, allowed, account) = (reason.to_owned(), allowed.to_vec(), account.map(str::to_owned));
        let ended = self
            .hunts
            .transaction(actor, move |tx| -> Result<Option<HuntSession>, RealmsError> {
                Ok(end_session(
                    tx,
                    &id,
                    generation,
                    &reason_owned,
                    &allowed,
                    transition_actor,
                    account.as_deref(),
                    fence,
                    now,
                )?)
            })
            .await?;
        if let Some(ended) = &ended {
            self.parts.log.info(&format!(
                "[P2P] session {} generation={} ended cleanly state={} reason={reason}",
                ended.session_id,
                ended.generation,
                ended.state.as_str()
            ));
        }
        Ok(ended)
    }

    /// Node's `EndRealmHub`: no cooldown, since the launcher restarts a dead hub within seconds.
    pub(crate) async fn end_hub(
        &self,
        session_id: &str,
        generation: i64,
        reason: &str,
        terminal: HuntState,
        actor_type: TransitionActor,
        actor_account: Option<&str>,
    ) -> Result<Option<HuntSession>, RealmsError> {
        let now = self.now();
        let actor = actor_account.map_or(LIFECYCLE_ACTOR, as_account);
        let (id, reason_owned, account) = (
            session_id.to_owned(),
            reason.to_owned(),
            actor_account.map(str::to_owned),
        );
        let ended = self
            .hunts
            .transaction(&actor, move |tx| -> Result<Option<HuntSession>, RealmsError> {
                let reason = reason_owned;
                let Some(hub) = tx.find_hunt(&id)? else {
                    return Ok(None);
                };
                if !hub.is_realm_hub() || hub.generation != generation || !is_hub_live(hub.state) {
                    return Ok(None);
                }
                tx.revoke_generation(&id, generation, now)?;
                let ended = tx.compare_and_set(&id, generation, &[hub.state], hub.state_version, |h| {
                    h.state = terminal;
                    h.terminal_at = Some(now);
                    h.completion_reason = Some(reason.clone());
                    if terminal == H::Aborted {
                        h.failure_reason = Some(reason.clone());
                    }
                    h.active_party_key = None;
                    h.lease_expires_at = None;
                    h.suspect_since = None;
                    h.suspect_deadline = None;
                    h.pre_suspect_state = None;
                    h.offer_expires_at = None;
                    h.startup_deadline = None;
                })?;
                let Some(ended) = ended else {
                    return Ok(None);
                };
                transition(tx, hub.state, &ended, &reason, actor_type, account.as_deref(), now)?;
                let rows = tx.end_hub_presence(&id, generation, now, presence_purge_at(now))?;
                let kind = ended.hub_kind.map(hub_kind_name);
                for row in rows.iter().filter(|row| row.account_id != ended.host_account_id) {
                    let payload = json!({
                        "sessionId": ended.session_id,
                        "generation": ended.generation,
                        "reason": reason,
                        "hubKind": kind,
                        "realmId": ended.realm_id,
                    });
                    queue(
                        tx,
                        &row.account_id,
                        "REALM_SESSION_LEAVE",
                        &ended,
                        payload,
                        now,
                        CONTROL_LIFE_MS,
                    )?;
                }
                let host = ended.host_account_id.clone();
                if !host.is_empty() {
                    let payload = json!({
                        "reason": reason,
                        "recoveryAction": "NONE",
                        "cleanEnd": terminal == H::Completed,
                    });
                    queue(tx, &host, "FENCE_NOW", &ended, payload, now, CONTROL_LIFE_MS)?;
                    let add = if terminal == H::Completed {
                        Some(ReliabilityIncrements {
                            clean_completions: 1,
                            ..ReliabilityIncrements::default()
                        })
                    } else if hub_end_counts_as_host_failure(&reason) {
                        Some(if reason == "HUB_STARTUP_TIMEOUT" {
                            ReliabilityIncrements {
                                starts_failed: 1,
                                ..ReliabilityIncrements::default()
                            }
                        } else {
                            ReliabilityIncrements {
                                unexpected_failures: 1,
                                ..ReliabilityIncrements::default()
                            }
                        })
                    } else {
                        None
                    };
                    if let Some(add) = add {
                        tx.increment_reliability(&host, add, now, None)?;
                    }
                }
                Ok(Some(ended))
            })
            .await?;
        if let Some(hub) = &ended {
            self.parts.log.info(&format!(
                "[RealmHub] hub {} ended state={} reason={reason}",
                hub.session_id,
                hub.state.as_str()
            ));
            self.invalidate_hub(&hub.session_id);
            self.mirror_hub(hub, HubState::None).await;
            if hub.hub_kind == Some(HubKind::Ramsgate)
                && let Some(realm) = &hub.realm_id
                && let Err(error) = self.realms.mark_offline(&LIFECYCLE_ACTOR, realm, now).await
            {
                self.parts
                    .log
                    .warn(&format!("[RealmHub] could not mark realm {realm} offline: {error}"));
            }
        }
        Ok(ended)
    }

    /// The hub's state, and the realm's live server count, as its realm shows them.
    pub(crate) async fn mirror_hub(&self, hub: &HuntSession, state: HubState) {
        let Some(realm) = &hub.realm_id else {
            return;
        };
        let active = self
            .hunts
            .count_active_realm_servers(realm)
            .await
            .ok()
            .map(|n| n.min(ACTIVE_SERVERS_CAP));
        let mut patch = HostingPatch {
            active_servers: active,
            ..HostingPatch::default()
        };
        if hub.hub_kind == Some(HubKind::Training) {
            patch.training_state = Some(state);
        } else {
            patch.hub_state = Some(state);
            patch.hub_session_id = if state == HubState::None {
                Patch::Remove
            } else {
                Patch::Set(hub.session_id.clone())
            };
        }
        if let Err(error) = self
            .realms
            .patch_hosting(&LIFECYCLE_ACTOR, realm, &patch, self.now())
            .await
        {
            self.parts.log.warn(&format!(
                "[RealmHub] could not mirror hub state into realm {realm}: {error}"
            ));
        }
    }

    /// Node's `RecordParticipantObservation`.
    pub async fn observe(
        &self,
        account: &str,
        session_id: &str,
        body: &Map<String, Value>,
    ) -> Result<ObservationReply, RealmsError> {
        let roster_refusal = || {
            refused(
                "P2P_GENERATION_FENCED",
                "The observation is not bound to the active roster generation.",
            )
        };
        let hunt = self.find_by_path(session_id).await?.ok_or_else(roster_refusal)?;
        let generation = body.get("generation").and_then(Value::as_i64);
        let reply = |stored| ObservationReply {
            accepted: true,
            session_id: hunt.session_id.clone(),
            generation: hunt.generation,
            stored,
            state: hunt.state.as_str(),
        };
        if hunt.is_realm_hub() {
            let present =
                hunt.host_account_id == account || self.hub.find_presence(&hunt.session_id, account).await?.is_some();
            if !present || generation != Some(hunt.generation) {
                return Err(refused(
                    "P2P_GENERATION_FENCED",
                    "The observation is not bound to the active hub generation.",
                ));
            }
            // A hub's health is its host's lease.
            return Ok(reply(Some(false)));
        }
        let rostered = hunt.participant(account).is_some();
        if generation != Some(hunt.generation) || !rostered {
            return Err(roster_refusal());
        }
        if matches!(hunt.state, H::Completed | H::Aborted | H::Draining) {
            return Ok(reply(Some(false)));
        }
        let record = self.observation_record(account, &hunt, body)?;
        if self.emergency_stop().await? {
            return Err(refused(
                "P2P_EMERGENCY_STOP",
                "Player-hosted hunt operations are temporarily stopped.",
            ));
        }
        let (id, generation, who) = (hunt.session_id.clone(), hunt.generation, account.to_owned());
        self.hunts
            .transaction(&as_account(account), move |tx| -> Result<(), RealmsError> {
                let current = tx.find_hunt(&id)?.ok_or_else(roster_refusal)?;
                let Some(participant) = current.participant(&who).filter(|_| current.generation == generation) else {
                    return Err(roster_refusal());
                };
                let record = ObservationRecord {
                    steam_id: participant.steam_id.clone(),
                    ..record
                };
                tx.record_observation(&record)?;
                Ok(())
            })
            .await?;
        Ok(reply(None))
    }

    fn observation_record(
        &self,
        account: &str,
        hunt: &HuntSession,
        body: &Map<String, Value>,
    ) -> Result<ObservationRecord, RealmsError> {
        let malformed = || {
            refused(
                "P2P_TICKET_INVALID",
                "The participant observation is malformed or stale.",
            )
        };
        let observed_at = short_text(body.get("observedAt"), 40)
            .and_then(parse_iso)
            .ok_or_else(malformed)?
            .0;
        let flag = |name: &str| body.get(name).and_then(Value::as_bool).ok_or_else(malformed);
        let (host_reachable, local_bridge_alive, metrics) = (
            flag("hostReachable")?,
            flag("localBridgeAlive")?,
            flag("metricsAvailable")?,
        );
        if (observed_at - self.now().0).abs() > OBSERVATION_WINDOW_MS {
            return Err(malformed());
        }
        let route = match body.get("routeKind") {
            None => "",
            Some(value) => value.as_str().ok_or_else(malformed)?,
        };
        if !matches!(route, "DIRECT" | "RELAY" | "CONNECTING" | "UNKNOWN") {
            return Err(malformed());
        }
        let metric_keys = ["lastPacketAgeMs", "pingMs", "jitterMs", "packetLossPercent"];
        let (age, ping, jitter, loss) = if metrics {
            if route == "UNKNOWN" {
                return Err(malformed());
            }
            let age = integer_in(body.get("lastPacketAgeMs"), 0, 600_000).ok_or_else(malformed)?;
            let ping = number_in(body.get("pingMs"), 0.0, 60_000.0).ok_or_else(malformed)?;
            let jitter = number_in(body.get("jitterMs"), 0.0, 60_000.0).ok_or_else(malformed)?;
            let loss = number_in(body.get("packetLossPercent"), 0.0, 100.0).ok_or_else(malformed)?;
            (
                Some(age),
                Some(ping.round() as i64),
                Some(jitter.round() as i64),
                Some(loss),
            )
        } else {
            let absent = metric_keys.iter().all(|key| body.get(*key).is_none_or(Value::is_null));
            if route != "UNKNOWN" || !absent {
                return Err(malformed());
            }
            (None, None, None, None)
        };
        Ok(ObservationRecord {
            observation_id: random_uuid(),
            session_id: hunt.session_id.clone(),
            generation: hunt.generation,
            account_id: account.to_owned(),
            steam_id: String::new(),
            host_reachable,
            local_bridge_alive,
            metrics_available: metrics,
            last_packet_age_ms: age,
            route_kind: route.to_owned(),
            ping_ms: ping,
            jitter_ms: jitter,
            packet_loss_percent: loss,
            observed_at: UnixMillis(observed_at),
        })
    }

    /// Node's `P2PHuntLifecycleMonitor.tick`: one failing session never stops the others.
    pub async fn sweep(&self) -> Result<Vec<(String, SweepStep)>, RealmsError> {
        let sessions = self
            .hunts
            .list_lifecycle_hunts(&LIFECYCLE_STATES, MONITOR_BATCH)
            .await?;
        let stop = self.emergency_stop().await?;
        let mut steps = Vec::new();
        for session in sessions {
            let id = session.session_id.clone();
            match self.tick(session, stop).await {
                Ok(Some(step)) => steps.push((id, step)),
                Ok(None) => {}
                Err(error) => self
                    .parts
                    .log
                    .warn(&format!("[P2P Lifecycle] session {id} tick failed: {error}")),
            }
        }
        Ok(steps)
    }

    async fn retry(&self, hunt: &HuntSession, reason: &str) -> Result<Option<SweepStep>, RealmsError> {
        let now = self.now();
        let (hunt_owned, reason_owned) = (hunt.clone(), reason.to_owned());
        let retried = self
            .hunts
            .transaction(
                &LIFECYCLE_ACTOR,
                move |tx| -> Result<Option<HuntSession>, RealmsError> {
                    Ok(retry_pre_active_host(tx, &hunt_owned, &reason_owned, now)?)
                },
            )
            .await?;
        if retried.is_none() {
            return Ok(None);
        }
        self.begin_host_selection(&hunt.session_id).await?;
        Ok(Some(SweepStep::RetriedHost {
            reason: reason.to_owned(),
        }))
    }

    async fn suspect(&self, hunt: &HuntSession, reason: &'static str) -> Result<Option<SweepStep>, RealmsError> {
        let now = self.now();
        let lifetime = self.settings().eligibility.suspect_lifetime_ms;
        let hunt_owned = hunt.clone();
        let marked = self
            .hunts
            .transaction(
                &LIFECYCLE_ACTOR,
                move |tx| -> Result<Option<HuntSession>, RealmsError> {
                    Ok(mark_suspect(tx, &hunt_owned, reason, lifetime, now)?)
                },
            )
            .await?;
        Ok(marked.map(|_| SweepStep::Suspect {
            reason: reason.to_owned(),
        }))
    }

    async fn recover(&self, hunt: &HuntSession, with_transition: bool) -> Result<Option<SweepStep>, RealmsError> {
        let now = self.now();
        let hunt_owned = hunt.clone();
        let recovered = self
            .hunts
            .transaction(
                &LIFECYCLE_ACTOR,
                move |tx| -> Result<Option<HuntSession>, RealmsError> {
                    let recovered = recover_from_suspect(tx, &hunt_owned)?;
                    if with_transition && let Some(back) = &recovered {
                        transition(
                            tx,
                            H::Suspect,
                            back,
                            "LEASE_RECOVERED",
                            TransitionActor::System,
                            None,
                            now,
                        )?;
                    }
                    Ok(recovered)
                },
            )
            .await?;
        Ok(recovered.map(|_| SweepStep::Recovered))
    }

    async fn prepare(
        &self,
        hunt: &HuntSession,
        reason: &str,
        before_active: bool,
    ) -> Result<Option<SweepStep>, RealmsError> {
        self.parts
            .recovery
            .prepare(
                hunt.session_id.clone(),
                hunt.generation,
                reason.to_owned(),
                before_active,
            )
            .await?;
        Ok(Some(SweepStep::Recovery {
            reason: reason.to_owned(),
            before_active,
        }))
    }

    async fn ended(
        &self,
        hunt: &HuntSession,
        reason: &str,
        allowed: &[HuntState],
        fence: bool,
    ) -> Result<Option<SweepStep>, RealmsError> {
        let ended = self
            .end_hunt(
                &LIFECYCLE_ACTOR,
                hunt,
                reason,
                allowed,
                TransitionActor::System,
                None,
                fence,
            )
            .await?;
        Ok(ended.map(|ended| SweepStep::Ended {
            reason: reason.to_owned(),
            state: ended.state,
        }))
    }

    async fn tick_hub(&self, hub: &HuntSession, stop: bool, now: UnixMillis) -> Result<Option<SweepStep>, RealmsError> {
        let settings = self.settings();
        let hosting_enabled = settings.realm_hosting && settings.policy.enabled;
        match decide_hub_tick(hub, now, stop, hosting_enabled) {
            HubTick::End { reason } => {
                let ended = self
                    .end_hub(
                        &hub.session_id,
                        hub.generation,
                        reason,
                        H::Aborted,
                        TransitionActor::System,
                        None,
                    )
                    .await?;
                Ok(ended.map(|ended| SweepStep::Ended {
                    reason: reason.to_owned(),
                    state: ended.state,
                }))
            }
            HubTick::Suspect { reason } => self.suspect(hub, reason).await,
            HubTick::Recover => self.recover(hub, true).await,
            HubTick::None if hub.state == H::Running => {
                let expired = self
                    .hub
                    .expire_stale(&LIFECYCLE_ACTOR, Some(&hub.session_id), now, presence_purge_at(now))
                    .await?;
                if expired.is_empty() {
                    return Ok(None);
                }
                self.invalidate_hub(&hub.session_id);
                Ok(Some(SweepStep::HubEntriesExpired(expired.len())))
            }
            HubTick::None => Ok(None),
        }
    }

    async fn failing_observers(&self, hunt: &HuntSession, now: UnixMillis) -> Result<usize, RealmsError> {
        let recent = self
            .hunts
            .list_recent_participant_observations(
                &hunt.session_id,
                hunt.generation,
                UnixMillis(now.0 - OBSERVATION_WINDOW_MS),
            )
            .await?;
        let mut failing: Vec<String> = recent
            .into_iter()
            .filter(|o| !o.host_reachable || !o.local_bridge_alive)
            .map(|o| o.account_id)
            .collect();
        failing.sort();
        failing.dedup();
        Ok(failing.len())
    }

    async fn tick(&self, hunt: HuntSession, stop: bool) -> Result<Option<SweepStep>, RealmsError> {
        let now = self.now();
        if hunt.is_realm_hub() {
            return self.tick_hub(&hunt, stop, now).await;
        }
        let lapsed_at = |deadline: Option<UnixMillis>| deadline.is_some_and(|at| at <= now);
        if hunt.state == H::Draining {
            let reason = hunt
                .completion_reason
                .clone()
                .unwrap_or_else(|| DRAIN_COMPLETION_REASON.to_owned());
            let lifetime = self.settings().eligibility.suspect_lifetime_ms;
            let lapsed = hunt
                .lease_expires_at
                .is_none_or(|lease| lease.saturating_add(lifetime) <= now);
            if lapsed && !stop {
                return self.ended(&hunt, &reason, &[H::Draining], false).await;
            }
            if stop || lapsed_at(hunt.drain_deadline) {
                return self.ended(&hunt, &reason, &[H::Draining], true).await;
            }
            return Ok(None);
        }
        if stop && !matches!(hunt.state, H::Aborting | H::Recovering) {
            let before_active = is_pre_active_operator_stop_state(hunt.state);
            return self.prepare(&hunt, "OPERATOR_P2P_STOP", before_active).await;
        }
        match hunt.state {
            H::HostSelecting => {
                self.begin_host_selection(&hunt.session_id).await?;
                Ok(Some(SweepStep::Selected))
            }
            H::HostOffered if lapsed_at(hunt.offer_expires_at) => {
                let hunt_owned = hunt.clone();
                let moved = self
                    .hunts
                    .transaction(
                        &LIFECYCLE_ACTOR,
                        move |tx| -> Result<Option<HuntSession>, RealmsError> {
                            Ok(expire_offer(tx, &hunt_owned, now)?)
                        },
                    )
                    .await?;
                if moved.is_none() {
                    return Ok(None);
                }
                self.begin_host_selection(&hunt.session_id).await?;
                Ok(Some(SweepStep::OfferExpired))
            }
            H::Starting if lapsed_at(hunt.startup_deadline) => self.retry(&hunt, "HOST_STARTUP_TIMEOUT").await,
            H::HostReady | H::Admitting | H::Running => {
                let lapsed = lapsed_at(hunt.lease_expires_at);
                if lapsed && let Some(clean) = host_loss_is_clean_end(&hunt) {
                    let reason = format!("{clean}_LEASE_RELEASED");
                    return self.ended(&hunt, &reason, &HOST_END_STATES, true).await;
                }
                let failing = self.failing_observers(&hunt, now).await?;
                // An empty roster meets the quorum at once (kept as found).
                if lapsed || failing >= hunt.roster.len().min(2) {
                    let reason = if failing >= 2 {
                        "PARTICIPANT_FAILURE_QUORUM"
                    } else {
                        "HOST_LEASE_MISSED"
                    };
                    return self.suspect(&hunt, reason).await;
                }
                Ok(None)
            }
            H::Suspect => {
                if hunt.lease_expires_at.is_some_and(|at| at > now) && hunt.pre_suspect_state.is_some() {
                    return self.recover(&hunt, false).await;
                }
                if !lapsed_at(hunt.suspect_deadline) {
                    return Ok(None);
                }
                if let Some(clean) = host_loss_is_clean_end(&hunt) {
                    let reason = format!("{clean}_LEASE_RELEASED");
                    return self.ended(&hunt, &reason, &HOST_END_STATES, true).await;
                }
                if hunt.pre_suspect_state == Some(H::HostReady) {
                    return self.retry(&hunt, "HOST_FAILED_BEFORE_RELEASE").await;
                }
                let reason = hunt.failure_reason.clone().unwrap_or_else(|| "HOST_FAILED".to_owned());
                self.prepare(&hunt, &reason, false).await
            }
            H::Recovering if lapsed_at(hunt.recovery_deadline) => {
                self.parts.recovery.advance(hunt).await?;
                Ok(Some(SweepStep::RecoveryAdvanced))
            }
            H::Aborting if hunt.recovery_retry_at.is_none_or(|at| at <= now) => {
                self.parts.recovery.advance(hunt).await?;
                Ok(Some(SweepStep::RecoveryAdvanced))
            }
            _ => Ok(None),
        }
    }
}
