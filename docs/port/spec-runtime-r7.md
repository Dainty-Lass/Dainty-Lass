<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R7, player data and loading (RD27, plan item 11)

> Since R21 (`spec-runtime-r21.md`) the AGPL runtime is gone; `rustic-isles-runtime.dll` is the only runtime. This
> slice's play-test runs on the MIT DLL alone, as that spec's section 9 adapts it: skip every step that builds or
> patches the AGPL runtime, injects both DLLs, counts `[MitSeam]` lines or runs without the MIT DLL.

R7 moves hooks **LD-1 to LD-9** (surface section 3.7) and two dispatch features, **`SP-2@R7`** (the restart data
gate inside `PlayerCanRestart`, patch 0012) and **`SP-3@R7`** (resuming parked progression components), into the MIT
runtime. It is the slice that gets a player through the loading screen: the readiness gate on both roles, the server's
handling of a player-data load that never finishes, and the progression configuration a hunt server must ask for before
anyone's data can load. Runtime patches 0004, 0008, 0011 and 0012 move with it (surface section 6); 0013 is dropped.

An implementer builds from this page, `spec-runtime-surface.md`, `spec-runtime-r0.md`, `spec-runtime-r1.md` and the
reflection kit of `spec-runtime-r17.md` section 3, and never reads `restricted/`. What crossed is facts: RVAs into
`Dauntless-Win64-Shipping.exe` (1.14.7, CL 647472), engine and game class, function and member names and offsets from
the maintainer's Dumper-7 dump of this build, parameter block layouts, flag names, exact log texts and behaviour.
Conventions are the surface page's: an RVA is `+0x0XXXXXXX` from the executable's image base; **C** client, **S**
server, **B** both. **(call)** marks a decision made here, **(departure)** a deliberate difference from the AGPL
runtime, **(ambiguity)** something the source did not settle; all are collected in section 13.

---

## 1. Scope

| R7 owns | Role | Section |
| --- | --- | --- |
| LD-1, the `HasFinishedLoading` dispatcher: the server's compatibility answer, the client's two narrow overrides | B | 4 |
| The client's possession capture that LD-1 needs (an observing SP-1 handler, no feature id) | C | 4.4 |
| LD-2, `OnPlayerDataLoadComplete`: marks the player's data loaded, ends a retry, backfills the hunt id | S | 5 |
| LD-3, `UArchonLoadManager::LoadFailed`: the one-shot retry, the drop of a load with no net id | S | 6 |
| `SP-2@R7`, the restart data gate | S | 7 |
| LD-5, LD-6, LD-7 and `SP-3@R7`: the progression configuration query and the resume of parked components | S | 8 |
| LD-4, LD-8, LD-9: owned and **not installed** (logging-only, RT-3) | B / S | 9 |
| RT-12, and RT-14, RT-17, RT-18 for R7 | | 10, 13 |

Not R7: the `PlayerCanRestart` force itself and the rest of `SP-2@R20` (R20, which consumes R7's gate, 7.4); the
client's input activation and HUD repair around `ClientRestart` (`SP-1@R14`, R14); the `GIsClient` boot patch that
removes the phantom local player (HL-2, R5; R7 only reads whether it took, 3.3); every logging-only probe near these
hooks (RT-3, section 9).

### 1.1 Where the code goes

| File | Holds |
| --- | --- |
| `crates/rustic-isles-runtime-core/src/loading.rs` (new) | everything pure: the retry slot table (6.3), the restart gate table (7.2), the hub class rule (7.1), the hunt id extraction (5.3), the waiter and candidate bookkeeping (8.2), the poll's throttle and give-up timing (8.4), the possession timing rule (4.4), the idle-trigger predicate over values already read (4.3), the rearm decision over a loader snapshot (6.2), the log texts (11) |
| `crates/rustic-isles-runtime/src/ffi/loading.rs` (new) | the six detours (LD-1, LD-2, LD-3, LD-5, LD-6, LD-7), the three handlers, every game read, write and call |
| `crates/rustic-isles-runtime-core/src/offsets.rs` | the R7 lines of section 3 (nine `TARGETS` lines, `pub mod r7 { ... }`), with a comment naming R7 |
| `crates/rustic-isles-runtime-core/src/registry.rs` | `LANDED` gains `"R7"`. Every R7 id is already there (`LD-1`..`LD-9` kind `Detour`, `SP-2@R7`, `SP-3@R7`): R7 adds no id |
| the reflection kit, `reflect.rs` in both crates (R17 section 3) | used as it is; R7 adds `process_event_original` if no earlier slice did (3.4) |
| one-line edits | `pub mod loading;` in the core `lib.rs` and in `ffi/mod.rs`; one install entry per role in `src/lifecycle.rs` (3.5) |

If `ffi/loading.rs` passes about 600 lines, split it into `ffi/loading/{readiness,player_data,progression}.rs` with a
`mod.rs`; nothing else changes.

### 1.2 What R7 reads from other slices

| Value | From | Use |
| --- | --- | --- |
| `config::is_server()`, `config::auth_mode()` | R0 3.2, R1 5 | role; the gate is `GlobalGameserver` only |
| `config::launch()` → `Launch::expected_players` | R1 6.1 | the shared hunt id (5.3) |
| `ffi::broker::hook`, `broker::original`, `NextCell` | R0 7.2 | the six wrappers; calling an original past the chain |
| `handlers::register_pe`, `register_tick` (`Point::Sp1`, `Sp2`, `Sp3`, `PeHandler`, `TickHandler`) | R0 8 | the three handlers |
| `ffi::memory::readable`, `read`; `ffi::engine::is_live_object`, `full_name` | R0 5, 6 | every read; liveness |
| `ffi::flags::flag` | R0 4.2 | R7's flags (10) |
| `ffi::log::line`, `ffi::detaching()`, `lifecycle::write_status_once` | R0 3.3, 4.3 | logging, the detach rule |
| `reflect::is_a`, `find_function`, `params_fit`, `ffi::reflect::call`, `write`, `FStringParam` | R17 3 | class checks, the two UFunction calls, the checked writes |

**R2 (call).** The surface lists R2 as a dependency. R7 calls nothing of R2's: the dependency is behavioural. The
requests R7 causes (the progression configuration query of 8.3, a player-data loader's reload after a retry, 6) are
ordinary game HTTP requests, which reach the Dainty Lass backend only through HTTP-1's redirect (`spec-runtime-r2.md`
section 4). That redirect is the AGPL's until R2 lands and the MIT's after; either serves R7. R7 can therefore be
implemented and merged without R2's code, but its play-test (12.3) assumes the redirect works, so run it on a build
where R2's play-test has passed or with the AGPL owning HTTP-1.

---

## 2. Ownership and the AGPL side

With `"R7"` in `LANDED`, `rir_owns` answers 1 for `LD-1` to `LD-9`, `SP-2@R7` and `SP-3@R7`. The status line's count
grows by 11.

Seam patch 0041 already covers every R7 id; **no further AGPL patch is needed (0051 is not used)**:

- LD-1 to LD-9 are hook ids. With them owned, the AGPL runtime makes none of the nine detours: its original pointer
  for each is set to `rir_original(target)`, so AGPL code that calls one of these functions "past its hook" still
  reaches the game (the AGPL's loadout repair, `SP-1@R15`, calls the `HasFinishedLoading` original that way; R15's MIT
  side does the same through `broker::original`).
- Everything the AGPL did inside those nine detours is R7's: the readiness answers, the retry, the hunt id backfill
  (its flag was listed under R20 in the surface's Appendix A; it lives in LD-2, so it moves to R7, section 10), the
  query, the waiter bookkeeping, and the probes R7 does not port.
- `SP-2@R7` gates exactly the AGPL's restart data gate inside its `PlayerCanRestart` block. That block belongs to
  `SP-2@R20` and keeps running: it still forces the answer to true and still never calls the original. Section 7 says
  how the MIT gate sits after it.
- `SP-3@R7` gates exactly the AGPL's resume poll in its tick.

**Cross-slice state.** R0 section 7.5's table gains these rows (R7's writer read the AGPL side):

| State | Set by | Read by | Rule |
| --- | --- | --- | --- |
| "hunt or hub" for the gate | the AGPL's tick (the watchdog's game mode lookup, R8) | the AGPL gate | **not carried, not needed:** the MIT gate decides from the game mode `PlayerCanRestart` runs on (7.1) |
| "the boot patch took" | HL-2 (R5, or the AGPL) | the resume poll (8.4) | carried: R7 reads the six patched bytes, as the seam does (3.3) |
| the client's last `ClientRestart` | the AGPL's `SP-1@R14` capture | LD-1 | not carried: R7 captures it itself (4.4); both observe the same calls. **R14 takes R7's capture over** (section 14) |
| the restart gate | `SP-2@R7` (MIT) | `SP-2@R20` when it moves | **R20 calls R7's query** (7.4) |
| the game instance LD-5 captures | LD-5 | the AGPL's Hunt Pass state tick (a logging-only probe, R19) | lost on purpose: that probe goes quiet once R7 lands |
| the last load manager LD-4 saw | LD-4 | the AGPL's 25 s loader dump (logging only) | lost on purpose: the dump logs that it has no manager |

**With the MIT DLL absent** the AGPL runtime does everything as today (seam 0041).

---

## 3. Facts

### 3.1 Targets

| Id | Target | RVA | Found by | Signature (Win64) | Installed |
| --- | --- | --- | --- | --- | --- |
| LD-1 | the `IArchonLoadingInterface` dispatcher (`HasFinishedLoading` for any participant) | `+0x01BA5E40` | RVA | `(object) -> u8` (a C++ `bool`) | B |
| LD-2 | `AArchonPlayerController::OnPlayerDataLoadComplete`, the native body | `+0x01CB33B0` | RVA | `(controller, success: u8)` | S |
| LD-3 | `UArchonLoadManager::LoadFailed` | `+0x01CB0010` | RVA | `(manager)` | S |
| LD-4 | `UArchonLoadManager::Begin` | `+0x01C997A0` | RVA | `(manager)` | never (9) |
| LD-5 | the progression configuration predicate | `+0x019053A0` | RVA | `(game_instance) -> u8` | S |
| LD-6 | `UProgressionComponent::CompleteLoadInternal`, the native body | `+0x01A4DE80` | RVA | `(component)` | S |
| LD-7 | `UProgressionComponent::ReadFromBackend` | `+0x01A6AAB0` | RVA | `(loader_interface)`: the component's `IArchonPlayerDataLoader` subobject, `0xB0` into the component | S |
| LD-8 | the progression configuration cache lookup | `+0x01902E60` | RVA | `(out) -> out` | never (9) |
| LD-9 | the progression component's readiness predicate | `+0x01A69560` | RVA | `(component) -> u8` | never (9) |

The AGPL checks no prologue on any of them; neither does R7 **(call)**, for HTTP-1's reason (R2 section 11): the RVAs
have held through every 1.14.7 build the project has run, and nobody recorded prologue bytes.

Functions R7 **calls** (not hooks):

| Function | RVA | Signature | Used by |
| --- | --- | --- | --- |
| `UArchonLoadManager::Begin` | `+0x01C997A0` | `(manager)` | the retry (6.4) |
| the weak object resolver (`FWeakObjectPtr` get) | `+0x0287AA20` | `(weak: *const [i32; 2]) -> object`: `{ object index, serial number }`, bounds- and serial-checked by the game, null when stale | the retry's snapshot (6.2) |
| `CompleteLoadInternal` | `+0x01A4DE80`, the **hooked entry** | `(component)` | the resume (8.3, 8.4) |

**(call)** `CompleteLoadInternal` is called at its RVA (the entry, which runs through the broker's chain), never
through `broker::original`: R7's own LD-6 wrapper must see the call to record the completion (8.2), as the AGPL's call
reached its own hook. `Begin` is called at its RVA too; with LD-4 not installed by either DLL, that is the game's
function.

### 3.2 Layout (`offsets.rs`, `pub mod r7`)

| Owner | Member | Offset | Notes |
| --- | --- | --- | --- |
| `UArchonLoadManager` | readable span | `0x70` | checked before any field |
| | owning controller | `+0x20` (pointer) | |
| | entry array data / count | `+0x48` (pointer) / `+0x50` (`i32`) | entries of `0x10` bytes |
| | `bBegun` | `+0x58` (`u8`) | `Begin` makes its timer only while this is 0 |
| entry | weak owner `{ index i32 +0x00, serial i32 +0x04 }`; loader interface pointer `+0x08` | | |
| `IArchonPlayerDataLoader` | readable span `0x28`; `bLoaded` `+0x10` (`u8`), `bLoading` `+0x11` (`u8`) | | `Load` refuses to start while `bLoading` is 1 |
| `APlayerController` | `PlayerState` `+0x228`; `Pawn` `+0x250`; `Player` `+0x298`; `NetConnection` `+0x418` | | `NetConnection` non-null = a remote player |
| `AArchonPlayerController` | `HuntSystem` `+0x768`; `LoadManager` `+0xA70` | | |
| `APlayerState` | `UniqueId` (`FUniqueNetIdRepl`) `+0x250`; its net id object pointer `+0x258` | | null = no net id |
| `UPlayer` | `PlayerController` `+0x30` (span `0x38`) | | |
| `APawn` | `PlayerState` `+0x240` (span `0x740`) | | |
| `AArchonCharacter` | `CurrentWeapon` `+0x5F0` | | |
| `UHuntSystemComponent` | `PlayerHuntId` (`FString`) `+0x180`, its `Num` `+0x188` (span `0x190`) | | `Num` counts the NUL |
| `UProgressionComponent` | loader interface `+0xB0`; configuration received `+0x3F0` (`u8`); finished setup `+0x3F8` (`u8`) (span `0x420`) | | stranded = received 1, finished 0 |
| `island_arrival_cinematic_trigger_bp_C` | span `0x4C0`; `PrimaryCineActor` `+0x328`; `SlayerActors` (`TArray`) `+0x330`, `Num` `+0x338`; `bClientStarted` `+0x3F0` (`u8`); `LoadingState` `+0x420` (`u8`); `IntroAnimSequenceRefs` (`TArray`) `+0x428`, `Num` `+0x430`; `IslandApproachLevelSequenceActor` `+0x480` | | |
| `GameModeBase.PlayerCanRestart` params | controller `+0x00`; `ReturnValue` (`bool`) `+0x08`; size `0x10` | | |
| `PlayerController.ClientRestart` params | new pawn `+0x00` | | |
| `HuntSystemComponent.DebugSetPlayerHuntID` params | `HuntID` (`FString`) `+0x00`; size `0x10` | | |
| `ArchonGameInstance.QueryCurrentHuntPassSeasonData` params | `Event` (a delegate, `0x10`) `+0x00`; size `0x10` | | zeroed = unbound |
| `UFunction` | `FunctionFlags` `+0xB0` (`u32`, R17 3.1); `FUNC_Native` = `0x400` | | |

`data` gains `LOAD_MANAGER_BEGIN = 0x01C9_97A0`, `WEAK_OBJECT_GET = 0x0287_AA20`,
`COMPLETE_LOAD_INTERNAL = 0x01A4_DE80`. `GWORLD` is R0's.

### 3.3 Whether the server has a phantom local player

`no_phantom()`: server role, and the six bytes at `+0x03E07353` read (`read_unaligned`, code) equal
`E9 D4 00 00 00 90` (HL-2's replacement, `spec-runtime-r5.md` 2.1). Read once, at the first use, and kept. This is
the answer whether R5 or the AGPL applied the patch; once R5 has landed, `headless::boot_patch_applied()` (R5 2.2) gives
the same answer and may be used instead.

### 3.4 Calls into the game

All on the game thread, from inside a detour or handler, never after detach:

- **`call` (R17 3.4):** a UFunction through the hooked `ProcessEvent` entry, so the whole SP-2 chain sees it. Used for
  the hunt pass query (8.3).
- **`process_event_original(object, function, params)`:** `broker::original(ProcessEvent)` (R0 7.2), so no handler
  of either DLL sees the call. Used for the hunt id backfill (5.3), which the AGPL also made past every hook. Same
  preconditions as `call`. If no earlier slice has added it to `ffi::reflect`, R7 does (R18's spec names it too).
- **Plain function pointers** for the RVAs of 3.1, called with arguments R7 checked first: the manager (readable `0x70`,
  live), a component (readable `0x420`, live, a `ProgressionComponent`), the weak pair (R7's own stack memory).

### 3.5 Install (RT-17 for R7)

In Init, after R0's dispatch chains and any earlier slice's entries, before `handlers::freeze()`:

- **Client:** `LD-1`; register the `ClientRestart` capture on SP-1 (4.4).
- **Server:** `LD-2`, `LD-1`, `LD-3`, `LD-5`, `LD-6`, `LD-7`; register the gate on SP-2 (7) and the poll on SP-3 (8.4).
- Both: `LD-4`, `LD-8`, `LD-9` report `[MitRuntime] install LD-<n> skipped: logging only (RT-3)` and are not hooked;
  on a client, the server-only ids are not mentioned.

Each wrapper is `broker::hook(base + rva, wrapper, order = 0, &NEXT)`, its result in the status as
`[MitRuntime] install LD-<n> ok|<reason>`. **Settled (RT-17):** no order dependency among R7's ids or with other
slices. Every target is installed before the engine runs, so the first player-data load, the first readiness check
and the first progression read all reach their wrappers. The surface's install list puts LD-2 early and LD-5..7 late on
the server; both orders are equivalent.

Every wrapper: if detaching, call `next` and return its result; `write_status_once()`; the work inside
`catch_unwind`; a panic logs `[Panic] LD-<n> ...` once and the wrapper returns `next(args)`, unless `next` has
already run, in which case it returns `next`'s result. Every game read goes through R0's checked reads; a failed read
ends that step as the section says.

---

## 4. LD-1: the loading gate (B)

The dispatcher is asked, for every participant in a load (the controller, the game state, the arrival trigger, armour,
weapons, lanterns, inventories), whether it has finished loading. The loading screen stays up while any answers false.

### 4.1 Every call

1. `ret = next(object)`. If `ret` is non-zero, return it unchanged.
2. **Server:** return 1. A dedicated or player-hosted server keeps the old compatibility answer: every participant
   counts as loaded. The first 20 forced answers in the process log
   `[Readiness] server: forced ready obj=<full name or null>`.
3. **Client:** return 1 when 4.3 (the idle arrival trigger) or 4.2 (the local controller) says so; otherwise 0.

The original runs on every call, first, and its true answer is never changed. The client never forces a participant
other than the two below: forcing armour, weapons or streamed levels released the loading screen too early.

### 4.2 The local controller (C)

True iff all of:

1. `object` is readable for `0x2A0`, live, and `is_a(object, "ArchonPlayerController")`.
2. `Player` (`+0x298`) is non-null, live, readable for `0x38`, `is_a(.., "LocalPlayer")`, and its `PlayerController`
   (`+0x30`) is `object`.
3. `object` is the captured controller (4.4).
4. The possession is stable (4.4) and gives a pawn.
5. That pawn's `CurrentWeapon` (`+0x5F0`) is non-null. **Never force while the character has no weapon**: the HUD the
   controller's readiness releases reads the weapon without a null check, and forcing without one crashed every client
   it happened to (three in one session on 16 Oct).
6. At least **20 000 ms** have passed since the captured possession began. The client's own readiness normally turns
   true about 1.5 s after `ClientRestart`; forcing earlier started gameplay before the client had built the player and
   crashed it 20 ms later. The override is a last resort for a load that is really stuck.

The first 20 forced answers log `[Readiness] client: forced ready local controller=<full name>
sinceClientRestartMs=<n>`.

### 4.3 The idle arrival trigger (C)

The Training Grounds' arrival cinematic trigger answers false for about 18 s after the player already has control,
then gives up on its own with nothing changed. R7 answers for it as soon as the possession is stable, the same end
state 15 s sooner. True iff all of:

1. `KEEP_ARRIVAL_TRIGGER_WAIT.flag` is absent (read once; the first read logs
   `[Readiness] idle arrival trigger release on (default)` or `... off (KEEP_ARRIVAL_TRIGGER_WAIT.flag)`).
2. `is_a(object, "island_arrival_cinematic_trigger_bp_C")` and `object` readable for `0x4C0`.
3. No sign of a real cinematic: `SlayerActors.Num == 0`, `IntroAnimSequenceRefs.Num == 0`, `bClientStarted == 0`,
   `LoadingState == 0`, `PrimaryCineActor` null, `IslandApproachLevelSequenceActor` null.
4. A controller is captured, live, and its possession is stable (4.4).

The first release of each trigger object logs
`[Readiness] client: released the idle arrival trigger sinceClientRestartMs=<n> (its own timeout is 18 s)`.

**(departure)** The AGPL recognised the trigger through a class object it looked up by name at most every 2 s and
compared against the last trigger it had seen; R7 checks the object's class chain by short name on each call. Same
objects, without the lookup's delay.

### 4.4 The possession capture and stability (C)

**Capture.** An SP-1 **after** handler, `order = 0`, matcher `Exact("Function Engine.PlayerController.ClientRestart")`
(the AGPL captured after the original too). It needs no feature id: it observes, and the AGPL's own capture
(`SP-1@R14`) keeps running beside it. On each call: `pawn` = the pointer at the params' `+0x00` (null when the params
are null or unreadable); a **new possession** when the controller (`object`) or the pawn differs from the captured pair.
Store the controller, the pawn, and the world (`GWorld`'s value) in every case; on a new possession also the time
(`GetTickCount64`) and reset the stability tracking. Never log.

**World change.** Before any use, if a controller or pawn is captured and `GWorld`'s value differs from the captured
world, forget the pair (the time stays, but with no controller every check below is false). After a travel or a return
to the login screen the old controller is freed; checking it would read freed memory.

**Stable** (`possession_stable(pc) -> Option<pawn>`):

1. A controller is captured and `pc` is it.
2. `pc` is live and readable for `0x2A0`; its `Player` is live, readable for `0x38`, a `LocalPlayer`, and points back
   at `pc`.
3. `pc`'s `Pawn` (`+0x250`) is non-null, equals the captured pawn when one was captured, is live and readable for
   `0x740`; the pawn's `PlayerState` (`+0x240`) is non-null, live and readable for `0x6B1`.
4. Stability tracking: when `(pc, pawn)` differs from the tracked pair, track it from now.
5. At least **3 000 ms** since the possession began and at least **750 ms** since the tracked pair was first seen.

Any failed step resets the tracking and answers `None`. The pure part (`loading.rs`) holds the timing rule and the
tracking as a state machine over values; the DLL does the reads.

---

## 5. LD-2: player data loaded (S)

`OnPlayerDataLoadComplete` runs once per controller when its load manager completes (or fails terminally); it reads
the controller's hunt id from the launch and asks to restart (spawn) the player.

### 5.1 Each call

1. `manager` = the controller's `LoadManager` (`+0xA70`) when the controller is readable for `0xA78`, else none.
   Clear every retry slot (6.3) whose manager is `manager` or whose controller is this one; if any was cleared, log
   `[PlayerData] load completed after a retry success=<0|1>`.
2. **Mark the controller's data loaded** in the gate's table (7.2), **before** the original: the original asks
   `PlayerCanRestart`, which the gate answers.
3. `next(controller, success)`.
4. If `success` is 0: done.
5. The hunt id backfill (5.2), unless `DISABLE_PLAYER_HUNTID_BACKFILL.flag` is present (checked on each call).

### 5.2 The hunt id backfill

A player routed onto a running public hunt by the DeployServer was not in the launch's `?PlayerHuntIds=` list, so the
original leaves their hunt system's `PlayerHuntId` empty, and every reward and objective check skips them. Everyone on
one server shares one hunt id, so R7 sets it from the server's own launch:

1. The controller is readable for `0x770`; `HuntSystem` (`+0x768`) is non-null and readable for `0x190`.
2. `PlayerHuntId.Num` (`+0x188`) `>= 2`: already set (an in-launch player); done.
3. `hunt_id` = `shared_hunt_id(expected_players)` (5.3). Empty: log
   `[PlayerData] hunt id empty and the launch names none; not set` and stop.
4. `find_function(class of HuntSystem, "DebugSetPlayerHuntID")`, cached once found (and re-found when the cached
   function fails `is_live_object`). Not found: stop (log once
   `[PlayerData] HuntSystemComponent.DebugSetPlayerHuntID not found; hunt ids are not backfilled`).
5. A `0x10` params block holding an `FStringParam` of `hunt_id`; `params_fit`; `process_event_original(HuntSystem,
   function, params)`. The function copies the string; it replicates to the client.
6. Log `[PlayerData] hunt id was empty: set '<hunt_id>'`.

### 5.3 `shared_hunt_id(expected_players) -> String` (pure)

The launch's sixth entry (R1 6.1, `Launch::expected_players`) is `accountId:hunterId, ...`. Take the text after its
**first** `:` up to the next `,` (or the end), verbatim (no trimming). No `:` (including `NO_EXPECTED_PLAYERS`), `None`,
or no launch: empty. Examples: `a:h1,b:h1` gives `h1`; `a:h1` gives `h1`; `a:` gives empty; `a:h1, b:h2` gives `h1`;
`a:b:c,d` gives `b:c`.

---

## 6. LD-3: a player-data load that failed (S)

The load manager calls `LoadFailed` from a one-shot timer about 60 s after `Begin` when a loader has not finished. The
game's handler then reports the failure, which on a hub reloads the map and disconnects everyone.

### 6.1 Each call, in order

1. `manager` must be readable for `0x70`; otherwise `next(manager)` and return.
2. `controller` = the manager's owning controller (`+0x20`).
3. **No net id (RT-12, 10).** If `controller` is non-null, its `PlayerState` (`+0x228`) is non-null, and that player
   state's net id object (`+0x258`) is null: log `[PlayerData] load failure dropped: its owner has no net id; the
   server keeps listening` and return **without** calling `next`.
4. Read the entry array (`+0x48`) and count (`+0x50`). Null, `count <= 0` or `count > 128`, or the array not readable
   for `count × 0x10`: `next` and return.
5. Take the snapshot (6.2).
6. `DISABLE_PLAYERDATA_LOAD_RETRY.flag` present (checked on each call): log
   `[PlayerData] retry disabled by flag; the game handles the failure` and `next`.
7. Snapshot not recoverable, or no incomplete loader: log
   `[PlayerData] load failure not recoverable incomplete=<n>; the game handles it` and `next`.
8. Claim a retry (6.3). Refused: log `[PlayerData] retry already used; the game handles the failure` and `next`.
9. Rearm (6.4). Failed: clear the slots of this manager and controller, log
   `[PlayerData] retry could not rearm; the game handles the failure` and `next`.
10. Log `[PlayerData] retry <n>/1 rearmed=<k>` and return **without** calling `next`.

### 6.2 The snapshot

For each entry `i`: the weak owner resolved through the game's resolver (3.1) with the entry's `{index, serial}`; the
loader pointer; when the loader is readable for `0x28`, its `bLoaded` and `bLoading` bytes, else both unknown.
`incomplete` = the number of entries whose `bLoaded` is 0.

**Recoverable** iff the controller is non-null, readable for `0xA78` and live, and every entry has a non-null owner, a
non-null loader, and `bLoaded` and `bLoading` each exactly 0 or 1. The decision over the read values is pure.

### 6.3 The retry budget

A table of **32** slots `{manager, controller, last failure (ms), retries}`, under a lock; the key is the pair, so a
recycled object address starts with a fresh budget.

`claim(manager, controller, now)`: refuse if either is null. Find the slot with this pair. Otherwise take, in this
order of preference, the first empty slot; else a slot whose last failure is **5 minutes** or more ago; else the slot
with the oldest last failure; reset it to the pair with 0 retries. Set its last failure to `now`. Grant iff its
retries are below **1**, then increment them; the grant returns the retry number. `clear(manager, controller)` empties
every slot matching either (a null argument matches nothing). Pure, tested on a clock.

### 6.4 Rearm

1. For each entry: a null loader fails the rearm. A loader whose `bLoaded` is 0 gets `bLoading` set to 0 (checked
   write) and is counted.
2. The count must be non-zero and equal `incomplete`; otherwise fail.
3. Write 0 to the manager's `bBegun` (`+0x58`) and call `Begin(manager)` (3.1). `Begin` replaces the one-shot timer
   (the one running now), refreshes the start time and calls each unfinished loader's `Load` again, which it refused
   while `bLoading` was 1. A late answer from the first request is harmless: a loader's completion is idempotent once
   `bLoaded` is 1.

A second failure of the same pair reaches step 8 refused, and the game's handler runs: a player is never held on an
endless loading screen.

---

## 7. `SP-2@R7`: the restart data gate (S)

The AGPL's `SP-2@R20` block answers `PlayerCanRestart` true for every controller, so any restart request spawns the
player at once. Usually nothing asks before `OnPlayerDataLoadComplete`, which asks itself. But in some hunts another
progression component finishes first and the joining player is restarted seconds before their own data loads; the
character then reaches the client with no weapon or armour, and the client crashed in the HUD once its readiness was
forced. The gate holds a hunt player's restart until LD-2 has run for them, with a 90 s fallback.

### 7.1 When it applies

All of: server role; auth mode `GlobalGameserver` (a player host keeps the old behaviour);
`DISABLE_RESTART_DATA_GATE.flag` absent (read once, at the first `PlayerCanRestart`); the game mode `PlayerCanRestart`
runs on is a **hunt**: `is_hub_class(short class name)` is false, where `is_hub_class(name)` = the name contains `City`
or `TrainingGrounds` (Ramsgate's `BPGM_City_C`, the Training Grounds' `BPGM_ArchonTrainingGrounds_C`). Then the params
are readable for `0x10`; the controller (params `+0x00`) is readable for `0x420` and live; its `NetConnection`
(`+0x418`) is non-null (a remote player).

**(departure)** The AGPL read "hunt or hub" from its watchdog's game mode lookup, which treated an unknown answer as
"hub". R7 reads the class of the object the call runs on, which is the game mode itself, so the answer is never
unknown. The class rule is the same.

### 7.2 The table and `should_hold(controller, now) -> bool` (pure)

A ring of **32** entries `{controller, first held (ms), loaded}`, under a lock. `entry_for(controller)`: the entry with
that controller, else the next ring slot (overwritten, with `first held` 0 and `loaded` false).

- `mark_loaded(controller)` (LD-2, 5.1): `entry_for(controller).loaded = true`; a null controller is ignored.
- `should_hold`: loaded: false. `first held` 0: set it to `now` and log
  `[PlayerData] restart held until the player's data loads: controller=<ptr>`. `now − first held >= 90 000`: set
  `loaded`, log `[PlayerData] data still not loaded after 90 s; restart allowed: controller=<ptr>`, false. Otherwise
  true.

### 7.3 The handler

An SP-2 **after** handler, `order = 0`, matcher `Exact("Function Engine.GameModeBase.PlayerCanRestart")`, registered
only while `SP-2@R20` is not owned (`registry::owns`, at registration). When 7.1 applies and `should_hold` is true:
write 0 to the `ReturnValue` byte (params `+0x08`, checked write).

**Why after (call).** The AGPL ran its gate inside the `SP-2@R20` block, after that block's connection fix-ups and
before its force, then returned false without the original. With `SP-2@R7` owned, that block still runs every time:
fix-ups, force to true, no original. R7's after handler then turns the answer back to false for a held player. Every
side effect the AGPL had on a held call still happens, in the same order, and the caller reads false, as before. A
before handler that absorbed would have skipped the block's fix-ups on held calls.

### 7.4 What R20 consumes

`loading::restart_gate::hold(controller: usize) -> bool`: 7.1's conditions and `should_hold`, for the DLL's game
thread. When R20 moves the `PlayerCanRestart` force, its handler calls this first and answers false (without the
original) when it is true, else forces true. R7's own after handler is then not registered (7.3), so the gate runs once.

---

## 8. Progression: the configuration query and parked components (S)

### 8.1 What goes wrong without it

A player's progression component reads from the backend in `ReadFromBackend` (LD-7). If the progression configuration
is not loaded yet (the predicate, LD-5, answers false), it parks itself: it registers a callback by a name no reflected
function carries, so nothing ever wakes it, and its loader stays unfinished until the load manager's 60 s timeout
(LD-3). Nothing on a server issues the configuration query by itself. A hub's own boot used to hide this; without the
phantom local player (HL-2), every first join of a hunt or the Training Grounds parked and waited 61 s, and a fresh
install's player was sent back to the menu 120 s after Ramsgate loaded. R7 issues the query at boot (8.3) and wakes
parked components once the configuration is there (8.3 at the predicate, 8.4 from the tick).

### 8.2 The bookkeeping (pure state, under one lock)

- `waiters`: components parked since the last resume, unique, in order. `wait_since`: when the first of the current
  waiters parked.
- `candidates`: components that ever parked and have not completed since; unique; at most **1024** (one more sets
  `overflow`, permanently).
- `completed`: components `CompleteLoadInternal` ran for; when it would exceed **4096**, it is emptied first.
- `waiter_count`: an atomic mirror of `waiters.len()` the tick reads without the lock.

LD-6 (`CompleteLoadInternal`, each call): the first call in the process logs `[Progression] CompleteLoadInternal runs`;
for a non-null component: insert into `completed`, remove from `waiters` and `candidates`. Then `next(component)`.

LD-7 (`ReadFromBackend`, each call): set this thread's **predicate answer** to "not asked"; `next(interface)`. Then,
only if the predicate was asked on this thread during that call and answered false (the WAIT branch; every other exit
leaves it "not asked" or true), and `interface` is non-null: `component = interface − 0xB0`; remove it from `completed`
(a new waiter at a recycled address is not complete); if `waiters` is empty, `wait_since = now`; add it to `waiters` and
to `candidates` (unique; or `overflow`); log `[Progression] parked component=<ptr> waiters=<n>`.

The thread-local predicate answer is written by LD-5 (8.3) on every call: true or false.

### 8.3 LD-5: the configuration predicate

Each call:

1. `loaded = next(game_instance)`.
2. **Resume stranded components**, when `loaded`, `game_instance` is non-null, and fewer than **8** resumes have
   happened in the process: for each component of a copy of `candidates` (or, when `overflow` is set, every live object
   in the object array):
   - not readable for `0x420`, not live, a default object (object flag `0x10`), or not `is_a(..,
     "ProgressionComponent")`: remove it from `candidates` (the walk just skips it);
   - in `completed`: skip;
   - configuration received (`+0x3F0`) is 0 or finished setup (`+0x3F8`) is non-zero: skip (not stranded);
   - otherwise count a resume (stop once the count reaches 8), log
     `[Progression] stranded component=<ptr>: calling CompleteLoadInternal`, and call `CompleteLoadInternal(component)`
     at its entry (3.1).
3. Capture `game_instance` (an atomic) when non-null.
4. **The query**, once in the process: at the first call whose `game_instance` is non-null, log
   `[Progression] querying the hunt pass season data instance=<ptr>`, then call the game instance's
   `QueryCurrentHuntPassSeasonData` (`find_function(class, "QueryCurrentHuntPassSeasonData")` on its class chain,
   class `ArchonGameInstance`) through `call` (3.4) with a zeroed `0x10` params block: an unbound delegate, so no
   callback fires, but the query that fills the configuration is issued. For the duration of the call set `FUNC_Native`
   (`0x400`) in the function's `FunctionFlags` and restore the previous value after (checked writes; the dump's own
   call to this function does the same). Then log `[Progression] query returned`. Not found: log once
   `[Progression] ArchonGameInstance.QueryCurrentHuntPassSeasonData not found; the configuration is not queried`.
5. The first true answer logs `[Progression] configuration loaded`; the first false answer logs
   `[Progression] configuration not loaded yet: progression reads will park`.
6. Set this thread's predicate answer to `loaded`; return `loaded`.

**(departure)** The AGPL issued the query on its first predicate call when the game instance was already captured, and
otherwise on its second call whatever it held (a client-side timing kept for the client, where LD-5 is not installed
at all). R7 issues it at the first call that carries a game instance; on a server that is the first call, so the
query goes out at boot, before anyone joins. Without that, a hunt or the Training Grounds (which check the predicate
once at boot) parked the first real player and made them wait the full minute.

### 8.4 `SP-3@R7`: the resume poll

An SP-3 **after** handler, `order = 100` (after the engine tick and after any replication graph drive a later slice
registers with a smaller `order`; the AGPL polled after its graph drive too). Each tick:

1. `waiter_count == 0`: return.
2. **Enabled** iff `no_phantom()` (3.3), decided at the first poll that has a waiter and logged
   `[Progression] resume of parked components on (no phantom local player)` or
   `... off (this server has a phantom local player, whose own load covers it)`. Disabled: empty `waiters` and return.
3. At most every **200 ms** (from the last check), else return.
4. The captured game instance must be non-null and live; else return.
5. Ask the predicate through `broker::original(LD-5 target)`, so 8.3's resume and query do not run from the tick.
6. False: if `wait_since` is set and more than **90 000 ms** ago, log
   `[Progression] configuration still not loaded after 90 s; leaving <n> parked component(s) to the load-failure retry`
   and empty `waiters`. Return.
7. True: take all `waiters` (emptying it). For each: in `completed`: log
   `[Progression] component=<ptr> already completed`; not readable for `0x420`, not live, a default object or not a
   `ProgressionComponent`: log `[Progression] component=<ptr> is gone; skipped`; otherwise log
   `[Progression] configuration loaded <ms> ms after the first park: CompleteLoadInternal component=<ptr>` and call
   `CompleteLoadInternal(component)` at its entry.

The predicate is a cheap check of the configuration map; the poll asks it at most five times a second, only while
something is parked.

---

## 9. LD-4, LD-8, LD-9: owned, not installed

| Id | What the AGPL did | Why it goes |
| --- | --- | --- |
| LD-4 | logged the load manager's owner identity chain on each `Begin`, kept the manager for a 25 s dump | logging only (RT-3) |
| LD-8 | logged up to 40 cache lookups and dumped the found interface's virtual table; with `CALL_IFACE_QUERY.flag`, called a guessed slot | logging, and an experiment |
| LD-9 | logged the readiness predicate and four flag bytes; dumped feature flags and the loaders; `FORCE_PROGRESSION_READY.flag` wrote the four bytes | logging, and an experiment the query (8.3) made unnecessary |

Owning them makes the AGPL stop installing them; R7 installs nothing in their place. A probe returns only for a live
bug (RT-3). The surface's LD-4 `B` role and LD-8/9 `S` roles are recorded in `TARGETS` for the table test.

---

## 10. Flags (RT-14 for R7) and RT-12

| Flag | Kept? | Meaning in the MIT runtime |
| --- | --- | --- |
| `KEEP_ARRIVAL_TRIGGER_WAIT` | kept | client, read once: no idle-trigger release (4.3) |
| `DISABLE_PLAYERDATA_LOAD_RETRY` | kept | server, each failure: no retry; the game handles every failure (6.1) |
| `DISABLE_RESTART_DATA_GATE` | kept | server, read once: no gate (7.1) |
| `DISABLE_PLAYER_HUNTID_BACKFILL` | kept, **moved from R20** | server, each load: no backfill (5.1) |
| `KEEP_PHANTOM_LOAD_FAILURE` | **dropped (call)** | the drop of 6.1 step 3 always applies (RT-12 below) |
| `PROGRESSION_WAIT_RESUME` | **dropped (call)** | it enabled the poll with a phantom; the poll follows `no_phantom()` alone (8.4) |
| `FORCE_PROGRESSION_CONFIG_READY` | **dropped** | an experiment that forced the predicate true; the query (8.3) replaced it |
| `FORCE_PROGRESSION_READY` | **dropped** | LD-9 is not installed (9) |
| `CALL_IFACE_QUERY` | **dropped** | LD-8 is not installed (9) |
| `CALL_FIND_ACTIVE_HUNTPASS` | **dropped** | an experiment that called `FindActiveHuntPassRow` from the predicate; superseded by the query |
| `FORCE_ACTIVE_SEASON` | **dropped** | its code sat in a probe that is never installed on 1.14.7 |

Not R7's and not read: `DIAG_NATURAL` (R12's diagnostic switch, which made the AGPL's LD-1 and `PlayerCanRestart`
answer naturally) and `LEGACY_LIFECYCLE_WALK` (which made the AGPL's stranded-component search walk every object). R7
honours neither **(call)**: both are diagnostics (RT-3).

**RT-12, settled: the drop no longer fires, and stays.** The drop of 6.1 step 3 was written for the hub's phantom local
player, whose player state had no net id, so its data load always timed out and the game's handler then reloaded the hub
two minutes after every start. Since HL-2's boot patch (every server role since 0.2.110) there is no phantom: every
controller with a load manager belongs to a real player, and a real player's player state carries a net id from login.
So in normal operation the drop never fires. It stays, without its flag: it costs one read, and if HL-2's byte check
ever fails on a new build, the phantom returns and the drop is again what keeps the hub alive. The one-shot retry
(6.2 to 6.4) is needed either way: it is what repairs a real player's stuck loader (the hunt-to-Ramsgate case).

RT-13, RT-15 and RT-16 are not R7's (R1, R1, R4).

---

## 11. Logging

Tags `[Readiness]`, `[PlayerData]`, `[Progression]` (R0 call 6), texts as given in sections 4 to 8. Pointers are
written `0x` and 16 lowercase hex digits. Budgets: LD-1's forced-answer lines 20 per kind per process; every other line
once per event as described. Nothing R7 logs carries an account id or a key; the hunt id (5.2) is a hunt's id, not a
secret.

Production profile (`PRODUCTION_LOG_PROFILE.flag`, R0 4.2): `[Progression] parked ...` and `[Readiness] server: forced
ready ...` are production-quiet (skipped on a server with the flag); every other R7 line is kept, since each marks a
recovery or a decision an operator needs. **(call)** The AGPL had no rule for these lines.

---

## 12. Tests

### 12.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`), pure, on every platform:

- **Retry budget:** first claim granted (retry 1), second refused; another pair granted; a 33rd pair takes an empty slot
  while one exists, else a slot 5 min stale (exactly 300 000 ms counts), else the oldest; a reused slot starts at 0;
  null manager or controller refused; `clear` by manager, by controller, with a null argument.
- **Rearm decision:** loaded/loading snapshots: all loaded (no rearm); one stuck at `0/1` (rearm one); a value 2 (not
  recoverable); a null owner or loader (not recoverable); the rearmed count must equal `incomplete`.
- **Gate:** hold at first ask (logs once), still held at 89 999 ms, released at 90 000 ms and stays released;
  `mark_loaded` before and during a hold; the ring overwrites the 33rd controller's slot; `is_hub_class` on
  `BPGM_City_C`, `BPGM_ArchonTrainingGrounds_C` (true) and a hunt game mode name, an empty name (false).
- **Hunt id:** every example of 5.3, plus `NO_EXPECTED_PLAYERS`, `None`, an empty string, a leading `:`.
- **Progression bookkeeping:** park, park again (unique), complete removes from both lists, a recycled address
  re-parks; 1024 candidates then overflow; `completed` cleared at 4097; `wait_since` set only when `waiters` was empty;
  the 8-resume budget; the poll's 200 ms throttle and 90 s give-up on a fake clock.
- **Possession:** the stability state machine: 2 999 ms after possession not stable, 3 000 ms and 750 ms of the same
  pair stable; a changed pawn restarts the 750 ms; a world change forgets the pair; the 20 000 ms controller rule and
  the weapon rule as predicates over values.
- **Idle trigger:** each of the six "real cinematic" signs alone keeps the answer false.
- **Log texts:** every line of sections 4 to 8 formats as written, with no field that is not listed.
- **Registry:** with `"R7"` added to the `LANDED` of the build under test, `owns` is true for `LD-1`..`LD-9`, `SP-2@R7`,
  `SP-3@R7` and still false for `SP-2@R20`; the count grows by 11. The offsets test finds a `TARGETS` line for each of
  the nine LD ids.

DLL (`cargo test -p rustic-isles-runtime`, Windows), against fakes in test memory, never the game's RVAs:

- LD-3: a fake manager with two entries (one loader `1/0`, one `0/1`), a fake resolver and a fake `Begin` injected as
  function pointers: one retry clears `bLoading` and `bBegun` and calls `Begin` once; a second failure calls `next`; a
  player state with a null net id is dropped without `next`; the flag path.
- LD-2: the slot cleared, the gate marked before `next` runs (the fake `next` checks it), the backfill skipped for
  `Num >= 2`.
- The gate handler on a fake params block: held writes 0 at `+0x08` after a fake chain wrote 1; a hub class and a
  local controller (null `NetConnection`) are not held.
- LD-7 and LD-5 together: a fake `next` for `ReadFromBackend` that calls the LD-5 wrapper (answering false) parks the
  component; one that does not ask leaves nothing parked.
- Every wrapper through the broker on a test function, as R0's broker test does.

### 12.2 Never in CI

The real targets, the load manager's timer, the backend's configuration answer, the client's loading screen, both
DLLs in the game.

### 12.3 The maintainer's play-test (surface section 8, R7 row)

"Fresh install joins Ramsgate and stays past 120 s; a hunt loads in seconds, not a minute; a restarted hunt; a hub runs
10 min." Build and deploy both DLLs as R0 11.1 step 1 (R2's redirect working, 1.2). Then:

1. **Status.** Every server's `rustic_isles_dll_port<N>.log` shows `install LD-1 ok`, `LD-2`, `LD-3`, `LD-5`, `LD-6`,
   `LD-7` ok, `install LD-4|LD-8|LD-9 skipped: logging only (RT-3)`, and the AGPL's seam line with nine more hooks
   left to it than before R7 (a client's: two more, LD-1 and LD-4). The client's log shows `install LD-1 ok`.
2. **Boot query.** Start Ramsgate and a hunt through the Director. Each server logs
   `[Progression] querying the hunt pass season data` within seconds of boot, before anyone joins, then
   `[Progression] configuration loaded` at the next predicate call; the backend's log shows the configuration request.
3. **Fresh install, Ramsgate past 120 s.** On a PC (or a fresh game profile) that has never run Dainty Lass, log in and
   enter Ramsgate; stay 3 minutes. No return to the menu; Ramsgate's log has no `[PlayerData] retry` line and no
   `load failure` line.
4. **A hunt loads in seconds.** Start a hunt from Ramsgate. Time the loading screen: seconds, not a minute. The hunt's
   log may show `[Progression] parked ...` followed within a second by `configuration loaded ... CompleteLoadInternal`,
   never a 60 s gap; if `[PlayerData] restart held ...` appears, it is followed by the player spawning (with a weapon)
   once the data loads.
5. **The Training Grounds.** Enter the Training Grounds: the client's log shows
   `[Readiness] client: released the idle arrival trigger sinceClientRestartMs=<about 3000>`; control comes about 15 s
   sooner than before R7.
6. **A restarted hunt.** Finish or abandon a hunt and start another on the same server (or rejoin a running public hunt
   from a second account): the late joiner's log line `[PlayerData] hunt id was empty: set '<id>'` appears on the
   server, and the joiner's kill XP and rewards are granted at the end.
7. **A hub for 10 minutes.** Leave Ramsgate running with one player for 10 min, join and leave from a second client
   twice. No map reload, nobody disconnected, no `[Panic]`.
8. **Without the MIT DLL**, steps 2 to 4 behave as before (the AGPL's own `[HuntPassQuery]` and `[ProgWait]` lines).

Only CI passed until these steps are done.

---

## 13. Calls, departures, ambiguities and settled questions

Calls:

1. R7 depends on R2 for behaviour only, not code (1.2).
2. No prologue checks on the LD targets (3.1).
3. `CompleteLoadInternal` and `Begin` are called at their entries; the predicate from the tick and the hunt id backfill
   go past the chains (3.1, 3.4, 8.4).
4. LD-4, LD-8, LD-9 are owned and not installed (9).
5. The restart gate is an SP-2 **after** handler, registered only while R20 does not own `SP-2@R20`; R20 calls
   `restart_gate::hold` (7.3, 7.4).
6. The client possession capture is R7's own observing handler, with no feature id; R14 takes it over (4.4, 14).
7. `KEEP_PHANTOM_LOAD_FAILURE` and `PROGRESSION_WAIT_RESUME` dropped; `DIAG_NATURAL` and `LEGACY_LIFECYCLE_WALK` not
   honoured; `DISABLE_PLAYER_HUNTID_BACKFILL` moved to R7 (10).
8. Two R7 lines are production-quiet (11).
9. The resume poll's `order = 100` (8.4).

Departures from the AGPL:

1. The arrival trigger is recognised by its class chain on each call, not a throttled class lookup (4.3).
2. "Hunt or hub" for the gate comes from the game mode the call runs on, never unknown (7.1).
3. The query goes out at the first predicate call that carries a game instance (8.3).
4. LD-1's log lines drop the caller's return address and the per-participant `PRESERVE_FALSE` lines (logging only).
5. Dropped with their probes (RT-3): the per-entry load-failure dump and the bounty flag dump in LD-3, the server
   arrival-trigger probe in LD-1, the client weapon-gap log (patch 0013, marked droppable), the identity log (LD-4).

Ambiguities:

- The surface gives LD-9's flag as setting "the four server flags"; it is a probe R7 does not install (9), so the
  question of what those bytes mean is left to whoever ports a probe for a live bug.
- `QueryCurrentHuntPassSeasonData`'s callback type is a delegate whose layout R7 never reads; a zeroed `0x10` block is
  an unbound delegate in UE 4.26, which is what the AGPL passed (8.3).

Settled here:

- **RT-12:** the phantom drop no longer fires in normal operation; kept without its flag; the retry stays (10).
- **RT-14 for R7:** four flags kept, seven dropped (10).
- **RT-17 for R7:** no order dependency (3.5).
- **RT-18 for R7:** two `Exact` matchers: `Function Engine.PlayerController.ClientRestart` (SP-1, 4.4) and
  `Function Engine.GameModeBase.PlayerCanRestart` (SP-2, 7.3). SP-3 matches nothing.
- **Patch 0051:** not needed; seam 0041 already stands the AGPL down for every R7 id (2).
- RT-13, RT-15, RT-16 are not R7's.

Open for the maintainer: none. Calls 5 and 7 are the ones worth a look.

---

## 14. For later slices

- **R20** (`SP-2@R20`, the `PlayerCanRestart` force): call `restart_gate::hold(controller)` first; on true answer false
  without the original. Once R20 owns `SP-2@R20`, R7's after handler is not registered (7.3, 7.4).
- **R14** (`SP-1@R14`, input activation and HUD lifecycle around `ClientRestart`): the client possession capture and
  `possession_stable` (4.4) are R7's module. R14 consumes them instead of capturing a second time, and may move them
  into its own module if it needs more state per possession (input and HUD flags reset on a new possession); the
  constants (3 000 ms settle, 750 ms stable chain, 20 000 ms readiness force) stay as they are. R14's progression HUD
  refresh can read whether the controller's data loaded through R7's gate table (`loaded` per controller, server side
  only); on the client it has no such signal from R7.
- **R12** decides `DIAG_NATURAL`; if it keeps the switch, R7's LD-1 and gate gain a check for it then.

---

## 15. Size

Rust without tests about 750 lines: the core module about 300 (the two tables, the bookkeeping, the hunt id, the hub
rule, the possession state machine, the trigger predicate, log texts), the `ffi` module about 450 (six wrappers, three
handlers, the reads, the two UFunction calls, the retry's writes and calls). Offsets and registry about 50. Tests about
550. The AGPL code it replaces is about 1 800 lines with its probes.
