# The Director injects the runtime into its game servers

Status: **implemented; tried on Windows against the test server, not yet with the game.** The security checklist is
[security/director-injection.md](security/director-injection.md). The launcher's half is
[launcher-injection.md](launcher-injection.md).

## Decision note (for the integration pass)

**Amends RD9 again (maintainer, 2026-11-13).** The first decision kept the `winmm.dll` runtime loader for dedicated
servers (option (a) in [launcher-injection.md](launcher-injection.md)). That broke on the first hands-on run on
2026-11-13. A host who plays uses the same 1.14.7 folder for their servers (`game_dir`), so the launcher refused PLAY
over the servers' `winmm.dll`. With it removed, Ramsgate started without the runtime, came up as a windowed client
and exited in a restart loop. The maintainer chose option (b): the Director injects the runtime too, and the loader
retires from the servers.

**What changes for a host:**

1. Every game server the Director starts (Ramsgate, the Training Grounds, each hunt) gets the host's published
   runtime, `.workspace/runtime/Rustic-Isles.dll`, loaded before its first instruction. The expected SHA-256 is the
   one `runtime.json` lists, read again at each start, so a `publish-runtime` reaches the next server without a
   restart.
2. Nothing is copied next to the server's executable any more. `setup`, `start` and `publish-runtime` remove a
   `winmm.dll` there when it is a loader this workspace published, now or before (`publish_runtime` records a
   loader it drops or replaces in `.workspace/runtime/retired-loaders.txt`), and leave any other one. Every injected server
   gets `DAINTY_RUNTIME_INJECTED=1`, so a loader that stands down (the launcher branch's `f8df0b58`) loads nothing;
   the Director names any other loader at start, since it would load a second runtime.
3. Under Wine (Linux, Docker) the game server starts through the `dainty-inject.exe` helper that Proton players use,
   from beside `dainty-host`. `WINEDLLOVERRIDES` now has `winmm=b`, Wine's own, so a leftover loader is never loaded.
4. `winmm.dll` is optional in `publish-runtime`: the runtime is required, the loader is published when built.
5. Fail closed. With `GAMESERVER_RUNTIME_DIR` set, a start whose `runtime.json` can't be read, whose file doesn't
   match it, or whose load fails is refused. The server process is ended in its job (Windows) or by the helper
   (Wine) and never resumed. The Director logs it in English.

**New setting:** `GAMESERVER_RUNTIME_DIR` in `director.env`, written by `setup` from the workspace layout (not a
`host.toml` field: no host chooses it). A `director.env` from an older kit lacks it. The Director then refuses
every start of the game (`Dauntless-Win64-Shipping.exe` or the `dainty-gameserver` wrapper), logging that the host
should run `setup`, rather than start a game that comes up as a windowed client and never reports ready. `update`
reruns `setup`; a host who replaced the binary by hand runs `setup` once.

**What can go (not deleted here):**
- `native/runtime-loader` is still needed by Linux players who load the runtime through the loader, the default
  until the helper passes its hands-on Proton check ([launcher-injection.md](launcher-injection.md), "Still to do").
  Once that is the default, nothing uses it, and it can go with `LOADER_DLL` in the protocol and the feed.
- The client feed's optional `winmm.dll` stays for the same players.
- Restricted's Node Director rollback (`DAINTY_ROLLBACK=1`) still expects `winmm.dll` and `Rustic-Isles.dll` next to
  the server exe, which nothing installs any more. It no longer starts a working server.

## Design, as built

**Windows.** `rustic_isles_director::runtime::for_start` reads `runtime.json` and names
`<GAMESERVER_RUNTIME_DIR>\Rustic-Isles.dll` with its hash in the start's `Command::inject`. `os::spawn` creates the
server with `CREATE_SUSPENDED` (inside its job, as before) and calls `dainty_inject::load_and_resume` on that child's
own handle: the same crate, the same held-handle hash check and the same `Sleep(0)`-then-`LoadLibraryW` sequence as
the launcher. On an error the job is terminated before the start fails.

The runtime decides it is a server from `-server` on the process command line in its own `DllMain`. That doesn't
depend on how it was loaded, and its logs and switch files are found next to the game's exe, not the DLL, so loading
it from `.workspace\runtime` changes nothing it reads.

**Wine.** The Director's program is the `dainty-gameserver` wrapper (`dainty-host gameserver`). It gets the DLL and
hash in its environment (`DAINTY_INJECT_DLL`, `DAINTY_INJECT_SHA256`; the Director removes any inherited value
first). `wine::build_launch` then runs:

```text
[xvfb-run ...] wine|proton run <bin>/dainty-inject.exe --dll Z:\...\.workspace\runtime\Rustic-Isles.dll <sha256> -- Z:\...\Dauntless-Win64-Shipping.exe <server arguments>
```

The helper waits for the game and exits with its code, so the Director's readiness, exit and restart handling are
unchanged. Wine starts the game as a child Unix process of the helper in the same process group, so the Director's
SIGTERM and SIGKILL to the group still reach it.

**Shipping the helper.** The root and restricted Dockerfiles build it with `gcc-mingw-w64-x86-64` into the image's
`bin/`. `server-release.yml` (RR3's) doesn't build it yet: the Linux server tarball needs
`cargo build --release --locked -p dainty-inject --target x86_64-pc-windows-gnu` and the exe in `bin/`, as
`launcher-release.yml` already does for the launcher. Until then a Linux release host's servers refuse to start, and
`doctor` says the helper is missing.

**Blocking.** `Launcher::start` is synchronous, so the injection runs on the calling task's thread. It waits for the
runtime's `DllMain` (up to 60 s per remote thread, `dainty-inject`'s limit).

## Hands-on checks (maintainer)

Not run with the game. `cargo test` covers the start plan with a recording launcher, the wrapper's helper line, and,
on Windows, `dbghelp.dll` injected into the test server inside its job (`tests/inject.rs`): with a wrong hash the
start is refused and nothing runs; with the right one the server reaches ready.

### Windows host who also plays

1. After updating the binary, `bin\dainty-host setup` (it writes `GAMESERVER_RUNTIME_DIR`), then
   `bin\dainty-host publish-runtime`. It prints that it removed `winmm.dll` from `Win64`, if the old kit put it there
   (a loader published before c3208d06 and already gone from `.workspace
untime` isn't known; delete that by hand).
   `Win64` holds no `winmm.dll`. Without `setup`, `start` logs that every game server is refused and none starts.
2. `bin\dainty-host start`. The Director's log has no runtime error. Ramsgate reaches ready, headless (no window),
   and stays up; `logs\gameservers\port<N>_ramsgate.log` has the runtime's start lines.
3. Process Explorer on the Ramsgate process lists `Rustic-Isles.dll` from `.workspace\runtime` and no `winmm.dll`.
4. Start a hunt from a client. Its server starts the same way.
5. With the server running, PLAY from the launcher on the same install. The launcher doesn't refuse over a loader,
   and the game reaches the menu.
6. Change one byte of `.workspace\runtime\Rustic-Isles.dll` and stop Ramsgate (Task Manager). The Director logs that
   the runtime was not loaded and the server was ended; no Ramsgate process stays. Run `publish-runtime` again.
7. Put any `winmm.dll` in `Win64` and restart. The Director's start log names it.

### Linux or Docker host

1. After updating, `bin/dainty-host setup`, then `bin/dainty-host doctor`: "inject helper ready at
   .../bin/dainty-inject.exe".
2. `start` (or `docker compose up -d`). Ramsgate reaches ready, headless, and stays up. The game server log's
   command line shows `dainty-inject.exe --dll Z:\...\Rustic-Isles.dll <sha256> -- Z:\...`.
3. If it doesn't, the game server's log has the helper's own `dainty-inject: ...` line. A Unix exit code keeps only
   the low byte, so the exit line shows `code=3` when a DLL wasn't loaded (Wine refused it, or the hash didn't
   match) and `code=2` when the game didn't start through the helper.
4. Start a hunt; stop the stack and check no Wine process is left.
