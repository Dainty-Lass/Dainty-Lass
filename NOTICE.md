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

## Where the parts come from

### Mystic Paradox

- By Pranav Karande (MysticFox). Repository:
  [github.com/pranav158/Mystic-Paradox](https://github.com/pranav158/Mystic-Paradox), AGPL-3.0-only with
  additional terms. Mystic Paradox is itself a modified version of Undaunted.
- **Mystic Paradox's Dauntless 1.12.0 port and related modifications were developed by Pranav Karande. See
  NOTICE.md for contribution and provenance information.** Mystic Paradox's own notice is
  [paradox/NOTICE.md](paradox/NOTICE.md), its licence [paradox/LICENSE](paradox/LICENSE) and its additional
  terms [paradox/ADDITIONAL_TERMS.md](paradox/ADDITIONAL_TERMS.md).
- Its backend and Director are in [`paradox/backend`](paradox/backend) and [`paradox/director`](paradox/director)
  (from `ParadoxBackend/` and `ParadoxDirector/` at commit `21b9ff5`); its game runtime (`ParadoxRuntime/`),
  the runtime's loader (`tools/RuntimeLoader/`) and the CatalogExporter (`tools/CatalogExporter/`) in
  [`native/`](native). Its launcher, transport, deployment scripts and self-hosting guide were not taken.
  Every file from Mystic Paradox, and every change made to it here, stays under Mystic Paradox's terms.
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
- `rust/crates/protocol`, `proxy`, `service-kit` and `store`, the workspace files and the fuzz targets
  `http_body` and `proxy_headers` are copies of its `rust/` folder at commit `e1c5fec`, modified here
  ([docs/port/migration-guide.md](docs/port/migration-guide.md) section 2 says how). `rust/services/gateway`
  and `rust/services/content` are modified versions of its gateway and content server;
  `rust/launcher/src-tauri` of its launcher's Rust side. The invite format, the account key and the pinned TLS
  connection follow its design.
- `rust/services/metagame`, `rust/services/deploy`, `native/internal-server` and `native/client-mods` are its
  1.4.4 server, carried here behind the `legacy` feature for Dauntless Revived servers.
- Files and functions taken over say so in their comments.

### Undaunted

- By gwog (Gregory Morford, [SyST3MDeV](https://github.com/SyST3MDeV)), EisigesEis and its other
  contributors. Repository: [github.com/SyST3MDeV/Undaunted](https://github.com/SyST3MDeV/Undaunted),
  AGPL-3.0-only.
- The project both of the above are built on: the server-mode DLL, the deploy server, the metagame and the
  original launcher.

### This project

By the DaintyLass contributors, in two groups.

- **MIT** ([LICENSE-MIT.txt](LICENSE-MIT.txt)), each crate's `Cargo.toml` says so: `rust/crates/controller`,
  `rust/services/admin`, `rust/bins`, and the `rust/services/backend` and `director` stubs. Written here
  against the services' contracts, from none of the projects above. A binary that links them to the AGPL
  crates is distributed under the AGPL as a whole; the MIT grant is on these crates' own source.
- **AGPL-3.0-only**: `rust/services/host`, a port of the Dauntless Reforged Revival line's Node host kit
  (`ReforgedHost`, AGPL-3.0-only, by Freeza and that project's contributors; each ported module names its
  source file); the launcher's modules that port that line's launcher additions (the runtime files, the Host
  page, the host kit driver); and the new parts of the components above. `docker/`, `docs/`, the workflows,
  the launcher's page and bridge files and `rust/launcher/src-tauri/gen/strings.json` are this project's own
  and carry the repository licence.

### Store captures

- Captures of the live Dauntless store service by EisigesEis:
  [github.com/EisigesEis/DauntlessEndpointDocumentation](https://github.com/EisigesEis/DauntlessEndpointDocumentation),
  published for academic and informational purposes, with no licence. Mystic Paradox's store generators read
  five of its files (Lady Luck's rotation, the platinum webstore and dyes, the Hunt Pass rank skips and
  passes). They are **not** copied into this repository:
  [paradox/backend/store-captures.json](paradox/backend/store-captures.json) names the commit and each
  file's SHA-256, and the host kit's `gamedata` fetches them on the host's own machine.

### Other material

- The services' npm dependencies are listed in each `package-lock.json`, and the Rust crates' in
  `rust/Cargo.lock`; each keeps its own licence (`cargo deny` checks the Rust ones).

## What this repository does not contain

No game files, no SDK generated from the game, no game data exported from it beyond the examples the services
ship with, and no store captures. Each host produces those from their own copy of the game and never shares
them through this project.
