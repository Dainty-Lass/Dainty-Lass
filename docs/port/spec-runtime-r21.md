<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R21, retiring the AGPL runtime (RD27, plan item 11)

R21 is the last slice of the MIT game runtime (`spec-runtime-surface.md` section 8, R21's row). After it the game loads
only `rustic-isles-runtime.dll`, the MIT runtime built from `crates/rustic-isles-runtime`. `Rustic-Isles.dll`, the
AGPL C++ runtime in `restricted/modules/rustic-isles/native/runtime`, is no longer built, shipped or injected, and the
coexistence machinery R0 built for two DLLs in one process (the seam exports, the broker's AGPL band, the per-id
ownership answer) goes with it.

This page has five parts: an audit of everything the AGPL runtime does against the MIT slices (section 2, the part
that decides whether R21 is safe); the one behaviour the audit found missing and its spec (section 3); the changes
inside the MIT runtime (section 4); the rows outside it, in the MIT tree (section 5) and in `restricted/` (section 6);
and the order, the rollback and the one hands-on pass (sections 7 to 9).

An implementer builds the MIT parts from this page, the earlier slice specs and the surface page alone, and never
reads `restricted/`. What crossed is facts: engine layout offsets, flag and variable names, the exact log text another
program reads, and behaviour. Section 6 is a list for a restricted session, which may read the AGPL tree.

Conventions are R0's: an RVA is `+0x0XXXXXXX` from the executable's image base; **C** client, **S** server, **B**
both. **(call)** marks a decision this page makes, **(ambiguity)** something the source did not settle,
**(departure)** a deliberate difference from the AGPL runtime; all are collected in section 10.

---

## 1. Scope

| R21 owns | Section |
| --- | --- |
| The coverage audit: every hook, patch, handler, thread, flag, variable, file and log line of the AGPL runtime, and who carries it now | 2 |
| `SP-3@R21`: the connection-count edge line the Director's occupancy reads (the audit's one gap) | 3 |
| The MIT runtime alone: the registry's last state, the seam exports and the broker's AGPL band removed, the AGPL-only paths in R9, R20 and R0's status line | 4 |
| The rows outside the runtime crates: protocol, content feed, auth, launcher, Director, host kit, realms, loader, docs, catalogs | 5 |
| The list for a restricted session | 6 |
| Landing order, release phases and rollback | 7 |
| R21's play-test and the collected checklist of every slice's play-test | 8, 9 |

Not R21: any game feature (each was its slice's); R19's diagnostics (RT-3: never ported); the C++ CatalogExporter and
the MIT exporter (`native/rustic-isles-exporter`, lane E), except where the AGPL runtime compiled part of the former
(2.12); the loader's code (section 5 finds it needs none).

**Dependencies.** Every slice R0–R18 and R20 in `LANDED`. R14 is merged on `plan` (`LANDED` now ends with `"R14"`),
so the MIT side of R21 can start. Section 7 says what must ship together.

---

## 2. The coverage audit

### 2.1 Method

The spec writer walked, in the AGPL source at `plan` of 2026-11-13 (after patches 0041–0059):

1. every hook creation (82 detour sites and the export-hook helper) and whether its install path runs on 1.14.7;
2. the seam's own table of targets to ids (patch 0041), which is the AGPL's answer to "which id is this hook";
3. every ownership gate in the dispatch bodies (74 sites over 27 ids) and every block **not** behind one, in the
   server tick, the server and client `ProcessEvent` bodies and the HTTP detour;
4. Init, `DllMain` and MainThread step by step;
5. every thread start, vectored handler, code or data write, process setting and file read or write;
6. every flag file name the source contains (137) against the slice specs, and every environment read;
7. which of its log lines another program parses.

Appendix A of the surface page was checked against step 6: it missed no live flag (2.8).

Each row below says where the behaviour lives now: **Rn** (the MIT slice that owns it, already merged), **dropped**
(by the ruling or slice call named), **gone** (dead or never installed on 1.14.7 before R21), or **missing** (a gap).

### 2.2 Hooks (surface section 3)

| Ids | AGPL install on 1.14.7 | Now |
| --- | --- | --- |
| SP-1, SP-2, SP-3 | yes | R0 (chains); every gated block on them by its slice (2.4) |
| HTTP-1 | yes | R2 (chain, `HTTP-1@R2`), R10 (`HTTP-1@R10`) |
| CL-1, CL-2, CL-5, CL-7, CL-8, CL-9 | yes | R4 |
| CL-3, CL-4, CL-6 | yes (client, logging only) | dropped: RT-3, R4 section 2 (registry slice `R19`) |
| CL-10..13 | flag-gated (R4's RT-16 table) | dropped: RT-3, R4 section 2 |
| EXIT-1..14 | yes | R3 (EXIT-12's spline recovery rule: R11) |
| HL-1..11, HL-12 (locale) | yes / opt-in | R5 |
| UI-1..13 | yes | R6 |
| LD-1..9 | yes (LD-4, LD-8, LD-9 log only) | R7 (R7 installs the three as skipped: RT-3) |
| NETD-1..8 | yes / opt-in | R11 |
| RG-1, RG-4..7 | yes | R12 |
| RG-2 | yes | R13 |
| RG-3, PR-1 | yes | R14 |
| ESC-1 | yes | R18 |
| SRV-1 | yes | R9 |
| TR-1 | yes | R16 |
| DX-1..9 | yes / opt-in | dropped: RT-3 (R19 not ported) |
| CON-1 (the F2 console, MainThread) | yes | R17 |
| Surface 3.12 (about 37 defined-but-never-installed detours and probes) | no: commented-out installs, `if (false)` blocks, `#if 0` | gone |

The seam's target table names exactly the ids above, with UI-3 at the function start `+0x01E54110` (surface:
pdata(`+0x01E5414E`)). Every detour site whose target is not in that table belongs to a never-installed probe
(the six progression probes, the active Hunt Pass driver and its window lookup, the two Tempest scale probes behind
`if (false)`, the XMPP redirect) except the web-browser skip, which is HL-4 under another call shape. **No installed
hook is unowned.**

The controller watchpoint's vectored handler (`ENABLE_CONTROLLER_WATCHPOINT.flag`, a debug-register diagnostic) is
R14's flag and was dropped there.

### 2.3 Code patches, data writes and process settings

| What | Now |
| --- | --- |
| `GIsServer` = 1, `GIsClient` = 0 at Init, before each engine tick and again straight after it | R5 (HL-1: Init write and the SP-3 before and after handlers; R5 section 4 already describes the order once the AGPL is gone) |
| Boot patch at `+0x03E07353` (HL-2), RHI branch at `+0x034AB73D` (HL-9), crash-wait shorten at `+0x0265CF15` (EXIT-14), EAC config byte (CL-7) | R5, R5, R3, R4 |
| Process locale `C` (HL-12) | R5 |
| Hub idle working-set trim, the out-of-memory reserve release | R8 (7.6, 7.7) |
| The 1.4.4 in-code `GIsServer` reinforcement patches | gone (`#if 0`) |
| Supply stack caps (0024), cell fusion time (0029), loadout repair (0027, 0037) | R15 |
| Trials schedule rows (ESC-1) | R18 |
| Bleedout floor, debug damage multiplier, manual stamina tick | R20 |
| Viewport size swap (HL-11), retried from the tick | R5 |

### 2.4 Gated blocks on the dispatch points

Every block behind an ownership gate is, by construction of patch 0041, owned by the slice whose id gates it, and each
of those slices is in `LANDED`: `SP-1@R14`, `@R15`, `@R16`, `@R17`; `SP-2@R5`, `@R7`, `@R8`, `@R10`, `@R14`, `@R15`,
`@R18`, `@R20`; `SP-3@R7`, `@R8`, `@R9`, `@R10`, `@R11`, `@R12`, `@R13`, `@R14`, `@R18`, `@R20`; `HTTP-1@R2`,
`@R10`; and the non-chain gates `HL-1`, `HL-2`, `HL-3`, `HL-9`, `HL-11`, `HL-12`, `CL-1`, `CL-7`, `EXIT-12`,
`EXIT-14`, `CON-1`. **No gated block is unowned.**

### 2.5 Ungated blocks on the dispatch points

These run in the AGPL whichever DLL owns what. Each was read in full.

| Where | Block | Now |
| --- | --- | --- |
| SP-3, before the tick | frame-time sample for the `[Perf]` line; the first-tick banner | dropped (RT-3); R8 has its own `[Perf]` (7.8) |
| SP-3 | records the game-tick thread id for the exit hooks' "which thread" field | R3 keeps its own |
| SP-3 | adopting R9's listen and the HL-11 retry call | seam plumbing (0052) / R5 |
| SP-3 | hub replication membership report (`ENABLE_HUB_REP_DIAGNOSTICS.flag`) | dropped: R13 section 10 |
| SP-3 | arrival-trigger server probe, Hunt Pass server state, Trials bleedout sampler, Tempest charge sampler | dropped: RT-3 (R7, R18, R14 name them) |
| SP-3 | the client-only F7 object dump (never runs: SP-3 is server-only) | gone |
| SP-3, listening | match-state change trace, send-path probe, net-tick traces, `[NetDiag]` once, the edge recovery probe, the tick-filter census | dropped: RT-3 |
| SP-3, listening | **the connection-count edge line `[NetConnEdge]`** | **missing: read by the Director (2.11). Section 3 closes it** |
| SP-3, listening | the bleedout watch (reports a bleedout that outlived any window) | dropped: RT-3 (log only) |
| SP-2 | `ClientReturnToMainMenu` / `ClientWasKicked` caller log; economy trace; Behemoth kill and XP traces; perfect-dodge trace; bleedout event note; the hang ring (last function and object on the game thread); the core-consumption capture (`CORE_CAPTURE.flag`); the post-login timing anchor and per-controller trace; possession trace; `MustSpectate` return log; drone and Tempest damage traces; escalation flow trace | dropped: RT-3 (R20 section 1 lists the post-login, possession and `MustSpectate` ones) |
| SP-2 | the old `OnPostMitDealtAnyDamage` forwarding | gone (commented out) |
| SP-1 | the F9 object dump | gone (commented out) |
| SP-1 | network and travel failure lines (`[ClientNetworkFailure #n]`, `[ClientTravelFailure #n]`); HUD trace; weapon XP bar and grant traces; perfect-dodge trace; player-role input diagnostics; accessory unlock diagnostic; movement input, movement mode traces; inventory trace (0026); escalation flow, relic choice screen and relic click diagnostics; Gauntlet census; travel cause recorder | dropped: RT-3 (R16 7, R15 6.6, R18 5, R14 name them) |
| HTTP-1 | nothing ungated | — |

### 2.6 Init, `DllMain` and MainThread

| Step | Now |
| --- | --- |
| `DllMain`: thread notifications off; detaching flag on detach | R0 (3.1, 3.3) |
| Executable folder, flag-cache mode, backend address | R0 (4.2), R1 (4) |
| Role from `-server`, image base | R0 (3.2) |
| Auth mode (`RUSTIC_ISLES_AUTH_MODE`), invalid ends the server with `0xE102` | R1 (5), R9 |
| Launch arguments, manual test mode, watchdog switch | R1 (6), R8 |
| Hook installs by role | R0 7.6 and each slice |
| MainThread: session banner, flag-cache line, backend line | R0 (4.3) |
| MainThread: player host start (pipes, fence) | R10 |
| MainThread: client store capture worker (`REWARD_CACHE_CAPTURE.flag`) | dropped: RT-10 (the MIT exporter's store runtime export, `spec-catalog-exporter.md` 6.13) |
| MainThread: server world wait (30 s), then "listen requested" | R9 |
| MainThread: client world wait, then the F2 console after 3 s | R17 (`CON-1`) |
| MainThread: the boot patch status line | R5 |

### 2.7 Threads

| AGPL thread | Now |
| --- | --- |
| MainThread | R9 (server), R17 (client) |
| Empty-server watchdog (started at listen) | R8 |
| Server window hider | R5 |
| Player-host control pipe | R10 |
| Client hitch sampler (`HITCH_SAMPLER.flag`) | dropped: RT-3 |
| Store capture worker | dropped: RT-10 |

### 2.8 Flags

The source contains 137 distinct `*.flag` names. 124 are in a slice spec (kept, dropped by that spec's RT-14 call,
moved to R19, or Lane E's). The other 13 appear **only in comments**: no code reads them (an experiment removed, a
gate made unconditional, or a name from an older build): `CALL_HUNTPASS_QUERY`, `DRIVER_WRITE_WATCH`,
`ENABLE_GRAPH_DRIVE`, `FORCE_NETUPDATE`, `HEADLESS_AUDIO_DISABLE`, `NATIVE_PLAYERSTART`, `NO_LEVELINIT_FORCE`,
`REPGRAPH_SELF_CONSTRUCT`, `SUPPRESS_EAC_POPUP`, `THROTTLE_REP`, `URL_REWRITE`, `USE_ARCHON_REPGRAPH`,
`XMPP_REDIRECT`. Gone. Every flag of R19's list in Appendix A goes with the AGPL DLL (RT-3).

So after R21 a flag file the AGPL read and no MIT slice kept does nothing. The hosting docs must not list one
(section 5, row D-4).

### 2.9 Environment

| Variable | Now |
| --- | --- |
| `RUSTIC_ISLES_RUNTIME_PROFILE`, `RUSTIC_ISLES_BACKEND_URL`, `DAINTY_BACKEND_URL`, `RUSTIC_ISLES_AUTH_MODE`, `RUSTIC_ISLES_GAMESERVER_LAUNCH_ID` | R0, R1, R9 |
| `DAINTY_SERVER_MAX_FPS`, `DAINTY_SERVER_IDLE_FPS` | R8 |
| `RUSTIC_ISLES_TRIALS_WEEK`, `TRIALS_ROTATION_MINUTES` | R1 (9), R18 |
| `CELL_FUSION_MINUTES` | R15, renamed `DAINTY_CELL_FUSION_MINUTES` (R15 departure 2) |
| `RUSTIC_ISLES_BLEEDOUT_SECONDS`, `RUSTIC_ISLES_ALLOW_DAMAGE_DEBUG`, `RUSTIC_ISLES_DEBUG_DAMAGE_MULTIPLIER` | R20 |
| The player host's set (surface 5.2) | R10 |
| `MYSTICPARADOX_` alias | renamed by RT-8 (R1, R10) |

No variable the AGPL reads is unowned.

### 2.10 Files

| File | Now |
| --- | --- |
| `rustic_isles_dll_port<N>.log` (written) | R0 (4.3): the same name and line format; after R21 only the MIT DLL writes it |
| `HUB_MAX_PLAYERS.txt`, `HUB_AFK_TIMEOUT_SECONDS.txt`, `HUNT_AFK_TIMEOUT_SECONDS.txt` (read) | R8, as `settings.toml` settings (RT-11) |
| `tickfilter_script_port<N>_<k>.bin` (DX-8/9's dump) | dropped: RT-3 |
| The flag files | 2.8 |

### 2.11 Log lines other programs read

| Line | Reader | Now |
| --- | --- | --- |
| `RUSTIC_ISLES_GAMESERVER_READY launchId=<id> port=<n>` on standard output | the Director's readiness wait | R9 |
| `RIS1 READY ...` on the ready pipe | the launcher (realms) | R10 |
| `[NetConnEdge] tick=<n> connCount <a> -> <b> ...` in the per-port log | the Director's Training Grounds occupancy (`crates/rustic-isles-director/src/occupancy.rs`, `spec-rustic-isles-director.md` 7.4) | **missing** |
| The whole per-port log file, uploaded by name | the launcher's log upload (`runtime-rustic_isles_dll_port<digits>.log`) | R0: the name is unchanged |

The Director's other inputs from a game server are its exit code and the READY line. No other parser of runtime log
text was found in the MIT tree (`crates/`, `ui/`, `launcher/`, `native/`).

**Consequence of the gap.** With the AGPL gone, the Training Grounds' remote-client count is "unknown" forever, and an
unknown count never stops the server (director spec 7.3). The on-demand Training Grounds (`training_on_demand`, on
by default, `training_idle_minutes` 5) would then run until the next restart. Nothing crashes; a host loses the
memory and CPU of an idle server. Section 3 ports the line.

### 2.12 Build-time parts

| Part | Now |
| --- | --- |
| The store runtime exporter compiled into the runtime | dropped: RT-10 |
| The embedded login background image | dropped: RT-9 |
| `RUSTIC_ISLES_INTERNAL_VERSION` | settled in RT-13's slice (unused) |
| `tools/compat-inventory.ps1` | not ported (a dev tool) |
| MinHook | R0 (vendored from upstream v1.3.3) |
| The player-host anti-cheat stub (no evidence produced) | R10 (none either) |

### 2.13 Result

One gap, **G-1**, the `[NetConnEdge]` line (section 3). Everything else the AGPL runtime does on 1.14.7 is owned by a
merged MIT slice, dropped by a named ruling or slice call, or was already dead. No call for the maintainer comes out
of the audit beyond section 10's.

---

## 3. G-1: the connection-count edge line (`SP-3@R21`, S)

### 3.1 Why

The Director follows each Training Grounds server's per-port log for lines that report a change in the server's
remote-client count, and stops an on-demand Training Grounds that has had no player for its idle time. Only the AGPL
writes the line today, as a trace no seam gate covers.

### 3.2 The handler

An SP-3 **after** handler, `order = 1900` **(call: no other handler depends on it)**, server role only, registered
under the new feature id `SP-3@R21`.

State, game thread only: `frame`, a counter of SP-3 calls in this process (the first engine tick is 1; R21 counts
its own if no earlier slice exposes one), and `last`, the last reported count, initially **-2**.

Each call:

1. `frame += 1`.
2. If R9's `state()` is `None` (not listening yet), return. The AGPL reports only once it has listened, on both its
   listen paths.
3. `driver` = R9's `net_driver()`. `count` = the driver's `ClientConnections` `Num` (`i32` at driver `+0x98`) and
   `max` = its `Max` (`i32` at `+0x9C`), both read with R0's checked read of 8 bytes at `+0x98`; when there is no
   driver or the read fails, `count` = `max` = **-1**.
4. If `count == last`, return.
5. Write one line with R0's ordinary prefix and **no** `[MitRuntime]` tag (call: the text is an interface, kept
   exactly):

   ```text
   [NetConnEdge] tick=<frame> connCount <last> -> <count> connMax=<max> NetDriver=<driver>
   ```

   every number in decimal (`-2` and `-1` with their sign); `<driver>` the driver's address in **decimal**, or `null`.
   Then `last = count`.

Not gated by `PRODUCTION_LOG_PROFILE` or `verbose_diag()`: the Director needs it on every server. One line per change,
so a busy hub writes one per join or leave.

**(departure)** The AGPL also took a lifecycle snapshot at each edge (off under `PRODUCTION_LOG_PROFILE`): logging only
(RT-3), not ported.

Example, a player joining and leaving:

```text
[12:00:01.234] pid=292 [NetConnEdge] tick=40 connCount -2 -> 0 connMax=0 NetDriver=2730612336
[12:00:31.020] pid=292 [NetConnEdge] tick=941 connCount 0 -> 1 connMax=4 NetDriver=2730612336
[12:05:02.511] pid=292 [NetConnEdge] tick=9077 connCount 1 -> 0 connMax=4 NetDriver=2730612336
```

The Director's rule (director spec 7.4) reads the second integer as the count, a negative one as unknown; the first
line above makes the count known (0) as soon as the server listens.

### 3.3 Where the code goes

`crates/rustic-isles-runtime-core/src/occupancy.rs` (pure: the edge decision from `(last, count)` and the line's
text) and `crates/rustic-isles-runtime/src/occupancy.rs` (registration and the read). Registry: the line
`e("SP-3@R21", Feature, "R21")` and `"R21"` in `LANDED` (4.1). The integration pass adds the line to the migration
guide's section 2.

---

## 4. What changes in the MIT runtime

### 4.1 The registry (R0 7.5)

- `LANDED` gains `"R21"`. With it, every slice but R19 is listed. The registry stays: it is the install table every
  slice reads through `owns(id)` (installs, handler registration, R9's and R10's decisions), and the status line's
  count. Only its outside answer goes (4.2).
- **The R21 gate (a core test).** `LANDED` holds every slice named in `ENTRIES` except `R19`, and contains `R21`.
  Every id whose slice is not `R19` is owned. This is the check that nothing the AGPL used to do is left to a DLL that
  is no longer loaded; it fails the build if a slice is removed from `LANDED` again.
- The `R19` lines (DX-1..9, CL-3, CL-4, CL-6, CL-10..13) stay, never owned, so the "every surface id exactly once"
  table test keeps holding **(call)**.
- The registry tests that branch on "slice X not landed" keep compiling; the branches for "not landed" become dead
  but harmless. The implementer may delete them.

### 4.2 The seam exports go (call)

`rir_seam_version`, `rir_owns`, `rir_chain_add_inner`, `rir_chain_remove_inner`, `rir_original` (R0 7.3) and
`rir_listen_driver` (R9 9.2) are **removed** from the DLL, with `SEAM_VERSION`.

Why remove rather than keep as no-ops: a stale `Rustic-Isles.dll` with patch 0041 that is still loaded beside the R21
DLL (a player's folder the launcher did not clean, a host's hand-made folder) finds the MIT DLL by name, fails to
resolve its exports and ends the game at start with exit code `0xE10A`, before any hook. That is fail-closed and loud.
Kept exports answering "owned" for everything would let it load and run its R19 hooks and ungated blocks silently in a
build that is supposed to carry no AGPL code. Section 5 makes the launcher and the Director remove or refuse such a
file, so a player should never meet `0xE10A`; the doctor and the play-test check it (8, step 9).

`rustic-isles-runtime.dll` then exports nothing but what Rust's `cdylib` adds. **(call)** No replacement marker
export: nothing outside the DLL needs to ask it anything.

### 4.3 The broker loses its AGPL band

The broker stays: several MIT slices put wrappers on one target (SP-1/2/3, HTTP-1) and the chain lets them do so
with one MinHook hook. What goes:

- `Band::Agpl`, `add_agpl`, `remove_agpl`, `has_agpl` in the core's chain model, and `add_inner`, `remove_inner` in
  `ffi::broker`; the chain is MIT wrappers in `order`, then the original. `original(T)` is unchanged.
- The chain-model tests about the AGPL band; the DLL test that drives the exports (R0 10.1, "Broker on real code")
  is rewritten with MIT wrappers only (two orders, one absorbing).
- The MinHook status numbers the exports returned (`3`, `4`) have no caller any more; the internal error type may
  keep them.

### 4.4 R20's stamina skip

`broker::chained_by_agpl` goes. With no AGPL band it is always false, so R20's skip (`SERVER_TICK_FILTER.flag` **and**
an AGPL wrapper on DX-8's target) can never be taken: the manual `TickStamina` runs every frame for connected players,
as R20 10.3 already does whenever the AGPL is absent. Changes:

- `stamina_skip` and the skip's log text go; R20 logs only `[Stamina] manual TickStamina each frame for connected
  players` once.
- R20 stops reading `SERVER_TICK_FILTER.flag` (it was R19's flag; R20 read it only for the skip). **(call)**

### 4.5 R9's listen decision

`Decision::AgplListens` and its line (`player host without R10 in this build: the AGPL runtime listens (patch 0052)`)
go. `listen_decision` answers `Mit` for a DeployServer and a player host, and `None` for an invalid auth mode, which
ends the process with `0xE102` as today. With `R10` landed this is already what happens; the change removes a
branch that can no longer be reached and its tests.

### 4.6 The status line and the tags

- R0 4.3's first line drops `seam=<n>`: `[MitRuntime] rustic-isles-runtime <crate version> role=client|server
  port=<n> owns=<count> ids` **(call: `owns=` kept, so earlier play-test notes still read)**.
- The `[MitRuntime]`, `[Dispatch]`, `[Broker]` and `[Panic]` tags stay (call). They told two DLLs apart; they cost
  nothing alone, and every earlier slice's play-test greps them.
- No MIT line names the AGPL any more after 4.4 and 4.5. Doc comments that describe the AGPL side ("the AGPL
  runtime leaves alone", "loaded before `Rustic-Isles.dll`") are tightened to the MIT-only behaviour in the same
  change; comments that cite a patch number as history may stay.

### 4.7 Load order and lifecycle (R0 3.1, 3.3, 7.6, 9)

- The DLL is the only runtime injected (or loaded by the Proton loader's fixed-name rule). Its `DllMain` rules do
  not change: it still runs under the loader lock before the engine starts.
- R0 3.3's note that `Rustic-Isles.dll` detaches first becomes moot; the detaching rule itself stays.
- R0 7.6's install order is unchanged: it never depended on the AGPL.
- R5's HL-1 after-tick write: R5 section 4 already says that once every SP-3 slice is MIT its after handler is the
  first thing after the engine tick, as today. Nothing to change; the play-test checks it (8, step 3).

### 4.8 Tests that change

| Test | Change |
| --- | --- |
| R0's export test (`tests/dll_file.rs`: the exports exist by name, `rir_seam_version() == 1`, `rir_listen_driver`) | inverted: none of the six names is in the export table |
| R0's broker test with an "AGPL" wrapper (`ffi/tests.rs`) | MIT wrappers only (4.3) |
| Chain-model AGPL band tests (`chain.rs`) | removed |
| R20's `chained_by_agpl_reads_the_broker` | removed |
| R9's `AgplListens` cases | removed; the player host case answers `Mit` |
| Registry: owns-by-`LANDED` branches | the R21 gate (4.1) added |
| R0 4.3's status line test | without `seam=` |
| New | section 11 |

---

## 5. Outside the runtime: the MIT tree

The set of runtime files after R21: **`rustic-isles-runtime.dll`** (required) and `winmm.dll` (the Proton loader,
optional as today). `Rustic-Isles.dll` and its pre-DX4 aliases are **retired names**: never published, never
injected, and removed when found where the launcher or the host kit keeps runtime files.

**The name (call, Q-1 in section 10).** This page keeps the MIT DLL's name (RT-5) and moves every consumer to it. The
alternative, publishing the MIT build under the old name `Rustic-Isles.dll`, would change no consumer and no route; it
is weighed in section 10.

Rows marked **seam** touch a file AGENTS.md reserves (the integration pass makes or approves them). Rows marked
**parity** change an answer that a recorded Node reply or golden covers, so they need the decision row of call 1 and a
re-record, said in the commit.

### 5.1 Protocol, feed and auth

| # | Where | Change |
| --- | --- | --- |
| P-1 | `crates/dainty-protocol/src/manifest.rs` (**seam**) | `REQUIRED_RUNTIME_FILES = [MIT_RUNTIME_DLL]`; `RUNTIME_FILE_NAMES = [MIT_RUNTIME_DLL, LOADER_DLL]` (sorted order as today's sort makes it); `RUNTIME_DLL` (`Rustic-Isles.dll`) and `LEGACY_RUNTIME_DLL` move to a `RETIRED_RUNTIME_DLLS` list (with `MystPaxInternalServer.dll`) that consumers remove and never serve; `runtime_file_slot` no longer maps the legacy name. Doc comments: the MIT DLL is the runtime |
| P-2 | `crates/dainty-protocol/tests/fixtures/runtime-manifest*.json`, `record.js` | the fixtures list `rustic-isles-runtime.dll`; the shared cases follow P-1 |
| P-3 | `crates/dainty-content/src/input.rs`, `runtime.rs` (**parity**) | a `runtime.json` is valid with `rustic-isles-runtime.dll` and without `Rustic-Isles.dll`; one that lists `Rustic-Isles.dll` (or an alias) is refused as an unknown file. The refusal texts follow the new names: `unknown file {shown} (only rustic-isles-runtime.dll and winmm.dll)`, `rustic-isles-runtime.dll is missing from the list`. The served index and file routes serve only P-1's names. The Node content server these replies were recorded from is not in the repository, so the changed cases in `crates/dainty-content/tests/parity/content*.json` are edited by hand, each citing call 1's decision row in the commit; every other recorded reply stays byte for byte |
| P-4 | `crates/rustic-isles-auth/src/runtime.rs` (**parity**) | `CORE_DLL = "rustic-isles-runtime.dll"`: the runtime manifest's top-level `sha256` is the MIT DLL's. `PROXY_DLL` unchanged. Tests and `tests/common` follow; the auth goldens in restricted's `rustic-isles-mit-parity` are re-recorded (6, R-9) |
| P-5 | `crates/rustic-isles-launcher-api/src/updates.rs` | nothing in code (it serves whatever the published manifest names); its publishing docs and tests that name `Rustic-Isles.dll` follow P-4 |
| P-6 | `fuzz/fuzz_targets/content_runtime_index.rs`, `runtime_input.rs`, `rustic_isles_auth_tokens.rs` | seeds and dictionaries use the new names |

### 5.2 Launcher

| # | Where | Change |
| --- | --- | --- |
| L-1 | `crates/dainty-launcher-kit/src/net/runtime_files.rs` | `INJECTED_RUNTIME_FILES = [MIT_RUNTIME_DLL]`; `OPTIONAL_RUNTIME_FILES` empty (the MIT DLL is required). After installing a set, remove every `RETIRED_RUNTIME_DLLS` file from `Win64` (today only the pre-DX4 name is removed). `sweep_aside` also sweeps the retired names' downloads and set-aside copies. **Accept both set shapes for one launcher release** (7.2): a pre-R21 set (the AGPL DLL required, the MIT optional) and an R21 set; inject what the offered set lists, MIT first, as today |
| L-2 | `crates/dainty-launcher-kit/src/controller/play.rs` (`runtime_dlls`) | inject `rustic-isles-runtime.dll` alone for an R21 set; the pre-R21 pair for a pre-R21 set (L-1's window). A local runtime folder follows the same rule |
| L-3 | `crates/dainty-launcher-kit/src/platform/loader_session.rs` | `RUNTIME_DLLS` (the loader's candidates) unchanged, since the loader's list does not change (L-6), but a candidate name found beside the loader when the offered set is an R21 set is a **leftover**: removed before PLAY like any retired file, and PLAY is refused if it cannot be removed (RD9's refusal, with its existing message). `OWN_RUNTIME_DLLS` keeps `MIT_RUNTIME_DLL` |
| L-4 | `crates/dainty-launcher-kit/src/platform/hostkit.rs`, `controller/host.rs`, the bridge's `build_runtime` action | the Host page's "build the runtime" builds the MIT DLL, or uses a prebuilt one (call 6): it no longer runs MSBuild on `Rustic-Isles.vcxproj` or needs the SDK for the runtime. `RUNTIME_FILE_NAMES` there follows P-1. The bridge spec is a **seam** (`restricted/launcher/src-tauri/src/bridge/spec.rs` generates the types): if the action's name or inputs change, the restricted session regenerates; if only its behaviour changes, no bridge change |
| L-5 | `ui/apps/web/messages/launcher/{en,pt-BR}.json` (key `...hint` at the "Compiles Rustic-Isles.dll against the SDK." entry), `ui/apps/web/messages/manager/{en,pt-BR}.json` (`runtime`, `build_runtime.hint`), `ui/packages/bridge/src/mock.ts` | new values in both languages in one commit, same keys (existing keys change value, so nothing is appended): launcher hint "Builds the Rustic Isles runtime (rustic-isles-runtime.dll)." / "Compila o runtime do Rustic Isles (rustic-isles-runtime.dll)."; manager `runtime` "rustic-isles-runtime.dll and its loader, which players install from your server." / "rustic-isles-runtime.dll e o loader dele, que os jogadores instalam a partir do seu servidor."; manager hint "rustic-isles-runtime.dll, with cargo." / "rustic-isles-runtime.dll, com o cargo." (the pt-BR is the spec writer's; flag it in the commit). If call 6 drops the build step, the strings say "downloaded with the release" instead |
| L-6 | `native/runtime-loader` | **no code change**: with only `rustic-isles-runtime.dll` beside it, the fixed-name rule loads it and the candidate choice finds nothing ("no candidate present: no runtime is loaded", loader spec 3). A stale candidate is L-3's to remove. `README.md` and the loader spec's section 3 note that after R21 the candidates are only for older sets |

### 5.3 Director and host kit

| # | Where | Change |
| --- | --- | --- |
| H-1 | `crates/rustic-isles-director/src/runtime.rs` | `published` injects `rustic-isles-runtime.dll` with its manifest hash, required; a `runtime.json` that lists `Rustic-Isles.dll` or lacks the MIT DLL refuses the start with a message naming the file (fail closed, as today for a bad hash). `RUNTIME_DLL` and the "MIT first, then the runtime" order go |
| H-2 | Director → Wine wrapper environment (`ENV_INJECT_DLL`, `ENV_INJECT_SHA256`, `ENV_INJECT_MIT_DLL`, `ENV_INJECT_MIT_SHA256` in `rustic-isles-director` and `crates/dainty-hostkit/src/wine.rs`) | `DAINTY_INJECT_DLL` / `DAINTY_INJECT_SHA256` carry the MIT DLL; the two `_MIT_` variables are no longer set, and the wrapper ignores them **(call)**. Internal variables, not `settings.toml` settings, so no host doc changes |
| H-3 | `crates/dainty-hostkit/src/paths.rs` | `RUNTIME_FILE_NAMES = ["rustic-isles-runtime.dll", "winmm.dll"]`; `runtime_dll` (the C++ build output under `native/runtime/x64/Release`) and `runtime_source` go or stop being used for the runtime (`runtime_source` may stay for the CatalogExporter's checks, if any read it); `mit_runtime_dll` is the runtime |
| H-4 | `crates/dainty-hostkit/src/publish.rs`, `commands/publish_runtime.rs` | `default_sources` names the MIT DLL (required: "is not built yet" when absent) and the loader (optional); `publish_runtime` removes `Rustic-Isles.dll` and the aliases from `.workspace/runtime` (as it removes the pre-DX4 name today); `--from` takes the MIT DLL under either its published or its build name, as today |
| H-5 | `crates/dainty-hostkit/src/commands/setup.rs` (auto-publish at setup), `commands/setup/paradox.rs` (the `Rustic-Isles.vcxproj` presence check) | setup auto-publishes when the MIT DLL is built (or shipped, call 6); the vcxproj check goes |
| H-6 | `crates/dainty-hostkit/src/doctor.rs` | the check that scans `Rustic-Isles.dll` for patches 0019 and 0020 goes **(call)**: the MIT runtime always carries R8's frame cap, and the CPU fixes are in its slices. A new check: a published `.workspace/runtime` or a game `Win64` folder holding a retired name is a warning with the fix "run publish-runtime" / "delete it" |
| H-7 | `crates/dainty-hostkit/src/selfupdate.rs` (`is_runtime_manifest_path`) | unchanged: it must still recognise an older release's manifest that names `Rustic-Isles.dll` |
| H-8 | `crates/rustic-isles-1-14-7/src/rifts.rs`, `crates/rustic-rifts-realms/src/host_policy.rs`, `hunt.rs`, `selection.rs` | the realm's `internal_server` hash is the runtime manifest's top-level `sha256`, which P-4 makes the MIT DLL's; nothing changes in code. `mit_runtime` (and Guard's `TRANSPORT_RUNTIME_MIT`) becomes redundant with it: keep the field (the wire keeps it optional and zero when unpinned) and stop documenting the role as needed **(call)**; a later row may remove it |
| H-9 | `crates/rustic-rifts-guard/src/lib.rs` (`GUARD_MANIFEST_ROLES`) | `TRANSPORT_RUNTIME_MIT` stays accepted (an older Guard manifest may list it); its doc comment says it pins the same file as the runtime manifest after R21 |
| H-10 | `crates/dainty-inject` | nothing: it loads the list it is given. Test file names may switch to the MIT name |
| H-11 | `crates/dainty-hostkit` `update` | nothing beyond H-4: `update` reruns setup, which publishes the MIT set and removes the retired files |

### 5.4 Build and release (the integration pass; workflows are RR3's)

| # | Where | Change |
| --- | --- | --- |
| B-1 | `.forgejo/workflows/ci.yml` | the runtime crates' Windows job (if it exists by then) builds the release DLL and runs R0's import and export checks; nothing builds the C++ runtime today, so nothing is removed |
| B-2 | `.forgejo/workflows/server-release.yml` (call 6) | a `windows-msvc` job builds `cargo build --release --locked --target x86_64-pc-windows-msvc -p rustic-isles-runtime` and the release carries `rustic_isles_runtime.dll` renamed `rustic-isles-runtime.dll`, with its SHA-256, beside the binaries; `publish-runtime` falls back to it when the host built none. No release workflow copies a runtime DLL today (RT-5's note), so nothing is removed |
| B-3 | `.forgejo/workflows/launcher-release.yml` | nothing (the launcher downloads the runtime from a server) |

### 5.5 Docs (rows write only their own pages; index pages are the integration pass's)

| # | Page | Change |
| --- | --- | --- |
| D-1 | `docs/launcher-injection.md`, `docs/director-injection.md` | the injected list is the MIT DLL alone; their hands-on checks name it (Process Explorer shows `rustic-isles-runtime.dll` and no `Rustic-Isles.dll`) |
| D-2 | `docs/security/game-runtime.md` | "In" loses the calls from the AGPL runtime through the seam exports; the boundary text loses "before `Rustic-Isles.dll`"; a new item: the DLL exports none of the six seam names (4.2); the R21 section with G-1 (the line carries no secret: counts and a pointer) |
| D-3 | `docs/security/content.md`, `docs/security/director-injection.md`, `docs/security/launcher-injection.md` | the served and injected names follow P-1, H-1, L-1; the leftover removal of L-3 is a checklist item |
| D-4 | `docs/sdk-and-game-data.md`, `docs/architecture.md` (section 4 "The runtime"), `docs/remote-management.md` | the runtime is `rustic-isles-runtime.dll`, built from the MIT crate (or shipped, call 6); the SDK is no longer needed for the runtime. Any flag listed for hosts that no MIT slice kept is deleted (2.8) |
| D-5 | `docs/port/migration-guide.md` section 4 (**integration pass**) | "A runtime patch written against Mystic Paradox's tree" now goes through a clean-room spec into the MIT runtime crates (a new or amended `spec-runtime-*.md`), never into `restricted/`; section 2 gains R21's module line (3.3) |
| D-6 | `docs/port/agpl-inventory.md`, `docs/port/spec-runtime-surface.md` | the integration pass marks the AGPL runtime retired (the surface page stays as the record of what was mapped) |
| D-7 | `README.md` (**integration pass**) | the runtime's name |

---

## 6. The restricted side (a list for a restricted session)

What stops being built or shipped, and what changes in `restricted/`, once section 7's phase B is out. Each row is
for a session that may read the AGPL tree.

| # | What | Change |
| --- | --- | --- |
| R-1 | `restricted/modules/rustic-isles/native/runtime/` (the C++ runtime: `dllmain.cpp`, `Networking.*`, `p2p/`, `MitSeam.*`, `BackendUrl.h`, `RuntimeShared.h`, `constants.h`, the project, solution and build scripts, `BUILD.md`, `assets/`, `Rustic-Isles.rc`, `resource.h`, `tools/`, its MinHook copy) | no longer built, published or injected. Kept or deleted: **call 2** |
| R-2 | `restricted/docs/paradox-patches.md` | the runtime rows (infra, 0004–0040, 0041–0059) become history: a note at the top of the section that the runtime retired at R21 and each row's behaviour now lives in the MIT slice the surface page's section 6 names. "Updating Mystic Paradox": an upstream runtime change goes through a clean-room spec (D-5), not a patch here |
| R-3 | `restricted/docs/paradox-runtime-patches.md` | "Applying a runtime patch" retires: replace its body with a pointer to the migration guide's section 4 (D-5); keep the loader paragraph's pointer to the loader spec |
| R-4 | `restricted/NOTICE.md`, `restricted/modules/rustic-isles/NOTICE.md`, `restricted/ADDITIONAL_TERMS.md` | the game runtime entries ("its game runtime (`ParadoxRuntime/`) ... in `modules/rustic-isles/native`", "The runtime includes MinHook and the Hacker Disassembler Engine 64") say the runtime was retired on the R21 date and, by call 2, where its source is (the tag, or still in the tree as an unbuilt oracle). The CatalogExporter's entry stays. `ADDITIONAL_TERMS.md`'s list "(its backend, Director, game runtime, ...)" follows |
| R-5 | `restricted/docker/Dockerfile` (it copies "the runtime and CatalogExporter sources setup checks") | stop copying the runtime source (H-5 removes the check); the CatalogExporter part stays |
| R-6 | `restricted/docs/hosting.md` (the `publish-runtime` lines and the paragraph on the second DLL), `hosting-linux.md`, `players.md` | one runtime, `rustic-isles-runtime.dll`; delete the matched-pair paragraph; Linux hosts take the DLL from the release (call 6) |
| R-7 | `restricted/docs/security/runtime-download.md`, `host-page.md`, `launcher-net.md`, `host-gamedata.md` | the names follow P-1, L-1, L-3, L-4 |
| R-8 | `restricted/modules/rustic-isles/docs/GENERATING_SDK.md`, `SCOPE_AND_LEGAL.md`, `README.md`, `.gitignore` | the SDK is needed for the C++ CatalogExporter only; the runtime build steps go |
| R-9 | `restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/security/runtimeAuthorization.ts` (and tests), `legacyNames.ts`; `restricted/modules/rustic-isles/crates/rustic-isles-mit-parity/tests/golden/auth/*` | the Node reference takes P-4's change (RD2: the reference takes any change, kept small), the inventory under `restricted/modules/rustic-isles/docs/inventory/` notes it, and the goldens are re-recorded, the commit saying why (call 1) |
| R-10 | `restricted/modules/rustic-isles/reference/1.14.7/ParadoxDirector/src/runtimeUpdater.ts`, `.env.example` (the `DAINTY_ROLLBACK=1` Node Director) | it copies `winmm.dll` and `Rustic-Isles.dll` next to the game and relies on the loader. After R21: copy `rustic-isles-runtime.dll` instead (the loader's fixed-name rule loads it), or document that the rollback Director needs the last pre-R21 set **(call 7)** |
| R-11 | `restricted/launcher/src-tauri/tests/controller.rs`, `tests/hostapi.rs`; `restricted/crates/dainty-host-parity/tests/{gamedata,setup,update}.rs` and `fixtures/node/gamedata/runtime/published/runtime.json` | expectations follow P-1, H-4, L-1; a parity fixture recorded from the Node host kit is re-recorded only if the Node side changed (R-9), else the Rust side's departure is listed in the test with call 1 |
| R-12 | `restricted/launcher/src-tauri/src/bridge/spec.rs` | only if L-4 renames or reshapes `build_runtime`: regenerate with `RR_UPDATE_GENERATED=1` |
| R-13 | `restricted/README.md`, `restricted/AGENTS.md` | the runtime is MIT and at the root; "a runtime patch is applied" (AGENTS) points to D-5 |

---

## 7. Order and safety

### 7.1 What must land together

- **U-1, one merge:** section 4 (the MIT runtime alone) **with** H-1, H-2, H-4, H-5 (the host kit and the Director
  stop publishing and injecting `Rustic-Isles.dll`). Section 4 without H-4 would let a host publish an R21 MIT DLL
  beside an AGPL DLL with patch 0041, which ends every game at start with `0xE10A` (4.2).
- **U-2, the feed and auth:** P-1 to P-6 with U-1 (one server release): the content server must accept and serve the
  set the host kit now publishes, and the auth check must name the same core file.
- **U-3, the launcher:** L-1 to L-5, released **before** U-1/U-2 reach any server (7.2).
- The restricted list (section 6) follows the server release; R-9's re-record is part of U-2's merge if the parity
  suites would otherwise fail.

### 7.2 Release phases

1. **Phase A, launcher.** A launcher release that accepts both set shapes (L-1, L-2) and removes retired files only
   when it installs an R21 set (L-1, L-3). It plays every current server exactly as before.
2. **The hands-on pass** (sections 8 and 9) on the R21 build: a local server tree from `plan` with U-1 and U-2, the
   Phase A launcher, the MIT DLL published alone.
3. **Phase B, server.** The server release with U-1 and U-2. A host's `update` publishes the MIT-only set (H-11);
   each launcher, already Phase A, injects it alone.
4. **Phase C, later.** One launcher release after phase B has been out for a while, the launcher may drop the
   pre-R21 shape. Not part of R21.

A launcher older than phase A, meeting a phase B server, refuses PLAY because the set lacks `Rustic-Isles.dll`
("runtime missing"), and updates itself as for any launcher release. Nothing is installed in that case.

### 7.3 Rollback

- **Before phase B is out:** nothing to roll back; the pre-R21 pair keeps running everywhere.
- **After phase B:** keep the last pre-R21 runtime set (both DLLs and its `runtime.json`) from the release before it.
  A host rolls back by installing the previous server release (its content, auth and Director accept the pair) and
  `publish-runtime --from <the kept folder>`. Phase A launchers accept the pair (L-1). The game then runs exactly as it
  did before R21.
- **What cannot roll back by republishing alone:** a phase B server (U-1, U-2) refuses a pair set by design (P-3,
  H-1). The rollback is the whole server release plus the kept set, never the set alone.
- **Call 2's tag** keeps the AGPL source of that kept DLL reachable, which the AGPL requires of anyone who ships it
  again.

### 7.4 What CI cannot check

The game with the MIT DLL alone, every flow, the Director's Training Grounds stop, Proton. Those are the hands-on
pass. CI checks section 11.

---

## 8. R21's play-test (surface section 8, R21 row)

"The full checklist on the MIT runtime alone: login, Ramsgate, a hunt, the store, realms." It is the frame of the one
hands-on pass: run it, and section 9's per-slice steps inside it.

**Set-up.** A server tree from `plan` with R21 (U-1, U-2), the Phase A launcher, the game at 1.14.7. Build the MIT DLL
(`cargo build --release --locked --target x86_64-pc-windows-msvc -p rustic-isles-runtime`), run `dainty-host
publish-runtime`, and keep a copy of the previous release's runtime set for step 10.

1. **The set.** `.workspace/runtime/runtime.json` lists `rustic-isles-runtime.dll` (and `winmm.dll` if built) and no
   `Rustic-Isles.dll`; the folder holds no `Rustic-Isles.dll`. `dainty-host doctor` shows no retired-name warning.
2. **Client start.** PLAY. The launcher's log names one DLL injected, `rustic-isles-runtime.dll`, with its SHA-256.
   Process Explorer on the game lists `rustic-isles-runtime.dll` and no `Rustic-Isles.dll` or `winmm.dll`. In
   `rustic_isles_dll_port0.log`: `[MitRuntime] rustic-isles-runtime <v> role=client port=0 owns=<n> ids` with no
   `seam=`, and **no** `[MitSeam]` line or any line without an MIT tag that the AGPL used to write (`[UrlRedirect]`,
   `[ExitTrace]`, `[HUDTrace]`, `[ClientInventoryTrace`).
3. **Login and Ramsgate.** No EAC dialog, a region is found, the login screen's branding, Ramsgate loads and stays past
   120 s; F2 opens the console. Ramsgate's server log (Director-started, MIT alone): `role=server`, the install lines,
   `RUSTIC_ISLES_GAMESERVER_READY` seen by the Director, `[NetConnEdge] tick=... connCount -2 -> 0 ...` once at listen
   and `0 -> 1` when you arrive. No window, no CEF process.
4. **The store.** Open Lady Luck's Store and the platinum store in Ramsgate; buy one item with a currency you have;
   it arrives in the inventory. Talk to a vendor (the camera fix).
5. **A hunt.** Start one from Ramsgate with two accounts. Both arrive, see each other and each other's weapons; the
   omnicell abilities charge and fire; kill the Behemoth; the kill's XP is saved (check after returning). Return to
   Ramsgate; the empty hunt ends itself about 60 s later and the Director records the exit.
6. **Trials and escalation.** The Trials are current and launchable; one escalation round offers relic choices.
7. **The Training Grounds (G-1).** With `training_on_demand` on and `training_idle_minutes = 1` (rerun `setup`): enter
   the Training Grounds, leave. The server log shows `connCount 0 -> 1` and `1 -> 0`; about 1 minute later the
   Director logs `[TrainingOnDemand] no player in the Training Grounds for 1 min - stopping them` and the server
   exits.
8. **Realms.** Host a realm hub from the launcher and join it from the second account; host a hunt in it; fence it
   from the launcher. The hosted server's log is MIT-only, as in step 3.
9. **Leftovers.** Copy an old `Rustic-Isles.dll` into the game's `Win64` and into `.workspace/runtime`. PLAY: the
   launcher removes the first before the game starts (or refuses PLAY naming it). `dainty-host doctor` warns about the
   second; `publish-runtime` removes it. Proton players who keep the loader: the same with the loader in place; the
   game loads only the MIT DLL.
10. **Rollback rehearsal.** On a copy of the server tree: install the previous release, `publish-runtime --from` the
    kept pair, PLAY with the same Phase A launcher: the game behaves as before R21 (the AGPL's `[MitSeam] ...
    present` line is back).
11. **Exit.** Quit the client from the menu: exit code 0, no crash dialog. Stop a server from the Director: clean exit.

Only CI passed until these steps are done. Note anything that differs from the same steps on the last pre-R21
release.

---

## 9. The one hands-on pass: every slice's play-test

The rulings ("Order, amended") collect every slice's play-test for one pass at the end. With R21 that pass runs on the
MIT DLL alone **(call 3)**. Each slice's steps apply as written with these adaptations, which hold for every row:

- **Set-up:** ignore "build the AGPL runtime with patch ..." and "inject both"; use section 8's set-up.
- **Skip** every check of an AGPL line: the `[MitSeam] ... present` counts and "left to it" lines, "no AGPL `[x]`
  line" (it holds trivially), and every **regression** step ("without the MIT DLL", "an MIT DLL without Rn").
- **Keep** every MIT log line, every game behaviour and every Director or launcher observation.
- If a step fails on the MIT DLL alone, rerun that step on the kept pre-R21 pair (8, step 10's set-up) to tell an R21
  regression from a slice bug, and report which.

| Slice | Play-test | Notes for the MIT-only pass |
| --- | --- | --- |
| R0 | `spec-runtime-r0.md` 11.1 | steps 2–4 (status line without `seam=`), skip 5; 11.2 (the seam patch build) is not run |
| R1 | none (CI only) | — |
| R2 | `spec-runtime-r2.md` 10.3 | skip 5; the capability header is now R10's |
| R3 | `spec-runtime-r3.md` 11.1 | skip the AGPL seam count |
| R4 | `spec-runtime-r4.md` 11.1 | skip the `[MitSeam]` checks and 7 |
| R5 | `spec-runtime-r5.md` 14.2 | skip 7 |
| R6 | `spec-runtime-r6.md` 11.2 | skip 8; the ruling's "AGPL's log lines about guards" no longer appear |
| R7 | `spec-runtime-r7.md` 12.3 | skip 8 |
| R8 | `spec-runtime-r8.md` 14.2 | step 4's stamina is R20's now; skip 9 |
| R9 | `spec-runtime-r9.md` 13.2 | the player host case now listens in the MIT DLL (4.5); skip 6 |
| R10 | `spec-runtime-r10.md` 16.2 | step 3's hub exemption is R8's; skip 8 |
| R11 | `spec-runtime-r11.md` 19.1 | skip the seam count |
| R12 | `spec-runtime-r12.md` 18.1 | step 7 compares against the last pre-R21 release's frame time |
| R13 | `spec-runtime-r13.md` 17.1 | skip the seam count |
| R14 | `spec-runtime-r14.md` 16.3 | skip the seam count |
| R15 | `spec-runtime-r15.md` 10.2 | exactly one `[SupplyCap] removed` line per catalog; skip 5 |
| R16 | `spec-runtime-r16.md` 9.3 | skip 7 |
| R17 | `spec-runtime-r17.md` 11.1 | skip the `[MitSeam] CON-1` line and 9 |
| R18 | `spec-runtime-r18.md` 9.2 | the networking fixes it notes are R12's (MIT); skip 8 |
| R20 | `spec-runtime-r20.md` 15.3 | step 3's AFK write is R8's; skip 9 |
| R21 | section 8 | — |
| Loader (Proton) | `spec-runtime-loader.md`, and `docs/launcher-injection.md` "Proton with the helper" | with the MIT DLL alone (L-6) |
| Injection | `docs/launcher-injection.md` "Hands-on checks", `docs/director-injection.md` "Hands-on checks" | after D-1 their file names are the MIT DLL's |

Order for one session at the PC: R0, R4, R17, R2, R15 (client and login); R5, R9, R8, R6, R3 (servers); R7, R16, R14,
R20 (loading, travel, roles); R11, R12, R13 (two players); R18; R10 and realms; section 8's 7, 9, 10 last.

---

## 10. Calls, departures, ambiguities

Calls:

1. **Decision row for the core DLL's name.** P-3 and P-4 change answers the Node reference and its recorded replies
   cover (content's `runtime.json` refusals, the auth core name). Section 6 of the plan ("each game version's routes and
   JSON, matched as its Node service answers them") makes that a departure unless a decision row says otherwise. R21
   asks the integration pass for the row (an RD); R-9 makes the backend's Node reference agree so its goldens stay a
   contract, and content's hand-edited cases cite the row (P-3).
2. **(for the maintainer) The AGPL source: oracle or deleted.** Recommended: **delete** `native/runtime/` from
   `restricted/` once phase B has been out one release, after tagging the last commit that has it
   (`agpl-runtime-final`). Reasons: no parity test builds or runs it (no CI can build it without a host's SDK); every
   behaviour is specified in the slice specs; an upstream runtime change is read from upstream's own tree by a spec
   writer (D-5); the tag keeps the source of the rollback DLL reachable (7.3). Keeping it unbuilt in the tree costs
   reviews and invites a stray patch. Until then it stays, unbuilt (R-1).
3. **The one pass runs on the MIT DLL alone.** Running each slice's play-test on the two-DLL builds first would test a
   configuration nobody will ship. The kept pair is the bisecting tool (9).
4. **The seam exports are removed, not kept as no-ops**, and no marker export replaces them (4.2).
5. **The broker keeps its MIT chain and loses the AGPL band** (4.3); `chained_by_agpl` and R20's skip go, and R20
   stops reading `SERVER_TICK_FILTER.flag` (4.4).
6. **(for the maintainer) Where hosts get the DLL.** Recommended: the server release ships the prebuilt
   `rustic-isles-runtime.dll` (B-2), and `publish-runtime` and setup use it when the host built none; the Host page's
   build action builds with cargo only in a source checkout (L-4). The MIT DLL needs no SDK (RT-2), so a Linux or
   Docker host, which cannot build an MSVC DLL, no longer needs a Windows PC to make one.
7. **(for the maintainer) The Node rollback Director.** Recommended: R-10 copies `rustic-isles-runtime.dll` (the
   loader loads it by its fixed name); the alternative is to document that `DAINTY_ROLLBACK=1` needs the last pre-R21
   set.
8. `[NetConnEdge]` is written with no `[MitRuntime]` tag and the AGPL's exact text (3.2), at `order = 1900`, under the
   new feature id `SP-3@R21`.
9. The status line keeps `owns=` and drops `seam=`; the MIT tags stay (4.6).
10. The R19 registry lines stay, never owned (4.1).
11. `mit_runtime` and Guard's `TRANSPORT_RUNTIME_MIT` stay accepted, redundant with the runtime manifest's hash (H-8,
    H-9).
12. The doctor's patch 0019/0020 scan goes (H-6).
13. The Wine wrapper's `_MIT_` variables are no longer set (H-2).

**Q-1 (for the maintainer): the MIT DLL's file name after R21.** The page follows RT-5 and the brief: the runtime is
`rustic-isles-runtime.dll`, and sections 5 to 7 move every consumer to it. The alternative is to publish the MIT build
as `Rustic-Isles.dll` from R21 on:

- *For the alternative:* no consumer changes (protocol, content, auth, Director, launcher, loader, realms, Guard), no
  route answers differently, so no decision row and no re-recorded goldens; launchers older than phase A keep working;
  the rollback is republishing a set. Sections 5.1–5.3 shrink to H-4 (publish the build under the old name, stop
  publishing the MIT name) and L-1's removal of a stale `rustic-isles-runtime.dll`.
- *Against it:* the name that meant "the AGPL runtime" for the whole transition would mean MIT code from one release
  to the next, in hash pins, docs, the security pages and a host's own folder; a stale transition-era
  `rustic-isles-runtime.dll` beside the new `Rustic-Isles.dll` loads the MIT runtime twice (two hook sets), which the
  launcher and the host kit must then guard against; and RT-5's name, the crate's name and the naming scheme
  (`rustic-` prefix) part ways.

Recommended: **keep `rustic-isles-runtime.dll`** (this page as written). Choose the alternative if launcher version
skew in players' hands is a bigger worry than one launcher-first release.

Departures:

1. The lifecycle snapshot the AGPL logged at each connection edge is not written (3.2; RT-3).
2. A stale `Rustic-Isles.dll` with patch 0041 loaded beside the R21 DLL ends the game with `0xE10A` instead of
   running (4.2): intended.

Ambiguities:

- The AGPL's frame counter for `tick=` counts every engine tick from the process's first; the MIT one counts SP-3
  calls from the first, which is the same count unless the MIT chain is installed later than the AGPL's was (it is
  installed in `DllMain`, earlier). The Director ignores the value.
- The auth manifest's `winmm.dll` requirement (`PROXY_DLL`) and content's "the loader is optional" disagree today; R21
  does not change either (P-4).

Settled here: none of RT-12 to RT-18 is R21's.

No open question beyond Q-1 and calls 2, 6 and 7.

---

## 11. Tests

### 11.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`, every platform):

- The R21 gate (4.1): every slice in `ENTRIES` but `R19` is in `LANDED`, `R21` too; every non-`R19` id is owned.
- `occupancy`: from `last = -2`, counts `0` (line), `0` (none), `1`, `1`, `0`, `-1` (each change a line); the line
  text exactly, with a null driver (`NetDriver=null`), negative values, and large decimal addresses.
- The chain model without the AGPL band: order, absorb, `original`.
- `listen_decision`: DeployServer and player host `Mit`, invalid `None`.
- The status line without `seam=`.

DLL (`cargo test -p rustic-isles-runtime`, Windows):

- The built DLL's export table contains none of `rir_seam_version`, `rir_owns`, `rir_chain_add_inner`,
  `rir_chain_remove_inner`, `rir_original`, `rir_listen_driver`; the import check of R0 unchanged.
- The broker on real code with MIT wrappers only.

Outside the runtime: each row's crate tests follow its change; the parity suites pass with the re-recorded replies
(call 1); `cargo test -p dainty-launcher-kit` covers both set shapes (L-1) and the leftover removal (L-3); the
Director refuses a set with `Rustic-Isles.dll` (H-1); `publish-runtime` removes retired names (H-4).

### 11.2 Never in CI

Everything in section 8 and section 9.

---

## 12. Size

MIT runtime: about 60 lines added (`occupancy`, core 30 and DLL 30) and about 400 removed (exports, AGPL band,
`chained_by_agpl`, `AgplListens`, tests). Outside it: about 600 lines changed across sections 5.1–5.3, most of them
tests and fixtures. Docs: sections 5.5 and 6. Restricted: section 6, plus the deletion of call 2 later.

---

## 13. Clean-room check

The page was searched for identifiers that only the AGPL source has (function, variable, macro and type names of the
C++ tree). What remains is facts: RVAs and offsets already in the surface page, engine member names
(`ClientConnections`, `Num`, `Max`), flag and variable names, log texts another program or the operator reads, file and folder names for the
restricted list, and patch numbers. Section 6 names AGPL files by path only, for the restricted session.
