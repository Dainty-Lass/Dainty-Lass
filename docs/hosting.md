# Hosting a server on Windows

One PC runs the whole server: the gateway, the 1.14.7 Rust backend and Director (`isles-1-14-7`), the content
server, the admin API, the SQLite (or Postgres) store and the game servers. A **private** server is reachable by
friends on your tailnet; a **public** one by anyone with an invite, over TLS. The host kit, `reforged-host`, does
the setting up from a terminal; **DaintyLass Manager** (`dainty-lass-manager`, or the client launcher's Host page)
drives the same server and setup with a graphical interface.

Linux: [hosting-linux.md](hosting-linux.md). Docker: [hosting-docker.md](hosting-docker.md).

## What you need

- Windows 10 or 11, and **your own Dauntless 1.14.7** install (the folder with `Archon` in it), or an Epic
  account whose library has Dauntless, from which `install-game` installs it ([below](#getting-the-game-from-epic)).
- The **server release** (`ReforgedRevivalServer-<version>-windows-x64.zip`), unpacked anywhere; `bin\reforged-host.exe`
  is the kit. Or a checkout of this repository built from source. No Node.js or MongoDB is needed on the default
  path: the 1.14.7 backend, Director, gateway, content server, allowlist helper and admin API are compiled into
  `reforged-host` and store player data in `.workspace\config\dainty-lass.db` (SQLite) or Postgres (`[database]`).
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

`setup` writes `.workspace\config\host.toml` (every setting, with a comment each), `secrets.env` (made
once, never shown) and the service `.env` files, and in public mode makes the gateway's certificate.
**It never remakes that certificate**: every invite's `fp` depends on it. Run `setup` again with different
options to change a setting, or edit `host.toml` and run `setup` with none.

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

`start` runs one process tree and stops all of it on Ctrl+C. **DaintyLass Manager** (`dainty-lass-manager`, built
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


## Getting the game from Epic

The game servers run your own Dauntless. If you own it on Epic, the host kit installs it from Epic's servers, as
the launcher does for players (DaintyLass never hosts or mirrors game files):

```powershell
bin\reforged-host install-game                       # 1.14.7, into .workspace\game\1.14.7
bin\reforged-host install-game --dir D:\Games\Dauntless   # into D:\Games\Dauntless\1.14.7
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
a long list is cut off at 200 lines, so read a mass revert in a terminal), in **DaintyLass Manager** (`dainty-lass-manager`, History tab) and in the admin API for the owner's key ([admin-api.md](admin-api.md)). In Docker:
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
| `--mongo-uri` | `[mongo]` `uri` | Existing MongoDB URI to migrate pre-cutover player data from on `setup` / `update` (or for a `--features paradox` rollback build; the default server uses `[database]`) |
| `--max-hunts 1-10` | `max_hunt_servers` | Hunts at once; the UDP range widens below 8770 past six |
| `--xp`, `--rewards`, `--no-multipliers` | `[multipliers]` | Private test servers only |
| `--public-port` | `public_port` | Another port than 443 |
| `--server-fps`, `--idle-fps`, `--training-on-demand`, `--training-idle-minutes`, `--verbose-game-logs`, `--memory-guard`, `--min-free-memory-mb`, `--hunt-memory-mb` | `[performance]` | What the game servers cost in CPU and RAM |
| `--wine`, `--proton`, `--xvfb`, `--interface` | `[linux]` | Linux only ([hosting-linux.md](hosting-linux.md)) |
| `--fresh-start` (or edit `host.toml`) | `[database]` `engine`, `postgres_host`, `postgres_port`, `postgres_user`, `postgres_database`, `allow_remote`, `fresh_start` | The Rust services' database: `sqlite` (the default, `dainty-lass.db` in `.workspace\config`) or `postgres` (below); `--fresh-start` / `fresh_start = true` starts with an empty database without migrating existing MongoDB data (MongoDB is never modified, and removing `fresh_start` from `host.toml` and running `setup` again clears the opt-out marker and runs the migration) |
| (edit `host.toml`) | `[guard]` `ban_limit`, `ban_window_minutes`, `ban_guard`, `revert_limit`, `revert_guard`, `revoke_guard`, `delay_minutes`, `expire_hours` | When a destructive or bulk admin action waits for a delay or a second operator (below). Left out: ten bans an hour from one key, a revert of over a hundred changes and the revoking of an admin or owner key each wait ten minutes; everything smaller runs at once |
| (edit `host.toml`) | `[history]` `ship_to_peer` | Ship this node's change log and database snapshots over `Service::LogShip` (`/__node/log_ship`) to a peer node in `[[network.nodes]]` (DH7, below). Left out: off |
| (edit `host.toml`) | `[switches]` `reforge`, `mastery_persistence`, `bounty_persistence`, `cooldown_persistence`, `account_erasure`, `log_purge`, `rate_limit`, `rate_limit_per_minute`, `gameserver_rate_limit_per_minute`, `host_rate_limit_per_minute` | Backend persistence, rule and request-budget switches written to `backend.env` (`MYSTICPARADOX_REFORGE`, `MYSTICPARADOX_MASTERY_PERSISTENCE`, `MYSTICPARADOX_BOUNTY_PERSISTENCE`, `MYSTICPARADOX_COOLDOWN_PERSISTENCE`, `MYSTICPARADOX_ACCOUNT_ERASURE`, `MYSTICPARADOX_LOG_PURGE`, `MYSTICPARADOX_RATE_LIMIT`, `MYSTICPARADOX_RATE_LIMIT_PER_MINUTE`, `MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE`, `MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE`) and forwarded to `isles-1-14-7`'s `[switches]`. Left out: the module's defaults |
| (edit `host.toml`) | `[modules]` `compiled`, `[[modules.external]]` `id`, `path`, `sha256`, `args`, `autoload`, `[modules.<id>.<section>]` | The game modules and plugins to load (below). Left out: every compiled-in one |
| (edit `host.toml`) | `[network]` `node`, `region`, `leader` | This server's name, region and optional directory leader in a network. Left out: a server on its own |
| (edit `host.toml`) | `[[network.nodes]]` `id`, `region`, `host`, `port`, `fingerprint` | The other nodes of the network: the only ones this node answers, and the ones it calls (below). Left out: no peers, and the server works as one outside a network |

### Secrets (`secrets.env`)

`setup` creates `.workspace/config/secrets.env` once (`SECRET_KEYS` in `rust/services/host/src/hostconfig.rs`) and never rotates an existing secret when `setup` runs again:

| Variable | Copied to | What |
| --- | --- | --- |
| `GATEWAY_SECRET` | `gateway.env` | Authenticates the gateway to its upstreams |
| `ALLOWLIST_SECRET` | `gateway.env`, `allowlist.env` | Authenticates the gateway to the allowlist helper |
| `REFORGED_ADMIN_KEY` | `backend.env`, `admin.env` | The backend's `/reforged/api/*` admin key |
| `GAMESERVER_API_KEY` | `backend.env` (`GAMESERVER_API_KEYS`), `director.env` (`METAGAME_API_KEY`) | Shared key between the game servers and the backend |
| `API_KEY_HASH_SECRET` | `backend.env` | HMAC secret for hashing account API keys at rest |
| `MYSTICPARADOX_METRICS_TOKEN` | `backend.env` | Bearer token for the backend's `/metrics` endpoint |
| `AUTH_SIGNING_PRIVKEY_B64`, `AUTH_SIGNING_PUBKEY_B64` | `backend.env` | 4096-bit RSA keypair (PKCS#8 private key and SPKI public key, base64-encoded PEM) that signs and verifies the auth service's RS256 access tokens |
| `REFORGED_ADMIN_API_KEY` | `admin.env` | The owner's bearer key for the admin GraphQL API ([admin-api.md](admin-api.md)) |
| `MYSTICPARADOX_MANAGER_OPERATOR_SECRET` | `backend.env`, `admin.env` | HMAC secret signing the admin API's `/admin/v1` requests to the backend |
| `REFORGED_DB_PASSWORD` | `admin.env` (when Postgres is enabled) | Password for the Postgres role when `[database]` uses `engine = "postgres"` |

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
network you trust.

Every Rust service and module repository (`isles-1-14-7`, `admin`, `network`, `chat`) runs on `store`'s `MultiConnection` over SQLite or Postgres with identical behaviour and portable
migrations. When a host switches from SQLite to Postgres (or back), `store db move` copies every table and the
hash-chained change log (`change_log`) across engines and verifies that row counts and contents match.

### Off-server history bucket and restore

In addition to peer log shipping (`[history] ship_to_peer`, below), a server can stream its change-log segments and
periodic database snapshots to an S3-compatible bucket with object lock (`history-ship`, DD23 layer 2: AWS S3,
Backblaze B2, MinIO, Cloudflare R2):

- **Write-only server credentials.** Create the bucket with object lock enabled (`S3Bucket::create_locked` in
  `rust/crates/history-ship`, retention mode `COMPLIANCE` or `GOVERNANCE`) using the owner's credentials off the
  server, and give the server a write-only key (`s3:PutObject` only) in `secrets.env`. Even an attacker or rogue
  operator with root on the server cannot delete or overwrite already-shipped segments or snapshots.
- **Restoring to a point in time.** `history_ship::restore_to` downloads the latest snapshot at or before the
  chosen `--to <RFC 3339>` timestamp, replays every shipped change-log segment up to that moment into a target
  SQLite or Postgres database, verifies every entry's SHA-256 hash chain (`history verify`) and any out-of-band
  `RestoreOptions::trusted_head`, and refuses unverified tail entries outside `trust_window`
  ([security/history.md](security/history.md)).
- **Shipping lag alerts.** Both the server status page and the admin API surface a warning whenever log shipping
  falls behind the local change log head.

### Realms and the TURN relay

On Dauntless 1.14.7 (`isles-1-14-7` with `modules/rustic-rifts`), players can host hubs and hunts on their own PCs
(**realms**, DD14) and let friends join directly over ICE (`ParadoxTransport.dll` on `libjuice`) or through a TURN
relay when NAT traversal needs one:

- **Admission keys.** Generate an ES256 admission key pair (`rifts-admission` / `generate-admission-key`) so the
  backend can sign short-lived `MPT2` admission tickets and per-pair AES-256-GCM transport keys.
- **The TURN relay (`turn-rs`, DD24).** The host kit configures, starts and stops `turn-rs` as its own child process
  (`modules/rustic-rifts/crates/rifts-transport/src/turn.rs`) with `static-auth-secret` set to `P2P_TURN_REST_SECRET`
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

`reforged-host admin guarded list`, `approve <id>` and `cancel <id>` decide as the host, and **DaintyLass Manager** (`dainty-lass-manager`)
lists what waits. A rule on `"approval"` on a server with one owner key waits for nobody and expires after
`expire_hours`: use it once the team has a second key. How it works, and what a bot sees, is in
[admin-api.md](admin-api.md#guarded-actions-dh6).

### Game modules and plugins

A game module serves one exact game version; a plugin extends the module of its version. A module compiled into
`reforged-host` is loaded unless `compiled` leaves it out; any other is an
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

[modules.rustic-freeza.tuning]  # a module's own settings: the ones it says it reads
```

For `isles-1-14-7` (`modules/rustic-isles`), the `[modules.<id>.switches]` section (or the top-level `[switches]` table, which `setup` also writes to `backend.env`) overrides the module's switch defaults (`crates/isles-core/src/settings.rs`):

| `[switches]` / `[modules.<id>.switches]` key | `backend.env` variable | Values | Default |
| --- | --- | --- | --- |
| `qos_port` (`[modules.<id>.switches]` only) | `QOS_PORT` | UDP port (`1`–`65535`) or `"0"`/`"off"` | `80` (`0` in `backend.env` when the launcher's relay answers QoS) |
| `reforge` | `MYSTICPARADOX_REFORGE` | `"on"`, `"log"`, `"off"` | `"on"` |
| `mastery_persistence` | `MYSTICPARADOX_MASTERY_PERSISTENCE` | `"on"`, `"off"` | `"on"` |
| `bounty_persistence` | `MYSTICPARADOX_BOUNTY_PERSISTENCE` | `"on"`, `"off"` | `"on"` |
| `cooldown_persistence` | `MYSTICPARADOX_COOLDOWN_PERSISTENCE` | `"on"`, `"off"` | `"on"` |
| `account_erasure` | `MYSTICPARADOX_ACCOUNT_ERASURE` | `"on"`, `"off"` | `"on"` (background sweep erases accounts past their deletion grace period) |
| `log_purge` | `MYSTICPARADOX_LOG_PURGE` | `"on"`, `"off"` | `"on"` (background sweep purges expired email verification codes and activity logs) |
| `rate_limit` | `MYSTICPARADOX_RATE_LIMIT` | `"on"`, `"off"` | `"on"` (1.14.7 app-wide per-minute HTTP request budget; `429` with `draft-8` `RateLimit` and `RateLimit-Policy` headers) |
| `rate_limit_per_minute` | `MYSTICPARADOX_RATE_LIMIT_PER_MINUTE` | positive integer | `600` (client requests per minute per address or `/56` IPv6 subnet on 1.14.7) |
| `gameserver_rate_limit_per_minute` | `MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE` | positive integer | `6000` (dedicated game server requests per minute per address on 1.14.7) |
| `host_rate_limit_per_minute` | `MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE` | positive integer | `6000` (player-hosted hunt requests per minute per validated capability on 1.14.7) |

Two game modules for one version are refused, whichever loads first. A port a module asks for that something already holds is moved up past it. In the
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
code is 1 when a peer is not up). When an existing standalone server joins a network (DN13),
`reforged-host network join` registers its existing accounts with this server as their `home` while keeping existing
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

One **DaintyLass Manager** (`dainty-lass-manager`) install or headless controller can manage several enrolled server
installs over pinned mTLS ([remote-management.md](remote-management.md), [security/remote-management.md](security/remote-management.md)).
The player launcher (`dainty-lass-client`) has no remote-management surface.

1. **Initialize the controller.** In **DaintyLass Manager** -> **Host** -> **Managed installs**, click **Enable
   Controller Mode** (or run `bin\reforged-host remote init-controller --node controller --region oregon --url https://203.0.113.10:443`).
   This creates `.workspace\tls\remote-cluster-ca.pem` (kept on the controller only) and writes `[remote] role = "controller"` in `host.toml`.
2. **Mint a single-use enrollment invite.** In the Manager's **Managed installs** panel, enter the target node ID,
   region and optional label and click **Create Enrollment Invite** (or run `bin\reforged-host remote invite --node us-east-1 --region us-east --label "US East 1"`).
   The URI (`dainty-lass-remote://enroll?v=1&url=...&fp=<sha256>&node=...&region=...&token=...`) is single-use and
   expires in 15 minutes (`--ttl-secs`).
3. **Enroll the managed install.** On the target machine, paste the URI in **DaintyLass Manager** -> **Managed installs**
   or run `bin\reforged-host remote enroll "dainty-lass-remote://enroll?..."`. The install connects over TLS pinned to
   `fp`, generates its own ECDSA P-256 private key locally (`.workspace\tls\node-key.pem` never leaves the machine),
   receives a signed leaf certificate from the controller on `POST /__node/enroll`, and syncs over mTLS on
   `POST /__node/remote`.
4. **Shared settings, local overrides and remote actions.** Changing shared settings on the controller
   (`bin\reforged-host remote set-config --set name="Friday Hunts" --set max_hunt_servers=4`) bumps the controller's
   config revision; enrolled installs merge the shared revision while keeping their local `host`, `game_address`,
   `ports`, `paths`, `database`, `tls`, `secrets.env` and any dotted keys listed in `[remote].local_overrides`. Remote
   admin actions (`bin\reforged-host remote exec --node us-east-1 --action ban --target PlayerName --reason "..."`, or
   the **Doctor** / **Backup** / **Revoke** buttons in the Manager) preserve `DH2` role checks, `DH6` guarded approval
   and `DD23` change-log attribution (`remote:<controller>:<operator>`). If the controller goes offline, managed
   installs keep serving with their last applied configuration.
5. **Revoke or leave.** `bin\reforged-host remote revoke <node>` on the controller immediately refuses that install on
   `/__node/remote` (`403 not_a_peer`); `bin\reforged-host remote leave` on a managed install detaches it back to
   `role = "standalone"`.

| `[remote]` key in `host.toml` | `gateway.env` / `host` variable | Values | Default |
| --- | --- | --- | --- |
| `role` | `REFORGED_REMOTE_ROLE` | `"standalone"`, `"controller"`, `"managed"` | `"standalone"` |
| `controller_url` | `REFORGED_REMOTE_CONTROLLER_URL` | `https://<host>[:<port>]` | unset |
| `controller_cert_fp` | `REFORGED_REMOTE_CONTROLLER_CERT_FP` | 64 lowercase hex digits (SHA-256 of controller gateway cert) | unset |
| `controller_node` | `REFORGED_REMOTE_CONTROLLER_NODE` | `[a-z0-9-]{1,32}` | unset |
| `node` | `REFORGED_REMOTE_NODE` | `[a-z0-9-]{1,32}` | `[network].node` |
| `region` | `REFORGED_REMOTE_REGION` | `[a-z0-9-]{1,32}` | `[network].region` |
| `label` | `REFORGED_REMOTE_LABEL` | display string (`1`–`64` chars) | unset |
| `sync_interval_secs` | `REFORGED_REMOTE_SYNC_INTERVAL_SECS` | integer `5`–`3600` | `15` |
| `local_overrides` | `REFORGED_REMOTE_LOCAL_OVERRIDES` | array of dotted `host.toml` paths preserved locally | `[]` |

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

`update` keeps `.workspace` and runs `setup` from your `host.toml`. On a server that ran the Node backend and has
existing MongoDB data, both `update` and the new release's own `setup` migrate and verify the MongoDB data into the
configured store (`.workspace\config\dainty-lass.db` on SQLite, or Postgres when `[database]` sets
`engine = "postgres"`), writing `.workspace\config\.mongo-migrated` once verified: if `verify` reports any difference,
`setup` and `update` refuse to switch and leave the server on its previous setup, and they never delete or modify the
MongoDB data. `start` and `serve` refuse to run the Rust backend while unmigrated MongoDB data exists (unless the host
explicitly opts into an empty database with `reforged-host setup --fresh-start` or `fresh_start = true` under
`[database]` in `host.toml`; removing `fresh_start` later and running `setup` clears the opt-out marker and runs the
migration). When `native\` changed, `update` says so: rebuild the runtime and publish it again.
Players' launchers notice the new hashes and ask once more.

### Live server cutover checklist (backup, update, smoke, rollback)

When cutting a live 1.14.7 server over from the Node backend to the Rust backend:

1. **Backup.** Stop the server (or take a snapshot first), run `bin\reforged-host backup` so `.workspace\backups\`
   holds a fresh database dump, and copy `.workspace\config` (`host.toml`, `secrets.env`) and `.workspace\tls`
   (`gateway-cert.pem`, `gateway-key.pem`) to safe off-server storage.
2. **Update and migrate.** Ensure `mongod` is running if the server has existing MongoDB data so `update` can read it,
   then run `bin\reforged-host update`. Confirm the output reports `MongoDB migration verified (... 0 differences;
   MongoDB data kept intact)` and finishes `setup`.
3. **Smoke check.** Run `bin\reforged-host doctor`, start the server with `bin\reforged-host start`, verify that
   `/dauntless-status`, `/reforged/api/Status` and `/content/v1/runtime` answer on the gateway, and verify with a
   1.14.7 client and launcher that an existing account can sign in, enter Ramsgate with its characters, inventory and
   loadouts intact, and depart for a hunt.
4. **Rollback (if needed).** Stop the server, export the SQLite or Postgres store back to MongoDB with
   `bin\reforged-host db export-mongo --to mongodb://127.0.0.1:27017/reforged` (source and target default from
   `host.toml`), verify with `bin\reforged-host db verify --against mongodb://127.0.0.1:27017/reforged`, and start the
   `paradox` build (below).

## Rollback to the Node backend and MongoDB (`paradox` feature)

For one release after the RU9/RB14 cutover (until RB15 deletes the reference Node services), building the server with
the opt-in `paradox` Cargo feature (`cargo build --release -p server-console --features paradox`) runs Mystic
Paradox's Node `ParadoxBackend` and `ParadoxDirector` from `modules/rustic-isles/reference/1.14.7/` against MongoDB
instead of the compiled-in `isles-1-14-7` Rust module. To export a server's SQLite or Postgres store back to MongoDB
before starting a `paradox` build, run `reforged-host db export-mongo --to <mongodb-uri>` (and optionally
`reforged-host db verify --against <mongodb-uri>`; `--from` and `--to` default from `host.toml`).
