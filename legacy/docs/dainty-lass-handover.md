# Handing old installs over from dainty-lass

dainty-legacy now carries everything DaintyLass did for old installs. This page lists what dainty-lass deletes once
it depends on this module, what it adds to reach it, and what the two must keep in step. dainty-legacy edits nothing
in dainty-lass; the 1.4.4 removal there does these.

Paths are dainty-lass `plan` at `fb46a4a`, the rev this repository pins.

## What dainty-lass deletes

### Old `.workspace` layout (`rust/crates/hostkit`)

Moved to `crates/legacy-import/src/reforged/layout.rs`.

- `src/commands/setup/paradox.rs`: `old_workspace_items`, `pre_submodule_items`, `old_folders`,
  `migrate_old_workspace` and the test `migrate_old_workspace_moves_legacy_dirs`; the `Layout` fields `old_upstream`,
  `pre_paradox` and `pre_native` (and `revived`, with 1.4.4).
- `src/commands/setup.rs`: the `migrate_old_workspace` call at the top of `run_with` and its
  "migrated legacy folder" lines.
- `src/paths.rs`: `Paths::old_upstream` (`.workspace/Mystic-Paradox`), and `Paths::revived` with 1.4.4.

### host.json → host.toml (`rust/crates/hostkit`)

Moved to `crates/legacy-import/src/reforged/hostjson.rs` and `reforged/mod.rs`.

- `src/hostconfig.rs`: `load_host_config`'s fallback to `p.host_json`; `has_host_config`'s `p.host_json`;
  `save_host_config`'s host.json → host.json.bak rename and its `had_json_only` result; `write_host_config`'s `.json`
  branch once nothing writes host.json. `HostConfig::from_json`/`to_json` stay: they are host.toml's document form.
- `src/commands/setup.rs`: `had_host_json` and "(host.json is now host.toml; the old file is kept as host.json.bak)".
- `src/commands/update.rs`: `has_prior_old_kit_artifacts`'s host.json and host.json.bak checks.
- `src/paths.rs`: `Paths::host_json` and its test line.
- Setup should refuse a checkout that still has `.workspace/config/host.json` and no host.toml, with "run
  `reforged-host legacy import reforged <checkout>` first", instead of reading it.

### The Node kit's MongoDB

Moved to `crates/legacy-import/src/reforged/mongo.rs`, which runs rustic-isles' `isles-db migrate` and `verify` as a
child process and writes the same `.workspace/config/.mongo-migrated` marker.

- `src/commands/update.rs`: `migrate_existing_mongo_data`, `migrate_existing_mongo_data_for_setup`,
  `UpdateEnv::migrate_mongo_data` and its call in `run_with`, `has_external_mongo_evidence`,
  `has_prior_old_kit_artifacts`, `is_default_local_mongo_uri`, `mongo_uri_with_database`, `store_target_for_migration`,
  `inspect_mongo_migration`, `unmigrated_mongo_doctor_problem`, `check_unmigrated_mongo_before_start` and
  `check_unmigrated_mongo_data`, and their tests. `redact_mongo_uri` goes when nothing else uses it.
- `src/commands/setup.rs`: both MongoDB migration calls and `args.fresh_start`'s marker (`legacy import reforged
  --fresh-start` writes it now).
- `src/commands/db.rs`: `resolve_mongo_uri`, and the `db migrate` and `db verify` subcommands. **Maintainer's call:**
  `db export-mongo` (the rollback to MongoDB) is the only tool left that writes MongoDB; keep it for one release or
  move it here.
- `src/doctor.rs` and `start`: the unmigrated-MongoDB check. `legacy import reforged` refuses to convert host.json
  while MongoDB has data it hasn't moved, so a DaintyLass install never has unmigrated data.
- `src/hooks.rs` and `rust/services/host/src/lib.rs`: `mongo_has_documents`, `db_migrate` and `db_verify`, which link
  `isles-db` into the host kit. `src/mongo.rs`, `src/mongo/wire.rs` and `[mongo]` in host.toml go if the managed
  `mongod` serves nothing else.

## What dainty-lass adds

- `reforged-host legacy import <revived|offline-save|reforged> ...`: while DaintyLass runs, `module run legacy
  import-<kind> <args>` (the console only: the plugin refuses an admin API caller). Before it first runs (a Reforged
  upgrade), the executable directly: `dainty-legacy import reforged <checkout> ...`. The arguments are the same; see
  `crates/legacy-import/src/cli.rs` or the README.
- An external module entry for `legacy` (`[[modules.external]]`), loaded after `isles-1-14-7`: a plugin needs the game
  module of its version loaded first (`controller/src/registry.rs`, `NoGameFor`).
- `--env-file` points at the 1.14.7 backend's env file setup writes; nothing else is needed for the target.

## Kept in step

- **host.toml's keys.** `reforged/hostjson.rs`'s `TOP` and table lists are `hosttoml.rs`'s at `fb46a4a` without
  `edition`, `legacyGameDir` and `legacyPublicPort`. A key dainty-lass removes (such as `[mongo]`) or renames must
  leave the converter in the same change, or setup refuses the host.toml it writes.
- **isles' tables.** `target.rs` writes `accounts`, `launcher_accounts`, `auth_identities` and `reforged_invites` as
  rustic-isles `af9c125` makes them, and puts back isles-core's soft-delete set (`db.rs`). A change to either there is
  a change here.
- **Account ids.** `target::account_id` is `isles-db`'s `deterministic_account_id`; if isles-db changes its
  derivation, change both.
- **The pinned rev.** `module-api` and `store` come from dainty-lass at `fb46a4a`; bump with the rest of the modules.
