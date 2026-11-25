# Director game-data

This directory holds the **locally generated** hunt and matchmaker tables the 1.14.7 Director
(`rustic-isles-director`) reads at start. A server keeps them in `modules/rustic-isles/game-data/1.14.7/director/`:

- `player_hunts_table.json`
- `matchmaker_hunts_table.json`
- `arena_easy_matchmaker_hunts.json`
- `arena_hard_matchmaker_hunts.json`
- `arena_elite_matchmaker_hunts.json`
- `arena_hard_matchmaker_hunts_new.json` and `arena_elite_matchmaker_hunts_new.json` (the second Hard and
  Elite Trials tables added in 1.14.7)

**These files are not distributed:** they are Phoenix Labs game data. Generate them from your own lawful
game installation (`docs/sdk-and-game-data.md`). Only the synthetic `*.example.json` placeholders are
committed, so the project builds and can smoke-test without the real data.

## Synthetic data

`dainty-host gamedata --examples` copies each placeholder to its real name where the real table is missing.
The placeholders let the Director start; they launch no real hunt.

## Real data

Export the hunt tables with the CatalogExporter (`EXPORT_HUNTS=1`, written to `../exports/hunts_1_14_7`);
`dainty-host gamedata` imports them here.
