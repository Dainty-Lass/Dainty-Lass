<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# The game, the SDK, the runtime and the game data

Nothing from the game is in this repository. Every host installs Dauntless 1.14.7 from their own Epic account and
produces three things from it, on a Windows PC with the game installed:

1. the **SDK**: C++ headers generated from the running game, which the CatalogExporter is compiled against;
2. the **runtime**: `rustic-isles-runtime.dll`, the MIT runtime built from the root's `crates/rustic-isles-runtime`
   with cargo and no SDK, or the prebuilt one the server release ships in `bin/` (`spec-runtime-r21.md`, call 6),
   which players install from your server and the launcher and Director inject (RD9); `winmm.dll`
   (`native/runtime-loader`) is optional, for Proton players who load it the old way;
3. the **game data**: JSON the backend and the Director serve, generated from an export the CatalogExporter
   (`native/rustic-isles-exporter`, MIT) makes inside the game.

The launcher's **Host page** (a build with the host tools; turn on **Tools for hosts** in Preferences) has a button
for each step and shows what is missing; `dainty-host doctor` says the same on the command line. A Linux host does
these on a Windows PC and brings the results over ([hosting.md](hosting.md)). Paths below are relative to the server
folder: a checkout or an unpacked server release. Section 6 says what is never shared.

## 1. Installing Dauntless 1.14.7

Dainty Lass plays one build: Dauntless **1.14.7**, Epic build `1.14.7.647472`, whose `Version.txt` reads
`dauntless_rel-1.14.7_Shipping_2024-04-01_14-59-21_647472` and whose
`Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe` has the SHA-256
`f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b`. The launcher and the host kit check that hash;
any other build is refused.

The game is installed from **your own Epic library**, and only when it holds Dauntless. Both installers work the same
way, on `crates/dainty-epic` (DD22): sign in to Epic, check the library, fetch the 1.14.7 manifest from the community
archive (`addison2k/DauntlessManifestArchive`, pinned to one commit and checked against the manifest's SHA-1 before
it is read), download the files from Epic's CDN, check every file against the manifest, then check the executable's
hash. Nothing from the game passes through a Dainty Lass server.

### For playing (and the Host page): the launcher

Open **Game management** in the launcher, sign in with Epic (in the browser, or by pasting the code Epic shows) and
press **Install** on 1.14.7. A download can be paused, and a stopped one carries on from where it was. The launcher
also finds an install that the Epic Games launcher, Legendary or Heroic already made (on Windows), or takes any folder
you choose: the one that contains `Archon`. [players.md](players.md) has the player's side.

### For the game servers: `dainty-host install-game`

The Director runs headless game servers from `settings.toml`'s `game_dir`. On a server without a desktop, install
there from the command line:

```powershell
bin\dainty-host install-game                           # into .workspace\game\1.14.7
bin\dainty-host install-game --dir D:\Games\Dauntless  # into D:\Games\Dauntless\1.14.7 (a full path)
bin\dainty-host install-game --code-file code.txt      # the Epic code from a file instead of pasted
bin\dainty-host install-game --sign-out                # end the Epic session, install nothing
```

1. The first run prints an Epic login address. Open it in a browser on any computer, sign in, and paste the
   authorization code Epic shows (or the whole page) back into the terminal.
2. The session is kept in `.workspace/config/epic-session.env`, owner-only, beside `secrets.env`, and refreshed when
   it nears its end. It is never written into `settings.toml`, an env file, a command line or a log.
3. An account whose library lacks Dauntless gets nothing: the check runs before anything is downloaded.
4. The command prints the download and disk sizes and its progress. Ctrl+C stops it; running the same command again
   resumes (the resume file and the record of finished installs are in `.workspace/epic/`). A finished install whose
   executable still has the pinned hash is left alone.
5. It then points `settings.toml`'s `game_dir` at the new folder and rewrites the env files, so the Director follows.
   With no `settings.toml` yet, give the folder to `setup` as `--game`.

`--version` takes only the versions the build table holds, which today is 1.14.7.

### Keeping it on 1.14.7

Don't let another launcher update or repair the folder: a changed executable fails the hash check, and the launcher
says the folder holds "1.14.7, but not the build this launcher plays". **Verify game files** in the launcher checks a
player's install again.

> **Maintainer:** what `install-game` does with an existing folder whose files were changed is not confirmed here.
> The code resumes from its own resume file and skips a finished install whose executable still matches; say whether
> it re-checks the other files, and how a host repairs a server's install.

## 2. The SDK

Dump the SDK with [Dumper-7](https://github.com/Encryqed/Dumper-7) from your own 1.14.7 client: the Host page
injects the Dumper-7 DLL into the game it starts (only that game, RD9), or do it by hand
as Dumper-7 documents. The dump is a `CppSDK` folder with `SDK.hpp` inside; the Host page looks for dumps under
`C:\Dumper-7`. The Host page's **Import SDK** copies what is inside the newest one into `.workspace/sdk` at the
server folder's root, replacing an earlier import, and the CatalogExporter is built from there. **Import an SDK
folder** takes a `CppSDK` you dumped somewhere else. Copied by hand, `.workspace/sdk` holds `SDK.hpp` and the `SDK`
folder directly. An SDK imported before 2026-11-13 went to `restricted/modules/rustic-isles/native/runtime`, which is
gone: import it again.

Build Dumper-7 at commit `3a57413` (main as of 2026-10-17) or older. From `42f37f6` (merged into main on
2026-10-26 as PR #588) it writes each function's lookup with its class's name instead of the function's, for
example `GetStaticName(L"BP_PlayerCharacter_C", FnName)` inside `ABP_PlayerCharacter_C::TickStamina`. That SDK
compiles, but the server dies with `Failed to find function BP_PlayerCharacter_C in BP_PlayerCharacter_C` as
soon as a player joins Ramsgate. The fix is upstream PR #592, still open on 2026-11-10; a commit after it is
fine once it is merged. The launcher's **Import SDK** repairs such a dump on the way in and logs how many
lookups it rewrote; an SDK copied in by hand is not repaired, so dump it again with a good Dumper-7.

The SDK must come from the build the server and the players run: the CatalogExporter compiles its layouts and offsets
in from it. A new dump is needed only when the game build changes.

The SDK is Phoenix Labs' material: never commit it, never share it. `.gitignore` refuses `.workspace/`,
`CppSDK/` and `SDK.hpp`.

## 3. The runtime

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

## 4. Generating the game data

The game data comes from the game in two moves: the CatalogExporter exports what the game holds, then
`dainty-host gamedata` turns the export into the files the services read.

### 4.1 The export

The CatalogExporter is the root's MIT crate `native/rustic-isles-exporter` (clean-room from
`docs/port/spec-catalog-exporter.md`), built with cargo against the same SDK. The Host page's three steps do it all:
**Build the CatalogExporter** (needs the Rust toolchain and the C++ tools' linker), **Start the game with
CatalogExporter** (injected after the runtime into the game it starts, RD9) and **Copy the exports**, which brings
them from the game folder's `Items_Analysis` into `modules/rustic-isles/game-data/1.14.7/exports/`. By hand, from the
server folder's root:

```powershell
$env:DAINTY_SDK_DIR = "$PWD\.workspace\sdk"
cargo build --release --target x86_64-pc-windows-msvc --manifest-path native\rustic-isles-exporter\Cargo.toml -p catalog-exporter-dll
dainty-launcher export --wait   # loads native\rustic-isles-exporter\target\x86_64-pc-windows-msvc\release\CatalogExporter.dll
```

In the game, the exporter waits for the game's catalog to load, writes the export and its status file, then beeps;
close the game if it stays open. [launcher-command-line.md](launcher-command-line.md) covers `dainty-launcher
export`. Built without `DAINTY_SDK_DIR`, the DLL uses the crate's made-up test SDK and exports nothing (its status
file says `built without DAINTY_SDK_DIR`). If a build against a real SDK stops with `the SDK lacks N fact(s)`, report
the fact ids it lists, never the SDK's text (`native/rustic-isles-exporter/README.md`).

The exports keep the C++ exporter's file names and formats (`catalog_1_14_7.jsonl`, the store images, the
`progression_1_14_7` folder), and the Host page's flags file turns on every export `gamedata` reads.
Mystic Paradox's C++ original was removed (tag `agpl-catalog-exporter-final`). The progression export comes from the
game the same way, and so does `weapon_slots_1_14_7.jsonl` (the live weapons' part slots): when it covers every
starter weapon, the server takes the starter kit's default weapon parts from it instead of the shipped ones
(ruling V-4).

### 4.2 `dainty-host gamedata`

```powershell
bin\dainty-host gamedata                         # generate backend\ and director\ in modules\rustic-isles\game-data\1.14.7
bin\dainty-host gamedata --progression ""        # only the progression source, from the workspace's exports
bin\dainty-host gamedata --progression C:\exports  # or from another folder of exports
bin\dainty-host gamedata --example-sources C:\src # copy supplied *.source.json files into the raw game data
bin\dainty-host gamedata --examples              # the shipped examples: the services start, nobody can play
```

`--progression` writes `backend\raw\progression_config.source.json` and stops; run `gamedata`
afterwards to build the progression config from it. A supplied source already there is kept.

`gamedata` runs the in-process Rust generators (the backend's in `crates/rustic-isles-gamedata`, the Director's hunt
tables in `crates/rustic-isles-director`) on your export. Each step runs when its input is there and is skipped, with
the reason, when it isn't; a failed step is reported and the others still run. A running server needs a restart to
read the new files.

| Step | Needs, in `exports/` unless said | Writes |
| --- | --- | --- |
| Progression config | `progression_1_14_7/player-experience-tracks.jsonl`, or a supplied `backend/raw/progression_config.source.json` | `backend/progression_config.json` |
| Director hunt tables | `hunts_1_14_7/` | the seven tables in `director/` |
| Slayer's Path graph and definitions | `slayers_path_1_14_7/player_journey_nodes.jsonl` | `backend/slayers_path.json`, `slayers_path_definitions.json` |
| Reward Cache store | `catalog_1_14_7.jsonl` | `backend/reward_cache_store.json` |
| Lady Luck store | a supplied `backend/raw/ladyluck_store.source.json`, or else the catalog and the store captures | `backend/ladyluck_store.json` |
| Platinum store and storage policy | `store_item_images_1_14_7.jsonl`, and the two stores above | `backend/platinum_store.json`, `inventory_storage_policy.json` |

The **store captures** are recordings of the official store's answers, published in a third-party repository
(EisigesEis's) that no licence lets this project copy. `modules/rustic-isles/game-data/1.14.7/store-captures.json`
pins them: the repository's URL, one commit, and each of the five files with its SHA-256. `gamedata` fetches that
commit into a bare repository under `.workspace/captures` on your machine and writes the files into the game data's
`captures/`, checking every hash. They are never committed here.

The launcher shows players whether a server runs on real or example data.

### 4.3 Moving it

Move the result as a pack, for a Linux host or a second PC:

```powershell
bin\dainty-host gamedata --export gamedata.zip
bin\dainty-host gamedata --import gamedata.zip
```

`dainty-host preset --create` keeps your generated data as a preset in `preset/` of your checkout, and
`setup` applies it to a new server; it is yours and is never committed.

## 5. What the game data is

`modules/rustic-isles/game-data/1.14.7/` holds four folders:

| Folder | What | Committed |
| --- | --- | --- |
| `backend/` | What the backend serves; `raw/` inside holds the generators' supplied sources | Only the `*.example.json` placeholders |
| `director/` | The Director's hunt and matchmaking tables | Only the `*.example.json` placeholders |
| `exports/` | The CatalogExporter's export, copied from the game | Never |
| `captures/` | The fetched store captures | Never; only the pin `store-captures.json` beside the folders |

The files the services need, and that `gamedata` makes:

- **Backend:** `progression_config.json`, `slayers_path.json`, `slayers_path_definitions.json`,
  `ladyluck_store.json`, `platinum_store.json`, `reward_cache_store.json`, `inventory_storage_policy.json`.
- **Director:** `player_hunts_table.json`, `matchmaker_hunts_table.json`, `arena_easy_matchmaker_hunts.json`,
  `arena_hard_matchmaker_hunts.json`, `arena_elite_matchmaker_hunts.json`, `arena_hard_matchmaker_hunts_new.json`,
  `arena_elite_matchmaker_hunts_new.json`.

### 5.1 Example data and real data

Each needed file has a placeholder beside it, `<name>.example.json`: made-up data, just enough for the services to
start, and the only game-data files the repository holds. `gamedata --examples` (or the Host page's **Use example
game data**) copies each missing file from its example. A server on examples starts, but nobody can play on it.

A folder's state is **missing** when a needed file is absent, **synthetic** when every file is there but some are
still their example (line endings aside), and **real** when every file is generated. `doctor`, the Host page and the
launcher's home page ("Real game data", "Synthetic game data") show it. Generating writes the real files over the
copied examples; the `*.example.json` files themselves stay.

### 5.2 The Ramsgate wishing well (`fountain_daily_bundle.json`, optional, hand-written)

The Ramsgate wishing well (a ram tossed once a day, store tag `fountain_daily_free_bundle`) needs
`backend/fountain_daily_bundle.json`, which nothing generates. When the backend game data has none, `setup` and
`gamedata` write a default that gives one Silver Cell Core (`CONTAINER_CORE_SILVER_CELLCORE`) per toss, provided the
storage policy lists it as an uncapped stack. A file you wrote is never replaced. Without a valid file the well stays
unavailable, and the backend logs why.

To change the reward, edit the file. The backend reads `{ "skus": [ <one offer> ] }` (or an array of one offer), and
refuses the file unless the offer:

- has an `id` of 1 to 128 letters, digits, `_`, `.` or `-` (it is a URL segment), used by no other store;
- has `fountain_daily_free_bundle` among its `tags`;
- has `prices` of exactly `[{ "currencyId": "id_currency_platinum", "price": 0 }]` (the client opens the coin toss only
  at a price of 0);
- has no `entitlements`, `skuProgression` or `loadoutSlots`;
- lists at least one item in `items`, each with a `catalogId`, `"instanced": false` and a whole `quantity` of 1 or
  more, and each one a **stacked, uncapped** entry in `inventory_storage_policy.json` (regenerate the policy after
  choosing a new item) that is not a prestige-gated reward currency.

### 5.3 The tester max kit (`tester_max_kit.json`, optional, hand-written)

`backend/tester_max_kit.json` is the kit the admin API's "tester max" action grants a tester account from
(`crates/rustic-isles-launcher-api/src/admin/tester_max.rs`). Nothing generates it, and without it that action
refuses.

> **Maintainer:** the backend's refusal tells the host to copy `game-data/tester_max_kit.example.json`, but no such
> example is committed, and this page doesn't document the kit's format. Add the example (or the format) and point
> this section at it.

## 6. Scope, and what is never distributed

Dainty Lass is a server and launcher for people who own Dauntless 1.14.7 to keep playing it on servers they run
themselves, after the official ones closed. It contains no game and works only with a copy installed from the
player's or host's own Epic library.

**Never in this repository, a release or a pack you share publicly:**

- the game's files, in whole or in part, and anything extracted from them beyond the services' made-up examples;
- the SDK (`CppSDK/`, `SDK.hpp`, `.workspace/sdk`), which is generated from Phoenix Labs' game;
- the exports and the generated game data, which are made from the game on each host's own machine;
- the store captures, which are third-party recordings, pinned by hash and fetched on each host;
- `.workspace/`, which also holds secrets, keys and players' data; and a host's preset.

A game-data pack (`gamedata --export`) is for moving your own data between your own machines, not for publishing.

The code is MIT OR Apache-2.0 ([`LICENSE-MIT.txt`](../LICENSE-MIT.txt), [`LICENSE-APACHE.txt`](../LICENSE-APACHE.txt)),
written clean-room (RD27): nothing here is copied from Dauntless Revived, Mystic Paradox or the Reforged Revival line.
Dauntless and its content belong to their owners; this project is not affiliated with or endorsed by Phoenix Labs or
Epic Games. The software comes with no warranty, as its licences say. Use it with a game you own, on servers you run
or are invited to, and don't use it to get at anyone else's accounts or data.

> **Maintainer:** this section is a plain statement written for this page, not legal advice. The licensing read
> before publication (DL5) should check it.
