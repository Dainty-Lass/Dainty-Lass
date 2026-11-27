<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Hosting in Docker

The repository's root `compose.yaml` runs a whole server in Docker: the same services `dainty-host start` runs, one
container each, with the game servers under Wine. Read [hosting.md](hosting.md) for what the settings mean, the two
modes and invites; this page covers what differs in Docker. Without Docker, see [hosting-linux.md](hosting-linux.md).

## What runs

Every service of one server shares one network namespace, `net`'s. They reach each other on 127.0.0.1 exactly as
setup's env files say, and from outside only the gateway's port and the game ports are reachable, not even from other
containers. `net` holds the namespace and the published ports, so a service that restarts never drops another's.

| Service | Image | Profile | What it is |
| --- | --- | --- | --- |
| `net` | front | | Holds the namespace and publishes the gateway's TCP port and the game's UDP range |
| `backend` | server | | The 1.14.7 backend |
| `director` | server | | The Director, and the game servers under Wine as its children |
| `content` | front | | The content server |
| `admin` | front | | The admin API |
| `allowlist` | front | `public` | Opens the game ports only to players who signed in |
| `gateway` | front | | The gateway; starts last, once the others are healthy |
| `postgres` | `postgres:17` | `postgres` | A Postgres database instead of SQLite |
| `configure` | server | `toml` | Applies a `settings.toml` you keep outside Docker, before the others start |
| `remote-agent` | server | `managed` | The remote management agent of a managed install |
| `host` | server | `tools` | `dainty-host` for your commands; `up` never starts it |

Two images are built from `docker/Dockerfile`. **front** holds `dainty-host` alone: its containers run as an
ordinary user (uid 1000), with a read-only root, no capabilities and `no-new-privileges`. The allowlist is the one
exception: it gets `NET_ADMIN`, and keeps its nftables table inside the server's namespace, so it filters the game
servers' traffic without touching the host's firewall. **server** adds Wine, xvfb and the game data's examples; its
containers keep Docker's default capabilities, which Wine needs.

The server keeps its state in named volumes. The project name is fixed (`dainty-lass`), so the volumes keep their
names whatever folder you run Compose from.

| Volume | Mounted at | Holds |
| --- | --- | --- |
| `workspace` | `/opt/dainty-lass/.workspace` | Everything in `.workspace`: settings, secrets, the database, certificates, logs |
| `backend-game-data` | `/opt/dainty-lass/modules/rustic-isles/game-data/1.14.7/backend` | The backend's game data |
| `director-game-data` | `.../game-data/1.14.7/director` | The Director's game data |
| `exports` | `.../game-data/1.14.7/exports` | The CatalogExporter's exports |
| `captures` | `.../game-data/1.14.7/captures` | The store captures |
| `postgres-data` | the Postgres data folder | The Postgres database (`postgres` profile) |
| `DAINTY_GAME_DIR` (a folder) | `/game` | The Dauntless 1.14.7 folder |
| `DAINTY_IMPORT_DIR` (a folder) | `/import`, read-only, in `host` | Files for your commands to read, such as a game-data pack |
| `DAINTY_CONFIG_DIR` (a folder) | `/config`, read-only, in `configure` | The `toml` profile's `settings.toml` |

## Before you start

- Docker with Compose v2. The Tailscale override needs Compose 2.24 or later.
- A checkout of this repository: Compose builds the images from it, and runs from its root.
- The Dauntless 1.14.7 folder, or an empty folder for `install-game` to fill.
- Your game data as a pack from a Windows PC ([sdk-and-game-data.md](sdk-and-game-data.md)); without it the server
  runs on the examples and nobody can play.

Copy `.env.example` to `.env` beside `compose.yaml`. It holds Compose's settings, read on this machine; the server's
settings are setup's, never these.

| Variable | Default | What it is |
| --- | --- | --- |
| `COMPOSE_PROFILES` | `public` in `.env.example` | Which profiles run: `public` adds the allowlist; add `toml`, `postgres` or `managed` with commas |
| `COMPOSE_FILE` | | `compose.yaml:compose.tailscale.yaml` for a private server (`;` instead of `:` on Windows) |
| `TS_AUTHKEY` | | The Tailscale auth key, needed for the Tailscale node's first start only |
| `TS_HOSTNAME`, `TAILSCALE_IMAGE` | `dainty-lass`, `tailscale/tailscale:stable` | The Tailscale node's name and image |
| `DAINTY_GAME_DIR` | `./game` | The folder mounted at `/game` |
| `DAINTY_IMPORT_DIR` | `./import` | The folder mounted at `/import` in `host` |
| `DAINTY_CONFIG_DIR` | `./config` | The folder holding the `toml` profile's `settings.toml` |
| `DAINTY_GATEWAY_PORT` | `443` | The gateway's port in the container: setup's (443, or its `--public-port`) |
| `DAINTY_GATEWAY_PUBLISH` | `443` | Where it is published on this machine, `[address:]port`. Invites name setup's port, so keep the two equal |
| `DAINTY_GAME_PORTS` | `8770-8777` | The game servers' UDP range, the same on both sides: 8770-8777 for up to 6 hunts, one more below 8770 per hunt above 6 (8766-8777 for 10) |
| `DAINTY_GAME_PUBLISH_ADDRESS` | `0.0.0.0` | The address the game ports are published on |
| `DAINTY_COMMIT` | | The commit the server names to players as its source, baked in at build time (`git rev-parse --short=12 HEAD`) |
| `DAINTY_IMAGE`, `DAINTY_TAG` | `dainty-lass`, `local` | The images' names: `<DAINTY_IMAGE>-front`, `-server`, `-netem`, `-test` |
| `POSTGRES_IMAGE` | `postgres:17` | The `postgres` service's image |

The `DAINTY_NETWORK_*` variables in `.env.example` are for the network test harness only.

Build the images:

```sh
docker compose build
```

## Running commands

`docker compose run --rm host <command>` runs `dainty-host` in the server image, in the server's namespace and with
its volumes. Every command behaves as on a Linux host, except:

- `setup` adds `--skip-install` (the services are in the image), and `--game /game` when `/game/Archon` exists and you
  gave no `--game`. Ignore the systemd and ufw steps it prints: Compose runs the services.
- `install-game` installs into `/game` unless you pass `--dir`.
- `start` and `update` refuse, saying what to do instead (`docker compose up -d`; a new image).
- With no command, `host` runs `doctor`.

## A public server

1. In `.env`: `COMPOSE_PROFILES=public`, and `DAINTY_GAME_DIR` pointing at the 1.14.7 folder.
2. Set the server up:

   ```sh
   docker compose run --rm host setup --mode public --host play.example.org --game-address 203.0.113.7 \
     --name "Friday Hunts"
   ```

3. Bring in your game data: put the pack in the import folder, then

   ```sh
   docker compose run --rm host gamedata --import /import/gamedata.zip
   ```

4. Start everything, and check it:

   ```sh
   docker compose up -d
   docker compose ps
   docker compose run --rm host doctor
   ```

5. Invite a player:

   ```sh
   docker compose run --rm host invite --for "Alex"
   ```

The gateway listens on 443 as an ordinary user. To use another port, run setup with `--public-port 4443` and set both
`DAINTY_GATEWAY_PORT` and `DAINTY_GATEWAY_PUBLISH` to 4443. With more than six hunts (`--max-hunts`), widen
`DAINTY_GAME_PORTS` to match. Your host's or cloud provider's firewall must let the gateway's TCP port and the UDP
range through.

The Director waits for the allowlist to be healthy before it starts, so a public server's game ports are closed
before its first game server runs. Until setup has run, the Director exits and Compose tries it again.

### Getting the game from Epic

With an empty folder as `DAINTY_GAME_DIR`:

```sh
docker compose run --rm host install-game
docker compose restart director
```

It prints Epic's login link; open it in any browser, sign in and paste the code Epic shows. It installs 1.14.7 into
the `/game` mount and sets `game_dir` in `settings.toml`. The Epic session is kept in the `workspace` volume.

## A private server (Tailscale)

`compose.tailscale.yaml` turns `net` into a Tailscale node of its own: the gateway and the game servers listen on its
Tailscale address, and nothing is published on the host. In `.env`:

```sh
COMPOSE_FILE=compose.yaml:compose.tailscale.yaml
COMPOSE_PROFILES=
TS_AUTHKEY=tskey-auth-...
```

A private server runs no allowlist, so leave `public` out of `COMPOSE_PROFILES`. Then:

```sh
docker compose up -d net
docker compose exec net tailscale ip -4
docker compose run --rm host setup --mode private --host <that address> --name "Friday Hunts"
docker compose up -d
```

The node keeps its Tailscale state in the `tailscale` volume, so the auth key is needed for the first start only. It
uses a kernel network interface, for the game servers' UDP, so `net` gets `NET_ADMIN`, `NET_RAW` and `/dev/net/tun`.
Players reach it as any other machine on your tailnet; share it with them from Tailscale's admin console.

## Managed by settings.toml alone

The `toml` profile makes a `settings.toml` you keep outside Docker the server's whole configuration. On every
`docker compose up`, the `configure` service copies `DAINTY_CONFIG_DIR/settings.toml` into the workspace and runs
setup with it, before the other services start. Without a file there it stops and says so.

```sh
COMPOSE_PROFILES=public,toml
DAINTY_CONFIG_DIR=./config
```

A small public server's `config/settings.toml`:

```toml
version = 1
mode = "public"
host = "play.example.org"
game_address = "203.0.113.7"
name = "Friday Hunts"
game_dir = "/game"
registration = "INVITECODE"
max_hunt_servers = 3
```

Every key in [hosting.md](hosting.md#settings) works here. Setup keeps the copy in the workspace, so edit the file in
`DAINTY_CONFIG_DIR`, never the one in the volume, and run `docker compose up -d` to apply it.

## Importing Ramsgate Reborn saves

`docker compose run --rm -T host setup --legacy-mongo-uri < rr-uri.txt` (`-T` so the URI arrives on stdin), then
restart the stack. The old MongoDB must be reachable from the backend's container
([legacy-ramsgate.md](legacy-ramsgate.md)).

## Postgres

The `postgres` profile adds a Postgres server in the same namespace, listening on 127.0.0.1 only. Run setup once
first, so `secrets.env` has the password, then tell the server to use it:

```toml
[database]
engine = "postgres"
```

The defaults (`127.0.0.1`, port 5432, role and database `dainty`) are what the `postgres` service makes, so nothing
else is needed. Run setup again (or let `configure` do it), and start with the profile:

```sh
COMPOSE_PROFILES=public,postgres
docker compose up -d
```

The `postgres` service reads `DAINTY_DB_PASSWORD` from `secrets.env` when its data volume is first made, and stops
with a message when setup has not run yet. Every container shares the namespace, so it asks for the password on
127.0.0.1 too. The connection has no TLS, which is why it never leaves loopback. `dainty-host backup` covers SQLite
only; back Postgres up with `pg_dump`, for example:

```sh
docker compose exec postgres pg_dump -U dainty dainty > dainty.sql
```

## Several installs

A managed install runs the remote management agent: add `managed` to `COMPOSE_PROFILES`. To enroll one with a
controller's invite as you set it up, pass the invite to setup; it enrolls once, unless the install is already
managed:

```sh
docker compose run --rm -e DAINTY_REMOTE_ENROLL_URI="<the invite>" host setup --mode public --host alpha.example.org \
  --game-address 203.0.113.21 --name "Alpha"
```

The `remote` profile is a test harness for this: a controller and two managed installs on one machine, each mounting
only its own workspace (`docker compose --profile remote up -d --wait remote`, then `docker/remote/test.sh`).

The `network` profile is the same for a network of servers: four nodes, each in its own namespace, with latency
injected between them, and a CA that only the init container mounts (`docker compose --profile network up -d --wait
network`, then `docker/network/test.sh`). It plays no game, and is not a way to run a real network; for that, run
one Compose project per machine and set each node up as [hosting.md](hosting.md#network-a-network-of-servers) says.

## Day to day

```sh
docker compose ps                        # each service and its health
docker compose logs -f                   # everything; or name a service: logs -f director
docker compose restart director
docker compose run --rm host doctor
docker compose run --rm host backup      # into .workspace/backups in the workspace volume
docker compose down                      # stops everything; the volumes stay
```

A change to `settings.toml` takes a `docker compose run --rm host setup` (or, with `toml`, an `up`), then a restart of
the services it touches. Each service is healthy once its port takes a connection (`dainty-healthcheck`).

## Updating

A new version is a new image:

```sh
git pull
docker compose build
docker compose up -d
```

Run `docker compose run --rm host setup` afterwards when the release notes say so; it rewrites the env files for the
new version. Back up first ([hosting.md](hosting.md#backups)).

An older checkout that ran Compose from `restricted/` keeps its volumes, since the project name is the same: copy
`restricted/.env` to `.env` at the root and run Compose from there. Setup moves game data out of the old reference
tree as it does on any host ([Upgrading from an older install](hosting.md#upgrading-from-an-older-install)).

## Moving an existing Linux host into Docker

Stop the old server first. The volumes above take what its server folder had: `.workspace/` into `workspace`, and the
four `modules/rustic-isles/game-data/1.14.7/` folders into their volumes. Keep `.workspace/tls/` and
`.workspace/config/secrets.env` with it, or every invite and every player's saved server stops working. One way to
copy the workspace, as the containers' user (uid 1000), which must be able to read the old files:

```sh
docker compose run --rm -v /srv/old-server:/old:ro --entrypoint cp host \
  -a /old/.workspace/. /opt/dainty-lass/.workspace/
```

Then point the settings at the container's paths and check:

```sh
docker compose run --rm host setup --game /game --wine ""
docker compose run --rm host doctor
```

`--wine ""` drops a Wine path from the old machine, so the image's own is found. The systemd units and the ufw
script setup wrote on the old host have no part here; disable them there.

## Testing the images

```sh
docker/run-tests.sh                      # the images, every suite in the test image, then the three harnesses
docker compose run --rm test             # only the server suites, in the test image
docker/smoke-test.sh                     # a public server without a game: setup, health, certificate, allowlist, invite
```

`run-tests.sh` skips a harness with `SKIP_SMOKE=1`, `SKIP_NETWORK=1` or `SKIP_REMOTE=1`. The test image runs the
allowlist's real firewall test when the container has `NET_ADMIN`, which the `test` service gives it. None of this
plays the game.
