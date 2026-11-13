<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R0, the spine (RD27, plan item 11)

R0 is the first slice of the MIT game runtime (`spec-runtime-surface.md` section 8, approved 2026-11-13 with rulings
RT-1 to RT-18 in `rulings-2026-11-13.md`). It ports **no game feature**. It builds the DLL every later slice adds to:
the crate layout, the process lifecycle, logging, flag files, environment reads, the pointer-safety helpers, the
hand-written offset table, MinHook, the **hook broker** and **ownership** exports that let it load beside the AGPL
`Rustic-Isles.dll`, and the dispatch chains on `ProcessEvent` and the engine tick with no feature on them.

An implementer builds from this page, the surface page and `spec-runtime-r1.md` alone, and never reads `restricted/`.
What crossed is facts: RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7, CL 647472), engine layout offsets, file and
variable names, exact texts the operator reads, and behaviour. The AGPL side of the coexistence (runtime patch 0041)
was written by the spec writer in `restricted/` and is described here only by the contract it keeps (section 7).

Conventions are the surface page's: an RVA is `+0x0XXXXXXX` from the executable's image base; **C** client, **S**
server, **B** both. **(call)** marks a decision this page makes; **(ambiguity)** something the source did not settle;
both are collected in section 12.

---

## 1. Scope

| R0 owns | Section |
| --- | --- |
| The crates `rustic-isles-runtime` (the DLL) and `rustic-isles-runtime-core` (pure, tested everywhere) | 2 |
| `DllMain`, the one-time init, the process-detach rule | 3 |
| Environment reads, the development profile | 4.1 |
| Flag files and their cache; R0's own flags | 4.2 |
| The per-port log file | 4.3 |
| Pointer-safety helpers, `.pdata` lookup, verified byte patches | 5 |
| The hand-written offset table (RT-2), and the engine reads R0 needs (object array, names) | 6 |
| MinHook, the broker, the seam exports, the ownership registry | 7 |
| The dispatch chains SP-1, SP-2, SP-3: depth guard, name cache, handler registration, no handlers | 8 |
| How both DLLs are loaded (RT-5, RT-6), for the later rows that change the launcher, Director and feeds | 9 |

Not R0: every hook id other than SP-1/2/3 (each later slice), the launch-argument and config parsers (R1), MainThread
and its world wait (R9), the store capture (lane E, RT-10).

**R0 and R1 land together (call).** The log file name needs the server's port, which R1's launch-argument parser
gives (section 4.3). R1 is pure and inert, so landing both in one release costs nothing.

---

## 2. Crate layout (RT-1)

Two crates in the root workspace (`members = ["crates/*"]` picks them up; the integration pass adds the
`[workspace.dependencies]` lines and `cc`):

| Crate | Kind | Holds | Builds on |
| --- | --- | --- | --- |
| `crates/rustic-isles-runtime-core` | `lib` | everything pure: the flag-cache policy, log line formatting, env value checks, the offset table, `.pdata` binary search and byte-pattern checks over a slice, the broker's chain model, the ownership registry, name matching, R1's `config` module | every platform; `cargo test` runs it on Linux CI |
| `crates/rustic-isles-runtime` | `["cdylib", "rlib"]` | `DllMain`, the C exports, the Windows calls, MinHook, the detours | `x86_64-pc-windows-msvc`; on any other target every module is `#[cfg(windows)]` and the crate is empty |

- `license = "MIT"`, `publish = false`, `[lints] workspace = true`.
- **`unsafe_code` (RT-1).** The workspace denies it. The DLL crate allows it on exactly one module:
  `#[allow(unsafe_code, reason = "FFI: DllMain, the seam exports, MinHook and game memory")] mod ffi;`. Every raw
  pointer read, Win32 call, MinHook call and detour body lives under `ffi`; the rest of the crate and all of the core
  crate stay safe. A `#[unsafe(no_mangle)]` export is unsafe code too, so the exports live in `ffi`.
- **MinHook through `cc` (RT-1).** Vendor upstream MinHook **v1.3.3** (TsudaKageyu/minhook, BSD-2-Clause) into
  `crates/rustic-isles-runtime/minhook/` with its `LICENSE.txt`: `src/buffer.c`, `src/hook.c`, `src/trampoline.c`,
  `src/hde/hde64.c` and their headers. **(call)** Take it from upstream's tag, not from `restricted/` (it is the same
  BSD code; this keeps the MIT tree free of anything copied out of `restricted/`). `build.rs` compiles it with `cc`
  only when `CARGO_CFG_TARGET_OS == "windows"` and `CARGO_CFG_TARGET_ARCH == "x86_64"`. The workspace's
  `.cargo/config.toml` already sets `+crt-static` for `x86_64-pc-windows-msvc`, and `cc` follows it (`/MT`).
- **Output name (call).** A Rust library name cannot hold `-`, so the build writes `rustic_isles_runtime.dll`
  (`[lib] name = "rustic_isles_runtime"`). Publishing installs it as **`rustic-isles-runtime.dll`** (RT-5): the
  host kit's `publish-runtime` and the release copy it under that name. Nothing loads it by its build name.
- **Imports.** The DLL imports no `MSVCP140*`, `VCRUNTIME140*` or `api-ms-win-crt-*` (the game folder ships an old
  `msvcp140.dll`, surface 2.1); a test checks it (section 10). It may import `kernel32`, `ntdll`, `user32`, `ws2_32`,
  `shell32` (`CommandLineToArgvW`, R1) and `bcryptprimitives`/`advapi32` as std pulls them.
- **Dependencies.** `std`, plus Win32 bindings: hand-declared `extern "system"` items or `windows-sys` (through the
  integration pass, smallest feature set). No async runtime, no logging crate (the log format is fixed, 4.3).
- **Panics never cross FFI.** The workspace's release profile unwinds. Every export and every detour body runs inside
  `std::panic::catch_unwind`. A panicking detour logs once (`[Panic] <id> ...`) and calls its `next` with the
  original arguments, so the game sees the original function. A panicking export returns its failure value.
- **Module per slice.** Each later slice adds its own folder under `crates/rustic-isles-runtime/src/` (and pure parts
  under the core crate), plus one line in the ownership registry (7.5). No two slices share a file except the
  registry.

---

## 3. Lifecycle

### 3.1 `DllMain`

| Reason | Does |
| --- | --- |
| `DLL_PROCESS_ATTACH` | `DisableThreadLibraryCalls`; pin the module (`GetModuleHandleExW` with `GET_MODULE_HANDLE_EX_FLAG_PIN`, so it is never unloaded while a chain points into it); run Init (3.2); return TRUE. Init never fails the attach: a broken step logs later and leaves that part off |
| `DLL_PROCESS_DETACH` | set the process-wide **detaching** flag (an atomic, SeqCst) and return. Nothing else |
| thread attach/detach | not delivered (disabled) |

The DLL is loaded **before** `Rustic-Isles.dll` (RT-5, section 9) by injection into a suspended game, or by the
runtime loader in its own `DllMain` (Proton). Either way `DllMain` runs under the loader lock, before the engine reads
its command line or starts plugins. So Init must: allocate only through the process heap, take only SRW locks, start no
thread and wait on none, load no library except through `GetModuleHandleW` of one already loaded, and write nothing to
the log (4.3 says when the first lines go out).

### 3.2 Init order

1. Resolve the executable's folder (4.2) and decide the flag-cache mode.
2. Run R1's environment alias pass (`MYSTICPARADOX_` to `RUSTIC_ISLES_`, R1 section 3), then R1's backend address
   resolution (R1 section 4); keep the results.
3. Role: **server** iff the process command line (`GetCommandLineW`) contains `-server` (a case-sensitive substring,
   anywhere). Image base: `GetModuleHandleW(NULL)`.
4. Server: R1's auth mode and launch arguments (R1 sections 5, 6) give the port; client: port 0. A full launch
   whose port R1 refuses ends the process here with exit code `0xE106` (R1 6.1).
5. `MH_Initialize`.
6. Install this build's owned hooks for the role, in the order of section 7.6. R0 installs only the dispatch
   chains: client SP-1; server SP-2 then SP-3.
7. Record a status (hook results, the seam version, the count of owned ids) for the first log write.

Init runs once; a second attach (impossible while pinned) would do nothing.

### 3.3 The detaching rule

Once the detaching flag is set, nothing in this DLL logs, opens a file, allocates on behalf of logging, or reads a flag
file. Every detour checks the flag first and, when set, only forwards to its `next`. (The AGPL runtime crashed every
client exit until it learnt this; its rule and this one are the same.) Process exit detaches the DLLs in reverse load
order, so `Rustic-Isles.dll` detaches before this one; the broker's chains stay valid because neither DLL is unloaded.

---

## 4. Environment, flags and the log

### 4.1 Environment reads

`env_value(name, max) -> Option<String>`: read the variable with `GetEnvironmentVariableW`; `None` when unset, empty,
longer than `max` UTF-16 units, or containing NUL, CR or LF. **(call)** Also `None` when it contains any non-ASCII
character (the AGPL reads the ANSI form, where a non-ASCII value turns into code-page bytes; no variable the runtime
reads can legitimately hold one). Every later slice reads the environment only through this helper, each with its own
`max`. The core crate holds the check (`check_env_value(&[u16], max)`) so it is tested everywhere.

**Development profile.** `development_profile()` is true iff `RUSTIC_ISLES_RUNTIME_PROFILE` is exactly `development`
(11 characters, case-sensitive). Read once and kept. Only the launcher sets it; it unlocks developer behaviour in later
slices. The host kit writes `production` for game servers.

### 4.2 Flag files

A flag is a file system entry next to the game executable: `<exe folder>\<NAME>.flag`. It is "present" when
`GetFileAttributesW` on that path succeeds (a folder of that name counts too: same as the AGPL).

- **The folder.** From `GetModuleFileNameW(NULL)`: everything up to and including the last `\` or `/`. Resolved once
  in Init into fixed storage and published through an atomic three-state word (unresolved, publishing, ready). If it
  cannot be resolved (a path of 260 or more characters), every probe resolves it again itself; a path that would reach
  260 characters with the name is "absent".
- **The cache.** A fixed table of **128** names of at most **63** UTF-16 units, each with its last answer and the
  tick (`GetTickCount64`) it was probed. A name is re-probed when its answer is older than **4000 ms**, so creating or
  deleting a flag takes effect within 4 s. The lock covers the table scan only; the file probe runs outside it. A name
  of 64 or more units, or a full table, is probed on every call. A tick of 0 is stored as 1 (0 means "never probed").
- **Mode.** On the first flag call, `UNCACHED_FLAG_FILES.flag` is probed directly (never cached). If present, every
  flag call probes the file system for the life of the process. The mode is decided once.
- **Read-once flags.** A slice that reads a flag "once" keeps the first answer in its own static; the cache does not
  change that.
- The core crate implements the policy against a `trait Probe { fn exists(&self, name: &str) -> bool }` and a clock,
  so the 4 s rule, the table limit, the long-name rule and the uncached mode are unit-tested.

**R0's flags (RT-14 for R0).** Appendix A of the surface gave R0 seven. Settled:

| Flag | Kept? | Meaning in the MIT runtime |
| --- | --- | --- |
| `UNCACHED_FLAG_FILES` | kept | every flag probe goes to the file system (above) |
| `PRODUCTION_LOG_PROFILE` | kept | servers only, read once at first use: the per-event traces a slice marks "production-quiet" are skipped. Clients ignore it |
| `VERBOSE_DIAG` | kept | read once; true only when `development_profile()` is also true. Later slices gate their high-volume traces on `verbose_diag()` |
| `NO_FRAME_POINTER_CACHE` | kept | read once at the first SP-3 tick: the frame pointer cache (5.2) is off |
| `PE_FULLNAME_UNCACHED` | kept | read once at the first SP-2 dispatch: the server builds the full name per call (8.3) |
| `MPLOG_OPEN_PER_LINE` | **dropped (call)** | it selected the AGPL's old per-line writer; the MIT runtime has one writer, which already falls back to a per-line open (4.3) |
| `PE_CATEGORY_CACHE_OFF` | **dropped (call)** | the MIT name cache keeps each handler's match result per UFunction (8.3); `PE_FULLNAME_UNCACHED` is the single revert switch |

### 4.3 The log

- **File:** `<exe folder>rustic_isles_dll_port<N>.log`, `N` the decimal port (R1: a full server launch's argv 2, the
  manual test mode's port, 0 on a client). The same file the AGPL runtime writes; both append to it.
- **Line:** `[HH:MM:SS.mmm] pid=<pid> <message>\n`, local time (`GetLocalTime`), two-digit fields and three-digit
  milliseconds, pid in decimal. One `WriteFile` per line, so a line is never interleaved with another process's or the
  other DLL's. **Raw lines** (`log_raw`) carry no prefix; R9's networking lines use them.
- **Writer:** one handle opened with `FILE_APPEND_DATA`, sharing read, write and delete, `OPEN_ALWAYS`. The handle is
  reopened when the port it was opened for changes, when a write fails, and when (checked at most every 5 s) the path
  no longer names the same file (volume serial number and file index differ, or the handle's link count is 0: the file
  was deleted or rotated away). A lock guards the handle; a writer that cannot take it after a short bounded try (spin,
  then yield, then 1 ms sleeps; about 50 tries) writes its line through a private open, append, close instead, so
  logging can never deadlock (a VEH line raised inside a write, or a lock owner killed by exit).
- A raw line for another port than the open handle's goes through a private open, so the shared handle never thrashes.
- A line over `i32::MAX` bytes is dropped. Nothing is written after detach (3.3) or under the loader lock.
- **When the first lines go out.** `DllMain` logs nothing. The first call into any R0 detour on any thread writes the
  status once (an atomic once-flag): `[MitRuntime] rustic-isles-runtime <crate version> role=client|server
  port=<n> seam=1 owns=<count> ids` and one line per install result, `[MitRuntime] install <id> ok|<reason>`.
  The flag-cache mode follows: `[MitRuntime] flag-file cache on (re-checked every 4 s)` or
  `... off (UNCACHED_FLAG_FILES.flag)`, then R1's backend line as `[MitRuntime] [BackendAddress] ...` (R1 4.4).
- Never logged: the command line, any environment value R1 marks secret, a backend URL that was refused (R1 4.4).

**(call)** The MIT lines use their own bracketed tags (`[MitRuntime]`, `[Dispatch]`, `[Broker]`, `[Panic]`) so a
reader can tell the two DLLs apart in the shared file.

---

## 5. Pointer safety and code checks

Rust cannot catch an access violation the way the AGPL's structured exception handling does. **(call)** The MIT
runtime never dereferences game memory it has not checked: every read of game memory goes through these helpers, and a
read that fails is a `None`, never a fault. (The vectored handler that recovers one known game fault is R3/R11's
EXIT-12, not a general guard.)

### 5.1 `readable(ptr, size) -> bool`

True iff `ptr` is non-null and 8-byte aligned, `VirtualQuery` succeeds on it, the region is `MEM_COMMIT`, its
protection has neither `PAGE_GUARD` nor `PAGE_NOACCESS`, and `[ptr, ptr+size)` lies inside that one region with no
overflow. It proves the memory is mapped, not that an object is alive.

`read<T: Copy>(addr) -> Option<T>` = `readable(addr, size_of::<T>())` then a volatile read. `read_unaligned` exists for
code bytes (no alignment rule, same region rule).

### 5.2 The frame pointer cache (server)

`readable_this_frame(ptr, size)` answers repeated checks on the server game thread from a 256-slot table: slot =
`((p >> 3) ^ (p >> 12)) & 0xFF`; a slot holds `(ptr, size, epoch)`; a hit needs the same pointer, the same epoch and
a stored size at least `size`. Only **positive** answers are stored. Any other thread, the client, and every call while
the cache is off (`NO_FRAME_POINTER_CACHE.flag`) go straight to `readable`. The **epoch** is a counter bumped at the top
of every SP-3 tick and on both sides of the original engine tick (where garbage collection frees objects), so a
remembered answer never spans a collection; when it wraps to 0 every slot is cleared and it restarts at 1. The cache
thread is the thread SP-3 runs on (recorded at each tick).

### 5.3 Liveness

- `is_live_object(ptr)`: `ptr` non-null and readable for `0x28`; read its object index (`i32` at `+0x0C`); look the
  index up in the global object array (6.2) with bounds checks; true iff that slot's object pointer equals `ptr`. A
  freed or recycled object fails the round trip.
- `is_actor_alive_for_replication(ptr)`: `readable(ptr, 0x60)`, `is_live_object`, the object flags (`u32` at `+0x08`)
  have neither `0x8000` (begin-destroyed) nor `0x10000` (finish-destroyed), and the actor flag byte at `+0x5C` lacks bit
  `0x08` (being destroyed). Later slices (R11 to R13) use it.
- `sane_pointer_array(data, num, max, limit)`: `0 <= num <= max`, `num <= limit`, and `num == 0` or
  `readable(data, num * 8)`.

### 5.4 Code checks and byte patches

- **`.pdata` lookup.** Parse the executable's headers from memory (DOS `MZ`, `e_lfanew`, `PE\0\0`, the section table)
  through `readable`; find the section named `.pdata`; treat it as an array of `RUNTIME_FUNCTION`
  (`BeginAddress`, `EndAddress`, `UnwindData`, each `u32` RVA). `function_entry(rva)` binary-searches it and returns the
  `BeginAddress` of the entry with `Begin <= rva < End`, or none. `primary_entry(rva)` also follows chained unwind info:
  while the entry's `UNWIND_INFO` (at `UnwindData`) has flag `UNW_FLAG_CHAININFO` (`0x4` in the top five bits of its
  first byte), take the `RUNTIME_FUNCTION` stored after its unwind codes (code count rounded up to even, 2 bytes each)
  and repeat, at most 32 times. The search itself is pure (core crate, tested on a byte buffer).
- **`verify_entry(rva, prologue)`**: `function_entry(rva) == rva` and the bytes at `rva` equal `prologue`. A slice
  that hooks "RVA + bytes" (surface 3) calls it and does not hook on `false`.
- **`patch_bytes(rva, expected, replacement) -> PatchResult`**: the site must be committed and readable (not the
  8-byte rule: code is unaligned) and must not cross its region; the current bytes must equal `expected`; then
  `VirtualProtect` to `PAGE_EXECUTE_READWRITE`, write, restore the old protection, `FlushInstructionCache`. Results:
  `Applied`, `AlreadyApplied` (the bytes already equal `replacement`: the other DLL or an earlier call did it),
  `Mismatch`, `Unreadable`, `ProtectFailed`. **(call)** `AlreadyApplied` is new; the AGPL reports it as a mismatch.

---

## 6. The offset table (RT-2) and the engine reads R0 needs

### 6.1 Format

`rustic-isles-runtime-core/src/offsets.rs`, hand-written, one entry per fact, each with the surface id that uses it:

```rust
/// Facts about Dauntless-Win64-Shipping.exe 1.14.7 (CL 647472), written by hand (RT-2). Not a generated SDK.
pub struct Target { pub id: &'static str, pub rva: u32, pub found_by: FoundBy, pub what: &'static str }
pub enum FoundBy {
    Rva,
    RvaAndBytes(&'static [u8]),
    PdataOf(u32),
    Export(&'static str, &'static str),
    UFunction(&'static str, &'static str),
}
pub const TARGETS: &[Target] = &[ /* one line per hook id */ ];
pub mod layout { pub const UOBJECT_INDEX: usize = 0x0C; /* ... */ }
pub mod data { pub const GWORLD: u32 = 0x06FC7F58; /* ... */ }
```

Rules: the table holds RVAs, offsets, sizes, prologue bytes and UE names, nothing else; no struct definitions copied
from a dump, no generated text, no code. Each slice adds its own lines (with the comment naming the slice). A test
(10.1) checks that every hook id in the registry (7.5) whose kind is a detour has exactly one `TARGETS` line, that
RVAs are unique except SP-1/SP-2 (one target, two roles), and that every `RvaAndBytes` prologue is non-empty.

### 6.2 What R0 itself needs

| Name | Value | Use |
| --- | --- | --- |
| `UObject::ProcessEvent` | `+0x02861880` | SP-1 (client), SP-2 (server). Found by RVA |
| `UEngine::Tick` (the base `UGameEngine` tick the server hooks) | `+0x03E10AB0` | SP-3. Found by RVA. Signature `(engine, f32 delta, u8 can_render)` |
| `GWorld` | data `+0x06FC7F58` | a `UWorld*` (R9's world wait; listed here for the table) |
| `UObject` object flags | `+0x08` (`u32`) | 5.3 |
| `UObject` index | `+0x0C` (`i32`) | 5.3, 8.3 |
| `UObject` class | `+0x10` (pointer) | names |
| `UObject` name | `+0x18`: `FName` = comparison index `i32` at `+0x18`, number `u32` at `+0x1C` | 8.3 |
| `UObject` outer | `+0x20` (pointer) | 8.3 |
| The global object array (`GObjects`, `FUObjectArray`'s chunked array) | data `+0x06E59ED0` | 5.3, names |
| The name pool (`GNames`, `FNamePool`) | data `+0x06E3F280` | names |
| `FName::AppendString` | `+0x02656410` | names: the fallback if decoding the pool fails the self-check (below) |

`ProcessEvent` signature: `(object, function, params)`, all pointers, returns nothing.

**Object array (UE 4.26, chunked):** at the array: `objects: *mut *mut Item` (chunk table) `+0x00`, preallocated
`+0x08`, max elements `i32 +0x10`, element count `i32 +0x14`, max chunks `+0x18`, chunk count `+0x1C`; 65 536 items
per chunk; an item is `0x18` bytes with the object pointer at `+0x00`. `object_at(i)`: `0 <= i < count`, chunk
`i / 65536 < chunk count`, both pointers readable. Bounds as the exporter's engine model (`MAX_OBJECTS = 4 000 000`).

**Names:** decode an `FName` from the name pool exactly as `spec-catalog-exporter.md` section 4.3 and the MIT
`native/rustic-isles-exporter/src/engine.rs` do (block table at `+0x10` of the pool, 2-byte stride, 16 offset bits,
entry header `bIsWide:1, probe hash:5, length:10`), using the **short form** (text after the last `/`), with the
exporter's settled answer for the number suffix. **(call)** Read that MIT code for the decoding; it is the same build.
Names are cached per comparison index (they never change in a process).

**Full name** (what dispatch handlers match, 8.3): `<class short name> <outer chain>.<name>`, the outer chain being the
outers' short names from outermost to innermost joined by `.`. Examples the later slices match:
`Function Engine.PlayerController.ClientRestart`,
`Function BP_PlayerCharacter.BP_PlayerCharacter_C.OnInputModeChanged`. A null function's name is `null`.

**Self-check (call).** The first time names are needed, R0 checks that object 0's full name is
`Package CoreUObject` (UE's first object is the `/Script/CoreUObject` package). On failure it logs
`[MitRuntime] name check failed: object 0 is "<text>"` once and every name lookup answers `None` for the process, so
no handler ever matches (features stay off; the game runs as if they were absent). This fails closed if an RVA above is
wrong.

The three name and object RVAs come from the maintainer's Dumper-7 dump of this build (`Basic.hpp`, `Offsets::GObjects`,
`GNames`, `AppendString`; Q-1, answered 2026-11-13); its `GWorld` (`0x06FC7F58`) agrees with the surface page. That
dump's SDK names objects through `AppendString` and keeps `GNames` only as a fallback. **(call)** The MIT runtime
decodes the pool (as the exporter does) and does not call game code for names; if the self-check fails with the pool,
R0 may retry it once through `AppendString` before giving up, and logs which path it uses.

---

## 7. MinHook, the broker and the seam (RT-4, surface section 7)

### 7.1 Why

Two DLLs hook one game. Two MinHook copies must never detour the same function (each would patch over the other's
jump). So exactly one DLL owns MinHook's patches: **this one**. It creates at most one MinHook hook per target and
runs on it an ordered **chain** of wrappers. Its own features register MIT wrappers; the AGPL runtime, once patch
0041 is in, hands every `MH_CreateHook` it would have made to the broker instead (7.3). A per-id **ownership** answer
(7.5) tells the AGPL which hooks and feature blocks to leave alone.

### 7.2 The chain

Per target `T`:

- The first registration on `T` makes the hook: an **entry stub** `E_T` in executable memory the broker allocates,
  `MH_CreateHook(T, E_T, &trampoline_T)`, `MH_EnableHook(T)`. The stub is a 6-byte indirect jump `FF 25 <disp32>`
  through a pointer cell `head_T` in a read-write page within ±2 GB (allocated next to the stub page). The stub page is
  `PAGE_EXECUTE_READ`; only the cell is ever rewritten, by one aligned 8-byte atomic store. **(call)** This makes the
  chain signature-agnostic and lets it change while game threads run.
- **Order:** wrappers in band `Mit` first, then band `Agpl`, then the original. Within `Mit`, by the registering
  slice's declared `order: i32` (smaller is outer), ties by registration order. Band `Agpl` holds at most one wrapper
  per target (MinHook refused a second create, so the AGPL never makes one).
- **Next:** each MIT wrapper owns a cell `next_W`, the entry of the wrapper inside it or `trampoline_T`. A MIT wrapper
  loads its `next` (Acquire) on **every** call. The AGPL wrapper's next is always `trampoline_T` (it is innermost), so
  it is handed out once (7.3) and never changes.
- `head_T` = the outermost wrapper, or `trampoline_T` when the chain is empty. The hook is never disabled or removed:
  an empty chain runs the original through the trampoline.
- Adding or removing a wrapper rewrites the affected cells under the broker's lock, inner cells first, then `head_T`
  last (Release). A call already in flight finishes on the chain it entered. Wrapper code, stubs, cells and trampolines
  are never freed (the DLL is pinned).
- A wrapper that does not call its `next` **absorbs** the call: nothing inside it runs, the AGPL wrapper and the
  original included. A slice spec says when its handler absorbs.
- `original(T)` = `trampoline_T` when the broker hooked `T`, else `T` itself. Calling it runs the game's function past
  every wrapper.

The chain bookkeeping (bands, order, the next of each wrapper, what `head` is after each add or remove, `original`)
is a pure model in the core crate; the DLL crate applies its decisions with MinHook and atomic stores.

The broker's lock is a std `Mutex` (an SRW lock: safe under the loader lock). MinHook calls happen inside it.

**Rust API for MIT slices** (shape, not names): `broker::hook::<F>(id, target, wrapper: F, order) -> Result<Hooked<F>>`
where `Hooked::next()` returns the typed `next` function to call; `broker::original::<F>(target) -> F`. A slice never
calls MinHook itself.

### 7.3 The seam exports (ABI version 1)

Exported by name from `rustic-isles-runtime.dll`, C ABI (`extern "C"`; on x64 the one Windows convention). The AGPL
patch (`restricted/.../MitSeam.cpp`) resolves all five; a copy missing one, or whose version is not 1, ends the AGPL
runtime's process with exit code `0xE10A` before any hook (two runtimes that disagree would double-detour).

| Export | Behaviour |
| --- | --- |
| `u32 rir_seam_version(void)` | returns `1` |
| `i32 rir_owns(const char* id)` | `1` when the MIT build owns `id` (7.5), else `0`; `0` for null, unknown, or over 47 bytes. Pure lookup, no lock, callable under the loader lock and from any thread |
| `i32 rir_chain_add_inner(void* target, void* wrapper, const char* id, void** next)` | adds `wrapper` in band `Agpl` on `target`'s chain (creating the hook if needed) and writes `trampoline_T` to `*next`. Returns MinHook's status numbers: `0` (`MH_OK`); `3` (`MH_ERROR_ALREADY_CREATED`) when the band already has a wrapper on `target`; any MinHook error from creating or enabling the hook, with nothing added. `id` is informational (may be empty) and is logged with the result |
| `i32 rir_chain_remove_inner(void* target, void* wrapper)` | removes that `Agpl` wrapper; `0`, or `4` (`MH_ERROR_NOT_CREATED`) when it is not there |
| `void* rir_original(void* target)` | `original(T)` (7.2) |

All five catch panics and never block for long (the lock is held only for bookkeeping and MinHook's own work).

### 7.4 What the AGPL seam (patch 0041) does with them

Recorded so the implementer knows the other side's contract (the patch is in `restricted/`, applied as runtime patch
0041, `restricted/docs/paradox-patches.md`):

- In its Init, after reading its role and image base, it looks for `rustic-isles-runtime.dll` with
  `GetModuleHandleW`. Absent: every `MH_*` call goes to its own MinHook as before; every ownership question answers
  "not owned". **With the MIT DLL absent the AGPL runtime behaves exactly as today.**
- Present: its `MH_CreateHook` and `MH_CreateHookApiEx` map the target to a hook id (the RVA table of surface section
  3, and the export table by module and procedure; `ProcessEvent` is SP-1 on a client, SP-2 on a server). Then:
  - a **chain id** (SP-1, SP-2, SP-3, HTTP-1): always `rir_chain_add_inner`; its feature blocks ask `rir_owns` per
    `<id>@<slice>` (7.5);
  - any other id the MIT owns: no wrapper; its original pointer is set to `rir_original(target)` (so code that calls
    "the original" directly still reaches the game), and the AGPL's enable/remove calls on it succeed as no-ops;
  - otherwise (not owned, or no id): `rir_chain_add_inner` with its detour.
- `MH_EnableHook`, `MH_QueueEnableHook`, `MH_ApplyQueued` only check (the broker enabled on add);
  `MH_RemoveHook`/`MH_DisableHook` call `rir_chain_remove_inner` (used only to roll back a hook group that failed).
- Hooks that are not MinHook detours are gated by id directly: HL-1 (its three `GIsServer`/`GIsClient` writes), HL-2
  (the boot patch; when owned, it reads whether the patch took from the bytes at `+0x03E07353` instead, because DX-8/9
  and the post-login match start depend on it), HL-3 (also the window-hider thread), HL-9, HL-11 (the UFunction swap),
  HL-12 (the server's locale; new id, 7.5), CL-7, EXIT-12 (both vectored-handler registrations), EXIT-14.
- Its MainThread logs `[MitSeam] rustic-isles-runtime.dll present (seam 1): <n> hooks left to it, <m> chained through
  its broker` or `[MitSeam] rustic-isles-runtime.dll absent: every hook is this runtime's own`.

### 7.5 The ownership registry

`rustic-isles-runtime-core/src/registry.rs`: every id, its kind and its owning slice, and the list of slices that have
landed in this build. `owns(id)` = the id is in the registry and its slice is in `LANDED`. A slice's only edit to this
file is adding its name to `LANDED` (and, if it introduces an id, that line). R0 lands with `LANDED = ["R0", "R1"]`.

| Kind | Ids | Owner |
| --- | --- | --- |
| chain | `SP-1`, `SP-2`, `SP-3` | R0 |
| chain | `HTTP-1` | R2 (the hook) |
| hook | every other id of surface section 3 (CL-1..13, EXIT-1..14, HL-1..11, UI-1..13, LD-1..9, NETD-1..8, RG-1..7, PR-1, ESC-1, SRV-1, TR-1, DX-1..9) | the slice of surface section 8 that lists it |
| hook | **`HL-12`** (new): the server process locale (surface 3.5, "no hook") | R5 |
| feature | `SP-1@R14`, `SP-1@R15`, `SP-1@R16`, `SP-1@R17` | the named slice |
| feature | `SP-2@R5`, `SP-2@R7`, `SP-2@R10`, `SP-2@R14`, `SP-2@R15`, `SP-2@R18`, `SP-2@R20` | the named slice |
| feature | `SP-3@R7`, `SP-3@R8`, `SP-3@R9`, `SP-3@R10`, `SP-3@R11`, `SP-3@R12`, `SP-3@R13`, `SP-3@R14`, `SP-3@R18`, `SP-3@R20` | the named slice |
| feature | `HTTP-1@R2`, `HTTP-1@R10` | the named slice |

DX-1..9 belong to R19, which is not ported (RT-3): they are never owned, stay AGPL until R21, and go with it.

**Feature granularity (call).** A feature id is "one dispatch point × one slice", not one per handler: the seam gates
every AGPL block of that slice on that dispatch point with the one id, and the slice ports all of them together. What
each id covers on the AGPL side (so the slice spec writer knows what to port; the per-slice spec lists the exact
UFunction names and order):

| Id | The AGPL blocks it switches off |
| --- | --- |
| `SP-1@R14` | airship HUD capture and reconcile; the client role pump and input activation (depth-1 pump); `OnInputModeChanged` input activation and movement repair; `InpAxisEvt_` movement forwarding; `ClientRestart` capture |
| `SP-1@R15` | patches 0024 and 0029 (the per-call data writes); the loadout slot repair (0027, 0037) |
| `SP-1@R16` | the 250 ms world census |
| `SP-1@R17` | the login-screen automation (auto press start, auto play, auto travel); login, credits and press-start branding |
| `SP-2@R5` | headless audio: absorbing `PostEvent` |
| `SP-2@R7` | the restart data gate inside `PlayerCanRestart` (patch 0012) |
| `SP-2@R10` | the player host's event observer |
| `SP-2@R14` | `ServerTryActivateAbility(WithEventData)` routing **(ambiguity: surface 4.2 does not name it; R14 is the closest owner; the R14 spec must confirm)** |
| `SP-2@R15` | patches 0024 and 0029 on the server |
| `SP-2@R18` | escalation state changes for buff choices |
| `SP-2@R20` | post-login (match start, AFK timeout write, capture reset), vendor camera, server `ClientRestart` pawn replication, `PlayerCanRestart` force, the debug damage multiplier |
| `SP-3@R7` | resuming parked progression components |
| `SP-3@R8` | frame pacing (0020); the connected-server block (watchdog, hub cap, AFK) |
| `SP-3@R9` | the listen steps, and the whole listening block, which also carries the expected-player sync (R8's) and the manual net tick (R11's) |
| `SP-3@R10` | the player host's fence check (the early return) |
| `SP-3@R11` | net driver sanitising before the engine tick |
| `SP-3@R12` | the controller channel quarantine sweep; the replication graph drive |
| `SP-3@R13` | base-actor routing; the weapon gap log and dormancy wake (0014, 0015) |
| `SP-3@R14` | player role retries, post-activation refresh, Tempest modifier ensure |
| `SP-3@R18` | escalation buff-choice spawn |
| `SP-3@R20` | the server mesh pose fix |
| `HTTP-1@R2` | the URL redirect, the progression grant id, the gameserver key header |
| `HTTP-1@R10` | the player host's capability header |

Logging-only AGPL blocks on the dispatch points (traces, censuses, probes) carry no id: they stay AGPL until R21
(RT-3).

**Cross-slice state the seam does not carry (rules for later slices).** Where an AGPL block that stays reads state an
owned id used to set, the slices must ship together or in order:

| State | Set by | Read by | Rule |
| --- | --- | --- | --- |
| "listening" | `SP-3@R9` | `SP-3@R8`, the watchdog, `SP-3@R10`'s fence path | **R8, R9 and R10 ship in one runtime release** |
| the restart data gate | `SP-2@R7` | `SP-2@R20` (it holds the call) | **R7 lands before R20** (the suggested waves already do) |
| "the boot patch took" | HL-2 | DX-8/9, the post-login match start | carried: the seam reads the patched bytes |
| the player host's launch values | the AGPL Init (not gated) | AGPL hub cap and AFK overrides | carried: the AGPL keeps reading them until R10 |

A later slice spec checks its ids against this table (its writer reads the AGPL side) and adds a row if it finds
another.

### 7.6 Install order (RT-17 for R0)

The MIT DLL installs in its own `DllMain`, before `Rustic-Isles.dll` loads, so every MIT hook exists before the AGPL
asks the broker for anything. Within the MIT DLL, by role, owned ids install in the surface's section 3.11 order
(client: SP-1 first; server: CL-1 and HL-4 first, then the rest), with one rule kept from the AGPL: DX-8/9 after
HL-2 (never ported). **Settled (RT-17):** no other order dependency exists for R0's ids; the chain order is the band
and `order` rule of 7.2, independent of install order. R0 installs only SP-1, or SP-2 then SP-3.

---

## 8. The dispatch chains

R0 registers one MIT wrapper per dispatch point (band `Mit`, `order = 0`) and gives slices a registration API. With
no slice landed the wrappers only guard and forward.

### 8.1 SP-1 (client) and SP-2 (server): `ProcessEvent`

On each call, in order:

1. If detaching: call `next` and return.
2. **Depth guard.** A thread-local counter is incremented on entry and decremented on every exit path. If it is now
   greater than **32**, the call is **absorbed**: return without calling `next` (a game recursion once overflowed the
   stack; passing the call on fed it). The first trip on each thread logs once:
   `[Dispatch] SP-1|SP-2 depth <n>: call absorbed fn=<full name or null> obj=<full name or null>`.
3. Server only, the first call: decide the name mode (`PE_FULLNAME_UNCACHED.flag`) and log
   `[Dispatch] SP-2 name cache on|off (PE_FULLNAME_UNCACHED.flag)`.
4. Run the **before** handlers registered on this point, in their `order`. A handler sees the object, the function,
   the params pointer and the cache entry (8.3), and returns `Continue` or `Absorb`. The first `Absorb` stops the
   before list and skips `next` and every after handler, but the depth counter still unwinds.
5. Call `next` (the AGPL wrapper, when the AGPL runtime is loaded, then the original).
6. Run the **after** handlers, in their `order`.

R0 registers no handlers. The order of handlers *relative to the AGPL's blocks* is fixed by the chain: every MIT before
handler runs before every AGPL block of the call, and every MIT after handler after every AGPL block. A slice spec
states where its handlers sit and whether moving them from the AGPL's position changes anything.

The AGPL wrapper keeps its own depth guard at the same threshold; since nested calls go through the whole chain, both
counters count the same depth and the MIT one trips first.

### 8.2 SP-3 (server): the engine tick

1. If detaching: call `next` and return.
2. Record this thread as the frame cache thread; bump the epoch (5.2). The first call logs
   `[Dispatch] SP-3 first tick` and the frame-cache mode.
3. **Before** handlers in `order`.
4. Bump the epoch; call `next` (the AGPL tick wrapper, which itself calls the engine tick); bump the epoch.
5. **After** handlers in `order`.

SP-3 handlers cannot absorb (the engine tick always runs). A slice that must skip a tick says so in its spec and R0's
API grows a variant then.

### 8.3 The name cache and matching (RT-18)

- **Entry**, per UFunction address, per thread (a thread-local map; entries are rewritten in place, never removed, so a
  reference held across a nested call stays valid): the object index (`+0x0C`), the name's comparison index and number
  (`+0x18`, `+0x1C`), the outer (`+0x20`), the full name (built lazily, the first time a handler needs it), and one
  match bit per registered handler with a "computed" bit.
- **Validation:** an entry is used only if the function's index, name and outer still equal the stored ones;
  otherwise it is rebuilt in place (a recycled address never returns a stale name).
- `PE_FULLNAME_UNCACHED.flag` (server): the name is built per call and nothing is cached; handlers still match.
- **Matching (settles RT-18 for the spine):** a handler declares its matcher as a list of `(Kind, pattern)` with
  `Kind` one of `Exact` (whole full name equal), `Prefix` (full name starts with), `Contains` (substring), or
  `FunctionName` (the part after the last `.` equal). The match result per handler is computed once per UFunction and
  kept in the entry's bit. Each slice spec states the kind for every name it matches; a `Contains` match is written as
  narrowly as the AGPL behaviour allows and its spec lists the names it catches.
- A handler that needs the function pointer itself (the AGPL caches some UFunction pointers) may also cache it; the
  name rule above stays the authority.

---

## 9. How both DLLs are loaded (RT-5, RT-6): for later rows

R0 makes no change outside its crates. The rows that change these files need this:

| Where | Change |
| --- | --- |
| `crates/dainty-protocol/src/manifest.rs` | a constant `MIT_RUNTIME_DLL = "rustic-isles-runtime.dll"`; the runtime set may list it beside `Rustic-Isles.dll`, optional during the transition |
| `crates/dainty-content` runtime feed (`runtime.rs`) and `dainty-hostkit` `publish-runtime`, `RUNTIME_FILE_NAMES` | publish and serve `rustic-isles-runtime.dll` with its SHA-256 when built; a set is valid with or without it. The two DLLs are a matched pair (same seam version): publish them together, never one alone |
| launcher kit: `controller/play.rs` (`runtime_dll`), `net/runtime_files.rs` (`INJECTED_RUNTIME_FILES`) | inject **`rustic-isles-runtime.dll` first, then `Rustic-Isles.dll`**, then the exporter if any; the MIT DLL only when the offered (or local) set has it, with the offered hash |
| launcher kit: `platform/loader_session.rs` (`RUNTIME_DLLS`, `is_own`), and the unknown-DLL refusal (RD9) | `rustic-isles-runtime.dll` is the runtime's own name: never a "leftover" and never refused beside the loader |
| Director: `crates/rustic-isles-director/src/runtime.rs` (`for_start`), `runtime.json` | name both DLLs with their hashes, MIT first; the MIT DLL optional while the set lacks it; fail closed as today when a listed file's hash does not match |
| Guard manifest (`rustic-rifts-guard`, `GUARD_MANIFEST_ROLES`) | **(call)** a new optional role `TRANSPORT_RUNTIME_MIT` pinning the MIT DLL for hosted realm runtimes; unpinned it expects the zero hash, as C2 settled for `TRANSPORT_MODULE` |
| `dainty-inject` | nothing: it already loads a list in order, each `LoadLibraryW` finishing before the next |
| Runtime loader (Proton, `native/runtime-loader`, `spec-runtime-loader.md`) | see below |

**The loader (RT-6 conflict; Q-2 answered: call 16).** The ruling says "the loader's ini lists both for Proton". As
built,
the loader loads its chosen runtime first and the ini's DLLs after it, skips any runtime name listed in the ini, and the
launcher refuses PLAY while an ini exists (RD9). So an ini cannot put the MIT DLL first. **Recommended:** the loader
learns one fixed name: when `rustic-isles-runtime.dll` is next to it, load it (by full path) **before** its runtime
choice; it is not a "runtime name" for the choice and not an ini entry. The loader spec's section 3 takes one rule;
the loader's tests take one case. Until the loader row lands, Proton players who keep the loader run the AGPL
runtime alone (which behaves as today).

**Injection timing.** The launcher and the Director start the game suspended and inject before its first instruction
(`docs/launcher-injection.md`, `docs/director-injection.md`): both `DllMain`s finish before the engine runs, which is
earlier than the loader path and satisfies 3.1.

---

## 10. Tests

### 10.1 In CI (core crate on every platform; DLL crate on the Windows runner)

Core (`cargo test -p rustic-isles-runtime-core`):

- Flag cache: a fake probe and clock: answer kept for 3999 ms and re-probed at 4000 ms; the 129th name probes every
  call; a 64-unit name probes every call; uncached mode probes every call; tick 0 stored as 1.
- Env checks: unset, empty, at and over `max`, NUL, CR, LF, a non-ASCII character; `development` exact, `Development`
  and `development ` refused.
- Log line format for a fixed time and pid; the raw form.
- `.pdata`: binary search over a synthetic table (first, last, gap, before the first, after the last, an empty table);
  chained unwind followed, the 32-step limit.
- Byte patch decisions over a buffer: `Applied`, `AlreadyApplied`, `Mismatch`.
- The chain model: bands and `order`; the AGPL band at most one; `head` and every `next` after each add and remove;
  `original` with and without a hook; remove of an absent wrapper.
- The registry: every id of surface section 3 present exactly once with its slice; every feature id of 7.5; `owns` for
  `LANDED = ["R0", "R1"]` is true only for SP-1/2/3; unknown, empty and 48-byte ids false.
- The offset table checks of 6.1.
- Name matching: each `Kind` on the two example names in 6.2; a null function never matches.

DLL (`cargo test -p rustic-isles-runtime`, Windows):

- **Broker on real code:** hook a test function in the test process through the broker; add two MIT wrappers with
  different `order` and one "AGPL" wrapper through the C export functions called as Rust; check the call order and
  the arguments and return value passed through; remove the AGPL wrapper and check it no longer runs; check
  `rir_original` calls straight through; an absorbing wrapper stops the chain.
- The exports exist by name in the built DLL (read its export table from the file) and `rir_seam_version() == 1`.
- The import table has no `MSVCP140`, `VCRUNTIME140` or `api-ms-win-crt-` entry.
- `readable` on a fresh page, a `PAGE_NOACCESS` page, a guard page, a range crossing a region end, an unaligned
  pointer.
- `patch_bytes` on a page the test allocates with code bytes.
- The depth guard: a detour on a test function that recurses into itself 40 times absorbs from depth 33 on.

**Never in CI:** anything against the real executable or with both DLLs in the game (surface section 9).

### 10.2 The AGPL seam (patch 0041)

It needs the host's SDK to build, so no CI can build it. The maintainer's steps are in 11.2.

---

## 11. The maintainer's hands-on checks

### 11.1 R0's play-test (surface section 8, R0 row)

"Both DLLs load; login, Ramsgate, a hunt, the store and a realm behave exactly as today (all features still AGPL).
Client exit is clean."

1. Build the AGPL runtime with patch 0041 (11.2) and the MIT DLL
   (`cargo build -p rustic-isles-runtime --release --target x86_64-pc-windows-msvc`); copy
   `rustic_isles_runtime.dll` as `rustic-isles-runtime.dll`. Publish both (or use a local runtime folder) once the
   launcher and Director rows of section 9 have landed; before that, start the client by hand with `dainty-inject.exe
   --dll <...>\rustic-isles-runtime.dll <sha256> --dll <...>\Rustic-Isles.dll <sha256> -- <game> <args>`.
2. **Client.** Log in, enter Ramsgate, open the store, start and finish a hunt, return to Ramsgate, quit from the menu.
   In `rustic_isles_dll_port0.log`: `[MitRuntime] rustic-isles-runtime ... role=client ... seam=1 owns=3 ids`,
   `[MitRuntime] install SP-1 ok`, and the AGPL's `[MitSeam] ... present (seam 1): 0 hooks left to it, <m> chained
   through its broker` with `<m>` equal to the number of hooks the AGPL logs as created. The game exits with code 0 and
   no crash dialog.
3. **Servers.** Let the Director start Ramsgate and a hunt (both DLLs injected). Each server's log shows `role=server`,
   `install SP-2 ok`, `install SP-3 ok`, `[Dispatch] SP-3 first tick`, and the AGPL's seam line. Join both, play the
   hunt, and let the empty hunt end itself.
4. **A realm.** Host a realm hub from the launcher and join it from a second client; fence it from the launcher.
5. **Regression without the MIT DLL.** Remove `rustic-isles-runtime.dll` and repeat step 2 quickly: the AGPL logs
   `[MitSeam] rustic-isles-runtime.dll absent` and everything behaves as before patch 0041.
6. Note anything that differs from the same steps on the previous release. Only CI passed until these steps are done.

### 11.2 Building and checking the seam patch (0041)

1. In `restricted/modules/rustic-isles/native/runtime`, with the host's Dumper-7 SDK in place as for any runtime build,
   run `_build.bat` (`BUILD.md`). The project now compiles `MitSeam.cpp`.
2. `dumpbin /imports x64\Release\Rustic-Isles.dll`: still no `MSVCP140`, `VCRUNTIME140*` or `api-ms-win-crt-*`.
3. Play test without the MIT DLL first (11.1 step 5), then with it (11.1 steps 2 to 4).
4. Publish with `dainty-host publish-runtime` as usual.

---

## 12. Calls, ambiguities and open questions

Calls made here:

1. R0 and R1 land together (the log needs R1's port).
2. MinHook v1.3.3 from upstream's tag, not copied out of `restricted/`.
3. Build output `rustic_isles_runtime.dll`, installed as `rustic-isles-runtime.dll`.
4. Non-ASCII environment values are refused.
5. `MPLOG_OPEN_PER_LINE` and `PE_CATEGORY_CACHE_OFF` are dropped; the five other R0 flags kept (RT-14 for R0).
6. MIT log tags `[MitRuntime]`, `[Dispatch]`, `[Broker]`, `[Panic]`.
7. No structured exception handling: every game read is checked first (5).
8. `patch_bytes` reports `AlreadyApplied`.
9. The name self-check on object 0 fails closed.
10. The broker's stub-and-cell chain (7.2), one AGPL wrapper per target, MIT outer.
11. Feature ids are "dispatch point × slice" (7.5); mixed AGPL blocks go to one slice, listed.
12. New hook id `HL-12` for the server locale (R5).
13. Seam mismatch ends the AGPL's process with `0xE10A`.
14. The Guard role `TRANSPORT_RUNTIME_MIT` (optional, zero hash when unpinned).
15. SP-3 handlers cannot absorb.
16. **Q-2, answered 2026-11-13 by the integration pass:** the runtime loader learns the fixed name
    `rustic-isles-runtime.dll` and loads it (by full path) before its runtime choice; it is neither a runtime name for
    the choice nor an ini entry (section 9). The ini route of RT-6 is not used.

Ambiguities:

- `SP-2@R14`: the server's `ServerTryActivateAbility` routing is not named in surface 4.2; R14's spec writer confirms
  the owner.
- The AGPL's `.pdata` helper does not follow chained unwind info, though the surface says "following chained unwind info
  to the primary entry"; R0 offers both (`function_entry`, `primary_entry`) and each slice says which it uses.

Open questions for the maintainer:

- **Q-1, answered:** the RVAs are in 6.2 (from the maintainer's dump).
- **Q-2, answered:** call 16.

No open question remains.

## 13. Size

Rust, without tests: the DLL crate about 1 100 lines (lifecycle 100, log 200, flags 150, memory and pdata 250,
broker and exports 300, dispatch 200), the core crate about 700 (registry 150, offsets 150, chain model 200, rest 200).
Tests about 900. MinHook vendored, not counted.
