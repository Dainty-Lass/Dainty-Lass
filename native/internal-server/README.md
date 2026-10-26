# UndauntedInternalServer (legacy servers only)

The game-server DLL for **Dauntless 1.4.4** ("legacy") servers: Undaunted's internal server, as Dauntless
Revived (DR) carried it, at DR `24b7624` (`native/internal-server`). It is loaded into the 1.4.4 game started with
`-server` and turns it into a dedicated hunt or city server; in the player's 1.4.4 game it shows the DualSense
prompts from `../client-mods/PS5Icons`.

It has nothing to do with Reforged's own 1.14.7 servers, which run Mystic Paradox's runtime
(`../paradox-runtime`). Build it only to host a legacy server, behind the `legacy` features.

## What is not here

DR's folder is 107 MB because it commits the complete Dumper-7 dump of the 1.4.4 client: `SDK/` (4,148 files,
64 MB) and `SDK.hpp`, `Assertions.inl` (37 MB of offset asserts), `NameCollisions.inl`, `PropertyFixup.hpp`,
`UnrealContainers.hpp` and `UtfN.hpp`. That dump is generated from Phoenix Labs' game and is never committed here
(AGENTS.md, "Nothing from the game is committed"); `../.gitignore` keeps it out. Everything else in DR's folder
is here: the source, MinHook, the Visual Studio project and the tests. Build output (`x64/`, `*.dll`) is
ignored as before.

DR edited two files of its dump by hand. `sdk-fixups.patch` carries those edits so they survive a fresh dump:

- `SDK/Basic.hpp`: `FName::GetRawString` no longer hands a stack buffer to `FString` (DR `3d227a3`).
- `SDK/Engine_classes.hpp`: `UWorld`'s unnamed `Pad_28` becomes `void* NetworkNotify`, which `dllmain.cpp` and
  `Networking.cpp` use when the server starts listening (DR `3d227a3`).

DR's third edit, a reshaped loop in `UClass::GetFunction`, behaves the same as Dumper-7's and is left out.

## Regenerate the SDK

The SDK must come from the 1.4.4 client (`1.4.4_shipping`, changelist 239827; `SDK.hpp` names
`4.25.3-239827+++dauntless+rel-1.4.4`). As for the runtime ([docs/sdk-and-game-data.md](../../docs/sdk-and-game-data.md)),
each host dumps their own:

1. Build [Dumper-7](https://github.com/Encryqed/Dumper-7) (Release x64), start the 1.4.4 game without any
   loader or server DLL next to it, inject Dumper-7 and wait in the main menu until
   `C:\Dumper-7\<game>\CppSDK\` is complete.
2. Copy the whole `CppSDK` folder's contents into this folder. A partial copy is not enough: the project
   compiles ten of the `SDK\*_functions.cpp` files.
3. From the repository root, apply DR's edits:

   ```bat
   git apply --ignore-whitespace --directory=native/internal-server native/internal-server/sdk-fixups.patch
   ```

   `--ignore-whitespace` lets it apply to a dump written with CRLF line endings. If it fails, the dump's
   Dumper-7 version lays `Basic.hpp` or `UWorld` out differently: make the two edits above by hand.

## Build

Visual Studio with Desktop development with C++ and a Windows SDK. The project asks for the v143 toolset
(Visual Studio 2022), as DR built it; on Visual Studio 2026 add `/p:PlatformToolset=v145` or retarget. From
this folder:

```bat
_build.bat
```

It refuses to start without the SDK, with a dump of another version, or with `sdk-fixups.patch` not applied,
then builds Release x64 into `x64\Release\UndauntedInternalServer.dll`. The file keeps DR's name: the 1.4.4
server kits and Game.ini settings name it.

## Tests

`tests/` holds DR's standalone checks of the policy headers. None needs the SDK. From the repository root:

```sh
for t in native/internal-server/tests/*.cpp; do g++ -std=c++20 -Wall -Wextra -Werror -O2 "$t" -o "/tmp/$(basename "$t" .cpp)" && "/tmp/$(basename "$t" .cpp)"; done
```

`server-locale.cpp`, `dualsense.cpp` and `console-visibility.cpp` use the Windows CRT, HID and XInput: build those
with `cl /std:c++20 /EHsc /W4 /WX /O2 <test>.cpp user32.lib` from a Developer Command Prompt. The three
`*_TESTING.md` files are DR's manual checks against a running server.

## How it reaches the game

The same DLL runs in the player's game and on the servers, next to `Archon\Binaries\Win64\Dauntless-Win64-Shipping.exe`
of the 1.4.4 install. Reforged loads it with its own runtime loader (`../runtime-loader`, `winmm.dll`), never with
DR's `dxgi.dll` proxy, whose source was never published (RD9: DLLs reach the game only through the loader):

```ini
; mystic_loader.ini, beside winmm.dll in the 1.4.4 Win64 folder
UndauntedInternalServer.dll
```

A 1.4.4 folder holds no `MysticParadox.dll`, so the loader loads nothing but this list. DR's `dxgi.dll` must not be
left beside it, or the DLL loads twice. Under Wine the native loader needs `WINEDLLOVERRIDES=winmm=n,b`, where DR
forced `dxgi=n,b`.

The DLL links the C++ runtime dynamically (`MSVCP140.dll`, `VCRUNTIME140_1.dll`), as DR shipped it: the player
and the server need the Visual C++ 2015-2022 x64 runtime.

## Release

Players need one DLL that works on every legacy server, so the launcher installs a single build pinned by
SHA-256, made by the maintainer, and hosts run that same build. A host may build their own for their servers;
players never do. For each release the maintainer:

1. Builds it as above from their own 1.4.4 dump, and `runtime-loader` with `cargo build --release`.
2. Checks the import list (`dumpbin /imports`): `MSVCP140.dll`, `VCRUNTIME140*.dll` and Windows system DLLs, nothing
   else unexpected.
3. Publishes `UndauntedInternalServer.dll` with its SHA-256 where the launcher and the host kit fetch it, and updates
   both pins together.

Before the first release, once, on a PC with the 1.4.4 game (not done yet):

1. `dumpbin /imports Dauntless-Win64-Shipping.exe` lists `WINMM.dll`. If it doesn't, the loader needs another proxy
   name for 1.4.4.
2. With `winmm.dll`, the `mystic_loader.ini` above and the DLL beside the exe (and no `dxgi.dll`), the player's game
   reaches the main menu through a legacy server and a DualSense shows the PS5 prompts.
3. A hunt server and Ramsgate started by the host kit with the same three files come up and accept that player.

## How a host runs it

The host kit starts the 1.4.4 game as DR's deploy server did (DR `rust/services/deploy/src/gameservers.rs`):
seven positional arguments (the game-server API key, the port, the map, the
behemoth or `NO_BEHEMOTH`, the matchmaker hunt id or `NO_MM_HUNTID`, the expected players or
`NO_EXPECTED_PLAYERS`, this server's `ip:port`), then `-EpicPortal -server -nullrhi`. Settings it reads from the
environment, unchanged from DR:

| Variable | Effect |
| --- | --- |
| `DR_SERVER_READY_FILE` | File written once the server listens |
| `GAMESERVER_READY_DIR` | Folder for the ready marker and native diagnostics |
| `DR_SERVER_MAX_FPS` | Server frame limit |
| `DR_RAMSGATE_POOL` | `1`: this Ramsgate is a pooled city |
| `DR_SHOW_CONSOLE` | `1`: keep the console window |

## Changes from DR

- The game-server API key goes out as `x-mysticparadox-gameserver-apikey`, not `x-undaunted-gameserver-apikey`
  (migration guide, section 3), so the ported metagame accepts it.
- `_build.bat` and the SDK checks in it, this README, `sdk-fixups.patch`, and the SDK's ignore rules in
  `../.gitignore`. Every other file is DR's, byte for byte.
