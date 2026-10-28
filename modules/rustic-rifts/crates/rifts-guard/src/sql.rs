//! SQLite and Postgres implementation of [`GuardStore`] on DaintyLass's `store` (DF7, DD15).

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::OnceLock;

use diesel::query_builder::{BoxedSqlQuery, SqlQuery};
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use diesel::{OptionalExtension, QueryableByName, RunQueryDsl};
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use rifts_seam::{AccountId, Actor, HostWriteKind, Json, RealmId, SessionId, StoreError, UnixMillis};
use store::portable::{insert_ignore, numbered, upsert};
use store::{AnyConnection, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema, MultiBackend, Tx};

use crate::store::{
    AlertRecord, BatchRecord, ChainRecord, ChainUpdate, CompletionMode, CompletionRecord, Disposition, EncounterState,
    FindingRecord, GuardStore, HostWriteRecord, TrafficReservation, TrafficScope, TrafficWindowRecord, XpSummaryRecord,
};
use crate::writes::kind_name;

pub const SQLITE_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/sqlite");
pub const POSTGRES_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/postgres");

/// Guard's schema on its own; `rifts-realms` also includes `2026-10-26-000002_rifts_guard` so a shared `RealmsDb`
/// carries all tables from both crates.
pub const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE_MIGRATIONS,
    postgres: || POSTGRES_MIGRATIONS,
    soft_delete: &[],
};

/// `p2pHuntXpSummaries` TTL (~400 days in milliseconds).
pub const XP_SUMMARY_TTL_MS: i64 = 400 * 86_400_000;

type Query = BoxedSqlQuery<'static, MultiBackend, SqlQuery>;

#[derive(Debug, thiserror::Error)]
enum SqlError {
    #[error(transparent)]
    Store(#[from] store::Error),
    #[error("database: {0}")]
    Query(#[from] diesel::result::Error),
    #[error("guard table: {0}")]
    Corrupt(String),
}

impl From<SqlError> for StoreError {
    fn from(error: SqlError) -> Self {
        Self::Database(error.to_string())
    }
}

fn sql(text: &str) -> Result<Query, SqlError> {
    let numbered_sql = numbered(text).map_err(SqlError::Corrupt)?;
    Ok(diesel::sql_query(numbered_sql).into_boxed())
}

fn corrupt(err: impl std::fmt::Display) -> SqlError {
    SqlError::Corrupt(err.to_string())
}

fn to_i64(value: u64) -> Result<i64, SqlError> {
    i64::try_from(value).map_err(corrupt)
}

fn to_u64(value: i64) -> Result<u64, SqlError> {
    u64::try_from(value).map_err(corrupt)
}

fn to_u32(value: i64) -> Result<u32, SqlError> {
    u32::try_from(value).map_err(corrupt)
}

fn is_unique_violation(err: &diesel::result::Error) -> bool {
    match err {
        diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _) => true,
        diesel::result::Error::DatabaseError(_, info) => {
            let msg = info.message();
            msg.contains("UNIQUE constraint failed") || msg.contains("duplicate key value")
        }
        _ => false,
    }
}

fn fallback_runtime() -> Result<&'static tokio::runtime::Runtime, StoreError> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    let result = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|err| err.to_string())
    });
    match result {
        Ok(rt) => Ok(rt),
        Err(msg) => Err(StoreError::Database(msg.clone())),
    }
}

/// Runs a `store::Database` future from a synchronous [`GuardStore`] or [`crate::ports::HuntSessions`] method,
/// whether the caller is inside a multi-thread Tokio runtime, a current-thread Tokio runtime, or a plain thread.
pub fn block_on_store<F, T>(fut: F) -> Result<T, StoreError>
where
    F: Future<Output = Result<T, StoreError>> + Send,
    T: Send,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
            return tokio::task::block_in_place(|| handle.block_on(fut));
        }
        let rt = fallback_runtime()?;
        return std::thread::scope(|scope| {
            scope
                .spawn(|| rt.block_on(fut))
                .join()
                .map_err(|_| StoreError::Database("store worker thread panicked".to_owned()))?
        });
    }
    let rt = fallback_runtime()?;
    rt.block_on(fut)
}

/// Maps `rifts-seam`'s [`Actor`] to `store`'s [`store::Actor`].
pub fn store_actor(actor: &Actor) -> store::Actor {
    match actor {
        Actor::Account(account) => store::Actor::Player(account.0.clone()),
        Actor::HostRuntime { host, .. } => store::Actor::Player(host.0.clone()),
        Actor::Admin { key_id } => store::Actor::Admin(key_id.clone()),
        Actor::System(task) => store::Actor::System((*task).to_owned()),
    }
}

fn actor_name(actor: &Actor) -> String {
    match actor {
        Actor::Admin { key_id } => key_id.clone(),
        Actor::Account(account) => account.0.clone(),
        Actor::HostRuntime { host, .. } => host.0.clone(),
        Actor::System(name) => (*name).to_owned(),
    }
}

fn completion_mode_str(mode: CompletionMode) -> &'static str {
    match mode {
        CompletionMode::FirstDefeat => "FIRST_DEFEAT",
        CompletionMode::EndOfHunt => "END_OF_HUNT",
        CompletionMode::NoCompletion => "NO_COMPLETION",
    }
}

fn parse_completion_mode(raw: &str) -> Result<CompletionMode, SqlError> {
    CompletionMode::parse(raw).ok_or_else(|| corrupt(format!("unknown completion_mode: {raw}")))
}

fn parse_encounter_state(raw: &str) -> Result<EncounterState, SqlError> {
    match raw {
        "CREATED" => Ok(EncounterState::Created),
        "SESSION_STARTED" => Ok(EncounterState::SessionStarted),
        "WORLD_READY" => Ok(EncounterState::WorldReady),
        "ENCOUNTER_ACTIVE" => Ok(EncounterState::EncounterActive),
        "BEHEMOTH_DEFEATED" => Ok(EncounterState::BehemothDefeated),
        "COMPLETION_REQUESTED" => Ok(EncounterState::CompletionRequested),
        "ABORTED" => Ok(EncounterState::Aborted),
        other => Err(corrupt(format!("unknown encounter_state: {other}"))),
    }
}

fn parse_disposition(raw: &str) -> Result<Disposition, SqlError> {
    match raw {
        "OBSERVE_CLEAR" => Ok(Disposition::ObserveClear),
        "OBSERVE_FLAGGED" => Ok(Disposition::ObserveFlagged),
        other => Err(corrupt(format!("unknown disposition: {other}"))),
    }
}

fn parse_host_write_kind(raw: &str) -> Result<HostWriteKind, SqlError> {
    match raw {
        "XP" => Ok(HostWriteKind::Xp),
        "MASTERY" => Ok(HostWriteKind::Mastery),
        "OBJECTIVES" => Ok(HostWriteKind::Objectives),
        "ESCALATION" => Ok(HostWriteKind::Escalation),
        "COOLDOWN" => Ok(HostWriteKind::Cooldown),
        "BOUNTY" => Ok(HostWriteKind::Bounty),
        "SLAYERS_PATH" => Ok(HostWriteKind::SlayersPath),
        "REFORGE" => Ok(HostWriteKind::Reforge),
        "RANK_CONFIRM" => Ok(HostWriteKind::RankConfirm),
        "LOADOUT_UNLOCK" => Ok(HostWriteKind::LoadoutUnlock),
        "HUNT_PASS_XP" => Ok(HostWriteKind::HuntPassXp),
        "ITEM_SAVE" => Ok(HostWriteKind::ItemSave),
        other => Err(corrupt(format!("unknown host write kind: {other}"))),
    }
}

fn parse_traffic_scope(raw: &str) -> Result<TrafficScope, SqlError> {
    match raw {
        "host" => Ok(TrafficScope::Host),
        "session" => Ok(TrafficScope::Session),
        other => Err(corrupt(format!("unknown traffic scope: {other}"))),
    }
}

#[derive(QueryableByName)]
struct RevisionRow {
    #[diesel(sql_type = BigInt)]
    revision: i64,
}

#[derive(QueryableByName)]
struct ChainRow {
    #[diesel(sql_type = Text)]
    session: String,
    #[diesel(sql_type = BigInt)]
    generation: i64,
    #[diesel(sql_type = BigInt)]
    revision: i64,
    #[diesel(sql_type = Nullable<Text>)]
    completion_mode: Option<String>,
    #[diesel(sql_type = BigInt)]
    last_sequence: i64,
    #[diesel(sql_type = Text)]
    head_hash: String,
    #[diesel(sql_type = Text)]
    encounter_state: String,
    #[diesel(sql_type = Text)]
    disposition: String,
    #[diesel(sql_type = BigInt)]
    finding_count: i64,
    #[diesel(sql_type = BigInt)]
    batch_count: i64,
    #[diesel(sql_type = BigInt)]
    event_count: i64,
    #[diesel(sql_type = BigInt)]
    event_bytes: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    last_monotonic_ms: Option<i64>,
    #[diesel(sql_type = Text)]
    event_types: String,
    #[diesel(sql_type = Integer)]
    has_unattributed_damage: i32,
    #[diesel(sql_type = Nullable<BigInt>)]
    last_received_at: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    final_sequence: Option<i64>,
    #[diesel(sql_type = Nullable<Text>)]
    final_hash: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    completed_at: Option<i64>,
}

impl ChainRow {
    fn into_record(self) -> Result<ChainRecord, SqlError> {
        let session = self.session.parse::<SessionId>().map_err(corrupt)?;
        let completion_mode = self.completion_mode.as_deref().map(parse_completion_mode).transpose()?;
        let encounter_state = parse_encounter_state(&self.encounter_state)?;
        let disposition = parse_disposition(&self.disposition)?;
        let event_types: BTreeMap<String, u64> = serde_json::from_str(&self.event_types).map_err(corrupt)?;
        let last_monotonic_ms = self.last_monotonic_ms.map(to_u64).transpose()?;
        let completion = match (self.final_sequence, self.final_hash, self.completed_at) {
            (Some(seq), Some(hash), Some(at)) => Some(CompletionRecord {
                final_sequence: to_u64(seq)?,
                final_hash: hash,
                at: UnixMillis(at),
            }),
            _ => None,
        };
        Ok(ChainRecord {
            session,
            generation: to_u64(self.generation)?,
            revision: to_u64(self.revision)?,
            completion_mode,
            last_sequence: to_u64(self.last_sequence)?,
            head_hash: self.head_hash,
            encounter_state,
            disposition,
            finding_count: to_u64(self.finding_count)?,
            batch_count: to_u64(self.batch_count)?,
            event_count: to_u64(self.event_count)?,
            event_bytes: to_u64(self.event_bytes)?,
            last_monotonic_ms,
            event_types,
            has_unattributed_damage: self.has_unattributed_damage != 0,
            last_received_at: self.last_received_at.map(UnixMillis),
            completion,
        })
    }
}

#[derive(QueryableByName)]
struct BatchIdRow {
    #[diesel(sql_type = Text)]
    #[allow(dead_code)]
    batch_id: String,
}

#[derive(QueryableByName)]
struct BatchRow {
    #[diesel(sql_type = Text)]
    session: String,
    #[diesel(sql_type = BigInt)]
    generation: i64,
    #[diesel(sql_type = Text)]
    batch_id: String,
    #[diesel(sql_type = Text)]
    previous_hash: String,
    #[diesel(sql_type = Text)]
    batch_hash: String,
    #[diesel(sql_type = BigInt)]
    first_sequence: i64,
    #[diesel(sql_type = BigInt)]
    last_sequence: i64,
    #[diesel(sql_type = BigInt)]
    event_count: i64,
    #[diesel(sql_type = BigInt)]
    event_bytes: i64,
    #[diesel(sql_type = Text)]
    event_types: String,
    #[diesel(sql_type = BigInt)]
    received_at: i64,
}

impl BatchRow {
    fn into_record(self) -> Result<BatchRecord, SqlError> {
        let session = self.session.parse::<SessionId>().map_err(corrupt)?;
        let event_types: Vec<String> = serde_json::from_str(&self.event_types).map_err(corrupt)?;
        Ok(BatchRecord {
            session,
            generation: to_u64(self.generation)?,
            batch_id: self.batch_id,
            previous_hash: self.previous_hash,
            batch_hash: self.batch_hash,
            first_sequence: to_u64(self.first_sequence)?,
            last_sequence: to_u64(self.last_sequence)?,
            event_count: to_u64(self.event_count)?,
            event_bytes: to_u64(self.event_bytes)?,
            event_types,
            received_at: UnixMillis(self.received_at),
        })
    }
}

#[derive(QueryableByName)]
struct FindingRow {
    #[diesel(sql_type = Text)]
    session: String,
    #[diesel(sql_type = BigInt)]
    generation: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    sequence: Option<i64>,
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    detail: String,
    #[diesel(sql_type = BigInt)]
    at: i64,
}

impl FindingRow {
    fn into_record(self) -> Result<FindingRecord, SqlError> {
        Ok(FindingRecord {
            session: self.session.parse::<SessionId>().map_err(corrupt)?,
            generation: to_u64(self.generation)?,
            sequence: self.sequence.map(to_u64).transpose()?,
            code: self.code,
            detail: self.detail,
            at: UnixMillis(self.at),
        })
    }
}

#[derive(QueryableByName)]
struct TrafficWindowRow {
    #[diesel(sql_type = Text)]
    window_id: String,
    #[diesel(sql_type = Text)]
    scope: String,
    #[diesel(sql_type = BigInt)]
    window_start: i64,
    #[diesel(sql_type = BigInt)]
    requests: i64,
    #[diesel(sql_type = BigInt)]
    bytes: i64,
    #[diesel(sql_type = BigInt)]
    expires_at: i64,
}

impl TrafficWindowRow {
    fn into_record(self) -> Result<TrafficWindowRecord, SqlError> {
        Ok(TrafficWindowRecord {
            window_id: self.window_id,
            scope: parse_traffic_scope(&self.scope)?,
            window_start: UnixMillis(self.window_start),
            requests: to_u64(self.requests)?,
            bytes: to_u64(self.bytes)?,
            expires_at: UnixMillis(self.expires_at),
        })
    }
}

#[derive(QueryableByName)]
struct HostWriteRow {
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = Text)]
    target: String,
    #[diesel(sql_type = Text)]
    host: String,
    #[diesel(sql_type = Text)]
    session: String,
    #[diesel(sql_type = BigInt)]
    generation: i64,
    #[diesel(sql_type = Nullable<Text>)]
    realm: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    session_kind: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    hub_kind: Option<String>,
    #[diesel(sql_type = Text)]
    summary: String,
    #[diesel(sql_type = BigInt)]
    at: i64,
}

impl HostWriteRow {
    fn into_record(self) -> Result<HostWriteRecord, SqlError> {
        let kind = parse_host_write_kind(&self.kind)?;
        let session = self.session.parse::<SessionId>().map_err(corrupt)?;
        let realm = self
            .realm
            .as_deref()
            .map(|r| r.parse::<RealmId>().map_err(corrupt))
            .transpose()?;
        Ok(HostWriteRecord {
            kind,
            target: AccountId(self.target),
            host: AccountId(self.host),
            session,
            generation: to_u64(self.generation)?,
            realm,
            session_kind: self.session_kind,
            hub_kind: self.hub_kind,
            summary: Json(self.summary),
            at: UnixMillis(self.at),
        })
    }
}

#[derive(QueryableByName)]
struct XpSummaryRow {
    #[diesel(sql_type = Text)]
    session: String,
    #[diesel(sql_type = Text)]
    member: String,
    #[diesel(sql_type = Text)]
    host: String,
    #[diesel(sql_type = Nullable<Text>)]
    realm: Option<String>,
    #[diesel(sql_type = BigInt)]
    minutes: i64,
    #[diesel(sql_type = Text)]
    grants: String,
    #[diesel(sql_type = Integer)]
    flagged: i32,
    #[diesel(sql_type = BigInt)]
    created_at: i64,
}

impl XpSummaryRow {
    fn into_record(self) -> Result<XpSummaryRecord, SqlError> {
        let session = self.session.parse::<SessionId>().map_err(corrupt)?;
        let realm = self
            .realm
            .as_deref()
            .map(|r| r.parse::<RealmId>().map_err(corrupt))
            .transpose()?;
        Ok(XpSummaryRecord {
            session,
            member: AccountId(self.member),
            host: AccountId(self.host),
            realm,
            minutes: to_u32(self.minutes)?,
            grants: Json(self.grants),
            flagged: self.flagged != 0,
            created_at: UnixMillis(self.created_at),
        })
    }
}

#[derive(QueryableByName)]
struct AlertRow {
    #[diesel(sql_type = Text)]
    alert_id: String,
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    severity: String,
    #[diesel(sql_type = Text)]
    message: String,
    #[diesel(sql_type = Text)]
    details: String,
    #[diesel(sql_type = BigInt)]
    created_at: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    acknowledged_at: Option<i64>,
    #[diesel(sql_type = Nullable<Text>)]
    acknowledged_by: Option<String>,
}

impl AlertRow {
    fn into_record(self) -> AlertRecord {
        AlertRecord {
            alert_id: self.alert_id,
            code: self.code,
            severity: self.severity,
            message: self.message,
            details: Json(self.details),
            created_at: UnixMillis(self.created_at),
            acknowledged_at: self.acknowledged_at.map(UnixMillis),
            acknowledged_by: self.acknowledged_by,
        }
    }
}

fn find_traffic_window(conn: &mut AnyConnection, window_id: &str) -> Result<Option<TrafficWindowRecord>, SqlError> {
    let row = sql("SELECT window_id, scope, window_start, requests, bytes, expires_at \
         FROM rifts_traffic_windows WHERE window_id = ?")?
    .bind::<Text, _>(window_id)
    .get_result::<TrafficWindowRow>(conn)
    .optional()?;
    row.map(TrafficWindowRow::into_record).transpose()
}

fn find_alert(conn: &mut AnyConnection, alert_id: &str) -> Result<Option<AlertRecord>, SqlError> {
    let row = sql(
        "SELECT alert_id, code, severity, message, details, created_at, acknowledged_at, acknowledged_by \
         FROM rifts_operations_alerts WHERE alert_id = ?",
    )?
    .bind::<Text, _>(alert_id)
    .get_result::<AlertRow>(conn)
    .optional()?;
    Ok(row.map(AlertRow::into_record))
}

fn reserve_one(current: Option<TrafficWindowRecord>, reservation: &TrafficReservation) -> Option<TrafficWindowRecord> {
    let mut window = current.unwrap_or_else(|| TrafficWindowRecord {
        window_id: reservation.window_id.clone(),
        scope: reservation.scope,
        window_start: reservation.window_start,
        requests: 0,
        bytes: 0,
        expires_at: reservation.expires_at,
    });
    let accepted = window.requests < reservation.max_requests
        && window.bytes.saturating_add(reservation.bytes) <= reservation.max_bytes;
    if !accepted {
        return None;
    }
    window.requests += 1;
    window.bytes += reservation.bytes;
    Some(window)
}

/// [`GuardStore`] backed by DaintyLass's [`Database`] on SQLite and Postgres.
#[derive(Clone)]
pub struct SqlGuardStore {
    db: Database,
}

impl SqlGuardStore {
    /// Opens (or creates) a database with Guard's migrations applied and change-log triggers installed.
    pub fn open(config: &DatabaseConfig, options: &DatabaseOptions) -> Result<Self, store::Error> {
        Ok(Self::new(Database::open(config, options, &SCHEMA)?))
    }

    /// Wraps an already-opened [`Database`] whose schema includes `2026-10-26-000002_rifts_guard`.
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn database(&self) -> &Database {
        &self.db
    }

    fn read_sync<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        F: FnOnce(&mut AnyConnection) -> Result<T, SqlError> + Send + 'static,
        T: Send + 'static,
    {
        let db = self.db.clone();
        block_on_store(async move { db.read::<T, SqlError, _>(f).await.map_err(StoreError::from) })
    }

    fn write_sync<T, F>(&self, actor: store::Actor, f: F) -> Result<T, StoreError>
    where
        F: FnOnce(&mut AnyConnection) -> Result<T, SqlError> + Send + 'static,
        T: Send + 'static,
    {
        let db = self.db.clone();
        block_on_store(async move {
            db.write::<T, SqlError, _>(actor, move |tx: &mut Tx<'_>| f(tx.conn()))
                .await
                .map_err(StoreError::from)
        })
    }

    pub fn host_writes(&self) -> Result<Vec<HostWriteRecord>, StoreError> {
        self.read_sync(|conn| {
            let rows = sql(
                "SELECT kind, target, host, session, generation, realm, session_kind, hub_kind, summary, at \
                 FROM rifts_host_writes ORDER BY id ASC",
            )?
            .load::<HostWriteRow>(conn)?;
            rows.into_iter().map(HostWriteRow::into_record).collect()
        })
    }

    pub fn windows(&self) -> Result<Vec<TrafficWindowRecord>, StoreError> {
        self.read_sync(|conn| {
            let rows = sql("SELECT window_id, scope, window_start, requests, bytes, expires_at \
                 FROM rifts_traffic_windows ORDER BY window_id ASC")?
            .load::<TrafficWindowRow>(conn)?;
            rows.into_iter().map(TrafficWindowRow::into_record).collect()
        })
    }

    pub fn alert(&self, alert_id: &str) -> Result<Option<AlertRecord>, StoreError> {
        let id = alert_id.to_owned();
        self.read_sync(move |conn| find_alert(conn, &id))
    }

    pub fn xp_summary(&self, session: &SessionId, member: &AccountId) -> Result<Option<XpSummaryRecord>, StoreError> {
        let session_text = session.to_string();
        let member_text = member.0.clone();
        self.read_sync(move |conn| {
            let row = sql(
                "SELECT session, member, host, realm, minutes, grants, flagged, created_at \
                 FROM rifts_hunt_xp_summaries WHERE session = ? AND member = ?",
            )?
            .bind::<Text, _>(&session_text)
            .bind::<Text, _>(&member_text)
            .get_result::<XpSummaryRow>(conn)
            .optional()?;
            row.map(XpSummaryRow::into_record).transpose()
        })
    }

    /// Sweeper replacement for the TTL indexes on `rifts_traffic_windows.expires_at` and `rifts_hunt_xp_summaries.ttl_at`.
    pub fn purge_expired(&self, now: UnixMillis) -> Result<u64, StoreError> {
        let actor = store::Actor::System("rifts-guard-purge".to_owned());
        self.write_sync(actor, move |conn| {
            let removed_windows = sql("DELETE FROM rifts_traffic_windows WHERE expires_at <= ?")?
                .bind::<BigInt, _>(now.0)
                .execute(conn)?;
            let removed_summaries = sql("DELETE FROM rifts_hunt_xp_summaries WHERE ttl_at <= ?")?
                .bind::<BigInt, _>(now.0)
                .execute(conn)?;
            Ok((removed_windows + removed_summaries) as u64)
        })
    }
}

impl GuardStore for SqlGuardStore {
    fn record_host_write(&self, actor: &Actor, write: &HostWriteRecord) -> Result<(), StoreError> {
        let record = write.clone();
        self.write_sync(store_actor(actor), move |conn| {
            let kind = kind_name(record.kind);
            let target = record.target.0;
            let host = record.host.0;
            let session = record.session.to_string();
            let generation_i64 = to_i64(record.generation)?;
            let realm = record.realm.map(|r| r.to_string());
            let session_kind = record.session_kind;
            let hub_kind = record.hub_kind;
            let summary = record.summary.0;
            sql("INSERT INTO rifts_host_writes \
                 (kind, target, host, session, generation, realm, session_kind, hub_kind, summary, at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")?
            .bind::<Text, _>(kind)
            .bind::<Text, _>(target)
            .bind::<Text, _>(host)
            .bind::<Text, _>(session)
            .bind::<BigInt, _>(generation_i64)
            .bind::<Nullable<Text>, _>(realm)
            .bind::<Nullable<Text>, _>(session_kind)
            .bind::<Nullable<Text>, _>(hub_kind)
            .bind::<Text, _>(summary)
            .bind::<BigInt, _>(record.at.0)
            .execute(conn)?;
            Ok(())
        })
    }

    fn reserve_traffic(
        &self,
        actor: &Actor,
        host: &TrafficReservation,
        session: &TrafficReservation,
    ) -> Result<bool, StoreError> {
        let host_res = host.clone();
        let session_res = session.clone();
        self.write_sync(store_actor(actor), move |conn| {
            let now = host_res.window_start.0;
            sql("DELETE FROM rifts_traffic_windows WHERE expires_at <= ?")?
                .bind::<BigInt, _>(now)
                .execute(conn)?;

            let current_host = find_traffic_window(conn, &host_res.window_id)?;
            let Some(next_host) = reserve_one(current_host, &host_res) else {
                return Ok(false);
            };

            let current_session = if session_res.window_id == host_res.window_id {
                Some(next_host.clone())
            } else {
                find_traffic_window(conn, &session_res.window_id)?
            };
            let Some(next_session) = reserve_one(current_session, &session_res) else {
                return Ok(false);
            };

            let to_write = if session_res.window_id == host_res.window_id {
                vec![next_session]
            } else {
                vec![next_host, next_session]
            };

            for record in to_write {
                diesel::sql_query(upsert(
                    "rifts_traffic_windows",
                    &["window_id", "scope", "window_start", "requests", "bytes", "expires_at"],
                    &["window_id"],
                    &["scope", "window_start", "requests", "bytes", "expires_at"],
                ))
                .bind::<Text, _>(record.window_id)
                .bind::<Text, _>(record.scope.as_str())
                .bind::<BigInt, _>(record.window_start.0)
                .bind::<BigInt, _>(to_i64(record.requests)?)
                .bind::<BigInt, _>(to_i64(record.bytes)?)
                .bind::<BigInt, _>(record.expires_at.0)
                .execute(conn)?;
            }
            Ok(true)
        })
    }

    fn chain(&self, session: &SessionId, generation: u64) -> Result<Option<ChainRecord>, StoreError> {
        let session_text = session.to_string();
        self.read_sync(move |conn| {
            let generation_i64 = to_i64(generation)?;
            let row = sql(
                "SELECT session, generation, revision, completion_mode, last_sequence, head_hash, \
                 encounter_state, disposition, finding_count, batch_count, event_count, event_bytes, \
                 last_monotonic_ms, event_types, has_unattributed_damage, last_received_at, \
                 final_sequence, final_hash, completed_at \
                 FROM rifts_hunt_guard WHERE session = ? AND generation = ?",
            )?
            .bind::<Text, _>(&session_text)
            .bind::<BigInt, _>(generation_i64)
            .get_result::<ChainRow>(conn)
            .optional()?;
            row.map(ChainRow::into_record).transpose()
        })
    }

    fn apply_chain(&self, actor: &Actor, update: &ChainUpdate, expected_revision: u64) -> Result<bool, StoreError> {
        let mutation = update.clone();
        self.write_sync(store_actor(actor), move |conn| {
            let session_text = mutation.chain.session.to_string();
            let generation_i64 = to_i64(mutation.chain.generation)?;
            let current_row = sql("SELECT revision FROM rifts_hunt_guard WHERE session = ? AND generation = ?")?
                .bind::<Text, _>(&session_text)
                .bind::<BigInt, _>(generation_i64)
                .get_result::<RevisionRow>(conn)
                .optional()?;
            let stored_row_rev = current_row.map(|row| to_u64(row.revision)).transpose()?;
            let stored = stored_row_rev.unwrap_or(0);
            if stored != expected_revision {
                return Ok(false);
            }

            if let Some(batch) = &mutation.batch {
                let existing = sql("SELECT batch_id FROM rifts_guard_batches \
                     WHERE session = ? AND generation = ? AND batch_id = ?")?
                .bind::<Text, _>(&session_text)
                .bind::<BigInt, _>(generation_i64)
                .bind::<Text, _>(&batch.batch_id)
                .get_result::<BatchIdRow>(conn)
                .optional()?;
                if existing.is_some() {
                    return Ok(false);
                }
            }

            let chain = &mutation.chain;
            let chain_rev = to_i64(chain.revision)?;
            let completion_mode = chain.completion_mode.map(completion_mode_str);
            let last_sequence = to_i64(chain.last_sequence)?;
            let encounter_state = chain.encounter_state.as_str();
            let disposition = chain.disposition.as_str();
            let finding_count = to_i64(chain.finding_count)?;
            let batch_count = to_i64(chain.batch_count)?;
            let event_count = to_i64(chain.event_count)?;
            let event_bytes = to_i64(chain.event_bytes)?;
            let last_monotonic_ms = chain.last_monotonic_ms.map(to_i64).transpose()?;
            let event_types = serde_json::to_string(&chain.event_types).map_err(corrupt)?;
            let has_unattributed_damage = i32::from(chain.has_unattributed_damage);
            let last_received_at = chain.last_received_at.map(|t| t.0);
            let final_sequence = chain
                .completion
                .as_ref()
                .map(|c| to_i64(c.final_sequence))
                .transpose()?;
            let final_hash = chain.completion.as_ref().map(|c| c.final_hash.as_str());
            let completed_at = chain.completion.as_ref().map(|c| c.at.0);

            match stored_row_rev {
                None => {
                    let insert_res = sql("INSERT INTO rifts_hunt_guard \
                         (session, generation, revision, completion_mode, last_sequence, head_hash, \
                          encounter_state, disposition, finding_count, batch_count, event_count, event_bytes, \
                          last_monotonic_ms, event_types, has_unattributed_damage, last_received_at, \
                          final_sequence, final_hash, completed_at) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")?
                    .bind::<Text, _>(&session_text)
                    .bind::<BigInt, _>(generation_i64)
                    .bind::<BigInt, _>(chain_rev)
                    .bind::<Nullable<Text>, _>(completion_mode)
                    .bind::<BigInt, _>(last_sequence)
                    .bind::<Text, _>(&chain.head_hash)
                    .bind::<Text, _>(encounter_state)
                    .bind::<Text, _>(disposition)
                    .bind::<BigInt, _>(finding_count)
                    .bind::<BigInt, _>(batch_count)
                    .bind::<BigInt, _>(event_count)
                    .bind::<BigInt, _>(event_bytes)
                    .bind::<Nullable<BigInt>, _>(last_monotonic_ms)
                    .bind::<Text, _>(&event_types)
                    .bind::<Integer, _>(has_unattributed_damage)
                    .bind::<Nullable<BigInt>, _>(last_received_at)
                    .bind::<Nullable<BigInt>, _>(final_sequence)
                    .bind::<Nullable<Text>, _>(final_hash)
                    .bind::<Nullable<BigInt>, _>(completed_at)
                    .execute(conn);
                    match insert_res {
                        Ok(1) => {}
                        Ok(_) => return Ok(false),
                        Err(err) if is_unique_violation(&err) => return Ok(false),
                        Err(err) => return Err(SqlError::Query(err)),
                    }
                }
                Some(exp_rev) => {
                    let exp_rev_i64 = to_i64(exp_rev)?;
                    let updated = sql("UPDATE rifts_hunt_guard SET \
                         revision = ?, completion_mode = ?, last_sequence = ?, head_hash = ?, \
                         encounter_state = ?, disposition = ?, finding_count = ?, batch_count = ?, \
                         event_count = ?, event_bytes = ?, last_monotonic_ms = ?, event_types = ?, \
                         has_unattributed_damage = ?, last_received_at = ?, final_sequence = ?, \
                         final_hash = ?, completed_at = ? \
                         WHERE session = ? AND generation = ? AND revision = ?")?
                    .bind::<BigInt, _>(chain_rev)
                    .bind::<Nullable<Text>, _>(completion_mode)
                    .bind::<BigInt, _>(last_sequence)
                    .bind::<Text, _>(&chain.head_hash)
                    .bind::<Text, _>(encounter_state)
                    .bind::<Text, _>(disposition)
                    .bind::<BigInt, _>(finding_count)
                    .bind::<BigInt, _>(batch_count)
                    .bind::<BigInt, _>(event_count)
                    .bind::<BigInt, _>(event_bytes)
                    .bind::<Nullable<BigInt>, _>(last_monotonic_ms)
                    .bind::<Text, _>(&event_types)
                    .bind::<Integer, _>(has_unattributed_damage)
                    .bind::<Nullable<BigInt>, _>(last_received_at)
                    .bind::<Nullable<BigInt>, _>(final_sequence)
                    .bind::<Nullable<Text>, _>(final_hash)
                    .bind::<Nullable<BigInt>, _>(completed_at)
                    .bind::<Text, _>(&session_text)
                    .bind::<BigInt, _>(generation_i64)
                    .bind::<BigInt, _>(exp_rev_i64)
                    .execute(conn)?;
                    if updated != 1 {
                        return Ok(false);
                    }
                }
            }

            if let Some(batch) = mutation.batch {
                let batch_session = batch.session.to_string();
                let batch_gen = to_i64(batch.generation)?;
                let first_sequence = to_i64(batch.first_sequence)?;
                let last_sequence = to_i64(batch.last_sequence)?;
                let batch_events = to_i64(batch.event_count)?;
                let batch_bytes = to_i64(batch.event_bytes)?;
                let batch_types = serde_json::to_string(&batch.event_types).map_err(corrupt)?;
                sql("INSERT INTO rifts_guard_batches \
                     (session, generation, batch_id, previous_hash, batch_hash, first_sequence, \
                      last_sequence, event_count, event_bytes, event_types, received_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")?
                .bind::<Text, _>(batch_session)
                .bind::<BigInt, _>(batch_gen)
                .bind::<Text, _>(batch.batch_id)
                .bind::<Text, _>(batch.previous_hash)
                .bind::<Text, _>(batch.batch_hash)
                .bind::<BigInt, _>(first_sequence)
                .bind::<BigInt, _>(last_sequence)
                .bind::<BigInt, _>(batch_events)
                .bind::<BigInt, _>(batch_bytes)
                .bind::<Text, _>(batch_types)
                .bind::<BigInt, _>(batch.received_at.0)
                .execute(conn)?;
            }

            for finding in mutation.findings {
                let finding_session = finding.session.to_string();
                let finding_gen = to_i64(finding.generation)?;
                let finding_seq = finding.sequence.map(to_i64).transpose()?;
                sql("INSERT INTO rifts_guard_findings \
                     (session, generation, sequence, code, detail, at) \
                     VALUES (?, ?, ?, ?, ?, ?)")?
                .bind::<Text, _>(finding_session)
                .bind::<BigInt, _>(finding_gen)
                .bind::<Nullable<BigInt>, _>(finding_seq)
                .bind::<Text, _>(finding.code)
                .bind::<Text, _>(finding.detail)
                .bind::<BigInt, _>(finding.at.0)
                .execute(conn)?;
            }

            Ok(true)
        })
    }

    fn batch(&self, session: &SessionId, generation: u64, batch_id: &str) -> Result<Option<BatchRecord>, StoreError> {
        let session_text = session.to_string();
        let batch_text = batch_id.to_owned();
        self.read_sync(move |conn| {
            let generation_i64 = to_i64(generation)?;
            let row = sql(
                "SELECT session, generation, batch_id, previous_hash, batch_hash, first_sequence, \
                 last_sequence, event_count, event_bytes, event_types, received_at \
                 FROM rifts_guard_batches WHERE session = ? AND generation = ? AND batch_id = ?",
            )?
            .bind::<Text, _>(&session_text)
            .bind::<BigInt, _>(generation_i64)
            .bind::<Text, _>(&batch_text)
            .get_result::<BatchRow>(conn)
            .optional()?;
            row.map(BatchRow::into_record).transpose()
        })
    }

    fn findings(&self, session: &SessionId, generation: u64) -> Result<Vec<FindingRecord>, StoreError> {
        let session_text = session.to_string();
        self.read_sync(move |conn| {
            let generation_i64 = to_i64(generation)?;
            let rows = sql("SELECT session, generation, sequence, code, detail, at \
                 FROM rifts_guard_findings WHERE session = ? AND generation = ? ORDER BY id ASC")?
            .bind::<Text, _>(&session_text)
            .bind::<BigInt, _>(generation_i64)
            .load::<FindingRow>(conn)?;
            rows.into_iter().map(FindingRow::into_record).collect()
        })
    }

    fn put_xp_summary(&self, actor: &Actor, summary: &XpSummaryRecord) -> Result<(), StoreError> {
        let record = summary.clone();
        self.write_sync(store_actor(actor), move |conn| {
            let ttl_at = record.created_at.0.saturating_add(XP_SUMMARY_TTL_MS);
            diesel::sql_query(upsert(
                "rifts_hunt_xp_summaries",
                &[
                    "session",
                    "member",
                    "host",
                    "realm",
                    "minutes",
                    "grants",
                    "flagged",
                    "created_at",
                    "ttl_at",
                ],
                &["session", "member"],
                &["host", "realm", "minutes", "grants", "flagged", "created_at", "ttl_at"],
            ))
            .bind::<Text, _>(record.session.to_string())
            .bind::<Text, _>(record.member.0)
            .bind::<Text, _>(record.host.0)
            .bind::<Nullable<Text>, _>(record.realm.map(|r| r.to_string()))
            .bind::<BigInt, _>(i64::from(record.minutes))
            .bind::<Text, _>(record.grants.0)
            .bind::<Integer, _>(i32::from(record.flagged))
            .bind::<BigInt, _>(record.created_at.0)
            .bind::<BigInt, _>(ttl_at)
            .execute(conn)?;
            Ok(())
        })
    }

    fn insert_alert(&self, actor: &Actor, alert: &AlertRecord) -> Result<(), StoreError> {
        let record = alert.clone();
        self.write_sync(store_actor(actor), move |conn| {
            // $setOnInsert: a retried enqueue keeps the first alert
            diesel::sql_query(insert_ignore(
                "rifts_operations_alerts",
                &[
                    "alert_id",
                    "code",
                    "severity",
                    "message",
                    "details",
                    "created_at",
                    "acknowledged_at",
                    "acknowledged_by",
                ],
            ))
            .bind::<Text, _>(record.alert_id)
            .bind::<Text, _>(record.code)
            .bind::<Text, _>(record.severity)
            .bind::<Text, _>(record.message)
            .bind::<Text, _>(record.details.0)
            .bind::<BigInt, _>(record.created_at.0)
            .bind::<Nullable<BigInt>, _>(record.acknowledged_at.map(|t| t.0))
            .bind::<Nullable<Text>, _>(record.acknowledged_by)
            .execute(conn)?;
            Ok(())
        })
    }

    fn acknowledge_alert(&self, actor: &Actor, alert_id: &str, at: UnixMillis) -> Result<bool, StoreError> {
        let id = alert_id.to_owned();
        let by = actor_name(actor);
        self.write_sync(store_actor(actor), move |conn| {
            let updated = sql("UPDATE rifts_operations_alerts \
                 SET acknowledged_at = ?, acknowledged_by = ? \
                 WHERE alert_id = ? AND acknowledged_at IS NULL")?
            .bind::<BigInt, _>(at.0)
            .bind::<Text, _>(&by)
            .bind::<Text, _>(&id)
            .execute(conn)?;
            Ok(updated == 1)
        })
    }
}
