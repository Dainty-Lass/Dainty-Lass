<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# DaintyLass migration plan: what is left

Rewritten 2026-11-05 against `plan` at `a5ac3d8f`: every completed row was removed and the decisions were brought up
to the current state. The full plan as it stood, with every struck row, is `MIGRATION_PLAN.md` at `a5ac3d8f`; the
map from the upstream code to where it lives now is [`docs/port/migration-guide.md`](docs/port/migration-guide.md),
and the design record for the DaintyLass rows (DS, DF, DU, RU, RF, DN, DE, DH, DX, DL and the DD decisions) is
[`docs/dainty-lass-plan.md`](docs/dainty-lass-plan.md).

Row ids: **RA** = launcher, **RS** = gateway, allowlist and content, **RH** = host kit, **RB** = backend and Director
(carried on as **RU** and **RF**), **RR** = repository and CI, **RX** = cross-cutting, **RD** = decisions, and the
DaintyLass ids above. "DR" means Dauntless Revived at `e1c5fec`; DR row ids (L27, S2, X4...) point into
[its plan](https://github.com/mixutin/dauntless-revived/blob/plan/MIGRATION_PLAN.md), where the method and the checks
for each piece are written down.

## Where it stands

The port is done in code. No Node runs on a player's PC or on a host on the default path:

- **One repository, two workspaces (RD27).** The root is an MIT Cargo workspace (`crates/`, `fuzz/`), with the UI
  (`ui/`, dainty-dash) and the launcher's JavaScript (`launcher/`). `restricted/` is a separate AGPL-3.0-only workspace
  that takes the root's crates by path: the host shell, the Tauri launcher, the server executables, the game runtime
  and its Node reference, the transport DLL, the 1.4.4 importer, the runtime loader and Docker. dainty-restricted was
  imported into it on 2026-11-05 and its Forgejo repository deleted; the side repositories (dainty-lass-ui,
  rustic-isles, rustic-rifts, dainty-dashboard, dainty-legacy, legacy-revived, rustic-freeza) are archived.
- **Clean-room MIT rewrites (RD27).** The gateway and allowlist, content, chat, the host kit (`hostkit`), the
  launcher's Rust core (`launcher-kit`) and JavaScript, the Rustic Rifts crates (`rifts-*`) and all of Rustic Isles
  (`rustic-isles` and its `rustic-isles-*` areas, composed for 1.14.7 by `rustic-isles-1-14-7`) were rewritten from
  specs in `docs/port/` by implementers who never saw the AGPL code; the reviews are in `docs/port/clean-room/`.
  restricted's AGPL `isles-*` and `rifts-*` crates were retired. The recorded Rustic Isles goldens replay 890 of 890
  against the MIT module (`restricted/modules/rustic-isles/docs/mit-parity-report.md`).
- **The server** is one executable (RD26): restricted's `server-console` (`reforged-host`) and `server-minimal`
  compile every service and the game module in; the root's `dainty-lass-server` loads them as libraries
  (`crates/plugin-host`, `host.toml`'s `[services.<name>]`). The game runs on `store` (SQLite by default, or Postgres).
- **Cut over in code, not in the field.** RU9/RB14 switched the default to Rust on 2026-10-28 without the canary
  week or a recorded shadow replay (maintainer decision, RD2). Nothing has been played against a real 1.14.7 client
  in CI.

## What's left

### For the maintainer (hands-on)

Not an agent's work (RD18): each needs a machine with the game, real hosts, signing keys or a person's judgement.

| # | What | Verify |
| --- | --- | --- |
| RU8 (RB2, RB12) | Record a live tester corpus on the canary with the gateway's capture option (RD19) and replay it with `reforged-host db replay` / `scripts/replay-capture.mjs` (`restricted/docs/canary.md`). The method was verified end to end on a synthetic capture (26 exchanges, `unexplained_diffed=0`) | A corpus covering login, Ramsgate, a hunt, chat, store, loadouts, bounties, the launcher API and the Director's game-server calls; no unexplained diff in values or in the JSON text the client parses |
| RU9 (RB14) | The canary week (RD13): a server on the Linux kit with a copy of a real database and invited testers, then each host through `update` (`setup` and `update` migrate and verify MongoDB data and refuse to switch on a difference) | The smoke list (section 3) after each move; the RU8 corpus answers the same before and after |
| — | Play through `serve` against a real 1.14.7 client, including CatalogExporter loaded through the runtime loader (RA12's last check, RD9) | Ramsgate, a hunt and a game-data export from the Host page |
| — | The shared front-door check on the canary: the Tauri launcher against the Rust gateway and content | Register, status, runtime download |
| RS4 | The allowlist's rules on real hosts: nftables on Debian 13 (`REFORGED_TEST_NFT=1`, or `restricted/docker/test-in-image.sh`) and Windows Firewall | Rule contents after login and logout |
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
| DL5 | A licensing lawyer reads `LICENSE.txt`, `restricted/NOTICE.md` and `restricted/ADDITIONAL_TERMS.md` and the RD27 layout before DX5 | The changes they ask for are made before the first public push |
| DX5 | Publish to GitHub once the launcher and the server are confirmed working: the repository pushed public, the Cargo registry switched or mirrored, the release workflows' tag trigger and publish job restored (`docs/ci-runners.md`) | — |

### For an agent (code)

| # | What | Needs |
| --- | --- | --- |
| RB15 | One release after the last host's rollback window: delete `restricted/modules/rustic-isles/reference/`, the `paradox` feature and the Node parts of the release archive, `export-mongo`, the host kit's MongoDB and Node handling, and Docker's `rollback` profile | RU9 done on every host |
| RR1 | Remove the `crates/backend` and `crates/director` stubs RR1 registered; the `rustic-isles` crates replaced them | — |
| RD27 | Rewrite the root workspace files that started from DR's (`Cargo.toml`'s layout, `deny.toml`, `clippy.toml`, `rustfmt.toml`, `rust-toolchain.toml`, `.cargo/config.toml`) clean-room, so nothing at the root descends from DR | — |
| RD24 | Phases 2 and 3 of [`docs/config-toml-plan.md`](docs/config-toml-plan.md): service tunables in `host.toml` with the `[env.<service>]` passthrough and early validation, then the other files hosts write | — |
| MS1–MS8 | [`docs/module-sharing-plan.md`](docs/module-sharing-plan.md): game calls in `module-api`, the event recorder, fleet game servers, the Discord bot and dainty-dash's views | The maintainer's answers to its D1–D6 |

### Parked

- **rustic-freeza** (DD25, DS8, RZ1–RZ4): the plugin that would carry Freeza's gameplay patches 0004–0039 and his
  launcher's look. Its repository was created and then archived with RD27. The patches stay where they are
  (`restricted/modules/rustic-isles/native/runtime`, behind `[switches]` in `host.toml` where they are settings;
  `restricted/docs/paradox-patches.md`).

Local notes, not plan changes: on the maintainer's Windows PC two of Content's test ports fall in Windows' reserved
range, and `restricted/` builds need `-j 6` or fewer (more exhausts the paging file and shows as "invalid metadata").

## 1. Decisions

Reference, not work: these stay when their rows are done. "Amended by DD..." points into
`docs/dainty-lass-plan.md` section 3.

| # | Topic | Decision | Consequence |
| --- | --- | --- | --- |
| RD1 | Mystic Paradox | Forked with its full history (`21b9ff5` on `main`, `replay` at `00d3be8` + the 39 patches) and kept in-tree at `restricted/modules/rustic-isles`: the 1.14.7 Node backend and Director (`reference/1.14.7`), the runtime, the CatalogExporter and the version data. The generic runtime loader (`winmm.dll`) is `restricted/native/runtime-loader` (DD13). Paradox's `ADDITIONAL_TERMS.md` stays with `modules/rustic-isles` and `modules/rustic-rifts`; attribution is in `restricted/NOTICE.md` | Hosts download nothing from Paradox's repository |
| RD2 | Backend and Director | **Ported to Rust**, now the MIT `rustic-isles-*` crates (RD27), on `store::Database` (SQLite and Postgres, DD15); there is no MongoDB implementation. **Maintainer decision (2026-10-28, departing from RB12 and RB14):** cut over in code without the canary week or a recorded shadow replay. The Node reference and `--features paradox` / `reforged-host db export-mongo` stay for one release as the rollback path until RB15. `reference/1.14.7` still takes fixes, which reach the MIT crates only through their specs | No Node or MongoDB on hosts on the default path |
| RD3 | Host kit | **Rust**: `crates/hostkit` (MIT) inside restricted's `host` shell, bin `reforged-host`, with the Node kit's commands (`setup`, `start`, `invite`, `doctor`, `gamedata`, `preset`, `multipliers`, `publish-runtime`, `update`, `backup`) plus `install-game`, `network`, `history`, `db` and `serve`, and the files it wrote (`host.toml` under RD24, the env files). It runs every service in one process (`serve`) or one process tree | One binary drives the whole server; the Host page runs it |
| RD4 | Launcher platforms | **Windows and Linux.** DR's Linux code came with it: Proton/Wine runtime, Secret Service and KWallet keystore, WebKitGTK 4.1 (DR D3), AppImage, deb and rpm. The Host page on Linux is a spike with a kill criterion (RA12a) | 1.14.7 and the runtime have to be shown to work under Proton first (RA10) |
| RD5 | DR's 1.4.4 servers | **Not played: a `dauntless-revived://` invite is a typed refusal** (`protocol::invite`). A 1.4.4 module existed from 2026-10-24 to 2026-10-31 and was removed (one game). A 1.4.4 server's accounts and bans import through the importer in `restricted/legacy/` | 1.14.7 is the one game module |
| RD6 | DR's Rust code | **Snapshot copy** at `e1c5fec`, recorded in `restricted/NOTICE.md`; nothing flows either way afterwards. What reached the MIT root was since rewritten clean-room (RD27) | The code is DaintyLass's own |
| RD7 | Launcher updates | DR's design (D2): `tauri-plugin-updater` with ed25519 signatures. Existing installs take a one-time bridge, Electron **1.6.0**, which downloads the Tauri **2.0.0** installer, checks its SHA-256 compiled into the bridge, runs it and removes the Squirrel install. The Squirrel feed is retired with 3.0.0; the Electron key copies are kept until then so a rollback works | One extra Electron release; 1.5.x installs that skip it before 3.0.0 reinstall by hand |
| RD8 | Server binaries | **Released by CI** (`.forgejo/workflows/server-release.yml`): `server-console` (`reforged-host`) and `server-minimal` with every module, and the service and module libraries, for `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`, one archive per OS with `SHA256SUMS.txt` and provenance attestations. `setup` and `update` download and check it. Until RB15 the archive also carries the Node rollback | No Rust toolchain on hosts; `update` is not a `git` fast-forward |
| RD9 | Loading Dumper-7 and CatalogExporter | **No injector.** The runtime loader (`winmm.dll`) loads the DLL the host picked through the `mystic_loader.ini` list it already reads: the launcher copies the DLL next to the loader, writes the ini naming only it, starts the game, and removes both when the game exits. Before every ordinary PLAY the launcher refuses to start while a `mystic_loader.ini` or an unknown DLL sits next to the loader | No process-injection code in the launcher and no change to the loader; under Proton it works the same way. The ini handling has its own checklist |
| RD10 | Security | **DR's full pass** (D26, X9): a checklist per trust-boundary component (`SECURITY_REVIEW.md`, `restricted/SECURITY_REVIEW.md`), `cargo-fuzz` targets for every parser of untrusted input run briefly on every push, a 24-hour fuzz run and a signed checklist before each component's cutover | Replaces a second reviewer where a mistake costs most |
| RD11 | Repository layout | **Amended by RD27:** one repository with the MIT workspace at the root and the AGPL workspace in `restricted/`, every package in a flat `crates/<package name>` in each | One history, one CI, a licence boundary CI checks |
| RD12 | Order | Parallel waves, synced on seam crates (`module-api`, `store`, the launcher bridge) (DD6, RD18) | Each cutover still waits for its own checks |
| RD13 | Test environment | **A canary run by the maintainer**, like DR's D23: a small server on the Linux kit with a copy of a real database and invited testers. The shared checks, the traffic capture, the shadow replay and every first cutover run there; then the hosts | The Linux path is tested first |
| RD14 | Languages | en and pt-BR, split one file per language as DR did (L13), so a third language is data only | — |
| RD15 | Database | **SQLite by default, Postgres where concurrency pays** (DD15), through `store::Database` (SQLite through Diesel, Postgres through `diesel-async`, no libpq) with the hash-chained change log on every write (DD23). `store::transfer::move_database` moves a database between the two engines in one transaction. A host's MongoDB data moves with `reforged-host db migrate` / `db verify` (`rustic-isles-db`) | Small servers need no database server; networked servers can run Postgres with no module change |
| RD16 | Upstream | **Closed 2026-10-26:** the Reforged Revival line was removed and nothing is merged from it. Its gameplay patches stay in rustic-isles (Parked: rustic-freeza) | The catch-up lists are closed |
| RD17 | Gate before the backend | **Dropped (DD6):** the port ran in parallel with the framework; only the field cutover (RU9) waits on the replay and the canary | — |
| RD18 | How rows are done | **One row, one agent, its own worktree and branch.** It pushes the branch and reports the commit; it opens no pull request. The integration pass merges branches into `plan` with `--no-ff` in dependency order, runs the checks and updates this file (section 2) | Canary runs, cutovers, spikes that need the game, signing and publishing stay with the maintainer |
| RD19 | Traffic capture | **In the Rust gateway** (`capture.rs`), from DR's Node `capture.ts`; off unless configured, redacting as `capture.ts` did | RU8 records on the Rust gateway |
| RD20 | Allowlist helper and gateway details | **Reforged's behaviour where the two differ, DR's where Reforged had nothing.** nft: one atomic transaction swapping the whole table, with Reforged's `lo` rule; `nft -f -` tried first, a `0600` file in a `0700` folder if Debian 13 refuses stdin; `nft` at its configured path, no PATH lookup. Env: only the `ALLOWLIST_*` names the host kit writes, not DR's `DR_*`; `ALLOWLIST_POWERSHELL` accepted and ignored (Windows Firewall goes through COM). DR's feed backoff from a helper that is down kept. Only `POST /heartbeat` feeds the allowlist. The certificate subcommand is `gateway make-cert` | An existing host's env file starts the helper unchanged |
| RD21 | Relay owner check | **DR's kept on Windows** (only connections whose socket belongs to the game process the launcher started are served). On Linux, RA10 decides whether the game process can be found under Proton; if not, the relay serves loopback only. The Dumper-7 session's game does not use the relay | A trust-boundary control at no cost on Windows |
| RD22 | Launcher behaviours that differ from DR | **1.4.4 servers** are not supported. **DR's `net/downloader.rs`** came back to download and resume game versions from Epic's CDN (`crates/epic`, DE5), while the runtime's two files use `request_to_file`. **Stored usernames** update when a login reply carries a new name after a network join clash (DN13). **Missing Visual C++ files** only warn before PLAY; **shutdown** waits for downloads and refuses PLAY; **keystore** reads `RRK2` and bare keys; **copying text** is a bridge command with its 4096-character limit | The 1.14.7 launcher with Epic install and resume |
| RD23 | Host page target | **The release layout only**: the Host page drives `reforged-host` from the server release, never `npm` or `node`; `publish-runtime` is the host kit's command, so the version format and `runtime.json` exist once. `reforged-host` keeps the process contract: `REFORGED_CONTROL=stdin`, the `REFORGED_SERVER_READY` line and the port 61000 probe | — |
| RD24 | Host configuration format | **TOML for the files hosts write.** `.workspace/config/host.toml`, snake_case keys, a left-out key in place of `null` (`hosttoml.rs`); writes patch the file with `toml_edit`, so a host's comments survive `setup`. A Node kit's `host.json` (or `host.yaml`) is read while there is no `host.toml`, and the first write moves it over and keeps it as `.bak`. Tables: `[modules.<id>]`, `[services.<name>]`, `[switches]`, `[network]`, `[history]`, `[relay]`, `[remote]`. Phases 2 and 3 are open (What's left) | Every setting is a key in `hosttoml.rs`'s table with a comment for new files (AGENTS.md) |
| RD25 | Admin API | **Per-person admin keys with roles** (`owner`, `admin`, `moderator`, `reader`), history revert, guarded actions and network-wide moderation (`crates/admin`, `docs/admin-api.md`; DD23, DH2, DH3b, DH6, DN11). `REFORGED_ADMIN_API_KEY` in `secrets.env` is the `owner` key; other keys are issued and revoked by command and recorded as the `Actor` in `store`'s change log. Destructive and bulk actions wait on a delay or a second operator; in a network, bans, timeouts and kicks propagate through the player's home node | Every admin change names its operator key and can be audited and reverted |
| RD26 | Server structure | **One executable per role on `crates/module-api`** (DD5, DF2–DF5). `crates/controller` hosts the game module and the core plugins (`gateway` with the allowlist, `content`, `admin`, `chat`, `network`, `remote`) in-process, as child processes over the frame protocol, or as libraries (RD27). The executables: `dainty-lass-client` and `dainty-lass-manager` (restricted's Tauri shells), `dainty-lass-server-console` (also `reforged-host`) and `dainty-lass-server-minimal` (restricted), and the MIT core `dainty-lass-server`. Each service is still runnable alone as a hidden `reforged-host <service>` subcommand | `docs/architecture.md` describes it; the controller and module host have checklists and fuzz targets |
| RD27 | Licence boundary | **MIT at the root, AGPL-3.0-only in `restricted/`** (maintainer decisions 2026-11-02 and 2026-11-05). A root package must be MIT or Apache-2.0 and must not reference `restricted/`; CI checks both. Code ported from DR, Paradox or the Reforged Revival line, or linking it, goes in `restricted/`; anything at the root that came from them was rewritten clean-room: a spec agent reads the AGPL code and writes `docs/port/spec-*.md` with no code in it, a separate implementer sees only the spec and the MIT code, and a similarity review is recorded in `docs/port/clean-room/`. Services and modules can be libraries the MIT core loads over a C ABI carrying the DD5 frame protocol (`crates/plugin-host`, the one MIT crate allowed `unsafe`; `controller::service_library`; `host.toml`'s `[services.<name>]` with `library` and `sha256`). Phase 1 (restricted's `host` composing the services in-process, `compose.rs`) and phase 2 (the libraries) are both in place | The MIT core and crates can ship on their own; restricted ships the binaries that link the AGPL shell |

## 2. Rules for a row

[`AGENTS.md`](AGENTS.md) has the full rules ("How work lands"). The shared files and who may change them:

| File or folder | Owner | Other rows |
| --- | --- | --- |
| `Cargo.toml`, `restricted/Cargo.toml`, `rust-toolchain.toml`, `deny.toml`, `.cargo/`, the lint configs | Integration pass | Never; a crate is added only by its skeleton row or the integration pass |
| `.forgejo/workflows/`, `.github/` | RR3, DX3; RH6 for `server-release.yml`; RA14 for `launcher-release.yml` | Never |
| `README.md`, `AGENTS.md`, `restricted/AGENTS.md`, `docs/` index pages, `SECURITY_REVIEW.md`, `restricted/SECURITY_REVIEW.md`, `MIGRATION_PLAN.md`, `docs/dainty-lass-plan.md`, `LICENSE.txt`, `restricted/NOTICE.md` | Integration pass | Never; a row writes its own page under `docs/` |
| `crates/module-api` | DF2 | A needed capability goes through the integration pass |
| `crates/protocol` | RS1 | A needed type goes through the integration pass |
| `crates/store` (`Database`, change log, migrations, `portable`, `transfer`) | DF7, DH1, DH3a, DH5 | A seam change goes through the integration pass |
| `crates/hostkit` (`cli.rs` and the `clap` tree, `paths`, `envfile`, `hostconfig`, `hosttoml`, `serve`) and `restricted/crates/host` (`compose.rs`) | RH1, DF3 | Fill the one command module the row owns |
| `restricted/launcher/src-tauri` (`main.rs`, `bridge/spec.rs`, the generated types, the isolation hook) | RA1–RA2, DF8 | Fill the one module the row owns |
| `crates/rustic-isles*` | Their specs (`docs/port/spec-rustic-isles-*.md`) | A behaviour change goes into the spec first (RD27) |

## 3. Things that must not change

- Launcher: the account keys already stored (read once, then re-stored), their binding to the certificate
  fingerprint, the `reforged-revival://` invite formats v1/v2, relay port 61100, the runtime acceptance per server
  and the check before every launch, the 1.14.7 executable hash, the page's look and both languages word for word;
  the key never crosses the bridge or reaches a log.
- Servers: the gateway's certificate and so every invite's `fp`; every route the game, the runtime and the launcher
  use, matched as Express matches them; JSON replies equal in value and in text (DR SB1); the XMPP stanzas the
  client reads; the host-only routes and headers stay refused; the content server's file layout; every player's
  data, carried over field for field from MongoDB to SQLite or Postgres (`db migrate`, `db verify`); API keys,
  password hashes and tokens issued by the Node backend keep working.
- Host kit: the env files it writes, the settings a Node kit's `host.json` holds (read as they are, and moved
  to `host.toml` by the first `setup`, RD24), and every command's name and options, so an existing host moves over
  with `update`.
- Smoke list at every step: register with an invite, login, Ramsgate, a hunt, chat in the City room, runtime
  download, ban, clean stop.
