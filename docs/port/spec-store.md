# Clean-room Behaviour Specification: `store` (`crates/store` SQLite / Drizzle Layer)

## 1. Purpose and Scope

`store` (`rust/crates/store`) provides the database persistence layer for DaintyLass services. Thirteen of its nineteen source files (`backend.rs`, `connection.rs`, `database.rs`, `drizzle_pg.rs`, `history.rs`, `mongo_export.rs`, `recovery.rs`, `revert.rs`, `schema_diff.rs`, `sql_rewrite.rs`, `transfer.rs`, `url.rs`, `verify.rs`) were authored from scratch for DaintyLass and remain untouched.

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

### Module Exports and Re-exports

```rust
pub mod adopt;
pub mod backend;
pub mod backup;
pub mod connection;
pub mod database;
pub mod drizzle;
pub mod drizzle_pg;
pub mod history;
pub mod mongo_export;
pub mod readonly;
pub mod recovery;
pub mod revert;
pub mod schema_diff;
pub mod snapshot;
pub mod sql_rewrite;
pub mod transfer;
pub mod url;
pub mod verify;

pub use database::{Database, DatabaseOptions};
pub use r2d2;
pub use rusqlite;
pub use rusqlite::Connection;
```

### Public Types and Functions (`src/lib.rs`)

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("connection pool: {0}")]
    Pool(#[from] r2d2::Error),
    #[error("migration failed at version {version}: {source}")]
    Migration {
        version: u32,
        #[source]
        source: rusqlite::Error,
    },
    #[error("database schema version {found} is newer than this binary supports ({supported})")]
    NewerSchema { found: u32, supported: u32 },
    #[error(transparent)]
    Adopt(#[from] adopt::AdoptError),
    #[error("backup failed: {0}")]
    Backup(#[source] Box<Error>),
    #[error("blocking database task failed: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
}

#[derive(Debug, Clone)]
pub struct Options {
    pub read_pool_size: u32,
    pub busy_timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            read_pool_size: 4,
            busy_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Clone)]
pub struct Store { /* private fields */ }

impl Store {
    pub fn open(
        path: impl AsRef<Path>,
        options: Options,
        migrations: &[&str],
    ) -> Result<Self, Error>;

    pub fn open_in_memory(migrations: &[&str]) -> Result<Self, Error>;

    pub fn path(&self) -> Option<&Path>;
    pub fn busy_timeout(&self) -> Duration;

    pub async fn write<F, T, E>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<Error> + Send + 'static;

    pub async fn read<F, T, E>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(&Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<Error> + Send + 'static;
}

pub fn migrate(conn: &mut Connection, migrations: &[&str]) -> Result<(), Error>;

pub(crate) async fn run_blocking<F, T, E>(f: F) -> Result<T, E>
where
    F: FnOnce() -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: From<Error> + Send + 'static;
```

### Observable Behaviour (`src/lib.rs`)

1. **Pragma setup on every SQLite connection**:
   - Every writer and reader connection applies:
     - `PRAGMA journal_mode = WAL;`
     - `PRAGMA synchronous = NORMAL;`
     - `PRAGMA foreign_keys = ON;`
     - `PRAGMA temp_store = MEMORY;`
     - `conn.busy_timeout(options.busy_timeout)`
   - Furthermore, reader pool connections enable `PRAGMA query_only = ON;` so accidental writes inside `Store::read` fail immediately with a SQLite error.

2. **`Store::open(path, options, migrations)`**:
   - Creates parent directories of `path` if non-empty (`std::fs::create_dir_all`).
   - Opens the single writer `Connection` at `path`, applies the pragmas, and runs `migrate(&mut writer, migrations)`.
   - Builds an `r2d2::Pool` of read-only connections (`max_size = options.read_pool_size.max(1)`) against `path` with the pragmas plus `PRAGMA query_only = ON`.
   - Stores `Some(path.to_path_buf())` for `Store::path(&self)` and `options.busy_timeout` for `Store::busy_timeout(&self)`.

3. **`Store::open_in_memory(migrations)`**:
   - Generates a unique per-call shared in-memory URI `file:memdb_{id}?mode=memory&cache=shared` (using an atomic counter) with `SQLITE_OPEN_URI`, applies pragmas, runs `migrate`, and creates a reader pool (`read_pool_size = 2`) against the same URI so reader and writer connections see the same in-memory database while different `open_in_memory` calls remain isolated.
   - `Store::path(&self)` returns `None`.

4. **`Store::write` and `Store::read`**:
   - Both run the caller's synchronous closure on `tokio::task::spawn_blocking` via `run_blocking`.
   - `Store::write` serialises writers through a single mutex-guarded `Connection`.
   - `Store::read` checks out a pooled read connection (`r2d2::Pool::get`).

5. **`migrate(conn, migrations)`**:
   - Reads `PRAGMA user_version` (`u32`).
   - Let `supported = migrations.len() as u32`.
   - If `current > supported`, returns `Err(Error::NewerSchema { found: current, supported })`.
   - For each index `idx` from `current as usize` to `migrations.len() - 1`:
     - Let `version = (idx + 1) as u32`.
     - Opens a SQLite transaction (`conn.transaction()`), executes `migrations[idx]` via `execute_batch`, sets `PRAGMA user_version = {version}`, and commits.
     - Any SQLite failure during that step is mapped to `Err(Error::Migration { version, source })`.

---

## 3. Module `drizzle` (`src/drizzle.rs`)

Reads Drizzle Kit migration folders (`*.sql`), computes SHA-256 hashes matching Node's `crypto.createHash("sha256").update(query).digest("hex")`, inspects `__drizzle_migrations`, and replays pending Drizzle SQL files.

### Public Signatures

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    pub tag: String,
    pub sql: String,
    pub sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DrizzleError {
    #[error("reading migrations folder {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("no .sql files in {}", .0.display())]
    Empty(PathBuf),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration {tag} failed on statement {statement}: {source}")]
    Statement {
        tag: String,
        statement: usize,
        #[source]
        source: rusqlite::Error,
    },
    #[error(
        "__drizzle_migrations row {row} has hash {found}, expected {expected} ({tag}) or its LF/CRLF twin"
    )]
    HashMismatch {
        row: usize,
        tag: String,
        found: String,
        expected: String,
    },
    #[error("__drizzle_migrations has {found} rows, but the folder only has {available} migrations")]
    AheadOfFolder { found: usize, available: usize },
}

pub fn hash(sql: &str) -> String;
pub fn hash_matches(sql: &str, recorded: &str) -> bool;
pub fn read_folder(dir: impl AsRef<Path>) -> Result<Vec<Migration>, DrizzleError>;
pub fn last_applied(conn: &Connection) -> Result<Option<(usize, String)>, rusqlite::Error>;
pub fn replay(
    conn: &mut Connection,
    migrations: &[Migration],
    now_ms: i64,
) -> Result<usize, DrizzleError>;
```

### Observable Behaviour

1. **`hash(sql)` and `hash_matches(sql, recorded)`**:
   - `hash(sql)` returns the 64-character lowercase hex SHA-256 digest of `sql.as_bytes()`.
   - `hash_matches(sql, recorded)` returns `true` if `recorded` equals the SHA-256 of `sql` as-is, or of `sql` normalised to LF (`\r\n` -> `\n`), or of `sql` normalised to CRLF (LF -> `\r\n`), so databases migrated on either Windows or Linux verify cleanly regardless of git `core.autocrlf`.

2. **`read_folder(dir)`**:
   - Reads directory entries in `dir`, keeping files whose extension is `"sql"`.
   - Sorts entries lexicographically by file name (`0000_...`, `0001_...`).
   - If no `.sql` files exist, returns `Err(DrizzleError::Empty(dir))`.
   - For each `.sql` file, sets `tag` to the file stem (`0000_clever_zarda`), `sql` to the file contents, and `sha256 = hash(&sql)`.

3. **`last_applied(conn)`**:
   - Checks `sqlite_master` for a table named `__drizzle_migrations`. If absent, returns `Ok(None)`.
   - Queries `SELECT hash FROM "__drizzle_migrations" ORDER BY created_at ASC, rowid ASC`.
   - If the table has 0 rows, returns `Ok(None)`; otherwise returns `Ok(Some((count, last_hash)))`.

4. **`replay(conn, migrations, now_ms)`**:
   - Ensures table `"__drizzle_migrations"` exists:
     ```sql
     CREATE TABLE IF NOT EXISTS "__drizzle_migrations" (
         id SERIAL PRIMARY KEY,
         hash text NOT NULL,
         created_at numeric
     )
     ```
   - Reads existing hashes in `ORDER BY created_at ASC, rowid ASC`.
   - If `applied.len() > migrations.len()`, returns `Err(DrizzleError::AheadOfFolder { found: applied.len(), available: migrations.len() })`.
   - Verifies every applied row `idx` against `migrations[idx]` using `hash_matches(&migrations[idx].sql, &applied[idx])`; if any row does not match, returns `Err(DrizzleError::HashMismatch { row: idx + 1, tag, found, expected })`.
   - For each unapplied migration in `&migrations[applied.len()..]`:
     - Opens a transaction (`conn.transaction()`).
     - Splits `migration.sql` on `"--> statement-breakpoint"`.
     - Executes each non-blank statement (`conn.execute_batch(stmt)`), mapping errors to `DrizzleError::Statement { tag, statement: i + 1, source }`.
     - Inserts `(migration.sha256, now_ms)` into `"__drizzle_migrations"`.
     - Commits the transaction.
   - Returns the number of newly applied migrations.

---

## 4. Module `snapshot` (`src/snapshot.rs`)

Produces a deterministic text fingerprint of a SQLite database's schema (tables, columns, indexes, foreign keys) so two databases created by different paths (e.g. Drizzle replay vs Rust `migrations`) can be compared for structural equivalence.

### Public Signatures

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSnapshot {
    pub name: String,
    pub columns: Vec<ColumnSnapshot>,
    pub foreign_keys: Vec<ForeignKeySnapshot>,
    pub indexes: Vec<IndexSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ColumnSnapshot {
    pub name: String,
    pub col_type: String,
    pub not_null: bool,
    pub default_value: Option<String>,
    pub primary_key: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ForeignKeySnapshot {
    pub from: String,
    pub table: String,
    pub to: String,
    pub on_update: String,
    pub on_delete: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IndexSnapshot {
    pub unique: bool,
    pub columns: Vec<String>,
}

pub fn schema_snapshot(conn: &Connection) -> Result<Vec<TableSnapshot>, rusqlite::Error>;
```

### Observable Behaviour

- Queries `sqlite_master` for user tables (`type = 'table'`, excluding `sqlite_%` and `__drizzle_migrations`), ordered by `name ASC`.
- For each table:
  - Reads `PRAGMA table_info("{table}")`: collects `ColumnSnapshot { name, col_type: type.to_ascii_uppercase(), not_null, default_value, primary_key }` in column order.
  - Reads `PRAGMA foreign_key_list("{table}")`: collects `ForeignKeySnapshot { from, table, to, on_update: on_update.to_ascii_uppercase(), on_delete: on_delete.to_ascii_uppercase() }`, sorted via `Ord`.
  - Reads `PRAGMA index_list("{table}")`: ignores autoindexes (`origin == "pk"` or `name.starts_with("sqlite_autoindex_")`), reads `PRAGMA index_info("{index_name}")` ordered by `seqno`, collects `IndexSnapshot { unique, columns }`, and sorts them via `Ord` (so index names do not affect equality).

---

## 5. Module `adopt` (`src/adopt.rs`)

Prepares a SQLite database file before `Store::open` runs, handling four cases:
1. Fresh database (no user tables and `user_version == 0`);
2. Already managed by Rust (`user_version >= 1`);
3. Legacy Drizzle database at or behind the Drizzle baseline (`__drizzle_migrations` present and `user_version == 0`);
4. Unrecognised schema (tables present, `user_version == 0`, no `__drizzle_migrations`).

### Public Signatures

```rust
#[derive(Debug, Clone, Copy)]
pub struct DrizzleBaseline<'a> {
    pub migrations_dir: &'a Path,
    pub target_version: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Schema<'a> {
    pub migrations: &'a [&'a str],
    pub drizzle: Option<DrizzleBaseline<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Fresh,
    AlreadyAt(u32),
    Adopted { replayed: usize, version: u32 },
}

#[derive(Debug, thiserror::Error)]
pub enum AdoptError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("drizzle: {0}")]
    Drizzle(#[from] drizzle::DrizzleError),
    #[error(
        "database has tables ({tables:?}) with user_version = 0 and no __drizzle_migrations table"
    )]
    UnknownSchema { tables: Vec<String> },
    #[error(
        "__drizzle_migrations is present, but no DrizzleBaseline was supplied to adopt it"
    )]
    DrizzleNotConfigured,
    #[error("adopted Drizzle schema does not match baseline version {version}: {diff}")]
    SchemaDrift { version: u32, diff: String },
}

impl Store {
    pub fn open_with_adoption(
        path: impl AsRef<Path>,
        options: Options,
        schema: Schema<'_>,
    ) -> Result<(Self, Outcome), Error>;
}

pub fn prepare(conn: &mut Connection, schema: Schema<'_>) -> Result<Outcome, AdoptError>;
```

### Observable Behaviour

1. **`prepare(conn, schema)`**:
   - Reads `PRAGMA user_version`. If `user_version > 0`, returns `Ok(Outcome::AlreadyAt(user_version))`.
   - Lists user tables in `sqlite_master` (`type = 'table' AND name NOT LIKE 'sqlite_%'`).
   - If the table list is empty, returns `Ok(Outcome::Fresh)`.
   - If `"__drizzle_migrations"` is not in the table list, returns `Err(AdoptError::UnknownSchema { tables })`.
   - If `schema.drizzle` is `None`, returns `Err(AdoptError::DrizzleNotConfigured)`.
   - With `Some(baseline)`:
     - Reads the Drizzle folder via `drizzle::read_folder(baseline.migrations_dir)` and replays any unapplied Drizzle migrations via `drizzle::replay(conn, &folder, now_ms)`.
     - Builds an in-memory reference `Connection`, runs `crate::migrate(&mut ref_conn, &schema.migrations[..baseline.target_version as usize])`, and compares `schema_snapshot(conn)` against `schema_snapshot(&ref_conn)`.
     - If the snapshots differ, returns `Err(AdoptError::SchemaDrift { version: baseline.target_version, diff })`.
     - Otherwise sets `PRAGMA user_version = {baseline.target_version}` and returns `Ok(Outcome::Adopted { replayed, version: baseline.target_version })`.

2. **`Store::open_with_adoption(path, options, schema)`**:
   - Creates parent directories of `path`, opens a connection with `options.busy_timeout`, runs `prepare(&mut conn, schema)`, closes the preparation connection, and then calls `Store::open(path, options, schema.migrations)`.

---

## 6. Module `backup` (`src/backup.rs`)

Performs online SQLite hot backups using SQLite's Online Backup API (`rusqlite::backup::Backup`).

### Public Signatures

```rust
pub const BACKUP_DEADLINE: Duration = Duration::from_secs(30);

impl Store {
    pub async fn backup(&self, dest: impl AsRef<Path>) -> Result<(), Error>;
}

pub fn backup_file(
    src: &Path,
    dest: &Path,
    busy_timeout: Duration,
    deadline: Duration,
) -> Result<(), Error>;
```

### Observable Behaviour

- `backup_file(src, dest, busy_timeout, deadline)`:
  - Creates parent directories of `dest` if needed, removes any leftover temp/dest file at `dest` before starting, opens `src` (read-only) and `dest` (read-write/create) with `busy_timeout`, and steps `rusqlite::backup::Backup::new(&src_conn, &mut dest_conn)` in chunks (e.g. 100 pages per step with brief pause on busy/locked) until completion or `deadline` expires.
  - Any error is wrapped in `Error::Backup(Box::new(...))`.
- `Store::backup(&self, dest)`:
  - For a file-backed `Store`, runs `backup_file(path, dest, self.busy_timeout(), BACKUP_DEADLINE)` on the blocking pool.
  - For an in-memory `Store`, runs `rusqlite::backup::Backup` from the store's connection to `dest` on the blocking pool.

---

## 7. Module `readonly` (`src/readonly.rs`)

Opens existing SQLite database files in strict read-only mode (`SQLITE_OPEN_READ_ONLY`) without creating files or running migrations, used by CLI diagnostic commands (`doctor`, `verify`, `backup`).

### Public Signatures

```rust
pub fn open_read_only(path: impl AsRef<Path>, busy_timeout: Duration) -> Result<Connection, Error>;
pub fn schema_version(conn: &Connection) -> Result<u32, Error>;
```

### Observable Behaviour

- `open_read_only(path, busy_timeout)` opens `path` with `OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX`, sets `conn.busy_timeout(busy_timeout)`, and executes `PRAGMA query_only = ON;`. Fails with `Error::Sqlite` if the file does not exist.
- `schema_version(conn)` queries and returns `PRAGMA user_version` (`u32`).

---

## 8. Test Expectations (`tests/store.rs` and Unit Tests)

The implementation must pass:
1. All unit tests in `drizzle`, `adopt`, `backup`, `readonly`, and `snapshot`.
2. Integration tests in `tests/store.rs`:
   - `Store::open` and `Store::open_in_memory` apply migrations sequentially and advance `PRAGMA user_version`.
   - Opening a database whose `user_version` exceeds `migrations.len()` fails with `Error::NewerSchema`.
   - `Store::read` rejects write statements (`INSERT`/`UPDATE`/`DELETE`) because `PRAGMA query_only = ON` is active on reader pool connections.
   - Concurrent `Store::read` calls and `Store::write` calls complete without `SQLITE_BUSY` under load.
   - `Store::backup` produces a consistent, queryable SQLite file while concurrent writes are running.
   - `Store::open_with_adoption` adopts both fully-applied and partially-applied Drizzle databases, rejects unknown schemas (`AdoptError::UnknownSchema`), detects schema drift (`AdoptError::SchemaDrift`), and verifies CRLF vs LF Drizzle migration hashes.
3. All tests in `crates/store` across the 13 born-here files (`cargo test -p store`).
