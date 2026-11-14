<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the MIT game runtime (RX3)

The checklist for `rustic-isles-runtime.dll`, the MIT game runtime built from `crates/rustic-isles-runtime` and its
pure half `crates/rustic-isles-runtime-core` (plan item 11, RT-1; [`spec-runtime-r0.md`](../port/spec-runtime-r0.md),
[`spec-runtime-r1.md`](../port/spec-runtime-r1.md), [`spec-runtime-r2.md`](../port/spec-runtime-r2.md)).
`SECURITY_REVIEW.md` indexes it. This page covers R0 (the spine), R1 (config) and R2 (backend HTTP); each later slice
adds its own section.

How to use it: before the first release that ships the MIT DLL beside `Rustic-Isles.dll`, copy this list into that
release's review, check each item in the code (not from memory), link the code or test that shows it, and sign at
the end with the date and the commit reviewed. An item that does not hold is a bug to fix before the release, not a
box to leave open.

## What it is and where the boundary is

The runtime runs inside the game process, a player's client or a host's game server, with that process's rights.
It is injected into a game started suspended (RD9 as amended), before `Rustic-Isles.dll`, and its `DllMain` runs
under the loader lock before the engine starts. It listens on nothing and opens no connection; from R2 on it
rewrites the game's own HTTP requests and adds headers to them. Everything it reads from the game's memory may be
half-built or freed, so no game pointer is trusted.

- **In:** the process environment (the backend address, the auth mode, R1's settings, a player host's launch
  variables), the
  command line (the role, a server's launch arguments: the gameserver key, the port, the map, the roster), flag
  files and number files next to the executable, the game's memory (object array, name pool), and calls from the
  AGPL runtime through the five seam exports. From R2: every URL the game's HTTP requests carry, and a grant id
  header already on a retried request. From R10: the control pipe's bytes from the launcher.
- **Out:** the per-port log file next to the
  executable (shared with the AGPL runtime), MinHook's patches on `ProcessEvent`, the engine tick and
  `FCurlHttpRequest::ProcessRequest`, and an exit code (`0xE106`) for a refused full-launch port. From R2: the URL of
  every request to an allowlisted host, rewritten to the backend, and on a server the `x-rustic-isles-grant-id` and
  `x-rustic-isles-gameserver-apikey` headers. From R10: the ready pipe's message, the `x-rustic-host-capability`
  header and the exit codes `0xE103`, `0xE104`, `0xE105` and `0` of a player host.
- **Trusted:** the AGPL runtime in the same process (it calls the exports with its own detours and pointers), and
  the launcher or Director that set the environment and command line.

Code: `crates/rustic-isles-runtime/src/` (`ffi/` holds every `unsafe` line: `dllmain.rs`, `exports.rs`,
`broker.rs`, `minhook.rs`, `dispatch.rs`, `http.rs`, `memory.rs`, `engine.rs`, `log.rs`, `flags.rs`, `win.rs`;
safe: `lifecycle.rs`, `config.rs`, `handlers.rs`), `build.rs`, the vendored `minhook/` (v1.3.3, BSD-2-Clause), and
the core crate's `config/`, `chain.rs`, `http.rs`, `registry.rs`, `engine.rs`, `memory.rs`, `pdata.rs`, `flags.rs`.

Fuzz targets in `fuzz`:

- `rustic_isles_runtime_config`: any text as a backend address is accepted only as `scheme://host[:port]` with a real
  port and a lower-case host that reads back the same, never a panic; any argv list gives a launch with a non-zero
  port or a refusal, a map URL that always carries `?MaxPlayers=`, and a roster count within the cap; any text as a
  game URL splits only into a non-empty host with no path, query or port and a tail starting `/`, is rewritten only
  for an allowlisted host and only to a URL under the API base shorter than 32 767 units, and gives a grant path only
  under `/progression/`.
- `rustic_isles_runtime_player_host`: any environment, argv and control message give the bootstrap check a verdict
  without a panic; a `Valid` verdict meets every grammar of R10 5.2 and 5.3, checked again in the target; and
  `is_fence` is true only for the exact fence request.

## Checklist

### Unsafe code and panics

- [ ] `unsafe_code` is allowed on exactly one module, `ffi` (`lib.rs`); the rest of the DLL crate and all of the core
      crate are safe (the workspace denies it).
- [ ] Every export and every detour body runs inside `catch_unwind` (`exports.rs`, `dispatch.rs`): a panicking
      export returns its failure value (`0`, `MH_UNKNOWN`, or the target itself for `rir_original`); a panicking
      detour logs `[Panic] <id>` once and calls its next with the original arguments, unless the chain already ran.
      `DllMain` catches a panicking Init and still returns TRUE.
- [ ] No panic crosses FFI: the release profile unwinds, so the catches above see every panic.

### Game memory

- [ ] Every read of game memory goes through `ffi::memory` (`readable`, `read`, `read_bytes`): `VirtualQuery` first,
      committed, neither guard nor no-access, inside one region, no overflow (`region_allows`, tests in
      `memory.rs`). A failed read is `None`, never a fault; no read relies on R3's exception handler.
- [ ] Engine reads (object array, name pool, full names) bound every index and count (`MAX_OBJECTS`, chunk count,
      8192 name blocks, 64 outers) and read through the same checks (`core::engine`).
- [ ] Names fail closed: if object 0 is not `Package CoreUObject` the check logs once and every name is `None`, so no
      handler matches (`ffi::engine::names_on`).
- [ ] `patch_bytes` writes only when the bytes are the expected ones, restores the old protection and flushes the
      instruction cache (`patch_bytes_at`; tests on a page the test allocates).

### The broker and the seam

- [ ] One MinHook copy patches a target, once: the broker hooks each target at most once and runs a chain on it
      (`broker.rs`); the stub page is `PAGE_EXECUTE_READ` and only the pointer cells are written, by aligned atomic
      stores, inner cells first and the head last.
- [ ] The AGPL band holds at most one wrapper per target; a second add is `3` and changes nothing; a remove of an
      absent wrapper is `4` (`core::chain`, `ffi::tests::broker_chain_on_real_code`).
- [ ] `rir_owns` reads at most 48 bytes of its argument and answers `0` for null, unknown or over 47 bytes; it takes
      no lock (`exports.rs`, `core::registry`).
- [ ] `rir_chain_add_inner` refuses a null `next` or wrapper and writes `*next` only on success.
- [ ] The exports log nothing directly: they run under the AGPL's loader lock, so their lines are deferred to the
      first detour call.
- [ ] The DLL is pinned (`GET_MODULE_HANDLE_EX_FLAG_PIN`) and frees no stub, cell or trampoline, so no chain ever
      points into freed memory.
- [ ] The ownership registry lists every surface id once with its slice; `owns` answers yes exactly for the ids of
      the slices in `LANDED` (`R0`, `R1`, `R2`, `R3`, `R4`: SP-1..3, HTTP-1, HTTP-1@R2, EXIT-1..14 and CL-1, 2, 5, 7,
      8, 9; `R7`: LD-1..9, `SP-2@R7`, `SP-3@R7`; `HTTP-1@R10` and `SP-2@R20` stay the AGPL's) (`core::registry`
      tests). R4's seven logging-only probes (CL-3, CL-4, CL-6, CL-10..13) are slice `R19`, never owned.

### Detach and the loader lock

- [ ] After `DLL_PROCESS_DETACH` nothing logs, opens a file or reads a flag; every detour only forwards
      (`ffi::detaching`, `dispatch.rs`, `flags::flag`, `log::enabled`).
- [ ] Init starts no thread, waits on nothing, loads no library and writes nothing to the log; the first lines go out
      on the first detour call (`lifecycle.rs`).
- [ ] A dispatch call nested deeper than 32 is absorbed, on every thread
      (`ffi::tests::depth_guard_absorbs_from_33`).

### Exit and crash handling (R3)

- [ ] Every R3 detour and the vectored handler forward at once while detaching or when this thread is already inside
      R3 (a const thread-local with no destructor), so an exit or exception raised by R3's own logging never
      re-enters it (`ffi::exit::enter`, `ffi::exit::tests::export_detour_on_a_stand_in`).
- [ ] Every R3 hook calls its next with the arguments it was given; the handler answers only
      `EXCEPTION_CONTINUE_SEARCH`, except for the FName recovery. `HandleError` always runs the original.
- [ ] The FName recovery rewrites one register (`Rdi`) of the faulting thread, only for an access violation at exactly
      `+0x02660278`, and only when the bytes there are `0F B7 07` and `.pdata` puts them in `+0x026601A0`; it points
      `Rdi` at a 32-byte zero buffer this DLL owns and never writes (`ffi::exit::veh`, the test-page test).
- [ ] The handler reads `EXCEPTION_POINTERS`, the record (0x98 bytes) and the context (0x4D0) through `readable`
      before touching them; the `CONTEXT` offsets are checked against `windows-sys`.
- [ ] Exception lines are bounded: one budget of 96 for EXIT-5 and EXIT-12 together, the MSVC C++, thread-name and
      debugger-print codes never logged, recovery lines throttled (64, then one per 1 024). A stack overflow is
      written from a fixed 1 KiB buffer, with no register dump or stack walk.
- [ ] The engine's error text is read unit by unit through checked reads, at most 1 023 units, and written as one
      line: controls as `?`, line breaks as `
`.
- [ ] Exit lines are written even under the loader lock (`CreateFileW`, `WriteFile`, `CloseHandle` only; R3 call 4),
      the one exception to "nothing under the loader lock".
- [ ] `ReportCrash`'s wait is patched only through `patch_bytes` (expected bytes, inside `ReportCrash` by `.pdata`),
      and `DISABLE_CRASH_WAIT_SHORTEN.flag` keeps the native 60 s.

### Configuration (R1)

- [ ] The backend address is accepted only as `scheme://host[:port]` (`http` or `https`; DNS name, dotted quad or
      bracketed IPv6; port 1 to 65535); no user info, path, query or fragment (`core::config::backend`).
- [ ] The first set variable is the only one used: a refused `RUSTIC_ISLES_BACKEND_URL` never falls to
      `DAINTY_BACKEND_URL`; a refused or absent address is `https://backend.invalid`, which never resolves.
- [ ] A refusal's reason text never echoes the value (it may carry credentials); the log names the variable, the
      reason and the length only (`reasons_never_echo_input`).
- [ ] An unreadable `RUSTIC_ISLES_AUTH_MODE` is `Invalid`, not a gameserver (R1 departure 2).
- [ ] A full launch's port must be 1 to 5 plain digits, 1 to 65535; anything else ends the process with `0xE106`
      before any hook.
- [ ] Every environment read is bounded before it is copied (`win::env_units` reads the size first) and refuses NUL,
      CR, LF and non-ASCII where R0's `env_value` applies.
- [ ] Number files are read up to 64 bytes.

### Backend HTTP (R2)

- [ ] Only `http`/`https` URLs whose host is one of `steelyard.ca`, `steelyard.online`, `ol.epicgames.com`,
      `api.epicgames.dev` or a dot-separated subdomain of one are rewritten, ignoring ASCII case; never a substring
      match (`core::http::is_redirect_host`, `allowlist`). The host is taken after the first `@` and before the
      first `:`, so user information and the port never reach `/__origin/`.
- [ ] A refused or absent backend address leaves the redirect pointing at `https://backend.invalid`, so the game's
      requests fail closed instead of reaching their original host.
- [ ] The gameserver key is set only on a DeployServer (`GlobalGameserver`, full launch) and only on a URL that is
      the API base followed by `/`; a host that only shares a prefix with the base (`backend.invalidx`) never gets
      it (call 4, `core::http::sends_key`, `key_header_per_section_6`).
- [ ] Every request field is read through `ffi::memory` after the `0xD0`-byte request, the URL buffer
      (`num` 2 to 32 768, `max >= num`) and the 17-slot vtable are checked readable; a failed read skips the step.
- [ ] The rewritten URL buffer comes only from `FMemory::Realloc`, never Rust's allocator; a null result leaves the
      old URL (`failed_realloc_leaves_the_url`). `GetHeader`'s copy goes back only through `GMalloc`'s `Free`.
- [ ] A grant id already on a request is kept, readable or not, so a retry never turns one grant into two; without
      random bytes no id is set and the backend refuses the grant (fail closed). Ids come from `ProcessPrng`.
- [ ] Every request reaches the game: the detour never absorbs a call and a panic in its steps is logged once and
      the call goes on (`detour_through_the_broker`).

### Login, EAC and the command line (R4)

- [ ] CL-1 never logs the command line (a client's carries the exchange code, a server's the gameserver key); its
      summary line holds booleans and the effective `MULTIHOME` only (`core::login::cmdline`).
- [ ] A `MULTIHOME` reaches the served line only from the OS line, on a server, and only as an IP literal by R1's
      strict grammars; one that fails is logged by length, never by value, and is not published
      (`multihome_outcomes`). `engine_multihome()` is `None` until the line is built, so R9/R10's pinned-listen check
      fails closed.
- [ ] The engine's line is read through `ffi::memory`, at most 32768 units, and frozen only once it is non-empty; the
      served copy is leaked, never freed, since the engine keeps the pointer (`command_line_served_once_non_empty`).
- [ ] CL-5 reads the EAC message through `readable` and at most 512 units; it absorbs only messages naming
      `IGameClient` or `Failed to create`, and never logs the text.
- [ ] CL-7 writes one byte, only in a committed page whose protection allows writing (`cl7_write_rule`).
- [ ] CL-8/9 rewrite only a node name holding `gce.latency-test.steelyard.ca` (NUL within 256 units) to a static
      `127.0.0.1`; hints and results pass through untouched. `ws2_32.dll` is a static import, so no library is loaded
      in Init (`tests/dll_file.rs`).

### Listen and readiness (R9)

- [ ] R9 owns `SRV-1` and `SP-3@R9` only; with them owned the AGPL stands down from the listen exactly when the MIT
      listens, and a player host keeps the AGPL's listen until `SP-3@R10` is owned, so the two DLLs never both listen
      or both stand down (`core::listen::listen_decision`, runtime patch 0052).
- [ ] An unknown `RUSTIC_ISLES_AUTH_MODE` ends the server with `0xE102` on its first frame, before any listen.
- [ ] SRV-1 writes the map URL only into an engine allocation (`FMemory::Realloc`, never Rust's allocator); a null
      result leaves the engine's own map (`ffi::listen::tests::srv1_write_path`). Its log line holds the map path,
      the length and the cap, never the hunt id or roster.
- [ ] `InitListen` is called only when its vtable slot is a function start in the executable's `.pdata`; the world
      list (64), the named drivers (128), client connections (1 024) and open channels (4 096) are bounded and read
      through `ffi::memory`; every driver write goes through the reflection kit's checked `write`.
- [ ] A pinned player-host address is honoured only when it equals the engine's `MULTIHOME` (R4); otherwise the
      listen fails closed and R10 fences the host. With no registered R10 module the listen fails without a fence.
- [ ] The ready lines go only to this process's standard output, with a launch id that passed R0's `env_value`
      (no CR, LF, NUL or non-ASCII, at most 127); no other environment value is logged.
- [ ] Per-frame driver upkeep writes nothing to a driver that is no longer live or readable (R9 departure 4).
- [ ] `rir_listen_driver` is a lock-free load that catches panics and answers null until a listen succeeded; it is
      outside the seam's required set (`tests/dll_file.rs`).

### Player data and loading (R7)

- [ ] LD-1 never turns the game's true answer false and calls the original first on every call; a client forces only
      the idle arrival trigger and its own local controller, never before a stable possession, 20 s, and a weapon
      (`core::loading::force_controller`).
- [ ] The possession capture forgets its controller when `GWorld` changes, so no check reads a freed controller.
- [ ] Every read of a manager, controller, loader, hunt system, component or trigger goes through `ffi::memory`
      after its span is checked; a failed read ends the step and the call reaches the game.
- [ ] R7 writes game memory in four places only, each a checked write into a writable region: a stuck loader's
      `bLoading`, the manager's `bBegun` (the retry), `PlayerCanRestart`'s `ReturnValue` (the gate), and the query
      function's `FunctionFlags`, restored after the call (`ffi::loading::game::write`).
- [ ] The retry runs once per manager and controller pair; a second failure reaches the game's handler, so a player
      is never held on a loading screen (`core::loading::RetryTable`). The restart gate releases after 90 s.
- [ ] The calls R7 makes (`Begin`, the weak resolver, `CompleteLoadInternal`, `ProcessEvent`) take arguments R7
      checked first; UFunction calls pass the parameter guard (`ParmsSize` within the block) and never run after
      detach.
- [ ] R7's lines carry pointers, counts and the hunt id only, never an account id or a key.

### The player host (R10)

- [ ] Every launch variable is bounded before it is copied and checked against R10 5.2 and 5.3's grammars
      (`core::player_host::bootstrap`); any failure ends the process with `0xE103` before listen, on the first
      frame. A manual-mode launch is never a player host. The log names the failed check, never a value.
- [ ] The capability is 43 safe characters, kept in memory only, sent only as `x-rustic-host-capability` to URLs
      under the API base (`ffi::player_host::add_header`, `header_only_under_the_api_base_and_idle`), never logged,
      never written to a pipe, never sent with the gameserver key header.
- [ ] The listen is pinned to `127.0.0.1`: the bind address must be exactly that, the command line must carry
      `-MULTIHOME=127.0.0.1`, and R9 refuses the pin unless the engine's `MULTIHOME` equals it.
- [ ] The pipe names are taken only under `\\.\pipe\RusticInternalServer.` with 16 to 96 safe characters after it
      (no other pipe, no path, no remote server; `core::player_host::is_pipe_name`).
- [ ] The control pipe: one instance, first instance, remote clients rejected, message mode, a protected DACL for this
      user and `LocalSystem` only (`the_dacl_holds_two_entries`); the connecting process must be
      `RUSTIC_ISLES_LAUNCHER_PID`; the only accepted bytes are the exact fence for this session and generation, at
      most 511 bytes. Anything else fences the host (fail closed): a wrong client can end a host, never steer it.
- [ ] The fence ends the process (exit 0) on the next frame, and within 5 s from the control thread (`0xE105`) if the
      game thread does not tick.
- [ ] The ready pipe is opened with `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`, so the launcher's end cannot
      impersonate the game process; the message carries session, generation, launch id, port and `bind=127.0.0.1`,
      no secret.
- [ ] A listen that fails, or a readiness report that fails, fences the host.
- [ ] Never logged by R10: the capability, the session id, the launch id (it shares its random part with the pipe
      names), the pipe names, the launcher pid, a client's pid. `Bootstrap`'s `Debug` prints the role only.
- [ ] No `MYSTICPARADOX_*` variable is read or written; the alias is gone (RT-8), and a test reads the runtime crates'
      sources for the old names (`no_mystic_paradox_names_in_the_runtime_sources`).
- [ ] Fuzzed by `rustic_isles_runtime_player_host`: the bootstrap check never panics, a `Valid` verdict meets every
      grammar, and `is_fence` holds only for the exact request bytes.

### Net driver safety (R11)

- [ ] `R11` is in `LANDED` only with `R9` (`registry::tests::r11_lands_with_or_after_r9`): owning `SP-3@R11` turns
      the AGPL's driver upkeep and manual tick off, and the MIT ones read R9's listen state.
- [ ] Every NETD detour and every function R11 only calls (`TickDispatch`, `TickFlush`, `FindNamedNetDriver`,
      `StaticConstructObject_Internal`) is checked against its first bytes in `.pdata` first; a mismatch hooks or calls
      nothing (`ffi/net/mod.rs`, `Callee`). NETD-8 is owned and never installed.
- [ ] The sanitiser walks `ClientConnections` only after `sane_pointer_array` (1 024) and keeps only connections
      readable for `0x138`; a bad array is emptied, never freed (an engine allocation is not ours to free). Its
      repairs and every other R11 write go through a checked write (readable, writable region, aligned).
- [ ] No fault is caught around the manual dispatch and flush (rulings R11): a fault there ends the server through
      R3's crash path. The drive runs once per frame per thread (a re-entry flag guards both calls).
- [ ] NETD-3's two temporary changes (the feature array's `Num`, the class's `Abstract` bit) are put back within the
      same call on every path, a panic included.
- [ ] NETD-4/5 read and write nothing and answer a full 32-bit 1, after detach too (`ffi::net::tests`).
- [ ] Every repeating `[MitNet]` line has a budget; NETD-6 logs the first 16 closes only, and never reads the
      connection's `+0x1318`. No line carries a player name, address or key: only pointers, counts and the stack.

### Logging

- [ ] Never logged: the command line, the gameserver key (`api_key`) or which request carried it, argv entry 1, any
      environment value R1 marks secret, a refused backend URL (`lifecycle.rs`, `config.rs`, `log.rs`), a URL's
      path, query or fragment beyond a grant's `/progression/...` path, or any header value but the grant id. R2's
      lines turn control characters into `?` (`core::http::log_lines`).
- [ ] One `WriteFile` per line; a writer that cannot take the lock within about 50 tries writes through a private
      open-append-close, so logging never deadlocks (`log.rs`).

### Build

- [ ] The DLL imports no `MSVCP140*`, `VCRUNTIME140*` or `api-ms-win-crt-*` (`tests/dll_file.rs`); MinHook is built
      `/MT` by `cc` from the vendored tag, never from `restricted/`.
- [ ] The five seam exports exist by name (`tests/dll_file.rs`).

### Headless server boot (R5)

- [ ] Nothing runs on a client; `headless::install` is called only with the server role (`lifecycle.rs`).
- [ ] HL-1's two bytes are checked committed and writable once in Init; the tick writes store only to those
      (`ffi/headless.rs`).
- [ ] HL-11 writes a UFunction's native pointer only after the object is found by its exact full name, is live, and
      the pointer it replaces is a `.pdata` function entry in the executable; otherwise it writes nothing and logs
      `HL-11 NOT installed` (fail closed against a wrong `+0xD8`).
- [ ] The HL-11 function writes the return slot only when 8 bytes there are writable.
- [ ] HL-4 hooks only after `verify_entry` matches the 18-byte prologue; on a mismatch CEF starts as before.
- [ ] Init never calls `LoadLibrary` and logs nothing (lines deferred); the HL-6 path hooks d3d11 without logging.
- [ ] The nullrhi cache (`+0x06CD7878`) and its guard are never written.

### Widget crash guards (R6)

- [ ] Every UI target fails closed: `widgets::verify` checks each entry's bytes (UI-1's leaf, UI-3's resolved entry,
      UI-5..7's parent calls and parents, UI-12's five getters) before any hook; a mismatch hooks nothing.
- [ ] UI-4..6 install all or nothing; a failed hook unhooks the others, and a failed unhook is logged
      (`widgets::install_group`).
- [ ] Every member a guard reads goes through checked reads (UI-7 through the frame cache); a guard calls the game's
      function, a verified parent body at `base + rva`, or nothing.
- [ ] Virtual calls (UI-4's `GetOwningPlayer`, the `FText` controller's two entries) are made only when the entry lies
      in the executable's image (`core::widgets::in_image`); otherwise nothing is called.
- [ ] The `FText` release decrements its two counts with interlocked operations, only on an aligned, writable,
      committed word (`ffi::widgets::GameRefs`).
- [ ] While detaching, a guard still decides (it never forwards a faulting call) but logs and counts nothing.

## Sign-off

Reviewed at commit `________` on `________` by `________`.
