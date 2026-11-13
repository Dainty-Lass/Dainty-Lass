<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the Director loading the runtime into its game servers (RX3)

The checklist for `rustic_isles_director::runtime` and the start path that feeds it (`gameservers.rs`'s start,
`launcher::OsLauncher`, `os::spawn`), and for the Wine wrapper's helper line (`dainty_hostkit::wine::build_launch`).
The decision is in [../director-injection.md](../director-injection.md). The injection itself is `dainty-inject`,
whose checklist is [launcher-injection.md](launcher-injection.md).

How to use it: copy this list into the review, check each item in the code (not from memory), link the code or test
that shows it, and sign at the end with the date and the commit reviewed. An item that does not hold is a bug to
fix first, not a box to leave open.

## What it is and where the boundary is

The Director writes into its game servers' memory and starts a thread in each. Whatever DLL it names runs inside a
server that holds the gameserver key on its command line and talks to the backend as a trusted game server.

- **Process:** only the server `os::spawn` has just created with `CREATE_SUSPENDED`, inside its own job, reached
  through the handle `CreateProcessW` returned. Under Wine, only the game the helper has just started.
- **File:** only `Rustic-Isles.dll` in `GAMESERVER_RUNTIME_DIR` (the host's `.workspace/runtime`), by absolute path.
- **Parsing:** `runtime.json`, which `publish-runtime` writes and the host owns, read for one SHA-256 (fuzz target
  `director_runtime_manifest`). The helper's command line is `inject_helper_args`.

## Checklist

### The process

- [ ] `CREATE_SUSPENDED` is set only when `Command::inject` is not empty, and then `load_and_resume` runs on that
      child before anything else is done with it.
- [ ] On any error the job is terminated (`TerminateJobObject`) and the start fails. A server is never resumed
      without its runtime (`the_runtime_is_loaded_before_the_server_runs`).
- [ ] Off Windows a command with DLLs is refused (`OsLauncher::start`, `ErrorKind::Unsupported`); under Wine only the
      helper injects.
- [ ] The helper's exit codes reach the Director as the server's exit code, and the restart backoff handles them as
      any other exit.

### The file

- [ ] The path is `GAMESERVER_RUNTIME_DIR` joined with `Rustic-Isles.dll`; the directory must be absolute
      (`runtime::published`). Nothing a player or the network sends chooses it.
- [ ] The hash is `runtime.json`'s single `Rustic-Isles.dll` entry, 64 lowercase hex; a second entry, a missing one,
      a bad hash or a file over 64 KiB refuses the start (`the_runtime_hash_is_read_strictly`).
- [ ] The file is hashed again through a handle held deny-write until `LoadLibraryW` returns (`dainty_inject::hold`),
      so a file changed after publishing is refused, not loaded.
- [ ] `DAINTY_INJECT_DLL` and `DAINTY_INJECT_SHA256` reach the wrapper only from the Director: any inherited value is
      removed before a start (`a_start_injects_or_hands_the_wrapper_the_dll`), and the wrapper removes both from the
      game's own environment.
- [ ] The helper is only the one beside the running `dainty-host` (`wine::inject_helper`); a missing helper refuses
      the start with its path, and is never looked for on `PATH`.

### The game folder

- [ ] `retire_server_loader` deletes `winmm.dll` next to the server exe only when it is byte for byte the published
      loader; any other is left and named by the Director at start (`runtime_problem`).
- [ ] Wine's `winmm` override is `b`, so a loader left there is not loaded.
- [ ] Every injected start sets `DAINTY_RUNTIME_INJECTED=1` (an inherited value is removed first), so on Windows a
      loader carrying the stand-down marker loads nothing; one without it is named at start.

### Secrets

- [ ] The gameserver key is still never logged: `Command` has no `Debug`, and the new errors name only paths and the
      injector's error.

## Hands-on (maintainer, with the game)

The steps are in [../director-injection.md](../director-injection.md), under "Hands-on checks". `cargo test`
passing proves nothing about the real game.

Reviewed by: ______  Date: ______  Commit: ______
