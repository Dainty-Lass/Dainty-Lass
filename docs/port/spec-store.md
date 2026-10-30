# Clean-room Behaviour Specification: `store` (`crates/store` SQLite / Drizzle Layer)

## 1. Purpose and Scope

`store` (`rust/crates/store`) provides the database persistence layer for DaintyLass services. Thirteen of its nineteen source files (`connection.rs`, `database.rs`, `declared.rs`, `engine.rs`, `history.rs`, `portable.rs`, `recovery.rs`, `revert.rs`, `transfer.rs`, and submodules) were authored from scratch for DaintyLass (`DD15`, `DD23`, `DH3`, `DR4`) and remain untouched.

This specification covers the six files that replace the legacy SQLite / Drizzle migration layer and its integration test suite so the entire `store` crate is licensed under MIT:
- `src/lib.rs`
- `src/adopt.rs`
- `src/backup.rs`
- `src/drizzle.rs`
- `src/readonly.rs`
- `src/snapshot.rs`
- `tests/store.rs`

- **Package name**: `store`
- **Edition**: `2024`
- **Licence**: `MIT` (`Copyright (c) 2026 Silent Spud`)

---

## 2. Crate Root (`src/lib.rs`)

### Module Declarations and Re-exports

```rust
pub mod adopt;
pub mod backup;
mod connection;
mod database;
mod declared;
pub mod drizzle;
mod engine;
pub mod history;
pub mod portable;
pub mod readonly;
pub mod recovery;
pub mod revert;
mod snapshot;
pub mod transfer;

pub use adopt::{DrizzleBaseline, Schema};
pub use connection::{AnyConnection, MultiBackend};
pub use database::{Database, DatabaseOptions, DatabaseSchema, KeyValue, Tx};
pub use declared::{declare_accounts, declared_accounts};
pub use engine::{DatabaseConfig, Engine, PostgresConfig, is_local_address};
pub use history::{Actor, ChangeEntry, Head};
pub use readonly::{open_read_only, schema_version};
pub use revert::{AccountColumn, Cursor, Filter, RevertOutcome, RevertPlan};
pub use snapshot::schema_snapshot;

pub type Connection = diesel::SqliteConnection;
```

### Public Types and Functions (`src/lib.rs`)

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Query(#[from] diesel::result::Error),
    #[error("database connection: {0}")]
    Connection(#[from] diesel::ConnectionError),
    #[error("database pool: {0}")]
    Pool(#[from] diesel::r2d2::PoolError),
    #[error("migration: {0}")]
    Migration(String),
    #[error("the database can't be adopted: {0}")]
    Adoption(String),
    #[error("backup: {0}")]
    Backup(String),
    #[error("database settings: {0}")]
    Config(String),
    #[error("schema: {0}")]
    Schema(String),
    #[error("not an actor the change log can record: {0:?}")]
    Actor(String),
    #[error("db move: {0}")]
    Move(String),
    #[error("revert: {0}")]
    Revert(String),
    #[error("the history changed since the dry run: run it again")]
    PlanChanged,
    #[error("restore: {0}")]
    Restore(String),
    #[error("the database task panicked")]
    Panicked,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub read_connections: u32,
    pub busy_timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            read_connections: 4,
            busy_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Clone)]
pub struct Store {
    pub(crate) path: Arc<PathBuf>,
    pub(crate) busy_timeout: Duration,
    /* private pool fields */
}

impl Store {
    pub fn open(path: &Path, options: &Options, schema: &Schema) -> Result<Self, Error>;

    pub async fn read<T, E, F>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<Error> + From<diesel::result::Error> + Send + 'static;

    pub async fn write<T, E, F>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<Error> + From<diesel::result::Error> + Send + 'static;

    pub async fn checkpoint(&self) -> Result<(), Error>;
}

pub fn migrate(conn: &mut Connection, schema: &Schema) -> Result<(), Error>;

pub(crate) async fn run_blocking<T, E, F>(f: F) -> Result<T, E>
where
    F: FnOnce() -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: From<Error> + Send + 'static;
```

### Observable Behaviour (`src/lib.rs`)

1. **Connection pool setup (`Store::open`)**:
   - Builds a Diesel `r2d2::Pool<ConnectionManager<SqliteConnection>>` for writers (`max_size = 1`, `min_idle = Some(0)`, `test_on_check_out = false`) and a pool for readers (`max_size = options.read_connections.max(1)`).
   - Every acquired connection runs a customiser setting:
     - `PRAGMA busy_timeout = <busy_timeout_ms>;`
     - `PRAGMA foreign_keys = ON;`
     - On reader pool connections only: `PRAGMA query_only = ON;`.
   - Checks out the write connection, executes `PRAGMA journal_mode = WAL`, verifies the returned `journal_mode` equals `"wal"` case-insensitively (otherwise returns `Err(Error::Migration(...))`), and runs `adopt::prepare(&mut conn, schema)?`.

2. **`Store::read`, `Store::write`, `Store::checkpoint`, `migrate`, `run_blocking`**:
   - `Store::read` checks out a reader connection on `run_blocking` and runs `conn.transaction(f)` (deferred transaction; writes fail because `query_only = ON`).
   - `Store::write` checks out the single writer connection on `run_blocking` and runs `conn.immediate_transaction(f)` (`BEGIN IMMEDIATE`).
   - `Store::checkpoint` runs `PRAGMA wal_checkpoint(TRUNCATE);` on the writer connection via `run_blocking`.
   - `migrate(conn, schema)` delegates to `adopt::prepare(conn, schema)` without changing `journal_mode`.
   - `run_blocking(f)` spawns `f` on `tokio::task::spawn_blocking` and maps a panic (`JoinError`) to `Err(E::from(Error::Panicked))`.

---

## 3. Module `adopt` (`src/adopt.rs`)

Brings a SQLite database to the current Diesel schema, adopting existing Drizzle databases at the baseline when applicable.

### Public Signatures

```rust
use diesel_migrations::EmbeddedMigrations;

pub struct Schema {
    pub migrations: fn() -> EmbeddedMigrations,
    pub drizzle: Option<DrizzleBaseline>,
}

pub struct DrizzleBaseline {
    pub created_at: i64,
    pub sql: &'static str,
    pub snapshot: &'static str,
}

pub(crate) fn prepare(conn: &mut Connection, schema: &Schema) -> Result<(), Error>;
```

### Observable Behaviour

- `prepare(conn, schema)`:
  1. If table `__diesel_schema_migrations` exists in `sqlite_schema`, runs pending migrations from `(schema.migrations)()` via `MigrationHarness::run_pending_migrations` (mapping errors to `Error::Migration`).
  2. Else if table `__drizzle_migrations` exists in `sqlite_schema`:
     - Requires `schema.drizzle` to be `Some(baseline)` (otherwise returns `Err(Error::Adoption(...))`).
     - Inside `conn.immediate_transaction`:
       - Calls `drizzle::last_applied(conn)?`; if `None`, returns `Err(Error::Adoption("__drizzle_migrations is empty".into()))`.
       - Verifies `created_at == Some(baseline.created_at)` and `drizzle::hash_matches(&hash, baseline.sql)` (otherwise returns `Err(Error::Adoption(...))`).
       - Verifies `schema_snapshot(conn)? == baseline.snapshot.replace("\r\n", "\n")` (otherwise returns `Err(Error::Adoption("its schema differs from the baseline's".into()))`).
       - Obtains the first migration version from `MigrationSource::<Sqlite>::migrations(&(schema.migrations)())`, ensures `__diesel_schema_migrations` exists (`conn.applied_migrations()`), and inserts `version` into `__diesel_schema_migrations`.
     - Runs any remaining pending Diesel migrations after the baseline.
  3. Else if the database has zero non-`sqlite_%` objects in `sqlite_schema`, runs all pending Diesel migrations from `(schema.migrations)()`.
  4. Otherwise (tables exist with neither `__diesel_schema_migrations` nor `__drizzle_migrations`), returns `Err(Error::Adoption("it has tables but no migration history".into()))`.

---

## 4. Module `backup` (`src/backup.rs`)

Creates a consistent single-file hot copy of a live SQLite database using `rusqlite::backup::Backup`.

### Public Signatures

```rust
impl Store {
    pub async fn backup(&self, destination: &Path, deadline: Duration) -> Result<(), Error>;
}

pub fn backup_file(
    source: &Path,
    destination: &Path,
    busy_timeout: Duration,
    deadline: Duration,
) -> Result<(), Error>;
```

### Observable Behaviour

- `backup_file(source, destination, busy_timeout, deadline)`:
  - If `!source.is_file()`, returns `Err(Error::Backup(format!("no database at {}", source.display())))`.
  - If `destination.exists()`, returns `Err(Error::Backup(format!("{} exists already", destination.display())))`.
  - Opens `source` with `rusqlite` in read-only mode (`SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`), sets `busy_timeout`, and starts a read transaction (`BEGIN; SELECT count(*) FROM sqlite_schema;`) so the backup copies a single consistent snapshot without restarting when concurrent WAL writers commit.
  - Opens `destination` with `rusqlite::Connection::open` and steps `rusqlite::backup::Backup::new(&src, &mut dst)` (e.g. 4096 pages per step, sleeping briefly on busy/locked) until `StepResult::Done` or `Instant::now() >= start + deadline` (which fails with `Error::Backup("the backup deadline passed".into())`).
  - Commits the read transaction on `src` and sets `PRAGMA journal_mode = DELETE` on `dst` so the backup is a self-contained single file.
  - If any step fails, removes any partial file at `destination` before returning `Err(Error::Backup(...))`.

---

## 5. Module `drizzle` (`src/drizzle.rs`)

Reads Drizzle Kit migration folders (`meta/_journal.json` and `<tag>.sql`), computes SHA-256 hashes, inspects `__drizzle_migrations`, and replays unapplied Drizzle migrations.

### Public Signatures

```rust
#[derive(Debug, Clone)]
pub struct Migration {
    pub tag: String,
    pub created_at: i64,
    pub sql: String,
}

pub fn read_folder(dir: &Path) -> Result<Vec<Migration>, Error>;
pub fn hash(sql: &str) -> String;
pub fn hash_matches(recorded: &str, sql: &str) -> bool;
pub fn last_applied(conn: &mut Connection) -> Result<Option<(Option<i64>, String)>, Error>;
pub fn replay(conn: &mut Connection, migrations: &[Migration]) -> Result<(), Error>;
```

### Observable Behaviour

- `read_folder(dir)` parses `dir/meta/_journal.json` (`{ "entries": [{ "tag": "...", "when": 123 }] }`) and reads `dir/<tag>.sql` for each entry in journal order, mapping I/O or JSON errors to `Error::Migration`.
- `hash(sql)` returns the 64-character lowercase hex SHA-256 digest of `sql.as_bytes()`.
- `hash_matches(recorded, sql)` returns `true` if `recorded` equals `hash` of `sql` with LF line endings (`\r\n` -> `\n`) or CRLF line endings (`\n` -> `\r\n`).
- `last_applied(conn)` queries `SELECT created_at, hash FROM __drizzle_migrations ORDER BY created_at DESC LIMIT 1` and returns `Ok(Option<(Option<i64>, String)>)`.
- `replay(conn, migrations)` creates `"__drizzle_migrations" (id SERIAL PRIMARY KEY, hash text NOT NULL, created_at numeric)` if absent, checks `last_applied(conn)`, and in an `immediate_transaction` executes every migration with `m.created_at > last` (splitting `m.sql` on `"--> statement-breakpoint"`, executing non-empty statements via `batch_execute`, and inserting `(hash(&m.sql), m.created_at)` into `"__drizzle_migrations"`).

---

## 6. Module `readonly` (`src/readonly.rs`)

Opens an existing SQLite database in read-only mode (`mode=ro` URI + `PRAGMA query_only = ON`) without creating missing files or taking write locks.

### Public Signatures

```rust
pub fn open_read_only(path: &Path, busy_timeout: Duration) -> Result<Connection, Error>;
pub fn schema_version(conn: &mut Connection) -> Result<Option<String>, Error>;
```

### Observable Behaviour

- `open_read_only(path, busy_timeout)` resolves `std::path::absolute(path)`, percent-encodes characters outside `[A-Za-z0-9-._~/:]`, formats a `file://...` (`//` on Unix, `///` on Windows drive paths) URI with `?mode=ro`, establishes a `SqliteConnection`, and executes `PRAGMA busy_timeout = <ms>; PRAGMA query_only = ON;`.
- `schema_version(conn)` checks whether `__diesel_schema_migrations` exists in `sqlite_master`; if not, returns `Ok(None)`. Otherwise returns `SELECT version FROM __diesel_schema_migrations ORDER BY version DESC LIMIT 1` (`Ok(Option<String>)`).

---

## 7. Module `snapshot` (`src/snapshot.rs`)

Returns a canonical SQL text representation of a SQLite database's schema.

### Public Signatures

```rust
pub fn schema_snapshot(conn: &mut Connection) -> Result<String, Error>;
```

### Observable Behaviour

- Queries `sqlite_schema`:
  ```sql
  SELECT sql FROM sqlite_schema
  WHERE sql IS NOT NULL
    AND name NOT LIKE 'sqlite!_%' ESCAPE '!'
    AND tbl_name NOT IN ('__drizzle_migrations', '__diesel_schema_migrations')
  ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1 WHEN 'view' THEN 2 ELSE 3 END, name
  ```
- Normalises CRLF to LF (`\r\n` -> `\n`) and formats each row as `"{sql};\n"`, concatenated in order.

---

## 8. Test Expectations (`tests/store.rs`)

The implementation must pass:
1. All unit tests in `drizzle` and `readonly`.
2. All integration tests in `tests/store.rs`:
   - `writes_commit_and_errors_roll_back`
   - `reads_cannot_write`
   - `the_database_is_in_wal_mode`
   - `sqlite_behaves_like_better_sqlite3`
   - `a_dropped_request_still_finishes_its_transaction`
   - `a_panicking_closure_rolls_back_and_the_store_keeps_working`
   - `a_database_with_unknown_tables_is_refused`
   - `a_backup_under_constant_writes_is_one_consistent_snapshot`
   - `a_read_only_connection_reads_but_refuses_writes`
   - `the_schema_version_is_readable_while_a_write_is_held`
3. All tests in `tests/database.rs` and `tests/revert.rs` (`cargo test -p store`).
