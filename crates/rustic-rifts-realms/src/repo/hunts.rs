// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Hunt and hub sessions with their roster and transitions, host capabilities, the launcher's control channel, host
//! presences, offers, reliability and participant observations. `HuntTx` is one write transaction over all of them,
//! so a lifecycle change commits every step or none.

use dainty_module_api::NodeId;
use dainty_store::AnyConnection;
use rustic_rifts_seam::{Actor, CapabilityScope, UnixMillis};
use serde_json::Value;

use super::hub::{HubPresenceRecord, end_generation};
use super::sql::{Arg, Bind, Statement, binds, json_error, marks, table_row, upsert};
use crate::db::{RealmsDb, RepoError, store_actor};
use crate::hunt::{
    AdmissionState, ControlMessageRecord, HostCapabilityRecord, HostOfferRecord, HostPresenceRecord,
    HostReliabilityRecord, HuntSession, HuntState, HuntTransition, ObservationRecord, OfferStatus, Participant,
    ReliabilityIncrements, ScoreBreakdown, hub_kind_name,
};
use crate::ids::{global_session_id, random_uuid};

use HuntState::{Admitting, Draining, HostOffered, HostReady, HostSelecting, Running, Starting, Suspect};

/// A host's sessions that hold its party key and keep its runtime busy.
pub const LIVE_HOST_STATES: &[HuntState] = &[HostOffered, Starting, HostReady, Admitting, Running, Suspect];

/// A realm's servers: its hubs, Training and hunts.
const REALM_SERVER_STATES: &[HuntState] = &[
    HostSelecting,
    HostOffered,
    Starting,
    HostReady,
    Admitting,
    Running,
    Suspect,
];

const ROSTER_LIVE_STATES: &[HuntState] = &[HostReady, Admitting, Running];

const RENEWABLE_STATES: &[HuntState] = &[HostReady, Admitting, Running, Draining, Suspect];

/// A host's runtime starting a session it accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostStartup {
    /// The session.
    pub session_id: String,
    /// The host.
    pub host_account_id: String,
    /// The host's bridge instance.
    pub bridge_instance_id: String,
    /// The nonce the runtime's metadata carries.
    pub metadata_nonce: String,
    /// When the host must be ready by.
    pub startup_deadline: UnixMillis,
    /// The game server's UDP port, if the host chose one.
    pub server_udp_port: Option<i64>,
}

/// A host's lease heartbeat.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaseRenewal {
    /// The session.
    pub session_id: String,
    /// The generation the host holds.
    pub generation: i64,
    /// The host's transport peer id.
    pub host_steam_id: String,
    /// The host's bridge instance.
    pub bridge_instance_id: String,
    /// The heartbeat's sequence number; only a newer one renews.
    pub lease_sequence: i64,
    /// When the heartbeat was sent.
    pub heartbeat_at: UnixMillis,
    /// The new lease expiry.
    pub lease_expires_at: UnixMillis,
}

/// A host's runtime reporting it is ready.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostReadyMark {
    /// The session.
    pub session_id: String,
    /// The generation the host holds.
    pub generation: i64,
    /// The host's transport peer id.
    pub host_steam_id: String,
    /// The host's bridge instance.
    pub bridge_instance_id: String,
    /// The transport lobby it opened.
    pub lobby_id: String,
    /// The game's virtual port; must match the session's.
    pub virtual_port: i64,
    /// The first lease expiry.
    pub lease_expires_at: UnixMillis,
}

/// A message to queue for an account's launcher; the queue gives its id and sequence.
#[derive(Clone, Debug, PartialEq)]
pub struct NewControlMessage {
    /// The account it is for.
    pub account_id: String,
    /// The message type, such as `ICE_SIGNAL`.
    pub message_type: String,
    /// The session it is about, if any.
    pub session_id: Option<String>,
    /// That session's generation.
    pub generation: Option<i64>,
    /// The message body.
    pub payload: Value,
    /// When it was queued.
    pub created_at: UnixMillis,
    /// When it is dropped undelivered.
    pub expires_at: UnixMillis,
}

const SESSIONS: &str = "rifts_hunt_sessions";
const ROSTER: &str = "rifts_hunt_roster";
const CAPABILITIES: &str = "rifts_host_capabilities";
const CONTROL: &str = "rifts_control_messages";
const OFFERS: &str = "rifts_host_offers";
const RELIABILITY: &str = "rifts_host_reliability";
const OBSERVATIONS: &str = "rifts_participant_observations";

/// `state IN (..)` and its values.
fn in_states(states: &[HuntState]) -> (String, Vec<Arg>) {
    let clause = format!("state IN ({})", marks(states.len()));
    (clause, states.iter().map(Bind::arg).collect())
}

fn columns(row: &[&str]) -> String {
    row.join(", ")
}

/// `INSERT INTO table (<every column>) VALUES (..)`, as `?` text.
fn insert_text(table: &str, row: &[&str]) -> String {
    format!("INSERT INTO {table} ({}) VALUES ({})", columns(row), marks(row.len()))
}

table_row!(SessionRow {
    session_id: String,
    detail: String,
});

table_row!(DetailRow {
    account_id: String,
    detail: String,
});

// The roster row is the participant plus its session, key and order.
table_row!(RosterRow for Participant {
    account_id: String,
    steam_id: String,
    candidate_id: String,
    reconnect_epoch: i64,
    admission_state: AdmissionState,
    local_bridge_instance_id: Option<String>,
    local_bridge_port: Option<i64>,
    local_bridge_ready_at: Option<UnixMillis>,
    admitted_at: Option<UnixMillis>,
    disconnected_at: Option<UnixMillis>,
    rejoin_deadline: Option<UnixMillis>,
    returned_to_hub_at: Option<UnixMillis>,
});

/// Indexed (queryable) copies of the detail's fields; `global_id` is fixed by the first save.
const SESSION_COLUMNS: [&str; 15] = [
    "session_id",
    "global_id",
    "generation",
    "state",
    "state_version",
    "party_id",
    "active_party_key",
    "host_account_id",
    "session_kind",
    "hub_kind",
    "realm_id",
    "lease_expires_at",
    "created_at",
    "expires_at",
    "detail",
];

table_row!(TransitionRow for HuntTransition {
    transition_id: String,
    session_id: String,
    generation: i64,
    from_state: HuntState,
    to_state: HuntState,
    state_version: i64,
    reason: String,
    actor_type: String,
    actor_account_id: Option<String>,
    at: UnixMillis,
});

table_row!(CapabilityRow for HostCapabilityRecord {
    capability_id: String,
    secret_digest: String,
    session_id: String,
    generation: i64,
    host_account_id: String,
    host_steam_id: String,
    bridge_instance_id: String,
    scopes: Vec<CapabilityScope>,
    issued_at: UnixMillis,
    expires_at: UnixMillis,
    revoked_at: Option<UnixMillis>,
    last_sequence: i64,
});

table_row!(ControlRow for ControlMessageRecord {
    message_id: String,
    account_id: String,
    sequence: i64,
    message_type: String,
    session_id: Option<String>,
    generation: Option<i64>,
    payload: Value,
    created_at: UnixMillis,
    expires_at: UnixMillis,
    acknowledged_at: Option<UnixMillis>,
});

table_row!(OfferRow for HostOfferRecord {
    offer_id: String,
    session_id: String,
    generation: i64,
    account_id: String,
    steam_id: String,
    rank: i64,
    score: f64,
    score_breakdown: ScoreBreakdown,
    status: OfferStatus,
    offered_at: UnixMillis,
    expires_at: UnixMillis,
    responded_at: Option<UnixMillis>,
    response_reason: Option<String>,
});

table_row!(ReliabilityRow for HostReliabilityRecord {
    account_id: String,
    offers_accepted: i64,
    offers_declined: i64,
    offers_expired: i64,
    starts_succeeded: i64,
    starts_failed: i64,
    clean_completions: i64,
    unexpected_failures: i64,
    cooldown_until: Option<UnixMillis>,
    updated_at: UnixMillis,
});

table_row!(ObservationRow for ObservationRecord {
    observation_id: String,
    session_id: String,
    generation: i64,
    account_id: String,
    steam_id: String,
    host_reachable: bool,
    local_bridge_alive: bool,
    metrics_available: bool,
    last_packet_age_ms: Option<i64>,
    route_kind: String,
    ping_ms: Option<i64>,
    jitter_ms: Option<i64>,
    packet_loss_percent: Option<f64>,
    observed_at: UnixMillis,
});

const HOST_PRESENCE_COLUMNS: [&str; 6] = [
    "account_id",
    "revision",
    "consent_to_host",
    "observed_at",
    "expires_at",
    "detail",
];

fn detail_error(session_id: &str, error: &serde_json::Error) -> RepoError {
    RepoError::Corrupt(format!("hunt {session_id}: {error}"))
}

fn with_roster(conn: &mut AnyConnection, row: SessionRow) -> Result<HuntSession, RepoError> {
    let mut hunt: HuntSession = serde_json::from_str(&row.detail).map_err(|e| detail_error(&row.session_id, &e))?;
    let text = format!(
        "SELECT {} FROM {ROSTER} WHERE session_id = ? ORDER BY seq",
        columns(RosterRow::COLUMNS)
    );
    let roster: Vec<RosterRow> = Statement::new(&text, binds![row.session_id])?.rows(conn)?;
    hunt.roster = roster.into_iter().map(RosterRow::record).collect::<Result<_, _>>()?;
    Ok(hunt)
}

fn sessions(conn: &mut AnyConnection, tail: &str, args: Vec<Arg>) -> Result<Vec<HuntSession>, RepoError> {
    let text = format!("SELECT session_id, detail FROM {SESSIONS} {tail}");
    let rows: Vec<SessionRow> = Statement::new(&text, args)?.rows(conn)?;
    rows.into_iter().map(|row| with_roster(conn, row)).collect()
}

fn first_session(conn: &mut AnyConnection, tail: &str, args: Vec<Arg>) -> Result<Option<HuntSession>, RepoError> {
    Ok(sessions(conn, tail, args)?.into_iter().next())
}

fn load(conn: &mut AnyConnection, session_id: &str) -> Result<Option<HuntSession>, RepoError> {
    first_session(conn, "WHERE session_id = ?", binds![session_id])
}

fn any(conn: &mut AnyConnection, text: &str, args: Vec<Arg>) -> Result<bool, RepoError> {
    Ok(Statement::new(text, args)?.number(conn)? > 0)
}

/// Whether a session other than `except` holds `key`.
fn key_taken(conn: &mut AnyConnection, key: &str, except: Option<&str>) -> Result<bool, RepoError> {
    let text = "SELECT COUNT(*) AS n FROM rifts_hunt_sessions WHERE active_party_key = ? AND session_id <> ?";
    any(conn, text, binds![key, except.unwrap_or_default()])
}

/// Upserts the row (its global id fixed by the first save) and syncs the roster rows to the record's.
fn save(conn: &mut AnyConnection, node: &NodeId, hunt: &HuntSession) -> Result<(), RepoError> {
    let detail = serde_json::to_string(hunt).map_err(|e| detail_error(&hunt.session_id, &e))?;
    let global = global_session_id(&hunt.session_id, hunt.is_realm_hub(), node).map(|id| id.to_string());
    let session_kind = hunt.hub_kind.map(|_| "REALM_HUB");
    let row = binds![
        hunt.session_id,
        global,
        hunt.generation,
        hunt.state,
        hunt.state_version,
        hunt.party_id,
        hunt.active_party_key,
        hunt.host_account_id,
        session_kind,
        hunt.hub_kind.map(hub_kind_name),
        hunt.realm_id,
        hunt.lease_expires_at,
        hunt.created_at,
        hunt.expires_at,
        detail,
    ];
    upsert(conn, SESSIONS, (&SESSION_COLUMNS, &["session_id"], &["global_id"]), row)?;

    let accounts: Vec<&str> = hunt.roster.iter().map(|p| p.account_id.as_str()).collect();
    let keep = match accounts.len() {
        0 => String::new(),
        n => format!(" AND account_id NOT IN ({})", marks(n)),
    };
    let mut prune = binds![hunt.session_id];
    prune.extend(accounts.iter().map(Bind::arg));
    Statement::new(&format!("DELETE FROM {ROSTER} WHERE session_id = ?{keep}"), prune)?.run(conn)?;

    // `(session_id, account_id, seq, <the rest of the participant>)`, keyed by the first two.
    let (key, rest) = RosterRow::COLUMNS.split_at(1);
    let roster_columns: Vec<&str> = ["session_id"]
        .iter()
        .chain(key)
        .chain(&["seq"])
        .chain(rest)
        .copied()
        .collect();
    for (participant, seq) in hunt.roster.iter().zip(1i64..) {
        let mut values = RosterRow::values(participant);
        let tail = values.split_off(1);
        let mut args = binds![hunt.session_id];
        args.extend(values);
        args.extend(binds![seq]);
        args.extend(tail);
        upsert(conn, ROSTER, (&roster_columns, &roster_columns[..2], &[]), args)?;
    }
    Ok(())
}

fn reliability(conn: &mut AnyConnection, account_id: &str) -> Result<Option<HostReliabilityRecord>, RepoError> {
    let text = format!(
        "SELECT {} FROM {RELIABILITY} WHERE account_id = ?",
        columns(ReliabilityRow::COLUMNS)
    );
    Statement::new(&text, binds![account_id])?
        .first::<ReliabilityRow>(conn)?
        .map(ReliabilityRow::record)
        .transpose()
}

fn offer(conn: &mut AnyConnection, offer_id: &str) -> Result<Option<HostOfferRecord>, RepoError> {
    let text = format!("SELECT {} FROM {OFFERS} WHERE offer_id = ?", columns(OfferRow::COLUMNS));
    Statement::new(&text, binds![offer_id])?
        .first::<OfferRow>(conn)?
        .map(OfferRow::record)
        .transpose()
}

fn latest_sequence(conn: &mut AnyConnection, account_id: &str) -> Result<i64, RepoError> {
    let text = "SELECT next_sequence AS n FROM rifts_control_sequences WHERE account_id = ?";
    Statement::new(text, binds![account_id])?.number(conn)
}

/// One write transaction over the hunt tables (Node's `WithP2PTransaction`).
pub struct HuntTx<'a> {
    conn: &'a mut AnyConnection,
    node: &'a NodeId,
}

impl HuntTx<'_> {
    fn exec(&mut self, text: &str, args: Vec<Arg>) -> Result<usize, RepoError> {
        Statement::new(text, args)?.run(self.conn)
    }

    /// The session with its roster, if it exists.
    pub fn find_hunt(&mut self, session_id: &str) -> Result<Option<HuntSession>, RepoError> {
        load(self.conn, session_id)
    }

    /// Stores a new session and its roster; `Duplicate` for a taken session id or active party key.
    pub fn create_hunt(&mut self, session: &HuntSession) -> Result<(), RepoError> {
        let text = "SELECT COUNT(*) AS n FROM rifts_hunt_sessions WHERE session_id = ?";
        if any(self.conn, text, binds![session.session_id])? {
            return Err(RepoError::Duplicate("session_id"));
        }
        let key = session.active_party_key.as_deref();
        if key.map(|key| key_taken(self.conn, key, None)).transpose()? == Some(true) {
            return Err(RepoError::Duplicate("active_party_key"));
        }
        save(self.conn, self.node, session)
    }

    /// `change` decides (false writes nothing) and edits; `bump` adds one to the state version.
    pub fn update(
        &mut self,
        session_id: &str,
        bump: bool,
        change: impl FnOnce(&mut HuntSession) -> bool,
    ) -> Result<Option<HuntSession>, RepoError> {
        let Some(mut hunt) = load(self.conn, session_id)? else {
            return Ok(None);
        };
        if !change(&mut hunt) {
            return Ok(None);
        }
        hunt.state_version += i64::from(bump);
        if let Some(key) = hunt.active_party_key.as_deref()
            && key_taken(self.conn, key, Some(&hunt.session_id))?
        {
            return Err(RepoError::Duplicate("active_party_key"));
        }
        save(self.conn, self.node, &hunt).map(|()| Some(hunt))
    }

    /// Node's `compareAndSetHunt`: `change` applies only while generation, state and version all still match.
    pub fn compare_and_set(
        &mut self,
        session_id: &str,
        generation: i64,
        expected_states: &[HuntState],
        expected_state_version: i64,
        change: impl FnOnce(&mut HuntSession),
    ) -> Result<Option<HuntSession>, RepoError> {
        self.update(session_id, true, |hunt| {
            let unchanged = (hunt.generation, hunt.state_version) == (generation, expected_state_version)
                && expected_states.contains(&hunt.state);
            unchanged.then(|| change(hunt)).is_some()
        })
    }

    /// Records a state change, giving it an id when it has none.
    pub fn append_transition(&mut self, t: &HuntTransition) -> Result<(), RepoError> {
        let mut stored = t.clone();
        if stored.transition_id.is_empty() {
            stored.transition_id = random_uuid();
        }
        let text = insert_text("rifts_hunt_transitions", TransitionRow::COLUMNS);
        self.exec(&text, TransitionRow::values(&stored)).map(drop)
    }

    /// HOST_OFFERED to STARTING for that host of an unexpired session; no generation check.
    pub fn begin_host_startup(&mut self, s: &HostStartup, now: UnixMillis) -> Result<Option<HuntSession>, RepoError> {
        self.update(&s.session_id, true, |hunt| {
            let offered =
                hunt.state == HostOffered && hunt.host_account_id == s.host_account_id && now < hunt.expires_at;
            if offered {
                hunt.state = Starting;
                hunt.bridge_instance_id.clone_from(&s.bridge_instance_id);
                hunt.metadata_nonce = Some(s.metadata_nonce.clone());
                hunt.startup_deadline = Some(s.startup_deadline);
                hunt.server_udp_port = s.server_udp_port.or(hunt.server_udp_port);
                (hunt.lease_expires_at, hunt.suspect_since, hunt.suspect_deadline) = (None, None, None);
                (hunt.lobby_id, hunt.pre_suspect_state) = (None, None);
            }
            offered
        })
    }

    /// Only a strictly newer lease sequence from the session's current host and bridge.
    pub fn renew_host_lease(&mut self, r: &LeaseRenewal) -> Result<Option<HuntSession>, RepoError> {
        self.update(&r.session_id, true, |hunt| {
            let same_host = hunt.generation == r.generation
                && hunt.host_steam_id == r.host_steam_id
                && hunt.bridge_instance_id == r.bridge_instance_id;
            let newer = hunt.last_lease_sequence.is_none_or(|last| r.lease_sequence > last);
            let renewed = same_host && RENEWABLE_STATES.contains(&hunt.state) && newer;
            if renewed {
                hunt.lease_expires_at = Some(r.lease_expires_at);
                hunt.last_heartbeat_at = Some(r.heartbeat_at);
                hunt.last_lease_sequence = Some(r.lease_sequence);
            }
            renewed
        })
    }

    /// Moves STARTING or HOST_READY to HOST_READY for the session's current host, bridge and virtual port; `None` when they do not match.
    pub fn mark_host_ready(&mut self, m: &HostReadyMark, now: UnixMillis) -> Result<Option<HuntSession>, RepoError> {
        self.update(&m.session_id, true, |hunt| {
            let same_host = hunt.generation == m.generation
                && hunt.host_steam_id == m.host_steam_id
                && hunt.bridge_instance_id == m.bridge_instance_id
                && hunt.virtual_port == m.virtual_port;
            let ready = same_host && matches!(hunt.state, Starting | HostReady);
            if ready {
                hunt.state = HostReady;
                hunt.lobby_id = Some(m.lobby_id.clone());
                (hunt.lease_expires_at, hunt.last_heartbeat_at) = (Some(m.lease_expires_at), Some(now));
                hunt.last_lease_sequence = Some(0);
            }
            ready
        })
    }

    /// One participant's change: generation and state set hold, `pick` finds the entry, `edit` may still refuse.
    fn participant_change(
        &mut self,
        (session_id, generation, states): (&str, i64, &[HuntState]),
        bump: bool,
        pick: impl Fn(&Participant) -> bool,
        edit: impl FnOnce(&mut Participant) -> bool,
    ) -> Result<Option<HuntSession>, RepoError> {
        self.update(session_id, bump, |hunt| {
            let current = generation == hunt.generation && states.contains(&hunt.state);
            current && hunt.roster.iter_mut().find(|p| pick(p)).is_some_and(edit)
        })
    }

    /// Matched by account and peer id; no version bump.
    #[allow(
        clippy::too_many_arguments,
        reason = "the arguments are the row's key (session, generation, account) and its new values"
    )]
    pub fn mark_participant_route_ready(
        &mut self,
        session_id: &str,
        generation: i64,
        account_id: &str,
        steam_id: &str,
        bridge_instance_id: &str,
        local_bridge_port: i64,
        ready_at: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let pick = |p: &Participant| p.steam_id == steam_id && p.account_id == account_id;
        let route = |p: &mut Participant| {
            p.local_bridge_instance_id = Some(bridge_instance_id.to_owned());
            (p.local_bridge_port, p.local_bridge_ready_at) = (Some(local_bridge_port), Some(ready_at));
            true
        };
        self.participant_change((session_id, generation, ROSTER_LIVE_STATES), false, pick, route)
    }

    /// Marks the participant admitted, clearing any disconnection; `None` when the session or participant does not match.
    pub fn mark_participant_admitted(
        &mut self,
        session_id: &str,
        generation: i64,
        account_id: &str,
        admitted_at: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let admit = |p: &mut Participant| {
            p.admission_state = AdmissionState::Admitted;
            p.admitted_at = Some(admitted_at);
            (p.disconnected_at, p.rejoin_deadline) = (None, None);
            true
        };
        let pick = |p: &Participant| p.account_id == account_id;
        self.participant_change((session_id, generation, ROSTER_LIVE_STATES), true, pick, admit)
    }

    /// Refused past the reconnect budget, or for a disconnected participant whose rejoin window has closed.
    #[allow(
        clippy::too_many_arguments,
        reason = "the arguments are the row's key (session, generation, account) and its new values"
    )]
    pub fn reserve_participant_ticket(
        &mut self,
        session_id: &str,
        generation: i64,
        account_id: &str,
        now: UnixMillis,
        rejoin_deadline: UnixMillis,
        max_reconnects: i64,
    ) -> Result<Option<HuntSession>, RepoError> {
        let reserve = |p: &mut Participant| {
            let window_open = p.disconnected_at.is_none() || p.rejoin_deadline.is_some_and(|deadline| now < deadline);
            let allowed = window_open && p.reconnect_epoch < max_reconnects;
            if allowed {
                p.reconnect_epoch += 1;
                p.admission_state = AdmissionState::Issued;
                p.rejoin_deadline = Some(rejoin_deadline);
            }
            allowed
        };
        let pick = |p: &Participant| p.account_id == account_id;
        self.participant_change((session_id, generation, ROSTER_LIVE_STATES), false, pick, reserve)
    }

    /// Marks the participant lost with its rejoin deadline; `None` when the session or participant does not match.
    pub fn mark_participant_disconnected(
        &mut self,
        session_id: &str,
        generation: i64,
        account_id: &str,
        disconnected_at: UnixMillis,
        rejoin_deadline: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let lose = |p: &mut Participant| {
            p.admission_state = AdmissionState::Lost;
            (p.disconnected_at, p.rejoin_deadline) = (Some(disconnected_at), Some(rejoin_deadline));
            true
        };
        let pick = |p: &Participant| p.account_id == account_id;
        self.participant_change((session_id, generation, RENEWABLE_STATES), false, pick, lose)
    }

    /// Stores a capability; `Duplicate` for a secret digest already stored.
    pub fn create_capability(&mut self, c: &HostCapabilityRecord) -> Result<(), RepoError> {
        let text = "SELECT COUNT(*) AS n FROM rifts_host_capabilities WHERE secret_digest = ?";
        if any(self.conn, text, binds![c.secret_digest])? {
            return Err(RepoError::Duplicate("secret_digest"));
        }
        let text = insert_text(CAPABILITIES, CapabilityRow::COLUMNS);
        self.exec(&text, CapabilityRow::values(c)).map(drop)
    }

    /// Raises the capability's last sequence to `sequence`; false when it is revoked or the sequence is not newer.
    pub fn advance_capability_sequence(&mut self, capability_id: &str, sequence: i64) -> Result<bool, RepoError> {
        let text = "UPDATE rifts_host_capabilities SET last_sequence = ? \
                    WHERE capability_id = ? AND revoked_at IS NULL AND last_sequence < ?";
        Ok(self.exec(text, binds![sequence, capability_id, sequence])? == 1)
    }

    /// An earlier revocation time stays.
    pub fn revoke_generation(&mut self, session_id: &str, generation: i64, at: UnixMillis) -> Result<(), RepoError> {
        let text = "UPDATE rifts_host_capabilities SET revoked_at = ? \
                    WHERE session_id = ? AND generation = ? AND revoked_at IS NULL";
        self.exec(text, binds![at, session_id, generation]).map(drop)
    }

    /// Sequences outlive the messages' purge, so they never repeat.
    pub fn enqueue_control(&mut self, m: &NewControlMessage) -> Result<ControlMessageRecord, RepoError> {
        let sequence = 1 + latest_sequence(self.conn, &m.account_id)?;
        let counter = (&["account_id", "next_sequence"][..], &["account_id"][..], &[][..]);
        upsert(
            self.conn,
            "rifts_control_sequences",
            counter,
            binds![m.account_id, sequence],
        )?;
        let NewControlMessage {
            account_id,
            message_type,
            session_id,
            generation,
            payload,
            created_at,
            expires_at,
        } = m.clone();
        let record = ControlMessageRecord {
            message_id: random_uuid(),
            acknowledged_at: None,
            sequence,
            account_id,
            message_type,
            session_id,
            generation,
            payload,
            created_at,
            expires_at,
        };
        let text = insert_text(CONTROL, ControlRow::COLUMNS);
        self.exec(&text, ControlRow::values(&record))?;
        Ok(record)
    }

    /// Only the account's own message; a second call dates it again.
    pub fn acknowledge_control(
        &mut self,
        account_id: &str,
        message_id: &str,
        at: UnixMillis,
    ) -> Result<bool, RepoError> {
        let text = "UPDATE rifts_control_messages SET acknowledged_at = ? WHERE message_id = ? AND account_id = ?";
        Ok(self.exec(text, binds![at, message_id, account_id])? > 0)
    }

    /// Stores the account's host presence, replacing any earlier one.
    pub fn upsert_host_presence(&mut self, p: &HostPresenceRecord) -> Result<(), RepoError> {
        let detail = serde_json::to_string(p).map_err(json_error)?;
        let args = binds![
            p.account_id,
            p.revision,
            p.consent_to_host,
            p.observed_at,
            p.expires_at,
            detail
        ];
        upsert(
            self.conn,
            "rifts_host_presences",
            (&HOST_PRESENCE_COLUMNS, &["account_id"], &[]),
            args,
        )
    }

    /// The host's reliability record, if it has one.
    pub fn find_reliability(&mut self, account_id: &str) -> Result<Option<HostReliabilityRecord>, RepoError> {
        reliability(self.conn, account_id)
    }

    /// A given cooldown replaces the old one; an absent one keeps it.
    pub fn increment_reliability(
        &mut self,
        account_id: &str,
        add: ReliabilityIncrements,
        now: UnixMillis,
        cooldown_until: Option<UnixMillis>,
    ) -> Result<(), RepoError> {
        let mut record =
            reliability(self.conn, account_id)?.unwrap_or_else(|| HostReliabilityRecord::new(account_id, now));
        record.add(add);
        record.updated_at = now;
        record.cooldown_until = cooldown_until.or(record.cooldown_until);
        let row = (ReliabilityRow::COLUMNS, &["account_id"][..], &[][..]);
        upsert(self.conn, RELIABILITY, row, ReliabilityRow::values(&record))
    }

    /// One accepted offer per generation.
    pub fn create_offer(&mut self, o: &HostOfferRecord) -> Result<(), RepoError> {
        let accepted = OfferStatus::Accepted;
        let text = "SELECT COUNT(*) AS n FROM rifts_host_offers WHERE session_id = ? AND generation = ? AND status = ?";
        if o.status == accepted && any(self.conn, text, binds![o.session_id, o.generation, accepted])? {
            return Err(RepoError::Duplicate("accepted offer"));
        }
        let row = (OfferRow::COLUMNS, &["offer_id"][..], &[][..]);
        upsert(self.conn, OFFERS, row, OfferRow::values(o))
    }

    /// The offer, if it exists.
    pub fn find_offer(&mut self, offer_id: &str) -> Result<Option<HostOfferRecord>, RepoError> {
        offer(self.conn, offer_id)
    }

    /// Moves that account's offer only from `expected`; `reason: None` clears it.
    pub fn update_offer(
        &mut self,
        offer_id: &str,
        account_id: &str,
        expected: OfferStatus,
        status: OfferStatus,
        responded_at: UnixMillis,
        reason: Option<&str>,
    ) -> Result<Option<HostOfferRecord>, RepoError> {
        let text = "UPDATE rifts_host_offers SET status = ?, responded_at = ?, response_reason = ? \
                    WHERE offer_id = ? AND account_id = ? AND status = ?";
        let args = binds![status, responded_at, reason, offer_id, account_id, expected];
        match self.exec(text, args)? {
            0 => Ok(None),
            _ => offer(self.conn, offer_id),
        }
    }

    /// The hub generation's ending, committed with the session's own change.
    pub fn end_hub_presence(
        &mut self,
        session_id: &str,
        generation: i64,
        at: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Vec<HubPresenceRecord>, RepoError> {
        end_generation(self.conn, session_id, generation, at, purge_at)
    }

    /// A repeated id is a database error.
    pub fn record_observation(&mut self, o: &ObservationRecord) -> Result<(), RepoError> {
        let text = insert_text(OBSERVATIONS, ObservationRow::COLUMNS);
        self.exec(&text, ObservationRow::values(o)).map(drop)
    }
}

/// Realms' part of Node's hunt repository.
#[derive(Clone)]
pub struct HuntRepository {
    db: RealmsDb,
}

impl HuntRepository {
    /// The hunt tables in `db`.
    pub fn new(db: RealmsDb) -> Self {
        Self { db }
    }

    /// `f` as one write: an `Err`, a refusal included, rolls every step back.
    pub async fn transaction<T, E, F>(&self, actor: &Actor, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut HuntTx<'_>) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<dainty_store::Error> + From<diesel::result::Error> + Send + 'static,
    {
        let node = self.db.node().clone();
        let body = move |tx: &mut dainty_store::Tx<'_>| {
            let mut hunt_tx = HuntTx {
                conn: tx.conn(),
                node: &node,
            };
            f(&mut hunt_tx)
        };
        self.db.database().write(store_actor(actor), body).await
    }

    async fn read<T, F>(&self, f: F) -> Result<T, RepoError>
    where
        F: FnOnce(&mut AnyConnection) -> Result<T, RepoError> + Send + 'static,
        T: Send + 'static,
    {
        self.db.read(f).await
    }

    /// As [`HuntTx::create_hunt`], in a transaction of its own.
    pub async fn create_hunt(&self, actor: &Actor, session: &HuntSession) -> Result<(), RepoError> {
        let hunt = session.clone();
        self.transaction(actor, move |tx| tx.create_hunt(&hunt)).await
    }

    /// As [`HuntTx::compare_and_set`], in a transaction of its own.
    pub async fn compare_and_set_hunt(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        expected_states: &[HuntState],
        expected_state_version: i64,
        change: impl FnOnce(&mut HuntSession) + Send + 'static,
    ) -> Result<Option<HuntSession>, RepoError> {
        let session = String::from(session_id);
        let states = Vec::from(expected_states);
        self.transaction(actor, move |tx| {
            tx.compare_and_set(&session, generation, &states, expected_state_version, change)
        })
        .await
    }

    /// As [`HuntTx::append_transition`], in a transaction of its own.
    pub async fn append_hunt_transition(&self, actor: &Actor, t: &HuntTransition) -> Result<(), RepoError> {
        let transition = t.clone();
        self.transaction(actor, move |tx| tx.append_transition(&transition))
            .await
    }

    /// As [`HuntTx::begin_host_startup`], in a transaction of its own.
    pub async fn begin_host_startup(
        &self,
        actor: &Actor,
        s: &HostStartup,
        now: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let startup = s.clone();
        self.transaction(actor, move |tx| tx.begin_host_startup(&startup, now))
            .await
    }

    /// As [`HuntTx::renew_host_lease`], in a transaction of its own.
    pub async fn renew_host_lease(&self, actor: &Actor, r: &LeaseRenewal) -> Result<Option<HuntSession>, RepoError> {
        let renewal = r.clone();
        self.transaction(actor, move |tx| tx.renew_host_lease(&renewal)).await
    }

    /// As [`HuntTx::mark_host_ready`], in a transaction of its own.
    pub async fn mark_host_ready(
        &self,
        actor: &Actor,
        m: &HostReadyMark,
        now: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let mark = m.clone();
        self.transaction(actor, move |tx| tx.mark_host_ready(&mark, now)).await
    }

    /// As [`HuntTx::mark_participant_route_ready`], in a transaction of its own.
    #[allow(clippy::too_many_arguments, reason = "mirrors the transaction method's arguments")]
    pub async fn mark_participant_route_ready(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        account_id: &str,
        steam_id: &str,
        bridge_instance_id: &str,
        local_bridge_port: i64,
        ready_at: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let [session, account, peer, bridge] = [session_id, account_id, steam_id, bridge_instance_id].map(String::from);
        self.transaction(actor, move |tx| {
            tx.mark_participant_route_ready(
                &session,
                generation,
                &account,
                &peer,
                &bridge,
                local_bridge_port,
                ready_at,
            )
        })
        .await
    }

    /// As [`HuntTx::mark_participant_admitted`], in a transaction of its own.
    pub async fn mark_participant_admitted(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        account_id: &str,
        admitted_at: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let [session, account] = [session_id, account_id].map(String::from);
        self.transaction(actor, move |tx| {
            tx.mark_participant_admitted(&session, generation, &account, admitted_at)
        })
        .await
    }

    /// As [`HuntTx::reserve_participant_ticket`], in a transaction of its own.
    #[allow(clippy::too_many_arguments, reason = "mirrors the transaction method's arguments")]
    pub async fn reserve_participant_ticket(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        account_id: &str,
        now: UnixMillis,
        rejoin_deadline: UnixMillis,
        max_reconnects: i64,
    ) -> Result<Option<HuntSession>, RepoError> {
        let [session, account] = [session_id, account_id].map(String::from);
        self.transaction(actor, move |tx| {
            tx.reserve_participant_ticket(&session, generation, &account, now, rejoin_deadline, max_reconnects)
        })
        .await
    }

    /// As [`HuntTx::mark_participant_disconnected`], in a transaction of its own.
    pub async fn mark_participant_disconnected(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        account_id: &str,
        disconnected_at: UnixMillis,
        rejoin_deadline: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let [session, account] = [session_id, account_id].map(String::from);
        self.transaction(actor, move |tx| {
            tx.mark_participant_disconnected(&session, generation, &account, disconnected_at, rejoin_deadline)
        })
        .await
    }

    /// As [`HuntTx::create_capability`], in a transaction of its own.
    pub async fn create_host_capability(&self, actor: &Actor, c: &HostCapabilityRecord) -> Result<(), RepoError> {
        let capability = c.clone();
        self.transaction(actor, move |tx| tx.create_capability(&capability))
            .await
    }

    /// As [`HuntTx::advance_capability_sequence`], in a transaction of its own.
    pub async fn advance_host_capability_sequence(
        &self,
        actor: &Actor,
        capability_id: &str,
        sequence: i64,
    ) -> Result<bool, RepoError> {
        let capability = String::from(capability_id);
        self.transaction(actor, move |tx| tx.advance_capability_sequence(&capability, sequence))
            .await
    }

    /// As [`HuntTx::revoke_generation`], in a transaction of its own.
    pub async fn revoke_generation(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        at: UnixMillis,
    ) -> Result<(), RepoError> {
        let session = String::from(session_id);
        self.transaction(actor, move |tx| tx.revoke_generation(&session, generation, at))
            .await
    }

    /// As [`HuntTx::enqueue_control`], in a transaction of its own.
    pub async fn enqueue_control_message(
        &self,
        actor: &Actor,
        m: &NewControlMessage,
    ) -> Result<ControlMessageRecord, RepoError> {
        let message = m.clone();
        self.transaction(actor, move |tx| tx.enqueue_control(&message)).await
    }

    /// As [`HuntTx::acknowledge_control`], in a transaction of its own.
    pub async fn acknowledge_control_message(
        &self,
        actor: &Actor,
        account_id: &str,
        message_id: &str,
        at: UnixMillis,
    ) -> Result<bool, RepoError> {
        let [account, message] = [account_id, message_id].map(String::from);
        self.transaction(actor, move |tx| tx.acknowledge_control(&account, &message, at))
            .await
    }

    /// As [`HuntTx::upsert_host_presence`], in a transaction of its own.
    pub async fn upsert_host_presence(&self, actor: &Actor, p: &HostPresenceRecord) -> Result<(), RepoError> {
        let presence = p.clone();
        self.transaction(actor, move |tx| tx.upsert_host_presence(&presence))
            .await
    }

    /// As [`HuntTx::increment_reliability`], in a transaction of its own.
    pub async fn increment_host_reliability(
        &self,
        actor: &Actor,
        account_id: &str,
        add: ReliabilityIncrements,
        now: UnixMillis,
        cooldown_until: Option<UnixMillis>,
    ) -> Result<(), RepoError> {
        let account = String::from(account_id);
        self.transaction(actor, move |tx| {
            tx.increment_reliability(&account, add, now, cooldown_until)
        })
        .await
    }

    /// As [`HuntTx::create_offer`], in a transaction of its own.
    pub async fn create_host_offer(&self, actor: &Actor, o: &HostOfferRecord) -> Result<(), RepoError> {
        let offered = o.clone();
        self.transaction(actor, move |tx| tx.create_offer(&offered)).await
    }

    /// As [`HuntTx::update_offer`], in a transaction of its own.
    #[allow(clippy::too_many_arguments, reason = "mirrors the transaction method's arguments")]
    pub async fn update_host_offer(
        &self,
        actor: &Actor,
        offer_id: &str,
        account_id: &str,
        expected: OfferStatus,
        status: OfferStatus,
        responded_at: UnixMillis,
        reason: Option<&str>,
    ) -> Result<Option<HostOfferRecord>, RepoError> {
        let [offered, account] = [offer_id, account_id].map(String::from);
        let reason = reason.map(String::from);
        self.transaction(actor, move |tx| {
            tx.update_offer(&offered, &account, expected, status, responded_at, reason.as_deref())
        })
        .await
    }

    /// As [`HuntTx::record_observation`], in a transaction of its own.
    pub async fn record_participant_observation(&self, actor: &Actor, o: &ObservationRecord) -> Result<(), RepoError> {
        let observation = o.clone();
        self.transaction(actor, move |tx| tx.record_observation(&observation))
            .await
    }

    /// The session with its roster, if it exists.
    pub async fn find_hunt(&self, session_id: &str) -> Result<Option<HuntSession>, RepoError> {
        let session = String::from(session_id);
        self.read(move |conn| load(conn, &session)).await
    }

    /// A party's key, or a realm hub kind's.
    pub async fn find_active_hunt_for_party(&self, party_key: &str) -> Result<Option<HuntSession>, RepoError> {
        let args = binds![party_key];
        self.read(move |conn| first_session(conn, "WHERE active_party_key = ?", args))
            .await
    }

    /// The account's newest active hunt; never a hub.
    pub async fn find_active_hunt_for_account(&self, account_id: &str) -> Result<Option<HuntSession>, RepoError> {
        const TAIL: &str = "WHERE session_id IN (SELECT session_id FROM rifts_hunt_roster WHERE account_id = ?) \
                            AND active_party_key IS NOT NULL AND session_kind IS NULL \
                            ORDER BY created_at DESC, session_id LIMIT 1";
        let args = binds![account_id];
        self.read(move |conn| first_session(conn, TAIL, args)).await
    }

    /// The account's newest unexpired RECOVERING hunt, if any.
    pub async fn find_recoverable_hunt_for_account(
        &self,
        account_id: &str,
        now: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        const TAIL: &str = "WHERE session_id IN (SELECT session_id FROM rifts_hunt_roster WHERE account_id = ?) \
                            AND state = ? AND expires_at > ? ORDER BY created_at DESC, session_id LIMIT 1";
        let args = binds![account_id, HuntState::Recovering, now];
        self.read(move |conn| first_session(conn, TAIL, args)).await
    }

    /// Oldest first.
    pub async fn list_lifecycle_hunts(&self, states: &[HuntState], max: u32) -> Result<Vec<HuntSession>, RepoError> {
        let (clause, mut args) = in_states(states);
        let tail = format!("WHERE {clause} ORDER BY created_at, session_id LIMIT ?");
        args.extend(binds![i64::from(max)]);
        self.read(move |conn| sessions(conn, &tail, args)).await
    }

    /// The session's state changes, oldest first.
    pub async fn list_transitions(&self, session_id: &str) -> Result<Vec<HuntTransition>, RepoError> {
        let text = format!(
            "SELECT {} FROM rifts_hunt_transitions WHERE session_id = ? ORDER BY at, state_version, transition_id",
            columns(TransitionRow::COLUMNS)
        );
        let args = binds![session_id];
        self.read(move |conn| {
            let rows: Vec<TransitionRow> = Statement::new(&text, args)?.rows(conn)?;
            rows.into_iter().map(TransitionRow::record).collect()
        })
        .await
    }

    /// The capability whose secret has this digest, revoked or not.
    pub async fn find_host_capability_by_digest(
        &self,
        digest: &str,
    ) -> Result<Option<HostCapabilityRecord>, RepoError> {
        let text = format!(
            "SELECT {} FROM {CAPABILITIES} WHERE secret_digest = ?",
            columns(CapabilityRow::COLUMNS)
        );
        let args = binds![digest];
        self.read(move |conn| {
            let row = Statement::new(&text, args)?.first::<CapabilityRow>(conn)?;
            row.map(CapabilityRow::record).transpose()
        })
        .await
    }

    /// Unacknowledged only is the stale-cursor answer.
    pub async fn list_control_messages(
        &self,
        account_id: &str,
        after_sequence: i64,
        now: UnixMillis,
        max: u32,
        unacknowledged_only: bool,
    ) -> Result<Vec<ControlMessageRecord>, RepoError> {
        let filter = if unacknowledged_only {
            " AND acknowledged_at IS NULL"
        } else {
            ""
        };
        let text = format!(
            "SELECT {} FROM {CONTROL} WHERE account_id = ? AND sequence > ? \
             AND expires_at > ?{filter} ORDER BY sequence LIMIT ?",
            columns(ControlRow::COLUMNS)
        );
        let args = binds![account_id, after_sequence, now, i64::from(max)];
        self.read(move |conn| {
            let rows: Vec<ControlRow> = Statement::new(&text, args)?.rows(conn)?;
            rows.into_iter().map(ControlRow::record).collect()
        })
        .await
    }

    /// 0 when the account never had a message.
    pub async fn latest_control_sequence(&self, account_id: &str) -> Result<i64, RepoError> {
        let account = String::from(account_id);
        self.read(move |conn| latest_sequence(conn, &account)).await
    }

    /// In the order the accounts were given.
    pub async fn list_host_presences(
        &self,
        account_ids: &[String],
        observed_after: UnixMillis,
        now: UnixMillis,
    ) -> Result<Vec<HostPresenceRecord>, RepoError> {
        if account_ids.is_empty() {
            return Ok(Vec::new());
        }
        let wanted = account_ids.to_vec();
        let text = format!(
            "SELECT account_id, detail FROM rifts_host_presences WHERE observed_at >= ? AND expires_at > ? \
             AND account_id IN ({})",
            marks(wanted.len())
        );
        let mut args = binds![observed_after, now];
        args.extend(wanted.iter().map(Bind::arg));
        self.read(move |conn| {
            let mut found: Vec<DetailRow> = Statement::new(&text, args)?.rows(conn)?;
            let mut ordered = Vec::with_capacity(found.len());
            for account in &wanted {
                let Some(index) = found.iter().position(|row| row.account_id == *account) else {
                    continue;
                };
                let row = found.swap_remove(index);
                ordered.push(serde_json::from_str(&row.detail).map_err(json_error)?);
            }
            Ok(ordered)
        })
        .await
    }

    /// Sessions matching `filter` (ending in `AND`) in one of `states`.
    async fn count_in(
        &self,
        filter: &'static str,
        value: &str,
        states: &'static [HuntState],
    ) -> Result<i64, RepoError> {
        let (clause, state_args) = in_states(states);
        let text = format!("SELECT COUNT(*) AS n FROM rifts_hunt_sessions WHERE {filter} ({clause})");
        let mut args = binds![value];
        args.extend(state_args);
        self.read(move |conn| Statement::new(&text, args)?.number(conn)).await
    }

    /// A draining session gave its key up.
    pub async fn count_active_host_sessions(&self, account_id: &str) -> Result<i64, RepoError> {
        let filter = "host_account_id = ? AND active_party_key IS NOT NULL AND";
        self.count_in(filter, account_id, LIVE_HOST_STATES).await
    }

    /// Hunts holding a party key in the realm, in the states that occupy a server.
    pub async fn count_active_realm_servers(&self, realm_id: &str) -> Result<i64, RepoError> {
        let filter = "realm_id = ? AND active_party_key IS NOT NULL AND";
        self.count_in(filter, realm_id, REALM_SERVER_STATES).await
    }

    /// Draining included: its server still runs on the host's PC.
    pub async fn count_running_realm_servers(&self, realm_id: &str) -> Result<i64, RepoError> {
        let (clause, state_args) = in_states(REALM_SERVER_STATES);
        let text = format!(
            "SELECT COUNT(*) AS n FROM rifts_hunt_sessions WHERE realm_id = ? \
             AND ((active_party_key IS NOT NULL AND {clause}) OR state = ?)"
        );
        let mut args = binds![realm_id];
        args.extend(state_args);
        args.extend(binds![Draining]);
        self.read(move |conn| Statement::new(&text, args)?.number(conn)).await
    }

    /// Never hubs; at most 64.
    pub async fn list_live_realm_hunts(&self, realm_id: &str, host: &str) -> Result<Vec<HuntSession>, RepoError> {
        let (clause, state_args) = in_states(REALM_SERVER_STATES);
        let tail = format!(
            "WHERE realm_id = ? AND host_account_id = ? AND session_kind IS NULL \
             AND active_party_key IS NOT NULL AND {clause} ORDER BY created_at, session_id LIMIT 64"
        );
        let mut args = binds![realm_id, host];
        args.extend(state_args);
        self.read(move |conn| sessions(conn, &tail, args)).await
    }

    /// The host's reliability record, if it has one.
    pub async fn find_host_reliability(&self, account_id: &str) -> Result<Option<HostReliabilityRecord>, RepoError> {
        let account = String::from(account_id);
        self.read(move |conn| reliability(conn, &account)).await
    }

    /// The offer, if it exists.
    pub async fn find_host_offer(&self, offer_id: &str) -> Result<Option<HostOfferRecord>, RepoError> {
        let offered = String::from(offer_id);
        self.read(move |conn| offer(conn, &offered)).await
    }

    /// The generation's offers, by rank.
    pub async fn list_host_offers(&self, session_id: &str, generation: i64) -> Result<Vec<HostOfferRecord>, RepoError> {
        let text = format!(
            "SELECT {} FROM {OFFERS} WHERE session_id = ? AND generation = ? ORDER BY rank, offer_id",
            columns(OfferRow::COLUMNS)
        );
        let args = binds![session_id, generation];
        self.read(move |conn| {
            let rows: Vec<OfferRow> = Statement::new(&text, args)?.rows(conn)?;
            rows.into_iter().map(OfferRow::record).collect()
        })
        .await
    }

    /// Newest first.
    pub async fn list_recent_participant_observations(
        &self,
        session_id: &str,
        generation: i64,
        observed_after: UnixMillis,
    ) -> Result<Vec<ObservationRecord>, RepoError> {
        let text = format!(
            "SELECT {} FROM {OBSERVATIONS} WHERE session_id = ? \
             AND generation = ? AND observed_at >= ? ORDER BY observed_at DESC, observation_id",
            columns(ObservationRow::COLUMNS)
        );
        let args = binds![session_id, generation, observed_after];
        self.read(move |conn| {
            let rows: Vec<ObservationRow> = Statement::new(&text, args)?.rows(conn)?;
            rows.into_iter().map(ObservationRow::record).collect()
        })
        .await
    }

    /// Control messages and host presences past their time. Nothing calls this yet (spec-rd27-rifts §7 risk 3).
    pub async fn purge_expired(&self, now: UnixMillis) -> Result<usize, RepoError> {
        self.db
            .write(&Actor::System("realms-purge"), move |conn| {
                let mut purged = 0;
                for table in [CONTROL, "rifts_host_presences"] {
                    let text = format!("DELETE FROM {table} WHERE expires_at <= ?");
                    purged += Statement::new(&text, binds![now])?.run(conn)?;
                }
                Ok(purged)
            })
            .await
    }
}
