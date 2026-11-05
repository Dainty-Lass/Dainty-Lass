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

Measured by the RD27 reviewer (who may read the AGPL code) with `similarity.md` §1's method, on the crate as committed
at `ee7d5db7`: every `.rs` file in `src/` and `tests/` against each of the 9 files of restricted `e0e238b`'s
`modules/rustic-isles/crates/isles-db/` outside the replay CLI (`src/{lib,main,migrate,mongo,schema,transforms,verify,
export}.rs` and `tests/migrate.rs`; `src/replay.rs` and `tests/replay_test.rs` are unit 10's), the best single file
named. "Any" counts a line matched by a 3+-line run in any of the 9. Paths in the third column are under `isles-db/`;
a file with no match names none.

| File | New lines | Best restricted file (old lines) | Matched | % | Any, % |
| --- | ---: | --- | ---: | ---: | ---: |
| `src/cli.rs` | 301 | `src/main.rs` (158) | 23 | **7.6** | 13.3 |
| `src/export.rs` | 850 | `src/export.rs` (812) | 6 | **0.7** | 0.7 |
| `src/extract.rs` | 1208 | `src/migrate.rs` (3149) | 43 | **3.6** | 4.5 |
| `src/ids.rs` | 133 | `src/mongo.rs` (947) | 7 | **5.3** | 9.8 |
| `src/lib.rs` | 355 | `src/migrate.rs` (3149) | 68 | **19.2** | 25.9 |
| `src/mongo/bson.rs` | 404 | `src/mongo.rs` (947) | 15 | **3.7** | 4.2 |
| `src/mongo/mock.rs` | 174 | `src/mongo.rs` (947) | 10 | **5.7** | 5.7 |
| `src/mongo/mod.rs` | 402 | `src/mongo.rs` (947) | 48 | **11.9** | 13.4 |
| `src/mongo/scram.rs` | 156 | `src/mongo.rs` (947) | 12 | **7.7** | 9.6 |
| `src/mongo/uri.rs` | 141 | `src/mongo.rs` (947) | 25 | **17.7** | 19.9 |
| `src/read.rs` | 194 | `src/transforms.rs` (519) | 34 | **17.5** | 19.6 |
| `src/repair.rs` | 253 | `src/transforms.rs` (519) | 11 | **4.3** | 6.7 |
| `src/tables.rs` | 613 | `src/mongo.rs` (947) | 15 | **2.4** | 2.8 |
| `src/target.rs` | 114 | `src/migrate.rs` (3149) | 6 | **5.3** | 7.9 |
| `src/verify.rs` | 102 | `src/verify.rs` (1673) | 24 | **23.5** | 23.5 |
| `tests/cli.rs` | 105 | none | 0 | 0.0 | 0.0 |
| `tests/common/mod.rs` | 233 | none | 0 | 0.0 | 0.0 |
| `tests/export.rs` | 188 | `tests/migrate.rs` (1181) | 12 | 6.4 | 8.0 |
| `tests/migrate.rs` | 370 | `tests/migrate.rs` (1181) | 9 | 2.4 | 2.4 |
| All 19 | 6296 | | 368 | **5.8** | 7.3 |

No file reaches 25 % against its best single file; `lib.rs` does on the "any" column (25.9), from the public records
below. The fuzz targets (`fuzz/fuzz_targets/rustic_isles_db_*.rs`) have no restricted counterpart and were not
measured.

**What the matches are.** Every run of 5 lines or more against any of the 9 files was read: 16 of them. Each is a
record, enum or text the spec fixes, or boilerplate:

- **Public error enums** (`lib.rs:47-60`, `mongo/mod.rs:34-46`): `DbError` and `MongoError`, the variants and
  `#[error]` texts of §12.5 in its order; the `#[from]` sources are the variants' payloads.
- **`Area`** (`lib.rs:68-98`, three runs): §14.4's seven variants, `ALL` and `as_str()`'s lower-case names (§3.3) in the
  spec's order.
- **Option and report records** (`lib.rs:145-149`, `162-176`, `221-230`; `verify.rs:16-32`): `MigrateOptions`'
  fields and its hand-written `PartialEq` (§14.4: "equality compares every field but `starter`", so a derive cannot
  do it), `MigrateReport` (§12.1), `CollectionVerification`/`VerifyReport` (§12.2), each in the spec's field order.
- **Output texts** (`cli.rs:203-214`, `223-226`): §12.4's `rb11 transforms:` and `skipped_invalid:` lines and the
  verify OK header, with the report's fields as arguments.
- **Wire and repair constants** (`mongo/mod.rs:83-87`, `117`, `123-134`): §4.3's opcode, no-body and `errmsg`
  fallback texts; `repair.rs:106-110`, §8.3's starter item defaults (`markedNew` ... `surged`).
- **Field readers** (`read.rs:77-90`, `153-157`): the match tails of §5.2's integer and float readers and §5.4's value
  reader, one arm per rule in the spec's order; nothing else in `read.rs` (time, ms, boolean, JSON text) matches
  beyond `_ => default, } }`.
- **Boilerplate**: `extract.rs:1311-1315` (`} } _ => {} } }`), and the 3- and 4-line `Ok(())`/`out`/`#[cfg(test)]
  mod tests {` closers that make up most of the remaining matched lines.

Shorter runs are of the same kinds: `MongoUri`'s fields (§4.1, D2), `Command`'s variants (§14.4), `ExportOptions` and
`ExportReport` (§12.3), `email_codes`' purpose renames (§7.4, `extract.rs:491-493`), `is_instance_id` (§16.2 D8's
26 characters of `A`-`Z0`-`9`), the account row's optional fields in `tests/export.rs` (the store's record), SCRAM's
user escaping (§4.6).

No comment was copied (no comment line of 20 characters or more equals a restricted one). Of the crate's 639 string
literals of 12 characters or more that also occur in the 9 files, all are spec texts (§4.3 to §4.6, §7.4's `missing
<field>` errors, §12; the script's check misses ten only where the spec wraps or templates them) or the spec's
`LoadoutSlot_0n` ids. 25 of the crate's 270 function names and 20 of its 73 type and constant names also occur in
restricted; the ones the spec does not give are generic (`deterministic_ulid` for §6.1's formula, `sha256_hex`,
`parse_postgres_url` for §9.1, `USAGE` for §3.5, `CROCKFORD` for §6.1's alphabet, and `PendingLoadout` for §8's
"pending loadouts", whose fields follow §10.4's `loadouts` columns under the crate's own names). No unusual structure
is shared: the large files (`extract.rs`, `export.rs`, `tables.rs`, `bson.rs`, `mongo/mod.rs`) match at 0.7 to 12 %,
and the extractor, the repair passes, the table layer, the BSON codec, the wire client, SCRAM and the verifier share
no logic beyond the lines above.

**Verdict: clean of structural or expressive copying; nothing to rewrite.** Every matched span is a public record,
enum or error type in the spec's order, a text the spec fixes, a spec rule written one arm per case, or boilerplate.
