<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# DaintyLass migration plan: one Rust codebase, game modules included

Status: draft, written 2026-10-22; the decisions in section 3 were made the same day and revised after a review the
same evening (RD5, RD15 changed; RD16 and RD17 added); section 5 was rewritten as parallel waves and RD18 added
afterwards; RD19–RD23 came from wave 0's delta lists (`docs/port/delta-*.md`); RD9 was changed in wave 3; RD26
records the server structure (2026-10-25); on 2026-10-26 `docs/dainty-lass-plan.md` expanded this plan into the
five-repository DaintyLass architecture (`dainty-lass`, `dainty-lass-ui`, `rustic-isles`, `rustic-rifts`,
`legacy-revived`), and its section 7 amendments are folded in below (2026-10-28 integration pass). Waves 0 to 3's
agent rows and the DaintyLass waves in `docs/dainty-lass-plan.md` are merged into `plan` (RD18). On 2026-10-31
DaintyLass narrowed to one game, Dauntless 1.14.7: `legacy-revived` and the 1.12 port were removed (`29631d2`), and
importing old installs moved to the `dainty-legacy` plugin.

Completed rows are ~~struck through~~ in the tables below, as in DR's plan, and the "What's left" table above
section 1 is kept current by each integration pass. The decisions, the reuse table and section 6 are reference, not
work, and are never struck through.

This plan replaces Dauntless Revived's Rust migration, which stopped on 2026-10-22 at `e1c5fec` on its `plan` branch
([its plan](https://github.com/mixutin/dauntless-revived/blob/plan/MIGRATION_PLAN.md)) and is not continued.
Most of what it built was written for code this repository already shares: the launcher, gateway, allowlist
helper and content server all started as Dauntless Revived's. This plan takes that Rust code over and fits it to a
module-hosting framework (`crates/module-api`), with Mystic Paradox's 1.14.7 in `modules/rustic-isles` and realms
and P2P in `modules/rustic-rifts`, so no Node is left on a player's PC or a host.

This repository began as a fork of the Dauntless Reforged Revival line. On 2026-10-26 that line was removed: its
Node projects (`legacy/`), preset, brand, docs and workflows are gone, its three infrastructure patches to Mystic
Paradox were rewritten, `main` was given a new root, and nothing is merged from it any more (RD16 is closed). Its
gameplay patches 0004–0039 stay in `modules/rustic-isles/` (behind `[switches]` until `rustic-freeza` extracts them),
attributed in `NOTICE.md`.

Row ids: **RA** = launcher, **RS** = gateway, allowlist and content, **RH** = host kit, **RP** = the Paradox fork,
**RB** = backend and Director (moved to **RU** and **RF** in `docs/dainty-lass-plan.md`), **RR** = repository and CI,
**RX** = cross-cutting, **RD** = decisions. See `docs/dainty-lass-plan.md` for the five-repository rows (**DS**,
**DF**, **DU**, **RU**, **RF**, **RV** (removed 2026-10-31: one game), **DN**, **DE**, **DH**, **DX**, **RZ**, **DL**). "DR" means Dauntless Revived
at `e1c5fec`; DR row ids (L27, S2, X4, …) point into its plan, where the method and the checks for each piece are
written down.

## What's left

Updated by the 2026-10-28 integration pass (see `docs/dainty-lass-plan.md` and `docs/port/handoff-ledger.md`) and
rebuilt on 2026-11-01 for one game. Merged into `plan`: waves 0 to 3 (RR1–RR5, RP1–RP4, RX1, RX3, RX4, RS1–RS4,
RH1–RH6, RA1–RA9, RA11–RA13) and the DaintyLass architecture rows (`DS2`–`DS7`, `DF1`–`DF8`, `DU1`–`DU4`,
`RU1`–`RU7`, `RU10`, `RF1`–`RF5`, `RF7`, `DN2`–`DN13`, `DE1`–`DE7`, `DH1`–`DH7`, `DX1`–`DX4`, `DL1`–`DL4`).
`RV1`–`RV3` were merged too, then removed with `legacy-revived` on 2026-10-31.

Merged since 2026-10-28: the RU9 replay runbook (`reforged-host db replay`, `scripts/replay-capture.mjs`,
`docs/canary.md`; `92b22cb`), DN5, DN6, DN8 and DN9's matchmaking ring, redirects and Ramsgate (`e50393e`), timed
bans through `module-api` (RD25, `2678d18`), the removal of Dauntless Revived 1.4.4 and the 1.12 port (`29631d2`) and
DD9's 1.12.0 opt-in (`efeee7b`).

[#1]: https://github.com/SilentSpud/Dauntless-Reforged-Revival/pull/1
[#9]: https://github.com/SilentSpud/Dauntless-Reforged-Revival/pull/9
[#16]: https://github.com/SilentSpud/Dauntless-Reforged-Revival/pull/16
[#17]: https://github.com/SilentSpud/Dauntless-Reforged-Revival/pull/17
[#19]: https://github.com/SilentSpud/Dauntless-Reforged-Revival/pull/19
[#23]: https://github.com/SilentSpud/Dauntless-Reforged-Revival/pull/23
[#24]: https://github.com/SilentSpud/Dauntless-Reforged-Revival/pull/24
[#25]: https://github.com/SilentSpud/Dauntless-Reforged-Revival/pull/25
[#26]: https://github.com/SilentSpud/Dauntless-Reforged-Revival/pull/26
[#27]: https://github.com/SilentSpud/Dauntless-Reforged-Revival/pull/27

| Area | Done | Next | Waiting on the maintainer |
| --- | --- | --- | --- |
| Repository and CI | RR1–RR5, DS1–DS7, DX1–DX4, RX3 (`SECURITY_REVIEW.md`) | — | Make `CI ok` the required check on `plan`; DS8 (`rustic-freeza`), then RZ1–RZ4; DL5 legal read, then DX5 GitHub publication |
| Paradox & game modules (`modules/*`) | RP1–RP4 (moved to `modules/rustic-isles`), RU1–RU7, RU10, RB11, RB13, RF1–RF5, RF7; RU9/RB14 done in code (maintainer decision 2026-10-28; canary and shadow replay not run); RU11 and RV1–RV3 removed 2026-10-31 (one game) | The rest of RU5 (the QoS listener on `QOS_PORT`, the `MultiplyRewardItems` progression hook); the rest of RF6 (Guard's enforce mode: grant-class and draft-token cap checks) | RU8/RB2 live traffic captures; the RU9 canary, then RB15 one release after it; RF8 two-PC NAT check |
| Gateway, allowlist, content | RX1, RS1–RS5, RX2's lists, DF4a, DF4b | — | RS4's nft and Windows Firewall checks ([#9]); the shared check on the canary |
| Host kit & controller | RH1–RH6, RD24 (`host.toml`), DF2–DF5, DE6, DH1–DH7, DL3a/DL3b | — | Publish a server prerelease and follow `docs/hosting-linux.md` on a clean Debian 13 |
| Launcher & UI (`ui/`) | RX1, RA1–RA9, RA11–RA13, DF8, DU1–DU4, DE1–DE7, DL4a/DL4b; DU5 in part (its two-PC check is RF8's) | RA14 packaging; RA15 and RA16 (the Electron bridge, then retiring Squirrel) | RA10 and RA12a Linux spikes; CatalogExporter through the loader in a real 1.14.7 client ([#27]) |
| Network (`plugins/network`) | DN2–DN13 | — | DN1 spike (1.14.7 client relay switch); DN14 four-region canary |
| Server structure (RD26, DF3–DF5) | Four executables (`dainty-lass-client`, `dainty-lass-server-gui`, `dainty-lass-server-console`, `dainty-lass-server-minimal`); `crates/module-api`; all modules and plugins wired (`DF3`–`DF5`) | — | Merge `plan` into `main` when ready; the release dry runs (`server-release.yml` and the Tauri release workflow); play through `serve` against a real 1.14.7 client |

Local notes, not plan changes: on the maintainer's Windows PC two of Content's test ports fall in Windows'
reserved range, and the npm config refuses git-sourced packages, which blocks the launcher's `npm ci`.

## 1. End state

| Component | Today | After |
| --- | --- | --- |
| `ReforgedLauncher` | Electron 44, Node main process (~8.2k lines with `main.ts` and `preload.ts`), Squirrel, Forge, Windows only; plays 1.14.7 and, through `src/main/revived/`, Dauntless Revived's 1.4.4 servers | Tauri 2 on DR's launcher backend (`src-tauri`, ~20k lines), the same TS page, isolation pattern, `tauri-plugin-updater`; Windows (NSIS) and Linux (AppImage, deb, rpm) (RD4); 1.14.7 only (RD5) |
| `ReforgedGateway` (+ allowlist helper) | Node, `node-forge` (~2.8k lines) | DR's Rust `gateway` (bins `gateway`, `allowlist`) with Reforged's routes, ports and nft changes |
| `ReforgedContent` | Node (~1.4k lines) | DR's Rust `content` with Reforged's runtime manifest added |
| `ReforgedHost` | Node TS CLI (~4.8k lines with `linux/`) | Rust bin `reforged-host`, same commands (RD3) |
| ParadoxBackend | Mystic Paradox at a pinned commit + 33 patches, downloaded by `setup`; Node, Express, `ws`, `ltx`, `mongodb`, `@node-rs/argon2`, `jsonwebtoken`, `pino` (~28.2k lines, 54 test files, 25 Mongo repositories, ~2.5k lines of XMPP in `realtime/`, plus `scripts/`: game-data generators, admin bootstrap, data migrations) | Rust bin `backend`: `tokio`, `axum` on `service-kit`; data in **Postgres** through Diesel on `store`, moved once per host from MongoDB (RD15, RB14); `quick-xml` XMPP from DR, `argon2`, `jsonwebtoken`, `tracing`; the scripts as `backend` subcommands (RB11) |
| ParadoxDirector | Same pin; Node, Express, `jsonwebtoken`, `pino` (~2.6k lines, 5 test files) | Rust bin `director` on `service-kit`; starts the game servers as today |
| ParadoxRuntime, RuntimeLoader (`MysticParadox.dll`, `winmm.dll`) | C++, patched upstream, built by each host | The same, in `native/`; still built by each host from their own SDK; Freeza's runtime patches keep being applied here after the cut-off (RD16) |
| `upstream/` | Pin + patches + `store-captures.json` | Gone; `store-captures.json` moves next to the backend, attribution in `NOTICE.md` |
| Releases | Launcher (Squirrel) only | Launcher (Tauri feed) and a server release of Rust binaries only, for Windows and Linux (RD8). Hosts need Postgres, nothing else (MongoDB and Node.js until RB14) |

## 2. What is reused from DR

Measured 2026-10-22 by diffing Reforged's Node sources against DR's at `e1c5fec`. "Delta" is the Reforged change
that has to be carried onto the Rust code; line counts are changed lines in the Node files, a rough guide only.

| DR piece (at `e1c5fec`) | Reforged counterpart | Reuse | Delta to carry over |
| --- | --- | --- | --- |
| `rust/crates/protocol` (invite, status, manifest, account) | `src/shared/invite.ts`, `status.ts`, `types.ts` | Most | One scheme, `reforged-revival://` (a `dauntless-revived://` invite is refused with a message, RD5); `RRK_` keys; the runtime manifest type |
| `rust/crates/proxy` (hop-by-hop, WebSocket tunnel) | relay and gateway proxying | Whole | None expected; `/__origin/<host><path>` rewriting stays in the relay |
| `rust/crates/service-kit` (axum setup, pino logs, env config, the X7 service contract) | gateway, content, host kit, backend and Director start-up | Whole | None |
| `rust/crates/store` (Diesel pools, the synchronous closure API `store::read`/`store::write` on the blocking pool, embedded migrations, D10, D11, D21) | none: Reforged's data is in MongoDB today | Core, on Postgres (RB3) | The connection type becomes `PgConnection` (DR kept it to one alias for this, D21); the SQLite-only parts (PRAGMAs, online backup, Drizzle adoption, offline saves) are dropped; `pq-sys` with its `bundled` feature so no host or CI machine needs libpq installed, checked by a spike on MSVC first (RB3) |
| `rust/services/gateway` (TLS, policy, IP checks, limits, redaction, feed, `cert.rs`, `allowlist` bin) | `ReforgedGateway/src` (allowlist `state`, `sync`, `ip`, `ratelimit`, `redact`, `log` identical) | Most | `policy.ts` (~108 lines: `/reforged/api` admin routes, Paradox's host-only routes and headers), `config.ts`, `gateway.ts`, `feed.ts`, `server.ts`; new `allowlist/nft.ts` and `ports.ts` (the game's UDP range from `ALLOWLIST_PORTS`, which `--max-hunts` widens); `tools/make-cert.js` becomes `gateway make-cert`; DR's Node `capture.ts` ported in (RD19); the allowlist details in RD20 |
| `rust/services/content` (ranges, news, branding, auth) | `ReforgedContent/src` (`branding`, `news`, `limits`, `log` identical) | Most | New `runtime.ts` (the two-file runtime manifest and download); `app.ts` (~218 lines: no 11 GB game zip), `auth.ts` against the backend's key check, `config.ts` |
| `rust/services/metagame`: `chat/` (XMPP over WebSocket on `quick-xml`: connection, MUC rooms, presence, nicks; ~3.9k lines), `js.rs` (JSON written as `JSON.stringify` does, `localeCompare`), `origin.rs`, `redact.rs`, `bodylog.rs`, the auth extractors | ParadoxBackend's `realtime/` (`ltx`, `ws`), its JSON replies, its middleware | Pieces, copied into `services/backend` (RB rows) | Paradox's own rooms (City rooms, patch 0025), JIDs and XMPP auth; the routes and game logic are Paradox's, not DR's 1.4.4 ones |
| `service-kit`'s Express-compatible routing and body parsers (DR M8) | Express in both Paradox services | Whole | Case-insensitive routes, trailing slashes and body limits as Express does them, so the game client sees the same answers |
| DR's shadow-traffic method (M2 capture at the gateway, M35 replay and diff) | none | Method and the gateway's capture option | Recorded against the Node backend on MongoDB, replayed against Node on MongoDB and Rust on a Postgres copy of the same snapshot (RB12) |
| `rust/services/deploy`, `discord-bot`, the rest of `metagame` | none | Not brought in | — |
| DR's server kits' Rust subcommands and `server-release.yml` (M38, S6) | `ReforgedHost`, `ReforgedHost/linux/` | Method and release workflow | The host kit's commands are Reforged's own (RH rows) |
| Launcher `src-tauri`: `net/pinned.rs`, `net/http.rs`, `net/relay.rs`, `net/hostapi.rs`, `net/verify.rs`, `net/downloader.rs` | `pinned.ts` (identical), `http.ts`, `relay.ts`, `hostapi.ts`, `verify.ts`, `revived/downloader.ts` | Most; `downloader.rs` not brought in (RD22) | Relay: port 61100, `/__origin/` paths passed through unchanged (the gateway and backend unwrap them), `/QoS` answered locally, DR's owner check kept (RD21); the runtime's two files through a small `request_to_file` (RD22); verify: the 1.14.7 executable hash |
| Launcher `platform/keystore` (Windows DPAPI, Linux Secret Service and KWallet, `legacy.rs` safeStorage reader; DR L27, D7) | `keystore.ts` (~14 lines changed) | Whole | One key per server, bound to the certificate fingerprint; `RRK2` and bare keys only (RD22); Reforged's `userData` folder and app name |
| Launcher `platform/linux_runtime.rs` (Proton and Wine, DR L32) | none | Whole | Run 1.14.7 under Proton with the runtime's `winmm.dll` loaded (`WINEDLLOVERRIDES`), `REFORGED_BACKEND_URL` passed through, Engine.ini/Game.ini inside the prefix (RA10) |
| Launcher `platform/settings.rs`, `log.rs`, `system.rs`, `window.rs`, `invite_link.rs`, `game_folder.rs`, `game_config.rs`, `launch.rs` | `settings.ts`, `log.ts`, `system.ts`, `links.ts`, `game-folder.ts`, `engineini.ts`, `launch.ts`, `display-repair.ts` | Most | Engine.ini XMPP section and Game.ini `[OnlineSubsystemPhoenix]` removal; the 1.14.7 command line and `REFORGED_BACKEND_URL`; Legendary/Heroic/Epic detection |
| Launcher `controller/`, `bridge/` (typed commands, generated TS types, isolation hook) | `controller.ts` (~2.2k lines changed), `ipc-validate.ts`, `preload.ts` | Structure | The largest delta: Host page state and runtime acceptance |
| Launcher `platform/updater.rs`, Squirrel bridge (DR L45–L50), Linux bundles (L47) | `update-electron-app` + Squirrel | Whole | Feed URL and version line (RD7) |
| Launcher `net/autoregion.rs`, `platform/client_mods.rs`, the 1.4.4 manifest and DLL pins | `revived/api.ts`, `revived/constants.ts`, `revived/dlls.ts`, `revived/manifest.ts` | Not brought in (RD5) | — |
| Launcher `offline/`, `controller/offline.rs` | none | Not brought in: Reforged has no offline mode | — |
| Launcher page split per language (DR L13–L15) | `src/shared/i18n.ts` (1.6k lines, en + pt-BR) | Method | Two languages instead of nine |
| CI by family, `cargo deny`, fuzz targets, `SECURITY_REVIEW.md` checklists (DR R8, X6, X9) | `ci.yml`, `launcher-release.yml` | Most | No metagame or bot jobs; a Node job for the Paradox fork |

Not in DR at all, so written new in Rust: the Host page backend (`hostkit.ts`, `hostserver.ts`, `requirements.ts`,
~980 lines), loading the host's chosen DLL through the runtime loader (RD9; `injector.ts` is not ported), the runtime
files (`runtime-files.ts`, ~290 lines), the whole host kit (RH rows) and the whole backend and Director (RB rows).

Rough share of the work as planned: backend and Director (with the Postgres move) about half; launcher a quarter;
host kit and front door the rest.

## 3. Decisions

Made 2026-10-22; RD5 and RD15 changed and RD16, RD17 added after the review the same day. RD24 and RD25
added 2026-10-24, and RD5 reversed behind the `legacy` feature the same day, then restored on 2026-10-31 (one
game). RD26 added 2026-10-25.
Amended 2026-10-26 by `docs/dainty-lass-plan.md` (DD1–DD26, section 7) and folded in on 2026-10-28.

| # | Topic | Decision | Consequence |
| --- | --- | --- | --- |
| RD1 | Mystic Paradox | **Amended (DD2, DS2, DS6): a submodule at `modules/rustic-isles`**, forked with Mystic Paradox's full history (`21b9ff5` on `main`, `replay` at `00d3be8` + the 39 patches, `upstream` = `1.12` at `9b9138e`). The 1.14.7 Node backend and Director (`reference/1.14.7`), runtime and preset live in `modules/rustic-isles/` alongside the Rust workspace (`isles-core`, `isles-db`, `isles-chat`, `isles-director`, `isles-1-14-7`; the 1.12.0 tree and `isles-1-12-0` were removed 2026-10-31: one game), while the generic `RuntimeLoader` (`winmm.dll`) stays at `native/runtime-loader`. (Was: merged as a subtree at `paradox/` with `ParadoxRuntime` at `native/paradox-runtime`.) Paradox's `ADDITIONAL_TERMS.md` stays in `modules/rustic-isles` and `modules/rustic-rifts` and its attribution is in `NOTICE.md` | Hosts no longer download anything from Paradox's repository. Each module repository has its own history and integration pass |
| RD2 | Backend and Director | **Ported to Rust in `modules/rustic-isles` and `modules/rustic-rifts`** (RU and RF rows in `docs/dainty-lass-plan.md`, replacing the RB rows below; no gate per DD6). Each Paradox repository is a Rust trait in `isles-store` with a two-engine (`store::Database`, SQLite and Postgres) implementation (DD15); there is no MongoDB implementation. **Maintainer decision (2026-10-28, departing from RB12 and RB14):** cut over to the Rust 1.14.7 backend and Director in code now without running the canary week (RB14/RU9) or replaying a recorded shadow-traffic corpus (RB12/RU8). No canary was run and no recorded traffic was replayed. The Node services in `modules/rustic-isles/reference/` and `--features paradox` / `reforged-host db export-mongo` stay for one release as a rollback path until RB15 | No Node or MongoDB on hosts on the default path |
| RD3 | Host kit | **Ported to Rust** as `rust/services/host`, bin `reforged-host`, with the same commands (`setup`, `start`, `invite`, `doctor`, `gamedata`, `preset`, `multipliers`, `publish-runtime`, `update`, plus `install-game`, `network`, `history` and `guarded`) and the same files it writes (`host.toml` under RD24, and the `.env` files). It starts every service under one process tree (Windows job object, Linux process group) or in-process via `crates/controller` (`serve`) | One binary drives the whole server; the Host page runs it instead of `node dist/cli.js` |
| RD4 | Launcher platforms | **Windows and Linux.** DR's Linux code comes with it: Proton/Wine runtime, Secret Service and KWallet keystore, WebKitGTK 4.1 (DR D3), AppImage, deb and rpm. The Host page on Linux (SDK and game-data generation against the 1.14.7 client under Proton, the runtime loader loading Dumper-7 or CatalogExporter in that Wine process, RD9) is a spike with a kill criterion (RA12a): if the spike fails, the Host page stays Windows-only and this row is amended | 1.14.7 and the runtime have to be shown to work under Proton first (RA10, a spike); Linux CI and release jobs |
| RD5 | DR's 1.4.4 servers | **Not played: a `dauntless-revived://` invite is a typed refusal** (`protocol::invite`). History: reversed behind the `legacy` feature on 2026-10-24 (`c1be7f7`) and moved to `modules/legacy-revived` (`revived-1-4-4`, RV1–RV3) on 2026-10-26; **removed 2026-10-31: one game** (`29631d2`). A 1.4.4 server's accounts and bans import through `dainty-legacy` | 1.14.7 (`rustic-isles`) is the one game module on `crates/module-api` |
| RD6 | How DR's Rust code comes in | **Snapshot copy** at `e1c5fec` in one commit, with the source commit in the message and in `NOTICE.md`. DR is not continued, so nothing flows either way afterwards | From the copy on, the code is Reforged's own |
| RD7 | Launcher updates | DR's design (D2): `tauri-plugin-updater` with ed25519 signatures. Existing installs take a one-time bridge, Electron **1.6.0**, which downloads the Tauri **2.0.0** installer, checks its SHA-256 compiled into the bridge, runs it and removes the Squirrel install. The Squirrel feed is retired with 3.0.0; the Electron key copies are kept until then so a rollback works | One extra Electron release; 1.5.x installs that skip it before 3.0.0 reinstall by hand |
| RD8 | Server binaries | **Released by CI**: a server release workflow (from DR's `server-release.yml`) builds the server executables (`dainty-lass-server-console` / `reforged-host` and `dainty-lass-server-minimal`, DF5) for `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`, in one archive per OS, with `SHA256SUMS.txt` and provenance attestations. `setup` and `update` download and check it. Until RU9 the archive also carries the built Node backend and Director, and hosts keep their own Node.js | No Rust toolchain on hosts, and no Node once RU9 ships; `update` stops being a `git` fast-forward of a checkout |
| RD9 | Loading Dumper-7 and CatalogExporter | **No injector** (changed in wave 3; was a remote-thread injector on the `windows` crate). The Host page has the runtime's own loader (`winmm.dll`, `native/runtime-loader`) load the DLL the host picked at game start, through the `mystic_loader.ini` list the loader already reads next to itself: the launcher copies the picked DLL next to the loader, writes the ini naming only it, starts the game it launches, and removes both when that game exits. In a dump session no `MysticParadox.dll` is present, so the loader loads the picked DLL alone. Before every ordinary PLAY the launcher refuses to start while a `mystic_loader.ini` or an unknown DLL sits next to the loader, so a leftover never rides along with the runtime | No process-injection code anywhere in the launcher, and no change to the loader. Under Proton it works the same way, since `WINEDLLOVERRIDES` already loads the loader (RA10, RA12a). The ini handling and the pre-launch check get their own security checklist (RX3) |
| RD10 | Security | **DR's full pass** (D26, X9): a checklist per trust-boundary component in `SECURITY_REVIEW.md`, `cargo-fuzz` targets for every parser of untrusted input run briefly on every push, a 24-hour fuzz run and a signed checklist before each component's cutover | Replaces a second reviewer where a mistake costs most |
| RD11 | Repository layout | **Amended (DD2, section 4.1): five repositories** (`dainty-lass` with `rust/`, `native/runtime-loader`, `docs/`, `docker/`; submodules `modules/rustic-isles`, `modules/rustic-rifts` and `ui/` for `dainty-lass-ui`; `modules/legacy-revived` was removed 2026-10-31: one game) | Each repository has its own history and CI; `dainty-lass` pins them as submodules and git/registry crates |
| RD12 | Order | **Amended (DD6, RD18): parallel waves across all five repositories** (`docs/dainty-lass-plan.md` section 5), synced on seam crates (`module-api`, `store`, launcher bridge, `isles-*`, `rifts-*`) | Each track's cutover still waits for its own checks |
| RD13 | Test environment | **A canary run by the maintainer**, like DR's D23: a small server on the Linux kit with a copy of a real database and invited testers. The shared checks, the traffic capture (RB2), the shadow replay (RB12/RU8) and every first cutover run there; then the hosts | The Linux path is tested first |
| RD14 | Languages | en and pt-BR, as today, split one file per language as DR did (L13), so a third language is data only | — |
| RD15 | Database | **Amended (DD15, DF7): SQLite by default, Postgres where concurrency pays**, through `store::Database` (`AnyConnection` over `SqliteConnection` and `PgConnection`, `pq-sys` `bundled_without_openssl`) with the hash-chained change log on every write (DD23, DH1–DH5). `rustic-isles` (`isles-core`) runs on both engines, and `store::transfer::move_database` moves a database between SQLite and Postgres in one transaction. A host's MongoDB data moves with `isles-store`'s `db migrate` / `db verify` / `db export-mongo` (RU7) | Small and single-PC servers need no database server; public and networked servers can run Postgres without changing a line of module code |
| RD16 | Upstream | **Closed 2026-10-26: the upstream line was removed and nothing is merged from it.** Its gameplay patches 0004–0039 are gated behind `[switches]` in `host.toml` and move to the optional `rustic-freeza` plugin (DD25, RZ1–RZ4) | RX2's catch-up lists are closed |
| RD17 | Gate before the backend | **Amended (DD6): no gate for the Paradox port.** The Rust port of Mystic Paradox (`modules/rustic-isles`, RU rows) and realms (`modules/rustic-rifts`, RF rows) ran in parallel with the framework and UI rows; only the production cutover (RU9) waits on the shadow replay (RU8) and canary checks | The port and the framework finish together |
| RD18 | How the rows are done | **One row, one agent, in parallel waves across five repositories** (section 5, `docs/dainty-lass-plan.md` section 5). Each repository has its own integration pass merging its rows into its own `plan` branch, so the review limit (about eight rows in flight) applies per repository, not to the whole plan. Seams (`DF2` `module-api`, `DF7` `store`, `DF8` bridge, `RU4`, `RF2`) land first | The canary runs, cutovers, spikes that need a machine with the game, signing and publishing stay with the maintainer |
| RD19 | Traffic capture | **Ported into the Rust gateway** (RS3) from DR's Node `capture.ts`; DR's Rust gateway at `e1c5fec` has none and Reforged's Node gateway never had one. Off unless configured, redacting as `capture.ts` does | RU8 records on the Rust gateway; nothing is added to the Node gateway |
| RD20 | Allowlist helper and gateway details | **Reforged's behavior where the two differ, DR's where Reforged has nothing.** nft: one atomic transaction swapping the whole table, with Reforged's `lo` rule (not DR's prepare-then-flush, which briefly empties the sets); `nft -f -` tried first, a `0600` file in a `0700` folder if Debian 13 refuses stdin; `nft` at its configured path, no PATH lookup. Env: only the `ALLOWLIST_*` names the host kit writes, not DR's `DR_*`; `ALLOWLIST_POWERSHELL` accepted and ignored (Windows Firewall goes through COM). DR's feed backoff from a helper that is down kept, listed in RS3's PR as a change from Node. `/game/login` feeds the allowlist only if Paradox checks the bearer before answering 2xx (RS3 checks against the fork); otherwise only `/heartbeat`, as today. The certificate subcommand is `gateway make-cert`, as in DR and `npm run make-cert` | An existing host's `.env` starts the Rust helper unchanged |
| RD21 | Relay owner check | **DR's kept on Windows** (`net/relay_owner.rs`: only connections whose socket belongs to the game process the launcher started are served), stricter than Reforged's Node relay. On Linux, RA10 decides whether the game process can be found under Proton; if not, the relay serves loopback only, as today. The Dumper-7 session's game does not use the relay and is not checked | A trust-boundary control at no cost on Windows; RX3 covers it |
| RD22 | Launcher behaviors that differ from DR | **Amended (DD12, DD22, DE5, DN13):** **1.4.4 servers** are not supported (they were through `revived-1-4-4`, RV2, from 2026-10-26 until its removal on 2026-10-31: one game). **DR's `net/downloader.rs`** is brought back to download and resume game versions from Epic's CDN (`crates/epic`, DE5), while the runtime's two files still use `request_to_file` in `net/http.rs`. **Stored usernames** update when a login reply carries a new name after a network join clash (DN13). **Missing Visual C++ files** only warn before PLAY; **shutdown** waits for downloads and refuses PLAY; **keystore** reads `RRK2` and bare keys; **copying text** stays a bridge command with its 4096-character limit | The 1.14.7 launcher with Epic install and resume |
| RD23 | Host page target | **The planned layout only.** RA12 drives `reforged-host` from the server release (RD3, RD8), never `npm run setup` or `node dist/cli.js`; `publish-runtime` is the host kit's command, which the Host page calls, so the version format and `runtime.json` exist once. `reforged-host` keeps today's process contract: `REFORGED_CONTROL=stdin`, the `REFORGED_SERVER_READY` line and the port 61000 probe (RH2) | RA12 waits for RH2; the Node-kit code paths in `hostkit.ts`, `hostserver.ts` and the controller (`upstream`, the git fallback) are not ported |
| RD24 | Host configuration format | **TOML for the files hosts write** (changes section 6's host-kit line). `host.json` becomes `.workspace/config/host.toml`, with the same fields and checks under snake_case keys, and a left-out key in place of `null` (`hosttoml.rs`). Writes patch the file with `toml_edit`, so a host's comments and layout survive `setup`. A Node kit's `host.json` is still read while there is no `host.toml`. The first write (`setup`, which `update` runs) moves it over and keeps it as `host.json.bak`. The launcher's Host page reads `host.toml` first. Extended with `[modules.<id>]`, `[switches]`, `[network]`, `[history]`, and `[relay]` (`docs/hosting.md`) | Phase 1 and the DaintyLass module/network/history/relay/switches tables landed. Every setting is a key in `hosttoml.rs`'s table with a comment for new files (AGENTS.md) |
| RD25 | Admin API | **Amended (DD23, DH2, DH3b, DH6, DN11): per-person admin keys with roles (`owner`, `admin`, `moderator`, `reader`), history revert, guarded actions, and network-wide moderation** (`rust/services/admin`, `docs/admin-api.md`). The existing `REFORGED_ADMIN_API_KEY` in `secrets.env` becomes the `owner` key; additional per-person keys are issued and revoked by command and recorded as the `Actor` in `store`'s change log. Destructive and bulk actions wait on a delay or second operator approval (`guarded.rs`, DH6), and in a network bans, timeouts and kicks propagate through the player's home node to every peer (DN11) | Every admin mutation names the operator key that made it and can be audited and reverted |
| RD26 | Server structure | **Amended (DD5, DF2–DF5): four executables and `crates/module-api`** (2026-10-25, updated 2026-10-26). `crates/controller` and `crates/module-api` host the game module (`isles-1-14-7`; `isles-1-12-0` and `revived-1-4-4` removed 2026-10-31: one game) and core plugins (`plugins/network`, `plugins/chat`, `rifts-*`) in-process or via the module-host subprocess frame protocol (`host::module_host`). **Four executables** (DF5): `dainty-lass-client` (`rust/launcher/src-tauri`), `dainty-lass-server-gui` (`rust/bins/dainty-lass-server-gui`), `dainty-lass-server-console` (`rust/services/host`, also built as `reforged-host`), and `dainty-lass-server-minimal` (`rust/bins/dainty-lass-server-minimal`, headless with no console or TUI). Settings read `host.toml`, `host.yaml`/`host.yml`, or `host.json`; writes are always `host.toml` (RD24) | `docs/app-structure-plan.md` and `docs/dainty-lass-plan.md` describe the merged design. The controller and module host have checklists (`docs/security/controller.md`, `docs/security/module-host.md`) and fuzz targets (`host_config`, `host_console`, `module_frame`) |

## 4. Work

### 4.1 Repository and CI (RR)

Target layout (amended by DD2, DS6, DF5):

```text
rust/                    one cargo workspace
  bins/                  dainty-lass-server-gui, dainty-lass-server-minimal
  crates/                protocol, proxy, service-kit, store, controller, module-api, epic, history-ship
  plugins/               network, chat
  services/              gateway, content, admin, host (dainty-lass-server-console / reforged-host), backend, director
  launcher/src-tauri     dainty-lass-client
modules/
  rustic-isles/          1.14.7 Rust crates (isles-*), runtime, preset, reference Node services
  rustic-rifts/          P2P transport (ParadoxTransport.dll), admission, TURN relay, realms, guard, host
ui/                      dainty-lass-ui (Astro + React + Tailwind v4 + shadcn/ui, built with Bun)
native/
  runtime-loader/        winmm.dll (generic DLL loader)
docker/  docs/  .forgejo/workflows/
```

| # | What | From DR | Verify |
| --- | --- | --- | --- |
| ~~RR1~~ | ~~Copy `rust/` (workspace files, `rust-toolchain.toml`, `deny.toml`, `.cargo/config.toml`, `clippy.toml`, `rustfmt.toml`, `crates/protocol`, `crates/proxy`, `crates/service-kit`, `crates/store`, `fuzz/` targets for those crates) in one commit (RD6)~~ | R4, X6 | `cargo test --workspace`, `cargo deny check`, `cargo clippy` pass on Windows and Linux |
| ~~RR2~~ | ~~`git mv` `ReforgedGateway`, `ReforgedContent`, `ReforgedLauncher`, `ReforgedHost` to `legacy/`~~ | R5 | Today's npm tests pass from `legacy/` |
| ~~RR3~~ | ~~CI by family behind one required check: Rust (fmt, clippy, test, deny, short fuzz) on Windows and Linux, the launcher page, the Paradox fork's Node tests, legacy Node~~ | R8 | A Rust-only push skips the Node jobs and the other way round |
| ~~RR4~~ | ~~Dependabot entries for `cargo` (`/rust`), the page and `paradox/*`, under the current policy; each npm entry under `legacy/` removed with its folder~~ | X6 | Dependabot opens a `cargo` PR |
| ~~RR5~~ | ~~Move the launcher page to `ui/` (`dainty-lass-ui` submodule, amended from `rust/launcher/ui` by DD11, DU1–DU4); `legacy/` and `upstream/` removed in the 2026-10-26 root reset~~ | R7, R10 | No reference to a deleted path |

### 4.2 The Paradox fork (RP)

Completed and moved to `modules/rustic-isles` (DS2, DS6).

| # | What | Verify |
| --- | --- | --- |
| ~~RP1~~ | ~~In the fork: branch `reforged` from `00d3be8`, patches 0001–0033 as 33 commits, in order; push it~~ | The tree equals what `npm run upstream` produces today (`.reforged-patches.json` set) |
| ~~RP2~~ | ~~Merge the fork's `reforged` branch as a subtree at `paradox/` (now the `modules/rustic-isles` submodule, DS2/DS6)~~ | `git log` in `modules/rustic-isles` shows Paradox's commits and the patch commits |
| ~~RP3~~ | ~~`git mv` the backend and Director, the runtime and loader; Paradox's top-level files (`ADDITIONAL_TERMS.md`, licence, `NOTICE.md`) stay in `modules/rustic-isles`~~ | The backend and Director tests pass; the runtime builds with a host's SDK |
| ~~RP4~~ | ~~Build from `modules/rustic-isles` instead of downloading; delete `upstream/` and `npm run upstream`; attribution in `NOTICE.md` and the READMEs~~ | `setup` on a clean host with no network access to Paradox's repository |

### 4.3 Gateway, allowlist and content (RS)

The file-by-file delta each row carries over is in `docs/port/delta-servers.md` (RX1).

| # | What | From DR | Delta | Verify |
| --- | --- | --- | --- | --- |
| ~~RS1~~ | ~~`protocol`: the `reforged-revival://` scheme only, `RRK_` keys, runtime manifest type; a `dauntless-revived://` invite parses to a typed refusal the page can word; the manifest serialises in the Node field order (`version`, `build { name, changelist }`, `files [{ name, size, sha256 }]`); `contentPort` stays in the status type~~ | X4 | Section 2 | DR's protocol tests plus Reforged's `invite.test.ts` cases as Rust tests; the manifest byte for byte against a body recorded from the Node content server |
| ~~RS2~~ | ~~Content server: copy `services/content`, drop the game zip, add the runtime manifest and the two-file download, auth against the backend's key check~~ | S1 | `runtime.ts`, `app.ts`, `auth.ts`, `config.ts` | Reforged's content tests ported; the Electron launcher downloads a runtime from the Rust server |
| ~~RS3~~ | ~~Gateway: copy `services/gateway`, apply `policy.ts` (Paradox and `/reforged/api` admin routes, host-only headers), config, feed and server deltas; `gateway make-cert` replaces `tools/make-cert.js`; DR's Node `capture.ts` ported in for RB2 (RD19); DR's feed backoff kept and `/game/login` feeding checked against the fork (RD20); **load the existing certificate, never regenerate**~~ | S2 | `policy.ts`, `config.ts`, `gateway.ts`, `feed.ts`, `server.ts`, `make-cert.js`, DR's `capture.ts` | Reforged's gateway tests ported; DR's parity test re-pointed at Reforged's Node gateway; an existing invite's `fp` still matches |
| ~~RS4~~ | ~~Allowlist helper: DR's `allowlist` bin with Reforged's game-port range (`ports.ts`) and its nftables variant (`nft.ts`) for the Linux VPS, as RD20 sets out (one atomic transaction, `lo` rule, `nft -f -` or a private file, `ALLOWLIST_*` names)~~ | S3 | `ports.ts`, `nft.ts`, `config.ts`, `firewall.ts` | Rule contents after login and logout on Windows and on Debian 13 (nft) |
| ~~RS5~~ | ~~Catch-up of Reforged's Node gateway and content commits since the copy (RX2); `legacy/ReforgedGateway` and `legacy/ReforgedContent` were deleted in the 2026-10-26 root reset; the front-door cutover is done in code (`rust_ported` returns `true` for `Gateway` and `Content`, and the Node front-service launch path is removed)~~ | X3 | — | Every commit on the list ported or marked not applicable |

### 4.4 Host kit (RH)

| # | What | Verify |
| --- | --- | --- |
| ~~RH1~~ | ~~`services/host` skeleton on `service-kit`: `paths`, `envfile`, `hostconfig` (`host.json`), `clap` commands with today's names and options~~ | Reforged's host kit tests for config and paths ported; a `host.json` written by the Node kit reads back unchanged |
| ~~RH2~~ | ~~`start`: the Rust services with `--env-file` (DR X7), the 1.14.7 Rust backend (`isles-1-14-7` on 61000) and Director (`isles-director` on 61001) by default (or Node behind `--features paradox` until RB15), the database when managed, the allowlist helper in public mode; one process tree, clean stop on Ctrl+C including the game servers; today's control contract kept (`REFORGED_CONTROL=stdin`, `REFORGED_SERVER_READY`, the port 61000 probe; RD23)~~ | Every service up; nothing left running after stop, on Windows and Linux |
| ~~RH3~~ | ~~`setup`, `invite`, `doctor`, `multipliers`; SQLite by default or a managed/`--postgres-url` Postgres (DD15), with the MongoDB replica-set setup (`mongo.ts`) retained behind `--features paradox` until RB15; `backup`~~ | A fresh private and a fresh public setup on Windows |
| ~~RH4~~ | ~~`gamedata`, `preset`, `progression`, `publish-runtime`, `gamepack`, `captures`; `gamedata` runs the in-process Rust generators in `isles-core` and `isles-director` (RB11/RU5i) with no Node.js dependency~~ | The generators give byte-identical files to the Node kit's from the same export |
| ~~RH5~~ | ~~`update` and setup's install step from the server release (RD8); `linux.ts`, `wine.ts` on the Linux release. `update` no longer fast-forwards a checkout and `npm run upstream` is gone (RP4); every other command keeps its name and options~~ | `docs/hosting-linux.md` followed on a clean Debian 13; an existing Node-kit host moves over with `update` |
| ~~RH6~~ | ~~Server release workflow (RD8, from DR M38)~~ | A run without publishing; its archives install on Windows and Debian 13 and pass the smoke list (section 6) |
| ~~RH7~~ | ~~Delete `legacy/ReforgedHost` (done in the 2026-10-26 root reset)~~ | — |

### 4.5 Backend and Director (RB → moved to RU and RF in `docs/dainty-lass-plan.md`)

Amended by `docs/dainty-lass-plan.md` sections 4.4 (`rustic-isles`, RU1–RU10) and 4.5 (`rustic-rifts`, RF1–RF8), with
no gate (DD6) and on `store::Database`'s two engines (SQLite and Postgres, DD15).

| # | What | Moved to | Verify |
| --- | --- | --- | --- |
| ~~RB1~~ | ~~Route inventory of both services (method, path, auth, body, reply), checked in and kept current by CI~~ | ~~`RU3a`, `RU3b`, `RU3c`, `RF1`~~ **Done** | Every Express route in the inventory; CI fails on a new route without an entry |
| RB2 | Traffic capture: the Rust gateway's capture option (RS3) records redacted request/reply pairs and chat sessions on the canary, played on by invited testers against the Node backend (**verified end-to-end with synthetic gateway capture in `docs/canary.md`; live tester canary corpus awaits maintainer**) | `RU8` | A corpus covering login, Ramsgate, a hunt, chat, store, loadouts, bounties, the Reforged API and the Director's game-server calls |
| ~~RB3~~ | ~~Two-engine schema and `store`: `store::Database` on SQLite and Postgres (`pq-sys` `bundled_without_openssl`), `isles-store` migrations for both engines designed from the 25 repositories (DD15), `schema.rs`~~ | ~~`DF7`, `RU4`~~ **Done** | `cargo test` of `store` and `isles-store` on both OSes |
| ~~RB4~~ | ~~`db migrate --from <mongodb-uri>` and `db verify --against <mongodb-uri>` in `isles-store::migrate`~~ | ~~`RU7`~~ **Done** | `isles-store` migration and verification tests pass; ready for real database copies |
| ~~RB5~~ | ~~Backend skeleton and seams (`isles-proto`, `isles-store`, `isles-game`, `isles-1-14-7`, `isles-1-12-0`), repository traits, deterministic `Clock` and `IdGen`~~ | ~~`RU4`~~ **Done** | A migrated database reads back field for field through the repositories |
| ~~RB6~~ | ~~Auth: API-key login (`AUTH_MODE=APIKEY`, key HMAC), argon2 hashes, JWTs, TOTP, account eligibility, launcher version checks, runtime authorization, the encryption key store~~ | ~~`RU5a`~~ **Done** | `test:auth-security` cases; tokens and keys issued by Node accepted by Rust and the other way round |
| ~~RB7~~ | ~~The Reforged launcher API (`/reforged/api`: ServerStatus, Register, invites, admin routes; patch 0002) and the login queue and maintenance status (0018)~~ | ~~`RU5b`~~ **Done** | `reforgedApiRules.test.ts`, `maintenanceStatus.test.ts`; the launcher registers against it |
| ~~RB8~~ | ~~Game areas: characters and loadouts (`RU5c`), inventory, storage policy and reforge (`RU5d`), progression, mastery, escalation and session supplies (`RU5e`), bounties, store, Lady Luck and platinum (`RU5f`), multipliers, matchmaking, parties and game-server posts (`RU5g`), plus realms, admission, relay and Guard (`RF4`–`RF7`)~~ | ~~`RU5c`–`RU5g`, `RF4`–`RF7`~~ **Done** | Each area's Node tests ported; its routes in the inventory answered |
| ~~RB9~~ | ~~Chat: XMPP over WebSocket in `plugins/chat` (`DN10a`, `DN10b`) with Paradox's JIDs, XMPP auth, rooms, City rooms (0025), presence, direct chat, quick chat and emojis (0017) in `isles-game::chat` (`RU5h`)~~ | ~~`DN10a`, `DN10b`, `RU5h`~~ **Done** | `test:realtime` cases; recorded chat sessions replay with the same stanzas |
| ~~RB10~~ | ~~Director (`isles-director`): game-server instances, bind host (0003), training on demand and idle stop (0021, 0033), hunt memory guard (0022), hunt tables; the backend's calls to it~~ | ~~`RU6`~~ **Done** | Its 5 test files ported; Ramsgate, the Training Grounds and a hunt start and stop |
| ~~RB11~~ | ~~The scripts, as Rust generators and commands (`isles-core::areas::game_data::generators`, `isles-core::areas::reforged_api::admin_discord`, `isles-director::hunt_tables`, `isles-db` data migrations), wired into `reforged-host gamedata` with no Node dependency~~ | ~~`RU5i`, `RU6`, `RU7`~~ **Done** | Each generator gives byte-identical output to its Node script from the same input (`crates/isles-core/tests/gamedata_generators_parity.rs`) |
| RB12 | Shadow replay: the RB2 corpus against Node on the MongoDB snapshot and Rust on `db migrate`'s copy of it, clock and ids pinned to the recorded values, normalizer for legitimate diffs (`reforged-host db replay`, `scripts/replay-capture.mjs`, `crates/isles-core/src/replay.rs`, and `crates/isles-db/src/replay.rs`; verified end-to-end with a 32-line live gateway capture across 25 HTTP routes and 1 XMPP WebSocket session replayed against a second server with `exact_matched=26, unexplained_diffed=0`, see `docs/canary.md`; **live tester canary corpus awaits maintainer**) | `RU8` | No unexplained diff in values or in the JSON text the client parses |
| ~~RB13~~ | ~~Rollback: `reforged-host db export-mongo --from <sqlite-or-postgres> --to <mongodb-uri>` writes the SQLite/Postgres data back as Paradox's MongoDB documents and round-trips cleanly through `reforged-host db verify --against <mongodb-uri>`~~ | ~~`RU7`, `RU9`~~ **Done** | Round-trip Mongo → SQL → Mongo test passes (`crates/isles-db/tests/migrate.rs`) |
| RB14 | Cutover: the canary first, a week of play, then each host through `update` — **Done in code; 7-day canary gate awaits maintainer (`docs/canary.md`)** (default 1.14.7 server runs entirely on Rust with SQLite/Postgres; `reforged-host setup` and `reforged-host update` run `reforged-host db migrate` + `reforged-host db verify` when legacy MongoDB data is present and refuse to switch on any difference while leaving MongoDB data untouched; rollback remains available for one release via `reforged-host db export-mongo` and `--features paradox` / Docker `--profile rollback`) | `RU9` | Smoke list (section 6) after each move; the RB2 corpus answers the same before and after |
| RB15 | One release after the last host's window: delete `modules/rustic-isles/reference/`, the Node parts of the release archive, `export-mongo`, and the host kit's MongoDB and Node handling | `RU9` | No `mongodb` in `cargo tree`; a fresh install from the docs alone needs only the release archive |

### 4.6 Launcher (RA)

Phases follow DR's Part A. Extended by `docs/dainty-lass-plan.md` sections 4.3 (`dainty-lass-ui`, DU1–DU5) and 4.8
(Installing from Epic, DE1–DE7).

| # | What | From DR | Verify |
| --- | --- | --- | --- |
| ~~RA1~~ | ~~Copy `src-tauri` without `offline/`, `controller/offline.rs`, `client_mods.rs`, `net/autoregion.rs` and the 1.4.4 manifest and DLL pins (RD5); the app's identifier, name and `userData` folder are Reforged's~~ | L4–L7 | It builds on Windows and Linux and opens today's page with a stub bridge |
| ~~RA2~~ | ~~Bridge, generated TS types and isolation hook re-generated for Reforged's command list (`ipc-validate.ts`, `preload.ts`; the list is in `docs/port/delta-launcher-controller.md`), minus the `revived/` commands; new argument kinds for an index, a fixed name list and objects, in `spec.rs`, the hook and its generator; `copy_text` a command (RD22); `controller/game.rs`, `controller/runtime.rs`, `controller/host.rs` registered as stubs so RA12 and RA13 never share `mod.rs` (extended by `DF8` for modules, Epic, network and realms)~~ | L6, L7, L9 | Every command the page uses is in the bridge spec; a refused command never reaches Rust |
| ~~RA3~~ | ~~Language files split (en, pt-BR) and the backend's text built from them~~ | L13–L15 | Reforged's `i18n.test.ts` passes on the split files |
| ~~RA4~~ | ~~Invite, status, username on `protocol` (RS1)~~ | L11, L12 | Shared test tables with the servers |
| ~~RA5~~ | ~~Pinned TLS, HTTP client, host API~~ | L19–L21 | Reforged's `pinned.test.ts` cases; register and status against the Rust gateway |
| ~~RA6~~ | ~~Relay on 61100 passing `/__origin/` paths through, chat WebSocket and local `/QoS`; DR's owner check on Windows (RD21)~~ | L22, L33 | Reforged's `relay.test.ts` cases |
| ~~RA7~~ | ~~`request_to_file` in `net/http.rs` and verify: the runtime's two files (accept once per set, check before every launch); `downloader.rs` restored in `DE5` for Epic game installs (DD22)~~ | L23–L25 | Reforged's `runtime.test.ts` cases |
| ~~RA8~~ | ~~Settings, log, folders; keystore with the safeStorage reader for Reforged's existing Windows keys (per server, bound to `fp`, `RRK2` and bare keys only); client profiles per game version (`RV2`, `DE4`); Reforged's username rule (RD22)~~ | L26–L29, LB1 | A real 1.5.1 profile's keys read back and work; a profile with a 1.4.4 server loads |
| ~~RA9~~ | ~~Game config (Engine.ini, Game.ini, display repair), game folder detection (Legendary, Heroic, Epic, folder), launch with `REFORGED_BACKEND_URL` and the client profile's command line; DR's Linux branches kept as they are, their Linux values left to RA10~~ | L30, L31 | Reforged's `game.test.ts`, `launch.test.ts`, `display-repair.test.ts` cases |
| RA10 | **Linux spike, then port**: 1.14.7 under Proton with the runtime loaded through `winmm.dll`, reaching the relay; then DR's Proton/Wine runtime fitted to it, game detection through Heroic and Legendary on Linux | L32 | Spike: Ramsgate loads and a hunt starts on Linux. Port: the same from the packaged launcher |
| ~~RA11~~ | ~~Links, dialogs, window, `reforged-revival://` scheme registration on both OSes~~ | L5, L34–L36 | Clicking an invite opens the running launcher |
| ~~RA12~~ | ~~Host page backend (Windows) for the planned layout only (RD23): workspace detection (`hostkit`), running `reforged-host` from the release and calling its `publish-runtime`, requirements, loading the host's chosen DLL through the runtime loader and the pre-launch check for leftovers (RD9)~~ | — | Reforged's `hostkit.test.ts`, `hostserver.test.ts` cases; the ini written and removed around a session, a leftover refused before PLAY; the loader loads CatalogExporter into a 1.14.7 client (maintainer) |
| RA12a | **Spike** for the Host page on Linux (RD4): load Dumper-7 and CatalogExporter into 1.14.7 running under Proton through the runtime loader (RD9) and generate an SDK and an export there. Kill criterion: if either the loading or the SDK generation does not work on Ubuntu 24.04 and SteamOS within the spike, the Host page stays Windows-only and RD4 is amended. On success: the port (the host kit started from the Linux launcher, requirements for Linux) | — | Spike: `gamedata` from the Linux export equals one made on Windows. Port: Reforged's host page tests on Linux |
| ~~RA13~~ | ~~Controller ported into `controller/mod.rs` and RA2's stub modules, every page command forwarded; missing Visual C++ files warn only, shutdown waits for downloads and refuses PLAY (RD22)~~ | L38, L39 | Reforged's `controller.test.ts` and `ui.test.ts` cases |
| RA14 | Package and sign updates: NSIS, AppImage, deb, rpm; release workflow; 2.0.0-beta | L45–L48 | Install, update and uninstall on a clean Windows VM; deb and rpm on Ubuntu 22.04/24.04, Debian 12, Fedora; the AppImage on SteamOS |
| RA15 | The Electron 1.6.0 bridge (RD7) | L50 | 1.5.1 → 1.6.0 → 2.0.0 on a clean VM keeps keys and settings |
| RA16 | 3.0.0: retire the Squirrel feed, delete the Electron key copies | L50a, L51 | — |

### 4.7 Cross-cutting (RX)

| # | What |
| --- | --- |
| ~~RX1~~ | ~~Per-file delta lists: for each Reforged Node file, `diff` against DR's Node file at `e1c5fec`, written as `docs/port/delta-<family>.md` (gateway, allowlist and content; launcher net and platform; launcher controller, bridge and page) and linked from the RS/RA row that ports it, so nothing Reforged changed is lost in the Rust copy. Read-only, so it runs first, one agent per file~~ |
| ~~RX2~~ | ~~Catch-up lists from the copy commit to the cut-off (RD16): written as `docs/port/catchup-<family>.md` (`catchup-servers.md`, `catchup-host.md`); closed when the upstream line was removed on 2026-10-26 (RD16)~~ |
| ~~RX3~~ | ~~Security pass (RD10), done per row rather than at the end: every row that writes a trust-boundary component delivers its fuzz target and a `docs/security/<component>.md` checklist with its code; `SECURITY_REVIEW.md` is the index, kept by the integration pass (extended by `DX2` across all five repositories). The 24-hour fuzz run and the signed checklist before each cutover stay with the maintainer~~ |
| ~~RX4~~ | ~~Container deployment (`compose.yaml`, `docker/`, `docs/hosting-docker.md`): the front door's Rust bins in a slim `front` image, the host kit (`reforged-host` with `backend` and `director` subcommands) and Wine in a Debian slim `server` image with no Node.js or MongoDB, both laid out as the server release (RD8) so the env files `setup` writes are valid in every container; one shared network namespace, so the services keep 127.0.0.1 and the allowlist's nftables table filters the game ports inside it with `CAP_NET_ADMIN` through `nft` only; an optional `rollback` Compose profile (`mongo`, `backend-rollback`, `director-rollback`, `server-rollback` stage) retained for one release until RB15; Tailscale as the namespace for a private server; CI builds the images and runs `docker/smoke-test.sh`~~ |

## 5. Order: parallel waves

See `docs/dainty-lass-plan.md` section 5 for the wave schedule (`DS`, `DF`, `DU`, `RU`, `RF`, `DN`, `DE`, `DH`,
`DX`, `RZ`, `DL`). Each repository (`dainty-lass`, `dainty-lass-ui`, `rustic-isles`, `rustic-rifts`) has its own integration pass merging its rows into its own `plan` branch, so the
review limit RD18 set (about eight rows in flight) applies per repository.

### 5.1 Rules for a row

- One row is one agent in its own worktree and branch. It pushes the branch and reports the commit; it opens no pull
  request, and the integration pass merges the branch into `plan`. CI runs on every pushed branch.
  An agent that finds it needs a change to a seam or to a file it does not own stops and reports; it does not make the
  change.
- Seams first. A skeleton row registers every planned module, command, route group or trait as a stub that compiles
  and is tested as a stub, so a later row only fills its own files: `DF2` (`crates/module-api`), `DF7` with `DH1`
  (`crates/store` and its change log), `DF8` (the launcher bridge spec and generated types), `RU4` (`rustic-isles`
  seams), and `RF2` (`rustic-rifts` seams).
- A row delivers its tests, its fuzz target and its `docs/security/<component>.md` with its code (RX3, DX2). Strings a
  launcher row needs are appended at the end of both language files; the integration pass resolves the resulting
  conflicts, which are append-only.
- The integration pass closes a wave: one agent, or the maintainer, merges each row's branch into `plan` with
  `--no-ff` in dependency order, runs CI's checks on the merged tree (`cargo test --workspace --no-fail-fast`, since
  a failing test binary otherwise hides the ones after it) and the smoke list (section 6) on the canary when the
  wave changed something that runs there, pushes `plan`, strikes the rows here and in `docs/dainty-lass-plan.md`,
  and updates the What's-left table. It is the only thing that edits the shared files below.
- Not an agent's work: the canary and every cutover, RA10 and RA12a's hands-on parts (a Linux machine with the game,
  Proton and a host's SDK), signing keys and publishing a release, the 24-hour fuzz runs and the signed checklists
  (RD10).

| File or folder | Owner | Other rows |
| --- | --- | --- |
| `rust/Cargo.toml`, `rust-toolchain.toml`, `deny.toml`, `.cargo/`, the lint configs | RR1 / Integration pass | Never; a crate is added only by its skeleton row or integration pass |
| `.forgejo/workflows/`, `.github/` | RR3, DX3; RH6 for `server-release.yml`; RA14 for `launcher-release.yml` | Never |
| `NOTICE.md`, `README.md`, `AGENTS.md`, `docs/` index pages, `SECURITY_REVIEW.md`, `MIGRATION_PLAN.md`, `docs/dainty-lass-plan.md` | Integration pass | Never; a row writes its own page under `docs/` |
| `crates/module-api` | DF2 (with DN2) | Never after DF2; a needed capability goes through the integration pass |
| `crates/protocol` | RS1 | Never after RS1; a needed type goes through the integration pass |
| `crates/store` (`Database`, change log, migrations, `portable`, `transfer`) | DF7 (with DH1), DH3a, DH5 | Never; a seam change goes through the integration pass |
| `services/host`: `main.rs`, the `clap` tree, `paths`, `envfile`, `hostconfig`, `hosttoml`, `serve` | RH1, DF3 | Fill the one command module the row owns |
| `launcher/src-tauri`: `main.rs`, the bridge spec, generated types, the isolation hook | RA1–RA2, DF8 | Fill the one module the row owns |
| `modules/rustic-isles`, `modules/rustic-rifts`, `ui/` | Each repository's own rows (`RU*`, `RF*`, `DU*`) and integration pass | Submodule pointers in `dainty-lass` updated by the integration pass |

### 5.2 Before the gate

| Wave | Row (one agent each unless said) | Needs | Hands-on |
| --- | --- | --- | --- |
| 0 | ~~RX1, three agents: gateway, allowlist and content; launcher net and platform; launcher controller, bridge and page~~ | — | — |
| 0 | ~~RP1 in the fork~~ | — | — |
| 0 | ~~RR1 copy of `rust/`~~ | — | — |
| 0 | ~~RR2 moves to `legacy/`~~ | — | — |
| 0 | RA10 spike: 1.14.7 under Proton with the runtime loaded | — | Linux machine with the game |
| 1 | ~~RP2–RP4~~ | RP1, RR2 | — |
| 1 | ~~RR3 CI, RR4 Dependabot (the `paradox/` job waits for RP3 to exist, it does not block)~~ | RR1, RR2 | — |
| 1 | ~~RS1 protocol~~ | RR1, RX1 | — |
| 1 | ~~RS4 allowlist helper~~ | RR1, RX1 | nft rules on Debian 13 |
| 1 | ~~RA1–RA2 launcher skeleton and bridge~~ | RR1, RX1 | — |
| 1 | ~~RH1 host kit skeleton~~ | RR1 | — |
| 2 | ~~RS2 content~~ | RS1, RX1 | — |
| 2 | ~~RS3 gateway~~ | RS1, RS4 (same crate), RX1 | — |
| 2 | ~~RH2 `start` (starts the Node services and MongoDB until the Rust ones exist)~~ | RH1 | — |
| 2 | ~~RH3 `setup`, `invite`, `doctor`, `multipliers`, MongoDB~~ | RH1 | — |
| 2 | ~~RH4 `gamedata`, `preset`, `progression`, `publish-runtime`, `gamepack`, `captures`~~ | RH1, RP3 | — |
| 2 | ~~RA3 languages~~ | RA1 | — |
| 2 | ~~RA4 + RA5 invite, status, TLS, HTTP, host API~~ | RA2, RS1 | — |
| 2 | ~~RA6 relay~~ | RA2 | — |
| 2 | ~~RA7 downloader and verify~~ | RA2, RS1 | — |
| 2 | ~~RA8 settings, log, keystore and legacy reader~~ | RA2 | A real 1.5.1 profile |
| 2 | ~~RA9 + RA11 game config, folders, launch, links, window~~ | RA2 | — |
| 3 | ~~RH6 release workflow, then RH5 `update` and install from it~~ | RS2–RS4, RH2–RH4 | — |
| 3 | ~~RA12 Host page backend and DLL loading through the runtime loader, Windows~~ | RA2, RH2 | A 1.14.7 client |
| 3 | RA10 port (if the spike passed) | RA9 | Linux machine |
| 3 | ~~RX2 lists for gateway, content and host kit~~ | RR1 (the copy commit) | — |
| 3 | Shared check: the Tauri debug build against the Rust gateway and content, with the Node backend | RS2, RS3, RA4–RA9 | Canary |
| 3 | Front-door cutover: canary, then every host through `update`; RX2's items for gateway, content and host kit ported first | RH5, RH6, RX2, the shared check | Canary, hosts |
| 3 | ~~RA13 controller~~ | RA3–RA12 | — |
| 3 | RA12a spike, then port if it passes | RA10 port, RA12 | Linux machine, SDK |
| 3 | RS5 and RH7, one agent: port the remaining catch-up items, delete the three legacy folders | The cutover | — |
| **Gate** | **The Rust front door and `reforged-host` are live on every host (RD17)** | RS5, RH7 | — |

Wave 2 is the widest, eleven rows; the servers and host kit rows and the launcher rows can be merged in two separate
integration passes, since they share no files.

### 5.3 After the gate

| Wave | Row (one agent each unless said) | Needs | Hands-on |
| --- | --- | --- | --- |
| 4 | RB1 route inventory, two agents: backend, Director | Gate | — |
| 4 | RB3 `pq-sys` spike, then the schema; the critical path of this half of the plan | Gate | — |
| 4 | RB10 Director (no database; `service-kit` only) with `director hunt-tables import` from RB11 | Gate | Game servers to start |
| 4 | RB11 game-data generators (pure file-to-file; `admin bootstrap` waits for RB5) | Gate | — |
| 4 | RB2 traffic capture on the canary; the cut-off date set and Freeza told (RD16) | Gate | Canary, testers |
| 4 | RA14 2.0.0-beta packages and release workflow | RA13 | Signing keys |
| 4 | RA15 Electron 1.6.0 bridge | RA14's installer hash | — |
| 5 | RB4 `db migrate` and `db verify` | RB3 | Copies of real databases |
| 5 | RB5 backend skeleton and seams | RB3 | — |
| 6 | RB6 auth | RB5, RB4 | — |
| 6 | RB7 Reforged API, login queue, maintenance status | RB5, RB4 | — |
| 6 | RB8 in five agents: characters and loadouts; inventory, storage policy and reforge; progression, mastery, escalation and session supplies; bounties, store, Lady Luck and platinum; multipliers, matchmaking and parties, the game servers' posts | RB5, RB4 | — |
| 6 | RB9 chat | RB5; RB6 for the XMPP auth, plugged in last | — |
| 6 | RB11 `admin bootstrap` and the data migrations folded into `db migrate`, by RB4's agent | RB4, RB5 | — |
| 6 | RB13 `db export-mongo` | RB4 | — |
| 6 | RB12 harness and normalizer, proven first on Node against Node (no diffs expected) | RB2, RB4 | — |
| 7 | RB12 replay; the diffs handed back one agent per RB8 group, RB6, RB7, RB9, RB10 | RB6–RB11, the harness | — |
| 7 | RX2 catch-up for the backend and Director, by the same agents, then the final replay | RB12 | — |
| 7 | RB13 rehearsal | RB13, the replay clean | Canary |
| 8 | RB14 cutover: canary, a week, then hosts | RB13 rehearsal, the signed checklists (RD10) | Canary, hosts |
| 8 | RA16 and RR5, one agent, when 3.0.0 is due | RA15 shipped | — |
| 9 | RB15 | The last host's rollback window closed | — |

Wave 6 is ten rows; RB13 and the RB12 harness can wait for a second integration pass. The Electron 1.5.x launcher
keeps working against the Rust front door, so the wave-3 cutover does not wait for 2.0.0, and the front door is live
long before the backend changes.

## 6. Things that must not change

- Launcher: the account keys already stored (read once, then re-stored), their binding to the certificate
  fingerprint, the `reforged-revival://` invite formats v1/v2, relay port 61100, the runtime acceptance per server
  and the check before every launch, the 1.14.7 executable hash, the page's look and both languages word for word;
  the key never crosses the bridge or reaches a log.
- Servers: the gateway's certificate and so every invite's `fp`; every route the game, the runtime and the launcher
  use, matched as Express matches them; JSON replies equal in value and in text (DR SB1); the XMPP stanzas the
  client reads; the host-only routes and headers stay refused; the content server's file layout; every player's
  data, unchanged on MongoDB and then carried over field for field to Postgres (RB4); API keys, password hashes
  and tokens issued by the Node backend keep working.
- Host kit: the `.env` files it writes, the settings a Node kit's `host.json` holds (read as they are, and moved
  to `host.toml` by the first `setup`, RD24), and every command's name and options except the two RH5 names
  (`update`'s meaning, `npm run upstream` removed), so an existing host moves over with `update`.
- Smoke list at every step: register with an invite, login, Ramsgate, a hunt, chat in the City room, runtime
  download, ban, clean stop.

## 7. Open questions

- RD16 is closed: the upstream line was removed on 2026-10-26, so no cut-off date is needed.
