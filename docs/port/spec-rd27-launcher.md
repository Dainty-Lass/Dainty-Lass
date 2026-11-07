<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# RD27 Clean-Room Specification: the Tauri launcher backend (`launcher/` in dainty-restricted)

This is the specification writer's output for relicensing dainty-restricted's launcher to MIT: the Tauri backend
`launcher/src-tauri` (the `dainty-lass-client` and `dainty-lass-manager` binaries) and the launcher's JavaScript and
TypeScript under `launcher/`. It follows [`spec-dl4-launcher.md`](spec-dl4-launcher.md) (DL4a) in format and
supersedes it where they differ (§1.5 lists where). Measurement follows
[`clean-room/similarity.md`](clean-room/similarity.md). Sources: dainty-restricted `feat/rd27-plugin-abi` at
`849f45e`; dainty-lass `feat/rd27-plugin-abi` at `92e797a`.

> **Clean-room rule.** The implementer of a Category B item reads this document, dainty-lass's MIT crates
> (`crates/launcher-kit`, `protocol`, `proxy`, `epic`, `module-api`, `controller`, `hostkit`, `admin`, `remote`),
> the generated interface files named in §2.3.6 and nothing else: not `launcher/` in dainty-restricted, not
> `legacy/UndauntedLauncher` (Dauntless Revived, DR), not `legacy/ReforgedLauncher` (Reforged's Electron launcher),
> not `launcher/` or `rust/launcher/` at any commit of either repository (dainty-lass held them until the split,
> `2e04dd6`). Category A files are moved, not rewritten. Nothing written from
> this document names DR's or Reforged's files, commits or TypeScript identifiers in comments.

The host half is [`spec-rd27-host.md`](spec-rd27-host.md). The rows that carry this out, and the maintainer's
decisions L1–L5, are in [`plan-rd27-launcher.md`](plan-rd27-launcher.md).

> **Refreshed (RL1, 2026-11-06)** against `restricted/launcher` at `60b984d4`, after dainty-restricted was imported
> into this repository as `restricted/` (2026-11-05). Read "dainty-restricted" below as `restricted/` and
> "dainty-lass" as this repository's root. In the clean-room rule, the excluded tree is `restricted/` (all of it)
> and the excluded history is every commit's `launcher/` and `restricted/launcher/`. What changed since `849f45e`:
>
> - **Names**: the binaries are `dainty-launcher` and `dainty-manager`; the host kit is `dainty-host`.
> - **New behaviour, specified here**: the tray (§2.2, §2.2.1), the Game log read from the game's console
>   (§2.7.16, command 54 `game_log_get`, event `game-log`), the settings `minimizeToTray`, `closeToTray` and
>   `showGameConsole` (§2.3.3, §2.4.2, §2.4.10, §2.4.11, §2.7.2), the backend keys `tray_open` and `tray_quit`
>   (§2.8.2: 14 keys, not 12), and `RUSTIC_RIFTS_` among the stripped prefixes (§2.7.5). The realm adapter speaks
>   the D3 routes and headers (`/rustic/v1/...`, `x-rustic-host-capability`).
> - **Licences**: `rifts-host`, `rifts-seam`, gateway, content, chat and every `rustic-isles-*` crate are MIT at the
>   root, and `restricted/crates/host` links only MIT crates. So realms and the manager's embedded server no longer
>   force the binaries to AGPL. §3.6 and §4 are rewritten for this.
> - **Line ranges** hold: `controller/host.rs` changed only in names since, so its B part is still lines 1–1171 and
>   its born-here tail (the server GUI's in-process host kit) starts at 1172.
> - **Done since**: the launcher's JavaScript (the isolation hook, the TS tests, the scripts) is MIT in `launcher/`
>   ([`clean-room/launcher-js.md`](clean-room/launcher-js.md)); `test/cases/manifest.json` is gone.

---

## 0. Summary

- 142 tracked files: 86 Rust sources, `build.rs`, 55 other files (manifests, configs, the isolation hook,
  generated files, icons, tests, scripts).
- DL4b already made `crates/launcher-kit` (MIT). Nine launcher files are now one-line re-exports of it, and three
  more take part of their content from it.
- What is still derived is DR's Tauri backend as fitted to Reforged (the RA rows): the app shell, the bridge,
  the controller, the network layer, the platform layer and the core rules. 58 Rust sources (about 21,000 lines
  with their tests) are Category B.
- launcher-kit already has clean-room versions of seven of them (`pinned`, `http`, `to_file`, `linux_runtime`,
  `system`, `core/text`, `core/pins`) that the shell never adopted because their APIs differ. For those the work is
  closing a documented gap, not a rewrite.
- *(Superseded by the RL1 refresh above and §3.6: realms and the embedded server are no longer AGPL.)* After the
  rewrite, the only AGPL parts a launcher binary still needs are the realm transport (`rifts-host`,
  `rifts-seam`, feature `realms`), the embedded server of the manager (feature `server-gui`, through
  `crates/host`), the language catalogs' text and DR's recorded test tables (§3).

---

## 1. File classification

### 1.1 Evidence

- **History.** The launcher came in as one commit, `a6e1870` ("rust/launcher: the Tauri launcher", 2026-10-23):
  "DR's launcher backend (src-tauri, isolation) with Reforged's identity and the page build". Its objects remain in
  dainty-lass. dainty-restricted keeps the history since the new root `7338b10` (2026-10-26); rows after that are
  named in file headers and commit subjects (DE2–DE5, DN7, DU3, DH3b, DH6, RF7, RD26, B3, RM2–RM5, RA14).
- **NOTICE.** restricted `NOTICE.md`: "`launcher/src-tauri` carries the DR-derived core of its launcher's Rust side
  alongside `crates/launcher-kit`", and "the DR-derived Tauri shell in `launcher/src-tauri` (including
  `net/epic_install.rs`, which keeps DR's downloader structure)".
- **Headers.** Most files name their DR row (`L5`–`L46`) or their Reforged source (`src/main/*.ts`) in their first
  lines; born-here files name DaintyLass rows.
- **DL4.** `spec-dl4-launcher.md` §1.2 classified the same tree before DL4b; §1.5 lists where this audit differs.
- **Similarity against DR.** The `similarity.md` measure (code lines in runs of 3 or more identical trimmed,
  non-comment lines), taken against every file of DR's `legacy/UndauntedLauncher` at `e1c5fec` at once. Reforged's
  launcher is TypeScript, so its influence does not show as matching lines: it is established from headers, the
  RA row a file was written in, and `delta-launcher-*.md`.

### 1.2 Categories

- **A**: born in DaintyLass; moves to MIT as it is. The table names what it depends on.
- **B**: derived from DR (shared lines, kept structure) or ported from Reforged's TypeScript (an RA row's
  translation); rewritten clean-room from §2. A B file with born-here parts says so; those parts move as they are.
- **G**: generated by a test or `build.rs` from other files; it follows its generator and its inputs.
- **D**: test data recorded from DR's or Reforged's programs.
- **Asset**: images.

### 1.3 Rust sources (`launcher/src-tauri/src/`)

| Path | Cat. | vs DR | Evidence | Depends on | Target (§4) |
| --- | --- | ---: | --- | --- | --- |
| `lib.rs` | B | 60.8% | Header: "on Dauntless Revived's launcher backend (e1c5fec)"; RA1, RA13; born-here parts: Epic store, embedded server wiring | Tauri, all modules | MIT shell crate (§4.2) |
| `main.rs` | B | 33.3% | DR's entry point (windows subsystem, one binary); RD26 picks the binary by feature | `lib.rs` | MIT shell crate |
| `window.rs` | B | 87.0% | Rows L5, L10, RA11 | Tauri, `dialogs`, `paths` | MIT shell crate |
| `bridge/mod.rs` | B | 33.3% | Module root, DR layout | — | MIT shell crate |
| `bridge/backend.rs` | B | 92.2% | Row L38 | Tauri `Channel` | MIT shell crate |
| `bridge/progress.rs` | B | 80.4% | Row L7 | — | launcher-kit `bridge` |
| `bridge/spec.rs` | B | 19.4% | Rows L6, L7, L9 (DR's command table design); RA2 (Reforged's `LauncherApi` list); born-here entries from DF8 on | `types` | launcher-kit `bridge::spec` (§2.3) |
| `bridge/types.rs` | B | 34.0% | DR's wire types fitted to Reforged's `src/shared/types.ts`; born-here types (Epic, installs, guarded, history, realms, doctor, config, remote) | launcher-kit `bridge::types` (already holds the rest) | launcher-kit `bridge::types` |
| `bridge/commands.rs` | B | 24.3% | Header: one command per `LauncherApi` method of Reforged's `preload.ts`, checks as `ipc-validate.ts`; born-here commands from `epic_*` on | controller, `spec`, `types` | MIT shell crate (commands) + launcher-kit (checks) |
| `bridge/generate.rs` | B | 26.7% | Rows L6, L8, L9; RA2 | `spec`, `types`, ts-rs | launcher-kit test (§2.3.6) |
| `bridge/modules.rs` | A | 0% | Re-export of `launcher_kit::bridge::modules` (DU3) | launcher-kit | delete (use launcher-kit) |
| `controller/mod.rs` | B | 64.8% | Header: row L38, "Reforged's `src/main/controller.ts`", DR's locking design | everything below | launcher-kit `controller` (§2.4) |
| `controller/platform.rs` | B | 74.6% | `Platform` of `controller.ts` on DR's trait | Tauri (impl only) | trait: launcher-kit; impl: MIT shell |
| `controller/stored.rs` | B | 73.7% | `StoredSettings` of Reforged's `settings.ts` | `keystore`, `settings` | launcher-kit `controller` |
| `controller/game.rs` | B | 14.2% | Reforged's `controller.ts` 949–1077 (RA13) | `game_folder`, `verify` | launcher-kit `controller` |
| `controller/runtime.rs` | B | 17.2% | RA13 on RA7; DL4 §2.8 | `runtime_files`, `hostapi` | launcher-kit `controller` |
| `controller/host.rs` | B | 3.3% | Lines 1–1171: RA12, Reforged's Host page controller. Lines 1172–2153: born here (DU3 in-process kit, B3 doctor/backup/config/migration/invites, RM4 remote management) | launcher-kit `hostkit`, `hostserver`, `requirements`, `loader_session`; the tail: `host` (restricted shell), `admin`, `remote` | B part: launcher-kit `controller::host`; A tail: launcher-kit behind `server-gui` (§4.3) |
| `controller/host_off.rs` | A | 0% | Refusals with `host.rs`'s signatures for a player-only build | `types` | with `controller` |
| `controller/tests.rs` | B | 14.5% | Header: "Reforged's `test/controller.test.ts`, on DR's Rust harness" | controller | launcher-kit tests (§2.10) |
| `controller/guarded.rs` | A | 10.0% | DD23, DH6 | `admin` (MIT) | launcher-kit `server-gui` |
| `controller/install.rs` | A | 6.4% | DE5 | `epic`, `epic_install` | launcher-kit |
| `controller/loader.rs` | A | 12.5% | RD9 | `loader_session` | launcher-kit |
| `controller/realms.rs` | A | 3.3% | RF7, DU5; D3 routes since `a260cf2e` | `rifts-host`, `rifts-seam` (MIT since RD27) | launcher-kit `controller::realms` behind `realms` (§4.3) |
| `controller/realms_off.rs` | A | 0% | RF7 stub | — | launcher-kit |
| `controller/tests/realms.rs` | A | 2.3% | RF7; scripted transport since `a260cf2e` | `rifts-host` (MIT) | launcher-kit tests |
| `controller/region.rs` | A | 0% | DN7 | `regionping` | launcher-kit |
| `controller/revived.rs` | A | 0% | RD5/RD22 refusal stubs | — | launcher-kit (or delete with the 1.4.4 commands) |
| `controller/versions.rs` | A | 8.6% | DE4 | `versions` | launcher-kit |
| `core/mod.rs` | B | 87.5% | Rows L11–L18 | — | delete |
| `core/game_language.rs` | B | 100% | Row L16 | `types` | launcher-kit `core` (§2.8) |
| `core/i18n.rs` | B | 47.6% | Rows L14, RA3 | `build.rs` output | launcher-kit `core::i18n` (§2.8, §3.1) |
| `core/invite.rs` | B | 74.4% | Row L11; re-export of `protocol::invite` plus a table runner | `protocol` | delete (use `protocol`) |
| `core/status.rs` | B | 39.2% | Row L12; re-export of `protocol::status` | `protocol` | delete |
| `core/username.rs` | B | 77.3% | Row L12; re-export of `protocol::account` | `protocol` | delete |
| `core/pins.rs` | B | 77.8% | Row L18 constants | — | delete (launcher-kit `core::pins`) |
| `core/text.rs` | B | 100% | DR's JavaScript string rules | `protocol::text` | delete (launcher-kit `core::text`) |
| `net/mod.rs` | B | 27.3% | Rows L19–L25 | — | launcher-kit `net` |
| `net/connector.rs` | B | 100% | Row L19 | `pinned` | launcher-kit `net::connector` (§2.6.2) |
| `net/pinned.rs` | B | 100% | Row L19, `pinned.ts` | rustls | launcher-kit `net::pinned` (exists; §2.6.1) |
| `net/http.rs` | B | 91.3% | Row L20 | `pinned` | launcher-kit `net::http` (exists; §2.6.1) |
| `net/http/to_file.rs` | B | 23.3% | Reforged's `http.ts` 133–203 (RA7) | `http` | launcher-kit (exists; §2.6.1) |
| `net/hostapi.rs` | B | 49.8% | Row L21, `hostapi.ts` | `http`, `protocol` | launcher-kit `net::hostapi` (§2.6.3) |
| `net/hostapi/parse.rs` | B | 58.8% | Row L21; fuzz target `launcher_net` | `protocol` | launcher-kit |
| `net/hostapi/tests.rs` | B | 30.1% | DR tests + `test/cases/hostapi.json` | — | §2.10 |
| `net/relay.rs` | B | 58.1% | DR contract C4, rows L22, L33, RA6, `relay.ts`; born-here part: re-pointing with session tickets (DN) | `connector`, `proxy` | launcher-kit `net::relay` (§2.6.4) |
| `net/relay/local.rs` | B | 68.6% | Fuzz target `relay_input` | std | launcher-kit |
| `net/relay/tests.rs` | B | 46.5% | Reforged's `relay.test.ts` cases | — | §2.10 |
| `net/relay_owner.rs` | B | 91.4% | RD21; DR rows L22, L31 | `launch` | launcher-kit (§2.6.5) |
| `net/relay_owner/table.rs` | B | 87.0% | Fuzz target `relay_input` | std | launcher-kit |
| `net/epic_install.rs` | B | 32.5% | Header: DR's `net/downloader.rs` structure kept (NOTICE names it) | `epic` | launcher-kit (§2.6.6) |
| `net/regionping.rs`, `net/runtime_files.rs`, `net/verify.rs`, `net/versions.rs` | A | 0% | Re-exports of launcher-kit (DL4b) | launcher-kit | delete |
| `platform/mod.rs` | B | 37.0% | Rows L26–L37 | — | launcher-kit `platform` |
| `platform/art.rs` | B | 92.3% | Row L37 | Tauri `http` types | MIT shell crate (§2.7.10) |
| `platform/clipboard.rs` | A | 5.6% | RD22, RA11: Reforged's handler was one Electron call; this Win32 and `wl-copy`/`xclip`/`xsel` implementation has no counterpart | `windows` crate | launcher-kit |
| `platform/dialogs.rs` | B | 74.4% | Rows L36, RA11 | Tauri dialog plugin, `i18n` | MIT shell crate (§2.7.11) |
| `platform/epic_library.rs`, `epic_login.rs`, `epic_session.rs` | A | 9.0%, 9.6%, 4.1% | DE2, DE3 | `epic`, Tauri (login window) | launcher-kit / MIT shell (login window) |
| `platform/keystore/epic.rs` | A | 19.3% | DE2; the matched lines are its test module's fake encryptor (22 lines, from DR's keystore tests): rewrite that fake when moving | `keystore` | launcher-kit |
| `platform/keystore/mod.rs` | B | 64.4% | Row L27, `keystore.ts` | `format`, `log` | launcher-kit `platform::keystore` (§2.7.1) |
| `platform/keystore/format.rs` | B | 19.7% | `RRK2` (RD22) and Electron framing; fuzz `launcher_files` | `protocol` | launcher-kit |
| `platform/keystore/legacy.rs` | B | 79.1% | D7, L2, L27 | aes-gcm, cbc, pbkdf2 | launcher-kit |
| `platform/keystore/os.rs` | B | 95.1% | Row L27 | `windows`, `dbus-secret-service` | launcher-kit |
| `platform/game_config.rs` | B | 72.2% | Rows L30, RA9; Reforged's `engineini.ts`, `display-repair.ts` | `core::text` | launcher-kit (§2.7.6) |
| `platform/game_config_tests.rs` | B | 32.8% | `test/cases/game-config.json` + Reforged cases | — | §2.10 |
| `platform/game_folder.rs` | B | 19.2% | Rows L30, RA9; Reforged's `game-folder.ts` | `core::pins` | launcher-kit (§2.7.7) |
| `platform/hostkit.rs`, `hostserver.rs`, `requirements.rs`, `loader_session.rs` | A | 0% | Re-exports of launcher-kit (DL4b); `hostkit.rs` also builds the test stand-in | launcher-kit | delete |
| `platform/invite_link.rs` | B | 46.3% | Row L34 | `protocol`, `paths` | launcher-kit (§2.7.8) |
| `platform/launch.rs` | B | 52.9% | Rows L31, RA9; `launch.ts` | `game_language`, `protocol` | launcher-kit (§2.7.5) |
| `platform/links.rs` | B | 24.8% | Row L35, Reforged's `links.ts` | url, `open` | launcher-kit (§2.7.9) |
| `platform/linux_runtime.rs` | B | 100% | Row L32 | `launch`, `log` | launcher-kit (exists; §2.7.13) |
| `platform/log.rs` | B | 82.7% | Row L29, `log.ts` | regex, tracing | launcher-kit (§2.7.4) |
| `platform/paths.rs` | B | 28.3% | Row L26 | dirs | launcher-kit (§2.7.3) |
| `platform/settings.rs` | B | 48.8% | Row L28, `settings.ts`; fuzz `launcher_files` | `protocol` | launcher-kit (§2.7.2) |
| `platform/system.rs` | B | 100% | Row L33 | fs4 | launcher-kit (exists; §2.7.14) |
| `platform/updater.rs` | B | 39.5% | Row L46, RD7 | Tauri updater plugin | MIT shell crate (§2.7.12) |
| `server_gui.rs` | A | 12.8% | RD26 | **`host` (restricted shell)**, `controller`, `service-kit` | restricted until RD27 phase 2 (§4.3) |
| `server_gui/modules.rs`, `server_gui/modules/tests.rs` | A | 6.1%, 0% | DU3, DF3 | `host::hostconfig` (= hostkit), `controller` | launcher-kit `server-gui` |
| `server_history.rs` | A | 2.6% | DD23, DH3b | `admin` | launcher-kit `server-gui` |
| `tray.rs` | A | not measured (RL14) | Written here in `d1a07964` (2026-11-05); no DR or Reforged counterpart | Tauri tray and menu, `window`, `i18n` | MIT shell crate (§2.2.1) |
| `platform/game_console.rs` | A | not measured (RL14) | Written here in `e55518d2` (2026-11-05) | `windows` crate (Windows only) | launcher-kit `platform` (§2.7.16) |
| `tests/bundles.rs` | A | 0% | RA14, `67e5fd67`: the two bundle configs | the Tauri configs | MIT shell crate tests |

### 1.4 Everything else under `launcher/`

| Path | Cat. | Evidence | Target |
| --- | --- | --- | --- |
| `src-tauri/build.rs` | B | 79.8% vs DR; rows L8, L14 | MIT shell crate (§2.1.4) |
| `src-tauri/Cargo.toml` | B | DR's manifest fitted (43.6% of lines; dependency lines and feature comments) | rewritten with the shell crate |
| `src-tauri/tauri.conf.json` | B | 69.2% vs DR's | rewritten (§2.1.3) |
| `src-tauri/tauri.server-gui.conf.json` | A | RD26 | moves |
| `src-tauri/capabilities/main.json` | B | 100% vs DR's (16 lines) | rewritten (§2.1.3) |
| `src-tauri/permissions/app-commands.toml` | G | from `bridge/spec.rs` | regenerated, identical (§2.3.6) |
| `src-tauri/gen/strings.json` | G | from the language catalogs | non-relicensable text (§3.1) |
| `src-tauri/linux/launcher.desktop` | B | DR's template; only `MimeType` differs | rewritten (§2.1.3) |
| `src-tauri/linux/manager.desktop` | A | RD26 | moves |
| `src-tauri/fixtures/host/src/fake_host_kit.rs` | B | Header: stand-in for Reforged's `hostserver.test.ts` fake | rewritten (§2.10) |
| `src-tauri/fixtures/host/Cargo.toml`, `Cargo.lock` | A | trivial manifest | moves |
| `src-tauri/.gitignore`, `.gitignore` | A | trivial | moves |
| `src-tauri/icons/**` (14), `brand/icon.png` | Asset | Not byte-identical to DR's or Reforged's `icon.ico`; their origin before the new root is not recorded | maintainer confirms (§3.5) |
| `isolation/hook.js` | B | 68.2% vs DR's | rewritten (§2.3.7) |
| `isolation/index.html` | B | 100% vs DR's (11 lines of markup) | rewritten (§2.3.7) |
| `isolation/commands.js` | G | from `bridge/spec.rs` | regenerated, identical |
| `package.json`, `package-lock.json` | A | Written for DaintyLass (scripts name the dashboard pages); the lock file is generated | moves |
| `tsconfig.json` | A | Stock compiler options (4 matching lines) | moves |
| `README.md` | A | RD26 | moves |
| `scripts/dashboard-page.mjs`, `release-config.mjs`, `sign-windows.ps1` | A | RA14 and the dashboard pages (2026-11-01) | moves |
| `test/api.test.ts` | B | 40.7% vs DR's `test/api.test.ts` | rewritten (§2.9) |
| `test/isolation-hook.test.ts` | B | 23.3% vs DR's | rewritten (§2.9) |
| `test/hook.ts`, `test/resolve-ui.ts`, `test/bundles.test.ts` | A | DU2, RD26 | move |
| `test/e2e-desktop.mjs`, `test/e2e-remote-desktop.mjs` | A | RD26, DD27/RM3/RM4 | move |
| `test/electron-keystore/**` (5 files) | A | RA8; written here, but runs Reforged's own `keystore.ts` and `settings.ts` inside Electron | stays with restricted (needs those sources) |
| `test/cases/game-config.json` | D | 99.8% vs DR's table | §3.2 |
| `test/cases/hostapi.json` | D | 100% vs DR's | §3.2 |
| `test/cases/log.json` | D | 51.2% vs DR's | §3.2 |
| `test/cases/settings.json` | D | 78.6% vs DR's | §3.2 |
| `test/cases/manifest.json` | D | 100% vs DR's; read by nothing since RD5 removed 1.4.4 | delete |

### 1.5 Counts, and where this differs from DL4

| | A | B | G | D | Asset | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Rust sources | 28 | 58 | — | — | — | 86 |
| Other files | 23 | 10 | 3 | 5 | 15 | 56 |
| **Launcher total** | **51** | **68** | **3** | **5** | **15** | **142** |

Since the RL1 refresh: three more A files (`tray.rs`, `platform/game_console.rs`, `tests/bundles.rs`), so 31 A Rust
sources; the JavaScript and TypeScript rows of §1.4 (the isolation hook, `test/*.ts`, the scripts, `package.json`,
`tsconfig.json`) are MIT in the root's `launcher/` already, and `restricted/launcher/isolation/hook.js` is a copy of
it; `test/cases/manifest.json` is deleted.

Differences from DL4 §1.2, found in this audit:

- `controller/host.rs` and `controller/runtime.rs` were DL4 Rewrite modules, but DL4b never moved them: they are
  still the RA12/RA13 code in restricted. DL4 §2.7 and §2.8 are also inaccurate against the code (for example the
  Dumper-7 session's game arguments, the error codes of `install_runtime`, and `trust_runtime` always installing);
  §2.5 below replaces them.
- `controller/host.rs` has a born-here half (lines 1172 on) that DL4 did not know about.
- `window.rs`, `core/i18n.rs`, `platform/{linux_runtime,updater,log,system}.rs` are 40–100% DR lines, so they are
  B, not "Born-MIT / DL4 Rewrite".
- `platform/clipboard.rs` (DL4: DR core) is A: Reforged's handler was a single Electron call; this implementation
  has no DR or Reforged counterpart.
- `platform/keystore/epic.rs` is A but carries DR's test fake (rewrite that fake).
- `net/pinned.rs`, `net/http.rs`, `net/http/to_file.rs`, `platform/linux_runtime.rs`, `platform/system.rs`,
  `core/text.rs`, `core/pins.rs` already have clean-room counterparts in launcher-kit with different APIs; §2.6.1,
  §2.7.13 and §2.7.14 give the gap.
- `test/cases/manifest.json` is dead.

---

## 2. Behaviour specification of the Category B groups

Conventions: "the page" is the web page in the main window; "a command" is a Tauri command the page calls;
"answers" means the command's return value as JSON. UTF-16 lengths are JavaScript `length`s. All limits, file
names, header names, ports and paths below are contracts: other programs, older launchers or the game depend on
them (plan section 6). Where text is a user-visible string from the language catalogs, this document gives its
**key** only (§3.1).

### 2.1 The application shell (`lib.rs`, `main.rs`, `build.rs`, the configs)

#### 2.1.1 Binaries and features

- One binary target. `tauri build` names it from the config: `dainty-launcher` (`tauri.conf.json`) or
  `dainty-manager` (`tauri.server-gui.conf.json`, built with `--no-default-features --features server-gui`).
  Before anything else, `main` checks whether this process is the game console's reader (§2.7.16: first argument
  `--read-game-console`); if so it runs only that and exits with its code.
  `main` runs the manager when the `server-gui` feature is on, the client otherwise; a start failure prints
  `DaintyLass could not start: <error>` to stderr and exits 1. Release builds on Windows have no console window.
- Features: `host` (default; the Host page backend), `server-gui` (implies `host`; the server inside the
  launcher), `realms` (realm hosting and joining), `isles-1-14-7`, `stub-module`, `all-modules` (= `isles-1-14-7`
  + `realms`), `custom-protocol` (set by `tauri build`). Every build registers every command (§2.3.1).
- The crate's version equals `launcher/package.json`'s (`2.0.0-beta`), checked by a test.

#### 2.1.2 Start-up, in order

1. Read the process arguments. On Linux, if `REFORGED_PARENT_PID` names a process, wait until `/proc/<pid>` is gone
   (at most 100 × 100 ms): a launcher restarted after installing secure-storage packages must not hand its
   arguments to the old one.
2. **Single instance** (§2.7.8): try to hand the arguments to a running launcher on this user's channel; if handed,
   exit 0 at once.
3. Install the logger (§2.7.4) and the rustls `aws-lc-rs` crypto provider as the process default.
4. Resolve the folders (§2.7.3); none (no home folder) is a start error. Log to `<userData>/logs`. Log
   `DaintyLass Launcher <version> starting`.
5. Make the key encryptors (§2.7.1). **Linux only:** if the current encryptor is unavailable (no Secret Service),
   run the secure-storage bootstrap: log a warning; unless this is a development build, `REFORGED_AUTO_INSTALL=0`, or
   `REFORGED_STORAGE_BOOTSTRAPPED=1`, find the package-manager plan for the secure-storage dependency (§2.7.13),
   run it, and on success start this executable again with the same arguments plus
   `REFORGED_STORAGE_BOOTSTRAPPED=1` and `REFORGED_PARENT_PID=<this pid>`, then exit 0. Any failure logs and
   continues without key storage.
6. Make the Epic ownership cache in `userData` (A code).
7. Keep an invite link found in the arguments (§2.7.8) for the page.
8. Build the Tauri app with: the dialog plugin; the updater plugin; managed state for updates, the close guard
   (§2.2), the art store (§2.7.10), the bridge backend (§2.3.5), the Epic ownership cache; the asynchronous URI
   scheme `rr-art` answered by the art store; the command handler with the 100 commands **in the order of §2.3.1**
   (a test reads the handler list and compares it with the command table).
9. In the setup step: remember the app handle for second-launcher messages; in a packaged client build (not a
   development build, not the manager, and without `REFORGED_DEV_PROFILE`) register the invite schemes for this
   executable (§2.7.8); restore the Epic session from `userData/keys` (A code); make the controller (§2.4.1) with
   this PC's folders, the resources folder (development builds: the repository's `brand/launcher`), the relay port
   (`REFORGED_RELAY_PORT` when it passes §2.6.4's override rule, else 61100), the pinned executable
   (`Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe`, SHA-256 `f3f95fbf…b66b`), the status poll interval
   (15 s), the OS language (§2.8.2) and locale (default `en-US`), the game's default config folder (§2.7.6), the
   platform (`win32`/`linux`) and the encryptors; wire the close guard to the controller (reason: "keep open" while
   the relay runs; on keep: set notice `keep_open`); with an embedded server (manager), register its module panel as
   managed state and hand the server to the controller; manage the controller; create the window (§2.2); in packaged
   builds start the updater (§2.7.12); start `controller.init()` in the background.
10. **Exit:** on the first exit request, cancel it, run `controller.shutdown()` (§2.4.12), then exit with code 0.
    Later exit requests pass.

#### 2.1.3 Configuration files (rewritten, same effect)

- **`tauri.conf.json`** (client): product name `DaintyLass Launcher`, main binary `dainty-launcher`, version
  from `../package.json`, identifier `io.github.freezamotoqueiro.dauntlessreforgedrevival` (kept so installs update
  in place). Dev URL `http://localhost:4321`; frontend `../dist/launcher`; before-dev and before-build commands run
  `bun scripts/lass.ts dashboard-page dev|build launcher` (the manager's: `... server-manager`, frontend
  `../dist/server-manager`, identifier `io.github.freezamotoqueiro.dauntlessreforgedrevival.server`, product name
  `DaintyLass Manager`, main binary `dainty-manager`). No windows declared (the backend makes the one window). No
  global Tauri object. Content Security Policy: `default-src 'none'`; scripts, styles, fonts and connections from
  `'self'` only; images from `'self'`, `rr-art:`, `http://rr-art.localhost` and `data:`; `base-uri`, `form-action`,
  `frame-ancestors` and `object-src` none. Prototype freezing on. The **isolation pattern**, with the folder
  `../isolation`. Bundles: NSIS (current user, the app icon), AppImage (no media framework), deb (depends
  `libwebkit2gtk-4.1-0`, recommends `gnome-keyring` and `pkexec`, section `games`, desktop template
  `linux/launcher.desktop`) and rpm (depends `webkit2gtk4.1`, the same template); the five icons; publisher
  `DaintyLass contributors`, homepage the dainty-lass repository, category `Game`, and short and long descriptions.
  Updater: an empty public key in the repository (release builds supply it), the endpoint of the
  `launcher-updates` release's `latest.json`, passive install mode on Windows. The `license` field becomes the
  shipped binary's licence (§4.5).
- **`capabilities/main.json`**: one capability `main` for the window `main`, granting the permission set
  `app-commands` (§2.3.6), Tauri's default event permissions, and the window permissions minimize,
  toggle-maximize, internal-toggle-maximize, is-maximized, start-dragging and close. Nothing else: no file
  system, shell, HTTP, opener or dialog.
- **`linux/launcher.desktop`**: Tauri's desktop template for the client, with Tauri's placeholders for categories,
  comment, exec, icon and name, `StartupWMClass` = the exec name, `Terminal=false`, `Type=Application`, and
  `MimeType=x-scheme-handler/dauntless;x-scheme-handler/reforged-revival;`.

#### 2.1.4 `build.rs`

- Reads `src/bridge/spec.rs` **as text** (a build script cannot use its crate) and takes the command names, in
  order, from the command table (tolerating a name on the line after the table entry opens). No names is a build
  error. These become Tauri's app manifest commands, so each command gets `allow-<command>`/`deny-<command>`
  permissions and only what the capability grants is callable.
- Reads `gen/strings.json` (an object: language code → object of key → text). Every code must be a two-letter
  lower-case language, optionally `-` and a two-letter upper-case region; at least one language. Writes
  `languages.rs` into the build output: a `Language` enum with one variant per code in file order (variant name:
  each `-` part capitalised, the rest lower-case: `en` → `En`, `pt-BR` → `PtBr`), serialised as the code, with
  `ALL`, `code()`, and one static table per language (`PT_BR` style names) holding its entries **sorted by key**
  for binary search. A non-text value is a build error.
- Re-runs when `src/bridge/spec.rs`, `permissions/`, `../isolation` or `gen/strings.json` change.

### 2.2 The window (`window.rs`)

- One window, label `main`, loading the app's `index.html`. Title (shown only by the task switcher) `Dainty Lass
  Launcher`, or `Dainty Lass Manager` in the manager. No decorations (the page draws its own title bar); created
  hidden with background colour `#03080F` (opaque) so nothing flashes; centred.
- **Size**: from the primary monitor's work area in logical pixels (work-area size ÷ scale factor): the window is
  `min(width, 1280) × min(height, 760)`, the minimum `min(width, 1024) × min(height, 640)`; with no monitor,
  1280 × 760 and 1024 × 640.
- **Navigation lock**: a navigation is allowed only to the app's own page: in a development build, any URL with the
  dev server's origin; otherwise `tauri://localhost/...` or `http(s)://tauri.localhost/...` (exact host). New
  windows are always denied.
- WebView data folder: `<userData>/WebView2` when the folders resolve.
- Shown when the page finishes loading, or after 10 s if it is still hidden.
- On every resize, emits `window-state` to the main window with `true` when maximised, else `false`.
- **Close guard**: on a close request, unless closing was already confirmed, ask the controller for a reason to
  stay open. With one (the relay carries a running game), prevent the close and show the keep-open dialog (§2.7.11)
  in the controller's language. Keep (the default, also on Esc or the dialog's own close button): minimise the
  window and, for the relay reason, call the "kept" callback (the controller shows notice `keep_open`). Close
  anyway: mark closing confirmed and close.
- **Tray settings** (read from the controller; both off until it exists). On a resize that leaves the window
  minimised: with `minimizeToTray`, hide it to the tray (§2.2.1), and do nothing else for that event (no
  `window-state`). On a close request, before the guard above: unless the tray's Quit asked for this close (a
  one-shot "quitting" flag, cleared as it is read), with `closeToTray` prevent the close and hide to the tray; a
  hidden window keeps the relay, so nothing is confirmed.

#### 2.2.1 The tray (`tray.rs`, A)

The icon exists only while the window is hidden to it. Hiding makes it if missing (id `main`, the app's window
icon, tooltip `app_title`, a menu with `tray_open` and `tray_quit`, the menu not shown on a left click); when the
tray can't be made (a Linux desktop without one), log it at info and leave the window as it is. Left click (on
release) or Open: show, unminimise and focus the window, and remove the icon. Quit: set the "quitting" flag,
restore the window and close it, so the close still passes the running game's keep-open dialog. A second launcher
(§2.7.8) restores the window the same way.

### 2.3 The bridge

#### 2.3.1 The command table

The page may call exactly these 100 commands, in this order (the order of the handler list, `commands.js`, the
permission set and `CommandName`). Argument names are the page's (camelCase). Kinds: **text ≤n** a string of at
most n UTF-16 units; **optional text** that, or absent, or null; **flag** a boolean; **index 0..n** an integral
number in range; **name** one of a fixed list of strings; **number a..b** a finite number in range; **object** an
object with exactly those fields (each may be absent only if its kind allows), or null when it says so;
**settings patch** an object whose keys are settings names and whose values are strings (≤16 units in the hook),
numbers or booleans; **channel** a Tauri channel. "Build" says which builds serve it (§2.3.2).

| # | Command | Arguments | Build |
| ---: | --- | --- | --- |
| 1 | `get_snapshot` | — | every |
| 2 | `check_invite` | `text`: text ≤2048 | every |
| 3 | `submit_invite` | `text`: text ≤2048; `acceptNewCertificate`: flag; `carryAccount`: flag | every |
| 4 | `retry_connect` | — | every |
| 5 | `forget_server` | — | every |
| 6 | `switch_server` | `id`: text ≤24 | every |
| 7 | `remove_saved_server` | `id`: text ≤24 | every |
| 8 | `register` | `username`: text ≤32 | every |
| 9 | `use_existing_key` | `text`: text ≤8192 | every |
| 10 | `import_key_file` | — | every |
| 11 | `save_key_backup` | — | every |
| 12 | `dismiss_backup_offer` | — | every |
| 13 | `choose_game_folder` | — | every |
| 14 | `use_game_path` | `gamePath`: text ≤260 | every |
| 15 | `use_detected_game` | `index`: index 0..15 | every |
| 16 | `verify_game` | — | every |
| 17 | `revived_install` | `mode`: name (install, repair) | every |
| 18 | `revived_choose_folder` | — | every |
| 19 | `revived_use_detected` | `index`: index 0..15 | every |
| 20 | `trust_runtime` | — | every |
| 21 | `install_runtime` | — | every |
| 22 | `choose_local_runtime` | — | every |
| 23 | `clear_local_runtime` | — | every |
| 24 | `cancel_task` | — | every |
| 25 | `play` | — | every |
| 26 | `set_settings` | `patch`: settingsPatch | every |
| 27 | `set_status_polling` | `on`: flag | every |
| 28 | `refresh_status` | — | every |
| 29 | `get_news` | — | every |
| 30 | `get_branding` | — | every |
| 31 | `open_external` | `target`: name (24 names, §2.3.3) | every |
| 32 | `open_game_folder` | — | every |
| 33 | `open_logs_folder` | — | every |
| 34 | `logout` | — | every |
| 35 | `install_update` | — | every |
| 36 | `host_get` | — | host |
| 37 | `host_choose_workspace` | — | host |
| 38 | `host_choose_dumper` | — | host |
| 39 | `host_import_gamedata` | — | host |
| 40 | `host_import_sdk_folder` | — | host |
| 41 | `host_download_project` | — | host |
| 42 | `host_action` | `action`: name (11 names, §2.3.3) | host |
| 43 | `host_open` | `folder`: name (7 names, §2.3.3) | host |
| 44 | `host_server_get` | — | host |
| 45 | `host_server_start` | — | host |
| 46 | `host_server_stop` | — | host |
| 47 | `host_server_restart` | `name`: name (11 names, §2.3.3) | host |
| 48 | `host_setup` | `request`: object {name: text ≤60; mode: name (private, public); publicHost: optional text ≤253} | host |
| 49 | `host_create_invite` | `forName`: optional text ≤40 | host |
| 50 | `host_update` | — | host |
| 51 | `host_set_multipliers` | `multipliers`: object or null {xp: number 1..100; rewards: number 1..100} | host |
| 52 | `copy_text` | `text`: text ≤4096 | every |
| 53 | `take_invite_link` | — | every |
| 54 | `game_log_get` | — | every |
| 55 | `dismiss_notice` | — | every |
| 56 | `dismiss_error` | — | every |
| 57 | `subscribe_progress` | `onProgress`: channel | every |
| 58 | `epic_sign_in_start` | — | every |
| 59 | `epic_sign_in_code` | `code`: text ≤1024 | every |
| 60 | `epic_get` | — | every |
| 61 | `epic_sign_out` | — | every |
| 62 | `epic_library_check` | — | every |
| 63 | `game_version_list` | — | every |
| 64 | `game_install_start` | `version`: text ≤64 | every |
| 65 | `game_install_pause` | `version`: text ≤64 | every |
| 66 | `game_install_resume` | `version`: text ≤64 | every |
| 67 | `game_install_cancel` | `version`: text ≤64 | every |
| 68 | `game_install_get` | `version`: text ≤64 | every |
| 69 | `region_ping` | — | every |
| 70 | `module_list` | — | server-gui |
| 71 | `module_get` | `module`: text ≤64 | server-gui |
| 72 | `module_run_command` | `module`: text ≤64; `command`: text ≤64; `input`: text ≤8192 | server-gui |
| 73 | `module_set_settings` | `module`: text ≤64; `values`: text ≤8192 | server-gui |
| 74 | `module_load` | `module`: text ≤64 | server-gui |
| 75 | `module_unload` | `module`: text ≤64 | server-gui |
| 76 | `realm_host` | `kind`: name (hub, hunt) | realms |
| 77 | `realm_join` | `realm`: text ≤128 | realms |
| 78 | `realm_stop` | — | realms |
| 79 | `realm_get` | — | realms |
| 80 | `history_query` | `filter`: object {account: optional text ≤64; actor: optional text ≤64; since: optional text ≤40; until: optional text ≤40; cursor: optional text ≤128} | server-gui |
| 81 | `history_revert_dry_run` | `selection`: object {account: optional text ≤64; actor: optional text ≤64; since: optional text ≤40; until: optional text ≤40} | server-gui |
| 82 | `history_revert_apply` | `plan`: text ≤64 | server-gui |
| 83 | `guarded_list` | — | server-gui |
| 84 | `guarded_approve` | `id`: text ≤64 | server-gui |
| 85 | `guarded_cancel` | `id`: text ≤64 | server-gui |
| 86 | `server_doctor` | — | server-gui |
| 87 | `server_backup` | — | server-gui |
| 88 | `server_config_get` | — | server-gui |
| 89 | `server_config_save` | `config`: object {registration: name (OPEN, INVITECODE, NONE, INVITE); database: name (sqlite, postgres); maxHubs: index 0..64; maxHunts: index 0..128; freshStart: flag} | server-gui |
| 90 | `server_migrate_mongo` | — | server-gui |
| 91 | `server_confirm_fresh_start` | — | server-gui |
| 92 | `server_invite_list` | — | server-gui |
| 93 | `admin_action` | `request`: object {action: name (lookup, kick, timeout, ban, unban); player: text ≤64; reason: optional text ≤500; until: optional text ≤40; module: optional text ≤64} | server-gui |
| 94 | `remote_status` | — | server-gui |
| 95 | `remote_init_controller` | `request`: object {node: optional text ≤64; region: optional text ≤64; url: optional text ≤512} | server-gui |
| 96 | `remote_invite` | `request`: object {node: text ≤64; region: optional text ≤64; label: optional text ≤128; url: optional text ≤512} | server-gui |
| 97 | `remote_enroll` | `request`: object {uri: text ≤2048; label: optional text ≤128} | server-gui |
| 98 | `remote_revoke` | `node`: text ≤64 | server-gui |
| 99 | `remote_leave` | — | server-gui |
| 100 | `remote_exec` | `request`: object {node: text ≤64; action: name (9 names, §2.3.3); target: optional text ≤64; reason: optional text ≤500} | server-gui |

#### 2.3.2 Which build serves a command

- **server-gui**: the 27 commands from `module_list` to `remote_exec` that are not `realm_*` (rows 70–75 and 80–100).
- **realms**: `realm_host`, `realm_join`, `realm_stop`, `realm_get`.
- **host**: every other command whose name starts with `host_`.
- **every**: the rest.

A build outside a command's group still registers it and refuses it: `{ ok: false, error: { code:
"not_in_this_build", detail: "<command>" } }` for server-gui and realms commands (realms through the stub, §2.4),
and `host_failed` with detail `host_tools_unavailable` for the Host commands in a player-only build (§2.5.1). The
list of commands registered ahead of their rows (`PLANNED`) is empty; such a command would answer `unknown` for
badly shaped arguments, else `not_implemented` (served build) or `not_in_this_build`, with the command name as the
detail.

#### 2.3.3 Constants and name lists

Limits: invite 2048; username 32; key text 8192; game path 260; server id exactly 24 lower-case hex; detected game
index 0..15; server name 60; public host 253; "invite for" 40; copied text 4096; Epic code 1024; game version 64;
module id 64; module command 64; module JSON 8192; realm id 128; history who 64; history time 40; history cursor
128; change id 64; admin player 64; admin reason 500; remote label 128; remote URL 512; remote URI 2048; multiplier
1..100 (`MAX_MULTIPLIER`, launcher-kit).

Name lists (each equals its wire enum's names, in order; a test checks):

- settings keys: `graphics`, `windowed`, `streamerMode`, `hostTools`, `language`, `gameLanguage`, `runtimeProfile`,
  `gameEdition`, `minimizeToTray`, `closeToTray`, `showGameConsole`;
- install modes: `install`, `repair`;
- external targets (24): `tailscale_download`, `tailscale_share`, `vc_redist`, `server_source`, `project_source`,
  `project_license`, `project_issues`, `project_docs_players`, `project_docs_hosting`, `project_docs_sdk`,
  `installing_guide`, `legendary`, `dumper7`, `visual_studio`, `rust`, `nodejs`, `mongodb`, `git`,
  `visual_studio_installer`, `rust_installer`, `dumper7_zip`, `revived_source`, `mystic_source`, `undaunted_source`;
- host actions (11): `inject_dumper`, `import_sdk`, `build_runtime`, `build_loader`, `build_exporter`,
  `inject_exporter`, `copy_exports`, `synthetic_data`, `publish_runtime`, `generate_gamedata`, `export_gamedata`;
- host server components (11, for `host_server_restart`): `allowlist`, `backend`, `director`, `content`, `admin`,
  `gateway`, `metagame`, `legacy-allowlist`, `deploy`, `legacy-content`, `legacy-gateway`;
- host folders (7): `workspace`, `runtime_source`, `dumps`, `game_data`, `runtime_publish`, `host_logs`,
  `gamedata_packs`;
- server modes: `private`, `public`; realm kinds: `hub`, `hunt`; admin actions: `lookup`, `kick`, `timeout`,
  `ban`, `unban`; registration modes: `OPEN`, `INVITECODE`, `NONE`, `INVITE`; database engines: `sqlite`,
  `postgres`;
- remote exec actions (9): `doctor`, `backup`, `invite`, `ban`, `unban`, `kick`, `announce`, `guarded_approve`,
  `guarded_cancel`.

`fits(text, max)` is true when the UTF-8 length or the UTF-16 length is at most `max` (the backend counts as
JavaScript does).

#### 2.3.4 What each command checks before it reaches the controller

- **Every object argument is an object.** A settings patch, and every argument whose kind is *object*, is refused
  when the JSON is anything else, an array included (serde's derived `Deserialize` takes a sequence for a struct
  unless told not to; RL1 gap, found in the RL8 switch).

The isolation hook (§2.3.7) checks shapes for the page; every command checks again (the hook guards the page, the
commands guard the backend). Refusals are `ActionResult`-style `{ ok: false, error: { code, detail } }` unless said
otherwise. Codes are launcher-kit's `ErrorCode` names.

- `check_invite`, `submit_invite`: text too long → `invite_invalid_format` (`check_invite` answers an
  `InviteCheck` error).
- `switch_server`, `remove_saved_server`: not a server id → `invite_invalid_format`.
- `register`: too long → `RegisterOutcome` error `username_invalid`.
- `use_existing_key`: too long → `key_invalid`. `use_game_path`: too long → `folder_invalid`.
- `use_detected_game`, `revived_use_detected`: the index must be a JSON number, integral, 0..15 → else
  `folder_invalid`.
- Name arguments (`revived_install`, `open_external`, `host_action`, `host_open`, `realm_host`): only a JSON
  string equal to a listed name → else `unknown`. `host_server_restart`: a string in the component list → else
  `unknown`.
- `set_settings`: the patch is decoded strictly (unknown field or wrong value type → the empty patch, which
  changes nothing); answers the resulting `Settings`.
- `host_setup`: the form must be an object; `name` trimmed with JavaScript's `trim`, 1..60 units, no control
  character (U+0000–U+001F, U+007F); `mode` `private` or `public`. Private ignores `publicHost`. Public needs
  `publicHost` trimmed, ≤253 units, matching `^[A-Za-z0-9](?:[A-Za-z0-9.-]{0,251}[A-Za-z0-9])?$`. Any failure →
  `host_failed` with detail `setup_form`.
- `host_set_multipliers`: null → off. Otherwise an object with numbers `xp` and `rewards`, each finite and in
  1..100, rounded to two decimals; both equal to 1 → off. Failure → `host_failed` with detail `multipliers_form`.
- `host_create_invite`: absent, null or empty string → nobody; otherwise the plain-text rule above with max 40 →
  else `unknown`.
- `copy_text`: answers `true` only when the text fits 4096 and the clipboard took it (A code, `platform/clipboard.rs`).
- `epic_sign_in_code`: too long → `epic_code_invalid`; on success also closes the sign-in window.
- `game_install_*`: version non-empty and ≤64 → else `unknown` (`InstallOutcome` error for start and get).
- `realm_join`: trimmed, non-empty, ≤128, no control character → else `unknown`.
- History (`history_query`, `history_revert_dry_run`): the whole argument must fit its object shape (the hook's
  rule, §2.3.7); each text field trimmed, empty as absent; then the server GUI's workspace, or `host_failed` with
  detail `workspace`. `history_revert_apply`: a non-empty string ≤64.
- `guarded_approve`, `guarded_cancel`: exactly 16 lower-case hex digits → else `unknown`.
- `server_config_save`, `admin_action`, `remote_init_controller`, `remote_invite`, `remote_enroll`,
  `remote_exec`: the object must fit its shape, then each text field is plain text (trimmed, no control
  characters, within its limit), optional ones empty as absent; required ones non-empty. `remote_revoke`: the node
  is required plain text ≤64.
- Server-GUI commands with no module panel (any build but the manager) answer `not_in_this_build` with the command
  name.

Then each command calls the controller method of the same meaning (§2.4, §2.5), the Epic session, the ownership
cache, the module panel or the history engine (A code). `get_snapshot`, `check_invite`, `get_news`,
`get_branding`, `open_external`, `open_game_folder`, `open_logs_folder`, `host_open`, `host_server_get`,
`cancel_task`, `set_status_polling`, `copy_text`, `take_invite_link`, `game_log_get`, `dismiss_notice`, `dismiss_error`,
`subscribe_progress`, `epic_get`, `game_install_pause|resume|cancel|get` are synchronous; the rest asynchronous.
`refresh_status` runs one status poll; `retry_connect` runs `connect`; `install_update` installs a waiting update
(§2.7.12); `take_invite_link` hands over and forgets the waiting link; `game_log_get` answers the kept Game log
lines (§2.7.16), oldest first.

#### 2.3.5 Events and the progress channel

| Event | Payload | When |
| --- | --- | --- |
| `snapshot` | `Snapshot` (§2.3.8) | whenever the controller publishes |
| `invite-link` | none | a link arrived for the page (start-up arguments or a second launcher); the page then calls `take_invite_link` |
| `host-kit` | `HostKit` | the Host page's state changed |
| `host-server` | `HostServerView` | the server's state changed; it may carry the newest invite link, a secret: never logged |
| `window-state` | boolean (maximised) | §2.2 |
| `game-log` | array of text (new lines, redacted) | the game's console printed lines (§2.7.16) |

All events go to the window `main` only. **Progress**: `subscribe_progress` stores the page's channel (replacing
any earlier one). A progress value is sent only when the throttle passes it: the first value; a value at least
100 ms after the last one sent; a value whose task kind differs from the last one sent; or a final value
(`doneBytes ≥ totalBytes` and `filesDone ≥ filesTotal`). Nothing is sent before a subscription.

#### 2.3.6 The generated files

A test regenerates four files and fails when any differs from the file on disk (line endings normalised to LF);
with `RR_UPDATE_GENERATED=1` it writes them instead. The test name contains `generated`. The outputs must stay
**byte-identical**, so a clean-room generator is checked against the current files.

1. `ui/packages/bridge/src/generated/types.ts`: two header comment lines, `// Generated from the launcher backend
   (launcher/src-tauri/src/bridge); do not edit.` and `// Regenerate with: RR_UPDATE_GENERATED=1 cargo test -p
   launcher generated`, a blank line, then one `export <declaration>` line per wire type in a fixed order (the
   protocol's status types first, then the launcher's, as ts-rs declares them with large integers as `number`),
   then the hand-written unions (`GraphicsPreset`, `ActionResult`, `InviteCheck`, `RegisterOutcome`, `RegionPing`,
   `EpicLibraryCheck`, `GameVersionList`, `InstallOutcome`, `GuardedList`, `GuardedOutcome`, `ModuleListOutcome`,
   `ModuleOutcome`, `ModuleCommandOutcome`, `HistoryOutcome`, `RevertPlanOutcome`, `RevertApplyOutcome`,
   `ServerDoctorOutcome`, `ServerConfigOutcome`, `ServerInviteListOutcome`, `RemoteStatusOutcome`,
   `RemoteInviteOutcome`, `RemoteExecOutcome`), then `export type CommandName = "<name>" | ...;` in table order.
   The **current file is the interface oracle**: it is generated output describing names and shapes, which the
   implementer may read (§3.4).
2. `launcher/isolation/commands.js`: the same two header lines, then `window.__RR_COMMANDS__ = {`, one line per
   command `  "<name>": <args>,` and `};`, then `window.__RR_SETTINGS_KEYS__ = [<keys>];`. Each argument list is
   `{}` when empty, else `{ "<arg>": <shape>, ... }`; a shape is `{ kind: "text", max: N }`,
   `{ kind: "optionalText", max: N }`, `{ kind: "flag" }`, `{ kind: "index", max: N }`,
   `{ kind: "name", names: ["a", "b"] }`, `{ kind: "number", min: A, max: B }` (whole numbers without a decimal
   point), `{ kind: "object", nullable: <bool>, fields: { ... } }`, `{ kind: "settingsPatch" }` or
   `{ kind: "channel" }`. Strings are JSON-quoted. Field order is the table's, never a map's.
3. `launcher/src-tauri/permissions/app-commands.toml`: the header lines as `# ` comments, a blank line, then a
   `[[set]]` with identifier `app-commands`, description `Every command in src/bridge/spec.rs, and nothing else.`,
   and `permissions = [` with one `  "allow-<name with - for _>",` per command, then `]`.
4. `launcher/src-tauri/gen/strings.json`: every catalog in `ui/packages/i18n/catalogs/*.json` (`en` first, then the
   others by code), each as an object sorted by key, written as pretty JSON with a final newline. A missing
   catalog folder fails the test rather than skipping it.

Also tested: the generator never branches on Cargo features (generated output is the same in every build), and
every server-GUI and realm command appears in all three command outputs.

The header lines name `launcher/src-tauri/src/bridge` and `cargo test -p launcher`. If the generator moves to
launcher-kit, the headers keep these exact words (they are part of the bytes) until the maintainer accepts a
one-time regeneration.

#### 2.3.7 The isolation hook (`isolation/hook.js`, `isolation/index.html`)

- `index.html` is a minimal HTML5 document (language `en`, UTF-8, any title) that loads `commands.js` and then
  `hook.js`. Tauri runs it in its isolation frame.
- `hook.js` is plain ES5 in strict mode, self-contained, no dependencies. It defines
  `window.__TAURI_ISOLATION_HOOK__(payload)` that returns the payload unchanged when allowed and otherwise a
  promise that never settles (the page's call times out).
- A payload is allowed when it is an object whose `cmd` is a string naming either an entry of
  `window.__RR_COMMANDS__` or one of Tauri's own calls the page needs, and whose `payload` (absent or null counts
  as `{}`) has **exactly** the entry's fields, each fitting its shape.
- Tauri's calls allowed: `plugin:event|listen` (`event` text ≤64, `target` anything, `handler` a number),
  `plugin:event|unlisten` (`event` text ≤64, `eventId` a number), and `plugin:window|minimize`, `toggle_maximize`,
  `is_maximized`, `start_dragging`, `internal_toggle_maximize`, `close`, each with an optional `label` text ≤64.
- Shapes: `text` present string with `length ≤ max`; `optionalText` absent, undefined, null or such a string;
  `flag` present boolean; `index` present number, integral, 0..max; `name` present string in the list; `number`
  present finite number within the bounds given; `object` present and either null when nullable or exactly its
  fields; `settingsPatch` an object whose every key is a settings key and every value a number, a boolean or a
  string of at most 16 units; `channel` a string matching `^__CHANNEL__:[0-9]{1,16}$`; `any` anything; any other
  kind refuses.

#### 2.3.8 Wire types still in the shell (`bridge/types.rs`)

launcher-kit's `bridge::types` already holds `RuntimeState`, `RuntimeFileInfo`, `ErrorCode`, `NoticeCode`,
`LauncherError`, `ActionResult`, the Host types, `ExternalTarget`, `HostAction`, `HostFolder`, `RealmKind`, region,
version, module and admin types. The B part of `types.rs` is the rest; JSON names below are as serialised
(camelCase fields unless said). Field order is as listed (it is the serialised order).

- `Language` (generated, §2.1.4): codes as in the catalogs. `Game`: `reborn` | `legacy`, from a game version
  (`legacy` for 1.4.4). `GraphicsPreset`: an integer −1..4, −1 = leave to the game's menu (the default), 0..4 =
  Low..Cinematic; out of range refuses.
- `GameLanguage`: `auto`, `en-US`, `de-DE`, `es-ES`, `fr-FR`, `it-IT`, `ja-JP`, `pt-BR`, `ru-RU`.
  `RuntimeProfile`: `production`, `development`.
- `Phase`: `loading`, `join`, `connect`, `register`, `locate`, `runtime`, `installing`, `ready`, `running`.
  `ConnectProblem` (snake case): `tailscale_missing`, `unreachable`, `bad_answer`, `cert_mismatch`. `GameProblem`:
  `not_set`, `not_found`, `unverified`, `wrong_version`. `ServerGame`: `reforged`, `revived`.
- `InviteCheck`: `{ ok: true, certificateChanged, previousFingerprint, movedFrom }` or `{ ok: false, error }`.
  `RegisterOutcome`: `{ ok: true, username }` or the error. `EpicLibraryCheck`: `{ ok: true, owned, checkedAt }`.
- `TaskKind`: `verify`, `runtime`, `download`, `dlls`, `install`. `TaskProgress`: `kind`, `totalBytes`,
  `doneBytes`, `filesTotal`, `filesDone`, `bytesPerSecond`, `etaSeconds` (nullable), `currentFile` (nullable).
- `ServerInfo`: `game`, `mode`, `host`, `port`, `name`, `hasShare`, `hasPendingInvite`, `loopback`,
  `fingerprint`. `SavedServerInfo` (field names as is): `id`, `game`, `mode`, `host`, `port`, `name`, `current`,
  `username`, `unsupported`.
- `GameSource`: `epic`, `legendary`, `heroic`, `chosen`, `revived`. `DetectedGame`: `root`, `source`, `version`.
  `GameInfo`: `root`, `problem`, `versionText`, `versionOk`, `exeVerified`, `detected`.
- `RevivedInfo` (always null in these builds): `root`, `chosen`, `problem`, `missingFiles`, `totalBytes`, `dllsOk`,
  `detected`. `InstallMode`: `install`, `repair`.
- `RuntimeInfo`: `state`, `version`, `files`, `localDir`.
- `Settings`: `graphics`, `windowed`, `language`, `gameLanguage`, `runtimeProfile`, `streamerMode`, `hostTools`,
  `gameEdition`. `SettingsPatch`: all optional, the same names plus nothing else (unknown fields refuse).
- `SnapshotConnect`: `checking`, `problem`, `tailscaleInstalled`, `lastCheckedAt`. `SnapshotAccount`: `username`,
  `hasKey`, `offerBackup`. `SnapshotRunning`: `game`, `relayPort`. `SnapshotApp`: `version`, `packaged`,
  `updateReady`, `platform` (`win32`/`linux`), `hostToolsAvailable`, `legacyAvailable`, `gameVersions`,
  `serverGui`.
- `Snapshot`: `phase`, `busy`, `server`, `savedServers`, `connect`, `account`, `game`, `revived`, `runtime`, `task`,
  `running`, `settings`, `app`, `status` (protocol's `ServerStatus`), `statusUnsupported`, `lastError`, `notice`.
- `NewsItem`: `date`, `title`, `body`. `Background`: `url`, `credit`. `Branding`: `backgrounds`, `accent`.

The rest of `types.rs` is born here and moves as it is: `EpicView`, `InstallState`, `InstallInfo`,
`InstallOutcome`, the guarded, history, revert, realm, doctor, MongoDB-migration, server-config, server-invite,
managed-install and remote types and their outcomes.

### 2.4 The controller (`controller/{mod,platform,stored,game,runtime}.rs`)

#### 2.4.1 Shape

- One controller per app, shared by reference. All mutable state sits behind **one** mutex that is never held
  across an await: each locked stretch is one step, so concurrent commands interleave only between steps. The
  settings store and the executable-hash cache have their own async locks.
- Construction: migrate legacy folders (§2.7.3); load settings (§2.7.2) with the OS language and the platform; make
  the key store (§2.7.1) over `<userData>/keys`, the encryptors and the recovery folder
  `<documents>/DaintyLass/Account Keys`; load the hash cache `<userData>/file-hashes.json` (launcher-kit
  `HashCache`); make the game process (§2.7.5); make the Host page state (§2.5) and the realms and installs state
  (A); listen to the game process; attach the Host page's server listener.
- **Platform trait** (what the controller needs from the app; tests use fakes): choose a folder (language,
  purpose, start folder), choose a save file (default name; starts in Documents, filter `Text`/`txt`), choose a
  file to open (starts in Documents, `Text`/`txt`), choose the Dumper-7 DLL (`DLL`/`dll`), choose a game data pack
  (`Game data pack`/`gz`), open a checked URL, open a folder, emit the snapshot / progress / host kit / host server,
  register an art file, find Tailscale, find running game clients, prepare the game launch (Windows: native; Linux:
  §2.7.13 or "missing"), list the missing Visual C++ and DirectX files, start reading a game's console (pid, hide,
  a sink for batches of lines; §2.7.16; default: nothing) and emit new Game log lines (default: nothing).
- **Test seams**: a stand-in spawner for the game, the game's base environment, the store folders, the extra QoS
  port (default 80; tests turn it off), a probe for "a Host page server answers on 61000", and the Dumper-7 output
  root.

#### 2.4.2 State and the snapshot

The state holds the stored settings view; reachability (unknown/true/false); whether a connection check runs; the
connection problem; whether Tailscale is installed; the last check time (ISO 8601 UTC with milliseconds); failed
polls; the last `ServerStatus`; whether status is unsupported; whether a key is stored; whether the server refused
it; a **status generation** counter; busy; the game info; the server's runtime manifest; the runtime info and
whether it is playable; the running task; the verify abort handle and the runtime download cancel token; the last
error; the notice; update ready; the poll task; news, when news was last asked, whether news is loading; branding;
the relay; shutting down; and the Host page's flags (its server is up, the game runs for Dumper-7, the loader
session).

**Resetting the server** clears reachability, status, unsupported, the problem, news, branding, key refusal, failed
polls and the manifest, and bumps the status generation.

**Phase**, first match wins: no current server → `join`; a task runs → `installing`; the game runs → `running`;
not reachable → `connect`; no key or key refused → `register`; a game problem → `locate`; runtime not playable →
`runtime`; else `ready`.

**Snapshot**: phase; busy; the saved servers (each with `current`, its username, `unsupported` for a 1.4.4 server);
the current server (`loopback` when its host is a loopback host); connection; account (`hasKey` = stored and not
refused; `offerBackup` when it has a key and no backup was offered for this server); game; `revived` null;
runtime; task; running (game, relay port); settings (with `hostTools` false in a build without `host`;
`gameEdition` always `reborn`; then `minimizeToTray`, `closeToTray`, `showGameConsole` as
stored); app (version, packaged, update ready, platform, `hostToolsAvailable` = the `host`
feature, `serverGui` = the `server-gui` feature, `legacyAvailable` false, `gameVersions` = the protocol's played
versions as text); status; status unsupported; last error; notice.

**Publishing** emits the snapshot. **Failing** sets the last error and publishes. **Busy**: an action that marks
busy clears the last error and publishes, and publishes again when done.

#### 2.4.3 Start (`init`)

Refresh the key flag; recover a loader session a closed launcher left (A); look at the game (with store detection,
§2.4.8); look at the runtime; ensure the Host workspace quietly (§2.5); publish. If a server is current, connect in
the background. If the game is found but unverified, verify it in the background.

#### 2.4.4 Joining

- **`check_invite(text)`**: parse with `protocol::invite::parse_invite`. A refused invite answers
  `invite_invalid_format` with the parser's error name as detail (`revived` for a 1.4.4 invite). Otherwise answer
  `certificateChanged` / `previousFingerprint` (public invites: the key store has a key for this host:port saved
  with another fingerprint, or with none: then `previousFingerprint` is null) and `movedFrom` (private invites only:
  no key at the invite's address, but a stored server of the same game kind, private, same port, other host, and
  the same name after JavaScript `trim` has a key: its host).
- **`submit_invite(text, acceptNewCertificate, carryAccount)`**: refuse `busy` while busy or a task runs;
  `already_running` while the game runs; invalid invite as above. Then join with the invite's code.
- **`switch_server(id)`**: same refusals; the saved server with that id; a 1.4.4 one is refused as
  `invite_invalid_format`/`revived`; none → `invite_invalid_format`. Then join without a code change.
- **`remove_saved_server(id)`**: the current server cannot be removed (`busy`); otherwise drop the entry with that
  id from `savedServers` and publish.
- **Join**: if a public key exists for another certificate: without consent fail with `cert_changed` and the old
  fingerprint (or empty) as detail, changing nothing; with consent move the key to the new fingerprint (§2.7.1
  `rebind`) and log both fingerprints. With `carryAccount` and a moved private server: copy its key to the new
  address, copy its username to the new id when the new id has none, drop the old address from the saved list,
  and log it. Store the server (`code` only while this PC has no key for it). If mode, host, port or fingerprint
  differ from the current server, reset the server state. Refresh the key flag, clear the last error, connect.
- **`forget_server`**: refused `busy` while a task or the game runs; clear the current server, reset, no key, look at
  the runtime, publish.

#### 2.4.5 Connecting and polling

- **`connect`**: without a server answer `server_unreachable`; if a check already runs answer ok; else mark
  checking, publish, check, unmark, publish, answer ok.
- **Check**: Tailscale is looked for in private mode only. Fetch the status with the status key (below); record the
  check time. On a status or "unsupported": reachable, no problem, failed polls 0, unsupported flag; keep the
  status, but if the status generation moved while the request was out, keep only the protocol's limited view and
  do not take its name; when the name differs from the stored one, store it. Then confirm the account (below),
  refresh the runtime offer (§2.4.9), and load news and art in the background. Otherwise: unreachable with the
  problem `bad_answer`, `cert_mismatch`, `tailscale_missing` (private, no Tailscale, not a loopback host) or
  `unreachable`; log an error for a certificate mismatch, a warning naming the problem otherwise.
- **Status key**: the stored key of the current server, read from the key store per request, only while it has a
  key and the server has not refused it.
- **Confirming the account**: load the key; none → no key. Ask user info (§2.6.3): ok → not refused, and store the
  username if it changed; `account_banned` → refused, last error `account_banned` with the reason; `key_rejected`
  → refused, last error `key_rejected`, logged; other errors change nothing.
- **Polling** (`set_status_polling(on)`): on starts a task that polls immediately and then every 15 s (missed
  ticks delay); off aborts it.
- **One poll**: nothing without a server or while a check runs. Not reachable: retry the whole connection quietly,
  except after a certificate mismatch. Else fetch the status; an answer overtaken by a generation change is dropped.
  Status → keep it, reset failed polls, then confirm the account (not while the game runs) and load news (and art
  if none yet) in the background; also refresh the runtime offer when the game does not run, no task runs, the
  server is not 1.4.4, and the runtime is unavailable or no manifest is known. Unsupported → flag it. Certificate
  mismatch → unreachable with that problem, status marked offline, error logged. Anything else → one more failed
  poll, status offline, and after two failures (not while the game runs) unreachable/`unreachable`. Publish.
- **News**: at most once a minute, not while loading; fetched from the server (§2.6.3) with the current list as
  fallback; dropped if the generation moved.
- **Art**: fetch the branding index; ensure `<userData>/art-cache`; for each background download the image,
  name it by its SHA-256 (lower-case hex), write it into the cache, register it in the art store, and record
  `rr-art://bg/<id>` with its credit; keep the branding only if the generation did not move; publish. A write
  failure logs a warning and stops.
- Background work is tracked; shutdown waits for it.

#### 2.4.6 The account

- **`register(username)`**: `busy` refusals; the username is checked by `protocol::account::check_username` (failure
  → `username_invalid` with the check's name as detail); no server → `server_unreachable`. Busy while: register
  (§2.6.3) with the stored invite code (or empty). A refusal sets the last error to its code and answers it. Store
  the key (failure → `key_storage_unavailable`). Clear the stored code, store the username, unmark the backup offer;
  key present, not refused, bump the generation; log; confirm the account; poll at once. Answer `{ ok: true,
  username }`.
- **`use_existing_key(text)`**: `busy`; the key is `protocol::account::extract_account_key(text)` (none →
  `key_invalid`); then adopt it.
- **`import_key_file`**: `busy`; choose a file (cancel → `cancelled`); read it only when it is at most 8192 bytes;
  extract the key as above (none → `key_invalid`); adopt it while busy.
- **Adopt a key**: no server → `server_unreachable`; ask user info with the key (an error becomes the last error and
  the answer); save it (an invalid key → `key_invalid`, other failures `key_storage_unavailable`); store the
  username, mark the backup as offered, clear the code; key present, not refused, bump the generation; log; poll.
- **`save_key_backup`**: no server → `server_unreachable`; no key → `key_invalid`. Default file name
  `DaintyLass key - <username>.txt`, or `DaintyLass key.txt` without a username. Choose a save file (cancel →
  `cancelled`). Write the backup text (§2.7.1) privately (owner-only on Unix); failure → `unknown`. Mark offered,
  notice `backup_saved`.
- **`dismiss_backup_offer`**: mark offered, publish.
- **`logout`**: refused `busy` while a task or the game runs; without a server ok. Remove the key (all copies),
  clear the username and the offer, no key, not refused, bump the generation, replace the status by its limited
  view, log, publish.

#### 2.4.7 PLAY

- **`play`**: refused `busy` while shutting down or busy; `already_running` while the game runs; no server →
  `server_unreachable`. Busy while it runs (below).
- **Steps**, any failure ends with the error published:
  1. Log missing Visual C++ files as a warning only.
  2. A running game client (§2.7.5) → `already_running`.
  3. Load the key (none → `key_invalid`); ask user info (an error is the answer; `key_rejected` and
     `account_banned` also mark the key refused); then not refused.
  4. The game folder and its Win64 folder must be known (else `game_folder_not_found`); hash the executable through
     the hash cache (no progress) and save the cache; no hash → look at the game again, `game_folder_not_found`;
     another hash → `game_wrong_version`.
  5. Loader leftovers (RD9, A code) → `loader_leftovers` with the file names joined by `, `, logged.
  6. With a local runtime folder set, copy it in (launcher-kit `install_local_runtime`): in use →
     `runtime_in_use`, other failures `local_runtime_invalid`.
  7. Look at the runtime; not playable → `runtime_untrusted`, `runtime_unavailable` or `runtime_failed` by state.
  8. For the Host page's export session only: begin a loader session with CatalogExporter (A code).
  9. Start the relay (§2.6.4) on the configured port with the QoS port, then launch; a launch failure stops the
     relay and ends the loader session.
- **Relay start**: stop any running relay; a public server without a fingerprint → `relay_failed`. Target: the
  server's host and port, with the fingerprint in public mode. The owner check is the game process's (§2.6.5). A
  pin failure sets the last error `cert_mismatch` and publishes. Relay log lines go to the log prefixed `relay: `.
  A busy port → `relay_port_busy`, other failures `relay_failed`, with the port as detail.
- **Launch**: prepare the platform's launch (Linux without Proton or Wine → `compat_runtime_missing`); write the
  game config (§2.7.6) for host `127.0.0.1`, the relay port, the graphics setting, the platform's config folder,
  the windowed setting as "safe window", and the Phoenix clean-up on; failure → `config_failed`. Log how many
  Phoenix endpoints were removed, if any. Build the launch arguments (§2.7.5) from the key, the user id, the
  username, windowed (setting or a display repair), and the game locale (§2.8.1); build the environment with the
  backend URL `http://127.0.0.1:<port>` and the runtime profile; failure → `launch_failed`. Log the masked command
  line with the target and the mode. Start the game in the Win64 folder (failure → `launch_failed`).
- **The game exits**: log the exit code. A non-zero code that is not 0xC000013A (Ctrl+C/console closed) and not a
  Dumper-7 run sets the last error `launch_failed` with the code as `0x` + eight upper-case hex digits. Then, in the
  background: end the loader session, stop the relay (logging its request, upgrade, failure and QoS counts), look at
  the runtime, publish. The game starting publishes.
- **`keep_open`**: the relay reason while the relay runs.
- **`cancel_task`**: aborts the verify and cancels the runtime download.

#### 2.4.8 The game folder (`game.rs`)

- **Look at the game** (optionally detecting store installs, §2.7.7, in a blocking task): with no folder chosen and
  installs found, take the first whose `Version.txt` is supported, else the only one when exactly one; store it as
  `gameRoot` and log the source. Then: no folder → `not_set`; no executable → `not_found`; otherwise read
  `Version.txt` and the cached hash: no cached hash and a version text that is not supported → `wrong_version`; no
  cached hash → `unverified`; a cached hash that is not the pin → `wrong_version`; the pin → verified, no problem.
- **`verify_game`**: refused `busy` while busy or the game runs; no folder → `game_folder_not_found`. Missing
  executable → look again, `game_folder_not_found`. Run a `verify` task (total = file size, one file, current file
  the executable name) and hash it with progress at most every 150 ms; save the cache. Ok: log the time and whether it
  is 1.14.7; look at the game and the runtime; a wrong version → `game_wrong_version`; else notice `game_verified`.
  Aborted → `cancelled`. I/O error → logged, `game_folder_not_found`. Publish.
- **`choose_game_folder`**: refused `busy` while busy or running; a folder dialog (purpose: game, starting at the
  current folder) (cancel → `cancelled`); then as a pasted path.
- **`use_game_path(path)`**: same refusal; locate (§2.7.7) in a blocking task, then check the refusal again; found →
  store, look at the game and runtime, publish, and verify at once when unverified; not found →
  `game_folder_not_found`; invalid or too long → `folder_invalid`.
- **`use_detected_game(index)`**: that detected root as a pasted path; none → `folder_invalid`.
- **`open_game_folder`**: opens the game folder when it exists.

#### 2.4.9 The server's runtime (`runtime.rs`)

- **Trusted** when the current server's `trustedRuntimes` entry contains the manifest's set id (launcher-kit
  `is_trusted`).
- **Refresh the offer**: fetch the manifest (§2.6.3): ok → keep it; none → forget it; certificate mismatch →
  unreachable with that problem; bad answer or unreachable → keep what was known. Then look at the runtime.
- **Look at the runtime**: version and file list from the manifest. Without a Win64 folder: `missing` when a
  manifest is known, else `unavailable`, never playable. Otherwise launcher-kit `check_runtime(win64, manifest,
  trusted, local folder)` gives state and playability.
- **`trust_runtime`**: without a server or a manifest → `runtime_unavailable`. Add the set to the server's trusted
  list (launcher-kit `accept`, which keeps the last eight), store it, log the version and the first 12 characters of
  the set id, then **install** (always).
- **`install_runtime`**: `busy` while busy; `already_running` while the game runs; no server →
  `server_unreachable`; no Win64 folder → `game_folder_not_found`; no manifest → `runtime_unavailable`; not trusted
  → `runtime_untrusted`; no key → `key_invalid`. Start a `runtime` task (total = the files' sizes, file count) with a
  cancel token (checking busy again under the lock). Install with the key headers (§2.6.3) over the server's
  endpoint, progress as a task with a rate from the start time and an ETA. Ok: log; notice
  `runtime_installed_restart` when a file was replaced while in use and the Host page's server is up, else
  `runtime_installed`. Errors: cancelled → `cancelled` (not published); key rejected → mark refused,
  `key_rejected`; certificate mismatch → unreachable with that problem, `cert_mismatch` without a file; not found →
  `runtime_unavailable`; different → `runtime_different_on_server`; download failed → `download_failed`; disk →
  `folder_invalid`; in use → `runtime_in_use`; other → `runtime_failed`; the failing file is the detail. Always:
  clear the task and token, look at the runtime, publish.
- **`choose_local_runtime`**: `busy` while busy or running; folder dialog (purpose: runtime) from the current
  local folder (cancel → `cancelled`); the folder must hold both runtime files (else `local_runtime_invalid` with
  the file names joined by `, `); store `localRuntimeDir`, look, publish.
- **`clear_local_runtime`**: clear it, look, publish.

#### 2.4.10 Settings, links and folders

- **`set_settings(patch)`**: a player-only build ignores `hostTools`. The edition is always `reborn`, so a
  `gameEdition` change does nothing. Each present field is written to the settings map (graphics as its number,
  language as its code, the tray and console flags as booleans, the rest as their names). Turning Host tools on
  ensures the Host workspace quietly. Publish; answer the settings.
- **`tray()`**: (`minimizeToTray`, `closeToTray`) for the window (§2.2).
- **Game log**: when the game starts, clear the kept lines and start reading its console (§2.7.16) with hide =
  not `showGameConsole`. Each batch is redacted line by line, appended, cut to the newest 5,000 lines, and the
  redacted batch is emitted as `game-log`. `game_log()` answers the kept lines.
- **`open_external(target)`**: `tailscale_share` → the current private server's share link; `server_source` → the
  status's source URL, else the project URL; every other target → its fixed URL (§2.7.9). The URL must pass the
  link check with the status's source URL; else `unknown`.
- **`open_logs_folder`**: opens `<userData>/logs` when it exists.
- **`set_notice`**, **`dismiss_error`**: set or clear and publish. **`set_update_ready`** marks the snapshot.
  **`may_install_update`**: an update waits, no task, no game, not shutting down.

#### 2.4.11 Stored settings view (`stored.rs`)

A typed view of the settings map (§2.7.2): the current server and the saved servers (each: `game` default
`reforged`, `mode`, `host`, `port`, `name`, `share`, `fp`, `code`), `gameRoot`, `graphics`, `windowed`,
`streamerMode`, `language`, `gameLanguage`, `runtimeProfile`, `hostTools`, `minimizeToTray`, `closeToTray`,
`showGameConsole` (each false when absent), `usernames`, `backupOffered`,
`trustedRuntimes`, `localRuntimeDir`, `hostWorkspace`, `dumperDll`. A server's id is the key store's id (§2.7.1);
its endpoint is host, port and, in public mode only, the fingerprint as the pin; its key slot is host, port, public
and fingerprint. Edits are made on the map (set the server, edit one field of the current server, set or remove a
username, set or remove a backup offer, set a text field or null), then the store sanitises and writes.

#### 2.4.12 Shutdown

Mark shutting down (PLAY refuses from now on); stop the Host page's server if this launcher started it (A); stop
realms (A); wait for every background task; stop polling; cancel the task; stop the relay; save the hash cache
(failure logged).

### 2.5 The Host page controller (`controller/host.rs` lines 1–1171; `host_off.rs`)

#### 2.5.1 Builds

Without `host`, every Host command answers `host_failed` with detail `host_tools_unavailable` (published),
`host_kit` answers an empty kit (no workspace, everything absent or false, game data missing) and the server view is
stopped; ensuring the workspace answers ok. With `host` the rest of this section applies. With an embedded server
(the manager), "the server" is the in-process one (A code, §4.3).

#### 2.5.2 State

The server (launcher-kit `HostServer`, with the probe seam and its stop timeout), the current job (kind, state
`running`/`done`/`failed`, message, log file), the last update view, the requirements cache (time, workspace, list;
15 s), and the Dumper-7 output root (`C:\Dumper-7` by default). The server's listener marks "server up" in the
controller for `starting`, `running`, `stopping` and `external` and emits `host-server`.

#### 2.5.3 Workspace

- **Valid**: with an embedded server, any existing folder; otherwise a host-kit workspace (launcher-kit
  `is_workspace`). **The workspace** is the stored `hostWorkspace` while it is valid.
- **Project target**: the folder `Reforged` beside the game folder (none while the game is not set or not found).
- **Ensure (quiet or not)**: embedded server: not quiet → `not_in_this_build`; quiet with no workspace → create
  `<userData>/Server`, store it, log, emit the kit, publish (a creation failure only logs). Otherwise: quiet with Host
  tools off → ok; a workspace → ok; a job running → `busy`; no project target → `host_failed`/`project_no_game`
  (quiet: ok); the target invalid → `project_folder_taken` when it exists, else `project_missing` (quiet: ok);
  else store it, log, emit, publish.
- Failures are `host_failed` with a detail; the workspace detail is `workspace_mgr` with an embedded server, else
  `workspace`. A host-kit tool failure logs and uses the tool error's detail.
- Jobs: starting sets `running` and emits the kit; finishing sets `done`/`failed` and emits; success sets notice
  `sdk_imported` (SDK import) or `host_done` and publishes; failure answers `host_failed` with the message, or the
  job kind's name.
- **Running the host kit**: with an embedded server, in process (A code); otherwise launcher-kit's `HostTool`
  found in the workspace (missing → the tool error) run through the server's command runner with a log name.
- **While stopped**: a running server this launcher started is stopped for a change and started again after it (a
  failed restart only logs).

#### 2.5.4 Commands

- **`host_kit`**: find the toolchain; detect the kit for the stored workspace and the dump root; `workspaceOk` when
  valid; requirements from the cache when fresh for this workspace, else checked (game ok and its version text, the
  toolchain, the Dumper-7 DLL, Tailscale, the count of missing Visual C++ files) and cached; `exports.inGame` when
  the game's `Win64/Items_Analysis` exists; the stored Dumper-7 DLL, the job, the update view and the project target.
- **`host_choose_workspace`**: folder dialog (purpose: workspace) from the stored workspace (embedded: the default
  data folder); cancel → `cancelled`; embedded: create the picked folder if missing; invalid → workspace failure;
  store, clear the requirements cache, emit, publish.
- **`host_choose_dumper`**: DLL dialog; cancel → `cancelled`; not a `.dll` file → `dumper`; store `dumperDll`, clear
  the cache, emit.
- **`host_import_gamedata`**: `busy` while a job runs; workspace; pack dialog (cancel → `cancelled`); job
  `import_gamedata` with the file name and the launcher log `gamedata-import`; run `gamedata --import <file>`; done
  with the file name, or failed with the tool's failure line.
- **`host_import_sdk_folder`**: `busy`; workspace; folder dialog (purpose: SDK) from the dump root; the SDK folder is
  the picked one if it holds `SDK.hpp`, else its `CppSDK` if that does, else `sdk_folder`; ensure the runtime source
  (below); import (below) named after the folder's parent.
- **`host_download_project`**: ensure the workspace, not quiet.
- **`host_action(action)`**: `busy` while a job runs. `inject_dumper` and `inject_exporter` are sessions (below).
  Others need the workspace; `import_sdk` and `build_loader` (and, without an embedded server, also
  `synthetic_data` and `generate_gamedata`) ensure the runtime source first.
  - `import_sdk`: the newest dump under the dump root (none → `no_dump`), imported.
  - `build_runtime` / `build_exporter`: MSBuild from the toolchain (none → `msbuild`); job with log
    `<logs>/build_runtime.log` or `build_exporter.log`; MSBuild on the runtime solution or exporter project with
    launcher-kit's arguments and platform toolset for the Visual Studio version, in the project's folder; done or
    failed with the last output line.
  - `build_loader`: Cargo (none → `cargo`); job with `<logs>/build_loader.log`; `cargo build --release` in the
    loader folder.
  - `copy_exports`: the game's `Items_Analysis` (none → `no_exports`) copied into the workspace's exports; done
    with `<n> files`.
  - `synthetic_data`: job; `gamedata --examples`.
  - `publish_runtime`: job with log `publish-runtime`; `publish-runtime`; done with the published version (from the
    output), logged.
  - `generate_gamedata`: job with log `gamedata`; copy the game's export into the workspace when present (a note
    on the server panel says how many files), else the workspace's exports must exist (`no_exports`); then
    `gamedata`; done with the message `restart` when the server runs.
  - `export_gamedata`: job; `gamedata --export`; done with the written pack's file name.
- **Ensure the runtime source**: present when `dllmain.cpp` is in the runtime source folder; otherwise a job
  `upstream` runs `upstream`; failure fails it; still absent → `upstream_old`; logged.
- **Import an SDK**: job `import_sdk` with the name; launcher-kit `import_sdk(dir, runtime source)` in a blocking
  task; done with `<n> files` (logged), or failed with the import error's code.
- **Sessions** share checks: Windows only (`windows_only`); `busy` while busy or shutting down; `game_running` while
  the game runs; no folder or not found → `game_folder_not_found`; wrong version → `game_wrong_version`.
  - Dumper-7: the stored DLL must be a file (`dumper`); a running client → `already_running`; begin a dump loader
    session (A code); job `inject` "Dumper-7"; mark dumping and keep the session; start the game in Win64 with the
    arguments `-NoEAC -EpicPortal -DisableEOS` and the launcher's environment without any runtime variable
    (§2.7.5); a start failure unmarks, ends the session and fails the job; else done "Dumper-7". The game's exit ends
    the session (§2.4.7).
  - CatalogExporter: needs the workspace and the built exporter DLL (`exporter`); write `export_flags.txt` into Win64
    (launcher-kit `EXPORT_FLAGS`; failure → `export_flags:<error>`); job `inject` "CatalogExporter"; PLAY with the
    exporter; ok → done (logged), else the job is cleared and PLAY's answer returned.
- **`host_open(folder)`**: workspace, runtime source, dumps (the dump root), game data (the parent of the backend
  game-data folder), runtime publish, host logs, game-data packs; a folder that does not exist → `folder`.
- **`host_server_view`**: the current view, refreshing whether an outside server answers in the background.
- **`host_server_start`**: workspace; `busy`; the tool; the detected kit must be configured (`not_configured`) and
  have backend and director game data (`gamedata_missing`); start; logged.
- **`host_server_restart(name)`**: restart that component; its error text is the detail.
- **`host_server_stop`**: stop; logged; ok.
- **`host_setup(request)`**: workspace; `busy`; refused `server_running` while the server runs or runs outside;
  arguments `setup --mode <private|public> --name <name>`, `--host <publicHost>` in public mode, `--game <game
  folder>` when known; job `setup` with log `setup`; clear the requirements cache; done with the server name.
- **`host_create_invite(for)`**: workspace; the server must be running or external (`server_not_running`); run
  `invite --json [--for <name>]`; the invite line from the output (launcher-kit parser) is stored in the server view
  (link and name), never logged; failure → its failure line or `invite`. Logged as "invite created" with or without
  a note.
- **`host_update`**: workspace; `busy`; refused `server_external`; job `update`; run `update --check --json`; read
  the last JSON line as an update check (failure → the reason from launcher-kit's update failure reader); nothing
  behind → update view `up_to_date` with the head, done. Otherwise, with the server stopped: run
  `update --json --no-setup`; read the result; logged; update view `updated` with the new head, file count and
  whether the runtime changed; if the workspace was configured, run `setup` (failure fails the job); done.
- **`host_set_multipliers(m)`**: workspace; `busy`; refused `server_external`; must be configured
  (`not_configured`) and private (`multipliers_public`); arguments `multipliers --xp <x> --rewards <r>` or
  `multipliers --off`; job `multipliers`, run with the server stopped; logged.

### 2.6 The network layer

#### 2.6.1 Requests to the invite's host (`http.rs`, `pinned.rs`, `http/to_file.rs`)

launcher-kit already has clean-room `net::{pinned, http, http::to_file}`. The shell must switch to them after
closing these differences (the restricted behaviour is the reference):

- **Endpoint check**: a valid host (protocol `is_valid_host`) and non-zero port; a pin, when present, must be a valid
  fingerprint; without a pin the host must be a private-mode host (Tailscale or loopback). The shell's check
  answers only ok/error; the kit's returns an extra value, which callers ignore.
- **Pinned target from an endpoint** (`pinned_target`): host, port and pin as fingerprint, only when a pin exists.
- **Requests**: the path must start with `/`; headers `accept: application/json`, `host` (`<host>` when pinned on
  port 443, else `<host>:<port>`), the caller's headers, and for a body `content-type: application/json` and its
  length; one connection per request (pinned TLS or plain TCP with no delay); HTTP/1.1; redirects never followed;
  a body over the limit fails `TooLarge`; a deadline for the whole exchange (`Timeout`); a cancel token (`Aborted`,
  also when already cancelled). A pin failure is `Pin { actual }`. The options' debug text never shows header
  values.
- **JSON body**: none when the body is empty and the content type does not mention `json`; else lossy UTF-8 parsed
  as JSON, or none.
- **Pinned TLS**: TLS 1.2 and 1.3 with the `aws-lc-rs` provider; ALPN `http/1.1`; SNI only for names (never for an
  IP address); the server certificate passes only when the SHA-256 of its DER is the fingerprint (lower-case hex),
  and the handshake signature is still verified with that certificate's key; the fingerprint seen is remembered so
  a mismatch reports it; a deadline over connect and handshake.
- **To a file**: GET with `accept: application/octet-stream`, the same endpoint rules; an idle timeout reset on the
  answer and each chunk (no data for that long → `Timeout`); for a non-200 answer write nothing and answer the
  status; a declared length or running total over the limit → `TooLarge`; each chunk's length reported; a failure
  after the file was created removes it; disk errors are their own kind.

#### 2.6.2 Pooled connections (`connector.rs`)

A connector for hyper-util's pooled client that ignores the URI and always connects to its one host: pinned TLS
(§2.6.1) or plain TCP (connect within the timeout, no delay). It counts connections opened. A helper walks an
error's source chain for a pin error and answers the fingerprint seen.

#### 2.6.3 The host API (`hostapi.rs`, `hostapi/parse.rs`)

- **Key headers**: the key goes in `x-reforged-user-api-key`, only when it is a plausible account key and the
  endpoint is pinned or a private-mode host; otherwise no header.
- **Status**: GET the protocol's status path with the key headers, timeout 5 s (or the caller's), 1 MiB. 200 →
  `protocol::status::parse_server_status` of the JSON body (fails → bad answer). 404 → GET `/dauntless-status`
  (64 KiB, no key); 200 with a JSON object holding `show-status` → unsupported; else bad answer. Pin failure →
  certificate mismatch; other errors → unreachable.
- **Register**: POST the protocol's register path with `protocol::account::RegisterRequest` as JSON, 20 s, 64 KiB.
  200 with a plausible key in `UUK` → ok, else `server_error`. 400 → `username_invalid` or `registration_closed`
  by the body's `error`, else `bad_request`, with the body's `message` cleaned to 200 units. 401 →
  `invite_invalid`; 409 → `username_taken`; 413 and 429 → `rate_limited`; else `server_error`. Pin failure →
  `cert_mismatch`, other errors `server_unreachable`.
- **User info**: an implausible key → `key_rejected` without a request. GET `/reforged/api/GetUserInfo` with the key
  headers, 10 s, 64 KiB. 403 with `error` `account_banned` or `account_disabled` → `account_banned` with `reason`
  cleaned to 500 units. 401/403 → `key_rejected`; 429 → `rate_limited`; non-200 → `server_error`. 200 needs
  `UserId` (cleaned to 128, matching `^[A-Za-z0-9_-]{1,128}$`) and `Username` (cleaned to 32); `IsAdmin` is true only
  for JSON `true`. Pin failure → `cert_mismatch`, others → `server_unreachable`.
- **Runtime manifest**: GET the protocol's manifest path, 10 s, 64 KiB, no key. 404/503 → none; 200 → parse; other →
  bad answer; pin → mismatch; others unreachable. **Parsing**: an object with `files` (array) and `version` (cleaned
  to 48); each file an object whose `name` is one of the protocol's runtime file slots (either name of the runtime
  DLL counts as its slot), each slot once, `sha256` exactly 64 lower-case hex, `size` a safe integer (read through a
  double: `1.0` counts, beyond ±(2^53−1) does not) from 1 to 64 MiB; exactly two files; sorted by name; the
  changelist is `build.changelist` when a safe integer, else none; anything else refuses the whole manifest.
- **News**: GET `/content/v1/news`, 10 s, 512 KiB; 200 → parse, else the fallback. Up to 30 items, each needing a
  title (cleaned to 120) and a body: CR LF and CR become LF, unsafe characters other than LF removed, at most two
  LFs in a row, trimmed, non-empty, cut to 4000 UTF-16 units plus `…`; `date` an ISO date or none.
- **Branding**: GET `/content/v1/branding`, 10 s, 64 KiB; non-200 → empty. Up to 8 backgrounds whose `url` is a
  branding path, each with `credit` cleaned to 120; `accent` kept (lower-cased) when exactly `#` and six hex
  digits. A branding path: starts with `/content/v1/branding/` (any case), no `..`, the name's first character
  `[A-Za-z0-9_-]`, then `[A-Za-z0-9_.-]`, at most 101 characters before the extension, extension `jpg`, `jpeg`,
  `png` or `webp` (any case).
- **Branding image**: only a branding path; GET, 30 s, 15 MiB; 200 only; the type is sniffed (JPEG `FF D8 FF`, PNG
  signature, `RIFF....WEBP`), never taken from the server.

#### 2.6.4 The local relay (`relay.rs`, `relay/local.rs`)

- **Purpose**: while the game runs, it listens on `127.0.0.1:<port>` (61100 by default) and forwards the game's
  HTTP requests and WebSocket upgrades to the server: public mode over pinned TLS to the gateway, private mode over
  plain HTTP (only to a private-mode host; anything else refuses to start). Paths pass as sent, `/__origin/...`
  included.
- **Start**: bind the port (in use or permission denied → port busy; other errors → listen failed); log
  `relay listening on 127.0.0.1:<port> for <host> (<mode>)`. An optional second listener on the QoS port (default
  80) answers only the QoS page; failing to bind it is logged at info and ignored.
- **Port override** (`REFORGED_RELAY_PORT`): trimmed, 4 or 5 digits, no leading zero, 1024..65535; otherwise
  ignored.
- **Per connection**: no-delay; the owner check (§2.6.5), when set, decides once per connection; a refused one
  logs a warning and answers every request 403 `{"error":"local_process_denied"}` with `connection: close` (an
  upgrade gets a bare 403). HTTP/1.1, keep-alive, 64 KiB buffer.
- **Local request rule** (also on the QoS listener), first match: peer not loopback (IPv4-mapped counts) →
  `not_local`; the Host header absent, not text, or whose host part (port and IPv6 brackets stripped) is not
  `localhost`, `::1` or a dotted `127.x.y.z` → `host`; a non-upgrade request with any `sec-fetch-*` header →
  `browser`; an Origin that is unreadable, or whose host (scheme optional, port optional, trailing `/` allowed) is
  not loopback → `origin`. A refused request answers 403 `{"error":"forbidden"}` (upgrades: bare 403) and logs.
- **QoS**: GET or HEAD whose target matches `^/qos(?:[/?]|$)` case-insensitively answers 200 `text/html`, the
  content length, `cache-control: no-store`, and the body
  `<!DOCTYPE html><html><body>pong</body></html>` (no body for HEAD); counted. The QoS listener answers the same for
  `/` and QoS paths, and 404 `{"error":"not_found"}` otherwise.
- **Forwarding a request**: a `transfer-encoding` other than `chunked` → 501
  `{"error":"unsupported_transfer_encoding"}`. Forward with the method, the target, HTTP/1.1, the end-to-end headers
  (launcher-kit/`proxy` rules, also dropping `host` and `http2-settings`), `host` set to the target (`<host>` for
  pinned port 443, else `<host>:<port>`), and `transfer-encoding: chunked` when the request had it. Pooled client:
  idle 60 s, up to 8 idle connections. The answer's status, reason phrase and end-to-end headers pass back. A pin
  failure → the pin callback with the seen fingerprint, an error log, and 502 `{"error":"certificate_mismatch"}`;
  another failure → warning and 502 `{"error":"upstream_unreachable"}`. JSON errors carry `content-type:
  application/json` and `cache-control: no-store`.
- **Upgrades** (an `upgrade` header and `connection` containing `upgrade`): a fresh connection (never pooled);
  forward with the end-to-end headers, `host`, the `upgrade` value and `connection: Upgrade`. On 101: answer 101 with
  the server's `connection`, `upgrade` and end-to-end headers, then tunnel both ways until 24 h of silence or the
  relay stops. Otherwise pass the status and end-to-end headers (without `content-length`) with up to 64 KiB of the
  body and `connection: close`. Failures → 502, bare.
- **Re-pointing** (born here, DN): the relay can be re-pointed to another node with a session ticket, or back home.
  Away from home (or with a ticket) `x-reforged-user-api-key` is removed and `x-reforged-session-ticket` set. A
  response carrying `x-reforged-redirect-host` and a numeric `x-reforged-redirect-port` (optionally
  `x-reforged-redirect-fingerprint` and a ticket) re-points it; a ticket is required for any non-home target; bad
  headers are logged and ignored.
- **Counters**: requests, upgrades, failures, QoS answers, upstream connections opened. **Stop** cancels every
  listener, connection and tunnel and frees both ports.

#### 2.6.5 The relay's owner check (`relay_owner.rs`, `relay_owner/table.rs`)

- Windows only; elsewhere there is no owner check (any loopback client passes the local rule), by decision RD21.
- A connection passes only while the launcher's game runs (its pid > 0), both ends are loopback IPv4 (mapped
  counts) with a port, fewer than 64 checks wait, and the TCP table names the game's pid as the owner of the
  **client** row.
- The table is `System32\netstat.exe -ano -p tcp` run hidden, within 2 s and 4 MiB. Rows are split on white space;
  the client row is the one with five fields, `TCP`, the client's `ip:port`, the server's `ip:port`; the state label
  (localised) is ignored; the last field must be all digits. No such row → "not yet"; several rows or a non-numeric
  pid → refused.
- Checks waiting together share one fresh table read (nothing cached); rows not found get one more read 25 ms
  later; anything else refuses.

#### 2.6.6 The Epic install job (`epic_install.rs`)

A progress and control wrapper around MIT `epic::download::install` (DE5):

- **Control**: pause, resume and cancel answer whether they applied (each applies once; cancel ends resume).
- **Progress**: the last `epic::download::Progress`, reported as a `TaskProgress` of kind `install`: total and done
  bytes from the download totals, files total and done (saturating to `u32`), the rate (downloading only), and the
  ETA (bytes left ÷ rate when the rate is positive); no current file.
- **Rate**: over a 5-second window of byte counts grouped in 250 ms buckets; the span is at least 1 s; reset when
  paused.
- **Run**: report progress on every update from the installer and on a 250 ms ticker until it ends, then once more;
  answer the installer's result.

### 2.7 The platform layer

#### 2.7.1 The key store (`keystore/{mod,format,os,legacy}.rs`)

- **Slot id**: lower-case hex of the first 12 bytes of SHA-256 of `public:<host>:<port>` (public) or
  `<host>:<port>` (private). For example `100.64.0.1:61000` private → `2f1974fb8b73463e94a88aed`;
  `203.0.113.10:443` public → `4513d283ab25060967c23596`.
- **Files**: `<keys>/<id>.key2` (this launcher's, encrypted), `<keys>/<id>.key` (the Electron launcher's, read
  only), and the plain recovery copy `<recovery>/<id>.txt`. Writes go to `<file>.tmp` then rename; the recovery
  copy is owner-only on Unix.
- **Text inside**: `RRK2\n<fingerprint or ->\n<key>` (no trailing newline), or, from the first format, the bare
  key. A fingerprint must be valid; the key must be a plausible account key (protocol); `DRK2` and anything else
  is no key.
- **Read**: the `.key2` file when it exists (encryptor unavailable → error "unavailable"; undecryptable → no key);
  else the `.key` file through the Electron reader (when available), which is then migrated: written as `.key2`,
  read back and compared; on mismatch the new file is removed and a warning logged; the old file stays. Else the
  recovery copy.
- **Save**: the key must be plausible; a public slot needs a valid fingerprint; the encryptor must be available.
  Register the key as a log secret; write the encrypted file; write the recovery copy (failure only warns).
- **Load**: read; a public slot answers only for its exact fingerprint. Register the secret. If the recovery copy is
  missing, save again to make it (failure warns).
- **Certificate of a slot**: what fingerprint the stored key has (or none for a bare key), or nothing stored.
- **Rebind** to a new fingerprint: public slot and valid fingerprint only (else invalid); nothing stored → false;
  else save under the new fingerprint, true.
- **Remove**: unregister the secret; delete the `.key2`, the `.key` and the recovery copy.
- **Encryptors**: Windows: DPAPI for the current user (no UI). Linux: AES-256-GCM, file = `rr1` + 12-byte random
  nonce + ciphertext and tag, with a 32-byte key kept in the Secret Service under attributes
  `application=io.github.freezamotoqueiro.dauntlessreforgedrevival`, `purpose=account-key-files`, label
  `DaintyLass Launcher account keys`, content type `application/octet-stream`, created on first use (locked items
  are unlocked first; D-Bus calls run on the blocking pool). No Secret Service → "unavailable". Other platforms:
  unavailable.
- **The Electron launcher's files (read only)**: Windows: `<userData>/Local State`'s `os_crypt.encrypted_key`, base64,
  must start with `DPAPI`; the rest DPAPI-decrypted is the 32-byte key; a file is `v10` + 12-byte nonce + AES-256-GCM
  ciphertext and tag. Linux: `v11` (password from the Secret Service item with
  `xdg:schema=chrome_libsecret_os_crypt_password_v2` and `application=Dauntless Reforged Revival Launcher`) or `v10`
  (password `peanuts`); key = PBKDF2-HMAC-SHA1(password, `saltysalt`, 1 iteration, 16 bytes); AES-128-CBC with an
  IV of 16 spaces and PKCS#7; the sealed part is non-empty and a multiple of 16.
- **Backup file** (`save_key_backup`): lines joined by CR LF: a title line naming the product's account key, a blank
  line, `Server: <name> (<host>:<port>)`, `Certificate: <fp>` (public only), `Username: <name or ->`,
  `Saved: <YYYY-MM-DD>`, `Key: <key>`, a blank line, a sentence telling the player to keep the file private
  because the key lets anyone play as them, a sentence saying how to use it on another PC (Register, I already have
  a key, Load a backup file), and a final empty line. The import finds the key anywhere in the text, so only the
  `Key:` line is load-bearing; see §3.3 for the two sentences.

#### 2.7.2 Settings (`settings.rs`)

- File `<userData>/settings.json`, read whole (unreadable or not JSON → defaults), sanitised, written as
  pretty-printed JSON (two-space indent, the key order below) through `.tmp` and rename after every change (which
  is sanitised again with the language it names, defaulting to English).
- **Key order and defaults**: `version` 1, `server` null, `savedServers` [], `gameRoot` null, `graphics` −1,
  `windowed` false, `streamerMode` false, `language` (the OS language's code), `gameLanguage` `auto`,
  `runtimeProfile` `production`, `hostTools` false, `usernames` {}, `backupOffered` {}, `trustedRuntimes` {},
  `localRuntimeDir` null, `hostWorkspace` null, `dumperDll` null; then, after `dumperDll` and in this order,
  `minimizeToTray`, `closeToTray` and `showGameConsole`, each written as `true` **only when it is JSON `true`** and
  absent otherwise, so a file without them is written byte for byte as before.
- **A server entry** is kept only when: `name` passes protocol `clean_server_name` (stored cleaned); `host` is a
  valid host and, unless `mode` is `public`, a private-mode host; `port` a number, integral, 1..65535; public needs a
  valid fingerprint. Written as `game` (`revived` only if it said so, else `reforged`), `mode`, `host`, `port`,
  `name`, `share` (private only, a valid share URL, else null), `fp` (public only), `code` (a valid invite code or
  null), in that order.
- **Saved servers**: valid entries, unique by mode+host+port, at most 20; the current server always replaces its
  entry or is inserted first, then the list is cut to 20. A current 1.4.4 server stays in the list but the current
  server becomes null.
- **Other fields**: `gameRoot` a folder path (below, < 200 units); `graphics` an integral number −1..4;
  `windowed`, `streamerMode`, `hostTools` true only when JSON `true`; `language` a known language code;
  `gameLanguage` one of the nine game languages; `runtimeProfile` one of two; `usernames` keys 24 lower-case hex with
  valid usernames (protocol); `backupOffered` such keys with `true` only; `trustedRuntimes` such keys with arrays of
  64-lower-hex ids, first occurrence of each, last 8, non-empty; `localRuntimeDir` a folder path < 260;
  `hostWorkspace` < 200; `dumperDll` a folder path < 260 ending in `.dll` (any case).
- **Folder path**: a string longer than 3 and shorter than the limit in UTF-16 units, no NUL; on Windows a drive
  letter, `:` and `\` or `/`; on Linux starting with `/`.

#### 2.7.3 Folders (`paths.rs`)

- `userData` = the OS config folder (roaming AppData on Windows, `$XDG_CONFIG_HOME` or `~/.config`) +
  `DaintyLass Launcher`; `documents` = the OS documents folder or `~/Documents`; `home`.
- `--user-data-dir <dir>` or `--user-data-dir=<dir>` (non-empty) among the arguments, else the environment variable
  `REFORGED_DEV_PROFILE`: then `userData` = `<dir>/User Data` and `documents` = `<dir>/Documents`.
- Keys in `<userData>/keys`, logs in `<userData>/logs`, recovery copies in `<documents>/DaintyLass/Account Keys`.
- **Migration** on every start: if `userData` is named `DaintyLass Launcher` and a sibling
  `Dauntless Reforged Revival Launcher` exists, move it there when the new one does not exist (rename, else copy),
  or copy in only the files the new one lacks (recursively, never overwriting). The same for
  `<documents>/Dauntless Reforged Revival` into `<documents>/DaintyLass`.

#### 2.7.4 The log (`log.rs`)

- Lines `<UTC time, ISO 8601 with milliseconds and Z> <INFO|WARN|ERROR> <message>`, each redacted, appended to
  `<logs>/launcher.log`; before a write, a file over 2 MiB is renamed to `launcher.old.log`. Before a log folder is
  set, lines go to stderr. Logging never fails the launcher.
- **Secrets**: registering a value of at least 6 units remembers it; a value starting `RRK_` also remembers the
  rest when that has at least 16 units and no line breaks. Unregistering removes the value and its `RRK_`-less rest.
- **Redaction**, in order: every remembered secret → `<hidden>`; `RRK_[A-Za-z0-9_-]+` → `RRK_<hidden>`;
  `-AUTH_PASSWORD=` (any case) followed by any run, **possibly empty**, of anything but JavaScript white space and
  quotes → the prefix + `<hidden>` (so `-auth_password='x y'` becomes `-auth_password=<hidden>'x y'`);
  `x-reforged-user-api-key` (any case), an optional quote, optional white space, `:` or `=`, optional
  white space and quote, then the value up to white space, a quote, `,` or `}` → the prefix + `<hidden>`.
  "White space" is JavaScript's `\s`.
- Library events (`tracing`, INFO and above) go into the same log as `<target>: <message> <field>=<value>...`;
  events from the `egs-api` crate are dropped for the whole subscriber (`epic::is_egs_api_target`), **including
  events the `log` crate forwarded through `tracing-log`**: their metadata's target is then `log`, and the real
  target is in the normalised metadata (`tracing_log::NormalizeEvent`), which is what must be checked (RL1 gap,
  found in the RL3 switch). Panics are
  logged as `panic: <info>`. An error's one-line description is its text, redacted.
- `test/cases/log.json` is the redaction oracle (§3.2).

#### 2.7.5 Starting the game (`launch.rs`)

- **Arguments** (key plausible, user id `^[A-Za-z0-9_-]{1,128}$`, username `^[A-Za-z0-9_]{1,32}$`, else refuse):
  `-DisableEOS`, `-EpicPortal`, `-NoEAC`, `-AUTH_TYPE=exchangecode`, `-AUTH_LOGIN=<user id>`,
  `-AUTH_PASSWORD=<key>`, `-epicapp=Archon`, `-epicenv=Prod`, `-epicusername=<username>`, `-epicuserid=<user id>`,
  `-epicaccountid=<user id>`, `-epicsandboxid=jackal`, `-epicdeploymentid=53565ba467df4edbb6f5a3d939a8b4f2`,
  `-epiclocale=<locale>` (§2.8.1, unknown → `en-US`); windowed adds `-windowed -ResX=1280 -ResY=720`.
- **Masking** for the log: any argument starting `-AUTH_PASSWORD=` (any case) → `-AUTH_PASSWORD=<hidden>`; the
  description is `<exe> <masked args joined by spaces>`.
- **Environment**: the backend URL must match `^http://127\.0\.0\.1:[1-9][0-9]{0,4}$`; from the base environment drop
  every variable whose upper-cased name starts with `REFORGED_`, `RUSTIC_ISLES_`, `RUSTIC_RIFTS_` or `MYSTICPARADOX_`; add
  `REFORGED_BACKEND_URL=<url>`, `RUSTIC_ISLES_RUNTIME_PROFILE=<profile>` and
  `MYSTICPARADOX_RUNTIME_PROFILE=<profile>`.
- **The game process**: one at a time (a second start is an error); the exe `Dauntless-Win64-Shipping.exe` in the
  Win64 folder, or, on Linux, the runtime's command with its prefix arguments, then the exe, then the arguments;
  working folder Win64; standard streams null; the environment cleared and set to exactly the computed one plus the
  runtime's variables. Listeners hear "started" and "exited with code"; the pid is cleared on exit. A test spawner
  can stand in.
- **Hidden helpers**: run with null stdin and stderr, killed on drop, without a console window on Windows, within a
  timeout and an output limit; output only on success, as UTF-8.
- **Running clients**: Linux: `pgrep -f Dauntless-Win64-Shipping.exe` (10 s), the numeric lines. Windows:
  `tasklist /FI "IMAGENAME eq Dauntless-Win64-Shipping.exe" /FO CSV /NH` (10 s, 1 MiB); if the exe is listed, a
  PowerShell (`-NoProfile -NonInteractive -ExecutionPolicy Bypass`, 15 s) query of Win32_Process for that name
  printing the ids of processes whose command line has no `-server` argument. Only ids ever leave PowerShell. The
  system folder is `%SystemRoot%`, else `%WINDIR%`, else `C:\Windows`.
- **Missing files** (Windows only, in System32): Visual C++ `MSVCP140.dll`, `VCRUNTIME140_1.dll`; DirectX
  `XINPUT1_3.dll`, `X3DAudio1_7.dll`, `XAPOFX1_5.dll`.

#### 2.7.6 The game's config files (`game_config.rs`)

- **Folder**: `%LOCALAPPDATA%` (else `%USERPROFILE%\AppData\Local`, else `C:\AppData\Local`) +
  `Archon\Saved\Config\WindowsClient`; on Linux the prepared launch's folder.
- **Reading**: UTF-16 LE with BOM, or UTF-8 (BOM dropped); lines split on CR LF, LF or CR; a final empty line
  dropped. **Writing**: lines each followed by CR LF; ASCII when possible, else UTF-16 LE with a BOM. Atomic through
  `<file>.rr-tmp` and rename.
- **Game.ini**: when present, remove the section `OnlineSubsystemPhoenix` (header matched trimmed and in any case,
  through the next header); count its key lines (not comments `;`/`#` or blanks, containing `=`); when anything was
  removed, copy the old file to `Game.ini.before-reforged` and write the rest.
- **Engine.ini**: drop the sections `[SystemSettings]` and `[OnlineSubsystemMcp.XMPP]` (headers matched by prefix, any
  case) and the blank lines that then lead the file; write first `[SystemSettings]` when a graphics level is forced
  (`sg.<group>Quality=<level>` for ViewDistance, AntiAliasing, Shadow, PostProcess, Texture, Effects, Foliage,
  Shading, then `sg.ResolutionQuality=100`, `r.ScreenPercentage=100`) and a blank line, then
  `[OnlineSubsystemMcp.XMPP]`, `ServerAddr="ws://<host>"`, `ServerPort=<port>`, `bUseSSL=false`, a blank line, then
  the kept lines. An invalid host or port 0 refuses.
- **GameUserSettings.ini** (when present, or when the safe window is asked): **display repair**: within sections
  whose name ends in `GameUserSettings` (case-insensitive, preceded by nothing or `.` after letters), a size key
  (`ResolutionSizeX/Y`, `LastUserConfirmedResolutionSizeX/Y`, `DesiredScreenWidth/Height`,
  `LastUserConfirmedDesiredScreenWidth/Height`, any case) whose value (up to `;`, parsed as JavaScript `Number`) is
  not finite or not 1..16384 makes it bad. Bad or safe window: add the two sections
  `[/Script/Engine.GameUserSettings]` and `[/Script/Archon.ArchonGameUserSettings]` when none exist; under each such
  section put `ResolutionSizeX=1280`, `ResolutionSizeY=720`, `LastUserConfirmedResolutionSizeX=1280`,
  `LastUserConfirmedResolutionSizeY=720`, `FullscreenMode=2`, `LastConfirmedFullscreenMode=2`,
  `PreferredFullscreenMode=2`, `WindowPosX=-1`, `WindowPosY=-1` and drop its other size and window keys (also
  `FullscreenMode`, `LastConfirmedFullscreenMode`, `PreferredFullscreenMode`, `WindowPosX`, `WindowPosY`). Repaired
  when the text changed; then the old file is copied to `GameUserSettings.ini.before-display-repair-<ms since
  epoch>`. A forced graphics level also rewrites each `sg.<group>Quality=` line (prefix any case) to the level.
- Answers the Engine.ini path, whether the display was repaired, and the Phoenix keys removed.
- `test/cases/game-config.json` is the oracle (§3.2).

#### 2.7.7 Finding the game (`game_folder.rs`)

- **Input**: trimmed; surrounding double quotes removed (and trimmed again); non-empty, ≤260 units, no NUL. Windows:
  a drive letter, `:`, `\` or `/` (nothing else: UNC, device and root-relative paths are refused before any file
  access); Linux: absolute. Normalised: separators unified, `.` and `..` folded, no trailing separator but the
  root's.
- **Candidates**, in order, deduplicated: if the last part is the exe name (any case), its folder; the folder;
  `<folder>/Dauntless`; when the folder is named `archon` (any case), its parent; when `win64`, three levels up.
  The first candidate holding the executable file is the root if it is ≤180 units; if every found one is longer:
  too long; none: not found.
- **Version.txt**: the first ≤256 bytes, lossy UTF-8, BOM dropped, trimmed, first line, `\r` dropped, trimmed; 1..128
  printable ASCII, else none. **Supported**: exactly
  `dauntless_rel-1.14.7_Shipping_2024-04-01_14-59-21_647472`, or containing `rel-1.14.7` (any case) and `647472` not
  adjacent to other digits.
- **Store detection** (Windows only; Linux answers none): Legendary's `installed.json` in `LEGENDARY_CONFIG_PATH`
  and `%USERPROFILE%\.config\legendary` (source `legendary`), Heroic's
  `%APPDATA%\heroic\legendaryConfig\legendary\installed.json` (source `heroic`): entries whose `app_name` (or key) is
  `Jackal`, with `install_path` (non-empty, ≤260) and `version` (first 64 units); Epic's
  `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests\*.item` (at most 500 files, each ≤1 MiB; JSON files ≤4 MiB),
  items whose `AppName` or `MainGameAppName` is `Jackal` or whose `DisplayName` is `Dauntless`, with
  `InstallLocation` and `AppVersionString`. JSON may start with a BOM. Each found path is located as above; keep
  found ones once (case-insensitive), with the store's version or else `Version.txt`'s.

#### 2.7.8 Invite links and the single launcher (`invite_link.rs`)

- **From arguments**: the first argument starting (any case) with `<scheme>:` for one of the protocol's invite
  schemes, kept (trimmed) only if it parses as an invite.
- **Channel name**: base id `com.daintylass.server` (manager) or
  `io.github.freezamotoqueiro.dauntlessreforgedrevival` (client); if a profile is in use (`--user-data-dir`, else
  `REFORGED_DEV_PROFILE`), append `-` and the profile path's last 24 ASCII alphanumeric characters. Windows:
  `\\.\pipe\<id>-<USERNAME's alphanumerics>`. Unix: `$XDG_RUNTIME_DIR/<id>.sock` when that folder exists, else
  `<temp>/<id>-<USER>.sock`.
- **Message**: the arguments as one JSON array line ending in `\n`; an unreadable line is an empty list.
- **Claim**: first try to send. Windows: open the pipe; "all instances busy" (error 231) retries every 50 ms up to 20
  times. Sent → hand over. Otherwise become the first: Windows creates the first pipe instance (if another launcher
  won the race, send again: sent → hand over, else first) and serves connections one at a time, making a new
  instance after each; Unix removes a stale socket, binds (on failure send again), sets it owner-only. Each
  connection's line goes to the handler (focus the window, unminimise, deliver the link).
- **Scheme registration** (packaged client builds): Windows, for each invite scheme in the current user's
  registry: `Software\Classes\<scheme>` default value `URL:DaintyLass invite` for `reforged-revival` and
  `URL:Dauntless invite` for the others, `URL Protocol` empty, and `...\shell\open\command` default value
  `"<exe>" "%1"`, written through `System32\reg.exe add ... /t REG_SZ /d <data> /f`, stopping at the first failure.
  Linux, only for an AppImage (`APPIMAGE` set): write `<data dir>/applications/dainty-lass-launcher-handler.desktop`
  (`[Desktop Entry]`, `Type=Application`, `Name=DaintyLass Launcher`, `Exec=<quoted path> %u`,
  `MimeType=x-scheme-handler/<scheme>;` for each scheme, `NoDisplay=true`, `Terminal=false`), remove the old
  `reforged-revival-launcher-handler.desktop`, and run `xdg-mime default <file> x-scheme-handler/<scheme>` for each.
  The `Exec` path is quoted with `"`, `` ` ``, `$` and `\` escaped, then `\` doubled and `%` doubled; a path that is not
  absolute or has control characters cannot be registered. Failures are logged.

#### 2.7.9 External links (`links.rs`)

- Fixed targets and URLs: `tailscale_download` `https://tailscale.com/download`; `vc_redist`
  `https://aka.ms/vs/17/release/vc_redist.x64.exe`; `project_source`, `project_license`, `project_issues`,
  `project_docs_players`, `project_docs_hosting`, `project_docs_sdk`: the project repository, its `LICENSE.txt`, its
  issues, and its `docs/players.md`, `docs/hosting.md`, `docs/sdk-and-game-data.md` (today the upstream project's
  GitHub addresses; the maintainer chooses DaintyLass's); `installing_guide` Mystic Paradox's
  `docs/INSTALLING_DAUNTLESS.md`; `legendary` Legendary's latest release; `dumper7` and `dumper7_zip` Dumper-7's
  repository and its `main` archive; `visual_studio` and `visual_studio_installer` the Visual Studio downloads page
  and the Community installer; `rust` and `rust_installer` the Rust install page and `https://win.rustup.rs/x86_64`;
  `nodejs`, `mongodb`, `git` their download pages; `revived_source`, `mystic_source`, `undaunted_source` the three
  upstream repositories. `tailscale_share` and `server_source` come from the server side.
- **Check**: at most 1024 units; parses as a URL; `https`, no user or password; no explicit port. Allowed: one of the
  fixed URLs exactly, or any URL on host `login.tailscale.com`; or the server's source URL exactly, where an
  explicit port is allowed.
- Opening uses the OS (`open` crate, detached); a folder opens only when it is a directory.

#### 2.7.10 The art scheme (`art.rs`)

A map from id (64 lower-case hex) to file and image type. A request names the id as `rr-art://bg/<id>` (WebKitGTK)
or `http://rr-art.localhost/bg/<id>` (WebView2). Found and readable → 200 with the type; else 404 with an empty
body; both with `cache-control: no-store` and `x-content-type-options: nosniff`.

#### 2.7.11 Dialogs (`dialogs.rs`)

- Pickers from the backend only, parented to the main window: folder (titled by purpose with the `dialog_*` keys
  of §2.8.2, starting at the given folder when it exists), save file (in Documents, with a default name and a filter),
  open file (optional start folder, filter), the Dumper-7 DLL (title `dialog_dumper_dll`, filter `DLL`/`dll`), a game
  data pack (title `dialog_gamedata_pack`, filter `Game data pack`/`gz`). Cancel → none.
- **Keep-open dialog**: a warning titled `app_title`, message `close_title` + blank line + `close_text`, buttons
  `close_keep` and `close_anyway`. Only pressing `close_anyway` closes; everything else keeps.

#### 2.7.12 Self-update (`updater.rs`)

- Updatable only for a bundle (Windows, or a Linux bundle type) with a non-empty updater public key in the config;
  otherwise log once and never check.
- Check at start and every hour; an update found is kept and the controller marks `updateReady` (logged); a failed
  check logs a warning.
- `install_update`: only when the controller allows (§2.4.10) and an update waits; download and install it, log,
  restart the app; failure logs.

#### 2.7.13 Proton and Wine (`linux_runtime.rs`)

launcher-kit has a clean-room `platform::linux_runtime`; the restricted behaviour below is the reference it must
match before the shell switches:

- Environment as a sorted map. **XWayland authority**: on a Wayland session without `XAUTHORITY`, the newest
  `.mutter-Xwaylandauth.*` file in `XDG_RUNTIME_DIR`.
- **Proton** (unless only a Wine is configured): `DAUNTLESS_REVIVED_PROTON` (a path, `~/` expanded, or a name on
  `PATH`, executable), else the newest `proton` under the Steam roots (`~/.local/share/Steam`, `~/.steam/root`,
  `~/.steam/steam`, and the Flatpak Steam's two) in `compatibilitytools.d/*/` and `steamapps/common/{proton*,
  ge-proton*}/`, newest by a natural, case-insensitive descending sort. Prefix data `<userData>/compat/proton`
  (`STEAM_COMPAT_DATA_PATH`), the Steam root found from the Proton path (`STEAM_COMPAT_CLIENT_INSTALL_PATH`),
  `STEAM_COMPAT_APP_ID=0`, `PROTON_LOG=0`, prefix `<compat>/pfx`; name `Proton (<folder>)`; command arguments `run`.
- **Wine**: `DAUNTLESS_REVIVED_WINE`, else `wine64` or `wine` on `PATH`, else the newest Lutris runner
  (`~/.local/share/lutris/runners/wine/*/bin/{wine64,wine}`); `WINEPREFIX=<userData>/compat/wine`, `WINEDEBUG=-all`
  unless set; name `Wine (<folder>)` or `Wine (system)`.
- `WINEDLLOVERRIDES`: the existing entries without any `dxgi=` one, plus `dxgi=n,b`.
- **Prepare**: detect; if none and auto-install is on (not a development build, `DAUNTLESS_REVIVED_AUTO_INSTALL` not
  `0`), install Wine with the package manager plan (logged) and detect again; none → missing. Initialise the prefix
  unless `drive_c` exists (Proton: `run cmd /c exit`; Wine: `wineboot -u`, from beside the Wine binary or `PATH`,
  else `wine wineboot -u`; 2 minutes; failure only warns). Config folder:
  `<prefix>/drive_c/users/<user>/AppData/Local/Archon/Saved/Config/WindowsClient`, user `steamuser` for Proton, else
  `USER`/`LOGNAME` with `\/:*?"<>|` replaced by `_`, at most 64, else `wineuser`.
- **Package manager plans**: the first of `apt-get` (`install -y wine64` / `install -y gnome-keyring libsecret-1-0`),
  `dnf` (`install -y wine` / `... gnome-keyring libsecret`), `zypper` (`--non-interactive install ...`), `pacman`
  (`-S --needed --noconfirm ...`), `xbps-install` (`-Sy ...`) found on `PATH` or in `/usr/bin`, `/usr/sbin`, `/bin`,
  `/sbin`, `/usr/local/bin`; run directly as root, else through `pkexec` (none → no plan); 15 minutes; the
  environment cleared to the given one.

#### 2.7.14 System checks (`system.rs`)

launcher-kit has a clean-room `platform::system`; reference behaviour:

- **Tailscale**: Linux `/usr/bin`, `/usr/local/bin`, `/snap/bin` and each `PATH` folder's `tailscale`; Windows
  `Tailscale\tailscale.exe` under `ProgramW6432`, `ProgramFiles`, `ProgramFiles(x86)`, `C:\Program Files` (unique),
  `%LOCALAPPDATA%\Tailscale`, and each `PATH` folder (quotes stripped). Found = the first that is a file.
- **Free space**: of the nearest existing ancestor.
- **Device signals** (not used by the controller since RA13; may be dropped): an SMBIOS UUID kept only in 8-4-4-4-12
  hex form, lower-cased, not all `0`/`f`, not `03000200-0400-0500-0006-000700080009`; a TPM EK SHA-256 kept only as 64
  hex, not all `0`/`f`; read on Windows by two hidden PowerShell queries (4 s, 4 KiB each).

#### 2.7.15 The rest (`platform/mod.rs`, `net/mod.rs`, `bridge/mod.rs`, `core/mod.rs`)

Module roots; the Host modules exist only with the `host` feature.

#### 2.7.16 The game's console (`game_console.rs`, A)

Moves as it is; summarised here for the controller and the shell. Windows only (elsewhere both calls do nothing).
`watch(pid, hide, sink)` runs this executable again as `--read-game-console <pid> [--hide]` with no window and its
stdout piped, replacing (killing) an earlier reader; a thread reads lines and hands each burst to the sink as one
batch. The reader attaches to the game's console, hides its window when asked, polls the screen buffer for new
rows and writes them to stdout until the game exits. `redact(line)` masks `RRK_` keys (to the first character that
is not alphanumeric, `_` or `-`) and the value of `-AUTH_PASSWORD=` (any case; a quoted value to its closing
quote, else to the next space) as `<hidden>`, because Unreal prints its command line. It needs `unsafe` for the
Win32 calls: at the root that is a per-item `#[allow(unsafe_code, reason = ...)]` with a SAFETY note, as
`crates/gateway/src/allowlist/windows.rs` does (the same holds for `keystore/os.rs` and `clipboard.rs`).

### 2.8 Core rules

#### 2.8.1 The game's language (`core/game_language.rs`)

- Locales in order: `en-US`, `de-DE`, `es-ES`, `fr-FR`, `it-IT`, `ja-JP`, `pt-BR`, `ru-RU`. A set game language is
  its locale. `auto`: the OS locale's first two characters, lower-cased, matched against the locales' language part
  in that order (`es-419` → `es-ES`, `pt-PT` → `pt-BR`), else `en-US`.
- The launch argument accepts only a listed locale, else `en-US`.

#### 2.8.2 The backend's own text (`core/i18n.rs`)

- Keys the backend shows (14): `app_title`, `close_title`, `close_text`, `close_keep`, `close_anyway`,
  `dialog_game_folder`, `dialog_runtime_folder`, `dialog_workspace_folder`, `dialog_sdk_folder`,
  `dialog_dumper_dll`, `dialog_gamedata_pack`, `epic_sign_in_title`, `tray_open`, `tray_quit`. Each must exist and
  be non-empty in every language (a test).
- What each says, for the new wording (L1; no placeholders in any of them): `app_title` the app's name;
  `close_title`, `close_text`, `close_keep`, `close_anyway` the keep-open dialog's title, its question (closing
  would end the game's connection while it plays), and its two buttons (stay open, close anyway); the six
  `dialog_*` the folder and file pickers' titles (choose the game folder, a local runtime folder, the Host
  workspace, the SDK dump folder, the Dumper-7 DLL, a game data pack); `epic_sign_in_title` the Epic sign-in
  window's title; `tray_open` and `tray_quit` the tray menu's two items (show the window, quit the app).
- `translate(language, key, vars)`: the language's text, else English's, else the key; every `{name}` replaced by
  its value.
- Language from a code: exact codes only. System language: the locale lower-cased with `_` as `-`; the exact code
  (case-insensitive), else the first language with the same language part, else English.

#### 2.8.3 Replaced by MIT crates

`core/invite.rs`, `status.rs`, `username.rs` re-export `protocol::{invite, status, account}`; `core/text.rs` and
`core/pins.rs` are covered by launcher-kit's `core::{text, pins}` (with `js_number`: trimmed, empty is 0, decimal
with exponent, `0x`/`0o`/`0b` integers, `Infinity`, otherwise NaN). The shared test tables
(`crates/protocol/tests/cases/*.json`) already run in dainty-lass.

### 2.9 The JavaScript and TypeScript tests (`launcher/test`)

- `api.test.ts` (B): drives dainty-lass-ui's page API (`ui/packages/bridge/src/api.ts`) through the isolation hook
  with a mocked Tauri: every command has a page method that calls it with the right argument names, the hook lets the
  page's calls through and drops malformed ones, events reach their listeners, and a dropped call times out. It also
  fails a command no page method calls.
- `isolation-hook.test.ts` (B): the hook's rules of §2.3.7, kind by kind, including Tauri's own calls and the
  channel pattern.
- Run with `node --import ./test/resolve-ui.ts --test "test/*.test.ts"` (Node ≥ 22.18).

### 2.10 The Rust tests to rewrite

- `controller/tests.rs`: the whole flow against a fake server on loopback (plain HTTP for private mode, a TLS
  gateway with a throwaway certificate for public): invite → status → register → the player's game (a fake exe and
  its pin) → the runtime offered, trusted, installed and checked → PLAY (relay, Engine.ini, arguments, environment) →
  the game's exit stops the relay; the Host page cases; saved 1.4.4 servers shown disabled.
- `net/relay/tests.rs`: forwarding (plain and pinned), headers, chunked bodies, upgrades and tunnels, the QoS page
  on both listeners, the local rules, `/__origin/` paths, the owner check, re-pointing, a pin mismatch.
- `net/hostapi/tests.rs` (with `test/cases/hostapi.json`): every parser of §2.6.3.
- `platform/game_config_tests.rs` (with `test/cases/game-config.json`): §2.7.6.
- `fixtures/host/src/fake_host_kit.rs`: a stand-in for `dainty-host`: a first argument `fail` makes `start` fail
  and `deaf` makes it ignore the stop line; `start` follows the host kit's control contract; `invite` answers one
  JSON line; anything else fails with an `Error:` line.
- Unit tests named in each section (throttle timings, window fit, link check, keystore ids, Electron decryption with
  Chromium's known key for `peanuts`, settings sanitising with `test/cases/settings.json`, redaction with
  `test/cases/log.json`).

---

## 3. What cannot be relicensed, and why

### 3.1 The language catalogs and `gen/strings.json`

`ui/packages/i18n/catalogs/en.json` and `pt-BR.json` (1,183 keys each) keep Dauntless Revived's and Reforged's text
word for word: plan section 6 lists "both languages word for word" among the things that must not change, and
restricted `NOTICE.md` leaves them under the repository's AGPL. `gen/strings.json` is generated from them, and
`build.rs` compiles its text into every launcher binary, so **every launcher binary carries AGPL text** as long as it
embeds that file.

A clean-room path, and what it costs:

1. **Rewrite the strings.** Someone who has not read the catalogs writes new text for every key from a description
   of what the key says (its screen, its purpose, its placeholders), in both languages. This changes what players
   and hosts read: every screen's wording. It contradicts plan section 6, so it is the maintainer's decision, not a
   row's.
2. **Narrow it to the backend.** The backend shows only the 14 keys of §2.8.2 (12 when this was written; the tray
   added two). A clean backend can embed only those 14, rewritten (14 new strings in each language: a small,
   visible change to the close dialog, the pickers' titles, the Epic window's title and the tray menu), and leave
   the page's other keys to the page. The page itself (dainty-dash)
   still ships the full catalogs, so the *installer* stays AGPL-bearing even then.
3. **Load the text at run time.** The backend reads the 12 strings from a data file installed beside it instead of
   compiling them in. The executable is then free of the text; the installer is not.

**Decided (L1, 2026-11-06): (2).** The 14 are rewritten in `en` and `pt-BR` from §2.8.2's descriptions,
into a new MIT catalog the MIT backend compiles in; the page's catalogs stay in `restricted/` as they are. The
key *names* and the placeholder syntax
(`{name}`) are an interface and can be used by MIT code.

### 3.2 DR's recorded test tables

`test/cases/game-config.json`, `hostapi.json`, `log.json` and `settings.json` are 51–100% DR's own tables (inputs
with expected outputs recorded from DR's TypeScript), extended for Reforged. They are the evidence that the Rust
code answers as the TypeScript did (plan section 6: settings bytes, redaction, the config files). They stay in
dainty-restricted and keep running there **against the MIT launcher-kit**: a test in dainty-restricted that
exercises an MIT crate does not change that crate's licence. A clean replacement would be new cases written from
§2.6.3, §2.7.2, §2.7.4 and §2.7.6 with expected values derived from the spec: Medium effort, optional.
`test/cases/manifest.json` is dead and can be deleted.

### 3.3 Prose inside the code

- The two advisory sentences of the key backup file (§2.7.1) are Reforged's wording. They are not in the catalogs
  because the file's format is English by design. A clean implementation writes its own two sentences; nothing
  reads them back (the import looks for the key), so the change is visible only to a player who opens the file.
  Maintainer's call, Small.
- Log messages and comments: implementers write their own; logs are not a contract (AGENTS.md: logs stay English
  and are a host's tool).

### 3.4 Generated interface files

`ui/packages/bridge/src/generated/types.ts`, `isolation/commands.js` and `permissions/app-commands.toml` are
generated from AGPL sources. Their names, shapes and limits must not change (§2.3.6). **Amended (L4, 2026-11-06):**
`types.ts` is not prose-free after all: ts-rs copies each Rust type's rustdoc into it, much of it the AGPL shell's.
The comparison is therefore on a normalised form, without doc comments, banners or the permission set's
`description` (`crates/launcher-kit/tests/fixtures/bridge/README.md`), and the MIT types carry their own docs.
This document reproduces their content as an interface (§2.3), and the implementer may compare the generator's
output against them. Treating interface declarations this way is the same footing the earlier clean rooms gave the
shared test tables in `crates/protocol/tests/cases`; if the maintainer prefers a stricter line, the implementer
works from §2.3 alone and the comparison is done by a reviewer outside the clean room.

### 3.5 Assets

The 15 icons are not code. They match neither DR's nor Reforged's icon file byte for byte, but their origin before
the 2026-10-26 root is not recorded in either repository. The maintainer confirms who drew them and under what
terms before they ship with an MIT shell.

### 3.6 What links AGPL code

Not a provenance question, but it decides the binaries' licence:

Refreshed by RL1:

- `controller/realms.rs` and its tests link `rifts-host` and `rifts-seam`: both are MIT clean-room crates at the
  root now. The `realms` feature no longer decides the client's licence.
- `server_gui.rs`, the A tail of `controller/host.rs` and the manager's features link `restricted/crates/host`
  (`host::serve::prepare_embedded`, its hooks, `host::commands::*`). That crate's own dependencies are all MIT; its
  one restricted input is `modules/rustic-isles/version-data/1.14.7/version_data.json` (`version_data.rs`, Paradox
  data). Until the host shell moves to the root, with the version data read at run time or supplied by a small
  restricted crate (outside this spec), `server-gui` makes `dainty-manager` an AGPL build.
- The page each shell serves is dainty-dash (`ui/`, MIT). It shows the language catalogs' text, which stays AGPL
  (§3.1), so the installers carry AGPL text whatever the binaries' licence.

---

## 4. Target MIT design

### 4.1 Principle

Everything that does not link an AGPL crate goes to the root under MIT: the B groups rewritten into
`crates/launcher-kit`, the A files moved, and a new MIT Tauri shell crate. `restricted/` keeps only what can't be
relicensed: the page's catalogs, DR's recorded tables and the tests that read them, the Electron key-store harness,
today's icons (L2), and, until the host shell moves, the manager build that links it.

### 4.2 Crates

| Crate | Repository | Licence | Holds |
| --- | --- | --- | --- |
| `crates/launcher-kit` | dainty-lass | MIT | Today's modules, plus: `bridge::{spec (full command table with argument shapes), types (all wire types), progress, checks}`; the generator as a test-only module; `controller` (§2.4, §2.5, the A controller modules) over a `Platform` trait with no Tauri types; `net::{connector, hostapi, relay, relay_owner, epic_install}`; `platform::{keystore, settings, paths, log, launch, game_config, game_folder, invite_link, links, clipboard}`; `core::{game_language, i18n (lookup over a table given at build or run time)}`. Features `host`, `server-gui`, `realms` gate the same things as today |
| `crates/launcher-app` (new, Tauri shell; the package name `launcher` is restricted's) | root | MIT | `main.rs`, `lib.rs` (§2.1), `window.rs`, `tray.rs`, `bridge::{commands, backend}`, the Tauri `Platform` impl, `art.rs`, `dialogs.rs`, `updater.rs`, `epic_login.rs` (the sign-in webview), `build.rs` (strings from the MIT catalog of §2.8.2), the Tauri configs with the icon folder given by the build (L2), `tests/bundles.rs`. The isolation hook, TS tests and scripts are already in the root's `launcher/`. Builds `dainty-launcher` with or without `realms` |
| `restricted/launcher/src-tauri` | restricted | AGPL | A thin crate over the MIT shell: today's icons, the DR test tables' tests against launcher-kit, the Electron keystore harness, the `gen/strings.json` generator for the page, and the manager build (`server-gui`) until the host shell moves |

### 4.3 Seams

- **Controller ↔ app**: launcher-kit `controller::Platform` (the trait of §2.4.1), implemented by the MIT shell.
- **Realms**: no trait. `controller/realms.rs` (A) moves into launcher-kit's `controller::realms` behind the
  `realms` feature and links `rifts-host`/`rifts-seam` directly; `realms_off.rs` stays the stub
  (`not_in_this_build`). Its surface, unchanged: `realm_host(kind)`, `realm_join(realm)`, `realm_stop()`,
  `realm_get()` (each answering `RealmOutcome`), `realms_shutdown()` (logout, server switch, exit) and, for tests,
  `set_realms_seams(RealmsSeams { loader, ... })` and `live_host_backend()`. Its requests go through launcher-kit's
  `net::{http, hostapi::key_headers}`, so it moves after RL7.
- **Embedded server**: launcher-kit's existing `EmbeddedServer` and `ModulePanel` traits. `server_gui.rs` (A) moves to
  launcher-kit once `prepare_embedded` and the hooks are reachable from the root, i.e. once the host shell
  (`restricted/crates/host`) moves; until then the manager build stays in `restricted/`.
- **The in-process host kit of the manager** (the A tail of `controller/host.rs`): it calls the host kit's commands
  through `host::*`, which are `hostkit::*` re-exports plus the hooks; it moves to launcher-kit calling `hostkit`
  directly, with the same caller-supplied hooks.
- **Backend strings**: `core::i18n` takes its tables from the MIT catalog of the 14 rewritten keys (L1, §3.1)
  instead of compiling `gen/strings.json`.
- **Fuzz targets**: `launcher_files`, `launcher_input`, `launcher_net`, `relay_input` include launcher files by path;
  they move with the rewritten modules to the root's `fuzz/`.

### 4.4 What dainty-restricted keeps

The page's language catalogs and the `gen/strings.json` generator, the DR test tables running against the MIT
crate (with new MIT tables beside them at the root, L5), the Electron keystore harness, today's icons (L2), and the
manager build until the host shell moves. The generator for `types.ts`, `commands.js` and `app-commands.toml` moves
with `bridge::spec` to launcher-kit; from then `bun run restricted-data` is no longer how the root gets them, and
`ui/packages/bridge/src/generated/types.ts` is MIT generated output.

### 4.5 The binaries afterwards

| Binary | Build | Licence |
| --- | --- | --- |
| `dainty-launcher`, with or without `realms`, with the 14 rewritten strings | root | MIT (the installer's page text and icons aside, §3.6, L2) |
| `dainty-manager` | `restricted/` | AGPL until the host shell moves to the root; then it can follow the launcher |

---

## 5. Effort estimate

Line counts are non-blank lines of today's files, tests included.

| Group (§) | Files | Lines | Work | Size |
| --- | --- | ---: | --- | --- |
| Core rules (§2.8) | `core/*` | 590 | Mostly deletion in favour of `protocol` and launcher-kit; `game_language` and `i18n` rewritten | Small |
| Requests, pinning, connector (§2.6.1, §2.6.2) | `http`, `pinned`, `to_file`, `connector`, `net/mod` | 1,170 | Close the API gap of the existing launcher-kit modules; write the connector | Small |
| Epic install job (§2.6.6) | `epic_install` | 205 | Fresh adapter over `epic::download` | Small |
| Linux runtime, system (§2.7.13, §2.7.14) | 2 | 850 | Bring launcher-kit's versions to parity, then switch | Small |
| Log (§2.7.4) | `log` | 210 | Rewrite; `log.json` oracle | Small |
| Host API and parsers (§2.6.3) | `hostapi`, `parse`, `tests` | 1,150 | Rewrite; `hostapi.json` oracle | Medium |
| Key store (§2.7.1) | `keystore/{mod,format,os,legacy}` | 1,110 | Rewrite; Windows DPAPI and Secret Service bindings; the Electron harness as oracle | Medium |
| Settings and folders (§2.7.2, §2.7.3) | `settings`, `paths` | 635 | Rewrite; `settings.json` oracle (byte-exact) | Medium |
| Launch, game config, game folder (§2.7.5–2.7.7) | 4 | 1,980 | Rewrite; `game-config.json` oracle | Medium |
| Links, invite link, art, dialogs, updater (§2.7.8–2.7.12) | 6 | 1,050 | Rewrite; Windows registry, AppImage entry, the single-instance channel | Medium |
| Relay and owner check (§2.6.4, §2.6.5) | 5 | 2,280 | Rewrite; HTTP/1.1 forwarding, upgrades, tunnels, the netstat check | Large |
| Bridge (§2.3) | `spec`, `types`, `commands`, `generate`, `backend`, `progress`, `hook.js`, `index.html`, TS tests | 4,600 | Mostly interface tables; byte-identical generated files are the hard part | Medium |
| App shell and window (§2.1, §2.2) | `lib`, `main`, `window`, `build.rs`, configs | 690 | Rewrite in the MIT shell crate | Medium |
| Controller (§2.4) | `mod`, `platform`, `stored`, `game`, `runtime`, `tests` | 4,460 | The state machine and its integration tests | Large |
| Host page controller (§2.5) | `host.rs` lines 1–1171 | 1,100 | Rewrite; DL4 §2.7 is not usable as it stands | Medium |
| Moving the A files (§1) | 51 | — | Mechanical, with the seams of §4.3 | Medium |
| Strings decision (§3.1) | catalogs | — | Maintainer's decision; then 12 or 1,183 strings × 2 languages | Small (12) / Large (all) |

Suggested order: the small network and platform groups first (they unblock the rest and three already exist in
launcher-kit), then the key store and settings (they guard plan section 6's stored data), the host API and the relay,
the bridge, the controller and the Host page, and the shell last, when every module it wires is MIT. The strings
decision can be taken at any point; it only gates the client binary's licence.

---

## 6. Acceptance for the implementers

- `RR_UPDATE_GENERATED` unset, the generator's test passes: its `types.ts`, `commands.js` and `app-commands.toml`,
  normalised, equal `crates/launcher-kit/tests/fixtures/bridge/` byte for byte (L4 as amended). `strings.json`
  stays generated in `restricted/` (L1).
- A reviewer outside the clean room runs `restricted/`'s suites against the MIT crates, unchanged; they pass: the
  controller flow, the relay, the host API and game-config tables, settings and log tables, `bun run test` in
  `launcher/`, and the fuzz targets build.
- The new MIT tables (L5) pass at the root.
- Similarity of each rewritten file, by `similarity.md`'s measure against both DR's `legacy/UndauntedLauncher` at
  `e1c5fec` and `restricted/launcher/` at the commit before RL2 (and `launcher/` at `849f45e`): below 30% per
  file, and no run longer than 6 lines outside import lists, constant tables that §2 fixes (command table, name
  lists, launch arguments, config keys) and
  match arms over the same enum.
- `cargo tree` of every dainty-lass crate shows no AGPL crate, and the MIT client builds without `realms`.
- No catalog text and no DR or Reforged identifier in comments of the new files.
