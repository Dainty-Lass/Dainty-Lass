# Hosting in Docker

The whole server in containers, with Docker Compose: the same services `dainty-host start` runs, one
container each, from two images built from this repository. A public server on a Linux VPS is the main use;
a private server over Tailscale works too.

> [!WARNING]
> **Experimental**, as the [Linux server](hosting-linux.md) is: the game servers run through Wine, and
> **the real Dauntless 1.14.7 has not yet been run under Wine**. Report what you find.

## What runs

| Container | Image | What it is |
| --- | --- | --- |
| `net` | front | Holds the network namespace every other container joins, and the published ports |
| `backend` | server | The 1.14.7 Rust backend (`dainty-host backend serve`), 127.0.0.1:61000 |
| `director` | server | The 1.14.7 Rust Director (`dainty-host director serve`), 127.0.0.1:61001; starts Ramsgate, the Training Grounds and hunts through Wine |
| `content` | front | The content server, 127.0.0.1:61002 (`dainty-host content serve`) |
| `allowlist` | front | Public servers: opens the game ports to players who logged in (nftables; `dainty-host allowlist`) |
| `gateway` | front | The front door: TLS on 443 (public) or HTTP on the Tailscale address (private; `dainty-host gateway serve`) |
| `host` | server | Not a service: `docker compose run --rm host <command>` runs `dainty-host` |
| `configure` | server | The `toml` profile only: runs setup from `host.toml` and exits, before the others start |
| `postgres` | `postgres:17` | The `postgres` profile only: Postgres on 127.0.0.1:5432 for a `host.toml` whose `[database]` picks it |
| `mongo`, `backend-rollback`, `director-rollback` | `mongo:8.0`, server-rollback | The `rollback` profile only: MongoDB (`rs0`) and the Node 1.14.7 backend and Director for one release after RU9/RB14 ([below](#migrating-existing-mongodb-data-and-rollback-rollback-profile)) |
| `net-*`, `netem-*`, `gateway-*`, `configure-*`, `network-init`, `network` | front, server, netem | The `network` profile only: four nodes of a network with injected latency, for tests ([below](#the-network-profile-four-nodes-on-one-machine)) |

**One network namespace.** Every container joins `net`'s, so they reach each other on 127.0.0.1 exactly as
the env files `dainty-host setup` writes say, and nothing but the gateway and the game ports is reachable
from outside, not even from other containers. The allowlist's nftables table lives in that namespace too: it
filters the game servers' traffic without any privilege on the host and without touching the host's firewall.
Only the allowlist has `CAP_NET_ADMIN`, and only through `nft`'s file capability.

**The same files as a Linux host.** Both images are laid out as the server release is, under `/opt/reforged`,
and every container mounts the volumes at the same paths, so `host.toml`, `secrets.env` and the env files are
the host kit's, unchanged, and a `.workspace` can move between a container host and a plain one.

| Volume | Path | Holds |
| --- | --- | --- |
| `workspace` | `/opt/reforged/.workspace` | Configuration and secrets, the certificate, the published runtime, branding, logs, the Wine prefix |
| `backend-game-data`, `director-game-data` | `/opt/reforged/modules/rustic-isles/reference/1.14.7/*/game-data` | Your game data |
| `exports`, `captures` | `/opt/reforged/modules/rustic-isles/reference/1.14.7/Items_Analysis`, `…/DauntlessEndpointDocumentation` | CatalogExporter exports and store captures, for `gamedata` |
| `mongo-data`, `mongo-config` | `/data/db` in `mongo`, `/opt/reforged/.workspace/mongodb` in `server` | Pre-cutover MongoDB data (moved into `store` by `host db migrate` and `host db verify`, and used by the `rollback` profile) |
| `postgres-data` | Postgres's | The `postgres` profile's database |
| bind mount `REFORGED_GAME_DIR` | `/game` | Your Dauntless 1.14.7 folder (the one with `Archon`), or the folder `install-game` makes `1.14.7` in |
| bind mount `REFORGED_IMPORT_DIR` | `/import` (read-only, `host` only) | Files to bring in: the runtime's DLLs, game data packs |
| bind mount `REFORGED_CONFIG_DIR` | `/config` (read-only, `configure` only) | The `toml` profile's `host.toml` |

## Before you start

- **Docker Engine with Compose 2.24 or newer** on Linux. A public server needs Docker Engine on the Linux
  machine itself: Docker Desktop (Windows, macOS) hides the players' addresses behind its own, so the
  gateway and the allowlist see one address for everyone. Docker Desktop is fine for a private server or a
  try-out.
- The kernel's nftables (`nf_tables`, loaded on any distribution whose Docker uses iptables-nft, which is the
  default on Debian 12 and 13 and Ubuntu 22.04 and newer).
- A Linux kernel older than 6.19 or at least 7.0.14: the `mongo:8.0` image refuses to start in between (its
  memory allocator and those kernels' `rseq` disagree, mongodb.org SERVER-121912), which `docker compose logs
  mongo` shows as "MongoDB cannot start". Ubuntu 26.04 ships 6.19; Debian 13 and Ubuntu 24.04 are fine.
- Your Dauntless 1.14.7 folder on the server, as for [a Linux host](hosting-linux.md) (about 15 GB), owned by
  or writable for uid 1000: the game servers write their logs and a switch file next to the executable. Or an
  empty folder of that kind and `install-game` ([below](#getting-the-game-from-epic)), which fetches it from your
  own Epic library.
- The game runtime and game data, from a Windows PC with your own game ([sdk-and-game-data.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/sdk-and-game-data.md)).
- Public: the provider's firewall open for TCP 443 and UDP 8770-8777, as in [hosting-linux.md](hosting-linux.md).

## A public server

```bash
git clone ssh://git@git.silentspud.com:222/silentspud/dainty-lass.git
cd dainty-lass/restricted
cp .env.example .env
```

In `.env`, set `REFORGED_GAME_DIR` to your Dauntless folder and leave `COMPOSE_PROFILES=public`. Then:

```bash
REFORGED_COMMIT=$(git rev-parse --short=12 HEAD) docker compose build
docker compose run --rm host setup --mode public --host play.example.org --name "Friday Hunts"
```

Every crate is in the checkout: the images build with the monorepo root as context (Compose's `context: ..`), so
`restricted/` reaches the root's MIT crates by path and the build needs no SSH key. The server binary is built with
`all-modules`, as the release is; to build without any game module, set the build argument
`REFORGED_MODULES=` (from the root: `docker build -f restricted/docker/Dockerfile --build-arg REFORGED_MODULES= --target server .`).
The server image also holds the libraries `host.toml` can pin (RD27) in `/opt/reforged/plugins`, so a
`[[modules.external]]` or `[services.<name>]` path there reads `plugins/libisles_1_14_7_plugin.so` and the like
([hosting.md](hosting.md#game-modules-and-plugins)).

`setup` takes every option it takes on a Linux host (`--max-hunts`, `--public-port`, `--registration` …;
`docker compose run --rm host setup --help`). In the container it also gets `--skip-install` (the services are
in the image) and `--game /game` (when `/game/Archon` exists), unless you give those yourself. Ignore what it
says about systemd and ufw: compose runs the server. On a DNS name, give `--game-address <public IPv4>` as on
a Linux host.

### Getting the game from Epic

With no Dauntless folder yet, point `REFORGED_GAME_DIR` at an empty one (writable for uid 1000) and install into it
from the owner's Epic library, as the launcher does for players:

```bash
docker compose run --rm host install-game
```

In the container `--dir` is `/game` unless you give it, so 1.14.7 lands in `REFORGED_GAME_DIR/1.14.7` (`/game/1.14.7`).
The command prints Epic's login address; open it in a browser on any computer and paste the `authorizationCode`
Epic shows when asked (`run` gives it a terminal; with `-T` or a pipe, put the code in `./import` and pass
`--code-file /import/code.txt`). The session it opens is kept in the `workspace` volume
(`config/epic-session.env`, owner-only): a stopped download continues when you run the command again, and
`docker compose run --rm host install-game --sign-out` ends and deletes the session, which should be the last thing
you run. If `host.toml` exists, `install-game` sets `game_dir` to `/game/1.14.7` and rewrites the env files, so the
`director` container's game servers run from it; if not, give `setup` that folder with `--game /game/1.14.7` (setup
adds `--game /game` itself only when `/game/Archon` exists). A `toml`-profile `host.toml` in `REFORGED_CONFIG_DIR`
replaces the workspace's, so put `game_dir = "/game/1.14.7"` in it. The `compose.yaml` needs nothing for this: the
`host` service already mounts the game folder and reaches the internet through `net`. About 15 GB comes down;
[hosting.md](hosting.md#getting-the-game-from-epic) has the rest.

The runtime and the game data: put the files in `./import` (or `REFORGED_IMPORT_DIR`), then

```bash
docker compose run --rm host publish-runtime --from /import/runtime
docker compose run --rm host gamedata --import /import/gamedata.zip
```

Start it, and make an invite:

```bash
docker compose up -d
docker compose ps                       # every service healthy
docker compose run --rm host invite --for "Alex"
```

`docker compose logs -f` shows every service (`docker compose logs -f director` one); the game servers' own
logs are in the `workspace` volume, `logs/gameservers/`. `docker compose run --rm host doctor` checks the
setup from inside the server's network. It checks for a plain Linux host, so in Docker a few of its lines
are expected and harmless: git, nftables and the systemd units are not in the `host` container, and the
gateway and content server count as "not built" because they run from `dainty-host` in the `front` image instead.

**Another port than 443** (`setup --public-port 4443`): set `REFORGED_GATEWAY_PORT=4443` and
`REFORGED_GATEWAY_PUBLISH=4443` in `.env` too, and allow TCP 4443 in the provider's firewall. **More than six
hunts** (`--max-hunts 10`): `REFORGED_GAME_PORTS=8766-8777`.

## A private server (Tailscale)

The `net` container becomes a Tailscale node of its own ([compose.tailscale.yaml](../compose.tailscale.yaml)):
the server's Tailscale address is inside the shared namespace, the gateway and the game servers listen on it,
and nothing is published on the host. Make a reusable auth key in the Tailscale admin console, then in `.env`:

```bash
COMPOSE_PROFILES=
COMPOSE_FILE=compose.yaml:compose.tailscale.yaml     # Windows: compose.yaml;compose.tailscale.yaml
TS_AUTHKEY=tskey-auth-...
```

```bash
docker compose up -d net
docker compose exec net tailscale ip -4                 # the server's Tailscale address
docker compose run --rm host setup --mode private --host 100.x.y.z --name "Friday Hunts"
docker compose up -d
```

The node is a machine of its own in your tailnet (`reforged-revival`, or `TS_HOSTNAME`): share that one with
your friends, and give its share link to setup with `--share`.

## Managed by host.toml alone

For a server nobody sets up by hand, such as one kept in a repository or deployed by a script: with the
`toml` profile, `host.toml` in `./config` (or `REFORGED_CONFIG_DIR`) is the server's whole configuration. At
every `docker compose up` the `configure` container copies it into the `workspace` volume and runs setup from
it with no options, and nothing else starts unless that worked. A setting is changed by editing the file and
running `docker compose up -d --force-recreate`. That includes `[guard]`, which holds the bulk admin actions
([hosting.md](hosting.md#guarded-admin-actions)), and `[switches]`, which configures backend persistence and the 1.14.7
per-minute HTTP request budget (`rate_limit` -> `RUSTIC_ISLES_RATE_LIMIT`, `rate_limit_per_minute` ->
`RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE`, `gameserver_rate_limit_per_minute` ->
`RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE`, `host_rate_limit_per_minute` ->
`RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE`; see [hosting.md](hosting.md#game-modules-and-plugins)); `docker compose run --rm host admin guarded list --env-file
/opt/reforged/.workspace/config/admin.env` lists what waits.

In `.env`, `COMPOSE_PROFILES=public,toml` (or `toml` for a private server). The file needs every setting a
first setup asks for, so the easiest way to get one is to let setup write it once and copy it out:

```bash
docker compose run --rm host setup --mode public --host play.example.org --name "Friday Hunts"
mkdir -p config
docker compose run --rm --entrypoint cat host /opt/reforged/.workspace/config/host.toml > config/host.toml
docker compose up -d
```

The file setup writes says what each setting is. As in any container, `game_dir` is `/game` when `/game/Archon`
exists, and the database is SQLite in `.workspace/config/dainty-lass.db` unless `[database]` picks `postgres`
(below). Secrets are not in it: setup makes `secrets.env` once in the `workspace` volume and keeps it. The
runtime, the game data and invites are still `host` commands.

## Postgres

The Rust services keep their database in SQLite unless `host.toml` says otherwise
([hosting.md](hosting.md#the-database-engine)). For Postgres, add the `postgres` profile to `COMPOSE_PROFILES`
(`public,postgres`) and put this in `host.toml`, then run setup (or `up`, with the `toml` profile):

```toml
[database]
engine = "postgres"
```

Leave `postgres_host`, `postgres_port`, `postgres_user` and `postgres_database` out: the container is
127.0.0.1:5432 with the role and database `dainty`. Its first start takes the role's password from `secrets.env`
(`REFORGED_DB_PASSWORD`), so setup has to have run before; the container stops with a message otherwise.
`POSTGRES_IMAGE` in `.env` picks another image.

## A network of servers

A node of a network ([hosting.md](hosting.md#a-network-of-servers)) is an ordinary server: `[network]` and
`[[network.nodes]]` in `host.toml`, and the three files `network issue-node-cert` wrote in the `workspace` volume's
`tls` folder. The CA file stays off the server: `network ca-init` and `issue-node-cert` run on your own machine.
Node links share the gateway's published TCP port (443); nothing else is published for them.

### The `network` profile: four nodes on one machine

For testing, not for running a network: `docker compose --profile network up -d --wait network` starts four nodes of
one network (DD16) on a private Docker network, with latency between them that matches the canary's regions
(DN12). Each node is a gateway with node links on, set up from a `host.toml` that lists the other three as
`[[network.nodes]]`, from the same images as a server and with volumes of its own. Nothing else of a server runs in
them (no backend, MongoDB or game): the gateway answers the node links and the launcher's region ping, which is
what the harness checks. The other services have no profile, so name the target `network`: a bare
`--profile network up` would start the single server too.

| Container | What it is |
| --- | --- |
| `network-init` | One-shot, as root: makes the cluster CA in the `network-ca` volume, which no other container mounts, and writes into each node's volume its public gateway certificate, its node certificate and key, the CA's certificate (never its key) and its `host.toml` ([security/network.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/security/network.md#the-docker-harness-dn12)) |
| `configure-<region>` | One-shot: `setup` from that `host.toml` (the `toml` profile's `apply`), then exits |
| `net-<region>` | The node's network namespace and published port, at a fixed address in the private network |
| `netem-<region>` | `tc` with CAP_NET_ADMIN in that namespace, the only container with it: shapes the node's outgoing traffic (below) |
| `gateway-<region>` | The node's gateway (`dainty-host gateway serve`), healthy once it listens |
| `network` | Nothing: it waits for the four gateways, so `up --wait network` is the whole cluster |

The regions are `oregon`, `new-york`, `uk` and `germany` (nodes `oregon-1` and so on), at `172.28.77.11` to `.14`
(`REFORGED_NETWORK_SUBNET`), their gateways published on `127.0.0.1` ports 18441 to 18444. The addresses are fixed
because the node certificates and the `tc` filters name them.

**The latency** is the canary's round trip between each pair of regions (milliseconds; each node delays its outgoing
traffic to a peer by half, so a round trip adds up to the table's value):

| | new-york | uk | germany |
| --- | ---: | ---: | ---: |
| **oregon** | 70 | 140 | 150 |
| **new-york** | | 75 | 85 |
| **uk** | | | 20 |

Each pair is a setting, `REFORGED_NETWORK_RTT_<A>_<B>` (`REFORGED_NETWORK_RTT_OREGON_NEW_YORK=70` and so on, listed
in `.env.example`). The Docker host plays a player: a node delays everything that is not another node by the round
trip between the player's region (`REFORGED_NETWORK_PLAYER_REGION`, `germany` unless set) and its own, which is what a
launcher running on the host measures. `docker/network/netem.sh` applies it with a `prio` qdisc, one `netem` band
per peer and filters by address; ARP is never delayed.

**The test**, `bash docker/network/test.sh` from the repository root (about two minutes once the images are built):

1. Builds the images and checks that this Docker can add a `netem` qdisc in a container.
2. Starts the profile, waits for the four gateways, and runs `dainty-host network status` inside each node: it
   pings each peer over the node link (mutual TLS under the cluster CA) and prints its state, round trip and clock
   skew. The test needs three peers `up` and each round trip from the injected one to 25 ms above it
   (`NETWORK_TOLERANCE_MS`).
3. Runs the launcher's real `region_ping` (`cargo test -p launcher`, the ignored test
   `the_region_order_follows_the_latency_the_harness_injects`) with each node as the home: the regions must come back
   in the order of the injected latencies from the player's region, each round trip within the same bounds, and
   the home must take the report. This step needs `cargo` and the nodes' private addresses reachable from the
   host, which a Linux host has and **Docker Desktop does not**; there it says so and skips
   (`NETWORK_REGION_PING=require` fails instead, as CI sets it; `skip` leaves it out).
4. Removes the containers, the network and the volumes.

`NETWORK_OPTIONAL=1` makes it exit 0 with a note when Docker, CAP_NET_ADMIN or the `sch_netem` kernel module is
missing; `docker/run-tests.sh` runs it that way as its last step (`SKIP_NETWORK=1` leaves it out). CI's `network`
job runs it on the Linux runner's own Docker, for changes to the network plugin, the gateway, the host kit, `docker/`
and the compose files.

To look around, start it and run what the test runs:

```bash
docker compose --profile network up -d --wait network
docker compose --profile network exec gateway-uk dainty-host network status \
  --env-file /opt/reforged/.workspace/config/gateway.env
curl -k https://127.0.0.1:18441/network/v1/nodes       # what the launcher is told about the other regions
docker compose --profile network exec netem-oregon tc qdisc show dev eth0
docker compose --profile network down --volumes
```

`network status` is the host kit's own look at the links (`--json` for scripts, `--count` for more pings per
peer); its exit code is 1 when a peer is not `up`.

## Managing several installs

A Docker server can enroll under a **DaintyLass Manager** controller (or a headless controller container) over pinned
mTLS ([hosting.md](hosting.md#managing-several-installs), [remote-management.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/remote-management.md)):

- **First-boot enrollment (`REFORGED_REMOTE_ENROLL_TOKEN`).** Set `REFORGED_REMOTE_ENROLL_TOKEN="dainty-lass-remote://enroll?v=1&..."`
  (and optional `REFORGED_REMOTE_LABEL`) in `.env` and add the `managed` profile (`COMPOSE_PROFILES=public,managed`).
  When `remote-agent` starts (`docker/host-entrypoint.sh`), it redeems the single-use enrollment URI once if
  `remote-managed.json` does not exist yet, writes the issued mTLS leaf certificate into `/opt/reforged/.workspace/tls/`,
  and syncs shared settings and queued remote commands from the controller every `REFORGED_REMOTE_SYNC_INTERVAL_SECS`
  (default `15`).
- **Manual CLI enrollment.** On an already running container, run
  `docker compose run --rm host remote enroll "dainty-lass-remote://enroll?v=1&..."` and
  `docker compose run --rm host remote sync`.
- **The `remote` test profile (`docker/remote/test.sh`).** `bash docker/remote/test.sh` (also run by
  `docker/run-tests.sh` unless `SKIP_REMOTE=1`) starts one controller (`gateway-controller`, `host-controller`) and two
  managed installs (`alpha`, `bravo`) on `172.28.77.20`–`.22`, proves single-use enrollment, shared config sync,
  remote ban execution with `remote:<controller>:<operator>` attribution, offline resilience when `gateway-controller`
  stops, revocation refusal (`403 not_a_peer`), and wire-payload inspection confirming zero secrets, account keys or
  database files cross the wire.

## Podman

Not tested yet; report what you find. The compose file is plain Compose, so `podman compose` runs it with
`docker-compose` 2.24 or newer as its provider, talking to Podman's socket
(`systemctl --user enable --now podman.socket`). podman-compose is not supported: the shared network
namespace and the optional dependencies depend on Compose's own behaviour. Rootless, also:

- A port below 1024 needs `sysctl net.ipv4.ip_unprivileged_port_start=443` on the host. The `net`
  container's own sysctl only covers the inside.
- The players' own addresses must reach the gateway, as with Docker Desktop: Podman 5's default `pasta`
  network should keep them. Check that the gateway's log names players' addresses, not one address for all.
- The game folder belongs to uid 1000 inside the container: `podman unshare chown -R 1000:1000 <game folder>`.
- On an SELinux host (Fedora, RHEL), a bind mount needs the `z` option to be readable. The `toml` profile's
  `/config` has it; add `:z` to the game folder's mount in a `compose.override.yaml`.

## Day to day

| What | How |
| --- | --- |
| Stop, start | `docker compose stop`, `docker compose up -d`; a clean stop includes the game servers |
| Change a setting | `docker compose run --rm host setup …` (or `multipliers`), then `docker compose up -d --force-recreate`. With the `toml` profile: edit `config/host.toml`, then the same `up` |
| Update | `git pull`, `docker compose build`, `docker compose up -d`. On a server updating from the pre-RU9 Node backend with existing `mongo-data`, run the one-time migration below first (`backend` refuses to start while `mongo-data` is unmigrated). `dainty-host update` is not used in Docker |
| Back up | `docker compose run --rm host backup` (into the `workspace` volume, `backups/`), and keep a copy of the `workspace` volume: it holds the certificate every invite names and the secrets |
| A shell | `docker compose run --rm --entrypoint bash host` |

Never delete the `workspace` volume of a server that has players: a new certificate changes the fingerprint
in every invite, and new secrets log everyone out.

## Testing the images

`bash docker/run-tests.sh` (from the repository root) is the full test run CI's `docker` job does on the built
images:

1. It builds `front`, `server` and `test`. The `test` image is the Debian slim `server` image plus the Rust
   toolchain, with the server crates' tests compiled in, so the code is tested on the OS, Wine and paths
   production runs on (without Node or MongoDB).
2. Inside the `test` image: `cargo test` for the server crates (protocol, proxy, service-kit, store, gateway,
   content, host), with the allowlist's nftables test applying its rules for real in the container's own network
   namespace.
3. The compose smoke test (`docker/smoke-test.sh`): a pre-cutover `mongo-data` volume seeded with an account and
   invite, `backend` refusing to start before migration, `host db migrate` and `host db verify` moving the data into
   the SQLite store, the `toml` profile's `configure` applying an edited `host.toml`, every service healthy, the
   gateway serving setup's certificate and answering `/reforged/api/ServerStatus` and account checks from the
   Rust backend, the allowlist's table in place, an invite, a clean stop.

`bash docker/run-tests.sh rust` runs only step 2; `SKIP_SMOKE=1` leaves out step 3. On a server,
`docker compose run --rm test` runs step 2 with your game-data volumes mounted read-only.

A fourth step, `docker/network/test.sh`, starts the `network` profile and checks the links and the region ping
([above](#the-network-profile-four-nodes-on-one-machine)); it is skipped, not failed, where Docker cannot add a
`netem` qdisc, and `SKIP_NETWORK=1` leaves it out.

## Moving an existing Linux host into Docker

Stop the systemd units, then copy the host's `.workspace` into the `workspace` volume and its game data into
the game-data volumes (`docker compose run --rm -v /home/reforged/Reforged:/old:ro --entrypoint bash host`,
then `cp -a` from `/old`). If the host still has pre-RU9 MongoDB data, copy its `mongodb` folder into the
`mongo-data` volume (or run `dainty-host setup` on the host before moving `.workspace`). Run `setup` in
the container once, so the env files name the container's paths; if `mongo-data` was copied unmigrated, migrate it as
below before starting it.

## Migrating existing MongoDB data and rollback (`rollback` profile)

The `mongo-data` volume is mounted at `/opt/reforged/.workspace/mongodb` inside `backend`, `configure` and `host`.
While `mongo-data` is non-empty and `/opt/reforged/.workspace/.mongo-migrated` has not been written, `backend`
refuses to start so an updated server never comes up silently on an empty database.

To migrate an existing `mongo-data` volume from the pre-RU9 Node backend:

1. Start `mongo` from the `rollback` profile for the duration of the migration:
   `docker compose --profile rollback up -d --wait mongo`
2. Run `docker compose run --rm host setup` (or `COMPOSE_PROFILES=public,toml,rollback docker compose run --rm configure`
   when using the `toml` profile), then `docker compose run --rm host db migrate` and
   `docker compose run --rm host db verify`. `db migrate` moves every collection from `mongo` into the configured
   store (SQLite in `/opt/reforged/.workspace/config/dainty-lass.db` or the `postgres` profile); `db verify` checks
   every collection and writes `.mongo-migrated` only when it finds no difference, leaving `mongo-data` untouched.
   (To start on a fresh empty database without migrating, pass `--fresh-start` to `host setup` or set
   `fresh_start = true` under `[database]` in `host.toml`.)
3. Stop `mongo` and bring up the Rust services:
   `docker compose --profile rollback rm -sf mongo && docker compose up -d`

For one release after the RU9/RB14 cutover (until RB15), rolling back to the Node backend and MongoDB in Docker uses
the `rollback` Compose profile (`mongo`, `backend-rollback` and `director-rollback`, built from the `server-rollback`
Dockerfile stage): start `mongo` (`docker compose --profile rollback up -d --wait mongo`), export the store back to
MongoDB with `docker compose run --rm host db export-mongo --to mongodb://127.0.0.1:27017/reforged` (source and target
default from `host.toml`), and start `mongo`, `backend-rollback` and `director-rollback` in place of `backend` and
`director` ([hosting.md](hosting.md#rollback-to-the-node-backend-and-mongodb-paradox-feature)).
