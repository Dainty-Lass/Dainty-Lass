<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Tauri desktop apps status inventory (B1-B4)

Complete inventory and verification record of the two Windows Tauri desktop applications (`dainty-lass-client` and
`dainty-lass-manager`) across Part B1 (baseline audit at `dainty-lass` `c7cea40` and
`dainty-lass-ui` `cd3490d`), Part B2 (clean-clone NSIS builds), Part B3 (in-process Server GUI + Client feature and UI
completion), Part B4 (installed Windows desktop end-to-end test and `en` / `pt-BR` screenshot capture), and T1-T6.

Both applications are built from `rust/launcher/src-tauri` serving the static SolidJS frontend in `ui/apps/web/dist`
(DD11) over the typed Tauri bridge declared in `rust/launcher/src-tauri/src/bridge/spec.rs` and
`ui/packages/bridge/src/api.ts`.

---

## 1. Applications and build configurations

| Application | Binary (`mainBinaryName`) | Tauri config | Cargo features (post-B3) | B1 baseline -> current state |
| --- | --- | --- | --- | --- |
| **Client** (`DaintyLass Launcher`) | `dainty-lass-client` | `rust/launcher/src-tauri/tauri.conf.json` (`io.github.freezamotoqueiro.dauntlessreforgedrevival`) | `default = ["host", "realms", "legacy"]` | **working**: `npm run build:client` (`tauri build --bundles nsis -- --features legacy,realms`) runs `beforeBuildCommand` (`bun install --cwd ../../ui --frozen-lockfile && bun run --cwd ../../ui build`) and produces `DaintyLass Launcher_2.0.0-alpha.0_x64-setup.exe`. |
| **Server GUI** (`DaintyLass Manager`) | `dainty-lass-manager` | `rust/launcher/src-tauri/tauri.server-gui.conf.json` (`io.github.freezamotoqueiro.dauntlessreforgedrevival.server`) | `server-gui = ["host", "dep:admin", "dep:server-controller", "dep:service-kit", "dep:host", "isles-1-14-7", "isles-1-12-0", "revived-1-4-4", "host?/realms"]` | **working**: `npm run build:manager` (alias `npm run build:server-gui`) builds `DaintyLass Manager_2.0.0-alpha.0_x64-setup.exe` with distinct `icons/server/*` icons, OS window title and header `"DaintyLass Manager"`, dedicated single-instance channel (`com.daintylass.server`), and all `services/host`, `server-controller`, `admin`, and `store` operations linked in-process. The bundle `identifier` (`...server`) is intentionally preserved so existing installs update in place, and the Cargo feature `server-gui` is kept consistently across `Cargo.toml`, `#[cfg(feature = "server-gui")]`, CI, `package.json`, and tests. |

---

## 2. Screens by application (`ui/apps/web/src/screens/`)

The shell (`ui/apps/web/src/app/app.tsx` and `ui/apps/web/src/components/chrome.tsx`) renders one active screen at a
time from the left navigation rail (`play`, `realms`, `server`, `game`, `host`, `settings` in `dainty-lass-client`;
`host` and `settings` in `dainty-lass-manager`), plus shared frame components (`TitleBar`, `Rail`, `Banners`,
`ActionBar`, `Dialogs`).

### 2.1 Client (`dainty-lass-client`)

| Screen / View | Source file(s) | Purpose | Bridge commands called | B1 baseline state | B4 final state | How verified |
| --- | --- | --- | --- | --- | --- | --- |
| **Window frame & banners** | `ui/apps/web/src/components/chrome.tsx`, `banners.tsx`, `action-bar.tsx`, `dialogs.tsx` | Title bar controls, navigation rail, error/notice/update banners, bottom step bar & primary action button, confirmation dialogs | `get_snapshot`, `minimize`, `toggle_maximize`, `close`, `dismiss_error`, `dismiss_notice`, `install_update`, `cancel_task`, `confirm_link_invite`, `dismiss_link_invite` | **working** (`install_update` is **maintainer: needs a real game, Epic account or second PC** for live GitHub release self-update) | **working** (`install_update` live self-update remains **maintainer: needs a real game, Epic account or second PC**) | Controller unit tests (`309 passed`) and live desktop E2E run (`node test/e2e-desktop.mjs`) |
| **Play (`play`)** | `ui/apps/web/src/screens/play.tsx` | Join a server by invite, switch/remove saved servers, register or paste/load an account key, save key backup, locate/verify Dauntless 1.14.7 (or 1.4.4 on Legacy servers), trust/install server runtime, and launch (`PLAY`) | `check_invite`, `join_server`, `switch_server`, `remove_saved_server`, `leave_server`, `accept_new_certificate`, `accept_moved_server`, `retry_connection`, `register`, `use_existing_key`, `load_key_backup`, `save_key_backup`, `dismiss_backup_reminder`, `choose_game_folder`, `use_game_path`, `use_detected_game`, `verify_game`, `trust_runtime`, `install_runtime`, `play`, `revived_choose_folder`, `revived_use_detected`, `revived_install`, `open_external` | **working** up to `Ready` phase; `play` / `play_revived` is **maintainer: needs a real game, Epic account or second PC** | **working** (`check_invite` -> `submit_invite` -> `register` -> `refresh_status` -> `account_banned` verified against live Server GUI in `e2e-desktop.mjs`; launching the real UE4 client binary via `play` remains **maintainer: needs a real game, Epic account or second PC**) | `e2e-desktop.mjs` (`client-{join,register,play,banned}-{en,pt-BR}.png`) and controller tests (`309 passed`) |
| **Realms (`realms`)** | `ui/apps/web/src/screens/realms.tsx` (`app/realms.ts`, `app/use-realms.ts`) | Host a hub or hunt session on this PC, join a friend's realm by session ID, view active session and local UDP/TURN relay status, stop/leave a realm | `realm_get`, `realm_host`, `realm_join`, `realm_stop` | **broken** in default `dainty-lass-client` build at B1 (`realms` not in `default` features) | **working** for single-PC state, capability gating, and session lifecycle (`realms` enabled in `default` features); two-PC NAT traversal (`RF8`) is **maintainer: needs a real game, Epic account or second PC** | `e2e-desktop.mjs` (`client-realms-{en,pt-BR}.png`), `ui/apps/web/test/realms.test.ts`, and `cargo test -p launcher --features realms` (`309 passed`) |
| **Server (`server`)** | `ui/apps/web/src/screens/servers.tsx` | Live status of the joined server (online players, active worlds/hunts, server details, news items) | `refresh_status`, `open_external` | **working** | **working** (displays live `Desktop E2E Hunts` status from in-process Rust backend and detects `account_banned` immediately on `refresh_status`) | `e2e-desktop.mjs` (`client-server-{en,pt-BR}.png`) and controller tests |
| **Game (`game`)** | `ui/apps/web/src/screens/game.tsx` | Sign in to Epic Games (WebView2 window or pasted authorization code), check library ownership of Dauntless, list available game versions and sizes, download/pause/resume/cancel a version from Epic CDN, and select an installed version | `epic_account_get`, `epic_sign_in`, `epic_sign_in_with_code`, `epic_sign_out`, `epic_check_library`, `game_version_list`, `game_install_get`, `game_install_start`, `game_install_pause`, `game_install_resume`, `game_install_cancel`, `use_game_path` | **working** against mocks and compiled-in manifest pins (`1.14.7`, `1.12.0`); live Epic OAuth and CDN download are **maintainer: needs a real game, Epic account or second PC** | **working** (`game_version_list`, `epic_account_get`, and `game_install_get` verified in installed client; live Epic OAuth and CDN download remain **maintainer: needs a real game, Epic account or second PC**) | `e2e-desktop.mjs` (`client-game-{en,pt-BR}.png`) and unit tests in `crates/epic` and `launcher` |
| **Host (`host`)** (when `settings.hostTools` is enabled) | `ui/apps/web/src/screens/host.tsx`, `host-tools.tsx`, `host-admin.tsx`, `modules.tsx` | Host tools inside the client when toggled on in Settings -> Advanced | Same as Server GUI below | **broken** in part at B1 | **working** | See Section 2.2 below |
| **Settings (`settings`)** | `ui/apps/web/src/screens/settings.tsx` | Game folder & edition (`reborn` / `legacy`), game language, graphics quality & safe 1280x720 window, account key backup & logout, UI language (`en` / `pt-BR`), streamer mode, Host tools toggle, runtime log profile, local runtime override, logs folder, and credits/license | `choose_game_folder`, `open_game_folder`, `verify_game`, `revived_choose_folder`, `revived_install`, `save_settings`, `save_key_backup`, `logout`, `choose_local_runtime`, `clear_local_runtime`, `open_logs_folder`, `open_external` | **working** | **working** (hides player-only game/account cards when running in `dainty-lass-manager`) | `e2e-desktop.mjs` (`client-settings-{en,pt-BR}.png`) and controller tests |

### 2.2 Server GUI (`dainty-lass-manager`)

In `dainty-lass-manager` (`snap.app.serverGui === true`), `ui/apps/web/src/app/launcher.tsx` opens directly on
`view === "host"` and `ui/apps/web/src/components/chrome.tsx` renders the server-focused rail (`Host` and `Settings`).
Inside `ui/apps/web/src/screens/host.tsx`, the Host screen has eight sub-tabs (`server`, `status`, `modules`, `admin`,
`history`, `gamedata`, `build`, `requirements`):

| Host Tab / Panel | Source file(s) | Purpose | Bridge commands called | B1 baseline state | B4 final state | How verified |
| --- | --- | --- | --- | --- | --- | --- |
| **Server tab (`server`)** | `ui/apps/web/src/screens/host.tsx` (`ServerTab`, `SetupCard`, `InviteCard`, `MultipliersCard`, `UpdateCard`, `ConsoleCard`), `host-admin.tsx` (`MongoMigrationCard`, `ServerConfigCard`, `DoctorBackupCard`) | Choose any server folder, run in-process `setup` (private/public), start/stop in-process Rust server, create/list invites, edit `host.toml` settings (`[server]`, `[database]`, `[modules]`, `[switches]`), inspect/migrate legacy MongoDB data with verify or confirmed `fresh_start`, run `doctor` and `backup`, set multipliers, and view console output | `host_choose_workspace`, `host_download_project`, `host_open_folder`, `host_open_logs`, `host_start_server`, `host_stop_server`, `host_run_setup`, `host_create_invite`, `server_invite_list`, `host_set_multipliers`, `host_update`, `server_config_get`, `server_config_save`, `server_mongo_status`, `server_mongo_migrate`, `server_mongo_fresh_start`, `server_doctor`, `server_backup`, `copy_text`, `open_external` | **broken** at B1 (shelled out to external `reforged-host` binary; lacked config editor, MongoDB migration UI, `doctor`, and `backup`) | **working** (calls `services/host` and `ServerGui` in-process; accepts any folder; includes `MongoMigrationCard`, `ServerConfigCard`, `DoctorBackupCard`, and active invite list) | `e2e-desktop.mjs` (`server-gui-{setup,server}-{en,pt-BR}.png`) and `controller/tests.rs` |
| **Status tab (`status`)** | `ui/apps/web/src/screens/host.tsx` (`StatusTab`) | Live status of in-process server components (`gateway`, `content`, `admin`, and loaded game modules) with per-component restart and per-module log viewer | `server_status`, `server_restart_component`, `server_module_logs` | **broken** in part at B1 (`host_start_server` did not start in-process `ServerGui`; lacked `all-modules` and per-module logs) | **working** (`host_start_server` starts in-process `ServerGui` when `server-gui` is enabled; shows live component states, restart buttons, and per-module log output) | `e2e-desktop.mjs` (`server-gui-status-{en,pt-BR}.png`) and `controller/tests.rs` |
| **Modules tab (`modules`)** | `ui/apps/web/src/screens/modules.tsx` (`ModulesTab`) | List modules from `host.toml` (`isles-1-14-7`, `isles-1-12-0`, `revived-1-4-4`), load/unload/restart modules, run module commands, edit module settings sections, preview plugin themes | `module_list`, `module_load`, `module_unload`, `module_restart`, `module_command`, `module_settings_get`, `module_settings_save` | **broken** at B1 (`server-gui` lacked `all-modules`) | **working** (`server-gui` enables `isles-1-14-7`, `isles-1-12-0`, `revived-1-4-4`, and `host?/realms`) | `e2e-desktop.mjs` (`server-gui-modules-{en,pt-BR}.png`) and `server_gui::modules::tests` (`5 passed`) |
| **Admin tab (`admin`)** | `ui/apps/web/src/screens/host-admin.tsx` (`AdminTab`) | Look up player accounts by username or account ID and run `kick`, `timeout`, `ban`, `unban`, or `lift_timeout` in-process with audit recording | `admin_lookup`, `admin_action` | **broken** (missing at B1) | **working** (wired in-process to `host::admin_cli` and `ServerGui::notify_account_revoked`; verified banning `Slayer_42` and immediate client revocation) | `e2e-desktop.mjs` (`server-gui-admin-{en,pt-BR}.png`) and `controller/tests.rs` |
| **History & Guarded tab (`history`)** | `ui/apps/web/src/screens/host-admin.tsx` (`HistoryTab`) | Query `store` change-log history by account/actor/time window, dry-run and apply reverts, and list/approve/cancel guarded admin actions | `history_query`, `history_revert_dry_run`, `history_revert_apply`, `guarded_list`, `guarded_approve`, `guarded_cancel` | **broken** at B1 (`api.ts` return types mismatched and no UI screen existed) | **working** (`api.ts` return types fixed; `HistoryTab` queries live `store` change log, previews/applies reverts, and manages guarded actions) | `e2e-desktop.mjs` (`server-gui-history-{en,pt-BR}.png`), `server_history::tests`, and `controller::guarded::tests` |
| **Game data tab (`gamedata`)** | `ui/apps/web/src/screens/host-tools.tsx` (`GameDataTab`) | Generate game data from `Items_Analysis`, copy/inject `CatalogExporter`, save/import game data packs, or copy example data | `host_generate_gamedata`, `host_inject_exporter`, `host_copy_exports`, `host_export_gamedata`, `host_import_gamedata`, `host_use_synthetic_data`, `host_open_folder` | **broken** in part at B1 (shelled out to `reforged-host` and showed `npm run gamedata`) | **working** in-process (`host::commands::gamedata`, `host::gamepack`, `host::gamedata::copy_example_game_data`, and `seed_embedded_examples` before `setup`); `host_inject_exporter` remains **maintainer: needs a real game, Epic account or second PC** | `e2e-desktop.mjs` (`server-gui-gamedata-{en,pt-BR}.png`) and `controller/tests.rs` |
| **Build tab (`build`)** | `ui/apps/web/src/screens/host-tools.tsx` (`BuildTab`) | Choose Dumper-7 DLL, launch game to dump SDK, import SDK, build `MysticParadox.dll`, `winmm.dll`, `CatalogExporter.dll`, and publish runtime | `host_choose_dumper`, `host_dump_sdk`, `host_import_sdk`, `host_choose_sdk_folder`, `host_build_target`, `host_publish_runtime`, `open_external` | **broken** in part at B1 (`host_publish_runtime` shelled out to `reforged-host`) | **working** in-process (`host::commands::publish_runtime`); `host_dump_sdk` and `host_build_target` against a live UE4 SDK dump remain **maintainer: needs a real game, Epic account or second PC** | `e2e-desktop.mjs` (`server-gui-build-{en,pt-BR}.png`) and `controller/host.rs` tests |
| **Requirements tab (`requirements`)** | `ui/apps/web/src/screens/host-tools.tsx` (`RequirementsTab`) | Check host/build/play prerequisites on this PC (`tailscale`, `visual_studio`, `rust`, `dumper7`, `dauntless`, `vc_redist`, `git`) | `host_inspect`, `host_choose_dumper`, `copy_text`, `open_external` | **broken** at B1 (required MongoDB and Git to host a server) | **working** (MongoDB removed from requirements; hosting only checks Tailscale for private mode while Git/VS/Rust/Dumper-7 are grouped under optional SDK/runtime build tools) | `e2e-desktop.mjs` (`server-gui-requirements-{en,pt-BR}.png`) and `platform::requirements::tests` |

---

## 3. Complete bridge command inventory (`spec.rs` & `ui/packages/bridge/src/api.ts`)

All 92 commands declared in `rust/launcher/src-tauri/src/bridge/spec.rs` (`spec::PLANNED` is empty):

| # | Command (`spec.rs` / `api.ts`) | Called from screen(s) | State at B1 | State at B4 | Verification |
| --- | --- | --- | --- | --- | --- |
| 1 | `get_snapshot` / `getSnapshot` | `app/launcher.tsx` (all screens) | **working** | **working** | `e2e-desktop.mjs` & controller tests (`309 passed`) |
| 2 | `check_invite` / `checkInvite` | `screens/play.tsx` | **working** | **working** | `e2e-desktop.mjs` & invite validation tests |
| 3 | `join_server` (`submit_invite`) / `joinServer` | `screens/play.tsx`, `app/launcher.tsx` | **working** | **working** | `e2e-desktop.mjs` & controller join tests |
| 4 | `switch_server` / `switchServer` | `screens/play.tsx`, `screens/settings.tsx` | **working** | **working** | Controller server-switching tests |
| 5 | `remove_saved_server` / `removeSavedServer` | `components/dialogs.tsx` | **working** | **working** | Controller tests |
| 6 | `leave_server` / `leaveServer` | `components/dialogs.tsx` | **working** | **working** | Controller tests |
| 7 | `accept_new_certificate` / `acceptNewCertificate` | `components/dialogs.tsx` | **working** | **working** | Pinning tests in `controller/tests.rs` |
| 8 | `accept_moved_server` / `acceptMovedServer` | `components/dialogs.tsx` | **working** | **working** | Controller tests |
| 9 | `retry_connection` / `retryConnection` | `screens/play.tsx`, `components/action-bar.tsx` | **working** | **working** | Controller tests |
| 10 | `register` / `register` | `screens/play.tsx`, `components/action-bar.tsx` | **working** | **working** | `e2e-desktop.mjs` (`Slayer_42`) & `controller/tests.rs` |
| 11 | `use_existing_key` / `useExistingKey` | `screens/play.tsx` | **working** | **working** | Key validation & redaction tests |
| 12 | `load_key_backup` / `loadKeyBackup` | `screens/play.tsx` | **working** | **working** | Key backup tests |
| 13 | `save_key_backup` / `saveKeyBackup` | `screens/play.tsx`, `screens/settings.tsx` | **working** | **working** | Key backup tests |
| 14 | `dismiss_backup_reminder` / `dismissBackupReminder` | `screens/play.tsx` | **working** | **working** | Controller tests |
| 15 | `logout` / `logout` | `components/dialogs.tsx` | **working** | **working** | Keystore removal tests |
| 16 | `choose_game_folder` / `chooseGameFolder` | `screens/play.tsx`, `screens/settings.tsx`, `components/action-bar.tsx` | **working** | **working** | `controller/game.rs` tests |
| 17 | `use_game_path` / `useGamePath` | `screens/play.tsx`, `screens/game.tsx` | **working** | **working** | `controller/game.rs` tests |
| 18 | `use_detected_game` / `useDetectedGame` | `screens/play.tsx` | **working** | **working** | `controller/game.rs` tests |
| 19 | `verify_game` / `verifyGame` | `screens/play.tsx`, `screens/settings.tsx`, `components/action-bar.tsx` | **working** | **working** | SHA-256 verification tests |
| 20 | `trust_runtime` / `trustRuntime` | `components/dialogs.tsx` | **working** | **working** | `controller/runtime.rs` tests |
| 21 | `install_runtime` / `installRuntime` | `screens/play.tsx`, `components/action-bar.tsx` | **working** | **working** | Runtime download & hash verification tests |
| 22 | `choose_local_runtime` / `chooseLocalRuntime` | `screens/settings.tsx` | **working** | **working** | `controller/runtime.rs` tests |
| 23 | `clear_local_runtime` / `clearLocalRuntime` | `screens/settings.tsx` | **working** | **working** | `controller/runtime.rs` tests |
| 24 | `cancel_task` / `cancelTask` | `components/action-bar.tsx` | **working** | **working** | Task cancellation tests |
| 25 | `play` / `play` | `components/action-bar.tsx` (`screens/play.tsx`) | **maintainer: needs a real game, Epic account or second PC** | **maintainer: needs a real game, Epic account or second PC** | Stand-in process launch verified in `platform::launch::tests`; launching a real Dauntless 1.14.7/1.4.4 client binary requires the game installed on disk |
| 26 | `save_settings` / `saveSettings` | `screens/settings.tsx`, `screens/play.tsx`, `components/dialogs.tsx` | **working** | **working** | `e2e-desktop.mjs` (`en` / `pt-BR` switching) & `platform::settings::tests` |
| 27 | `refresh_status` / `refreshStatus` | `screens/servers.tsx` | **working** | **working** | `e2e-desktop.mjs` (online status + ban detection) & `controller/tests.rs` |
| 28 | `dismiss_error` / `dismissError` | `components/banners.tsx` | **working** | **working** | `e2e-desktop.mjs` & controller tests |
| 29 | `dismiss_notice` / `dismissNotice` | `components/banners.tsx` | **working** | **working** | `e2e-desktop.mjs` & controller tests |
| 30 | `confirm_link_invite` / `confirmLinkInvite` | `components/dialogs.tsx` | **working** | **working** | Deep-link invite tests |
| 31 | `dismiss_link_invite` / `dismissLinkInvite` | `components/dialogs.tsx` | **working** | **working** | Deep-link invite tests |
| 32 | `open_logs_folder` / `openLogsFolder` | `screens/settings.tsx` | **working** | **working** | Code read & path resolution tests |
| 33 | `open_game_folder` / `openGameFolder` | `screens/settings.tsx` | **working** | **working** | Code read & path resolution tests |
| 34 | `open_external` / `openExternal` | `screens/play.tsx`, `servers.tsx`, `settings.tsx`, `host.tsx`, `host-tools.tsx` | **working** | **working** | `platform::links::tests` allowlist verification |
| 35 | `install_update` / `installUpdate` | `components/banners.tsx` | **maintainer: needs a real game, Epic account or second PC** | **maintainer: needs a real game, Epic account or second PC** | `platform::updater::tests`; live self-update requires a published GitHub release |
| 36 | `minimize` / `minimize` | `components/chrome.tsx` | **working** | **working** | Window control handler in `window.rs` |
| 37 | `toggle_maximize` / `toggleMaximize` | `components/chrome.tsx` | **working** | **working** | Window control handler in `window.rs` |
| 38 | `close` / `close` | `components/chrome.tsx` | **working** | **working** | Window control handler in `window.rs` |
| 39 | `copy_text` / `copyText` | `screens/host.tsx`, `host-tools.tsx`, `host-admin.tsx`, `game.tsx`, `realms.tsx` | **working** | **working** | Clipboard handler in `commands.rs` |
| 40 | `host_inspect` / `hostInspect` | `app/host-store.ts`, `screens/host-tools.tsx` | **broken** | **working** | `e2e-desktop.mjs` & `platform::requirements::tests` (no MongoDB requirement; any workspace directory accepted) |
| 41 | `host_choose_workspace` / `hostChooseWorkspace` | `screens/host.tsx` | **broken** | **working** | `controller/tests.rs` (`set_workspace_root` accepts any existing directory) |
| 42 | `host_action` (`download_project`, `choose_dumper`, `inject_dumper`, `import_sdk`, `choose_sdk_folder`, `build_*`, `inject_exporter`, `copy_exports`, `synthetic_data`, `generate_gamedata`, `export_gamedata`, `import_gamedata`, `publish_runtime`, `open_folder_*`, `open_logs`) / `host*` | `screens/host.tsx`, `screens/host-tools.tsx` | **broken** in part | **working** in-process (`inject_dumper`, `inject_exporter`, and `build_*` against a live UE4 client/SDK dump are **maintainer: needs a real game, Epic account or second PC**) | `e2e-desktop.mjs` (`synthetic_data`) & `controller/tests.rs` |
| 43 | `host_setup` / `hostRunSetup` | `screens/host.tsx` | **broken** | **working** | `e2e-desktop.mjs` (`Desktop E2E Hunts`) & `controller/tests.rs` |
| 44 | `host_server_start` / `hostStartServer` | `screens/host.tsx` | **broken** | **working** | `e2e-desktop.mjs` & `controller/tests.rs` (starts in-process `ServerGui` controller) |
| 45 | `host_server_stop` / `hostStopServer` | `screens/host.tsx` | **broken** | **working** | `e2e-desktop.mjs` & `controller/tests.rs` (cleanly stops in-process `ServerGui` controller) |
| 46 | `host_create_invite` / `hostCreateInvite` | `screens/host.tsx` | **broken** | **working** | `e2e-desktop.mjs` & `controller/tests.rs` (calls `host::commands::invite::create_invite` in-process) |
| 47 | `host_update` / `hostUpdate` | `screens/host.tsx` | **broken** | **working** | Calls `host::commands::update` in-process |
| 48 | `host_set_multipliers` / `hostSetMultipliers` | `screens/host.tsx` | **broken** | **working** | Calls `host::commands::multipliers` in-process |
| 49 | `revived_choose_folder` / `revivedChooseFolder` | `screens/play.tsx`, `screens/settings.tsx` | **working** | **working** | `controller/legacy.rs` tests |
| 50 | `revived_use_detected` / `revivedUseDetected` | `screens/play.tsx` | **working** | **working** | `controller/legacy.rs` tests |
| 51 | `revived_install` / `revivedInstall` | `screens/play.tsx`, `screens/settings.tsx`, `components/action-bar.tsx` | **working** | **working** | Tested against local mock content server in `controller/tests.rs` |
| 52 | `epic_sign_in` / `epicSignIn` | `screens/game.tsx` | **maintainer: needs a real game, Epic account or second PC** | **maintainer: needs a real game, Epic account or second PC** | Mock HTTP tested in `epic_login::tests`; live sign-in requires a real Epic account (`DE1`) |
| 53 | `epic_sign_in_with_code` / `epicSignInWithCode` | `screens/game.tsx` | **maintainer: needs a real game, Epic account or second PC** | **maintainer: needs a real game, Epic account or second PC** | Input validation & mock exchange tested in `epic_login::tests`; live exchange requires a real Epic auth code (`DE1`) |
| 54 | `epic_sign_out` / `epicSignOut` | `screens/game.tsx` | **working** | **working** | `epic_session::tests` |
| 55 | `epic_account_get` / `epicAccountGet` | `screens/game.tsx` | **working** | **working** | `e2e-desktop.mjs` & `epic_session::tests` |
| 56 | `epic_check_library` / `epicCheckLibrary` | `screens/game.tsx` | **maintainer: needs a real game, Epic account or second PC** | **maintainer: needs a real game, Epic account or second PC** | Recorded fixture tested in `epic_library::tests`; live check requires a real Epic session (`DE1`) |
| 57 | `game_version_list` / `gameVersionList` | `screens/game.tsx` | **working** | **working** | `e2e-desktop.mjs` & `net::versions::tests` |
| 58 | `game_install_start` / `gameInstallStart` | `screens/game.tsx` | **maintainer: needs a real game, Epic account or second PC** | **maintainer: needs a real game, Epic account or second PC** | Mock chunk server tested in `crates/epic` and `net::epic_install::tests`; live CDN download requires a real Epic account owning Dauntless (`DE1`) |
| 59 | `game_install_pause` / `gameInstallPause` | `screens/game.tsx` | **working** | **working** | `net::epic_install::tests` |
| 60 | `game_install_resume` / `gameInstallResume` | `screens/game.tsx` | **working** | **working** | `net::epic_install::tests` |
| 61 | `game_install_cancel` / `gameInstallCancel` | `screens/game.tsx` | **working** | **working** | `net::epic_install::tests` |
| 62 | `game_install_get` / `gameInstallGet` | `screens/game.tsx` | **working** | **working** | `controller::install::tests` |
| 63 | `region_ping` / `regionPing` | `screens/servers.tsx` / `net/regionping.rs` | **working** | **working** | `net::regionping::tests` |
| 64 | `guarded_list` / `guardedList` | `screens/host-admin.tsx` (`HistoryTab`) | **broken** (no UI) | **working** | `e2e-desktop.mjs` (`HistoryTab`) & `controller::guarded::tests` |
| 65 | `guarded_approve` / `guardedApprove` | `screens/host-admin.tsx` (`HistoryTab`) | **broken** (no UI) | **working** | `HistoryTab` & `controller::guarded::tests` |
| 66 | `guarded_cancel` / `guardedCancel` | `screens/host-admin.tsx` (`HistoryTab`) | **broken** (no UI) | **working** | `HistoryTab` & `controller::guarded::tests` |
| 67 | `server_status` / `serverStatus` | `screens/host.tsx` (`StatusTab`) | **broken** in part | **working** | `e2e-desktop.mjs` (`StatusTab`) & `controller/tests.rs` |
| 68 | `server_restart_component` / `serverRestartComponent` | `screens/host.tsx` (`StatusTab`) | **broken** in part | **working** | `StatusTab` & `server_gui.rs` |
| 69 | `module_list` / `moduleList` | `screens/modules.tsx`, `screens/host.tsx` | **broken** in part | **working** | `e2e-desktop.mjs` (`ModulesTab`) & `server_gui::modules::tests` |
| 70 | `module_load` / `moduleLoad` | `screens/modules.tsx` | **broken** in part | **working** | `ModulesTab` & `server_gui::modules::tests` |
| 71 | `module_unload` / `moduleUnload` | `screens/modules.tsx` | **broken** in part | **working** | `ModulesTab` & `server_gui::modules::tests` |
| 72 | `module_restart` / `moduleRestart` | `screens/modules.tsx` | **broken** in part | **working** | `ModulesTab` & `server_gui::modules::tests` |
| 73 | `module_command` / `moduleCommand` | `screens/modules.tsx` | **broken** in part | **working** | `ModulesTab` & `server_gui::modules::tests` |
| 74 | `module_settings_get` / `moduleSettingsGet` | `screens/modules.tsx` | **broken** in part | **working** | `ModulesTab` & `server_gui::modules::tests` |
| 75 | `module_settings_save` / `moduleSettingsSave` | `screens/modules.tsx` | **broken** in part | **working** | `ModulesTab` & `server_gui::modules::tests` |
| 76 | `history_query` / `historyQuery` | `screens/host-admin.tsx` (`HistoryTab`) | **broken** | **working** | `e2e-desktop.mjs` (verified change-log entries for `Slayer_42`) & `server_history::tests` |
| 77 | `history_revert_dry_run` / `historyRevertDryRun` | `screens/host-admin.tsx` (`HistoryTab`) | **broken** | **working** | `HistoryTab` & `server_history::tests` |
| 78 | `history_revert_apply` / `historyRevertApply` | `screens/host-admin.tsx` (`HistoryTab`) | **broken** | **working** | `HistoryTab` & `server_history::tests` |
| 79 | `admin_lookup` / `adminLookup` | `screens/host-admin.tsx` (`AdminTab`) | *(Added in B3)* | **working** | `e2e-desktop.mjs` (`AdminTab`) & `controller/tests.rs` |
| 80 | `admin_action` / `adminAction` | `screens/host-admin.tsx` (`AdminTab`) | *(Added in B3)* | **working** | `e2e-desktop.mjs` (`ban` on `Slayer_42`) & `controller/tests.rs` |
| 81 | `server_doctor` / `serverDoctor` | `screens/host-admin.tsx` (`DoctorBackupCard`) | *(Added in B3)* | **working** | `e2e-desktop.mjs` (`ok: true`, 6 checks) & `controller/tests.rs` |
| 82 | `server_backup` / `serverBackup` | `screens/host-admin.tsx` (`DoctorBackupCard`) | *(Added in B3)* | **working** | `e2e-desktop.mjs` (`ok: true`, `filesCopied: 6`) & `controller/tests.rs` |
| 83 | `server_config_get` / `serverConfigGet` | `screens/host-admin.tsx` (`ServerConfigCard`) | *(Added in B3)* | **working** | `e2e-desktop.mjs` & `controller/tests.rs` |
| 84 | `server_config_save` / `serverConfigSave` | `screens/host-admin.tsx` (`ServerConfigCard`) | *(Added in B3)* | **working** | `controller/tests.rs` (round-trip `host.toml` + `.workspace/config/*.env` regeneration) |
| 85 | `server_mongo_status` / `serverMongoStatus` | `screens/host-admin.tsx` (`MongoMigrationCard`) | *(Added in B3)* | **working** | `e2e-desktop.mjs` & `controller/tests.rs` |
| 86 | `server_mongo_migrate` / `serverMongoMigrate` | `screens/host-admin.tsx` (`MongoMigrationCard`) | *(Added in B3)* | **working** | Calls `host::commands::update::migrate_existing_mongo_data` (migrate + verify) in-process |
| 87 | `server_mongo_fresh_start` / `serverMongoFreshStart` | `screens/host-admin.tsx` (`MongoMigrationCard`) | *(Added in B3)* | **working** | Calls `host::commands::update::mark_mongo_fresh_start` after explicit UI confirmation |
| 88 | `server_module_logs` / `serverModuleLogs` | `screens/host.tsx` (`StatusTab`) | *(Added in B3)* | **working** | Reads per-component log lines from `.workspace/logs` |
| 89 | `server_invite_list` / `serverInviteList` | `screens/host.tsx` (`InviteCard`) | *(Added in B3)* | **working** | `e2e-desktop.mjs` (lists created invites with code, recipient, and expiry) |
| 90 | `realm_host` / `realmHost` | `screens/realms.tsx` | **broken** in default client build | **working** (single-PC state; two-PC NAT is **maintainer: needs a real game, Epic account or second PC**) | `ui/apps/web/test/realms.test.ts` & `cargo test -p launcher --features realms` (`309 passed`) |
| 91 | `realm_join` / `realmJoin` | `screens/realms.tsx` | **broken** in default client build | **working** (single-PC state; two-PC NAT is **maintainer: needs a real game, Epic account or second PC**) | `ui/apps/web/test/realms.test.ts` & `cargo test -p launcher --features realms` (`309 passed`) |
| 92 | `realm_stop` / `realmStop` & `realm_get` / `realmGet` | `screens/realms.tsx`, `app/launcher.tsx` | **broken** in default client build | **working** | `e2e-desktop.mjs` (`client-realms-{en,pt-BR}.png`) & `cargo test -p launcher --features realms` (`309 passed`) |

---

## 4. Strings and flows that assumed Node, npm, MongoDB, or a repository clone (resolved in B3 & T2)

### 4.1 Code and setup flow changes

| Location | Earlier Node / npm / MongoDB / repository assumption | Resolution (B3 & T2) |
| --- | --- | --- |
| `rust/launcher/src-tauri/src/platform/requirements.rs` | Checked for Windows MongoDB Community Server (`C:\Program Files\MongoDB\Server\*\bin\mongod.exe`) and reported `RequirementState::Missing` when absent | Removed MongoDB from host requirements (`DATABASE = Database::Sqlite`); only checks tools the Rust stack uses (`tailscale`, `visual_studio`, `rust`, `dumper7`, `dauntless`, `vc_redist`, `git`) |
| `rust/launcher/src-tauri/src/bridge/spec.rs` & `platform/links.rs` | Listed `"mongodb"` in `REQUIREMENT_IDS` and `"mongodb"` / `"node"` in `EXTERNAL_URL_KEYS` | Removed `"mongodb"` from `REQUIREMENT_IDS` and `"mongodb"` / `"node"` from `EXTERNAL_URL_KEYS`; regenerated `isolation/commands.js` |
| `rust/launcher/src-tauri/src/bridge/spec.rs` & `platform/hostserver.rs` | Listed `"mongod"` as component index 0 in `HOST_SERVER_COMPONENTS` | Removed `"mongod"` from `HOST_SERVER_COMPONENTS` (`&["backend", "director", "content", "admin", "gateway", "allowlist"]`) and `hostserver.rs` |
| `ui/apps/web/src/app/host-model.ts` | Listed `"git"` and `"mongodb"` under `"To host a server"` on the Requirements tab | Removed `"mongodb"` completely and moved `"git"` under the optional SDK/runtime build tools group; hosting only checks `"tailscale"` |
| `ui/apps/web/src/screens/host-tools.tsx` | Rendered `t("host_data_step_pack")` (`npm run gamedata -- --export`) | Switched to `t("host_data_step_pack_rust")` (`reforged-host gamedata --export`) |
| `rust/launcher/src-tauri/src/controller/host.rs` (`ensure_host_workspace`, `host_choose_workspace`) | Required a pre-existing folder or `<game_parent>/Reforged` repository checkout; offered `host_download_project` (`Download it`) on the Server GUI | In `DaintyLass Manager` (`has_embedded()`), `ensure_host_workspace(true)` creates and selects `<user_data>/Server` automatically on a fresh profile, `host_choose_workspace` creates any picked folder if it does not exist yet, and `host_download_project` (`ensure_host_workspace(false)`) returns `ErrorCode::NotInThisBuild` |
| `ui/apps/web/src/screens/host.tsx` (`WorkspaceBar`, `SetupPanel`, `ServerCard`) | Showed repository checkout copy (`host_ws_text`), the `Download it` button (`host_ws_download`), Node-era setup copy (`hsetup_text`), no game-folder picker on `SetupPanel`, and `"Closing the launcher stops the server."` | In `DaintyLass Manager` (`snap.app.serverGui`), `WorkspaceBar` shows `host_ws_title_mgr` / `host_ws_text_mgr` (`Server data folder`) with no `Download it` button, `SetupPanel` includes a direct Dauntless `1.14.7` game-folder picker (`Choose folder` / `Change folder`) alongside synthetic data and `hsetup_text_mgr`, and `ServerCard` shows `hsrv_text_stopped_mgr` / `hsrv_text_running_mgr` (`Closing DaintyLass Manager stops the server.`) |
| `ui/apps/web/src/components/chrome.tsx` & `ui/apps/web/src/app/app.tsx` | Always rendered `app_name` (`DaintyLass Launcher`) in the header bar and `document.title` | Renders `app_name_server` (`DaintyLass Manager`) when `snap.app.serverGui` is true, matching `tauri.server-gui.conf.json` (`productName: "DaintyLass Manager"`) |

#### Why `host_download_project` is kept in `dainty-lass-client` Host tools and disabled in `dainty-lass-manager`

- **`dainty-lass-manager` (`server-gui`)** compiles the Rust server services (`gateway`, `content`, `admin`, `isles-1-14-7`, `isles-1-12-0`, `revived-1-4-4`) and `services/host` library commands into the binary itself. It needs only a writable data directory (defaulting to `<user_data>/Server`) for `host.toml`, `.workspace/data/reforged.db`, TLS certificates, and logs. `host_download_project` has no purpose there: the UI hides the button when `snap.app.serverGui` is true and the controller refuses `host_download_project` with `ErrorCode::NotInThisBuild`.
- **`dainty-lass-client` (with Host tools enabled in Settings)** does not embed the server or C++ runtime source; it drives an external `reforged-host` checkout and the optional C++ runtime/exporter MSBuild projects (`Build` tab). In the client's Host tools, `ensure_host_workspace` checks whether `<game_parent>/Reforged` already holds a valid workspace (`is_workspace`) and selects it if present. Keeping `host_download_project` on the client side preserves that external-checkout lookup for developers using the client's Host tools.

### 4.2 Catalog audit (`ui/packages/i18n/catalogs/en.json` & `pt-BR.json`)

Every occurrence in both catalogs of `"Mystic Paradox"` (as something the host downloads), `"npm"`, `"Node"`, `"MongoDB"` (outside migration/rollback screens), `"repository"`, and `"clone"` was audited in T2:

| Catalog key(s) | Term matched | Action taken in T2 |
| --- | --- | --- |
| `host_ws_text`, `host_ws_pick`, `host_ws_missing` | `"repository"`, `"Mystic Paradox"` | Updated `host_ws_text` to describe the Rust workspace without claiming setup downloads Mystic Paradox; added `host_ws_title_mgr`, `host_ws_text_mgr`, `host_ws_pick_mgr`, and `host_ws_missing_mgr` (`"Server data folder"`) for `DaintyLass Manager` |
| `hsetup_text` | `"Mystic Paradox"`, `"installs and builds the services"` | Updated `hsetup_text` to describe writing `host.toml`, generating keys/certificates, and preparing the SQLite database; added `hsetup_text_mgr` for `DaintyLass Manager` |
| `host_req_git_how`, `host_req_git_detail_missing` | `"clone"`, `"repository"`, `"Mystic Paradox"` | Updated in both `en.json` and `pt-BR.json` to state that Git is optional and only used by the Build tab when fetching C++ runtime sources (`reforged-host upstream`), not to clone the server or download Mystic Paradox during setup |
| `host_job_setup` | `"Mystic Paradox"` | Updated from `"Preparing the server (Mystic Paradox, services, configuration)..."` to `"Preparing the server (configuration, certificates, database)..."` (`"Preparando o servidor (configuracao, certificados, banco de dados)..."`) |
| `err_host_workspace` | `"repository"` | Added `err_host_workspace_mgr` (`"Choose a folder for the server's data first."`) and routed `DaintyLass Manager` workspace errors through `workspace_fail()` (`"workspace_mgr"`); kept `err_host_workspace` for the client's external Host tools |
| `app_name`, `hsrv_text_stopped`, `hsrv_text_running` | Launcher name in Server GUI | Added `app_name_server` (`"DaintyLass Manager"`), `hsrv_text_stopped_mgr`, and `hsrv_text_running_mgr` (`"Closing DaintyLass Manager stops the server."`) |
| `hmgo_title`, `hmgo_migrated`, `hmgo_skipped`, `hmgo_unmigrated`, `hmgo_unreachable`, `hmgo_confirm_fresh` | `"MongoDB"` | **Deliberately kept**: these are the M1/M4 MongoDB-to-SQLite migration and fresh-start confirmation strings on the Server Settings card, which only render when `mongoMigration.state !== "none"` |
| `host_job_upstream`, `err_host_upstream_old` | `"Mystic Paradox"` | **Deliberately kept**: only used when a developer explicitly triggers a C++ runtime/SDK action on the `Build` tab that invokes `reforged-host upstream` to populate C++ runtime sources |
| `host_build_intro`, `host_build_step_dump`, `host_build_step_sdk`, `host_build_step_compile`, `host_req_dumper7_why`, `host_req_vc_why`, `err_runtime_missing` | `"Mystic Paradox"` | **Deliberately kept**: refers accurately to the client-side `MysticParadox.dll` C++ UE4 runtime DLL compiled on the optional `Build` tab, not to server setup or downloading |
| `host_data_step_pack` | `"npm"` | **Deliberately kept as legacy dead key**: `ui/apps/web/src/screens/host-tools.tsx` renders `host_data_step_pack_rust` (`reforged-host gamedata --export`); `host_data_step_pack` is retained only for catalog key stability |

---

## 5. Part B4 end-to-end desktop verification and screenshots

### 5.1 Repeatable desktop E2E script (`rust/launcher/test/e2e-desktop.mjs`)

Run with `node test/e2e-desktop.mjs` from `rust/launcher`:

```text
=== B4 Desktop E2E Verification ===
Root directory: R:\Temp\dl-e2e-desktop-mfe8m3
=== 1. Installing Client & Server GUI from NSIS installers ===
Installed client: R:\Temp\dl-e2e-desktop-mfe8m3\install-client\dainty-lass-client.exe
Installed manager: R:\Temp\dl-e2e-desktop-mfe8m3\install-server\dainty-lass-manager.exe
=== 2. Launching Server GUI and Client Desktop Apps ===
Connected to Server GUI WebView2 CDP and Client WebView2 CDP
=== 3. Driving Server GUI: seed synthetic data & run setup ===
host_action(synthetic_data): true null
host_setup: true null
server_config_get: mode=private, serverName=Desktop E2E Hunts, dbEngine=sqlite
server_doctor: ok=true, checks=6
=== 4. Driving Server GUI: start server & create invite ===
host_server_start: true null
server_status: running=true, embedded=true, components=gateway:running, content:running, admin:running, isles-1-14-7:running
Created invite for E2ETester: code=H3M3-A82J-R433
=== 5. Driving Client: join server with invite & register account ===
check_invite valid: true serverName: Desktop E2E Hunts
submit_invite phase: need_account lastError: null
register phase: need_game account: Slayer_42 hasKey: true
Client server status online: true
=== 6. Capturing screenshots of all screens (en & pt-BR) ===
=== 7. Driving Server GUI: ban player & verify Client reflects ban ===
admin_action(ban): ok=true, message=Banned Slayer_42 (059b4bd8-8e4e-4598-a2f6-a9b06ca0cd54).
Client after ban & refresh_status: phase=need_account, lastError=account_banned
=== 8. Driving Server GUI: history query, backup & clean stop ===
history_query count: 2
server_backup: ok=true, filesCopied=6
host_server_stop: true null
server_status after stop: running=false
=== B4 Desktop E2E Verification PASSED ===
```

### 5.2 Saved screenshots (`docs/port/screenshots/`)

All 34 screenshots captured from the installed Windows desktop applications in English (`en`) and Brazilian Portuguese
(`pt-BR`):

- **Client (`dainty-lass-client.exe`) — 16 screenshots**:
  - `client-join-en.png`, `client-join-pt-BR.png` (Play screen — invite paste & check before joining)
  - `client-register-en.png`, `client-register-pt-BR.png` (Play screen — connected to `Desktop E2E Hunts`, account registration step)
  - `client-play-en.png`, `client-play-pt-BR.png` (Play screen — signed in as `Slayer_42` with key backup banner and game folder step)
  - `client-realms-en.png`, `client-realms-pt-BR.png` (Realms screen — host hub/hunt & join realm by ID)
  - `client-server-en.png`, `client-server-pt-BR.png` (Server screen — live `Desktop E2E Hunts` status, online badge, game build info)
  - `client-game-en.png`, `client-game-pt-BR.png` (Game screen — Epic Games sign-in card & `1.14.7` / `1.12.0` version list)
  - `client-settings-en.png`, `client-settings-pt-BR.png` (Settings screen — game folder, edition, language, graphics, account key backup, logs)
  - `client-banned-en.png`, `client-banned-pt-BR.png` (Play screen — `account_banned` error banner shown after Server GUI admin ban)
- **Server GUI (`dainty-lass-manager.exe`) — 18 screenshots**:
  - `server-gui-setup-en.png`, `server-gui-setup-pt-BR.png` (Host -> Server tab before setup — initial setup wizard & workspace selector)
  - `server-gui-server-en.png`, `server-gui-server-pt-BR.png` (Host -> Server tab running — live status, invites list, `host.toml` settings editor, doctor & backup, multipliers, console)
  - `server-gui-status-en.png`, `server-gui-status-pt-BR.png` (Host -> Status tab — live `gateway`, `content`, `admin`, and `isles-1-14-7` component cards with restart & module log controls)
  - `server-gui-modules-en.png`, `server-gui-modules-pt-BR.png` (Host -> Modules tab — compiled-in `isles-1-14-7`, `isles-1-12-0`, and `revived-1-4-4` modules, commands, and settings)
  - `server-gui-admin-en.png`, `server-gui-admin-pt-BR.png` (Host -> Admin tab — player lookup and kick/timeout/ban/unban/lift-timeout controls)
  - `server-gui-history-en.png`, `server-gui-history-pt-BR.png` (Host -> History tab — `store` change-log query, revert dry-run/apply, and guarded actions)
  - `server-gui-gamedata-en.png`, `server-gui-gamedata-pt-BR.png` (Host -> Game Data tab — synthetic data, pack import/export, and `Items_Analysis` generator)
  - `server-gui-build-en.png`, `server-gui-build-pt-BR.png` (Host -> Build tab — Dumper-7, SDK import, runtime/loader/exporter build, and publish runtime)
  - `server-gui-requirements-en.png`, `server-gui-requirements-pt-BR.png` (Host -> Requirements tab — Tailscale host check and optional SDK/build tool checks with no MongoDB or Node requirement)

### 5.3 Actions that require maintainer hardware, accounts, or proprietary binaries

Every screen and tab was rendered and screenshotted in both languages. The following specific actions could not be
exercised end-to-end against live external systems on this machine:

1. **Launching the proprietary Dauntless UE4 game client (`play` / `revived_install` launch)**: Requires a full retail
   installation of Dauntless `1.14.7` (`Dauntless-Win64-Shipping.exe` matching the pinned SHA-256) or `1.4.4`. Stand-in
   process launch, `mystic_loader.ini` writing, loopback relay binding, and RD9 cleanup are verified in
   `platform::launch::tests`.
2. **Dumping the UE4 C++ SDK (`host_action` `inject_dumper`) and exporting live catalog items (`inject_exporter`)**:
   Requires launching the retail Dauntless `1.14.7` client with `Dumper-7.dll` or `CatalogExporter.dll` configured in
   `mystic_loader.ini`.
3. **Live Epic Games OAuth login, library ownership check, and CDN chunk download (`epic_sign_in`,
   `epic_sign_in_with_code`, `epic_check_library`, `game_install_start`)**: Requires a real Epic Games account with
   Dauntless (`Jackal`) in its library (`DE1`). Tested against mock HTTP and chunk servers in `crates/epic` and
   `launcher`.
4. **Two-PC NAT realm traversal (`realm_host` + `realm_join` across two machines, `RF8`)**: Requires a second PC and a
   retail `1.14.7` server binary. Single-PC realm state, capability gating, and `ParadoxTransport.dll` FFI lifecycle
   are verified in `launcher` (`--features realms`) and `rustic-rifts` (`rifts-host`).
5. **Live GitHub release self-update (`install_update`)**: Requires a published release asset on the configured release
   repository. Verified against mock release payloads in `platform::updater::tests`.

---

## 6. Summary of work tracked across B1-B4

| Step | Scope | Status |
| --- | --- | --- |
| **B1** | Write and merge `docs/port/tauri-apps-status.md` inventory on `docs/tauri-apps-status` | **Done** (`f20e2df`, merged `effe5dd`) |
| **B2** | Build both Windows installers (`dainty-lass-client` and `dainty-lass-manager` with its own product name, identifier, and icon) from a clean clone via `rust/launcher/package.json` and `tauri*.conf.json`, with `ui/` built by Bun automatically; document in `rust/launcher/README.md` and add a bundle build check | **Done** (`8b122b8`, merged `dd8a433`; renamed to `dainty-lass-manager` / `DaintyLass Manager` in T3) |
| **B3** | Implement every stub/broken command and missing Server GUI / Client workflow (`services/host` in-process library calls, `server-gui` `all-modules` + `client` `realms` features, admin kick/timeout/ban/unban/lookup, history query/dry-run/apply + guarded actions UI, `doctor`, `backup`, `host.toml` settings editor, per-module logs, M1/M4 MongoDB migration status/migrate-with-verify/confirmed-fresh-start, and removal of obsolete Node/npm/MongoDB assumptions) | **Done** (`dainty-lass-ui` `8a69a05`, merged `0314dde`; `dainty-lass` `fix/b3-tauri-apps`) |
| **B4** | Install both built apps, run repeatable end-to-end test script (`server GUI setup -> start -> invite -> client join -> register -> sign in -> status -> server GUI ban -> client shows ban -> server GUI clean stop`), save `en` and `pt-BR` screenshots under `docs/port/screenshots/`, and update this table | **Done** (`dainty-lass` `fix/b3-tauri-apps`) |
