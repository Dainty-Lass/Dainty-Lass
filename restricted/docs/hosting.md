# Hosting a server on Windows

One PC runs the whole server: the gateway, the 1.14.7 Rust backend and Director (`isles-1-14-7`), the content
server, the admin API, the SQLite (or Postgres) store and the game servers. A **private** server is reachable by
friends on your tailnet; a **public** one by anyone with an invite, over TLS. The host kit, `dainty-host`, does
the setting up from a terminal; **Dainty Lass Manager** (`dainty-manager`, or the client launcher's Host page)
drives the same server and setup with a graphical interface.

Linux: [hosting-linux.md](hosting-linux.md). Docker: [hosting-docker.md](hosting-docker.md).

## What you need

- Windows 10 or 11, and **your own Dauntless 1.14.7** install (the folder with `Archon` in it), or an Epic
  account whose library has Dauntless, from which `install-game` installs it ([below](#getting-the-game-from-epic)).
- The **server release** (`DaintyLassServer-<version>-windows-x64.zip`), unpacked anywhere; `bin\dainty-host.exe`
  is the kit. Or a checkout of this repository built from source. No Node.js or MongoDB is needed on the default
  path: the 1.14.7 backend, Director, gateway, content server, allowlist helper and admin API are compiled into
  `dainty-host` and store player data in `.workspace\config\dainty-lass.db` (SQLite) or Postgres (`[database]`).
- **Visual Studio 2022** with Desktop development with C++, and **Dumper-7**, to generate the SDK and build
  the game runtime from your own game ([sdk-and-game-data.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/sdk-and-game-data.md)). Players get the runtime
  you build; nobody else's works with your server's build.
- Private: **Tailscale** on your PC and your friends' PCs. Public: a public IPv4 or a DNS name, and TCP 443
  and UDP 8770-8777 open on your router and firewall.

## Setting up

A private server, from the release folder:

```powershell
bin\dainty-host setup --mode private --name "Friday Hunts" --game "D:\Games\Dauntless"
```

`--host` defaults to this PC's Tailscale address. A public one:

```powershell
bin\dainty-host setup --mode public --host play.example.org --game-address 203.0.113.10 --name "Friday Hunts" --game "D:\Games\Dauntless"
```

`setup` writes `.workspace\config\host.toml` (every setting, with a comment each), `secrets.env` (made
once, never shown) and the service `.env` files, and in public mode makes the gateway's certificate.
**It never remakes that certificate**: every invite's `fp` depends on it. Run `setup` again with different
options to change a setting, or edit `host.toml` and run `setup` with none.

No 1.14.7 folder yet? `install-game` fetches one from your own Epic library
([below](#getting-the-game-from-epic)) and points `game_dir` at it.

Then the SDK, the runtime and the game data ([sdk-and-game-data.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/sdk-and-game-data.md)):

```powershell
bin\dainty-host gamedata                  # from your CatalogExporter export in modules\rustic-isles\reference\1.14.7\Items_Analysis
bin\dainty-host publish-runtime           # the Rustic-Isles.dll and winmm.dll you built
bin\dainty-host doctor                    # what is still missing or wrong
```

## Running it

```powershell
bin\dainty-host start                     # every service, until Ctrl+C
bin\dainty-host invite --for "Alex"       # an invite link to send
bin\dainty-host invite --list
```

`start` runs one process tree and stops all of it on Ctrl+C. **Dainty Lass Manager** (`dainty-manager`, built
with the `server-gui` feature) runs the same server in-process from its Host page. Logs are in `.workspace\logs` (the game servers' in `logs\gameservers`).

| Command | What |
| --- | --- |
| `setup` | Set up or change the server (see the options below) |
| `start` | Run every service |
| `invite` | Make an invite (`--uses N`, `--infinite`, `--for who`), or `--list` them |
| `doctor` | Check the game, the SDK, the builds, the network, the tools and the services |
| `gamedata` | Generate the game data from your export (`--examples` for a start-up test only); `--export` and `--import` move it as a pack |
| `publish-runtime` | Publish the runtime for players (`--from <folder>` for DLLs built elsewhere) |
| `install-game` | Install Dauntless 1.14.7 from your own Epic library for the game servers (`--version`, `--dir`, `--code-file`; `--sign-out`) |
| `preset` | Keep your game data as a preset (`--create`), or apply one (`--apply`) |
| `multipliers` | XP and loot multipliers for a private test server (`--xp 5 --rewards 5`, `--off`) |
| `update` | Install a newer server release and run `setup` again (`--check` only compares) |
| `backup` | Dump the database |
| `network` | The cluster CA and node certificates of a network, and joining an existing host database (`ca-init`, `issue-node-cert`, `list`, `status`, `join`; see [A network of servers](#a-network-of-servers)) |
| `history` | What changed in the server's data, by whom and when (`--actor`, `--account`, `--since`, `--until`); `history verify` checks the log's hash chain. See [Undoing a change](#undoing-a-change) |
| `revert` | Undo what an actor, an account or a time range changed: a dry run first, then `--apply <plan id>` |
| `db` | Migrate or verify between MongoDB and the SQLite/Postgres store (`migrate`, `verify`), or export the store back to MongoDB for one-release rollback (`export-mongo`) |
| `legacy` | `legacy import <revived\|offline-save\|reforged> [args]` runs [dainty-legacy](../../crates/dainty-legacy)'s importer for an old install: the `legacy` module `[[modules.external]]` pins in `host.toml`, or `--program <path>` before the first setup. In the console, `legacy import` runs it as `module run legacy import-<kind>` |


## Getting the game from Epic

The game servers run your own Dauntless. If you own it on Epic, the host kit installs it from Epic's servers, as
the launcher does for players (Dainty Lass never hosts or mirrors game files):

```powershell
bin\dainty-host install-game                       # 1.14.7, into .workspace\game\1.14.7
bin\dainty-host install-game --dir D:\Games\Dauntless   # into D:\Games\Dauntless\1.14.7
```

It prints Epic's login address. Open it in a browser on any computer, sign in, and Epic shows a page of text with an
`"authorizationCode"`. Paste the code (or the whole page) when asked, or save it in a file and pass `--code-file`
(the host never takes the code, or any token, as a command line option). The code is spent by that one sign-in.
What the host keeps is the session it opens, in `.workspace\config\epic-session.env` beside `secrets.env` (owner-only
on Linux): the next run, a resume or another version needs no code. It is worth your Epic account until Epic ends
it (weeks), so **`install-game --sign-out` when you are done**; it ends the session at Epic and deletes the file.

Nothing is downloaded for an account without Dauntless in its library. The manifest is checked against a pinned
SHA-1 before it is read, every file against the manifest, and the executable against its pinned SHA-256, so what
lands is the Epic build and nothing else. A run that is stopped (Ctrl+C, a lost connection, a reboot) continues
where it left off when you run the same command again. 1.14.7 is about 15 GB.

It goes into a folder of its own under `--dir` (`.workspace\game` unless given). Installing also sets `game_dir` in `host.toml` to its folder and rewrites the env files, so the
Director's game servers run from it: restart the server for that to take effect. Without a `host.toml` yet, it
tells you the `--game` to give `setup`.

## Undoing a change

Every change to the server's data is in a change log with who made it, when, and the row before and after
([DD23](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/dainty-lass-plan.md)). A partner who turns on you, a bot that deletes the wrong accounts or an admin key
used by someone it wasn't issued to can be seen and undone from the host, with no key:

```powershell
bin\dainty-host history --actor admin:58ef5419 --since 2d       # what that key did in the last two days, newest first
bin\dainty-host history --account <account id>                  # everything that touched one player's rows
bin\dainty-host revert --actor admin:58ef5419 --since 2d        # a dry run: prints what would change, and a plan id
bin\dainty-host revert --actor admin:58ef5419 --since 2d --apply <plan id>
bin\dainty-host history verify                                  # is the log itself intact?
```

- **Who and when.** `--actor` is `admin:<key id>` (the ids are in `admin key list`; a bare id works, `owner` is
  `DAINTY_ADMIN_API_KEY`), `host:console`, `player:<account>` or `system:<task>`. A time is RFC 3339
  (`2026-10-27T10:30:00Z`), a date or a date and time in UTC (`2026-10-27`, `2026-10-27T10:30`) or how long ago
  (`90m`, `6h`, `2d`, `1w`). `--since` counts, `--until` doesn't. `history` shows 50 changes a page
  (`--limit`, up to 1000) and prints the `--cursor` for the next, older page.
- **A dry run changes nothing.** It lists the rows a revert would change (a deleted row brought back, a changed row
  given its earlier values, an added row removed), the rows that **stay as they are** and why, and the rows that
  are already right. `revert` with no `--apply` is a dry run. A revert needs an actor, an account or a start
  time; there is no "revert everything".
- **A row changed since stays.** If someone else changed a row after the selection did (the owner fixed it by
  hand, a player played on), the revert doesn't overwrite it: the dry run lists it as "changed since by
  admin:owner at ..." and the apply leaves it alone.
- **Apply takes the plan's id** and the same options. It plans again and runs only if the plan is still the one
  you saw; otherwise it says "the history changed since the dry run: run it again" and writes nothing. It is
  logged as `host:console`, so it shows in `history` and can be reverted the same way.
- **One revert, several databases.** The game's data, the admin API's keys and chat's messages are separate
  databases on SQLite (one on Postgres). A revert is atomic in each, and if a later one fails the earlier ones
  stay reverted and the message names them.
- **`history verify`** walks each log's hash chain from its start and names the first entry that was edited or
  removed. It exits 1 when one is broken. The head it prints is what to keep somewhere off the server.

The same commands are in the `serve` console and the dashboard (`history --actor ...`, `revert ... --apply ...`;
a long list is cut off at 200 lines, so read a mass revert in a terminal), in **Dainty Lass Manager** (`dainty-manager`, History tab) and in the admin API for the owner's key ([admin-api.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/admin-api.md)). In Docker:
`docker compose run --rm host history --actor admin:58ef5419`.

## Settings

`setup`'s options and the `host.toml` keys they write. A secret (the admin key, the gateway secret, the game
servers' key) lives only in `secrets.env`.

| Option | `host.toml` | What |
| --- | --- | --- |
| `--mode private\|public` | `mode` | Friends over Tailscale, or the internet |
| `--host` | `host` | The Tailscale IP, or the public IPv4 or DNS name |
| `--game-address` | `game_address` | The IPv4 the game servers announce (needed on a DNS name) |
| `--name` | `name` | The server's name, as players see it |
| `--game` | `game_dir` | The Dauntless folder |
| `--registration INVITECODE\|OPEN\|NONE` | `registration` | Who may register |
| `--share` | `share_url` | A Tailscale share link put in private invites |
| `--mongo-uri` | `[mongo]` `uri` | Existing MongoDB URI `db migrate`, `db verify` and `db export-mongo` default to (and a `--features paradox` rollback build's; the default server uses `[database]`) |
| `--max-hunts 1-10` | `max_hunt_servers` | Hunts at once; the UDP range widens below 8770 past six |
| `--xp`, `--rewards`, `--no-multipliers` | `[multipliers]` | Private test servers only |
| `--public-port` | `public_port` | Another port than 443 |
| (edit `host.toml`) | `dashboard_url` | The staff dashboard ([dainty-lass `ui/`](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/ui), dainty-dash), written to `gateway.env` as `CHAT_DASHBOARD_URL` with `DASHBOARD_INGEST_KEY`. Chat takes its filter every 30 s and applies it before delivery (flag, mask, or block, which the sender sees as sent), sends it every room line and whisper with the verdict, and files `/report <player> [category] <what happened>` with it. Left out: chat as before, and `/report` answers that reports are not set up |
| `--server-fps`, `--idle-fps`, `--training-on-demand`, `--training-idle-minutes`, `--verbose-game-logs`, `--memory-guard`, `--min-free-memory-mb`, `--hunt-memory-mb` | `[performance]` | What the game servers cost in CPU and RAM |
| `--wine`, `--proton`, `--xvfb`, `--interface` | `[linux]` | Linux only ([hosting-linux.md](hosting-linux.md)) |
| `--fresh-start` (or edit `host.toml`) | `[database]` `engine`, `postgres_host`, `postgres_port`, `postgres_user`, `postgres_database`, `allow_remote`, `fresh_start` | The Rust services' database: `sqlite` (the default, `dainty-lass.db` in `.workspace\config`) or `postgres` (below); `--fresh-start` / `fresh_start = true` starts with an empty database although `.workspace\mongodb` still holds unmigrated data (MongoDB is never modified) |
| (edit `host.toml`) | `[guard]` `ban_limit`, `ban_window_minutes`, `ban_guard`, `revert_limit`, `revert_guard`, `revoke_guard`, `delay_minutes`, `expire_hours` | When a destructive or bulk admin action waits for a delay or a second operator (below). Left out: ten bans an hour from one key, a revert of over a hundred changes and the revoking of an admin or owner key each wait ten minutes; everything smaller runs at once |
| (edit `host.toml`) | `[history]` `ship_to_peer` | Ship this node's change log and database snapshots over `Service::LogShip` (`/__node/log_ship`) to a peer node in `[[network.nodes]]` (DH7, below). Left out: off |
| (edit `host.toml`) | `[switches]` `reforge`, `mastery_persistence`, `bounty_persistence`, `cooldown_persistence`, `account_erasure`, `log_purge`, `rate_limit`, `rate_limit_per_minute`, `gameserver_rate_limit_per_minute`, `host_rate_limit_per_minute` | Backend persistence, rule and request-budget switches written to `backend.env` (`RUSTIC_ISLES_REFORGE`, `RUSTIC_ISLES_MASTERY_PERSISTENCE`, `RUSTIC_ISLES_BOUNTY_PERSISTENCE`, `RUSTIC_ISLES_COOLDOWN_PERSISTENCE`, `RUSTIC_ISLES_ACCOUNT_ERASURE`, `RUSTIC_ISLES_LOG_PURGE`, `RUSTIC_ISLES_RATE_LIMIT`, `RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE`, `RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE`, `RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE`) and forwarded to `isles-1-14-7`'s `[switches]`. Left out: the module's defaults |
| (edit `host.toml`) | `[modules]` `compiled`, `[[modules.external]]` `id`, `path`, `sha256`, `args`, `autoload`, `[modules.<id>.<section>]` | The game modules and plugins to load (below). Left out: every compiled-in one |
| (edit `host.toml`) | `[[modules.external]]` `restart` | `true` starts the module again when its process dies: after 1 s, doubling to 60 s, back to 1 s once it has run 10 minutes; five failures in 10 minutes leave it failed with the last reason. Each try is a `module.restarted` event. Left out: `false`, a dead module stays failed until it is loaded again |
| (edit `host.toml`) | `[services.<name>]` `library`, `sha256` (and `serves` for a name the server doesn't know) | A service loaded as a library in place of the one built into the binary: `content`, `chat` or `gateway`, from the release's `plugins/` folder, pinned by its SHA-256 (below). It runs on its own env file (`chat` on `gateway.env`), and a `gateway` library takes the allowlist's place too. Left out: every service built in, as before |
| (edit `host.toml`) | `[ports]` `gateway`, `backend`, `director`, `qos`, `content`, `admin`, `allowlist`, `chat` | The TCP ports the server listens on, for a machine where one is taken. `gateway` is a private server's, the port its invites name; a public one's is `public_port`. The others listen on 127.0.0.1. Each needs its own port, and none may be the relay's, MongoDB's or a game server's. On Windows, setup and start move a port Windows has lent out and write the move here ([Ports Windows holds](#ports-windows-holds)). `doctor`, `invite` and the launcher's Host page and invite list follow it. Left out: 61000, 41000, 41001, 41010, 41002, 41003, 41005 and 41006 |
| (edit `host.toml`) | `[env.<service>]` `KEY = value` (`backend`, `director`, `content`, `gateway`, `admin`, `allowlist`) | Extra lines for that service's env file, for a variable with no setting of its own (`GATEWAY_GAME = "reborn"`). Secrets, ports, bind addresses, `AUTH_*`, the server mode, the multipliers and any variable `setup` already writes are refused, and `setup` stops on one rather than skip it; `doctor` lists the ones in use. Left out: nothing extra |
| (edit `host.toml`) | `[gateway.limits]`, `[gateway.rate]`, `[gateway.timeouts]`, `[gateway]`, `[content]`, `[allowlist]`, `[backend]`, `[director]` | The services' limits, timeouts and log levels ([below](#service-tunables)). `setup` checks them with each service's own rules and stops on a bad one. Left out: each service's default |
| (edit `host.toml`) | `[network]` `node`, `region`, `leader`, `wrong_node_policy` | This server's name, region and optional directory leader in a network, and what a join that reaches the wrong node gets: `refuse` (the default) or `redirect` to the node that owns it (DN6). Left out: a server on its own |
| (edit `host.toml`) | `[network]` `failover`, `heartbeat_ms`, `election_timeout_ms` | `failover = true`: the nodes elect the directory leader among themselves and elect another when it goes down, and a registration is acknowledged only once a majority of the nodes holds it; leave `leader` out. Run three or five nodes. The timings default to 1000 and 5000 ms ([distributed-mode.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/distributed-mode.md)). Left out: the fixed leader |
| (edit `host.toml`) | `[logs]` `format`, `color`, `gelf_url` | The console log: `text` (Docker-like lines, `service \| time LEVEL module: message key=value`) or `json`; color `auto` (on a terminal, unless `NO_COLOR`), `always` or `never`; and a GELF collector (Graylog) every record is shipped to, `https://`, or `http://`/`tcp://` only to this machine or a private network ([logging.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/logging.md)). Left out: colored text, nothing shipped |
| `--discord-bot-token` (the token on stdin), `--no-discord`, `--discord-guild <id>` (repeat it), `--discord-application-id`, `--discord-dashboard-url` | `[discord]` `enabled`, `guild_ids`, `application_id`, `dashboard_url` | The Discord bot ([below](#the-discord-bot)), written to `discord.env`: `/key` and `/status` in the guilds `guild_ids` lists (left out: every guild the bot is in), `application_id` checked against the token's, and the dashboard asked whether signups are on (left out: `dashboard_url`). `--discord-bot-token` turns it on, `--no-discord` off. Left out: no bot |
| (edit `host.toml`) | `[events]` `retention_days` | How many days the server keeps the game modules' events (a game server up or down, a module restarted, `events.lagged` when the recorder fell behind) in its database: `events.db` beside the game's on SQLite, the server's Postgres database otherwise. 0 to 3650; 0 keeps every event. Left out: 90 days |
| (edit `host.toml`) | `[split]` `role`, `peer`, `open_game_ports` | This server as one half of two: `metagame` (backend, content, admin) or `gameservers` (the Director and its game servers), the other half's node id in `[[network.nodes]]`, and for a public game half its open game ports ([split-roles.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/split-roles.md)). Left out: one server runs both |
| (edit `host.toml`) | `[[network.nodes]]` `id`, `region`, `host`, `port`, `fingerprint`, `link` | The other nodes of the network: the only ones this node answers, and the ones it calls (below). `link` (`host:port`) is where node links dial instead of `host`: a back channel such as the peer's Tailscale address, which its node certificate must carry too. Left out: no peers, and the server works as one outside a network |

### Service tunables

Every key is optional: left out, the service keeps its own default. Each goes to that service's env file as the
variable shown, and `setup` runs the gateway's, content's and the allowlist helper's own settings check on the
result, so a value out of range stops `setup` with that service's message and the `host.toml` key.

```toml
[gateway.limits]
max_connections = 4096

[gateway.rate]
ws = { burst = 40, per_minute = 24 }

[backend]
log_level = "debug"
```

| `host.toml` key | Variable | Values | Default |
| --- | --- | --- | --- |
| `[gateway.limits]` `max_body_bytes` | `GATEWAY_MAX_BODY_BYTES` | 1024–67108864 | 131072 |
| `[gateway.limits]` `max_connections` | `GATEWAY_MAX_CONNECTIONS` | 1–100000 | 2048 |
| `[gateway.limits]` `max_connections_per_ip` | `GATEWAY_MAX_CONNECTIONS_PER_IP` | 1–100000 | 128 |
| `[gateway.rate]` `general`, `content`, `register`, `token`, `ws`, `connect` | `GATEWAY_RATE_GENERAL` … `GATEWAY_RATE_CONNECT` | `{ burst = <1–1000000>, per_minute = <0–1000000> }` | general 300/180, content 600/600, register 5/0.2, token 10/1, ws 20/12, connect 200/300 |
| `[gateway.timeouts]` `handshake_ms`, `headers_ms` | `GATEWAY_HANDSHAKE_TIMEOUT_MS`, `GATEWAY_HEADERS_TIMEOUT_MS` | 1000–120000 | the gateway's |
| `[gateway.timeouts]` `request_ms`, `keep_alive_ms` | `GATEWAY_REQUEST_TIMEOUT_MS`, `GATEWAY_KEEPALIVE_TIMEOUT_MS` | 1000–600000 | the gateway's |
| `[gateway.timeouts]` `idle_ms`, `upstream_idle_ms` | `GATEWAY_IDLE_TIMEOUT_MS`, `GATEWAY_UPSTREAM_IDLE_TIMEOUT_MS` | 1000–3600000 | the gateway's |
| `[gateway.timeouts]` `ws_idle_ms` | `GATEWAY_WS_IDLE_TIMEOUT_MS` | 1000–86400000 | the gateway's |
| `[gateway.timeouts]` `shutdown_grace_ms` | `GATEWAY_SHUTDOWN_GRACE_MS` | 0–300000 | the gateway's |
| `[gateway]` `allowlist_refresh_seconds` | `GATEWAY_ALLOWLIST_REFRESH_SECONDS` | 5–540 (public only) | 60 |
| `[content]` `max_streams_per_account` | `CONTENT_MAX_STREAMS_PER_ACCOUNT` | 1–64 | 4 |
| `[content]` `max_streams_total` | `CONTENT_MAX_STREAMS_TOTAL` | 1–1024 | 48 |
| `[content]` `auth_cache_seconds` | `CONTENT_AUTH_CACHE_SECONDS` | 0–3600 | 300 |
| `[content]` `news_url` | `CONTENT_NEWS_URL` | an `https://` URL | unset: the news file |
| `[allowlist]` `ttl_seconds` | `ALLOWLIST_TTL_SECONDS` | 1–86400 (public only) | 600 |
| `[allowlist]` `min_interval_ms` | `ALLOWLIST_MIN_INTERVAL_MS` | 50–600000 | 3000 |
| `[allowlist]` `max_entries` | `ALLOWLIST_MAX_ENTRIES` | 1–1000 | 256 |
| `[allowlist]` `nft_table` | `ALLOWLIST_NFT_TABLE` | an nftables table name (Linux) | `dainty_lass` |
| `[backend]` `log_level` | `LOG_LEVEL` | `debug`, `info`, `warn`, `error` | `info` |
| `[backend]` `transaction_max_commit_ms` | `MONGODB_TRANSACTION_MAX_COMMIT_TIME_MS` | 1–600000 (`--features paradox` builds) | 15000 |
| `[backend]` `hunt_go_now_min_ms`, `hunt_go_now_max_ms` | `HUNT_GO_NOW_MIN_DELAY_MS`, `HUNT_GO_NOW_MAX_DELAY_MS` | 1–600000, min no more than max | 1, 1000 |
| `[backend]` `xmpp_capture` | `REALTIME_XMPP_CAPTURE` | `true`, `false` | `false` |
| `[director]` `log_level` | `LOG_LEVEL` | `debug`, `info`, `warn`, `error` | `info` |
| `[director]` `gameserver_startup_gap_seconds` | `SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP` | 0–600 | 3 |
| `[director]` `gameserver_ready_timeout_ms` | `GAMESERVER_READY_TIMEOUT_MS` | 1000–600000 | 30000 (90000 on Linux) |

### Ports Windows holds

Windows lends blocks of 100 ports from its dynamic range (49152 to 65535) to Hyper-V, WSL and Docker, and a
program can't listen on a lent port: the start fails with `os error 10013`. The blocks move on every reboot.
`netsh interface ipv4 show excludedportrange protocol=tcp` lists them.

The services that listen on 127.0.0.1 sit below the range (41000 to 41010), so this only touches a private
server's gateway, whose default is 61000, or a port you set in `[ports]` yourself. On Windows, `dainty-host setup`
and every start read the lent ranges and move each port of the server's that falls inside one, writing the move
to `[ports]` so it stays put after the next reboot:

- a private server's gateway goes to 41080, or the next port free above it;
- any other port goes back to its default, or the next port free above that.

Each move is printed (`- ports.gateway: Windows has lent 61000 out ..., moved to 41080`). **Invites name the
gateway's port**, so after the gateway moves, make new invites with `dainty-host invite` and send them again; the
old ones still name 61000. To keep 61000 instead, reserve it before Windows lends it, from an administrator
PowerShell, then remove `gateway` from `[ports]` and run setup again:

```powershell
net stop winnat
netsh int ipv4 add excludedportrange protocol=tcp startport=61000 numberofports=1
net start winnat
```

`net stop winnat` drops WSL's and Docker's networking for a moment.

### Environment variables override host.toml

A variable set in the environment `dainty-host start` runs in wins over what `setup` wrote, when the service
reads it: a line of its env file, or a variable of its table above (`GATEWAY_RATE_WS=40,24`). Every override is a
warning in the console and the log each time the service starts, naming the `host.toml` key it beats, and
`doctor` lists them under `overrides`. A secret, an `AUTH_*` variable, the server mode, the executable hash, the
registration mode or a multiplier can be overridden too, but its value is never shown. Other variables pass
through as before.

`PORT`, `BIND_HOST`, `NODE_ENV`, `LOG_LEVEL`, `MY_IP` and `BACKEND_URL` are set for everything on many machines,
so they are ignored, with a warning: override them for one service as `DAINTY_<SERVICE>_<NAME>`, such as
`DAINTY_BACKEND_LOG_LEVEL=debug` or `DAINTY_DIRECTOR_LOG_LEVEL=warn`. `setup` checks the services' settings
with the overrides applied, so run it in the same environment as `start`.

### News and session supplies

Two more files in `.workspace\config` are yours to edit, both TOML. `setup` turns an older install's
`news.json` and `session-supplies.json` into them once, and keeps the old file as `.bak`.

`news.toml` is what the launcher shows, newest first. Content reads it again within a few seconds of a change.
A body over several lines goes between triple quotes:

```toml
[[items]]
date = 2026-11-08
title = "Weekend event"
body = """
Double loot from Friday.
Bring friends.
"""
```

`session-supplies.toml` tops up what an account holds at each sign-in, by catalog id (0 to 1000000). The
backend reads JSON, so `setup` writes it into `.workspace\generated\session-supplies.json`: run `setup` after
editing. Without the file, nobody is topped up.

```toml
[accounts.alice]
Consumable_Potion = 10
```

### Secrets (`secrets.env`)

`setup` creates `.workspace/config/secrets.env` once (`SECRET_KEYS` in dainty-lass's `crates/dainty-hostkit/src/hostconfig.rs`) and never rotates an existing secret when `setup` runs again:

| Variable | Copied to | What |
| --- | --- | --- |
| `GATEWAY_SECRET` | `gateway.env` | Authenticates the gateway to its upstreams |
| `ALLOWLIST_SECRET` | `gateway.env`, `allowlist.env` | Authenticates the gateway to the allowlist helper |
| `DAINTY_ADMIN_KEY` | `backend.env`, `admin.env` | The backend's `/dainty/api/*` admin key |
| `GAMESERVER_API_KEY` | `backend.env` (`GAMESERVER_API_KEYS`), `director.env` (`METAGAME_API_KEY`) | Shared key between the game servers and the backend |
| `API_KEY_HASH_SECRET` | `backend.env` | HMAC secret for hashing account API keys at rest |
| `RUSTIC_ISLES_METRICS_TOKEN` | `backend.env` | Bearer token for the backend's `/metrics` endpoint |
| `AUTH_SIGNING_PRIVKEY_B64`, `AUTH_SIGNING_PUBKEY_B64` | `backend.env` | 4096-bit RSA keypair (PKCS#8 private key and SPKI public key, base64-encoded PEM) that signs and verifies the auth service's RS256 access tokens |
| `DAINTY_ADMIN_API_KEY` | `admin.env` | The owner's bearer key for the admin GraphQL API ([admin-api.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/admin-api.md)) |
| `RUSTIC_ISLES_MANAGER_OPERATOR_SECRET` | `backend.env`, `admin.env` | HMAC secret signing the admin API's `/admin/v1` requests to the backend |
| `DAINTY_DB_PASSWORD` | `admin.env` (when Postgres is enabled) | Password for the Postgres role when `[database]` uses `engine = "postgres"` |
| `DASHBOARD_INGEST_KEY` | `gateway.env` (`CHAT_DASHBOARD_INGEST_KEY`, when `dashboard_url` is set) | What chat sends the staff dashboard with: set the dashboard's own `DASHBOARD_INGEST_KEY` to the same value |
| `DISCORD_BOT_TOKEN` | `discord.env` (when `[discord]` is on) | The Discord bot's token. Setup never makes it: `setup --discord-bot-token` reads it from stdin ([the Discord bot](#the-discord-bot)) |

The `RUSTIC_ISLES_*` names were `MYSTICPARADOX_*` before the Rustic Isles rename. `setup` moves the two secrets to
their new names in `secrets.env` with the same values, and writes only the new names into the env files; the backend,
the Director and the admin API still read an old name for one release, with a warning.

### The database engine

Left out, `[database]` means SQLite: one file, nothing to install. A dedicated host can pick Postgres:

```toml
[database]
engine = "postgres"
# postgres_host = "127.0.0.1"   # 5432, role dainty, database dainty when left out
```

Postgres is yours to run: make a role and a database for it, with the role's password set to
`DAINTY_DB_PASSWORD` from `secrets.env` (setup makes it once), then run `setup` again. The connection is not
encrypted, so `setup` refuses a Postgres server on another machine unless `allow_remote = true`, which is only for a
network you trust.

Every Rust service and module repository (`isles-1-14-7`, `admin`, `network`, `chat`) runs on `store`'s `MultiConnection` over SQLite or Postgres with identical behaviour and portable
migrations. When a host switches from SQLite to Postgres (or back), `store db move` copies every table and the
hash-chained change log (`change_log`) across engines and verifies that row counts and contents match.

### Off-server history bucket and restore

In addition to peer log shipping (`[history] ship_to_peer`, below), a server can stream its change-log segments and
periodic database snapshots to an S3-compatible bucket with object lock (`history-ship`, DD23 layer 2: AWS S3,
Backblaze B2, MinIO, Cloudflare R2):

- **Write-only server credentials.** Create the bucket with object lock enabled (`S3Bucket::create_locked` in
  `crates/dainty-history-ship`, retention mode `COMPLIANCE` or `GOVERNANCE`) using the owner's credentials off the
  server, and give the server a write-only key (`s3:PutObject` only) in `secrets.env`. Even an attacker or rogue
  operator with root on the server cannot delete or overwrite already-shipped segments or snapshots.
- **Restoring to a point in time.** `history_ship::restore_to` downloads the latest snapshot at or before the
  chosen `--to <RFC 3339>` timestamp, replays every shipped change-log segment up to that moment into a target
  SQLite or Postgres database, verifies every entry's SHA-256 hash chain (`history verify`) and any out-of-band
  `RestoreOptions::trusted_head`, and refuses unverified tail entries outside `trust_window`
  ([security/history.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/security/history.md)).
- **Shipping lag alerts.** Both the server status page and the admin API surface a warning whenever log shipping
  falls behind the local change log head.

### Realms and the TURN relay

On Dauntless 1.14.7 (`isles-1-14-7` with `modules/rustic-rifts`), players can host hubs and hunts on their own PCs
(**realms**, DD14) and let friends join directly over ICE (`ParadoxTransport.dll` on `libjuice`) or through a TURN
relay when NAT traversal needs one:

- **Admission keys.** Generate an ES256 admission key pair (`rustic-rifts-admission` / `generate-admission-key`) so the
  backend can sign short-lived `RPT2` admission tickets and per-pair AES-256-GCM transport keys.
- **The TURN relay (`turn-rs`, DD24).** The host kit configures, starts and stops `turn-rs` as its own child process
  (`../crates/rustic-rifts-transport/src/turn.rs`) with `static-auth-secret` set to `P2P_TURN_REST_SECRET`
  from `secrets.env`. `turn-rs` listens on UDP and TCP 3478 (plus TLS 5349 when a certificate is configured) and
  allocates relay ports from its configured UDP `port-range`, which the allowlist firewall rules open while the
  server runs ([security/turn-relay.md](security/turn-relay.md)).
- **Drop-in alternatives and relay-only mode.** Because the backend mints standard `turn-rest` HMAC-SHA1 credentials
  (`P2P_TURN_CREDENTIAL_TTL_SECONDS`, default 3600s), `coturn` is a drop-in fallback using the same secret, and
  Cloudflare TURN remains a configuration option (`P2P_TURN_PROVIDER=cloudflare`). Setting `P2P_FORCE_SDR=true`
  enforces relay-only mode so neither host nor joiner learns the other's IP address.

### Guarded admin actions

The admin API holds the actions that cannot be taken back, or that do a lot at once, instead of running them: a key
that bans or times out more than `ban_limit` players in `ban_window_minutes`, a history revert of more than
`revert_limit` changes, and the revoking of an admin or owner key. A held action waits `delay_minutes` and then
runs, or, with the rule set to `"approval"`, waits for a second person with a different key of at least the same role.
It can be canceled while it waits. **The owner key is held too**; there is no setting that lifts the guard for it
(`"off"` turns a rule off for everyone). Nothing needs setting: the defaults hold only the bulk cases.

```toml
[guard]
ban_limit = 5             # bans and timeouts one key may make in the window before the next is held
ban_guard = "approval"    # "delay" (the default), "approval" or "off"
delay_minutes = 30
```

`dainty-host admin guarded list`, `approve <id>` and `cancel <id>` decide as the host, and **Dainty Lass Manager** (`dainty-manager`)
lists what waits. A rule on `"approval"` on a server with one owner key waits for nobody and expires after
`expire_hours`: use it once the team has a second key. How it works, and what a bot sees, is in
[admin-api.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/admin-api.md#guarded-actions-dh6).

### The Discord bot

The server can run a Discord bot in its own process (dainty-lass's `docs/module-sharing-plan.md`, A7). `/key` gives
the member who asked, and only them, the dashboard's signup link when the dashboard takes signups
(`GET /api/public/signup`), and otherwise an invite of their own: one use, valid 30 minutes, made only while Ramsgate
is up. A member gets the same invite again until it is close to expiring, and at most five `/key` in 15 minutes.
`/status` says whether the game is up. Each `/key`'s outcome, never the invite, is kept with the server's events.

1. In Discord's developer portal, make an application and its bot, and copy the bot's token. The bot needs no
   privileged intents and no permissions.
2. Invite it to your Discord server with the scopes `bot` and `applications.commands`.
3. Give setup the token on stdin, never as an argument, and the guild's id (Developer Mode, then Copy Server ID):

   ```powershell
   Get-Content discord-token.txt | bin\dainty-host setup --discord-bot-token --discord-guild 123456789012345678
   ```

4. Start the server. The bot connects out to Discord, so it needs no port; `/key` and `/status` appear in the
   guild within a minute.

The token stays in `secrets.env`; run setup with `--discord-bot-token` again to replace it, and `--no-discord` to
turn the bot off. Its security checklist is dainty-lass's `docs/security/discord.md`.

### Game modules and plugins

A game module serves one exact game version; a plugin extends the module of its version. A module compiled into
`dainty-host` is loaded unless `compiled` leaves it out; any other is an
executable you list with its SHA-256, and nothing else is ever started. Its path is taken from the server folder (the
one that holds `.workspace`), and that folder must be writable by the server's account only.

```toml
[modules]
# compiled = ["isles-1-14-7"]   # left out: every compiled-in module

[[modules.external]]
id = "rustic-freeza"
path = "modules/rustic-freeza.exe"
sha256 = "<the file's SHA-256: certutil -hashfile <file> SHA256, or sha256sum>"
# autoload = false              # listed, but loaded from the console only
# restart = true                # started again when it dies, with backoff

[modules.rustic-freeza.tuning]  # a module's own settings: the ones it says it reads
```

The 1.14.7 module ships as a library too, in the release's `plugins/` folder (`rustic_isles_plugin.dll`, or
`librustic_isles_plugin.so` on Linux): list it under `[[modules.external]]` with that path and its SHA-256, and leave
it out of `compiled`. It carries the same version data as the compiled-in module; `setup`, `start` and `gamedata`
also install it as `version_data.json` in the backend game data folder, for the module run as a child. A service can be a library the same way (RD27), in place of the one compiled in; a library
whose SHA-256 is not the one written here is never loaded:

```toml
[services.content]
library = "plugins/dainty_content_plugin.dll"
sha256 = "<the file's SHA-256>"
```

For `isles-1-14-7` (`modules/rustic-isles`), the `[modules.<id>.switches]` section (or the top-level `[switches]` table, which `setup` also writes to `backend.env`) overrides the module's switch defaults (dainty-lass's `crates/rustic-isles/src/settings.rs`):

| `[switches]` / `[modules.<id>.switches]` key | `backend.env` variable | Values | Default |
| --- | --- | --- | --- |
| `qos_port` (`[modules.<id>.switches]` only) | `QOS_PORT` | UDP port (`1`–`65535`) or `"0"`/`"off"` | `80` (`0` in `backend.env` when the launcher's relay answers QoS) |
| `reforge` | `RUSTIC_ISLES_REFORGE` | `"on"`, `"log"`, `"off"` | `"on"` |
| `mastery_persistence` | `RUSTIC_ISLES_MASTERY_PERSISTENCE` | `"on"`, `"off"` | `"on"` |
| `bounty_persistence` | `RUSTIC_ISLES_BOUNTY_PERSISTENCE` | `"on"`, `"off"` | `"on"` |
| `cooldown_persistence` | `RUSTIC_ISLES_COOLDOWN_PERSISTENCE` | `"on"`, `"off"` | `"on"` |
| `account_erasure` | `RUSTIC_ISLES_ACCOUNT_ERASURE` | `"on"`, `"off"` | `"on"` (background sweep erases accounts past their deletion grace period) |
| `log_purge` | `RUSTIC_ISLES_LOG_PURGE` | `"on"`, `"off"` | `"on"` (background sweep purges expired email verification codes and activity logs) |
| `rate_limit` | `RUSTIC_ISLES_RATE_LIMIT` | `"on"`, `"off"` | `"on"` (1.14.7 app-wide per-minute HTTP request budget; `429` with `draft-8` `RateLimit` and `RateLimit-Policy` headers) |
| `rate_limit_per_minute` | `RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE` | positive integer | `600` (client requests per minute per address or `/56` IPv6 subnet on 1.14.7) |
| `gameserver_rate_limit_per_minute` | `RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE` | positive integer | `6000` (dedicated game server requests per minute per address on 1.14.7) |
| `host_rate_limit_per_minute` | `RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE` | positive integer | `6000` (player-hosted hunt requests per minute per validated capability on 1.14.7) |

Two game modules for one version are refused, whichever loads first. A port a module asks for that something already holds is moved up past it. In the
console (`serve`), `modules` lists them; `module load|unload|restart|status <id>` and `module run <id> <command>`
act on one, and `start`, `stop` and `restart` take a module's id too. The admin API has `modules`, `loadModule`,
`unloadModule` and `restartModule`. A module whose process dies shows as failed until it is loaded again, or,
with `restart = true`, until the server starts it again; an unload stops it and kills everything it started.

Each service reads one env file `setup` writes into `.workspace\config` (`gateway.env`, `content.env`,
`backend.env`, `director.env`, `allowlist.env`, `admin.env`), and nothing else. Don't edit them: `setup`
rewrites them from `host.toml`.

### A network of servers

Several public servers, one in each region, can form one network (DD16). Each server stays an ordinary server; they
call each other through their gateways, on the same TLS port players use, under a certificate authority you run.
The steps are yours to do by hand:

1. On your own machine (not a server), make the CA once: `dainty-host network ca-init --out cluster-ca.pem`. The
   file holds the CA's key. Back it up, and never copy it to a server.
2. For each server, issue its certificate from that file:
   `dainty-host network issue-node-cert --ca cluster-ca.pem --node oregon-1 --region oregon --host 203.0.113.7`
   (`--host` is the address its gateway answers on, an IP or a DNS name, as the other nodes will list it; repeat it
   for more; `--days` sets how long it is valid, 365 by default). It writes `node-cert.pem`, `node-key.pem` and
   `cluster-ca-cert.pem` to a folder named after the node.
3. Copy those three files to the server's `.workspace/tls` folder, next to the gateway's own certificate.
4. In each server's `host.toml`, name it and list the others, then run `setup` again:

```toml
[network]
node = "oregon-1"
region = "oregon"
# leader = "oregon-1"       # optional directory leader; defaults to the alphabetically first node id

[[network.nodes]]
id = "uk-1"
region = "uk"
host = "uk.example.org"     # an IP address or a lower-case DNS name, as in the certificate
port = 443
fingerprint = "..."         # optional: that server's public certificate, from `gateway make-cert --fingerprint`
```

A node answers only the nodes it lists, and a certificate from the CA is not enough: to cut a node off, remove it
from the other nodes' lists and run `setup` again. Node links need `mode = "public"`. Certificates are renewed by
issuing a new one before `dainty-host network list` says "expires soon"; the server logs a warning 30 days ahead.
The `fingerprint` is what the launcher pins when it measures the ping to that region; a peer without one is
still a peer, only not listed to players. `dainty-host network list` shows this server's place in the network,
or, with `--ca <file>`, what a CA has issued. `dainty-host network status` pings every peer over the node link now
with this server's own certificate and shows each one's state, round trip and clock skew (`--json` for scripts; the exit
code is 1 when a peer is not up). When an existing standalone server joins a network (DN13),
`dainty-host network join` registers its existing accounts with this server as their `home` while keeping existing
invites and account keys intact; if a username clashes with an account already in the directory, the earliest-registered
account keeps the name and the later one gets a temporary `_2` suffix (shortened to 16 characters) and a free rename on
its next login. Docker's `network` profile runs four such nodes with injected latency, for testing
([hosting-docker.md](hosting-docker.md#the-network-profile-four-nodes-on-one-machine)).

To ship a node's change log and database snapshots to a peer region's node without paying for an external S3 bucket
(DH7, DD23 layer 2), add `[history]` naming one of the peers in `[[network.nodes]]`:

```toml
[history]
ship_to_peer = "uk-1"
```

The peer stores each node's shipped segments and snapshots append-only in `.workspace/peer-logs/<origin>/` (using the
peer's own clock for `received_at` metadata and verifying every object's SHA-256 against its content-addressed name),
so a destroyed node's database can be restored from the peer's folder with `history_ship::restore_to` while verifying
the hash chain and any off-server `trusted_head`.

### Managing several installs

One **Dainty Lass Manager** (`dainty-manager`) install or headless controller can manage several enrolled server
installs over pinned mTLS ([remote-management.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/remote-management.md), [security/remote-management.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/security/remote-management.md)).
The player launcher (`dainty-launcher`) has no remote-management surface.

1. **Initialize the controller.** In **Dainty Lass Manager** -> **Host** -> **Managed installs**, click **Enable
   Controller Mode** (or run `bin\dainty-host remote init-controller --node controller --region oregon --url https://203.0.113.10:443`).
   This creates `.workspace\tls\remote-cluster-ca.pem` (kept on the controller only) and writes `[remote] role = "controller"` in `host.toml`.
2. **Mint a single-use enrollment invite.** In the Manager's **Managed installs** panel, enter the target node ID,
   region and optional label and click **Create Enrollment Invite** (or run `bin\dainty-host remote invite --node us-east-1 --region us-east --label "US East 1"`).
   The URI (`dainty-lass-remote://enroll?v=1&url=...&fp=<sha256>&node=...&region=...&token=...`) is single-use and
   expires in 15 minutes (`--ttl-secs`).
3. **Enroll the managed install.** On the target machine, paste the URI in **Dainty Lass Manager** -> **Managed installs**
   or run `bin\dainty-host remote enroll "dainty-lass-remote://enroll?..."`. The install connects over TLS pinned to
   `fp`, generates its own ECDSA P-256 private key locally (`.workspace\tls\node-key.pem` never leaves the machine),
   receives a signed leaf certificate from the controller on `POST /__node/enroll`, and syncs over mTLS on
   `POST /__node/remote`.
4. **Shared settings, local overrides and remote actions.** Changing shared settings on the controller
   (`bin\dainty-host remote set-config --set name="Friday Hunts" --set max_hunt_servers=4`) bumps the controller's
   config revision; enrolled installs merge the shared revision while keeping their local `host`, `game_address`,
   `ports`, `paths`, `database`, `tls`, `secrets.env` and any dotted keys listed in `[remote].local_overrides`. Remote
   admin actions (`bin\dainty-host remote exec --node us-east-1 --action ban --target PlayerName --reason "..."`, or
   the **Doctor** / **Backup** / **Revoke** buttons in the Manager) preserve `DH2` role checks, `DH6` guarded approval
   and `DD23` change-log attribution (`remote:<controller>:<operator>`). If the controller goes offline, managed
   installs keep serving with their last applied configuration.
5. **Revoke or leave.** `bin\dainty-host remote revoke <node>` on the controller immediately refuses that install on
   `/__node/remote` (`403 not_a_peer`); `bin\dainty-host remote leave` on a managed install detaches it back to
   `role = "standalone"`.

| `[remote]` key in `host.toml` | `gateway.env` / `host` variable | Values | Default |
| --- | --- | --- | --- |
| `role` | `DAINTY_REMOTE_ROLE` | `"standalone"`, `"controller"`, `"managed"` | `"standalone"` |
| `controller_url` | `DAINTY_REMOTE_CONTROLLER_URL` | `https://<host>[:<port>]` | unset |
| `controller_cert_fp` | `DAINTY_REMOTE_CONTROLLER_CERT_FP` | 64 lowercase hex digits (SHA-256 of controller gateway cert) | unset |
| `controller_node` | `DAINTY_REMOTE_CONTROLLER_NODE` | `[a-z0-9-]{1,32}` | unset |
| `node` | `DAINTY_REMOTE_NODE` | `[a-z0-9-]{1,32}` | `[network].node` |
| `region` | `DAINTY_REMOTE_REGION` | `[a-z0-9-]{1,32}` | `[network].region` |
| `label` | `DAINTY_REMOTE_LABEL` | display string (`1`–`64` chars) | unset |
| `sync_interval_secs` | `DAINTY_REMOTE_SYNC_INTERVAL_SECS` | integer `5`–`3600` | `15` |
| `local_overrides` | `DAINTY_REMOTE_LOCAL_OVERRIDES` | array of dotted `host.toml` paths preserved locally | `[]` |

## Public servers

- The invite carries the certificate's SHA-256; players' launchers connect to nothing else. Keep
  `.workspace\tls` with your backups.
- The **allowlist** opens UDP 8770-8777 only to players who logged in, through Windows Firewall rules. It runs
  as administrator; `start` asks.
- **Registration**: `INVITECODE` (the default) needs a code per player, `OPEN` lets anyone with the address
  register, `NONE` closes it.
- A ban: `dainty-host` has no command for it yet; the admin API ([admin-api.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/admin-api.md)) and the
  backend's `SetAccountStatus` route do it.

## Updating

```powershell
bin\dainty-host update --check            # is there a newer server release?
bin\dainty-host update                    # install it and run setup again
```

`update` keeps `.workspace` and runs `setup` from your `host.toml`. Neither moves a Node backend's MongoDB data any
more: [dainty-legacy](../../crates/dainty-legacy)'s `import reforged` upgrades an old Node
kit (its `host.json`, layout and MongoDB), and on a server already on `host.toml`, `bin\dainty-host db migrate`
followed by `bin\dainty-host db verify` moves the data into the configured store (`.workspace\config\dainty-lass.db`
on SQLite, or Postgres when `[database]` sets `engine = "postgres"`). `verify` writes `.workspace\config\.mongo-migrated`
once it finds no difference; MongoDB is never modified. `start` and `serve` refuse to run the Rust backend while
`.workspace\mongodb` holds data without that marker (unless the host opts into an empty database with
`dainty-host setup --fresh-start` or `fresh_start = true` under `[database]` in `host.toml`). A `host.json` with no
`host.toml` is refused until dainty-legacy has imported it. When `native\` changed, `update` says so: rebuild the
runtime and publish it again.
Players' launchers notice the new hashes and ask once more.

### Live server cutover checklist (backup, update, smoke, rollback)

When cutting a live 1.14.7 server over from the Node backend to the Rust backend:

1. **Backup.** Stop the server (or take a snapshot first), run `bin\dainty-host backup` so `.workspace\backups\`
   holds a fresh database dump, and copy `.workspace\config` (`host.toml`, `secrets.env`) and `.workspace\tls`
   (`gateway-cert.pem`, `gateway-key.pem`) to safe off-server storage.
2. **Update and migrate.** Run `bin\dainty-host update`. If the server has existing MongoDB data, start `mongod`
   and run `bin\dainty-host db migrate` then `bin\dainty-host db verify`, and confirm `verify` reports no
   difference.
3. **Smoke check.** Run `bin\dainty-host doctor`, start the server with `bin\dainty-host start`, verify that
   `/dauntless-status`, `/dainty/api/Status` and `/content/v1/runtime` answer on the gateway, and verify with a
   1.14.7 client and launcher that an existing account can sign in, enter Ramsgate with its characters, inventory and
   loadouts intact, and depart for a hunt.
4. **Rollback (if needed).** Stop the server, export the SQLite or Postgres store back to MongoDB with
   `bin\dainty-host db export-mongo --to mongodb://127.0.0.1:27017/reforged` (source and target default from
   `host.toml`), verify with `bin\dainty-host db verify --against mongodb://127.0.0.1:27017/reforged`, and start the
   `paradox` build (below).

## Rollback to the Node backend and MongoDB (`paradox` feature)

> [!WARNING]
> Since the 2026-11-06 protocol rename (`/dainty/api`, `x-dainty-*` headers), the Node backend and Director no
> longer speak the launcher's protocol, so a rollback to them cannot serve a current launcher. Docker's `rollback`
> profile is gone; RB15 removes the rest.

For one release after the RU9/RB14 cutover (until RB15 deletes the reference Node services), building the server with
the opt-in `paradox` Cargo feature (`cargo build --release -p dainty-server --features paradox`) runs Mystic
Paradox's Node `ParadoxBackend` and `ParadoxDirector` from `modules/rustic-isles/reference/1.14.7/` against MongoDB
instead of the compiled-in `isles-1-14-7` Rust module. To export a server's SQLite or Postgres store back to MongoDB
before starting a `paradox` build, run `dainty-host db export-mongo --to <mongodb-uri>` (and optionally
`dainty-host db verify --against <mongodb-uri>`; `--from` and `--to` default from `host.toml`).
