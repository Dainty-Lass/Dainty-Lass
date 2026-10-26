<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# DaintyLass plan: one controller, game modules in their own repositories

Status: draft, written 2026-10-26. The maintainer chose the decisions in section 3 the same day, after two rounds of
questions; section 8 keeps what is still open.

This plan renames the app and splits this repository. `dauntless-eternal` (this repository, Reforged Revival)
becomes **DaintyLass**: the controller, the module framework, the core plugins and the admin API. The game code
leaves for repositories of its own: **rustic-isles** (Mystic Paradox's 1.14.7 and 1.12.0, ported to Rust, as two
game modules), **rustic-rifts** (Mystic Paradox's player-hosted realms, ported to Rust, as libraries) and
**dauntless-revived** (Dauntless Revived's 1.4.4, one game module). The launcher page is replaced by an Astro and
shadcn UI in **dainty-lass-ui**. DaintyLass mounts the other four as git submodules.

It amends [`MIGRATION_PLAN.md`](../MIGRATION_PLAN.md) where section 7 says so and leaves the rest of it standing.
The integration pass folds the amendments in; a row here never edits that file (AGENTS.md).

Row ids: **DS** = repositories, split and rename, **DF** = module framework and controller, **DU** = UI,
**RU** = rustic-isles, **RF** = rustic-rifts, **RV** = dauntless-revived, **DX** = cross-cutting, **DD** =
decisions. "DR" still means Dauntless Revived at `e1c5fec`, and the RB, RA, RS, RH and RD ids point into
`MIGRATION_PLAN.md`.

## 1. Where things stand

Measured 2026-10-26.

| What | State |
| --- | --- |
| This repository | `main` at `c57f157`: one cargo workspace with the controller (`crates/controller`), service libraries (gateway with the allowlist, content, admin, host kit; backend and Director are stubs), DR's 1.4.4 `metagame` and `deploy` behind the `legacy` feature, 12 edition binary crates (`Refined` = 1.4.4, `Reforged` = 1.14.7). Paradox's Node backend and Director at `paradox/` (`00d3be8` + 33 patches + fixes since, such as RD25's kick route). C++ in `native/`: `paradox-runtime`, `runtime-loader`, `catalog-exporter`, DR's `internal-server` and `client-mods` |
| Forgejo | `git.silentspud.com`, Forgejo 15.0.10, with access to the instance restricted. One repository, `silentspud/dauntless-eternal` (public on the instance), reached as `origin` over SSH on port 222 |
| Mystic Paradox | Upstream `main` at `21b9ff5` (`mystic` remote, `pranav158/Mystic-Paradox`). The local `replay` branch is `21b9ff5` plus Reforged's patches 0001–0039 as one commit each and their follow-ups (`ce1c42e`, 41 commits). The 1.12.0 release is the tag `dauntless-1.12.0` (`4d5dfaa`), an ancestor of `main` 91 commits back |
| Paradox's size (TS, JS and C++, tests included) | 1.12.0: backend 16.8k lines, Director 1.7k, runtime 11.4k. `replay`: backend 103.5k, Director 4.5k, runtime 29.5k, `ParadoxTransport` 8.8k |
| Realms on `replay` | Players host hubs and hunts on their own PCs; joiners connect through `ParadoxTransport.dll` (ICE on libjuice: direct, or through a TURN relay). A self-hosted backend brings its own relay (Cloudflare TURN or a `turn-rest` server such as coturn) and an admission key pair. Mystic's anti-cheat and Steam transport are private, so Guard runs on stubs in observe mode. In the backend, `p2p/`, `realms/` and Guard are about 2.0 of its 5.1 MiB of source (210 files mention realms) |
| Dauntless Revived | `SilentSpud/dauntless-revived` on GitHub: `plan` at `24b7624` is upstream `main` (`5eb74ff`, `mixutin/dauntless-revived`) plus 468 commits, DR's Rust migration among them. This repository carries pieces of it with Reforged's changes: merge `54e271a` (commits `c1be7f7`…`18ac587`) |
| Rust Director | `feat/rb10-director-single-binary` (`cdc0a87`): RB10, the Director in Rust, never merged; based on the old `plan` layout |

## 2. End state

```text
silentspud/dainty-lass            (renamed from dauntless-eternal)
  rust/
    crates/module-api             the framework: GameVersion, manifests, traits, the out-of-process protocol
    crates/controller             the registry, lifecycle, commands and events
    crates/protocol, proxy, service-kit, store (Postgres)
    plugins/gateway (allowlist), content, admin, host, launcher (Tauri shell)
    bins/server, server-console, server-gui, client
  native/runtime-loader           winmm.dll, shared by every game's client payload
  ui/                             submodule: silentspud/dainty-lass-ui
  modules/rustic-isles/           submodule: silentspud/rustic-isles
  modules/rustic-rifts/           submodule: silentspud/rustic-rifts
  modules/dauntless-revived/      submodule: silentspud/dauntless-revived
  docs/ docker/ brand/

silentspud/rustic-isles           Mystic Paradox's history, `replay` as main
  crates/isles-*                  one Rust backend and Director for both versions
  modules/isles-1-14-7            game module "Rustic Isles 1.14.7", links rustic-rifts' server libraries
  modules/isles-1-12-0            game module "Rustic Isles 1.12"
  native/runtime/                 MysticParadox.dll per version; catalog-exporter
  reference/                      the Node services each version is ported from, until the replay passes
  preset/                         1.14.7 game data (from dainty-lass)

silentspud/rustic-rifts           realms, one library per component
  crates/rifts-admission, rifts-relay, rifts-realms, rifts-guard   server side, used by isles-1-14-7
  crates/rifts-host               client side, used by DaintyLass's launcher
  native/transport/               ParadoxTransport.dll, with its history

silentspud/dauntless-revived      DR's history, SilentSpud's `plan` as main
  modules/revived-1-4-4           game module "Dauntless Revived": metagame, deploy
  native/internal-server, client-mods

silentspud/dainty-lass-ui         Astro + shadcn monorepo from the preset
```

## 3. Decisions

| # | Topic | Decision | Consequence |
| --- | --- | --- | --- |
| DD1 | Name | The app is **DaintyLass**; the repository `dainty-lass`, renamed on Forgejo so the old URL redirects. Names people read change: README, docs, window titles, both languages' strings, crate and binary names. **Names an install depends on stay as they are**, with no migration planned: the `dauntless://` invite scheme and the `reforged-revival://` refusal, `RRK_` keys, the launcher's identifier and `userData` folder (the stored keys live there), env prefixes (`REFORGED_` and the rest), the `x-reforged-*` headers, the `/reforged/api` routes, the nftables table, the certificate and every invite's `fp`, and the file names the game loads (`MysticParadox.dll`, `ParadoxTransport.dll`, `winmm.dll`) | No host or player redoes anything on update. Players and hosts never read those names; renaming one would need a migration for no visible gain |
| DD2 | Repositories | **Five**, on `git.silentspud.com` under `silentspud`, all public on the instance. DaintyLass pins the other four as submodules at `ui/` and `modules/<name>/`. Module repositories never depend on DaintyLass's git history: they depend on `module-api`, `protocol`, `service-kit` and `store` by version from Forgejo's Cargo registry, and DaintyLass's workspace `[patch]`es those to its own paths, so a checkout with submodules builds as one tree. The module repositories sit under `exclude` in DaintyLass's workspace and keep workspaces of their own. rustic-isles takes rustic-rifts the same way | No circular git dependency; a module builds alone against a released API, or inside DaintyLass against the working copy. DF1 proves the cargo layout before anything depends on it. Public repositories also let CI and clones fetch submodules without a token |
| DD3 | Module kinds | **Core plugins** add capabilities other modules use: gateway with the allowlist, content, store, admin API, host kit commands, the Tauri shell, the console and TUI. They live in DaintyLass and are compiled in, picked by each binary's features. **Game modules** carry everything a server for one game version needs and use the core plugins only through `ModuleContext`: they register routes behind the gateway, UDP ranges with the allowlist, files with content, commands, events, admin actions (kick, ban, lookup) and config sections | One gateway, one allowlist and one content server per host, whatever games it runs. A game module never opens a public listener (AGENTS.md: the gateway is the only one off loopback) |
| DD4 | Game module identity | Each game module declares an **exact game version** (`GameVersion`: dot-separated integers, compared exactly, no ranges), a display name, a module id, the invite `version` aliases it answers to (`revived`, `reforged`), its module API version, its default ports and its capabilities (realms, for one). The controller keys modules by game version and **refuses a second module for a version that is already loaded**, compiled in or not. The display names: **"Rustic Isles 1.14.7"**, **"Rustic Isles 1.12"** (Rustic Isles' names always carry the version) and **"Dauntless Revived"** | Replaces `Edition`, `EditionSelection` and `Game` (`protocol`): an invite's `version` already names a game version, so it maps straight to a module. Port defaults come from manifests, and the controller moves a clash instead of hard-coding 61000/61010 |
| DD5 | Loading at runtime | **Out of process.** A game module crate is a library (compiled in) and, through `module-api`'s adapter, an executable the controller starts and talks to over the child's stdin and stdout: length-prefixed frames, a handshake carrying the manifest and API version, then calls both ways. The same `GameModule` trait backs both, so a module is written once. Unload: a stop request, a grace period, then the process tree is killed (job object on Windows, process group on Linux, as `start` already does). Only modules named in `host.toml` with their SHA-256 can be loaded; the admin API and the console can load and unload those, never an arbitrary path | A crashing module doesn't take the controller down, and no Rust ABI is involved. Every call crosses a pipe, which costs nothing next to the game's HTTP. The frame parser is a trust boundary with a fuzz target (DX2) |
| DD6 | Paradox's backend and Director | **Ported to Rust now, inside rustic-isles** (realms in rustic-rifts, DD14). The RB rows move there as RU rows, and RD17's gate no longer holds for them. Until the port passes its replay, today's `main` keeps shipping and hosts keep running Node Paradox; the 1.14.7 module ships only once its Rust services pass | The largest part of this plan: about 108k lines of TS and JS for 1.14.7 against 18.5k for 1.12.0, of which realms are about 40% |
| DD7 | rustic-isles' base and name | **The `replay` branch**: Mystic `main` at `21b9ff5` with Reforged's 39 patches, pushed with its history as rustic-isles' `main`; the `dauntless-1.12.0` tag comes with it. Upstream `main` is kept as a branch for later cherry-picks. Fixes made on this repository's `paradox/` since RP2 that `replay` lacks are listed (DS5) and applied first. **No new name borrows from "Mystic Paradox"** (its additional terms, sections 3 and 4): repositories, crates and display names are Rustic Isles' and Rustic Rifts'. Mystic Paradox is named only in truthful attribution (DX4) | The latest Mystic, nothing Reforged fixed lost, and no name that could read as affiliated with Mystic Paradox |
| DD8 | One codebase for 1.14.7 and 1.12.0 | The Rust port is written once, from the 1.14.7 Node code. 1.12.0's differences come from diffing the tag's Node services against the same files on `replay`, and each becomes a version hook or a data table in the shared crates. The per-version module crates stay thin: manifest, hooks, tables, Director config. The C++ runtime is built per version from each host's SDK; folding the two runtime trees into one source is a spike (RU10), not a promise | 1.12.0 costs a route inventory, a diff and its hooks, not a second port |
| DD9 | 1.12.0's verification | **Built, verified later.** "Rustic Isles 1.12" is ported against its Node tests and the route diff only, and its manifest marks it unverified until someone records 1.12.0 traffic for a replay. It never gates the 1.14.7 cutover | The UI shows it as unverified; hosts can still load it |
| DD10 | dauntless-revived's base | **SilentSpud's fork** (`plan`, `24b7624`) pushed with its history, upstream `main` kept as a branch. Then what this repository changed in its copy of DR (DS5) is brought across. The module is DR's `metagame`, `deploy`, `internal-server` and `client-mods`; DR's own gateway, content, launcher, `discord-bot` and `friend-kit` are dropped from the build, since DaintyLass's core plugins and admin API replace them | DR's 1.4.4 server keeps its history and its Reforged changes; one gateway serves every game |
| DD11 | The UI | **A new repository, `dainty-lass-ui`**, made by `bunx --bun shadcn@latest init --preset b1cYwfJGNu --template astro --monorepo` and mounted at `ui/`. Tauri stays the shell and serves Astro's static build. The bridge's generated TS types are written into the submodule (`RR_UPDATE_GENERATED=1`), and DaintyLass's staleness test reads them there. Both language catalogs move to the UI repository, and `gen/strings.json` is generated from them. Module-specific screens are data: the UI draws a game module's commands, status and settings from its manifest, so no module ships UI code. Realm hosting is the one UI that is code, shown for modules with the realms capability | Section 6's "the page's look word for word" no longer holds; both languages and every string do. Bun joins the build. The Electron launcher in `legacy/` is untouched until RA16 |
| DD12 | Binaries | The 12 edition crates **collapse to four**: `server`, `server-console`, `server-gui`, `client`, each with one Cargo feature per compiled-in game module and one for realms. The release builds every module in. The player `client` links no game server code: what it needs per game (the executable hash, the command line, the ini edits, which files to fetch) comes from the module's client profile, served by content. With the realms feature it links `rifts-host` | Fewer crates and one feature list. The launcher's legacy 1.4.4 paths become a client profile from the revived module |
| DD13 | Native code | **`runtime-loader` stays in DaintyLass** at `native/runtime-loader`, with Paradox's attribution: the launcher's PLAY check (RD9) and every game's client DLLs go through it, 1.4.4's included (`920a411`). `paradox-runtime`, `catalog-exporter` and `preset/` go to rustic-isles, `ParadoxTransport` to rustic-rifts, `internal-server` and `client-mods` to dauntless-revived. RD16 stands: until its cut-off Freeza's Paradox patches land in rustic-isles, after it only runtime patches, still there | No module depends on another game module's repository. The runtime patch method (`docs/paradox-runtime-patches.md`) moves with the runtime |
| DD14 | Realms | **In scope, in their own repository, `rustic-rifts`**: a library monorepo, one crate per component, ported from `replay`'s `p2p/`, `realms/` and Guard code and its `ParadoxTransport`. Rifts is neither a core plugin nor a game module: realms exist only on 1.14.7 and share its accounts, sessions and hunt data, so `isles-1-14-7` links the server crates and declares the realms capability, and DaintyLass's launcher links `rifts-host` for hosting and joining. Admission, relay credentials and leases are admin actions and commands of the 1.14.7 module. Guard keeps observe mode on stubs, since Mystic's anti-cheat is private. A host chooses a TURN relay (Cloudflare TURN or a `turn-rest` server such as coturn) | Realms ride the module framework without a third module kind. About 40% of the backend port moves here, and the UI gains a hosting flow. Hosts that turn realms on need a TURN relay; the rest need nothing new |
| DD15 | Database | **Postgres for every module.** `store` moves to `PgConnection` (RB3's spike for `pq-sys` `bundled` first) and is shared through `module-api`; the host kit manages a local Postgres or takes `--postgres-url` (RD15, RH3). DR's metagame leaves SQLite: its schema becomes Postgres migrations, and `revived db migrate --from-sqlite` moves an existing 1.4.4 database once | One engine to install, back up and support. Every host, 1.4.4-only ones included, runs Postgres. What the realm host's PC stores locally is section 8's question 1 |

## 4. Work

### 4.1 Repositories, split and rename (DS)

| # | What | Verify |
| --- | --- | --- |
| DS1 | Rename `silentspud/dauntless-eternal` to `dainty-lass` on Forgejo; point every clone's `origin` at the new URL (maintainer) | The old URL redirects; `git ls-remote` works on both |
| DS2 | Create `silentspud/rustic-isles` (public): push `replay` as `main`, the `dauntless-1.12.0` and `v1.12-final` tags, Mystic's `main` as `upstream/main`; `mystic` stays a fetch-only remote | `git log` shows Mystic's history; the tag resolves to `4d5dfaa` |
| DS3 | Create `silentspud/dauntless-revived` (public): push SilentSpud's `plan` as `main`, `mixutin`'s `main` as `upstream/main` | `git log` shows DR's history to `24b7624` |
| DS4 | Create `silentspud/dainty-lass-ui` (public) from the shadcn command (DD11); commit the generated tree untouched as the first commit | `bun install` and `bun run build` pass |
| DS5 | Catch-up lists, read-only: this repository's commits touching `paradox/`, `native/paradox-runtime`, `native/catalog-exporter` and `preset/` that `replay` lacks (`docs/port/catchup-isles.md`); its DR-derived commits (`c1be7f7`…`18ac587` and later ones touching `metagame`, `deploy`, `native/internal-server`, `native/client-mods`) as `docs/port/catchup-revived.md` | Every commit is listed as needed or not applicable, with the reason |
| DS6 | Swap the code for submodules: delete `paradox/`, `native/paradox-runtime`, `native/catalog-exporter`, `native/internal-server`, `native/client-mods`, `preset/`, `rust/services/metagame`, `rust/services/deploy`; add the module submodules; path sweep of the host kit, Docker, CI and docs. Lands once RU1 and RV1 hold that code, and before the Rust port, so hosts keep running what they run today | `cargo test`, `npm test` in the moved Node services, `bash docker/run-tests.sh` pass from a fresh `git clone --recurse-submodules` |
| DS7 | Create `silentspud/rustic-rifts` (public) from rustic-isles with `git subtree split` of `ParadoxTransport`, so the transport keeps its history; the Rust crates start empty | `git log` in the new repository shows the transport's commits |

### 4.2 Module framework and controller (DF)

| # | What | Verify |
| --- | --- | --- |
| DF1 | **Spike**: the cargo layout of DD2. A module workspace under `exclude`, used as a path dependency; a crate on Forgejo's Cargo registry `[patch]`ed to a path; `workspace = true` inheritance inside the submodule; on Windows and Linux. Kill criterion: if the registry or the `[patch]` fails, modules take `module-api` as a git dependency on a tag | A toy module builds both alone and inside DaintyLass |
| DF2 | `crates/module-api`, the seam: `GameVersion`, `ModuleManifest` (capabilities, the unverified mark), the `GameModule` and `CorePlugin` traits, `Registrar` (routes, ports, commands, events, admin actions, config sections), `ModuleContext`; the frame protocol and its adapters for both sides; every capability registered as a stub (RD18's seam rule) | A stub module runs compiled in and out of process and gives the same answers; fuzz target `module_frame` |
| DF3 | Controller: the registry keyed by game version with the duplicate refusal (DD4), load and unload, the `[modules]` section of `host.toml` (compiled-in list, external modules by path and SHA-256), commands into the console, TUI, server GUI and admin API, the event bus, port assignment, logs and status from out-of-process modules into the tracing layers. `Edition`, `EditionSelection` and `Game` removed | Two modules for one version refused in either order; a module killed mid-run shows as failed and restarts on command; nothing left running after stop |
| DF4 | Core plugins on the registry: the gateway routes by game version to the module's upstream (the invite's `version`, then the route), the allowlist guards the union of the modules' UDP ranges, content keys runtime manifests and client profiles by version (`edition=` still accepted, a request without one still gets 1.14.7), the admin API sends kick, ban and lookup to the module of the player's game, the host kit's commands gain `module list`, `load`, `unload` | The gateway and content parity tests pass unchanged; RD25's admin API cases pass against every game |
| DF5 | Bins: four crates with one feature per module and one for realms (DD12); the release workflow builds them (`server-release.yml` owner) | Each bin starts with zero, one and every module |
| DF6 | Rename sweep (DD1): crate and binary names, window titles, docs, both languages' strings; the contract names DD1 keeps are left as they are | No `Reforged Revival` or `dauntless-eternal` left outside history, the catch-up lists and the contract names |
| DF7 | `store` on Postgres (DD15): RB3's `pq-sys` `bundled` spike on MSVC and Linux, `PgConnection`, the SQLite-only parts dropped; the host kit's managed Postgres (RH3) and Docker's Postgres container | `cargo test -p store` on both OSes against a real Postgres |

### 4.3 UI (DU)

| # | What | Verify |
| --- | --- | --- |
| DU1 | Tauri serves the UI: `frontendDist` at the Astro build, the CSP and the isolation hook checked against Astro's output (no inline script Tauri's policy refuses), bridge types generated into `ui/` | The empty UI opens in the Tauri debug build and a bridge call round-trips |
| DU2 | Player screens: servers, invites, PLAY with the runtime acceptance, settings, the game folder; both languages moved over with every key | The launcher tests that drive the page pass; `i18n` checks pass on the new catalogs |
| DU3 | Server screens: the Host page, the per-component status page, module panels drawn from manifests (DD11) with the unverified mark, module load and unload | A server GUI build with every module repository runs one of each game |
| DU4 | Drop `rust/launcher/vite.config.ts`'s build of `legacy/ReforgedLauncher` | No Tauri build reads `legacy/` |
| DU5 | Realm screens: host a hub or hunt, join one, the relay status; shown only for modules with the realms capability | Two launchers on two PCs host and join a realm (maintainer, RF8) |

### 4.4 rustic-isles (RU)

The RB rows of `MIGRATION_PLAN.md` section 4.5, moved here (DD6) and done for both versions. Realms are left to the
RF rows: an area that touches them calls through rustic-rifts' crates.

| # | What | From | Verify |
| --- | --- | --- | --- |
| RU1 | Lay out the monorepo with `git mv`: the Node services to `reference/1.14.7/`, `ParadoxRuntime` to `native/runtime/`, `catalog-exporter` and `preset/` brought from DaintyLass; `ParadoxLauncher` deleted, `ParadoxTransport` deleted once DS7 holds it; `ADDITIONAL_TERMS.md`, `NOTICE.md` and the licence kept at the top; DS5's isles list applied | RP3 | The Node tests pass from the new paths |
| RU2 | 1.12.0's Node services and runtime from the tag into `reference/1.12.0/` | — | The tag's own tests pass there |
| RU3 | Route inventory of both versions and their diff (DD8), each route marked isles or rifts | RB1 | Every Express route in each inventory; CI fails on a new route without an entry |
| RU4 | Cargo workspace and seams: `isles-core` (one trait per repository, Postgres on `store`), `isles-chat`, `isles-director`, the two module crates with their manifests and hooks; every route answering 501 | RB5 | Both modules load into DaintyLass and answer 501 |
| RU5 | Areas, one agent each, both versions: auth, the Reforged API, game areas (five agents, as RB8), chat, game data and scripts | RB4, RB6–RB9, RB11 | Each area's Node tests ported, for both versions where 1.12.0 has the area |
| RU6 | Director in Rust, starting from `cdc0a87` | RB10 | Ramsgate, the Training Grounds and a hunt start and stop, on both versions (maintainer) |
| RU7 | `db migrate` from MongoDB and `db verify` | RB4 | Copies of real databases migrate with no difference |
| RU8 | Shadow replay for 1.14.7, realms included | RB2, RB12 | No unexplained diff |
| RU9 | Rollback, cutover and Node removal | RB13–RB15 | As in those rows |
| RU10 | **Spike**: one runtime source tree for both versions, SDK glue apart | — | Kill criterion: if the shared part is under half the code, the two trees stay apart |
| RU11 | 1.12.0's replay, when someone records 1.12.0 traffic (DD9); clears the unverified mark | RB12 | No unexplained diff |

### 4.5 rustic-rifts (RF)

| # | What | Verify |
| --- | --- | --- |
| RF1 | Inventory of realms: the `p2p/`, `realms/` and Guard routes (RU3's rifts marks), the runtime's hosting hooks, the transport's ABI 2 and 3, the `P2P_*` settings, what the host's PC stores | Every route, setting and ABI export listed with its component |
| RF2 | Cargo workspace and seams: `rifts-admission`, `rifts-relay`, `rifts-realms`, `rifts-guard`, `rifts-host`, each a trait surface with stubs; `isles-1-14-7` calls them | The 1.14.7 module builds with rifts stubbed and without them |
| RF3 | Transport build: `ParadoxTransport.dll` and its self-test in CI (MSVC v145, libjuice pinned by SHA-256) | `ParadoxTransportTest.exe all --relay-local` passes in CI |
| RF4 | Admission (tickets, the key pair, its generator as a command) and relay credentials (Cloudflare TURN, `turn-rest`) | The transport's self-test passes against Rust-minted tickets; credentials from both providers |
| RF5 | Realms, leases and hosting tiers on Postgres | `realms/` and `p2p/` Node tests ported |
| RF6 | Guard in observe mode on stubs, with its evidence chain intake | The Guard tests that don't need the private module ported |
| RF7 | `rifts-host`: the launcher's side of hosting and joining, behind DaintyLass's realms feature | A realm hosted from the debug launcher on one PC |
| RF8 | Two-PC check across NAT, direct and through a relay (maintainer) | A hub and a hunt hosted and joined both ways |

### 4.6 dauntless-revived (RV)

| # | What | Verify |
| --- | --- | --- |
| RV1 | DS5's revived list applied; `metagame` and `deploy` on `module-api`; module crate `revived-1-4-4` (display name "Dauntless Revived", game version 1.4.4, alias `revived`); DR's gateway, content, launcher, `discord-bot` and `friend-kit` out of the build | The metagame's tests pass; the module loads into DaintyLass |
| RV2 | The 1.4.4 client profile (DD12): executable hash, command line, the DLL through the runtime loader | The launcher starts 1.4.4 against a DaintyLass server (maintainer) |
| RV3 | The metagame on Postgres (DD15): its schema as Postgres migrations, `revived db migrate --from-sqlite` and a `verify` that compares the two | The metagame's tests pass on Postgres; a copy of a real 1.4.4 database migrates with no difference |

### 4.7 Cross-cutting (DX)

| # | What |
| --- | --- |
| DX1 | Docs: `architecture.md` for modules and core plugins, the hosting guides (Postgres for every host, realms and their relay), the migration guide's paths, a "writing a game module" page |
| DX2 | Security (RX3): `docs/security/module-host.md` for the frame protocol, external module loading and the SHA-256 list; fuzz target `module_frame`. Each module repository keeps the checklists for what it carries (`metagame.md`, `deploy.md` move to dauntless-revived; rustic-rifts gets admission tickets, the pair cipher and relay credentials, with fuzz targets for the ticket and control-message parsers) |
| DX3 | CI on Forgejo Actions: a runner with MSVC for the native builds (maintainer), submodule checkout, Bun; `.github/workflows` read as Forgejo reads them, or moved to `.forgejo/workflows`. GitHub stays a read-only source (`upstream`, `mystic`) until DX5 |
| DX4 | Licensing: AGPL-3.0-only in every repository; Mystic's `ADDITIONAL_TERMS.md` and its section 1 notice in rustic-isles' and rustic-rifts' README and `NOTICE.md`, which also say they are modified, unofficial versions (its section 2); DR's attribution in dauntless-revived; libjuice's MPL-2.0 notice in rustic-rifts; `NOTICE.md` here points to all four |
| DX5 | Publishing to GitHub, once the launcher and the server are confirmed working (maintainer): each repository pushed public, submodule URLs and the Cargo registry switched or mirrored, release workflows pointed at it |

## 5. Order

| Wave | Rows | Needs | Hands-on |
| --- | --- | --- | --- |
| 0 | DS5 lists; DF1 spike; DF7's `pq-sys` spike | — | — |
| 0 | DS1–DS4 | — | Forgejo: rename, create the repositories, push |
| 1 | DF2 seam; RU1, RU2; DS7; RV1's history work; DU1 | DS2–DS4, DF1 | — |
| 2 | DS6 submodule swap | RU1, RV1, DS7 | — |
| 2 | DF3 controller, DF4 core plugins, DF7 store; RU3 inventories, RF1 | DF2 | — |
| 3 | RU4 and RF2 seams; RV1's module crate; DF5 bins; DU2, DU3 | DF3, DF4, DF7 | — |
| 3 | DF6 rename sweep, by itself, between integration passes | DS6 | — |
| 4 | RU5 (about nine agents), RU6, RU7; RF3–RF6; RV2, RV3 | RU4, RF2 | Game servers, the 1.4.4 client |
| 5 | RF7, DU5, RF8 | RF3–RF6 | Two PCs, a TURN relay |
| 6 | RU8 replay, then RU9 | RU5–RU7, RF5–RF6 | Canary, 1.14.7 captures |
| any | RU10 spike; RU11 when 1.12.0 traffic exists; DX rows with the rows they cover | — | Host SDKs for both versions |
| last | DX5 GitHub publication | The launcher and server confirmed working | Maintainer |

Today's `main` keeps shipping until DS6. After DS6, hosts update to a DaintyLass build that still runs Node Paradox
from the rustic-isles submodule; the Rust 1.14.7 module replaces it at RU9's cutover.

## 6. Things that must not change

`MIGRATION_PLAN.md` section 6 holds, with two changes: the launcher page's look is replaced (DD11), its strings in
both languages are not; and "the host-only routes and headers stay refused" now covers every module's routes. Also:

- An invite made before the split joins the same server after it: scheme, `version`, `fp`.
- A host's `.env` files and `host.toml` read unchanged; the module section has defaults that reproduce today's
  server.
- Each game version's routes and JSON, matched as its Node service answers them.
- Every name DD1 keeps.

## 7. Amendments to `MIGRATION_PLAN.md`

For the integration pass: RD1 (Paradox becomes a submodule, no longer a subtree at `paradox/`), RD11 and section
4.1 (the layout in section 2 here), RD15 (Postgres for every module, not only the Paradox port, DD15), RD17 (no gate
for the Paradox port, DD6), section 4.5 (the RB rows move to RU and RF), RP rows (their results move to
rustic-isles), RR5 (the page moves to `ui/`, not `rust/launcher/ui`), section 5.1's ownership table (the module
repositories and `ui/` own their own files; `crates/module-api` is a seam owned by DF2).

## 8. Open questions

Answered 2026-10-26: display names (DD4), Mystic's name clause (DD7), how deep the rename goes (DD1), the database
(DD15), 1.12.0's verification (DD9), realms (DD14) and visibility (DD2, DX5).

1. **The realm host's local data.** Mystic's guide says "local realms use the built-in `node:sqlite`". If that is a
   player's PC hosting a realm, Postgres for every module (DD15) would put Postgres on players' PCs. RF1 finds out
   what is stored there; the choice is then SQLite for that case only, or keeping the state on the server.
2. **A relay for the canary.** Realm checks (RF8) need a TURN relay: a Cloudflare account, or a coturn the
   maintainer runs.
