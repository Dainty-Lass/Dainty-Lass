# The launcher injects the game's DLLs

Status: **implemented on Windows, not yet tried with the game.** The security checklist is
[security/launcher-injection.md](security/launcher-injection.md).

## Decision note (for the integration pass)

**Amends RD9.** Asked for by the maintainer on 2026-11-12. When the Windows launcher starts the game (PLAY, and
the Host page's export and dump sessions), it loads the runtime DLL itself and, for an export, `CatalogExporter.dll`.
On Windows the game client no longer uses the `winmm.dll` runtime loader or `mystic_loader.ini`.

**Why:** one less native component on a player's PC. No proxy DLL sits in the game folder, and no ini is written
and removed around a session.

**The maintainer's answers (2026-11-12):**

1. **Dedicated servers keep the loader.** The director still starts its game servers with `winmm.dll` beside the
   server exe and `winmm=n,b` under Wine. The host kit still builds the loader and copies it there, and
   `native/runtime-loader` stays. The host kit also keeps publishing `winmm.dll` in the client feed for Proton
   players (maintainer, 2026-11-12), and Windows launchers skip it there.
2. **The feed's file list changes (seam, approved).** `dainty-protocol`'s manifest needs only `Rustic-Isles.dll`
   (`REQUIRED_RUNTIME_FILES`), and `winmm.dll` (`LOADER_DLL`) is optional for one release. The content server's
   `runtime.json` rule and the launcher's manifest parser follow it.
3. **Linux/Proton players keep the loader** (`WINEDLLOVERRIDES`, `winmm.dll`, and `mystic_loader.ini` for the Host
   sessions) until a `dainty-inject.exe` helper ships. That helper is not in this cut.

**What replaces "the launcher never injects":**

1. **Only the process this launcher just created.** The game starts with `CREATE_SUSPENDED`, and the DLLs go in
   through the handle that start returned. The launcher never opens another process by pid or name.
2. **Only DLLs the launcher chose, by absolute path, with checked hashes.** The runtime is checked against the
   offered set the player trusted. CatalogExporter and Dumper-7 are hashed as found, and the hash is logged; the
   host's pick is the trust decision. Each file is opened without write or delete sharing and hashed through
   that handle, which stays open until `LoadLibraryW` has returned.
3. **All or nothing.** If anything fails, the suspended game is killed. It is never resumed half-loaded.
4. **Leftovers are still refused.** On Windows a `winmm.dll` in `Win64` refuses PLAY, the Host sessions and
   realms, since it would load the runtime a second time. A loader the launcher installed earlier, shown by the
   runtime marker's hash, is removed instead. `mystic_loader.ini` and Host tools DLLs left by a session are refused
   as before.
5. **Realms change nothing.** Realm hosting and joining still inject nothing.

The rows that cite RD9 and need the integration pass to reword them:
- AGENTS.md's Security bullet, and `restricted/AGENTS.md`'s.
- DD13.
- The migration guide's `injector.ts` row, now ported clean-room as `platform/inject.rs`.
- `spec-catalog-exporter.md`, which says "never injected".
- `docs/security/catalog-exporter.md`'s "Who runs it".

## Design, as built

**Mechanism.** `LoadLibraryW` runs on a remote thread while the main thread is still suspended.
`platform/launch.rs`'s `OsSpawner` adds `CREATE_SUSPENDED` when `LaunchPlan.inject` is not empty. Then
`platform/inject.rs`:

1. Opens and hashes every DLL, holding each one.
2. Takes the process's only thread id, which is the main thread.
3. For each DLL in order (the runtime first, then the exporter): `VirtualAllocEx` and `WriteProcessMemory` write the
   path, `CreateRemoteThread` starts at `LoadLibraryW`, and the launcher waits up to 60 s. `LoadLibraryW` has the
   same address in the game: both are x64, and `kernel32` sits at one base per boot. A zero exit code is checked
   against the module list, since the truncated `HMODULE` can read as zero.
4. Resumes the main thread.

The first thread to run in a suspended process does the loader's start-up. So the game's imports are ready before
the runtime's `DllMain`, and both run before the game's entry point, as under the loader. An APC on the main thread
was not chosen: it reports no failure, and it runs inside the loader's own start-up at a point Windows doesn't
document.

**Crates.** The maintainer asked for existing crates where available. `dll-syringe` 0.18 handles suspended
processes but needs nightly (`#![feature(linked_list_cursors)]`), and the toolchain is pinned stable. `inject-lib`
finds `LoadLibraryW` by walking the target's module list, which a suspended process doesn't have yet. So the code
uses the `windows` crate the launcher kit already depends on, with three more features
(`Win32_System_Diagnostics_Debug`, `Win32_System_Diagnostics_ToolHelp`, `Win32_System_LibraryLoader`). No
workspace dependency changed. The `unsafe` is item-scoped `#[allow(unsafe_code, reason)]` with `SAFETY:` notes, as
in `platform/keystore/os.rs`.

**The feed and the install.**
- `runtime_files::PLAYER_RUNTIME_FILES` lists what a game folder holds: `Rustic-Isles.dll` on Windows, and the
  runtime and the loader elsewhere.
- The Windows install skips a listed `winmm.dll` and retires one it installed itself (`retire_loader`).
- A Linux launcher shown a feed without the loader reports the runtime as unavailable, rather than starting a game
  that wouldn't load it.

**Host tools.** On Windows the export session injects the runtime and then CatalogExporter, and the dump session
injects Dumper-7 alone. Neither writes an ini or a session record. Elsewhere `loader_session.rs` works as before.

**Still to do.** `native/runtime-loader` stays: servers and Proton players use it. Delete it once the director
injects too and `dainty-inject.exe` has shipped. Then drop `LOADER_DLL` from the protocol, ending the one release
it is optional for.

## Hands-on checks (maintainer, Windows PC with the game)

Not run. `cargo test` only covers injecting a system DLL into a suspended `cmd.exe`.

1. Install the runtime from a server whose feed still lists `winmm.dll`. The launcher downloads only
   `Rustic-Isles.dll`, and a `winmm.dll` that the launcher installed before is removed.
2. PLAY. The log shows the runtime starting with no ini. The game reaches the menu signed in through the local
   backend.
3. While the game runs, Process Explorer (or `listdlls`) on `Dauntless-Win64-Shipping.exe` lists `Rustic-Isles.dll`
   from `Win64` and no `winmm.dll` from `Win64`.
4. Copy some other `winmm.dll` into `Win64`. PLAY refuses with the leftover message. Remove it.
5. Change one byte of `Rustic-Isles.dll`. PLAY refuses (the runtime check, or the injector's hash check), and no game
   process is left behind.
6. On the Host page, run a game-data export. The log shows CatalogExporter's SHA-256, the module list shows it in the
   game, and the export files appear.
7. Run a Dumper-7 session. The SDK is written, and `Rustic-Isles.dll` is not in the game's module list.
8. On Linux/Proton, PLAY as before: the loader still loads the runtime.
