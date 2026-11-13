# Clean-room Behaviour Specification: the runtime loader (`native/runtime-loader`)

## 1. Purpose and scope

The runtime loader is a `winmm.dll` the game (`Dauntless-Win64-Shipping.exe`, x86_64) loads from its own folder in
place of Windows' `winmm.dll`. While the process starts it loads the Rustic Isles runtime DLL beside it, plus any
extra DLLs a host lists in `mystic_loader.ini`. Every `winmm.dll` (and `version.dll`) function it exports forwards to
the real one in the Windows system directory, so the game's audio, timers and joystick calls behave as without it.
The Director also loads it under Wine (`WINEDLLOVERRIDES="winmm=n,b"`).

- **Folder**: `native/runtime-loader` at the repository root, a crate of its own outside both Cargo workspaces
  (its `Cargo.toml` carries an empty `[workspace]` table).
- **Package**: `rustic-isles-loader`, edition 2024, `license = "MIT"`, `publish = false`.
- **Library**: `name = "winmm"`, `crate-type = ["cdylib"]`. The release build for `x86_64-pc-windows-msvc` must land at
  `native/runtime-loader/target/x86_64-pc-windows-msvc/release/winmm.dll` (`dainty-host publish` and the launcher
  look there).
- **Release profile**: `lto = true`, `strip = true`, `opt-level = "z"`, `codegen-units = 1`, `panic = "abort"`.
- **Toolchain**: its own `rust-toolchain.toml` pinning the same stable channel as the root's (`1.99.1`, profile
  `minimal`, components `clippy`, `rustfmt`). No nightly feature is used.
- **Dependencies**: at most `windows-sys` (types and the few kernel32 functions in section 4) or none, declaring the
  externs by hand. It must never link against `winmm.dll` or `version.dll` itself (an import from the DLL it replaces
  resolves to itself).
- **Lints**: the crate needs `unsafe` (FFI and `#[unsafe(no_mangle)]` exports); allow it at the crate root with a
  one-line reason. Deny `unwrap_used`, `expect_used`, `dbg_macro` and `todo` outside tests, as the root does.
- **Other platforms**: on a non-Windows target the crate compiles to an empty library (everything behind
  `#![cfg(windows)]` or per-item cfgs), so `cargo build` and `cargo test` pass on Linux too.
- A `.gitignore` with `/target`, a committed `Cargo.lock`, an MIT `LICENSE` (`Copyright (c) 2026 Silent Spud`) and a
  short `README.md` (what it is, the build command, where the output goes, what the ini does, the warning that every
  listed DLL runs inside the game).

## 2. Process attach

On `DLL_PROCESS_ATTACH` (and only then) the loader:

0. Stands down when the process's `DAINTY_RUNTIME_INJECTED` variable is exactly `1`, and loads nothing (neither
   the runtime nor the ini's DLLs). The launcher sets it on every game it loads the runtime into itself (RD9 as
   amended on 2026-11-12), so a host whose servers use this loader can play from the same `Win64` without the
   runtime loading twice. The file carries the bytes `dainty-loader-stands-down:DAINTY_RUNTIME_INJECTED=1`. The
   launcher looks for them to tell a loader that stands down from an older one, which it refuses beside an
   injected game.
1. Finds its own file path (`GetModuleFileNameW` on its own module handle), in UTF-16 with room for long paths
   (32 768 units). If the call fails or the path fills the buffer, it does nothing more.
2. Takes that path's parent folder: the **loader folder**.
3. Loads the runtime chosen by section 3 (at most one file).
4. Then loads the ini's extra DLLs (section 5), in file order.

Every failure is silent: no message box, no log, no file written. `DllMain` returns `TRUE` in every case, so the game
starts even when nothing could be loaded. Loading happens from `DllMain` itself (under the loader lock), as the game
expects the runtime to be in place before its own code runs. Nothing in `DllMain` may unwind across the FFI boundary.

Loading a DLL means `LoadLibraryW` with the full path (loader folder joined with the name), NUL-terminated UTF-16,
the result ignored apart from success or failure. Nothing is ever unloaded.

## 3. Choosing the runtime

The candidate names, in this order:

| Index | Name | Why |
| --- | --- | --- |
| 0 | `Rustic-Isles.dll` | The canonical name |
| 1 | `MysticParadox.dll` | Its name before the Rustic Isles rename, kept for one release |
| 2 | `MystPaxInternalServer.dll` | An older alias the launcher still keeps in sync |

The launcher's `crates/dainty-launcher-kit/src/platform/loader_session.rs` (`RUNTIME_DLLS`) carries the same list and
order; the two must not drift.

For each candidate in the loader folder that exists and is a regular file (following symlinks; a folder with that
name does not count), take its last-modified time; when that time can't be read, use the Unix epoch. The chosen file
is the one with the newest time. Ties:

- a tie with the canonical name goes to the canonical name;
- a tie between the two aliases goes to `MystPaxInternalServer.dll`.

(Equivalently: rank by `(modified, is_canonical, index)` and take the maximum.) The canonical-name check ignores ASCII
case. No candidate present: no runtime is loaded, which is normal for a 1.4.4 folder whose only DLL comes from the
ini.

## 4. Forwarding to the system DLLs

Each export (section 6) forwards to the function of the same name in the real system DLL:

- The system DLL is loaded by full path: the folder `GetSystemDirectoryW` returns (same buffer size and failure rule as
  section 2), joined with `winmm.dll` or `version.dll`, then `LoadLibraryW`. Never by bare name, which would find the
  loader itself.
- Each system DLL is loaded at most once per process and each function looked up (`GetProcAddress`, by its ASCII
  name) at most once per process in the steady state; the result, found or not, is reused. Lookups are lazy, on the
  first call of an export, not in `DllMain`.
- Resolution is thread-safe: any thread may make the first call of any export, concurrently with others. A cached
  function pointer is read without taking a lock on later calls (`timeGetTime` is called every frame). A race that
  resolves the same function twice is acceptable; a torn or wrong pointer is not.
- **Arguments and results pass through unchanged.** Every argument reaches the system function bit for bit and its
  return value comes back bit for bit. On x86_64 none of these functions takes or returns a float or a struct by
  value, so each forwarder may declare its parameters and result as register-width integers (`usize`) and still be
  ABI-exact, provided it declares exactly the parameter count section 6 gives (the documented Windows SDK count). A
  forwarder that widens a 32-bit parameter to `usize` passes the caller's register through untouched, which is
  correct; one that narrows a pointer-sized parameter is a bug.
- When the system DLL or the function can't be found, the export returns its **fallback** (section 6) instead and
  calls nothing, except the four `version.dll` `Ex` functions, which call the loader's own non-`Ex` export with the
  `dwFlags` argument dropped (for systems whose `version.dll` predates them).

## 5. `mystic_loader.ini`

An optional file named `mystic_loader.ini` in the loader folder lists extra DLLs to load after the runtime. Missing,
unreadable or not valid UTF-8: no extra DLL is loaded and nothing else changes. A leading UTF-8 byte-order mark is
skipped.

Each line (split on `\n`, a trailing `\r` dropped):

1. Everything from the first `;` or `#` on is a comment and is removed.
2. The rest is trimmed of whitespace.
3. Empty lines and lines starting with `[` (section headers) are skipped.
4. A line containing `=` uses what follows the first `=`, trimmed; otherwise the whole line.
5. An empty value is skipped.

Each value, in file order:

- is skipped when it equals one of the three runtime names of section 3, ignoring ASCII case (the runtime is chosen
  only by section 3; an ini line can never load a second or older copy). The comparison is on the value as written,
  so `sub\Rustic-Isles.dll` is not skipped;
- is joined to the loader folder with the platform's path join (an absolute value replaces the folder);
- is loaded only if that path exists; a failed load is ignored and the next value is tried.

Duplicate values are loaded again (Windows counts the reference; the DLL is mapped once).

Examples, values listed for each file:

| ini text | values |
| --- | --- |
| `extra.dll` | `extra.dll` |
| `; note`, blank, `[mods]`, `  b.dll  ` | `b.dll` |
| `dll1 = a.dll`, `x=  ` | `a.dll` |
| `a.dll ; trailing`, `c.dll#x` | `a.dll`, `c.dll` |
| `k = v = w.dll` | `v = w.dll` |
| `Rustic-Isles.dll`, `rustic-isles.DLL`, `MYSTPAXINTERNALSERVER.dll`, `UndauntedInternalServer.dll` | `UndauntedInternalServer.dll` loaded; the other three parsed, then skipped |
| `C:\mods\x.dll` | `C:\mods\x.dll` (absolute) |

## 6. The exports

Exactly these names are exported, undecorated (`#[unsafe(no_mangle)]`, `extern "system"`), and no others besides
`DllMain`. The set is what the shipped loader exported; the game and its middleware import from it, so none may be
dropped. `n` is the parameter count. The fallback is `0` (which is also `FALSE`, `MMSYSERR_NOERROR` and a null
handle) unless the row says otherwise.

### 6.1 `version.dll`

| Export | n | Fallback |
| --- | --- | --- |
| `GetFileVersionInfoA` | 4 | 0 |
| `GetFileVersionInfoSizeA` | 2 | 0 |
| `GetFileVersionInfoW` | 4 | 0 |
| `GetFileVersionInfoSizeW` | 2 | 0 |
| `VerQueryValueA` | 4 | 0 |
| `VerQueryValueW` | 4 | 0 |
| `GetFileVersionInfoExA` | 5 | `GetFileVersionInfoA(args 2..5)` |
| `GetFileVersionInfoExW` | 5 | `GetFileVersionInfoW(args 2..5)` |
| `GetFileVersionInfoSizeExA` | 3 | `GetFileVersionInfoSizeA(args 2..3)` |
| `GetFileVersionInfoSizeExW` | 3 | `GetFileVersionInfoSizeW(args 2..3)` |
| `VerFindFileA` | 8 | 1 |
| `VerFindFileW` | 8 | 1 |
| `VerInstallFileA` | 8 | 1 |
| `VerInstallFileW` | 8 | 1 |
| `VerLanguageNameA` | 3 | 0 |
| `VerLanguageNameW` | 3 | 0 |
| `GetFileVersionInfoByHandle` | 5 | 0 |

`GetFileVersionInfoByHandle` is undocumented; five register-width parameters forward any call of up to five
arguments unchanged, which is safe on x86_64 because a callee reading a slot the caller didn't fill only reads the
caller's frame.

### 6.2 `winmm.dll`

All fall back to 0. Counts are the Windows SDK's (`mmsystem.h`, `mmddk.h`); `mmDrvInstall` is undocumented and takes
four.

| Group | Exports (n) |
| --- | --- |
| waveIn | `waveInAddBuffer` 3, `waveInClose` 1, `waveInGetDevCapsA` 3, `waveInGetDevCapsW` 3, `waveInGetErrorTextA` 3, `waveInGetErrorTextW` 3, `waveInGetID` 2, `waveInGetNumDevs` 0, `waveInGetPosition` 3, `waveInMessage` 4, `waveInOpen` 6, `waveInPrepareHeader` 3, `waveInReset` 1, `waveInStart` 1, `waveInStop` 1, `waveInUnprepareHeader` 3 |
| waveOut | `waveOutBreakLoop` 1, `waveOutClose` 1, `waveOutGetDevCapsA` 3, `waveOutGetDevCapsW` 3, `waveOutGetErrorTextA` 3, `waveOutGetErrorTextW` 3, `waveOutGetID` 2, `waveOutGetNumDevs` 0, `waveOutGetPitch` 2, `waveOutGetPlaybackRate` 2, `waveOutGetPosition` 3, `waveOutGetVolume` 2, `waveOutMessage` 4, `waveOutOpen` 6, `waveOutPause` 1, `waveOutPrepareHeader` 3, `waveOutReset` 1, `waveOutRestart` 1, `waveOutSetPitch` 2, `waveOutSetPlaybackRate` 2, `waveOutSetVolume` 2, `waveOutUnprepareHeader` 3, `waveOutWrite` 3 |
| midi | `midiConnect` 3, `midiDisconnect` 3 |
| midiIn | `midiInAddBuffer` 3, `midiInClose` 1, `midiInGetDevCapsA` 3, `midiInGetDevCapsW` 3, `midiInGetErrorTextA` 3, `midiInGetErrorTextW` 3, `midiInGetID` 2, `midiInGetNumDevs` 0, `midiInMessage` 4, `midiInOpen` 5, `midiInPrepareHeader` 3, `midiInReset` 1, `midiInStart` 1, `midiInStop` 1, `midiInUnprepareHeader` 3 |
| midiOut | `midiOutCacheDrumPatches` 4, `midiOutCachePatches` 4, `midiOutClose` 1, `midiOutGetDevCapsA` 3, `midiOutGetDevCapsW` 3, `midiOutGetErrorTextA` 3, `midiOutGetErrorTextW` 3, `midiOutGetID` 2, `midiOutGetNumDevs` 0, `midiOutGetVolume` 2, `midiOutLongMsg` 3, `midiOutMessage` 4, `midiOutOpen` 5, `midiOutPrepareHeader` 3, `midiOutReset` 1, `midiOutSetVolume` 2, `midiOutShortMsg` 2, `midiOutUnprepareHeader` 3 |
| midiStream | `midiStreamClose` 1, `midiStreamOpen` 6, `midiStreamOut` 3, `midiStreamPause` 1, `midiStreamPosition` 3, `midiStreamProperty` 3, `midiStreamRestart` 1, `midiStreamStop` 1 |
| mixer | `mixerClose` 1, `mixerGetControlDetailsA` 3, `mixerGetControlDetailsW` 3, `mixerGetDevCapsA` 3, `mixerGetDevCapsW` 3, `mixerGetID` 3, `mixerGetLineControlsA` 3, `mixerGetLineControlsW` 3, `mixerGetLineInfoA` 3, `mixerGetLineInfoW` 3, `mixerGetNumDevs` 0, `mixerMessage` 4, `mixerOpen` 5, `mixerSetControlDetails` 3 |
| time | `timeBeginPeriod` 1, `timeEndPeriod` 1, `timeGetDevCaps` 2, `timeGetSystemTime` 2, `timeGetTime` 0, `timeKillEvent` 1, `timeSetEvent` 5 |
| joy | `joyConfigChanged` 1, `joyGetDevCapsA` 3, `joyGetDevCapsW` 3, `joyGetNumDevs` 0, `joyGetPos` 2, `joyGetPosEx` 2, `joyGetThreshold` 2, `joyReleaseCapture` 1, `joySetCapture` 4, `joySetThreshold` 2 |
| mci | `mciDriverNotify` 3, `mciDriverYield` 1, `mciExecute` 1, `mciFreeCommandResource` 1, `mciGetCreatorTask` 1, `mciGetDeviceIDA` 1, `mciGetDeviceIDFromElementIDA` 2, `mciGetDeviceIDFromElementIDW` 2, `mciGetDeviceIDW` 1, `mciGetDriverData` 1, `mciGetErrorStringA` 3, `mciGetErrorStringW` 3, `mciGetYieldProc` 2, `mciLoadCommandResource` 3, `mciSendCommandA` 4, `mciSendCommandW` 4, `mciSendStringA` 4, `mciSendStringW` 4, `mciSetDriverData` 2, `mciSetYieldProc` 3 |
| mmio | `mmioAdvance` 3, `mmioAscend` 3, `mmioClose` 2, `mmioCreateChunk` 3, `mmioDescend` 4, `mmioFlush` 2, `mmioGetInfo` 3, `mmioInstallIOProcA` 3, `mmioInstallIOProcW` 3, `mmioOpenA` 3, `mmioOpenW` 3, `mmioRead` 3, `mmioRenameA` 4, `mmioRenameW` 4, `mmioSeek` 3, `mmioSendMessage` 4, `mmioSetBuffer` 4, `mmioSetInfo` 3, `mmioStringToFOURCCA` 2, `mmioStringToFOURCCW` 2, `mmioWrite` 3 |
| sound | `PlaySound` 3, `PlaySoundA` 3, `PlaySoundW` 3, `sndPlaySoundA` 2, `sndPlaySoundW` 2 |
| aux | `auxGetDevCapsA` 3, `auxGetDevCapsW` 3, `auxGetNumDevs` 0, `auxGetVolume` 2, `auxOutMessage` 4, `auxSetVolume` 2 |
| driver | `CloseDriver` 3, `DefDriverProc` 5, `DriverCallback` 7, `DrvGetModuleHandle` 1, `GetDriverModuleHandle` 1, `OpenDriver` 3, `SendDriverMessage` 4 |
| mm | `mmsystemGetVersion` 0, `mmDrvInstall` 4, `mmGetCurrentTask` 0, `mmTaskBlock` 1, `mmTaskCreate` 3, `mmTaskSignal` 1, `mmTaskYield` 0, `WOWAppExit` 1 |

That is 17 `version.dll` exports and 180 `winmm.dll` exports, 197 in all.

## 7. Tests (Windows; on other targets they compile out)

`cargo test` in the crate's folder must cover:

1. **Runtime choice** (section 3), in a fresh temporary folder per test: only the canonical name; only each alias;
   none (only `UndauntedInternalServer.dll`); a newer alias beats an older canonical; an equal time goes to the
   canonical; an equal time between the aliases goes to `MystPaxInternalServer.dll`; a folder named
   `Rustic-Isles.dll` is ignored.
2. **Ini parsing** (section 5): every row of the examples table, CRLF line ends, a leading BOM, and invalid UTF-8
   giving no values.
3. **Ini filtering**: the runtime names skipped in any ASCII case; an absolute value replacing the folder.
4. **Export table**: the names the crate exports equal section 6's 197 names exactly (no missing, no extra, no
   duplicate), and each declared parameter count equals the table's. Keep the expected list in the test, written out,
   not derived from the code under test.
5. **The built DLL**: an integration test that loads the debug `winmm.dll` cargo built (found from the test
   executable's path: `target/<profile>/deps/<test>.exe` → `target/<profile>/winmm.dll`) by full path, checks every
   section 6 name resolves with `GetProcAddress`, and calls a few through it, comparing with the system DLL's own
   answer: `timeGetTime` (within a small window), `waveOutGetNumDevs`, `mmsystemGetVersion`, and
   `GetFileVersionInfoSizeW` on `kernel32.dll` in the system folder (nonzero, equal to the system's). Loading it runs
   `DllMain` in a folder with no runtime, which must be harmless.

## 8. Out of scope

- 32-bit builds. The game is x86_64 only.
- Logging, configuration beyond the ini, signature checks on the DLLs it loads (the launcher refuses an unknown DLL or
  leftover ini before PLAY; that is the launcher's check, not the loader's).
- Byte-identical output with the shipped `winmm.dll`; a release keeps the shipped file until a rebuilt one is
  published on purpose.
