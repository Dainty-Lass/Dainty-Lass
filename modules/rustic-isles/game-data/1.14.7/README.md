# Rustic Isles 1.14.7 game data

`dainty-host gamedata` generates the backend's and the Director's game data here, into `backend/` and
`director/`, from a host's own CatalogExporter export (`exports/`) and the store captures (`captures/`).
None of that is committed (`../../.gitignore`). How to make it is in
[`docs/sdk-and-game-data.md`](../../../../docs/sdk-and-game-data.md).

What is committed:

- `backend/*.example.json` and `director/*.example.json`: synthetic placeholders, empty tables of the right
  shape. `dainty-host gamedata --examples` (and `setup` or `start`, for a file still missing) copies them into
  place so the services start. Nobody can play on them, and the launcher shows the server as running on example
  data.
- `store-captures.json`: the pin for the five store captures `gamedata` fetches on each host, by repository,
  commit and SHA-256 per file. The captures are third-party data and are never copied here.

## The Ramsgate wishing well

`backend/fountain_daily_bundle.json` is the well's offer, and nothing generates it. When it is missing, `setup`
and `gamedata` write a default (one Silver Cell Core per toss) if the storage policy lists that item as an
uncapped stack. A file you wrote is never replaced. The store refuses the whole file, and the well stays closed,
unless it holds exactly one offer (`{"skus": [ ... ]}` or a one-element array) with:

- an `id` of 1 to 128 letters, digits, `_`, `.` or `-`;
- `fountain_daily_free_bundle` among its `tags`;
- `prices` exactly `[{"currencyId": "id_currency_platinum", "price": 0}]`;
- no `entitlements`, `skuProgression` or `loadoutSlots`;
- at least one entry in `items`, each with a `catalogId`, `"instanced": false` and a whole `quantity` of 1 or
  more, and each an uncapped stacked item in `inventory_storage_policy.json`.
