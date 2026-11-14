<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: the legacy importer (RD27)

This page specifies `restricted/legacy` (`dainty-legacy-import` and the `legacy` plugin `dainty-legacy`) so it can
be re-created as an MIT crate at the root, the way the gateway, the store and the Rifts crates were. The implementer
works from this page alone and never opens `restricted/legacy`. What is copied from Dauntless Revived today (the
1.4.4 metagame's SQL and schema code, and the `.drsave` reader) becomes facts in TOML files: migration names, their
times, their hashes, table and column names. No SQL from Dauntless Revived crosses.

Quoted text in `code` is exact. "1.4.4 database" means a SQLite file made by Dauntless Revived's metagame, Node
(Drizzle) or Rust (Diesel). "Target" means the 1.14.7 database rustic-isles made.

---

## 1. Scope and provenance

| Part today | Provenance | In the MIT crate |
| --- | --- | --- |
| `sql/drizzle/*.sql` (31 files), `migrations/revived/0001_baseline` | DR, copied byte for byte | Gone. Facts in `revived-1.4.4.toml` (section 2) |
| `revived/schema.rs`, `revived/source.rs` | DR, ported | Rewritten from sections 5 and 6. The source is checked and read where it stands; it is no longer migrated forward |
| `revived/drsave.rs`, the offline-save checks in `revived/mod.rs` | DR, ported | Rewritten from section 7 |
| `revived/read.rs`, `target.rs`, `report.rs`, `error.rs`, `cli.rs`, the plugin | Born in Dainty Lass, AGPL by location | Rewritten from sections 3, 4, 8, 9, 10 |
| `reforged/hostjson.rs`, `layout.rs`, `mongo.rs` | Ported from `dainty-hostkit`, already MIT | Rewritten from section 11; the key and folder tables become `reforged.toml` |
| `reforged/secrets.rs` | Born in Dainty Lass | Section 11.4 |

**Decision for the maintainer (L-1).** The born-in-Dainty-Lass and hostkit-derived files are the maintainer's own
work and could move to the MIT crate as they are, which the clean-room review would record. This page specifies them
anyway, so either route works; the DR-derived rows above must be rewritten from this page.

The new crate is `crates/dainty-legacy` (`license = "MIT"`), module id `legacy`. It depends only on root crates:
`dainty-store` (SQLite connections, the online backup, `drizzle::hash_matches`, read-only opens, `Database` with the
change log), `dainty-module-api`, and `rustic-isles-db` for `ids::derived_account_id`. Whether it still runs out of
process (DD5 made it a child process only to keep AGPL code out of the core) is decision L-2: in process, it is one
more `[modules]` plugin and `dainty-host legacy` calls it directly.

## 2. The data files

All three are embedded in the binary (`include_str!`) and parsed once; a parse failure is a bug and a test checks it.

### 2.1 `revived-1.4.4.toml`

The 1.4.4 metagame's history, as facts. Appendix A is its full content.

```toml
[[drizzle]]                 # oldest first; index 0 is migration 0000
tag = "0000_natural_korg"
when = 1782098648901        # the journal's `when`, which Drizzle stores as `created_at`
sha256_lf = "<64 hex>"      # SHA-256 of the file with LF line ends
sha256_crlf = "<64 hex>"    # the same file with CRLF line ends (a server whose checkout was CRLF)

[diesel]
versions = ["0001"]         # the Rust metagame's migrations; "0001" is the baseline cut from drizzle 0030

[[column]]                  # a column the importer reads, and the drizzle migration that made it
table = "users"
name = "userId"
since = 0
```

`since` is a drizzle index. A database "at prefix k" has the first k drizzle migrations applied (1 to 31); a Diesel
database is at prefix 31. A column with `since = s` is present exactly when k > s.

The lists `tables`, `triggers`, `carried`, `not_player_data` and `cleared` (section 6) are in the same file.

### 2.2 `reforged.toml`

The `host.json` to `settings.toml` key tables (`top`, `keys.<table>`, each entry `[json name, toml name]` in
settings.toml's order), the retired keys and modules, the file header, the secret names and their old names, and the
folder moves (`moves`, `sdk_files`, `old_folders`). Appendix B is its full content.

## 3. Commands

Two entry points run the same three imports with the same arguments:

- **The console:** `dainty-host legacy import <kind> ...`, which is `module run legacy import-<kind> ...`.
- **The command line:** `dainty-legacy import <kind> ...` (or, with L-2 in process, `dainty-host legacy import`
  before the server first starts). With no arguments the executable is the plugin (out of process only).

`<kind>` is `revived`, `offline-save` or `reforged`. The usage text, appended to every usage error after a newline:

```text
usage:
  import revived      <database> (--to <1.14.7 database> | --env-file <backend env file>) [--node <id>] [--dry-run]
  import offline-save <.drsave or database> (--to <file> | --env-file <file>) [--node <id>] [--dry-run]
  import reforged     <checkout> [--to <file> | --env-file <file>] [--fresh-start] [--db-tool <program>] [--dry-run]
every path is absolute
```

Argument rules, checked in this order as each argument is read; the first failure is the error:

| Argument | Rule | Error |
| --- | --- | --- |
| `--dry-run` | any kind | — |
| `--fresh-start` | `reforged` only; elsewhere it is an unknown option | — |
| `--to`, `--env-file` | once in total | `give the 1.14.7 database once: --to or --env-file` |
| `--to <path>`, `--env-file <path>` | absolute | `--to must be an absolute path` (`--env-file` likewise) |
| a flag missing its value | | `<flag> needs a value` |
| `--node <id>` | a valid `NodeId` | `--node: <parse error>` |
| `--db-tool <path>` | `reforged` only, absolute | `--db-tool must be an absolute path` |
| `--isles-db` | `reforged` | `--isles-db is gone: the database steps run through dainty-host db; name that program with --db-tool <program> (dainty-host from PATH by default)` |
| any other `--…` | | `unknown option <flag>` |
| the first non-flag | absolute | `the source must be an absolute path` |
| a second non-flag | | `one source only` |

Then: no source is `which install or file to import?`; `revived` or `offline-save` without a target is
`which 1.14.7 database to import into? --to or --env-file`; `--fresh-start` with a target is
`--fresh-start moves no MongoDB, so it takes no --to or --env-file`.

On the command line a report prints to stdout and exits 0; a failure prints `dainty-legacy: <error>` to stderr and
exits 1; an unknown kind or a first argument other than `import` prints the usage to stderr and exits 2. The node
defaults to `local` there, and to the running node from the console.

**`--env-file`.** `NAME=value` lines; blank lines and `#` lines skipped; the name and value trimmed, then one layer
of `"` or `'` trimmed from the value; the last line for a name wins. A setting `DAINTY_X` is read as `DAINTY_X`, or
else as `REFORGED_X` (an older kit's file). The engine is `DAINTY_DB_ENGINE` (default `sqlite`). For SQLite without
`DAINTY_DB_FILE`: `<file>: no DAINTY_DB_FILE, so not the 1.14.7 backend's env file`. Otherwise the database settings
are the store's own (`DatabaseConfig::from_lookup`); its error is `<file>: <error>`. A password is never printed.

## 4. The target

An import writes only these tables of the target, which rustic-isles' migrations `0001`, `0020` and `0200` make:
`accounts`, `launcher_accounts`, `auth_identities`, `reforged_invites`. Their columns are in the MIT
`rustic-isles-db` crate and `docs/port/spec-rustic-isles-db.md`.

**Opening.** For a SQLite target, first: no file there is
`target database: <path>: there is no database here; start the 1.14.7 server once so it makes one`. It is opened
read-only and `SELECT version FROM __diesel_schema_migrations` read; a failure is
`target database: not a database rustic-isles made (<error>)`. Any of `0001`, `0020`, `0200` missing is
`target database: not a 1.14.7 database rustic-isles made, or one too old: migrations <comma-joined> are missing`.
Then it is opened as a store `Database` with no migrations of its own, the 16 soft-deleted tables of rustic-isles'
schema declared (`accounts`, `launcher_accounts`, `characters`, `loadouts`, `inventories`, `progression_tracks`,
`progression_objectives`, `escalation_progress`, `player_journey`, `breadcrumbs`, `encountered_content`,
`bounty_states`, `cooldown_states`, `entitlements`, `wallets`, `wallet_balances`) so the store puts back the same
triggers the backend runs with. A Postgres target is checked the same way inside the write. Version `0610` present
means the target keeps timeouts.

**Writing.** One transaction as actor `host:legacy-import` (the change log, DD23, so an import can be reverted). On a
dry run the same writes run and the transaction is rolled back, so the report counts what a real run would write.

**The account id.** `rustic_isles_db::ids::derived_account_id(node, user_id.trim())`, the same id `dainty-host db
migrate` gives a MongoDB user, so a second import finds the first and a user carried both ways is one account.

**Per account**, in source order (`userId` ascending):

1. Trim the user id. Its derived id already in `accounts`: count `unchanged accounts`, next account.
2. Another `accounts` row with this `user_id`: skip, `a 1.14.7 account already has this userId`.
3. The name trimmed is empty: skip, `it has no name`.
4. The normalised name (trimmed, lower-cased) held by a `launcher_accounts` row with `deleted_at IS NULL`: skip,
   `the name "<name>" is taken on this server` (the name as a Rust `{:?}` string).
5. Insert `accounts (id, user_id, name, notes)`.
6. Insert `launcher_accounts`: `account_id`, `display_name_normalized`, `display_name`, `status`,
   `approval_status = 'approved'`, `approval_updated_at = now`, `approval_updated_by = 'legacy-import:<source>'`,
   `roles = '["user"]'`, `created_at = now`, `username_set = 1`. Count `accounts`.
7. The ban, if any: with an end time and a target that keeps timeouts, `status = 'active'` and
   `timeout_until = <end as YYYY-MM-DDTHH:MM:SS.mmmZ>`, `timeout_since = now`, `timeout_reason = <reason>`; count
   `timeouts`. Otherwise `status = 'banned'`; count `bans`, and also `temporary bans made permanent` when it had an
   end. No ban: `status = 'active'`.
8. A 1.4.4 admin: count `1.4.4 admins imported as players` (no role carries).
9. The Discord link, if any: another row in `auth_identities` with `provider = 'discord'` and this subject: skip
   `discord_link`, `that Discord account is linked to another 1.14.7 account`. Otherwise insert
   `(provider 'discord', provider_subject, account_id, provider_username = the subject, provider_avatar_url NULL,
   linked_at)`; count `discord links`. (1.4.4 kept no Discord username; 1.14.7 refreshes it at the next sign-in.)

**Per invite** (server imports only, code ascending): the code already in `reforged_invites`: `unchanged invites`.
Not infinite and no uses left: skip `invite`, `it has no uses left`. Else insert `(code, max(uses, 0), infinite,
note 'imported from <source>', created_at now)`; count `invites`.

`now` is the import's start time as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC). `<source>` is `revived` or `offline-save`.
Any database error inside the write is `target database: <error>`.

## 5. A 1.4.4 database: copy and check

The host's file is never written or opened for writing.

**Copy.** Into a new private temporary folder (prefix `dainty-legacy-`):

- A server database: SQLite's online backup (store's `backup_file`), 5 s busy timeout, 15 min deadline; then the
  copy's journal mode set to `DELETE`.
- An offline database: refused if `<file>-wal`, `<file>-shm` or `<file>-journal` exists
  (`it has journal files beside it, so it was not stopped cleanly`); else a plain file copy.
- No file at the path: `<path>: there is no file here`.

Every refusal in sections 5 and 6 is an invalid-source error with the text
`not a Dauntless Revived 1.4.4 database: <reason>`.

**Checks**, on the copy, in order:

1. `PRAGMA trusted_schema = OFF`, before anything reads the schema.
2. Drop every trigger, then every table, whose name starts `__store_` (the change log a Dainty Lass-run metagame
   carried; not the metagame's).
3. `__drizzle_migrations` and `__diesel_schema_migrations`, where present, must have exactly the `CREATE` text the
   store's own Drizzle replay and Diesel bookkeeping make, white space collapsed to single spaces; else
   `a migration table is not the one its tool makes`.
4. `PRAGMA integrity_check` must give exactly one row, `ok`; else `it fails SQLite's integrity check`.
5. An offline database must be in journal mode `delete`; else
   `an offline save is a stopped database in rollback-journal mode`.
6. **The prefix.** With `__diesel_schema_migrations`: its versions, ascending, must equal `[diesel] versions`
   (`its Diesel history is not one the 1.4.4 metagame made`); the prefix is 31. Else with `__drizzle_migrations`:
   its rows by `created_at`, at most 32 read, must be 1 to 31 rows, and row i must have `created_at` = `when` of
   drizzle entry i and a hash equal to that entry's `sha256_lf` or `sha256_crlf`; else
   `its Drizzle history is not one the Node metagame made`. The prefix is the row count. Neither table:
   `it has neither Drizzle's nor Diesel's migration history`.
7. **The objects.** Every table must be in `tables`, be `__drizzle_migrations` or `__diesel_schema_migrations`, or
   start `sqlite_` (SQLite's own: `sqlite_sequence`, and the `sqlite_stat*` tables `ANALYZE` or `PRAGMA optimize`
   leave); every trigger must be in `triggers`, and there must be no view; else
   `it holds objects the 1.4.4 metagame never made`.
8. **The columns.** For each `[[column]]` with `since` below the prefix, `PRAGMA table_info` of its table must list
   it; else `its schema is not the one its migration history makes`.

> **Departure from today (L-3).** Today's importer replays the frozen SQL into a scratch database, compares the full
> schema, and migrates the copy forward to the final 1.4.4 schema before reading. Without the SQL, the copy is
> checked by history, object names and the columns read, and read where it stands. With `trusted_schema` off, no
> function in a view or trigger can run, and only named columns of five tables are read, so what the full comparison
> added was refusing a hand-altered file whose read columns still exist. The oracle (section 13) checks that both
> give the same import on every real prefix.

## 6. A 1.4.4 database: what is read

Columns a prefix lacks take the defaults below; a table it lacks is empty.

| Read | Query | Missing |
| --- | --- | --- |
| Accounts | `users`: `userId`, `name`, `notes`, `isAdmin`, by `userId` | `isAdmin` (before 0008): false |
| Bans | `account_bans`: `account_id`, `reason`, `expires_at` where `active <> 0` and (`expires_at` null or > now in ms) | the table (before 0023): none; `expires_at` (before 0029): null, a permanent ban |
| Discord links | `discordlinks`: `discordId`, `userId`, `linkedAt`, by `discordId` | before 0017: none |
| Invites | `invitecodes`: `invitecode`, `usesRemaining`, `infiniteUses`, by code | before 0007: none |

A ban and a link attach to the account whose `userId` they name; one per account (the last read wins). `linkedAt` is
re-written as `YYYY-MM-DDTHH:MM:SS.mmmZ` UTC when it parses as RFC 3339, else it is `now`. A database error while
reading: SQLite's text containing `disk I/O error`, `database or disk is full` or `unable to open` is an I/O error;
anything else is `not a readable 1.4.4 database (<text>)`.

**Left behind.** Every other table that isn't `sqlite_*` or `__*`, isn't in `carried` (`users`, `account_bans`,
`discordlinks`, `invitecodes`) and isn't in `not_player_data` (credentials and bookkeeping: `userapikeys`,
`userapikeystoregister`, `gameserverapikeys`, `gameserverapikeystoregister`, `accountkeyrecovery`, `recoveryproofs`,
`recoverychallenges`, `recoveryevents`, `authepochs`, `onetimejobs`, `player_addresses`, `moderation_events`) is
counted; a nonzero count goes in the report's `dropped` under the table's name.

**Server import** (`revived`): every account and every invite. Notes, after the write:

- always: `1.4.4 account keys are not carried: 1.14.7 keeps keys as HMACs, so players sign in through the launcher`
- admins counted: `1.4.4 admins were imported as players: give them their roles in the admin UI`
- temporary bans made permanent counted: `this 1.14.7 database keeps no timeouts yet (rustic-isles before migration
  0610), so timed bans were imported as bans: update the server and import again, or lift them in the admin UI when
  they run out`
- n accounts read without a link: `<n> 1.4.4 account(s) had no Discord link, so their players have no way to sign in
  yet: link one from the admin UI`

## 7. An offline save

The source is a `.drsave` container when its first 8 bytes are `DRSAVE\r\n`; otherwise it is a bare offline
database.

### 7.1 The container

| Bytes | Content |
| --- | --- |
| 0–7 | `DRSAVE\r\n` |
| 8–11 | version, u32 big endian, must be 1 |
| 12–15 | metadata length m, u32 BE, 2 to 65536 |
| 16–23 | database length d, u64 BE, 512 to 536870912 |
| 24 … 24+m | metadata, UTF-8 JSON |
| … +d | the SQLite database |
| last 32 | SHA-256 of every byte before it |

The file must be a regular file of exactly 24 + m + d + 32 bytes. The metadata JSON:

```json
{"format":"dauntless-revived-offline-save","version":1,"createdAt":"2026-10-21T22:00:00.000Z",
 "account":{"userId":"UID-1","username":"Sam"},
 "database":{"bytes":4096,"sha256":"<64 lowercase hex>"},
 "credential":{"version":1,"kind":"uuk-sha256","key":"UUK_<48 lowercase hex>"}}
```

It must parse, and: `format` and `version` as shown; `createdAt` RFC 3339 and equal to itself re-written as
`YYYY-MM-DDTHH:MM:SS.mmmZ` UTC; `userId` 1 to 128 bytes of ASCII letters, digits, `_`, `-`; `username` 1 to 128
UTF-16 code units with no CR, LF or NUL; `database.bytes` = d; `database.sha256` 64 lowercase hex; `credential` as
shown. The database bytes are streamed into `offline.db` in a second private folder (`dainty-legacy-save-`), created
new, mode 0600 on Unix; then the trailer must equal the running SHA-256 and the database's own SHA-256 must equal
`database.sha256`. Any failure: `not a Dauntless Revived offline save (.drsave), or a damaged one`. Nothing names a
path or a file name inside the container. The key never reaches a `Debug`, a log line or an error.

### 7.2 Whose account

After sections 5 and 6 on the extracted (or bare) database:

- **A container:** the rows of `userapikeys` joined to `users` on `userId` whose `keyHash` is the lowercase hex
  SHA-256 of the credential's key, as UTF-8. Exactly one row, whose `userId` and `name` equal the metadata's
  `userId` and `username`. And every one of the `cleared` tables that exists (`userapikeystoregister`,
  `gameserverapikeystoregister`, `gameserverapikeys`, `accountkeyrecovery`, `recoveryproofs`, `recoverychallenges`)
  must be empty. A prefix before 0006 has no `userapikeys`, so the save is refused.
- **A bare database:** `users` has exactly one row.

Else `not a Dauntless Revived 1.4.4 offline save: it doesn't hold one player`. Only that account is carried; no
invites. The notes are section 6's first three, then, when it has no link: `the save's account had no Discord link,
so its player has no way to sign in yet: link one from the admin UI`.

## 8. The report

Serialised for the console's `data` as JSON with these fields, in this order: `source`, `dry_run`, `imported`
(name → count), `unchanged`, `skipped` (`kind`, `id`, `reason`), `dropped`, `notes`. Maps are ordered by key.

The text:

```text
<source> import[ (dry run: nothing written)]:
  imported <what>: <n>          one per imported entry
  already there <what>: <n>     one per unchanged entry
  nothing to import             only when both are empty
  not carried to 1.14.7, <table>: <n> row(s)
skipped (<count>):
  <kind> <id>: <reason>
note: <note>
```

A report never holds a key, a hash, a password or a secret's value.

## 9. Errors

Four kinds: invalid source (text as given), target (`target database: <text>`), I/O (`<path>: <error>` where there
is a path), usage (text, a newline, the usage). From the console, invalid and usage are the module error kind
`Invalid`, target and I/O are `Failed`. No message holds a value read from the data other than the ids and names a
report holds.

## 10. The plugin

Manifest: id `legacy`, kind plugin, display name `Legacy import`, game version `1.14.7`, no ports, no invite
aliases, default capabilities. It serves nothing; `start` and `stop` only set its phase (Ready, Stopped).

Commands, with their argument descriptions:

| Command | Description | Arguments |
| --- | --- | --- |
| `import-revived` | `Import a Dauntless Revived 1.4.4 server's accounts` | `database` (required): `The 1.4.4 metagame's SQLite database (absolute path); never written`; `target` (required); `options`: `--node <id>, --dry-run` |
| `import-offline-save` | `Import a player's Dauntless Revived 1.4.4 offline save` | `save` (required): `The .drsave file or the bare offline database (absolute path)`; `target`; `options` as above |
| `import-reforged` | `Upgrade a Reforged Revival Node-kit install to Dainty Lass` | `checkout` (required): `The old install's folder (absolute path)`; `options`: `--to/--env-file, --fresh-start, --db-tool <program>, --dry-run` |

`target` reads `--to <1.14.7 database> or --env-file <the 1.14.7 backend's env file>`. An unknown command is
unsupported. **Only the console may run one**: any other actor gets `Invalid`,
`an import reads the host's files: run it from the host's console`. The output is the report's text and its JSON.

## 11. The Reforged Node-kit upgrade

`root` is the old checkout. Paths: `.workspace/config/host.json`, `settings.toml` beside it, `.workspace/mongodb`.

Order, each step before the next so a failure leaves the rest as it was:

1. Read `host.json` if it is there: not a file, or over 262144 bytes, is
   `<path>: not a host.json of at most 262144 bytes`. Neither it nor `settings.toml`:
   `<root>: no .workspace/config/host.json or settings.toml, so not a Reforged install`.
2. With `host.json` and no `settings.toml`, convert it now (section 11.2), so a bad file changes nothing.
3. MongoDB (11.5). 4. Folders (11.3). 5. `secrets.env` (11.4).
6. Write `settings.toml` (unless a dry run): write `settings.toml.partial`, rename it over `settings.toml`, delete an
   old `host.json.bak`, rename `host.json` to `host.json.bak`. Count `settings.toml`; note
   `host.json is now settings.toml; the old file is kept as host.json.bak`; with keys dropped, note
   `left out of settings.toml (1.4.4, 1.12 or unknown): <comma-joined>`; with `edition` `"legacy"`, note
   `this server ran only Dauntless 1.4.4: set game_dir in settings.toml to a 1.14.7 install before setup`.
   `settings.toml` already there: `unchanged settings.toml`, and if `host.json` is too, note
   `settings.toml is already there, so host.json was left as it is: delete it once you've checked`.
7. For each old folder still there (`.workspace/Mystic-Paradox`, `paradox`, `native/runtime`,
   `native/catalog-exporter`), note `<relative path> is no longer used: delete it once you've checked what moved`.
8. Note `next: run dainty-host setup, which checks settings.toml and writes the services' settings`.

### 11.2 `host.json` to `settings.toml`

Not JSON: `host.json is not valid JSON: <error>`; not an object: `host.json must hold a JSON object`.

- Remove `retired_keys`, each one present going to `dropped` by its JSON name.
- In `modules`: remove `retired_modules` ids from the `compiled` list (`modules.compiled: <id>` dropped) and entries
  of `external` whose `id` is retired (`modules.external: <id>`); a list left empty is removed, so the default
  applies; remove a `modules.<retired id>` table (`modules.<id>`).
- Any other top-level key not in `top` is dropped by name; a key inside a known table not in its `keys` list is
  dropped as `<toml table>.<json key>`.

The output: the header (Appendix B), then every non-table top key present and not null as `toml_name = value`, in
`top` order; then each table present as `[toml_name]` with its known keys present and not null, in `keys`
order, lists written inline. `network.nodes` is written last in its table as an array of tables
(`[[network.nodes]]`). `modules` goes last, converted freely: an object is a table (a table holding only tables gets
no header of its own), a nonempty list of objects an array of tables, any other list an inline array. A value is a
string, a bool, an integer, or a float; anything else (null inside a list, a list inside a list, an object where a
single value goes) is `<key> must be a single value`, and a number neither integer nor float is
`<key>: <n> is not a number settings.toml can hold`.

### 11.3 Folders

For each of `moves` in order, `<sdk>` expanded to each of `sdk_files`: skip when
the source is missing or equals the destination; when the destination exists, skip `folder` `<relative source>`,
`<relative destination> is already there`; else (unless a dry run) create the destination's parent and rename.
Count `folders moved` and note `moved <relative source> -> <relative destination>`. Nothing is overwritten or deleted.

### 11.4 `secrets.env`

Carried untouched; read only for the names with a non-empty value (`NAME=value` lines, an optional `export `, quotes
trimmed). Missing: note `there is no secrets.env: dainty-host setup makes a new one, which logs every player out`.
Present: `unchanged secrets.env`. Of the host kit's `SECRET_KEYS`, a name present only under its old name (Appendix
B, `secret_renames`) is listed in `secrets.env keeps the old names <old names>; dainty-host setup moves each to its
new name, same value`; a name present under neither in `secrets.env is kept; dainty-host setup adds what it lacks:
<names>`. On Unix, unless a dry run, the file is made mode 0600. A value is never read into a message.

### 11.5 MongoDB

The marker is `.workspace/config/.mongo-migrated`. It exists: `unchanged mongodb`, done. `.workspace/mongodb` holds
nothing but `mongod.lock` (or doesn't exist): done. The URI is `backend.env`'s `MONGODB_URI`, else `host.json`'s
`mongo.uri` trimmed, else `mongodb://127.0.0.1:27017`; its database is the URI's path if it has one, else
`backend.env`'s `MONGODB_DB`, else `reforged`. Every message shows it redacted: a `user:pass@` becomes `user:***@`,
a bare `user@` `***@`.

- **No target, no `--fresh-start`:** usage error `this install has MongoDB data (<uri>): give the 1.14.7 database to
  move it into (--to), or --fresh-start to start on a new one and keep MongoDB as it is`.
- **`--fresh-start`:** write the marker `fresh-start\n`; note `MongoDB (<uri>) was not moved (--fresh-start); its
  files are kept`.
- **A target:** on a dry run note `MongoDB (<uri>) would be moved with dainty-host db migrate and verify`. Else run
  `<db tool> db migrate --from <uri> --to <to> --json [--allow-remote-postgres]`, then `db verify --against <uri>
  --to <to> --json [...]`. `<to>` is the SQLite path, or `postgres://user[:password]@host:port/database` with the
  flag when the target allows remote. A failed migrate: `dainty-host db migrate from <uri> failed (<status>): is
  MongoDB running?`. Verify's stdout JSON lists `collections` with `name` and `ok`; not JSON is
  `dainty-host db verify gave no report (<status>)`; collections not ok:
  `dainty-host db verify found differences in [<names>]: nothing else of the old install was changed, and MongoDB was
  not modified`; a failed status with none listed: `dainty-host db verify failed (<status>)`. Success writes the
  marker `migrated from <uri>\n`, counts `mongodb`, notes `MongoDB (<uri>) moved and verified; its files are kept`.

## 12. Security

The checklist moves to `docs/security/legacy-import.md` (RX3), with today's items: the source is never written; a
private copy; `trusted_schema` off before any read; the history and object checks before any row is read; bounded
`.drsave` reads; no path from inside a container; only the console runs an import; no secret in a message, a report
or a `Debug`; `secrets.env` values never read into memory beyond the line split; the MongoDB URI redacted.

Fuzz targets in `fuzz/`: `legacy_database_readers` (arbitrary bytes as a server database, an offline database and a
`.drsave`; none may panic or write outside its temporary folder) and `legacy_host_json` (arbitrary text through
section 11.2).

## 13. Tests and the oracle

MIT tests build their 1.4.4 fixtures from this page: tables with the columns of section 6 for a chosen prefix, and a
`__drizzle_migrations` history written from `revived-1.4.4.toml`. Cases: every prefix boundary in Appendix A's
`[[column]]` list; Drizzle and Diesel histories; a wrong hash, a wrong time, a too-long history; an unknown table,
trigger or view; journal files beside an offline save; a WAL server copy; every skip and count of section 4; a
target with and without `0610`; a dry run that writes nothing; every `.drsave` damage (each header field, one byte of
JSON, one byte of the database, the trailer); a key that matches no row and a non-empty cleared table.

**The oracle** stays in `restricted/legacy` (AGPL) until the maintainer retires it: it replays the real frozen SQL
at each prefix 1 to 31 and the Diesel baseline, fills them, and asserts the MIT importer's report equals today's.
It also asserts each `sha256_lf` in `revived-1.4.4.toml` is the frozen file's. After the switch, `restricted/legacy`
holds only that test, the SQL and its NOTICE; deleting it later leaves no AGPL code for the importer.

---

## Appendix A: `revived-1.4.4.toml`

```toml
# The Dauntless Revived 1.4.4 metagame's migration history, as facts (spec-legacy-import.md, section 2.1)

# Every table and trigger any 1.4.4 migration leaves behind
tables = [
  "account_bans", "account_device_signals", "accountkeyrecovery", "authepochs", "blocks", "bounties", "bountydraft",
  "breadcrumbs", "characterhistory", "characters", "cooldowns", "device_ban_signals", "discordlinks",
  "encounteredcontent", "entitlements", "escalationprogression", "escalationtalents", "escalationunlocks",
  "friendships", "gameserverapikeys", "gameserverapikeystoregister", "guildinvites", "guildmembers", "guilds",
  "huntpassselection", "huntregions", "inventories", "inventorylog", "inventorytransactions", "invitecodes",
  "leaderboardprofiles", "loadouthistory", "loadouts", "loadoutslots", "moderation_events", "objectives",
  "onetimejobs", "player_addresses", "progress_tracks", "progression_events", "recoverychallenges", "recoveryevents",
  "recoveryproofs", "slayerlinkinvites", "slayerlinks", "storepurchases", "trialruns", "trialweeks", "userapikeys",
  "userapikeystoregister", "users",
]
triggers = ["inventorylog_no_update", "inventorylog_no_delete", "progression_events_no_update",
  "progression_events_no_delete"]

carried = ["users", "account_bans", "discordlinks", "invitecodes"]
not_player_data = [
  "userapikeys", "userapikeystoregister", "gameserverapikeys", "gameserverapikeystoregister", "accountkeyrecovery",
  "recoveryproofs", "recoverychallenges", "recoveryevents", "authepochs", "onetimejobs", "player_addresses",
  "moderation_events",
]
cleared = [
  "userapikeystoregister", "gameserverapikeystoregister", "gameserverapikeys", "accountkeyrecovery", "recoveryproofs",
  "recoverychallenges",
]

[diesel]
versions = ["0001"]

[[drizzle]]
tag = "0000_natural_korg"
when = 1782098648901
sha256_lf = "9f62d381d9f555788f610c6504ce3e9ebb3bc8dd84c2a961c4ec03289c078f86"
sha256_crlf = "79094d02bf5730a3f13458684957a07b17c44ef2428de20d662afc917cdb3b1a"
[[drizzle]]
tag = "0001_slow_storm"
when = 1782160527617
sha256_lf = "ead368ca0fc85f9df5516b576d298fed5a59ef5884fd54bf43ff8a05ef1b333c"
sha256_crlf = "60e1cf6235a051ec5fd366e5ed13577f8a727179a64ddc38fecc76d07ee25113"
[[drizzle]]
tag = "0002_gorgeous_saracen"
when = 1782190471658
sha256_lf = "043c49eb990a78f3d1233183b94c21486e87096112dcd97dad6cb58c41a3efe4"
sha256_crlf = "1a58b320049a6935dfc06726727bf1c720fa43f3ce56620fe6a93884bdc4fa3d"
[[drizzle]]
tag = "0003_slippery_blackheart"
when = 1782580106809
sha256_lf = "0544e75878705b030a86624027e69635b3a30c1cb4795594fc7f257d5bf74060"
sha256_crlf = "0f5fdc192e68f06d72be72c9b19b7f0b827622a4903cd21bdf2ee586c5ee2f79"
[[drizzle]]
tag = "0004_silky_lady_mastermind"
when = 1782581556194
sha256_lf = "be055ce0a63e1a3ca7d7545432ed5ba92dcc68332925e288398a0f165549a550"
sha256_crlf = "be055ce0a63e1a3ca7d7545432ed5ba92dcc68332925e288398a0f165549a550"
[[drizzle]]
tag = "0005_mighty_zeigeist"
when = 1782586138792
sha256_lf = "00c5e606c6969b3eb0fac14cbb6f36f07737afe7a477ecca153f14bfed57bf04"
sha256_crlf = "9e4ce82c8388c27f7b76811427020426430a5d74e448f449f8b1946e82f93275"
[[drizzle]]
tag = "0006_woozy_slipstream"
when = 1782664386398
sha256_lf = "06440ff2cdb5681d666c10d66b871fa2629e00ecc3ed27e12b8865a8eacf3a2f"
sha256_crlf = "b40fd260056b844aae35d492712ebf639b4e1196e38baf2f90ede2b6da4071d9"
[[drizzle]]
tag = "0007_last_maximus"
when = 1783236454860
sha256_lf = "45d775ee6b347c550b969557a8cff077d690acb6d3830850e0b6e57b97b412c0"
sha256_crlf = "fb742189b05cfced33f741ae5bace8a13fb03270591b6a745243e3dd0b1e5027"
[[drizzle]]
tag = "0008_vengeful_spirit"
when = 1783285636434
sha256_lf = "57e80c480d0cda97928716bb767ad66fb4d132d38d74334df8c3955c1b7b5f10"
sha256_crlf = "57e80c480d0cda97928716bb767ad66fb4d132d38d74334df8c3955c1b7b5f10"
[[drizzle]]
tag = "0009_nice_dazzler"
when = 1783299942758
sha256_lf = "e28538a731058ee2dfe01f1d6cf6892c782cb0defe84a17e180baa10ddabc69e"
sha256_crlf = "63020e6a8affced39263ba0815d11f98622de2c40ecb03aa2b55eaa5ebbf944b"
[[drizzle]]
tag = "0010_save_history_and_item_log"
when = 1789991294333
sha256_lf = "2376503a7aa16ace58087b8525e928fcfa83af3738d120d30c9b64995bd18f57"
sha256_crlf = "ace5ee936839556276d40742e63278d51d0d084af894aa19d4d91e4ef3eaf06c"
[[drizzle]]
tag = "0011_real_progression"
when = 1789993643640
sha256_lf = "3f8aa53a838638df5d871d57b878db490acb02a3418f511a86c701075b9602d4"
sha256_crlf = "f6b47762b562c9cc556877aaf56ae21bf825ae0006a891a2fee1326ec0738eba"
[[drizzle]]
tag = "0012_friends_and_blocks"
when = 1790005698166
sha256_lf = "27a390ec953be0c889496e6bfe319668aed4943862c7dc7ab5673e731b9cd660"
sha256_crlf = "e931e66c916bc51f365401c022238012e4ba074b5f3af326d7e11b94d9256c55"
[[drizzle]]
tag = "0013_guilds"
when = 1790048284578
sha256_lf = "07f3510e595be9e6b53e0970081fa3c40278d18b0c695f151bc11e2b529edc25"
sha256_crlf = "0084b64c42d38b59fdd7afc814526a3d9f793b067322bf2d4e3fb0ef531b83a7"
[[drizzle]]
tag = "0014_escalation"
when = 1790160221950
sha256_lf = "4eed5a85fed8296ac842c4fd9af05d4bc5f207105a8f39e7be7f8914c1289f43"
sha256_crlf = "c034fcf1ca22e33c7ab0e9c14989b8f2cec7442f27c320ecb125558215f554a6"
[[drizzle]]
tag = "0015_store_purchases"
when = 1790161221759
sha256_lf = "f10ea816686051d7eaec5300d4cdc7f1d14667256f266ffe283f51e74d062434"
sha256_crlf = "5f1d0fee1c660039147a0a76edc5b3cc4a613551f9fcb0aa2b73b4f004d89636"
[[drizzle]]
tag = "0016_slayer_links"
when = 1790163279299
sha256_lf = "a51d773dd89f6882a0f538de303d238b1f5c2b18f88e39f238fbc8610167d332"
sha256_crlf = "5f52e3ef196c5f6d2b966cdef2bd66584991ba7228240f0c12c7ae4ee0ad915b"
[[drizzle]]
tag = "0017_discord_links"
when = 1791000000000
sha256_lf = "9f4f2ec274c9de8b26885645976cdfe3723462621f6d2d2f71bc39696359e4ae"
sha256_crlf = "0a87e779c8fbc0358e6d115efca1aa2aa4c242625ab907a831ddaaed1ec5c4f1"
[[drizzle]]
tag = "0018_trials_leaderboards"
when = 1791266400000
sha256_lf = "8cf487e73e1f39e4d19e4d12495ce1def9762b35034c0dc3a0a74b277dc198f0"
sha256_crlf = "52146123869bf3ba848b2f11312aa598dd652ce3a9725b70cc739533186ded10"
[[drizzle]]
tag = "0019_trials_profiles"
when = 1791309600000
sha256_lf = "736b7500beb92fa45b7a1278ca40afeb58ac9e9002bb6f6e0287c0ecf347280b"
sha256_crlf = "b722ecd07bdaf232bae8ed8cee44f95548153e501123d36d5fcc2557b7a6f3f9"
[[drizzle]]
tag = "0020_hunt_regions"
when = 1791400000000
sha256_lf = "3ff300ef02d53220f3d864700a3fbccb56bc6be86b5f0dce88b7f367323ea3b6"
sha256_crlf = "d0af34ad4c880263aa01bd10be901a03ce632d40cdcacc5738432bf46d3fa006"
[[drizzle]]
tag = "0021_onetime_jobs"
when = 1791500000000
sha256_lf = "3e1df55535cba3fcf7649e65c1a87de26066bbbd95b21d3039939ee2a613a2dd"
sha256_crlf = "15280502743d3c966bdc1a7b949358e528562ac0fef54317553df4cb86f8624d"
[[drizzle]]
tag = "0022_germany_region"
when = 1791600000000
sha256_lf = "103082ee0ab5372f7dd3eded1344f0aaffd0fd9dd7d7ae901cd39d4fd8903118"
sha256_crlf = "14cb1e55bfbb5253c3f17dcaa805a0105f189b88d325095a0491c7c580ba3684"
[[drizzle]]
tag = "0023_moderation"
when = 1791700000000
sha256_lf = "683ea9c6c0198487048a3e6214976911dd147c90ad011f9b84ada544516e9742"
sha256_crlf = "58415ed1a095e295269ac647f99614b27c90610fd0255238e2850956ed3f0539"
[[drizzle]]
tag = "0024_ephemeral_addresses"
when = 1791800000000
sha256_lf = "751b39f3335004c9b751538c323406bfc2de1d294032a22f9aa178f264d30e04"
sha256_crlf = "ebd22ebe54c1c6ee030e2b62b228c6fb35213dbe5bc8b12d1cd629356b5f0b76"
[[drizzle]]
tag = "0025_inventory_expiry_index"
when = 1791900000000
sha256_lf = "2d0d6744b64adadca2de23697148d8c801e254f285073f48f8e8f0b2c75a3fd4"
sha256_crlf = "cfb053967bca92ae8b68c75cc6657cfa478643c9b76f896025c8e0e169e1429d"
[[drizzle]]
tag = "0026_account_key_recovery"
when = 1792000000000
sha256_lf = "92a003359fdddaaccb45b54d8c21bca077080b51f972941b4d4c83ba131fb398"
sha256_crlf = "a62c04886163a84c8a65c406b50efa9420d58c1ab3402d82424d0a8f7a41184d"
[[drizzle]]
tag = "0027_player_key_rotation"
when = 1792100000000
sha256_lf = "2318e9ceac9ec2f02504adf8c6fb929ed0b795e86932f7a92d41dc9d58af0d7a"
sha256_crlf = "fc2791dc141a027ec91480e415a06c4c9d6eabc337e9405ff0357fec9480c396"
[[drizzle]]
tag = "0028_us_region"
when = 1792200000000
sha256_lf = "4fc24b136db56ca3a8ea573251277d624024f73fe428a68bc8bbee3162710b50"
sha256_crlf = "3d4c67b5f7df03d6325b56300ec7e7d92d1867634b4bbf423307623ce059696e"
[[drizzle]]
tag = "0029_temporary_bans"
when = 1792200000001
sha256_lf = "ae310e8e570c3eea68294959f2d7c4ad674b04c61d1bcd159559f0dbf974379a"
sha256_crlf = "3b69da6212db32fb90949f8452a65f01529ee65f9276bbe33d1e83d7bbe2c8f6"
[[drizzle]]
tag = "0030_device_signals"
when = 1792200000002
sha256_lf = "ec8542c0676510d7090b7f5f9a4301757eea3581e686d599191efd232fadcdc7"
sha256_crlf = "41aebc843dff8455426945f6f22dfd4c090b11b32e728c2dc12fa8599c4afb3e"

[[column]]
table = "users"
name = "userId"
since = 0
[[column]]
table = "users"
name = "name"
since = 0
[[column]]
table = "users"
name = "notes"
since = 0
[[column]]
table = "users"
name = "isAdmin"
since = 8
[[column]]
table = "userapikeys"
name = "userId"
since = 6
[[column]]
table = "userapikeys"
name = "keyHash"
since = 6
[[column]]
table = "invitecodes"
name = "invitecode"
since = 7
[[column]]
table = "invitecodes"
name = "usesRemaining"
since = 7
[[column]]
table = "invitecodes"
name = "infiniteUses"
since = 7
[[column]]
table = "discordlinks"
name = "discordId"
since = 17
[[column]]
table = "discordlinks"
name = "userId"
since = 17
[[column]]
table = "discordlinks"
name = "linkedAt"
since = 17
[[column]]
table = "account_bans"
name = "account_id"
since = 23
[[column]]
table = "account_bans"
name = "reason"
since = 23
[[column]]
table = "account_bans"
name = "active"
since = 23
[[column]]
table = "account_bans"
name = "expires_at"
since = 29

```

## Appendix B: `reforged.toml`

```toml
# The Node kit's host.json keys and old folders, for the reforged upgrade (spec-legacy-import.md, section 11)

retired_keys = ["edition", "legacyGameDir", "legacyPublicPort"]
retired_modules = ["revived-1-4-4", "isles-1-12-0"]

header = """
# This server's settings (dainty-host), moved here from the Node kit's host.json by dainty-legacy. Edit them
# here, then run `dainty-host setup` again to apply them; setup keeps your comments and every value it does not
# change. A setting that is left out takes its default. The secrets are not here: they are in secrets.env.
"""

top = [
  ["version", "version"], ["mode", "mode"], ["host", "host"], ["gameAddress", "game_address"], ["name", "name"],
  ["gameDir", "game_dir"], ["registration", "registration"], ["shareUrl", "share_url"], ["mongo", "mongo"],
  ["maxHuntServers", "max_hunt_servers"], ["multipliers", "multipliers"], ["publicPort", "public_port"],
  ["performance", "performance"], ["linux", "linux"], ["database", "database"], ["guard", "guard"],
  ["history", "history"], ["switches", "switches"], ["network", "network"], ["remote", "remote"],
  ["modules", "modules"],
]
# The host kit's SECRET_KEYS, and the old name each was written under
secret_keys = [
  "GATEWAY_SECRET", "ALLOWLIST_SECRET", "DAINTY_ADMIN_KEY", "GAMESERVER_API_KEY", "API_KEY_HASH_SECRET",
  "RUSTIC_ISLES_METRICS_TOKEN", "AUTH_SIGNING_PRIVKEY_B64", "AUTH_SIGNING_PUBKEY_B64", "DAINTY_ADMIN_API_KEY",
  "RUSTIC_ISLES_MANAGER_OPERATOR_SECRET", "DAINTY_DB_PASSWORD",
]
# Folder moves, in order: the pre-submodule layout, then the Node kit's upstream clone. `<sdk>` expands to each of
# SDK.hpp, Assertions.inl, NameCollisions.inl, PropertyFixup.hpp, UnrealContainers.hpp, UtfN.hpp
sdk_files = ["SDK.hpp", "Assertions.inl", "NameCollisions.inl", "PropertyFixup.hpp", "UnrealContainers.hpp", "UtfN.hpp"]
moves = [
  ["native/runtime/<sdk>", "modules/rustic-isles/native/runtime/<sdk>"],
  ["native/runtime/SDK", "modules/rustic-isles/native/runtime/SDK"],
  ["native/runtime/x64", "modules/rustic-isles/native/runtime/x64"],
  ["native/catalog-exporter/x64", "modules/rustic-isles/native/catalog-exporter/x64"],
  ["paradox/Items_Analysis", "modules/rustic-isles/reference/1.14.7/Items_Analysis"],
  ["paradox/ParadoxBackend/game-data", "modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data"],
  ["paradox/ParadoxDirector/game-data", "modules/rustic-isles/reference/1.14.7/ParadoxDirector/game-data"],
  [".workspace/Mystic-Paradox/MysticParadox/<sdk>", "modules/rustic-isles/native/runtime/<sdk>"],
  [".workspace/Mystic-Paradox/MysticParadox/SDK", "modules/rustic-isles/native/runtime/SDK"],
  [".workspace/Mystic-Paradox/MysticParadox/x64", "modules/rustic-isles/native/runtime/x64"],
  [".workspace/Mystic-Paradox/CatalogExporter/x64", "modules/rustic-isles/native/catalog-exporter/x64"],
  [".workspace/Mystic-Paradox/Items_Analysis", "modules/rustic-isles/reference/1.14.7/Items_Analysis"],
  [".workspace/Mystic-Paradox/ParadoxBackend/game-data", "modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data"],
  [".workspace/Mystic-Paradox/ParadoxDirector/game-data", "modules/rustic-isles/reference/1.14.7/ParadoxDirector/game-data"],
]
old_folders = [".workspace/Mystic-Paradox", "paradox", "native/runtime", "native/catalog-exporter"]

# `modules` is free-form; these are the tables, each [json, toml] in settings.toml's order
[keys]
mongo = [["uri", "uri"], ["manage", "manage"], ["mongod", "mongod"]]
multipliers = [["xp", "xp"], ["rewards", "rewards"]]
performance = [
  ["serverFps", "server_fps"], ["idleFps", "idle_fps"], ["trainingOnDemand", "training_on_demand"],
  ["trainingIdleMinutes", "training_idle_minutes"], ["verboseGameLogs", "verbose_game_logs"],
  ["memoryGuard", "memory_guard"], ["minFreeMemoryMb", "min_free_memory_mb"], ["huntMemoryMb", "hunt_memory_mb"],
]
linux = [["wine", "wine"], ["proton", "proton"], ["xvfb", "xvfb"], ["firewallInterface", "firewall_interface"]]
database = [
  ["engine", "engine"], ["postgresHost", "postgres_host"], ["postgresPort", "postgres_port"],
  ["postgresUser", "postgres_user"], ["postgresDatabase", "postgres_database"], ["allowRemote", "allow_remote"],
  ["freshStart", "fresh_start"],
]
guard = [
  ["banLimit", "ban_limit"], ["banWindowMinutes", "ban_window_minutes"], ["banGuard", "ban_guard"],
  ["revertLimit", "revert_limit"], ["revertGuard", "revert_guard"], ["revokeGuard", "revoke_guard"],
  ["delayMinutes", "delay_minutes"], ["expireHours", "expire_hours"],
]
history = [["shipToPeer", "ship_to_peer"]]
switches = [
  ["reforge", "reforge"], ["masteryPersistence", "mastery_persistence"], ["bountyPersistence", "bounty_persistence"],
  ["cooldownPersistence", "cooldown_persistence"], ["accountErasure", "account_erasure"], ["logPurge", "log_purge"],
  ["rateLimit", "rate_limit"], ["rateLimitPerMinute", "rate_limit_per_minute"],
  ["gameserverRateLimitPerMinute", "gameserver_rate_limit_per_minute"],
  ["hostRateLimitPerMinute", "host_rate_limit_per_minute"],
]
network = [
  ["node", "node"], ["region", "region"], ["leader", "leader"], ["wrongNodePolicy", "wrong_node_policy"],
  ["nodes", "nodes"],
]
remote = [
  ["role", "role"], ["node", "node"], ["region", "region"], ["controllerUrl", "controller_url"],
  ["controllerNode", "controller_node"], ["controllerCertFp", "controller_cert_fp"],
  ["syncIntervalSecs", "sync_interval_secs"], ["label", "label"], ["localOverrides", "local_overrides"],
]

[secret_renames]
DAINTY_ADMIN_KEY = "REFORGED_ADMIN_KEY"
DAINTY_ADMIN_API_KEY = "REFORGED_ADMIN_API_KEY"
DAINTY_DB_PASSWORD = "REFORGED_DB_PASSWORD"
RUSTIC_ISLES_METRICS_TOKEN = "MYSTICPARADOX_METRICS_TOKEN"
RUSTIC_ISLES_MANAGER_OPERATOR_SECRET = "MYSTICPARADOX_MANAGER_OPERATOR_SECRET"

```

Fields that may be null in
`host.json` (`gameDir`, `shareUrl`, `mongo.mongod`, `linux.wine`, `linux.proton`, `linux.firewallInterface`) need no
flag: every null is left out.
