# The SDK, the runtime and the game data

Nothing from the game is in this repository. Every host produces three things from their own Dauntless 1.14.7,
on a Windows PC with the game installed:

1. the **SDK**: C++ headers generated from the running game, which the CatalogExporter is compiled against;
2. the **runtime**: `rustic-isles-runtime.dll`, the MIT runtime built from the root's `crates/rustic-isles-runtime`
   with cargo and no SDK, or the prebuilt one the server release ships in `bin/` (`spec-runtime-r21.md`, call 6),
   which players install from your server and the launcher and Director inject (RD9); `winmm.dll`
   (`native/runtime-loader`) is optional, for Proton players who load it the old way;
3. the **game data**: JSON the backend and the Director serve, generated from an export the CatalogExporter
   (`native/rustic-isles-exporter`, MIT) makes inside the game.

The launcher's **Host page** (a build with the host tools) has a button for each step and shows what is
missing; `dainty-host doctor` says the same on the command line. A Linux host does these on a Windows PC and
brings the results over (`restricted/docs/hosting-linux.md`). Every path below is under `restricted/`.

## 1. The SDK

Dump the SDK with [Dumper-7](https://github.com/Encryqed/Dumper-7) from your own 1.14.7 client: the Host page
injects the Dumper-7 DLL into the game it starts (only that game, RD9), or do it by hand
as Dumper-7 documents. The dump is a `CppSDK` folder with `SDK.hpp` inside. Import it into
`modules/rustic-isles/native/runtime` (the Host page's **Import SDK**, or copy it there as the folder's `BUILD.md` says).
`doctor` reports the SDK as imported once it sees the generated `*_functions.cpp` files.

Build Dumper-7 at commit `3a57413` (main as of 2026-10-17) or older. From `42f37f6` (merged into main on
2026-10-26 as PR #588) it writes each function's lookup with its class's name instead of the function's, for
example `GetStaticName(L"BP_PlayerCharacter_C", FnName)` inside `ABP_PlayerCharacter_C::TickStamina`. That SDK
compiles, but the server dies with `Failed to find function BP_PlayerCharacter_C in BP_PlayerCharacter_C` as
soon as a player joins Ramsgate. The fix is upstream PR #592, still open on 2026-11-10; a commit after it is
fine once it is merged. The launcher's **Import SDK** repairs such a dump on the way in and logs how many
lookups it rewrote; an SDK copied in by hand is not repaired, so dump it again with a good Dumper-7.

The SDK is Phoenix Labs' material: never commit it, never share it. `.gitignore` refuses `CppSDK/` and
`SDK.hpp`.

## 2. The runtime

Since R21 the runtime is `rustic-isles-runtime.dll` alone, MIT, from the root's `crates/rustic-isles-runtime`
(`docs/port/spec-runtime-r21.md`). A server release ships it prebuilt in `bin/`, so `setup` and `publish-runtime`
need no build. In a source checkout, with the Rust toolchain and the MSVC linker (Visual Studio's C++ workload, no
SDK), the Host page's **Build the runtime** or:

```powershell
cargo build --release --locked --target x86_64-pc-windows-msvc -p rustic-isles-runtime
cargo build --release --target x86_64-pc-windows-msvc --manifest-path native\runtime-loader\Cargo.toml   # winmm.dll, optional
```

The runtime reads the backend's address from the environment at run time, so one build serves every server and
every player of yours. Publish it:

```powershell
bin\dainty-host publish-runtime                  # this checkout's build, else the release's bin\rustic-isles-runtime.dll
bin\dainty-host publish-runtime --from C:\dlls   # or from a folder holding it (cargo's rustic_isles_runtime.dll counts)
```

`publish-runtime` removes `Rustic-Isles.dll` and its old names from `.workspace/runtime`, and `doctor` warns about
one left there or in the servers' `Win64`. The content server then lists the files with their hashes; players accept
them once in the launcher. A rebuilt runtime has new hashes, and players are asked again.

## 3. The game data

The CatalogExporter is the root's MIT crate `native/rustic-isles-exporter` (clean-room from
`docs/port/spec-catalog-exporter.md`), built with cargo against the same SDK. The Host page's three steps do it all:
**Build the CatalogExporter** (needs the Rust toolchain and the C++ tools' linker), **Start the game with
CatalogExporter** (injected after the runtime into the game it starts, RD9) and **Copy the exports**, which brings
them from the game folder into `modules/rustic-isles/reference/1.14.7/Items_Analysis/`. By hand, from the checkout's
root:

```powershell
$env:DAINTY_SDK_DIR = "$PWD\restricted\modules\rustic-isles\native\runtime"
cargo build --release --target x86_64-pc-windows-msvc --manifest-path native\rustic-isles-exporter\Cargo.toml -p catalog-exporter-dll
dainty-launcher export --wait   # loads native\rustic-isles-exporter\target\x86_64-pc-windows-msvc\release\CatalogExporter.dll
```

The exports keep the C++ exporter's file names and formats (`catalog_1_14_7.jsonl`, the store images, the
`progression_1_14_7` folder), and the Host page's flags file turns on every export `gamedata` reads.
`modules/rustic-isles/native/catalog-exporter` is Mystic Paradox's C++ original, which nothing builds by default
any more. The progression export comes from the game the same way, and
so does `weapon_slots_1_14_7.jsonl` (the live weapons' part slots): when it covers every starter weapon, the server
takes the starter kit's default weapon parts from it instead of the shipped ones (ruling V-4). Then:

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
