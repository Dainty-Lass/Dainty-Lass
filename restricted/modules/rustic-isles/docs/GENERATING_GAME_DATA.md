# Generating game data

Rustic Isles does not distribute Phoenix Labs game data. Extract the required material from your
own lawful Dauntless 1.14.7 installation and generate the service payloads locally.

> [!NOTE]
> This guide is for Dauntless 1.14.7, which `main` targets. For 1.12.0, follow the guide on the
> [`dauntless-1.12.0`](https://github.com/pranav158/Mystic-Paradox/tree/dauntless-1.12.0) tag.

Synthetic *.example.json files only allow build and startup smoke tests. They are not playable.

## Final required files

Backend reads these files from reference/1.14.7/ParadoxBackend/game-data:

| File | Generated from |
|---|---|
| slayers_path.json | CatalogExporter `EXPORT_SLAYERS_PATH` |
| slayers_path_definitions.json | CatalogExporter `EXPORT_SLAYERS_PATH` |
| reward_cache_store.json | CatalogExporter catalog |
| platinum_store.json | CatalogExporter catalog and `EXPORT_STORE_RUNTIME` store images |
| inventory_storage_policy.json | CatalogExporter catalog, for every item the code and data reference |
| progression_config.json | A progression-config response you supply |
| ladyluck_store.json | A Lady Luck store response you supply |

Director reads these files from reference/1.14.7/ParadoxDirector/game-data, all generated from CatalogExporter
`EXPORT_HUNTS`:

- player_hunts_table.json
- matchmaker_hunts_table.json
- arena_easy_matchmaker_hunts.json
- arena_hard_matchmaker_hunts.json
- arena_elite_matchmaker_hunts.json
- arena_hard_matchmaker_hunts_new.json
- arena_elite_matchmaker_hunts_new.json

Override either final directory with PARADOX_GAME_DATA_DIR. The two supplied sources go in
reference/1.14.7/ParadoxBackend/game-data/raw (override with PARADOX_GAME_DATA_RAW_DIR). A missing file stops the service
at startup or fails the first request that needs it (the storage policy is read on first use, and
missing Slayer's Path definitions only log an error and grant no node rewards).

## 1. Build and run CatalogExporter

The CatalogExporter is Dainty Lass's MIT Rust crate `native/rustic-isles-exporter`, built against the complete SDK
described in [GENERATING_SDK.md](GENERATING_SDK.md). Section 3 of Dainty Lass's
[SDK and game data guide](../../../../docs/sdk-and-game-data.md) builds and injects it and copies its
`Items_Analysis` exports into `reference/1.14.7`, where the generators read them. The Host page's flags file turns on
every export the services need:

| Flag | Output under Items_Analysis | Reach first |
|---|---|---|
| EXPORT_CATALOG=1 | catalog_1_14_7.jsonl, equipment_1_14_7.jsonl | Main menu or Ramsgate |
| EXPORT_SLAYERS_PATH=1 | slayers_path_1_14_7/player_journey_nodes.jsonl | Open the Slayer's Path screen |
| EXPORT_HUNTS=1 | hunts_1_14_7/ (player_hunts.jsonl, matchmaker_hunts.jsonl, hunt_export_manifest.json and more) | Ramsgate, then open the Hunt/Map screen |
| EXPORT_STORE_RUNTIME=1 | store_item_images_1_14_7.jsonl and two store view-model captures | Ramsgate, then Journal > Challenges > Reward Cache |

Inject the exporter into your own local client only. Flags are read once at injection, so re-inject
after changing them. The exporter waits up to two minutes for the catalog to load; open the listed
screen while it runs. Each run writes export_manifest_1_14_7.json and appends to
catalog_export_status.txt: check them instead of assuming an injection produced every table. The research exports
of Mystic Paradox's C++ exporter (cells, combat, skins, drop tables, currency names, table inventory) are not
supported; the services never read them.

## 2. Director hunt tables

From reference/1.14.7/ParadoxDirector, on a fresh checkout (do not copy the hunt examples first; the importer would keep
their rows):

    Set-Location reference\1.14.7\ParadoxDirector
    npm run generate:hunt-tables
    npm run generate:hunt-tables -- --apply

The first run is a dry run that reports, per table, the rows it would add and any row it cannot
convert. It reads Items_Analysis/hunts_1_14_7 by default (`-- --export <folder>` or HUNT_EXPORT_DIR
for another one) and refuses to write rows without a map path or with an unknown enum value. With
`--apply` it creates every table, including the two `_new` arena tables, and backs up any table it
replaces. Re-running later adds new rows only; `--update-existing` also updates changed rows.

## 3. Slayer's Path

From ParadoxBackend:

    Set-Location ..\ParadoxBackend
    Copy-Item game-data\slayers_path.example.json game-data\slayers_path.json
    npm run generate:slayers-path
    npm run generate:slayers-path -- --apply
    node scripts/generate_slayers_path_definitions.cjs
    node scripts/generate_slayers_path_definitions.cjs --apply

The graph generator fills the node list of an existing slayers_path.json, which is why the example is
copied first. The definitions generator writes the rewards and costs of each node. Both preview without
`--apply`.

## 4. Reward Cache store

    npm run reward-cache:generate

This writes reward_cache_store.json from the catalog and then rebuilds the storage policy. It stops
with an error if an item it needs is missing from your catalog. Set RUSTIC_ISLES_CATALOG_PATH to read
a catalog from another location.

## 5. Progression config and Lady Luck store

These two files cannot be generated from CatalogExporter output: the client does not hold the hunt-pass
reward tables or the Lady Luck store offers. Supply a compatible response for each in
reference/1.14.7/ParadoxBackend/game-data/raw:

| Source file | Shape |
|---|---|
| progression_config.source.json | A progression-config response object with `payload.paths` as an array |
| ladyluck_store.source.json | A JSON array of store entries (`id`, `prices`, `items`, ...) |

Then validate and write:

    npm run generate:progression-config
    npm run generate:progression-config -- --apply
    npm run generate:ladyluck-store
    npm run generate:ladyluck-store -- --apply

The generators check the shape, preview without `--apply`, and back up any file they replace. The
synthetic examples show the expected structure but contain no real rewards or offers.

## 6. Platinum store and storage policy

    npm run platinum:generate

This needs reward_cache_store.json and ladyluck_store.json (steps 4 and 5), the catalog, and
store_item_images_1_14_7.jsonl. Prices it cannot read from a store capture are reconstructed. It then
rebuilds the storage policy, so run it last. To rebuild only the storage policy later:

    npm run inventory:storage-policy:generate

## 7. Verify final outputs

From the repository root:

    $backendRequired = @(
      'progression_config.json', 'slayers_path.json', 'slayers_path_definitions.json',
      'ladyluck_store.json', 'platinum_store.json', 'reward_cache_store.json',
      'inventory_storage_policy.json'
    )
    $directorRequired = @(
      'player_hunts_table.json', 'matchmaker_hunts_table.json',
      'arena_easy_matchmaker_hunts.json', 'arena_hard_matchmaker_hunts.json',
      'arena_elite_matchmaker_hunts.json', 'arena_hard_matchmaker_hunts_new.json',
      'arena_elite_matchmaker_hunts_new.json'
    )
    $backendRequired | ForEach-Object { Test-Path (Join-Path 'reference\1.14.7\ParadoxBackend\game-data' $_) }
    $directorRequired | ForEach-Object { Test-Path (Join-Path 'reference\1.14.7\ParadoxDirector\game-data' $_) }

Every result must be True. Then run both test suites: once every real file is present they include
the `*.gamedata.test.ts` checks of the data.

    Set-Location reference\1.14.7\ParadoxBackend
    npm test
    Set-Location ..\ParadoxDirector
    npm test

At startup the Director logs a hunt-table audit (`[HuntTableAudit] ... problems=0`).

## Synthetic smoke-test data

To test compilation and wiring only, copy each *.example.json to its corresponding *.json name.

Do not describe a synthetic deployment as playable. Replace all fixtures with data generated from
the same compatible installation before testing account progression, matchmaking, or Ramsgate.

## Provenance and privacy

Keep raw captures, Items_Analysis and the final extracted tables out of the public repository. They are
ignored for this reason. Before sharing logs, remove account identifiers, tokens, private paths, and
captured payloads that contain user or proprietary data.

Continue with Dainty Lass's [hosting guide](../../../docs/hosting.md) to install the data and start the server.
