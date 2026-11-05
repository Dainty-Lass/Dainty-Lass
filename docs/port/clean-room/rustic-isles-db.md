# Clean-Room Provenance Record: `crates/rustic-isles-db` (RD27)

- **Component**: `crates/rustic-isles-db` (crate `rustic-isles-db`, `license = "MIT"`, `Copyright (c) 2026 Silent
  Spud`)
- **Behaviour specification**: `docs/port/spec-rustic-isles-db.md` at dainty-lass `fb3484f8` (branch
  `feat/rustic-isles-clean-room`), with its closing coordinator decisions of 2026-11-05
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a23ec3e9544d683c2` (branch
  `feat/rustic-isles-db`, reset to `feat/rustic-isles-clean-room` before the work)
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, globbed or opened anything under `R:\source\dauntless\dainty-restricted` (any
worktree of it, including the session's additional working directory `splashloader-integration-90130d`),
`legacy-revived`, any `rustic-isles` or `rustic-rifts` tree outside dainty-lass, or any other repository. It ran no
`git show` or `git log -p` of another branch. It searched nothing on the web. No recorded dump or golden was looked
for: the tests run against the crate's own mock server with documents written for them, and the one external vector
used (SCRAM, RFC 7677 section 3) is from the public RFC.

### 1.2 Other sources

None outside this repository: no crate sources from the cargo registry were read. The MongoDB wire protocol (OP_MSG,
BSON) and SCRAM-SHA-256 were written from the spec's own description and general knowledge of the public standards.

## 2. Files read

- `docs/port/spec-rustic-isles-db.md` (whole), `AGENTS.md`, the brief. The dependency specs (store, characters,
  economy-inventory, core) were not read: their contracts were taken from the crates' public items below.
- `Cargo.toml` (workspace dependencies and lints), `fuzz/Cargo.toml`, the header of
  `fuzz/fuzz_targets/rustic_isles_store_decode.rs`
- `crates/store/src`: `lib.rs`, `engine.rs` (first 130 lines), `database.rs` (lines 160-300, 430-500, public items),
  `portable.rs` (first 100 lines), `history.rs` (`Actor`, `ChangeEntry`), `history/chain.rs`' `Verification`, the
  first 60 lines of `connection.rs`
- `crates/rustic-isles-store`: `Cargo.toml`, `lib.rs`, `sql.rs`, `test_support.rs`, both engines' migrations
  (`migrations/*/*/up.sql`), `repo/characters/slots.rs` (first 80 lines), the public items of `repo/mod.rs`, the
  records and traits of `repo/auth.rs` (lines 1-130, 160-200, 280-360, 360-530), the Postgres helper of
  `tests/common/mod.rs` (first 60 lines)
- `crates/rustic-isles-characters`: `Cargo.toml`, `StarterKit` in `lib.rs`, the module list
- `crates/rustic-isles-economy`: `Cargo.toml`, `inventory/storage_policy.rs` (lines 1-135, 270-330), the module lists
- `crates/rustic-isles`: `Cargo.toml` (first 40 lines), `ids.rs` (first 60 lines)
- `crates/module-api/src/network.rs` (`NodeIdentity`, `IdKind`, `GlobalId`)
- How other crates call `rand`, `hmac`, `base64` and `percent-encoding` (by grep)
- `docs/security/rustic-isles-auth.md` for the checklist's format; `docs/port/clean-room/rustic-isles-economy.md`
  (first 120 lines) for this record's

## 3. Files written

### 3.1 The crate, from the spec

| File | Spec |
| --- | --- |
| `src/lib.rs` | §12.1 to §12.3, §12.5, §14 (the seam), §7.1, §9.3, §10.1, §11.1 |
| `src/cli.rs` | §3, §12.4 |
| `src/mongo/{mod,uri,bson,scram}.rs` | §4 |
| `src/mongo/mock.rs` | §16.1 (feature `test-support`) |
| `src/read.rs` | §5 |
| `src/ids.rs` | §6 |
| `src/extract.rs` | §7, §8.1, §8.2, §8.5 |
| `src/repair.rs` | §8.3, §8.4, §14.3 |
| `src/tables.rs` | §9.2, §10.2, §14.2 |
| `src/target.rs` | §9.1 |
| `src/verify.rs` | §10.3, §10.4 |
| `src/export.rs` | §11.2 |
| `tests/{common/mod.rs,cli.rs,migrate.rs,export.rs}` | §16.2 D1, D5 to D15 (D2 to D4 are unit tests) |

### 3.2 Elsewhere

- `Cargo.toml`: `rustic-isles-db` in `[workspace.dependencies]` (a seam change for the integration pass, its own
  commit); `Cargo.lock` with the crate's commit. `hmac` and `percent-encoding` were already there.
- `fuzz/fuzz_targets/rustic_isles_db_{bson,reply,uri,extract,scram}.rs`, their `[[bin]]` entries and the dependency in
  `fuzz/Cargo.toml`, `fuzz/Cargo.lock`
- `docs/security/rustic-isles-db.md`, this record

No other crate was changed.

## 4. Verification

- `cargo test -p rustic-isles-db --locked --no-fail-fast`: 21 unit tests and 17 black-box tests (`cli` 4, `migrate`
  11, `export` 2), 0 failed. The Postgres and real-MongoDB halves (`REFORGED_TEST_POSTGRES_URL`,
  `REFORGED_TEST_MONGO_URI`, `REFORGED_TEST_SEED_MONGO`) were not set, so the rehearsal skipped.
- `cargo clippy -p rustic-isles-db --all-targets --locked -- -D warnings` clean, and `--no-default-features --lib`
  (no export) clean; `cargo fmt --check` clean
- The five fuzz targets ran under `cargo +nightly fuzz` for 30 to 45 s each without a finding
- §6.1's five vectors were computed by this crate's own reading of the formulas (`ids::tests::spec_vectors`) and agree
  with the spec's table. SCRAM matches RFC 7677's example, client proof and server signature included.
- Not run against a real MongoDB, the Node backend or the 1.14.7 client

## 5. Deviations and ambiguities, and how they were resolved

1. **Table declarations are column lists, not `table!`.** The crate declares the 37 tables plus `wallet_balances` and
   `party_members` (names, columns, types from the store's migrations) as data, and writes and reads them with bound
   SQL through `store::portable`, as the store crate does. The read-back selects into one struct of typed slots
   (unused slots are typed NULLs), so no Diesel column-count feature is needed.
2. **The storage policy is read by the crate.** `StoragePolicy::load` takes a module `Shared`, which the tool has not;
   the crate reads `<dir>/inventory_storage_policy.json`, then `<dir>/data/...`, and gives the first that parses as
   JSON to `StoragePolicy::from_document`. Neither loading is still silent (P10).
3. **Placeholder accounts over existing rows.** Insert-only (coordinator item 3). So that verify still compares equal,
   a placeholder for an account the target already has carries the stored name and notes in the expected state.
4. **Non-finite floats.** JSON has no infinity, so an `inf` duration or risk score (P13) is carried in the records as
   its text (`"inf"`), bound as the float and read back as the same text. A NaN still fails on SQLite, which stores
   NaN as NULL in a NOT NULL column.
5. **Character `ord` default**: the first passing document gets 0 (the counter before it is incremented).
6. **An empty banner or glider catalog id seeds nothing** (a starter for a version without one).
7. **verify's report on failure.** `verify_report` returns the report whatever it says; `run_verify` turns a mismatch
   into `VerificationFailed(n)`. `main_with` prints the report (text: a `FAILED` header and `[DIFF]` lines with their
   mismatches; JSON: the report), then the error, and exits 1 (coordinator item 4).
8. **`replay`** parses to `DbError::Replay(args)`, the typed "not this crate" the front routes to unit 10.
9. **Parse errors** are `DbError::Config`, so they print as `error: invalid argument: <text>`. Quoted values in error
   texts use Rust `Debug` quoting throughout.
10. **NUL in a key** on export is `MongoError::Command("BSON key contains NUL: <collection>")`.
11. **SCRAM texts** for the departures: `SCRAM iteration count <n> is outside 1 to 1000000` (0 is refused too),
    `missing v= in SCRAM`, `server signature did not match`, `server refused the proof: <e>`.
12. **Years outside 0 to 9999** are written as JavaScript's `toISOString` writes them (`+010000-...`).
13. **Approval `updated_by` and `reason`**: the first string found (`approvalUpdatedBy`, else `approval.updatedBy`),
    then null when it is empty.
14. **A boolean from a fractional or float-typed number** (`2.0` from a BSON double) reads false: it has no integer
    value (§5.2).
15. **Hidden entry points** for the fuzz targets and tests: `mongo::read_reply`, `MongoClient::over_stream`,
    `mongo::scram`, `extract::Source`, and `fuzzing::{extract_documents, rows, read_target}`.
16. **Texts name `isles-db`**, restricted's binary, while it is the front (§17 item 11 as decided).
17. **Item 6 (remote MongoDB)** is parity: no new flag here (coordinator decision).

## 6. Similarity

To be measured by the coordinator against restricted e0e238b.
