<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Catch-up list: rustic-isles (DS5)

> **Closed**: RU1 applied what this list marks needed (DS5). The rustic-isles repository is in-tree as
> `restricted/modules/rustic-isles` since RD27, and its Rust port is the MIT `rustic-isles-*` crates at the root.
> Kept as the record; paths name the layout of the time.

Every commit of this repository since RP2 that touched `paradox/`, `native/paradox-runtime`,
`native/catalog-exporter` or `preset/`, and whether rustic-isles' base, the `replay` branch, already has it. RU1
applies what is marked needed when it lays out rustic-isles (`docs/dainty-lass-plan.md`, DD7, DD13, RU1). The
entries follow the migration guide's section 6, with the `Rust:` line replaced by an `Isles:` line, since the
target here is rustic-isles' tree and not a Rust module.

## Range

- **`replay`:** `ce1c42e` (2026-10-23), Mystic Paradox `main` at `21b9ff5` plus 41 commits: patches 0001–0039 as one
  commit each, in Mystic's own layout (`ParadoxBackend/`, `ParadoxDirector/`, `ParadoxRuntime/`,
  `tools/CatalogExporter/`, `shared/`), the `0005` follow-up `3385265` and the follow-ups commit `ce1c42e`.
- **Start:** RP2. Its original subtree merge did not survive the 2026-10-23 rewrite of `plan`: RP1–RP4 became one
  commit, `dc8f939` "Fork Mystic Paradox into paradox/ and native/" (2026-10-23 17:50 -0600). That commit is the
  first entry, because it holds more than patches 0001–0033.
- **End:** `main` at `f891d92` (2026-10-26).
- **This repository:** `main` was given a new root on 2026-10-26 (`3a756cf`), so the history before it comes from
  `dainty-lass-pre-rewrite-2026-10-26.bundle`: its `main` (`c57f157`), `plan` (`ca8b95e`) and every other branch in
  it. `git log --full-history` on the four paths, then every bundle branch with `--not c57f157`, then every local and
  `origin/` branch with `--not main`. `preset/` was at the repository root until `3a756cf` deleted it.
- **Method:** "already in `replay`" is decided by content, not ancestry. Folder against folder, with the layout
  mapped: `replay:ParadoxBackend` to `paradox/backend`, `ParadoxDirector` to `paradox/director`, `ParadoxRuntime` to
  `native/paradox-runtime`, `tools/CatalogExporter` to `native/catalog-exporter`, `shared` to `paradox/shared`, and
  Mystic's top-level files and `docs/` to `paradox/`. Each comparison was made at `ed6822b`, at `c57f157` and at
  `main`. Duplicate copies of a commit were matched with `git patch-id --stable`.
- **Freeza's line:** not covered. His branches after `477a6ac` are RZ4's, not this project's commits.

## Summary

| Status | Commits |
| --- | --- |
| Needed | 3 |
| Not applicable | 7 |
| For rustic-freeza (RZ2) | 1 |

A commit with parts in more than one status is counted by the part that needs the most work; each part is listed
below it. Two needed commits also have parts for rustic-freeza: `dc8f939` (0004–0033) and `3a756cf` (0035
dropped).

What RU1 has to do, in short: after the `git mv` of the layout, replace `replay`'s versions of patches 0001, 0002
and 0003 with `main`'s files (table below), add the Director's log fix, and add `store-captures.json` with its
`.gitignore` line. The gameplay patches 0004–0039 that `replay` carries are not RU1's to touch: they leave
rustic-isles with RZ2.

### `replay` against `main`, by folder

Every difference between the two trees in the four paths, and the entry that explains it.

| `main` path | Difference from `replay` | Entry |
| --- | --- | --- |
| `paradox/backend/src/routes/reforgedApi.ts` | Rewritten (290+, 207−); adds `POST /reforged/api/Kick`; `HasAdminKey` becomes `HostOnly` | `3a756cf`, `8c38219` |
| `paradox/backend/src/launcherApiRules.ts`, `.test.ts` | New (275 and 152 lines); replace `reforgedApiRules.ts` and `.test.ts` (226 and 134), which go | `3a756cf`, `8c38219` |
| `paradox/backend/src/routes/eos.ts` | Comments and two log lines | `3a756cf` |
| `paradox/backend/src/config/listenerPlan.ts`, `.test.ts` | The `REFORGED_HTTP_ONLY` and `QOS_PORT=0` aliases rewritten, same behaviour | `3a756cf` |
| `paradox/backend/src/app.ts` | One comment | `3a756cf` |
| `paradox/backend/store-captures.json` | New | `dc8f939` |
| `paradox/director/src/controllers/gameservers.ts` | The log stream fix; `ListInstancesForStatus` rewritten as `InstanceSummaries` | `dc8f939`, `3a756cf` |
| `paradox/director/src/app.ts`, `server.ts` | The instance list's new name; comments | `3a756cf` |
| `native/paradox-runtime/dllmain.cpp` | Patch 0001's backend URL and XMPP TLS switch rewritten; patch 0035 removed | `3a756cf` |
| `native/paradox-runtime/tools/compat-inventory.ps1` | Paths for `native/paradox-runtime` | `dc8f939` |
| `native/catalog-exporter/CatalogExporter.vcxproj` | `SdkRoot` is `..\paradox-runtime\` | `dc8f939` |
| `paradox/README.md`, `.gitignore`, `docs/FAQ.md`, `docs/GENERATING_GAME_DATA.md` | This repository's layout and links; `docs/SELF_HOSTING.md` and the launcher workflows deleted | `dc8f939`, `ed6822b` |
| `paradox/shared`, `paradox/ADDITIONAL_TERMS.md`, `LICENSE`, `NOTICE.md`, `SECURITY.md` | None | — |
| `preset/` | Not on `main`; deleted by `3a756cf` | `3a756cf`, `cdc67f0` |

### `replay`'s own commits, by owner

For RU1 and RZ2: `replay` holds every patch as a commit, and DD13 splits them.

| Patches | `replay` commits | Owner |
| --- | --- | --- |
| 0001 runtime backend URL, 0002 launcher API, 0003 Director bind and instances | `4948437`, `d5eab70`, `df86643` | This project's infrastructure. Their text is the Reforged Revival line's; `main` has them rewritten (`3a756cf`), and RU1 takes `main`'s files instead |
| 0004–0039 | `f8bcf60` … `63c6f3c`, with `3385265` and `ce1c42e` (follow-ups to 0005, 0014 and 0015) | The Reforged Revival line's gameplay patches: rustic-freeza, through RZ2 (DD25). 0006, 0009, 0019, 0032 and 0034 are empty marker commits, because `21b9ff5` already has them. 0035 (`7e808a9`) is on `replay` but no longer on `main` |

## This repository

### `dc8f939` Fork Mystic Paradox into paradox/ and native/

- **SHA:** `dc8f939497a7dad4fa842ff27b0595e1db0ced39`; the same patch as `e34cf60` on the bundle's
  `feat/player-launcher-headless-server`, the SHA the migration guide cites
- **Date:** 2026-10-23T17:50:08-06:00
- **Author:** Silent Spud
- **Files:** `paradox/` (backend, Director, docs, top-level files), `native/paradox-runtime`,
  `native/catalog-exporter`, `preset/README.md`, `preset/preset.json`
- **Behavior:** RP1–RP4 in one commit: Mystic `00d3be8` with patches 0001–0033 (the fork's `reforged` branch,
  `24e60c8`) moved into this repository's layout. Compared with `24e60c8`, it also carries:
  - The Director no longer crashes when a game server dies at once. Both output pipes ended the log stream before
    the exit event, so the exit line was written after the end, and the unhandled `write after end` stopped the
    Director. The log now ends on `close` and every write checks `writableEnded`
    (`paradox/director/src/controllers/gameservers.ts`, marked `[2026-10-23]`).
  - `paradox/backend/store-captures.json`, moved from the Reforged Revival line's `upstream/` (RP4), with
    `DauntlessEndpointDocumentation/` in `paradox/.gitignore`. The store generators read the captures it pins.
  - Layout fixes: `compat-inventory.ps1` and `CatalogExporter.vcxproj` paths, `paradox/.gitignore` and
    `paradox/README.md` for the new folders, docs links to `docs/hosting.md`, Mystic's `SELF_HOSTING.md`,
    `ParadoxLauncher`, `scripts/` and launcher workflows deleted, and the preset's paths.
- **Isles:** needed, by part:
  - The Director's log fix: needed. This project's own Director fix, missing from `replay`'s
    `ParadoxDirector/src/controllers/gameservers.ts` (13+, 5−). It applies cleanly on `replay` at the same place.
  - `store-captures.json` and its `.gitignore` line: needed. The host kit reads the file next to the backend
    (`crates/host`, `docs/sdk-and-game-data.md`), and it stays with the backend in rustic-isles. It came from
    the Reforged Revival line but is not one of the gameplay patches; see "To decide" below.
  - Patches 0001–0003: in `replay` (`4948437`, `d5eab70`, `df86643`), then rewritten on `main`; under `3a756cf`.
  - Patches 0004–0033: in `replay` as commits; for rustic-freeza (RZ2), not for RU1.
  - Layout fixes: not applicable. They point at DaintyLass's folders. RU1 lays out its own (`native/runtime/`,
    `reference/1.14.7/`) and fixes paths for it. The one to remember: `CatalogExporter.vcxproj`'s `SdkRoot` must
    point at wherever the runtime lands, since RU1 brings `catalog-exporter` over from this repository with
    `..\paradox-runtime\` in it.
  - `preset/` paths: not applicable; `preset/` is gone (`3a756cf`).

### `5f23e0e` Merge upstream main into plan

- **SHA:** `5f23e0e342b0d82df47be374b8cacd985107f247`; the same merge result as `02a8070` (`git diff` against the
  first parent, by patch-id)
- **Date:** 2026-10-23T18:12:23-06:00
- **Author:** Silent Spud
- **Files:** `native/paradox-runtime/dllmain.cpp`, `paradox/backend/src/routes/inventory.ts`,
  `paradox/backend/src/routes/system.ts`, `paradox/backend/src/weaponBaseLevel.ts`, `.test.ts` (the merge's own
  resolution; the merged upstream side touches none of the four paths)
- **Behavior:** patches 0034–0039, which arrived as files under `upstream/`, applied by hand to the backend and
  the runtime.
- **Isles:** for rustic-freeza (RZ2). These are the Reforged Revival line's gameplay patches. `replay` has them
  (`1c9b5c5`, `7e808a9`, `05d32f5`, `19c63e8`, `c48d8d4`, `63c6f3c`); `ed6822b` and `replay` match on these files.

### `ed6822b` Update Mystic Paradox to 21b9ff5

- **SHA:** `ed6822bd7ce47746701b6a8ed12b487006dca28f`; the same patch as `ab1830a`
- **Date:** 2026-10-23T19:33:05-06:00
- **Author:** Silent Spud
- **Files:** 500 files under `paradox/` and `native/`, `paradox/shared/hunt-manifest` new
- **Behavior:** `paradox/` and `native/` moved to Mystic `21b9ff5`, with the 39 patches replayed onto it.
- **Isles:** not applicable. `replay` is this commit's replay, made the same evening. The trees match folder for
  folder, except for what `dc8f939` added before it (the log fix, `store-captures.json`, the layout fixes), which
  is listed there.

### `8c38219` Admin API: GraphQL moderation and status for a Discord bot

- **SHA:** `8c382196767a23195c9c7301a864a7b529e2a174`; the same patch as `053a571`
- **Date:** 2026-10-24T02:28:27-06:00
- **Author:** Silent Spud
- **Files:** `paradox/backend/src/reforgedApiRules.ts`, `.test.ts`, `paradox/backend/src/routes/reforgedApi.ts`
  (the rest of the commit is in `rust/`)
- **Behavior:** `POST /reforged/api/Kick {Username}` (admin key, host only) revokes the account's refresh sessions
  and unused exchange codes, closes its chat connections with 1008, clears the status cache and answers
  `{UserId, Username, Closed}`; 404 for an unknown name, 400 without one. The admin service's `kick` calls it
  (MIGRATION_PLAN.md RD25).
- **Isles:** needed. `replay` has no kick route. Taken in `3a756cf`'s form (`DisconnectAccount` in
  `launcherApiRules.ts`, the route in `reforgedApi.ts`), not this commit's, since `3a756cf` rewrote both files.
  RU3a's inventory must carry the route.

### `b2b8f90` Merge feat/admin-graphql into plan: the GraphQL admin API

- **SHA:** `b2b8f90b7fd47c5256a65a7bed0f29f94d237f62`; on the other copy, `8515c6f`
- **Date:** 2026-10-24T02:30:25-06:00
- **Author:** Silent Spud
- **Files:** none of its own (`git show --cc` is empty for the four paths)
- **Behavior:** none beyond `8c38219`.
- **Isles:** not applicable; a merge with no change of its own.

### `9a319f5` RD2: paradox/ takes any change, with fixes as the baseline instead of a rule

- **SHA:** `9a319f548adfa70d1e51a6e4f1c94bc65d00f8c9`; the same patch as `cb3d7e5`
- **Date:** 2026-10-24T02:42:57-06:00
- **Author:** Silent Spud
- **Files:** `paradox/backend/src/routes/reforgedApi.ts` (the rest is in the plan and docs)
- **Behavior:** none; the kick route's comment names RD25 instead of calling the route an exception to RD2.
- **Isles:** not applicable. A comment, and `3a756cf`'s rewritten comment already names RD25.

### `3a756cf` DaintyLass: a new root without the Dauntless Reforged Revival line

- **SHA:** `3a756cf8f6093af21fbc3acc85fae88acc954fef`
- **Date:** 2026-10-26T08:34:22-06:00
- **Author:** Silent Spud
- **Files:** compared with the bundle's `c57f157`: `native/paradox-runtime/dllmain.cpp`;
  `paradox/backend/src/app.ts`, `config/listenerPlan.ts`, `.test.ts`, `launcherApiRules.ts`, `.test.ts` (new),
  `reforgedApiRules.ts`, `.test.ts` (deleted), `routes/eos.ts`, `routes/reforgedApi.ts`;
  `paradox/director/src/app.ts`, `controllers/gameservers.ts`, `server.ts`; every file under `preset/` (deleted)
- **Behavior:** the three infrastructure patches written again from the Rust side's contract, with the same routes,
  headers, env names and answers. Runtime (0001): `RUSTIC_ISLES_BACKEND_URL`, then `REFORGED_BACKEND_URL`; the
  fail-closed host is `backend.invalid` instead of `reforged.invalid`; `bUseSSL` follows the address's scheme.
  Backend (0002): the launcher API in `launcherApiRules.ts` and `reforgedApi.ts`, the kick included; the EOS
  client-credentials token and the refusal of banned keys; the listener-plan aliases. Director (0003): loopback bind
  and `GET /api/instances` (`InstanceSummaries`). Patch 0035 removed from the runtime. The preset removed with the
  rest of the Reforged Revival line.
- **Isles:** needed, by part:
  - Runtime, backend and Director rewrites: needed. They are this project's launcher API, Director and runtime
    changes, the ones RU1 applies first. On rustic-isles they replace `replay`'s 0001–0003 (`4948437`, `d5eab70`,
    `df86643`): take `main`'s files for the paths in the table above, delete `ParadoxBackend/src/reforgedApiRules.ts`
    and `.test.ts`. `main`'s `dllmain.cpp` is `replay`'s with the 0001 hunks rewritten and 0035 removed (21+,
    42−): taking it whole also drops 0035; keeping 0035 means moving the 0001 hunks by hand.
  - Patch 0035 removed: for rustic-freeza (RZ2). It is in the 0004–0039 range, and `replay` still has it
    (`7e808a9`). RZ2's patch set should leave it out, as `main` does.
  - `preset/` deleted: not applicable to `replay`, which never had a `preset/`. Whether rustic-isles gets one is
    under "To decide".

## Before RP2: `preset/`

`preset/` arrived from the Reforged Revival line before RP2 and was not touched again except for the path fix in
`dc8f939`. Listed because DS5 names the folder.

### `cdc67f0` Ship a server preset with the project, as Dauntless Revived does

- **SHA:** `cdc67f0f975046d575d2838aaff3b705c0db4b7e`
- **Date:** 2026-10-15T12:56:40Z
- **Author:** Claude, in `FreezaMotoqueiro/Dauntless-Reforged-Revival` (merged there as `35deb0e`, PR #8)
- **Files:** `preset/` (`preset.json`, `README.md`, `backend/`, `director/`)
- **Behavior:** a server preset shipped with the project, which the host kit's `preset` command applies.
- **Isles:** not applicable. Before RP2, from the Reforged Revival line (it is in rustic-freeza's history at
  `477a6ac`), and `main` no longer has `preset/`.

### `35deb0e` Merge pull request #8 from FreezaMotoqueiro/claude/lucid-dijkstra-yggw3y

- **SHA:** `35deb0ed367087527f67a0ff447c33066ab5bbf2`
- **Date:** 2026-10-15T10:02:51-03:00
- **Author:** Freeza
- **Files:** `preset/`, through `cdc67f0`
- **Behavior:** none of its own.
- **Isles:** not applicable; the merge of `cdc67f0`.

### `e6ef3bf` Ship the "Freeza" server preset, without the capture-built stores

- **SHA:** `e6ef3bfbbc23015df642bda3271529c318d5fc73`
- **Date:** 2026-10-15T14:52:37Z
- **Author:** Claude, in `FreezaMotoqueiro/Dauntless-Reforged-Revival` (merged there as `6220067`, PR #9)
- **Files:** `preset/`
- **Behavior:** the preset becomes Freeza's tuning, without the stores built from the live captures.
- **Isles:** not applicable, for the same reasons as `cdc67f0`. It is Freeza's tuning, so if it lives anywhere it
  is rustic-freeza.

### `6220067` Merge pull request #9 from FreezaMotoqueiro/claude/lucid-dijkstra-yggw3y

- **SHA:** `622006735791b9d7af4814d2f489a5187674e9f5`
- **Date:** 2026-10-15T12:11:09-03:00
- **Author:** Freeza
- **Files:** `preset/`, through `e6ef3bf`
- **Behavior:** none of its own.
- **Isles:** not applicable; the merge of `e6ef3bf`.

## Searched, nothing found

- The bundle's other branches (`plan`, `docs/app-structure-rd26`, `feat/app-structure`, `feat/app-structure-docker`,
  `feat/server-gui`, `feat/rb10-director-single-binary`, `reforged-plus`, the five `fix/app-structure-*`, the plan
  branches): no commit on the four paths that `c57f157` lacks, except the second copies on
  `feat/player-launcher-headless-server` (`e34cf60`, `02a8070`, `ab1830a`, `053a571`, `8515c6f`, `cb3d7e5`), which
  match the entries above by patch-id.
- `main` after `3a756cf`, and every local and `origin/` branch not merged into it: none touch the four paths.

## To decide

- **`preset/`.** DD13 and RU1 have rustic-isles bring `preset/` from DaintyLass, but `3a756cf` deleted it as part of
  the Reforged Revival line (plan section 1 says so too), and its two commits are Freeza's tuning. Either RU1 takes
  it from the bundle (`c57f157:preset`, or Freeza's `477a6ac`) as 1.14.7 game data, and RB11 still names `preset/`
  as its generators' fixture, or it goes to rustic-freeza with the rest of his line, and DD13, RU1 and RB11 change.
  If it does go to rustic-isles, check first that nothing in it is game-derived data (AGENTS.md, "Nothing from the
  game is committed").
- **`store-captures.json`.** It came from the Reforged Revival line (`d3048cd`, in Freeza's repository) and `main`
  keeps it as infrastructure (`docs/paradox-patches.md`). This list counts it as needed. Its `_comment` still
  points at `legacy/ReforgedHost/src/captures.ts`, which is gone; the Rust host kit fetches the captures now.
- **0035 on rustic-isles before RZ2.** `replay` carries 0035, which `main` dropped. Leaving it until RZ2 takes every
  gameplay patch out is the simplest; reverting `7e808a9` in RU1 makes rustic-isles match `main` sooner.
- **The Director's log fix is undocumented.** It is a change of this project's in `paradox/` that
  `docs/paradox-patches.md` does not list.
