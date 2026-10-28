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
| [`rust/`](rust) | One cargo workspace: `crates/` (`controller`, `module-api`, `epic`, `history-ship`, `protocol`, `proxy`, `service-kit`, `store`), `services/` (`gateway` with the allowlist, `content`, `admin`, `host`), `plugins/` (`network`, `chat`), `launcher/` (Tauri), `bins/` and `fuzz/` |
| [`ui/`](ui) | [`dainty-lass-ui`](https://git.silentspud.com/silentspud/dainty-lass-ui) submodule: the launcher and server GUI front-end (Astro + Preact + Tailwind CSS, built with Bun into `ui/dist/` and served by `rust/launcher`) |
| [`modules/rustic-isles/`](modules/rustic-isles) | [`rustic-isles`](https://git.silentspud.com/silentspud/rustic-isles) submodule: the Dauntless 1.14.7 (`isles-1-14-7`) and 1.12.0 (`isles-1-12-0`) game modules, `isles-core`, `isles-director`, `isles-chat`, `rifts-seam`, the `ParadoxRuntime` C++ source and CatalogExporter (`native/`), and the Node reference sources (`reference/1.14.7`, `reference/1.12.0`) |
| [`modules/rustic-rifts/`](modules/rustic-rifts) | [`rustic-rifts`](https://git.silentspud.com/silentspud/rustic-rifts) submodule: peer-to-peer realms, `rifts-transport` (WebRTC over `libjuice` and `turn-rs`), `rifts-guard`, `rifts-server`, and the client hosting bridge |
| [`modules/legacy-revived/`](modules/legacy-revived) | [`legacy-revived`](https://git.silentspud.com/silentspud/legacy-revived) submodule: the Dauntless 1.4.4 game module (`revived-1-4-4`), `metagame`, `deploy`, and the 1.4.4 client patches (`native/client-patches/`, `native/xinput1_3-proxy/`) |
| [`native/`](native) | The runtime loader (`runtime-loader`, Rust `winmm.dll` proxy) |
| [`docker/`](docker), `compose.yaml` | The server as containers |
| [`docs/`](docs) | Guides for players and hosts, the architecture, the migration plans and the security checklists ([SECURITY_REVIEW.md](SECURITY_REVIEW.md)) |

Where it is going: [MIGRATION_PLAN.md](MIGRATION_PLAN.md) (the Rust port of the Node services) and
[docs/dainty-lass-plan.md](docs/dainty-lass-plan.md) (the module framework, the split into game-module
repositories, the network).

## Building

Clone with submodules (`ui/`, `modules/rustic-isles`, `modules/rustic-rifts`, `modules/legacy-revived`):

```bash
git clone --recurse-submodules https://git.silentspud.com/silentspud/dainty-lass.git
```

The Rust workspace, from `rust/` (the toolchain is pinned in `rust-toolchain.toml`):

```bash
cargo build --release -p host -p gateway -p content -p admin --bins
```

The Node reference services, Node.js 22:

```bash
cd modules/rustic-isles/reference/1.14.7/ParadoxBackend && npm ci && npm test
cd modules/rustic-isles/reference/1.14.7/ParadoxDirector && npm ci && npm test
```

The UI (`ui/`) builds with Bun (`packageManager` in `ui/package.json`):

```bash
cd ui && bun install --frozen-lockfile && bun run build
```

The launcher is `rust/launcher` (`npm ci && npm run tauri build`, which uses Bun to build the page from `ui/`). The runtime and the CatalogExporter need a host's own SDK
([docs/sdk-and-game-data.md](docs/sdk-and-game-data.md)). `bash docker/run-tests.sh` builds the images and runs
every server suite inside them.

## Credits and licence

DaintyLass stands on [Mystic Paradox](https://github.com/pranav158/Mystic-Paradox) (Pranav Karande), whose backend,
Director and runtime live in `modules/rustic-isles`; on [Dauntless Revived](https://github.com/mixutin/dauntless-revived)
(mixutin and contributors), whose Rust crates the workspace started from and whose invite and account-key design
the launcher follows; and on [Undaunted](https://github.com/SyST3MDeV/Undaunted) (gwog and contributors), which both
of those build on. [NOTICE.md](NOTICE.md) says what came from where.

Free software under the GNU AGPL-3.0 (`AGPL-3.0-only`): [LICENSE.txt](LICENSE.txt). The controller, `module-api`,
`epic`, `history-ship`, the plugins, the admin service and the binaries, written here from no other project, are also
under the MIT License ([LICENSE-MIT.txt](LICENSE-MIT.txt); NOTICE.md lists the crates). Additional terms apply to the
Mystic Paradox material: [ADDITIONAL_TERMS.md](ADDITIONAL_TERMS.md) and Mystic Paradox's own
[modules/rustic-isles/ADDITIONAL_TERMS.md](modules/rustic-isles/ADDITIONAL_TERMS.md). Mystic Paradox's Dauntless 1.12.0 port and related
modifications were developed by Pranav Karande. See NOTICE.md for contribution and provenance information.

An unofficial fan project, not affiliated with or endorsed by Phoenix Labs, Epic Games, Mystic Paradox or
Dauntless Revived. Dauntless is a trademark of its owners.
