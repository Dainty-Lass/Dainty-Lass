<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R16, travel (RD27, plan item 11)

R16 is the travel slice of the MIT game runtime (`spec-runtime-surface.md` section 8, row R16; rulings RT-1 to RT-18 in
`rulings-2026-11-13.md`). It ports what keeps the client from crashing when it travels back to a map it has already
been on (hunt → Ramsgate, Training Grounds → Ramsgate): the **leftover-world release** before `UEngine::LoadMap`'s
purge (TR-1) and on every world change (the 250 ms watcher on SP-1), plus the opt-in **leftover-world guard** that
fails such a travel instead of crashing. Everything else the AGPL runtime does around travel only logs, and RT-3
drops it (section 7).

An implementer builds from this page, `spec-runtime-r0.md`, `spec-runtime-r1.md` and the surface page alone, and never
reads `restricted/`. What crossed is facts: RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7, CL 647472), prologue
bytes, engine layout offsets and flag values, game object names, flag file names and behaviour. The AGPL side needs no
patch beyond seam 0041 (section 2).

Conventions are the surface page's: an RVA is `+0x0XXXXXXX` from the executable's image base; **C** client, **S**
server. **(call)** marks a decision this page makes, **(departure)** a deliberate difference from the AGPL runtime,
**(ambiguity)** something the source did not settle; all are collected in section 10.

---

## 1. Scope

| R16 owns | Section |
| --- | --- |
| Hook TR-1 on `UEngine::LoadMap` (C), through R0's broker | 4 |
| The leftover-world pass: find pending-kill worlds that outlived their map, clear their stuck `Async` keep flags | 3 |
| The world-change watcher on SP-1 (C), feature id `SP-1@R16` | 5 |
| The opt-in leftover-world guard (`ENABLE_ZOMBIE_LOADMAP_GUARD.flag`) | 4.3 |
| R16's flags (RT-14 for R16), RT-17 and RT-18 for its ids | 6, 4.4, 5.2 |

R16 is client-only. A server never loads a map it left through this path in a way the crash needs (a hunt server
loads one map and ends), and the AGPL never installs TR-1 or the watcher on a server.

Not R16: the vectored handler's one-time dump at the crash site (EXIT-12, R3; logging-only, section 7), possession
bookkeeping on world change (R14), the server's map URL (SRV-1, R9).

### 1.1 Why it exists (the game behaviour)

The crash: on the second arrival at a map in one session (typically hunt → Ramsgate after Ramsgate was left once),
`UEngine::LoadMap` faults at `+0x04272D28` reading `World()->PersistentLevel->GetOutermost()` with `PersistentLevel`
null (the shipping build drops the engine's check). It happens about 0.2 s after the connect, far too soon for a disk
load: `LoadMap`'s package lookup found the map's **previous** `UWorld` still in memory and reused it, though garbage
collection had already destroyed its level.

The cause, as observed on 1.14.7: when the client leaves a map, one object inside the old world, the input component
of class `default_input_bpc_C`, still carries the `FUObjectItem` internal flag **`Async` (`0x04000000`)**, which
1.14.7's garbage collector treats as a keep flag (its mark phase, `+0x027D9580` and three siblings, passes fast keep
flags `0x0E000000`), so it keeps the object even when it is pending-kill. Collection never eliminates outer references,
so that one object keeps its chain alive: component → its `PlayerController` → `PersistentLevel` → the `UWorld` → the
map's package. The world survives as a pending-kill leftover with a few objects inside.

The fix point: `LoadMap` runs a full-purge `CollectGarbage` (`+0x027DFE10`, called at `+0x042721A6` with keep flags 0)
after it detaches the old world and before its package lookup. If the stale `Async` bit is cleared before that purge,
the leftover is freed and `LoadMap` loads the map fresh. R16 clears it at the top of `LoadMap` (TR-1) and, earlier,
on each world change (the watcher), so the next ordinary collection frees the leftover even sooner.

---

## 2. Ownership and the AGPL side

With `"R16"` in `LANDED`, `rir_owns` answers 1 for `TR-1` and `SP-1@R16` (both already in R0's registry with slice
R16; R16 adds no id). On a client with R0, R1 and R16 landed the status line reads `owns=5 ids`.

Seam patch 0041 already covers R16; **no further AGPL patch is needed (0048 is not used)**:

- **TR-1** is an ordinary hook id. When the MIT owns it, the AGPL's `MH_CreateHook` on `+0x04271370` makes no wrapper;
  its "original" pointer is set to `rir_original(target)` and its enable call succeeds as a no-op. So the AGPL's
  `LoadMap` detour never runs: its enter/exit logs, its census at entry, its guard and its travel-history hook-up all
  stop with it.
- **`SP-1@R16`** gates the AGPL's 250 ms world watcher inside its client `ProcessEvent` wrapper. With R16 landed it
  does nothing, so no AGPL code clears any flag.
- **What stays AGPL and runs anyway** (logging-only, no id, RT-3): the travel-history recorder around the original
  `ProcessEvent` (it reads a world context only the AGPL `LoadMap` detour provided, so with TR-1 owned it never
  reports), the client network/travel failure log lines, and the vectored handler's crash-site dump. None writes game
  memory and none reads state R16 sets. R0 section 7.5's cross-slice table gains no row.
- **With the MIT DLL absent** the AGPL runtime does everything as today.

TR-1 and `SP-1@R16` move together (they share the released-worlds ring, 3.4): one slice, one `LANDED` entry, so they
can never be split between the DLLs.

---

## 3. The leftover-world pass

One routine, run from TR-1 (4.2) and the watcher (5.1), always on the game thread, never two at once (a `try_lock`;
if busy, skip). It takes a **reason** text: `loadmap-enter`, `world-change`, `arrival+10s` or `arrival+60s`.

### 3.1 Engine facts

| Name | Value | Notes |
| --- | --- | --- |
| `GWorld` | data `+0x06FC7F58` | already in R0's table; a `UWorld*`, may be null |
| Object array | R0 6.2 | chunked, 65 536 items per chunk, item `0x18` bytes, object pointer at item `+0x00` |
| `FUObjectItem` flags | `i32` at item `+0x08` | the internal object flags |
| `RootSet` | `0x40000000` | item flag |
| `PendingKill` | `0x20000000` | item flag |
| `Unreachable` | `0x10000000` | item flag (not used by the pass; listed for completeness) |
| `AsyncLoading` | `0x08000000` | item flag |
| `Async` | `0x04000000` | item flag: the stuck keep flag |
| `RF_ClassDefaultObject` | `0x00000010` | `UObject` object flags (`u32` at `+0x08`) |
| `UObject` class, outer | `+0x10`, `+0x20` | R0 6.2 |
| `UStruct::SuperStruct` | pointer at `+0x40` | a class's parent class (UE 4.26 layout; **(ambiguity)** confirm against the maintainer's dump, section 10) |
| The world class | the object whose full name is `Class Engine.World` | found by name once (3.2) |
| `UWorld::PersistentLevel` | pointer at `+0x30` | not read by the MIT pass (the AGPL logged it); listed so the guard's log may show it |

`offsets.rs` lines (shape as R0 6.1), with a comment naming R16:

- `TARGETS`: `{ id: "TR-1", rva: 0x0427_1370, found_by: RvaAndBytes(&LOADMAP_PROLOGUE), what: "UEngine::LoadMap" }`
  (bytes in 4.1).
- `layout`: `OBJECT_ITEM_FLAGS = 0x08`, `ITEM_ROOT_SET = 0x4000_0000`, `ITEM_PENDING_KILL = 0x2000_0000`,
  `ITEM_ASYNC_LOADING = 0x0800_0000`, `ITEM_ASYNC = 0x0400_0000`, `RF_CLASS_DEFAULT_OBJECT = 0x10`,
  `USTRUCT_SUPER = 0x40`, `WORLD_CONTEXT_WORLD = 0x280`, `WORLD_CONTEXT_SIZE = 0x288`, `FURL_MAP = 0x28`,
  `FURL_SIZE = 0x38`, `FSTRING_MAP_MAX_NUM = 1024`.

### 3.2 Finding the leftover worlds

1. **The world class.** On first use, walk the object array once for the object whose R0 full name is exactly
   `Class Engine.World`; keep its address for the process (a class is never collected). Not found, or names off (R0's
   self-check failed): the pass does nothing this time and tries again next time. **(call)** The AGPL compares classes
   with the SDK's `IsA`; the MIT finds the class by name, which needs no dump.
2. **Is a world.** An object is a world when its class pointer is the world class, or the class's `SuperStruct` chain
   (at most 16 steps, every read checked) reaches it. Cache the answer per class pointer for the length of one pass.
3. **Leftovers.** Walk every item of the object array (3.5 says how). An item is a leftover when: its object is
   non-null; its item flags have `PendingKill`; the object is a world; its object flags lack `RF_ClassDefaultObject`;
   and it is not the current `GWorld` (read once at the start of the pass). Stop collecting at **64**.

Only pending-kill items have their object read, so a pass reads object memory for few items. **(call)** The AGPL
found every world first and then filtered for pending-kill; the result is the same set.

No leftover: the pass ends here (nothing is logged).

### 3.3 Releasing their stuck keep flags

Read `KEEP_STUCK_ASYNC_FLAGS.flag` once (the first pass that finds a leftover) and log once, either
`[Travel] leftover-world release on` or
`[Travel] leftover-world release off (KEEP_STUCK_ASYNC_FLAGS.flag; report only)`.

Walk the object array again. An item is **eligible** when its flags have `PendingKill` and `Async` and have neither
`RootSet` nor `AsyncLoading` (that is, `flags & 0x6C000000 == 0x24000000`), and the object or one of its outers,
checking the object itself and then at most 16 outers (17 checks), is one of the leftover worlds. Nothing else is ever
touched: a live object, a rooted one, one still loading, or one outside the leftovers.

For each eligible item, unless report-only: clear **only** the `Async` bit, with one atomic AND on the 32-bit flags word
(`fetch_and(!0x04000000)`; the engine's own flag writes are atomic too). Record the leftover world it lies in.

If at least one item was eligible, log one line, `[Travel] <reason>: released the stuck Async keep flag on <n>
object(s) in <w> leftover world(s): ` followed by `<obj>(<class>) in <world>` entries joined by `, `,

with `released` replaced by `found (not released)` in report-only mode, listing at most 8 objects by their short names
and their class's short name. Expected on the crash path: `default_input_bpc_C` objects. Nothing is logged when no item
was eligible. **(departure)** The AGPL also counted keep-flag holders it left alone and logged when that count changed;
that count is diagnostic and dropped.

### 3.4 The released ring and the survivor warning

- **Released ring.** Each leftover world in which the pass cleared at least one bit (not in report-only mode) is
  remembered in a ring of **16** world addresses (oldest overwritten), once per world. The guard (4.3) reads it.
- **Survivor warning.** On a `world-change` pass only, before releasing: each leftover that is already in the released
  ring was released before and still survived the purge, so something else holds it. Log once per world (a second
  ring of 16): `[Travel] leftover world <address> <name> survived the purge after its keep flag was released: something
  else holds it`. It is one line from data the pass already has, kept so the play-test can tell a second holder from
  the known one. **(call)**

An address can be reused by a later world; the rings then treat a new world as released, which only skips a guard
refusal and a warning. Harmless, and the AGPL has the same rule.

### 3.5 Walking the object array without faulting

R0 5 forbids unchecked reads, and R0's `object_at` checks two pointers per index, which is too slow for walks of
several hundred thousand items several times per travel. The pass walks **by chunk**:

- Read the count, chunk count and chunk table once (R0's bounds: count `0..=4 000 000`, chunk index below the chunk
  count).
- For each chunk: read its pointer; check the whole item span of that chunk (`items_in_chunk × 0x18` bytes) readable
  once; then read each item's object pointer and flags from that span without a further check.
- Object, class and outer reads go through a **region cache** (call): a `Memory` implementation that remembers the
  last 8 regions `VirtualQuery` returned in this pass and answers `readable` from them when a range falls inside one
  (still under R0's region rule, `region_allows`). It lives for one pass: no collection can run during a pass, since
  the pass runs on the game thread inside `LoadMap` or `ProcessEvent` and calls no game code.
- The count is read once; objects added during the pass (by the loading thread) are not seen, which is harmless.

The walk and both item predicates are pure (core crate, over R0's `Memory` trait and `BufferMemory`); the atomic clear
is the only write and lives in `ffi`.

---

## 4. TR-1: `UEngine::LoadMap` (C)

### 4.1 Facts

| Name | Value |
| --- | --- |
| Target | `UEngine::LoadMap`, RVA `+0x04271370`, found by RVA + bytes: `verify_entry` (R0 5.4, `function_entry == rva`) and these 31 bytes |
| Prologue | `48 89 5C 24 20 55 56 57 41 54 41 55 41 56 41 57 48 8D AC 24 00 FE FF FF 48 81 EC 00 03 00 00` |
| Signature | `(engine, world_context: *mut FWorldContext, url: *const FURL, pending: *mut UPendingNetGame, error: *mut FString) -> u8` (a C++ `bool`); Win64, the fifth argument on the stack. From its only engine caller, the game's own `LoadMap` wrapper at `+0x01909C10` (call at `+0x01909EA6`) |
| `FWorldContext` | readable for `0x288`; `World()` = pointer at `+0x280` (the world being left, at entry) |
| `FURL` | readable for `0x38`; the map path is the `FString` at `+0x28` (`{ data: *u16, num: i32, max: i32 }`, `num` counts the NUL). A path, never options: no secret is in it |
| Crash site | `+0x04272D28`, inside this function (1.1) |

### 4.2 The wrapper

One MIT wrapper through `broker::hook(base + 0x04271370, wrapper, order = 0, &NEXT)`. Each call (always the game
thread):

1. If detaching: return `next(...)` with the same five arguments.
2. `write_status_once()`.
3. Record this thread as the game thread if it differs from the one recorded (5.1) **(call)**: `LoadMap` only runs on
   the game thread, so this is the authority.
4. Inside `catch_unwind`: run the leftover-world pass with reason `loadmap-enter` (3); then the guard (4.3). The pass
   runs whether or not the guard is on, and **before** it, so a leftover that only had the known holder is released
   and left for `LoadMap`'s own purge.
5. If the guard refused: return `0` (false) **without calling `next`** (an absorb, R0 7.2). Otherwise return
   `next(...)`'s value unchanged.

A panic in step 4 logs `[Panic] TR-1 ...` once (R0 2) and goes to step 5 as "not refused". Nothing is read or logged
after `next` returns. **(departure)** The AGPL logged the destination, the old world and every world at entry and the
result and duration at exit; those lines are a probe and are dropped (RT-3).

### 4.3 The leftover-world guard (opt-in)

`ENABLE_ZOMBIE_LOADMAP_GUARD.flag`, read once at the first TR-1 call, which logs once:
`[Travel] leftover-world guard on (ENABLE_ZOMBIE_LOADMAP_GUARD.flag)` or `[Travel] leftover-world guard off`.

When on, and the map path (4.1) is readable and non-empty:

1. **The map's short name.** The path's text after its last `/`, then before its first `.` (so
   `/Game/Maps/ramsgate/ramsgate_01_persistent` and `.../ramsgate_01_persistent.ramsgate_01_persistent` both give
   `ramsgate_01_persistent`). The path is read as UTF-16, `num` 2 to 1024, every unit above 127 turned into `?`.
2. **Find a leftover by that name.** Using the leftovers the `loadmap-enter` pass just found (3.2; if the pass found
   none, there are none): the first whose short object name (R0) equals the map's short name exactly, that is not the
   world context's world at entry, and that is **not** in the released ring (3.4).
3. Found: log `[Travel] REFUSED map=<path>: its previous world <address> is a pending-kill leftover; failing this travel
   instead of reusing it (ENABLE_ZOMBIE_LOADMAP_GUARD.flag)` and refuse (4.2 step 5). The engine handles a `false`
   `LoadMap` as a failed load (`UPendingNetGame`'s load completion then browses to the default map): the player lands
   on the login screen instead of crashing.

Only pending-kill worlds count, never a world that is merely loading. A leftover in the released ring is left to
`LoadMap`'s purge, which now frees it. It stays **opt-in** as in the AGPL: if another holder kept a leftover alive
across the login screen, every later travel to that map would be refused, a login loop worse than the crash.
**(departure, small)** The AGPL rescanned every world for the guard; the MIT reuses the pass's leftovers, the same set
taken a moment earlier on the same thread.

### 4.4 Install (RT-17 for R16)

Client only, in Init after the dispatch chain: `SP-1`, then `TR-1` (the surface's 3.11 client order puts TR-1 right
after SP-1 and UI-13; R16 has no dependency on UI-13 or any other id). **Settled (RT-17):** TR-1 depends on nothing
installed before it; any position after `MH_Initialize` is equivalent.

- `NO_LOADMAP_PROBE.flag` present at Init: not installed; status
  `[MitRuntime] install TR-1 off (NO_LOADMAP_PROBE.flag)`.
- `verify_entry` false: not installed; status `[MitRuntime] install TR-1 prologue mismatch`. The game runs unhooked
  (fails closed: a wrong address is never detoured).
- Otherwise the broker's result: `ok` or MinHook's status name.

**Cross-slice note.** R0's status list holds a MinHook status number per install; TR-1 is the first install with a
reason that is not one (`prologue mismatch`, the flag). The status entry's error type widens to a reason text (a
one-line change in R0's `lifecycle.rs`, the same R0 file a slice already edits for its install line). Whichever
RVA-and-bytes slice lands first makes that change; the others reuse it.

TR-1 is owned even when the flag keeps it uninstalled: the AGPL then installs nothing either, exactly as the AGPL alone
did under the same flag.

---

## 5. The world-change watcher (`SP-1@R16`, C)

### 5.1 Behaviour

A before handler on SP-1, `order = 0`, registered by R16 in Init before R0's `handlers::freeze()`. Per call:

1. Run only on the game thread: the first SP-1 call records its thread (the game thread, as the AGPL decides it); TR-1
   corrects it if it ever differs (4.2 step 3). On any other thread: return `Continue`.
2. At most every **250 ms** (`GetTickCount64`; the first call always passes): read `GWorld`.
3. A state machine of three stages, with the world last seen and the time it changed:
   - `GWorld` differs from the world last seen (null counts as a world value: to or from null is a change): remember it
     and the time, set the stage to "+10 s pending", run the pass with reason `world-change`.
   - Stage "+10 s pending" and 10 000 ms since the change: stage "+60 s pending", run the pass with `arrival+10s`.
   - Stage "+60 s pending" and 60 000 ms since the change: stage idle, run the pass with `arrival+60s`.
4. Always return `Continue`: the handler never absorbs and never touches the call's arguments.

**(call)** The +10 s and +60 s passes are kept: in the AGPL they released too (not only logged), and a pass that finds
no leftover costs one walk of the item flags. Their census and live-`Async` listings are dropped (section 7).

### 5.2 Matcher (RT-18 for R16)

The handler runs for **every** call: its matcher is `[(Prefix, "")]`, which matches any UFunction with a name and no
null function (R0 8.3). With names off for the process (R0's self-check failed) nothing matches, so the watcher is off,
and the pass could not find the world class anyway. No name is matched by `Contains`.

**Position on SP-1.** The AGPL ran its watcher at the top of its client wrapper, before its own depth guard; the MIT
handler runs inside R0's chain, after R0's depth guard, before every AGPL block. A call absorbed by the depth guard
(nesting beyond 32) does not run the watcher; the next normal call does, so at most one 250 ms slot is late. Moving it
changes nothing else: it reads `GWorld` and the object array only, and no other handler reads or writes what it does.

---

## 6. Flag files (RT-14 for R16)

| Flag | Kept? | Meaning in the MIT runtime |
| --- | --- | --- |
| `NO_LOADMAP_PROBE` | kept **(call)** | read once at Init: TR-1 is not installed (no `loadmap-enter` pass, no guard). The name stays so an existing file keeps its effect |
| `ENABLE_ZOMBIE_LOADMAP_GUARD` | kept | read once at the first TR-1 call: the guard of 4.3 |
| `KEEP_STUCK_ASYNC_FLAGS` | kept | read once at the first pass that finds a leftover: report only, nothing is cleared (the revert switch for the fix) |
| `NO_WORLD_CENSUS` | **dropped (call, departure)** | it switched off the AGPL's census and with it the world-change releases. The census is gone; `KEEP_STUCK_ASYNC_FLAGS` is the one switch for releasing. A stale file is ignored, so a client that had it now also releases on world change (the `loadmap-enter` release never depended on it) |
| `ZOMBIE_REF_SCAN` | dropped | it enabled a sweep for references to a leftover (logging-only, RT-3); the code goes |
| `TRAVEL_CAUSE_OFF` | dropped | it disabled the travel-history recorder (logging-only, RT-3); the code goes |

Every kept flag is read through R0's flag cache with a slice-local "read once" static (R0 4.2).

---

## 7. Not ported (RT-3)

Each of these only logs; none writes game memory or changes control flow. They stay AGPL until R21 where they are not
gated, and go with it. A later slice ports one only for a live bug.

| AGPL behaviour | Where | Note |
| --- | --- | --- |
| `[LoadMap] enter ... / exit ok= ms= ...` lines | TR-1 | gone with TR-1 ownership |
| `[WorldCensus]` lines: every `UWorld` with package, level, flags and objects inside, at each world change, +10 s, +60 s, and at `LoadMap` entry | TR-1, watcher | the pass keeps only the leftover set (3.2) |
| `[ZombieWorld]` listing of the objects inside each leftover, once per world | TR-1, watcher | the release line (3.3) names the objects it clears |
| The reference sweep for holders of a leftover (`ZOMBIE_REF_SCAN.flag`) | watcher | read raw heap words as pointer arrays; raised about 190 first-chance faults per world in the AGPL; refuted as a tool |
| `[AsyncFlagCensus]`: live objects of the current world carrying `Async`/`AsyncLoading` at +10 s and +60 s | watcher | |
| `[TravelCause]`: a 2048-entry history of client `ProcessEvent` calls, the moment `FWorldContext`'s travel URL (`+0xB8` `FString`, `+0xC8` travel type) becomes set, and a dump on a return to `Map_LoginMenu` | SP-1 around the original, TR-1 | ungated in the AGPL; with TR-1 owned it never learns the world context and reports nothing. Note for a future port: it redacted every URL option value (a travel URL carries the session's encryption token) |
| `[ClientNetworkFailure #n]`, `[ClientTravelFailure #n]`: the first byte(s) of `HandleNetworkError` / `HandleTravelError` params | SP-1 | ungated; stays AGPL until R21 |
| `[TravelCrash]`: the vectored handler's one-time dump of the world at the crash site, when an access violation's RIP is `+0x0285E6F0` (the `GetOutermost` helper; `r12` still holds `LoadMap`'s world context) | EXIT-12 (R3) | **R3's spec writer: do not port it.** It recovers nothing |

---

## 8. Where the code goes

| File | Holds |
| --- | --- |
| `crates/rustic-isles-runtime-core/src/travel.rs` (new) | pure: the item predicates (leftover, eligible), the chunked walk over `Memory` returning leftovers and the flags addresses to clear, the outer-chain test (17 checks), the world-class test with its per-pass cache, the released and warned rings, the watcher's state machine against a clock, the map short-name rule, the guard decision, every log line text |
| `crates/rustic-isles-runtime/src/ffi/travel.rs` (new) | the TR-1 wrapper and install, the SP-1 handler and its registration, `GWorld`, world-context and URL reads, the per-pass region-cached `Memory`, the atomic clear, the game-thread record |
| `crates/rustic-isles-runtime-core/src/offsets.rs` | R16's lines of 3.1 |
| `crates/rustic-isles-runtime-core/src/registry.rs` | `LANDED` gains `"R16"`; R0's test asserting `owns` only for SP-1/2/3 follows `LANDED` |
| one-line edits | `pub mod travel;` in the core `lib.rs` and in `ffi/mod.rs`; in `lifecycle.rs`, R16's register call before `handlers::freeze()` and the client's `TR-1` install entry after `SP-1` (and the status type of 4.4 if no earlier slice changed it) |

Nothing is added to `[workspace.dependencies]`.

---

## 9. Tests

### 9.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`), on a `BufferMemory` holding a synthetic object array (two chunks,
items with chosen flags, objects with class and outer pointers, a world class with a subclass):

- Leftovers: a pending-kill world that is not `GWorld` is one; the current `GWorld`, a world without `PendingKill`, a
  class default object and a non-world are not; a world of a subclass is one; a class chain longer than 16 is not;
  the 65th leftover is not collected.
- Eligibility: each of the four bits flips the answer as 3.3 says; an object 17 outers deep is not inside, 16 is; an
  object inside a non-leftover world is not eligible.
- The walk: an unreadable chunk is skipped and the rest are walked; a count above 4 000 000 walks nothing; a null
  chunk pointer is skipped.
- Report-only returns the same set and no address to clear.
- Rings: the 17th released world overwrites the first; a world is recorded once; the survivor warning fires once per
  world and only on `world-change`.
- The watcher: first call runs; nothing before 250 ms; a change runs `world-change`; +10 000 ms runs `arrival+10s`
  once; +60 000 ms `arrival+60s` once, then idle; a change during "+60 s pending" restarts at +10 s; null to a world
  and a world to null both count.
- The map short name: the two examples of 4.3, a path without `/`, without `.`, empty, `num` 0, 1 and 1025, a unit
  above 127.
- The guard: refuses on a leftover of the same name; not on a released one, not on the context's own world, not on a
  different name, not when off.
- Log texts: each line of sections 3, 4 and 6 for fixed inputs; at most 8 objects listed.
- Offsets: TR-1's `TARGETS` line exists once with a 31-byte prologue (R0's table test, now with R16 landed).
- Registry: with `LANDED` holding R16, `owns("TR-1")` and `owns("SP-1@R16")` are true.

DLL (`cargo test -p rustic-isles-runtime`, Windows):

- The atomic clear on a test buffer: only bit `0x04000000` changes, other bits kept.
- The region-cached `Memory`: a range inside a cached region answers without a new query; a range across two regions
  is refused; a `PAGE_NOACCESS` page is refused even when adjacent to a cached one.
- The TR-1 wrapper over a test function with `LoadMap`'s five-argument signature through the broker: arguments and
  return value pass through; with a fake refusal it returns 0 without calling the function.

### 9.2 Never in CI

`LoadMap` on the real executable, the prologue match, the world class's name, the flag values' effect on 1.14.7's
collector, and the crash itself. Only the play-test shows them.

### 9.3 The maintainer's play-test (surface section 8, R16 row; collected for the end, rulings "Order, amended")

"Hunt → Ramsgate three times, no crash."

1. Client with both DLLs (R16 landed in the MIT one, seam 0041 in the AGPL one). In `rustic_isles_dll_port0.log`:
   `[MitRuntime] ... role=client ... owns=<n> ids` with `n` counting TR-1 and `SP-1@R16`, `[MitRuntime] install TR-1
   ok`, and the AGPL's `[MitSeam] ... present (seam 1): <k> hooks left to it` with `k` at least 1 (TR-1).
2. Log in, enter Ramsgate, go to the Training Grounds, come back to Ramsgate. Then three times: start a hunt, finish
   or leave it, return to Ramsgate. No crash, no fault at `+0x04272D28`.
3. In the log, after a return: `[Travel] leftover-world release on`, and at least once a line
   `[Travel] world-change: released ...` or `[Travel] loadmap-enter: released ...` naming `default_input_bpc_C`
   objects. No `survived the purge` line expected; if one appears, note it (a second holder exists).
4. No AGPL `[LoadMap]` or `[WorldCensus]` lines appear (the AGPL's TR-1 and watcher are off).
5. **Revert check.** Create `KEEP_STUCK_ASYNC_FLAGS.flag` next to the game executable, restart, repeat step 2: the log
   says `report only` and `found (not released)`; the crash on return may now reproduce (it is the unfixed game). Delete
   the flag.
6. **Guard check (optional).** With `KEEP_STUCK_ASYNC_FLAGS.flag` and `ENABLE_ZOMBIE_LOADMAP_GUARD.flag` both present,
   repeat step 2: instead of a crash, a `[Travel] REFUSED map=...` line and the client back on the login screen.
   Delete both flags.
7. **Regression without the MIT DLL.** Remove `rustic-isles-runtime.dll` and repeat step 2 once: the AGPL's own
   `[LoadMap]` and `[WorldCensus]` lines return, and no crash (the AGPL fix as today).

Only CI passed until these steps are done.

---

## 10. Calls, departures, ambiguities and settled questions

Calls:

1. The world class is found by its full name `Class Engine.World`, with a `SuperStruct` walk for subclasses (3.2).
2. Leftovers are found by filtering pending-kill items first (3.2); same set as the AGPL.
3. The pass walks the object array chunk by chunk with one check per chunk, and reads objects through a per-pass
   region cache (3.5).
4. The survivor warning is kept as one line (3.4).
5. TR-1's thread is the authority for the game thread (4.2).
6. The +10 s and +60 s passes are kept (5.1).
7. `NO_LOADMAP_PROBE` is kept as TR-1's off switch under its old name (6).
8. `NO_WORLD_CENSUS` is dropped (6).
9. R0's install status widens to a reason text, by whichever RVA-and-bytes slice lands first (4.4).
10. No AGPL patch: 0048 is not used (2).

Departures:

- The `[LoadMap]`, `[WorldCensus]`, `[ZombieWorld]`, `[AsyncFlagCensus]` and `[TravelCause]` lines are not written
  (RT-3); the MIT writes `[Travel]` lines only where it acts (3.3, 3.4, 4.3) or says once whether a switch is on.
- The left-alone holder count is dropped (3.3).
- The guard reuses the pass's leftovers instead of rescanning (4.3).
- With a stale `NO_WORLD_CENSUS.flag`, world-change releases now run (6).

Ambiguities:

- `UStruct::SuperStruct` at `+0x40` is the UE 4.26 layout (it is consistent with `UStruct`'s properties size at
  `+0x58`, which the AGPL reads). The maintainer's Dumper-7 dump of this build (`CoreUObject` classes, `UStruct`)
  confirms it; if it differs, change the one `layout` line. A wrong offset fails safe: a subclass is then not
  recognised, and `UWorld` itself still is by its exact class.

Settled for R16 (RT-12 to RT-18):

- **RT-14:** R16's flags are section 6.
- **RT-17:** TR-1 has no install-order dependency (4.4).
- **RT-18:** the watcher matches every named call with `Prefix ""` (5.2); TR-1 is a detour and matches no name.
- RT-12, RT-13, RT-15 and RT-16 do not touch R16.

No open question remains for the maintainer.

---

## 11. Size

Today's C++ the slice replaces is about 1 000 lines (the census, the leftover report and reference sweep, the release,
the `LoadMap` detour and guard, the travel-history recorder). The MIT rewrite without the diagnostics: core about 300
lines (walk 90, predicates 50, rings and watcher 80, guard and texts 80), DLL about 180 (wrapper and install 70,
handler 40, region cache 50, clear and reads 20). Tests about 350.
