# The launcher injects the game's DLLs

Status: **decision proposed, design done, not implemented.** Two questions in "Blocked on" need the maintainer
before code lands.

## Decision note (for the integration pass)

**Reverses RD9.** Asked for by the maintainer on 2026-11-12: when the launcher starts the game (PLAY, and the Host
page's export and dump sessions), it loads the runtime DLL and, for an export, `CatalogExporter.dll` into the game
process itself. The game client no longer uses the `winmm.dll` runtime loader or `mystic_loader.ini`.

**Why:** one less native component on a player's PC. No proxy DLL sits in the game folder, no ini is written and
removed around a session, and no session record has to be recovered after a crash.

**What replaces "the launcher never injects":**

1. **Only the process this launcher just created.** The game is started with `CREATE_SUSPENDED`, and the DLLs go
   in through the handle that call returned. The launcher never opens another process by pid or name, and never
   injects into a game that is already running.
2. **Only DLLs the launcher chose, from absolute paths, with checked hashes.**
   - The runtime: `Rustic-Isles.dll` as installed by the launcher, and only when its SHA-256 is in the runtime
     set the player accepted (`runtime_files::is_trusted`).
   - `CatalogExporter.dll`: the host kit's build output (`platform/hostkit.rs`), with its hash logged at session start.
   - Dumper-7: the file the host picked, with its hash logged at session start. The pick is the trust decision, as
     it is today.

   Each file is opened without write or delete sharing and hashed through that handle. The handle stays open
   until `LoadLibraryW` has returned in the game, so nobody can swap the file between the check and the load.
   `LoadLibraryW` always gets the full path, so the DLL search order never comes into it.
3. **All or nothing.** If any step fails, the launcher calls `TerminateProcess` on the suspended game and reports the
   error. A game whose injection went half-way is never resumed.
4. **Leftovers are still refused.** PLAY keeps refusing while `winmm.dll` or `mystic_loader.ini` sits in `Win64`:
   an old loader there would load the runtime a second time, and the game picks it up without being asked
   because it imports `winmm`. A Host tools DLL that a crashed session left behind is refused as it is today.
5. **Realms change nothing.** Realm hosting and joining still inject nothing and write nothing beside the game.

RD9's rows that follow from this: AGENTS.md's Security bullet and `restricted/AGENTS.md`'s, DD13 (whose PLAY check
now goes through the launcher, not the loader), the migration guide's `injector.ts` row (now ported, as a
clean-room design), and `spec-catalog-exporter.md`'s "never injected".

## Design

### Mechanism: `LoadLibraryW` on a remote thread, before the main thread runs

1. `std::process::Command` with `creation_flags(CREATE_SUSPENDED)`, through tokio as today. That way `OsChild`, the
   exit watcher and the console handling don't change.
2. For each DLL in order (runtime first, then the exporter, since the exporter waits for the catalog the runtime
   builds):
   - `VirtualAllocEx` and `WriteProcessMemory` write the NUL-terminated UTF-16 path into the game.
   - `CreateRemoteThread` starts a thread at `LoadLibraryW`. Its address is the same in the game as in the
     launcher, because both are x64 and `kernel32` sits at one base per boot.
   - `WaitForSingleObject` waits up to 30 s. `GetExitCodeThread` must be non-zero (the truncated `HMODULE`), and
     zero means the load failed. Then `VirtualFreeEx`.
3. Resume the main thread. `std` doesn't hand out the thread handle, so it comes from a Toolhelp32 thread snapshot
   filtered on the child's pid: a process created suspended has exactly one thread. `OpenThread(THREAD_SUSPEND_RESUME)`,
   then `ResumeThread`.

**Timing.** The first thread to run in a new process does the loader's process start-up. Here that is the
injected thread, which runs `kernel32` and every static import's `DllMain` before it reaches `LoadLibraryW`. So
the runtime's `DllMain`, and the exporter's, run before the game's entry point and before the menu, as they did
under the loader. One difference from the loader: there, the runtime loaded from inside `winmm`'s own `DllMain`,
while the game's other imports were still starting up. Now it loads after all of them, which is if anything safer
for code that must not run under the loader lock.

**Why not an APC on the suspended main thread.** It would also run before the entry point, but it reports nothing
back: a DLL that fails to load fails silently, and the game starts without its runtime. It also runs inside the
loader's own start-up, at a point Windows doesn't document. The remote thread runs synchronously, says whether the
load worked, and needs nothing undocumented.

### Where the `unsafe` lives

No seam change is needed. The repository's pattern is item-scoped `#[allow(unsafe_code, reason = "...")]` with a
`SAFETY:` note, in a `cfg(windows)` module. That is how `crates/dainty-gateway/src/allowlist/windows.rs` does it, and
so do the launcher kit's own `platform/keystore/os.rs` and the host kit's `commands/start/control.rs`. The
injector would be `crates/dainty-launcher-kit/src/platform/inject.rs` (the decisions: which DLLs, the hash check,
the leftover refusal, all of them unit-tested on every OS) plus `platform/inject/windows.rs` (the Win32 calls, each
`unsafe` in its own allowed item).

The launcher kit's own `windows` dependency gains three features, `Win32_System_Diagnostics_Debug`,
`Win32_System_Diagnostics_ToolHelp` and `Win32_System_LibraryLoader`. The workspace's version stays as it is, so
this is not a new pin.

`LaunchPlan` gains `inject: Vec<PathBuf>`, and the `Spawner` trait lets tests check the list without starting a
process.

### Linux and Proton

Under Proton the launcher is a Linux process and the game is a Wine process, so the Win32 calls above can't be
made from the launcher. The plan is a small MIT helper, `dainty-inject.exe`: the same `inject` code built for
`x86_64-pc-windows-gnu`, which Proton or Wine runs in place of the game exe. The helper starts the game suspended
inside the same wineserver, injects, resumes and waits, passing the exit code through. Wine implements
`VirtualAllocEx`, `WriteProcessMemory` and `CreateRemoteThread` within one prefix. Until the helper ships, the
Linux build still compiles: there is no `cfg(windows)` code on that path, and PLAY under Linux needs one of the
answers below.

## Blocked on

**1. Dedicated servers still load the runtime through `winmm.dll`.** The director starts each game server, not the
launcher. `crates/rustic-isles-director/src/lib.rs` requires `winmm.dll` beside the server exe (`RUNTIME_FILES`),
and the servers run under Wine with `winmm=n,b` (`crates/dainty-hostkit/src/wine.rs`,
`docker/director-entrypoint.sh` and restricted's copy). Taking `loader_dll` out of the host kit (`paths.rs`,
`setup`) would stop those servers from loading the runtime. The options:

- **(a, recommended)** Keep the loader for servers only. The host kit still builds `winmm.dll` and copies it beside
  the server exe. It stops publishing it in the *client* runtime feed (`publish.rs`). `native/runtime-loader`
  stays.
- (b) The director injects too, through the same helper exe under Wine. Then the loader can be retired
  everywhere, but that is a server row of its own.

**2. The client runtime feed's file list is a seam.** `dainty-protocol::manifest::RUNTIME_FILE_NAMES` lists
`winmm.dll`. The content server refuses a `runtime.json` without it (`crates/dainty-content/src/input.rs`, covered
by the Node parity tests). The old Paradox launcher route `rustic-isles-auth::runtime::assert_artifact_set` requires
it in every artifact set (`PROXY_DLL`), and the director lists it. Under (a), the protocol list and the content
input rule would change to "`Rustic-Isles.dll`, and `winmm.dll` optional for one release". That is a seam change,
plus a parity re-record with a reason. In the meantime the launcher could ignore a listed `winmm.dll` and keep it out
of `Win64` without changing the contract: it would download nothing for it and leave it out of the runtime's
completeness check.

**And for Linux:** either ship `dainty-inject.exe` with the first cut, or keep `WINEDLLOVERRIDES` and the loader on
Linux players only until it ships.

## Once decided: the implementation, in order

1. `platform/inject.rs` and `inject/windows.rs` as above. `LaunchPlan.inject`. PLAY passes the runtime, and the
   export session passes the runtime and the exporter.
2. `loader_session.rs` becomes a smaller "Host tools session": no copying, no ini, no runtime set aside, no record.
   The dump session injects Dumper-7 alone, so the runtime is never loaded. `leftovers` keeps `winmm.dll`,
   `mystic_loader.ini`, and a `*.rr-dump-off` file or a session record left by an earlier launcher.
3. The launcher's runtime install skips `winmm.dll` and removes a copy it installed earlier (`sweep_aside`). PLAY's
   leftover check refuses one it did not install.
4. The host kit stops publishing `winmm.dll` to the client feed (under (a)), the release stops shipping it to
   players, and `docs/security/launcher-injection.md` is written as the RX3 checklist. The bridge spec doesn't
   change: `LoaderLeftovers` keeps its code and its wording.

## Tests

Unit tests, on every OS:

- The DLL list for PLAY, export and dump.
- A hash that doesn't match the trusted set is refused, and nothing is started.
- A relative path or a bare name is refused.
- A leftover `winmm.dll` or `mystic_loader.ini` refuses PLAY.
- A failed injection ends in a terminated, never resumed, child (with a fake `Spawner`).

## Hands-on checks (maintainer, Windows PC with the game)

1. Delete `winmm.dll` and `mystic_loader.ini` from `Win64`, then PLAY. The launcher log shows the runtime's hash
   and `injected Rustic-Isles.dll`. The game reaches the menu signed in through the local backend.
2. While the game runs, Process Explorer (or `listdlls`) on `Dauntless-Win64-Shipping.exe` lists `Rustic-Isles.dll`
   from `Win64`, and no `winmm.dll` from `Win64`.
3. Put an old `winmm.dll` back in `Win64`. PLAY refuses with the leftover message.
4. Change one byte of `Rustic-Isles.dll`. PLAY refuses before the game starts, and no game process is left behind.
5. On the Host page, run a game-data export. `CatalogExporter.dll` is listed in the game, and the export files appear.
6. Run a Dumper-7 session. The SDK is written, and `Rustic-Isles.dll` is *not* in the game's module list.
7. Under Proton (once the helper ships), repeat steps 1 and 2.
