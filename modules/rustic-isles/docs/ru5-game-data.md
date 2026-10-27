# RU5 game data and scripts

The `game-data` group of the route inventory: 29 routes, all ported (`crates/isles-core/src/areas/game_data.rs`),
for both modules. The payloads live in `crates/isles-core/src/game_data/` so the other groups can reach them.

| Where | What |
| --- | --- |
| `game_data/season_window.rs` | `seasonWindow.ts`: the rolling 84-day Hunt Pass window (`active_season_window_ms`, `active_season_window`, `iso_utc`, `now_ms`). Bounties, cooldowns and progression use it too. |
| `game_data/progression_config.rs` | `WidenHuntPassWindows` (`widen_huntpass_windows`, `served`), `served_huntpass_id` (`GetServedHuntPassId`), `recent_seasons`. The hunt-pass route and `GET /progression/objectives` use these. |
| `game_data/loader.rs` | `loadGameData` and `tryLoadGameData` on `GameDataRoot` (`load_backend`, `try_load_backend`, `load_backend_cached`). |
| `game_data/gauntlet.rs`, `tuning.rs`, `patchnotes.rs` | The Gauntlet stub payloads, the `/game_tuning/*` answers, the patch notes. |
| `game_data.rs` | `GameDataRoot`, the file lists (`director_files(trials_tables)`: the 1.14.7 Director also reads the two `_new` Trials tables). |

## Tests

- Ported from Node: `seasonWindow.test.ts`, `gauntletPayloads.test.ts` (unit tests beside the code), and the
  `GET /motd/trigger` test of `systemHostWrites.gamedata.test.ts` (`golden::check_edges`).
- Golden: `crates/isles-core/tests/golden/capture.mjs` runs the reference `routes/system.ts` and `routes/tuning.ts`
  handlers unchanged under Node 22 (router, logger, auth and the Mongo-backed imports stubbed, `Date.now` pinned) and
  records status, content type and body text for 92 cases of both versions into `game_data.json`. Each module's
  `tests/module.rs` requests the same cases from the Rust module over HTTP. To re-record (only when the Node side
  changed on purpose):

      docker run --rm -v "$PWD":/repo -w /repo node:22 node crates/isles-core/tests/golden/capture.mjs reference crates/isles-core/tests/golden

  The progression config it reads is `progression_config.synthetic.json`, made for the test, not a game export.

## Scripts (`reference/1.14.7/ParadoxBackend/scripts`)

Nothing in `scripts/` runs while the server serves. The modules need only what the generators write, read through
`GameDataRoot`. So no script is ported here:

| Kind | Scripts |
| --- | --- |
| Stay Node, host tooling (RB11's `gamedata` subcommands port them, byte-identical output) | `generate_progression_config.cjs`, `clean_progression_config.cjs`, `generate_slayers_path.cjs`, `generate_slayers_path_definitions.cjs`, `generate_ladyluck_store.cjs`, `enrich_ladyluck_store.ts`, `generate_reward_cache_store.ts`, `generate_platinum_store.ts`, `generate_inventory_storage_policy.ts`, `audit_tier1_catalog.cjs` |
| Mongo-only, replaced by RU7's `db migrate` | `migrate_*.ts`, `encrypt_mongo_backup.cjs`, `test_mongo_schema_migration_rehearsal*`, `ensure-build-fresh.cjs`, `run-tests.cjs` |
| Account tooling for the auth and admin groups | `bootstrap_admin.ts`, `clone_account.ts`, `credit_challenge_rewards.ts`, `process_account_deletions.ts` (the 30-day erasure sweep), `purge_uploaded_logs.ts` (the 90-day launcher log purge) |
| Rifts | `scripts/p2p/*` |

The two sweeps (`process_account_deletions.ts`, `purge_uploaded_logs.ts`) are jobs the Rust modules will need, since a
hosted server owes the Privacy Policy's retention periods; they belong to the auth and Reforged API groups.
