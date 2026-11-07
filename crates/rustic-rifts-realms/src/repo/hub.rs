// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Who is in a realm hub: one row per (hub session, account), written without touching the session row. Every move
//! of a row follows the presence table in `crate::hub`.

use dainty_store::AnyConnection;
use rustic_rifts_seam::{Actor, UnixMillis};

use super::sql::{Arg, Bind, Statement, binds, marks, table_row, upsert};
use crate::db::{RealmsDb, RepoError};
use crate::hub::{
    EntryDecision, PresenceEvent, PresenceState, decide_entry_request, is_presence_expired, is_presence_live,
    next_presence_state,
};

/// One account's row in one realm hub session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HubPresenceRecord {
    /// The hub session.
    pub session_id: String,
    /// The hub generation the row belongs to.
    pub generation: i64,
    /// The hub's realm.
    pub realm_id: String,
    /// The account.
    pub account_id: String,
    /// The transport peer id the ticket and the link carry.
    pub peer_id: String,
    /// Where the row stands.
    pub state: PresenceState,
    /// The matchmaking candidate of the current entry.
    pub candidate_id: String,
    /// 1 for the first entry, one more per re-entry.
    pub entry_epoch: i64,
    /// The account's local bridge port, once it reported.
    pub local_bridge_port: Option<i64>,
    /// When its local bridge reported ready.
    pub local_bridge_ready_at: Option<UnixMillis>,
    /// The admission of the current connection.
    pub admission_id: Option<String>,
    /// `relay` or `direct`.
    pub route: Option<String>,
    /// When the current entry was asked for.
    pub requested_at: UnixMillis,
    /// When a ticket was issued.
    pub ticketed_at: Option<UnixMillis>,
    /// When the host admitted the account.
    pub admitted_at: Option<UnixMillis>,
    /// When the connection was lost.
    pub lost_at: Option<UnixMillis>,
    /// When the account left or was removed.
    pub left_at: Option<UnixMillis>,
    /// Why it left, such as `REMOVED` or `REALM_RELEASED`.
    pub left_reason: Option<String>,
    /// Waiting rows expire then.
    pub expires_at: UnixMillis,
    /// Set once the row stops being live.
    pub purge_at: Option<UnixMillis>,
}

/// What a hub entry request did to the account's row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryOutcome {
    /// Made a new row.
    Created,
    /// Kept a waiting row still valid.
    Reused,
    /// Started a new entry on an old row.
    Reentered,
}

/// One account asking to enter a hub.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HubEntryRequest {
    /// The account.
    pub account_id: String,
    /// Its transport peer id.
    pub peer_id: String,
    /// Its matchmaking candidate.
    pub candidate_id: String,
}

/// The hub and time an entry request is for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryAt {
    /// The hub session.
    pub session_id: String,
    /// Its generation.
    pub generation: i64,
    /// Its realm.
    pub realm_id: String,
    /// The time now.
    pub now: UnixMillis,
    /// When a new waiting entry expires.
    pub expires_at: UnixMillis,
}

/// How a batch of hub entry requests went.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HubEntryBatch {
    /// (account id, row, outcome), in request order.
    Placed(Vec<(String, HubPresenceRecord, EntryOutcome)>),
    /// The hub is full; nothing was written.
    Full {
        /// Live rows in the hub.
        live: i64,
        /// The hub's player limit.
        max_peers: i64,
    },
}

const TABLE: &str = "rifts_hub_presence";

// Keyed by the first two columns.
table_row!(PresenceRow for HubPresenceRecord {
    session_id: String,
    account_id: String,
    generation: i64,
    realm_id: String,
    peer_id: String,
    state: PresenceState,
    candidate_id: String,
    entry_epoch: i64,
    local_bridge_port: Option<i64>,
    local_bridge_ready_at: Option<UnixMillis>,
    admission_id: Option<String>,
    route: Option<String>,
    requested_at: UnixMillis,
    ticketed_at: Option<UnixMillis>,
    admitted_at: Option<UnixMillis>,
    lost_at: Option<UnixMillis>,
    left_at: Option<UnixMillis>,
    left_reason: Option<String>,
    expires_at: UnixMillis,
    purge_at: Option<UnixMillis>,
});

/// The rows `tail` selects.
fn rows_where(conn: &mut AnyConnection, tail: &str, args: Vec<Arg>) -> Result<Vec<HubPresenceRecord>, RepoError> {
    let text = format!("SELECT {} FROM {TABLE} {tail}", PresenceRow::COLUMNS.join(", "));
    let rows: Vec<PresenceRow> = Statement::new(&text, args)?.rows(conn)?;
    rows.into_iter().map(PresenceRow::record).collect()
}

fn find(conn: &mut AnyConnection, session_id: &str, account_id: &str) -> Result<Option<HubPresenceRecord>, RepoError> {
    let found = rows_where(
        conn,
        "WHERE session_id = ? AND account_id = ?",
        binds![session_id, account_id],
    )?;
    Ok(found.into_iter().next())
}

/// The whole row, upserted on its key.
fn put(conn: &mut AnyConnection, row: &HubPresenceRecord) -> Result<(), RepoError> {
    let columns = PresenceRow::COLUMNS;
    upsert(conn, TABLE, (columns, &columns[..2], &[]), PresenceRow::values(row))
}

/// `state IN (..)` and its values.
fn in_states(states: &[PresenceState]) -> (String, Vec<Arg>) {
    let clause = format!("state IN ({})", marks(states.len()));
    (clause, states.iter().map(Bind::arg).collect())
}

/// A row no entry has touched yet: nothing connected, nothing ended.
fn blank(session_id: &str, account_id: &str, now: UnixMillis) -> HubPresenceRecord {
    HubPresenceRecord {
        session_id: session_id.to_owned(),
        account_id: account_id.to_owned(),
        requested_at: now,
        expires_at: now,
        state: PresenceState::Pending,
        entry_epoch: 0,
        generation: 0,
        realm_id: String::new(),
        peer_id: String::new(),
        candidate_id: String::new(),
        purge_at: None,
        left_reason: None,
        left_at: None,
        lost_at: None,
        admitted_at: None,
        ticketed_at: None,
        route: None,
        admission_id: None,
        local_bridge_ready_at: None,
        local_bridge_port: None,
    }
}

fn clear_connection(row: &mut HubPresenceRecord) {
    let HubPresenceRecord {
        local_bridge_port,
        local_bridge_ready_at,
        admission_id,
        route,
        ticketed_at,
        admitted_at,
        lost_at,
        left_at,
        left_reason,
        purge_at,
        ..
    } = row;
    (*local_bridge_port, *admission_id, *route, *left_reason) = (None, None, None, None);
    for stamp in [
        local_bridge_ready_at,
        ticketed_at,
        admitted_at,
        lost_at,
        left_at,
        purge_at,
    ] {
        *stamp = None;
    }
}

/// A fresh entry over `row`: pending again from `at`, one epoch on.
fn begin_entry(row: &mut HubPresenceRecord, at: &EntryAt, request: &HubEntryRequest) {
    clear_connection(row);
    row.state = PresenceState::Pending;
    row.entry_epoch += 1;
    row.generation = at.generation;
    row.realm_id.clone_from(&at.realm_id);
    row.requested_at = at.now;
    row.expires_at = at.expires_at;
    row.peer_id.clone_from(&request.peer_id);
    row.candidate_id.clone_from(&request.candidate_id);
}

/// One entry under the domain's entry decision; writes only what changed.
fn enter(
    conn: &mut AnyConnection,
    at: &EntryAt,
    request: &HubEntryRequest,
    existing: Option<HubPresenceRecord>,
) -> Result<(HubPresenceRecord, EntryOutcome), RepoError> {
    let decision = decide_entry_request(existing.as_ref().map(|row| (row.state, row.expires_at)), at.now);
    let (mut row, outcome) = match (decision, existing) {
        (EntryDecision::Reuse, Some(mut row)) => {
            // Expiry only grows.
            if row.expires_at < at.expires_at {
                row.expires_at = at.expires_at;
                put(conn, &row)?;
            }
            return Ok((row, EntryOutcome::Reused));
        }
        (EntryDecision::Reenter, Some(row)) => (row, EntryOutcome::Reentered),
        // A first entry is a re-entry from epoch 0.
        _ => (
            blank(&at.session_id, &request.account_id, at.now),
            EntryOutcome::Created,
        ),
    };
    begin_entry(&mut row, at, request);
    put(conn, &row)?;
    Ok((row, outcome))
}

/// Moves each selected row that has a move for `event`; answers the moved rows as they were before.
fn bulk(
    conn: &mut AnyConnection,
    rows: Vec<HubPresenceRecord>,
    event: PresenceEvent,
    change: impl Fn(&mut HubPresenceRecord),
) -> Result<Vec<HubPresenceRecord>, RepoError> {
    let mut before = Vec::new();
    for row in rows {
        if let Some(next) = next_presence_state(row.state, event) {
            let mut moved = row.clone();
            moved.state = next;
            change(&mut moved);
            put(conn, &moved)?;
            before.push(row);
        }
    }
    Ok(before)
}

pub(crate) fn end_generation(
    conn: &mut AnyConnection,
    session_id: &str,
    generation: i64,
    at: UnixMillis,
    purge_at: UnixMillis,
) -> Result<Vec<HubPresenceRecord>, RepoError> {
    let tail = "WHERE session_id = ? AND generation = ? ORDER BY account_id";
    let rows = rows_where(conn, tail, binds![session_id, generation])?;
    bulk(conn, rows, PresenceEvent::SessionEnded, |row| {
        (row.lost_at, row.purge_at) = (Some(at), Some(purge_at));
    })
}

fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// The hub presence rows of every realm hub.
#[derive(Clone)]
pub struct RealmHubRepository {
    db: RealmsDb,
}

impl RealmHubRepository {
    /// The hub presence table in `db`.
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
            let earlier = requests.iter().position(|r| r.account_id == entry.account_id);
            match earlier {
                Some(index) => requests[index] = entry.clone(),
                None => requests.push(entry.clone()),
            }
        }
        let at = at.clone();
        self.db
            .write(actor, move |conn| {
                let live_now = |row: &HubPresenceRecord| is_presence_live(row.state, row.expires_at, at.now);
                let generation_rows = rows_where(
                    conn,
                    "WHERE session_id = ? AND generation = ?",
                    binds![at.session_id, at.generation],
                )?;
                let existing = requests
                    .iter()
                    .map(|request| find(conn, &at.session_id, &request.account_id))
                    .collect::<Result<Vec<_>, _>>()?;
                let live = count(generation_rows.iter().filter(|row| live_now(row)).count());
                let newcomers = count(
                    existing
                        .iter()
                        .filter(|row| !row.as_ref().is_some_and(live_now))
                        .count(),
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
    #[allow(clippy::too_many_arguments, reason = "each argument is one column of the row")]
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
                let mut row = find(conn, &session_id, &account_id)?.unwrap_or_else(|| {
                    let mut fresh = blank(&session_id, &account_id, now);
                    fresh.candidate_id = format!("host:{session_id}");
                    fresh
                });
                clear_connection(&mut row);
                row.state = PresenceState::Admitted;
                row.entry_epoch += 1;
                row.generation = generation;
                (row.realm_id, row.peer_id) = (realm_id, peer_id);
                row.local_bridge_port = Some(local_bridge_port);
                (row.local_bridge_ready_at, row.admitted_at) = (Some(now), Some(now));
                put(conn, &row)?;
                Ok(row)
            })
            .await
    }

    /// The account's row in the session, if any.
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
        let (clause, state_args) = in_states(states.unwrap_or(&PresenceState::LIVE));
        let tail = format!("WHERE account_id = ? AND {clause} ORDER BY requested_at DESC, session_id LIMIT 1");
        let mut args = binds![account_id];
        args.extend(state_args);
        self.db
            .read(move |conn| Ok(rows_where(conn, &tail, args)?.into_iter().next()))
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
        let (clause, state_args) = in_states(states);
        let tail =
            format!("WHERE session_id = ? AND generation = ? AND {clause} ORDER BY requested_at, account_id LIMIT ?");
        let mut args = binds![session_id, generation];
        args.extend(state_args);
        args.extend(binds![i64::from(max)]);
        self.db.read(move |conn| rows_where(conn, &tail, args)).await
    }

    /// Rows of the generation in one of `states`.
    pub async fn count_by_state(
        &self,
        session_id: &str,
        generation: i64,
        states: &[PresenceState],
    ) -> Result<i64, RepoError> {
        let (clause, state_args) = in_states(states);
        let text = format!("SELECT COUNT(*) AS n FROM {TABLE} WHERE session_id = ? AND generation = ? AND {clause}");
        let mut args = binds![session_id, generation];
        args.extend(state_args);
        self.db
            .read(move |conn| Statement::new(&text, args)?.number(conn))
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
                let movable = find(conn, &session_id, &account_id)?
                    .filter(|row| row.generation == generation)
                    .and_then(|row| next_presence_state(row.state, event).map(|next| (row, next)))
                    .filter(|(row, _)| guard(row));
                let Some((mut row, next)) = movable else {
                    return Ok(None);
                };
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
        let unexpired = move |row: &HubPresenceRecord| !is_presence_expired(row.state, row.expires_at, now);
        let ticket = move |row: &mut HubPresenceRecord, next| {
            row.state = next;
            row.ticketed_at = Some(now);
            row.expires_at = ticket_expires_at.max(row.expires_at);
        };
        self.transition(
            actor,
            (session_id, generation, account_id),
            PresenceEvent::TicketIssued,
            unexpired,
            ticket,
        )
        .await
    }

    /// Records the loopback port only; the state stays.
    #[allow(
        clippy::too_many_arguments,
        reason = "the arguments are the row's key (session, generation, account) and its new values"
    )]
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
        let unexpired = move |row: &HubPresenceRecord| !is_presence_expired(row.state, row.expires_at, now);
        let ready = move |row: &mut HubPresenceRecord, _: PresenceState| {
            (row.local_bridge_port, row.local_bridge_ready_at) = (Some(local_bridge_port), Some(ready_at));
        };
        self.transition(
            actor,
            (session_id, generation, account_id),
            PresenceEvent::Admitted,
            unexpired,
            ready,
        )
        .await
    }

    /// Marks the account admitted under `admission_id`, unless its waiting entry expired; a report without a route keeps the known one.
    #[allow(
        clippy::too_many_arguments,
        reason = "the arguments are the row's key (session, generation, account) and its new values"
    )]
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
        let unexpired = move |row: &HubPresenceRecord| !is_presence_expired(row.state, row.expires_at, now);
        let admit = move |row: &mut HubPresenceRecord, next| {
            row.state = next;
            row.admission_id = Some(admission_id);
            row.admitted_at = Some(admitted_at);
            // A report without a route keeps the one already known.
            row.route = route.or(row.route.take());
            (row.lost_at, row.left_at, row.left_reason, row.purge_at) = (None, None, None, None);
        };
        self.transition(
            actor,
            (session_id, generation, account_id),
            PresenceEvent::Admitted,
            unexpired,
            admit,
        )
        .await
    }

    /// Only the current connection's report ends it.
    #[allow(
        clippy::too_many_arguments,
        reason = "the arguments are the row's key (session, generation, account) and its new values"
    )]
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
        let current = move |row: &HubPresenceRecord| {
            admission_id
                .as_ref()
                .is_none_or(|id| row.admission_id.as_ref() == Some(id))
        };
        let lose = move |row: &mut HubPresenceRecord, next| {
            row.state = next;
            (row.lost_at, row.purge_at) = (Some(lost_at), Some(purge_at));
        };
        self.transition(
            actor,
            (session_id, generation, account_id),
            PresenceEvent::Disconnected,
            current,
            lose,
        )
        .await
    }

    /// `reason` is `TRAVEL` when the member went into a realm hunt.
    #[allow(
        clippy::too_many_arguments,
        reason = "the arguments are the row's key (session, generation, account) and its new values"
    )]
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
        let leave = move |row: &mut HubPresenceRecord, next| {
            row.state = next;
            row.left_reason = Some(reason);
            (row.left_at, row.purge_at) = (Some(left_at), Some(purge_at));
        };
        self.transition(
            actor,
            (session_id, generation, account_id),
            PresenceEvent::Travelled,
            |_| true,
            leave,
        )
        .await
    }

    async fn remove_where(
        &self,
        actor: &Actor,
        (tail, args): (&'static str, Vec<Arg>),
        reason: &'static str,
        at: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Vec<HubPresenceRecord>, RepoError> {
        self.db
            .write(actor, move |conn| {
                let rows = rows_where(conn, tail, args)?;
                bulk(conn, rows, PresenceEvent::Removed, |row| {
                    row.left_reason = Some(reason.to_owned());
                    (row.left_at, row.purge_at) = (Some(at), Some(purge_at));
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
        let tail = "WHERE realm_id = ? AND account_id = ? ORDER BY session_id, account_id";
        let selected = (tail, binds![realm_id, account_id]);
        self.remove_where(actor, selected, "REMOVED", at, purge_at).await
    }

    /// Removes every row in the realm's hubs, as when the realm is released.
    pub async fn remove_realm(
        &self,
        actor: &Actor,
        realm_id: &str,
        at: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Vec<HubPresenceRecord>, RepoError> {
        let selected = ("WHERE realm_id = ? ORDER BY session_id, account_id", binds![realm_id]);
        self.remove_where(actor, selected, "REALM_RELEASED", at, purge_at).await
    }

    /// Waiting rows past their time; connected rows have no move and stay.
    pub async fn expire_stale(
        &self,
        actor: &Actor,
        session_id: Option<&str>,
        now: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Vec<HubPresenceRecord>, RepoError> {
        let (tail, args) = match session_id {
            Some(session) => (
                "WHERE session_id = ? AND expires_at <= ? ORDER BY session_id, account_id",
                binds![session, now],
            ),
            None => ("WHERE expires_at <= ? ORDER BY session_id, account_id", binds![now]),
        };
        self.db
            .write(actor, move |conn| {
                let rows = rows_where(conn, tail, args)?;
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
                let text = "DELETE FROM rifts_hub_presence WHERE purge_at IS NOT NULL AND purge_at <= ?";
                Statement::new(text, binds![now])?.run(conn)
            })
            .await
    }
}
