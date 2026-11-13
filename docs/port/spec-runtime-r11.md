<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R11, net driver safety (RD27, plan item 11)

R11 is the net driver slice of the MIT game runtime (`spec-runtime-surface.md` section 3.8 and section 8's R11 row,
approved 2026-11-13 with rulings RT-1 to RT-18 in `rulings-2026-11-13.md`). It owns the eight ids NETD-1 to NETD-8 and
the feature id `SP-3@R11`. On a server it keeps the game's net driver usable: it checks the client connection list
before every walk of it, keeps the hub's own net driver bound to its world, drives that driver once a frame (the
engine never does on this build), makes the engine build the game's replication graph, and makes the engine treat the
process as a dedicated server. On both roles it logs the first connection closes. It also carries one crash guard on
the spline direction query.

An implementer builds from this page, `spec-runtime-r0.md`, `spec-runtime-r1.md`, the specs of the slices it names in
section 3, and the surface page alone, and never reads `restricted/`. What crossed is facts: RVAs into
`Dauntless-Win64-Shipping.exe` (1.14.7, CL 647472), instruction bytes and `.pdata` entries (each checked against the
maintainer's copy of that executable on 2026-11-13), engine layout offsets, engine and game names, flag names, and
behaviour. The AGPL side stands down through seam patch 0041 alone (section 15); R11's reserved patch number 0053 is
not used.

Conventions are R0's: an RVA is `+0x0XXXXXXX` from the executable's image base; **C** client, **S** server, **B**
both. **(call)** marks a decision this page makes; **(ambiguity)** something the source did not settle;
**(departure)** a place the MIT runtime deliberately behaves differently from the AGPL one. All three are collected in
section 19.

---

## 1. Scope

| R11 owns | Role | Section |
| --- | --- | --- |
| The connection-list sanitiser every R11 net path uses | S | 5 |
| NETD-1: `UNetDriver::TickDispatch` (the base dispatch) | S | 6 |
| `SP-3@R11`: the hub driver's upkeep before the engine tick, with its level-collection registration | S | 7 |
| The manual net tick after the engine tick (the AGPL ran it inside `SP-3@R9`'s listening block) | S | 8 |
| NETD-2: `UNetDriver::SetReplicationDriver`, with the player character's default-object boost | S | 9 |
| NETD-3: `CreateReplicationDriver` | S | 10 |
| NETD-4 and NETD-5: the two net mode queries | S | 11 |
| NETD-6: `UNetConnection::Close` | B | 12 |
| NETD-7: the spline direction query guard | S | 13 |
| NETD-8: owned, not installed | S | 13.3 |

**Not R11, though the surface page puts them near it:**

- **EXIT-12's spline recovery.** Surface 3.4 and section 8 give R11 a recovery of an access violation at
  `+0x01091D5D` inside EXIT-12's vectored handler. R3's spec (section 1 and its finding 1) settled that the AGPL
  removed that recovery on 2026-10-08, and this page confirms it from the same source: the "hub spline crash" was the
  AGPL runtime itself calling a stale 1.12 address, `+0x03D47AC0`, which on 1.14.7 lies one byte inside an instruction
  of the spline direction query (`+0x03D47A90`, NETD-7's target). Entered there, the query called the curve evaluator
  (`+0x01091D50`) with a 32-bit image address as the curve. That call is gone from the AGPL. **R11 ports no fault
  recovery and asks nothing of EXIT-12.** Its only use of R3 is R3's address text (section 3).
- RG-1 and RG-3..7, the replication-graph drive and the channel quarantine are R12's; RG-2 and the per-tick
  player-pawn rate writes are R13's. R11 hands R12 what section 16 lists.
- The listen steps, R9's own per-frame driver upkeep and the readiness announcement are R9's; R11 reads R9's listen
  state (section 3).
- The watchdog's connection count and the expected-player sync, in the same AGPL listening block, are R8's.

---

## 2. Where it goes

### 2.1 Files

| Crate | Path | Holds |
| --- | --- | --- |
| `rustic-isles-runtime-core` | `src/net.rs` (or a folder `src/net/` past ~400 lines) | everything pure: the sanitiser's decisions over a `Memory` (section 5), the level-collection choice (7.3), the spline curve check and its one-entry cache (13.1), the half-step and re-entry state of the manual tick (8), the line builders (every text on this page), the log budgets (4.3), the flag table (14) |
| `rustic-isles-runtime` | `src/ffi/net/` (`mod.rs`, `sanitize.rs`, `dispatch.rs`, `upkeep.rs`, `manual_tick.rs`, `repdriver.rs`, `netmode.rs`, `close.rs`, `spline.rs`) | the detours, the tick handlers, the calls into game code and the writes of game memory; under `ffi` because all of it is FFI (R0 2, RT-1) |

`core/src/lib.rs` gains `pub mod net;`, `ffi/mod.rs` gains `pub mod net;`.

### 2.2 Shared lines R11 adds

| File | Edit |
| --- | --- |
| `core/src/registry.rs` | `LANDED` gains `"R11"`, **only together with `"R9"`** (15.2). The NETD-1..8 lines (`Detour`) and `SP-3@R11` (`Feature`) exist already. One new test (17.1) |
| `core/src/offsets.rs` `TARGETS` | seven lines, NETD-1..7, all `FoundBy::RvaAndBytes` (section 18) |
| `core/src/offsets.rs`, a new `pub mod net` | the called functions, the data addresses and the layout offsets of section 18 |
| `src/lifecycle.rs` | before `handlers::freeze()`: `ffi::net::register()` (the two SP-3 handlers, server only). In the install list: R11's results at the positions of 4.4. Nothing else changes there |

`owned_count()` then counts 9 more ids (NETD-1..8 and `SP-3@R11`); `owns` is role-blind, so the client's status line
counts them too.

### 2.3 What is pure and what is Windows-only

Pure (core, `cargo test` on every platform): every decision on this page over R0's `Memory` trait (a fake memory in
tests), every log text, the budgets, the flag table. Windows-only (DLL crate): the detours, the calls into game code,
`RtlCaptureStackBackTrace`, writes of game memory. Nothing in R11 runs against the real executable in CI (surface 9).

---

## 3. What R11 consumes from other slices

| From | What | Used in |
| --- | --- | --- |
| R0 | the broker (`broker::hook`, `original`), `handlers::register_tick`, `ffi::memory` (`readable`, `readable_this_frame`, `read`, `verify_entry`, `function_entry`), `frame_tick` (the frame epoch bump), the object array and names (`engine`), `flags::flag`, `flags::production_log_profile`, the log, the detaching flag | everywhere |
| R3 | the address text `frame_text` and the own-module stack trim (R3 3.2) | NETD-6's stack (12) |
| R15 / R17's reflection kit (`reflect.rs`, R17 section 3; whichever of R15 and R17 lands first creates it) | `find_class(name)`, `default_object(class)`, and the checked `write<T>` (R17 3.5) | NETD-2 (9), NETD-3 (10) |
| R9 | the listen state, below | 7, 8 |
| R5 | nothing called; R11's SP-3 before handler runs after R5's (`order = -1000`), as R5 9 expects | 7 |

**R9's interface (R9 is specified in parallel; R11 builds on this shape, R9's spec gives the names).** Three
functions, callable from SP-3 handlers on the game thread, never blocking, never logging:

- `listening() -> bool`: true from the frame R9's listen succeeded, on either listen path (R9's own `Listen` or the
  engine's listener with `MAP_URL_LISTEN.flag`), until the process ends.
- `net_driver() -> Option<usize>`: the `UNetDriver` R9's own `Listen` created or resolved (the hub's "GameNetDriver").
  `None` before listen, and on the engine-listener path, where the engine registered and ticks its own driver.
- `game_net_driver_name() -> Option<(i32, u32)>`: the `FName` `GameNetDriver` (comparison index, number), which R9
  builds for its own driver lookup.

R11 reads them; it never writes R9's state. If R9's names differ, the R11 implementer follows R9's spec and changes
only the call sites.

---

## 4. Common rules for every R11 hook and handler

### 4.1 Before anything else

1. **Detaching** (R0 3.3): if set, the detours call `next` with their arguments and return its answer; the tick
   handlers return. NETD-4 and NETD-5 are the one exception (11).
2. Call R0's `lifecycle::write_status_once()` (first use only; the R0 once-flag makes it cheap after).
3. The work runs inside `catch_unwind`; a panic logs `[Panic] <id> ...` once (R0 2) and the detour then calls `next`
   with its arguments, the handler returns.

Every game read goes through R0's checked reads (R0 5); a read that fails ends that piece of work as the section says.
No R11 code catches a hardware fault **(departure: section 8.4)**.

### 4.2 Install

Every NETD detour is `broker::hook`ed with `order = 0`, band `Mit`, after `verify_entry(rva, bytes)` of section 18
succeeds **(departure: the AGPL hooks by bare RVA; R11 fails closed on a wrong build)**. An id whose check fails is not
hooked and reports `prologue mismatch`; one gated off by its flag reports `flag <NAME>`. The functions R11 only calls
(18.2) are checked the same way the first time they are needed; a failed check turns that call off for the process and
logs once (section 8, 10).

### 4.3 Lines

Every R11 line carries the tag `[MitNet]` (R0 call 6). Addresses print as `0x<HEX>` (upper case, no padding; `null`
for 0), counts in decimal. A line that can repeat has a **budget**: the first `B` lines of that kind go out, the
`B+1`-th becomes `[MitNet] <kind>: further lines suppressed`, and the rest are dropped **(departure: the AGPL logs
most of these on every occurrence)**. Budgets are per process, counted with relaxed atomics. Each section names its
budget.

### 4.4 Install order (RT-17 for R11)

**Settled:** no R11 id depends on another id's install order, and NETD-2, NETD-3 and NETD-4/5 need only be in place
before the map loads, which every install in `DllMain` is. The MIT build keeps the AGPL's positions (surface 3.11) so
its install lines read in the same order:

- **Client:** NETD-6 right after SP-1 (the AGPL installs it just before; R0 7.6 keeps SP-1 first).
- **Server:** NETD-1, NETD-2, NETD-7, NETD-8's line (13.3), NETD-3, NETD-4, NETD-5, NETD-6, in that order, at the
  positions surface 3.11 gives them among the other slices' ids (after SP-3). The two SP-3 handlers are registered
  before `handlers::freeze()`.

Each id gives one install line through R0's status (`[MitRuntime] install NETD-<n> ok|<reason>`). The client installs
only NETD-6; NETD-1..5 and NETD-7 do not exist on a client and report nothing there.

---

## 5. The connection-list sanitiser (S)

One function, `sanitize(driver, site) -> bool`, used by NETD-1 (6) and the manual tick (8). It checks and, where it
must, repairs two arrays of the net driver; it answers **true** only when the driver is safe to walk this frame
unchanged. It runs on the game thread.

| Layout fact | Value |
| --- | --- |
| `UNetDriver.ServerConnection` | `+0x88` (pointer) |
| `UNetDriver.ClientConnections` | `+0x90`: a `TArray<UNetConnection*>` (data `+0x90`, `Num` `i32` `+0x98`, `Max` `i32` `+0x9C`) |
| The driver's pending-cleanup array | `+0xF0`: data pointer, `Num` `i32` `+0xF8`, `Max` `i32` `+0xFC`; elements `0x18` bytes each |
| A connection, bytes checked readable | `0x138` |

In order:

1. `readable_this_frame(driver, 0xA0)`; else line `invalid driver`, answer false.
2. `readable_this_frame(driver + 0x90, 0x10)`; else line `unreadable connection list`, answer false.
3. `sane_pointer_array(data, num, max, 1024)` (R0 5.3); else **reset**: write `data = 0`, `num = 0`, `max = 0`, line
   `reset connection list data=<p> num=<n> max=<m>` (the values before), answer false. The array's memory is not
   freed (the AGPL does the same: an engine allocation may not be freed from outside the engine).
4. **Compact:** walk `0..num`; keep, in order, each element for which `readable_this_frame(element, 0x138)` holds; write
   the kept ones to the front; zero every slot from the new count up to the old `num`. If any was dropped: one line
   `dropped connection index=<i> ptr=<p>` per dropped element, then `compacted connection list <old> -> <new>`, write
   the new `num`, answer false.
5. `readable_this_frame(driver + 0xF0, 0x10)`; else line `unreadable cleanup list`, answer false.
6. The cleanup array is valid when `num >= 0`, `max >= 0`, `num <= max`, `num <= 1024`, and `num == 0` or
   `readable(data, num * 0x18)`. Else reset it as in step 3, line `reset cleanup list data=<p> num=<n> max=<m>`, answer
   false.
7. Answer true.

Every line is `[MitNet] sanitize <site>: <text>`, `site` one of `dispatch-inner` (6), `before-dispatch`, `before-flush`
(8). Budget **64** per site **(departure)**. The core crate holds steps 3 to 6 as decisions over a `Memory` (what to
write and which line), the DLL crate applies them.

---

## 6. NETD-1: `UNetDriver::TickDispatch` (S)

| | |
| --- | --- |
| Target | `+0x03FAE420`, the base `UNetDriver::TickDispatch` (the one holding the engine's "Very long time between ticks" text). The IP driver's own `TickDispatch` (`+0x00A82CF0`, section 8) reads the socket and then calls this one at `+0x00A82D2F` |
| Signature | `extern "C" fn(driver: *mut c_void, delta: f32)`, no return value |
| Why | the engine walks `ClientConnections` here with no per-element check; a torn connection list after a failed local load faulted at `+0x03D91CEB` |

Behaviour: run `sanitize(driver, "dispatch-inner")`. True: call `next(driver, delta)`. False: **absorb** (do not call
`next`) and log `[MitNet] TickDispatch skipped this frame: unsafe connection state` (budget 64). The next frame runs it
again on the repaired lists.

The AGPL also logs its first three calls and every 600th; R11 logs only the first:
`[MitNet] TickDispatch first call driver=<p>` **(departure, RT-3)**.

---

## 7. `SP-3@R11`: the hub driver's upkeep before the engine tick (S)

### 7.1 Registration

`handlers::register_tick` with `slice: "R11"`, `order: -500` **(call: after R5's -1000, before every other MIT before
handler)**, `before` only. When R11 is in `LANDED`, `SP-3@R11` is owned and the AGPL skips its own block (seam 0041).

**Position against the AGPL (cross-slice).** The AGPL ran this block inside its tick wrapper after its frame pacing,
the player host's fence check and its server-flag write, just before the engine tick. As an MIT before handler it now
runs before all of those (R0 8.2). Nothing between them reads or writes the driver fields below, so the order change
has no effect **(call)**. On the frame the player host's fence ends the process, the upkeep has already run; it is
harmless.

### 7.2 Each frame

1. `listening()` false, or `net_driver()` `None`: return. (The engine-listener path never reaches step 2: its driver
   is the engine's own.)
2. `driver` = `net_driver()`; require `readable(driver, 0x2B0)`; else return.
3. `world` = the pointer at `GWorld` (data `+0x06FC7F58`, R0 6.2); require `readable(world, 0x40)`; else return.
4. **Register the driver in the world's level collections** (7.3).
5. Write `driver.World` (`+0x140`) = `world`; `driver.NetDriverName` (`+0x190`, 8 bytes) = `game_net_driver_name()`
   (skip this one write when it is `None`); `driver.ServerConnection` (`+0x88`) = 0.

Each write goes through the reflection kit's checked `write<T>` (R17 3.5). Nothing logs in steps 1 to 3 and 5.

### 7.3 The level-collection registration

Why (a fact of this build): `UWorld::SetActiveLevelCollection` copies a level collection's net driver into
`UWorld.NetDriver` (`+0x38`). The world's gameplay collections hold none for the hub's own driver, so without this the
engine overwrites `UWorld.NetDriver` with null, and a native path that reaches the world through the driver's network
notify interface (the world's `+0x28`; its `+0x10` is the world's `NetDriver`) faults on it.

| Fact | Value |
| --- | --- |
| `UWorld.NetDriver` | `+0x38` |
| `UWorld.LevelCollections` | `+0x148`: data pointer, `Num` `i32` at `+0x150` |
| `FLevelCollection` | `0x78` bytes: the collection type `u8` at `+0x00` (0 dynamic source, 1 dynamic duplicated, 2 static), `NetDriver` at `+0x10` |
| `UNetDriver.World` | `+0x140` |
| `GEngine` | data `+0x06FC3CE8` (a `UEngine*`); the engine must be readable for `0xC48` |
| `UEngine::FindNamedNetDriver(engine, world, FName)` | `+0x042667F0`, `extern "C" fn(engine, world, name: u64) -> *mut c_void`, the name passed as its 8 bytes. It looks the world's context up (it reads the engine's world list at `+0xC38`) and returns that context's net driver of that name, or null |

Per process the handler keeps three remembered worlds: **registered**, **deferred**, **rejected** (pointers; 0 at
start). Each frame:

1. `readable(driver, 0x198)` and `readable(world, 0x160)`, else stop.
2. `world == registered`: stop (done for this world).
3. `driver.World` (`+0x140`) `!= world`, or the collections are unusable (`Num` not in `1..=8`, or the data not
   readable for `Num * 0x78`): when `world != deferred`, set `deferred = world` and log
   `[MitNet] level collections: registration deferred world=<p> driverWorld=<p> collections=<n|unavailable>`. Stop.
4. Read `GEngine`; require it readable for `0xC48`; call `FindNamedNetDriver(engine, world, driver.NetDriverName)`
   (the 8 bytes at `driver + 0x190`). If the answer is not `driver`: when `world != rejected`, set `rejected = world`
   and log `[MitNet] level collections: registration refused, the world's context does not resolve this driver
   world=<p> resolved=<p> driver=<p>`. Stop (it tries again next frame).
5. For each collection whose type is 0 or 2, write `driver` to its `+0x10`. Set `registered = world` and log
   `[MitNet] level collections: driver set on <n> gameplay collection(s) world=<p> driver=<p>`.

Step 4 guards step 5: if the context did not resolve this driver by name, `SetActiveLevelCollection`'s own
re-resolution would undo the write, so nothing is written. `UWorld.NetDriver` is never written directly. The choice
of collections and the four outcomes are pure (core) over the read values; the call is in `ffi`.

`FindNamedNetDriver` is checked (`verify_entry`, section 18) before its first call; a failed check logs
`[MitNet] level collections: FindNamedNetDriver prologue mismatch, registration off` once and step 4 always stops.

---

## 8. The manual net tick (S)

### 8.1 Why

The hub's driver is R9's own and is not in the engine's world context, so the engine tick never dispatches or
flushes it (measured on this build: the hub made no socket reads at all without the manual drive, and the engine never
entered NETD-1). R11 drives it once a frame after the engine tick: the IP driver's `TickDispatch` (receive), then
`UNetDriver::TickFlush` (send; it runs `ServerReplicateActors` through the driver's virtual table `+0x2A8`).

| Called function | RVA | Signature |
| --- | --- | --- |
| The IP net driver's `TickDispatch` (reads the socket, then calls NETD-1's target) | `+0x00A82CF0` | `extern "C" fn(driver, delta: f32)` |
| `UNetDriver::TickFlush` | `+0x03FAE700` | `extern "C" fn(driver, delta: f32)` |

Both are called at their entries (not through the broker: neither is hooked). The first argument is the driver; the
AGPL measured that passing the engine faults at once.

### 8.2 Registration and position

`handlers::register_tick`, `slice: "R11"`, `order: 800` **(call)**, `after` only.

**Position (cross-slice).** In the AGPL this ran last in the listening block: after the engine tick and every post-tick
block (the quarantine sweep, the weapon gap and dormancy work, the graph drive, the progression resume), and within
the listening block after R8's expected-player sync and connection count and R9's per-frame driver upkeep. As an
MIT after handler it runs after every AGPL post-tick block (R0 8.2), so the order holds while those are AGPL. Among
MIT after handlers: **R8's and R9's listening-block steps and R12's graph drive use orders below 800**; R11 tells their spec
writers so (section 16).

### 8.3 Each frame

1. `listening()` false, or `net_driver()` `None`: return silently **(departure: on the engine-listener path the AGPL
   ran the sanitiser on a null driver and logged two lines a frame)**.
2. **Re-entry:** a thread-local flag (const-initialised, no destructor). When set, return: a nested engine tick inside
   the drive must not drive again (the AGPL measured unbounded recursion ending in a stack overflow). It guards both
   calls **(departure: the AGPL guarded only the dispatch)**.
3. Set the flag for steps 4 and 5; clear it on every exit path.
4. **Dispatch:** `sanitize(driver, "before-dispatch")` true and the dispatch target's check passed: call
   `dispatch(driver, delta)`, then R0's frame epoch bump (`frame_tick`), since game code ran and freed nothing R11 may
   still trust.
5. **Flush:** `sanitize(driver, "before-flush")` true and the flush target's check passed: call
   `flush(driver, delta)`, then the epoch bump.

`delta` is the frame's delta the SP-3 after handler receives. A sanitiser answering false skips that one call for the
frame; the other still runs.

The first time either target's check fails, log
`[MitNet] manual net tick: <TickDispatch|TickFlush> prologue mismatch at +0x<RVA>, call off` and never call it.
The first frame that calls both logs `[MitNet] manual net tick on driver=<p>`.

### 8.4 No fault recovery **(departure, call)**

The AGPL calls both inside structured exception handling: a fault in either is logged, absorbed, summarised every 30 s
(a first line, then a count every 30 s, then a recovery line), and the server keeps running. The MIT runtime has
no structured exception handling (R0 call 7) and does not build one here. A fault inside the engine's dispatch or flush
ends the server through R3's crash path (EXIT-12 logs it with its stack, EXIT-14 bounds the crash-report wait), and
the Director restarts it. Reasons: every fault the AGPL recorded there came from wrong addresses or wrong arguments
that are fixed; its own notes say a caught fault "can become persistent" and leave a half-dead hub that players can
join but not play on; and skipping engine frames on a fault leaks whatever the engine held. The maintainer may rule
otherwise (19, call 5); a recovery would then be a VEH-armed call stub, and this page would grow a section for it.

---

## 9. NETD-2: `UNetDriver::SetReplicationDriver` (S)

| | |
| --- | --- |
| Target | `+0x03FACF60` |
| Signature | `extern "C" fn(driver, rep_driver)`, no return value |

Behaviour:

1. Log the first **16** calls: `[MitNet] SetReplicationDriver #<n> driver=<p> repDriver=<p> class=<short name|null>`
   (the class of `rep_driver`, `+0x10`, by R0's names; `null` for a null driver; `no-class` when its class is
   unreadable).
2. **The player character's default object** (9.1), when the rate boost or the always-relevant bit is on.
3. Call `next(driver, rep_driver)` unchanged. (A 1.4.4-era version forced null here; on this build only the game's
   replication graph opens actor channels, so the driver passes through.)

The AGPL's post-install snapshot of the graph's internals is a diagnostic and is not ported **(departure, RT-3)**.

### 9.1 The default-object boost

Why: the graph fixes each actor's replication period when it registers it, from the class default. Players register
at about 10 Hz where the behemoth gets about 30 Hz. Raising the default object's rates **before** the graph
initialises (which `SetReplicationDriver` does) gives every player pawn period 1.

- The object: `default_object(find_class("BP_PlayerCharacter_C"))` (the reflection kit). **(departure)** The AGPL looks
  it up once by the name `Default__BP_PlayerCharacter_C` and keeps a miss forever; R11 looks again on each NETD-2 call
  until found (NETD-2 runs a handful of times per process), since the class may load after the first driver.
- Require it readable for `0x110`.
- **Rate boost** (default **on**; off with `DISABLE_PLAYER_REP_BOOST.flag`, read once): write `f32` `60.0` at
  `+0x108` (`NetUpdateFrequency`) and `30.0` at `+0x10C` (`MinNetUpdateFrequency`). The first write logs
  `[MitNet] player default object: NetUpdateFrequency <old> -> 60, MinNetUpdateFrequency <old> -> 30`.
- **Always relevant** (default **off**; on with `PLAYER_ALWAYS_RELEVANT.flag`, read once): set bit `0x08`
  (`bAlwaysRelevant`) of the byte at `+0x58`. The first logs `[MitNet] player default object: bAlwaysRelevant set
  (byte <old hex> -> <new hex>)`.
- Not found yet: log `[MitNet] player default object: BP_PlayerCharacter_C not loaded yet` (budget 4).

Both flags are listed under R13 in the surface's Appendix A; R13's per-tick pawn writes read the same two files. Two
slices reading one flag file is fine (R0 4.2) **(call: the default-object half moves with NETD-2)**.

---

## 10. NETD-3: `CreateReplicationDriver` (S)

| | |
| --- | --- |
| Target | `+0x03F8D8E0` |
| Signature | `extern "C" fn(driver, a2, a3) -> *mut c_void` (the new replication driver, or null) |
| Why | the game's function gives up ("ReplicationDriverClass is null! Not using ReplicationDriver.") when a global enable gate reads 0 or the driver's `ReplicationDriverClass` is null; then no `ArchonReplicationGraph` exists, no actor channel opens and the client waits at the loading screen |

| Fact | Value |
| --- | --- |
| The enable gate | data `+0x06AD6F54`, a `u32`; the game tests it against 0 |
| The replication-graph feature array | data `+0x06FB12E0`: data pointer, `Num` `i32` at `+0x06FB12E8`. When non-empty, the game takes a feature path that skips `ReplicationDriverClass` |
| `UNetDriver.ReplicationDriverClass` | `+0x178` (`UClass*`) |
| `UClass` class flags | `u32` at `+0xCC`; `Abstract` is bit `0x1` |
| `GetTransientPackage` | `+0x02810780` is a two-instruction leaf that returns the pointer stored at data `+0x06E57480`; R11 reads that pointer instead of calling it **(call)** |
| `StaticConstructObject_Internal` | `+0x02886C70`, `extern "C" fn(params: *const u8) -> *mut c_void`; `params` is the engine's construct-parameters struct: class pointer at `+0x00`, outer at `+0x08`, every other byte zero, `0x50` bytes, 8-aligned |

Behaviour, per call (`n` counts calls from 0):

1. **Gate:** if the gate is readable, write `1`. Calls 0..2 log `[MitNet] CreateReplicationDriver #<n> gate <old> -> 1`.
2. **Class:** if `readable(driver, 0x200)`: read `+0x178`; when null, choose the class (10.1) and write it there. Calls
   0..5 log `[MitNet] CreateReplicationDriver #<n> class before=<p> after=<p> (<short name|null>)`.
3. **Feature path off:** if the feature array's `Num` is readable and greater than 0, remember it and write 0.
4. **Abstract off:** if the class's flags are readable and have bit `0x1`, remember them and clear that bit only.
5. **Construct:** with a class chosen and the construct function's check passed: `outer` = the pointer at
   `+0x06E57480` (require it live, R0 5.3; else skip to step 6). Build the zeroed `0x50`-byte parameters with the class
   and the outer; call `StaticConstructObject_Internal`. Log `[MitNet] CreateReplicationDriver #<n> constructed
   <p> (<class short name of the result|null>)` for calls 0..5. Non-null: restore (step 7) and **return it without
   calling `next`** (absorb). Null: log `[MitNet] CreateReplicationDriver #<n> construct returned null, calling the
   game's own` and go on.
6. `result = next(driver, a2, a3)`.
7. **Restore,** on every path that reaches it: the class flags (when step 4 changed them), then the feature array's
   `Num` (when step 3 changed it).
8. Calls 0..5 log `[MitNet] CreateReplicationDriver #<n> game returned <p> (<short name|null>)`; return `result`.

Steps 3 and 4 are restored within the same call, on the game thread, so nothing else sees them changed.

### 10.1 The class

`find_class("ArchonReplicationGraph")` (the game's own graph); when absent, `find_class("BasicReplicationGraph")`;
when both are absent, none (step 2 writes nothing and step 5 is skipped). Searched once per process and kept, found or
not, as in the AGPL. The first search logs `[MitNet] replication graph class: <ArchonReplicationGraph|
BasicReplicationGraph|none> <p>`.

The AGPL's diagnostic dumps here (the feature array's first helper, a class-flags scan, the class default object, the
transient package) are not ported **(departure, RT-3)**.

R11 exposes what R12 needs (16): the chosen class and whether the last call constructed the graph itself.

---

## 11. NETD-4 and NETD-5: the net mode queries (S)

| Id | Target | What the game function does |
| --- | --- | --- |
| NETD-4 | `+0x03F91EB0` `InternalGetNetMode` | asks the driver's `IsServer` through its virtual table `+0x378` |
| NETD-5 | `+0x042CC560` `UWorld::GetNetMode` | reads `UWorld.NetDriver` (`+0x38`) and asks the same |

Both detours are `extern "C" fn(this: *mut c_void) -> i32` and return **1** (`NM_DedicatedServer`) without calling
`next` (absorb). They read nothing, write nothing and never log.

- **The full register.** The return type is a 32-bit integer and the whole `EAX` must be 1. The AGPL once returned a
  one-byte value, leaving the upper bits of `EAX` from the driver pointer (measured `0xEE47C001`); the engine then
  misclassified the server when routing `ServerDraftBounty` and the automatic bounty draft stalled.
- `+0x042CC6E0` is a different function on this build (it returns `GIsClient`) and must never be hooked.
- **Detaching (call).** These two answer 1 even after detach instead of forwarding: answering touches nothing, while
  the original would call into a driver that may be torn down. It is R0 3.3's intent (touch nothing after detach) met
  more strictly than its letter.
- No panic is possible in a constant; they still sit behind R0's catch for uniformity.

---

## 12. NETD-6: `UNetConnection::Close` (B)

| | |
| --- | --- |
| Target | `+0x03F62B90` |
| Signature | `extern "C" fn(connection)`, no return value |
| Why | the only native view of who closes a connection: on the server for a failed join, on the client that presses "Go Now" |

| Fact | Value |
| --- | --- |
| Bytes checked readable | `0x1D8` |
| Connection state | `u8` at `+0x134` |
| Last receive time | `f64` at `+0x1D0` |

Behaviour (`seq` counts calls from 0 across the process, valid or not):

1. `seq >= 16`: call `next(connection)` and return **(departure: the AGPL logged an unreadable connection without a
   bound)**.
2. Connection null or not readable for `0x1D8`: log `[MitNet] Close <client|server> #<seq> invalid connection=<p>`,
   call `next`, return.
3. Capture the stack: `RtlCaptureStackBackTrace(0, 24, ...)`, R3's own-module trim, keep at most **12** frames, each as
   R3's `frame_text`, joined by single spaces.
4. Log `[MitNet] Close <client|server> #<seq> enter conn=<p> state=<u8> lastReceive=<f64, six decimals> stack=<...>`.
5. Call `next(connection)`.
6. Log `[MitNet] Close <client|server> #<seq> exit conn=<p>`.

The connection's field at `+0x1318` and the two virtual calls `Close` makes are never read or interpreted (an earlier
AGPL version that read `+0x1318` as an object crashed).

---

## 13. NETD-7 and NETD-8: the spline guards (S)

### 13.1 NETD-7: the spline direction query

| | |
| --- | --- |
| Target | `+0x03D47A90` (its `.pdata` entry ends at `+0x03D47D05`): a component's direction at a spline input value. It hands the component's curve at `+0x448` to the curve evaluator (`+0x01091D50`, called at `+0x03D47ACB`), normalises the result, and may rotate it by the component's rotation |
| Signature | `extern "C" fn(component, out: *mut f32, input: f32, apply_rotation: i32) -> *mut f32` (returns `out`) |
| Gate | off-flag `DISABLE_SPLINE_GUARD.flag`, read once at install |

| Fact | Value |
| --- | --- |
| The component's curve | `+0x448`: an `FInterpCurveVector`; its points array data pointer at `+0x00`, `Num` `i32` at `+0x08` |
| One curve point | `0x2C` bytes |
| Bytes checked readable on the component | `0x448 + 0x18` |

Behaviour:

1. `out` null: call `next` and return its answer.
2. **Usable** when the component is non-null and readable for `0x460`, and then: the read `(component, data, num)`
   equals the last accepted triple with `data != 0`, or `data != 0`, `0 < num <= 10000` and `readable(data, num *
   0x2C)`; a fresh acceptance replaces the remembered triple. The triple lives in a thread-local (the game thread
   only calls it; the AGPL kept one unsynchronised static).
3. Usable: return `next(component, out, input, apply_rotation)`.
4. Not usable: write `0.0` to `out[0..3]`, log `[MitNet] spline guard: unusable curve on component=<p> data=<p>
   num=<n>, returned a zero direction` (budget 8), return `out`.

The usable test is pure (core) over the read values and the remembered triple.

At install, when the flag is present: install line `flag DISABLE_SPLINE_GUARD` and nothing hooked. Either way one line
`[MitNet] spline guard <on|off (DISABLE_SPLINE_GUARD.flag)>`.

**Status (call).** Since the crash it was written for came from the AGPL's own stale call (section 1), this guard has
never caught a real fault. It is ported because it is today's default and costs one cached check per distinct curve;
if the play-tests never show its line, a later row may retire it.

### 13.2 What the evaluator is

`+0x01091D50` evaluates an `FInterpCurveVector`: it reads `Num` at the curve's `+0x08` first (the faulting instruction
at `+0x01091D5D` was that read) and bisects the points. Signature `(curve, out, input, param4)`.

### 13.3 NETD-8: owned, not installed **(call)**

The AGPL installs a guard on the evaluator itself only with `ENABLE_SPLINE_EVAL_HOOK.flag`, and its own note marks it
"measured unsafe": the detour faulted in its own prologue on an aligned-store of a vector register, because the stack
it was entered with was misaligned. That misalignment came from the same stale mid-instruction entry. With that entry
gone the evaluator guard protects nothing.

R11 owns NETD-8 (so the AGPL never installs it) and installs nothing: install line `not ported`. The flag
`ENABLE_SPLINE_EVAL_HOOK` is dropped (14). The `TARGETS` table has no NETD-8 line; R0's offset-table test is told that
NETD-8 is a registered detour id with no target on purpose (17.1).

---

## 14. Flags (RT-14 for R11)

Appendix A gave R11 seven flags. Settled, with the two R11 shares with R13:

| Flag | Kept? | Meaning in the MIT runtime |
| --- | --- | --- |
| `DISABLE_SPLINE_GUARD` | kept | read once at install: NETD-7 not installed |
| `ENABLE_SPLINE_EVAL_HOOK` | **dropped** | NETD-8 is not ported (13.3) |
| `MANUAL_NET_TICK` | **dropped** | in the AGPL it skips the server's manual drive (despite its name), and so does `NATIVE_NET_TICK`; measured on this build, a server without the manual drive never reads its socket and nobody can join. A switch that can only make a hub unjoinable is not kept |
| `NATIVE_NET_TICK` | **dropped** | as above |
| `MANUAL_TICK_ZERO_DT` | **dropped** | marked broken in the AGPL: a zero delta freezes the connection handshake timers and nobody can join |
| `MANUAL_TICK_HALF_RATE` | **dropped** | an A/B for the old double drive (engine plus manual, about 60 Hz), which no longer exists; with today's single drive at the server's frame cap it would halve the net rate |
| `EMERGENCY_LEGACY_REPLICATION` | **dropped** | its R11 effect only skips the graph self-construct (10 step 5); its R9 effect turns on a hand-written actor loop that the AGPL itself refuses on 1.14.7, so with it a server replicates nothing. R9's spec is asked to drop it too |
| `DISABLE_PLAYER_REP_BOOST` (R13's) | read | 9.1, read once |
| `PLAYER_ALWAYS_RELEVANT` (R13's) | read | 9.1, read once |

A dropped flag left in a game folder has no effect on the MIT side; while the AGPL still owns R9 it may still read
`EMERGENCY_LEGACY_REPLICATION` (15.2 makes that impossible: R9 and R11 land together).

---

## 15. The AGPL side and the other slices

### 15.1 How the AGPL stands down (seam patch 0041, no new patch)

- NETD-1..8 are in 0041's RVA table. When R11 owns them, the AGPL's `MH_CreateHook` for each makes no wrapper and its
  "original" pointer is `rir_original(target)`; its enable calls succeed as no-ops. NETD-8's install is behind its
  flag in the AGPL anyway.
- `SP-3@R11` gates the AGPL's before-tick block (the level-collection registration and the three driver writes).
- The AGPL's manual net tick sits inside its listening block, which `SP-3@R9` gates as a whole (R0 7.5 already records
  that this block "also carries ... the manual net tick (R11's)"). 15.2 makes owning `SP-3@R9` and owning R11's ids
  the same event.
- What stays AGPL and is harmless: its install lines (the seam answers `MH_OK`, so it still logs "create=MH_OK" for
  NETD hooks it no longer installs) and its once-only spline-guard flag line. Its replication-driver globals are set
  but only its own (now uninstalled) detours read them.

So nothing beyond 0041 is needed, and **R11's reserved patch number 0053 is unused**.

### 15.2 R9 and R11 land in one build **(call)**

Each side switches off an AGPL piece the other needs:

- Owning `SP-3@R9` turns off the AGPL's whole listening block, **including its manual net tick**: an MIT runtime
  with R9 and without R11 never drives the hub's driver, and nobody can join.
- Owning `SP-3@R11` turns off the AGPL's driver upkeep. R11's replacement needs R9's listen state (3), which the AGPL
  cannot hand across: an MIT runtime with R11 and without R9 leaves the driver unbound, and `UWorld.NetDriver` is
  nulled under the engine.

So `LANDED` holds both `"R9"` and `"R11"` or neither, and a registry test enforces it (17.1). The R11 implementer's
branch adds `"R11"` to `LANDED` only if `"R9"` is already there on `plan`; otherwise it leaves `LANDED` alone and the
integration pass adds both when it merges the second of the two. Until then R11's code builds and its pure tests run,
and it installs nothing (nothing of it is owned). R0's cross-slice rule "R8, R9 and R10 ship in one runtime release"
becomes **"R8, R9, R10 and R11"**.

Row for R0 7.5's cross-slice table:

| State | Set by | Read by | Rule |
| --- | --- | --- | --- |
| the hub driver, "listening", the GameNetDriver name | `SP-3@R9` | `SP-3@R11`, the manual net tick | **R9 and R11 land in one build** (15.2) |

### 15.3 Rulings settled here

- **RT-14 for R11:** section 14.
- **RT-17 for R11:** section 4.4; no order dependency.
- **RT-18 for R11:** R11 registers no `ProcessEvent` handler and matches no names.
- RT-12, RT-13, RT-15 and RT-16 do not touch R11.

---

## 16. What other slices need from R11

| Slice | Needs |
| --- | --- |
| R9 | the interface of section 3 (R9's spec writer was told on 2026-11-13); its listening-block after handlers at orders below 800; no port of the manual net tick; dropping `EMERGENCY_LEGACY_REPLICATION` |
| R8 | its listening-block steps (expected-player sync, connection count) at orders below 800 |
| R12 | `net::sanitize(driver, site) -> bool` (section 5) for any walk of `ClientConnections` it makes, with its own site names; `net::graph_class() -> Option<usize>` (10.1) and `net::graph_self_constructed() -> bool` (the last NETD-3 call returned a graph it built); the graph drive at an order below 800 so the manual tick's flush runs after it, as in the AGPL; the frame epoch rule (bump after calling game code) |
| R13 | the default-object boost is R11's (9.1); R13 keeps the per-tick pawn writes and reads the same two flag files |
| R3 | nothing: R11 asks EXIT-12 for no recovery |

---

## 17. Tests

### 17.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`), over a fake `Memory`:

- **Sanitiser:** a good driver answers true and writes nothing; an unreadable driver, an unreadable list header, each
  of the bad arrays (`num < 0`, `num > max`, `num > 1024`, unreadable data) resets the right array and answers false
  with the right line; compaction keeps order, zeroes the tail, writes the count and logs each dropped index; a bad
  cleanup array (`num * 0x18` unreadable, `num > 1024`) resets it; the per-site budget's 65th line is the
  "suppressed" line and the 66th is dropped.
- **Level collections:** the four outcomes (already registered, deferred with the right "unavailable" text, refused,
  registered) and that only types 0 and 2 are chosen; the deferred and refused lines once per world.
- **Spline check:** usable/unusable for null, unreadable component, `data == 0`, `num` 0, -1, 10000, 10001, unreadable
  points; the remembered triple short-circuits only when `data != 0`.
- **Manual tick state:** the re-entry flag blocks a nested call and is cleared after a panic path.
- **Lines:** every text on this page for fixed values (addresses, `null`, six-decimal float).
- **Registry:** `LANDED` contains `"R9"` iff it contains `"R11"`; with both, `owns` is true for NETD-1..8 and
  `SP-3@R11`.
- **Offsets:** NETD-1..7 each have one `RvaAndBytes` line with the bytes of section 18; NETD-8 is the one registered
  detour id allowed no `TARGETS` line (R0's test gets that exception list).

DLL (`cargo test -p rustic-isles-runtime`, Windows):

- NETD-4/5 bodies return exactly 1 in a full `i32` (call the detour function directly with a garbage pointer).
- NETD-7 on a test-allocated "component" with a good curve calls `next`; with a curve pointing at a `PAGE_NOACCESS`
  page writes zeros and returns `out` without faulting.
- The sanitiser on a test-allocated driver block with one connection pointer into a `PAGE_NOACCESS` page compacts it.
- The manual tick and NETD-3's calls are not exercised (they need the game).

### 17.2 What CI cannot show

Every hook against the real executable, the replication graph's construction, joins and leaves, the level-collection
registration, the manual drive's effect on joining: the play-test.

---

## 18. Offset-table lines (R11)

### 18.1 `TARGETS` (`FoundBy::RvaAndBytes`; each RVA is a `.pdata` function entry with these first bytes)

| Id | RVA | Bytes | What |
| --- | --- | --- | --- |
| NETD-1 | `0x03FA_E420` | `48 8B C4 48 89 58 18 55` | `UNetDriver::TickDispatch` |
| NETD-2 | `0x03FA_CF60` | `40 56 41 56 48 83 EC 28` | `UNetDriver::SetReplicationDriver` |
| NETD-3 | `0x03F8_D8E0` | `48 89 5C 24 08 48 89 6C 24 10` | `CreateReplicationDriver` |
| NETD-4 | `0x03F9_1EB0` | `48 83 EC 28 48 8B 01 FF 90 78 03 00 00` | `InternalGetNetMode` |
| NETD-5 | `0x042C_C560` | `40 53 48 81 EC 90 00 00 00 48 83 79 38 00` | `UWorld::GetNetMode` |
| NETD-6 | `0x03F6_2B90` | `48 8B C4 53 41 56 48 83 EC 68` | `UNetConnection::Close` |
| NETD-7 | `0x03D4_7A90` | `48 89 5C 24 08 48 89 74 24 10 57 48 83 EC 60` | spline direction query |

### 18.2 `pub mod net`

| Name | Value | Use |
| --- | --- | --- |
| `IP_TICK_DISPATCH` | `0x00A8_2CF0`, bytes `48 8B C4 55 53 56 41 55 41 57` | 8 |
| `TICK_FLUSH` | `0x03FA_E700`, bytes `4C 8B DC 55 53 56 57 41 57` | 8 |
| `FIND_NAMED_NET_DRIVER` | `0x0426_67F0`, bytes `40 53 48 83 EC 20 48 8B 81 38 0C 00 00` | 7.3 |
| `STATIC_CONSTRUCT_OBJECT` | `0x0288_6C70`, bytes `48 89 5C 24 10 48 89 74 24 18 55 57` | 10 |
| `SPLINE_EVALUATOR` | `0x0109_1D50` (not hooked; for the record) | 13 |
| data `GENGINE` | `0x06FC_3CE8` | 7.3 |
| data `REP_DRIVER_GATE` | `0x06AD_6F54` | 10 |
| data `REP_GRAPH_FEATURES` | `0x06FB_12E0` (`Num` at `+8`) | 10 |
| data `TRANSIENT_PACKAGE` | `0x06E5_7480` | 10 |
| layout | `NETDRIVER_SERVER_CONNECTION 0x88`, `NETDRIVER_CLIENT_CONNECTIONS 0x90`, `NETDRIVER_CLEANUP 0xF0`, `NETDRIVER_WORLD 0x140`, `NETDRIVER_REP_DRIVER_CLASS 0x178`, `NETDRIVER_NAME 0x190`, `CONNECTION_STATE 0x134`, `CONNECTION_LAST_RECEIVE 0x1D0`, `WORLD_NET_DRIVER 0x38`, `WORLD_LEVEL_COLLECTIONS 0x148`, `LEVEL_COLLECTION_SIZE 0x78`, `LEVEL_COLLECTION_NET_DRIVER 0x10`, `CLASS_FLAGS 0xCC`, `CLASS_ABSTRACT 0x1`, `SPLINE_CURVE 0x448`, `CURVE_POINT_SIZE 0x2C`, `ACTOR_NET_UPDATE_FREQUENCY 0x108`, `ACTOR_MIN_NET_UPDATE_FREQUENCY 0x10C`, `ACTOR_ALWAYS_RELEVANT_BYTE 0x58`, `ACTOR_ALWAYS_RELEVANT_BIT 0x08` | throughout |

`GWorld` (`0x06FC_7F58`) is R0's.

---

## 19. The maintainer's play-test, calls and ambiguities

### 19.1 Play-test (surface section 8, R11 row; collected for the one hands-on pass at the end)

"Hub with two clients joining and leaving for 10 min; 'Go Now' from a hunt." Needs an MIT build with R9 and R11
landed (15.2), both DLLs injected as in R0 11.1.

1. **Hub boot.** Let the Director start Ramsgate. In `rustic_isles_dll_port<port>.log`: `install NETD-1 ok` through
   `install NETD-7 ok`, `install NETD-8 not ported`, `[MitNet] spline guard on`,
   `[MitNet] replication graph class: ArchonReplicationGraph ...`, `[MitNet] CreateReplicationDriver #0 gate ... -> 1`,
   `... constructed <p> (ArchonReplicationGraph)`, `[MitNet] SetReplicationDriver #0 ... class=ArchonReplicationGraph`,
   `[MitNet] player default object: NetUpdateFrequency ... -> 60 ...` (or the "not loaded yet" line and then the boost
   on a later call; note which), and the AGPL's `[MitSeam] ... <n> hooks left to it` with `n` at least 8 larger than
   before R11.
2. **Listen and the drive.** After R9's ready line: `[MitNet] level collections: driver set on <n> gameplay
   collection(s)` (expect 2), `[MitNet] manual net tick on driver=<p>`, `[MitNet] TickDispatch first call`. No
   `registration refused` line after the first few frames.
3. **Two clients, 10 min.** Log both in to Ramsgate, then have them leave and rejoin alternately, about once a minute,
   sometimes by quitting the game, sometimes by travelling to a hunt. Both see each other each time; the hub stays up.
   Count `[MitNet] sanitize` lines (expected: none, or a few `compacted` after an abrupt quit) and
   `[MitNet] Close server #<n>` lines (the first 16 closes, each with a stack).
4. **"Go Now".** From a hunt, press "Go Now" on one client. Its client log (`..._port0.log`) has
   `[MitNet] Close client #0 enter ... stack=...` and the matching `exit`; the client returns to Ramsgate.
5. **The bounty draft.** Start a hunt from the airship; the automatic bounty draft completes (NETD-4/5's full-register
   answer).
6. **Off switch.** With `DISABLE_SPLINE_GUARD.flag` next to the server executable: `install NETD-7 flag
   DISABLE_SPLINE_GUARD` and `[MitNet] spline guard off (DISABLE_SPLINE_GUARD.flag)`; the hub still runs 10 min.
7. Note any `[MitNet] spline guard: unusable curve` line: none is expected (13.1's status call).

Only CI passed until these steps are done.

### 19.2 Calls, ambiguities and departures

Calls (for the maintainer to confirm; the ones marked **ruling** need one):

1. **No fault recovery in R11:** EXIT-12's spline recovery is gone from the AGPL (R3 finding 1, confirmed here).
2. **Ruling: R9 and R11 land in one build** (15.2), enforced by a registry test; R0's cross-slice rule becomes "R8,
   R9, R10 and R11 ship in one runtime release". Recommended: yes. The alternative, R9 porting the manual net tick
   itself, puts R11's safety code in R9 and splits one behaviour across two slices.
3. R11 consumes R3 only for `frame_text` and the stack trim, R15/R17's reflection kit for lookups and checked writes,
   and R9's listen state (3).
4. SP-3 orders: the upkeep `-500` (before), the manual tick `800` (after).
5. **Ruling: no structured fault catch around the manual dispatch and flush** (8.4): a fault there ends the server
   through R3's crash path and the Director restarts it. Recommended: yes.
6. NETD-8 is owned and not installed; `ENABLE_SPLINE_EVAL_HOOK` dropped (13.3).
7. NETD-7 ported as today though it has never caught a real fault (13.1).
8. **Ruling: six flags dropped** (14): `ENABLE_SPLINE_EVAL_HOOK`, `MANUAL_NET_TICK`, `NATIVE_NET_TICK`,
   `MANUAL_TICK_ZERO_DT`, `MANUAL_TICK_HALF_RATE`, `EMERGENCY_LEGACY_REPLICATION`. Recommended: yes.
9. The default-object boost moves with NETD-2 and reads R13's two flags (9.1).
10. `GetTransientPackage` is read as its data pointer, not called (10).
11. NETD-4/5 answer 1 after detach instead of forwarding (11).
12. The upkeep's move ahead of the AGPL's pacing, fence and flag write changes nothing (7.1).
13. Patch 0053 unused (15.1).

Departures:

1. Every NETD target is checked against its first bytes before hooking (4.2).
2. New tag `[MitNet]` and new texts; budgets on every repeating line (4.3, 5).
3. NETD-1 logs only its first call (6); NETD-2's post-install snapshot and NETD-3's dumps are not ported (9, 10).
4. The manual tick is silent when there is no driver, and its re-entry guard covers both calls (8.3).
5. The player default object is looked up again until found (9.1).
6. NETD-6's bound counts every call, valid or not (12).
7. No fault catch around the drive (8.4).

Ambiguities:

- `ParmsSize`-style layout risk does not arise here (no `ProcessEvent` calls), but the construct-parameters struct is
  stock UE 4.26's (class `+0x00`, outer `+0x08`, the rest zero); the AGPL's comment says the game reads up to `+0x38`
  of it, and `0x50` zeroed bytes cover that. Checked only by the play-test's step 1 line.
- The pending-cleanup array at the driver's `+0xF0` has no name in the dump (its elements are `0x18` bytes); its
  meaning is the AGPL's reading of the engine, kept as a fact.

No open question remains beyond the three rulings (calls 2, 5, 8).

---

## 20. Size

Rust, without tests: the DLL crate about 650 lines (sanitiser 80, NETD-1 40, upkeep and registration 130, manual tick
90, NETD-2 70, NETD-3 120, NETD-4/5 20, NETD-6 60, NETD-7 60), the core crate about 400 (decisions 200, lines and
budgets 120, flags and offsets 80). Tests about 500. Smaller than the surface's 1 300: the diagnostics, the dropped
A/B paths and the fault recovery are not ported.
