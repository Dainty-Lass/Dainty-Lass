# DL4a Clean-Room Specification — Launcher MIT Crate Split & Rewrite (`launcher/src-tauri`)

This specification is the clean-room input for **DL4b** (`docs/dainty-lass-plan.md`, decision **DD26**). It:

1. Classifies every source file under `launcher/src-tauri/src/` across three provenance groups (**Born-MIT**, **DL4 Rewrite / Freeza-Consent**, and **AGPL DR Core**).
2. Defines the crate boundary and dependency seams between the new MIT crate (`crates/dainty-launcher-kit`, or equivalent workspace member with `license = "MIT"`) and the AGPL-3.0-only `launcher/src-tauri` binary/Tauri shell.
3. Specifies the complete clean-room behaviour, public types, constants, traits, function signatures, bridge commands/events, file-system invariants, RD9 no-injection rules, and test expectations for the 8 **DL4 Rewrite** modules and the **Born-MIT / DL4** modules that interact with them.

> **Clean-room rule for DL4b:** Every file placed in the MIT crate must be written from this specification (or moved intact if Born-MIT with zero external derivation) and must contain **no** source comments, identifiers, or docstrings naming external launcher repositories, commits, or TypeScript source files.

---

## 1. Complete Module Classification (`launcher/src-tauri/src/`)

All 95 `.rs` files in `launcher/src-tauri/src/` are classified below into three categories:

- **Born-MIT**: Written originally for Dainty Lass (rows RD9, DU2–DU5, DE2–DE5, DN7, DH3b, DH6, RF7, RD26) without derivation from Dauntless Revived or external launcher code. Moves into the MIT crate (or stays at the Tauri boundary if it is a `#[tauri::command]` wrapper, with its pure logic in the MIT crate).
- **DL4 Rewrite (Freeza-Consent / Clean-Room)**: The 8 module groups specified in Section 2 (`net/runtime_files`, `net/verify`, `platform/hostkit`, `platform/hostserver`, `platform/requirements`, `platform/dialogs`, `controller/host` + `host_off`, `controller/runtime`). Rewritten from Section 2 into the MIT crate under `license = "MIT"`.
- **AGPL DR Core**: Files derived from Dauntless Revived at `e1c5fec` / `24b7624` (or keeping its state-machine/downloader/HTTP/relay/keystore structure). These remain in `launcher/src-tauri` under `AGPL-3.0-only`.

### 1.1 Audit Note on Mixed-Provenance Files Named in DL4

During the file-by-file provenance audit of the files listed in row DL4, seven files were found to carry Dauntless Revived (DR) provenance or structure:

1. **`net/epic_install.rs`** (listed in `NOTICE.md` under AGPL-3.0-only): Explicitly documents that its `SpeedMeter`, `Inner`/`State` lock layout, and 250 ms ticker keep DR's `net/downloader.rs` (`L23`) structure around `epic::download::install`. **DL4b action:** Either leave `net/epic_install.rs` in the AGPL `src-tauri` crate or rewrite it cleanly from Section 3.8 as a fresh progress-reporting adapter over `epic::download::install` with no DR `SpeedMeter` copy.
2. **`platform/linux_runtime.rs`**, **`platform/updater.rs`**, **`core/i18n.rs`**, **`window.rs`**, **`platform/log.rs`**, **`platform/system.rs`**: Listed in DL4's target list, but originally ported in rows L14, L27, L29, L32, L33, L37 from DR at `e1c5fec` (`NOTICE.md` lines 101–103) and extended in RA10/RA11/DE2. **DL4b action:** If moved into the MIT crate, they must be rewritten from their clean-room specifications in Section 3.3 rather than moved verbatim with DR structure/comments; otherwise, keep the DR-derived portions in `src-tauri` and expose trait seams to the MIT crate.

### 1.2 Full 95-File Classification Table

| Path under `launcher/src-tauri/src/` | Category | Feature Gate | Target Crate in DL4b | Notes |
| --- | --- | --- | --- | --- |
| `main.rs` | AGPL DR Core | `server-gui` (branch) | `launcher` (`src-tauri`) | Binary entry point (`run_server_gui` / `run_client`). |
| `lib.rs` | AGPL DR Core | `host`, `server-gui`, `legacy` | `launcher` (`src-tauri`) | Tauri builder, URI protocols (`rr-art`), lifecycle hooks, single-instance. |
| `window.rs` | Born-MIT / DL4 Rewrite | `server-gui` | MIT crate (or `src-tauri` Tauri shell) | Main window builder (`dainty-lass-ui`), geometry clamping (`WindowBounds`). Rewritten per §3.3.4 if in MIT crate. |
| `server_gui.rs` | Born-MIT | `server-gui` | MIT crate | `EmbeddedServerState` implementing `EmbeddedServer` over `server_controller` + `service_kit` (RD26). |
| `server_gui/modules.rs` | Born-MIT | `server-gui` | MIT crate | `Panel` implementing `ModulePanel` over `server_controller::ModuleRegistry` and `host::hostconfig` (DU3, DF3). |
| `server_gui/modules/tests.rs` | Born-MIT | `server-gui`, `test` | MIT crate | Unit tests for embedded module panel. |
| `server_history.rs` | Born-MIT | `server-gui` | MIT crate | `query`, `dry_run`, `apply` over `admin::history` with `PlanCache` and actor `"server-gui"` (DD23, DH3b). |
| `bridge/mod.rs` | Born-MIT | — | MIT crate + `src-tauri` | Module root for bridge types, spec, modules, commands, backend, generate. |
| `bridge/spec.rs` | Born-MIT | `server-gui` | MIT crate | Authoritative `COMMANDS` (62), `EVENTS` (6), `Arg` shapes, limits, `serves()`, `arg_accepts()`. |
| `bridge/modules.rs` | Born-MIT | — | MIT crate | Pure validation/sanitization for module panels (`ModulePanel` trait, `view`, `command_args`, `merge_settings`, CSS colour check). |
| `bridge/commands.rs` | Born-MIT | `host`, `server-gui`, `realms`, `legacy` | `src-tauri` (thin `#[tauri::command]` wrappers) + MIT crate (validators) | Input validators (`host_setup_request`, `host_multipliers`, `invite_for`, `realm_id`, `history_args`, `guarded_id`, `planned`) move to MIT crate; `#[tauri::command]` fns delegate from `src-tauri`. |
| `bridge/generate.rs` | Born-MIT | `test` | MIT crate / `src-tauri` | Generates `gen/strings.json`, Tauri ACL permissions, TS `generated.ts`, and checks `dainty-lass-ui` parity. |
| `bridge/types.rs` | Shared Seam (MIT types) | `server-gui`, `realms`, `legacy` | MIT crate (re-exported by `src-tauri`) | Wire data transfer structs/enums for IPC commands and events (`Snapshot`, `HostKit`, `HostServerView`, `ErrorCode`, etc.). |
| `bridge/backend.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Tauri `Channel` state holder (`SNAPSHOT_EVENT`, `PROGRESS_EVENT`, `HOST_KIT_EVENT`, `HOST_SERVER_EVENT`). |
| `core/mod.rs` | AGPL DR Core | `legacy` | Both (module root) | Re-exports core submodules. |
| `core/i18n.rs` | Born-MIT / DL4 Rewrite | `server-gui` | MIT crate | `Language` lookup table (generated from `gen/strings.json`), `translate()`, `pick_system_language()`. Spec in §3.3.3. |
| `core/pins.rs` | Shared Constants | — | MIT crate (re-exported by `src-tauri`) | Pure constants (`WIN64_RELATIVE_DIR`, `EXE_NAME`, `EXE_RELATIVE_PATH`, `PINNED_EXE_SHA256`, `VC_RUNTIME_FILES`, `DISK_MARGIN_BYTES`). |
| `core/text.rs` | Shared Helpers | — | MIT crate (re-exported by `src-tauri`) | Re-exports / mirrors `protocol::text` (`is_js_space`, `js_trim`, `js_length`, `is_unsafe_char`). |
| `core/game_language.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | `GAME_LOCALES` and `-epiclocale` mapping (L16). |
| `core/invite.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Re-exports `protocol::invite` with shared test table runner (L11). |
| `core/status.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Re-exports `protocol::status` with shared test table runner (L12). |
| `core/username.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Re-exports `protocol::account` with shared test table runner (L12). |
| `core/legacy_pins.rs` | AGPL DR Core | `legacy` | `launcher` (`src-tauri`) | 1.4.4 pins and constants (L18). |
| `core/manifest.rs` | AGPL DR Core | `legacy` | `launcher` (`src-tauri`) | 1.4.4 compiled-in manifest loader (L17). |
| `core/manifest_check.rs` | AGPL DR Core | `legacy` | `launcher` (`src-tauri`) | 1.4.4 manifest validator (included by `build.rs` and `launcher_legacy` fuzz target). |
| `core/manifest_tests.rs` | AGPL DR Core | `legacy`, `test` | `launcher` (`src-tauri`) | Tests for 1.4.4 manifest validation. |
| `net/mod.rs` | AGPL DR Core | `legacy` | Both (module root) | Network layer root. |
| **`net/runtime_files.rs`** | **DL4 Rewrite (1/8)** | — | **MIT crate** | Runtime file verification, trust set IDs, atomic/aside replacement, download & local install. Spec in §2.1. |
| **`net/runtime_files/manifest.rs`** | **DL4 Rewrite (1/8)** | — | **MIT crate** | Runtime manifest fetch (`/content/v1/runtime/manifest`), local `.reforged-runtime.json` marker & directory inspection. Spec in §2.1. |
| **`net/runtime_files/tests.rs`** | **DL4 Rewrite (1/8)** | `test` | **MIT crate** | Unit and local HTTP server tests for runtime files. Spec in §2.1. |
| **`net/verify.rs`** | **DL4 Rewrite (2/8)** | `legacy` (for `mod install`) | **MIT crate** | `VerifyError`, `Abort`, `hash_file`, `mtime_ms`, `HashCache`, `ExeCheck`, `cached_exe_check`, `check_game_exe`. Spec in §2.2. |
| **`net/verify/cache_file.rs`** | **DL4 Rewrite (2/8)** | — | **MIT crate** | Pure parser/serializer for `verify-cache.json` (`MAX_ENTRIES = 32`). Spec in §2.2. |
| `net/verify/install.rs` | AGPL DR Core | `legacy` | `launcher` (`src-tauri`) | 1.4.4 full game manifest verifier (`verify_install`, L24). |
| `net/verify/tests.rs` | AGPL DR Core + DL4 Rewrite | `test` | Split: `HashCache` / `ExeCheck` tests in MIT crate; ` #[cfg(feature = "legacy")]` manifest tests in `src-tauri` | Tests for `verify`. |
| `net/regionping.rs` | Born-MIT | — | MIT crate | Region latency measurement (`fetch_nodes`, `measure`, `order`, `report`, `region_ping`, DN7). Spec in §3.9. |
| `net/regionping/parse.rs` | Born-MIT | — | MIT crate | Pure parser for `GET /network/v1/nodes` (`MAX_NODES = 16`, `is_region_id`, `read_nodes`); used by `launcher_region` fuzz target. |
| `net/regionping/harness.rs` | Born-MIT | `test` | MIT crate | Docker network harness test (`DN12`). |
| `net/regionping/tests.rs` | Born-MIT | `test` | MIT crate | Region ping unit and local TLS server tests. |
| `net/versions.rs` | Born-MIT | — | MIT crate | Game version list & Epic manifest size fetcher (`Sources`, `HttpSources`, `ManifestCache`, `list`, DE4). Spec in §3.10. |
| `net/versions/builtin.rs` | Born-MIT | — | MIT crate | Compiled-in `KnownProfile` table from `epic::builds::BUILDS` + `1.4.4` locate entry (DE4). |
| `net/versions/parse.rs` | Born-MIT | — | MIT crate | Pure parser/validator for `/content/v1/versions` and `/client` via `ModuleManifest::validate`. |
| `net/versions/tests.rs` | Born-MIT | `test` | MIT crate | Unit tests for version merging, pin enforcement, and manifest size caching. |
| `net/epic_install.rs` | Born-MIT / DL4 Rewrite | — | MIT crate (rewritten per §3.8) or `src-tauri` | Progress/control job wrapper (`InstallJob`) driving `epic::download::install` (DE5). |
| `net/connector.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | TCP/TLS stream connector (L19). |
| `net/pinned.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | SPKI/DER certificate fingerprint-pinned TLS handshake (L19). |
| `net/http.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Invite-scoped HTTP/1.1 client without redirects (L20). |
| `net/http/to_file.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | `request_to_file` streaming downloader (RA7; exposed via trait/callback seam to MIT `runtime_files`). |
| `net/hostapi.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | `/reforged/api` and `/content/v1` client calls (L21). |
| `net/hostapi/parse.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Parser for register, user info, runtime manifest, news, branding (`launcher_net` fuzz target). |
| `net/hostapi/tests.rs` | AGPL DR Core | `test` | `launcher` (`src-tauri`) | Tests for `hostapi`. |
| `net/relay.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Local loopback HTTP/WebSocket relay on `127.0.0.1:61100` and QoS listener (L22, RA6). |
| `net/relay/forwarding.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | HTTP request/response and WebSocket upgrade forwarding for local relay. |
| `net/relay/tests.rs` | AGPL DR Core | `test` | `launcher` (`src-tauri`) | Tests for local relay. |
| `net/relay_owner.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Windows TCP table owner PID check (`GetExtendedTcpTable`, L33). |
| `net/autoregion.rs` | AGPL DR Core | `legacy` | `launcher` (`src-tauri`) | 1.4.4 UDP hunt-region probe (L25). |
| `net/downloader.rs` | AGPL DR Core | `legacy` | `launcher` (`src-tauri`) | 1.4.4 multi-file range downloader (L23). |
| `net/downloader/tests.rs` | AGPL DR Core | `legacy`, `test` | `launcher` (`src-tauri`) | Tests for 1.4.4 downloader. |
| `platform/mod.rs` | AGPL DR Core | `host`, `legacy` | Both (module root) | Platform layer root. |
| **`platform/hostkit.rs`** | **DL4 Rewrite (3/8)** | `host` | **MIT crate** | Host workspace layout, toolchain detection (`vswhere`, MSBuild, Cargo), SDK import, Dumper-7 discovery, `run_logged`. Spec in §2.3. |
| **`platform/hostkit/parse.rs`** | **DL4 Rewrite (3/8)** | `host` | **MIT crate** | Pure parser for `reforged-host` output, `host.toml`/`host.json`, `vswhere`, invite links, multipliers (`launcher_host` fuzz target). Spec in §2.3. |
| **`platform/hostkit/tests.rs`** | **DL4 Rewrite (3/8)** | `host`, `test` | **MIT crate** | Tests for `hostkit` and `hostkit::parse`. Spec in §2.3. |
| **`platform/hostserver.rs`** | **DL4 Rewrite (4/8)** | `host` | **MIT crate** | `HostServer` state machine, `EmbeddedServer` trait, `reforged-host` child management, secret redaction on `invite`. Spec in §2.4. |
| **`platform/hostserver/tests.rs`** | **DL4 Rewrite (4/8)** | `host`, `test` | **MIT crate** | Fake host script & embedded server state machine tests. Spec in §2.4. |
| **`platform/requirements.rs`** | **DL4 Rewrite (5/8)** | `host` | **MIT crate** | Host requirements checker (`Dauntless`, `Mongodb`/`Postgres`, `Tailscale`, `VisualStudio`, `Rust`, `Dumper7`, `VcRedist`). Spec in §2.5. |
| **`platform/dialogs.rs`** | **DL4 Rewrite (6/8)** | `legacy` (for `Revived` variant) | **MIT crate** | Native file/folder pickers (`rfd` / `tauri_plugin_dialog`) and close confirmation (`confirm_keep_open`). Spec in §2.6. |
| `platform/loader_session.rs` | Born-MIT | — | MIT crate | RD9 `mystic_loader.ini` session writer, leftover detector, and Dumper-7 completion watcher. Spec in §3.2. |
| `platform/epic_login.rs` | Born-MIT | — | MIT crate | Incognito Epic sign-in webview (`epic-sign-in`), HTTPS-only `Watch` state machine, `epic-sign-in.invalid` handoff (DE2). Spec in §3.7. |
| `platform/epic_library.rs` | Born-MIT | — | MIT crate | Dauntless ownership check and cache wrapper over `epic::library::Ownership` (DE3). Spec in §3.7. |
| `platform/epic_session.rs` | Born-MIT | — | MIT crate | `Epic` session manager, token refresh (`REFRESH_MARGIN_SECONDS = 600`), sign-in/sign-out (DE2). Spec in §3.7. |
| `platform/keystore/epic.rs` | Born-MIT | — | MIT crate | Encrypted `keys/epic.session` persistence (`VERSION = 1`) and log secret registration (DE2). Spec in §3.7. |
| `platform/linux_runtime.rs` | Born-MIT / DL4 Rewrite | — | MIT crate | Linux Wine/Proton/ULWGL discovery and `PreparedLaunch` builder. Rewritten per §3.3.1. |
| `platform/updater.rs` | Born-MIT / DL4 Rewrite | — | MIT crate | Self-updater check (`check_for_update` with `TAURI_UPDATER_ENDPOINTS` / `TAURI_UPDATER_PUBKEY`). Rewritten per §3.3.2. |
| `platform/log.rs` | Born-MIT / DL4 Rewrite | — | MIT crate | Secret-redacting logger (`add_secret`, `remove_secret`, `redact`, 2 MiB rotation, `egs_api` mute filter). Rewritten per §3.3.5. |
| `platform/system.rs` | Born-MIT / DL4 Rewrite | — | MIT crate | Tailscale binary finder, `free_bytes`, and `DeviceSignals` normalization. Rewritten per §3.3.6. |
| `platform/art.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | `rr-art://bg/<id>` custom protocol store (L36). |
| `platform/clipboard.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Clipboard write (`COPY_TEXT_MAX = 4096`) and invite link reader (L36). |
| `platform/client_mods.rs` | AGPL DR Core | `legacy` | `launcher` (`src-tauri`) | 1.4.4 client DLL and mod manager (L30). |
| `platform/game_config.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | `GameUserSettings.ini` and `Engine.ini` writer (L30). |
| `platform/game_config_tests.rs` | AGPL DR Core | `test` | `launcher` (`src-tauri`) | Tests for `game_config`. |
| `platform/game_folder.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Game folder layout resolution, `Version.txt` check, Epic/Legendary/Heroic store discovery (L30, RA9). |
| `platform/invite_link.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Deep-link (`dauntless://`) argv extraction and OS scheme registration (L37). |
| `platform/keystore.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | DPAPI / Secret Service account key store and Documents backup (L26, RA8). |
| `platform/keystore/dpapi.rs` | AGPL DR Core | `windows` | `launcher` (`src-tauri`) | Windows `CryptProtectData` / `CryptUnprotectData` wrapper. |
| `platform/keystore/format.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Key file & Electron migration framing (`launcher_files` fuzz target). |
| `platform/keystore/secret_service.rs` | AGPL DR Core | `target_os = "linux"` | `launcher` (`src-tauri`) | Linux `secret-tool` key encryption wrapper. |
| `platform/keystore/tests.rs` | AGPL DR Core | `test` | `launcher` (`src-tauri`) | Tests for account keystore. |
| `platform/launch.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Game process spawner, command-line builder, `mask_args`, process tree killer (L31, RA9). |
| `platform/links.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Fixed `ExternalTarget` URL table and `open` crate wrapper (L35, RA11). |
| `platform/paths.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | `AppPaths` (`userData`, `documents`, `home`, Electron migration, L27). |
| `platform/settings.rs` | AGPL DR Core | `legacy` | `launcher` (`src-tauri`) | `settings.json` sanitizer and serializer (`launcher_files` fuzz target, L28). |
| `controller/mod.rs` | AGPL DR Core | `host`, `server-gui`, `realms`, `legacy` | `launcher` (`src-tauri`) | Core `Controller` state machine: join, connect, account, status, news, settings, play, shutdown (L38, RA13). |
| `controller/game.rs` | AGPL DR Core | — | `launcher` (`src-tauri`) | Game folder inspection, executable hash verification orchestration, store detection (`choose_game_folder`, `use_detected_game`, `verify_game`, RA13). |
| **`controller/host.rs`** | **DL4 Rewrite (7/8)** | `host` | **MIT crate** | Host tools controller state & actions (`HostState`, `host_attach`, `host_action`, `host_setup`, `host_create_invite`, `host_update`, `host_set_multipliers`, server start/stop/restart, RD9 dump/export sessions). Spec in §2.7. |
| **`controller/host_off.rs`** | **DL4 Rewrite (7/8)** | `not(host)` | **MIT crate** | Player-only build stub for Host tools (`ErrorCode::HostFailed` with detail `"host_tools_unavailable"`). Spec in §2.7. |
| **`controller/runtime.rs`** | **DL4 Rewrite (8/8)** | — | **MIT crate** | Server runtime offer inspection, trust acceptance, download/install, and local runtime folder selection (`refresh_runtime_offer`, `inspect_runtime`, `trust_runtime`, `install_runtime`, `choose_local_runtime`, `clear_local_runtime`). Spec in §2.8. |
| `controller/loader.rs` | Born-MIT | — | MIT crate | RD9 `LoaderSession` lifecycle methods on controller (`loader_leftovers`, `clean_loader_session`, `begin_loader_session`, `end_loader_session`). Spec in §3.4. |
| `controller/guarded.rs` | Born-MIT | `server-gui` | MIT crate | Guarded admin action list/approve/cancel (`guarded_list`, `guarded_decide`, DD23, DH6). Spec in §3.5. |
| `controller/install.rs` | Born-MIT | — | MIT crate | Epic version installer (`EpicGate`, `Installs`, `game_install_start/pause/resume/cancel/get`, DE5). Spec in §3.8. |
| `controller/realms.rs` | Born-MIT | `realms` | MIT crate | Local realm hosting and joining over `rustic-rifts-host` (`RealmsState`, `realm_host`, `realm_join`, `realm_stop`, `realm_get`, RF7, DU5). Spec in §3.6. |
| `controller/realms_off.rs` | Born-MIT | `not(realms)` | MIT crate | Stub returning `ErrorCode::NotInThisBuild` when `realms` is disabled. Spec in §3.6. |
| `controller/region.rs` | Born-MIT | — | MIT crate | `region_ping` controller entry point (DN7). Spec in §3.9. |
| `controller/versions.rs` | Born-MIT | — | MIT crate | `game_version_list` and `joined_servers` collector (DE4). Spec in §3.10. |
| `controller/platform.rs` | AGPL DR Core | `host` | `launcher` (`src-tauri`) + trait in MIT crate | `Platform` trait and `TauriPlatform` implementation. |
| `controller/stored.rs` | AGPL DR Core | `legacy` | `launcher` (`src-tauri`) + view traits in MIT crate | Typed `Stored` and `StoredServer` over `settings.json`. |
| `controller/legacy.rs` | AGPL DR Core | `legacy` | `launcher` (`src-tauri`) | 1.4.4 controller flow (download, verify, mods, launch). |
| `controller/legacy_off.rs` | AGPL DR Core | `not(legacy)` | `launcher` (`src-tauri`) | Stubs when `legacy` feature is off. |
| `controller/tests.rs` | AGPL DR Core + DL4 Rewrite | `test` | Split between MIT crate and `src-tauri` | Controller unit and integration tests. |
| `controller/tests/host.rs` | **DL4 Rewrite** | `host`, `test` | **MIT crate** | Host controller unit/integration tests. |
| `controller/tests/legacy.rs` | AGPL DR Core | `legacy`, `test` | `launcher` (`src-tauri`) | 1.4.4 controller tests. |
| `controller/tests/realms.rs` | Born-MIT | `realms`, `test` | MIT crate | Realms controller tests. |

### 1.3 Crate Seam Between the MIT Crate and AGPL `src-tauri`

To allow the MIT crate (e.g. `launcher-kit`, `license = "MIT"`) to compile without depending on the AGPL `launcher` (`src-tauri`) crate:

1. **Direction of dependency**: `launcher` (`src-tauri`, AGPL-3.0-only) depends on `launcher-kit` (MIT), `protocol` (MIT), `epic` (MIT), `module-api` (MIT), `service-kit` (MIT), `server-controller` (MIT), `host` (MIT), `admin` (MIT), `rustic-rifts-host` (MIT), `rustic-rifts-seam` (MIT). `launcher-kit` **never** depends on `launcher`.
2. **Shared Wire Types & Pins**: `bridge::types`, `bridge::spec`, `bridge::modules`, `core::pins`, `core::text`, and `core::i18n` live in `launcher-kit` (and are re-exported by `launcher::bridge::*` and `launcher::core::*` so existing imports in `src-tauri` continue to work).
3. **HTTP / Host-API Seam for `net::runtime_files`, `net::regionping`, and `net::versions`**:
   - Define a lightweight MIT transport trait or struct in `launcher-kit` (or pass callbacks/trait objects implemented by `src-tauri`'s `net::http`, `net::http::to_file`, `net::pinned`, and `net::hostapi`):
     - `Endpoint { pub host: String, pub port: u16, pub pin: Option<String> }`
     - `HttpTransport` trait providing:
       - `get_json(&self, endpoint: &Endpoint, path: &str, timeout: Duration, max_bytes: usize) -> Future<Output = Result<(u16, Option<Value>), TransportError>>`
       - `request_bytes(&self, endpoint: &Endpoint, path: &str, options: TransportRequest) -> Future<Output = Result<(u16, Bytes), TransportError>>`
       - `download_to_file(&self, endpoint: &Endpoint, path: &str, target: &Path, options: FileDownloadOptions<'_>) -> Future<Output = Result<u16, FileDownloadError>>`
       - `connect_pinned(&self, host: &str, port: u16, fingerprint: &str, timeout: Duration) -> Future<Output = Result<PinnedStream, ConnectProblem>>`
       - `key_headers(&self, endpoint: &Endpoint, key: Option<&str>) -> Vec<(&'static str, String)>`
4. **Controller Accessor Seam (`ControllerContext` / `ControllerAccess` trait)**:
   - The controller sub-modules in `launcher-kit` (`controller::host`, `controller::runtime`, `controller::loader`, `controller::guarded`, `controller::install`, `controller::realms`, `controller::region`, `controller::versions`) either operate on their own state structs (`HostState`, `RealmsState`, `Installs`) plus a trait `ControllerAccess` implemented by `src-tauri`'s `Controller`, or `src-tauri`'s `Controller` delegates thin one-line wrapper methods to functions/structs in `launcher-kit`.

---

## 2. Clean-Room Specification of the 8 DL4 Rewrite Modules

### 2.1 Module 1: `net::runtime_files` (`runtime_files.rs`, `runtime_files/manifest.rs`, `runtime_files/tests.rs`)

Manages inspection, trust calculation, manifest parsing, downloading, atomic/in-use replacement, and local developer copying of the two game runtime files (`Rustic-Isles.dll` and `winmm.dll`) inside `<game_root>/Archon/Binaries/Win64`.

#### Constants
- `pub const RUNTIME_MARKER_FILE: &str = ".reforged-runtime.json"` (re-exported from `manifest`)
- `pub const DOWNLOAD_SUFFIX: &str = ".rr-download"`
- `pub const ASIDE_MARK: &str = ".rr-old-"`
- `const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(30)`
- `const MANIFEST_TIMEOUT: Duration = Duration::from_secs(10)`
- `const MANIFEST_MAX_BYTES: usize = 64 * 1024`
- `const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1` (`9_007_199_254_740_991`)
- From `protocol::manifest`: `RUNTIME_FILE_NAMES = ["Rustic-Isles.dll", "winmm.dll"]`, `RUNTIME_FILES_BASE_PATH = "/content/v1/runtime/files"`, `RUNTIME_MANIFEST_PATH = "/content/v1/runtime/manifest"`, `DEFAULT_CHANGELIST = 647_472`, `MAX_RUNTIME_FILE_BYTES = 64 * 1024 * 1024`, ` RuntimeFile`, `RuntimeManifest`.

#### Public Types & Signatures

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeManifestResult {
    Ok(RuntimeManifest),
    /// Server returned HTTP 404 (no runtime offered).
    None,
    /// Non-200/404 status, invalid JSON, or failed manifest schema check.
    BadAnswer,
    /// TLS certificate fingerprint did not match the endpoint's pin.
    CertMismatch,
    /// Network timeout or connection failure.
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeErrorCode {
    KeyRejected,
    NotFound,
    Different,
    DownloadFailed,
    CertMismatch,
    DiskError,
    InUse,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    pub code: RuntimeErrorCode,
    pub file: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProgress {
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub files_done: usize,
    pub files_total: usize,
    pub current_file: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeInstallResult {
    pub replaced_in_use: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replaced {
    Replaced,
    InUse,
}

pub trait ReplaceOps {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn remove(&self, file: &Path) -> std::io::Result<()>;
}

pub struct DiskOps;

pub struct InstallRequest<'a> {
    pub endpoint: &'a Endpoint,
    pub headers: Vec<(&'static str, String)>,
    pub manifest: &'a RuntimeManifest,
    pub win64_dir: &'a Path,
    pub cancel: Option<CancellationToken>,
    pub on_progress: Option<&'a (dyn Fn(RuntimeProgress) + Send + Sync)>,
}

pub fn file_infos(files: &[RuntimeFile]) -> Vec<RuntimeFileInfo>;
pub fn runtime_set_id(files: &[RuntimeFile]) -> String;
pub fn is_trusted(trusted: &[String], files: &[RuntimeFile]) -> bool;
pub fn accept(trusted: &mut Vec<String>, files: &[RuntimeFile]) -> String;

pub fn parse_runtime_manifest(raw: &Value) -> Option<RuntimeManifest>;
pub async fn fetch_runtime_manifest(endpoint: &Endpoint) -> RuntimeManifestResult;
pub async fn installed_runtime(win64_dir: &Path) -> Vec<(&'static str, Option<String>)>;
pub async fn is_complete_runtime_dir(dir: &Path) -> bool;
pub async fn check_runtime(
    win64_dir: &Path,
    offered: Option<&RuntimeManifest>,
    trusted: bool,
    local_dir: Option<&Path>,
) -> (RuntimeState, bool);

pub async fn unblock(file: &Path);
pub fn replace_file(tmp: &Path, target: &Path, ops: &dyn ReplaceOps) -> std::io::Result<Replaced>;
pub fn sweep_aside(win64_dir: &Path);
pub async fn install_runtime(request: InstallRequest<'_>) -> Result<RuntimeInstallResult, RuntimeError>;
pub async fn install_local_runtime(local_dir: &Path, win64_dir: &Path) -> Result<RuntimeInstallResult, RuntimeError>;
```

#### Behavioural Invariants
1. **`file_infos`**: Maps each `RuntimeFile { name, size, sha256 }` into `bridge::types::RuntimeFileInfo { name, size, sha256 }`.
2. **`runtime_set_id`**:
   - Clones `files` and sorts them in ascending UTF-16 code-unit order of `name` (`a.name.encode_utf16().cmp(b.name.encode_utf16())`).
   - Formats each entry as `"{name}:{size}:{sha256}"` (with `sha256` lowercased), joins them with `"\n"`, and returns the 64-character lowercase hex SHA-256 of the UTF-8 string.
3. **`is_trusted` & `accept`**:
   - `is_trusted(trusted, files)` returns `!files.is_empty()` and `trusted.contains(&runtime_set_id(files))`.
   - `accept(trusted, files)` computes `id = runtime_set_id(files)`, removes any existing entries equal to `id`, appends `id` at the end, and if `trusted.len() > 8`, removes the oldest leading entries so at most 8 remain. Returns `id`.
4. **`parse_runtime_manifest`**:
   - Expects a JSON object.
   - `version`: required string cleaned via `clean_text(v, 48)` (trimmed, non-empty, `<= 48` UTF-16 units, no unsafe/control chars).
   - `files`: required JSON array whose length equals `RUNTIME_FILE_NAMES.len()` (2). For each entry:
     - `name` must be in `RUNTIME_FILE_NAMES` and must not duplicate an earlier entry in the array.
     - `sha256` must be a 64-character lowercase ASCII hex string (`0-9a-f`).
     - `size` must be an integer JSON number (`as_u64()`) in `1..=MAX_RUNTIME_FILE_BYTES` (`64 MiB`).
   - All names in `RUNTIME_FILE_NAMES` must be present; the resulting `Vec<RuntimeFile>` is sorted by `name` in UTF-16 code-unit order.
   - `changelist`: reads `raw["build"]["changelist"]`: if that field is a JSON integer in `1..=MAX_SAFE_INTEGER`, uses `u32::try_from(n).ok()?`; otherwise defaults to `DEFAULT_CHANGELIST` (`647_472`).
5. **`fetch_runtime_manifest`**:
   - Sends `GET /content/v1/runtime/manifest` with a 10 s timeout and 64 KiB body cap, without an account key (public metadata).
   - Maps TLS pin mismatch to `RuntimeManifestResult::CertMismatch`, other transport errors to `Unreachable`, HTTP 404 to `None`, HTTP 200 to `parse_runtime_manifest` (`Ok(m)` or `BadAnswer`), and any other status to `BadAnswer`.
6. **`installed_runtime` & `is_complete_runtime_dir`**:
   - `installed_runtime(win64_dir)`: for each name in `RUNTIME_FILE_NAMES`, hashes `<win64_dir>/<name>` via `hash_file(..., Abort::new(), ...)`, returning `(name, Some(sha256))` if the file exists, is non-empty, and hashes cleanly, or `(name, None)` if missing/empty/unreadable.
   - `is_complete_runtime_dir(dir)`: returns `true` iff `installed_runtime(dir).await.iter().all(|(_, hash)| hash.is_some())`.
7. **`check_runtime(win64_dir, offered, trusted, local_dir)`**:
   - Returns `(RuntimeState, bool)` where the boolean indicates whether the runtime files are playable right now:
   - If `local_dir` is `Some(dir)`: returns `(RuntimeState::Local, is_complete_runtime_dir(dir).await)`.
   - Otherwise, inspects `installed = installed_runtime(win64_dir).await` and `present = installed.iter().filter(|(_, hash)| hash.is_some()).count()`.
   - If `offered` is `None`:
     - If `present == RUNTIME_FILE_NAMES.len()` -> `(RuntimeState::Ok, true)`
     - Else -> `(RuntimeState::Unavailable, false)`
   - If `offered` is `Some(manifest)`:
     - `matches = manifest.files.iter().all(|f| installed.iter().any(|(n, h)| *n == f.name && h.as_deref() == Some(f.sha256.as_str())))`
     - If `matches` -> `(RuntimeState::Ok, true)`
     - Else if `!trusted` -> `(RuntimeState::Untrusted, false)`
     - Else if `present == 0` -> `(RuntimeState::Missing, false)`
     - Else -> `(RuntimeState::Outdated, false)`
8. **`replace_file` & `sweep_aside` & `unblock`**:
   - `replace_file(tmp, target, ops)`:
     - Calls `ops.rename(tmp, target)`. If `Ok(())`, returns `Ok(Replaced::Replaced)`.
     - If rename fails (e.g. `target` is mapped into a running game process on Windows), constructs `aside = <target>.rr-old-<epoch_millis>`.
     - Calls `ops.rename(target, &aside)` (propagating any error).
     - Calls `ops.rename(tmp, target)`. If this second rename fails, attempts `let _ = ops.rename(&aside, target);` to restore the original file and returns the error.
     - If the second rename succeeds, tries `let _ = ops.remove(&aside);` and returns `Ok(Replaced::InUse)`.
   - `sweep_aside(win64_dir)`: reads directory entries of `win64_dir` and best-effort removes any entry whose file name starts with `<runtime_file>.rr-old-` or equals `<runtime_file>.rr-download` for any `<runtime_file>` in `RUNTIME_FILE_NAMES`.
   - `unblock(file)`: on Windows (`#[cfg(windows)]`), best-effort removes the NTFS Alternate Data Stream `<file>:Zone.Identifier`; no-op on non-Windows.
9. **`install_runtime`**:
   - Verifies that `win64_dir` exists and is a directory, returning `RuntimeErrorCode::DiskError` if missing/not a directory.
   - Calls `sweep_aside(win64_dir).await`.
   - Inspects `current = installed_runtime(win64_dir).await` and filters `manifest.files` to `needed`: files whose current on-disk SHA-256 does not already match `file.sha256`.
   - Computes `total_bytes = needed.iter().map(|f| f.size).sum()`, `files_total = needed.len() as u32`.
   - Checks `free_bytes(win64_dir)`: if `Some(free)` and `free < total_bytes.saturating_add(DISK_MARGIN_BYTES)`, fails with `RuntimeErrorCode::DiskError` (`"Not enough disk space"`).
   - Emits initial `RuntimeProgress { done_bytes: 0, total_bytes, files_done: 0, files_total, current_file: None }`.
   - Builds key headers via `key_headers(endpoint, Some(key))`.
   - For each `file` in `&needed`:
     - Emits progress with `current_file: Some(file.name.clone())`.
     - Downloads `{RUNTIME_FILES_BASE_PATH}/{file.name}` to `<win64_dir>/<file.name>.rr-download` with `idle_timeout = 30s`, `max_bytes = file.size`, `cancel`, and `on_bytes` callback that emits updated `done_bytes = completed_bytes + file_bytes`.
     - Error mapping on download:
       - `Aborted` -> `RuntimeErrorCode::Cancelled`
       - `Pin` -> `RuntimeErrorCode::CertMismatch`
       - `TooLarge` -> `RuntimeErrorCode::Different`
       - `Disk` -> `RuntimeErrorCode::DiskError`
       - `Http(_)` -> `RuntimeErrorCode::DownloadFailed`
     - Status code mapping if not 200:
       - `401 | 403` -> `RuntimeErrorCode::KeyRejected`
       - `404` -> `RuntimeErrorCode::NotFound`
       - `503` -> `RuntimeErrorCode::Different`
       - Other -> `RuntimeErrorCode::DownloadFailed`
     - Hashes `<win64_dir>/<file.name>.rr-download` and checks metadata length:
       - If cancelled -> removes temp file, returns `Cancelled`.
       - If size != `file.size` or `sha256 != file.sha256` -> removes temp file, returns `RuntimeErrorCode::Different`.
     - Calls `unblock(&tmp)`.
     - Calls `replace_file(&tmp, &target, &mut DiskOps)`:
       - On error -> removes temp file, returns `RuntimeErrorCode::InUse` if `PermissionDenied`, else `RuntimeErrorCode::DiskError`.
       - On `Replaced::InUse` -> pushes `file.name` to `replaced_in_use`.
     - Advances `completed_bytes += file.size`, `files_done += 1`, and emits progress.
   - Writes `<win64_dir>/.reforged-runtime.json` with `{"version": manifest.version, "source": format!("{}:{}", endpoint.host, endpoint.port), "installedAt": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true), "files": { ... }}` (2-space pretty JSON).
10. **`install_local_runtime(local_dir, win64_dir)`**:
    - Calls `sweep_aside(win64_dir).await`.
    - Verifies every file in `RUNTIME_FILE_NAMES` exists in `local_dir` with `len() > 0` (else `RuntimeErrorCode::NotFound`).
    - For each name in `RUNTIME_FILE_NAMES`: copies `<local_dir>/<name>` to `<win64_dir>/<name>.rr-download`, calls `unblock(&tmp)`, and calls `replace_file(&tmp, &target, &mut DiskOps)` (mapping `PermissionDenied` to `InUse`, other I/O errors to `DiskError`).
    - Writes `<win64_dir>/.reforged-runtime.json` with `{"version": "local", "source": local_dir.to_string_lossy(), "installedAt": ...}`.

#### Test Expectations
- `set_id_and_trust_follow_the_electron_launcher`: verifies UTF-16 sort order, SHA-256 digest of `{name}:{size}:{sha256}` lines, and 8-entry FIFO cap in `accept`.
- `manifest_parses_only_the_two_runtime_files`: rejects missing/extra/unknown files, uppercase hashes, zero/oversized files, control chars in version, and defaults `changelist` to `647472` when `build.changelist` is absent or out of range.
- `check_runtime_states`: verifies `Missing`, `Untrusted`, `Outdated`, `UpToDate`, `Installed` (with `.reforged-runtime.json` when server offers none), and `Local`.
- `replace_file_moves_a_locked_dll_aside_and_puts_it_back_if_the_second_rename_fails`: tests `ReplaceOps` double for normal rename, first-rename failure + aside rename + second-rename success (`Replaced::InUse`), and second-rename failure rollback.
- `install_runtime_downloads_checks_replaces_and_writes_the_marker`: tests against a local HTTP server that an already-matching file is skipped, the remaining file is downloaded with the account key header, verified, replaced, and recorded in `.reforged-runtime.json`.
- `install_runtime_refuses_a_wrong_hash_a_401_and_a_wrong_pin`: verifies `Different`, `KeyRejected`, and `CertMismatch` outcomes and confirms no `.rr-download` file is left behind.

---

### 2.2 Module 2: `net::verify` (`verify.rs`, `verify/cache_file.rs`)

Provides streaming SHA-256 file hashing with cancellation, mtime/size-keyed persistent hash caching (`verify-cache.json`), and pinned game executable verification. (Note: `net/verify/install.rs` is the `#[cfg(feature = "legacy")]` 1.4.4 manifest verifier and remains in the AGPL `src-tauri` crate.)

#### Constants & Types

```rust
// verify/cache_file.rs
pub const MAX_ENTRIES: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct CacheEntry {
    pub size: u64,
    pub mtime_ms: f64,
    pub sha256: String,
}

pub fn parse_cache_file(raw: &str) -> Vec<(String, CacheEntry)>;
pub fn write_cache_file(entries: &[(String, CacheEntry)]) -> String;

// verify.rs
pub const EXE_PIN: &str = crate::core::pins::PINNED_EXE_SHA256;

#[derive(Debug)]
pub enum VerifyError {
    Aborted,
    Io(std::io::Error),
}

#[derive(Clone, Default)]
pub struct Abort(Arc<AtomicBool>);

impl Abort {
    pub fn new() -> Self;
    pub fn abort(&self);
    pub fn aborted(&self) -> bool;
}

pub struct HashCache;

impl HashCache {
    pub fn load(file: PathBuf) -> Self;
    pub fn cached(&self, file: &Path) -> Option<String>;
    pub async fn hash(
        &mut self,
        file: &Path,
        abort: Abort,
        on_bytes: Arc<dyn Fn(u64) + Send + Sync>,
    ) -> Result<String, VerifyError>;
    pub async fn save(&mut self) -> std::io::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExeCheck {
    Pinned,
    OtherBuild,
    Missing,
}

pub fn mtime_ms(metadata: &std::fs::Metadata) -> f64;
pub async fn hash_file(
    file: PathBuf,
    abort: Abort,
    on_bytes: Arc<dyn Fn(u64) + Send + Sync>,
) -> Result<String, VerifyError>;
pub fn cached_exe_check(cache: &HashCache, exe: &Path, expected_sha256: &str) -> Option<ExeCheck>;
pub async fn check_game_exe(
    cache: &mut HashCache,
    exe: &Path,
    expected_sha256: &str,
    abort: Abort,
    on_bytes: Arc<dyn Fn(u64) + Send + Sync>,
) -> Result<ExeCheck, VerifyError>;
```

#### Behavioural Invariants
1. **`parse_cache_file` & `write_cache_file`**:
   - JSON format: `{"files": {"<path>": {"size": <u64>, "mtimeMs": <f64>, "sha256": "<64 lowercase hex>"}}}` pretty-printed with 2-space indent.
   - `parse_cache_file` rejects non-empty keys failing `size` (must be finite integer `>= 0` and `<= u64::MAX`), `mtimeMs` (must be finite `> 0.0`), or `sha256` (must be 64 lowercase ASCII hex chars).
   - If more than `MAX_ENTRIES` (32) valid entries exist, keeps the last 32 entries.
2. **`mtime_ms`**:
   - Converts `metadata.modified()` duration since `UNIX_EPOCH` into milliseconds (`as_secs_f64() * 1000.0`), or `0.0` on error.
3. **`hash_file`**:
   - Runs on `tokio::task::spawn_blocking`. Reads the file in `1 MiB` (`1024 * 1024`) chunks, checking `abort.aborted()` before each read and returning `Err(VerifyError::Aborted)` if set.
   - Calls `on_bytes(n as u64)` after each non-empty read and returns the 64-character lowercase hex SHA-256 digest.
4. **`HashCache`**:
   - `cached(file)`: checks that `file` exists as a regular file with `mtime_ms > 0.0`, and returns the cached `sha256` iff a cached entry for `file` has matching byte size and `|cached_mtime_ms - mtime_ms| < 1.0`.
   - `hash(file, abort, on_bytes)`: returns `Ok(sha256)` immediately if `self.cached(file)` hits. Otherwise inspects metadata before and after hashing via `hash_file`; only records the entry in the cache (trimming oldest entries past `MAX_ENTRIES = 32`) if the file size and mtime (`|after_mtime_ms - before_mtime_ms| < 1.0`, `mtime_ms > 0.0`) were unchanged during hashing (so a file modified while being hashed is never cached).
   - `save()`: no-op if no entries changed since load/save; otherwise creates parent dirs, writes `write_cache_file` to `<path>.tmp`, and atomically renames `<path>.tmp` to `<path>`.
5. **`cached_exe_check` & `check_game_exe`**:
   - `cached_exe_check(cache, exe, expected_sha256)`: if `!exe.is_file()`, returns `Some(ExeCheck::Missing)`. Otherwise maps `cache.cached(exe)` to `Some(ExeCheck::Pinned)` if `sha256.eq_ignore_ascii_case(expected_sha256)`, `Some(ExeCheck::OtherBuild)` if different, or `None` on cache miss.
   - `check_game_exe(cache, exe, expected_sha256, abort, on_bytes)`: if `!exe.is_file()`, returns `Ok(ExeCheck::Missing)`. Otherwise hashes via `cache.hash(exe, abort, on_bytes).await` (mapping `ErrorKind::NotFound` to `Ok(ExeCheck::Missing)`) and compares case-insensitively with `expected_sha256`.

#### Test Expectations
- Cache round-trip test: `write_cache_file` -> `parse_cache_file` preserves entries, evicts entries beyond 32, and drops malformed entries.
- `HashCache` hit/invalidation test: second `hash` call does not re-read bytes (`on_bytes` not called); changing file size or mtime invalidates the cached digest.
- `check_game_exe` test: returns `Missing` for absent file, `Pinned` when SHA-256 matches `EXE_PIN`, and `OtherBuild` when bytes differ.

---

### 2.3 Module 3: `platform::hostkit` (`hostkit.rs`, `hostkit/parse.rs`, `hostkit/tests.rs`)

Inspects a Dainty Lass checkout ("workspace"), discovers the host toolchain (Visual Studio / MSBuild via `vswhere.exe` and Rust `cargo`), scans Dumper-7 outputs, imports CppSDK headers/functions into `ParadoxRuntime`, reads `.workspace/config/host.toml` (or legacy `host.json`), and runs logged build/host commands.

#### Constants
- `pub const RUNTIME_FILE_NAMES: [&str; 2] = ["Rustic-Isles.dll", "winmm.dll"]`
- `pub const BACKEND_GAME_DATA: [&str; 7] = ["progression_config.json", "slayers_path.json", "slayers_path_definitions.json", "ladyluck_store.json", "platinum_store.json", "reward_cache_store.json", "inventory_storage_policy.json"]`
- `pub const DIRECTOR_GAME_DATA: [&str; 7] = ["player_hunts_table.json", "matchmaker_hunts_table.json", "arena_easy_matchmaker_hunts.json", "arena_hard_matchmaker_hunts.json", "arena_elite_matchmaker_hunts.json", "arena_hard_matchmaker_hunts_new.json", "arena_elite_matchmaker_hunts_new.json"]`
- `pub const EXPORT_FLAGS: &str = "# Written by the Dainty Lass launcher (Host page). See docs/sdk-and-game-data.md.\r\nEXPORT_CATALOG=1\r\nEXPORT_SLAYERS_PATH=1\r\nEXPORT_HUNTS=1\r\nEXPORT_STORE_RUNTIME=1\r\nEXPORT_PROGRESSION=1\r\nEXPORT_COMBAT=0\r\nEXPORT_SKINS=0\r\nEXPORT_SKINS_RESOLVE_STRINGTABLE=0\r\nEXPORT_WEAPON_SLOTS=0\r\nEXPORT_TABLE_INVENTORY=0\r\nEXPORT_CELLS=0\r\nEXPORT_DROP_TABLES=0\r\nEXPORT_CURRENCY_NAMES=0\r\n"`
- `pub const DUMPER_OUTPUT_ROOT: &str = "C:\\Dumper-7"`
- `pub const HOST_BINARY: &str = "reforged-host.exe"` (Windows) / `"reforged-host"` (non-Windows)
- `pub const MAX_MULTIPLIER: f64 = 100.0` (in `parse.rs`)
- `pub const INVITE_LINK_PREFIXES: [&str; 3] = ["dauntless://", "reforged-revival://", "dauntless-revived://"]` (in `parse.rs`)

#### Public Types & Signatures

```rust
// hostkit/parse.rs (pure parser: serde_json + toml_edit only; compiled directly into fuzz target `launcher_host`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteLine {
    pub link: String,
    pub for_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheck {
    pub behind: u64,
    pub head: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateDone {
    pub to: String,
    pub files: u32,
    pub runtime_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerMode {
    Private,
    Public,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    pub name: Option<String>,
    pub mode: ServerMode,
    pub host: Option<String>,
    pub multipliers: Option<(f64, f64)>,
}

pub fn cut(text: &str, max: usize) -> String;
pub fn clean_line(raw: &str) -> String;
pub fn failure_line(lines: &[String]) -> Option<String>;
pub fn last_json(lines: &[String]) -> Option<serde_json::Map<String, Value>>;
pub fn invite_line(lines: &[String]) -> Option<InviteLine>;
pub fn update_check(json: &serde_json::Map<String, Value>) -> Option<UpdateCheck>;
pub fn update_done(json: &serde_json::Map<String, Value>) -> Option<UpdateDone>;
pub fn update_failure(json: Option<&serde_json::Map<String, Value>>, fallback: Option<&str>) -> String;
pub fn published_version(lines: &[String]) -> Option<String>;
pub fn pack_written(lines: &[String]) -> Option<String>;
pub fn server_config(raw: &str) -> Option<ServerConfig>;
pub fn server_config_toml(raw: &str) -> Option<ServerConfig>;
pub fn vswhere_install(raw: &str) -> Option<(String, Option<u32>)>;
pub fn vswhere_msbuild(raw: &str) -> Option<String>;
pub fn platform_toolset(vs_major: Option<u32>) -> &'static str;

// hostkit.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePaths {
    pub root: PathBuf,
    pub runtime_source: PathBuf,
    pub sdk_header: PathBuf,
    pub sdk_dir: PathBuf,
    pub runtime_solution: PathBuf,
    pub runtime_dll: PathBuf,
    pub loader_dir: PathBuf,
    pub loader_dll: PathBuf,
    pub exporter_project: PathBuf,
    pub exporter_dll: PathBuf,
    pub backend_game_data: PathBuf,
    pub director_game_data: PathBuf,
    pub exports: PathBuf,
    pub publish: PathBuf,
    pub logs: PathBuf,
    pub host_toml: PathBuf,
    pub host_json: PathBuf,
    pub gamedata_packs: PathBuf,
}

impl WorkspacePaths {
    pub fn new(root: &Path) -> Self;
    pub fn launcher_log(&self, name: &str) -> PathBuf;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Toolchain {
    pub visual_studio: Option<String>,
    pub major_version: Option<u32>,
    pub msbuild: Option<PathBuf>,
    pub cargo: Option<PathBuf>,
}

impl Toolchain {
    pub fn view(&self) -> HostToolchain;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdkError {
    NoHeader,
    NoRuntimeSource,
    NoFunctions,
    Io,
}

impl SdkError {
    pub fn code(self) -> &'static str;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logged {
    pub code: i32,
    pub last_line: Option<String>,
}

pub fn find_host_binary(root: &Path) -> Option<PathBuf>;
pub fn is_workspace(root: &Path) -> bool;
pub fn count_function_files(sdk_dir: &Path) -> u32;
pub fn detect_game_data(dir: &Path, files: &[&str]) -> GameDataState;
pub fn find_dumps(root: &Path) -> Vec<HostDump>;
pub async fn exec_text(program: &Path, args: &[&str]) -> Option<String>;
pub fn vswhere_path(env: &dyn Fn(&str) -> Option<OsString>) -> PathBuf;
pub fn cargo_candidates(env: &dyn Fn(&str) -> Option<OsString>) -> Vec<PathBuf>;
pub fn system_env(name: &str) -> Option<OsString>;
pub async fn find_toolchain() -> Toolchain;
pub async fn any_visual_studio() -> Option<String>;
pub fn msbuild_args(project: &Path, toolset: &str) -> Vec<String>;
pub fn read_server_config(root: &Path) -> HostServerConfig;
pub fn detect_host_kit(workspace: Option<&Path>, toolchain: &Toolchain, dump_root: &Path) -> HostKit;
pub fn copy_tree(from: &Path, to: &Path) -> std::io::Result<u32>;
pub fn import_sdk(dump_dir: &Path, runtime_source: &Path) -> Result<u32, SdkError>;
pub async fn run_logged(program: &Path, args: &[String], cwd: &Path, log_file: &Path) -> Logged;
```

#### Behavioural Invariants
1. **Workspace Layout (`WorkspacePaths::new(root)`)**:
   - Supports both the submodule layout (`<root>/modules/rustic-isles/native/...`, `<root>/modules/rustic-isles/gamedata/...`) and the flat layout (`<root>/paradox/...`, `<root>/backend/data/GameData`, `<root>/director/data/GameData`), picking `<root>/modules/rustic-isles` when that directory exists.
   - `find_host_binary(root)` checks `<root>/<HOST_BINARY>`, `<root>/bin/<HOST_BINARY>`, and `<root>/rust/target/release/<HOST_BINARY>`.
   - `is_workspace(root)` returns `true` iff `find_host_binary(root).is_some()` AND (`<root>/modules/rustic-isles` is a dir OR `<root>/paradox` is a dir).
   - `runtime_publish`: `<root>/.workspace/runtime` if `.workspace` is a dir or `content/data/runtime` is not a dir; otherwise `<root>/content/data/runtime`.
   - `server_config_toml`: `<root>/.workspace/config/host.toml`; `server_config`: `<root>/.workspace/config/host.json`.
   - `launcher_log(name)`: `<root>/.workspace/logs/launcher-<name>.log`.
2. **Game Data Detection (`detect_game_data(dir, required)`)**:
   - If any file in `required` is missing, not a regular file, or has `len() == 0`, returns `GameDataState::Missing`.
   - If any `<name>` has a sibling `<stem>.example.json` whose contents are byte-for-byte identical (`len()` equal and bytes equal), returns `GameDataState::Synthetic`.
   - Otherwise returns `GameDataState::Real`.
3. **Dumper-7 Discovery (`find_dumps(root)`)**:
   - Scans up to 200 immediate subdirectories of `root` (`C:\Dumper-7`).
   - Keeps subdirectories containing `CppSDK/SDK.hpp` as a regular file, recording `dir` (`<root>/<name>/CppSDK`), `name`, and `modified` (mtime of `SDK.hpp` formatted as RFC 3339 milliseconds with `Z`), sorted newest mtime first.
4. **Toolchain Detection (`find_toolchain`, `platform_toolset`, `msbuild_args`)**:
   - Runs `vswhere.exe -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -format json` and `vswhere.exe -latest -products * -requires Microsoft.Component.MSBuild -find MSBuild\**\Bin\MSBuild.exe`.
   - `platform_toolset(vs_major)`: returns `"v145"` when `vs_major >= Some(18)` (Visual Studio 2026+), else `"v143"`.
   - `msbuild_args(vcxproj, toolset)`: returns `[vcxproj, "/p:Configuration=Release", "/p:Platform=x64", "/p:PlatformToolset=<toolset>", "/m", "/nologo", "/verbosity:minimal"]`.
5. **SDK Import (`import_sdk(dump_dir, runtime_source)`)**:
   - Accepts either `<dump_dir>` or `<dump_dir>/CppSDK` as the source folder containing `SDK.hpp` and `SDK/`.
   - Validates `SDK.hpp` exists (`SdkError::NoHeader`), `runtime_source` is a dir (`SdkError::NoRuntimeSource`), and `count_function_files(&source.join("SDK")) > 0` (`SdkError::NoFunctions`).
   - Copies `SDK.hpp`, `PropertyFixup.hpp`, `UnrealContainers.hpp`, `UtfN.hpp` (when present) into `runtime_source`, removes any existing `<runtime_source>/SDK`, and copies `<source>/SDK` into `<runtime_source>/SDK` via `copy_tree` (which skips symlinks). Returns the count of `*_functions.cpp` files.
6. **Pure Parser (`hostkit/parse.rs`)**:
   - `clean_line(raw)`: strips ANSI CSI sequences (`\x1b[` followed by `0x20..=0x3F` parameter bytes and `0x40..=0x7E` final byte), removes `\r`, `trim_end()`, truncates to 400 chars.
   - `failure_line(lines)`: searches in reverse for a line starting with `"Error:"` (returning its trimmed remainder up to 300 chars), falling back to the last non-empty trimmed line up to 300 chars.
   - `invite_line(lines)`: parses `last_json(lines)`; requires `invite` string starting with one of `INVITE_LINK_PREFIXES`, `<= 4096` bytes, no control chars; optional `for` string trimmed, `1..=40` chars, no control chars.
   - `server_config_toml(raw)` & `server_config(raw)`: parses `host.toml` (via `toml_edit::DocumentMut`) or `host.json` (via `serde_json`), reading `server.name` (trimmed, non-empty, `<= 80` chars, no control chars), `server.mode` (`"private"` or `"public"`), `server.public_host` (when public, trimmed, `<= 255` chars, no control chars or whitespace), and private-mode `test_multipliers` / `gameplay` (`xp` and `rewards` finite in `1.0..=100.0`, returning `Some((xp, rewards))` only when `xp > 1.0 || rewards > 1.0`).

#### Test Expectations
- Fuzz target `fuzz/fuzz_targets/launcher_host.rs` compiles `hostkit/parse.rs` directly and verifies all output bounds on arbitrary byte inputs.
- Unit tests verify `WorkspacePaths` detection on both submodule and flat directory trees, `detect_game_data` (`Missing` / `Synthetic` / `Real`), `import_sdk` file copying and error codes, `server_config_toml` and `server_config` parsing, and `vswhere` output parsing.

---

### 2.4 Module 4: `platform::hostserver` (`hostserver.rs`, `hostserver/tests.rs`)

Manages the host server lifecycle from the Host page—either as an in-process `EmbeddedServer` (`server-gui` feature, RD26) or as a child `reforged-host` process—and runs one-shot `reforged-host` CLI subcommands (`setup`, `invite`, `update`, `multipliers`, `generate-gamedata`, `export-gamedata`, `import-gamedata`, `publish-runtime`).

#### Constants
- `pub const CONTROL_ENV: &str = "REFORGED_CONTROL"` (`"stdin"` when spawning `reforged-host start`)
- `pub const SERVER_READY_LINE: &str = "REFORGED_SERVER_READY"`
- `pub const BACKEND_PORT: u16 = 61000`
- `const STOP_TIMEOUT: Duration = Duration::from_secs(45)`
- `const KILL_GRACE: Duration = Duration::from_secs(5)`
- `const PROBE_TIMEOUT: Duration = Duration::from_millis(700)`
- `const CHANGE_EVERY: Duration = Duration::from_millis(250)`
- `pub const MAX_LINES: usize = 300`

#### Public Types & Signatures

```rust
pub type Probe = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;
pub type Listener = Arc<dyn Fn(HostServerView) + Send + Sync>;
pub type LineSink = Arc<dyn Fn(String) + Send + Sync>;

pub trait EmbeddedServer: Send + Sync {
    fn start(&self, workspace: PathBuf, lines: LineSink) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
    fn stop(&self) -> Pin<Box<dyn Future<Output = ()> + Send>>;
    fn restart(&self, name: String) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
    fn components(&self) -> Vec<HostServerComponent>;
    fn modules(&self) -> Option<Arc<dyn crate::bridge::modules::ModulePanel>> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostTool {
    pub program: PathBuf,
    pub prefix: Vec<OsString>,
}

impl HostTool {
    pub fn find(workspace: &Path) -> Option<Self>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKitError {
    Missing,
    Spawn(String),
}

impl HostKitError {
    pub fn detail(&self) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub code: i32,
    pub lines: Vec<String>,
    pub failure: Option<String>,
}

#[derive(Clone)]
pub struct HostServer;

impl Default for HostServer { fn default() -> Self; }

impl HostServer {
    pub fn new(probe: Probe, stop_timeout: Duration) -> Self;
    pub fn set_listener(&self, listener: Listener);
    pub fn set_embedded(&self, server: Arc<dyn EmbeddedServer>);
    pub fn has_embedded(&self) -> bool;
    pub fn modules(&self) -> Option<Arc<dyn crate::bridge::modules::ModulePanel>>;
    pub fn view(&self) -> HostServerView;
    pub fn state(&self) -> HostServerState;
    pub fn running(&self) -> bool;
    pub fn set_invite(&self, invite: Option<HostInvite>);
    pub fn note(&self, line: &str);
    pub async fn command(
        &self,
        tool: &HostTool,
        workspace: &Path,
        args: &[String],
        log_name: &str,
    ) -> Result<CommandOutput, HostKitError>;
    pub async fn refresh(&self);
    pub async fn start(&self, tool: &HostTool, workspace: &Path) -> Result<(), HostKitError>;
    pub async fn restart(&self, name: &str) -> Result<(), String>;
    pub async fn stop(&self);
}

pub async fn port_answers(port: u16) -> bool;
pub async fn kill_tree(pid: u32);
```

#### Behavioural Invariants
1. **Secret Redaction on `invite` Commands**:
   - When `subcommand == "invite"`, any output line whose trimmed text starts with `'{'` (the JSON line carrying the raw invite link) MUST NOT be appended to `HostServerView::lines` and MUST NOT be written to `.workspace/logs/launcher-invite.log`. It is kept only in `CommandOutput::lines` in memory so `controller::host` can parse `invite_line` and store it in `HostServerView::invite`.
2. **Readiness & State Transitions**:
   - Child mode (`embedded` is `None`):
     - Before spawning `reforged-host start`, probes `BACKEND_PORT` (61000). If it already answers, sets state to `HostServerState::External` and returns `Err(ErrorCode::ServerBusy)`.
     - Spawns `reforged-host start` with `REFORGED_CONTROL=stdin`, `CREATE_NO_WINDOW` on Windows, piped stdin/stdout/stderr, logging to `.workspace/logs/launcher-server.log`.
     - State starts at `HostServerState::Starting`. When a stdout/stderr line equals `"REFORGED_SERVER_READY"`, that line is suppressed from `lines`/log and state transitions from `Starting` to `Running`, immediately emitting `HostServerView`.
     - `stop()`: writes `"stop\n"` to child stdin and waits up to `STOP_TIMEOUT` (45 s). If the child does not exit in time, sends kill signal (`kill_tree`) and waits up to `KILL_GRACE` (5 s).
   - Embedded mode (`embedded` is `Some`):
     - `start()` sets `Starting`, calls `embedded.start(workspace_root.to_path_buf(), sink)`, and transitions to `Running` on `Ok(())` or `Failed` (with `message: Some(why)`) on `Err(why)`.
     - `restart(name)` requires `state == HostServerState::Running` (else `ErrorCode::ServerStopped`) and calls `embedded.restart(name)`, mapping errors to `ErrorCode::HostFailed`.
     - `stop()` transitions to `Stopping`, calls `embedded.stop().await`, and transitions to `Stopped`.
3. **Line Buffer Throttling**:
   - `lines` is a ring buffer capped at `MAX_LINES` (300) lines, each cleaned via `hostkit::parse::clean_line`.
   - Listener notifications for new log lines are throttled to at most once per `CHANGE_EVERY` (250 ms), while state/invite/message changes emit immediately.

#### Test Expectations
- State machine test with fake host script (`hostserver/tests.rs`): verifies `Stopped -> Starting -> Running` on `REFORGED_SERVER_READY`, `stop\n` graceful shutdown to `Stopped`, `External` detection when probe returns `true`, and `Failed` transition with `failure_line` when child exits non-zero.
- Secret redaction test: verifies that running `command(..., "invite", ...)` returns the JSON line in `CommandOutput::lines` while neither `view().lines` nor `launcher-invite.log` contains the JSON line or invite link.
- Embedded server test: verifies `start`, `restart`, `stop`, and `components` delegation through `EmbeddedServer`.

---

### 2.5 Module 5: `platform::requirements` (`requirements.rs`)

Checks the host PC for the 7 prerequisites shown on the Host page's Requirements tab.

#### Public Types & Signatures

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Database {
    MongoDb,
    Postgres,
}

pub const DATABASE: Database = Database::Postgres;
pub const POSTGRES_MINIMUM: u32 = 17;

pub struct Probes<'a> {
    pub env: &'a (dyn Fn(&str) -> Option<OsString> + Sync),
    pub exists: &'a (dyn Fn(&Path) -> bool + Sync),
    pub list_dir: &'a (dyn Fn(&Path) -> Vec<String> + Sync),
}

impl Probes<'static> {
    pub fn system() -> Self;
}

pub struct Inputs<'a> {
    pub game: (bool, Option<String>),
    pub toolchain: &'a Toolchain,
    pub dumper_dll: Option<&'a Path>,
    pub tailscale: bool,
    pub missing_vc_runtime: usize,
}

pub fn list_dir(dir: &Path) -> Vec<String>;
pub fn find_mongod(probes: &Probes<'_>) -> Option<PathBuf>;
pub fn find_postgres(probes: &Probes<'_>) -> Option<(PathBuf, Option<u32>)>;
pub async fn check_requirements(
    inputs: &Inputs<'_>,
    probes: &Probes<'_>,
    service_running: impl AsyncFnOnce() -> bool,
    any_visual_studio: impl AsyncFnOnce() -> Option<String>,
) -> Vec<Requirement>;
pub async fn check_requirements_here(inputs: &Inputs<'_>) -> Vec<Requirement>;
```

#### Behavioural Invariants
`check_requirements_here` returns a `Vec<Requirement>` of length 7 in the exact order:
1. **`RequirementId::Dauntless`**:
   - `Ok` with `detail: game.root.clone()` when `game.root.is_some()` and `game.verified && game.problem.is_none()`.
   - `Problem` with `detail: game.root.clone()` and `problem: Some("wrong_version" | "executable_missing" | "not_verified")` when `game.root.is_some()`.
   - `Missing` when `game.root.is_none()`.
2. **`RequirementId::Mongodb`**:
   - When `DATABASE == Database::MongoDb`: looks for `mongod.exe` via `find_mongod` (`<ProgramW6432|ProgramFiles>\MongoDB\Server\<ver>\bin\mongod.exe`, sorted descending by version, then `PATH`). `Ok` with `detail: Some("MongoDB <ver>" | path)` when found, `Missing` otherwise. (Note: a running Windows MongoDB service is NOT a problem; `problem` is `None`.)
   - When `DATABASE == Database::Postgres`: uses `find_postgres` (`PostgreSQL/<ver>/bin/pg_ctl.exe` + `initdb.exe`); if highest major version `< POSTGRES_MINIMUM` (17), returns `Problem` with `problem: Some("too_old")`.
3. **`RequirementId::Tailscale`**:
   - `Ok` if `inputs.tailscale` is `true`, else `Missing`.
4. **`RequirementId::VisualStudio`**:
   - `Ok` with `detail: toolchain.visual_studio.clone()` when `toolchain.msbuild.is_some()`.
   - `Problem` with `detail: any_vs`, `problem: Some("no_cpp")` when `msbuild` is `None` but `any_vs` is `Some(_)`.
   - `Missing` otherwise.
5. **`RequirementId::Rust`**:
   - `Ok` with `detail: cargo_version` (trimmed first line of `cargo --version`, stripping `"cargo "` prefix, e.g. `"cargo 1.85.0 ..." -> "Rust 1.85.0 ..."`) when `toolchain.cargo.is_some()`, else `Missing`.
6. **`RequirementId::Dumper7`**:
   - `Ok` with `detail: Some(path)` when `inputs.dumper_dll` is `Some(p)` and `(probes.exists)(p)` is `true`, else `Missing`.
7. **`RequirementId::VcRedist`**:
   - `Ok` when `inputs.missing_vc_runtime.is_empty()`, else `Missing` with `detail: Some(inputs.missing_vc_runtime.join(", "))`.

---

### 2.6 Module 6: `platform::dialogs` (`dialogs.rs`)

Wraps native OS folder/file dialogs (`rfd` on desktop) and the custom two-button close-confirmation dialog (`tauri_plugin_dialog`) localized through `core::i18n`.

#### Constants & Signatures

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderPurpose {
    Game,
    Runtime,
    Sdk,
    Workspace,
    #[cfg(feature = "legacy")]
    Revived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepOpen {
    Relay,
}

pub const DIALOG_GAME_FOLDER: &str = "dialog.gameFolder";
pub const DIALOG_REVIVED_FOLDER: &str = "dialog.revivedFolder";
pub const DIALOG_RUNTIME_FOLDER: &str = "dialog.runtimeFolder";
pub const DIALOG_SDK_FOLDER: &str = "dialog.sdkFolder";
pub const DIALOG_WORKSPACE_FOLDER: &str = "dialog.workspaceFolder";
pub const DIALOG_DUMPER_DLL: &str = "dialog.dumperDll";
pub const DIALOG_GAMEDATA_PACK: &str = "dialog.gamedataPack";
pub const DIALOG_GAME_RUNNING_TITLE: &str = "dialog.gameRunning.title";
pub const DIALOG_GAME_RUNNING_MESSAGE: &str = "dialog.gameRunning.message";
pub const DIALOG_GAME_RUNNING_DETAIL: &str = "dialog.gameRunning.detail";
pub const DIALOG_KEEP_OPEN: &str = "dialog.gameRunning.keepOpen";
pub const DIALOG_CLOSE_ANYWAY: &str = "dialog.gameRunning.closeAnyway";

pub const TEXT_FILES: (&str, &str) = ("Text", "txt");
pub const DLL_FILES: (&str, &str) = ("DLL", "dll");
pub const GAMEDATA_PACKS: (&str, &str) = ("Game data pack", "gz");

pub fn folder_title(purpose: FolderPurpose) -> &'static str;
pub async fn choose_folder(app: &AppHandle, language: Language, purpose: FolderPurpose, start: &Path) -> Option<PathBuf>;
pub async fn choose_save_file(app: &AppHandle, default_dir: &Path, default_name: &str, filter: (&str, &str)) -> Option<PathBuf>;
pub async fn choose_open_file(app: &AppHandle, default_dir: Option<&Path>, filter: (&str, &str)) -> Option<PathBuf>;
pub async fn choose_dll(app: &AppHandle, language: Language, start: Option<&Path>) -> Option<PathBuf>;
pub async fn choose_pack(app: &AppHandle, language: Language) -> Option<PathBuf>;
pub async fn confirm_keep_open(app: &AppHandle, language: Language, reason: KeepOpen) -> bool;
```

#### Behavioural Invariants
1. **Parent Window Attachment**: On Windows (`#[cfg(windows)]`), file dialogs attach to `app.get_webview_window(window::MAIN)` via `rfd::AsyncFileDialog::new().set_parent(&main)`.
2. **Start Directory Fallback**: Only calls `.set_directory(dir)` when `dir.exists()` is `true`.
3. **Close Confirmation (`confirm_keep_open`)**:
   - Displays a Warning message dialog with custom buttons `OkCustom(translate(language, DIALOG_KEEP_OPEN))` and `CancelCustom(translate(language, DIALOG_CLOSE_ANYWAY))`.
   - Returns `true` (keep the launcher open) for every outcome **except** when the user explicitly clicks the `close_anyway` label (`MessageDialogResult::Custom(chosen) if chosen == close_anyway`). Dismissing the dialog or pressing Escape keeps the launcher open (`true`).

---

### 2.7 Module 7: `controller::host` and `controller::host_off` (`host.rs`, `host_off.rs`)

Orchestrates the Host page state (`HostKit`, `HostServerView`), background `HostJob` execution, RD9 loader-session integration for `InjectDumper` and `InjectExporter`, and host workspace commands.

#### Public Types & Controller Methods

```rust
pub use crate::platform::hostserver::Probe;

pub struct HostState;

impl Default for HostState { fn default() -> Self; }
impl HostState {
    pub fn with(probe: Probe, dump_root: PathBuf) -> Self;
}

impl Controller {
    pub fn host_attach(&self);
    pub async fn host_shutdown(&self);
    pub fn ensure_host_workspace(&self, auto: bool) -> Option<PathBuf>;
    pub fn host_workspace(&self) -> Option<PathBuf>;
    pub fn workspace(&self) -> Option<PathBuf>;
    pub async fn host_kit(&self) -> HostKit;
    pub async fn host_choose_workspace(&self) -> ActionResult;
    pub async fn host_choose_dumper(&self) -> ActionResult;
    pub async fn host_import_gamedata(&self) -> ActionResult;
    pub async fn host_import_sdk_folder(&self) -> ActionResult;
    pub async fn host_download_project(&self) -> ActionResult;
    pub async fn host_action(&self, action: HostAction) -> ActionResult;
    pub fn host_open(&self, folder: HostFolder) -> ActionResult;
    pub async fn host_server_view(&self) -> HostServerView;
    pub async fn host_server_start(&self) -> ActionResult;
    pub async fn host_server_restart(&self, service: &str) -> ActionResult;
    pub fn set_embedded_server(&self, server: Arc<dyn crate::platform::hostserver::EmbeddedServer>);
    pub fn module_panel(&self) -> Option<Arc<dyn crate::bridge::modules::ModulePanel>>;
    pub async fn host_server_stop(&self) -> ActionResult;
    pub async fn host_setup(&self, request: HostSetupRequest) -> ActionResult;
    pub async fn host_create_invite(&self, for_name: Option<String>) -> ActionResult;
    pub async fn host_update(&self) -> ActionResult;
    pub async fn host_set_multipliers(&self, multipliers: Option<HostMultipliers>) -> ActionResult;
}
```

#### Behavioural Invariants
1. **Feature Gate (`host` vs `host_off.rs`)**:
   - When `#[cfg(not(feature = "host"))]`, `host_off.rs` provides the identical public method signatures on `Controller`:
     - `host_attach`, `host_shutdown`: no-ops.
     - `workspace`: returns `None`.
     - `host_kit`: returns an empty `HostKit` (`workspace: None`, `workspace_ok: false`, all booleans `false`, empty `dumps` and `requirements`).
     - `host_server_view`: returns `HostServerView` in `HostServerState::Stopped`.
     - Every action method (`host_choose_workspace`, `host_choose_dumper`, `host_import_gamedata`, `host_import_sdk_folder`, `host_download_project`, `host_action`, `host_open`, `host_server_start`, `host_server_restart`, `host_server_stop`, `host_setup`, `host_create_invite`, `host_update`, `host_set_multipliers`) returns `ActionResult::Err(LauncherError::with_detail(ErrorCode::HostFailed, "host_tools_unavailable"))`.
2. **Host Tools Gate**:
   - Every mutating Host page command requires `host_tools` to be enabled in settings (in builds without `cfg(feature = "server-gui")`, where `host_tools` is always enabled). If `host_tools` is `false`, returns `ErrorCode::HostFailed` with detail `"host_tools_disabled"`.
3. **Single Job Concurrency**:
   - Only one `HostJob` may be in `HostJobState::Running` at a time. Starting another job while one is running returns `ActionResult::Err(LauncherError::new(ErrorCode::Busy))`.
   - When a job starts, transitions to `Running`, and when it finishes (`Done` or `Failed`), an updated `HostKit` event (`"launcher:host-kit"`) is published immediately.
4. **RD9 No-Injection Invariant (`InjectDumper` and `InjectExporter`)**:
   - **NEVER** opens another process handle (`OpenProcess`, `VirtualAllocEx`, `WriteProcessMemory`, `CreateRemoteThread` are strictly forbidden).
   - **`HostAction::InjectDumper`**:
     - Refuses with `ErrorCode::GameRunning` if the game is already running or dumping, or if any running game client process is detected on the system.
     - Requires a verified game folder (`GameFolderNotFound` / `GameNotVerified` / `GameWrongVersion`), a selected Dumper-7 DLL file (`HostFailed("dumper_dll")`), and the built runtime loader `winmm.dll` (`HostFailed("loader_not_built")`).
     - Calls `self.begin_loader_session(SessionKind::Dump, &win64, &paths.loader_dll, &dumper)` which copies `winmm.dll` and the Dumper-7 DLL into `Win64`, moves `Rustic-Isles.dll` aside to `Rustic-Isles.dll.rr-dump-aside` so Dumper-7 sees a clean game process, and writes `mystic_loader.ini` listing the Dumper-7 DLL filename.
     - Spawns `Dauntless-Win64-Shipping.exe` with `["-DisableEOS", "-EpicPortal", "-NoEAC", "-windowed", "-ResX=1280", "-ResY=720"]`.
     - Polls `loader_session::dump_ready(&dump_root, started_at, &baseline)` every 2 seconds up to 180 seconds; once a new `SDK.hpp` is complete (and settled for 3 seconds), stops the game process, calls `end_loader_session().await` (restoring `Rustic-Isles.dll` and removing `mystic_loader.ini` and the copied Dumper-7 DLL), and marks the job `Done`.
   - **`HostAction::InjectExporter`**:
     - Refuses with `ErrorCode::GameRunning` if the game is already running or dumping.
     - Requires `CatalogExporter.dll` (`HostFailed("exporter_not_built")`), writes `export_flags.txt` (`EXPORT_FLAGS`) to `Win64`, and calls `self.begin_loader_session(SessionKind::Export, &win64, &paths.loader_dll, &paths.exporter_dll)` (which writes `mystic_loader.ini` listing both `Rustic-Isles.dll` and `CatalogExporter.dll`).
     - Starts the game through the normal `self.play().await` flow; when `play()` finishes or fails, `end_loader_session()` cleans up `CatalogExporter.dll`, `export_flags.txt`, and `mystic_loader.ini`.
5. **Auto-Applying Local Runtime & Server Invite**:
   - After `PublishRuntime` succeeds, if the host server is running and the joined server is on this PC (`is_loopback_host(&sv.host)`), automatically installs the published runtime into the local game folder (`install_local_runtime`) and refreshes the runtime offer.
   - After `host_setup` or `host_create_invite` produces an invite link (`invite_line(&out.lines)`), stores `HostInvite` in `HostServer` and, if the launcher has no joined server or is already joined to a loopback server, automatically joins the new invite (`self.join_invite(&found.link, false).await`).

---

### 2.8 Module 8: `controller::runtime` (`runtime.rs`)

Connects the controller state machine to `net::runtime_files` for inspecting, trusting, downloading, and selecting local overrides for the game runtime files.

#### Controller Methods & Invariants

```rust
impl Controller {
    pub(super) async fn refresh_runtime_offer(&self, endpoint: &Endpoint, gen: u64);
    pub(super) async fn inspect_runtime(&self);
    pub async fn trust_runtime(&self) -> ActionResult;
    pub async fn install_runtime(&self) -> ActionResult;
    pub async fn choose_local_runtime(&self) -> ActionResult;
    pub async fn clear_local_runtime(&self) -> ActionResult;
}
```

1. **`refresh_runtime_offer(endpoint, gen)`**:
   - Calls `fetch_runtime_manifest(endpoint).await`.
   - Checks connection generation `self.current_gen(gen)` before mutating state.
   - On `Ok(manifest)`: records the offered `RuntimeManifest` and calls `self.inspect_runtime().await`.
   - On `None`: clears the offered `RuntimeManifest` and `RuntimeView` version/files, and calls `self.inspect_runtime().await`.
   - On `CertMismatch`: marks connection `Disconnected` with `ConnectProblem::CertMismatch`, `Phase::Blocked`, and `ErrorCode::CertMismatch`.
   - On `BadAnswer | Unreachable`: clears the offered `RuntimeManifest`, sets `RuntimeView.state = RuntimeState::Failed`, and recomputes the launcher phase.
2. **`inspect_runtime()`**:
   - Calls `check_runtime(&win64, offered.as_ref(), &trusted, local.as_deref()).await` (or `(RuntimeState::Missing, false)` when no game root is set) and updates `RuntimeView` (`state`, `version`, `accepted`, `local_dir`, `files`), then recomputes the launcher phase and publishes a snapshot.
3. **`trust_runtime()`**:
   - Requires a joined server and an offered `RuntimeManifest` (`ErrorCode::RuntimeNotOffered` if absent).
   - Calls `runtime_files::accept(&mut list, &manifest.files)` for `sv.id()`, persists `trustedRuntimes` in `settings.json`, re-runs `inspect_runtime().await`, and if `RuntimeView.state != RuntimeState::UpToDate`, immediately chains into `self.install_runtime().await`.
4. **`install_runtime()`**:
   - Refuses with `ErrorCode::Busy` if a task is already running.
   - Requires a joined server (`ServerUnreachable`), an account key in `KeyStore` (`AccountRequired`), a verified game folder (`GameFolderNotFound` / `GameNotVerified` / `GameWrongVersion`), and an offered `RuntimeManifest` (`RuntimeNotOffered`).
   - If the offered runtime set is not yet trusted for `sv.id()`, returns `ErrorCode::RuntimeUntrusted`.
   - Runs `runtime_files::install_runtime` with a cancellation token and progress emitted as `TaskKind::Runtime`.
   - If `result.replaced_in_use` is non-empty or the game is currently running, sets notice `NoticeCode::RuntimeInstalledRestart`; otherwise `NoticeCode::RuntimeInstalled`.
5. **`choose_local_runtime()` & `clear_local_runtime()`**:
   - `choose_local_runtime()` opens `choose_folder(language, FolderPurpose::Runtime, start)`. If cancelled, returns `ErrorCode::Cancelled`. Validates `is_complete_runtime_dir(&picked)` (returning `ErrorCode::RuntimeLocalIncomplete` with detail `RUNTIME_FILE_NAMES.join(", ")` if incomplete), saves `localRuntimeDir` in `settings.json`, and calls `inspect_runtime().await`.
   - `clear_local_runtime()` sets `localRuntimeDir` to `Null` in `settings.json` and calls `inspect_runtime().await`.

---

## 3. Specification of Born-MIT & DL4 Launcher Modules

### 3.1 Bridge Specification, Modules & Commands (`bridge/spec.rs`, `bridge/modules.rs`, `bridge/commands.rs`, `bridge/generate.rs`)

1. **`bridge/spec.rs`**:
   - Defines `EVENTS: [&str; 6]` (`"launcher:snapshot"`, `"launcher:progress"`, `"launcher:host-kit"`, `"launcher:host-server"`, `"launcher:ServerStateChanged"`, `"launcher:ModuleStateChanged"`).
   - Defines `COMMANDS: [CommandSpec; 62]` with exact `Arg` shapes (`Text`, `OptionalText`, `Flag`, `Index`, `Name`, `Number`, `Object`, `SettingsPatch`, `Channel`).
   - Defines `PLANNED`: `["realm_host", "realm_join", "realm_stop", "realm_get"]` when `not(feature = "realms")`, or `[]` when `feature = "realms"`.
   - Defines `SERVER_GUI_ONLY`: `["host_server_restart", "module_list", "module_get", "module_run_command", "module_set_settings", "module_load", "module_unload", "history_query", "history_revert_dry_run", "history_revert_apply", "guarded_list", "guarded_approve", "guarded_cancel"]`.
   - `serves(name)`: returns `false` for `SERVER_GUI_ONLY` commands when `not(feature = "server-gui")`, and `false` for `REALMS_ONLY` commands when `not(feature = "realms")`; `true` otherwise.
   - `arg_accepts(arg, value)`: validates JSON values against `Arg` shapes using UTF-16 code-unit length (`js_length`) and control-character rejection (`!c.is_control()`).
2. **`bridge/modules.rs`**:
   - Pure validation and view transformation for `ModulePanel` (DU3):
     - Validates `module_id` via `ModuleId::new`, `command_args` against `CommandSpec`, `merge_settings` against `ConfigSchema`, `colour` against safe CSS colour syntax (`#hex`, `rgb(...)`, `hsl(...)`, `oklch(...)`, ASCII-lowercase keywords), and strips control characters via `plain()` and `output_text()`.
3. **`bridge/commands.rs`**:
   - Validates all untrusted IPC `serde_json::Value` arguments before invoking controller methods:
     - `host_setup_request`: `name` trimmed `1..=60` UTF-16 units without control chars; `mode` `"private" | "public"`; `publicHost` required in public mode and validated via `is_valid_host`.
     - `host_multipliers`: `null` or `{ xp: 1, rewards: 1 }` -> `Ok(None)`; `{ xp, rewards }` in `1.0..=100.0` rounded to 2 decimal places -> `Ok(Some(HostMultipliers))`.
     - `invite_for`: optional string trimmed `1..=40` UTF-16 units without control chars.
     - `realm_id`: trimmed `1..=REALM_ID_MAX` (64) UTF-16 units without control chars.
     - `guarded_id`: 16 lowercase ASCII hex chars (`0-9a-f`).
4. **`bridge/generate.rs`**:
   - Tests and generator (`RR_UPDATE_GENERATED=1`):
     - `every_command_is_served_and_nothing_else`: verifies `COMMANDS` matches `#[tauri::command]` functions in `commands.rs` and `PLANNED` 1-to-1.
     - `name_lists_are_the_enums`: verifies `HOST_ACTIONS`, `HOST_FOLDERS`, `EXTERNAL_TARGETS`, `REALM_KINDS`, `HOST_COMPONENT_PHASES` serialize identically to their Rust enums.
     - `generated_files_are_current`: verifies `gen/strings.json`, `permissions/commands.toml`, `capabilities/main.json`, and `dainty-lass-ui/src/shared/generated.ts` are up to date.

### 3.2 Runtime Loader Sessions (`platform/loader_session.rs`, `controller/loader.rs`)

Implements **RD9** (zero process injection):
- `LOADER_INI = "mystic_loader.ini"`, `DUMP_ASIDE = "Rustic-Isles.dll.rr-dump-aside"`, `HOST_SESSION_DLLS = ["CatalogExporter.dll", "Dumper-7.dll"]`, `EXPORT_FLAGS_FILE = "export_flags.txt"`, `LOADER_DLL = "winmm.dll"`, `RUNTIME_DLL = "Rustic-Isles.dll"`.
- `SessionKind { Dump, Export }`:
  - `LoaderSession::begin(kind, win64_dir, loader_dll, session_dll)`:
    - Copies `loader_dll` to `<win64_dir>/winmm.dll` (and unblocks it) if not already identical.
    - Copies `session_dll` into `<win64_dir>/<file_name>` (and unblocks it).
    - For `SessionKind::Dump`: renames `<win64_dir>/Rustic-Isles.dll` to `<win64_dir>/Rustic-Isles.dll.rr-dump-aside` so Dumper-7 inspects the unpatched game, and writes `mystic_loader.ini` containing `<session_dll_name>\r\n`.
    - For `SessionKind::Export`: requires `<win64_dir>/Rustic-Isles.dll` to exist, and writes `mystic_loader.ini` containing `Rustic-Isles.dll\r\n<session_dll_name>\r\n`.
  - `LoaderSession::cleanup(&self)` / `clean_leftovers(win64_dir)`:
    - Removes `mystic_loader.ini`, `export_flags.txt`, and any `HOST_SESSION_DLLS` (plus the session's copied DLL) from `win64_dir`.
    - Restores `Rustic-Isles.dll.rr-dump-aside` back to `Rustic-Isles.dll` (or removes the aside file if `Rustic-Isles.dll` already exists).
  - `leftover_files(win64_dir)`: returns the list of any leftover session files (`mystic_loader.ini`, `Rustic-Isles.dll.rr-dump-aside`, `export_flags.txt`, `CatalogExporter.dll`, `Dumper-7.dll`). Both `Controller::play()` and `Controller::ensure_realms_ready()` refuse to start if leftovers remain after cleanup (`ErrorCode::LoaderLeftovers`).

### 3.3 Platform Support Files (`linux_runtime.rs`, `updater.rs`, `core/i18n.rs`, `window.rs`, `log.rs`, `system.rs`)

1. **`platform/linux_runtime.rs`**:
   - Discovers Linux compatibility runners (`RunnerKind { Proton, Wine }`, `PreparedLaunch { program, args_before_exe, env, kind }`) and native Linux game stores (`discover_linux_stores`: Heroic `installed.json` and Legendary `installed.json` across native `~/.config` and Flatpak `~/.var/app/...`).
   - Searches override env vars (`DAUNTLESS_PROTON`, `DAUNTLESS_WINE`, `PROTON_PATH`), Heroic tools (`wine`/`proton`), Steam `compatibilitytools.d` and `steamapps/common/Proton*`, ULWGL (`umu-run`), and `wine64`/`wine` on `PATH`.
   - Sets `WINEDLLOVERRIDES="winmm=n,b"` (preserving existing overrides with `;winmm=n,b`) so Wine/Proton loads the runtime loader `winmm.dll` beside the game executable.
2. **`platform/updater.rs`**:
   - Reads `option_env!("TAURI_UPDATER_ENDPOINTS")` and `option_env!("TAURI_UPDATER_PUBKEY")`. When both are non-empty, builds `tauri_plugin_updater` (`parse_endpoints` requires `https://` URLs with host and no credentials) and runs a background check 4 seconds after startup (`INITIAL_DELAY`).
3. **`core/i18n.rs`**:
   - Includes `$OUT_DIR/languages.rs` generated from `gen/strings.json` (`Language` enum with `Language::ALL`, `code()`, `from_code()`, `pick_system_language()`, and `translate(language, key, &[(&str, &str)])` falling back to `Language::En`).
4. **`window.rs`**:
   - Builds the main Tauri window (`LABEL = "main"`, initial `1120x720`, min `980x640`, `decorations(false)`, `background_color(#080b12)`).
   - Persists and clamps `WindowBounds { x, y, width, height, maximized }` in `userData/window-state.json` against available monitor work areas (`clamp_to_monitors` requires `>= 120` px visible horizontally and `>= 80` px vertically).
5. **`platform/log.rs`**:
   - Maintains an in-memory secret list (`add_secret`, `remove_secret`): registers any secret with `js_length >= 6`, plus the suffix after `RRK_` if `>= 16` chars.
   - `redact(text)` replaces all registered secrets with `"<hidden>"`, `RRK_[A-Za-z0-9_-]+` with `"RRK_<hidden>"`, `(?i)(-AUTH_PASSWORD=)[^\s"']*` with `"${1}<hidden>"`, and `(?i)(x-reforged-user-api-key["']?\s*[:=]\s*["']?)[^\s"',}]+` with `"${1}<hidden>"`.
   - Writes to `<userData>/logs/launcher.log`, rotating to `launcher.old.log` above `2 MiB`, and drops all tracing/log events whose target matches `epic::is_egs_api_target`.
6. **`platform/system.rs`**:
   - `find_tailscale()` / `tailscale_candidates()`: checks standard Windows (`ProgramW6432\Tailscale\tailscale.exe`, `LOCALAPPDATA\Tailscale\tailscale.exe`, `PATH`) and Linux (`/usr/bin/tailscale`, `/usr/local/bin/tailscale`, `/snap/bin/tailscale`, `PATH`) locations.
   - `free_bytes(dir)`: walks `dir` and its parent directories until `fs4::available_space` succeeds.
   - `normalize_device_signals(uuid, tpm)`: validates canonical 8-4-4-4-12 hex SMBIOS UUID (rejecting all-0, all-f, and `"03000200-0400-0500-0006-000700080009"`) and 64-char hex TPM EK SHA-256 (rejecting all-0 and all-f).

### 3.4 Server GUI Modules (`server_gui.rs`, `server_gui/modules.rs`, `server_history.rs`, `controller/guarded.rs`)

1. **`server_gui.rs`**:
   - `EmbeddedServerState` implements `EmbeddedServer` over `server_controller::ServerController` and `host::serve::prepare_embedded`.
   - `init_telemetry()` combines `platform::log::Layer` and `service_kit::telemetry::ChannelLogLayer` into the global tracing subscriber and forwards only service-span events (`entry.service == "host"`, `HOST_SERVER_COMPONENTS`, or loaded module IDs) to the Host page's log sink formatted as `"{:<14} | {}"`.
2. **`server_gui/modules.rs`**:
   - `Panel` implements `bridge::modules::ModulePanel` over the running `ServerController`'s `ModuleRegistry` and `.workspace/config/host.toml` (`COMMAND_TIMEOUT = 120s`, `Actor::Console`).
3. **`server_history.rs`**:
   - Implements `query`, `dry_run`, and `apply` over `admin::history::HistorySet` opened from `<workspace>/.workspace/config/admin.env`.
   - Caches dry-run plans in a process-wide `PlanCache` (`PLANS`) so `apply(workspace, plan_id)` executes only a plan previously shown to the operator and records the revert under actor `"host:server-gui"`.
4. **`controller/guarded.rs`**:
   - Implements `guarded_list()` and `guarded_decide(id, approve)` over `host::guarded::{list, approve, cancel}`, mapping store failures to `GuardedNotFound`, `GuardedNotWaiting`, `Unknown`, or `HostFailed`.

### 3.5 Local Realms (`controller/realms.rs`, `controller/realms_off.rs`)

- Links `rustic-rifts-host` and `rustic-rifts-seam` behind `#[cfg(feature = "realms")]`.
- Enforces RD9 before loading `ParadoxTransport.dll`: refuses with `ErrorCode::LoaderLeftovers` if `loader_leftovers(&win64)` is non-empty, requires `ParadoxTransport.dll` and `ADMISSION_PUBLIC_KEY_FILE` in `Win64`, and verifies `ParadoxTransport.dll`'s SHA-256 against the offered `RuntimeManifest` (or local runtime).
- Derives `AccountId` strictly from the public username (`[A-Za-z0-9._-]{1,64}`, defaulting to `"local-player"`), never reading `KeyStore`.

### 3.6 Epic Integration (`platform/epic_login.rs`, `platform/epic_library.rs`, `platform/epic_session.rs`, `platform/keystore/epic.rs`, `controller/install.rs`, `net/epic_install.rs`)

1. **`platform/keystore/epic.rs` (`EpicSessionStore`)**:
   - Persists the Epic `Session` encrypted via `Encryptor` at `<userData>/keys/epic.session` (`VERSION = 1`), writing atomically via `.tmp` rename and registering both access and refresh tokens with `log::add_secret`.
2. **`platform/epic_session.rs` (`Epic`)**:
   - Serializes sign-in, refresh (`REFRESH_MARGIN_SECONDS = 600`, `EXCHANGE_TIMEOUT = 30s`), and sign-out (`SIGN_OUT_TIMEOUT = 10s`) behind a `tokio::sync::Mutex<Option<Session>>`.
   - Exposes only `EpicView { signed_in, display_name, signing_in }` to the bridge.
3. **`platform/epic_login.rs`**:
   - Opens an incognito secondary `WebviewWindow` (`label = "epic-sign-in"`) on `epic::auth::LOGIN_URL` with popups denied (`NewWindowResponse::Deny`), non-HTTPS URLs denied, and a navigation state machine (`Watch`) that accepts `#<code>` handoffs to `https://epic-sign-in.invalid/#<code>` **only** when immediately preceded by `https://www.epicgames.com/id/api/redirect`.
4. **`platform/epic_library.rs`**:
   - Wraps `epic::library::Ownership<EgsLibrary>` to serve `epic_library_check` (`EpicLibraryCheck::Ok { owned, checked_at }`) and `require` (`ErrorCode::EpicNotOwned`, `EpicSignedOut`, `EpicLibraryFailed`).
5. **`controller/install.rs` & `net/epic_install.rs`**:
   - `EpicGate` refreshes the Epic session and checks Dauntless ownership before starting or resuming a download.
   - Fetches the pinned Epic manifest from joined servers first (`/content/v1/versions/<version>/epic/manifest` with that server's account key), falling back to `epic.manifest_sources`, verifying `Sha1Digest`.
   - `InstallJob` wraps `epic::download::install` with `pause`, `resume`, `cancel`, a 5-second rolling throughput calculation, and a 250 ms progress ticker emitting `TaskProgress` (`TaskKind::Install`).
   - Stores resume state in `<userData>/epic/<version>.resume` and completion record in `<userData>/epic/<version>.installed`.

### 3.7 Region Ping & Version Discovery (`net/regionping*`, `controller/region.rs`, `net/versions*`, `controller/versions.rs`)

1. **`net/regionping` & `controller/region.rs`**:
   - `parse::read_nodes(status, body)` (`MAX_NODES = 16`): HTTP 404 -> empty list; HTTP 200 -> keeps first valid `NodeEntry { region, host, port, fingerprint }` per `region` (`is_region_id`: `1..=32` chars `[a-z0-9-]`, no leading/trailing `-`). Compiled into fuzz target `launcher_region`.
   - `measure`: connects to up to `concurrency = 4` nodes at once over TLS pinned to each node's `fingerprint` (never sending the account key to non-home nodes), sends up to `samples = 5` `GET /network/v1/ping` requests on a single HTTP/1.1 connection per node (`connect_timeout = 2s`, `sample_timeout = 1500ms`, `node_deadline = 4s`), computes median RTT in tenths of a millisecond, and orders regions nearest-first with silent nodes last (`order`).
   - `report`: sends `POST /network/v1/regions` with `{"regions": [{"region", "rttMs"}, ...]}` and the player's account key to the home server only.
2. **`net/versions` & `controller/versions.rs`**:
   - Queries `/content/v1/versions` and `/content/v1/versions/<version>/client` on each joined server, validates each `ClientProfile` via `ModuleManifest::validate()`, and merges them with `builtin::profiles()` (compiled-in versions keep their compiled-in pin; a server offering a different pin for a compiled-in version is ignored; versions sort newest-first).
   - Resolves `download_bytes` and `install_bytes` from `Manifest::parse(bytes, &pin)` (cached in `ManifestCache` by SHA-1 digest for the session).

---

## 4. Post-cutover and Tauri App Updates (`Part B`, `T1`–`T5`, `DL4b`)

### 4.1 `Dainty Lass Manager` (`dainty-lass-manager`) & Default Workspace (`Part B`)

1. **Product & Binary Identity**:
   - Client launcher binary: `dainty-lass-client` (`Dainty Lass`).
   - Server GUI binary (`--features server-gui`): `dainty-lass-manager` (`Dainty Lass Manager`, identifier `com.silentspud.daintylass.manager`, separate `userData` folder).
2. **Default Server Workspace (`ensure_host_workspace`)**:
   - In `server-gui` mode (or when `auto` is true), if no workspace is configured in `settings.json`, defaults to `<userData>/Server`, creates `.workspace/config`, `.workspace/data`, and `.workspace/logs`, and writes a starter `.workspace/config/host.toml` if absent so `HostKit::workspace_ok` is `true` on a fresh Manager install without requiring a git checkout.
3. **`host_download_project` Retired**:
   - `Controller::host_download_project` is retired and immediately returns `ActionResult::Err(LauncherError::new(ErrorCode::NotInThisBuild))`.

### 4.2 Host Requirements Without MongoDB or Git (`T4`)

- `RequirementId` and `check_requirements_here` check **6** prerequisites in order (MongoDB and Git are no longer required for hosting):
  1. `RequirementId::Dauntless`
  2. `RequirementId::Tailscale`
  3. `RequirementId::VisualStudio`
  4. `RequirementId::Rust`
  5. `RequirementId::Dumper7`
  6. `RequirementId::VcRedist`
- `find_postgres` remains available for optional PostgreSQL checks.

### 4.3 Feature-Independent Bridge Code Generation (`T1`)

- `bridge::generate` emits identical output regardless of which Cargo features (`server-gui`, `realms`, `legacy`, `host`) are enabled at build time:
  - `ui/packages/bridge/src/generated/types.ts`
  - `launcher/src-tauri/isolation/commands.js`
  - `launcher/src-tauri/permissions/app-commands.toml`
  - `launcher/src-tauri/gen/strings.json`
- Every command in `spec::COMMANDS` and every wire type is always included in the generated files; commands disabled by the current binary's features return `ErrorCode::NotInThisBuild` at runtime (`*_off.rs` stubs).

### 4.4 Server GUI Bridge Commands (`admin_*` and `server_*`, `T2`–`T3`)

In addition to the module, history, and guarded-action commands, `spec::SERVER_GUI_ONLY` includes 11 server-management and admin commands:
1. `admin_lookup(query: Text)` -> looks up player accounts by username, email, or account ID via `admin` / `store::Database`.
2. `admin_action(request: Object)` -> executes `ban`, `unban`, `kick`, `grant_currency`, `grant_item`, `reset_character`, or `set_role` on a player account, recording the change-log entry under actor `"host:server-gui"`.
3. `server_doctor()` -> runs `hostkit::doctor::run_doctor` on the active workspace and returns structured `ServerDoctorReport { checks, fails, warnings }`.
4. `server_backup()` -> creates a hot database backup in `<workspace>/.workspace/backups/` and returns `ServerBackupResult { path, bytes }`.
5. `server_config_get()` -> reads `<workspace>/.workspace/config/host.toml` and returns `ServerConfigView`.
6. `server_config_save(patch: Object)` -> validates and writes updated settings (`name`, `mode`, `public_host`, `public_port`, `registration`, `max_hunts`, `xp_multiplier`, `reward_multiplier`, `memory_guard`, `database`) to `host.toml` and regenerates service `.env` files.
7. `server_mongo_status()` -> inspects `<workspace>` for unmigrated legacy MongoDB data (`.mongo-migrated`, `.workspace/data/mongo`, reachable MongoDB collections) and returns `ServerMongoStatus`.
8. `server_mongo_migrate(source: OptionalText)` -> runs the MongoDB-to-SQLite migration and verification, writing `.mongo-migrated` on success.
9. `server_mongo_fresh_start()` -> sets `database.skip_mongo_migration = true` in `host.toml` and writes `.mongo-migrated` so a fresh server starts cleanly without migrating legacy MongoDB data.
10. `server_module_logs(module_id: OptionalText)` -> returns recent structured log entries (`Vec<ServerLogLine>`) from the in-memory ring buffer filtered by optional service/module name.
11. `server_invite_list()` -> lists active invite codes from the backend database (`Vec<ServerInviteEntry>`).

### 4.5 Account Ban Handling (`T5`)

- When `/undaunted/api/verify` (or `/undaunted/api/register`) returns HTTP 403 with `"error": "account_banned"`:
  - `VerifyOutcome::Banned { reason }` marks `ServerView::key_rejected = true`, sets the snapshot's `last_error` to `LauncherError::new(ErrorCode::AccountBanned)` (with optional ban reason detail), and transitions the launcher phase to `Phase::Register` so the player sees the ban error banner and can enter a different key.

### 4.6 Target Crate Split (`crates/dainty-launcher-kit` vs `launcher/src-tauri`, `DL4b`)

1. **`crates/dainty-launcher-kit` (`license = "MIT"`, `Copyright (c) 2026 Silent Spud`)**:
   - Contains the pure, clean-room and born-here launcher modules that do not depend on `tauri::AppHandle` or DR's `controller`/`keystore`/`proxy` core:
     - `bridge/spec.rs` (command/event/arg specifications and `arg_accepts` validator)
     - `bridge/modules.rs` (module panel validation and sanitisation)
     - `hostkit/mod.rs` and `hostkit/parse.rs` (workspace layout, Dumper-7 discovery, SDK copy, output sanitisation)
     - `net/runtime_files.rs` and `net/runtime_files/parse.rs` (runtime manifest verification, trust store, atomic download & `.rr-old-*` replacement)
     - `net/regionping.rs` and `net/regionping/parse.rs` (multi-region ping measurement and node list parser)
     - `net/versions.rs` and `net/versions/parse.rs` (server version & client profile discovery)
     - `platform/loader_session.rs` (RD9 `mystic_loader.ini` dump/export session management)
     - `platform/linux_runtime.rs` (Linux Wine/Proton/Heroic/Legendary discovery)
     - `platform/requirements.rs` (host prerequisite checks)
   - Has **zero** normal dependencies on any AGPL crate (`launcher`, `host`, `rustic-rifts-host`, `rustic-rifts-seam`, `gateway`, `content`, `chat`, `isles-*`, `revived-1-4-4`).
2. **`launcher/src-tauri` (`license = "AGPL-3.0-only"`)**:
   - Thin Tauri shell and DR state machine (`controller`, `keystore`, `localproxy`, `singleinstance`, `game`, Tauri window/dialog/command glue) that depends on `launcher-kit` and re-exports its modules at their existing paths so all unit, integration, and fuzz targets compile and pass unchanged.

---

## 5. Verification & Acceptance Checklist for DL4b

1. **Zero Prohibited Provenance in `crates/dainty-launcher-kit`**:
   - `crates/dainty-launcher-kit/Cargo.toml` sets `license = "MIT"`.
   - Grepping `crates/dainty-launcher-kit` for `ReforgedLauncher`, `Dauntless-Reforged-Revival`, `dauntless-revived`, `mixutin`, `Freeza`, `e1c5fec`, `24b7624`, or `.ts` source citations returns **0 matches**.
   - `cargo tree -e normal -p dainty-launcher-kit` has **zero** paths to any AGPL crate.
2. **RD9 No-Injection Check**:
   - Grepping both `crates/dainty-launcher-kit` and `src-tauri` for `OpenProcess`, `VirtualAllocEx`, `WriteProcessMemory`, `CreateRemoteThread`, or `LoadLibrary` injection calls returns **0 matches**.
3. **Bridge & Generated File Parity**:
   - `cargo test -p dainty-launcher-kit` and `cargo test -p dainty-launcher` pass all unit and integration tests across default features, `--features server-gui`, and `--no-default-features`.
   - `every_command_is_served_and_nothing_else`, `name_lists_are_the_enums`, and `generated_files_are_current` pass without diff under both default features and `--features server-gui`.
4. **Fuzz Target Compatibility**:
   - `fuzz/fuzz_targets/launcher_host.rs` and `fuzz/fuzz_targets/launcher_region.rs` compile cleanly against `launcher-kit`.
