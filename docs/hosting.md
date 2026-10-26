# Hosting a server on Windows

One PC runs the whole server: the gateway, the backend and Director, the content server, MongoDB and the
game servers. A **private** server is reachable by friends on your tailnet; a **public** one by anyone with an
invite, over TLS. The host kit, `reforged-host`, does the setting up; the launcher's Host page drives the same
commands with buttons.

Linux: [hosting-linux.md](hosting-linux.md). Docker: [hosting-docker.md](hosting-docker.md).

## What you need

- Windows 10 or 11, and **your own Dauntless 1.14.7** install (the folder with `Archon` in it).
- The **server release** (`ReforgedRevivalServer-<version>-windows-x64.zip`), unpacked anywhere; `bin\reforged-host.exe`
  is the kit. Or a checkout of this repository built from source.
- **Node.js 22** and **MongoDB** (`mongod`): the backend and the Director are still Node, and the kit runs
  `mongod` for you unless you point it at your own (`--mongo-uri`, `--no-manage-mongo`).
- **Visual Studio 2022** with Desktop development with C++, and **Dumper-7**, to generate the SDK and build
  the game runtime from your own game ([sdk-and-game-data.md](sdk-and-game-data.md)). Players get the runtime
  you build; nobody else's works with your server's build.
- Private: **Tailscale** on your PC and your friends' PCs. Public: a public IPv4 or a DNS name, and TCP 443
  and UDP 8770-8777 open on your router and firewall.

## Setting up

A private server, from the release folder:

```powershell
bin\reforged-host setup --mode private --name "Friday Hunts" --game "D:\Games\Dauntless"
```

`--host` defaults to this PC's Tailscale address. A public one:

```powershell
bin\reforged-host setup --mode public --host play.example.org --game-address 203.0.113.10 --name "Friday Hunts" --game "D:\Games\Dauntless"
```

`setup` writes `.workspace\config\host.toml` (every setting, with a comment each) and `secrets.env` (made
once, never shown), installs the Node services (`--skip-install` to keep yours), starts MongoDB's replica set,
and in public mode makes the gateway's certificate. **It never remakes that certificate**: every invite's
`fp` depends on it. Run `setup` again with different options to change a setting, or edit `host.toml` and run
`setup` with none.

Then the SDK, the runtime and the game data ([sdk-and-game-data.md](sdk-and-game-data.md)):

```powershell
bin\reforged-host gamedata                  # from your CatalogExporter export in paradox\Items_Analysis
bin\reforged-host publish-runtime           # the MysticParadox.dll and winmm.dll you built
bin\reforged-host doctor                    # what is still missing or wrong
```

## Running it

```powershell
bin\reforged-host start                     # every service, until Ctrl+C
bin\reforged-host invite --for "Alex"       # an invite link to send
bin\reforged-host invite --list
```

`start` runs one process tree and stops all of it on Ctrl+C. The server GUI build of the launcher runs the
same tree from its Host page. Logs are in `.workspace\logs` (the game servers' in `logs\gameservers`).

| Command | What |
| --- | --- |
| `setup` | Set up or change the server (see the options below) |
| `start` | Run every service |
| `invite` | Make an invite (`--uses N`, `--infinite`, `--for who`), or `--list` them |
| `doctor` | Check the game, the SDK, the builds, the network, the tools and the services |
| `gamedata` | Generate the game data from your export (`--examples` for a start-up test only); `--export` and `--import` move it as a pack |
| `publish-runtime` | Publish the runtime for players (`--from <folder>` for DLLs built elsewhere) |
| `preset` | Keep your game data as a preset (`--create`), or apply one (`--apply`) |
| `multipliers` | XP and loot multipliers for a private test server (`--xp 5 --rewards 5`, `--off`) |
| `update` | Install a newer server release and run `setup` again (`--check` only compares) |
| `backup` | Dump the database |

## Settings

`setup`'s options and the `host.toml` keys they write. A secret (the admin key, the gateway secret, the game
servers' key) lives only in `secrets.env`.

| Option | `host.toml` | What |
| --- | --- | --- |
| `--mode private\|public` | `mode` | Friends over Tailscale, or the internet |
| `--edition reborn\|legacy` | `edition` | Dauntless 1.14.7 (the default), or a Dauntless Revived 1.4.4 server |
| `--host` | `host` | The Tailscale IP, or the public IPv4 or DNS name |
| `--game-address` | `game_address` | The IPv4 the game servers announce (needed on a DNS name) |
| `--name` | `name` | The server's name, as players see it |
| `--game` | `game_dir` | The Dauntless folder |
| `--registration INVITECODE\|OPEN\|NONE` | `registration` | Who may register |
| `--share` | `share_url` | A Tailscale share link put in private invites |
| `--mongo-uri`, `--manage-mongo`, `--no-manage-mongo`, `--mongod` | `[mongo]` `uri`, `manage`, `mongod` | The database |
| `--max-hunts 1-10` | `max_hunt_servers` | Hunts at once; the UDP range widens below 8770 past six |
| `--xp`, `--rewards`, `--no-multipliers` | `[multipliers]` | Private test servers only |
| `--public-port` | `public_port` | Another port than 443 |
| `--server-fps`, `--idle-fps`, `--training-on-demand`, `--training-idle-minutes`, `--verbose-game-logs`, `--memory-guard`, `--min-free-memory-mb`, `--hunt-memory-mb` | `[performance]` | What the game servers cost in CPU and RAM |
| `--wine`, `--proton`, `--xvfb`, `--interface` | `[linux]` | Linux only ([hosting-linux.md](hosting-linux.md)) |
| (edit `host.toml`) | `[database]` `engine`, `postgres_host`, `postgres_port`, `postgres_user`, `postgres_database`, `allow_remote` | The Rust services' database: `sqlite` (the default, `dainty-lass.db` in `.workspace\config`) or `postgres` (below) |

### The database engine

Left out, `[database]` means SQLite: one file, nothing to install. A dedicated host can pick Postgres:

```toml
[database]
engine = "postgres"
# postgres_host = "127.0.0.1"   # 5432, role dainty, database dainty when left out
```

Postgres is yours to run: make a role and a database for it, with the role's password set to
`REFORGED_DB_PASSWORD` from `secrets.env` (setup makes it once), then run `setup` again. The connection is not
encrypted, so `setup` refuses a Postgres server on another machine unless `allow_remote = true`, which is only for a
network you trust. Nothing stores its data there yet: the backend is still Mystic Paradox's on MongoDB, and the
setting is for the Rust services as they move over. `reforged-host` does not start a Postgres of its own yet.

Each service reads one env file `setup` writes into `.workspace\config` (`gateway.env`, `content.env`,
`backend.env`, `director.env`, `allowlist.env`, `admin.env`), and nothing else. Don't edit them: `setup`
rewrites them from `host.toml`.

## Public servers

- The invite carries the certificate's SHA-256; players' launchers connect to nothing else. Keep
  `.workspace\tls` with your backups.
- The **allowlist** opens UDP 8770-8777 only to players who logged in, through Windows Firewall rules. It runs
  as administrator; `start` asks.
- **Registration**: `INVITECODE` (the default) needs a code per player, `OPEN` lets anyone with the address
  register, `NONE` closes it.
- A ban: `reforged-host` has no command for it yet; the admin API ([admin-api.md](admin-api.md)) and the
  backend's `SetAccountStatus` route do it.

## Updating

```powershell
bin\reforged-host update --check            # is there a newer server release?
bin\reforged-host update                    # install it and run setup again
```

`update` keeps `.workspace` and runs `setup` from your `host.toml`. When `native\` changed, it says so: rebuild
the runtime and publish it again. Players' launchers notice the new hashes and ask once more.

## Known limits

- The backend and the Director are Mystic Paradox's Node services with this project's patches
  ([paradox-patches.md](paradox-patches.md)); the Rust port is in progress ([../MIGRATION_PLAN.md](../MIGRATION_PLAN.md)).
- The launcher's page is a stand-in until `dainty-lass-ui` lands.
- One server runs one game version. Several versions on one host come with the module framework
  ([dainty-lass-plan.md](dainty-lass-plan.md)).
