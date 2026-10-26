<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# deploy (legacy servers only)

Dauntless Revived's deploy server (`UndauntedDeployServer`), from DR `rust/services/deploy` at `24b7624`, for
**Dauntless 1.4.4** ("legacy") servers. It starts, tracks and stops the 1.4.4 game-server processes of one machine
for the legacy metagame (`../metagame`) on the same machine: Ramsgate (one process, or a pool of cities), the
Training Dojo, and hunts on the ports between. Reforged's own 1.14.7 servers don't use it; Mystic Paradox's
Director does that job there.

It has no authentication and starts processes, so it listens on loopback only and refuses anything a proxy
relayed. The security checklist is [`docs/security/deploy.md`](../../../docs/security/deploy.md); the fuzz target is
`deploy_input`.

## Running it

```text
deploy [serve] --env-file <file>      the server (the default command)
deploy run-game <game arguments...>   one game server under Proton or Wine (Linux)
deploy worker <payload>               the same from one encoded argument list (an SSH forced command)
deploy ssh-game <game arguments...>   starts the game on a Linux worker over SSH (DR's OpenBSD kit)
```

The same binary linked or copied as `dr-run-game` or `dr-ssh-game` is that launcher. Nothing is read from the
working directory: the settings come from the `--env-file` and the process environment, which wins over it.

## Routes

| Route | What it does |
| --- | --- |
| `POST /api/matchmaker/handle-matchmaking-for-player` | The metagame's matchmaking call (`GameMode`, `GameArgs`, `HuntId`, `ExpectedPlayers`, `Region`, `Overflow`). Starts a hunt, or hands out Ramsgate or the Dojo, and answers `{ "host", "port", "sessionId" }`. `400 bad_request` / `invalid_region` for a request it can't start, `503 capacity_unavailable` with `Retry-After: 10` when full, `500 no_game_server` when the start failed |
| `GET /gameservers` | The running game servers of this machine and its tunnelled workers, with `complete` and the admission `capacity`. Read by the metagame's ServerStatus |

Both answer 403 to a peer that is not loopback, and to any request carrying a proxy's headers (`x-forwarded-*`,
`forwarded`, `x-real-ip`, `via`, `x-dauntless-gateway`, `x-reforged-gateway`).

## Variables

Every variable DR's deploy server read as `NAME` is read here as `DEPLOY_NAME`, and nothing else is renamed. The
game process gets each one back under DR's name (`DEPLOY_DR_GAME_EXE` reaches `run-game` as `DR_GAME_EXE`), with
the system's own variables, because the internal-server DLL and `run-game` read DR's names.

| Variable | Default | Effect |
| --- | --- | --- |
| `DEPLOY_PORT` | required | HTTP port. The host kit's convention is `61001` |
| `DEPLOY_BIND_HOST` | `127.0.0.1` | Listen address; anything not loopback is refused |
| `DEPLOY_LOG_LEVEL` | `info` | `trace`, `debug`, `info`, `warn`, `error`, `fatal`, `silent` |
| `DEPLOY_NODE_ENV` | | `production`: JSON log lines instead of pretty ones |
| `DEPLOY_SERVER_ROLE` | | `database-only`: refuse to start |
| `DEPLOY_GAMESERVER_BINARY_PATH` | required | What starts one game server: `Dauntless-Win64-Shipping.exe` on Windows, `deploy run-game` (linked as `dr-run-game`) on Linux |
| `DEPLOY_METAGAME_API_KEY` | required | The game-server key, the game's first argument; must equal the metagame's game-server key. A secret |
| `DEPLOY_MY_IP` | required | The address players reach the game servers on, handed out with each port |
| `DEPLOY_PORT_RANGE_BEGIN`, `DEPLOY_PORT_RANGE_END` | required | Game ports: hunts from `BEGIN` to `END-2`, the Dojo on `END-1`, Ramsgate on `END` |
| `DEPLOY_SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP` | `0` | Spacing between two game starts |
| `DEPLOY_GAMESERVER_READY_DIR` | a temporary folder | Ready markers and the game's occupancy files |
| `DEPLOY_GAMESERVER_STARTUP_GRACE_MS` | `90000` | How long a start may take to listen (at most 180000; `0` doesn't wait) |
| `DEPLOY_RAMSGATE_POOL` | on | `0`: one Ramsgate on `END` instead of a pool of cities started on demand |
| `DEPLOY_RAMSGATE_CITY_CAPS` | | `10,10,8`: at most that many cities with those caps (each at most 20) |
| `DEPLOY_RAMSGATE_PLAYER_LIMIT` | `28` | With `CITY_OVERFLOW`: players in this city before the worker's is used |
| `DEPLOY_CITY_OVERFLOW` | | `1`: balance Ramsgate with the overflow worker (needs `CITY_STATUS_KEY`) |
| `DEPLOY_CITY_STATUS_KEY` | | An account key the metagame's ServerStatus accepts, for city balancing. A secret |
| `DEPLOY_METAGAME_URL` | `http://127.0.0.1:61000` | The metagame, for city balancing; `http://127.0.0.1:<port>` only. New here: DR had it fixed |
| `DEPLOY_PERSISTENT_WORLD_LIVENESS` | on | `0`: hand out Ramsgate and the Dojo without checking the process still runs |
| `DEPLOY_ENABLE_DOJO` | | `1`: start the Dojo at boot instead of on first use |
| `DEPLOY_MAX_LOCAL_HUNTS` | no limit | Hunts this machine runs at once; `auto`: every hunt port, CPU and memory guards required |
| `DEPLOY_MAX_HUNTS` | no limit | The same on a hunt worker (`HUNT_WORKER=1`) |
| `DEPLOY_MAX_PENDING_HUNTS` | `8` | Hunts waiting to start |
| `DEPLOY_DRAIN_FILE` | `drain.flag` | While this file exists no new hunt starts. Relative to the working directory: set an absolute path |
| `DEPLOY_GAMESERVER_CPU_GUARD` | off | `1`: refuse hunts while CPU is above the limit |
| `DEPLOY_GAMESERVER_CPU_LIMIT_PERCENT`, `DEPLOY_GAMESERVER_CPU_RESUME_PERCENT` | `80`, `65` | The CPU guard's thresholds |
| `DEPLOY_GAMESERVER_MEMORY_GUARD` | on | `0`: no memory check before a start |
| `DEPLOY_GAMESERVER_MIN_FREE_MB` | `3072` | Free memory to keep |
| `DEPLOY_GAMESERVER_STARTUP_MB`, `DEPLOY_GAMESERVER_HUNT_STARTUP_MB` | `1536`, the former | Memory a start reserves |
| `DEPLOY_GAMESERVER_RESERVATION_SECONDS` | `60` | How long that reservation is held |
| `DEPLOY_HUNT_MAX_AGE_MINUTES` | `120` | A hunt is stopped after this long |
| `DEPLOY_HUNT_EMPTY_MINUTES` | `2` | ... or this long without players |
| `DEPLOY_HUNT_SILENT_MINUTES` | `15` | ... or this long without an occupancy line |
| `DEPLOY_HUNT_UNREADY_GRACE_MINUTES` | `5` | ... or this long without listening |
| `DEPLOY_HUNT_STARTUP_GRACE_MINUTES` | none | Extra minutes after a hunt is ready before the empty and silent rules apply |
| `DEPLOY_TRIAL_ROTATION_START` | `2019-07-18T18:00:00.000Z` | When the weekly Trials rotation starts |
| `DEPLOY_HUNT_WORKER` | | `1`: this machine is a hunt worker for another one |
| `DEPLOY_WORKER_RAMSGATE`, `DEPLOY_WORKER_DOJO` | | `1`: a worker that also runs Ramsgate or the Dojo |
| `DEPLOY_OVERFLOW_DEPLOYSERVER_URL` | | The overflow worker's deploy server, through a loopback tunnel |
| `DEPLOY_OVERFLOW_AFTER_HUNTS` | `4` | Local hunts before new ones go to the overflow worker |
| `DEPLOY_GERMANY_DEPLOYSERVER_URL`, `DEPLOY_US_DEPLOYSERVER_URL` | | Regional workers, through loopback tunnels |
| `DEPLOY_GERMANY_PUBLIC_HOST`, `DEPLOY_US_PUBLIC_HOST` | | The address a regional worker must answer with |
| `DEPLOY_US_RESERVED_HUNTS`, `DEPLOY_AUS_RESERVED_HUNTS` | `0` | Hunt slots kept for parties that chose that region |

What the game process reads (set them with the prefix; the game sees DR's name):

| Variable | Read by | Effect |
| --- | --- | --- |
| `DEPLOY_DR_SHOW_CONSOLE` | deploy, the DLL | `1` shows every game console, `0` hides them; unset hides hunts only |
| `DEPLOY_DR_SERVER_MAX_FPS` | the DLL | The server frame limit |
| `DEPLOY_DR_GAME_EXE` | `run-game` | Absolute path of `Dauntless-Win64-Shipping.exe` |
| `DEPLOY_DR_PROTON_BINARY`, `DEPLOY_DR_WINE_BINARY` | `run-game` | Proton or Wine to use; without either, the newest Proton under Steam, then `wine64`/`wine` on `PATH` |
| `DEPLOY_DR_PROTON_COMPAT_DATA` | `run-game` | Proton's prefix (`~/.local/share/dauntless-revived/proton`) |
| `DEPLOY_DR_WINEPREFIX` | `run-game` | Wine's prefix (`~/.local/share/dauntless-revived/wineprefix`) |
| `DEPLOY_DR_XVFB`, `DEPLOY_DR_XVFB_RUN` | `run-game` | `0`: no `xvfb-run` without a display; another `xvfb-run` |
| `DEPLOY_WINEDEBUG`, `DEPLOY_WINEDLLOVERRIDES` | `run-game` | Passed to Wine; `winmm=n,b` is always added |
| `DEPLOY_DR_WORKER_HOST`, `_USER`, `_COMMAND`, `_KEY`, `_SSH` | `ssh-game` | DR's OpenBSD kit only |

The deploy server sets `GAMESERVER_READY_DIR`, `DR_SERVER_READY_FILE` and `DR_RAMSGATE_POOL` for each game itself.

## The game folder

The deploy server names no DLL. The 1.4.4 game folder holds `UndauntedInternalServer.dll` next to
`Dauntless-Win64-Shipping.exe`, loaded by Reforged's runtime loader (`winmm.dll` from
[`native/runtime-loader`](../../../native/runtime-loader), with a `mystic_loader.ini` naming the DLL), never by DR's
`dxgi.dll` proxy, which must not be left there (RD9; [`native/internal-server`](../../../native/internal-server)'s
README). Under Wine `run-game` forces `winmm=n,b` so the native loader is used, where DR forced `dxgi=n,b`. The DLL
is built from `native/internal-server` into `x64\Release\UndauntedInternalServer.dll`. Putting the loader, its ini
and the DLL there, and pinning them, is the host kit's job.

On Windows `DEPLOY_GAMESERVER_BINARY_PATH` is the game executable itself. On Linux it is `dr-run-game`, which runs
the same arguments under Proton or Wine.

## Changes from DR

- `DEPLOY_` names (above), read through `settings.rs`; no `.env` from the working directory.
- A non-loopback `DEPLOY_BIND_HOST` is refused.
- The metagame call follows the migration guide's rename dictionary: `/reforged/api/ServerStatus` with
  `x-reforged-user-api-key`, at `DEPLOY_METAGAME_URL`.
- `x-reforged-gateway` is refused next to `x-dauntless-gateway`.
- `generate-config` and `signing-keys` (DR's `kitconfig.rs`) are gone: `reforged-host setup` writes the settings.
- `run-game` forces `winmm=n,b` (the runtime loader) instead of DR's `dxgi=n,b`.
- The hunt tables are under `node/UndauntedDeployServer/src/vendor/`, as the metagame keeps its own.
