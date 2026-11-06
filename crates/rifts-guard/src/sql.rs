// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `GuardStore` on `store` (DD15): SQLite or Postgres, every write in the change log under its actor.

use std::{collections::BTreeMap, future::Future, str::FromStr, sync::OnceLock};

use diesel::{
    prelude::*,
    query_builder::{BoxedSqlQuery, SqlQuery},
    sql_types::{BigInt, Integer, Nullable, Text},
};
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use rifts_seam::{AccountId, Actor, Json, RealmId, SessionId, StoreError, UnixMillis};
use store::{AnyConnection, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema, MultiBackend, portable};
use tokio::runtime::{Handle, Runtime, RuntimeFlavor};

use crate::{
    store::{
        AlertRecord, BatchRecord, ChainRecord, ChainUpdate, CompletionMode, CompletionRecord, Disposition,
        EncounterState, FindingRecord, GuardStore, HostWriteRecord, TrafficReservation, TrafficScope,
        TrafficWindowRecord, XP_SUMMARY_TTL_MS, XpSummaryRecord, actor_name, charged,
    },
    writes::{kind_name, parse_kind},
};

/// Guard's SQLite schema migrations.
pub const SQLITE_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/sqlite");
/// Guard's Postgres schema migrations.
pub const POSTGRES_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/postgres");

/// Guard's schema for `store::Database`, both backends.
pub const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE_MIGRATIONS,
    postgres: || POSTGRES_MIGRATIONS,
    soft_delete: &[],
};

/// A host runtime acts with its host's capability, so its writes are that player's to revert.
pub fn store_actor(actor: &Actor) -> store::Actor {
    match actor {
        Actor::Admin { key_id } => store::Actor::Admin(key_id.clone()),
        Actor::System(task) => store::Actor::System(String::from(*task)),
        Actor::Account(player) | Actor::HostRuntime { host: player, .. } => store::Actor::Player(player.0.clone()),
    }
}

/// Two workers for callers on no runtime or on a current-thread one, made once.
fn fallback_runtime() -> Result<&'static Runtime, StoreError> {
    static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();
    let made = RUNTIME.get_or_init(|| {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder.worker_threads(2).thread_name("rifts-guard-store").enable_all();
        builder.build().map_err(|e| e.to_string())
    });
    made.as_ref().map_err(|e| StoreError::Database(e.clone()))
}

/// Runs a store future to its end from sync code, whatever runtime (if any) the caller is on.
pub fn block_on_store<F, T>(future: F) -> Result<T, StoreError>
where
    F: Future<Output = Result<T, StoreError>> + Send,
    T: Send,
{
    let Ok(handle) = Handle::try_current() else {
        return fallback_runtime()?.block_on(future);
    };
    if handle.runtime_flavor() == RuntimeFlavor::MultiThread {
        return tokio::task::block_in_place(|| handle.block_on(future));
    }
    // A current-thread runtime cannot block in place: its one thread is ours.
    let runtime = fallback_runtime()?;
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| runtime.block_on(future));
        worker
            .join()
            .unwrap_or_else(|_| Err(StoreError::Database("store worker thread panicked".into())))
    })
}

/// A database failure or a row that does not map back; both become `StoreError::Database`.
#[derive(Debug)]
struct Fail(String);

impl From<store::Error> for Fail {
    fn from(error: store::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<diesel::result::Error> for Fail {
    fn from(error: diesel::result::Error) -> Self {
        store::Error::Query(error).into()
    }
}

impl From<Fail> for StoreError {
    fn from(Fail(text): Fail) -> Self {
        Self::Database(text)
    }
}

// ---- values in and out ----------------------------------------------------------------------------------------

fn signed(value: u64, what: &str) -> Result<i64, Fail> {
    value
        .try_into()
        .map_err(|_| Fail(format!("{what} {value} is out of range")))
}

fn unsigned(value: i64, what: &str) -> Result<u64, Fail> {
    value
        .try_into()
        .map_err(|_| Fail(format!("{what} {value} is negative")))
}

fn parsed<T: FromStr>(text: &str, what: &str) -> Result<T, Fail> {
    T::from_str(text).map_err(|_| Fail(format!("{what} {text:?} does not parse")))
}

fn known<T>(value: Option<T>, text: &str, what: &str) -> Result<T, Fail> {
    match value {
        Some(value) => Ok(value),
        None => Err(Fail(format!("unknown {what} {text:?}"))),
    }
}

fn realm_of(text: Option<String>) -> Result<Option<RealmId>, Fail> {
    text.map(|realm| parsed(&realm, "realm")).transpose()
}

fn json_text(text: &str, what: &str) -> Result<serde_json::Value, Fail> {
    serde_json::from_str(text).map_err(|_| Fail(format!("{what} is not valid JSON")))
}

fn encoded(text: serde_json::Result<String>) -> Result<String, Fail> {
    text.map_err(|e| Fail(format!("event_types: {e}")))
}

/// A chain's `{"TYPE":count}` object.
fn type_counts(text: &str) -> Result<BTreeMap<String, u64>, Fail> {
    let value = json_text(text, "event_types")?;
    let counts = value.as_object().and_then(|object| {
        object
            .iter()
            .map(|(kind, count)| Some((kind.clone(), count.as_u64()?)))
            .collect::<Option<_>>()
    });
    counts.ok_or_else(|| Fail("event_types is not an object of counts".into()))
}

/// A batch's `["TYPE",…]` array.
fn type_names(text: &str) -> Result<Vec<String>, Fail> {
    let value = json_text(text, "event_types")?;
    let names = value
        .as_array()
        .and_then(|items| items.iter().map(|n| n.as_str().map(String::from)).collect());
    names.ok_or_else(|| Fail("event_types is not an array of names".into()))
}

/// One bind, in its column's SQL type.
enum Param {
    Text(String),
    MaybeText(Option<String>),
    Big(i64),
    MaybeBig(Option<i64>),
    Int(i32),
}

impl From<&str> for Param {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

impl From<String> for Param {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<i64> for Param {
    fn from(number: i64) -> Self {
        Self::Big(number)
    }
}

type Query = BoxedSqlQuery<'static, MultiBackend, SqlQuery>;

/// `text` with `params` bound in order; `text` is already numbered for the backend.
fn query(text: String, params: Vec<Param>) -> Query {
    let start = diesel::sql_query(text).into_boxed::<MultiBackend>();
    params.into_iter().fold(start, |query, param| match param {
        Param::Text(value) => query.bind::<Text, _>(value),
        Param::MaybeText(value) => query.bind::<Nullable<Text>, _>(value),
        Param::Big(value) => query.bind::<BigInt, _>(value),
        Param::MaybeBig(value) => query.bind::<Nullable<BigInt>, _>(value),
        Param::Int(value) => query.bind::<Integer, _>(value),
    })
}

/// Portable `?` placeholders numbered for the backend.
fn sql(text: &str) -> Result<String, Fail> {
    portable::numbered(text).map_err(Fail)
}

/// `INSERT INTO <table> (<columns>) VALUES (?, …)<tail>`.
fn insert_sql(table: &str, columns: &[&str], tail: &str) -> Result<String, Fail> {
    let marks = vec!["?"; columns.len()].join(", ");
    sql(&format!(
        "INSERT INTO {table} ({}) VALUES ({marks}){tail}",
        columns.join(", ")
    ))
}

// ---- rows -----------------------------------------------------------------------------------------------------

type MaybeText = Nullable<Text>;
type MaybeBig = Nullable<BigInt>;

// A row read back by name: each field with its SQL type.
macro_rules! rows {
    ($($row:ident { $($field:ident: $sql:ty => $rust:ty,)+ })+) => {$(
        #[derive(QueryableByName)]
        struct $row {
            $(#[diesel(sql_type = $sql)] $field: $rust,)+
        }
    )+};
}

rows! {
    ChainRow {
        session: Text => String,
        generation: BigInt => i64,
        revision: BigInt => i64,
        completion_mode: MaybeText => Option<String>,
        last_sequence: BigInt => i64,
        head_hash: Text => String,
        encounter_state: Text => String,
        disposition: Text => String,
        finding_count: BigInt => i64,
        batch_count: BigInt => i64,
        event_count: BigInt => i64,
        event_bytes: BigInt => i64,
        last_monotonic_ms: MaybeBig => Option<i64>,
        event_types: Text => String,
        has_unattributed_damage: Integer => i32,
        last_received_at: MaybeBig => Option<i64>,
        final_sequence: MaybeBig => Option<i64>,
        final_hash: MaybeText => Option<String>,
        completed_at: MaybeBig => Option<i64>,
    }
    BatchRow {
        session: Text => String,
        generation: BigInt => i64,
        batch_id: Text => String,
        previous_hash: Text => String,
        batch_hash: Text => String,
        first_sequence: BigInt => i64,
        last_sequence: BigInt => i64,
        event_count: BigInt => i64,
        event_bytes: BigInt => i64,
        event_types: Text => String,
        received_at: BigInt => i64,
    }
    FindingRow {
        session: Text => String,
        generation: BigInt => i64,
        sequence: MaybeBig => Option<i64>,
        code: Text => String,
        detail: Text => String,
        at: BigInt => i64,
    }
    WriteRow {
        kind: Text => String,
        target: Text => String,
        host: Text => String,
        session: Text => String,
        generation: BigInt => i64,
        realm: MaybeText => Option<String>,
        session_kind: MaybeText => Option<String>,
        hub_kind: MaybeText => Option<String>,
        summary: Text => String,
        at: BigInt => i64,
    }
    WindowRow {
        window_id: Text => String,
        scope: Text => String,
        window_start: BigInt => i64,
        requests: BigInt => i64,
        bytes: BigInt => i64,
        expires_at: BigInt => i64,
    }
    SummaryRow {
        session: Text => String,
        member: Text => String,
        host: Text => String,
        realm: MaybeText => Option<String>,
        minutes: BigInt => i64,
        grants: Text => String,
        flagged: Integer => i32,
        created_at: BigInt => i64,
    }
    AlertRow {
        alert_id: Text => String,
        code: Text => String,
        severity: Text => String,
        message: Text => String,
        details: Text => String,
        created_at: BigInt => i64,
        acknowledged_at: MaybeBig => Option<i64>,
        acknowledged_by: MaybeText => Option<String>,
    }
    Revision {
        revision: BigInt => i64,
    }
    Counts {
        requests: BigInt => i64,
        bytes: BigInt => i64,
    }
}

impl ChainRow {
    fn completion(
        final_sequence: Option<i64>,
        final_hash: Option<String>,
        at: Option<i64>,
    ) -> Result<Option<CompletionRecord>, Fail> {
        match (final_sequence, final_hash, at) {
            (Some(sequence), Some(hash), Some(at)) => Ok(Some(CompletionRecord {
                final_sequence: unsigned(sequence, "final_sequence")?,
                final_hash: hash,
                at: UnixMillis(at),
            })),
            (None, None, None) => Ok(None),
            _ => Err(Fail("a completion with some of its columns null".into())),
        }
    }

    fn into_record(self) -> Result<ChainRecord, Fail> {
        let completion_mode = match self.completion_mode {
            Some(text) => Some(known(CompletionMode::parse(&text), &text, "completion mode")?),
            None => None,
        };
        let event_types = type_counts(&self.event_types)?;
        let completion = Self::completion(self.final_sequence, self.final_hash, self.completed_at)?;
        let state = &self.encounter_state;
        Ok(ChainRecord {
            session: parsed(&self.session, "session")?,
            generation: unsigned(self.generation, "generation")?,
            revision: unsigned(self.revision, "revision")?,
            completion_mode,
            last_sequence: unsigned(self.last_sequence, "last_sequence")?,
            encounter_state: known(EncounterState::parse(state), state, "encounter state")?,
            disposition: known(Disposition::parse(&self.disposition), &self.disposition, "disposition")?,
            finding_count: unsigned(self.finding_count, "finding_count")?,
            batch_count: unsigned(self.batch_count, "batch_count")?,
            event_count: unsigned(self.event_count, "event_count")?,
            event_bytes: unsigned(self.event_bytes, "event_bytes")?,
            last_monotonic_ms: self
                .last_monotonic_ms
                .map(|ms| unsigned(ms, "last_monotonic_ms"))
                .transpose()?,
            event_types,
            has_unattributed_damage: self.has_unattributed_damage != 0,
            last_received_at: self.last_received_at.map(UnixMillis),
            head_hash: self.head_hash,
            completion,
        })
    }
}

impl BatchRow {
    fn into_record(self) -> Result<BatchRecord, Fail> {
        Ok(BatchRecord {
            session: parsed(&self.session, "session")?,
            generation: unsigned(self.generation, "generation")?,
            first_sequence: unsigned(self.first_sequence, "first_sequence")?,
            last_sequence: unsigned(self.last_sequence, "last_sequence")?,
            event_count: unsigned(self.event_count, "event_count")?,
            event_bytes: unsigned(self.event_bytes, "event_bytes")?,
            event_types: type_names(&self.event_types)?,
            received_at: UnixMillis(self.received_at),
            batch_id: self.batch_id,
            previous_hash: self.previous_hash,
            batch_hash: self.batch_hash,
        })
    }
}

impl FindingRow {
    fn into_record(self) -> Result<FindingRecord, Fail> {
        Ok(FindingRecord {
            session: parsed(&self.session, "session")?,
            generation: unsigned(self.generation, "generation")?,
            sequence: self.sequence.map(|s| unsigned(s, "sequence")).transpose()?,
            at: UnixMillis(self.at),
            code: self.code,
            detail: self.detail,
        })
    }
}

impl WriteRow {
    fn into_record(self) -> Result<HostWriteRecord, Fail> {
        Ok(HostWriteRecord {
            kind: known(parse_kind(&self.kind), &self.kind, "write kind")?,
            session: parsed(&self.session, "session")?,
            generation: unsigned(self.generation, "generation")?,
            realm: realm_of(self.realm)?,
            target: AccountId(self.target),
            host: AccountId(self.host),
            session_kind: self.session_kind,
            hub_kind: self.hub_kind,
            summary: Json(self.summary),
            at: UnixMillis(self.at),
        })
    }
}

impl WindowRow {
    fn into_record(self) -> Result<TrafficWindowRecord, Fail> {
        Ok(TrafficWindowRecord {
            scope: known(TrafficScope::parse(&self.scope), &self.scope, "scope")?,
            requests: unsigned(self.requests, "requests")?,
            bytes: unsigned(self.bytes, "bytes")?,
            window_id: self.window_id,
            window_start: UnixMillis(self.window_start),
            expires_at: UnixMillis(self.expires_at),
        })
    }
}

impl SummaryRow {
    fn into_record(self) -> Result<XpSummaryRecord, Fail> {
        let out_of_range = |minutes: i64| Fail(format!("minutes {minutes} is out of range"));
        Ok(XpSummaryRecord {
            session: parsed(&self.session, "session")?,
            realm: realm_of(self.realm)?,
            minutes: u32::try_from(self.minutes).map_err(|_| out_of_range(self.minutes))?,
            member: AccountId(self.member),
            host: AccountId(self.host),
            grants: Json(self.grants),
            flagged: self.flagged != 0,
            created_at: UnixMillis(self.created_at),
        })
    }
}

impl From<AlertRow> for AlertRecord {
    fn from(row: AlertRow) -> Self {
        Self {
            details: Json(row.details),
            created_at: UnixMillis(row.created_at),
            acknowledged_at: row.acknowledged_at.map(UnixMillis),
            alert_id: row.alert_id,
            code: row.code,
            severity: row.severity,
            message: row.message,
            acknowledged_by: row.acknowledged_by,
        }
    }
}

// ---- tables ---------------------------------------------------------------------------------------------------

/// `rifts_hunt_guard`: the key, then the columns a write sets, in bind order.
const CHAIN_KEY: [&str; 2] = ["session", "generation"];
const CHAIN_VALUES: [&str; 17] = [
    "revision",
    "completion_mode",
    "last_sequence",
    "head_hash",
    "encounter_state",
    "disposition",
    "finding_count",
    "batch_count",
    "event_count",
    "event_bytes",
    "last_monotonic_ms",
    "event_types",
    "has_unattributed_damage",
    "last_received_at",
    "final_sequence",
    "final_hash",
    "completed_at",
];
const BATCH_COLUMNS: [&str; 11] = [
    "session",
    "generation",
    "batch_id",
    "previous_hash",
    "batch_hash",
    "first_sequence",
    "last_sequence",
    "event_count",
    "event_bytes",
    "event_types",
    "received_at",
];
const FINDING_COLUMNS: [&str; 6] = ["session", "generation", "sequence", "code", "detail", "at"];
const WRITE_COLUMNS: [&str; 10] = [
    "kind",
    "target",
    "host",
    "session",
    "generation",
    "realm",
    "session_kind",
    "hub_kind",
    "summary",
    "at",
];
const WINDOW_COLUMNS: [&str; 6] = ["window_id", "scope", "window_start", "requests", "bytes", "expires_at"];
const SUMMARY_COLUMNS: [&str; 9] = [
    "session",
    "member",
    "host",
    "realm",
    "minutes",
    "grants",
    "flagged",
    "created_at",
    "ttl_at",
];
const ALERT_COLUMNS: [&str; 8] = [
    "alert_id",
    "code",
    "severity",
    "message",
    "details",
    "created_at",
    "acknowledged_at",
    "acknowledged_by",
];

fn chain_columns() -> Vec<&'static str> {
    CHAIN_KEY.iter().chain(&CHAIN_VALUES).copied().collect()
}

/// The chain's values in `CHAIN_VALUES`' order.
fn chain_values(chain: &ChainRecord) -> Result<Vec<Param>, Fail> {
    let done = chain.completion.as_ref();
    let counted = |value: u64, what: &str| signed(value, what).map(Param::Big);
    Ok(vec![
        counted(chain.revision, "revision")?,
        Param::MaybeText(chain.completion_mode.map(|mode| mode.as_str().into())),
        counted(chain.last_sequence, "last_sequence")?,
        chain.head_hash.as_str().into(),
        chain.encounter_state.as_str().into(),
        chain.disposition.as_str().into(),
        counted(chain.finding_count, "finding_count")?,
        counted(chain.batch_count, "batch_count")?,
        counted(chain.event_count, "event_count")?,
        counted(chain.event_bytes, "event_bytes")?,
        Param::MaybeBig(
            chain
                .last_monotonic_ms
                .map(|ms| signed(ms, "last_monotonic_ms"))
                .transpose()?,
        ),
        encoded(serde_json::to_string(&chain.event_types))?.into(),
        Param::Int(chain.has_unattributed_damage.into()),
        Param::MaybeBig(chain.last_received_at.map(|at| at.0)),
        Param::MaybeBig(done.map(|c| signed(c.final_sequence, "final_sequence")).transpose()?),
        Param::MaybeText(done.map(|c| c.final_hash.clone())),
        Param::MaybeBig(done.map(|c| c.at.0)),
    ])
}

fn chain_key(session: &str, generation: i64) -> Vec<Param> {
    vec![session.into(), generation.into()]
}

fn first_row<R, T>(rows: Vec<R>, into: impl FnOnce(R) -> Result<T, Fail>) -> Result<Option<T>, Fail> {
    rows.into_iter().next().map(into).transpose()
}

fn read_chain(conn: &mut AnyConnection, session: &str, generation: i64) -> Result<Option<ChainRecord>, Fail> {
    let text = format!(
        "SELECT {} FROM rifts_hunt_guard WHERE session = ? AND generation = ?",
        chain_columns().join(", ")
    );
    let rows = query(sql(&text)?, chain_key(session, generation)).load::<ChainRow>(conn)?;
    first_row(rows, ChainRow::into_record)
}

fn stored_revision(conn: &mut AnyConnection, session: &str, generation: i64) -> Result<i64, Fail> {
    let text = sql("SELECT revision FROM rifts_hunt_guard WHERE session = ? AND generation = ?")?;
    let rows = query(text, chain_key(session, generation)).load::<Revision>(conn)?;
    Ok(rows.first().map_or(0, |row| row.revision))
}

fn batch_exists(conn: &mut AnyConnection, batch: &BatchRecord) -> Result<bool, Fail> {
    let text = sql(
        "SELECT CAST(1 AS BIGINT) AS revision FROM rifts_guard_batches WHERE session = ? AND generation = ? AND batch_id = ?",
    )?;
    let mut key = chain_key(&batch.session.to_string(), signed(batch.generation, "generation")?);
    key.push(batch.batch_id.as_str().into());
    Ok(!query(text, key).load::<Revision>(conn)?.is_empty())
}

/// A fresh chain is inserted (a racing insert wins); a later one moves only from the expected revision.
fn write_chain(conn: &mut AnyConnection, chain: &ChainRecord, expected: u64) -> Result<bool, Fail> {
    let key = chain_key(&chain.session.to_string(), signed(chain.generation, "generation")?);
    let values = chain_values(chain)?;
    let changed = if expected == 0 {
        let text = insert_sql("rifts_hunt_guard", &chain_columns(), " ON CONFLICT DO NOTHING")?;
        query(text, key.into_iter().chain(values).collect()).execute(conn)?
    } else {
        let sets: Vec<String> = CHAIN_VALUES.iter().map(|column| format!("{column} = ?")).collect();
        let text = sql(&format!(
            "UPDATE rifts_hunt_guard SET {} WHERE session = ? AND generation = ? AND revision = ?",
            sets.join(", ")
        ))?;
        let mut params = values;
        params.extend(key);
        params.push(signed(expected, "revision")?.into());
        query(text, params).execute(conn)?
    };
    Ok(changed == 1)
}

fn insert_batch(conn: &mut AnyConnection, batch: &BatchRecord) -> Result<(), Fail> {
    let text = insert_sql("rifts_guard_batches", &BATCH_COLUMNS, "")?;
    let params = vec![
        batch.session.to_string().into(),
        signed(batch.generation, "generation")?.into(),
        batch.batch_id.as_str().into(),
        batch.previous_hash.as_str().into(),
        batch.batch_hash.as_str().into(),
        signed(batch.first_sequence, "first_sequence")?.into(),
        signed(batch.last_sequence, "last_sequence")?.into(),
        signed(batch.event_count, "event_count")?.into(),
        signed(batch.event_bytes, "event_bytes")?.into(),
        encoded(serde_json::to_string(&batch.event_types))?.into(),
        batch.received_at.0.into(),
    ];
    query(text, params).execute(conn)?;
    Ok(())
}

fn insert_finding(conn: &mut AnyConnection, finding: &FindingRecord) -> Result<(), Fail> {
    let text = insert_sql("rifts_guard_findings", &FINDING_COLUMNS, "")?;
    let params = vec![
        finding.session.to_string().into(),
        signed(finding.generation, "generation")?.into(),
        Param::MaybeBig(finding.sequence.map(|s| signed(s, "sequence")).transpose()?),
        finding.code.as_str().into(),
        finding.detail.as_str().into(),
        finding.at.0.into(),
    ];
    query(text, params).execute(conn)?;
    Ok(())
}

fn window_counts(conn: &mut AnyConnection, window_id: &str) -> Result<(u64, u64), Fail> {
    let text = sql("SELECT requests, bytes FROM rifts_traffic_windows WHERE window_id = ?")?;
    let rows = query(text, vec![window_id.into()]).load::<Counts>(conn)?;
    let counts = first_row(rows, |c| {
        Ok((unsigned(c.requests, "requests")?, unsigned(c.bytes, "bytes")?))
    })?;
    Ok(counts.unwrap_or_default())
}

fn put_window(
    conn: &mut AnyConnection,
    reservation: &TrafficReservation,
    (requests, bytes): (u64, u64),
) -> Result<(), Fail> {
    let text = portable::upsert(
        "rifts_traffic_windows",
        &WINDOW_COLUMNS,
        &["window_id"],
        &["requests", "bytes"],
    );
    let params = vec![
        reservation.window_id.as_str().into(),
        reservation.scope.as_str().into(),
        reservation.window_start.0.into(),
        signed(requests, "requests")?.into(),
        signed(bytes, "bytes")?.into(),
        reservation.expires_at.0.into(),
    ];
    query(text, params).execute(conn)?;
    Ok(())
}

fn drop_windows_through(conn: &mut AnyConnection, at: UnixMillis) -> Result<usize, Fail> {
    let text = sql("DELETE FROM rifts_traffic_windows WHERE expires_at <= ?")?;
    Ok(query(text, vec![at.0.into()]).execute(conn)?)
}

/// Guard's tables on a `store::Database`; cloning shares the pools.
#[derive(Clone)]
pub struct SqlGuardStore {
    db: Database,
}

impl SqlGuardStore {
    /// Opens (or creates) the database with Guard's migrations. Blocking.
    pub fn open(config: &DatabaseConfig, options: &DatabaseOptions) -> Result<Self, store::Error> {
        let db = Database::open(config, options, &SCHEMA)?;
        Ok(Self::new(db))
    }

    /// A store over an opened database.
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// The database underneath, for sharing its pools.
    pub fn database(&self) -> &Database {
        &self.db
    }

    fn read<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        F: FnOnce(&mut AnyConnection) -> Result<T, Fail> + Send + 'static,
        T: Send + 'static,
    {
        block_on_store(async { Ok(self.db.read(f).await?) })
    }

    fn write<T, F>(&self, actor: &Actor, f: F) -> Result<T, StoreError>
    where
        F: FnOnce(&mut AnyConnection) -> Result<T, Fail> + Send + 'static,
        T: Send + 'static,
    {
        let actor = store_actor(actor);
        block_on_store(async { Ok(self.db.write(actor, move |tx| f(tx.conn())).await?) })
    }

    /// Every recorded host write, oldest first.
    pub fn host_writes(&self) -> Result<Vec<HostWriteRecord>, StoreError> {
        self.read(|conn| {
            let text = format!("SELECT {} FROM rifts_host_writes ORDER BY id", WRITE_COLUMNS.join(", "));
            let rows = diesel::sql_query(text).load::<WriteRow>(conn)?;
            rows.into_iter().map(WriteRow::into_record).collect()
        })
    }

    /// Every traffic window, by id.
    pub fn windows(&self) -> Result<Vec<TrafficWindowRecord>, StoreError> {
        self.read(|conn| {
            let text = format!(
                "SELECT {} FROM rifts_traffic_windows ORDER BY window_id",
                WINDOW_COLUMNS.join(", ")
            );
            let rows = diesel::sql_query(text).load::<WindowRow>(conn)?;
            rows.into_iter().map(WindowRow::into_record).collect()
        })
    }

    /// One operator alert, `None` when there is none by that id.
    pub fn alert(&self, alert_id: &str) -> Result<Option<AlertRecord>, StoreError> {
        let id = Param::from(alert_id);
        self.read(move |conn| {
            let text = format!(
                "SELECT {} FROM rifts_operations_alerts WHERE alert_id = ?",
                ALERT_COLUMNS.join(", ")
            );
            let rows = query(sql(&text)?, vec![id]).load::<AlertRow>(conn)?;
            first_row(rows, |row| Ok(row.into()))
        })
    }

    /// A member's XP summary for a hunt session, `None` when none was kept.
    pub fn xp_summary(&self, session: &SessionId, member: &AccountId) -> Result<Option<XpSummaryRecord>, StoreError> {
        let key = vec![Param::Text(session.to_string()), Param::Text(member.0.clone())];
        self.read(move |conn| {
            let read = &SUMMARY_COLUMNS[..SUMMARY_COLUMNS.len() - 1];
            let text = format!(
                "SELECT {} FROM rifts_hunt_xp_summaries WHERE session = ? AND member = ?",
                read.join(", ")
            );
            let rows = query(sql(&text)?, key).load::<SummaryRow>(conn)?;
            first_row(rows, SummaryRow::into_record)
        })
    }

    /// Drops lapsed traffic windows and summaries; how many rows went.
    pub fn purge_expired(&self, now: UnixMillis) -> Result<u64, StoreError> {
        self.write(&Actor::System("rifts-guard-purge"), move |conn| {
            let windows = drop_windows_through(conn, now)?;
            let text = sql("DELETE FROM rifts_hunt_xp_summaries WHERE ttl_at <= ?")?;
            let summaries = query(text, vec![now.0.into()]).execute(conn)?;
            Ok((windows + summaries) as u64)
        })
    }
}

impl GuardStore for SqlGuardStore {
    fn chain(&self, session: &SessionId, generation: u64) -> Result<Option<ChainRecord>, StoreError> {
        let (session, generation) = (session.to_string(), signed(generation, "generation")?);
        self.read(move |conn| read_chain(conn, &session, generation))
    }

    fn apply_chain(&self, actor: &Actor, update: &ChainUpdate, expected_revision: u64) -> Result<bool, StoreError> {
        let update = update.clone();
        self.write(actor, move |conn| {
            let chain = &update.chain;
            let stored = stored_revision(
                conn,
                &chain.session.to_string(),
                signed(chain.generation, "generation")?,
            )?;
            if stored != signed(expected_revision, "revision")? {
                return Ok(false);
            }
            // A batch id already stored, or a revision moved under us, writes nothing.
            if let Some(batch) = &update.batch
                && batch_exists(conn, batch)?
            {
                return Ok(false);
            }
            if !write_chain(conn, chain, expected_revision)? {
                return Ok(false);
            }
            update.batch.iter().try_for_each(|batch| insert_batch(conn, batch))?;
            update
                .findings
                .iter()
                .try_for_each(|finding| insert_finding(conn, finding))?;
            Ok(true)
        })
    }

    fn batch(&self, session: &SessionId, generation: u64, batch_id: &str) -> Result<Option<BatchRecord>, StoreError> {
        let mut key = chain_key(&session.to_string(), signed(generation, "generation")?);
        key.push(batch_id.into());
        self.read(move |conn| {
            let text = format!(
                "SELECT {} FROM rifts_guard_batches WHERE session = ? AND generation = ? AND batch_id = ?",
                BATCH_COLUMNS.join(", ")
            );
            let rows = query(sql(&text)?, key).load::<BatchRow>(conn)?;
            first_row(rows, BatchRow::into_record)
        })
    }

    fn findings(&self, session: &SessionId, generation: u64) -> Result<Vec<FindingRecord>, StoreError> {
        let key = chain_key(&session.to_string(), signed(generation, "generation")?);
        self.read(move |conn| {
            let text = format!(
                "SELECT {} FROM rifts_guard_findings WHERE session = ? AND generation = ? ORDER BY id",
                FINDING_COLUMNS.join(", ")
            );
            let rows = query(sql(&text)?, key).load::<FindingRow>(conn)?;
            rows.into_iter().map(FindingRow::into_record).collect()
        })
    }

    fn reserve_traffic(
        &self,
        actor: &Actor,
        host: &TrafficReservation,
        session: &TrafficReservation,
    ) -> Result<bool, StoreError> {
        let (host, session) = (host.clone(), session.clone());
        self.write(actor, move |conn| {
            drop_windows_through(conn, host.window_start)?;
            let (requests, bytes) = window_counts(conn, &host.window_id)?;
            let Some(host_after) = charged(requests, bytes, &host) else {
                return Ok(false);
            };
            // One window for both scopes sees the host's charge first.
            let (requests, bytes) = if session.window_id == host.window_id {
                host_after
            } else {
                window_counts(conn, &session.window_id)?
            };
            let Some(session_after) = charged(requests, bytes, &session) else {
                return Ok(false);
            };
            put_window(conn, &host, host_after)?;
            put_window(conn, &session, session_after)?;
            Ok(true)
        })
    }

    fn record_host_write(&self, actor: &Actor, write: &HostWriteRecord) -> Result<(), StoreError> {
        let write = write.clone();
        self.write(actor, move |conn| {
            let text = insert_sql("rifts_host_writes", &WRITE_COLUMNS, "")?;
            let params = vec![
                kind_name(write.kind).into(),
                write.target.0.into(),
                write.host.0.into(),
                write.session.to_string().into(),
                signed(write.generation, "generation")?.into(),
                Param::MaybeText(write.realm.map(|realm| realm.to_string())),
                Param::MaybeText(write.session_kind),
                Param::MaybeText(write.hub_kind),
                write.summary.0.into(),
                write.at.0.into(),
            ];
            query(text, params).execute(conn)?;
            Ok(())
        })
    }

    fn put_xp_summary(&self, actor: &Actor, summary: &XpSummaryRecord) -> Result<(), StoreError> {
        let summary = summary.clone();
        self.write(actor, move |conn| {
            let text = portable::upsert(
                "rifts_hunt_xp_summaries",
                &SUMMARY_COLUMNS,
                &["session", "member"],
                &SUMMARY_COLUMNS[2..],
            );
            let expires = summary.created_at.0.saturating_add_unsigned(XP_SUMMARY_TTL_MS);
            let params = vec![
                summary.session.to_string().into(),
                summary.member.0.into(),
                summary.host.0.into(),
                Param::MaybeText(summary.realm.map(|realm| realm.to_string())),
                i64::from(summary.minutes).into(),
                summary.grants.0.into(),
                Param::Int(summary.flagged.into()),
                summary.created_at.0.into(),
                expires.into(),
            ];
            query(text, params).execute(conn)?;
            Ok(())
        })
    }

    fn insert_alert(&self, actor: &Actor, alert: &AlertRecord) -> Result<(), StoreError> {
        let alert = alert.clone();
        self.write(actor, move |conn| {
            let text = portable::insert_ignore("rifts_operations_alerts", &ALERT_COLUMNS);
            let params = vec![
                alert.alert_id.into(),
                alert.code.into(),
                alert.severity.into(),
                alert.message.into(),
                alert.details.0.into(),
                alert.created_at.0.into(),
                Param::MaybeBig(alert.acknowledged_at.map(|at| at.0)),
                Param::MaybeText(alert.acknowledged_by),
            ];
            query(text, params).execute(conn)?;
            Ok(())
        })
    }

    fn acknowledge_alert(&self, actor: &Actor, alert_id: &str, at: UnixMillis) -> Result<bool, StoreError> {
        let params = vec![at.0.into(), actor_name(actor).into(), alert_id.into()];
        self.write(actor, move |conn| {
            let text = sql(
                "UPDATE rifts_operations_alerts SET acknowledged_at = ?, acknowledged_by = ? \
                 WHERE alert_id = ? AND acknowledged_at IS NULL",
            )?;
            Ok(query(text, params).execute(conn)? == 1)
        })
    }
}
