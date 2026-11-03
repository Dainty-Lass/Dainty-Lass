<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# RD27 Clean-Room Specification: the launcher's JavaScript and TypeScript (`launcher/` in dainty-lass)

Addendum to [`spec-rd27-launcher.md`](spec-rd27-launcher.md) (the launcher spec). That document classified every
non-Rust file under dainty-restricted's `launcher/` (its §1.4) and specified the isolation hook (§2.3.7) and the
bridge tests (§2.9). This one covers the rest: the scripts, the end-to-end desktop tests and the package itself.
It also records the maintainer's decision of 2026-11-02 to rebuild all of it as MIT in dainty-lass, on better tools
where there are some, instead of moving the Category A files as they are. Source: dainty-restricted `plan` at
`4efc8b5`.

> **Clean-room rule.** The implementer reads this document, the launcher spec's §2.3 (the bridge, for the command
> table's shape) and §2.3.7 and §2.9, dainty-lass's own files (`ui/` above all: `ui/packages/bridge/src/api.ts`,
> `call.ts`, `ui/scripts/restricted-data.mjs`) and nothing else. That means nothing under dainty-restricted, at any
> commit, including its `launcher/` and the copies of it under `legacy/`. The one exception is the generated
> `isolation/commands.js`, which the new package copies from a restricted checkout at test time (§2.2) and which
> the implementer may open to see the table's format (launcher spec §3.4). Nothing written from this document
> names DR's or Reforged's files, commits or identifiers in comments.

---

## 1. Scope and target

### 1.1 What is replaced

| restricted `launcher/` today | Cat. | Becomes (dainty-lass `launcher/`, MIT) |
| --- | --- | --- |
| `isolation/hook.js`, `isolation/index.html` | B | `isolation/hook.js`, `isolation/index.html` (§2.1) |
| `isolation/commands.js` | G | not committed here; copied from restricted by `restricted-data` (§2.2) |
| `test/isolation-hook.test.ts` | B | `test/isolation-hook.test.ts` (§3.1) |
| `test/api.test.ts` | B | `test/api.test.ts` (§3.2) |
| `test/hook.ts` | A | `test/hook.ts` (§3.3) |
| `test/resolve-ui.ts` | A | **dropped**: Bun resolves the UI's extensionless imports itself (§1.3) |
| `test/bundles.test.ts` | A | **not here**: it checks restricted's Tauri configs, so it becomes a Rust test there (§5) |
| `test/e2e-desktop.mjs`, `test/e2e-remote-desktop.mjs` | A | `e2e/desktop.ts`, `e2e/remote.ts`, `e2e/lib/*.ts` on Playwright (§4) |
| `test/electron-keystore/**` | A | **stays in restricted**: it runs Reforged's own Electron sources |
| `test/cases/*.json` | D | **stay in restricted** (launcher spec §3.2); `manifest.json` is deleted there |
| `scripts/dashboard-page.mjs` | A | `scripts/dashboard-page.ts` (§2.3) |
| `scripts/release-config.mjs` | A | `scripts/release-config.ts` (§2.4) |
| `scripts/sign-windows.ps1` | A | `scripts/sign-windows.ts` (§2.5) |
| `package.json`, `package-lock.json`, `tsconfig.json` | A | `package.json`, `bun.lock`, `tsconfig.json` (§1.3) |

The Rust shell (`src-tauri`), its configs and icons are the launcher spec's work, not this one's. Until the MIT shell
lands, restricted's `src-tauri` keeps building, and takes the files above from a dainty-lass checkout (§5).

### 1.2 Licence

Every new file starts with `// SPDX-License-Identifier: MIT` (`<!-- ... -->` in HTML). `package.json` says
`"license": "MIT"`. The package depends on nothing AGPL.

### 1.3 Tools

| Today | Replaced by | Why |
| --- | --- | --- |
| npm, `package-lock.json` | Bun, `bun.lock`, `"packageManager": "bun@1.4.4"` | `ui/` and the dashboard already build with Bun; one tool for every JS package in both repositories |
| Node's test runner, `node:test`, plus a module-resolution hook (`resolve-ui.ts`) | `bun test` (`bun:test`'s `test`, `expect`, `mock`) | Bun runs TypeScript and resolves the UI's extensionless relative imports itself, so the hook goes |
| `node scripts/*.mjs` spawning `bun` | `bun scripts/*.ts` | One runtime; the scripts are typed |
| `pwsh -File sign-windows.ps1` | `bun scripts/sign-windows.ts` | No PowerShell 7 needed on the release runner; same tool (`signtool`) underneath |
| Hand-written CDP client over `WebSocket`, duplicated in both e2e files | Playwright's `chromium.connectOverCDP` (`playwright-core`, no browser download) | Official WebView2 support, real locators and `fill()` in place of setting `value` through the prototype, one shared helper |
| `tsc --noEmit` | unchanged (`typescript` ~6.0) | Bun doesn't type-check |

Versions are pinned exactly in `package.json` (`@tauri-apps/api` 2.12.4, `playwright-core`, `typescript`,
`@types/bun`), and `bun install --frozen-lockfile` must pass in CI.

### 1.4 Layout

```text
launcher/
  package.json  bun.lock  tsconfig.json  README.md  .gitignore
  isolation/hook.js  isolation/index.html        (commands.js: copied, git-ignored)
  scripts/restricted-data.ts  dashboard-page.ts  release-config.ts  sign-windows.ts
  test/hook.ts  isolation-hook.test.ts  api.test.ts  scripts.test.ts
  e2e/lib/app.ts  e2e/lib/nsis.ts  e2e/desktop.ts  e2e/remote.ts
```

`package.json` scripts: `restricted-data`, `test` (`bun run restricted-data && bun test test/`), `typecheck`
(`bun run restricted-data && tsc --noEmit`), `build:pages`, `e2e:desktop`, `e2e:remote`. Nothing under `e2e/` runs in
`bun test`: those files are not named `*.test.ts`.

---

## 2. Runtime files and scripts

### 2.1 The isolation hook

As the launcher spec §2.3.7, which stands, with these details it leaves implicit:

- `commands.js` defines two globals: `window.__RR_COMMANDS__` (command name → field name → shape) and
  `window.__RR_SETTINGS_KEYS__` (the settings keys a `settingsPatch` may name: today `graphics`, `windowed`,
  `streamerMode`, `hostTools`, `language`, `gameLanguage`, `runtimeProfile`, `gameEdition`). The hook reads both and
  hard-codes neither.
- A shape is an object with `kind` and, by kind: `max` (`text`, `optionalText`, `index`, and optionally `number`),
  `min` (optionally `number`), `names` (`name`), `nullable` and `fields` (`object`).
- The payload Tauri passes has `cmd` and `payload` (the arguments) beside other fields the hook ignores. A payload that
  is not a non-null object is dropped.
- Command lookup uses own properties only: `__proto__`, `constructor`, `toString` and other inherited names are
  unknown commands. Tauri's own calls are looked up the same way, after the generated table.
- In an `object`'s `fields`, a field absent from the value is checked as not present (so `optionalText` may be absent,
  every other kind refuses), and a value's field absent from `fields` refuses.
- `number` also refuses `NaN` and ±`Infinity`; `index` refuses non-integers, negatives, `NaN` and values above `max`.
- A `settingsPatch` value that is not a plain object (an array, null, a string) refuses. An empty object passes.
- The hook builds its never-settling promise once and returns that same object for every dropped call.
- `index.html`: `<!doctype html>`, `lang="en"`, UTF-8, then `commands.js` and `hook.js` as classic scripts, in that
  order.

### 2.2 `scripts/restricted-data.ts`

Copies `launcher/isolation/commands.js` from a dainty-restricted checkout into this package's `isolation/` (git-ignored
here). The checkout is `DAINTY_RESTRICTED_DIR`, or `dainty-restricted` beside this repository's root, as
`ui/scripts/restricted-data.mjs` finds it. A missing source prints which path it looked at and that
`DAINTY_RESTRICTED_DIR` sets it, then exits 1. It also runs `ui`'s `restricted-data` so `api.test.ts` sees the
generated bridge types.

### 2.3 `scripts/dashboard-page.ts`

Builds or serves one of the dashboard's pages for a Tauri shell.

- Usage: `bun scripts/dashboard-page.ts <build|dev> <launcher|server-manager>`. Anything else prints the usage line
  and exits 2.
- The dashboard checkout: `DAINTY_DASHBOARD_DIR`, else `dashboard/` in the restricted checkout (§2.2's rule). The
  page's app is `<dashboard>/apps/<page>`; without its `package.json`, print where it looked and that
  `DAINTY_DASHBOARD_DIR` sets it, and exit 1.
- In the dashboard: `bun install --frozen-lockfile`, then `bun run theme:build` (the apps import the theme's built
  CSS). Any step that fails exits with that step's code.
- `dev`: in the app, `bun run dev --port <p> --strictPort`, with `p` = 4321 for `launcher` and 4322 for
  `server-manager` (the ports the two Tauri configs' `devUrl` name).
- `build`: in the app, `bun run build`; then replace `<out>/<page>` with a copy of `<app>/dist/client`, leaving out
  any `.vite` folder (Vite's manifest, never fetched). `<out>` is `LAUNCHER_DIST_DIR`, else `dist/` in the restricted
  checkout's `launcher/` (where its Tauri configs' `frontendDist` point). Print `<page>: <from> -> <to>`.
- Child processes inherit stdio. Use `Bun.spawnSync`.

### 2.4 `scripts/release-config.ts`

Writes the release builds' `tauri build --config` overlays.

- Usage: `bun scripts/release-config.ts [<out dir>] [--tauri-dir <dir>]`. `<out dir>` defaults to `.` and is created.
  `--tauri-dir` (default: the restricted checkout's `launcher/src-tauri`) holds `tauri.conf.json` and
  `tauri.server-gui.conf.json`.
- Updater signing is on when `TAURI_SIGNING_PRIVATE_KEY` is non-empty. Then, if `tauri.conf.json` has no non-empty
  `plugins.updater.pubkey`, print
  `::error::TAURI_SIGNING_PRIVATE_KEY is set but tauri.conf.json has no updater pubkey` to stderr and exit 1 (a signed
  feed the launcher can't verify would strand every install, RD7). Otherwise the overlay sets
  `bundle.createUpdaterArtifacts: true`.
- Code signing is on when `WINDOWS_CERTIFICATE_PATH` is non-empty. Then the overlay sets
  `bundle.windows.signCommand` to `{ "cmd": "bun", "args": [<absolute path of sign-windows.ts>, "%1"] }`.
- The overlay always has a `bundle` object, possibly empty.
- `client.json` = the overlay. `manager.json` = `tauri.server-gui.conf.json` merged with the overlay as a JSON merge
  patch (RFC 7386, which is how Tauri merges `--config`: objects merge key by key, anything else replaces; this
  overlay has no nulls). Both are written with two-space indentation and a trailing newline.
- Last, print `updater signatures: on|off; Windows code signing: on|off`.
- The merge is an exported function, so `test/scripts.test.ts` can check it.

### 2.5 `scripts/sign-windows.ts`

Authenticode-signs one file; Tauri runs it once per file as `signCommand`.

- Usage: `bun scripts/sign-windows.ts <file>`. No file: usage, exit 2.
- `WINDOWS_CERTIFICATE_PATH` must name an existing file, else exit 1 with
  `WINDOWS_CERTIFICATE_PATH is not set to the certificate file`. The password is `WINDOWS_CERTIFICATE_PASSWORD`. Both
  come from the environment, never the command line Tauri prints.
- `signtool.exe`: the one under `%ProgramFiles(x86)%\Windows Kits\10\bin\<version>\x64\` with the highest version
  (compared as dotted numbers; a folder whose name isn't one sorts last). `SIGNTOOL` overrides the search. None found:
  exit 1 with `signtool.exe not found: install a Windows 10/11 SDK`.
- The timestamp server: `WINDOWS_TIMESTAMP_URL`, else `http://timestamp.digicert.com`.
- Run `signtool sign /q /fd sha256 /td sha256 /tr <timestamp> /f <certificate> /p <password> <file>`, with arguments
  passed as an array (no shell). A non-zero exit: exit 1 with `signtool failed on <file> (exit <code>)`. Never print
  the password or the full argument list.

---

## 3. Tests (`bun test`)

### 3.1 `test/isolation-hook.test.ts`

Tests the hook against the real `commands.js` (copied by `restricted-data`), through `test/hook.ts`. They are the
launcher spec's §2.9 cases, which must all be covered:

- Every generated command passes with arguments of the right shape. Build each from its own shapes (a string of
  length `max` for `text`, `true` for `flag`, `max` for `index`, the first of `names`, a value inside the bounds for
  `number`, a recursively built object, a valid channel), so a new command needs no test edit. Also assert the table
  isn't empty and has more than 90 entries.
- Tauri's calls pass: `plugin:event|listen` with an event, any target and a numeric handler; `plugin:event|unlisten`;
  each window call with and without `label`.
- Unknown and forged names are dropped: plugin calls the page never makes (`plugin:fs|read_file`,
  `plugin:shell|execute`, `plugin:opener|open_url`, `plugin:clipboard-manager|write_text`), a known name with a
  trailing space, `__proto__`, `constructor`, `toString`, a number, null. A dropped call returns a thenable that is not
  the payload.
- Per kind, at its limits: `text` at `max` passes and at `max + 1` fails, a 1 MiB string fails; `index` at 0 and
  `max` passes, `max + 1`, −1, 1.5, `NaN`, `Infinity`, `"1"`, null and `[1]` fail; `name` passes exactly a listed name
  and fails a case or whitespace variant, `__proto__`, `""`, a number, null; `object` refuses an unknown field, a
  missing required field, null unless nullable, an array, a string; an absent `optionalText` field passes;
  `number` refuses out-of-bounds, `NaN` and strings; `settingsPatch` refuses unknown keys, a string over 16 units, an
  array; `channel` refuses anything not `__CHANNEL__:` plus 1 to 16 digits; arguments that are an array fail; absent
  or null arguments count as `{}`.
- A dropped call stays pending: race it against a 20 ms timer; the timer wins.

Use commands the table has today where a case needs a concrete one (`register`'s `username`, max 32; `copy_text`,
max 4096; `use_detected_game`'s `index`, max 15; `host_setup`'s `request` object; `host_set_multipliers`, a nullable
object of two bounded numbers; `set_settings`' `patch`; `subscribe_progress`' `onProgress` channel). Look the shapes
up from the table rather than restating the limits, so the test follows the generator.

### 3.2 `test/api.test.ts`

As the launcher spec §2.9. It imports `ui/packages/bridge/src/api.ts` by relative path within this repository (no
environment variable: the UI is in-tree), installs Tauri's IPC mocks (`@tauri-apps/api/mocks`: `mockIPC`,
`mockWindows`, `clearMocks`), and checks:

- Calling every method `api` exposes reaches the command of the generated table with the argument names the table
  gives, and every command in the table is reached by some method. Enumerate the methods from the `api` object;
  a fixed list of arguments per method is fine.
- Every call `api` makes passes the hook (feed each recorded `{ cmd, args }` to it as a payload).
- A call the hook would drop times out: with `CALL_TIMEOUT_MS` from `api.ts` shortened through a fake timer or by
  answering after the timeout, the page's promise rejects or resolves to its timeout result, as `call.ts` defines.
- Events reach their listeners with the shapes `api.ts` checks (a malformed window-state payload counts as `false`),
  and progress arrives over the channel `subscribe_progress` is given.
- The window controls make Tauri's window calls in order and each passes the hook.

The Tauri API reads `window.__TAURI_INTERNALS__`; set `globalThis.window = globalThis` before importing it.

### 3.3 `test/hook.ts`

Loads `isolation/commands.js` and `isolation/hook.js` into a fresh `node:vm` context whose only global is an empty
`window` object, in that order, and returns `{ hook, commands, settingsKeys }`. Each call makes a new context.

### 3.4 `test/scripts.test.ts`

- `release-config.ts`'s merge: nested objects merge, arrays and scalars replace, a key only in the patch is added.
- `release-config.ts` run as a process with a temporary `--tauri-dir`: no secrets gives `{ "bundle": {} }` and the
  manager file is the server-gui config plus that; the updater key without a pubkey exits 1; with a pubkey and a
  certificate path both overlay fields are set.
- `dashboard-page.ts` with bad arguments exits 2.

---

## 4. End-to-end desktop tests (`e2e/`, Windows, opt-in)

These drive the real apps. They need a Windows PC with WebView2 and the built bundles. They don't run in CI.

### 4.1 `e2e/lib/nsis.ts`

- `install(installer, dir, exe)`: runs `<installer> /S /D=<dir>` (`/D=` last and unquoted, as NSIS requires), asserts
  exit 0 and that `<dir>/<exe>` exists, returns its path.
- `uninstall(dir)`: when `<dir>/uninstall.exe` exists, runs it with `/S _?=<dir>` (synchronous, in place, removes the
  Add/Remove Programs entry); ignores failures.
- `installer(bundleDir, productName)`: the one file matching `<productName>_<version>_x64-setup.exe` in `bundleDir`,
  with `version` read from the restricted checkout's `launcher/src-tauri/tauri.conf.json` (or its `Cargo.toml`), not
  hard-coded. Zero or several matches throw, naming the folder.

### 4.2 `e2e/lib/app.ts`

`launch(exe, { profile, port, label, env? })`:

- Spawns the exe with `REFORGED_DEV_PROFILE=<profile>` and
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>`.
- `chromium.connectOverCDP("http://127.0.0.1:<port>")`, retrying for up to 30 s. The app's page is the first page
  whose URL starts with `http://tauri.localhost` and doesn't contain `__tauri_isolation__`.
- Waits up to 16 s for `window.__TAURI_INTERNALS__.invoke` to be a function.
- Returns an `App` with:
  - `invoke(cmd, args)`: `page.evaluate` of `__TAURI_INTERNALS__.invoke`, rejecting after 15 s with
    `invoke timeout for <cmd>`. It goes through the isolation hook like any page call.
  - `setLanguage(lang)`: `invoke("set_settings", { patch: { language } })`, then waits until
    `document.documentElement.lang` equals it (2 s at most).
  - `view(name)`: clicks `[data-view="<name>"]`. `tab(index)`: clicks the index-th `[role="tab"]`.
  - `shot(name)`: `page.screenshot` to `docs/port/screenshots/<name>.png` in this repository.
  - `shots(base, langs = ["en", "pt-BR"])`: for each language, set it and take `<base>-<lang>.png`; leaves the page
    in English.
  - `waitFor(cmd, args, predicate, timeoutMs)`: polls `invoke` every 200 ms until the predicate holds.
  - `close()`: closes the CDP connection and kills the process if it is still running.
- Use Playwright locators (`page.locator(...).click()`, `.fill()`) for UI interaction; `fill` replaces the
  value-setter trick.

### 4.3 `e2e/desktop.ts`

The flow, with numbered progress lines (`[n/8] ...`):

1. A temporary root (`dl-e2e-desktop-` in the OS temp folder) with `install-client`, `install-server`,
   `profile-client/User Data` and `profile-server/User Data`.
2. Install both bundles (`DaintyLass Launcher` → `dainty-lass-client.exe`, `DaintyLass Manager` →
   `dainty-lass-manager.exe`) from `<restricted>/target/release/bundle/nsis`, or `E2E_BUNDLE_DIR`.
3. Launch the manager (CDP port 9321) and the client (9322). Wait until the manager's `get_snapshot` has
   `app.serverGui === true`. Assert: the manager's `host_get` workspace ends with `Server` and `workspaceOk` is true;
   `host_download_project` answers `ok: false` with error code `not_in_this_build`; the manager's `document.title` is
   `DaintyLass Manager`; the client's snapshot has `app.serverGui === false` and `phase === "join"`. Screenshots:
   `client-join-*`; and `server-gui-setup-*` with the manager's `main > div` scrolled to 620 px (then back to 0).
4. Manager: `host_action { action: "synthetic_data" }`; `host_setup { request: { name: "Desktop E2E Hunts", mode:
   "private", publicHost: null } }`; `server_config_get` shows `configured` and that name; `server_doctor` returns at
   least one check; `host_server_start`; wait (15 s) until `host_server_get`'s state isn't `starting` and assert
   `running`; `host_create_invite { forName: "Slayer42" }`, then the server view has an `invite.link` and
   `invite.for === "Slayer42"`; `server_invite_list` lists exactly one invite with a code. On tab 0 click the buttons
   labelled `Run doctor` and `Refresh invites`. Screenshots for tabs 0 `server`, 1 `status`, 2 `modules`,
   5 `gamedata`, 6 `build`, 7 `requirements` as `server-gui-<name>-*`.
5. Client: `check_invite { text: link }` and `submit_invite { text: link, acceptNewCertificate: false, carryAccount:
   false }` both `ok`; `client-register-*`; `register { username: "Slayer_42" }` is `ok` with that username;
   `dismiss_backup_offer`; the snapshot's account is `Slayer_42` with `hasKey`, and `status.online` with the server's
   name. Screenshots for views `play`, `realms`, `server`, `game`, `settings` as `client-<view>-*`, then back to
   `play`.
6. Manager: `admin_action { request: { action: "ban", player: "Slayer_42", reason: "Desktop E2E ban check", until:
   null, module: null } }` is `ok` with `result.player.banned`. On tab 3, fill `#admin-player` with `Slayer_42` and
   click `Look up`; `server-gui-admin-*`. Client: `refresh_status`, then the snapshot's `lastError.code` is
   `account_banned`; `client-banned-*`.
7. Manager: `history_query` with an all-null filter (`account`, `actor`, `since`, `until`, `cursor`) returns entries;
   tab 4, `server-gui-history-*`; `server_backup` is `ok`; `host_server_stop` is `ok` and the state is then `stopped`.
8. Close both apps.

Whatever happens, `finally` closes both apps, uninstalls both bundles and removes the temporary root, each step
ignoring its own failure. Exit 1 on any failure, after printing it.

### 4.4 `e2e/remote.ts`

Drives the manager's "Managed installs" screen against the Docker remote-management stack that restricted's
`docker/remote/test.sh` leaves running (Compose project `reforged-remote`, profile `remote`, file `compose.yaml` in the
restricted checkout).

1. A temporary root `dl-e2e-remote-`; the manager's workspace is `profile-server/User Data/Server`.
2. The manager exe: `<restricted>/target/release/dainty-lass-manager.exe` when it exists, otherwise install the
   `DaintyLass Manager` bundle (§4.1) into the temporary root.
3. Copy the controller's workspace: first run `reforged-host remote sync` in service `host-alpha` (so alpha's last
   contact is inside the 60 s online window), then read from service `host-controller` with `cat` through
   `docker compose exec -T`: `config/host.toml`, `secrets.env`, `remote-controller.json` and `tls/gateway-cert.pem`,
   `gateway-key.pem`, `node-cert.pem`, `node-key.pem`, `cluster-ca-cert.pem`, `remote-cluster-ca.pem`, all under
   `/opt/reforged/.workspace/`, into the same paths under the profile's workspace `.workspace/`. The files hold keys:
   they stay in the temporary root and are removed with it.
4. Launch the manager on CDP port 9325 with a 1280×920 viewport. Wait for `app.serverGui`. `remote_status` is `ok`
   with role `controller`, revision 3, two installs: `alpha` `online` at applied revision 3, and `bravo` revoked
   with state `revoked`. `remote_invite { request: { node: "charlie", region: "ap-east", label: "Install Charlie",
   url: "https://172.28.77.20" } }` is `ok` and its `invite.uri` starts with `dainty-lass-remote://enroll?`.
5. Tab 0; fill `#rem-inv-node` `delta`, `#rem-inv-region` `ap-east`, `#rem-inv-label` `Install Delta`; scroll the
   managed-installs panel into view; `server-gui-managed-*`.

`finally` as in §4.3.

---

## 5. What dainty-restricted does (the integration pass, outside the clean room)

Not for the clean-room implementer. Recorded so the two halves meet:

- Delete `launcher/isolation/hook.js`, `index.html`, `scripts/`, `test/` except `test/electron-keystore/` and
  `test/cases/` (minus `manifest.json`), `package.json`, `package-lock.json`, `tsconfig.json`.
- Tauri's `isolation.dir` needs the hook, the page and `commands.js` in one folder at compile time. Restricted keeps
  generating `isolation/commands.js` and gets `hook.js` and `index.html` from the dainty-lass checkout at the rev its
  `Cargo.toml` pins (the folder CI already clones as `.dainty-lass`), through a small sync step with a staleness test,
  as for the other generated files. They stay MIT in restricted.
- `beforeBuildCommand`/`beforeDevCommand` call `bun <dainty-lass>/launcher/scripts/dashboard-page.ts ...`; the release
  workflow calls `release-config.ts` with `--tauri-dir`.
- `bundles.test.ts` becomes a Rust test in the launcher crate: the same checks on the two configs, the icons and the
  features, without JavaScript.

## 6. Acceptance

- `bun install --frozen-lockfile`, `bun run typecheck` and `bun run test` pass in `launcher/` with a restricted
  checkout beside dainty-lass.
- Every new file carries the MIT SPDX line. `package.json` has no AGPL dependency.
- Similarity by `clean-room/similarity.md`'s measure, against restricted `launcher/` at `4efc8b5` and against DR's
  `legacy/UndauntedLauncher` at `e1c5fec`: below 30% per Category B file, measured by a reviewer outside the clean room.
- The e2e scripts type-check. They are run on the maintainer's PC, not in CI: when only the type check passed, say so.
