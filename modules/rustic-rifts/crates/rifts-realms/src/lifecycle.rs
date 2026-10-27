//! The player-hosted session lifecycle (`p2p/controllers/p2pHostLifecycle.ts`): its pure rules, and the steps the
//! lease handler and the monitor take, each one write transaction. Recovery to a dedicated server or back to Ramsgate
//! goes through the DeployServer and the party, which are isles' (RU4): realms hands it to a [`Recovery`] port.

use rifts_seam::{BoxFuture, UnixMillis};
use serde_json::{Value, json};

use crate::RealmsError;
use crate::hunt::{
    AdmissionState, HuntSession, HuntState, HuntTransition, OfferStatus, Participant, ReliabilityIncrements,
    TransitionActor,
};
use crate::repo::hunts::{HuntTx, NewControlMessage};

use HuntState::{
    Aborting, Admitting, Draining, HostOffered, HostReady, HostSelecting, Recovering, Running, Starting, Suspect,
};

/// `P2P_LIFECYCLE_STATES`: what the monitor scans.
pub const LIFECYCLE_STATES: &[HuntState] =
    &[HostSelecting, HostOffered, Starting, HostReady, Admitting, Running, Draining, Suspect, Aborting, Recovering];
/// The host's own end applies in these.
pub const HOST_END_STATES: &[HuntState] = &[HostReady, Admitting, Running, Suspect];
/// The last hub return closes these: the host lease is live.
pub const HUB_RETURN_END_STATES: &[HuntState] = &[HostReady, Admitting, Running];
/// A party's new request may supersede these (RECOVERING closes on its own).
pub const SUPERSEDE_STATES: &[HuntState] =
    &[HostSelecting, HostOffered, Starting, HostReady, Admitting, Running, Suspect, Aborting];
const STARTED_STATES: &[HuntState] = &[HostReady, Admitting, Running, Suspect];
pub const DRAIN_COMPLETION_REASON: &str = "SUPERSEDED_REQUESTER_LEFT_HUNT";
/// The host's first Guard verdict follows its process by a few seconds.
pub const HOST_GUARD_START_GRACE_MS: i64 = 90_000;
/// `CANCELLABLE_FORMING_STATES`.
pub const CANCELLABLE_FORMING_STATES: &[HuntState] = &[HostSelecting, HostOffered, Starting, HostReady, Admitting];

/// Active recovery takes the admitted participants with a published loopback route; pre-active takes everyone.
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

/// A pre-active failure falls back to a dedicated server only while there is a dedicated route.
pub fn should_use_dedicated_pre_active_fallback(before_active: bool, dedicated_hunts_disabled: bool) -> bool {
    before_active && !dedicated_hunts_disabled
}

pub fn is_pre_active_operator_stop_state(state: HuntState) -> bool {
    [HostSelecting, HostOffered, Starting, HostReady].contains(&state)
}

/// A launcher Guard role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GuardRole {
    Client,
    Host,
}

fn guard_role(value: &Value) -> Option<GuardRole> {
    match value.as_str() {
        Some("client") => Some(GuardRole::Client),
        Some("host") => Some(GuardRole::Host),
        _ => None,
    }
}

/// `StringIn`: 1 to `maximum` UTF-16 units, no NUL.
pub(crate) fn string_in(value: Option<&Value>, maximum: usize) -> Option<String> {
    let text = value?.as_str()?;
    let length = text.encode_utf16().count();
    ((1..=maximum).contains(&length) && !text.contains('\0')).then(|| text.to_owned())
}

/// A malformed presence body (`P2P_TICKET_INVALID`) or a refused Guard set (`GUARD_REQUIRED`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardListError {
    pub code: &'static str,
    pub message: &'static str,
}

const SESSIONS_MALFORMED: GuardListError = GuardListError {
    code: "P2P_TICKET_INVALID",
    message: "The Launcher Guard session list is malformed.",
};

/// `ParseGuardPresenceSessions`: the native launcher's independently bound Guard sessions.
pub fn parse_guard_presence_sessions(body: &Value) -> Result<Vec<(GuardRole, String)>, GuardListError> {
    let Some(raw) = body.get("guardSessions").filter(|v| !v.is_null()) else {
        return Ok(string_in(body.get("guardSessionId"), 64)
            .map(|legacy| vec![(GuardRole::Client, legacy)])
            .unwrap_or_default());
    };
    let Some(entries) = raw.as_array().filter(|list| list.len() <= 2) else {
        return Err(SESSIONS_MALFORMED);
    };
    let mut out: Vec<(GuardRole, String)> = Vec::new();
    for entry in entries {
        let role = entry.get("role").and_then(guard_role);
        let id = string_in(entry.get("guardSessionId"), 64)
            .filter(|id| id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'));
        match (role, id) {
            (Some(role), Some(id)) if !out.iter().any(|(r, _)| *r == role) => out.push((role, id)),
            _ => return Err(SESSIONS_MALFORMED),
        }
    }
    Ok(out)
}

/// `ParseGuardRequiredRoles`: the host role stands alone only for a realm host the server confirmed.
pub fn parse_guard_required_roles(
    body: &Value,
    sessions: &[(GuardRole, String)],
    realm_host_only: bool,
) -> Result<Vec<GuardRole>, GuardListError> {
    let Some(raw) = body.get("guardRequiredRoles").filter(|v| !v.is_null()) else {
        return Ok(sessions.iter().map(|(role, _)| *role).collect());
    };
    let malformed = GuardListError {
        code: "P2P_TICKET_INVALID",
        message: "The Launcher Guard role list is malformed.",
    };
    let Some(values) = raw.as_array().filter(|list| list.len() <= 2) else {
        return Err(malformed);
    };
    let mut roles = Vec::new();
    for value in values {
        match guard_role(value) {
            Some(role) if !roles.contains(&role) => roles.push(role),
            _ => return Err(malformed),
        }
    }
    // A stale server session must not stand in for the game client
    if roles.contains(&GuardRole::Host) && !roles.contains(&GuardRole::Client) && !(realm_host_only && roles.len() == 1) {
        return Err(GuardListError {
            code: "GUARD_REQUIRED",
            message: "A hosted process requires both client and host Guard roles.",
        });
    }
    Ok(roles)
}

/// A just-started host whose first host verdict is pending is not recovered within the grace window.
pub fn host_guard_still_starting(
    created_at: UnixMillis,
    required: &[GuardRole],
    verified: &[GuardRole],
    now: UnixMillis,
) -> bool {
    let unverified: Vec<&GuardRole> = required.iter().filter(|role| !verified.contains(role)).collect();
    let age = now.0 - created_at.0;
    !unverified.is_empty()
        && unverified.iter().all(|role| **role == GuardRole::Host)
        && (0..HOST_GUARD_START_GRACE_MS).contains(&age)
}

/// A drained hunt ran too: its players were in it until they left.
fn hunt_ran(hunt: &HuntSession) -> bool {
    hunt.state == Running || hunt.state == Draining || (hunt.state == Suspect && hunt.pre_suspect_state == Some(Running))
}

/// Not back in a hub and admitted, or the host (on its own server until it leaves; a LOST host is gone).
pub fn participants_still_in_hunt(roster: &[Participant], host: Option<&str>) -> Vec<String> {
    roster
        .iter()
        .filter(|p| {
            p.returned_to_hub_at.is_none()
                && (p.admission_state == AdmissionState::Admitted
                    || (Some(p.account_id.as_str()) == host && p.admission_state != AdmissionState::Lost))
        })
        .map(|p| p.account_id.clone())
        .collect()
}

/// COMPLETED for a Tier 0 session that ran; ABORTED otherwise (a Tier 1 COMPLETED is the Guard completion's).
pub fn clean_end_terminal_state(hunt: &HuntSession) -> HuntState {
    if hunt.trust_tier.unwrap_or(0) == 0 && hunt_ran(hunt) {
        HuntState::Completed
    } else {
        HuntState::Aborted
    }
}

/// Why losing this Tier 0 session's host is a clean end: Guard recorded the completion, or nobody is still in it.
pub fn host_loss_is_clean_end(hunt: &HuntSession) -> Option<&'static str> {
    if hunt.trust_tier.unwrap_or(0) != 0 || !STARTED_STATES.contains(&hunt.state) {
        return None;
    }
    if hunt.guard_encounter_state.as_deref() == Some("COMPLETION_REQUESTED") {
        return Some("TIER0_HUNT_COMPLETED");
    }
    (hunt_ran(hunt) && participants_still_in_hunt(&hunt.roster, Some(&hunt.host_account_id)).is_empty())
        .then_some("TIER0_PARTY_LEFT")
}

/// The roster with `account` back in a hub, or `None` when that participant was never in the hunt.
pub fn mark_participant_returned_to_hub(
    roster: &[Participant],
    account: &str,
    at: UnixMillis,
    host: Option<&str>,
) -> Option<(Vec<Participant>, bool, Vec<String>)> {
    let participant = roster.iter().find(|p| p.account_id == account)?;
    if Some(account) != host && ![AdmissionState::Admitted, AdmissionState::Lost].contains(&participant.admission_state)
    {
        return None;
    }
    let changed = participant.returned_to_hub_at.is_none();
    let next: Vec<Participant> = roster
        .iter()
        .map(|p| {
            let mut p = p.clone();
            if p.account_id == account && changed {
                p.returned_to_hub_at = Some(at);
            }
            p
        })
        .collect();
    let still = participants_still_in_hunt(&next, host);
    Some((next, changed, still))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartySessionAction {
    /// The fan-out of a session still forming.
    Reuse,
    Supersede(&'static str),
    /// A running Tier 0 hunt keeps its server for the players still in it.
    Drain(&'static str),
    Conflict(&'static str),
}

/// What a party's new player-hosted request does with the party's current session.
pub fn classify_existing_party_session(
    existing: &HuntSession,
    hunt_id: &str,
    requester: &str,
    now: UnixMillis,
) -> PartySessionAction {
    let action = classify_before_drain(existing, hunt_id, requester, now);
    if let PartySessionAction::Supersede(reason) = action
        && existing.state == Running
        && existing.trust_tier.unwrap_or(0) == 0
        && !existing.is_realm_hub()
        && reason != "HOST_LEASE_EXPIRED"
        && reason != "SESSION_ENDING"
    {
        return PartySessionAction::Drain(reason);
    }
    action
}

fn classify_before_drain(existing: &HuntSession, hunt_id: &str, requester: &str, now: UnixMillis) -> PartySessionAction {
    use PartySessionAction::{Conflict, Drain, Reuse, Supersede};
    if existing.state == Aborting || existing.state == Suspect {
        return Supersede("SESSION_ENDING");
    }
    if matches!(existing.guard_encounter_state.as_deref(), Some("COMPLETION_REQUESTED" | "ABORTED")) {
        return Supersede("HUNT_FINISHED");
    }
    let started = STARTED_STATES.contains(&existing.state);
    if started && existing.lease_expires_at.is_some_and(|at| at <= now) {
        return Supersede("HOST_LEASE_EXPIRED");
    }
    let participant = existing.participant(requester);
    let left = participant.is_some_and(|p| {
        matches!(p.admission_state, AdmissionState::Admitted | AdmissionState::Lost)
            || p.returned_to_hub_at.is_some()
            || (existing.state == Running && requester == existing.host_account_id)
    });
    if started && left {
        if existing.state == Running && existing.trust_tier.unwrap_or(0) == 0 && !existing.is_realm_hub() {
            return Drain("REQUESTER_LEFT_HUNT");
        }
        return Supersede("REQUESTER_LEFT_HUNT");
    }
    if existing.hunt_definition_id != hunt_id {
        return Supersede("PARTY_CHANGED_HUNT");
    }
    if participant.is_none() {
        return if existing.state == Running || existing.state == Admitting {
            Conflict("ROSTER_FROZEN")
        } else {
            Supersede("ROSTER_CHANGED")
        };
    }
    Reuse
}

/// A forming hunt one of its players may still cancel.
pub fn is_cancellable_forming(hunt: &HuntSession) -> bool {
    CANCELLABLE_FORMING_STATES.contains(&hunt.state)
        && !hunt.roster.iter().any(|p| p.admission_state == AdmissionState::Admitted)
}

/// Where a recovery sends the party: a pre-active failure to the same hunt on a dedicated server (Training to the
/// Training hub), an active one back to Ramsgate.
pub fn recovery_destination(before_active: bool, hunt_definition_id: &str) -> (&'static str, String, bool) {
    if !before_active {
        return ("CITY", "ShatteredIsles_ReturnToRamsgate".to_owned(), true);
    }
    if crate::tiers::is_training_hunt_id(hunt_definition_id) {
        return ("SHARED", hunt_definition_id.to_owned(), true);
    }
    ("ISLAND", hunt_definition_id.to_owned(), false)
}

// ------------------------------------------------------------------ the steps

/// What isles does when realms gives a session up: `PrepareRecovery` and the recovery's own lifecycle (the
/// DeployServer, the party, the travel), which live outside rifts until RU4.
pub trait Recovery: Send + Sync {
    fn prepare(
        &self,
        session_id: String,
        generation: i64,
        reason: String,
        before_active: bool,
    ) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `FinalizeRecovery` (a RECOVERING session's window ran out) and `ResumeRecovery` (an ABORTING one is due).
    fn advance(&self, session: HuntSession) -> BoxFuture<'_, Result<(), RealmsError>>;
}

/// A recovery that records what it was asked, for tests and until isles wires its own.
#[derive(Debug, Default)]
pub struct RecordedRecovery(pub std::sync::Mutex<Vec<(String, i64, String, bool)>>);

impl Recovery for RecordedRecovery {
    fn prepare(
        &self,
        session_id: String,
        generation: i64,
        reason: String,
        before_active: bool,
    ) -> BoxFuture<'_, Result<(), RealmsError>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((session_id, generation, reason, before_active));
        Box::pin(async { Ok(()) })
    }

    fn advance(&self, _: HuntSession) -> BoxFuture<'_, Result<(), RealmsError>> {
        Box::pin(async { Ok(()) })
    }
}

fn transition(hunt: &HuntSession, to: &HuntSession, reason: &str, actor: TransitionActor, account: Option<&str>, at: UnixMillis) -> HuntTransition {
    HuntTransition {
        transition_id: String::new(),
        session_id: hunt.session_id.clone(),
        generation: to.generation,
        from_state: hunt.state,
        to_state: to.state,
        state_version: to.state_version,
        reason: reason.to_owned(),
        actor_type: actor.as_str().to_owned(),
        actor_account_id: account.map(str::to_owned),
        at,
    }
}

fn fence(account: &str, hunt: &HuntSession, generation: i64, payload: Value, now: UnixMillis, life_ms: i64) -> NewControlMessage {
    NewControlMessage {
        account_id: account.to_owned(),
        message_type: "FENCE_NOW".to_owned(),
        session_id: Some(hunt.session_id.clone()),
        generation: Some(generation),
        payload,
        created_at: now,
        expires_at: UnixMillis(now.0 + life_ms),
    }
}

/// `EndP2PHuntSession`, realms' part: revoke the generation, close the session (`clean_end_terminal_state`), give its
/// party key up, cancel an open offer, credit a clean completion and fence every bridge. Admission tickets (RF4) and
/// the party's activity (isles) are the caller's. `None` when the session had moved on.
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
) -> Result<Option<HuntSession>, RealmsError> {
    let Some(hunt) = tx.find_hunt(session_id)? else {
        return Ok(None);
    };
    if hunt.generation != generation || !allowed.contains(&hunt.state) {
        return Ok(None);
    }
    let terminal = clean_end_terminal_state(&hunt);
    tx.revoke_generation(session_id, generation, now)?;
    let reason_text = reason.to_owned();
    let Some(ended) = tx.compare_and_set(session_id, generation, &[hunt.state], hunt.state_version, move |h| {
        h.state = terminal;
        h.terminal_at = Some(now);
        h.completion_reason = Some(reason_text.clone());
        if terminal == HuntState::Aborted {
            h.failure_reason = Some(reason_text);
        }
        h.active_party_key = None;
        h.lease_expires_at = None;
        h.suspect_since = None;
        h.suspect_deadline = None;
        h.pre_suspect_state = None;
        h.offer_expires_at = None;
        h.startup_deadline = None;
        h.drain_deadline = None;
        h.recovery_retry_at = None;
    })?
    else {
        return Ok(None);
    };
    tx.append_transition(&transition(&hunt, &ended, reason, actor, actor_account, now))?;
    if hunt.state == HostOffered
        && let Some(offer) = &hunt.host_offer_id
        && !hunt.host_account_id.is_empty()
    {
        tx.update_offer(offer, &hunt.host_account_id, OfferStatus::Offered, OfferStatus::Cancelled, now, Some(reason))?;
    }
    if terminal == HuntState::Completed && !hunt.host_account_id.is_empty() {
        let add = ReliabilityIncrements {
            clean_completions: 1,
            ..ReliabilityIncrements::default()
        };
        tx.increment_reliability(&hunt.host_account_id, add, now, None)?;
    }
    // Nothing runs while a host is still being selected
    if hunt.state != HostSelecting && fence_bridges {
        let payload = json!({ "reason": reason, "recoveryAction": "NONE", "cleanEnd": true });
        for p in &hunt.roster {
            tx.enqueue_control(&fence(&p.account_id, &hunt, generation, payload.clone(), now, 10 * 60_000))?;
        }
    }
    Ok(Some(ended))
}

/// `MarkSuspect`: the lease lapsed or the participants report the host gone.
pub fn mark_suspect(
    tx: &mut HuntTx<'_>,
    hunt: &HuntSession,
    reason: &str,
    suspect_lifetime_ms: i64,
    now: UnixMillis,
) -> Result<Option<HuntSession>, RealmsError> {
    let (from, reason_text) = (hunt.state, reason.to_owned());
    let suspect = tx.compare_and_set(&hunt.session_id, hunt.generation, &[hunt.state], hunt.state_version, move |h| {
        h.state = Suspect;
        h.pre_suspect_state = Some(from);
        h.suspect_since = Some(now);
        h.suspect_deadline = Some(UnixMillis(now.0 + suspect_lifetime_ms));
        h.failure_reason = Some(reason_text);
    })?;
    if let Some(next) = &suspect {
        tx.append_transition(&transition(hunt, next, reason, TransitionActor::System, None, now))?;
    }
    Ok(suspect)
}

/// The lease came back during the suspect window: back to the state before.
pub fn recover_from_suspect(tx: &mut HuntTx<'_>, hunt: &HuntSession) -> Result<Option<HuntSession>, RealmsError> {
    let Some(before) = hunt.pre_suspect_state else {
        return Ok(None);
    };
    Ok(tx.compare_and_set(&hunt.session_id, hunt.generation, &[Suspect], hunt.state_version, move |h| {
        h.state = before;
        h.failure_reason = Some("LEASE_RECOVERED".to_owned());
        h.suspect_since = None;
        h.suspect_deadline = None;
        h.pre_suspect_state = None;
    })?)
}

/// `RetryPreActiveHost`: the host never came up; a new generation goes back to host selection and the old host is
/// fenced and charged a failed start.
pub fn retry_pre_active_host(tx: &mut HuntTx<'_>, hunt: &HuntSession, reason: &str, now: UnixMillis) -> Result<Option<HuntSession>, RealmsError> {
    tx.revoke_generation(&hunt.session_id, hunt.generation, now)?;
    let reason_text = reason.to_owned();
    let Some(next) = tx.compare_and_set(&hunt.session_id, hunt.generation, &[hunt.state], hunt.state_version, move |h| {
        h.generation += 1;
        h.state = HostSelecting;
        h.host_account_id = String::new();
        h.host_steam_id = String::new();
        h.bridge_instance_id = String::new();
        h.failure_reason = Some(reason_text);
        h.host_offer_id = None;
        h.offer_expires_at = None;
        h.startup_deadline = None;
        h.lobby_id = None;
        h.lease_expires_at = None;
        h.metadata_nonce = None;
    })?
    else {
        return Ok(None);
    };
    tx.append_transition(&transition(hunt, &next, reason, TransitionActor::System, None, now))?;
    if !hunt.host_account_id.is_empty() {
        let add = ReliabilityIncrements {
            starts_failed: 1,
            ..ReliabilityIncrements::default()
        };
        tx.increment_reliability(&hunt.host_account_id, add, now, None)?;
        tx.enqueue_control(&fence(&hunt.host_account_id, hunt, hunt.generation, json!({ "reason": reason }), now, 5 * 60_000))?;
    }
    Ok(Some(next))
}

/// `ExpireOffer`: the offered host never answered; back to host selection.
pub fn expire_offer(tx: &mut HuntTx<'_>, hunt: &HuntSession, now: UnixMillis) -> Result<Option<HuntSession>, RealmsError> {
    let Some(offer) = &hunt.host_offer_id else {
        return Ok(None);
    };
    let expired =
        tx.update_offer(offer, &hunt.host_account_id, OfferStatus::Offered, OfferStatus::Expired, now, Some("TIMEOUT"))?;
    if expired.is_none() {
        return Ok(None);
    }
    let add = ReliabilityIncrements {
        offers_expired: 1,
        ..ReliabilityIncrements::default()
    };
    tx.increment_reliability(&hunt.host_account_id, add, now, None)?;
    let reset = reset_to_selection(tx, hunt)?;
    if let Some(next) = &reset {
        tx.append_transition(&transition(hunt, next, "HOST_OFFER_EXPIRED", TransitionActor::System, None, now))?;
    }
    Ok(reset)
}

/// HOST_OFFERED -> HOST_SELECTING with the offered host cleared.
pub fn reset_to_selection(tx: &mut HuntTx<'_>, hunt: &HuntSession) -> Result<Option<HuntSession>, RealmsError> {
    Ok(tx.compare_and_set(&hunt.session_id, hunt.generation, &[HostOffered], hunt.state_version, |h| {
        h.state = HostSelecting;
        h.host_account_id = String::new();
        h.host_steam_id = String::new();
        h.host_offer_id = None;
        h.offer_expires_at = None;
    })?)
}
