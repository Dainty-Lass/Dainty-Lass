<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the legacy deploy server (RX3)

The checklist for Dauntless Revived's deploy server, ported for legacy (Dauntless 1.4.4) servers:
`rust/services/deploy` in the legacy-revived submodule (`modules/legacy-revived`, DS6; DR's at `24b7624`, RD5
reversed). This checklist moves there with RV1; its fuzz target (`deploy_input`) left this repository's `rust/fuzz`
with the code, and RV1 carries it over.
`SECURITY_REVIEW.md` indexes it.

How to use it: before legacy servers ship, copy this list into the review, check each item in the code (not from
memory), link the code or test that shows it, and sign at the end with the date and the commit reviewed. An item
that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

The deploy server starts, tracks and stops the 1.4.4 game-server processes of one machine for the legacy metagame
on the same machine: Ramsgate (one process or a pool of cities), the Training Dojo, and hunts on the ports between.
It has **no authentication**. Whoever can call its matchmaking route starts game processes with arguments taken
from the request, so the boundary is "only the metagame on this machine, straight over loopback".

- **In:** HTTP/1.1 on `DEPLOY_BIND_HOST:DEPLOY_PORT` (`127.0.0.1`, 61001 by the host kit's convention):
  `POST /api/matchmaker/handle-matchmaking-for-player` (starts or picks a game server) and `GET /gameservers`
  (the running servers, with the account ids of their expected players).
- **In, from the game:** the ready file (`<id>.ready`, `pid:port`), the city occupancy snapshot
  (`city-occupancy-<pid>.txt`) and the native diagnostic log's `net driver=` lines, all in
  `DEPLOY_GAMESERVER_READY_DIR`.
- **Out, processes:** `DEPLOY_GAMESERVER_BINARY_PATH` with nine or more arguments, without a shell; on Linux that
  binary is `deploy run-game` (Proton or Wine, under `xvfb-run` without a display).
- **Out, HTTP:** the metagame's `GET /reforged/api/ServerStatus` (`DEPLOY_METAGAME_URL`, only with
  `DEPLOY_CITY_OVERFLOW=1`), and other machines' deploy servers through loopback tunnels
  (`DEPLOY_OVERFLOW_DEPLOYSERVER_URL`, `DEPLOY_GERMANY_DEPLOYSERVER_URL`, `DEPLOY_US_DEPLOYSERVER_URL`).
- **Out, SSH (DR's OpenBSD kit, not set up by the host kit):** `deploy ssh-game` runs `ssh -T -o BatchMode=yes
  <user>@<host> <command> <payload>`; `deploy worker <payload>` is the forced command on the other side.

Code: `app.rs` (the two routes and the loopback rule), `matchmaker.rs` (`check_request`, routing a request),
`gameservers.rs` (the launch, the arguments and environment of the game, exits, the watchdog), `readiness.rs`,
`occupancy.rs`, `remote.rs` (tunnels, the metagame URL), `kit.rs` (`run-game`, `worker`, `ssh-game`), `system.rs`
(the OS calls), `settings.rs` (the `DEPLOY_` names), `service.rs` (the listener).

Fuzz target: `deploy_input` in `rust/fuzz` (the peer address and proxy headers, the matchmaking body as `app.rs`
reads it, a worker payload, the occupancy lines). It checks that only a loopback peer without a proxy's headers
passes, that every field `check_request` accepts is one argument the game reads as data (a game path, a hunt id,
an id list without separators), and that a worker payload decodes to 8 to 64 strings without NUL.

## Checklist

### Who can use it

- [ ] The listener binds a loopback address only: `DEPLOY_BIND_HOST` defaults to `127.0.0.1` and anything not
      loopback is a fatal start error (`service.rs`). Test: `a_bind_off_loopback_is_refused`.
- [ ] Both routes answer 403 unless the peer is loopback (`127.0.0.0/8`, `::1`, also IPv4-mapped) and the request
      carries none of `x-forwarded-for`, `forwarded`, `x-real-ip`, `x-forwarded-host`, `x-forwarded-proto`, `via`,
      `x-dauntless-gateway` or `x-reforged-gateway` (`app::is_direct_loopback`). A request the gateway relayed on
      loopback is refused that way. Tests: `anything_relayed_by_a_proxy_gets_403`, `loopback_callers_only`.
- [ ] The gateway never routes to the deploy server's port: its legacy policy names the metagame, content and chat
      upstreams only (`services/gateway`, legacy mode).
- [ ] Any local process can still call it. That is DR's model (the metagame has no secret to show it); it is
      accepted only because a local process can start the game binary itself.

### What reaches the game's command line

- [ ] The game is started with `tokio::process::Command` (no shell), `stdin` closed, its whole environment set
      explicitly (`env_clear`), and on Windows hunts without a console unless `DEPLOY_DR_SHOW_CONSOLE=1`.
- [ ] A request is checked before anything starts, and a bad one answers `400 bad_request` (`check_request`):
      `HuntId` is at most 128 of `[A-Za-z0-9_+]`; `GameArgs` is a string, and for `ISLAND` at most 2048 of
      `[A-Za-z0-9_/.?=+-]` whose map and behemoth are `/Game/...` paths; `ExpectedPlayers` is at most 16 ids of at
      most 64 of `[A-Za-z0-9_-]`; `Region` is one of `main`, `aus`, `ger`, `us`, `mixed` or absent
      (`400 invalid_region`). Tests: `refuses_anything_else_on_a_command_line`, `bad_input_gets_400_and_starts_nothing`.
- [ ] Maps, behemoths and game modes for catalog hunts and Trials come from the built-in tables
      (`node/UndauntedDeployServer/src/vendor/`), never from the request; an unknown hunt id is an error, not a
      path.
- [ ] The game's arguments are fixed in count and order: the game-server key, the port, the map, the behemoth or
      `NO_BEHEMOTH`, the matchmaker hunt or `NO_MM_HUNTID`, `uid:hunt,...` or `NO_EXPECTED_PLAYERS`,
      `DEPLOY_MY_IP:port`, `-EpicPortal -server -nullrhi`. No request field can add an argument.
- [ ] Ports come only from `DEPLOY_PORT_RANGE_BEGIN..=END` (hunts below `END-1`, the Dojo on `END-1`, Ramsgate on
      `END`); a persistent world's port that is already bound is refused rather than shared. Test:
      `refuses_to_spawn_over_a_stale_listener_on_ramsgates_port`.

### Limits

- [ ] Hunts are admitted against the hunt limit (`DEPLOY_MAX_LOCAL_HUNTS`, `DEPLOY_MAX_PENDING_HUNTS`), the CPU
      guard and the memory guard before a process starts; a refusal is `503 capacity_unavailable` with
      `Retry-After: 10`. Starts are queued one at a time and spaced by
      `DEPLOY_SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP`. Tests: `a_capacity_refusal_answers_503_with_retry_after`,
      `a_worker_limits_pending_launches_and_running_hunts`, `low_memory_blocks_new_worlds_before_spawn`.
- [ ] A drain file (`DEPLOY_DRAIN_FILE`) stops new hunts without stopping running ones.
- [ ] Hunts past their lifetime (`DEPLOY_HUNT_MAX_AGE_MINUTES`, empty, silent or never ready) are stopped by the
      watchdog; only processes this server started are ever signalled (`stop_expired_hunt` checks the pid).
- [ ] The request body goes through axum's default 2 MB limit before it is parsed.

### What it reads from the game

- [ ] The ready file counts only as `<digits>:<this port>`; the occupancy snapshot only when at most 64 bytes, fresh
      (2.5 s), newer than the process and naming its own pid; the diagnostic log only from its last 8 KiB and
      when fresh (90 s). A stale or foreign count never proves free seats. Tests:
      `city_snapshots_are_checked`, `reads_the_last_complete_driver_line`.

### Out of the machine

- [ ] Tunnel and metagame URLs must be `http://127.0.0.1:<port>` with no user, path, query or fragment; anything
      else is a configuration error (`remote::tunnel_url`, `remote::metagame_url`). Tests:
      `tunnels_must_be_loopback_and_oceania_stays_retired`, `the_metagame_is_reached_on_loopback_only`.
- [ ] `deploy ssh-game` accepts only `[A-Za-z0-9_.:-]` hosts, `[A-Za-z0-9_.-]` users, `[A-Za-z0-9_./-]` commands
      and an absolute key path, runs `ssh` in batch mode, and passes the game's arguments as one base64url JSON
      payload, never as shell text; the game-server key never appears as an SSH argument. Test:
      `openbsd_sends_one_payload_over_batch_ssh`.

### Secrets

- [ ] `DEPLOY_METAGAME_API_KEY` (the game-server key) is never logged: no log line prints the arguments or the
      environment of a game process.
- [ ] `DEPLOY_CITY_STATUS_KEY` goes only to `DEPLOY_METAGAME_URL` (loopback) as `x-reforged-user-api-key`.

### Lifetime

- [ ] On a stop request (`shutdown` on stdin, Ctrl+C, SIGTERM) it stops every game process it started (asked,
      then forced after 4 s) and restarts nothing. Tests: `shutdown_stops_its_game_servers_without_restarting_ramsgate`,
      `shutdown_interrupts_a_start_in_flight_and_prevents_new_ones`.
- [ ] A game process that fails to start gives its port back and answers 500 (`no_game_server`). Test:
      `a_missing_binary_is_logged_and_gives_the_port_back`.

## Open points

These must be settled, or accepted in writing, before legacy servers ship:

- **The game-server key is on the game's command line**, as DR passes it (argument one, which the internal-server
  DLL reads). Any local user can read a process's command line. The key lets a caller post as a game server to
  the metagame. Moving it to the environment needs a change to `modules/legacy-revived/native/internal-server` as well.
- **The game process inherits every deploy setting** under DR's names (`DR_*` for the DLL and `run-game`, plus
  `METAGAME_API_KEY`, `CITY_STATUS_KEY`), as DR did. The game needs some of them; a narrower list is safer.
- **No caller authentication.** DR has none. A shared secret with the metagame (as the gateway has) would close
  the "any local process" gap; it needs the metagame's side too.
- **Wine/Proton on Linux.** `run-game` finds Proton under the host's Steam folders or Wine on `PATH`. A writable
  folder ahead of the real one on `PATH` would run instead; the host kit should pin `DEPLOY_DR_WINE_BINARY` or
  `DEPLOY_DR_PROTON_BINARY` to an absolute path.

## Fuzzing

From `rust/fuzz`:

```bash
cargo +nightly fuzz run deploy_input -- -max_total_time=86400
```

## Changes from DR

- Every variable is `DEPLOY_<DR name>` and passed to the game process under DR's name (`settings.rs`); no `.env`
  is read from the working directory.
- A non-loopback `DEPLOY_BIND_HOST` is refused; DR only defaulted to loopback.
- `x-reforged-gateway` is refused next to DR's `x-dauntless-gateway`.
- `run-game` forces Wine's `winmm=n,b` (Reforged's runtime loader loads the server DLL, RD9) instead of DR's
  `dxgi=n,b` proxy.
- The metagame's status route and key header follow the rename dictionary (`/reforged/api/ServerStatus`,
  `x-reforged-user-api-key`), at `DEPLOY_METAGAME_URL` instead of a fixed `127.0.0.1:61000`.
- DR's `generate-config` and `signing-keys` subcommands are gone: `reforged-host setup` writes every service's
  settings.

Signed: ________ (date, commit)
