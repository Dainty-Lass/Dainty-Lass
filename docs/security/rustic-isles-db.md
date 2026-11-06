<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Isles database tool (RX3)

The checklist for `crates/rustic-isles-db`, the MIT library behind `reforged-host db migrate`, `verify` and `export-mongo`
(RD27, `docs/port/spec-rustic-isles-db.md`).

How to use it: before a change to this crate merges, and before its cutover is signed (RD10), copy this list into the
change, check each item in the code (not from memory), link the code or test that shows it, and sign at the end with the
date and the commit reviewed. An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

A host runs the tool by hand, once, to move a Node 1.14.7 deployment into the store (and, for one release, back). It
parses untrusted input on two fronts: **the MongoDB server's replies** (network bytes, plaintext TCP) and **the
documents** (the host's own database, but filled by players: names, JSON blobs, arbitrary keys).

- **In:** the command line or the host kit's options (`cli.rs`, `MigrateOptions`, `ExportOptions`); OP_MSG replies
  (`mongo/mod.rs`, `mongo/bson.rs`); SCRAM-SHA-256 server messages (`mongo/scram.rs`); the documents
  (`extract.rs`, `read.rs`); the storage policy file under `--game-data-dir` (`lib.rs`, `load_policy`).
- **Out:** the store, through `store::Database` and bound SQL only (`tables.rs`); MongoDB `insert`/`delete` for
  export only (`MongoWriter`); stdout reports and one warn line per skipped document.
- **Settings:** none. The crate reads no environment variable; the host kit resolves every default first.

Code: `crates/rustic-isles-db/src/` (`lib.rs`, `cli.rs`, `mongo/{mod,bson,scram,uri,mock}.rs`, `read.rs`, `ids.rs`,
`extract.rs`, `repair.rs`, `tables.rs`, `verify.rs`, `export.rs`, `target.rs`).

Fuzz targets in `fuzz`:

- `rustic_isles_db_bson`: arbitrary bytes into the BSON decoder never panic or overflow the stack, and a decoded
  document re-encodes and decodes to an equal value modulo number widths.
- `rustic_isles_db_reply`: arbitrary bytes as the replies to a `find` and its `getMore`s (an in-memory stream) never
  panic and answer documents or an error.
- `rustic_isles_db_uri`: arbitrary text into the MongoDB URI and Postgres URL parsers never panics, and a parsed
  address's `Debug` is the same whatever its password, so it can't show it.
- `rustic_isles_db_extract`: arbitrary JSON documents as every collection, through extraction, the RB11 repairs and
  the export builder, never panic; every JSON text column parses, every loadout keeps at least one slot, every
  repaired item id is 26 characters of `A`-`Z0`-`9`.
- `rustic_isles_db_scram`: arbitrary server-first and server-final payloads never panic and are refused unless
  well-formed, with the client's nonce and an iteration count within the cap; a random signature never passes.

## Checklist

### Read-only source

- [ ] `MongoClient` can send only `hello`, `saslStart`, `saslContinue`, `find` and `getMore`; it has no write method
      (`mongo/mod.rs`). migrate and verify use nothing else.
- [ ] The write commands (`delete`, `insert`) live in `MongoWriter`, which exists only with the `export-mongo`
      feature; `cargo build --no-default-features` has no way to write to MongoDB.

### Bounded parsing

- [ ] A reply's declared length must be 21 bytes to 64 MiB before anything is allocated; the allocation is that
      length (`read_reply`).
- [ ] Every BSON length (document, subdocument, array, string, binary) is checked against the bytes left, and a
      negative int32 length is refused, never cast (`bson.rs`, `Cursor`).
- [ ] Nesting stops at 200 levels with `BSON nesting deeper than 200` (`MAX_DEPTH`; the departure of coordinator item
      7): `deep_nesting_is_an_error_not_a_crash` decodes 10 000 levels.
- [ ] A kind-1 section is skipped by its declared size only after checking it fits; an unknown section kind is an
      error.
- [ ] A collection is held whole in memory (as restricted): acceptable for one host's data, and noted.

### Authentication

- [ ] SCRAM-SHA-256 only, run only when the URI has both a user and a password (`MongoUri::credentials`).
- [ ] The server's signature `v=` is checked in constant time against the one this conversation computed; a missing or
      wrong one, or an `e=`, fails the connection (`verify_server_final`; coordinator item 5).
- [ ] An iteration count of 0 or above 1 000 000 is refused before PBKDF2 runs (`MAX_ITERATIONS`).
- [ ] The server nonce must start with the client's 24-byte random nonce.
- [ ] The RFC 7677 vector passes (`scram::tests::rfc_7677_vector`).
- [ ] The MongoDB connection is plaintext and a non-loopback host is accepted (parity, coordinator item 6, a
      candidate row): a host migrating across a network should tunnel it.

### Secrets

- [ ] `MongoUri`'s `Debug` has no password; `MigrateOptions` and `ExportOptions` print their URIs with the password
      as `***` (`redact`); `PostgresConfig` keeps its own redacted `Debug`.
- [ ] `TargetState`'s `Debug` prints table names and counts only, never a record: password hashes, token, code and
      key hashes, verifiers, CSRF tokens and Guard challenges stay out.
- [ ] No error text holds a password: the URI's password never enters a message, and Postgres errors are the store's.
- [ ] The one log line, `skipping invalid document in <collection>: <detail>`, names a collection and a fixed detail,
      never a document's contents.

### SQL and the store

- [ ] Every value is bound (`store::portable::numbered`, `Param`); identifiers come from the crate's own table
      declarations and are quoted (`tables.rs`). No document text is spliced into SQL.
- [ ] migrate writes in one store transaction under `system:migrate`: an extraction failure writes nothing, and every
      change is in the hash-chained change log (`migrate_counts_and_logs_under_system_migrate`).
- [ ] A Postgres host off this machine is refused unless `--allow-remote-postgres` (`PostgresConfig::check_address`).
- [ ] Nothing is deleted from the store; `deleted_at`, the timeout columns and identity keys are never written.
- [ ] Placeholder account rows (users met in other collections) are insert-only, so a run without `auth` never resets
      a name or notes (coordinator item 3; `existing_ids_and_names_survive_later_runs`).

### Destructive paths

- [ ] verify and export-mongo refuse a SQLite path that does not exist, before opening it and before any MongoDB
      connection (coordinator item 1; `missing_sqlite_files_are_refused_by_verify_and_export`). migrate still creates
      its target.
- [ ] export-mongo replaces whole collections and is not transactional; a dry run connects to nothing.
- [ ] A document with a key holding NUL is refused before anything is sent (`BSON key contains NUL: <collection>`;
      coordinator item 7).
- [ ] A failed verify prints its per-table report and exits 1 (coordinator item 4; `main_with`).

### Known weak spots (kept for parity, spec §17)

- [ ] `characters` alone exports, and so replaces, the whole `playerJourney` collection (§17 item 10).
- [ ] The banner and glider repair turns a 1.14.7 instance-id string in `persistent` into the legacy object (§17 item
      2): check a real 1.14.7 MongoDB before running migrate.
- [ ] Postgres URLs are not percent-decoded (P11).
- [ ] An empty user id aborts the run even with `--skip-invalid` (P2).

## Sign-off

Reviewed at commit `________` on `________` by `________`.
