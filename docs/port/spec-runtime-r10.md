<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R10, the player host for realms (RD27, plan item 11)

R10 makes the MIT game runtime a **player host**: the game server a player's launcher starts on its own PC for a
realm's hub or a hosted hunt (`RUSTIC_ISLES_AUTH_MODE=HUNT_CAPABILITY`). It checks the launcher's bootstrap before the
server listens, pins the listen to loopback, tells the launcher it is ready over a private pipe, takes the launcher's
fence over a second pipe, sends the session's host capability on every backend request instead of the gameserver key,
and answers the launcher's hosted role (hub or hunt) to the slices that apply it. It owns the feature ids `SP-2@R10`,
`SP-3@R10` and `HTTP-1@R10` (`spec-runtime-surface.md` sections 5.2 and 8; `spec-runtime-r0.md` section 7.5) and
implements R9's `PlayerHostListen` interface (`spec-runtime-r9.md` section 8.2).

R10 also carries ruling **RT-8** (`rulings-2026-11-13.md`): the last Mystic Paradox names between the runtime, the
launcher and the Director are renamed in one joint cutover, and section 12 lists every place that changes with it.

An implementer builds from this page, `spec-runtime-surface.md`, `spec-runtime-r0.md`, `spec-runtime-r1.md`,
`spec-runtime-r2.md`, `spec-runtime-r4.md` and `spec-runtime-r9.md` alone, and never reads `restricted/`. What crossed
is facts: the exact environment names, value grammars, pipe settings, message texts, exit codes and log texts the
launcher and the operator depend on, Win32 calls and their flags, and behaviour. The AGPL side (seam patch 0041, R9's
patch 0052, and runtime patch **0055** written with this page) is described only by the contract it keeps (section
13).

Conventions are R0's: **S** server. **(call)** marks a decision this page makes, **(departure)** a deliberate
difference from the AGPL runtime, **(ambiguity)** something the source did not settle; all are collected in section
17.

---

## 1. Scope

| Id | What | Kind (R0 registry) | Section |
| --- | --- | --- | --- |
| `SP-3@R10` | the player host's per-frame work: the bootstrap verdict and the control thread on the first frame, the fence on every frame | Feature | 5, 6, 7 |
| `HTTP-1@R10` | the host capability header on backend requests | Feature | 9 |
| `SP-2@R10` | the Hunt Guard evidence observer: owned and empty (no producer exists, 10) | Feature | 10 |

Also R10's, with no id of its own: the bootstrap check (5), the `PlayerHostListen` implementation R9 calls (8), the
hosted role other slices read (11), and the RT-8 renames inside the MIT runtime (12.1).

Not R10:

- The listen itself, the map URL and the DeployServer's stdout marker (R9). R9 calls R10 at three points (8).
- The engine command line's `MULTIHOME` (R4's CL-1 carries `-MULTIHOME=127.0.0.1` into it; R9 compares the pin with it).
- Applying the hosted role: the empty-server watchdog's hub exemption, the hub cap enforcement and the AFK timeouts
  (R8), and the `?MaxPlayers=` value of the map URL (R1 section 10, built by R9). R10 publishes the role (11).
- The launcher's side of the pipes and the hosted server's start (`rustic-rifts-host`, section 12.2).
- Hunt Guard evidence: no producer exists in either runtime (10).

**Dependencies.** R0 (broker, SP-3 chain, handlers, log, flags, `env_value`, `readable`/`read`, `terminate`), R1
(`config::auth_mode()`, `config::launch()` with `raw_entry_1`, `config::port()`, the raw environment read
`env_raw`, `config::backend_url().api_base()`), R2 (the HTTP-1 chain and its request facts, R2 section 3.1), R4
(indirectly, through R9's pin check), and **R9** (it calls R10's interface). **(call)** R10 lands **with or after R9
and R2**: a build with `"R10"` in `LANDED` and without `"R9"` or `"R2"` is refused by a registry test (16.1), because
the AGPL would then listen for a player host whose pipes and fence the MIT holds.

---

## 2. Where it lives

| File | Kind | Holds |
| --- | --- | --- |
| `crates/rustic-isles-runtime-core/src/player_host.rs` (new) | pure, every platform | the names of section 4; the bootstrap check over a fake environment (5); the role check and its problem texts (5.3); the message texts and the fence comparison (7, 8.2); the role line texts (6); the log texts of section 15 |
| `crates/rustic-isles-runtime/src/player_host/mod.rs` (new, safe) | Windows | `register()`, the SP-3 handler, the fence state, the `PlayerHostListen` implementation, the hosted role (11), the HTTP-1 wrapper's decision |
| `crates/rustic-isles-runtime/src/ffi/player_host.rs` (new, under `ffi`) | Windows, unsafe | the control pipe thread, the ready pipe write, the pipe's security descriptor, the `SetHeader` call, `TerminateProcess` |

Lines R10 adds to files other slices own (one each, append-only, as R0 section 2 allows):

- `registry.rs`: `"R10"` in `LANDED`. The three ids are already in `ENTRIES`. A test (16.1): `"R10"` in `LANDED`
  implies `"R9"` and `"R2"` in it.
- `offsets.rs`: **none**. R10 hooks nothing new; the request facts it uses are R2's lines.
- `lifecycle.rs`: on a server whose auth mode is `PlayerHost`, `player_host::register()` **before**
  `handlers::freeze()` (it reads the bootstrap, registers the SP-3 handler and registers with R9); and in the server
  install list, `("HTTP-1@R10", player_host::install_header(base))` right after R2's `HTTP-1` entry.
- `ffi/mod.rs`: `pub mod player_host;`. `lib.rs`: `pub mod player_host;` under the hooking slices' `cfg`. Core
  `lib.rs`: `pub mod player_host;`.

And the RT-8 edits inside the runtime (12.1): R1's alias is deleted, R9's legacy stdout line is deleted, and R1's
launcher-override source reads R10's role.

---

## 3. Registry, ownership and the AGPL in one table

With `"R10"` in `LANDED`, `rir_owns` answers 1 for `SP-2@R10`, `SP-3@R10` and `HTTP-1@R10`; the status line's `owns=`
count grows by 3.

| Situation | Bootstrap check, control pipe, fence | Listen pin and ready pipe | Capability header |
| --- | --- | --- | --- |
| MIT DLL absent | AGPL | AGPL | AGPL |
| R9 landed, R10 not | AGPL | AGPL (R9's stand-down, patch 0052) | AGPL |
| R10 landed (so R9 too) | MIT | MIT, called by R9 | MIT |

Every row speaks the same wire once patch 0055 and this slice are both in (section 4): the AGPL takes the RT-8 names
in 0055 whether or not the MIT DLL is present, so the launcher sees one protocol whichever DLL holds it.

---

## 4. Names (RT-8 and D3)

The player host's wire and environment after the cutover. Old names are never accepted, not even as aliases (D3's
rule, `spec-rd27-rifts.md`).

| What | Old (AGPL today) | New | Who reads it |
| --- | --- | --- | --- |
| Message tag on both pipes | `MYS1` | **`RIS1`** **(call)** | launcher ↔ hosted runtime |
| Ready and control pipe prefix | `\\.\pipe\MysticInternalServer.` | **`\\.\pipe\RusticInternalServer.`** | the runtime checks it; `rustic-rifts-host` already writes it |
| Host capability header | `x-mystic-host-capability` | **`x-rustic-host-capability`** (D3) | the MIT backend accepts only this one already |
| DeployServer's legacy stdout line | `MYSTICPARADOX_GAMESERVER_READY launchId=… port=…` | **removed**; `RUSTIC_ISLES_GAMESERVER_READY …` stays the only line | the Director parses only the new line already |
| Environment alias | every `MYSTICPARADOX_<X>` copied to `RUSTIC_ISLES_<X>` | **removed** | — |
| The hosted server's launch variables | (the AGPL reads `RUSTIC_ISLES_<NAME>`; `rustic-rifts-host` writes `RUSTIC_RIFTS_<NAME>`) | **`RUSTIC_ISLES_<NAME>`** **(call)** | the runtime; `rustic-rifts-host` changes (12.2) |

Why these and not `DAINTY_` names: AGENTS.md's naming gives `dainty-`/`DAINTY_` to the core modules and `rustic-` to
the game servers and realms. Everything above is between a realm's launcher and a Rustic Isles game server, so it is
`Rustic`/`RUSTIC_ISLES_`, as D3 already made the header and the Guard pipe. `RIS1` (Rustic Isles server, version 1)
keeps the old tag's length and digit, as D3 did for its magic values; D3's mechanical `M`→`R` would give `RYS1`, which
reads as nothing. The pipe prefix is the one `rustic-rifts-host` already writes (its clean room renamed it under D3's
"none of the old names" rule), so the launcher keeps it.

**The launch variables (call, for the maintainer).** `rustic-rifts-host` names the hosted server's variables
`RUSTIC_RIFTS_<NAME>` (its clean room applied D3's settings row to them and asked for "a runtime patch in the
cutover"). The runtime reads `RUSTIC_ISLES_<NAME>` for the same process from the Director (`RUSTIC_ISLES_AUTH_MODE`,
`_GAMESERVER_LAUNCH_ID`, `_TRIALS_WEEK`, `_RUNTIME_PROFILE`, `_ALLOW_DAMAGE_DEBUG`, `_DEBUG_DAMAGE_MULTIPLIER`), and R0,
R1, R9 and R18 already read those. Reading `RUSTIC_RIFTS_` too would be an alias by another name, keyed on who started
the game. **Recommended:** the game's variables are `RUSTIC_ISLES_` whoever starts it; `rustic-rifts-host`'s
`rifts_var!` prefix becomes `RUSTIC_ISLES_` for its 17 launch and 2 cleared variables. D3's `RUSTIC_RIFTS_` stays for
what it named: the server's realm settings in the env files setup writes. This also fixes a live gap: the launcher
clears `RUSTIC_RIFTS_ALLOW_DAMAGE_DEBUG`, but the runtime reads `RUSTIC_ISLES_ALLOW_DAMAGE_DEBUG`, so a debug switch
in the launcher's own environment would reach a hosted server.

---

## 5. The bootstrap (`SP-3@R10`, decided in Init)

### 5.1 When

In `player_host::register()`, during R0's Init on a server whose R1 auth mode is `PlayerHost`. Init may read the
environment (R0 4.1) and must not log or start a thread, so R10 only **decides** here: the verdict, the values it
keeps and the problems it found. The first engine frame reports and acts on it (6). Nothing in this section runs for
`GlobalGameserver` or on a client.

### 5.2 The bootstrap checks (all required)

Checked in this order; the first that fails is the **failed check** the log names (by its label, never a value).
Each variable is read with R0's `env_value(name, max)` (unset, empty, over `max`, NUL, CR, LF or non-ASCII are absent).
"Safe" characters are ASCII letters, digits, `.`, `_` and `-`.

| # | Label | Rule |
| --- | --- | --- |
| 1 | `full launch` | R1's launch is a full launch (more than 8 argv entries). A player host never runs in the manual mode |
| 2 | `argv 1` | R1's `raw_entry_1` is exactly `-` (the player host never carries a gameserver key) |
| 3 | `MULTIHOME` | some argv entry after the program equals `-MULTIHOME=127.0.0.1`, ignoring ASCII case |
| 4 | `RUSTIC_ISLES_BIND_ADDRESS` | max 15; exactly `127.0.0.1` |
| 5 | `RUSTIC_ISLES_SESSION_ID` | max 36; exactly 36 safe characters |
| 6 | `RUSTIC_ISLES_SESSION_GENERATION` | max 10; 1 to 10 ASCII digits (leading zeros allowed; kept as text) |
| 7 | `RUSTIC_ISLES_HOST_RUNTIME_CAPABILITY` | max 43; exactly 43 safe characters. **A secret** (14) |
| 8 | `RUSTIC_ISLES_CONTROL_PIPE` | max 240; starts with `\\.\pipe\RusticInternalServer.` (case-sensitive), followed by 16 to 96 safe characters and nothing else |
| 9 | `RUSTIC_ISLES_READY_PIPE` | the same rule |
| 10 | `RUSTIC_ISLES_GAMESERVER_LAUNCH_ID` | max 96; 16 to 96 safe characters |
| 11 | `RUSTIC_ISLES_LAUNCHER_PID` | max 10; 1 to 10 ASCII digits whose value is 1 to 4 294 967 295 |

The values kept: session id, generation text, capability, both pipe names, launch id, launcher pid. They never change
in the process **(departure)**: the AGPL read the bind address, the ready pipe, the session, the generation and the
launch id again at listen time; the MIT uses what it checked.

### 5.3 The hosted role (optional variables)

Checked even when 5.2 failed, so the log can name a role problem as well. Each is read **raw**: unset (the variable
does not exist) is "absent"; a variable that exists but is empty, longer than its cap, or holds a character below
`0x20` or `0x7F` is **malformed** (R1's `env_raw` reports an empty value as unset, so R10 also asks whether the
variable exists, as R1 3 does).

| Variable | Cap | Valid | Problem text when malformed | Problem text when not valid |
| --- | --- | --- | --- | --- |
| `RUSTIC_ISLES_HOSTED_ROLE` | 8 | `HUNT` or `HUB` exactly; absent is `HUNT` | `RUSTIC_ISLES_HOSTED_ROLE is set but empty or malformed` | `RUSTIC_ISLES_HOSTED_ROLE is neither HUNT nor HUB` |
| `RUSTIC_ISLES_MAX_PLAYERS` | 5 | a plain decimal (digits only, no sign, no leading zero except `0` itself, at most 5 digits) from 1 to 64 | `RUSTIC_ISLES_MAX_PLAYERS is set but empty or malformed` | `RUSTIC_ISLES_MAX_PLAYERS is not a whole number from 1 to 64` |
| `RUSTIC_ISLES_HUB_AFK_SECONDS` | 5 | the same grammar, 0 to 7200 | `RUSTIC_ISLES_HUB_AFK_SECONDS is set but empty or malformed` | `RUSTIC_ISLES_HUB_AFK_SECONDS is not a whole number from 0 to 7200` |

In that order; the first problem stops the role check and is the **role problem**. The numbers are checked for a
`HUNT` role too (a launcher bug fails loudly either way) but only a `HUB` uses them.

### 5.4 The verdict

`Valid(Bootstrap)` when 5.2 and 5.3 both pass, else `Invalid { failed_check: Option<label>, role_problem:
Option<text> }`. Pure: `bootstrap(read: impl Fn(&str) -> Raw, exists: impl Fn(&str) -> bool, launch: &Launch, argv:
&[Vec<u16>]) -> Verdict` in the core crate, tested over a map.

`RUSTIC_ISLES_HUNT_GUARD_EVENTS` is read too (max 8), only for the log line of 6: `1` means the launcher opted the
process in; anything else, or absent, means it did not.

---

## 6. The first frame (`SP-3@R10`)

R10 registers one SP-3 **before** handler: `slice = "R10"`, `order = -2000` **(call)**, before R5's `-1000` and
R11's `-500`, so a fenced or refused host does nothing else the frame. Registered only in `PlayerHost` mode. SP-3
handlers cannot absorb (R0 call 15); R10 never needs to, because it ends the process instead (7.3).

On its first call only, in this order:

1. **Invalid bootstrap:** log `[MitRuntime] [PlayerHost] invalid player-host bootstrap (<failed check, or "hosted
   role">); terminating before listen`, then, when there is a role problem, `[MitRuntime] [HostedRole] invalid
   launcher role: <role problem>`; then end the process with exit code **`0xE103`**. **(departure)** The AGPL's line
   did not name the failed check.
2. **The role line**, once:
   - hub: `[MitRuntime] [HostedRole] launcher role=HUB (persistent hub) maxPlayers=<n> (launcher)|maxPlayers=not set
     (host setting or default) hubAfkSeconds=<n> (launcher)|hubAfkSeconds=not set (host setting or default)`;
   - hunt: `[MitRuntime] [HostedRole] launcher role=HUNT (disposable)`, followed, when either number is set, by
     ` - RUSTIC_ISLES_MAX_PLAYERS / RUSTIC_ISLES_HUB_AFK_SECONDS are set but ignored (they apply to a HUB only)`.
3. **Hunt Guard line**, once: `[MitRuntime] [HuntGuard] no evidence producer in this runtime: the hunt is unmonitored
   and earns no rewards (opt-in <1|not set>)`.
4. **The control thread** (7.1): `CreateThread` with the default stack. Failure: log `[MitRuntime] [PlayerHost]
   control thread creation failed; terminating before listen` and end the process with **`0xE104`**. Success: log
   `[MitRuntime] [PlayerHost] control pipe worker started`.

Every call, the first included, after the steps above: the fence check (7.3).

**(departure)** The AGPL started the control thread from its MainThread, which ran from `DllMain` while the engine
booted; the MIT starts it on the first engine frame (R0 3.1 forbids a thread from `DllMain`). The window in which the
launcher cannot fence yet is the engine's boot either way; the launcher's side ends the process itself when the
control pipe does not open (12.2).

---

## 7. The control pipe and the fence

### 7.1 The control thread

1. `CreateNamedPipeW(control pipe name, PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_TYPE_MESSAGE |
   PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 1, 512, 512, 0, &attributes)`. **(departure)**
   `attributes` carries a protected DACL that grants `GENERIC_ALL` to this process token's user SID and to
   `LocalSystem` only (built from SDDL `D:P(A;;GA;;;<user SID>)(A;;GA;;;SY)`); the AGPL used the default DACL, which
   lets every local account open the pipe for reading. If building the descriptor fails, R10 does not fall back to
   the default: it logs and fences as for a creation failure. Failure: log `[MitRuntime] [PlayerHost] control pipe
   creation failed error=<GetLastError>` and fence (7.2).
2. `ConnectNamedPipe(pipe, NULL)`; a false answer with `ERROR_PIPE_CONNECTED` (535) counts as connected. Failure: log
   `[MitRuntime] [PlayerHost] the launcher did not connect error=<n>`, close, fence.
3. `GetNamedPipeClientProcessId`; a failure, or a pid other than the bootstrap's launcher pid: log `[MitRuntime]
   [PlayerHost] rejected an unexpected client pid` (no pid in the line), disconnect, close, fence.
4. One `ReadFile` of at most 511 bytes. A failed read (a message over 511 bytes fails with `ERROR_MORE_DATA`), zero
   bytes, or bytes not exactly equal to the fence message of 7.4: log `[MitRuntime] [PlayerHost] rejected a malformed
   fence request`, disconnect, close, fence.
5. The fence message: **fence first**, then write the acknowledgement of 7.4 with one `WriteFile`, `FlushFileBuffers`,
   `DisconnectNamedPipe`, `CloseHandle`, and log `[MitRuntime] [PlayerHost] generation fenced by the launcher`. A
   failed write changes nothing (the fence already holds).
6. **(departure)** The thread then waits 5000 ms. If the process is still running, it logs `[MitRuntime] [PlayerHost]
   the game thread did not apply the fence within 5 s; forcing termination` and ends the process with **`0xE105`**.
   The AGPL measured its 5 s on the game thread, so a hung game thread never ended; the MIT's fallback is on the
   thread that does not hang.

The thread serves one connection in its life, as the AGPL's did: any first contact either fences or is the fence.

### 7.2 "Fence"

One process-wide atomic flag, set by the control thread (7.1) and by `on_listen_failed` (8.3). Never cleared.

### 7.3 Applying it on the game thread

In the SP-3 handler, every frame: when the flag is set, log once `[MitRuntime] [PlayerHost] applying the fence on the
game thread; ending the process` and end the process with exit code **`0`** (`TerminateProcess` on the current
process). **(call)** The AGPL did the same through its server quit path (exit 0, skipping the engine's teardown, which
hangs for 60 s and aborts on this build); the old alternative behind `SERVER_QUITGAME_EXIT.flag` (R8's flag) asked the
game to quit, which does nothing on a server without a local player, so R10 does not offer it. A player host keeps
nothing in process: the backend holds all of it.

### 7.4 The messages (exact bytes, ASCII, one LF, no CR)

```text
RIS1 FENCE session=<session id> generation=<generation>\n         launcher → runtime, on the control pipe
RIS1 FENCE_ACK session=<session id> generation=<generation>\n     runtime → launcher, the answer
RIS1 READY session=<session id> generation=<generation> launch=<launch id> port=<port> bind=127.0.0.1\n
                                                                  runtime → launcher, on the ready pipe
```

`<session id>`, `<generation>` and `<launch id>` are the bootstrap's texts verbatim; `<port>` is the decimal port R9
passes. The pure `fence_request`, `fence_ack` and `ready_message` build them; `is_fence(bytes, session, generation)`
is exact byte equality with `fence_request`.

---

## 8. What R9 calls (`PlayerHostListen`, R9 section 8.2)

`player_host::register()` registers one implementation with R9's `register_player_host` in `PlayerHost` mode with a
valid bootstrap. With an invalid bootstrap it registers nothing: the process ends on the first frame (6) before R9's
listen handler (an SP-3 **after** handler) runs.

### 8.1 `prepare_listen() -> Result<Option<&'static str>, ()>`

`Ok(Some("127.0.0.1"))`. The bind address was checked to be exactly `127.0.0.1` (5.2 check 4), and R9 honours the pin
only when the engine's `MULTIHOME` (R4) is the same address, which check 3 made the launcher pass. `Err(())` only if
the verdict is somehow not `Valid` (a build error; the process should already have ended), logging `[MitRuntime]
[PlayerHost] bind address is not loopback; refusing listen`.

### 8.2 `report_ready(port: u16) -> bool`: the ready pipe

On the game thread, inside R9's listen (the server has no players yet, so blocking here costs nothing):

1. Note the start time. For attempt 1, 2, …:
   1. `WaitNamedPipeW(ready pipe name, 5000)`. False: log `[MitRuntime] [PlayerHost] the launcher's readiness pipe did
      not become available error=<n> attempt=<k>` and return false.
   2. `CreateFileW(ready pipe name, GENERIC_WRITE, 0, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL |
      SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, NULL)`. **(departure)** The quality-of-service flags stop the
      pipe's server from impersonating the game process; the AGPL opened it without them. Success: go to step 2.
   3. Failure: log `[MitRuntime] [PlayerHost] the launcher's readiness pipe open failed error=<n> attempt=<k>`. If the
      error is not `ERROR_PIPE_BUSY` (231), or 5000 ms have passed since the start: return false. Else sleep 100 ms
      and try again (another opener can take the single instance between the wait and the open).
2. Write `ready_message` (7.4) with one `WriteFile`; `FlushFileBuffers`; `CloseHandle`. A failed or short write: log
   `[MitRuntime] [PlayerHost] the readiness message write failed` and return false.
3. Log `[MitRuntime] [PlayerHost] readiness reported to the launcher` and return true.

A false answer makes R9 fail the listen (`readiness report failed`) and call `on_listen_failed`.

### 8.3 `on_listen_failed()`

Sets the fence flag (7.2). The next frame ends the process with exit code 0, as the AGPL did.

---

## 9. `HTTP-1@R10`: the host capability header

One MIT wrapper on the HTTP-1 chain through R0's broker, **`order = 100`** (R2 section 3.2 asks for a larger order
than its own `0`, so R2's redirect has run when R10 looks at the URL). Installed only in `PlayerHost` mode with a valid
bootstrap; otherwise the install entry is `skipped (not a player host)`. Each call, on whatever thread issued the
request:

1. If detaching: return `next(request)`.
2. Inside `catch_unwind`: when the request is readable for `0xD0`, its in-flight word (`i32` at `+0xC4`) is not `1`,
   its vtable is readable for 17 slots, **and its URL (the `FString` at `+0x70`, read as R2 section 4.1 reads it, after
   R2's redirect) starts with R1's API base followed by `/`** **(departure, the rule of R2's call 4, ruled 2026-11-13)**: call the
   vtable's slot 16, `SetHeader(request, name, value)`, with `name` = `x-rustic-host-capability` and `value` = the
   capability, each a UTF-16 `FString` R10 owns for the call (`num` = length + 1 with the NUL, `max` = `num`).
   `SetHeader` copies both.
3. Return `next(request)`, its answer passed through.

R10 never sends the gameserver key (R2 sends it only in `GlobalGameserver` mode) and never logs the capability, its
length, or that a request carried it. The AGPL set the header on every request in this mode, whatever its host; with
every backend request rewritten to the API base, the URL condition changes nothing the backend sees and keeps the
capability off any other host, as the ruling on R2's call 4 does for the gameserver key.

---

## 10. `SP-2@R10`: the Hunt Guard observer

The AGPL passes every server `ProcessEvent` call of a player host to an evidence producer that is not in this tree: the
anti-cheat module is absent, its stub produces nothing, and a hosted hunt is unmonitored and earns no rewards (the
Guard's policy, not the runtime's). **(call)** R10 owns `SP-2@R10` and registers **no** SP-2 handler: owning the id
retires a call that did nothing. A future producer is a slice of its own with its own spec; it would write `RGE1`
messages to `RusticHuntGuard.events.<X>` (D3) for `rustic-rifts-transport`. R10 does not read
`RUSTIC_ISLES_GUARD_PIPE` or `RUSTIC_ISLES_HUNT_GUARD_COMPLETION_MODE`.

---

## 11. The hosted role (for R1 and R8)

`rustic_isles_runtime::player_host`:

```rust
pub struct HostedRole {
    pub hub: bool,
    /// `RUSTIC_ISLES_MAX_PLAYERS`; `Some` only for a hub.
    pub max_players: Option<u8>,
    /// `RUSTIC_ISLES_HUB_AFK_SECONDS`; `Some` only for a hub.
    pub hub_afk_seconds: Option<u16>,
}
/// `Some` only in `PlayerHost` mode with a valid bootstrap. Set in Init, read from any thread.
pub fn hosted_role() -> Option<&'static HostedRole>;
pub fn is_hub() -> bool; // hosted_role().is_some_and(|r| r.hub)
```

(Shape, not names.) Today's meaning, which R8's spec applies: a launcher `HUB` makes a named hub class (`City`,
`TrainingGrounds`) persistent (never reaped empty, the hub AFK rule) and takes its cap and AFK timeout from the two
numbers; a `HUB` role on any other class changes nothing; a `HUNT` role, or no role, keeps every class disposable.

**R1's launcher source (one edit in `config.rs`).** R1 section 10's source 1 currently reads the two variables itself
in `PlayerHost` mode, whatever the role. With R10 landed it takes `hosted_role()`'s `max_players` and
`hub_afk_seconds` instead, so the override counts **only for a hub with a valid bootstrap**, as in the AGPL. R1's
source texts and clamping do not change.

---

## 12. The joint cutover (RT-8)

One release carries all of 12.1 to 12.3. Nothing on the MIT side speaks the old names today except R1's alias, R9's
legacy line and the launcher kit's second profile variable, so the cutover breaks no MIT pairing; it does retire the
retired Paradox launcher's pairing for good. Cut over with no hosted session running (D3's rule).

### 12.1 In the MIT runtime (R10's implementer)

| File | Change |
| --- | --- |
| `crates/rustic-isles-runtime-core/src/config/alias.rs` and its tests | deleted |
| `crates/rustic-isles-runtime/src/config.rs` | `run_alias` deleted; source 1 from `player_host::hosted_role()` (11) |
| `crates/rustic-isles-runtime/src/lifecycle.rs` | the `config::run_alias()` call deleted; R10's two lines (2) |
| `crates/rustic-isles-runtime-core/src/config/mod.rs` | `pub mod alias;` deleted |
| R9's `ready_lines` (core `listen.rs`) and its test | the `MYSTICPARADOX_GAMESERVER_READY` line deleted: one line, `RUSTIC_ISLES_GAMESERVER_READY launchId=<id> port=<port>\r\n` |
| `crates/rustic-isles-runtime/src/player_host/`, `ffi/player_host.rs`, core `player_host.rs` | new (2) |
| `docs/security/game-runtime.md` | R10's section (14); the "In" line loses `MYSTICPARADOX_*` and the "Out" line its alias writes |
| `fuzz/` | the target of 14.2 |

### 12.2 Elsewhere in the MIT tree (other rows; scheduled by the integration pass)

| Place | Change |
| --- | --- |
| `crates/rustic-rifts-host/src/session/mod.rs` (`rifts_var!`), `rules.rs`, `tests/hosting.rs` | **(call, 4)** the 17 launch and 2 cleared variables become `RUSTIC_ISLES_<NAME>`; the tests' `RUSTIC_RIFTS_*` expectations follow. The pipe names (`RusticInternalServer.ready.<X>`, `.control.<X>`) already match |
| `crates/rustic-rifts-host`: a real `ServerSpawner` and the launcher's pipe ends (its spec's known gap R8: the launcher passes `NoopServerSpawner`, so no hosted server is started today) | the row that closes the gap speaks this page's wire: **before** starting the server, create the ready pipe's server end (`\\.\pipe\RusticInternalServer.ready.<X>`, first instance, remote clients rejected, inbound, readable to this user only), wait for one client and read one line ending in LF of at most 511 bytes, and accept it only when it equals 7.4's `RIS1 READY …` for this session, generation, launch id and port with `bind=127.0.0.1`; to fence, open the control pipe as a client, write 7.4's `RIS1 FENCE …` in one write, and read `RIS1 FENCE_ACK …` for the same session and generation. If the control pipe cannot be opened (the server is still booting, or already gone) or the acknowledgement does not come within a few seconds, end the server's process itself. The server is started suspended and injected as RD9 says, with `RUSTIC_ISLES_LAUNCHER_PID` the launcher's own pid |
| `crates/dainty-launcher-kit/src/platform/launch.rs` | `PROFILE_VARS` loses `MYSTICPARADOX_RUNTIME_PROFILE` (one entry, `RUSTIC_ISLES_RUNTIME_PROFILE`), and its test at the end of the file follows. `STRIPPED_PREFIXES` keeps `MYSTICPARADOX_` **(call)**: stripping a stale inherited variable is harmless and keeps it from any older runtime |
| `crates/rustic-isles-director` | **nothing**: it parses only `RUSTIC_ISLES_GAMESERVER_READY` (its test that refuses the old marker stays) and sets no `MYSTICPARADOX_*` copies |
| `crates/dainty-host` (the fake gameserver script), `rustic-isles-director`'s fake gameserver | nothing: they print only the new line |
| `crates/dainty-hostkit`, `crates/dainty-admin`, `crates/dainty-legacy` | nothing: their `MYSTICPARADOX_*` names are server settings moved by setup, not the runtime's (D3's settings row) |
| `native/runtime-loader` | nothing: `MysticParadox.dll` and `MystPaxInternalServer.dll` are runtime DLL aliases, not RT-8's names |
| `launcher/`, `ui/` | nothing: no hit |
| `docs/port/spec-runtime-surface.md` 2.3, 5.1, 5.2, `spec-runtime-r0.md` 3.2 step 2, `spec-runtime-r1.md` section 3, `spec-runtime-r9.md` 7.1 and 10 | the integration pass marks them as superseded by this page (they are records) |
| `docs/port/spec-rd27-rifts-host.md` §6.2.1 and §11 R1, `docs/port/clean-room/rifts-host.md` §6.2 items 6 and 7 | note the launch variables' prefix (`RUSTIC_ISLES_`) and that the runtime reads the new pipe names (this page) |
| `docs/security/rifts-host.md` | "The hosted server" gains the pipe ends' items when the spawner row lands (12.2 row 2) |

### 12.3 In `restricted/` (patch 0055, written with this page; section 13)

| Place | Change |
| --- | --- |
| `modules/rustic-isles/native/runtime/p2p/PlayerHost.cpp` | the RT-8 and D3 names (4), and the stand-down of 13.2 |
| `modules/rustic-isles/native/runtime/Networking.cpp` | both `MYSTICPARADOX_GAMESERVER_READY` stdout lines removed |
| `modules/rustic-isles/native/runtime/dllmain.cpp` | the `MYSTICPARADOX_` alias pass and its call removed |
| `docs/paradox-patches.md` | row 0055 |
| `modules/rustic-isles/reference/1.14.7/ParadoxDirector` (the Node rollback Director until RB15) | **nothing required**: it accepts both markers and its `MYSTICPARADOX_*` copies are now ignored |
| `modules/rustic-rifts/docs/inventory.md`, `modules/rustic-rifts/docs/security/guard.md` | nothing: records of Mystic Paradox's tree |
| `launcher/src-tauri` | nothing: it has no realm pipe code; the launcher kit (12.2) is its |

The AGPL rebuild is a hands-on step (it needs the maintainer's SDK): the release that ships R10's MIT DLL ships the
AGPL DLL rebuilt with 0055.

---

## 13. The AGPL side

### 13.1 What seam patch 0041 and patch 0052 already do when the MIT owns R10's ids

- `HTTP-1@R10`: the AGPL's HTTP-1 wrapper skips its capability header.
- `SP-2@R10`: the AGPL's server `ProcessEvent` skips its evidence observer.
- `SP-3@R10`: the AGPL's tick skips its fence check (its early return).
- Listening: with `SP-3@R9` owned and `SP-3@R10` owned, patch 0052's rule makes the AGPL not listen for a player host,
  so its pin, ready pipe and listen-failure fence never run.

Not gated by them: the AGPL's player-host module still claims the auth mode, still checks the bootstrap in its Init,
and its MainThread still **starts its own control pipe thread**, or ends the process with `0xE103` when its own check
fails. Two control pipes of one name cannot both exist (first instance); whichever DLL made it would take the
launcher's fence, and the AGPL's would set a fence nothing applies. Hence 0055.

### 13.2 Runtime patch 0055 (written with this page)

In `restricted/modules/rustic-isles/native/runtime/` (`p2p/PlayerHost.cpp`, `Networking.cpp`, `dllmain.cpp`), marked
`[DaintyLass 0055]`, row 0055 in `restricted/docs/paradox-patches.md`. It keeps this contract:

- **The RT-8 and D3 names, always** (with or without the MIT DLL): the pipe prefix `\\.\pipe\RusticInternalServer.`
  in both of its checks, `RIS1` in the three messages, the header `x-rustic-host-capability`, no
  `MYSTICPARADOX_GAMESERVER_READY` line on stdout, and no `MYSTICPARADOX_` alias. Nothing else in its wire changes.
- **Stand-down, when the MIT runtime owns `SP-3@R10`:** the player host's start step logs once `[MitSeam] SP-3@R10 is
  the MIT runtime's: the player host's control pipe, readiness and fence are left to it` and returns success without
  ending the process on its own bootstrap check, without the control pipe thread and without the evidence worker. Its
  bootstrap check still runs in Init with the same rules and names as section 5, because its hub-role, cap and AFK
  answers feed its R8 blocks until R8 lands; the MIT ends the process on an invalid bootstrap (6), so the AGPL's
  answers are only ever read for a valid one.
- No seam ABI change; seam version stays 1.
- **With the MIT DLL absent, or R10 not landed**, the AGPL behaves as before apart from the names.

### 13.3 Cross-slice state (R0 7.5's table)

| State | Set by | Read by | Rule |
| --- | --- | --- | --- |
| the player host's launch values | R10's bootstrap (MIT) and the AGPL's own Init check (0055 keeps it) | the AGPL's hub cap and AFK overrides and hub exemption (R8 until it lands); the MIT's R1 source 1 and R8 | **carried**: both DLLs read the same variables with the same rules; R0's row "the AGPL keeps reading them until R10" now reads "until R8" |
| "fence requested" | R10 (control thread, listen failure) | R10's SP-3 handler only | not shared: each DLL's fence is its own, and 0055 keeps the AGPL's control pipe off when R10 is the MIT's |
| the player host's listen | R10's interface | R9's listen | R9 section 9.4's row "carried by the stand-down" ends when R10 lands: both listen and pipes are the MIT's |

R0's row "R8, R9 and R10 ship in one runtime release" was already lifted for R9 by patch 0052 (R9 9.4); R10 lifts it
for itself: R10 needs R9 and R2, not R8. R8 must still land **with or after R10** (R9 8.1).

---

## 14. Security (the pipes are a trust boundary)

### 14.1 What `docs/security/game-runtime.md` gains: a section "The player host (R10)"

- [ ] Every launch variable is bounded before it is copied and checked against 5.2 and 5.3's grammars; any failure
      ends the process with `0xE103` before listen, on the first frame. A manual-mode launch is never a player host.
- [ ] The capability is 43 safe characters, kept in memory only, sent only as `x-rustic-host-capability` to URLs
      under the API base (9), never logged, never written to a pipe, never sent with the gameserver key header.
- [ ] The listen is pinned to `127.0.0.1`: the bind address must be exactly that, the command line must carry
      `-MULTIHOME=127.0.0.1`, and R9 refuses the pin unless the engine's `MULTIHOME` equals it.
- [ ] The pipe names are taken only under `\\.\pipe\RusticInternalServer.` with 16 to 96 safe characters after it
      (no other pipe, no path, no remote server).
- [ ] The control pipe: one instance, first instance, remote clients rejected, message mode, a protected DACL for this
      user and `LocalSystem` only; the connecting process must be `RUSTIC_ISLES_LAUNCHER_PID`; the only accepted bytes
      are the exact fence for this session and generation, at most 511 bytes. Anything else fences the host (fail
      closed): a wrong client can end a host, never steer it.
- [ ] The fence ends the process (exit 0) on the next frame, and within 5 s from the control thread (`0xE105`) if the
      game thread does not tick.
- [ ] The ready pipe is opened with `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`, so the launcher's end cannot
      impersonate the game process; the message carries session, generation, launch id, port and `bind=127.0.0.1`,
      no secret.
- [ ] A listen that fails, or a readiness report that fails, fences the host.
- [ ] Never logged by R10: the capability, the session id, the launch id (it shares its random part with the pipe
      names), the pipe names, the launcher pid, a client's pid.
- [ ] No `MYSTICPARADOX_*` variable is read or written; the alias is gone (RT-8).

The page's "In" line gains the control pipe's bytes from the launcher; its "Out" line gains the ready pipe message, the
capability header and the exit codes `0xE103`, `0xE104`, `0xE105` and `0`.

### 14.2 Fuzz target

`rustic_isles_runtime_player_host` in `fuzz/`: arbitrary bytes split into an environment map, argv entries and one
control-pipe message. The bootstrap check never panics; a `Valid` verdict implies every grammar of 5.2 and 5.3 (checked
again independently in the target); `is_fence` is true only for the exact `fence_request` bytes.

---

## 15. Log lines

All through R0's prefixed writer, tagged `[MitRuntime]`. The texts of sections 6 to 8 in one list:

| When | Text |
| --- | --- |
| invalid bootstrap | `[MitRuntime] [PlayerHost] invalid player-host bootstrap (<label>); terminating before listen`; `[MitRuntime] [HostedRole] invalid launcher role: <problem>` |
| role | `[MitRuntime] [HostedRole] launcher role=HUB (persistent hub) maxPlayers=… hubAfkSeconds=…` / `launcher role=HUNT (disposable)[ - … ignored (they apply to a HUB only)]` |
| Hunt Guard | `[MitRuntime] [HuntGuard] no evidence producer in this runtime: the hunt is unmonitored and earns no rewards (opt-in <1\|not set>)` |
| control thread | `[MitRuntime] [PlayerHost] control pipe worker started` / `control thread creation failed; terminating before listen` |
| control pipe | `control pipe creation failed error=<n>`; `the launcher did not connect error=<n>`; `rejected an unexpected client pid`; `rejected a malformed fence request`; `generation fenced by the launcher`; `the game thread did not apply the fence within 5 s; forcing termination` (each after `[MitRuntime] [PlayerHost] `) |
| fence applied | `[MitRuntime] [PlayerHost] applying the fence on the game thread; ending the process` |
| ready pipe | `the launcher's readiness pipe did not become available error=<n> attempt=<k>`; `the launcher's readiness pipe open failed error=<n> attempt=<k>`; `the readiness message write failed`; `readiness reported to the launcher` (each after `[MitRuntime] [PlayerHost] `) |
| refused listen | `[MitRuntime] [PlayerHost] bind address is not loopback; refusing listen` |

The install line is R0's: `install HTTP-1@R10 ok|<reason>|skipped (not a player host)`.

---

## 16. Tests

### 16.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`), every platform:

- Bootstrap: a valid map (with the shapes `rustic-rifts-host` writes: a 36-character hyphenated session id, `host-`
  plus 32 hex as launch id, `\\.\pipe\RusticInternalServer.ready.` plus 32 hex) is `Valid`; then each of the 11 checks
  failing alone names its label: manual launch, `argv 1` = `x`, no `MULTIHOME` and `-multihome=127.0.0.1` (accepted:
  case), bind `127.0.0.2`, session of 35 and 37 characters and one with `/`, generation empty, `12345678901` and `1a`,
  capability of 42 and 44, a pipe under `MysticInternalServer.` (refused), a pipe suffix of 15 and 97 characters and
  one with `\`, launch id of 15, pid `0`, `4294967296` and `4294967295` (accepted).
- Role: absent (HUNT); `HUB`; `hub` (not valid); empty (malformed); `HUNTER` (not valid); 9 characters
  (malformed); `MAX_PLAYERS` `0`, `1`, `64`, `65`, `012`, `+5`, `123456` (malformed: over 5); `HUB_AFK_SECONDS` `0`,
  `7200`, `7201`; a control character (malformed); the numbers set with `HUNT` (valid, ignored); each problem text
  byte for byte; a bootstrap failure and a role problem reported together.
- The three messages byte for byte for a fixed session, generation `7`, launch id and port `8791`; `is_fence` refuses
  a trailing CR, a missing LF, another generation, `MYS1`, and 512 bytes.
- The role lines and the Hunt Guard line for each case.
- The registry with `"R10"` in `LANDED`: `owns` true for the three ids; the rule "R10 implies R9 and R2".
- R9's `ready_lines` gives one line (12.1).
- The alias module is gone: no `MYSTICPARADOX_` text in the runtime crates (a test greps its own sources, or the
  integration pass's grep in 17.4 stands in).

DLL (`cargo test -p rustic-isles-runtime`, Windows):

- The control thread against a test client in the same process with the bootstrap's pid set to the test's own: the
  exact fence gets the exact ack and sets the flag; a malformed message, and a client whose pid is not the expected
  one (set the expected pid to another number), set the flag and get no ack. The pipe's DACL holds two entries.
- `report_ready` against a pipe server the test creates: the message arrives; a pipe that never exists answers false
  after the wait; a busy single instance is retried.
- The HTTP-1 wrapper's decision over a fake request: header set under the API base, not set for another host, not set
  in flight. (The `SetHeader` call goes through a function pointer the test can set, as R2's tests do.)

**Never in CI:** a real launcher, the engine, injection, a hosted hunt, patch 0055 (it needs the host's SDK to build).

### 16.2 The maintainer's play-test (collected for the one hands-on pass, rulings "Order, amended")

Surface row R10: "Host a realm hub and a hosted hunt; the launcher fences it". It needs 12.2's spawner row; until that
lands, steps 2 to 5 cannot run from the MIT launcher. Nothing waits on it.

1. Build the AGPL runtime with 0041, 0047, 0052 and 0055, the MIT DLL with `"R10"` (and R2, R4, R9) in `LANDED`, and
   a launcher with 12.2's rows. Inject both DLLs as R0 11.1 says.
2. **A realm hub.** Host one from the launcher. In the hub's `rustic_isles_dll_port<port>.log`: `install HTTP-1@R10 ok`;
   `[HostedRole] launcher role=HUB (persistent hub) maxPlayers=…`; `control pipe worker started`; the AGPL's
   `[MitSeam] SP-3@R10 is the MIT runtime's: …`; R9's `engine MULTIHOME matches the pinned listen address
   127.0.0.1`; `readiness reported to the launcher`; `listening on port …`. No `MYSTICPARADOX` anywhere in the log or
   on stdout. The launcher shows the hub ready; a second client joins through the realm; the backend's log shows the
   host's requests authenticated by `x-rustic-host-capability`.
3. Leave the hub empty for two minutes: it stays up (R8's hub exemption still reads the AGPL's role answer until R8).
4. **A hosted hunt.** Start one; `[HostedRole] launcher role=HUNT (disposable)`; finish it and return.
5. **The fence.** Stop the realm session from the launcher: `generation fenced by the launcher`, then `applying the
   fence on the game thread; ending the process`; the process exits with code 0 within a second.
6. **Refusals.** Start the hosted server by hand with one launch variable wrong (for example
   `RUSTIC_ISLES_BIND_ADDRESS=0.0.0.0`): it exits with `0xE103` and the log names `RUSTIC_ISLES_BIND_ADDRESS`, never
   its value. From another user account on the same PC, try to open the control pipe: access is denied and the host
   keeps running.
7. **A DeployServer hub** (Director): its stdout carries only `RUSTIC_ISLES_GAMESERVER_READY …` and the Director
   marks it ready.
8. **Regression without R10.** An MIT DLL without `"R10"` and the AGPL with 0055: the AGPL hosts as before, with the
   new names on its pipes and header.
9. Note anything that differs. Only CI passed until these steps are done.

---

## 17. Calls, departures and ambiguities

### 17.1 RT-8 and RT-12 to RT-18 for R10

- **RT-8:** settled here (4, 12): `RIS1`, `RusticInternalServer.`, the legacy stdout line and the alias removed, the
  launch variables `RUSTIC_ISLES_`.
- **RT-14:** R10 reads no flag file. `SERVER_QUITGAME_EXIT` (R8's) does not apply to the fence (7.3).
- **RT-17:** the SP-3 handler's order is `-2000` (6); the HTTP-1 wrapper's `100` (9). Nothing else depends on order.
- **RT-18:** R10 registers no `ProcessEvent` handler (10).
- RT-12, RT-13, RT-15 and RT-16 are not R10's.

### 17.2 Calls

1. **`RIS1`** for `MYS1` (4). Alternative: D3's mechanical `RYS1`.
2. **The hosted server's launch variables are `RUSTIC_ISLES_<NAME>`**, and `rustic-rifts-host` changes its prefix
   (4, 12.2). **Needs the maintainer's ruling**: it amends how `rustic-rifts-host`'s clean room applied D3. The
   alternative (the runtime reads `RUSTIC_RIFTS_` for the player host) splits one process's variables across two
   prefixes keyed on who started it.
3. R10 lands with or after R9 and R2, enforced by a registry test (1).
4. The SP-3 handler at order `-2000`, the HTTP-1 wrapper at `100` (6, 9).
5. `SP-2@R10` is owned with no handler (10).
6. The fence always ends the process with exit 0; no QuitGame option (7.3).
7. `STRIPPED_PREFIXES` keeps `MYSTICPARADOX_` (12.2).
8. Patch 0055 renames the AGPL's wire whether or not the MIT DLL is present, so every situation of section 3 speaks one
   protocol (13.2).
9. The bootstrap is decided in Init and acted on at the first frame (5.1, 6).

### 17.3 Departures

1. The control pipe's DACL is this user and `LocalSystem` only (7.1).
2. The ready pipe is opened with identification-only impersonation (8.2).
3. The fence's 5 s fallback runs on the control thread, so a hung game thread still ends (7.1).
4. The invalid-bootstrap line names the failed check (6).
5. The bootstrap is read once; no second read at listen time (5.2).
6. The control thread starts on the first engine frame, not from a thread started in `DllMain` (6).
7. The capability header only under the API base (9), as ruled for R2's call 4.
8. The Hunt Guard line is logged only by a player host (the AGPL logged it on every server).

### 17.4 Ambiguities

- **Hosting under Proton.** The control pipe's client check needs the launcher's Windows pid; a native Linux launcher
  can neither open a Wine pipe nor have such a pid. Realm hosting is Windows-only today; a Proton host needs the
  launcher's pipe ends inside the Wine prefix (the inject helper's process is one candidate). Not settled here.
- **The launcher's pipe ends** (12.2) were in the retired Paradox launcher, which is not in this tree. Their shape
  above is what the runtime needs, not a copy of that launcher; the spawner row settles the rest (timeouts).
- **The 5 s fallback and a fence that arrives before the first frame.** The flag is set; the control thread's fallback
  ends the process 5 s later even though no frame ran. That is the intent (the launcher asked for the end).

Integration-pass check after the cutover: outside `restricted/` and `docs/port/`, this finds nothing:

```bash
git grep -n -e MYS1 -e MysticInternalServer -e MYSTICPARADOX_GAMESERVER_READY -e x-mystic-host-capability
```

and `git grep -n MYSTICPARADOX_ -- 'crates/rustic-isles-runtime*'` finds nothing.

---

## 18. Size

Rust, without tests: the core crate about 300 lines (bootstrap and role checks 160, messages and fence comparison 40,
role and log texts 60, names 40); the DLL crate about 450 (registration, verdict and SP-3 handler 90, control thread and
DACL 150, ready pipe 80, HTTP-1 wrapper 70, hosted role and R9 interface 60). The cutover deletes about 80 (the alias
and its tests, the legacy line). Tests about 450. Patch 0055: about 30 lines of C++.
