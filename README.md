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
> host generates the SDK and the game data from their own copy ([docs/sdk-and-game-data.md](docs/sdk-and-game-data.md)).

## How it fits together

The launcher on a player's PC talks to the host's **gateway**, the one thing the server exposes: TLS on 443 with a
pinned certificate for a public server, plain HTTP inside the tailnet for a private one. Behind it sit the
***backend** (Rustic Isles's 1.14.7 backend and launcher API, re-created clean-room in Rust as the MIT
`rustic-isles-*` crates, on SQLite or Postgres), the **content server** (the game runtime, news and art) and the
**allowlist**, which opens the game's UDP ports only to players who have logged in. The **Director** (`rustic-isles-director`)
starts Ramsgate, the Training Grounds and the hunts. The game itself runs the host's runtime
(`rustic-isles-runtime.dll`, MIT, which the launcher and the Director inject) and reaches the backend through a relay
the launcher runs on `127.0.0.1:61100`.
[docs/architecture.md](docs/architecture.md) has the trust boundaries.

## What is in this repository

Dainty Lass is one monorepo, all of it **MIT OR Apache-2.0** ([LICENSE-MIT.txt](LICENSE-MIT.txt),
[LICENSE-APACHE.txt](LICENSE-APACHE.txt)). What came from AGPL projects was rewritten clean-room (RD27), and the
AGPL `restricted/` folder that held the rest was retired once its docs and data were remade.

| Folder | Licence | What |
| --- | --- | --- |
| [root](.) (`crates/`, `fuzz/`) | MIT OR Apache-2.0 | The MIT Cargo workspace, every package in `crates/<package name>` (libraries, the gateway, content, admin and chat services, core plugins, the Rustic Isles game module and runtime, the Rustic Rifts crates, the server executables, the launcher's Tauri shell `dainty-launcher-app`, and `dainty-lass`, the core binary), and `fuzz/` |
| [`ui/`](ui) | MIT OR Apache-2.0 | dainty-dash: the staff dashboard, the launcher and server-manager pages the Tauri shell serves, and the fleet agent (Astryx on Next.js through vinext, built with Bun) |
| [`launcher/`](launcher) | MIT OR Apache-2.0 | The launcher's JavaScript side: the isolation hook, the scripts the Tauri shell and the release run, and their tests |
| [`scripts/`](scripts) | MIT OR Apache-2.0 | `replay-capture.mjs`, the shadow-replay wrapper |
| [`docs/`](docs) | MIT OR Apache-2.0 | The host and player guides, the architecture, the plans and the security checklists ([SECURITY_REVIEW.md](SECURITY_REVIEW.md)) |
| [`modules/`](modules) | MIT OR Apache-2.0 | Rustic Isles' synthetic game-data placeholders and the store captures' pin, laid out as in a server tree; the version data is `crates/rustic-isles-1-14-7/data/` |

What is left to do: [MIGRATION_PLAN.md](MIGRATION_PLAN.md). The design record behind it:
[docs/dainty-lass-plan.md](docs/dainty-lass-plan.md).

## Building

The MIT Rust workspace, from the root (the toolchain is pinned in `rust-toolchain.toml`):

```bash
cargo test --workspace --locked
```

The UI (`ui/`) builds with Bun:

```bash
cd ui && bun install --frozen-lockfile && bun run bridge-data && bun run build
```

The full server, from the root (the release builds the launcher from `crates/dainty-launcher-app`):

```bash
cargo build --release --locked -p dainty-server --features all-modules --bins
```

## Credits and licence

Dainty Lass stands on [Mystic Paradox](https://github.com/pranav158/Mystic-Paradox) (Pranav Karande), whose Node
backend, Director and C++ runtime are the behaviour the MIT crates re-created clean-room (the runtime's source is kept
by the tag `agpl-runtime-final`);
on [Dauntless Revived](https://github.com/mixutin/dauntless-revived)
(mixutin and contributors), whose Rust crates the project started from and whose invite and account-key design
the launcher follows; and on [Undaunted](https://github.com/SyST3MDeV/Undaunted) (gwog and contributors), which both
of those build on.

Everything here is licensed under either of the MIT License ([LICENSE-MIT.txt](LICENSE-MIT.txt)) or the Apache
License, Version 2.0 ([LICENSE-APACHE.txt](LICENSE-APACHE.txt)), at your option; it was written or rewritten
clean-room from no other project. Unless you state otherwise, any contribution you submit for inclusion is
dual-licensed the same way, without further terms.

An unofficial fan project, not affiliated with or endorsed by Phoenix Labs, Epic Games, Mystic Paradox or
Dauntless Revived. Dauntless is a trademark of its owners.
