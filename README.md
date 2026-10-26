# DaintyLass

A private-server kit for **Dauntless 1.14.7**: a launcher for players, a server and host kit for whoever runs it,
and a module framework that is growing to hold more than one version of the game. Players join with one invite
and one account key; hosts run the server on a Windows PC for friends over Tailscale, on a Linux machine on the
internet, or in Docker.

- **Players:** install the launcher, paste the host's invite, pick a name, point it at your own Dauntless 1.14.7,
  press PLAY. [docs/players.md](docs/players.md)
- **Hosts, Windows:** [docs/hosting.md](docs/hosting.md)
- **Hosts, Linux:** the game servers run under Wine or Proton. [docs/hosting-linux.md](docs/hosting-linux.md)
- **Hosts, Docker:** the same server as containers. [docs/hosting-docker.md](docs/hosting-docker.md)

> [!IMPORTANT]
> Nothing here contains, downloads or mirrors the game. Every player needs their own Dauntless 1.14.7, and every
> host generates the SDK and the game data from their own copy ([docs/sdk-and-game-data.md](docs/sdk-and-game-data.md)).

## How it fits together

The launcher on a player's PC talks to the host's **gateway**, the one thing the server exposes: TLS on 443 with a
pinned certificate for a public server, plain HTTP inside the tailnet for a private one. Behind it sit the
**backend** (Mystic Paradox's, with a launcher API), the **content server** (the game runtime, news and art) and the
**allowlist**, which opens the game's UDP ports only to players who have logged in. The **Director** starts
Ramsgate, the Training Grounds and the hunts. The game itself loads the host's runtime (`MysticParadox.dll`
through `winmm.dll`) and reaches the backend through a relay the launcher runs on `127.0.0.1:61100`.
[docs/architecture.md](docs/architecture.md) has the trust boundaries.

## What is in this repository

| Folder | What |
| --- | --- |
| [`rust/`](rust) | One cargo workspace: `crates/` (`controller`, `protocol`, `proxy`, `service-kit`, `store`), `services/` (`gateway` with the allowlist, `content`, `admin`, `host`), `launcher/` (Tauri) and `bins/` |
| [`paradox/`](paradox) | Mystic Paradox's backend and Director, Node, with this project's patches ([docs/paradox-patches.md](docs/paradox-patches.md)), until the Rust port replaces them |
| [`native/`](native) | The game runtime (`paradox-runtime`, C++), its loader (`runtime-loader`, Rust) and the CatalogExporter, built by each host |
| [`docker/`](docker), `compose.yaml` | The server as containers |
| [`docs/`](docs) | Guides for players and hosts, the architecture, the migration plans and the security checklists |

Where it is going: [MIGRATION_PLAN.md](MIGRATION_PLAN.md) (the Rust port of the Node services) and
[docs/dainty-lass-plan.md](docs/dainty-lass-plan.md) (the module framework, the split into game-module
repositories, the network).

## Building

The Rust workspace, from `rust/` (the toolchain is pinned in `rust-toolchain.toml`):

```bash
cargo build --release -p host -p gateway -p content -p admin --bins
```

The Node services, Node.js 22:

```bash
cd paradox/backend && npm ci && npm test
cd paradox/director && npm ci && npm test
```

The launcher is `rust/launcher` (`npm ci && npm run tauri build`; its page is a stand-in until
`dainty-lass-ui` lands). The runtime and the CatalogExporter need a host's own SDK
([docs/sdk-and-game-data.md](docs/sdk-and-game-data.md)). `bash docker/run-tests.sh` builds the images and runs
every server suite inside them.

## Credits and licence

DaintyLass stands on [Mystic Paradox](https://github.com/pranav158/Mystic-Paradox) (Pranav Karande), whose backend,
Director and runtime are forked into `paradox/` and `native/`; on [Dauntless Revived](https://github.com/mixutin/dauntless-revived)
(mixutin and contributors), whose Rust crates the workspace started from and whose invite and account-key design
the launcher follows; and on [Undaunted](https://github.com/SyST3MDeV/Undaunted) (gwog and contributors), which both
of those build on. [NOTICE.md](NOTICE.md) says what came from where.

Free software under the GNU AGPL-3.0 (`AGPL-3.0-only`): [LICENSE.txt](LICENSE.txt). Additional terms apply to the
Mystic Paradox material: [ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md) and Mystic Paradox's own
[paradox/ADDITIONAL_TERMS.md](paradox/ADDITIONAL_TERMS.md). Mystic Paradox's Dauntless 1.12.0 port and related
modifications were developed by Pranav Karande. See NOTICE.md for contribution and provenance information.

An unofficial fan project, not affiliated with or endorsed by Phoenix Labs, Epic Games, Mystic Paradox or
Dauntless Revived. Dauntless is a trademark of its owners.
