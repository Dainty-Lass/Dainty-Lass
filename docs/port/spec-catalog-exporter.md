<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: the CatalogExporter (RD27, port plan item 8)

This page specifies `restricted/modules/rustic-isles/native/catalog-exporter` (CatalogExporter, Mystic Paradox,
AGPL-3.0-only with `ADDITIONAL_TERMS.md`) so it can be re-created at the root under the MIT licence. The implementer
works from this page alone and never opens `restricted/`. Nothing here is code from the exporter: it is what the
exporter reads, in what order, and the exact bytes it writes.

Written against `plan` at `d80cff53`. "The SDK" means the C++ headers Dumper-7 generates from the host's own running
1.14.7 client (`docs/sdk-and-game-data.md` step 1): Phoenix Labs' material, never committed, the host's input today
and the host's input for the rewrite. "Old" is the C++ DLL, "new" the rewrite. A sentence marked **(call)** is a
decision this page made; **(ambiguity)** marks something the code leaves unclear or that only a run on the real client
settles. Both are collected in section 12.

---

## 1. Scope and provenance

| Part today | Lines | What it does | In the MIT rewrite |
| --- | --- | --- | --- |
| `dllmain.cpp` | 865 | entry thread, catalog and equipment dumps, four weapon-part diagnostics, cell slots, the run manifest | sections 3, 6.1-6.5, 6.15 |
| `ExportFlags.hpp`, `export_flags.example.txt` | 147 + 29 | the flags file | section 3.2 |
| `ExportPaths.hpp` | 60 | output folder and flags-file search | section 3.3 |
| `ProgressionExporter.cpp` | 592 | eight progression tables | section 6.6 |
| `CombatExporter.cpp` | 207 | damage and weapon-power tables | section 6.7 |
| `SkinsExporter.cpp` | 440 | weapon skins, optional string-table names | section 6.8 |
| `SlayersPathExporter.cpp` | 302 | Slayer's Path nodes | section 6.9 |
| `HuntExporter.cpp` | 665 | eight hunt tables and their manifest | section 6.10 |
| `DropTableExporter.cpp` | 197 | weighted drop tables | section 6.11 |
| `CurrencyExporter.cpp` | 324 | currency names through string tables | section 6.12 |
| `StoreRuntimeExporter.cpp` | 431 | store view models and store image rows | section 6.13 |
| `MasteryExporter.cpp` | 357 | mastery cards and achievements | section 6.14 |
| `TableInventoryExporter.cpp` | 179 | a census of loaded data tables | section 6.16 |
| `ExportFlags.md` | 498 | research notes (several statements are stale; the code wins, section 12) | not carried |
| `CatalogExporter.vcxproj`, `_build.bat`, `.gitignore` | 75 + 13 | the build | section 2.1 |

All of it is Paradox's (headers name MysticFox / Pranav Karande); `git log` shows only the import (`772248c2`) and one
rename fix (`5890e60e`) here. No Dainty Lass patch touches it (`restricted/docs/paradox-patches.md` names the folder
only in its layout note). 5,381 lines in all, 4,883 of them C++.

## 2. How it is built and run today

### 2.1 Build

- One MSBuild project, configuration `Release|x64` only: a `DynamicLibrary`, toolset `v145`, Unicode, whole-program
  optimisation, C++ `stdcpplatest`, warning level 1, multi-processor compile, no precompiled header, debug info on.
- It compiles its twelve `.cpp` files plus four files of the SDK (`SDK\Basic.cpp`, `SDK\CoreUObject_functions.cpp`,
  `SDK\Engine_functions.cpp`, `SDK\Archon_functions.cpp`), taken from `$(SdkRoot)`, which is `..\runtime\`: the SDK
  imported into the runtime's folder. There is no other dependency (no MinHook, no libraries).
- `_build.bat` finds MSBuild with `vswhere -latest -products * -requires Microsoft.Component.MSBuild` and builds the
  project `Release|x64`; exit code 3 when vswhere or MSBuild is missing, otherwise MSBuild's.
- Output: `x64\Release\CatalogExporter.dll` beside the project. The host kit and the launcher look for exactly that
  path (`crates/dainty-hostkit/src/paths.rs` `exporter_dll`, `crates/dainty-launcher-kit/src/platform/hostkit.rs`).
- The DLL must be built against the same SDK as the client it runs in: the struct layouts and engine offsets are
  compiled in.

### 2.2 Run

The DLL runs **inside the game process**, loaded by the Host page's export session (RD9 as amended in
`docs/launcher-injection.md`). The session
(`crates/dainty-launcher-kit/src/platform/loader_session.rs`, `controller/host.rs`) does this:

1. writes `export_flags.txt` into the game's `Win64` folder with this exact content (CRLF line endings, as
   `EXPORT_FLAGS` in `platform/hostkit/parse.rs`): a comment line
   `# Written by the Dainty Lass launcher (Host page). See docs/sdk-and-game-data.md.`, then
   `EXPORT_CATALOG=1`, `EXPORT_SLAYERS_PATH=1`, `EXPORT_HUNTS=1`, `EXPORT_STORE_RUNTIME=1`, `EXPORT_PROGRESSION=1`,
   and `=0` for `EXPORT_COMBAT`, `EXPORT_SKINS`, `EXPORT_SKINS_RESOLVE_STRINGTABLE`, `EXPORT_WEAPON_SLOTS`,
   `EXPORT_TABLE_INVENTORY`, `EXPORT_CELLS`, `EXPORT_DROP_TABLES`, `EXPORT_CURRENCY_NAMES`. `EXPORT_MASTERY` is absent,
   so it is off (section 3.2);
2. starts the game as PLAY starts it (connected to the host's own server, so the player can reach Ramsgate). On
   Windows it starts it suspended and injects the runtime, then `CatalogExporter.dll`, before it runs. Under Proton
   with the loader, it copies the DLL beside `winmm.dll` and writes `mystic_loader.ini` naming only it;
3. when the game exits, removes any DLL and ini it wrote. The Host page then copies `Win64\Items_Analysis` into the
   workspace's `modules/rustic-isles/reference/1.14.7/Items_Analysis`, which the generators read.

Either way the DLL loads before the game's first instruction, so `DllMain` runs before the game reaches its menu; the working directory is
`Win64`. By hand, any loader or injector into the local client works the same way; the guide
(`restricted/modules/rustic-isles/docs/GENERATING_GAME_DATA.md`) says to inject into one's own local client only.

## 3. The process

### 3.1 Entry

On `DLL_PROCESS_ATTACH` it disables thread-library calls and starts one worker thread; `DllMain` returns TRUE at once.
Nothing happens on detach; the DLL is never unloaded and installs no hooks. The worker, in order:

1. reads the flags (3.2) and appends the status line `thread start; flags: catalog=<0|1> progression=… combat=…
   skins=… skinsResolveStringTable=… slayersPath=… weaponSlots=… hunts=… tableInventory=… cells=… dropTables=…
   currencyNames=… storeRuntime=…` (one space between pairs; mastery is not listed);
2. if `EXPORT_SKINS_RESOLVE_STRINGTABLE` or `EXPORT_CURRENCY_NAMES` is on, appends a warning line each (text in 6.17);
3. appends `waiting for GObjects + catalog...`;
4. **waits for the catalog**: up to 240 tries, 500 ms apart (about 120 s). A try succeeds when the global object array
   exists, has more than 0 entries, and holds a live `UArchonCatalog` (4.2). On timeout it appends
   `TIMEOUT: catalog never appeared. Reach the main menu/Ramsgate, then re-inject.` and the thread ends: no file other
   than the status file is written;
5. runs the enabled exports in this fixed order, appending `<FLAG>=0, skipping <what>` for each one that is off (texts
   in 6.17): catalog (6.1, 6.2), weapon slots (6.3), cells (6.4), progression, combat, skins, Slayer's Path, hunts,
   drop tables, currency names, store runtime, mastery, table inventory;
6. writes the run manifest (6.15), appends the `ALL DONE ...` line, and calls `MessageBeep(MB_OK)`;
7. **closes the game** (a departure from the old DLL, which left it running; maintainer, 2026-11-13): shows a topmost
   message box, title `CatalogExporter`, text `Export complete, closing the game.` (English: a host's tool), on a
   thread of its own. Once the box is dismissed or 10 s have passed, whichever is first (a box hidden behind a
   fullscreen game must not hold it open), it posts `WM_CLOSE` to each top-level window of the game's process, the
   game's own quit. If the game is still running 15 s later, it ends it with `TerminateProcess` and exit code 0.
   `ExitProcess` is not used: called from the export thread it faults in `libcef.dll`'s teardown (0xC0000005, seen on
   1.14.7 on 2026-11-13). A run that ends early (step 4's timeout, a synthetic build) does none of this, and the game
   stays open.

There is **no wait between the catalog appearing and the exports**: every export reads what is loaded at that moment.
**(ambiguity)** With the loader starting the DLL at process start, the catalog appears at the main menu, before the
player can open the Slayer's Path, the hunt map or the Reward Cache, so those exports find nothing loaded unless the
catalog shows up late. The guide's "open the listed screen while it runs" cannot work as the code stands. The new one
keeps this order and timing by default (byte parity), and section 12 asks whether to add a wait.

Each export is independent: one that finds nothing or fails returns a count of 0 or -1 and the next one runs.

### 3.2 The flags file

- Found by the search in 3.3; read once at thread start; no reload.
- **No file** (none of the candidates exists, or it cannot be opened): `EXPORT_CATALOG` on, everything else off.
- **A file**: read as bytes, line by line on `\n`. Each line has trailing `\r`, spaces and tabs removed, then leading
  spaces and tabs. Empty lines and lines starting with `#` are skipped; a line without `=` is skipped. Key is
  everything before the first `=`, value everything after, both kept as they are (no further trimming, so
  `KEY = 1` is the key `KEY ` and never matches). A value is on when it is exactly `1`, `true` or `TRUE`; any other
  value is off. A later line for the same key replaces an earlier one.
- Once a file is found, **every key absent from it is off**, `EXPORT_CATALOG` included.
- The fourteen keys: `EXPORT_CATALOG`, `EXPORT_PROGRESSION`, `EXPORT_COMBAT`, `EXPORT_SKINS`,
  `EXPORT_SKINS_RESOLVE_STRINGTABLE`, `EXPORT_SLAYERS_PATH`, `EXPORT_WEAPON_SLOTS`, `EXPORT_HUNTS`,
  `EXPORT_TABLE_INVENTORY`, `EXPORT_CELLS`, `EXPORT_DROP_TABLES`, `EXPORT_CURRENCY_NAMES`, `EXPORT_STORE_RUNTIME`,
  `EXPORT_MASTERY`. Unknown keys are ignored. `EXPORT_SKINS_RESOLVE_STRINGTABLE` only matters with `EXPORT_SKINS`.

### 3.3 Where it reads and writes

A build may carry an **export root**, fixed at compile time (old: `RUSTIC_ISLES_EXPORT_ROOT`, or the older
`MYSTICPARADOX_EXPORT_ROOT`, defined in a git-ignored `ExportPaths.local.h`). Without one, only the relative
candidates exist.

- **Flags file candidates**, first existing regular file wins: `<root>\native\catalog-exporter\export_flags.txt`, then
  `.\export_flags.txt` (relative to the game's working directory).
- **Output folder** (`<out>` below), resolved again at every use: the first of `<root>\Items_Analysis` and
  `.\Items_Analysis` that exists as a folder; when neither does, the first candidate is created (not recursively) and
  used. Without a root that is `.\Items_Analysis` in `Win64`.
- Subfolders are created under `<out>` by the exports that use them (not recursively; an existing one is fine).

**(call)** The new one keeps the compile-time root, with the same path layout, so the hands-on diff (section 10) can
send old and new to different folders. In Rust it is read at build time from the build environment variable
`RUSTIC_ISLES_EXPORT_ROOT` (`option_env!`); the old header name is not carried.

## 4. What it reads from the game, and how

It reads only **live process memory**, through the SDK's types. It opens no pak, no asset file, no save and no
network connection, and it reads no file of the game's. (The `game-data/mastery_objectives.json` that the backend
ships was extracted statically from the paks by other means; the exporter's mastery mode has never run, 6.14.)

### 4.1 The engine primitives it relies on

| Primitive | Used for | Notes |
| --- | --- | --- |
| The global object array (`GObjects`): its count and the object at an index | every walk | walked from index 0 to count-1, null slots skipped. Location: the SDK's offsets for this build |
| `UObject`: class pointer, name, `IsA(class)`, "is a class default object", full name | filters and names | "default object" is the object's class-default flag; the equipment dump instead checks that its name starts `Default__`. `IsA` walks the class's parents through the SDK member `UStruct.SuperStruct` |
| `StaticClass()` of a named class | filters | the SDK finds the class by name and caches it; a pure read |
| Name to text (`FName`) | every id | see 4.3 |
| `FString` to text | every authored string | empty when its length is 0 or its data pointer is null |
| `FText` to text | display text in progression, store and skins | reads the text's display string. Reading an `FText` as anything else faults the process (Slayer's Path v1 crashed so); the exports that read `FText` do it through the guard (4.4) |
| `TArray<T>` | lists | data pointer, count. Counts are read as signed 32-bit |
| `TMap` (a data table's `RowMap`, name to row pointer) | rows | iterated in the map's own order, skipping free slots of its sparse array |
| `UDataTable`: `RowStruct` (a struct object whose name is compared), `RowMap` | every table export | a table is used only when `RowStruct` is non-null and its name matches |
| `FDataTableRowHandle` | references | `{DataTable pointer, RowName}` |
| `FDateTime` | schedule dates | the int64 at offset 0 of the `FDateTime` member, no member lookup inside it: the SDK's `FDateTime` has no named member, only 8 bytes of padding |
| Soft object and soft class pointers | asset paths | hunts read the path name at a fixed offset (4.5); the catalog's item class (6.1) and the store images (6.13) read it through SDK members |
| `UFunction` calls through `ProcessEvent` | the catalog, live inventory, string tables | 4.2. Every SDK wrapper of a reflected function dispatches through `ProcessEvent` on the worker thread |

### 4.2 Calls into game code

Not everything is a pure read. These are calls the game executes, on the exporter's worker thread (not the game
thread):

| Call | Where | Purpose |
| --- | --- | --- |
| `UArchonCatalog::GetAllItems(out TArray<FArchonCatalogItem>)` | catalog (6.1), skins (6.8), currency (6.12) | the whole general catalog. The live catalog is the first non-default object of class `ArchonCatalog` in `GObjects` order |
| `UArchonInventoryItem_Weapon::GetItemPartSlots()`, `GetEquippedItemParts()`, `IsItemPartSlotLocked(type)` | weapon slots (6.3) | live inventory |
| `UArchonInventoryItem_CellContainer::GetCellSlots()`, `GetPermanentCells()`, `GetAllPermanentCellEffects()` | cells (6.4) | live inventory |
| `UKismetStringLibrary::Conv_StringToName`, `UKismetStringTableLibrary::IsRegisteredTableId`, `GetTableEntrySourceString`, `GetRegisteredStringTables`, `GetKeysFromStringTable`, and the two libraries' default objects | skins with string tables (6.8), currency (6.12) | resolving display names |

These are why the guide says "your own local client only". `GetAllItems` alone makes the default run (catalog on)
call game code.

**(call)** The new one may read `UArchonCatalog`'s catalog map directly instead of calling `GetAllItems`, **only if**
the hands-on diff (section 10) shows the same rows in the same order; until then it calls the function, so parity
holds by construction.

### 4.3 Names

The SDK's name-to-text has **two forms**, and the bytes depend on which one each field uses:

- **Short form** (the SDK's `ToString`): the name's text with its number suffix, then cut after the last `/`: if the
  text contains a `/`, only what follows the last one is kept. Every field in section 6 uses this form unless it says
  "raw".
- **Raw form** (`GetRawString`): the full text, no cut. Only the hunt exporter's soft paths use it (4.5).

An unset name reads as `None`. **(ambiguity)** Whether the number suffix (`_<n-1>` for a name with number n > 0) is
part of both forms is the SDK's, not the exporter's; the new one must match what the host's SDK does (check the SDK's
`FName::ToString`/`GetRawString` in `UnrealContainers.hpp`/`Basic.hpp`, or settle it in the diff).

**(ambiguity)** How the SDK turns an engine `FString`/name (UTF-16) into bytes (UTF-8, or each code unit cut to one
byte) is the SDK's too. All ids are ASCII; display names may not be. The new one writes UTF-8; if the diff shows the old
one truncating, section 12 decides whether parity or UTF-8 wins (every consumer reads lossy UTF-8).

### 4.4 Faults

The old one guards some reads with structured exception handling (an access violation becomes "skip" instead of a
crash). Where it does, the fallback is part of the output:

| Export | Guarded | On a fault |
| --- | --- | --- |
| catalog, equipment, weapon slots, cells, combat, Slayer's Path, skins | nothing | the game crashes |
| progression, hunts, store runtime | each string, name and text read (fault: empty text); each whole row | the row is skipped with a status line |
| drop tables | each read and each row | as above |
| mastery | each name and string read, each class-name read | empty text |
| table inventory | each table's name, row count and row-struct name | the table is skipped (row struct: empty) |

The new one must never crash where the old one did not, and may be safer where the old one crashed (it writes the same
bytes on every run that did not crash). **(call)** It guards every read: a read of memory that faults yields the same
fallback the guarded exports use (empty text, row skipped).

### 4.5 Fixed offsets

These are read at fixed offsets rather than through SDK members, and the new one must use the same:

| Read | Offset | Why |
| --- | --- | --- |
| A soft pointer's path name (hunts) | `+0x10` from the soft pointer | the SDK sizes the weak pointer part wrongly; the real layout is 8 bytes weak pointer, 4 bytes tag, 4 bytes padding, then the path (name 8 bytes, sub-path string 16). Every soft pointer read uses it: hunts (raw form, a path that reads as `None` becomes empty) and the store images (6.13, short form). 6.1's item class is a soft class path, not a soft pointer |
| Equipment item id | `+0x28` (index, signed 32-bit) and `+0x2C` (number) on the object | read as a name; an index `<= 0` means no id |
| `UStoreViewModel` tag filter | `+0x170` (an `FString`) | a protected field |
| `UStoreItemViewModel` offer | `+0x650` (an `FOnlineStorePhoenixOffer`, the protected member `UStoreItemViewModel.StoreItem`) | read directly; the SDK's getter would dispatch `ProcessEvent` |

**(ambiguity)** The two store offsets are documented as the 1.12 SDK's; nothing shows they were re-checked for 1.14.7.
The new one takes them from the host's SDK at build time like every other layout (section 9), so a moved field reads
correctly in the new one and wrongly in the old; the diff would show it.

Every other layout (struct member offsets, enum values, class names) comes from the host's SDK.

### 4.6 What the exports filter on

| Export | Objects or tables it takes |
| --- | --- |
| catalog | `GetAllItems` of the live catalog |
| equipment | every object whose class's **short name** is `EquipmentCatalogItem_Weapon`, `_Armour`, `_Lantern`, `_PlayerRole` or `_WeaponPart` and whose own name does not start `Default__` |
| weapon slots | non-default objects that are a `ArchonInventoryItem_Weapon` (live slots), `ArchonInventoryItem_WeaponPart` (live parts), data tables with row struct `PlayFabWeaponPartCatalogTableData`, non-default `LoadoutGearViewModel` and `LoadoutGearScreen` objects |
| cells | non-default `ArchonInventoryItem_CellContainer` objects |
| progression | tables found **by object name** (6.6), their row struct checked; experience awards **by row struct** |
| every other table export | every data table whose row struct's name matches (by row struct), in `GObjects` order |
| store runtime | objects that are a `StoreViewModel` or a `StoreItemViewModel` (**default objects included**), tables with row struct `StoreItemTable` |

A data table is any object that `IsA` `DataTable`, default objects included (they have no rows).

## 5. Output conventions (all files unless a section says otherwise)

- **Line endings: CRLF.** Every file is written through a text-mode stream, so each `\n` written becomes `\r\n` on
  disk. The one exception is `mastery_1_14_7\mastery_objectives.json`, written in binary: LF.
- **No BOM.** Bytes as produced by name/string conversion (4.3); no re-encoding.
- **Opening**: an export opens its file truncating it, writes, closes. Where an export finds nothing it may return
  **before** opening, leaving a file from an earlier run untouched; each section says which. This matters for the
  diff: delete `<out>` before every run.
- **JSON lines** (`.jsonl`): one object per line, no spaces anywhere (no space after `:` or `,`), every line ended
  by `\n` (so CRLF), the last line too. Empty output is a 0-byte file.
- **String escaping** (every quoted value): `"` as `\"`, `\` as `\\`, newline `\n`, carriage return `\r`, tab `\t`;
  every other byte below 0x20 as `\u00XX` with two **lowercase** hex digits; every other byte, 0x7F and all bytes
  >= 0x80 included, as it is.
- **Numbers**: integers in plain decimal (enum values as their integer; `uint8` enums as 0-255). **Floats as C's
  `%f`**: six digits after the point, no exponent (`1.000000`, `-0.500000`, `0.000000`); NaN and infinities as the
  platform's `%f` writes them **(ambiguity: none observed)**.
- **Booleans**: `true` / `false`. **Absent pointer** (no table, no class): empty string `""`.
- **Rows** of one table are written in **ascending byte order of the row name** (the name in short form); for each
  name the first map entry with that name supplies the row. **(ambiguity)** Two rows whose names read the same would
  both print the first one's data; not seen, and the new one must do the same for parity. Tables of one row struct are
  written one after another in `GObjects` order.
- Field order is exactly as listed in section 6. A field listed as `name: what` is written `"name":<value>`.
- **Names read as `None`** are written as `"None"` unless a section says they are dropped or emptied.

## 6. Every file it writes

`<out>` is the output folder (3.3). The `_1_14_7` suffix is fixed in the code, not read from the game. Type notation:
**s** quoted string, **n** integer, **f** float (`%f`), **b** boolean, **[x]** array of x, **{...}** object.

### 6.1 `catalog_1_14_7.jsonl` (`EXPORT_CATALOG`)

Calls `GetAllItems` (4.2) and appends `general: GetAllItems returned <n> items`. When the catalog is not found:
`general: UArchonCatalog instance NOT found yet` (cannot happen after 3.1 step 4). When n <= 0 the file is **not
opened**. Otherwise one line per item, in the returned order, skipping items whose `ItemId` is empty:

1. `itemId`: s, `ItemId`
2. `itemClass`: s, `ItemClass.AssetPathName`, **short form** (so the text after the last `/`). `ItemClass` is a soft
   *class path* (an `FSoftObjectPath`), not a soft pointer: its `AssetPathName` is at the member's own offset plus the
   SDK's offset of `AssetPathName`, with no weak pointer in front, so the `+0x10` of 4.5 does not apply
3. `displayName`: s, `DisplayNameInvariant` (an `FString`, not the `FText`)
4. `description`: s, `DescriptionInvariant`
5. `tags`: [s], `Tags`
6. `customData`: s, `CustomData` (itself a JSON text, escaped as any string)
7. `entitlements`: [s]
8. `isStackable`, `isBundle`, `isContainer`: b, `IsStackable`, `IsBundle`, `IsContainer` (no `b` prefix)
9. `maxQuantity`: n
10. `containerItemContents`: [s]
11. `bundledItems`: [s]
12. `containerItemContentsWithQuantity`: [{`itemId`: s, `quantity`: n}] (from `Item`, `Amount`)
13. `containerResultTableContents`: [s]
14. `containerResultTableContentsWithQuantity`: [{`itemId`, `quantity`}]
15. `bundledItemsWithQuantity`: [{`itemId`, `quantity`}]
16. `bundledResultTables`: [s]
17. `bundledResultTablesWithQuantity`: [{`itemId`, `quantity`}]
18. `otherItemQuantities`: [{`itemId`, `quantity`}]
19. `alternateItemsToGrant`: [{`itemId`, `quantity`}]
20. `virtualCurrencyPrices`: [{`currency`: n (the currency enum), `amount`: n}], from `VirtualCurrencyPrices`, whose
    element (`FPlayFabCatalogCurrency`) holds `CurrencyType` and `Amount`
21. `journeyUnlockRow`: s, `PlayerJourneyUnlockRow.RowName`

Status: `general: wrote <n> definitions -> catalog_1_14_7.jsonl`; on open failure `general: cannot open
catalog_1_14_7.jsonl`.

### 6.2 `equipment_1_14_7.jsonl` (`EXPORT_CATALOG`)

Always opened (truncated). One line per matching object (4.6) in `GObjects` order whose id (4.5) is non-empty:
`itemId`: s, `type`: s (`weapon`, `armour`, `lantern`, `player_role`, `weapon_part` for the five classes in 4.6's
order), `class`: s (the class's short name). Status: `equipment: wrote <n> definitions -> equipment_1_14_7.jsonl`.
Then `catalog DONE general=<n> equipment=<n>`.

### 6.3 Weapon slots (`EXPORT_WEAPON_SLOTS`): four files

Each is always opened. Two name tables are used:

- **Weapon part type names** (enum value to text): `default`, `dp_barrel`, `dp_receiver`, `dp_grip`, `dp_mod`,
  `dp_prism`, `dp_legendary`, `eb_special`, `eb_passive`, `eb_legendary`, `ih_special`, `ih_passive`,
  `ih_legendary`, `ga_special`, `ga_passive`, `ga_legendary`, `cb_special`, `cb_passive`, `cb_legendary`,
  `ms_special`, `ms_passive`, `ms_legendary`, `ac_special`, `ac_passive`, `ac_legendary`, `soul`,
  `lantern_capacitor`, `lantern_cell`, `tracker`, for the enumerators `Weapon_Part_Default`, `Weapon_Part_DP_Barrel`,
  ... in that order of names (the SDK's `EWeaponPartType` names each one: `Weapon_Part_<X>` for weapon parts,
  `Item_Part_Lantern_Capacitor`, `Item_Part_Lantern_Cell`, `Weapon_Part_Soul`, `Weapon_Part_Tracker`); any other value
  `unknown`. The mapping is by enumerator **name**, so the integer values come from the SDK.
- **Cell type names** (6.4).

`weapon_slots_1_14_7.jsonl`, one line per live weapon with a non-empty `ItemId`: `itemId`: s, `instanceId`: s
(`ItemInstanceID`), `weaponType`: n, `slots`: [{`slotIndex`: n, `weaponPartType`: n, `weaponPartTypeName`: s,
`defaultWeaponPartId`: s, `catalogLocked`: b (`bIsLocked`), `hideFromUI`: b, `effectiveLocked`: b (the
`IsItemPartSlotLocked` call for that slot's type)}], `equippedParts`: [{`slotIndex`, `weaponPartType`,
`weaponPartTypeName`, `weaponPartId`: s (`WeaponPartID.ItemId`)}]. Status `weapon-slots: wrote <n> live weapons ->
weapon_slots_1_14_7.jsonl`; when the class is missing `weapon-slots: UArchonInventoryItem_Weapon class unavailable`.

`live_weapon_parts_1_14_7.jsonl`, one line per live part with a non-empty `ItemId`: `itemId`: s, `instanceId`: s,
`ownerWeaponType`: n, `objectClass`: s, `catalogName`: s (the part's `Catalog` table name, or empty),
`catalogRowStruct`: s, `catalogRowFound`: b, `catalogWeaponPartType`: n (-1 when not found), `catalogIsUnassignable`: n
(1, 0, or -1 when not found), `clientIsAssignable`: b (true only when `catalogIsUnassignable` is 0). The row is the
first entry of the part's `Catalog` table whose row name equals the part's `ItemId`, read as
`PlayFabWeaponPartCatalogTableData`, only when that table's row count is in `[0, 200000)`.

`weapon_part_catalog_rows_1_14_7.jsonl`: for each table with row struct `PlayFabWeaponPartCatalogTableData` and a row
count in `[0, 200000)`, its rows in **map order** (not sorted): `tableName`: s, `rowName`: s, `weaponPartType`: n,
`weaponPartTypeName`: s, `isUnassignable`: b, `clientIsAssignable`: b (its negation).

`loadout_gear_ui_1_14_7.jsonl`, in `GObjects` order, two kinds of line (an object matching both writes both, view
model first):

- view model: `kind`: `"gearViewModel"`, `objectName`: s, `objectClass`: s, then from its `ItemViewModel`: `itemId`,
  `itemInstanceId`: s, `weaponType`: n, `itemWeaponPartType`: n, `requiredSlayerNodeId`: s, `isRequiredNodeUnlocked`:
  b; `partSlotCount`: n, `partCandidateCount`: n, `partSlots`: [{`arrayIndex`: n, `isLocked`: b, `hasPartEquipped`:
  b, `rarity`: n}], `partCandidates`: [{`arrayIndex`: n, `itemId`, `itemInstanceId`, `weaponType`, `weaponPartType`,
  `weaponPartTypeName`, `isEquipped`, `hasBeenCrafted`, `canBeCrafted`: b, `requiredSlayerNodeId`: s,
  `isRequiredNodeUnlocked`: b}];
- screen: `kind`: `"screen"`, `objectName`, `objectClass`, `hasGearViewModel`: b, `hasPartSlotBox`: b,
  `isAssignPanelOpen`: b.

Status lines `weapon-parts: wrote <n> live parts -> live_weapon_parts_1_14_7.jsonl`, `weapon-part-catalog: wrote <n>
rows -> weapon_part_catalog_rows_1_14_7.jsonl`, `loadout-gear-ui: wrote <n> rows -> loadout_gear_ui_1_14_7.jsonl`.
Only `weapon_slots`' count reaches the manifest.

### 6.4 `live_cell_slots_1_14_7.jsonl` (`EXPORT_CELLS`)

Always opened. One line per live cell container with a non-empty `ItemId`: `itemId`, `instanceId`, `objectClass`: s;
`cellSlots`: [{`slotIndex`: n, `cellType`: n, `cellTypeName`: s, `cellRarity`: n}]; `permanentCells`:
[{`permanentSlotIndex`: n, `cellRowName`: s (row handle's row name), `cellTableName`: s}]; `permanentCellEffects`:
[{`cellEffectId`: s, `cellEffectRowName`: s, `magnitude`: f, `rank`: n}].

Cell type names, by `ECellType` enumerator name: `None` `none`, `CellType_Generic` `generic`, `_Weapon_Generic`
`weapon_generic`, `_Weapon_Axe` `weapon_axe`, `_Weapon_Sword` `weapon_sword`, `_Weapon_Hammer` `weapon_hammer`,
`_Weapon_CBlades` `weapon_cblades`, `_Weapon_Spear` `weapon_spear`, `_Armour_Generic` `armour_generic`,
`_Armour_Head` `armour_head`, `_Armour_Chest` `armour_chest`, `_Armour_Arms` `armour_arms`, `_Armour_Legs`
`armour_legs`, `_Lantern_Generic` `lantern_generic`, `_Lantern_Ability` `lantern_ability`, `_Defence` `defence`,
`_Power` `power`, `_Mobility` `mobility`, `_Technique` `technique`, `_Utility` `utility`, `_Gun_Receiver`
`gun_receiver`, `_Gun_Grip` `gun_grip`, `_Gun_Barrel` `gun_barrel`, `_Gun_Passive` `gun_passive`, `_Any` `any`,
`_Legendary_Ability` `legendary_ability`, `Max` `max_sentinel`, anything else `UNNAMED`. Status `cells: wrote <n> live
cell containers -> live_cell_slots_1_14_7.jsonl`. Not in the manifest.

### 6.5 Shared: the `ExportTable`/`ExportByRowStruct` pattern

Most table exports below share one shape; deviations are listed per section.

- **By row struct**: find the tables (4.6). **None**: one status line, return 0, the file is **not opened**.
- Open the file. For each table: a row count outside `[0, 200000)` skips that table with a status line; otherwise its
  rows sorted (5), each serialized and written; a status line `wrote <k>/<count> ... from <table>`.

### 6.6 Progression (`EXPORT_PROGRESSION`): `<out>\progression_1_14_7\`, eight files

Seven tables found **by exact object name** (the first data table in `GObjects` order with that name), each checked
before any row is read: not found (status `table not found, skipping`), null row struct, a row struct whose name
differs from the expected one (status `RowStruct mismatch on <t>: expected <x> got <y> - skipping (never guessing a
wrong cast)`), or a row count outside `[0, 200000)`: the file is **not opened**. Rows sorted; each row serialized
under the guard (a fault skips the row).

| File | Table name | Row struct |
| --- | --- | --- |
| `player-experience-tracks.jsonl` | `player_experience_track_table` | `ExperienceTrackTableData` |
| `weapon-experience-tracks.jsonl` | `weapon_experience_track_table` | `ExperienceTrackTableData` |
| `experience-tracks.jsonl` | `experience_track_table` | `ExperienceTrackTableData` |
| `huntpass-season-table.jsonl` | `hunt_pass_season_table` | `HuntPassSeasonDataTable` |
| `experience-awards.jsonl` | by row struct `ExperienceTableData` (below) | |
| `progression-tracks.jsonl` | `progression_track_table` | `ProgressionTrackTableData` |
| `online-track-info.jsonl` | `online_progression_track_info_table` | `OnlineProgressionTrackInfo` |
| `mastery-tracks.jsonl` | `mastery_track_table` | `MasteryTrackTableData` |

Written in that order. **Base object** (`base`, shared by experience, hunt-pass and mastery rows): `progressionTrack`:
s, `premiumGatingEntitlement`: s, `resetWithCharacter`: b, `progressionMultiplier`: f, `requirementCount`: n,
`requirements`: [{`rankId`: n, `requiredXP`: n}] in array order, `freeRewards`, `premiumRewards`: [reward]. A
**reward**: `rankId`: n, `customRewardString`: s (an `FText`), `stackedItems`: [{`catalogId`: s, `quantity`: n,
`priority`: n}], `instancedItems`: [s], `orderedInstancedItems`: [{`catalogId`: s, `priority`: n}], `entitlements`:
[{`entitlementId`: s, `duration`: n}], `buffCount`: n.

- Experience track row: `rowName`: s, `base`, `prestigeTrack`: s (row handle's row name), `expDisplayShortName`,
  `expDisplayName`, `expDisplayDescription`: s (`FText`s of its experience-type display data),
  `experienceBankingLevelCount`: n.
- Hunt-pass season row: `rowName`, `base`, `mustClaimRewards`: b, `seasonTitle`, `seasonDescription`, `seasonDate`: s
  (`FText`), `huntRewardItemId`: s, `huntRewardItemAmount`: n, `nextProgressionTrack`, `previewsProgressionTrack`: s,
  `buyLevelsUsingRank`, `giveSeasonalCurrencies`: b, `overallSeasonName`: s (`FText`).
- Experience award row: `rowName`, `experienceType`: n, `amount`: n, `objectiveCount`: n, then `table`: s (the source
  table's name) as the **last** field. Tables: every one by row struct `ExperienceTableData`; when there is none, the
  `ExperienceTable` of the first live `PlayerExperienceComponent` whose table's row struct is `ExperienceTableData`
  (status `experience awards: using UPlayerExperienceComponent::ExperienceTable composite fallback (<name>)`). None at
  all: status, return 0, **not opened**.
- Progression track row: `rowName`, `displayName`: s (`FText`), `currentVersion`: n, `repeats`, `isPrestigeTrack`,
  `isCoreTrack`, `visibleInUI`: b, `prestigeTrack`: s (a name), `rankTable`: s (table name or empty),
  `unlockConditionCount`, `challengeSlotCount`, `craftItemRewardAmount`, `upgradeItemRewardAmount`: n.
- Online track info row: `rowName`, `displayName`, `progressDisplaySingular`, `progressDisplayPlural`,
  `progressDescription`: s (`FText`s).
- Mastery track row: `rowName`, `base`, `displayName`, `shortDisplayName`: s (`FText`), `isUIEnabled`: b, `category`: n.

SDK members where the JSON name does not give them:

- base: `resetWithCharacter` is `ResetWithCharacter` (no `b` prefix).
- experience track: `expDisplayShortName`, `expDisplayName`, `expDisplayDescription` are
  `FExperienceTrackTableData.ExperienceTypeDisplayData.ShortDisplayName`, `.DisplayName` and `.Description`;
  `experienceBankingLevelCount` is the length of the array `FExperienceTrackTableData.ExperienceBankingDataPerLevel`.
- progression track: `repeats`, `isPrestigeTrack`, `isCoreTrack`, `visibleInUI` are
  `FProgressionTrackTableData.Repeats`, `.IsPrestigeTrack`, `.IsCoreTrack`, `.VisibleInUI` (no `b` prefix).
- mastery track: `category` is `FMasteryTrackTableData.Categtory` (the game's spelling).

Status prefix `[Progression] `. The run total counts written rows, failures as 0.

### 6.7 Combat (`EXPORT_COMBAT`): `<out>\combat_1_14_7\`

By row struct, pattern 6.5, no row guard. `damage-tables.jsonl` (`DamageTableData`): `table`, `rowName`: s,
`baseDamage`: f, `physicalTypeOverride`: n, `explicitPartBreak`, `explicitStagger`, `explicitExpose`, `damageTrue`,
`physicalDamage`, `flameDamage`, `frostDamage`, `shockDamage`, `radiantDamage`, `umbralDamage`, `terraDamage`,
`rawElementalDamage`, `elementalHealthDamageMod`, `meter`: the SDK's member types, each written as an integer or `%f`
by its type **(ambiguity: the member types are the SDK's; write each as its C type prints)**, `assetTagCount`: n.
`weapon-power-tables.jsonl` (`WeaponPowerTableData`): `table`, `rowName`, `globalPower`, `globalPowerPerLevel`,
`physicalType`: n, `elementalType`: n, `elementalPower`, `elementalPowerPerLevel`, `flamePower`, `frostPower`,
`shockPower`, `radiantPower`, `umbralPower`, `terraPower`, `rawElementalPower`, `staminaCostScale`,
`staminaRegenScale`, `freezeAmountPerHit` (by type), `tagCount`, `refAbilityCount`: n. Status prefix `[Combat] `.

### 6.8 `weapon_skins_1_14_7.jsonl` (`EXPORT_SKINS`)

Calls `GetAllItems`; no catalog or n <= 0: return 0, **not opened**. With string tables on, first the diagnostic
(6.17) runs. Then the file is opened and, for each item in returned order whose tags contain `transmog` and a family tag
(the **first** tag, in tag order, that is one of `gaxe` Axe, `eblade` Sword, `ihammer` Hammer, `cblades`
ChainBlades, `dp` Repeaters, `mspear` Spear, `ac` Strikers) and whose `ItemId` is non-empty:

`itemId`, `weapon`, `itemClass` (as 6.1), `displayName`, `displayNameSource`, `description`, `descriptionSource`,
`tags`, `customData`, `virtualCurrencyPrices` (as 6.1).

**Resolving a text** (name: `DisplayNameInvariant` and `DisplayName`; description likewise): "missing" means empty or
exactly `<MISSING STRING TABLE ENTRY>`. If the invariant is not missing: it, source `invariant`. Else if the `FText`
has data and its text is not missing: that, source `text`. Else the invariant unchanged, source `none`. With string
tables on and source `none`: find the locator in `customData` (below); found and resolved to a non-missing text: that,
source `stringtable`.

**The locator**: in the raw `customData` text find `"<Block>":"` (`<Block>` is `DisplayNameLocKey` or
`DescriptionLocKey`); in the up to 600 bytes after it, find `\"Namespace\": \"` (backslash-escaped quotes, one space
after the colon) and take the text up to the next `\"`; the same for `Key`. Both non-empty: resolve with
`Conv_StringToName(namespace)` then `GetTableEntrySourceString(table, key)` (each byte widened to a UTF-16 unit).
Not found: status `  [stringtable] <id>/displayName EXTRACTION FAILED (no LocKey block found in customData)`.

### 6.9 `slayers_path_1_14_7\player_journey_nodes.jsonl` (`EXPORT_SLAYERS_PATH`)

By row struct `PlayerJourneyNodeData`, pattern 6.5, no guard. No table: status `no tables found with
RowStruct=PlayerJourneyNodeData (is the Slayer's Path screen loaded? open it once, then re-run)`, **not opened**. Per
node:

`nodeId`: s (row name), `table`: s, `nodeType`: n, `nodeLevel`: n, `childNodes`: [s] (each handle's row name),
`autoUnlockIfParentUnlocked`: b, `currencyCosts`: [{`currency`: s (the currency handle's row name), `amount`: n}],
`experienceCosts`: [{`experienceType`: n, `amount`: n}], `rewards`: [{`itemId`: s, `amount`: n, `entitlementId`: s,
`duration`: n, `passPrefix`: s, `experienceType`: n, `autoEquip`: b}] (the ids are names), `systemRewardCount`: n,
`activationEventId`: s, `questIds`: [s], `questIdToNotifyOnUnlock`: s, `tutorialToShowWhenUnlocked`: s,
`objectiveCount`: n, `gameplayAttributeValues`: {name: f, ...} in map order.

No `FText` is read (display name, descriptions). Status prefix `[SlayersPath] `.

### 6.10 Hunts (`EXPORT_HUNTS`): `<out>\hunts_1_14_7\`, eight files and a manifest

All by row struct, pattern 6.5, every string/name read and every row guarded. No table: status `no loaded table with
RowStruct=<x>; open the hunt/map UI and re-inject`, **not opened**. In this order:

| File | Row struct |
| --- | --- |
| `player_hunts.jsonl` | `PlayerHuntTableData` |
| `matchmaker_hunts.jsonl` | `MatchmakerHuntTableData` |
| `hunt_regions.jsonl` | `Hunt_Region` |
| `escalation_mode_specs.jsonl` | `EscalationModeSpecification` |
| `hunt_modifiers.jsonl` | `HuntModifierTableRow` |
| `game_activities.jsonl` | `GameActivityTableData` |
| `activity_unlock_criteria.jsonl` | `ProgressionUnlockCriteria` |
| `schedule_data.jsonl` | `ScheduleData` |

Shared shapes: **handle** `{"table":s (table name or empty),"rowName":s}`; **tags** [s], the container's authored tag
names only (not its parent tags); **tag query** `{"tokenStreamVersion":n,"tagDictionary":[s],"tokenStream":[n] (bytes
as 0-255),"userDescription":s,"autoDescription":s}`; **soft path** (4.5, raw form, `None` emptied); **behemoth**
`{"behemothName":s,"behemothAssetPath":soft path,"behemothAssetResolvedName":s (the loaded object's name, or empty
when not loaded),"powerOverride":n,"weighting":f}`; **map** `{"mapName":s,"mapAssetName":s,"mapAssetPath":soft
path,"mapAssetResolvedName":s,"biome":n,"weighting":f}`; **matchmaker list** `{"matchmakerTable":s,"queries":[tag
query]}`. Every row starts `sourceTable`: s, `rowName`: s. (Number vs. float follows the SDK member type;
**(ambiguity)** as 6.7.)

- player hunt: `playerHuntId`: s, `region`: handle, `matchmakingType`, `matchmakingGameType`: n, `huntTags`: tags,
  `matchmakerHuntIDs`: [handle], `matchmakerHuntsByTag`: [matchmaker list], `escalationModeSpecification`: handle,
  `escalationPatrolInitialChallengeLevel`, `escalationPatrolMaxChallengeLevel`, `recommendedEscalationLevel`: n,
  `hasPortals`, `hasGlitterEvent`, `hasPhaelanxEvent`: b, `huntSuccessReward`, `firstCompletionSuccessReward`,
  `targetedHuntSuccessReward`, `huntFailureReward`, `targetedHuntFailureReward`: s, `minWeaponSkillLevel`,
  `recommendedWeaponSkillLevel` (the SDK member is spelt `RecomendedWeaponSkillLevel`): n, `visibleWhileLocked`: b.
- matchmaker hunt: `region`: handle, `huntTags`, `huntThreatLevel`, `gameModeOverride`: s, `dangerPerSecOverride`,
  `specificBehemoth`: behemoth, `additionalSpecificBehemoths`: [behemoth], `mapMetaData`: handle, `mapList`: [map],
  `specificModifier`: s (object name or empty), `modifiers`: [s], `gatherableTier`: n, `maxPlayers`: n,
  `isGeneratedEncounter`: b, `gameModeSpecificData`: handle.
- hunt region: `matchmakingId`, `tokenId`: s, `numTokensRequired`, `numHuntPassBehemothRewards`: n,
  `targetedBehemothList`: s (object name), `behemothPresets`: [soft path].
- escalation spec: `initialChallengeLevel`, `maxChallengeLevel`, `roundStructureCount`: n, `lootTable`: handle,
  `optionalPowerOverride`, `escalationModeTags`: tags, `backupEncountersTable`: s, `completionRewards`: [s],
  `escalationAmountPerScoreValue`: [number].
- hunt modifier: `modifierAssetPath`: soft path, `huntComplexityModifier`, `threatLevel`, `modifierApplicationType`:
  n, `modifierMetaTags`: tags, `behemothElementCondition`: n.
- game activity: `enabled`: b, `playerHunt`: s, `matchmakerHunts`: [s], `unlockRequirement`: n, `unlockCriteria`:
  [handle].
- unlock criteria: `featureFlagsOperation`: n, `featureFlags`: [soft path], `itemIdsOperation`: n, `itemIds`: [s],
  `progressionTrackRanksOperation`: n, `progressionTrackRanks`: [{`track`: s, `rank`: n}],
  `questRequirementsOperation`, `questRequirementsCondition`: n, `questRequirements`: [{`questName`: s,
  `statusRequired`: n}], `playerJourneyNodesOperation`: n, `playerJourneyNodes`: [handle].
- schedule: `startTimeTicks`, `endTimeTicks`: n (the 64-bit tick value of each date), `isRepeatable`, `eachDay`,
  `eachWeek`, `shouldQueue`: b, `scheduledItems`: [{`id`: s, `maxCompletionPerInterval`: n}], `itemsLeftOut`: [n].

SDK members where the JSON name does not give them:

- player hunt: `matchmakerHuntsByTag` is `MatchmakerHuntsByTag` (element `FMatchmakerHuntList`); a list's `queries`
  is written from its `HuntTags`. `visibleWhileLocked` is `FPlayerHuntTableData.IsVisibleWhileLocked`.
- escalation spec: `roundStructureCount` is the length of `FEscalationModeSpecification.RoundStructures`.
- unlock criteria: a quest requirement's `statusRequired` is `QuestRequirements[].QuestStatusRequired`.
- schedule: `startTimeTicks`, `endTimeTicks` are the int64 at the start of `FScheduleData.StartTime`, `.EndTime` (4.1);
  `isRepeatable` is `FScheduleData.IsRepeatable`; `eachDay`, `eachWeek` are `FScheduleData.RepeatableDetails.EachDay`
  and `.RepeatableDetails.EachWeek` (a nested struct); `shouldQueue` is `FScheduleData.bShouldQueueTheSheduleItems`
  (the game's spelling); a scheduled item's `id` is `ScheduledItems[].ID`; `itemsLeftOut` is
  `FScheduleData.ItemsToBeLeftOutOfTheRotation`.

`map_metadata_table` is not exported (a Blueprint struct with no layout in the SDK).

**`hunt_export_manifest.json`**, always written after the eight, pretty JSON with two-space indent, CRLF:

```text
{
  "gameVersion": "1.14.7",
  "changelist": 647472,
  "format": "jsonl",
  "playerHuntRows": <n>,
  "matchmakerHuntRows": <n>,
  "huntRegionRows": <n>,
  "escalationModeSpecRows": <n>,
  "huntModifierRows": <n>,
  "gameActivityRows": <n>,
  "unlockCriteriaRows": <n>,
  "scheduleRows": <n>,
  "notes": "<the fixed note below>"
}
```

Each count is the export's return (0 when not found, -1 when its file could not be opened). The note, exactly:
`Read-only live table export. Tag-routed query dictionaries and raw token streams are preserved. FText display fields
are deliberately omitted (FText is a shared-ref to FTextData, not an FString; reading it faults). map_metadata_table is
omitted: Blueprint-defined row struct with no verified layout. Do not replace backend tables until importer validation
is complete.` The hunt total for the run manifest: -1 when either of the first two is negative, else the sum with
negatives of the other six counted as 0. Status prefix `[Hunts] `; the first line reads `starting read-only 1.12 hunt
export` (sic).

### 6.11 `drop_tables_1_14_7.jsonl` (`EXPORT_DROP_TABLES`)

By row struct `PlayFabDropTableTableData`, guarded, pattern 6.5 (none: **not opened**): `table`, `rowName`: s,
`ignoreOwnedItems`: b, `items`, `dropTables`: [{`resultItem`: s, `weight`: by type, `amount`: n}]. Its count is not in
the manifest. Status prefix `[DropTables] `.

### 6.12 `currency_names_1_14_7.jsonl` (`EXPORT_CURRENCY_NAMES`)

Calls `GetAllItems` (none or n <= 0: return 0, **not opened**); runs its diagnostic (6.17); opens the file; for each
item in returned order whose tags contain `currency` and whose `ItemId` is non-empty: `itemId`: s, `displayName`: s,
`displayNameSource`: s (resolved as 6.8, string tables always on), `hadLocKey`: b (the locator was found),
`locNamespace`, `locKey`: s (empty when not found), `icon`: s, `tags`: [s]. **Icon**: in raw `customData`, the text
after `\"Icon\":\"` (escaped quotes, no space) up to the next `\"`, or empty. Not in the manifest.

### 6.13 Store runtime (`EXPORT_STORE_RUNTIME`): three files

All three are opened (truncated) first; if any cannot be: status, return -1. Then one pass over `GObjects`; an object
that is a `StoreViewModel` writes to the first file and one that is a `StoreItemViewModel` to the second (each line
serialized under the guard; a fault is counted as skipped). Default objects are included and flagged.

`store_view_models_1_14_7.jsonl`: `objectName`, `objectClass`: s, `defaultObject`: b, `tagFilter`: s (4.5),
`storeItemsTable`, `currencyDataTable`: s (table names), `categories`: [{`name`: s (`FText`), `breadcrumbCategory`: s
(the tag's name), `requiresEntitlement`: b, `entitlementId`, `skuId`: s, `subcategories`: [{`name`: s (`FText`),
`breadcrumbCategory`: s, `requiresEntitlement`: b, `entitlementId`, `skuId`: s, `subTagIds`: [s],
`hidePurchasedItems`, `shouldHideTab`, `hideTab`: b}]}].

`store_item_view_models_1_14_7.jsonl`: `objectName`, `objectClass`, `defaultObject`, `displayName`,
`displayDescription`, `displayItemType`: s (`FText`), `itemPriceNumber`, `itemCurrentPriceNumber`: n,
`storeCurrencyEnum`: n, `currencyCode`: s, `displayPriority`: n, `purchased`, `canPurchaseAgain`, `expired`,
`prestigeItemElite`: b, `rewardRarityLevel`: n, `catalogItemId`: s, `offer`: {`skuId`, `displayName`,
`displayDescription`: s, `displayPriority`: n, `catalogId`, `imageUrl`: s, `platPrice`, `regularPlatPrice`,
`dustPrice`, `maxAllowed`, `remaining`: n, `tags`: [s], `grantedCatalogIds`: [{`itemId`: s, `amount`: n}],
`grantedEntitlements`, `grantedProgressInTracks`: [s], `grantedProgress`: {`trackId`: s, `amount`: n, `ranks`: n},
`grantedLoadoutSlots`: n, `payWithAllCurrencies`: b, `currencyCode`: s, `currencyEnum`: n, `inUserCollection`: b,
`platformOfferId`, `timeAvailabilityReason`: s, `missingEntitlementNames`, `previewItemIds`: [s]} (4.5).

`store_item_images_1_14_7.jsonl`: after the pass, every table with row struct `StoreItemTable` and a row count in
`[0, 200000]` (**inclusive** here), rows sorted, guarded: `table`, `rowName`, `featureImage`, `standardImage`: s, each
image `FStoreItemTable.FeatureImage.ObjectID.AssetPathName` (and `StandardImage`) in **short form**, an unset image
written `None`. Each image's `AssetPathName` is at the soft pointer member's offset plus the fixed `+0x10` of 4.5, as
for hunts. `TPersistentObjectPtr.ObjectID` is never taken from the SDK: Dumper-7 declares that template by hand
with `ObjectID` at `+0x0C` and size 0, but the compiled layout aligns it to 8, so it sits at `+0x10`. Reading at
`+0x0C` takes the padding as the name index and writes every set image as `None_<n>` (settled by the maintainer's
diff on the real client, ambiguity 12).

Status `DONE storeViewModels=<n> itemViewModels=<n> imageRows=<n> skipped=<n>`; the run count is the sum of the three.
SDK members where the JSON name does not give them:

- view model: `categories` is `UStoreViewModel.StoreCategoryViewModels` (element `FStoreCategoryViewModels`); a
  category's `name` is its `CategoryName` (an `FText`) and its `subcategories` its `SubCategoryViewModels` (element
  `FStoreSubCategoryViewModels`); a subcategory's `name` is its `SubCategoryName` (an `FText`). `requiresEntitlement`
  is `RequiresEntitlement` on both, `hidePurchasedItems` is `HidePurchasedItems`; the other leaves are spelt
  `BreadcrumbCategory`, `EntitlementId`, `SkuId`, `SubTagIds`, `bShouldHideTab`, `bHideTab`.
- item view model: `storeCurrencyEnum` is `UStoreItemViewModel.StoreCurrency`; `purchased`, `expired`,
  `prestigeItemElite` are `bIsPurchased`, `bIsExpired`, `bIsPrestigeItemElite`; `catalogItemId` is
  `UStoreItemViewModel.StoreItemCatalogData.ItemId`; `offer` is the protected member `StoreItem`, at `+0x650` (4.5).
- offer: `imageUrl` is `FOnlineStorePhoenixOffer.ImageURL`, `currencyEnum` is `.Currency`, `inUserCollection` is
  `.bIsInUserCollection`.

A second entry point that waits for a `season_store` view model exists in the old code but nothing calls it; not
carried.

### 6.14 `mastery_1_14_7\mastery_objectives.json` (`EXPORT_MASTERY`)

The only pretty file, written **binary (LF)** and in one write. Rows come from every table with row struct
`MasteryCardTableData`, then `AchievementTableData` (in `GObjects` order; row count outside `[0, 200000)` skips the
table); within each struct a row name already seen (empty names skipped) is dropped with a status line; each list is
then sorted by row name. Both empty: status `neither table is loaded; nothing written`, return 0, **not opened**.

**Objective** (from a tracked objective): skipped when `UniqueOnlineObjectiveId` is empty. Written
`{"id":s,"target":n}` with, in this order and only when present: `"type":s` (the objective data class's name with a
leading `ObjectiveData_` removed, only when non-empty), `"compoundId":s` (cards only; when non-empty and not `None`),
`"rank":n` and then `"progressTrack":s` (when `ProgressRank > 0`; the track only when non-empty and not `None`).
Compact, no spaces.

The tracked objectives are `FMasteryCardTableData.Objectives` (element `FMasteryObjectiveData`) and
`FAchievementTableData.TrackedObjectives` (element `FTrackedObjectiveData`). In both, `UniqueOnlineObjectiveId` is
on the element and the rest is in its `FGameplayObjective` member `Objective`: `target` is `Objective.Amount`, the
objective data class `Objective.ObjectiveDataClass`, the rank `Objective.ProgressRank` and the track
`Objective.ProgressTrack`. A card objective's `compoundId` is its `CompoundObjectiveId`. A card's
`baseXpObjectiveId` and `prestigeXpObjectiveId` are `FMasteryCardTableData.BaseXPBarOnlineObjectiveId` and
`.PrestigeXPBarOnlineObjectiveId`.

**Card** (skipped with a status line when its objectives count is outside `[0, 2000]` or reward tracks outside
`[0, 64]`): `track` is the first reward track row name that is non-empty, not `None` and not
`MasteryTrack_PlayerLevel`, else `null`. `type` is `"Weapon"` or `"Behemoth"` by the card-type enum, else `null`.
`gameObjectId` only when non-empty; `baseXpObjectiveId`, `prestigeXpObjectiveId` only when non-empty and not `None`
(both also join the id set). Achievement rows skip on objective count outside `[0, 2000]`.

**Layout**, exactly (LF, the indents are spaces):

```text
{
  "schema": 1,
  "source": "Dauntless 1.14.7 CL 647472: mastery_card_table, achievements_table (CatalogExporter live export of <T>), extracted <YYYY-MM-DD>",
  "objectiveIds": [
    "<id>",
    "<id>"
  ],
  "cards": [
    {
      "cardId": "<row>", "track": "<track>"|null,
      "objectives": [
        <objective>,
        <objective>
      ],
      <extras>
    },
    {
      ...
    }
  ],
  "achievements": [
    {
      "id": "<row>",
      "objectives": [
        <objective>
      ]
    }
  ]
}
```

- `<T>`: the names of the tables read, joined by `, `, cards' first. The date is UTC.
- `objectiveIds`: every id written (objectives of both kinds, plus the two XP-bar ids), unique, ascending byte order.
- An empty list is `[]` on the same line: `"objectiveIds": [],`, `"objectives": [],` (cards) or `"objectives": []`
  (achievements), `"cards": [],`, `"achievements": []`.
- `<extras>`: `"type": ...` then the optional ones, joined by `, ` on one line indented six spaces.
- Cards are separated by `,` + newline; the closing `}` of each card is indented four spaces.

Status prefix `[Mastery] `.

### 6.15 `export_manifest_1_14_7.json` (every run that got past the wait)

Pretty, two-space indent, CRLF, always written last:

```text
{
  "gameVersion": "1.14.7",
  "changelist": 647472,
  "engineVersion": "4.26.2",
  "exportedAt": "<YYYY-MM-DDTHH:MM:SSZ>",
  "flags": {
    "EXPORT_CATALOG": <b>,
    "EXPORT_PROGRESSION": <b>,
    "EXPORT_COMBAT": <b>,
    "EXPORT_SKINS": <b>,
    "EXPORT_SKINS_RESOLVE_STRINGTABLE": <b>,
    "EXPORT_SLAYERS_PATH": <b>,
    "EXPORT_WEAPON_SLOTS": <b>,
    "EXPORT_HUNTS": <b>,
    "EXPORT_TABLE_INVENTORY": <b>,
    "EXPORT_STORE_RUNTIME": <b>,
    "EXPORT_MASTERY": <b>
  },
  "results": {
    "catalogGeneralItems": <n>,
    "catalogEquipmentItems": <n>,
    "liveWeaponSlotItems": <n>,
    "progressionRows": <n>,
    "combatRows": <n>,
    "skinsRows": <n>,
    "slayersPathNodes": <n>,
    "huntRows": <n>,
    "tableInventoryRows": <n>,
    "storeRuntimeRows": <n>,
    "masteryRows": <n>
  }
}
```

`exportedAt` is UTC, zero-padded. `EXPORT_CELLS`, `EXPORT_DROP_TABLES` and `EXPORT_CURRENCY_NAMES` are not listed.
Each result is the export's return (0 when off; -1 is possible). A file that cannot be opened is skipped silently.

### 6.16 Table inventory (`EXPORT_TABLE_INVENTORY`): two files

Every data table (guarded name, row count; row count outside `[0, 200000]` skips with a status line; the row struct's
name, empty when null), sorted by name (stable order for equal names is not guaranteed **(ambiguity)**).
`table_inventory_1_14_7.jsonl`: `name`: s, `rowStruct`: s, `rowCount`: n, `objectIndex`: n (the table's `GObjects`
index; so this file differs between game runs). `table_inventory_1_14_7.txt`: the line
`# Loaded UDataTables in the 1.12 client (read-only census)`, then `# name | rowStruct | rowCount`, then one line per
table `<name> | <rowStruct or <null>> | <rowCount>`, unescaped. Status `DONE tables=<n> skipped=<n>`.

### 6.17 `catalog_export_status.txt`

Appended (never truncated) for every status line, each also sent to the debugger output prefixed with the export's tag
(`[CatalogExporter] `, `[ProgressionExporter] `...). Lines in the file carry a prefix by exporter: none (dllmain,
skins, currency), `[Progression] `, `[Combat] `, `[SlayersPath] `, `[Hunts] `, `[TableInventory] `, `[DropTables] `,
`[StoreRuntime] `, `[Mastery] `. The skins and currency lines go through a wide stream that narrows each character
**(ambiguity: non-ASCII in those lines)**. The full text of every line is in the sections above where it matters for a
host; the skipped-flag lines are `<FLAG>=0, skipping <what>` with `<what>`: `catalog export`, `live weapon-slot export`,
`live cell-slot export`, `progression export`, `combat export`, `weapon skins export`, `Slayer's Path node export`,
`live hunt table export`, `drop-table export`, `currency name resolution`, `store runtime capture`,
`mastery/achievement objective export`, `DataTable census`. The string-table warnings and diagnostics
(`=== STRINGTABLE DIAGNOSTIC START ===`, the `[ST-obj]`, `[ST-reg]`, `[ST-keys]` and `[stringtable]` lines) are
research aids: the new one writes equivalent lines but **byte parity of the status file is not required** (call): it is
a log, appended across runs, and nothing reads it but a person.

## 7. What reads the output

| File | Reader (MIT unless noted) | Fields it reads |
| --- | --- | --- |
| `catalog_1_14_7.jsonl` | `rustic-isles-gamedata` generators (reward cache, platinum store, storage policy; spec-rustic-isles-gamedata §2.2) | `itemId`, `displayName`, `description`, `tags`, `isStackable`, `maxQuantity`, `customData` (its `ItemData`'s `RarityLevel`, `Icon`, `LargeIcon`, `CustomIcon`) |
| `slayers_path_1_14_7/player_journey_nodes.jsonl` | gamedata (Slayer's Path and its definitions) | `nodeId`, `autoUnlockIfParentUnlocked`, `childNodes`, `currencyCosts`, `rewards[].itemId`; `None` means empty |
| `store_item_images_1_14_7.jsonl` | gamedata (platinum store) | `rowName`, `standardImage`, `featureImage` (only "present or `None`") |
| `hunts_1_14_7/*` | `rustic-isles-director` `hunt_tables` (spec-rustic-isles-director §12) | the player and matchmaker hunt fields; the manifest's `gameVersion` |
| `progression_1_14_7/*.jsonl` | `dainty-hostkit` `progression` (four track files and the hunt-pass file) | `rowName`, `base.progressionTrack`, `base.requirements`, `prestigeTrack`, rewards |
| `mastery_1_14_7/mastery_objectives.json` | copied by hand to the backend's `game-data/` (the backend's mastery reader) | the whole document |
| everything else | nobody: research exports | |

Every reader drops a trailing `\r`, so CRLF is safe. The Host page's flags (2.2) turn on exactly the five services'
exports plus progression, not mastery.

## 8. `version_data.json`: what is exported and what is authored

**Finding: no field of `restricted/modules/rustic-isles/version-data/1.14.7/version_data.json` is produced from the
exporter's output today.** The file is generated by restricted's `rustic-isles-mit-parity` test
(`tests/common/version_data.rs`, RR_UPDATE_GENERATED) from Rust constants that were ported from Paradox's TypeScript
(`controllers/character.ts`, `controllers/starterManifest.ts`, `controllers/wallet.ts`,
`scripts/generate_reward_cache_store.ts`, isles-db's migration transforms) and from `src_tokens.txt` (commit
`8ba69323`). The exporter's output is consumed by the generators *with* parts of it (the reward cache plan, the token
seeds), not to make it. Item 9's classes are those of spec-rustic-isles-1-14-7 §4.1: (a) wire fact, (b) game data,
(c) authored.

| Field | Contents | Exported from the game? | Evidence | Fact or design? (the maintainer decides) |
| --- | --- | --- | --- | --- |
| `format`, `gameVersion` | `1`, `1.14.7` | no | the document's own header | Dainty Lass's, trivially MIT |
| `characterData.fill` | nine `[key, default]` pairs: `RecentPlayers`, `AppearanceData`, `PlayerAccountProgressStep` (`New`), `CharacterFlagData`, `PlayerDataRepair`, four `SERIE_*` quest series | no | `character.ts` fills each with `??=` when missing; the values are empty shapes (`{"Flags":[]}`, `{"Data":[]}`, `{"ID":"CR20_PJM_Quests"}`) | key names and empty shapes are what the client reads and writes: **facts** (a). *Which* series are pre-filled (CR20 PJM, D24 A/B/D) is upstream's choice; the series ids themselves are game data |
| `characterData.defaultAppearance` | a creator preset: `Feminine`, `Hair12`, `Tan`, two colours written as UE's `(R=0.610496,G=…,A=1.000000)` | no (captured) | the colour text is the engine's own colour format, so it was copied from what the client wrote; `character.ts` hard-codes it | (b) captured output, but the **choice** of this preset is upstream's: **design** of a minor kind. Any valid preset works; the client's creator overwrites it |
| `characterData.forcedCharacterFlags` | 17 `TutorialSlate_*` flags and `CharacterFlag_CityGatherableTutorialShown`, each `True` | no | client flag names; forcing them on (suppressing tutorials) is a server policy in `character.ts` | names are **facts** (a); forcing them is **design** |
| `characterData.forcedLoginFlag` | `WatchedNewRamsgateCinematic` = `true` | no | `character.ts` comments record observing the client store it in `LoginFlagData` | name and shape **fact**; when to set it is behaviour (code), not data |
| `characterData.dojoSeriesKey`, `dojoSeries`, `dojoNodeId` | `SERIE_dojo`, a blob of quest-node GUIDs with statuses, `Dojo` | no | GUIDs are the game's quest asset ids (b); upstream notes 1.12 consults the blob for the Training Grounds destination | GUIDs **fact/game data**; the statuses (0, one 1) are a chosen initial state: arguably **fact** if they are the game's fresh state, unverified |
| `characterData.triggerSeriesKey`, `triggerNode`, `triggerStatus` | `SERIE_cr19_series_1_ftue`, `929A333B40E413C41E47B0A425EC3349`, `3` | no | `character.ts`: when the client reports that FTUE node at status 3, the server adds the Dojo series. Observed client behaviour | **fact** (a game GUID and the status the client reports) |
| `starterKit.instanced` | 23 catalog ids: seven `WP_*_BEGINNER`, eight default parts, `AR_UNEQUIPPED_HELM`, `AR_BEGINNER_*`, `LT_BASIC`, `FL_HEALING_DEFAULT`, `GD_FRAME_STARTER_BASE`, `BN_BEGINNER_00` | no; existence checkable against `catalog_1_14_7.jsonl` | the game names them beginner/default/starter; the list was a server grant (PlayFab), not client data | the ids are **facts**; the *set* given to a new account was Phoenix Labs' server data, reconstructed upstream: likely mirrors the original but unproven. The 2026-11-05 ruling already made the starter manifest's id lists wire facts; the same argument applies |
| `starterKit.defaultWeaponParts` | eight `PART_*_DEFAULT` (and `PART_MS_SPECIAL_PROJECTILE`) | **derivable**: `EXPORT_WEAPON_SLOTS`' `defaultWeaponPartId` per slot | the constant's own comment: "each owned starter weapon must also own the default part its slot data names" | **fact**: the game's slot data names them |
| `starterKit.equipment`, `flask`, `banner`, `glider` | the six slots (weapon `WP_EB_BEGINNER` …), `FL_HEALING_DEFAULT`, `BN_BEGINNER_00`, `GD_FRAME_STARTER_BASE` | no | picks from `instanced` | Sword as the first weapon is a **design** choice (the 1.14.7 FTUE may dictate it: unverified); the rest follow from the beginner items |
| `starterKit.instanceData` | the starter sword's item JSON: `SheenType` 73, dye fields `None`, the equipped default part | no | an item-instance blob in the client's shape | shape **fact**; `SheenType: 73` is unexplained (captured or chosen?) **(ambiguity)** |
| `starterKit.bannerCustomization` | `BNC_*_BEGINNER_00` mesh, fabric, sigil, animation; dyes `DYE_GREEN04_DURABLE`, `DYE_BROWN07_DURABLE` ×2; sheen 0 | no | the BNC ids are the game's beginner set; the two dye colours are a pick | ids **fact**; the colours **design** (minor) |
| `starterKit.introEmote`, `flare` | `EM_INTRO_BEGINNER_01`, `QI_BASIC_FLARE_DURABLE` | no | beginner/basic items | **fact**-like (the game's basic items), chosen upstream |
| `starterKit.defaultQuickChats` | nine `qc_*` ids | no | the chat wheel's nine slots | ids **fact**; their order on the wheel likely the game's default, unverified |
| `starterKit.wallet` | `CURRENCY_NOTES` 5000, five `CURRENCY_PJM_*` 50 | no | `wallet.ts` constants | **design**: amounts are upstream's numbers (the original grant was server data) |
| `legacyStarter` | `DYE_DEFAULT`, `BN_SIGIL_BEGINNER_00` | no | isles-db's 1.4.4 migration: what a migrated banner without a dye or sigil gets | **design** (a migration default). Note it names `BN_SIGIL_…` while the starter banner uses `BNC_SIGIL_…`; whether `BN_SIGIL_BEGINNER_00` exists in the 1.14.7 catalog is unchecked **(ambiguity)** |
| `rewardCachePlan` | 178 selections (catalog id, tab, price, featured, rarity-3 allowance, name/description overrides), armour and banner family orders, expected counts per tab (48/34/23/55/10/8) and 7 featured | no; the ids are checked against the catalog, the tags came from an `EXPORT_STORE_RUNTIME` capture | `generate_reward_cache_store.ts`'s header: "The original Phoenix Labs rotation and prices were remote service data and are not claimed here"; prices from a fixed tier set {700, 800, 1000, 2000, 2500, 3000}; comments such as "Four marquee offers also populate the FEATURED tab" | **design**, clearly: a reconstructed rotation with invented prices. Only the tab tags (`tab_season_armour` …) are facts, and those are already code in gamedata |
| `storageTokenSeeds` | 2,946 tokens | no | mechanically extracted from Paradox's `ParadoxBackend/src` by regex (`generate_inventory_storage_policy.ts`; tokens such as `A-F`, `A-HJ-KM-NP-Z2-9` are regex fragments). Its effective part is the catalog ids the code names | neither: **a derivative of AGPL source**. Its effective content (ids the server needs storage rules for) is recomputable from the MIT crates' own ids (spec-rustic-isles-1-14-7 §15 item 6) |

**Summary for item 9**: the exporter rewrite does not by itself make any of these generatable. Facts that could be
generated on the host: `starterKit.defaultWeaponParts` (from weapon-slot data) and the existence of every id (from the
catalog). Clearly design: `rewardCachePlan`, `starterKit.wallet`, `legacyStarter`, the forced-flag policy, the default
appearance choice. Clearly derivative: `storageTokenSeeds`. The rest are wire facts or game GUIDs observed from the
client, which the 2026-11-05 ruling's reasoning (identifier lists are facts) would cover.

## 9. Rust or C++

**Recommendation (call): Rust, as an MIT `cdylib` in its own crate, loaded the same way (`CatalogExporter.dll`,
through the runtime loader).** It can be Rust; nothing in what it does needs C++:

| C++ feature used today | Rust replacement |
| --- | --- |
| The SDK's headers for every layout and the four SDK `.cpp` files | **Build-time extraction from the host's own SDK**: a `build.rs` reads the Dumper-7 dump (the host imports it already) for the few dozen members, enum values and engine offsets this page names, and fails the build naming any it cannot find. Dumper-7 writes each member's offset and size beside it, and its `Basic.hpp` holds the engine offsets (`GObjects`, the name pool or `AppendString`, the `ProcessEvent` index). Nothing from the SDK is committed (AGENTS.md) |
| `__try/__except` around reads | A guarded-read primitive: copy memory with `ReadProcessMemory` on the own process (fails instead of faulting) or check pages with `VirtualQuery` first; every read in section 4 goes through it |
| Calling reflected functions (`ProcessEvent`) | A function pointer taken from the object's vtable at the SDK's index, called with the parameter block laid out from the SDK's function layout. Four functions (4.2) for the services' exports; the string-table ones only for research modes |
| Dumper-7's name/string helpers | Re-implemented from UE 4.26's public behaviour: name pool decoding (or a call to `AppendString`, as the SDK does **(ambiguity: which one the host's SDK uses)**), the short/raw forms of 4.3, UTF-16 to text |
| `std::sort`, `std::to_string(float)`, text-mode streams | byte-order sort of `String`s; a `%f` formatter (six decimals, correctly rounded from the exact binary value as MSVC's CRT has done since
VS 2019; the float is widened to double first); `\n` written as `\r\n` |

What Rust buys: the whole output layer (JSON writer, `%f`, escaping, ordering, the flags parser, the mastery layout) is
plain code that the root's CI tests on every OS against golden bytes, and the memory layer can be tested against a
synthetic memory image (fake `GObjects`, names, tables). C++ would need MSVC and the SDK for every test.

What it costs, and these are **seams for the integration pass, not this row**:

1. **`unsafe_code` is denied workspace-wide.** An in-process memory reader needs `unsafe` (raw pointers, the
   function-pointer call, `DllMain`). Proposed: the crate opts out with a crate-level allow and keeps all `unsafe` in
   one module (`mem`), reviewed like a trust boundary (its `docs/security/` page and a fuzz target over the parsers).
2. **AGENTS.md says there is no native code outside `restricted/`.** A Windows-only `cdylib` at the root changes that
   sentence; the crate builds as an empty library on other targets so `cargo test --workspace` still passes there.
3. **The SDK at build time**: the crate's build needs the host's dump path (the same place the runtime build uses,
   `restricted/modules/rustic-isles/native/runtime/SDK` today; an MIT crate cannot point into `restricted/`, so the
   path comes from a build variable, proposed `DAINTY_SDK_DIR`, which the Host page's build step sets). Without it the
   crate builds the testable layers only.
4. Proposed name `rustic-isles-exporter` (`crates/rustic-isles-exporter`), output `CatalogExporter.dll` so the launcher
   and the host kit need no change (`EXPORTER_DLL`, `exporter_dll`; the host kit's path would move from
   `native/catalog-exporter/x64/Release` to the cargo target folder, a hostkit and launcher-kit change for their rows).

C++ at the root would avoid item 1 but keep MSVC and the SDK in every build and test, and it still breaks item 2.
**Out of process** (a Rust exe reading the game with `ReadProcessMemory`) was considered and rejected for now: it
cannot call `GetAllItems`, and it would need the catalog map read directly (4.2's call), which only the diff can
approve.

**Scope (call)**: phase 1 carries the exports a reader uses (catalog, equipment, Slayer's Path, hunts, store runtime,
progression, mastery), the manifest and the status file. The research exports (weapon slots, cells, combat, skins,
drop tables, currency names, table inventory) are specified here so nothing is lost; they are phase 2 or dropped, the
maintainer's choice (section 12). Until they exist, a flags file that turns one on must say so: the new one writes the
status line `<FLAG>=1 is not supported by this build, skipping` for it and leaves its files alone.

## 10. The maintainer's hands-on check: old and new on the real 1.14.7 client

This needs the game, so it is not an agent's job. Goal: the new DLL writes the same bytes as the old for every file a
reader uses, on the same client and SDK. Game state differs between runs (object order, what is loaded), so first
measure how much the **old** one differs from itself (the noise floor), then compare new against old.

Prerequisites: a Windows PC with the 1.14.7 client, the launcher's Host page working, the SDK imported, a local server
running and a character that reaches Ramsgate.

**1. Build the old DLL with its own export root** (so the launcher's `Win64` flags are bypassed and the runs don't
mix). In `restricted\modules\rustic-isles\native\catalog-exporter`:

```powershell
New-Item -ItemType Directory -Force C:\dl-diff\runs | Out-Null
Set-Content -Encoding ascii ExportPaths.local.h '#define RUSTIC_ISLES_EXPORT_ROOT L"C:\\dl-diff\\old"'
.\_build.bat
Copy-Item x64\Release\CatalogExporter.dll C:\dl-diff\CatalogExporter-old.dll
```

**2. Build the new DLL with its root** (the crate and variable names are this page's proposals; adjust to what lands):

```powershell
$env:RUSTIC_ISLES_EXPORT_ROOT = 'C:\dl-diff\new'
$env:DAINTY_SDK_DIR = (Resolve-Path restricted\modules\rustic-isles\native\runtime).Path
cargo build --release -p rustic-isles-exporter --target x86_64-pc-windows-msvc
Copy-Item target\x86_64-pc-windows-msvc\release\rustic_isles_exporter.dll C:\dl-diff\CatalogExporter-new.dll
```

**3. Flags for both roots**, every key on except the two that call string tables (turn those on in a separate pair of
runs if phase 2 carries them):

```powershell
$keys = 'CATALOG','PROGRESSION','COMBAT','SKINS','SLAYERS_PATH','WEAPON_SLOTS','HUNTS','TABLE_INVENTORY',
        'CELLS','DROP_TABLES','STORE_RUNTIME','MASTERY'
$flags = ($keys | ForEach-Object { "EXPORT_$_=1" }) + 'EXPORT_SKINS_RESOLVE_STRINGTABLE=0', 'EXPORT_CURRENCY_NAMES=0'
foreach ($r in 'old', 'new') {
  New-Item -ItemType Directory -Force "C:\dl-diff\$r\native\catalog-exporter" | Out-Null
  Set-Content -Encoding ascii "C:\dl-diff\$r\native\catalog-exporter\export_flags.txt" $flags
}
```

**4. One run** (`<which>` is `old` or `new`, `<label>` names the run): clear the output, put that DLL where the Host
page takes it, run the export, open the screens, quit.

```powershell
$which = 'old'; $label = 'old-1'
$dll = 'restricted\modules\rustic-isles\native\catalog-exporter\x64\Release\CatalogExporter.dll'
Remove-Item -Recurse -Force "C:\dl-diff\$which\Items_Analysis" -ErrorAction SilentlyContinue
New-Item -ItemType Directory "C:\dl-diff\$which\Items_Analysis" | Out-Null
Copy-Item "C:\dl-diff\CatalogExporter-$which.dll" $dll -Force
```

Then on the Host page press **Run export** (the export session; run the commands above from the workspace root the
Host page uses). Play the same way in every run: log in, reach Ramsgate, open the Slayer's Path, the hunt map,
Journal → Challenges → Reward Cache and the Mastery board. The system beep marks the end of the export; the new DLL
then closes the game itself (3, step 7), the old one does not. Because of
3.1's timing the table exports may find nothing; that is the same for old and new, so the comparison still holds, but
note each run's counts in `export_manifest_1_14_7.json` **(ambiguity: whether any run loads those tables in time)**.
Quit the game, then keep the output:

```powershell
Copy-Item -Recurse "C:\dl-diff\$which\Items_Analysis" "C:\dl-diff\runs\$label"
```

Do runs `old-1`, `old-2` (the noise floor) and `new-1`, the same way and in the same game state.

**5. Compare.** Byte-exact for every file, then line-sorted for the files whose line order follows `GObjects`
(equipment, the live and view-model files, the census), with the known per-run fields masked:

```powershell
function Compare-Run($a, $b) {
  $mask = '"exportedAt": "[^"]*"', '"objectIndex":\d+', 'extracted \d{4}-\d{2}-\d{2}'
  $files = Get-ChildItem -Recurse -File "C:\dl-diff\runs\$a" | Where-Object Name -ne 'catalog_export_status.txt'
  $files | ForEach-Object {
    $rel = $_.FullName.Substring("C:\dl-diff\runs\$a".Length + 1)
    $other = Join-Path "C:\dl-diff\runs\$b" $rel
    if (-not (Test-Path $other)) { "MISSING in $b  $rel"; return }
    if ((Get-FileHash $_.FullName).Hash -eq (Get-FileHash $other).Hash) { "same          $rel"; return }
    $x = [IO.File]::ReadAllText($_.FullName); $y = [IO.File]::ReadAllText($other)
    foreach ($m in $mask) { $x = $x -replace $m, '<masked>'; $y = $y -replace $m, '<masked>' }
    if ($x -ceq $y) { "same (masked) $rel"; return }
    $sx = ($x -split "`r?`n" | Sort-Object -CaseSensitive) -join "`n"
    $sy = ($y -split "`r?`n" | Sort-Object -CaseSensitive) -join "`n"
    if ($sx -ceq $sy) { "same (sorted) $rel" } else { "DIFFERENT     $rel" }
  }
}
Compare-Run old-1 old-2
Compare-Run old-1 new-1
Compare-Run new-1 old-1
```

The third call catches files only the new one wrote. **Pass**: every file that is `same` (any form) in
`old-1`/`old-2` is the same form or better in `old-1`/`new-1`. `same` and `same (masked)` compare line endings too;
`same (sorted)` does not, so for those check the first bytes of both (CRLF is `0D 0A`):

```powershell
Format-Hex "C:\dl-diff\runs\new-1\equipment_1_14_7.jsonl" | Select-Object -First 4
```

For any `DIFFERENT`, find the first differing lines:

```powershell
$p = 'catalog_1_14_7.jsonl'
$old = Get-Content "C:\dl-diff\runs\old-1\$p"; $new = Get-Content "C:\dl-diff\runs\new-1\$p"
Compare-Object $old $new | Select-Object -First 5
```

**6. Afterwards**: put the old DLL back at the Host page's path (or rebuild it without `ExportPaths.local.h`), delete
`ExportPaths.local.h`, and keep `C:\dl-diff` private: it is game data and never goes into the repository or a report.
Record in the report only file names, verdicts and row counts.

## 11. What the implementer must not do

- Open `restricted/`, Paradox's repository or the exporter's history. This page and the host's own SDK dump (at build
  time, never committed, never pasted into the crate) are the inputs.
- Commit any SDK file, any offset table generated from it, or any exported file, even as a test fixture: tests use
  made-up ids and a synthetic memory image.
- Write to game memory, install a hook, or call any reflected function not in 4.2.
- Carry a DR name: none appears here. Carry the Paradox file names as they are (`CatalogExporter.dll`,
  `Items_Analysis`, the `RUSTIC_ISLES_` root variable): they are the launcher's and the generators' contract.

## 12. Calls made, ambiguities and open questions

**Calls this page made** (each the maintainer may overturn):

1. Rust `cdylib`, in process, same DLL name (9). Seams 9.1-9.4 go to the integration pass.
2. Phase 1 scope: the read exports, manifest and status; research exports phase 2 or dropped (9).
3. Status-file byte parity not required (6.17).
4. Every read guarded; the new one may survive where the old crashed (4.4).
5. Keep calling `GetAllItems` until a diff approves the direct map read (4.2).
6. The export root becomes a build-time environment variable of the same name (3.3).
7. Phase-1 refusal line for unsupported flags (9).

**Ambiguities only a run or the SDK settles**:

1. Name number suffix and UTF-16 conversion in the SDK's helpers (4.3); whether names decode through the pool or a
   call to `AppendString` (9).
2. The 1.12-era store offsets `+0x170`, `+0x650` on 1.14.7 (4.5), and where a store image's `ObjectID` sits in
   the soft pointer (6.13).
3. Member types (integer or float) of the combat and some hunt fields (6.7, 6.10); `%f` rounding and NaN (5).
4. Duplicate row names (5); equal names in the census sort (6.16).
5. Whether any export session can load the Slayer's Path, hunt, store and mastery tables before the catalog appears
   (3.1); the guide's instructions do not match the code's timing.
6. `SheenType: 73` and `BN_SIGIL_BEGINNER_00` in the version data (8).

**Open questions for the maintainer**:

1. **Timing (3.1)**: keep "export the moment the catalog appears" (parity), or add a wait (for example until the
   requested tables are loaded or a key is pressed)? A wait breaks nothing a reader relies on but changes when files
   appear. Recommended: parity first, then a follow-up row.
2. **Research exports**: carry them (phase 2) or drop them? Nothing reads them; two of them call string-table
   functions off the game thread.
3. **Item 9**, from section 8: rule `rewardCachePlan`, `starterKit.wallet`, `legacyStarter` and the default
   appearance as design (they stay a small AGPL data file); rule the identifier lists and GUIDs facts (MIT, committed
   like the starter manifest); replace `storageTokenSeeds` with an MIT list (spec-rustic-isles-1-14-7 §15 item 6).
4. **The seams** of section 9 (unsafe opt-out, native code at the root, the SDK build variable, the host kit path).
5. `ExportFlags.md` says the census `.jsonl` carries each table's object path and that `export_flags.txt` is tracked;
   the code writes the object index and the file is ignored. This page follows the code.

## 13. Size

| Part | Estimate (Rust lines, tests included) |
| --- | --- |
| Memory layer: guarded reads, `GObjects`, names, containers, tables, soft paths, the `ProcessEvent` call, `DllMain` | 900-1,300 |
| `build.rs`: SDK extraction of the needed members, enums and offsets | 400-600 |
| Output layer: JSON writer, `%f`, escaping, CRLF, flags, paths, manifest, status | 400-500 |
| Phase-1 exporters (catalog, equipment, Slayer's Path, hunts, store runtime, progression, mastery) | 1,300-1,700 |
| Tests (golden bytes, synthetic memory, flags) | 800-1,200 |
| **Phase 1 total** | **about 3,800-5,300** |
| Phase 2 research exporters | +1,000-1,400 |

Against 4,883 lines of C++ (which leans on the SDK's own code for names and calls). Plus the security page and a fuzz
target over the flags parser and the SDK extractor.
