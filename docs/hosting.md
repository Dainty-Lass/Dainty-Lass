<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Hosting a Dainty Lass server

This guide is for running a server. To play on someone else's server you only need the launcher and the invite link
they send you ([players.md](players.md)).

A Dainty Lass server runs Dauntless 1.14.7 for you and your players. It is one program, `dainty-host`, that sets the
server up, runs it and looks after it. In short:

1. Pick a kind of server: **private** (friends who share a Tailscale network with you) or **public** (anyone on the
   internet who has an invite).
2. Unpack a server release, or build one from this repository.
3. Run `dainty-host setup` once with your choices.
4. Run `dainty-host start` (or `serve`, to get a console).
5. Make an invite with `dainty-host invite` and send it.

`dainty-host doctor` says at any point what is still missing. The rest of this page goes through each step, then
lists every setting.

Other guides:

- [hosting-linux.md](hosting-linux.md): a Linux server, its systemd units, nftables, Wine and Proton.
- [hosting-docker.md](hosting-docker.md): the same server in Docker with the root `compose.yaml`.
- [sdk-and-game-data.md](sdk-and-game-data.md): the SDK, the game runtime and the game data in detail.

## What you need

- A 64-bit Windows PC (or a Linux server, see [hosting-linux.md](hosting-linux.md)) that stays on while people play.
- Dauntless 1.14.7 for the game servers. `dainty-host install-game` fetches it from your own Epic library
  ([Installing the game](#installing-the-game)); any other copy of that exact build works too.
- About 1.5 GB of free memory for each hunt you want to run at once, on top of what the system needs (see
  `[performance]`).
- For a **private** server: [Tailscale](https://tailscale.com) running and signed in on the server, and on every
  player's PC.
- For a **public** server: a public IPv4 address (or a DNS name that points at one), TCP 443 (or another port you
  choose) and the game's UDP ports reachable from the internet.
- Your own game data, made from your own copy of the game. A server runs on the shipped examples, but nobody can
  play on them ([Game data, the runtime and the SDK](#game-data-the-runtime-and-the-sdk)).

## Installing

### From a release

A server release is one archive per platform, `DaintyLassServer-<version>-windows-x64.zip` or
`DaintyLassServer-<version>-linux-x64.tar.gz`, with `SHA256SUMS.txt` beside them. Unpack it anywhere you like; the
folder it makes, `dainty-lass-server-<version>/`, is the **server folder**. It holds:

| Path | What it is |
| --- | --- |
| `bin/dainty-host` | The server in one binary: setup, the services, the console and every command on this page |
| `bin/dainty-server`, `bin/dainty-server-minimal` | The same server started straight into `serve` (the second without a console or dashboard) |
| `bin/dainty-legacy` | The importer for old installs ([Upgrading from an older install](#upgrading-from-an-older-install)) |
| `bin/rustic-isles-runtime.dll` | The prebuilt game runtime, which `setup` publishes for the game servers and your players |
| `data/1.14.7/` | `version_data.json` and `version_facts.json`, which the server reads at start |
| `plugins/` | Libraries `settings.toml` can load in place of the built-in services and module (`[services]`, `[modules]`) |
| `modules/rustic-isles/game-data/1.14.7/` | The game data's examples and `store-captures.json`; your generated data goes here too |
| `release.json` | The version, commit and platform, and every file with its SHA-256, which `update` checks |
| `docs/` | These guides |

Run every command from the server folder (or anywhere inside it): `dainty-host` finds the server folder from the
current folder, and otherwise from its own location. On Windows, in PowerShell:

```powershell
cd D:\Servers\dainty-lass-server-1.0.0
bin\dainty-host --help
```

### From a source checkout

A checkout of this repository works as a server folder too. Build the server and run it from the checkout's root:

```powershell
cargo build --release --locked -p dainty-server --features dainty-server/all-modules --bins
target\release\dainty-host --help
```

A checkout has no `data/` folder; the server then reads the version data from `crates/rustic-isles-1-14-7/data/`.

### What the server keeps

Everything the server makes lives in `.workspace/` in the server folder. Back it up; never share it.

| Path | What it is |
| --- | --- |
| `.workspace/config/settings.toml` | The server's settings, which you edit |
| `.workspace/config/secrets.env` | Its keys and passwords, made once by `setup` |
| `.workspace/config/<service>.env` | Each service's settings, written by `setup` from the two files above. Don't edit them |
| `.workspace/config/dainty-lass.db` | The SQLite database (accounts, characters, everything players own) |
| `.workspace/config/news.toml`, `session-supplies.toml` | The launcher's news and per-account session supplies, which you write |
| `.workspace/tls/` | The gateway's certificate and key (public mode) and the node certificates of a network |
| `.workspace/runtime/` | The game runtime published for the game servers and for players to download |
| `.workspace/branding/` | The art the launcher shows on your server's page (background images, credits and an accent colour) |
| `.workspace/logs/` | `setup.log`, `update.log`, the game servers' logs (`gameservers/`) and the allowlist's audit log |
| `.workspace/backups/` | What `dainty-host backup` writes |
| `.workspace/game/` | Where `install-game` puts the game unless told otherwise |

## Private or public

**Private** is for friends. The server listens only on its Tailscale address, so only people on your tailnet (or
people you share the machine with) can reach it. There is no TLS of the server's own: Tailscale encrypts the
traffic. The gateway is plain HTTP on port 61000 of the Tailscale address, and the Windows firewall script setup
writes lets only Tailscale addresses (100.64.0.0/10) in.

**Public** is for anyone with an invite. The gateway listens on every interface with TLS, on port 443 by default,
with a self-signed certificate whose SHA-256 goes into every invite: the launcher checks it before it sends anything.
The game servers' UDP ports start closed, and the **allowlist** opens them only to the addresses of players who
signed in, for ten minutes at a time while they stay signed in.

Some settings work in one mode only. XP and reward multipliers and the play-test cheats are for private servers.
`public_port`, `[network]` with peers and `[split]` need a public one.

## Setting up

`setup` writes `settings.toml` from its options, makes `secrets.env` the first time, writes every service's env file,
makes the gateway certificate (public mode), publishes the runtime, seeds or generates the game data, and writes the
firewall rules. Run it again whenever you change a setting: an option you leave out keeps the value `settings.toml`
already has.

A private server. Tailscale must be running; setup takes this PC's Tailscale address when you give no `--host`:

```powershell
bin\dainty-host setup --mode private --name "Friday Hunts" --game "D:\Games\Dauntless 1.14.7"
```

A public server on a DNS name. The game servers announce an IP address, not a name, so give it as well:

```powershell
bin\dainty-host setup --mode public --host play.example.org --game-address 203.0.113.7 --name "Friday Hunts" `
  --game "D:\Games\Dauntless 1.14.7"
```

What setup prints ends with the next steps. Nothing is written when an option is wrong. Put a value with spaces in
quotes; setup refuses words it did not expect, which is usually a path that lost its quotes.

The [setup options](#setup-options) table lists every option and the `settings.toml` key it sets. Things setup has no
option for, you set in `settings.toml` and then run `setup` again.

## Running it

```powershell
bin\dainty-host start          # the server, without a console; Ctrl+C stops it
bin\dainty-host serve          # the same server, with a console
bin\dainty-host serve --tui    # with the dashboard instead of the line console
```

The server is one process. Before anything starts it checks the configuration, the ports, the game folder, the game
data and the database, and refuses to start on a problem it would otherwise crash on. When everything is up it prints
`Server "<name>" is up (<mode> mode, gateway <host>:<port>).`

`serve --config <file>` runs another `settings.toml` (or a `.yaml`, `.yml` or `.json` one); `--log-level` takes a
filter as `RUST_LOG` does (`info` by default).

The console (`serve`) takes these commands, and `help` lists them:

| Command | What it does |
| --- | --- |
| `status` | The services and modules and their state |
| `start <service>`, `stop <service>`, `restart <service>` | One service or module |
| `stop`, `quit` | Stops the whole server |
| `players`, `instances` | How many players are connected, and how many hunts run out of the most allowed |
| `invite` | Makes an invite (as `dainty-host invite`) |
| `doctor`, `backup` | As the commands of the same name |
| `log <service> <debug\|info\|warn\|error>` | Changes a service's log level while it runs |
| `modules`, `module list\|status\|load\|unload\|restart <id>`, `module run <id> <command> [args]` | The game modules and plugins |
| `history ...`, `revert ...` | The change log ([Undoing a change](#undoing-a-change)) |
| `legacy import <kind> [args]` | An old install's import while the server runs |

The dashboard (`--tui`): Up and Down pick a service, `r` restarts it, `s` starts or stops it, `f` or Tab cycles the
log filter, PgUp and PgDn scroll, `:` opens the command prompt, and `q`, Esc or Ctrl+C stop the server.

To keep the server running after you log off on Windows, run `start` from a scheduled task. A public server's
allowlist changes Windows Firewall, so it needs administrator rights: give the task "Run with highest privileges".

## Invites

```powershell
bin\dainty-host invite --for "Alex"            # one account
bin\dainty-host invite --for "Guild" --uses 20
bin\dainty-host invite --unlimited
bin\dainty-host invite --list                  # every code and how often it was used
```

`invite` makes an invite code on the running server and prints the whole link. Send the whole line: the player pastes
it into the launcher, or clicks it once the launcher is installed. `--uses` takes 1 to 10000 (1 by default), `--for`
is kept as the code's note, and `--json` prints the link as JSON.

The link carries the server's address, port and name and the code. A private server's is
`dainty://join?v=1&host=...&port=61000&code=...&name=...`, with `&share=<link>` when `share_url` is set. A public
server's is `dainty://join?v=2&mode=public&host=...&port=443&fp=<certificate SHA-256>&code=...&name=...`. The
launcher refuses a public server whose certificate does not have that `fp`.

`registration` decides what a code is for:

| `registration` | Who may make an account |
| --- | --- |
| `INVITECODE` (the default) | Whoever has a code with uses left; `invite` makes one each time |
| `OPEN` | Anyone with the link; `invite` puts `OPEN` in it and makes no code |
| `NONE` | Nobody; `invite` refuses |

For a private server the player also needs to be on your tailnet, or to have the machine shared with them from
<https://login.tailscale.com/admin/machines>. Give setup that share link with `--share` and it goes into every
invite.

With `[oauth]` set, players can make their account with **Continue with Discord** (or your own provider) instead of a
name and a code ([oauth.md](oauth.md)).

## Ports

| Port | What | Where it listens |
| --- | --- | --- |
| TCP 443 (`public_port`) | The gateway of a public server, TLS | Every interface |
| TCP 61000 (`[ports] gateway`) | The gateway of a private server, HTTP | The Tailscale address |
| UDP 8770-8777 | The game servers, up to 6 hunts; one more port below 8770 for each hunt above 6 (8766-8777 at 10) | Every interface; on a public server, open only to signed-in players |
| TCP 41000 | The backend | 127.0.0.1 |
| TCP 41001 | The Director, which starts the game servers | 127.0.0.1 |
| TCP 41002 | The content server (runtime downloads, news, branding, crash reports) | 127.0.0.1 |
| TCP 41003 | The admin API, which dainty-dash and its fleet agent use | 127.0.0.1 |
| TCP 41005 | The allowlist helper (public mode) | 127.0.0.1 |
| TCP 41006 | Chat, reached through the gateway | 127.0.0.1 |
| TCP 41010 | The backend's QoS listener | 127.0.0.1 |

Only the gateway and the game ports need to be reachable from outside; open or forward those two and nothing else.
The gateway refuses the host-only routes, so the admin API and the backend's own admin routes are never reachable
from the internet. `[ports]` moves any loopback port when another program holds it.

### Ports Windows holds

Windows lends blocks of ports from 49152 to 65535 to Hyper-V, WSL and Docker, and a lent port can't be bound (os error
10013). The blocks move on every reboot. `setup` and `start` read them from netsh and move any port the server
listens on that falls in one: the private gateway to 41080, every other port to the next free one above its default.
Each move is written to `[ports]` so it stays put, and printed. An invite made before the gateway moved names the old
port; make a new one.

## The firewall

On Windows, setup tries to add the inbound rules with netsh, which works only from an administrator prompt. It also
writes `.workspace/config/firewall.ps1`; run it once as administrator if setup could not add the rules:

```powershell
powershell -ExecutionPolicy Bypass -File .workspace\config\firewall.ps1
```

A private server's rules let in only Tailscale addresses. A public server's let anyone reach the gateway's port, and
its game-port rule starts disabled: the allowlist opens the game ports to players who signed in. On Linux setup writes
`.workspace/config/firewall.sh` for ufw instead ([hosting-linux.md](hosting-linux.md)).

If your router or cloud provider has a firewall of its own, open the gateway's TCP port and the game's UDP range there
too.

## TLS

A public server's gateway uses a self-signed certificate, `.workspace/tls/gateway-cert.pem` and `gateway-key.pem`,
made by the first `setup` for `host` (and `game_address` when it differs). It is an ECDSA P-256 certificate valid for
ten years. Setup keeps it from then on and never makes a new one, because every invite's `fp` is its SHA-256: a new
certificate would break every invite and every player's saved server. Setup prints the fingerprint, and `doctor`
checks the files.

Back the `tls` folder up with the rest of `.workspace`. If you move the server to another machine, take it along.

`invite` refuses to make a public invite while the running gateway serves another certificate than the one on disk;
restart the server so it loads the current one.

## Checking with doctor

```powershell
bin\dainty-host doctor
bin\dainty-host doctor --no-hash    # skips hashing the game executable
```

`doctor` checks, by area: the configuration (and any variable from the environment that overrides it), the published
runtime, the game folder and its executable against the pinned 1.14.7 build, a game install left unfinished, a runtime
DLL that must be removed, the memory for your hunts, on Linux Wine or Proton and its prefix, xvfb-run, the inject
helper and nftables, the network (Tailscale's address against `game_address`, or the gateway certificate), the
database, and whether the server answers right now. Each problem comes with the command that fixes it. It exits with
a failure when any check fails, and a summary of the failures and warnings.

## Installing the game

```powershell
bin\dainty-host install-game                        # into .workspace\game
bin\dainty-host install-game --dir D:\Games\Dauntless
bin\dainty-host install-game --sign-out             # ends the Epic session and installs nothing
```

`install-game` installs Dauntless 1.14.7 from your own Epic library. It prints Epic's login link; open it in any
browser, sign in and paste the code Epic shows (or pass it in a file with `--code-file`). The session is kept in
`.workspace/config/epic-session.env`, readable only by you, and nowhere else. When the install is done it sets
`game_dir` in `settings.toml` and rewrites the env files; restart the server for it to take effect.

## Game data, the runtime and the SDK

The server needs three things made from your own copy of the game, none of which this repository contains:

- **The runtime**, `rustic-isles-runtime.dll`. A release ships it prebuilt in `bin/`, and `setup` publishes it into
  `.workspace/runtime/`. The Director loads it into every game server, and the content server offers it to players,
  whose launcher asks them to accept it once. `dainty-host publish-runtime [--from <folder>]` publishes a runtime you
  built yourself. No game server starts without it.
- **The game data**: the JSON the backend and the Director serve, generated from an export the CatalogExporter makes
  inside the running game. `dainty-host gamedata` generates it into `modules/rustic-isles/game-data/1.14.7/`, and
  fetches the store captures `store-captures.json` names. Until then the server runs on the shipped examples: the
  services start, but nobody can play.
- **The SDK**, which the CatalogExporter is built against, dumped from your running game with Dumper-7.

The launcher's Host page has a button for each step. [sdk-and-game-data.md](sdk-and-game-data.md) covers them in
full. The commands:

```powershell
bin\dainty-host gamedata                          # generate from the exports
bin\dainty-host gamedata --examples               # seed the shipped examples
bin\dainty-host gamedata --progression ""         # only the progression source, from the workspace's exports
bin\dainty-host gamedata --example-sources C:\src # copy supplied *.source.json files into the raw game data
bin\dainty-host gamedata --export gamedata.zip    # move it to another server as a pack
bin\dainty-host gamedata --import gamedata.zip
bin\dainty-host preset --create mine              # keep this server's game data as the checkout's preset
bin\dainty-host preset --apply default            # apply it; --force replaces what differs, keeping backups
```

The SDK and the game data are Phoenix Labs' material: never commit them and never share them beyond your own servers.

## Settings

`settings.toml` in `.workspace/config/` is the server's whole configuration. Edit it, then run `dainty-host setup`
again to apply it. Setup keeps your comments, your order and every value it does not change. A new file has a
comment above each key that says what it does. A key that is left out takes its default.

An unknown key stops setup, and a key in the old camelCase form gets its right name (`max_hunt_servers`, "did you
mean max_hunt_servers?"), so a typo is never silently ignored. The services never read `settings.toml` themselves:
setup turns it into one env file per service, and `start` hands each service its own. Those files say at the top
that setup rewrites them; don't edit them.

### Setup options

Each option left out keeps what `settings.toml` has, or the default on a first setup.

| Option | Sets | Notes |
| --- | --- | --- |
| `--mode private\|public` | `mode` | Required on the first setup |
| `--host <address>` | `host` | Private: this PC's Tailscale address when left out |
| `--game-address <IPv4>` | `game_address` | Needed when `host` is a DNS name |
| `--name <name>` | `name` | |
| `--game <folder>` | `game_dir` | The full path of the 1.14.7 folder |
| `--registration INVITECODE\|OPEN\|NONE` | `registration` | |
| `--share <link>` | `share_url` | Private only; an empty value removes it |
| `--max-hunts <1-10>` | `max_hunt_servers` | |
| `--xp-multiplier <n>` (`--xp`), `--reward-multiplier <n>` (`--rewards`) | `[multipliers]` | Private only; `x2` and `2` both work |
| `--no-multipliers` (`--off`) | `[multipliers]` | Removes them |
| `--public-port <port>` | `public_port` | Public only; 443 removes the key |
| `--server-fps <n>`, `--idle-fps <n>` | `performance.server_fps`, `idle_fps` | |
| `--training-on-demand`, `--training-at-boot` | `performance.training_on_demand` | |
| `--training-idle-minutes <n>` | `performance.training_idle_minutes` | |
| `--verbose-game-logs`, `--quiet-game-logs` | `performance.verbose_game_logs` | |
| `--memory-guard`, `--no-memory-guard` | `performance.memory_guard` | |
| `--min-free-ram <MB>`, `--hunt-ram <MB>` | `performance.min_free_memory_mb`, `hunt_memory_mb` | `1024` or `1024MB` |
| `--warm-spare`, `--no-warm-spare` | `performance.warm_spare` | |
| `--wine <path or name>` | `linux.wine` | A name is looked up on PATH; empty finds it at run time |
| `--proton <path>`, `--no-proton` | `linux.proton` | |
| `--xvfb`, `--no-xvfb` | `linux.xvfb` | |
| `--firewall-interface <name>`, `--no-firewall-interface` | `linux.firewall_interface` | |
| `--fresh-start` | `database.fresh_start` | Records that this server started on an empty database |
| `--skip-install` | | Skips installing the services from the release |
| `--discord-bot-token` | `discord.enabled`, `DISCORD_BOT_TOKEN` in `secrets.env` | Reads the token from stdin |
| `--no-discord` | `discord.enabled` | The token stays in `secrets.env` |
| `--discord-guild <id>` (repeatable) | `discord.guild_ids` | Replaces the list |
| `--discord-application-id <id>` | `discord.application_id` | |
| `--discord-dashboard-url <url>` | `discord.dashboard_url` | |
| `--oauth-discord <client id>` | `[oauth]` with `preset = "discord"` | |
| `--oauth-client-secret` | `DAINTY_OAUTH_CLIENT_SECRET` in `secrets.env` | Reads the secret from stdin |
| `--oauth-guild <id>` (repeatable) | `oauth.guild_ids` | Replaces the list |
| `--oauth-required`, `--oauth-optional` | `oauth.required` | |
| `--no-oauth` | `[oauth]` | Removes the table; the secret stays in `secrets.env` |

`--discord-bot-token` and `--oauth-client-secret` both read stdin, so run setup once for each. A secret is never an
option on the command line, where other users and your shell's history would see it.

Some settings have a command of their own, which rewrites the env files too:

```powershell
bin\dainty-host multipliers                       # shows them
bin\dainty-host multipliers --xp 2 --rewards 1.5
bin\dainty-host multipliers --off
```

### The main settings

In the "Becomes" column, a variable is written to the env file in brackets; "the server" means the host process reads
it itself. Every service env file is in `.workspace/config/`.

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `version` | | `1` | `1` |
| `mode` | `DAINTY_SERVER_MODE` (backend, admin); the gateway's TLS and bind address; whether the allowlist runs | `"private"` or `"public"` | none: setup asks |
| `host` | `DAINTY_SERVER_HOST`, `REALTIME_XMPP_ALLOWED_HOSTS` (backend); every invite; the certificate | Private: a Tailscale IPv4 (100.64.0.0/10) or a `.ts.net` name. Public: a public IPv4 or a DNS name | Private: this PC's Tailscale address |
| `game_address` | `MY_IP` (director); `GATEWAY_BIND` (gateway, private) | An IPv4 address; private: the Tailscale one; public: a public one | `host`, when that is an IPv4 address |
| `name` | `DAINTY_SERVER_NAME` (backend); every invite | Up to 48 characters | `"Dainty Lass server"` |
| `game_dir` | `GAMESERVER_BINARY_PATH` (director, Windows) or `DAINTY_GAME_EXE` (director, Linux); `GAMESERVER_RUNTIME_LOG_DIR` | The full path of the 1.14.7 folder (the one with `Archon`) | none: no game server can start |
| `registration` | `REGISTRATION_MODE` (backend) | `"INVITECODE"`, `"OPEN"` or `"NONE"` | `"INVITECODE"` |
| `share_url` | invites only | A `https://login.tailscale.com/` link; private only | none |
| `max_hunt_servers` | `MAX_CONCURRENT_HUNT_SERVERS`, `PORT_RANGE_BEGIN`, `PORT_RANGE_END` (director); `DAINTY_MAX_HUNT_SERVERS` (admin); `ALLOWLIST_PORTS` (allowlist) | 1 to 10 | 3 |
| `public_port` | `GATEWAY_PORT` (gateway); every invite | 1 to 65535, not a port the server uses itself; public only | 443 |
| `dashboard_url` | `CHAT_DASHBOARD_URL` (gateway), `CONTENT_DASHBOARD_URL` (content), each with the ingest key | The staff dashboard's `http://` or `https://` address | none |
| `version_data` | the server, at start | A path relative to the server folder | `data/1.14.7/version_data.json` |
| `version_facts` | the server, at start | A path relative to the server folder | `data/1.14.7/version_facts.json` |
| `[multipliers] xp`, `rewards` | `DAINTY_XP_MULTIPLIER`, `DAINTY_REWARD_MULTIPLIER` (backend), only when one is not 1 | 1 to 100, rounded to two decimals; private only | 1 (off) |

`dashboard_url` is for dainty-dash: chat sends it its log and reports and takes its chat filter, and the content
server sends it a copy of players' crash reports, both with `DASHBOARD_INGEST_KEY` from `secrets.env` (set the same
key in the dashboard).

Two keys are only read to tell you they are gone: `edition` (`"legacy"` and `"both"` ran Dauntless Revived's 1.4.4,
and are refused; import such a server with `dainty-host legacy import revived`) and `[mongo]` (MongoDB is retired:
the table is ignored and left out the next time setup writes the file).

### `[performance]`: the game servers

All of these go to the Director's env file, and every game server inherits them.

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `server_fps` | `DAINTY_SERVER_MAX_FPS` | 0 (no cap) or 10 to 240 | 30 |
| `idle_fps` | `DAINTY_SERVER_IDLE_FPS`; after 15 s without a player | 0 (same as `server_fps`) or 1 to 240 | 10 |
| `training_on_demand` | `TRAINING_ON_DEMAND`; the Training Grounds start the first time someone goes there, not at boot | `true` or `false` | `true` |
| `training_idle_minutes` | `TRAINING_IDLE_STOP_MINUTES`; on demand, they stop again after this long without a player | 0 (keep running) to 1440 | 5 |
| `verbose_game_logs` | `GAMESERVER_LOG_CMDS`, set to quiet the noisy log categories when `false` | `true` or `false` | `false` |
| `memory_guard` | `GAMESERVER_MEMORY_GUARD`; a new hunt starts only with `min_free_memory_mb + hunt_memory_mb` free | `true` or `false` | `true` |
| `min_free_memory_mb` | `GAMESERVER_MIN_FREE_MB` | 0 to 1048576 | 1024 |
| `hunt_memory_mb` | `GAMESERVER_HUNT_MEMORY_MB` | 256 to 1048576 | 1536 |
| `warm_spare` | `GAMESERVER_WARM_SPARE`; one hunt server waits booted, so the next hunt starts in about 1.5 s instead of 5. It holds one hunt slot and about 1 GB. A prototype | `true` or `false` | `false` |

### `[linux]`

Read on Linux only ([hosting-linux.md](hosting-linux.md)).

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `wine` | `DAINTY_WINE` (director) | The full path of the wine program | the first `wine64` or `wine` on PATH |
| `proton` | `DAINTY_PROTON` (director); runs the game servers with Proton instead of Wine | The full path of a Proton (or GE-Proton) `proton` script | none |
| `xvfb` | `DAINTY_XVFB` (director); a virtual display (xvfb-run) when there is no `DISPLAY` | `true` or `false` | `true` |
| `firewall_interface` | `ALLOWLIST_INTERFACE` (allowlist); the interface the allowlist filters | An interface name such as `eth0` | every interface |

### `[database]`

The server keeps everything in one database: SQLite by default, a file in `.workspace/config/` with nothing to
install, or a Postgres server you run. The admin API gets the same settings in its env file.

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `engine` | `DAINTY_DB_ENGINE` (admin); with SQLite also `DAINTY_DB_FILE` | `"sqlite"` or `"postgres"` | `"sqlite"` (`.workspace/config/dainty-lass.db`) |
| `postgres_host` | `DAINTY_DB_HOST` | An address on this machine, unless `allow_remote` | `127.0.0.1` |
| `postgres_port` | `DAINTY_DB_PORT` | 1 to 65535 | 5432 |
| `postgres_user` | `DAINTY_DB_USER`; its password is `DAINTY_DB_PASSWORD` in `secrets.env` | A role name | `dainty` |
| `postgres_database` | `DAINTY_DB_NAME` | A database name | `dainty` |
| `allow_remote` | `DAINTY_DB_ALLOW_REMOTE=1` | `true` to accept a Postgres server on another machine | `false` |
| `fresh_start` | nothing; a record | `true`: this server started on a new database rather than an old install's MongoDB data | none |

Connections to Postgres are not encrypted, so setup refuses a Postgres host off this machine unless `allow_remote =
true`. Set that only over a network you trust: a private one, never the internet. Setup makes the password in
`secrets.env`; the server does not create the role or the database, so make both with that password before it starts:

```toml
[database]
engine = "postgres"
# postgres_host = "127.0.0.1"
# postgres_port = 5432
# postgres_user = "dainty"
# postgres_database = "dainty"
```

### `[switches]`: the game's rules

These go to the backend's env file, except `test_cheats`. A switch left out takes the game module's default. The same
switches can also be set in the module's own section, `[modules.isles-1-14-7.switches]`; `[switches]` wins when both
set one.

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `reforge` | `RUSTIC_ISLES_REFORGE`; weapon reforge rules | `"on"`, `"log"` or `"off"` | `"on"` |
| `mastery_persistence` | `RUSTIC_ISLES_MASTERY_PERSISTENCE`; whether a game server's mastery report is saved | `"on"` or `"off"` | `"on"` |
| `bounty_persistence` | `RUSTIC_ISLES_BOUNTY_PERSISTENCE`; whether bounty saves are stored | `"on"` or `"off"` | `"on"` |
| `cooldown_persistence` | `RUSTIC_ISLES_COOLDOWN_PERSISTENCE`; whether cooldown saves are stored | `"on"` or `"off"` | `"on"` |
| `account_erasure` | `RUSTIC_ISLES_ACCOUNT_ERASURE`; the background sweep erases accounts past their deletion grace period | `"on"` or `"off"` | `"off"` |
| `log_purge` | `RUSTIC_ISLES_LOG_PURGE`; the background sweep purges expired verification codes and activity logs | `"on"` or `"off"` | `"off"` |
| `rate_limit` | `RUSTIC_ISLES_RATE_LIMIT`; the per-minute HTTP request budget | `"on"` or `"off"` | `"on"` |
| `rate_limit_per_minute` | `RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE`; for one client address | 1 to 1000000 | 600 |
| `gameserver_rate_limit_per_minute` | `RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE`; for a dedicated game server's address | 1 to 1000000 | 6000 |
| `host_rate_limit_per_minute` | `RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE`; for a player-hosted hunt | 1 to 1000000 | 6000 |
| `gauntlet` | `RUSTIC_ISLES_GAUNTLET`; Gauntlet hunts are matched and the Slayer's Path shows Gauntlet. Untested in game | `"on"` or `"off"` | `"off"` |
| `test_cheats` | `DAINTY_TEST_CHEATS` (director, so every game server); play-test cheats | `"on"` (one-hit Behemoth kills, and a Gauntlet level ends after its first round), `"kills"` (one-hit kills only) or `"off"`; private only | `"off"` |

### Service tunables

Each of these tables is optional, and so is each key in it. A key left out is not written, and the service's own
default holds. The gateway's, content's and the allowlist's values are checked by those services' own loaders when
setup runs, so a bad value stops setup with the service's message rather than a crash at start.

`[gateway]`, `[gateway.limits]`, `[gateway.rate]` and `[gateway.timeouts]` (`gateway.env`):

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `gateway.allowlist_refresh_seconds` | `GATEWAY_ALLOWLIST_REFRESH_SECONDS`; how often a signed-in player is reported to the allowlist again (public) | 5 to 540 | 60 |
| `gateway.limits.max_body_bytes` | `GATEWAY_MAX_BODY_BYTES` | 1024 to 67108864 | 131072 |
| `gateway.limits.max_connections` | `GATEWAY_MAX_CONNECTIONS` | 1 to 100000 | 2048 |
| `gateway.limits.max_connections_per_ip` | `GATEWAY_MAX_CONNECTIONS_PER_IP` | 1 to 100000 | 128 |
| `gateway.rate.general` | `GATEWAY_RATE_GENERAL` | `{ burst = <whole number, 1 or more>, per_minute = <number, 0 or more> }` | 300 / 180 |
| `gateway.rate.content` | `GATEWAY_RATE_CONTENT` | as above | 600 / 600 |
| `gateway.rate.register` | `GATEWAY_RATE_REGISTER` | as above | 5 / 0.2 |
| `gateway.rate.token` | `GATEWAY_RATE_TOKEN` | as above | 10 / 1 |
| `gateway.rate.ws` | `GATEWAY_RATE_WS` | as above | 20 / 12 |
| `gateway.rate.connect` | `GATEWAY_RATE_CONNECT` | as above | 200 / 300 |
| `gateway.timeouts.handshake_ms` | `GATEWAY_HANDSHAKE_TIMEOUT_MS`; the TLS handshake | 1000 to 120000 | 10000 |
| `gateway.timeouts.headers_ms` | `GATEWAY_HEADERS_TIMEOUT_MS`; a request's head | 1000 to 120000 | 10000 |
| `gateway.timeouts.request_ms` | `GATEWAY_REQUEST_TIMEOUT_MS`; a whole request | 1000 to 600000 | 30000 |
| `gateway.timeouts.keep_alive_ms` | `GATEWAY_KEEPALIVE_TIMEOUT_MS`; an idle keep-alive connection | 1000 to 600000 | 65000 |
| `gateway.timeouts.idle_ms` | `GATEWAY_IDLE_TIMEOUT_MS`; a connection with no traffic | 1000 to 3600000 | 120000 |
| `gateway.timeouts.upstream_idle_ms` | `GATEWAY_UPSTREAM_IDLE_TIMEOUT_MS`; an idle pooled connection to a service | 1000 to 3600000 | 120000 |
| `gateway.timeouts.ws_idle_ms` | `GATEWAY_WS_IDLE_TIMEOUT_MS`; a WebSocket with no traffic | 1000 to 86400000 | 300000 |
| `gateway.timeouts.shutdown_grace_ms` | `GATEWAY_SHUTDOWN_GRACE_MS`; how long open connections get to finish at shutdown | 0 to 300000 | 10000 |

A rate is a token bucket: `burst` requests at once, refilled at `per_minute`. It is written inline and reaches the env
file as `burst,per_minute`:

```toml
[gateway.limits]
max_connections = 4096

[gateway.rate]
ws = { burst = 40, per_minute = 24 }
```

`[content]` (`content.env`):

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `max_streams_per_account` | `CONTENT_MAX_STREAMS_PER_ACCOUNT`; downloads one account may run at once | 1 to 64 | 4 |
| `max_streams_total` | `CONTENT_MAX_STREAMS_TOTAL`; downloads at once in all | 1 to 1024 | 48 |
| `auth_cache_seconds` | `CONTENT_AUTH_CACHE_SECONDS`; how long a checked sign-in is kept | 0 to 3600 | 300 |
| `news_url` | `CONTENT_NEWS_URL`; news fetched from there wins over `news.toml` once a fetch succeeds | A `https://raw.githubusercontent.com/...` file URL | none |

`[allowlist]` (`allowlist.env`, public servers only; on a private server it writes nothing):

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `ttl_seconds` | `ALLOWLIST_TTL_SECONDS`; how long a signed-in address keeps the game ports open | 1 to 86400 | 600 |
| `min_interval_ms` | `ALLOWLIST_MIN_INTERVAL_MS`; the least time between two firewall updates | 50 to 600000 | 3000 |
| `max_entries` | `ALLOWLIST_MAX_ENTRIES`; the most addresses let in at once | 1 to 1000 | 256 |
| `nft_table` | `ALLOWLIST_NFT_TABLE`; Linux: the nftables table the helper owns | A letter, then up to 31 letters, digits or `_` | `dainty_lass` |

`[backend]` (`backend.env`):

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `log_level` | `LOG_LEVEL` | `"debug"`, `"info"`, `"warn"` or `"error"` | `"info"` |
| `hunt_go_now_min_ms` | `HUNT_GO_NOW_MIN_DELAY_MS`; the least wait before a hunt's Go Now starts it | 1 to 600000, not more than the max | 1 |
| `hunt_go_now_max_ms` | `HUNT_GO_NOW_MAX_DELAY_MS`; the most wait | 1 to 600000 | 1000 |
| `xmpp_capture` | `REALTIME_XMPP_CAPTURE`; chat records its traffic for debugging | `true` or `false` | `false` |
| `min_launcher_version` | `DAINTY_MIN_LAUNCHER_VERSION`; the oldest launcher that may play; older ones update first | A launcher version such as `"2.1.0"` or `"2.1.0-rc.1"` | none: any launcher |
| `max_online_players` | `DAINTY_MAX_ONLINE_PLAYERS`; past it, PLAY waits in a queue | 1 to 10000 | none: no limit, no queue |
| `transaction_max_commit_ms` | `MONGODB_TRANSACTION_MAX_COMMIT_TIME_MS`; unused since MongoDB was retired | 1 to 600000 | none |

`[director]` (`director.env`; the Director hands its environment to every game server):

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `log_level` | `LOG_LEVEL` | `"debug"`, `"info"`, `"warn"` or `"error"` | `"info"` |
| `gameserver_startup_gap_seconds` | `SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP` | 0 to 600 | 3 |
| `gameserver_ready_timeout_ms` | `GAMESERVER_READY_TIMEOUT_MS`; how long a game server has to report ready | 1000 to 600000 | 30000 on Windows, 90000 on Linux |
| `hub_max_players` | `DAINTY_HUB_MAX_PLAYERS`; players a hub (Ramsgate) holds | 1 to 64 | `HUB_MAX_PLAYERS.txt` beside the game server, else 4 |
| `hub_afk_seconds` | `DAINTY_HUB_AFK_SECONDS`; when an idle player is kicked from a hub | 0 (never) to 604800; 1 to 59 count as 60 | `HUB_AFK_TIMEOUT_SECONDS.txt`, else 1200 |
| `hunt_afk_seconds` | `DAINTY_HUNT_AFK_SECONDS`; the same on a hunt | as above | `HUNT_AFK_TIMEOUT_SECONDS.txt`, else the game's own |
| `keep_supply_stack_caps` | `DAINTY_KEEP_SUPPLY_STACK_CAPS` (director and backend); the game servers and players' games keep the game's supply stack caps | `true` or `false` | supplies stack uncapped |
| `cell_fusion_minutes` | `DAINTY_CELL_FUSION_MINUTES` (director and backend); how long a Middleman cell fusion takes, also on players' screens | 0 (the game's own) to 10080 | 30 |

### `[env.<service>]`: variables with no key of their own

For a variable a service reads that has no `settings.toml` key, add it to that service's table. It is appended to the
service's env file:

```toml
[env.gateway]
GATEWAY_SOME_VARIABLE = "40,24"
```

The services are `backend`, `director`, `content`, `gateway`, `admin` and `allowlist`. A table holds at most 64
variables; a name is A-Z, 0-9 and `_`, starting with a letter, at most 64 characters, and a value at most 4096 bytes.
Setup refuses a secret, anything `settings.toml` or setup already decides (the server mode, the approved executable's
hash, the multipliers, the registration mode, `NODE_ENV`, `PORT`, `BIND_HOST`, `MY_IP`), and any name that starts with
`AUTH_`, `DAINTY_DB_` or `DAINTY_OAUTH_` or ends in `_SECRET`, `_KEY`, `_KEYS`, `_TOKEN`, `_PASSWORD`, `_B64`, `_PORT`
or `_BIND`. That keeps the passthrough from turning a public server's safety settings off. `doctor` lists every
variable passed this way.

### Environment variables override settings.toml

A variable in the environment `dainty-host` runs in wins over the value setup wrote, when the service reads it: a key
in its env file, or one its table above may set. Each override is a warning every time the service starts, naming the
`settings.toml` key it overrides:

```text
gateway: GATEWAY_RATE_WS=40,24 from the environment overrides settings.toml (gateway.rate.ws = 20,12)
```

A secret's or a safety setting's value is never shown. Overrides are checked by the services' loaders like any other
value, and `doctor` lists them.

Some names are set for every program on a dev box or by a hosting panel: `PORT`, `BIND_HOST`, `NODE_ENV`,
`LOG_LEVEL`, `MY_IP` and `BACKEND_URL`. These are ignored, with a warning, and override only with the service's
prefix: `DAINTY_BACKEND_PORT`, `DAINTY_DIRECTOR_LOG_LEVEL`. A server variable left in your shell from an earlier
session reaches the service too, so prefer `settings.toml` and keep the shell clean.

### Secrets (`secrets.env`)

`setup` makes every secret the first time it runs and keeps them from then on; one that is missing or empty is made on
the next setup. They are never in `settings.toml`, never on a command line and never logged. Never share or commit the
file.

| Key | What it is |
| --- | --- |
| `GATEWAY_SECRET` | Shared by the gateway and the services behind it |
| `ALLOWLIST_SECRET` | The gateway's key to the allowlist helper |
| `DAINTY_ADMIN_KEY` | The backend's admin key, which `invite` uses to make codes |
| `GAMESERVER_API_KEY` | The game servers' key to the backend |
| `API_KEY_HASH_SECRET` | Used by the backend when it hashes keys |
| `RUSTIC_ISLES_METRICS_TOKEN` | Guards the backend's and the admin API's metrics |
| `AUTH_SIGNING_PRIVKEY_B64`, `AUTH_SIGNING_PUBKEY_B64` | The backend's signing key pair |
| `DAINTY_ADMIN_API_KEY` | The admin API's key, which dainty-dash and your own tools send ([admin-api.md](admin-api.md)) |
| `RUSTIC_ISLES_MANAGER_OPERATOR_SECRET` | Signs the admin API's calls to the backend |
| `DAINTY_DB_PASSWORD` | The Postgres role's password |
| `DASHBOARD_INGEST_KEY` | What chat and the content server send dainty-dash |
| `DISCORD_BOT_TOKEN` | Yours to bring: `setup --discord-bot-token` |
| `DAINTY_OAUTH_CLIENT_SECRET` | Yours to bring: `setup --oauth-client-secret` |

Changing a secret breaks everything signed with the old one, so don't. A secret under its name from before the rename
(`REFORGED_ADMIN_KEY`, `MYSTICPARADOX_METRICS_TOKEN` and so on) is moved to its new name with the same value.

### `[ports]`

The TCP ports the services listen on, for a machine where one is taken. Each takes 1 to 65535; no two may be the same,
and none may be the gateway's, the relay's (61100) or a game port. On Windows setup and start fill this in
themselves ([Ports Windows holds](#ports-windows-holds)).

| Key | Becomes | Default |
| --- | --- | --- |
| `gateway` | `GATEWAY_PORT`; a private server's gateway, the port its invites name. A public server's is `public_port` | 61000 |
| `backend` | the backend's `PORT`, and every service's address for it | 41000 |
| `director` | the Director's `PORT` | 41001 |
| `qos` | the backend's QoS listener | 41010 |
| `content` | content's `PORT` | 41002 |
| `admin` | the admin API's `PORT` | 41003 |
| `allowlist` | `ALLOWLIST_PORT` (allowlist), `ALLOWLIST_URL` (gateway) | 41005 |
| `chat` | `CHAT_PORT` (gateway) | 41006 |

### News and session supplies

Two more files in `.workspace/config/` are yours to write, in TOML:

- `news.toml`: what the launcher shows under your server's news, one `[[items]]` with `date`, `title` and `body` each.
  The content server reads it itself. `[content] news_url` replaces it with a file fetched from GitHub.
- `session-supplies.toml`: supplies given to named accounts, one `[accounts.<username>]` table each, with
  `"<catalog id>" = <quantity>` lines. Setup turns it into `.workspace/generated/session-supplies.json` for the backend,
  so a change takes a setup.

An older install's `news.json` and `session-supplies.json` are converted once and kept beside the new files as `.bak`.

### `[guard]`: guarded admin actions

Destructive and bulk admin actions wait before they run. A rule is `"delay"` (wait `delay_minutes`), `"approval"` (a
second operator approves) or `"off"`. Each key goes to `admin.env` only when set; left out, the admin API's default
holds ([admin-api.md](admin-api.md)).

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `ban_limit` | `DAINTY_GUARD_BAN_LIMIT`; bans and timeouts one admin key may make in the window before the next is held | 1 to 1000000 | 10 |
| `ban_window_minutes` | `DAINTY_GUARD_BAN_WINDOW_MINUTES` | 1 to 1440 | 60 |
| `ban_guard` | `DAINTY_GUARD_BAN`; what a ban over the limit waits for | a rule | `"delay"` |
| `revert_limit` | `DAINTY_GUARD_REVERT_LIMIT`; a history revert of more changes than this is held | 1 to 1000000 | 100 |
| `revert_guard` | `DAINTY_GUARD_REVERT` | a rule | `"delay"` |
| `revoke_guard` | `DAINTY_GUARD_REVOKE`; revoking an admin or owner key (other keys go at once) | a rule | `"delay"` |
| `delay_minutes` | `DAINTY_GUARD_DELAY_MINUTES` | 1 to 1440 | 10 |
| `expire_hours` | `DAINTY_GUARD_EXPIRE_HOURS`; how long an action nobody approved stays | 1 to 720 | 48 |

### `[discord]`: the Discord bot

The bot hands a member a signup link or a single-use invite with `/key`, and says whether the game is up with
`/status`. It gets an env file of its own, `discord.env`, so its token reaches no other service; the file is written
only when the bot is on and its token is set.

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `enabled` | whether `discord.env` is written, with `DISCORD_BOT_TOKEN` from `secrets.env` | `true` or `false` | `false` |
| `application_id` | `DISCORD_APPLICATION_ID`; checked against the token's | The application's id, digits | not checked |
| `guild_ids` | `DISCORD_GUILD_IDS`; the Discord servers it answers in | A list of ids as strings | every one it is in |
| `dashboard_url` | `DISCORD_DASHBOARD_URL`; dainty-dash, asked whether signups are on | An `http://` or `https://` address | `dashboard_url` above |

```powershell
Get-Content token.txt | bin\dainty-host setup --discord-bot-token --discord-guild 123456789012345678
```

### `[oauth]`: signing in with Discord

Players make their account with "Continue with Discord" (or your own provider) in the launcher. Make an application at
<https://discord.com/developers/applications>, add `http://127.0.0.1:61190/oauth/callback` under OAuth2 > Redirects,
then give setup its client id and secret. Sign-in stays off until the secret is set. Every key goes to the backend's
env file as `DAINTY_OAUTH_<KEY>`. [oauth.md](oauth.md) covers it in full.

| Key | Takes | Default |
| --- | --- | --- |
| `preset` | `"discord"`, or `"custom"` for any OAuth 2.0 provider with PKCE and a JSON userinfo endpoint | required |
| `client_id` | The application's client id | required |
| `guild_ids` | Discord only: players must be in one of these servers (ids as strings) | anyone with a Discord account |
| `required` | `true`: new accounts are made only by signing in. Sign-up still follows `registration` | `false` |
| `provider_id` | custom: the id accounts are linked under; lower-case letters, digits and `-`, not `discord` | required for custom |
| `name` | custom: the name on the button ("Continue with {name}"), up to 32 characters | required for custom |
| `authorize_url`, `token_url`, `userinfo_url` | custom: the provider's endpoints, `https://` | required for custom |
| `scopes` | custom: the scopes asked for | `["openid", "profile"]` |
| `subject_field` | custom: the userinfo field with the user's stable id | `"sub"` |
| `username_field` | custom: the userinfo field with their name | `"preferred_username"` |

```powershell
bin\dainty-host setup --oauth-discord 123456789012345678 --oauth-guild 123456789012345678
Get-Content secret.txt | bin\dainty-host setup --oauth-client-secret
```

### `[modules]` and `[services]`: game modules and plugins

The 1.14.7 game module is built into `dainty-host`. These tables load others, or load the built-in parts from
`plugins/` instead; most servers need neither.

| Key | Takes | Default |
| --- | --- | --- |
| `modules.compiled` | The compiled-in modules to load, by id (`["isles-1-14-7"]`) | every compiled-in module |
| `[[modules.external]]` `id`, `path`, `sha256` | A program the server runs as a module, by its path (from the server folder) and its SHA-256 | none |
| `[[modules.external]]` `args` | Its arguments, up to 64 | none |
| `[[modules.external]]` `autoload` | `false`: listed, loaded from the console only | `true` |
| `[[modules.external]]` `restart` | `true`: started again when it dies, with backoff | `false` |
| `[modules.<id>.<section>]` | A module's own settings, as it registered them | none |
| `[services.<name>]` `library`, `sha256` | A service loaded as a library (its path from the server folder, and its SHA-256) in place of the built-in one. `content`, `chat` and `gateway` are known by name | the built-in ones |
| `[services.<name>]` `serves` | For a service the core doesn't know by name: what it serves | none |

A file whose SHA-256 does not match is not loaded.

### `[network]`: a network of servers

Several public servers can form one network that shares accounts and lets players hunt across regions. Each server is a
node with a certificate from a cluster CA you keep off the nodes. [distributed-mode.md](distributed-mode.md) covers
failover; the commands are under `dainty-host network`.

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `node` | `GATEWAY_NODE_ID` (gateway) | This node's name: a-z, 0-9 and `-`, such as `oregon-1` | required |
| `region` | `GATEWAY_NODE_REGION` (gateway) | The region it serves, such as `oregon` | required |
| `leader` | the server | The directory leader's node id | the alphabetically first node id |
| `wrong_node_policy` | the server | `"refuse"` (a join that reaches the wrong node is refused, naming the right one) or `"redirect"` (sent on to it) | `"refuse"` |
| `failover` | the server | `true`: the nodes elect the leader and replace it when it goes down. Leave `leader` out, use three or five nodes | `false` |
| `heartbeat_ms` | the server | 10 to 600000 | 1000 |
| `election_timeout_ms` | the server | 10 to 600000 | 5000 |
| `[[network.nodes]]` | `GATEWAY_NETWORK_NODES` (gateway); only these nodes may call this one | `id`, `region`, `host`, `port`, and optionally `fingerprint` (the peer's public certificate, listed to players) and `link` (a `host:port` node links dial instead, such as a Tailscale address) | no peers |
| `[history] ship_to_peer` | the server | A peer node id from `[[network.nodes]]` that receives this node's change log and snapshots | off |

Peers need `mode = "public"`: node links run over the gateway's TLS port. The node's certificate, key and the CA's
certificate go in `.workspace/tls/` as `node-cert.pem`, `node-key.pem` and `cluster-ca-cert.pem`.

```powershell
bin\dainty-host network ca-init                    # on your own PC: cluster-ca.pem, which signs every node
bin\dainty-host network issue-node-cert --ca cluster-ca.pem --node uk-1 --region uk --host uk.example.org
bin\dainty-host network list                       # this server's node and peers (--ca: what a CA issued)
bin\dainty-host network status                     # pings every peer over the node link
bin\dainty-host network join                       # joins this server's database to the network's directory
```

### `[split]`: one server on two machines

The metagame (backend, content, admin) on one machine and the game servers (the Director and what it starts) on
another, both nodes of one `[network]` ([split-roles.md](split-roles.md)).

| Key | Becomes | Takes | Default |
| --- | --- | --- | --- |
| `role` | `RUSTIC_ISLES_PORT_SERVICES` (backend) | `"metagame"` or `"gameservers"` | required |
| `peer` | the server | The other half's node id, listed in `[[network.nodes]]` | required |
| `open_game_ports` | the server | Game servers half in public mode: `true` runs with the game ports open to everyone, since players' sign-ins reach the other half's gateway | `false` |

### `[remote]`: managing several installs

One install, the controller, can manage others over node links: push a shared configuration, run commands and see
their state ([remote-management.md](remote-management.md)).

| Key | Takes | Default |
| --- | --- | --- |
| `role` | `"standalone"`, `"controller"` or `"managed"` | `"standalone"` |
| `node`, `region` | This install's node id and region, when `[network]` is not set | |
| `controller_url` | Managed: the controller gateway's `https://` URL | required when managed |
| `controller_node` | Managed: the controller's node id | required when managed |
| `controller_cert_fp` | Managed: the controller certificate's SHA-256 | required when managed |
| `sync_interval_secs` | Managed: seconds between syncs, 1 to 86400 | 10 |
| `label` | The install's name in the controller's list, 1 to 64 characters | |
| `local_overrides` | Managed: dotted `settings.toml` keys this install keeps its own value for | none |

On a public server, a controller or managed install gets its node settings in the gateway's env file as a network
node does; node links run over the gateway's TLS port.

```powershell
bin\dainty-host remote init-controller             # on the controller
bin\dainty-host remote invite --node alpha         # a single-use enrollment invite
bin\dainty-host remote enroll "<invite>"           # on the install
bin\dainty-host remote list
bin\dainty-host remote set-config                  # publishes the controller's settings.toml to the installs
bin\dainty-host remote exec --node all --action doctor
```

`remote exec` takes `start`, `stop`, `restart`, `setup`, `setup-fresh-start`, `doctor`, `backup`, `ban`, `unban` and
`kick`; a guarded one waits for `remote approve <id>` or `remote cancel <id>`. `remote status`, `revoke <node>`,
`leave` and `sync [--loop]` do what they say.

### `[logs]` and `[events]`

Both are read by the server itself ([logging.md](logging.md)).

| Key | Takes | Default |
| --- | --- | --- |
| `logs.format` | `"text"` (`service \| time LEVEL module: message key=value`) or `"json"` (one object a line) | `"text"` |
| `logs.color` | `"auto"` (on a terminal, unless `NO_COLOR` is set), `"always"` or `"never"` | `"auto"` |
| `logs.gelf_url` | A GELF collector (Graylog) to ship every record to: `https://host:12201/gelf`, or `http://` or `tcp://host:port` on this machine or a private network | none |
| `events.retention_days` | Days the registry keeps an event, 0 to 3650; 0 keeps every event | 90 |

Secrets and account keys are redacted from every log line.

## Updating

```powershell
bin\dainty-host update --check     # says whether a newer release is out
bin\dainty-host update             # installs it, then runs setup again
bin\dainty-host update --no-setup  # installs it and stops there
```

`update` finds the newest server release for this platform, checks it against the release's `SHA256SUMS.txt` (and its
build attestation, when the `gh` CLI is installed), unpacks it and installs it over the server folder. It refuses to
overwrite a release file you changed, and lists them, so move your changes out first. A release holds no
`.workspace`, so your settings, secrets and database stay as they are. Then it runs the new
`dainty-host setup --skip-install`, which rewrites the env files for the new release. When the game runtime changed
it says so: players' launchers will ask them to accept the new one. Restart the server afterwards so it runs the new
binaries. `--json` prints the result as JSON.

An update that stopped halfway is finished by running `update` again. `DAINTY_SERVER_RELEASE` points `update` at
another source: a GitHub `owner/repo`, an `https://` URL or a folder holding the release files.

Before updating a live server:

1. `bin\dainty-host update --check`, and read the release notes.
2. `bin\dainty-host backup` (or `pg_dump` for Postgres).
3. Stop the server, then `bin\dainty-host update`.
4. `bin\dainty-host doctor`, then start the server again.

### Upgrading from an older install

`setup` (which `update` runs) brings an older server folder up to date by itself:

- Game data in the retired `modules/rustic-isles/reference/1.14.7/` tree, or in a checkout's `restricted/`, is moved
  into `modules/rustic-isles/game-data/1.14.7/`. A file already there is never overwritten; setup says what it left
  behind, so you can delete the old folder.
- `[mongo]` is dropped from `settings.toml`, and the env files a 1.4.4 edition used are removed.
- Secrets under their old names are moved to their new ones with the same values.
- `news.json` and `session-supplies.json` become TOML.

An install that predates the Rust server is imported with `dainty-legacy`, a separate program in `bin/`. Its kinds are
`reforged` (a Node kit with `host.json` and MongoDB), `revived` (a Dauntless Revived 1.4.4 server) and `offline-save`:

```powershell
bin\dainty-host legacy import reforged D:\old\server --program bin\dainty-legacy.exe
bin\dainty-host legacy import reforged D:\old\server --program bin\dainty-legacy.exe --dry-run
```

The source is an absolute path. `reforged` turns `host.json` into `settings.toml`, moves the MongoDB data into the
new database and the old folders' contents to where Dainty Lass looks. `--fresh-start` takes the settings and moves no
data. `revived` and `offline-save` need `--to <database>` or `--env-file <file>` to name the database to import into.
`--program` is needed until `settings.toml` pins `dainty-legacy` as a `[[modules.external]]` with `id = "legacy"`.
A server folder with only a Node kit's `host.json` refuses every other command until it is imported.

`dainty-host db migrate` and `db verify` are the MongoDB steps on their own, for checking an import by hand.

## Backups

```powershell
bin\dainty-host backup
```

`backup` copies the SQLite database into `.workspace/backups/sqlite-<date>-<time>.db` and says how to restore it:
with the server stopped, copy it over `.workspace/config/dainty-lass.db`. A Postgres database is yours to back up with
`pg_dump`. Back up `.workspace/config/` and `.workspace/tls/` as well: without `secrets.env` and the gateway
certificate the database is no good to players.

### Undoing a change

Every change to an account is kept in a hash-chained change log.

```powershell
bin\dainty-host history --account <account id> --since 2026-11-01
bin\dainty-host history verify                      # checks the chain is unbroken
bin\dainty-host revert --account <account id> --since 2d --dry-run
bin\dainty-host revert --account <account id> --since 2d --apply <plan id>
```

`history` lists the changes, 50 a page (`--limit`, `--cursor`), filtered by `--account`, `--actor`, `--since` and
`--until`. A time is RFC 3339 (`2026-10-27T10:30:00Z`), a date, or how long ago (`6h`, `2d`). `revert` always
starts with a dry run that shows what would change and a plan id; `--apply <plan id>` applies it only if the same
selection still plans the same changes.

## Troubleshooting

| What you see | What to do |
| --- | --- |
| `server is not configured yet: run dainty-host setup first` | Run setup. `doctor` says the same |
| `missing backend.env, ... run dainty-host setup first` | Setup has not written the env files yet, or they were deleted: run setup |
| `settings.toml has no setting ...` | A typo or an old key: the message names the right one |
| `.workspace/config/host.json is a Node kit's` | Import it first ([Upgrading from an older install](#upgrading-from-an-older-install)) |
| `Tailscale is not running here` | Start Tailscale and sign in, or pass `--host <your Tailscale IP>` |
| `the host is a DNS name: pass --game-address ...` | Give the IPv4 address the game servers announce |
| `gateway port N is already in use` | Another program holds it; the message names it when it can. Stop it, or move the port (`public_port` or `[ports] gateway`) |
| os error 10013 on Windows | A port Windows lent out; run `setup` or `start` again so they move it |
| `published runtime is missing ...: no game server can start` | `dainty-host publish-runtime` |
| Players can't reach a public server | Open the gateway's TCP port and the UDP game range on the machine and the router; check `doctor` |
| `ALLOWLIST_DRY_RUN=0 needs administrator rights` | A public server on Windows: run `start` as administrator |
| `gateway ... is serving certificate ..., which does not match` | The certificate on disk changed while the gateway ran: restart the server |
| `registration is disabled (NONE)` | Set `registration` to `INVITECODE` or `OPEN` and run setup |
| `database.postgres_host ... is not on this machine` | Use 127.0.0.1, or set `allow_remote = true` over a private network |
| `settings.toml's [env] tables can't be applied` | The listed variable is a secret or one setup decides; use its key instead |
| A warning that a variable from the environment overrides settings.toml | Unset it in your shell, or move the value into `settings.toml` |
| `edition "legacy" and "both" ran Dauntless Revived's 1.4.4 ...` | Remove `edition`; import the 1.4.4 server with `legacy import revived` |
| Players see the example game data | Generate your own ([Game data, the runtime and the SDK](#game-data-the-runtime-and-the-sdk)) |

When something else is wrong, run `doctor` first, then look in `.workspace/logs/`. The server's console names the
service on every line.
