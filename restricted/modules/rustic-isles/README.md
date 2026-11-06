# Rustic Isles (`isles-1-14-7`)

> **Modified, unofficial version.** This repository (`silentspud/rustic-isles` on `git.silentspud.com`) is a modified,
> unofficial version of [Mystic Paradox](https://github.com/pranav158/Mystic-Paradox) (`21b9ff5` and tag
> `dauntless-1.12.0` at `4d5dfaa`), maintained as the **Rustic Isles 1.14.7** game module of
> [DaintyLass](https://git.silentspud.com/silentspud/dainty-lass). It is **not** an official release of the Mystic
> Paradox project, and Mystic Paradox does not endorse it.
>
> "Mystic Paradox's Dauntless 1.12.0 and 1.14.7 ports and related modifications were developed by MysticFox / Pranav
> Karande. See NOTICE.md for contribution and provenance information."
>
> The Rust port of Mystic Paradox's 1.14.7 backend and Director that lived here (`isles-core`, `isles-director`,
> `isles-chat`, `isles-db`, `isles-1-14-7`) was replaced by DaintyLass's clean-room MIT `rustic-isles-*` crates (RD27)
> and removed; it stays in the history. What is left here is the library DaintyLass loads over the MIT module
> (`crates/isles-1-14-7-plugin`), its version data, the recorded goldens replayed against it (`crates/isles-mit-parity`),
> the Node reference under `reference/1.14.7` the port came from, and the runtime and tools under `native/`.

<h1 align="center">Rustic Isles</h1>

<p align="center">
  <b>Open-source private server for Dauntless 1.14.7</b> — backend, game-server orchestration and runtime
  compatibility layer, run as a game module of DaintyLass.
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: AGPL-3.0-only" src="https://img.shields.io/badge/license-AGPL--3.0--only-blue"></a>
  <a href="docs/DAUNTLESS_1_14_7_PORT.md"><img alt="Dauntless: 1.14.7" src="https://img.shields.io/badge/Dauntless-1.14.7-orange"></a>
  <img alt="Platform: Windows x64" src="https://img.shields.io/badge/platform-Windows%20x64-lightgrey">
</p>

<p align="center">
  <a href="#-get-started">Get started</a> ·
  <a href="#-documentation">Docs</a> ·
  <a href="docs/FAQ.md">FAQ</a> ·
  <a href="docs/DAUNTLESS_1_14_7_PORT.md">1.14.7 progress</a>
</p>

> [!IMPORTANT]
> Unofficial and community-developed. Not affiliated with, endorsed by, or sponsored by Phoenix Labs,
> Epic Games, Forte Labs, or any current or former Dauntless rights holder. This repository ships
> **source code only** — no game files, SDK or game data.

## 📊 Status

**Dauntless 1.14.7** (CL `647472`): Ramsgate, the Training Grounds, hunts, parties and saved progress (loot, XP,
weapon levels, reforge, mastery, bounties) work. Hubs and hunts run on dedicated servers or as **Paradox realms**
hosted on a player's PC — [progress notes](docs/DAUNTLESS_1_14_7_PORT.md) ·
[co-op and realms](docs/SELF_HOSTING.md#co-op-hunts-and-paradox-realms).

Expect active development, not production-grade uptime. The Gauntlet is not available yet.

## 🚀 Get started

1. **Read the [FAQ](docs/FAQ.md)** — what the project is, what it isn't, and where to get help.
2. **Install Dauntless 1.14.7** from Epic with your own account — [Installing Dauntless](docs/INSTALLING_DAUNTLESS.md).
3. **Generate the SDK** from your own 1.14.7 install — [Generating the SDK](docs/GENERATING_SDK.md).
4. **Generate the game data** from the same install — [Generating game data](docs/GENERATING_GAME_DATA.md).
5. **Deploy and play** — [Self-hosting](docs/SELF_HOSTING.md) walks from source to Ramsgate:
   prerequisites, DNS/TLS, services, launcher and the first acceptance test.

## 📚 Documentation

| Guide | What's inside |
|---|---|
| 🎮 [Installing Dauntless](docs/INSTALLING_DAUNTLESS.md) | Getting the 1.14.7 client from Epic with your own account and Legendary |
| 🏠 [Self-hosting](docs/SELF_HOSTING.md) | Full deployment: prerequisites, configuration helper, ports, validation, troubleshooting |
| 🧩 [Generating the SDK](docs/GENERATING_SDK.md) | Dumping the Unreal SDK with Dumper-7 and building the C++ projects |
| 📦 [Generating game data](docs/GENERATING_GAME_DATA.md) | Exporting the catalogue and tables from your install |
| 🔭 [1.14.7 port](docs/DAUNTLESS_1_14_7_PORT.md) | Progress, findings and open issues for the final game release |
| ❓ [FAQ](docs/FAQ.md) | Common questions, support and contact |
| ⚖️ [Scope and legal](docs/SCOPE_AND_LEGAL.md) | What isn't included, responsible use, license details |
| 🔒 [Security](SECURITY.md) | Safe configuration and how to report a vulnerability |

## 🗂️ Repository layout

| Folder | What it is |
|---|---|
| [`crates/isles-1-14-7-plugin`](crates/isles-1-14-7-plugin), [`crates/isles-mit-parity`](crates/isles-mit-parity), [`version-data`](version-data) | The library DaintyLass loads (its MIT `rustic-isles-1-14-7` with this version data), and the Node goldens replayed against it |
| [`reference/1.14.7/ParadoxBackend`](reference/1.14.7/ParadoxBackend) | The Node account and metagame backend the Rust port came from (login, characters, inventory, progression, store, party, co-op hunts, Paradox realms…) |
| [`reference/1.14.7/ParadoxDirector`](reference/1.14.7/ParadoxDirector) | The Node Director: starts and supervises the dedicated game servers for hubs and hunts |
| [`native/runtime`](native/runtime) | `Rustic-Isles.dll`, the C++ runtime loaded into the client and servers to adapt them to the private backend (MSVC + MinHook) |
| [`reference/1.14.7/shared/hunt-manifest`](reference/1.14.7/shared/hunt-manifest) | Hunt manifest resolution shared by the backend and the Director |
| [`tools/RuntimeLoader`](tools/RuntimeLoader) | `winmm.dll` proxy that loads the runtime at startup |
| [`native/catalog-exporter`](native/catalog-exporter) | Exports compatibility data from your own installation |

## 💬 Community and help

- 🐛 **Bugs and feature requests:** open an issue on the
  [DaintyLass](https://git.silentspud.com/silentspud/dainty-lass) tracker. Include the component, commit,
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
