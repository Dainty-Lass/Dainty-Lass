# NOTICE

dainty-restricted is the AGPL half of DaintyLass (RD27), licensed under the GNU Affero General Public License,
version 3 only (`AGPL-3.0-only`); see [LICENSE.txt](LICENSE.txt). Additional terms under AGPLv3 section 7 apply to the
Mystic Paradox material only; see [ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md). It builds on the MIT crates of
[dainty-lass](https://git.silentspud.com/silentspud/dainty-lass), taken as git dependencies; a program built from both
is distributed under the AGPL-3.0-only as a whole. This file records where each part of this repository comes from.

## Modification notice

This repository, `silentspud/dainty-restricted` on `git.silentspud.com`, combines and modifies two AGPL-3.0-only
projects to run Dauntless 1.14.7 (`rel-1.14.7-Archon`, changelist 647472) on private servers. Every change is
recorded in the commit history. It is **not** an official release of Mystic Paradox, Dauntless Revived,
Undaunted or any other project named here, and none of them endorses it.

## Merged repositories

These repositories were merged in with their history (RD27) and are archived; each kept its own `NOTICE.md` and
licence files in its folder: `modules/rustic-isles` ([NOTICE.md](modules/rustic-isles/NOTICE.md), with Mystic Paradox's
additional terms), `modules/rustic-rifts` ([NOTICE.md](modules/rustic-rifts/NOTICE.md); libjuice is `MPL-2.0`),
and `legacy/` (dainty-legacy). dainty-dash was here too until 2026-11-02, when it moved to dainty-lass's
`dashboard/` (now `ui/`) under the MIT licence; its history up to then stays in this repository's. The UI's two AGPL inputs (`ui/`) came from
dainty-lass-ui.

## Where the parts come from

### Mystic Paradox

- By Pranav Karande (MysticFox). Repository:
  [github.com/pranav158/Mystic-Paradox](https://github.com/pranav158/Mystic-Paradox), AGPL-3.0-only with
  additional terms. Mystic Paradox is itself a modified version of Undaunted.
- **"Mystic Paradox's Dauntless 1.12.0 and 1.14.7 ports and related modifications were developed by MysticFox /
  Pranav Karande. See NOTICE.md for contribution and provenance information."** Mystic Paradox's own notice is
  [modules/rustic-isles/NOTICE.md](modules/rustic-isles/NOTICE.md), its licence
  [modules/rustic-isles/LICENSE](modules/rustic-isles/LICENSE) and its additional terms
  [modules/rustic-isles/ADDITIONAL_TERMS.md](modules/rustic-isles/ADDITIONAL_TERMS.md).
- Its backend and Director reference sources and Rust ports (`isles-core`, `isles-director`, `isles-chat`,
  `isles-1-14-7`, `isles-1-12-0`) are in [`modules/rustic-isles`](modules/rustic-isles) (from `ParadoxBackend/`
  and `ParadoxDirector/` at commit `21b9ff5` and tag `dauntless-1.12.0` at `4d5dfaa`), and its realm and Guard
  ports (`rifts-transport`, `rifts-guard`, `rifts-server`) are in [`modules/rustic-rifts`](modules/rustic-rifts);
  its game runtime (`ParadoxRuntime/`) and CatalogExporter (`tools/CatalogExporter/`) are in
  [`modules/rustic-isles/native`](modules/rustic-isles/native), and the runtime's loader (`tools/RuntimeLoader/`)
  is in [`native/runtime-loader`](native/runtime-loader). Its launcher, deployment scripts and self-hosting guide
  were not taken. Every file from Mystic Paradox, and every change made to it here, stays under Mystic Paradox's
  terms.
- The changes made to it are listed patch by patch in [docs/paradox-patches.md](docs/paradox-patches.md).
  Patches 0004 to 0039 were written in Freeza's Dauntless Reforged Revival line
  ([github.com/FreezaMotoqueiro/Dauntless-Reforged-Revival](https://github.com/FreezaMotoqueiro/Dauntless-Reforged-Revival),
  AGPL-3.0-only) and are kept here as they were received; the files they touch say so in their comments. The
  rest of that project is not part of this repository.
- The runtime's loader (`native/runtime-loader`) is derived from
  [coldloader-proxy](https://github.com/denuvosanctuary/coldloader-proxy); Mystic Paradox's README, NOTICE.md
  and Cargo.toml give its licence as Apache-2.0, while the LICENSE file in that folder holds the GNU LGPL-3.0
  text. The runtime includes MinHook and the Hacker Disassembler Engine 64 (BSD-2-Clause).

### Dauntless Revived

- By mixutin and the Dauntless Revived contributors. Repository:
  [github.com/mixutin/dauntless-revived](https://github.com/mixutin/dauntless-revived), AGPL-3.0-only.
  Dauntless Revived is itself a modified version of Undaunted.
- The workspace files and the fuzz targets `http_body` and `proxy_headers` are modified versions of its `rust/` folder at commit `e1c5fec`
  (dainty-lass's [docs/port/migration-guide.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/port/migration-guide.md) section 2 says how); `launcher/src-tauri`
  carries the DR-derived core of its launcher's Rust side alongside `crates/launcher-kit`. (`crates/protocol`,
  `proxy`, `service-kit`, the SQLite helpers of `store`, and `gateway`, `content` and `chat` were rewritten clean-room from behaviour specs under MIT in
  `DL3b`/`DD26`; see `docs/port/clean-room/`.)
- Files and functions taken over say so in their comments.

### Undaunted

- By gwog (Gregory Morford, [SyST3MDeV](https://github.com/SyST3MDeV)), EisigesEis and its other
  contributors. Repository: [github.com/SyST3MDeV/Undaunted](https://github.com/SyST3MDeV/Undaunted),
  AGPL-3.0-only.
- The project both of the above are built on: the server-mode DLL, the deploy server, the metagame and the
  original launcher.

### This project

By the DaintyLass contributors (`Copyright (c) 2026 Silent Spud`), AGPL-3.0-only: `crates/host` (the shell that wires
dainty-lass's `hostkit` to `gateway`, `content`, `chat` and `isles-1-14-7`; its `compose.rs` is MIT code moved from
dainty-lass's `controller::modules` and keeps its MIT header); the one-line library crates `crates/gateway-plugin`, `crates/content-plugin` and
`crates/chat-plugin`, over dainty-lass's MIT `gateway`, `content` and `chat`; the DR-derived Tauri shell in `launcher/src-tauri` (including `net/epic_install.rs`, which keeps DR's
downloader structure); `crates/server-console` and `crates/server-minimal`; the fuzz targets in `fuzz/` (`http_body` and `proxy_headers` from DR's `rust/`).
`docker/`, `docs/`, the workflows, the launcher's language catalogs (`ui/packages/i18n/catalogs`) and
`launcher/src-tauri/gen/strings.json` carry the repository licence.

### Store captures

- Captures of the live Dauntless store service by EisigesEis:
  [github.com/EisigesEis/DauntlessEndpointDocumentation](https://github.com/EisigesEis/DauntlessEndpointDocumentation),
  published for academic and informational purposes, with no licence. Mystic Paradox's store generators read
  five of its files (Lady Luck's rotation, the platinum webstore and dyes, the Hunt Pass rank skips and
  passes). They are **not** copied into this repository:
  [modules/rustic-isles/reference/1.14.7/ParadoxBackend/store-captures.json](modules/rustic-isles/reference/1.14.7/ParadoxBackend/store-captures.json) names the commit and each
  file's SHA-256, and the host kit's `gamedata` fetches them on the host's own machine.

### Other material

- The services' npm dependencies are listed in each `package-lock.json`, and the Rust crates' in
  `Cargo.lock`; each keeps its own licence (`cargo deny` checks the Rust ones).

## What this repository does not contain

No game files, no SDK generated from the game, no game data exported from it beyond the examples the services
ship with, and no store captures. Each host produces those from their own copy of the game and never shares
them through this project.
