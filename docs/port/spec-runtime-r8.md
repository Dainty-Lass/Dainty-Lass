<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R8, server lifecycle (RD27, plan item 11)

R8 decides how long a game server lives and how fast it runs. It paces the server's game thread (patch 0020), keeps the
game mode's expected-player count on the launch roster, publishes the connection count, writes the hub and hunt AFK
timeouts the clients read, ends a disposable server that is empty, stuck or no longer serving, gives the engine back its
32 MB out-of-memory reserve, trims an idle hub's working set, and is the one way a server ends itself (the reap exit,
which R10's fence also uses). It owns the feature ids `SP-3@R8` and, new here, `SP-2@R8` (`spec-runtime-surface.md`
sections 2.6, 2.7, 4.1, 4.2 and 6; `spec-runtime-r0.md` section 7.5). It installs nothing on a client.

An implementer builds from this page, `spec-runtime-surface.md`, `spec-runtime-r0.md`, `spec-runtime-r1.md`,
`spec-runtime-r3.md`, `spec-runtime-r4.md` (section 3.5), `spec-runtime-r9.md` and section 3 of `spec-runtime-r17.md`
alone, and never reads `restricted/`. What crossed is facts: RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7,
CL 647472), byte patterns, UE 4.26 and game layout offsets, engine and game names, flag and variable names, and
behaviour. The AGPL side (seam patch 0041, R9's patch 0052, and runtime patch 0054 written with this page) is described
only by the contract it keeps (section 10).

Conventions are R0's: an RVA is `+0x0XXXXXXX` from the executable's image base; **S** server. Times are
`GetTickCount64` milliseconds unless a line says otherwise. **(call)** marks a decision this page makes,
**(departure)** a deliberate difference from the AGPL runtime, **(ambiguity)** something the source did not settle; all
are collected in section 13.

---

## 1. Scope

| Id | What | Kind (R0 registry) | Section |
| --- | --- | --- | --- |
| `SP-3@R8` | frame pacing (patch 0020) and the frame sample; the listening-block steps (expected-player sync, connection count); the connected-server step (game-mode sample, hub role, AFK write, the tick-side empty reap); the watchdog thread | Feature | 4 to 7 |
| `SP-2@R8` **(new id, call)** | the AFK write at each `K2_PostLogin` | Feature | 6.4 |

Also R8's, with no id of its own: the reap exit and the server quit path (section 8), the out-of-memory reserve release
and the working-set trim (7.6, 7.7), the `[Perf]` and `[Memory]` lines (7.8).

Not R8:

- The map URL and its `?MaxPlayers=` (R9's SRV-1, from R1's `config::hub_cap()`). The "hub cap" of the surface row is
  that value; R8 only logs it (6.1) and clamps the roster with it (through R1's `expected_player_count`).
- The parsers: R1 owns the frame-pacing settings (R1 8), the roster count (R1 7.1), the hub cap and the AFK timeouts
  with their sources, including RT-11's environment variables (R1 10).
- The listen, readiness and the listening driver (R9), the player host's pipes and fence (R10; R10 calls R8's quit,
  8.2), the logged process exits (R3: R8 only *calls* `TerminateProcess`, which R3's EXIT-4 logs).
- Work the AGPL kept in the same tick block that is not lifecycle (section 10.3): the manual `TickStamina` per player
  pawn and the bleedout duration floor go to **R20**, the player pawn's update-frequency boost to **R13** **(call)**.
- The post-login match start, the restart and possession work (R20, R7), the hub AFK exemption on a phantom local
  controller (not ported, 13.3), and the diagnostics around all of this (RT-3, 11.2).

**Dependencies.** R0 (SP-2/SP-3 chains, flags, log, memory, names, registry, `.pdata`), R1 (`config::launch()`,
`config::auth_mode()`, `config::port()`, `config::hub_cap()`, `config::hub_afk()`, `config::hunt_afk()`,
`config::frame_pacing()`, `roster::expected_player_count`), R4 (`login::cmdline::engine_multihome()`), R9
(`listen::state()`, the driver layout of R9 2.1), R10 (`hosted_role_is_hub()`, 9.1), and the reflection kit's lookups of
`spec-runtime-r17.md` section 3.2 (`is_a`), shared as R9 1 says. **R8 lands with or after R9 and R10 (call, kept from
R9 8.1):** with R9 alone, a player host's listen stays AGPL (R9 6.5), R9 publishes no listening state, and an MIT R8
would never start; with R10 landed the decision of R9 6.5 is `Mit` for every server. A registry test enforces it (14.1).

---

## 2. Where it lives

| File | Kind | Holds |
| --- | --- | --- |
| `crates/rustic-isles-runtime-core/src/server_life.rs` (new) | pure, every platform | the pacer (4.2); the hub role rule (5.4); the AFK write plan (6.3); the watchdog's sample rules and its three reap timers (7.3 to 7.5); the trim timer (7.7); the out-of-memory reserve checks over bytes and region facts (7.6); the perf window arithmetic (7.8); every log text of section 12 |
| `crates/rustic-isles-runtime/src/server_life/mod.rs` (new, safe) | Windows | `install()`, the SP-3 before and after handlers, the SP-2 handler, the state the watchdog thread reads (5.6), the thread itself, the quit interface (8.2) |
| `crates/rustic-isles-runtime/src/ffi/server_life.rs` (new, under `ffi`) | Windows, unsafe | the game-memory writes (expected players, AFK floats), the thread start, the UDP probe socket, `VirtualFree` of the reserve, `SetProcessWorkingSetSizeEx`, `K32GetProcessMemoryInfo`, `TerminateProcess` |

Lines R8 adds to R0's files (one each, append-only):

- `registry.rs`: `"R8"` in `LANDED`, and the new line `e("SP-2@R8", Feature, "R8")` in `ENTRIES`. `SP-3@R8` is
  already there.
- `offsets.rs`: the lines of 2.1.
- `lifecycle.rs`: on a server, `server_life::install(...)` after `minhook::initialize()` and **before**
  `handlers::freeze()` (it registers handlers); its result goes into the status like every slice's.
- `ffi/mod.rs`: `pub mod server_life;`. `lib.rs`: `pub mod server_life;` under the same `cfg` as the other hooking
  slices. Core `lib.rs`: `pub mod server_life;`.

R8 hooks nothing: no `TARGETS` line. The DLL may import `ws2_32` (R0 2 allows it) for the probe socket.

### 2.1 Offset table lines (RT-2)

`data` (addresses R8 reads or checks, not hooks):

| Name | Value | What |
| --- | --- | --- |
| `SETUP_MEMORY_POOLS` | `0x0252_5890` | `FGenericPlatformMemory::SetupMemoryPools`, checked only (7.6) |
| `ON_OUT_OF_MEMORY` | `0x0252_9C00` | `FGenericPlatformMemory::OnOutOfMemory`, checked only (7.6) |
| `BACKUP_MEMORY_POOL` | `0x06DA_CBD0` | the global holding the engine's 32 MB out-of-memory reserve |

`layout` (UE 4.26 and the game's classes as built into 1.14.7):

| Name | Value | What |
| --- | --- | --- |
| `WORLD_AUTHORITY_GAME_MODE` | `0x118` | `UWorld::AuthorityGameMode` |
| `WORLD_GAME_STATE` | `0x120` | `UWorld::GameState` (a world is read only when readable for `0x128`) |
| `GAME_MODE_NUM_PLAYERS` | `0x2D0` | `AGameMode::NumPlayers` (`i32`) |
| `GAME_MODE_NUM_TRAVELLING` | `0x2DC` | `AGameMode::NumTravellingPlayers` (`i32`); a game mode is sampled only when readable for `0x2E0` |
| `GAME_MODE_AFK_TIMEOUT` | `0x468` | `AArchonGameMode::AFKTimeout` (`f32`) |
| `GAME_MODE_AFK_TIMEOUT_FRIENDLY` | `0x46C` | `AArchonGameMode::AFKTimeoutFriendly` (`f32`); written only when readable for `0x470` |
| `GAME_MODE_EXPECTED_PLAYERS` | `0x498` | `AArchonGameMode::ExpectedPlayerCount` (`i32`) |
| `GAME_STATE_EXPECTED_PLAYERS` | `0x2E8` | `AArchonGameState::ExpectedPlayerCount` (`i32`, replicated) |
| `GAME_STATE_AFK_TIMEOUT` | `0x3F8` | `AArchonGameState::AFKTimeout` (`f32`, replicated) |
| `GAME_STATE_AFK_TIMEOUT_FRIENDLY` | `0x3FC` | `AArchonGameState::AFKTimeoutFriendly` (`f32`, replicated); written only when readable for `0x400` |
| `CONNECTION_OWNING_ACTOR` | `0x98` | `UNetConnection::OwningActor` (a connection is read only when readable for `0x140`) |

`NET_DRIVER_CLIENT_CONNECTIONS` (`0x90`, `Num` at `+0x98`, `Max` at `+0x9C`) and `CONNECTION_STATE` (`0x134`, `3` =
open) are R9's lines (R9 2.1). `GWORLD` is R0's.

Class names R8 tests with the kit's `is_a`: `ArchonGameMode`, `ArchonGameState` (the game's `AArchonGameMode` and
`AArchonGameState`; `BPGM_City_C`, `BPGM_ArchonTrainingGrounds_C` and the hunt game modes derive from the first).

---

## 3. Registry, ownership and the AGPL in one table

With `"R8"` in `LANDED`, `rir_owns` answers 1 for `SP-3@R8` and `SP-2@R8`; the status line's `owns=` count grows by 2.

| Work | MIT DLL absent, or R8 not landed | R8 landed |
| --- | --- | --- |
| Frame pacing | AGPL | MIT (4) |
| Expected-player sync, connection count | AGPL (gated on `SP-3@R8` by 0052) | MIT (5.2, 5.3) |
| Game-mode sample, hub role, AFK write, tick-side reap | AGPL | MIT (5.4 to 5.6, 6) |
| AFK write at `K2_PostLogin` | AGPL | MIT (6.4); the AGPL's is gated on `SP-2@R8` (0054) |
| Watchdog thread (reaps, reserve release, trim, `[Perf]`/`[Memory]`) | AGPL | MIT (7); the AGPL starts none (0054) |
| Manual `TickStamina`, bleedout floor | AGPL | AGPL until R20 (0054) |
| Player pawn update-frequency boost | AGPL | AGPL until R13 (0054) |
| Server quit (the fence's exit) | AGPL, called by the AGPL's R10 | whichever DLL owns R10 calls its own (8.2) |

---

## 4. Frame pacing (`SP-3@R8`, patch 0020)

### 4.1 Why

Every game server is a whole Dauntless process, and nothing in this build caps its game thread: each frame runs as fast
as the CPU allows, which keeps a core busy per server even with nobody connected (and more under Wine). The cap sleeps
real time between frames, so `DeltaTime` stays the real frame time: game time is unchanged, only the number of frames
per second drops. 30 is the configured `NetServerMaxTickRate`, so replication keeps its rate.

### 4.2 The before handler (`slice = "R8"`, `order = -2000`) **(call: order)**

An SP-3 **before** handler, first of the MIT before handlers (R5's is `-1000`, R11's `-500`), so the sleep happens
before every other piece of the frame, the AGPL's tick wrapper included: the AGPL paced at the very top of its tick.
Each call, on the game thread:

1. If detaching: return.
2. **Frame sample** (7.8): read the performance counter and record the interval since the previous call.
3. **First call only:** `(active, idle) = config::frame_pacing()`; log `[MitRuntime] ` followed by R1's
   `frame_pace_line(active, idle)` (R1 8). `active` 0 = no cap; `idle` 0 = the same as `active`.
4. **Idle?** (pure `Pacer::step`) The server is *idle* when `idle > 0`, `listen::state()` is `Some`, and the connection
   count R8 last published (5.3) is exactly 0 (the initial "unknown", -1, is not 0). The first idle frame starts a
   period; any non-idle frame clears it. `idle_now` = a period exists and is at least **15 000 ms** old.
5. When `idle_now` differs from the previous frame's: log `[MitRuntime] [FramePace] no client for 15 s: <idle> fps`, or
   `[MitRuntime] [FramePace] client connected: <active> fps` (`no cap` when `active` is 0). The first frame counts as
   "not idle" and logs nothing.
6. `fps = idle_now ? idle : active`. When `fps > 0`: `target = previous + 1 000 000 / fps` microseconds (integer
   division); when `target` is later than now, sleep until `target`. Then `previous = now` (read after the sleep), on
   every frame, capped or not. `previous` starts at the first call's time, so the first frame does not wait.

Clock: a monotonic microsecond clock (`std::time::Instant`). The sleep is `std::thread::sleep(target - now)`.
**(ambiguity)** The sleep's resolution is the OS timer's; a frame can overshoot by up to a millisecond or two, as the
AGPL's did. The pure pacer takes the times as numbers and answers "sleep `n` µs".

So pacing applies from the server's **first frame**: boot and the map load run at the active cap, the idle cap only
ever after listening. (The surface's "boot, map load and the first 15 s after listening are uncapped" means "not at the
idle cap"; the active cap applies throughout.) A client's first connection returns to the active cap on the frame
after R8 publishes the count.

---

## 5. The tick steps after the engine tick (`SP-3@R8`)

### 5.1 The after handler (`slice = "R8"`, `order = 600`) **(call: one handler, order)**

One SP-3 **after** handler below R9's maintenance (700, R9 5.1), as R9 asked. It runs only on a server and only while
`listen::state()` is `Some`; it reads `GWorld` through R0's checked read and does nothing in a frame without one. The
AGPL ran the sync and the count inside its listening block and the connected-server step after its listen step, so a
listen's own frame did none of it; R8 sees `state()` first on the frame after R9's listen (900), which keeps that.
Each call, in this order:

1. **Start, once** (the first call that sees `state()`): reset the perf counters (7.8), prime the AFK settings (6.1),
   start the watchdog thread (7.1).
2. The expected-player sync (5.2).
3. The connection count (5.3).
4. When `launch.watchdog` is true: the connected-server step (5.4 to 5.6, and the AFK tick write 6.2).

`launch.watchdog` is R1's: true for a full launch (more than 8 arguments: the Director's and the player host's), false
in manual test mode.

### 5.2 Expected-player sync

`ExpectedPlayerCount` must equal the launch roster's size, written every frame: a two-player hunt left at the native
default entered `InProgress` as soon as the first player arrived, and a second joiner arriving seconds later then hit
the fragile late-join path; `InitGame` or replication can restore the native value, hence every frame. Hubs launch with
`NO_EXPECTED_PLAYERS` and get 0.

- `value` = `roster::expected_player_count(launch.expected_players, config::hub_cap().value)` (R1 7.1), computed once.
- **First call:** log `[MitRuntime] [ExpectedPlayers] roster sync <on|off (DISABLE_EXPECTED_PLAYER_ZERO.flag)>
  launchRoster=<value>`. `DISABLE_EXPECTED_PLAYER_ZERO.flag` is read once here; present, R8 never writes (an A/B switch
  that leaves the native session state alone).
- Each frame, for the world's game mode (`WORLD_AUTHORITY_GAME_MODE`) with `GAME_MODE_EXPECTED_PLAYERS`, then the
  world's game state (`WORLD_GAME_STATE`) with `GAME_STATE_EXPECTED_PLAYERS`: write `value` only if the object passes
  the **live-instance test** (below) for `ArchonGameMode` (resp. `ArchonGameState`), the field is readable and writable
  for 4 bytes, and its current value differs. The first write to each logs once
  `[MitRuntime] [ExpectedPlayers] <GameMode|GameState> 0x<p> class=<class short name> <before> -> <value>`.

**Live-instance test** (used throughout): non-null; R0's `is_live_object`; not a default object (flag `0x10` clear);
its class pointer (`+0x10`) is itself a live object; and `is_a(object, name)`.

### 5.3 Connection count

The driver is `listen::state().driver`. If `driver + 0x98` is readable for 8 bytes (R0's frame cache may answer), read
`Num` (`i32`, `+0x98`). When `Num >= 0`: publish `conn_count = Num` and `conn_stamp = now`; when `Num > 0` also
`had_client = true` and `last_client_ms = now`. Otherwise publish nothing this frame.

This is the raw `ClientConnections.Num`, null entries included, as the AGPL counted. It is the emptiness signal: a
1.14.7 server has no local player since HL-2 (R5), and the connection count is what drops to 0 when the last player
leaves.

### 5.4 Game-mode sample and the hub role

- **The game mode.** The world's `WORLD_AUTHORITY_GAME_MODE` when it passes the live-instance test for `ArchonGameMode`
  and is readable for `0x2E0`. Otherwise the last one R8 sampled while it is still a live object readable for `0x2E0`
  (a hub that leaves its map keeps its game mode; the counts then stay readable). Otherwise none: this frame's sample is
  `unknown`. **(departure)** The AGPL found the game mode by scanning the object array (every 3 s until found) and kept
  the first `ArchonGameMode` instance; R8 takes the world's own, the only one the engine consults.
- **The hub role, classified once** from the first game mode sampled (pure `hub_role(class_name, mode,
  hosted_role_is_hub, keep_flag)`), `class_name` = the game mode's class short name:
  - `named` = `class_name` contains `City` or `TrainingGrounds` (substrings, case-sensitive: `BPGM_City_C`,
    `BPGM_ArchonTrainingGrounds_C`).
  - `GlobalGameserver`: `Hub` when `named`, else `Disposable`.
  - `PlayerHost`, not `named`: `Disposable`; when R10's `hosted_role_is_hub()` is true, log once `[MitRuntime] [HubRole]
    WARNING: <class> is disposable although the launcher role is HUB: not a named hub class (City / TrainingGrounds), so
    the empty reap and the hunt AFK rule apply`.
  - `PlayerHost`, `named`: `Hub` when `hosted_role_is_hub()` or `KEEP_HOSTED_HUB_ROLE.flag` (read once), else
    `Disposable`; log once `[MitRuntime] [HubRole] <class> -> <persistent hub (launcher role)|persistent hub
    (KEEP_HOSTED_HUB_ROLE.flag, launcher role HUNT)|disposable (launcher role HUNT): reaped when empty, hunt AFK rule>`.

  A `Hub` is never reaped by the empty or stale-tick rules (7.3, 7.4): the Director (or the launcher's supervisor for a
  realm hub) owns its life. A player-hosted Training started as a hunt is disposable, so it ends when empty instead of
  holding idle players for 20 minutes. Until classified the role is `Unknown`, which never reaps and never writes AFK.
- **Published** (5.6): `num_players` (`GAME_MODE_NUM_PLAYERS`), `num_travelling` (`GAME_MODE_NUM_TRAVELLING`), their
  stamp, `ever_had_counts = true`, `last_tick_ms = now`, the role; `saw_player = true` once `num_players > 0`.
- The step's *empty state*: `Empty` when `num_players <= 0` and `num_travelling <= 0`, `Occupied` otherwise, `Unknown`
  without a game mode.

### 5.5 The tick-side empty reap

The game thread's own reap, beside the watchdog thread's (7.3), with the same limit:

- `has_connection`: any client connection (R9's `client_connections(driver)`; each readable for `0x140` through the
  frame cache) whose `CONNECTION_OWNING_ACTOR` is non-null and whose `CONNECTION_STATE` is `3`.
- `empty` = state `Empty`, or state `Unknown` and not `has_connection`.
- `empty`: `empty_seconds += DeltaTime` (the handler's delta); otherwise `empty_seconds = 0`.
- `empty_seconds >= 60` and the role is `Disposable`: the reap exit (8.1) with reason
  `tick: disposable instance empty 60 s (game mode <empty|unknown>)`.

**(ambiguity)** The surface says 50 s; 1.14.7's runtime reaps at **60 s** on both paths (the 50 s figure is older), so
R8 takes 60 s.

### 5.6 What the watchdog thread reads

Plain atomics written on the game thread, read by the thread; the thread never reads game memory:

| State | Initial | Written by |
| --- | --- | --- |
| `conn_count` (`i32`), `conn_stamp` | -1, 0 | 5.3 |
| `had_client`, `last_client_ms` | false, 0 | 5.3 |
| `num_players`, `num_travelling` (`i32`), `counts_stamp` | -1, -1, 0 | 5.4 |
| `ever_had_counts`, `saw_player` | false | 5.4 |
| `last_tick_ms` | 0 | 5.4 (the heartbeat) |
| role (`Unknown`/`Hub`/`Disposable`) | `Unknown` | 5.4 |
| the frame counters | 0 | 4.2, 7.8 |

Relaxed ordering suffices (each value stands alone; stamps decide freshness).

---

## 6. AFK timeouts

### 6.1 The settings (primed once, game thread)

At the first of: the start step (5.1), or the first write (6.2, 6.4).

- `DISABLE_AFK_TIMEOUT_OVERRIDE.flag` (read once): present, R8 never writes and logs `[MitRuntime] [AfkTimeout] override
  off (DISABLE_AFK_TIMEOUT_OVERRIDE.flag): every role keeps the game's own timeout`.
- Otherwise `hub = config::hub_afk()`, `hunt = config::hunt_afk()` (R1 10: the launcher's override for a player host,
  then RT-11's `DAINTY_HUB_AFK_SECONDS` / `DAINTY_HUNT_AFK_SECONDS`, then the number files, then hub 1200 s and hunt
  native). Log `[MitRuntime] [AfkTimeout] config hub=<text> hunt=<text> maxPlayers=<cap> (<cap source>)`, a text being
  `native (<source>)` or `<n> s (<source>)` with R1's source texts and `config::hub_cap()` **(call: the cap's line moves
  here and is logged for every server; the AGPL logged it only on a player host)**.
- Warnings: hub `Some(0)`: `[MitRuntime] [AfkTimeout] WARNING: the hub timeout is 0 (<source>): hub players are never
  idle-kicked and can hold every hub slot`; hunt `Some(0)`: `... WARNING: the hunt timeout is 0 (<source>): hunt
  players are never idle-kicked`.

### 6.2 Why the game state, and the tick write

The client kicks itself: its own idle check runs only for local controllers and fires when `0 < T < idle seconds` and
the controller's `bNoAFK` is clear, where `T` is the game state's `AFKTimeoutFriendly` or `AFKTimeout`. Both game-state
floats replicate **initial-only** (`COND_InitialOnly`), so the value must be in place before a client's game-state
channel opens; a later change reaches only clients that join after it. 0 or less never kicks. The game mode keeps its
own copy, which `InitGameState` reads once; R8 writes both so they agree. These are the same writes as the game's own
`AFKTimeout` cheat; no hook, no byte patch.

Tick write: in the connected-server step (5.1 step 4), once a second (the first call at once): `apply("tick", none)`.

### 6.3 `apply(why, hint)` (game thread)

1. Primed, and not disabled (6.1).
2. The world, readable for `0x128`. `gs` = `WORLD_GAME_STATE`: the live-instance test for `ArchonGameState` and readable
   for `0x400`, else return. `gm` = `WORLD_AUTHORITY_GAME_MODE` when it passes the test for `ArchonGameMode` and is
   readable for `0x470`, else none.
3. The role: the classified one (5.4); while `Unknown`, classify now from `gm`'s class, or from `hint`'s when `gm` is
   none and `hint` passes the test for `ArchonGameMode` (`hint` only classifies; it is never written). Still `Unknown`:
   return, writing nothing.
4. `setting = role == Hub ? hub : hunt`.
5. `setting.seconds` is `None`: write nothing (`no override (native)`). `Some(s)`: `v = s as f32`; when either
   game-state float differs from `v`, write both; when `gm` exists and either of its floats differs, write both.
6. Once per game-state instance (a new game state logs again): `[MitRuntime] [AfkTimeout] role=<hub|hunt>
   class=<game mode class or ?> before GM=<a>/<b or ?> GS=<a>/<b> -> <applied <s> s[ (never kick)]|no override (native)>
   (why=<tick|postlogin>) GS=0x<p>`, floats printed with no decimals.

The pure `afk_plan(setting, gs_now, gm_now) -> writes` is CI-tested.

### 6.4 The post-login write (`SP-2@R8`)

`K2_PostLogin` runs on the server for each joining player, before that connection's first game-state bunch; the tick
write may not have run yet (a game state the server just made, a login within the second). So:

- One SP-2 **before** handler, `slice = "R8"`, `order = 0`, matcher `FunctionName("K2_PostLogin")` (the part after the
  last `.` equals `K2_PostLogin`; R0 8.3). **(departure, RT-18)** The AGPL tested a substring `K2_PostLogin` of the full
  name; the function-name match catches the same calls on 1.14.7 and nothing that merely contains the text.
- Server only. It calls `apply("postlogin", call.object)` (the object receiving `K2_PostLogin` is the game mode) and
  returns `Continue`. It never absorbs. As a MIT before handler it runs before every AGPL block of the call, the AGPL's
  post-login match start included; the order between the two does not matter (the write targets the world's game state,
  the match start the game mode's match state).

---

## 7. The watchdog thread

### 7.1 Start

Started once, by the start step (5.1), with `std::thread::Builder` named `rir-watchdog` **(call: R0 3.1 forbids a thread
from `DllMain`; this one starts from the game thread at listen, as the AGPL's did)**. A failed spawn logs
`[MitRuntime] [Watchdog] could not start: <error>; no empty-server reap in this process` and R8 carries on without it.
At start it reads once: `KEEP_OOM_POOL.flag`, `DISABLE_PERF_LOG.flag`, `KEEP_WORKING_SET.flag`; `trim_allowed` =
`auth_mode() == GlobalGameserver` and not `KEEP_WORKING_SET`. It logs:

```text
[MitRuntime] [Watchdog] started (poll 2 s; reaps a disposable server with no client for 60 s after its last client left, or 300 s if none ever arrived)
[MitRuntime] [Memory] reserve: <released at listen + 30 s|kept (KEEP_OOM_POOL.flag)>; idle hub trim: <on (hubs only, after 60 s with no client, once per idle period)|off (KEEP_WORKING_SET.flag)|off (not a Director server)>; [Perf]/[Memory] lines: <every 60 s|off (DISABLE_PERF_LOG.flag)>
```

### 7.2 Each poll

Every 2000 ms (`Sleep`), `iter += 1`, in this order:

1. Detaching: the thread returns **(departure: the AGPL kept looping without acting; ending is the same)**.
2. **Reserve**: once, when 30 000 ms have passed since the start and `KEEP_OOM_POOL` is absent: release it (7.6).
3. **Perf**: when the lines are on and `iter % 30 == 0`: the `[Perf]` and `[Memory]` lines (7.8).
4. When `launch.watchdog` is false: `empty_seconds = 0` and the poll ends here (the manual test mode never reaps).
5. **Sample** (pure `Sample::from`): `counts_usable` = `counts_stamp` non-zero and at most **10 000 ms** old, and both
   `num_players` and `num_travelling` in `0..=64`. `conn_usable` = `conn_stamp` non-zero and at most 10 000 ms old and
   `conn_count` in `0..=64`. `empty` = `counts_usable` and `conn_usable` and `conn_count <= 0` and `num_travelling <=
   0`. A stale or implausible sample is unknown and never reaps. Counts out of range with a non-zero stamp log
   `[MitRuntime] [Watchdog] ignoring implausible counts (players=<p> travelling=<t>, at most 64): no reap`, at most 4
   times per process.
6. **Heartbeat line**, when `iter % 15 == 0`, or `empty` and the role is `Disposable` and `iter % 5 == 0`:
   `[MitRuntime] [Watchdog] role=<unknown|hub|disposable> players=<p> travelling=<t> connections=<c> hadClient=<0|1>
   countsUsable=<0|1> empty=<0|1> emptySec=<s>/<limit>` (unusable values print -1).
7. **Idle hub trim** (7.7).
8. **Port no longer served** (7.5).
9. **Empty reap** (7.3).
10. **Stale tick** (7.4).

### 7.3 Empty reap

`limit` = **60 s** when `had_client`, else **300 s** (a server spawned for players who never arrived; a slow first load
is never cut short). When `empty` and the role is `Disposable`: `empty_seconds += 2`; at `empty_seconds >= limit`, the
reap exit with reason `disposable server empty 60 s after its last client left` or `disposable server never joined by a
client within 300 s`. Any other poll (occupied, unknown, a hub, not yet classified) sets `empty_seconds = 0`: only
continuous emptiness counts, so a long hunt with brief empty moments is never cut.

Why a thread and not only the tick: once the last player leaves a hunt, its frames can stop or slow so far that the
tick-side timer (5.5) never reaches the limit; an empty hunt then held a Director slot until new hunts were refused.

### 7.4 Stale tick

When the role is `Disposable`, `saw_player` is true and `last_tick_ms` is non-zero, with `since = now − last_tick_ms`:

- `since >= 180 000` (3 min, longer than any legitimate hitch or load): log `[MitRuntime] [Watchdog] disposable server's
  game thread silent for <since/1000> s (players=<p>): a hung game thread or a ghost connection` and the reap exit with
  reason `tick stale`.
- else `since >= 60 000` and `iter % 15 == 0`: log `[MitRuntime] [Watchdog] game thread silent for <since/1000> s
  (players=<p>); reap at 180 s`.

Why: a player who leaves without a clean close leaves a connection the engine only times out from its own net tick,
which is what stops once players are gone; the game mode then still counts one player and the empty rule never fires. A
disposable server whose game thread has not reached R8's step for minutes is finished.

**(departure, RT-3)** The AGPL also suspended the game thread at 20 s and 180 s to log its instruction pointer and an
unwound call stack (a hang diagnostic). R8 does not port that sampling.

### 7.5 Port no longer served

A hub whose map ends (its own game mode's idle-out sends it to `LeavingMap`, and the re-listen fails) keeps running
without its UDP socket; the Director only checks that the process exists, so nothing restarts it. Exiting reports it.

- Only once `ever_had_counts` is true (never a server still loading); every role.
- **Probe** (pure plan, Windows socket): the address is R4's `engine_multihome()`: `None` → `0.0.0.0`; a dotted quad →
  that address; anything else (IPv6, a name) → never act, logged once `[MitRuntime] [Watchdog] port probe off: MULTIHOME
  <value> is not an IPv4 address`. Port `config::port()`; 0 → never act. Create a UDP socket (`AF_INET`, `SOCK_DGRAM`,
  `IPPROTO_UDP`), `bind` it to address:port with no options, close it. A successful bind means the port is **free**; a
  failed bind (the server holds it) or a socket that cannot be made means "not free".
  Why the same address: Windows lets a wildcard bind succeed while another socket holds a specific address on the same
  port, so a loopback-bound player host would look free to a `0.0.0.0` probe.
- `free_since` starts at the first free poll and clears on any not-free poll. Free for more than **45 000 ms**: log
  `[MitRuntime] [Watchdog] UDP port <n> has been free for 45 s after this server had served: it is alive but not
  listening; exiting so its supervisor restarts it` and the reap exit with reason `port no longer served`.
- **(call)** R8 calls `WSAStartup(2.2)` once before the first probe (the engine has Winsock up already; the call only
  adds a reference). A failure turns the probe off for the process, logged once.

### 7.6 The engine's out-of-memory reserve

`FGenericPlatformMemory::SetupMemoryPools` commits a 32 MB block at start and stores it in a global; its only reader,
`OnOutOfMemory`, skips a null pool and otherwise frees it before its fatal log. The pages are never touched, so
releasing it saves 32 MB of commit charge (pagefile budget) per server, not resident memory. The cost: on a real
out-of-memory death that last headroom is gone. `KEEP_OOM_POOL.flag` keeps it.

Checks, each failure logged as `[MitRuntime] [Memory] reserve left alone: <reason>` and nothing freed:

1. `function_entry(0x02525890) == 0x02525890` and its first 21 bytes are
   `48 83 EC 28 B9 00 00 00 02 E8 F2 C4 13 00 48 89 05 2B 73 88 04` (reason `SetupMemoryPools not as verified`).
2. `function_entry(0x02529C00) == 0x02529C00`; at `+0x57` the 7 bytes `48 8B 0D 72 2F 88 04` and at `+0x68` the 5 bytes
   `48 85 C9 0F 84` (reason `OnOutOfMemory not as verified`). These prove the global is `0x06DACBD0`, the size
   `0x2000000`, and that a null pool is skipped.
3. The global at `+0x06DACBD0` readable for 8 bytes (reason `global unreadable`).
4. Its value `block`: non-null, and `VirtualQuery(block)` gives `BaseAddress == AllocationBase == block`, `RegionSize ==
   0x2000000`, `MEM_COMMIT`, `MEM_PRIVATE`, `PAGE_READWRITE` (reason `block is not the expected 32 MB private read-write
   allocation`).
5. Compare-exchange the global from `block` to null (reason `the global changed`).
6. `VirtualFree(block, 0, MEM_RELEASE)`; on failure put `block` back with a compare-exchange from null and log
   `[MitRuntime] [Memory] reserve kept: VirtualFree failed (error <n>)`.
7. Log `[MitRuntime] [Memory] released the engine's 32 MB out-of-memory reserve: private commit <a> -> <b> MiB, working
   set <c> -> <d> MiB` (from `K32GetProcessMemoryInfo` before and after; `?` when unavailable).

The byte and region checks are pure (`reserve_sites_ok(bytes)`, `reserve_block_ok(region)`).

### 7.7 Idle hub trim

Only a classified `Hub` on a Director server (`trim_allowed`), only after **no client for 60 s straight**, once per idle
period. Each poll:

- If trimmed and `last_client_ms > trimmed_at`: re-arm (a client connected since).
- `idle_now` = role `Hub`, `counts_usable`, `conn_stamp` at most **1000 ms** old, `conn_count == 0`, `num_travelling ==
  0`.
- Not `idle_now`: `idle_since = 0`. Else when `idle_since == 0` or `last_client_ms >= idle_since`: `idle_since = now`
  (only continuous zero counts). When `trim_allowed`, not trimmed, and `now − idle_since >= 60 000`: trim, set
  `trimmed_at = now`.
- Trim: `SetProcessWorkingSetSizeEx(GetCurrentProcess(), usize::MAX, usize::MAX, 0)` (the "empty the working set" form),
  timed; log `[MitRuntime] [WorkingSet] trimmed (hub idle, no client for <s> s): working set <a> -> <b> MiB, private
  commit <c> MiB, <ms> ms[ (failed: error <n>)]`.

The trim takes the process's working-set lock, so the game thread may wait on it for the trim's length; that is accepted
only because no client has been connected for 60 s. An arrival during it gets a short hitch on its loading screen.

### 7.8 `[Perf]` and `[Memory]` (call)

RT-3 leaves logging-only probes unported, but these two lines are how a host (and R8's play-test) sees the frame cap
and a server's memory, so R8 keeps them in a reduced form. `[PerfDetail]`, `[PerfNet]` and `[PerfThreads]` are not
ported, so CL-10 stays R19's (R4 section 2).

- **Frame sample** (4.2 step 2): the interval since the previous before-call, in performance-counter ticks: add to
  `frames`, `ticks_sum`, keep `ticks_max`, and count a *hitch* when the interval is over 100 ms. The first sample after
  a reset only sets the start. Reset at the start step (5.1), so boot and map-load frames never reach the first window.
- **Every 60 s** (7.2 step 3), drained with exchange-to-zero:

  ```text
  [MitRuntime] [Perf] <secs>s: <frames> frames (<rate>/s), frame avg <avg> ms / max <max> ms, hitches>100ms <n>, connections <c>[ (stale)], role <role>[ (first window since listen)]
  [MitRuntime] [Memory] connections <c>[ (stale)], working set <ws> MiB (peak <peak>), private commit <pc> MiB, page faults <n total|+n since last line>[ (first window since listen)]
  ```

  `secs` = the summed intervals (1 decimal), `rate` 1 decimal, `avg` 2 decimals, `max` 1 decimal, MiB 1 decimal;
  `(stale)` when `conn_stamp` is older than 10 000 ms. With no frames measured the first line is `[MitRuntime] [Perf] no
  frames measured in this window (game thread not ticking?)`.

---

## 8. Ending a server

### 8.1 The reap exit

`reap_exit(reason)`: log `[MitRuntime] [Reap] <reason> -> TerminateProcess(0)`, then
`TerminateProcess(GetCurrentProcess(), 0)`; should that return, `ExitProcess(0)`. Callable from the game thread or the
watchdog thread; safe to call twice (the second never runs).

Why not a normal exit: the engine's static teardown on this build hangs for 60 s and then aborts (the Director saw
`0xC0000409`), and the engine's own crash reporter faults during it. A game server keeps nothing in process (persistence
is in the backend), so ending at once frees the port and the slot a minute sooner and stops producing alarming exit
codes. With R3 landed the call is logged by EXIT-4's `[MitExit]` line (R3 4.3); R8 adds nothing for that.
**(departure)** The AGPL slept 50 ms before terminating so its buffered log could flush; R0's writer writes each line
with one `WriteFile`, so R8 does not wait.

### 8.2 The server quit (for R10)

`server_life::server_quit(reason)`: log `[MitRuntime] [ServerQuit] quit requested (<reason>): ending through the reap
exit`, then `reap_exit("server quit (<reason>)")`. A server asked to quit (R10's fence) ends this way: the engine's own
quit asks for a player controller to run "quit" through, finds none on a server without a local player and returns at
once, and when it does find one (a remote player's) it reaches the hanging teardown of 8.1. So the AGPL's player host
always ended in its 5 s `0xE105` fallback before it learnt this. R10 calls `server_quit` from its fence; R10 keeps its
own `0xE105` fallback for the case where even this does not end the process.

**(call)** `SERVER_QUITGAME_EXIT.flag` (the revert to the engine's quit on a server) is dropped: that path cannot work
on a server without a local player (section 11). The client's quit is not R8's.

---

## 9. Interfaces

### 9.1 What R8 expects from other slices

| Slice | Needs |
| --- | --- |
| R1 | `config::launch()` (`watchdog`, `expected_players`), `config::auth_mode()`, `config::port()`, `config::hub_cap()`, `config::hub_afk()`, `config::hunt_afk()`, `config::frame_pacing()`, `roster::expected_player_count` (all on `plan`) |
| R4 | `login::cmdline::engine_multihome() -> Option<&'static str>` (R4 3.5) |
| R9 | `listen::state() -> Option<Listening>` (`driver`, `port`, `since_ms`), `listen::client_connections(driver)` (R9 8.1, 8.3) |
| R10 | `hosted_role_is_hub() -> bool`: true only for a player host whose launcher set `RUSTIC_ISLES_HOSTED_ROLE=HUB` (validated by R10); false on a Director server and for role `HUNT` or no role. Shape, not name; registered or exported by R10 as R10's spec says. R10 calls `server_quit` (8.2) |
| R17 kit | `is_a` (R17 3.2), R0's `is_live_object`, `function_entry`, checked reads and writes |
| R3 | nothing called; its EXIT-4 logs R8's `TerminateProcess` |

### 9.2 What R8 publishes

In `rustic_isles_runtime::server_life`:

- `server_quit(reason: &str) -> !` (8.2), and `reap_exit(reason: &str) -> !` (8.1).
- `hub_role() -> HubRole` (`Unknown`, `Hub`, `Disposable`), for a later slice that treats hubs differently (R20).
- `connection_count() -> Option<i32>`: the last published count while fresh (10 s), for R20 or diagnostics.

---

## 10. The AGPL side

### 10.1 What seam patch 0041 and R9's patch 0052 already do

- 0041 gates the AGPL's frame pacing on `SP-3@R8`, and its connected-server block (the client-connection scan, the
  watchdog's game-mode sample and tick-side reap, the AFK tick write, and other work inside it) on `SP-3@R8` as a whole.
- 0052 gates the AGPL's expected-player sync and the watchdog's connection count on `SP-3@R8`, and starts the AGPL's
  watchdog thread when it adopts the MIT's driver only when `SP-3@R8` is not owned.

### 10.2 Why patch 0054

Without it, an MIT R8 would leave three things wrong:

1. The AGPL's post-login AFK write still runs (it sits in a block of `SP-2@R20`), from its own settings. The AGPL reads
   only the number files, not RT-11's variables, so a host with `[director] hub_afk_seconds` set would see the two DLLs
   write different values, and a client would get whichever came last before its channel opened.
2. The AGPL's own listen paths (a player host without R10) still start its watchdog thread beside the MIT's.
3. The AGPL's connected-server block also drives work that is not lifecycle: the manual `TickStamina` call on every
   connected player's `BP_PlayerCharacter_C` pawn, the bleedout duration floor on the island game mode
   (`RUSTIC_ISLES_BLEEDOUT_SECONDS`), and the player pawn's update-frequency boost (R13's switches). 0041's whole-block
   gate would turn them off with R8: no stamina for remote players.

### 10.3 Runtime patch 0054 (written with this page)

`restricted/modules/rustic-isles/native/runtime/dllmain.cpp`, marked `[DaintyLass 0054]`, row 0054 in
`restricted/docs/paradox-patches.md`. Contract:

- **The connected-server block is gated per part, not as a whole:** the block itself runs whenever this runtime is
  listening with a driver (its own or the MIT's, 0052); inside it the watchdog's game-mode sample, AFK tick write and
  tick-side reap run only when `SP-3@R8` is not owned; the manual `TickStamina` call only when `SP-3@R20` is not owned
  (with its existing tick-filter switch); the pawn update-frequency boost only when `SP-3@R13` is not owned; the
  diagnostics (edge-recovery probe, tick-filter census, bleedout watch) not gated.
- **The bleedout floor without the watchdog:** when `SP-3@R8` is owned and the launch is a full one, the block takes the
  world's game mode itself and applies the floor when `SP-3@R20` is not owned (when R8 is not owned it does so inside
  its watchdog sample, as before).
- **The post-login AFK write** runs only when `SP-2@R8` is not owned. The rest of that post-login block (the match
  start, the login stamp, the capture reset) stays on `SP-2@R20`.
- **No AGPL watchdog thread** when `SP-3@R8` is owned, whichever listen path asks for one.
- No seam ABI change: seam version stays 1. **With the MIT DLL absent**, or R8 not landed, the AGPL behaves exactly as
  before.

So `SP-3@R8` now covers pacing, the expected-player sync, the connection count, the watchdog (both sides) and the AFK
tick write; `SP-2@R8` the post-login AFK write. **(call)** This refines R0 7.5's descriptions of `SP-3@R8` and
`SP-2@R20` (the AFK write moves to `SP-2@R8`).

### 10.4 Cross-slice state (R0 7.5's table: rows added)

| State | Set by | Read by | Rule |
| --- | --- | --- | --- |
| the listening driver and its connection count | `SP-3@R9` | R8's pacing, sync, count and watchdog | **R8 lands with or after R9 and R10** (1, 14.1) |
| the AFK values on the game state | `SP-3@R8` and `SP-2@R8` | the client | carried: patch 0054 moves the post-login write with R8, so one DLL writes them |
| the manual stamina tick, the bleedout floor | the AGPL's connected-server block | the game | carried: patch 0054 gates them on `SP-3@R20`; R20's spec ports them |
| the pawn update-frequency boost | the same block | the replication graph | carried: patch 0054 gates it on `SP-3@R13`; R13's spec ports it |
| the player host's launch values | R10's environment | R8's cap and AFK (through R1) | R1 reads them in `PlayerHost` mode itself; R8 needs R10 only for the hosted role (9.1) |
| the hub role | `SP-3@R8` | R20 (later) | published (9.2) |

The watchdog thread's loader-state dump at listen + 25 s (an AGPL diagnostic of R7's loading work) stops when R8 owns
the watchdog, with the AGPL thread. It is logging-only (RT-3).

---

## 11. Flag files (RT-14 for R8) and environment (RT-11)

| Flag | Kept? | Meaning in the MIT runtime |
| --- | --- | --- |
| `DISABLE_AFK_TIMEOUT_OVERRIDE` | kept | read once (6.1): R8 writes no AFK value |
| `DISABLE_EXPECTED_PLAYER_ZERO` | kept | read once (5.2): no expected-player sync |
| `LEGACY_EXPECTED_PLAYER_WALK` | **dropped (call)** | it restored an every-frame scan of the object array for the same write; R8 writes the world's own objects |
| `KEEP_OOM_POOL` | kept | read at the watchdog's start (7.1): the reserve stays |
| `KEEP_WORKING_SET` | kept | read at the watchdog's start: no idle hub trim |
| `KEEP_HOSTED_HUB_ROLE` | kept | read once (5.4): a player-hosted named hub with launcher role `HUNT` stays a hub |
| `DISABLE_PERF_LOG` | kept | read at the watchdog's start: no `[Perf]`/`[Memory]` lines |
| `SERVER_QUITGAME_EXIT` | **dropped (call)** | the engine's quit cannot end a server without a local player (8.2) |

**Environment.** R8 reads none itself; R1 reads everything (R1 8, 10). The variables that reach R8's behaviour:
`DAINTY_SERVER_MAX_FPS`, `DAINTY_SERVER_IDLE_FPS` (patch 0020, 0040), and RT-11's `DAINTY_HUB_MAX_PLAYERS`,
`DAINTY_HUB_AFK_SECONDS`, `DAINTY_HUNT_AFK_SECONDS`, plus the player host's `RUSTIC_ISLES_MAX_PLAYERS`,
`RUSTIC_ISLES_HUB_AFK_SECONDS` and `RUSTIC_ISLES_HOSTED_ROLE` (R10's).

**RT-11's host-kit side is already on `plan`:** `settings.toml`'s `[director]` `hub_max_players`, `hub_afk_seconds`,
`hunt_afk_seconds` write the three `DAINTY_` names into the Director's env file, which every game server inherits, and
`restricted/docs/hosting.md` lists them. Two facts for the integration pass: the AGPL runtime reads only the number
files, so the AFK keys take effect only once R8 lands (the cap once R9 lands, through SRV-1); and the host kit's range
starts at 0, so "native" (a negative value, R1 10.2) is reachable only by leaving the key unset for hunts or through
the files. A later row removes the files once a release that writes the variables is out (R1 10.3).

---

## 12. Log lines

All through R0's prefixed writer, tagged `[MitRuntime]`. Pointers as `0x<hex>`. Never logged: the roster's ids, the
command line, any environment value (the settings are logged as R1's parsed values with their source).

| When | Text |
| --- | --- |
| first SP-3 call | `[MitRuntime] [FramePace] server frame cap: ...` (R1's `frame_pace_line`) |
| idle edge | `[MitRuntime] [FramePace] no client for 15 s: <n> fps` / `client connected: <n> fps\|no cap` |
| sync | `[MitRuntime] [ExpectedPlayers] roster sync <on\|off (DISABLE_EXPECTED_PLAYER_ZERO.flag)> launchRoster=<n>`; `[MitRuntime] [ExpectedPlayers] <GameMode\|GameState> 0x<p> class=<c> <before> -> <n>` (once each) |
| hub role | the two `[MitRuntime] [HubRole]` lines of 5.4 |
| AFK | `[MitRuntime] [AfkTimeout] config ...`, `override off ...`, the two warnings (6.1); the per-game-state line (6.3) |
| watchdog | `started ...`, `could not start ...`, the heartbeat, `ignoring implausible counts ...`, the stale-tick lines, `port probe off ...`, the port line (7) |
| memory | the reserve lines (7.6), `[MitRuntime] [WorkingSet] trimmed ...` (7.7), `[MitRuntime] [Memory] reserve: ...` (7.1) |
| perf | `[MitRuntime] [Perf] ...`, `[MitRuntime] [Memory] connections ...` (7.8) |
| exit | `[MitRuntime] [Reap] <reason> -> TerminateProcess(0)`; `[MitRuntime] [ServerQuit] quit requested (<reason>): ending through the reap exit` |

Reap reasons: `tick: disposable instance empty 60 s (game mode <empty|unknown>)`, `disposable server empty 60 s after
its last client left`, `disposable server never joined by a client within 300 s`, `tick stale`, `port no longer served`,
`server quit (<reason>)`.

---

## 13. Calls, departures and ambiguities

### 13.1 RT-12 to RT-18 for R8

- **RT-11:** R1 reads the variables (R1 10); the host-kit keys are on `plan` (11). Nothing left for R8.
- **RT-14:** section 11.
- **RT-17:** R8 installs no hook. Its SP-3 handlers: before at `-2000` (first: pacing must precede the frame's work),
  after at `600` (below R9's 700, as R9 asked); its SP-2 handler at `0`. Nothing else depends on order.
- **RT-18:** one name, `K2_PostLogin`, matched as `FunctionName` (6.4).
- RT-8, RT-12, RT-13, RT-15, RT-16 are not R8's.

### 13.2 Calls

1. New feature id `SP-2@R8` for the post-login AFK write; patch 0054 moves that write off `SP-2@R20` (6.4, 10.3).
2. Patch 0054 gates the AGPL's connected-server block per part; the manual stamina tick and the bleedout floor go to
   R20, the pawn boost to R13 (1, 10.3).
3. R8 lands with or after R9 and R10 (1).
4. Pacing is a before handler at `-2000`; the tick steps are one after handler at `600` (4.2, 5.1).
5. The watchdog thread starts from the game thread at listen (7.1).
6. `[Perf]` and `[Memory]` are kept, reduced; `[PerfDetail]`, `[PerfNet]`, `[PerfThreads]` are not ported (7.8).
7. `WSAStartup` once before the port probe (7.5).
8. The hub cap's source is logged on every server, in the AFK config line (6.1).
9. `LEGACY_EXPECTED_PLAYER_WALK` and `SERVER_QUITGAME_EXIT` dropped (11).
10. `server_quit` for R10 is the reap exit (8.2).

### 13.3 Departures

1. The game mode is the world's, with the last live one kept, not the first found in the object array (5.4).
2. `K2_PostLogin` is matched by function name, not a substring of the full name (6.4).
3. The hang sampler (thread suspension, instruction pointer, stack unwind) is not ported; the stale-tick reap is (7.4).
4. No 50 ms wait before `TerminateProcess` (8.1).
5. The watchdog thread returns on detach instead of idling (7.2).
6. **The AFK exemption is not ported:** the AGPL set `bNoAFK` on local controllers every 5 s, but only when the
   no-local-player patch (HL-2) had failed and a phantom local controller existed; HL-2 is R5's, and a failed HL-2 is
   R5's loud failure (R5 13.2), not something R8 papers over.
7. The engine-tick time in `[Perf]` and the per-phase timings are dropped with `[PerfDetail]` (7.8).

### 13.4 Ambiguities

- The empty reap is 60 s, not the surface's 50 s (5.5).
- The sleep's resolution is the OS timer's (4.2).
- The surface's "uncapped" boot means "not idle-capped" (4.2).

No open question for the maintainer.

---

## 14. Tests

### 14.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`), every platform:

- `Pacer::step`: active 30 → 33 333 µs from the previous frame; a frame already past the target sleeps 0; active 0 never
  sleeps; idle 0 never idles; idle 5 with count 0 and listening: not idle at 14 999 ms, idle at 15 000 ms (sleep
  200 000 µs); count -1 (unknown) never idle; not listening never idle; count back to 1 leaves idle on that frame and a
  later 0 restarts the 15 s; the edge texts.
- `hub_role`: `BPGM_City_C`, `BPGM_ArchonTrainingGrounds_C`, a hunt game mode, in `GlobalGameserver`; in `PlayerHost`
  with launcher role HUB and HUNT, with and without `KEEP_HOSTED_HUB_ROLE`; the warning for HUB on a hunt class.
- `afk_plan`: `None` writes nothing; `Some(1200)` writes both floats only when either differs; game mode absent;
  `Some(0)`.
- `Sample::from`: stamps 0, 10 000 and 10 001 ms old; players 64 and 65; travelling -1; conn usable or not; `empty`.
- The empty timer: 30 polls empty with `had_client` reaps at the 30th (60 s); one occupied poll resets; `Hub` and
  `Unknown` never; without `had_client` 150 polls (300 s).
- The stale tick: not `saw_player` never; 179 999 no, 180 000 reap; the 60 s notice cadence.
- The port timer: free 45 000 ms no, 45 001 reap; one not-free poll resets; `ever_had_counts` false never; the address
  plan (`None` → `0.0.0.0`, `127.0.0.1`, `::1` and a name → off).
- The trim timer: 60 s of zero then trim once; a client between polls (`last_client_ms >= idle_since`) restarts; re-arm
  after a client; `conn_stamp` 1001 ms old is not idle; not `trim_allowed` never trims.
- `reserve_sites_ok` against the two byte strings (and one changed byte each); `reserve_block_ok` for each region field.
- The perf window: averages and rate formatting, the no-frames line, `(stale)`.
- The registry with `"R8"` in `LANDED`: `owns` true for `SP-3@R8` and `SP-2@R8`; and a test that `"R8"` in `LANDED`
  implies `"R9"` and `"R10"` in `LANDED`.

DLL (`cargo test -p rustic-isles-runtime`, Windows):

- The port probe against a UDP socket the test binds on `127.0.0.1:0`: "not free" for that address and port, "free" once
  the test closes it.
- `reap_exit` and `server_quit` are not called (they end the process); a test checks the reason texts only.
- The working-set trim on the test process: the call succeeds and logs a line (behaviour only, no figure checked).

**Never in CI:** pacing in the game, the reaps against a real server, the AFK kick, the reserve release on the
executable, patch 0054 (it needs the host's SDK to build).

### 14.2 The maintainer's play-test (collected for the one hands-on pass, rulings "Order, amended")

Surface row R8: "Empty hunt ends after 50 s, hub never; AFK kick; idle hub's CPU drops after 15 s" (the reap is 60 s on
1.14.7). Nothing waits on it.

1. Build the AGPL runtime with patches 0041, 0047, 0052 and 0054 and the MIT DLL with `"R8"`, `"R9"` and `"R10"` in
   `LANDED` (R4 landed too). Inject both as R0 11.1 says. In the Director's `settings.toml` set `[director]`
   `hub_afk_seconds = 120` and `hunt_afk_seconds = 120`, and `[performance]` `idle_fps = 5` (`DAINTY_SERVER_IDLE_FPS`);
   rerun `setup`.
2. **Ramsgate.** Let the Director start the hub. Its log shows `[MitRuntime] [FramePace] server frame cap: 30 fps, idle
   (no client for 15 s): 5 fps ...`, `[MitRuntime] [ExpectedPlayers] roster sync on launchRoster=0`, `[MitRuntime]
   [AfkTimeout] config hub=120 s (env) hunt=120 s (env) maxPlayers=4 (default)`, `[MitRuntime] [Watchdog] started ...`,
   `[MitRuntime] [Memory] reserve: released at listen + 30 s ...`, and about 30 s later `released the engine's 32 MB
   out-of-memory reserve`. No AGPL `[Watchdog/thread]`, `[FramePace]` or `[AfkTimeout]` lines. 15 s after listening:
   `no client for 15 s: 5 fps`; the process's CPU in Task Manager drops; the next `[MitRuntime] [Perf]` line shows
   about 5 frames/s. About 60 s after that, `[WorkingSet] trimmed`.
3. Log in to Ramsgate: `client connected: 30 fps` within a frame or two; `[AfkTimeout] role=hub class=BPGM_City_C ...
   applied 120 s (why=postlogin)`. Stand still: the client returns to the menu after about 120 s (the AFK kick). The hub
   stays up (`role=hub` in the heartbeat; never a reap).
4. Log in again, talk to a vendor, sprint and dodge: stamina drains and refills (the AGPL's stamina tick still runs
   through 0054).
5. **A hunt.** Start one with two players; both arrive before it starts (`[ExpectedPlayers] GameMode ... -> 2`). Play
   it; one player goes down and is revived or bleeds out normally (the floor still applies). Both leave: the heartbeat
   counts `emptySec` up and about 60 s later `[MitRuntime] [Reap] disposable server empty 60 s after its last client
   left -> TerminateProcess(0)`, then R3's `[MitExit] kernelbase!TerminateProcess ... code=0`. The Director records exit
   code 0 and frees the slot.
6. **A hunt nobody joins.** Have the Director start one and cancel the matchmaking before anyone loads: it ends after
   300 s (`never joined by a client within 300 s`).
7. **A realm hub** (player host, R10 landed). Host a realm Ramsgate from the launcher: `[HubRole] BPGM_City_C ->
   persistent hub (launcher role)`; its AFK is the launcher's. Fence it from the launcher: `[ServerQuit] quit requested
   ...` and the process ends with code 0 at once (not 0xE105 after 5 s).
8. **Port no longer served** (optional, needs a hub that leaves its map; skip if it cannot be provoked): the log shows
   `UDP port <n> has been free for 45 s ...` and the Director restarts the hub.
9. **Regression without R8.** An MIT DLL without `"R8"`: the AGPL's own `[FramePace]`, `[Watchdog/thread]` and
   `[AfkTimeout]` lines appear, no `[MitRuntime] [Watchdog]` lines, and steps 2 to 5 behave as before.
10. Note anything that differs from the same steps on the previous release. Only CI passed until these steps are done.

---

## 15. Size

Rust, without tests: the core crate about 450 lines (pacer 70, hub role 50, AFK plan 40, watchdog sample and timers 140,
trim 40, reserve checks 50, perf formatting and log texts 60); the DLL crate about 650 (install and handlers 80, sync
and count 80, game-mode sample and tick reap 90, AFK 110, the thread 120, port probe 60, reserve and trim 70, exit and
quit 40). Tests about 500. Patch 0054: about 20 lines of C++.
