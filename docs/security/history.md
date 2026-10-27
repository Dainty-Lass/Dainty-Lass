<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the change log (DD23)

The checklist for `store`'s change log, `rust/crates/store/src/history.rs` and `database.rs`, its hash chain,
`history/chain.rs` and `history/record.rs` (DH5), its history and revert engine, `revert.rs` (DH3a), and its
shipping off the server and restore, `recovery.rs` with `rust/crates/history-ship` (DH4) (docs/dainty-lass-plan.md
DD23, DH1, DH3a, DH4, DH5, DF7). A stub for DX2 to finish: it covers DH1's layer, the log written in the same
transaction as each change, DH3a's engine, DH5's chain and DH4's shipping and restore. The `history`/`revert` commands
(DH3b) add their own section.

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

### History and revert (DH3a)

- [ ] A revert never overwrites a newer change silently (plan section 6): a row changed since the selection's first
      entry on it by an entry outside the selection is reported as `changed_since` and left as it is; one that isn't
      as its last entry left it (a migration) as `not_as_logged`. Test: `a_partners_mass_delete_is_reverted_in_one_call`
      (both engines).
- [ ] A row already as the revert would leave it is reported unchanged, so a second revert of the same selection
      writes nothing. Same test.
- [ ] `revert_apply` runs as one `Database::write` of the reverting actor: every compensating change is logged and
      can be reverted in turn, and a failure rolls the whole revert back. Same test (the revert is reverted).
- [ ] The plan is recomputed under the write lock and applied only while its id (SHA-256 of the selection and every
      action, conflict and current row) is the dry run's; otherwise `Error::PlanChanged` and nothing is written. The
      actions a caller passes back are never trusted. Test: `a_plan_that_no_longer_holds_is_refused_and_nothing_is_written`.
- [ ] A selection needs an actor, an account or a start time: there is no "revert everything" call. Test:
      `a_revert_needs_a_selection_and_accounts_need_their_columns`.
- [ ] An inserted row is removed by its table's policy (soft-deleted on a soft-delete table), so a revert doesn't
      hard-delete player data. Test: `inserts_and_updates_are_undone_by_each_tables_policy`.
- [ ] Rows come back parents first and go out children first, so foreign keys hold on both engines.
- [ ] Every identifier in the engine's SQL comes from the catalog or the log and is quoted; every value is bound,
      the account id included.
- [ ] A log copied by `db move` reverts on the other engine: key text and booleans are compared normalised. Test:
      `a_delete_logged_on_sqlite_is_reverted_after_moving_to_postgres`.
- [ ] Who may revert, and the delay or second approval for a bulk one, are DH3b's and DH6's: the engine trusts its
      caller.

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

## Shipping off the server and restore (DH4)

`rust/crates/history-ship` (the shipper, the S3 client, restore, lag) and `rust/crates/store/src/recovery.rs`
(snapshots and replay). Fuzz target: `history_bucket` (listings, object names, segment bodies); record lines
themselves are `history_record`'s.

The boundary: the server writes to a bucket it can't read; the owner's credentials, which never sit on the server,
read it back. Anyone with root on the server has the server's key, so everything a restore reads is untrusted.

### Bucket credentials

- [ ] The server's key may only `s3:PutObject` on the bucket (or the prefix): no `GetObject`, `ListBucket`,
      `DeleteObject`, `PutObjectRetention`, `PutBucketObjectLockConfiguration` or bypass-governance permission. On B2
      an application key with `writeFiles` only; on R2 an API token with Object Write on the one bucket. The shipper
      needs nothing else: `Shipper` only calls `Bucket::put`, and progress lives in the local state file
      (`ShipState`), never read back from the bucket. Test: `the_server_only_ever_puts`.
- [ ] The server's key ID and secret live in `secrets.env`, never in `host.toml` or on a command line; `S3Config`'s
      `Debug` leaves the secret out, and bucket errors carry the status and S3's `<Code>`, never a header, a
      signature or the reply body (`S3Bucket::send`). Test: `endpoints_and_debug`.
- [ ] The owner's key (list and read) is never written on the server: a restore takes it for one run.
- [ ] The endpoint is `https://` (rustls, webpki roots); `http://` is refused unless the host is loopback (a local
      MinIO). No redirects are followed. Every request is SigV4-signed with the payload's SHA-256, and a PUT carries
      `x-amz-checksum-sha256`, which the bucket checks. Tests: `sigv4_matches_aws_examples` (AWS's own examples),
      `endpoints_and_debug`.

### Object lock

- [ ] The bucket is created with object lock and a default retention (`S3Bucket::create_locked`, or the provider's
      console), in COMPLIANCE mode for a real server: in GOVERNANCE mode an account with the bypass permission can
      still delete. The retention is longer than the time it may take the owner to notice (30 days or more).
- [ ] Object lock implies versioning: a PUT to an existing key keeps the version before it, so a server can add
      objects but never change or remove one. Names end with the body's SHA-256, so a body put under a real
      segment's name doesn't match it. Tests: `the_server_only_ever_puts`,
      `planted_objects_and_holes_are_named_not_trusted`.
- [ ] Lifecycle rules that expire objects, if any, are longer than the retention.

### Restore trusts names only as far as the bucket backs them

- [ ] Every body is checked against the hash in its name, and a segment's header against its name's range; entries
      must be in `(after, through]`, increasing. Gzip is inflated with a limit (256 MiB a segment, 16 GiB a snapshot).
- [ ] An object the bucket received (its own `LastModified`, which the server can't set) more than
      `RestoreOptions::trust_window` (1 hour) after the moment restored to is not used: a snapshot or segment
      planted after the fact under an earlier name is ignored and reported.
- [ ] Overlapping segments must agree entry for entry, or the restore stops and names both keys (`skip` one to go on).
      A hole in the segments' ranges is an error, not a quiet stop.
- [ ] Segments are `history::record` lines (DH5), decoded with `record::decode` (which refuses what the log couldn't
      have written) and checked with `Chain` inside the segment (`decode_segment`), across segments and onto the
      snapshot's head (`store::recovery::restore`), and the snapshot's own log is walked from genesis
      (`history::verify`). An entry edited, removed or reordered fails the restore and names the entry (and, for a
      segment, its key). The header's `head_hash` must be the last entry's hash. Tests:
      `segments_round_trip_and_refuse_what_their_name_does_not_say`,
      `planted_objects_and_holes_are_named_not_trusted` (an edited entry; a segment rewritten consistently).
- [ ] `RestoreOptions::trusted_head`, a head the owner kept off the server (the shipped head on the status page,
      `ShipState::last_shipped_hash`, `Lag::shipped_head_hash`), fails a log that has another hash at that entry or
      lacks it: the only way to see a newest segment rewritten with its hashes recomputed. Same test.
- [ ] A snapshot is read as rows only: a SQLite snapshot is opened with `trusted_schema = OFF` and
      `cell_size_check = ON`, its rows copied into a database this build's migrations made. Nothing in its schema
      (triggers, views) reaches the restored database. Its log must end at the id its name says.
- [ ] A restore writes only into a new database: a SQLite file that doesn't exist (written beside it and renamed in
      once complete) or a Postgres database with no tables; one transaction, so a failure leaves nothing.
- [ ] The log is replayed with its own migrations where its `migrate` entries fall, and the restored database is
      opened by `Database::open` at the end, so its triggers are back before anything writes to it. Test:
      `restore_to_a_minute_before_a_delete_on_either_engine` (both engines each way).

### Lag

- [ ] `lag` works from the database and the state file, not from what the shipper says: a stopped shipper shows as
      `Silent` and, once an entry is older than the limit (5 minutes), `Unshipped`. Test:
      `with_shipping_stopped_the_lag_alert_fires`.
- [ ] The shipper checks every entry against the chain from the last shipped head before it leaves
      (`Shipper::ship_through`): a local log edited, cut or reordered since is not shipped on, the failure names the
      entry, and the state's `chain_broken` raises `HistoryBroken`, which always counts as behind. Test:
      `a_local_log_that_no_longer_chains_is_not_shipped_on_and_alerts`.
- [ ] The status page and the admin API show the alert (host-kit and admin rows).

## Known gaps

- Root on the server can drop the triggers or the log. DH4 bounds the loss to what was not shipped yet; DH5's hash
  chain shows where a local history was edited.
- The chain alone proves nothing against root, who can recompute every hash from the first entry edited: only a head
  kept off the server shows it. Restore checks the segments against the snapshot's chain, and against a head the
  owner holds if given one (`RestoreOptions::trusted_head`); entries written after the last shipped head can be
  rewritten unseen.
- A segment someone with the server's key writes for ids the real shipper never shipped (after it was stopped) and
  chains correctly onto the last real one can't be told from a real one, except by its `LastModified` and the trust
  window. Restore to a moment before the compromise.
- Entries put in the log around `store` after the last sealed one are sealed by the next write as if they were its
  own; anyone who can do that can compute the hashes anyway.
- A log sealed at its first open with DH5 is hashed as it stood then: nothing shows an edit made before that open.
- A snapshot is held in memory and sent as one PUT (S3's limit is 5 GiB a PUT); multipart upload would need no more
  permissions, if a server outgrows it.
- DR's metagame still writes through the SQLite-only `Store`, outside the log, until RV3 moves it.
- A change that moved a row's primary key splits its history in two; the engine reports `key_changed` rather than
  revert it.
- A dry run loads every entry the selection holds and every later entry on the tables it touched: no cap yet. DH6's
  guard on bulk reverts is where a limit belongs.

Reviewed by: ________ on ________ at commit ________
