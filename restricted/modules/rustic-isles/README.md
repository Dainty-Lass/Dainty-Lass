# Rustic Isles (`isles-1-14-7`)

> **Modified, unofficial version.** This repository (`silentspud/rustic-isles` on `git.silentspud.com`) is a modified,
> unofficial version of [Mystic Paradox](https://github.com/pranav158/Mystic-Paradox) (`21b9ff5` and tag
> `dauntless-1.12.0` at `4d5dfaa`), maintained as the **Rustic Isles 1.14.7** game module of
> [Dainty Lass](https://git.silentspud.com/silentspud/dainty-lass). It is **not** an official release of the Mystic
> Paradox project, and Mystic Paradox does not endorse it.
>
> "Mystic Paradox's Dauntless 1.12.0 and 1.14.7 ports and related modifications were developed by MysticFox / Pranav
> Karande. See NOTICE.md for contribution and provenance information."
>
> The Rust port of Mystic Paradox's 1.14.7 backend and Director that lived here (`isles-core`, `isles-director`,
> `isles-chat`, `isles-db`, `isles-1-14-7`) was replaced by Dainty Lass's clean-room MIT `rustic-isles-*` crates (RD27)
> and removed; it stays in the history. The Node reference it came from (`reference/1.14.7`) and the goldens
> replayed against the MIT module (`crates/rustic-isles-mit-parity`) were deleted at RB15. What is left here is the
> 1.14.7 version data, the game-data examples and store captures, and the host guides. The C++
> game runtime retired at R21 (its last commit is the tag `agpl-runtime-final`): the runtime is dainty-lass's MIT
> `rustic-isles-runtime.dll`, and the C++ CatalogExporter was replaced by its MIT `native/rustic-isles-exporter`
> (tag `agpl-catalog-exporter-final`).

> [!IMPORTANT]
> Unofficial and community-developed. Not affiliated with, endorsed by, or sponsored by Phoenix Labs,
> Epic Games, Forte Labs, or any current or former Dauntless rights holder. This repository ships
> **source code only** — no game files, SDK or game data.

## Using it

Rustic Isles runs as the 1.14.7 game module of Dainty Lass: a host sets a server up with `dainty-host setup` as
[restricted/docs/hosting.md](../../docs/hosting.md) describes, and players follow
[restricted/docs/players.md](../../docs/players.md). The guides kept here cover what a host does with their own copy
of the game:

| Guide | What's inside |
|---|---|
| [Installing Dauntless](docs/INSTALLING_DAUNTLESS.md) | Getting the 1.14.7 client from Epic with your own account and Legendary |
| [Generating the SDK](docs/GENERATING_SDK.md) | Dumping the Unreal SDK with Dumper-7 and building the CatalogExporter |
| [Generating game data](docs/GENERATING_GAME_DATA.md) | Exporting the catalogue and tables from your install |
| [Scope and legal](docs/SCOPE_AND_LEGAL.md) | What isn't included, responsible use, license details |
| [Security](SECURITY.md) | Safe configuration and how to report a vulnerability |

## 🗂️ Repository layout

| Folder | What it is |
|---|---|
| [`version-data`](version-data) | The 1.14.7 version data the MIT `rustic-isles-1-14-7` module reads at start (settings.toml's `version_data`) |
| [`game-data/1.14.7/backend`](game-data/1.14.7/backend), [`director`](game-data/1.14.7/director) | The backend's and the Director's game-data examples; a host's generated tables go beside them |
| `game-data/1.14.7/exports`, `captures` | A host's CatalogExporter exports and endpoint captures (never committed) |
| [`game-data/1.14.7/store-captures.json`](game-data/1.14.7/store-captures.json) | The pin Lady Luck's Store and the platinum store are built from on each server |
| [`../../../native/runtime-loader`](../../../native/runtime-loader) | `winmm.dll` proxy that loads the MIT runtime at startup, for Proton players who keep it |
| [`../../../native/rustic-isles-exporter`](../../../native/rustic-isles-exporter) | The CatalogExporter: exports compatibility data from your own installation (Dainty Lass's MIT Rust rewrite) |

## 💬 Community and help

- 🐛 **Bugs and feature requests:** open an issue on the
  [Dainty Lass](https://git.silentspud.com/silentspud/dainty-lass) tracker. Include the component, commit,
  configuration with secrets removed, reproduction steps and logs.
- 🔒 **Security problems:** report them privately — see [SECURITY.md](SECURITY.md).

Never submit game files, leaked source or secrets.

## 📜 License and credits

[AGPL-3.0-only](LICENSE), with additional terms in [ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md) — details in
[Scope and legal](docs/SCOPE_AND_LEGAL.md). This repository is a modified, unofficial version of Mystic Paradox and is
not an official Mystic Paradox release.

- "Mystic Paradox's Dauntless 1.12.0 and 1.14.7 ports and related modifications were developed by MysticFox / Pranav
  Karande. See NOTICE.md for contribution and provenance information."
- Based on **[Undaunted](https://github.com/SyST3MDeV/Undaunted)** by gwog / Gregory Morford (AGPLv3).
- **Mystic Paradox** is a separate community project with substantial independent changes, maintained
  by **Mystic / Pranav Karande**.

Full provenance in [NOTICE.md](NOTICE.md).
