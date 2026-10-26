<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Catch-up list: legacy-revived (DS5)

Every commit of this repository that carried Dauntless Revived (DR) code or changed it, and whether legacy-revived
needs it. legacy-revived's base is SilentSpud's fork of DR, `plan` at `24b7624` (DD10); RV1 applies what is marked
needed when it puts `metagame` and `deploy` on `module-api` (`docs/dainty-lass-plan.md`, DD10, RV1, RV3). The
entries follow the migration guide's section 6, with the `Rust:` line replaced by a `Revived:` line, since the
target here is legacy-revived's tree and not a Rust module here.

A needed part is one of two kinds:

- **Reforged-specific:** a change this repository made to DR's code that legacy-revived takes as it is (the rename
  dictionary, loopback-only listeners, the runtime loader, no game-derived files). RV1 applies it to DR's files.
- **Framework glue:** what made DR's code run inside this repository's workspace (the env-file prefixes, the
  `ServiceContext` entry points, `protocol` types). legacy-revived needs the same job done, but RV1 does it with
  `module-api` (`GameModule`, `ModuleContext`, the config sections of `Registrar`) instead of carrying this code.

## Range

- **legacy-revived's base:** `24b7624` (DR `plan`, 2026-10-24), `e1c5fec` plus 16 commits. None of those 16 is a
  copy of a commit listed here (`git patch-id --stable` over `e1c5fec..24b7624`); this repository's DR code was taken
  at `24b7624`, so they are all in it already.
- **Start:** merge `54e271a` (2026-10-25), which brought `feat/player-launcher-headless-server` onto the old `main`:
  `c76cb09` … `825d67f`, sixteen commits. The DR-derived ones are `c1be7f7` … `18ac587`; the two before and the four
  after are listed too, because they ride in the same merge and two of them touch DR-derived code.
- **Later commits:** every commit after `54e271a` that touched `rust/services/metagame`, `rust/services/deploy`,
  `native/internal-server` or `native/client-mods`: only `fef7ca9` (in merge `c57f157`).
- **End:** `main` at `f891d92` (2026-10-26).
- **This repository:** `main` was given a new root on 2026-10-26 (`3a756cf`), so the history before it comes from
  `dainty-lass-pre-rewrite-2026-10-26.bundle`: `git log --all --full-history` on the four paths over every branch in
  it, then the merge's own range. `main`'s four commits after the root (`74d87c4`, `804f1f4`, `d7ef297`, `f891d92`)
  touch none of the four paths. The four folders are identical in the bundle's `main` (`c57f157`) and in `main`, so
  the root brought nothing to them; what it changed next to them is under `3a756cf`.
- **Duplicates:** before the merge was rebased, the same work sat on `feat/player-launcher-headless-server` and six
  `feat/legacy-*` branches. Each copy is matched to the entry below by `git patch-id --stable`, or by `git range-diff`
  where only a lockfile hunk differs (table at the end). The copies are not listed again.
- **Method:** "already in DR" is decided by content, not ancestry. Folder against folder at `24b7624`:
  `rust/services/metagame` and `rust/services/deploy` to the same paths, `native/internal-server` to the same path,
  `native/client-mods` to DR's `client-mods`, and the vendored `node/` folders to DR's `legacy/UndauntedMetagame` and
  `legacy/UndauntedDeployServer`. Each difference is accounted for in the table below.

## Summary

| Status | Commits |
| --- | --- |
| Needed, Reforged-specific | 5 |
| Needed, framework glue (RV1, `module-api`) | 1 |
| Not applicable | 12 |

A commit with parts in more than one status is counted by the part that needs the most work; each part is listed
below it. The three big needed commits (`98d0112`, `9b8279c`, `77118f6`) also have glue and not-applicable parts.

What RV1 has to do, in short, on DR's own files:

1. The rename dictionary (migration guide section 3) in the metagame, the deploy server and the internal-server DLL:
   `/reforged/api`, `x-reforged-user-api-key`, `x-mysticparadox-gameserver-apikey`, `x-reforged-gateway`, new account
   keys `RRK_` (old `UUK_` keys still accepted and redacted). The parity fixtures follow (renames only).
2. Loopback only: the metagame and the deploy server refuse a non-loopback `BIND_HOST` at start.
3. The deploy server asks the metagame's `ServerStatus` at a loopback-only `METAGAME_URL`, not a fixed URL.
4. 1.4.4 game servers load `UndauntedInternalServer.dll` through the runtime loader: `winmm=n,b` under Wine, where DR
   forced `dxgi=n,b`; the internal server's README says how (RD9).
5. No game-derived files: DR's Dumper-7 dump leaves `native/internal-server` for `sdk-fixups.patch`, a checking
   `_build.bat` and ignore rules (and, if the maintainer agrees, the metagame's `store-images/`: see below).
6. The fuzz targets and checklists that go with 1 and 2 (`deploy_input`, the redaction check in the token target,
   `docs/security/metagame.md` and `deploy.md`).

Then, with `module-api` and not by carrying code: settings through the module's config section instead of
`METAGAME_`/`DEPLOY_` env prefixes, `GameModule` entry points instead of `run_with_context`, the 1.4.4 identity from
`GameVersion` instead of `protocol::game::Game`, and settings written by the host kit instead of DR's `deploy
generate-config` and `signing-keys`.

### `24b7624` against `main`, by folder

Every difference between DR's tree and `main` in the four folders and the files beside them that came with them.

| `main` path | Difference from DR `24b7624` | Entry |
| --- | --- | --- |
| `rust/services/metagame/src/` (`auth.rs`, `bodylog.rs`, `origin.rs`, `routes/*.rs`, `server.rs`, `health.rs`, comments elsewhere) | Rename dictionary: routes, the user, game-server and gateway headers | `98d0112` |
| `rust/services/metagame/src/accounts.rs`, `keyrotation.rs`, `offline_save.rs`, `redact.rs` | New keys `RRK_` from `protocol::account::ACCOUNT_KEY_PREFIX`; both prefixes redacted | `98d0112` |
| `rust/services/metagame/src/config.rs` | `BIND_HOST` off loopback refused; `ENV_PREFIX = "METAGAME_"` | `98d0112` |
| `rust/services/metagame/src/service.rs`, `main.rs`, `kitinvite.rs` | The `METAGAME_` prefix stripped at start (`service_kit::env::Env::strip_prefix`, new); invites as `Game::Legacy` | `98d0112` |
| `rust/services/metagame/src/serverstatus.rs` | DR's `ServerStatus` shape kept in a local struct, since this repository's `protocol` changed it | `98d0112` |
| `rust/services/metagame/src/db.rs`, `drizzle_frozen.rs`, `vendor.rs` | `include_str!` paths to `node/` instead of `../../../../legacy/` | `98d0112` |
| `rust/services/metagame/src/server.rs` | `run_with_context` and the status reports for `ServerController` | `fef7ca9` |
| `rust/services/metagame/Cargo.toml` | Description, `[[bin]]`, `tokio-tungstenite` from the workspace | `98d0112` |
| `rust/services/metagame/README.md` | New: the `METAGAME_` variables, ports and routes | `98d0112` |
| `rust/services/metagame/node/` | New: 62 files copied unchanged from DR's `legacy/UndauntedMetagame` (Drizzle SQL and snapshots, vendor JSON, `routes.json`, `test/data/store_art_skus.json`) and `legacy/UndauntedDeployServer` (`trials_pool.json`) | `98d0112` |
| `rust/services/metagame/tests/` | Renames in the tests and every parity fixture (no other change in the fixtures); `METAGAME_` in env files; empty stand-ins for `store-images/` (`fixtures/store-images.txt`, new); `ported.rs` skips without DR's Node tests; a loopback test in `m31/permissions.rs`; `node/` paths | `98d0112` |
| `rust/services/metagame/migrations/`, `diesel.toml` | None | — |
| `rust/services/deploy/src/settings.rs`, error texts across `src/` | Every variable read as `DEPLOY_<DR name>`; the game process still gets DR's names | `9b8279c` |
| `rust/services/deploy/src/service.rs` | `BIND_HOST` off loopback refused | `9b8279c` |
| `rust/services/deploy/src/service.rs` | `run_with_context`, errors returned instead of exit codes | `fef7ca9` |
| `rust/services/deploy/src/matchmaker.rs`, `remote.rs` | `ServerStatus` at `METAGAME_URL` (loopback only) with the renamed route and header | `9b8279c` |
| `rust/services/deploy/src/app.rs` | `x-reforged-gateway` refused next to `x-dauntless-gateway` | `9b8279c` |
| `rust/services/deploy/src/kitconfig.rs`, `main.rs`, `lib.rs`, `Cargo.toml` | `generate-config`, `signing-keys` and `rcgen` gone; no `.env` read from the working directory | `9b8279c` |
| `rust/services/deploy/src/kit.rs` | `winmm=n,b` instead of `dxgi=n,b` | `265bd93` |
| `rust/services/deploy/src/tables.rs`, `trials.rs` | `node/` paths; the Trials test skips when `native/internal-server` is missing | `9b8279c` |
| `rust/services/deploy/node/` | New: five vendor tables copied unchanged from DR's `legacy/UndauntedDeployServer` | `9b8279c` |
| `rust/services/deploy/tests/routing.rs` | `DEPLOY_` names; tests for the loopback rule and `METAGAME_URL` | `9b8279c` |
| `rust/services/deploy/README.md` | New | `9b8279c`, `265bd93` |
| `native/internal-server/SDK/`, `SDK.hpp`, `Assertions.inl`, `NameCollisions.inl`, `PropertyFixup.hpp`, `UnrealContainers.hpp`, `UtfN.hpp` | Not here (4,154 files): DR's Dumper-7 dump of the 1.4.4 client | `77118f6` |
| `native/internal-server/sdk-fixups.patch`, `_build.bat`, `README.md` | New | `77118f6`, `920a411` |
| `native/internal-server/dllmain.cpp` | `x-mysticparadox-gameserver-apikey` | `77118f6` |
| `native/client-mods/` | DR's `client-mods/`, moved; only `README.md` differs | `77118f6` |
| `native/.gitignore` | New rules for the 1.4.4 SDK and the two UE4SS DLLs | `77118f6` |
| `rust/fuzz/fuzz_targets/metagame_chat.rs`, `metagame_token.rs` | DR's `chat_frame.rs` and `auth_token.rs` renamed; the token target also checks the redaction | `98d0112` |
| `rust/fuzz/fuzz_targets/deploy_input.rs` | New | `9b8279c` |
| `docs/security/metagame.md`, `deploy.md` | New | `98d0112`, `9b8279c`, `265bd93` |

## This repository

### `c76cb09` Launcher: a player-only build without the Host tools

- **SHA:** `c76cb09081fcf0ebcafa0c56ff18e290f1bb979c`
- **Date:** 2026-10-24T08:41:34-06:00
- **Author:** Silent Spud
- **Files:** `rust/launcher/src-tauri/` (`Cargo.toml`, `src/controller/host_off.rs`, `loader.rs` and others), the
  generated types, `rust/launcher/test/ui.test.ts`, `legacy/ReforgedLauncher/` (two files)
- **Behavior:** a launcher built without the `host` feature has no Host page.
- **Revived:** not applicable. This repository's launcher; DR's launcher is dropped from legacy-revived (DD10).

### `f66da2a` Docker: a server managed by host.toml alone (the toml profile)

- **SHA:** `f66da2a26c03151f75abe3aaa7a8392aaa21b237`
- **Date:** 2026-10-24T08:48:26-06:00
- **Author:** Silent Spud
- **Files:** `.env.example`, `compose.yaml`, `docker/host-entrypoint.sh`, `docker/smoke-test.sh`,
  `docs/hosting-docker.md`
- **Behavior:** a `toml` Compose profile applies `host.toml` at every `up`.
- **Revived:** not applicable. This repository's Docker deployment; nothing DR-derived.

### `c1be7f7` Legacy seam: dauntless-revived:// invites and `legacy` features (RD5 reversed)

- **SHA:** `c1be7f7d25d21be9e2e33186e71375368001da30`
- **Date:** 2026-10-24T09:08:18-06:00
- **Author:** Silent Spud
- **Files:** `rust/crates/protocol/` (`game.rs` new, `invite.rs`, `lib.rs`, `Cargo.toml`, two tests),
  `rust/services/{content,gateway,host}/Cargo.toml`, `rust/services/host/src/invite.rs`,
  `rust/launcher/src-tauri/` (`Cargo.toml`, two files), `rust/services/metagame/` (a stub crate),
  `rust/Cargo.toml`, `rust/Cargo.lock`
- **Behavior:** with the `legacy` features, a `dauntless-revived://` invite is read as a 1.4.4 invite instead of
  refused.
- **Revived:** not applicable, by part:
  - `protocol` (`Game`, the `legacy` feature, invite reading) and the empty `legacy` features: this repository's
    seams. DF2's `GameVersion` and DF3's registry replace them; the invite stays the launcher's and the gateway's.
  - `rust/services/metagame` stub: replaced by `98d0112` the same day.

### `98d0112` Legacy metagame: Dauntless Revived's Rust metagame for 1.4.4 servers (RD5 reversed)

- **SHA:** `98d0112db59020ce81dd090dec5ecb2615dacc0d`
- **Date:** 2026-10-24T09:45:27-06:00
- **Author:** Silent Spud
- **Files:** `rust/services/metagame/` (`src/`, `tests/`, `migrations/`, `node/`, `README.md`, `Cargo.toml`,
  `diesel.toml`), `rust/crates/service-kit/src/env.rs`, `rust/fuzz/` (`Cargo.toml`, `Cargo.lock`,
  `fuzz_targets/metagame_chat.rs`, `metagame_token.rs`), `docs/security/metagame.md`, `.gitattributes`,
  `rust/Cargo.toml`, `rust/Cargo.lock`
- **Behavior:** DR's metagame at `24b7624` replaces the stub, with these changes for players and hosts: the launcher
  API is under `/reforged/api` with `x-reforged-user-api-key`, game servers send `x-mysticparadox-gameserver-apikey`,
  the gateway's secret is `x-reforged-gateway`; new account keys are `RRK_<48 hex>` (a DR database's `UUK_` keys
  still work and both are redacted from logs); a non-loopback `BIND_HOST` stops the start (DR warned); every setting
  is DR's name under `METAGAME_`; `metagame invite` writes `dauntless-revived://` invites.
- **Revived:** needed, Reforged-specific, by part:
  - Rename dictionary in `src/accounts.rs`, `app.rs`, `auth.rs`, `bodylog.rs`, `guild.rs`, `health.rs`, `origin.rs`,
    `realprogression.rs`, `routes/admin_accounts.rs`, `admin_history.rs`, `backup.rs`, `mod.rs`, `progression.rs`,
    `undaunted.rs`, `server.rs`, `serverstatus.rs` (doc comment), and the matching tests and parity fixtures (the
    fixtures change by renames only; `tests/parity.rs` maps the recorded Node routes to the new names): **needed,
    Reforged-specific.** This repository's gateway (`policy/legacy.rs`), its host kit and the internal-server DLL
    (`77118f6`) speak these names. See "For the maintainer", item 1, for the launcher's side.
  - `RRK_` keys (`accounts.rs`, `keyrotation.rs`, `offline_save.rs`) and the redaction of both prefixes (`redact.rs`):
    **needed, Reforged-specific** (rename dictionary; the launcher finds a server-made key in a damaged backup file
    only as `RRK_`, `protocol::account` `made_key`). How the prefix is named (`ACCOUNT_KEY_PREFIX` here) is glue.
  - `BIND_HOST` off loopback refused (`config.rs`, its test, `tests/m31/permissions.rs`): **needed,
    Reforged-specific** (AGENTS.md: the gateway is the only listener off loopback).
  - `METAGAME_` prefix (`config.rs` `ENV_PREFIX`, `service.rs`, `main.rs`, `kitinvite.rs`, the env files in the
    tests) and `Env::strip_prefix` in `service-kit`: **framework glue.** The module's settings come through its
    `module-api` config section.
  - `serverstatus.rs`'s local `ServerStatus` struct: **framework glue.** It exists only because this repository's
    `protocol::status::ServerStatus` is not DR's; on DR's own `protocol` it is not needed.
  - `kitinvite.rs` writing `Game::Legacy` invites: **framework glue**, on this repository's `protocol`.
  - `node/` (62 files, all unchanged copies of DR's) and the `include_str!` paths into it (`db.rs`,
    `drizzle_frozen.rs`, `vendor.rs`, tests): not applicable while legacy-revived keeps DR's
    `legacy/UndauntedMetagame` and `legacy/UndauntedDeployServer`, where DR's paths already point. Needed if RV1
    removes those folders ("For the maintainer", item 4).
  - Empty stand-ins for `store-images/` (`tests/common/mod.rs`, `tests/parity.rs`, `tests/fixtures/store-images.txt`)
    and `ported.rs`'s skip without DR's Node tests: not applicable while DR's folder is there; needed together with
    removing `store-images/` ("For the maintainer", item 3).
  - `Cargo.toml` (`[[bin]]`, the description, `tokio-tungstenite.workspace`), the workspace dependencies, the fuzz
    workspace's crate list: not applicable; legacy-revived's workspace is DR's until RV1 makes the module crate.
  - `fuzz_targets/metagame_chat.rs`, `metagame_token.rs`: DR's `chat_frame` and `auth_token` under new names; the
    renames are not applicable, **the redaction check in `metagame_token.rs` is needed, Reforged-specific** (it goes
    with `RRK_`).
  - `docs/security/metagame.md` and `README.md`: **needed, Reforged-specific**: they record the changes above.
  - `.gitattributes` (`rust/services/metagame/** text eol=lf`): not applicable. DR's `.gitattributes` is
    `* text=auto eol=lf`.

### `9b8279c` Legacy deploy: Dauntless Revived's deploy server for 1.4.4 game servers

- **SHA:** `9b8279c17f9b533d65ea284f76eab81979bd2e01`
- **Date:** 2026-10-24T09:35:59-06:00
- **Author:** Silent Spud
- **Files:** `rust/services/deploy/` (`src/`, `tests/`, `node/`, `README.md`, `Cargo.toml`), `rust/fuzz/`
  (`Cargo.toml`, `Cargo.lock`, `fuzz_targets/deploy_input.rs`), `docs/security/deploy.md`, `rust/Cargo.toml`,
  `rust/Cargo.lock`
- **Behavior:** DR's deploy server at `24b7624`, with these changes: every setting is `DEPLOY_<DR name>` (the game
  process still gets DR's names); a non-loopback `DEPLOY_BIND_HOST` stops the start; the Ramsgate balancing asks
  `/reforged/api/ServerStatus` with `x-reforged-user-api-key` at `DEPLOY_METAGAME_URL` (loopback HTTP only, default
  DR's `http://127.0.0.1:61000`); `x-reforged-gateway` is refused as a proxy header; `generate-config`,
  `signing-keys` and the `.env` fallback are gone.
- **Revived:** needed, Reforged-specific, by part:
  - Non-loopback `BIND_HOST` refused (`service.rs`, `tests/routing.rs`): **needed, Reforged-specific.**
  - `ServerStatus` through the rename dictionary (`matchmaker.rs`) and `x-reforged-gateway` in `PROXY_HEADERS`
    (`app.rs`): **needed, Reforged-specific**, with `98d0112`'s renames.
  - `METAGAME_URL` instead of the fixed `http://127.0.0.1:61000` (`remote.rs` `metagame_url`, its test):
    **needed, Reforged-specific**: DF3 assigns ports, so the metagame may not be on 61000. Its loopback-only check
    stays; where the value comes from is glue.
  - `DEPLOY_` prefix (`settings.rs`, its test, the variable names in every error text): **framework glue.**
  - `kitconfig.rs`, `generate-config`, `signing-keys` and `rcgen` removed, no `.env` from the working directory
    (`main.rs`, `lib.rs`, `Cargo.toml`): **framework glue.** The host kit writes the module's settings (DF4d); DR's
    `deploy/` kits used these commands ("For the maintainer", item 4).
  - `service.rs`'s doc comment dropping the launcher's offline `--service deploy`: not applicable; DR's launcher and
    offline mode are dropped (DD10).
  - `node/` (five unchanged copies of DR's vendor tables) and the `tables.rs` paths: not applicable, as for
    `98d0112`.
  - The Trials test skipping without `native/internal-server/TrialsRotation.h` (`trials.rs`): not applicable. DR's
    tree has the header, so the test should keep comparing.
  - `deploy_input` fuzz target, `docs/security/deploy.md`, `README.md`: **needed, Reforged-specific** (RX3: DR had
    no target for the deploy server).
  - `uuid` in the workspace dependencies: not applicable.

### `265bd93` Legacy deploy: run-game forces the runtime loader (winmm), not DR's dxgi proxy

- **SHA:** `265bd934b8167c2ec772e64cac5a571b824632c6`
- **Date:** 2026-10-24T09:38:11-06:00
- **Author:** Silent Spud
- **Files:** `rust/services/deploy/src/kit.rs`, `rust/services/deploy/README.md`, `docs/security/deploy.md`
- **Behavior:** `deploy run-game` sets `WINEDLLOVERRIDES=...;winmm=n,b` where DR set `dxgi=n,b`, so Wine and
  Proton load the runtime loader, which loads `UndauntedInternalServer.dll` (RD9).
- **Revived:** **needed, Reforged-specific.** The maintainer's choice of one pinned DLL loaded by the runtime loader
  instead of DR's unpublished `dxgi.dll` proxy (`native/internal-server/README.md`, "How it reaches the game").

### `076ae70` Legacy gateway and content: Dauntless Revived 1.4.4 servers behind `legacy` (RD5 reversed)

- **SHA:** `076ae70d09fc45e6e4e93547df0c62b7d11c37ac`
- **Date:** 2026-10-24T09:52:19-06:00
- **Author:** Silent Spud
- **Files:** `rust/services/gateway/` (`policy/legacy.rs` new, `config.rs`, `gateway.rs`, `policy.rs`, `server.rs`,
  `Cargo.toml`, `tests/legacy.rs`), `rust/services/content/` (`src/legacy/` new, `src/app/game_files.rs` new,
  `data/dauntless-1.4.4.json`, others), `rust/fuzz/` (`gateway_policy_legacy.rs` new, `content_range.rs`,
  `content_request.rs`), `docs/security/gateway.md`, `content.md`, `legacy/ReforgedGateway/README.md`,
  `legacy/ReforgedContent/README.md`
- **Behavior:** behind `legacy`, the gateway fronts DR's metagame with DR's public routes under the renamed prefix,
  and the content server serves the 1.4.4 manifest and files as DR's did.
- **Revived:** not applicable. DR's gateway and content server are dropped from legacy-revived; this repository's
  core plugins serve every game (DD10, DF4a, DF4b). The route table in `policy/legacy.rs` is the metagame's,
  though ("For the maintainer", item 5).

### `77118f6` Legacy native: DR's 1.4.4 server DLL source and the UE4SS client mods

- **SHA:** `77118f6475277fee0705649e6b880ac13fef09c9`
- **Date:** 2026-10-24T09:20:15-06:00
- **Author:** Silent Spud
- **Files:** `native/internal-server/` (DR's folder without the SDK dump, plus `sdk-fixups.patch`, `_build.bat`,
  `README.md`), `native/client-mods/` (DR's `client-mods/`), `native/.gitignore`
- **Behavior:** the 1.4.4 server DLL sends `x-mysticparadox-gameserver-apikey`. The SDK is each host's own dump with
  DR's two hand edits applied from `sdk-fixups.patch`; `_build.bat` refuses a missing, wrong-version or unpatched
  SDK. The client mods are DR's, byte for byte.
- **Revived:** needed, Reforged-specific, by part:
  - `dllmain.cpp`, the game-server header: **needed, Reforged-specific**, with `98d0112`'s renames.
  - The Dumper-7 dump removed (4,154 files under `native/internal-server`), `sdk-fixups.patch` (DR `3d227a3`'s two
    hand edits), `_build.bat`, the ignore rules in `native/.gitignore`, `README.md`: **needed, Reforged-specific**
    (AGENTS.md, "Nothing from the game is committed"), once the maintainer confirms it for legacy-revived ("For the
    maintainer", item 3). In legacy-revived the ignore rules go in `native/internal-server/.gitignore` or the root.
  - `native/client-mods`: not applicable. Every file but `README.md` is DR's `client-mods/` byte for byte; the move
    to `native/` is this repository's layout, and the new README describes this repository's launcher installing
    them. legacy-revived keeps DR's `client-mods/` and DR's README.

### `920a411` Legacy native: the 1.4.4 DLL reaches the game through the runtime loader

- **SHA:** `920a41192a9a1751144ef00570778ecd482bd00e`
- **Date:** 2026-10-24T09:34:27-06:00
- **Author:** Silent Spud
- **Files:** `native/internal-server/README.md`, `native/runtime-loader/src/lib.rs`
- **Behavior:** documents `mystic_loader.ini` naming `UndauntedInternalServer.dll`, the `winmm=n,b` override, the
  release steps and the one-time WINMM import check; the loader gets a test that a folder without
  `MysticParadox.dll` loads only the ini's list.
- **Revived:** needed, Reforged-specific, by part:
  - `native/internal-server/README.md`: **needed, Reforged-specific**, with `265bd93` (RD9).
  - `native/runtime-loader/src/lib.rs`: not applicable. The loader stays outside legacy-revived (DS6 does not move
    it); the 1.4.4 server depends on it ("For the maintainer", item 6).

### `6a436ab` Legacy host: reforged-host and Docker set up and run a Dauntless 1.4.4 server (RD5 reversed)

- **SHA:** `6a436abb5a6f9f65cac389af64dbc4efe9618773`
- **Date:** 2026-10-24T09:52:31-06:00
- **Author:** Silent Spud
- **Files:** `rust/services/host/src/` (`legacy/` new and 15 others), `rust/services/host/tests/`, `docker/`
  (`Dockerfile`, `healthcheck.sh`, `host-entrypoint.sh`, `legacy-entrypoint.sh` new), `compose.yaml`,
  `compose.legacy.yaml` (new), `.env.example`, `docs/hosting.md`, `hosting-linux.md`, `hosting-docker.md`
- **Behavior:** `setup --edition legacy` writes `metagame.env` and `deploy.env` (DR's names under `METAGAME_` and
  `DEPLOY_`), `start` runs the metagame on 61000 and the deploy server on 61001, `invite` and `doctor` know a 1.4.4
  server; Docker's `legacy` profile.
- **Revived:** not applicable. This repository's host kit and Docker; DF4d and DS6 move them onto the module. It is
  the other half of the env-prefix glue in `98d0112` and `9b8279c`, which goes with it.

### `a0b2e54` Legacy launcher: the game edition setting and DR's 1.4.4 client path

- **SHA:** `a0b2e54186d18eb8d8b7213d90eb8ee8e7390367`
- **Date:** 2026-10-24T10:00:11-06:00
- **Author:** Silent Spud
- **Files:** `rust/launcher/src-tauri/` (31 files under `src/`: `controller/legacy.rs`, `core/manifest.rs`,
  `manifest_check.rs`, `legacy_pins.rs`, `net/downloader.rs`, `autoregion.rs`, `hostapi/legacy.rs`,
  `platform/client_mods.rs`, `net/verify/install.rs` and others; `build.rs`, `tauri.legacy.conf.json`, `gen/`,
  `permissions/`), `rust/launcher/` page and tests, `rust/fuzz/` (`launcher_legacy.rs`),
  `docs/security/launcher-legacy.md`, `legacy/ReforgedLauncher/` (four files)
- **Behavior:** behind the launcher's `legacy` feature, a `gameEdition` setting and DR's launcher's 1.4.4 install,
  repair and PLAY path, ported from DR's launcher at `24b7624`.
- **Revived:** not applicable. This repository's launcher; DR's launcher is dropped from legacy-revived (DD10).
  It asks a 1.4.4 server by DR's names ("For the maintainer", item 1).

### `18ac587` Legacy launcher: the 1.4.4 DLL through the runtime loader, not DR's binaries

- **SHA:** `18ac587c7ceb63bf6893c2de986928646fdb566a`
- **Date:** 2026-10-24T10:10:27-06:00
- **Author:** Silent Spud
- **Files:** `rust/launcher/src-tauri/src/` (seven files: `core/legacy_pins.rs`, `platform/client_mods.rs` and
  others), `gen/strings.json`, `rust/fuzz/fuzz_targets/launcher_legacy.rs`, `docs/security/launcher-legacy.md`,
  `legacy/ReforgedLauncher/` (two files)
- **Behavior:** the launcher installs the maintainer's pinned `UndauntedInternalServer.dll` and the loader's
  `winmm.dll` with a `mystic_loader.ini`, and removes DR's `dxgi.dll`; the pins are empty until the first release.
- **Revived:** not applicable. This repository's launcher.

### `d6d6e9b` Invites: one dauntless:// scheme, with per-game aliases

- **SHA:** `d6d6e9bbab878d5b2e253c79265be43629ac552f`
- **Date:** 2026-10-24T17:53:09-06:00
- **Author:** Silent Spud
- **Files:** `rust/crates/protocol/` (two files), `rust/launcher/src-tauri/` (`linux/` and three files under `src/`),
  `rust/fuzz/fuzz_targets/protocol_input.rs`, `docs/players.md`, `docs/security/` (two files),
  `legacy/ReforgedLauncher/` (two files)
- **Behavior:** `dauntless://join` is read by every launcher build; the per-game schemes are aliases of it.
- **Revived:** not applicable. The invite is this repository's `protocol` and launcher.

### `7b34598` Invites: `version` names the game version, from the features built

- **SHA:** `7b345983a9c2ba8beb14f69e975f99173f2fa3eb`
- **Date:** 2026-10-24T18:20:01-06:00
- **Author:** Silent Spud
- **Files:** `rust/crates/protocol/` (three files), `rust/launcher/src-tauri/src/` (two files), the generated types,
  `rust/launcher/test/ui.test.ts`, `docs/security/` (one file), `legacy/ReforgedLauncher/` (four files)
- **Behavior:** the invite's parameter is `version=1.14.7|1.4.4`; a build refuses versions it does not play.
- **Revived:** not applicable. As `d6d6e9b`. The 1.4.4 version string is the module's `GameVersion` under DF2.

### `459a526` Invites: `reforged` and `revived` as names for the game versions

- **SHA:** `459a526e5ae16c69dd8386654c27aaede310d506`
- **Date:** 2026-10-24T19:00:17-06:00
- **Author:** Silent Spud
- **Files:** `rust/crates/protocol/` (three files), `docs/security/` (one file), `legacy/ReforgedLauncher/` (two
  files)
- **Behavior:** `version=revived` means 1.4.4, `version=reforged` 1.14.7.
- **Revived:** not applicable. As `d6d6e9b`; the alias `revived` is the module crate's (RV1).

### `825d67f` Allowlist: `settled` waits until the result is reported

- **SHA:** `825d67f09e106435916f2b23f3dcd35f00594ac0`
- **Date:** 2026-10-24T19:07:56-06:00
- **Author:** Silent Spud
- **Files:** `rust/services/gateway/src/allowlist/sync.rs`
- **Behavior:** none for players; a test race in the allowlist's audit is closed.
- **Revived:** not applicable. `allowlist/sync.rs` came from DR's gateway, which is dropped from legacy-revived
  (DD10). DR's own copy has the same race.

### `fef7ca9` App structure: unified ServerController, tracing layers, ratatui TUI and 12 edition binary crates

- **SHA:** `fef7ca9b7ee70eddc533972216b5e00314bdc893`
- **Date:** 2026-10-25T04:13:12-06:00
- **Author:** Silent Spud
- **Files (DR-derived part):** `rust/services/metagame/src/server.rs`, `rust/services/deploy/src/service.rs`. The
  rest (`crates/controller`, `service-kit`'s `context.rs` and `telemetry.rs`, `protocol`'s `edition.rs`, the twelve
  `rust/bins/` crates and the other services) is this repository's.
- **Behavior:** none for players. The metagame and the deploy server gain `run_with_context`, an in-process entry
  point taking a `ServiceContext`, and report starting, ready, stopping and stopped; the deploy server's start
  failures become errors instead of exit codes.
- **Revived:** **needed, framework glue.** legacy-revived needs an entry point the controller drives, but RV1 writes
  it as the module's `GameModule` with `ModuleContext` (DF2), and DF3 removes `ServiceContext`'s edition model.
  Nothing of this code is carried.

### `3a756cf` DaintyLass: a new root without the Dauntless Reforged Revival line

- **SHA:** `3a756cf8f6093af21fbc3acc85fae88acc954fef`
- **Date:** 2026-10-26T08:34:22-06:00
- **Author:** Silent Spud
- **Files (DR-adjacent part):** `.gitattributes` (the metagame's comment), `rust/launcher/src-tauri/build.rs` and
  `gen/dauntless-1.4.4.json` (new), the hosting and player pages
- **Behavior:** none in the four folders: they are the same before and after. DR's 1.4.4 manifest that the launcher
  compiles in moved from `legacy/ReforgedLauncher/src/main/revived/` to the launcher's `gen/`, still DR's
  `UndauntedContent/data/dauntless-1.4.4.json` byte for byte.
- **Revived:** not applicable. The launcher and this repository's docs.

## Duplicate copies

The same commits before the merge was rebased. Matched by `git patch-id --stable`, except the three marked
`range-diff`, whose only difference is a hunk of `rust/Cargo.toml`, `rust/fuzz/Cargo.lock` or `rust/fuzz/Cargo.toml`
from resolving the branch merges. The merge commits on those branches (`dd5cf03`, `b12de73`, `3370bbc`, `e64ad53`,
`9776f84`, `aebf3b8`) add nothing of their own.

| Entry | Copy | Branch in the bundle |
| --- | --- | --- |
| `c76cb09` | `c417bb6` | `feat/player-launcher-headless-server` |
| `f66da2a` | `548bac5` | same |
| `c1be7f7` | `197bfd3` | same |
| `98d0112` | `fb26683` | `feat/legacy-metagame` (deleted), on the branch above |
| `9b8279c` | `cecdeca` (`range-diff`) | `feat/legacy-deploy` (deleted), same |
| `265bd93` | `4c5b468` | same |
| `076ae70` | `9bbe127` (`range-diff`) | `feat/legacy-gateway-content` (deleted), same |
| `77118f6` | `c93f9c7` | `feat/legacy-internal-server-client-mods` (deleted), same |
| `920a411` | `85a7ea4` | same |
| `6a436ab` | `fe7ddbb` | `feat/legacy-host-docker` (deleted), same |
| `a0b2e54` | `316399e` (`range-diff`) | `feat/legacy-launcher` (deleted), same |
| `18ac587` | `a37867c` | same |
| `d6d6e9b` | `9614583` | `feat/player-launcher-headless-server` |
| `7b34598` | `867d60e` | same |
| `459a526` | `5bec66f` | same |
| `825d67f` | `43a8dda` | same |

## For the maintainer

Decisions this list cannot make, in the order RV1 meets them.

1. **Wire names.** DD10 brings "its Reforged changes" across, so the rename dictionary is marked needed. But this
   repository's own launcher asks a 1.4.4 server by DR's names: `REVIVED` in
   `rust/launcher/src-tauri/src/net/hostapi.rs:53-56` is `/undaunted/api` with `x-undaunted-user-api-key`, used for
   `GetUserInfo` before PLAY (`controller/legacy.rs:788`), `HuntRegion` (`net/autoregion.rs:66-67`,
   `net/hostapi/legacy.rs:37-39`) and `PlayingEnded` (`net/hostapi/legacy.rs:51-52`). The ported metagame answers
   only `/reforged/api`, and the gateway's legacy policy proxies `/undaunted/api/...` to it as an unknown path
   (`policy/legacy.rs:217-221`). So today a 1.4.4 server hosted from this repository would not answer the
   launcher's legacy path. Either the launcher's `REVIVED` flavour moves to the Reforged names (a DaintyLass change,
   with legacy-revived taking the renames as listed), or legacy-revived keeps DR's names and the renames in
   `98d0112`, `9b8279c` and `77118f6` become not applicable while the gateway's legacy policy goes back to DR's.
2. **Which crates the module builds on.** DD10 has legacy-revived take "module API changes, `store`, the change log"
   from the framework, which reads as this repository's `store` and `module-api` replacing DR's `crates/`. If DR's
   `protocol` and `service-kit` stay in legacy-revived instead, the `serverstatus.rs` struct, `Game::Legacy` and
   `ACCOUNT_KEY_PREFIX` glue in `98d0112` are not needed at all.
3. **Game-derived files in DR's tree.** DR commits its 1.4.4 Dumper-7 dump (`native/internal-server/SDK/` and six
   headers, 4,154 files, about 100 MB) and 57 store icons from the 1.4.4 client
   (`legacy/UndauntedMetagame/store-images/`). This repository keeps both out (AGENTS.md), and DS6 makes legacy-revived
   a submodule of it. Removing them from legacy-revived's tree is marked needed for the SDK (`77118f6`); the icons
   follow the same rule but need the test stand-ins from `98d0112`. Either way they stay in DR's history.
4. **DR's other folders.** DD10 drops DR's gateway, content, launcher, `discord-bot` and `friend-kit`. It does not
   say what happens to DR's Node services under `legacy/` and its `deploy/` kits. The metagame and the deploy server
   compile in files from `legacy/UndauntedMetagame` and `legacy/UndauntedDeployServer`: if those go, `98d0112`'s and
   `9b8279c`'s `node/` copies become needed. `deploy generate-config` and `signing-keys` exist for DR's `deploy/`
   kits; they go with the kits.
5. **Per-game routing.** `rust/services/gateway/src/policy/legacy.rs` (from `076ae70`) holds the metagame's public
   routes (Register, GetUserInfo, ServerStatus, HuntRegion, DeviceSignals, RegistrationStatus) and its `/internal`
   wall; `rust/services/content/src/legacy/` holds the 1.4.4 manifest. They stay here as not applicable, but if
   DF2's `Registrar` lets a module declare its routes, the table belongs to legacy-revived.
6. **Dependencies outside the module.** The 1.4.4 DLL reaches the game only through `native/runtime-loader`, which
   DS6 leaves in this repository; the one-time check that the 1.4.4 executable imports `WINMM.dll` has not been made
   on a PC with the game, and the launcher's and host kit's DLL pins are empty until the first release. The launcher
   bundles the client mods from `native/client-mods` (`rust/launcher/src-tauri/tauri.legacy.conf.json`) and the host
   kit names `native/internal-server` in its texts (`commands/setup.rs:341`, `legacy/mod.rs:555`): DS6's path sweep
   has to point them at the submodule, where DR's folder is `client-mods/` at the top.
7. **Ports.** The host kit runs the metagame on 61000 (where the launcher probes) and the deploy server on 61001; the
   metagame branch's draft had 61010. `METAGAME_URL` in the deploy server is there so DF3's port assignment can move
   it.
8. Nothing here was played against a 1.4.4 client or game server; the legacy smoke test was never run (`6a436ab`'s
   body), so "needed" says what the code expects, not that it works.
