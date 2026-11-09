// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The event recorder (module sharing A5, D2): every event on the registry's bus, kept in `store`'s `events`
//! table on the engine `[database]` picks, so history survives a restart and the admin API can page through it.
//!
//! The bus is a lossy `broadcast`. The recorder subscribes when it is composed, before any module loads, and
//! writes an `events.lagged` row with the count when it falls behind, so a gap shows. Rows go through
//! [`Database::write`] and so into the change log like every table's (D2). `[events] retention_days` deletes
//! rows by `at_ms` once a day.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dainty_module_api::{Event, ModuleId};
use dainty_service_kit::context::ServiceContext;
use dainty_store::{Actor, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema};
use diesel::prelude::*;
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use serde::Serialize;
use tokio::sync::broadcast::{self, error::RecvError, error::TryRecvError};

use crate::registry::ModuleRegistry;

const SQLITE: EmbeddedMigrations = embed_migrations!("migrations/sqlite");
const POSTGRES: EmbeddedMigrations = embed_migrations!("migrations/postgres");

/// The events table's migrations for each engine, for `Database::open`.
pub const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE,
    postgres: || POSTGRES,
    soft_delete: &[],
};

/// The SQLite file's name beside the node's database.
pub const FILE_NAME: &str = "events.db";
/// The row written when the recorder fell behind the bus; its payload is `{"missed": n}`.
pub const LAGGED_EVENT: &str = "events.lagged";
/// `[events] retention_days` when host.toml leaves it out.
pub const DEFAULT_RETENTION_DAYS: u32 = 90;
/// The most rows [`EventRecorder::list`] returns.
pub const MAX_LIST: i64 = 1000;
/// Events written in one transaction when the bus has more waiting.
const BATCH: usize = 256;
const SWEEP_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

diesel::table! {
    /// The registry's events, oldest first.
    events (seq) {
        /// Row number; never reused.
        seq -> BigInt,
        /// Unix milliseconds the recorder received it.
        at_ms -> BigInt,
        /// The event's name, such as `game.server.down`.
        name -> Text,
        /// The module that emitted it; null for the host.
        source -> Nullable<Text>,
        /// The event's JSON.
        payload -> Text,
    }
}

/// One kept event.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventEntry {
    /// Row number, for paging with `after`.
    pub seq: i64,
    /// Unix milliseconds.
    pub at_ms: i64,
    /// The event's name.
    pub name: String,
    /// The emitting module's id; `None` for the host.
    pub source: Option<String>,
    /// The event's data.
    pub payload: serde_json::Value,
}

/// How the recorder keeps events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventsOptions {
    /// Rows older than this are deleted daily; `None` keeps them all.
    pub retention: Option<Duration>,
}

impl EventsOptions {
    /// `retention_days` as host.toml gives it: 0 keeps every row.
    #[must_use]
    pub fn from_days(days: u32) -> Self {
        Self {
            retention: (days > 0).then(|| Duration::from_secs(u64::from(days) * 24 * 60 * 60)),
        }
    }
}

impl Default for EventsOptions {
    fn default() -> Self {
        Self::from_days(DEFAULT_RETENTION_DAYS)
    }
}

struct Row {
    at_ms: i64,
    name: String,
    source: Option<String>,
    payload: String,
}

impl Row {
    fn event(from: Option<ModuleId>, event: &Event) -> Self {
        Self {
            at_ms: now_ms(),
            name: event.name.as_str().to_owned(),
            source: from.map(|id| id.to_string()),
            payload: event.payload.to_string(),
        }
    }

    fn lagged(missed: u64) -> Self {
        Self {
            at_ms: now_ms(),
            name: LAGGED_EVENT.to_owned(),
            source: None,
            payload: serde_json::json!({ "missed": missed }).to_string(),
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The events database.
#[derive(Clone)]
pub struct EventRecorder {
    db: Database,
}

impl EventRecorder {
    /// Opens (or creates) the events database and brings its schema up to date. Blocking.
    pub fn open(config: &DatabaseConfig) -> Result<Self, String> {
        if let DatabaseConfig::Sqlite { file } = config
            && let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty())
        {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let db = Database::open(config, &DatabaseOptions::default(), &SCHEMA)
            .map_err(|e| format!("the events database: {e}"))?;
        Ok(Self { db })
    }

    /// `events.db` beside the node's SQLite file, or the node's Postgres database (as admin's `history::sources`).
    #[must_use]
    pub fn database_for(node: &DatabaseConfig) -> DatabaseConfig {
        match node {
            DatabaseConfig::Sqlite { file } => DatabaseConfig::Sqlite {
                file: file.with_file_name(FILE_NAME),
            },
            other => other.clone(),
        }
    }

    async fn record(&self, rows: Vec<Row>) -> Result<(), dainty_store::Error> {
        if rows.is_empty() {
            return Ok(());
        }
        self.db
            .write(Actor::System("events".into()), move |tx| {
                for row in rows {
                    diesel::insert_into(events::table)
                        .values((
                            events::at_ms.eq(row.at_ms),
                            events::name.eq(row.name),
                            events::source.eq(row.source),
                            events::payload.eq(row.payload),
                        ))
                        .execute(tx.conn())?;
                }
                Ok(())
            })
            .await
    }

    /// The events after `after`, oldest first, only those named in `names` when it isn't empty; at most
    /// `limit` (1 to [`MAX_LIST`]).
    pub async fn list(&self, after: i64, names: &[String], limit: i64) -> Result<Vec<EventEntry>, dainty_store::Error> {
        let names = names.to_vec();
        let limit = limit.clamp(1, MAX_LIST);
        let rows: Vec<(i64, i64, String, Option<String>, String)> = self
            .db
            .read(move |conn| {
                let mut query = events::table
                    .select((
                        events::seq,
                        events::at_ms,
                        events::name,
                        events::source,
                        events::payload,
                    ))
                    .filter(events::seq.gt(after))
                    .order(events::seq.asc())
                    .limit(limit)
                    .into_boxed();
                if !names.is_empty() {
                    query = query.filter(events::name.eq_any(names));
                }
                query.load(conn).map_err(dainty_store::Error::from)
            })
            .await?;
        Ok(rows
            .into_iter()
            .map(|(seq, at_ms, name, source, payload)| EventEntry {
                seq,
                at_ms,
                name,
                source,
                payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::String(payload)),
            })
            .collect())
    }

    /// Deletes the rows received before `cutoff_ms`; returns how many.
    pub async fn purge_before(&self, cutoff_ms: i64) -> Result<usize, dainty_store::Error> {
        self.db
            .write(Actor::System("retention".into()), move |tx| {
                Ok(diesel::delete(events::table.filter(events::at_ms.lt(cutoff_ms))).execute(tx.conn())?)
            })
            .await
    }

    async fn sweep(&self, retention: Option<Duration>) {
        let Some(keep) = retention else { return };
        let keep_ms = i64::try_from(keep.as_millis()).unwrap_or(i64::MAX);
        match self.purge_before(now_ms().saturating_sub(keep_ms)).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(deleted = n, days = keep_ms / DAY_MS, "events past retention deleted"),
            Err(e) => tracing::warn!(error = %e, "the events retention sweep failed"),
        }
    }

    async fn write(&self, rows: Vec<Row>) {
        let count = rows.len();
        if let Err(e) = self.record(rows).await {
            tracing::warn!(error = %e, events = count, "events not recorded");
        }
    }

    /// Records from `rx` until `stop` turns true, then writes what is still queued.
    async fn run(
        &self,
        mut rx: broadcast::Receiver<(Option<ModuleId>, Event)>,
        options: EventsOptions,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) {
        let mut sweep = tokio::time::interval(SWEEP_EVERY);
        sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut open = true;
        while !*stop.borrow() {
            tokio::select! {
                _ = stop.changed() => break,
                _ = sweep.tick() => self.sweep(options.retention).await,
                got = rx.recv(), if open => match got {
                    Ok((from, event)) => {
                        let mut rows = vec![Row::event(from, &event)];
                        open = drain(&mut rx, &mut rows);
                        self.write(rows).await;
                    }
                    Err(RecvError::Lagged(missed)) => self.write(vec![Row::lagged(missed)]).await,
                    Err(RecvError::Closed) => open = false,
                },
            }
        }
        while open {
            let mut rows = Vec::new();
            open = drain(&mut rx, &mut rows) && rows.len() == BATCH;
            self.write(rows).await;
        }
    }
}

/// Moves what the bus already holds into `rows`, up to [`BATCH`]; false once the bus is closed.
fn drain(rx: &mut broadcast::Receiver<(Option<ModuleId>, Event)>, rows: &mut Vec<Row>) -> bool {
    let start = rows.len();
    while rows.len() - start < BATCH {
        match rx.try_recv() {
            Ok((from, event)) => rows.push(Row::event(from, &event)),
            Err(TryRecvError::Lagged(missed)) => rows.push(Row::lagged(missed)),
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Closed) => return false,
        }
    }
    true
}

/// The `events` module: subscribes to `registry` now, so nothing a module emits while loading is missed, and
/// records into `database` once started. Optional: the server runs on without it.
pub fn module(registry: &ModuleRegistry, database: DatabaseConfig, options: EventsOptions) -> crate::Module {
    let first = Arc::new(Mutex::new(Some(registry.subscribe_events())));
    let registry = registry.downgrade();
    let database = Arc::new(database);
    crate::Module::new(
        "events",
        Arc::new(move |ctx: ServiceContext| {
            let first = Arc::clone(&first);
            let registry = registry.clone();
            let database = Arc::clone(&database);
            Box::pin(async move {
                // A restart subscribes again; the gap between the two shows as nothing, as the stop did
                let rx = match first.lock().unwrap_or_else(PoisonError::into_inner).take() {
                    Some(rx) => rx,
                    None => registry
                        .upgrade()
                        .ok_or_else(|| String::from("the module registry is gone"))?
                        .subscribe_events(),
                };
                let config = (*database).clone();
                let recorder = tokio::task::spawn_blocking(move || EventRecorder::open(&config))
                    .await
                    .map_err(|e| format!("the events database: {e}"))??;
                let detail = match options.retention {
                    Some(keep) => format!("kept {} days", keep.as_secs() / (24 * 60 * 60)),
                    None => "kept for good".to_owned(),
                };
                ctx.status.report_ready(None, detail);
                recorder.run(rx, options, ctx.stop_receiver()).await;
                ctx.status.stopped();
                Ok(())
            })
        }),
    )
    .optional()
    .ready_within(Duration::from_secs(30))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dainty_module_api::EventName;

    fn event(name: &str, n: u64) -> Event {
        Event {
            name: EventName::new(name).unwrap(),
            payload: serde_json::json!({ "n": n }),
        }
    }

    fn open(dir: &tempfile::TempDir) -> EventRecorder {
        let node = DatabaseConfig::Sqlite {
            file: dir.path().join("dainty.db"),
        };
        let config = EventRecorder::database_for(&node);
        assert_eq!(
            config,
            DatabaseConfig::Sqlite {
                file: dir.path().join(FILE_NAME)
            }
        );
        EventRecorder::open(&config).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn records_every_event_and_a_gap_when_behind() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = open(&dir);
        let (tx, rx) = broadcast::channel(4);
        // Six into a bus four deep: the first two are lost before the recorder reads
        for n in 0..6 {
            tx.send((Some("isles-1-14-7".parse().unwrap()), event("game.server.up", n)))
                .unwrap();
        }
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let run = {
            let recorder = recorder.clone();
            tokio::spawn(async move { recorder.run(rx, EventsOptions::default(), stop_rx).await })
        };
        tokio::time::sleep(Duration::from_millis(200)).await;
        stop_tx.send(true).unwrap();
        run.await.unwrap();

        let all = recorder.list(0, &[], 100).await.unwrap();
        let names: Vec<&str> = all.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names[0], LAGGED_EVENT);
        assert_eq!(all[0].payload, serde_json::json!({ "missed": 2 }));
        assert_eq!(all[0].source, None);
        assert_eq!(names[1..].len(), 4);
        assert_eq!(all[1].source.as_deref(), Some("isles-1-14-7"));
        assert_eq!(all[1].payload, serde_json::json!({ "n": 2 }));

        let up = recorder.list(all[1].seq, &["game.server.up".into()], 2).await.unwrap();
        assert_eq!(
            up.iter().map(|e| e.payload["n"].as_u64()).collect::<Vec<_>>(),
            [Some(3), Some(4)]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn writes_what_is_queued_when_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = open(&dir);
        let (tx, rx) = broadcast::channel(1024);
        let (_stop_tx, stop_rx) = tokio::sync::watch::channel(true);
        for n in 0..300 {
            tx.send((None, event("server.ready", n))).unwrap();
        }
        recorder.run(rx, EventsOptions::default(), stop_rx).await;
        assert_eq!(recorder.list(0, &[], MAX_LIST).await.unwrap().len(), 300);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn retention_deletes_by_age_and_never_reuses_a_seq() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = open(&dir);
        recorder
            .record(vec![
                Row::event(None, &event("a.b", 1)),
                Row::event(None, &event("a.b", 2)),
            ])
            .await
            .unwrap();
        assert_eq!(recorder.purge_before(now_ms() + 1).await.unwrap(), 2);
        recorder.record(vec![Row::event(None, &event("a.b", 3))]).await.unwrap();
        let left = recorder.list(0, &[], 10).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].seq, 3);
        assert_eq!(EventsOptions::from_days(0).retention, None);
        assert_eq!(
            EventsOptions::default().retention,
            Some(Duration::from_secs(90 * 86_400))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_module_records_what_the_registry_carries() {
        let dir = tempfile::tempdir().unwrap();
        let status = dainty_service_kit::context::StatusRegistry::new("private", Vec::new());
        let registry = ModuleRegistry::new(crate::RegistryOptions::standalone(), status.clone());
        let node = DatabaseConfig::Sqlite {
            file: dir.path().join("dainty.db"),
        };
        let module = module(&registry, EventRecorder::database_for(&node), EventsOptions::default());
        // Emitted before the module starts: kept, as the subscription is the composer's
        registry.emit(event("server.ready", 1));
        let (ctx, stop) = status.service_context("events");
        let run = tokio::spawn((module.start)(ctx));
        tokio::time::sleep(Duration::from_millis(500)).await;
        registry.emit(event("server.ready", 2));
        tokio::time::sleep(Duration::from_millis(200)).await;
        stop.send(true).unwrap();
        run.await.unwrap().unwrap();

        let recorder = EventRecorder::open(&EventRecorder::database_for(&node)).unwrap();
        let kept = recorder.list(0, &[], 10).await.unwrap();
        assert_eq!(
            kept.iter().map(|e| e.payload["n"].as_u64()).collect::<Vec<_>>(),
            [Some(1), Some(2)]
        );
        // The change log holds every row too (D2)
        assert!(recorder.db.last_change().await.unwrap().is_some());
    }
}
