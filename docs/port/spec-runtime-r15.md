<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R15, inventory and economy (RD27, plan item 11)

R15 moves three game-data features from the AGPL `Rustic-Isles.dll` to the MIT `rustic-isles-runtime.dll`
(`spec-runtime-surface.md` sections 4.3, 6 and 8): supplies lose their stack cap (patch 0024), the Middleman's cell
fusion takes 30 minutes (patch 0029), and the arsenal re-resolves loadout items whose inventory arrived late (patches
0027 and 0037). Patch 0026, the client's inventory trace, is logging only and is not ported (RT-3). R15 installs no
hook of its own: it registers handlers on R0's dispatch chains SP-1 (client) and SP-2 (server) and owns the feature
ids `SP-1@R15` and `SP-2@R15`.

An implementer builds from this page, `spec-runtime-r0.md`, `spec-runtime-r1.md` and the surface page alone, and never
reads `restricted/`. What crossed is facts: UE class, struct, field and UFunction names, layout offsets of the 1.14.7
build (`Dauntless-Win64-Shipping.exe`, CL 647472, from the maintainer's Dumper-7 dump, RT-2), RVAs, numbers, exact log
texts, and behaviour. **(call)** marks a decision made here, **(departure)** a deliberate difference from the AGPL
runtime, **(ambiguity)** something the source did not settle; all are collected in section 11. Conventions are the
surface page's.

---

## 1. Scope

| R15 owns | Section | Role | Feature id |
| --- | --- | --- | --- |
| Supplies without a stack cap (0024) | 4 | B | `SP-1@R15` (client), `SP-2@R15` (server) |
| Cell fusion time (0029) | 5 | B | `SP-1@R15`, `SP-2@R15` |
| Loadout slot repair (0027, 0037) | 6 | C | `SP-1@R15` |
| The settings both read, per RT-11 | 3 | B | — |

Not R15: the 0026 inventory trace (not ported, section 6.6); the server's inventory refresh guard UI-12 (R6); the
`ClientRestart` capture R14 does for input activation (R15 keeps its own, 6.2); the loading evaluator LD-1 (R7; R15
only calls the game's original, 6.3).

Deps: R0 and R1 (both on `plan`). R15 can land beside R2 and R17 in any order.

---

## 2. Where the code goes

| Crate | New | Holds |
| --- | --- | --- |
| `rustic-isles-runtime-core` | `src/economy/` (`mod.rs`, `settings.rs`, `catalog.rs`, `fusion.rs`, `loadout.rs`, `text.rs`) | Everything pure, over R0's `Memory` trait: the settings parsers (3), the catalog walk and its uncap decisions (4), the data table walk (5), the loadout walk and the repair bookkeeping (6: the 1 s timer, attempts, the "waiting" once-per-loadout rule), the log texts, the `TMap`/`TArray`/`FString` readers |
| `rustic-isles-runtime-core` | `src/offsets.rs`: a `pub mod r15 { ... }` block | The layout and RVA lines of section 7 |
| `rustic-isles-runtime-core` | `src/registry.rs`: `"R15"` appended to `LANDED` | Nothing else: both feature ids already have their lines (R0 7.5) |
| `rustic-isles-runtime` | `src/economy/` (`mod.rs`, `writes.rs`, `repair.rs`) | `register()`, the handlers, the object-array scans, the game calls (through the FFI file below), the flag and environment reads |
| `rustic-isles-runtime` | `src/ffi/economy.rs` (+ `pub mod economy;` in `ffi/mod.rs`) | The only new unsafe code: a checked 4- and 8-byte write into game memory (7.4), the virtual `ProcessEvent` call (7.3), and the one-argument call of the loading evaluator (6.3) |
| `rustic-isles-runtime` | `src/lib.rs`: `#[cfg(minhook)] mod economy;`; `src/lifecycle.rs`: `crate::economy::register();` before `handlers::freeze()` | One line each. **(call)** This is how a slice registers in Init (R0 8, "slices register in Init"); R2 and R17 add their own line beside it, so the integration pass sees only adjacent one-line additions |

Pure and CI-tested: section 3's parsers, every walk of sections 4 to 6 against a fake `Memory` that lays out the
structures of section 7, the repair's bookkeeping and the log texts. Windows-only: the object-array scans (they run
R0's `full_name` on the live engine), the writes, the `ProcessEvent` and evaluator calls, the handler registration.

---

## 3. Settings (RT-11)

RT-11 makes 0024 and 0029 `settings.toml` settings passed in the environment. The two flag files
`KEEP_SUPPLY_STACK_CAPS.flag` and `KEEP_CELL_FUSION_TIME.flag` are **not read** by the MIT runtime (departure 1).

| Variable | Read with | Accepted | Default (unset) | Other values |
| --- | --- | --- | --- | --- |
| `DAINTY_KEEP_SUPPLY_STACK_CAPS` | R0's `env_value(name, 15)` | `true` (keep the game's caps), `false` (uncap) | `false`: uncap | the default, and a note (below) |
| `DAINTY_CELL_FUSION_MINUTES` | R0's `env_value(name, 15)` | a `strtol` integer (R1 section 2) from 0 to 10 080 (seven days); 0 keeps the game's own times | 30 | the default, and a note |

- Exact, case-sensitive values. Each is read **once**, at the feature's first run (4.1, 5.1), and kept for the process.
- A refused value (set but not accepted, or one `env_value` refuses: too long, a control or non-ASCII character)
  logs once, at that first run: `[SupplyCap] DAINTY_KEEP_SUPPLY_STACK_CAPS='<text>' invalid (default used)` or
  `[CellFusion] DAINTY_CELL_FUSION_MINUTES='<text>' invalid (default used)`, `<text>` being the value when
  `env_value` returned one, else empty. (These are number and switch settings, never secrets.)
- **(departure 2)** The AGPL read an unprefixed `CELL_FUSION_MINUTES` (same range, same default, 0 keeps). Nothing in
  this repository sets it; the server prefix rule of AGENTS.md renames it.

**Where the values come from (call).** A game server gets them from the Director, which hands its whole environment to
every game server it starts (R1 4.4, RT-15). The host kit writes them from `settings.toml` (seam line, section 9). A
**client** is started by the launcher, which knows no host's `settings.toml`: it runs on the defaults (uncap, 30
minutes) unless the player sets the variables. The defaults are the AGPL's, so nothing changes for a player. A host who
changes either value changes what its servers do (the authority: a fusion's end time is built on the server), while the
client's Middleman screen still shows 30 minutes, and a client with caps kept would still drop a stack its server sent
above the cap. Passing the server's values to the client (through the live status the launcher reads) is a separate
row if the maintainer wants it (question Q-1, section 11). A realm's player host is started by the launcher too and
runs on the defaults until that row.

---

## 4. Supplies without a stack cap (patch 0024)

Why: a few tonics (Granny's Heartbreaker, Granny's Antidote and three test tonics) carry `MaxQuantity` 30 in the game's
own catalog, so a server that grants more than 30 sees the client drop the stack from its inventory. Patch 0023
(backend) refills supplies above that.

### 4.1 When it runs

On every `ProcessEvent` dispatch of the role's point (SP-1 on a client, SP-2 on a server) whose function has a name
(matcher `(Prefix, "")`, section 8), as a **before** handler, at most once every **5000 ms** (`GetTickCount64`): the
next due time is set to now + 5000 at the start of a run, before anything else, so a run that finds nothing waits 5 s.
The first dispatch runs it at once. A run that finds another run in progress (an atomic flag; the write and the scans
never nest, but two threads may dispatch) returns at once. **(departure 5)** The AGPL had no such guard.

At the first run: read the setting (3). `true` logs once
`[SupplyCap] DAINTY_KEEP_SUPPLY_STACK_CAPS=true: the game's stack caps are kept` and every later run returns at once.

### 4.2 Finding the catalog

The catalog is the first object in the global object array (R0 6.2), by index from 0, that is **not** a class default
object (object flags bit `0x10`, `RF_ClassDefaultObject`) and **is a** `UArchonCatalog` (7.2). Found, it is kept; at
each later run it is kept only while R0's `is_live_object` holds, else the search runs again. Not found: the run ends
(next try in 5 s).

### 4.3 The walk

`UArchonCatalog::Catalog` is a `TMap<FString, FArchonCatalogItem>` (7.1). For each allocated element, in storage
order:

1. `MaxQuantity` (`i32`) at most 1: skip. (Flares and other one-per-account items, cap 1, are never touched; nor is
   an item already at 0.)
2. Its `Tags` (`TArray<FString>`) holds no tag whose text is exactly `supplies` (case-sensitive, whole string): skip.
3. Otherwise write 0 to `MaxQuantity` (0 = no cap), through the checked write (7.4). For the first **8** such items,
   remember `<item id>(<old MaxQuantity>)`, the item id being the element's key `FString`.

After the walk, when at least one item was written:
`[SupplyCap] removed the stack cap of <n> supply item(s): <list>`, `<list>` the remembered entries joined by `,`
(no space). Because a written item reads 0 afterwards, later runs find nothing and log nothing, until the game
rebuilds its catalog (a new catalog object, or entries reloaded).

A map that cannot be read (its count or element storage unreadable, or more than 1 000 000 elements), an element whose
`Tags` count is negative or above 1 024, or a tag string that cannot be read is skipped without a write (the AGPL would
have faulted; the MIT never reads unchecked memory, R0 5).

---

## 5. Cell fusion time (patch 0029)

Why: each fusion recipe's `TimeRequired` comes from the game's cell exchange data table. The game server builds a
started fusion's end time from it, and the client shows it on the Middleman's screen. R15 sets every recipe to the
configured minutes in memory, on clients and game servers. Fusions started before the change keep the end time they
were given.

### 5.1 When it runs

Same handler as 4.1 (each `ProcessEvent` dispatch with a name, before the chain), a separate timer: the next due time
is set at the start of a run to now + **30 000 ms** if the table was already known when the run began, else now +
**2000 ms**. The first dispatch runs it at once. The same in-progress guard as 4.1 (one guard for both features).

At the first run: read the setting (3). Minutes 0 logs once `[CellFusion] the game's own cell fusion times are kept`
and every later run returns at once. Otherwise the target is `minutes × 600 000 000` ticks (`FTimespan` ticks are
100 ns).

### 5.2 Finding the table

The first object in the object array that is not a class default object, **is a** `UDataTable` (7.2), and whose
`RowStruct` (a `UScriptStruct*`, 7.1) is non-null and has the object name (R0's short name, no package)
`CellExchangeTableData`. Kept and re-validated as in 4.2. (The game instance holds the same table as
`UArchonGameInstance::CellExchangeTable`; the scan finds it without that pointer.)

### 5.3 The walk

`UDataTable::RowMap` is a `TMap<FName, uint8*>` (7.1). For each allocated element with a non-null row pointer: count
it as a row; read the row's `TimeRequired` (`i64` ticks); if it equals the target, go on; else, for the first **8**
changed rows, remember `<row name>(<old ticks / 600 000 000>min)` (integer division; the row name is the element's
`FName` as R0 decodes names, number suffix included), write the target (7.4), and count it as changed.

After the walk, when at least one row changed:
`[CellFusion] set <changed>/<rows> fusion recipe(s) of <table object name> to <minutes> min; before: <list>`, the
list joined by `,`. Later runs change nothing (each row already holds the target) and log nothing.

---

## 6. Loadout slot repair (patches 0027 and 0037)

### 6.1 Why

The game facts the repair rests on:

- A loadout item is a `UArchonLoadoutItemSlot`. Of its item id (`FItemInstanceIdRepl`) only `ItemInstanceIdCrc`
  replicates; the text `ItemInstanceID` is `RepSkip`.
- On the client, `UArchonLoadoutItemSlot::OnRep_ItemInstanceIdChanged` (native `+0x01BADCE0`) maps the CRC to an id
  through the slot's `Inventory` pointer, and does nothing while that pointer is null.
- The slot's `Inventory` is not RepNotify, and `AArchonLoadout::OnRep_Inventory` (native `+0x01BAD7D0`) re-resolves
  only the loadout's own ids, not its slots'. So a slot whose notification ran before the inventory actor reached the
  client is never resolved by the game.
- The slot's cells, weapon parts, dye and transmog notifications then also ran against no item (0037).

Seen: the arsenal shows an empty loadout (no armour, power 0) and the HUD a placeholder weapon XP of 8,888 / 100,000
after an inventory refresh or a hunt search, while the server still has the gear (October 2026). The data is not lost:
running the slot's own notifications again once the inventory has loaded restores it.

### 6.2 The local controller

**(departure 4)** The AGPL repair reads the controller its R14 block captured. R15 keeps its own, so it does not
depend on which DLL owns R14:

- A **before** handler on SP-1, matcher `(Exact, "Function Engine.PlayerController.ClientRestart")`: when the object
  is non-null, remember it as the local controller and remember the current world (`GWorld`, data `+0x06FC7F58`, read
  through R0's checked read; null if unreadable).
- Whenever the repair reads the controller (6.3 step 3), it first reads `GWorld` again; if that differs from the
  remembered world, it forgets the controller (and the world). The next `ClientRestart` in the new world sets it again.
  (After a travel or a return to the login screen the old controller is freed.)

### 6.3 One repair pass

An **after** handler on SP-1, matcher `(Prefix, "")` (every named call, after the chain returned). Client only. Once,
at the first pass, `DISABLE_LOADOUT_SLOT_REPAIR.flag` is read (R0's flag call, the answer kept for the process);
present: the repair never runs and logs nothing.

1. Run only on the **client game thread**: the thread of the first SP-1 dispatch the MIT DLL saw (recorded by R15's
   handlers on their first call, before any check). Any other thread returns.
2. Return if a pass is in progress (the pass itself dispatches `ProcessEvent`, which re-enters this handler) or if
   less than **1000 ms** passed since the last pass started (the due time is set at the start, before the collect).
3. **Collect** (pure walk, 7.1), from the local controller (6.2); every read checked, every object checked with R0's
   `is_live_object`, and any failure ends the collect with what it has:
   - the controller is readable for `0x9D0` bytes, live, and **is an** `AArchonPlayerController` (7.2); its `LOADOUT`
     (`AArchonLoadout*`) is non-null, readable for `0x690`, live. Remember the loadout pointer.
   - `LoadoutSlotDataArray` (`TArray<UArchonLoadoutSlotData*>`): for slot data indices `i` from 0 while
     `i < Num` and `i < 8`, the element non-null, readable for `0x390`, live;
   - its eight item slot pointers, in this order and with these names (for the log): `role`, `weapon`, `subweapon`,
     `helmet`, `chest`, `legs`, `arms`, `lantern` (7.1); each non-null, readable for `0xC0`, live; else skip it;
   - skip the item slot when its `ItemInstanceIdCrc` is 0 or its `ItemInstanceID` `FString` has `Num > 0` (it already
     knows its item);
   - its `Inventory` (`AArchonInventory*`) must be non-null, readable for `0x400`, live, **and** the game's loading
     evaluator must answer true for it: call `original(+0x01BA5E40)` (R0's `broker::original`: past every wrapper on
     LD-1, so LD-1's client and server overrides do not answer) with the inventory as its one argument; it returns a
     `bool` in `al`. Otherwise count the slot as **waiting** and skip it;
   - otherwise collect `(item slot, slot data index i, item slot position, crc)`, at most **64**.
4. If any slot is waiting and the loadout pointer differs from the one this was last logged for, log
   `[LoadoutSlotRepair] loadout=<loadout> <waiting> item slot(s) have an item CRC but no item id, and their inventory is
   missing or still loading` and remember the loadout. (Once per loadout, so the log shows a gap even when it heals
   itself.)
5. Nothing collected: the pass ends.
6. **Attempts.** A map from item slot address to attempts made, cleared when the loadout differs from the one it was
   built for or when it holds more than 512 entries. A slot with 3 attempts is skipped (an item really gone from the
   inventory is tried three times, not forever).
7. For each collected slot not skipped, in collect order: add one attempt; find the UFunction
   `Function Archon.ArchonLoadoutItemSlot.OnRep_ItemInstanceIdChanged` (7.3). Not found: log
   `[LoadoutSlotRepair] ArchonLoadoutItemSlot.OnRep_ItemInstanceIdChanged not found` and end the loop (step 8 still
   runs). Found: call `ProcessEvent(slot, that function, params)` (7.3), `params` a zeroed, 8-byte aligned block of
   `0x40` bytes (the parameter is the old `FItemInstanceIdRepl`, `0x18` bytes: an empty one). Then the slot is
   **resolved** iff it is still readable for `0xC0` and its `ItemInstanceID` `Num > 0`. Resolved: count it, and call
   `ProcessEvent(slot, f, null)` for each of `OnRep_CellsChanged`, `OnRep_ItemPartsChanged`, `OnRep_DyeChanged`,
   `OnRep_TransmogChanged` (each `Function Archon.ArchonLoadoutItemSlot.<name>`, skipped when not found). Log
   `[LoadoutSlotRepair] loadout slot <i> <name> crc=<CRC> resolved|still unknown (attempt <n>)`, `<CRC>` as `0x`
   followed by upper-case hex digits without leading zeros (`0x1A2B3C`).
8. If at least one slot resolved: call `ProcessEvent(loadout, Function Archon.ArchonLoadout.OnRep_GenericLoadoutChanged,
   null)` so the menus redraw (skipped when that function is not found).
9. The pass ends (clear the in-progress mark on every exit path, a panic included).

`<loadout>` in the log is the pointer in **decimal** (the AGPL's pointer format), `null` for a null pointer.

### 6.4 Why the order changes nothing

- The AGPL ran its writes (4, 5) at the top of the client and server `ProcessEvent` bodies, after its depth guard and
  before every other feature block. As MIT before handlers they run before every AGPL block of the call (R0 8.1), after
  R0's depth guard: the same place in effect. Order `-100` (section 8) keeps them before other MIT before handlers.
- The AGPL ran the repair after the original returned, after its inventory trace and before its R17 branding blocks.
  As an MIT after handler it runs after every AGPL after block. The repair only re-runs notifications on objects the
  branding never touches, so the order between them does not matter; R17's after handlers take orders above R15's.

### 6.5 What the calls re-enter

The repair's `ProcessEvent` calls go through the object's virtual `ProcessEvent` (7.3), so they enter the SP-1 chain
again: R0's depth guard counts them, every MIT and AGPL handler sees them (as when the AGPL made them), and R15's own
handlers see them too. Step 2's in-progress mark stops the repair from nesting; the writes' guard (4.1) stops those.

### 6.6 Patch 0026 (not ported)

The AGPL's client inventory trace (`[ClientInventoryTrace #n]` lines around every `ArchonInventory`/`ArchonLoadout`
RPC and notification, switched off by `DISABLE_CLIENT_INVENTORY_TRACE.flag`) is logging only and carries no feature id
(R0 7.5): it keeps running in the AGPL DLL whichever DLL owns R15, and goes with it at R21 (RT-3). Its five loadout
getters (`+0x01BA4FF0`, `+0x01B9CA60`, `+0x01B9FF30`, `+0x01B9A620`, `+0x01BA08D0`) are R6's facts (UI-12), not R15's.

---

## 7. Facts: the offset table lines and engine reads

### 7.1 Layouts (`offsets::r15`, UE 4.26 as built into 1.14.7)

Containers (UE's own):

| Fact | Value |
| --- | --- |
| `TArray<T>` | data pointer `+0x00`, `Num` `i32 +0x08`, `Max` `i32 +0x0C` |
| `FString` | a `TArray<u16>`; `Num` counts the terminating NUL, so the text is `Num − 1` units (none when `Num` is 0) |
| `TMap` (`TSet` over a `TSparseArray`) | element storage: a `TArray` at `+0x00` (`Num` = allocated slots); the allocation bit array's 4 inline words at `+0x10`, its heap pointer at `+0x20` (used when non-null); free count `i32 +0x34`. Element `i` is live iff bit `i` is set. Size `0x50`. The same model the MIT exporter uses (`native/rustic-isles-exporter/src/engine.rs`, `map_elements`) |
| A `TMap` element | the key/value pair, then two `i32` (hash next id, hash index); stride = pair size rounded to 8, + 8 |

Game types:

| Type | Field | Offset | Type |
| --- | --- | --- | --- |
| `UArchonCatalog` | `Catalog` | `+0x30` | `TMap<FString, FArchonCatalogItem>`; element stride `0x358`: key `FString +0x00`, value at `+0x10` |
| `FArchonCatalogItem` (size `0x340`) | `Tags` | `+0x98` | `TArray<FString>` (element `+0xA8`) |
| | `MaxQuantity` | `+0x300` | `i32` (element `+0x310`) |
| `UDataTable` | `RowStruct` | `+0x28` | `UScriptStruct*` |
| | `RowMap` | `+0x30` | `TMap<FName, uint8*>`; element stride `0x18`: key `FName +0x00`, row pointer `+0x08` |
| `FCellExchangeTableData` (a row) | `TimeRequired` | `+0x20` | `FTimespan` = `i64` ticks |
| `AArchonPlayerController` | `LOADOUT` | `+0x9C8` | `AArchonLoadout*` |
| `AArchonLoadout` | `LoadoutSlotDataArray` | `+0x670` | `TArray<UArchonLoadoutSlotData*>` |
| `UArchonLoadoutSlotData` | `PlayerRoleItemSlot` … `LanternItemSlot` | `+0x348`, `+0x350`, `+0x358`, `+0x360`, `+0x368`, `+0x370`, `+0x378`, `+0x380` | `UArchonLoadoutItemSlot*`: role, weapon, subweapon, helmet, chest, legs, arms, lantern |
| `UArchonLoadoutItemSlot` | `Inventory` | `+0x28` | `AArchonInventory*` |
| | `ItemInstanceID` | `+0x38` | `FItemInstanceIdRepl` (`0x18`): `ItemInstanceID` `FString +0x00` (so `Num` at slot `+0x40`), `ItemInstanceIdCrc` `u32 +0x10` (slot `+0x48`) |
| `UStruct` | `SuperStruct` | `+0x40` | `UStruct*` (for "is a", 7.2) |
| `UObject` | vtable | `+0x00` | `ProcessEvent` is entry `0x44` (byte `+0x220`) |
| `RF_ClassDefaultObject` | | `0x10` | in the object flags (`u32 +0x08`, R0 6.2) |

Readable sizes the collect checks (6.3): controller `0x9D0`, loadout `0x690`, slot data `0x390`, item slot `0xC0`,
inventory `0x400`.

RVAs (no hook; listed for the table and named for the slice):

| Name | RVA | Use |
| --- | --- | --- |
| The loading evaluator (`HasFinishedLoading`, LD-1's target) | `+0x01BA5E40` | 6.3, called through `original`; `bool (UObject*)` |
| `UArchonLoadoutItemSlot::OnRep_ItemInstanceIdChanged` (native) | `+0x01BADCE0` | not called directly: R15 dispatches the UFunction (7.3). Recorded for the reader |
| `AArchonLoadout::OnRep_Inventory` (native) | `+0x01BAD7D0` | not called. Recorded for the reader |
| `GWorld` | data `+0x06FC7F58` | 6.2 (already in R0's `data`) |

`offsets::r15` holds these as constants with one-line comments naming the type and field; no struct definitions, no
generated text (RT-2). The table test of R0 6.1 is unaffected (no new `TARGETS` line).

### 7.2 "Is a"

`is_a(object, class)`: read the object's class (`+0x10`), then follow `SuperStruct` (`+0x40`) at most 256 times; true
iff one of them equals `class`. Every read is checked; a failed read is false.

The class objects are found once by full name (R0 6.2's format) and kept for the process (native classes are never
freed): `Class Archon.ArchonCatalog`, `Class Engine.DataTable`, `Class Archon.ArchonPlayerController`. **(call)** The
search walks the object array comparing each object's short name first (`ArchonCatalog`, `DataTable`,
`ArchonPlayerController`) and builds the full name only for those; not found yet, it runs again at the feature's next
run. A name lookup that fails R0's self-check finds nothing, so every feature stays off (fails closed).

### 7.3 UFunctions and the `ProcessEvent` call

- UFunctions are found by full name, the same way as the classes, once each, and kept while `is_live_object` holds:
  `Function Archon.ArchonLoadoutItemSlot.OnRep_ItemInstanceIdChanged`, `...OnRep_CellsChanged`,
  `...OnRep_ItemPartsChanged`, `...OnRep_DyeChanged`, `...OnRep_TransmogChanged`, and
  `Function Archon.ArchonLoadout.OnRep_GenericLoadoutChanged`. (`AArchonLoadout` and `UArchonLoadoutItemSlot` each
  declare an `OnRep_GenericLoadoutChanged`; R15 calls the loadout's.)
- `process_event(object, function, params)`: read the object's vtable pointer (`+0x00`) and its entry `0x44`
  (`+0x220`), both checked; the entry must lie inside the executable's image (base to base + `SizeOfImage`, from the
  PE header R0 parses); call it as `(object, function, params)`. A failed check skips the call and counts as "not
  resolved". **(call)** The virtual call is what the AGPL's call does, and it matters: `AArchonLoadout` is an actor,
  whose `ProcessEvent` override runs before `UObject::ProcessEvent`.

### 7.4 The checked write

`write_i32(addr, value)` and `write_i64(addr, value)`: R0's `readable(addr, size)` and the region's protection is one of
`PAGE_READWRITE`, `PAGE_WRITECOPY`, `PAGE_EXECUTE_READWRITE`, `PAGE_EXECUTE_WRITECOPY`; then a volatile aligned write.
Anything else: no write (the item or row is skipped and not counted). Both live in `ffi/economy.rs`.

---

## 8. Registration and ownership

`economy::register()` (called in Init, before `handlers::freeze()`):

| Point | Slice | `order` | Matcher (RT-18 for R15) | Before | After |
| --- | --- | --- | --- | --- | --- |
| SP-1 | R15 | `-100` | `(Prefix, "")`: every call whose function has a name | the writes (4, 5) | — |
| SP-1 | R15 | `0` | `(Exact, "Function Engine.PlayerController.ClientRestart")` | the controller capture (6.2) | — |
| SP-1 | R15 | `0` | `(Prefix, "")` | — | the repair (6.3) |
| SP-2 | R15 | `-100` | `(Prefix, "")` | the writes (4, 5) | — |

- No handler absorbs: every before handler returns `Continue`.
- **(departure 6)** The AGPL ran the writes on every call, a null or unnamed function included; R15's matcher needs a
  name, so a call without one does not trigger the timed run (the next named call does, milliseconds later).
- Each handler first checks R0's detaching flag (R0 3.3) and returns when it is set. Each records the first-call thread
  (6.3 step 1) before anything else.
- `registry::LANDED` gains `"R15"`, so `rir_owns` answers 1 for `SP-1@R15` and `SP-2@R15`. The status line's
  `owns=<n>` grows by 2.

### 8.1 The AGPL side: no new seam patch

Patch 0041 already gates every R15 block on the two ids (R0 7.5): with the MIT DLL owning them, the AGPL runs neither
write on either role and no repair. Read against the AGPL source: no other code writes `MaxQuantity` or the fusion
table, no other caller runs the repair, and nothing the AGPL keeps reads state these blocks set. **No runtime patch
0043 is needed.** The ungated AGPL block left is the 0026 trace (6.6), logging only.

### 8.2 Cross-slice state (R0 7.5's table, a row to add)

| State | Set by | Read by | Rule |
| --- | --- | --- | --- |
| the local controller at `ClientRestart` | the AGPL's `SP-1@R14` block | the AGPL 0026 trace (logging only) | none: R15 keeps its own (6.2); once R14 is MIT the AGPL trace reports no loadout, which only changes a log |

### 8.3 Install order (RT-17 for R15)

R15 installs no hook, so it adds no order. Its handlers depend only on SP-1/SP-2 being installed; when an install
fails, R15's handlers never run (the features are off, the game runs as without them).

---

## 9. Seam lines for other rows (not R15's change)

- **The host kit** (`crates/dainty-hostkit/src/hostconfig/tunables.rs`, the `DIRECTOR` table, beside the RT-11 hub and
  AFK lines): `of("keepSupplyStackCaps", "DAINTY_KEEP_SUPPLY_STACK_CAPS", Kind::Switch)` and
  `ranged("cellFusionMinutes", "DAINTY_CELL_FUSION_MINUTES", 0, 10_080)`, so `settings.toml`'s `[director]` takes
  `keep_supply_stack_caps = true|false` and `cell_fusion_minutes = 0..10080`; unset writes nothing (today's behaviour:
  uncap, 30 minutes). A test that both reach the Director's env file only when set, as for the hub lines. The host kit
  needs no other change: the Director forwards its environment.
- **The docs** (`restricted/docs/hosting.md`, the `[director]` rows of the tunables table, and `hosting-linux.md` /
  `hosting-docker.md` where they list them): two rows, `keep_supply_stack_caps` (`DAINTY_KEEP_SUPPLY_STACK_CAPS`,
  `true`/`false`, unset: supplies uncapped) and `cell_fusion_minutes` (`DAINTY_CELL_FUSION_MINUTES`, 0 = the game's
  own, 1–10080, unset: 30). A note that the two flag files are no longer read once the MIT runtime owns R15.
- **`restricted/docs/paradox-patches.md`** (integration pass): 0024 and 0029 are `settings.toml` settings in the MIT
  runtime; 0026 is not ported; 0027 and 0037 are ported by R15.
- **The migration guide**, section 2: the module line `rustic-isles-runtime/src/economy/` and
  `rustic-isles-runtime-core/src/economy/` ← runtime patches 0024, 0027, 0029, 0037 (R15).
- **The launcher** (only on Q-1's answer): pass the server's two values to the game it starts.

---

## 10. Tests

### 10.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`), against a fake `Memory` holding the structures of 7.1:

- **Settings:** `DAINTY_KEEP_SUPPLY_STACK_CAPS` unset, `true`, `false`, `TRUE`, `1`, 16 characters (refused by
  `env_value`), each with its outcome and note; `DAINTY_CELL_FUSION_MINUTES` unset (30), `0`, `1`, `10080`, `10081`,
  `-1`, ` 30` (accepted, R1's `strtol` rule), `30 `, `x`, with the ticks (`30` → 18 000 000 000).
- **Catalog:** a map with free slots between live ones (the bit array inline, and a second case on the heap); items with
  caps 0, 1, 2 and 30, tagged `supplies`, `Supplies`, `supplies ` and untagged: only cap ≥ 2 with exactly `supplies`
  is written; the written values; the log text with fewer and more than 8 items (the list stops at 8, the count does
  not); a second walk writes nothing and logs nothing; a map of 1 000 001 elements and a tags count of 1 025 are
  skipped; an unreadable element is skipped without a write.
- **Fusion:** a table whose rows hold 30 and 60 minutes and a null row pointer: the counts (`changed/rows`), the
  `before` list with names and minutes, the second walk silent; the target 0 path is never walked.
- **Data table search:** the first non-default `DataTable` whose row struct is named `CellExchangeTableData`; a default
  object and another row struct are passed over.
- **Is a:** a three-level class chain; a cycle stops at 256; an unreadable link is false.
- **Loadout collect:** a controller → loadout → two slot data → item slots graph: slots with CRC 0, with an id, with a
  null inventory, with an unloaded inventory (the evaluator is an injected function), resolvable; the waiting count and
  the collected tuples in order; more than 8 slot data entries (only 8 walked); 64 as the collect limit; a dead object
  (index round trip fails) skipped.
- **Repair bookkeeping:** the 1000 ms timer (999 ms skips, 1000 runs); attempts stop at 3; the map cleared on a new
  loadout and past 512 entries; the waiting line once per loadout; the controller forgotten when the world changes.
- **Texts:** every log line of sections 3 to 6 for fixed inputs; the CRC format (`0x0` is never logged: CRC 0 is
  skipped; `0x1A2B3C`); the decimal pointer and `null`.
- The registry: `owns("SP-1@R15")` and `owns("SP-2@R15")` true with R15 landed.

DLL (`cargo test -p rustic-isles-runtime`, Windows): the checked write refuses a read-only page and a `PAGE_NOACCESS`
page and writes a read-write one; `register()` puts four handlers on the two points with the orders of section 8.

### 10.2 The maintainer's play-test (surface section 8, R15 row)

"Tonic stacks beyond the cap; a Middleman fusion takes 30 min; arsenal keeps its loadout when the inventory arrives
late." With both DLLs injected (R0 11.1 step 1) and the R15 build:

1. **Ownership.** The client's `rustic_isles_dll_port0.log` shows `owns=<n>` two higher than the R0-only build, and
   exactly one `[SupplyCap] removed ...` line per catalog (both DLLs use the same texts, so two lines would mean both
   ran it).
2. **Supplies.** Log in and enter Ramsgate: `[SupplyCap] removed the stack cap of <n> supply item(s): ...` in the
   client's log and in the Ramsgate server's log (`rustic_isles_dll_port<N>.log`). Have the backend grant more than 30
   of a tonic (patch 0023 refills at login): the stack shows above 30 and stays after a hunt.
3. **Fusion.** At the Middleman, start a cell fusion: 30 minutes on the screen; the server's log shows
   `[CellFusion] set .../... fusion recipe(s) of ... to 30 min`. Set `cell_fusion_minutes = 5` (or
   `DAINTY_CELL_FUSION_MINUTES=5` in the Director's env) and restart the hub: a new fusion ends after 5 minutes (the
   client's screen still says 30, section 3). `0` keeps the game's own times and logs that.
4. **Loadout.** Search for a hunt and return, several times, then play a hunt and return to Ramsgate: the arsenal keeps
   its loadout, cells, parts, dye and transmog. When the log shows `[LoadoutSlotRepair] ... resolved`, the arsenal is
   right after it. With `DISABLE_LOADOUT_SLOT_REPAIR.flag` beside the client's executable, no `[LoadoutSlotRepair]`
   line appears.
5. **Without the MIT DLL**, the AGPL does all three as before (patch 0041's fallback).
6. **Keep caps.** `DAINTY_KEEP_SUPPLY_STACK_CAPS=true` on the client: `[SupplyCap] ... the game's stack caps are kept`.

Only CI has run until these steps are done; nothing here is checked against the real client in CI.

---

## 11. Calls, departures, ambiguities and settled questions

Calls:

1. RT-11 names: `DAINTY_KEEP_SUPPLY_STACK_CAPS` (`true`/`false`, default uncap) and `DAINTY_CELL_FUSION_MINUTES`
   (0–10080, default 30, 0 = the game's own); `settings.toml` `[director]` `keep_supply_stack_caps`,
   `cell_fusion_minutes`.
2. The client reads the same variables and otherwise runs on the defaults; passing the server's values to the client is
   question Q-1.
3. A slice registers through one line in `lifecycle.rs` and one in `lib.rs`.
4. The feature log texts are the AGPL's (`[SupplyCap]`, `[CellFusion]`, `[LoadoutSlotRepair]`), so a host's searches
   keep working; the DLL that wrote a line is told by the ownership status, not the tag.
5. Classes and UFunctions found by full name, short name first, cached.
6. The repair's `ProcessEvent` calls go through the virtual entry `0x44`.

Departures (none changes what a player sees with the defaults):

1. The flag files `KEEP_SUPPLY_STACK_CAPS.flag` and `KEEP_CELL_FUSION_TIME.flag` are not read (RT-11).
2. `CELL_FUSION_MINUTES` becomes `DAINTY_CELL_FUSION_MINUTES`.
3. Patch 0026's trace is not ported (RT-3); `DISABLE_CLIENT_INVENTORY_TRACE.flag` stays with the AGPL trace.
4. R15 captures its own local controller at `ClientRestart` instead of reading R14's.
5. One in-progress guard keeps the writes from overlapping across threads.
6. The writes are triggered only by calls whose function has a name.

Ambiguity: the AGPL never states which thread its writes run on; R15 runs them on whichever thread dispatches (with the
guard of departure 5), and the repair only on the client game thread, as the AGPL.

Settled here:

- **RT-11 for 0024 and 0029:** the two variables and `settings.toml` keys of call 1, seam lines in section 9.
- **RT-14 for R15:** of the four flags Appendix A gave R15, `DISABLE_LOADOUT_SLOT_REPAIR` is kept;
  `KEEP_SUPPLY_STACK_CAPS` and `KEEP_CELL_FUSION_TIME` are replaced by the settings; `DISABLE_CLIENT_INVENTORY_TRACE`
  stays AGPL with its trace and goes at R21.
- **RT-17 for R15:** no hooks, no order (8.3).
- **RT-18 for R15:** the matchers of section 8.
- RT-12, RT-13, RT-15, RT-16 are not R15's.
- **No runtime patch 0043** (8.1).

Question for the maintainer:

- **Q-1.** Should the launcher pass a server's `keep_supply_stack_caps` and `cell_fusion_minutes` to the client, so a
  host who changes them also changes what the client's Middleman screen shows and whether the client keeps caps?
  Recommended: not now. With the defaults nothing differs; a host who changes them is told in `hosting.md` that the
  client shows the default fusion time. **Taken up (maintainer, 2026-11-13):** the host kit writes both into the
  backend's file too, the status sends them as `economy` (`dainty-protocol`, refused rather than clamped), and PLAY
  adds them to the game's environment.

## 12. Size

Rust without tests: the core crate about 330 lines (settings 50, containers and walks 180, repair bookkeeping 60, texts
40) plus 40 offset lines; the DLL crate about 220 (registration and handlers 90, scans and caches 70, FFI writes and
calls 60). Tests about 400 lines.
