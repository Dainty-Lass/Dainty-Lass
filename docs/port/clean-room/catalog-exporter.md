# Clean-room Provenance Record: the catalog exporter (`native/rustic-isles-exporter`)

## 1. Specification

- **Spec file**: `docs/port/spec-catalog-exporter.md`, written in `8d0363d4` and merged into `plan` as `0c18068c`.
- **Spec writer**: the session that read the replaced exporter (`restricted/modules/rustic-isles/native/catalog-exporter`,
  Mystic Paradox's C++ `CatalogExporter`). It wrote no code in `native/rustic-isles-exporter`.
- **Replaced code**: the C++ DLL, 16 source files. It stays in `restricted/` until the diff in section 6 passes.

## 2. Implementer isolation

- **Implementer**: a fresh session given the spec, the root `AGENTS.md` and the runtime loader's crate as a pattern. It
  reported that it opened nothing under `restricted/`.
- **Pre-invocation removal**: none. The C++ stayed in the working tree; isolation rested on the implementer's
  instructions. The guessed member names in section 4 confirm it: an implementer who had looked at the C++ would not
  have guessed 60 of them wrong.
- **Placement**: the brief named `native/catalog-exporter`, but root `.gitignore` ignores that path (a pre-DS6 host
  leftover), so the crate is at `native/rustic-isles-exporter`. Moving it is the integration pass's call.

## 3. Commits

- Spec: `8d0363d4` (merge `0c18068c`). Implementation: `f990d430` on `feat/catalog-exporter-clean-room`, merged as
  `7c83df17`. Integration (fuzz target, CI, lockfile): `a9bb2a57`.

## 4. Review findings

### 4.1 Similarity

The new code is Rust and the old is C++, so the 3-line run measure of `similarity.md` finds nothing to compare. The
output strings (status lines, JSON field names, file names) match because the spec fixes them byte for byte. A
provenance grep over the crate for Mystic Paradox, Reforged, coldloader, Freeza and pranav finds only
`mystic_loader.ini`, the runtime loader's file name, which the README cites (RD9).

### 4.2 Member names the spec should have given

`src/facts.rs` names every SDK member the exporter reads. The implementer guessed the names from the spec's JSON field
names and expected the first real build to report the misses. Checked against the members the C++ reads, these
differ. Each is written as the SDK path the spec should state. The integration pass adds them to the spec, and the
implementer fixes `facts.rs` from the spec. The implementer never sees the C++.

**Catalog (spec 6.1)**

| Fact | Guessed | SDK path |
| --- | --- | --- |
| `CAT_IS_STACKABLE`, `CAT_IS_BUNDLE`, `CAT_IS_CONTAINER` | `bIsStackable` ... | `FArchonCatalogItem.IsStackable`, `.IsBundle`, `.IsContainer` (no `b`) |
| `CAT_PRICE_CURRENCY` | `VirtualCurrencyPrices[].Currency` | `VirtualCurrencyPrices[].CurrencyType` (element `FPlayFabCatalogCurrency`) |

**Hunts (spec 6.10)**

| Fact | Guessed | SDK path |
| --- | --- | --- |
| `PH_BY_TAG_QUERIES` | `MatchmakerHuntsByTag[].Queries` | `MatchmakerHuntsByTag[].HuntTags` (element `FMatchmakerHuntList`; the JSON `queries` array is written from it) |
| `PH_VISIBLE_LOCKED` | `bVisibleWhileLocked` | `FPlayerHuntTableData.IsVisibleWhileLocked` |
| `ES_ROUNDS` | `RoundStructure` | `FEscalationModeSpecification.RoundStructures` |
| `UC_QUEST_STATUS` | `QuestRequirements[].StatusRequired` | `QuestRequirements[].QuestStatusRequired` |
| `SD_REPEATABLE` | `bIsRepeatable` | `FScheduleData.IsRepeatable` |
| `SD_EACH_DAY`, `SD_EACH_WEEK` | `bEachDay`, `bEachWeek` | `FScheduleData.RepeatableDetails.EachDay`, `.RepeatableDetails.EachWeek` (a nested struct) |
| `SD_SHOULD_QUEUE` | `bShouldQueue` | `FScheduleData.bShouldQueueTheSheduleItems` (the game's spelling) |
| `SD_ITEM_ID` | `ScheduledItems[].Id` | `ScheduledItems[].ID` |
| `SD_LEFT_OUT` | `ItemsLeftOut` | `FScheduleData.ItemsToBeLeftOutOfTheRotation` |

**Store runtime (spec 6.13)**

| Fact | Guessed | SDK path |
| --- | --- | --- |
| `SV_CATEGORIES` | `UStoreViewModel.Categories` | `UStoreViewModel.StoreCategoryViewModels` (element `FStoreCategoryViewModels`) |
| `SV_CAT_NAME` | `Categories[].Name` | `StoreCategoryViewModels[].CategoryName` (an `FText`) |
| `SV_CAT_REQUIRES`, `SV_SUB_REQUIRES` | `bRequiresEntitlement` | `RequiresEntitlement` on both |
| `SV_CAT_SUBCATEGORIES` | `Categories[].Subcategories` | `StoreCategoryViewModels[].SubCategoryViewModels` (element `FStoreSubCategoryViewModels`) |
| `SV_SUB_NAME` | `Subcategories[].Name` | `SubCategoryViewModels[].SubCategoryName` (an `FText`) |
| `SV_SUB_HIDE_PURCHASED` | `bHidePurchasedItems` | `HidePurchasedItems` |
| other `SV_CAT_*` / `SV_SUB_*` | under `Categories` / `Subcategories` | same leaf names (`BreadcrumbCategory`, `EntitlementId`, `SkuId`, `SubTagIds`, `bShouldHideTab`, `bHideTab`) under the parents above |
| `SI_CURRENCY_ENUM` | `StoreCurrencyEnum` | `UStoreItemViewModel.StoreCurrency` |
| `SI_PURCHASED`, `SI_EXPIRED`, `SI_PRESTIGE_ELITE` | `bPurchased`, `bExpired`, `bPrestigeItemElite` | `bIsPurchased`, `bIsExpired`, `bIsPrestigeItemElite` |
| `SI_CATALOG_ITEM_ID` | `CatalogItemId` | `UStoreItemViewModel.StoreItemCatalogData.ItemId` |
| `SI_OFFER` | `@FOnlineStorePhoenixOffer`, default `+0x650` | the protected member `StoreItem`; spec 4.5 keeps the fixed `+0x650`, so the type search and default both land there if the SDK agrees |
| `OF_IMAGE_URL` | `ImageUrl` | `FOnlineStorePhoenixOffer.ImageURL` |
| `OF_CURRENCY_ENUM` | `CurrencyEnum` | `FOnlineStorePhoenixOffer.Currency` |
| `OF_IN_COLLECTION` | `bInUserCollection` | `FOnlineStorePhoenixOffer.bIsInUserCollection` |

**Progression (spec 6.6)**

| Fact | Guessed | SDK path |
| --- | --- | --- |
| `EXP_DESCRIPTION` | `ExperienceTypeDisplayData.DisplayDescription` | `ExperienceTypeDisplayData.Description` |
| `EXP_BANKING_LEVELS` | `ExperienceBankingLevelCount` | `FExperienceTrackTableData.ExperienceBankingDataPerLevel` (an array; the JSON count is its length) |
| `PT_REPEATS`, `PT_IS_PRESTIGE`, `PT_IS_CORE`, `PT_VISIBLE` | `bRepeats` ... | `FProgressionTrackTableData.Repeats`, `.IsPrestigeTrack`, `.IsCoreTrack`, `.VisibleInUI` (no `b`) |
| `MT_CATEGORY` | `Category` | `FMasteryTrackTableData.Categtory` (the game's spelling) |
| base member 2 (`B_RESET`) | `bResetWithCharacter` | `ResetWithCharacter` |

**Mastery (spec 6.14)**

| Fact | Guessed | SDK path |
| --- | --- | --- |
| `MC_OBJ_COMPOUND` | `Objectives[].CompoundId` | `FMasteryCardTableData.Objectives[].CompoundObjectiveId` (element `FMasteryObjectiveData`) |
| `MC_OBJ_TARGET`, `MC_OBJ_DATA`, `MC_OBJ_RANK`, `MC_OBJ_TRACK` | `Objectives[].Target`, `.ObjectiveData`, `.ProgressRank`, `.ProgressTrack` | `Objectives[].Objective.Amount`, `.Objective.ObjectiveDataClass`, `.Objective.ProgressRank`, `.Objective.ProgressTrack` (an `FGameplayObjective` member named `Objective`) |
| `MC_BASE_XP`, `MC_PRESTIGE_XP` | `BaseXpObjectiveId`, `PrestigeXpObjectiveId` | `FMasteryCardTableData.BaseXPBarOnlineObjectiveId`, `.PrestigeXPBarOnlineObjectiveId` |
| `AC_OBJECTIVES` | `FAchievementTableData.Objectives` | `FAchievementTableData.TrackedObjectives` (element `FTrackedObjectiveData`) |
| `AC_OBJ_*` | `Objectives[].<leaf>` | `TrackedObjectives[].UniqueOnlineObjectiveId`, and `TrackedObjectives[].Objective.Amount`, `.Objective.ObjectiveDataClass`, `.Objective.ProgressRank`, `.Objective.ProgressTrack` |

**Not checkable from the C++.** The old code reaches these through SDK helpers or raw offsets, never by member name:
`UObject.Flags`, `.Name`, `.Outer`, `UStruct.Super`, `TUObjectArray.Objects`, `.NumElements`, `FUObjectItem.Object`,
`.SerialNumber`, `UStoreViewModel.TagFilter` (fixed `+0x170`), and `FDateTime.Ticks`. For `FDateTime.Ticks`,
`SD_START` and `SD_END`, the C++ reads the 64-bit value at the start of `StartTime`/`EndTime`. The guesses follow
Dumper-7's usual spelling and may stand. The first real build will say. The rest of the guessed paths match the
members the C++ reads.

### 4.3 A behaviour that departs from the spec: soft paths

The implementer read every soft pointer's path at `+0x10` (`engine.rs` `SOFT_PATH`, through `Kind::SoftPtr`). The spec
gives `+0x10` only for the hunt exporter's soft paths (4.5). The other two are read differently:

- **Catalog `itemClass` (6.1 item 2)**: `FArchonCatalogItem.ItemClass` is a soft *class path*. Its `AssetPathName`
  is at the member's own offset, with no weak pointer in front, read in short form. At `+0x10` the exporter reads the
  sub-path string's bytes as a name. The spec should say: "`ItemClass.AssetPathName`, at the member's offset, short
  form".
- **Store images (6.13)**: the spec already says "this read goes through the SDK member". The path is
  `FStoreItemTable.FeatureImage.ObjectID.AssetPathName` (and `StandardImage`), at the SDK's offsets, short form, with
  an unset image written as `None`. The spec should spell out the path, because "the SDK member" was read as the hunts
  rule.

Both need a change to `facts.rs` and to the kind the catalog and store exports read with. The diff in section 6
would catch them too, but they are known now.

### 4.4 The implementer's other calls

Checked against the C++. Each agrees:

- `catalogGeneralItems` is the count of catalog lines written.
- `export_manifest_1_14_7.json` and `hunt_export_manifest.json` end with `}` and a newline. They are written in text
  mode, so the newline is CRLF. `mastery_objectives.json` ends with `}` and LF; it is written in binary mode, so every
  line in it is LF.
- Hunt soft paths use the raw name form (no cut at the last `/`), and `None` is emptied.

The name decoding (number suffix, wide and narrow entries) and the weak-pointer serial check can only be settled by
the diff.

### 4.5 Fixes before the diff

1. The integration pass adds section 4.2's paths and section 4.3's two soft-path reads to the spec (6.1, 6.6, 6.10,
   6.13, 6.14), as spec corrections.
2. The implementer, from the corrected spec only, fixes `facts.rs` and the two soft-path reads, and adds a synthetic
   fixture case for a soft class path.
3. Then the first real build. Any fact it still reports missing is fixed the same way, by a spec correction.

## 5. Verification

- The implementer: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test
  --workspace` in the crate (golden bytes of every phase-1 file on a synthetic memory image), and a Windows build of
  `CatalogExporter.dll` against the synthetic SDK.
- This review: the member-name check in 4.2 and the read of the C++ in 4.3 and 4.4. It built or ran nothing new.
- **Not run against the 1.14.7 client.** Nothing here shows the DLL writes the old bytes. Section 6 is that check.

## 6. The maintainer's diff on the real 1.14.7 client (spec 10)

Run this on the Windows PC with the 1.14.7 client, after section 4.5's fixes. You also need the launcher's Host page
working, the SDK imported (`restricted\modules\rustic-isles\native\runtime\` holds the dump), a local server, and a
character that reaches Ramsgate. Every command runs in PowerShell from the workspace root the Host page uses.

1. **Build the old DLL with its own root**, so its output and flags don't mix with the Host page's:

   ```powershell
   New-Item -ItemType Directory -Force C:\dl-diff\runs | Out-Null
   Push-Location restricted\modules\rustic-isles\native\catalog-exporter
   Set-Content -Encoding ascii ExportPaths.local.h '#define RUSTIC_ISLES_EXPORT_ROOT L"C:\\dl-diff\\old"'
   .\_build.bat
   Copy-Item x64\Release\CatalogExporter.dll C:\dl-diff\CatalogExporter-old.dll
   Pop-Location
   ```

2. **Build the new DLL against the real SDK** with root `C:\dl-diff\new`:

   ```powershell
   $env:RUSTIC_ISLES_EXPORT_ROOT = 'C:\dl-diff\new'
   $env:DAINTY_SDK_DIR = (Resolve-Path restricted\modules\rustic-isles\native\runtime).Path
   cargo build --release --target x86_64-pc-windows-msvc --manifest-path native\rustic-isles-exporter\Cargo.toml -p catalog-exporter-dll
   Copy-Item native\rustic-isles-exporter\target\x86_64-pc-windows-msvc\release\CatalogExporter.dll C:\dl-diff\CatalogExporter-new.dll
   Remove-Item Env:RUSTIC_ISLES_EXPORT_ROOT
   ```

   If the build stops with `the SDK lacks N fact(s)`, copy only the fact ids it lists (never SDK text) into a report
   for a spec correction. Stop here until that lands.

3. **Write the same flags file under both roots.** Turn on the phase-1 keys and nothing else: the new DLL refuses the
   research exports, and the two string-table keys stay off.

   ```powershell
   $flags = 'CATALOG','PROGRESSION','SLAYERS_PATH','HUNTS','STORE_RUNTIME','MASTERY' | ForEach-Object { "EXPORT_$_=1" }
   $flags += 'EXPORT_SKINS_RESOLVE_STRINGTABLE=0', 'EXPORT_CURRENCY_NAMES=0'
   foreach ($r in 'old', 'new') {
     New-Item -ItemType Directory -Force "C:\dl-diff\$r\native\catalog-exporter" | Out-Null
     Set-Content -Encoding ascii "C:\dl-diff\$r\native\catalog-exporter\export_flags.txt" $flags
   }
   ```

4. **Do three runs**: `old-1`, `old-2` and `new-1`, in the same game state. For each one:

   ```powershell
   $which = 'old'; $label = 'old-1'   # then 'old','old-2'; then 'new','new-1'
   $dll = 'restricted\modules\rustic-isles\native\catalog-exporter\x64\Release\CatalogExporter.dll'
   Remove-Item -Recurse -Force "C:\dl-diff\$which\Items_Analysis" -ErrorAction SilentlyContinue
   New-Item -ItemType Directory "C:\dl-diff\$which\Items_Analysis" | Out-Null
   Copy-Item "C:\dl-diff\CatalogExporter-$which.dll" $dll -Force
   ```

   Then on the Host page press **Run export**. Log in, reach Ramsgate, and open the Slayer's Path, the hunt map,
   Journal → Challenges → Reward Cache and the Mastery board, in that order every run. Wait for the beep, then quit
   the game. Note each run's counts from `export_manifest_1_14_7.json`. Keep the output:

   ```powershell
   Copy-Item -Recurse "C:\dl-diff\$which\Items_Analysis" "C:\dl-diff\runs\$label"
   ```

5. **Compare** with spec 10 step 5's `Compare-Run` (it masks `exportedAt`, `objectIndex` and the `extracted` date, and
   compares byte-exact, then masked, then line-sorted):

   ```powershell
   Compare-Run old-1 old-2   # the noise floor
   Compare-Run old-1 new-1   # MISSING only for the research files the flags turned off
   Compare-Run new-1 old-1   # files only the new one wrote
   ```

   **Pass**: each phase-1 file compares the same way or better in `old-1`/`new-1` as in `old-1`/`old-2`. For every
   `same (sorted)` file, check the line endings by hand (`Format-Hex <file> | Select-Object -First 4`: CRLF is
   `0D 0A`). `mastery_objectives.json` must be LF only. For a `DIFFERENT` file, list its first differing lines with
   `Compare-Object` (spec 10 step 5) and look first at `itemClass`, the store images and the store offsets (4.3, and
   spec 4.5's ambiguity).

6. **Clean up**: put the old DLL back at the Host page's path, or rebuild it after deleting `ExportPaths.local.h`, then
   delete `ExportPaths.local.h`. `C:\dl-diff` is game data: keep it off the repository and out of reports. Report only
   file names, verdicts and row counts.

When the diff passes, the Host page's exporter path moves to the new DLL (spec 9 seam 4), and the C++ in `restricted/`
can go.
