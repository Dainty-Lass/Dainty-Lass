# Hosting a server on Windows

One PC runs the whole server: the gateway, the backend and Director, the content server, MongoDB and the
game servers. A **private** server is reachable by friends on your tailnet; a **public** one by anyone with an
invite, over TLS. The host kit, `reforged-host`, does the setting up; the launcher's Host page drives the same
commands with buttons.

Linux: [hosting-linux.md](hosting-linux.md). Docker: [hosting-docker.md](hosting-docker.md).

## What you need

- Windows 10 or 11, and **your own Dauntless 1.14.7** install (the folder with `Archon` in it), or an Epic
  account whose library has Dauntless, from which `install-game` installs it ([below](#getting-the-game-from-epic)).
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

No 1.14.7 folder yet? `install-game` fetches one from your own Epic library
([below](#getting-the-game-from-epic)) and points `game_dir` at it.

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
| `install-game` | Install Dauntless 1.14.7 (or 1.12.0) from your own Epic library for the game servers (`--version`, `--dir`, `--code-file`; `--sign-out`) |
| `preset` | Keep your game data as a preset (`--create`), or apply one (`--apply`) |
| `multipliers` | XP and loot multipliers for a private test server (`--xp 5 --rewards 5`, `--off`) |
| `update` | Install a newer server release and run `setup` again (`--check` only compares) |
| `backup` | Dump the database |
| `network` | The cluster CA and node certificates of a network (`ca-init`, `issue-node-cert`, `list`, `status`; see [A network of servers](#a-network-of-servers)) |
| `history` | What changed in the server's data, by whom and when (`--actor`, `--account`, `--since`, `--until`); `history verify` checks the log's hash chain. See [Undoing a change](#undoing-a-change) |
| `revert` | Undo what an actor, an account or a time range changed: a dry run first, then `--apply <plan id>` |


## Getting the game from Epic

The game servers run your own Dauntless. If you own it on Epic, the host kit installs it from Epic's servers, as
the launcher does for players (DaintyLass never hosts or mirrors game files):

```powershell
bin\reforged-host install-game                       # 1.14.7, into .workspace\game\1.14.7
bin\reforged-host install-game --dir D:\Games\Dauntless   # into D:\Games\Dauntless\1.14.7
bin\reforged-host install-game --version 1.12.0
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

Each version gets a folder of its own under `--dir` (`.workspace\game` unless given), so several sit side by side.
Installing **1.14.7** also sets `game_dir` in `host.toml` to its folder and rewrites the env files, so the
Director's game servers run from it: restart the server for that to take effect. Without a `host.toml` yet, it
tells you the `--game` to give `setup`. 1.12.0 is only installed (`game_dir` is 1.14.7's). **1.4.4 cannot be
installed from Epic** (there is no archived manifest): use your own folder with `setup --legacy-game`.

## Undoing a change

Every change to the server's data is in a change log with who made it, when, and the row before and after
([DD23](dainty-lass-plan.md)). A partner who turns on you, a bot that deletes the wrong accounts or an admin key
used by someone it wasn't issued to can be seen and undone from the host, with no key:

```powershell
bin\reforged-host history --actor admin:58ef5419 --since 2d       # what that key did in the last two days, newest first
bin\reforged-host history --account <account id>                  # everything that touched one player's rows
bin\reforged-host revert --actor admin:58ef5419 --since 2d        # a dry run: prints what would change, and a plan id
bin\reforged-host revert --actor admin:58ef5419 --since 2d --apply <plan id>
bin\reforged-host history verify                                  # is the log itself intact?
```

- **Who and when.** `--actor` is `admin:<key id>` (the ids are in `admin key list`; a bare id works, `owner` is
  `REFORGED_ADMIN_API_KEY`), `host:console`, `player:<account>` or `system:<task>`. A time is RFC 3339
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
a long list is cut off at 200 lines, so read a mass revert in a terminal), in the server GUI's commands (its
screen comes with DU3) and in the admin API for the owner's key ([admin-api.md](admin-api.md)). In Docker:
`docker compose run --rm host history --actor admin:58ef5419`.

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
| (edit `host.toml`) | `[guard]` `ban_limit`, `ban_window_minutes`, `ban_guard`, `revert_limit`, `revert_guard`, `revoke_guard`, `delay_minutes`, `expire_hours` | When a destructive or bulk admin action waits for a delay or a second operator (below). Left out: ten bans an hour from one key, a revert of over a hundred changes and the revoking of an admin or owner key each wait ten minutes; everything smaller runs at once |
| (edit `host.toml`) | `[history]` `ship_to_peer` | Ship this node's change log and database snapshots over `Service::LogShip` (`/__node/log_ship`) to a peer node in `[[network.nodes]]` (DH7, below). Left out: off |
| (edit `host.toml`) | `[modules]` `compiled`, `[[modules.external]]` `id`, `path`, `sha256`, `args`, `autoload`, `[modules.<id>.<section>]` | The game modules and plugins to load (below). Left out: every compiled-in one (this build has none yet) and no other |
| (edit `host.toml`) | `[network]` `node`, `region`, `leader` | This server's name, region and optional directory leader in a network. Left out: a server on its own |
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

`reforged-host admin guarded list`, `approve <id>` and `cancel <id>` decide as the host, and the server GUI Host page
lists what waits. A rule on `"approval"` on a server with one owner key waits for nobody and expires after
`expire_hours`: use it once the team has a second key. How it works, and what a bot sees, is in
[admin-api.md](admin-api.md#guarded-actions-dh6).

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
issuing a new one before `reforged-host network list` says "expires soon"; the server logs a warning 30 days ahead.
The `fingerprint` is what the launcher pins when it measures the ping to that region; a peer without one is
still a peer, only not listed to players. `reforged-host network list` shows this server's place in the network,
or, with `--ca <file>`, what a CA has issued. `reforged-host network status` pings every peer over the node link now
with this server's own certificate and shows each one's state, round trip and clock skew (`--json` for scripts; the exit
code is 1 when a peer is not up). Docker's `network` profile runs four such nodes with injected latency, for testing
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
