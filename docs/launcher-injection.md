# The launcher injects the game's DLLs

Status: **implemented on Windows, and for Proton behind a setting; not yet tried with the game.** The security
checklist is [security/launcher-injection.md](security/launcher-injection.md).

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
3. **Linux/Proton players choose (amended later on 2026-11-12).** A Settings switch, "Load the runtime with the
   inject helper" (`injectHelper`, Linux only, off by default), picks between the runtime loader (`winmm.dll`, and
   `mystic_loader.ini` for the Host sessions) and the `dainty-inject.exe` helper. The default stays the loader until
   the hands-on Proton check below passes.

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

Before the first DLL, a `Sleep(0)` thread runs once in the game. It finishes the loader's start-up on a thread that
loads nothing, which Wine may need and Windows doesn't mind.

**Crates.** The maintainer asked for existing crates where available. `dll-syringe` 0.18 handles suspended
processes but needs nightly (`#![feature(linked_list_cursors)]`), and the toolchain is pinned stable. `inject-lib`
finds `LoadLibraryW` by walking the target's module list, which a suspended process doesn't have yet. So the code
is a new MIT crate, `crates/dainty-inject`, on the `windows` crate the workspace already uses. The launcher kit
calls its library, and its binary is the Proton helper. The `unsafe` is item-scoped
`#[allow(unsafe_code, reason)]` with `SAFETY:` notes, as in `platform/keystore/os.rs`. The workspace `Cargo.toml`
gained the crate's `[workspace.dependencies]` line, and `fuzz/` gained the `inject_helper_args` target.

**Proton: the helper.** The launcher is a Linux process and can't reach into a Wine process, so with the switch on,
Proton runs `dainty-inject.exe` in the game's place
(`LaunchPlan::through_helper`):

```text
proton run <resources>/dainty-inject.exe --dll Z:\...\Rustic-Isles.dll <sha256> [--dll ...] -- Z:\...\Dauntless-Win64-Shipping.exe <game arguments>
```

The helper parses only that form (`dainty_inject::helper::parse`). It starts the game suspended inside the same
prefix, runs the same `load_and_resume`, waits, and exits with the game's code. Its own failures exit with
`0xD1A00001` (a bad command line), `0xD1A00002` (the game didn't start) or `0xD1A00003` (a DLL wasn't loaded; the
game was killed), which the launcher reports as a failed launch with that code. A launcher with no helper in its
resources folder refuses PLAY with `inject_helper_missing`. With the switch on:
- The game folder holds the runtime alone (`player_runtime_files(false)`).
- A loader the launcher installed is retired, and any other loader is refused, as on Windows.
- An export session goes through the helper too.

Turning the switch off puts the loader back at the next runtime check.

**Shipping the helper.** The Linux job of `launcher-release.yml` (both the `.forgejo/` and `.github/` copies; the
maintainer asked for it, though workflows are RR3's) installs `gcc-mingw-w64-x86-64` and builds
`cargo build --release --locked -p dainty-inject --target x86_64-pc-windows-gnu`. It bundles the exe into the
launcher's AppImage, deb and rpm, not the manager's, as the resource `dainty-inject.exe`, through an extra
`--config` (`inject-helper.json`). Its package check then fails a deb or rpm without it. A local or development
build without the helper still runs; only PLAY with the switch on refuses.

**The feed and the install.**
- `runtime_files::player_runtime_files` lists what a game folder holds: `Rustic-Isles.dll` when the runtime is
  injected (Windows, or Proton with the helper), and the runtime and the loader when the loader loads it.
- The Windows install skips a listed `winmm.dll` and retires one it installed itself (`retire_loader`).
- A Linux launcher shown a feed without the loader reports the runtime as unavailable, rather than starting a game
  that wouldn't load it.

**Host tools.** On Windows the export session injects the runtime and then CatalogExporter, and the dump session
injects Dumper-7 alone. Neither writes an ini or a session record. Under Proton the export follows the switch, and the
dump session (which starts the game without a runner) keeps `loader_session.rs`.

**Still to do.** `native/runtime-loader` stays: servers and Proton players on the loader use it. Once the helper
has passed under Proton and become the default, and the director injects too, delete it. Then drop `LOADER_DLL`
from the protocol and the client feed.

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
8. On Linux/Proton with the switch off, PLAY as before: the loader still loads the runtime.

### Proton with the helper (Linux PC with the game)

Not run. `cargo test` covers the helper against a real `cmd.exe` on Windows, not under Wine.

1. Build the helper (`cargo build -p dainty-inject --release --target x86_64-pc-windows-gnu`). Put it in the
   launcher's resources folder as `dainty-inject.exe`: next to the AppImage's other resources, or in
   `crates/../brand/launcher` for a development run.
2. In Settings, turn on "Load the runtime with the inject helper". The runtime check asks to reinstall. The launcher
   removes the `winmm.dll` it installed, and the folder holds `Rustic-Isles.dll` alone.
3. PLAY. The log's command line shows `dainty-inject.exe --dll Z:\...\Rustic-Isles.dll <sha256> -- Z:\...`
   (the key masked). The game reaches the menu signed in.
4. If it doesn't, the game-exit log line shows the code: `0xD1A00003` means Wine refused the load (keep the
   loader as the default and report it), and `0xD1A00002` means the game didn't start through the helper.
5. Turn the switch off. The runtime check reinstalls `winmm.dll`, and PLAY works through the loader again.
6. With the switch on and the helper removed from the resources folder, PLAY refuses with the "inject helper is
   missing" message.
