<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the change log (DD23)

The checklist for `store`'s change log, `rust/crates/store/src/history.rs` and `database.rs`, and its history and
revert engine, `revert.rs` (docs/dainty-lass-plan.md DD23, DH1, DH3a, DF7). A stub for DX2 to finish: it covers DH1's
layer, the log written in the same transaction as each change, and DH3a's engine. Shipping it off the server (DH4),
the hash chain (DH5) and the `history`/`revert` commands (DH3b) add their own sections.

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
- [ ] The log refuses `UPDATE` and `DELETE` (and `TRUNCATE` on Postgres), from inside the write path too. Test:
      `a_write_around_store_is_refused`.
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

## Known gaps

- Root on the server can drop the triggers or the log. DH4 and DH5 are the answer.
- DR's metagame still writes through the SQLite-only `Store`, outside the log, until RV3 moves it.
- A change that moved a row's primary key splits its history in two; the engine reports `key_changed` rather than
  revert it.
- A dry run loads every entry the selection holds and every later entry on the tables it touched: no cap yet. DH6's
  guard on bulk reverts is where a limit belongs.

Reviewed by: ________ on ________ at commit ________
