<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-room review: the launcher backend (RL14)

- **Component**: `crates/launcher-kit` (rows RL2–RL12b) and `crates/launcher-app` (RL13), both `license = "MIT"`,
  written from [`spec-rd27-launcher.md`](../spec-rd27-launcher.md) under the plan
  [`plan-rd27-launcher.md`](../plan-rd27-launcher.md) (decisions L1–L8).
- **Reviewed at**: `feat/rd27-launcher-rewrite` at `1207ce3d`, on 2026-11-07, by a session outside the clean room
  (it read `restricted/` and the baselines' history).
- **Verdict**: one finding to fix before the MIT client ships (§2.3, `linux_runtime::natural_cmp`), and one
  acceptance item not met (§5: the fuzz targets don't build, root `runtime_input` and restricted's four launcher
  targets). The rest of §6 is met, with the judgement calls in §2.3 left for the maintainer.

## 1. Method

similarity.md's measure, unchanged: trim each line, drop blank and comment-only lines (`//`, `/*`, `*`, `*/`), and
count the lines of the new file that sit in a run of 3 or more consecutive lines found identically in the baseline;
similarity = matched / code lines. Two differences from the script printed there, both making the measure stricter:

- **Every baseline file at once**, as spec §1.1 measured DR: the new crates' files don't map one-to-one onto the
  old tree (`bridge/types.rs` became three files, the controller seven), so each new file is compared with every
  text file of the baseline tree. The files are joined with a unique separator line, so no run crosses two old files.
- **Text files of every kind**: `.rs`, `.toml`, `.json`, `.desktop`, `.ts`, `.js`, so the Tauri configs and the MIT
  case tables are measured too. Lock files and `node_modules` are left out.

Baselines:

| # | Tree | Commit | Text files | Code lines |
| --- | --- | --- | ---: | ---: |
| 1 | `restricted/launcher/` | `7bdc57fc` (the tip before RL2's merge) | 113 | 33,364 |
| 2 | `launcher/` (the old two-repository layout) | `849f45e` | 120 | 35,917 |
| 3 | DR's `legacy/UndauntedLauncher` | `e1c5fec` | — | — |

**Baseline 3 is not reachable.** `e1c5fec` is not an object in this repository (all 1,886 commits on every ref), no
ref or path names `UndauntedLauncher`, and the folder that held the side repositories (`R:\source\dauntless\`) is
gone from this machine. The archived Forgejo repositories (`legacy-revived` among them) were not fetched. Baselines
1 and 2 contain DR's lines wherever restricted kept them (spec §1.3 measured that share per file), so a run copied
from DR into a new file would also show against them, unless DaintyLass had already rewritten that line before
`7bdc57fc`. Measuring against DR itself is left to the maintainer (§7).

The script is [`measure-launcher.mjs`](#appendix-the-script) below; run from the repository root it prints the
table and writes every run to a JSON file.

## 2. Similarity

Categories follow spec §1.3: **B** is what a clean-room row wrote (the measured set); **DL4b** files are launcher-kit
files that DL4b's clean room wrote before RL2 (reviewed in [`launcher-kit.md`](launcher-kit.md); the RL rows touched
some of them); **A** files were moved by sessions outside the clean room and are expected to match (they are the
baseline's own born-here files); **G** files are generated.

### 2.1 Category B (the threshold applies)

| File | Code lines | vs 1 | Longest run | vs 2 | Longest run |
| --- | ---: | ---: | ---: | ---: | ---: |
| `launcher-app/Cargo.toml` | 46 | 15.2% | 4 | 6.5% | 3 |
| `launcher-app/build.rs` | 74 | 8.1% | 3 | 8.1% | 3 |
| `launcher-app/capabilities/main.json` | 16 | 93.8% | 12 | 93.8% | 12 |
| `launcher-app/linux/launcher.desktop` | 12 | 0.0% | 0 | 0.0% | 0 |
| `launcher-app/scripts/page.ts` | 8 | 0.0% | 0 | 0.0% | 0 |
| `launcher-app/src/art.rs` | 102 | 2.9% | 3 | 2.9% | 3 |
| `launcher-app/src/bridge.rs` | 2 | 0.0% | 0 | 0.0% | 0 |
| `launcher-app/src/bridge/backend.rs` | 61 | 21.3% | 6 | 21.3% | 6 |
| `launcher-app/src/bridge/commands.rs` | 614 | 5.9% | 3 | 5.9% | 3 |
| `launcher-app/src/dialogs.rs` | 185 | 13.0% | 5 | 13.0% | 5 |
| `launcher-app/src/lib.rs` | 555 | 8.5% | 7 | 8.5% | 7 |
| `launcher-app/src/main.rs` | 14 | 21.4% | 3 | 21.4% | 3 |
| `launcher-app/src/platform.rs` | 75 | 0.0% | 0 | 0.0% | 0 |
| `launcher-app/src/updater.rs` | 110 | 17.3% | 7 | 17.3% | 7 |
| `launcher-app/src/window.rs` | 231 | 16.9% | 8 | 16.9% | 8 |
| `launcher-app/tauri.conf.json` | 68 | 66.2% | 6 | 64.7% | 6 |
| `launcher-kit/Cargo.toml` | 95 | 30.5% | 9 | 21.1% | 5 |
| `launcher-kit/src/bridge.rs` | 7 | 57.1% | 4 | 57.1% | 4 |
| `launcher-kit/src/bridge/checks.rs` | 600 | 6.5% | 6 | 6.5% | 6 |
| `launcher-kit/src/bridge/generate.rs` | 374 | 13.6% | 7 | 13.6% | 7 |
| `launcher-kit/src/bridge/progress.rs` | 98 | 20.4% | 7 | 20.4% | 7 |
| `launcher-kit/src/bridge/spec.rs` | 687 | 20.5% | 76 | 20.1% | 64 |
| `launcher-kit/src/bridge/types.rs` | 838 | 9.7% | 7 | 9.7% | 7 |
| `launcher-kit/src/bridge/types/manager.rs` | 388 | 55.7% | 17 | 55.7% | 17 |
| `launcher-kit/src/bridge/types/player.rs` | 436 | 77.3% | 37 | 76.4% | 37 |
| `launcher-kit/src/controller.rs` | 696 | 9.9% | 6 | 9.9% | 6 |
| `launcher-kit/src/controller/account.rs` | 207 | 13.0% | 4 | 13.0% | 4 |
| `launcher-kit/src/controller/connect.rs` | 308 | 13.6% | 5 | 13.6% | 5 |
| `launcher-kit/src/controller/game.rs` | 252 | 15.1% | 8 | 15.1% | 8 |
| `launcher-kit/src/controller/host.rs` | 1075 | 10.2% | 6 | 10.2% | 6 |
| `launcher-kit/src/controller/join.rs` | 228 | 11.0% | 4 | 11.0% | 4 |
| `launcher-kit/src/controller/platform.rs` | 143 | 7.0% | 4 | 7.0% | 4 |
| `launcher-kit/src/controller/play.rs` | 315 | 9.8% | 4 | 9.5% | 3 |
| `launcher-kit/src/controller/runtime.rs` | 252 | 19.0% | 4 | 19.0% | 4 |
| `launcher-kit/src/controller/stored.rs` | 235 | 11.1% | 5 | 11.1% | 5 |
| `launcher-kit/src/core/game_language.rs` | 61 | 13.1% | 5 | 13.1% | 5 |
| `launcher-kit/src/core/i18n.rs` | 202 | 29.2% | 15 | 26.7% | 12 |
| `launcher-kit/src/core/i18n/catalog.rs` | 51 | 0.0% | 0 | 0.0% | 0 |
| `launcher-kit/src/net.rs` | 11 | 100.0% | 11 | 100.0% | 11 |
| `launcher-kit/src/net/connector.rs` | 258 | 22.5% | 5 | 22.5% | 5 |
| `launcher-kit/src/net/epic_install.rs` | 329 | 14.0% | 4 | 14.0% | 4 |
| `launcher-kit/src/net/hostapi.rs` | 169 | 11.8% | 6 | 11.8% | 6 |
| `launcher-kit/src/net/hostapi/parse.rs` | 358 | 14.0% | 5 | 14.0% | 5 |
| `launcher-kit/src/net/hostapi/tests.rs` | 321 | 10.6% | 5 | 10.6% | 5 |
| `launcher-kit/src/net/http.rs` | 279 | 22.9% | 12 | 22.9% | 12 |
| `launcher-kit/src/net/http/tests.rs` | 431 | 8.6% | 4 | 8.6% | 4 |
| `launcher-kit/src/net/http/to_file.rs` | 123 | 28.5% | 7 | 28.5% | 7 |
| `launcher-kit/src/net/pinned.rs` | 347 | 20.2% | 7 | 20.2% | 7 |
| `launcher-kit/src/net/relay.rs` | 684 | 12.3% | 10 | 12.3% | 10 |
| `launcher-kit/src/net/relay/local.rs` | 251 | 8.4% | 3 | 8.4% | 3 |
| `launcher-kit/src/net/relay/tests.rs` | 975 | 7.3% | 5 | 7.3% | 5 |
| `launcher-kit/src/net/relay_owner.rs` | 290 | 8.3% | 5 | 7.6% | 4 |
| `launcher-kit/src/net/relay_owner/table.rs` | 90 | 16.7% | 3 | 16.7% | 3 |
| `launcher-kit/src/net/runtime_files.rs` | 539 | 8.3% | 8 | 7.8% | 8 |
| `launcher-kit/src/net/runtime_files/tests.rs` | 549 | 4.6% | 4 | 4.6% | 4 |
| `launcher-kit/src/platform.rs` | 21 | 76.2% | 9 | 57.1% | 9 |
| `launcher-kit/src/platform/game_config.rs` | 572 | 11.4% | 10 | 11.4% | 10 |
| `launcher-kit/src/platform/game_folder.rs` | 555 | 12.1% | 5 | 12.1% | 5 |
| `launcher-kit/src/platform/invite_link.rs` | 167 | 8.4% | 5 | 8.4% | 5 |
| `launcher-kit/src/platform/invite_link/channel.rs` | 381 | 7.1% | 5 | 7.1% | 5 |
| `launcher-kit/src/platform/invite_link/schemes.rs` | 304 | 9.5% | 6 | 9.5% | 6 |
| `launcher-kit/src/platform/keystore.rs` | 352 | 16.5% | 4 | 16.5% | 4 |
| `launcher-kit/src/platform/keystore/format.rs` | 158 | 5.1% | 4 | 5.1% | 4 |
| `launcher-kit/src/platform/keystore/legacy.rs` | 165 | 6.7% | 5 | 5.5% | 3 |
| `launcher-kit/src/platform/keystore/os.rs` | 254 | 13.4% | 6 | 13.4% | 6 |
| `launcher-kit/src/platform/keystore/tests.rs` | 92 | 16.3% | 3 | 16.3% | 3 |
| `launcher-kit/src/platform/launch.rs` | 734 | 9.3% | 5 | 9.3% | 5 |
| `launcher-kit/src/platform/links.rs` | 322 | 3.4% | 5 | 3.4% | 5 |
| `launcher-kit/src/platform/linux_runtime.rs` | 769 | 12.9% | 15 | 12.9% | 15 |
| `launcher-kit/src/platform/log.rs` | 334 | 9.3% | 5 | 9.3% | 5 |
| `launcher-kit/src/platform/paths.rs` | 272 | 11.4% | 6 | 11.4% | 6 |
| `launcher-kit/src/platform/settings.rs` | 305 | 11.1% | 6 | 11.5% | 6 |
| `launcher-kit/src/platform/settings/tests.rs` | 171 | 15.2% | 5 | 15.2% | 5 |
| `launcher-kit/src/platform/system.rs` | 252 | 9.1% | 5 | 9.1% | 5 |
| `launcher-kit/tests/cases/game-config.json` | 263 | 1.1% | 3 | 1.1% | 3 |
| `launcher-kit/tests/cases/hostapi.json` | 236 | 0.0% | 0 | 0.0% | 0 |
| `launcher-kit/tests/cases/log.json` | 199 | 1.5% | 3 | 1.5% | 3 |
| `launcher-kit/tests/cases/settings.json` | 3966 | 27.7% | 6 | 28.0% | 6 |
| `launcher-kit/tests/controller.rs` | 809 | 5.9% | 5 | 5.9% | 5 |
| `launcher-kit/tests/fixtures/host/fake_host_kit.rs` | 56 | 32.1% | 7 | 32.1% | 7 |
| `launcher-kit/tests/game_config_cases.rs` | 115 | 12.2% | 4 | 12.2% | 4 |
| `launcher-kit/tests/host.rs` | 561 | 3.9% | 4 | 3.9% | 4 |
| `launcher-kit/tests/hostapi_cases.rs` | 137 | 8.8% | 3 | 8.8% | 3 |
| `launcher-kit/tests/keystore.rs` | 350 | 4.3% | 3 | 4.3% | 3 |
| `launcher-kit/tests/log_cases.rs` | 51 | 13.7% | 4 | 13.7% | 4 |
| `launcher-kit/tests/settings_cases.rs` | 53 | 13.2% | 4 | 13.2% | 4 |

### 2.2 DL4b, A and G (for the record; the threshold does not apply)

#### DL4b

| File | Code lines | vs 1 | Longest run | vs 2 | Longest run |
| --- | ---: | ---: | ---: | ---: | ---: |
| `launcher-kit/src/bridge/modules.rs` | 611 | 6.1% | 7 | 6.1% | 7 |
| `launcher-kit/src/core.rs` | 13 | 0.0% | 0 | 0.0% | 0 |
| `launcher-kit/src/core/pins.rs` | 60 | 20.0% | 9 | 20.0% | 9 |
| `launcher-kit/src/core/text.rs` | 170 | 6.5% | 5 | 6.5% | 5 |
| `launcher-kit/src/lib.rs` | 5 | 0.0% | 0 | 0.0% | 0 |
| `launcher-kit/src/net/regionping.rs` | 202 | 6.9% | 5 | 6.9% | 5 |
| `launcher-kit/src/net/regionping/harness.rs` | 44 | 0.0% | 0 | 0.0% | 0 |
| `launcher-kit/src/net/regionping/parse.rs` | 135 | 11.9% | 6 | 11.9% | 6 |
| `launcher-kit/src/net/regionping/tests.rs` | 418 | 12.4% | 15 | 12.4% | 15 |
| `launcher-kit/src/net/runtime_files/manifest.rs` | 3 | 0.0% | 0 | 0.0% | 0 |
| `launcher-kit/src/net/verify.rs` | 353 | 8.5% | 3 | 8.5% | 3 |
| `launcher-kit/src/net/verify/cache_file.rs` | 77 | 3.9% | 3 | 0.0% | 0 |
| `launcher-kit/src/net/versions.rs` | 304 | 10.5% | 4 | 10.5% | 4 |
| `launcher-kit/src/net/versions/builtin.rs` | 27 | 0.0% | 0 | 0.0% | 0 |
| `launcher-kit/src/net/versions/parse.rs` | 52 | 7.7% | 4 | 7.7% | 4 |
| `launcher-kit/src/net/versions/tests.rs` | 690 | 5.1% | 5 | 5.1% | 5 |
| `launcher-kit/src/platform/dialogs.rs` | 45 | 37.8% | 6 | 37.8% | 6 |
| `launcher-kit/src/platform/hostkit.rs` | 790 | 9.7% | 19 | 9.7% | 19 |
| `launcher-kit/src/platform/hostkit/parse.rs` | 458 | 7.2% | 4 | 7.2% | 4 |
| `launcher-kit/src/platform/hostserver.rs` | 848 | 10.3% | 6 | 10.3% | 6 |
| `launcher-kit/src/platform/loader_session.rs` | 407 | 7.1% | 4 | 7.1% | 4 |
| `launcher-kit/src/platform/requirements.rs` | 311 | 7.4% | 4 | 7.4% | 4 |

#### A (moved outside the clean room)

| File | Code lines | vs 1 | Longest run | vs 2 | Longest run |
| --- | ---: | ---: | ---: | ---: | ---: |
| `launcher-app/linux/manager.desktop` | 10 | 100.0% | 10 | 100.0% | 10 |
| `launcher-app/src/epic_login.rs` | 166 | 96.4% | 157 | 96.4% | 157 |
| `launcher-app/src/tray.rs` | 69 | 81.2% | 21 | 4.3% | 3 |
| `launcher-app/tauri.server-gui.conf.json` | 35 | 80.0% | 9 | 71.4% | 9 |
| `launcher-app/tests/bundles.rs` | 107 | 42.1% | 30 | 5.6% | 3 |
| `launcher-kit/src/controller/guarded.rs` | 103 | 43.7% | 7 | 43.7% | 7 |
| `launcher-kit/src/controller/host/manager.rs` | 276 | 8.7% | 3 | 8.7% | 3 |
| `launcher-kit/src/controller/host_off.rs` | 117 | 91.5% | 68 | 91.5% | 68 |
| `launcher-kit/src/controller/install.rs` | 341 | 83.9% | 51 | 83.9% | 51 |
| `launcher-kit/src/controller/loader.rs` | 88 | 76.1% | 10 | 76.1% | 10 |
| `launcher-kit/src/controller/realms.rs` | 1135 | 90.7% | 633 | 90.1% | 374 |
| `launcher-kit/src/controller/realms_off.rs` | 24 | 95.8% | 16 | 95.8% | 16 |
| `launcher-kit/src/controller/region.rs` | 18 | 77.8% | 6 | 77.8% | 6 |
| `launcher-kit/src/controller/revived.rs` | 13 | 76.9% | 10 | 76.9% | 10 |
| `launcher-kit/src/controller/versions.rs` | 45 | 40.0% | 6 | 40.0% | 6 |
| `launcher-kit/src/platform/clipboard.rs` | 108 | 99.1% | 99 | 99.1% | 99 |
| `launcher-kit/src/platform/epic_library.rs` | 146 | 99.3% | 100 | 99.3% | 100 |
| `launcher-kit/src/platform/epic_session.rs` | 339 | 99.1% | 173 | 99.1% | 173 |
| `launcher-kit/src/platform/game_console.rs` | 307 | 99.3% | 129 | 8.5% | 4 |
| `launcher-kit/src/platform/keystore/epic.rs` | 186 | 71.0% | 31 | 71.0% | 31 |
| `launcher-kit/src/platform/server_engine.rs` | 32 | 9.4% | 3 | 9.4% | 3 |
| `launcher-kit/tests/epic_session_log.rs` | 122 | 60.7% | 15 | 60.7% | 15 |
| `launcher-kit/tests/realms.rs` | 739 | 60.2% | 101 | 43.3% | 86 |

#### G (generated)

| File | Code lines | vs 1 | Longest run | vs 2 | Longest run |
| --- | ---: | ---: | ---: | ---: | ---: |
| `launcher-kit/generated/app-commands.toml` | 107 | 95.3% | 102 | 94.4% | 54 |
| `launcher-kit/generated/commands.js` | 103 | 100.0% | 103 | 98.1% | 54 |
| `launcher-kit/generated/types.ts` | 546 | 0.0% | 0 | 0.0% | 0 |
| `launcher-kit/tests/fixtures/bridge/app-commands.toml` | 104 | 98.1% | 102 | 97.1% | 54 |
| `launcher-kit/tests/fixtures/bridge/commands.js` | 103 | 100.0% | 103 | 98.1% | 54 |
| `launcher-kit/tests/fixtures/bridge/types.ts` | 147 | 0.0% | 0 | 0.0% | 0 |

### 2.3 Over a limit

Files at or over 30%, or with a run longer than 6 lines, in Category B, with each run over 6 lines and what it is.
"Excluded" means spec §6's exclusions cover it: import lists (module declarations and `use` lists count), the
constant tables §2 fixes, and match arms over the same enum.

**To fix: `platform/linux_runtime.rs`, `natural_cmp`** (lines 129–160; runs of 7 and 15 against restricted's
`platform/linux_runtime.rs` `natural`, lines 105–137). The whole function has the shape of the shell's: a pair of
`peekable` char iterators in a `loop` over `match (a.peek().copied(), b.peek().copied())` with the same three end
arms, a digit arm whose `take` closure collects digits and `trim_start_matches('0')`s them, the same `len` then
lexical `then_with` comparison, and a fallback arm doing `to_lowercase().cmp` then `a.next(); b.next();`. Only the
digit loop (`next_if` against `peek`/`next`) and three names differ. The spec says only "newest by a natural,
case-insensitive descending sort" (§2.7.13), and RL3's commit lists nothing that held this code (no MIT crate has a
natural sort of this shape), so this is either a striking convergence or a leak. Either way it doesn't meet §6.
**Rewrite the function in a fresh clean-room pass** from that one spec sentence (for example by splitting into
digit and non-digit chunks first), keeping `natural_order_is_numeric_and_case_insensitive`'s cases.

**Over a limit, and judged allowed by the exclusions (the maintainer may disagree):**

| File | Over | Runs over 6 lines | What they are |
| --- | --- | --- | --- |
| `bridge/types/player.rs` | 77.3%, run 37 | 26 runs, 7–37 lines | Wire structs and enums: field names, their order and the `serde` renames are the wire format, fixed by §2.3 and checked byte for byte by the L4 fixture (`types.ts`). No method bodies match |
| `bridge/types/manager.rs` | 55.7%, run 17 | 13 runs, 7–17 lines | Same, for the manager's wire types (guarded actions, history, revert, server config, invites, remote) |
| `bridge/types.rs` | run 7 | one: the end of `ActionResult`'s `Serialize` (`.serialize(serializer)`, `Failed { ok: false, error }`) and a derive line | Present in launcher-kit before RL2 (DL4b); 7 lines, mostly braces |
| `bridge/spec.rs` | runs 76, 64, 11, 10, 10 | the argument-kind enum, the length limits, the name lists (`SETTINGS_KEYS`, `EXTERNAL_TARGETS`, `HOST_ACTIONS`, `HOST_SERVER_COMPONENTS`, `HOST_FOLDERS`, `REMOTE_EXEC_ACTIONS`, ...) | The command table and the name lists §2.3.1 fixes (excluded by name) |
| `core/i18n.rs` | runs 15, 14 | the 14 key constants and the list of them | The backend's key names §2.8.2 fixes; the wording is L1's new catalog (`i18n/catalog.rs`, 0%) |
| `capabilities/main.json` | 93.8%, run 12 | the permission list | The Tauri permission identifiers §2.1.3 lists; 16 lines with no other content. `lib.rs` repeats 7 of them (its window capability) |
| `tauri.conf.json` | 66.2% | none over 6 | Config keys and values §2.1.3 fixes (`identifier`, the isolation pattern, the bundle targets); runs of up to 6 lines of keys and braces |
| `launcher-kit/Cargo.toml` | 30.5%, run 9 | the `windows` crate's feature names | A dependency list (import list) |
| `net.rs`, `platform.rs`, `bridge.rs` | 100%, 76.2%, 57.1% | module declarations | Import lists; 11, 21 and 7 lines |
| `platform/game_config.rs` | runs 10, 9, 10 | the `[ScalabilityGroups]` names, the windowed display lines (`ResolutionSizeX=1280` ...), and one test's `read_dir().filter(...).count()` of backups | The first two are config keys §2.7.6 fixes. The third (a test counting `before-display-repair` files, 10 lines with the `unwrap`s on their own lines) is a common chain, but it is a 10-line run outside the exclusions: worth a rewrite with the fix above |
| `net/http.rs` | runs 12, 10, 8, 7 | `RequestOptions` (struct and `get`), `pinned_target`, `host_header` | RL2 closed the API gap to the shell's callers (§2.6.1 names those functions and their rules); the struct's fields are the callers' API. `host_header` and `pinned_target` are 7 and 8 lines with the rule in one `if` |
| `net/http/to_file.rs`, `net/pinned.rs`, `net/runtime_files.rs` | runs 7, 7, 8 | `FileRequestOptions`'s fields; rustls's `verify_tls12_signature` signature; the `RuntimeManifestResult` enum | All present before RL2 (DL4b's); the second is the trait's own signature |
| `net/relay.rs` | run 10 | the end of `Relay::start`: `Ok(Self { shared, port, qos_port, tasks })` | A struct literal, 6 of the 10 lines braces and field names |
| `controller/game.rs` | run 8 | the `GameInfo { root, problem: None, ... }` literal | A wire struct's fields |
| `bridge/progress.rs` | run 7 | a test helper building `TaskProgress` | A wire struct's fields |
| `bridge/generate.rs` | run 7 | the `use` list | Import list |
| `window.rs` | run 8 | `navigation_allowed`'s scheme `match` (`"tauri"` → `localhost`, `"http" \| "https"` → `tauri.localhost`) | §2.2 states the rule in those terms; the 8 lines include 4 closing braces |
| `updater.rs` | run 7 | `} } } #[cfg(test)] mod tests { use super::*; #[test]` | Boilerplate |
| `tests/fixtures/host/fake_host_kit.rs` | 32.1%, run 7 | the argument dispatch (`skip(1)`, a `fail`/`deaf` mode word, then `match` on the command) | The stand-in's modes are §2.10's; 56 code lines, so 18 matched lines put it over 30%. Small enough to leave, or rewrite with the fix above |

**Known to check, `net::relay::local::is_qos_target`.** Its body is identical to the shell's `is_qos_path`:
`let bytes = target.as_bytes();` and `bytes.len() >= 4 && bytes[..4].eq_ignore_ascii_case(b"/qos") &&
matches!(bytes.get(4), None | Some(b'/' | b'?'))`. That is one run of 3 lines (with the closing brace), inside the
limits, and the spec gives the rule as the regex `^/qos(?:[/?]|$)` (§2.6.4), which this is the direct byte form of.
Under the threshold; recorded because it is token for token the same. Rewriting it costs one line if the maintainer
prefers.

Nothing else in Category B has a run over 6 lines or reaches 30%. The MIT case tables are 0–1.5% against
restricted's DR tables, except `tests/cases/settings.json` at 27.7% (longest run 6): a byte-exact settings file
shares its key names and default values with any other table of the same files.

## 3. Licence graph

`cargo tree --locked -e normal,build,dev`, from the root:

| Package | Default | `--all-features` | `--no-default-features` |
| --- | --- | --- | --- |
| `launcher-kit` | 311 lines, no `restricted/` path | 592, none | 311, none |
| `launcher-app` | 542, none | 740, none | 483, none |

Every path dependency reached with all features is a root crate with `license = "MIT"`: `admin`, `controller`, `epic`,
`history-ship`, `hostkit`, `launcher-kit`, `module-api`, `network`, `plugin-host`, `protocol`, `proxy`, `remote`,
`rifts-admission`, `rifts-host`, `rifts-relay`, `rifts-seam`, `rifts-transport`, `service-kit`, `service-plugin`,
`store`. In `cargo metadata` for the root no package is GPL-only (the one GPL mention is `r-efi`, `MIT OR
Apache-2.0 OR LGPL-2.1-or-later`). `cargo deny check licenses` (0.20.2) from the root: **licenses ok**.
`cargo build -p launcher-app --no-default-features` (the client without `realms`): **builds** (`Finished`, 32 s on
top of the test build).

**SPDX headers.** Every `.rs` file, both `Cargo.toml`s, `build.rs` and `scripts/page.ts` start with
`// Copyright (c) 2026 Silent Spud` and `// SPDX-License-Identifier: MIT` (or the `#` form). Without one: the JSON
files (`capabilities/main.json`, the two Tauri configs, the four `tests/cases/*.json`), which can't carry a comment;
the generated files and the L4 fixtures (`generated/*`, `tests/fixtures/bridge/*`), which must match their generator;
the icons; and four files that could take a header: `linux/launcher.desktop` and `linux/manager.desktop` (a
leading `#` line, which the desktop template keeps), `.gitignore`, and `tests/fixtures/bridge/README.md`. Other
root crates carry none in those kinds of file either, so this is a gap only if the maintainer wants one everywhere.

## 4. Provenance

Every clean-room row worked in its own worktree with `restricted/` excluded by sparse checkout and recorded the
listing and what it read in its commit body. The listing was the same for every row:

```text
/*
!/restricted/
```

| Row | Commit | Listing | Read (from the commit body) |
| --- | --- | --- | --- |
| RL4 | `7dc61af2` | as above; "restricted/ was absent from the working tree throughout" | AGENTS.md; the plan (§1–§4, RL4); the spec (header, refresh note, §2.8, §2.10, §3.1); launcher-kit (`Cargo.toml`, `lib.rs`, `core.rs`, `core/text.rs`, `core/pins.rs`); protocol's module list; root `Cargo.toml`, `clippy.toml` |
| RL2 | `3a4efac6` | as above | AGENTS.md, `ui/AGENTS.md`; the plan (§1–§4); the spec (refresh note, summary, §2.3.5, §2.3.8's `TaskProgress`, §2.6.1, §2.6.2, §2.6.6); launcher-kit `net` and callers; `epic` (lib, download API, testing, a test's setup); protocol `invite.rs`; `ui/packages/bridge/src/mock.ts` |
| RL3 | `7b7c3323` | as above | AGENTS.md; the plan; the spec (refresh note, §2.1.2, §2.4.7, §2.4.10, §2.7.3–§2.7.5, §2.7.13, §2.7.14, §2.10, §3.1, §3.2); `clean-room/launcher-js.md` (one grep hit); root `Cargo.toml`, `rust-toolchain.toml`; launcher-kit (`Cargo.toml`, `lib.rs`, `platform.rs`, `platform/{linux_runtime,system}.rs`); `epic/src/lib.rs`; `gateway/src/allowlist/windows.rs` |
| RL6 | `dcf224be` | as above | AGENTS.md; the plan (§1–§4); the spec (header, refresh note, §2.3.3, §2.3.8, §2.4.10, §2.4.11, §2.7.2, §2.7.3, §2.8.1, §2.8.2, §2.10, §6); launcher-kit (`platform.rs`, `core*`, `platform/paths.rs`, `tests/log_cases.rs`, `Cargo.toml`); protocol (`invite.rs`, `account.rs`); the RL3 hand-over note |
| RL5 | `74138bf4` | as above | AGENTS.md; the spec; the plan; `spec-dl4-launcher.md` §3.6; hand-over notes `rl2-api.md`, `rl3-handover.md`; launcher-kit (`platform::{log,paths}`, `Cargo.toml`); protocol (`account`, `invite`); `epic/src/auth.rs`; `gateway/src/allowlist/windows.rs`; `rifts-transport/src/seal.rs`; the `aes`, `cbc`, `cipher`, `pbkdf2`, `dbus-secret-service` and `windows` crate sources |
| RL7 | `e4b54219` | as above | AGENTS.md; the plan; the spec (refresh note, §2.3.8, §2.4.5, §2.4.6, §2.6.1–§2.6.3, §2.10, §4.3); `spec-dl4-launcher.md` (host API rows); notes `rl2-api.md`, `rl3-handover.md`; launcher-kit (`net::{http,pinned,hostapi,runtime_files,versions,regionping}`, `bridge::types`, `tests/log_cases.rs`); protocol (`account`, `status`, `text`, `manifest`, `invite`); content (`branding.rs`, `news.rs`, `input.rs`); `fuzz/Cargo.toml`, `fuzz_targets/launcher_region.rs` |
| RL8 | `fd7e1f1a` | as above | AGENTS.md; the spec (refresh note, §2.3); the plan; `clean-room/launcher-kit.md`; launcher-kit (`bridge`, `core`, `net`, `platform`); protocol; the L4 fixtures (README and the three files); the ts-rs 12 sources; the wave's hand-over notes |
| RL9 | `9b5569a0` | as above | AGENTS.md; the spec (refresh note, §0, §1.3, §2.4.7, §2.4.8, §2.7.5–§2.7.7, §2.7.13–§2.7.16, §2.8.1, §2.10, §3.2); the plan; launcher-kit (`core::{game_language,pins,text}`, `platform::{linux_runtime,system,paths,log}`, `bridge::types::player`, `tests/log_cases.rs`, `cases/log.json`, `Cargo.toml`); protocol `account` |
| RL10 | `6a71c872` | as above | AGENTS.md; the plan; the spec; protocol (`invite`, `text`, `lib`, `tests/cases/invite.json`); launcher-kit (`Cargo.toml`, `platform.rs`, `platform/{paths,system}.rs`, `bridge/{spec,types}.rs`, `keystore/os.rs`); root `Cargo.toml`; sibling hand-over notes |
| RL11 | `21226a2c` | as above; "a first `set` from Git Bash mangled the pattern; before it was corrected, `ls restricted` printed that folder's top-level file names, no contents" | AGENTS.md; the plan; the spec (refresh note, §2.6.1–§2.6.5, §2.4.7, §2.10, §4.3); `docs/security/gateway.md`; launcher-kit (`net::{connector,http,pinned}`, `platform::{launch,log,system}`, `Cargo.toml`); `proxy/src/lib.rs`; `gateway/src/network.rs`; root `Cargo.toml` |
| RL12a | `fc03b75b` | as above | AGENTS.md; the plan; the spec (refresh note, §1.3, §2.1–§2.10, §3.6, §4, §6); the RL2–RL11 hand-over notes; launcher-kit (`bridge`, `core`, `net`, `platform`); protocol (`invite`, `account`, `status`, `manifest`, `text`); `ui/packages/bridge/src/api.ts` |
| RL12b | `3e9685ac` | as above; the same Git Bash slip, top-level names only, redone from PowerShell | AGENTS.md; the plan (§1–§5); the spec (refresh note, §1.3, §2.3, §2.4, §2.5, §2.7.5, §2.10, §4); `spec-dl3-hostkit.md`; the RL7 and RL12a notes; launcher-kit's controller, `platform::{hostkit,hostserver,requirements,loader_session,launch}`, `bridge::types` and tests |
| RL13 | `ffda42ba` | as above; the same slip, top-level names once, no file opened | AGENTS.md; the plan; the spec (refresh note, §2.1–§2.5, §2.7.8–§2.7.16, §2.8, §3, §4, §6); sibling hand-over notes; launcher-kit (`controller`, `bridge`, `platform`, `core::i18n`, `net::relay`); `launcher/` (isolation, scripts, `package.json`); `ui/packages/bridge/src/api.ts`; the tauri, tauri-build, tauri-codegen, tauri-utils and the dialog and updater plugin sources |

Notes on the record:

- **The three listing slips** (RL11, RL12b, RL13) showed `restricted/`'s top-level names, which are public in the
  plan's own layout table; no file under it was opened.
- **Hand-over notes.** RL12b and RL13 read sibling notes written after `rl12a-moves-handover.md` (2026-11-06 23:29),
  the one note written outside the clean room. It lists the moved Category A modules' signatures and seams, no code
  and nothing from a B file; the A modules are MIT and in launcher-kit, which those rows read anyway.
- **Messages from outside.** The rows took follow-ups from the session that started them, all behaviour-only: rules
  the spec had left loose, found when restricted's ported suites ran against the MIT crates. Each also became a spec
  edit (the `RL1:` commits `cdfae590`, `2b8f53fb`, `dadf3471`, `3a51dc0e`, `3d36d08f`, `03856be3`), and the row's
  fix cites the spec (`a4bd036a`, `3d58e9e2`, `2db97a7b`, `d31a4096`). For example, RL11's second follow-up gave
  the rule as "**A blank re-point ticket counts as none.** A ticket that is empty once trimmed (`" "`) is no
  ticket", and RL11 answered with `d31a4096`. No message carried code or a restricted path.
- **Outside the clean room, by design**: the Category A moves (`1fc164d1` RL12a moves, `8ba7fe8b` RL13's tray, Epic
  sign-in window and manager bundle files), the cutover (`242ad6b6`, which also moved the host page's born-here tail
  into `controller/host/manager.rs` and `platform/server_engine.rs`), the generated files (`b879fbdd`), restricted's
  ported tests (`e0441678`, `042d8eed`, `0c62c829`, `541c910b` to `5ea6fc1e` and the "run again" commits), and the
  workspace dependencies (`bf5cd9c6`, `b2c8514a`). `platform/keystore/epic.rs`'s test fake was rewritten in the move,
  as spec §1.3 asked: its `Fake` (RR1 sealing under a fixed key) matches nothing; the file's matched runs are the A
  body and its A tests.

## 5. Acceptance (spec §6)

| §6 item | Run | Result |
| --- | --- | --- |
| The generator's test against `tests/fixtures/bridge/` (normalised, L4) | `cargo test -p launcher-kit -p launcher-app --locked --no-fail-fast` | **passed**: `generated_files_match_the_interface_fixtures` and `generated_files_are_current` with `RR_UPDATE_GENERATED` unset, and the four other `bridge::generate` tests. The whole run: **403 passed, 0 failed, 2 ignored** (15 binaries; the two ignored need the Docker network harness and Linux `pgrep`) |
| The new MIT tables (L5) pass at the root | same run (`log_cases`, `settings_cases`, `hostapi_cases`, `game_config_cases`) | **passed**: `game_config_cases`, `hostapi_cases`, `log_cases`, `settings_cases` |
| Root tests, all features | `cargo test -p launcher-kit -p launcher-app --all-features --locked --no-fail-fast` | **403 passed, 0 failed, 2 ignored**, once the manager page is built (`bun scripts/page.ts build server-manager` in `crates/launcher-app`). Without it, `launcher-app`'s `generate_context!` stops the build with `frontendDist` `dist/server-manager` missing (see below); `launcher-kit` alone with all features: 375 passed, 0 failed, 2 ignored |
| restricted's suites against the MIT crates | `cargo test -p launcher -j 6 --locked --no-fail-fast` in `restricted/` | **194 passed, 0 failed, 2 ignored** (21 test binaries plus `lib`) |
| same, with the manager and realms | `... --features server-gui,realms` | **206 passed, 0 failed, 2 ignored** |
| `bun run test` in `launcher/` | after `bun install --frozen-lockfile` | **165 passed, 0 failed** (502 `expect()` calls, 3 files) |
| The fuzz targets build | root `fuzz/`; `restricted/fuzz/` | **Not met.** Root `fuzz/` (`cargo check`, and `cargo +nightly fuzz build --dev`): `runtime_input` fails, E0433 in its `#[path]` copy of `launcher-kit/src/net/runtime_files/manifest.rs`, which RL7 made a re-export of `crate::net::hostapi::parse`. `restricted/fuzz/`: its four launcher targets name nine deleted files (below) |
| `cargo tree` shows no AGPL crate; the client builds without `realms` | §3 | met |
| No catalog text and no DR or Reforged identifier in comments of the new files | `git grep` of the comments | met, with one note below |

The two ignored tests in restricted are the record of L8 (`a_negative_changelist_is_kept`) and the Electron profile
check that needs a profile made by `test/electron-keystore` (`an_electron_profile_reads_back`, maintainer-run).
restricted's suites are DR's and Reforged's tests repointed at launcher-kit and launcher-app by RL13's cutover
(`541c910b` to `5ea6fc1e`), not the files as they were at `7bdc57fc`: the cases and expected values are restricted's,
and where a case's expected value changed the commit says why (the `fake-host-kit` stand-in's two fixed strings,
`5ea6fc1e`). "Unchanged" in §6 holds for the assertions, not for the imports.

**Not met: the root's `runtime_input`.** `fuzz/fuzz_targets/runtime_input.rs` compiles
`crates/launcher-kit/src/net/runtime_files/manifest.rs` in by `#[path]`. RL7 (`e4b54219`) made that file a
re-export, `pub use crate::net::hostapi::parse::parse_runtime_manifest;`, which can't resolve in the fuzz crate
(E0433). The target should take `net::hostapi::parse` itself, which is the `launcher_hostapi` target RL7 reported
as a seam. Reported, not fixed.

**Not met: restricted's launcher fuzz targets.** `restricted/fuzz`'s `launcher_files`, `launcher_input`,
`launcher_net` and `relay_input` compile the shell's sources in with `#[path = "../../launcher/src-tauri/src/..."]`,
and the cutover deleted all nine of those files (`core/{invite,status,text,username}.rs`, `net/hostapi/parse.rs`,
`net/relay/local.rs`, `net/relay_owner/table.rs`, `platform/keystore/format.rs`, `platform/settings.rs`). They don't
build. The plan has them moving to the root's `fuzz/` (RL13) over launcher-kit's modules; none has been added there
yet (the root has `launcher_host` and `launcher_region` only). Reported, not fixed. (Found by checking every
`#[path]` against the tree; `restricted/fuzz` was not compiled.)

**A build step the root needs.** `cargo test -p launcher-app --features server-gui` (and so `--all-features`, or
`cargo test --workspace --all-features`) fails until `crates/launcher-app/dist/server-manager` exists:
`generate_context!` stops with "The `frontendDist` configuration is set to `"dist/server-manager"` but this path
doesn't exist". The default build has no `dist/launcher` either and builds; why only the manager's path is checked
was not traced. CI's root job runs the default features, so it won't catch this; a host of the manager build
needs `bun scripts/page.ts build server-manager` first.

**Comments.** `bridge/types.rs` names two link targets by their projects ("Dauntless Revived's source code",
"Undaunted's source code"): the doc comments of `ExternalTarget::RevivedSource` and `UndauntedSource`, whose names
§2.7.9 fixes. That is a project name, not a DR identifier; left as it is. No comment names a DR row, a Reforged source file
or catalog text.

## 6. Spec gaps the rows decided

- **The keep-open dialog's buttons (RL13, `ffda42ba`).** The first button is "close anyway", so Esc and the close
  box keep the window open on every OS (rfd maps Cancel to the second label on Linux). "Keep" is therefore not the
  default button, which the spec's wording didn't settle.
- **The desktop entry's `Exec` takes `%u`** (RL13), and **a root-built client reports version 0.0.0**: the version
  comes from `launcher/package.json`, while restricted's launcher is `2.0.0-beta`, so a client built at the root
  would always see an update (noted at the cutover).
- **L8** (a negative changelist), decided by the maintainer during RL7.

## 7. For the maintainer before DL5 and DX5

1. **Rewrite `platform::linux_runtime::natural_cmp`** in a clean-room pass (§2.3). Optionally also the two small
   runs in `platform/game_config.rs`'s backup-count test and `fake_host_kit.rs`'s dispatch, and `is_qos_target`'s
   one line.
2. **Decide the wire-type runs** in `bridge/types/{player,manager}.rs` (77% and 56%): §2.3 treats a wire struct's
   field list as a table §2 fixes, since the L4 fixture checks it byte for byte, but §6 names only the command table,
   name lists, launch arguments and config keys.
3. **Measure against DR's `legacy/UndauntedLauncher` at `e1c5fec`**, which this machine no longer has: fetch the
   archived side repository (or a bundle) and rerun the script with it as a third baseline.
4. **Draw the icons (L2).** `crates/launcher-app/icons/` holds RL13's plain blue placeholder squares (5 files, 104 to
   1,820 bytes); restricted hands in today's icons, whose origin is unknown. An MIT client needs new ones.
5. **Confirm the keep-open dialog's button order** (§6) and the 0.0.0 version a root build reports.
6. **Add the root fuzz targets** `launcher_hostapi` (over `net::hostapi::parse`, which RL7 reported), `relay_input`
   (over `net::relay::local` and `net::relay_owner::table`) and `launcher_files` (over `platform::keystore::format`
   and `platform::settings`), and `launcher_input` (invites and statuses, now `protocol`'s), then retire or repoint
   restricted's four broken ones, and repoint the root's `runtime_input`, broken since RL7 (§5).
7. **The manager page before an all-features build** (§5): build it first, or find why the manager's `frontendDist` is
   checked when the launcher's isn't, so `cargo test --all-features` at the root works on a clean checkout.
8. **Optional headers** on `linux/*.desktop`, `.gitignore` and the fixtures' README (§3).

## Appendix: the script

`measure-launcher.mjs <repo> <out.json>`, Node 20 or later, from the repository root:

```javascript
// similarity.md's measure (3+-line runs of identical trimmed non-comment lines), with every baseline file at once:
// files are joined with unique sentinel lines so no run crosses a file boundary.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const repo = process.argv[2];
const out = process.argv[3];
const git = (...a) => execFileSync("git", ["-c", "core.fsmonitor=false", ...a], { cwd: repo, encoding: "utf8", maxBuffer: 1 << 28 });
const textExt = /\.(rs|toml|json|desktop|js|ts|mjs|html|md)$/;

function codeLines(text) {
  return text.split(/\r?\n/).map((l) => l.trim()).filter((l) => l.length > 0 && !l.startsWith("//") && !l.startsWith("/*") && !l.startsWith("*") && !l.startsWith("*/"));
}

function baseline(rev, dir) {
  const files = git("ls-tree", "-r", "--name-only", rev, dir).split("\n").filter((f) => textExt.test(f) && !f.includes("node_modules") && !f.endsWith("package-lock.json") && !f.endsWith("Cargo.lock"));
  const lines = [];
  const origin = [];
  for (const f of files) {
    const t = git("show", `${rev}:${f}`);
    for (const l of codeLines(t)) { lines.push(l); origin.push(f); }
    lines.push(`\u0000${f}`); origin.push(null);
  }
  const index = new Map();
  lines.forEach((l, j) => { if (!index.has(l)) index.set(l, []); index.get(l).push(j); });
  return { rev, dir, files: files.length, lines, origin, index };
}

function measure(base, newText) {
  const nl = codeLines(newText);
  const flags = new Array(nl.length).fill(false);
  const runs = [];
  for (let i = 0; i < nl.length; i++) {
    let best = 0, bestJ = -1;
    for (const j of base.index.get(nl[i]) || []) {
      let k = 0;
      while (i + k < nl.length && j + k < base.lines.length && nl[i + k] === base.lines[j + k]) k++;
      if (k > best) { best = k; bestJ = j; }
    }
    if (best >= 3) {
      for (let k = 0; k < best; k++) flags[i + k] = true;
      runs.push({ start: i, len: best, from: base.origin[bestJ], sample: nl.slice(i, i + best) });
      i += best - 1;
    }
  }
  const matched = flags.filter(Boolean).length;
  return { n: nl.length, matched, pct: nl.length ? (matched / nl.length) * 100 : 0, runs, maxRun: Math.max(0, ...runs.map((r) => r.len)) };
}

const bases = [baseline("7bdc57fc", "restricted/launcher"), baseline("849f45e", "launcher")];
const newFiles = git("ls-files", "crates/launcher-kit", "crates/launcher-app").split("\n").filter((f) => textExt.test(f));
const result = [];
for (const f of newFiles) {
  const t = fs.readFileSync(path.join(repo, f), "utf8");
  result.push({ file: f, by: bases.map((b) => measure(b, t)) });
}
fs.writeFileSync(out, JSON.stringify({ bases: bases.map((b) => ({ rev: b.rev, dir: b.dir, files: b.files, lines: b.lines.length })), result }, null, 1));
for (const r of result) console.log(`${r.file.padEnd(70)} n=${String(r.by[0].n).padStart(5)} ${r.by.map((m) => `${m.pct.toFixed(1).padStart(5)}% max${String(m.maxRun).padStart(3)}`).join(" | ")}`);
```
