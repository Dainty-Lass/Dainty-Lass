# NOTICE

`restricted/` is the AGPL subtree of Dainty Lass (RD27), licensed under the GNU Affero General Public License,
version 3 only (`AGPL-3.0-only`); see [LICENSE.txt](LICENSE.txt). Additional terms under AGPLv3 section 7 apply to the
Mystic Paradox material only; see [ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md). Since RB15 it holds docs and data
only, no code; the server release ships it beside the MIT binaries built from [`../crates/`](../crates). This file
records where each part of `restricted/` comes from, and what was here before.

## Modification notice

This subtree, `restricted/` of `silentspud/dainty-lass` on `git.silentspud.com`, combines and modifies two AGPL-3.0-only
projects to run Dauntless 1.14.7 (`rel-1.14.7-Archon`, changelist 647472) on private servers. Every change is
recorded in the commit history. It is **not** an official release of Mystic Paradox, Dauntless Revived,
Undaunted or any other project named here, and none of them endorses it.

## Merged repositories

These repositories were merged in with their history (RD27) and are archived; each kept its own `NOTICE.md` and
licence files in its folder: `modules/rustic-isles` ([NOTICE.md](modules/rustic-isles/NOTICE.md), with Mystic Paradox's
additional terms), `modules/rustic-rifts` ([NOTICE.md](modules/rustic-rifts/NOTICE.md)),
and `legacy/` (dainty-legacy, the oracle for the root's MIT `crates/dainty-legacy` until RB15 deleted it). dainty-dash was here too until 2026-11-02, when it moved to dainty-lass's
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
- Its backend's and Director's game data examples and generator inputs are in
  [`modules/rustic-isles/game-data/1.14.7`](modules/rustic-isles/game-data/1.14.7) (from `ParadoxBackend/game-data` and
  `ParadoxDirector/game-data` at commit `21b9ff5`). Their Node sources (the reference tree, at `21b9ff5` and tag
  `dauntless-1.12.0` at `4d5dfaa`) and the goldens recorded from them (`rustic-isles-mit-parity`) were here until RB15
  deleted them; their Rust ports `isles-core`, `isles-director`, `isles-chat`, `isles-db`, `isles-1-14-7` and
  `isles-1-12-0` were replaced earlier by dainty-lass's MIT clean-room `rustic-isles-*` crates (RD27). All stay in this
  repository's history. The Node vectors recorded from its realm modules were in
  [`modules/rustic-rifts`](modules/rustic-rifts) (`oracle`, deleted at RB15; its Rust realm and Guard ports were replaced by dainty-lass's MIT
  clean-room `rustic-rifts-*` crates, RD27, and removed, as was its C++ co-op transport (`ParadoxTransport/`), replaced
  by the launcher's built-in `rustic-rifts-transport`; they stay in this repository's history);
  its CatalogExporter (`tools/CatalogExporter/`) was in `modules/rustic-isles/native/catalog-exporter` until
  2026-11-13, when the root's MIT clean-room `native/rustic-isles-exporter` (`docs/port/spec-catalog-exporter.md`)
  replaced it; the tag `agpl-catalog-exporter-final` is its last commit. Its game runtime (`ParadoxRuntime/`) was in
  `modules/rustic-isles/native/runtime` until it retired at R21 (2026-11-14), replaced by dainty-lass's MIT
  `rustic-isles-runtime.dll`, and was deleted; the tag `agpl-runtime-final` is its last commit, with its patches.
  Its runtime loader (`tools/RuntimeLoader/`) was
  in `restricted/native/runtime-loader` until 2026-11-11, when the root's MIT clean-room rewrite
  (`native/runtime-loader`, `docs/port/spec-runtime-loader.md`) replaced it. Its launcher, deployment scripts and self-hosting guide
  were not taken. Every file from Mystic Paradox, and every change made to it here, stays under Mystic Paradox's
  terms.
- The 1.14.7 version data in
  [`modules/rustic-isles/version-data/1.14.7/version_data.json`](modules/rustic-isles/version-data/1.14.7/version_data.json)
  holds only Mystic Paradox's design since 2026-11-13 (the reward cache plan, the starter wallet, the legacy starter
  and the default appearance, from its `character.ts`, `wallet.ts`, `generate_reward_cache_store.ts` and isles-db's
  migration). The game's identifiers and GUIDs it held are facts about the game and moved to dainty-lass's MIT
  `crates/rustic-isles-1-14-7/data/version_facts.json`, and its storage token seeds, extracted from Mystic Paradox's
  source, were replaced there by the catalog ids dainty-lass's own crates name (rulings V-1 to V-3,
  `docs/port/rulings-2026-11-13.md` at the root).
- The changes made to it are listed patch by patch in [docs/paradox-patches.md](docs/paradox-patches.md), now a
  record: the code they changed is gone.
  Patches 0004 to 0039 were written in Freeza's Dauntless Reforged Revival line
  ([github.com/FreezaMotoqueiro/Dauntless-Reforged-Revival](https://github.com/FreezaMotoqueiro/Dauntless-Reforged-Revival),
  AGPL-3.0-only) and are kept here as they were received; the files they touch say so in their comments. The
  rest of that project is not part of this repository.
- The runtime loader removed on 2026-11-11 was derived from
  [coldloader-proxy](https://github.com/denuvosanctuary/coldloader-proxy); Mystic Paradox's README, NOTICE.md
  and Cargo.toml gave its licence as Apache-2.0, while the LICENSE file in its folder held the GNU LGPL-3.0
  text. It stays in the history only. The retired game runtime (at the tag `agpl-runtime-final`) included MinHook
  and the Hacker Disassembler Engine 64 (BSD-2-Clause).

### Dauntless Revived

- By mixutin and the Dauntless Revived contributors. Repository:
  [github.com/mixutin/dauntless-revived](https://github.com/mixutin/dauntless-revived), AGPL-3.0-only.
  Dauntless Revived is itself a modified version of Undaunted.
- The workspace files, the fuzz targets `http_body` and `proxy_headers` (modified from its `rust/` folder at commit
  `e1c5fec`) and `launcher/src-tauri` (the DR-derived core of its launcher's Rust side, kept as tests) were here
  until RB15 deleted them; they stay in this repository's history. (`crates/dainty-protocol`,
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

By the Dainty Lass contributors (`Copyright (c) 2026 Silent Spud`), AGPL-3.0-only: `docs/`, the module docs and the
relay self-test. Until RB15 this also held `crates/dainty-host-parity` (the Node host kit's tests, ported from
Freeza's `ReforgedHost`), the DR-derived Tauri shell's tests in `launcher/src-tauri`, `fuzz/` and `docker/`; they stay
in this repository's history.

### Store captures

- Captures of the live Dauntless store service by EisigesEis:
  [github.com/EisigesEis/DauntlessEndpointDocumentation](https://github.com/EisigesEis/DauntlessEndpointDocumentation),
  published for academic and informational purposes, with no licence. Mystic Paradox's store generators read
  five of its files (Lady Luck's rotation, the platinum webstore and dyes, the Hunt Pass rank skips and
  passes). They are **not** copied into this repository:
  [modules/rustic-isles/game-data/1.14.7/store-captures.json](modules/rustic-isles/game-data/1.14.7/store-captures.json) names the commit and each
  file's SHA-256, and the host kit's `gamedata` fetches them on the host's own machine.

### Other material

- The Rust crates' dependencies are listed in the root's `Cargo.lock`; each keeps its own licence (`cargo deny`
  checks them).

## What this repository does not contain

No game files, no SDK generated from the game, no game data exported from it beyond the examples the services
ship with, and no store captures. Each host produces those from their own copy of the game and never shares
them through this project.
