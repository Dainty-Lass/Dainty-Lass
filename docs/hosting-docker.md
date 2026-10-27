# Hosting in Docker

The whole server in containers, with Docker Compose: the same services `reforged-host start` runs, one
container each, from two images built from this repository. A public server on a Linux VPS is the main use;
a private server over Tailscale works too.

> [!WARNING]
> **Experimental**, as the [Linux server](hosting-linux.md) is: the game servers run through Wine, and
> **the real Dauntless 1.14.7 has not yet been run under Wine**. The containers also run the **Rust**
> gateway, content server and allowlist helper (MIGRATION_PLAN.md RS2–RS4), which `reforged-host start` on a
> plain host only switches to at the front-door cutover. Report what you find.

## What runs

| Container | Image | What it is |
| --- | --- | --- |
| `net` | front | Holds the network namespace every other container joins, and the published ports |
| `mongo` | `mongo:8.0` | MongoDB with the one-member replica set `rs0` the backend needs (until RB14) |
| `backend` | server | The Node backend (`modules/rustic-isles/reference/1.14.7/ParadoxBackend`), 127.0.0.1:61000 |
| `director` | server | The Node Director, 127.0.0.1:61001; starts Ramsgate, the Training Grounds and hunts through Wine |
| `content` | front | The content server, 127.0.0.1:61002 (`reforged-host content serve`) |
| `allowlist` | front | Public servers: opens the game ports to players who logged in (nftables; `reforged-host allowlist`) |
| `gateway` | front | The front door: TLS on 443 (public) or HTTP on the Tailscale address (private; `reforged-host gateway serve`) |
| `host` | server | Not a service: `docker compose run --rm host <command>` runs `reforged-host` |
| `configure` | server | The `toml` profile only: runs setup from `host.toml` and exits, before the others start |
| `metagame` | server | A legacy server only: DR's metagame, 127.0.0.1:61000, in place of `backend` and `mongo` |
| `deploy` | server | A legacy server only: DR's deploy server, 127.0.0.1:61001; starts the 1.4.4 game servers through Wine |
| `postgres` | `postgres:17` | The `postgres` profile only: Postgres on 127.0.0.1:5432 for a `host.toml` whose `[database]` picks it |

**One network namespace.** Every container joins `net`'s, so they reach each other on 127.0.0.1 exactly as
the env files `reforged-host setup` writes say, and nothing but the gateway and the game ports is reachable
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
| `mongo-data`, `mongo-config` | MongoDB's | The database |
| `postgres-data` | Postgres's | The `postgres` profile's database |
| bind mount `REFORGED_GAME_DIR` | `/game` | Your Dauntless 1.14.7 folder (the one with `Archon`) |
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
  or writable for uid 1000: the game servers write their logs and a switch file next to the executable.
- The game runtime and game data, from a Windows PC with your own game ([sdk-and-game-data.md](sdk-and-game-data.md)).
- Public: the provider's firewall open for TCP 443 and UDP 8770-8777, as in [hosting-linux.md](hosting-linux.md).

## A public server

```bash
git clone --recurse-submodules ssh://git@git.silentspud.com:222/silentspud/dainty-lass.git
cd dainty-lass
cp .env.example .env
```

In `.env`, set `REFORGED_GAME_DIR` to your Dauntless folder and leave `COMPOSE_PROFILES=public`. Then:

```bash
REFORGED_COMMIT=$(git rev-parse --short=12 HEAD) docker compose build
docker compose run --rm host setup --mode public --host play.example.org --name "Friday Hunts"
```

`setup` takes every option it takes on a Linux host (`--max-hunts`, `--public-port`, `--registration` …;
`docker compose run --rm host setup --help`). In the container it also gets `--skip-install` (the services are
in the image), `--no-manage-mongo --mongo-uri mongodb://127.0.0.1:27017/?replicaSet=rs0` (the `mongo`
container) and `--game /game`, unless you give those yourself. Ignore what it says about systemd and ufw:
compose runs the server. On a DNS name, give `--game-address <public IPv4>` as on a Linux host.

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
gateway and content server count as "not built" because they run from `reforged-host` in the `front` image instead.

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
running `docker compose up -d --force-recreate`.

In `.env`, `COMPOSE_PROFILES=public,toml` (or `toml` for a private server). The file needs every setting a
first setup asks for, so the easiest way to get one is to let setup write it once and copy it out:

```bash
docker compose run --rm host setup --mode public --host play.example.org --name "Friday Hunts"
mkdir -p config
docker compose run --rm --entrypoint cat host /opt/reforged/.workspace/config/host.toml > config/host.toml
docker compose up -d
```

The file setup writes says what each setting is. As in any container, `game_dir` is always `/game` and
`[mongo]` the `mongo` container, whatever the file says. Secrets are not in it: setup makes `secrets.env` once
in the `workspace` volume and keeps it. The runtime, the game data and invites are still `host` commands.

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

## A legacy server (Dauntless 1.4.4)

Docker runs one game per server: `--edition both` ([hosting.md](hosting.md#both-games-on-one-server)) is the host
kit's for now, since Compose would need a second gateway, content server and allowlist container, a second
published TCP port and UDP range.

A Dauntless Revived 1.4.4 server ([hosting.md](hosting.md#legacy-servers-dauntless-144)) runs from the same
files. In `.env`:

```bash
REFORGED_EDITION=legacy
COMPOSE_FILE=compose.yaml:compose.legacy.yaml
COMPOSE_PROFILES=legacy,public
REFORGED_GAME_DIR=/srv/dauntless-1.4.4
```

`REFORGED_EDITION=legacy` builds the images with the `legacy` features and adds DR's metagame and deploy server,
built from the legacy-revived submodule (`modules/legacy-revived`), to the server image. The `legacy` profile adds the `metagame` and `deploy`
containers, in the shared namespace like the others, and `compose.legacy.yaml` leaves `mongo`, `backend`,
`director` and `admin` out and has the content server and the gateway wait for the metagame. Use both: the
profile alone would run the metagame next to the backend, on the same port. Then:

```bash
docker compose build
docker compose run --rm host setup --mode public --edition legacy --host play.example.org --name "Old Hunts"
docker compose up -d
docker compose run --rm host invite
```

The `metagame` container migrates its database (in the `workspace` volume) and registers the game-server key
before it serves. The `deploy` container starts the 1.4.4 game servers under Wine. Both read DR's own setting
names, so their containers hand them their env files with the `METAGAME_`/`DEPLOY_` prefix taken off. Put `winmm.dll`,
`UndauntedInternalServer.dll` and a `mystic_loader.ini` naming it beside the executable in your game folder. The `toml` profile works the same with
`edition = "legacy"` in `host.toml` (`COMPOSE_PROFILES=legacy,public,toml`).

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
| Update | `git pull --recurse-submodules` (once after a clone made without them: `git submodule update --init --recursive`), `docker compose build`, `docker compose up -d`. `reforged-host update` is not used in Docker |
| Back up | `docker compose run --rm host backup` (into the `workspace` volume, `backups/`), and keep a copy of the `workspace` volume: it holds the certificate every invite names and the secrets |
| A shell | `docker compose run --rm --entrypoint bash host` |

Never delete the `workspace` volume of a server that has players: a new certificate changes the fingerprint
in every invite, and new secrets log everyone out.

## Testing the images

`bash docker/run-tests.sh` (from the repository root) is the full test run CI's `docker` job does on the built
images:

1. It builds `front`, `server` and `test`. The `test` image is the `server` image plus the Rust toolchain, with
   the server crates' tests compiled in, so the code is tested on the OS, Node, Wine and paths production runs on.
2. Inside the `test` image: `cargo test` for the server crates (protocol, proxy, service-kit, store, gateway,
   content, host), with the allowlist's nftables test applying its rules for real in the container's own network
   namespace, then the Node backend's and Director's suites.
3. The compose smoke test (`docker/smoke-test.sh`): a public server set up on example game data in throwaway
   volumes, the `toml` profile's `configure` applying an edited `host.toml`, every service healthy, the gateway
   serving setup's certificate, the allowlist's table in place, an invite, a clean stop.

`bash docker/run-tests.sh rust` (or `backend`, `director`) runs only those suites in step 2; `SKIP_SMOKE=1`
leaves out step 3. On a server, `docker compose run --rm test` runs step 2 with your game-data volumes mounted
read-only, so the backend's and Director's game-data tests (skipped without real data) check your data too.

The smoke test runs a reborn server only: the `legacy` profile is checked as far as `docker compose config`.

## Moving an existing Linux host into Docker

Stop the systemd units, then copy the host's `.workspace` into the `workspace` volume and its game data into
the game-data volumes (`docker compose run --rm -v /home/reforged/Reforged:/old:ro --entrypoint bash host`,
then `cp -a` from `/old`), and the MongoDB database with `mongodump` and `mongorestore` into the `mongo`
container. Run setup in the container once, so the env files name the container's paths, and start it.

## Later in the migration

When the Rust backend and Director ship (RB14), the `backend` and `director` containers run the Rust bins, and
the `mongo` container gives way to Postgres (RD15) after the rollback window; the volumes and the `host`
commands stay.
