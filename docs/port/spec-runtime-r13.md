<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R13, relevance and weapons (RD27, plan item 11)

R13 is the relevance slice of the MIT game runtime (`spec-runtime-surface.md` section 3.8 and section 8's R13 row,
approved 2026-11-13 with rulings RT-1 to RT-18 in `rulings-2026-11-13.md`). On a server it decides, at the one place
the game's replication graph files a newly registered actor, which node the actor goes to and how long its channels
survive a relevancy gap; and once a frame it repairs a player's current weapon that the graph filed where no client
will ever gather it. Without it a hunt with a hammer never finishes loading (the weapon is never sent), a player's
omnicell comes and goes, a remote player stands in the reference pose after an escalation transition, and the hub
hands a client another player's controller. It owns the hook RG-2 and the feature id `SP-3@R13`.

An implementer builds from this page, `spec-runtime-r0.md`, `spec-runtime-r9.md` section 8, `spec-runtime-r12.md`
sections 5, 11 and 15, section 3 of `spec-runtime-r17.md`, section 3.4 of `spec-runtime-r7.md` and the surface page
alone, and never reads `restricted/`. What crossed is facts: RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7, CL
647472), instruction bytes and `.pdata` entries (each checked against the maintainer's copy of that executable on
2026-11-13), engine layout offsets, engine and game names, flag names, and behaviour. The AGPL side stands down through
seam patch 0041, the merged patches 0052 and 0054, and R13's patch 0057 (section 13), which stands the AGPL's
relevance settings pass down when the MIT owns `SP-3@R13`.

Conventions are R0's: an RVA is `+0x0XXXXXXX` from the executable's image base; **S** server. **(call)** marks a
decision this page makes; **(ambiguity)** something the source did not settle; **(departure)** a place the MIT
runtime deliberately behaves differently from the AGPL one. All three are collected in section 17.

---

## 1. Scope

| R13 owns | Role | Section |
| --- | --- | --- |
| The graph's settings toolkit: find an actor's global replication info, find a connection's manager and its per-actor info, apply the cull and channel-grace settings, clear a stale per-connection cull snapshot | S | 5 |
| RG-2: `UArchonReplicationGraph::RouteAddNetworkActorToNodes`, the route at registration (controllers, player states, player roles, weapons, player characters; patches 0015 and 0016 at registration) | S | 6 |
| `SP-3@R13`, after the engine tick: the weapon wake (patches 0015 and 0016, the tick half) | S | 7 |
| `SP-3@R13`: the player pawn update-frequency write (from R8's block, patch 0054) | S | 8 |
| `SP-3@R13`: the relevance settings pass, its role and pawn halves (patch 0057 stands the AGPL's down) | S | 9 |
| Base-actor routing: owned (the AGPL stops running it), not ported | S | 10 |

**Not R13, though the surface page or Appendix A puts them near it:**

- **Patch 0014** (the server's weapon gap log) is logging only: **dropped, not ported (RT-3)**. Its client half (0013)
  went with R7.
- **The default-object boost** (`Default__BP_PlayerCharacter_C`'s update frequencies and `bAlwaysRelevant` bit, before
  the graph is made) is R11's, with NETD-2 (R11 9.1). R13 keeps only the per-tick write on live pawns (8).
- **The owner-pawn relevancy fix** (pointing the owning connection's view target at its pawn at `ClientRestart`) runs
  inside the server `ProcessEvent` handler `SP-2@R20`; it is **R20's** with its flag (12).
- RG-3 is R14's (R12 12.2); the graph drive and the controller channel guards are R12's.

---

## 2. Where it goes

### 2.1 Files

| Crate | Path | Holds |
| --- | --- | --- |
| `rustic-isles-runtime-core` | `src/relevance.rs` (a folder `src/relevance/` past ~400 lines) | everything pure: the map search over R0's `Memory` (5.1), the settings decision (5.3), the route decisions per class (6), the weapon wake decision and its "woken" ring (7), the pass's change test (9), the line builders and budgets, the flag table (12) |
| `rustic-isles-runtime` | `src/ffi/relevance/` (`mod.rs`, `route.rs`, `tick.rs`) | the RG-2 detour, the SP-3 handler, the reads and checked writes of game memory, the one `ProcessEvent` call (7.3); under `ffi` because all of it is FFI (R0 2, RT-1) |

`core/src/lib.rs` gains `pub mod relevance;`, `ffi/mod.rs` gains `pub mod relevance;`.

### 2.2 Shared lines R13 adds

| File | Edit |
| --- | --- |
| `core/src/registry.rs` | `LANDED` gains `"R13"` **only once it holds `"R12"`** (13.2). RG-2 and `SP-3@R13` exist already. One new test (15.1) |
| `core/src/offsets.rs` `TARGETS` | one line, RG-2, `FoundBy::RvaAndBytes` (16.1) |
| `core/src/offsets.rs`, a new `pub mod relevance` | the layout offsets and constants of 16.2 |
| `src/ffi/reflect.rs` | `process_event_original(object, function, params)` exactly as R7 3.4 defines it, if no slice has added it yet |
| `src/lifecycle.rs` | before `handlers::freeze()`: `ffi::relevance::register()` (the SP-3 handler, server only). In the install list: RG-2 at the position of 4.4. Nothing else changes there |

`owned_count()` then counts 2 more ids (RG-2, `SP-3@R13`).

### 2.3 What is pure and what is Windows-only

Pure (core, `cargo test` on every platform): every decision on this page over values read through R0's `Memory` trait
(a fake memory in tests), the map search, the woken ring, the change test, every log text, the budgets, the flag
table. Windows-only (DLL crate): the detour, the writes of game memory, the `ProcessEvent` call. Nothing in R13 runs
against the real executable in CI (surface 9).

---

## 3. What R13 consumes from other slices

| From | What | Used in |
| --- | --- | --- |
| R0 | the broker (`broker::hook`, `original`), `handlers::register_tick`, `ffi::memory` (`readable`, `readable_this_frame`, `read`, `verify_entry`, `frame_tick`), `is_live_object` and `is_actor_alive_for_replication` (R0 5.3), `sane_pointer_array`, the object names (`engine`), `flags::flag` (the 4 s cache) and the read-once form, the log, the detaching flag, `lifecycle::write_status_once` | everywhere |
| R9 | `listen::net_driver()`, `listen::client_connections(driver)` (R9 8.1, 8.3) | 7, 8, 9 |
| R12 | `repgraph::graph() -> Option<usize>` (the graph of R9's driver after R12 11.2's checks, computed on call); the class-pointer `is_a` walk and its class memo (R12 5.2), if R12 exposes them; the layout names `GRAPH_CONNECTIONS` (`0x38`, `Num` `0x40`, `Max` `0x44`) and `CONN_MANAGER_CONNECTION` (`0x28`) (R12 17.2) | 5, 6, 7, 9 |
| R17's reflection kit (`reflect.rs`, R17 section 3) | `find_class(name)`, `is_a`, `find_function(class, name)`, `params_fit`, the checked `write<T>` (R17 3.5) | 6, 7 |
| R7 | `process_event_original` (R7 3.4; R13 adds it if R7 has not landed, 2.2) | 7.3 |
| R11 | nothing called; R11 reads `DISABLE_PLAYER_REP_BOOST` once for the default object, R13 reads the same file once for the live pawns (8) | 8 |

R13 does not call R12's `is_quarantined`: the weapon wake never touches a channel, and the route runs before any
channel exists **(call)**.

---

## 4. Common rules for R13's hook and handler

### 4.1 Before anything else

1. **Detaching** (R0 3.3): if set, the detour calls `next` with its arguments; the tick handler returns.
2. Call R0's `lifecycle::write_status_once()` (first use only).
3. The work runs inside `catch_unwind`; a panic logs `[Panic] <id> ...` once (R0 2), then the detour calls `next` with
   its arguments (no route correction for that actor), the handler returns.

Every game read goes through R0's checked reads, every write through R17's checked `write<T>` (readable and writable);
a read or write that fails ends that piece of work as the section says. No R13 code catches a hardware fault, as in R11
and R12.

### 4.2 Install

RG-2 is `broker::hook`ed with `order = 0`, band `Mit`, after `verify_entry(rva, bytes)` of 16.1 succeeds. A failed
check reports `prologue mismatch` and RG-2 is not hooked. The AGPL hooked RG-2 by bare RVA **(departure: R13 checks
the bytes)**. Because the MIT owns RG-2 either way, a failed check means **no** route correction on that server, MIT or
AGPL: the install line says so, and the play-test's first step looks for it.

### 4.3 Lines

Every R13 line carries the tag `[MitRelevance]`. Addresses print as `0x<HEX>` (upper case, no padding; `null` for 0),
flag bytes as `0x<HH>`, counts in decimal, floats with no decimals (`%.0f`), names by R0's `engine` names (`null` or
`unreadable` when they cannot be read). A line that can repeat has a **budget**, as R12 4.3: the first `B` lines of
that kind go out, the `B+1`-th becomes `[MitRelevance] <kind>: further lines suppressed`, the rest are dropped. Budgets
are per process, relaxed atomics. Each section names its budget **(departure: the AGPL logs some of these on every
occurrence and adds a membership read after each role and player state route, which is not ported, RT-3)**.

### 4.4 Threads and install order (RT-17 for R13)

RG-2 runs on the server's game thread (actor registration and the graph's initialisation of the world); the handler
runs in SP-3. R13's tables (the woken ring, the pass's last inputs) are game-thread state, never locked. The class
lookups are found once and kept in once-cells.

**Settled:** RG-2 has no order dependency on another id; it must be in place before the first actor registers with the
graph, which every install in `DllMain` is (the graph is made at map load). The MIT build keeps the AGPL's position
(surface 3.11): after R12's RG-1 line, before RG-3. One install line through R0's status
(`[MitRuntime] install RG-2 ok|<reason>`). R13 installs nothing on a client.

---

## 5. The graph's settings toolkit (S)

Pure decisions over R0's `Memory`, used by RG-2, the tick, and offered to R14 (14).

### 5.1 Finding a value in an engine `TMap` (UE 4.26)

A map at `base` whose elements are `{ key: pointer, value: pointer, hash }` at a fixed `stride`. The layout (the
`TSet`'s sparse array): `Data` pointer `+0x00`, `ArrayNum` `i32` `+0x08`, `ArrayMax` `i32` `+0x0C`; the allocation
bit array: four inline `u32` words at `+0x10`, a secondary word pointer `+0x20` (null while inline), `NumBits` `i32`
`+0x28`.

`map_find(base, key, stride) -> Option<usize>`:

1. `base` and `key` non-null; `readable(base, 0x30)`.
2. `num` = `ArrayNum`; require `0 < num <= 65 536` and `ArrayMax >= num`; `Data` non-null and readable for
   `num * stride`.
3. `count` = `NumBits` when `0 < NumBits < num`, else `num`.
4. The words: the secondary pointer when non-null (readable for `ceil(count / 32) * 4` bytes, else `None`); else the
   inline words, and `count` is capped at 128.
5. For `i` in `0..count`: skip when bit `i` is clear; the element at `Data + i * stride`; when its first pointer equals
   `key`, answer the pointer at element `+0x08`.
6. `None`.

Nothing but the caller's own key is compared, and nothing but the answer is dereferenced by the search: a malformed
map answers `None`, never a fault.

### 5.2 The graph's maps

| Lookup | Map | Stride | Requires |
| --- | --- | --- | --- |
| `global_info(graph, actor)`: the actor's `FGlobalActorReplicationInfo` (`0x110` bytes) | `graph + 0xC0` | `0x18` | `readable(graph, 0x118)` |
| `connection_manager(graph, connection)`: the `UNetReplicationGraphConnection` serving `connection` | the graph's `Connections` array (`GRAPH_CONNECTIONS`), sane with limit 1 024 (`graph` readable for `0x48`); the first manager readable for `0x30` whose `+0x28` equals `connection` | — | — |
| `connection_info(manager, actor)`: the connection's `FConnectionReplicationActorInfo` (`0x30` bytes) | `manager + 0x30` | `0x20` | — |

### 5.3 `apply_settings(global_info, zero_cull, min_grace) -> -1 | 0 | 1`

On a global info (requires `readable(info, 0x110)`, else `-1`): `cull` = `f32` at `+0x88` (`CullDistance`), `cull_sq` =
`f32` at `+0x8C` (`CullDistanceSquared`), `grace` = `u16` at `+0x30` (`ActorChannelFrameTimeout`).

- `zero_cull`: write `0.0` to both floats.
- **Raise only:** when `min_grace != 0`, `grace != 0` and `grace < min_grace`, write `min_grace`. A class that asks for
  a longer grace keeps it; a class with `0` (never time the channel out) stays `0`.
- Answer `1` when any of the three values changed, `0` when all were already right. The before and after values are
  returned for the line that logs them.

### 5.4 `clear_connection_cull(graph, actor) -> count`

For every manager in the graph's `Connections` (sane, limit 1 024; `graph` readable for `0x4C8`) that is readable for
`0xE0`: `info` = `connection_info(manager, actor)`, readable for `0x30`; if its `f32` at `+0x08` or `+0x0C` is non-zero,
write `0.0` to both and count it. Why: a connection's actor info is a **snapshot** of the global settings taken the
first time that connection gathers the actor, so clearing the global cull afterwards does not reach a connection that
already has one.

### 5.5 Why these settings (facts of this build)

The graph's per-connection gather (`ReplicateActorListsForConnections_Default`) builds a connection's actor info from
the global info the first time it gathers the actor, skips the actor when its cull distance is set and every viewer is
beyond it, and only for an actor it did not skip pushes the channel's close frame forward by
`ActorChannelFrameTimeout` + the period + the graph's global timeout. `ServerReplicateActors` then closes every channel
whose close frame has passed, with reason `Relevancy`, and a client that receives a non-dormancy close **destroys** the
actor. So routing an actor to a node decides only whether it is gathered; the cull distance and the grace decide
whether it then survives.

`PLAYER_CHANNEL_GRACE_FRAMES = 300`: 10 s at 30 Hz, long enough to bridge an escalation's synchronised island
transition, short enough that a player who is gone still releases the channel. The stock grace is about 7 replication
frames.

---

## 6. RG-2: the route on `RouteAddNetworkActorToNodes` (S)

| | |
| --- | --- |
| Target | `+0x01BFE430`, `UArchonReplicationGraph::RouteAddNetworkActorToNodes`, virtual table slot `+0x308` of the Archon graph (R12 11.2) |
| Signature | `extern "C" fn(graph, info: *const NewReplicatedActorInfo, global_info)`, no return value. `info` is the engine's `FNewReplicatedActorInfo` (`0x18` bytes): `Actor` `+0x00`, `StreamingLevelName` (`FName`) `+0x08`, `Class` `+0x10`. `global_info` is the actor's `FGlobalActorReplicationInfo` |
| Caller | `UReplicationGraph::AddNetworkActor` (`+0x00F12D40`), which has just set the global info's `bWantsToBeDormant` (`+0x14`) to "NetDormancy (`+0xF1`) is above `DORM_Awake`" |
| Gate | none (12) |

**What the route does with an actor (fact).** It reads the actor's flag byte `+0x58`: bit `0x08` (`bAlwaysRelevant`)
files it in the graph's global always-relevant node; else bit `0x04` (`bOnlyRelevantToOwner`) files it for its owning
connection; else it goes to the spatial grid, whose dormancy-aware add (`+0x00F12820`) files a dormant actor as
**static**: its grid cells are picked once, from where it stands then, and never follow it. Changing the bits after
the route has run leaves the actor in the node it was first filed in, so every correction happens **before** the
original is called.

### 6.1 Behaviour

1. `actor` = the pointer at `info + 0x00` when `readable(info, 0x18)`, else 0. When `actor` is 0 or not
   `is_actor_alive_for_replication(actor)`: call `next(graph, info, global_info)` and return.
2. Classify, in this order, and take the **first** branch that applies (class tests through the kit: classes found once
   with `find_class`, then a class-pointer `is_a`; while a class is not found yet its test answers false and the search
   is retried at most once a second):
   1. `PlayerController` → 6.2
   2. `PlayerState` → 6.3
   3. `ArchonPlayerRole` → 6.4
   4. `ArchonWeapon` → 6.5
   5. `ArchonCharacter` → 6.6
   6. anything else: nothing.
3. Call `next(graph, info, global_info)`.

The step order and every decision are pure (core) over the read values; the DLL crate reads, writes and calls.

### 6.2 A PlayerController: only to its owner

Write `flags58 = (before | 0x04) & !0x08`. The route tests always-relevant first, so both bits set would still file
the controller globally. Why (measured, 2026-07-26, two-player escalation): a controller filed anywhere but its owner's
connection reaches another client, whose stock `OnActorChannelOpen` → `HandleClientPlayer` adopts it; the amp choice
then freezes. R12's guards (RG-4..7) are the backstop behind this; this is the first line.

Line (budget 64): `[MitRelevance] route: controller actor=<p> connection=<p at actor+0x418> flags58=0x<HH>->0x<HH>
route=owner`.

### 6.3 A PlayerState: globally relevant, never culled

- `flags58 = (before | 0x08) & !0x04`; `flags59` (the byte at `+0x59`) `= before & !0x02` (`bNetUseOwnerRelevancy`
  cleared, so the owner's relevancy is not consulted).
- `apply_settings(global_info, zero_cull: true, min_grace: 300)`.

Why: everything a client builds about its own player hangs off its replicated player state (the role id and its
`OnRep`, the load manager's identity gate, the loading participants). A player state has no meaningful place for the
grid to gather it from. Measured (2026-10-05): with no player state the client's readiness poll can never finish.

Line (budget 12): `[MitRelevance] route: player-state actor=<p> class=<short name> flags58=0x<HH>->0x<HH>
flags59=0x<HH>->0x<HH> settings=<-1|0|1> cull=<a>-><b> cullSq=<a>-><b> grace=<a>-><b> route=global`.

### 6.4 A player role (omnicell): globally relevant, never culled

The same writes and settings as 6.3 (`flags58 | 0x08 & !0x04`, `flags59 & !0x02`, `apply_settings(.., true, 300)`).

Why (measured, 2026-07-26): a role is spawned at the world origin and keeps the class's spatial cull distance in every
connection's snapshot. Routed globally but still culled, both of a player's role channels closed together for one
connection whenever that player stood more than the cull distance from the origin; the client destroyed its role
replica, the character's role went null and the HUD unbound. `bNetUseOwnerRelevancy` left set made the global role
re-evaluate through its owning pawn and churn its channels. The role's `Owner` is not touched (gameplay and RPC
ownership stay).

Unconditional: `DISABLE_PLAYER_ROLE_GLOBAL_ROUTE`, which turned this route off for an A/B, is dropped with R14's
agreement (12).

Line (budget 64): `[MitRelevance] route: role actor=<p> flags58=0x<HH>->0x<HH> flags59=0x<HH>->0x<HH>
settings=<-1|0|1> cull=<a>-><b> cullSq=<a>-><b> grace=<a>-><b> route=global`.

### 6.5 A weapon (patches 0015 and 0016, at registration)

Read `flags58`; `dormancy` = `u8` at `actor + 0xF1` (`NetDormancy`: 0 `DORM_Never`, 1 `DORM_Awake`, 2 `DORM_DormantAll`,
3 `DORM_DormantPartial`, 4 `DORM_Initial`); `g` = `global_info` when `readable(global_info, 0x18)`, else 0.

- **Dormancy (0015).** When `flags58 & 0x0C == 0` (the spatial route), `g != 0` and `dormancy > 1`: write `1` to
  `actor + 0xF1` and `0` to `g + 0x14`; `woke` = both writes succeeded. The weapon is then filed as a dynamic actor,
  whose cell follows it every frame.
- **Cull (0016).** When `flags58 & 0x08 != 0` (the global route) and `g != 0`: `apply_settings(global_info, true, 0)`
  (cull cleared, grace untouched).

Why (measured, 17 Oct 2026, every hunt with a hammer, `WP_IH_BEGINNER` on a new account and `WP_IH_ROCK`, timing out
in loading): a weapon is spawned before it is attached to its pawn, at its spawn point. **Dormant**, the grid filed it
static there, and a player who starts a hunt far from that point never gathers it; the armour and lantern, awake, follow
their pawn. **Always relevant** (the hammer registers with `flags58 = 0x88`), it sits in the global node, which still
applies the class cull distance (`225000000`, 15 000 units squared) measured from the global info's `WorldLocation`
(`+0x08`), which only the grid updates: it stays at the world origin, and the hunt map's start (about 24 000 units away)
is beyond it. Either way the client never gets a current weapon and loading never finishes. In the city both points
happen to be close enough.

Line (budget 32): `[MitRelevance] route: weapon actor=<name> owner=<name of actor+0xE0> dormancy=<a>-><b>
wantsDormant=<a>-><b> woke=<0|1> cull=<-2|-1|0|1> route=<global|owner|spatial-static|spatial-dynamic>` (`cull` is
`-2` when 0016 did not apply; `route` from the bits after the writes, `spatial-static` when `g + 0x14` reads 1).

### 6.6 A player character: a longer channel grace

`apply_settings(global_info, zero_cull: false, min_grace: 300)`. The character stays on its spatial route; nothing makes
it globally relevant (the old global always-relevant pawn hid armour in co-op).

Why (measured, 2026-07-26, escalation): a ready transition moves both players about 10^6 units in one instant; for a
few frames a remote pawn is beyond the viewer's cull distance, the stock grace (about 7 frames, ~230 ms at 30 Hz)
runs out, and the client destroys and rebuilds the pawn. The character's role (`+0x738`) is not replicated and its
setter is authority-only, so the rebuilt pawn never regains its role and stays in the reference pose. Behemoths derive
from a sibling class (`ArchonCreatureBase`) and are not touched.

Line when the answer is not `0` (budget 32): `[MitRelevance] route: character actor=<p> class=<short name>
settings=<-1|1> grace=<a>-><b>`.

---

## 7. `SP-3@R13`: the weapon wake (S, patches 0015 and 0016, the tick half)

### 7.1 Registration and position

`handlers::register_tick`, `slice: "R13"`, `order: 400`, `after` only **(call)**: below R12's sweep and drive (500),
as R12 15 asked, which is the AGPL's order (its weapon blocks ran right before the drive). When R13 is in `LANDED`,
`SP-3@R13` is owned and the AGPL skips its weapon gap log, weapon wake and base-actor routing (seam 0041) and its
per-tick pawn write (patch 0054). The one handler runs sections 7, 8 and 9 in that order.

Each frame: `driver` = R9's `net_driver()`, `None` → return (not listening yet). `connections` =
`client_connections(driver)`.

### 7.2 Every 250 ms

The wake runs when at least 250 ms (`GetTickCount64`) passed since its last run. `graph` = R12's `repgraph::graph()`;
`None` → return. For each `connection` in `connections`:

1. `connection` readable for `0x98` and `is_live_object`; `pc` = the pointer at `+0x30`, readable for `0x420`; `pawn` =
   the pointer at `pc + 0x250`, readable for `0x748`, live, `is_a ArchonCharacter`. Else next connection.
2. `weapon` = the pointer at `pawn + 0x5F0` (`AArchonCharacter::Weapon`): readable for `0x300`,
   `is_actor_alive_for_replication`, `is_a ArchonWeapon`. Else next.
3. **Once per weapon.** The woken ring (16 entries of `(weapon, object index at +0x0C)`, overwritten in turn) holds it →
   next.
4. `g` = `global_info(graph, weapon)`, readable for `0x110`; none → next (not registered with the graph yet).
5. `dormancy` = `weapon + 0xF1`; `wants` = `g + 0x14`; `flags58`; `cull_sq` = `f32` at `g + 0x8C`.
   `needs_wake` = `dormancy > 1`; `needs_cull` = `flags58 & 0x08 != 0` and `cull_sq != 0.0`. Neither → next.
6. `manager` = `connection_manager(graph, connection)`; none → next. `info` = `connection_info(manager, weapon)`; when
   it is readable for `0x30` and its channel (`+0x00`) is non-null or its `LastRepFrameNum` (`i32`, `+0x14`) is non-zero
   → next: **this connection has had the weapon already**. A weapon that was sent once and then went dormant normally
   (the city) is left alone.
7. Record `(weapon, index)` in the ring.
8. `needs_cull`: `apply_settings(g, true, 0)` and `clear_connection_cull(graph, weapon)`; line (budget 32)
   `[MitRelevance] weapon: cull cleared weapon=<name> pawn=<name> connection=<p> flags58=0x<HH> global=<-1|0|1>
   snapshotsCleared=<n> info=<unsent|none>`.
9. `needs_wake`: wake it (7.3); line (budget 32) `[MitRelevance] weapon: woke weapon=<name> pawn=<name>
   connection=<p> dormancy=<a>-><b> wantsDormant=<a>-><b> info=<unsent|none> call=<made|not found|skipped>`.

Why the tick half: the route fixes a weapon that is dormant when it registers; a weapon can also be put to sleep later
(its Blueprint calls `SetNetDormancy`, for instance while it still stands at its spawn point), and the grid then
refiles it static where it is.

### 7.3 The wake call

Through the engine, not by hand: `AActor::SetNetDormancy(DORM_Awake)` goes through
`UReplicationGraph::NotifyActorDormancyChange`, whose dormancy event makes the grid move the actor from its static cells
to the dynamic list itself; no graph state is written by hand.

- `function` = `find_function(class of weapon, "SetNetDormancy")` (exact name, on the class chain; it is declared on
  `Actor`), cached once found and found again if the cached function is no longer live.
- Parameters: an 8-byte zeroed block, byte 0 = `1` (`DORM_Awake`); `params_fit(function, 8)` (R17 3.3).
- `process_event_original(weapon, function, block)` (R7 3.4: the original past every wrapper, so no SP-1/SP-2
  handler, MIT or AGPL, sees the call, as in the AGPL).
- After the call: R0's frame epoch bump (`frame_tick(true)`): game code ran (R11 8.3's rule).
- `call=` is `made`, `not found` (no function) or `skipped` (`params_fit` refused).

### 7.4 Which servers **(departure)**

The AGPL ran the wake only on Director servers (`RUSTIC_ISLES_AUTH_MODE` global game server), not on a player host.
R13 runs it on every server: a realm's hosted hunt loads the same maps with the same weapons, and the registration half
(6.5) already runs everywhere. The play-test covers a hosted hunt (17.1 step 7).

---

## 8. `SP-3@R13`: the player pawn update-frequency write (S)

Each frame (no throttle, as in the AGPL), unless `DISABLE_PLAYER_REP_BOOST.flag` was present at the first read (read
once per process; R11 reads the same file once for the default object):

For each `connection` in `connections` (7.1): `pc` = the pointer at `connection + 0x30`, live; `pawn` = the pointer at
`pc + 0x250`, live, `is_a BP_PlayerCharacter_C`; when `readable(pawn + 0x108, 8)`: write `60.0` to the `f32` at
`+0x108` (`NetUpdateFrequency`) and `30.0` to the `f32` at `+0x10C` (`MinNetUpdateFrequency`). No line.

Why (measured, 2026-07-17): player pawns replicated at about 10 Hz (period 3) against a behemoth's 30 Hz, and remote
players jittered; the server's `NetServerMaxTickRate` (30) caps the result at period 1.

**(ambiguity)** The AGPL's own note on the default-object boost says the graph fixes an actor's replication period at
registration from its class default, so the per-tick write on a live pawn may change nothing once R11's default-object
boost has run. R13 keeps it **(call)**: it costs one pair of writes per player per frame, and dropping it is a change no
play-test has measured.

---

## 9. `SP-3@R13`: the relevance settings pass (S)

### 9.1 What it is

The AGPL's relevance settings pass re-asserts, once a second, the settings the route already wrote at registration:
(1) every player role's settings and every connection's cull snapshot of it, (2) every possessed player pawn's channel
grace, (3) a read-only diagnostic. R13 ports (1) and (2); (3) is not ported (RT-3).

The AGPL's own logs show the pass never had to correct anything from 26 Jul to 11 Oct 2026 (zero lines in the hub logs
of 9–11 Oct). It is a safety net for an actor registered before the route hook or re-registered by a graph rebuild.

In the AGPL the pass is called only from inside R14's once-a-second role tick (`SP-3@R14`), and it walks R14's role
route table for (1). The MIT side does neither (9.3 finds the roles from the pawns), and patch 0057 makes the AGPL pass
stand down when the MIT owns `SP-3@R13` (13.1), so the two never both run. R14 lands with or after R13 (R14's spec),
so an MIT R14 never switches off an AGPL R13 pass.

### 9.2 When

At most once a second (`GetTickCount64`), and then only when the inputs changed or 15 s passed since the last run:
the inputs are the graph pointer and the ordered list of `(connection, pawn, role)` triples read as in 9.3 (0 for what
cannot be read). The last inputs are kept; equal inputs and less than 15 s → nothing. The first call always runs and
logs once `[MitRelevance] relevance re-assert on change, else every 15 s`. **(call)** The AGPL's order-independent
hash is replaced by a direct comparison of the list; the effect is the same.

### 9.3 Each run

`graph` = `repgraph::graph()`; `None` → return. For each `connection` in `connections`: readable for `0x38`; `pc` = the
pointer at `+0x30`, readable for `0x258` and live; `pawn` = the pointer at `pc + 0x250`, readable for `0x748`, live, and
`is_a ArchonCharacter` **(call: the AGPL took any pawn; RG-2 sets the grace only on characters, so the re-assert
matches it)**.

1. **The pawn.** `apply_settings(global_info(graph, pawn), false, 300)`; when it answers `1`: line (budget 32)
   `[MitRelevance] pawn grace: corrected pawn=<p> connection=<p> grace=<a>-><b>`.
2. **Its role.** `role` = the pointer at `pawn + 0x738` (`AArchonCharacter::PlayerRole`); non-null,
   `is_actor_alive_for_replication`, `is_a ArchonPlayerRole`, and not already handled in this run (a role seen through
   a second connection is skipped). `global` = `apply_settings(global_info(graph, role), true, 300)`; `cleared` =
   `clear_connection_cull(graph, role)`. When `global` is `1` or `cleared > 0`: line (budget 32) `[MitRelevance] role
   cull: corrected role=<p> pawn=<p> global=<-1|0|1> snapshotsCleared=<n> cull=<a>-><b> cullSq=<a>-><b>
   grace=<a>-><b>`.

**(departure)** The AGPL found the roles in R14's route table, which holds every role its `ApplyPlayerRole` routed on
the server. R13 reads each connected player's current role from the pawn instead, so it needs no R14 state; a role a
player no longer holds is not re-asserted (it is no longer drawn), and a role whose pawn has no connection is not a
player's. A line in either case means something reset settings after registration: worth a note in the play-test.

## 10. Base-actor routing: owned, not ported **(call)**

The AGPL's `SP-3@R13` also held, before the engine tick, an opt-in routine (`ENABLE_BASE_ACTOR_BOOTSTRAP.flag` or
`ENABLE_HUB_REP_DIAGNOSTICS.flag`, both default off) that every 2 s added the game state and every player state to each
connection's always-relevant node (`+0x01BDB8F0`, the node's `NotifyAddNetworkActor` at virtual table `+0x270`) and,
with the bootstrap flag, opened their actor channels by hand, once per pair. It was a 2026-10-05 experiment for "no
player state reaches the client". R12's graph drive is what fixed that (R12 11.3: channels went from 2–5 to 89–100 and
Ramsgate became playable), and the by-hand bootstrap is the kind of channel push no slice may do on 1.14.7 (R9 8.3; its
first form killed a hub with `0xC0000409`).

So R13 owns it (owning `SP-3@R13` stops the AGPL running it) and ports nothing; both flags are dropped (12). The
AGPL's membership report behind `ENABLE_HUB_REP_DIAGNOSTICS` is a separate, ungated diagnostic and stays AGPL until
R21.

---

## 11. RG-2 and R12, R14 at the same boundary

- R12's guards (RG-4..7) never read what RG-2 writes, and RG-2 never reads R12's quarantine. Either may be MIT while
  the other is AGPL (R12 1); R13 still lands after R12 because its tick reads `repgraph::graph()` (13.2).
- R14's role routing (AGPL until R14) decides between the global node and a per-connection node from the role's
  `flags58 & 0x08`, which RG-2 sets exactly as the AGPL did. R14's MIT side drops the per-connection fallback and
  reads no flag for it (R14's spec).
- R14's RG-3 purge runs on route removal; RG-2 is route addition. No shared state.

---

## 12. Flags (RT-14 for R13)

Appendix A gave R13 eleven flags; one more (R14's) was read by the AGPL's RG-2. Settled:

| Flag | Kept? | Meaning in the MIT runtime |
| --- | --- | --- |
| `DISABLE_PLAYER_REP_BOOST` | kept | read once: no per-tick pawn write (8). R11 reads the same file once for the default object (R11 9.1) |
| `DISABLE_PLAYER_ROLE_GLOBAL_ROUTE` | **dropped** | Appendix A's R14 flag, read by the AGPL's RG-2 and its R14 role routing to go back to per-connection role routes for an A/B; R14's spec drops it too, so neither MIT side reads it. It can only bring the omnicell coming and going back **(ruling)**. While R14 is AGPL its routing still reads the file, so a host must not create it (the two halves would disagree) |
| `PLAYER_ALWAYS_RELEVANT` | **moves to R11** | its only reader is the default-object boost, which R11 ports (R11 9.1); R13 has no code that reads it |
| `DISABLE_OWNER_PAWN_RELEVANCY` | **moves to R20** | its only reader is the owner-pawn view-target write inside `SP-2@R20`'s `ClientRestart` handling (1) |
| `DISABLE_WEAPON_DORMANCY_FIX` | **dropped** | turned off 0015 at registration and the tick wake; it can only bring back hunts that never load with a hammer **(ruling)** |
| `DISABLE_WEAPON_CULL_FIX` | **dropped** | turned off 0016 at registration and in the tick; same reason **(ruling)** |
| `DISABLE_REPGRAPH_RELEVANCE_FIX` | **dropped** | turned off the settings writes of 6.3, 6.4, 6.6 and the relevance pass (9); it can only bring back the omnicell coming and going and the reference-pose pawns **(ruling)** |
| `DISABLE_SERVER_WEAPON_GAP_LOG` | **dropped** | 0014 is not ported **(RT-3)** |
| `RELEVANCE_PASS_EVERY_TICK` | **dropped** | an A/B switch for the re-assert's cadence (diagnostics, RT-3) |
| `ENABLE_BASE_ACTOR_BOOTSTRAP` | **dropped** | 10 |
| `ENABLE_PLAYERSTATE_OWNER_ROUTE` | **dropped** | a 2026-10-05 experiment that put the player state on its owner's route, which hides a player's state from every other player; the global route works since the graph drive (6.3) |
| `ENABLE_HUB_REP_DIAGNOSTICS` | **dropped** on the MIT side | diagnostics (RT-3); 10 |

A dropped flag left in a game folder has no effect on the MIT side. While the AGPL still runs a block that reads one
(R14's role routing), the AGPL still honours it there.

---

## 13. The AGPL side and the other slices

### 13.1 How the AGPL stands down (seam patch 0041, 0052, 0054, and patch 0057)

- RG-2 is in 0041's RVA table. When R13 owns it, the AGPL makes no wrapper; its "original" pointer becomes
  `rir_original(target)` and its enable call succeeds as a no-op. Its install line still logs `create=MH_OK`.
- `SP-3@R13` gates the AGPL's base-actor routing (before the engine tick) and its weapon gap log and weapon wake (after
  it) (0041), and its per-tick pawn write in the connected-server block (0054).
- **Patch 0057 (new):** the AGPL's relevance settings pass is called from inside `SP-3@R14`'s block, so 0041's gate on
  `SP-3@R13` does not reach it. With `SP-3@R13` owned by the MIT, 0057 makes the pass skip its role and pawn parts
  (it still runs its read-only drone diagnostic). Without the MIT runtime, or with R13 not landed, nothing changes.
  Recorded in `restricted/docs/paradox-patches.md`.
- What stays AGPL and is harmless: the pass's read-only drone diagnostic; the membership report behind
  `ENABLE_HUB_REP_DIAGNOSTICS`; the default-object boost (R11's, NETD-2).
- The AGPL's driver pointer is R9's (0052), which its remaining blocks read; R13 reads R9's and R12's interfaces.
- No AGPL code reads anything R13 keeps (the woken ring, the pass's last inputs).

So the AGPL needs 0041, 0052, 0054 and **R13's patch 0057**. 0057 was not built or run here: the C++ needs the
maintainer's SDK, and the play-test (17.1) is its check.

### 13.2 R13 lands with or after R12 **(call)**

R13's tick needs R12's `repgraph::graph()`, and R12 needs R11, R11 needs R9. So `LANDED` holds `"R13"` only when it
holds `"R12"`, and a registry test enforces it (15.1). The R13 implementer's branch adds `"R13"` only if `"R12"` is
already there on `plan`; otherwise the integration pass adds it when it merges R13 after R12. Until then R13's code
builds and its pure tests run, and it installs nothing.

**R13 with R12 (R14, R20 still AGPL) is safe:** 9.1, 11 and 0057. R14 lands with or after R13 (R14's spec), so the
pass is never lost.

Row for R0 7.5's cross-slice table:

| State | Set by | Read by | Rule |
| --- | --- | --- | --- |
| a role's route bits (`flags58 & 0x08`) | RG-2 | R14's role routing | carried: RG-2 writes the same bits |
| the relevance settings pass | `SP-3@R13` (MIT), or the AGPL pass called inside `SP-3@R14` | — | **patch 0057**: one side runs it; **R14 lands with or after R13** |

### 13.3 Rulings settled here

- **RT-3 for R13:** 0014, the re-assert's role diagnostic, the membership lines after a route, and the base-actor
  routing's report are not ported.
- **RT-14 for R13:** section 12.
- **RT-17 for R13:** 4.4; no order dependency.
- **RT-18 for R13:** R13 registers no `ProcessEvent` handler and matches no names. It calls one function by exact name
  (`SetNetDormancy`, 7.3).
- RT-12, RT-13, RT-15 and RT-16 do not touch R13.

---

## 14. What other slices need from R13

| Slice | Needs |
| --- | --- |
| R14 | nothing called. RG-2's role route (6.4) and the relevance pass's role half (9.3) keep its roles relevant; the toolkit of section 5 (`global_info`, `connection_manager`, `connection_info`, `apply_settings`, `clear_connection_cull`, `PLAYER_CHANNEL_GRACE_FRAMES`) is public should it need one. R14 lands with or after R13 (9.1) |
| R20 | nothing called. `DISABLE_OWNER_PAWN_RELEVANCY` is its flag (12). Its `ClientRestart` pawn writes and RG-2's character grace (6.6) touch different fields |
| R11 | nothing called. `PLAYER_ALWAYS_RELEVANT` is its flag (12); `DISABLE_PLAYER_REP_BOOST` is read by both |
| R12 | nothing called; R13's handler at order 400 < 500 (R12 15) |

---

## 15. Tests

### 15.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`), over a fake `Memory`:

- **Map search (5.1):** a hit; a hole (bit clear) with the key skipped; `NumBits` smaller than `ArrayNum` caps the walk;
  the inline words cap at 128; an unreadable secondary word array, `ArrayNum` 0, above 65 536 or above `ArrayMax`, and
  unreadable data each answer `None`; only the key is compared.
- **`connection_manager`:** found by `+0x28`; an insane array (`Num` 1 025, `Num > Max`) answers `None`.
- **`apply_settings`:** zeroing both floats answers 1, already zero answers 0; the grace raised from 4 to 300, kept at
  600, kept at 0; an unreadable info answers −1.
- **`clear_connection_cull`:** clears only non-zero snapshots and counts them; skips unreadable managers and infos.
- **RG-2 decisions:** each class branch's writes (6.2–6.6) for fixed flag bytes; the branch order (a class that is both
  never happens, but the first test wins); a dead or destroying actor and an unreadable `info` take no branch; the
  role branch is unconditional; the weapon: dormant spatial woken, dormant global not woken but culled, awake spatial
  untouched, `g` unreadable untouched; the route text from the bits.
- **Weapon wake (7.2):** the 250 ms throttle; each skip (no pawn, not a character, no weapon, destroying, in the ring,
  no global info, nothing to correct, no manager, already sent by channel, already sent by frame); the ring wraps after
  16 and a re-seen weapon at a reused address with another index is new.
- **Relevance pass (9):** first run; same inputs within 15 s skip; a changed triple, a changed graph, and 15 s each
  run; only characters; a role reached through two connections handled once; a dead, destroying or non-role pointer at
  `+0x738` skipped; the role line only when the global answer is 1 or a snapshot was cleared.
- **Lines:** every text on this page for fixed values; the budgets.
- **Registry:** if `LANDED` contains `"R13"` it contains `"R12"`; with both, `owns` is true for RG-2 and `SP-3@R13`.
- **Offsets:** RG-2 has one `RvaAndBytes` line with the bytes of 16.1.

DLL (`cargo test -p rustic-isles-runtime`, Windows):

- RG-2's detour with a test `info` whose actor is a test-allocated block that is not a live object calls a test `next`
  with the same three arguments and writes nothing.
- The wake's call path refuses when `params_fit` fails (a test function block whose `ParmsSize` is 16).
- The hook against the game and the `ProcessEvent` call are not exercised.

### 15.2 What CI cannot show

The hook against the real executable, whether a weapon reaches its client, the omnicell and reference-pose fixes, and
the cost on a full hub, and patch 0057: the play-test. `restricted/`'s cargo tests were not run for this page (the RAM
rule; the C++ of 0057 needs the maintainer's SDK to build).

---

## 16. Offset-table lines (R13)

### 16.1 `TARGETS` (`FoundBy::RvaAndBytes`: a `.pdata` entry with these first bytes, checked 2026-11-13)

| Id | RVA | Bytes | What |
| --- | --- | --- | --- |
| RG-2 | `0x01BF_E430` | `40 56 48 83 EC 20 48 8B 02 48 8B F2 0F B6 50 58` | `UArchonReplicationGraph::RouteAddNetworkActorToNodes`; its first reads are the actor from the info and the actor's `+0x58` flag byte |

### 16.2 `pub mod relevance`

| Name | Value | Use |
| --- | --- | --- |
| `ADD_NETWORK_ACTOR` | `0x00F1_2D40` (`UReplicationGraph::AddNetworkActor`, the caller; not hooked, for the record) | 6 |
| `GRID_ADD_ACTOR_DORMANCY` | `0x00F1_2820` (the grid's dormancy-aware add; not hooked, for the record) | 6 |
| actor layout | `ACTOR_FLAGS_58 0x58` (`ONLY_RELEVANT_TO_OWNER 0x04`, `ALWAYS_RELEVANT 0x08`), `ACTOR_FLAGS_59 0x59` (`NET_USE_OWNER_RELEVANCY 0x02`), `ACTOR_OWNER 0xE0`, `ACTOR_NET_DORMANCY 0xF1`, `ACTOR_NET_UPDATE_FREQUENCY 0x108`, `ACTOR_MIN_NET_UPDATE_FREQUENCY 0x10C`, `DORM_AWAKE 1` | 6, 7, 8 |
| game layout | `CHARACTER_WEAPON 0x5F0`, `CHARACTER_SPAN 0x748`, `WEAPON_SPAN 0x300`, `PLAYER_CONTROLLER_PAWN 0x250` | 7, 8, 9 |
| graph layout | `NEW_ACTOR_INFO_SIZE 0x18`, `GRAPH_GLOBAL_ACTOR_MAP 0xC0` (stride `0x18`, span `0x118`), `CONN_MANAGER_ACTOR_INFO_MAP 0x30` (stride `0x20`, manager span `0xE0`), `GLOBAL_INFO_SIZE 0x110`, `GLOBAL_WANTS_DORMANT 0x14`, `GLOBAL_CHANNEL_TIMEOUT 0x30`, `GLOBAL_CULL_DISTANCE 0x88`, `GLOBAL_CULL_DISTANCE_SQ 0x8C`, `CONN_INFO_SIZE 0x30`, `CONN_INFO_CHANNEL 0x00`, `CONN_INFO_CULL_DISTANCE 0x08`, `CONN_INFO_CULL_DISTANCE_SQ 0x0C`, `CONN_INFO_LAST_REP_FRAME 0x14` | 5, 7 |
| map layout | `MAP_DATA 0x00`, `MAP_NUM 0x08`, `MAP_MAX 0x0C`, `MAP_INLINE_BITS 0x10`, `MAP_SECONDARY_BITS 0x20`, `MAP_NUM_BITS 0x28`, `MAP_VALUE 0x08` (in an element) | 5.1 |
| constants | `PLAYER_CHANNEL_GRACE_FRAMES 300`, `WEAPON_WAKE_PERIOD_MS 250`, `WOKEN_RING 16`, `GRACE_PASS_FLOOR_MS 1000`, `GRACE_PASS_MAX_MS 15000` | 5, 7, 9 |
| role layout | `CHARACTER_PLAYER_ROLE 0x738` (`AArchonCharacter::PlayerRole`, server-only) | 9 |

`GRAPH_CONNECTIONS`, `CONN_MANAGER_CONNECTION` and `CONNECTION_PLAYER_CONTROLLER` (`0x30`) are R12's (R12 17.2).
Where R9, R12 or R17 already defines a name for the same fact, use theirs.

---

## 17. The maintainer's play-test, calls and ambiguities

### 17.1 Play-test (surface section 8, R13 row; collected for the one hands-on pass at the end)

"Two-player hunt: each sees the other's weapon from the start and after dormancy." Needs an MIT build with R9, R11,
R12 and R13 landed (13.2), both DLLs injected as in R0 11.1. Have one account carry a hammer (`WP_IH_ROCK`, or a new
account's `WP_IH_BEGINNER`), the other any other weapon.

1. **Hub boot.** In `rustic_isles_dll_port<port>.log`: `install RG-2 ok` (a `prologue mismatch` here means no route
   correction at all: stop and report), and the AGPL's `[MitSeam] ... <n> hooks left to it` with `n` one larger than
   with R12 alone.
2. **Ramsgate.** Both clients join the hub and see each other's character and weapon. Lines: `[MitRelevance] route:
   controller` once per player, `route: player-state` per player, `route: character` per player (grace `4->300` or
   similar).
3. **Hunt, from the start.** Both join one hunt from the airship. Loading finishes for the hammer account in seconds
   (before this fix it never finished). Each sees the other's weapon in hand from the first frame of the hunt. Lines:
   `route: weapon ... cull=1 ... route=global` for the hammer, or `woke=1` for a dormant weapon; possibly
   `weapon: cull cleared` or `weapon: woke`.
4. **After dormancy.** Stay in the hunt for a few minutes, sheathe and draw, walk far from the start (more than
   15 000 units, across the island), swap places. Each still sees the other's weapon. Go back to Ramsgate, wait a few
   minutes, then start a second hunt together: same result.
5. **Omnicell.** On both clients, the omnicell ability and its HUD charge stay put for the whole hunt, also after
   walking far from the island's origin (no coming and going). `route: role` lines once per role. The AGPL's
   `[RelevancePass]` and `[PlayerRoleCullGuard]` lines do not appear (patch 0057 stands its pass down).
6. **Escalation.** A two-player escalation through at least two ready transitions: neither player sees the other in
   the reference pose after a transition.
7. **Hosted hunt (7.4).** Host a realm hunt from the launcher with the hammer account; loading finishes and the
   weapon shows.
8. **Quiet.** No `[MitRelevance] pawn grace: corrected` or `role cull: corrected` lines (one means something reset
   settings after registration: note it); no `[Panic]`.
9. **Cost.** On a hub with both players idle for 5 minutes, compare the server's frame time against the build with
   R12 only: no clear rise.

Only CI passed until these steps are done.

### 17.2 Calls, ambiguities and departures

Calls (for the maintainer to confirm; the ones marked **ruling** need one):

1. **Ruling: three revert switches dropped** (12): `DISABLE_WEAPON_DORMANCY_FIX`, `DISABLE_WEAPON_CULL_FIX`,
   `DISABLE_REPGRAPH_RELEVANCE_FIX`, as R12's ruling did for its guard switches. Recommended: yes.
2. **Ruling: base-actor routing owned and not ported** (10), with its two flags and `ENABLE_PLAYERSTATE_OWNER_ROUTE`:
   superseded by R12's drive. Recommended: yes.
3. **Ruling: `DISABLE_PLAYER_ROLE_GLOBAL_ROUTE` dropped** (12), agreed with R14's writer: neither MIT side reads it.
   Recommended: yes.
4. **The whole relevance pass is R13's** (9), roles found from the pawns; **patch 0057** stands the AGPL pass down on
   `SP-3@R13` (13.1); R14 lands with or after R13 (agreed with R14's writer). `PLAYER_ALWAYS_RELEVANT` moves to R11,
   `DISABLE_OWNER_PAWN_RELEVANCY` to R20 (12).
5. **R13 lands with or after R12** (13.2), enforced by a registry test.
6. The handler at SP-3 after `order = 400` (7.1).
7. The per-tick pawn write kept though it may be a no-op (8).
8. The relevance pass compares its inputs directly instead of hashing them, and checks the pawn's class (9).
9. `is_quarantined` not used (3).
10. Patch 0057 (13.1): built and checked only by the maintainer.

Departures:

1. RG-2's target checked against its first bytes (4.2).
2. New tag `[MitRelevance]` and new texts; budgets on every repeating line; no membership lines after a route (4.3).
3. The weapon wake runs on a player host too (7.4).
4. The relevance pass touches player characters only, and finds roles from the pawns rather than R14's table (9.3).

Ambiguities:

- The per-tick pawn write may change nothing after the default-object boost (8).
- `NumBits` smaller than `ArrayNum` caps the map walk (5.1 step 3), as the AGPL does; with UE 4.26's sparse array they
  are equal in practice.
- The woken ring of 16 can evict a weapon that is then woken again; the "already sent" check of 7.2 step 6 keeps that
  harmless.
- Whether the engine ever resets a global info's settings after registration (the re-assert's reason to exist) was
  never observed.

No open question remains beyond the three rulings (calls 1, 2, 3).

---

## 18. Size

Rust, without tests: the DLL crate about 320 lines (RG-2 and its writes 120, the handler with the wake, the pawn write
and the relevance pass 170, install 30), the core crate about 420 (the map search and toolkit 120, route decisions 90,
wake and pass decisions 110, lines, budgets, flags and offsets 100). Tests about 420. The AGPL patch 0057 is a few
lines. Far smaller than the surface's 2 000: 0014's log, the base-actor routing and the diagnostics are not here.
