# Build the combined Mystic runtime

This project builds one combined DLL for both the player client and the dedicated server:

`x64\Release\Rustic-Isles.dll`

The same binary detects `-server` at runtime and selects the existing server hooks; without that
argument it selects the client hooks. Do not build, package, or load separate client/server DLLs.

## Optional player-hosted module

`P2PHooks.h` declares the few hook points of the optional player-hosted (P2P) server mode. When
`p2p\PlayerHost.cpp` exists, the project compiles it; otherwise it compiles `P2PHooksStub.cpp`,
whose dedicated-only hooks claim no auth mode (a server always runs as a DeployServer gameserver). The
player-hosted mode has its own optional anti-cheat module behind `p2p\AntiCheatHooks.h`: `anticheat\*.cpp`
when present, otherwise `p2p\AntiCheatHooksStub.cpp` (no hunt evidence). The conditions are in
`Rustic-Isles.vcxproj`. Code in `p2p\` and `anticheat\` uses the shared helpers in `RuntimeShared.h`;
`dllmain.cpp` and `Networking.cpp` never include anything from either folder. Check every combination after
touching a hook: rename the folders aside and build into another `OutDir`/`IntDir`, never `x64\Release`.

## The SDK

The runtime builds against a Dumper-7 `CppSDK` dumped from the host's own 1.14.7 client and copied into this
folder (`SDK.hpp`, `SDK\`; never committed). Use Dumper-7 `3a57413` or older until upstream PR #592 is merged:
`42f37f6` writes `GetStaticName(L"<class>", FnName)` in every function lookup instead of the function's name,
and the server stops with `Failed to find function BP_PlayerCharacter_C in BP_PlayerCharacter_C` when a player
joins (`GameEngineTickHook` calls `TickStamina`). The launcher's Import SDK repairs such a dump; a copy made by
hand is not. `docs/sdk-and-game-data.md` has the rest.

## Requirements and command

Install Visual Studio with Desktop development with C++, the v145 toolset, and a Windows SDK.
Run from this directory:

```bat
_build.bat
build-loader.bat
```

The runtime script discovers MSBuild with Visual Studio Installer's `vswhere.exe`, builds Release x64,
returns the real build exit code, and does not pause or copy anything into the game directory.

The DLL links the C/C++ runtime statically (`/MT`, set in `Rustic-Isles.vcxproj`). The game folder ships
`msvcp140.dll` 14.24, which the loader prefers over System32, and v145-built code is not supported against it.
Check a new build with an import listing: it must not import `MSVCP140.dll`, `VCRUNTIME140*.dll` or
`api-ms-win-crt-*`. Nothing in the DLL may log or call into the C runtime after `DLL_PROCESS_DETACH`
(`g_MpProcessDetaching` in `dllmain.cpp`); doing so crashed every client exit until 0.2.96.
The historical project/solution directory names remain unchanged for source compatibility; only
the output runtime filename changes.

After compiling, publish the same `Rustic-Isles.dll` separately to the signed client and server
runtime feeds. The target-specific manifests may have different URLs/version metadata, but their
DLL hash must match.

The loader script requires Rust MSVC. It builds the proxy and creates
`tools\RuntimeLoader\target\release\winmm.dll`. The proxy loads `Rustic-Isles.dll`, or the legacy
`MystPaxInternalServer.dll` when that copy is newer, and then any extra DLLs listed in
`mystic_loader.ini` (see `tools\RuntimeLoader\README.md`).
