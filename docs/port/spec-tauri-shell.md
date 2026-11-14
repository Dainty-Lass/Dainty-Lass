<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-room behaviour specification: the Tauri shell's AGPL half (RD27, port plan item 5)

This page specifies what is left of `restricted/launcher/src-tauri` (the package `dainty-launcher`, the AGPL half of
the two desktop apps) so the apps can be built from the root alone under MIT. The implementer works from this page
and the root's MIT crates, and never opens `restricted/`. The language catalogs and `gen/strings.json` are
[`spec-launcher-catalogs.md`](spec-launcher-catalogs.md).

Read against `plan` at `d80cff53` (2026-11-11). Quoted text in `code` is exact. "The kit" is
`crates/dainty-launcher-kit`, "the app" is `crates/dainty-launcher-app`, "the host" is `crates/dainty-host`, all MIT.
**Calls** (where this page had to choose or found an ambiguity) are marked **[T-n]** and collected in section 13.

---

## 0. What the survey found

The port plan's item 5 and the brief describe a shell of about 2k lines to rebuild, with `bridge/spec.rs` as its
contract. Most of that already moved in RL13 (see [`spec-rd27-launcher.md`](spec-rd27-launcher.md) §4):

- **The shell is MIT already.** `crates/dainty-launcher-app` (MIT) is the Tauri app: start-up, the main window, the
  close dialog, the tray, the Epic sign-in window, the pickers, the `rr-art:` scheme, the updater, the command
  handlers, the Tauri `Platform` and the build script that writes Tauri's app manifest. Both `main.rs` files call it.
- **The bridge is MIT already.** `bridge/spec.rs` lives in the kit (`crates/dainty-launcher-kit/src/bridge/spec.rs`),
  and its generator (`bridge/generate.rs`, the `generated` test) writes `crates/dainty-launcher-kit/generated/`
  (`types.ts`, `commands.js`, `app-commands.toml`). `launcher/scripts/restricted-data.ts` and
  `ui/packages/bridge/scripts/restricted-data.ts` copy from there, not from `restricted/`.
- **What restricted's package still holds:** the manager's embedded server (`src/server_gui.rs`,
  `src/server_gui/{engine,modules}.rs`, `src/server_history.rs`; 1,600 lines with their tests), the two bundler
  configs, today's icons, a 29-line `lib.rs` that hands the icons and the server to the app, a 17-line `main.rs`, an
  8-line `build.rs`, the `gen/strings.json` generator (test-only `strings.rs`) and the oracle tests.
- **Everything the embedded server links is MIT:** `dainty-host`, `dainty-controller` (`server-controller` in
  restricted's workspace), `dainty-admin`, `dainty-module-api`, `dainty-service-kit`, the kit. The only restricted
  input left on the manager's path is the 1.14.7 version data, compiled into `dainty-host` when
  `DAINTY_VERSION_DATA` names it (`restricted/.cargo/config.toml` sets it; the root's does not). Port plan item 1
  (lane A) reads it at run time instead.

So what this page specifies is the embedded server (sections 3–7), the shell's remaining build pieces (sections 8–10)
and the move (11–12). The rest of the shell is summarised in section 2 so the implementer knows what to leave alone.

**Provenance [T-1].** [`spec-rd27-launcher.md`](spec-rd27-launcher.md) §1.3 classifies `server_gui.rs`,
`server_gui/modules.rs` and `server_history.rs` as **A** (born in Dainty Lass; DR similarity 12.8%, 6.1%, 2.6%), and
`server_gui/engine.rs` is the A tail of the old `controller/host.rs` (B3, RM4). Category A files "move to MIT as they
are": the clean-room review would only record it. This page specifies them anyway, as
[`spec-legacy-import.md`](spec-legacy-import.md) did for its L-1, so either route works. A rewrite from this page is
the lane's default.

## 1. Scope and provenance

| Part today (`restricted/launcher/`) | Lines | Category | In the MIT build |
| --- | ---: | --- | --- |
| `src-tauri/src/server_gui.rs` | 186 | A | Section 4, in the new crate (section 11) |
| `src-tauri/src/server_gui/engine.rs` | 671 | A | Section 5 |
| `src-tauri/src/server_gui/modules.rs`, `modules/tests.rs` | 671 | A | Section 6, and the tests of section 12.2 |
| `src-tauri/src/server_history.rs` | 304 | A | Section 7 |
| `src-tauri/src/lib.rs`, `main.rs` | 46 | A | Section 8: the app's `main.rs` with `server-gui` |
| `src-tauri/build.rs` | 8 | A | Already the app's (`crates/dainty-launcher-app/build.rs`) |
| `src-tauri/src/strings.rs` (test only) | 78 | A | Retired with the catalogs ([`spec-launcher-catalogs.md`](spec-launcher-catalogs.md)) |
| `src-tauri/tauri.conf.json`, `tauri.server-gui.conf.json` | 134 | A | Section 9: the app's configs, with the differences settled |
| `src-tauri/icons/**`, `icons/server/**`, `brand/icon.png` | 15 files | Asset | L2: stay restricted; new icons drawn [T-2] |
| `src-tauri/Cargo.toml` features | — | A | Section 10 |
| `src-tauri/tests/**` | 9,771 | A, D | Section 12: stay as the oracle |
| `package.json`, `scripts/lass.ts` | 50 | A | Section 11.3: the release runs from the root |
| `test/cases/*.json` | — | D | Stay (L5) |
| `test/electron-keystore` | — | A | Stays (the keystore oracle) |

## 2. What the MIT shell already does (leave it alone)

The implementer reads these MIT files; this section only says where each concern lives so nothing is duplicated.

| Concern | Where | Spec |
| --- | --- | --- |
| Start-up order, single instance, invite links, schemes, storage bootstrap | the app's `lib.rs` (`run`, `Flavour`) | spec-rd27-launcher §2.1.2 |
| The window (`main`, title by flavour, close guard and dialog) | the app's `window.rs`, `dialogs.rs` | §2.2 |
| The tray (`tray_open`, `tray_quit`) | the app's `tray.rs` | §2.2.1 |
| The Epic sign-in window | the app's `epic_login.rs` | §2.4.7 |
| Pickers and their titles (the 15 backend keys) | the app's `dialogs.rs`, the kit's `core::i18n` | §2.7.11, §2.8.2 |
| Self-update (check, download, install on restart) | the app's `updater.rs` | §2.7.12 |
| The `rr-art:` image scheme | the app's `art.rs` | §2.7.10 |
| Commands, argument checks, outcomes, progress channel | the kit's `bridge::{spec,checks,types,progress}`; the app's `bridge/` | §2.3 |
| Tauri app manifest, isolation hook table | the app's `build.rs`, the kit's `generated/` | §2.1.4, §2.3.6 |
| The controller, the Host page jobs, the server panel, logs | the kit's `controller`, `platform::hostserver` | §2.4, §2.5 |

`Options` (the app's `lib.rs`) is the seam this page fills: `icon: Option<tauri::image::Image<'static>>` (the
window's and the tray's icon) and, with `server-gui`, `embedded_server: Option<Arc<dyn EmbeddedServer>>`. With no
embedded server the manager's Host page runs `dainty-host start` as a child, as the client's does; with one, the kit's
controller calls `set_embedded_server` and the server runs in the manager's process (RD26).

## 3. The bridge commands

The contract is the kit's `COMMANDS` table: 102 commands, each with its arguments, its `Group` and whether it is
answered on the calling thread. This page changes none of them. The generated files must come out byte-identical,
which they do as long as `spec.rs` is untouched: **acceptance** is `cargo test -p dainty-launcher-kit generated`
passing with `RR_UPDATE_GENERATED` unset and no diff under `crates/dainty-launcher-kit/generated/`, and the app's
`build.rs` producing the same `app-commands` permission set.

| Group | Count | Served by | Commands |
| --- | ---: | --- | --- |
| `Every` | 55 | the kit's controller | `get_snapshot`, the invite, account, game, runtime, task, settings, status, news, links, log, notice, Epic and `game_install_*` commands, `region_ping` |
| `Host` | 16 | the kit's Host page controller | `host_get` … `host_set_multipliers` |
| `ServerGui` | 27 | the kit, through the three traits below | `module_*` (6), `history_*` (3), `guarded_*` (3), `server_*` (7), `admin_action`, `remote_*` (7) |
| `Realms` | 4 | the kit's `controller::realms` | `realm_host`, `realm_join`, `realm_stop`, `realm_get` |

Every build registers every command; a build without the group answers `not_in_this_build` (the kit's
`bridge::checks`). In the manager, the commands that reach this page's code:

| Command(s) | Kit path | Implemented here by |
| --- | --- | --- |
| `host_server_start`, `host_server_stop`, `host_server_restart`, `host_server_get` | `HostServer` with an embedded server | `EmbeddedServer::{start, stop, restart, components}` (§4) |
| `host_setup`, `host_create_invite`, `host_update`, `host_set_multipliers`, `host_import_gamedata`, `host_action` (`synthetic_data`, `publish_runtime`, `generate_gamedata`, `export_gamedata`) | `controller/host.rs` → `run_kit` | `ServerEngine::run_kit` (§5.1) |
| `server_doctor`, `server_backup`, `server_migrate_mongo`, `server_config_get`, `server_config_save`, `server_confirm_fresh_start`, `server_invite_list` | `controller/host/manager.rs` | `ServerEngine` (§5.2–5.6) |
| `remote_status`, `remote_init_controller`, `remote_invite`, `remote_enroll`, `remote_revoke`, `remote_leave`, `remote_exec` | the same | `ServerEngine::remote_*` (§5.7) |
| `history_query`, `history_revert_dry_run`, `history_revert_apply` | the same | `ServerEngine::history_*` → §7 |
| `module_list`, `module_get`, `module_run_command`, `module_set_settings`, `module_load`, `module_unload`, `admin_action` | `bridge::modules::ModulePanel` | §6 |
| `guarded_list`, `guarded_approve`, `guarded_cancel` | the kit's `controller/guarded.rs` | nothing here |

The kit checks every argument before any of this is called (`bridge::checks`, `bridge::modules::module_id`, the
`HistoryFilter` and request structs); this code never sees a value the table refuses.

## 4. The embedded server (`EmbeddedServer`)

One value, made once at start-up and handed to the app in `Options::embedded_server`. State:

- the controller: `Mutex<Option<ServerController>>` (async mutex; held across a start);
- the status receiver: `Option<watch::Receiver<ServerSnapshot>>`, readable without the controller's lock, because a
  start holds that lock for a long time;
- the line sink: `Option<LineSink>`, the last one `start` was given;
- the log receiver: `Option<mpsc::UnboundedReceiver<HostLogEntry>>`, from section 8, taken once;
- the panel's two slots (section 6): the module registry and the workspace.

Std mutexes are locked ignoring poisoning (take the inner guard).

### 4.1 `start(workspace, lines)`

1. Store `lines` as the sink.
2. The first time only, take the log receiver and spawn a task that, for each entry, forwards
   `format!("{:<14} | {}", entry.service, entry.message)` to the **current** sink (none set: dropped) when the entry
   is the server's own: `service == "host"`, or `service` is in the kit's `bridge::spec::HOST_SERVER_COMPONENTS`, or
   the current status snapshot has a component of that name (a game module logs under its id, DF3). The launcher's
   own events never reach the page.
3. Lock the controller. A server already there: return `Ok(())`.
4. Set the panel's workspace to `workspace`.
5. On a blocking thread, `dainty_host::serve::prepare_embedded(&Paths::new(workspace), printer)`, where the printer
   sends `format!("{name:<14} | {line}")` to `lines`. A join error becomes its text; the function's own `Err(String)`
   is returned as it is. (Setup's checks and files and the database migrations block, so they stay off the async
   threads.)
6. Store `server.subscribe_status()` and `server.registry().cloned()` in the slots, so the module panels see the
   modules as they load.
7. `server.start_all().await`. On `Err(why)`: `shutdown().await`, clear the status and registry slots, return
   `Err(why)`.
8. Keep the server in the controller slot; `Ok(())`.

### 4.2 `stop()`, `restart(name)`, `components()`

- `stop`: take the server out of its slot and `shutdown().await` it (none: nothing), then clear the status and
  registry slots. Never fails.
- `restart(name)`: `restart_component(&name).await` on the running server; with none, `Err("the server is not
  running")`.
- `components()`: from the stored status receiver's current value, each component as `HostServerComponent { name,
  phase, port, detail }`, phases mapped one to one (`Stopped`, `Starting`, `Ready`, `Stopping`, `Failed`,
  `Disabled`). No receiver: an empty list.
- `modules()`: `Some` panel over the two slots (section 6). `engine()`: `Some` engine (section 5).

## 5. The in-process host kit (`ServerEngine`)

A stateless value. Every method builds `dainty_host::paths::Paths::new(workspace)` and calls the host's public API.
Errors are returned as their `Display` text; the kit decides what the page shows.

### 5.1 `run_kit(workspace, args, lines) -> Result<bool, String>`

`args[0]` is the subcommand; the rest are `dainty-host`-style flags. A flag that takes a value takes the next item
when one follows; an unknown flag is skipped, and a value flag at the end takes nothing. `Ok(true)` means the command
exited with success.

| `args[0]` | Flags read | Calls |
| --- | --- | --- |
| `setup` | `--mode`, `--name`, `--host`, `--game` (values), `--fresh-start` | `commands::setup::run_with(paths, &SetupArgs, &InProcessSetupEnv, lines)` |
| `invite` | `--json`, `--for <name>`; `uses` defaults to `"1"` | `commands::invite::run_with(paths, &args, invite::backend_port(paths), lines)` |
| `multipliers` | `--off`, `--xp <v>`, `--rewards <v>` | `commands::multipliers::run_with(paths, &args, Platform::current(), commit, lines)`; `commit` returns `Ok("")` (no checkout to commit to) |
| `gamedata` | `--examples`; `--export` with an optional folder (the next item unless it starts with `--`; none: an empty string); `--import <pack>` | first, when neither `paths.backend_dir` nor `paths.settings_toml` exists, `gamedata::seed_embedded_examples(paths)` (its result ignored); then `commands::gamedata::run_with` |
| `publish-runtime` | none | `commands::publish_runtime::run_with(paths, &Default::default(), lines)` |
| `update` | ignored | prints the one line `{"ok":true,"behind":0,"head":"embedded"}` and succeeds: the manager updates itself, not a checkout |
| anything else | — | `Err("unsupported command: <name>")` |

`InProcessSetupEnv` (the host's `commands::setup::SetupEnv`):

| Method | Answer |
| --- | --- |
| `platform` | `Platform::current()` |
| `is_root` | `dainty_host::linux::is_root()` |
| `tailscale_ipv4` | `doctor::tailscale_ipv4()`, or `Some("100.64.0.1")` when there is none **[T-3]** |
| `find_mongod` | `Ok(None)` |
| `find_on_path(name)` | `wine::find_on_path(&[name])` |
| `source_commit(repo)` | the release manifest's short commit (`selfupdate::read_manifest`), else `""` |
| `apply_preset` | `preset::apply_preset(paths, false)` |
| `install_services` | does nothing (`Ok`): the services are in this process |
| `make_certificate(hosts, tls)` | `dainty_host::service_main("gateway", ["make-cert", ("--host", h)…, "--out", tls, "--json"])`; not success: `HostError::message("could not make the gateway certificate")` |
| `gateway_program` | this executable's path, or `"dainty-manager"` when it cannot be found |
| `require_checkout_sources` | `false` |

### 5.2 `doctor(workspace)`

`doctor::run_doctor(paths, &DoctorOptions { hash_exe: false })`. Each check becomes `DoctorCheckView { level, label:
area, detail }`: `Ok` and `Info` → `Ok`, `Warn` → `Warning` (counted), `Fail` → `Problem` (counted). `detail` is
the check's text, or `"<text> — <fix>"` (space, U+2014, space) when it has a fix. The report carries the two counts.

### 5.3 `backup(workspace, lines)` and `migrate_mongo(workspace, lines)`

- `backup`: `commands::backup::run_with(paths, &commands::backup::System, lines)`, success as `bool`.
- `migrate_mongo`: refuse with `"not_configured"` when `load_host_config` finds no `settings.toml`; then
  `commands::db::run_with` with `DbCommand::Migrate(Default)`, then with `DbCommand::Verify(Default)`. Verify writes
  the marker `start` waits for, and only when it finds no difference (as a host does by hand).

### 5.4 `config_get(workspace)`

The page's view of `settings.toml`:

- `mongo_migration`: from `commands::update::inspect_mongo_migration` (`None`, `Migrated`, `Skipped`,
  `Unmigrated { source }` with its source), `detail: None`.
- `config_file`: `settings_toml`'s path as text when the file exists.
- No `settings.toml`: `configured: false`, `name: ""`, `mode: Private`, `public_host: None`, `registration:
  "INVITECODE"`, `database: "sqlite"`, `max_hubs: 1`, `max_hunts: 3`, `fresh_start: false`.
- Otherwise `configured: true`, the config's `name`, `mode`, `public_host` (its `host` in public mode only),
  `registration` as its string, `database` (`[database] engine`, default `"sqlite"`), `max_hubs: 1`, `max_hunts`
  (`max_hunt_servers`), and `fresh_start` when `[database] fresh_start` is true or the migration state is `Skipped`.

### 5.5 `config_save(workspace, form)` and `confirm_fresh_start(workspace)`

`config_save`, in order; any error before the save is returned and nothing is written:

1. Load `settings.toml`; none: `"not_configured"`.
2. Registration: the form's value, with `"INVITE"` read as `"INVITECODE"`; `Registration::parse` failing:
   `"registration"`.
3. `max_hunt_servers = form.max_hunts` clamped to 1..=128. `[database] engine = form.database`;
   `fresh_start = Some(true)` when the form asks, else unset.
4. `save_host_config`.
5. Form asks for a fresh start: `commands::update::confirm_fresh_start`. Otherwise, when the migration marker
   (`paths.config/MONGO_MIGRATED_MARKER`) is a fresh-start marker, delete it (ignore failure).
6. Best effort, failures ignored: load or create the secrets (`setup::secrets::new_secrets`), then
   `build_env_files(config, secrets, paths, "", Platform::current())` and `write_env_files`; and when
   `[remote] role` is `"controller"`, `remote::set_shared_config_from_settings_toml`.
7. Answer `config_get`.

`confirm_fresh_start`: `commands::update::confirm_fresh_start(paths)`, then `config_get`.

### 5.6 `invite_list(workspace)`

`settings.toml` required (`"not_configured"`); the secrets are loaded or created. In public mode, the fingerprint is
`invite::certificate_fingerprint(&paths.cert_file)` (an error is returned); private mode has none. The codes come from
`invite::list_invite_codes(service_ports(config).backend, &secrets.admin_key)`. Each item with a string `code` becomes
`ServerInviteEntry { code, link, for_name, uses_remaining, infinite_uses, created_at }`: `link` is
`invite::invite_for(config, code, fp)`, or the bare code when that fails; `for_name` is `note` when non-empty;
`uses_remaining` is `usesRemaining` (saturating to `u32`, missing 0); `infinite_uses` is `infiniteUses` (missing
false); `created_at` is `createdAt` (missing `""`). Items without a code are skipped.

### 5.7 `remote_*`

Each calls `dainty_host::commands::remote` and then answers `remote_status`, except `remote_invite` and `remote_exec`.

| Method | Call | Arguments |
| --- | --- | --- |
| `remote_status` | `status_json(paths)` | — |
| `remote_init` | `init_controller` | `node`, `region`, `url` from the request; `json: true` |
| `remote_invite` | `create_invite` | `node`, `region`, `label`, `url`; `ttl_secs: None`; `json: true` |
| `remote_enroll` | `enroll` | `uri`, `label`; `gateway_url: None`; `json: true` |
| `remote_revoke` | `revoke(paths, node)` | — |
| `remote_leave` | `leave(paths)` | — |
| `remote_exec` | `exec_command` | `node`, `action`, `account` and `for_whom` both the request's `target`, `reason`; `role: "owner"`, `actor: "manager-gui"`, `json: true` |

The status JSON → `RemoteStatusView`: `role` (default `"standalone"`), `node`, `region`, `controller_url`,
`controller_cert_fp`, `controller_node` (optional strings), `revision` (`revision`, else `applied_revision`, else 0),
`pending_enrollments`, `detached`, and `installs`, each `ManagedInstallView` with `node`, `region` (default `""`),
`label`, `state` (default `"offline"`), `enrolled_at_ms`, `last_seen_ms`, `revoked`, `applied_revision`,
`players_online`, `active_hunts`, `version`, `queued_commands`. Missing numbers are 0, missing flags false, numbers
narrowed to `u32` saturate. `remote_invite`'s JSON → `RemoteInviteView { uri, node, region, label, controller_url,
controller_fp, expires_at_ms }`; `remote_exec`'s → `RemoteExecView { command_id, node, action, status }`, same
defaults.

## 6. The module panel (`ModulePanel`, DU3, DF3)

Two shared slots, set by §4 and cleared on stop: the registry (`ModuleRegistry`) and the workspace. Nothing here takes
a path or a program from the page (DD5): the registry only knows what `settings.toml` listed.

- **No registry** (server stopped): every call is refused `ServerStopped`.
- **Listing** a module: `Listed { id, origin, autoload, state, detail, manifest, ports, commands, config }` from the
  registry's `ModuleInfo`; `origin` is `External` when `source == "external"`, else `CompiledIn`; states map one to
  one (`NotLoaded`, `Starting`, `Ready`, `Failed`). The page's view is the kit's `modules::view(listed, settings)`,
  where `settings` is that module's table from `settings.toml`'s `[modules]` settings (none when there is no
  `settings.toml` or workspace; read on a blocking thread).
- **Registry errors:** `NotListed(id)` → `ModuleUnknown` with the id; `AlreadyLoaded` → `ModuleBadState` `"loaded"`;
  `NotLoaded` → `ModuleBadState` `"not_loaded"`; any other → `ModuleFailed` with the error text cut by the kit's
  `plain(…, DETAIL_MAX)`. Refusals are the kit's `refusal(code, detail)`.
- **`settings.toml` errors** (read, parse, write, or no file when one is needed): log a warning
  `host: module settings: <why>` and refuse `ModuleFailed` `"settings_toml"`.
- `list`, `get(module)`, `load(module)`, `unload(module)`: the id through the kit's `module_id`; load and unload log
  `host: module <id> loaded|unloaded from the server GUI` and answer the module's fresh view.
- `run(module, command, input)`: the module must be `Ready` (else `ModuleBadState` `"not_loaded"`); the command must be
  one it registered (else `ModuleInputInvalid` `"command"`); the kit's `command_args(spec, input)` makes the
  arguments. The request's actor is `Actor::Console` (whoever has this window has the server, DH1). Log
  `host: module <id> command <name> run from the server GUI`. Give up after **120 s** (`ModuleFailed` `"timeout"`;
  the module keeps running, its answer is dropped); a module error is `ModuleFailed` with its message through
  `plain(…, DETAIL_MAX)`. The answer is `output_text(text)` and the data serialised as JSON, dropped when longer than
  `OUTPUT_DATA_MAX`.
- `set_settings(module, values)`: workspace required (`ServerStopped`); the module's registered config schema is the
  check. On one blocking step, load `settings.toml` (none: a `settings.toml` error), merge with the kit's
  `merge_settings(schema, saved, values)` (its refusal is returned and the file is untouched), store, save. Then
  `registry.update_settings(id, merged)`: the next load takes them, a running module keeps what it started with. Log
  `host: module <id> settings saved from the server GUI`; answer the view.
- `admin(action, player, reason, until, module)`: the target is `module` when given and non-blank, else the first
  `Ready` module whose manifest is a `Game` module (or has none), else the first `Ready` module, else
  `ModuleBadState` `"not_loaded"`. It must be `Ready`. A `Timeout` with no `until` (or a blank one) lasts one hour
  from now, as RFC 3339. Requests: `Lookup`, `Kick { reason }`, `Timeout` → `Ban { reason, until }`, `Ban { until:
  None }`, `Unban { reason }`, all as `Actor::Console`, with the same 120 s limit and error mapping; the log line
  names the action and the player (cut to 64). Answers (`AdminResultView`):
  - `NotFound`: `ok: false`, a message that no player matched (the player's text cut to 64), no player.
  - `Player(found)`: `ok: true`, a "found" message with the display name, and the player view: `global_id` the
    account's text form (or the input), `module_user_id` the account's ULID (or the input), `display_name` cut to
    64, `online`, `banned: false`, the rest `None`.
  - `Done`: a second `Lookup` (same limit; failure gives no player view); the view as above with `banned` true for
    `Ban`, `ban_reason` the reason for `Ban` and `Timeout`, `timeout_until` the computed `until`; `ok: true` and one
    message per action (found, kicked, timed out, banned, unbanned).
  The messages are English sentences in today's code **[T-4]**.

## 7. History and revert (DD23, DH3b)

Against the databases of the workspace's server, read through `dainty_admin::history`, the engine `dainty-host
history` and `revert` use. Whether the server runs does not matter. Opening: `paths.config/admin.env` must be a file
(else `HistoryUnavailable`); `history::node_database(Some(env))` (its error → `HistoryFailed`), then
`HistorySet::open`.

Error mapping: `HistoryError::Bad` → `HistoryBadFilter`; `NoDatabase` → `HistoryUnavailable`; `PlanChanged` →
`HistoryPlanChanged`; anything else → `HistoryFailed` with detail = the text's first line, cut to 200 characters (it
is shown in a banner).

The page's filter (`HistoryFilter`: `account`, `actor`, `since`, `until`, `cursor`) becomes `Selection { account,
actor, since, until }`, then `to_filter(dainty_admin::now_ms())`.

- **`query`**: `set.query(filter, cursor, history::DEFAULT_PAGE)`; each entry → `HistoryEntryView { id (text),
  database, at (ISO, `dainty_admin::input::iso`), actor, key (row key), op, before, after, table }`, `before` and
  `after` through `history::redacted_text(table, …)`; plus `next`.
- **`dry_run`**: `set.dry_run(filter)`; the plan is kept in one process-wide `PlanCache` and answered as
  `RevertPlanView { id, entries, action_count, conflict_count, unchanged_count, actions, conflicts, truncated }`.
  `actions`: every part's actions, each `RevertRowView { database, table, key, kind (history::kind_name), current,
  target }` (both through `history::redacted_json`); `conflicts`: every part's conflicts as `RevertConflictView {
  database, table, key, reason, entry, by, at, detail }` (from `history::conflict_fields`). Each list stops at
  `history::MAX_LISTED`; `truncated` when any of the three counts exceeds it. Counts are `u64`, saturating.
- **`apply(plan_id)`**: only a plan this process answered to a dry run (else `HistoryPlanChanged`): the engine plans
  again under its write lock and refuses a plan that no longer holds, so the page applies exactly what it showed.
  The actor is `history::host_actor("server-gui")`, which the change log records as `host:server-gui`. On success the
  plan is forgotten (a second apply is refused) and the answer is `RevertDoneView { at (the first part's time, ISO),
  rows_changed (sum of applied actions), databases ([{ database, rows }]), conflicts, conflict_count, truncated }`.

## 8. Start-up of the manager, and its log

The MIT `main.rs` (the app's) does, before anything else, what it does today: the game console helper first
(`platform::game_console::helper_main()`; when it answers, return that code). Then:

- **Client** (no `server-gui`): `run(Options { icon })`.
- **Manager** (`server-gui`): install the telemetry, make the embedded server with its log receiver, and
  `run(Options { embedded_server: Some(Arc::new(server)), icon })`.

Telemetry: one `tracing_subscriber::registry()` with the kit's `platform::log::LogFilter` and `LogLayer`, and the
service kit's `telemetry::ChannelLogLayer` over an unbounded channel, filtered at `INFO`. `try_init`; when it
succeeds, `dainty_service_kit::telemetry::set_tracing_bridge_active(true)`. The kit's `log::install` then finds a
subscriber set and only adds its panic hook. It must run before the app starts.

`icon` is the 32×32 PNG of each app's icon set (`tauri::include_image!`), or `None` for the config's (section 9,
[T-2]). A start error prints `Dainty Lass could not start: <error>` to stderr and exits 1.

## 9. The configs

The app's `tauri.conf.json` and `tauri.server-gui.conf.json` are both the compiled-in context (the app's `build.rs`)
and the bundler's input. Restricted's two configs are copies with paths relative to `restricted/launcher/src-tauri`;
after the move only the app's remain. The values a release depends on (bundles.rs in both packages checks them):

| Setting | Launcher | Manager (overlay) |
| --- | --- | --- |
| `productName` | `Dainty Lass Launcher` | `Dainty Lass Manager` |
| `mainBinaryName` | `dainty-launcher` | `dainty-manager` |
| `version` | `../../launcher/package.json` (2.0.0-beta) | inherited |
| `identifier` | `io.github.freezamotoqueiro.dauntlessreforgedrevival` | `io.github.freezamotoqueiro.dauntlessreforgedrevival.server` |
| `build.devUrl` | `http://localhost:4321` | `http://localhost:4322` |
| `build.frontendDist` | `dist/launcher` | `dist/server-manager` |
| `build.before{Dev,Build}Command` | `bun scripts/page.ts {dev,build} launcher` | `… server-manager` |
| `app.windows` | `[]` (the app makes its window) | inherited |
| `app.withGlobalTauri` | `false` | inherited |
| `app.security.csp` | `default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; connect-src 'self'; img-src 'self' rr-art: http://rr-art.localhost data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'` | inherited |
| `app.security.freezePrototype` | `false` | inherited |
| `app.security.pattern` | isolation, `dir` `../../launcher/isolation` | inherited |
| `bundle.targets` | `nsis`, `appimage`, `deb`, `rpm` | inherited |
| `bundle.publisher` / `homepage` / `category` | `Dainty Lass contributors` / `https://github.com/SilentSpud/DaintyLass` / `Game` | inherited |
| `bundle.license` | `MIT` (restricted's says `AGPL-3.0-only`) | inherited |
| `bundle.windows.nsis.installMode` | `currentUser` | `currentUser` |
| `bundle.windows.webviewInstallMode` | restricted: `embedBootstrapper`; the app: unset **[T-5]** | inherited |
| `bundle.linux.deb` | depends `libwebkit2gtk-4.1-0`, recommends `gnome-keyring`, `pkexec`, section `games`, template `linux/launcher.desktop` | template `linux/manager.desktop` |
| `bundle.linux.rpm` | depends `webkit2gtk4.1`, template `linux/launcher.desktop` | template `linux/manager.desktop` |
| `bundle.linux.appimage.bundleMediaFramework` | `false` | inherited |
| `plugins.updater.pubkey` | `""` **[T-6]** | inherited |
| `plugins.updater.endpoints` | `…/releases/download/launcher-updates/latest.json` | `…/latest-manager.json` |
| `plugins.updater.windows.installMode` | `passive` | inherited |
| short/long description | the launcher's | the manager's |

The identifiers, product names and binary names must not change: NSIS installs per user under the identifier, so an
existing install updates in place only while they hold. **Icons [T-2]:** restricted lists `icons/*.png`,
`icon.icns`, `icon.ico` for the launcher and `icons/server/*` for the manager, with distinct NSIS installer icons;
the app lists one placeholder set (`icons/32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.png`, `icon.ico`) for
both. Restricted's `bundles.rs` requires distinct installer icons per app; an MIT manager needs its own set
(`icons/manager/`).

## 10. Cargo features

The app's features after the move (restricted's in brackets where they differ):

| Feature | Turns on |
| --- | --- |
| `default` | `host`, `realms` [restricted: `host` only; the release passes `--features realms` anyway] |
| `host` | the kit's `host` (the Host page) |
| `realms` | the kit's `realms` (and the host's `realms` in a manager) |
| `server-gui` | `host`, the kit's `server-gui`, the new crate (section 11), and the 1.14.7 module (`isles-1-14-7`) with realms on the host — restricted's list, which `bundles.rs` checks contains `isles-1-14-7` |
| `isles-1-14-7`, `stub-module`, `all-modules` | pass through to the host's features of the same name (DF5) [restricted's, missing in the app today] |
| `custom-protocol` | Tauri's; set by `tauri build` |

The manager is built `--no-default-features --features server-gui`, the launcher `--features realms`.

## 11. Where it lives, and the release

### 11.1 The crate **[T-7]**

A new MIT library crate, **`crates/dainty-manager-server`** (package `dainty-manager-server`, `license = "MIT"`),
holding sections 4–8's telemetry: `EmbeddedServerState::new(logs)`, `init_telemetry()`, the engine, the panel and
the history functions, behind no feature of its own. It depends on `dainty-launcher-kit` (features `host`,
`server-gui`), `dainty-host` (default features), `dainty-controller`, `dainty-admin`, `dainty-module-api`,
`dainty-service-kit`, `chrono`, `serde_json`, `tokio` (`sync`, `rt`), `tracing`, `tracing-subscriber` (`registry`),
and in tests `tempfile`. It needs no Tauri: today's two `tauri::async_runtime::spawn_blocking` calls are
`tokio::task::spawn_blocking` (Tauri's runtime is Tokio). The app's `server-gui` feature adds it as an optional
dependency, and the app's `main.rs` does section 8.

Why a crate and not a module of the app: the tests of sections 6 and 7 run without a webview, and the client's
dependency tree stays free of the host kit. Tauri's layout allows either: `tauri build` only needs the config beside
the package it builds, which stays `crates/dainty-launcher-app`.

### 11.2 Shared-file lines this needs (for the integration pass)

- Root `Cargo.toml`: the member `crates/dainty-manager-server` and `dainty-manager-server = { path =
  "crates/dainty-manager-server" }` in `[workspace.dependencies]`; `Cargo.lock`.
- `restricted/Cargo.toml`: the same workspace dependency, for the oracle (section 12); its lock.
- `launcher/package.json`: `@tauri-apps/cli` `2.12.4` as a dev dependency (today only restricted's `package.json`
  has it), and the root `bun.lock`.
- `docs/port/migration-guide.md` section 2: the new crate's line. `NOTICE.md`: nothing ported.
- `fuzz/Cargo.toml`: nothing. The page's input is checked by the kit's `bridge::checks` and `bridge::modules`,
  which the existing `launcher_input` target covers **[T-8]**.
- The version data: the manager built at the root needs lane A's run-time read (port plan item 1), or
  `DAINTY_VERSION_DATA` set for the build, or its 1.14.7 module has no version data.

### 11.3 `launcher-release.yml`, as it would be (not edited here)

Both copies (`.forgejo/` and `.github/`). Today every step runs in `restricted/launcher` and `restricted`. After:

1. **Version:** read `launcher/package.json`'s version (the app's config reads it); drop the check against
   restricted's `Cargo.toml` (the app's crate version is the workspace's and is not the release version).
2. **Toolchain:** `rustup` from the root; `CARGO_BUILD_JOBS=6` on `windows-msvc` stays.
3. **Tauri CLI:** `bun install --frozen-lockfile` at the root (the CLI from `launcher/package.json`).
4. **Signing:** unchanged secrets (`LAUNCHER_UPDATER_KEY`, `LAUNCHER_UPDATER_KEY_PASSWORD`, `WINDOWS_CERTIFICATE`,
   `WINDOWS_CERTIFICATE_PASSWORD`, the optional timestamp server); `bun launcher/scripts/release-config.ts <dir>
   --tauri-dir crates/dainty-launcher-app` instead of `scripts/lass.ts release-config` (its default tauri dir is
   restricted's). It refuses to sign the updater while the app's `pubkey` is empty [T-6].
5. **Build**, working directory `crates/dainty-launcher-app`:
   `bunx tauri build --ci --bundles nsis --config <dir>/client.json -- --features realms`, collect, then
   `bunx tauri build --ci --bundles nsis --config <dir>/manager.json -- --no-default-features --features
   server-gui`. Linux the same with `--bundles appimage,deb,rpm`. Bundles land in the root `target/release/bundle/`
   (today `restricted/target/…`); the collect step's paths change with it. File names stay
   `DaintyLass{Launcher,Manager}_<version>_…`.
6. **Authenticode check, feeds, `SHA256SUMS.txt`:** unchanged, except the feed step reads the endpoints from
   `crates/dainty-launcher-app/tauri.conf.json`.
7. Paths filters in `ci.yml`'s `launcher` family gain `crates/dainty-launcher-app/**` and
   `crates/dainty-manager-server/**` if they are not already covered by `crates/**`.

Static CRT on Windows (M38) is the root `.cargo/config.toml`'s already.

## 12. The tests

### 12.1 What stays in `restricted/` as the oracle

`restricted/launcher/src-tauri/tests` (9,771 lines with fixtures) already tests the MIT crates: every file but
`bundles.rs` uses only the kit (and `dainty-protocol`, `rustic-rifts-*`, `dainty-epic`), and `controller.rs` also
uses restricted's own `server_gui` and `server_history` in its two `server-gui` cases. All of it stays and keeps
running there; the reviewer re-points it.

| File | Tests | Checks | After the move |
| --- | ---: | --- | --- |
| `bridge.rs` | 29 | the argument checks, the command table and groups, outcomes' JSON as the page reads it, the progress throttle and channel, the generated files feature-independent, numbers as JavaScript writes them | stays, unchanged |
| `controller.rs` | 23 | the player flows against a fake server (public over TLS, private over HTTP): invite, register, game, runtime, PLAY, relay, exit; bans, moves, saved 1.4.4 servers, notices; the Host page (`host`); two in-process manager flows (`server-gui`): setup, examples, config, doctor, backup; and start, invite, client join, ban, admin unban, history, stop | stays; the three references to `dainty_launcher::server_gui::EmbeddedServerState` and `dainty_launcher::server_history::query` become the new crate's |
| `controller/realms.rs` | 2 | realm host and join over a scripted transport, RD9 (no injection, leftover loader files refused), the key never reaching the realm side | stays |
| `controller_parts.rs` | 3 | game installs and the stored settings view | stays |
| `core.rs` | 17 | the game's language, the backend's 15 strings, the shared invite, status and username tables | stays |
| `epic.rs` | 10 | Epic sign-in, session file, library check | stays |
| `game_config.rs` | 11 | DR's `game-config.json` table, Engine.ini and Game.ini, display repair | stays (L5) |
| `game_folder.rs` | 7 | folder input, layouts, `Version.txt`, store manifests | stays |
| `hostapi.rs` | 12 | DR's `hostapi.json` table, the parsers, pinned requests against a fake server | stays (L5) |
| `invite_link.rs` | 13 | invite links, the single-instance channel, the game console reader | stays |
| `keystore.rs` | 19 | key files, the Electron files' framing and decryption, OS protection, backups, the harness's profile | stays |
| `launch.rs` | 7 | the command line, the masked key, the environment, exit codes | stays |
| `log.rs` | 2 | DR's `log.json` table, the redacted key | stays (L5) |
| `pinned.rs` | 3 | pinned TLS | stays |
| `platform.rs` | 7 | folders, external links, system checks | stays |
| `relay.rs` | 27 | the relay against a local TLS gateway and a plain host: QoS, `/__origin/`, the owner check, re-pointing | stays |
| `relay_owner.rs` | 4 | the TCP table, fail-closed snapshots, a real socket on Windows | stays |
| `settings.rs` | 5 | DR's `settings.json` byte for byte | stays (L5) |
| `bundles.rs` | 2 | restricted's configs against the app's, today's icons distinct per app, version and features | **retired** with restricted's configs; the app's own `tests/bundles.rs` (MIT) covers names, identifiers, icons, feeds and version. The "`server-gui` includes `isles-1-14-7`" check moves there |
| `common/mod.rs`, `fixtures/` | — | the throwaway certificate, DR's tables, invite-link and relay-rule fixtures | stay |

The package becomes test-only: no `[[bin]] dainty-launcher`, no `lib.rs` beyond what the tests need, no configs, no
`build.rs` (it existed only for Windows resources); it keeps the `fake-host-kit` binary the Host page tests start, and
`test/electron-keystore`. The reviewer makes these changes; the implementer touches nothing in `restricted/`.

### 12.2 What the implementer writes at the root

In `crates/dainty-manager-server/tests` (or unit tests):

- **The module panel** against a registry holding the stub module (`dainty_module_api::stub`, compiled in, not
  autoloaded) and a temporary workspace whose `settings.toml` is minimal (version 1, private, a name, a host, a game
  address, `INVITECODE`, 3 hunts): a stopped server refuses `ServerStopped`; before load the stub is `NotLoaded`,
  shows its manifest's display name, is unverified, and has no commands or settings; load makes it `Ready` with an
  `echo` command and a `greeting` setting; a second load is `ModuleBadState` `"loaded"`; `echo` answers `hello
  there` for `{"words":"there"}` and `hello` for `null`; an unknown command is `ModuleInputInvalid`; a failing one is
  `ModuleFailed`; unload, then unload again is `"not_loaded"`; ids not listed are `ModuleUnknown` for load, get and
  run, and nothing is added; settings are saved to `settings.toml`, a running module keeps its value until reloaded, a
  value that does not fit is refused with the file unchanged, an unloaded module has nothing to set; settings with no
  `settings.toml` are `ModuleFailed` `"settings_toml"`.
- **History** against a workspace with `admin.env` (SQLite) where an owner revoked two moderators' keys: the query by
  `admin:<owner>` lists two `admin_keys` rows in `admin`, with no key hash shown; the dry run has 2 updates on the two
  keys and no conflicts; an unknown 64-hex plan is `HistoryPlanChanged`; apply changes 2 rows in `admin`; a second
  apply is refused; the change log then holds two entries by `host:server-gui`; an unknown actor is
  `HistoryBadFilter` for query and dry run; a workspace without `admin.env` is `HistoryUnavailable`.
- **The flag reader** (§5.1): unknown flags skipped, values taken, a value flag at the end takes nothing.
- **Telemetry:** the line filter of §4.1 (host, a listed component, a module in the snapshot pass; others don't).

The restricted oracle's `server-gui` cases in `controller.rs` are the end-to-end check, run by the reviewer.

## 13. Calls and open questions

| Id | Where | The call | For |
| --- | --- | --- | --- |
| T-1 | §0 | The four embedded-server files are Category A and could move as they are; this page specifies them so a rewrite also works. Default: rewrite | maintainer |
| T-2 | §8, §9 | L2 keeps today's icons restricted. An MIT release ships the app's placeholders, the same set for both apps, until new icons are drawn, so the two installers look alike (restricted's check that they differ retires with its configs). New icon sets (`icons/`, `icons/manager/`) are hands-on work | maintainer |
| T-3 | §5.1 | Setup in the manager falls back to `100.64.0.1` when Tailscale has no address. Kept, as today; it reads as a placeholder a host may not notice | maintainer |
| T-4 | §6 | `admin_action`'s messages are English sentences made in Rust, against AGENTS.md's rule (Rust returns codes). The page does not call `admin_action` yet. Options: keep English (the implementer writes fresh sentences), or return a code per outcome and let the page's catalog word it. This page asks for fresh English and flags it | maintainer |
| T-5 | §9 | Restricted's config embeds the WebView2 bootstrapper (`embedBootstrapper`); the app's does not set it (Tauri's default downloads it). A release built from the root changes the Windows installer unless the app's config gains it. Recommended: add it | integration |
| T-6 | §9, §11.3 | Both configs' updater `pubkey` is empty, so a signed release refuses to build and no installed launcher can verify an update. Whether any shipped release had a key decides whether "an update from the previous release" can be tested through the updater at all, or only by installing over it | maintainer |
| T-7 | §11.1 | Crate name and split: `crates/dainty-manager-server` beside the app, rather than a module of the app or the plan's `launcher/src-tauri` / `dainty-launcher-shell` (the shell already exists as `dainty-launcher-app`) | integration |
| T-8 | §11.2 | No new fuzz target or security checklist: the manager adds no parser of its own. If the integration pass counts the embedded server as a trust-boundary component (RX3), a checklist page is small | integration |
| T-9 | §11.2 | Lane C's implementation of the manager waits on lane A (version data at run time); the client does not | integration |

## 14. Size

| Piece | Lines (approx.) | Size |
| --- | ---: | --- |
| `dainty-manager-server`: §4 (150), §5 (550), §6 (400), §7 (230), §8 (40) | 1,370 | Medium |
| Its tests (§12.2) | 400 | Small |
| The app: `main.rs`, `Cargo.toml` features, config changes (T-5, icons list) | 60 | Small |
| Restricted's oracle re-pointing (reviewer) | 50 | Small |

About 1.8k lines for one implementer, a day or two; nothing in it needs the game.

## 15. Hands-on checks for the maintainer

Run on Windows with the release build (NSIS), and the Linux AppImage where noted.

1. **Launch.** Install `DaintyLassLauncher_<version>_x64-setup.exe` per user; the window opens titled `Dainty Lass
   Launcher` with its icon in the taskbar and tray; tray Open and Quit work; closing while the game runs asks first.
   The same for the manager (`Dainty Lass Manager`), side by side with the launcher (distinct identifiers).
2. **PLAY.** Join a server from an invite link (the `dainty://` scheme opens the running launcher), register, pick the
   1.14.7 folder, trust and install the runtime, PLAY: the game reaches the menu through the relay, and the game log
   shows its console with the key masked. A leftover `mystic_loader.ini` or unknown DLL beside the loader refuses
   PLAY (RD9).
3. **The Host page and the manager.** In the manager: set up a server, use example data, start it (the components
   go `Ready`, the lines show `<service> | …`), make an invite, join it from the launcher, ban and unban from the
   admin tools, read the history and revert a change, run the doctor and a backup, load and unload a module and run
   a command, stop the server. In the launcher with Tools for hosts on: the Host page's requirements and folders.
4. **Update from the previous release.** With the previous release installed (launcher and manager), install the new
   one over it: settings, saved servers and the account key survive, and the install is in place (one entry in
   Apps). Then, if T-6 allows, publish a feed and let the old build update itself.
5. **Signing.** With the secrets set: every `.exe` and installer passes `Get-AuthenticodeSignature`; each bundle has
   its `.sig`; `latest.json` and `latest-manager.json` are written and the app verifies them; SmartScreen shows the
   publisher.
6. **Linux.** The AppImage starts; the deb installs with its dependencies; `dainty://` links reach the running
   launcher.
