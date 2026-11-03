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
**backend** (Rustic Isles's 1.14.7 backend and launcher API, ported to Rust in `modules/rustic-isles` on SQLite or
Postgres), the **content server** (the game runtime, news and art) and the
**allowlist**, which opens the game's UDP ports only to players who have logged in. The **Director** (`isles-director`)
starts Ramsgate, the Training Grounds and the hunts. The game itself loads the host's runtime (`Rustic-Isles.dll`
through `winmm.dll`) and reaches the backend through a relay the launcher runs on `127.0.0.1:61100`.
[docs/architecture.md](docs/architecture.md) has the trust boundaries.

## What is in this repository

DaintyLass is two repositories (RD27): this one holds the AGPL parts and builds the server, the launcher and the
Docker images; [dainty-lass](https://git.silentspud.com/silentspud/dainty-lass) holds the MIT crates (taken here as
git dependencies at one rev), the UI and the plan.

| Folder | What |
| --- | --- |
| [root](.) | One cargo workspace: `crates/` (`host`, the libraries of dainty-lass's MIT gateway, content and chat, and the server executables), `launcher/` (Tauri: `dainty-lass-client` and `dainty-lass-manager`), and the game modules' crates |
| [`modules/rustic-isles/`](modules/rustic-isles) | The Dauntless 1.14.7 game module (`isles-1-14-7`), `isles-core`, `isles-director`, `isles-chat`, the runtime's C++ source and CatalogExporter (`native/`), and the Node reference sources (`reference/1.14.7`) |
| [`modules/rustic-rifts/`](modules/rustic-rifts) | Peer-to-peer realms and the realm transport |
| [`legacy/`](legacy) | The importer for Dauntless Revived 1.4.4 and Reforged Revival installs |
| [`native/`](native) | The runtime loader (`runtime-loader`, Rust `winmm.dll` proxy) |
| [`docker/`](docker), `compose.yaml` | The server as containers |
| [`docs/`](docs) | Guides for players and hosts and the AGPL components' security checklists ([SECURITY_REVIEW.md](SECURITY_REVIEW.md)) |

## Building

The Rust workspace, from the root (the toolchain is pinned in `rust-toolchain.toml`); cargo fetches dainty-lass over
SSH, so your key must read git.silentspud.com:

```bash
cargo build --release -p server-console --features all-modules --bins
```

The desktop apps live in `launcher` (`npm ci && npm run build:client` for `dainty-lass-client` and
`npm run build:manager` for `dainty-lass-manager`, which use Bun to build the pages from dainty-lass's `dashboard/`); those builds and its
bridge tests (`npm test`) need a dainty-lass checkout beside this one, or `DAINTY_LASS_DIR`. The runtime and the
CatalogExporter need a host's own SDK ([docs/sdk-and-game-data.md](../dainty-lass/docs/sdk-and-game-data.md)).
`bash docker/run-tests.sh` builds the images (with your SSH agent) and runs every server suite inside them.

## Credits and licence

DaintyLass stands on [Mystic Paradox](https://github.com/pranav158/Mystic-Paradox) (Pranav Karande), whose backend,
Director and runtime live in `modules/rustic-isles`; on [Dauntless Revived](https://github.com/mixutin/dauntless-revived)
(mixutin and contributors), whose Rust crates the workspace started from and whose invite and account-key design
the launcher follows; and on [Undaunted](https://github.com/SyST3MDeV/Undaunted) (gwog and contributors), which both
of those build on. [NOTICE.md](NOTICE.md) says what came from where.

Free software under the GNU AGPL-3.0 (`AGPL-3.0-only`): [LICENSE.txt](LICENSE.txt). The MIT crates it builds on are
dainty-lass's. Additional terms apply to the
Mystic Paradox material: [ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md) and Mystic Paradox's own
[modules/rustic-isles/ADDITIONAL_TERMS.md](modules/rustic-isles/ADDITIONAL_TERMS.md). "Mystic Paradox's Dauntless 1.12.0 and 1.14.7 ports and related
modifications were developed by MysticFox / Pranav Karande. See NOTICE.md for contribution and provenance information."

An unofficial fan project, not affiliated with or endorsed by Phoenix Labs, Epic Games, Mystic Paradox or
Dauntless Revived. Dauntless is a trademark of its owners.
