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

- **In:** the process environment (the backend address, the auth mode, R1's settings, `MYSTICPARADOX_*`), the
  command line (the role, a server's launch arguments: the gameserver key, the port, the map, the roster), flag
  files and number files next to the executable, the game's memory (object array, name pool), and calls from the
  AGPL runtime through the five seam exports. From R2: every URL the game's HTTP requests carry, and a grant id
  header already on a retried request.
- **Out:** `RUSTIC_ISLES_*` variables set in its own process (the alias), the per-port log file next to the
  executable (shared with the AGPL runtime), MinHook's patches on `ProcessEvent`, the engine tick and
  `FCurlHttpRequest::ProcessRequest`, and an exit code (`0xE106`) for a refused full-launch port. From R2: the URL of
  every request to an allowlisted host, rewritten to the backend, and on a server the `x-rustic-isles-grant-id` and
  `x-rustic-isles-gameserver-apikey` headers.
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
      `memory.rs`). A failed read is `None`, never a fault; there is no structured exception handling.
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
- [ ] The ownership registry lists every surface id once with its slice, and `LANDED` is `R0`, `R1`, `R2`: this
      build owns only SP-1, SP-2, SP-3, HTTP-1 and HTTP-1@R2; `HTTP-1@R10` stays the AGPL's (`core::registry`
      tests).

### Detach and the loader lock

- [ ] After `DLL_PROCESS_DETACH` nothing logs, opens a file or reads a flag; every detour only forwards
      (`ffi::detaching`, `dispatch.rs`, `flags::flag`, `log::enabled`).
- [ ] Init starts no thread, waits on nothing, loads no library and writes nothing to the log; the first lines go out
      on the first detour call (`lifecycle.rs`).
- [ ] A dispatch call nested deeper than 32 is absorbed, on every thread
      (`ffi::tests::depth_guard_absorbs_from_33`).

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

## Sign-off

Reviewed at commit `________` on `________` by `________`.
