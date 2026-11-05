<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the legacy importer

The checklist for dainty-legacy's importers, `crates/legacy-import` and the `legacy` plugin in `crates/legacy`,
in the shape of DaintyLass's `docs/security/*.md` (RX3).

How to use it: before a release, check each item in the code (not from memory), link the code or test that shows it,
and sign at the end with the date and the commit reviewed. An item that does not hold is a bug to fix before the
release, not a box to leave open.

## What it is and where the boundary is

An import reads files a host was handed or kept from an old install and writes accounts into a live 1.14.7 server.
Whoever made the file chose every byte of it: a player's offline save in particular comes from the player.

- **In, untrusted:** a 1.4.4 metagame database (SQLite), a 1.4.4 offline save (`.drsave` container or bare SQLite),
  and an old checkout's `host.json`, `secrets.env`, `backend.env` and folders.
- **In, trusted:** the command's arguments, from the host's console or the host's own command line.
- **Out:** rows in the 1.14.7 database (`accounts`, `launcher_accounts`, `auth_identities`, `reforged_invites`), files
  moved inside the old checkout, `host.toml`, the `.mongo-migrated` marker, and `isles-db` run as a child process.
- **Process:** the plugin runs as its own process (DD5); stdout is the frame pipe, so it never prints there.

Code: `revived/source.rs` (copy and bring forward), `revived/read.rs`, `revived/drsave.rs`, `target.rs`,
`reforged/*.rs`, `cli.rs`, and `crates/legacy/src/lib.rs` (who may run a command).

Fuzz targets: `database_readers` (arbitrary bytes as a server database, an offline database and a `.drsave`) and
`host_json` (arbitrary text as a host.json), in `fuzz/`.

## Checklist

### Who can run an import

- [ ] A module command runs only for `Actor::Console`; an admin API caller or a player is refused before any argument
      is read (`LegacyPlugin::command`; test `commands_compiled_in`, `commands_out_of_process`).
- [ ] Every path argument is absolute; a relative one is refused (`cli::parse`; test `cli::tests::arguments`).
- [ ] Unknown options, a second source or a second target are refused, not ignored.

### The source is never written, and is untrusted until compared

- [ ] A server database is copied with SQLite's online backup into a private temporary folder; an offline database is
      copied byte for byte after refusing journal files beside it. Only the copy is opened for writing
      (`source::prepare`; test `a_drizzle_server_is_imported_once` compares the source's bytes before and after).
- [ ] `trusted_schema = OFF` on the copy before anything else, so none of its views, triggers' functions or
      generated columns run code the file chose.
- [ ] The bookkeeping tables must be exactly what Drizzle and Diesel make before their rows are read.
- [ ] The history must be a nonempty prefix of the frozen one (Drizzle hashes and `created_at`, Diesel versions), and
      the schema exactly what that prefix makes; a table, index or trigger of its own is refused (test
      `a_tampered_or_foreign_database_is_refused`).
- [ ] After the bring-forward: the final 1.4.4 schema, `integrity_check` ok, Diesel's table present.
- [ ] DaintyLass's change log (`__store_*`) is dropped from the copy before comparing, never from the source.
- [ ] A `.drsave`: magic, version, sizes, metadata rules, the database's SHA-256 and the trailer digest all checked;
      the payload bounded to 512 MiB and the metadata to 64 KiB; nothing named in the file is used as a path
      (`drsave::read_save`; tests in `drsave.rs`).
- [ ] An offline save holds exactly one player: the one whose key hash is the save's credential and whom the metadata
      names, with none of the credentials an export clears (test `a_save_that_isn_t_its_player_s_is_refused`).

### What crosses into 1.14.7

- [ ] Only accounts, active bans (a timed one as a timeout where the target has migration `0610`), Discord links and
      invite codes are written; characters, inventory and progression
      are counted, not carried.
- [ ] No key crosses: 1.4.4 keeps SHA-256 hashes, 1.14.7 HMACs; `user_api_keys` is never written (test
      `check_first_import`).
- [ ] 1.4.4 admins are imported as players (`roles = ["user"]`); no role is ever granted by an import.
- [ ] A row that would take over something already there is skipped and reported, never overwritten: a userId, a
      display name (live, case-folded) or a Discord account another 1.14.7 account has (test
      `conflicts_are_skipped_and_reported`).
- [ ] Everything is one transaction under the change-log actor `host:legacy-import`, so it can be reverted; a dry run
      rolls the same transaction back (`Target::import`).
- [ ] The target is checked before it is opened for writing: a SQLite file must exist and carry isles' migrations
      `0001`, `0020` and `0200`, read-only (test `a_database_isles_did_not_make_is_refused_as_a_target`). The
      triggers put back are isles' own soft-delete set.

### Secrets

- [ ] Errors and reports carry no key, hash, password or row value: `ImportError`'s conversions keep SQLite's words
      only, and the `.drsave` credential has a `Debug` that leaves the key out.
- [ ] `secrets.env` is read for names only and kept byte for byte; on Unix it is made `0600` (test
      `only_names_are_read_and_reported`).
- [ ] A MongoDB URI's password is redacted in every message, the marker and the report (test `an_old_kit_is_upgraded`).
- [ ] An env file given as `--env-file` may hold the Postgres password: `PostgresConfig`'s `Debug` leaves it out
      (test `an_env_file_gives_the_backend_s_database`).
- [ ] Known gap: `isles-db` takes the MongoDB URI and a Postgres target on its command line, where another user of
      the machine can read them while it runs. Run the Reforged upgrade on a machine only the host uses.

### The old checkout

- [ ] Nothing is deleted. Folders move only when the destination is free; host.json is renamed to host.json.bak only
      after host.toml was written in full (`.partial`, then rename).
- [ ] host.json is at most 256 KiB, must be a JSON object, and is converted before anything moves, so a bad one
      changes nothing (test `a_folder_that_is_no_install_is_refused`).
- [ ] MongoDB is moved first, and a failed `isles-db migrate` or a difference in `isles-db verify` stops the upgrade
      with nothing else changed (test `nothing_changes_when_mongodb_is_not_moved`).

## Sign-off

Reviewed by: \_\_\_\_\_\_ on \_\_\_\_\_\_ at commit \_\_\_\_\_\_.
