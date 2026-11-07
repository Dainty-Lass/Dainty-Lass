<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Migration guide: where every upstream piece lives in the Rust codebase

The map from the code this repository came from to the code it is now, and the method for carrying the next
upstream change across. It is the backbone for every later port: a change in Dauntless Revived's, Mystic Paradox's
or Freeza's line is first placed with section 4, then moved with section 5, and the row that makes a new module
adds its line to section 2. Written 2026-10-24 against `plan` at `cb3d7e5`; the paths follow the RD27 split
(2026-11-02), the clean-room rewrites that came with it, and `restricted/` moving in-tree (2026-11-05).

[`MIGRATION_PLAN.md`](../../MIGRATION_PLAN.md) says what is done and why (the RD decisions, the RS/RH/RA/RB rows);
[`AGENTS.md`](../../AGENTS.md) says how work lands. This guide does not repeat them. It answers two questions those
documents leave to the reader: *where did this upstream file go*, and *where does the next upstream change go*.

How to read the tables:

- "DR" is Dauntless Revived at `e1c5fec`, the tip of its `plan` branch, where its own Rust port stopped on
  2026-10-22. "DR Node" means its Node sources (`legacy/Undaunted*/` there), "DR Rust" its `rust/` and
  `legacy/UndauntedLauncher/src-tauri/` there.
- "Reforged Node" means the Reforged Revival line's Node projects, carried under `legacy/` here until 2026-10-26, as they were when the Rust
  copy was made (`main` at `77fb622`, 2026-10-20).
- Paths are from the repository root on `plan`. The MIT workspace is the root (`crates/`, `fuzz/`, `ui/`,
  `launcher/`); everything AGPL is the separate workspace under `restricted/` (`restricted/crates/dainty-host`, the
  `*-plugin` shells, `restricted/launcher/src-tauri`, `restricted/modules/`, `restricted/native/`,
  `restricted/legacy/`, `restricted/docker/`, the host and Paradox docs). A bare `modules/...` path in a table is
  under `restricted/`. There is no `rust/` folder any more.
- Since RD27 the MIT crates hold no DR or Paradox code: each was rewritten clean-room from a spec in this folder
  (`spec-*.md`, reviewed in `clean-room/`). The tables still name the DR file a module answers to, because that is
  where an upstream change is read; the Rust file is the rewrite, not a copy.
- Rows in the plan are named as there: RS3 is the gateway row, RA13 the controller row, and so on.

## 1. The sources

| Source | Taken at | What it is | How it came in |
| --- | --- | --- | --- |
| [Dauntless Revived](https://github.com/mixutin/dauntless-revived) (mixutin) | `plan` at `e1c5fec` | Node services for Dauntless 1.4.4 (`UndauntedGateway`, `UndauntedContent`, `UndauntedMetagame`, `UndauntedDeployServer`, `UndauntedDiscordBot`), an Electron launcher, and a half-finished Rust port of all of them (`rust/`, the Tauri launcher in `src-tauri`) | One snapshot copy of its Rust code (RD6: commit `5281638` for `rust/`, `c9f9d6a` for the gateway and content server, `a6e1870` for the launcher), fitted to Reforged. Nothing flows from DR afterwards |
| Reforged's own Node (Freeza's line, [`main`](https://github.com/FreezaMotoqueiro/Dauntless-Reforged-Revival)) | `main` at `77fb622` for the deltas; merged into `plan` since | `ReforgedGateway`, `ReforgedContent`, `ReforgedHost`, `ReforgedLauncher`: DR's Node code modified for 1.14.7, plus the host kit, which has no DR counterpart | Moved to `legacy/` (RR2, `b67966b`). What it changed against DR is in the delta lists (RX1) and was carried onto the DR Rust copy; what changes after the copy goes on the catch-up lists (RX2) |
| [Mystic Paradox](https://github.com/pranav158/Mystic-Paradox) (Pranav Karande) | Base `00d3be8`, now `21b9ff5` | The 1.14.7 port: Node backend and Director, the C++ game runtime (`MysticParadox.dll`, `Rustic-Isles.dll` since DX4), its loader (`winmm.dll`, Rust) and the CatalogExporter | Forked with its history through [SilentSpud/Mystic-Paradox](https://github.com/SilentSpud/Mystic-Paradox) (`reforged` branch at `24e60c8`) and merged as a subtree into `paradox/` and `native/` (RD1, `e34cf60`). Updated to `21b9ff5` by replaying the Reforged patches (`ab1830a`) |
| Freeza's patches 0001–0039 | `upstream/patches/` on `main` | The Reforged changes to Mystic Paradox, kept as `git diff` files applied by the Node host kit | One commit each in the fork's `reforged` branch (0001–0033), then ordinary commits to `paradox/` and `native/` (0034–0039). `restricted/docs/paradox-patches.md` says what each one did and what is left of it after `21b9ff5` |
| [Undaunted](https://github.com/SyST3MDeV/Undaunted) (gwog) | — | The project both DR and Mystic Paradox are modified versions of | Nothing taken directly; it is in both lines' history. Attribution in `NOTICE.md` |

## 2. The map

### 2.1 Folders

| Upstream | Here | Note |
| --- | --- | --- |
| DR `rust/` (workspace files, `crates/`, `fuzz/`) | the root workspace and `restricted/`'s | Every package in a flat `crates/<package name>` (RD27; DR's layout before, RD11) |
| DR `crates/dainty-gateway`, `crates/dainty-content` | `crates/dainty-gateway`, `crates/dainty-content` | Fitted to Reforged, then rewritten clean-room as MIT (RD27); sections 2.3 and 2.4 |
| DR `services/metagame`, `deploy` (at its tip, `24b7624`) | Not here | Removed on 2026-10-31 with Dauntless Revived 1.4.4: importing an old 1.4.4 server's accounts is the dainty-legacy plugin module's (`silentspud/dainty-legacy`) |
| DR `native/internal-server`, `client-mods` | Not here | Removed with Dauntless Revived 1.4.4 (2026-10-31) |
| — | `crates/rustic-rifts-*`; `restricted/modules/rustic-rifts` | The rifts crates (DD14) are MIT clean-room rewrites at the root (RD27); the AGPL ones were retired. `restricted/modules/rustic-rifts` keeps the transport DLL (`ParadoxTransport`) and `oracle`, which replays Paradox's answers against the MIT crates |
| DR `services/discord-bot` | not copied | The admin service (RD25) is Reforged's answer |
| DR `legacy/UndauntedLauncher/src-tauri`, `isolation/` | `restricted/launcher/src-tauri`; [`launcher/isolation`](../../launcher/isolation) here | Section 2.6 |
| DR `legacy/Undaunted*/` (the Node services) | not copied | Reforged's Node code is DR's modified; it is under `legacy/` here instead |
| DR `deploy/` (Windows, Linux and OpenBSD kits) | not copied | Replaced by the host kit (section 2.5). Ideas taken earlier into the Node kit: the nftables allowlist table (`deploy/unix-common/allowlist-core.mjs`), the Wine runner on `xvfb-run`, one systemd unit per capability |
| DR `.github/workflows/server-release.yml`, `ci.yml` | [`.forgejo/workflows/`](../../.forgejo/workflows) (`ci.yml` for both workspaces, `server-release.yml`) | Method reused (RH6, RR3); Reforged's binaries and families |
| Reforged `ReforgedGateway/`, `ReforgedContent/`, `ReforgedHost/`, `ReforgedLauncher/` | gone | Carried under `legacy/` until 2026-10-26, then removed with the rest of the upstream line; the Rust modules below replaced them. The catch-up lists name what they still owed |
| Reforged `upstream/` (the Paradox pin, patches, `store-captures.json`) | gone | `store-captures.json` is `restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/store-captures.json`; the patch texts are commit messages and `restricted/docs/paradox-patches.md` |
| Mystic Paradox `ParadoxBackend/`, `ParadoxDirector/` | `restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend`, `.../ParadoxDirector` | The rustic-isles submodule from DS6 (`paradox/backend`, `paradox/director` before), in-tree since RD27. Ported to Rust and cut over in code on 2026-10-28 (RU9), then re-created clean-room as the MIT `rustic-isles-*` crates (section 2.7); the Node reference stays for one release behind `--features paradox` until RB15 |
| Mystic Paradox's hunt-manifest module (new at `21b9ff5`) | `restricted/modules/rustic-isles/reference/1.14.7/shared/hunt-manifest` | Imported by the backend and the Director |
| Mystic Paradox `ParadoxRuntime/` | `restricted/modules/rustic-isles/native/runtime` | C++, built by each host (`native/paradox-runtime` before DS6) |
| Mystic Paradox `tools/RuntimeLoader/` | `restricted/native/runtime-loader` | A Rust crate of its own, outside the workspace (DD13) |
| Mystic Paradox `tools/CatalogExporter/` | `restricted/modules/rustic-isles/native/catalog-exporter` | Loaded into the game through the runtime loader (RD9) |
| Mystic Paradox `ParadoxLauncher/`, `ParadoxTransport/`, its release workflows, `.gitleaks.toml`, `docs/SELF_HOSTING.md` | not taken | Reforged has its own launcher; the ICE transport belongs to Paradox's player-hosted realms |
| Mystic Paradox top-level `ADDITIONAL_TERMS.md`, `LICENSE`, `NOTICE.md`, `README.md`, `docs/` | `restricted/modules/rustic-isles/<same>` | rustic-isles' top level (`paradox/<same>` before DS6) |

### 2.2 Shared crates (from DR `crates/`)

| Crate | DR origin | Changed here | Not used here |
| --- | --- | --- | --- |
| [`crates/dainty-protocol`](../../crates/dainty-protocol) | DR X4: wire types the launcher and servers share | Rewritten clean-room from [`spec-protocol.md`](spec-protocol.md) as MIT (`DL3b`, [`clean-room/protocol.md`](clean-room/protocol.md)): `dauntless://` and `reforged-revival://` schemes, `dauntless-revived://` typed refusal (RD5), `RRK_` keys (`account.rs`), runtime manifest (`manifest.rs`), status sanitiser (`status.rs`), and `text.rs`. Shared test tables in `tests/cases/*.json` | DR's 1.4.4 fields |
| [`crates/dainty-proxy`](../../crates/dainty-proxy) | DR X5: hop-by-hop headers, idle cut, WebSocket tunnel | Rewritten clean-room from [`spec-proxy.md`](spec-proxy.md) as MIT (`DL3b`, [`clean-room/proxy.md`](clean-room/proxy.md)) | — |
| [`crates/dainty-service-kit`](../../crates/dainty-service-kit) | DR B1: Express-compatible routing, `JSON.stringify` output, pino logs, `--env-file` config, the X7 listen and stop cycle | `context.rs` and `telemetry.rs` are born here (RD26, section 2.10); the remaining 12 HTTP/routing/JSON/rate-limit/env/log modules were rewritten clean-room from [`spec-service-kit.md`](spec-service-kit.md) as MIT (`DL3b`, [`clean-room/service-kit.md`](clean-room/service-kit.md)) | — |
| [`crates/dainty-controller`](../../crates/dainty-controller) | none | New (RD26, DF3, section 2.10, MIT): `controller.rs`, `modules.rs`, and the `ModuleRegistry` (`registry.rs`, `registry/process.rs`, `registry/remote.rs`); decoupled from `gateway`, `content`, and the game module via `CorePluginFactory` | — |
| [`crates/dainty-module-api`](../../crates/dainty-module-api) | none | New (DF2; DD3–DD5, MIT per DD26): `GameVersion`, manifests, the `GameModule` and `CorePlugin` traits, `Registrar`, the out-of-process frame protocol and both adapters, and the `network`, `chat`, `history` and `panel` seams | — |
| [`crates/dainty-epic`](../../crates/dainty-epic) | none | New (DE1, DD22, MIT): Epic sign-in, library, manifest and chunk reading, and CDN download | — |
| [`crates/dainty-history-ship`](../../crates/dainty-history-ship) | none | New (DH4, DD23, MIT): shipping change-log segments and database snapshots to an object-locked S3 bucket or peer log directory and restoring from them | — |
| [`crates/dainty-store`](../../crates/dainty-store) | DR D10, D11, D21: synchronous Diesel on a blocking pool, embedded migrations | DF7 (DD15): `MultiConnection` over SQLite and Postgres (`connection`, `database`, `engine`, `portable`, `transfer`, `recovery`, `revert`, `declared`), plus DH1/DH3a/DH5 change log and hash chain (`history`, `history/chain.rs`, `history/record.rs`). The 6 SQLite helper modules (`lib.rs`, `adopt.rs`, `backup.rs`, `drizzle.rs`, `readonly.rs`, `snapshot.rs`) were rewritten clean-room from [`spec-store.md`](spec-store.md) as MIT (`DL3b`, [`clean-room/store.md`](clean-room/store.md)) | — |
| [`crates/dainty-hostkit`](../../crates/dainty-hostkit) | none | New (`DL3b`, MIT, [`clean-room/hostkit.md`](clean-room/hostkit.md)): the 18 born-here host-kit modules moved from `crates/dainty-host` plus the 22 host-kit module groups rewritten clean-room from [`spec-dl3-hostkit.md`](spec-dl3-hostkit.md), leaving `crates/dainty-host` as a thin AGPL binary/legacy/isles adapter shell | — |
| [`crates/dainty-launcher-kit`](../../crates/dainty-launcher-kit) | none | New (`DL4b`, MIT, [`clean-room/launcher-kit.md`](clean-room/launcher-kit.md)): the born-here launcher modules (`bridge/modules`, `net/regionping`, `net/versions`) plus the 8 launcher module groups (`bridge/types`, `net/http`, `net/runtime_files`, `net/verify`, `platform/{hostkit,hostserver,requirements,loader_session,dialogs}`) rewritten clean-room from [`spec-dl4-launcher.md`](spec-dl4-launcher.md) | — |
| [`crates/dainty-network`](../../crates/dainty-network) | none | New (DN2–DN5, DH7, MIT): node CA/certificates (`cluster`), directory (`directory`), home-server API and session tickets (`home`), replication (`replication`) and peer log shipping (`logship`) | — |
| [`crates/dainty-chat`](../../crates/dainty-chat) | DR `services/metagame/src/chat/` at `e1c5fec` | Ported into a standalone `CorePlugin` (DN10a, DN10b), then rewritten clean-room as MIT (RD27, [`clean-room/chat.md`](clean-room/chat.md)): XMPP over WebSocket, MUC rooms, whispers, presence, and cross-node room/whisper federation over `Service::Chat` | — |
| [`fuzz/`](../../fuzz) | DR X9: `http_body`, `proxy_headers`, `gateway_policy`, `launcher_input` | Reforged and Dainty Lass targets added, one per untrusted-input parser (RX3/DX2); the AGPL components' targets are in `restricted/fuzz/` | DR's `auth_token`, `bot_input`, `chat_frame`, `content_range` (metagame, bot and ranges) |

### 2.3 Gateway and allowlist helper (`crates/dainty-gateway`, rows RS3 and RS4)

Three-way: the Reforged Node file, the DR Node file it was modified from, the Rust module that answers to it, and
what Reforged changed. The per-line detail is in [`delta-servers.md`](delta-servers.md); the clean-room rewrite
(RD27) is [`clean-room/gateway.md`](clean-room/gateway.md). The processes start through `cli.rs`, not a `main.rs`.

| Reforged Node (`legacy/ReforgedGateway/src/`) | DR Node (`legacy/UndauntedGateway/src/`) | Rust (`crates/dainty-gateway/src/`) | Reforged's changes carried |
| --- | --- | --- | --- |
| `policy.ts` | `policy.ts` | `policy.rs` | `/reforged/api` with its four public routes; `/__origin/<host>` unwrapping; host-only prefixes `/admin`, `/launcher`, `/internal`, `/health`; Paradox's host-only headers; `backend` for `metagame`; only `POST /heartbeat` feeds the allowlist (Paradox has no `/game/login`, RD20) |
| `config.ts` | `config.ts` | `config.rs` | `GATEWAY_TLS=0` private mode on a Tailscale or loopback address; 443 or 61000 by mode; `GATEWAY_BACKEND_URL`; `GATEWAY_WS_URL` to the backend's own listener |
| `gateway.ts` | `gateway.ts` | `gateway.rs`, `guard.rs` | `x-reforged-gateway`; `x-forwarded-proto` by mode; plain HTTP in private mode |
| `feed.ts` | `feed.ts` | `feed.rs` | None; DR's helper-down backoff kept on purpose (RD20) |
| `server.ts` | `server.ts` | `server.rs`, `cli.rs` | Certificate loaded and checked in public mode only, never made; `mode` in the start-up log |
| `tools/make-cert.js` | `tools/make-cert.js` | `cert.rs` (`gateway make-cert`) | Reforged's organization name and usage text |
| — | `capture.ts` (DR M2) | `capture.rs` | Ported into Rust for RB2 (RD19); off unless configured |
| `ip.ts`, `ratelimit.ts`, `redact.ts` | identical | `ip.rs`, `ratelimit.rs`, `redact.rs` | None |
| `log.ts` | identical | `service-kit` `linelog` | None |
| `allowlist/config.ts` | `allowlist/config.ts` | `allowlist/config.rs` | `ALLOWLIST_FIREWALL` (`windows` or `nftables`), `ALLOWLIST_NFT`, `ALLOWLIST_NFT_TABLE`, `ALLOWLIST_INTERFACE`; DR's `DR_*` names ignored |
| `allowlist/ports.ts` | (`CheckPorts` in `config.ts`) | `allowlist/ports.rs` | The game's UDP range from `ALLOWLIST_PORTS`, widened by the host kit's `--max-hunts` |
| `allowlist/nft.ts` | — (adapted from DR's `deploy/unix-common/allowlist-core.mjs`) | `allowlist/nft.rs`, `allowlist/firewall.rs` | Table `reforged_revival` swapped in one transaction, `lo` rule, nft at its configured path only (RD20) |
| `allowlist/firewall.ts` | `allowlist/firewall.ts` | `allowlist/firewall.rs`, `allowlist/windows.rs` | Reforged's Windows Firewall rule name and display name; the backend choice |
| `allowlist/helper.ts` | `allowlist/helper.ts` | `allowlist/helper.rs` | The backend choice |
| `allowlist/server.ts` | `allowlist/server.ts` | `allowlist/cli.rs` | nftables needs Linux and a working `nft list tables`; the administrator check is Windows-only |
| `allowlist/state.ts`, `allowlist/sync.ts` | identical | `allowlist/state.rs`, `allowlist/sync.rs` | None |
| — | — | `modules.rs`, `nodelink.rs`, `network.rs` | New (DF4a, DN3): module route table and `/versions`, mutual-TLS `/__node/<service>` listener and client pool, `/network` peer directory |
| tests | — | `tests/`, `tests/parity/` (recorded from the Node gateway) | Parity fixtures are the contract |

### 2.4 Content server (`crates/dainty-content`, row RS2)

| Reforged Node (`legacy/ReforgedContent/src/`) | DR Node (`legacy/UndauntedContent/src/`) | Rust (`crates/dainty-content/src/`) | Reforged's changes carried |
| --- | --- | --- | --- |
| `runtime.ts` | — | `runtime.rs` (type in `protocol::manifest`) | New: the two-file runtime (`Rustic-Isles.dll`, `winmm.dll`), `runtime.json`, states `ok`/`none`/`invalid`, the 10 s re-check, the manifest written byte for byte for a stable ETag |
| `app.ts` | `app.ts` | `app.rs`, `input.rs` | `GET /content/v1/runtime/manifest` and `/runtime/files/<name>` in place of DR's game manifest and files; no HTTP ranges; `x-reforged-user-api-key`; no CORS headers; 256 KiB chunks |
| `auth.ts` | `auth.ts` | `auth.rs`, `client.rs` | Key lookup at `GET /reforged/api/GetUserInfo` |
| `config.ts` | `config.ts` | `config.rs` | Loopback binds only; `BACKEND_URL` (loopback `http:`); `CONTENT_RUNTIME_DIR`; `CONTENT_GAME_DIR`, `CONTENT_MANIFEST`, `CONTENT_DOWNLOAD_BASE_URL`, `CONTENT_ALLOW_ANY_BIND` gone; 4 streams per account |
| `server.ts` | `server.ts` | `server.rs`, `cli.rs` | No game-folder check at start; a missing runtime is a warning; `SIGBREAK` |
| `branding.ts`, `limits.ts`, `log.ts`, `news.ts` | identical | `branding.rs`, `limits.rs`, `log.rs`, `news.rs` | None |
| — | `gamedir.ts`, `manifest.ts`, `paths.ts`, `verify.ts`, `range.ts`, `data/dauntless-1.4.4.json` | dropped | The 1.4.4 game download. Only `if_none_match_hits` survives, in `app.rs` |

### 2.5 Host kit (`crates/dainty-hostkit`, bin `reforged-host`, rows RH1–RH6)

No DR counterpart: DR's kits were PowerShell and shell scripts under `deploy/`. The Rust kit follows the Node kit's
files one to one; it is MIT in `crates/dainty-hostkit` here (`DL3b`, [`clean-room/hostkit.md`](clean-room/hostkit.md)),
and `restricted/crates/dainty-host` is the AGPL shell that composes the services around it. Its tests run
against fixtures generated from the Node kit (`restricted/crates/dainty-host/tests/fixtures/node/`).

| Reforged Node (`legacy/ReforgedHost/src/`) | Rust (`crates/dainty-hostkit/src/`) | Note |
| --- | --- | --- |
| `cli.ts` | `cli.rs`, `commands/mod.rs`; the binaries are `restricted/crates/dainty-server` | The same commands and options: `setup`, `start`, `invite`, `doctor`, `gamedata`, `publish-runtime`, `preset`, `multipliers`, `update`, `backup`, and the Linux-only `gameserver` |
| `hostconfig.ts` (`host.json`) | `hostconfig.rs`, `hosttoml.rs` | `host.toml` with the same fields (RD24); a Node kit's `host.json` is read until the first write |
| `envfile.ts`, `paths.ts`, `run.ts` | `envfile.rs`, `paths.rs`, `run.rs` | `paths.rs` knows the `modules/rustic-isles` and `native/` layout (DS6; `paradox/` before) |
| `services.ts` (the process tree) | `services.rs`, `commands/start/` | One process tree; runs the Rust gateway, content, allowlist, admin, `rustic-isles-1-14-7` backend and `rustic-isles-director` by default (Node and MongoDB only behind `--features paradox` until RB15); each service gets the shell's environment, then its env file (`service_environment`; no `SERVER_PREFIXES` filter since the clean-room rewrite) |
| `doctor.ts` | `doctor.rs`, `commands/doctor.rs` | Checks `modules/rustic-isles` and `native/` instead of the patch marker |
| `invite.ts` | `invite.rs`, `commands/invite.rs` | Admin routes on loopback only |
| `mongo.ts` | `mongo.rs`, `mongo/wire.rs` | Retained for `update` migration and `--features paradox` rollback until RB15 |
| `gamedata.ts`, `gamepack.ts`, `progression.ts`, `captures.ts` | `gamedata.rs`, `gamepack.rs`, `progression.rs`, `captures.rs`, `commands/gamedata.rs` | Runs the in-process Rust generators in `rustic-isles-gamedata` and `rustic-isles-director` (RB11) |
| `preset.ts` | `preset.rs`, `commands/preset.rs` | `preset/` hash-locked by `preset.json` |
| `publish.ts` | `publish.rs`, `commands/publish_runtime.rs` | The Host page calls this command (RD23) |
| `selfupdate.ts` | `selfupdate.rs`, `commands/update.rs` | From the server release (RD8), no longer a `git` fast-forward |
| `upstream.ts` | gone | RP4: nothing is downloaded from Mystic Paradox |
| `linux.ts`, `wine.ts`, `gameserver.ts` | `linux.rs`, `wine.rs`, `cli.rs` (`Gameserver`) | systemd units, Wine or Proton on `xvfb-run` |
| `linux/setup-vps.sh`, `linux/Deploy-FromWindows.ps1` | not carried; gone with `legacy/` on 2026-10-26 | RH5 reduces them to fetching the release and calling `reforged-host`; `restricted/docs/hosting-linux.md` is the host's view |
| — | `commands/setup/` (`database`, `firewall`, `install`, `options`, `paradox`, `secrets`), `commands/backup.rs`, `commands/multipliers.rs` | The Rust split of the Node `setup` and its helpers; `paradox` also copies a host's files from the layouts before RP4 and DS6 |
| — | `serve.rs`, `repl.rs`, `tui.rs`, `hostconfig/yaml.rs` | New (RD26): the server as one process, its console and dashboard, and `host.yaml`. No Node counterpart, so an upstream host-kit change never lands here first |
| — | `guarded.rs`, `hostconfig/guard.rs`, `hostconfig/modules.rs`, `hostconfig/env.rs`, `commands/history.rs`, `commands/network.rs`, `epicinstall.rs`, `commands/install_game.rs` | New (DF3, DN2/DN3, DH3b, DH6, DE6, RD24): `[guard]`, `[modules]`/`[network]` and `[env.<service>]` config, guarded action CLI, `history`/`revert` CLI, `network ca-init`/`issue-node-cert`/`list`/`status`, and `install-game` from Epic |

### 2.6 Launcher (`launcher`, rows RA1–RA13)

The Rust backend is DR's Tauri launcher fitted to Reforged, in `restricted/launcher/src-tauri/src/`; the
"Here" column is relative to it. The modules listed for `crates/dainty-launcher-kit` in section 2.2 were rewritten
clean-room into this repository (`DL4b`). The page is dainty-dash, this repository's `ui/` (`apps/web`, built with
`DAINTY_MODE=launcher` or `manager` by `launcher/scripts/dashboard-page.ts`), talking to Rust through
`ui/packages/bridge` and the bridge types copied in from `restricted/`. The per-line detail is in
[`delta-launcher-platform.md`](delta-launcher-platform.md) and
[`delta-launcher-controller.md`](delta-launcher-controller.md).

| Reforged Electron (`legacy/ReforgedLauncher/src/`) | DR Rust module (`src-tauri/src/`) | Here | Reforged's changes carried |
| --- | --- | --- | --- |
| `main/controller.ts` | `controller/mod.rs`, `platform.rs`, `stored.rs` | the same, plus `controller/game.rs`, `runtime.rs`, `host.rs` | The 1.14.7 flow: find the game instead of downloading it, accept and install the runtime, the Host page; the relay in both modes; moved-server detection; the 1.4.4 path dropped (RD5) |
| `main/ipc-validate.ts`, `preload.ts`, `renderer/global.d.ts` | `bridge/spec.rs`, `commands.rs`, `generate.rs`, `isolation/hook.js` | the same | Reforged's command list; new argument kinds (index, fixed names, objects); `copy_text` a command (RD22) |
| `main.ts` | `lib.rs`, `window.rs`, `platform/dialogs.rs`, `invite_link.rs`, `art.rs` | the same, plus `platform/clipboard.rs` | Reforged's identity, `userData` folder, `reforged-revival://` scheme, `rr-art` art scheme |
| `shared/types.ts`, `status.ts`, `invite.ts`, `username.ts` | `bridge/types.rs`, `core/status.rs`, `core/invite.rs`, `core/username.rs` | the same, on `crates/dainty-protocol` | Snapshot shape; `/reforged/api`; one scheme |
| `shared/i18n/` (`en.ts`, `pt-BR.ts`) | `core/i18n.rs`, `core/text.rs`, `build.rs` (`gen/strings.json`) | the same | Two languages (RD14); the 1.4.4 strings removed, the refusal string added |
| `main/constants.ts` | `core/pins.rs`, `platform/paths.rs`, `platform/links.rs`, `tauri.conf.json` | the same | 1.14.7 build, changelist and executable hash; `Jackal`; relay 61100; the runtime's two file names; Reforged's URLs |
| `main/pinned.ts`, `http.ts`, `hostapi.ts` | `net/pinned.rs`, `connector.rs`, `http.rs`, `hostapi.rs` | the same, plus `net/http/to_file.rs`, `net/hostapi/parse.rs` | `/reforged/api`; `request_to_file` for the runtime files (RD22) |
| `main/relay.ts` | `net/relay.rs`, `net/relay_owner.rs` | the same, plus `net/relay/local.rs`, `net/relay_owner/table.rs` | Port 61100, `/__origin/` paths passed through, `/QoS` answered locally, both modes; DR's owner check kept on Windows (RD21) |
| `main/runtime-files.ts`, `verify.ts` | — , `net/verify.rs` | `net/runtime_files.rs` (+ `manifest.rs`), `net/verify.rs` (+ `cache_file.rs`) | New: accept a runtime set once per server, check before every launch; verify the 1.14.7 executable hash only |
| `main/keystore.ts` | `platform/keystore/{mod,os,legacy}.rs` | the same, plus `keystore/format.rs` | `RRK2` and bare keys only, bound to the certificate fingerprint; the Electron launcher's keys read once (RD22) |
| `main/settings.ts`, `log.ts`, `system.ts` | `platform/settings.rs`, `log.rs`, `system.rs` | the same | Reforged's folders; saved 1.4.4 servers read and shown disabled (RD22) |
| `main/engineini.ts`, `display-repair.ts`, `game-folder.ts`, `launch.ts` | `platform/game_config.rs`, `game_folder.rs`, `launch.rs` | the same | Engine.ini XMPP section to the relay, Game.ini `[OnlineSubsystemPhoenix]` removed; Legendary, Heroic, Epic or a folder; the 1.14.7 command line with `REFORGED_BACKEND_URL` |
| `main/links.ts` | `platform/links.rs` | the same | Reforged's URLs |
| `main/hostkit.ts`, `hostserver.ts`, `requirements.ts` | — | `platform/hostkit.rs` (+ `parse.rs`), `hostserver.rs`, `requirements.rs` | New: the Host page drives `reforged-host` from the release (RD23) |
| `main/injector.ts` | — | `platform/loader_session.rs` | Not ported as written: the runtime loader's `mystic_loader.ini` loads the host's chosen DLL (RD9) |
| `main/revived/` (nine files) | `net/autoregion.rs`, `platform/client_mods.rs`, `core/manifest.rs`, `manifest_check.rs`, `net/downloader.rs` | dropped | DR's 1.4.4 servers (RD5); the downloader went with them (RD22) |
| — | `offline/`, `controller/offline.rs` | dropped | Reforged has no offline mode |
| — | `platform/linux_runtime.rs`, `platform/updater.rs` | the same | Kept for RA10 (Proton) and RA14 (updates) |
| — | `net/downloader.rs` | `net/epic_install.rs`, `platform/epic_{session,login,library}.rs`, `platform/keystore/epic.rs`, `controller/install.rs` | New (DE2–DE5, DD22): Epic OAuth login, token refresh in the keystore, library check, and chunked CDN installer (keeping DR's `downloader.rs` structure) |
| — | — | `net/regionping*`, `net/versions*`, `controller/{region,versions,guarded}.rs`, `server_history.rs`, `bridge/modules.rs`, `server_gui/modules.rs` | New (DF8, DU3, DN7, DH3b, DH6): `/versions` and `/network` probe, pinned-TLS region ping, module control and declarative panels, change-log history and revert in the server GUI, and guarded-action approval |
| test cases | — | `src-tauri/src/*/tests.rs`, this repository's `launcher/test/` and `crates/dainty-protocol/tests/cases/` | Reforged's `*.test.ts` cases ported |

### 2.7 Backend and Director: the `rustic-isles-*` crates (RU1–RU7, RD27)

The RU rows built an AGPL port (`isles-core`, `isles-db`, `isles-chat`, `isles-director`, `isles-1-14-7`) in
`modules/rustic-isles/crates`. On 2026-11-05 it was replaced by MIT crates at the root, each written from a
behaviour spec (`spec-rustic-isles-*.md`) by an implementer who never saw the AGPL code, and the AGPL crates were
retired. `restricted/modules/rustic-isles/crates/rustic-isles-mit-parity` replays the recorded goldens against the composed
MIT module (890 of 890 matched, `restricted/modules/rustic-isles/docs/mit-parity-report.md`), and
`rustic-isles-plugin` builds it as a library with restricted's version data.

| Mystic Paradox | Reference tree (under `restricted/`) | Rust home |
| --- | --- | --- |
| `ParadoxBackend` (Express, `ws`, `ltx`, MongoDB) | `modules/rustic-isles/reference/1.14.7/ParadoxBackend` | `crates/rustic-isles` (pipeline, route table, settings) and its areas: `rustic-isles-auth`, `-reforged` (`/reforged/api` and admin), `-characters`, `-economy` (inventory, reforge, progression, bounties, store), `-matchmaking`, `-chat`, `-gamedata`, on `rustic-isles-store` (SQLite + Postgres, DD15); composed for 1.14.7 by `crates/rustic-isles-1-14-7` |
| `ParadoxBackend/scripts/` (game-data generators, admin bootstrap, data migrations) | `modules/rustic-isles/reference/1.14.7/ParadoxBackend/scripts` | `crates/rustic-isles-gamedata`, `crates/rustic-isles-director` (hunt tables), and `crates/rustic-isles-db` (`migrate`, `verify`, `export-mongo`) and `crates/rustic-isles-replay` (RU7/RB11/RB13) |
| `ParadoxBackend/realtime/` (XMPP) | `modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/realtime` | `crates/dainty-chat` (DN10a/b) and `crates/rustic-isles-chat` (rules and routes) |
| `ParadoxDirector` | `modules/rustic-isles/reference/1.14.7/ParadoxDirector` | `crates/rustic-isles-director` (RU6: instance registry, hub supervisor, port leases, memory guard, hunt tables) |
| Realms and Guard (`p2p/`, `realms/`, Guard) | `modules/rustic-isles/reference/1.14.7/ParadoxBackend` | `crates/rustic-rifts-*` (`rustic-rifts-seam`, `rustic-rifts-admission`, `rustic-rifts-relay`, `rustic-rifts-realms`, `rustic-rifts-guard`, `rustic-rifts-host`, `rustic-rifts-transport`; RF1–RF7, MIT clean-room), wired in by `rustic-isles-1-14-7`'s `rifts.rs`; the transport DLL stays in `restricted/modules/rustic-rifts/src` |

### 2.8 The patches to Mystic Paradox

Each patch's current state (kept, reduced, dropped because upstream now does the same) is the table in
`restricted/docs/paradox-patches.md`. By where they live (all under `restricted/`) and who ports them:

| Patches | Live in | Carried into Rust by |
| --- | --- | --- |
| 0001 (runtime part), 0004, 0008, 0011–0016, 0020, 0024, 0026, 0027, 0029, 0031, 0035, 0037–0039 | `modules/rustic-isles/native/runtime` | Nobody: the runtime stays C++ and keeps taking Freeza's patches (`restricted/docs/paradox-runtime-patches.md`) |
| 0002 (`/reforged/api`, `AUTH_MODE=APIKEY`, HTTP-only mode), 0018 (login queue) | `modules/rustic-isles/reference/1.14.7/ParadoxBackend` | RB7; now `rustic-isles-reforged` and `rustic-isles-auth` |
| 0005, 0007, 0010, 0017, 0023, 0025, 0028, 0030, 0036; 0009 and 0032 as upstream's own code | `modules/rustic-isles/reference/1.14.7/ParadoxBackend` | RB8 (areas) and RB9 (chat: 0017, 0025); now the `rustic-isles-*` areas |
| 0003, 0021, 0022, 0033 | `modules/rustic-isles/reference/1.14.7/ParadoxDirector` | RB10; now `rustic-isles-director` |
| 0006, 0019, 0034 | dropped | Upstream `21b9ff5` has the same fix |
| The RD25 kick route (`POST /reforged/api/Kick`) | `modules/rustic-isles/reference/1.14.7/ParadoxBackend` | RB7, as the one post-fork backend route; now `rustic-isles-reforged` |

### 2.9 Not ported, on purpose

- DR's `discord-bot` service, its `deploy/` kits, `tools/`, `friend-kit/` and documentation site: Reforged's
  equivalents are `reforged-host` and the admin service. (DR's `metagame` and `deploy` came in for 1.4.4 servers;
  section 2.1.)
- DR's 1.4.4 launcher code (RD5): `legacy/ReforgedLauncher/src/main/revived/`, `net/autoregion.rs`,
  `platform/client_mods.rs`, the 1.4.4 manifest and DLL pins, `net/downloader.rs`, device signals, the DirectX check.
- DR's offline mode (`offline/`, `controller/offline.rs`), the `DRK2` key format and the `DR_*` allowlist names.
- Mystic Paradox's launcher and ICE transport, release workflows and self-hosting guide.
- Reforged's `injector.ts` (RD9) and `ReforgedHost/src/upstream.ts` (RP4).

### 2.10 New here, with no upstream

The admin service ([`crates/dainty-admin`](../../crates/dainty-admin), RD25); the whole host kit; the runtime manifest
and download (`content/src/runtime.rs`, launcher `net/runtime_files.rs`); the Host page backend and the loader session
(`platform/hostkit.rs`, `hostserver.rs`, `requirements.rs`, `loader_session.rs`); `protocol/src/text.rs`; the
nftables backend as Reforged wrote it; the fuzz targets and the `docs/security/` checklists; `host.toml`;
`docker/` and `compose.yaml`.

The server structure (RD26), from the `feat/app-structure` merge (`c57f157`) as its fix branches left it, has no
upstream either: the controller ([`crates/dainty-controller`](../../crates/dainty-controller): `controller.rs`, the
start and stop cycle; `modules.rs`, the Rust services as modules), `service-kit`'s `context.rs` (`ServiceContext`,
`StatusRegistry`) and `telemetry.rs` (the `tracing` layers: the console's table, the dashboard's ring, the server
GUI's channel, the log file), the host kit's `serve.rs`, `repl.rs`, `tui.rs` and `hostconfig/yaml.rs`, the
binaries ([`crates/dainty-lass`](../../crates/dainty-lass), `restricted/crates/dainty-server` and
`restricted/crates/dainty-server-minimal`), and the launcher's `server_gui.rs`. A DR or Reforged change to a
service lands in the service's own crate (sections 2.3 to 2.5) and reaches the controller unchanged, since a
module only calls the service's `Config::from_env` and `run_with_context`.

The Dainty Lass plan adds [`crates/dainty-epic`](../../crates/dainty-epic) (DE1, DD22: Epic sign-in, library, manifest and
chunk reading, and the download for installing from the player's Epic library), [`crates/dainty-module-api`](../../crates/dainty-module-api)
(DF2), [`crates/dainty-history-ship`](../../crates/dainty-history-ship) (DH4), [`crates/dainty-network`](../../crates/dainty-network)
(DN2–DN5, DH7, DN13), and [`crates/dainty-chat`](../../crates/dainty-chat) (DN10a/b), documented in
[`docs/architecture.md`](../architecture.md), [`docs/writing-a-module.md`](../writing-a-module.md), and
`restricted/docs/security/turn-relay.md`.
`store`'s `connection`, `database`, `engine`, `history`, `portable`, `transfer`, `recovery`, `revert` and `declared`
modules (DF7, DD15) and its change log and hash chain (`history.rs`, `history/chain.rs`, `history/record.rs`, DH1,
DH3a, DH5 and DD23) are new here too; `tests/db/{sqlite,postgres}` are the two-dialect test migrations. The SQLite-only
`Store` beside them answers to DR's and was rewritten clean-room (section 2.2). In `crates/dainty-admin`, `guard.rs`, `guarded.rs`, `guarded_cli.rs`, `key_cli.rs`,
`history.rs` and `history_text.rs` implement per-person keys, guarded actions and history/revert (DH2, DH3b, DH6);
`crates/dainty-hostkit` adds `guarded.rs`, `hostconfig/guard.rs`, `hostconfig/modules.rs`, `commands/history.rs`,
`commands/network.rs`, `epicinstall.rs` and `commands/install_game.rs`; `crates/dainty-gateway` adds `modules.rs`,
`nodelink.rs` and `network.rs`; and `restricted/launcher/src-tauri` adds `controller/guarded.rs`, `server_history.rs`,
`bridge/modules.rs`, `server_gui/modules.rs`, `platform/epic_{session,login,library}.rs`, `platform/keystore/epic.rs`,
`net/regionping*`, `net/versions*`, `net/epic_install.rs` and `controller/{region,versions,install}.rs`.

## 3. Rename dictionary

What a DR name became, so a DR commit or a DR Rust module reads straight across. The Node side of each pair is in
DR's `legacy/Undaunted*/`; Reforged's was under `legacy/` until 2026-10-26.

| DR | Reforged | Where |
| --- | --- | --- |
| `metagame` (the upstream, the config key) | `backend` | gateway `policy.rs`, `config.rs`; content `config.rs` |
| `/undaunted/api/...` | `/reforged/api/...` | gateway policy, content auth, launcher `net/hostapi.rs`, `protocol` |
| `x-undaunted-user-api-key` | `x-reforged-user-api-key` | content `input.rs`, gateway, launcher |
| `x-undaunted-gameserver-apikey` | `x-rustic-isles-gameserver-apikey` (`x-mysticparadox-gameserver-apikey` before DX4, still refused) | gateway policy (host-only header) |
| `x-dauntless-gateway` | `x-reforged-gateway` | gateway `gateway.rs` |
| `GATEWAY_METAGAME_URL`, `METAGAME_URL` | `GATEWAY_BACKEND_URL`, `BACKEND_URL` | gateway and content `config.rs` |
| `GATEWAY_WS_URL` to port 61099 | the backend's own listener (61000) | gateway `config.rs` |
| `DR_NFT_BINARY`, `DR_NFT_TABLE`, `DR_FIREWALL_INTERFACE` | `ALLOWLIST_NFT`, `ALLOWLIST_NFT_TABLE`, `ALLOWLIST_INTERFACE` | `allowlist/config.rs` |
| nft table `dauntless_revived` | `reforged_revival` | `allowlist/nft.rs` |
| Windows Firewall rule `Dauntless Revived game ports (allowlist)` | `Dauntless Reforged Revival game ports (allowlist)`, name `ReforgedRevival-GamePorts-Allowlist` | `allowlist/windows.rs` |
| certificate organization `Dauntless Revived server` | `Dainty Lass server` (DD1; Reforged Node's was `Dauntless Reforged Revival server`) | gateway `cert.rs` |
| `/content/v1/manifest`, `/content/v1/files/<path>` | `/content/v1/runtime/manifest`, `/content/v1/runtime/files/<name>` | content `app.rs` |
| `CONTENT_GAME_DIR` (required) | `CONTENT_RUNTIME_DIR` (optional) | content `config.rs` |
| `dauntless-revived://join` | `reforged-revival://join`; the old scheme parses to a typed refusal | `protocol/src/invite.rs`, launcher `platform/invite_link.rs` |
| `DRK2` key files | `RRK2` | launcher `platform/keystore/format.rs` |
| `UUK_` account keys (DR's metagame) | `RRK_<48 hex>` (the JSON field is still `UUK`) | `protocol/src/account.rs` |
| relay on 61000, XMPP on 61099 | relay on 61100 carrying HTTP and chat, `/QoS` answered locally | launcher `net/relay.rs`, `core/pins.rs` |
| `.dr-tmp` | `.rr-tmp` | launcher `platform/game_config.rs` |
| `dr-art` scheme | `rr-art` | launcher `platform/art.rs` |
| Dauntless 1.4.4, `PINNED_EXE_SHA256` `d3d41e61…`, exe size pin | 1.14.7 `rel-1.14.7-Archon`, changelist 647472, `f3f95fbf…`, no size pin, `-epicapp=Archon`, Epic app name `Jackal` | launcher `core/pins.rs` |
| `[SystemSettings]` memory lines, exposure option | gone; `[OnlineSubsystemPhoenix]` removed from Game.ini before launch | launcher `platform/game_config.rs` |
| DR's nine languages | `en`, `pt-BR` | launcher `core/i18n.rs` |

## 4. Where the next upstream change goes

Place the change first; every row below names the method and where the port is recorded.

| The change is in | It lands in | How | Recorded in |
| --- | --- | --- | --- |
| A runtime patch written against Mystic Paradox's tree | `restricted/modules/rustic-isles/native/runtime`, `restricted/native/runtime-loader` | Paths rewritten and `git apply --3way`, one commit per patch named `NNNN-runtime-<name>`, as `restricted/docs/paradox-runtime-patches.md` says | A section in `restricted/docs/paradox-patches.md` |
| A backend or Director patch written against Mystic Paradox's tree | `restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend`, `.../ParadoxDirector` | The same rewrite (`ParadoxBackend/` to `restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/`), applied as an ordinary commit; a new or changed route goes in RB1's inventory. The MIT `rustic-isles-*` crate takes it only through its spec: describe the change in `docs/port/spec-rustic-isles-<area>.md` without code, implement from the spec, and replay the goldens with `rustic-isles-mit-parity` | `restricted/docs/paradox-patches.md`; the spec's change note |
| Game data, `store-captures.json` | `restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/store-captures.json`; a host's own preset | Taken as they are | The commit |
| Mystic Paradox's own `main` | rustic-isles (its `upstream/main` branch, archived), then `restricted/modules/rustic-isles` | Advance the fork's `reforged` branch, replay the Reforged patches one by one onto the new base, keep, reduce or drop each, as `ab1830a` did. `ParadoxLauncher`, `ParadoxTransport` and the workflows stay out | The "Updated to" section and table in `restricted/docs/paradox-patches.md`; its `NOTICE.md` |
| Dauntless Revived | nothing | RD6: the copy is final. A DR Rust fix someone still wants is found by its module in section 2 and ported by hand, naming the DR commit | The commit body |
| A Rust service (not a migration) | its crate, in the workspace its licence puts it in (the root or `restricted/`) | AGENTS.md's rules | — |
| A new Rust module from a row | — | Its line in section 2 of this guide, by the integration pass | This guide |

The upstream line this repository forked from was removed on 2026-10-26 (`main` has a new root), and nothing is
merged from it any more. Its Node services lived under `legacy/` until then; the catch-up lists record what they
still owed, and are closed.

## 5. How to port one change

1. **Place it** with section 4 and read its entry: the catch-up list for a Node change, Freeza's patch section for a
   Paradox change, the delta file for a file ported for the first time.
2. **Find the Rust home** in section 2 and open the Node file and the Rust module side by side. Code copied from DR
   (only under `restricted/` now) keeps DR's structure, so a later reader can still diff it against `e1c5fec`; make
   the Reforged change and nothing else. An MIT crate here takes the change from a written description, never from
   the upstream code (RD27).
3. **Match Node where a client can see it.** Routes as Express matches them (`service-kit::routing`), JSON as
   `JSON.stringify` writes it (`service-kit::js`, `json`), the same status codes, headers and field order. When the Node
   and Rust answers differ, the Rust one is the bug unless a decision row says otherwise. Section 3 of the plan lists
   what must not change at all.
4. **Translate names** with section 3 before copying a DR change, and never the other way (a Reforged `.env` starts
   the Rust services unchanged).
5. **Port the tests with the code.** Reforged's `*.test.ts` cases become Rust tests; a reply the Node service made goes
   in the parity fixtures (`tests/parity/`, re-recorded only when the Node side changed on purpose, with the reason in
   the commit); an invite, status or username case goes in `protocol/tests/cases/*.json`, which the launcher and the
   servers both read. Tests that read the shared log take `serial()`; listeners bind `127.0.0.1:0`.
6. **A trust-boundary change** (gateway policy and TLS, allowlist, relay, pinned TLS, keystore, invite parsing, runtime
   download, the loader session, the host kit's secrets, the admin API) updates its fuzz target in `fuzz/` and
   its checklist in `docs/security/<component>.md` in the same commit (RX3).
7. **A configuration variable** is written and read in the same commit: `crates/dainty-hostkit/src/hostconfig.rs` and
   `hosttoml.rs` write it, the service's `config.rs` reads it, a test round-trips it, an older `host.toml` without it
   keeps today's behaviour, its name carries a server prefix, a secret goes in `secrets.env`, and the host docs'
   tables move with it (AGENTS.md, "Configuration").
8. ***Launcher text** goes at the end of both language files with the same keys and placeholders. In
   `restricted/`, `RR_UPDATE_GENERATED=1 cargo test -p dainty-launcher generated` regenerates `gen/strings.json` and,
   for a bridge command changed in `bridge/spec.rs`, its TS types, hook list and permissions. Generated files are
   never edited.
9. **Check** from the root of each workspace the change touched (the root, `restricted/`):

   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets --locked -- -D warnings
   cargo test --workspace --locked --no-fail-fast
   ```

   `bun run test` and `bun run lint` in `ui/`, `bun run test` in `launcher/`; in `restricted/`, `npm ci && npm test`
   in a rustic-isles `reference/1.14.7` package the change touched, `cargo test -p rustic-isles-mit-parity` when an
   `rustic-isles-*` crate changed, and `bash docker/run-tests.sh` when what the containers run changed. The real nftables test needs `REFORGED_TEST_NFT=1` on Linux. Nothing here plays
   the 1.14.7 client: say so when only the tests passed.
10. **Record it.** The catch-up entry's `Rust:` line becomes `ported` with the file and lines (section 6's format).
    The commit subject names the row, the body says what changed and why and names the source commit (`From
    FreezaMotoqueiro/Dauntless-Reforged-Revival <path> at <commit>`). The branch is `{type}/{slug}`; push it,
    report the SHA, open no pull request; no AI attribution lines (AGENTS.md, "Commits").
11. **Leave the hands-on part to the maintainer**, written down: a runtime build with a host's SDK and a played hunt,
    the canary, a real `start` with MongoDB, signing and publishing.

## 6. Catch-up entry format

One entry per Node commit, in the family's `docs/port/catchup-<family>.md`, newest last, with the range kept at the
top of the file and the summary counts updated:

```markdown
### `<short sha>` <subject>

- **SHA:** `<full sha>`
- **Date:** <author date, ISO with offset>
- **Author:** <name>
- **Files:** <paths under legacy/..., or the patch files>
- **Behavior:** <what a player, host or the game sees differently; "none" for a rename or comment>
- **Rust:** ported in `<path>:<lines>` | needs porting (<which row, what is missing>) | not applicable (<why>)
```

A commit with parts in more than one status is counted by the part that needs the most work and lists each part
under its `Rust:` line, as `catchup-host.md` does for `0ba6016`.

## 7. Pins

| What | Commit | Date |
| --- | --- | --- |
| Dauntless Revived, `plan` (the copy; its Rust port stopped here) | `e1c5fec` | 2026-10-22 |
| Reforged `main`, the Node sources the delta lists diffed | `77fb622` | 2026-10-20 |
| Reforged `main` last merged into `plan` (`02a8070`, patches 0034–0039) | `aea8f64` | 2026-10-23 |
| Mystic Paradox base (Freeza's pin) | `00d3be8` | — |
| Mystic Paradox now in `restricted/modules/rustic-isles` and `restricted/native/runtime-loader` | `21b9ff5` | 2026-10-18 |
| Submodules (DS6): rustic-isles, rustic-rifts, legacy-revived at their `plan` (since in-tree, RD27; legacy-revived removed) | `54970ea`, `b19248c`, `8a98d83` | 2026-10-26 |
| dainty-restricted imported as `restricted/` | `772248c2` (subtree of `e0e238b`) | 2026-11-05 |
| SilentSpud/Mystic-Paradox `reforged` (base + patches 0001–0033) | `24e60c8` | 2026-10-22 |
| Game build | `rel-1.14.7-Archon`, changelist 647472, Unreal Engine 4.26.2 | — |
| The upstream line (RD16) | removed 2026-10-26; `main` has a new root | — |

## 8. Related documents

- [`MIGRATION_PLAN.md`](../../MIGRATION_PLAN.md): decisions, rows, what is left, what must not change.
- [`AGENTS.md`](../../AGENTS.md): how a row works, lints, tests, configuration, text, branches and commits.
- [`delta-servers.md`](delta-servers.md), [`delta-launcher-platform.md`](delta-launcher-platform.md),
  [`delta-launcher-controller.md`](delta-launcher-controller.md): the file-by-file Reforged-against-DR deltas (RX1).
- [`catchup-host.md`](catchup-host.md), [`catchup-isles.md`](catchup-isles.md): Node commits since the copy and
  their Rust state (RX2, DS5), closed.
- `restricted/docs/paradox-patches.md` and `restricted/docs/paradox-runtime-patches.md`: how a runtime patch is
  applied.
- [`docs/architecture.md`](../architecture.md): how the parts talk and what each trusts.
- [`docs/security/`](../security): one checklist per trust-boundary component.
- `restricted/NOTICE.md`: where each AGPL part comes from, for attribution.
- Dauntless Revived's own plan, for the method behind the DR Rust code:
  [`MIGRATION_PLAN.md` on its `plan` branch](https://github.com/mixutin/dauntless-revived/blob/plan/MIGRATION_PLAN.md).
