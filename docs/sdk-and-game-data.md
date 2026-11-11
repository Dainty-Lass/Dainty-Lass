# The SDK, the runtime and the game data

Nothing from the game is in this repository. Every host produces three things from their own Dauntless 1.14.7,
on a Windows PC with the game installed:

1. the **SDK**: C++ headers generated from the running game, which the runtime is compiled against;
2. the **runtime**: `Rustic-Isles.dll` (built from `modules/rustic-isles/native/runtime` with the SDK) and its loader
   `winmm.dll` (`native/runtime-loader`, Rust), which players install from your server;
3. the **game data**: JSON the backend and the Director serve, generated from an export the CatalogExporter
   makes inside the game.

The launcher's **Host page** (a build with the host tools) has a button for each step and shows what is
missing; `dainty-host doctor` says the same on the command line. A Linux host does these on a Windows PC and
brings the results over (`restricted/docs/hosting-linux.md`). Every path below is under `restricted/`.

## 1. The SDK

Dump the SDK with [Dumper-7](https://github.com/Encryqed/Dumper-7) from your own 1.14.7 client: the Host page
loads the Dumper-7 DLL through the runtime loader (the launcher never injects on its own), or do it by hand
as Dumper-7 documents. The dump is a `CppSDK` folder with `SDK.hpp` inside. Import it into
`modules/rustic-isles/native/runtime` (the Host page's **Import SDK**, or copy it there as the folder's `BUILD.md` says).
`doctor` reports the SDK as imported once it sees the generated `*_functions.cpp` files.

The SDK is Phoenix Labs' material: never commit it, never share it. `.gitignore` refuses `CppSDK/` and
`SDK.hpp`.

## 2. The runtime

Visual Studio 2022 with Desktop development with C++ (the v145 toolset and a Windows SDK). Then:

```bat
modules\rustic-isles\native\runtime\_build.bat          REM Rustic-Isles.dll, one DLL for the client and the dedicated server
cd native\runtime-loader && cargo build --release      REM winmm.dll (needs the Rust toolchain; the loader stays here, DD13)
```

`modules/rustic-isles/native/runtime/BUILD.md` explains the build and the optional
player-hosted module. The runtime reads the backend's address from the environment at run time, so one build
serves every server and every player of yours. Publish it:

```powershell
bin\dainty-host publish-runtime                  # from this checkout's builds
bin\dainty-host publish-runtime --from C:\dlls   # or from a folder with both DLLs
```

The content server then lists the two files with their hashes; players accept them once in the launcher. A
rebuilt runtime has new hashes, and players are asked again.

## 3. The game data

The CatalogExporter (`modules/rustic-isles/native/catalog-exporter`, built with `_build.bat` against the same SDK) runs inside the
game, loaded through the runtime loader like the runtime, and writes the catalogs the backend needs into
`modules/rustic-isles/reference/1.14.7/Items_Analysis/`. The progression export comes from the game the same way. Then:

```powershell
bin\dainty-host gamedata                         # generate ParadoxBackend\game-data and ParadoxDirector\game-data in modules\rustic-isles\reference\1.14.7
bin\dainty-host gamedata --progression ""        # only the progression source, from the workspace's exports
bin\dainty-host gamedata --progression C:\exports  # or from another folder of exports
bin\dainty-host gamedata --example-sources C:\src # copy supplied *.source.json files into the raw game data
bin\dainty-host gamedata --examples              # the shipped examples: the services start, nobody can play
```

`--progression` writes `ParadoxBackend\game-data\raw\progression_config.source.json` and stops; run `gamedata`
afterwards to build the progression config from it. A supplied source already there is kept.

`gamedata` runs the in-process Rust game-data generators (`isles_core::areas::game_data::generators` and
`isles_director::hunt_tables`) on your export with no Node.js required, and fetches the five store captures
(`modules/rustic-isles/reference/1.14.7/ParadoxBackend/store-captures.json` names them, with their hashes) from EisigesEis's repository on your
machine, because no licence allows copying them here. The launcher shows players whether a server runs on real
or example data.

Move the result as a pack, for a Linux host or a second PC:

```powershell
bin\dainty-host gamedata --export gamedata.zip
bin\dainty-host gamedata --import gamedata.zip
```

`dainty-host preset --create` keeps your generated data as a preset in `preset/` of your checkout, and
`setup` applies it to a new server; it is yours and is never committed.
