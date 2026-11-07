// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The hosted hunt's lifecycle: the pure rules, the steps that each run inside one hunt transaction, and recovery
//! when a session cannot go on.

use std::sync::Arc;

use rustic_rifts_seam::{AccountId, Actor, BoxFuture, IslesCore, UnixMillis};
use serde_json::{Map, Value, json};

use crate::RealmsError;
use crate::db::{RealmsDb, RepoError};
use crate::hunt::OfferStatus::{Cancelled, Expired, Offered};
use crate::hunt::{AdmissionState, HuntSession, HuntState, HuntTransition, Participant};
use crate::hunt::{ReliabilityIncrements, TransitionActor};
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
pub const HOST_END_STATES: [HuntState; 4] = STARTED_STATES;
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
/// The host's server is up: it reported ready, whatever came after.
const STARTED_STATES: [HuntState; 4] = [H::HostReady, H::Admitting, H::Running, H::Suspect];
/// States in which a party's forming hunt may still be cancelled.
pub const CANCELLABLE_FORMING_STATES: [HuntState; 5] = [
    H::HostSelecting,
    H::HostOffered,
    H::Starting,
    H::HostReady,
    H::Admitting,
];

/// Why a drained hunt completed when it recorded no reason of its own.
pub const DRAIN_COMPLETION_REASON: &str = "SUPERSEDED_REQUESTER_LEFT_HUNT";
/// How long after a hunt is made its host's Guard session may still be starting, in milliseconds.
pub const HOST_GUARD_START_GRACE_MS: i64 = 90_000;

const MINUTE_MS: i64 = 60_000;
pub(crate) const CONTROL_LIFE_MS: i64 = 10 * MINUTE_MS;
const RETRY_FENCE_LIFE_MS: i64 = 5 * MINUTE_MS;
const RECOVERY_COOLDOWN_MS: i64 = 5 * MINUTE_MS;

const fn tier_zero(hunt: &HuntSession) -> bool {
    matches!(hunt.trust_tier, None | Some(0))
}

/// Whether the hunt was running, before suspicion if it is suspect; a draining hunt ran.
pub fn hunt_ran(hunt: &HuntSession) -> bool {
    match hunt.state {
        H::Running | H::Draining => true,
        H::Suspect => hunt.pre_suspect_state == Some(H::Running),
        _ => false,
    }
}

/// Before active the whole roster; once active, the admitted participants whose local bridge came up.
pub fn select_recovery_participants(roster: &[Participant], before_active: bool) -> Vec<Participant> {
    let bridged = |p: &&Participant| {
        p.admission_state == AdmissionState::Admitted
            && p.local_bridge_ready_at.is_some()
            && p.local_bridge_port.is_some()
    };
    roster.iter().filter(|p| before_active || bridged(p)).cloned().collect()
}

/// Whether a hunt that failed before going live falls back to a dedicated server.
pub fn should_use_dedicated_pre_active_fallback(before_active: bool, dedicated_hunts_disabled: bool) -> bool {
    !dedicated_hunts_disabled && before_active
}

/// Whether an operator stop in this state comes before the hunt went live.
pub fn is_pre_active_operator_stop_state(state: HuntState) -> bool {
    [H::HostSelecting, H::HostOffered, H::Starting, H::HostReady].contains(&state)
}

/// A Launcher Guard session's role; a host's covers a client's.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum GuardRole {
    /// The player's game client.
    Client,
    /// The hosted server; covers the client's role too.
    Host,
}

const GUARD_ROLE_TEXT: [(GuardRole, &str); 2] = [(GuardRole::Client, "client"), (GuardRole::Host, "host")];

impl GuardRole {
    /// The role's wire name.
    pub const fn as_str(self) -> &'static str {
        GUARD_ROLE_TEXT[self as usize].1
    }

    /// The role for its wire name.
    pub fn parse(text: &str) -> Option<Self> {
        GUARD_ROLE_TEXT
            .iter()
            .find(|(_, name)| *name == text)
            .map(|(role, _)| *role)
    }
}

/// Why a Launcher Guard list in a ticket request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuardListError {
    /// The error code.
    pub code: &'static str,
    /// The text for the caller.
    pub message: &'static str,
}

const fn guard_list_error(code: &'static str, message: &'static str) -> GuardListError {
    GuardListError { code, message }
}

const SESSIONS_MALFORMED: GuardListError =
    guard_list_error("P2P_TICKET_INVALID", "The Launcher Guard session list is malformed.");
const ROLES_MALFORMED: GuardListError =
    guard_list_error("P2P_TICKET_INVALID", "The Launcher Guard role list is malformed.");
const BOTH_ROLES_REQUIRED: GuardListError = guard_list_error(
    "GUARD_REQUIRED",
    "A hosted process requires both client and host Guard roles.",
);

/// 1 to 64 UTF-16 units without NUL.
fn guard_id_text(value: &Value) -> Option<&str> {
    let text = value.as_str()?;
    let fits = !text.contains('\0') && (1..=64).contains(&text.encode_utf16().count());
    fits.then_some(text)
}

/// `None` and `null` are absent; anything but an array of at most two entries is malformed.
fn short_list(value: Option<&Value>) -> Result<Option<&Vec<Value>>, ()> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(entries)) if entries.len() < 3 => Ok(Some(entries)),
        Some(_) => Err(()),
    }
}

fn session_entry(entry: &Value) -> Option<(GuardRole, String)> {
    let role = GuardRole::parse(entry.get("role")?.as_str()?)?;
    let id = guard_id_text(entry.get("guardSessionId")?)?;
    let uuid_shaped = id.len() == 36 && id.chars().all(|c| c == '-' || c.is_ascii_hexdigit());
    uuid_shaped.then(|| (role, id.to_owned()))
}

/// `guardSessions`, or the older single `guardSessionId` as a client session.
pub fn parse_guard_presence_sessions(body: &Value) -> Result<Vec<(GuardRole, String)>, GuardListError> {
    let Some(entries) = short_list(body.get("guardSessions")).map_err(|()| SESSIONS_MALFORMED)? else {
        let legacy = body.get("guardSessionId").and_then(guard_id_text);
        return Ok(legacy
            .map(|id| (GuardRole::Client, id.to_owned()))
            .into_iter()
            .collect());
    };
    let mut sessions = Vec::with_capacity(entries.len());
    for entry in entries {
        // One session per role.
        match session_entry(entry) {
            Some((role, id)) if sessions.iter().all(|(seen, _)| *seen != role) => sessions.push((role, id)),
            _ => return Err(SESSIONS_MALFORMED),
        }
    }
    Ok(sessions)
}

/// A stale server session must not stand in for the game client: `host` needs `client`, unless only a realm host
/// is asking.
pub fn parse_guard_required_roles(
    body: &Value,
    sessions: &[(GuardRole, String)],
    realm_host_only: bool,
) -> Result<Vec<GuardRole>, GuardListError> {
    let roles: Vec<GuardRole> = match short_list(body.get("guardRequiredRoles")).map_err(|()| ROLES_MALFORMED)? {
        None => sessions.iter().map(|&(role, _)| role).collect(),
        Some(entries) => {
            let mut listed = Vec::with_capacity(entries.len());
            for entry in entries {
                match entry.as_str().and_then(GuardRole::parse) {
                    Some(role) if !listed.contains(&role) => listed.push(role),
                    _ => return Err(ROLES_MALFORMED),
                }
            }
            listed
        }
    };
    let host_without_client = roles.contains(&GuardRole::Host) && !roles.contains(&GuardRole::Client);
    let realm_exception = realm_host_only && roles == [GuardRole::Host];
    if host_without_client && !realm_exception {
        Err(BOTH_ROLES_REQUIRED)
    } else {
        Ok(roles)
    }
}

/// The host role's Guard session is allowed a grace while the hosted server starts.
pub fn host_guard_still_starting(
    created_at: UnixMillis,
    required: &[GuardRole],
    verified: &[GuardRole],
    now: UnixMillis,
) -> bool {
    let mut missing = required.iter().filter(|role| !verified.contains(role)).peekable();
    let within_grace = (0..HOST_GUARD_START_GRACE_MS).contains(&(now.0 - created_at.0));
    missing.peek().is_some() && missing.all(|role| *role == GuardRole::Host) && within_grace
}

fn still_in_hunt(p: &Participant, host: &str) -> bool {
    if p.returned_to_hub_at.is_some() {
        return false;
    }
    match p.admission_state {
        AdmissionState::Admitted => true,
        AdmissionState::Lost => false,
        _ => p.account_id == host,
    }
}

/// In roster order: admitted and not back in the hub, or the host while it has not lost its link.
pub fn participants_still_in_hunt(roster: &[Participant], host: &str) -> Vec<String> {
    roster
        .iter()
        .filter(|p| still_in_hunt(p, host))
        .map(|p| p.account_id.clone())
        .collect()
}

/// A tier-1 COMPLETED is Guard's completion, never a clean end's.
pub fn clean_end_terminal_state(hunt: &HuntSession) -> HuntState {
    match tier_zero(hunt) && hunt_ran(hunt) {
        true => H::Completed,
        false => H::Aborted,
    }
}

/// For a tier-0 hunt whose host left: the clean end's reason when the hunt completed or every player left, else `None`.
pub fn host_loss_is_clean_end(hunt: &HuntSession) -> Option<&'static str> {
    if !(tier_zero(hunt) && STARTED_STATES.contains(&hunt.state)) {
        return None;
    }
    if hunt.guard_encounter_state.as_deref() == Some("COMPLETION_REQUESTED") {
        Some("TIER0_HUNT_COMPLETED")
    } else if hunt_ran(hunt) && participants_still_in_hunt(&hunt.roster, &hunt.host_account_id).is_empty() {
        Some("TIER0_PARTY_LEFT")
    } else {
        None
    }
}

/// The new roster, whether this was the first return, and who is still in the hunt.
pub fn mark_participant_returned_to_hub(
    roster: &[Participant],
    account: &str,
    at: UnixMillis,
    host: &str,
) -> Option<(Vec<Participant>, bool, Vec<String>)> {
    let position = roster.iter().position(|p| p.account_id == account)?;
    let was_in = matches!(
        roster[position].admission_state,
        AdmissionState::Admitted | AdmissionState::Lost
    );
    if account != host && !was_in {
        return None;
    }
    let mut updated = roster.to_vec();
    let first = updated[position].returned_to_hub_at.is_none();
    // A row already back keeps its time.
    for p in updated.iter_mut().filter(|p| p.account_id == account) {
        p.returned_to_hub_at.get_or_insert(at);
    }
    let remaining = participants_still_in_hunt(&updated, host);
    Some((updated, first, remaining))
}

/// What to do with a party's existing session when it asks for a hunt again.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum PartySessionAction {
    /// Join it again.
    Reuse,
    /// End it and make a new one, for this reason.
    Supersede(&'static str),
    /// Let it drain and make a new one, for this reason.
    Drain(&'static str),
    /// Refuse the request, for this reason.
    Conflict(&'static str),
}

/// The requester played and is back, or hosts the running hunt.
fn requester_left(existing: &HuntSession, requester: &str) -> bool {
    let played = existing.participant(requester).is_some_and(|p| {
        p.returned_to_hub_at.is_some() || matches!(p.admission_state, AdmissionState::Admitted | AdmissionState::Lost)
    });
    played || (existing.host_account_id == requester && existing.state == H::Running)
}

/// A running tier-0 hunt that is not a hub drains rather than ends.
fn drains_instead(existing: &HuntSession) -> bool {
    existing.state == H::Running && tier_zero(existing) && !existing.is_realm_hub()
}

fn classify_raw(existing: &HuntSession, hunt_id: &str, requester: &str, now: UnixMillis) -> PartySessionAction {
    use PartySessionAction::{Conflict, Drain, Reuse, Supersede};
    let state = existing.state;
    let started = STARTED_STATES.contains(&state);
    let finished = matches!(
        existing.guard_encounter_state.as_deref(),
        Some("COMPLETION_REQUESTED" | "ABORTED")
    );
    let lease_gone = existing.lease_expires_at.is_some_and(|at| at <= now);
    if state == H::Aborting || state == H::Suspect {
        Supersede("SESSION_ENDING")
    } else if finished {
        Supersede("HUNT_FINISHED")
    } else if started && lease_gone {
        Supersede("HOST_LEASE_EXPIRED")
    } else if started && requester_left(existing, requester) {
        if drains_instead(existing) {
            Drain("REQUESTER_LEFT_HUNT")
        } else {
            Supersede("REQUESTER_LEFT_HUNT")
        }
    } else if existing.hunt_definition_id != hunt_id {
        Supersede("PARTY_CHANGED_HUNT")
    } else if existing.participant(requester).is_some() {
        Reuse
    } else if state == H::Running || state == H::Admitting {
        Conflict("ROSTER_FROZEN")
    } else {
        Supersede("ROSTER_CHANGED")
    }
}

/// A running tier-0 hunt keeps serving its players: superseding it drains it instead, unless its host is gone.
pub fn classify_existing_party_session(
    existing: &HuntSession,
    hunt_id: &str,
    requester: &str,
    now: UnixMillis,
) -> PartySessionAction {
    let action = classify_raw(existing, hunt_id, requester, now);
    let PartySessionAction::Supersede(reason) = action else {
        return action;
    };
    let host_gone = reason == "HOST_LEASE_EXPIRED" || reason == "SESSION_ENDING";
    if drains_instead(existing) && !host_gone {
        PartySessionAction::Drain(reason)
    } else {
        action
    }
}

/// Whether the hunt is still forming with nobody admitted, so it may be cancelled.
pub fn is_cancellable_forming(hunt: &HuntSession) -> bool {
    let anyone_admitted = hunt
        .roster
        .iter()
        .any(|p| p.admission_state == AdmissionState::Admitted);
    !anyone_admitted && CANCELLABLE_FORMING_STATES.contains(&hunt.state)
}

/// (game mode, hunt id, whether it is a hub) the party goes back to.
pub fn recovery_destination(before_active: bool, hunt_id: &str) -> (&'static str, String, bool) {
    match (before_active, crate::tiers::is_training_hunt_id(hunt_id)) {
        (false, _) => ("CITY", String::from("ShatteredIsles_ReturnToRamsgate"), true),
        (true, true) => ("SHARED", String::from(hunt_id), true),
        (true, false) => ("ISLAND", String::from(hunt_id), false),
    }
}

pub(crate) fn transition(
    tx: &mut HuntTx<'_>,
    before: HuntState,
    after: &HuntSession,
    why: &str,
    by: TransitionActor,
    by_account: Option<&str>,
    at: UnixMillis,
) -> Result<(), RepoError> {
    let record = HuntTransition {
        // The repository assigns the id.
        transition_id: String::new(),
        from_state: before,
        to_state: after.state,
        session_id: after.session_id.clone(),
        generation: after.generation,
        state_version: after.state_version,
        actor_type: by.as_str().into(),
        actor_account_id: by_account.map(String::from),
        reason: why.into(),
        at,
    };
    tx.append_transition(&record)
}

/// A system transition, the common case.
fn system_transition(
    tx: &mut HuntTx<'_>,
    step: (HuntState, &HuntSession),
    why: &str,
    at: UnixMillis,
) -> Result<(), RepoError> {
    transition(tx, step.0, step.1, why, TransitionActor::System, None, at)
}

pub(crate) fn queue(
    tx: &mut HuntTx<'_>,
    recipient: &str,
    kind: &str,
    hunt: &HuntSession,
    payload: Value,
    sent_at: UnixMillis,
    ttl_ms: i64,
) -> Result<(), RepoError> {
    let message = NewControlMessage {
        session_id: Some(hunt.session_id.clone()),
        generation: Some(hunt.generation),
        account_id: recipient.into(),
        message_type: kind.into(),
        created_at: sent_at,
        expires_at: sent_at.saturating_add(ttl_ms),
        payload,
    };
    tx.enqueue_control(&message).map(drop)
}

/// A reliability change with no cooldown.
pub(crate) fn bump(
    tx: &mut HuntTx<'_>,
    host: &str,
    add: ReliabilityIncrements,
    at: UnixMillis,
) -> Result<(), RepoError> {
    tx.increment_reliability(host, add, at, None)
}

/// One counter of a host's reliability record.
pub(crate) fn one(counter: fn(&mut ReliabilityIncrements) -> &mut i64) -> ReliabilityIncrements {
    let mut add = ReliabilityIncrements::default();
    *counter(&mut add) = 1;
    add
}

/// The session's current generation, state and version must still hold for `change` to apply.
fn advance(
    tx: &mut HuntTx<'_>,
    hunt: &HuntSession,
    from: &[HuntState],
    change: impl FnOnce(&mut HuntSession),
) -> Result<Option<HuntSession>, RepoError> {
    tx.compare_and_set(&hunt.session_id, hunt.generation, from, hunt.state_version, change)
}

/// The deadlines and keys that only mean something while a generation is live.
pub(crate) fn clear_live_fields(hunt: &mut HuntSession) {
    (hunt.lease_expires_at, hunt.offer_expires_at, hunt.startup_deadline) = (None, None, None);
    (hunt.suspect_since, hunt.suspect_deadline, hunt.pre_suspect_state) = (None, None, None);
    hunt.active_party_key = None;
}

/// Forgets the offered host and its offer.
fn clear_offered_host(hunt: &mut HuntSession) {
    hunt.host_account_id.clear();
    hunt.host_steam_id.clear();
    (hunt.host_offer_id, hunt.offer_expires_at) = (None, None);
}

fn cancel_open_offer(tx: &mut HuntTx<'_>, hunt: &HuntSession, reason: &str, now: UnixMillis) -> Result<(), RepoError> {
    let Some(offer) = hunt.host_offer_id.as_deref() else {
        return Ok(());
    };
    if hunt.state == H::HostOffered && !hunt.host_account_id.is_empty() {
        let host = hunt.host_account_id.as_str();
        tx.update_offer(offer, host, Offered, Cancelled, now, Some(reason))?;
    }
    Ok(())
}

/// Ends one generation (realms' part of the session end). Admission tickets and party activity are the caller's.
#[allow(clippy::too_many_arguments, reason = "each argument is one part of the transition")]
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
    let found = tx.find_hunt(session_id)?;
    let Some(hunt) = found.filter(|h| h.generation == generation && allowed.contains(&h.state)) else {
        return Ok(None);
    };
    let terminal = clean_end_terminal_state(&hunt);
    tx.revoke_generation(session_id, generation, now)?;
    let finish = |h: &mut HuntSession| {
        clear_live_fields(h);
        (h.drain_deadline, h.recovery_retry_at) = (None, None);
        (h.state, h.terminal_at) = (terminal, Some(now));
        if terminal == H::Aborted {
            h.failure_reason = Some(reason.into());
        }
        h.completion_reason = Some(reason.into());
    };
    let Some(ended) = advance(tx, &hunt, &[hunt.state], finish)? else {
        return Ok(None);
    };
    transition(tx, hunt.state, &ended, reason, actor, actor_account, now)?;
    cancel_open_offer(tx, &hunt, reason, now)?;
    let has_host = !hunt.host_account_id.is_empty();
    if has_host && terminal == H::Completed {
        bump(tx, &hunt.host_account_id, one(|r| &mut r.clean_completions), now)?;
    }
    // A session still choosing its host has no bridges to fence.
    if fence_bridges && hunt.state != H::HostSelecting {
        let fence = json!({ "reason": reason, "recoveryAction": "NONE", "cleanEnd": true });
        for p in &hunt.roster {
            queue(
                tx,
                &p.account_id,
                "FENCE_NOW",
                &ended,
                fence.clone(),
                now,
                CONTROL_LIFE_MS,
            )?;
        }
    }
    Ok(Some(ended))
}

/// Marks the hunt suspect for `reason` and records the transition; `None` when its state moved on.
pub fn mark_suspect(
    tx: &mut HuntTx<'_>,
    hunt: &HuntSession,
    reason: &str,
    suspect_lifetime_ms: i64,
    now: UnixMillis,
) -> Result<Option<HuntSession>, RepoError> {
    let before = hunt.state;
    let suspect = |h: &mut HuntSession| {
        (h.state, h.pre_suspect_state) = (H::Suspect, Some(before));
        (h.suspect_since, h.suspect_deadline) = (Some(now), Some(now.saturating_add(suspect_lifetime_ms)));
        h.failure_reason = Some(reason.to_owned());
    };
    let changed = advance(tx, hunt, &[before], suspect)?;
    if let Some(after) = changed.as_ref() {
        system_transition(tx, (before, after), reason, now)?;
    }
    Ok(changed)
}

/// Appends no transition: the hub tick appends its own.
pub fn recover_from_suspect(tx: &mut HuntTx<'_>, hunt: &HuntSession) -> Result<Option<HuntSession>, RepoError> {
    let Some(back) = hunt.pre_suspect_state else {
        return Ok(None);
    };
    advance(tx, hunt, &[H::Suspect], |h| {
        (h.suspect_since, h.suspect_deadline, h.pre_suspect_state) = (None, None, None);
        h.failure_reason = Some(String::from("LEASE_RECOVERED"));
        h.state = back;
    })
}

/// A new generation back in selection, without the host that failed.
pub fn retry_pre_active_host(
    tx: &mut HuntTx<'_>,
    hunt: &HuntSession,
    reason: &str,
    now: UnixMillis,
) -> Result<Option<HuntSession>, RepoError> {
    tx.revoke_generation(&hunt.session_id, hunt.generation, now)?;
    let next_generation = |h: &mut HuntSession| {
        clear_offered_host(h);
        h.bridge_instance_id.clear();
        (h.startup_deadline, h.lease_expires_at) = (None, None);
        (h.lobby_id, h.metadata_nonce) = (None, None);
        h.failure_reason = Some(reason.to_owned());
        h.state = H::HostSelecting;
        h.generation += 1;
    };
    let Some(retried) = advance(tx, hunt, &[hunt.state], next_generation)? else {
        return Ok(None);
    };
    system_transition(tx, (hunt.state, &retried), reason, now)?;
    let failed_host = hunt.host_account_id.as_str();
    if !failed_host.is_empty() {
        bump(tx, failed_host, one(|r| &mut r.starts_failed), now)?;
        // Fenced for the generation it held.
        let payload = json!({ "reason": reason });
        queue(tx, failed_host, "FENCE_NOW", hunt, payload, now, RETRY_FENCE_LIFE_MS)?;
    }
    Ok(Some(retried))
}

/// HOST_OFFERED back to HOST_SELECTING without the offered host. No transition.
pub fn reset_to_selection(tx: &mut HuntTx<'_>, hunt: &HuntSession) -> Result<Option<HuntSession>, RepoError> {
    advance(tx, hunt, &[H::HostOffered], |h| {
        clear_offered_host(h);
        h.state = H::HostSelecting;
    })
}

/// Expires the host offer and goes back to host selection; `None` when there was no offer to expire.
pub fn expire_offer(
    tx: &mut HuntTx<'_>,
    hunt: &HuntSession,
    now: UnixMillis,
) -> Result<Option<HuntSession>, RepoError> {
    let Some(offer) = hunt.host_offer_id.as_deref() else {
        return Ok(None);
    };
    let host = hunt.host_account_id.as_str();
    let expired = tx.update_offer(offer, host, Offered, Expired, now, Some("TIMEOUT"))?;
    if expired.is_none() {
        return Ok(None);
    }
    bump(tx, host, one(|r| &mut r.offers_expired), now)?;
    let reset = reset_to_selection(tx, hunt)?;
    if let Some(back) = reset.as_ref() {
        system_transition(tx, (hunt.state, back), "HOST_OFFER_EXPIRED", now)?;
    }
    Ok(reset)
}

/// Where a session that cannot go on is sent. Recovery to a dedicated server or back to Ramsgate is the core's.
pub trait Recovery: Send + Sync {
    /// Sends the session to recovery: a dedicated server, or back to Ramsgate.
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
    log: Arc<dyn RealmsLog>,
    clock: Arc<dyn Clock>,
    core: Arc<dyn IslesCore>,
    hunts: HuntRepository,
}

const RECOVERY_ACTOR: Actor = Actor::System("realms-recovery");
/// Without a dedicated server an ABORTING session simply ends.
const NO_DEDICATED: &str = "NO_DEDICATED_SERVERS";

enum Prepared {
    Nothing,
    WasAborting(HuntSession),
    Ended(HuntSession),
}

/// A control payload of string fields, in order.
fn strings(pairs: &[(&str, &str)]) -> Value {
    let fields: Map<String, Value> = pairs.iter().map(|&(k, v)| (k.to_owned(), Value::from(v))).collect();
    Value::Object(fields)
}

/// A generation: the session's id and number.
type Key<'a> = (&'a str, i64);

fn end_aborting(
    tx: &mut HuntTx<'_>,
    (id, generation): Key<'_>,
    now: UnixMillis,
) -> Result<Option<HuntSession>, RepoError> {
    let (aborting, system) = ([H::Aborting], TransitionActor::System);
    end_session(tx, id, generation, NO_DEDICATED, &aborting, system, None, true, now)
}

/// FENCED then ABORTED in one transaction, telling the party and charging the host.
fn fence_and_abort(
    tx: &mut HuntTx<'_>,
    key: Key<'_>,
    why: &str,
    before_active: bool,
    now: UnixMillis,
) -> Result<Prepared, RealmsError> {
    let settled = [H::Completed, H::Aborted, H::Recovering, H::Draining];
    let live = |h: &HuntSession| h.generation == key.1 && !settled.contains(&h.state);
    let Some(hunt) = tx.find_hunt(key.0)?.filter(live) else {
        return Ok(Prepared::Nothing);
    };
    if hunt.state == H::Aborting {
        return Ok(end_aborting(tx, key, now)?.map_or(Prepared::Nothing, Prepared::WasAborting));
    }
    tx.revoke_generation(key.0, key.1, now)?;
    let fence = |h: &mut HuntSession| {
        h.failure_reason = Some(why.to_owned());
        h.state = H::Fenced;
    };
    let Some(fenced) = advance(tx, &hunt, &[hunt.state], fence)? else {
        return Ok(Prepared::Nothing);
    };
    system_transition(tx, (hunt.state, &fenced), why, now)?;
    cancel_open_offer(tx, &hunt, why, now)?;
    let abort = |h: &mut HuntSession| {
        clear_live_fields(h);
        (h.recovery_retry_at, h.host_offer_id) = (None, None);
        h.failure_reason = Some(why.to_owned());
        (h.state, h.terminal_at) = (H::Aborted, Some(now));
    };
    let Some(ended) = advance(tx, &fenced, &[H::Fenced], abort)? else {
        return Ok(Prepared::Nothing);
    };
    let suffix = match (before_active, hunt.realm_id.is_some()) {
        (true, _) => "_P2P_ONLY",
        (false, true) => "_REALM_ONLY",
        (false, false) => "_NO_DEDICATED_FALLBACK",
    };
    system_transition(tx, (H::Fenced, &ended), &format!("{why}{suffix}"), now)?;

    // Before active the party goes back to matchmaking; after, its bridges are fenced. A realm's host is told
    // even when it does not play.
    let (message, action, tell_unrostered_host) = match before_active {
        true => ("HOST_OFFER_CANCELLED", "CANCEL_MATCHMAKING", hunt.realm_id.is_some()),
        false => ("FENCE_NOW", "NONE", true),
    };
    let payload = strings(&[("reason", why), ("recoveryAction", action)]);
    let ttl = CONTROL_LIFE_MS;
    for p in &hunt.roster {
        queue(tx, &p.account_id, message, &ended, payload.clone(), now, ttl)?;
    }
    let host = hunt.host_account_id.as_str();
    if !host.is_empty() {
        if tell_unrostered_host && hunt.roster.iter().all(|p| p.account_id != host) {
            queue(tx, host, message, &ended, payload, now, ttl)?;
        }
        let charge = one(if before_active {
            |r| &mut r.starts_failed
        } else {
            |r| &mut r.unexpected_failures
        });
        tx.increment_reliability(host, charge, now, Some(now.saturating_add(RECOVERY_COOLDOWN_MS)))?;
    }
    Ok(Prepared::Ended(ended))
}

fn expire_recovery(
    tx: &mut HuntTx<'_>,
    (id, generation): Key<'_>,
    now: UnixMillis,
) -> Result<Option<HuntSession>, RepoError> {
    let recovering = |h: &HuntSession| h.state == H::Recovering && h.generation == generation;
    let Some(hunt) = tx.find_hunt(id)?.filter(recovering) else {
        return Ok(None);
    };
    let ended = advance(tx, &hunt, &[H::Recovering], |h| {
        (h.active_party_key, h.recovery_deadline) = (None, None);
        h.failure_reason = Some(String::from("RECOVERY_WINDOW_EXPIRED"));
        (h.state, h.terminal_at) = (H::Aborted, Some(now));
    })?;
    if let Some(after) = &ended {
        system_transition(tx, (H::Recovering, after), "RECOVERY_WINDOW_EXPIRED", now)?;
    }
    Ok(ended)
}

impl RealmsRecovery {
    /// Recovery over the hunt tables and the module's core.
    pub fn new(db: RealmsDb, core: Arc<dyn IslesCore>, clock: Arc<dyn Clock>, log: Arc<dyn RealmsLog>) -> Self {
        let hunts = HuntRepository::new(db);
        Self {
            log,
            clock,
            core,
            hunts,
        }
    }

    /// Best effort: the core may already have let the party go.
    fn cancel_travel(&self, hunt: &HuntSession, reason: &str) {
        let accounts: Vec<AccountId> = hunt.roster.iter().map(|p| AccountId(p.account_id.clone())).collect();
        if !accounts.is_empty() {
            let _ = self.core.cancel_travel(&hunt.hunt_definition_id, &accounts, reason);
        }
    }

    fn not_wired(&self, session_id: &str, reason: &str) {
        let line = format!(
            "[P2P] ending {session_id} instead of recovering it: dedicated recovery seam not wired reason={reason}"
        );
        self.log.warn(&line);
    }

    async fn prepare_now(
        &self,
        (id, generation): (String, i64),
        reason: String,
        before_active: bool,
    ) -> Result<(), RealmsError> {
        let now = self.clock.now();
        let why = reason.clone();
        let work = move |tx: &mut HuntTx<'_>| fence_and_abort(tx, (&id, generation), &why, before_active, now);
        match self.hunts.transaction(&RECOVERY_ACTOR, work).await? {
            Prepared::Nothing => {}
            Prepared::WasAborting(hunt) => {
                self.cancel_travel(&hunt, &reason);
                self.not_wired(&hunt.session_id, &reason);
            }
            Prepared::Ended(hunt) => {
                self.cancel_travel(&hunt, &reason);
                let id = &hunt.session_id;
                self.log.warn(&match (before_active, hunt.realm_id.is_some()) {
                    (true, _) => format!(
                        "[P2P] pre-active failure canceled without dedicated hunt fallback session={id} reason={reason}"
                    ),
                    (false, true) => {
                        format!("[P2P] active realm hunt failed; ended without recovery session={id} reason={reason}")
                    }
                    (false, false) => format!(
                        "[P2P] active hunt failed without dedicated recovery seam; ended session={id} reason={reason}"
                    ),
                });
            }
        }
        Ok(())
    }

    async fn advance_now(&self, session: HuntSession) -> Result<(), RealmsError> {
        let now = self.clock.now();
        let (id, generation) = (session.session_id.clone(), session.generation);
        if session.state == H::Recovering {
            let work = move |tx: &mut HuntTx<'_>| Ok::<_, RealmsError>(expire_recovery(tx, (&id, generation), now)?);
            if self.hunts.transaction(&RECOVERY_ACTOR, work).await?.is_some() {
                let ended = &session.session_id;
                self.log
                    .warn(&format!("[P2P] recovery window expired; ended session={ended}"));
            }
        } else if session.state == H::Aborting {
            let reason = session.failure_reason.as_deref().unwrap_or("HOST_FAILED");
            self.not_wired(&session.session_id, reason);
            let work = move |tx: &mut HuntTx<'_>| Ok::<_, RealmsError>(end_aborting(tx, (&id, generation), now)?);
            self.hunts.transaction(&RECOVERY_ACTOR, work).await?;
        }
        Ok(())
    }
}

impl Recovery for RealmsRecovery {
    fn advance(&self, session: HuntSession) -> BoxFuture<'_, Result<(), RealmsError>> {
        Box::pin(self.advance_now(session))
    }

    fn prepare(
        &self,
        session_id: String,
        generation: i64,
        reason: String,
        before_active: bool,
    ) -> BoxFuture<'_, Result<(), RealmsError>> {
        Box::pin(self.prepare_now((session_id, generation), reason, before_active))
    }
}

#[cfg(test)]
mod tests {
    use super::GuardRole::{self, Client, Host};

    #[test]
    fn roles_order_and_text() {
        assert!(Client < Host);
        assert!(
            [Client, Host]
                .into_iter()
                .all(|role| GuardRole::parse(role.as_str()) == Some(role))
        );
        assert_eq!(GuardRole::parse("HOST"), None);
    }
}
