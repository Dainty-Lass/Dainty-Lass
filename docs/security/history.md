<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the change log (DD23)

The checklist for `store`'s change log, `rust/crates/store/src/history.rs` and `database.rs`, its hash chain,
`history/chain.rs` and `history/record.rs` (DH5), its history and revert engine, `revert.rs` (DH3a), and its
shipping off the server and restore, `recovery.rs` with `rust/crates/history-ship` (DH4) (docs/dainty-lass-plan.md
DD23, DH1, DH3a, DH3b, DH4, DH5, DF7). A stub for DX2 to finish: it covers DH1's layer, the log written in the same
transaction as each change, DH3a's engine, DH5's chain, DH4's shipping and restore and DH3b's commands.

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

## The commands: host kit, console, launcher and admin API (DH3b)

`rust/services/admin/src/history.rs` and `history_text.rs` (the engine over a server's databases and its words), the
host kit's `commands/history.rs` (`reforged-host history` and `revert`, the console's and dashboard's `history` and
`revert`), the admin API's `history`, `historyCheck`, `revertPlan` and `revertApply` (`schema.rs`) and the server GUI's
`history_query`, `history_revert_dry_run` and `history_revert_apply` (`launcher/src-tauri/src/server_history.rs`).
Fuzz target: `history_input` (`rust/fuzz/fuzz_targets/history_input.rs`), for the text a person or the bot
types (an actor, a time, an account, a cursor). Everything below the filter is `store`'s (`history_record` and the
engine's tests).

The boundary: whoever may run these can undo any change to player data, so the roles that may are the point. The host
kit runs on the machine with `admin.env`, so it needs no key, as `admin key` does not; the admin API wants the owner;
the server GUI wants the host's own launcher. The commands trust `store` to write only through its write path.

### Who may

- [ ] Reading the logs (`history`, `historyCheck`) needs `Role::Admin`; planning or applying a revert needs
      `Role::Owner`, checked before anything is read (`allow` is the first line of each). A viewer, a moderator and an
      admin each get `FORBIDDEN` with `requiredRole`. Tests: `only_the_owner_reverts_and_an_admin_reads`, and the
      role table in `each_role_does_what_it_does_and_the_refusal_names_the_role_it_lacks`.
- [ ] The actor of a revert is never taken from the request: `admin:<the key's id>` from the authenticated key,
      `host:console` for the host kit's commands and console, `host:server-gui` for the launcher. Test:
      `a_partners_revoked_keys_are_restored_by_the_owner_in_one_revert_and_a_changed_row_stays` (the audit row and
      the revert's entries name the key).
- [ ] Every revert through the API also leaves a `revertApply` row in `admin_actions` under the key, with the plan's
      id and the counts.
- [ ] A revert of more changes than `revert_limit` goes through `Admin::gate` before anything is written, and a held
      one runs later as `Actor::Admin(<the key that asked>)` through `HistorySet::apply`, which plans again and refuses
      a plan whose id is not the held `planId`. Tests: `a_revert_over_the_limit_is_held_and_applied_once_after_its_delay`,
      `a_revert_under_the_limit_is_immediate`, `a_held_revert_whose_plan_no_longer_holds_is_refused_not_applied`.
- [ ] The host kit's commands read `admin.env` from the workspace and open the databases beside the game's; they do
      not take a database path from the command line, so they cannot be pointed at another server's file.

### What is shown

- [ ] Row bodies come out only through the admin API (admin or owner) and the server GUI, never in the host kit's or
      the console's text, which prints the table, the key and the change. Test: the host kit's
      `a_partners_mass_delete_is_reverted_in_one_command_and_a_row_changed_since_stays`.
- [ ] A row of `admin_keys` goes out with `key_hash` as `<hidden>` (`history::redact`), in a history page, a plan and
      the launcher's page. The hash cannot be turned back into a key, but there is no reason to hand it out. Test:
      `a_partners_revoked_keys_are_restored_by_the_owner_in_one_revert_and_a_changed_row_stays`.
- [ ] A plan or a result lists at most 500 rows of each kind (`MAX_LISTED`), with the real counts, so a mass delete
      cannot make a reply the size of the table. The console prints at most 200 lines of a command.
- [ ] Input is bounded and checked before a database is opened: an account, an actor or a time is 1 to 64 (40)
      characters with no control character, a time is one of the documented forms, `since` is before `until`, a
      cursor is at most 128 characters of `<database>:<id>` or `<database>:end` pairs, a page is 1 to 1000. Text the
      user typed is not echoed back (the console prints it through `plain`). Tests:
      `people_type_actors_and_times_in_the_forms_the_help_gives`,
      `one_cursor_pages_every_log_newest_first_without_a_gap_or_a_repeat`.

### What is applied

- [ ] Applying runs only what the dry run showed: the apply plans again from the same selection and refuses unless the
      plan's id (SHA-256 over every database's plan id) is the one given, then `store` checks each database's id again
      under its write lock. A plan the server no longer has is `PLAN_EXPIRED`, never guessed. Tests: the same in the
      host kit and admin API tests, `a_plan_that_has_gone_is_made_again_from_the_same_selection_or_refused`.
- [ ] The admin API keeps at most 16 plans for ten minutes; a plan is dropped when applied. The server GUI keeps its
      own and applies only a plan its dry run made.
- [ ] Rows changed since stay as they are and are reported by the engine (DH3a's check); the commands report them and
      never offer a way to force them.
- [ ] `Database::attach` opens a database without its owner's migrations: it refuses a SQLite file that is not
      there (it would make an empty one), runs no migration, puts the change-log triggers back on every table and keeps
      a table with a `deleted_at` column soft-deleted. Test:
      `a_database_attached_without_its_migrations_reverts_as_its_owner_does`.
- [ ] The tools write through `Database::write` like every other writer: there is no second path to a table.

## Known gaps (DH3b)

- A revert that spans databases is atomic in each, not across them. A failure after the first leaves it reverted and
  says so; there is no distributed transaction. On Postgres the databases are one.
- A module's own database that is not the game's, the admin API's or chat's is not read until it is added to
  `history::sources` (a file name) or declared with `store::declare_accounts` (account columns): today a module in
  another repository declares its tables by calling `store::declare_accounts("node", ..)` where it opens its
  `Database`, and the host kit, which does not run the module, sees only what `NODE_ACCOUNTS` lists. RU5's tables go
  there.
- The host kit's `revert`, the console and the launcher are not guarded (DH6 guards the admin API's `revertApply`): the
  host's machine is the check there.
- The dry run loads every entry the selection holds; a `--since` far back on a busy server reads a lot.

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

### Shipping to a peer node (DH7)

`rust/plugins/network/src/logship.rs` (`PeerBucket`, `PeerLogStore`, `Service::LogShip` on `/__node/log_ship`). Fuzz
target: `network_logship` (`rust/fuzz/fuzz_targets/network_logship.rs`).

- [ ] `PeerBucket` implements `history_ship::Bucket` over `PeerLink` + `Service::LogShip`, so `history_ship::Shipper`,
      `ShipState`, `lag` and `restore_to` work unchanged whether shipping to S3 (DH4) or to a peer node (DH7).
- [ ] The receiving node (`PeerLogStore`) authenticates the sender via the mTLS node link (`from` comes from the
      verified client certificate, never the body), refuses any node not in its peer list (`UnknownPeer`), and enforces
      that a peer `from` can only write keys under `"<from>/log/..."` or `"<from>/snapshot/..."` (`WrongPrefix`).
- [ ] Every object key is parsed through `SegmentName::parse` or `SnapshotName::parse` before touching disk (rejecting
      any path traversal, dotfile or unknown layout with `InvalidKey`), and `sha256_hex(&body)` is verified against the
      64-hex digest in the key (`HashMismatch`).
- [ ] Storage in `.workspace/peer-logs/<origin>/` is strictly append-only: existing files are never overwritten or
      deleted through `Service::LogShip` (a repeat of the identical content-addressed key returns `Stored { created:
      false }` with the original `received_at`), and `.meta.json` records `received_at` from the receiving peer's own
      `Clock`, which `BucketRead::list` exposes as `ObjectInfo::last_modified` for `RestoreOptions::trust_window`.
- [ ] Segments and full database snapshots shipped via `PeerBucket` are transferred over mTLS (`PeerLink`), but they are
      stored unencrypted at rest in `.workspace/peer-logs/<origin>/` on the receiving peer's filesystem and contain every
      logged table row and database snapshot table, including player IP addresses, Discord user IDs, usernames, and
      HMAC-hashed account keys. Before enabling `ship_to_peer`, a host operator must treat the receiving peer operator as
      having full read access to that player and audit data, ensure the peer's `.workspace/peer-logs/` directory is on
      encrypted storage with restricted OS permissions, and disclose off-node backup storage to players where required.
- [ ] Tests in `rust/plugins/network/tests/logship.rs`:
      `a_destroyed_nodes_database_restores_from_its_peers_folder_with_chain_and_trusted_head_verified`,
      `a_tampered_segment_is_named_and_peer_enforces_prefix_and_append_only`,
      `a_down_peer_makes_lag_alert_fire_and_catch_up_works`.

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
  guard holds an apply over `revert_limit` changes (`Admin::gate(Gate::Revert { .. })`, `docs/admin-api.md`), but a
  dry run changes nothing and is not held.

Reviewed by: ________ on ________ at commit ________
