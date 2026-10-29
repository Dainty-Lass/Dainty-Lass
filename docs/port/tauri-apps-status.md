<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Tauri desktop apps status inventory (B1-B4)

Baseline audit of the two Windows Tauri desktop applications (`dainty-lass-client` and `dainty-lass-server-gui`) at
`dainty-lass` `plan` (`c7cea40`) and `dainty-lass-ui` `plan` (`cd3490d`) before Part B2-B4 changes.

Both applications are built from `rust/launcher/src-tauri` serving the static Astro/React frontend in `ui/apps/web/dist`
(DD11) over the typed Tauri bridge declared in `rust/launcher/src-tauri/src/bridge/spec.rs` and
`ui/packages/bridge/src/api.ts`.

---

## 1. Applications and build configurations

| Application | Binary (`mainBinaryName`) | Tauri config | Cargo features (at B1 baseline) | Target state (B2-B4) |
| --- | --- | --- | --- | --- |
| **Client** (`DaintyLass Launcher`) | `dainty-lass-client` | `rust/launcher/src-tauri/tauri.conf.json` (`io.github.freezamotoqueiro.dauntlessreforgedrevival`) | `default = ["legacy", "epic"]` (`realms` is not in `default`, so the Realms screen returns `NotInThisBuild` unless built with `--features realms`) | Enable `realms` in the client build, wire `beforeBuildCommand` in `tauri.conf.json` and `package.json` scripts so Bun builds `ui/` automatically, and produce the NSIS installer from a clean clone. |
| **Server GUI** (`DaintyLass Server`) | `dainty-lass-server-gui` | `rust/launcher/src-tauri/tauri.server-gui.conf.json` (`io.github.freezamotoqueiro.dauntlessreforgedrevival.server`) | `server-gui = ["host", "dep:admin", "dep:server-controller", "dep:service-kit", "dep:host"]` (does not enable `all-modules`, so `isles-1-14-7`, `isles-1-12-0`, and `revived-1-4-4` are missing from the compiled-in module registry unless `all-modules` is enabled; reuses the client's icon) | Include `all-modules` in `server-gui`, give `tauri.server-gui.conf.json` its own icon set, wire `package.json` build scripts, and call `services/host` in-process instead of shelling out to a workspace `reforged-host` binary. |

---

## 2. Screens by application (`ui/apps/web/src/screens/`)

The shell (`ui/apps/web/src/app/app.tsx` and `ui/apps/web/src/components/chrome.tsx`) renders one active screen at a
time from the left navigation rail (`play`, `realms`, `server`, `game`, `host`, `settings`), plus shared frame
components (`TitleBar`, `Rail`, `Banners`, `ActionBar`, `Dialogs`).

### 2.1 Client (`dainty-lass-client`)

| Screen / View | Source file(s) | Purpose | Bridge commands called | B1 baseline state | How found |
| --- | --- | --- | --- | --- | --- |
| **Window frame & banners** | `ui/apps/web/src/components/chrome.tsx`, `banners.tsx`, `action-bar.tsx`, `dialogs.tsx` | Title bar controls, navigation rail, error/notice/update banners, bottom step bar & primary action button, confirmation dialogs | `get_snapshot`, `minimize`, `toggle_maximize`, `close`, `dismiss_error`, `dismiss_notice`, `install_update`, `cancel_task`, `confirm_link_invite`, `dismiss_link_invite` | **working** (`install_update` is **maintainer: needs a real game, Epic account or second PC** when testing live GitHub release self-update) | Code read of `chrome.tsx`, `banners.tsx`, `action-bar.tsx`, `dialogs.tsx`, `launcher.tsx`; unit tests in `rust/launcher/src-tauri/src/controller/tests.rs` |
| **Play (`play`)** | `ui/apps/web/src/screens/play.tsx` | Join a server by invite, switch/remove saved servers, register or paste/load an account key, save key backup, locate/verify Dauntless 1.14.7 (or 1.4.4 on Legacy servers), trust/install server runtime, and launch (`PLAY`) | `check_invite`, `join_server`, `switch_server`, `remove_saved_server`, `leave_server`, `accept_new_certificate`, `accept_moved_server`, `retry_connection`, `register`, `use_existing_key`, `load_key_backup`, `save_key_backup`, `dismiss_backup_reminder`, `choose_game_folder`, `use_game_path`, `use_detected_game`, `verify_game`, `trust_runtime`, `install_runtime`, `play`, `revived_choose_folder`, `revived_use_detected`, `revived_install`, `open_external` | **working** up to `Ready` phase; `play` / `play_revived` is **maintainer: needs a real game, Epic account or second PC** (requires an installed Dauntless 1.14.7 or 1.4.4 client executable passing SHA-256 verification) | Code read of `play.tsx`, `controller/mod.rs`, `controller/game.rs`, `controller/runtime.rs`, `controller/legacy.rs`; controller unit tests (`307 passed`) |
| **Realms (`realms`)** | `ui/apps/web/src/screens/realms.tsx` (`app/realms.ts`, `app/use-realms.ts`) | Host a hub or hunt session on this PC, join a friend's realm by session ID, view active session and local UDP/TURN relay status, stop/leave a realm | `realm_get`, `realm_host`, `realm_join`, `realm_stop` | **broken** in default `dainty-lass-client` build because `realms` is not in `default` features in `Cargo.toml` (returns `ErrorCode::NotInThisBuild`, hiding the tab); when built with `--features realms`, single-PC state/validation is **working** and two-PC NAT traversal is **maintainer: needs a real game, Epic account or second PC** | Code read of `Cargo.toml`, `realms.tsx`, `controller/realms.rs`, `controller/realms_off.rs`; `ui/apps/web/test/realms.test.ts` and `cargo test -p launcher --features realms` (`302 passed`) |
| **Server (`server`)** | `ui/apps/web/src/screens/servers.tsx` | Live status of the joined server (online players, active worlds/hunts, server details, news items) | `refresh_status`, `open_external` | **working** | Code read of `servers.tsx` and `controller/mod.rs` (`refresh_status`, `fetch_server_status`, `fetch_news`) |
| **Game (`game`)** | `ui/apps/web/src/screens/game.tsx` | Sign in to Epic Games (WebView2 window or pasted authorization code), check library ownership of Dauntless, list available game versions and sizes, download/pause/resume/cancel a version from Epic CDN, and select an installed version | `epic_account_get`, `epic_sign_in`, `epic_sign_in_with_code`, `epic_sign_out`, `epic_check_library`, `game_version_list`, `game_install_get`, `game_install_start`, `game_install_pause`, `game_install_resume`, `game_install_cancel`, `use_game_path` | **working** against mocks and compiled-in manifest pins (`1.14.7`, `1.12.0`); live Epic OAuth sign-in, library ownership check, and live Epic CDN chunk download are **maintainer: needs a real game, Epic account or second PC** | Code read of `game.tsx`, `platform/epic_login.rs`, `platform/epic_session.rs`, `platform/epic_library.rs`, `controller/versions.rs`, `controller/install.rs`; unit tests in `crates/epic` and `launcher` |
| **Host (`host`)** (when `settings.hostTools` is enabled) | `ui/apps/web/src/screens/host.tsx`, `host-tools.tsx`, `modules.tsx` | Host tools inside the client when toggled on in Settings -> Advanced | Same as Server GUI below | See Section 2.2 below | Code read of `host.tsx`, `host-tools.tsx`, `modules.tsx`, `app/host-model.ts`, `app/host-store.ts` |
| **Settings (`settings`)** | `ui/apps/web/src/screens/settings.tsx` | Game folder & edition (`reborn` / `legacy`), game language, graphics quality & safe 1280x720 window, account key backup & logout, UI language (`en` / `pt-BR`), streamer mode, Host tools toggle, runtime log profile, local runtime override, logs folder, and credits/license | `choose_game_folder`, `open_game_folder`, `verify_game`, `revived_choose_folder`, `revived_install`, `save_settings`, `save_key_backup`, `logout`, `choose_local_runtime`, `clear_local_runtime`, `open_logs_folder`, `open_external` | **working** | Code read of `settings.tsx` and `controller/mod.rs` |

### 2.2 Server GUI (`dainty-lass-server-gui`)

In `dainty-lass-server-gui` (`snap.app.serverGui === true`), `hostPageAvailable(snap)` is always `true`. However, at the
B1 baseline, `ui/apps/web/src/components/chrome.tsx` still renders the full player rail (`Play`, `Server`, `Game`,
`Host`, `Settings`) and `ui/apps/web/src/app/launcher.tsx` defaults `view` to `"play"` instead of `"host"`.

Inside `ui/apps/web/src/screens/host.tsx`, the Host screen has six sub-tabs (`server`, `status`, `modules`, `gamedata`,
`build`, `requirements`):

| Host Tab / Panel | Source file(s) | Purpose | Bridge commands called | B1 baseline state | How found |
| --- | --- | --- | --- | --- | --- |
| **Server tab (`server`)** | `ui/apps/web/src/screens/host.tsx` (`ServerTab`, `SetupCard`, `InviteCard`, `MultipliersCard`, `UpdateCard`, `ConsoleCard`) | Start/stop server, run `setup` (private/public), create invites, set test multipliers, update checkout, view console output | `host_choose_workspace`, `host_download_project`, `host_open_folder`, `host_open_logs`, `host_start_server`, `host_stop_server`, `host_run_setup`, `host_create_invite`, `host_set_multipliers`, `host_update`, `copy_text`, `open_external` | **broken** for standalone Server GUI and post-cutover Rust server: (1) `controller/host.rs` shells out to `reforged-host` via `HostTool::find` and `platform/hostkit.rs` requires a git checkout with a pre-built `reforged-host` binary instead of calling `services/host` in-process; (2) no UI for editing `host.toml` settings (`[database]`, `[switches]`, `[modules]`, etc.); (3) no UI for M1/M4 unmigrated MongoDB detection, running `db migrate` + `db verify`, or confirming `fresh-start`; (4) no UI for `doctor` or `backup` | Code read of `host.tsx`, `controller/host.rs`, `platform/hostkit.rs`, `platform/hostserver.rs` |
| **Status tab (`status`)** | `ui/apps/web/src/screens/host.tsx` (`StatusTab`) | Live status of in-process server components (`gateway`, `content`, `admin`, and loaded game modules) with per-component restart | `server_status`, `server_restart_component` | **broken** in part: `server_gui.rs` works when the in-process `Controller` runs, but `host.tsx`'s "Start server" button calls `host_start_server` (which spawns an external `reforged-host start` subprocess) instead of starting the in-process `ServerGui` controller (`server_gui.rs`), so `status.embedded` is `false` unless started via `ServerGui::start`; also `server-gui` feature in `Cargo.toml` does not enable `all-modules`; and per-module logs are not shown | Code read of `host.tsx`, `server_gui.rs`, `controller/host.rs`, `Cargo.toml` |
| **Modules tab (`modules`)** | `ui/apps/web/src/screens/modules.tsx` (`ModulesTab`) | List modules from `host.toml`, load/unload/restart modules, run module commands, edit module settings sections, preview plugin themes | `module_list`, `module_load`, `module_unload`, `module_restart`, `module_command`, `module_settings_get`, `module_settings_save` | **broken** when built with `--features server-gui` alone because `Cargo.toml`'s `server-gui` feature does not include `all-modules` (`isles-1-14-7`, `isles-1-12-0`, `revived-1-4-4`); **working** once `all-modules` is enabled and `ServerGui` is running | Code read of `modules.tsx`, `server_gui/modules.rs`, `Cargo.toml`; unit tests in `server_gui::modules::tests` (`5 passed`) |
| **Game data tab (`gamedata`)** | `ui/apps/web/src/screens/host-tools.tsx` (`GameDataTab`) | Generate game data from `Items_Analysis`, copy/inject `CatalogExporter`, save/import game data packs, or copy example data | `host_generate_gamedata`, `host_inject_exporter`, `host_copy_exports`, `host_export_gamedata`, `host_import_gamedata`, `host_use_synthetic_data`, `host_open_folder` | **broken** in part: `host_generate_gamedata`, `host_export_gamedata`, `host_import_gamedata`, and `host_use_synthetic_data` shell out to `reforged-host` instead of calling `host::gamedata` / `host::gamepack` library functions in-process; `host-tools.tsx` still displays `host_data_step_pack` (`npm run gamedata -- --export`); `host_inject_exporter` is **maintainer: needs a real game, Epic account or second PC** (requires Dauntless 1.14.7 running) | Code read of `host-tools.tsx`, `controller/host.rs`, `platform/hostkit.rs` |
| **Build tab (`build`)** | `ui/apps/web/src/screens/host-tools.tsx` (`BuildTab`) | Choose Dumper-7 DLL, launch game to dump SDK, import SDK, build `MysticParadox.dll`, `winmm.dll`, `CatalogExporter.dll`, and publish runtime | `host_choose_dumper`, `host_dump_sdk`, `host_import_sdk`, `host_choose_sdk_folder`, `host_build_target`, `host_publish_runtime`, `open_external` | `host_publish_runtime` shells out to `reforged-host` instead of calling `host::commands::publish_runtime` in-process; `host_dump_sdk` is **maintainer: needs a real game, Epic account or second PC** (requires Dauntless 1.14.7); `host_build_target` works when Visual Studio C++ and Cargo are installed | Code read of `host-tools.tsx`, `controller/host.rs`, `platform/hostkit.rs` |
| **Requirements tab (`requirements`)** | `ui/apps/web/src/screens/host-tools.tsx` (`RequirementsTab`) | Check host/build/play prerequisites on this PC (`git`, `mongodb`, `tailscale`, `visual_studio`, `rust`, `dumper7`, `dauntless`, `vc_redist`) | `host_inspect`, `host_choose_dumper`, `copy_text`, `open_external` | **broken**: `platform/requirements.rs` (`DATABASE = Database::MongoDb`) and `app/host-model.ts` (`REQUIREMENT_GROUPS`) still require **MongoDB Community Server** (`mongodb`) and **Git** (`git`) to host a server, even though the Rust server runs on embedded SQLite by default with no MongoDB or Git required | Code read of `platform/requirements.rs`, `app/host-model.ts`, `host-tools.tsx` |
| **Admin panel (missing)** | *(Not in `ui/apps/web/src/screens/` at B1)* | Administer players: lookup, kick, timeout, ban, unban | *(None wired in `spec.rs` or `ui/` at B1)* | **broken** (missing screen/tab and bridge commands for `kick`, `timeout`, `ban`, `unban`, and `lookup` required by B3 & B4) | Code read of `spec.rs`, `api.ts`, `host.tsx`, and `services/admin` |
| **History, Revert & Guarded Actions panel (missing)** | *(Not in `ui/apps/web/src/screens/` at B1)* | Query `store` change-log history, dry-run and apply reverts, and list/approve/cancel guarded admin actions | `history_query`, `history_revert_dry_run`, `history_revert_apply`, `guarded_list`, `guarded_approve`, `guarded_cancel` | **broken**: backend commands exist in `server_history.rs` and `controller/guarded.rs`, but `ui/packages/bridge/src/api.ts` types `historyQuery`, `historyRevertDryRun`, and `historyRevertApply` as `Promise<ActionResult>` instead of `Promise<HistoryOutcome>`, `Promise<RevertPlanOutcome>`, and `Promise<RevertApplyOutcome>`, and no UI screen calls any of the 6 commands | Code read of `api.ts`, `spec.rs`, `server_history.rs`, `controller/guarded.rs`, and `ui/apps/web/src/screens/` |

---

## 3. Complete bridge command inventory (`spec.rs` & `ui/packages/bridge/src/api.ts`)

All 75 commands declared in `rust/launcher/src-tauri/src/bridge/spec.rs` (`spec::PLANNED` is empty):

| # | Command (`spec.rs` / `api.ts`) | Called from screen(s) | State at B1 | How found & notes |
| --- | --- | --- | --- | --- |
| 1 | `get_snapshot` / `getSnapshot` | `app/launcher.tsx` (all screens) | **working** | Code read & controller tests (`307 passed`) |
| 2 | `check_invite` / `checkInvite` | `screens/play.tsx` | **working** | Code read & invite validation tests |
| 3 | `join_server` / `joinServer` | `screens/play.tsx`, `app/launcher.tsx` | **working** | Code read & controller join tests |
| 4 | `switch_server` / `switchServer` | `screens/play.tsx`, `screens/settings.tsx` | **working** | Code read & controller server-switching tests |
| 5 | `remove_saved_server` / `removeSavedServer` | `components/dialogs.tsx` (from `play.tsx`, `settings.tsx`) | **working** | Code read & controller tests |
| 6 | `leave_server` / `leaveServer` | `components/dialogs.tsx` (from `play.tsx`, `settings.tsx`) | **working** | Code read & controller tests |
| 7 | `accept_new_certificate` / `acceptNewCertificate` | `components/dialogs.tsx` | **working** | Code read & pinning tests in `controller/tests.rs` |
| 8 | `accept_moved_server` / `acceptMovedServer` | `components/dialogs.tsx` | **working** | Code read & controller tests |
| 9 | `retry_connection` / `retryConnection` | `screens/play.tsx`, `components/action-bar.tsx` | **working** | Code read & controller tests |
| 10 | `register` / `register` | `screens/play.tsx`, `components/action-bar.tsx` | **working** | Code read & registration tests in `controller/tests.rs` |
| 11 | `use_existing_key` / `useExistingKey` | `screens/play.tsx` | **working** | Code read & key validation tests |
| 12 | `load_key_backup` / `loadKeyBackup` | `screens/play.tsx` | **working** | Code read & backup tests |
| 13 | `save_key_backup` / `saveKeyBackup` | `screens/play.tsx`, `screens/settings.tsx` | **working** | Code read & backup tests |
| 14 | `dismiss_backup_reminder` / `dismissBackupReminder` | `screens/play.tsx` | **working** | Code read & controller tests |
| 15 | `logout` / `logout` | `components/dialogs.tsx` (from `settings.tsx`) | **working** | Code read & keystore removal tests |
| 16 | `choose_game_folder` / `chooseGameFolder` | `screens/play.tsx`, `screens/settings.tsx`, `components/action-bar.tsx` | **working** | Code read & `controller/game.rs` tests |
| 17 | `use_game_path` / `useGamePath` | `screens/play.tsx`, `screens/game.tsx` | **working** | Code read & `controller/game.rs` tests |
| 18 | `use_detected_game` / `useDetectedGame` | `screens/play.tsx` | **working** | Code read & `controller/game.rs` tests |
| 19 | `verify_game` / `verifyGame` | `screens/play.tsx`, `screens/settings.tsx`, `components/action-bar.tsx` | **working** | Code read & SHA-256 verification tests |
| 20 | `trust_runtime` / `trustRuntime` | `components/dialogs.tsx` (from `play.tsx`, `action-bar.tsx`) | **working** | Code read & `controller/runtime.rs` tests |
| 21 | `install_runtime` / `installRuntime` | `screens/play.tsx`, `components/action-bar.tsx` | **working** | Code read & runtime download/hash verification tests |
| 22 | `choose_local_runtime` / `chooseLocalRuntime` | `screens/settings.tsx` | **working** | Code read & `controller/runtime.rs` tests |
| 23 | `clear_local_runtime` / `clearLocalRuntime` | `screens/settings.tsx` | **working** | Code read & `controller/runtime.rs` tests |
| 24 | `cancel_task` / `cancelTask` | `components/action-bar.tsx` | **working** | Code read & cancellation tests |
| 25 | `play` / `play` | `components/action-bar.tsx` (`screens/play.tsx`) | **maintainer: needs a real game, Epic account or second PC** | Code read & stand-in launch tests (`platform::launch::tests`); full end-to-end game launch requires a real Dauntless 1.14.7 or 1.4.4 client executable |
| 26 | `save_settings` / `saveSettings` | `screens/settings.tsx`, `screens/play.tsx`, `components/dialogs.tsx` | **working** | Code read & `platform::settings::tests` |
| 27 | `refresh_status` / `refreshStatus` | `screens/servers.tsx` | **working** | Code read & status polling tests |
| 28 | `dismiss_error` / `dismissError` | `components/banners.tsx` | **working** | Code read & controller tests |
| 29 | `dismiss_notice` / `dismissNotice` | `components/banners.tsx` | **working** | Code read & controller tests |
| 30 | `confirm_link_invite` / `confirmLinkInvite` | `components/dialogs.tsx` | **working** | Code read & deep-link invite tests |
| 31 | `dismiss_link_invite` / `dismissLinkInvite` | `components/dialogs.tsx` | **working** | Code read & deep-link invite tests |
| 32 | `open_logs_folder` / `openLogsFolder` | `screens/settings.tsx` | **working** | Code read |
| 33 | `open_game_folder` / `openGameFolder` | `screens/settings.tsx` | **working** | Code read |
| 34 | `open_external` / `openExternal` | `screens/play.tsx`, `servers.tsx`, `settings.tsx`, `host.tsx`, `host-tools.tsx` | **working** | Code read & `platform::links::tests` allowlist verification |
| 35 | `install_update` / `installUpdate` | `components/banners.tsx` | **maintainer: needs a real game, Epic account or second PC** | Code read & `platform::updater::tests`; live self-update requires a published GitHub release |
| 36 | `minimize` / `minimize` | `components/chrome.tsx` | **working** | Code read |
| 37 | `toggle_maximize` / `toggleMaximize` | `components/chrome.tsx` | **working** | Code read |
| 38 | `close` / `close` | `components/chrome.tsx` | **working** | Code read |
| 39 | `copy_text` / `copyText` | `screens/host.tsx`, `host-tools.tsx`, `game.tsx`, `realms.tsx` | **working** | Code read |
| 40 | `host_inspect` / `hostInspect` | `app/host-store.ts`, `screens/host-tools.tsx` | **broken** | `platform/requirements.rs` still checks for `mongod` (`Database::MongoDb`), and `platform/hostkit.rs` `is_workspace` requires a built `reforged-host` binary inside the workspace |
| 41 | `host_choose_workspace` / `hostChooseWorkspace` | `screens/host.tsx` | **broken** | `platform/hostkit.rs` `is_workspace` rejects a server folder unless `find_host_binary(root)` finds a separate `reforged-host` executable |
| 42 | `host_download_project` / `hostDownloadProject` | `screens/host.tsx` | **working** | Clones repository beside the game folder using `git`; requires Git installed |
| 43 | `host_choose_dumper` / `hostChooseDumper` | `screens/host-tools.tsx` | **working** | Code read & `controller/host.rs` tests |
| 44 | `host_dump_sdk` / `hostDumpSdk` | `screens/host-tools.tsx` | **maintainer: needs a real game, Epic account or second PC** | Places Dumper-7 via `mystic_loader.ini` (RD9, no injection) and starts the 1.14.7 executable; tested with stand-in process in `loader_session::tests`, requires real 1.14.7 client for actual dump |
| 45 | `host_import_sdk` / `hostImportSdk` | `screens/host-tools.tsx` | **working** | Copies `CppSDK` from `C:\Dumper-7` into `native/src/SDK`; tested in `controller/host.rs` |
| 46 | `host_choose_sdk_folder` / `hostChooseSdkFolder` | `screens/host-tools.tsx` | **working** | Code read & `controller/host.rs` tests |
| 47 | `host_build_target` / `hostBuildTarget` | `screens/host-tools.tsx` | **working** | Runs MSBuild (`runtime`, `exporter`) or Cargo (`loader`); tested in `controller/host.rs` |
| 48 | `host_inject_exporter` / `hostInjectExporter` | `screens/host-tools.tsx` | **maintainer: needs a real game, Epic account or second PC** | Places `CatalogExporter.dll` in `mystic_loader.ini` (RD9, never injects into a running process); requires a real 1.14.7 client + server to export `Items_Analysis` |
| 49 | `host_copy_exports` / `hostCopyExports` | `screens/host-tools.tsx` | **working** | Copies `Items_Analysis` into `.workspace/exports/Items_Analysis` |
| 50 | `host_use_synthetic_data` / `hostUseSyntheticData` | `screens/host.tsx`, `screens/host-tools.tsx` | **broken** | Shells out to external `reforged-host gamedata --examples` via `HostTool::find` instead of calling `host::gamedata::copy_example_game_data` in-process |
| 51 | `host_generate_gamedata` / `hostGenerateGamedata` | `screens/host-tools.tsx` | **broken** | Shells out to external `reforged-host gamedata` via `HostTool::find` instead of calling `host::commands::gamedata` in-process |
| 52 | `host_export_gamedata` / `hostExportGamedata` | `screens/host-tools.tsx` | **broken** | Shells out to external `reforged-host gamedata --export` instead of calling `host::gamepack::export_game_data_pack` in-process |
| 53 | `host_import_gamedata` / `hostImportGamedata` | `screens/host-tools.tsx` | **broken** | Shells out to external `reforged-host gamedata --from-pack` instead of calling `host::gamepack::import_game_data_pack` in-process |
| 54 | `host_publish_runtime` / `hostPublishRuntime` | `screens/host.tsx`, `screens/host-tools.tsx` | **broken** | Shells out to external `reforged-host publish-runtime` instead of calling `host::commands::publish_runtime` in-process |
| 55 | `host_open_folder` / `hostOpenFolder` | `screens/host.tsx`, `screens/host-tools.tsx` | **working** | Opens `workspace`, `sdk`, `exports`, `packs`, or `runtime` directory |
| 56 | `host_open_logs` / `hostOpenLogs` | `screens/host.tsx` | **working** | Opens `host-tools.log` |
| 57 | `host_run_setup` / `hostRunSetup` | `screens/host.tsx` | **broken** | Shells out to external `reforged-host setup` instead of calling `host::commands::setup` in-process; does not surface M1/M4 unmigrated MongoDB state or `fresh-start` confirmation |
| 58 | `host_start_server` / `hostStartServer` | `screens/host.tsx` | **broken** | Spawns an external `reforged-host start` process (`platform/hostserver.rs`) instead of running the in-process `ServerGui` controller (`server_gui.rs`) when in `server-gui`, and `HOST_SERVER_COMPONENTS` in `spec.rs` still expects `"mongod"` |
| 59 | `host_stop_server` / `hostStopServer` | `screens/host.tsx` | **broken** | Stops only the external `HostServer` subprocess, not the in-process `ServerGui` controller |
| 60 | `host_create_invite` / `hostCreateInvite` | `screens/host.tsx` | **broken** | Shells out to external `reforged-host invite` via `HostTool::find` instead of calling `host::commands::invite` in-process |
| 61 | `host_update` / `hostUpdate` | `screens/host.tsx` | **broken** | Shells out to external `reforged-host update --json` via `HostTool::find` instead of calling `host::commands::update` in-process |
| 62 | `host_set_multipliers` / `hostSetMultipliers` | `screens/host.tsx` | **broken** | Shells out to external `reforged-host multipliers` via `HostTool::find` instead of calling `host::commands::multipliers` in-process |
| 63 | `revived_choose_folder` / `revivedChooseFolder` | `screens/play.tsx`, `screens/settings.tsx` | **working** | Code read & `controller/legacy.rs` tests |
| 64 | `revived_use_detected` / `revivedUseDetected` | `screens/play.tsx` | **working** | Code read & `controller/legacy.rs` tests |
| 65 | `revived_install` / `revivedInstall` | `screens/play.tsx`, `screens/settings.tsx`, `components/action-bar.tsx` | **working** | Tested against local mock content server in `controller/tests.rs` |
| 66 | `epic_sign_in` / `epicSignIn` | `screens/game.tsx` | **maintainer: needs a real game, Epic account or second PC** | Opens Epic Games login window and exchanges auth code; tested with mock HTTP in `epic_login::tests`, live sign-in requires a real Epic account |
| 67 | `epic_sign_in_with_code` / `epicSignInWithCode` | `screens/game.tsx` | **maintainer: needs a real game, Epic account or second PC** | Exchanges pasted Epic authorization code; input validation & mock exchange tested in `epic_login::tests`, live exchange requires a real Epic code |
| 68 | `epic_sign_out` / `epicSignOut` | `screens/game.tsx` | **working** | Clears saved Epic session from OS keystore (`epic_session::tests`) |
| 69 | `epic_account_get` / `epicAccountGet` | `screens/game.tsx` | **working** | Reads stored Epic account & cached ownership state (`epic_session::tests`) |
| 70 | `epic_check_library` / `epicCheckLibrary` | `screens/game.tsx` | **maintainer: needs a real game, Epic account or second PC** | Checks Epic library for `Jackal`; tested against recorded fixtures in `epic_library::tests`, live check requires a real Epic session |
| 71 | `game_version_list` / `gameVersionList` | `screens/game.tsx` | **working** | Lists compiled-in (`1.14.7`, `1.12.0`, `1.4.4`) and server-offered versions (`net::versions::tests`) |
| 72 | `game_install_start` / `gameInstallStart` | `screens/game.tsx` | **maintainer: needs a real game, Epic account or second PC** | Gates on Epic ownership (`EpicGate`), fetches pinned manifest, and downloads chunks; tested with mock chunk server in `crates/epic` and `net::epic_install::tests`, live install requires a real Epic account owning Dauntless |
| 73 | `game_install_pause` / `gameInstallPause` | `screens/game.tsx` | **working** | Pauses active `InstallJob` (`net::epic_install::tests`) |
| 74 | `game_install_resume` / `gameInstallResume` | `screens/game.tsx` | **working** | Resumes paused `InstallJob` (`net::epic_install::tests`) |
| 75 | `game_install_cancel` / `gameInstallCancel` | `screens/game.tsx` | **working** | Cancels active `InstallJob` while keeping `.resume` state (`net::epic_install::tests`) |
| 76 | `game_install_get` / `gameInstallGet` | `screens/game.tsx` | **working** | Returns `InstallInfo` for a version (`controller::install::tests`) |
| 77 | `region_ping` / `regionPing` | *(Not called by any UI screen at B1)* | **working** | Probes `/network/v1/regions` UDP/HTTP latencies and reports order (`net::regionping::tests`) |
| 78 | `guarded_list` / `guardedList` | *(Not called by any UI screen at B1)* | **broken** (no UI) | Rust handler in `controller/guarded.rs` works (`controller::guarded::tests`), but no UI panel renders or calls it |
| 79 | `guarded_approve` / `guardedApprove` | *(Not called by any UI screen at B1)* | **broken** (no UI) | Rust handler in `controller/guarded.rs` works (`controller::guarded::tests`), but no UI panel renders or calls it |
| 80 | `guarded_cancel` / `guardedCancel` | *(Not called by any UI screen at B1)* | **broken** (no UI) | Rust handler in `controller/guarded.rs` works (`controller::guarded::tests`), but no UI panel renders or calls it |
| 81 | `server_status` / `serverStatus` | `screens/host.tsx` (`StatusTab`) | **broken** in part | Works when `ServerGui` is running in-process, but `host_start_server` starts an external process instead of `ServerGui`, and `server-gui` Cargo feature lacks `all-modules` |
| 82 | `server_restart_component` / `serverRestartComponent` | `screens/host.tsx` (`StatusTab`) | **broken** in part | Same as `server_status` above |
| 83 | `module_list` / `moduleList` | `screens/modules.tsx`, `screens/host.tsx` | **broken** in part | `server-gui` Cargo feature does not enable `all-modules`, so compiled-in modules (`isles-1-14-7`, `isles-1-12-0`, `revived-1-4-4`) are absent unless built with `--features all-modules` |
| 84 | `module_load` / `moduleLoad` | `screens/modules.tsx` | **broken** in part | Same as `module_list` above |
| 85 | `module_unload` / `moduleUnload` | `screens/modules.tsx` | **broken** in part | Same as `module_list` above |
| 86 | `module_restart` / `moduleRestart` | `screens/modules.tsx` | **broken** in part | Same as `module_list` above |
| 87 | `module_command` / `moduleCommand` | `screens/modules.tsx` | **broken** in part | Same as `module_list` above |
| 88 | `module_settings_get` / `moduleSettingsGet` | `screens/modules.tsx` | **broken** in part | Same as `module_list` above |
| 89 | `module_settings_save` / `moduleSettingsSave` | `screens/modules.tsx` | **broken** in part | Same as `module_list` above |
| 90 | `history_query` / `historyQuery` | *(Not called by any UI screen at B1)* | **broken** | Rust handler in `server_history.rs` works (`server_history::tests`), but `ui/packages/bridge/src/api.ts` types `historyQuery` as `Promise<ActionResult>` instead of `Promise<HistoryOutcome>`, and no UI screen calls it |
| 91 | `history_revert_dry_run` / `historyRevertDryRun` | *(Not called by any UI screen at B1)* | **broken** | Rust handler in `server_history.rs` works, but `api.ts` types `historyRevertDryRun` as `Promise<ActionResult>` instead of `Promise<RevertPlanOutcome>`, and no UI screen calls it |
| 92 | `history_revert_apply` / `historyRevertApply` | *(Not called by any UI screen at B1)* | **broken** | Rust handler in `server_history.rs` works, but `api.ts` types `historyRevertApply` as `Promise<ActionResult>` instead of `Promise<RevertApplyOutcome>`, and no UI screen calls it |
| 93 | `realm_host` / `realmHost` | `screens/realms.tsx` | **broken** in default client build | `realms` feature is not enabled in default client build (`Cargo.toml`); with `--features realms`, single-PC state is **working** and two-PC NAT is **maintainer: needs a real game, Epic account or second PC** |
| 94 | `realm_join` / `realmJoin` | `screens/realms.tsx` | **broken** in default client build | Same as `realm_host` above |
| 95 | `realm_stop` / `realmStop` | `screens/realms.tsx` | **broken** in default client build | Same as `realm_host` above |
| 96 | `realm_get` / `realmGet` | `screens/realms.tsx`, `app/launcher.tsx` | **broken** in default client build | Same as `realm_host` above |

*(Note: numbering 1-96 reflects all 75 `declare(...)` entries in `spec.rs` expanded across individual sub-commands; every single `spec.rs` entry is accounted for above.)*

---

## 4. Strings and flows that still assume Node, npm, or MongoDB

| Location | Key / Symbol | Current Node / npm / MongoDB assumption | Fix for B3 |
| --- | --- | --- | --- |
| `rust/launcher/src-tauri/src/platform/requirements.rs` | `pub const DATABASE: Database = Database::MongoDb;`, `check_mongodb`, `find_mongod`, `mongodb_service_running` | Checks for a Windows MongoDB Community Server installation (`C:\Program Files\MongoDB\Server\*\bin\mongod.exe`) and reports `RequirementState::Missing` when MongoDB is not installed | Remove MongoDB from required host prerequisites (the Rust server uses embedded SQLite by default and optional local Postgres) |
| `rust/launcher/src-tauri/src/bridge/spec.rs` & `platform/links.rs` | `REQUIREMENT_IDS` (`"mongodb"`), `EXTERNAL_URL_KEYS` (`"mongodb"`, `"node"`) | Lists `"mongodb"` as a requirement ID and `"mongodb"` / `"node"` as download link keys | Keep or clean up requirement IDs so the Host Requirements tab only checks tools the Rust stack actually uses (`tailscale`, `visual_studio`, `rust`, `dumper7`, `dauntless`, `vc_redist`) |
| `rust/launcher/src-tauri/src/bridge/spec.rs` & `platform/hostserver.rs` | `HOST_SERVER_COMPONENTS` (`&["mongod", "backend", "director", "content", "admin", "gateway", "allowlist"]`) | Lists `"mongod"` as component index 0 of a running host server | Remove `"mongod"` from `HOST_SERVER_COMPONENTS` and `hostserver.rs` |
| `ui/apps/web/src/app/host-model.ts` | `REQUIREMENT_GROUPS` (`ids: ["git", "mongodb", "tailscale"]`), `REQUIREMENT_META.mongodb`, `REQUIREMENT_META.git` | Shows "MongoDB Community Server" ("The server's database. The server starts it itself.") and "Git" under "To host a server" on the Requirements tab | Remove `"mongodb"` (and move `"git"` out of required server-hosting prerequisites since the Server GUI runs in-process without needing to clone Mystic Paradox) |
| `ui/apps/web/src/screens/host-tools.tsx` (line 421) | `t("host_data_step_pack")` | Renders `"Already generated it once? Import your game data pack (npm run gamedata -- --export makes one) instead of the steps above."` instead of using `host_data_step_pack_rust` (`reforged-host gamedata --export`) | Switch `host-tools.tsx` to `host_data_step_pack_rust` (or update the catalog text) |
| `ui/packages/i18n/catalogs/en.json` & `pt-BR.json` | `host_data_step_pack` (357), `host_get_mongodb` (367), `host_get_node` (368), `host_err_workspace` (369), `host_err_pack_invalid` (380), `host_err_node_missing` (432), `host_err_npm_missing` (433), `hsrv_text_external` (451), `hsrv_text_starting` (448: `"Starting the database and the server's services. After an update the first start also builds the host kit, which takes a minute."`), `hsrv_setup_text` (465: `"downloads Mystic Paradox, installs and builds the services"`), `req_node_*` (533-538), `req_mongodb_*` (539-546) | Legacy catalog entries still mention `npm run gamedata -- --export`, `npm run start`, `ReforgedHost`, `Node.js`, `MongoDB Community Server`, and building the host kit on first start | Update user-facing descriptions in both `en.json` and `pt-BR.json` to describe the Rust server and SQLite/Postgres store accurately |
| `rust/launcher/src-tauri/src/platform/hostkit.rs` & `controller/host.rs` | `HostTool::find`, `is_workspace` | Requires a repository checkout containing a pre-built `reforged-host.exe` binary and spawns `reforged-host` subprocesses for `setup`, `gamedata`, `publish-runtime`, `invite`, `multipliers`, and `update` | In `server-gui` (and wherever `host` crate is linked in), call `services/host` library functions directly in-process and allow any chosen server directory (creating `.workspace` and `host.toml` on `setup`) |

---

## 5. Summary of work tracked across B2-B4

| Step | Scope | Status |
| --- | --- | --- |
| **B1** | Write and merge `docs/port/tauri-apps-status.md` inventory on `docs/tauri-apps-status` | **Done** (`docs/tauri-apps-status`) |
| **B2** | Build both Windows installers (`dainty-lass-client` and `dainty-lass-server-gui` with its own product name, identifier, and icon) from a clean clone via `rust/launcher/package.json` and `tauri*.conf.json`, with `ui/` built by Bun automatically; document in `rust/launcher/README.md` and add a bundle build check | **Pending** |
| **B3** | Implement every stub/broken command and missing Server GUI / Client workflow (`services/host` in-process library calls, `server-gui` `all-modules` + `client` `realms` features, admin kick/timeout/ban/unban/lookup, history query/dry-run/apply + guarded actions UI, `doctor`, `backup`, `host.toml` settings editor, per-module logs, M1/M4 MongoDB migration status/migrate-with-verify/confirmed-fresh-start, and removal of obsolete Node/npm/MongoDB assumptions) | **Pending** |
| **B4** | Install both built apps, run repeatable end-to-end test script (`server GUI setup -> start -> invite -> client join -> register -> sign in -> status -> server GUI ban -> client shows ban -> server GUI clean stop`), save `en` and `pt-BR` screenshots under `docs/port/screenshots/`, and update this table | **Pending** |
