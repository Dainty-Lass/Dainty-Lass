<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R9, listen and readiness (RD27, plan item 11)

> Since R21 (`spec-runtime-r21.md`) the AGPL runtime is gone; `rustic-isles-runtime.dll` is the only runtime. This
> slice's play-test runs on the MIT DLL alone, as that spec's section 9 adapts it: skip every step that builds or
> patches the AGPL runtime, injects both DLLs, counts `[MitSeam]` lines or runs without the MIT DLL.

R9 makes a game server reachable: it gives the engine the map URL a server boots with (SRV-1), creates and binds the
server's `GameNetDriver` once the world exists, keeps that driver pointed at its world every frame, and tells whoever
started the server that it is ready (the `RUSTIC_ISLES_GAMESERVER_READY` line the Director waits for). It owns the
hook SRV-1 and the feature id `SP-3@R9` (`spec-runtime-surface.md` sections 2.3, 3.9, 4.1 and 5.1;
`spec-runtime-r0.md` section 7.5). It installs nothing on a client.

An implementer builds from this page, `spec-runtime-surface.md`, `spec-runtime-r0.md`, `spec-runtime-r1.md`,
`spec-runtime-r4.md` and section 3 of `spec-runtime-r17.md` alone, and never reads `restricted/`. What crossed is facts:
RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7, CL 647472), UE 4.26 layout offsets, engine and game names, flag and
variable names, the exact texts the Director reads, and behaviour. The AGPL side (seam patch 0041, and runtime patch
0052 written with this page) is described only by the contract it keeps (section 9).

Conventions are R0's: an RVA is `+0x0XXXXXXX` from the executable's image base; **S** server. **(call)** marks a
decision this page makes, **(departure)** a deliberate difference from the AGPL runtime, **(ambiguity)** something the
source did not settle; all are collected in section 12.

---

## 1. Scope

| Id | What | Kind (R0 registry) | Section |
| --- | --- | --- | --- |
| SRV-1 | the `GetGameDefaultMap` getter answers the server's map URL | Detour | 4 |
| `SP-3@R9` | the listen request (the server half of the old MainThread), the listen itself, readiness, and the per-frame driver maintenance | Feature | 5 to 7 |

Also R9's, with no id of its own: the server's auth-mode exit `0xE102` (R1 section 5 leaves it here, section 5.2), and
the read-only channel lookups other slices use (section 8.3).

Not R9:

- The client half of the old MainThread: the F2 console is R17's (`CON-1`), the store capture left the runtime (RT-10),
  and the banner is R0's status lines (R0 4.3).
- The player host's start-up, environment, control pipe, ready pipe and fence (R10). R9 calls R10's interface where the
  listen needs it (section 8.2) and stands down for a player host until R10 lands (section 6.5).
- The empty-server watchdog, hub cap enforcement, AFK, the expected-player sync and frame pacing (R8). R9 publishes the
  state they read (section 8.1).
- Net-driver sanitising and level-collection registration before the engine tick, and the manual net tick (R11); the
  replication graph drive (R12); relevance and weapon routing (R13); the player role (R14).
- The map URL's text and the launch values: R1 builds them (R1 sections 6, 7, 10); R9 writes the result into the engine.
- Logging-only work around the listen (connection-edge, match-state and send-path traces, the replication graph dump):
  not ported (RT-3); the AGPL keeps them until R21 (section 9).

**Dependencies.** R0 (broker, SP-3 chain, flags, log, memory, names, registry), R1 (`config::launch()`,
`config::auth_mode()`, `config::port()`, `config::hub_cap()`, `roster::map_url`), R4
(`login::cmdline::engine_multihome()`,
for a pinned listen address), and the reflection kit of `spec-runtime-r17.md` section 3 (`find_class`, `is_a`,
`static_call`, `write`). **(call)** The kit is shared: whichever of R15, R17 or R9 lands first creates `reflect.rs` as
R17 section 3 defines it, with no slice's name in it. R2's `FMemory::Realloc` line is shared the same way (2.1).

---

## 2. Where it lives

| File | Kind | Holds |
| --- | --- | --- |
| `crates/rustic-isles-runtime-core/src/listen.rs` (new) | pure, every platform | the readiness texts (7.1); the listen decision table (6.5); the listen-request schedule (5.1); the `FURL` and `FString` plans (4.3, 6.3); the array checks of 6.2; the launch id check (7.1); the log texts of section 11 |
| `crates/rustic-isles-runtime/src/listen/mod.rs` (new, safe) | Windows | `install()`, the SP-3 handler, the listen state other slices read (8.1), the R10 interface slot (8.2), the channel lookups (8.3) |
| `crates/rustic-isles-runtime/src/ffi/listen.rs` (new, under `ffi`) | Windows, unsafe | the SRV-1 detour body, the calls into the engine (`CreateNamedNetDriver`, `InitListen`, `FMemory::Realloc`), the raw writes, the stdout write, and the `rir_listen_driver` export (9.2) |

Lines R9 adds to R0's files (one each, append-only):

- `registry.rs`: `"R9"` in `LANDED`. SRV-1 and `SP-3@R9` are already in `ENTRIES`.
- `offsets.rs`: the lines of 2.1.
- `lifecycle.rs`: on a server, `listen::install(...)` after `minhook::initialize()` and **before** `handlers::freeze()`
  (it registers a tick handler); its result goes into the status like every slice's.
- `ffi/mod.rs`: `pub mod listen;`. `lib.rs`: `pub mod listen;` under the same `cfg` as the other hooking slices.
- Core `lib.rs`: `pub mod listen;`.

### 2.1 Offset table lines (RT-2)

`TARGETS`:

| id | rva | found_by | what |
| --- | --- | --- | --- |
| SRV-1 | `0x02F00330` | `Rva` | the `GetGameDefaultMap` getter (`UGameMapsSettings`) |

`data` (functions R9 calls, not hooks):

| Name | Value | What |
| --- | --- | --- |
| `CREATE_NAMED_NET_DRIVER` | `0x0425_FDA0` | `UEngine::CreateNamedNetDriver` |
| `FMEMORY_REALLOC` | `0x0256_4AB0` | `FMemory::Realloc`; **shared with R2** (R2 3.1): whichever lands first adds the line |

`GWORLD` (`0x06FC_7F58`) is already R0's.

`layout` (UE 4.26 as built into 1.14.7):

| Name | Value | What |
| --- | --- | --- |
| `ENGINE_WORLD_LIST` | `0xC38` | `UEngine`: pointer to an array of `FWorldContext*` |
| `ENGINE_WORLD_LIST_NUM` | `0xC40` | `i32` count of that array |
| `WORLD_CONTEXT_WORLD` | `0x280` | `FWorldContext`: its `UWorld*` (readable span `0x288`) |
| `WORLD_CONTEXT_ACTIVE_NET_DRIVERS` | `0x220` | `TArray<FNamedNetDriver>`: data `+0x220`, `Num` `+0x228`, `Max` `+0x22C` |
| `NAMED_NET_DRIVER_STRIDE` | `0x10` | element size; the `UNetDriver*` is at element `+0x00` |
| `NET_DRIVER_SIZE` | `0x2B0` | the span every driver access stays inside |
| `NET_DRIVER_SERVER_CONNECTION` | `0x88` | pointer; null on a server |
| `NET_DRIVER_CLIENT_CONNECTIONS` | `0x90` | `TArray<UNetConnection*>`: data `+0x90`, `Num` `+0x98`, `Max` `+0x9C` |
| `NET_DRIVER_WORLD` | `0x140` | `UWorld*` |
| `NET_DRIVER_NAME` | `0x190` | `FName` `NetDriverName` (8 bytes) |
| `NET_DRIVER_INIT_LISTEN_SLOT` | `0x290` | byte offset of `InitListen` in the driver's vtable |
| `WORLD_NETWORK_NOTIFY` | `0x28` | the world's `FNetworkNotify` interface (an address inside the world, passed as is) |
| `WORLD_NET_DRIVER` | `0x38` | `UWorld::NetDriver` (readable span `0x40`) |
| `FURL_SIZE` | `0x68` | |
| `FURL_HOST` | `0x10` | `FString` |
| `FURL_PORT` | `0x20` | `i32` |
| `CONNECTION_OPEN_CHANNELS` | `0x70` | `UNetConnection`: `TArray<UChannel*>`: data `+0x70`, `Num` `+0x78`, `Max` `+0x7C` (readable span `0x80`) |
| `CONNECTION_STATE` | `0x134` | `u32`; `3` is open (for R8; R9 does not read it) |
| `CHANNEL_ACTOR` | `0x70` | `UActorChannel`: its `AActor*` |
| `GAME_MAPS_DEFAULT_MAP` | `0x70` | `UGameMapsSettings`: the `GameDefaultMap` `FString` the getter copies (named for the reader) |

**(ambiguity)** `FURL_SIZE` and `FURL_HOST` are stock UE 4.26's layout (`Protocol`, `Host`, `Port`, `Valid`, `Map` at
`+0x28`, `RedirectURL`, `Op`, `Portal`); the AGPL's notes confirm `Port` and `Map` on this build, not the size. A zeroed
`0x68` block with `Port` written is what the AGPL passes (a default-constructed URL), and its listen works.

---

## 3. Registry, ownership and the AGPL in one table

With `"R9"` in `LANDED`, `rir_owns` answers 1 for `SRV-1` and `SP-3@R9`; the status line's `owns=` count grows by 2.

| Situation | Who builds the map URL | Who listens and announces | Who keeps the driver maintained |
| --- | --- | --- | --- |
| MIT DLL absent | AGPL | AGPL | AGPL |
| R9 landed, DeployServer | MIT | MIT | MIT |
| R9 landed, player host, R10 landed | MIT | MIT (through R10's interface) | MIT |
| R9 landed, player host, R10 **not** landed | MIT | **AGPL** (patch 0052) | AGPL |

The last row is the stand-down of 6.5: the AGPL's listen needs its own player-host module (pinned address, ready
pipe, fence), which the MIT cannot call. Both sides decide it the same way, from the auth mode and whether `SP-3@R10`
is owned.

---

## 4. SRV-1: the map URL (S)

### 4.1 Why

A dedicated server loads the map its `GetGameDefaultMap` getter names. The game's own value is a fixed config map; the
Director (and the realm launcher) pass the real map, Behemoth, hunt and roster on the command line. Without
`?MaxPlayers=` in the URL, the game mode's `InitGame` caps the server at **one** player and strands a second joiner on
the loading screen.

### 4.2 The detour (band `Mit`, `order = 0`)

Target `+0x02F00330`, found by RVA. Signature: hidden-return, `fn(out: *mut FString) -> *mut FString`; the getter
fills `*out` with an engine-allocated copy of the configured default map and returns `out`.

Each call (any thread; in practice the game thread, many times per second while the engine boots and loads):

1. If detaching: return `next(out)`.
2. `ret = next(out)`. If `ret` is null or not readable for `0x10`: return `ret` untouched.
3. Take the URL (4.3), built **once** per process.
4. Write it into `*ret` (4.4). Return `ret`.

R9 does not log per call. The first call logs the one line of section 11 (`[MapUrl]`).

### 4.3 The URL (built once)

`roster::map_url(config::launch(), config::hub_cap().value, listen)` (R1 7.2), UTF-16, plus a terminating NUL, where:

- `listen` = **false**, always. **(call)** `MAP_URL_LISTEN.flag` (an A/B switch that made the engine build the driver
  during map load instead) is dropped (flags 10): R9 always listens through the net driver itself (6.4), so every slice
  that reads the listening driver gets the one R9 made.
- **(call)** When `config::launch()` is `None` (a server whose launch R1 could not parse would already have ended in
  Init, so this is a client misclassified or a broken Init), SRV-1 returns `next`'s answer untouched.

### 4.4 Writing into the engine's string

The engine owns `*ret` and will later grow or free it, so the bytes must live in an engine allocation (the AGPL crashed
the server at boot when it aliased a buffer of its own: the engine's allocator refuses a block it did not make).

- `count` = URL length + 1 (the NUL), in UTF-16 units.
- If `ret.data` is null or `ret.max < count`: `data = FMemory::Realloc(ret.data, count * 2, 0)` (`+0x02564AB0`; a null
  pointer acts as a malloc). A null result logs once `[MitRuntime] [MapUrl] engine allocation failed; the engine keeps
  its own map` and returns `ret` unchanged. Otherwise store `data` in `ret.data`.
- Copy the URL and its NUL into `data`; `ret.num = count`; `ret.max = max(ret.max, count)`.
- The plan (`realloc needed?`, the new `num` and `max`) is the pure `fstring_plan(num, max, data_null, count)`.

### 4.5 Install

Server only, in Init with R9's other work (2), after `minhook::initialize()`. **Settled (RT-17 for R9):** the only
order rule is that SRV-1 is in place before the engine's first map query, which every install in `DllMain` satisfies
(R0 3.1). The result is R0's `install SRV-1 ok|<reason>` line.

---

## 5. The listen request (`SP-3@R9`, the server half of the old MainThread)

The AGPL started a thread from `DllMain` that waited for the world and then asked the engine tick to listen. R0 3.1
forbids starting a thread in `DllMain`, so **(departure)** the MIT does the same work on the game thread, inside its
SP-3 handler, with the same outcome: the listen happens on the first engine frame at which the world exists.

### 5.1 The handler

Two SP-3 **after** handlers, `slice = "R9"` **(call: orders)**. They run after the engine tick and after the AGPL's
tick wrapper (the chain puts every MIT after handler outside the AGPL's work, R0 8.1). The AGPL's order inside its
listening block was: R8's expected-player sync and connection count, then the driver maintenance, then R11's manual net
tick; and the listen itself was the last step of its tick, so features acting "once listening" first saw it on the
next frame. Hence:

- **maintenance, `order = 700`**: while listening, driver maintenance (7.2). Below R11's manual net tick (`order = 800`,
  R11's spec); R8's listening-block steps take orders below 700.
- **listen, `order = 900`**: the steps below, after every other after handler of the frame.

The listen handler, each call, in this order:

1. **First call only:** if `config::auth_mode()` is `Invalid`, log `[MitRuntime] [ServerAuth] unknown
   RUSTIC_ISLES_AUTH_MODE; terminating before listen` and end the process with `TerminateProcess(current, 0xE102)`.
   (R1 classifies an unreadable value as `Invalid` too, a departure R1 records.)
2. If listening (8.1): return.
3. If the decision of 6.5 is `AgplListens`: log once (section 11) and return. R9 does nothing more in this process.
4. If the listen is not yet requested: the request schedule (pure, `ListenSchedule`): requested when `GWorld`
   (`+0x06FC7F58`, read through R0's checked read) is non-null, or when 30 000 ms have passed since this handler's first
   call. Log `[MitRuntime] [Listen] listen requested (world present)` or `... (no world after 30 s; trying anyway)`.
   **(departure)** The AGPL's 30 s ran from `DllMain`; the MIT's runs from the first engine frame. On a real server the
   world exists at the first frame (the initial map loads before the engine ticks), so both request on that frame.
5. If requested and not yet tried: the listen step (6.4). A failed listen is not retried (as the AGPL).

`SP-3` handlers cannot absorb (R0 call 15); R9's never needs to.

### 5.2 What the old MainThread did that R9 drops

The per-second `[MainThread] wait=` lines, the session banner (R0's status lines replace it), the `[BootPatch]` status
(R5's), and the player host's start-up call (R10's). The client half is R17's and lane E's (section 1).

---

## 6. The listen (`SP-3@R9`)

### 6.1 Inputs

- `engine`: the SP-3 handler's engine argument.
- `world`: `GWorld`. `port`: `config::port()`.
- The `GameNetDriver` name: the `FName` returned by `KismetStringLibrary.Conv_StringToName("GameNetDriver")`, called
  once through the reflection kit's `static_call` (parameter block `0x18`: `InString` `FString` at `+0x00`,
  `ReturnValue` `FName` at `+0x10`) and kept for the process. **(call)** The call is made from R9's SP-3 handler on the
  server's game thread; it goes through `ProcessEvent` and so down the server's SP-2 chain, as any game call does. No
  SP-2 matcher of any slice matches `Conv_StringToName`.
- `FName` equality is equality of both halves (comparison index `i32` at `+0x00`, number `u32` at `+0x04`).

### 6.2 Finding the driver

1. **World context.** Read `ENGINE_WORLD_LIST` and `ENGINE_WORLD_LIST_NUM` from `engine` (readable `engine + 0xC48`).
   Refuse a count below 0 or above 64 **(call)**, and an array not readable for `count * 8`. The context is the first
   non-null entry, readable for `0x288`, whose `WORLD_CONTEXT_WORLD` equals `world`. None: fail `no world context for
   the world`.
2. **Create.** Call `CreateNamedNetDriver` at `+0x0425FDA0` as `extern "C" fn(engine, context, name: u64, definition:
   u64) -> u8`, passing the `GameNetDriver` `FName` as both name and definition (an 8-byte `FName` goes by value in a
   register: comparison index in the low 32 bits, number in the high). Log its answer. A `false` answer is not a
   failure by itself (a driver of that name may already exist); the next step decides.
3. **Find it, active list first.** The context's `WORLD_CONTEXT_ACTIVE_NET_DRIVERS`: refuse `Num < 0`, `Num > Max` or
   `Max > 128`, and data not readable for `Num * 0x10` (each refusal logs once and skips to step 4). For each element,
   the driver pointer at `+0x00`: readable for `0x2B0`, `is_a(candidate, "NetDriver")`, and `NET_DRIVER_NAME` equal to
   the `GameNetDriver` name. The first match wins.
4. **Else scan the object array** (R0 6.2) by index: skip default objects (object flag `0x10`), keep the first object
   that is readable for `0x2B0`, `is_a(_, "NetDriver")`, and has that name. Log that the scan was used.
5. None: fail `named net driver not found`.

The array checks are the pure `world_list_ok(count, readable)` and `named_drivers_ok(num, max)` (CI-tested).

### 6.3 Binding and `InitListen`

With the driver `d`, through the kit's checked `write`:

1. `NET_DRIVER_NAME` ← the `GameNetDriver` name; `NET_DRIVER_SERVER_CONNECTION` ← null; `NET_DRIVER_WORLD` ← `world`.
2. **The URL.** A zeroed `FURL_SIZE` (`0x68`) byte block R9 owns for the call; `FURL_PORT` ← `port`.
3. **The player host's address (only when R10 is landed and the mode is `PlayerHost`, 6.5).** Ask R10's
   `prepare_listen()` (8.2): `Err` fails `player host refused the listen`; `Ok(None)` pins nothing; `Ok(Some(host))`
   pins `host`. A pinned host is honoured only when R4's `engine_multihome()` is `Some(m)` and `m` equals `host`
   ignoring ASCII case: the socket binds the engine command line's `MULTIHOME`, not the URL's host, so a pin the engine
   line does not carry would bind every interface. Otherwise fail with the line of section 11 naming both (`none` when
   the engine has no `MULTIHOME`). When honoured, log the match and write `FURL_HOST` as an `FString` over a
   process-lifetime UTF-16 copy of `host` (NUL-terminated, `num = max = len + 1`).
4. **`InitListen`.** The driver's vtable (its first pointer, readable for `0x298`); the function at byte `+0x290`.
   **(departure)** R9 calls it only if R0's `function_entry(target − image base)` equals `target − image base` (a
   function start inside the executable); otherwise fail `InitListen is not a function entry`. Call it as
   `extern "C" fn(driver, notify, url, reuse_address_and_port: u8, error: *mut FString) -> u8` with
   `notify = world + 0x28`, `url` the block above, `reuse = 0`, and `error` a zeroed `FString` R9 owns.
5. If `error.num > 1` and its data is readable: decode at most 512 units (lossy) and log `InitListen error: <text>`.
   **(call)** The engine allocated that text; R9 does not free it (a few bytes, once; the AGPL does not either).
6. `InitListen` answered 0: fail `InitListen returned false`.
7. **Bind the world.** Again `NET_DRIVER_WORLD` ← `world`, `NET_DRIVER_NAME` ← the name,
   `NET_DRIVER_SERVER_CONNECTION` ← null. Then the context's own world `w = context + 0x280` (readable `0x288`): if
   `w` is non-null and readable for `0x40`, write `w + WORLD_NET_DRIVER` ← `d` when it differs (log `was null`,
   `replaced another driver` or `already ours`), and if `d`'s `NET_DRIVER_WORLD` is not `w`, write `w` there (log
   `realigned`). If `w` cannot be read, log the warning of section 11 and carry on. Without this write the engine's
   tick never reads the new socket (the server makes no receive calls at all); and the driver belongs to exactly one
   world, so only the context's world is written, never every world.
8. **Readiness** (7.1). A failure there fails the listen (`readiness report failed`).
9. **Listening:** publish `d` (8.1), log the success line.

Any failure: log `[MitRuntime] [Listen] failed: <reason>`; when the mode is `PlayerHost`, call R10's
`on_listen_failed()` (8.2; it fences the host). A DeployServer that failed stays up without listening; the Director's
ready timeout then stops it (as the AGPL).

### 6.4 One listen path

6.2 then 6.3, always. **(departure)** The AGPL also had an engine-listener path behind `MAP_URL_LISTEN.flag` (the map
URL carried `?listen` and it only waited for the engine's driver, announcing readiness but keeping no driver, so its
connected-server features never ran there). It was never the default; R9 drops it (4.3, flags 10).

### 6.5 Who listens: the decision (pure, `listen_decision`)

| Auth mode | `owns("SP-3@R10")` | Decision |
| --- | --- | --- |
| `GlobalGameserver` | any | `Mit` |
| `PlayerHost` | 1 | `Mit` (with R10's interface) |
| `PlayerHost` | 0 | `AgplListens` |
| `Invalid` | any | (the process ended at 5.1 step 1) |

`AgplListens` is the AGPL's own rule in patch 0052 (9.3), computed from the same two facts, so the two DLLs never both
listen and never both stand down. **(call)** It means R9 can land before R10 without breaking realm hosting: the AGPL
keeps the player host's whole listen until R10 lands. With `AgplListens`, R9 publishes no listening state (8.1 is
`None` for the process) and keeps no driver; SRV-1 is still the MIT's and builds the same URL the AGPL would.

---

## 7. Readiness and driver maintenance

### 7.1 Readiness

- **DeployServer** (`GlobalGameserver`): read `RUSTIC_ISLES_GAMESERVER_LAUNCH_ID` with R0's `env_value(name, 127)`.
  None: log `[MitRuntime] [Listen] no RUSTIC_ISLES_GAMESERVER_LAUNCH_ID: no ready marker`; readiness still succeeds
  (as the AGPL: a server started by hand has no launch id). Some(`id`): write to the process's standard output
  (`GetStdHandle(STD_OUTPUT_HANDLE)`), each with one `WriteFile`, the two lines

  ```text
  RUSTIC_ISLES_GAMESERVER_READY launchId=<id> port=<port>\r\n
  MYSTICPARADOX_GAMESERVER_READY launchId=<id> port=<port>\r\n
  ```

  in that order (`\r\n` is what the AGPL's text-mode stream writes; the Director trims both forms). A null or invalid
  handle, or a short write, logs `stdout unavailable: no ready marker` and readiness still succeeds (nothing on the
  other end could read it). Then log `ready marker written launchId=<id>`.
  The Director parses only the first line, with a lowercase-hex-and-hyphen id; the second is for a Director from before
  the rename and goes in R10's joint cutover (RT-8, section 10). R9 keeps both, byte for byte.
- **Player host** (R10 landed): `report_ready(port)` (8.2); `false` is a readiness failure. R9 writes nothing to
  stdout for a player host.

The texts are built by the pure `ready_lines(id, port)`; `launch_id_ok` is R0's env check (CR, LF, NUL and non-ASCII
refused: **(departure)** the AGPL printed whatever 1 to 127 bytes it read).

### 7.2 Driver maintenance, every frame while listening

On the listening driver `d`, in R9's maintenance handler (5.1, order 700):

- If `d` is no longer a live object (R0's `is_live_object`), or not readable for `0x2B0`, or `GWorld` is null: skip
  this frame. Log `[MitRuntime] [Listen] driver maintenance skipped: <driver no longer live|driver unreadable|no
  world>` once per streak (again after a frame that did the work). **(departure)** The AGPL wrote through the pointer
  without a liveness check.
- Else write `NET_DRIVER_WORLD` ← `GWorld`, `NET_DRIVER_SERVER_CONNECTION` ← null, `NET_DRIVER_NAME` ← the
  `GameNetDriver` name. (The engine resets these on this build; a server driver with a stale world or a non-null server
  connection is what faulted the engine's dispatch.)

This is all of the AGPL's per-tick networking routine that runs on 1.14.7. Its manual replication loop and the frame
counters it advanced run only with `EMERGENCY_LEGACY_REPLICATION.flag`, and even then the loop refuses itself because
it calls 1.12 addresses that are not function starts on 1.14.7; R9 ports none of it (flags 10). Its `[GraphState]`
dump is logging-only (RT-3).

---

## 8. Interfaces

### 8.1 What R9 publishes (for R8, R10, R11, R12, R13, R14, R20)

`rustic_isles_runtime::listen`:

- `state() -> Option<Listening>`: `None` until the listen succeeded, and always in the `AgplListens` case.
  `Listening { driver: usize, port: u16, since_ms: u64 /* GetTickCount64 at success */ }`.
  Set once (an atomic pointer to a `OnceLock` value, Acquire/Release), never cleared. Readable from any thread.
- `listening() -> bool` = `state().is_some()`, true from the frame the listen succeeded.
- `net_driver() -> Option<usize>` = `state().map(|s| s.driver)`: the driver R9's listen made or found (6.2); with
  the one listen path (6.4) it is `Some` whenever `listening()` is true.
- `game_net_driver_name() -> Option<(i32, u32)>` (6.1: comparison index, number), once known.
- `decision() -> Decision` (6.5).

What each later slice needs from it:

| Slice | Needs |
| --- | --- |
| R8 | `state()` (the watchdog and pacing start "once listening"; pacing's 15 s uncapped window counts from `since_ms`); the driver's client connections (`client_connections`, 8.3) and `CONNECTION_STATE`. **R8 must land with or after R10**, or treat `AgplListens` itself: in that case R9 publishes no state and the AGPL's watchdog runs (patch 0052) |
| R10 | implements 8.2; the ready pipe uses the port R9 passes; owns the `MYS1`, pipe-prefix and `MYSTICPARADOX_` renames (RT-8) |
| R11 | `listening()`, `net_driver()`, `game_net_driver_name()` for its sanitising and level-collection registration before the engine tick and its manual net tick (an after handler at `order = 800`, between R9's two). R11 lands **with or after** R9: its MIT parts need R9's driver. R9 does not need R11: patch 0052 gates the AGPL's manual net tick on `SP-3@R11` and hands it R9's driver, so with R9 alone the AGPL's R11 keeps running |
| R12, R13, R14 | `net_driver()`, `client_connections`, `actor_channel` (8.3) |
| R20 | `state()` where a post-login behaviour waits for a listening server |

### 8.2 What R9 expects from R10

A trait R10 implements and registers once in its own install (before `handlers::freeze()`):

```rust
pub trait PlayerHostListen: Sync {
    /// `Err` refuses the listen; `Ok(Some(host))` pins the address the engine must already bind.
    fn prepare_listen(&self) -> Result<Option<&'static str>, ()>;
    /// Reports readiness to the launcher (R10's ready pipe); false fails the listen.
    fn report_ready(&self, port: u16) -> bool;
    /// The listen failed: fence the host.
    fn on_listen_failed(&self);
}
pub fn register_player_host(listen: &'static dyn PlayerHostListen) -> Result<(), &'static str>;
```

(Shape, not names.) Today's contract, which R10's spec restates: a player host pins `127.0.0.1` when
`RUSTIC_ISLES_BIND_ADDRESS` is exactly that and refuses otherwise; the launch is `-MULTIHOME=127.0.0.1`, which R4
carries into the engine line. If the decision is `Mit` for a player host but nothing is registered (a build error),
R9 fails the listen with `player host has no listen interface` and does not fence (there is nothing to fence with):
fails closed.

### 8.3 Read-only lookups for later slices

In `listen` (pure checks in the core crate over R0's `Memory`):

- `client_connections(driver) -> Vec<usize>`: the `NET_DRIVER_CLIENT_CONNECTIONS` array if `0 <= Num <= Max`,
  `Num <= 1024` and the data is readable for `Num * 8` (R0's `sane_pointer_array`); else empty. Null entries kept out.
- `actor_channel(connection, actor) -> Option<usize>`: `connection` readable for `0x80`; its `CONNECTION_OPEN_CHANNELS`
  array sane with limit 4096 (else `None`); the first channel, 8-byte aligned and readable for `0x78`, whose class
  (`+0x10`) is **exactly** the `ActorChannel` class (`find_class("ActorChannel")`, not `is_a`) and whose
  `CHANNEL_ACTOR` is `actor`.
- `channel_presence(actor, connection) -> i32`: `-2` when `actor` is null or `connection` not readable for `0x80`;
  `1` when `actor_channel` finds one; else `-1`.

R9 calls none of these itself. They are the 1.14.7 behaviour of the AGPL's channel helpers: **no slice creates or
pushes an actor channel by hand**, because the addresses the AGPL used for that are 1.12's and are not function starts
on 1.14.7 (one of them lands inside the spline direction query, NETD-7's target, and crashed hubs); the driven
replication graph opens channels itself. A slice that wants to report on a channel uses these lookups.

---

## 9. The AGPL side

### 9.1 What seam patch 0041 already does when the MIT owns R9's ids

- SRV-1 is a plain hook id: the AGPL makes no wrapper on `+0x02F00330`; its original pointer becomes `rir_original`.
- `SP-3@R9` gated the AGPL's listen steps and its whole "listening" block. Patch 0052 replaces that gate (9.3).
- The AGPL's MainThread is not gated. It still logs its banner, still ends the process on an unknown
  `RUSTIC_ISLES_AUTH_MODE` (`0xE102`, harmless beside R9's), still starts the player host (R10's, until R10), and still
  waits for the world and asks its tick to listen; with R9 owning the listen, nothing acts on that request.

### 9.2 The export R9 adds: `rir_listen_driver`

`void* rir_listen_driver(void)`, C ABI, exported by name from `rustic-isles-runtime.dll`, defined in R9's
`ffi/listen.rs` (R0's `exports.rs` is not touched): the driver of `state()`, or null. Pure load (Acquire), no lock,
any thread, catches panics (null). It is not part of seam ABI version 1's required set: `rir_seam_version` stays 1,
and the AGPL resolves it with `GetProcAddress` only when it needs it (9.3). R9's DLL test checks it is exported.

### 9.3 Runtime patch 0052 (written with this page)

Without it, a server whose `SP-3@R9` is the MIT's would leave the AGPL's "listening" flag false and its net-driver
pointer null for the process. Every AGPL feature still on the tick reads them: the empty-server watchdog and its
connection count, frame pacing, the replication graph drive (without which nothing replicates), base-actor and weapon
routing, the player role routes, the Hunt Pass server state, net-driver sanitising. The server would accept a
connection and replicate nothing.

The patch (`restricted/modules/rustic-isles/native/runtime/dllmain.cpp`, `Networking.cpp`, `Networking.h`, marked
`[DaintyLass 0052]`, row 0052 in `restricted/docs/paradox-patches.md`) keeps this contract:

- **Who listens.** The AGPL listens (its listen request, `DISABLE_MANUAL_LISTEN.flag`, the engine-listener wait, the
  manual listen and its per-tick driver maintenance) exactly when `SP-3@R9` is not owned, **or** the auth mode is
  `HUNT_CAPABILITY` and `SP-3@R10` is not owned: the table of 6.5, read from the same facts.
- **Adopting the MIT's driver.** Otherwise, at the top of each server tick, before anything reads "listening": while
  not yet listening, it calls `rir_listen_driver()`; the first non-null answer becomes its net driver (and the port its
  networking lines log to), sets "listening", logs once `[MitSeam] SP-3@R9 is the MIT runtime's: listening on its
  GameNetDriver <ptr>`, and, when `SP-3@R8` is not owned, starts its empty-server watchdog thread (as its own listen
  step did). A MIT DLL without the export logs once `[MitSeam] SP-3@R9 is the MIT runtime's but it exports no
  rir_listen_driver; listening stays off here` and never listens.
- **The listening block is gated per part, not as a whole:** the expected-player sync and the watchdog's connection
  count on `SP-3@R8`; the per-tick driver maintenance on the "who listens" rule above; the manual net tick on
  `SP-3@R11`; the logging-only traces (connection edges, match state, send path, the tick trace) not gated. So
  `SP-3@R9` now covers the listen steps and the driver maintenance only. **(call)** This refines R0 section 7.5's
  description of `SP-3@R9`, which put R8's sync and R11's manual net tick inside it.
- No seam ABI change: seam version stays 1.
- **With the MIT DLL absent**, or with R9 not landed, the AGPL behaves exactly as before.

### 9.4 Cross-slice state (R0 7.5's table: one row replaced, two added)

| State | Set by | Read by | Rule |
| --- | --- | --- | --- |
| "listening" and the net driver | `SP-3@R9` | the AGPL's R7 (Hunt Pass), R8, R11, R12, R13, R14 and diagnostic blocks; the MIT's 8.1 readers | **carried: patch 0052.** Replaces R0's row "R8, R9 and R10 ship in one runtime release": R9 may land alone |
| the player host's listen | R10's module | R9's listen (6.3) | carried by the stand-down (6.5): the AGPL listens for a player host until R10 lands |
| the watchdog's start | the listen step | R8's watchdog | carried: patch 0052 starts the AGPL's watchdog when it adopts the MIT's driver and `SP-3@R8` is not owned; the MIT's R8 starts its own from `state()` |

---

## 10. Flag files (RT-14 for R9) and names (RT-8)

| Flag | Kept? | Meaning in the MIT runtime |
| --- | --- | --- |
| `MAP_URL_LISTEN` | **dropped (call)** | the engine-listener A/B path (6.4); the map URL never carries `?listen` |
| `DISABLE_MANUAL_LISTEN` | **dropped (call)** | an A/B switch that made a server never listen; no host wants it |
| `NATIVE_REPLICATION_ONLY`, `REVERSE_CONNECTION_ORDER`, `HYBRID_REPLICATION`, `LEVEL_VISIBILITY_GATE` | **dropped** | read only by the manual replication loop, which never runs on 1.14.7 (7.2) |
| `EMERGENCY_LEGACY_REPLICATION` (appendix A gives it to R11) | **dropped** | in the AGPL's networking routine it only advanced two frame counters and turned on the refused loop; R9 ports neither. R11's spec drops it for R11's own code too |

`NATIVE_GRAPH_ONLY` in the surface's list is not a file; it was the routine's built-in default.

**RT-8.** R9 keeps today's names: the stdout `MYSTICPARADOX_GAMESERVER_READY` line beside the `RUSTIC_ISLES_` one
(7.1). R10's joint cutover with the launcher and the Director renames or removes it (the Director already ignores it);
`MYS1`, the `MysticInternalServer.` pipe prefix and the `MYSTICPARADOX_` environment alias are R10's and R1's, not
R9's.

---

## 11. Log lines

All through R0's prefixed writer, tagged `[MitRuntime]` **(call: the AGPL wrote its networking lines raw, without the
timestamp; R0 4.3 offered the raw form for this, and the tagged form reads better beside the AGPL's in the shared
file)**.
Pointers as `0x<hex>`. Never logged: the map URL's hunt ids and roster (only its map path and length), the command
line, any environment value except the launch id.

| When | Text |
| --- | --- |
| first SRV-1 call | `[MitRuntime] [MapUrl] map=<map path> length=<n> maxPlayers=<cap>` |
| SRV-1 allocation failed (once) | `[MitRuntime] [MapUrl] engine allocation failed; the engine keeps its own map` |
| auth mode invalid | `[MitRuntime] [ServerAuth] unknown RUSTIC_ISLES_AUTH_MODE; terminating before listen` |
| stand-down (once) | `[MitRuntime] [Listen] player host without R10 in this build: the AGPL runtime listens (patch 0052)` |
| request | `[MitRuntime] [Listen] listen requested (world present)` / `(no world after 30 s; trying anyway)` |
| create | `[MitRuntime] [Listen] CreateNamedNetDriver returned <true\|false>` |
| found | `[MitRuntime] [Listen] GameNetDriver 0x<p> (<active drivers\|object scan>)` |
| pinned | `[MitRuntime] [Listen] engine MULTIHOME matches the pinned listen address <host>` / `[MitRuntime] [Listen] failed: pinned listen address <host> is not the engine's MULTIHOME (<m\|none>)` |
| error text | `[MitRuntime] [Listen] InitListen error: <text>` |
| world | `[MitRuntime] [Listen] UWorld::NetDriver on the world context's world: <was null\|replaced another driver\|already ours>`; `... driver World realigned to the world context's world`; `[MitRuntime] [Listen] WARNING: the world context's world is unreadable; the engine will not drive this driver` |
| readiness | `[MitRuntime] [Listen] ready marker written launchId=<id>` / `no RUSTIC_ISLES_GAMESERVER_LAUNCH_ID: no ready marker` / `stdout unavailable: no ready marker` |
| success | `[MitRuntime] [Listen] listening on port <n>, driver 0x<p>` |
| failure | `[MitRuntime] [Listen] failed: <reason>`, `<reason>` one of `no world`, `no world context for the world`, `GameNetDriver name unavailable`, `named net driver not found`, `player host refused the listen`, `player host has no listen interface`, `InitListen is not a function entry`, `InitListen returned false`, `readiness report failed`, or the pinned line above |
| maintenance | `[MitRuntime] [Listen] driver maintenance skipped: <driver no longer live\|driver unreadable\|no world>` (once per streak) |

---

## 12. Calls, departures and ambiguities

### 12.1 RT-12 to RT-18 for R9

- **RT-14:** section 10.
- **RT-17:** SRV-1 only needs to be installed before the engine's first map query (4.5); R9's SP-3 handler order is
  `700` (maintenance) and `900` (listen) (5.1), around R11's `800`. Nothing else in R9 depends on order.
- **RT-18:** R9 registers no `ProcessEvent` handler; it matches no UFunction name.
- **RT-8:** names kept, R10 renames (10).
- RT-12 (R7), RT-13, RT-15 (R1), RT-16 (R4) are not R9's.

### 12.2 Calls

1. Runtime patch 0052 and the optional export `rir_listen_driver` carry "listening" and the net driver to the AGPL;
   seam version stays 1 (9.2, 9.3).
2. `SP-3@R9` is narrowed to the listen steps and the driver maintenance; R8's sync and count and R11's manual net tick
   are gated on their own ids (9.3). R0's rule "R8, R9 and R10 ship in one release" no longer binds R9 (9.4).
3. The player host's listen stays AGPL until R10 lands (6.5); R8 lands with or after R10 (8.1).
4. The listen request runs on the game thread from SP-3; two after handlers, maintenance at order 700 and the listen
   at 900, around R11's manual net tick at 800; no thread is started (5).
5. The `GameNetDriver` name comes from the game's `Conv_StringToName`, through R17's reflection kit, called from SP-3.
6. `MAP_URL_LISTEN`, `DISABLE_MANUAL_LISTEN`, `EMERGENCY_LEGACY_REPLICATION` and the replication-loop flags
   dropped (10).
7. The world list is refused above 64 entries.
8. `InitListen`'s error text is not freed.
9. SRV-1 returns the engine's own answer when there is no launch.
10. The channel lookups (8.3) are R9's module but called only by later slices; nothing creates a channel by hand.
11. Log lines are tagged and timestamped, and the map URL is never logged in full (11).

### 12.3 Departures

1. No MainThread; the 30 s world window counts from the first engine frame (5).
2. No engine-listener path (`MAP_URL_LISTEN.flag`, 6.4).
3. `InitListen`'s target must be a function start in `.pdata` (6.3).
4. Driver maintenance checks liveness and skips a dead driver (7.2).
5. The launch id is refused with control or non-ASCII characters (7.1).

### 12.4 Ambiguities

- `FURL_SIZE` (`0x68`) and `FURL_HOST` are stock UE 4.26's; only `Port` and `Map` are confirmed on this build (2.1).
- The AGPL's notes list `InitListen` at vtable `+0x280` as unverified while its working call uses `+0x290`. This page
  takes `+0x290`, the one that listens today; the `.pdata` check (6.3) fails closed if it is wrong.
- The engine can later overwrite `UWorld::NetDriver` from its level collections; R11's level-collection registration
  (AGPL until R11) is what keeps it, not R9.

No open question for the maintainer.

---

## 13. Tests

### 13.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`), all on every platform:

- `fstring_plan`: null data; `max` below, at and above `count`; the new `num` and `max`.
- The map URL is `roster::map_url(launch, cap, false)`: never `?listen` (R1's tests cover the text).
- `ListenSchedule`: world absent then present (requested on that call); absent for 29 999 ms (not requested) and
  30 000 ms (requested); requested once only.
- `listen_decision`: every row of 6.5.
- `world_list_ok`: counts −1, 0, 64, 65; unreadable array. `named_drivers_ok`: `Num < 0`, `Num > Max`, `Max = 128`
  and `129`.
- `ready_lines("0f…-…", 8780)`: both lines byte for byte, `\r\n` endings; and the first line parses with the
  Director's `parse_marker` (a dev-dependency on `rustic-isles-director`, or the same rule copied into the test).
- The FURL block: `0x68` zeroed bytes, port at `+0x20` little-endian, host `FString` at `+0x10` only when pinned.
- `actor_channel` over R0's test `Memory`: a connection with three channels, one of another class with the actor, one
  `ActorChannel` with another actor, one match; an unaligned channel pointer; `Num > Max`; `Num = 4097`;
  `channel_presence` −2, −1, 1. `client_connections`: `Num = 1025`, null entries.
- The registry with `"R9"` in `LANDED`: `owns` true for `SRV-1` and `SP-3@R9`, still false for `SP-3@R8`,
  `SP-3@R10`, `SP-3@R11`; the offsets table test with SRV-1's line.

DLL (`cargo test -p rustic-isles-runtime`, Windows):

- `rir_listen_driver` is exported by name and answers null in the test process.
- SRV-1's write path against a fake `FString` and a test allocator in place of `FMemory::Realloc` (the realloc call
  goes through a function pointer the test can set): grows when `max` is short, reuses when it fits, leaves the string
  alone when the allocator answers null.
- The stdout writer against a pipe the test creates as the standard output handle: the two lines arrive in order.

**Never in CI:** the listen against the engine, the Director's ready wait, a client joining, patch 0052 (it needs the
host's SDK to build).

### 13.2 The maintainer's play-test (collected for the one hands-on pass, rulings "Order, amended")

Surface row R9: "Director starts a hub, READY seen, login to Ramsgate, start a hunt". Nothing waits on it.

1. Build the AGPL runtime with patches 0041, 0047 and 0052 (R0 11.2) and the MIT DLL with `"R9"` in `LANDED` (R4
   landed too). Inject both as R0 11.1 says.
2. **Ramsgate.** Let the Director start the hub. In `rustic_isles_dll_port<port>.log`: `install SRV-1 ok`;
   `[MitRuntime] [MapUrl] map=... maxPlayers=4` (or the host's cap); `listen requested (world present)`;
   `CreateNamedNetDriver returned ...`; `GameNetDriver 0x... (active drivers)`; `UWorld::NetDriver on the world
   context's world: ...`; `ready marker written launchId=<id>`; `listening on port <port> (manual), driver 0x...`;
   then the AGPL's `[MitSeam] SP-3@R9 is the MIT runtime's: listening on its GameNetDriver 0x...` with the same pointer.
   No AGPL `[Networking::Listen]` lines. The Director marks the server ready within its timeout.
3. Log in, enter Ramsgate, talk to a vendor, open the store. A second client joins the same hub and both see each other
   (the graph drive is still the AGPL's and runs on the MIT's driver).
4. **A hunt.** Start one from the airship, play it, finish, return. The hunt server's log shows the same lines. Leave
   it empty: it ends itself about 50 s later (the AGPL's watchdog, started by patch 0052).
5. **A realm host** (player host, R10 not landed). Host a realm hub from the launcher and join from a second client;
   its log shows `player host without R10 in this build: the AGPL runtime listens (patch 0052)` and the AGPL's own
   `[Networking::Listen]` lines with `engine MULTIHOME matches the pinned listen address 127.0.0.1`; the launcher sees
   it ready and can fence it.
6. **Regression without R9.** An MIT DLL without `"R9"`: the AGPL's own listen lines appear, no `[MitRuntime] [Listen]`
   lines, and everything behaves as before.
7. Note anything that differs from the same steps on the previous release. Only CI passed until these steps are done.

---

## 14. Size

Rust, without tests: the core crate about 300 lines (decision, schedule, plans and checks 150, ready texts 40, channel
lookups 70, log texts 40); the DLL crate about 550 (SRV-1 detour and write 90, the listen 250, readiness and stdout 70,
maintenance 40, state, R10 slot and export 60, install 40). Tests about 400. Patch 0052: about 50 lines of C++.
