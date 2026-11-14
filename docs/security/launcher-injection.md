<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the launcher loading DLLs into the game (RX3)

The checklist for `platform::inject` in `crates/dainty-launcher-kit`, and for the launch path that feeds it
(`platform::launch::OsSpawner`, `controller/play.rs`, `controller/host.rs`, `controller/loader.rs`). The decision is in
[../launcher-injection.md](../launcher-injection.md). RD9 as amended on 2026-11-12: on Windows the launcher injects,
and under Proton the runtime loader (`winmm.dll`) still loads the runtime.

How to use it: copy this list into the review, check each item in the code (not from memory), link the code or test
that shows it, and sign at the end with the date and the commit reviewed. An item that does not hold is a bug to
fix first, not a box to leave open.

## What it is and where the boundary is

The launcher writes into another process's memory and starts a thread in it. Whatever DLL it names runs with the
player's rights inside the game, and the game holds the account key on its command line. So the boundary covers
both which process is touched and which file is loaded.

- **Process:** only the child `OsSpawner` itself just created with `CREATE_SUSPENDED`, reached through that child's
  own handle (`tokio::process::Child::raw_handle`). There is no lookup by pid or name and no `OpenProcess`.
- **Files:** the MIT runtime (`rustic-isles-runtime.dll` in `Win64`, only when the offered or local set has it,
  RT-5), then the runtime (`Rustic-Isles.dll`, or its pre-DX4 name, in `Win64`), the host's CatalogExporter (the host
  kit's build output) and the host's Dumper-7 pick. Each is named by absolute path.
- **Little parsing:** nothing read from the network or a file chooses a path. The expected runtime hash comes from
  the offered manifest that the runtime check (`check_runtime`) has just matched and the player has trusted. The one
  new parser is the Proton helper's command line, fuzzed by `inject_helper_args`.

Code: `crates/dainty-inject` (the library and the helper), `crates/dainty-launcher-kit/src/platform/inject.rs`,
`platform/launch.rs` (`OsSpawner::spawn`, `LaunchPlan::through_helper`). Tests are in
`inject.rs` and in `launch.rs` (`a_dll_is_loaded_before_the_process_runs_then_it_resumes` on Windows runs the whole
path against `cmd.exe`).

## Checklist

### The process

- [ ] Injection happens only in `OsSpawner::spawn` and, for a realm's hosted server, `InjectingStarter::start`
      (`platform/hosted_server.rs`), each on the child it has just spawned suspended. No other caller of
      `load_and_resume` exists. The hosted server gets the runtimes PLAY would load (`runtime_dlls`, the offered
      hashes), is killed rather than resumed when a load fails, and starts with `rifts-host`'s launch environment
      minus every other inherited server-prefix variable, plus `DAINTY_RUNTIME_INJECTED=1`
      (`a_hosted_launch_carries_exactly_what_the_runtime_reads`).
- [ ] The only server-supplied values in the game's environment are the status's `economy` (R15 Q-1):
      `DAINTY_KEEP_SUPPLY_STACK_CAPS` from a JSON bool and `DAINTY_CELL_FUSION_MINUTES` from a whole number 0–10080.
      `dainty-protocol`'s parser refuses anything else (a string, a float, past the week) rather than clamping, and
      the launcher writes nothing for a refused or absent field, its own inherited copies stripped already
      (`status.json`'s economy cases, `economy_reaches_the_game_only_as_the_server_sent_it`). A refusal is logged by
      field name only, never with the value (`tests/controller.rs`, the public server's PLAY).
- [ ] The child is created with `CREATE_SUSPENDED` whenever `plan.inject` is not empty, and only then.
- [ ] The main thread is found before any remote thread exists (`only_thread` refuses a process with more than one
      thread), and only that thread is resumed.
- [ ] On any error (a hash, a load, a timeout, the resume itself) the child is killed (`start_kill`) and the spawn
      fails. A game is never resumed without every DLL it was meant to have.
- [ ] Off Windows a plan with DLLs to inject is refused (`ErrorKind::Unsupported`), and PLAY under a runner never
      sets any: under Proton the helper injects, inside Wine.

### The Proton helper (`dainty-inject.exe`)

- [ ] It runs only when the player has turned the Linux-only `injectHelper` switch on, and only from the launcher's
      own resources folder (`ControllerConfig::resources`). A missing helper refuses PLAY
      (`inject_helper_missing`) and is never looked for anywhere else.
- [ ] Its command line is built by `inject::helper_line` and parsed by `helper::parse`, which accepts only
      `--dll <path> <64 lowercase hex>` (one to four times), then `--`, the program and its arguments. Anything else
      exits `0xD1A00001` without starting anything (fuzz target `inject_helper_args`).
- [ ] It starts only the program it was given, suspended, and loads into that child with the same
      `dainty_inject::load_and_resume`: the same hash re-check through a held handle, the same kill on failure
      (`0xD1A00003`).
- [ ] The key reaches the helper only as the game's own argument, which it passes on untouched. The launcher's log
      shows the line through `LaunchPlan::describe`, masked. The helper prints only paths and errors.

### The files

- [ ] Every path is absolute and ends in `.dll` (`is_dll_path`). `LoadLibraryW` gets that full path, so the search
      order never picks the file.
- [ ] Each file is opened with `FILE_SHARE_READ` only, hashed through that handle, and the handle is held until the
      last `LoadLibraryW` has returned (`hold`). A held file can't be written, renamed or deleted
      (`a_held_dll_cannot_be_replaced`).
- [ ] The runtime's expected hash is the offered manifest's, when a server offers one and no local runtime is set
      (`Controller::runtime_dll`). Otherwise it is the file as found, which is no weaker than the runtime check
      before it.
- [ ] The Host tools' hashes are logged at session start (`host: loading ..., SHA-256 ...`).
- [ ] A Dumper-7 session injects Dumper-7 alone. The runtime is never loaded with it.

### The game folder

- [ ] On Windows, PLAY, the Host sessions and realms refuse while a `winmm.dll` without the stand-down marker sits
      in `Win64` (`Controller::loader_leftovers`): an older loader would load the runtime a second time.
- [ ] A loader with the marker (`inject::loader_stands_down`) is accepted, because it loads nothing when
      `DAINTY_RUNTIME_INJECTED=1`. Every injected plan sets that variable (`LaunchPlan::with_inject`,
      `through_helper`). The marker says how the file behaves, not who made it. A hostile `winmm.dll` would run in
      the game whatever PLAY decided, since the game imports `winmm`, so the refusal guards against loading twice,
      not against tampering.
- [ ] `runtime_files::retire_loader` removes a `winmm.dll` only when the launcher's runtime marker lists that exact
      hash and it lacks the stand-down marker. A loader someone else placed, or a host's servers' loader, is never
      deleted.
- [ ] The Windows runtime install never downloads `winmm.dll` (`PLAYER_RUNTIME_FILES`), even when the feed lists it.
- [ ] `rustic-isles-runtime.dll` is injected before `Rustic-Isles.dll` with the offered set's hash, and only when that
      set lists it; one the set doesn't list makes the check `Outdated` and the install removes it, so neither PLAY
      nor the Proton loader loads it beside a runtime it doesn't pair with (`runtime_dlls`,
      `the_mit_runtime_is_held_only_when_the_set_lists_it`). It is the runtime's own name: never a leftover, never a
      session's DLL, set aside for a dump (`the_mit_runtime_is_the_runtimes_own`).
- [ ] `mystic_loader.ini`, the session record and set-aside runtime files are still refused as before
      (`loader_session::leftovers`).

### Secrets

- [ ] Nothing here logs the account key. The plan's command line is logged only through `LaunchPlan::describe`,
      which masks it.

## Hands-on (maintainer, with the game)

The steps are in [../launcher-injection.md](../launcher-injection.md), under "Hands-on checks". `cargo test` passing
proves nothing about the real game.

Reviewed by: ______  Date: ______  Commit: ______
