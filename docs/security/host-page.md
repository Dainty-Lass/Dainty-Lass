<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the launcher's Host page (RX3)

The checklist for the launcher's Host page (the `host` build): the workspace it drives, the `dainty-host` it runs
and supervises, the build tools it starts, and the Host tools (Dumper-7 and CatalogExporter) it loads into the game.
Written from the MIT code: `crates/dainty-launcher-kit/src/controller/host.rs` and `controller/host/manager.rs`,
`platform/hostkit.rs` and `platform/hostkit/parse.rs`, `platform/hostserver.rs`, `platform/loader_session.rs`,
`controller/loader.rs` and `platform/inject.rs`. The injection itself is `docs/security/launcher-injection.md`'s.
`SECURITY_REVIEW.md` indexes it.

How to use it: before a release that ships the Host page, copy this list into the release commit, check each item in
the code (not from memory), link the code or test that shows it, and sign at the end with the date and the commit
reviewed.

## What it is and where the boundary is

- **In, from the page (untrusted as any command's caller):** `host_*` commands with the shapes in `spec.rs`'s
  `Host` group: a listed `HostAction`, `HostFolder` or server component, a checked `host_setup` form, multipliers,
  the invite's note.
- **In, from the host's own files (anything running as the host can rewrite them):** the chosen workspace and the
  `dainty-host` in it, `.workspace/config/settings.toml`, the Dumper-7 DLL and game-data pack the host picked,
  the loader session record in the launcher's user data.
- **In, from programs it runs:** `dainty-host`'s output (its `Error:` lines and `--json` lines), `vswhere`, cargo
  and MSBuild output.
- **Out:** `dainty-host` subcommands with arguments the launcher builds, build commands, logs under
  `.workspace/logs`, the invite link in the server view, and DLLs loaded into a game this launcher started.

The Host page is a convenience for the PC's own user: the workspace, the host kit and the tools are the host's
choice, so the boundary is about not widening what a page call or a leftover file can do, and about keeping the
invite (a secret) out of logs.

## Checklist

### No injection beyond the Host tools

- [ ] Dumper-7 and CatalogExporter load only into a game this launcher just started (RD9): `session_game` refuses off
  Windows, while the launcher is busy or a game runs, with no or the wrong game folder, and `inject_dumper` refuses
  while another client is running.
- [ ] Dumper-7 is loaded alone, never with the runtime, and its launch environment drops every variable whose name
  starts with one of `STRIPPED_PREFIXES`.
- [ ] Each Host tool DLL is opened and hashed as found (`InjectDll::as_found`) and the hash is logged; the injector
  holds the file and loads only that hash (`dainty-inject`, see `launcher-injection.md`).
- [ ] CatalogExporter is the workspace's own build (`WorkspacePaths::exporter_dll`) unless the caller names one; it
  rides an ordinary PLAY, so every PLAY check below applies.

### The loader's list (`mystic_loader.ini`)

Under Proton without the inject helper the loader loads the Host tools instead (`loader_session.rs`).

- [ ] `begin` refuses a picked file whose name is not `*.dll`, or is the loader's or a runtime's own name (`is_own`),
  refuses leftovers and a clash with an existing file of the picked name, and needs the loader present.
- [ ] The session record is written before anything is copied, so a launcher closed mid-session undoes it at its
  next start (`recover`); any failure while placing undoes what was done (test `a_failed_copy_undoes_the_session`).
- [ ] The ini names only the picked DLL; a Dump session sets the runtime DLLs aside (`.rr-dump-off`) and puts them
  back on `end`, unless a newer runtime was installed meanwhile.
- [ ] A record whose `dll` is not a plain file name is ignored (test `a_record_naming_a_path_is_ignored`); records
  and inis over 64 KiB are not read.

### The check before every PLAY

- [ ] `loader_leftovers` lists the ini, every plain name a leftover ini lists (read as the loader reads it,
  `ini_entries`), an unfinished session's DLL and set-aside runtimes, and any DLL named like the host's Dumper-7
  or `CatalogExporter.dll`; PLAY refuses while the list is not empty (`LoaderLeftovers`, test
  `leftovers_are_the_ini_what_it_names_and_the_hosts_tools`).
- [ ] When the launcher injects, a `winmm.dll` beside the game is a leftover unless it stands down for an injected
  game (`loader_stands_down`); a loader the launcher installed earlier is retired first (`retire_loader`).
- [ ] Ending a session retries five times; a session whose files stay is logged as an error and PLAY keeps refusing.

### Secrets

- [ ] The invite link is a secret: `host_create_invite` runs `invite --json`, and only the server view carries the
  link (`set_invite`). The launcher's log records only that an invite was made (with or without a note).
- [ ] A `dainty-host invite` line that starts with `{` (the JSON with the link) reaches neither the view nor the log
  file, in the child path (`HostServer::command`, test `invite_command_redacts_json_lines_from_view_and_log`) and
  the embedded path (`KitLog::open(.., command == "invite")`).
- [ ] An invite is accepted only from the last JSON line, at most 4096 bytes, with no whitespace or control
  character and a known scheme (`clean_invite_link`); the note is at most 40 characters with no control character.
- [ ] Every output line is cleaned (`clean_line`: no CR, at most 400 characters) before it is shown or logged;
  failure reasons are cut to 300 (`failure_line`).
- [ ] The launcher reads `settings.toml` (at most 1 MiB, `read_bounded_string`) for the name, mode, host and
  multipliers only; it never reads `secrets.env`.

### Running the host kit and the build tools

- [ ] `dainty-host` is the workspace's own (`find_host_binary`: `<root>`, `<root>/bin`, or a checkout's
  `target/<profile>`); a folder without it and `modules/rustic-isles` is not a workspace.
- [ ] Every subcommand's arguments are built by the launcher (`setup --mode --name [--host] [--game]`,
  `invite --json [--for]`, `update --check --json`, `multipliers`); page values reach them only after
  `checks::host_setup`, `checks::multipliers` (1–100) and `invite_for`. Arguments go as a vector, never through a
  shell.
- [ ] Children run with stdin closed, `NO_COLOR`, hidden windows on Windows, and `kill_on_drop`; the server is
  stopped cleanly and its tree killed after the stop timeout (`kill_tree`).
- [ ] Host jobs refuse while another job runs (`busy_job`); `host_setup` also refuses while the server runs, and work
  that needs the server stopped stops and restarts it (`while_stopped`).
- [ ] `host_server_restart` takes only a name from `HOST_SERVER_COMPONENTS`; `host_open` only a `HostFolder`, mapped
  to a workspace path, never a page path.
- [ ] `exec_text` (vswhere and version probes) has a 20 s deadline and a 4 MiB cap.
- [ ] `import_sdk` copies regular files and folders only, skipping symlinks (`copy_tree`), and replaces the old SDK
  folder rather than merging into it.
- [ ] Multipliers are refused on a public server (`multipliers_public`).

## Fuzz targets

- `launcher_host`: `dainty-host`'s output as the page reads it (`clean_line`, `failure_line`, `invite_line`,
  the `update` and pack lines, the published version), `settings.toml` and `host.json`, and `vswhere`'s answers;
  compiles `platform/hostkit/parse.rs` in directly.
- `launcher_bridge`: the hook shapes of every command in the table and `checks::host_setup` and the other checks
  behind it.

## Open points

- [ ] **Fixed 2026-11-26: the session record is checked whole.** `read_record` takes a record only when `dll` is a
  plain `.dll` name that is not the loader's or a runtime's, `dir` is absolute and holds the loader or a runtime the
  record set aside, and every `aside` pair is `dir/<own runtime DLL>` and `dir/<it>.rr-dump-off` (test
  `a_record_reaching_outside_the_game_folder_is_ignored`). `dir` is not compared with the configured game folder,
  which may have changed since the session began.
- [ ] **Accepted (local writer): `loader_stands_down` is a marker search, not a hash:** any `winmm.dll` holding
  `dainty-loader-stands-down:DAINTY_RUNTIME_INJECTED=1` passes the PLAY check. Pin the loaders the launcher knows by
  hash, or record that a local writer of the game folder is out of scope.
- [ ] **Fixed 2026-11-26:** `INVITE_LINK_PREFIXES` is `dauntless://` and `dainty://` only, so the Host page refuses a
  `dauntless-revived://` link as the invite parser does (RD5; test `a_revived_invite_link_is_refused`). The host kit
  never prints one.
- [ ] **Accepted, host docs to follow: `dainty-host` and the toolchain are run as found**, with no hash: the workspace's binary, cargo from
  `CARGO_HOME`, the home folder or `PATH`, `vswhere` under `ProgramFiles(x86)`. That is the host's own choice of
  folder; note it in the host docs.
- [ ] **Accepted: `run_logged` and `HostServer::command` have no overall deadline** (builds and `setup` can run long, and only
  a page stop or exit ends them); the build logs record the full command line and every output line.

## Sign-off

| Date | Commit reviewed | Reviewer | Notes |
| --- | --- | --- | --- |
| | | | |
