# NOTICE

DaintyLass is licensed as a whole under the GNU Affero General Public License, version 3 only
(`AGPL-3.0-only`); see [LICENSE.txt](LICENSE.txt). Additional terms under AGPLv3 section 7 apply to the
Mystic Paradox material only; see [ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md). The crates written for this
project that derive from none of the projects below are also offered under the MIT License
([LICENSE-MIT.txt](LICENSE-MIT.txt)); "This project" says which. This file records where each part of the
repository comes from.

## Modification notice

This repository, `silentspud/dainty-lass` on `git.silentspud.com`, combines and modifies two AGPL-3.0-only
projects to run Dauntless 1.14.7 (`rel-1.14.7-Archon`, changelist 647472) on private servers. Every change is
recorded in the commit history. It is **not** an official release of Mystic Paradox, Dauntless Revived,
Undaunted or any other project named here, and none of them endorses it.

## Submodule and companion repositories

DaintyLass mounts its UI and game modules from separate repositories under `silentspud` on `git.silentspud.com`, each
with its own `NOTICE.md` and licence files (DD2, DD26, DX4):

- **[`ui/`](ui) (`silentspud/dainty-lass-ui`)**: [ui/NOTICE.md](ui/NOTICE.md) — AGPL-3.0-only as a whole
  ([ui/LICENSE-AGPL.txt](ui/LICENSE-AGPL.txt)); application and component code also under MIT
  ([ui/LICENSE-MIT.txt](ui/LICENSE-MIT.txt)), language catalogs (`packages/i18n/catalogs`) and generated bridge types
  under AGPL-3.0-only.
- **[`modules/rustic-isles`](modules/rustic-isles) (`silentspud/rustic-isles`)**:
  [modules/rustic-isles/NOTICE.md](modules/rustic-isles/NOTICE.md) — AGPL-3.0-only
  ([modules/rustic-isles/LICENSE](modules/rustic-isles/LICENSE)) with Mystic Paradox's section 7 additional terms
  ([modules/rustic-isles/ADDITIONAL_TERMS.md](modules/rustic-isles/ADDITIONAL_TERMS.md)).
- **[`modules/rustic-rifts`](modules/rustic-rifts) (`silentspud/rustic-rifts`)**:
  [modules/rustic-rifts/NOTICE.md](modules/rustic-rifts/NOTICE.md) — AGPL-3.0-only
  ([modules/rustic-rifts/LICENSE](modules/rustic-rifts/LICENSE)) with Mystic Paradox's section 7 additional terms
  ([modules/rustic-rifts/ADDITIONAL_TERMS.md](modules/rustic-rifts/ADDITIONAL_TERMS.md)) and libjuice (`MPL-2.0`).
- **[`modules/legacy-revived`](modules/legacy-revived) (`silentspud/legacy-revived`)**:
  [modules/legacy-revived/NOTICE.md](modules/legacy-revived/NOTICE.md) — AGPL-3.0-only
  ([modules/legacy-revived/LICENSE.txt](modules/legacy-revived/LICENSE.txt)), attributing Dauntless Revived, Undaunted
  and Harmonic's 1.4.4 fork.
- **`silentspud/rustic-freeza`** (when mounted at `modules/rustic-freeza`, DD25): AGPL-3.0-only, attributing Freeza's
  Dauntless Reforged Revival line (`FreezaMotoqueiro/Dauntless-Reforged-Revival`).

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
- `rust/services/gateway` and `rust/services/content`, the workspace files and the fuzz targets `http_body` and
  `proxy_headers` are modified versions of its `rust/` folder at commit `e1c5fec`
  ([docs/port/migration-guide.md](docs/port/migration-guide.md) section 2 says how); `rust/launcher/src-tauri`
  carries the DR-derived core of its launcher's Rust side alongside `rust/crates/launcher-kit`. (`rust/crates/protocol`,
  `proxy`, `service-kit` and the SQLite helpers of `store` were rewritten clean-room from behaviour specs under MIT in
  `DL3b`/`DD26`; see `docs/port/clean-room/`.)
- `modules/legacy-revived` (`rust/services/metagame`, `rust/services/deploy`, `rust/crates/revived-1-4-4`,
  `native/client-patches` and `native/xinput1_3-proxy`) is its 1.4.4 server and client patch set, carried in the
  `legacy-revived` submodule (AGPL-3.0-only).
- Files and functions taken over say so in their comments.

### Undaunted

- By gwog (Gregory Morford, [SyST3MDeV](https://github.com/SyST3MDeV)), EisigesEis and its other
  contributors. Repository: [github.com/SyST3MDeV/Undaunted](https://github.com/SyST3MDeV/Undaunted),
  AGPL-3.0-only.
- The project both of the above are built on: the server-mode DLL, the deploy server, the metagame and the
  original launcher.

### This project

By the DaintyLass contributors (`Copyright (c) 2026 Silent Spud`), in two groups.

- **MIT** ([LICENSE-MIT.txt](LICENSE-MIT.txt)), each crate's `Cargo.toml` says so (`license = "MIT"`), with zero AGPL
  normal or dev dependencies:
  - **Born here**: `rust/crates/controller`, `rust/crates/module-api` (DF2, the module framework and frame protocol),
    `rust/crates/epic` (DE1, written from Epic's manifest and chunk formats as Legendary documents them),
    `rust/crates/history-ship` (DH4, shipping the change log to an object-locked bucket and restoring from it),
    `rust/crates/remote` (DD27/RM2, remote management of enrolled server installs over pinned mTLS),
    `rust/plugins/network` (DN4), `rust/services/admin` (including the DH2/DH3/DH6 additions `guard.rs`, `guarded.rs`,
    `guarded_cli.rs`, `key_cli.rs`, `history.rs` and `history_text.rs`), and `rust/bins` (`server-console`,
    `server-minimal`).
  - **Clean-room MIT rewrites and extracted MIT kits** (`DD26`, `DL2` answered No on 2026-10-30; behaviour specs in
    `docs/port/spec-*.md` and clean-room provenance records in `docs/port/clean-room/*.md`): `rust/crates/protocol`,
    `rust/crates/proxy`, `rust/crates/service-kit` (clean-room HTTP/routing/env/log/JSON helpers plus born-here
    `context.rs` and `telemetry.rs`), `rust/crates/store` (born-here `MultiConnection` over SQLite and Postgres and
    hash-chained change log plus clean-room SQLite helpers), `rust/crates/hostkit` (`DL3b`: the 18 born-here host-kit
    modules plus clean-room rewrites of the 22 host-kit modules specified in `docs/port/spec-dl3-hostkit.md`), and
    `rust/crates/launcher-kit` (`DL4b`: the born-here launcher modules plus clean-room rewrites of the 8 launcher module
    groups specified in `docs/port/spec-dl4-launcher.md`).
  Written here against the services' contracts and behaviour specs, from none of the AGPL projects above. A binary that
  links them to the AGPL crates is distributed under the AGPL as a whole; the MIT grant is on these crates' own source.
- **AGPL-3.0-only**: `rust/services/host` (the thin binary and legacy 1.4.4 shell that wires `crates/hostkit` to
  `gateway`, `content`, `isles-1-14-7`, `isles-1-12-0`, `isles-core`, `isles-db`, `isles-director`, and DR's 1.4.4
  stack); `rust/services/gateway` and `rust/services/content`; `modules/legacy-revived/rust/crates/revived-1-4-4` (RV1,
  wrapping DR's `metagame` and `deploy` as a `GameModule`); `rust/plugins/chat` (DN10a, a port of Dauntless Revived's
  metagame chat at `e1c5fec` with behaviour, no code, from Mystic Paradox's realtime gateway); and the DR-derived Tauri
  shell in `rust/launcher/src-tauri` (including `net/epic_install.rs`, which keeps DR's downloader structure).
  `docker/`, `docs/`, the workflows, the launcher's language catalogs (`ui/packages/i18n/catalogs`) and
  `rust/launcher/src-tauri/gen/strings.json` carry the repository licence.

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
  `rust/Cargo.lock`; each keeps its own licence (`cargo deny` checks the Rust ones).
- libpq, built from source by `pq-src` and linked into every binary that links `store` (DF7): PostgreSQL
  Licence. Portions Copyright (c) 1996-2025, PostgreSQL Global Development Group; Portions Copyright (c) 1994,
  The Regents of the University of California. Permission to use, copy, modify, and distribute this software
  and its documentation for any purpose, without fee, and without a written agreement is hereby granted,
  provided that the above copyright notice and this paragraph and the following two paragraphs appear in all
  copies. IN NO EVENT SHALL THE UNIVERSITY OF CALIFORNIA BE LIABLE TO ANY PARTY FOR DIRECT, INDIRECT, SPECIAL,
  INCIDENTAL, OR CONSEQUENTIAL DAMAGES, INCLUDING LOST PROFITS, ARISING OUT OF THE USE OF THIS SOFTWARE AND ITS
  DOCUMENTATION, EVEN IF THE UNIVERSITY OF CALIFORNIA HAS BEEN ADVISED OF THE POSSIBILITY OF SUCH DAMAGE. THE
  UNIVERSITY OF CALIFORNIA SPECIFICALLY DISCLAIMS ANY WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
  WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE. THE SOFTWARE PROVIDED HEREUNDER IS ON AN
  "AS IS" BASIS, AND THE UNIVERSITY OF CALIFORNIA HAS NO OBLIGATIONS TO PROVIDE MAINTENANCE, SUPPORT, UPDATES,
  ENHANCEMENTS, OR MODIFICATIONS.

## What this repository does not contain

No game files, no SDK generated from the game, no game data exported from it beyond the examples the services
ship with, and no store captures. Each host produces those from their own copy of the game and never shares
them through this project.
