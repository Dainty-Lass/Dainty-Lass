<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the server controller and its console (RX3)

The checklist for the server as one process (RD26): the controller in `rust/crates/controller`, `reforged-host
serve` in `rust/services/host/src/serve.rs`, its console (`repl.rs`) and dashboard (`tui.rs`), the settings readers
it loads (`hostconfig.rs`, `hostconfig/yaml.rs`) and the server GUI's embedded server
(`rust/launcher/src-tauri/src/server_gui.rs`). `SECURITY_REVIEW.md` indexes it.

How to use it: before `serve` replaces `reforged-host start` for hosts, copy this list into the cutover PR, check
each item in the code (not from memory), link the code or test that shows it, and sign at the end with the date and
the commit reviewed. An item that does not hold is a bug to fix before the cutover, not a box to leave open.

## What it is and where the boundary is

The controller starts the server's modules in order (allowlist, gateway, content, admin, the compiled-in `isles-1-14-7`
Rust backend on port 61000 and in-process `isles-director` on port 61001 over SQLite or Postgres; or MongoDB and the Node backend/Director when built with the opt-in `paradox` rollback
feature), keeps each one's phase, and stops them in reverse. It opens no listener of its own: every port belongs to a
module, and each module keeps the checklist of its component (`gateway.md`, `allowlist.md`, `content.md`,
`admin-api.md`, `host-start.md`). What the controller adds is one process holding all of them, a console that drives
them, and the files it reads to compose them.

- **In, the host's files:** `host.toml`, `host.yaml`/`host.yml` or a Node kit's `host.json`, and the env files setup
  wrote. They are the host's own, but a broken or planted file must only ever be refused.
- **In, the terminal:** console lines on stdin (`serve --interactive`) and the dashboard's keys and `:` prompt
  (`serve --tui`). Whoever has the terminal is the host: the console has no login, by design.
- **In, the services' output:** every line a supervised process (Node, MongoDB) prints, and the in-process services'
  log events. A Node service can echo what a player sent (a name, a chat line) into its output.
- **In, the server GUI:** `host_server_start`, `host_server_stop` and `host_server_restart` from the launcher page,
  over the bridge (`bridge/spec.rs`).
- **Out:** the host's terminal, `logs/server.log` (dashboard mode), the page's server panel, and the env each
  module gets.

Code: `crates/controller/src/controller.rs` (start, ready timeout, stop grace, restart, shutdown),
`services/host/src/serve.rs` (`compose`, `prepare`, `run`, `prepare_embedded`, the printers), `repl.rs`
(`parse_command`, `run_command`, `plain`), `tui.rs` (`on_key`, `run_tui`), `hostconfig.rs` (`read_host_config`),
`hostconfig/yaml.rs`, `services.rs` (`pump`), `launcher/src-tauri/src/server_gui.rs` and `bridge/commands.rs`
(`host_server_restart`).

Fuzz targets: `host_config` in `rust/fuzz` (host.toml, the YAML reader and host.json, and an accepted file written
back as host.toml and read again as the same settings), and `host_console` (a console line: what the parser accepts
names only plain service names and known levels, what it answers carries no control character, and a service's line
as `plain` prints it carries none either).

## Checklist

### What it reads

- [ ] A settings file larger than 256 KiB is refused before it is read (`MAX_HOST_CONFIG_BYTES`), so a planted file
      cannot hold the server's memory.
- [ ] The YAML reader refuses nesting deeper than 32 levels, blocks and brackets alike, and any text over 256 KiB
      (`MAX_DEPTH`, `MAX_YAML_BYTES`): nothing in it recurses without end. Test:
      `refuses_what_would_recurse_without_end_or_is_too_large`. host.toml (`toml_edit`) and host.json (`serde_json`,
      128 levels) are bounded by their parsers.
- [ ] All three forms go through the same checks as the Node kit's host.json (`HostConfig::from_json`), so a value a
      form accepts is one setup would have written. The `host_config` target checks the round trip.
- [ ] Secrets stay in `secrets.env` and the env files; the settings file never holds one, and the controller passes
      each module only its own env file (`module_env`, `the_env_files_are_the_ones_the_modules_read`).

### The console and the dashboard

- [ ] A console line over 512 characters is refused before it is split (`MAX_LINE`); a service name is 1 to 32 of
      `a-z`, `0-9` and `-` (`is_service_name`), and a level is one of `LEVELS`. Test:
      `reads_each_command_and_refuses_the_rest_without_echoing_it`.
- [ ] What the console answers never repeats what was typed unless it is a plain word: an escape sequence typed at
      the prompt is not written back to the terminal.
- [ ] The dashboard's `:` prompt takes only printable characters, up to `MAX_LINE`, and runs through the same parser;
      its keys map to the same commands (`on_key`). Tests: `the_keys_pick_restart_start_stop_filter_and_scroll`,
      `the_prompt_runs_a_command_and_takes_only_text`.
- [ ] Every line from a supervised process is cut to 400 characters (`pump`) and passed through `repl::plain` before
      it reaches the terminal, and the dashboard's log pane does the same for every event: escape sequences and
      other control characters are dropped. The in-process services' lines in console mode go through
      tracing-subscriber's own escaping (0.3.20 and later).
- [ ] In dashboard mode a process's lines go into the log pane (`ring_printer`), never printed over the screen.
- [ ] `invite` prints how to make an invite and the certificate's SHA-256, never a key or an invite link; `status`
      and `doctor` print each module's phase, port and detail. Check that no module's detail can hold a secret
      (a URL with credentials, a key, a token).
- [ ] `log <service> debug` changes only that service's level. Check that every service's redaction (the gateway's,
      the content server's) holds at `debug` and `trace` as it does at `info`.

### Lifecycle

- [ ] A module that does not report ready within its timeout (5 s unless it says otherwise) is aborted and shown
      `Failed`, not left `Starting`. Test: `a_module_that_never_reports_ready_is_a_failed_start`.
- [ ] A module whose port is taken fails its start with the reason, and a required module's failure stops
      `start_all`, with what had started stopped by `shutdown`. Tests:
      `a_port_already_taken_fails_the_start_with_the_reason`,
      `a_required_failure_stops_the_start_and_an_optional_one_does_not`.
- [ ] A module that ignores its stop signal is aborted after its grace (10 s unless it says otherwise), so it keeps no
      port or thread, and is shown `Stopped` with the reason. Test:
      `a_module_that_ignores_stop_is_aborted_after_its_grace`.
- [ ] `shutdown` stops the last started first; a restart moves the module to the end of that order, and a stopped
      neighbour stays stopped. Tests: `starts_in_order_and_stops_in_reverse`,
      `a_restart_leaves_a_stopped_neighbour_stopped`.
- [ ] Ctrl+C, the other stop signals, and `stop` or a closed stdin when the launcher's Host page runs the kit, stop
      the server through the same `shutdown` (`Control::begin`, `watch_stop_input`, as `reforged-host start` does).
- [ ] A build with no modules says so and serves nothing (`empty_build_message`, `an_empty_build_says_so`): it never
      binds a port.

### The module registry (DF3)

- [ ] Only a module `host.toml` lists can be loaded (`ModuleEntry`, `RegistryError::NotListed`); the console and
      the admin API name a module by its id, never a path. An external one is checked against its SHA-256 and its
      hello must name the id listed (`ExternalModule::expected`).
- [ ] A second game module for a version already served is refused whichever loads first, and so is one for a
      version the host kit's own stack serves (`taken_versions`). Test:
      `two_modules_for_one_version_are_refused_in_either_order`.
- [ ] A module may emit only the events it registered, and an event reaches only the modules that subscribed
      (`Link::call`, `publish`).
- [ ] `[modules.<id>.<section>]` sets only what the module registered: anything else refuses the load, rather than
      being passed on. Test: `a_load_moves_clashing_ports_takes_its_settings_and_joins_the_bus`.
- [ ] Ports: the server's own (`RESERVED_PORTS`, the gateway's, the game servers' UDP range) are never given to a
      module, and a clash moves the module's port, never the server's (`PortBook`).
- [ ] A module's log line and stderr reach the logs under its own id, control characters replaced (`log_safe`); the
      console prints a module's answers and details through `plain`. Check that the server GUI shows a module's lines
      only when it is one of the server's components (`drain_logs`).
- [ ] A module whose process dies shows `Failed` with the reason, its tree is killed, and it stays failed until it
      is loaded again; `shutdown` unloads every module before it stops the services. Tests:
      `a_module_killed_mid_run_shows_failed_and_restarts_on_command`, `nothing_is_left_running_after_stop`.
- [ ] The console's `module run` takes at most 16 arguments of 128 characters of text, no control character
      (`parse_module`); the `host_console` fuzz target checks it.

### The server GUI

- [ ] `host_server_restart` takes only a name in `HOST_SERVER_COMPONENTS`; anything else is refused before the
      controller sees it (`bridge/commands.rs`).
- [ ] Only the server's own log events reach the page (the host kit's, and those inside a module's span), never the
      launcher's own (`EmbeddedServerState::drain_logs`): the player's side of the launcher stays out of the
      server's panel.
- [ ] The page shows the server's lines as text, never as markup. Check the Host page's log view.
- [ ] Closing the launcher stops the embedded server through `shutdown`, as the console's `stop` does
      (`RunEvent::ExitRequested`, `host_shutdown`).

## Open points

These do not block Phase 6 but must be settled, or accepted in writing, before the cutover:

- **One process, one failure domain.** A module that panics is caught by its task, but one that blocks a runtime
  thread or leaks memory hurts every other module. Node's separate processes did not share that.
- **The console's own log file.** Dashboard mode writes `logs/server.log` with every event at the levels the host
  chose; the file's permissions are the workspace's. Check them on Linux (`hosting-linux.md`).
- **Supervised restarts.** A Node service that dies is restarted by the supervisor without limit; a crash loop
  fills the log. `reforged-host start` behaves the same.

## Fuzzing

Short runs of `host_config` and `host_console` happen on every PR (CI's `fuzz` job runs every target in `cargo fuzz
list`). The run before the cutover, from `rust/fuzz`:

```bash
cargo +nightly fuzz run host_config -- -max_total_time=86400
```

```bash
cargo +nightly fuzz run host_console -- -max_total_time=86400
```

Seeding `corpus/host_config` with the kit's host.json fixtures, each prefixed with the byte `0x02`, gets the run to
the write-back check in seconds.

## Changes from `reforged-host start`

- One process runs the Rust services as modules; `start` runs each as its own process. The Node services and MongoDB
  are supervised processes in both.
- The console, the dashboard and the server GUI are new: `start` had only Ctrl+C.
- Settings may be YAML as well as TOML and host.json (RD26); YAML is read by the kit's own small reader, capped as
  above.

Signed: ________ (date, commit)
