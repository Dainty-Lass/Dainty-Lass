<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# RD27 launcher backend: the rewrite plan (rows RL1–RL14)

The Rust side of the launcher, `restricted/launcher/src-tauri` (about 30,000 lines, the `dainty-launcher` and
`dainty-manager` binaries), is the largest code left in `restricted/` that can become MIT. Its behaviour is specified
in [`spec-rd27-launcher.md`](spec-rd27-launcher.md) (the "spec" below): 58 Category B files (about 21,000 lines with
tests) to rewrite clean-room, and 51 Category A files to move. The JavaScript half is done already
(`launcher/`, [`clean-room/launcher-js.md`](clean-room/launcher-js.md)), and so is DL4b's part of `crates/launcher-kit`.

This page orders that work into rows. It changes no decision; the integration pass adds the rows to
`MIGRATION_PLAN.md`.

## 1. What has changed since the spec

The spec was written before the repository split was undone (sources `849f45e` / `92e797a`, two repositories). Four
of its conclusions are now out of date, and they make the target larger:

| Spec says | Now |
| --- | --- |
| `controller/realms.rs` links `rifts-host` and `rifts-seam` (AGPL), so the client with `realms` stays AGPL (§3.6, §4.4) | Both are MIT crates at the root (`crates/rifts-host`, `crates/rifts-seam`). The realm adapter (A) moves to MIT, and the release client can be MIT |
| `server_gui.rs` links `crates/host`, `isles-1-14-7` and realms; the manager stays AGPL until RD27 phase 2 (§3.6, §4.5) | `restricted/crates/host` links only MIT crates now (gateway, content, chat, `rustic-isles-*`, `hostkit`). Its one restricted input is `version_data.json` (Paradox data, `version_data.rs`). Once the host shell moves to the root (separate row, §5), the manager can be MIT too |
| "dainty-restricted" and "dainty-lass" | `restricted/` and the root of this repository |
| The isolation hook and the TS tests are B (§2.3.7, §2.9) | Done: `launcher/` at the root is MIT |

So after these rows, the only parts of the launcher left in `restricted/` are the text and data that can't be
relicensed (spec §3): the page's language catalogs, DR's recorded test tables
(`test/cases/{game-config,hostapi,log,settings}.json`), the Electron key-store harness and today's icons (L2).

## 2. Decisions (maintainer, 2026-11-06)

| # | Question | Spec | Decision |
| --- | --- | --- | --- |
| L1 | The backend's strings (12 when asked; the tray made them 14) | §3.1 | **Rewrite them**, en and pt-BR, compiled into the MIT backend from a new MIT catalog. The close dialog's, the pickers' and the Epic window's wording changes slightly. The page's 1,171 other keys stay in `restricted/` as they are |
| L2 | The 15 icons | §3.5 | **Origin unknown: treat them as restricted.** They stay in `restricted/`, and new icons are drawn before an MIT client ships |
| L3 | The key backup file's two advisory sentences | §3.3 | **Write new ones** |
| L4 | Comparing the generator's output against today's `types.ts`, `commands.js` and `app-commands.toml` | §3.4 | **The implementer compares, ignoring doc comments** (amended 2026-11-06): `types.ts` carries ts-rs copies of the Rust types' rustdoc, much of it restricted's prose, so the comparison is on a normalised form without doc comments, banners or the permission set's description. The normalised files are `crates/launcher-kit/tests/fixtures/bridge/` (its README has the rule); the MIT types get their own doc comments |
| L5 | DR's recorded test tables | §3.2 | **Keep them in `restricted/`, running against the MIT crates, and also write new MIT cases** from the spec, so the root has its own tables |

## 3. How each row runs

- **Clean room.** A Category B implementer works in a worktree where `restricted/` is excluded by sparse checkout
  (`/*`, `!/restricted/`), as DL4b did ([`clean-room/launcher-kit.md`](clean-room/launcher-kit.md) §1). They read
  the spec, this page and the MIT crates, and never `git show` a restricted path. The three generated files are
  copied into RL8's brief as fixtures (L4). A row that only moves Category A files may read `restricted/`.
- **Where the code goes.** The B groups go into `crates/launcher-kit` behind its existing `host`, `server-gui` and
  `realms` features. The Tauri-facing parts go into a new MIT shell crate (RL12). Until RL13, `restricted/launcher/src-tauri`
  keeps building: each row ends by switching its module there to a re-export of the new launcher-kit module, which is
  how DL4b left nine files.
- **Checks per row.** Root: fmt, clippy and test. `restricted/`: `cargo test -p launcher` (with `-j 6`), with
  restricted's own tests running against the re-export. The recorded tables in `restricted/launcher/test/cases` are
  the oracle (spec §3.2). A row with an oracle also writes its own MIT table in `crates/launcher-kit/tests/cases/`,
  with cases and expected values taken from the spec, never from the restricted table (L5). A row that touches a
  fuzzed file moves its fuzz target to the root's `fuzz/`.
- **Review (RL14).** Someone outside the clean room measures each new file with
  [`clean-room/similarity.md`](clean-room/similarity.md)'s script. The baseline is `restricted/launcher` at the commit
  before RL2, and the threshold is spec §6: under 30% per file, no run longer than 6 lines outside fixed tables. The
  reviewer writes `clean-room/launcher.md`.
- **Seams.** New workspace dependencies (`tauri` and its plugins, `windows`, `dbus-secret-service`, `cbc`, `pbkdf2`,
  `ts-rs` at the root), a new workspace member, fuzz targets and the CI families are seam changes: each row reports
  them, and the integration pass makes them.

## 4. Rows

Rows in the same wave touch different files and can run at the same time. Sizes come from spec §5.

### Wave 1: the small leaves (launcher-kit only)

| Row | What | Spec | Size | Oracle |
| --- | --- | --- | --- | --- |
| ~~RL1~~ | **Spec refresh**, done 2026-11-06: the spec now has a refresh note, the tray, the Game log, the three new settings, command 54 `game_log_get`, the 14 backend keys, `RUSTIC_RIFTS_`, the `dainty-` names, and §3.6/§4 rewritten. No `RealmHost` trait: `rifts-host` is MIT, so the realm adapter moves as it is | all | S | — |
| RL2 | Requests: close the API gap between launcher-kit's `pinned`, `http`, `to_file` and the shell's callers; write `connector`; the Epic install job over `epic::download` | §2.6.1, §2.6.2, §2.6.6 | S | relay and runtime tests in restricted |
| RL3 | Platform leaves: bring `linux_runtime` and `system` to parity; rewrite `log` (redaction) and `paths` (including the legacy-folder migration) | §2.7.3, §2.7.4, §2.7.13, §2.7.14 | S | `log.json`; new MIT `log` table |
| RL4 | Core: `game_language`; `i18n` over a new MIT catalog of the backend's 14 keys in `en` and `pt-BR`, with new wording written from §2.8.2's description of each key (L1; the pt-BR is flagged for the maintainer); point every caller of the shell's `core/{invite,status,username,text,pins}` at `protocol` and launcher-kit | §2.8 | S | — |

### Wave 2: stored data and the network (after wave 1)

| Row | What | Spec | Size | Oracle |
| --- | --- | --- | --- | --- |
| RL5 | Key store: `keystore/{mod,format,os,legacy}` (DPAPI, Secret Service, `RRK2` and Electron framing, legacy decryption), with the backup file's two advisory sentences newly written (L3); move `keystore/epic.rs` with its test fake rewritten | §2.7.1 | M | Electron harness (stays in restricted) |
| RL6 | Settings, byte-exact (section 3 of the plan: stored data must not change) | §2.7.2 | M | `settings.json`; new MIT `settings` table |
| RL7 | Host API and its parsers | §2.6.3 | M | `hostapi.json`; new MIT `hostapi` table |
| RL8 | Bridge: `spec` (full command table with argument shapes), `types` (all wire types), `progress`, the argument checks, and the generator as a launcher-kit test producing byte-identical `types.ts`, `commands.js` and `app-commands.toml` | §2.3.1–2.3.6, §2.3.8 | M | the three generated files (L4) |

### Wave 3: the game side (after RL3, RL4)

| Row | What | Spec | Size | Oracle |
| --- | --- | --- | --- | --- |
| RL9 | Launch, game config, game folder; move `clipboard` (A) | §2.7.5–2.7.7 | M | `game-config.json`; new MIT `game-config` table |
| RL10 | Invite link and the single-instance channel, external links | §2.7.8, §2.7.9 | M | — |

### Wave 4: relay, then the state machine

| Row | What | Spec | Size | After |
| --- | --- | --- | --- | --- |
| RL11 | Relay and owner check (`relay`, `relay/local`, `relay_owner`, its table), with re-pointing (DN) and the netstat check; fuzz `relay_input` moves | §2.6.4, §2.6.5 | L | RL2, RL9 |
| RL12a | Controller: the `Platform` trait with no Tauri types, state and snapshot, init, join, connect and poll, account, PLAY, game, runtime, settings, stored view, shutdown; move the A modules (`install`, `loader`, `region`, `versions`, `realms_off`, `revived`, `host_off`, `realms.rs` linking `rifts-host` directly, its tests, `platform/game_console.rs`); rewrite `controller/tests.rs` (§2.10) | §2.4 | L | waves 1–3, RL11 |
| RL12b | Host page controller (`host.rs` up to line 1171, B), and moving its born-here tail, `guarded.rs`, `server_gui/modules.rs` and `server_history.rs` (A) into launcher-kit's `server-gui`; rewrite `fake_host_kit.rs` | §2.5, §4.3 | M | RL12a |

### Wave 5: the shell and the cutover

| Row | What | Spec | Size |
| --- | --- | --- | --- |
| RL13 | The MIT Tauri shell crate at the root (suggested `crates/launcher-app`; the name `launcher` is taken by restricted's package): `main`, `lib`, `window`, `tray` (A), `tests/bundles.rs` (A), `bridge::{commands, backend}`, the Tauri `Platform` impl, `art`, `dialogs`, `updater`, the Epic sign-in window, `build.rs`, rewritten `tauri.conf.json`, `capabilities/main.json` and `launcher.desktop`; `tauri.server-gui.conf.json` and `manager.desktop` move. The icons come from a folder the build is given: `restricted/` supplies today's until new ones exist (L2). Then `restricted/launcher/src-tauri` shrinks to a thin crate that adds the icons, the DR tables' tests and the Electron harness; `restricted/fuzz`'s `launcher_*` targets move to `fuzz/`. Report the seam lines: release workflows, `bun run restricted-data` sourcing the generated files from the root, and AGENTS.md's launcher-bridge section | §2.1, §2.2, §2.7.10–2.7.12, §4 | M |
| RL14 | Review outside the clean room: similarity per file, `cargo tree` showing no `restricted/` crate under the MIT shell, restricted's suites passing unchanged against the MIT crates, `bun run test` in `launcher/`; `clean-room/launcher.md` | §6 | S |

## 5. Outside these rows

- **The host shell to the root.** `restricted/crates/host` → MIT, with `version_data.json` read at run time or supplied
  by a small restricted crate. That frees the manager build (`server-gui`) from `restricted/`. Independent of RL2–RL12;
  it must land before RL13 if the manager should build at the root.
- **Hands-on (maintainer).** After RL13: the launcher against a real 1.14.7 client and the canary's gateway (already
  listed in `MIGRATION_PLAN.md`), and RA14's packaging from the new shell.
- **New icons (L2).** The launcher's and the manager's icon sets drawn anew (Windows `.ico`, the PNG sizes Tauri
  asks for, `brand/icon.png`) before an MIT client ships. Design work, not a row.
- **Not covered.** `restricted/` keeps its AGPL test data and the page's catalogs. Changing the release licence is
  DL5's call (the lawyer's read).
