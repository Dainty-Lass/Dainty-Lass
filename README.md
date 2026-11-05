# DaintyLass

A private-server kit for **Dauntless 1.14.7**: a launcher for players, a server and host kit for whoever runs it,
and a module framework that is growing to hold more than one version of the game. Players join with one invite
and one account key; hosts run the server on a Windows PC for friends over Tailscale, on a Linux machine on the
internet, or in Docker.

- **Players:** install the launcher, paste the host's invite, pick a name, point it at your own Dauntless 1.14.7,
  press PLAY. [restricted/docs/players.md](restricted/docs/players.md)
- **Hosts, Windows:** [restricted/docs/hosting.md](restricted/docs/hosting.md)
- **Hosts, Linux:** the game servers run under Wine or Proton. [restricted/docs/hosting-linux.md](restricted/docs/hosting-linux.md)
- **Hosts, Docker:** the same server as containers. [restricted/docs/hosting-docker.md](restricted/docs/hosting-docker.md)

> [!IMPORTANT]
> Nothing here contains, downloads or mirrors the game. Every player needs their own Dauntless 1.14.7, and every
> host generates the SDK and the game data from their own copy ([docs/sdk-and-game-data.md](docs/sdk-and-game-data.md)).

## How it fits together

The launcher on a player's PC talks to the host's **gateway**, the one thing the server exposes: TLS on 443 with a
pinned certificate for a public server, plain HTTP inside the tailnet for a private one. Behind it sit the
**backend** (Rustic Isles's 1.14.7 backend and launcher API, ported to Rust in `restricted/modules/rustic-isles`, on SQLite
or Postgres), the **content server** (the game runtime, news and art) and the
**allowlist**, which opens the game's UDP ports only to players who have logged in. The **Director** (`isles-director`)
starts Ramsgate, the Training Grounds and the hunts. The game itself loads the host's runtime (`Rustic-Isles.dll`
through `winmm.dll`) and reaches the backend through a relay the launcher runs on `127.0.0.1:61100`.
[docs/architecture.md](docs/architecture.md) has the trust boundaries.

## What is in this repository

DaintyLass is a monorepo with a strict licence boundary (RD27): the root workspace and everything outside
[`restricted/`](restricted) is **MIT** ([LICENSE.txt](LICENSE.txt)); [`restricted/`](restricted) is a separate
Cargo workspace licensed under **GNU AGPL-3.0-only** ([restricted/LICENSE.txt](restricted/LICENSE.txt),
[restricted/NOTICE.md](restricted/NOTICE.md)) holding the host shell, the Tauri launcher, the AGPL game modules,
the 1.4.4 importer, the runtime loader, the server binaries that link them, the Docker images, and the host and
player guides. No MIT crate at the root depends on anything in `restricted/`.

| Folder | Licence | What |
| --- | --- | --- |
| [root](.) (`crates/`, `fuzz/`) | MIT | The MIT Cargo workspace, every package in `crates/<package name>` (libraries, the gateway, content, admin and chat services, core plugins, the Rustic Rifts crates and the `dainty-lass-server` binary), and `fuzz/` |
| [`ui/`](ui) | MIT | dainty-dash: the staff dashboard, the launcher and server-manager pages the Tauri shells serve, and the fleet agent (Astryx on Next.js through vinext, built with Bun); its generated bridge types are copied from `restricted/ui` at build time |
| [`launcher/`](launcher) | MIT | The launcher's JavaScript side: the isolation hook, the scripts the Tauri shells and the release run, and their tests |
| [`scripts/`](scripts) | MIT | `replay-capture.mjs`, the shadow-replay wrapper |
| [`docs/`](docs) | MIT | The architecture, the plans and the MIT components' security checklists ([SECURITY_REVIEW.md](SECURITY_REVIEW.md)) |
| [`restricted/`](restricted) | AGPL-3.0-only | Separate Cargo workspace: the host shell (`restricted/crates/host`), the Tauri launcher (`restricted/launcher/src-tauri`), the AGPL game modules (`restricted/modules/`), the 1.4.4 importer (`restricted/legacy/`), the Apache-2.0 runtime loader (`restricted/native/`), the server executables, Docker (`restricted/docker/`, `restricted/compose.yaml`), and the host and player guides ([restricted/SECURITY_REVIEW.md](restricted/SECURITY_REVIEW.md)) |

Where it is going: [MIGRATION_PLAN.md](MIGRATION_PLAN.md) and [docs/dainty-lass-plan.md](docs/dainty-lass-plan.md).

## Building

The MIT Rust workspace, from the root (the toolchain is pinned in `rust-toolchain.toml`):

```bash
cargo test --workspace --locked
```

The UI (`ui/`) builds with Bun:

```bash
cd ui && bun install --frozen-lockfile && bun run restricted-data && bun run build
```

The full server, the Tauri launcher and the Docker images are built in [`restricted/`](restricted):

```bash
cd restricted && cargo build --release -p server-console --features all-modules --bins
```

## Credits and licence

DaintyLass stands on [Mystic Paradox](https://github.com/pranav158/Mystic-Paradox) (Pranav Karande), whose backend,
Director and runtime live in `restricted/modules/rustic-isles`; on [Dauntless Revived](https://github.com/mixutin/dauntless-revived)
(mixutin and contributors), whose Rust crates the workspace started from and whose invite and account-key design
the launcher follows; and on [Undaunted](https://github.com/SyST3MDeV/Undaunted) (gwog and contributors), which both
of those build on.

MIT License ([LICENSE.txt](LICENSE.txt)) at the root: everything outside [`restricted/`](restricted) was written or
rewritten clean-room from no other project. Everything inside [`restricted/`](restricted) is licensed under the GNU
AGPL-3.0-only ([restricted/LICENSE.txt](restricted/LICENSE.txt), [restricted/NOTICE.md](restricted/NOTICE.md)), with
the Apache-2.0 runtime loader in `restricted/native/runtime-loader` and the additional terms for the Mystic Paradox
material in [restricted/ADDITIONAL_TERMS.md](restricted/ADDITIONAL_TERMS.md).

An unofficial fan project, not affiliated with or endorsed by Phoenix Labs, Epic Games, Mystic Paradox or
Dauntless Revived. Dauntless is a trademark of its owners.
