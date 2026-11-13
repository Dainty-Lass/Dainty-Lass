# Catalog exporter: old and new on the real 1.14.7 client

The hands-on check of [catalog-exporter.md](catalog-exporter.md) section 6 and spec 10
([spec-catalog-exporter.md](../spec-catalog-exporter.md)), run on the maintainer's PC on 2026-11-13. This page records
only file names, field names, verdicts and row counts: the exported game data stays off the repository (AGENTS.md).

**Verdict: pass.** With the store-image fix (`plan` `b28bdc9b`), the new DLL writes all 24 phase-1 files the same
as the old DLL in the same game state: 23 byte-identical, and the manifest differs only in `exportedAt` (section 6).
The first run, `new-1` from `99488ca9`, failed only on `store_item_images_1_14_7.jsonl`. Every image path was read 4
bytes early (section 3), and the spec correction in section 4 fixed it. The Host page's exporter path can move to the
new DLL (spec 9 seam 4).

## 1. Setup

| | |
| --- | --- |
| Client | Dauntless 1.14.7, changelist 647472, UE 4.26.2; `Dauntless-Win64-Shipping.exe` SHA-256 `f3f95fbf...fbb66b` |
| SDK | Dumper-7 dump imported into `restricted/modules/rustic-isles/native/runtime` (the same for both DLLs) |
| Old DLL | `restricted/modules/rustic-isles/native/catalog-exporter`, `_build.bat`, root `C:\dl-diff\old` (SHA-256 `40351061...`) |
| New DLL | `native/rustic-isles-exporter` at `plan` `99488ca9` (exporter tip `fcdc80c3`), `DAINTY_SDK_DIR` at the dump, root `C:\dl-diff\new` (SHA-256 `0daeb48d...`) |
| Flags | Phase 1 only, in both roots: `CATALOG`, `PROGRESSION`, `SLAYERS_PATH`, `HUNTS`, `STORE_RUNTIME`, `MASTERY` on; `SKINS_RESOLVE_STRINGTABLE`, `CURRENCY_NAMES` off |
| Loading | The launcher injected both DLLs (`feat/launcher-injects-dlls`, RD9 reversed 2026-11-12): the runtime first, then the exporter, into the suspended game. No `mystic_loader.ini` was involved |
| Compare | Spec 10 step 5's `Compare-Run`, unchanged |

The first build against the real SDK stopped on four missing facts (`UStruct.Super`, `FDateTime.Ticks` and the two
`FScheduleData` ticks). Their spec correction (`UStruct.SuperStruct`; an `FDateTime` is the int64 at its offset 0)
landed as `fcdc80c3` before these runs.

## 2. Runs and counts

| Run | DLL | general | equipment | progression | slayersPath | hunts | storeRuntime | mastery |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `old-1` | old | 1907 | 0 | 139 | 509 | 1378 | 3173 | 304 |
| `old-2` | old | 7318 | 1608 | 139 | 509 | 1378 | 3173 | 304 |
| `new-1` | new | 7318 | 1608 | 139 | 509 | 1378 | 3173 | 304 |

Every run finished on its own about 8 s after the game started, before the player opened any screen
(`exportedAt` 8 s after the launcher's start line). So spec 3.1's timing decides what is loaded, not the screens the
procedure asks for. `old-1` found far fewer catalog objects than the two later runs, which found the same.

## 3. Comparison

**Noise floor, `old-1` against `old-2`:** 20 files the same, 4 different, all because of timing: `catalog_1_14_7.jsonl`
(1907 against 7318 lines), `equipment_1_14_7.jsonl` (0 against 1608), `export_manifest_1_14_7.json` (those counts) and
`hunts_1_14_7/player_hunts.jsonl` (`playerHuntId` is `None` in all 161 rows of `old-1`, because the hunt objects
were not loaded yet).

**`old-2` against `new-1` (the same game state), and the reverse:**

| File | Verdict |
| --- | --- |
| `catalog_1_14_7.jsonl` | same |
| `equipment_1_14_7.jsonl` | same |
| `export_manifest_1_14_7.json` | same (masked): `exportedAt` only |
| `store_item_images_1_14_7.jsonl` | **DIFFERENT** |
| `store_item_view_models_1_14_7.jsonl`, `store_view_models_1_14_7.jsonl` | same |
| `hunts_1_14_7/` (all 9: `activity_unlock_criteria`, `escalation_mode_specs`, `game_activities`, `hunt_export_manifest.json`, `hunt_modifiers`, `hunt_regions`, `matchmaker_hunts`, `player_hunts`, `schedule_data`) | same |
| `mastery_1_14_7/mastery_objectives.json` | same |
| `progression_1_14_7/` (all 7) | same |
| `slayers_path_1_14_7/player_journey_nodes.jsonl` | same |

Neither run wrote a file the other lacks. Every `same` is byte-exact, so the line endings match too: CRLF in the text-mode
manifests and LF in `mastery_objectives.json`, as review 4.4 expected. This settles the things only the diff could
settle: name decoding (number suffix, wide and narrow entries), the weak-pointer serial check, the catalog's
`itemClass` soft class path (review 4.3), `catalogGeneralItems`, and every number's width.

**`store_item_images_1_14_7.jsonl`:** 3168 rows in both, same order, same `table` and `rowName`. Only the two image
fields differ, and only where the old wrote a path:

| Field | Rows that differ | Old writes | New writes |
| --- | ---: | --- | --- |
| `featureImage` | 3074 | the short asset name (`<asset>.<asset>`) | `None_<n>`, with `<n>` different on every row |
| `standardImage` | 1472 | the short asset name | `None_<n>` |

Rows whose image is unset match: `None` in both, 94 `featureImage` and 1696 `standardImage`.

## 4. Cause and the spec correction

The new DLL reads a store image's path at `member + TPersistentObjectPtr.ObjectID + FSoftObjectPath.AssetPathName`,
taking `ObjectID`'s offset from the SDK (`engine.rs` `soft_path_sdk`, fact `SOFT_OBJECT_ID`). Dumper-7's `Basic.hpp`
declares `TPersistentObjectPtr` by hand (a "predefined", not auto-generated, template), with `ObjectID` at `+0x0C`
and size 0. In the compiled game the `FSoftObjectPath` inside is aligned to 8, so `ObjectID` really starts at `+0x10`.
The `FStoreItemTable` members are 0x28 bytes each, which fits: 8 for the weak pointer, 4 for the tag, 4 of padding,
and 0x18 for the path. Read at `+0x0C`, the FName's index is the 4 padding bytes, which is 0, so `None`. Its number
is the real `AssetPathName`'s index, hence `None_<n>`.

**Spec correction (6.13, and 4.5 for every soft pointer):** a soft pointer's `AssetPathName` is the FName at the
member's offset `+ 0x10` (`TPersistentObjectPtr`'s 8-byte weak pointer, 4-byte tag, 4 bytes of alignment). This holds
for every reader, the hunts' raw form and the store images' short form alike. `TPersistentObjectPtr.ObjectID` is not
taken from the SDK, because the dump's predefined template gives the unaligned `+0x0C`. The store images keep the short
form, with an unset image written as `None`.

The implementer then makes `SOFT_OBJECT_ID` a fixed `0x10` (it is already the fact's default; the SDK lookup
overrides it). A synthetic fixture whose SDK declares `ObjectID` at `0x0C` would keep the case covered.

## 5. Also found on the way

None of these block the diff. Each is reported to the session that owns it.

- **Launcher, `restricted` workspace root (fixed).** Running `dainty-host` from `restricted\` set up a second, empty
  server in `restricted\.workspace`, so the launcher's key was "not known" there. The fix was to run from the
  repository root. The launcher also sent a registration with an empty invite code (fixed in `364cb856`).
- **Launcher, Host page (fixed in `4bd4f751`).** A checkout whose `dainty-host` was only built in `target\debug` was
  "not a Dainty Lass checkout". A debug launcher now finds it, and it shows the Host tools.
- **Injection, shared `Win64` (fixed on the injection branches).** A host's servers run from the install the host
  plays from. PLAY refused their `winmm.dll`, and without it Ramsgate started as a windowed client in a restart
  loop. Fixed by `f8df0b58` (the loader stands down for an injected game) and `feat/director-injects-runtime` (the
  director injects the runtime into every server). Two bugs there were fixed after this run: `publish-runtime` never
  removed the old loader (`c3208d06`), and `start` with an old `director.env` started servers without the runtime
  (`c636f4df`).
- **Relay (open).** `relay refused ... not the game's` appears a few times per game session for the game's own
  requests: progression, auth `accountinfo`, Epic `oauth/token`. It also appeared with the loader on 2026-11-11.
- **Spec 10 step 2 (stale).** It names `-p rustic-isles-exporter` and `rustic_isles_exporter.dll`. The crate builds
  `-p catalog-exporter-dll` to `CatalogExporter.dll`, as section 6 of the review already says. Step 3 turns on every
  key, but the new DLL refuses the research exports, so section 6's phase-1 flags are the ones to use.
- **Timing (spec 12's open question).** Both DLLs export about 8 s into the start. The tables were loaded in every
  run, and the catalog only partly in the first. Compare against a run of the old DLL made just before, as `old-2`
  and `new-1` were here.

## 6. The re-run: `new-2`

The spec correction landed as `86cffa7e` (merged into `plan` as `b28bdc9b`): every soft pointer's `AssetPathName`
is at the member + 0x10, and `TPersistentObjectPtr.ObjectID` is no longer an SDK fact. The DLL was rebuilt from
`b28bdc9b` against the same SDK (SHA-256 `954e203f...`) and run once more, injected the same way.

| Run | DLL | general | equipment | progression | slayersPath | hunts | storeRuntime | mastery |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `new-2` | new, fixed | 7318 | 1608 | 139 | 509 | 1378 | 3173 | 304 |

`Compare-Run old-2 new-2`: 23 files `same`, `export_manifest_1_14_7.json` `same (masked)` (`exportedAt`).
`store_item_images_1_14_7.jsonl` is now byte-identical. `Compare-Run new-2 old-2` lists nothing that the old run
lacks. That is better than the noise floor (`old-1`/`old-2`, 4 files different), so the check passes.

Clean-up (spec 10 step 6): the maintainer's own `ExportPaths.local.h` and `CatalogExporter.dll` were put back from
`C:\dl-diff\backup`. `C:\dl-diff` stays on the maintainer's PC and off the repository.
