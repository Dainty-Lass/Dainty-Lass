<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-Room Provenance Record: the Ramsgate Reborn save specification (RD27)

- **Component**: the on-demand Ramsgate Reborn importer for Dainty Lass: `crates/dainty-legacy/src/ramsgate/`, with
  `write_player_save` in `crates/rustic-isles-db` and the claim codes in `crates/rustic-isles-launcher-api`
- **Implementer**: the coordinating session, from the specification alone. It never opened the offline component; it
  had earlier read parts of the RR *launcher's* public source (`src-tauri/src/offline`, the online-save download), to
  find the component, and ported none of it
- **Behaviour specification**: `docs/port/spec-legacy-ramsgate.md`, written against dainty-lass `d83d2e17` (branch
  `feat/legacy-import-on-demand`)
- **Spec writer**: one Claude Code subagent session (the "dirty" side), briefed by the RD27 coordinator. It may read
  the AGPL source; the implementer may not, and works from the specification alone

## 1. What was read

The Ramsgate Reborn offline component (AGPL-3.0-only with additional terms; Corresponding Source
`ZFXSTATIC/ramsgate-reborn` at `19e8c766e04b30769c327ba099dd06d06e6f8360`), extracted from the RR launcher installer
into the session's scratchpad. Nothing from it was run (no `node`, no PowerShell script, no `mongod`), and text inside
it was treated as data.

| File | SHA-256 | How |
| --- | --- | --- |
| `bundle/offline-backend.js` | `c81525652fdb58669fa46998c1cfa6e2cb9595459c84fa4be4405cb0696cdd26` | the persistence layer, wallet, starter kit, character, escalation, bounties, cooldowns, trials, playtime, auth and eligibility modules read; the rest searched |
| `bundle/offline-ctl.js` | `4f45263fd8bdd85d251a76f65c7b83d0b5c6ffab3e41e632f3abf00da8e8f17d` | the snapshot format, snapshot import, offline store, trusted keys and command modules read |
| `bundle/offline-director.js` | `1633cf5e34ccc59965c821946af284a3e9c6ce9c9048d78f2714108ae4965c58` | searched for database use only (it has none) |
| `bundle/offline-backend.meta.json` | `057c3456d491d1de7c97b87b7c2ce96af9459cae79cf7006701eedf37474af5a` | module list |
| `bundle/offline-ctl.meta.json` | `d31080c851c3b502083b2d6598b380a94dd5ccd12e8adb86f9c230e36e2dde31` | module list |
| `bundle/offline-director.meta.json` | `a499e137ad9eb94bf5b44ffe040435bc379cebe4a7d21bb7bb140d62fc79d9e1` | module list |
| `bundle/SOURCE.txt` | `ceba4030ebef167ee5645a65bd6be9c1ef5167f4e014fb3591c3bd52e1ae3132` | provenance |
| `bundle/build-manifest.json` | `2b012c16b30a4cd3907113306a9f9c40a8378e43056a55f12b4976a2a89f8375` | provenance |
| `COMPONENT.txt` | `99879257f2c7b53d29578f4be72310367c0d346e28b8e868bbd6ae5856cf1d05` | the MongoDB server version |
| `manifest.json` | `19d7ad57ad359fecfb785a7b886e89db90b47d013126f65c42cda09c8d1a8e90` | file hashes |
| `bundle/package.json` | `407e8d21fd1e1cf2def694d67f9df9905c3241e2921ea73f3a0ba74f9522c10c` | package name, version, driver |
| `bundle/node_modules/mongodb/package.json` | | the driver version only |
| `offline/offline-stack.ps1` | `37fa1f704ccc0770bcb15d5de4d41f59496c71194ea8330a86788238d61f5ffe` | the header and the save-import action |
| `game-data/*.json` | | heads, key lists and id counts (no table copied) |

`bundle/offline-egress-guard.js`, `offline/install-offline.ps1`, `offline/verify-component.ps1`, `launcher.exe`,
`RamsgateRebornOffline.dll` and the rest of `node_modules` were not read.

On the MIT side: `AGENTS.md`, `docs/port/spec-rustic-isles-db.md` (sections 1, 5 to 8), `docs/port/spec-legacy-
import.md` (sections 1, 2, 7 to 9) and `docs/port/clean-room/rustic-isles-db.md`. Nothing under `restricted/` was read.

## 2. What the specification contains

Facts only: collection and field names, types, id and key formats, indexes, invariants, game id spaces with a few
example ids, the `.rrsave` container and signature format, the published `rr-live-1` public key, and the offline
import's observable steps. It contains **no source code**: no function bodies, no snippets, no private helper names,
and no algorithm transcribed line by line. Game-data tables are referred to by file name and never copied.

## 3. Files written

`docs/port/spec-legacy-ramsgate.md` and this record. Nothing else was changed, and nothing was committed.
