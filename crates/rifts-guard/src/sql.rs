// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `GuardStore` on `store` (DD15): SQLite or Postgres, every write in the change log under its actor.

use std::collections::BTreeMap;
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use rifts_seam::{AccountId, Actor, Json, RealmId, SessionId, StoreError, UnixMillis};
use store::{AnyConnection, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema, MultiBackend, portable};
use tokio::runtime::{Handle, Runtime, RuntimeFlavor};

use crate::store::{
    AlertRecord, BatchRecord, ChainRecord, ChainUpdate, CompletionMode, CompletionRecord, Disposition, EncounterState,
    FindingRecord, GuardStore, HostWriteRecord, TrafficReservation, TrafficScope, TrafficWindowRecord,
    XP_SUMMARY_TTL_MS, XpSummaryRecord, actor_name, charged,
};
use crate::writes::{kind_name, parse_kind};

pub const SQLITE_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/sqlite");
pub const POSTGRES_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/postgres");

pub const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE_MIGRATIONS,
    postgres: || POSTGRES_MIGRATIONS,
    soft_delete: &[],
};

/// A host runtime acts with its host's capability, so its writes are that player's to revert.
pub fn store_actor(actor: &Actor) -> store::Actor {
    match actor {
        Actor::Account(account) => store::Actor::Player(account.0.clone()),
        Actor::HostRuntime { host, .. } => store::Actor::Player(host.0.clone()),
        Actor::Admin { key_id } => store::Actor::Admin(key_id.clone()),
        Actor::System(task) => store::Actor::System((*task).to_owned()),
    }
}

fn fallback_runtime() -> Result<&'static Runtime, StoreError> {
    static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("rifts-guard-store")
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| StoreError::Database(e.clone()))
}

/// Runs a store future to its end from sync code, whatever runtime (if any) the caller is on.
pub fn block_on_store<F, T>(future: F) -> Result<T, StoreError>
where
    F: Future<Output = Result<T, StoreError>> + Send,
    T: Send,
{
    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| handle.block_on(future))
        }
        // a current-thread runtime cannot block in place: its one thread is ours
        Ok(_) => {
            let runtime = fallback_runtime()?;
            std::thread::scope(|scope| {
                scope
                    .spawn(|| runtime.block_on(future))
                    .join()
                    .unwrap_or_else(|_| Err(StoreError::Database("store worker thread panicked".to_owned())))
            })
        }
        Err(_) => fallback_runtime()?.block_on(future),
    }
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
        Self(store::Error::Query(error).to_string())
    }
}

impl From<Fail> for StoreError {
    fn from(fail: Fail) -> Self {
        Self::Database(fail.0)
    }
}

fn sql(text: &str) -> Result<String, Fail> {
    portable::numbered(text).map_err(Fail)
}

fn signed(value: u64, what: &str) -> Result<i64, Fail> {
    i64::try_from(value).map_err(|_| Fail(format!("{what} {value} is out of range")))
}

fn unsigned(value: i64, what: &str) -> Result<u64, Fail> {
    u64::try_from(value).map_err(|_| Fail(format!("{what} {value} is negative")))
}

fn parsed<T: FromStr>(text: &str, what: &str) -> Result<T, Fail> {
    text.parse()
        .map_err(|_| Fail(format!("{what} {text:?} does not parse")))
}

fn known<T>(value: Option<T>, text: &str, what: &str) -> Result<T, Fail> {
    value.ok_or_else(|| Fail(format!("unknown {what} {text:?}")))
}

fn json_text(text: &str, what: &str) -> Result<serde_json::Value, Fail> {
    serde_json::from_str(text).map_err(|_| Fail(format!("{what} is not valid JSON")))
}

/// A chain's `{"TYPE":count}` object.
fn type_counts(text: &str) -> Result<BTreeMap<String, u64>, Fail> {
    let bad = || Fail("event_types is not an object of counts".to_owned());
    let value = json_text(text, "event_types")?;
    let object = value.as_object().ok_or_else(bad)?;
    object
        .iter()
        .map(|(kind, count)| Ok((kind.clone(), count.as_u64().ok_or_else(bad)?)))
        .collect()
}

/// A batch's `["TYPE",…]` array.
fn type_names(text: &str) -> Result<Vec<String>, Fail> {
    let bad = || Fail("event_types is not an array of names".to_owned());
    let value = json_text(text, "event_types")?;
    value
        .as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|name| name.as_str().map(str::to_owned).ok_or_else(bad))
        .collect()
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
    fn into_record(self) -> Result<ChainRecord, Fail> {
        let completion_mode = match self.completion_mode {
            None => None,
            Some(text) => Some(known(CompletionMode::parse(&text), &text, "completion mode")?),
        };
        let event_types = type_counts(&self.event_types)?;
        let completion = match (self.final_sequence, self.final_hash, self.completed_at) {
            (None, None, None) => None,
            (Some(sequence), Some(hash), Some(at)) => Some(CompletionRecord {
                final_sequence: unsigned(sequence, "final_sequence")?,
                final_hash: hash,
                at: UnixMillis(at),
            }),
            _ => return Err(Fail("a completion with some of its columns null".to_owned())),
        };
        Ok(ChainRecord {
            session: parsed(&self.session, "session")?,
            generation: unsigned(self.generation, "generation")?,
            revision: unsigned(self.revision, "revision")?,
            completion_mode,
            last_sequence: unsigned(self.last_sequence, "last_sequence")?,
            head_hash: self.head_hash,
            encounter_state: known(
                EncounterState::parse(&self.encounter_state),
                &self.encounter_state,
                "encounter state",
            )?,
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
            completion,
        })
    }
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
    fn into_record(self) -> Result<BatchRecord, Fail> {
        Ok(BatchRecord {
            session: parsed(&self.session, "session")?,
            generation: unsigned(self.generation, "generation")?,
            batch_id: self.batch_id,
            previous_hash: self.previous_hash,
            batch_hash: self.batch_hash,
            first_sequence: unsigned(self.first_sequence, "first_sequence")?,
            last_sequence: unsigned(self.last_sequence, "last_sequence")?,
            event_count: unsigned(self.event_count, "event_count")?,
            event_bytes: unsigned(self.event_bytes, "event_bytes")?,
            event_types: type_names(&self.event_types)?,
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

#[derive(QueryableByName)]
struct WriteRow {
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

#[derive(QueryableByName)]
struct WindowRow {
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

impl WindowRow {
    fn into_record(self) -> Result<TrafficWindowRecord, Fail> {
        Ok(TrafficWindowRecord {
            scope: known(TrafficScope::parse(&self.scope), &self.scope, "scope")?,
            window_id: self.window_id,
            window_start: UnixMillis(self.window_start),
            requests: unsigned(self.requests, "requests")?,
            bytes: unsigned(self.bytes, "bytes")?,
            expires_at: UnixMillis(self.expires_at),
        })
    }
}

#[derive(QueryableByName)]
struct SummaryRow {
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

#[derive(QueryableByName)]
struct Revision {
    #[diesel(sql_type = BigInt)]
    revision: i64,
}

#[derive(QueryableByName)]
struct Counts {
    #[diesel(sql_type = BigInt)]
    requests: i64,
    #[diesel(sql_type = BigInt)]
    bytes: i64,
}

const CHAIN_COLUMNS: &str = "session, generation, revision, completion_mode, last_sequence, head_hash, \
     encounter_state, disposition, finding_count, batch_count, event_count, event_bytes, last_monotonic_ms, \
     event_types, has_unattributed_damage, last_received_at, final_sequence, final_hash, completed_at";

const BATCH_COLUMNS: &str = "session, generation, batch_id, previous_hash, batch_hash, first_sequence, \
     last_sequence, event_count, event_bytes, event_types, received_at";

fn read_chain(conn: &mut AnyConnection, session: &str, generation: i64) -> Result<Option<ChainRecord>, Fail> {
    let query = sql(&format!(
        "SELECT {CHAIN_COLUMNS} FROM rifts_hunt_guard WHERE session = ? AND generation = ?"
    ))?;
    diesel::sql_query(query)
        .bind::<Text, _>(session)
        .bind::<BigInt, _>(generation)
        .load::<ChainRow>(conn)?
        .into_iter()
        .next()
        .map(ChainRow::into_record)
        .transpose()
}

fn batch_exists(conn: &mut AnyConnection, session: &str, generation: i64, batch_id: &str) -> Result<bool, Fail> {
    let query = sql(
        "SELECT CAST(1 AS BIGINT) AS revision FROM rifts_guard_batches WHERE session = ? AND generation = ? AND batch_id = ?",
    )?;
    Ok(!diesel::sql_query(query)
        .bind::<Text, _>(session)
        .bind::<BigInt, _>(generation)
        .bind::<Text, _>(batch_id)
        .load::<Revision>(conn)?
        .is_empty())
}

/// The chain's values in `CHAIN_COLUMNS`' order after the key.
fn bind_chain(
    query: diesel::query_builder::BoxedSqlQuery<'static, MultiBackend, diesel::query_builder::SqlQuery>,
    chain: &ChainRecord,
) -> Result<diesel::query_builder::BoxedSqlQuery<'static, MultiBackend, diesel::query_builder::SqlQuery>, Fail> {
    let event_types = serde_json::to_string(&chain.event_types).map_err(|e| Fail(format!("event_types: {e}")))?;
    let completion = chain.completion.as_ref();
    Ok(query
        .bind::<BigInt, _>(signed(chain.revision, "revision")?)
        .bind::<Nullable<Text>, _>(chain.completion_mode.map(|m| m.as_str().to_owned()))
        .bind::<BigInt, _>(signed(chain.last_sequence, "last_sequence")?)
        .bind::<Text, _>(chain.head_hash.clone())
        .bind::<Text, _>(chain.encounter_state.as_str().to_owned())
        .bind::<Text, _>(chain.disposition.as_str().to_owned())
        .bind::<BigInt, _>(signed(chain.finding_count, "finding_count")?)
        .bind::<BigInt, _>(signed(chain.batch_count, "batch_count")?)
        .bind::<BigInt, _>(signed(chain.event_count, "event_count")?)
        .bind::<BigInt, _>(signed(chain.event_bytes, "event_bytes")?)
        .bind::<Nullable<BigInt>, _>(
            chain
                .last_monotonic_ms
                .map(|ms| signed(ms, "last_monotonic_ms"))
                .transpose()?,
        )
        .bind::<Text, _>(event_types)
        .bind::<Integer, _>(i32::from(chain.has_unattributed_damage))
        .bind::<Nullable<BigInt>, _>(chain.last_received_at.map(|at| at.0))
        .bind::<Nullable<BigInt>, _>(
            completion
                .map(|c| signed(c.final_sequence, "final_sequence"))
                .transpose()?,
        )
        .bind::<Nullable<Text>, _>(completion.map(|c| c.final_hash.clone()))
        .bind::<Nullable<BigInt>, _>(completion.map(|c| c.at.0)))
}

fn write_chain(conn: &mut AnyConnection, chain: &ChainRecord, expected: u64) -> Result<bool, Fail> {
    let session = chain.session.to_string();
    let generation = signed(chain.generation, "generation")?;
    let changed = if expected == 0 {
        let query = sql(&format!(
            "INSERT INTO rifts_hunt_guard ({CHAIN_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT DO NOTHING"
        ))?;
        let query = diesel::sql_query(query)
            .into_boxed::<MultiBackend>()
            .bind::<Text, _>(session)
            .bind::<BigInt, _>(generation);
        bind_chain(query, chain)?.execute(conn)?
    } else {
        let query = sql(
            "UPDATE rifts_hunt_guard SET revision = ?, completion_mode = ?, last_sequence = ?, head_hash = ?, \
             encounter_state = ?, disposition = ?, finding_count = ?, batch_count = ?, event_count = ?, \
             event_bytes = ?, last_monotonic_ms = ?, event_types = ?, has_unattributed_damage = ?, \
             last_received_at = ?, final_sequence = ?, final_hash = ?, completed_at = ? \
             WHERE session = ? AND generation = ? AND revision = ?",
        )?;
        let query = diesel::sql_query(query).into_boxed::<MultiBackend>();
        bind_chain(query, chain)?
            .bind::<Text, _>(session)
            .bind::<BigInt, _>(generation)
            .bind::<BigInt, _>(signed(expected, "revision")?)
            .execute(conn)?
    };
    Ok(changed == 1)
}

fn insert_batch(conn: &mut AnyConnection, batch: &BatchRecord) -> Result<(), Fail> {
    let query = sql(&format!(
        "INSERT INTO rifts_guard_batches ({BATCH_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    ))?;
    let types = serde_json::to_string(&batch.event_types).map_err(|e| Fail(format!("event_types: {e}")))?;
    diesel::sql_query(query)
        .bind::<Text, _>(batch.session.to_string())
        .bind::<BigInt, _>(signed(batch.generation, "generation")?)
        .bind::<Text, _>(&batch.batch_id)
        .bind::<Text, _>(&batch.previous_hash)
        .bind::<Text, _>(&batch.batch_hash)
        .bind::<BigInt, _>(signed(batch.first_sequence, "first_sequence")?)
        .bind::<BigInt, _>(signed(batch.last_sequence, "last_sequence")?)
        .bind::<BigInt, _>(signed(batch.event_count, "event_count")?)
        .bind::<BigInt, _>(signed(batch.event_bytes, "event_bytes")?)
        .bind::<Text, _>(types)
        .bind::<BigInt, _>(batch.received_at.0)
        .execute(conn)?;
    Ok(())
}

fn insert_finding(conn: &mut AnyConnection, finding: &FindingRecord) -> Result<(), Fail> {
    let query = sql(
        "INSERT INTO rifts_guard_findings (session, generation, sequence, code, detail, at) VALUES (?, ?, ?, ?, ?, ?)",
    )?;
    diesel::sql_query(query)
        .bind::<Text, _>(finding.session.to_string())
        .bind::<BigInt, _>(signed(finding.generation, "generation")?)
        .bind::<Nullable<BigInt>, _>(finding.sequence.map(|s| signed(s, "sequence")).transpose()?)
        .bind::<Text, _>(&finding.code)
        .bind::<Text, _>(&finding.detail)
        .bind::<BigInt, _>(finding.at.0)
        .execute(conn)?;
    Ok(())
}

fn window_counts(conn: &mut AnyConnection, window_id: &str) -> Result<Option<(u64, u64)>, Fail> {
    let query = sql("SELECT requests, bytes FROM rifts_traffic_windows WHERE window_id = ?")?;
    diesel::sql_query(query)
        .bind::<Text, _>(window_id)
        .load::<Counts>(conn)?
        .into_iter()
        .next()
        .map(|c| Ok((unsigned(c.requests, "requests")?, unsigned(c.bytes, "bytes")?)))
        .transpose()
}

fn put_window(conn: &mut AnyConnection, reservation: &TrafficReservation, counts: (u64, u64)) -> Result<(), Fail> {
    let query = portable::upsert(
        "rifts_traffic_windows",
        &["window_id", "scope", "window_start", "requests", "bytes", "expires_at"],
        &["window_id"],
        &["requests", "bytes"],
    );
    diesel::sql_query(query)
        .bind::<Text, _>(&reservation.window_id)
        .bind::<Text, _>(reservation.scope.as_str())
        .bind::<BigInt, _>(reservation.window_start.0)
        .bind::<BigInt, _>(signed(counts.0, "requests")?)
        .bind::<BigInt, _>(signed(counts.1, "bytes")?)
        .bind::<BigInt, _>(reservation.expires_at.0)
        .execute(conn)?;
    Ok(())
}

/// Guard's tables on a `store::Database`; cloning shares the pools.
#[derive(Clone)]
pub struct SqlGuardStore {
    db: Database,
}

impl SqlGuardStore {
    /// Opens (or creates) the database with Guard's migrations. Blocking.
    pub fn open(config: &DatabaseConfig, options: &DatabaseOptions) -> Result<Self, store::Error> {
        Database::open(config, options, &SCHEMA).map(Self::new)
    }

    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn database(&self) -> &Database {
        &self.db
    }

    fn read<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        F: FnOnce(&mut AnyConnection) -> Result<T, Fail> + Send + 'static,
        T: Send + 'static,
    {
        block_on_store(async { self.db.read(f).await.map_err(StoreError::from) })
    }

    fn write<T, F>(&self, actor: &Actor, f: F) -> Result<T, StoreError>
    where
        F: FnOnce(&mut AnyConnection) -> Result<T, Fail> + Send + 'static,
        T: Send + 'static,
    {
        let actor = store_actor(actor);
        block_on_store(async {
            self.db
                .write(actor, move |tx| f(tx.conn()))
                .await
                .map_err(StoreError::from)
        })
    }

    /// Every recorded host write, oldest first.
    pub fn host_writes(&self) -> Result<Vec<HostWriteRecord>, StoreError> {
        self.read(|conn| {
            let rows = diesel::sql_query(
                "SELECT kind, target, host, session, generation, realm, session_kind, hub_kind, summary, at \
                 FROM rifts_host_writes ORDER BY id",
            )
            .load::<WriteRow>(conn)?;
            rows.into_iter()
                .map(|row| {
                    Ok(HostWriteRecord {
                        kind: known(parse_kind(&row.kind), &row.kind, "write kind")?,
                        target: AccountId(row.target),
                        host: AccountId(row.host),
                        session: parsed(&row.session, "session")?,
                        generation: unsigned(row.generation, "generation")?,
                        realm: row.realm.map(|r| parsed::<RealmId>(&r, "realm")).transpose()?,
                        session_kind: row.session_kind,
                        hub_kind: row.hub_kind,
                        summary: Json(row.summary),
                        at: UnixMillis(row.at),
                    })
                })
                .collect()
        })
    }

    pub fn windows(&self) -> Result<Vec<TrafficWindowRecord>, StoreError> {
        self.read(|conn| {
            diesel::sql_query(
                "SELECT window_id, scope, window_start, requests, bytes, expires_at FROM rifts_traffic_windows \
                 ORDER BY window_id",
            )
            .load::<WindowRow>(conn)?
            .into_iter()
            .map(WindowRow::into_record)
            .collect()
        })
    }

    pub fn alert(&self, alert_id: &str) -> Result<Option<AlertRecord>, StoreError> {
        let alert_id = alert_id.to_owned();
        self.read(move |conn| {
            let query = sql(
                "SELECT alert_id, code, severity, message, details, created_at, acknowledged_at, acknowledged_by \
                 FROM rifts_operations_alerts WHERE alert_id = ?",
            )?;
            Ok(diesel::sql_query(query)
                .bind::<Text, _>(alert_id)
                .load::<AlertRow>(conn)?
                .into_iter()
                .next()
                .map(|row| AlertRecord {
                    alert_id: row.alert_id,
                    code: row.code,
                    severity: row.severity,
                    message: row.message,
                    details: Json(row.details),
                    created_at: UnixMillis(row.created_at),
                    acknowledged_at: row.acknowledged_at.map(UnixMillis),
                    acknowledged_by: row.acknowledged_by,
                }))
        })
    }

    pub fn xp_summary(&self, session: &SessionId, member: &AccountId) -> Result<Option<XpSummaryRecord>, StoreError> {
        let (session, member) = (session.to_string(), member.0.clone());
        self.read(move |conn| {
            let query = sql(
                "SELECT session, member, host, realm, minutes, grants, flagged, created_at \
                 FROM rifts_hunt_xp_summaries WHERE session = ? AND member = ?",
            )?;
            diesel::sql_query(query)
                .bind::<Text, _>(session)
                .bind::<Text, _>(member)
                .load::<SummaryRow>(conn)?
                .into_iter()
                .next()
                .map(|row| {
                    Ok(XpSummaryRecord {
                        session: parsed(&row.session, "session")?,
                        member: AccountId(row.member),
                        host: AccountId(row.host),
                        realm: row.realm.map(|r| parsed::<RealmId>(&r, "realm")).transpose()?,
                        minutes: u32::try_from(row.minutes)
                            .map_err(|_| Fail(format!("minutes {} is out of range", row.minutes)))?,
                        grants: Json(row.grants),
                        flagged: row.flagged != 0,
                        created_at: UnixMillis(row.created_at),
                    })
                })
                .transpose()
        })
    }

    /// Drops lapsed traffic windows and summaries; how many rows went.
    pub fn purge_expired(&self, now: UnixMillis) -> Result<u64, StoreError> {
        self.write(&Actor::System("rifts-guard-purge"), move |conn| {
            let windows = diesel::sql_query(sql("DELETE FROM rifts_traffic_windows WHERE expires_at <= ?")?)
                .bind::<BigInt, _>(now.0)
                .execute(conn)?;
            let summaries = diesel::sql_query(sql("DELETE FROM rifts_hunt_xp_summaries WHERE ttl_at <= ?")?)
                .bind::<BigInt, _>(now.0)
                .execute(conn)?;
            Ok((windows + summaries) as u64)
        })
    }
}

impl GuardStore for SqlGuardStore {
    fn record_host_write(&self, actor: &Actor, write: &HostWriteRecord) -> Result<(), StoreError> {
        let write = write.clone();
        self.write(actor, move |conn| {
            let query = sql(
                "INSERT INTO rifts_host_writes (kind, target, host, session, generation, realm, session_kind, \
                 hub_kind, summary, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )?;
            diesel::sql_query(query)
                .bind::<Text, _>(kind_name(write.kind))
                .bind::<Text, _>(write.target.0)
                .bind::<Text, _>(write.host.0)
                .bind::<Text, _>(write.session.to_string())
                .bind::<BigInt, _>(signed(write.generation, "generation")?)
                .bind::<Nullable<Text>, _>(write.realm.map(|r| r.to_string()))
                .bind::<Nullable<Text>, _>(write.session_kind)
                .bind::<Nullable<Text>, _>(write.hub_kind)
                .bind::<Text, _>(write.summary.0)
                .bind::<BigInt, _>(write.at.0)
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
        let (host, session) = (host.clone(), session.clone());
        self.write(actor, move |conn| {
            diesel::sql_query(sql("DELETE FROM rifts_traffic_windows WHERE expires_at <= ?")?)
                .bind::<BigInt, _>(host.window_start.0)
                .execute(conn)?;
            let host_before = window_counts(conn, &host.window_id)?.unwrap_or((0, 0));
            let Some(host_after) = charged(host_before.0, host_before.1, &host) else {
                return Ok(false);
            };
            let session_before = if session.window_id == host.window_id {
                host_after
            } else {
                window_counts(conn, &session.window_id)?.unwrap_or((0, 0))
            };
            let Some(session_after) = charged(session_before.0, session_before.1, &session) else {
                return Ok(false);
            };
            put_window(conn, &host, host_after)?;
            put_window(conn, &session, session_after)?;
            Ok(true)
        })
    }

    fn chain(&self, session: &SessionId, generation: u64) -> Result<Option<ChainRecord>, StoreError> {
        let session = session.to_string();
        let generation = signed(generation, "generation")?;
        self.read(move |conn| read_chain(conn, &session, generation))
    }

    fn apply_chain(&self, actor: &Actor, update: &ChainUpdate, expected_revision: u64) -> Result<bool, StoreError> {
        let update = update.clone();
        self.write(actor, move |conn| {
            let chain = &update.chain;
            let session = chain.session.to_string();
            let generation = signed(chain.generation, "generation")?;
            let query = sql("SELECT revision FROM rifts_hunt_guard WHERE session = ? AND generation = ?")?;
            let stored = diesel::sql_query(query)
                .bind::<Text, _>(&session)
                .bind::<BigInt, _>(generation)
                .load::<Revision>(conn)?
                .first()
                .map_or(0, |row| row.revision);
            if stored != signed(expected_revision, "revision")? {
                return Ok(false);
            }
            if let Some(batch) = &update.batch
                && batch_exists(
                    conn,
                    &batch.session.to_string(),
                    signed(batch.generation, "generation")?,
                    &batch.batch_id,
                )?
            {
                return Ok(false);
            }
            if !write_chain(conn, chain, expected_revision)? {
                return Ok(false);
            }
            if let Some(batch) = &update.batch {
                insert_batch(conn, batch)?;
            }
            for finding in &update.findings {
                insert_finding(conn, finding)?;
            }
            Ok(true)
        })
    }

    fn batch(&self, session: &SessionId, generation: u64, batch_id: &str) -> Result<Option<BatchRecord>, StoreError> {
        let (session, batch_id) = (session.to_string(), batch_id.to_owned());
        let generation = signed(generation, "generation")?;
        self.read(move |conn| {
            let query = sql(&format!(
                "SELECT {BATCH_COLUMNS} FROM rifts_guard_batches WHERE session = ? AND generation = ? AND batch_id = ?"
            ))?;
            diesel::sql_query(query)
                .bind::<Text, _>(session)
                .bind::<BigInt, _>(generation)
                .bind::<Text, _>(batch_id)
                .load::<BatchRow>(conn)?
                .into_iter()
                .next()
                .map(BatchRow::into_record)
                .transpose()
        })
    }

    fn findings(&self, session: &SessionId, generation: u64) -> Result<Vec<FindingRecord>, StoreError> {
        let session = session.to_string();
        let generation = signed(generation, "generation")?;
        self.read(move |conn| {
            let query = sql(
                "SELECT session, generation, sequence, code, detail, at FROM rifts_guard_findings \
                 WHERE session = ? AND generation = ? ORDER BY id",
            )?;
            diesel::sql_query(query)
                .bind::<Text, _>(session)
                .bind::<BigInt, _>(generation)
                .load::<FindingRow>(conn)?
                .into_iter()
                .map(|row| {
                    Ok(FindingRecord {
                        session: parsed(&row.session, "session")?,
                        generation: unsigned(row.generation, "generation")?,
                        sequence: row.sequence.map(|s| unsigned(s, "sequence")).transpose()?,
                        code: row.code,
                        detail: row.detail,
                        at: UnixMillis(row.at),
                    })
                })
                .collect()
        })
    }

    fn put_xp_summary(&self, actor: &Actor, summary: &XpSummaryRecord) -> Result<(), StoreError> {
        let summary = summary.clone();
        self.write(actor, move |conn| {
            let ttl = summary.created_at.0.saturating_add_unsigned(XP_SUMMARY_TTL_MS);
            let query = portable::upsert(
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
            );
            diesel::sql_query(query)
                .bind::<Text, _>(summary.session.to_string())
                .bind::<Text, _>(summary.member.0)
                .bind::<Text, _>(summary.host.0)
                .bind::<Nullable<Text>, _>(summary.realm.map(|r| r.to_string()))
                .bind::<BigInt, _>(i64::from(summary.minutes))
                .bind::<Text, _>(summary.grants.0)
                .bind::<Integer, _>(i32::from(summary.flagged))
                .bind::<BigInt, _>(summary.created_at.0)
                .bind::<BigInt, _>(ttl)
                .execute(conn)?;
            Ok(())
        })
    }

    fn insert_alert(&self, actor: &Actor, alert: &AlertRecord) -> Result<(), StoreError> {
        let alert = alert.clone();
        self.write(actor, move |conn| {
            let query = portable::insert_ignore(
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
            );
            diesel::sql_query(query)
                .bind::<Text, _>(alert.alert_id)
                .bind::<Text, _>(alert.code)
                .bind::<Text, _>(alert.severity)
                .bind::<Text, _>(alert.message)
                .bind::<Text, _>(alert.details.0)
                .bind::<BigInt, _>(alert.created_at.0)
                .bind::<Nullable<BigInt>, _>(alert.acknowledged_at.map(|at| at.0))
                .bind::<Nullable<Text>, _>(alert.acknowledged_by)
                .execute(conn)?;
            Ok(())
        })
    }

    fn acknowledge_alert(&self, actor: &Actor, alert_id: &str, at: UnixMillis) -> Result<bool, StoreError> {
        let (alert_id, by) = (alert_id.to_owned(), actor_name(actor));
        self.write(actor, move |conn| {
            let query = sql(
                "UPDATE rifts_operations_alerts SET acknowledged_at = ?, acknowledged_by = ? \
                 WHERE alert_id = ? AND acknowledged_at IS NULL",
            )?;
            let changed = diesel::sql_query(query)
                .bind::<BigInt, _>(at.0)
                .bind::<Text, _>(by)
                .bind::<Text, _>(alert_id)
                .execute(conn)?;
            Ok(changed == 1)
        })
    }
}
