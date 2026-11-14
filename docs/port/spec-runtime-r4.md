<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R4, client login, EAC and the command line (RD27, plan item 11)

> Since R21 (`spec-runtime-r21.md`) the AGPL runtime is gone; `rustic-isles-runtime.dll` is the only runtime. This
> slice's play-test runs on the MIT DLL alone, as that spec's section 9 adapts it: skip every step that builds or
> patches the AGPL runtime, injects both DLLs, counts `[MitSeam]` lines or runs without the MIT DLL.

R4 makes a private 1.14.7 client log in through the Dainty Lass backend instead of Epic's, without Easy Anti-Cheat,
and lets it find a matchmaking region; it also carries the switches a game server needs into the engine's own command
line (`-nullrhi`, the player host's `MULTIHOME`, the en-US culture of patch 0031). Surface ids CL-1 to CL-13
(`spec-runtime-surface.md` section 3.3). Six of them are ported; seven are logging-only probes that RT-3 leaves
unported (section 2).

An implementer builds from this page, `spec-runtime-r0.md`, `spec-runtime-r1.md` and the surface page alone, and never
reads `restricted/`. What crossed is facts: RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7, CL 647472), engine and
game names, switch and host names, flag names, and behaviour. The AGPL side's change for this slice (runtime patch
0047) was written by the spec writer in `restricted/` and is described here only by the contract it keeps (section 8).

Conventions are the surface page's and R0's: an RVA is `+0x0XXXXXXX` from the executable's image base; **C** client,
**S** server, **B** both. **(call)** marks a decision this page makes, **(departure)** a deliberate difference from the
AGPL runtime, **(ambiguity)** something the source did not settle; all are collected in section 12.

---

## 1. Scope

| R4 owns | Section |
| --- | --- |
| CL-1: the engine command line the game reads (`-NoEAC`, `-DisableEOS`, `-nullrhi`, `MULTIHOME`, `-CULTURE=en-US`) | 3 |
| CL-2: the MCP login path (EOS off) | 4 |
| CL-5, CL-7: Easy Anti-Cheat stays unloaded, and its expected start-up error stays silent | 5 |
| CL-8, CL-9: the retired QoS latency-test hosts resolve to loopback | 6 |
| The engine `MULTIHOME` value other slices read (R8, R9, R10) | 3.5, 9 |
| Runtime patch 0047 (the AGPL side keeps reading that value while it still owns R8 and R9) | 8 |
| RT-16 and RT-3 for CL-3, CL-4, CL-6, CL-10 to CL-13 | 2 |

Not R4: the backend URL rewrite (R2, HTTP-1), the server's CRT locale (R5, HL-12, the other half of patch 0031), the
login screen's branding and automation (R17), listening and the pinned-address check (R9, R10), the `[Perf]` lines
(R8).

**Where the code goes (R0 2).**

| Part | Crate, module | Pure? |
| --- | --- | --- |
| The served-line builder, the `MULTIHOME` scanners, the switch tests, the EAC message test, the QoS host test | `rustic-isles-runtime-core/src/login/` (`cmdline.rs`, `eac.rs`, `qos.rs`) | yes: CI on every platform |
| The detours, the CL-7 write, installs, the published `MULTIHOME` | `rustic-isles-runtime/src/login/` (safe code) and `src/ffi/login.rs` (the detour bodies and raw reads, under R0's one `unsafe` module) | Windows only |

**Shared lines R4 adds** (the only edits outside its own folders):

- `registry.rs`: `"R4"` in `LANDED`; CL-3, CL-4, CL-6, CL-10, CL-11, CL-12, CL-13 move from slice `R4` to `R19`
  (section 2) and the registry tests follow (section 10).
- `offsets.rs`: five `TARGETS` lines and one `data` constant (section 7).
- `lifecycle.rs`: Init calls R4's installer for the role and appends its results to the status (section 7.3).
- `lib.rs` and `ffi/mod.rs`: the `login` module lines.
- `Cargo.toml` of the DLL crate: whatever makes it import `ws2_32.dll` statically (section 6.1; the integration pass
  adds a `windows-sys` feature if one is used).

---

## 2. What is ported (RT-3) and RT-16

| Id | Role | What it does today | Ported? | Why |
| --- | --- | --- | --- | --- |
| CL-1 | B | serves the engine a command line with the switches it needs | **yes** | the root of the EAC skip, the MCP login, a GPU-less hub and the player host's loopback bind |
| CL-2 | C | `IsEOSEnabled` answers 0 | **yes** | without it login goes to Epic's EOS SDK and fails |
| CL-3 | C | logs `LoginOperationFailed`, then the original | no | logging only |
| CL-4 | C | logs `LoginOperationSuccess`, then the original | no | logging only |
| CL-5 | C | absorbs EAC's expected client-init error | **yes** | a modal dialog otherwise, if EAC ever starts |
| CL-6 | C | logs a redacted disconnect reason and the caller, then the original | no | logging only |
| CL-7 | C | writes 0 to `EnableEasyAntiCheatFeatures` | **yes** | the game's own "EAC will not load" path |
| CL-8, CL-9 | C | latency-test hosts resolve to `127.0.0.1` | **yes** | without it the client travels with no region |
| CL-10 | B | client: per-packet log with `SOCKET_DIAG.flag`; server: counts bytes per peer for `[PerfNet]` | no | see below |
| CL-11 to CL-13 | B | per-packet receive logs | no | logging only |

**CL-10's byte count is a probe, not a server feature (settled).** The counter feeds one thing: the `[PerfNet]` line
the server's watchdog thread writes once a minute beside `[Perf]`, `[Memory]`, `[PerfDetail]` and `[PerfThreads]`
(capacity measurements for a busy Ramsgate). Nothing in the game, the server or the Director reads it, and no
behaviour depends on it. RT-3 names "perf and memory" and "socket" among the probes it leaves unported. If R8's spec
decides to port the `[Perf]` family, CL-10 moves to R8 with it; R4 does not hook `sendto`.

**Not ported means "stays AGPL until R21", not "removed now" (call).** These seven ids follow R0 7.5's rule for DX-1..9:
they move to slice `R19` in the registry, so they are never owned and the AGPL runtime keeps installing them exactly as
today. Two reasons. Owning them would delete working diagnostics before the AGPL DLL retires, for no gain. And owning
CL-10 would leave the AGPL's `[PerfNet]` reader believing its counter is on (the seam answers its hook creation with
success) while nothing counts, so the line would read zero traffic: a misleading line, not a missing one.

**RT-16, settled: no.** The socket hooks are installed by one routine, at client init and late in server init:

| Hook | Client | Server |
| --- | --- | --- |
| CL-10 `sendto` | only with `SOCKET_DIAG.flag` | unless `NO_PERF_DETAIL.flag` (counting, no logging); with `SOCKET_DIAG.flag` it also logs |
| CL-11 `recvfrom`, CL-12 `WSARecvFrom`, CL-13 `WSARecv` | only with `SOCKET_DIAG.flag` | only with `SOCKET_DIAG.flag` |

With neither condition the routine installs nothing. The surface page's reading was right; this table replaces its
"(?)".

**R4's flags (RT-14 for R4).** Appendix A of the surface gave R4 three:

| Flag | In the MIT runtime |
| --- | --- |
| `DISABLE_EAC_LOAD` | **dropped (call).** The AGPL's gate on CL-7 is "this flag, or always", so it never switched anything |
| `SOCKET_DIAG` | not R4's: it gates CL-10..13, which stay AGPL |
| `NO_PERF_DETAIL` | not R4's: same |

R4 reads one flag it does not own: `DISABLE_SERVER_LOCALE_FIX` (R5's), read once, for the culture switch (3.3).

---

## 3. CL-1: the engine command line (B)

### 3.1 The target

`FCommandLine::Get` at `+0x025E8F70`, found by RVA. Signature: no arguments, returns a pointer to the engine's
NUL-terminated UTF-16 command line (the engine's own copy, which it builds independently of the OS's
`GetCommandLineW`). Every engine consumer reads its switches through it: EAC's `StartupModule` asks it for `NoEAC`,
`IsEOSEnabled` for `DisableEOS`, the RHI selector for `nullrhi`, the socket subsystem's multihome lookup for
`MULTIHOME=` (on every bind), the localisation start-up for `CULTURE=`.

**Why it is needed.** The launcher and the Director put `-NoEAC`, `-DisableEOS`, `-nullrhi` and `-MULTIHOME=...` on
the process command line, but the engine's own line was measured not to carry them: EAC then initialised and failed,
EOS login ran, and a `-nullrhi` hub still brought up a hardware D3D11 device. So CL-1 serves the engine a line with the
switches added.

**(call) Found by RVA only, as the AGPL.** No `.pdata` entry check: a one-instruction getter is a leaf function and
may have no unwind entry, and `verify_entry` would then refuse a correct target. No prologue bytes are known.

### 3.2 The detour (band `Mit`, `order = 0`)

On every call:

1. Call `next` and keep its result `orig`. If `orig` is null, return null.
2. If the served line is built, return it (step 4). This holds after detach too: the line is a static buffer, so
   returning it neither allocates nor logs.
3. If detaching, return `orig`. If `orig` points at an empty line (its first unit is NUL), return `orig` and do not
   build. **(departure)** The AGPL builds on the first non-null answer even when it is empty, which would freeze a line
   holding only the added switches; it has not happened in practice (login would fail), so the MIT waits for the
   engine's real line.
4. Otherwise build the served line once (3.3) and return it. Concurrent first callers wait on one `OnceLock`; the
   loser returns the winner's line.

The served line is a `Vec<u16>` with a trailing NUL, leaked into a `&'static [u16]` (never freed: the engine keeps the
pointer). The engine's later `FCommandLine::Set` or `Append` calls are not reflected **(ambiguity: the AGPL freezes the
line the same way; no consumer is known to append after the first `Get`).**

### 3.3 Building the served line

Pure: `build_served_line(engine: &[u16], os_argv: &[Vec<u16>], os_line: &[u16], role: Role, culture_fix: bool) ->
Served`. Start with a copy of the engine's line, then append, each with one leading space and in this order:

| # | Appended | When | "Has it" test on the line built so far |
| --- | --- | --- | --- |
| 1 | `-NoEAC` | both roles, when missing | substring `-NoEAC`, case-sensitive |
| 2 | `-DisableEOS` | both roles, when missing | substring `-DisableEOS`, case-sensitive |
| 3 | `-nullrhi` | server, when missing **and** the OS line (`GetCommandLineW`) contains `-nullrhi` (case-sensitive substring) | substring `-nullrhi`, case-sensitive |
| 4 | `-MULTIHOME=<value>` | server, per 3.4 | 3.4 |
| 5 | `-CULTURE=en-US` | server, when `culture_fix` and missing | substring `-culture=` compared case-insensitively (ASCII) |

`culture_fix` = `DISABLE_SERVER_LOCALE_FIX.flag` is absent, read once (patch 0031: a server whose Windows culture
writes decimals with a comma misread values such as `105.6`, and broken Behemoth parts stayed on the players' screens).
The same flag turns off R5's CRT locale (HL-12); the two halves share it but no state.

The substring tests are deliberately loose (as the AGPL): `-NoEACx` counts as present. The engine's own matcher is
`FParse::Param`, which is stricter, so a line that merely contains the text would have its switch ignored; no launch
the launcher or the Director makes contains such text.

`Served` also records, for the log line (3.6): whether the engine's original line had `-NoEAC`, `-DisableEOS` and
`-nullrhi`; whether the OS line has `-nullrhi`; and the effective `MULTIHOME`.

### 3.4 `MULTIHOME`

The engine binds its net driver to the `MULTIHOME=` value of its own command line, else to every interface. The
player host (R10) is launched with `-MULTIHOME=127.0.0.1` so it is reachable through the realm relay only; without CL-1
carrying it, the host bound every interface and bypassed the relay's admission. DeployServer hubs and hunts are given
no `MULTIHOME`, and nothing changes for them.

**Engine-line scan** `engine_multihome_in(line)`, as the engine's `FParse::Value` finds it: walk the line keeping an
"inside double quotes" state (each `"` toggles it) and the previous unit; the first position outside quotes, whose
previous unit is not an ASCII letter or digit, where `MULTIHOME=` matches case-insensitively (ASCII) starts the value;
the value runs to the first space, tab, `,`, `"` or `)` or the end. Not found: empty.

**OS-line scan** `os_multihome_in(argv)`: the first argument after argv 0 (`CommandLineToArgvW`, R1's argv) that starts
case-insensitively (ASCII) with `-MULTIHOME=`; the value is the rest of that argument, verbatim. Found or not found
(found with an empty value is "found").

**IP literal (departure).** A dotted quad by R1's `is_dotted_quad`, or an IPv6 address by R1's `is_ipv6` (no brackets,
no zone), at most 45 units, all ASCII. The AGPL's IPv6 test is looser (hex digits, `:` and `.` with at least two
colons); the MIT one is R1's strict grammar. The launcher always passes `127.0.0.1`.

**Server only, after step 3 of 3.3:**

1. `effective` = the engine-line scan of the line built so far.
2. If the OS line has one and `effective` is empty: if the OS value is an IP literal, append ` -MULTIHOME=<OS value>`
   (verbatim) and set `effective` to it; log `carried`. Otherwise do not append; log `not carried`.
3. Else if the OS line has one and it differs from `effective` (ASCII case-insensitive): keep the engine's; log
   `kept`.
4. Last (both roles): if `effective` is not empty and is not an IP literal, or is 64 units or longer, log `invalid`
   and clear it.

A client computes step 4 on its engine line only; nothing reads it there.

### 3.5 The published `MULTIHOME` (for R8, R9, R10)

When the served line is built, `effective` is published once: `cmdline::engine_multihome() -> Option<&'static str>`,
`None` before the line is built and when `effective` is empty. It is the address the engine will bind:

- R9/R10 (the player host's listen): a pinned listen address is honoured only when it equals this value
  (ASCII case-insensitive); otherwise the listen is refused and the host fenced. This fails closed when CL-1 did not
  install.
- R8 (the watchdog's "port is free" probe): bind the probe socket to this address when it is a dotted quad, to every
  interface when `None`, and never act when it is IPv6.

While the AGPL runtime still owns R8, R9 and R10, it reads the same value through patch 0047 (section 8).

### 3.6 Log lines

All after the served line is built, at most once each, on the thread that built it (the first engine call, never under
the loader lock):

- `[MitRuntime] [CommandLine] engine line served: NoEAC=<0|1> DisableEOS=<0|1> NullRHI=<0|1> engineNoEAC=<0|1>
  engineDisableEOS=<0|1> engineNullRHI=<0|1> osNullRHI=<0|1> Culture=<en-US|engine|off> Multihome=<value|none>`
  (one line; the first three are what the served line has, the `engine*` ones what the engine's own line had;
  `Culture` is `en-US` when appended, `engine` when the line already had one, `off` on a client or with the flag).
- Server, step 2 carried: `[MitRuntime] [CommandLine] MULTIHOME=<value> carried from the OS command line`.
- Step 2 not carried: `[MitRuntime] [CommandLine] MULTIHOME on the OS command line is not an IP literal (<n> chars);
  not carried, the engine binds every interface`.
- Step 3: `[MitRuntime] [CommandLine] the engine line already has MULTIHOME=<value>, which differs from the OS
  command line's; the engine's is kept`.
- Step 4: `[MitRuntime] [CommandLine] the engine line's MULTIHOME is not an IP literal (<n> chars); ignored`.

Never logged: the command line itself (a client's carries the one-time exchange code and account identifiers, a
server's the gameserver key and hunt routing), and no `MULTIHOME` value that failed the IP-literal test (only its
length). A value that passed is an IP literal and may be logged.

---

## 4. CL-2: the MCP login (C)

`IsEOSEnabled` at `+0x01C30030`, found by RVA. No arguments, returns a `u8` (the caller reads the low byte). The game
answers "the command line lacks `-DisableEOS`" and caches the answer the first time it is asked, which was measured to
be before the engine's line carries the switch. With 1 the client authenticates through the EOS SDK, whose own HTTP
stack bypasses the engine's (so HTTP-1 never rewrites it) and fails with `EOS_Auth_ExchangeCodeNotFound`; with 0 it
takes the MCP login, issued through the engine's HTTP stack, which R2 (or the AGPL's HTTP-1) redirects to the backend.

Detour (band `Mit`, `order = 0`): if detaching, call `next` and return its answer. Otherwise **absorb**: return 0
without calling `next`. The first call logs once `[MitRuntime] [Login] IsEOSEnabled -> 0 (MCP login)`.

---

## 5. Easy Anti-Cheat (C)

A private client must never start EAC: it cannot create its game client against a private backend, and EAC then
reports the failure in a modal dialog and the client sits at the login screen ("Online Play may not be available").
EAC's `StartupModule` (`+0x01474ED0`, not hooked) skips the load when either:

- the `EnableEasyAntiCheatFeatures` setting is false (the game's log then says `EnableEasyAntiCheatFeatures is false,
  EAC client will not load`), or
- the engine's command line has `NoEAC` (CL-1 supplies it).

### 5.1 CL-7: the setting

`EnableEasyAntiCheatFeatures`, one byte at data `+0x06CFBD44`. In Init, client only: if the byte is in a committed
region whose protection allows writing (`PAGE_READWRITE`, `PAGE_WRITECOPY`, `PAGE_EXECUTE_READWRITE` or
`PAGE_EXECUTE_WRITECOPY`, without `PAGE_GUARD`), write `0` once. **(call)** The AGPL checks only that the byte is
readable; the MIT also checks that it is writable, so a wrong RVA landing in read-only data is a logged failure, not
a fault. Install result for the status: `install CL-7 ok`, `install CL-7 unreadable` or `install CL-7 not writable`.

**(ambiguity)** The engine may set the byte again from its configuration after Init (the AGPL's comments worry about
this but write only once). CL-1's `-NoEAC` is the effective guard either way; the play-test (11) says how to see
which path the game took.

### 5.2 CL-5: EAC's error report

`HandleError` of the EAC client at `+0x01476F20`, found by RVA. Signature `(this, message: *const FString) -> u64`;
the caller reads the low byte, 1 meaning "handled, show nothing". The original dispatches to a game callback when one
is bound, else opens the native `EasyAntiCheat Error: %s` dialog.

`FString` layout: data pointer (`*const u16`) at `+0x00`, `Num` (`i32`) at `+0x08`, `Max` (`i32`) at `+0x0C`; `Num`
counts the terminating NUL.

Detour (band `Mit`, `order = 0`):

1. If detaching, call `next` and return its result.
2. Read the message: `readable(message, 0x10)`; `Num > 0`; take `min(Num, 512)` units, which must be readable from the
   data pointer, and drop the last of them (the AGPL's reading: the NUL, or the 512th unit when the text is longer).
   Any failure is an empty message.
3. **Absorb** when the text contains `IGameClient` or `Failed to create` (case-sensitive): return 1 without calling
   `next`. The first 4 absorbs log `[MitRuntime] [EAC] absorbed the expected client-init error (<n> chars,
   IGameClient=<0|1>)`.
4. Otherwise call `next` with the original arguments and return its result. The first 4 forwards log
   `[MitRuntime] [EAC] forwarded an EAC error to the game (<n> chars)`.

The message text itself is never logged. **(ambiguity)** `Failed to create` is broader than the one failure the hook
is for; it is kept because the measured message is not recorded in the source.

---

## 6. CL-8, CL-9: the QoS region (C)

The client measures each region by pinging the `regionUrl`s the backend's `/candidate/regions` lists over plain HTTP on
port 80. The official list names seven `*.gce.latency-test.steelyard.ca` hosts, which no longer resolve, so every
region was "Invalid" and the client travelled with no region at all. The backend answers those pings on port 80
itself; R4 makes the names resolve to loopback (no hosts-file edit, which needs elevation).

### 6.1 Targets

`GetAddrInfoW` (CL-8) and `getaddrinfo` (CL-9), exports of `ws2_32.dll`, both hooked (the engine's socket subsystem
reaches both). Resolved with `GetModuleHandleW("ws2_32.dll")` and `GetProcAddress`; R0's Init never loads a library.
**(call)** The DLL imports `ws2_32.dll` statically (one referenced function is enough), so the loader maps it before
`DllMain` and the hooks never miss it; the AGPL instead skips both hooks when the module is not yet loaded. If the
lookup still fails the status says `install CL-8 ws2_32 not loaded` (or `... export not found`).

Signatures (system ABI): `GetAddrInfoW(node: *const u16, service: *const u16, hints: *const c_void, result: *mut *mut
c_void) -> i32`; `getaddrinfo` the same with `*const u8` strings. Hints and result are passed through untouched.

### 6.2 The detours (band `Mit`, `order = 0`)

1. If detaching, or `node` is null, call `next` unchanged.
2. If the node name (NUL-terminated, read up to 256 units/bytes; a longer name is passed through) contains
   `gce.latency-test.steelyard.ca` (case-sensitive, as the AGPL), call `next` with `node` replaced by a static
   NUL-terminated `127.0.0.1` in the same width, and the other arguments unchanged.
3. Otherwise call `next` unchanged.

The first 3 rewrites (both detours together) log `[MitRuntime] [QoS] <host> resolves to 127.0.0.1 (the backend answers
the latency test)`, `<host>` the node name when it is ASCII.

---

## 7. Offsets, registry and install

### 7.1 `offsets.rs` lines (R4)

| `TARGETS` id | `rva` | `found_by` | `what` |
| --- | --- | --- | --- |
| CL-1 | `0x025E8F70` | `Rva` | `FCommandLine::Get` |
| CL-2 | `0x01C30030` | `Rva` | `IsEOSEnabled` |
| CL-5 | `0x01476F20` | `Rva` | EAC client `HandleError` |
| CL-8 | 0 | `Export("ws2_32.dll", "GetAddrInfoW")` | DNS, wide |
| CL-9 | 0 | `Export("ws2_32.dll", "getaddrinfo")` | DNS, ANSI |

`data::EAC_ENABLED: u32 = 0x06CF_BD44` (`EnableEasyAntiCheatFeatures`, CL-7, a patch kind: no `TARGETS` line).

Not hooked, named for the reader: EAC `StartupModule` `+0x01474ED0`; the engine's `FParse::Param` `+0x02634DE0` and
`FParse::Value` `+0x02640610`; the socket subsystem's multihome lookup `+0x032AE4B0`.

### 7.2 Registry

`LANDED` gains `"R4"`. Owned then: CL-1, CL-2, CL-5, CL-7, CL-8, CL-9 (six). CL-3, CL-4, CL-6, CL-10, CL-11, CL-12,
CL-13 move to `"R19"` with their kinds unchanged, under the "Diagnostics" comment.

What the AGPL does once R4 lands (patch 0041's contract, R0 7.4): no wrapper on CL-1, CL-2, CL-5, CL-8, CL-9 (their
original pointers become `rir_original`); no CL-7 write (gated by id); CL-3, CL-4, CL-6 and the socket hooks are its
own, chained through the broker as today.

### 7.3 Install (RT-17 for R4)

In Init, after `MH_Initialize` and with the dispatch chains, R4 contributes `login::install(server, base) ->
Vec<(&'static str, Result<(), String>)>`:

- client: after SP-1, in the surface's order: CL-2, CL-1, CL-8, CL-9, CL-7, CL-5;
- server: CL-1, **before** SP-2 and SP-3 (R0 7.6: CL-1 first).

**Settled (RT-17):** nothing in R4 depends on install order. All of them install in `DllMain`, before the engine's
first instruction (R0 3.1, 9), which is the only timing that matters: CL-1 and CL-2 must be in place before the engine
first reads its command line or asks about EOS. The status lines are R0's `install <id> ok|<reason>`; a MinHook
failure gives R0's status name.

R4 registers no dispatch handler and no tick handler.

### 7.4 Cross-slice state (R0 7.5's table, one row added)

| State | Set by | Read by | Rule |
| --- | --- | --- | --- |
| the engine's `MULTIHOME` | CL-1 (R4) | the AGPL player-host listen check (R9/R10) and watchdog port probe (R8) | carried: patch 0047 (section 8); the MIT slices read `engine_multihome()` |
| `DISABLE_SERVER_LOCALE_FIX.flag` | the host | CL-1's culture (R4), HL-12's CRT locale (R5) | no shared state: each reads the flag |

---

## 8. The AGPL side: runtime patch 0047

Without it, a server whose CL-1 is the MIT runtime's would lose the player host: the AGPL publishes the engine's
`MULTIHOME` from inside its own CL-1 detour, which no longer runs, so its listen check would see none and refuse every
pinned listen, and its watchdog would probe every interface and reap a serving host about 45 s in.

The patch (`restricted/modules/rustic-isles/native/runtime/dllmain.cpp`, marked `[DaintyLass 0047]`, a row in
`restricted/docs/paradox-patches.md`) keeps this contract:

- Server role only, and only while `rir_owns("CL-1")` is 1. Otherwise nothing changes.
- The first time the AGPL's listen check or its port probe asks for the engine's `MULTIHOME` and none has been
  published, it calls the engine's `FCommandLine::Get` at `+0x025E8F70` itself. That address is hooked, so the call
  runs the broker's chain and returns the line the MIT runtime serves. It applies its own engine-line scan and
  IP-literal check (the same rules as 3.4's step 4) and publishes the result once (a Windows one-time initialisation,
  so the game thread and the watchdog thread cannot race).
- It logs once `[MitSeam] CL-1 is the MIT runtime's: engine MULTIHOME=<value|none> read from the line it serves`.
- No seam ABI change: seam version stays 1.

The AGPL's `[EACFlags]` line (client init, logging only, not gated) keeps printing its `engine_FCommandLine_*`
booleans, which only its own CL-1 detour sets; with R4 landed they read 0. Its own MIT counterpart is 3.6's line.

---

## 9. Interfaces for later slices

- `rustic_isles_runtime::login::cmdline::engine_multihome() -> Option<&'static str>` (3.5). R9 and R10 use it for the
  pinned-listen check, R8 for the port probe. Before the engine has read its line it is `None`; every consumer runs
  long after.
- Pure, in the core crate: `login::cmdline::{build_served_line, engine_multihome_in, os_multihome_in,
  is_ip_literal}`, so R8, R9 and R10 test against the same rules.

R4 consumes from landed slices only: R0's broker, flags, memory helpers, log and Init; R1's argv and `is_dotted_quad`
and `is_ipv6`. It consumes no slice that has not landed.

---

## 10. Tests

### 10.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`), every platform:

- `build_served_line`:
  - a client line with neither switch gets ` -NoEAC -DisableEOS`, in that order;
  - a line with both is unchanged;
  - `-NoEACx` counts as present;
  - a server with `-nullrhi` only on the OS line gets it, a client never does;
  - a server line containing `-Culture=de-DE` gets no culture switch;
  - with `culture_fix` false none is added;
  - the full server order is NoEAC, DisableEOS, nullrhi, MULTIHOME, CULTURE.
- `engine_multihome_in`:
  - found plain, and `-MULTIHOME=` and `multihome=` both match;
  - not found after a letter or digit (`XMULTIHOME=1.2.3.4`);
  - not found inside quotes;
  - the value stops at space, tab, `,`, `"`, `)`.
- `os_multihome_in`: argv 0 ignored, the first match wins, case-insensitive prefix, an empty value is "found".
- The four `MULTIHOME` outcomes of 3.4 (carried, not carried, kept, invalid) and the 64-unit limit.
- `is_ip_literal`:
  - `127.0.0.1`, `::1` and `fe80::1` accepted;
  - `127.0.0.01`, `[::1]`, `fe80::1%3` and `localhost` refused;
  - a non-ASCII value refused.
- The EAC message test: `IGameClient` and `Failed to create` absorb, `igameclient` does not, an empty message does not;
  the `Num` handling (1, 2, 600).
- The QoS host test on `euw11.gce.latency-test.steelyard.ca` (rewritten), `GCE.LATENCY-TEST.STEELYARD.CA` (not), and
  `steelyard.ca` (not).
- Registry:
  - `owns` for `LANDED = ["R0", "R1", "R4"]` is true for SP-1/2/3 and the six R4 ids only;
  - CL-3, CL-4, CL-6, CL-10..13 are slice `R19`;
  - R0's `every_surface_id_once_with_its_slice` table is updated to match, and `owns_for_r0_and_r1` becomes a check
    computed from `LANDED`, so each slice's landing does not rewrite it.
- The offset-table test of R0 6.1 passes with the five lines.

DLL (`cargo test -p rustic-isles-runtime`, Windows):

- The CL-1 detour body on a fake getter: the first non-empty answer is served with the switches; the same pointer is
  returned on every later call, including after the fake's answer changes; an empty answer is passed through and not
  frozen; null passes through.
- The QoS detour on a fake `GetAddrInfoW`: the rewritten node arrives as `127.0.0.1`, the other arguments untouched.
- The EAC detour on a fake `HandleError`: absorbs return 1 without calling the fake; others reach it.
- The CL-7 write rule on a page the test allocates `PAGE_READWRITE` (written), `PAGE_READONLY` (not writable) and
  `PAGE_NOACCESS` (unreadable).
- The import table lists `ws2_32.dll` (6.1).

**Never in CI:** the real executable, EAC, EOS, the backend login, real DNS, the player host's bind.

### 10.2 Patch 0047

It needs the host's SDK to build, so no CI builds it (R0 10.2). The maintainer's steps are in 11.2.

---

## 11. The maintainer's hands-on checks

Collected for the one hands-on pass at the end (rulings, "Order, amended"); nothing waits on them.

### 11.1 R4's play-test (surface section 8, R4 row)

"Login with no EAC dialog, region found (QoS), Ramsgate; a hub boots `-nullrhi`; a realm host listens on
127.0.0.1."

1. Build the MIT DLL with R4 and the AGPL runtime with patches 0041 and 0047 (R0 11.2). Inject both as in R0 11.1.
2. **Client login.** Start the launcher's PLAY. No EAC dialog appears; login completes; the client reaches Ramsgate.
   In `rustic_isles_dll_port0.log`:
   - `[MitRuntime] ... owns=<n> ids`, `<n>` = 9 with only R0, R1 and R4 landed;
   - `install CL-2 ok`, `install CL-1 ok`, `install CL-8 ok`, `install CL-9 ok`, `install CL-7 ok`,
     `install CL-5 ok`;
   - `[MitRuntime] [CommandLine] engine line served: NoEAC=1 DisableEOS=1 ...`;
   - `[MitRuntime] [Login] IsEOSEnabled -> 0 (MCP login)`;
   - and the AGPL's `[MitSeam] ... <n> hooks left to it` with `<n>` at least 5.

   The backend's log shows the MCP token request (`/account/api/oauth/token` through `/__origin/`). In the game's own
   log (`Saved/Logs`), note whether `EnableEasyAntiCheatFeatures is false, EAC client will not load` appears (CL-7
   took) or EAC is skipped silently (CL-1's `-NoEAC`); either is a pass.
3. **Region.** Open matchmaking (start a hunt search). The MIT log has `[MitRuntime] [QoS] ... resolves to 127.0.0.1`
   (up to 3 lines), and the game's log has a current region (`LogQos` names one, not empty, and not every datacenter
   `Invalid`). Start and finish a hunt; return to Ramsgate.
4. **A hub.** Let the Director start Ramsgate. The server's log has `install CL-1 ok` and the served-line line with
   `NullRHI=1 osNullRHI=1 Culture=en-US Multihome=none`. In Task Manager the hub process uses no GPU 3D engine time
   (a hardware device measured 37–41 % before CL-1 carried `-nullrhi`).
5. **A realm host.** Host a realm hub from the launcher and join it from a second client. The host's log has
   `MULTIHOME=127.0.0.1 carried from the OS command line`, `Multihome=127.0.0.1` on the served-line line, the AGPL's
   `[MitSeam] CL-1 is the MIT runtime's: engine MULTIHOME=127.0.0.1 ...` (patch 0047), and its listen line saying the
   engine `MULTIHOME` matches the pinned address. `netstat -ano -p udp` shows the host's port bound on `127.0.0.1`
   only. Leave the host idle with one player connected for two minutes: it is not reaped. Fence it from the launcher.
6. **The culture.** On a host whose Windows format is pt-BR or de-DE, break a Behemoth part in a hunt: it shows as
   broken. With `DISABLE_SERVER_LOCALE_FIX.flag` the served-line line says `Culture=off`.
7. **Regression without R4.** Run step 2 with an MIT DLL built without R4 (`LANDED = ["R0", "R1"]`): the AGPL's own
   `[CommandLine]` and `[IsEOSEnabled]` lines appear instead, and everything behaves the same.
8. Note anything that differs from the same steps on the previous release. Only CI passed until these steps are done.

### 11.2 Building and checking patch 0047

1. Build the AGPL runtime as R0 11.2 says (`_build.bat` with the host's Dumper-7 SDK).
2. `dumpbin /imports x64\Release\Rustic-Isles.dll`: still no `MSVCP140`, `VCRUNTIME140*` or `api-ms-win-crt-*`.
3. Without R4 in the MIT DLL, step 5 of 11.1 behaves as before (the AGPL's own `MULTIHOME=127.0.0.1 carried` line, no
   0047 line). With R4, step 5 as written.

---

## 12. Calls, departures and ambiguities

Calls:

1. CL-3, CL-4, CL-6, CL-10, CL-11, CL-12 and CL-13 are not ported (RT-3) and move to slice `R19` in the registry, so the
   AGPL keeps them until R21 (section 2).
2. **RT-16, settled:** the server installs CL-11..13 only with `SOCKET_DIAG.flag`; it installs CL-10 (counting) unless
   `NO_PERF_DETAIL.flag`. The client installs any of them only with `SOCKET_DIAG.flag`.
3. CL-10's `[PerfNet]` byte count is a capacity probe read only by a log line, not a server feature; R8 takes it if
   it ports the `[Perf]` family.
4. `DISABLE_EAC_LOAD` is dropped (it never switched anything).
5. CL-1 is found by RVA only, with no `.pdata` check (a leaf getter).
6. CL-7 writes only to a writable page.
7. The DLL imports `ws2_32.dll` statically, so CL-8/9 never miss it.
8. MIT log tags: `[MitRuntime] [CommandLine]`, `[MitRuntime] [Login]`, `[MitRuntime] [EAC]`, `[MitRuntime] [QoS]`.
9. Runtime patch 0047 carries the engine `MULTIHOME` to the AGPL's listen check and port probe (section 8).
10. `engine_multihome()` is R4's interface for R8, R9 and R10.

Departures from the AGPL (none changes a launch the launcher or the Director makes):

1. The served line is built only once the engine's line is non-empty.
2. The IPv6 test for `MULTIHOME` is R1's strict grammar.
3. The QoS log line names the host.

Ambiguities:

- The engine's later `FCommandLine::Set` or `Append` calls are not reflected in the frozen line (as the AGPL).
- Whether the engine rewrites `EnableEasyAntiCheatFeatures` after Init; CL-1 covers it either way.
- CL-5's `Failed to create` match is broader than the one failure it is for.

Open questions for the maintainer: none. RT-16 is settled above; RT-3 is applied, with call 1 open to reversal if the
maintainer would rather drop the seven diagnostics when R4 lands than at R21.

## 13. Size

Rust, without tests: the core crate about 300 lines (served line and scanners 200, EAC and QoS tests 40, IP literal
reuse 20, registry and offsets 40); the DLL crate about 350 (CL-1 detour and publish 100, CL-2 20, CL-5 70, CL-7 40,
CL-8/9 80, install 40). Tests about 450. Patch 0047: 29 lines of C++.
