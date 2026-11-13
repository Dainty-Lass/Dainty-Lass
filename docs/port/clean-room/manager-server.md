<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-room provenance record: the manager's embedded server (`crates/dainty-manager-server`)

## 1. Specification

- **Spec**: [`docs/port/spec-tauri-shell.md`](../spec-tauri-shell.md), commit `6fb038c1`, read against `plan` at
  `d80cff53`. Catalogs: [`spec-launcher-catalogs.md`](../spec-launcher-catalogs.md) (C-1).
- **Spec writer**: lane C's spec session, which read `restricted/launcher/src-tauri` and wrote no code.
- **Replaced code**: restricted's `src/server_gui.rs`, `src/server_gui/{engine,modules}.rs`,
  `src/server_gui/modules/tests.rs` and `src/server_history.rs` (1,600 lines). The spec classes all of them
  Category **A** (born in Dainty Lass; DR similarity 12.8%, 6.1%, 2.6%; `engine.rs` the A tail of RM4's
  `controller/host.rs`), so they could have moved as they were (T-1). The lane rewrote them anyway.

## 2. Implementer isolation

- **Reviewer**: lane C's review session (this record), which read both sides. Its only MIT changes are section 5.1's,
  which restricted's code did not have.
- **Implementer**: lane C's implementation session, working from the spec and the root's MIT crates, told not to
  open `restricted/`. Its commit says "from the spec alone".
- **Isolation was by instruction.** `restricted/` was in the implementer's checkout; no sparse checkout or removal
  kept it out. For Category A code that is enough (nothing in it is DR's or Paradox's), and the similarity below
  shows no copying either way.

## 3. Commits

- Spec: `6fb038c1`. Implementation: `a602f00f` (`RD27: the manager's embedded server as MIT code at the root (lane
  C)`), merged into `plan` as `0e64c8fb`. This review, the oracle and the catalog retirement:
  `feat/lane-c-review-cutover`.

## 4. Similarity

The F6 measure ([`similarity.md`](similarity.md) §1: code lines with blanks and comments stripped, a new line counted
when it sits in a run of three or more identical lines anywhere in the old file), each new file against the
restricted file it replaces at `a9bb2a57`:

| New file | Old file | Code lines | In 3+ runs | Similarity | Longest run |
| --- | --- | ---: | ---: | ---: | ---: |
| `src/lib.rs` | `server_gui.rs` | 189 | 56 | 29.6% | 11 |
| `src/engine.rs` | `server_gui/engine.rs` | 600 | 112 | 18.7% | 10 |
| `src/modules.rs` | `server_gui/modules.rs` | 410 | 107 | 26.1% | 10 |
| `src/history.rs` | `server_history.rs` | 167 | 23 | 13.8% | 6 |
| `tests/modules.rs` | `server_gui/modules/tests.rs` | 205 | 44 | 21.5% | 12 |

Every run of six or more lines is one the shared API dictates, written the way `rustfmt` lays it out:

- the one-to-one enum maps (`ComponentPhase` → `HostComponentPhase`, `ModuleState` → `ModuleRunState`), the
  `HostServerComponent { name, phase, port, detail }` and `Listed { … }` struct literals, field for field from the
  kit's types;
- the `use dainty_launcher_kit::bridge::{checks, types}::{…}` import lists (the same names, sorted);
- the `make-cert` argument vector (§5.1 gives it token by token) and `RemoteExecArgs`'s literal (§5.7 gives every
  field);
- the `ModulePanel::admin` signature (the kit's trait);
- the stub-module registry fixture in the tests (§12.2 asks for that registry; `ModuleRegistry::new` and
  `ModuleEntry` take nothing else);
- the `init_telemetry` prologue (three `use … as _` lines for `tracing_subscriber`'s traits and `unbounded_channel`).

The structure around them differs: the state is one `Arc<Inner>` rather than five shared slots, the module panel owns
its slots (`ServerModules::{attach_registry, set_workspace}`), the flag reader is a table (`Flags::read` with switch,
value and optional-value lists) rather than an iterator with a predicate, `history::{query, dry_run, apply}` return
`Result`s that `Engine` turns into outcomes, and the admin flow is split into `admin_target`, `ask` and
`player_view`. Provenance grep over the crate: no Freeza, Mystic Paradox, Reforged or DR name; the one hit,
`setup::paradox::Layout`, is the MIT host kit's own type name.

**Verdict: no code was copied.** The rewrite follows the spec, and where it reads like the old code the shared API
leaves no other way to write it.

## 5. Behaviour differences

Found by reading the two side by side and by running restricted's oracle (§6) against the new crate. None of them
reaches the player; the page sees 1, 2 and 6 at most.

| # | Where | Restricted | MIT crate | Matters? |
| --- | --- | --- | --- | --- |
| 1 | `admin_action` messages (§6, T-4) | `No player matching '<p>' was found.`, `Found player <n>.`, `Player banned and active sessions revoked.` … | `No player matched <p>.`, `Found <n>.`, `<n> is banned.` … | No: T-4 asked for fresh English, and the page does not call `admin_action` yet |
| 2 | Module views when `host.toml` cannot be read (§6) | `list`, `get`, `load` and `unload` refuse `ModuleFailed` `"host_toml"` and log a warning | They answer with no saved settings; only `set_settings` refuses | Small: a broken `host.toml` stops the server starting anyway. The spec's §6 bullet reads like restricted's. **Fix or accept** |
| 3 | History, a bad filter on a workspace without `admin.env` (§7) | `HistoryBadFilter` (the filter is read first) | `HistoryUnavailable` (the database is opened first) | No: both refuse, the page shows either code |
| 4 | `history_revert_apply`'s `conflicts` and `conflict_count` (§7) | From the whole fresh plan (`applied.plan`), so a database with conflicts and no actions is counted | From the databases that changed (`applied.done`); one with only conflicts is not counted | Small: the dry run shows those conflicts first; after the apply the count can be lower. The spec does not say which. **Fix or accept** |
| 5 | Timeout's default `until` (§6) | `to_rfc3339()`: nanoseconds, `+00:00` | Whole seconds, `Z` | No: both RFC 3339, and the module parses either |
| 6 | Module panel log lines (§6) | The kit's `log::info`/`warn` with a `host: ` prefix, into the launcher's log only | `tracing` events with `service = "host"`: the launcher's log, and through `ChannelLogLayer` the page's server lines as `host           \| module stub loaded …` | Arguably better: the host sees what the panel did. The admin line's wording also differs |
| 7 | `migrate_mongo` (§5.3) | Ignores `db migrate`'s and `db verify`'s exit codes; only an `Err` fails | A non-zero exit fails the step with `"<step> failed; the log has the details"` | Better: the page no longer reports a failed migration as done. The text is English, as every engine error is |
| 8 | `remote_status`'s counts (§5.7) | A JSON array where a number was expected reads as 0 | An array is counted (`pending_enrollments`) | No in practice: `status_json` writes numbers |
| 9 | `remote_exec`'s view (§5.7) | Missing `node` and `action` are `""` | They fall back to the request's | No |
| 10 | `source_commit` in setup (§5.1) | A release manifest that cannot be read fails setup | Reads as `""` | No: setup records an empty commit, as with no manifest |
| 11 | `run_kit` and `backup` | Rely on the hooks being set elsewhere | Call `dainty_host::ensure_hooks()` first | No: idempotent; it makes the engine work before the server ever started |
| 12 | The log forwarder (§4.1) | Started by `start()` itself | Started when `start()`'s future first runs | No: the kit awaits the future at once |
| 13 | `Flags::read` (§5.1) | A value flag given twice, the second at the end with no value, keeps the first value | The last occurrence wins, so the value is `None` | No: the kit never sends that |
| 14 | A manifest's player fallback (§6) | `global_id`/`module_user_id` fall back to the player text cut to 64 | The player text as given | No: the kit caps the input |

Items 2 and 4 are the ones worth a decision; the others are kept as they are.

### 5.1 Found while finishing, and changed on this branch

These are not differences from restricted: both builds had them, or lane A's change made them after the spec was
written. They needed no knowledge of restricted's code, so the reviewer fixed them in MIT files.

- **The manager had no version data.** Lane A stopped compiling the 1.14.7 version data into `dainty-host`; the
  host reads it from the workspace (`data/1.14.7/version_data.json`, or host.toml's `version_data`). Restricted's
  manager and the MIT one both failed `host_server_start` with "the 1.14.7 version data is missing" (the oracle's
  end-to-end case showed it). `dainty-manager-server::version_data` now copies the file the bundle ships (beside the
  executable on Windows, Tauri's `../lib/Dainty Lass Manager/` on Linux) into the workspace before `start`, `run_kit`
  and `migrate_mongo`, unless host.toml names its own; a newer manager's copy replaces an older one. The release ships
  the file as a bundle resource (section 8).
- **`tauri build` from the app's folder could not build the page.** The CLI runs `beforeBuildCommand` and
  `beforeDevCommand` from the folder above the tauri dir (`crates/`), so the app's `bun scripts/page.ts …` (the spec's
  §9 value, copied from restricted's relative to `restricted/launcher`) found nothing. They are
  `bun dainty-launcher-app/scripts/page.ts …` now, and `tests/bundles.rs` checks the new text.

### 5.2 Follow-ups (not changed here)

- **History while the server runs.** Fixed (M-H, `fix/admin-history-lock`). Querying the history twice while the
  embedded server ran once answered `HistoryFailed` `"database: database is locked"`, and the runs logged tokio's
  "Cannot drop a runtime in a context where blocking is not allowed" from worker threads. They were two bugs:
  - *The lock:* every query attaches each database (`Database::attach`), and attaching runs `prepare` in a
    deferred transaction that reads the catalog before it writes. A service's write in between turned its first
    write into an immediate `SQLITE_BUSY`, which the busy timeout does not cover. `prepare` now begins
    `BEGIN IMMEDIATE` on SQLite (`dainty-store`, `write_transaction`), so it waits for the lock like any write.
  - *The panic:* not from history. A compiled-in service's run (`dainty-controller`'s `run_once`), cancelled
    before it closed the service, here by the test's runtime shutting down after the failed query, dropped
    `dainty_service_plugin::InProcessCloser` and the service's own `Runtime` on a worker thread. An unclosed
    closer now lets its runtime go with `shutdown_background`.

  `crates/dainty-admin/tests/history_live.rs` queries in a row and concurrently while two services write, and
  `dainty-service-plugin` drops an unclosed closer on a worker. The oracle keeps the one query restricted's test
  made; with three more in a row and four concurrent ones it passed with both fixes and failed with both
  messages without them.
- Items 2 and 4 above, if the maintainer wants restricted's behaviour back.

## 6. Oracle

Restricted's package (`restricted/launcher/src-tauri`, `dainty-launcher`) now links the MIT crate instead of its own
copy, and its tests run against it with `--features server-gui`:

- `tests/controller.rs`'s two `server-gui` cases (spec §12.1) take `dainty_manager_server::EmbeddedServerState` and
  `dainty_manager_server::history::query`. The end-to-end case puts the version data beside the test binary, as the
  bundle puts it beside the manager (section 5.1), and now also checks the module panel listing the running
  server's module as `Ready`, and the log forwarding of §4.1 (a `host` and a `gateway` entry reach the page as
  `<service> | <line>`, a launcher entry does not).
- `tests/manager_server.rs` is restricted's old module-panel and history tests, run unchanged in substance against
  `dainty_manager_server::{ServerModules, history}`.
- `src/server_gui*`, `src/server_history.rs` and `src/strings.rs` are deleted; `src/lib.rs` hands the app the MIT
  server with today's icons. `tests/bundles.rs` stays until the release builds from the root (spec §11.3).

## 7. Catalogs (C-1 A)

`restricted/ui/` (the two catalogs, 1,186 keys each), `launcher/src-tauri/gen/strings.json` and its test-only
generator `strings.rs` are deleted: no page or binary read them (the page uses `ui/apps/web/messages/`, the backend
the kit's 15-key MIT catalog). `restricted/docker/Dockerfile` no longer copies `restricted/ui/`.

## 8. Release cutover (spec §11.3)

Prepared for the integration pass, not committed here (the workflows, `launcher/package.json` and `bun.lock` are
shared files): both forges' `launcher-release.yml` build from `crates/dainty-launcher-app` (`bunx tauri build`, the
launcher `--features realms`, the manager `--no-default-features --features server-gui`), take the version from
`launcher/package.json`, run `bun launcher/scripts/release-config.ts <dir> --tauri-dir crates/dainty-launcher-app`,
read the feeds' endpoints from the app's `tauri.conf.json`, and give the manager a second `--config` that adds
`restricted/modules/rustic-isles/version-data/1.14.7/version_data.json` as the resource
`data/1.14.7/version_data.json`. `ci.yml`'s `installers-windows` job does the same unsigned; its path filters drop
`restricted/ui/**`. `launcher/package.json` gains `@tauri-apps/cli` 2.12.4.

Once that lands, restricted's package can go test-only as §12.1 says (no `[[bin]] dainty-launcher`, configs, icons,
`build.rs`, `package.json` or `scripts/lass.ts`; `tests/bundles.rs` retired), which this branch leaves for after
the cutover so the release keeps building in between.

## 9. Verification

See the commit body for the commands and results.
