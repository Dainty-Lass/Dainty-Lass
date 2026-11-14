<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the legacy importer (RX3)

The checklist for `crates/dainty-legacy`, the MIT clean-room rewrite of the legacy importer
([`docs/port/spec-legacy-import.md`](../port/spec-legacy-import.md), RD27, section 12). `SECURITY_REVIEW.md` indexes
it.

How to use it: before the MIT importer replaces `restricted/legacy`, copy this list into the switch's commit or
review, check each item in the code (not from memory), link the code or test that shows it, and sign at the end
with the date and the commit reviewed. An item that does not hold is a bug to fix before the switch, not a box to
leave open.

## What it is and where the boundary is

The importer reads files an old install left behind and writes the 1.14.7 database and the old checkout. None of
its input is trusted: a 1.4.4 server database may have been edited by hand, and a player's offline save comes from
the player. It listens on nothing.

- **In:** a 1.4.4 server's SQLite database (`import revived`), a player's `.drsave` container or bare offline
  database (`import offline-save`), a Reforged Node kit's checkout (`import reforged`: `host.json`, `secrets.env`,
  `backend.env`, the old folders), and the 1.14.7 backend's env file (`--env-file`).
- **Out:** the 1.14.7 database (one transaction as `host:legacy-import`, in the change log so it can be reverted,
  DD23); in a checkout, `settings.toml`, `host.json.bak`, the MongoDB marker and the folders it moves; `dainty-host db
  migrate` and `db verify` as child processes (`--db-tool`).
- **Who runs it:** the host's console (`dainty-host legacy import`, `module run legacy import-<kind>`) or the host on
  the command line (`dainty-legacy import <kind>`). No other actor reaches it.

Code: `crates/dainty-legacy/src/` (`source.rs`, `drsave.rs`, `target.rs`, `reforged/mod.rs`,
`reforged/hostjson.rs`, `envfile.rs`, `args.rs`, `plugin.rs`, `cli.rs`), with the facts it checks against in
`crates/dainty-legacy/data/`.

Fuzz targets in `fuzz`:

- `legacy_database_readers`: arbitrary bytes as a server database, a bare offline database and a `.drsave` (the
  bytes as its database) give an error or a count, never a panic; the source is unchanged and nothing is written
  beside it but the `-wal` and `-shm` any SQLite reader of a WAL database makes.
- `legacy_host_json`: arbitrary text through the `host.json` to `settings.toml` conversion gives an error or a file
  that starts with the header and reads back as TOML.

## Checklist

### The source is never written

- [ ] A server database is copied with SQLite's online backup into a new private temporary folder
      (`dainty-legacy-`), the source opened read-only (`source.rs`, `copy_source`; the store's `backup_file`).
- [ ] An offline database is refused when `-wal`, `-shm` or `-journal` sits beside it, then copied as bytes into a
      new file in the private folder; every later step works on the copy.
- [ ] The test `the_store_change_log_is_dropped_and_other_tables_counted` checks the source's bytes are unchanged.

### Before any row is read

- [ ] `PRAGMA trusted_schema = OFF` runs on the copy before anything reads its schema, so no function in a view or
      trigger runs (`Reader::check`).
- [ ] `__store_*` triggers and tables are dropped (the copy only); the migration tables must have exactly the
      `CREATE` text the store's Drizzle replay and Diesel make; `integrity_check` must be one `ok`; an offline copy
      must be in rollback-journal mode.
- [ ] The history must be the metagame's: the Diesel versions exactly, or 1 to 31 Drizzle rows (at most 32 read)
      each with the recorded time and the LF or CRLF hash from `revived-1.4.4.toml`.
- [ ] Every table and trigger must be one the 1.4.4 metagame made, or a table of SQLite's own (`sqlite_*`:
      `sqlite_sequence`, the `sqlite_stat*` tables `ANALYZE` leaves), and there is no view; every column read must be
      there for the history's prefix.
- [ ] Only named columns of `users`, `account_bans`, `discordlinks`, `invitecodes` (and, for a save, `userapikeys`
      and the cleared tables' counts) are read, each through `CAST`, with the table names quoted.

### The `.drsave` container

- [ ] The header is read first and bounded: version 1, metadata 2 to 65536 bytes, database 512 to 536870912
      bytes, and the file exactly their sum plus 56; every later read is bounded by them (`drsave.rs`).
- [ ] The metadata is checked field by field before the database is streamed; the database goes to `offline.db`
      in its own private folder (`dainty-legacy-save-`), created new, 0600 on Unix.
- [ ] The trailer must equal the running SHA-256 and the database the metadata's; every failure is one message
      that names nothing inside the container. No path comes from inside it.
- [ ] The save's account is the one row whose key hash matches the credential and whose `userId` and name match the
      metadata, with every cleared table empty; nothing else is carried.

### Who may run it

- [ ] Only `Actor::Console` runs an import; any other actor gets `Invalid` (`plugin.rs`, test
      `only_the_console_runs_an_import`). The plugin serves nothing and has no ports.

### Secrets

- [ ] The credential key is held in `drsave::Key`, whose `Debug` hides it; it is hashed and compared, never logged,
      put in an error or a report.
- [ ] No report holds a key, a hash, a password or a secret's value; no error holds a value from the data beyond the
      ids and names a report holds.
- [ ] `secrets.env` is read only for which names have a non-empty value; the values are not kept past the line
      split, and the file is carried untouched (0600 on Unix after a real run) (`envfile::secret_names`).
- [ ] The MongoDB URI is shown redacted in every message and in the marker (`user:***@`, `***@`); a Postgres
      password from the env file is passed only to `dainty-host db`, never printed (`PostgresConfig`'s `Debug`
      leaves it out).

### The Reforged checkout

- [ ] `host.json` is read only when it is a file of at most 262144 bytes, and converted before anything moves, so a
      bad file changes nothing.
- [ ] `settings.toml` is written to `settings.toml.partial` and renamed; `host.json` is kept as `host.json.bak`.
- [ ] Folders are only renamed, never overwritten or deleted: a destination that exists is skipped.
- [ ] MongoDB is moved only through `dainty-host db migrate` and `db verify`; the marker is written only after a
      verify with no differences. Their output is captured, so it never reaches a module's frame pipe.

## Sign-off

Reviewed at commit `________` on `________` by `________`.
