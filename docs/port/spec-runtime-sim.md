<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime simulated players (RD27)

To see what a game server costs with players on it (the census, `spec-runtime-census.md`, and R8's `[Perf]` and
`[Memory]` lines), the maintainer needs players. This page adds server-side bots: pawns the server spawns itself, each
driven by an AI controller that wanders, so the server ticks, moves, collides and replicates their actors with no
client, no account and no network. A host ramps them up and down while the server runs by writing a file. It is new
behaviour: nothing here is ported, and nothing was read from `restricted/`, Dauntless Revived or Mystic Paradox. What
crossed is UE 4.26 engine facts (Blueprint-callable functions and the `FProperty` layout, 2) and names already in the
runtime's specs.

Conventions are R0's: **S** server, times in `GetTickCount64` milliseconds. **(call)** marks a decision this page
makes, **(ambiguity)** something only the game can settle; both are collected in section 7.

---

## 1. Scope

| What | Where |
| --- | --- |
| The command file's grammar, the plan, positions, the layout check and every log text (pure) | `rustic-isles-runtime-core::sim` |
| Reading the file once a second, spawning, wandering and removing bots, the pacing hint | `rustic-isles-runtime` `server_life::sim` |

Bots run only on a server with `RUNTIME_SIM_PLAYERS.flag` beside its executable (call 1). Without it the command file
is never read and nothing here runs. R8 installs nothing on a client, and neither does this.

What bots load: actor ticking, character movement, collision, AI controllers and path following, and the replication
of each bot's actors to whatever real clients are connected. What they do not load: a connection per player (no
`UNetConnection`, no channels, no client-to-server RPCs), the backend's login and loadout calls, and a player state.
So bots measure the world's cost per player, not the network's; a full-path run still needs real clients (call 2).

Bots are not players to R8: they do not count as connections, so an empty disposable server with bots is still reaped
(R8 7.3). Run stress tests on a hub. The one exception is the frame pacer (4.4).

## 2. Engine facts

### 2.1 Functions (all Blueprint-callable, called through R17's `reflect::call` / `static_call`)

| Function | Called on | Parameters (offset) |
| --- | --- | --- |
| `AIBlueprintHelperLibrary.SpawnAIFromClass` | the library's default object | `WorldContextObject` 0x00, `PawnClass` 0x08, `BehaviorTree` 0x10, `Location` 0x18 (3 × `f32`), `Rotation` 0x24 (3 × `f32`), `bNoCollisionFail` 0x30 (`bool`), `Owner` 0x38, `ReturnValue` 0x40 (`APawn*`) |
| `AIBlueprintHelperLibrary.SimpleMoveToLocation` | the library's default object | `Controller` 0x00, `Goal` 0x08 (3 × `f32`) |
| `Pawn.GetController` | the bot's pawn | `ReturnValue` 0x00 |
| `Actor.K2_GetActorLocation` | a `PlayerStart` | `ReturnValue` 0x00 (3 × `f32`) |
| `Actor.K2_DestroyActor` | a bot's controller, then its pawn | none |

### 2.2 The layout check (call 3)

Before the first call of each function in a process, its parameters are read from the function itself and compared to
2.1: the `FProperty` chain from `UStruct::ChildProperties` (`0x50`), each `FField::Next` at `0x20`, its name
(`FField::NamePrivate`, an `FName` at `0x28`) and `FProperty::Offset_Internal` (`i32`) at `0x4C`, at most 64 fields.
Every name in 2.1 must be present with that offset; extra fields are allowed. A function that fails is not called for
the rest of the process, and the server logs once:

```text
[MitRuntime] [Sim] <Class>.<Function> refused: expected <name>@0x<o>, ...; found <name>@0x<o>, ...|nothing|unreadable
```

When a wrong offset table makes every name unreadable, the check fails closed: no bot is spawned.

## 3. The command file

- Name: `rustic_isles_sim_port<port>.txt` beside the executable, `<port>` R1's `config::port()` (as the kick file,
  `spec-runtime-kick.md` 3).
- Read once a second from R8's after handler while listening, then **removed before it is acted on**.
- One command a line; blank lines and lines starting with `#` are ignored; the last valid command of each kind wins:

| Command | Meaning |
| --- | --- |
| `bots <n> [<class>]` | keep `n` bots (`0..=256`), of pawn class `<class>` (a short class name, 1 to 128 of `A-Za-z0-9_`); without a class, the current one (default `BP_PlayerCharacter_C`) |
| `clear` | `bots 0` |
| `wander <seconds>` | a new goal for every bot each `<seconds>` (`0` off, else `1..=600`; default 5) |

A malformed line is counted, never acted on. The read logs:

```text
[MitRuntime] [Sim] read <file>: target <n> bots of <class>, wander <s> s[, <k> lines ignored]
```

A host writes the file with any editor or shell, for example
`Set-Content rustic_isles_sim_port7777.txt "bots 16"` in the game's folder.

## 4. The server's step (once a second, on the game thread)

### 4.1 Dropping the dead

A bot whose pawn is no longer a live object (`is_live_object`) is dropped from the list.

### 4.2 Toward the target

- **Fewer than the target**: spawn up to **4** this step (call 4). The origin is the first `PlayerStart`'s
  location, found once per process (`first_instance`, then `K2_GetActorLocation`); no `PlayerStart` logs once
  `[MitRuntime] [Sim] no PlayerStart: no bots` and spawns nothing. Bot `i` (its slot, the lowest free one) stands on a
  ring: angle `i * 22.5°`, radius `300 + 150 * (i / 16)` units, at the origin's height; rotation zero;
  `bNoCollisionFail` true; no behaviour tree; owner null. The class is looked up with `find_class`; a class not loaded
  logs once per class `[MitRuntime] [Sim] class <class> is not loaded: no bots` and the target is set to the current
  count. A `ReturnValue` that is null or not a live object counts as a failure; after 8 failures in a row the target is
  set to the current count and the server logs `[MitRuntime] [Sim] 8 spawns failed in a row: target lowered to <n>`.
- **More than the target**: remove up to **8** this step, highest slot first: `K2_DestroyActor` on the pawn's
  controller (when `GetController` gives a live one), then on the pawn.
- A change of count logs `[MitRuntime] [Sim] bots <n>/<target> (<+k|-k>)`.

### 4.3 Wandering

When wander is on and `<seconds>` have passed since the last wander, each bot with a live controller gets
`SimpleMoveToLocation` to a point on a disc of radius 1500 around the origin, at the origin's height, from a
linear-congruential generator seeded with the tick at the first wander. A level without a navigation mesh leaves the
bots standing (ambiguity 2).

### 4.4 Pacing

R8's pacer drops to the idle frame cap with no client (R8 4.2), which would measure bots at 5 fps. While the list is
not empty, the pacer sees at least one connection **(call 5)**. Nothing else in R8 does.

### 4.5 Start

At the first step: `[MitRuntime] [Sim] on (RUNTIME_SIM_PLAYERS.flag): reading rustic_isles_sim_port<port>.txt once a
second`. Without the flag, no line and no step.

---

## 5. Log lines

All through R0's prefixed writer, `[MitRuntime] [Sim] ...`, as listed in 2.2, 3 and 4. Never logged: the file's other
contents, player data.

## 6. Tests

### 6.1 In CI

- The grammar: each command, the last-wins rule, `clear`, the bounds (`bots 65`, `wander 601`, a class with a space),
  comments, the ignored count, and the read line.
- The plan: spawn at most 4, remove at most 8 highest slot first, the lowest free slot, the failure streak.
- The ring and the disc: slot 0, slot 16's larger radius, every wander goal within 1500 units.
- The layout check on found layouts: a match with extra fields, a wrong offset, a missing name, an unreadable chain
  (fails closed even for a function with no parameters), and the refusal line. The chain walk itself is checked by the
  play-test (a `refused` line names what it found).

### 6.2 The maintainer's check (with the other hands-on runs)

1. Put `RUNTIME_SIM_PLAYERS.flag` and `RUNTIME_CENSUS.flag` beside the server executable; start a hub; join with one
   client.
2. Write `bots 8` to the port's file. Within 2 s: the read line and `bots 8/8 (+8)`; the client sees eight characters
   near the spawn, walking after 5 s. Note any `refused` line.
3. `bots 32`, `bots 64`, `bots 128`, then `bots 256`: record `[Perf]`, `[Memory]` and the census summary at each step (the input for choosing
   what to stub).
4. `clear`: `bots 0/0 (-256)` over 32 s; the next census's actor deltas go back down.
5. Disconnect the client with bots running: the frame rate stays at the active cap.
6. If `BP_PlayerCharacter_C` crashes or misbehaves without a player state, record it, and try another pawn class with
   `bots 8 <class>` (ambiguity 1).

---

## 7. Calls and ambiguities

### 7.1 Calls

1. **Opt-in by flag file**, as the census: a production server never reads the command file.
2. **Server-side bots, not headless clients** (maintainer, 2026-11-17): many bots fit on one PC, a run repeats, and the
   world's cost per player is what the census needs. Real headless clients remain the way to measure the network.
3. **Check every parameter layout against the function's own properties.** The offsets in 2.1 are stock UE 4.26; the
   check makes a wrong one a logged refusal instead of a write past a parameter.
4. **4 spawns and 8 removals a step**, so ramping to 256 bots takes 64 s and never stalls one frame for long.
5. **Bots keep the pacer active** so a test with no client measures the active frame rate. They do not stop R8's reaps
   (1): a stress test runs on a hub.

### 7.2 Ambiguities

1. Whether `BP_PlayerCharacter_C` lives without a player controller and player state (the game's own code may expect a
   loadout, an inventory or a player state). If not, a host names another pawn class in the file.
2. Whether hub levels carry a navigation mesh. Without one, `SimpleMoveToLocation` does nothing and bots stand still,
   which still loads ticking and replication but not movement.
