# Generating the SDK

Dainty Lass's CatalogExporter (`native/rustic-isles-exporter` at the checkout's root) builds against a C++ SDK
generated from your own compatible game installation. No
game-derived SDK is distributed in this repository. The game runtime no longer needs it: since R21 it is the MIT
`rustic-isles-runtime.dll`, built with cargo at the monorepo root, and the C++ runtime that once lived in
native/runtime was deleted (its last commit is the tag `agpl-runtime-final`). The folder native/runtime now only
holds this SDK, where the CatalogExporter's build looks for it (`DAINTY_SDK_DIR`). Mystic Paradox's C++ exporter
that once lived in native/catalog-exporter was deleted too (tag `agpl-catalog-exporter-final`).

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
by the CatalogExporter's build and must be present.

Quick checks from the repository root:

    Test-Path native\runtime\SDK.hpp
    Test-Path native\runtime\SDK
    (Get-ChildItem native\runtime\SDK -Filter '*_functions.cpp').Count

The final count must be greater than zero.

All generated SDK files are ignored. Confirm that they are not staged before committing public
source.

## Build CatalogExporter

The CatalogExporter is Dainty Lass's MIT Rust crate `native/rustic-isles-exporter`, built with cargo against the
SDK in native/runtime. The Host page's **Build the CatalogExporter**, **Start the game with CatalogExporter** and
**Copy the exports** steps build it, inject it and bring its `Items_Analysis` exports into `reference.14.7`.
Section 3 of Dainty Lass's [SDK and game data guide](../../../../docs/sdk-and-game-data.md) gives the commands to do
it by hand and the exports it writes.

Some exports depend on tables that the client streams only after opening a menu or reaching a world.
Review the exporter status output and manifest instead of assuming a successful injection produced
every required row.

Continue with [GENERATING_GAME_DATA.md](GENERATING_GAME_DATA.md) after extraction.
