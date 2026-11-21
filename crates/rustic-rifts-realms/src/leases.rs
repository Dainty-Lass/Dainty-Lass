// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `Hosting`, the facade the session routes call; the host lease, participant observations, the end of a hub and
//! the lifecycle monitor's sweep. Nothing in the crate schedules the sweep: the caller drives it.

use std::sync::Arc;

use dainty_module_api::NodeId;
use rustic_rifts_admission::{AdmissionStore, Es256Tickets, LinkStatus, PeerLinkRecord};
use rustic_rifts_seam::{AccountId, Actor, HubKind, IslesCore, PeerId, Refusal, Settings, UnixMillis};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::RealmsError;
use crate::capabilities::HostCapabilities;
use crate::db::RealmsDb;
use crate::host_policy::EligibilityPolicy;
use crate::hub::{HubTick, RealmHubPolicy, decide_hub_tick, hub_end_counts_as_host_failure, is_hub_live};
use crate::hub::{presence_purge_at, realm_hosting_enabled};
use crate::hunt::{ArtifactHashes, HostCapabilityRecord, HuntSession, HuntState, ObservationRecord};
use crate::hunt::{TransitionActor, hub_kind_name};
use crate::input::whole_number;
use crate::lifecycle::transition;
use crate::lifecycle::{CONTROL_LIFE_MS, DRAIN_COMPLETION_REASON, HOST_END_STATES, LIFECYCLE_STATES, Recovery};
use crate::lifecycle::{bump, clear_live_fields, end_session, expire_offer, host_loss_is_clean_end, one, queue};
use crate::lifecycle::{is_pre_active_operator_stop_state, mark_suspect, recover_from_suspect, retry_pre_active_host};
use crate::ports::{Clock, GuardSessions, OperationsView, RealmsLog};
use crate::realm::{HostingPatch, HubState, Patch};
use crate::repo::realms::RealmsRepository;
use crate::repo::{hub::RealmHubRepository, hunts::HuntRepository, hunts::HuntTx, hunts::LeaseRenewal};
use crate::tiers::HostingPolicy;
use crate::time::{Later as _, iso, parse_iso};

use HuntState as H;

/// Sessions read per sweep.
const MONITOR_BATCH: u32 = 100;
/// An observation's own clock may be this far from ours, and a recent one is this young.
const OBSERVATION_WINDOW_MS: i64 = 15_000;
const HUNT_MAX_PEERS: i64 = 4;
const HUB_MAX_PEERS: i64 = 64;
/// `Number.MAX_SAFE_INTEGER`.
const LARGEST_SEQUENCE: i64 = (1 << 53) - 1;
/// The realm mirror's active server count stops here.
const ACTIVE_SERVERS_CAP: i64 = 32;

const FENCED: &str = "P2P_GENERATION_FENCED";
const STOPPED: (&str, &str) = (
    "P2P_EMERGENCY_STOP",
    "Player-hosted hunt operations are temporarily stopped.",
);
const STALE_OBSERVATION: (&str, &str) = (
    "P2P_TICKET_INVALID",
    "The participant observation is malformed or stale.",
);
const OFF_HUB: (&str, &str) = (FENCED, "The observation is not bound to the active hub generation.");

/// What a lifecycle step reports to the sweep.
type Step = Result<Option<SweepStep>, RealmsError>;

pub(crate) const LIFECYCLE_ACTOR: Actor = Actor::System("realms-lifecycle");

/// What hosting reads from settings; isles may change `policy` and `realm_hosting` after reading it.
#[derive(Clone, Debug, PartialEq)]
pub struct HostingSettings {
    /// Player hosting's switches and limits.
    pub policy: HostingPolicy,
    /// What a player needs to be offered a host.
    pub eligibility: EligibilityPolicy,
    /// The realm hub settings.
    pub realm_hub: RealmHubPolicy,
    /// Whether realm hosting is on.
    pub realm_hosting: bool,
}

impl HostingSettings {
    /// Reads every part from the server's settings.
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let realm_hosting = realm_hosting_enabled(settings);
        let realm_hub = RealmHubPolicy::from_settings(settings);
        let eligibility = EligibilityPolicy::from_settings(settings);
        let policy = HostingPolicy::from_settings(settings);
        Self {
            policy,
            eligibility,
            realm_hub,
            realm_hosting,
        }
    }
}

/// What a server that issues bootstraps and tickets wires in.
#[derive(Clone)]
pub struct HostingRuntime {
    /// Issues and checks host capabilities.
    pub capabilities: Arc<HostCapabilities>,
    /// Where admission records live.
    pub admission_store: Arc<dyn AdmissionStore>,
    /// Signs admission tickets.
    pub tickets: Arc<Es256Tickets>,
    /// The module's accounts, parties and matchmaking.
    pub core: Arc<dyn IslesCore>,
    /// The backend URL a host's runtime calls.
    pub backend_url: String,
    /// The game server UDP port a host uses when the session sets none.
    pub host_server_udp_port: u16,
    /// Whether the realms module is on.
    pub realms_enabled: bool,
    /// This server's node, for global ids.
    pub node: NodeId,
}

/// What player hosting runs on.
pub struct HostingParts {
    /// The realms database.
    pub db: RealmsDb,
    /// The hosting settings.
    pub settings: HostingSettings,
    /// The time now.
    pub clock: Arc<dyn Clock>,
    /// The operations policy as hosting reads it.
    pub operations: Arc<dyn OperationsView>,
    /// Launcher Guard sessions.
    pub guard: Arc<dyn GuardSessions>,
    /// Where a hunt that lost its host is recovered.
    pub recovery: Arc<dyn Recovery>,
    /// The artifact hashes a host must run, or why they could not be read.
    pub expected_artifacts: Arc<dyn Fn() -> Result<ArtifactHashes, String> + Send + Sync>,
    /// Hosting's log.
    pub log: Arc<dyn RealmsLog>,
}

/// What one lifecycle sweep did to a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SweepStep {
    /// The session became suspect.
    Suspect {
        /// Why, as a code.
        reason: String,
    },
    /// The lease came back; the session returned to its state before suspicion.
    Recovered,
    /// The host offer expired; another host is being chosen.
    OfferExpired,
    /// The host failed before the session went live; another host is being chosen.
    RetriedHost {
        /// Why, as a code.
        reason: String,
    },
    /// The session ended.
    Ended {
        /// Why, as a code.
        reason: String,
        /// The terminal state it ended in.
        state: HuntState,
    },
    /// Host selection ran.
    Selected,
    /// The session was handed to recovery.
    Recovery {
        /// Why, as a code.
        reason: String,
        /// Whether the session had not gone live yet.
        before_active: bool,
    },
    /// Recovery took its next step.
    RecoveryAdvanced,
    /// This many waiting hub entries expired.
    HubEntriesExpired(usize),
}

/// The answer to a host's lease heartbeat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseReply {
    /// Always true.
    pub renewed: bool,
    /// The session.
    pub session_id: String,
    /// Its generation.
    pub generation: i64,
    /// The new lease expiry, as ISO 8601.
    pub lease_expires_at: String,
    /// The session's state, as its wire name.
    pub state: &'static str,
}

/// The answer to a participant's observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationReply {
    /// Always true.
    pub accepted: bool,
    /// The session.
    pub session_id: String,
    /// Its generation.
    pub generation: i64,
    /// Whether the observation was stored; false for a hub or a session that has ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored: Option<bool>,
    /// The session's state, as its wire name.
    pub state: &'static str,
}

/// Player hosting: sessions, hosts, leases, and the sweep that moves them along.
pub struct Hosting {
    pub(crate) parts: HostingParts,
    pub(crate) runtime: Option<HostingRuntime>,
    pub(crate) hunts: HuntRepository,
    pub(crate) hub: RealmHubRepository,
    pub(crate) realms: RealmsRepository,
}

pub(crate) fn refused(code: &str, message: &str) -> RealmsError {
    let refusal = Refusal {
        code: String::from(code),
        message: String::from(message),
    };
    RealmsError::Refused(refusal)
}

pub(crate) fn as_account(account: &str) -> Actor {
    Actor::Account(AccountId(String::from(account)))
}

/// A JSON number, finite and within range.
pub(crate) fn number_in(value: Option<&Value>, min: f64, max: f64) -> Option<f64> {
    let number = value?.as_f64()?;
    (number.is_finite() && number >= min && number <= max).then_some(number)
}

/// That, and a whole value.
pub(crate) fn integer_in(value: Option<&Value>, min: i64, max: i64) -> Option<i64> {
    let whole = whole_number(value?)?;
    (min..=max).contains(&whole).then_some(whole)
}

/// A string of 1 to `max` UTF-16 units without NUL.
pub(crate) fn short_text(value: Option<&Value>, max: usize) -> Option<&str> {
    let text = value?.as_str()?;
    if text.is_empty() || text.contains('\0') || text.encode_utf16().count() > max {
        None
    } else {
        Some(text)
    }
}

/// The reliability change a hub's end makes to its host, if any.
fn hub_end_charge(terminal: HuntState, reason: &str) -> Option<crate::hunt::ReliabilityIncrements> {
    if terminal == H::Completed {
        Some(one(|r| &mut r.clean_completions))
    } else if !hub_end_counts_as_host_failure(reason) {
        None
    } else if reason == "HUB_STARTUP_TIMEOUT" {
        Some(one(|r| &mut r.starts_failed))
    } else {
        Some(one(|r| &mut r.unexpected_failures))
    }
}

/// Who ends a hub, and how.
struct HubEnd {
    id: String,
    generation: i64,
    reason: String,
    terminal: HuntState,
    by: TransitionActor,
    by_account: Option<String>,
    now: UnixMillis,
}

/// The hub generation's end in one transaction: its presence rows close, every guest is told to leave and its host
/// is fenced.
fn close_hub(tx: &mut HuntTx<'_>, end: HubEnd) -> Result<Option<HuntSession>, RealmsError> {
    let HubEnd {
        id,
        generation,
        reason,
        terminal,
        by,
        by_account,
        now,
    } = end;
    let live = |h: &HuntSession| h.is_realm_hub() && h.generation == generation && is_hub_live(h.state);
    let Some(hub) = tx.find_hunt(&id)?.filter(live) else {
        return Ok(None);
    };
    tx.revoke_generation(&id, generation, now)?;
    let close = |h: &mut HuntSession| {
        clear_live_fields(h);
        if terminal == H::Aborted {
            h.failure_reason = Some(reason.clone());
        }
        h.completion_reason = Some(reason.clone());
        (h.state, h.terminal_at) = (terminal, Some(now));
    };
    let Some(ended) = tx.compare_and_set(&id, generation, &[hub.state], hub.state_version, close)? else {
        return Ok(None);
    };
    transition(tx, hub.state, &ended, &reason, by, by_account.as_deref(), now)?;
    let host = ended.host_account_id.as_str();
    let guests = tx.end_hub_presence(&id, generation, now, presence_purge_at(now))?;
    let leave = json!({
        "sessionId": ended.session_id,
        "generation": ended.generation,
        "reason": reason,
        "hubKind": ended.hub_kind.map(hub_kind_name),
        "realmId": ended.realm_id,
    });
    let ttl = CONTROL_LIFE_MS;
    for guest in guests.iter().filter(|row| row.account_id != host) {
        queue(
            tx,
            &guest.account_id,
            "REALM_SESSION_LEAVE",
            &ended,
            leave.clone(),
            now,
            ttl,
        )?;
    }
    if !host.is_empty() {
        let fence = json!({ "reason": reason, "recoveryAction": "NONE", "cleanEnd": terminal == H::Completed });
        queue(tx, host, "FENCE_NOW", &ended, fence, now, ttl)?;
        if let Some(charge) = hub_end_charge(terminal, &reason) {
            bump(tx, host, charge, now)?;
        }
    }
    Ok(Some(ended))
}

/// A participant's observation as the report states it; the steam id is the roster's, filled in later.
fn read_observation(
    body: &Map<String, Value>,
    hunt: &HuntSession,
    account: &str,
    now: UnixMillis,
) -> Result<ObservationRecord, RealmsError> {
    let malformed = || refused(STALE_OBSERVATION.0, STALE_OBSERVATION.1);
    let stamp = short_text(body.get("observedAt"), 40).and_then(parse_iso);
    let observed_at = stamp.ok_or_else(malformed)?;
    let flag = |key: &str| body.get(key).and_then(Value::as_bool).ok_or_else(malformed);
    let host_reachable = flag("hostReachable")?;
    let local_bridge_alive = flag("localBridgeAlive")?;
    let metrics_available = flag("metricsAvailable")?;
    if (observed_at.0 - now.0).abs() > OBSERVATION_WINDOW_MS {
        return Err(malformed());
    }
    // A missing route kind reads as empty, which no kind matches.
    let route_kind = match body.get("routeKind").map(Value::as_str) {
        None => "",
        Some(Some(kind)) => kind,
        Some(None) => return Err(malformed()),
    };
    if !["DIRECT", "RELAY", "CONNECTING", "UNKNOWN"].contains(&route_kind) {
        return Err(malformed());
    }
    let unknown = route_kind == "UNKNOWN";
    let mut record = ObservationRecord {
        observed_at,
        metrics_available,
        local_bridge_alive,
        host_reachable,
        route_kind: route_kind.into(),
        account_id: account.into(),
        generation: hunt.generation,
        session_id: hunt.session_id.clone(),
        observation_id: crate::ids::random_uuid(),
        steam_id: String::new(),
        packet_loss_percent: None,
        jitter_ms: None,
        ping_ms: None,
        last_packet_age_ms: None,
    };
    if !metrics_available {
        // Without metrics the route is unknown and no metric is sent.
        let quiet = ["lastPacketAgeMs", "pingMs", "jitterMs", "packetLossPercent"]
            .iter()
            .all(|key| body.get(*key).is_none_or(Value::is_null));
        return if unknown && quiet { Ok(record) } else { Err(malformed()) };
    }
    if unknown {
        return Err(malformed());
    }
    let millis = |key: &str| {
        number_in(body.get(key), 0.0, 60_000.0)
            .map(|n| n.round() as i64)
            .ok_or_else(malformed)
    };
    record.last_packet_age_ms = Some(integer_in(body.get("lastPacketAgeMs"), 0, 600_000).ok_or_else(malformed)?);
    record.ping_ms = Some(millis("pingMs")?);
    record.jitter_ms = Some(millis("jitterMs")?);
    record.packet_loss_percent = Some(number_in(body.get("packetLossPercent"), 0.0, 100.0).ok_or_else(malformed)?);
    Ok(record)
}

impl Hosting {
    /// Hosting over these parts, with no runtime: it issues no bootstraps or tickets.
    pub fn new(parts: HostingParts) -> Hosting {
        let realms = RealmsRepository::new(parts.db.clone());
        let hub = RealmHubRepository::new(parts.db.clone());
        let hunts = HuntRepository::new(parts.db.clone());
        Hosting {
            realms,
            hub,
            hunts,
            runtime: None,
            parts,
        }
    }

    /// Adds the runtime that issues bootstraps and tickets.
    #[must_use]
    pub fn with_runtime(mut self, runtime: HostingRuntime) -> Self {
        self.runtime.replace(runtime);
        self
    }

    pub(crate) fn now(&self) -> UnixMillis {
        self.parts.clock.now()
    }

    pub(crate) fn settings(&self) -> &HostingSettings {
        &self.parts.settings
    }

    pub(crate) async fn emergency_stop(&self) -> Result<bool, RealmsError> {
        let stopped = self.parts.operations.emergency_stop().await;
        stopped.map_err(RealmsError::Database)
    }

    pub(crate) fn node(&self) -> &NodeId {
        self.parts.db.node()
    }

    /// Drop everything up to the first `:`; a seam id is read as its wire form. Looked up by that, then the raw text.
    pub(crate) async fn find_by_path(&self, raw: &str) -> Result<Option<HuntSession>, RealmsError> {
        let rest = match raw.split_once(':') {
            Some((_, after)) => after,
            None => raw,
        };
        let wire = match rest.parse::<rustic_rifts_seam::SessionId>() {
            Ok(seam) => crate::ids::wire_session_id(&seam),
            Err(_) => rest.to_owned(),
        };
        let found = self.hunts.find_hunt(&wire).await?;
        if found.is_some() || wire == raw {
            return Ok(found);
        }
        Ok(self.hunts.find_hunt(raw).await?)
    }

    /// Under both spellings of the session.
    pub(crate) fn invalidate_hub(&self, session_id: &str) {
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        let cache = runtime.capabilities.hub_presence();
        cache.invalidate(session_id);
        if let Some(seam) = seam_spelling(session_id, self.node()) {
            cache.invalidate(&seam);
        }
    }

    /// The account's link to a peer id, verified; the result is ignored.
    pub(crate) fn ensure_verified_link(&self, account: &str, peer: &str, revision: u64, method: &str) {
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        let Ok(peer) = peer.parse::<u64>().map(PeerId) else {
            return;
        };
        let link = PeerLinkRecord {
            verified_at: self.now(),
            verification_method: method.into(),
            status: LinkStatus::Verified,
            account: AccountId(account.into()),
            revision,
            peer,
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
        let whole = |key: &str, min: i64, max: i64| integer_in(body.get(key), min, max).ok_or_else(malformed);
        let flag = |key: &str| body.get(key).and_then(Value::as_bool).ok_or_else(malformed);
        let sequence = whole("leaseSequence", 1, LARGEST_SEQUENCE)?;
        let (bridge_alive, server_ready) = (flag("bridgeAlive")?, flag("serverReady")?);
        let peers = whole("admittedPeerCount", 0, HUB_MAX_PEERS)?;
        // Checked, not used.
        whole("queuePressure", 0, 100)?;
        let hunt = self.hunts.find_hunt(&capability.session_id).await?;
        // Only a hub takes more than a hunt's party.
        if peers > HUNT_MAX_PEERS && !hunt.as_ref().is_some_and(HuntSession::is_realm_hub) {
            return Err(malformed());
        }
        if !(bridge_alive && server_ready) {
            if let Some(hunt) = hunt {
                self.host_process_down(capability, hunt).await?;
            }
            return Err(refused(FENCED, "The host process reported a fatal failure."));
        }
        if self.emergency_stop().await? {
            return Err(refused(STOPPED.0, STOPPED.1));
        }
        let (now, lease_sequence) = (self.now(), sequence);
        let lease_expires_at = now.saturating_add(self.settings().eligibility.lease_lifetime_ms);
        let HostCapabilityRecord {
            session_id,
            generation,
            host_steam_id,
            bridge_instance_id,
            ..
        } = capability.clone();
        let renewal = LeaseRenewal {
            heartbeat_at: now,
            lease_expires_at,
            lease_sequence,
            bridge_instance_id,
            host_steam_id,
            generation,
            session_id,
        };
        let capability_id = capability.capability_id.clone();
        let host = as_account(&capability.host_account_id);
        let renewed = self
            .hunts
            .transaction(&host, move |tx| {
                // A replayed or reordered heartbeat is refused before it touches the lease.
                match tx.advance_capability_sequence(&capability_id, sequence)? {
                    false => Err(refused(FENCED, "The lease sequence is stale.")),
                    true => tx
                        .renew_host_lease(&renewal)?
                        .ok_or_else(|| refused(FENCED, "The hunt generation no longer accepts heartbeats.")),
                }
            })
            .await?;
        let expires = renewed.lease_expires_at.unwrap_or(now);
        Ok(LeaseReply {
            renewed: true,
            lease_expires_at: iso(expires),
            state: renewed.state.as_str(),
            generation: renewed.generation,
            session_id: renewed.session_id,
        })
    }

    async fn host_process_down(&self, capability: &HostCapabilityRecord, hunt: HuntSession) -> Result<(), RealmsError> {
        const UNHEALTHY: &str = "HOST_PROCESS_UNHEALTHY";
        let host = capability.host_account_id.as_str();
        let by = TransitionActor::Host;
        let (id, held) = (hunt.session_id.as_str(), capability.generation);
        if hunt.is_realm_hub() {
            self.end_hub(id, held, UNHEALTHY, H::Aborted, by, Some(host)).await?;
            return Ok(());
        }
        let actor = as_account(host);
        if hunt.generation == held {
            // A draining hunt ends as it was going to; a clean host loss ends cleanly.
            let clean_end = if hunt.state == H::Draining {
                let reason = hunt.completion_reason.as_deref().unwrap_or(DRAIN_COMPLETION_REASON);
                Some((reason.to_owned(), &[H::Draining][..]))
            } else {
                host_loss_is_clean_end(&hunt).map(|clean| (format!("{clean}_HOST_STOPPED"), &HOST_END_STATES[..]))
            };
            if let Some((reason, allowed)) = clean_end {
                self.end_hunt(&actor, &hunt, &reason, allowed, by, Some(host), true)
                    .await?;
                return Ok(());
            }
        }
        self.parts
            .recovery
            .prepare(id.to_owned(), held, UNHEALTHY.into(), false)
            .await
    }

    #[allow(clippy::too_many_arguments, reason = "each argument is one part of the transition")]
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
        let (id, generation, why) = (hunt.session_id.clone(), hunt.generation, reason.to_owned());
        let (states, by_account) = (allowed.to_vec(), account.map(String::from));
        let work = move |tx: &mut HuntTx<'_>| {
            let by = by_account.as_deref();
            let ended = end_session(tx, &id, generation, &why, &states, transition_actor, by, fence, now)?;
            Ok::<_, RealmsError>(ended)
        };
        let ended = self.hunts.transaction(actor, work).await?;
        if let Some(done) = ended.as_ref() {
            let (session, number, state) = (&done.session_id, done.generation, done.state.as_str());
            self.parts.log.info(&format!(
                "[P2P] session {session} generation={number} ended cleanly state={state} reason={reason}"
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
        let actor = match actor_account {
            Some(account) => as_account(account),
            None => LIFECYCLE_ACTOR,
        };
        let end = HubEnd {
            id: session_id.to_owned(),
            generation,
            reason: reason.to_owned(),
            terminal,
            by: actor_type,
            by_account: actor_account.map(String::from),
            now,
        };
        let ended = self.hunts.transaction(&actor, move |tx| close_hub(tx, end)).await?;
        let Some(hub) = ended.as_ref() else {
            return Ok(ended);
        };
        let (id, state, log) = (&hub.session_id, hub.state.as_str(), &self.parts.log);
        log.info(&format!("[RealmHub] hub {id} ended state={state} reason={reason}"));
        self.invalidate_hub(id);
        self.mirror_hub(hub, HubState::None).await;
        // The realm is offline once its Ramsgate hub is gone.
        if let (Some(HubKind::Ramsgate), Some(realm)) = (hub.hub_kind, hub.realm_id.as_deref())
            && let Err(error) = self.realms.mark_offline(&LIFECYCLE_ACTOR, realm, now).await
        {
            log.warn(&format!("[RealmHub] could not mark realm {realm} offline: {error}"));
        }
        Ok(ended)
    }

    /// The hub's state, and the realm's live server count, as its realm shows them.
    pub(crate) async fn mirror_hub(&self, hub: &HuntSession, state: HubState) {
        let Some(realm) = hub.realm_id.as_deref() else {
            return;
        };
        let counted = self.hunts.count_active_realm_servers(realm).await;
        let mut patch = HostingPatch {
            active_servers: counted.ok().map(|n| n.min(ACTIVE_SERVERS_CAP)),
            ..Default::default()
        };
        match hub.hub_kind {
            Some(HubKind::Training) => patch.training_state = Some(state),
            _ => {
                patch.hub_state = Some(state);
                // A stopped hub no longer names its session.
                patch.hub_session_id = match state {
                    HubState::None => Patch::Remove,
                    _ => Patch::Set(hub.session_id.clone()),
                };
            }
        }
        let (now, log) = (self.now(), &self.parts.log);
        if let Err(error) = self.realms.patch_hosting(&LIFECYCLE_ACTOR, realm, &patch, now).await {
            log.warn(&format!(
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
        let off_roster = || refused(FENCED, "The observation is not bound to the active roster generation.");
        let Some(hunt) = self.find_by_path(session_id).await? else {
            return Err(off_roster());
        };
        let bound = body.get("generation").and_then(Value::as_i64) == Some(hunt.generation);
        let reply = |stored| ObservationReply {
            stored,
            state: hunt.state.as_str(),
            generation: hunt.generation,
            session_id: hunt.session_id.clone(),
            accepted: true,
        };
        if hunt.is_realm_hub() {
            let present =
                hunt.host_account_id == account || self.hub.find_presence(&hunt.session_id, account).await?.is_some();
            if present && bound {
                // A hub's health is its host's lease.
                return Ok(reply(Some(false)));
            }
            return Err(refused(OFF_HUB.0, OFF_HUB.1));
        }
        if !bound || hunt.participant(account).is_none() {
            return Err(off_roster());
        }
        if [H::Completed, H::Aborted, H::Draining].contains(&hunt.state) {
            return Ok(reply(Some(false)));
        }
        let record = read_observation(body, &hunt, account, self.now())?;
        if self.emergency_stop().await? {
            return Err(refused(STOPPED.0, STOPPED.1));
        }
        let (id, generation, who) = (hunt.session_id.clone(), hunt.generation, account.to_owned());
        let work = move |tx: &mut HuntTx<'_>| {
            let current = tx.find_hunt(&id)?.filter(|current| current.generation == generation);
            let steam_id = current
                .as_ref()
                .and_then(|current| current.participant(&who))
                .map(|p| p.steam_id.clone());
            let Some(steam_id) = steam_id else {
                return Err(off_roster());
            };
            tx.record_observation(&ObservationRecord { steam_id, ..record })?;
            Ok(())
        };
        self.hunts.transaction(&as_account(account), work).await?;
        Ok(reply(None))
    }

    /// Node's `P2PHuntLifecycleMonitor.tick`: one failing session never stops the others.
    pub async fn sweep(&self) -> Result<Vec<(String, SweepStep)>, RealmsError> {
        let (states, log) = (&LIFECYCLE_STATES, &self.parts.log);
        let due = self.hunts.list_lifecycle_hunts(states, MONITOR_BATCH).await?;
        let stop = self.emergency_stop().await?;
        let mut steps = Vec::with_capacity(due.len());
        for hunt in due {
            let id = hunt.session_id.clone();
            match self.tick(hunt, stop).await {
                Ok(None) => {}
                Ok(Some(step)) => steps.push((id, step)),
                Err(error) => log.warn(&format!("[P2P Lifecycle] session {id} tick failed: {error}")),
            }
        }
        Ok(steps)
    }

    /// Runs one lifecycle step in its own transaction.
    async fn step<T, F>(&self, work: F) -> Result<T, RealmsError>
    where
        T: Send + 'static,
        F: FnOnce(&mut HuntTx<'_>) -> Result<T, crate::db::RepoError> + Send + 'static,
    {
        self.hunts
            .transaction(&LIFECYCLE_ACTOR, move |tx| Ok::<_, RealmsError>(work(tx)?))
            .await
    }

    async fn retry(&self, hunt: &HuntSession, reason: &str) -> Step {
        let now = self.now();
        let (snapshot, why) = (hunt.clone(), reason.to_owned());
        let retried = self
            .step(move |tx| retry_pre_active_host(tx, &snapshot, &why, now))
            .await?;
        if retried.is_none() {
            return Ok(None);
        }
        self.begin_host_selection(&hunt.session_id).await?;
        Ok(Some(SweepStep::RetriedHost { reason: reason.into() }))
    }

    async fn suspect(&self, hunt: &HuntSession, reason: &'static str) -> Step {
        let lifetime = self.settings().eligibility.suspect_lifetime_ms;
        let (now, snapshot) = (self.now(), hunt.clone());
        let marked = self
            .step(move |tx| mark_suspect(tx, &snapshot, reason, lifetime, now))
            .await?;
        Ok(marked.and(Some(SweepStep::Suspect { reason: reason.into() })))
    }

    /// Only the hub tick records the recovery as a transition (`with_transition`).
    async fn recover(&self, hunt: &HuntSession, with_transition: bool) -> Step {
        let (now, snapshot) = (self.now(), hunt.clone());
        let recovered = self
            .step(move |tx| {
                let back = recover_from_suspect(tx, &snapshot)?;
                if with_transition && let Some(after) = back.as_ref() {
                    transition(
                        tx,
                        H::Suspect,
                        after,
                        "LEASE_RECOVERED",
                        TransitionActor::System,
                        None,
                        now,
                    )?;
                }
                Ok(back)
            })
            .await?;
        Ok(recovered.and(Some(SweepStep::Recovered)))
    }

    async fn prepare(&self, hunt: &HuntSession, reason: &str, before_active: bool) -> Step {
        let (id, generation) = (hunt.session_id.clone(), hunt.generation);
        self.parts
            .recovery
            .prepare(id, generation, reason.to_owned(), before_active)
            .await?;
        Ok(Some(SweepStep::Recovery {
            reason: reason.into(),
            before_active,
        }))
    }

    async fn ended(&self, hunt: &HuntSession, reason: &str, allowed: &[HuntState], fence: bool) -> Step {
        let by = TransitionActor::System;
        let ended = self
            .end_hunt(&LIFECYCLE_ACTOR, hunt, reason, allowed, by, None, fence)
            .await?;
        Ok(ended.map(|done| SweepStep::Ended {
            reason: reason.into(),
            state: done.state,
        }))
    }

    async fn tick_hub(&self, hub: &HuntSession, stop: bool, now: UnixMillis) -> Step {
        let settings = self.settings();
        let hosting_enabled = settings.policy.enabled && settings.realm_hosting;
        let decision = decide_hub_tick(hub, now, stop, hosting_enabled);
        if let HubTick::End { reason } = decision {
            let by = TransitionActor::System;
            let ended = self
                .end_hub(&hub.session_id, hub.generation, reason, H::Aborted, by, None)
                .await?;
            return Ok(ended.map(|done| SweepStep::Ended {
                reason: reason.into(),
                state: done.state,
            }));
        }
        match decision {
            HubTick::Suspect { reason } => self.suspect(hub, reason).await,
            HubTick::Recover => self.recover(hub, true).await,
            // A running hub drops the entries that waited too long.
            HubTick::None if hub.state == H::Running => {
                let purge = presence_purge_at(now);
                let expired = self
                    .hub
                    .expire_stale(&LIFECYCLE_ACTOR, Some(&hub.session_id), now, purge)
                    .await?;
                if expired.is_empty() {
                    return Ok(None);
                }
                self.invalidate_hub(&hub.session_id);
                Ok(Some(SweepStep::HubEntriesExpired(expired.len())))
            }
            _ => Ok(None),
        }
    }

    /// Distinct participants whose recent observation says the host or their bridge is down.
    async fn failing_observers(&self, hunt: &HuntSession, now: UnixMillis) -> Result<usize, RealmsError> {
        let since = UnixMillis(now.0 - OBSERVATION_WINDOW_MS);
        let recent = self
            .hunts
            .list_recent_participant_observations(&hunt.session_id, hunt.generation, since)
            .await?;
        let failing: std::collections::BTreeSet<String> = recent
            .into_iter()
            .filter(|seen| !(seen.host_reachable && seen.local_bridge_alive))
            .map(|seen| seen.account_id)
            .collect();
        Ok(failing.len())
    }

    async fn tick(&self, hunt: HuntSession, stop: bool) -> Step {
        let now = self.now();
        if hunt.is_realm_hub() {
            return self.tick_hub(&hunt, stop, now).await;
        }
        let passed = |deadline: Option<UnixMillis>| matches!(deadline, Some(at) if at <= now);
        if hunt.state == H::Draining {
            let reason = hunt.completion_reason.as_deref().unwrap_or(DRAIN_COMPLETION_REASON);
            let grace = self.settings().eligibility.suspect_lifetime_ms;
            // A host that stopped renewing is gone: nothing left to fence.
            let host_gone = match hunt.lease_expires_at {
                None => true,
                Some(lease) => lease.saturating_add(grace) <= now,
            };
            return match (host_gone, stop || passed(hunt.drain_deadline)) {
                (true, _) if !stop => self.ended(&hunt, reason, &[H::Draining], false).await,
                (_, true) => self.ended(&hunt, reason, &[H::Draining], true).await,
                _ => Ok(None),
            };
        }
        if stop && hunt.state != H::Aborting && hunt.state != H::Recovering {
            return self
                .prepare(
                    &hunt,
                    "OPERATOR_P2P_STOP",
                    is_pre_active_operator_stop_state(hunt.state),
                )
                .await;
        }
        match hunt.state {
            H::HostSelecting => {
                self.begin_host_selection(&hunt.session_id).await?;
                Ok(Some(SweepStep::Selected))
            }
            H::HostOffered if passed(hunt.offer_expires_at) => {
                let snapshot = hunt.clone();
                if self.step(move |tx| expire_offer(tx, &snapshot, now)).await?.is_none() {
                    return Ok(None);
                }
                self.begin_host_selection(&hunt.session_id).await?;
                Ok(Some(SweepStep::OfferExpired))
            }
            H::Starting if passed(hunt.startup_deadline) => self.retry(&hunt, "HOST_STARTUP_TIMEOUT").await,
            H::HostReady | H::Admitting | H::Running => {
                self.tick_started(&hunt, passed(hunt.lease_expires_at), now).await
            }
            H::Suspect => self.tick_suspect(&hunt, passed(hunt.suspect_deadline), now).await,
            H::Recovering if passed(hunt.recovery_deadline) => self.advance_recovery(hunt).await,
            H::Aborting if hunt.recovery_retry_at.is_none_or(|at| at <= now) => self.advance_recovery(hunt).await,
            _ => Ok(None),
        }
    }

    /// A started hunt: a lapsed lease ends it cleanly when it can, else a lapse or a failing quorum makes it suspect.
    async fn tick_started(&self, hunt: &HuntSession, lease_lapsed: bool, now: UnixMillis) -> Step {
        if lease_lapsed && let Some(clean) = host_loss_is_clean_end(hunt) {
            return self
                .ended(hunt, &format!("{clean}_LEASE_RELEASED"), &HOST_END_STATES, true)
                .await;
        }
        let failing = self.failing_observers(hunt, now).await?;
        // An empty roster meets the quorum at once (kept as found).
        let quorum = hunt.roster.len().min(2);
        if !lease_lapsed && failing < quorum {
            return Ok(None);
        }
        let reason = if failing < 2 {
            "HOST_LEASE_MISSED"
        } else {
            "PARTICIPANT_FAILURE_QUORUM"
        };
        self.suspect(hunt, reason).await
    }

    /// A suspect hunt recovers on a fresh lease; once its window ends it ends cleanly, retries or is recovered.
    async fn tick_suspect(&self, hunt: &HuntSession, window_over: bool, now: UnixMillis) -> Step {
        let lease_back = matches!(hunt.lease_expires_at, Some(at) if at > now);
        if lease_back && hunt.pre_suspect_state.is_some() {
            return self.recover(hunt, false).await;
        }
        if !window_over {
            return Ok(None);
        }
        if let Some(clean) = host_loss_is_clean_end(hunt) {
            return self
                .ended(hunt, &format!("{clean}_LEASE_RELEASED"), &HOST_END_STATES, true)
                .await;
        }
        if hunt.pre_suspect_state == Some(H::HostReady) {
            return self.retry(hunt, "HOST_FAILED_BEFORE_RELEASE").await;
        }
        let reason = hunt.failure_reason.as_deref().unwrap_or("HOST_FAILED");
        self.prepare(hunt, reason, false).await
    }

    async fn advance_recovery(&self, hunt: HuntSession) -> Step {
        self.parts.recovery.advance(hunt).await?;
        Ok(Some(SweepStep::RecoveryAdvanced))
    }
}

/// The seam's spelling of a stored session id, when it has one.
fn seam_spelling(session_id: &str, node: &NodeId) -> Option<String> {
    crate::ids::seam_session_id(session_id, node).map(|seam| seam.to_string())
}
