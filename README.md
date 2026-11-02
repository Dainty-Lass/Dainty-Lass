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

DaintyLass is two repositories (RD27): this one holds the MIT code; [dainty-restricted](https://git.silentspud.com/silentspud/dainty-restricted)
holds the AGPL services (gateway, content, host, chat), the launcher, the game modules (Rustic Isles, Rustic Rifts),
the staff dashboard, the 1.4.4 importer, the server binaries and the Docker images, and builds the server and the
launcher from both.

| Folder | What |
| --- | --- |
| [root](.) | One cargo workspace, every package in `crates/<package name>` (libraries, services, core plugins and the `dainty-lass-server` binary), and `fuzz/` |
| [`ui/`](ui) | The launcher's page (SolidJS + Vite + Tailwind CSS, built with Bun); its language catalogs come from dainty-restricted |
| [`native/`](native) | The runtime loader (`runtime-loader`, Rust `winmm.dll` proxy) |
| [`docs/`](docs) | The architecture, the plans and the MIT components' security checklists ([SECURITY_REVIEW.md](SECURITY_REVIEW.md)) |

Where it is going: [MIGRATION_PLAN.md](MIGRATION_PLAN.md) and [docs/dainty-lass-plan.md](docs/dainty-lass-plan.md).

## Building

The Rust workspace, from the root (the toolchain is pinned in `rust-toolchain.toml`):

```bash
cargo test --workspace --locked
```

The UI (`ui/`) builds with Bun and needs a dainty-restricted checkout beside this one, or `DAINTY_RESTRICTED_DIR`:

```bash
cd ui && bun install --frozen-lockfile && bun run build
```

The server, the launcher and the Docker images are built from dainty-restricted.

## Credits and licence

DaintyLass stands on [Mystic Paradox](https://github.com/pranav158/Mystic-Paradox) (Pranav Karande), whose backend,
Director and runtime live in `modules/rustic-isles`; on [Dauntless Revived](https://github.com/mixutin/dauntless-revived)
(mixutin and contributors), whose Rust crates the workspace started from and whose invite and account-key design
the launcher follows; and on [Undaunted](https://github.com/SyST3MDeV/Undaunted) (gwog and contributors), which both
of those build on. [NOTICE.md](NOTICE.md) says what came from where.

MIT License ([LICENSE-MIT.txt](LICENSE-MIT.txt)): everything here was written or rewritten clean-room from no other
project ([NOTICE.md](NOTICE.md)). The runtime loader is Apache-2.0. The AGPL parts, and the additional terms for the
Mystic Paradox material, are dainty-restricted's.

An unofficial fan project, not affiliated with or endorsed by Phoenix Labs, Epic Games, Mystic Paradox or
Dauntless Revived. Dauntless is a trademark of its owners.
