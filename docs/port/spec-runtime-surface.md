<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room surface specification: the game runtime (RD27, plan item 11 step 1)

This page maps `restricted/modules/rustic-isles/native/runtime` (`Rustic-Isles.dll`, about 28.8k lines of C++, 26.6k
of them in `dllmain.cpp`) so it can be re-created as an MIT runtime **in slices**. It is the surface and the slicing
only: every hook and patch the runtime installs, what each is for, and how the work splits. It is not a per-slice
implementation spec; each slice gets its own `docs/port/spec-runtime-<slice>.md` once the maintainer approves the
slicing in section 8.

An implementer never reads this page's source. What crosses is facts: RVAs into the 1.14.7 executable
(`Dauntless-Win64-Shipping.exe`, CL 647472), UE function and class names, environment variable and file names, wire
formats, and behaviour. No code, comment text or structure of `dllmain.cpp` crosses. MinHook (`native/runtime/MinHook`,
BSD-2-Clause, vendored) is not specified: it moves as it is.

Conventions:

- An RVA is relative to the game executable's image base (`GetModuleHandle(nullptr)`), written `+0x0XXXXXXX`.
- **Role:** **C** client (no `-server` on the command line), **S** server (`-server`), **B** both. A server is further a
  DeployServer gameserver (`GLOBAL_GAMESERVER`) or a player host (`HUNT_CAPABILITY`), section 2.4.
- **State:** *default* (always installed for its role), *off-flag* (installed unless the named file is present),
  *opt-in* (installed only when the named file is present). Every "flag" is a file of that name next to the game
  executable (section 2.6).
- **(?)** marks something I read but could not settle from the source; each is listed in section 10.
- "Patch NNNN" is a row of `restricted/docs/paradox-patches.md`; "infra" is that page's Infrastructure table.

---

## 1. Scope and provenance

| Part today | Lines | Provenance | In the MIT runtime |
| --- | --- | --- | --- |
| `dllmain.cpp` | 26 632 | Undaunted (gwog), Mystic Paradox, Reforged/Dainty Lass patches | Rewritten slice by slice from the per-slice specs |
| `Networking.cpp`, `Networking.h` | 1 168 + 87 | Undaunted, Mystic Paradox | Slice R9 |
| `p2p/PlayerHost.cpp`, `P2PHooks.h`, `P2PHooksStub.cpp`, `p2p/AntiCheatHooks*` | 456 + 71 + 29 + 49 | Mystic Paradox | Slice R10 |
| `BackendUrl.h` | 247 | Mystic Paradox, realms S1 | Slice R1 (pure, unit-tested) |
| `RuntimeShared.h`, `constants.h`, `deployment_config.generated.h.example` | 47 + 13 + 6 | Mystic Paradox | Slice R0 / R1 |
| `Rustic-Isles.vcxproj`, `.sln`, `_build.bat`, `build-loader.bat`, `BUILD.md` | ~300 | Mystic Paradox | Replaced by the MIT build (section 9) |
| `assets/mp_login_background.png`, `Rustic-Isles.rc`, `resource.h` | — | Mystic Paradox | Question RT-9 |
| `tools/compat-inventory.ps1` | 125 | Mystic Paradox | Not ported (a dev tool) |
| `MinHook/` | ~3k | TsudaKageyu, BSD-2 | Moves as it is |
| `..\catalog-exporter\StoreRuntimeExporter.cpp` (compiled into the runtime) | — | Mystic Paradox | Lane E's spec; question RT-10 |

Not here: the runtime loader (the root's MIT `native/runtime-loader`, `spec-runtime-loader.md`) and the CatalogExporter (lane E).
The game SDK (`SDK.hpp`, `SDK/`) is a Dumper-7 dump each host makes and is never committed; the runtime compiles
about 400 references to it (question RT-2).

---

## 2. How the runtime is built, loaded and configured

### 2.1 Build

- One DLL for both roles, `x64\Release\Rustic-Isles.dll`: MSBuild, v145 toolset, C++ latest, Release x64, static CRT
  (`/MT`). The DLL must import no `MSVCP140`, `VCRUNTIME140*` or `api-ms-win-crt-*`: the game folder ships an old
  `msvcp140.dll` that the loader search order prefers.
- It compiles against the host's own Dumper-7 dump (Dumper-7 `3a57413` or older; a later one writes
  `GetStaticName(L"<class>", FnName)` lookups and the server stops when a player joins). So no CI can build it today.
- Optional parts by file presence: `p2p\PlayerHost.cpp` replaces `P2PHooksStub.cpp`; `anticheat\HuntGuard.cpp`
  replaces `p2p\AntiCheatHooksStub.cpp` (absent in this tree, so the stub is built: a player-hosted hunt produces no
  Hunt Guard evidence).
- `RUSTIC_ISLES_INTERNAL_VERSION = "0.0.3"` (`constants.h`) (? RT-13: nothing in the runtime reads it that I found).
- The same DLL is published to the client and the server runtime feeds; their manifests' DLL hashes must match.

### 2.2 Loading (RD9)

`winmm.dll` (the runtime loader, a proxy of the system `winmm.dll`) sits next to the game executable. In its own
`DllMain` it loads the runtime by name, choosing the most recently written of `Rustic-Isles.dll`,
`MysticParadox.dll`, `MystPaxInternalServer.dll` (rename aliases), then each DLL listed one per line in
`mystic_loader.ini` if that file exists. RD9 reserves the ini for host-picked tools (Dumper-7, the CatalogExporter):
the launcher writes it for one session and refuses PLAY while an ini or an unknown DLL is next to the loader.

So the runtime's `DllMain` runs under the loader lock, very early (before engine PreInit and before plugin modules
start). That timing is load-bearing: the FCommandLine hook (CL-1) and the web-browser skip (HL-5) must be in place
before the engine reads its command line or starts plugins.

### 2.3 Start-up order

1. `DllMain(PROCESS_ATTACH)`: disable thread notifications; copy every `MYSTICPARADOX_<X>` environment variable to
   `RUSTIC_ISLES_<X>` unless that is already set (one-release rename alias); run Init.
2. Init (loader lock: no logging except a capped init log, no file I/O beyond flag probes):
   1. Resolve the executable's folder and the flag-file cache mode (section 2.6).
   2. Resolve the backend address once (section 2.5).
   3. Role: server iff the process command line contains `-server`. Image base from `GetModuleHandle(nullptr)`.
   4. Server only: auth mode from `RUSTIC_ISLES_AUTH_MODE` (section 2.4); write `GIsServer` = 1 at `+0x06E1EADA` and
      `GIsClient` = 0 at `+0x06E1EAD9` (patch HL-1).
   5. Server only: parse the launch arguments (section 2.4), then install the server hooks (section 3, in the order
      of section 3.10). Client: install the client hooks (section 3.10).
   6. Start MainThread (stack 0x1000).
3. `DllMain(PROCESS_DETACH)`: set a process-wide "detaching" flag first. From then on nothing logs or calls into the
   C runtime (every hook checks it). Logging after detach crashed every client exit until runtime 0.2.96.

MainThread:

- Writes the session banner (role, port, map, image base; never the command line, which carries the exchange code or
  the deploy key), the flag-cache mode and one `[BackendAddress]` line.
- Server with an invalid auth mode: `TerminateProcess(0xE102)` before listen.
- Player host: `P2PHooks::StartServer()`; false ends the thread (the module has already ended the process).
- Client with `REWARD_CACHE_CAPTURE.flag`: start the store-capture worker (15-minute wait for the `season_store`
  view model; lane E's code).
- Waits for a world: the SDK's `UWorld::GetWorld()` or the `GWorld` pointer at `+0x06FC7F58`. Server: once a second,
  up to 30 s, then sets "listen requested" for the engine tick (section 4.1) and exits. Client: every 1 ms, then
  after 3 s spawns a `UConsole` on the game viewport and binds the console key to **F2**.

### 2.4 Server launch arguments and auth mode

`RUSTIC_ISLES_AUTH_MODE`: absent or `GLOBAL_GAMESERVER` = DeployServer gameserver; a value the player-host module
claims (`HUNT_CAPABILITY`) = player host; anything else = invalid (exit 0xE102 in MainThread).

With more than 8 argv entries (`CommandLineToArgvW`), argv is positional. Every value is copied into storage that
outlives the argv block (a dangling pointer here crashed late in the map-URL hook):

| argv | Meaning |
| --- | --- |
| 1 | the gameserver API key (DeployServer only; the player host requires `-` and never carries a key) |
| 2 | port (decimal) |
| 3 | map path |
| 4 | Behemoth class path, or a value containing `NO_BEHEMOTH` |
| 5 | matchmaker hunt id, or `NO_MM_HUNTID` |
| 6 | expected players, `accountId:..., accountId:...`, or `NO_EXPECTED_PLAYERS` |
| 7 | the server's own `ip:port` |

Port ≥ 8776 or a player host: the empty-server watchdog is on and logging is on. With 8 or fewer entries the server
is in manual test mode: port 8777 unless the command line has `Port=<n>`, map `Ramsgate` unless the first argument
after the program is a map name (up to `?`, space or tab), watchdog off.

### 2.5 Backend address (infra; realms S1; patch 0040)

- Variables, first set wins: `RUSTIC_ISLES_BACKEND_URL`, `DAINTY_BACKEND_URL`. Read once (an interlocked
  three-state word: unresolved, resolving, ready; readers spin until ready) into fixed buffers.
- Grammar, exactly (`BackendUrl.h`): `scheme://host[:port]`, scheme `http` or `https` (any case); host a DNS name
  (labels of letters, digits and inner `-`, each 1–63, total ≤ 253, last label not all digits, no trailing dot), a
  dotted quad (four parts 0–255, no leading zeros) or a bracketed IPv6 literal (RFC 4291 text, no zone id, an
  embedded dotted quad only as the last group); port 1–65535 with no leading zero, default 80/443. Rejected: empty,
  over 300 characters, any byte ≤ 0x20 or ≥ 0x7F, user info (`@`), any path, query or fragment (even a lone `/`),
  unbracketed IPv6, empty host or port. Each refusal has a fixed reason text that never echoes the input.
- Result: an API base `http(s)://host[:port]` and a chat address `ws(s)://host` plus port. A refused value is logged
  as reason and length only, then ignored.
- Fallback: `https://` + `MP_PUBLIC_HOST` and `wss://MP_PUBLIC_HOST` port 443. `MP_PUBLIC_HOST` comes from an
  optional `deployment_config.generated.h`; without it it is `backend.invalid`, a reserved name that never resolves, so
  a game started with no address fails closed.
- Who sets it: the launcher for a client, and (per the runtime's own note on patch 0040) the Dainty Lass Director for
  its game servers. An older note in the same file says a DeployServer sets none and keeps the fallback (? RT-15).

### 2.6 Files next to the executable

The runtime reads about 120 flag files (`*.flag`) and three number files from the game executable's folder:
`HUB_MAX_PLAYERS.txt`, `HUB_AFK_TIMEOUT_SECONDS.txt`, `HUNT_AFK_TIMEOUT_SECONDS.txt`. Presence is cached per
process unless `UNCACHED_FLAG_FILES.flag` exists. Every flag is a developer or host switch; none is shipped to
players. Appendix A lists them by the slice that owns them; each per-slice spec gives each one's meaning.

Logs: one file per port, `rustic_isles_dll_port<N>.log` next to the executable, flushed per line, each line
prefixed with a timestamp and the pid (a raw variant without the prefix is used by the networking code).
`PRODUCTION_LOG_PROFILE.flag` on a server skips the per-event traces. `MPLOG_OPEN_PER_LINE.flag` reopens the file per
line.

Other environment it reads: `RUSTIC_ISLES_RUNTIME_PROFILE=development` (set only by the launcher; unlocks dev
behaviour), `DAINTY_SERVER_MAX_FPS`, `DAINTY_SERVER_IDLE_FPS` (patch 0020), `RUSTIC_ISLES_TRIALS_WEEK`,
`RUSTIC_ISLES_BLEEDOUT_SECONDS`, `RUSTIC_ISLES_ALLOW_DAMAGE_DEBUG`, `RUSTIC_ISLES_DEBUG_DAMAGE_MULTIPLIER`,
`RUSTIC_ISLES_GAMESERVER_LAUNCH_ID`, and the player host's set (section 5.2). Every read is bounded in length and
refuses control characters.

### 2.7 Threads

| Thread | Role | Started by | Does |
| --- | --- | --- | --- |
| Loader thread (`DllMain`) | B | the loader | Init and every hook install |
| MainThread | B | Init | section 2.3 |
| Game thread | B | the engine | every detour body (ProcessEvent, engine tick, the guards) |
| Empty-server watchdog | S | the engine tick, after listen | reaps a disposable server (hunt, island, escalation) empty for 50 s; never a hub (section 4.1) |
| Server window hider | S | headless setup (HL-3) | hides any window the process creates, unless `SHOW_SERVER_WINDOW.flag` |
| Player-host control pipe | S (player host) | `P2PHooks::StartServer` | section 5.2 |
| Hitch sampler | C | client init, `HITCH_SAMPLER.flag` only | samples the game thread's stack on hitches (diagnostic) |
| Store capture worker | C | MainThread, `REWARD_CACHE_CAPTURE.flag` only | lane E |

Shared state across threads is small: the role, auth mode, port and argv copies (written in Init, read-only after),
the backend address buffers (section 2.5), the detaching flag, interlocked once-flags, atomics counting log budgets,
and the watchdog's published "game mode pointer, is hub" pair (written by the tick, read by the watchdog thread).

---

## 3. Every hook

**93 installed hooks under 92 ids** (HL-3 is two detours): 86 MinHook detours (64 on executable RVAs, 22 on
Windows exports), one vectored exception handler, one UFunction native-pointer swap, and 5 code or data patches.
About 37 more detours and probes are defined but never installed (section 3.12). The ids are stable: per-slice
specs, the ownership seam (section 7) and test notes cite them.

**How a target is found.** *RVA*: a fixed address in the 1.14.7 executable. *RVA + bytes*: the runtime first checks
that the RVA is a function entry in the executable's `.pdata` (following chained unwind info to the primary entry) and
that its first bytes match an expected prologue; otherwise it does not hook (fails closed). *pdata(x)*: the start of
the function containing RVA x, from `.pdata`. *Export*: `GetProcAddress` on a Windows DLL. *UFunction*: found by
class and function name at run time.

### 3.1 Dispatch points (slice R0 owns the chain; features register on them)

| Id | Role | Target | Found by | Original | What the hook does | Flow | Patch |
| --- | --- | --- | --- | --- | --- | --- | --- |
| SP-1 | C | `UObject::ProcessEvent` | RVA `+0x02861880` | dispatches a UFunction call | client feature dispatch (section 4.3): a depth guard absorbs calls nested deeper than 32 (a game recursion once overflowed the stack); a per-UFunction name cache; then each feature's before/after handling around the original | every client flow | many |
| SP-2 | S | same | same | same | server feature dispatch (section 4.2), same depth guard and cache | every server flow | many |
| SP-3 | S | `UEngine::Tick` (base) | RVA `+0x03E10AB0` | one engine frame | the server's per-frame driver: listen, readiness, pacing, watchdog, the graph drive and every per-tick feature (section 4.1) | everything on a server | 0014, 0015, 0020, 0038 |

### 3.2 Backend HTTP (slice R2)

| Id | Role | Target | Found by | Original | Hook | Flow | Patch |
| --- | --- | --- | --- | --- | --- | --- | --- |
| HTTP-1 | B | `FCurlHttpRequest::ProcessRequest` | RVA `+0x032C5FE0` | starts a prepared HTTP request; reads the URL FString at request `+0x70` | before the original: (1) rewrite the URL when its host is an allowlisted official host (exact or dot-subdomain of `steelyard.ca`, `steelyard.online`, `ol.epicgames.com`, `api.epicgames.dev`; http/https only) to `<API base>/__origin/<host><path?query#frag>`, growing the FString through the engine allocator (`FMemory::Realloc` `+0x023BB590`) and skipping a request already in flight (`+0xC4` = 1); (2) server: give a progression grant an idempotent grant id (section 4.2); (3) DeployServer: set header `x-rustic-isles-gameserver-apikey` to argv 1, via the request's virtual `SetHeader` (vtable slot 16); player host: the module's header (section 5.2). Always on: no flag can turn the rewrite off | login, all backend traffic, XP grants | infra |

### 3.3 Client login, EAC and the command line (slice R4)

| Id | Role | Target | Found by | Original | Hook | Flow | Patch |
| --- | --- | --- | --- | --- | --- | --- | --- |
| CL-1 | B | `FCommandLine::Get` | RVA `+0x025E8F70` | returns the engine's command line | builds once and serves the engine a line with `-NoEAC` and `-DisableEOS` appended when missing; server: `-nullrhi` carried from the OS line, the OS line's `MULTIHOME=` carried (the player host's listen depends on it, section 5.1), and the en-US culture switch (patch 0031). Records booleans of what the engine originally had. Installed before anything reads the line | login (EAC skipped), headless boot, player-host bind | 0031 |
| CL-2 | C | `IsEOSEnabled` | RVA `+0x01C30030` | decides EOS-SDK vs MCP login; caches early | returns 0, so the MCP login path runs (with only the OS flag, EOS auth ran and failed `EOS_Auth_ExchangeCodeNotFound`) | login | — |
| CL-3 | C | `LoginOperationFailed` | RVA `+0x01C3A160` | login failure callback | logs class and error text, then the original | login | — |
| CL-4 | C | `LoginOperationSuccess` | RVA `+0x01C3A590` | login success callback | logs, then the original | login | — |
| CL-5 | C | EAC `HandleError` | RVA `+0x01476F20` | reports an EAC error (modal) | absorbs only the expected private-client failure (IGameClient creation); every other error goes to the original | login | — |
| CL-6 | C | `UArchonGameInstance::EnqueueDisconnectError` | RVA `+0x018FB4D0` | queues a disconnect reason | logs the redacted reason and caller RVA, then the original | any disconnect | — |
| CL-7 | C | `EnableEasyAntiCheatFeatures` byte | data `+0x06CFBD44` | config byte EAC's StartupModule reads | written 0 at client init when readable, so StartupModule takes its own no-load path (the gate is "file present or always": effectively always) | login | — |
| CL-8 | C | `ws2_32!GetAddrInfoW` | export | DNS | the retired QoS latency-test hosts resolve to loopback, so the client measures a region | matchmaking region | — |
| CL-9 | C | `ws2_32!getaddrinfo` | export | DNS | same, ANSI | same | — |
| CL-10 | B | `ws2_32!sendto` | export | send | client: logging only with `SOCKET_DIAG.flag`; server: counts bytes per peer for the `[PerfNet]` line unless `NO_PERF_DETAIL.flag` | diagnostics | — |
| CL-11 | B | `ws2_32!recvfrom` | export | receive | logging, `SOCKET_DIAG.flag` only | diagnostics | — |
| CL-12 | B | `ws2_32!WSARecvFrom` | export | receive | same | diagnostics | — |
| CL-13 | B | `ws2_32!WSARecv` | export | receive | same | diagnostics | — |

CL-10 to CL-13 are installed by one function; whether the server installs the receive hooks without the flag is (?)
RT-16 (I read the gate as: receive hooks only with the flag, sendto also on servers).

### 3.4 Process exit and crash handling (slice R3)

| Id | Role | Target | Found by | Hook | Patch |
| --- | --- | --- | --- | --- | --- |
| EXIT-1..11 | B | `kernel32!ExitProcess`, `ntdll!RtlExitUserProcess`, `ntdll!NtTerminateProcess`, `kernelbase!TerminateProcess`, `kernelbase!RaiseException`, `ucrtbase!exit`, `!_exit`, `!abort`, `msvcrt!exit`, `!_exit`, `!abort` | export | log the exit code (or exception code) with a bounded stack of module+RVA frames and the thread, then the original. Silent after detach | — |
| EXIT-12 | B | vectored exception handler | `AddVectoredExceptionHandler(1, …)` | logs selected exception codes with registers and stack; **recovers** one known fault: an access violation inside the spline curve evaluator at `+0x1091D5D` returns a zero result instead (pairs with NETD-7) | — |
| EXIT-13 | S | `FWindowsErrorOutputDevice::HandleError` | RVA `+0x02659A20` | logs the error-history text (a UTF-16 buffer at `+0x06E14AD0`, read with a 1023-character bound through a fault-safe read), **then always calls the original** (an earlier version swallowed it and the engine ran on corrupted) | — |
| EXIT-14 | S | `ReportCrash` wait | patch at `+0x0265CF15` (inside `+0x0265CEC0`) | bytes `BA 60 EA 00 00` (`mov edx, 60000`) become `BA 88 13 00 00` (5000): a stalled crash reporter froze every client for 60 s. Bytes verified first; `DISABLE_CRASH_WAIT_SHORTEN.flag` | — |

`UEUnhandledExceptionFilter` is deliberately not hooked on 1.14.7: `+0x02657770` is a formatter with another ABI.

### 3.5 Headless server boot (slice R5)

| Id | Role | Target | Found by | Hook | Flow | Patch |
| --- | --- | --- | --- | --- | --- | --- |
| HL-1 | S | `GIsServer`, `GIsClient` | data `+0x06E1EADA` = 1, `+0x06E1EAD9` = 0 | written in Init | server boot | — |
| HL-2 | S (every server role) | `UGameEngine::Init` | patch at `+0x03E07353`: `0F 84 D3 00 00 00` (`jz`) → `E9 D4 00 00 00 90` (`jmp` same target; `nop`) | skips the viewport-client and first-local-player block, so a server has no window, viewport or local player ("phantom"); the first real login starts the match (section 4.2). Verified bytes or loud failure; there is no phantom suppression any more | hub and hunt boot | — |
| HL-3 | S | `user32!CreateWindowExW`, `user32!ShowWindow` | export | windows hidden; plus the hider thread; `SHOW_SERVER_WINDOW.flag` off | headless | — |
| HL-4 | S | `WebBrowserWidget::StartupModule` | RVA + bytes `+0x00F357D0` | replaced (original never called): no Chromium/CEF on a server (one `UnrealCEFSubProcess` of ~111 MB per hub otherwise). `KEEP_WEB_BROWSER.flag` | headless | — |
| HL-5 | S | Wwise output-device init | RVA `+0x017B1620` | opt-in `DISABLE_SERVER_AUDIO.flag`: return without calling (measured: silences the hub but crashes boot; left off) | — | — |
| HL-6 | S | `kernel32!LoadLibraryExW` | export | opt-in `MP_FORCE_WARP.flag`: when `d3d11.dll` loads, hook HL-7/HL-8 | GPU-less VPS | — |
| HL-7, HL-8 | S | `d3d11!D3D11CreateDevice`, `D3D11CreateDeviceAndSwapChain` | export | opt-in: force driver type WARP (5) with a null adapter | same | — |
| HL-9 | S | the RHI `IsSupported` branch | patch at `+0x034AB73D`: `84 C0 75` → `84 C0 EB` | opt-in with HL-6 | same | — |
| HL-10 | S | `GetViewportSize` exec thunk | RVA `+0x0398BB50` | headless viewport fallback (vendor camera `UpdateCameraForAspect` divided by a 0×0 viewport) | Ramsgate vendors | — |
| HL-11 | S | `UWidgetLayoutLibrary::GetViewportSize` | UFunction native-pointer swap, retried once a second from the tick until found | after the original, a (0,0) answer becomes 1920×1080 (the War Pike special divided by it and missed). `DISABLE_SERVER_VIEWPORT_SIZE.flag` | hunts | 0038 |

Also server-only, no hook: the process locale is set to `C` for numbers (patch 0031: Behemoth part breaks failed
under a decimal-comma locale; `DISABLE_SERVER_LOCALE_FIX.flag`), and the nullrhi cache byte (`+0x06CD7878`, guard
`+0x06CD787C`) is only logged, never written (writing it once gave every hub a hardware D3D device).

### 3.6 Widget crash guards (slice R6)

Server roles build widgets without the sub-widgets they bind, and several natives dereference them unchecked.

| Id | Role | Target | Found by | Hook | Flow |
| --- | --- | --- | --- | --- | --- |
| UI-1 | S | `UInteractionCalloutWidget::HideHoldText` | RVA `+0x01E42F00` | skip when the member at widget `+0x3C0` is null | Ramsgate boot |
| UI-2 | S | `UArchonLoadingScreen::ScreenFadeIn` | RVA `+0x01E0FBB0` | skip when `+0x3E8` is null (fault at `+0x01CADEA6`) | failed player load. Compiled off on the client |
| UI-3 | S | `UInteractionCalloutWidget::OnSetDisabled` | pdata(`+0x01E5414E`) | skip when `InteractionHint` (`+0x3A8`) is null | Training Grounds |
| UI-4..6 | S | `RefreshInputKeys` `+0x01E3EB10`, `NativeConstruct` `+0x01E4BA00`, `NativeOnInputSwap` `+0x01E4E050` | RVA + bytes, and each parent call's target verified | all-or-nothing group: run the parent, drop the visual work on a missing sub-widget (Keyboard `+0x388`, Gamepad `+0x390`, RadialAnimImage `+0x398`) | hunt (fault at `+0x1E3EB23`) |
| UI-7..11 | S | `NativeTick` `+0x01E50A70`, `OnShowFillProgress` `+0x01E544B0`, `OnHideFillProgress` `+0x01E52450`, `OnSetShowDisabledIconWhenEnabled` `+0x01E542C0`, `OnSetShowDisabledTextWhenEnabled` `+0x01E54320` | RVA + bytes | each on its own, same rule | hunt |
| UI-12 | S | inventory equipped-items refresh | RVA `+0x01B86160` | skip when the joining player's active loadout slot is still -1 or any of five equipped-item getters (`+0x01BA4FF0`, `+0x01B9CA60`, `+0x01B9FF30`, `+0x01B9A620`, `+0x01BA08D0`) is null; `DISABLE_INVENTORY_REFRESH_GUARD.flag` | join Ramsgate |
| UI-13 | C | `HUDLegendaryAbilityWidget::HandleWeaponEquipped` | RVA + bytes `+0x01F6D4F0` | skip a null weapon (late join); `DISABLE_HUD_LEGENDARY_GUARD.flag` | late join a hunt |

UI-4..11 are off with `DISABLE_CALLOUT_GUARDS.flag`.

### 3.7 Player data, progression and loading (slice R7)

| Id | Role | Target | Found by | Hook | Flow | Patch |
| --- | --- | --- | --- | --- | --- | --- |
| LD-1 | B | `HasFinishedLoading` evaluator | RVA `+0x01BA5E40` | the loading gate. Server: keeps a compatibility override on a false answer. Client: overrides only for the exact local `AArchonPlayerController`, and never while its character has no weapon; no forced ready 3 s after ClientRestart in a hunt | loading screens | 0011, 0013 (weapon-gap log) |
| LD-2 | S | `OnPlayerDataLoadComplete` native body | RVA `+0x01CB33B0` | marks the player's data loaded (feeds the restart gate, section 4.2) | hunt restart | 0012 |
| LD-3 | S | `UArchonLoadManager::LoadFailed` | RVA `+0x01CB0010` | a one-shot retry of a loader stuck loading; drops the failure of a player state with no net id (was the phantom; `KEEP_PHANTOM_LOAD_FAILURE.flag`) | hub survives a failed load | 0004 |
| LD-4 | B | `UArchonLoadManager` Begin | RVA `+0x01C997A0` | read-only identity log | diagnostics | — |
| LD-5 | S | progression configuration predicate | RVA `+0x019053A0` | server: issue the progression-config query at the first readiness check and once at start, so no player waits the 60 s timeout; `FORCE_PROGRESSION_CONFIG_READY.flag` | hunt loading | 0008 |
| LD-6 | S | progression `CompleteLoadInternal` | RVA `+0x01A4DE80` | notes completion | hunt loading | 0008 |
| LD-7 | S | progression `ReadFromBackend` | RVA `+0x01A6AAB0` | records components parked waiting for the config; the tick resumes them (`PROGRESSION_WAIT_RESUME.flag` (?)) | hunt loading | 0008 |
| LD-8 | S | progression config cache lookup | RVA `+0x01902E60` | read-only probe | diagnostics | — |
| LD-9 | S | progression readiness predicate | RVA `+0x01A69560` | probe; `FORCE_PROGRESSION_READY.flag` sets the four server flags | diagnostics | — |

### 3.8 Net driver, replication graph and relevance (slices R11, R12, R13)

| Id | Role | Target | Found by | Hook | Flow | Patch |
| --- | --- | --- | --- | --- | --- | --- |
| NETD-1 | S | `UNetDriver::TickDispatch` inner helper | RVA `+0x03FAE420` | validate `ClientConnections` (`+0x90`, `+0x98`) before the walk (fault `+0x03D91CEB`) | joins/leaves | — |
| NETD-2 | S | `UNetDriver::SetReplicationDriver` | RVA `+0x03FACF60` | installs the replication driver | server boot | — |
| NETD-3 | S | `CreateReplicationDriver` | RVA `+0x03F8D8E0` | force-populates the enable gate (data `+0x06AD6F54`) and the graph class, using the feature array at `+0x06FB12E0` | server boot | — |
| NETD-4 | S | `InternalGetNetMode` | RVA `+0x03F91EB0` | answers dedicated server | everything | — |
| NETD-5 | S | `UWorld::GetNetMode` | RVA `+0x042CC560` | same (`+0x042CC6E0` returns GIsClient; never hook it) | everything | — |
| NETD-6 | B | `UNetConnection::Close` | RVA `+0x03F62B90` | log scalar fields and caller stack, then the original | disconnect diagnosis | — |
| NETD-7 | S | spline direction query | RVA `+0x03D47A90` | zero direction for a corrupt curve pointer (`+0x448`); `DISABLE_SPLINE_GUARD.flag` | hub after ~90 s | — |
| NETD-8 | S | spline curve evaluator | RVA `+0x01091D50` | opt-in `ENABLE_SPLINE_EVAL_HOOK.flag` (measured unsafe; EXIT-12 recovers instead) | — | — |
| RG-1 | S | `UArchonReplicationGraph::ServerReplicateActors` | RVA `+0x01C034C0` | pass-through counter | diagnostics | — |
| RG-2 | S | `RouteAddNetworkActorToNodes` | RVA `+0x01BFE430` | normalise every PlayerController before the graph files it; route PlayerRoles and weapons (a dormant weapon is not static; an always-relevant weapon is not distance-culled) | 2+ players, weapons | 0015, 0016 |
| RG-3 | S | `RouteRemoveNetworkActor` | RVA `+0x01BFE4C0` | pass-through, role membership log | diagnostics | — |
| RG-4 | S | `UReplicationGraph::ReplicateSingleActor` | RVA + bytes `+0x00F2C230` | PlayerController pre-channel guard | 2+ players | — |
| RG-5 | S | `UActorChannel::SetChannelActor` | RVA `+0x03D8FB20` | refuse binding a PlayerController to another player's connection | 2+ players | — |
| RG-6 | S | `UActorChannel::ReplicateActor` | RVA `+0x03D8A2D0` | the final fail-closed guard (else stock `OnActorChannelOpen` replaces the receiver's controller); quarantines the channel; frequency tally with `REPGRAPH_DIAG.flag` | 2+ players | — |
| RG-7 | S | `UNetDriver::ProcessRemoteFunctionForChannel` | RVA `+0x03FA09B0` | drop an RPC on a channel whose bind was refused; `DISABLE_RPC_UNBOUND_CHANNEL_GUARD.flag` | 2+ players | 0039 |

Per tick (no hook of their own, owned by these slices): the replication-graph drive (the engine's own TickFlush never
drives the graph on this build), the channel quarantine sweep, the weapon-gap log and dormancy wake (patches 0014,
0015), the relevance settings pass, and the manual net-tick paths behind `MANUAL_NET_TICK.flag` and kin.

### 3.9 Game features (slices R14–R16)

| Id | Role | Target | Found by | Hook | Flow | Patch |
| --- | --- | --- | --- | --- | --- | --- |
| PR-1 | B | `ApplyPlayerRole` | RVA `+0x01B92B90` | runs the omnicell role application on the owning client too (the role at character `+0x738` is not replicated) and records validity for retries | omnicell abilities, HUD | — |
| ESC-1 | S | `FGameplayScheduler::Initialize` | RVA `+0x01D95B70` | before the scheduler copies its rows, stretch the arena schedule row's window to cover now (`RUSTIC_ISLES_TRIALS_WEEK` picks the week), so the Trials are current and launchable | Trials | — |
| SRV-1 | S | `GetGameDefaultMap` getter | RVA `+0x02F00330` | returns the map URL: map path, `?MonsterClass=`, `?HuntId=`, `?PlayerHuntIds=` (each unless its `NO_` sentinel), always `?MaxPlayers=<cap>`, and `?listen` with `MAP_URL_LISTEN.flag` (never on a player host); written into the engine-owned FString through the engine allocator | every server map load | — |
| TR-1 | C | `UEngine::LoadMap` | RVA + bytes `+0x04271370` | observe destination, old world and every UWorld before and after; opt-in `ENABLE_ZOMBIE_LOADMAP_GUARD.flag` fails a travel whose destination world is a pending-kill leftover; `NO_LOADMAP_PROBE.flag` | hunt → Ramsgate | — |

### 3.10 Diagnostics (slice R19, or not ported: RT-3)

| Id | Role | Target | Found by | Hook | Gate |
| --- | --- | --- | --- | --- | --- |
| DX-1 | S | inventory transaction body builder | pdata-checked RVA `+0x00DBB560` | log source, ids, call chain | off-flag `GRANT_TRACE_OFF.flag` |
| DX-2 | S | cooldown start time | pdata-checked RVA `+0x01A032F0` | log | with DX-1 |
| DX-3..7 | S | kill XP chain: weapon level `+0x01A57A50`, base XP `+0x01A57F80`, amount `+0x01A44EC0`, `GrantWeaponExperience` `+0x01A62540`, Hunt Pass add `+0x01A1BCF0` | pdata-checked RVA | pass-through logs | off-flag `KILL_XP_TRACE_OFF.flag` |
| DX-8 | S (DeployServer) | `UArchonGameplayStatics::TickFilterHelper` | RVA + bytes `+0x019480B0` | opt-in: answer for `BP_PlayerCharacter_C` / census | `SERVER_TICK_FILTER.flag`, `TICK_FILTER_CENSUS.flag` |
| DX-9 | S (DeployServer) | `execTickFilterHelper` | RVA + bytes `+0x0229DB70` | same | same |

### 3.11 Install order

 Client: MinHook init, hitch sampler, NETD-6, LD-1, SP-1, UI-13, TR-1, PR-1, HTTP-1, CL-2, CL-3, CL-4,
LD-4, CL-1, CL-8/9, CL-10..13, CL-7, CL-5, CL-6, EXIT-1..12. Server: MinHook init, CL-1, locale, headless setup
(HL-3 and the nullrhi log), HL-4, the player host's hooks (section 5.2), EXIT-1..12, SP-2, LD-2, ESC-1, PR-1, SRV-1,
SP-3, UI-1, UI-2, NETD-1, HTTP-1, NETD-2, NETD-7, NETD-8, RG-1, RG-2, RG-3, RG-4, RG-5, RG-6, RG-7, NETD-3, NETD-4,
NETD-5, NETD-6, LD-1, HL-10, EXIT-13, LD-3, LD-4, LD-5..7, DX-1..7, HL-5, CL-10..13, LD-8, LD-9, UI-3, UI-4..11,
EXIT-14, UI-12, HL-6..9, HL-2, DX-8/9. Two order rules are real: CL-1 and HL-4 first (before the engine reads them),
and DX-8/9 after HL-2 (they need its result). The rest is order of writing (? RT-17).

### 3.12 Defined but never installed (not ported)

The C++ keeps these detours or probes with no install path on 1.14.7 (an unresolved 1.12 RVA, a refuted ABI, or a
finished investigation): the XMPP `GetConfigString` redirect (`+0x025E6890`, see RT-7), `GetCommandLineW/A`,
`FCommandLine::Set`, `FMessageDialog::Open`, the EAC error proc and startup module, `HasDisconnectError`
(`+0x0227BCF0`), `UEUnhandledExceptionFilter`, the Tempest scale probes (`+0x0156D7D0`, `+0x03CCF210`, refused),
knockout, `MakeDoDamage`, Frank drone `CanActivateAbilityBP`, encounterable setup, the old `ServerReplicateActors`
(`+0x03F90E20`), `GetStartSpot`, `IsNetReady`, `IsLevelInitForActor`, `NotifyClientDisconnected`, `FixupNetworkNotify`,
`AGameMode::PostLogin`, the stamina `TryConsumeStamina`, the native `GetCommandLine` (`+0x0243A310`, mid-instruction),
the loader-frame and parse probes, `SetURL` (`+0x03102740`), the vtable helper, and six progression probes
(active Hunt Pass driver with its window lookup, `LoadComplete`, `FindRow`, `ConfigComplete`, game-instance query,
mastery/unclaimed rewards), plus the `#if 0` 1.4.4 GIsServer code patches. None has to be re-created.

---

## 4. Feature handling on the dispatch points

These features have no hook of their own: they run inside SP-1, SP-2 or SP-3 when a UFunction name or a tick matches.
Each per-slice spec lists its UFunction names exactly; here they are only grouped. Name matching is on the
UFunction's full name (cached per UFunction), by equality, prefix or substring (?) RT-18.

### 4.1 Server tick (SP-3)

In order each frame: frame-time sample and a new "pointer checked this frame" epoch; patch 0020 pacing (below);
HL-11 retry; the optional membership probes; force dedicated-server mode again (the engine resets it); net-driver
sanitising; the original engine tick; the channel quarantine sweep; weapon-gap log and dormancy wake (0014, 0015);
the graph drive; resuming parked progression components (LD-7); the arrival-trigger and Hunt Pass probes; the
optional server mesh pose fix (`FORCE_SERVER_MESH_POSE.flag`); bleedout sampling (diagnostic); once a second the
player-role retries, post-activation refresh, Tempest modifier ensure and escalation buff-choice spawn; the expected
player count from the roster; connection-edge and match-state traces; the manual net tick (opt-in); then **listen**:

- once "listen requested" is set: with `MAP_URL_LISTEN.flag` wait for the engine's listener and announce it, else call
  `Networking::Listen(port)` (section 5.1) unless `DISABLE_MANUAL_LISTEN.flag`; on success start the watchdog.
- the watchdog: a disposable server (not `BPGM_City_C` or `BPGM_ArchonTrainingGrounds_C`, and not a player-host hub)
  with no client for 50 s ends itself; the hub cap from `HUB_MAX_PLAYERS.txt` or the player host's override; the AFK
  timeouts from the `*_AFK_TIMEOUT_SECONDS.txt` files or the override.

Patch 0020 pacing: `DAINTY_SERVER_MAX_FPS` (default 30, 10–240, 0 = no cap) while a client is connected;
`DAINTY_SERVER_IDLE_FPS` (default 0 = same; 1–240) once listened with no connection for 15 s straight; boot, map
load and the first 15 s after listening are uncapped; the first connection returns to full rate next frame. Real
sleeps, so `DeltaTime` stays real. Read once; an invalid value falls back to the default and is logged.

### 4.2 Server ProcessEvent (SP-2)

Grouped: post-login (`K2_PostLogin`, `PostLogin`: start a match still waiting without a phantom, set the game state's
AFK timeout before it replicates, reset capture windows); possession (`AcknowledgePossession`, `AcknowledgePawn`,
`CheckClientPossession`); the vendor interaction fix (`vendor_interactive_bp_C.UpdateCameraForAspect`);
`PlayerCanRestart` forced true and must-spectate handling; **the restart data gate** (patch 0012: in a hunt, a
connected player is not restarted until LD-2 marked their data loaded; `DISABLE_RESTART_DATA_GATE.flag`); pawn
replication and owner-pawn relevancy fixes; escalation round transitions; XP grant inputs (`PlayerExperienceComponent.*`)
and the grant id used by HTTP-1; economy traces (`ConsumeItem`, `GrantAndConsumeItemsWithQuantity`, bounty, draft,
seasonal coin; diagnostic); Behemoth damage traces (diagnostic); `ClientWasKicked`, `ClientReturnToMainMenu` (who sends
a player to the menu; diagnostic); the player host's observer (section 5.2).

### 4.3 Client ProcessEvent (SP-1)

Grouped: every call first runs patch 0024 (supplies lose their stack cap: a few tonics; `KEEP_SUPPLY_STACK_CAPS.flag`)
and patch 0029 (the Middleman's fusion recipes take 30 minutes; `KEEP_CELL_FUSION_TIME.flag`), each a one-time
data write once the objects exist; network and travel failure logs (`HandleNetworkError`, `HandleTravelError`); the
world census every 250 ms (TR-1's travel probe); login screen, credits and press-start branding (`LoginScreen`,
`bpw_Credits_C.PreConstruct`, `PressStart`; `DISABLE_*_BRANDING.flag`, `AUTO_PRESS_START.flag`; RT-9); ClientRestart
capture and input activation (`EnableCharacterInput`, `OnInputModeChanged`, lantern and role inputs); HUD
reconciliation and progression HUD refresh (`*_HUD_*`, `OnAirshipStateUpdate`, `ClientIslandGameplayStarted`);
player role charge (`ArchonPlayerRole.*`); escalation relic choice (`*Relic*`, `EscalationGameModeComponent.*`);
loadout (patches 0026 trace, 0027 and 0037 repair: when the inventory arrives after the arsenal looked its items up,
the slot's items, cells, parts, dye and transmog are re-applied; `DISABLE_LOADOUT_SLOT_REPAIR.flag`); Tempest and
damage traces (diagnostic).

---

## 5. Networking and the player host

### 5.1 `Networking.cpp` (slice R9)

- `Listen(engine, port)`: find the world context for the current world; create the `GameNetDriver` with
  `CreateNamedNetDriver` (RVA `+0x0425FDA0`); find it (the active-net-drivers list, else a scan of all objects); name it,
  clear `ServerConnection`, set `World`; build an `FURL` with the port; let the player host refuse or pin the address
  (a pinned address must equal the engine command line's `MULTIHOME`, which CL-1 carries, else refuse); call the
  driver's virtual `InitListen` (vtable `+0x290`) with the world's notify (`World+0x28`). On success: readiness
  (below). False on any failure.
- Readiness: a DeployServer prints `RUSTIC_ISLES_GAMESERVER_READY launchId=<RUSTIC_ISLES_GAMESERVER_LAUNCH_ID> port=<n>`
  and the legacy `MYSTICPARADOX_GAMESERVER_READY …` on stdout (the Director waits for one); a player host reports
  over its pipe instead (5.2). `AnnounceEngineReady` does the same when the engine made the listener.
- `TickNetworking`: per-tick networking work, and a legacy manual replication loop that is disabled on 1.14.7
  (it calls 1.12 RVAs that are not entries here).
- `BootstrapActorChannel(actor, connection)`: open and send an initial actor channel for owner equipment routed into
  the graph without one (`CreateChannelByName` `+0x03F62E90`, `SetChannelActor` `+0x03D8FB20`, `ReplicateActor`
  `+0x03D8A2D0`, the last two being RG-5/RG-6's targets).
- `GetActorChannelIndexForConnection`: read-only probe (channel index, -1, -2).
- Switches: `NATIVE_REPLICATION_ONLY`, `REVERSE_CONNECTION_ORDER`, `HYBRID_REPLICATION`, `LEVEL_VISIBILITY_GATE`,
  `NATIVE_GRAPH_ONLY` (?), `REPGRAPH_DIAG` flags.

### 5.2 The player host (`p2p/PlayerHost.cpp`, slice R10)

Behind the `P2PHooks` interface (the stub answers "not claimed" to everything). Realms' hosted hubs and hunts.

- Claims `RUSTIC_ISLES_AUTH_MODE=HUNT_CAPABILITY`. Valid bootstrap needs: argv 1 is `-`; `-MULTIHOME=127.0.0.1`;
  `RUSTIC_ISLES_BIND_ADDRESS=127.0.0.1`; `RUSTIC_ISLES_SESSION_ID` (36 safe characters); `RUSTIC_ISLES_SESSION_GENERATION`
  (1–10 digits); `RUSTIC_ISLES_HOST_RUNTIME_CAPABILITY` (43 safe characters); `RUSTIC_ISLES_CONTROL_PIPE` and
  `RUSTIC_ISLES_READY_PIPE` (prefix `\\.\pipe\MysticInternalServer.`); `RUSTIC_ISLES_GAMESERVER_LAUNCH_ID` (16–96);
  `RUSTIC_ISLES_LAUNCHER_PID` (digits, non-zero); optional `RUSTIC_ISLES_HOSTED_ROLE` (`HUNT`|`HUB`),
  `RUSTIC_ISLES_MAX_PLAYERS` (1–64), `RUSTIC_ISLES_HUB_AFK_SECONDS` (0–7200), `RUSTIC_ISLES_HUNT_GUARD_EVENTS=1`.
  Any invalid present value ends the process before listen.
- Control pipe (a thread): one message-mode instance, remote clients rejected; the connecting process id must be the
  launcher's; the only accepted message is `MYS1 FENCE session=<id> generation=<g>\n`, answered
  `MYS1 FENCE_ACK session=<id> generation=<g>\n`. Any failure or the fence itself sets "fence requested": the next tick
  quits the game (exit 0), and after 5 s terminates with 0xE105.
- Ready pipe: after listen, open the ready pipe (wait up to 5 s, retry a busy pipe for 5 s) and write
  `MYS1 READY session=<id> generation=<g> launch=<launchId> port=<n> bind=127.0.0.1\n`.
- Backend header: `x-mystic-host-capability: <capability>` on every backend request (HTTP-1). Never the gameserver key.
- Hub role: the hub cap and AFK overrides (4.1); the watchdog never reaps it.
- Hunt Guard: `AntiCheatHooks` is the stub in this tree (no evidence).

**D3 is not applied here** (lane F1's finding, confirmed): the runtime still sends `x-mystic-host-capability` and
uses `MYS1` and `MysticInternalServer.`; `spec-rd27-rifts.md` D3 renames the header to `x-rustic-host-capability`
and requires the hosted runtime to cut over with the server and launcher. The MIT runtime should be born with D3's
names; question RT-8 covers the pipe prefix and `MYS1`, which D3's table does not name.

---

## 6. The runtime patches, mapped

| Patch | Ids and features | Slice |
| --- | --- | --- |
| infra (backend address, fail closed) | section 2.5, HTTP-1 | R1, R2 |
| infra (XMPP `bUseSSL` follows the scheme) | the XMPP redirect is defined but not installed on 1.14.7 (RT-7) | — |
| 0004 | LD-3 | R7 |
| 0008 | LD-5, LD-6, LD-7 | R7 |
| 0011, 0013 | LD-1 (0013 is its weapon-gap log, droppable) | R7 |
| 0012 | SP-2 restart gate + LD-2 | R7 |
| 0014 (droppable log), 0015, 0016 | SP-3 per tick + RG-2 | R13 |
| 0020 | SP-3 pacing | R8 |
| 0024, 0029 | SP-1 data writes | R15 |
| 0026 (droppable log), 0027, 0037 | SP-1 loadout repair | R15 |
| 0031 | CL-1 culture switch + locale | R4, R5 |
| 0038 | HL-11 | R5 |
| 0039 | RG-7 | R12 |
| 0040 | section 2.5 and 4.1 variable names | R1, R8 |

0024 and 0029 are policy, not fixes; the patches page already names them as `settings.toml` candidates (RT-11).

---

## 7. How an MIT runtime loads beside the old one

Slices land one at a time, each play-tested, while the AGPL runtime keeps every hook not yet ported. Three problems
need solving, and each needs a change outside this lane:

**1. Two DLLs in one process.** Recommended: the loader learns a second fixed name, `rustic-isles-runtime.dll` (MIT),
loaded **before** `Rustic-Isles.dll` when present; the launcher's known-DLL list and the runtime feed's manifests and
hashes take it too. Not the ini: RD9 keeps `mystic_loader.ini` for host-picked tools and PLAY refuses while one
exists. (Lane B's loader spec and the launcher-kit's `loader_session.rs` own those changes; report them, don't make
them here.)

**2. Two DLLs hooking one target.** Never let two MinHook copies detour the same function. The MIT DLL exports a
**hook broker**: for a target it creates exactly one detour and runs a chain of registered wrappers, each given the
next function to call (outermost first: MIT features, then AGPL ones, then the original). SP-1/2/3, HTTP-1, LD-1,
PR-1 and NETD-6 are shared by many features and need it from the start; any other target can go through it too.

**3. A feature done twice, or not at all.** Each feature id in this page is owned by exactly one DLL. The MIT DLL
exports an ownership query (`owns(id)`); the AGPL runtime asks it before installing each hook and before each feature
block inside its dispatch bodies, and does the work only when the MIT side does not. When the MIT DLL is absent, the
AGPL runtime behaves exactly as today.

That means **one seam patch to the AGPL runtime** (a runtime patch in `restricted/`, applied like any other, RT-4):
route every hook creation through the broker when present, and wrap each id's install and each feature block of
SP-1/2/3 in the ownership check. After it, a slice is purely additive on the MIT side: implement, report `owns`, ship.
The AGPL DLL stops loading when every id is owned (the last row of section 8), and the broker's AGPL chain slot goes
with it.

Rules every slice keeps: a feature moving across changes its place in a wrapper chain, so a slice spec states whether
its handler runs before or after the original and before or after other features on the same dispatch point; and
nothing may log or touch the CRT after process detach, in either DLL.

The alternative, a per-slice AGPL patch deleting the moved hooks with no broker, is simpler but cannot share SP-1/2/3:
it forces all ProcessEvent and tick features into one slice of about 6k lines, which breaks the 1–3k rule.

---

## 8. Slicing proposal

Sizes are today's C++ lines the slice replaces (approximate, from the file's layout); an MIT rewrite without the
diagnostics is smaller. Each slice is its own module folder in the MIT crate, so no two slices touch the same file
except the shared registry in R0, which each slice extends by one line (section 7's id list). "Deps" are the slices
that must have landed first; any slices whose deps are met can run two or three at a time. The `#` is the slice's id;
the rows are in the suggested order, smallest first where the deps allow.

| # | Slice | Ids | ~Lines | Deps | Maintainer's play-test |
| --- | --- | --- | --- | --- | --- |
| R0 | Spine: logging, flag files, environment reads, pointer-safety helpers, process-detach rule, MinHook, the broker and ownership exports, the dispatch chains for SP-1/2/3 (depth guard, name cache) with no features | SP-1, SP-2, SP-3 (chains only) | 1 300 | seam patch, loader name | Both DLLs load; login, Ramsgate, a hunt, the store and a realm behave exactly as today (all features still AGPL). Client exit is clean |
| R1 | Config: backend URL grammar, `MYSTICPARADOX_` alias, `MP_PUBLIC_HOST` fallback, launch-argument parsing (2.4), the FPS, trials-week and number-file parsers | none (pure) | 500 | — | None: inert until R2/R8/R9. CI unit tests only |
| R2 | Backend HTTP | HTTP-1 (+ grant id) | 900 | R0, R1 | Login; Ramsgate store opens; a hunt's kill XP is saved (grant id); `DAINTY_BACKEND_URL` to a bad value logs the reason and fails closed |
| R15 | Inventory and economy (client) | SP-1 handlers 0024, 0026, 0027, 0029, 0037 | 400 | R0 | Tonic stacks beyond the cap; a Middleman fusion takes 30 min; arsenal keeps its loadout when the inventory arrives late |
| R17 | Client branding and console | SP-1 handlers: login background, credits, press start; F2 console | 600 | R0, RT-9 | Login screen shows the Dainty Lass background; credits; F2 opens the console |
| R5 | Headless server boot | HL-1..11, locale | 700 | R0 | Start a hub: no window, no sound, no CEF processes, GPU idle; a War Pike special hits a Behemoth (0038); optionally WARP on a GPU-less box |
| R3 | Exit and crash | EXIT-1..14 | 800 | R0 | Client quits cleanly; a forced server crash logs and exits within 5 s, clients not frozen |
| R4 | Login and command line | CL-1..13 | 900 | R0 | Login with no EAC dialog, region found (QoS), Ramsgate; a hub boots `-nullrhi`; a realm host listens on 127.0.0.1 |
| R6 | Widget crash guards | UI-1..13 | 1 000 | R0, R5 | Ramsgate boots; Training Grounds; a hunt with interaction callouts; late join a hunt with a legendary weapon |
| R16 | Travel | TR-1, client world census and leftover-world release | 1 000 | R0 | Hunt → Ramsgate three times, no crash |
| R7 | Player data and loading | LD-1..9, SP-2 restart gate | 1 800 | R0, R2 | Fresh install joins Ramsgate and stays past 120 s; a hunt loads in seconds, not a minute; a restarted hunt; a hub runs 10 min |
| R9 | Listen and readiness | SRV-1, `Networking.cpp`, SP-3 listen step, MainThread | 2 000 | R0, R1, R4 | Director starts a hub, READY seen, login to Ramsgate, start a hunt |
| R8 | Server lifecycle | SP-3: watchdog, hub cap, AFK, pacing 0020, quit | 1 500 | R9 | Empty hunt ends after 50 s, hub never; AFK kick; idle hub's CPU drops after 15 s |
| R10 | Player host (realms) | `P2PHooks`, control and ready pipes, header, hosted roles; D3 names | 600 | R9, RT-8 | Host a realm hub and a hosted hunt; the launcher fences it |
| R11 | Net driver safety | NETD-1..8, EXIT-12's spline recovery | 1 300 | R0, R3 | Hub with two clients joining and leaving for 10 min; "Go Now" from a hunt |
| R12 | Replication graph and channel guards | RG-1, RG-3..7, the graph drive, quarantine | 1 800 | R11 | Two-player hunt: both see each other, nobody's controller swaps; RPC guard quiet |
| R13 | Relevance and weapons | RG-2, 0014–0016, relevance settings, route base actors | 2 000 | R12 | Two-player hunt: each sees the other's weapon from the start and after dormancy |
| R14 | Player role (omnicell) | PR-1, server routing and retries, client input activation, HUD lifecycle, progression HUD | 3 000 | R12, R7 | Each omnicell's ability and HUD charge, lantern, airship HUD restore after a hunt |
| R18 | Escalation and Trials | ESC-1, buff choices, relic choice | 1 200 | R0 | Trials ungreyed and launchable; an escalation's relic choices |
| R20 | Server ProcessEvent behaviours | SP-2 handlers: post-login, possession, vendor fix, restart, spectate, pawn relevancy | 1 400 | R9, R12 | Vendors in Ramsgate; respawn in a hunt; join mid-hunt |
| R19 | Diagnostics | DX-1..9 and the other logging-only code | ~6 000 today | — | Per RT-3: recommended not ported |
| R21 | Retire the AGPL runtime | — | — | every slice | The full checklist on the MIT runtime alone: login, Ramsgate, a hunt, the store, realms |

R14 is the one slice above 3k: the server routing and the client side are entangled through the role's replication.
If the maintainer prefers, split it after R12 lands into R14a (server, ~1.8k) and R14b (client, ~1.2k), with R14b
depending on R14a.

Suggested waves: R0 alone (it needs the seam patch and the loader name), with R1 beside it; then R2, R15, R17; then
R5, R3, R4; then R6, R16, R7; then R9, R11, R18; then R8, R10, R12; then R13, R20; then R14; then R21.

---

## 9. What CI can and cannot check

**Never in CI:** any hook target, RVA or byte pattern against the real executable; any game flow (login, Ramsgate, a
hunt, the store, realms); EAC, CEF, Wwise and D3D behaviour; Wine/Proton; the two-DLL coexistence in the game process.
Those are the maintainer's play-tests, per slice. When only CI passed, say so.

**In CI:**

- Build: the DLL for `x86_64-pc-windows-msvc` on the Windows runner, provided it has no dependency on the host's SDK
  dump (RT-2). Its import table: no dynamic CRT (2.1). Exports: the broker and ownership functions exist.
- Unit tests of pure code, on every platform if it lives in a separate pure crate: the backend URL grammar (every
  accept and refuse case, the reason texts), the redirect host allowlist and URL split, the map URL builder, launch
  argument parsing and the `NO_` sentinels, the `MYSTICPARADOX_` alias rule, the player host's environment validation
  and its exact pipe message texts, the FPS / trials week / AFK / hub-cap parsers and their defaults, byte-pattern
  verification against a buffer, the broker's chain order and ownership answers (with fake targets), the flag-cache
  modes.
- A table test that every id in this page appears exactly once in the registry, with its slice.

---

## 10. Calls I made and open questions

Calls (mine, for the maintainer to confirm):

- I counted hooks as installed or defined-only from the install paths, not the definitions; a detour whose install
  is commented out or behind `if (false)` counts as not installed (section 3.12).
- The two ProcessEvent detours count as two ids (one target, different bodies per role).
- I grouped the dispatch-point features (section 4) by feature and gave them no ids of their own; each slice spec
  will list its UFunction names.
- Sizes in section 8 are from the file's layout (region boundaries), not a precise count.
- I put the spline fault recovery in EXIT-12 (where it lives) and assigned its rule to R11.

Questions:

- **RT-1 Language and place.** Rust `cdylib` at `crates/rustic-isles-runtime` (with the pure parts in a second crate
  tested everywhere), MinHook built from its vendored C through `cc`; or MIT C++ in a new root native folder. AGENTS.md
  says there is no native code outside `restricted/`, and the workspace denies `unsafe_code`, so either needs a
  decision; a Rust crate needs a crate-level exception for its FFI module. Recommended: Rust, because of RT-2 and CI.
- **RT-2 The SDK.** The C++ compiles ~400 references to the host's Dumper-7 dump, so it cannot be built in CI. An MIT
  runtime can instead carry a small hand-written table of the offsets and names it uses (facts from the 1.14.7
  executable, like the RVAs) and find UFunctions by name at run time. Is such a table acceptable under "never commit
  generated SDKs"? I read it as yes: it is not generated and holds no game code.
- **RT-3 Diagnostics.** About 6k lines are logging-only probes (kill XP, grant trace, Tempest, Frank drone, bleedout,
  perf and memory, socket, URL, hitch sampler, the identity and progression probes). Recommended: port none; port a
  probe later only for a live bug. 0013, 0014 and 0026 are already marked droppable.
- **RT-4 The seam patch.** Who writes the AGPL seam patch (section 7), and is the broker design approved? It is a
  `restricted/` runtime patch read against AGPL code, so a restricted-side agent, not an implementer.
- **RT-5 The MIT DLL's name and load order.** `rustic-isles-runtime.dll`, loaded before `Rustic-Isles.dll`? Lane B's
  loader spec has to carry it.
- **RT-6 Launcher and feeds.** The known-DLL list (RD9's refusal), the runtime feed manifests and the Guard manifest
  must accept the second DLL during the transition.
- **RT-7 XMPP.** The patches page says the XMPP `bUseSSL` switch follows the address's scheme, but on 1.14.7 the
  `GetConfigString` redirect (`+0x025E6890`) is never installed ("ABI unresolved"), so neither `ServerAddr`,
  `ServerPort` nor `bUseSSL` is rewritten. Where does the client's chat connect today, and does the MIT runtime need
  the redirect at all?
- **RT-8 D3 for the player host.** D3 renames the header; should the MIT runtime also rename `MYS1`, the
  `MysticInternalServer.` pipe prefix, `MYSTICPARADOX_GAMESERVER_READY` and the `MYSTICPARADOX_` environment alias? The
  launcher and the Director read these, so it is a joint cutover.
- **RT-9 The login background.** `assets/mp_login_background.png` is embedded as a resource and shown on the login
  screen. Its provenance is not recorded. The MIT runtime needs its own image, or none.
- **RT-10 Store capture.** The runtime compiles the CatalogExporter's `StoreRuntimeExporter.cpp` for
  `REWARD_CACHE_CAPTURE.flag`. Keep that in the MIT runtime (from lane E's MIT exporter) or leave it to the exporter
  DLL through RD9?
- **RT-11 Policy patches.** 0024 and 0029 (and the `*_AFK_TIMEOUT_SECONDS.txt` / `HUB_MAX_PLAYERS.txt` files) as
  `settings.toml` settings passed in the environment, instead of hard-coded behaviour and files next to the exe?
- **RT-12 0004 after the phantom.** Since HL-2 removed the phantom local player, does LD-3's phantom-failure drop still
  fire? Its one-shot loader retry is still needed either way.
- **RT-13** `RUSTIC_ISLES_INTERNAL_VERSION`: is it read anywhere (the launcher?), or can it go?
- **RT-14 The ~120 flag files.** Keep the mechanism for the slices that keep a switch, and drop flags whose code is
  not ported? A slice spec would then list only its live flags.
- **RT-15** Confirm the Dainty Lass Director exports `DAINTY_BACKEND_URL` to every game server (the two notes in the
  runtime disagree). With the generated header absent the fallback is `backend.invalid`, so a server without the
  variable fails closed on every rewritten request.
- **RT-16** Whether the server installs CL-11..13 without `SOCKET_DIAG.flag` (section 3.3).
- **RT-17** Whether the server's install order matters beyond CL-1/HL-4 first and DX-8/9 after HL-2.
- **RT-18** The exact name matching used on the dispatch points (full name equality, prefix or substring) is per
  handler; each slice spec must state it per name, since a substring match on a full name catches more than it seems.

---

## Appendix A: flag files by owning slice

Every name is a file next to the game executable. Meanings are in each slice's spec.

- **R0:** `UNCACHED_FLAG_FILES`, `PRODUCTION_LOG_PROFILE`, `MPLOG_OPEN_PER_LINE`, `VERBOSE_DIAG`,
  `NO_FRAME_POINTER_CACHE`, `PE_CATEGORY_CACHE_OFF`, `PE_FULLNAME_UNCACHED`
- **R2:** `URL_LOG`, `URL_PATH_DIAG`, `CHARACTER_ASSOC_PROBE`
- **R3:** `DISABLE_CRASH_WAIT_SHORTEN`
- **R4:** `DISABLE_EAC_LOAD`, `SOCKET_DIAG`, `NO_PERF_DETAIL`
- **R5:** `SHOW_SERVER_WINDOW`, `KEEP_WEB_BROWSER`, `DISABLE_SERVER_AUDIO`, `ENABLE_SERVER_AUDIO`, `MP_FORCE_WARP`,
  `DISABLE_SERVER_VIEWPORT_SIZE`, `VIEWPORT_FALLBACK`, `DISABLE_SERVER_LOCALE_FIX`
- **R6:** `DISABLE_CALLOUT_GUARDS`, `DISABLE_INVENTORY_REFRESH_GUARD`, `DISABLE_HUD_LEGENDARY_GUARD`
- **R7:** `KEEP_PHANTOM_LOAD_FAILURE`, `DISABLE_PLAYERDATA_LOAD_RETRY`, `FORCE_PROGRESSION_CONFIG_READY`,
  `FORCE_PROGRESSION_READY`, `PROGRESSION_WAIT_RESUME`, `DISABLE_RESTART_DATA_GATE`, `KEEP_ARRIVAL_TRIGGER_WAIT`,
  `FORCE_ACTIVE_SEASON`, `CALL_FIND_ACTIVE_HUNTPASS`, `CALL_IFACE_QUERY`
- **R8:** `DISABLE_AFK_TIMEOUT_OVERRIDE`, `DISABLE_EXPECTED_PLAYER_ZERO`, `LEGACY_EXPECTED_PLAYER_WALK`,
  `KEEP_OOM_POOL`, `KEEP_WORKING_SET`, `SERVER_QUITGAME_EXIT`, `KEEP_HOSTED_HUB_ROLE`, `DISABLE_PERF_LOG`
- **R9:** `MAP_URL_LISTEN`, `DISABLE_MANUAL_LISTEN`, `NATIVE_REPLICATION_ONLY`, `REVERSE_CONNECTION_ORDER`,
  `HYBRID_REPLICATION`, `LEVEL_VISIBILITY_GATE`
- **R11:** `DISABLE_SPLINE_GUARD`, `ENABLE_SPLINE_EVAL_HOOK`, `MANUAL_NET_TICK`, `MANUAL_TICK_HALF_RATE`,
  `MANUAL_TICK_ZERO_DT`, `NATIVE_NET_TICK`, `EMERGENCY_LEGACY_REPLICATION`
- **R12:** `DISABLE_PC_CHANNEL_GUARD`, `DISABLE_RPC_UNBOUND_CHANNEL_GUARD`, `DISABLE_GRAPH_DRIVE`, `REPGRAPH_DIAG`,
  `SRA_RATE_CAP` (dead), `DIAG_NATURAL`
- **R13:** `DISABLE_WEAPON_CULL_FIX`, `DISABLE_WEAPON_DORMANCY_FIX`, `DISABLE_SERVER_WEAPON_GAP_LOG`,
  `DISABLE_REPGRAPH_RELEVANCE_FIX`, `DISABLE_OWNER_PAWN_RELEVANCY`, `DISABLE_PLAYER_REP_BOOST`, `PLAYER_ALWAYS_RELEVANT`,
  `RELEVANCE_PASS_EVERY_TICK`, `ENABLE_BASE_ACTOR_BOOTSTRAP`, `ENABLE_PLAYERSTATE_OWNER_ROUTE`,
  `ENABLE_HUB_REP_DIAGNOSTICS`
- **R14:** `DISABLE_PLAYER_ROLE_GLOBAL_ROUTE`, `DISABLE_PLAYER_ROLE_OWNER_ROUTE`, `DISABLE_PLAYER_ROLE_REMOTE_ROUTE`,
  `DISABLE_CLIENT_ROLE_LIFECYCLE_REPLAY`, `USE_LEGACY_ROLE_ONREP_REPAIR`, `ROLE_PASS_SNAPSHOT_OFF`,
  `NO_PLAYER_ROLE_CHANNEL_LOG`, `NO_PAWN_ROLE_CENSUS`, `NO_CONTROLLER_SPLIT_LOG`, `ENABLE_CONTROLLER_WATCHPOINT`,
  `IMMEDIATE_AIRSHIP_HUD_RESTORE`, `FORCE_CINEMATIC_CLEAR`
- **R15:** `KEEP_SUPPLY_STACK_CAPS`, `KEEP_CELL_FUSION_TIME`, `DISABLE_LOADOUT_SLOT_REPAIR`,
  `DISABLE_CLIENT_INVENTORY_TRACE`
- **R16:** `NO_LOADMAP_PROBE`, `ENABLE_ZOMBIE_LOADMAP_GUARD`, `NO_WORLD_CENSUS`, `KEEP_STUCK_ASYNC_FLAGS`,
  `ZOMBIE_REF_SCAN`, `TRAVEL_CAUSE_OFF`
- **R17:** `DISABLE_LOGIN_BRANDING`, `DISABLE_CREDITS_BRANDING`, `DISABLE_PRESS_START_BRANDING`, `AUTO_PRESS_START`
- **R18:** `ESCALATION_NO_BUFF_CHOICE_SPAWN`, `GAUNTLET_CENSUS_OFF`
- **R20:** `FORCE_SERVER_MESH_POSE`, `FORCE_SERVER_MESH_POSE_UNSAFE`, `DISABLE_PLAYER_HUNTID_BACKFILL`,
  `DEBUG_DAMAGE_MULTIPLIER`
- **R19 (diagnostics):** `GRANT_TRACE_OFF`, `KILL_XP_TRACE_OFF`, `ECON_TRACE_OFF`, `SERVER_TICK_FILTER`,
  `TICK_FILTER_CENSUS`, `TEMPEST_SCALE_DIAG`, `TEMPEST_DAMAGE_DIAG`, `TEMPEST_CHARGE_DIAG`, `TRIALS_GRACE_DIAG`,
  `HITCH_SAMPLER`, `PE_NAME_DIAG`, `CORE_CAPTURE`, `XMPP_TRACE`, `ACCESSORY_UNLOCK_DIAG`
- **Lane E:** `REWARD_CACHE_CAPTURE`

Some flags' owners are my reading of their names and where they are tested; a slice spec moves a flag if its code
lives elsewhere.
