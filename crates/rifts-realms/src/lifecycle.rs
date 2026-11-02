// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The hosted hunt's lifecycle: the pure rules, the steps that each run inside one hunt transaction, and recovery
//! when a session cannot go on.

use std::sync::Arc;

use rifts_seam::{AccountId, Actor, BoxFuture, IslesCore, UnixMillis};
use serde_json::{Map, Value, json};

use crate::RealmsError;
use crate::db::{RealmsDb, RepoError};
use crate::hunt::{
    AdmissionState, HuntSession, HuntState, HuntTransition, OfferStatus, Participant, ReliabilityIncrements,
    TransitionActor,
};
use crate::ports::{Clock, RealmsLog};
use crate::repo::hunts::{HuntRepository, HuntTx, NewControlMessage};
use crate::time::Later as _;

use HuntState as H;

/// What the monitor scans.
pub const LIFECYCLE_STATES: [HuntState; 10] = [
    H::HostSelecting,
    H::HostOffered,
    H::Starting,
    H::HostReady,
    H::Admitting,
    H::Running,
    H::Draining,
    H::Suspect,
    H::Aborting,
    H::Recovering,
];
/// The host's own end applies in these.
pub const HOST_END_STATES: [HuntState; 4] = [H::HostReady, H::Admitting, H::Running, H::Suspect];
/// The last hub return closes these.
pub const HUB_RETURN_END_STATES: [HuntState; 3] = [H::HostReady, H::Admitting, H::Running];
/// A party's new request may supersede these.
pub const SUPERSEDE_STATES: [HuntState; 8] = [
    H::HostSelecting,
    H::HostOffered,
    H::Starting,
    H::HostReady,
    H::Admitting,
    H::Running,
    H::Suspect,
    H::Aborting,
];
const STARTED_STATES: [HuntState; 4] = [H::HostReady, H::Admitting, H::Running, H::Suspect];
pub const CANCELLABLE_FORMING_STATES: [HuntState; 5] = [
    H::HostSelecting,
    H::HostOffered,
    H::Starting,
    H::HostReady,
    H::Admitting,
];

pub const DRAIN_COMPLETION_REASON: &str = "SUPERSEDED_REQUESTER_LEFT_HUNT";
pub const HOST_GUARD_START_GRACE_MS: i64 = 90_000;

pub(crate) const CONTROL_LIFE_MS: i64 = 10 * 60 * 1000;
const RETRY_FENCE_LIFE_MS: i64 = 5 * 60 * 1000;
const RECOVERY_COOLDOWN_MS: i64 = 5 * 60 * 1000;

pub fn hunt_ran(hunt: &HuntSession) -> bool {
    matches!(hunt.state, H::Running | H::Draining)
        || (hunt.state == H::Suspect && hunt.pre_suspect_state == Some(H::Running))
}

/// Before active the whole roster; once active, the admitted participants whose local bridge came up.
pub fn select_recovery_participants(roster: &[Participant], before_active: bool) -> Vec<Participant> {
    if before_active {
        return roster.to_vec();
    }
    roster
        .iter()
        .filter(|p| {
            p.admission_state == AdmissionState::Admitted
                && p.local_bridge_ready_at.is_some()
                && p.local_bridge_port.is_some()
        })
        .cloned()
        .collect()
}

pub fn should_use_dedicated_pre_active_fallback(before_active: bool, dedicated_hunts_disabled: bool) -> bool {
    before_active && !dedicated_hunts_disabled
}

pub fn is_pre_active_operator_stop_state(state: HuntState) -> bool {
    matches!(state, H::HostSelecting | H::HostOffered | H::Starting | H::HostReady)
}

/// A Launcher Guard session's role; a host's covers a client's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GuardRole {
    Client,
    Host,
}

impl GuardRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Host => "host",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "client" => Some(Self::Client),
            "host" => Some(Self::Host),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuardListError {
    pub code: &'static str,
    pub message: &'static str,
}

const SESSIONS_MALFORMED: GuardListError = GuardListError {
    code: "P2P_TICKET_INVALID",
    message: "The Launcher Guard session list is malformed.",
};
const ROLES_MALFORMED: GuardListError = GuardListError {
    code: "P2P_TICKET_INVALID",
    message: "The Launcher Guard role list is malformed.",
};

fn guard_id_text(value: &Value) -> Option<&str> {
    let text = value.as_str()?;
    let units = text.encode_utf16().count();
    ((1..=64).contains(&units) && !text.contains('\0')).then_some(text)
}

/// `guardSessions`, or the older single `guardSessionId` as a client session.
pub fn parse_guard_presence_sessions(body: &Value) -> Result<Vec<(GuardRole, String)>, GuardListError> {
    match body.get("guardSessions") {
        None | Some(Value::Null) => Ok(body
            .get("guardSessionId")
            .and_then(guard_id_text)
            .map(|id| vec![(GuardRole::Client, id.to_owned())])
            .unwrap_or_default()),
        Some(Value::Array(entries)) if entries.len() <= 2 => {
            let mut sessions: Vec<(GuardRole, String)> = Vec::new();
            for entry in entries {
                let role = entry
                    .get("role")
                    .and_then(Value::as_str)
                    .and_then(GuardRole::parse)
                    .ok_or(SESSIONS_MALFORMED)?;
                let id = entry
                    .get("guardSessionId")
                    .and_then(guard_id_text)
                    .filter(|id| id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
                    .ok_or(SESSIONS_MALFORMED)?;
                if sessions.iter().any(|(seen, _)| *seen == role) {
                    return Err(SESSIONS_MALFORMED);
                }
                sessions.push((role, id.to_owned()));
            }
            Ok(sessions)
        }
        Some(_) => Err(SESSIONS_MALFORMED),
    }
}

/// A stale server session must not stand in for the game client: `host` needs `client`, unless only a realm host
/// is asking.
pub fn parse_guard_required_roles(
    body: &Value,
    sessions: &[(GuardRole, String)],
    realm_host_only: bool,
) -> Result<Vec<GuardRole>, GuardListError> {
    let roles = match body.get("guardRequiredRoles") {
        None | Some(Value::Null) => sessions.iter().map(|(role, _)| *role).collect(),
        Some(Value::Array(entries)) if entries.len() <= 2 => {
            let mut roles: Vec<GuardRole> = Vec::new();
            for entry in entries {
                let role = entry.as_str().and_then(GuardRole::parse).ok_or(ROLES_MALFORMED)?;
                if roles.contains(&role) {
                    return Err(ROLES_MALFORMED);
                }
                roles.push(role);
            }
            roles
        }
        Some(_) => return Err(ROLES_MALFORMED),
    };
    let host_only = roles.contains(&GuardRole::Host) && !roles.contains(&GuardRole::Client);
    if host_only && !(realm_host_only && roles == [GuardRole::Host]) {
        return Err(GuardListError {
            code: "GUARD_REQUIRED",
            message: "A hosted process requires both client and host Guard roles.",
        });
    }
    Ok(roles)
}

/// The host role's Guard session is allowed a grace while the hosted server starts.
pub fn host_guard_still_starting(
    created_at: UnixMillis,
    required: &[GuardRole],
    verified: &[GuardRole],
    now: UnixMillis,
) -> bool {
    let unverified: Vec<&GuardRole> = required.iter().filter(|r| !verified.contains(r)).collect();
    !unverified.is_empty()
        && unverified.iter().all(|r| **r == GuardRole::Host)
        && (0..HOST_GUARD_START_GRACE_MS).contains(&(now.0 - created_at.0))
}

/// In roster order: admitted and not back in the hub, or the host while it has not lost its link.
pub fn participants_still_in_hunt(roster: &[Participant], host: &str) -> Vec<String> {
    roster
        .iter()
        .filter(|p| {
            p.returned_to_hub_at.is_none()
                && (p.admission_state == AdmissionState::Admitted
                    || (p.account_id == host && p.admission_state != AdmissionState::Lost))
        })
        .map(|p| p.account_id.clone())
        .collect()
}

/// A tier-1 COMPLETED is Guard's completion, never a clean end's.
pub fn clean_end_terminal_state(hunt: &HuntSession) -> HuntState {
    if hunt.trust_tier.unwrap_or(0) == 0 && hunt_ran(hunt) {
        H::Completed
    } else {
        H::Aborted
    }
}

pub fn host_loss_is_clean_end(hunt: &HuntSession) -> Option<&'static str> {
    if hunt.trust_tier.unwrap_or(0) != 0 || !STARTED_STATES.contains(&hunt.state) {
        return None;
    }
    if hunt.guard_encounter_state.as_deref() == Some("COMPLETION_REQUESTED") {
        return Some("TIER0_HUNT_COMPLETED");
    }
    (hunt_ran(hunt) && participants_still_in_hunt(&hunt.roster, &hunt.host_account_id).is_empty())
        .then_some("TIER0_PARTY_LEFT")
}

/// The new roster, whether this was the first return, and who is still in the hunt.
pub fn mark_participant_returned_to_hub(
    roster: &[Participant],
    account: &str,
    at: UnixMillis,
    host: &str,
) -> Option<(Vec<Participant>, bool, Vec<String>)> {
    let entry = roster.iter().find(|p| p.account_id == account)?;
    let eligible = account == host || matches!(entry.admission_state, AdmissionState::Admitted | AdmissionState::Lost);
    if !eligible {
        return None;
    }
    let first = entry.returned_to_hub_at.is_none();
    let updated: Vec<Participant> = roster
        .iter()
        .map(|p| {
            let mut p = p.clone();
            if p.account_id == account && p.returned_to_hub_at.is_none() {
                p.returned_to_hub_at = Some(at);
            }
            p
        })
        .collect();
    let remaining = participants_still_in_hunt(&updated, host);
    Some((updated, first, remaining))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartySessionAction {
    Reuse,
    Supersede(&'static str),
    Drain(&'static str),
    Conflict(&'static str),
}

fn requester_left(existing: &HuntSession, requester: &str) -> bool {
    let entry = existing.participant(requester);
    entry.is_some_and(|p| {
        matches!(p.admission_state, AdmissionState::Admitted | AdmissionState::Lost) || p.returned_to_hub_at.is_some()
    }) || (existing.state == H::Running && existing.host_account_id == requester)
}

fn classify_raw(existing: &HuntSession, hunt_id: &str, requester: &str, now: UnixMillis) -> PartySessionAction {
    use PartySessionAction as A;
    let state = existing.state;
    if matches!(state, H::Aborting | H::Suspect) {
        return A::Supersede("SESSION_ENDING");
    }
    if matches!(
        existing.guard_encounter_state.as_deref(),
        Some("COMPLETION_REQUESTED" | "ABORTED")
    ) {
        return A::Supersede("HUNT_FINISHED");
    }
    let started = STARTED_STATES.contains(&state);
    if started && existing.lease_expires_at.is_some_and(|at| at <= now) {
        return A::Supersede("HOST_LEASE_EXPIRED");
    }
    if started && requester_left(existing, requester) {
        let drains = state == H::Running && existing.trust_tier.unwrap_or(0) == 0 && !existing.is_realm_hub();
        return if drains {
            A::Drain("REQUESTER_LEFT_HUNT")
        } else {
            A::Supersede("REQUESTER_LEFT_HUNT")
        };
    }
    if existing.hunt_definition_id != hunt_id {
        return A::Supersede("PARTY_CHANGED_HUNT");
    }
    if existing.participant(requester).is_none() {
        return if matches!(state, H::Running | H::Admitting) {
            A::Conflict("ROSTER_FROZEN")
        } else {
            A::Supersede("ROSTER_CHANGED")
        };
    }
    A::Reuse
}

/// A running tier-0 hunt keeps serving its players: superseding it drains it instead, unless its host is gone.
pub fn classify_existing_party_session(
    existing: &HuntSession,
    hunt_id: &str,
    requester: &str,
    now: UnixMillis,
) -> PartySessionAction {
    match classify_raw(existing, hunt_id, requester, now) {
        PartySessionAction::Supersede(reason)
            if existing.state == H::Running
                && existing.trust_tier.unwrap_or(0) == 0
                && !existing.is_realm_hub()
                && !matches!(reason, "HOST_LEASE_EXPIRED" | "SESSION_ENDING") =>
        {
            PartySessionAction::Drain(reason)
        }
        action => action,
    }
}

pub fn is_cancellable_forming(hunt: &HuntSession) -> bool {
    CANCELLABLE_FORMING_STATES.contains(&hunt.state)
        && !hunt
            .roster
            .iter()
            .any(|p| p.admission_state == AdmissionState::Admitted)
}

/// (game mode, hunt id, whether it is a hub) the party goes back to.
pub fn recovery_destination(before_active: bool, hunt_id: &str) -> (&'static str, String, bool) {
    if !before_active {
        ("CITY", "ShatteredIsles_ReturnToRamsgate".to_owned(), true)
    } else if crate::tiers::is_training_hunt_id(hunt_id) {
        ("SHARED", hunt_id.to_owned(), true)
    } else {
        ("ISLAND", hunt_id.to_owned(), false)
    }
}

pub(crate) fn transition(
    tx: &mut HuntTx<'_>,
    from: HuntState,
    to: &HuntSession,
    reason: &str,
    actor: TransitionActor,
    account: Option<&str>,
    at: UnixMillis,
) -> Result<(), RepoError> {
    tx.append_transition(&HuntTransition {
        transition_id: String::new(),
        session_id: to.session_id.clone(),
        generation: to.generation,
        from_state: from,
        to_state: to.state,
        state_version: to.state_version,
        reason: reason.to_owned(),
        actor_type: actor.as_str().to_owned(),
        actor_account_id: account.map(str::to_owned),
        at,
    })
}

pub(crate) fn queue(
    tx: &mut HuntTx<'_>,
    account: &str,
    message_type: &str,
    hunt: &HuntSession,
    payload: Value,
    now: UnixMillis,
    life_ms: i64,
) -> Result<(), RepoError> {
    tx.enqueue_control(&NewControlMessage {
        account_id: account.to_owned(),
        message_type: message_type.to_owned(),
        session_id: Some(hunt.session_id.clone()),
        generation: Some(hunt.generation),
        payload,
        created_at: now,
        expires_at: now.saturating_add(life_ms),
    })?;
    Ok(())
}

pub(crate) fn bump(
    tx: &mut HuntTx<'_>,
    account: &str,
    add: ReliabilityIncrements,
    now: UnixMillis,
) -> Result<(), RepoError> {
    tx.increment_reliability(account, add, now, None)
}

fn clear_live_fields(hunt: &mut HuntSession) {
    hunt.active_party_key = None;
    hunt.lease_expires_at = None;
    hunt.suspect_since = None;
    hunt.suspect_deadline = None;
    hunt.pre_suspect_state = None;
    hunt.offer_expires_at = None;
    hunt.startup_deadline = None;
}

/// Ends one generation (realms' part of the session end). Admission tickets and party activity are the caller's.
#[allow(clippy::too_many_arguments)]
pub fn end_session(
    tx: &mut HuntTx<'_>,
    session_id: &str,
    generation: i64,
    reason: &str,
    allowed: &[HuntState],
    actor: TransitionActor,
    actor_account: Option<&str>,
    fence_bridges: bool,
    now: UnixMillis,
) -> Result<Option<HuntSession>, RepoError> {
    let Some(hunt) = tx.find_hunt(session_id)? else {
        return Ok(None);
    };
    if hunt.generation != generation || !allowed.contains(&hunt.state) {
        return Ok(None);
    }
    let terminal = clean_end_terminal_state(&hunt);
    tx.revoke_generation(session_id, generation, now)?;
    let ended = tx.compare_and_set(session_id, generation, &[hunt.state], hunt.state_version, |h| {
        h.state = terminal;
        h.terminal_at = Some(now);
        h.completion_reason = Some(reason.to_owned());
        if terminal == H::Aborted {
            h.failure_reason = Some(reason.to_owned());
        }
        clear_live_fields(h);
        h.drain_deadline = None;
        h.recovery_retry_at = None;
    })?;
    let Some(ended) = ended else {
        return Ok(None);
    };
    transition(tx, hunt.state, &ended, reason, actor, actor_account, now)?;
    let has_host = !hunt.host_account_id.is_empty();
    if hunt.state == H::HostOffered
        && has_host
        && let Some(offer) = &hunt.host_offer_id
    {
        tx.update_offer(
            offer,
            &hunt.host_account_id,
            OfferStatus::Offered,
            OfferStatus::Cancelled,
            now,
            Some(reason),
        )?;
    }
    if terminal == H::Completed && has_host {
        let add = ReliabilityIncrements {
            clean_completions: 1,
            ..ReliabilityIncrements::default()
        };
        bump(tx, &hunt.host_account_id, add, now)?;
    }
    if hunt.state != H::HostSelecting && fence_bridges {
        for p in &hunt.roster {
            let payload = json!({ "reason": reason, "recoveryAction": "NONE", "cleanEnd": true });
            queue(tx, &p.account_id, "FENCE_NOW", &ended, payload, now, CONTROL_LIFE_MS)?;
        }
    }
    Ok(Some(ended))
}

pub fn mark_suspect(
    tx: &mut HuntTx<'_>,
    hunt: &HuntSession,
    reason: &str,
    suspect_lifetime_ms: i64,
    now: UnixMillis,
) -> Result<Option<HuntSession>, RepoError> {
    let old = hunt.state;
    let changed = tx.compare_and_set(&hunt.session_id, hunt.generation, &[old], hunt.state_version, |h| {
        h.state = H::Suspect;
        h.pre_suspect_state = Some(old);
        h.suspect_since = Some(now);
        h.suspect_deadline = Some(now.saturating_add(suspect_lifetime_ms));
        h.failure_reason = Some(reason.to_owned());
    })?;
    if let Some(changed) = &changed {
        transition(tx, old, changed, reason, TransitionActor::System, None, now)?;
    }
    Ok(changed)
}

/// Appends no transition: the hub tick appends its own.
pub fn recover_from_suspect(tx: &mut HuntTx<'_>, hunt: &HuntSession) -> Result<Option<HuntSession>, RepoError> {
    let Some(back) = hunt.pre_suspect_state else {
        return Ok(None);
    };
    tx.compare_and_set(
        &hunt.session_id,
        hunt.generation,
        &[H::Suspect],
        hunt.state_version,
        |h| {
            h.state = back;
            h.failure_reason = Some("LEASE_RECOVERED".to_owned());
            h.suspect_since = None;
            h.suspect_deadline = None;
            h.pre_suspect_state = None;
        },
    )
}

/// A new generation back in selection, without the host that failed.
pub fn retry_pre_active_host(
    tx: &mut HuntTx<'_>,
    hunt: &HuntSession,
    reason: &str,
    now: UnixMillis,
) -> Result<Option<HuntSession>, RepoError> {
    tx.revoke_generation(&hunt.session_id, hunt.generation, now)?;
    let retried = tx.compare_and_set(
        &hunt.session_id,
        hunt.generation,
        &[hunt.state],
        hunt.state_version,
        |h| {
            h.generation += 1;
            h.state = H::HostSelecting;
            h.host_account_id = String::new();
            h.host_steam_id = String::new();
            h.bridge_instance_id = String::new();
            h.failure_reason = Some(reason.to_owned());
            h.host_offer_id = None;
            h.offer_expires_at = None;
            h.startup_deadline = None;
            h.lobby_id = None;
            h.lease_expires_at = None;
            h.metadata_nonce = None;
        },
    )?;
    let Some(retried) = retried else {
        return Ok(None);
    };
    transition(tx, hunt.state, &retried, reason, TransitionActor::System, None, now)?;
    if !hunt.host_account_id.is_empty() {
        let add = ReliabilityIncrements {
            starts_failed: 1,
            ..ReliabilityIncrements::default()
        };
        bump(tx, &hunt.host_account_id, add, now)?;
        // Fenced for the generation it held.
        queue(
            tx,
            &hunt.host_account_id,
            "FENCE_NOW",
            hunt,
            json!({ "reason": reason }),
            now,
            RETRY_FENCE_LIFE_MS,
        )?;
    }
    Ok(Some(retried))
}

/// HOST_OFFERED back to HOST_SELECTING without the offered host. No transition.
pub fn reset_to_selection(tx: &mut HuntTx<'_>, hunt: &HuntSession) -> Result<Option<HuntSession>, RepoError> {
    tx.compare_and_set(
        &hunt.session_id,
        hunt.generation,
        &[H::HostOffered],
        hunt.state_version,
        |h| {
            h.state = H::HostSelecting;
            h.host_account_id = String::new();
            h.host_steam_id = String::new();
            h.host_offer_id = None;
            h.offer_expires_at = None;
        },
    )
}

pub fn expire_offer(
    tx: &mut HuntTx<'_>,
    hunt: &HuntSession,
    now: UnixMillis,
) -> Result<Option<HuntSession>, RepoError> {
    let Some(offer) = &hunt.host_offer_id else {
        return Ok(None);
    };
    let moved = tx.update_offer(
        offer,
        &hunt.host_account_id,
        OfferStatus::Offered,
        OfferStatus::Expired,
        now,
        Some("TIMEOUT"),
    )?;
    if moved.is_none() {
        return Ok(None);
    }
    let add = ReliabilityIncrements {
        offers_expired: 1,
        ..ReliabilityIncrements::default()
    };
    bump(tx, &hunt.host_account_id, add, now)?;
    let reset = reset_to_selection(tx, hunt)?;
    if let Some(reset) = &reset {
        transition(
            tx,
            hunt.state,
            reset,
            "HOST_OFFER_EXPIRED",
            TransitionActor::System,
            None,
            now,
        )?;
    }
    Ok(reset)
}

/// Where a session that cannot go on is sent. Recovery to a dedicated server or back to Ramsgate is the core's.
pub trait Recovery: Send + Sync {
    fn prepare(
        &self,
        session_id: String,
        generation: i64,
        reason: String,
        before_active: bool,
    ) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// A RECOVERING session whose window ran out, or a due ABORTING one.
    fn advance(&self, session: HuntSession) -> BoxFuture<'_, Result<(), RealmsError>>;
}

/// Recovery on `store` without a dedicated server: an unrecoverable session ends ABORTED.
pub struct RealmsRecovery {
    hunts: HuntRepository,
    core: Arc<dyn IslesCore>,
    clock: Arc<dyn Clock>,
    log: Arc<dyn RealmsLog>,
}

const RECOVERY_ACTOR: Actor = Actor::System("realms-recovery");

enum Prepared {
    Nothing,
    WasAborting(HuntSession),
    Ended(HuntSession),
}

fn object(pairs: &[(&str, &str)]) -> Value {
    let mut map = Map::new();
    for (key, value) in pairs {
        map.insert((*key).to_owned(), Value::String((*value).to_owned()));
    }
    Value::Object(map)
}

impl RealmsRecovery {
    pub fn new(db: RealmsDb, core: Arc<dyn IslesCore>, clock: Arc<dyn Clock>, log: Arc<dyn RealmsLog>) -> Self {
        Self {
            hunts: HuntRepository::new(db),
            core,
            clock,
            log,
        }
    }

    fn cancel_travel(&self, hunt: &HuntSession, reason: &str) {
        if hunt.roster.is_empty() {
            return;
        }
        let accounts: Vec<AccountId> = hunt.roster.iter().map(|p| AccountId(p.account_id.clone())).collect();
        let _ = self.core.cancel_travel(&hunt.hunt_definition_id, &accounts, reason);
    }

    async fn prepare_now(
        &self,
        session_id: String,
        generation: i64,
        reason: String,
        before_active: bool,
    ) -> Result<(), RealmsError> {
        let now = self.clock.now();
        let tx_reason = reason.clone();
        let prepared = self
            .hunts
            .transaction(&RECOVERY_ACTOR, move |tx| -> Result<Prepared, RealmsError> {
                let reason = tx_reason;
                let Some(hunt) = tx.find_hunt(&session_id)? else {
                    return Ok(Prepared::Nothing);
                };
                if hunt.generation != generation
                    || matches!(hunt.state, H::Completed | H::Aborted | H::Recovering | H::Draining)
                {
                    return Ok(Prepared::Nothing);
                }
                if hunt.state == H::Aborting {
                    let ended = end_session(
                        tx,
                        &session_id,
                        generation,
                        "NO_DEDICATED_SERVERS",
                        &[H::Aborting],
                        TransitionActor::System,
                        None,
                        true,
                        now,
                    )?;
                    return Ok(ended.map_or(Prepared::Nothing, Prepared::WasAborting));
                }
                tx.revoke_generation(&session_id, generation, now)?;
                let fenced = tx.compare_and_set(&session_id, generation, &[hunt.state], hunt.state_version, |h| {
                    h.state = H::Fenced;
                    h.failure_reason = Some(reason.clone());
                })?;
                let Some(fenced) = fenced else {
                    return Ok(Prepared::Nothing);
                };
                transition(tx, hunt.state, &fenced, &reason, TransitionActor::System, None, now)?;
                let host = hunt.host_account_id.clone();
                if hunt.state == H::HostOffered
                    && !host.is_empty()
                    && let Some(offer) = &hunt.host_offer_id
                {
                    tx.update_offer(
                        offer,
                        &host,
                        OfferStatus::Offered,
                        OfferStatus::Cancelled,
                        now,
                        Some(&reason),
                    )?;
                }
                let ended = tx.compare_and_set(&session_id, generation, &[H::Fenced], fenced.state_version, |h| {
                    h.state = H::Aborted;
                    h.terminal_at = Some(now);
                    h.failure_reason = Some(reason.clone());
                    clear_live_fields(h);
                    h.recovery_retry_at = None;
                    h.host_offer_id = None;
                })?;
                let Some(ended) = ended else {
                    return Ok(Prepared::Nothing);
                };
                let suffix = if before_active {
                    "_P2P_ONLY"
                } else if hunt.realm_id.is_some() {
                    "_REALM_ONLY"
                } else {
                    "_NO_DEDICATED_FALLBACK"
                };
                let ended_reason = format!("{reason}{suffix}");
                transition(tx, H::Fenced, &ended, &ended_reason, TransitionActor::System, None, now)?;
                let cooldown = Some(now.saturating_add(RECOVERY_COOLDOWN_MS));
                let rostered = |account: &str| hunt.roster.iter().any(|p| p.account_id == account);
                if before_active {
                    let payload = object(&[("reason", &reason), ("recoveryAction", "CANCEL_MATCHMAKING")]);
                    for p in &hunt.roster {
                        queue(
                            tx,
                            &p.account_id,
                            "HOST_OFFER_CANCELLED",
                            &ended,
                            payload.clone(),
                            now,
                            CONTROL_LIFE_MS,
                        )?;
                    }
                    if hunt.realm_id.is_some() && !host.is_empty() && !rostered(&host) {
                        queue(tx, &host, "HOST_OFFER_CANCELLED", &ended, payload, now, CONTROL_LIFE_MS)?;
                    }
                    if !host.is_empty() {
                        let add = ReliabilityIncrements {
                            starts_failed: 1,
                            ..ReliabilityIncrements::default()
                        };
                        tx.increment_reliability(&host, add, now, cooldown)?;
                    }
                } else {
                    let payload = object(&[("reason", &reason), ("recoveryAction", "NONE")]);
                    for p in &hunt.roster {
                        queue(
                            tx,
                            &p.account_id,
                            "FENCE_NOW",
                            &ended,
                            payload.clone(),
                            now,
                            CONTROL_LIFE_MS,
                        )?;
                    }
                    if !host.is_empty() && !rostered(&host) {
                        queue(tx, &host, "FENCE_NOW", &ended, payload, now, CONTROL_LIFE_MS)?;
                    }
                    if !host.is_empty() {
                        let add = ReliabilityIncrements {
                            unexpected_failures: 1,
                            ..ReliabilityIncrements::default()
                        };
                        tx.increment_reliability(&host, add, now, cooldown)?;
                    }
                }
                Ok(Prepared::Ended(ended))
            })
            .await?;
        match prepared {
            Prepared::Nothing => {}
            Prepared::WasAborting(hunt) => {
                self.cancel_travel(&hunt, &reason);
                self.log.warn(&format!(
                    "[P2P] ending {} instead of recovering it: dedicated recovery seam not wired reason={reason}",
                    hunt.session_id
                ));
            }
            Prepared::Ended(hunt) => {
                self.cancel_travel(&hunt, &reason);
                let id = &hunt.session_id;
                let line = if before_active {
                    format!(
                        "[P2P] pre-active failure canceled without dedicated hunt fallback session={id} reason={reason}"
                    )
                } else if hunt.realm_id.is_some() {
                    format!("[P2P] active realm hunt failed; ended without recovery session={id} reason={reason}")
                } else {
                    format!(
                        "[P2P] active hunt failed without dedicated recovery seam; ended session={id} reason={reason}"
                    )
                };
                self.log.warn(&line);
            }
        }
        Ok(())
    }

    async fn advance_now(&self, session: HuntSession) -> Result<(), RealmsError> {
        let now = self.clock.now();
        match session.state {
            H::Recovering => {
                let (id, generation) = (session.session_id.clone(), session.generation);
                let ended = self
                    .hunts
                    .transaction(&RECOVERY_ACTOR, move |tx| -> Result<Option<HuntSession>, RealmsError> {
                        let Some(hunt) = tx.find_hunt(&id)? else {
                            return Ok(None);
                        };
                        if hunt.state != H::Recovering || hunt.generation != generation {
                            return Ok(None);
                        }
                        let ended = tx.compare_and_set(&id, generation, &[H::Recovering], hunt.state_version, |h| {
                            h.state = H::Aborted;
                            h.terminal_at = Some(now);
                            h.failure_reason = Some("RECOVERY_WINDOW_EXPIRED".to_owned());
                            h.active_party_key = None;
                            h.recovery_deadline = None;
                        })?;
                        if let Some(ended) = &ended {
                            transition(
                                tx,
                                H::Recovering,
                                ended,
                                "RECOVERY_WINDOW_EXPIRED",
                                TransitionActor::System,
                                None,
                                now,
                            )?;
                        }
                        Ok(ended)
                    })
                    .await?;
                if ended.is_some() {
                    self.log.warn(&format!(
                        "[P2P] recovery window expired; ended session={}",
                        session.session_id
                    ));
                }
            }
            H::Aborting => {
                let reason = session
                    .failure_reason
                    .clone()
                    .unwrap_or_else(|| "HOST_FAILED".to_owned());
                self.log.warn(&format!(
                    "[P2P] ending {} instead of recovering it: dedicated recovery seam not wired reason={reason}",
                    session.session_id
                ));
                let (id, generation) = (session.session_id.clone(), session.generation);
                self.hunts
                    .transaction(&RECOVERY_ACTOR, move |tx| -> Result<Option<HuntSession>, RealmsError> {
                        Ok(end_session(
                            tx,
                            &id,
                            generation,
                            "NO_DEDICATED_SERVERS",
                            &[H::Aborting],
                            TransitionActor::System,
                            None,
                            true,
                            now,
                        )?)
                    })
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }
}

impl Recovery for RealmsRecovery {
    fn prepare(
        &self,
        session_id: String,
        generation: i64,
        reason: String,
        before_active: bool,
    ) -> BoxFuture<'_, Result<(), RealmsError>> {
        Box::pin(self.prepare_now(session_id, generation, reason, before_active))
    }

    fn advance(&self, session: HuntSession) -> BoxFuture<'_, Result<(), RealmsError>> {
        Box::pin(self.advance_now(session))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_order_and_text() {
        assert!(GuardRole::Client < GuardRole::Host);
        for role in [GuardRole::Client, GuardRole::Host] {
            assert_eq!(GuardRole::parse(role.as_str()), Some(role));
        }
        assert_eq!(GuardRole::parse("HOST"), None);
    }
}
