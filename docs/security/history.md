<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the change log (DD23)

The checklist for `store`'s change log, `rust/crates/store/src/history.rs` and `database.rs` (docs/dainty-lass-plan.md
DD23, DH1, DF7) and `history/chain.rs` and `history/record.rs` (DH5). A stub for DX2 to finish: it covers DH1's
layer, the log written in the same transaction as each change, and DH5's hash chain. Shipping it off the server (DH4)
and `history`/`revert` (DH3) add their own sections.

How to use it: before the first service writes player data through `store::Database`, copy this list into the
review, check each item in the code (not from memory), link the code or test that shows it, and sign at the end with
the date and the commit reviewed. An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

DD23's threat is a partner with access to the admin API or the host tools who turns on the owner and erases data.
The log is what lets the owner see and undo it. Layer 1 alone does not stop someone with root on the server, who can
drop the triggers or edit the database file: DH4 (shipping) and DH5 (the hash chain) are what make that visible.

- **In:** every write a service makes through `Database::write(actor, ..)`, on SQLite or Postgres.
- **Out:** `__store_change_log` rows: time, actor, table, row key, operation, the row before and after as JSON.
- **Who writes the log:** triggers `Database::open` puts on every table, never the repositories.
- **Who seals it:** `store` itself, at the end of `Database::write` and at open (`history::seal`), never the closure.
- **Fuzz target:** `history_record` (`rust/fuzz/fuzz_targets/history_record.rs`), for records read back during a
  restore.

## Checklist

### Every change is logged

- [ ] `Database::open` drops and recreates the triggers on every table but `__store_change_log`, `__store_context`
      and `__diesel_schema_migrations`, after the migrations, in one transaction (`prepare`). A table without a
      primary key is refused. Test: `migrations_are_logged_once_and_a_table_without_a_key_is_refused`.
- [ ] The log entry is written by an `AFTER` row trigger in the same transaction as the change: a failed write leaves
      neither. Test: `a_failed_write_leaves_neither_rows_nor_entries`.
- [ ] Insert, update, delete, soft delete and restore each leave one entry with the row before and after. Test:
      `every_change_is_logged_with_its_actor_before_and_after` (both engines).
- [ ] Migrations run with the triggers off; each applied one leaves a `migrate` entry instead.

### Nothing writes around it

- [ ] A write on a connection that did not come from `Database::write` is refused by the trigger
      (`outside store's write path`): SQLite finds no `__store_context` row, Postgres no `dainty.actor` setting.
      Test: `a_write_around_store_is_refused` (both engines).
- [ ] `Database` hands out no connection that can write outside a write closure: `read` runs on read-only
      connections (`query_only`, `default_transaction_read_only`), and the pools are `pub(crate)`.
- [ ] The log refuses `DELETE` (and `TRUNCATE` on Postgres), and every `UPDATE` but the seal (below), from inside
      the write path too. Test: `a_write_around_store_is_refused`.
- [ ] Writes are serial on both engines (SQLite's one writer, `pg_advisory_xact_lock` on Postgres), so the log's
      `id` order is the order of the changes. Test: `writes_are_serial_and_the_log_keeps_their_order`.

### The actor

- [ ] Every write names an `Actor`; one whose text does not parse back (empty, control characters) is refused before
      anything runs.
- [ ] The actor comes from the request context, never from the request body. (For the services that adopt
      `Database`: RU5, DN4, DH2's admin keys.)

### Soft deletes and retention

- [ ] Player-facing tables are listed in `DatabaseSchema::soft_delete` and have a `deleted_at` column; `open`
      refuses one without it.
- [ ] `purge_soft_deleted` removes only rows deleted before the retention period, as `system:retention`, and each
      removal is logged with the row as it was. Test: `old_soft_deletes_are_purged_and_the_purge_is_logged`.

### Postgres

- [ ] A Postgres address off the machine is refused unless `allow_remote` is set: libpq is built without TLS (DF7
      spike), so the password and every row would cross the network in clear. Tests:
      `a_postgres_server_off_this_machine_is_refused_before_connecting`,
      `a_postgres_server_off_this_machine_needs_allow_remote`.
- [ ] The password is `REFORGED_DB_PASSWORD` in `secrets.env`, never in `host.toml`, never logged
      (`PostgresConfig`'s `Debug` leaves it out).
- [ ] Compose's `postgres` container listens on 127.0.0.1 in the shared namespace and asks for the password on it
      (`--auth-host=scram-sha-256`).

### The hash chain (DH5)

- [ ] Every entry has `prev_hash` (the entry before's `hash`, `GENESIS` for the first) and `hash`, `entry_hash` of
      its fields and `prev_hash`: SHA-256 over a domain string, then each field length-prefixed (integers as 8
      big-endian bytes, text as its byte length then the bytes, a missing `before`/`after` as one zero byte), so no
      two entries encode alike. The text is hashed as stored, so `db move` keeps the hashes. Tests:
      `every_field_is_hashed`, `the_encoding_is_pinned` (a change to the encoding breaks every shipped head).
- [ ] The hashes are computed in Rust, once, for writer and verifier alike: SQLite has no SHA-256 a trigger can call,
      and a second encoding in PL/pgSQL could drift from it.
- [ ] Sealing happens inside the write's transaction, before it commits, under the write lock, chaining from the head
      taken when the write began: a failed write leaves no hash, and no two writes chain onto one head. Opening
      takes the Postgres write lock before it touches the log's table, then seals the migration entries.
- [ ] The update guard (`__store_change_log_no_update` on SQLite, `__store_seal_only` on Postgres) lets an entry be
      sealed once: both hashes set on one that has none, nothing else changed. A closure that seals, reseals or
      unseals an entry itself makes its write fail. Test: `a_write_can_seal_nothing_itself`.
- [ ] A log written before the chain gets the columns at open and is sealed from `GENESIS` in the same transaction.
      Test: `a_log_written_before_the_chain_is_sealed_from_genesis_at_open` (both engines).
- [ ] `history::verify` walks the log from `GENESIS` and names the first entry edited (`Edited`), removed or
      reordered (`Unlinked`, with the entry before it), or put in without a hash (`Unsealed`); given a trusted head,
      it also finds a log cut short (`HeadMissing`) or rewritten consistently from some entry on (`HeadMismatch`).
      `history::verify_after` checks only what follows a trusted head. Test:
      `an_entry_edited_or_removed_is_found_and_named` (both engines).
- [ ] `history::record::decode`, for records read back from storage the server doesn't control, refuses anything the
      log couldn't have written: unknown or repeated fields, an id below 1, an actor that doesn't parse, an unknown
      operation, a `before`/`after` its operation doesn't have, a row that isn't a JSON object, a hash that isn't 64
      lower-case hex digits, a record over `MAX_RECORD`. Tests: `anything_the_log_could_not_have_written_is_refused`
      and the `history_record` fuzz target.

## Known gaps

- Root on the server can drop the triggers or the log. DH4 and DH5 are the answer.
- The chain alone proves nothing against root, who can recompute every hash from the first entry edited: only a head
  kept off the server (DH4) shows it. Entries written after the last shipped head can be rewritten unseen.
- Entries put in the log around `store` after the last sealed one are sealed by the next write as if they were its
  own; anyone who can do that can compute the hashes anyway.
- A log sealed at its first open with DH5 is hashed as it stood then: nothing shows an edit made before that open.
- DR's metagame still writes through the SQLite-only `Store`, outside the log, until RV3 moves it.

Reviewed by: ________ on ________ at commit ________
