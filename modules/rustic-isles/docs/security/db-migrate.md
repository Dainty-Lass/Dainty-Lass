# Security: `isles-db` MongoDB-to-`store` Migration

This document covers the security boundaries and invariants of `crates/isles-db` (`isles-db migrate` and `isles-db verify`, rows **RU7** and **RB11**).

---

## 1. Read-Only Source Guarantee

- `crates/isles-db/src/mongo.rs` implements a minimal MongoDB `OP_MSG` (opcode `2013`) wire client that only issues `hello`, SCRAM-SHA-256 `saslStart`/`saslContinue`, `find`, and `getMore`.
- No write commands (`insert`, `update`, `delete`, `findAndModify`, `drop`) exist in the wire client. Even without `--dry-run`, `isles-db` cannot mutate or delete documents in the source MongoDB deployment.
- All socket operations enforce a 30-second read/write timeout (`IO_TIMEOUT`) and bound incoming `OP_MSG` frames to 64 MiB.

## 2. Change-Log Attribution and Hash-Chain Integrity (DD23, DH1, DH5)

- Every write to the destination SQLite or Postgres database runs inside `store::Database::write` with `store::Actor::System("migrate")` (recorded in `__store_change_log` as `system:migrate`).
- Because `store::Database::open` attaches change-log triggers to every table and seals each transaction into the SHA-256 hash chain (`store::history::verify`), migrated rows are auditable and verifiable via `dainty-lass db verify` and `store::history::verify`.
- In `--dry-run` mode, the target database is never opened and zero writes occur.

## 3. Local-by-Default Destination Guard

- When `--to` points to a `postgres://` or `postgresql://` URL, `store::Database::open` enforces `PostgresConfig::allow_remote = false` by default, refusing non-loopback Postgres hosts unless `--allow-remote-postgres` is explicitly passed on the command line.

## 4. Credential and Secret Handling

- MongoDB connection URIs (`--from`) and Postgres URLs (`--to`) may contain credentials; `isles-db` never prints or logs connection URIs.
- Password hashes (`argon2`), session token hashes, exchange code hashes, email verification code hashes, API key hashes, and Discord OAuth PKCE verifiers are migrated as opaque strings without logging their values.

## 5. Cryptographic Content Verification (`isles-db verify`)

- `isles-db verify` recomputes the canonical record state from MongoDB (including deterministic DD18 global IDs and RB11 transforms), reads the live state from the destination database, and compares both the record count and a per-record + per-collection SHA-256 digest across all 31 collections/tables.
- Any missing record, unexpected extra record, or single-byte field discrepancy causes `isles-db verify` to report the offending key and exit with a non-zero status code.
