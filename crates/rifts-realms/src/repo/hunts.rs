// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Hunt and hub sessions with their roster and transitions, host capabilities, the launcher's control channel, host
//! presences, offers, reliability and participant observations. `HuntTx` is one write transaction over all of them,
//! so a lifecycle change commits every step or none.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Double, Nullable, Text};
use module_api::NodeId;
use rifts_seam::{Actor, UnixMillis};
use serde_json::Value;
use store::AnyConnection;

use super::hub::{HubPresenceRecord, end_generation};
use super::{Boxed, Count, corrupt, limit, placeholders, query};
use crate::db::{RealmsDb, RepoError, store_actor};
use crate::hunt::{
    AdmissionState, ControlMessageRecord, HostCapabilityRecord, HostOfferRecord, HostPresenceRecord,
    HostReliabilityRecord, HuntSession, HuntState, HuntTransition, ObservationRecord, OfferStatus, Participant,
    ReliabilityIncrements, ScoreBreakdown, hub_kind_name, parse_scope, scope_name,
};
use crate::ids::{global_session_id, random_uuid};

use HuntState as H;

/// A host's sessions that hold its party key and keep its runtime busy.
pub const LIVE_HOST_STATES: &[HuntState] = &[
    H::HostOffered,
    H::Starting,
    H::HostReady,
    H::Admitting,
    H::Running,
    H::Suspect,
];

/// A realm's servers: its hubs, Training and hunts.
const REALM_SERVER_STATES: &[HuntState] = &[
    H::HostSelecting,
    H::HostOffered,
    H::Starting,
    H::HostReady,
    H::Admitting,
    H::Running,
    H::Suspect,
];

const ROSTER_LIVE_STATES: &[HuntState] = &[H::HostReady, H::Admitting, H::Running];

const RENEWABLE_STATES: &[HuntState] = &[H::HostReady, H::Admitting, H::Running, H::Draining, H::Suspect];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostStartup {
    pub session_id: String,
    pub host_account_id: String,
    pub bridge_instance_id: String,
    pub metadata_nonce: String,
    pub startup_deadline: UnixMillis,
    pub server_udp_port: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseRenewal {
    pub session_id: String,
    pub generation: i64,
    pub host_steam_id: String,
    pub bridge_instance_id: String,
    pub lease_sequence: i64,
    pub heartbeat_at: UnixMillis,
    pub lease_expires_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostReadyMark {
    pub session_id: String,
    pub generation: i64,
    pub host_steam_id: String,
    pub bridge_instance_id: String,
    pub lobby_id: String,
    pub virtual_port: i64,
    pub lease_expires_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewControlMessage {
    pub account_id: String,
    pub message_type: String,
    pub session_id: Option<String>,
    pub generation: Option<i64>,
    pub payload: Value,
    pub created_at: UnixMillis,
    pub expires_at: UnixMillis,
}

fn states_clause(column: &str, states: &[HuntState]) -> String {
    format!("{column} IN ({})", placeholders(states.len()))
}

fn bind_states(mut q: Boxed, states: &[HuntState]) -> Boxed {
    for state in states {
        q = q.bind::<Text, _>(state.as_str());
    }
    q
}

fn count(conn: &mut AnyConnection, text: &str, bind: impl FnOnce(Boxed) -> Boxed) -> Result<i64, RepoError> {
    Ok(bind(query(text)?)
        .load::<Count>(conn)?
        .into_iter()
        .next()
        .map_or(0, |c| c.n))
}

#[derive(QueryableByName)]
struct SessionRow {
    #[diesel(sql_type = Text)]
    session_id: String,
    #[diesel(sql_type = Text)]
    detail: String,
}

#[derive(QueryableByName)]
struct RosterRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    steam_id: String,
    #[diesel(sql_type = Text)]
    candidate_id: String,
    #[diesel(sql_type = BigInt)]
    reconnect_epoch: i64,
    #[diesel(sql_type = Text)]
    admission_state: String,
    #[diesel(sql_type = Nullable<Text>)]
    local_bridge_instance_id: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    local_bridge_port: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    local_bridge_ready_at: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    admitted_at: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    disconnected_at: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    rejoin_deadline: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    returned_to_hub_at: Option<i64>,
}

const ROSTER_COLUMNS: [&str; 14] = [
    "session_id",
    "account_id",
    "seq",
    "steam_id",
    "candidate_id",
    "reconnect_epoch",
    "admission_state",
    "local_bridge_instance_id",
    "local_bridge_port",
    "local_bridge_ready_at",
    "admitted_at",
    "disconnected_at",
    "rejoin_deadline",
    "returned_to_hub_at",
];

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

fn assemble(conn: &mut AnyConnection, row: SessionRow) -> Result<HuntSession, RepoError> {
    let mut hunt: HuntSession =
        serde_json::from_str(&row.detail).map_err(|e| RepoError::Corrupt(format!("hunt {}: {e}", row.session_id)))?;
    let columns = ROSTER_COLUMNS[1..]
        .iter()
        .filter(|c| **c != "seq")
        .copied()
        .collect::<Vec<_>>();
    let roster = query(&format!(
        "SELECT {} FROM rifts_hunt_roster WHERE session_id = ? ORDER BY seq",
        columns.join(", ")
    ))?
    .bind::<Text, _>(row.session_id)
    .load::<RosterRow>(conn)?;
    hunt.roster = roster
        .into_iter()
        .map(|r| {
            Ok(Participant {
                admission_state: AdmissionState::parse(&r.admission_state)
                    .ok_or_else(|| corrupt("admission state", &r.admission_state))?,
                account_id: r.account_id,
                steam_id: r.steam_id,
                candidate_id: r.candidate_id,
                reconnect_epoch: r.reconnect_epoch,
                local_bridge_instance_id: r.local_bridge_instance_id,
                local_bridge_port: r.local_bridge_port,
                local_bridge_ready_at: r.local_bridge_ready_at.map(UnixMillis),
                admitted_at: r.admitted_at.map(UnixMillis),
                disconnected_at: r.disconnected_at.map(UnixMillis),
                rejoin_deadline: r.rejoin_deadline.map(UnixMillis),
                returned_to_hub_at: r.returned_to_hub_at.map(UnixMillis),
            })
        })
        .collect::<Result<_, RepoError>>()?;
    Ok(hunt)
}

fn sessions(
    conn: &mut AnyConnection,
    tail: &str,
    bind: impl FnOnce(Boxed) -> Boxed,
) -> Result<Vec<HuntSession>, RepoError> {
    let rows = bind(query(&format!(
        "SELECT session_id, detail FROM rifts_hunt_sessions {tail}"
    ))?)
    .load::<SessionRow>(conn)?;
    rows.into_iter().map(|row| assemble(conn, row)).collect()
}

fn load(conn: &mut AnyConnection, session_id: &str) -> Result<Option<HuntSession>, RepoError> {
    let id = session_id.to_owned();
    Ok(sessions(conn, "WHERE session_id = ?", |q| q.bind::<Text, _>(id))?
        .into_iter()
        .next())
}

fn session_exists(conn: &mut AnyConnection, session_id: &str) -> Result<bool, RepoError> {
    let id = session_id.to_owned();
    Ok(count(
        conn,
        "SELECT COUNT(*) AS n FROM rifts_hunt_sessions WHERE session_id = ?",
        |q| q.bind::<Text, _>(id),
    )? > 0)
}

/// Whether a session other than `except` holds `key`.
fn key_taken(conn: &mut AnyConnection, key: &str, except: Option<&str>) -> Result<bool, RepoError> {
    let key = key.to_owned();
    let except = except.unwrap_or("").to_owned();
    Ok(count(
        conn,
        "SELECT COUNT(*) AS n FROM rifts_hunt_sessions WHERE active_party_key = ? AND session_id <> ?",
        |q| q.bind::<Text, _>(key).bind::<Text, _>(except),
    )? > 0)
}

/// Upserts the row (its global id fixed by the first save) and syncs the roster rows to the record's.
fn save(conn: &mut AnyConnection, node: &NodeId, hunt: &HuntSession) -> Result<(), RepoError> {
    let detail =
        serde_json::to_string(hunt).map_err(|e| RepoError::Corrupt(format!("hunt {}: {e}", hunt.session_id)))?;
    let global = global_session_id(&hunt.session_id, hunt.is_realm_hub(), node).map(|id| id.to_string());
    let update: Vec<&str> = SESSION_COLUMNS[2..].to_vec();
    diesel::sql_query(store::portable::upsert(
        "rifts_hunt_sessions",
        &SESSION_COLUMNS,
        &["session_id"],
        &update,
    ))
    .into_boxed::<store::MultiBackend>()
    .bind::<Text, _>(hunt.session_id.clone())
    .bind::<Nullable<Text>, _>(global)
    .bind::<BigInt, _>(hunt.generation)
    .bind::<Text, _>(hunt.state.as_str())
    .bind::<BigInt, _>(hunt.state_version)
    .bind::<Text, _>(hunt.party_id.clone())
    .bind::<Nullable<Text>, _>(hunt.active_party_key.clone())
    .bind::<Text, _>(hunt.host_account_id.clone())
    .bind::<Nullable<Text>, _>(hunt.hub_kind.map(|_| "REALM_HUB"))
    .bind::<Nullable<Text>, _>(hunt.hub_kind.map(hub_kind_name))
    .bind::<Nullable<Text>, _>(hunt.realm_id.clone())
    .bind::<Nullable<BigInt>, _>(hunt.lease_expires_at.map(|at| at.0))
    .bind::<BigInt, _>(hunt.created_at.0)
    .bind::<BigInt, _>(hunt.expires_at.0)
    .bind::<Text, _>(detail)
    .execute(conn)?;

    let mut prune = query(&format!(
        "DELETE FROM rifts_hunt_roster WHERE session_id = ?{}",
        if hunt.roster.is_empty() {
            String::new()
        } else {
            format!(" AND account_id NOT IN ({})", placeholders(hunt.roster.len()))
        }
    ))?
    .bind::<Text, _>(hunt.session_id.clone());
    for participant in &hunt.roster {
        prune = prune.bind::<Text, _>(participant.account_id.clone());
    }
    prune.execute(conn)?;

    let upsert = store::portable::upsert(
        "rifts_hunt_roster",
        &ROSTER_COLUMNS,
        &ROSTER_COLUMNS[..2],
        &ROSTER_COLUMNS[2..],
    );
    for (seq, p) in (1i64..).zip(&hunt.roster) {
        diesel::sql_query(upsert.clone())
            .into_boxed::<store::MultiBackend>()
            .bind::<Text, _>(hunt.session_id.clone())
            .bind::<Text, _>(p.account_id.clone())
            .bind::<BigInt, _>(seq)
            .bind::<Text, _>(p.steam_id.clone())
            .bind::<Text, _>(p.candidate_id.clone())
            .bind::<BigInt, _>(p.reconnect_epoch)
            .bind::<Text, _>(p.admission_state.as_str())
            .bind::<Nullable<Text>, _>(p.local_bridge_instance_id.clone())
            .bind::<Nullable<BigInt>, _>(p.local_bridge_port)
            .bind::<Nullable<BigInt>, _>(p.local_bridge_ready_at.map(|at| at.0))
            .bind::<Nullable<BigInt>, _>(p.admitted_at.map(|at| at.0))
            .bind::<Nullable<BigInt>, _>(p.disconnected_at.map(|at| at.0))
            .bind::<Nullable<BigInt>, _>(p.rejoin_deadline.map(|at| at.0))
            .bind::<Nullable<BigInt>, _>(p.returned_to_hub_at.map(|at| at.0))
            .execute(conn)?;
    }
    Ok(())
}

#[derive(QueryableByName)]
struct TransitionRow {
    #[diesel(sql_type = Text)]
    transition_id: String,
    #[diesel(sql_type = Text)]
    session_id: String,
    #[diesel(sql_type = BigInt)]
    generation: i64,
    #[diesel(sql_type = Text)]
    from_state: String,
    #[diesel(sql_type = Text)]
    to_state: String,
    #[diesel(sql_type = BigInt)]
    state_version: i64,
    #[diesel(sql_type = Text)]
    reason: String,
    #[diesel(sql_type = Text)]
    actor_type: String,
    #[diesel(sql_type = Nullable<Text>)]
    actor_account_id: Option<String>,
    #[diesel(sql_type = BigInt)]
    at: i64,
}

fn hunt_state(text: &str) -> Result<HuntState, RepoError> {
    HuntState::parse(text).ok_or_else(|| corrupt("state", text))
}

impl TransitionRow {
    fn into_record(self) -> Result<HuntTransition, RepoError> {
        Ok(HuntTransition {
            from_state: hunt_state(&self.from_state)?,
            to_state: hunt_state(&self.to_state)?,
            transition_id: self.transition_id,
            session_id: self.session_id,
            generation: self.generation,
            state_version: self.state_version,
            reason: self.reason,
            actor_type: self.actor_type,
            actor_account_id: self.actor_account_id,
            at: UnixMillis(self.at),
        })
    }
}

const CAPABILITY_COLUMNS: &str = "capability_id, secret_digest, session_id, generation, host_account_id, \
     host_steam_id, bridge_instance_id, scopes, issued_at, expires_at, revoked_at, last_sequence";

#[derive(QueryableByName)]
struct CapabilityRow {
    #[diesel(sql_type = Text)]
    capability_id: String,
    #[diesel(sql_type = Text)]
    secret_digest: String,
    #[diesel(sql_type = Text)]
    session_id: String,
    #[diesel(sql_type = BigInt)]
    generation: i64,
    #[diesel(sql_type = Text)]
    host_account_id: String,
    #[diesel(sql_type = Text)]
    host_steam_id: String,
    #[diesel(sql_type = Text)]
    bridge_instance_id: String,
    #[diesel(sql_type = Text)]
    scopes: String,
    #[diesel(sql_type = BigInt)]
    issued_at: i64,
    #[diesel(sql_type = BigInt)]
    expires_at: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    revoked_at: Option<i64>,
    #[diesel(sql_type = BigInt)]
    last_sequence: i64,
}

impl CapabilityRow {
    fn into_record(self) -> Result<HostCapabilityRecord, RepoError> {
        let scopes = self
            .scopes
            .split(',')
            .filter(|piece| !piece.is_empty())
            .map(|piece| parse_scope(piece).ok_or_else(|| corrupt("capability scope", piece)))
            .collect::<Result<_, RepoError>>()?;
        Ok(HostCapabilityRecord {
            capability_id: self.capability_id,
            secret_digest: self.secret_digest,
            session_id: self.session_id,
            generation: self.generation,
            host_account_id: self.host_account_id,
            host_steam_id: self.host_steam_id,
            bridge_instance_id: self.bridge_instance_id,
            scopes,
            issued_at: UnixMillis(self.issued_at),
            expires_at: UnixMillis(self.expires_at),
            revoked_at: self.revoked_at.map(UnixMillis),
            last_sequence: self.last_sequence,
        })
    }
}

const CONTROL_COLUMNS: &str = "message_id, account_id, sequence, message_type, session_id, generation, payload, \
     created_at, expires_at, acknowledged_at";

#[derive(QueryableByName)]
struct ControlRow {
    #[diesel(sql_type = Text)]
    message_id: String,
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = BigInt)]
    sequence: i64,
    #[diesel(sql_type = Text)]
    message_type: String,
    #[diesel(sql_type = Nullable<Text>)]
    session_id: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    generation: Option<i64>,
    #[diesel(sql_type = Text)]
    payload: String,
    #[diesel(sql_type = BigInt)]
    created_at: i64,
    #[diesel(sql_type = BigInt)]
    expires_at: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    acknowledged_at: Option<i64>,
}

fn json_error(error: serde_json::Error) -> RepoError {
    RepoError::Corrupt(error.to_string())
}

impl ControlRow {
    fn into_record(self) -> Result<ControlMessageRecord, RepoError> {
        Ok(ControlMessageRecord {
            payload: serde_json::from_str(&self.payload).map_err(json_error)?,
            message_id: self.message_id,
            account_id: self.account_id,
            sequence: self.sequence,
            message_type: self.message_type,
            session_id: self.session_id,
            generation: self.generation,
            created_at: UnixMillis(self.created_at),
            expires_at: UnixMillis(self.expires_at),
            acknowledged_at: self.acknowledged_at.map(UnixMillis),
        })
    }
}

#[derive(QueryableByName)]
struct DetailRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    detail: String,
}

const OFFER_COLUMNS: &str = "offer_id, session_id, generation, account_id, steam_id, rank, score, score_breakdown, \
     status, offered_at, expires_at, responded_at, response_reason";

#[derive(QueryableByName)]
struct OfferRow {
    #[diesel(sql_type = Text)]
    offer_id: String,
    #[diesel(sql_type = Text)]
    session_id: String,
    #[diesel(sql_type = BigInt)]
    generation: i64,
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    steam_id: String,
    #[diesel(sql_type = BigInt)]
    rank: i64,
    #[diesel(sql_type = Double)]
    score: f64,
    #[diesel(sql_type = Text)]
    score_breakdown: String,
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = BigInt)]
    offered_at: i64,
    #[diesel(sql_type = BigInt)]
    expires_at: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    responded_at: Option<i64>,
    #[diesel(sql_type = Nullable<Text>)]
    response_reason: Option<String>,
}

impl OfferRow {
    fn into_record(self) -> Result<HostOfferRecord, RepoError> {
        let breakdown: ScoreBreakdown = serde_json::from_str(&self.score_breakdown).map_err(json_error)?;
        Ok(HostOfferRecord {
            status: OfferStatus::parse(&self.status).ok_or_else(|| corrupt("offer status", &self.status))?,
            offer_id: self.offer_id,
            session_id: self.session_id,
            generation: self.generation,
            account_id: self.account_id,
            steam_id: self.steam_id,
            rank: self.rank,
            score: self.score,
            score_breakdown: breakdown,
            offered_at: UnixMillis(self.offered_at),
            expires_at: UnixMillis(self.expires_at),
            responded_at: self.responded_at.map(UnixMillis),
            response_reason: self.response_reason,
        })
    }
}

const RELIABILITY_COLUMNS: [&str; 10] = [
    "account_id",
    "offers_accepted",
    "offers_declined",
    "offers_expired",
    "starts_succeeded",
    "starts_failed",
    "clean_completions",
    "unexpected_failures",
    "cooldown_until",
    "updated_at",
];

#[derive(QueryableByName)]
struct ReliabilityRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = BigInt)]
    offers_accepted: i64,
    #[diesel(sql_type = BigInt)]
    offers_declined: i64,
    #[diesel(sql_type = BigInt)]
    offers_expired: i64,
    #[diesel(sql_type = BigInt)]
    starts_succeeded: i64,
    #[diesel(sql_type = BigInt)]
    starts_failed: i64,
    #[diesel(sql_type = BigInt)]
    clean_completions: i64,
    #[diesel(sql_type = BigInt)]
    unexpected_failures: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    cooldown_until: Option<i64>,
    #[diesel(sql_type = BigInt)]
    updated_at: i64,
}

impl From<ReliabilityRow> for HostReliabilityRecord {
    fn from(row: ReliabilityRow) -> Self {
        Self {
            account_id: row.account_id,
            offers_accepted: row.offers_accepted,
            offers_declined: row.offers_declined,
            offers_expired: row.offers_expired,
            starts_succeeded: row.starts_succeeded,
            starts_failed: row.starts_failed,
            clean_completions: row.clean_completions,
            unexpected_failures: row.unexpected_failures,
            cooldown_until: row.cooldown_until.map(UnixMillis),
            updated_at: UnixMillis(row.updated_at),
        }
    }
}

const OBSERVATION_COLUMNS: &str = "observation_id, session_id, generation, account_id, steam_id, host_reachable, \
     local_bridge_alive, metrics_available, last_packet_age_ms, route_kind, ping_ms, jitter_ms, \
     packet_loss_percent, observed_at";

#[derive(QueryableByName)]
struct ObservationRow {
    #[diesel(sql_type = Text)]
    observation_id: String,
    #[diesel(sql_type = Text)]
    session_id: String,
    #[diesel(sql_type = BigInt)]
    generation: i64,
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    steam_id: String,
    #[diesel(sql_type = Bool)]
    host_reachable: bool,
    #[diesel(sql_type = Bool)]
    local_bridge_alive: bool,
    #[diesel(sql_type = Bool)]
    metrics_available: bool,
    #[diesel(sql_type = Nullable<BigInt>)]
    last_packet_age_ms: Option<i64>,
    #[diesel(sql_type = Text)]
    route_kind: String,
    #[diesel(sql_type = Nullable<BigInt>)]
    ping_ms: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    jitter_ms: Option<i64>,
    #[diesel(sql_type = Nullable<Double>)]
    packet_loss_percent: Option<f64>,
    #[diesel(sql_type = BigInt)]
    observed_at: i64,
}

impl From<ObservationRow> for ObservationRecord {
    fn from(row: ObservationRow) -> Self {
        Self {
            observation_id: row.observation_id,
            session_id: row.session_id,
            generation: row.generation,
            account_id: row.account_id,
            steam_id: row.steam_id,
            host_reachable: row.host_reachable,
            local_bridge_alive: row.local_bridge_alive,
            metrics_available: row.metrics_available,
            last_packet_age_ms: row.last_packet_age_ms,
            route_kind: row.route_kind,
            ping_ms: row.ping_ms,
            jitter_ms: row.jitter_ms,
            packet_loss_percent: row.packet_loss_percent,
            observed_at: UnixMillis(row.observed_at),
        }
    }
}

fn reliability(conn: &mut AnyConnection, account_id: &str) -> Result<Option<HostReliabilityRecord>, RepoError> {
    Ok(query(&format!(
        "SELECT {} FROM rifts_host_reliability WHERE account_id = ?",
        RELIABILITY_COLUMNS.join(", ")
    ))?
    .bind::<Text, _>(account_id.to_owned())
    .load::<ReliabilityRow>(conn)?
    .into_iter()
    .next()
    .map(HostReliabilityRecord::from))
}

fn offer(conn: &mut AnyConnection, offer_id: &str) -> Result<Option<HostOfferRecord>, RepoError> {
    query(&format!(
        "SELECT {OFFER_COLUMNS} FROM rifts_host_offers WHERE offer_id = ?"
    ))?
    .bind::<Text, _>(offer_id.to_owned())
    .load::<OfferRow>(conn)?
    .into_iter()
    .next()
    .map(OfferRow::into_record)
    .transpose()
}

fn latest_sequence(conn: &mut AnyConnection, account_id: &str) -> Result<i64, RepoError> {
    let account = account_id.to_owned();
    count(
        conn,
        "SELECT next_sequence AS n FROM rifts_control_sequences WHERE account_id = ?",
        |q| q.bind::<Text, _>(account),
    )
}

/// One write transaction over the hunt tables (Node's `WithP2PTransaction`).
pub struct HuntTx<'a> {
    conn: &'a mut AnyConnection,
    node: &'a NodeId,
}

impl HuntTx<'_> {
    pub fn find_hunt(&mut self, session_id: &str) -> Result<Option<HuntSession>, RepoError> {
        load(self.conn, session_id)
    }

    pub fn create_hunt(&mut self, session: &HuntSession) -> Result<(), RepoError> {
        if session_exists(self.conn, &session.session_id)? {
            return Err(RepoError::Duplicate("session_id"));
        }
        if let Some(key) = &session.active_party_key
            && key_taken(self.conn, key, None)?
        {
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
        if bump {
            hunt.state_version += 1;
        }
        if let Some(key) = &hunt.active_party_key
            && key_taken(self.conn, key, Some(&hunt.session_id))?
        {
            return Err(RepoError::Duplicate("active_party_key"));
        }
        save(self.conn, self.node, &hunt)?;
        Ok(Some(hunt))
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
            let holds = hunt.generation == generation
                && expected_states.contains(&hunt.state)
                && hunt.state_version == expected_state_version;
            if holds {
                change(hunt);
            }
            holds
        })
    }

    pub fn append_transition(&mut self, t: &HuntTransition) -> Result<(), RepoError> {
        let id = if t.transition_id.is_empty() {
            random_uuid()
        } else {
            t.transition_id.clone()
        };
        query(
            "INSERT INTO rifts_hunt_transitions (transition_id, session_id, generation, from_state, to_state, \
             state_version, reason, actor_type, actor_account_id, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )?
        .bind::<Text, _>(id)
        .bind::<Text, _>(t.session_id.clone())
        .bind::<BigInt, _>(t.generation)
        .bind::<Text, _>(t.from_state.as_str())
        .bind::<Text, _>(t.to_state.as_str())
        .bind::<BigInt, _>(t.state_version)
        .bind::<Text, _>(t.reason.clone())
        .bind::<Text, _>(t.actor_type.clone())
        .bind::<Nullable<Text>, _>(t.actor_account_id.clone())
        .bind::<BigInt, _>(t.at.0)
        .execute(self.conn)?;
        Ok(())
    }

    /// HOST_OFFERED to STARTING for that host of an unexpired session; no generation check.
    pub fn begin_host_startup(&mut self, s: &HostStartup, now: UnixMillis) -> Result<Option<HuntSession>, RepoError> {
        self.update(&s.session_id, true, |hunt| {
            if hunt.host_account_id != s.host_account_id || hunt.state != H::HostOffered || hunt.expires_at <= now {
                return false;
            }
            hunt.state = H::Starting;
            hunt.bridge_instance_id = s.bridge_instance_id.clone();
            hunt.metadata_nonce = Some(s.metadata_nonce.clone());
            hunt.startup_deadline = Some(s.startup_deadline);
            if s.server_udp_port.is_some() {
                hunt.server_udp_port = s.server_udp_port;
            }
            hunt.lobby_id = None;
            hunt.lease_expires_at = None;
            hunt.suspect_since = None;
            hunt.suspect_deadline = None;
            hunt.pre_suspect_state = None;
            true
        })
    }

    /// Only a strictly newer lease sequence from the session's current host and bridge.
    pub fn renew_host_lease(&mut self, r: &LeaseRenewal) -> Result<Option<HuntSession>, RepoError> {
        self.update(&r.session_id, true, |hunt| {
            let newer = hunt.last_lease_sequence.is_none_or(|last| last < r.lease_sequence);
            if hunt.generation != r.generation
                || hunt.host_steam_id != r.host_steam_id
                || hunt.bridge_instance_id != r.bridge_instance_id
                || !RENEWABLE_STATES.contains(&hunt.state)
                || !newer
            {
                return false;
            }
            hunt.last_lease_sequence = Some(r.lease_sequence);
            hunt.last_heartbeat_at = Some(r.heartbeat_at);
            hunt.lease_expires_at = Some(r.lease_expires_at);
            true
        })
    }

    pub fn mark_host_ready(&mut self, m: &HostReadyMark, now: UnixMillis) -> Result<Option<HuntSession>, RepoError> {
        self.update(&m.session_id, true, |hunt| {
            if hunt.generation != m.generation
                || hunt.host_steam_id != m.host_steam_id
                || hunt.bridge_instance_id != m.bridge_instance_id
                || hunt.virtual_port != m.virtual_port
                || !matches!(hunt.state, H::Starting | H::HostReady)
            {
                return false;
            }
            hunt.state = H::HostReady;
            hunt.lobby_id = Some(m.lobby_id.clone());
            hunt.lease_expires_at = Some(m.lease_expires_at);
            hunt.last_heartbeat_at = Some(now);
            hunt.last_lease_sequence = Some(0);
            true
        })
    }

    /// One participant's change: generation and state set hold, `pick` finds the entry, `edit` may still refuse.
    fn participant_change(
        &mut self,
        session_id: &str,
        generation: i64,
        states: &[HuntState],
        bump: bool,
        pick: impl Fn(&Participant) -> bool,
        edit: impl FnOnce(&mut Participant) -> bool,
    ) -> Result<Option<HuntSession>, RepoError> {
        self.update(session_id, bump, |hunt| {
            if hunt.generation != generation || !states.contains(&hunt.state) {
                return false;
            }
            hunt.roster.iter_mut().find(|p| pick(p)).is_some_and(edit)
        })
    }

    /// Matched by account and peer id; no version bump.
    #[allow(clippy::too_many_arguments)]
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
        self.participant_change(
            session_id,
            generation,
            ROSTER_LIVE_STATES,
            false,
            |p| p.account_id == account_id && p.steam_id == steam_id,
            |p| {
                p.local_bridge_instance_id = Some(bridge_instance_id.to_owned());
                p.local_bridge_port = Some(local_bridge_port);
                p.local_bridge_ready_at = Some(ready_at);
                true
            },
        )
    }

    pub fn mark_participant_admitted(
        &mut self,
        session_id: &str,
        generation: i64,
        account_id: &str,
        admitted_at: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        self.participant_change(
            session_id,
            generation,
            ROSTER_LIVE_STATES,
            true,
            |p| p.account_id == account_id,
            |p| {
                p.admission_state = AdmissionState::Admitted;
                p.admitted_at = Some(admitted_at);
                p.disconnected_at = None;
                p.rejoin_deadline = None;
                true
            },
        )
    }

    /// Refused past the reconnect budget, or for a disconnected participant whose rejoin window has closed.
    #[allow(clippy::too_many_arguments)]
    pub fn reserve_participant_ticket(
        &mut self,
        session_id: &str,
        generation: i64,
        account_id: &str,
        now: UnixMillis,
        rejoin_deadline: UnixMillis,
        max_reconnects: i64,
    ) -> Result<Option<HuntSession>, RepoError> {
        self.participant_change(
            session_id,
            generation,
            ROSTER_LIVE_STATES,
            false,
            |p| p.account_id == account_id,
            |p| {
                let window_closed = p.disconnected_at.is_some() && p.rejoin_deadline.is_none_or(|d| d <= now);
                if p.reconnect_epoch >= max_reconnects || window_closed {
                    return false;
                }
                p.reconnect_epoch += 1;
                p.admission_state = AdmissionState::Issued;
                p.rejoin_deadline = Some(rejoin_deadline);
                true
            },
        )
    }

    pub fn mark_participant_disconnected(
        &mut self,
        session_id: &str,
        generation: i64,
        account_id: &str,
        disconnected_at: UnixMillis,
        rejoin_deadline: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        self.participant_change(
            session_id,
            generation,
            RENEWABLE_STATES,
            false,
            |p| p.account_id == account_id,
            |p| {
                p.admission_state = AdmissionState::Lost;
                p.disconnected_at = Some(disconnected_at);
                p.rejoin_deadline = Some(rejoin_deadline);
                true
            },
        )
    }

    pub fn create_capability(&mut self, c: &HostCapabilityRecord) -> Result<(), RepoError> {
        let digest = c.secret_digest.clone();
        if count(
            self.conn,
            "SELECT COUNT(*) AS n FROM rifts_host_capabilities WHERE secret_digest = ?",
            |q| q.bind::<Text, _>(digest),
        )? > 0
        {
            return Err(RepoError::Duplicate("secret_digest"));
        }
        let scopes = c.scopes.iter().map(|s| scope_name(*s)).collect::<Vec<_>>().join(",");
        query(&format!(
            "INSERT INTO rifts_host_capabilities ({CAPABILITY_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))?
        .bind::<Text, _>(c.capability_id.clone())
        .bind::<Text, _>(c.secret_digest.clone())
        .bind::<Text, _>(c.session_id.clone())
        .bind::<BigInt, _>(c.generation)
        .bind::<Text, _>(c.host_account_id.clone())
        .bind::<Text, _>(c.host_steam_id.clone())
        .bind::<Text, _>(c.bridge_instance_id.clone())
        .bind::<Text, _>(scopes)
        .bind::<BigInt, _>(c.issued_at.0)
        .bind::<BigInt, _>(c.expires_at.0)
        .bind::<Nullable<BigInt>, _>(c.revoked_at.map(|at| at.0))
        .bind::<BigInt, _>(c.last_sequence)
        .execute(self.conn)?;
        Ok(())
    }

    pub fn advance_capability_sequence(&mut self, capability_id: &str, sequence: i64) -> Result<bool, RepoError> {
        Ok(query(
            "UPDATE rifts_host_capabilities SET last_sequence = ? \
             WHERE capability_id = ? AND revoked_at IS NULL AND last_sequence < ?",
        )?
        .bind::<BigInt, _>(sequence)
        .bind::<Text, _>(capability_id.to_owned())
        .bind::<BigInt, _>(sequence)
        .execute(self.conn)?
            == 1)
    }

    /// An earlier revocation time stays.
    pub fn revoke_generation(&mut self, session_id: &str, generation: i64, at: UnixMillis) -> Result<(), RepoError> {
        query(
            "UPDATE rifts_host_capabilities SET revoked_at = ? \
             WHERE session_id = ? AND generation = ? AND revoked_at IS NULL",
        )?
        .bind::<Nullable<BigInt>, _>(Some(at.0))
        .bind::<Text, _>(session_id.to_owned())
        .bind::<BigInt, _>(generation)
        .execute(self.conn)?;
        Ok(())
    }

    /// Sequences outlive the messages' purge, so they never repeat.
    pub fn enqueue_control(&mut self, m: &NewControlMessage) -> Result<ControlMessageRecord, RepoError> {
        let sequence = latest_sequence(self.conn, &m.account_id)? + 1;
        diesel::sql_query(store::portable::upsert(
            "rifts_control_sequences",
            &["account_id", "next_sequence"],
            &["account_id"],
            &["next_sequence"],
        ))
        .bind::<Text, _>(m.account_id.clone())
        .bind::<BigInt, _>(sequence)
        .execute(self.conn)?;
        let record = ControlMessageRecord {
            message_id: random_uuid(),
            account_id: m.account_id.clone(),
            sequence,
            message_type: m.message_type.clone(),
            session_id: m.session_id.clone(),
            generation: m.generation,
            payload: m.payload.clone(),
            created_at: m.created_at,
            expires_at: m.expires_at,
            acknowledged_at: None,
        };
        let payload = serde_json::to_string(&record.payload).map_err(json_error)?;
        query(&format!(
            "INSERT INTO rifts_control_messages ({CONTROL_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))?
        .bind::<Text, _>(record.message_id.clone())
        .bind::<Text, _>(record.account_id.clone())
        .bind::<BigInt, _>(record.sequence)
        .bind::<Text, _>(record.message_type.clone())
        .bind::<Nullable<Text>, _>(record.session_id.clone())
        .bind::<Nullable<BigInt>, _>(record.generation)
        .bind::<Text, _>(payload)
        .bind::<BigInt, _>(record.created_at.0)
        .bind::<BigInt, _>(record.expires_at.0)
        .bind::<Nullable<BigInt>, _>(None::<i64>)
        .execute(self.conn)?;
        Ok(record)
    }

    /// Only the account's own message; a second call dates it again.
    pub fn acknowledge_control(
        &mut self,
        account_id: &str,
        message_id: &str,
        at: UnixMillis,
    ) -> Result<bool, RepoError> {
        Ok(
            query("UPDATE rifts_control_messages SET acknowledged_at = ? WHERE message_id = ? AND account_id = ?")?
                .bind::<Nullable<BigInt>, _>(Some(at.0))
                .bind::<Text, _>(message_id.to_owned())
                .bind::<Text, _>(account_id.to_owned())
                .execute(self.conn)?
                > 0,
        )
    }

    pub fn upsert_host_presence(&mut self, p: &HostPresenceRecord) -> Result<(), RepoError> {
        let detail = serde_json::to_string(p).map_err(json_error)?;
        diesel::sql_query(store::portable::upsert(
            "rifts_host_presences",
            &[
                "account_id",
                "revision",
                "consent_to_host",
                "observed_at",
                "expires_at",
                "detail",
            ],
            &["account_id"],
            &["revision", "consent_to_host", "observed_at", "expires_at", "detail"],
        ))
        .bind::<Text, _>(p.account_id.clone())
        .bind::<BigInt, _>(p.revision)
        .bind::<Bool, _>(p.consent_to_host)
        .bind::<BigInt, _>(p.observed_at.0)
        .bind::<BigInt, _>(p.expires_at.0)
        .bind::<Text, _>(detail)
        .execute(self.conn)?;
        Ok(())
    }

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
        let mut r = reliability(self.conn, account_id)?.unwrap_or_else(|| HostReliabilityRecord::new(account_id, now));
        r.offers_accepted += add.offers_accepted;
        r.offers_declined += add.offers_declined;
        r.offers_expired += add.offers_expired;
        r.starts_succeeded += add.starts_succeeded;
        r.starts_failed += add.starts_failed;
        r.clean_completions += add.clean_completions;
        r.unexpected_failures += add.unexpected_failures;
        r.updated_at = now;
        if cooldown_until.is_some() {
            r.cooldown_until = cooldown_until;
        }
        diesel::sql_query(store::portable::upsert(
            "rifts_host_reliability",
            &RELIABILITY_COLUMNS,
            &["account_id"],
            &RELIABILITY_COLUMNS[1..],
        ))
        .bind::<Text, _>(r.account_id)
        .bind::<BigInt, _>(r.offers_accepted)
        .bind::<BigInt, _>(r.offers_declined)
        .bind::<BigInt, _>(r.offers_expired)
        .bind::<BigInt, _>(r.starts_succeeded)
        .bind::<BigInt, _>(r.starts_failed)
        .bind::<BigInt, _>(r.clean_completions)
        .bind::<BigInt, _>(r.unexpected_failures)
        .bind::<Nullable<BigInt>, _>(r.cooldown_until.map(|at| at.0))
        .bind::<BigInt, _>(r.updated_at.0)
        .execute(self.conn)?;
        Ok(())
    }

    /// One accepted offer per generation.
    pub fn create_offer(&mut self, o: &HostOfferRecord) -> Result<(), RepoError> {
        if o.status == OfferStatus::Accepted {
            let session = o.session_id.clone();
            let accepted = count(
                self.conn,
                "SELECT COUNT(*) AS n FROM rifts_host_offers WHERE session_id = ? AND generation = ? AND status = ?",
                |q| {
                    q.bind::<Text, _>(session)
                        .bind::<BigInt, _>(o.generation)
                        .bind::<Text, _>(OfferStatus::Accepted.as_str())
                },
            )?;
            if accepted > 0 {
                return Err(RepoError::Duplicate("accepted offer"));
            }
        }
        let breakdown = serde_json::to_string(&o.score_breakdown).map_err(json_error)?;
        let columns: Vec<&str> = OFFER_COLUMNS.split(", ").collect();
        diesel::sql_query(store::portable::upsert(
            "rifts_host_offers",
            &columns,
            &["offer_id"],
            &columns[1..],
        ))
        .into_boxed::<store::MultiBackend>()
        .bind::<Text, _>(o.offer_id.clone())
        .bind::<Text, _>(o.session_id.clone())
        .bind::<BigInt, _>(o.generation)
        .bind::<Text, _>(o.account_id.clone())
        .bind::<Text, _>(o.steam_id.clone())
        .bind::<BigInt, _>(o.rank)
        .bind::<Double, _>(o.score)
        .bind::<Text, _>(breakdown)
        .bind::<Text, _>(o.status.as_str())
        .bind::<BigInt, _>(o.offered_at.0)
        .bind::<BigInt, _>(o.expires_at.0)
        .bind::<Nullable<BigInt>, _>(o.responded_at.map(|at| at.0))
        .bind::<Nullable<Text>, _>(o.response_reason.clone())
        .execute(self.conn)?;
        Ok(())
    }

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
        let changed = query(
            "UPDATE rifts_host_offers SET status = ?, responded_at = ?, response_reason = ? \
             WHERE offer_id = ? AND account_id = ? AND status = ?",
        )?
        .bind::<Text, _>(status.as_str())
        .bind::<Nullable<BigInt>, _>(Some(responded_at.0))
        .bind::<Nullable<Text>, _>(reason.map(str::to_owned))
        .bind::<Text, _>(offer_id.to_owned())
        .bind::<Text, _>(account_id.to_owned())
        .bind::<Text, _>(expected.as_str())
        .execute(self.conn)?;
        if changed == 0 {
            return Ok(None);
        }
        offer(self.conn, offer_id)
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
        query(&format!(
            "INSERT INTO rifts_participant_observations ({OBSERVATION_COLUMNS}) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))?
        .bind::<Text, _>(o.observation_id.clone())
        .bind::<Text, _>(o.session_id.clone())
        .bind::<BigInt, _>(o.generation)
        .bind::<Text, _>(o.account_id.clone())
        .bind::<Text, _>(o.steam_id.clone())
        .bind::<Bool, _>(o.host_reachable)
        .bind::<Bool, _>(o.local_bridge_alive)
        .bind::<Bool, _>(o.metrics_available)
        .bind::<Nullable<BigInt>, _>(o.last_packet_age_ms)
        .bind::<Text, _>(o.route_kind.clone())
        .bind::<Nullable<BigInt>, _>(o.ping_ms)
        .bind::<Nullable<BigInt>, _>(o.jitter_ms)
        .bind::<Nullable<Double>, _>(o.packet_loss_percent)
        .bind::<BigInt, _>(o.observed_at.0)
        .execute(self.conn)?;
        Ok(())
    }
}

/// Realms' part of Node's hunt repository.
#[derive(Clone)]
pub struct HuntRepository {
    db: RealmsDb,
}

impl HuntRepository {
    pub fn new(db: RealmsDb) -> Self {
        Self { db }
    }

    /// `f` as one write: an `Err`, a refusal included, rolls every step back.
    pub async fn transaction<T, E, F>(&self, actor: &Actor, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut HuntTx<'_>) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<store::Error> + From<diesel::result::Error> + Send + 'static,
    {
        let node = self.db.node().clone();
        self.db
            .database()
            .write(store_actor(actor), move |tx| {
                f(&mut HuntTx {
                    conn: tx.conn(),
                    node: &node,
                })
            })
            .await
    }

    async fn read<T, F>(&self, f: F) -> Result<T, RepoError>
    where
        F: FnOnce(&mut AnyConnection) -> Result<T, RepoError> + Send + 'static,
        T: Send + 'static,
    {
        self.db.read(f).await
    }

    pub async fn create_hunt(&self, actor: &Actor, session: &HuntSession) -> Result<(), RepoError> {
        let session = session.clone();
        self.transaction(actor, move |tx| tx.create_hunt(&session)).await
    }

    pub async fn compare_and_set_hunt(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        expected_states: &[HuntState],
        expected_state_version: i64,
        change: impl FnOnce(&mut HuntSession) + Send + 'static,
    ) -> Result<Option<HuntSession>, RepoError> {
        let (id, states) = (session_id.to_owned(), expected_states.to_vec());
        self.transaction(actor, move |tx| {
            tx.compare_and_set(&id, generation, &states, expected_state_version, change)
        })
        .await
    }

    pub async fn append_hunt_transition(&self, actor: &Actor, t: &HuntTransition) -> Result<(), RepoError> {
        let t = t.clone();
        self.transaction(actor, move |tx| tx.append_transition(&t)).await
    }

    pub async fn begin_host_startup(
        &self,
        actor: &Actor,
        s: &HostStartup,
        now: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let s = s.clone();
        self.transaction(actor, move |tx| tx.begin_host_startup(&s, now)).await
    }

    pub async fn renew_host_lease(&self, actor: &Actor, r: &LeaseRenewal) -> Result<Option<HuntSession>, RepoError> {
        let r = r.clone();
        self.transaction(actor, move |tx| tx.renew_host_lease(&r)).await
    }

    pub async fn mark_host_ready(
        &self,
        actor: &Actor,
        m: &HostReadyMark,
        now: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let m = m.clone();
        self.transaction(actor, move |tx| tx.mark_host_ready(&m, now)).await
    }

    #[allow(clippy::too_many_arguments)]
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
        let (id, account, steam, bridge) = (
            session_id.to_owned(),
            account_id.to_owned(),
            steam_id.to_owned(),
            bridge_instance_id.to_owned(),
        );
        self.transaction(actor, move |tx| {
            tx.mark_participant_route_ready(&id, generation, &account, &steam, &bridge, local_bridge_port, ready_at)
        })
        .await
    }

    pub async fn mark_participant_admitted(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        account_id: &str,
        admitted_at: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let (id, account) = (session_id.to_owned(), account_id.to_owned());
        self.transaction(actor, move |tx| {
            tx.mark_participant_admitted(&id, generation, &account, admitted_at)
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
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
        let (id, account) = (session_id.to_owned(), account_id.to_owned());
        self.transaction(actor, move |tx| {
            tx.reserve_participant_ticket(&id, generation, &account, now, rejoin_deadline, max_reconnects)
        })
        .await
    }

    pub async fn mark_participant_disconnected(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        account_id: &str,
        disconnected_at: UnixMillis,
        rejoin_deadline: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let (id, account) = (session_id.to_owned(), account_id.to_owned());
        self.transaction(actor, move |tx| {
            tx.mark_participant_disconnected(&id, generation, &account, disconnected_at, rejoin_deadline)
        })
        .await
    }

    pub async fn create_host_capability(&self, actor: &Actor, c: &HostCapabilityRecord) -> Result<(), RepoError> {
        let c = c.clone();
        self.transaction(actor, move |tx| tx.create_capability(&c)).await
    }

    pub async fn advance_host_capability_sequence(
        &self,
        actor: &Actor,
        capability_id: &str,
        sequence: i64,
    ) -> Result<bool, RepoError> {
        let id = capability_id.to_owned();
        self.transaction(actor, move |tx| tx.advance_capability_sequence(&id, sequence))
            .await
    }

    pub async fn revoke_generation(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        at: UnixMillis,
    ) -> Result<(), RepoError> {
        let id = session_id.to_owned();
        self.transaction(actor, move |tx| tx.revoke_generation(&id, generation, at))
            .await
    }

    pub async fn enqueue_control_message(
        &self,
        actor: &Actor,
        m: &NewControlMessage,
    ) -> Result<ControlMessageRecord, RepoError> {
        let m = m.clone();
        self.transaction(actor, move |tx| tx.enqueue_control(&m)).await
    }

    pub async fn acknowledge_control_message(
        &self,
        actor: &Actor,
        account_id: &str,
        message_id: &str,
        at: UnixMillis,
    ) -> Result<bool, RepoError> {
        let (account, message) = (account_id.to_owned(), message_id.to_owned());
        self.transaction(actor, move |tx| tx.acknowledge_control(&account, &message, at))
            .await
    }

    pub async fn upsert_host_presence(&self, actor: &Actor, p: &HostPresenceRecord) -> Result<(), RepoError> {
        let p = p.clone();
        self.transaction(actor, move |tx| tx.upsert_host_presence(&p)).await
    }

    pub async fn increment_host_reliability(
        &self,
        actor: &Actor,
        account_id: &str,
        add: ReliabilityIncrements,
        now: UnixMillis,
        cooldown_until: Option<UnixMillis>,
    ) -> Result<(), RepoError> {
        let account = account_id.to_owned();
        self.transaction(actor, move |tx| {
            tx.increment_reliability(&account, add, now, cooldown_until)
        })
        .await
    }

    pub async fn create_host_offer(&self, actor: &Actor, o: &HostOfferRecord) -> Result<(), RepoError> {
        let o = o.clone();
        self.transaction(actor, move |tx| tx.create_offer(&o)).await
    }

    #[allow(clippy::too_many_arguments)]
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
        let (id, account, reason) = (offer_id.to_owned(), account_id.to_owned(), reason.map(str::to_owned));
        self.transaction(actor, move |tx| {
            tx.update_offer(&id, &account, expected, status, responded_at, reason.as_deref())
        })
        .await
    }

    pub async fn record_participant_observation(&self, actor: &Actor, o: &ObservationRecord) -> Result<(), RepoError> {
        let o = o.clone();
        self.transaction(actor, move |tx| tx.record_observation(&o)).await
    }

    pub async fn find_hunt(&self, session_id: &str) -> Result<Option<HuntSession>, RepoError> {
        let id = session_id.to_owned();
        self.read(move |conn| load(conn, &id)).await
    }

    /// A party's key, or a realm hub kind's.
    pub async fn find_active_hunt_for_party(&self, party_key: &str) -> Result<Option<HuntSession>, RepoError> {
        let key = party_key.to_owned();
        self.read(move |conn| {
            Ok(
                sessions(conn, "WHERE active_party_key = ?", |q| q.bind::<Text, _>(key))?
                    .into_iter()
                    .next(),
            )
        })
        .await
    }

    /// The account's newest active hunt; never a hub.
    pub async fn find_active_hunt_for_account(&self, account_id: &str) -> Result<Option<HuntSession>, RepoError> {
        let account = account_id.to_owned();
        self.read(move |conn| {
            Ok(sessions(
                conn,
                "WHERE session_id IN (SELECT session_id FROM rifts_hunt_roster WHERE account_id = ?) \
                 AND active_party_key IS NOT NULL AND session_kind IS NULL \
                 ORDER BY created_at DESC, session_id LIMIT 1",
                |q| q.bind::<Text, _>(account),
            )?
            .into_iter()
            .next())
        })
        .await
    }

    pub async fn find_recoverable_hunt_for_account(
        &self,
        account_id: &str,
        now: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let account = account_id.to_owned();
        self.read(move |conn| {
            Ok(sessions(
                conn,
                "WHERE session_id IN (SELECT session_id FROM rifts_hunt_roster WHERE account_id = ?) \
                 AND state = ? AND expires_at > ? ORDER BY created_at DESC, session_id LIMIT 1",
                |q| {
                    q.bind::<Text, _>(account)
                        .bind::<Text, _>(H::Recovering.as_str())
                        .bind::<BigInt, _>(now.0)
                },
            )?
            .into_iter()
            .next())
        })
        .await
    }

    /// Oldest first.
    pub async fn list_lifecycle_hunts(&self, states: &[HuntState], max: u32) -> Result<Vec<HuntSession>, RepoError> {
        let states = states.to_vec();
        self.read(move |conn| {
            let tail = format!(
                "WHERE {} ORDER BY created_at, session_id LIMIT ?",
                states_clause("state", &states)
            );
            sessions(conn, &tail, |q| bind_states(q, &states).bind::<BigInt, _>(limit(max)))
        })
        .await
    }

    pub async fn list_transitions(&self, session_id: &str) -> Result<Vec<HuntTransition>, RepoError> {
        let id = session_id.to_owned();
        self.read(move |conn| {
            query(
                "SELECT transition_id, session_id, generation, from_state, to_state, state_version, reason, \
                 actor_type, actor_account_id, at FROM rifts_hunt_transitions WHERE session_id = ? \
                 ORDER BY at, state_version, transition_id",
            )?
            .bind::<Text, _>(id)
            .load::<TransitionRow>(conn)?
            .into_iter()
            .map(TransitionRow::into_record)
            .collect()
        })
        .await
    }

    pub async fn find_host_capability_by_digest(
        &self,
        digest: &str,
    ) -> Result<Option<HostCapabilityRecord>, RepoError> {
        let digest = digest.to_owned();
        self.read(move |conn| {
            query(&format!(
                "SELECT {CAPABILITY_COLUMNS} FROM rifts_host_capabilities WHERE secret_digest = ?"
            ))?
            .bind::<Text, _>(digest)
            .load::<CapabilityRow>(conn)?
            .into_iter()
            .next()
            .map(CapabilityRow::into_record)
            .transpose()
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
        let account = account_id.to_owned();
        self.read(move |conn| {
            let unacknowledged = if unacknowledged_only {
                " AND acknowledged_at IS NULL"
            } else {
                ""
            };
            query(&format!(
                "SELECT {CONTROL_COLUMNS} FROM rifts_control_messages WHERE account_id = ? AND sequence > ? \
                 AND expires_at > ?{unacknowledged} ORDER BY sequence LIMIT ?"
            ))?
            .bind::<Text, _>(account)
            .bind::<BigInt, _>(after_sequence)
            .bind::<BigInt, _>(now.0)
            .bind::<BigInt, _>(limit(max))
            .load::<ControlRow>(conn)?
            .into_iter()
            .map(ControlRow::into_record)
            .collect()
        })
        .await
    }

    /// 0 when the account never had a message.
    pub async fn latest_control_sequence(&self, account_id: &str) -> Result<i64, RepoError> {
        let account = account_id.to_owned();
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
        let accounts = account_ids.to_vec();
        self.read(move |conn| {
            let mut q = query(&format!(
                "SELECT account_id, detail FROM rifts_host_presences WHERE observed_at >= ? AND expires_at > ? \
                 AND account_id IN ({})",
                placeholders(accounts.len())
            ))?
            .bind::<BigInt, _>(observed_after.0)
            .bind::<BigInt, _>(now.0);
            for account in &accounts {
                q = q.bind::<Text, _>(account.clone());
            }
            let mut rows = q.load::<DetailRow>(conn)?;
            let mut out = Vec::with_capacity(rows.len());
            for account in &accounts {
                if let Some(at) = rows.iter().position(|row| &row.account_id == account) {
                    let row = rows.swap_remove(at);
                    out.push(serde_json::from_str(&row.detail).map_err(json_error)?);
                }
            }
            Ok(out)
        })
        .await
    }

    async fn count_where(
        &self,
        tail: &'static str,
        value: &str,
        states: &'static [HuntState],
    ) -> Result<i64, RepoError> {
        let value = value.to_owned();
        self.read(move |conn| {
            let text = format!(
                "SELECT COUNT(*) AS n FROM rifts_hunt_sessions WHERE {tail} ({})",
                states_clause("state", states)
            );
            count(conn, &text, |q| bind_states(q.bind::<Text, _>(value), states))
        })
        .await
    }

    /// A draining session gave its key up.
    pub async fn count_active_host_sessions(&self, account_id: &str) -> Result<i64, RepoError> {
        self.count_where(
            "host_account_id = ? AND active_party_key IS NOT NULL AND",
            account_id,
            LIVE_HOST_STATES,
        )
        .await
    }

    pub async fn count_active_realm_servers(&self, realm_id: &str) -> Result<i64, RepoError> {
        self.count_where(
            "realm_id = ? AND active_party_key IS NOT NULL AND",
            realm_id,
            REALM_SERVER_STATES,
        )
        .await
    }

    /// Draining included: its server still runs on the host's PC.
    pub async fn count_running_realm_servers(&self, realm_id: &str) -> Result<i64, RepoError> {
        let realm = realm_id.to_owned();
        self.read(move |conn| {
            let text = format!(
                "SELECT COUNT(*) AS n FROM rifts_hunt_sessions WHERE realm_id = ? \
                 AND ((active_party_key IS NOT NULL AND {}) OR state = ?)",
                states_clause("state", REALM_SERVER_STATES)
            );
            count(conn, &text, |q| {
                bind_states(q.bind::<Text, _>(realm), REALM_SERVER_STATES).bind::<Text, _>(H::Draining.as_str())
            })
        })
        .await
    }

    /// Never hubs; at most 64.
    pub async fn list_live_realm_hunts(&self, realm_id: &str, host: &str) -> Result<Vec<HuntSession>, RepoError> {
        let (realm, host) = (realm_id.to_owned(), host.to_owned());
        self.read(move |conn| {
            let tail = format!(
                "WHERE realm_id = ? AND host_account_id = ? AND session_kind IS NULL \
                 AND active_party_key IS NOT NULL AND {} ORDER BY created_at, session_id LIMIT 64",
                states_clause("state", REALM_SERVER_STATES)
            );
            sessions(conn, &tail, |q| {
                bind_states(q.bind::<Text, _>(realm).bind::<Text, _>(host), REALM_SERVER_STATES)
            })
        })
        .await
    }

    pub async fn find_host_reliability(&self, account_id: &str) -> Result<Option<HostReliabilityRecord>, RepoError> {
        let account = account_id.to_owned();
        self.read(move |conn| reliability(conn, &account)).await
    }

    pub async fn find_host_offer(&self, offer_id: &str) -> Result<Option<HostOfferRecord>, RepoError> {
        let id = offer_id.to_owned();
        self.read(move |conn| offer(conn, &id)).await
    }

    pub async fn list_host_offers(&self, session_id: &str, generation: i64) -> Result<Vec<HostOfferRecord>, RepoError> {
        let id = session_id.to_owned();
        self.read(move |conn| {
            query(&format!(
                "SELECT {OFFER_COLUMNS} FROM rifts_host_offers WHERE session_id = ? AND generation = ? \
                 ORDER BY rank, offer_id"
            ))?
            .bind::<Text, _>(id)
            .bind::<BigInt, _>(generation)
            .load::<OfferRow>(conn)?
            .into_iter()
            .map(OfferRow::into_record)
            .collect()
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
        let id = session_id.to_owned();
        self.read(move |conn| {
            Ok(query(&format!(
                "SELECT {OBSERVATION_COLUMNS} FROM rifts_participant_observations WHERE session_id = ? \
                 AND generation = ? AND observed_at >= ? ORDER BY observed_at DESC, observation_id"
            ))?
            .bind::<Text, _>(id)
            .bind::<BigInt, _>(generation)
            .bind::<BigInt, _>(observed_after.0)
            .load::<ObservationRow>(conn)?
            .into_iter()
            .map(ObservationRecord::from)
            .collect())
        })
        .await
    }

    /// Control messages and host presences past their time. Nothing calls this yet (spec-rd27-rifts §7 risk 3).
    pub async fn purge_expired(&self, now: UnixMillis) -> Result<usize, RepoError> {
        self.db
            .write(&Actor::System("realms-purge"), move |conn| {
                let messages = query("DELETE FROM rifts_control_messages WHERE expires_at <= ?")?
                    .bind::<BigInt, _>(now.0)
                    .execute(conn)?;
                let presences = query("DELETE FROM rifts_host_presences WHERE expires_at <= ?")?
                    .bind::<BigInt, _>(now.0)
                    .execute(conn)?;
                Ok(messages + presences)
            })
            .await
    }
}
