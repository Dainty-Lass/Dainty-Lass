<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Dainty Lass migration plan: what is left

Rewritten 2026-11-05 against `plan` at `a5ac3d8f`: every completed row was removed and the decisions were brought up to
the current state. Since then: RR1 and RD24's `[env.<service>]` passthrough (`8f4dd537`), the MIT game runtime (R0–R21),
the MIT catalog exporter and the `settings.toml` rename (swept on 2026-11-13, below). The full plan as it stood, with
every struck row, is `MIGRATION_PLAN.md` at `a5ac3d8f`; the map from the upstream code to where it lives now is
[`docs/port/migration-guide.md`](docs/port/migration-guide.md), and the design record for the Dainty Lass rows (DS, DF,
DU, RU, RF, DN, DE, DH, DX, DL and the DD decisions) is [`docs/dainty-lass-plan.md`](docs/dainty-lass-plan.md).

Row ids: **RA** = launcher, **RS** = gateway, allowlist and content, **RH** = host kit, **RB** = backend and Director
(carried on as **RU** and **RF**), **RR** = repository and CI, **RX** = cross-cutting, **RD** = decisions, and the
Dainty Lass ids above. "DR" means Dauntless Revived at `e1c5fec`; DR row ids (L27, S2, X4...) point into
[its plan](https://github.com/mixutin/dauntless-revived/blob/plan/MIGRATION_PLAN.md), where the method and the checks
for each piece are written down.

## Where it stands

The port is done in code. No Node runs on a player's PC or on a host, and the Node and MongoDB rollback is gone (RB15):

- **One repository, one workspace (RD27).** The root is an MIT Cargo workspace (`crates/`, `fuzz/`), with the UI
  (`ui/`, dainty-dash) and the launcher's JavaScript (`launcher/`). `restricted/` is AGPL-3.0-only docs and data, with
  no code since RB15: Rustic Isles' version data and game data (`modules/rustic-isles/game-data/1.14.7`), the Rustic
  Rifts docs and the guides. The Node reference, restricted's Cargo workspace with its oracle and parity tests, and
  Docker's AGPL copies were deleted in RB15. The server executables and the launcher's Tauri shell are MIT at the root. dainty-restricted was imported into it on 2026-11-05 and its Forgejo repository deleted; the side
  repositories (dainty-lass-ui, rustic-isles, rustic-rifts, dainty-dashboard, dainty-legacy, legacy-revived,
  rustic-freeza) are archived.
- **Clean-room MIT rewrites (RD27).** The gateway and allowlist, content, chat, the host kit (`hostkit`), the
  launcher's Rust core (`launcher-kit`) and JavaScript, the Rustic Rifts crates (`rustic-rifts-*`) and all of Rustic Isles
  (`rustic-isles` and its `rustic-isles-*` areas, composed for 1.14.7 by `rustic-isles-1-14-7`) were rewritten from
  specs in `docs/port/` by implementers who never saw the AGPL code; the reviews are in `docs/port/clean-room/`.
  restricted's AGPL `isles-*` and `rustic-rifts-*` crates were retired. The recorded Rustic Isles goldens replayed 890 of
  890 against the MIT module before RB15 retired the replay (`restricted/modules/rustic-isles/docs/mit-parity-report.md`).
- **The server** is one executable (RD26): the root's `dainty-server` (`dainty-host`) and `dainty-server-minimal`
  compile every service and the game module in; the root's `dainty-lass` loads them as libraries
  (`crates/dainty-plugin-host`, `settings.toml`'s `[services.<name>]`). The game runs on `store` (SQLite by default, or
  Postgres).
- **One MIT game runtime (plan item 11).** `rustic-isles-runtime.dll` (`crates/rustic-isles-runtime`, clean-room from
  `docs/port/spec-runtime-*.md`) replaced the AGPL C++ `Rustic-Isles.dll` slice by slice, both loaded meanwhile: R0 and
  R1 `c63e171f`, R2 `5faab1a2`, R3 `fbe0c40e`, R4 `97d68572`, R5 `84d947c6`, R6 `2dd4b7f0`, R7 `0867ed29`, R8
  `a07bc8a8`, R9 `4493d5fb`, R10 `62e01091` (its launcher pipes `18736b35`), R11 `92011cc2`, R12 `fe339b69`, R13
  `f84a8bf9`, R14 `e119d68b`, R15 `ef80ae32` and `dc756273`, R16 `bed089c8`, R17 `41823d2b`, R18 `911cd8fc`, R20
  `54f1cecc` (R19, diagnostics, is never ported: RT-3). R21 (spec `912194b6`, MIT side `b5537996`, restricted side
  `50f6cbf4` and `0650bcb3`, Import SDK into `.workspace/sdk` `32ee33ba`) makes it the only runtime and deletes the C++
  source from `restricted/`, kept by the tag `agpl-runtime-final`. Realm starts moved off the tokio worker thread
  (`6847bbee`).
- **MIT catalog exporter.** `native/rustic-isles-exporter` (clean-room, `spec-catalog-exporter.md`) is what the host
  kit and the Host page build (`2f9c5f59`); the C++ one is deleted (`658c20a7`), kept by the tag
  `agpl-catalog-exporter-final`.
- **`settings.toml`.** `host.toml` was renamed `settings.toml` (`0f23e03d`, with the runtime's host settings, RT-11),
  with no migrator or fallback for a `host.toml`; a Node kit's `host.json` is still read and moved over.
- **Cut over in code, not in the field.** RU9/RB14 switched the default to Rust on 2026-10-28 without the canary
  week or a recorded shadow replay (maintainer decision, RD2). Nothing has been played against a real 1.14.7 client
  in CI.

## What's left

### For the maintainer (hands-on)

Not an agent's work (RD18): each needs a machine with the game, real hosts, signing keys or a person's judgement.

The first is **the one hands-on pass**: every runtime slice's play-test, run once on the MIT DLL alone, as the
checklist at <https://claude.ai/artifact/MytWdsz9QkyQ5VJU8NRYLu> collects them from
[`spec-runtime-r21.md`](docs/port/spec-runtime-r21.md) sections 8 and 9 (set-up, R21's eleven steps, then each
slice's own). Until it is done only CI has passed. No AGPL patch rebuild is left: that runtime is gone and the MIT one needs no
SDK (the SDK is still dumped, into `.workspace/sdk`, for the catalog exporter).

| # | What | Verify |
| --- | --- | --- |
| R21 | The one hands-on pass above, on a server tree from `plan` with R21 and the phase A launcher (R21 section 7.2), then the server release with R21 (phase B) | Every item of the checklist passes; anything that differs from the last pre-R21 release is reported against the kept pre-R21 set |
| RU8 (RB2, RB12) | Record a live tester corpus on the canary with the gateway's capture option (RD19) and replay it with `reforged-host db replay` / `scripts/replay-capture.mjs` (`restricted/docs/canary.md`). The method was verified end to end on a synthetic capture (26 exchanges, `unexplained_diffed=0`) | A corpus covering login, Ramsgate, a hunt, chat, store, loadouts, bounties, the launcher API and the Director's game-server calls; no unexplained diff in values or in the JSON text the client parses |
| RU9 (RB14) | The canary week (RD13): a server on the Linux kit with a copy of a real database and invited testers, then each host through `update`; a host still on MongoDB imports it first with `dainty-legacy import reforged` (`db migrate`, `db verify`) | The smoke list (section 3) after each move; the RU8 corpus answers the same before and after |
| — | The shared front-door check on the canary: the Tauri launcher against the Rust gateway and content | Register, status, runtime download |
| RS4 | The allowlist's rules on real hosts: nftables on Debian 13 (`DAINTY_TEST_FIREWALL=1`, or `docker/test-in-image.sh`) and Windows Firewall | Rule contents after login and logout |
| RH5, RH6 | Publish a server prerelease and follow `restricted/docs/hosting-linux.md` on a clean Debian 13; dry-run `server-release.yml` and `launcher-release.yml` | The archives install and pass the smoke list |
| RA10 | **Linux spike, then port**: 1.14.7 under Proton with the runtime loaded through `winmm.dll`, reaching the relay; then DR's Proton/Wine runtime (DR L32) fitted to it, with game detection through Heroic and Legendary | Spike: Ramsgate loads and a hunt starts on Linux. Port: the same from the packaged launcher |
| RA12a | **Spike** for the Host page on Linux (RD4): load Dumper-7 and CatalogExporter into 1.14.7 under Proton through the runtime loader and generate an SDK and an export. Kill criterion: if either fails on Ubuntu 24.04 and SteamOS, the Host page stays Windows-only and RD4 is amended | `gamedata` from the Linux export equals one made on Windows |
| RA14 | Package and sign: NSIS, AppImage, deb, rpm; the release workflow; 2.0.0-beta (DR L45–L48) | Install, update and uninstall on a clean Windows VM; deb and rpm on Ubuntu 22.04/24.04, Debian 12, Fedora; the AppImage on SteamOS |
| RA15 | The Electron 1.6.0 bridge (RD7, DR L50), after RA14's installer hash | 1.5.1 → 1.6.0 → 2.0.0 on a clean VM keeps keys and settings |
| RA16 | 3.0.0: retire the Squirrel feed, delete the Electron key copies (DR L50a, L51), once RA15 has shipped | — |
| RF8 | Two-PC realms check across NAT, direct and relay-only through turn-rs, and the trial that keeps turn-rs or switches the host kit to coturn (DD24). Verified on one machine across two isolated launcher profiles | A hosted hub and hunt joined from a second PC behind NAT |
| DN1 | **Spike** (a 1.14.7 client): the launcher's relay re-pointed to another gateway mid-session, in Ramsgate and between hunts; and a hunt on one server's game server while the player's backend is another's. The kill criterion is in `docs/dainty-lass-plan.md` | Both cases hold a session |
| DN14 | Canary network: four nodes in the four regions | The smoke list across regions, then a week of play |
| RD10 | The 24-hour fuzz run and the signed checklist (`SECURITY_REVIEW.md`) for each component before its cutover | Every checklist signed with the date and the commit reviewed |
| RR3 | Make `CI ok` the required check on `plan` and `main` (`docs/ci-runners.md`) | A red family blocks a merge |
| — | Merge `plan` into `main` | — |
| DL5 | A licensing lawyer reads `LICENSE-MIT.txt`, `LICENSE-APACHE.txt`, `restricted/NOTICE.md` and `restricted/ADDITIONAL_TERMS.md` and the RD27 layout before DX5 | The changes they ask for are made before the first public push |
| DX5 | Publish to GitHub once the launcher and the server are confirmed working: the repository pushed public, the Cargo registry switched or mirrored, the release workflows' tag trigger and publish job restored (`docs/ci-runners.md`) | — |

### For an agent (code)

| # | What | Needs |
| --- | --- | --- |
| RD27 | Rewrite the root workspace files that started from DR's (`Cargo.toml`'s layout, `deny.toml`, `clippy.toml`, `rustfmt.toml`, `rust-toolchain.toml`, `.cargo/config.toml`) clean-room, so nothing at the root descends from DR | — |
| RD24 | The rest of phase 2 of [`docs/config-toml-plan.md`](docs/config-toml-plan.md) (the typed `[gateway.*]`, `[content]`, `[allowlist]`, `[backend]` and `[director]` tables, early validation through each service's loader, environment overrides with a warning), then phase 3 (the other files hosts write). The `[env.<service>]` passthrough and its denylist are in (`hostconfig/env.rs`) | — |
| MS1–MS8 | [`docs/module-sharing-plan.md`](docs/module-sharing-plan.md): game calls in `module-api`, the event recorder, fleet game servers, the Discord bot and dainty-dash's views | The maintainer's answers to its D1–D6 |

### Parked

- **rustic-freeza** (DD25, DS8, RZ1–RZ4): the plugin that would carry Freeza's gameplay patches 0004–0039 and his
  launcher's look. Its repository was created and then archived with RD27. The runtime patches among them now live
  in the MIT runtime's slices (behind `[switches]` in `settings.toml` where they are settings); their record is
  `restricted/docs/paradox-patches.md`.

Local notes, not plan changes: on the maintainer's Windows PC two of Content's test ports fall in Windows' reserved
range, and cargo builds need `-j 6` or fewer (more exhausts the paging file and shows as "invalid metadata").

## 1. Decisions

Reference, not work: these stay when their rows are done. "Amended by DD..." points into
`docs/dainty-lass-plan.md` section 3.

| # | Topic | Decision | Consequence |
| --- | --- | --- | --- |
| RD1 | Mystic Paradox | Forked with its full history (`21b9ff5` on `main`, `replay` at `00d3be8` + the 39 patches) and kept in-tree at `restricted/modules/rustic-isles`: the 1.14.7 Node backend and Director (`reference/1.14.7`, deleted in RB15; its game data is in `game-data/1.14.7`), the runtime, the CatalogExporter and the version data. The generic runtime loader (`winmm.dll`) is the root's MIT `native/runtime-loader`, rewritten clean-room (DD13, RD27). Paradox's `ADDITIONAL_TERMS.md` stays with `modules/rustic-isles` and `modules/rustic-rifts`; attribution is in `restricted/NOTICE.md` | Hosts download nothing from Paradox's repository |
| RD2 | Backend and Director | **Ported to Rust**, now the MIT `rustic-isles-*` crates (RD27), on `store::Database` (SQLite and Postgres, DD15); there is no MongoDB implementation. **Maintainer decision (2026-10-28, departing from RB12 and RB14):** cut over in code without the canary week or a recorded shadow replay. The Node reference and `--features paradox` / `db export-mongo` were the rollback path until RB15 deleted them; a change now reaches the MIT crates only through their specs | No Node or MongoDB on hosts |
| RD3 | Host kit | **Rust**: `crates/dainty-hostkit` (MIT) inside restricted's `host` shell, bin `reforged-host`, with the Node kit's commands (`setup`, `start`, `invite`, `doctor`, `gamedata`, `preset`, `multipliers`, `publish-runtime`, `update`, `backup`) plus `install-game`, `network`, `history`, `db` and `serve`, and the files it wrote (`settings.toml` under RD24, the env files). It runs every service in one process (`serve`) or one process tree | One binary drives the whole server; the Host page runs it |
| RD4 | Launcher platforms | **Windows and Linux.** DR's Linux code came with it: Proton/Wine runtime, Secret Service and KWallet keystore, WebKitGTK 4.1 (DR D3), AppImage, deb and rpm. The Host page on Linux is a spike with a kill criterion (RA12a) | 1.14.7 and the runtime have to be shown to work under Proton first (RA10) |
| RD5 | DR's 1.4.4 servers | **Not played: a `dauntless-revived://` invite is a typed refusal** (`protocol::invite`). A 1.4.4 module existed from 2026-10-24 to 2026-10-31 and was removed (one game). A 1.4.4 server's accounts and bans import through `dainty-legacy` (`crates/dainty-legacy`) | 1.14.7 is the one game module |
| RD6 | DR's Rust code | **Snapshot copy** at `e1c5fec`, recorded in `restricted/NOTICE.md`; nothing flows either way afterwards. What reached the MIT root was since rewritten clean-room (RD27) | The code is Dainty Lass's own |
| RD7 | Launcher updates | DR's design (D2): `tauri-plugin-updater` with ed25519 signatures. Existing installs take a one-time bridge, Electron **1.6.0**, which downloads the Tauri **2.0.0** installer, checks its SHA-256 compiled into the bridge, runs it and removes the Squirrel install. The Squirrel feed is retired with 3.0.0; the Electron key copies are kept until then so a rollback works | One extra Electron release; 1.5.x installs that skip it before 3.0.0 reinstall by hand |
| RD8 | Server binaries | **Released by CI** (`.forgejo/workflows/server-release.yml`): `server-console` (`reforged-host`) and `server-minimal` with every module, and the service and module libraries, for `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`, one archive per OS with `SHA256SUMS.txt` and provenance attestations. `setup` and `update` download and check it. The archive carried the Node rollback until RB15 | No Rust toolchain on hosts; `update` is not a `git` fast-forward |
| RD9 | Loading the runtime, Dumper-7 and CatalogExporter | **Amended 2026-11-12 and 13 (maintainer): the launcher and the Director inject.** Each starts the game suspended and loads only DLLs it chose, by absolute path with checked hashes, killing it on any failure (`crates/dainty-inject`; [docs/launcher-injection.md](docs/launcher-injection.md), [docs/director-injection.md](docs/director-injection.md)). Under Wine and Proton the `dainty-inject.exe` helper does it; Proton players can keep the `winmm.dll` loader and its `mystic_loader.ini` (a Settings switch, loader by default until the hands-on Proton check). The loader stands down for an injected game (`DAINTY_RUNTIME_INJECTED`), and PLAY still refuses a leftover ini, Host tools DLL, a retired `Rustic-Isles.dll` it can't remove (R21) or a loader that would load the runtime twice. Replaces the first decision, no injector at all | One less native component on a player's PC and none beside a host's servers; the injection and the helper have their own checklists |
| RD10 | Security | **DR's full pass** (D26, X9): a checklist per trust-boundary component (`SECURITY_REVIEW.md`, `restricted/SECURITY_REVIEW.md`), `cargo-fuzz` targets for every parser of untrusted input run briefly on every push, a 24-hour fuzz run and a signed checklist before each component's cutover | Replaces a second reviewer where a mistake costs most |
| RD11 | Repository layout | **Amended by RD27:** one repository with the MIT workspace at the root and the AGPL workspace in `restricted/`, every package in a flat `crates/<package name>` in each | One history, one CI, a licence boundary CI checks |
| RD12 | Order | Parallel waves, synced on seam crates (`module-api`, `store`, the launcher bridge) (DD6, RD18) | Each cutover still waits for its own checks |
| RD13 | Test environment | **A canary run by the maintainer**, like DR's D23: a small server on the Linux kit with a copy of a real database and invited testers. The shared checks, the traffic capture, the shadow replay and every first cutover run there; then the hosts | The Linux path is tested first |
| RD14 | Languages | en and pt-BR, split one file per language as DR did (L13), so a third language is data only | — |
| RD15 | Database | **SQLite by default, Postgres where concurrency pays** (DD15), through `store::Database` (SQLite through Diesel, Postgres through `diesel-async`, no libpq) with the hash-chained change log on every write (DD23). `store::transfer::move_database` moves a database between the two engines in one transaction. An old install's MongoDB data moves with `dainty-legacy import reforged` (`db migrate` / `db verify`, `rustic-isles-db`) | Small servers need no database server; networked servers can run Postgres with no module change |
| RD16 | Upstream | **Closed 2026-10-26:** the Reforged Revival line was removed and nothing is merged from it. Its gameplay patches stay in rustic-isles (Parked: rustic-freeza) | The catch-up lists are closed |
| RD17 | Gate before the backend | **Dropped (DD6):** the port ran in parallel with the framework; only the field cutover (RU9) waits on the replay and the canary | — |
| RD18 | How rows are done | **One row, one agent, its own worktree and branch.** It pushes the branch and reports the commit; it opens no pull request. The integration pass merges branches into `plan` with `--no-ff` in dependency order, runs the checks and updates this file (section 2) | Canary runs, cutovers, spikes that need the game, signing and publishing stay with the maintainer |
| RD19 | Traffic capture | **In the Rust gateway** (`capture.rs`), from DR's Node `capture.ts`; off unless configured, redacting as `capture.ts` did | RU8 records on the Rust gateway |
| RD20 | Allowlist helper and gateway details | **Reforged's behaviour where the two differ, DR's where Reforged had nothing.** nft: one atomic transaction swapping the whole table, with Reforged's `lo` rule; `nft -f -` tried first, a `0600` file in a `0700` folder if Debian 13 refuses stdin; `nft` at its configured path, no PATH lookup. Env: only the `ALLOWLIST_*` names the host kit writes, not DR's `DR_*`; `ALLOWLIST_POWERSHELL` accepted and ignored (Windows Firewall goes through COM). DR's feed backoff from a helper that is down kept. Only `POST /heartbeat` feeds the allowlist. The certificate subcommand is `gateway make-cert` | An existing host's env file starts the helper unchanged |
| RD21 | Relay owner check | **DR's kept on Windows** (only connections whose socket belongs to the game process the launcher started are served). On Linux, RA10 decides whether the game process can be found under Proton; if not, the relay serves loopback only. The Dumper-7 session's game does not use the relay | A trust-boundary control at no cost on Windows |
| RD22 | Launcher behaviours that differ from DR | **1.4.4 servers** are not supported. **DR's `net/downloader.rs`** came back to download and resume game versions from Epic's CDN (`crates/dainty-epic`, DE5), while the runtime's two files use `request_to_file`. **Stored usernames** update when a login reply carries a new name after a network join clash (DN13). **Missing Visual C++ files** only warn before PLAY; **shutdown** waits for downloads and refuses PLAY; **keystore** reads `RRK2` and bare keys; **copying text** is a bridge command with its 4096-character limit | The 1.14.7 launcher with Epic install and resume |
| RD23 | Host page target | **The release layout only**: the Host page drives `reforged-host` from the server release, never `npm` or `node`; `publish-runtime` is the host kit's command, so the version format and `runtime.json` exist once. `reforged-host` keeps the process contract: `REFORGED_CONTROL=stdin`, the `REFORGED_SERVER_READY` line and the port 61000 probe | — |
| RD24 | Host configuration format | **TOML for the files hosts write.** `.workspace/config/settings.toml` (named `host.toml` until 2026-11-14, renamed with no migrator), snake_case keys, a left-out key in place of `null` (`settingstoml.rs`); writes patch the file with `toml_edit`, so a host's comments survive `setup`. A Node kit's `host.json` (or `host.yaml`) is read while there is no `settings.toml`, and the first write moves it over and keeps it as `.bak`. Tables: `[modules.<id>]`, `[services.<name>]`, `[env.<service>]`, `[switches]`, `[network]`, `[history]`, `[relay]`, `[remote]`. `[env.<service>]` appends a variable to one service's env file and refuses secrets, ports, binds, safety keys and anything `setup` already writes; the rest of phases 2 and 3 is open (What's left) | Every setting is a key in `settingstoml.rs`'s table with a comment for new files (AGENTS.md) |
| RD25 | Admin API | **Per-person admin keys with roles** (`owner`, `admin`, `moderator`, `reader`), history revert, guarded actions and network-wide moderation (`crates/dainty-admin`, `docs/admin-api.md`; DD23, DH2, DH3b, DH6, DN11). `REFORGED_ADMIN_API_KEY` in `secrets.env` is the `owner` key; other keys are issued and revoked by command and recorded as the `Actor` in `store`'s change log. Destructive and bulk actions wait on a delay or a second operator; in a network, bans, timeouts and kicks propagate through the player's home node | Every admin change names its operator key and can be audited and reverted |
| RD26 | Server structure | **One executable per role on `crates/dainty-module-api`** (DD5, DF2–DF5). `crates/dainty-controller` hosts the game module and the core plugins (`gateway` with the allowlist, `content`, `admin`, `chat`, `network`, `remote`) in-process, as child processes over the frame protocol, or as libraries (RD27). The executables: `dainty-lass-client` and `dainty-lass-manager` (restricted's Tauri shells), `dainty-lass-server-console` (also `reforged-host`) and `dainty-lass-server-minimal` (restricted), and the MIT core `dainty-lass`. Each service is still runnable alone as a hidden `reforged-host <service>` subcommand | `docs/architecture.md` describes it; the controller and module host have checklists and fuzz targets |
| RD27 | Licence boundary | **MIT at the root, AGPL-3.0-only in `restricted/`** (maintainer decisions 2026-11-02 and 2026-11-05). A root package must be MIT or Apache-2.0 and must not reference `restricted/`; CI checks both. Code ported from DR, Paradox or the Reforged Revival line, or linking it, goes in `restricted/`; anything at the root that came from them was rewritten clean-room: a spec agent reads the AGPL code and writes `docs/port/spec-*.md` with no code in it, a separate implementer sees only the spec and the MIT code, and a similarity review is recorded in `docs/port/clean-room/`. Services and modules can be libraries the MIT core loads over a C ABI carrying the DD5 frame protocol (`crates/dainty-plugin-host`, the one MIT crate allowed `unsafe`; `controller::service_library`; `settings.toml`'s `[services.<name>]` with `library` and `sha256`). Phase 1 (`crates/dainty-host` composing the services in-process, `compose.rs`) and phase 2 (the libraries) are both in place. Since RB15 `restricted/` holds no code, only AGPL docs and data | The MIT core and crates ship on their own; the release adds `restricted/`'s data |

## 2. Rules for a row

[`AGENTS.md`](AGENTS.md) has the full rules ("How work lands"). The shared files and who may change them:

| File or folder | Owner | Other rows |
| --- | --- | --- |
| `Cargo.toml`, `rust-toolchain.toml`, `deny.toml`, `.cargo/`, the lint configs | Integration pass | Never; a crate is added only by its skeleton row or the integration pass |
| `.forgejo/workflows/`, `.github/` | RR3, DX3; RH6 for `server-release.yml`; RA14 for `launcher-release.yml` | Never |
| `README.md`, `AGENTS.md`, `restricted/AGENTS.md`, `docs/` index pages, `SECURITY_REVIEW.md`, `restricted/SECURITY_REVIEW.md`, `MIGRATION_PLAN.md`, `docs/dainty-lass-plan.md`, `LICENSE-MIT.txt`, `LICENSE-APACHE.txt`, `restricted/NOTICE.md` | Integration pass | Never; a row writes its own page under `docs/` |
| `crates/dainty-module-api` | DF2 | A needed capability goes through the integration pass |
| `crates/dainty-protocol` | RS1 | A needed type goes through the integration pass |
| `crates/dainty-store` (`Database`, change log, migrations, `portable`, `transfer`) | DF7, DH1, DH3a, DH5 | A seam change goes through the integration pass |
| `crates/dainty-hostkit` (`cli.rs` and the `clap` tree, `paths`, `envfile`, `hostconfig`, `settingstoml`, `serve`) and `crates/dainty-host` (`compose.rs`) | RH1, DF3 | Fill the one command module the row owns |
| `crates/dainty-launcher-kit` (`bridge/spec.rs`, the generated types) and `launcher/isolation` | RA1–RA2, DF8 | Fill the one module the row owns |
| `crates/rustic-isles*` | Their specs (`docs/port/spec-rustic-isles-*.md`) | A behaviour change goes into the spec first (RD27) |

## 3. Things that must not change

- Launcher: the account keys already stored (read once, then re-stored), their binding to the certificate
  fingerprint, the `reforged-revival://` invite formats v1/v2, relay port 61100, the runtime acceptance per server
  and the check before every launch, the 1.14.7 executable hash, the page's look and both languages word for word;
  the key never crosses the bridge or reaches a log.
- Servers: the gateway's certificate and so every invite's `fp`; every route the game, the runtime and the launcher
  use, matched as Express matches them; JSON replies equal in value and in text (DR SB1); the XMPP stanzas the
  client reads; the host-only routes and headers stay refused; the content server's file layout; every player's
  data, carried over field for field from an old install's MongoDB to SQLite or Postgres (`dainty-legacy import reforged`); API keys,
  password hashes and tokens issued by the Node backend keep working.
- Host kit: the env files it writes, the settings a Node kit's `host.json` holds (read as they are, and moved
  to `settings.toml` by the first `setup`, RD24), and every command's name and options, so an existing host moves over
  with `update`.
- Smoke list at every step: register with an invite, login, Ramsgate, a hunt, chat in the City room, runtime
  download, ban, clean stop.
