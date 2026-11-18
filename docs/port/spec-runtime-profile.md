<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime sampling profiler (RD27)

The census (`spec-runtime-census.md`) and the stub sweeps (`spec-runtime-stub.md`) could not say where a hub with 32
bots spent its frame time. This page adds a sampling profiler of the game thread to the runtime, and records the first
thing it found: the runtime's own `ProcessEvent` dispatch spending most of a busy hub's frame in `VirtualQuery` (4). New
behaviour: nothing here is ported, and nothing was read from `restricted/`, Dauntless Revived or Mystic Paradox.

Conventions are R0's: **S** server, times in `GetTickCount64` milliseconds. **(call)** marks a decision this page
makes.

---

## 1. Scope

| What | Where |
| --- | --- |
| The schedule, the tallies and every log text (pure) | `rustic-isles-runtime-core::profile` |
| Suspending, unwinding and naming modules | `rustic-isles-runtime` `ffi::profile` |
| The sampling thread | `server_life::profile` |
| The dispatch fix (4) | `ffi::dispatch::identity`, `ffi::memory::read_bytes_this_frame` |

Off unless `RUNTIME_PROFILE.flag` is beside the executable (call 1). Servers only: it starts from R8's start step.

## 2. Sampling

- **Thread.** At R8's start step (R8 5.1) the game thread is the one SP-3 recorded (`frame_thread`). It is opened with
  suspend, get-context and query rights; a failure logs `[MitRuntime] [Profile] could not open the game thread:
  profiler off`. A thread named `rir-profiler` does the rest; it ends with the process's detach.
- **Windows.** 20 s windows, the first 90 s after the start step and then every 300 s (by start time).
- **One sample.** `SuspendThread`, `GetThreadContext` (control and integer registers, a 16-aligned record), then the
  stack is unwound in place with `RtlLookupFunctionEntry` and `RtlVirtualUnwind`, at most 48 frames; a frame without
  unwind data is a leaf whose return address is read at the stack pointer. Each frame is kept as its function's start
  (the unwind entry's `BeginAddress`), else the bare address. Then `ResumeThread`. Nothing between suspend and resume
  allocates or takes a lock the runtime owns **(call 2)**. A sample that cannot suspend or read the thread counts as
  missed.
- **After the resume.** A game-executable frame that starts a chained unwind fragment is replaced by its primary
  function (`.pdata`, R0). Between samples the thread sleeps 1 ms; the real rate is logged (about 620 a second).

## 3. Lines

At the end of each window, all through R0's writer:

```text
[MitRuntime] [Profile] on (RUNTIME_PROFILE.flag): the game thread, 20 s windows from 90 s after listen, every 300 s
[MitRuntime] [Profile] game thread: <n> samples in <s> s (<rate>/s, <m> missed), connections <c>, role <role>, bots <b>
[MitRuntime] [Profile] self <i>/<k> <module>+0x<rva>: <pct>% (<count>)      (the innermost function; top 25)
[MitRuntime] [Profile] total <i>/<k> <module>+0x<rva>: <pct>% (<count>)     (anywhere in the stack, once a sample; top 25)
[MitRuntime] [Profile] stack <i>/<k> <pct>% (<count>): <f1> < <f2> < ...    (whole stacks, innermost first, 24 frames; top 6)
```

A function is named by its module's file name and its offset there; the reader maps it with the module's exports, its
PDB, or the game's catalog. At most 20 000 distinct stacks are kept a window.

## 4. What it found: the dispatch's `VirtualQuery` (Ramsgate, 32 bots, no client, 2026-11-17)

The first window had 98% of samples in `ntdll!NtQueryVirtualMemory` under `KERNELBASE!VirtualQuery`, called straight
from the runtime's `ProcessEvent` dispatch (`ffi::dispatch::pe_dispatch`), under game callers of `ProcessEvent`. On every
call the dispatch validated its name cache with `identity(function)`, which checked the `UFunction` with `readable` and
then copied it with `read_bytes`: two `VirtualQuery` system calls per `ProcessEvent`. Thirty-two characters make
enough `ProcessEvent` calls that the queries were most of the frame.

**The fix:** `identity` reads through `read_bytes_this_frame`: on the game thread, a readability check against the
last 64 regions `VirtualQuery` returned this epoch (`readable_region_this_frame`, cleared whenever R0's frame epoch
moves), then a plain copy; off it a fresh query as before. The guarantee is R0 5.2's (memory checked readable this
epoch on the game thread stays mapped until the epoch moves); the name cache's validation is unchanged. A first
version used R0's per-pointer cache instead: 256 direct-mapped slots, which thrash once more distinct functions run
in a frame than there are slots (12% of the game thread at 256 bots, 1.7% with regions).

| Build | fps, 32 bots (60 s windows) | Profile |
| --- | --- | --- |
| Before, 32 bots | 13.0 to 14.8 | 98% `VirtualQuery` under `ProcessEvent` dispatch |
| Per-pointer cache, 32 bots | 29.5, 29.7 (the 30 fps cap), worst frame 99 ms | 92% the frame pacer's wait (idle), 2.5% `VirtualQuery` |
| Per-pointer cache, 64 bots | 29.7, worst frame 46 ms | 87% idle, 3.9% `VirtualQuery` |
| Per-pointer cache, 128 bots | 29.7, worst frame 105 ms | 82% idle, 2.1% `VirtualQuery` |
| Per-pointer cache, 256 bots | 29.7, worst frame 111 ms | 53% idle, 12.2% `VirtualQuery` (cache thrash) |
| Region cache, 256 bots | 29.7, worst frame 110 ms | 64% idle, 1.7% `VirtualQuery` |

Private commit grows about 0.35 MiB a bot (1370 MiB at 32, 1484 MiB at 256).

So an MIT-runtime hub with 32 standing player characters now has most of its game thread to spare, and the effects
and animation sweeps (`spec-runtime-stub.md` 4.3, 5) measured the runtime's cost, not the game's.

## 5. Tests

In CI: the schedule, the tallies (recursion counted once in totals, empty stacks missed), the three lists, their caps
and the labels; on Windows, a sample of another thread of the test process whose stack reaches the sampled function's
module, and module naming. By hand: the runs in 4.

## 6. Calls

1. **Opt-in by flag file**, as the census, the bots and the stubs. Suspending the game thread 600 times a second costs
   it a little; the windows are short and far apart.
2. **Unwind while suspended, allocate after.** The thread may hold the heap lock when suspended; the profiler only
   writes into a fixed array until it resumes the thread.
