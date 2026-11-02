# Clean-room Provenance Record: `store` (`crates/store`)

## 1. Specification Version

- **Spec file**: [`docs/port/spec-store.md`](../spec-store.md)
- **Behaviour-only spec commit**: `08d54b6` (branch `docs/f5-spec-store-behaviour-only`, merged into `plan` at `5abdb91`)
- **Spec audit**: Describes only public types, public function/method signatures, and observable behaviour. Contains zero private/`pub(crate)` struct fields, zero private helper names, and zero internal builder chains or ordered pragma lists (`run_blocking` lives in the born-here MIT `src/database.rs` module).

## 2. Fresh-Folder Clean-Room Isolation

To ensure the clean-room implementer had no access to `dainty-lass`'s Git object store (`.git/objects`), commit history, reflogs, or prior `store` files:

- **Fresh folder path**: `R:\source\dauntless\fresh-store-cleanroom` (initialized with `git init`, zero remotes, zero prior commits, and outside the `dainty-lass` repository tree)
- **Implementer subagent session**: [`7085b750-cbfc-4a97-b948-d30634259171`](conversation://7085b750-cbfc-4a97-b948-d30634259171) (`store-cleanroom-implementer`)
- **Transcript log**: `C:/Users/Nick/.gemini/antigravity/brain/7085b750-cbfc-4a97-b948-d30634259171/.system_generated/logs/transcript.jsonl`
- **Exact files placed in `R:\source\dauntless\fresh-store-cleanroom` before the implementer ran (48 files)**:
  - `.cargo/config.toml`
  - `Cargo.lock`
  - `Cargo.toml` (minimal two-member workspace containing only `crates/module-api` and `crates/store`)
  - `rust-toolchain.toml`
  - `spec-store.md` (copy of `docs/port/spec-store.md` at `08d54b6`)
  - `crates/module-api/Cargo.toml`
  - `crates/module-api/src/actor.rs`
  - `crates/module-api/src/bin/module-api-stub.rs`
  - `crates/module-api/src/chat.rs`
  - `crates/module-api/src/error.rs`
  - `crates/module-api/src/frame.rs`
  - `crates/module-api/src/history.rs`
  - `crates/module-api/src/host.rs`
  - `crates/module-api/src/lib.rs`
  - `crates/module-api/src/manifest.rs`
  - `crates/module-api/src/module.rs`
  - `crates/module-api/src/network.rs`
  - `crates/module-api/src/panel.rs`
  - `crates/module-api/src/plugin.rs`
  - `crates/module-api/src/registrar.rs`
  - `crates/module-api/src/runner.rs`
  - `crates/module-api/src/stub.rs`
  - `crates/module-api/src/types.rs`
  - `crates/module-api/src/version.rs`
  - `crates/module-api/tests/in_process.rs`
  - `crates/module-api/tests/manifests.rs`
  - `crates/module-api/tests/out_of_process.rs`
  - `crates/store/Cargo.toml`
  - `crates/store/src/connection.rs` (born-here MIT, `DD15`)
  - `crates/store/src/database.rs` (born-here MIT, `DD15` / `DD23`)
  - `crates/store/src/declared.rs` (born-here MIT, `DH3b`)
  - `crates/store/src/engine.rs` (born-here MIT, `DD15`)
  - `crates/store/src/history.rs` (born-here MIT, `DH1`)
  - `crates/store/src/history/chain.rs` (born-here MIT, `DH5`)
  - `crates/store/src/history/record.rs` (born-here MIT, `DH1`)
  - `crates/store/src/portable.rs` (born-here MIT, `DF7`)
  - `crates/store/src/recovery.rs` (born-here MIT, `DH4`)
  - `crates/store/src/revert.rs` (born-here MIT, `DH3a`)
  - `crates/store/src/transfer.rs` (born-here MIT, `DD15`)
  - `crates/store/tests/database.rs` (born-here MIT, `DD15` / `DH1`)
  - `crates/store/tests/revert.rs` (born-here MIT, `DH3a`)
  - `crates/store/tests/fixtures/drizzle/0000_baseline.sql`
  - `crates/store/tests/fixtures/drizzle/meta/_journal.json`
  - `crates/store/tests/fixtures/migrations/00000000000000_baseline/up.sql`
  - `crates/store/tests/fixtures/migrations/20260401000000_add_notes/up.sql`
  - `crates/store/tests/fixtures/Postgres/00000000000000_baseline/up.sql`
  - `crates/store/tests/fixtures/Postgres/20260401000000_add_notes/up.sql`
  - `crates/store/tests/fixtures/Sqlite/00000000000000_baseline/up.sql`
  - `crates/store/tests/fixtures/Sqlite/20260401000000_add_notes/up.sql`

## 3. Branch & Commit Trail (`feat/f5-store-clean-room-redo`)

1. **Deletion commit (`67e2881`)**: Moved `pub(crate) async fn run_blocking` into the born-here MIT `src/database.rs` module and executed `git rm` on the 7 files (`src/lib.rs`, `src/adopt.rs`, `src/backup.rs`, `src/drizzle.rs`, `src/readonly.rs`, `src/snapshot.rs`, `tests/store.rs`).
2. **Addition commit**: Copied the 7 newly authored files from `R:\source\dauntless\fresh-store-cleanroom\crates\store` into `crates/store/`.

## 4. 3+-Line Similarity Against `b190010`

Measured using 3+-consecutive-line exact matches on trimmed non-blank, non-comment lines against `b190010`:

| File | `b190010` Lines | New Lines | Matched Lines (3+ runs) | Similarity | Notes on Matching Runs |
| --- | ---: | ---: | ---: | ---: | --- |
| `src/lib.rs` | 189 | 201 | 110 | 54.7% | 103 of the 110 matched lines are the crate's public API surface (`pub mod` / `mod` declarations, `pub use` re-exports, `pub enum Error` with its 14 `#[error("...")]` strings, `pub struct Options` + `Default`, and the public method signatures `open`, `read`, `write`, `checkpoint`, `migrate`). Only 7 lines outside the public declarations match (`(*conn).transaction(f) }) .await` and `conn.batch_execute("PRAGMA wal_checkpoint(TRUNCATE);")?; Ok(()) }) .await`). |
| `src/adopt.rs` | 84 | 97 | 26 | 26.8% | Standard Diesel imports, `pub struct Schema`, `pub struct DrizzleBaseline`, and `conn.run_pending_migrations(...)`. |
| `src/backup.rs` | 53 | 89 | 11 | 12.4% | Closing braces and the 1-line `run_blocking` call in `Store::backup_to`. |
| `src/drizzle.rs` | 101 | 108 | 34 | 31.5% | `pub struct Migration`, public signatures (`hash_matches`, `last_applied`, `replay`), and Drizzle Kit's `"CREATE TABLE IF NOT EXISTS \"__drizzle_migrations\" ..."` DDL. |
| `src/readonly.rs` | 66 | 42 | 6 | 14.3% | Only `conn.batch_execute(&format!("PRAGMA busy_timeout = {}; PRAGMA query_only = ON;", ...))?; Ok(conn)`. |
| `src/snapshot.rs` | 22 | 32 | 8 | 25.0% | Only `use diesel` imports, `#[derive(QueryableByName)] #[diesel(sql_type = Text)] sql: String`, and the public signature `pub fn schema_snapshot(conn: &mut Connection) -> Result<String, Error>`. |
| `tests/store.rs` | 250 | 373 | 22 | 5.9% | Only `#[derive(QueryableByName)]` 1-field row structs and `.await.unwrap()`. |

## 5. Verification

- In `R:\source\dauntless\fresh-store-cleanroom`:
  - `cargo fmt --all -- --check`: passed (exit code 0)
  - `cargo clippy -j 2 -p store --all-targets --locked -- -D warnings`: passed (0 warnings)
  - `cargo test -j 2 -p store --locked -- --test-threads=1`: **44 passed, 0 failed** (16 unit tests in `src/lib.rs`, 13 in `tests/database.rs`, 7 in `tests/revert.rs`, 8 in `tests/store.rs`)
