<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# DaintyLass plan: one controller, game modules in their own repositories

Status: draft, written 2026-10-26. The decisions in section 3 marked "chosen" were made by the maintainer the same
day; the rest are proposals waiting on section 8's questions.

This plan renames the app and splits this repository. `dauntless-eternal` (this repository, Reforged Revival)
becomes **DaintyLass**: the controller, the module framework, the core plugins and the admin API. The game code
leaves for two repositories of its own, each publishing game modules: **rustic-paradox** (Mystic Paradox's
1.14.7 and 1.12.0, ported to Rust) and **dauntless-revived** (Dauntless Revived's 1.4.4). The launcher page is
replaced by an Astro and shadcn UI in a fourth repository. DaintyLass mounts the other three as git submodules.

It amends [`MIGRATION_PLAN.md`](../MIGRATION_PLAN.md) where section 7 says so and leaves the rest of it standing.
The integration pass folds the amendments in; a row here never edits that file (AGENTS.md).

Row ids: **DS** = repositories, split and rename, **DF** = module framework and controller, **DU** = UI,
**RU** = rustic-paradox, **RV** = dauntless-revived, **DX** = cross-cutting, **DD** = decisions. "DR" still means
Dauntless Revived at `e1c5fec`, and the RB, RA, RS, RH and RD ids point into `MIGRATION_PLAN.md`.

## 1. Where things stand

Measured 2026-10-26.

| What | State |
| --- | --- |
| This repository | `main` at `c57f157`: one cargo workspace with the controller (`crates/controller`), service libraries (gateway with the allowlist, content, admin, host kit; backend and Director are stubs), DR's 1.4.4 `metagame` and `deploy` behind the `legacy` feature, 12 edition binary crates (`Refined` = 1.4.4, `Reforged` = 1.14.7). Paradox's Node backend and Director at `paradox/` (`00d3be8` + 33 patches + fixes since, such as RD25's kick route). C++ in `native/`: `paradox-runtime`, `runtime-loader`, `catalog-exporter`, DR's `internal-server` and `client-mods` |
| Forgejo | `git.silentspud.com`, Forgejo 15.0.10. One repository, `silentspud/dauntless-eternal` (public), reached as `origin` over SSH on port 222 |
| Mystic Paradox | Upstream `main` at `21b9ff5` (`mystic` remote, `pranav158/Mystic-Paradox`). The local `replay` branch is `21b9ff5` plus Reforged's patches 0001–0039 as one commit each and their follow-ups (`ce1c42e`, 41 commits). The 1.12.0 release is the tag `dauntless-1.12.0` (`4d5dfaa`), an ancestor of `main` 91 commits back |
| Paradox's size (TS, JS and C++, tests included) | 1.12.0: backend 16.8k lines, Director 1.7k, runtime 11.4k. `replay`: backend 103.5k, Director 4.5k, runtime 29.5k, `ParadoxTransport` (ICE, realms) 8.8k |
| Dauntless Revived | `SilentSpud/dauntless-revived` on GitHub: `plan` at `24b7624` is upstream `main` (`5eb74ff`, `mixutin/dauntless-revived`) plus 468 commits, DR's Rust migration among them. This repository carries pieces of it with Reforged's changes: merge `54e271a` (commits `c1be7f7`…`18ac587`) |
| Rust Director | `feat/rb10-director-single-binary` (`cdc0a87`): RB10, the Director in Rust, never merged; based on the old `plan` layout |

## 2. End state

```text
silentspud/dainty-lass            (renamed from dauntless-eternal)
  rust/
    crates/module-api             the framework: GameVersion, manifests, traits, the out-of-process protocol
    crates/controller             the registry, lifecycle, commands and events
    crates/protocol, proxy, service-kit, store
    plugins/gateway (allowlist), content, admin, host, launcher (Tauri shell)
    bins/server, server-console, server-gui, client
  native/runtime-loader           winmm.dll, shared by every game's client payload
  ui/                             submodule: silentspud/dainty-lass-ui
  modules/rustic-paradox/         submodule: silentspud/rustic-paradox
  modules/dauntless-revived/      submodule: silentspud/dauntless-revived
  docs/ docker/ brand/

silentspud/rustic-paradox         Mystic Paradox's history, `replay` as main
  crates/paradox-*                one Rust backend and Director for both versions
  modules/paradox-1-14-7          game module, game version 1.14.7
  modules/paradox-1-12-0          game module, game version 1.12.0
  native/runtime/                 MysticParadox.dll per version; catalog-exporter
  reference/                      the Node services each version is ported from, until the replay passes
  preset/                         1.14.7 game data (from dainty-lass)

silentspud/dauntless-revived      DR's history, SilentSpud's `plan` as main
  modules/revived-1-4-4           game module, game version 1.4.4: metagame, deploy
  native/internal-server, client-mods

silentspud/dainty-lass-ui         Astro + shadcn monorepo from the preset
```

## 3. Decisions

| # | Topic | Decision | Consequence |
| --- | --- | --- | --- |
| DD1 | Name | The app is **DaintyLass**; the repository `dainty-lass`, renamed on Forgejo so the old URL redirects. Names people read change: README, docs, window titles, both languages' strings, crate and binary names. **Names an install depends on stay** until a row of their own moves them with a migration: the `dauntless://` invite scheme and the `reforged-revival://` refusal, `RRK_` keys, the launcher's identifier and `userData` folder (the stored keys live there), env prefixes (`REFORGED_` and the rest), the `x-reforged-*` headers, the `/reforged/api` routes, the nftables table, the certificate and every invite's `fp` | No host or player redoes anything on update. Which contract names move later is section 8's question 3 |
| DD2 | Repositories | **Four**, on `git.silentspud.com` under `silentspud` (chosen). DaintyLass pins the other three as submodules at `ui/`, `modules/rustic-paradox/`, `modules/dauntless-revived/`. Game repositories never depend on DaintyLass's git history: they depend on `module-api`, `protocol` and `service-kit` by version from Forgejo's Cargo registry, and DaintyLass's workspace `[patch]`es those to its own paths, so a checkout with submodules builds as one tree. The module repositories sit under `exclude` in DaintyLass's workspace and keep workspaces of their own | No circular git dependency; a module builds alone against a released API, or inside DaintyLass against the working copy. DF1 proves the cargo layout before anything depends on it |
| DD3 | Module kinds | **Core plugins** add capabilities other modules use: gateway with the allowlist, content, store, admin API, host kit commands, the Tauri shell, the console and TUI (chosen: they live in DaintyLass). They are compiled in, picked by each binary's features. **Game modules** carry everything a server for one game version needs and use the core plugins only through `ModuleContext`: they register routes behind the gateway, UDP ranges with the allowlist, files with content, commands, events, admin actions (kick, ban, lookup) and config sections | One gateway, one allowlist and one content server per host, whatever games it runs. A game module never opens a public listener (AGENTS.md: the gateway is the only one off loopback) |
| DD4 | Game module identity | Each game module declares an **exact game version** (`GameVersion`: dot-separated integers, compared exactly, no ranges), a display name ("Dauntless Revived", "Rustic Paradox"), a module id, the invite `version` aliases it answers to (`revived`, `reforged`), its module API version and default ports. The controller keys modules by game version and **refuses a second module for a version that is already loaded**, compiled in or not | Replaces `Edition`, `EditionSelection` and `Game` (`protocol`): an invite's `version` already names a game version, so it maps straight to a module. Port defaults come from manifests, and the controller moves a clash instead of hard-coding 61000/61010 |
| DD5 | Loading at runtime | **Out of process** (chosen). A game module crate is a library (compiled in) and, through `module-api`'s adapter, an executable the controller starts and talks to over the child's stdin and stdout: length-prefixed frames, a handshake carrying the manifest and API version, then calls both ways. The same `GameModule` trait backs both, so a module is written once. Unload: a stop request, a grace period, then the process tree is killed (job object on Windows, process group on Linux, as `start` already does). Only modules named in `host.toml` with their SHA-256 can be loaded; the admin API and the console can load and unload those, never an arbitrary path | A crashing module doesn't take the controller down, and no Rust ABI is involved. Every call crosses a pipe, which costs nothing next to the game's HTTP. The frame parser is a trust boundary with a fuzz target (DX2) |
| DD6 | Paradox's backend and Director | **Ported to Rust now, inside rustic-paradox** (chosen). The RB rows move there as RU rows, and RD17's gate no longer holds for them. Until the port passes its replay, today's `main` keeps shipping and hosts keep running Node Paradox; the 1.14.7 module ships only once its Rust services pass | The largest part of this plan: about 108k lines of TS and JS for 1.14.7 against 18.5k for 1.12.0. RD15 (Postgres) still applies unless section 8's question 4 changes it |
| DD7 | rustic-paradox's base | **The `replay` branch** (chosen): Mystic `main` at `21b9ff5` with Reforged's 39 patches, pushed with its history as rustic-paradox's `main`; the `dauntless-1.12.0` tag comes with it. Upstream `main` is kept as a branch for later cherry-picks. Fixes made on this repository's `paradox/` since RP2 that `replay` lacks are listed (DS5) and applied first | The latest Mystic, nothing Reforged fixed lost. Realms and `ParadoxTransport` come along; whether DaintyLass carries them is section 8's question 6 |
| DD8 | One codebase for 1.14.7 and 1.12.0 | The Rust port is written once, from the 1.14.7 Node code. 1.12.0's differences come from diffing the tag's Node services against the same files on `replay`, and each becomes a version hook or a data table in the shared crates. The per-version module crates stay thin: manifest, hooks, tables, Director config. The C++ runtime is built per version from each host's SDK; folding the two runtime trees into one source is a spike (RU10), not a promise | 1.12.0 costs a route inventory, a diff and its hooks, not a second port |
| DD9 | dauntless-revived's base | **SilentSpud's fork** (`plan`, `24b7624`) pushed with its history, upstream `main` kept as a branch. Then what this repository changed in its copy of DR (DS5) is brought across. The module is DR's `metagame`, `deploy`, `internal-server` and `client-mods`; DR's own gateway, content, launcher, `discord-bot` and `friend-kit` are dropped from the build, since DaintyLass's core plugins and admin API replace them | DR's 1.4.4 server keeps its history and its Reforged changes; one gateway serves both games |
| DD10 | The UI | **A new repository, `dainty-lass-ui`**, made by `bunx --bun shadcn@latest init --preset b1cYwfJGNu --template astro --monorepo` and mounted at `ui/`. Tauri stays the shell and serves Astro's static build. The bridge's generated TS types are written into the submodule (`RR_UPDATE_GENERATED=1`), and DaintyLass's staleness test reads them there. Both language catalogs move to the UI repository, and `gen/strings.json` is generated from them. Module-specific screens are data: the UI draws a game module's commands, status and settings from its manifest, so no module ships UI code | Section 6's "the page's look word for word" no longer holds; both languages and every string do. Bun joins the build. The Electron launcher in `legacy/` is untouched until RA16 |
| DD11 | Binaries | The 12 edition crates **collapse to four**: `server`, `server-console`, `server-gui`, `client`, each with one Cargo feature per compiled-in game module. The release builds every module in. The player `client` links no server code: what it needs per game (the executable hash, the command line, the ini edits, which files to fetch) comes from the module's client profile, served by content | Fewer crates and one feature list. The launcher's legacy 1.4.4 paths become a client profile from the revived module |
| DD12 | `runtime-loader` | **Stays in DaintyLass** at `native/runtime-loader`, with Paradox's attribution. The launcher's PLAY check (RD9) and every game's client DLLs go through it, 1.4.4's included (`920a411`). `paradox-runtime` and `catalog-exporter` go to rustic-paradox, `internal-server` and `client-mods` to dauntless-revived, `preset/` to rustic-paradox | No module depends on another module's repository |
| DD13 | Freeza's upstream | RD16 stands. Until its cut-off, Freeza's Paradox patches land in rustic-paradox, not here; after it, only runtime patches, still in rustic-paradox | The runtime patch method (`docs/paradox-runtime-patches.md`) moves with the runtime |

## 4. Work

### 4.1 Repositories, split and rename (DS)

| # | What | Verify |
| --- | --- | --- |
| DS1 | Rename `silentspud/dauntless-eternal` to `dainty-lass` on Forgejo; point every clone's `origin` at the new URL (maintainer) | The old URL redirects; `git ls-remote` works on both |
| DS2 | Create `silentspud/rustic-paradox`: push `replay` as `main`, the `dauntless-1.12.0` and `v1.12-final` tags, Mystic's `main` as `upstream/main`; `mystic` stays a fetch-only remote | `git log` shows Mystic's history; the tag resolves to `4d5dfaa` |
| DS3 | Create `silentspud/dauntless-revived`: push SilentSpud's `plan` as `main`, `mixutin`'s `main` as `upstream/main` | `git log` shows DR's history to `24b7624` |
| DS4 | Create `silentspud/dainty-lass-ui` from the shadcn command (DD10); commit the generated tree untouched as the first commit | `bun install` and `bun run build` pass |
| DS5 | Catch-up lists, read-only: this repository's commits touching `paradox/`, `native/paradox-runtime`, `native/catalog-exporter` and `preset/` that `replay` lacks (`docs/port/catchup-rustic.md`); its DR-derived commits (`c1be7f7`…`18ac587` and later ones touching `metagame`, `deploy`, `native/internal-server`, `native/client-mods`) as `docs/port/catchup-revived.md` | Every commit is listed as needed or not applicable, with the reason |
| DS6 | Swap the code for submodules: delete `paradox/`, `native/paradox-runtime`, `native/catalog-exporter`, `native/internal-server`, `native/client-mods`, `preset/`, `rust/services/metagame`, `rust/services/deploy`; add the two module submodules; path sweep of the host kit, Docker, CI and docs. Lands once RU1 and RV1 hold that code, and before the Rust port, so hosts keep running what they run today | `cargo test`, `npm test` in the moved Node services, `bash docker/run-tests.sh` pass from a fresh `git clone --recurse-submodules` |

### 4.2 Module framework and controller (DF)

| # | What | Verify |
| --- | --- | --- |
| DF1 | **Spike**: the cargo layout of DD2. A module workspace under `exclude`, used as a path dependency; a crate on Forgejo's Cargo registry `[patch]`ed to a path; `workspace = true` inheritance inside the submodule; on Windows and Linux. Kill criterion: if the registry or the `[patch]` fails, modules take `module-api` as a git dependency on a tag | A toy module builds both alone and inside DaintyLass |
| DF2 | `crates/module-api`, the seam: `GameVersion`, `ModuleManifest`, the `GameModule` and `CorePlugin` traits, `Registrar` (routes, ports, commands, events, admin actions, config sections), `ModuleContext`; the frame protocol and its adapters for both sides; every capability registered as a stub (RD18's seam rule) | A stub module runs compiled in and out of process and gives the same answers; fuzz target `module_frame` |
| DF3 | Controller: the registry keyed by game version with the duplicate refusal (DD4), load and unload, the `[modules]` section of `host.toml` (compiled-in list, external modules by path and SHA-256), commands into the console, TUI, server GUI and admin API, the event bus, port assignment, logs and status from out-of-process modules into the tracing layers. `Edition`, `EditionSelection` and `Game` removed | Two modules for one version refused in either order; a module killed mid-run shows as failed and restarts on command; nothing left running after stop |
| DF4 | Core plugins on the registry: the gateway routes by game version to the module's upstream (the invite's `version`, then the route), the allowlist guards the union of the modules' UDP ranges, content keys runtime manifests and client profiles by version (`edition=` still accepted, a request without one still gets 1.14.7), the admin API sends kick, ban and lookup to the module of the player's game, the host kit's commands gain `module list`, `load`, `unload` | The gateway and content parity tests pass unchanged; RD25's admin API cases pass against both games |
| DF5 | Bins: four crates with one feature per module (DD11); the release workflow builds them (`server-release.yml` owner) | Each bin starts with zero, one and every module |
| DF6 | Rename sweep (DD1): crate and binary names, window titles, docs, both languages' strings; the contract names DD1 keeps are left as they are | No `Reforged Revival` or `dauntless-eternal` left outside history, the catch-up lists and the contract names |

### 4.3 UI (DU)

| # | What | Verify |
| --- | --- | --- |
| DU1 | Tauri serves the UI: `frontendDist` at the Astro build, the CSP and the isolation hook checked against Astro's output (no inline script Tauri's policy refuses), bridge types generated into `ui/` | The empty UI opens in the Tauri debug build and a bridge call round-trips |
| DU2 | Player screens: servers, invites, PLAY with the runtime acceptance, settings, the game folder; both languages moved over with every key | The launcher tests that drive the page pass; `i18n` checks pass on the new catalogs |
| DU3 | Server screens: the Host page, the per-component status page, module panels drawn from manifests (DD10), module load and unload | A server GUI build with both module repositories runs one of each game |
| DU4 | Drop `rust/launcher/vite.config.ts`'s build of `legacy/ReforgedLauncher` | No Tauri build reads `legacy/` |

### 4.4 rustic-paradox (RU)

The RB rows of `MIGRATION_PLAN.md` section 4.5, moved here (DD6) and done for both versions.

| # | What | From | Verify |
| --- | --- | --- | --- |
| RU1 | Lay out the monorepo with `git mv`: the Node services to `reference/1.14.7/`, `ParadoxRuntime` to `native/runtime/`, `catalog-exporter` and `preset/` brought from DaintyLass; `ParadoxLauncher` deleted; `ADDITIONAL_TERMS.md`, `NOTICE.md` and the licence kept at the top; DS5's rustic list applied | RP3 | The Node tests pass from the new paths |
| RU2 | 1.12.0's Node services and runtime from the tag into `reference/1.12.0/` | — | The tag's own tests pass there |
| RU3 | Route inventory of both versions and their diff (DD8) | RB1 | Every Express route in each inventory; CI fails on a new route without an entry |
| RU4 | Cargo workspace and seams: `paradox-core` (one trait per repository, Postgres on `store`), `paradox-chat`, `paradox-director`, the two module crates with their manifests and hooks; every route answering 501 | RB3, RB5 | Both modules load into DaintyLass and answer 501 |
| RU5 | Areas, one agent each, both versions: auth, the Reforged API, game areas (five agents, as RB8), chat, game data and scripts | RB4, RB6–RB9, RB11 | Each area's Node tests ported, for both versions where 1.12.0 has the area |
| RU6 | Director in Rust, starting from `cdc0a87` | RB10 | Ramsgate, the Training Grounds and a hunt start and stop, on both versions (maintainer) |
| RU7 | `db migrate` from MongoDB and `db verify` | RB4 | Copies of real databases migrate with no difference |
| RU8 | Shadow replay per version | RB2, RB12 | No unexplained diff. 1.12.0 needs its own capture (section 8's question 5) |
| RU9 | Rollback, cutover and Node removal | RB13–RB15 | As in those rows |
| RU10 | **Spike**: one runtime source tree for both versions, SDK glue apart | — | Kill criterion: if the shared part is under half the code, the two trees stay apart |

### 4.5 dauntless-revived (RV)

| # | What | Verify |
| --- | --- | --- |
| RV1 | DS5's revived list applied; `metagame` and `deploy` on `module-api`; module crate `revived-1-4-4` (display name "Dauntless Revived", game version 1.4.4, alias `revived`); DR's gateway, content, launcher, `discord-bot` and `friend-kit` out of the build | The metagame's tests pass; the module loads into DaintyLass |
| RV2 | The 1.4.4 client profile (DD11): executable hash, command line, the DLL through the runtime loader | The launcher starts 1.4.4 against a DaintyLass server (maintainer) |

### 4.6 Cross-cutting (DX)

| # | What |
| --- | --- |
| DX1 | Docs: `architecture.md` for modules and core plugins, the hosting guides, the migration guide's paths, a "writing a game module" page |
| DX2 | Security (RX3): `docs/security/module-host.md` for the frame protocol, external module loading and the SHA-256 list; fuzz target `module_frame`. Each module repository keeps the checklists for what it carries (`metagame.md`, `deploy.md` move to dauntless-revived) |
| DX3 | CI on Forgejo Actions: a runner (maintainer), submodule checkout, Bun; `.github/workflows` read as Forgejo reads them, or moved to `.forgejo/workflows`. GitHub stays a read-only source (`upstream`, `mystic`) |
| DX4 | Licensing: AGPL-3.0-only in every repository; Mystic's `ADDITIONAL_TERMS.md` and its section 1 notice in rustic-paradox's README and `NOTICE.md`, which also says it is a modified, unofficial version (its section 2); DR's attribution in dauntless-revived; `NOTICE.md` here points to all three |

## 5. Order

| Wave | Rows | Needs | Hands-on |
| --- | --- | --- | --- |
| 0 | DS5 lists; DF1 spike | — | — |
| 0 | DS1–DS4 | — | Forgejo: rename, create the repositories, push |
| 1 | DF2 seam; RU1, RU2; RV1's history work; DU1 | DS2–DS4, DF1 | — |
| 2 | DS6 submodule swap | RU1, RV1 | — |
| 2 | DF3 controller, DF4 core plugins, RU3 inventories | DF2 | — |
| 3 | RU4 seams; RV1's module crate; DF5 bins; DU2, DU3 | DF3, DF4 | — |
| 3 | DF6 rename sweep, by itself, between integration passes | DS6 | — |
| 4 | RU5 (about nine agents), RU6, RU7, RV2 | RU4 | Game servers, the 1.4.4 client |
| 5 | RU8 replay, then RU9 | RU5–RU7 | Canary, captures for both versions |
| any | RU10 spike; DX rows with the rows they cover | — | Host SDKs for both versions |

Today's `main` keeps shipping until DS6. After DS6, hosts update to a DaintyLass build that still runs Node Paradox
from the rustic-paradox submodule; the Rust 1.14.7 module replaces it at RU9's cutover.

## 6. Things that must not change

`MIGRATION_PLAN.md` section 6 holds, with two changes: the launcher page's look is replaced (DD10), its strings in
both languages are not; and "the host-only routes and headers stay refused" now covers every module's routes. Also:

- An invite made before the split joins the same server after it: scheme, `version`, `fp`.
- A host's `.env` files and `host.toml` read unchanged; the module section has defaults that reproduce today's
  server.
- Each game version's routes and JSON, matched as its Node service answers them.

## 7. Amendments to `MIGRATION_PLAN.md`

For the integration pass: RD1 (Paradox becomes a submodule, no longer a subtree at `paradox/`), RD11 and section
4.1 (the layout in section 2 here), RD17 (no gate for the Paradox port, DD6), section 4.5 (the RB rows move to RU),
RP rows (their results move to rustic-paradox), RR5 (the page moves to `ui/`, not `rust/launcher/ui`), section 5.1's
ownership table (the module repositories and `ui/` own their own files; `crates/module-api` is a seam owned by
DF2).

## 8. Open questions

1. **Display names.** 1.14.7: "Rustic Paradox" or "Dauntless Reforged"? 1.12.0: the same name with its version,
   or one of its own? 1.4.4: "Dauntless Revived"?
2. **Mystic's name clause.** Its additional terms (sections 3 and 4) bar using "Mystic Paradox" in a way that
   implies affiliation. "Rustic Paradox" is close to it; asking MysticFox first, or picking a name further from
   theirs, removes the question.
3. **How deep the rename goes.** DD1 keeps every contract name. Should any of them move later (env prefixes, the
   launcher identifier, the nftables table), each with its own migration row?
4. **Database.** RD15 puts the Paradox port on Postgres; DR's metagame is on SQLite. Keep both, or one engine for
   every module?
5. **1.12.0's test oracle.** The replay needs traffic from a real 1.12.0 client against its Node services. Is there
   a 1.12.0 install and anyone to play it, and does 1.12.0 have hosts at all?
6. **Realms.** Is Mystic's player-hosted realms feature (`ParadoxTransport`, ICE) in scope for the 1.14.7 module,
   and then for DaintyLass's launcher?
7. **Visibility.** `dauntless-eternal` is public. Are the three new repositories public too?
