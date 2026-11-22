# Dainty Lass

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
> host generates the SDK and the game data from their own copy ([../docs/sdk-and-game-data.md](../docs/sdk-and-game-data.md)).

## How it fits together

The launcher on a player's PC talks to the host's **gateway**, the one thing the server exposes: TLS on 443 with a
pinned certificate for a public server, plain HTTP inside the tailnet for a private one. Behind it sit the
**backend** (Rustic Isles's 1.14.7 backend and launcher API: dainty-lass's clean-room `rustic-isles-1-14-7` with
`modules/rustic-isles`' version data, on SQLite or Postgres), the **content server** (the game runtime, news and
art) and the **allowlist**, which opens the game's UDP ports only to players who have logged in. The **Director**
(`rustic-isles-director`) starts Ramsgate, the Training Grounds and the hunts. The game itself loads the host's
runtime (the MIT `rustic-isles-runtime.dll`, injected by the launcher) and reaches the backend through a relay the launcher runs on
`127.0.0.1:61100`.
[../docs/architecture.md](../docs/architecture.md) has the trust boundaries.

## What is in this directory

`restricted/` is the **AGPL-3.0-only** subtree of the Dainty Lass monorepo (RD27). Since RB15 it holds no code: the
server, the launcher and the Docker images all build from the root's MIT workspace, and the server release adds this
folder's docs and data to the binaries.

| Folder | What |
| --- | --- |
| [`modules/rustic-isles/`](modules/rustic-isles) | The 1.14.7 module's version data (the server reads it at start) and game data (`game-data/1.14.7`: examples, exports, captures and the store pin). The module itself is the root's MIT `rustic-isles-*` crates |
| [`modules/rustic-rifts/`](modules/rustic-rifts) | The realms' docs and relay self-test. The crates are the root's MIT `rustic-rifts-*` |
| [`launcher/`](launcher) | The launcher's icon ([launcher/README.md](launcher/README.md)) |
| [`docs/`](docs) | Guides for players and hosts and the security checklists first written here ([SECURITY_REVIEW.md](SECURITY_REVIEW.md)) |

## Building

Everything builds from the monorepo root (the toolchain is pinned in `../rust-toolchain.toml`):

```bash
cargo build --release -p dainty-server --features all-modules --bins
```

The desktop apps are the root's MIT shell, `../crates/dainty-launcher-app`; the bridge tests are in `../launcher`
(`bun run test`). The root's CatalogExporter needs a host's own SDK; the game runtime is the root's MIT
`rustic-isles-runtime.dll`, which needs none ([../docs/sdk-and-game-data.md](../docs/sdk-and-game-data.md)).
`bash docker/run-tests.sh` from the root builds the images and runs every server suite inside them.

## Credits and licence

Dainty Lass stands on [Mystic Paradox](https://github.com/pranav158/Mystic-Paradox) (Pranav Karande), whose backend and
Director lived in `modules/rustic-isles` until RB15 (its runtime until R21, its CatalogExporter until 2026-11-13) and
whose version data and game data still do; on [Dauntless Revived](https://github.com/mixutin/dauntless-revived)
(mixutin and contributors), whose Rust crates the project started from and whose invite and account-key design
the launcher follows; and on [Undaunted](https://github.com/SyST3MDeV/Undaunted) (gwog and contributors), which both
of those build on. [NOTICE.md](NOTICE.md) says what came from where.

Free software under the GNU AGPL-3.0 (`AGPL-3.0-only`): [LICENSE.txt](LICENSE.txt). The MIT crates are
dainty-lass's. Additional terms apply to the
Mystic Paradox material: [ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md) and Mystic Paradox's own
[modules/rustic-isles/ADDITIONAL_TERMS.md](modules/rustic-isles/ADDITIONAL_TERMS.md). "Mystic Paradox's Dauntless 1.12.0 and 1.14.7 ports and related
modifications were developed by MysticFox / Pranav Karande. See NOTICE.md for contribution and provenance information."

An unofficial fan project, not affiliated with or endorsed by Phoenix Labs, Epic Games, Mystic Paradox or
Dauntless Revived. Dauntless is a trademark of its owners.
