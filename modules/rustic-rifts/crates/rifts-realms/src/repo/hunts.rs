//! Realms' part of `P2PHuntRepository` on `store`: sessions with their roster, transitions, host capabilities, the
//! control channel, host presences, offers, reliability and observations. Node's guarded `updateOne`s become one write
//! transaction that reads, checks and writes the row, which is atomic because store's writes are serial.

use diesel::RunQueryDsl;
use diesel::sql_types::{BigInt, Bool, Double, Nullable, Text};
use rifts_seam::{Actor, UnixMillis};
use store::AnyConnection;

use crate::db::{Count, RealmsDb, RepoError, corrupt, marks, sql};
use crate::hunt::{
    AdmissionState, ControlMessageRecord, HostCapabilityRecord, HostOfferRecord, HostPresenceRecord,
    HostReliabilityRecord, HuntSession, HuntState, HuntTransition, ObservationRecord, OfferStatus, Participant,
    ReliabilityIncrements, ScoreBreakdown, parse_scope, scope_name,
};
use crate::ids::{global_session_id, random_uuid};

use HuntState::{Admitting, Draining, HostOffered, HostReady, HostSelecting, Running, Starting, Suspect};

/// `LIVE_HOST_STATES`: a host's session counts while in these.
pub const LIVE_HOST_STATES: &[HuntState] = &[HostOffered, Starting, HostReady, Admitting, Running, Suspect];
const REALM_SERVER_STATES: &[HuntState] = &[HostSelecting, HostOffered, Starting, HostReady, Admitting, Running, Suspect];
const ROSTER_LIVE_STATES: &[HuntState] = &[HostReady, Admitting, Running];

#[derive(diesel::QueryableByName)]
struct SessionRow {
    #[diesel(sql_type = Text)]
    detail: String,
}

#[derive(diesel::QueryableByName)]
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

const ROSTER_COLUMNS: &str = "account_id, steam_id, candidate_id, reconnect_epoch, admission_state, \
    local_bridge_instance_id, local_bridge_port, local_bridge_ready_at, admitted_at, disconnected_at, rejoin_deadline, \
    returned_to_hub_at";

#[derive(diesel::QueryableByName)]
struct IdRow {
    #[diesel(sql_type = Text)]
    session_id: String,
}

fn states_in(states: &[HuntState]) -> String {
    format!("state IN ({})", marks(states.len()))
}

fn load_session(conn: &mut AnyConnection, session_id: &str) -> Result<Option<HuntSession>, RepoError> {
    let Some(row) = sql("SELECT detail FROM rifts_hunt_sessions WHERE session_id = ?")?
        .bind::<Text, _>(session_id.to_owned())
        .load::<SessionRow>(conn)?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let mut session: HuntSession =
        serde_json::from_str(&row.detail).map_err(|e| RepoError::Corrupt(format!("hunt {session_id}: {e}")))?;
    let ms = |v: Option<i64>| v.map(UnixMillis);
    session.roster = sql(&format!(
        "SELECT {ROSTER_COLUMNS} FROM rifts_hunt_roster WHERE session_id = ? ORDER BY seq"
    ))?
    .bind::<Text, _>(session_id.to_owned())
    .load::<RosterRow>(conn)?
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
            local_bridge_ready_at: ms(r.local_bridge_ready_at),
            admitted_at: ms(r.admitted_at),
            disconnected_at: ms(r.disconnected_at),
            rejoin_deadline: ms(r.rejoin_deadline),
            returned_to_hub_at: ms(r.returned_to_hub_at),
        })
    })
    .collect::<Result<Vec<_>, RepoError>>()?;
    Ok(Some(session))
}

fn load_many(conn: &mut AnyConnection, ids: Vec<IdRow>) -> Result<Vec<HuntSession>, RepoError> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(session) = load_session(conn, &id.session_id)? {
            out.push(session);
        }
    }
    Ok(out)
}

/// The row's columns from the record, and the roster rows to match it.
fn save_session(conn: &mut AnyConnection, db_node: &module_api::NodeId, session: &HuntSession) -> Result<(), RepoError> {
    let detail = serde_json::to_string(session).map_err(|e| RepoError::Corrupt(e.to_string()))?;
    let global = global_session_id(&session.session_id, session.is_realm_hub(), db_node).map(|id| id.to_string());
    diesel::sql_query(store::portable::upsert(
        "rifts_hunt_sessions",
        &[
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
        ],
        &["session_id"],
        &[
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
        ],
    ))
    .bind::<Text, _>(session.session_id.clone())
    .bind::<Nullable<Text>, _>(global)
    .bind::<BigInt, _>(session.generation)
    .bind::<Text, _>(session.state.as_str())
    .bind::<BigInt, _>(session.state_version)
    .bind::<Text, _>(session.party_id.clone())
    .bind::<Nullable<Text>, _>(session.active_party_key.clone())
    .bind::<Text, _>(session.host_account_id.clone())
    .bind::<Nullable<Text>, _>(session.is_realm_hub().then_some("REALM_HUB"))
    .bind::<Nullable<Text>, _>(session.hub_kind.map(crate::realm::hub_kind_name))
    .bind::<Nullable<Text>, _>(session.realm_id.clone())
    .bind::<Nullable<BigInt>, _>(session.lease_expires_at.map(|t| t.0))
    .bind::<BigInt, _>(session.created_at.0)
    .bind::<BigInt, _>(session.expires_at.0)
    .bind::<Text, _>(detail)
    .execute(conn)?;
    save_roster(conn, session)
}

fn save_roster(conn: &mut AnyConnection, session: &HuntSession) -> Result<(), RepoError> {
    let keep: Vec<String> = session.roster.iter().map(|p| p.account_id.clone()).collect();
    let mut delete = sql(&format!(
        "DELETE FROM rifts_hunt_roster WHERE session_id = ?{}",
        if keep.is_empty() {
            String::new()
        } else {
            format!(" AND account_id NOT IN ({})", marks(keep.len()))
        }
    ))?
    .bind::<Text, _>(session.session_id.clone());
    for account in keep {
        delete = delete.bind::<Text, _>(account);
    }
    delete.execute(conn)?;
    let columns: Vec<&str> = ["session_id", "seq"]
        .into_iter()
        .chain(ROSTER_COLUMNS.split(", "))
        .collect();
    let update: Vec<&str> = columns
        .iter()
        .copied()
        .filter(|c| !matches!(*c, "session_id" | "account_id"))
        .collect();
    let upsert = store::portable::upsert("rifts_hunt_roster", &columns, &["session_id", "account_id"], &update);
    let ms = |v: Option<UnixMillis>| v.map(|t| t.0);
    for (seq, p) in (1i64..).zip(&session.roster) {
        diesel::sql_query(upsert.clone())
            .bind::<Text, _>(session.session_id.clone())
            .bind::<BigInt, _>(seq)
            .bind::<Text, _>(p.account_id.clone())
            .bind::<Text, _>(p.steam_id.clone())
            .bind::<Text, _>(p.candidate_id.clone())
            .bind::<BigInt, _>(p.reconnect_epoch)
            .bind::<Text, _>(p.admission_state.as_str())
            .bind::<Nullable<Text>, _>(p.local_bridge_instance_id.clone())
            .bind::<Nullable<BigInt>, _>(p.local_bridge_port)
            .bind::<Nullable<BigInt>, _>(ms(p.local_bridge_ready_at))
            .bind::<Nullable<BigInt>, _>(ms(p.admitted_at))
            .bind::<Nullable<BigInt>, _>(ms(p.disconnected_at))
            .bind::<Nullable<BigInt>, _>(ms(p.rejoin_deadline))
            .bind::<Nullable<BigInt>, _>(ms(p.returned_to_hub_at))
            .execute(conn)?;
    }
    Ok(())
}

fn count(conn: &mut AnyConnection, text: &str, binds: &[&str]) -> Result<i64, RepoError> {
    let mut query = sql(text)?;
    for bind in binds {
        query = query.bind::<Text, _>((*bind).to_owned());
    }
    Ok(query.load::<Count>(conn)?.into_iter().next().map_or(0, |c| c.n))
}

#[derive(diesel::QueryableByName)]
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

#[derive(diesel::QueryableByName)]
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

const CAPABILITY_COLUMNS: &str = "capability_id, secret_digest, session_id, generation, host_account_id, \
    host_steam_id, bridge_instance_id, scopes, issued_at, expires_at, revoked_at, last_sequence";

fn capability_of(row: CapabilityRow) -> Result<HostCapabilityRecord, RepoError> {
    let scopes = row
        .scopes
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| parse_scope(s).ok_or_else(|| corrupt("capability scope", s)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(HostCapabilityRecord {
        capability_id: row.capability_id,
        secret_digest: row.secret_digest,
        session_id: row.session_id,
        generation: row.generation,
        host_account_id: row.host_account_id,
        host_steam_id: row.host_steam_id,
        bridge_instance_id: row.bridge_instance_id,
        scopes,
        issued_at: UnixMillis(row.issued_at),
        expires_at: UnixMillis(row.expires_at),
        revoked_at: row.revoked_at.map(UnixMillis),
        last_sequence: row.last_sequence,
    })
}

#[derive(diesel::QueryableByName)]
struct MessageRow {
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

const MESSAGE_COLUMNS: &str = "message_id, account_id, sequence, message_type, session_id, generation, payload, \
    created_at, expires_at, acknowledged_at";

fn message_of(row: MessageRow) -> Result<ControlMessageRecord, RepoError> {
    Ok(ControlMessageRecord {
        payload: serde_json::from_str(&row.payload).map_err(|e| RepoError::Corrupt(e.to_string()))?,
        message_id: row.message_id,
        account_id: row.account_id,
        sequence: row.sequence,
        message_type: row.message_type,
        session_id: row.session_id,
        generation: row.generation,
        created_at: UnixMillis(row.created_at),
        expires_at: UnixMillis(row.expires_at),
        acknowledged_at: row.acknowledged_at.map(UnixMillis),
    })
}

#[derive(diesel::QueryableByName)]
struct DetailRow {
    #[diesel(sql_type = Text)]
    detail: String,
}

#[derive(diesel::QueryableByName)]
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

const OFFER_COLUMNS: &str = "offer_id, session_id, generation, account_id, steam_id, rank, score, score_breakdown, \
    status, offered_at, expires_at, responded_at, response_reason";

fn offer_of(row: OfferRow) -> Result<HostOfferRecord, RepoError> {
    let breakdown: ScoreBreakdown =
        serde_json::from_str(&row.score_breakdown).map_err(|e| RepoError::Corrupt(e.to_string()))?;
    Ok(HostOfferRecord {
        status: OfferStatus::parse(&row.status).ok_or_else(|| corrupt("offer status", &row.status))?,
        offer_id: row.offer_id,
        session_id: row.session_id,
        generation: row.generation,
        account_id: row.account_id,
        steam_id: row.steam_id,
        rank: row.rank,
        score: row.score,
        score_breakdown: breakdown,
        offered_at: UnixMillis(row.offered_at),
        expires_at: UnixMillis(row.expires_at),
        responded_at: row.responded_at.map(UnixMillis),
        response_reason: row.response_reason,
    })
}

fn find_offer(conn: &mut AnyConnection, offer_id: &str) -> Result<Option<HostOfferRecord>, RepoError> {
    sql(&format!("SELECT {OFFER_COLUMNS} FROM rifts_host_offers WHERE offer_id = ?"))?
        .bind::<Text, _>(offer_id.to_owned())
        .load::<OfferRow>(conn)?
        .into_iter()
        .next()
        .map(offer_of)
        .transpose()
}

#[derive(diesel::QueryableByName)]
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

const RELIABILITY_COLUMNS: &str = "account_id, offers_accepted, offers_declined, offers_expired, starts_succeeded, \
    starts_failed, clean_completions, unexpected_failures, cooldown_until, updated_at";

fn find_reliability(conn: &mut AnyConnection, account_id: &str) -> Result<Option<HostReliabilityRecord>, RepoError> {
    Ok(sql(&format!("SELECT {RELIABILITY_COLUMNS} FROM rifts_host_reliability WHERE account_id = ?"))?
        .bind::<Text, _>(account_id.to_owned())
        .load::<ReliabilityRow>(conn)?
        .into_iter()
        .next()
        .map(|r| HostReliabilityRecord {
            account_id: r.account_id,
            offers_accepted: r.offers_accepted,
            offers_declined: r.offers_declined,
            offers_expired: r.offers_expired,
            starts_succeeded: r.starts_succeeded,
            starts_failed: r.starts_failed,
            clean_completions: r.clean_completions,
            unexpected_failures: r.unexpected_failures,
            cooldown_until: r.cooldown_until.map(UnixMillis),
            updated_at: UnixMillis(r.updated_at),
        }))
}

#[derive(diesel::QueryableByName)]
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

const OBSERVATION_COLUMNS: &str = "observation_id, session_id, generation, account_id, steam_id, host_reachable, \
    local_bridge_alive, metrics_available, last_packet_age_ms, route_kind, ping_ms, jitter_ms, packet_loss_percent, \
    observed_at";

/// What `beginHostStartup` writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostStartup {
    pub session_id: String,
    pub host_account_id: String,
    pub bridge_instance_id: String,
    pub metadata_nonce: String,
    pub startup_deadline: UnixMillis,
    pub server_udp_port: Option<i64>,
}

/// What `renewHostLease` checks and writes.
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

/// What `markHostReady` checks and writes.
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

/// A message to queue: the repository gives it its id and the account's next sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct NewControlMessage {
    pub account_id: String,
    pub message_type: String,
    pub session_id: Option<String>,
    pub generation: Option<i64>,
    pub payload: serde_json::Value,
    pub created_at: UnixMillis,
    pub expires_at: UnixMillis,
}

/// One write transaction over realms' hunt tables (Node's `WithP2PTransaction`): every step a lifecycle change makes
/// (the compare-and-set, its transition, the offer, the control messages) commits together or not at all.
pub struct HuntTx<'a> {
    conn: &'a mut AnyConnection,
    node: &'a module_api::NodeId,
}

impl HuntTx<'_> {
    pub fn find_hunt(&mut self, session_id: &str) -> Result<Option<HuntSession>, RepoError> {
        load_session(self.conn, session_id)
    }

    /// The session id and the active party key are each unique.
    pub fn create_hunt(&mut self, session: &HuntSession) -> Result<(), RepoError> {
        let text = "SELECT COUNT(*) AS n FROM rifts_hunt_sessions WHERE session_id = ?";
        if count(self.conn, text, &[&session.session_id])? > 0 {
            return Err(RepoError::Duplicate("session_id"));
        }
        if let Some(key) = &session.active_party_key {
            let text = "SELECT COUNT(*) AS n FROM rifts_hunt_sessions WHERE active_party_key = ?";
            if count(self.conn, text, &[key])? > 0 {
                return Err(RepoError::Duplicate("active_party_key"));
            }
        }
        save_session(self.conn, self.node, session)
    }

    /// Loads, lets `change` decide and edit, then saves; `bump` adds one to the state version.
    pub fn update(
        &mut self,
        session_id: &str,
        bump: bool,
        change: impl FnOnce(&mut HuntSession) -> bool,
    ) -> Result<Option<HuntSession>, RepoError> {
        let Some(mut hunt) = load_session(self.conn, session_id)? else {
            return Ok(None);
        };
        if !change(&mut hunt) {
            return Ok(None);
        }
        if bump {
            hunt.state_version += 1;
        }
        if hunt.active_party_key.is_some() {
            let text = "SELECT COUNT(*) AS n FROM rifts_hunt_sessions WHERE active_party_key = ? AND session_id <> ?";
            let key = hunt.active_party_key.clone().unwrap_or_default();
            if count(self.conn, text, &[&key, &hunt.session_id])? > 0 {
                return Err(RepoError::Duplicate("active_party_key"));
            }
        }
        save_session(self.conn, self.node, &hunt)?;
        Ok(Some(hunt))
    }

    /// `compareAndSetHunt`: `change` runs only while the generation, a state in `expected_states` and the state
    /// version still hold; the version is bumped.
    pub fn compare_and_set(
        &mut self,
        session_id: &str,
        generation: i64,
        expected_states: &[HuntState],
        expected_state_version: i64,
        change: impl FnOnce(&mut HuntSession),
    ) -> Result<Option<HuntSession>, RepoError> {
        self.update(session_id, true, |hunt| {
            if hunt.generation != generation
                || !expected_states.contains(&hunt.state)
                || hunt.state_version != expected_state_version
            {
                return false;
            }
            change(hunt);
            true
        })
    }

    /// An empty `transition_id` gets a fresh one.
    pub fn append_transition(&mut self, t: &HuntTransition) -> Result<(), RepoError> {
        sql("INSERT INTO rifts_hunt_transitions (transition_id, session_id, generation, from_state, to_state, \
             state_version, reason, actor_type, actor_account_id, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")?
        .bind::<Text, _>(if t.transition_id.is_empty() {
            random_uuid()
        } else {
            t.transition_id.clone()
        })
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

    /// HOST_OFFERED -> STARTING for the offered host of an unexpired session.
    pub fn begin_host_startup(&mut self, s: &HostStartup, now: UnixMillis) -> Result<Option<HuntSession>, RepoError> {
        let s = s.clone();
        self.update(&s.session_id.clone(), true, move |hunt| {
            if hunt.host_account_id != s.host_account_id || hunt.state != HostOffered || hunt.expires_at <= now {
                return false;
            }
            hunt.state = Starting;
            hunt.bridge_instance_id = s.bridge_instance_id;
            hunt.metadata_nonce = Some(s.metadata_nonce);
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

    /// A strictly newer lease sequence from the session's host process, while it may still hold a lease.
    pub fn renew_host_lease(&mut self, r: &LeaseRenewal) -> Result<Option<HuntSession>, RepoError> {
        let r = r.clone();
        self.update(&r.session_id.clone(), true, move |hunt| {
            let renewable = [HostReady, Admitting, Running, Draining, Suspect].contains(&hunt.state);
            if hunt.generation != r.generation
                || hunt.host_steam_id != r.host_steam_id
                || hunt.bridge_instance_id != r.bridge_instance_id
                || !renewable
                || hunt.last_lease_sequence.is_some_and(|last| last >= r.lease_sequence)
            {
                return false;
            }
            hunt.last_lease_sequence = Some(r.lease_sequence);
            hunt.last_heartbeat_at = Some(r.heartbeat_at);
            hunt.lease_expires_at = Some(r.lease_expires_at);
            true
        })
    }

    /// STARTING (or HOST_READY again) -> HOST_READY, with the lobby and the first lease.
    pub fn mark_host_ready(&mut self, m: &HostReadyMark, now: UnixMillis) -> Result<Option<HuntSession>, RepoError> {
        let m = m.clone();
        self.update(&m.session_id.clone(), true, move |hunt| {
            if hunt.generation != m.generation
                || hunt.host_steam_id != m.host_steam_id
                || hunt.bridge_instance_id != m.bridge_instance_id
                || hunt.virtual_port != m.virtual_port
                || ![Starting, HostReady].contains(&hunt.state)
            {
                return false;
            }
            hunt.state = HostReady;
            hunt.lobby_id = Some(m.lobby_id);
            hunt.lease_expires_at = Some(m.lease_expires_at);
            hunt.last_heartbeat_at = Some(now);
            hunt.last_lease_sequence = Some(0);
            true
        })
    }

    /// Changes one rostered participant of a generation in `states`.
    fn participant(
        &mut self,
        session_id: &str,
        generation: i64,
        states: &[HuntState],
        bump: bool,
        pick: impl Fn(&Participant) -> bool,
        change: impl FnOnce(&mut Participant) -> bool,
    ) -> Result<Option<HuntSession>, RepoError> {
        self.update(session_id, bump, |hunt| {
            if hunt.generation != generation || !states.contains(&hunt.state) {
                return false;
            }
            match hunt.roster.iter_mut().find(|p| pick(p)) {
                Some(p) => change(p),
                None => false,
            }
        })
    }

    /// A rostered participant published its loopback route (no version bump, as Node's).
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
        let pick = |p: &Participant| p.account_id == account_id && p.steam_id == steam_id;
        self.participant(session_id, generation, ROSTER_LIVE_STATES, false, pick, |p| {
            p.local_bridge_instance_id = Some(bridge_instance_id.to_owned());
            p.local_bridge_port = Some(local_bridge_port);
            p.local_bridge_ready_at = Some(ready_at);
            true
        })
    }

    pub fn mark_participant_admitted(
        &mut self,
        session_id: &str,
        generation: i64,
        account_id: &str,
        admitted_at: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let pick = |p: &Participant| p.account_id == account_id;
        self.participant(session_id, generation, ROSTER_LIVE_STATES, true, pick, |p| {
            p.admission_state = AdmissionState::Admitted;
            p.admitted_at = Some(admitted_at);
            p.disconnected_at = None;
            p.rejoin_deadline = None;
            true
        })
    }

    /// One more ticket for a participant within its reconnect budget and rejoin window.
    pub fn reserve_participant_ticket(
        &mut self,
        session_id: &str,
        generation: i64,
        account_id: &str,
        now: UnixMillis,
        rejoin_deadline: UnixMillis,
        max_reconnects: i64,
    ) -> Result<Option<HuntSession>, RepoError> {
        let pick = |p: &Participant| p.account_id == account_id;
        self.participant(session_id, generation, ROSTER_LIVE_STATES, false, pick, |p| {
            let within_window = p.disconnected_at.is_none() || p.rejoin_deadline.is_some_and(|d| d > now);
            if p.reconnect_epoch >= max_reconnects || !within_window {
                return false;
            }
            p.reconnect_epoch += 1;
            p.admission_state = AdmissionState::Issued;
            p.rejoin_deadline = Some(rejoin_deadline);
            true
        })
    }

    pub fn mark_participant_disconnected(
        &mut self,
        session_id: &str,
        generation: i64,
        account_id: &str,
        disconnected_at: UnixMillis,
        rejoin_deadline: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let states = [HostReady, Admitting, Running, Draining, Suspect];
        let pick = |p: &Participant| p.account_id == account_id;
        self.participant(session_id, generation, &states, false, pick, |p| {
            p.admission_state = AdmissionState::Lost;
            p.disconnected_at = Some(disconnected_at);
            p.rejoin_deadline = Some(rejoin_deadline);
            true
        })
    }

    pub fn create_capability(&mut self, c: &HostCapabilityRecord) -> Result<(), RepoError> {
        let text = "SELECT COUNT(*) AS n FROM rifts_host_capabilities WHERE secret_digest = ?";
        if count(self.conn, text, &[&c.secret_digest])? > 0 {
            return Err(RepoError::Duplicate("secret_digest"));
        }
        let scopes: Vec<&str> = c.scopes.iter().map(|s| scope_name(*s)).collect();
        sql(&format!(
            "INSERT INTO rifts_host_capabilities ({CAPABILITY_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))?
        .bind::<Text, _>(c.capability_id.clone())
        .bind::<Text, _>(c.secret_digest.clone())
        .bind::<Text, _>(c.session_id.clone())
        .bind::<BigInt, _>(c.generation)
        .bind::<Text, _>(c.host_account_id.clone())
        .bind::<Text, _>(c.host_steam_id.clone())
        .bind::<Text, _>(c.bridge_instance_id.clone())
        .bind::<Text, _>(scopes.join(","))
        .bind::<BigInt, _>(c.issued_at.0)
        .bind::<BigInt, _>(c.expires_at.0)
        .bind::<Nullable<BigInt>, _>(c.revoked_at.map(|t| t.0))
        .bind::<BigInt, _>(c.last_sequence)
        .execute(self.conn)?;
        Ok(())
    }

    /// A strictly newer sequence on an unrevoked capability.
    pub fn advance_capability_sequence(&mut self, capability_id: &str, sequence: i64) -> Result<bool, RepoError> {
        Ok(sql("UPDATE rifts_host_capabilities SET last_sequence = ? WHERE capability_id = ? \
             AND revoked_at IS NULL AND last_sequence < ?")?
        .bind::<BigInt, _>(sequence)
        .bind::<Text, _>(capability_id.to_owned())
        .bind::<BigInt, _>(sequence)
        .execute(self.conn)?
            > 0)
    }

    /// Revokes every live capability of the generation.
    pub fn revoke_generation(&mut self, session_id: &str, generation: i64, at: UnixMillis) -> Result<(), RepoError> {
        sql("UPDATE rifts_host_capabilities SET revoked_at = ? WHERE session_id = ? AND generation = ? \
             AND revoked_at IS NULL")?
        .bind::<BigInt, _>(at.0)
        .bind::<Text, _>(session_id.to_owned())
        .bind::<BigInt, _>(generation)
        .execute(self.conn)?;
        Ok(())
    }

    /// Appends with the account's next sequence; sequences survive the messages' purge.
    pub fn enqueue_control(&mut self, m: &NewControlMessage) -> Result<ControlMessageRecord, RepoError> {
        let sequence = count(
            self.conn,
            "SELECT COALESCE(MAX(next_sequence), 0) + 1 AS n FROM rifts_control_sequences WHERE account_id = ?",
            &[&m.account_id],
        )?;
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
        sql(&format!(
            "INSERT INTO rifts_control_messages ({MESSAGE_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))?
        .bind::<Text, _>(record.message_id.clone())
        .bind::<Text, _>(record.account_id.clone())
        .bind::<BigInt, _>(record.sequence)
        .bind::<Text, _>(record.message_type.clone())
        .bind::<Nullable<Text>, _>(record.session_id.clone())
        .bind::<Nullable<BigInt>, _>(record.generation)
        .bind::<Text, _>(record.payload.to_string())
        .bind::<BigInt, _>(record.created_at.0)
        .bind::<BigInt, _>(record.expires_at.0)
        .bind::<Nullable<BigInt>, _>(None::<i64>)
        .execute(self.conn)?;
        Ok(record)
    }

    pub fn acknowledge_control(&mut self, account_id: &str, message_id: &str, at: UnixMillis) -> Result<bool, RepoError> {
        Ok(sql("UPDATE rifts_control_messages SET acknowledged_at = ? WHERE message_id = ? AND account_id = ?")?
            .bind::<BigInt, _>(at.0)
            .bind::<Text, _>(message_id.to_owned())
            .bind::<Text, _>(account_id.to_owned())
            .execute(self.conn)?
            > 0)
    }

    pub fn upsert_host_presence(&mut self, p: &HostPresenceRecord) -> Result<(), RepoError> {
        let detail = serde_json::to_string(p).map_err(|e| RepoError::Corrupt(e.to_string()))?;
        diesel::sql_query(store::portable::upsert(
            "rifts_host_presences",
            &["account_id", "revision", "consent_to_host", "observed_at", "expires_at", "detail"],
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
        find_reliability(self.conn, account_id)
    }

    /// Adds to the counters (a fresh row starts at zero); a cooldown, when given, replaces the old one.
    pub fn increment_reliability(
        &mut self,
        account_id: &str,
        add: ReliabilityIncrements,
        now: UnixMillis,
        cooldown_until: Option<UnixMillis>,
    ) -> Result<(), RepoError> {
        let mut r = find_reliability(self.conn, account_id)?.unwrap_or_else(|| HostReliabilityRecord::new(account_id, now));
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
        let columns: Vec<&str> = RELIABILITY_COLUMNS.split(", ").collect();
        diesel::sql_query(store::portable::upsert(
            "rifts_host_reliability",
            &columns,
            &["account_id"],
            &columns[1..],
        ))
        .bind::<Text, _>(r.account_id)
        .bind::<BigInt, _>(r.offers_accepted)
        .bind::<BigInt, _>(r.offers_declined)
        .bind::<BigInt, _>(r.offers_expired)
        .bind::<BigInt, _>(r.starts_succeeded)
        .bind::<BigInt, _>(r.starts_failed)
        .bind::<BigInt, _>(r.clean_completions)
        .bind::<BigInt, _>(r.unexpected_failures)
        .bind::<Nullable<BigInt>, _>(r.cooldown_until.map(|t| t.0))
        .bind::<BigInt, _>(r.updated_at.0)
        .execute(self.conn)?;
        Ok(())
    }

    /// One accepted offer per generation.
    pub fn create_offer(&mut self, o: &HostOfferRecord) -> Result<(), RepoError> {
        if o.status == OfferStatus::Accepted {
            let accepted = sql("SELECT COUNT(*) AS n FROM rifts_host_offers WHERE session_id = ? \
                 AND generation = ? AND status = ?")?
            .bind::<Text, _>(o.session_id.clone())
            .bind::<BigInt, _>(o.generation)
            .bind::<Text, _>(OfferStatus::Accepted.as_str())
            .load::<Count>(self.conn)?
            .into_iter()
            .next()
            .map_or(0, |c| c.n);
            if accepted > 0 {
                return Err(RepoError::Duplicate("accepted offer"));
            }
        }
        let breakdown = serde_json::to_string(&o.score_breakdown).map_err(|e| RepoError::Corrupt(e.to_string()))?;
        let columns: Vec<&str> = OFFER_COLUMNS.split(", ").collect();
        diesel::sql_query(store::portable::upsert("rifts_host_offers", &columns, &["offer_id"], &columns[1..]))
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
            .bind::<Nullable<BigInt>, _>(o.responded_at.map(|t| t.0))
            .bind::<Nullable<Text>, _>(o.response_reason.clone())
            .execute(self.conn)?;
        Ok(())
    }

    pub fn find_offer(&mut self, offer_id: &str) -> Result<Option<HostOfferRecord>, RepoError> {
        find_offer(self.conn, offer_id)
    }

    /// Moves the account's offer from `expected` to `status`; `None` when it isn't that account's or has moved on.
    pub fn update_offer(
        &mut self,
        offer_id: &str,
        account_id: &str,
        expected: OfferStatus,
        status: OfferStatus,
        responded_at: UnixMillis,
        reason: Option<&str>,
    ) -> Result<Option<HostOfferRecord>, RepoError> {
        let changed = sql("UPDATE rifts_host_offers SET status = ?, responded_at = ?, response_reason = ? \
             WHERE offer_id = ? AND account_id = ? AND status = ?")?
        .bind::<Text, _>(status.as_str())
        .bind::<BigInt, _>(responded_at.0)
        .bind::<Nullable<Text>, _>(reason.map(str::to_owned))
        .bind::<Text, _>(offer_id.to_owned())
        .bind::<Text, _>(account_id.to_owned())
        .bind::<Text, _>(expected.as_str())
        .execute(self.conn)?;
        if changed == 0 {
            return Ok(None);
        }
        find_offer(self.conn, offer_id)
    }

    /// The hub generation ended: its live presence rows become LOST in this same transaction.
    pub fn end_hub_presence(
        &mut self,
        session_id: &str,
        generation: i64,
        at: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Vec<crate::repo::hub::HubPresenceRecord>, RepoError> {
        crate::repo::hub::end_generation(self.conn, session_id, generation, at, purge_at)
    }

    pub fn record_observation(&mut self, o: &ObservationRecord) -> Result<(), RepoError> {
        sql(&format!(
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

#[derive(Clone)]
pub struct HuntRepository {
    db: RealmsDb,
}

impl HuntRepository {
    pub fn new(db: RealmsDb) -> Self {
        Self { db }
    }

    /// Runs `f` as one write transaction; an `Err` (a refusal included) rolls every step back.
    pub async fn transaction<T, E, F>(&self, actor: &Actor, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut HuntTx<'_>) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<store::Error> + From<diesel::result::Error> + Send + 'static,
    {
        let node = self.db.node().clone();
        self.db
            .write(actor, move |conn| f(&mut HuntTx { conn, node: &node }))
            .await
    }

    async fn tx<T, F>(&self, actor: &Actor, f: F) -> Result<T, RepoError>
    where
        F: FnOnce(&mut HuntTx<'_>) -> Result<T, RepoError> + Send + 'static,
        T: Send + 'static,
    {
        self.transaction(actor, f).await
    }

    pub async fn find_hunt(&self, session_id: &str) -> Result<Option<HuntSession>, RepoError> {
        let id = session_id.to_owned();
        self.db.read(move |conn| load_session(conn, &id)).await
    }

    /// By `activePartyKey` (a party's, or a realm hub kind's).
    pub async fn find_active_hunt_for_party(&self, party_key: &str) -> Result<Option<HuntSession>, RepoError> {
        let key = party_key.to_owned();
        self.db
            .read(move |conn| {
                let ids = sql("SELECT session_id FROM rifts_hunt_sessions WHERE active_party_key = ?")?
                    .bind::<Text, _>(key)
                    .load::<IdRow>(conn)?;
                Ok(load_many(conn, ids)?.into_iter().next())
            })
            .await
    }

    /// The account's newest active hunt (never a hub).
    pub async fn find_active_hunt_for_account(&self, account_id: &str) -> Result<Option<HuntSession>, RepoError> {
        let account = account_id.to_owned();
        self.db
            .read(move |conn| {
                let ids = sql("SELECT s.session_id FROM rifts_hunt_sessions s JOIN rifts_hunt_roster r \
                     ON r.session_id = s.session_id WHERE r.account_id = ? AND s.active_party_key IS NOT NULL \
                     AND s.session_kind IS NULL ORDER BY s.created_at DESC, s.session_id LIMIT 1")?
                .bind::<Text, _>(account)
                .load::<IdRow>(conn)?;
                Ok(load_many(conn, ids)?.into_iter().next())
            })
            .await
    }

    pub async fn create_hunt(&self, actor: &Actor, session: &HuntSession) -> Result<(), RepoError> {
        let session = session.clone();
        self.tx(actor, move |tx| tx.create_hunt(&session)).await
    }

    pub async fn find_recoverable_hunt_for_account(
        &self,
        account_id: &str,
        now: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let account = account_id.to_owned();
        self.db
            .read(move |conn| {
                let ids = sql("SELECT s.session_id FROM rifts_hunt_sessions s JOIN rifts_hunt_roster r \
                     ON r.session_id = s.session_id WHERE r.account_id = ? AND s.state = ? AND s.expires_at > ? \
                     ORDER BY s.created_at DESC, s.session_id LIMIT 1")?
                .bind::<Text, _>(account)
                .bind::<Text, _>(HuntState::Recovering.as_str())
                .bind::<BigInt, _>(now.0)
                .load::<IdRow>(conn)?;
                Ok(load_many(conn, ids)?.into_iter().next())
            })
            .await
    }

    /// Oldest first.
    pub async fn list_lifecycle_hunts(&self, states: &[HuntState], limit: u32) -> Result<Vec<HuntSession>, RepoError> {
        let states = states.to_vec();
        self.db
            .read(move |conn| {
                let mut query = sql(&format!(
                    "SELECT session_id FROM rifts_hunt_sessions WHERE {} ORDER BY created_at, session_id LIMIT ?",
                    states_in(&states)
                ))?;
                for state in &states {
                    query = query.bind::<Text, _>(state.as_str());
                }
                let ids = query.bind::<BigInt, _>(i64::from(limit)).load::<IdRow>(conn)?;
                load_many(conn, ids)
            })
            .await
    }

    /// `compareAndSetHunt` as a transaction of its own.
    pub async fn compare_and_set_hunt<F>(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        expected_states: &[HuntState],
        expected_state_version: i64,
        change: F,
    ) -> Result<Option<HuntSession>, RepoError>
    where
        F: FnOnce(&mut HuntSession) + Send + 'static,
    {
        let (id, states) = (session_id.to_owned(), expected_states.to_vec());
        self.tx(actor, move |tx| tx.compare_and_set(&id, generation, &states, expected_state_version, change))
            .await
    }

    pub async fn append_hunt_transition(&self, actor: &Actor, transition: &HuntTransition) -> Result<(), RepoError> {
        let t = transition.clone();
        self.tx(actor, move |tx| tx.append_transition(&t)).await
    }

    /// The session's transitions, in the order they happened.
    pub async fn list_transitions(&self, session_id: &str) -> Result<Vec<HuntTransition>, RepoError> {
        let id = session_id.to_owned();
        self.db
            .read(move |conn| {
                sql("SELECT transition_id, session_id, generation, from_state, to_state, state_version, reason, \
                     actor_type, actor_account_id, at FROM rifts_hunt_transitions WHERE session_id = ? \
                     ORDER BY at, state_version")?
                .bind::<Text, _>(id)
                .load::<TransitionRow>(conn)?
                .into_iter()
                .map(|r| {
                    Ok(HuntTransition {
                        from_state: HuntState::parse(&r.from_state).ok_or_else(|| corrupt("state", &r.from_state))?,
                        to_state: HuntState::parse(&r.to_state).ok_or_else(|| corrupt("state", &r.to_state))?,
                        transition_id: r.transition_id,
                        session_id: r.session_id,
                        generation: r.generation,
                        state_version: r.state_version,
                        reason: r.reason,
                        actor_type: r.actor_type,
                        actor_account_id: r.actor_account_id,
                        at: UnixMillis(r.at),
                    })
                })
                .collect()
            })
            .await
    }

    pub async fn begin_host_startup(
        &self,
        actor: &Actor,
        startup: &HostStartup,
        now: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let s = startup.clone();
        self.tx(actor, move |tx| tx.begin_host_startup(&s, now)).await
    }

    pub async fn renew_host_lease(&self, actor: &Actor, renewal: &LeaseRenewal) -> Result<Option<HuntSession>, RepoError> {
        let r = renewal.clone();
        self.tx(actor, move |tx| tx.renew_host_lease(&r)).await
    }

    pub async fn mark_host_ready(
        &self,
        actor: &Actor,
        mark: &HostReadyMark,
        now: UnixMillis,
    ) -> Result<Option<HuntSession>, RepoError> {
        let m = mark.clone();
        self.tx(actor, move |tx| tx.mark_host_ready(&m, now)).await
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
        let (id, account, steam, bridge) =
            (session_id.to_owned(), account_id.to_owned(), steam_id.to_owned(), bridge_instance_id.to_owned());
        self.tx(actor, move |tx| {
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
        self.tx(actor, move |tx| tx.mark_participant_admitted(&id, generation, &account, admitted_at))
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
        self.tx(actor, move |tx| {
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
        self.tx(actor, move |tx| {
            tx.mark_participant_disconnected(&id, generation, &account, disconnected_at, rejoin_deadline)
        })
        .await
    }

    pub async fn create_host_capability(&self, actor: &Actor, capability: &HostCapabilityRecord) -> Result<(), RepoError> {
        let c = capability.clone();
        self.tx(actor, move |tx| tx.create_capability(&c)).await
    }

    pub async fn find_host_capability_by_digest(&self, digest: &str) -> Result<Option<HostCapabilityRecord>, RepoError> {
        let digest = digest.to_owned();
        self.db
            .read(move |conn| {
                sql(&format!("SELECT {CAPABILITY_COLUMNS} FROM rifts_host_capabilities WHERE secret_digest = ?"))?
                    .bind::<Text, _>(digest)
                    .load::<CapabilityRow>(conn)?
                    .into_iter()
                    .next()
                    .map(capability_of)
                    .transpose()
            })
            .await
    }

    pub async fn advance_host_capability_sequence(
        &self,
        actor: &Actor,
        capability_id: &str,
        sequence: i64,
    ) -> Result<bool, RepoError> {
        let id = capability_id.to_owned();
        self.tx(actor, move |tx| tx.advance_capability_sequence(&id, sequence)).await
    }

    pub async fn revoke_generation(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        revoked_at: UnixMillis,
    ) -> Result<(), RepoError> {
        let id = session_id.to_owned();
        self.tx(actor, move |tx| tx.revoke_generation(&id, generation, revoked_at)).await
    }

    pub async fn enqueue_control_message(
        &self,
        actor: &Actor,
        message: &NewControlMessage,
    ) -> Result<ControlMessageRecord, RepoError> {
        let m = message.clone();
        self.tx(actor, move |tx| tx.enqueue_control(&m)).await
    }

    /// Unexpired messages after the cursor, by sequence; `unacknowledged_only` for the stale-cursor answer.
    pub async fn list_control_messages(
        &self,
        account_id: &str,
        after_sequence: i64,
        now: UnixMillis,
        limit: u32,
        unacknowledged_only: bool,
    ) -> Result<Vec<ControlMessageRecord>, RepoError> {
        let account = account_id.to_owned();
        self.db
            .read(move |conn| {
                sql(&format!(
                    "SELECT {MESSAGE_COLUMNS} FROM rifts_control_messages WHERE account_id = ? AND sequence > ? \
                     AND expires_at > ?{} ORDER BY sequence LIMIT ?",
                    if unacknowledged_only { " AND acknowledged_at IS NULL" } else { "" }
                ))?
                .bind::<Text, _>(account)
                .bind::<BigInt, _>(after_sequence)
                .bind::<BigInt, _>(now.0)
                .bind::<BigInt, _>(i64::from(limit))
                .load::<MessageRow>(conn)?
                .into_iter()
                .map(message_of)
                .collect()
            })
            .await
    }

    /// The account's last sequence, 0 when it never had a message.
    pub async fn latest_control_sequence(&self, account_id: &str) -> Result<i64, RepoError> {
        let account = account_id.to_owned();
        self.db
            .read(move |conn| {
                count(
                    conn,
                    "SELECT COALESCE(MAX(next_sequence), 0) AS n FROM rifts_control_sequences WHERE account_id = ?",
                    &[&account],
                )
            })
            .await
    }

    pub async fn acknowledge_control_message(
        &self,
        actor: &Actor,
        account_id: &str,
        message_id: &str,
        at: UnixMillis,
    ) -> Result<bool, RepoError> {
        let (account, id) = (account_id.to_owned(), message_id.to_owned());
        self.tx(actor, move |tx| tx.acknowledge_control(&account, &id, at)).await
    }

    pub async fn upsert_host_presence(&self, actor: &Actor, presence: &HostPresenceRecord) -> Result<(), RepoError> {
        let p = presence.clone();
        self.tx(actor, move |tx| tx.upsert_host_presence(&p)).await
    }

    /// The accounts' presences observed since `observed_after` and unexpired at `now`, in the accounts' order.
    pub async fn list_host_presences(
        &self,
        account_ids: &[String],
        observed_after: UnixMillis,
        now: UnixMillis,
    ) -> Result<Vec<HostPresenceRecord>, RepoError> {
        if account_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = account_ids.to_vec();
        self.db
            .read(move |conn| {
                let mut query = sql(&format!(
                    "SELECT detail FROM rifts_host_presences WHERE account_id IN ({}) AND observed_at >= ? \
                     AND expires_at > ?",
                    marks(ids.len())
                ))?;
                for id in &ids {
                    query = query.bind::<Text, _>(id.clone());
                }
                let mut presences = query
                    .bind::<BigInt, _>(observed_after.0)
                    .bind::<BigInt, _>(now.0)
                    .load::<DetailRow>(conn)?
                    .into_iter()
                    .map(|row| {
                        serde_json::from_str::<HostPresenceRecord>(&row.detail)
                            .map_err(|e| RepoError::Corrupt(e.to_string()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                presences.sort_by_key(|p| ids.iter().position(|id| *id == p.account_id));
                Ok(presences)
            })
            .await
    }

    /// The host's live sessions that hold their party key (a draining one has given it up).
    pub async fn count_active_host_sessions(&self, account_id: &str) -> Result<i64, RepoError> {
        self.count_keyed("host_account_id", account_id, LIVE_HOST_STATES).await
    }

    /// The realm's live servers (hub, Training and hunts) its server limit is checked against.
    pub async fn count_active_realm_servers(&self, realm_id: &str) -> Result<i64, RepoError> {
        self.count_keyed("realm_id", realm_id, REALM_SERVER_STATES).await
    }

    async fn count_keyed(&self, column: &'static str, value: &str, states: &'static [HuntState]) -> Result<i64, RepoError> {
        let value = value.to_owned();
        self.db
            .read(move |conn| {
                let mut query = sql(&format!(
                    "SELECT COUNT(*) AS n FROM rifts_hunt_sessions WHERE {column} = ? AND active_party_key IS NOT NULL \
                     AND {}",
                    states_in(states)
                ))?
                .bind::<Text, _>(value);
                for state in states {
                    query = query.bind::<Text, _>(state.as_str());
                }
                Ok(query.load::<Count>(conn)?.into_iter().next().map_or(0, |c| c.n))
            })
            .await
    }

    /// The realm's sessions whose server still runs on the host's PC, a draining one included.
    pub async fn count_running_realm_servers(&self, realm_id: &str) -> Result<i64, RepoError> {
        let realm = realm_id.to_owned();
        self.db
            .read(move |conn| {
                let mut query = sql(&format!(
                    "SELECT COUNT(*) AS n FROM rifts_hunt_sessions WHERE realm_id = ? AND \
                     ((active_party_key IS NOT NULL AND {}) OR state = ?)",
                    states_in(REALM_SERVER_STATES)
                ))?
                .bind::<Text, _>(realm);
                for state in REALM_SERVER_STATES {
                    query = query.bind::<Text, _>(state.as_str());
                }
                let query = query.bind::<Text, _>(Draining.as_str());
                Ok(query.load::<Count>(conn)?.into_iter().next().map_or(0, |c| c.n))
            })
            .await
    }

    /// Live realm hunts (never hubs) of this realm and host, at most 64.
    pub async fn list_live_realm_hunts(&self, realm_id: &str, host: &str) -> Result<Vec<HuntSession>, RepoError> {
        let (realm, host) = (realm_id.to_owned(), host.to_owned());
        self.db
            .read(move |conn| {
                let mut query = sql(&format!(
                    "SELECT session_id FROM rifts_hunt_sessions WHERE realm_id = ? AND host_account_id = ? \
                     AND session_kind IS NULL AND active_party_key IS NOT NULL AND {} ORDER BY created_at LIMIT 64",
                    states_in(REALM_SERVER_STATES)
                ))?
                .bind::<Text, _>(realm)
                .bind::<Text, _>(host);
                for state in REALM_SERVER_STATES {
                    query = query.bind::<Text, _>(state.as_str());
                }
                let ids = query.load::<IdRow>(conn)?;
                load_many(conn, ids)
            })
            .await
    }

    pub async fn find_host_reliability(&self, account_id: &str) -> Result<Option<HostReliabilityRecord>, RepoError> {
        let account = account_id.to_owned();
        self.db.read(move |conn| find_reliability(conn, &account)).await
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
        self.tx(actor, move |tx| tx.increment_reliability(&account, add, now, cooldown_until))
            .await
    }

    pub async fn create_host_offer(&self, actor: &Actor, offer: &HostOfferRecord) -> Result<(), RepoError> {
        let o = offer.clone();
        self.tx(actor, move |tx| tx.create_offer(&o)).await
    }

    pub async fn find_host_offer(&self, offer_id: &str) -> Result<Option<HostOfferRecord>, RepoError> {
        let id = offer_id.to_owned();
        self.db.read(move |conn| find_offer(conn, &id)).await
    }

    /// By rank.
    pub async fn list_host_offers(&self, session_id: &str, generation: i64) -> Result<Vec<HostOfferRecord>, RepoError> {
        let id = session_id.to_owned();
        self.db
            .read(move |conn| {
                sql(&format!(
                    "SELECT {OFFER_COLUMNS} FROM rifts_host_offers WHERE session_id = ? AND generation = ? ORDER BY rank"
                ))?
                .bind::<Text, _>(id)
                .bind::<BigInt, _>(generation)
                .load::<OfferRow>(conn)?
                .into_iter()
                .map(offer_of)
                .collect()
            })
            .await
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
        self.tx(actor, move |tx| {
            tx.update_offer(&id, &account, expected, status, responded_at, reason.as_deref())
        })
        .await
    }

    pub async fn record_participant_observation(&self, actor: &Actor, observation: &ObservationRecord) -> Result<(), RepoError> {
        let o = observation.clone();
        self.tx(actor, move |tx| tx.record_observation(&o)).await
    }

    /// Newest first.
    pub async fn list_recent_participant_observations(
        &self,
        session_id: &str,
        generation: i64,
        observed_after: UnixMillis,
    ) -> Result<Vec<ObservationRecord>, RepoError> {
        let id = session_id.to_owned();
        self.db
            .read(move |conn| {
                Ok(sql(&format!(
                    "SELECT {OBSERVATION_COLUMNS} FROM rifts_participant_observations WHERE session_id = ? \
                     AND generation = ? AND observed_at >= ? ORDER BY observed_at DESC, observation_id"
                ))?
                .bind::<Text, _>(id)
                .bind::<BigInt, _>(generation)
                .bind::<BigInt, _>(observed_after.0)
                .load::<ObservationRow>(conn)?
                .into_iter()
                .map(|r| ObservationRecord {
                    observation_id: r.observation_id,
                    session_id: r.session_id,
                    generation: r.generation,
                    account_id: r.account_id,
                    steam_id: r.steam_id,
                    host_reachable: r.host_reachable,
                    local_bridge_alive: r.local_bridge_alive,
                    metrics_available: r.metrics_available,
                    last_packet_age_ms: r.last_packet_age_ms,
                    route_kind: r.route_kind,
                    ping_ms: r.ping_ms,
                    jitter_ms: r.jitter_ms,
                    packet_loss_percent: r.packet_loss_percent,
                    observed_at: UnixMillis(r.observed_at),
                })
                .collect())
            })
            .await
    }

    /// Node's TTL indexes: control messages and host presences once expired.
    pub async fn purge_expired(&self, now: UnixMillis) -> Result<usize, RepoError> {
        self.db
            .write(&Actor::System("realms-purge"), move |conn| {
                let messages = sql("DELETE FROM rifts_control_messages WHERE expires_at <= ?")?
                    .bind::<BigInt, _>(now.0)
                    .execute(conn)?;
                let presences = sql("DELETE FROM rifts_host_presences WHERE expires_at <= ?")?
                    .bind::<BigInt, _>(now.0)
                    .execute(conn)?;
                Ok::<_, RepoError>(messages + presences)
            })
            .await
    }
}
