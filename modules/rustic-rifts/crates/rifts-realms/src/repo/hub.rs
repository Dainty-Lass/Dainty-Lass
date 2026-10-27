//! `RealmHubRepository` (`p2p/persistence/contracts/RealmHubRepository.ts`) on `store`: who is in a realm's hub, one row
//! per (hub session, account), written without touching the session row. Every method applies `hub`'s transition
//! tables in one write transaction. Node's `realmHubGates` serialised `requestEntries` on MongoDB; store's writes are
//! serial on both engines, so it has no table here.

use diesel::RunQueryDsl;
use diesel::sql_types::{BigInt, Nullable, Text};
use rifts_seam::{Actor, UnixMillis};
use store::AnyConnection;

use crate::db::{Count, RealmsDb, RepoError, corrupt, marks, sql};
use crate::hub::{
    EntryDecision, PRESENCE_LIVE_STATES, PresenceEvent, PresenceState, decide_entry_request, is_presence_expired,
    is_presence_live, next_presence_state,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubPresenceRecord {
    pub session_id: String,
    pub generation: i64,
    pub realm_id: String,
    pub account_id: String,
    /// The account's transport peer id, which the ticket and the link carry.
    pub peer_id: String,
    pub state: PresenceState,
    /// The matchmaking candidate of the current entry.
    pub candidate_id: String,
    /// 1 for the first entry, +1 for each re-entry.
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
    /// PENDING and TICKETED rows expire at this time.
    pub expires_at: UnixMillis,
    /// Set once the row stopped being live: a week later it may go.
    pub purge_at: Option<UnixMillis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryOutcome {
    Created,
    Reused,
    Reentered,
}

/// One member asking for a seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubEntryRequest {
    pub account_id: String,
    pub peer_id: String,
    pub candidate_id: String,
}

/// Where and when an entry is asked for.
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
    Placed(Vec<(String, HubPresenceRecord, EntryOutcome)>),
    /// Nothing written: the newcomers did not fit next to `live` rows.
    Full { live: i64, max_peers: i64 },
}

#[derive(diesel::QueryableByName)]
struct Row {
    #[diesel(sql_type = Text)]
    session_id: String,
    #[diesel(sql_type = BigInt)]
    generation: i64,
    #[diesel(sql_type = Text)]
    realm_id: String,
    #[diesel(sql_type = Text)]
    account_id: String,
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

const COLUMNS: &str = "session_id, generation, realm_id, account_id, peer_id, state, candidate_id, entry_epoch, \
    local_bridge_port, local_bridge_ready_at, admission_id, route, requested_at, ticketed_at, admitted_at, lost_at, \
    left_at, left_reason, expires_at, purge_at";

fn record(row: Row) -> Result<HubPresenceRecord, RepoError> {
    let ms = |v: Option<i64>| v.map(UnixMillis);
    Ok(HubPresenceRecord {
        state: PresenceState::parse(&row.state).ok_or_else(|| corrupt("hub presence state", &row.state))?,
        session_id: row.session_id,
        generation: row.generation,
        realm_id: row.realm_id,
        account_id: row.account_id,
        peer_id: row.peer_id,
        candidate_id: row.candidate_id,
        entry_epoch: row.entry_epoch,
        local_bridge_port: row.local_bridge_port,
        local_bridge_ready_at: ms(row.local_bridge_ready_at),
        admission_id: row.admission_id,
        route: row.route,
        requested_at: UnixMillis(row.requested_at),
        ticketed_at: ms(row.ticketed_at),
        admitted_at: ms(row.admitted_at),
        lost_at: ms(row.lost_at),
        left_at: ms(row.left_at),
        left_reason: row.left_reason,
        expires_at: UnixMillis(row.expires_at),
        purge_at: ms(row.purge_at),
    })
}

fn find(conn: &mut AnyConnection, session_id: &str, account_id: &str) -> Result<Option<HubPresenceRecord>, RepoError> {
    sql(&format!("SELECT {COLUMNS} FROM rifts_hub_presence WHERE session_id = ? AND account_id = ?"))?
        .bind::<Text, _>(session_id.to_owned())
        .bind::<Text, _>(account_id.to_owned())
        .load::<Row>(conn)?
        .into_iter()
        .next()
        .map(record)
        .transpose()
}

/// Inserts or replaces the whole row.
fn put(conn: &mut AnyConnection, r: &HubPresenceRecord) -> Result<(), RepoError> {
    let columns: Vec<&str> = COLUMNS.split(", ").collect();
    let update: Vec<&str> = columns[2..].to_vec();
    let ms = |v: Option<UnixMillis>| v.map(|t| t.0);
    diesel::sql_query(store::portable::upsert(
        "rifts_hub_presence",
        &columns,
        &["session_id", "account_id"],
        &update,
    ))
    .bind::<Text, _>(r.session_id.clone())
    .bind::<BigInt, _>(r.generation)
    .bind::<Text, _>(r.realm_id.clone())
    .bind::<Text, _>(r.account_id.clone())
    .bind::<Text, _>(r.peer_id.clone())
    .bind::<Text, _>(r.state.as_str())
    .bind::<Text, _>(r.candidate_id.clone())
    .bind::<BigInt, _>(r.entry_epoch)
    .bind::<Nullable<BigInt>, _>(r.local_bridge_port)
    .bind::<Nullable<BigInt>, _>(ms(r.local_bridge_ready_at))
    .bind::<Nullable<Text>, _>(r.admission_id.clone())
    .bind::<Nullable<Text>, _>(r.route.clone())
    .bind::<BigInt, _>(r.requested_at.0)
    .bind::<Nullable<BigInt>, _>(ms(r.ticketed_at))
    .bind::<Nullable<BigInt>, _>(ms(r.admitted_at))
    .bind::<Nullable<BigInt>, _>(ms(r.lost_at))
    .bind::<Nullable<BigInt>, _>(ms(r.left_at))
    .bind::<Nullable<Text>, _>(r.left_reason.clone())
    .bind::<BigInt, _>(r.expires_at.0)
    .bind::<Nullable<BigInt>, _>(ms(r.purge_at))
    .execute(conn)?;
    Ok(())
}

/// `entryRow`: create, reuse a waiting row (only its expiry ever grows), or start over with the connection cleared.
fn entry_row(
    conn: &mut AnyConnection,
    at: &EntryAt,
    request: &HubEntryRequest,
) -> Result<(HubPresenceRecord, EntryOutcome), RepoError> {
    let existing = find(conn, &at.session_id, &request.account_id)?;
    let decision = decide_entry_request(existing.as_ref().map(|r| (r.state, r.expires_at)), at.now);
    let (next, outcome) = match (decision, existing) {
        (EntryDecision::Reuse, Some(mut row)) => {
            if at.expires_at > row.expires_at {
                row.expires_at = at.expires_at;
                put(conn, &row)?;
            }
            return Ok((row, EntryOutcome::Reused));
        }
        (EntryDecision::Reenter, Some(row)) => (
            HubPresenceRecord {
                generation: at.generation,
                realm_id: at.realm_id.clone(),
                peer_id: request.peer_id.clone(),
                state: PresenceState::Pending,
                candidate_id: request.candidate_id.clone(),
                requested_at: at.now,
                expires_at: at.expires_at,
                entry_epoch: row.entry_epoch + 1,
                local_bridge_port: None,
                local_bridge_ready_at: None,
                admission_id: None,
                route: None,
                ticketed_at: None,
                admitted_at: None,
                lost_at: None,
                left_at: None,
                left_reason: None,
                purge_at: None,
                ..row
            },
            EntryOutcome::Reentered,
        ),
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
    put(conn, &next)?;
    Ok((next, outcome))
}

fn states_in(states: &[PresenceState]) -> String {
    format!("state IN ({})", marks(states.len()))
}

/// Applies `event` to every selected row it applies to; the rows as they were.
fn bulk(
    conn: &mut AnyConnection,
    rows: Vec<HubPresenceRecord>,
    event: PresenceEvent,
    apply: impl Fn(&mut HubPresenceRecord),
) -> Result<Vec<HubPresenceRecord>, RepoError> {
    let mut changed = Vec::new();
    for row in rows {
        let Some(to) = next_presence_state(row.state, event) else {
            continue;
        };
        let mut next = row.clone();
        next.state = to;
        apply(&mut next);
        put(conn, &next)?;
        changed.push(row);
    }
    Ok(changed)
}

fn select(conn: &mut AnyConnection, filter: &str, binds: &[&str]) -> Result<Vec<HubPresenceRecord>, RepoError> {
    let mut query = sql(&format!("SELECT {COLUMNS} FROM rifts_hub_presence WHERE {filter} ORDER BY session_id, account_id"))?;
    for bind in binds {
        query = query.bind::<Text, _>((*bind).to_owned());
    }
    query.load::<Row>(conn)?.into_iter().map(record).collect()
}

/// The hub ended: every live row of the generation -> LOST; the rows as they were.
pub(crate) fn end_generation(
    conn: &mut AnyConnection,
    session_id: &str,
    generation: i64,
    at: UnixMillis,
    purge_at: UnixMillis,
) -> Result<Vec<HubPresenceRecord>, RepoError> {
    let rows = sql(&format!(
        "SELECT {COLUMNS} FROM rifts_hub_presence WHERE session_id = ? AND generation = ? ORDER BY account_id"
    ))?
    .bind::<Text, _>(session_id.to_owned())
    .bind::<BigInt, _>(generation)
    .load::<Row>(conn)?
    .into_iter()
    .map(record)
    .collect::<Result<Vec<_>, _>>()?;
    bulk(conn, rows, PresenceEvent::SessionEnded, |row| {
        row.lost_at = Some(at);
        row.purge_at = Some(purge_at);
    })
}

#[derive(Clone)]
pub struct RealmHubRepository {
    db: RealmsDb,
}

impl RealmHubRepository {
    pub fn new(db: RealmsDb) -> Self {
        Self { db }
    }

    /// An entry with no capacity check; production entries go through [`Self::request_entries`].
    pub async fn request_entry(
        &self,
        actor: &Actor,
        at: &EntryAt,
        request: &HubEntryRequest,
    ) -> Result<(HubPresenceRecord, EntryOutcome), RepoError> {
        let (at, request) = (at.clone(), request.clone());
        self.db.write(actor, move |conn| entry_row(conn, &at, &request)).await
    }

    /// Several members of one hub in one step: the live rows plus the newcomers must fit `max_peers`, or nobody is
    /// written. A member whose own row is live takes no new seat and is never refused.
    pub async fn request_entries(
        &self,
        actor: &Actor,
        at: &EntryAt,
        max_peers: i64,
        entries: &[HubEntryRequest],
    ) -> Result<HubEntryBatch, RepoError> {
        let at = at.clone();
        let mut requests: Vec<HubEntryRequest> = Vec::new();
        // Node's Map keeps the first position and the last value of an account listed twice
        for entry in entries {
            match requests.iter_mut().find(|r| r.account_id == entry.account_id) {
                Some(existing) => *existing = entry.clone(),
                None => requests.push(entry.clone()),
            }
        }
        self.db
            .write(actor, move |conn| {
                let generation_rows = sql(&format!(
                    "SELECT {COLUMNS} FROM rifts_hub_presence WHERE session_id = ? AND generation = ?"
                ))?
                .bind::<Text, _>(at.session_id.clone())
                .bind::<BigInt, _>(at.generation)
                .load::<Row>(conn)?
                .into_iter()
                .map(record)
                .collect::<Result<Vec<_>, _>>()?;
                let live = generation_rows
                    .iter()
                    .filter(|row| is_presence_live(row.state, row.expires_at, at.now))
                    .count();
                let mut newcomers = 0;
                for request in &requests {
                    let existing = find(conn, &at.session_id, &request.account_id)?;
                    if !existing.is_some_and(|row| is_presence_live(row.state, row.expires_at, at.now)) {
                        newcomers += 1;
                    }
                }
                let live = i64::try_from(live).unwrap_or(i64::MAX);
                if newcomers > 0 && live + newcomers > max_peers {
                    return Ok(HubEntryBatch::Full { live, max_peers });
                }
                let mut placed = Vec::with_capacity(requests.len());
                for request in &requests {
                    let (row, outcome) = entry_row(conn, &at, request)?;
                    placed.push((request.account_id.clone(), row, outcome));
                }
                Ok(HubEntryBatch::Placed(placed))
            })
            .await
    }

    /// The host's own row at hub start: created or replaced ADMITTED with its loopback port.
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
        let (session, realm, account, peer) =
            (session_id.to_owned(), realm_id.to_owned(), account_id.to_owned(), peer_id.to_owned());
        self.db
            .write(actor, move |conn| {
                let existing = find(conn, &session, &account)?;
                let epoch = existing.as_ref().map_or(0, |row| row.entry_epoch) + 1;
                let base = existing.unwrap_or_else(|| HubPresenceRecord {
                    session_id: session.clone(),
                    generation,
                    realm_id: realm.clone(),
                    account_id: account.clone(),
                    peer_id: peer.clone(),
                    state: PresenceState::Admitted,
                    candidate_id: format!("host:{session}"),
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
                let next = HubPresenceRecord {
                    generation,
                    realm_id: realm,
                    peer_id: peer,
                    state: PresenceState::Admitted,
                    local_bridge_port: Some(local_bridge_port),
                    local_bridge_ready_at: Some(now),
                    admitted_at: Some(now),
                    entry_epoch: epoch,
                    ticketed_at: None,
                    lost_at: None,
                    left_at: None,
                    left_reason: None,
                    purge_at: None,
                    admission_id: None,
                    route: None,
                    ..base
                };
                put(conn, &next)?;
                Ok(next)
            })
            .await
    }

    pub async fn find_presence(&self, session_id: &str, account_id: &str) -> Result<Option<HubPresenceRecord>, RepoError> {
        let (session, account) = (session_id.to_owned(), account_id.to_owned());
        self.db.read(move |conn| find(conn, &session, &account)).await
    }

    /// The account's newest row (by request time) in one of `states`, any hub; live states when `None`.
    pub async fn find_for_account(
        &self,
        account_id: &str,
        states: Option<&[PresenceState]>,
    ) -> Result<Option<HubPresenceRecord>, RepoError> {
        let states: Vec<PresenceState> = states.unwrap_or(PRESENCE_LIVE_STATES).to_vec();
        let account = account_id.to_owned();
        self.db
            .read(move |conn| {
                let mut query = sql(&format!(
                    "SELECT {COLUMNS} FROM rifts_hub_presence WHERE account_id = ? AND {} \
                     ORDER BY requested_at DESC, session_id LIMIT 1",
                    states_in(&states)
                ))?
                .bind::<Text, _>(account);
                for state in &states {
                    query = query.bind::<Text, _>(state.as_str());
                }
                query.load::<Row>(conn)?.into_iter().next().map(record).transpose()
            })
            .await
    }

    /// Rows of the hub generation in `states`, oldest request first.
    pub async fn list_by_state(
        &self,
        session_id: &str,
        generation: i64,
        states: &[PresenceState],
        limit: u32,
    ) -> Result<Vec<HubPresenceRecord>, RepoError> {
        let (session, states) = (session_id.to_owned(), states.to_vec());
        self.db
            .read(move |conn| {
                let mut query = sql(&format!(
                    "SELECT {COLUMNS} FROM rifts_hub_presence WHERE session_id = ? AND generation = ? AND {} \
                     ORDER BY requested_at, account_id LIMIT ?",
                    states_in(&states)
                ))?
                .bind::<Text, _>(session)
                .bind::<BigInt, _>(generation);
                for state in &states {
                    query = query.bind::<Text, _>(state.as_str());
                }
                query
                    .bind::<BigInt, _>(i64::from(limit))
                    .load::<Row>(conn)?
                    .into_iter()
                    .map(record)
                    .collect()
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
                let mut query = sql(&format!(
                    "SELECT COUNT(*) AS n FROM rifts_hub_presence WHERE session_id = ? AND generation = ? AND {}",
                    states_in(&states)
                ))?
                .bind::<Text, _>(session)
                .bind::<BigInt, _>(generation);
                for state in &states {
                    query = query.bind::<Text, _>(state.as_str());
                }
                Ok(query.load::<Count>(conn)?.into_iter().next().map_or(0, |c| c.n))
            })
            .await
    }

    /// Changes one row of the generation when `event` applies and `guard` agrees.
    #[allow(clippy::too_many_arguments)]
    async fn transition(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        account_id: &str,
        event: PresenceEvent,
        guard: impl FnOnce(&HubPresenceRecord) -> bool + Send + 'static,
        apply: impl FnOnce(&mut HubPresenceRecord) + Send + 'static,
    ) -> Result<Option<HubPresenceRecord>, RepoError> {
        let (session, account) = (session_id.to_owned(), account_id.to_owned());
        self.db
            .write(actor, move |conn| {
                let Some(mut row) = find(conn, &session, &account)? else {
                    return Ok(None);
                };
                if row.generation != generation || next_presence_state(row.state, event).is_none() || !guard(&row) {
                    return Ok(None);
                }
                apply(&mut row);
                put(conn, &row)?;
                Ok(Some(row))
            })
            .await
    }

    /// PENDING or TICKETED and not expired -> TICKETED; the row lives at least as long as the ticket.
    pub async fn mark_ticketed(
        &self,
        actor: &Actor,
        session_id: &str,
        generation: i64,
        account_id: &str,
        now: UnixMillis,
        ticket_expires_at: UnixMillis,
    ) -> Result<Option<HubPresenceRecord>, RepoError> {
        let guard = move |row: &HubPresenceRecord| !is_presence_expired(row.state, row.expires_at, now);
        let apply = move |row: &mut HubPresenceRecord| {
            row.state = PresenceState::Ticketed;
            row.ticketed_at = Some(now);
            row.expires_at = row.expires_at.max(ticket_expires_at);
        };
        self.transition(actor, session_id, generation, account_id, PresenceEvent::TicketIssued, guard, apply)
            .await
    }

    /// The member's launcher published its loopback port (a waiting row not expired, or a connected one).
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
        let guard = move |row: &HubPresenceRecord| !is_presence_expired(row.state, row.expires_at, now);
        let apply = move |row: &mut HubPresenceRecord| {
            row.local_bridge_port = Some(local_bridge_port);
            row.local_bridge_ready_at = Some(ready_at);
        };
        self.transition(actor, session_id, generation, account_id, PresenceEvent::Admitted, guard, apply)
            .await
    }

    /// The host consumed the member's ticket: PENDING, TICKETED (not expired) or ADMITTED (idempotent) -> ADMITTED.
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
        let (admission, route) = (admission_id.to_owned(), route.map(str::to_owned));
        let guard = move |row: &HubPresenceRecord| !is_presence_expired(row.state, row.expires_at, now);
        let apply = move |row: &mut HubPresenceRecord| {
            row.state = PresenceState::Admitted;
            row.admission_id = Some(admission);
            row.admitted_at = Some(admitted_at);
            if route.is_some() {
                row.route = route;
            }
            row.lost_at = None;
            row.left_at = None;
            row.left_reason = None;
            row.purge_at = None;
        };
        self.transition(actor, session_id, generation, account_id, PresenceEvent::Admitted, guard, apply)
            .await
    }

    /// ADMITTED -> LOST; with `admission_id`, only the current connection's report ends it.
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
        let admission = admission_id.map(str::to_owned);
        let guard = move |row: &HubPresenceRecord| admission.is_none() || row.admission_id == admission;
        let apply = move |row: &mut HubPresenceRecord| {
            row.state = PresenceState::Lost;
            row.lost_at = Some(lost_at);
            row.purge_at = Some(purge_at);
        };
        self.transition(actor, session_id, generation, account_id, PresenceEvent::Disconnected, guard, apply)
            .await
    }

    /// ADMITTED -> LEFT (`TRAVEL` when the member went into a realm hunt).
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
        let apply = move |row: &mut HubPresenceRecord| {
            row.state = PresenceState::Left;
            row.left_at = Some(left_at);
            row.left_reason = Some(reason);
            row.purge_at = Some(purge_at);
        };
        self.transition(actor, session_id, generation, account_id, PresenceEvent::Travelled, |_| true, apply)
            .await
    }

    /// Kick or ban: every live row of the account in the realm -> REMOVED. The rows as they were.
    pub async fn remove_account_from_realm(
        &self,
        actor: &Actor,
        realm_id: &str,
        account_id: &str,
        at: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Vec<HubPresenceRecord>, RepoError> {
        let (realm, account) = (realm_id.to_owned(), account_id.to_owned());
        self.db
            .write(actor, move |conn| {
                let rows = select(conn, "realm_id = ? AND account_id = ?", &[&realm, &account])?;
                bulk(conn, rows, PresenceEvent::Removed, |row| {
                    row.left_at = Some(at);
                    row.left_reason = Some("REMOVED".to_owned());
                    row.purge_at = Some(purge_at);
                })
            })
            .await
    }

    /// The realm was released: every live row of it -> REMOVED.
    pub async fn remove_realm(
        &self,
        actor: &Actor,
        realm_id: &str,
        at: UnixMillis,
        purge_at: UnixMillis,
    ) -> Result<Vec<HubPresenceRecord>, RepoError> {
        let realm = realm_id.to_owned();
        self.db
            .write(actor, move |conn| {
                let rows = select(conn, "realm_id = ?", &[&realm])?;
                bulk(conn, rows, PresenceEvent::Removed, |row| {
                    row.left_at = Some(at);
                    row.left_reason = Some("REALM_RELEASED".to_owned());
                    row.purge_at = Some(purge_at);
                })
            })
            .await
    }

    /// Waiting rows past their expiry -> EXPIRED, in one hub or all.
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
                let mut query = sql(&format!(
                    "SELECT {COLUMNS} FROM rifts_hub_presence WHERE expires_at <= ?{} ORDER BY session_id, account_id",
                    if session.is_some() { " AND session_id = ?" } else { "" }
                ))?
                .bind::<BigInt, _>(now.0);
                if let Some(session) = session {
                    query = query.bind::<Text, _>(session);
                }
                let rows = query.load::<Row>(conn)?.into_iter().map(record).collect::<Result<Vec<_>, _>>()?;
                bulk(conn, rows, PresenceEvent::Expired, |row| row.purge_at = Some(purge_at))
            })
            .await
    }

    /// The hub ended: every live row of the generation -> LOST.
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
            .write(actor, move |conn| end_generation(conn, &session, generation, at, purge_at))
            .await
    }

    /// Node's TTL index: rows a week after they stopped being live.
    pub async fn purge(&self, now: UnixMillis) -> Result<usize, RepoError> {
        self.db
            .write(&Actor::System("realms-purge"), move |conn| {
                Ok(sql("DELETE FROM rifts_hub_presence WHERE purge_at IS NOT NULL AND purge_at <= ?")?
                    .bind::<BigInt, _>(now.0)
                    .execute(conn)?)
            })
            .await
    }
}
