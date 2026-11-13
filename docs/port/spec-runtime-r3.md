<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R3, process exit and crash handling (RD27, plan item 11)

R3 is the exit and crash slice of the MIT game runtime (`spec-runtime-surface.md` section 3.4 and section 8's R3 row,
approved 2026-11-13 with rulings RT-1 to RT-18 in `rulings-2026-11-13.md`). It owns the fourteen ids EXIT-1 to EXIT-14:
eleven export detours that log every way the process ends, one vectored exception handler that logs faults and
recovers one known engine fault, one detour on the engine's fatal-error writer, and one code patch that bounds the
engine's crash-report wait on servers.

An implementer builds from this page, `spec-runtime-r0.md`, `spec-runtime-r1.md` and the surface page alone, and
never reads `restricted/`. What crossed is facts: export names, RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7,
CL 647472), instruction bytes (each checked against the maintainer's copy of that executable on 2026-11-13), engine
names, exception codes, the flag name, and behaviour. The AGPL side stands down through seam patch 0041 alone
(section 8); R3's reserved patch number 0046 is not used.

Conventions are R0's: an RVA is `+0x0XXXXXXX` from the executable's image base; **C** client, **S** server, **B**
both. **(call)** marks a decision this page makes; **(ambiguity)** something the source did not settle;
**(departure)** a place the MIT runtime deliberately behaves differently from the AGPL one. All three are collected in
section 11.

---

## 1. Scope

| R3 owns | Role | Section |
| --- | --- | --- |
| EXIT-1 to EXIT-11: the process-ending and exception-raising exports | B | 4 |
| EXIT-12: the vectored exception handler, its fault log and the FName text-read recovery | B | 5 |
| EXIT-13: `FWindowsErrorOutputDevice::HandleError` | S | 6 |
| EXIT-14: the `ReportCrash` wait, 60 000 ms to 5 000 ms | S | 7 |
| The game thread id the exit lines report (an SP-3 tick handler) | S | 3.3 |

**Not R3, though the surface page puts them near it:**

- **The spline fault recovery.** Surface 3.4 says EXIT-12 recovers an access violation at `+0x01091D5D` inside the
  spline curve evaluator, and section 8 gives that rule to R11. The AGPL runtime **removed** that recovery on
  2026-10-08: its spline crash came from the runtime itself calling a stale 1.12 RVA, and the recovery unwound the
  stack 8 bytes short. Today's AGPL handler recovers no spline fault. So R11 has nothing to port from EXIT-12 and
  **R11 no longer depends on R3**; the spline guards are NETD-7 and NETD-8 (R11's own detours). Recorded for the R11
  spec writer (11, finding 1).
- **The client travel-crash probe.** The AGPL handler also writes, once per client process, a dump of the world a
  travel was loading when an access violation hits `+0x0285E6F0` (an engine `GetOutermost`-style helper reached from
  `UEngine::LoadMap`), followed by a world census. It only logs, so per RT-3 it is **not ported** **(call)**. It goes
  when R3 lands because it lives in the same handler. R16 (travel) may bring it back as its own fault observer if a
  live bug needs it (11, finding 2).
- `UEUnhandledExceptionFilter` stays unhooked: on 1.14.7 `+0x02657770` is a formatter with another ABI (surface
  3.4). The AGPL logs a line saying it skipped it; the MIT runtime writes nothing about it **(call)**.
- The empty-server reap, `QuitGame` and the server quit path (R8) only *call* the exports R3 traces.

---

## 2. Where it goes

### 2.1 Files

| Crate | Path | Holds |
| --- | --- | --- |
| `rustic-isles-runtime-core` | `src/exit.rs` (one module, or a folder `src/exit/` if it grows past ~400 lines) | everything pure: the export table (4.1), the exception-code filter and the shared budget (5.3), the line formatters (4.3, 5.4, 6.2, 7.2), frame text and the own-module trim (3.2), the error-text sanitiser (6.2), the FName recovery decision and its log throttle (5.2) |
| `rustic-isles-runtime` | `src/ffi/exit/` (`mod.rs`, `exports.rs`, `veh.rs`, `engine_error.rs`, `crash_wait.rs`) | the detours, the handler, the patch, the Win32 calls; under `ffi` because every line of it is FFI (R0 2, RT-1) |

`core/src/lib.rs` gains `pub mod exit;`, `ffi/mod.rs` gains `pub mod exit;`.

### 2.2 Shared lines R3 adds

| File | Edit |
| --- | --- |
| `core/src/registry.rs` | `LANDED` gains `"R3"`. The EXIT lines exist already (EXIT-1..11 and EXIT-13 `Detour`, EXIT-12 `Veh`, EXIT-14 `Patch`) |
| `core/src/offsets.rs` `TARGETS` | twelve lines (section 9): EXIT-1..11 as `FoundBy::Export(module, procedure)` with `rva: 0`, EXIT-13 as `FoundBy::RvaAndBytes` |
| `core/src/offsets.rs` `data` | `ERROR_HISTORY = 0x06E1_4AD0` (6.1) |
| `core/src/offsets.rs`, a new `pub mod exit` (or lines in `data`, the implementer's choice) | `REPORT_CRASH = 0x0265_CEC0`, `REPORT_CRASH_WAIT = 0x0265_CF15` and the two 5-byte sequences (7.1); `FNAME_TEXT_FN = 0x0266_01A0`, `FNAME_HEADER_LOAD = 0x0266_0278` and its 3 bytes (5.2) |
| `src/lifecycle.rs` | before `handlers::freeze()`: `ffi::exit::register()` (3.3). In the install list: R3's results, in the order of 3.4. Nothing else changes there |

`owned_count()` then counts 17 ids on either role (`owns` is role-blind: SP-1/2/3 plus EXIT-1..14), so the status
line reads `owns=17 ids`.

### 2.3 What is pure and what is Windows-only

Pure (core, `cargo test` on every platform): every text this page fixes, every decision (filter, budget, throttle,
recovery match, trim, sanitiser, the patch decision R0 already has), and the export table. Windows-only (DLL crate,
tested on the Windows runner where it can be): `GetProcAddress` and `GetModuleHandleExW` lookups,
`RtlCaptureStackBackTrace`, `AddVectoredExceptionHandler`, the `CONTEXT` edit, the detour bodies. Nothing in R3 runs
against the real executable in CI (surface 9).

---

## 3. Common rules for every R3 hook

### 3.1 Before anything else

Every R3 detour and the handler, in this order:

1. **Detaching** (R0 3.3): if set, forward at once (the detours call `next` with the original arguments; the handler
   returns `EXCEPTION_CONTINUE_SEARCH`). Process exit runs `RtlExitUserProcess`, then `NtTerminateProcess(NULL, …)`,
   then every DLL's detach, then `NtTerminateProcess(-1, …)`; the last call arrives after this DLL detached and must
   only forward. The AGPL runtime crashed every client exit until it learnt exactly this.
2. **Re-entry (call).** A thread-local flag, const-initialised with no destructor (so touching it never allocates or
   registers anything): when it is already set on this thread, forward as in step 1. Set it for the rest of the body,
   clear it on every exit path. It stops an exit or exception raised inside R3's own logging (the log writer, a
   module lookup) from re-entering R3. The AGPL guards only its handler this way **(departure: all R3 entry points)**.
3. Call R0's `lifecycle::write_status_once()`, so the status lines precede any exit line even when the process ends
   before the first dispatch call.
4. Do the hook's work inside `catch_unwind`; a panic logs `[Panic] <id> ...` once (R0 2) and the hook then forwards
   exactly as in step 1.

R3 is never the place that ends the process: every detour calls its `next` with the arguments it was given, and the
handler never answers anything but `EXCEPTION_CONTINUE_SEARCH`, except for the one recovery of 5.2.

**Writing from a `DllMain` (call).** An exit export can be called while the loader lock is held: the AGPL runtime
ends its process from its own `DllMain` when the seam does not match (`0xE10A`), and RtlExitUserProcess runs the
detaches under it. R3 writes its exit lines anyway (R0's writer is ready by then): the writes are `CreateFileW`,
`WriteFile` and `CloseHandle`, which load no library, and the process is ending. This is the one exception to
R0's "nothing under the loader lock"; R3 takes no other lock than R0's log lock and its own budget atomics.

### 3.2 Frames, addresses and the stack

- **Address text** `addr(p)`:
  - `null` for 0;
  - else, when `GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT)` finds the module holding `p`:
    `<module file name>+0x<HEX offset>`, the file name being the text after the last `\` or `/` of
    `GetModuleFileNameW` (UTF-16 to UTF-8, lossy);
  - else, when `base <= p < base + 0x0800_0000` (the executable's image base): `Dauntless-Win64-Shipping.exe+0x<HEX>`;
  - else `0x<HEX>`.

  `HEX` is upper-case hexadecimal without padding, as in every hex value on this page (`0x0`, `0x2659A20`).
- **The stack.** `RtlCaptureStackBackTrace(0, 48, frames, NULL)`, then drop every leading frame whose address lies
  inside this DLL's own image (from `GetModuleHandleExW` on an address in it), then keep at most **32**. **(call)** The
  AGPL skips a fixed count of its own frames; the MIT build trims by module, which survives inlining.
- **The caller** (`retAddr`): the first frame after the trim, or `null` when there is none. **(call)** Rust has no
  return-address intrinsic; the first foreign frame is the same address on every path because the broker's entry stub
  is a jump and adds no frame.
- **The game thread** (`gameTid`): 3.3. `isGameThread` is `1` iff `gameTid != 0` and the calling thread's id equals
  it.

The core crate owns the text (`frame_text(module: Option<&str>, offset, raw, in_exe_range)`), the trim (over a list
of addresses and the own image's range) and the line builders. The DLL crate feeds them.

### 3.3 The game thread id (S)

R3 registers one SP-3 tick handler (`handlers::register_tick`, `slice: "R3"`, `order: i32::MIN`, `before` only) that
stores the calling thread's id into a process-wide `AtomicU32` once (compare-exchange from 0). It has no AGPL
counterpart and no feature id: the AGPL runtime records its own copy in its own tick, which keeps running and which
nothing of R3 reads. On a client nothing records it, so `gameTid=0` and `isGameThread=0`, as in the AGPL runtime
**(ambiguity: a client game thread id would be useful; nothing in the source gives one, and SP-1 runs on more than
one thread)**.

### 3.4 Install order (RT-17 for R3)

**Settled:** no R3 id depends on another id's install order, nor on any id of another slice. The AGPL runtime installs
them at these points of surface 3.11, and the MIT build keeps the positions so its install lines read in the same
order:

- **Client:** SP-1, then EXIT-1 to EXIT-11 in the order of 4.1, then EXIT-12.
- **Server:** EXIT-1 to EXIT-11, EXIT-12, then SP-2, SP-3, then EXIT-13, then EXIT-14.

EXIT-12 and EXIT-14 use no MinHook, so they install even when `MH_Initialize` failed **(call)**; EXIT-1..11 and EXIT-13
then report the MinHook status, as R0's do. Each id gives one install line through R0's status
(`[MitRuntime] install EXIT-<n> ok|<reason>`). Reasons R3 adds: `module not loaded`, `export not found`,
`prologue mismatch`, `not in ReportCrash`, `flag DISABLE_CRASH_WAIT_SHORTEN`, and the patch results of 7.2.

---

## 4. EXIT-1 to EXIT-11: the exports (B)

### 4.1 Targets

Found with `GetModuleHandleW(module)` (never loaded: R0 3.1) and `GetProcAddress`. A module that is not loaded or a
missing export leaves that id off with its reason; it is not retried later. All are hooked through the broker
(`broker::hook`, band `Mit`, `order: 0`).

| Id | Module | Export | Signature (x64) | Tag in the lines |
| --- | --- | --- | --- | --- |
| EXIT-1 | `kernel32.dll` | `ExitProcess` | `(u32 code)`, no return | `kernel32!ExitProcess` |
| EXIT-2 | `ntdll.dll` | `RtlExitUserProcess` | `(u32 status)`, no return | `ntdll!RtlExitUserProcess` |
| EXIT-3 | `ntdll.dll` | `NtTerminateProcess` | `(HANDLE, i32 status) -> i32` | `ntdll!NtTerminateProcess` |
| EXIT-4 | `kernelbase.dll` | `TerminateProcess` | `(HANDLE, u32 code) -> i32` | `kernelbase!TerminateProcess` |
| EXIT-5 | `kernelbase.dll` | `RaiseException` | `(u32 code, u32 flags, u32 count, *const usize args)` | `kernelbase!RaiseException` |
| EXIT-6 | `ucrtbase.dll` | `exit` | `(i32 code)`, no return | `ucrtbase!exit` |
| EXIT-7 | `ucrtbase.dll` | `_exit` | `(i32 code)`, no return | `ucrtbase!_exit` |
| EXIT-8 | `ucrtbase.dll` | `abort` | `()`, no return | `ucrtbase!abort` |
| EXIT-9 | `msvcrt.dll` | `exit` | `(i32 code)`, no return | `msvcrt!exit` |
| EXIT-10 | `msvcrt.dll` | `_exit` | `(i32 code)`, no return | `msvcrt!_exit` |
| EXIT-11 | `msvcrt.dll` | `abort` | `()`, no return | `msvcrt!abort` |

`kernel32!TerminateProcess` forwards to `kernelbase`, so EXIT-4 catches both. A detour of a no-return function is
declared returning `()` and simply never sees its `next` return. The `-> !` form is not used, so a `next` that does
return (it never should) still leaves the detour well-formed.

### 4.2 Behaviour

After 3.1: write the entry line (4.3), then the stack line, then call `next` with the original arguments and return
what it returns. Every call is logged: there is no budget on exits (there are a handful per process).

EXIT-5 is different: it is the hottest of the eleven, because the C++ `throw` and the debugger-print APIs raise
through it. Its body first applies the **filter** of 5.3 to `code`; a filtered code forwards at once, before step 3 of
3.1 and before any formatting. A traced code takes one unit of the shared **budget** (5.3); past the budget it
forwards silently. Otherwise it writes the exception line of 5.4 built from its arguments (the address is `retAddr`,
the parameters are `args[0..min(count, 15)]`, flags as given, no registers), then the stack line, then forwards.

### 4.3 Lines **(departure: new tags, R0 call 6)**

`<tail>` is ` tid=<n> gameTid=<n> isGameThread=0|1 retAddr=<addr> retModule=<module>`, where `retModule` is the
module file name alone from `addr(retAddr)` (`?` when no module holds it, `null` for none). Numbers are decimal
unless shown as hex.

| Ids | Entry line |
| --- | --- |
| EXIT-1, 2, 6, 7, 9, 10 | `[MitExit] <tag> code=<code as decimal; i32 for 6, 7, 9, 10>` + `<tail>` |
| EXIT-3 | `[MitExit] ntdll!NtTerminateProcess handle=<handle> status=0x<HEX of the u32 bits>` + `<tail>` |
| EXIT-4 | `[MitExit] kernelbase!TerminateProcess handle=<handle> code=<decimal>` + `<tail>` |
| EXIT-8, 11 | `[MitExit] <tag>` + `<tail>` |

`<handle>` is `null` for 0, `self` for the current-process pseudo handle (`-1`), else `0x<HEX>`. **(departure)**
The AGPL prints handles as unsigned decimal.

**Stack line**, every id: `[MitExit] <tag> code=<n> stack=<addr> <addr> ...` (frames separated by one space, none
when the trim left nothing). `<n>` is the code as an unsigned decimal: EXIT-3's status bits, EXIT-5's exception code,
`3` for both `abort`s (the exit code `abort` ends with).

---

## 5. EXIT-12: the vectored exception handler (B)

### 5.1 Registration

`AddVectoredExceptionHandler(1, handler)` once, in Init (both roles; 3.4). First-in-list: since this DLL loads before
`Rustic-Isles.dll`, any handler the AGPL adds later with `1` (its R14 controller watchpoint, opt-in) runs before
R3's, which is harmless because that one answers only its own single-step traps. The handle is kept, never removed
(the DLL is pinned). Install result `ok`, or `AddVectoredExceptionHandler failed` when it returns null.

On each exception, after 3.1 steps 1 and 2 (the handler skips 3's status write until it knows it will log, so a
filtered first-chance exception costs two atomic loads):

1. If `ExceptionInfo`, its record or its context is not readable (R0's `readable`, the record for `0x98` bytes and the
   context for `0x4D0`), go to 4 with whatever is readable.
2. The FName recovery (5.2): when it matches, recover and return `EXCEPTION_CONTINUE_EXECUTION`.
3. The filter and budget (5.3): a filtered code or an exhausted budget returns `EXCEPTION_CONTINUE_SEARCH` silently.
4. Write the exception line (5.4) and the stack line (`[MitExit] VEH code=<code decimal> stack=...`, 3.2; the walk
   from inside a vectored handler passes through the dispatcher's frame into the faulting code, which is what it is
   for), then return `EXCEPTION_CONTINUE_SEARCH`.

Every exception reaches the handler first-chance, including those the game, the AGPL runtime or R0's checked reads
(which use `VirtualQuery`, not faults) would handle themselves. That is today's behaviour and why the budget exists.

### 5.2 The FName text-read recovery (the one fault R3 recovers)

**The fault.** On both roles the engine occasionally formats an `FName` whose name-pool entry pointer is wild. The
function at `+0x026601A0` (its `.pdata` entry runs to `+0x0266034A`) computes the entry address with
`8D 3C 00` (`lea edi,[rax+rax]`) then `49 03 7C C8 10` (`add rdi,[r8+rcx*8+0x10]`: the pool's block table at `+0x10`,
as in R0 6.2), and at **`+0x02660278`** loads the entry's 16-bit header with **`0F B7 07`** (`movzx eax, word ptr
[rdi]`), followed by `C1 E8 06` (`shr eax, 6`: the 10-bit length). With a bad block the load faults.

**Match:** the exception code is `0xC0000005` (access violation), the context is readable, and `Rip` equals
`image base + 0x02660278`.

**Recovery:** set the context's `Rdi` to the address of a static, zero-filled, 8-byte-aligned buffer of **32 bytes**
owned by this DLL, and return `EXCEPTION_CONTINUE_EXECUTION`. The `movzx` runs again and reads a header of 0: an empty,
non-wide name of length 0, so the engine formats an empty name and goes on. The buffer is never written.

**Arming check (call).** At install, R3 reads the 3 bytes at `+0x02660278` (R0's `read_unaligned` over the code) and
arms the recovery only when they are `0F B7 07` and `function_entry(0x02660278)` is `0x026601A0`. Otherwise the
handler still logs but never recovers, and Init defers the line
`[MitException] FName recovery not armed: +0x2660278 is not the expected header load`. The AGPL checks neither; on any
other executable a context rewrite at a wrong address would corrupt the thread.

**Line:** `[MitException] recovered invalid FName text read at +0x2660278 rdi=0x<HEX old Rdi> access=<0x<HEX> of the
record's second parameter, or (unknown) when it has fewer than 2>`. **(departure)** The AGPL writes one line per
occurrence; the MIT build writes the first **64**, then one line every **1 024** occurrences with ` (occurrence <n>)`
appended, so a fault that recurs every frame cannot fill the disk. The recovery itself happens every time and
spends nothing of 5.3's budget.

The decision ("is this the fault?") and the throttle ("write this occurrence?") are pure (core): a function of the
code, the `Rip`, the image base and the armed state, and a function of the occurrence number.

### 5.3 The filter and the shared budget

**Filtered codes** (never logged, by EXIT-5 nor EXIT-12): `0x40010005` (`DBG_CONTROL_C`), `0x40010006`
(`DBG_PRINTEXCEPTION_C`), `0x40010007` (`DBG_RIPEXCEPTION`), `0x40010008` (`DBG_CONTROL_BREAK`), `0x4001000A`
(`DBG_PRINTEXCEPTION_WIDE_C`), `0x406D1388` (the MSVC thread-name exception), `0xE06D7363` (the MSVC C++ exception;
Rust panics on this target raise it too, so the MIT runtime's own caught panics never reach the log here).

**Budget:** one process-wide counter shared by EXIT-5 and EXIT-12, incremented once per traced (unfiltered) exception
before logging. While it is at most **96** the exception is logged; from the 97th on, nothing is. It never resets.
The FName recovery (5.2) does not count.

### 5.4 The exception line

`[MitException] <tag> code=0x<HEX> flags=0x<HEX> address=<addr(exception address)>`, then:

- ` rva=+0x<HEX>` when the exception address lies in `[image base, image base + 0x0800_0000)`;
- ` params=0x<HEX>,0x<HEX>,...` when the record has parameters (at most 15, `EXCEPTION_MAXIMUM_PARAMETERS`);
- the handler only, when the context is readable: ` rip=<addr(Rip)> rsp=0x<HEX> regs{rax=0x<HEX> rcx=0x<HEX>
  rdx=0x<HEX> rbx=0x<HEX> rsi=0x<HEX> rdi=0x<HEX> r8=0x<HEX> r9=0x<HEX>}`.

`<tag>` is `VEH` for the handler and `kernelbase!RaiseException` for EXIT-5.

**Stack overflow (call).** For `0xC00000FD` the handler writes the exception line without `regs{…}` and skips the stack
walk and the stack line: the faulting thread is out of stack, and the handler runs on the same stack. Formatting uses
a fixed 1 KiB stack buffer for this one case. **(departure)**

---

## 6. EXIT-13: `FWindowsErrorOutputDevice::HandleError` (S)

### 6.1 Target and the error history

- **Target:** `+0x02659A20`, signature `(this: *mut c_void)`, no return value. **Found by (call):** RVA and bytes:
  `function_entry` must be `0x02659A20` (its `.pdata` entry runs to `+0x02659BC7`) and the first 10 bytes must be
  `48 89 5C 24 08 57 48 83 EC 30`; otherwise no hook (`prologue mismatch`). The AGPL hooks the bare RVA.
- **What the original does** (engine behaviour, kept): sets `GIsCriticalError`, writes `=== Critical error: ===` and the
  error text into the engine's error history (`GErrorHist`), flushes the error device and lets the engine request
  exit. On this build `ReportCrash` (7) has already run when a crash gets here.
- **The error history:** a UTF-16 character buffer (not a pointer) in `.data` at **`+0x06E14AD0`**.

### 6.2 Behaviour

After 3.1: read the buffer one UTF-16 unit at a time, at most **1 023** units, stopping at the first 0. Each unit is
read through R0's checked reads (`read_unaligned::<u16>`); the first unreadable unit stops the read and appends
` <unreadable/truncated>`. Write

`[MitEngineError] fatal error (native handler preserved) msg=<text>`

then **always** call `next` with `this`. An earlier AGPL version logged and returned without calling the original, and
the engine ran on in a corrupted state and died later of something unrelated; so this hook never absorbs.

**The text (call, departure).** UTF-16 to UTF-8, unpaired surrogates as U+FFFD; CR dropped; LF written as the two
characters `\n`; any other character below U+0020 as `?`. The log stays one line per entry. The AGPL truncates each
unit to a byte and writes the line breaks raw.

---

## 7. EXIT-14: the `ReportCrash` wait (S)

### 7.1 The patch

After a crash on the game thread, the engine's `ReportCrash` (`+0x0265CEC0`, `.pdata` entry to `+0x0265CF2A`) hands
the exception to the crash-reporter thread and waits for it with `WaitForSingleObject(<crash handled event>, 60000)`.
On an 8 October 2026 hunt the reporter never finished, and every connected client saw a frozen world for 60 s before
the server printed `Fatal error!` and exited with code 3. The 60 000 is one instruction:

| RVA | Expected | Replacement |
| --- | --- | --- |
| `+0x0265CF15` | `BA 60 EA 00 00` (`mov edx, 60000`) | `BA 88 13 00 00` (`mov edx, 5000`) |

The bytes that follow are `FF 15 …` (the indirect call to `WaitForSingleObject`). Healthy reporters finished in under
0.8 s, so 5 s keeps the engine's crash folder and minidump wherever they work and lets a stalled reporter hold clients
for 5 s at most.

**Known cost (engine behaviour, recorded):** when the wait expires, `ReportCrash` returns `EXCEPTION_EXECUTE_HANDLER`
and the handler unwinds over the stack the reporter is still reading, so a reporter that is still writing
`UE4Minidump.dmp` after 5 s may leave a truncated dump or one with a wrong exception context.

### 7.2 Behaviour

Server only, in Init (3.4), once:

1. `DISABLE_CRASH_WAIT_SHORTEN.flag` present (R0's `flag`, read once here): leave the code alone. Deferred line
   `[MitCrashWait] DISABLE_CRASH_WAIT_SHORTEN.flag present: native 60000 ms crash wait kept`; install reason
   `flag DISABLE_CRASH_WAIT_SHORTEN`.
2. `function_entry(0x0265CF15)` is not `0x0265CEC0`: leave it. Deferred line
   `[MitCrashWait] +0x265CF15 is not inside ReportCrash (+0x265CEC0) on this exe: not patched`; reason
   `not in ReportCrash`.
3. Else R0's `patch_bytes(0x0265CF15, expected, replacement)` and the deferred line
   `[MitCrashWait] ReportCrash wait at +0x265CF15 60000->5000 ms: <result>`, `<result>` being `patched` (`Applied`),
   `already patched` (`AlreadyApplied`), or `NOT patched (bytes differ)`, `NOT patched (unreadable)`,
   `NOT patched (protection failed)`. Install line `ok` for the first two, the result text otherwise.

"Deferred" is R0's `log::defer`: Init runs under the loader lock, so these lines come out right after the status.

No hook, nothing at run time. The flag is the only switch (RT-14 for R3, 8.2).

---

## 8. The AGPL side and the other slices

### 8.1 How the AGPL stands down (seam patch 0041, no new patch)

When `owns` answers yes for these ids (that is, once `"R3"` is in `LANDED`), the AGPL runtime, through 0041 alone:

- **EXIT-1 to EXIT-11:** its 22 export hook creations (11 per role) map by module and export name to these ids, so it
  creates no wrapper; each original pointer it keeps is `rir_original(target)`, its enable calls succeed as no-ops.
  Its own exit detours never run.
- **EXIT-12:** it skips both of its `AddVectoredExceptionHandler` registrations (client and server).
- **EXIT-13:** the RVA maps to the id; no wrapper.
- **EXIT-14:** it skips the patch, its flag read and its lines.

Nothing in the AGPL reads state these ids set: its record of the game thread (its own tick, not gated), its exception
budget and its handler handle are used only by the code that stands down. So R3 adds **no row** to R0 7.5's
cross-slice table, needs **no ordering rule**, and **patch 0046 is not written** (the number stays reserved and
unused).

What the operator still sees from the AGPL after R3 lands **(call: accepted, no patch)**: one
`[ExitTrace] hook <tag> create=MH_OK enable=MH_OK target=<decimal>` line per export, because its install helper logs
the status 0041 returned, and its `[ExitTrace] UEUnhandledExceptionFilter hook skipped` line. Both are install-time
noise; its seam status line counts the eleven (plus EXIT-13) among the `hooks left to it`. The MIT lines use their own
tags (`[MitExit]`, `[MitException]`, `[MitEngineError]`, `[MitCrashWait]`), so a reader can tell whose line it is.

### 8.2 Rulings settled here

- **RT-14 for R3 (flags).** `DISABLE_CRASH_WAIT_SHORTEN` is kept with its meaning (7.2). R3 reads no other flag.
- **RT-17 for R3 (install order).** No dependency (3.4).
- **RT-18 for R3 (name matching).** R3 matches no UFunction names; its one dispatch registration (3.3) is a tick
  handler.
- RT-12, RT-13, RT-15 and RT-16 are not R3's.

### 8.3 Other slices

| Slice | What it gets from R3 |
| --- | --- |
| R11 | nothing to port from EXIT-12 (1); its dependency on R3 can go |
| R16 | the travel-crash probe is gone with the AGPL handler (1); R16's spec decides whether a live bug needs it back |
| R8 | its reap and quit paths call `TerminateProcess` and `ExitProcess`; with R3 landed those calls are logged by `[MitExit]` lines, before R8 lands too |
| any slice | nothing calls into R3; no interface is exported |

---

## 9. Offset-table lines (R3)

```text
EXIT-1   Export("kernel32.dll", "ExitProcess")
EXIT-2   Export("ntdll.dll", "RtlExitUserProcess")
EXIT-3   Export("ntdll.dll", "NtTerminateProcess")
EXIT-4   Export("kernelbase.dll", "TerminateProcess")
EXIT-5   Export("kernelbase.dll", "RaiseException")
EXIT-6   Export("ucrtbase.dll", "exit")
EXIT-7   Export("ucrtbase.dll", "_exit")
EXIT-8   Export("ucrtbase.dll", "abort")
EXIT-9   Export("msvcrt.dll", "exit")
EXIT-10  Export("msvcrt.dll", "_exit")
EXIT-11  Export("msvcrt.dll", "abort")
EXIT-13  0x02659A20  RvaAndBytes(48 89 5C 24 08 57 48 83 EC 30)  FWindowsErrorOutputDevice::HandleError
data     ERROR_HISTORY      0x06E14AD0   GErrorHist, UTF-16 buffer
exit     REPORT_CRASH       0x0265CEC0   ReportCrash (.pdata begin)
exit     REPORT_CRASH_WAIT  0x0265CF15   BA 60 EA 00 00 -> BA 88 13 00 00
exit     FNAME_TEXT_FN      0x026601A0   the FName-to-text routine (.pdata begin)
exit     FNAME_HEADER_LOAD  0x02660278   0F B7 07
```

R0's offset test (every detour id of a landed slice has one `TARGETS` line) then covers EXIT-1..11 and EXIT-13; the
`Export` lines carry `rva: 0` and are exempt from the RVA-uniqueness rule (R0 6.1), which the implementer confirms
the existing test already allows (it checks RVAs of `Rva`/`RvaAndBytes` lines).

---

## 10. Tests

### 10.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`, every platform):

- The export table: eleven rows, ids EXIT-1..11 in order, each `(module, export, tag)` as 4.1; each has a `TARGETS`
  line with the same `Export` pair.
- Address text: null; a module and offset; the executable fallback at the range's first and last byte and one past
  it; raw hex. Upper-case, no padding.
- Trim: own frames first, then foreign ones (kept, at most 32 of 48); all own (empty); none own.
- Entry lines for each of the eleven tags, with fixed numbers: `code` as signed decimal for EXIT-6/7/9/10 (`-1` stays
  `-1`), NtTerminateProcess's status as hex bits, the three handle forms, both `abort`s with no code and stack code 3.
- The filter: each of the seven codes filtered; `0xC0000005`, `0x80000003`, `0xC00000FD` traced.
- The budget: the 96th traced exception logs, the 97th does not; filtered codes and recoveries do not count.
- The exception line: with and without `rva`, with 0, 1 and 15 parameters (and a record claiming 20), with and without
  registers; the stack-overflow form.
- The FName decision: matches only for `0xC0000005` at exactly base + `0x02660278` while armed; not for another code,
  `Rip` one byte off, or unarmed. The arming check on a buffer: `0F B7 07` with the right entry arms; other bytes or
  another entry do not. The throttle: occurrences 1..64 written, 65 not, 1 024 and 2 048 written with the count.
- The error text: plain ASCII; CR LF becomes `\n`; a tab becomes `?`; an unpaired surrogate becomes U+FFFD; 1 023 units
  stop the read; the `<unreadable/truncated>` suffix.
- The crash-wait lines for each `PatchResult`, the flag and the wrong-function case; the patch decision over a buffer
  holding the expected, the replacement and other bytes (R0's `decide`).
- The registry with `LANDED = ["R0", "R1", "R3"]`: `owns` true for EXIT-1..14 and SP-1/2/3, false for every other id;
  `owned_count() == 17`.

DLL (`cargo test -p rustic-isles-runtime`, Windows runner):

- **An export detour on a stand-in.** Through the broker, hook a test function with EXIT-4's detour shape
  (`(HANDLE, u32) -> i32`) and check: it forwards the arguments and the return value; it writes nothing and
  forwards when the detaching flag is set; a nested call from inside the body forwards without logging (re-entry).
  Never hook the real `TerminateProcess` or `ExitProcess` in the test process.
- **EXIT-5 on the real export, in a child process (call).** The test re-runs its own binary as a child (an
  environment switch selects the child's path), so the real `RaiseException` is hooked only there. The child installs
  EXIT-5 alone, adds a vectored handler of its own that answers `EXCEPTION_CONTINUE_EXECUTION` for `0xE0000001`, raises
  that code once (continuable, two parameters) and once `0xE06D7363`, and exits. The parent checks the child's log
  holds exactly one exception line (with both parameters) and one stack line. If the runner cannot host this, it is a
  play-test item instead (11.1 step 6).
- **The handler and the recovery on a test page.** Allocate an executable page holding `0F B7 07 C3` (`movzx eax,
  word ptr [rdi]; ret`), point the recovery's match at it (the match takes the address as input, so the test passes
  the page's address instead of the image's), call it with `rdi` = an unmapped address, and check it returns 0 and the
  recovery line is written; with the recovery unarmed the access violation reaches the test's own outer handler.
- The `CONTEXT` sizes and offsets the handler reads (`Rip`, `Rsp`, `Rax`..`R9`, `Rdi`) against `windows-sys`'s
  `CONTEXT` (`size_of` and field offsets), so a binding change cannot shift them.
- `patch_bytes` with EXIT-14's bytes on a test page (`Applied`, then `AlreadyApplied`).

**Never in CI:** the real exports of the test process, a real crash, the engine's crash path, the executable's bytes.

### 10.2 What CI cannot show

That the eleven exports are the only ways the game ends (it is how today's runtime finds them), that the FName fault
is recovered in the real engine, that `HandleError` sees the fatal text, and that 5 s is short enough. Those are
11.1.

---

## 11. The maintainer's play-test (surface section 8, R3 row), calls and ambiguities

### 11.1 Play-test

"Client quits cleanly; a forced server crash logs and exits within 5 s, clients not frozen." Collected for the one
hands-on pass at the end (rulings, "Order, amended"); nothing waits on it.

Set-up: both DLLs injected, the MIT build with `"R3"` in `LANDED` (R0 11.1 step 1).

1. **Status.** In each log, `[MitRuntime] rustic-isles-runtime ... owns=17 ids`, `install EXIT-1 ok` to `install
   EXIT-12 ok` (an `ucrtbase` or `msvcrt` id may read `module not loaded` on a box where the game never loads that
   CRT: note which). Servers also `install EXIT-13 ok` and `install EXIT-14 ok` with
   `[MitCrashWait] ReportCrash wait at +0x265CF15 60000->5000 ms: patched`. No `[MitException] FName recovery not
   armed` line. The AGPL's seam line counts EXIT-1..11 and EXIT-13 among the hooks left to it, and the AGPL writes no
   `[ExceptionTrace] AddVectoredExceptionHandler` line.
2. **Client quit.** Log in, enter Ramsgate, quit from the menu. The log ends with `[MitExit] kernel32!ExitProcess` or
   `ntdll!RtlExitUserProcess` (whichever the game uses) and `ntdll!NtTerminateProcess handle=null` lines with their
   stacks, and nothing after them. The game exits with code 0 and no crash dialog; Windows' Event Viewer shows no
   Application Error for `Dauntless-Win64-Shipping.exe`. Repeat once from a hunt.
3. **Forced server crash.** Start a hub (or a hunt) through the Director and join it from a client. Find the server's
   pid (Task Manager, or the Director's log). With the Debugging Tools for Windows' `cdb`:

   ```powershell
   cdb -p <pid>
   ```

   then at its prompt `~0s` (the engine's main thread is the game thread), `r rip=0`, and `.detach`. The thread
   resumes at address 0 and faults, as a crash in the game would.

   Expected in the server's log, in order: a `[MitException] VEH code=0xC0000005 ... rip=null ...` line and its stack
   line; within **5 s**, a `[MitEngineError] fatal error (native handler preserved) msg=...` line (the engine's
   `Fatal error!` text, one line, `\n` for its breaks); then `[MitExit]` lines ending the process (exit code 3). The
   client sees the server go, then its own disconnect message, after at most about 5 s of frozen world, not 60 s.
   Note the times from the log's timestamps.
4. **The flag.** Create `DISABLE_CRASH_WAIT_SHORTEN.flag` next to the server executable, start a hub, check
   `[MitCrashWait] DISABLE_CRASH_WAIT_SHORTEN.flag present: native 60000 ms crash wait kept`, then delete the flag. (Do
   not repeat step 3 with it; the 60 s freeze is known.)
5. **FName recovery.** Nothing to trigger. Over the whole pass, search every log for
   `[MitException] recovered invalid FName text read`; each hit is a fault that would have crashed. Note them.
6. **Exceptions in the log.** Over the pass, at most 96 `[MitException]` lines (VEH plus RaiseException) per process,
   and none of the filtered codes.
7. Only CI passed until these steps are done.

### 11.2 Calls, ambiguities and departures

Calls made here:

1. The client travel-crash probe in the AGPL handler is not ported (RT-3); R16 may bring it back.
2. The AGPL's "UEUnhandledExceptionFilter hook skipped" line is not ported.
3. Every R3 entry point has a per-thread re-entry guard (3.1).
4. Exit lines are written even from a `DllMain` (3.1).
5. The stack is trimmed by module, and the caller is the first frame outside this DLL (3.2).
6. EXIT-12 and EXIT-14 install even when MinHook failed (3.4).
7. The FName recovery arms only after checking the instruction bytes and its function's entry (5.2).
8. EXIT-13 is found by RVA and prologue bytes (6.1).
9. No patch 0046: 0041 suffices; the AGPL's install-time `[ExitTrace] hook` lines stay (8.1).
10. EXIT-5's line carries `retAddr` as the exception address, as the AGPL builds its record.
11. The EXIT-5 child-process test is optional on the runner and falls back to the play-test (10.1).

Departures from the AGPL behaviour:

1. Tags `[MitExit]`, `[MitException]`, `[MitEngineError]`, `[MitCrashWait]` in place of `[ExitTrace]`,
   `[ExceptionTrace]` and the AGPL's error and crash-wait tags (R0 call 6).
   Nothing in the repository parses the old ones.
2. Handles print as `null`, `self` or hex, not as unsigned decimal.
3. The FName recovery line is throttled (64, then every 1 024th).
4. A stack overflow is logged without registers or a stack walk.
5. The engine error text is sanitised to one UTF-8 line.

Ambiguities:

- A client has no game thread id (3.3).
- Whether `ucrtbase.dll` and `msvcrt.dll` are always loaded by the time the DLL's Init runs on every path (injection,
  and Proton's loader, whose `DllMain` runs during static import loading). The AGPL has the same exposure and never
  retries; 11.1 step 1 records what each path shows.

Findings for other slices (also left in the session notes):

1. **R11:** the spline recovery the surface page credits to EXIT-12 was removed from the AGPL on 2026-10-08; R11
   ports none from EXIT-12 and no longer depends on R3. The AGPL's comment beside NETD-8 still claims the handler
   recovers the `+0x01091D5D` fault; that comment is stale.
2. **R16:** the client travel-crash probe (`+0x0285E6F0`) leaves with the AGPL handler when R3 lands.

No open question for the maintainer.

---

## 12. Size

Rust, without tests: the core module about 350 lines (export table 40, filter and budget 40, text and trim 120,
lines 100, FName decision and throttle 30, error text 20), the DLL module about 450 (eleven detours through one
generic body per signature 150, the handler 120, HandleError 60, the crash wait 40, install and register 80). Shared
edits: about 30 lines (`LANDED`, the offset lines, `lifecycle.rs`). Tests about 600. The surface page's estimate was
800 for the C++ it replaces.
