# Generating the SDK

native/catalog-exporter builds against a C++ SDK generated from your own compatible game installation. No
game-derived SDK is distributed in this repository. The game runtime no longer needs it: since R21 it is the MIT
`rustic-isles-runtime.dll`, built with cargo at the monorepo root, and the C++ runtime that once lived in
native/runtime was deleted (its last commit is the tag `agpl-runtime-final`). The folder native/runtime now only
holds this SDK, where the CatalogExporter's project looks for it.

> [!NOTE]
> This guide is for Dauntless 1.14.7, which `main` targets. For 1.12.0, follow the guide on the
> [`dauntless-1.12.0`](https://github.com/pranav158/Mystic-Paradox/tree/dauntless-1.12.0) tag.

## Required target

Generate the SDK from exactly:

| Property | Value |
|---|---|
| Game | Dauntless 1.14.7 |
| Build label | rel-1.14.7-Archon |
| Changelist | 647472 |
| Unreal Engine | 4.26.2 |
| Platform | Windows x64 |

A dump from a different client may compile but has incompatible layouts, functions, or offsets.
Nothing generated from 1.12.0 is reusable: class layouts moved even though the engine version did not
change.

## Generate and copy the SDK

1. Build [Dumper-7](https://github.com/Encryqed/Dumper-7).
2. Start your own 1.14.7 game installation.
3. Inject Dumper-7 into that process and wait for the complete CppSDK output.
4. Copy the entire CppSDK output into native/runtime (create the folder).

Expected layout:

    native\runtime\
      SDK\
        ...per-package headers...
        ...generated *_functions.cpp files...
      SDK.hpp
      Assertions.inl
      NameCollisions.inl
      PropertyFixup.hpp
      UnrealContainers.hpp
      UtfN.hpp

Do not copy only SDK.hpp or only headers. The generated implementation files under SDK are compiled
by the CatalogExporter project and must be present.

Quick checks from the repository root:

    Test-Path native\runtime\SDK.hpp
    Test-Path native\runtime\SDK
    (Get-ChildItem native\runtime\SDK -Filter '*_functions.cpp').Count

The final count must be greater than zero.

All generated SDK files are ignored. Confirm that they are not staged before committing public
source.

## Build CatalogExporter

Confirm the project's include paths resolve to the complete native/runtime SDK, then build (platform toolset
v145, Visual Studio with the Desktop development with C++ workload):

    Set-Location native\catalog-exporter
    .\_build.bat

Visual Studio 2022 users can retarget CatalogExporter.vcxproj to v143 locally.

By default the exporter writes to `Items_Analysis` in the injected game's working directory and reads
`export_flags.txt` from there. To send every export to your repository instead, create the ignored
file native/catalog-exporter/ExportPaths.local.h before building:

    #define RUSTIC_ISLES_EXPORT_ROOT L"D:\\RusticIsles"

Exports then land in `D:\RusticIsles\Items_Analysis` and the flags file is read from
`D:\RusticIsles\native\catalog-exporter\export_flags.txt`. The game-data generators read
`reference\1.14.7\Items_Analysis`, so move the exports there before running them.

Copy export_flags.example.txt to export_flags.txt, enable only the required modes, and inject the
resulting DLL into your own game process. The modes and what each one needs are listed in
[GENERATING_GAME_DATA.md](GENERATING_GAME_DATA.md) and
[ExportFlags.md](../native/catalog-exporter/ExportFlags.md).

Some exports depend on tables that the client streams only after opening a menu or reaching a world.
Review the exporter status output and manifest instead of assuming a successful injection produced
every required row.

Continue with [GENERATING_GAME_DATA.md](GENERATING_GAME_DATA.md) after extraction.
