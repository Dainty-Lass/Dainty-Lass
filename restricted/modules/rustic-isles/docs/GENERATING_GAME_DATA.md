# Generating game data

Rustic Isles does not distribute Phoenix Labs game data. Extract the required material from your
own lawful Dauntless 1.14.7 installation and generate the service payloads locally.

> [!NOTE]
> This guide is for Dauntless 1.14.7, which `main` targets. For 1.12.0, follow the guide on the
> [`dauntless-1.12.0`](https://github.com/pranav158/Mystic-Paradox/tree/dauntless-1.12.0) tag.

Synthetic *.example.json files only allow build and startup smoke tests. They are not playable.

The paths below are under `modules/rustic-isles/game-data/1.14.7` of a server folder (`restricted/modules/...` in a
checkout). Before RB15 they were under `reference/1.14.7` (`ParadoxBackend/game-data`, `ParadoxDirector/game-data`,
`Items_Analysis`, `DauntlessEndpointDocumentation`); `dainty-host setup` and `update` move an old install's files once.

## Final required files

The backend reads these files from `backend/`:

| File | Generated from |
|---|---|
| slayers_path.json | CatalogExporter `EXPORT_SLAYERS_PATH` |
| slayers_path_definitions.json | CatalogExporter `EXPORT_SLAYERS_PATH` |
| reward_cache_store.json | CatalogExporter catalog |
| platinum_store.json | CatalogExporter catalog and `EXPORT_STORE_RUNTIME` store images |
| inventory_storage_policy.json | CatalogExporter catalog, for every item the code and data reference |
| progression_config.json | The progression export, or a progression-config response you supply |
| ladyluck_store.json | The store captures, or a Lady Luck store response you supply |

The Director reads these files from `director/`, all generated from CatalogExporter `EXPORT_HUNTS`:

- player_hunts_table.json
- matchmaker_hunts_table.json
- arena_easy_matchmaker_hunts.json
- arena_hard_matchmaker_hunts.json
- arena_elite_matchmaker_hunts.json
- arena_hard_matchmaker_hunts_new.json
- arena_elite_matchmaker_hunts_new.json

Supplied sources go in `backend/raw`. A missing file stops the service at startup or fails the first request that
needs it (the storage policy is read on first use, and missing Slayer's Path definitions only log an error and grant
no node rewards).

## 1. Build and run CatalogExporter

The CatalogExporter is Dainty Lass's MIT Rust crate `native/rustic-isles-exporter`, built against the complete SDK
described in [GENERATING_SDK.md](GENERATING_SDK.md). Section 3 of Dainty Lass's
[SDK and game data guide](../../../../docs/sdk-and-game-data.md) builds and injects it and copies its exports into
`exports/`, where the generators read them. The Host page's flags file turns on every export the services need:

| Flag | Output under exports/ | Reach first |
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

## 2. Generate

From the server folder:

    bin\dainty-host gamedata

This runs the in-process Rust generators on `exports/`, in order: the progression config, the Director's hunt
tables, the Slayer's Path graph and definitions, the Reward Cache store, the Lady Luck store, then the platinum store
and the storage policy. It fetches the store captures `store-captures.json` pins into `captures/` first. A step whose
input is missing is skipped and reported, and the rest still run; the log is `.workspace/logs/gamedata.log`.

The progression config and the Lady Luck store can't always come from the exporter: the client may not hold the
hunt-pass reward tables or the store's offers. Supply a response for each in `backend/raw`:

| Source file | Shape |
|---|---|
| progression_config.source.json | A progression-config response object with `payload.paths` as an array |
| ladyluck_store.source.json | A JSON array of store entries (`id`, `prices`, `items`, ...) |

`dainty-host gamedata --example-sources <folder>` copies them there, and `--progression <folder>` builds the
progression source from a progression export. The synthetic examples show the expected structure but contain no
real rewards or offers. The [SDK and game data guide](../../../../docs/sdk-and-game-data.md) has every option.

## 3. Verify final outputs

From the server folder:

    $data = 'modules\rustic-isles\game-data\1.14.7'
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
    $backendRequired | ForEach-Object { Test-Path (Join-Path "$data\backend" $_) }
    $directorRequired | ForEach-Object { Test-Path (Join-Path "$data\director" $_) }

Every result must be True. At startup the Director logs a hunt-table audit (`[HuntTableAudit] ... problems=0`).

## Synthetic smoke-test data

To test compilation and wiring only, run `dainty-host gamedata --examples`, which copies each *.example.json to
its *.json name.

Do not describe a synthetic deployment as playable. Replace all fixtures with data generated from
the same compatible installation before testing account progression, matchmaking, or Ramsgate.

## Provenance and privacy

Keep raw captures, exports and the final extracted tables out of the public repository. They are
ignored for this reason. Before sharing logs, remove account identifiers, tokens, private paths, and
captured payloads that contain user or proprietary data.

Continue with Dainty Lass's [hosting guide](../../../docs/hosting.md) to install the data and start the server.
