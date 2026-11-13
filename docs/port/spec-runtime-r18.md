<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R18, escalation and Trials (RD27, plan item 11)

R18 moves two game fixes into the MIT runtime (`spec-runtime-surface.md` section 8, R18's row): **ESC-1**, which makes
the Trials current and launchable by rewriting the arena schedule rows before `FGameplayScheduler::Initialize` copies
them, and the **escalation buff-choice spawn**, which makes every player of an Escalation get their own relic pickups
at the end of each round. The surface page's third item, "the relic choice" on the client's `ProcessEvent`, turns out
to be diagnostics only; section 5 settles it.

An implementer builds from this page, `spec-runtime-r0.md`, `spec-runtime-r1.md` and the surface page alone, and never
reads `restricted/`. What crossed is facts: RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7, CL 647472), engine and
game layout offsets and names (from the maintainer's own Dumper-7 dump of this build, RT-2), variable and flag names,
the exact log texts the operator reads, and behaviour. The AGPL side stands down through seam patch 0041 alone
(section 8); R18 needs no patch of its own, so number 0049 stays unused.

Conventions are the surface page's: an RVA is `+0x0XXXXXXX` from the executable's image base; **C** client, **S**
server, **B** both. **(call)** marks a decision this page makes, **(ambiguity)** something the source did not settle,
**(departure)** a deliberate difference from the AGPL runtime; all are collected in section 11.

---

## 1. Scope

| R18 owns | Kind (R0 7.5) | Role | Section |
| --- | --- | --- | --- |
| `ESC-1`: the Trials schedule rewrite on `FGameplayScheduler::Initialize` | hook (detour) | S | 3 |
| `SP-2@R18`: noting the escalation round-finished transition | feature on SP-2 | S | 4.2 |
| `SP-3@R18`: the deferred buff-choice spawn | feature on SP-3 | S | 4.3 |
| The flag `ESCALATION_NO_BUFF_CHOICE_SPAWN` | flag | S | 4.4 |

All three ids are already in R0's registry with slice `R18` (`ESC-1` as a detour, the two features). R18's registry
edit is adding `"R18"` to `LANDED`. No new id (no `SP-1@R18`: section 5).

Not R18: the escalation networking fixes a two-player Escalation needs (the controller channel guards and the RPC guard
on an unbound channel, patch 0039: R12's `RG-*` and `SP-3@R12`); the Trials airship HUD restore (R14); the escalation
and relic traces, the relic-click diagnostic, the Gauntlet census and the Trials bleedout sampler (diagnostics, RT-3,
section 5).

**Dependencies.** R0 (broker, dispatch chains, flags, log, checked memory, engine names) and R1 (section 9's trials
rotation and week, already in `rustic-isles-runtime-core::config::trials` and `rustic-isles-runtime::config`). It also
needs a small reflection kit to find and call one UFunction (section 6.3); R17's spec defines it, and whichever of R17
and R18 lands first creates it.

---

## 2. Where the code goes

| Path | Holds | Pure? |
| --- | --- | --- |
| `crates/rustic-isles-runtime-core/src/escalation/mod.rs` | the module, re-exports | yes |
| `crates/rustic-isles-runtime-core/src/escalation/trials.rs` | reading the scheduler's table array over R0's `Memory` trait and planning the row writes (3.3 to 3.5); the log texts | yes, CI-tested everywhere |
| `crates/rustic-isles-runtime-core/src/escalation/buff_choice.rs` | the buff-choice state machine (4.1): note, take, dedupe; the tick cadence; the log texts | yes |
| `crates/rustic-isles-runtime/src/escalation.rs` | registration of the two dispatch handlers and the ESC-1 install, called from Init; the safe glue that applies the plans | no (Windows) |
| `crates/rustic-isles-runtime/src/ffi/escalation.rs` | the ESC-1 detour body and the one call into the game's `ProcessEvent` original (R0 2: every detour body and raw call lives under `ffi`) | no (Windows) |

Shared files R18 touches, each by one or two lines: `rustic-isles-runtime-core/src/registry.rs` (`LANDED`),
`rustic-isles-runtime-core/src/offsets.rs` (the `TARGETS` line and the layout lines of 6.1, each commented
`// R18`), `rustic-isles-runtime-core/src/lib.rs` and `rustic-isles-runtime/src/ffi/mod.rs` (`pub mod escalation;`),
`rustic-isles-runtime/src/lib.rs` (`mod escalation;`) and `rustic-isles-runtime/src/lifecycle.rs` (Init: call
`escalation::register()` before `handlers::freeze()`, and add `("ESC-1", escalation::install_esc1(base))` to the server
install list after `SP-2`). If a slice that lands earlier has already turned the install list into per-slice
registration, use that instead.

Everything R18 does runs **on servers only**. On a client nothing is registered or installed, so the slice is inert.

---

## 3. ESC-1: the Trials schedule (`FGameplayScheduler::Initialize`)

### 3.1 Why

The game's scheduler builds one state per schedule row **once**, copying each row's start and end into its state, and
later compares those copies with the real UTC time. The arena (Trials) rows of 1.14.7 are fixed, non-repeating windows
from 2019 to 2022, so on a 2026 clock no arena row is ever active and the Trials board stays greyed. Changing the rows
after `Initialize` does nothing (the copies are already made), and the scheduler component's own time offset is not
applied by the comparison. So the rows must be rewritten **before** `Initialize` copies them. The scheduler then marks
the current row active, the server replicates that to clients, and the Trials board ungreys with that week's Behemoth.

### 3.2 The hook

| Fact | Value |
| --- | --- |
| Target | `FGameplayScheduler::Initialize`, RVA `+0x01D95B70`, found by RVA (as the AGPL does; no prologue check) |
| Signature | `(scheduler: *mut c_void, tables: *const TArray<UDataTable*>)`; the AGPL treats the return as void |
| Role | S (every server: DeployServer gameservers and player hosts alike) |
| Install | through the broker, `order = 0`, band `Mit`; in the server install list after `SP-2` |
| Handler position | **before** the original: plan and apply the row writes (3.3 to 3.5), then call `next` with the original arguments |

**(call) Return value.** Declare the wrapper as returning a `u64` and return what `next` returned. The AGPL's wrapper is
declared void, so whatever the original left in `RAX` reaches the caller unchanged in practice; passing it through
keeps that true by construction whatever the real return type is.

The wrapper never absorbs. If detaching, it only calls `next` (R0 3.3). A panic in the planning logs once
(`[Panic] ESC-1 ...`) and still calls `next` (R0 2).

**(call) No prologue check.** The AGPL hooks the bare RVA and it has worked on 1.14.7 since 2026-07. An implementer who
reads the prologue bytes from the executable on the maintainer's PC may switch the line to `RvaAndBytes`; nothing else
changes.

### 3.3 Reading the tables

`tables` points at a `TArray<UDataTable*>`: data pointer at `+0x00`, `Num` (`i32`) at `+0x08`.

1. `tables` must be readable for `0x10`; the data pointer non-null; `1 <= Num <= 64`; and **(departure)** the data
   readable for `Num × 8` bytes (the AGPL checked only the first 8, then read every entry under its exception guard).
   Anything else: no write, no log line; call `next`.
2. For each entry in order: skip a null table or one not readable for `0x40`. Take the table's **name** (its `FName`
   at `+0x18`, rendered with its number suffix as R0's `fname_at` does, short form). Skip the table unless the name
   contains `arena` or `Arena` (two case-sensitive substrings; `ARENA` does not match).
3. Iterate the table's `RowMap` (6.1). For each used element: the key is the row name (`FName`, rendered with its number
   suffix), the value the row pointer. Skip a null row or one not readable for `0x30`. Parse the name with R1's
   `parse_week_row` (exactly `Scheduled_Arena_Hunt_` and three digits, 1 to 181); skip any other name. Record
   `rows[week] = row` (a later duplicate replaces an earlier one) and count every parsed row in `found` (duplicates
   counted).

**The number suffix matters.** UE stores a name that ends in `_<digits>` without a leading zero as the base name plus a
number: `Scheduled_Arena_Hunt_181` is the text `Scheduled_Arena_Hunt` with number 182, while `Scheduled_Arena_Hunt_042`
keeps its digits in the text (number 0). R0's `with_number` (text, then `_` and number − 1 when the number is non-zero)
rebuilds both; reading the pool text alone would lose rows 100 to 181. A CI test pins it (section 9.1).

### 3.4 The clock and the week

- `now` = `GetSystemTimeAsFileTime` converted with R1's `filetime_to_ticks` (.NET ticks), read once per `Initialize`
  call.
- `minutes` = R1's `trials_rotation_minutes()` (`TRIALS_ROTATION_MINUTES`, default 10 080); `rotation = minutes ×
  600 000 000` ticks.
- `rotation_at(now, minutes)` (R1 9) gives `bucket_start` and `current` (the week, 1 to 181).
- `pinned` = R1's `trials_week()` (`RUSTIC_ISLES_TRIALS_WEEK`, 1 to 181, else none). The Director sets it for an arena
  hunt so a trial keeps the row it launched with.

Both variables are read once per process (R1 keeps them). The AGPL re-read the pinned week on every call; the process
environment does not change after start, so the answers agree.

### 3.5 The writes

Each row is an `FScheduleData`: `StartTime` (`FDateTime`, an `i64` of .NET ticks) at `+0x20`, `EndTime` at `+0x28`.
Constants: one hour = 36 000 000 000 ticks, one day = 864 000 000 000 ticks.

**Pinned** (`pinned = Some(week)`), per matching table:

- if `rows[week]` exists: `start = now − 1 hour`, `end = start + 7 days`; write both.
- log `[TrialsSchedule] Initialize: launch-pinned row='<row>' duration=7d table=<table>` when the row exists, else
  `[TrialsSchedule] Initialize: launch-pinned row missing '<row>' table=<table>`, `<row>` from R1's `week_row(week)`.
- nothing else in that table is written (the other rows keep their stock windows).

The fixed seven days, not the rotation, are deliberate: a launched trial keeps its row for a native week, so a rotation
boundary during the fight cannot change its modifiers.

**Rotating** (no pinned week), per matching table:

- for every week 1 to 181 with a row: `forward = (week − current + 181) mod 181`;
  `start = bucket_start + forward × rotation`; `end = start + rotation`; write both; count it in `patched`.
- log `[TrialsSchedule] Initialize: rotating intervalMinutes=<minutes> currentRow='<week_row(current)>'
  patchedRows=<patched>/<found> bucketStartTicks=<bucket_start> horizonEndTicks=<bucket_start + 181 × rotation>
  table=<table>` (one line; numbers in decimal).

So the 181 rows become contiguous windows, the current one first, the rest following in week order and wrapping,
which covers the next 181 rotations without another restart.

**Applying a write.** The plan is a list of `(address, i64)`; the DLL applies each with a checked write: the 8 bytes
must lie in one committed region whose protection is `PAGE_READWRITE`, `PAGE_WRITECOPY`, `PAGE_EXECUTE_READWRITE` or
`PAGE_EXECUTE_WRITECOPY` without `PAGE_GUARD`; otherwise that write is skipped (and not counted). R0 has no data writer;
R18 adds `ffi::memory::write::<T: Copy>(addr, value) -> bool` with the protection test as a pure function in the core
crate's `memory.rs` beside `region_allows` **(call)**. R15's data writes need the same helper; whichever lands first
adds it.

**(departure) Counted writes.** The AGPL counted a row as patched before writing it under its exception guard; the MIT
counts only writes that were applied. On a healthy server the two are equal (`181/181`).

---

## 4. The escalation buff-choice spawn (`SP-2@R18`, `SP-3@R18`)

### 4.1 Why, and the state

In an Escalation (`BPGM_ArchonEscalation_C`), every player should get their own relic pickups when a round finishes.
The game has the function that does it, `UEscalationGameModeComponent::SpawnPlayerBuffChoicesForFinishedRound`
(BlueprintCallable, no parameters): it walks every player controller with an escalation component and season data,
builds that player's options and spawns their pickups. The shipped Blueprint graph never calls it, so without the fix
the party shares the one level-placed crystal: the first player to finish the interaction advances the round for
everyone and the others are stuck. The fix calls the game's own function at the round-finished transition; the
option lists, the player walk and the spawning stay the game's.

State (server, one process-wide value; both handlers run on the game thread, but guard it with a `Mutex` anyway):

- `pending: Option<Request>` with `Request { component: usize, round: i32 }` (one slot: a newer note replaces an
  undispatched one);
- `last: Option<(usize, i32)>`: the component and round last **dispatched** (not last noted).

The core crate holds this as a pure state machine (`note`, `take`, `mark_dispatched`) so 9.1 tests it.

### 4.2 Noting the transition (SP-2, before)

| | |
| --- | --- |
| Point, position | SP-2, a **before** handler, `order = 1800` **(call)**; never absorbs (returns `Continue`) |
| Matcher | `Contains` `EscalationGameModeComponent.AuthOnGameStateChanged` (RT-18 for R18). In 1.14.7 it catches exactly one UFunction, `Function Archon.EscalationGameModeComponent.AuthOnGameStateChanged`; a Blueprint subclass's override would be named `..._C.AuthOnGameStateChanged` and is not caught (the AGPL's test was the same substring) |
| Params | `{ NewState: u8 at +0x00 (EEscalationGameModeState), StateTransitionType: u8 at +0x01 (EStateTransitionType) }`, size 2 |

On a match, in order (any failed step returns without a note or a log line):

1. object and params non-null; `ESCALATION_NO_BUFF_CHOICE_SPAWN.flag` absent (R0's cached probe, 4.4); params readable
   for 2 bytes.
2. `NewState == 7` (`EventEnding`) **and** `StateTransitionType == 0` (`Begin`). Entering `EventEnding` is the moment
   the round has just finished, about 4 s before the level-placed crystal becomes interactable, so the personal pickups
   exist before anyone can queue on the shared one.
3. The object (the mode component) readable for `0xC0`. Its state component is the pointer at `+0xB8`
   (`UEscalationGameStateComponent*`). `round` = the `i32` `CurrentRound` at state `+0x134` if the state component is
   non-null and readable for `0x138`, else `i32::MIN`.
4. If `last == Some((object, round))`: already served, return.
5. `pending = Some(Request { object, round })`; log `[EscalationBuffChoiceSpawn] queued mode=<object> round=<round>
   trigger=EventEnding(7) entry`.

Pointers in these lines are written in **decimal**, `null` for zero (the AGPL's format, kept so the maintainer's greps
work across both runtimes, **(call)**). `round` is a signed decimal (`-2147483648` when unknown).

Nothing is dispatched from inside `ProcessEvent`: spawning actors underneath the dispatch risks recursion, so the call
waits for the tick.

The before position matches the AGPL, whose block ran before the original. As a MIT before handler it now also runs
before every AGPL block of the same call, which changes nothing: it only records.

### 4.3 The spawn (SP-3, after)

| | |
| --- | --- |
| Point, position | SP-3, an **after** handler (after the engine tick, as the AGPL), `order = 1800` **(call)** |
| Cadence | its own tick counter, incremented on every call; the body runs when `counter & 0x3F == 0` (every 64th tick, about once a second at 60 fps), as the AGPL's once-a-second block |

The body, on its ticks:

1. `take()` the pending request (clearing it); none: return.
2. The component must be readable for `0xC0` **and** pass R0's `is_live_object` (the object array round trip).
   Otherwise log `[EscalationBuffChoiceSpawn] skipped: mode component no longer live mode=<component> round=<round>`
   and return.
3. Find the UFunction `SpawnPlayerBuffChoicesForFinishedRound` declared by the class named
   `EscalationGameModeComponent`, searching the component's class and its super chain (6.3). Cache it once found (a
   `UFunction` is never unloaded). Not found: log `[EscalationBuffChoiceSpawn] SpawnPlayerBuffChoicesForFinishedRound
   UFunction not found on mode=<component>` and return **without** updating `last` (the request is still consumed).
4. `last = Some((component, round))`.
5. Log `[EscalationBuffChoiceSpawn] dispatching mode=<component> round=<round>`.
6. Call the game's `ProcessEvent` **original** (`broker::original(ProcessEvent)`, past every wrapper, MIT and AGPL) with
   `(component, function, null)`: the function takes no parameters.
7. Log `[EscalationBuffChoiceSpawn] dispatched  mode=<component> round=<round>` (two spaces after `dispatched`, as the
   AGPL writes it so the two lines align).

Calling the original, not the hooked entry, matches the AGPL (it called its own trampoline): the spawn call itself is
seen by no dispatch handler in either DLL. The game's own `ProcessEvent` calls made **inside** the spawn still go
through the whole chain as usual.

**(call) The cadence counter is R18's own.** The AGPL shared one counter between this block and R14's retries; with the
two in different DLLs or modules the phases may differ by up to 63 ticks, which nothing depends on.

### 4.4 The flag

| Flag | Kept? | Meaning |
| --- | --- | --- |
| `ESCALATION_NO_BUFF_CHOICE_SPAWN` | kept | read through R0's flag cache on every note (so it takes effect within 4 s): no request is queued, so nothing spawns. For A/B against the stock shared-crystal behaviour |
| `GAUNTLET_CENSUS_OFF` | **moved to R19, not ported (call)** | it switches off a client census of the Gauntlet panel, logging only (RT-3) |

Surface Appendix A listed both for R18. `TRIALS_GRACE_DIAG` (surface: R19) and the retired `ESCALATION_NO_RELIC_FANOUT`
(named only in a comment; its code was removed on 2026-07-26) are not R18's either.

---

## 5. The relic choice (surface 4.3): settled as diagnostics

The surface page grouped "escalation relic choice (`*Relic*`, `EscalationGameModeComponent.*`)" under the client's
`ProcessEvent`. Read against the AGPL, everything there is logging:

- the escalation flow trace on both SP-1 and SP-2 (about 50 escalation, relic and state-machine UFunction names, with
  per-category budgets), off with `PRODUCTION_LOG_PROFILE` on servers;
- a read-only relic-click diagnostic on the client: it remembers which escalation component opened the relic screen
  and, when a relic click returns without the server RPC (`ServerSelectRelicOption`), logs why (it calls only pure
  getters);
- the Gauntlet panel census (`GAUNTLET_CENSUS_OFF`).

None writes game state or changes a call's outcome, so per RT-3 none is ported, and there is **no `SP-1@R18`** feature
id (R0's registry already has none; the seam gates none of these blocks). They stay in the AGPL runtime until R21.

The behaviour that made relic choices work in multiplayer lives elsewhere: the per-player pickups (this slice, 4), and
the server networking fixes that stop the chooser's controller channel from being adopted or crashing the RPC (R12).
Until R12 lands those stay AGPL and keep working through the seam, so R18 does not wait for R12; its two-player
play-test simply runs with them still in the AGPL runtime.

---

## 6. Facts the slice adds

### 6.1 Offset table lines (`offsets.rs`, each commented `// R18`)

`TARGETS`:

```text
ESC-1  rva 0x01D95B70  FoundBy::Rva  "FGameplayScheduler::Initialize"
```

`layout` (UE 4.26 and Archon types as built into 1.14.7; names from the maintainer's Dumper-7 dump, RT-2):

| Name | Value | What |
| --- | --- | --- |
| `TARRAY_DATA`, `TARRAY_NUM` | `0x00`, `0x08` (`i32`) | any `TArray` |
| `DATATABLE_ROW_MAP` | `0x30` | `UDataTable.RowMap`, a `TMap<FName, uint8*>` (`0x50` bytes) |
| `SPARSE_DATA`, `SPARSE_NUM`, `SPARSE_MAX` | `0x00`, `0x08` (`i32`), `0x0C` (`i32`) | the map's sparse array: element pointer, `ArrayNum`, `ArrayMax` |
| `SPARSE_BITS_INLINE`, `SPARSE_BITS_HEAP` | `0x10` (four `u32`), `0x20` (pointer) | the allocation bits: at the heap pointer when it is non-null, else inline |
| `SPARSE_FREE_COUNT` | `0x34` (`i32`) | `NumFreeIndices` |
| `ROW_MAP_STRIDE`, `ROW_MAP_KEY`, `ROW_MAP_VALUE` | `0x18`, `0x00` (`FName`), `0x08` (row pointer) | one map element |
| `SCHEDULE_ROW_START`, `SCHEDULE_ROW_END` | `0x20`, `0x28` (`i64` .NET ticks) | `FScheduleData.StartTime`, `.EndTime` |
| `SCHEDULE_ROW_READ` | `0x30` | how much of a row must be readable |
| `ESC_MODE_STATE_COMPONENT` | `0xB8` (pointer) | `UEscalationGameModeComponent.EscalationGameStateComponent` |
| `ESC_MODE_READ` | `0xC0` | how much of the mode component must be readable |
| `ESC_STATE_CURRENT_ROUND` | `0x134` (`i32`) | `UEscalationGameStateComponent.CurrentRound` |
| `ESC_STATE_READ` | `0x138` | how much of the state component must be readable |
| `ESC_STATE_EVENT_ENDING` | `7` | `EEscalationGameModeState::EventEnding` (`InEvent` 6, `EventSummary` 8, `BetweenEvents` 9) |
| `TRANSITION_BEGIN` | `0` | `EStateTransitionType::Begin` (`End` is 1) |

The row map is iterated exactly as the MIT exporter's `map_elements` does (`native/rustic-isles-exporter/src/engine.rs`,
MIT; read it): an element index `i` from 0 to `ArrayNum − 1` is used iff bit `i` of the allocation bits is set
(`word i / 32`, bit `i % 32`). **(call)** Bounds: `0 <= ArrayNum <= ArrayMax <= 4096`, and the element block readable
for `ArrayNum × 0x18` bytes, else the table is skipped; the bits readable for `ceil(ArrayNum / 32) × 4` bytes, else
the table is skipped. (The AGPL iterated through the dump's container with no bound; the arena table has a few hundred
rows at most.)

### 6.2 Engine names

| Name | Use |
| --- | --- |
| `Scheduled_Arena_Hunt_001` to `_181` | the arena rows (R1 9) |
| `EscalationGameModeComponent.AuthOnGameStateChanged` | 4.2's matcher |
| class `EscalationGameModeComponent`, function `SpawnPlayerBuffChoicesForFinishedRound` | 4.3 step 3 |
| `BPGM_ArchonEscalation_C` | the Escalation game mode (play-test, logs) |

### 6.3 The reflection kit (shared with R17)

R0 gives no way to find or call a UFunction. R17's spec defines the kit; R18 uses these parts of it, with these facts:

- **Find a function in a class chain** (core `reflect.rs`, pure over `Memory`): starting at the object's class
  (`UObject +0x10`), walk `UStruct.SuperStruct` (`+0x40`); in each class walk its `UStruct.Children` (`+0x48`) list
  through `UField.Next` (`+0x28`); a child matches when its name is `SpawnPlayerBuffChoicesForFinishedRound` and its
  outer (the declaring class) is named `EscalationGameModeComponent`. Bound the walks (64 supers, 4096 children per
  class) and check every pointer is readable for `0x30` first.
- **Call the original** (`ffi/reflect.rs`): `process_event_original(object, function, params)` calls
  `broker::original(base + 0x02861880)` with the three arguments. This is the one variant R18 adds if R17 has not:
  R17's own call goes through the hooked entry, R18's must not (4.3 step 6).

---

## 7. Install order and cross-slice state (RT-17, R0 7.5's table)

**RT-17 for R18: settled, no order rule.** `FGameplayScheduler::Initialize` runs when the game creates its scheduler,
long after `DllMain`; R18 installs ESC-1 in Init after `SP-2`, as the surface's 3.11 list places it, but any position
in the server list works. The two handlers' places in their chains are fixed by `order` (section 4), not install order.

**Cross-slice state.** The buff-choice note and spawn share state only with each other, and they move together (one
slice, both features). ESC-1 shares none. The AGPL blocks that stay (the escalation traces) read none of R18's state.
No row is added to R0 7.5's table.

**RT-18 for R18: settled** in 4.2: one `Contains` matcher, catching one name.

---

## 8. The AGPL side: seam patch 0041 is enough

When `rir_owns` answers yes for R18's ids, patch 0041 already does everything R18 needs (no patch 0049):

| Id | What the AGPL runtime does when the MIT owns it |
| --- | --- |
| `ESC-1` | its `MH_CreateHook` on `+0x01D95B70` maps to `ESC-1`, which is owned: no wrapper; its original pointer becomes `rir_original(target)`; its enable succeeds as a no-op. Its install log line still prints a `create=` and `enable=` status |
| `SP-2@R18` | its server `ProcessEvent` skips its round-finished note |
| `SP-3@R18` | its tick skips its spawn pump |

So with the MIT owning R18 the AGPL neither patches the schedule nor queues or spawns buff choices, and with the MIT
DLL absent it does both exactly as before. Its escalation traces, the relic-click diagnostic and the Gauntlet census
stay on and are unaffected. The AGPL also carries a never-called scheduler-offset helper (dead code); it is not ported.

---

## 9. Tests

### 9.1 In CI (`cargo test -p rustic-isles-runtime-core`, every platform)

Over R0's `BufferMemory` with a fake name source:

- **Table array:** `Num` 0, 1, 64, 65; a null data pointer; data readable for fewer than `Num × 8` bytes; a null and
  an unreadable table entry skipped.
- **Table names:** `DT_ArenaSchedule` and `arena_rows` match, `ARENA_ROWS` and `DT_Escalation` do not.
- **Row map:** used and free slots (bits inline and on the heap); `ArrayNum` over 4096 skips the table; an element
  block or bits block that is unreadable skips the table; a null or short row skipped; non-week names skipped;
  a duplicate week replaces and both count in `found`.
- **Number suffix:** a key of text `Scheduled_Arena_Hunt` with number 182 parses as week 181; text
  `Scheduled_Arena_Hunt_042` with number 0 as week 42; text `Scheduled_Arena_Hunt` with number 0 is skipped.
- **Rotating plan** at 2026-11-12T00:00:00Z with 10 080 minutes (week 72, R1's vector) and all 181 rows: row 72 starts
  at `bucket_start`, row 73 one rotation later, row 71 180 rotations later; every end is start + rotation;
  `patchedRows=181/181`; `horizonEndTicks = bucket_start + 181 × rotation`; the exact log line. With 30 minutes (week
  146). With rows 1 to 10 only: `patchedRows=10/10`.
- **Pinned plan:** week 5 present: start `now − 36 000 000 000`, end start + 7 days, only that row written, the exact
  line; week 5 absent: no write, the "missing" line.
- **Two arena tables** in one array: each planned and logged on its own.
- **Write protection test:** each accepted protection, `PAGE_READONLY`, `PAGE_EXECUTE_READ`, a guard page, a range
  crossing a region end.
- **Buff-choice state:** `(7, 0)` queues; `(7, 1)`, `(6, 0)`, `(8, 0)` do not; params shorter than 2 bytes do not; a
  second note before a take replaces the first; `take` clears; after `mark_dispatched(c, r)` a note of `(c, r)` is
  ignored and `(c, r + 1)` queues; `i32::MIN` round when the state component is null or short; the queued, skipped,
  not-found, dispatching and dispatched texts, with `null` and decimal pointers.
- **Cadence:** the body runs on calls 64, 128, ... and not on 1 to 63.
- **Reflection** (with R17's kit): finds the function on a class two supers up; ignores a same-named child whose outer
  is another class; stops at the bounds.
- **Registry:** with `LANDED` holding `R18`, `owns` is true for `ESC-1`, `SP-2@R18`, `SP-3@R18`; the offset table check
  (R0 6.1) passes with the new `TARGETS` line.

In the DLL crate (Windows runner): `ffi::memory::write` on a fresh read-write page, a read-only page and a guard page.

**Never in CI:** the hook against the real executable, the schedule in the running game, an Escalation.

### 9.2 The maintainer's play-test (surface 8, R18's row)

"Trials ungreyed and launchable; an escalation's relic choices." Collected for the one hands-on pass at the end
(rulings, "Order, amended"); nothing waits on it. Build both DLLs with R18 in `LANDED` and run them as in R0 11.1
step 1.

1. **Trials board.** Let the Director start Ramsgate. In that server's `rustic_isles_dll_port<N>.log`: `[MitRuntime]
   install ESC-1 ok`; one `[TrialsSchedule] Initialize: rotating intervalMinutes=10080
   currentRow='Scheduled_Arena_Hunt_<NNN>' patchedRows=181/181 ...` line per arena table; the AGPL's
   `[MitSeam] ... <n> hooks left to it` counts ESC-1. No AGPL `[TrialsSchedule]` line follows. Log in and open the Trials board: it is not greyed, and shows a Behemoth.
2. **The week.** `currentRow` is the week R1 9's formula gives for the server's start time (the Director's
   `current_trials_week` computes the same); step 3 checks that the launched trial pins that same row.
3. **Launch a trial.** Start it from the board. The arena server's log shows `[TrialsSchedule] Initialize:
   launch-pinned row='Scheduled_Arena_Hunt_<NNN>' duration=7d table=...` with the same `<NNN>`. The Behemoth is the one
   the board showed. Finish (or abandon) and return to Ramsgate.
4. **Short rotation (optional).** Set `TRIALS_ROTATION_MINUTES=30` for Ramsgate's server, restart it, check the line
   says `intervalMinutes=30`; restart after the next half-hour boundary and see `currentRow` move by one.
5. **Escalation, one player.** Start an Escalation. When round 1 ends, the hunt server's log shows
   `[EscalationBuffChoiceSpawn] queued mode=... round=0 trigger=EventEnding(7) entry`, then within about a
   second `dispatching` and `dispatched` for the same mode and round. Your own relic pickups appear; pick a relic; the next round starts. Each later round
   repeats the three lines once, with the next round number.
6. **Escalation, two players.** Both players get their own pickups after each round and each can choose; the round
   advances. (The networking fixes this also needs are R12's and are still in the AGPL runtime; a stall here with
   `[PCChannelBind]` lines is R12's, not R18's.)
7. **A/B.** Create `ESCALATION_NO_BUFF_CHOICE_SPAWN.flag` next to the game executable and repeat step 5: no `queued`
   line, and only the level-placed crystal appears. Delete the flag.
8. **Regression without the MIT DLL.** Remove `rustic-isles-runtime.dll` and repeat steps 1 and 5 quickly: the AGPL's
   `[TrialsSchedule]` and `[EscalationBuffChoiceSpawn]` lines appear instead, with the same outcomes.

Only CI passed until these steps are done.

---

## 10. Scheduled events later (the maintainer's wish, 2026-11-03)

**(call) R18 leaves room and settles nothing.** The maintainer wants scheduled events (Trials and shop rotations)
eventually. What R18 fixes is narrow: which arena row is current is a pure function `(now, minutes, pinned week) ->
row windows` in the core crate, and the DLL only applies the writes before `Initialize`. A later schedule source (for
example a rotation list from `host.toml`, passed in the environment as RT-11 does for other settings, or chosen by the
Director and pinned per launch as `RUSTIC_ISLES_TRIALS_WEEK` already is) replaces that one function and keeps the
hook, the row reading and the writer. Other schedule tables (seasonal events) would be a second row filter beside the
arena one. Shop rotations are not the runtime's at all: the stores are built by the backend (`rustic-isles` store
captures), so they need no runtime change. Two limits a design must respect: the windows are fixed once per process
(the scheduler copies them once, so a change needs a server restart or a new launch), and the Director must compute the
same answer for the hunts it launches (R1 9's shared vectors).

---

## 11. Calls, departures and ambiguities

Calls:

1. The surface's "relic choice" is diagnostics only: not ported (RT-3), no `SP-1@R18` (section 5).
2. `GAUNTLET_CENSUS_OFF` moves to R19 (not ported); `ESCALATION_NO_BUFF_CHOICE_SPAWN` kept (RT-14 for R18).
3. ESC-1's wrapper passes the original's return value through as a `u64`.
4. ESC-1 stays found by RVA; a prologue check may be added from the executable's bytes.
5. A checked data writer (`ffi::memory::write`) with a pure protection test, shared with R15.
6. The row map's bounds (4096 elements) and readability checks.
7. Handler orders `1800` on SP-2 and SP-3.
8. The AGPL's log texts kept exactly, decimal pointers included.
9. R18's own 64-tick cadence counter.
10. No seam patch 0049: 0041 covers ESC-1, `SP-2@R18` and `SP-3@R18`.
11. Scheduled events: room left, nothing settled (section 10).

Departures:

1. The table array's data is checked readable for every entry, not only the first.
2. Only applied writes count in `patchedRows`.

Ambiguities:

- The real return type of `FGameplayScheduler::Initialize` is not known; call 3 makes it irrelevant.
- The name of the arena schedule table is not recorded in the AGPL (it matches any table whose name contains `arena` or
  `Arena`); play-test step 1 logs it.

Settled here: **RT-17** for R18 (no order rule, section 7), **RT-18** for R18 (4.2), **RT-14** for R18 (call 2).
RT-12, RT-13, RT-15 and RT-16 are not R18's.

No open question for the maintainer.

## 12. Size

Rust without tests: about 300 lines (core: trials reading and plan 130, buff-choice state 50, protection test 15;
DLL: registration and glue 50, detour and calls 40, writer 15), plus R17's reflection kit if R18 lands first (about
120).
Tests about 350 lines.
