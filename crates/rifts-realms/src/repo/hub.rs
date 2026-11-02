// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Who is in a realm hub: one row per (hub session, account), written without touching the session row. Every move
//! of a row follows the presence table in `crate::hub`.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text};
use rifts_seam::{Actor, UnixMillis};
use store::AnyConnection;

use super::{Boxed, Count, corrupt, limit, placeholders, query};
use crate::db::{RealmsDb, RepoError};
use crate::hub::{
    EntryDecision, PresenceEvent, PresenceState, decide_entry_request, is_presence_expired, is_presence_live,
    next_presence_state,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubPresenceRecord {
    pub session_id: String,
    pub generation: i64,
    pub realm_id: String,
    pub account_id: String,
    /// The transport peer id the ticket and the link carry.
    pub peer_id: String,
    pub state: PresenceState,
    /// The matchmaking candidate of the current entry.
    pub candidate_id: String,
    /// 1 for the first entry, one more per re-entry.
    pub entry_epoch: i64,
    pub local_bridge_port: Option<i64>,
    pub local_bridge_ready_at: Option<UnixMillis>,
    pub admission_id: Option<String>,
    /// `relay` or `direct`.
    pub route: Option<String>,
    pub requested_at: UnixMillis,
    pub ticketed_at: Option<UnixMillis>,
    pub admitted_at: Option<UnixMillis>,
    pub lost_at: Option<UnixMillis>,
    pub left_at: Option<UnixMillis>,
    pub left_reason: Option<String>,
    /// Waiting rows expire then.
    pub expires_at: UnixMillis,
    /// Set once the row stops being live.
    pub purge_at: Option<UnixMillis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryOutcome {
    Created,
    Reused,
    Reentered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubEntryRequest {
    pub account_id: String,
    pub peer_id: String,
    pub candidate_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryAt {
    pub session_id: String,
    pub generation: i64,
    pub realm_id: String,
    pub now: UnixMillis,
    pub expires_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubEntryBatch {
    /// (account id, row, outcome), in request order.
    Placed(Vec<(String, HubPresenceRecord, EntryOutcome)>),
    /// Nothing was written.
    Full { live: i64, max_peers: i64 },
}

const COLUMNS: [&str; 20] = [
    "session_id",
    "account_id",
    "generation",
    "realm_id",
    "peer_id",
    "state",
    "candidate_id",
    "entry_epoch",
    "local_bridge_port",
    "local_bridge_ready_at",
    "admission_id",
    "route",
    "requested_at",
    "ticketed_at",
    "admitted_at",
    "lost_at",
    "left_at",
    "left_reason",
    "expires_at",
    "purge_at",
];

#[derive(QueryableByName)]
struct Row {
    #[diesel(sql_type = Text)]
    session_id: String,
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = BigInt)]
    generation: i64,
    #[diesel(sql_type = Text)]
    realm_id: String,
    #[diesel(sql_type = Text)]
    peer_id: String,
    #[diesel(sql_type = Text)]
    state: String,
    #[diesel(sql_type = Text)]
    candidate_id: String,
    #[diesel(sql_type = BigInt)]
    entry_epoch: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    local_bridge_port: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    local_bridge_ready_at: Option<i64>,
    #[diesel(sql_type = Nullable<Text>)]
    admission_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    route: Option<String>,
    #[diesel(sql_type = BigInt)]
    requested_at: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    ticketed_at: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    admitted_at: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    lost_at: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    left_at: Option<i64>,
    #[diesel(sql_type = Nullable<Text>)]
    left_reason: Option<String>,
    #[diesel(sql_type = BigInt)]
    expires_at: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    purge_at: Option<i64>,
}

impl Row {
    fn into_record(self) -> Result<HubPresenceRecord, RepoError> {
        Ok(HubPresenceRecord {
            state: PresenceState::parse(&self.state).ok_or_else(|| corrupt("hub presence state", &self.state))?,
            session_id: self.session_id,
            generation: self.generation,
            realm_id: self.realm_id,
            account_id: self.account_id,
            peer_id: self.peer_id,
            candidate_id: self.candidate_id,
            entry_epoch: self.entry_epoch,
            local_bridge_port: self.local_bridge_port,
            local_bridge_ready_at: self.local_bridge_ready_at.map(UnixMillis),
            admission_id: self.admission_id,
            route: self.route,
            requested_at: UnixMillis(self.requested_at),
            ticketed_at: self.ticketed_at.map(UnixMillis),
            admitted_at: self.admitted_at.map(UnixMillis),
            lost_at: self.lost_at.map(UnixMillis),
            left_at: self.left_at.map(UnixMillis),
            left_reason: self.left_reason,
            expires_at: UnixMillis(self.expires_at),
            purge_at: self.purge_at.map(UnixMillis),
        })
    }
}

fn select(
    conn: &mut AnyConnection,
    tail: &str,
    bind: impl FnOnce(Boxed) -> Boxed,
) -> Result<Vec<HubPresenceRecord>, RepoError> {
    let rows = bind(query(&format!(
        "SELECT {} FROM rifts_hub_presence {tail}",
        COLUMNS.join(", ")
    ))?)
    .load::<Row>(conn)?;
    rows.into_iter().map(Row::into_record).collect()
}

fn find(conn: &mut AnyConnection, session_id: &str, account_id: &str) -> Result<Option<HubPresenceRecord>, RepoError> {
    let (session, account) = (session_id.to_owned(), account_id.to_owned());
    Ok(select(conn, "WHERE session_id = ? AND account_id = ?", |q| {
        q.bind::<Text, _>(session).bind::<Text, _>(account)
    })?
    .into_iter()
    .next())
}

/// The whole row, upserted on its key.
fn put(conn: &mut AnyConnection, row: &HubPresenceRecord) -> Result<(), RepoError> {
    let text = store::portable::upsert("rifts_hub_presence", &COLUMNS, &COLUMNS[..2], &COLUMNS[2..]);
    diesel::sql_query(text)
        .into_boxed::<store::MultiBackend>()
        .bind::<Text, _>(row.session_id.clone())
        .bind::<Text, _>(row.account_id.clone())
        .bind::<BigInt, _>(row.generation)
        .bind::<Text, _>(row.realm_id.clone())
        .bind::<Text, _>(row.peer_id.clone())
        .bind::<Text, _>(row.state.as_str())
        .bind::<Text, _>(row.candidate_id.clone())
        .bind::<BigInt, _>(row.entry_epoch)
        .bind::<Nullable<BigInt>, _>(row.local_bridge_port)
        .bind::<Nullable<BigInt>, _>(row.local_bridge_ready_at.map(|at| at.0))
        .bind::<Nullable<Text>, _>(row.admission_id.clone())
        .bind::<Nullable<Text>, _>(row.route.clone())
        .bind::<BigInt, _>(row.requested_at.0)
        .bind::<Nullable<BigInt>, _>(row.ticketed_at.map(|at| at.0))
        .bind::<Nullable<BigInt>, _>(row.admitted_at.map(|at| at.0))
        .bind::<Nullable<BigInt>, _>(row.lost_at.map(|at| at.0))
        .bind::<Nullable<BigInt>, _>(row.left_at.map(|at| at.0))
        .bind::<Nullable<Text>, _>(row.left_reason.clone())
        .bind::<BigInt, _>(row.expires_at.0)
        .bind::<Nullable<BigInt>, _>(row.purge_at.map(|at| at.0))
        .execute(conn)?;
    Ok(())
}

fn clear_connection(row: &mut HubPresenceRecord) {
    row.local_bridge_port = None;
    row.local_bridge_ready_at = None;
    row.admission_id = None;
    row.route = None;
    row.ticketed_at = None;
    row.admitted_at = None;
    row.lost_at = None;
    row.left_at = None;
    row.left_reason = None;
    row.purge_at = None;
}

/// One entry under the domain's entry decision; writes only what changed.
fn enter(
    conn: &mut AnyConnection,
    at: &EntryAt,
    request: &HubEntryRequest,
    existing: Option<HubPresenceRecord>,
) -> Result<(HubPresenceRecord, EntryOutcome), RepoError> {
    let decision = decide_entry_request(existing.as_ref().map(|row| (row.state, row.expires_at)), at.now);
    let (row, outcome) = match (decision, existing) {
        (EntryDecision::Reuse, Some(mut row)) => {
            // Expiry only grows.
            if at.expires_at > row.expires_at {
                row.expires_at = at.expires_at;
                put(conn, &row)?;
            }
            return Ok((row, EntryOutcome::Reused));
        }
        (EntryDecision::Reenter, Some(mut row)) => {
            row.generation = at.generation;
            row.realm_id = at.realm_id.clone();
            row.peer_id = request.peer_id.clone();
            row.candidate_id = request.candidate_id.clone();
            row.state = PresenceState::Pending;
            row.requested_at = at.now;
            row.expires_at = at.expires_at;
            row.entry_epoch += 1;
            clear_connection(&mut row);
            (row, EntryOutcome::Reentered)
        }
        _ => (
            HubPresenceRecord {
                session_id: at.session_id.clone(),
                generation: at.generation,
                realm_id: at.realm_id.clone(),
                account_id: request.account_id.clone(),
                peer_id: request.peer_id.clone(),
                state: PresenceState::Pending,
                candidate_id: request.candidate_id.clone(),
                entry_epoch: 1,
                local_bridge_port: None,
                local_bridge_ready_at: None,
                admission_id: None,
                route: None,
                requested_at: at.now,
                ticketed_at: None,
                admitted_at: None,
                lost_at: None,
                left_at: None,
                left_reason: None,
                expires_at: at.expires_at,
                purge_at: None,
            },
            EntryOutcome::Created,
        ),
    };
    put(conn, &row)?;
    Ok((row, outcome))
}

fn states_clause(states: &[PresenceState]) -> String {
    format!("state IN ({})", placeholders(states.len()))
}

fn bind_states(mut q: Boxed, states: &[PresenceState]) -> Boxed {
    for state in states {
        q = q.bind::<Text, _>(state.as_str());
    }
    q
}

/// Moves each selected row that has a move for `event`; answers the moved rows as they were before.
fn bulk(
    conn: &mut AnyConnection,
    rows: Vec<HubPresenceRecord>,
    event: PresenceEvent,
    change: impl Fn(&mut HubPresenceRecord),
) -> Result<Vec<HubPresenceRecord>, RepoError> {
    let mut moved = Vec::new();
    for row in rows {
        let Some(next) = next_presence_state(row.state, event) else {
            continue;
        };
        let mut changed = row.clone();
        changed.state = next;
        change(&mut changed);
        put(conn, &changed)?;
        moved.push(row);
    }
    Ok(moved)
}

pub(crate) fn end_generation(
    conn: &mut AnyConnection,
    session_id: &str,
    generation: i64,
    at: UnixMillis,
    purge_at: UnixMillis,
) -> Result<Vec<HubPresenceRecord>, RepoError> {
    let session = session_id.to_owned();
    let rows = select(
        conn,
        "WHERE session_id = ? AND generation = ? ORDER BY account_id",
        |q| q.bind::<Text, _>(session).bind::<BigInt, _>(generation),
    )?;
    bulk(conn, rows, PresenceEvent::SessionEnded, |row| {
        row.lost_at = Some(at);
        row.purge_at = Some(purge_at);
    })
}

/// The hub presence rows of every realm hub.
#[derive(Clone)]
pub struct RealmHubRepository {
    db: RealmsDb,
}

impl RealmHubRepository {
    pub fn new(db: RealmsDb) -> Self {
        Self { db }
    }

    /// No capacity check: tests only; the routes use `request_entries`.
    pub async fn request_entry(
        &self,
        actor: &Actor,
        at: &EntryAt,
        request: &HubEntryRequest,
    ) -> Result<(HubPresenceRecord, EntryOutcome), RepoError> {
        let (at, request) = (at.clone(), request.clone());
        self.db
            .write(actor, move |conn| {
                let existing = find(conn, &at.session_id, &request.account_id)?;
                enter(conn, &at, &request, existing)
            })
            .await
    }

    /// All seats in one step or none: a member whose own row is live takes no seat and is never refused.
    pub async fn request_entries(
        &self,
        actor: &Actor,
        at: &EntryAt,
        max_peers: i64,
        entries: &[HubEntryRequest],
    ) -> Result<HubEntryBatch, RepoError> {
        // An account listed twice keeps its first place and its last value.
        let mut requests: Vec<HubEntryRequest> = Vec::with_capacity(entries.len());
        for entry in entries {
            match requests.iter_mut().find(|r| r.account_id == entry.account_id) {
                Some(slot) => *slot = entry.clone(),
                None => requests.push(entry.clone()),
            }
        }
        let at = at.clone();
        self.db
            .write(actor, move |conn| {
                let session = at.session_id.clone();
                let generation_rows = select(conn, "WHERE session_id = ? AND generation = ?", |q| {
                    q.bind::<Text, _>(session).bind::<BigInt, _>(at.generation)
                })?;
                let live = generation_rows
                    .iter()
                    .filter(|row| is_presence_live(row.state, row.expires_at, at.now))
                    .count();
                let mut existing = Vec::with_capacity(requests.len());
                for request in &requests {
                    existing.push(find(conn, &at.session_id, &request.account_id)?);
                }
                let newcomers = existing
                    .iter()
                    .filter(|row| {
                        !row.as_ref()
                            .is_some_and(|r| is_presence_live(r.state, r.expires_at, at.now))
                    })
                    .count();
                let (live, newcomers) = (
                    i64::try_from(live).unwrap_or(i64::MAX),
                    i64::try_from(newcomers).unwrap_or(i64::MAX),
                );
                if newcomers > 0 && live.saturating_add(newcomers) > max_peers {
                    return Ok(HubEntryBatch::Full { live, max_peers });
                }
                let mut placed = Vec::with_capacity(requests.len());
                for (request, row) in requests.iter().zip(existing) {
                    let (row, outcome) = enter(conn, &at, request, row)?;
                    placed.push((request.account_id.clone(), row, outcome));
                }
                Ok(HubEntryBatch::Placed(placed))
            })
            .await
    }

    /// The host's own row at hub start, created or replaced as connected.
    #[allow(clippy::too_many_arguments)]
    pub async fn admit_host(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        realm_id: &str,
        account_id: &str,
        peer_id: &str,
        local_bridge_port: i64,
        now: UnixMillis,
    ) -> Result<HubPresenceRecord, RepoError> {
        let (session_id, realm_id, account_id, peer_id) = (
            session_id.to_owned(),
            realm_id.to_owned(),
            account_id.to_owned(),
            peer_id.to_owned(),
        );
        self.db
            .write(actor, move |conn| {
                let existing = find(conn, &session_id, &account_id)?;
                let epoch = existing.as_ref().map_or(0, |row| row.entry_epoch);
                let mut row = existing.unwrap_or_else(|| HubPresenceRecord {
                    session_id: session_id.clone(),
                    generation,
                    realm_id: realm_id.clone(),
                    account_id: account_id.clone(),
                    peer_id: peer_id.clone(),
                    state: PresenceState::Admitted,
                    candidate_id: format!("host:{session_id}"),
                    entry_epoch: 0,
                    local_bridge_port: None,
                    local_bridge_ready_at: None,
                    admission_id: None,
                    route: None,
                    requested_at: now,
                    ticketed_at: None,
                    admitted_at: None,
                    lost_at: None,
                    left_at: None,
                    left_reason: None,
                    expires_at: now,
                    purge_at: None,
                });
                clear_connection(&mut row);
                row.generation = generation;
                row.realm_id = realm_id;
                row.peer_id = peer_id;
                row.state = PresenceState::Admitted;
                row.local_bridge_port = Some(local_bridge_port);
                row.local_bridge_ready_at = Some(now);
                row.admitted_at = Some(now);
                row.entry_epoch = epoch + 1;
                put(conn, &row)?;
                Ok(row)
            })
            .await
    }

    pub async fn find_presence(
        &self,
        session_id: &str,
        account_id: &str,
    ) -> Result<Option<HubPresenceRecord>, RepoError> {
        let (session_id, account_id) = (session_id.to_owned(), account_id.to_owned());
        self.db.read(move |conn| find(conn, &session_id, &account_id)).await
    }

    /// The account's newest row in any hub in `states` (the live states when `None`).
    pub async fn find_for_account(
        &self,
        account_id: &str,
        states: Option<&[PresenceState]>,
    ) -> Result<Option<HubPresenceRecord>, RepoError> {
        let account = account_id.to_owned();
        let states = states.unwrap_or(&PresenceState::LIVE).to_vec();
        self.db
            .read(move |conn| {
                let tail = format!(
                    "WHERE account_id = ? AND {} ORDER BY requested_at DESC, session_id LIMIT 1",
                    states_clause(&states)
                );
                Ok(
                    select(conn, &tail, |q| bind_states(q.bind::<Text, _>(account), &states))?
                        .into_iter()
                        .next(),
                )
            })
            .await
    }

    /// Oldest request first.
    pub async fn list_by_state(
        &self,
        session_id: &str,
        generation: i64,
        states: &[PresenceState],
        max: u32,
    ) -> Result<Vec<HubPresenceRecord>, RepoError> {
        let (session, states) = (session_id.to_owned(), states.to_vec());
        self.db
            .read(move |conn| {
                let tail = format!(
                    "WHERE session_id = ? AND generation = ? AND {} ORDER BY requested_at, account_id LIMIT ?",
                    states_clause(&states)
                );
                select(conn, &tail, |q| {
                    bind_states(q.bind::<Text, _>(session).bind::<BigInt, _>(generation), &states)
                        .bind::<BigInt, _>(limit(max))
                })
            })
            .await
    }

    pub async fn count_by_state(
        &self,
        session_id: &str,
        generation: i64,
        states: &[PresenceState],
    ) -> Result<i64, RepoError> {
        let (session, states) = (session_id.to_owned(), states.to_vec());
        self.db
            .read(move |conn| {
                let text = format!(
                    "SELECT COUNT(*) AS n FROM rifts_hub_presence WHERE session_id = ? AND generation = ? AND {}",
                    states_clause(&states)
                );
                let q = bind_states(
                    query(&text)?.bind::<Text, _>(session).bind::<BigInt, _>(generation),
                    &states,
                );
                Ok(q.load::<Count>(conn)?.into_iter().next().map_or(0, |c| c.n))
            })
            .await
    }

    /// Load, check generation, table and guard, change, write; `None` writes nothing.
    async fn transition(
        &self,
        actor: &Actor,
        key: (&str, i64, &str),
        event: PresenceEvent,
        guard: impl FnOnce(&HubPresenceRecord) -> bool + Send + 'static,
        change: impl FnOnce(&mut HubPresenceRecord, PresenceState) + Send + 'static,
    ) -> Result<Option<HubPresenceRecord>, RepoError> {
        let (session_id, generation, account_id) = (key.0.to_owned(), key.1, key.2.to_owned());
        self.db
            .write(actor, move |conn| {
                let Some(mut row) = find(conn, &session_id, &account_id)? else {
                    return Ok(None);
                };
                if row.generation != generation {
                    return Ok(None);
                }
                let Some(next) = next_presence_state(row.state, event) else {
                    return Ok(None);
                };
                if !guard(&row) {
                    return Ok(None);
                }
                change(&mut row, next);
                put(conn, &row)?;
                Ok(Some(row))
            })
            .await
    }

    /// The row lives at least as long as its ticket.
    pub async fn mark_ticketed(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        account_id: &str,
        now: UnixMillis,
        ticket_expires_at: UnixMillis,
    ) -> Result<Option<HubPresenceRecord>, RepoError> {
        self.transition(
            actor,
            (session_id, generation, account_id),
            PresenceEvent::TicketIssued,
            move |row| !is_presence_expired(row.state, row.expires_at, now),
            move |row, next| {
                row.state = next;
                row.ticketed_at = Some(now);
                row.expires_at = row.expires_at.max(ticket_expires_at);
            },
        )
        .await
    }

    /// Records the loopback port only; the state stays.
    #[allow(clippy::too_many_arguments)]
    pub async fn mark_route_ready(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        account_id: &str,
        local_bridge_port: i64,
        ready_at: UnixMillis,
        now: UnixMillis,
    ) -> Result<Option<HubPresenceRecord>, RepoError> {
        self.transition(
            actor,
            (session_id, generation, account_id),
            PresenceEvent::Admitted,
            move |row| !is_presence_expired(row.state, row.expires_at, now),
            move |row, _| {
                row.local_bridge_port = Some(local_bridge_port);
                row.local_bridge_ready_at = Some(ready_at);
            },
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn mark_admitted(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        account_id: &str,
        admission_id: &str,
        admitted_at: UnixMillis,
        now: UnixMillis,
        route: Option<&str>,
    ) -> Result<Option<HubPresenceRecord>, RepoError> {
        let (admission_id, route) = (admission_id.to_owned(), route.map(str::to_owned));
        self.transition(
            actor,
            (session_id, generation, account_id),
            PresenceEvent::Admitted,
            move |row| !is_presence_expired(row.state, row.expires_at, now),
            move |row, next| {
                row.state = next;
                row.admission_id = Some(admission_id);
                row.admitted_at = Some(admitted_at);
                if route.is_some() {
                    row.route = route;
                }
                row.lost_at = None;
                row.left_at = None;
                row.left_reason = None;
                row.purge_at = None;
            },
        )
        .await
    }

    /// Only the current connection's report ends it.
    #[allow(clippy::too_many_arguments)]
    pub async fn mark_lost(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        account_id: &str,
        admission_id: Option<&str>,
        lost_at: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Option<HubPresenceRecord>, RepoError> {
        let admission_id = admission_id.map(str::to_owned);
        self.transition(
            actor,
            (session_id, generation, account_id),
            PresenceEvent::Disconnected,
            move |row| admission_id.is_none() || admission_id == row.admission_id,
            move |row, next| {
                row.state = next;
                row.lost_at = Some(lost_at);
                row.purge_at = Some(purge_at);
            },
        )
        .await
    }

    /// `reason` is `TRAVEL` when the member went into a realm hunt.
    #[allow(clippy::too_many_arguments)]
    pub async fn mark_left(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        account_id: &str,
        reason: &str,
        left_at: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Option<HubPresenceRecord>, RepoError> {
        let reason = reason.to_owned();
        self.transition(
            actor,
            (session_id, generation, account_id),
            PresenceEvent::Travelled,
            |_| true,
            move |row, next| {
                row.state = next;
                row.left_at = Some(left_at);
                row.left_reason = Some(reason);
                row.purge_at = Some(purge_at);
            },
        )
        .await
    }

    async fn remove_where(
        &self,
        actor: &Actor,
        tail: String,
        bind: impl FnOnce(Boxed) -> Boxed + Send + 'static,
        reason: &'static str,
        at: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Vec<HubPresenceRecord>, RepoError> {
        self.db
            .write(actor, move |conn| {
                let rows = select(conn, &tail, bind)?;
                bulk(conn, rows, PresenceEvent::Removed, |row| {
                    row.left_at = Some(at);
                    row.left_reason = Some(reason.to_owned());
                    row.purge_at = Some(purge_at);
                })
            })
            .await
    }

    /// A kick or ban: the account's rows in every hub of the realm.
    pub async fn remove_account_from_realm(
        &self,
        actor: &Actor,
        realm_id: &str,
        account_id: &str,
        at: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Vec<HubPresenceRecord>, RepoError> {
        let (realm, account) = (realm_id.to_owned(), account_id.to_owned());
        self.remove_where(
            actor,
            "WHERE realm_id = ? AND account_id = ? ORDER BY session_id, account_id".to_owned(),
            move |q| q.bind::<Text, _>(realm).bind::<Text, _>(account),
            "REMOVED",
            at,
            purge_at,
        )
        .await
    }

    pub async fn remove_realm(
        &self,
        actor: &Actor,
        realm_id: &str,
        at: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Vec<HubPresenceRecord>, RepoError> {
        let realm = realm_id.to_owned();
        self.remove_where(
            actor,
            "WHERE realm_id = ? ORDER BY session_id, account_id".to_owned(),
            move |q| q.bind::<Text, _>(realm),
            "REALM_RELEASED",
            at,
            purge_at,
        )
        .await
    }

    /// Waiting rows past their time; connected rows have no move and stay.
    pub async fn expire_stale(
        &self,
        actor: &Actor,
        session_id: Option<&str>,
        now: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Vec<HubPresenceRecord>, RepoError> {
        let session = session_id.map(str::to_owned);
        self.db
            .write(actor, move |conn| {
                let rows = match session {
                    Some(session) => select(
                        conn,
                        "WHERE session_id = ? AND expires_at <= ? ORDER BY session_id, account_id",
                        |q| q.bind::<Text, _>(session).bind::<BigInt, _>(now.0),
                    )?,
                    None => select(conn, "WHERE expires_at <= ? ORDER BY session_id, account_id", |q| {
                        q.bind::<BigInt, _>(now.0)
                    })?,
                };
                bulk(conn, rows, PresenceEvent::Expired, |row| row.purge_at = Some(purge_at))
            })
            .await
    }

    /// The hub generation ended: its live rows are lost.
    pub async fn end_session(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        at: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Vec<HubPresenceRecord>, RepoError> {
        let session = session_id.to_owned();
        self.db
            .write(actor, move |conn| {
                end_generation(conn, &session, generation, at, purge_at)
            })
            .await
    }

    /// Node's TTL index: rows a week past being live. Nothing calls this yet (spec-rd27-rifts §7 risk 3).
    pub async fn purge(&self, now: UnixMillis) -> Result<usize, RepoError> {
        self.db
            .write(&Actor::System("realms-purge"), move |conn| {
                Ok(
                    query("DELETE FROM rifts_hub_presence WHERE purge_at IS NOT NULL AND purge_at <= ?")?
                        .bind::<BigInt, _>(now.0)
                        .execute(conn)?,
                )
            })
            .await
    }
}
