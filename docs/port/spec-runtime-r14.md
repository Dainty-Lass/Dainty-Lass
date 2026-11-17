<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R14, the player role (omnicell) (RD27, plan item 11)

> Since R21 (`spec-runtime-r21.md`) the AGPL runtime is gone; `rustic-isles-runtime.dll` is the only runtime. This
> slice's play-test runs on the MIT DLL alone, as that spec's section 9 adapts it: skip every step that builds or
> patches the AGPL runtime, injects both DLLs, counts `[MitSeam]` lines or runs without the MIT DLL.

> The play-test of 2026-11-16 added `spec-runtime-r14-omnicell-fix.md`: the client handlers run only on the game thread,
> a sheathe watch re-applies the role after the glider or the end of combat, and R13's route (its 6.7) makes the Frank
> drone and any other actor a role owns globally relevant, so the omnicell works in every combat area.

R14 is the omnicell slice of the MIT game runtime (`spec-runtime-surface.md` section 3.9, section 4 and section 8's
R14 row, approved 2026-11-13 with rulings RT-1 to RT-18 in `rulings-2026-11-13.md`). An omnicell is a player's
`AArchonPlayerRole` equipment actor: its charge ring and ability on the HUD, the lantern ability (tap V) and, for
Tempest, the perfect-dodge stacks. On this build the role's construction is fragile on both sides of the wire, and
the client's first input and HUD lifecycle after a travel needs help to finish. R14 owns the hooks **PR-1** and
**RG-3** (moved here from R12 by the ruling of 2026-11-13) and the feature ids **`SP-1@R14`**, **`SP-2@R14`** and
**`SP-3@R14`**:

- **server:** the role application's validity record and its bounded retry; a post-activation refresh that asks the
  engine to send a role's final state; the Tempest modifier repair; the ability activation route on
  `ServerTryActivateAbility`; the role purge on graph removal (RG-3);
- **client:** the same application and retry for the owning client; the input activation after `ClientRestart` and
  `OnInputModeChanged`; the role lifecycle replay; the movement repair; the airship HUD hide and restore.

An implementer builds from this page, `spec-runtime-r0.md`, `spec-runtime-r7.md` section 4.4, `spec-runtime-r9.md`
section 8, section 3 of `spec-runtime-r17.md`, `spec-runtime-r18.md`'s `process_event_original` and the surface page
alone, and never reads `restricted/`. What crossed is facts: RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7, CL
647472), instruction bytes and `.pdata` entries (each checked against the maintainer's copy of that executable on
2026-11-13), engine and game class, function and member names and offsets from the maintainer's Dumper-7 dump of this
build, parameter block layouts, flag names and behaviour. The AGPL side stands down through seam patch 0041 alone
(section 13); R14's reserved patch number 0058 is not used.

Conventions are R0's: an RVA is `+0x0XXXXXXX` from the executable's image base; **C** client, **S** server, **B** both.
**(call)** marks a decision this page makes; **(ambiguity)** something the source did not settle; **(departure)** a
place the MIT runtime deliberately behaves differently from the AGPL one. All three are collected in section 17.

---

## 1. Scope

| R14 owns | Role | Section |
| --- | --- | --- |
| PR-1: `ApplyPlayerRole`, its validity record and the bounded retry | B | 5 |
| `SP-3@R14`: the server's once-a-second pump (retries, the post-activation refresh, the Tempest modifier repair) | S | 6, 7, 8 |
| RG-3: `RouteRemoveNetworkActor`, the purge of R14's role tables | S | 9 |
| `SP-2@R14`: the ability activation route on `ServerTryActivateAbility(WithEventData)` | S | 10 |
| `SP-1@R14`: the client's possession reset, input activation, role lifecycle replay, movement repair, airship HUD | C | 11, 12 |
| RT-14, RT-17 and RT-18 for R14 | | 14, 4.4, 15 |

**What the surface's R14 row named and this page does not port (call).** The surface sized R14 at about 3 000 C++ lines
and allowed a split. Read against the AGPL source, most of that is diagnostics or code that does nothing on 1.14.7:

- **The progression HUD refresh** (the weapon-XP bar's placeholder after `ClientRestart`): its tick function is
  defined but never called, and nothing arms it; the AGPL's own `ClientRestart` line reports it disabled. Not ported.
- **Per-connection role routing.** The AGPL keeps a table of (role, connection) pairs, but on 1.14.7 every route
  call only records the pair: the role's one global route is RG-2's (R13: the role is made always relevant when the
  graph first files it), the per-connection node insert runs only under an A/B flag, and the channel "bootstrap" it
  calls opens and pushes nothing on this build (R9 8.3: no slice creates or pushes a channel by hand). What the
  table still drives is the post-activation refresh, which R14 keeps per role (7).
- **Diagnostics (RT-3):** the role channel churn log, the remote pawn role census, the controller split detector and
  its hardware watchpoints, the Frank drone traces, the Tempest charge trace, the lantern input trace, the
  arrival census, the NPC collision probe, the HUD visibility trace, the axis-event block (whose movement writes are
  all commented out; it only logs that they are disabled), the movement diagnostic line.

So **R14 is not split (call)**: about 1 300 lines of Rust without tests (section 19). If the maintainer still wants
two halves, the only clean cut is by dispatch point: R14a = PR-1, RG-3, `SP-2@R14`, `SP-3@R14` (server) and R14b =
`SP-1@R14` (client), with R14b depending on R14a, because PR-1 is one id for both roles and the client's retries
are pumped by `SP-1@R14`. Recommended: one slice.

Not R14: the role's always-relevant routing and the relevance pass that re-asserts it (RG-2 and `SP-3@R13`, R13); the
client possession capture itself (R7 4.4, which R14 reads); `ClientRestart`'s server side, `PlayerCanRestart` and
possession acknowledgement (R20); the escalation relic screen (R18, and R12's guards).

---

## 2. Where it goes

### 2.1 Files

| Crate | Path | Holds |
| --- | --- | --- |
| `rustic-isles-runtime-core` | `src/player_role.rs` (a folder `src/player_role/` past ~400 lines) | everything pure: the retry table and its timing (5.3), the role table and the refresh state machine (7), the Tempest repair decisions (8), the input activation decisions and the possession state (11), the airship HUD state machine (12), the movement repair decisions (11.6), the line builders, budgets and the flag table |
| `rustic-isles-runtime` | `src/ffi/player_role/` (`mod.rs`, `apply.rs`, `server.rs`, `client.rs`, `hud.rs`) | the PR-1 and RG-3 detours, the handlers, every game read, write and call; under `ffi` because all of it is FFI (R0 2, RT-1) |

`core/src/lib.rs` gains `pub mod player_role;`, `ffi/mod.rs` gains `pub mod player_role;`.

### 2.2 Shared lines R14 adds

| File | Edit |
| --- | --- |
| `core/src/registry.rs` | `LANDED` gains `"R14"` **only once it holds `"R7"`, `"R13"` and `"R15"`** (13.3). The `RG-3` line's slice is `"R14"` (if R12's implementer has not already changed it, R14 does, with the `surface_ids` test's RG rule: "RG-2 → R13, RG-3 → R14, the rest → R12"). PR-1, `SP-1@R14`, `SP-2@R14`, `SP-3@R14` exist already. One new test (16.1) |
| `core/src/offsets.rs` `TARGETS` | two lines, PR-1 and RG-3, both `FoundBy::RvaAndBytes` (18.1) |
| `core/src/offsets.rs`, a new `pub mod r14` | the data RVAs and layout of 18.2 |
| `src/ffi/dispatch.rs` (R0) | `pub fn depth() -> u32`: the current thread's SP-1/SP-2 nesting depth (R0's depth counter), if no slice has added it |
| R7's possession module (`ffi/loading.rs` and its core part) | the three read-only accessors of 3.1, if R7's implementer has not added them |
| `src/lifecycle.rs` | before `handlers::freeze()`: `ffi::player_role::register()` (the handlers for the role). In the install list: PR-1 and RG-3 at the positions of 4.4 |

`owned_count()` then counts 5 more ids (PR-1, RG-3, `SP-1@R14`, `SP-2@R14`, `SP-3@R14`).

### 2.3 What is pure and what is Windows-only

Pure (core, `cargo test` on every platform): every decision and state machine on this page over values read through
R0's `Memory` trait (a fake memory and a fake clock in tests), every log text, the budgets, the flag table. Windows-only
(DLL crate): the two detours, the handlers' reads and writes, the calls into the game. Nothing in R14 runs against the
real executable in CI (surface 9).

---

## 3. What R14 consumes from other slices

| From | What | Used in |
| --- | --- | --- |
| R0 | `broker::hook`, `broker::original`, `NextCell`; `handlers::register_pe`, `register_tick` (`Point::Sp1`, `Sp2`, `Sp3`); `ffi::memory` (`readable`, `read`, `verify_entry`); `is_live_object`; `sane_pointer_array`; `engine` names; `flags::flag`; the log; the detaching flag; `lifecycle::write_status_once`; `dispatch::depth()` (2.2) | everywhere |
| R7 4.4 | the client possession capture: the accessors of 3.1 | 11, 12 |
| R9 8.1, 8.3 | `net_driver()`, `client_connections(driver)` | 7.3 |
| R17 3 (the reflection kit) | `is_a`, `find_class`, `find_function`, `params_fit`, `call`, `write<T>`, `FStringParam` | 5, 7, 8, 10, 11, 12 |
| R18 / R7 | `ffi::reflect::process_event_original(object, function, params)`: `ProcessEvent` past every handler of both DLLs | 7, 8, 11, 12 |
| R12 | nothing called. Its identity tests are not needed: R14 no longer routes per connection (1) | — |
| R13 | nothing called; a landing rule (13.3) | — |

### 3.1 What R14 needs from R7's possession capture

R7 (4.4) captures the client's controller and pawn at every `ClientRestart` (an SP-1 after handler, `order = 0`) and
answers `possession_stable(pc)`. R7's section 14 lets R14 consume it. R14 needs three read-only accessors, which R14
adds to R7's module if R7's implementer has not:

- `captured() -> Option<Captured>`: `{ pc, pawn, since_ms, generation }` after R7's world-change rule ran (a world
  change forgets the pair, so this is `None` then). `since_ms` is the `GetTickCount64` of the possession's start.
- `generation`: a `u64` R7 increments on every **new possession** (controller or pawn differs from the captured
  pair); a repeated `ClientRestart` for the same pair does not change it.
- `chain_for(pc, expected_pawn) -> Option<pawn>`: R7's stability steps 1 to 3 without the timing (step 4 and 5): `pc`
  is the captured controller, live, readable for `0x2A0`; its `Player` (`+0x298`) live, readable for `0x38`, a
  `LocalPlayer`, whose `PlayerController` (`+0x30`) is `pc`; `pc`'s `Pawn` (`+0x250`) non-null, equal to
  `expected_pawn` when one is given, live, readable for `0x740`; the pawn's `PlayerState` (`+0x240`) non-null, live,
  readable for `0x6B1`.

`possession_stable(pc)` (R7: the chain, at least 3 000 ms since the possession began and 750 ms of the same pair) is
used as R7 defines it. No R14 code captures a second time.

---

## 4. Common rules for every R14 hook and handler

### 4.1 Before anything else

1. **Detaching** (R0 3.3): the detours call `next` with their arguments and return; handlers return (`Continue` for a
   before handler).
2. `lifecycle::write_status_once()` on first use.
3. The work runs inside `catch_unwind`; a panic logs `[Panic] <id> ...` once (R0 2); a detour then calls `next` with
   its arguments unless it already did, a before handler returns `Continue`.

Every game read goes through R0's checked reads; a failed read ends that piece of work as the section says. No R14 code
catches a hardware fault **(departure)**: the AGPL wrapped the retry's native calls, the client pump and the HUD calls
in structured exception handling after three travel crashes in July. Each of those crashes was a stale pointer kept
across a travel; R14 instead never calls into the game with a pointer it has not checked live (`is_live_object`) in
the same call, and every cached pointer is dropped on a world change (R7's rule, and 12.1's).

### 4.2 Calls into the game

All on the game thread, from inside a detour or handler, never after detach:

- **`process_event_original`** for every UFunction R14 calls that the AGPL called past its own hooks: the input
  cascade (11.4), the lifecycle replay (11.5), the HUD (12), the refresh (7.4), the Tempest repair (8). Parameter
  blocks are R14's own zeroed buffers sized by R17's rule (`max(round_up(size, 8), 16)`), checked with `params_fit`.
- **R17's `call`** (the hooked entry) for `ServerSetInputPressed` and `ServerSetInputReleased` (10), which the AGPL
  made through the component's ordinary `ProcessEvent`.
- **The PR-1 entry** `+0x01B92B90` (through the broker, so R14's own wrapper sees it) when the client's input cascade
  re-applies the role (11.4); **`broker::original(PR-1)`** for the retry (5.4), which the AGPL also made past its hook.
- **Plain function pointers** for the getters of 5.1 and `InternalTryActivateAbility` (10), with arguments checked
  first.

UFunctions are found by `find_function` on the object's class chain, by the names this page gives, once per process
and cached (a class is never collected; the cache is keyed by the class pointer the function was found on).

### 4.3 Lines

Tags: `[MitPlayerRole]` (5 to 10), `[MitInput]` (11), `[MitHud]` (12). Pointers print as `0x` and 16 lowercase hex
digits (R7's form); `null` for 0. A line that can repeat has a budget per process; the `B+1`-th becomes `<tag> <kind>:
further lines suppressed`. Production profile (`PRODUCTION_LOG_PROFILE.flag`, R0 4.2): the lines marked *quiet* are
skipped on a server with the flag **(call)**. Nothing R14 logs carries an account id or a key.

### 4.4 Install order and registration (RT-17 for R14)

**Settled:** no R14 id depends on another id's install order; every target must be hooked before the first player
joins, which every install in `DllMain` is. The MIT build keeps the AGPL's positions (surface 3.11): client PR-1 after
TR-1 and before HTTP-1; server PR-1 after ESC-1 and before SRV-1, RG-3 after RG-2's position and before RG-4. Each
gives `[MitRuntime] install PR-1|RG-3 ok|<reason>` through R0's status. RG-3 is server-only; a client does not mention
it.

Handlers, registered before `handlers::freeze()`:

| Point | Kind | `order` | Matcher (RT-18) | Section |
| --- | --- | --- | --- | --- |
| SP-1 | before | 0 | `(FunctionName, "OnAirshipStateUpdate")` | 12.1 |
| SP-1 | before | 50 | `(Prefix, "")` (every call) | 11.2, the pump |
| SP-1 | after | 10 | `(Exact, "Function Engine.PlayerController.ClientRestart")` | 11.1 |
| SP-1 | after | 20 | `(Exact, "Function BP_PlayerCharacter.BP_PlayerCharacter_C.OnInputModeChanged")` | 11.3, 11.6 |
| SP-1 | after | 30 | the HUD names of 12.2 | 12.2 |
| SP-2 | before | 0 | `(Exact, "Function GameplayAbilities.AbilitySystemComponent.ServerTryActivateAbility")`, `(Exact, "Function GameplayAbilities.AbilitySystemComponent.ServerTryActivateAbilityWithEventData")` | 10 |
| SP-3 | after | 600 | none (a tick) | 6 |

**Position on the chains.** The AGPL ran the airship capture and the pump before the original, the rest after; the MIT
handlers keep those sides. As MIT handlers they run before (before handlers) or after (after handlers) every AGPL block
of the same call; R14's AGPL blocks are all off once R14 lands (13), and no other slice's handler reads or writes what
R14's do, so the move changes nothing. The ClientRestart handler's `order = 10` puts it after R7's capture (`order =
0`), which it reads. On SP-3 the pump at 600 sits after R12's drive (500) and before R9's upkeep (700); the AGPL ran it
before its listening block, which matches.

---

## 5. PR-1: `ApplyPlayerRole` (B)

| | |
| --- | --- |
| Target | `+0x01B92B90`, the game's role application on an `AArchonLoadout` (found through its string "`[ApplyPlayerRole] %s - null PlayerRole [%s] and/or Slot [%s]`"). It reads the loadout's active slot data, the role inventory item and its slot, and ends by calling the character's `SetPlayerRole` (a virtual) |
| Signature | `extern "C" fn(loadout)`, no return value |
| Found by | RVA and bytes `40 56 48 83 EC 70 48 8B F1` (a `.pdata` entry) **(departure: the AGPL checked no bytes)** |
| Gate | none |

Why both roles: `AArchonCharacter.PlayerRole` (`+0x738`) is **not** replicated. The server's application creates the
server's role; the owning client must run the same application for its own pawn, or it has no local role object, no
HUD binding and no lantern ability. The replicated path (`AArchonPlayerState.PlayerRoleId`, `+0x648`, `RepNotify`) only
mirrors the id string on 1.14.7; it does not install the pawn's role.

### 5.1 The role and slot, and "applied"

The game's own getters, in order, on `loadout` (readable for `0x40`):

1. `inter = (+0x01B99FB0)(loadout)`: the active loadout slot's data (it reads the active slot index at loadout
   `+0x680`). Require non-null, readable for `0x40`.
2. `role_item = (+0x01BE1EE0)(inter)`: the role inventory item (`UArchonInventoryItem`), or null.
3. `slot`: the pointer at `inter + 0x348`. (`+0x01861E40`, the game's slot getter, is a leaf with no `.pdata` entry
   that returns exactly this; R14 reads it rather than calls it **(call)**.)

Other reads: the loadout's `PlayerController` (`+0x630`, loadout readable for `0x638`); its `Pawn` (`+0x250`, readable
`0x258`); the pawn's `PlayerRole` (`+0x738`, readable `0x740`) as `pawn_role`. `ItemId` is an `FString`: on an
inventory item at `+0x70`, on an `AArchonEquipment` (the role) at `+0x240`.

**applied** = `role_item`, `slot`, `pawn` and `pawn_role` all non-null, and the item's `ItemId` is non-empty and
equals `pawn_role`'s `ItemId`.

A null `role_item` with a non-null `slot` is **legitimate**: no omnicell equipped (a new account's starter loadout has
none). A null `slot` means the loadout has not resolved yet.

### 5.2 Each call

1. Call `next(loadout)` first (the AGPL read the getters before for its log only).
2. Read 5.1. When `role_item` is non-null or `slot` is null (an omnicell equipped, or not yet known), **note validity**
   (5.3) with `applied`. An explicitly unequipped loadout (slot known, no item) is never noted.
3. **Server, applied:** add `pawn_role` to the role table (7.1) and to the Tempest watch (8.1).
4. Log `[MitPlayerRole] apply loadout=<p> item=<item id or ->  pawnRole=<p> pawnRoleItem=<id or -> applied=<0|1>`
   (budget 256; *quiet*).

### 5.3 The retry table (pure)

Eight entries `{ loadout, first_seen_ms, last_retry_ms, last_wait_log_ms, retries }`, one table per process (on a
client it holds the owning player's loadouts, on a server every player's).

- **note(loadout, applied):** an entry for `loadout` exists: applied clears it; otherwise nothing. No entry: applied
  does nothing; otherwise take the first free entry (none free: nothing) with `first_seen_ms = last_retry_ms = now`.
  Starting `last_retry_ms` at now keeps a new entry from being retried in the same tick.
- Constants: interval **1 500 ms**, at most **20** retries, window **30 000 ms** (interval × attempts), wait line
  every **5 000 ms**. Why: the game's own repair (the role's rep-notify re-running the application once the inventory
  replicates) was measured taking 20 to 40 s; this one acts every 1.5 s for as long as the slowest natural case.
- **Client travel:** every entry is cleared on a new possession (11.1): the loadouts belong to the old world.

### 5.4 The retry (each pump: client 11.2, server 6)

For each used entry:

1. `now - first_seen_ms >= 30 000`: log `[MitPlayerRole] retry: gave up loadout=<p> after <ms> ms retries=<n>`,
   clear, next.
2. `retries >= 20`, or the loadout not readable for `0x40`: clear silently, next.
3. `now - last_retry_ms < 1 500`: next.
4. **Live check first:** `is_live_object(loadout)` false → log `[MitPlayerRole] retry: loadout=<p> is gone (world torn
   down), dropped` and clear. Why: a level travel (Ramsgate to the Training Grounds) destroys the old loadout while
   its memory stays mapped and is soon reused; calling the game's application on it crashed clients.
5. Read 5.1. Applied: clear, next.
6. `role_item`, `slot` or `pawn` null: log at most every 5 000 ms per entry `[MitPlayerRole] retry: waiting
   loadout=<p> item=<p> slot=<p> pawn=<p>` and go on to the next entry **without calling the application**. Why: a
   live loadout can hold its role and slot before its controller owns a pawn; applying then runs `SetPlayerRole`
   against an unfinished possession and can stop `ClientRestart` from completing. `pawn_role` is not required: making
   it is the application's job.
7. Call `broker::original(PR-1)(loadout)` (past R14's own wrapper, so no new validity note), then re-read 5.1.
   `last_retry_ms = now`, `retries += 1`; log `[MitPlayerRole] retry: attempt=<n> loadout=<p> elapsedMs=<ms>
   applied=<0|1>`; applied clears the entry.

Server: an applied retry also adds the role to the tables as 5.2 step 3 does.

---

## 6. `SP-3@R14`: the server pump (S)

An SP-3 after handler, `order = 600`, `slice: "R14"`. At most once every **1 000 ms** (`GetTickCount64`; the first
tick after the first role or retry entry exists passes) **(departure: the AGPL ran every 64th tick, about 1 s at 60
fps but 2 s under patch 0020's 30 fps cap)**. In order:

1. the retry (5.4);
2. the post-activation refresh (7);
3. the Tempest modifier repair (8).

Nothing in the pump runs when both tables and the retry table are empty, apart from the time check.

---

## 7. The post-activation refresh (S)

### 7.1 Why, and the role table

The first time the server applies a role it is merely equipped; it becomes fully active a moment later. The engine may
send the role's first bunch before that, and a later application is not guaranteed to run: measured in a Shock
escalation, the server's Tempest role held full charge and `CanActivateAbility` true while the owning client stayed on
the pre-active zero charge for the whole hunt. Once a role has stayed fully active for a moment, R14 asks the engine to
replicate it: `AActor.FlushNetDormancy` and `AActor.ForceNetUpdate`. Nothing about the role is written.

The **role table** (pure, game-thread state): up to **256** roles `{ role, active_since_ms, last_push_ms, pushes,
was_active, done }` **(call: the AGPL's table held 4 096 (role, connection) pairs; per role, 256 covers a 64-player
hub four times over)**. `add(role)`: present → nothing; else the first free entry (none: log `[MitPlayerRole] role
table full, role=<p> not watched` once per process).

**Fully active**, read from the role (readable for `0x300`): `bIsEquipped` `+0x2F4`, `bIsEquippedForGameplay`
`+0x2F5`, `bIsActiveGameplay` `+0x2F6`, `bCalledBPHandleEquipForGameplay` `+0x2F7` all equal to 1.

**Live role** (`role_live(p)`): readable for `0x60`, `is_live_object`, the actor flags byte at `+0x5C` has bit `0x08`
clear (the actor is being destroyed), and the object flags (`u32` at `+0x08`) have neither `0x8000`
(`RF_BeginDestroyed`) nor `0x10000` (`RF_FinishDestroyed`).

### 7.2 Constants

Settle **1 500 ms**, retry **1 500 ms**, at most **3** pushes per active cycle.

### 7.3 New connections re-arm every role

When the count of `client_connections(net_driver())` differs from the count last seen (start: -1), log
`[MitPlayerRole] connections <old> -> <new>: re-arming the role refresh` and reset every entry's `active_since_ms`,
`last_push_ms`, `pushes`, `was_active` and `done`. Why: a player who joins after the others' roles went active never
received their final state; the AGPL did this by routing every known role to the new connection, which made fresh
(role, connection) entries that went through the same settle and pushes **(departure: R14 re-arms per role, not per
pair; the pushes it makes are per actor anyway)**. `net_driver()` `None`: skip this step.

### 7.4 Each pump, per used entry

1. Not `role_live(role)`: clear the entry (purge reason `tick-invalid-actor`, the line of 9), next.
2. Not fully active: reset `active_since_ms`, `last_push_ms`, `pushes`, `was_active`, `done` (a role unequipped and
   later active again gets another cycle), next.
3. Not `was_active`: set it, `active_since_ms = now`, log `[MitPlayerRole] refresh: wait role=<p> item=<id>`
   (*quiet*), next.
4. `done`, or `now - active_since_ms < 1 500`, or (`last_push_ms != 0` and `now - last_push_ms < 1 500`): next.
5. `pushes >= 3`: `done = true`, log `[MitPlayerRole] refresh: done role=<p> item=<id> pushes=3` (*quiet*), next.
6. Call `FlushNetDormancy` then `ForceNetUpdate` (both found on the role's class chain, outer `Actor`, no parameters)
   through `process_event_original`. `last_push_ms = now`, `pushes += 1`; log `[MitPlayerRole] refresh: push=<n>
   role=<p> item=<id> dormancy=<u8 at +0xF1>` (*quiet*).

Why both calls, though one usually does nothing: `FlushNetDormancy`'s body only acts when the actor's `NetDormancy`
(`+0xF1`) is above `DORM_Awake` (1), and these roles read 1, so on this build it is a no-op; it is kept because the
AGPL called it and it costs one call. `ForceNetUpdate` is what makes the graph consider the actor in its next pass.
**(departure)** The AGPL also "pushed" each pair's channel; on 1.14.7 that push only reported whether a channel
existed and wrote nothing (R9 8.3), and its "blocked" deferral (up to 20 extra tries while a channel was pending
dormancy) therefore only lengthened the window. R14 does three pushes per cycle.

---

## 8. The Tempest modifier repair (S)

### 8.1 Why, and the watch

Tempest's catalog row carries two gameplay modifiers to add while the role is active: a buff class
(`windfury_pdodge_atkspeed_buff_C`, which binds the perfect-dodge delegate and grants `Buffs.WindFuryStacks`) and an
ability class (`tempest_player_role_ab_C`, which spends one stack on tap V). The role reaching full activity does not
prove that the asynchronous applications those start actually landed. A missing buff explains the observed UI exactly:
full outer charge ring, dim centre, `CanActivateAbility` false forever. R14 checks once after activation and re-runs the
game's own application only when nothing is applied and nothing is pending.

The **watch** (pure): **64** entries `{ role, active_since_ms, last_repair_ms, last_state_log_ms, repairs }`.
`watch(role)`: only when the role is live (7.1), readable for `0x300`, and its `ItemId` (`+0x240`) is exactly
`PR_TEMPEST`; present → nothing; else the first free entry; full → `[MitPlayerRole] tempest watch full, role=<p> not
watched` once per process. A watched role logs `[MitPlayerRole] tempest: watching role=<p>`.

### 8.2 The modifier snapshot

From the role (readable for `0x300`), the catalog arrays' counts (each a `TArray`, `Num` at `+0x08` of the array):
desired buffs `+0x2B0` (array at `+0x2A8`), desired effects `+0x2C0` (array `+0x2B8`), desired abilities `+0x2D0`
(array `+0x2C8`); the applied-modifier group pointer at `+0x2F8`. From the group, when non-null (readable for `0xD0`):
applied effects `+0x68`, applied abilities `+0x78`, applied buffs `+0x88`, pending buffs `+0xA8`, pending effects
`+0xB8`, pending abilities `+0xC8` (each an `i32`). **Valid** when every count read is in `0..=256`; a null group counts
every applied and pending number as 0.

### 8.3 Each pump, per used entry

Constants: settle **3 000 ms**, retry **3 500 ms**, at most **3** repairs, state lines at most every **10 000 ms**.

1. Not live, not readable for `0x300`, or `ItemId` no longer `PR_TEMPEST`: clear, next.
2. Not fully active (7.1): `active_since_ms = 0`, next.
3. `active_since_ms == 0`: set it and `last_state_log_ms` to now; log `[MitPlayerRole] tempest: active, waiting
   role=<p>`; next.
4. `now - active_since_ms < 3 000`: next.
5. Snapshot (8.2). Not valid: at most every 10 s `[MitPlayerRole] tempest: unreadable role=<p>`; next.
6. **Healthy first:** applied buffs > 0 and applied abilities > 0 → log `[MitPlayerRole] tempest: healthy role=<p>`,
   clear, next. (The AGPL once checked "catalog empty" first and so never settled a role that was in fact healthy.)
7. Desired buffs ≤ 0 or desired abilities ≤ 0: at most every 10 s `[MitPlayerRole] tempest: catalog empty role=<p>`;
   next. A missing catalog is not repaired by guessing a class.
8. Pending buffs > 0 or pending abilities > 0: the game is still applying; at most every 10 s `[MitPlayerRole]
   tempest: pending role=<p> activeForMs=<ms>`; next.
9. `need_buffs` = applied buffs ≤ 0; `need_abilities` = applied abilities ≤ 0. Neither, or (`last_repair_ms != 0` and
   `now - last_repair_ms < 3 500`): next.
10. `repairs >= 3`: log `[MitPlayerRole] tempest: gave up role=<p> repairs=3`, clear, next.
11. `last_repair_ms = now`, `repairs += 1`. When `need_buffs`: call `ArchonEquipment.ApplyGameplayBuffsToOwner` on the
    role with a `0x10` parameter block holding a byte copy of the `TArray` header at role `+0x2A8` (data pointer, `Num`,
    `Max`). When `need_abilities`: `ArchonEquipment.ApplyGameplayAbilitiesToOwner` with a copy of the header at
    `+0x2C8`. Each only when that array's data is non-null, `0 < Num <= 256`, `Max >= Num`; else its dispatch counts as
    failed. Both through `process_event_original`. The game copies the soft classes; the role's own array is never
    modified, and R14 never frees the copy's data (it is the role's). Log `[MitPlayerRole] tempest: repair=<n>
    role=<p> buffs=<requested 0|1>/<dispatched 0|1> abilities=<requested>/<dispatched>`.

---

## 9. RG-3: the role purge on `RouteRemoveNetworkActor` (S)

| | |
| --- | --- |
| Target | `+0x01BFE4C0`, the Archon replication graph's actor removal (R12 12.2) |
| Signature | `extern "C" fn(graph, info)`, no return value; `info` is the engine's `FNewReplicatedActorInfo`, the actor at `+0x00` |
| Found by | RVA and bytes `48 89 5C 24 08 57 48 83 EC 20` (a `.pdata` entry, R12 17.2's `ROUTE_REMOVE`) |
| Gate | none |

Why a hook and not the next pump: when the graph removes a role its object can still be registered while its
destroying bit (`+0x5C`, `0x08`) is set, and a role freed and reallocated at the same address before the next pump
would inherit the old entry's state.

Each call: `actor` = the pointer at `info` when `info` is readable for 8, else 0. When `actor` is non-null, readable
for `0x60`, `is_live_object`, and `is_a(actor, "ArchonPlayerRole")`: **purge** it from the role table (7.1) and the
Tempest watch (8.1). Then call `next(graph, info)`.

**Purge** (shared with 7.4 step 1): clear every entry for that role in both tables; when anything was cleared, log
`[MitPlayerRole] purge role=<p> reason=<route-remove|tick-invalid-actor> roles=<n> tempest=<n>`.

The AGPL's membership log around the call (before and after counts of the graph's global node) is not ported (RT-3).

---

## 10. `SP-2@R14`: the ability activation route (S)

A before handler on the two exact names of 4.4 (**RT-18 (call):** the AGPL matched cached function pointers, or on
first sight a substring of the full name, `ServerTryActivateAbilityWithEventData` first and then
`ServerTryActivateAbility`; a substring could have cached a different function of the same stem as the first; the two
exact names are what it meant). It always returns `Continue`: **the original RPC still runs afterwards**, as in the
AGPL.

Parameter blocks (the dump's): `ServerTryActivateAbility` (size `0x18`): `AbilityToActivate`
(`FGameplayAbilitySpecHandle`, a `u32`) `+0x00`, `InputPressed` (`bool`) `+0x04`,
`PredictionKey` (`FPredictionKey`, `0x10`) `+0x08`. `...WithEventData` (size `0xC8`): the same three, then
`TriggerEventData` (`FGameplayEventData`, `0xB0`) at `+0x18`.

Each call, with `component` = the object (an `AbilitySystemComponent`) and the params readable for their size:

1. When `InputPressed`: `call` `AbilitySystemComponent.ServerSetInputPressed` on the component with `{ AbilityHandle:
   u32 at +0x00 }` (size 4).
2. `activated = InternalTryActivateAbility(component, handle, &key_copy, &instanced, null, event_data)` at
   `+0x016C85D0` (bytes `4C 89 4C 24 20 4C 89 44 24 18 89 54 24 10`, a `.pdata` entry; checked once before the first
   call, and a mismatch disables this handler with one line). Win64: `extern "C" fn(component, handle: u32,
   prediction_key: *const [u8; 16], out_instanced: *mut usize, on_ended: *const c_void, trigger_event_data: *const
   c_void) -> u8`. `key_copy` is a 16-byte copy of the params' key **(call)**; `event_data` is the params' `+0x18` for
   `...WithEventData`, else null; `instanced` is R14's own slot, unused after.
3. When `!activated` and `InputPressed`: `call` `AbilitySystemComponent.ServerSetInputReleased` with the handle.
4. Log `[MitPlayerRole] ability: handle=<n> inputPressed=<0|1> activated=<0|1> owner=<name>` (budget 128; *quiet*),
   `owner` the short name of the pointer at component `+0x3D0` (readable `0x3E0`), or `null`.

This is the activation the Undaunted-era runtime added and every later runtime kept: the server activates the ability
itself before the stock RPC implementation runs. Why the stock path alone is not enough is not recorded in the source
**(ambiguity)**; R14 keeps the behaviour exactly, since the play-test of every omnicell depends on it.

---

## 11. `SP-1@R14`: input activation, the role lifecycle and movement (C)

### 11.1 The possession reset (after `ClientRestart`, `order = 10`)

Read R7's `captured()`. When its `generation` differs from the last one R14 saw (a **new possession**): record it and
reset R14's per-possession state: input activated (false), activation pending (false), natural edge seen (false), the
game's last input value (-1) and its time (0), activation depth (0); clear the retry table (5.3); forget the lifecycle
replay's per-pawn state (11.5). Log `[MitInput] new possession pc=<p> pawn=<p>`. A repeated `ClientRestart` for the
same pair changes nothing. Why: a seamless or map travel makes a new controller and pawn, and the whole cascade must
run again for them.

### 11.2 The pump (before handler on every call, `order = 50`)

Runs only when `dispatch::depth() == 1` (an outermost call: the AGPL never ran this inside a nested call) and at most
every **100 ms** **(departure: the AGPL ran it on every 256th outermost call; at a client's call rate that is several
times a second, and every step below has its own slower throttle)**. Always returns `Continue`.

1. `captured()` is `None`: return.
2. `stable = possession_stable(pc)`.
3. Stable: the client retry (5.4); the lifecycle replay with trigger `pump` (11.5) unless
   `DISABLE_CLIENT_ROLE_LIFECYCLE_REPLAY.flag`; the HUD reconcile with trigger `pump` (12.3).
4. Not stable but a possession exists: at most every 5 000 ms `[MitInput] waiting for a stable possession; pump
   skipped`.
5. **Activation, deferred or by timeout** (stable only, and input not yet activated):
   - activation pending (11.3 deferred it): log `[MitInput] activate: deferred event replayed after <ms> ms`, run the
     cascade (11.4) with trigger `deferred-event`, cinematic clear allowed; if it activated, pending = false.
   - still not activated and **8 000 ms** since the possession began: log `[MitInput] activate: timeout after <ms> ms,
     restoring input without the cinematic clear`, run the cascade with trigger `timeout`, cinematic clear **not**
     allowed.

### 11.3 `OnInputModeChanged` (after handler, `order = 20`)

The pawn's Blueprint event `BP_PlayerCharacter_C.OnInputModeChanged`; its params' byte `+0x00` is `bGameInputEnabled`.

1. `from_runtime` = R14's activation depth > 0 (the event was raised by R14's own cascade); `current` =
   `chain_for(pc, object)` is `Some` (the event's pawn is the captured possession's pawn).
2. When not `from_runtime` and `current`: the game's last input value = `bGameInputEnabled`, its time = now.
3. Log `[MitInput] input mode gameInput=<0|1> source=<game|runtime>` (budget 256).
4. When input is not yet activated and a possession is captured: if not `current`, log `[MitInput] ignored an input
   mode change from another controller or pawn`; else natural edge seen = true, and if `possession_stable(pc)` run the
   cascade (11.4) with trigger `event`, cinematic clear allowed; otherwise activation pending = true and log
   `[MitInput] activate: deferred, <ms> ms since the possession (settle 3000 ms)`.
5. The movement repair (11.6) on `object`.

Why the event is a trigger only: captures showed the first natural `OnInputModeChanged` can come before the world is
stable, so it starts the cascade only once the possession is stable, and otherwise the pump replays it (11.2).

### 11.4 The input cascade (`activate(pc, trigger, allow_cinematic_clear)`)

Activation depth += 1 for the whole cascade (and -= 1 on every exit). In order:

1. **`bOnlineDataReady`** (`AArchonPlayerController` `+0xC69`, `u8`; the game's own input gate, written by
   `OnPlayerDataLoadComplete`). Not 1: write 1 with R17's checked `write`, read it back, log `[MitInput] activate
   trigger=<t>: bOnlineDataReady was <n>, now <n>`. Still not 1: return (input not activated; the next trigger tries
   again). Why: when the player-data responses are late the game never opens its own input gate, and the player cannot
   move; the flag only gates the controller's input, not network state.
2. Input activated = true **before** any call: `EnableCharacterInput` raises `OnInputModeChanged` itself, and without
   the flag first the cascade re-entered itself until the stack overflowed (measured).
3. `ArchonPlayerController.EnableCharacterInput` with `{ bEnableInput: true }` (size 1). Never
   `EnableControllerInput` (it re-enters the event path); a generic `Actor.EnableInput` does not undo Dauntless's own
   UI input suppression.
4. **The active loadout and the role.** `ArchonPlayerController.GetLoadout` (`{ ReturnValue }`, size 8). Non-null:
   `ArchonLoadout.GetActiveLoadoutSlotIndex` (`{ ReturnValue: i32 }`). An index outside `0..6`:
   `ArchonLoadout.ServerInternalSetActiveLoadoutSlotIndex` with `{ InDesiredLoadoutSlotIndex: 0, bForceApply: true }`
   (size 8), index = 0, log `[MitInput] active loadout slot was invalid; slot 0 applied`; else log `[MitInput] kept
   the active loadout slot <n>`. (A valid slot 1 to 5 must survive a hunt's travel; an older bootstrap always forced
   0.) With an index ≥ 0: call the **PR-1 entry** on the loadout (5.2 runs through R14's wrapper and notes validity),
   log `[MitInput] re-applied the role for slot <n>`; then, when `possession_stable(pc)` and not
   `DISABLE_CLIENT_ROLE_LIFECYCLE_REPLAY.flag`, the lifecycle replay with trigger `post-apply` (11.5), else `[MitInput]
   role lifecycle deferred until the possession settles`. A null loadout: `[MitInput] no loadout; slot and role
   skipped`.
5. `Controller.ResetIgnoreMoveInput` and `Controller.ResetIgnoreLookInput` (no parameters): a cinematic that set
   "ignore move" and never cleared it leaves the engine's reference count above zero for good.
6. **The cinematic clear**, only when `allow_cinematic_clear`, the natural edge was seen, the game's last input value
   is 1 (the game handed control to the player) and `possession_stable(pc)`: `PlayerController.SetCinematicMode` with
   five `false` bytes (`bInCinematicMode`, `bHidePlayer`, `bAffectsHUD`, `bAffectsMovement`, `bAffectsTurning`, size
   5), log `[MitInput] cinematic mode cleared`. Otherwise `[MitInput] cinematic clear skipped trigger=<t> allow=<0|1>
   edge=<0|1> gameInput=<-1|0|1> stable=<0|1>`. Why the hand-over test: on a hunt the first natural edge is the arrival
   cinematic taking input away (false); clearing then made the Blueprint show the gameplay HUD over the arrival beam
   about 15 s before the cinematic ended. Ramsgate and the Training Grounds hand over with a true edge and still get the
   clear.
7. **The input configuration.** `ArchonPlayerControllerBase.ApplyInputConfiguration` (params: `InputConfiguration`
   `u8` `+0x00`, `ReturnValue` `bool` `+0x01`, size 2) with `Hunter` (1), **then with `Default` (0)**; then
   `ArchonPlayerControllerBase.ApplyInputConfigurationFromSettings` (no parameters). See (ambiguity) 17.3: the AGPL
   meant "Default only if Hunter fails" but read the return value from the wrong byte, so it has always called both,
   and Default is what every play-test has run on. R14 keeps that observable behaviour **(call)** and logs `[MitInput]
   input configuration Hunter -> <ret>, Default -> <ret>`.
8. `SetupArchonInput` on the controller: found first as `player_controller_bp_C.SetupArchonInput`, else
   `ArchonPlayerController.SetupArchonInput`; then on the controller's `Pawn` (`+0x250`, live): first
   `BP_PlayerCharacter_C.SetupArchonInput`, else `ArchonCharacter.SetupArchonInput`. No parameters. Why: the natural
   login never ran the game's input setup on this server, so only the pawn's hard-wired axes worked (no strafe,
   interact, sprint, attacks or dodge).
9. The HUD reconcile with trigger `input-activation` (12.3): the cascade can rebuild and show the combat HUD while the
   airship screen is still up.

Every call through `process_event_original`; a function not found is skipped with one line per name per process
(`[MitInput] <Class>.<Function> not found`).

### 11.5 The role lifecycle replay (`replay(pc, trigger)`)

The pawn's role exists on the client only through the owning client's own application (5). Two things go wrong after a
travel: the new pawn has no role though the replicated id names one, or the role was installed after the player state
became active and stays "equipped but not active" (`bIsEquipped` 1, the three gameplay bytes 0).

1. `pc` live, readable for `0x258`; `pawn` (`+0x250`) non-null, readable for `0x740`; `player_state` (pawn `+0x240`)
   non-null, readable for `0x6B1`; `role` = pawn `+0x738`.
2. **No role.** `id` = the player state's `PlayerRoleId` (`FString`, `+0x648`, readable `0x658`).
   - Empty: log once per pawn `[MitPlayerRole] client: PlayerRoleId empty, waiting for replication`; return.
   - A state per (pawn, id): attempts, last attempt time, exhausted time; a different pawn or id restarts it.
   - After **20** attempts: wait **30 000 ms** from the moment it was exhausted, then log `[MitPlayerRole] client:
     retrying the role after a 30 s pause` and restart the count. Why: a mid-hunt loss once exhausted a one-shot
     budget and the player had no omnicell for the rest of the hunt.
   - At most one attempt every **1 500 ms**. An attempt: `ArchonPlayerController.GetLoadout`; the loadout must be
     non-null and live, else `[MitPlayerRole] client: no live loadout to re-apply; will retry`. Then the retry's step
     of 5.4 (read, apply past R14's wrapper when not applied, re-read) on it; re-read `role`; log `[MitPlayerRole]
     client: re-applied the role attempt=<n> id=<id> roleAfter=<p>`. Still null: return. Why not the replicated
     notify: `AArchonPlayerState.OnRep_PlayerRoleId` only mirrors the id string; it never installs the pawn's role. Only
     the application does (it ends in `SetPlayerRole`).
3. `role` readable for `0x2F8`. `active` = the player state's `bIsActiveGameplay` (`+0x6B0`). When `active != 1`, or
   the role's three gameplay bytes (`+0x2F5`, `+0x2F6`, `+0x2F7`) are all 1: return.
4. **At most once per (pawn, role) pair**: call `ArchonCharacter.HandleActiveGameplayStateChanged` on the pawn with
   `{ bInIsActiveGameplay: true, bForceUpdate: true }` (size 2). Log `[MitPlayerRole] client: lifecycle replayed
   trigger=<t> gameplayEquipped=<a>-><b> roleActive=<a>-><b> bpEquipCalled=<a>-><b>`. Why once: on the owning client
   these bytes legitimately stay 0 even when the role works, so a loop until they read 1 never converges and replays
   gameplay and HUD notifications.

The lifecycle bytes are never written.

### 11.6 The movement repair (on every `OnInputModeChanged`, on its pawn)

From the pawn `p` (readable for `0x1608`): when `CanMove_` (`+0x15E0`, `u8`) is not 1, write 1 there and at
`PreviousCanMove` (`+0x1604`). `CharacterMovement` = the pointer at `+0x288`; when non-null (readable `0x3C0`) and its
`MovementMode` (`+0x168`) is 0 (`MOVE_None`), write 1 (`MOVE_Walking`) there, at `DefaultLandMovementMode` (`+0x3B8`)
and at the pawn's `ReplicatedMovementMode` (`+0x328`). `RootComponent` = the pointer at `+0x130`; when non-null
(readable `0x150`) and its `Mobility` (`+0x14F`) is not 2, write 2 (`Movable`). The pawn's `Role` (`+0xF0`): when
neither 2 (`AutonomousProxy`) nor 3 (`Authority`), write 2. Each write that happens logs once per process per field
`[MitInput] movement repair: <field> <old> -> <new>`. All writes through R17's checked `write`.

These are the 1.12-era repairs that made the first private client move. **(ambiguity)** Whether any still fires on
1.14.7 is not recorded; each is a no-op on a healthy pawn, so R14 keeps them, and the play-test (16.3 step 3) looks for
their lines.

---

## 12. `SP-1@R14`: the airship HUD (C)

The game's own presentation contract: while `AArchonGameState.IsInPreMatchAirship` is true the combat HUD is hidden
(`AArchonHUD.HideGameplayHUD`); after the airship it comes back (`ShowGameplayHUD`). The input cascade and the role
re-application can show the combat HUD while the airship screen is up, so R14 reconciles after each of them. Only the
game's reflected functions are called; no widget or visibility byte is written. Party, chat and the airship's Ready
panel stay the Blueprint's.

### 12.1 The capture (before `OnAirshipStateUpdate`, `order = 0`)

`BPH_ArchonHUD_C.OnAirshipStateUpdate`'s params `+0x00` is the `AArchonGameState`. When the params are readable for 8
and that pointer is non-null and readable for `0x2D1`: remember the HUD (the object) and the game state.

### 12.2 Triggers (after handler, `order = 30`)

Matcher, all `FunctionName` **(RT-18 (call): the AGPL used substrings of the full name starting at the `.`; the
function-name form is the same set for every name below except a longer name with one of these as its start, which
the AGPL would also have caught)**: `OnAirshipStateUpdate` (trigger `state-update`); `ShowGameplayHUD`,
`Player_HUD_Ready`, `Progression_HUD_Ready`, `ReceiveGameplayStart`, `Refresh_HUD_Widget_Visibility`, `Show_HUD`,
`Show HUD` (trigger `hud-show-path`); `OnHUDVisibilityChanged` (trigger `hud-visibility-changed`). Most of the
show-path names are Blueprint-to-Blueprint calls that never reach `ProcessEvent`; the native visibility setters raise
`OnHUDVisibilityChanged` through it on every change, which is the reliable "someone just showed the HUD" signal.

### 12.3 The reconcile (`reconcile(trigger)`)

State (pure): `hud`, `game_state`, `hidden_by_us`, `airship_ended_ms`, a `busy` flag.

1. `busy`: return. (Hide and Show raise `OnHUDVisibilityChanged`, which is itself a trigger.) No `hud` or `game_state`:
   return.
2. Either is not `is_live_object`: forget both, `hidden_by_us = false`, `airship_ended_ms = 0`; return. They belong
   to one world and a travel frees them.
3. `hud` readable for `0x362`, `game_state` for `0x2D1`; else return. `in_airship` = the game state's `+0x2D0`;
   `visibility` = the HUD's `GameplayHUDVisibility` (`+0x361`, an `ESlateVisibility`), for the lines only.
4. **In the airship** (`in_airship == 1`): `airship_ended_ms = 0`; `busy` while calling `ArchonHUD.HideGameplayHUD` on
   the HUD; not found → one line per process `[MitHud] HideGameplayHUD not found`, return. Log `[MitHud] hide
   trigger=<t> visibility=<before>-><after>` the first time and whenever the visibility changed or we had not hidden
   it yet (budget 32). `hidden_by_us = true`.
5. **After the airship** (`in_airship == 0` and `hidden_by_us`): when `airship_ended_ms == 0`, set it to now and log
   `[MitHud] airship ended trigger=<t>; the combat HUD stays hidden until the player has control`. Restore when
   - **player control:** the game's last input value (11.3) is 1, its time ≥ `airship_ended_ms`, and at least
     **1 000 ms** ago (the drop opens with a 6 to 8 ms true/false blip); or
   - **timeout:** **45 000 ms** since `airship_ended_ms`, so a missed edge never leaves the HUD hidden.

   Otherwise return. Restore: `busy` while calling `ArchonHUD.ShowGameplayHUD` (not found: one line, return); log
   `[MitHud] restore trigger=<t> reason=<player-control|timeout> sinceAirshipEndMs=<ms> visibility=<before>-><after>`;
   `hidden_by_us = false`, `airship_ended_ms = 0`. If the game already showed the HUD the call changes nothing.

Why the wait: in every escalation capture `IsInPreMatchAirship` went false while the airship UI was still up, about
3 s before the drop and 20 s before the first step; an immediate restore put the combat HUD over the launch and the
arrival beam.

`busy` is cleared on every exit of the call, including a panic (R14 catches panics around the handler).

---

## 13. The AGPL side and the other slices

### 13.1 How the AGPL stands down (seam patch 0041, no new patch)

- PR-1 and RG-3 are in 0041's RVA table (`+0x01B92B90`, `+0x01BFE4C0`). With R14 owning them the AGPL makes no
  wrapper; its "original" pointers become `rir_original(target)`.
- `SP-1@R14` gates every AGPL client block R14 replaces: the airship capture, the client pump (with its census and
  probes), the HUD reconcile after the call, the `OnInputModeChanged` activation and movement blocks, the axis-event
  block and the `ClientRestart` capture. The AGPL's own input cascade calls its role application body directly; that
  call sits inside a gated block.
- `SP-2@R14` gates the AGPL's ability activation route.
- `SP-3@R14` gates the AGPL's once-a-second block of role retries, the post-activation refresh and the Tempest
  ensure. **That block also runs the AGPL's relevance re-assert (R13's)** and the Frank drone diagnostics: see 13.3.
- With PR-1 owned, nothing fills the AGPL's role route table, Tempest tables or retry table any more; RG-3's AGPL
  purge (now not installed) only purged those. No state crosses.

So nothing beyond 0041 is needed, and **R14's reserved patch number 0058 is unused**.

### 13.2 Cross-slice state (R0 7.5's table, rows added)

| State | Set by | Read by | Rule |
| --- | --- | --- | --- |
| the client's captured controller | the AGPL's `SP-1@R14` capture | the AGPL's R7 readiness (LD-1), R15's loadout repair, and logging | **not carried:** R7 (MIT) captures its own; R15 (MIT) captures its own (R15 6.2). R14 lands with or after R7 and R15 (13.3) |
| the AGPL's relevance re-assert | inside the `SP-3@R14` block | — | **R14 lands with or after R13**, whose spec ports the pass and gates the AGPL's copy on `SP-3@R13` (13.3) |
| the player role caches' purge on route removal | RG-3 | R14's tables | moves with R14 (R12 12.2's row is settled here) |

### 13.3 Landing: R14 with or after R7, R13 and R15 **(call)**

`LANDED` holds `"R14"` only when it holds `"R7"`, `"R13"` and `"R15"`; a registry test enforces it (16.1). Why each:

- **R7:** R14 reads R7's possession capture (3.1); the surface lists R7 as a dependency.
- **R13:** the AGPL runs its relevance re-assert (each role's cull settings and each player pawn's channel grace,
  R13's `SP-3@R13` work) only from inside the `SP-3@R14` block. Owning `SP-3@R14` while R13 is AGPL would silently
  stop that pass. R13 lands after R12 (its dependency), so R12 is in too. R13's spec (9, 13.1) ports the whole pass,
  finds the roles itself (each client connection's controller `+0x30` → `Pawn` `+0x250` → `PlayerRole` `+0x738`),
  not from R14's table, and its patch 0057 makes the AGPL's copy stand down on `SP-3@R13`, so MIT R13 with AGPL R14
  never runs it twice (agreed between the two writers on 2026-11-13).
- **R15:** the AGPL's loadout repair (0027, 0037) reads the controller the AGPL's `SP-1@R14` capture kept. R15's MIT
  side keeps its own (R15 6.2), so with R15 landed nothing reads the AGPL's.

The R14 implementer's branch adds `"R14"` to `LANDED` only if all three are already on `plan`; otherwise the
integration pass adds it when it merges R14 after them. Until then R14's code builds and its pure tests run, and it
installs nothing.

### 13.4 With the MIT DLL absent

The AGPL runtime does everything as today (seam 0041).

---

## 14. Flags (RT-14 for R14)

| Flag | Kept? | Meaning in the MIT runtime |
| --- | --- | --- |
| `DISABLE_CLIENT_ROLE_LIFECYCLE_REPLAY` | kept | client, read once: no lifecycle replay (11.2 step 3, 11.4 step 4). The one switch for a client-side behaviour that writes nothing but replays game events |
| `DISABLE_PLAYER_ROLE_GLOBAL_ROUTE` | **dropped** | its R14 use selected the per-connection node insert, which R14 does not port; its other reader, RG-2, drops it too (R13 12). Without the global route a role's channel closes and reopens and the client's role (and its HUD binding) is rebuilt every few seconds **(ruling, shared with R13)** |
| `DISABLE_PLAYER_ROLE_OWNER_ROUTE` | **dropped** | an A/B switch that turned off the role routing and with it the post-activation refresh; it can only bring back the stale zero charge (7.1) **(ruling)** |
| `DISABLE_PLAYER_ROLE_REMOTE_ROUTE` | **dropped** | suppressed the remote "bootstraps" (no-ops on 1.14.7) and the connection-count re-route; the latter is R14's re-arm (7.3), which only makes late joiners see final role state **(ruling)** |
| `USE_LEGACY_ROLE_ONREP_REPAIR` | **dropped** | selected the replicated-notify replay, which is proven a no-op for installing the role (11.5) |
| `ROLE_PASS_SNAPSHOT_OFF` | **dropped** | switched off a performance snapshot R14 has no need of (no per-pair table) |
| `IMMEDIATE_AIRSHIP_HUD_RESTORE` | **dropped** | brought back the restore that put the combat HUD over the launch (12.3) **(ruling)** |
| `FORCE_CINEMATIC_CLEAR` | **dropped** | brought back the clear that showed the HUD over the arrival beam (11.4 step 6) **(ruling)** |
| `NO_PLAYER_ROLE_CHANNEL_LOG`, `NO_PAWN_ROLE_CENSUS`, `NO_CONTROLLER_SPLIT_LOG`, `ENABLE_CONTROLLER_WATCHPOINT` | **dropped** | diagnostics R14 does not port **(RT-3)** |
| `DIAG_NATURAL` | not read | R12's ruling for every MIT slice |

A dropped flag left in a game folder has no effect on the MIT side. While R14 is AGPL, the AGPL still honours them.

---

## 15. Rulings settled here

- **RT-14 for R14:** section 14 (one kept, nine dropped; `DISABLE_PLAYER_ROLE_GLOBAL_ROUTE` with R13).
- **RT-17 for R14:** section 4.4; no order dependency.
- **RT-18 for R14:** the matchers of 4.4 and 12.2: `Exact` for `ClientRestart`, `OnInputModeChanged` and the two
  ability RPCs; `FunctionName` for the HUD names and `OnAirshipStateUpdate`; `(Prefix, "")` for the pump (R16 5.2's
  form). No `Contains` match.
- RT-3: the diagnostics of section 1, the channel churn log, the census and probes are not ported.
- RT-12, RT-13, RT-15 and RT-16 do not touch R14.
- The surface's split question: not split (1).

---

## 16. Tests

### 16.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`), over a fake `Memory` and a fake clock:

- **Applied:** every combination of null item, slot, pawn, pawn role; equal and different item ids; an empty id.
- **Retry table:** note with applied clears; note of an unequipped loadout (slot known, no item) not called (5.2);
  eight entries then the ninth ignored; no retry in the tick it was noted; 1 499 ms waits, 1 500 ms retries; the 21st
  attempt never happens; 30 000 ms gives up with its line; a dead loadout dropped without the application (the fake
  application asserts it was not called); the wait line at most every 5 000 ms; a new possession clears all.
- **Role table and refresh:** add twice keeps one; the 257th role logs once; inactive resets the cycle; wait →
  1 500 ms → push 1, 1 500 ms → push 2, → push 3 → done; a role going inactive and active again gets three more; a
  connection count change re-arms every entry and logs; `role_live` false for each of the destroying bit and the two
  object flags.
- **Tempest:** a non-Tempest role is not watched; the healthy check before the catalog check (applied 1/1 with desired
  0 settles as healthy); pending waits; repair only what is missing; 3 500 ms between repairs; the fourth gives up;
  the counts' 0..=256 validity; the array guard (null data, `Num` 0, `Num` > 256, `Max` < `Num`).
- **RG-3 purge:** a role in both tables is purged with one line; an unknown role logs nothing.
- **Ability route:** pressed and activated (pressed once, no release); pressed and not activated (pressed, released);
  not pressed (neither); the event data pointer only for the long form; the key passed is a copy (the fake callee
  writes to it; the params' key is unchanged).
- **Input:** a runtime-raised event does not update the game's input value; a non-current pawn is ignored; stable →
  cascade, not stable → pending, then the pump replays it; the 8 000 ms timeout without the cinematic clear; the
  cinematic clear's four conditions each alone keep it skipped; `bOnlineDataReady` written and read back, a failed
  write stops the cascade; input activated set before the first call (a fake `EnableCharacterInput` that raises the
  event sees the cascade not re-run); the slot rule (−1 and 6 → slot 0 applied; 0..5 kept).
- **Input configuration:** both calls are made, Hunter then Default, whatever the return byte.
- **Lifecycle:** empty id logs once per pawn; 20 attempts, a 30 000 ms pause, then a new batch; 1 500 ms between
  attempts; the replay at most once per (pawn, role); not when inactive or already active.
- **Movement:** each of the four repairs alone; no write on a healthy pawn.
- **HUD:** hide in the airship; after: no restore before control, restore at 1 000 ms of control seen after the end;
  control seen before the end does not count; the 45 000 ms timeout; `busy` blocks the nested trigger; dead cached
  pointers forget the state.
- **Lines:** every text on this page for fixed values; the *quiet* lines are skipped under the production profile on a
  server and kept on a client.
- **Registry:** if `LANDED` contains `"R14"` it contains `"R7"`, `"R13"` and `"R15"`; with all four, `owns` is true for
  PR-1, RG-3, `SP-1@R14`, `SP-2@R14`, `SP-3@R14`; RG-3's slice is R14; the count grows by 5.
- **Offsets:** PR-1 and RG-3 have one `RvaAndBytes` line each with the bytes of 18.1.

DLL (`cargo test -p rustic-isles-runtime`, Windows), against fakes in test memory, never the game's RVAs:

- PR-1's wrapper through the broker on a test function: `next` runs first; validity is noted after it.
- RG-3's wrapper with a test `info` whose actor is not a role calls `next` and purges nothing.
- The ability handler with a fake `InternalTryActivateAbility` injected as a function pointer: argument order and the
  null `on_ended`.
- `dispatch::depth()` is 1 inside an outermost handler and 2 inside a nested call.

### 16.2 What CI cannot show

Every hook against the real executable; whether the client's role, HUD and lantern work; Tempest's stacks; the input
cascade on a real travel; the airship HUD in a real Trials or escalation launch: the play-test.

### 16.3 The maintainer's play-test (surface section 8, R14 row; collected for the one hands-on pass at the end)

"Each omnicell's ability and HUD charge, lantern, airship HUD restore after a hunt." Needs an MIT build with R7, R9 to
R13, R15 and R14 landed (13.3), both DLLs injected as in R0 11.1, R2's redirect working. Use two accounts where a step
says so; have at least Tempest and one other omnicell equipped (Iceborne, Darkness or Artificer), and one account with
**no** omnicell.

1. **Status.** Each server log shows `install PR-1 ok`, `install RG-3 ok` and the AGPL's seam line with two more hooks
   left to it than before R14; the client log shows `install PR-1 ok`.
2. **Ramsgate, login.** Log in with the Tempest account. Within a few seconds of control: `[MitInput] new possession`,
   then `[MitInput] activate ...` lines ending without `not found` lines; no `[MitInput] movement repair` line is
   expected (note any). Walk, strafe (A/D), sprint, dodge, interact with a vendor: every input works. The HUD shows
   the omnicell's icon and charge ring. On the server: `[MitPlayerRole] apply ... applied=1`.
3. **No omnicell.** Log in with the account without one: no `[MitPlayerRole] retry:` lines at all for it (an
   unequipped loadout is never noted), input works, no omnicell on the HUD.
4. **A hunt, each omnicell.** Start a hunt with Tempest from Ramsgate. Through the airship: the combat HUD stays hidden
   on the airship (`[MitHud] hide`), stays hidden through the launch and the arrival beam, and returns when you can
   move (`[MitHud] restore ... reason=player-control`, a second or so after control, never `timeout` in a normal
   arrival). The charge ring fills; tap V uses the ability (server: `[MitPlayerRole] ability: ... activated=1`). For
   Tempest: perfect-dodge an attack, then tap V: the ability fires; the server shows `[MitPlayerRole] tempest:
   healthy` once (a `repair=` line means the game dropped its own application and R14 repaired it: note it). Repeat
   with the second omnicell (no Tempest lines). The lantern (hold V) works.
5. **Late joiner.** With player A already in the hunt with an active omnicell, player B joins. Server: `[MitPlayerRole]
   connections 1 -> 2: re-arming the role refresh`, then `refresh: push=` lines. B sees A in a proper pose (not the
   reference "stretch" pose) and A's omnicell effects; A's charge ring on A's screen stays correct.
6. **Back to Ramsgate.** Finish or abandon the hunt and return. The combat HUD is back in Ramsgate; the omnicell is on
   the HUD with its charge; input works without `timeout` in `[MitInput] activate` (a `deferred-event` is fine). No
   crash on the travel (the July travel crashes were in exactly this window).
7. **Training Grounds and back.** Board the airship to the Training Grounds and back to Ramsgate twice, quickly (within
   30 s of arriving). No crash; `[MitPlayerRole] retry: loadout=... is gone (world torn down), dropped` may appear and
   is the fix working.
8. **Trials or an escalation launch.** Launch one: the airship HUD hides and restores as in step 4; in an escalation,
   the omnicell keeps its charge across rounds (the post-activation refresh: the client must not sit at zero charge
   while the server's role is full).
9. **Active loadout slot.** Select loadout slot 3, start a hunt, return: still slot 3 (`[MitInput] kept the active
   loadout slot 3`).
10. **Off switch.** With `DISABLE_CLIENT_ROLE_LIFECYCLE_REPLAY.flag` beside the client executable, repeat step 2: no
    `client: lifecycle replayed` line; note whether the omnicell still activates (it tells the maintainer whether the
    replay is still needed on 1.14.7).

Only CI passed until these steps are done.

---

## 17. Calls, departures and ambiguities

### 17.1 Calls (for the maintainer to confirm; the ones marked **ruling** need one)

1. **Not split** (1); the progression HUD refresh is dead code and not ported; per-connection routing reduces to a
   per-role refresh.
2. **Landing: R14 with or after R7, R13 and R15** (13.3), enforced by a registry test; R13's spec ports the
   relevance pass and gates the AGPL's copy (patch 0057).
3. **Ruling: five A/B switches dropped** (14): `DISABLE_PLAYER_ROLE_OWNER_ROUTE`, `DISABLE_PLAYER_ROLE_REMOTE_ROUTE`,
   `IMMEDIATE_AIRSHIP_HUD_RESTORE`, `FORCE_CINEMATIC_CLEAR`, and `USE_LEGACY_ROLE_ONREP_REPAIR` (with
   `ROLE_PASS_SNAPSHOT_OFF` and the RT-3 diagnostics); `DISABLE_PLAYER_ROLE_GLOBAL_ROUTE` is R13's ruling 3.
   Recommended: yes; each can only bring back a fixed bug.
4. **Both input configurations** (11.4 step 7): keep calling Hunter then Default, as every play-test ran. Recommended:
   yes for the port; whether Hunter alone is better is a separate gameplay question for after the play-test (17.3).
5. RG-3 installed purge-only (9).
6. The slot getter read rather than called (5.1).
7. The prediction key passed as a copy (10).
8. The role table holds 256 roles (7.1).
9. *Quiet* lines under the production profile (4.3).
10. R14 adds `dispatch::depth()` to R0 and three accessors to R7's possession module if absent (2.2, 3.1).
11. Patch 0058 unused (13.1).

### 17.2 Departures

1. PR-1 checked against its first bytes (5).
2. No fault catch anywhere; liveness checks before every native call with a remembered pointer (4.1).
3. The server pump by time (1 000 ms), the client pump by time (100 ms) on outermost calls (6, 11.2).
4. The refresh per role, three pushes per cycle, re-armed by a connection-count change; no channel "push" and no
   blocked deferrals (7.3, 7.4).
5. `Exact` and `FunctionName` matchers in place of substrings (10, 12.2).
6. New tags and texts; budgets on repeating lines; the caller labels and per-call state dumps of the AGPL's apply line
   are not ported (4.3, 5.2).

### 17.3 Ambiguities

- **`ApplyInputConfiguration`'s return value.** The dump puts `ReturnValue` at `+0x01` of a 2-byte block. The AGPL read
  it at `+0x04` of its own 8-byte block, which nothing writes, so it always saw false and always applied `Default`
  after `Hunter`. Whether the intended Hunter-only configuration is better (its comment says Hunter carries the full
  combat controls) was never tested; R14 keeps the observed behaviour (call 4).
- **`bOnlineDataReady`** is at `+0xC69` in the dump and in the AGPL's code; the AGPL's comments say `+0xC59` (a 1.12
  offset). R14 uses `+0xC69`.
- **The ability route** (10): why the server activates the ability before the stock RPC runs, and whether the stock
  RPC then activates it a second time, is not recorded. Kept exactly.
- **The movement repairs** (11.6): whether any still fires on 1.14.7.
- **`FlushNetDormancy`** does nothing for a role at `DORM_Awake`; kept because the AGPL called it (7.4).

No open question remains beyond the ruling of call 3 and the confirmation of calls 2 and 4.

---

## 18. Offset-table lines (R14)

### 18.1 `TARGETS` (`FoundBy::RvaAndBytes`: `.pdata` entries with these first bytes, checked 2026-11-13)

| Id | RVA | Bytes | What |
| --- | --- | --- | --- |
| PR-1 | `0x01B9_2B90` | `40 56 48 83 EC 70 48 8B F1` | `ApplyPlayerRole` (on an `AArchonLoadout`) |
| RG-3 | `0x01BF_E4C0` | `48 89 5C 24 08 57 48 83 EC 20` | the Archon graph's `RouteRemoveNetworkActor` (R12's `ROUTE_REMOVE`; use R12's constant if it exists) |

### 18.2 `pub mod r14`

| Name | Value | Use |
| --- | --- | --- |
| `LOADOUT_ACTIVE_SLOT_DATA` | `0x01B9_9FB0` (bytes `48 89 5C 24 08 48 89 74 24 10 57`), `(loadout) -> inter` | 5.1 |
| `SLOT_ROLE_ITEM` | `0x01BE_1EE0` (bytes `48 83 EC 28 48 8B 89 48 03 00 00`), `(inter) -> item` | 5.1 |
| `INTERNAL_TRY_ACTIVATE_ABILITY` | `0x016C_85D0` (bytes `4C 89 4C 24 20 4C 89 44 24 18 89 54 24 10`) | 10 |
| loadout and items | `INTER_SLOT 0x348`, `LOADOUT_SPAN 0x40`, `LOADOUT_CONTROLLER 0x630` (span `0x638`), `LOADOUT_ACTIVE_SLOT_INDEX 0x680`, `ITEM_ID 0x70` (inventory item), `EQUIPMENT_ITEM_ID 0x240` | 5 |
| controller, pawn, player state | `PC_PAWN 0x250`, `PC_ONLINE_DATA_READY 0xC69`, `CHARACTER_PLAYER_ROLE 0x738` (span `0x740`), `PAWN_PLAYER_STATE 0x240`, `PS_PLAYER_ROLE_ID 0x648` (span `0x658`), `PS_ACTIVE_GAMEPLAY 0x6B0` (span `0x6B1`) | 5, 11 |
| role | `ROLE_SPAN 0x300`, `ROLE_EQUIPPED 0x2F4`, `ROLE_GAMEPLAY_EQUIPPED 0x2F5`, `ROLE_ACTIVE_GAMEPLAY 0x2F6`, `ROLE_BP_EQUIP_CALLED 0x2F7`, `ROLE_BUFF_CLASSES 0x2A8`, `ROLE_EFFECT_CLASSES 0x2B8`, `ROLE_ABILITY_CLASSES 0x2C8`, `ROLE_MODIFIER_GROUP 0x2F8`, `ACTOR_NET_DORMANCY 0xF1`, `ACTOR_FLAGS_5C 0x5C` (destroying `0x08`), object flags `RF_BEGIN_DESTROYED 0x8000`, `RF_FINISH_DESTROYED 0x10000` | 7, 8 |
| modifier group | `GROUP_SPAN 0xD0`, applied effects `0x68`, abilities `0x78`, buffs `0x88`; pending buffs `0xA8`, effects `0xB8`, abilities `0xC8` | 8 |
| ability system | `ASC_OWNER_ACTOR 0x3D0` (span `0x3E0`) | 10 |
| movement | `PAWN_CAN_MOVE 0x15E0`, `PAWN_PREVIOUS_CAN_MOVE 0x1604` (span `0x1608`), `PAWN_CHARACTER_MOVEMENT 0x288`, `PAWN_ROOT_COMPONENT 0x130`, `PAWN_REPLICATED_MOVEMENT_MODE 0x328`, `ACTOR_ROLE 0xF0`, `CMC_MOVEMENT_MODE 0x168`, `CMC_DEFAULT_LAND_MOVEMENT_MODE 0x3B8` (span `0x3C0`), `SCENE_MOBILITY 0x14F` (span `0x150`) | 11.6 |
| HUD | `HUD_GAMEPLAY_VISIBILITY 0x361` (span `0x362`), `GAME_STATE_IN_PRE_MATCH_AIRSHIP 0x2D0` (span `0x2D1`) | 12 |

Where R7, R9, R12 or R17 already defines a name for the same fact (`APlayerController.Pawn`, `APawn.PlayerState`, the
`TArray` and `FString` layouts), use theirs.

---

## 19. Size

Rust, without tests: the DLL crate about 750 lines (PR-1 and the retry 130, the server pump, refresh and Tempest 200,
RG-3 30, the ability route 60, the input cascade, lifecycle and movement 220, the HUD 80, registration 30), the core
crate about 550 (the three tables and their timing 200, the input and HUD state machines 150, decisions over reads 80,
lines, budgets and flags 120). Tests about 650. Far below the surface's 3 000: the diagnostics, the dead progression HUD
refresh, the per-pair routing and its performance snapshot, and the structured exception wrappers are not ported.
