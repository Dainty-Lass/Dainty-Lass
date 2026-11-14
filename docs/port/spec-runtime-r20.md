<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R20, server ProcessEvent behaviours (RD27, plan item 11)

R20 moves the server's behaviour handlers on the dispatch points into the MIT runtime: the feature ids **`SP-2@R20`**
(post-login match start, the server `ClientRestart` pawn fixes, the vendor camera fix, the `PlayerCanRestart` force,
the debug damage multiplier) and **`SP-3@R20`** (the manual stamina tick, the bleedout duration floor, and the retired
server mesh pose experiment). These are the handlers that let a first player spawn at once on a server without a
phantom local player, let a player's own pawn reach them, let vendors open on a server with no viewport, and keep
stamina and bleedout working on a dedicated server (`spec-runtime-surface.md` sections 4.1, 4.2 and 8, R20 row;
`spec-runtime-r0.md` section 7.5).

An implementer builds from this page, `spec-runtime-surface.md`, `spec-runtime-r0.md`, `spec-runtime-r1.md`,
`spec-runtime-r5.md` section 2.2, `spec-runtime-r7.md` sections 3.3 and 7, `spec-runtime-r9.md` section 8,
`spec-runtime-r12.md` sections 5 and 15 and the reflection kit of `spec-runtime-r17.md` section 3, and never reads
`restricted/`. What crossed is facts: RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7, CL 647472), engine and game
class, function and member names and offsets from the maintainer's Dumper-7 dump of this build, parameter block
layouts, flag and variable names, and behaviour. The AGPL side (seam patch 0041, patches 0054 and 0059) is described
only by the contract it keeps (section 12).

Conventions are R0's: an RVA is `+0x0XXXXXXX` from the executable's image base; **S** server. R20 installs nothing on
a client. **(call)** marks a decision made here, **(departure)** a deliberate difference from the AGPL runtime,
**(ambiguity)** something the source did not settle; all are collected in section 14.

---

## 1. Scope

| R20 owns | Id | Section |
| --- | --- | --- |
| Post-login: start a match still waiting, at the first login, on a server without a phantom local player | `SP-2@R20` | 5 |
| The server `ClientRestart` pawn fixes: replication flags and the owner's per-connection relevancy | `SP-2@R20` | 6 |
| The vendor camera fix: absorb `UpdateCameraForAspect` on the server | `SP-2@R20` | 7 |
| `PlayerCanRestart`: the connection fix-ups, R7's restart data gate, the force to true | `SP-2@R20` | 8 |
| The debug damage multiplier (development builds only) | `SP-2@R20` | 9 |
| The manual stamina tick per connected player and the bleedout duration floor | `SP-3@R20` | 10 |
| The server mesh pose experiment: owned, **not ported** | `SP-3@R20` | 10.4 |
| RT-14, RT-17, RT-18 for R20 | | 11, 14 |

Not R20, though the surface's section 4.2 lists it near these handlers:

- **The post-login AFK write** moved to `SP-2@R8` (`spec-runtime-r8.md` 10.3). R8's own handler does it; R20 does not.
- **The restart data gate** is R7's (`SP-2@R7`); R20 calls it (8.2 step 5).
- **The possession traces** (`AcknowledgePossession`, `AcknowledgePawn`, `CheckClientPossession`), **must-spectate**
  (`GameModeBase.MustSpectate`), `ClientWasKicked`, `ClientReturnToMainMenu`, the XP, economy, Behemoth and Tempest
  traces: all are logging only. RT-3: not ported. The surface's "must-spectate handling" is only a log line in the
  AGPL; nothing about must-spectate changes behaviour, so R20 has nothing to port for it.
- **The per-login capture reset and the login stamp** the AGPL kept beside the match start feed only its diagnostics
  (a per-login `ClientRestart` trace budget and the server arrival-trigger probe). RT-3: not ported (14, departures).
- Escalation round transitions (`SP-2@R18`), the player host's observer (`SP-2@R10`), the input and role work on the
  client (`SP-1@R14`), the player pawn update-frequency boost in the same tick block (`SP-3@R13`).

### 1.1 Where the code goes

| File | Holds |
| --- | --- |
| `crates/rustic-isles-runtime-core/src/server_events.rs` (new) | everything pure: the connection choice (8.2 step 1), the outer walk's stop rule (8.2 step 2), the pawn write plan (6.2), the bleedout floor decision and the fallback parse (10.2), the multiplier settings and the damage field table (9), the vendor log throttle (7), the stamina skip decision (10.3), the log texts (13) |
| `crates/rustic-isles-runtime/src/ffi/server_events.rs` (new) | the six SP-2 handlers and the SP-3 handler, every game read, write and call |
| `crates/rustic-isles-runtime-core/src/offsets.rs` | the R20 lines of section 4 (`pub mod r20 { ... }`), with a comment naming R20; no `TARGETS` line (R20 hooks nothing) |
| `crates/rustic-isles-runtime-core/src/registry.rs` | `LANDED` gains `"R20"`. `SP-2@R20` and `SP-3@R20` are already there: R20 adds no id |
| one-line edits | `pub mod server_events;` in the core `lib.rs` and in `ffi/mod.rs`; one install entry in `src/lifecycle.rs` (3.3) |

If `ffi/server_events.rs` passes about 600 lines, split it into `ffi/server_events/{login,restart,tick}.rs` with a
`mod.rs`; nothing else changes.

**Pure and Windows-only.** The core module holds every decision over values already read and is tested on every
platform. The DLL module only reads and writes game memory, calls game functions and logs; on a non-Windows target it
is `#[cfg(windows)]` and absent, as R0 sets up.

---

## 2. Ownership and what R20 consumes

With `"R20"` in `LANDED`, `rir_owns` answers 1 for `SP-2@R20` and `SP-3@R20`; the status line's count grows by 2.

### 2.1 Interfaces R20 consumes

| Value | From | Use |
| --- | --- | --- |
| `config::is_server()`, `config::auth_mode()`, `config::launch()` → `Launch::watchdog` | R0 3.2, R1 5 and 6.1 | role; the floor's "full launch" condition (10.2) |
| `handlers::register_pe`, `register_tick` (`Point::Sp2`, `Point::Sp3`, `PeHandler`, `TickHandler`, `Verdict`) | R0 8 | every handler |
| `ffi::memory::readable`, `read`, `readable_this_frame`; `ffi::engine::is_live_object`, `full_name`, the `FName` to string | R0 5, 6 | every read; liveness; the match state's name |
| `ffi::flags::flag`, `development_profile()`, `production_log_profile()`; `env_value` | R0 4 | R20's flags and variables (11) |
| `ffi::log::line`, `ffi::detaching()` | R0 3.3, 4.3 | logging, the detach rule |
| `ffi::broker::chained_by_agpl(target) -> bool` | R0 7.2 (added here if absent, 10.3) | whether the AGPL chained a wrapper on DX-8's target |
| `reflect::is_a`, `find_class`, `find_function`, `params_fit`, `ffi::reflect::call`, `write` | R17 3 | class checks, the three UFunction calls, every write |
| `headless::boot_patch_applied()`, or R7's `no_phantom()` until R5 lands | R5 2.2, R7 3.3 | the match start's precondition (5) |
| `loading::restart_gate::hold(controller) -> bool` | R7 7.4 | the restart data gate (8.2 step 5) |
| `listen::net_driver()`, `listen::client_connections(driver)` | R9 8.1, 8.3 | the stamina tick's connections (10.1) |
| nothing called | R12 15 | R20's possession and post-login behaviours rely on R12's drive and guards running, MIT or AGPL |

`readable_this_frame(ptr, size)` is R0's frame pointer cache (5.2) as R12 uses it. `boot_patch_applied()` and
`no_phantom()` give the same answer (R5 2.2): HL-2's six bytes at `+0x03E07353` read `E9 D4 00 00 00 90`.

### 2.2 Landing order **(call)**

- **R20 lands with or after R7.** The AGPL's restart data gate sits inside its own `PlayerCanRestart` block, which
  `SP-2@R20` turns off. R7's MIT gate marks a player's data loaded only when R7's LD-2 is the MIT's; with R7 not
  landed, R20's call to `restart_gate::hold` would hold every hunt player 90 s. A registry test (15.1) fails a
  `LANDED` that holds `"R20"` without `"R7"`. R0's table already asks for this order.
- **R9 and R12 before R20** is the surface's wave order, not a code need: R20 calls R9's lookups, which answer
  `None` until R9 lands (10.1 then uses the world's driver), and calls nothing of R12's.
- **R8 may land before or after R20.** Patch 0059 (12.2) keeps the AGPL's post-login AFK write alive when R20 lands
  first.

---

## 3. Handlers and install

### 3.1 The handlers

All registered with `slice = "R20"`, on the server only.

| Handler | Point | Kind | `order` | Matcher (RT-18) | Section |
| --- | --- | --- | --- | --- | --- |
| damage multiplier | SP-2 | before | **−500** | five `Exact` names (9.2) | 9 |
| post-login | SP-2 | before | 0 | `FunctionName("K2_PostLogin")` | 5 |
| client restart | SP-2 | before | 0 | `Exact("Function Engine.PlayerController.ClientRestart")`, `Exact("Function Engine.PlayerController.ClientRetryClientRestart")` | 6 |
| vendor camera | SP-2 | before, absorbs | 0 | `Contains("vendor_interactive_bp_C.UpdateCameraForAspect")` | 7 |
| player can restart | SP-2 | before, absorbs | 0 | `Exact("Function Engine.GameModeBase.PlayerCanRestart")` | 8 |
| connected players | SP-3 | after | **650** | (tick) | 10 |

**Where they sit (call).** Every AGPL block R20 replaces ran before the original call, inside the AGPL wrapper; R0's
chain runs every MIT before handler before every AGPL block. Moving them outward changes nothing a player sees:

- The damage multiplier ran before the AGPL's other blocks so that the player host's evidence observer and the damage
  traces saw the multiplied value; an MIT before handler at −500 runs before all of them and before any later MIT
  observer with a larger order (R10 registers none today).
- The absorbing handlers (vendor, `PlayerCanRestart`) now skip the AGPL wrapper entirely for those calls. The AGPL
  blocks that ran on such a call before its own early return were its once-per-process data writes (R15, on every
  call until they take), the server audio suppression's `PostEvent` match (another name) and the logging-only traces.
  The data writes run on the next call; the rest is logging. Nothing lost.
- The SP-3 handler at **650**: after R8's after handler (600) and R12's drive (500), in the AGPL's order (its
  connected-server block ran the watchdog, then the floor, then stamina, after the graph drive). R13's pawn boost
  belongs after it if R13 keeps the AGPL's order (it ran after the stamina call in the same loop).

### 3.2 Common rules

- If `ffi::detaching()`: every handler returns `Continue` at once.
- A read that fails ends that handler's work for the call; a before handler then returns `Continue`, so the game's
  own function runs. No handler faults: every pointer is checked with R0's `readable` before use, every write goes
  through R17's checked `write`.
- Every handler runs on the game thread (SP-2 and SP-3 are the game thread's calls).
- **No fault catch (departure).** The AGPL wrapped the match start's call and the world package name setter in an
  exception catch. R20 checks liveness and class instead (5, 8.2 step 3), as R12 decided for its drive (R12 11.4).

### 3.3 Install (RT-17 for R20)

In Init, after R0's dispatch chains and any earlier slice's entries, before `handlers::freeze()`: when
`config::is_server()` and `registry::owns("SP-2@R20")`, register the six SP-2 handlers; when
`registry::owns("SP-3@R20")`, the SP-3 handler. R20 makes no hook and patches no byte, so **no install order
dependency** beyond the freeze. R7's `PlayerCanRestart` after handler is not registered once `SP-2@R20` is owned
(R7 7.3), so the gate runs once, inside R20's handler.

---

## 4. Facts

### 4.1 Functions R20 calls

| Function | Found by | Parameters | Used by |
| --- | --- | --- | --- |
| `AGameMode::StartMatch` | UFunction `StartMatch` on class `GameMode` (R17 `find_function`) | none (`ParmsSize` 0) | 5 |
| `BP_PlayerCharacter_C::TickStamina` | UFunction `TickStamina` on the pawn's class | `0x02`: `Where` `u8` `+0x00` (`ECityExecFilter`), `Whom` `u8` `+0x01` (`ERemoteExecFilter`) | 10.1 |
| `UNetConnection::SetClientWorldPackageName` | RVA **`+0x03F7F050`** | `(connection, name: FName by value as u64)`; stores the name at `connection + 0x1600` and notifies the connection's actor channels | 8.2 step 3 |

UFunction calls go through R17's `call`, which enters the hooked `ProcessEvent` (`+0x02861880`): the whole SP-2 chain
sees them, as the AGPL's calls through the object's virtual `ProcessEvent` did. R20's matchers never match
`StartMatch` or `TickStamina` (15.1 tests it).

Enum values R20 passes: `ECityExecFilter::Both = 2`; `ERemoteExecFilter::All = 5`.

### 4.2 Offset-table lines (`pub mod r20`, RT-2)

`data`:

| Name | Value | What |
| --- | --- | --- |
| `SET_CLIENT_WORLD_PACKAGE_NAME` | `0x03F7_F050` | `UNetConnection::SetClientWorldPackageName` |
| `TICK_FILTER_HELPER` | `0x0194_80B0` | `UArchonGameplayStatics::TickFilterHelper`, DX-8's target: R20 only asks the broker about it (10.3) |

`layout` (UE 4.26 and Archon classes as built into 1.14.7):

| Name | Value | What |
| --- | --- | --- |
| `GAME_MODE_MATCH_STATE` | `0x2C0` | `AGameMode::MatchState`, an `FName` (readable span `0x2C8`) |
| `ISLAND_BLEEDOUT_DURATION` | `0x540` | `AArchonGameMode_Island::BleedoutDuration`, `f32` (readable span `0x548`). **Only on the island game mode:** the same offset on another game mode is an unrelated field |
| `WORLD_AUTHORITY_GAME_MODE` | `0x118` | `UWorld::AuthorityGameMode`; **shared with R8** (R8 2.1): whichever lands first adds the line |
| `PC_PLAYER` | `0x298` | `APlayerController::Player` (`UPlayer*`: a `UNetConnection` or a local player) |
| `PC_NET_CONNECTION` | `0x418` | `APlayerController::NetConnection` (readable span `0x420`); R12's line, shared |
| `CONTROLLER_PAWN` | `0x250` | `AController::Pawn` (readable span `0x258`) |
| `CONN_PLAYER_CONTROLLER` | `0x30` | `UPlayer::PlayerController` on a connection; R12's, shared |
| `CONN_VIEW_TARGET` | `0x90` | `UNetConnection::ViewTarget` (`AActor*`) |
| `CONN_OWNING_ACTOR` | `0x98` | `UNetConnection::OwningActor`; R12's, shared (connection readable span `0xA0`) |
| `CONN_CLIENT_WORLD_PACKAGE_NAME` | `0x1600` | `FName` (readable span `0x1608`) |
| `CONN_SCAN_SPAN` | `0x140` | the span the stamina loop checks per connection |
| `ACTOR_RELEVANCY_FLAGS` | `0x58` | `u8`; bit `0x08` is `bAlwaysRelevant` |
| `ACTOR_REMOTE_ROLE` | `0x5F` | `u8` `ENetRole` (`2` = `ROLE_AutonomousProxy`) |
| `ACTOR_NET_DORMANCY` | `0xF1` | `u8` `ENetDormancy` (`0` = `DORM_Never`) |
| `ACTOR_NET_UPDATE_FREQUENCY` | `0x108` | `f32` |
| `ACTOR_NET_PRIORITY` | `0x110` | `f32` |
| `PAWN_SCAN_SPAN` | `0x120` | the span the pawn fixes check |

`UOBJECT_CLASS` (`+0x10`), `UOBJECT_NAME` (`+0x18`, comparison index and number, 8 bytes) and `UOBJECT_OUTER`
(`+0x20`) are R0's.

### 4.3 Parameter blocks

| Function | Size | Fields |
| --- | --- | --- |
| `GameModeBase.K2_PostLogin` | `0x08` | `NewPlayer` pointer `+0x00` |
| `PlayerController.ClientRestart`, `ClientRetryClientRestart` | `0x08` | `NewPawn` pointer `+0x00` |
| `GameModeBase.PlayerCanRestart` | `0x10` | `Player` pointer `+0x00`; `ReturnValue` `bool` `+0x08` |
| `ArchonBehemoth.ReceiveArchonDamage` | `0x48` | `Damage` `f32` `+0x00` |
| `ArchonBehemoth.ReceiveArchonPointDamage` | `0x78` | `Damage` `f32` `+0x00` |
| `ArchonBehemoth.ReceiveArchonRadialDamage` | `0x60` | `DamageReceived` `f32` `+0x00` |
| `ArchonBehemoth.ReceivedArchonAbilityDamage` | `0x100` | `DamageReceived` `f32` `+0x00` |
| `ArchonBehemoth.ReceivedArchonAbilityDamageOnMonsterPart` | `0x108` | `DamageReceived` `f32` `+0x08` |
| `BP_PlayerCharacter_C.TickStamina` | `0x02` | 4.1 |

---

## 5. Post-login: the match start (S)

**Why.** HL-2 (R5) removes the phantom local player a dedicated server used to create at boot. That phantom's own
login used to start the match, so a real player arriving later found it `InProgress` and spawned at once. Without it,
a server sits in `WaitingToStart` until the first real login; that login then misses its spawn (the game mode's
`HandleStartingNewPlayer` saw `WaitingToStart`) and the player waits for the game mode's 60 s retry. Measured on
1.14.7: 61 s from post-login to possession on the Training Grounds and on a hunt, 1.2 s on Ramsgate. Starting the
match at boot instead is wrong: the Training Grounds end a match that starts with no player in it
(`WaitingToStart` → `WaitingPostMatch`). So the match is started at the first login, before `HandleStartingNewPlayer`
runs; `K2_PostLogin` is called right before it, and the player count is already 1 there.

The handler, on each matching call (`object` is the game mode, the call's params hold the new player):

1. The boot patch took (`boot_patch_applied()`, 2.1). Otherwise `Continue` (a server with a phantom starts its match
   as it always did).
2. `object` is non-null, live, readable for `0x2C8`, and `is_a(object, "GameMode")` **(call: the AGPL did not check
   the class; every 1.14.7 game mode that reaches here is an `AGameMode`, and the check keeps `+0x2C0` from being read
   on a `GameModeBase`)**.
3. The `FName` at `object + 0x2C0`, as a string, equals `WaitingToStart`.
4. `object` is not the game mode R20 last started (one slot, process lifetime: once per game mode).
5. Resolve `StartMatch` once per process: `find_function(find_class("GameMode"), "StartMatch")`; log
   `[NoLocalPlayer] GameMode.StartMatch resolved` or `[NoLocalPlayer] GameMode.StartMatch not found; the match starts
   at the game mode's retry`. Not found: `Continue` (and never try again).
6. Record `object` as started, then `call(object, StartMatch, zeroed 16-byte buffer)`; log
   `[NoLocalPlayer] StartMatch at the first login: called gm=<ptr> class=<short class name>` or, when the call's
   preconditions failed, `... skipped gm=<ptr>`.
7. Unless `production_log_profile()`: `[PostLogin] gm=<ptr> newPlayer=<ptr>` (new player from params `+0x00` when the
   params are readable for 8; else `null`).
8. `Continue`: the event runs, and then the game's `HandleStartingNewPlayer` finds the match in progress.

The matcher `FunctionName("K2_PostLogin")` catches `Function Engine.GameModeBase.K2_PostLogin` and any Blueprint
game mode's implementation of the event (the dump has one, on `BPGM_Archon_Prototype_C`); the AGPL matched the
substring `K2_PostLogin` of the full name, which catches the same functions on 1.14.7 **(call)**. The AGPL's wider
`PostLogin` substring (also `ClientOnPostLogin`, `PlayerPostLogin`) fed only its trace (RT-3).

R8's post-login AFK write (`SP-2@R8`) is a separate handler of R8's; when both are MIT, their relative order does not
matter (the AFK write reads the world's game state, which the match start does not change).

---

## 6. The server `ClientRestart` pawn fixes (S)

### 6.1 Why

When the server restarts a player it calls `ClientRestart(pawn)` on the controller (and `ClientRetryClientRestart`
until the client acknowledges the possession). Two things kept 1.14.7's possession from completing:

- The pawn reached the client as a simulated proxy, so the client never sent its movement (the engine sends moves only
  from an autonomous proxy) and the server never moved it. The pawn's remote role is set to autonomous proxy before
  the restart is sent, so the first replication of the pawn to its owner carries it.
- During loading the owner's spatial grid cell is not resolved yet, so the replication graph never gathers the
  player's own pawn for its owner, its channel never opens, the pawn's network id never resolves on the client, and
  the loading screen stays up. Pointing the owning connection's view target at the pawn makes the graph's
  per-connection always-relevant gather include it, for that connection only.

### 6.2 The handler (`Continue` always)

1. The params are readable for 8; `pawn` = params `+0x00`. `controller` = the call's object.
2. **The pawn's flags**, when `pawn` is non-null, live and readable for `0x120`. The write plan (pure, over the values
   read):
   - `+0x58` bit `0x08` (`bAlwaysRelevant`) is set **only when the owner fix (step 3) is off**. With the fix on, the
     pawn stays purely spatial for everyone else: marking it globally always-relevant as well left it in both the
     spatial grid and the always-relevant set, which the grid rebuild faults on, and hid armour in co-op.
   - `+0xF1` (`NetDormancy`) = `0` (never dormant).
   - `+0x108` (`NetUpdateFrequency`) = 60.0 when it is below 60.0.
   - `+0x110` (`NetPriority`) = 3.0 when it is below 3.0.
   - `+0x5F` (`RemoteRole`) = `2` (autonomous proxy).

   Every call, idempotent: the values stay set, and a reset by the game is undone on the next retry.
3. **The owner fix**, unless `DISABLE_OWNER_PAWN_RELEVANCY.flag` (read once, at the first call): `pawn` non-null;
   `controller` readable for `0x420`; `connection` = `controller + 0x418`, non-null, readable for `0x98`. When
   `connection + 0x90` (`ViewTarget`) is not `pawn`, write `pawn` there and, for the first 8 such writes in the process,
   log `[OwnerPawnRelevancy] conn=<ptr> ViewTarget <old> -> pawn <ptr>`.

Re-asserted on each `ClientRestart` and retry, so it holds through the possession handshake. `PlayerCanRestart`'s view
target write (8.2 step 4) runs earlier in the same restart and points the field at the controller's pawn or the
controller; this step then points it at the pawn the client must resolve.

---

## 7. The vendor camera fix (S)

A vendor's interaction Blueprint (`vendor_interactive_bp_C`) calls `UpdateCameraForAspect` when its interaction
completes, which reads the screen's aspect ratio. A server runs with no viewport: the ratio is 0, the Blueprint divides
by zero (the log shows `Divide by zero detected: 0.000000 / 0` under `OnUserCompletedInteraction` →
`UpdateCameraForAspect` → `GetCurrentScreenAspectRatio`) and the interaction never completes for the player. The
owning client runs its own copy with its own viewport, so the server's call has no purpose.

The handler **absorbs** every matching call on the server (the original is not called). The `n`-th absorb in the
process (from 0) logs `[VendorFix] absorbed UpdateCameraForAspect on the server (slot=<n>)` when `n < 3` or
`n % 60 == 0`.

The matcher is the AGPL's substring, `Contains("vendor_interactive_bp_C.UpdateCameraForAspect")`: it catches that
class's own function only (a subclass's override would carry its own class name). The dump has the one.

---

## 8. `PlayerCanRestart` (S)

### 8.1 Why

The engine's `GameModeBase::PlayerCanRestart` refuses a player whose connection has not reported the world loaded.
On 1.14.7 the server never marks a remote connection's client world package, and the default answer kept players from
ever spawning. The AGPL sets the package name properly through the engine's own setter, repairs the connection's view
target, then answers true for every controller (it never calls the original), except a hunt player whose data has not
loaded (R7's gate).

### 8.2 The handler (before, absorbs whenever it acts)

If the params are not readable for `0x10`: `Continue` (the original answers). Otherwise, `player` = params `+0x00`,
`prior` = the `ReturnValue` byte at `+0x08`, and in order:

1. **The connection** (pure choice over values read). When `player` is non-null and readable for `0x420`:
   `from_418` = `player + 0x418` (`NetConnection`), `from_298` = `player + 0x298` (`Player`). Choose `from_418` when
   non-null (the engine sets it only for a real connection). Else choose `from_298` when it is non-null, its class is
   readable, and its class's short name contains `Connection` and does not contain `local_player` (the engine's own
   cast of `Player` to a connection; a listen server's local player is a `bp_archon_local_player_C`). Else none.
2. **The world's package name.** Walk the outer chain from `player`: `top` = `player`; at most 8 steps, stop when the
   current object is not readable for `0x28` or its outer (`+0x20`) is null; otherwise `top` = that outer. If `top` is
   readable for `0x20`, `name` = the 8 bytes at `top + 0x18` (comparison index and number). A controller's chain is
   level, world, package (three steps), so `top` is the map's package.
3. **Set it.** When a connection was chosen, it is live, `is_a(connection, "NetConnection")`, readable for `0x1608`,
   and `name` is not all zero: call `SetClientWorldPackageName(connection, name)` at `+0x03F7F050` **(call: the AGPL
   checked readability only and caught a fault; liveness and class replace the catch, 3.2)**.
4. **The view target.** When a connection was chosen and it is readable for `0xA0`: `owning` = `connection + 0x98`;
   `view` = `owning`; when `owning` is readable for `0x258` and its `Pawn` (`+0x250`) is non-null, `view` = that pawn.
   Write `view` to `connection + 0x90`. This runs on every call: the AGPL once did it only within a five-call log
   budget, and a long-running Ramsgate stopped repairing view targets from its sixth restart on.
5. **R7's gate.** `loading::restart_gate::hold(player)` true: write `0` to the `ReturnValue` byte and **absorb** (the
   original is not called). R7 7.1 decides when it applies (a remote player in a hunt on a `GlobalGameserver` whose
   data has not loaded, up to 90 s) and logs its own lines.
6. **The force.** Write `1` to the `ReturnValue` byte. The first time in the process, log
   `[PlayerCanRestart] forced true: player=<ptr> priorRet=<0|1>`. **Absorb.**

Steps 1 to 4 run before the gate, as in the AGPL: a held call still gets its connection fix-ups.

`DIAG_NATURAL.flag` (which let the original answer) is dropped for every MIT slice (R12 13); R20 does not read it.

---

## 9. The debug damage multiplier (S, development builds only)

### 9.1 When it is on

All of, decided once, at the handler's first call, and kept: `development_profile()` (R0 4.1:
`RUSTIC_ISLES_RUNTIME_PROFILE` is exactly `development`); `RUSTIC_ISLES_ALLOW_DAMAGE_DEBUG` is exactly `1` (the
launcher never sets it; `env_value` max 7); `DEBUG_DAMAGE_MULTIPLIER.flag` present. Then the factor is
`RUSTIC_ISLES_DEBUG_DAMAGE_MULTIPLIER` (`env_value` max 63) parsed as an `f32` **(call: the whole trimmed value must
parse; the AGPL took a leading number and ignored the rest)**; unset, unparsable, not finite or not above 0 gives
1.0. A factor of exactly 1.0 means off. Off: the handler returns `Continue` at once, for the process.

It scales the damage a Behemoth receives before its Blueprint handles it, so the scaled value is what leaves its
health. Health is the server's, so this only ever works on the server; it is for local repro runs (killing a Behemoth
fast), never production, which is why four switches guard it.

### 9.2 Each call

The matchers are `Exact` on the five full names `Function Archon.ArchonBehemoth.<name>` of 4.3. **(call)** The AGPL
matched substrings `ArchonBehemoth.<name>` with `ReceivedArchonAbilityDamage` excluding names containing
`OnMonsterPart`; on 1.14.7 those substrings catch exactly these five full names, because a Blueprint subclass's
function carries its own class name. When the params are readable for the block's size (4.3): `before` = the `f32` at
the damage field's offset; write `before × factor`; log
`[DebugDamageMultiplier] fn=<full name> obj=<object name> before=<f> after=<f> multiplier=<f>` (each `f` with six
decimals). `Continue`.

---

## 10. `SP-3@R20`: connected players (S)

One after handler, `order = 650`, each frame:

### 10.1 Which driver and which players

- `driver` = R9's `listen::net_driver()`. When that is `None` (R9 not landed, or a player host the AGPL listens for,
  R9 6.5), the world's net driver: `GWorld` readable for `0x40`, `UWorld::NetDriver` (`+0x38`, R9's
  `WORLD_NET_DRIVER`) non-null and live. Still none: nothing this frame (the AGPL's block also ran only once listening
  with a driver).
- `connections` = R9's `client_connections(driver)` (sane array, null entries out; empty when not sane).
- A player, per connection: `readable_this_frame(connection, 0x140)`; `controller` = `connection + 0x30`, non-null and
  readable for `0x258`; `pawn` = `controller + 0x250`, non-null, live, `is_a(pawn, "BP_PlayerCharacter_C")`.

### 10.2 The bleedout floor (once per process)

**Why.** `UArchonBleedoutComponent::TryStartBleedout` copies the island game mode's `BleedoutDuration` into the
component's current duration, and the Blueprint feeds that to `SetTimer`. On our island game modes the designer value
arrives as 0 (the log shows `SetTimer passed a negative or zero time`), so the timer is never created, the bleedout
never elapses, and a downed player sits at 0 health forever: no damage taken, never dies, can still attack. Why the
default is not applied is not known; the floor is a runtime patch, not a root fix.

Before the stamina loop, until done, when `config::launch()` is a full launch (`Launch::watchdog`, R1 6.1; the AGPL
applied the floor only on a launch with its watchdog) and a driver was found (10.1):

1. `gm` = `GWorld + 0x118` (`AuthorityGameMode`), non-null, live, readable for `0x548`. Otherwise try next frame.
2. Not `is_a(gm, "ArchonGameMode_Island")`: done, silently (Ramsgate's `BPGM_City_C` is not an island mode, and
   `+0x540` there is another field; never relax this to the `ArchonGameMode` base).
3. `d` = the `f32` at `gm + 0x540`. `d > 0`: log `[Bleedout] BleedoutDuration=<d>s on the island game mode; no floor
   needed`; done.
4. Else write the fallback and log `[Bleedout] BleedoutDuration was <d>; set to <fallback>s so a downed player's
   timer starts`; done.

The fallback, read once: `RUSTIC_ISLES_BLEEDOUT_SECONDS` (`env_value` max 31) parsed as an `f32` (whole trimmed value,
as 9.1) when `0 < v < 600`; otherwise **30.0**.

**(departure)** The AGPL's two paths found the game mode differently (the first `ArchonGameMode` in the object array,
or the world's); R20 uses the world's authority game mode only, as R8 does. On a server that serves one map they are
the same object.

### 10.3 The manual stamina tick

**Why.** The character's own tick calls `TickStamina` behind a Blueprint execution filter whose answer on a dedicated
server leaves the call out, so remote players' stamina never drains or refills on the server. The server calls it
itself once per frame for every connected player's character.

For each player of 10.1: `call(pawn, TickStamina, {Where = 2 (Both), Whom = 5 (All)})`, `TickStamina` found on the
pawn's class (`find_function`) and cached per class pointer.

**The tick filter (call).** The AGPL has an opt-in, logging-and-fix hook on the filter itself (DX-8, R19, never
ported: RT-3) behind `SERVER_TICK_FILTER.flag`. When that hook answers for the character, the character's own tick
runs `TickStamina`, and a manual call as well would evaluate stamina twice per frame; the AGPL then skipped its manual
call. R20 skips the manual call when **both**: `SERVER_TICK_FILTER.flag` was present at the handler's first frame (read
once), and `broker::chained_by_agpl(+0x019480B0)` is true (the AGPL actually put its wrapper on DX-8's target through
R0's broker, R0 7.4; it does so only on a `GlobalGameserver` with the boot patch applied and a matching prologue).
The skip decision is made once, at the first frame that has a player, and logged once:
`[Stamina] manual TickStamina skipped: the AGPL's tick filter hook (SERVER_TICK_FILTER.flag) runs it in the character's
tick` or `[Stamina] manual TickStamina each frame for connected players`.

`chained_by_agpl(target)` is a read of R0's chain model (true when band `Agpl` holds a wrapper on `target`). If R0's
broker does not have it yet, R20 adds it (a pure query in the core's chain model and a locked accessor in
`ffi::broker`), touching nothing else in R0's files.

### 10.4 The server mesh pose experiment: not ported **(call)**

`SP-3@R20` also gated the AGPL's `FORCE_SERVER_MESH_POSE` pass. It is a retired diagnostic: with that flag alone it
only logs that it is retired, and with `FORCE_SERVER_MESH_POSE_UNSAFE.flag` as well it walks every object every 4 s
forcing each skeletal mesh's `VisibilityBasedAnimTickOption` (`+0x5FC`) to `0` (always tick pose and refresh bones),
an experiment that was never shown to fix anything and costs a full object walk. R20 owns the id, so the AGPL pass is
off, and ports nothing; both flags are dropped (11).

---

## 11. Flags and variables (RT-14 for R20)

| Flag | Decision |
| --- | --- |
| `DISABLE_OWNER_PAWN_RELEVANCY` | **kept, moved from R13 (call):** its only reader is the `ClientRestart` block, which is `SP-2@R20`. Read once. Present: no view target write at `ClientRestart`, and the pawn is marked globally always-relevant instead (6.2) |
| `DEBUG_DAMAGE_MULTIPLIER` | kept, development builds only (9.1). Read once |
| `SERVER_TICK_FILTER` | not R20's (R19, DX-8); read once to decide the stamina skip (10.3) |
| `FORCE_SERVER_MESH_POSE`, `FORCE_SERVER_MESH_POSE_UNSAFE` | **dropped** (10.4) |
| `DISABLE_PLAYER_HUNTID_BACKFILL` | moved to R7 (R7 10) |
| `DIAG_NATURAL` | dropped by R12 (R12 13); not read |

Variables, all read through R0's `env_value`: `RUSTIC_ISLES_BLEEDOUT_SECONDS` (10.2), `RUSTIC_ISLES_ALLOW_DAMAGE_DEBUG`,
`RUSTIC_ISLES_DEBUG_DAMAGE_MULTIPLIER` (9.1), and R0's `RUSTIC_ISLES_RUNTIME_PROFILE`. None is a `settings.toml` setting
today and R20 makes none: the bleedout floor has never needed a different value, and the debug pair must stay out of
host configuration. **(call)** If a host ever needs another bleedout length it becomes an RT-11 setting then.

---

## 12. The AGPL side

### 12.1 What seam patch 0041 already does

With `SP-2@R20` owned, the AGPL's server `ProcessEvent` skips: the debug damage multiplier, its post-login block (the
trace, the match start, the login stamp, the capture reset; and the AFK write nested inside it, see 12.2), the
`ClientRestart` pawn block, the vendor absorb and the whole `PlayerCanRestart` block (fix-ups, its own call of R7's
gate, the force). With `SP-3@R20` owned, its tick skips the mesh pose pass. Its logging-only possession and
must-spectate traces are not gated and keep running.

Patch 0054 already gates, on `SP-3@R20`, the AGPL's manual `TickStamina` call (with its tick-filter switch) and the
bleedout floor in its connected-server block when `SP-3@R8` is owned.

### 12.2 Runtime patch 0059 (written with this page)

`restricted/modules/rustic-isles/native/runtime/dllmain.cpp`, marked `[DaintyLass 0059]`; row 0059 in
`restricted/docs/paradox-patches.md`. Contract:

- **The post-login AFK write keeps running when R20 lands before R8.** The AGPL nested it, gated on `SP-2@R8`, inside
  its `SP-2@R20` post-login block; 0041 would turn it off with R20. With `SP-2@R20` owned and `SP-2@R8` not, the AGPL
  writes the AFK timeout at `K2_PostLogin` on its own, as before. The MIT's before handlers have run by then, so the
  order (match start, then AFK) is unchanged. Without that write a client would get the game state's AFK timeout only
  from the once-a-second tick, which may come after the connection's first game state bunch (the value replicates
  only initially).
- **The bleedout floor has one owner.** The AGPL's other floor call, in its watchdog's game-mode sample (which runs
  when `SP-3@R8` is not owned), is gated on `SP-3@R20` too, so the floor is the MIT's alone once R20 lands, whichever
  of R8 and R20 lands first.
- No seam ABI change: seam version stays 1. **With the MIT DLL absent**, or R20 not landed, the AGPL behaves exactly
  as before.

### 12.3 Cross-slice state (R0 7.5's table: rows added)

| State | Set by | Read by | Rule |
| --- | --- | --- | --- |
| "the boot patch took" | HL-2 (R5, or the AGPL) | R20's match start | carried: the bytes (2.1) |
| the restart data gate | R7 (`SP-2@R7`) | R20's `PlayerCanRestart` | **R20 lands with or after R7** (2.2); R20 calls `restart_gate::hold` |
| the AFK values on the game state | `SP-2@R8` (MIT or AGPL) | the client | carried: patch 0059 keeps the AGPL's write alive while R8 is not landed |
| the login stamp, the per-login `ClientRestart` trace budget | the AGPL's post-login block | the AGPL's server arrival-trigger probe and pawn trace | **lost on purpose** (logging only, RT-3): those probes go quiet once R20 lands |
| the first-login time | the AGPL's ungated post-login anchor | its expected-player census | unchanged: not R20's, still the AGPL's |
| the tick filter hook | DX-8 (AGPL only) | R20's stamina skip | carried: R20 asks the broker (10.3) |
| the bleedout floor | `SP-3@R20` | the game | carried: patches 0054 and 0059 gate both AGPL calls |

---

## 13. Logging

Every line through `ffi::log::line`. Production-quiet (skipped under `production_log_profile()`): the `[PostLogin]`
line. Everything else is once per process or budgeted (sections 5 to 10). The texts are R20's own; nothing parses
them. All of them, for the tests:

- `[NoLocalPlayer] GameMode.StartMatch resolved`
- `[NoLocalPlayer] GameMode.StartMatch not found; the match starts at the game mode's retry`
- `[NoLocalPlayer] StartMatch at the first login: called gm=<ptr> class=<name>`, `... skipped gm=<ptr>`
- `[PostLogin] gm=<ptr> newPlayer=<ptr>`
- `[OwnerPawnRelevancy] conn=<ptr> ViewTarget <ptr> -> pawn <ptr>`
- `[VendorFix] absorbed UpdateCameraForAspect on the server (slot=<n>)`
- `[PlayerCanRestart] forced true: player=<ptr> priorRet=<0|1>`
- `[DebugDamageMultiplier] fn=<name> obj=<name> before=<f> after=<f> multiplier=<f>`
- `[Bleedout] BleedoutDuration=<d>s on the island game mode; no floor needed`
- `[Bleedout] BleedoutDuration was <d>; set to <f>s so a downed player's timer starts`
- `[Stamina] manual TickStamina skipped: the AGPL's tick filter hook (SERVER_TICK_FILTER.flag) runs it in the
  character's tick`, `[Stamina] manual TickStamina each frame for connected players`

`<ptr>` is R0's pointer format.

---

## 14. Calls, departures, ambiguities and settled questions

Calls:

1. R20 lands with or after R7 (a registry test enforces it); R9 and R12 first is wave order, not a code need; R8 may
   land either side thanks to patch 0059 (2.2).
2. All six SP-2 handlers are before handlers; the multiplier at −500, the rest at 0; the SP-3 handler after, at 650
   (3.1).
3. The match start checks the game mode's class before reading its match state (5 step 2).
4. `K2_PostLogin` is matched by function name, the multiplier's five names by `Exact`; both equal the AGPL's substring
   matches on 1.14.7 (5, 9.2).
5. Liveness and class checks replace the AGPL's fault catches around the two game calls (3.2, 8.2 step 3).
6. The stamina skip asks R0's broker whether the AGPL chained DX-8, and adds `chained_by_agpl` to R0's broker if
   absent (10.3).
7. The mesh pose experiment is owned and not ported; its two flags dropped (10.4).
8. `DISABLE_OWNER_PAWN_RELEVANCY` moves from R13 to R20 (11).
9. No `settings.toml` setting for the bleedout length (11).
10. Numeric variables parse the whole trimmed value (9.1, 10.2).

Departures from the AGPL:

1. The bleedout floor reads the world's authority game mode only (10.2).
2. The `[PostLoginPE]` trace with its lifecycle snapshot, the login stamp and the per-login trace budget reset are
   gone (RT-3); the `[PAWN #]` and `[CWPN #]` connection dumps around `ClientRestart` and `PlayerCanRestart` are gone
   (RT-3). R20's own lines (13) replace the ones a play-test needs.
3. No fault catch (3.2).
4. Absorbing handlers skip the AGPL wrapper for those calls (3.1); only logging and once-per-process work is skipped.

Ambiguities:

- **The view target write with no owning actor (8.2 step 4).** When the connection's `OwningActor` is null the AGPL
  wrote null to `ViewTarget`. R20 keeps that (the write is the AGPL's, and a connection reaching `PlayerCanRestart`
  has always had an owner in the logs), but nobody verified that null is harmless; if a play-test shows a view target
  cleared, skip the write when `view` is null.
- **The stamina skip's residue (10.3).** The AGPL also waited for its own class cache before skipping; R20 skips from
  the first frame with a player. The difference is at most the first few frames of the first player, and only with the
  opt-in flag.
- **The AGPL listen's world driver (10.1).** With R9 not owning the listen, R20 takes `UWorld::NetDriver`; R9 records
  that the listen sets it on the world context's world. Not observed for the AGPL's player-host listen; if a hosted
  realm shows no stamina for remote players with R20 landed and R10 not, that is the place.

Settled here:

- **RT-14 for R20:** two flags kept, two dropped, one moved in, one moved out, one only consulted (11).
- **RT-17 for R20:** no install order dependency (3.3).
- **RT-18 for R20:** `FunctionName("K2_PostLogin")`; `Exact` on `ClientRestart`, `ClientRetryClientRestart`,
  `PlayerCanRestart` and the five `ArchonBehemoth` damage events; `Contains("vendor_interactive_bp_C.
  UpdateCameraForAspect")`, which catches exactly that function (3.1, 5, 7, 9.2).
- **RT-12, RT-13, RT-15, RT-16:** not R20's (R7, R1, R1/R2 and R4 settle them).
- The surface's "must-spectate handling" and "possession" groups are logging only: nothing to port (1).

---

## 15. Tests

### 15.1 In CI

Core crate (every platform):

- **Registry:** with `"R20"` in `LANDED`, `owns` is true for `SP-2@R20` and `SP-3@R20`; a `LANDED` with `"R20"` and
  without `"R7"` fails the landing-order test (2.2).
- **Matchers:** each matcher of 3.1 against a table of full names: the five damage events match and
  `...ReceivedArchonAbilityDamageOnMonsterPart` matches only its own `Exact`; `Function Engine.GameModeBase.
  K2_PostLogin` and `Function /Game/.../BPGM_Archon_Prototype.BPGM_Archon_Prototype_C.K2_PostLogin` match, `Function
  Engine.PlayerController.ClientOnPostLogin` does not; `Function Engine.GameMode.StartMatch` and
  `Function BP_PlayerCharacter.BP_PlayerCharacter_C.TickStamina` match none of R20's handlers; the vendor `Contains`
  catches the vendor's function and not `UserWidgetFunctions_C.GetCurrentScreenAspectRatio`.
- **Connection choice:** `from_418` set wins; `from_418` null with a class `IpConnection` chooses `from_298`; with
  `bp_archon_local_player_C` or `LocalPlayer` chooses none; both null chooses none.
- **Outer walk:** a chain of three outers ends at the third; an unreadable second object stops at the first; a chain
  of ten stops after 8 steps; a null outer stops there.
- **Pawn write plan:** owner fix on leaves `+0x58` alone, off sets bit `0x08` and keeps the other bits; frequency
  59.9 → 60, 75 → 75; priority 1 → 3, 4 → 4; dormancy 1 → 0; role 1 → 2.
- **Bleedout:** the floor decision for `d` = 0, −1, NaN (treated as not above 0), 45; the fallback for unset, `45`,
  `0`, `600`, `599.5`, `45s`, ` 45 `.
- **Multiplier settings:** each of the three switches off gives off; factor unset, `abc`, `0`, `-2`, `inf`, `1`,
  `2.5`; the damage field table's offsets and sizes equal 4.3.
- **Vendor throttle:** slots 0, 1, 2, 60, 120 log; 3, 59, 61 do not.
- **Stamina skip:** flag absent or AGPL chain absent → manual; both → skipped.
- **Log texts** of 13, exact.
- The offsets test with R20's lines.

DLL crate (Windows runner):

- The handlers register on SP-2 and SP-3 with the slice, orders and matchers of 3.1, only when the role is server.
- `chained_by_agpl` against the broker's model in the test process: false for an untouched target, true after a
  test `rir_chain_add_inner`, false after its remove.

### 15.2 Never in CI

Every game read, write and call; the seam patch 0059 (it needs the host's SDK to build). The maintainer's
`restricted/` cargo tests are skipped for this row (the C++ patch needs the SDK anyway).

### 15.3 The maintainer's play-test (surface section 8, R20 row; collected for the one hands-on pass)

Surface row R20: "Vendors in Ramsgate; respawn in a hunt; join mid-hunt". Nothing waits on it.

1. Build the AGPL runtime with patches 0041, 0047, 0052, 0054 and 0059 and the MIT DLL with `"R20"` in `LANDED`
   (R7 landed too; R9 and R12 per the waves). Inject both as R0 11.1 says.
2. **Ramsgate.** In `rustic_isles_dll_port<port>.log` at the first login: `[PlayerCanRestart] forced true ...`,
   `[OwnerPawnRelevancy] conn=... ViewTarget ... -> pawn ...`; no `[NoLocalPlayer] StartMatch` line (Ramsgate's match
   is already in progress, about 1 s from login to possession as before). No AGPL `[PostLoginPE` or `[CWPN` lines.
   Walk, sprint and dodge: stamina drains and refills. Talk to two vendors and the store: each opens, and the log shows
   `[VendorFix] absorbed UpdateCameraForAspect ... (slot=0)`. No `Divide by zero` warning for `UpdateCameraForAspect`
   in the game's log.
3. **Training Grounds.** Enter from the airship. The log shows `[NoLocalPlayer] GameMode.StartMatch resolved` and
   `[NoLocalPlayer] StartMatch at the first login: called gm=... class=BPGM_ArchonTrainingGrounds_C`; the player
   spawns within a few seconds of the loading screen, not after a minute. Leave and enter again: the match stays in
   progress and no second `StartMatch` line appears for the same game mode.
4. **A hunt.** Start one solo. Same `StartMatch` line with the hunt's game mode class; spawn within seconds; the log
   shows `[Bleedout] ...` once (either "no floor needed" or "set to 30s"). Get downed by the Behemoth: the bleedout
   timer runs out or a revive works; the player never stays downed at 0 health. Die and **respawn**: the respawn works
   and the player can move (the remote role fix); stamina works after the respawn.
5. **Join mid-hunt.** A second client joins the running hunt from Ramsgate. They load in within seconds, see the
   first player and the Behemoth, can move, and their stamina drains and refills; the first player sees them move. If
   the second player's data was slow, R7's `[PlayerData] restart held ...` line appears and the spawn follows once it
   loads.
6. **R20 before R8** (if R8 has not landed in this build): set `[director] hub_afk_seconds` to a short value, log in to
   Ramsgate and idle: the AFK kick comes at that time, and the AGPL log shows its post-login AFK write for the login.
7. **Owner fix switch.** Create `DISABLE_OWNER_PAWN_RELEVANCY.flag` next to the server's executable, restart the
   server, log in: no `[OwnerPawnRelevancy]` line. Note whether the login still completes (it did on 1.12 only with
   the global flag); delete the flag.
8. **Damage multiplier** (development launch only): with `RUSTIC_ISLES_RUNTIME_PROFILE=development`,
   `RUSTIC_ISLES_ALLOW_DAMAGE_DEBUG=1`, `RUSTIC_ISLES_DEBUG_DAMAGE_MULTIPLIER=10` and `DEBUG_DAMAGE_MULTIPLIER.flag`,
   hit a Behemoth: `[DebugDamageMultiplier]` lines with `after` ten times `before`, and it dies fast. Without the flag:
   no lines, normal damage.
9. **Regression without R20.** An MIT DLL without `"R20"`: the AGPL's `[PostLoginPE`, `[CWPN` and `[VendorFix]` lines
   appear, no R20 lines, and steps 2 to 5 behave as before.
10. Note anything that differs from the same steps on the previous release. Only CI passed until these steps are done.

---

## 16. For later slices

- **R13:** `DISABLE_OWNER_PAWN_RELEVANCY` is R20's now (11); R13's relevance work must not undo the `ClientRestart`
  view target (6.2 step 3) or re-mark the pawn globally always-relevant while that flag is absent. R13's pawn boost
  (also 60 for `NetUpdateFrequency`) runs in the same frame; an after handler above 650 keeps the AGPL's order.
- **R14:** nothing of R20's is called; R14's client input and role work follows the possession R20 makes possible.
- **R8:** R20 publishes nothing R8 reads. The two post-login handlers are independent (5).

---

## 17. Size

Rust, without tests: the core module about 300 lines (connection choice and outer walk 50, pawn plan 40, bleedout and
parse 50, multiplier settings and table 50, throttle and skip 20, texts 60, offsets 30); the DLL module about 450
(post-login 80, `ClientRestart` 70, vendor 20, `PlayerCanRestart` 110, multiplier 50, tick 90, install and
`chained_by_agpl` 30). Tests about 350. Patch 0059: 8 lines of C++.
