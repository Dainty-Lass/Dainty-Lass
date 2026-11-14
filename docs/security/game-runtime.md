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
      8, 9; `R7`: LD-1..9, `SP-2@R7`, `SP-3@R7`; `R15`: `SP-1@R15`, `SP-2@R15`; `R8`: `SP-2@R8`, `SP-3@R8`;
      `R17`: `SP-1@R17`, `CON-1`; `HTTP-1@R10` and `SP-2@R20` stay the AGPL's) (`core::registry` tests). R4's seven
      logging-only probes (CL-3, CL-4, CL-6, CL-10..13) are slice `R19`, never owned.
- [ ] R17 adds `CON-1` (the client console) and owns it with `SP-1@R17`; the AGPL's console stands down only through
      `rir_owns("CON-1")` (runtime patch 0044).

### Game data writes and calls (R15)

- [ ] R15 writes only two fields, `FArchonCatalogItem::MaxQuantity` (`i32`, to 0, supplies with a cap above 1) and
      `FCellExchangeTableData::TimeRequired` (`i64`), and only through `ffi::economy::write_i32`/`write_i64`: R0's
      `readable`, an aligned address, and a `PAGE_READWRITE`/`WRITECOPY` (or execute) region; anything else is no
      write (`ffi::economy::tests::checked_writes`).
- [ ] Every map and array the walks read is bounded before use: 1 000 000 catalog or table slots, 1 024 tags, `FString`s
      of 65 536 units, 8 slot data entries, 64 collected slots; allocation bits past the 4 inline words need the heap
      pointer (`core::economy` tests).
- [ ] The repair calls the game only through an object's own vtable entry `0x44`, read through checked reads and
      inside the executable's image range from its PE header (`process_event`), and the loading evaluator through
      `broker::original`, on objects R0's `is_live_object` passed.
- [ ] The object-array scans read through `ScanMemory`, which reuses a `VirtualQuery` region for one scan only.
- [ ] The two settings (`DAINTY_KEEP_SUPPLY_STACK_CAPS`, `DAINTY_CELL_FUSION_MINUTES`) are read with `env_value(name,
      15)`; a refused value is echoed in its note, which is safe because neither is a secret.

### Calling the game and writing its data (R17)

- [ ] Game functions are called only through `ffi::reflect::call`, on a live object, with a function found in its
      class chain, from inside an SP-1 handler on the game thread, never after detach.
- [ ] Every call's parameter block is zeroed, 8-byte aligned and at least the dump's struct size; a function whose
      `ParmsSize` is unreadable or larger is not called and is logged once, so a wrong layout skips a feature rather
      than overwriting the stack (`core::reflect::params_fit`).
- [ ] R17 writes game data only through `ffi::reflect::write`: readable by R0's rule and in a `PAGE_READWRITE`,
      `PAGE_WRITECOPY` or execute-variant region (`is_writable_protect`). R17 writes a credits flag byte, the credits
      panel's slot order (checked writable for the whole array first), the console key's name and the viewport's
      console pointer, nothing else.
- [ ] Object scans and class walks are bounded (`MAX_OBJECTS`, 64 supers, 8 192 fields per class); `FString`s read
      back from the game stop at 32 units.
- [ ] Handlers that call into the game hold no lock across the call (`try_lock` where they come back through
      themselves), so a nested dispatch never deadlocks.
- [ ] `AUTO_PRESS_START.flag` is a test aid for scripted runs; it is never shipped to players.

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

### Server lifecycle (R8)

- [ ] R8 lands only with R9 and R10 (`registry_r8`); it owns `SP-3@R8` and `SP-2@R8` and hooks nothing. Its writes
      are the game's own fields (`ExpectedPlayerCount`, the four AFK floats) on objects that pass the live-instance
      test and `is_a`, through checked, writable, aligned writes; nothing is written to an object it did not classify.
- [ ] The watchdog thread starts from the game thread at listen, never from `DllMain`, reads only R8's atomics (never
      game memory), and returns on detach. A failed spawn logs and the server runs without it.
- [ ] Every way R8 ends a server is the reap exit, `TerminateProcess(0)`: a disposable server empty 60 s (300 s if
      never joined), a game thread silent 180 s, a port free 45 s after serving, and R10's fence (`server_quit`). A
      hub is never reaped empty or stale; a sample that is stale (over 10 s) or implausible (over 64) never reaps.
- [ ] The UDP port probe binds a fresh socket to the server's own address and port and closes it at once; it never
      sends, and an address that is not a dotted quad turns it off. Winsock is started once (`WSAStartup(2.2)`).
- [ ] The out-of-memory reserve is freed only when both engine functions' entries and bytes, the global and the
      block's region (base, allocation base, 32 MB, committed, private, read-write) check out, after a
      compare-exchange takes it from the engine's only reader; a failed `VirtualFree` puts it back.
- [ ] The working-set trim runs only on a Director hub with no client for 60 s, once per idle period.
- [ ] Never logged by R8: the roster's ids, the command line, any environment value (settings print as R1's parsed
      values and their source).

### Replication graph and channel guards (R12)

- [ ] `R12` is in `LANDED` only with `R11` (`registry::tests::r12_lands_with_or_after_r11`); RG-3 is R14's, so the
      AGPL keeps purging its player role caches on route removal until R14 lands.
- [ ] The invariant: a `PlayerController` is never bound to, sent on or given an RPC on another player's connection
      (RG-4..7). Each guard fails open only when it cannot prove the controller is another player's.
- [ ] RG-4..7 are checked against their first bytes in `.pdata` before hooking; RG-1 is owned and never installed.
      A blocked RG-4 and RG-6 return a full 64-bit 0 (`ffi::repgraph::tests`).
- [ ] The quarantine never closes or frees a channel; its one write is RG-4 clearing the cached-channel slot, through
      the checked write, only while the slot still holds the channel read. Stale entries are swept every frame so a
      reused address is never refused.
- [ ] The drive calls the graph's virtual slot `+0x2F8` only when it is a `.pdata` function entry inside the
      executable and the graph's `+0x30` is R9's driver; no fault catch (rulings R11), a re-entry flag per thread.
- [ ] Every repeating `[MitRepGraph]` line has a budget or is once per pair; lines carry pointers, RVAs, counts and
      engine names only.

### Relevance and weapons (R13)

- [ ] `R13` is in `LANDED` only with `R12` (`registry::tests::r13_lands_with_or_after_r12`); its tick reads R12's
      `repgraph::graph()` and runs at order 400, below the drive's 500.
- [ ] RG-2 is checked against its first bytes in `.pdata` before hooking; a mismatch means no route correction on
      that server at all, MIT or AGPL (the install line says so). The detour always calls the original, a panic
      only skipping the corrections.
- [ ] RG-2 writes only the registering actor's relevancy bits, its `NetDormancy` and its global info's dormancy,
      cull and grace fields; the tick only cull snapshots, a character's update rates and one
      `SetNetDormancy(DORM_Awake)` through `process_event_original` after `params_fit`. Every write is checked
      readable and writable (without R17's alignment rule, for single bytes); no channel is opened or closed.
- [ ] The map search dereferences nothing but the answer: a malformed `TMap` (counts out of range, unreadable data
      or words) answers `None`.
- [ ] Every repeating `[MitRelevance]` line has a budget; lines carry pointers, flag bytes, counts and engine names.

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

### Travel (R16)

[`spec-runtime-r16.md`](../port/spec-runtime-r16.md): TR-1 on `UEngine::LoadMap` and the world-change watcher on
SP-1, client only. `LANDED` gains `R16`, so the build also owns `TR-1` and `SP-1@R16`.

- [ ] The only write is one atomic AND clearing `Async` (`0x04000000`) in an `FUObjectItem`'s flags word, on an
      aligned, committed, writable word (`ffi::travel::clear_async`; test `clear_changes_only_async`).
- [ ] Only an item that is pending-kill and `Async`, neither rooted nor still loading, and inside a pending-kill
      world that is not `GWorld` nor a class default object, is cleared (`core::travel::is_eligible`,
      `find_leftovers`, `world_holding`; the core tests).
- [ ] The object array walk bounds the count (`MAX_OBJECTS`), the chunk index (the chunk count), checks each chunk's
      item span readable once and skips an unreadable or null chunk; class chains stop at 16 `SuperStruct` steps and
      outer chains at 17 checks (`core::travel::walk`).
- [ ] The per-pass region cache answers only under `region_allows` from a region `VirtualQuery` returned in that pass,
      and lives for one pass on the game thread (`ffi::travel::RegionCache`; test `region_cache`).
- [ ] TR-1 is installed only when its 31 prologue bytes start a `.pdata` function at `+0x04271370`; a mismatch logs
      `prologue mismatch` and the game runs unhooked. A panic in the pass or guard logs once and `LoadMap` runs.
- [ ] The guard is opt-in (`ENABLE_ZOMBIE_LOADMAP_GUARD.flag`) and refuses only for a leftover of the destination's
      name not already released; a refusal returns `false` without calling `LoadMap`. The map path read is bounded
      to 1024 units, non-ASCII becomes `?`, and it is a path, never the URL's options.
- [ ] `KEEP_STUCK_ASYNC_FLAGS.flag` turns every clear off (report only).

### Server ProcessEvent behaviours (R20)

[`spec-runtime-r20.md`](../port/spec-runtime-r20.md): five SP-2 before handlers and one SP-3 after handler, server
only. `LANDED` gains `R20` (only with `R7`: `registry::tests::r20_lands_with_or_after_r7`), so the build owns
`SP-2@R20` and `SP-3@R20`. R20 hooks nothing and patches no byte.

- [ ] Every read is checked (`readable`, `readable_this_frame`); every write goes through `put`, which checks the
      range committed and in a writable region (`ffi::server_events`). Pawn writes need the pawn live and readable
      for `0x120`; connection writes need the connection readable through the field.
- [ ] `SetClientWorldPackageName` (`+0x03F7F050`) is called only when that RVA starts a `.pdata` function, on a live
      object that `is_a` `NetConnection` and is readable for `0x1608`, with a non-zero name. No fault catch: liveness
      and class replace the AGPL's.
- [ ] `PlayerCanRestart` absorbs only when its `ReturnValue` byte is writable; R7's gate runs inside it, and R7's own
      after handler is not registered once `SP-2@R20` is owned.
- [ ] The damage multiplier needs all of the development profile, `RUSTIC_ISLES_ALLOW_DAMAGE_DEBUG=1`,
      `DEBUG_DAMAGE_MULTIPLIER.flag` and a factor that parses whole, finite, above 0 and not 1
      (`core::server_events::multiplier`); decided once. The launcher never sets the switch.
- [ ] The bleedout floor writes only on `ArchonGameMode_Island` (never the `ArchonGameMode` base), once, on a full
      launch; the fallback is bounded `0 < v < 600`, else 30 s.
- [ ] Every log line is once per process or budgeted (8 owner fix lines, the vendor throttle); `[PostLogin]` is
      off in the production log profile.

## Sign-off

Reviewed at commit `________` on `________` by `________`.
