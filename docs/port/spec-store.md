# Clean-room Behaviour Specification: `store` (`crates/store` SQLite / Drizzle Layer)

## 1. Purpose and Scope

`store` (`rust/crates/store`) provides the database persistence layer for DaintyLass services. Thirteen of its nineteen source files (`connection.rs`, `database.rs`, `declared.rs`, `engine.rs`, `history.rs`, `portable.rs`, `recovery.rs`, `revert.rs`, `transfer.rs`, and submodules) were authored from scratch for DaintyLass (`DD15`, `DD23`, `DH3`, `DR4`) and remain untouched.

This specification defines only the public API and observable behaviour of the six source files and one integration test file that provide single-engine SQLite connection pooling, Drizzle-to-Diesel baseline adoption, online SQLite backups, read-only SQLite inspection, and schema snapshots:
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

`src/lib.rs` declares the crate's modules and re-exports the public items below so downstream crates can import them from the crate root:

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

### Public Types and Signatures

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
pub struct Store { /* opaque internal state */ }

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
```

### Observable Behaviour

- **`Store::open(path, options, schema)`**:
  - Opens a SQLite database at `path` with a single writer connection and up to `options.read_connections` (at least 1) concurrent reader connections.
  - Every connection enforces foreign-key constraints and waits up to `options.busy_timeout` when encountering a lock.
  - Reader connections are restricted to read-only queries so any attempt to mutate data inside `Store::read` fails with a SQLite error.
  - Enables Write-Ahead Logging (`WAL` journal mode) on the database (returning `Err(Error::Migration(...))` if SQLite does not switch to `wal` mode) and applies `schema` (same migration/adoption rules as `migrate`).
- **`Store::read(f)`**:
  - Executes `f` off the async executor thread inside a read transaction. Commits if `f` returns `Ok`, rolls back if `f` returns `Err` or panics, and still completes the closure if the awaiting future is dropped mid-flight. Maps a task panic to `Err(E::from(Error::Panicked))`.
- **`Store::write(f)`**:
  - Executes `f` off the async executor thread inside an immediate write transaction (`BEGIN IMMEDIATE`) on the writer connection. Commits on `Ok`, rolls back on `Err` or panic (without poisoning the writer connection for subsequent calls), and still completes if the awaiting future is dropped. Maps a task panic to `Err(E::from(Error::Panicked))`.
- **`Store::checkpoint()`**:
  - Truncates the WAL back into the main database file (`TRUNCATE` checkpoint).
- **`migrate(conn, schema)`**:
  - Brings `conn` up to date with `schema` without changing the connection's journal mode.

---

## 3. Module `adopt` (`src/adopt.rs`)

### Public Types

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
```

### Observable Behaviour

When preparing a connection for `schema` (via `Store::open` or `migrate`):
1. **Existing Diesel database**: If the `__diesel_schema_migrations` table already exists, runs any unapplied migrations from `(schema.migrations)()` (mapping migration errors to `Error::Migration`).
2. **Existing Drizzle database**: If `__diesel_schema_migrations` is absent and `__drizzle_migrations` exists:
   - If `schema.drizzle` is `None`, refuses with `Err(Error::Adoption(...))`.
   - If `schema.drizzle` is `Some(baseline)`:
     - Inspects the most recent entry in `__drizzle_migrations` (`drizzle::last_applied`). Refuses with `Err(Error::Adoption(...))` if the table is empty, if the last entry's `created_at` does not equal `Some(baseline.created_at)`, or if its recorded hash does not match `baseline.sql` (`drizzle::hash_matches`).
     - Compares `schema_snapshot(conn)` against `baseline.snapshot` (normalising CRLF to LF); if they differ, refuses with `Err(Error::Adoption(...))` and leaves the database untouched.
     - Records the first migration from `(schema.migrations)()` as already applied in `__diesel_schema_migrations` without re-executing its SQL, then runs any subsequent Diesel migrations after the baseline.
3. **Fresh empty database**: If the database contains no user tables/views/indexes/triggers, runs all migrations from `(schema.migrations)()`.
4. **Unrecognised non-empty database**: If user objects exist without either `__diesel_schema_migrations` or `__drizzle_migrations`, refuses with `Err(Error::Adoption(...))`.

---

## 4. Module `backup` (`src/backup.rs`)

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

- `Store::backup(destination, deadline)` performs an online backup of the store's database file to `destination` within `deadline`.
- `backup_file(source, destination, busy_timeout, deadline)`:
  - Returns `Err(Error::Backup(...))` if `source` is not an existing regular file or if `destination` already exists.
  - Copies a consistent point-in-time snapshot of `source` into `destination` using SQLite's online backup API while holding a read snapshot on `source` so concurrent WAL writes do not restart the copy from page 1.
  - If `deadline` expires before the copy completes, aborts and returns `Err(Error::Backup(...))`.
  - Ensures the completed `destination` file is a self-contained single-file database (classic `DELETE` journal mode, no leftover `-wal` or `-shm` sidecars).
  - If any error occurs after `destination` is created, deletes the partial `destination` file before returning `Err(Error::Backup(...))`.

---

## 5. Module `drizzle` (`src/drizzle.rs`)

### Public Types and Signatures

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

- **`read_folder(dir)`**: Reads Drizzle Kit's `meta/_journal.json` (which contains an `entries` array of objects with string `tag` and integer `when` fields) inside `dir`, loads `<tag>.sql` from `dir` for each entry in journal order, and returns the resulting `Vec<Migration>` (`created_at = when`). Returns `Err(Error::Migration(...))` on I/O or JSON parse failure.
- **`hash(sql)`**: Returns the 64-character lowercase hexadecimal SHA-256 digest of `sql`.
- **`hash_matches(recorded, sql)`**: Returns `true` if `recorded` matches the SHA-256 hex digest of `sql` with LF line endings or with CRLF line endings (so Windows git checkouts with `core.autocrlf` still match).
- **`last_applied(conn)`**: Returns `Ok(None)` if `__drizzle_migrations` has no rows, or `Ok(Some((created_at, hash)))` for the row with the largest `created_at`.
- **`replay(conn, migrations)`**: Ensures the `__drizzle_migrations` table exists (`id`, `hash`, `created_at`), checks the highest `created_at` already recorded, and inside a single transaction executes each migration whose `created_at` is strictly greater than that timestamp (splitting multi-statement files on Drizzle's `--> statement-breakpoint` marker) and records `(hash(&m.sql), m.created_at)` in `__drizzle_migrations`.

---

## 6. Module `readonly` (`src/readonly.rs`)

### Public Signatures

```rust
pub fn open_read_only(path: &Path, busy_timeout: Duration) -> Result<Connection, Error>;
pub fn schema_version(conn: &mut Connection) -> Result<Option<String>, Error>;
```

### Observable Behaviour

- **`open_read_only(path, busy_timeout)`**:
  - Opens an existing SQLite database at `path` in read-only mode (`mode=ro`) with the given `busy_timeout` and read-only query enforcement.
  - Never creates a file if `path` does not exist (returns `Err(Error::Connection(...))`), handles paths containing spaces or special characters (such as `#` or `?`), and rejects write statements on the returned connection.
- **`schema_version(conn)`**:
  - Returns `Ok(None)` if the database has no `__diesel_schema_migrations` table or if that table is empty; otherwise returns `Ok(Some(version))` with the highest applied Diesel migration version string.

---

## 7. Module `snapshot` (`src/snapshot.rs`)

### Public Signature

```rust
pub fn schema_snapshot(conn: &mut Connection) -> Result<String, Error>;
```

### Observable Behaviour

- Inspects the SQLite schema catalog and returns a deterministic text snapshot of all user-defined schema objects that have a SQL definition, excluding:
  - SQLite internal objects whose names begin with `sqlite_`, and
  - Migration tracking tables (`__drizzle_migrations` and `__diesel_schema_migrations`) and any objects attached to them.
- Objects are sorted by object type in the order:
  1. `table`
  2. `index`
  3. `view`
  4. any other object type (e.g. `trigger`)
  and alphabetically by object `name` within each type.
- Each object's `CREATE ...` statement has CRLF line endings normalized to LF (`\n`) and is followed by `;\n`.

---

## 8. Verification (`tests/store.rs`)

`tests/store.rs` must verify the observable contracts above across:
- committing writes, rolling back errors, and rejecting writes inside `Store::read`;
- WAL journal mode, foreign-key enforcement, and `busy_timeout` waiting;
- completing in-flight transactions when the caller's future is dropped;
- recovering from a panicking write closure without losing subsequent store usability;
- adopting a Drizzle database at the baseline, running subsequent Diesel migrations, and refusing databases with unknown tables, empty Drizzle history, wrong Drizzle migration hash/timestamp, or modified schema;
- producing a consistent single-file backup under concurrent writes and honouring the backup deadline;
- opening read-only connections (refusing missing files, handling special path characters, rejecting writes, and reading `schema_version` while a write transaction is open).
