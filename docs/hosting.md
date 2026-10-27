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
bin\reforged-host gamedata                  # from your CatalogExporter export in modules\rustic-isles\reference\1.14.7\Items_Analysis
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
| `network` | The cluster CA and node certificates of a network (`ca-init`, `issue-node-cert`, `list`, `status`; see [A network of servers](#a-network-of-servers)) |

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
| (edit `host.toml`) | `[modules]` `compiled`, `[[modules.external]]` `id`, `path`, `sha256`, `args`, `autoload`, `[modules.<id>.<section>]` | The game modules and plugins to load (below). Left out: every compiled-in one (this build has none yet) and no other |
| (edit `host.toml`) | `[network]` `node`, `region` | This server's name and region in a network. Left out: a server on its own |
| (edit `host.toml`) | `[[network.nodes]]` `id`, `region`, `host`, `port`, `fingerprint` | The other nodes of the network: the only ones this node answers, and the ones it calls (below). Left out: no peers, and the server works as one outside a network |

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

### Game modules and plugins

A game module serves one exact game version; a plugin extends the module of its version. A module compiled into
`reforged-host` is loaded unless `compiled` leaves it out; any other is an executable you list with its SHA-256,
and nothing else is ever started. Its path is taken from the server folder (the one that holds `.workspace`), and
that folder must be writable by the server's account only.

```toml
[modules]
# compiled = ["isles-1-14-7"]   # left out: every compiled-in module

[[modules.external]]
id = "rustic-freeza"
path = "modules/rustic-freeza.exe"
sha256 = "<the file's SHA-256: certutil -hashfile <file> SHA256, or sha256sum>"
# autoload = false              # listed, but loaded from the console only

[modules.rustic-freeza.tuning]  # a module's own settings: the ones it says it reads
```

Two game modules for one version are refused, whichever loads first, and so is one for a version this server's
own stack serves (`edition`). A port a module asks for that something already holds is moved up past it. In the
console (`serve`), `modules` lists them; `module load|unload|restart|status <id>` and `module run <id> <command>`
act on one, and `start`, `stop` and `restart` take a module's id too. The admin API has `modules`, `loadModule`,
`unloadModule` and `restartModule`. A module whose process dies shows as failed until it is loaded again; an
unload stops it and kills everything it started.

Each service reads one env file `setup` writes into `.workspace\config` (`gateway.env`, `content.env`,
`backend.env`, `director.env`, `allowlist.env`, `admin.env`), and nothing else. Don't edit them: `setup`
rewrites them from `host.toml`.

### A network of servers

Several public servers, one in each region, can form one network (DD16). Each server stays an ordinary server; they
call each other through their gateways, on the same TLS port players use, under a certificate authority you run.
The steps are yours to do by hand:

1. On your own machine (not a server), make the CA once: `reforged-host network ca-init --out cluster-ca.pem`. The
   file holds the CA's key. Back it up, and never copy it to a server.
2. For each server, issue its certificate from that file:
   `reforged-host network issue-node-cert --ca cluster-ca.pem --node oregon-1 --region oregon --host 203.0.113.7`
   (`--host` is the address its gateway answers on, an IP or a DNS name, as the other nodes will list it; repeat it
   for more; `--days` sets how long it is valid, 365 by default). It writes `node-cert.pem`, `node-key.pem` and
   `cluster-ca-cert.pem` to a folder named after the node.
3. Copy those three files to the server's `.workspace/tls` folder, next to the gateway's own certificate.
4. In each server's `host.toml`, name it and list the others, then run `setup` again:

```toml
[network]
node = "oregon-1"
region = "oregon"

[[network.nodes]]
id = "uk-1"
region = "uk"
host = "uk.example.org"     # an IP address or a lower-case DNS name, as in the certificate
port = 443
fingerprint = "..."         # optional: that server's public certificate, from `gateway make-cert --fingerprint`
```

A node answers only the nodes it lists, and a certificate from the CA is not enough: to cut a node off, remove it
from the other nodes' lists and run `setup` again. Node links need `mode = "public"`. Certificates are renewed by
issuing a new one before `reforged-host network list` says "expires soon"; the server logs a warning 30 days ahead.
The `fingerprint` is what the launcher pins when it measures the ping to that region; a peer without one is
still a peer, only not listed to players. `reforged-host network list` shows this server's place in the network,
or, with `--ca <file>`, what a CA has issued. `reforged-host network status` pings every peer over the node link now
with this server's own certificate and shows each one's state, round trip and clock skew (`--json` for scripts; the exit
code is 1 when a peer is not up). Docker's `network` profile runs four such nodes with injected latency, for testing
([hosting-docker.md](hosting-docker.md#the-network-profile-four-nodes-on-one-machine)).

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
