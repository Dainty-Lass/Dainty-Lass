<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R5, headless server boot (RD27, plan item 11)

> Since R21 (`spec-runtime-r21.md`) the AGPL runtime is gone; `rustic-isles-runtime.dll` is the only runtime. This
> slice's play-test runs on the MIT DLL alone, as that spec's section 9 adapts it: skip every step that builds or
> patches the AGPL runtime, injects both DLLs, counts `[MitSeam]` lines or runs without the MIT DLL.

R5 makes a game server headless: no window, no viewport or local player, no Chromium/CEF, no sound posted, the server
flags the net driver needs, the C number locale, and the two viewport-size fixes a headless server needs to aim and to
frame vendors (patch 0038, the War Pike special). It owns HL-1 to HL-12 and the feature id `SP-2@R5`
(`spec-runtime-surface.md` sections 3.5, 6 and 8; `spec-runtime-r0.md` section 7.5). It installs nothing on a client.

An implementer builds from this page, `spec-runtime-surface.md`, `spec-runtime-r0.md` and `spec-runtime-r1.md` alone,
and never reads `restricted/`. What crossed is facts: RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7, CL 647472),
byte patterns, engine and Windows names, UE layout offsets, flag names, and behaviour. The AGPL side (seam patch 0041,
and runtime patch 0045 written with this page) is described only by the contract it keeps (section 9).

Conventions are R0's: an RVA is `+0x0XXXXXXX` from the executable's image base; **S** server. **(call)** marks a
decision this page makes, **(departure)** a deliberate difference from the AGPL runtime, **(ambiguity)** something the
source did not settle; all are collected in section 12.

---

## 1. Scope

| Id | What | Kind (R0 registry) | Section | Installed? |
| --- | --- | --- | --- | --- |
| HL-1 | `GIsServer` = 1, `GIsClient` = 0: at Init and around every engine tick | Patch | 4 | yes |
| HL-2 | `UGameEngine::Init` skips the viewport and first-local-player block | Patch | 5 | yes |
| HL-3 | `user32!CreateWindowExW` and `user32!ShowWindow` hide this process's windows; the hider thread | Detour (two targets) | 6 | yes, unless `SHOW_SERVER_WINDOW.flag` |
| HL-4 | `WebBrowserWidget` module startup replaced: no Chromium/CEF | Detour | 7 | yes, unless `KEEP_WEB_BROWSER.flag` |
| HL-5 | Wwise output-device init skipped | Detour | 8.2 | **no (call)**: owned, never installed |
| HL-6 | `kernel32!LoadLibraryExW`: hook d3d11 when it loads | Detour | 10 | only with `MP_FORCE_WARP.flag` |
| HL-7, HL-8 | `d3d11!D3D11CreateDevice`, `D3D11CreateDeviceAndSwapChain`: force WARP | Detour | 10 | only with `MP_FORCE_WARP.flag` |
| HL-9 | the RHI `IsSupported` branch made unconditional | Patch | 10 | only with `MP_FORCE_WARP.flag` |
| HL-10 | the `GetViewportSize` exec thunk detour | Detour | 11.3 | **no (call)**: owned, never installed |
| HL-11 | `UWidgetLayoutLibrary::GetViewportSize`: a (0,0) answer becomes 1920×1080 (patch 0038) | Swap | 11 | yes, unless `DISABLE_SERVER_VIEWPORT_SIZE.flag` |
| HL-12 | the server's process number locale (patch 0031) | Setting | 3 | yes, unless `DISABLE_SERVER_LOCALE_FIX.flag` |
| `SP-2@R5` | the server's `ProcessEvent` absorbs sound posting | Feature | 8 | yes, unless `ENABLE_SERVER_AUDIO.flag` |

Not R5: the `-CULTURE=en-US` and `-nullrhi` appends to the engine command line (CL-1, R4; section 3.3 says what R4 reads
from here), MainThread's role banner (R9), the empty-server watchdog and pacing (R8), the post-login match start that
relies on HL-2 (R20), and the nullrhi cache log line (a logging-only probe, RT-3; section 5.3 keeps its one rule).

R5 depends on R0 and R1 only (R1's role and auth mode). It is independent of the other slices specified beside it.

---

## 2. Where it lives

| File | Kind | Holds |
| --- | --- | --- |
| `crates/rustic-isles-runtime-core/src/headless.rs` (new) | pure, every platform | the byte patterns and data RVAs of HL-1, HL-2, HL-9; the viewport rule (11.2); the d3d11 path test (10.2); the PostEvent matcher (8.1); the install reason texts and log lines (section 13); the UE layout offset of 11.1 |
| `crates/rustic-isles-runtime/src/headless.rs` (new, safe) | Windows | `install()`, called from Init; the SP-2 and SP-3 handler registration; the once-a-second HL-11 retry; the state other slices read (2.2) |
| `crates/rustic-isles-runtime/src/ffi/headless.rs` (new, under `ffi`) | Windows, unsafe | the detour bodies (HL-3, HL-4, HL-6, HL-7, HL-8), the hider thread, the locale calls into `ucrtbase.dll`, the HL-11 exec function and its pointer write, the HL-1 byte writes |

Lines R5 adds to R0's files (one each, append-only so the slices landing beside it merge cleanly):

- `registry.rs`: `"R5"` in `LANDED`. Every R5 id is already in `ENTRIES` (HL-1..12, `SP-2@R5`).
- `offsets.rs`: the `TARGETS` lines of 2.1, and the table test's uniqueness exception for HL-3 (2.1).
- `lifecycle.rs`: on a server, `headless::install(...)` right after `minhook::initialize()` and **before**
  `handlers::freeze()` (it registers handlers), passing whether MinHook initialised.
- `ffi/mod.rs`: `pub mod headless;`. `lib.rs`: `pub mod headless;` under `cfg(minhook)`.
- `ffi/engine.rs`: one function, `find_object_by_full_name(name: &str) -> Option<usize>` (11.1). **(call)** If R17's
  reflection kit (`ffi/reflect.rs`, `find_function(class, name)`) is on `plan` when R5 is implemented, use it instead
  and add nothing to `engine.rs`.

### 2.1 Offset table lines (RT-2)

`TARGETS`, one line each (the test of R0 6.1 requires a line for every `Detour` id of a landed slice, installed or not):

| id | rva | found_by | what |
| --- | --- | --- | --- |
| HL-3 | 0 | `Export("user32.dll", "CreateWindowExW")` | window creation |
| HL-3 | 0 | `Export("user32.dll", "ShowWindow")` | window show |
| HL-4 | `0x00F3_57D0` | `RvaAndBytes(&[0x48, 0x8B, 0xC4, 0x56, 0x48, 0x83, 0xEC, 0x70, 0x48, 0x83, 0x79, 0x08, 0x00, 0x48, 0x8B, 0xF1, 0x0F, 0x85])` | `FWebBrowserWidgetModule::StartupModule` |
| HL-5 | `0x017B_1620` | `Rva` | Wwise output-device init (never installed) |
| HL-6 | 0 | `Export("kernel32.dll", "LoadLibraryExW")` | library load |
| HL-7 | 0 | `Export("d3d11.dll", "D3D11CreateDevice")` | D3D11 device |
| HL-8 | 0 | `Export("d3d11.dll", "D3D11CreateDeviceAndSwapChain")` | D3D11 device and swap chain |
| HL-10 | `0x0398_BB50` | `Rva` | `UWidgetLayoutLibrary::execGetViewportSize` (never installed) |

The table test's "each id once" rule gains one exception: HL-3 has exactly two lines (its two exports), as SP-1 and
SP-2 share an RVA. HL-11 is a `Swap` and has no `TARGETS` line; its facts are in `headless.rs` (11.1).

Data and patch facts, in the core `headless.rs` (not `TARGETS`, whose test admits only detours):

| Name | Value |
| --- | --- |
| `GIsServer` | data `+0x06E1EADA`, one byte |
| `GIsClient` | data `+0x06E1EAD9`, one byte |
| HL-2 site | `+0x03E07353`, expected `0F 84 D3 00 00 00`, replacement `E9 D4 00 00 00 90` |
| HL-9 site | `+0x034AB73D`, expected `84 C0 75`, replacement `84 C0 EB` |
| nullrhi cache (never written) | data `+0x06CD7878` (byte), its guard `+0x06CD787C` (`i32`) |
| `UFunction` native pointer | `+0xD8` (11.1) |

### 2.2 What R5 exposes to other slices

- `headless::boot_patch_applied() -> bool`: HL-2's result was `Applied` or `AlreadyApplied` (false on a client, with an
  invalid auth mode, or a failed patch). R20's post-login match start and any later "no phantom" logic read it.
- Nothing for the culture switch: R4's CL-1 reads `DISABLE_SERVER_LOCALE_FIX.flag` itself (3.3).

---

## 3. HL-12: the server's number locale (patch 0031)

### 3.1 Why

A server whose Windows user locale writes decimals with a comma (de-DE, pt-BR) misread decimal-point values the game
parses with the C runtime, and Behemoth part breaks that were rewarded stayed attached on players' screens. The fix is
the C runtime's `"C"` locale in the game's own CRT, plus the engine culture `en-US` on the command line (CL-1, R4).

### 3.2 Behaviour

Server only, in Init (under the loader lock; logged later, 13.1):

1. `DISABLE_SERVER_LOCALE_FIX.flag` present (read once): do nothing.
2. Read the Windows user locale name (`GetUserDefaultLocaleName`, `?` on failure) for the log.
3. `GetModuleHandleW(L"ucrtbase.dll")`. Resolve its exports `setlocale`, `_configthreadlocale` and `localeconv`.
   Read the decimal point before: `localeconv()` returns a `struct lconv*` whose first field is `char* decimal_point`;
   `?` when anything is missing.
4. If `setlocale` resolved: `_configthreadlocale(1)` (`_ENABLE_PER_THREAD_LOCALE`) when resolved, `setlocale(0, "C")`
   (`LC_ALL` = 0), `_configthreadlocale(2)` (`_DISABLE_PER_THREAD_LOCALE`) when resolved, then `setlocale(0, "C")`
   again and keep its return (the new locale name) for the log. The first pair sets the calling thread's own locale,
   the second the process-wide one.
5. Read the decimal point after.

**(departure)** The AGPL also sets the locale of its own statically linked CRT. The MIT DLL formats and parses no
number through the C runtime (Rust's are locale-independent), so it sets only the game's.

**(call)** `ucrtbase.dll` not loaded at Init: the DLL never calls `LoadLibrary` under the loader lock (R0 3.1), so it logs
`game_crt=not-found` and retries steps 3 to 5 once at the first SP-3 tick, logging the result with ` (late)`. The game
imports `ucrtbase.dll` statically, so on an injected server (launcher, Director, `dainty-inject.exe`) it is loaded
before the MIT `DllMain` runs; the retry only covers an unexpected load order.

### 3.3 The culture switch belongs to CL-1 (R4)

The engine culture is not R5's: CL-1 (R4) appends `-CULTURE=en-US` to the engine command line it serves, on a server,
when the locale fix is on and the line holds no `-culture=` already (case-insensitive). With R4 not landed the AGPL's
CL-1 does it; with R4 landed the MIT CL-1 does (`spec-runtime-r4.md` 3.3). Either reads the same flag file itself: the
two halves of patch 0031 share the flag and no state.

---

## 4. HL-1: the server flags

The executable is the Windows client build. Its boot code sets client flags and the engine tick resets them, so the
net driver refused connections ("Failure received") unless the flags are forced. Three writes, all server only, all a
plain byte store of `1` to `GIsServer` and `0` to `GIsClient` (both in the executable's writable data):

| When | Where in the MIT runtime |
| --- | --- |
| Init | `headless::install`, before any other R5 step |
| every frame, before the engine tick | an SP-3 **before** handler |
| every frame, after the engine tick | an SP-3 **after** handler |

Both tick handlers register with `order = -1000` **(call)**, so they run before every other MIT handler on SP-3 (R11's
net driver sanitising and R9's listen read the flags). They never fail and never log.

**Position against the AGPL (cross-slice, section 9).** On SP-3 every MIT after handler runs after the AGPL tick
wrapper's own post-tick blocks (R0 8.1, 8.2). The AGPL re-forces the flags straight after the engine tick returns,
before its own post-tick blocks (listen, the manual net tick, the graph drive). Runtime patch 0045 keeps that one write
in the AGPL even when the MIT owns HL-1 (9.2), so those blocks keep seeing server flags. The duplicate write is
idempotent. Once every SP-3 slice is MIT (R21), R5's after handler is the first thing after the engine tick, as today.

The pre-tick write in the AGPL sat after the player host's fence check (an early return of the tick, `SP-3@R10`). The
MIT handler runs before it; a fenced frame then also gets the write, which changes nothing.

---

## 5. HL-2: no viewport, no local player

### 5.1 Behaviour

Server only, and only when R1's auth mode is not `Invalid` (an invalid mode ends the process before listen anyway).
In Init, R0's `patch_bytes(0x03E07353, [0F 84 D3 00 00 00], [E9 D4 00 00 00 90])`.

What it does (engine facts): `UGameEngine::Init` (`+0x03E07010`) tests `GIsClient` at `+0x03E07349` and, when it is 0,
jumps at `+0x03E07353` over the block that creates the game viewport client. The client build boots with `GIsClient` = 1,
so the block runs and makes a window, a viewport and a first local player (the "phantom"). The replacement is an
unconditional `jmp` to the same target (`jz rel32 0xD3` from a 6-byte instruction and `jmp rel32 0xD4` from a 5-byte one
both land on `+0x03E0742C`) plus a `nop`. A following check (`+0x03E07457`) then skips the window and first-player
setup, the engine's own dedicated-server path. `GIsClient` stays 1 for the rest of the boot: forcing it to 0 instead
stalled Slate and the map's listen gate. Result: no viewport, no window, no local player; the first `LoadMap` spawns
no `PlayerController`, and the first real login starts the match (R20).

### 5.2 Results

| `PatchResult` | `boot_patch_applied()` | Log (13.1) |
| --- | --- | --- |
| `Applied`, `AlreadyApplied` | true | `install HL-2 ok` / `install HL-2 already applied` |
| `Mismatch`, `Unreadable`, `ProtectFailed` | false | the reason, then the loud line of 13.2: a server without the patch **has** a phantom local player and nothing suppresses it any more |

It must be applied **in Init**, never later: the AGPL reads the bytes once in its own Init, which runs after the MIT
`DllMain` (9.1), and its post-login match start and the never-ported DX-8/9 depend on that reading.

### 5.3 The nullrhi cache: never written

The byte at `+0x06CD7878` (guard `+0x06CD787C`) is the engine's cached answer to "was `-nullrhi` on the engine command
line"; nonzero selects the null RHI. R5 never writes it or its guard (writing 0 once gave every hub a hardware D3D
device). Whether a hub gets the null RHI is CL-1's `-nullrhi` append (R4). The AGPL's line logging the byte is a
diagnostic and is not ported (RT-3).

---

## 6. HL-3: no window

### 6.1 Install

Server only. `SHOW_SERVER_WINDOW.flag` present at Init: install nothing, start no thread, log `off`. Otherwise, in
Init: the two export targets through `GetModuleHandleW(L"user32.dll")` and `GetProcAddress` (never `LoadLibrary`;
`module not loaded` when absent **(ambiguity)**: the game imports `user32.dll` statically, so it is expected loaded), each
`broker::hook`ed with `order = 0`. Both must succeed for `install HL-3 ok`; one failing logs its reason and the other
stays.

Use exactly the address `GetProcAddress` returns (with forwarders resolved): the AGPL seam identifies the target the
same way to leave HL-3 to the MIT.

### 6.2 The detours

- **`CreateWindowExW`** (12 arguments, returns `HWND`): call `next` with the arguments unchanged. When the result is
  non-null, call `ShowWindow(result, SW_HIDE)` (the export, so it passes the chain; `SW_HIDE` = 0) and count one hide.
  Every window this process creates is hidden, children included.
- **`ShowWindow(hwnd, cmd)`**: when `cmd != SW_HIDE`, `hwnd` is non-null, `GetWindowThreadProcessId(hwnd)` is this
  process and `hwnd != GetConsoleWindow()`: count one hide and call `next(hwnd, SW_HIDE)`. Otherwise `next(hwnd, cmd)`.
- After detach both forward unchanged (R0 3.3).

### 6.3 The hider thread

A safety net for a window shown by a path the two detours miss (the engine can re-show its window on a map change).
**(call)** R0's Init starts no thread, so the thread starts on the first call into either HL-3 detour or the first SP-3
tick, whichever comes first (one atomic once-flag). It runs:

- 240 rounds of `EnumWindows` then `Sleep(250)` (about 60 s while the engine boots), then `EnumWindows` every 3000 ms
  for the life of the process.
- For each top-level window: skip it unless its thread's process is this one, it is not the console window, and it is
  visible (`IsWindowVisible`); then `ShowWindow(hwnd, SW_HIDE)` and count one hide.
- The first time the hide count is above 0 during the 240 fast rounds, log `hid the server window` once.
- **(departure)** It ends when the detaching flag is set (checked each round) and never logs after it. The AGPL thread
  never ends.

Thread start failure logs `hider thread failed (error <GetLastError>)`; the detours stay.

---

## 7. HL-4: no Chromium/CEF

`FWebBrowserWidgetModule::StartupModule` at `+0x00F357D0` creates a web-browser asset manager, loads the WebBrowser
module and, on its first call, `FWebBrowserSingleton`, which initialises CEF and starts an `UnrealCEFSubProcess.exe`
GPU-process child (about 111 MB private memory per hub). A server shows no web page, and the engine module ignores
`-nocef`. The module's `ShutdownModule` and destructor tolerate a skipped startup, so skipping leaves it loaded and inert.

- Server only, in Init (before the engine's PreInit starts plugin modules: `DllMain` time satisfies this).
- `KEEP_WEB_BROWSER.flag` present: not installed, log `off`.
- R0's `verify_entry(0x00F357D0, prologue)` with the 18 bytes of 2.1 (`function_entry`, not chained): on `false`, not
  installed, log `not a function entry or bytes differ`, and CEF starts as before (fail closed).
- Otherwise `broker::hook` with `order = 0`. The detour takes the module (`this`), logs `skipped WebBrowserWidget
  startup (no Chromium/CEF)` once, and returns **without calling `next`** (it absorbs: the AGPL wrapper, if any, and the
  original never run). Signature: one pointer argument, no return value.

A later client-UI call of the singleton would still create CEF lazily; no server path does.

---

## 8. Sound: `SP-2@R5`, and HL-5 not installed

### 8.1 `SP-2@R5`: absorb sound posting

A server must play no sound. Removing the output device crashes the boot (8.2), so the safe layer is to absorb the
game's own sound posting while every engine pointer stays valid.

- A `ProcessEvent` **before** handler on SP-2 (`handlers::register_pe(Point::Sp2, ...)`), `order = 50` **(call)** (a
  slice whose handler must run on every call, as R15's per-call data writes did in the AGPL, takes a smaller order).
- **Matcher (RT-18):** `[(Kind::Contains, "PostEvent")]` on the full name, exactly the AGPL's test. It catches
  `Function AkAudio.AkGameplayStatics.PostEvent` and `Function AkAudio.AkGameplayStatics.PostEventAtLocation` (the
  entry points the game posts sounds through) and any other UFunction whose full name holds `PostEvent`; it does not
  catch `PostAkEvent`. **(ambiguity)** No list of every 1.14.7 UFunction holding `PostEvent` was available; the play-test
  (14.2 step 3) checks nothing else broke.
- **The switch:** read `ENABLE_SERVER_AUDIO.flag` once, at the handler's first call. Present: the handler always
  returns `Continue` and logs `sound posting left on (ENABLE_SERVER_AUDIO.flag)` once. Absent: logs `sound posting
  suppressed (server)` once, then for every matching call returns **`Absorb`** (no AGPL block, no original, no after
  handler), logging the first 5 as `absorbed sound post #<n> fn=<full name> obj=<object full name or null>`.

**Position against the AGPL.** In the AGPL the absorb came after its depth guard, its per-call R15 data writes and
some diagnostics. The MIT handler runs before the AGPL wrapper, so for an absorbed call those AGPL blocks do not run.
The data writes are one-time writes retried on every call, so skipping them on sound calls changes nothing; the
diagnostics are not ported (RT-3).

### 8.2 HL-5: owned, never installed (call)

The AGPL's opt-in `DISABLE_SERVER_AUDIO.flag` replaced the Wwise output-device init (`+0x017B1620`) with a return. It
was measured to silence the hub and then crash the boot (an access violation writing near address `0xA4`), and it was
left off. R5 owns HL-5 so the AGPL never installs it, and does not install it either: the flag is dropped (RT-14). A
server with the flag behaves as one without it, which is what the AGPL did by default.

---

## 9. The AGPL side

### 9.1 What seam patch 0041 already does when the MIT owns R5's ids

- HL-1 (its three writes), HL-2, HL-3, HL-9, HL-11 and HL-12 are gated by id directly (R0 7.4). With HL-2 owned, the
  AGPL reads whether the patch took from the six bytes at `+0x03E07353` (for its post-login match start and its DX-8/9),
  once, in its Init, after the MIT `DllMain` has run.
- HL-3 owned: the AGPL starts no hider thread and installs neither window detour; it does not read `SHOW_SERVER_WINDOW`.
- HL-4, HL-5, HL-6, HL-7, HL-8, HL-10: the AGPL still calls its hook creation, which the seam maps to the id and
  leaves to the MIT (no wrapper; a direct call of "the original" reaches the game function). Its log lines saying
  "installed" then mean "left to the MIT runtime".
- `SP-2@R5`: the AGPL's sound absorb is gated.
- Not gated, and left as is: the AGPL's role line (`[InitRole] ...`) and its nullrhi cache line. They only log.

### 9.2 Runtime patch 0045 (written with this page)

One change, in `restricted/` (`restricted/docs/paradox-patches.md`, row 0045): **the AGPL keeps re-forcing
`GIsServer` = 1 and `GIsClient` = 0 straight after the engine tick returns even when the MIT owns HL-1**, so its own
post-tick blocks still see server flags (section 4). Its Init write and its pre-tick write stay gated by HL-1. With
the MIT DLL absent nothing changes. The patch adds nothing to the seam ABI (still version 1).

### 9.3 Cross-slice state (R0 7.5's table, new rows)

| State | Set by | Read by | Rule |
| --- | --- | --- | --- |
| server flags after the engine tick | HL-1 (MIT after handler) | AGPL post-tick blocks (R8, R9, R11, R12, R13 until each lands) | carried: patch 0045 keeps the AGPL's own write |
| "the boot patch took" | HL-2 (MIT, in Init) | AGPL post-login match start and DX-8/9; MIT R20 | carried: the seam reads the bytes; MIT slices call `boot_patch_applied()` |
| the locale fix switch | the flag file | HL-12 (R5), CL-1's culture append (R4) | no shared state: each reads the flag (as `spec-runtime-r4.md` records) |

---

## 10. HL-6 to HL-9: WARP on a server without a GPU

### 10.1 Why

On a machine with no Feature Level 11 GPU the engine's RHI selector asks the D3D11 RHI module `IsSupported`, gets false
and exits with "A D3D11-compatible GPU (Feature Level 11.0, Shader Model 5.0) is required to run the engine";
`-nullrhi` and `-warp` do not bypass it on this build. Opt-in, for a GPU-less VPS.

### 10.2 Behaviour

Server only. `MP_FORCE_WARP.flag` read once at Init. Absent: none of the four is installed (`off`). Present, in Init:

1. **HL-9:** `patch_bytes(0x034AB73D, [84 C0 75], [84 C0 EB])`: after `call [rdx+0x40]` (`IsSupported`), the
   `test al,al; jne` to the failure path becomes `test al,al; jmp`, so the selector always accepts D3D11. A mismatch is
   logged and the rest still installs.
2. **HL-6:** `broker::hook` on `kernel32!LoadLibraryExW` (`GetProcAddress` result, forwarders resolved). Detour:
   call `next`; when the result is non-null, HL-7/8 are not hooked yet, and the path (UTF-16, may be null) contains
   `d3d11` compared case-insensitively over ASCII letters, hook HL-7/8 on the returned module (step 4).
3. If `GetModuleHandleW(L"d3d11.dll")` already answers, hook HL-7/8 on it now.
4. **HL-7/HL-8**, once (an atomic once-flag; the second attempt does nothing): `GetProcAddress` both exports on that
   module and `broker::hook` each found one. The HL-6 detour can run inside another module's `DllMain`, so this path
   **logs nothing**: it records each result in an atomic, and R5's SP-3 before handler logs `install HL-7 ...` and
   `install HL-8 ...` once at the first tick after they were recorded.
5. **HL-7 detour** `D3D11CreateDevice(adapter, driver_type, software, flags, levels, n_levels, sdk, device, level,
   context)` and **HL-8 detour** `D3D11CreateDeviceAndSwapChain(adapter, driver_type, software, flags, levels,
   n_levels, sdk, swap_desc, swap_chain, device, level, context)`: call `next` with `adapter` = null and `driver_type` =
   5 (`D3D_DRIVER_TYPE_WARP`), every other argument unchanged; return its `HRESULT` (`i32`). WARP is always present in
   `d3d11.dll` and needs a null adapter.

The path test is pure (core) and also matches names such as `d3d11on12.dll`, as the AGPL did; hooking stops after the
first match.

---

## 11. HL-11 (patch 0038) and HL-10: the viewport size

### 11.1 HL-11: the swap

A headless server has no viewport, so `UWidgetLayoutLibrary::GetViewportSize` answers (0,0). The War Pike's projectile
special (`mspear_special_projectile_bpc` → `player_aiming_bpc.StartAiming` →
`camera_helper_function_library.ConvertFovHorzToVert`) divides by it on the server ("Divide by zero detected"), so
the server-side shot aims wrong and passes through Behemoths. Vendor cameras (`vendor_interactive_bp_C`,
`UpdateCameraForAspect`) divide by it too. Only the aspect ratio matters on the server and the aim is the screen's
centre at any size, so 1920×1080 is used.

The UFunction's native function pointer is replaced, so every Blueprint call of it goes through the MIT function:

- **When:** an SP-3 before handler (`order = -1000`, after HL-1's in registration order) tries once a second
  (`GetTickCount64`) until done. `DISABLE_SERVER_VIEWPORT_SIZE.flag` present at the first try: done, nothing swapped,
  log `off`.
- **Find:** the object whose full name is exactly `Function UMG.WidgetLayoutLibrary.GetViewportSize`, live
  (`is_live_object`). `find_object_by_full_name` walks R0's object array, compares the object's own short name to the
  part after the last `.` first, and builds the full name only for those. Not found: try again next second.
- **Read** the native pointer, `u64` at UFunction `+0xD8` (`UFunction::Func`, UE 4.26's layout: `FunctionFlags`
  `+0xB0` ... `EventGraphCallOffset` `+0xD0`, `Func` `+0xD8`) through R0's `read`.
- **Self-check (call):** the pointer must lie in the game executable and be a `.pdata` function entry
  (`function_entry(rva) == Some(rva)`). Otherwise write nothing, stop retrying and log the loud line of 13.2: a wrong
  offset fails closed instead of writing into a game object.
- **Swap:** when the pointer is already the MIT function, done. Otherwise store the pointer in an atomic `original`,
  then write the MIT function's address at `+0xD8` (one aligned 8-byte store). Log `GetViewportSize answers 1920x1080
  instead of 0 on this server (native +0x<rva>)`. On 1.14.7 the RVA is expected to be `+0x0398BB50` (HL-10's target).
- **The MIT function** `(context, stack, result)` (UE's native thunk: `UObject*`, `FFrame&`, return-value pointer):
  call `original` with the same arguments. Then, if `result` is non-null and readable for 8 bytes **without** the
  8-byte alignment rule (`readable_unaligned`: the return slot holds an `FVector2D` of two `f32`), apply 11.2. Log the
  first 3 substitutions: `GetViewportSize answered <x>x<y> on the server; returned 1920x1080`.

The pointer is never restored (the DLL is pinned; detach leaves it). The R0 detaching rule applies: after detach the
function only calls `original`.

### 11.2 The rule (pure)

`fix_viewport(x: f32, y: f32) -> Option<(f32, f32)>`: `Some((1920.0, 1080.0))` when `!(x > 1.0) || !(y > 1.0)` (so 0,
negative, 1 and NaN on either axis), else `None`. A real size is never touched.

### 11.3 HL-10: owned, never installed (call)

The AGPL also detours the exec thunk at `+0x0398BB50` and, only with `VIEWPORT_FALLBACK.flag`, applies a looser rule
(`> 0`). HL-11 covers every value that rule would change, through the same function, and by default. R5 owns HL-10 so
the AGPL does not install it, and does not install it either; `VIEWPORT_FALLBACK.flag` is dropped (RT-14).
**(departure)** With both `DISABLE_SERVER_VIEWPORT_SIZE.flag` and `VIEWPORT_FALLBACK.flag` present the AGPL still
applied the looser rule; the MIT runtime applies none.

---

## 12. Calls, departures and ambiguities

### 12.1 RT-12 to RT-18 for R5

- **RT-14 (flags), settled for R5:**

  | Flag | Kept? | Meaning |
  | --- | --- | --- |
  | `SHOW_SERVER_WINDOW` | kept | HL-3 off: windows stay visible, no hider thread (read once at Init) |
  | `KEEP_WEB_BROWSER` | kept | HL-4 off: CEF starts (read once at Init) |
  | `ENABLE_SERVER_AUDIO` | kept | `SP-2@R5` off: sound is posted (read once at the first SP-2 call) |
  | `MP_FORCE_WARP` | kept | HL-6 to HL-9 on (read once at Init) |
  | `DISABLE_SERVER_VIEWPORT_SIZE` | kept | HL-11 off (read once at the first try) |
  | `DISABLE_SERVER_LOCALE_FIX` | kept | HL-12 off, and R4's culture append off (read once at Init) |
  | `DISABLE_SERVER_AUDIO` | **dropped** | HL-5 is not installed (8.2) |
  | `VIEWPORT_FALLBACK` | **dropped** | HL-10 is not installed (11.3) |

- **RT-17 (install order), settled for R5:** every R5 step runs in the MIT Init (`DllMain`), which finishes before
  `Rustic-Isles.dll` loads and before the engine reads its command line, starts plugins, creates its window, picks its
  RHI or runs `UGameEngine::Init`. So the surface's order rules hold by construction: HL-4 before plugin startup, HL-2
  before `UGameEngine::Init`, HL-9 and HL-6 before RHI startup, HL-3 before the first window, HL-12 before the engine
  parses numbers. The one cross-DLL rule: HL-2 is applied in Init, never deferred, because the AGPL reads its bytes
  once in its own Init (5.2). Within R5 the order is HL-12, HL-1, HL-3, HL-4, HL-6..9, HL-2, then the handler
  registrations (any order works; this one keeps the log readable). With MinHook not initialised, the detours report
  its status and the patches, writes and locale still apply.
- **RT-18 (matching):** `SP-2@R5` is `Contains "PostEvent"` on the full name (8.1). HL-11's lookup is an exact full name.
- RT-12, RT-13, RT-15, RT-16: not R5's.

### 12.2 Calls

1. HL-5 and HL-10 are owned and never installed; `DISABLE_SERVER_AUDIO` and `VIEWPORT_FALLBACK` dropped.
2. HL-11 checks the native pointer is a `.pdata` function entry in the executable before swapping.
3. `ucrtbase.dll` absent at Init: one retry at the first SP-3 tick, never `LoadLibrary` under the loader lock.
4. The hider thread starts on the first HL-3 detour call or SP-3 tick, not in Init.
5. HL-1's tick handlers and HL-11's retry use `order = -1000`; the sound handler `order = 50`.
6. HL-7/8 results logged from the next SP-3 tick, never from the `LoadLibraryExW` path.
7. Runtime patch 0045 keeps the AGPL's post-tick flag write (9.2).
8. If R17's reflection kit is on `plan`, HL-11's lookup uses it (2).
9. The nullrhi cache log line is not ported (RT-3); its "never write" rule is (5.3).
10. R5's log lines use the tag `[Headless]` (section 13), distinct from every AGPL tag.

### 12.3 Departures

1. The game's CRT locale only; the DLL's own CRT is not set (3.2).
2. The hider thread ends on detach (6.3).
3. With `DISABLE_SERVER_VIEWPORT_SIZE.flag` and `VIEWPORT_FALLBACK.flag` both present, no viewport rule at all (11.3).
4. The `[WARP] exe=<path>` hint line is not ported: the install line says `off (no MP_FORCE_WARP.flag)` and names
   no path.

### 12.4 Ambiguities

- The full list of 1.14.7 UFunctions whose full name holds `PostEvent` (8.1).
- `user32.dll` and `ucrtbase.dll` are expected loaded at Init on an injected server; not verified on every host (3.2,
  6.1). The install lines show it.
- `UFunction::Func` at `+0xD8` is UE 4.26's layout, not read from this build's dump; the self-check of 11.1 fails
  closed if it is wrong, and the log names the RVA it found.

No question is left open for the maintainer.

---

## 13. Log lines

Every line goes through R0's log. Lines made in Init go through `log::defer` (written after R0's status lines on the
first detour call); later ones are written directly. Install lines use R0's `install_line` format.

### 13.1 Install lines (one per id, deferred from Init)

`[MitRuntime] install <id> <reason>`, `<reason>` one of: `ok`, `already applied` (HL-2, HL-9), `off (<FLAG>.flag)`,
`off (no MP_FORCE_WARP.flag)` (HL-6 to HL-9), `skipped (invalid auth mode)` (HL-2), `not installed (dropped)` (HL-5,
HL-10), `pending (tried each second from the engine tick)` (HL-11), `on d3d11 load` (HL-7, HL-8 when `d3d11.dll` is
not yet loaded), `bytes differ`, `unreadable`, `protect failed`, `not a function entry or bytes differ` (HL-4),
`module not loaded`, `export not found`, or a MinHook status name. HL-12's line is followed by:

`[Headless] locale windows_user_locale=<name> game_crt=ucrtbase|not-found decimal_before='<c>' decimal_after='<c>'
numeric=<setlocale answer or ?>` (one line; ` (late)` appended when it ran at the first tick).

### 13.2 Other lines

| When | Text |
| --- | --- |
| HL-2 failed | `[Headless] HL-2 NOT applied: this server has a phantom local player and nothing suppresses it; check the game build` |
| hider thread | `[Headless] hid the server window` (once); `[Headless] hider thread failed (error <n>)` |
| HL-4 detour | `[Headless] skipped WebBrowserWidget startup (no Chromium/CEF)` (once) |
| `SP-2@R5` first call | `[Headless] sound posting suppressed (server)` or `[Headless] sound posting left on (ENABLE_SERVER_AUDIO.flag)` |
| absorbed post | `[Headless] absorbed sound post #<n> fn=<full name> obj=<full name or null>` (first 5) |
| HL-11 swapped | `[Headless] GetViewportSize answers 1920x1080 instead of 0 on this server (native +0x<rva>)` |
| HL-11 refused | `[Headless] HL-11 NOT installed: GetViewportSize's native pointer +0x<rva> is not a function entry (wrong UFunction layout?)`; `<rva>` is `?` outside the executable |
| HL-11 substituted | `[Headless] GetViewportSize answered <x>x<y> on the server; returned 1920x1080` (first 3; `x`, `y` with Rust's `{}` for `f32`) |

Nothing is logged after detach or under the loader lock, and no line holds the command line or an environment value.

---

## 14. Tests

### 14.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`, every platform):

- `fix_viewport`: (0,0), (1,1), (1.0001, 1080) kept, (1920,0), (−5,600), (NaN,1080), (1080,NaN), (∞,∞) kept,
  (1920,1080) kept.
- The d3d11 path test: `d3d11.dll`, `C:\Windows\System32\D3D11.DLL`, `d3d11on12.dll` match; `d3d10.dll`, `d3d1`,
  empty and null do not; a non-ASCII path does not panic.
- The PostEvent matcher on `Function AkAudio.AkGameplayStatics.PostEvent`, `...PostEventAtLocation` (match),
  `Function AkAudio.AkComponent.PostAkEvent` and null (no match).
- HL-2's bytes: both instructions reach the same target (`site + 6 + 0xD3 == site + 5 + 0xD4`), the replacement ends in
  `0x90`; `patch::decide` gives `Applied`, `AlreadyApplied` and `Mismatch` for HL-2 and HL-9 over buffers.
- The install reason texts and every line of 13.2 for fixed inputs.
- The registry with `LANDED` holding R5: `owns` true for HL-1..12 and `SP-2@R5`, false for `SP-2@R7`.
- The offset table test with R5's lines: HL-3 twice is accepted, any other id twice is not.

DLL (`cargo test -p rustic-isles-runtime`, Windows runner):

- HL-3 in the test process: install the two detours through the broker; a window the test creates is not visible;
  `ShowWindow(w, SW_SHOW)` leaves it hidden; `ShowWindow(w, SW_HIDE)` passes; the console window (if any) is untouched.
- The locale routine: load `ucrtbase.dll` in the test (allowed in a test), set `setlocale(LC_ALL, "de-DE")`, check the
  decimal point is `,`; run step 3 to 5; it is `.` and the log line reads `decimal_before=',' decimal_after='.'`.
- The HL-11 MIT function with a fake `original` writing (0,0), (2560,1440) and NaN into an unaligned 8-byte slot.
- HL-7: a fake `D3D11CreateDevice` behind the broker records it got a null adapter and driver type 5.
- `boot_patch_applied()` after `patch_bytes_at` on a test page holding HL-2's expected bytes (true), and other bytes
  (false).

**Never in CI:** anything against the real executable: the RVAs, the byte patterns, the UFunction layout, the window,
CEF, Wwise, WARP and the locale in the game.

### 14.2 The maintainer's play-test (collected for the one hands-on pass, rulings "Order, amended")

Surface section 8: "Start a hub: no window, no sound, no CEF processes, GPU idle; a War Pike special hits a Behemoth
(0038); optionally WARP on a GPU-less box." Build the AGPL runtime with patches 0041 and 0045 and the MIT DLL with R5
landed, publish both (R0 11.1 step 1).

1. **Boot.** Let the Director start Ramsgate. In its `rustic_isles_dll_port<N>.log`: `[MitRuntime] install HL-1 ok`,
   `HL-2 ok`, `HL-3 ok`, `HL-4 ok`, `HL-5 not installed (dropped)`, `HL-6`..`HL-9 off (no MP_FORCE_WARP.flag)`,
   `HL-10 not installed (dropped)`, `HL-11 pending ...`, `HL-12 ok` with `[Headless] locale ... decimal_after='.'`;
   later `[Headless] GetViewportSize answers 1920x1080 ... (native +0x0398BB50)` and `[Headless] sound posting
   suppressed (server)`. The AGPL's `[MitSeam] ... present` line counts R5's hooks among those left to it.
2. **Headless.** No window appears at any point (watch the first 10 s); Task Manager shows no
   `UnrealCEFSubProcess.exe` under the server; the GPU's 3D engine stays near 0 for the server process with `-nullrhi`;
   the server plays no sound (the log shows `absorbed sound post` lines once players move around).
3. **Play.** Log in, walk Ramsgate, open a vendor (its camera frames normally), start a hunt with a War Pike and use
   its projectile special on the Behemoth: it hits. Break a part: it shows broken. Finish the hunt and return.
4. **Locale.** On a server whose Windows user locale is pt-BR or de-DE, repeat the part break: it shows broken; the log's
   locale line reads `decimal_before=','` and `decimal_after='.'`.
5. **Flags revert.** With `SHOW_SERVER_WINDOW.flag` the window shows; with `KEEP_WEB_BROWSER.flag` a CEF child starts;
   with `ENABLE_SERVER_AUDIO.flag` the absorb lines stop; with `DISABLE_SERVER_VIEWPORT_SIZE.flag` the swap line is
   `off`. Remove each afterwards.
6. **WARP (optional, a machine without a D3D11 GPU or with the GPU disabled).** Put `MP_FORCE_WARP.flag` next to the
   game and start a hub: it boots instead of showing the GPU error; the log has `install HL-9 ok`, `install HL-6 ok`
   and, at the first tick, `install HL-7 ok` and `install HL-8 ok`.
7. **Without the MIT DLL.** Remove `rustic-isles-runtime.dll` and repeat step 2: the AGPL does it all as before.
8. A hub that logs `[Headless] HL-2 NOT applied` or `HL-11 NOT installed` is a failed play-test: report the line.

Only CI passed until these steps are done.

---

## 15. Size

Rust without tests: the core `headless.rs` about 120 lines; the DLL's `headless.rs` about 150 and `ffi/headless.rs`
about 300 (window 90, hider 50, CEF 25, WARP 70, locale 40, viewport 25). Tests about 250. Runtime patch 0045: two lines.
