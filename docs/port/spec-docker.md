<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: the Docker deployment (RD27, port plan item 7)

This page specifies `restricted/docker/`, `restricted/compose.yaml`, `restricted/compose.tailscale.yaml` and
`restricted/.env.example` as a host and CI see them, so they can be re-created as MIT files at the monorepo root. The
implementer works from this page alone and never opens `restricted/`. Everything here is behaviour: image contents
and paths, Compose service names, profiles, volumes, ports, variables, entry points, exit codes, health checks and
the test harnesses' steps and expected results. Facts a host or the host kit already depends on are **fixed** and
must not change (they are marked so); the rest is free.

The images run MIT binaries the root workspace builds. The only AGPL bytes they may still carry are data files
(section 2.4). Nothing from the Node reference, the C++ trees or restricted's Rust crates goes into an MIT image.

Notation: **Call D-n** is a decision this page made, which the implementer follows and the reviewer checks.
**Q-n** is an open question for the maintainer, with the answer this page assumes until one comes. **[RB15]** marks
what exists only for the one release after the RU9/RB14 cutover and goes when RB15 deletes the Node backend and the
MongoDB handling.

---

## 1. Scope and what replaces what

| Today (`restricted/`) | Lines | At the root (MIT) |
| --- | --- | --- |
| `docker/Dockerfile` | 245 | `docker/Dockerfile` (section 2) |
| `compose.yaml` | 918 | `compose.yaml` (section 3) |
| `compose.tailscale.yaml` | 39 | `compose.tailscale.yaml` (section 3.9) |
| `.env.example` | 54 | `.env.example` (section 4) |
| `docker/healthcheck.sh` | 30 | `docker/healthcheck.sh` (section 5.1) |
| `docker/host-entrypoint.sh` | 77 | `docker/host-entrypoint.sh` (section 5.2) |
| `docker/director-entrypoint.sh` | 45 | `docker/director-entrypoint.sh` (section 5.3) |
| `docker/network/init.sh`, `netem.sh` | 87 + 74 | `docker/network/init.sh`, `netem.sh` (sections 5.4, 5.5) |
| `docker/test-in-image.sh` | 60 | `docker/test-in-image.sh` (section 6.1) |
| `docker/run-tests.sh` | 37 | `docker/run-tests.sh` (section 6.2) |
| `docker/smoke-test.sh` | 150 | `docker/smoke-test.sh` (section 6.3) |
| `docker/network/test.sh` | 632 | `docker/network/test.sh` (section 6.4) |
| `docker/remote/test.sh` | 258 | `docker/remote/test.sh` (section 6.5) |

**Call D-1: where it lives.** `compose.yaml`, `compose.tailscale.yaml` and `.env.example` at the repository root,
the build context `.` (the root), and everything else under a root `docker/` folder with the same relative layout as
today (`docker/network/`, `docker/remote/`). Compose reads `.env` from the folder of the first compose file, so a host
runs `docker compose ...` from the root. A host moving from a `restricted/` checkout copies `restricted/.env` to the
root `.env`; the volumes carry over because the project name is fixed (section 3.1).

**Call D-2: the build-context ignore file.** The root's `.dockerignore` is today the context filter for restricted's
Dockerfile too. The new Dockerfile takes its own `docker/Dockerfile.dockerignore` (BuildKit reads the file beside the
Dockerfile in preference to `.dockerignore`), so neither build changes the other's context. Its content is section
2.6.

**What stays as the oracle** (the integration pass and the reviewer run these against the new images; neither is
rewritten into the oracle's place until restricted's Docker is deleted):

- `restricted/docker/run-tests.sh` and `restricted/docker/smoke-test.sh`, run against the new images and the new
  compose file (section 8).
- The new root `docker/smoke-test.sh` and `docker/run-tests.sh` are the implementer's own, written from sections
  6.2 and 6.3. Both pairs must pass.

---

## 2. The images

Four images are deployed or used by the harnesses, one is test-only, and the rollback image is not ported.

| Image | Compose name (fixed) | Base | Deployed |
| --- | --- | --- | --- |
| `front` | `${DAINTY_IMAGE:-dainty-lass}-front:${DAINTY_TAG:-local}` | Debian slim (trixie) | Yes |
| `server` | `${DAINTY_IMAGE:-dainty-lass}-server:${DAINTY_TAG:-local}` | Debian slim (trixie) | Yes |
| `netem` | `${DAINTY_IMAGE:-dainty-lass}-netem:${DAINTY_TAG:-local}` | Debian slim (trixie) | Test harness only |
| `test` | `${DAINTY_IMAGE:-dainty-lass}-test:${DAINTY_TAG:-local}` | `server` | Never |
| `server-rollback` | none in compose.yaml today | Node 22 slim | **[RB15] not ported** (Call D-9) |

The Dockerfile's build arguments, each with its default (keep the names; CI and hosts pass them):

| `ARG` | Default | Use |
| --- | --- | --- |
| `RUST_IMAGE` | `rust:1-slim-trixie` | The build stage's base |
| `DEBIAN_IMAGE` | `debian:trixie-slim` | Every runtime image's base |
| `DAINTY_MODULES` | `all-modules` | Comma-separated features of the server package that link the game modules. Empty builds none, which still serves. Each word `f` becomes `--features <server package>/f` |
| `DAINTY_COMMIT` | empty | Baked into the `server` image as `ENV DAINTY_SOURCE_COMMIT` (section 2.3) |
| `NODE_IMAGE`, `MONGO_IMAGE` | `node:22-trixie-slim`, `mongo:8.0` | **[RB15]** only the rollback stages use them; not ported |

The file starts with `# syntax=docker/dockerfile:1` and uses BuildKit cache mounts.

### 2.1 Build stages

1. **`rust-sources`** (`FROM scratch`): what cargo reads, under `/src`. Today it copies both workspaces, the
   launcher's `package.json` (the launcher kit's build reads its version) and restricted's module data. After lane A
   (section 7) it is the root workspace alone: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/`,
   `crates/`, `launcher/package.json`, and whatever data files the root crates' builds and tests read. The Node
   code, native code, the dashboard and the docs are not copied.
2. **`rust-build`** (`FROM ${RUST_IMAGE}`): installs `build-essential pkg-config ca-certificates git`; installs the
   pinned toolchain from `rust-toolchain.toml` in its own layer (`rustup show active-toolchain || rustup toolchain
   install`) before copying the sources; then builds with cache mounts for the cargo registry, cargo git and the
   target folder (cache ids `dainty-cargo-registry`, `dainty-cargo-git`, `dainty-cargo-target`; the ids are free, but
   keeping them lets a CI runner's cache survive the switch). It produces:
   - `/out/bin/dainty-host`: `cargo build --release --locked -p <server package> <features> --bins`, the one server
     binary (RD26) with the gateway, allowlist, content server, admin API, backend and Director inside;
   - `/out/bin/dainty-legacy`: the root's MIT legacy importer binary (`-p dainty-legacy --bin dainty-legacy`);
   - `/out/plugins/`: the `.so` libraries `settings.toml` can pin (RD27), as the release ships them: the 1.14.7 module
     library and `libdainty_content_plugin.so`, `libdainty_chat_plugin.so`, `libdainty_gateway_plugin.so`
     (`cargo build --release --locked -p <pkg> --lib` for each). Q-3 covers the module library's file name.
   The launcher is never built.
3. **`front`**, **`server`**, **`netem`**, **`test`**: sections 2.2 to 2.5.

### 2.2 `front`: gateway, allowlist, content server, admin API

`FROM ${DEBIAN_IMAGE}`. Contents and settings, all **fixed** unless said otherwise:

- Packages: `ca-certificates`, `nftables`. `setcap cap_net_admin+ep /usr/sbin/nft` is applied with `libcap2-bin`,
  which is then purged (`apt-get purge -y libcap2-bin && apt-get autoremove -y`), and the apt lists are removed.
  `nft` is the only file in the image with a capability; the allowlist helper itself runs unprivileged and gets
  `CAP_NET_ADMIN` only through `nft` (and only when the container's bounding set has it, section 3.5).
- User and group `dainty`, uid 1000, gid 1000, home `/opt/dainty-lass` (not created as a home), shell
  `/usr/sbin/nologin`.
- `/opt/dainty-lass/.workspace` exists, owned `dainty:dainty`.
- `/opt/dainty-lass/bin/dainty-host` (from `rust-build`) and `/opt/dainty-lass/bin/dainty-healthcheck` (section 5.1,
  mode 0755).
- `ENV PATH=/opt/dainty-lass/bin:$PATH`, `WORKDIR /opt/dainty-lass/.workspace`, `USER dainty`.
- No `ENTRYPOINT` and no `CMD`: compose gives each service its command and `init: true` (Docker's init) reaps.
- Labels: `org.opencontainers.image.title="Dainty Lass front door"`,
  `org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass"`, and
  `org.opencontainers.image.licenses` (Q-1).

The `front` image needs `bash` (the health check is bash, section 5.1); Debian slim has it.

### 2.3 `server`: the host kit, backend and Director, Wine

`FROM ${DEBIAN_IMAGE}`. The image is laid out as the server release is (RD8), under `/opt/dainty-lass`, so every path
`dainty-host setup` writes into the env files is the same in every container. **Fixed:**

- Packages: `ca-certificates wine wine64 xvfb xauth tini` (Wine on a virtual display for the Windows game servers;
  tini reaps them). Apt lists removed.
- User and group `dainty`, uid/gid 1000, as in `front`. The `dainty` user owns everything a host writes.
- `WORKDIR /opt/dainty-lass`.
- **The project-root marker.** The host kit finds its server folder by walking up from the working folder, then from
  the executable's folder, for a folder that has `Cargo.toml` together with `modules/rustic-isles/` (or a
  `release.json`, which has a strict format and must not be faked). The image therefore has
  `/opt/dainty-lass/Cargo.toml` and the folder `/opt/dainty-lass/modules/rustic-isles/`. Call D-3: the `Cargo.toml`
  is the root's own MIT manifest (its content is never read, only its presence), owned `dainty:dainty`.
- The folders under `modules/rustic-isles/reference/1.14.7/` (call it `ISLES`), each owned `dainty:dainty` because
  compose mounts volumes over them (a new named volume takes the image folder's ownership and files):
  - `ISLES/ParadoxBackend/game-data/` holding the **example** game data (`*.example.json`) only,
  - `ISLES/ParadoxBackend/store-captures.json`,
  - `ISLES/ParadoxDirector/game-data/` holding the example game data only,
  - `ISLES/Items_Analysis/` and `ISLES/DauntlessEndpointDocumentation/`, empty,
  - `.workspace/`, empty.
  These names come from the MIT host kit's layout (`dainty-hostkit`'s paths), not from Paradox's tree, and setup
  writes them into the env files: they are fixed even though the folder names read as Node's.
- `bin/` from `rust-build`: `dainty-host`, `dainty-legacy`. `plugins/` from `rust-build`.
- Scripts, mode 0755: `bin/dainty-director` (section 5.3), `bin/dainty-host-docker` (section 5.2),
  `bin/dainty-healthcheck` (section 5.1), `bin/dainty-network-init` (section 5.4).
- Licence files at `/opt/dainty-lass/`: the root's `LICENSE.txt` and, while the image carries AGPL data (section
  2.4), that data's licence and notice beside it (Q-1). Today the image also carries restricted's `README.md`,
  `NOTICE.md`, `ADDITIONAL_TERMS.md` and the module's `LICENSE`, `NOTICE.md`, `ADDITIONAL_TERMS.md`: those go with
  the AGPL data, under `data/` (Call D-4).
- `ENV DAINTY_SOURCE_COMMIT=${DAINTY_COMMIT}` and `PATH=/opt/dainty-lass/bin:$PATH`. `setup` reads the commit from git
  or `release.json`, neither of which is in the image; the process environment wins over the env file, so the
  backend names this commit to players (the source offer). Call D-5: keep it even when the binaries are MIT, since
  the shipped data is AGPL and the variable costs nothing.
- `USER dainty`, `ENTRYPOINT ["/usr/bin/tini", "--"]`, `CMD ["dainty-host", "--help"]`.
- Labels: title `"Dainty Lass server"`, source as in `front`, licences per Q-1.

**Not in the MIT image (Call D-6):** today's `native/` (the runtime loader's source), `modules/rustic-isles/native/`
(the runtime's and CatalogExporter's C++ sources) and restricted's `Cargo.toml`. `setup` checks for those sources
only in a checkout and only without `--skip-install`; the host entrypoint always adds `--skip-install` (section 5.2),
so their absence is invisible to setup. `doctor` may report them missing, which is already one of its "expected and
harmless" lines in Docker. The smoke test's `doctor` step tolerates failure (section 6.3).

### 2.4 Data the images carry

| File in the image | Source (repository) | Licence | Read by |
| --- | --- | --- | --- |
| `/opt/dainty-lass/data/1.14.7/version_data.json` | Lane A's shipped file (section 7) | AGPL (Mystic Paradox) | The backend at start-up (default path) |
| `ISLES/ParadoxBackend/game-data/*.example.json`, `ISLES/ParadoxDirector/game-data/*.example.json` | The 1.14.7 module's example game data | AGPL today (Q-2) | `gamedata --examples`, the smoke test |
| `ISLES/ParadoxBackend/store-captures.json` | The module's store captures | AGPL today (Q-2) | setup and `gamedata` build Lady Luck's Store from it |

No other game data and no exports are baked in. A host's own game data lives in volumes.

### 2.5 `netem`: latency for the `network` profile (DN12)

`FROM ${DEBIAN_IMAGE}`; package `iproute2` only; `/usr/local/bin/dainty-netem` (section 5.5, mode 0755);
`ENTRYPOINT ["/usr/local/bin/dainty-netem"]`. Runs as root (the default) but with `cap_drop: [ALL]` and
`cap_add: [NET_ADMIN]` in compose. Title label `"Dainty Lass network harness latency"`. The health check (in
compose) is `tc qdisc show dev eth0 | grep -q netem`, so the image needs `sh`, `tc` and `grep`.

### 2.6 `test`: every server suite on the server image

`FROM server AS test`. Never deployed. As root:

- Adds `build-essential pkg-config git nftables libcap2-bin procps libdbus-1-dev`, and re-applies
  `setcap cap_net_admin+ep /usr/sbin/nft`. `procps` because the host kit's `start` tests send Ctrl+C with `kill(1)`;
  `libdbus-1-dev` because the gateway's tests take the launcher kit, whose keyring links D-Bus on Linux.
- Copies `/usr/local/cargo` and `/usr/local/rustup` from `rust-build`, `chmod -R a+rwX` both;
  `ENV CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup PATH=/usr/local/cargo/bin:/opt/dainty-lass/bin:$PATH
  CARGO_TERM_COLOR=never`.
- The sources from `rust-sources` at `/opt/src`, owned `dainty:dainty`, laid out as the repository is.
- `bin/dainty-test` (section 6.1).
- As `dainty`, at build time: `cd /opt/src && cargo build --locked -p dainty-gateway-plugin --lib` first (the
  controller's tests load that library, and a cdylib is not built for its own tests), then
  `cargo test --locked --no-run -p <each suite package>` (section 6.1's list). The downloaded crates stay in the
  image: a run rebuilds nothing and needs no network.
- `ENTRYPOINT ["/usr/bin/tini", "--", "dainty-test"]`, `CMD []`.

`docker/Dockerfile.dockerignore` (Call D-2) excludes, as today's `.dockerignore` does: `.git`, `.github`, `.claude`,
`.workspace`, `**/node_modules`, `**/dist`, `**/build`, `**/target`, `**/target*` (but not `**/src/**/target.rs`),
`paradox`, `**/*.tsbuildinfo`, `**/.env`, `**/.env.*` (but not `**/.env.example`), `**/*.pem`, `**/*.key`, `**/*.pfx`,
`**/*.log` (but not `**/tests/fixtures/**`, which hold fake env files and certificates the test image runs), a host's
own game data (every `game-data/*.json` that is not `*.example.json`, `game-data/raw`, `Items_Analysis`,
`DauntlessEndpointDocumentation`), `ui`, `docs`, and every `restricted/` path except the data files section 2.4 copies
(Call D-7: the MIT build reads nothing else from `restricted/`, and the ignore file makes that checkable).

---

## 3. `compose.yaml`

### 3.1 Top level

- `name: dainty-lass` (**fixed**: the volume names `dainty-lass_workspace` and so on derive from it, and existing
  hosts' data is in them). The harnesses override it with `COMPOSE_PROJECT_NAME`.
- Every `build:` is `context: .` (the root, Call D-1), `dockerfile: docker/Dockerfile`, with the `target` named per
  service. Every `server`-image build passes `args: DAINTY_COMMIT: ${DAINTY_COMMIT:-}`.
- Three YAML anchors carry the common settings (names free):
  - **front service**: image `…-front`, target `front`, `network_mode: service:net`, `restart: unless-stopped`,
    `init: true`, `read_only: true`, `tmpfs: [/tmp]`, `security_opt: [no-new-privileges:true]`, `cap_drop: [ALL]`.
  - **server service**: image `…-server`, target `server`, `network_mode: service:net`, `restart: unless-stopped`,
    and the server volume set (section 3.3). No `read_only`, no `cap_drop`, no `security_opt` (Q-5).
  - the `network` and `remote` profile anchors (sections 3.6, 3.7).

### 3.2 The single server's services

Service names are **fixed** (hosts type them, docs and scripts name them). "Front" and "server" mean the anchors.

| Service | Anchor / image | Profile | Command | Health check (`interval`, `start_period`, `retries`) | `depends_on` |
| --- | --- | --- | --- | --- | --- |
| `net` | front image, own settings | none | `["sleep", "infinity"]` | `["CMD", "true"]` every 30s | — |
| `mongo` | `${MONGO_IMAGE:-mongo:8.0}` | `mongo` **[RB15]** | section 3.4 | section 3.4 | `net` started |
| `postgres` | `${POSTGRES_IMAGE:-postgres:17}` | `postgres` | section 3.4 | `pg_isready -h 127.0.0.1 -U dainty -d dainty`; 5s, 30s, 6 | `net` started; `configure` completed successfully, `required: false` |
| `configure` | server | `toml` | entrypoint `["/usr/bin/tini", "--", "dainty-host-docker"]`, command `["apply"]`, `restart: "no"` | none | `net` started; `mongo` healthy `required: false` **[RB15]** |
| `backend` | server | none | `["dainty-host", "backend", "serve", "--env-file", "/opt/dainty-lass/.workspace/config/backend.env"]` | `dainty-healthcheck backend`; 5s, 120s, 6 | `net` started; `postgres` healthy (`required: false`); `configure` completed successfully (`required: false`) |
| `director` | server | none | `["dainty-director"]` | `dainty-healthcheck director`; 5s, 120s, 6 | `backend` healthy; `allowlist` healthy `required: false` (a public server's game ports are closed before the first game server starts) |
| `content` | front | none | `["dainty-host", "content", "serve", "--env-file", "/opt/dainty-lass/.workspace/config/content.env"]` | `dainty-healthcheck content`; 5s, 30s | `backend` healthy |
| `admin` | front | none | `["dainty-host", "admin", "serve", "--env-file", "/opt/dainty-lass/.workspace/config/admin.env"]` | `dainty-healthcheck admin`; 5s, 30s | `backend` healthy |
| `allowlist` | front, with overrides (section 3.5) | `public` | `["dainty-host", "allowlist", "--env-file", "/opt/dainty-lass/.workspace/config/allowlist.env"]` | `dainty-healthcheck allowlist`; 5s, 30s | `net` started; `configure` completed successfully `required: false` |
| `gateway` | front | none | `["dainty-host", "gateway", "serve", "--env-file", "/opt/dainty-lass/.workspace/config/gateway.env"]` | `dainty-healthcheck gateway`; 5s, 30s | `net` healthy; `backend` healthy; `content` healthy; `allowlist` healthy `required: false` |
| `remote-agent` | server | `managed` | `["dainty-host", "remote-agent"]` | none | `gateway` healthy |
| `host` | server | `tools` | entrypoint `["/usr/bin/tini", "--", "dainty-host-docker"]`, command `["doctor"]`, `restart: "no"` | none | none |
| `test` | `…-test`, target `test` | `test` | the image's | none | none |

Health checks use the `["CMD", ...]` form with every `test`. The gateway starts last, as the host kit's `start` does,
so no player request reaches a service that is not up. `host` has the `tools` profile only so that `up` never starts
it; `docker compose run --rm host …` runs it anyway.

`net` (own settings, **fixed**): `init: true`, `restart: unless-stopped`, `read_only: true`,
`security_opt: [no-new-privileges:true]`, `cap_drop: [ALL]`, `sysctls: net.ipv4.ip_unprivileged_port_start: "0"` (the
gateway listens on 443 as an ordinary user), and the only published ports of the single server:

```yaml
ports:
  - "${DAINTY_GATEWAY_PUBLISH:-443}:${DAINTY_GATEWAY_PORT:-443}/tcp"
  - "${DAINTY_GAME_PUBLISH_ADDRESS:-0.0.0.0}:${DAINTY_GAME_PORTS:-8770-8777}:${DAINTY_GAME_PORTS:-8770-8777}/udp"
```

**One network namespace.** Every single-server service joins `net`'s namespace (`network_mode: service:net`), so they
reach each other on 127.0.0.1 exactly as setup's env files say, and nothing but the gateway and the game ports is
reachable from outside, not even from other containers. `net` holds the namespace and the ports so a service
restarting never drops another's. `net` itself is on the project's default network.

### 3.3 Volumes and mounts

Named volumes (all at the top-level `volumes:`, **fixed** names): `workspace`, `backend-game-data`,
`director-game-data`, `exports`, `captures`, `mongo-data` **[RB15]**, `mongo-config` **[RB15]**, `postgres-data`,
`network-ca`, `network-oregon`, `network-new-york`, `network-uk`, `network-germany`, `remote-controller`,
`remote-alpha`, `remote-bravo`.

The **server volume set** (`backend`, `director`, `remote-agent` through the anchor; `configure` and `host` repeat it
and add one bind mount each):

| Source | Target |
| --- | --- |
| `workspace` | `/opt/dainty-lass/.workspace` |
| `mongo-data` **[RB15]** | `/opt/dainty-lass/.workspace/mongodb` |
| `backend-game-data` | `/opt/dainty-lass/modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data` |
| `director-game-data` | `/opt/dainty-lass/modules/rustic-isles/reference/1.14.7/ParadoxDirector/game-data` |
| `exports` | `/opt/dainty-lass/modules/rustic-isles/reference/1.14.7/Items_Analysis` |
| `captures` | `/opt/dainty-lass/modules/rustic-isles/reference/1.14.7/DauntlessEndpointDocumentation` |
| `${DAINTY_GAME_DIR:-./game}` (bind) | `/game` |

Extra mounts: `configure` adds `${DAINTY_CONFIG_DIR:-./config}:/config:ro,z` (`z` relabels for SELinux hosts and is
ignored elsewhere); `host` adds `${DAINTY_IMPORT_DIR:-./import}:/import:ro`. The front services: `content` and
`gateway` mount `workspace` read-only at `/opt/dainty-lass/.workspace`; `admin` and `allowlist` mount it read-write
(the admin API writes its timeouts next to `secrets.env`; the allowlist writes its state). `postgres` mounts
`postgres-data:/var/lib/postgresql/data` and `workspace:/opt/dainty-lass/.workspace:ro`. `test` mounts
`backend-game-data` and `director-game-data` read-only at the server paths, so the game-data tests check the
server's real data. `mongo` mounts `mongo-data:/data/db` and `mongo-config:/data/configdb` **[RB15]**.

### 3.4 The database services

**`postgres`** (profile `postgres`): `network_mode: service:net`, `restart: unless-stopped`, and an entrypoint
`["sh", "-ec", <script>]` that:

1. reads `DAINTY_DB_PASSWORD` from `/opt/dainty-lass/.workspace/config/secrets.env`, the value optionally in double
   quotes (`DAINTY_DB_PASSWORD="…"` or `DAINTY_DB_PASSWORD=…`);
2. if empty, prints `no DAINTY_DB_PASSWORD in <path>: run setup first` to stderr and exits 1;
3. exports `POSTGRES_PASSWORD` (that value), `POSTGRES_USER=dainty`, `POSTGRES_DB=dainty`,
   `POSTGRES_INITDB_ARGS=--auth-host=scram-sha-256` (initdb trusts 127.0.0.1 by default, and every container shares
   this namespace, so passwords there too);
4. `exec docker-entrypoint.sh postgres -c listen_addresses=127.0.0.1`.

In compose's YAML every `$` in the script is doubled (`$$`) so Compose does not interpolate it. The password is read
only when the data volume is first made. libpq is built without TLS (DF7), which is why it stays on loopback.

**`mongo`** **[RB15]** (profile `mongo`): `network_mode: service:net`, `restart: unless-stopped`, command
`["mongod", "--replSet", "rs0", "--bind_ip", "127.0.0.1", "--port", "27017", "--wiredTigerCacheSizeGB", "1"]`. Health
check: `mongosh --quiet --eval "try { rs.status() } catch (e) { rs.initiate({ _id: 'rs0', members: [{ _id: 0, host:
'127.0.0.1:27017' }] }) } quit(db.hello().isWritablePrimary ? 0 : 1)"`, interval 5s, timeout 10s, start_period 60s,
retries 12: it initiates the one-member replica set the first time and is healthy once primary. It exists to import a
pre-cutover server's MongoDB data into `store` (`host db migrate`, `host db verify`).

### 3.5 Capabilities and privileges (**fixed**)

- Every `front`-image service: `cap_drop: [ALL]`, `no-new-privileges:true`, read-only root, `/tmp` on tmpfs.
- **`allowlist`**: as front, but `security_opt: []` and `cap_add: [NET_ADMIN]`. `no-new-privileges` must be off,
  because it stops a file capability from taking effect, and `nft`'s file capability is how the unprivileged helper
  gets `CAP_NET_ADMIN`. The allowlist's nftables table (`inet dainty_lass`) lives in the shared namespace: it filters
  the game servers' traffic without any privilege on the host and without touching the host's firewall. It is the
  only single-server service with a capability.
- `netem-*`: `cap_drop: [ALL]`, `cap_add: [NET_ADMIN]`, in a node's namespace; the node's own containers keep none.
- `network-init`: `user: "0:0"`, `cap_drop: [ALL]`, `cap_add: [CHOWN, DAC_OVERRIDE, FOWNER]` (to write into the node
  volumes and hand them to uid 1000), read-only root, `/tmp` tmpfs, `network_mode: none`.
- `test`: `cap_add: [NET_ADMIN]` on its own network, for the allowlist's real firewall test.
- The `server`-image services keep Docker's default capability set (Q-5).

### 3.6 The `network` profile (DN12): four nodes on one machine

A test cluster of four nodes, the canary's regions, with injected latency between them. Run alone with
`docker compose --profile network up -d --wait network`: the single server's services have no profile, so a bare
`--profile network up` would start the single server too. Every service below has `profiles: [network]`.

Per region `R` in `oregon`, `new-york`, `uk`, `germany` (node id `R-1`, address `${DAINTY_NETWORK_SUBNET:-172.28.77}.N`
with N = 11, 12, 13, 14 in that order, published port `${DAINTY_NETWORK_PORT_<REGION>:-1844k}` with k = 1, 2, 3, 4,
the region in upper case with `-` as `_`):

| Service | Image | What |
| --- | --- | --- |
| `net-R` | front | `command: ["sleep", "infinity"]`, `hostname: R-1`, on network `cluster` at the fixed `ipv4_address`, `ports: ["${DAINTY_NETWORK_PUBLISH_ADDRESS:-127.0.0.1}:<port>:443/tcp"]`, the `net` sysctl, `init`, `restart: unless-stopped`, read-only, `no-new-privileges`, `cap_drop: [ALL]`, health `true` every 30s |
| `netem-R` | netem | `network_mode: service:net-R`, environment = the shared netem table (below) plus `NETEM_SELF: R`, `init`, `restart: unless-stopped`, read-only, privileges per 3.5, health `tc qdisc show dev eth0 \| grep -q netem` every 2s, 15 retries; depends on `net-R` started |
| `configure-R` | server | `network_mode: none`, `restart: "no"`, entrypoint `["/usr/bin/tini", "--", "dainty-host-docker"]`, command `["apply"]`, `no-new-privileges`, `cap_drop: [ALL]`, **not** read-only (setup writes the image's game-data folders, which a node with no game never reads); volume `network-R:/opt/dainty-lass/.workspace`; depends on `network-init` completed successfully |
| `gateway-R` | **server** | `network_mode: service:net-R`, environment `DAINTY_NETWORK_FULL_NODE: "1"` and `DAINTY_WRONG_NODE_POLICY: ${DAINTY_WRONG_NODE_POLICY:-refuse}`, command `["dainty-host", "gateway", "serve", "--env-file", "/opt/dainty-lass/.workspace/config/gateway.env"]`, `restart: unless-stopped`, `init`, `no-new-privileges`, `cap_drop: [ALL]`, not read-only; volume `network-R:/opt/dainty-lass/.workspace`; health `dainty-healthcheck gateway` every 5s, start_period 30s; depends on `net-R` healthy, `netem-R` healthy, `configure-R` completed successfully |

With `DAINTY_NETWORK_FULL_NODE=1` the gateway process serves the whole node (the harness reaches the backend on
127.0.0.1:41000 and chat on 127.0.0.1:41006 inside `gateway-R`), which is why it uses the server image.

The shared netem environment (each `$` default as shown):

```text
NETEM_NODES="oregon=<subnet>.11 new-york=<subnet>.12 uk=<subnet>.13 germany=<subnet>.14"
NETEM_RTT="oregon:new-york=${DAINTY_NETWORK_RTT_OREGON_NEW_YORK:-70} oregon:uk=${…_OREGON_UK:-140}
  oregon:germany=${…_OREGON_GERMANY:-150} new-york:uk=${…_NEW_YORK_UK:-75}
  new-york:germany=${…_NEW_YORK_GERMANY:-85} uk:germany=${…_UK_GERMANY:-20}"   (one line, space-separated)
NETEM_PLAYER_REGION=${DAINTY_NETWORK_PLAYER_REGION:-germany}
```

Two more services:

- `network-init`: server image, entrypoint `["/usr/bin/tini", "--", "dainty-network-init"]`, environment
  `NETWORK_NODES: "oregon=oregon-1@<subnet>.11 new-york=new-york-1@<subnet>.12 uk=uk-1@<subnet>.13
  germany=germany-1@<subnet>.14"`, `restart: "no"`, privileges per 3.5; volumes `network-ca:/ca` (nothing else mounts
  it) and `network-R:/nodes/R` for each region.
- `network`: front image, `command: ["sleep", "infinity"]`, `network_mode: none`, `init`, read-only,
  `no-new-privileges`, `cap_drop: [ALL]`, health `true` every 30s; depends on all four `gateway-R` healthy. It exists
  so `up --wait network` starts the four nodes and nothing else.

The network **`cluster`** (top-level `networks:`):
`ipam.config: [{subnet: "${DAINTY_NETWORK_SUBNET:-172.28.77}.0/24"}]`.
The addresses are fixed because the node certificates and tc's filters name them.

### 3.7 The `remote` profile (RM1-RM5): a controller and two managed installs

Every service has `profiles: [remote]`. Started with `docker compose --profile remote up -d --wait remote`.

| Service | Built on | What |
| --- | --- | --- |
| `net-controller`, `net-alpha`, `net-bravo` | the `net-R` settings | `hostname` `controller`, `alpha`, `bravo`; `cluster` addresses `<subnet>.20`, `.21`, `.22`; published `${DAINTY_NETWORK_PUBLISH_ADDRESS:-127.0.0.1}:${DAINTY_REMOTE_PORT_CONTROLLER:-18450}` / `…_ALPHA:-18451` / `…_BRAVO:-18452` to 443/tcp |
| `configure-controller` | the `configure-R` settings | entrypoint `["/usr/bin/tini", "--", "sh", "-ec"]`, one command string: `dainty-host-docker setup --mode public --host controller.cluster.test --game-address 203.0.113.20 --name "Fleet Controller" --wine /nonexistent/wine --xvfb --fresh-start`, then `dainty-host remote init-controller --node controller --region oregon --url https://<subnet>.20:443`; volume `remote-controller` at the workspace |
| `configure-alpha`, `configure-bravo` | the same | `dainty-host-docker setup --mode public --host alpha.cluster.test --game-address 203.0.113.21 --name "Install Alpha" --wine /nonexistent/wine --xvfb --fresh-start` (bravo: `bravo.cluster.test`, `203.0.113.22`, `"Install Bravo"`); volumes `remote-alpha`, `remote-bravo` |
| `gateway-controller`, `gateway-alpha`, `gateway-bravo` | a **front**-image gateway: the `gateway-R` command and health check, `init`, read-only, `/tmp` tmpfs, `no-new-privileges`, `cap_drop: [ALL]` | `network_mode: service:net-<x>`, environment `DAINTY_REMOTE_WIRE_LOG: /opt/dainty-lass/.workspace/wire.jsonl`, the install's volume at the workspace (read-write). Controller depends on its `net` healthy and its `configure` completed; alpha and bravo on their `net` healthy and their `backend-*` and `content-*` healthy |
| `backend-alpha`, `backend-bravo` | server anchor | `network_mode: service:net-<x>`, the backend command, the install's volume only, health `dainty-healthcheck backend` 5s / start_period 60s / 6 retries; depend on their `net` healthy and `configure` completed |
| `content-alpha`, `content-bravo` | front anchor | the content command, the install's volume read-only, health as `content`; depend on their backend healthy |
| `host-controller`, `host-alpha`, `host-bravo` | server anchor | `network_mode: service:net-<x>`, `command: ["sleep", "infinity"]`, `DAINTY_REMOTE_WIRE_LOG` as above, the install's volume only, health `true` every 10s; depend on their gateway healthy |
| `remote` | the `network` service's settings | depends on the three `host-*` healthy |

Volumes listed for these services replace the anchor's set (each install's containers mount only its own workspace
volume).

### 3.8 The `test` service

Image `…-test`, target `test`, `profiles: [test]`, `restart: "no"`, `cap_add: [NET_ADMIN]`, its own (default)
network, the two game-data volumes read-only (section 3.3). `docker compose run --rm test [rust]`.

### 3.9 `compose.tailscale.yaml`: a private server over Tailscale

An override file, used as `docker compose -f compose.yaml -f compose.tailscale.yaml …` or with
`COMPOSE_FILE=compose.yaml:compose.tailscale.yaml` in `.env` (`;` on Windows). It replaces `net` so the shared
namespace becomes a Tailscale node of its own: the gateway and the game servers listen on its Tailscale address and
nothing is published on the host. `net` becomes:

- `image: ${TAILSCALE_IMAGE:-tailscale/tailscale:stable}`, with `build: !reset null`, `command: !reset null`,
  `read_only: !reset false`, `security_opt: !reset []`, `cap_drop: !reset []`, `ports: !reset []` (Compose 2.24+);
- `cap_add: [NET_ADMIN, NET_RAW]`, `devices: ["/dev/net/tun:/dev/net/tun"]`;
- environment `TS_AUTHKEY: ${TS_AUTHKEY:-}` (needed for the first start only),
  `TS_HOSTNAME: ${TS_HOSTNAME:-dainty-lass}`,
  `TS_STATE_DIR: /var/lib/tailscale`, `TS_USERSPACE: "false"` (a kernel interface, for the game servers' UDP);
- volume `tailscale:/var/lib/tailscale` (top-level volume `tailscale`);
- health `["CMD", "tailscale", "ip", "-4"]`, interval 5s, start_period 60s.

The host then runs `docker compose up -d net`, `docker compose exec net tailscale ip -4`, and
`docker compose run --rm host setup --mode private --host <that address> --name "…"`.

---

## 4. The variables Compose reads, and `.env.example`

These are Compose's own, read on the host. Server settings are never here: they are setup's. A new variable goes in
`.env.example` with a comment and is interpolated as `${VAR:-default}` with the default the comment gives.

| Variable | Default | In `.env.example` | Used by |
| --- | --- | --- | --- |
| `COMPOSE_PROFILES` | none (Compose's) | `public`, with comments for `toml` and `postgres` (`public,toml`, `public,postgres`) | which profiles run |
| `COMPOSE_FILE` | none | commented: `compose.yaml:compose.tailscale.yaml` (`;` on Windows) | private servers |
| `TS_AUTHKEY` | empty | commented: `tskey-auth-...` | tailscale override |
| `TS_HOSTNAME`, `TAILSCALE_IMAGE` | `dainty-lass`, `tailscale/tailscale:stable` | no | tailscale override |
| `DAINTY_GAME_DIR` | `./game` | `./game` | `/game` bind mount. An empty folder works: `install-game` fetches 1.14.7 into `<folder>/1.14.7` |
| `DAINTY_IMPORT_DIR` | `./import` | `./import` | `/import` (read-only, `host`) |
| `DAINTY_CONFIG_DIR` | `./config` | `./config` | `/config` (read-only, `configure`) |
| `DAINTY_GATEWAY_PORT` | `443` | `443` | the container port; must equal setup's port (443 or `--public-port`) |
| `DAINTY_GATEWAY_PUBLISH` | `443` | `443` | the host side, `[address:]port`; the invites name setup's port, so keep them equal |
| `DAINTY_GAME_PORTS` | `8770-8777` | `8770-8777` | UDP range, the same on both sides. 8770-8777 for up to 6 hunts, one more below 8770 per hunt above 6 (`8766-8777` for 10) |
| `DAINTY_GAME_PUBLISH_ADDRESS` | `0.0.0.0` | `0.0.0.0` | UDP publish address |
| `DAINTY_COMMIT` | empty | commented (`git rev-parse --short=12 HEAD`) | build arg (section 2.3) |
| `DAINTY_IMAGE`, `DAINTY_TAG` | `dainty-lass`, `local` | commented | every image name |
| `POSTGRES_IMAGE` | `postgres:17` | commented | `postgres` |
| `MONGO_IMAGE` | `mongo:8.0` | no | `mongo` **[RB15]** |
| `DAINTY_NETWORK_SUBNET` | `172.28.77` | commented | `cluster`, every node address (three octets) |
| `DAINTY_NETWORK_PUBLISH_ADDRESS` | `127.0.0.1` | commented | every `net-*` publish address |
| `DAINTY_NETWORK_PORT_OREGON` / `_NEW_YORK` / `_UK` / `_GERMANY` | `18441` / `18442` / `18443` / `18444` | commented | `net-R` ports |
| `DAINTY_NETWORK_RTT_OREGON_NEW_YORK`, `_OREGON_UK`, `_OREGON_GERMANY`, `_NEW_YORK_UK`, `_NEW_YORK_GERMANY`, `_UK_GERMANY` | `70`, `140`, `150`, `75`, `85`, `20` | commented | `NETEM_RTT` (round trips in ms; each way takes half) |
| `DAINTY_NETWORK_PLAYER_REGION` | `germany` | commented | `NETEM_PLAYER_REGION` |
| `DAINTY_WRONG_NODE_POLICY` | `refuse` | no | `gateway-R` |
| `DAINTY_REMOTE_PORT_CONTROLLER` / `_ALPHA` / `_BRAVO` | `18450` / `18451` / `18452` | no | `net-controller`, `net-alpha`, `net-bravo` |

`.env.example`'s opening comment says to copy it to `.env`, that server settings are setup's
(`docker compose run --rm host setup ...`), and the network block is introduced as test-only ("None of this is a
server's setting"). The comments are the implementer's to write; the names, defaults and which lines are commented
out are as in the table.

Variables a container reads (not Compose): `DAINTY_REMOTE_ENROLL_URI` (section 5.2; a host passes it with `-e`),
`DAINTY_ROLLBACK` **[RB15]** (section 5.3), the `NETEM_*` and `NETWORK_NODES` lists, `DAINTY_NETWORK_FULL_NODE`,
`DAINTY_WRONG_NODE_POLICY`, `DAINTY_REMOTE_WIRE_LOG`, `DAINTY_SOURCE_COMMIT`.

---

## 5. Entry points and helper scripts

All are POSIX `sh` with `set -eu` unless said otherwise; comments terse per AGENTS.md.

### 5.1 `dainty-healthcheck <name>` (bash)

`<name>` is one of `gateway`, `allowlist`, `content`, `admin`, `backend`, `director`; anything else prints
`unknown service <name>` to stderr and exits 2; no argument prints a usage line and fails. It reads each setting as
the service does: the **process environment first**, then the last `KEY=` line of
`/opt/dainty-lass/.workspace/config/<name>.env` with one pair of surrounding double quotes stripped, then the default:

| Name | Address variable (default) | Port variable (default) |
| --- | --- | --- |
| `gateway` | `GATEWAY_BIND` (`0.0.0.0`) | `GATEWAY_PORT` (`443`) |
| `allowlist` | `ALLOWLIST_BIND` (`127.0.0.1`) | `ALLOWLIST_PORT` (`41005`) |
| `content` | `BIND_HOST` (`127.0.0.1`) | `PORT` (`41002`) |
| `admin` | `BIND_HOST` (`127.0.0.1`) | `PORT` (`41003`) |
| `backend` | `BIND_HOST` (`127.0.0.1`) | `PORT` (`41000`) |
| `director` | `BIND_HOST` (`127.0.0.1`) | `PORT` (`41001`) |

An address of `0.0.0.0` is tried as `127.0.0.1`. Healthy (exit 0) once a TCP connection to the address and port
opens (bash's `/dev/tcp`), unhealthy otherwise. This is the same readiness the host kit's `start` waits for. Note the
generic `BIND_HOST`/`PORT` names: a service's env file is the only one read, so they do not collide.

### 5.2 `dainty-host-docker` (the `host`, `configure` and `configure-*` entry point)

Runs `dainty-host` inside the server image, in the server's namespace, with its volumes; every command behaves as on
a Linux host except:

- A helper `has <flag> <args…>` is true when an argument is exactly `<flag>` or starts with `<flag>=`. "Adds X" below
  means only when the command line does not already have it.
- **docker setup** (used by `setup` and `apply`): adds `--skip-install` (the services are in the image); adds
  `--game /game` when `/game/Archon` is a directory; runs `dainty-host setup <args…>`. Then, when
  `DAINTY_REMOTE_ENROLL_URI` is set and non-empty and `/opt/dainty-lass/.workspace/config/remote-managed.json` does not
  exist, runs `dainty-host remote enroll "$DAINTY_REMOTE_ENROLL_URI"`.
- `start`: prints to stderr `In Docker the services are compose's: docker compose up -d (docker compose logs -f shows
  them).` and exits **2**.
- `update`: prints to stderr two lines, that a new version is a new image (`git pull && docker compose build && docker
  compose up -d`) and to run `docker compose run --rm host setup` afterwards if the release notes say so; exits **2**.
- `install-game [args…]`: adds `--dir /game`; `exec dainty-host install-game <args…>`.
- `apply` (no arguments): if `/config/settings.toml` exists, creates the folder and copies it over
  `/opt/dainty-lass/.workspace/config/settings.toml`, printing `settings.toml taken from DAINTY_CONFIG_DIR`; else, if
  the workspace has no `settings.toml`, prints to stderr `No settings.toml: put one in DAINTY_CONFIG_DIR (./config by
  default; docs/hosting-docker.md shows how).` and exits **2**. Then docker setup with no arguments; exits 0. So in the
  `toml` profile the file in `DAINTY_CONFIG_DIR` is the server's whole configuration.
- `setup [args…]`: docker setup; then prints a blank line and a three-line note: ignore the systemd and ufw steps
  above, `docker compose up -d` starts it (`COMPOSE_PROFILES=public` in `.env` for a public server), and an example
  `docker compose run --rm host invite --for "Alex"`; exits 0.
- Anything else, including no argument: `exec dainty-host <args…>`. (The compose default command is `doctor`.)

The exact wording of the printed notes is free (host-kit terminal output stays English); the exit codes and the
`settings.toml` message's meaning are fixed, since the harnesses and docs rely on them.

### 5.3 `dainty-director` (the `director` service)

Does what `dainty-host start` does before the Director on Linux, then runs it:

1. If `/opt/dainty-lass/.workspace/config/director.env` is missing or
   `/opt/dainty-lass/.workspace/bin/dainty-gameserver` is not executable: prints
   `director | not set up yet: docker compose run --rm host setup ... (docs/hosting-docker.md)`
   to stderr and exits **78** (`EX_CONFIG`; with `restart: unless-stopped` the container retries).
2. The Wine prefix is `/opt/dainty-lass/.workspace/wine`. If it has no `system.reg`: prints `director | making the
   Wine prefix (once, a minute or two)`, creates it, runs `wine wineboot --init` under `xvfb-run -a` with
   `WINEPREFIX` set, `WINEDEBUG=-all` and `WINEDLLOVERRIDES="winmm=n,b;mscoree=d;mshtml=d"`, output (both streams)
   written over `.workspace/logs/wine.log`, failure ignored; then `wineserver -w` (failure ignored);
   if `system.reg` is still missing, prints a warning naming `.workspace/logs/wine.log`. Done here so the first game
   server's readiness timeout does not include it; setup usually made the prefix already.
3. Reads `DAINTY_GAME_EXE` from `director.env` (quotes stripped). If it names an existing file, ensures an empty
   `CALL_HUNTPASS_QUERY.flag` exists in that file's folder (a runtime switch the game servers need beside the
   executable; failure ignored). Otherwise prints two warning lines: no game executable at `<path or (none)>`, the
   Director runs but no game server can start, and mount the 1.14.7 folder (`DAINTY_GAME_DIR`) and run setup with
   `--game /game`. It does not exit.
4. `exec dainty-host director serve --env-file /opt/dainty-lass/.workspace/config/director.env`.

The game servers are the Director's children (`.workspace/bin/dainty-gameserver`, which is `dainty-host gameserver`,
Wine on a virtual display), in this container, so they stop with it.

**[RB15] not ported:** today, with `DAINTY_ROLLBACK=1`, step 4 instead runs the Node Director
(`node --env-file=<director.env> dist/server.js` in `ISLES/ParadoxDirector`). The MIT image has no Node (Call D-9).

### 5.4 `dainty-network-init` (the `network-init` service)

Runs as root with `/ca` and `/nodes/<region>` for each node. Input `NETWORK_NODES`, space-separated
`region=node-id@address` entries (required: unset or empty fails with a message). Port 443 throughout.

1. If `/ca/cluster-ca.pem` does not exist: `dainty-host network ca-init --out /ca/cluster-ca.pem` (output discarded),
   then prints `cluster CA made in the network-ca volume`.
2. For each node: creates `/nodes/<region>/tls` and `/nodes/<region>/config`; if `tls/gateway-cert.pem` is missing,
   `dainty-host gateway make-cert --host <region>.cluster.test --host <address> --out <tls>`; if `tls/node-cert.pem`
   is missing, `dainty-host network issue-node-cert --ca /ca/cluster-ca.pem --node <id> --region <region> --host
   <address> --host <region>.cluster.test --out <tls>`; records the gateway certificate's fingerprint from
   `dainty-host gateway make-cert --fingerprint <tls>/gateway-cert.pem` (the text after `Fingerprint:` and spaces).
3. For each node, writes `/nodes/<region>/config/settings.toml` afresh (the only file rewritten on a second run, so the
   CA, every certificate and every fingerprint survive):

   ```toml
   version = 1
   mode = "public"
   host = "<region>.cluster.test"
   game_address = "203.0.113.<last octet of address>"
   name = "Network node <id>"
   registration = "OPEN"
   max_hunt_servers = 3

   [switches]
   rate_limit = "off"

   [linux]
   wine = "/nonexistent/wine"
   xvfb = true

   [network]
   node = "<id>"
   region = "<region>"
   ```

   followed, for every other node in list order, by a blank line and

   ```toml
   [[network.nodes]]
   id = "<other id>"
   region = "<other region>"
   host = "<other address>"
   port = 443
   fingerprint = "<other node's gateway fingerprint>"
   ```

   The documentation range address and the nonexistent Wine are deliberate: the harness plays no game. Comments in
   the generated file are free.
4. `chown -R 1000:1000 /nodes/<region>`; `chmod 600` on `tls/node-key.pem` and `tls/gateway-key.pem`; prints
   `<id> (<region>, <address>): certificates and settings.toml written`.
5. `chmod 700 /ca`. The CA's key never leaves `/ca`, which no other service mounts.

### 5.5 `dainty-netem` (the `netem-*` services)

Inputs: `NETEM_SELF` (this node's region), `NETEM_NODES` (`region=address` list), `NETEM_RTT`
(`regionA:regionB=milliseconds` list), all required; `NETEM_PLAYER_REGION` (optional); `NETEM_IFACE` (default `eth0`).
Shapes this namespace's **egress** with tc:

- A round trip between two regions is 0 for the same region, else the table's value for the pair in either order.
  A missing pair prints `no round trip for <a> and <b> in NETEM_RTT` and exits 2; a value that is not a plain
  non-negative decimal (digits and at most one `.`, not `.` alone) prints `not a number of milliseconds: '<v>'` and
  exits 2.
- Deletes any root qdisc (ignoring failure), then adds a root `prio` qdisc, handle `1:`, with `bands = peers + 2`
  (more than 16 bands prints `too many nodes for one prio qdisc` and exits 2) and a priomap of 16 entries all equal
  to `bands - 1` (everything unmatched goes to the last band).
- Band 1 is untouched: a filter `protocol arp parent 1: prio 1 u32 match u32 0 0 flowid 1:1` (ARP is never delayed).
- Each other node, in list order, gets the next band b (2, 3, …): a `netem delay <half>ms` qdisc at `parent 1:b`,
  handle `<b*10>:`, where half is the pair's round trip divided by 2 printed with one decimal; and a filter
  `protocol ip parent 1: prio 2 u32 match ip dst <address>/32 flowid 1:b`. Prints `netem: <self> -> <region>
  (<address>): <half> ms each way`. The two directions add up to the table's value.
- The last band (everything else: the Docker host, which plays a player) gets `netem delay <rtt>ms`, rtt the whole
  round trip between `NETEM_PLAYER_REGION` and this node (0 when unset), because nothing delays the way in. Prints
  `netem: <self> -> everything else (a player in <region or "no region">): <rtt> ms`.
- Prints `tc qdisc show dev <iface>` and `exec sleep infinity`.

---

## 6. The tests and harnesses

All harness scripts are bash with `set -euo pipefail`, run from the folder holding `compose.yaml`, print each step as
a blank line and `== <step>`, and on exit (trap) print `compose ps -a` and the logs' tail when they failed after
starting, then `compose down --volumes --remove-orphans` (output discarded, failure ignored). The network and remote
harnesses export `MSYS_NO_PATHCONV=1` (Git Bash would otherwise rewrite `/opt/...` paths handed to docker).

### 6.1 `dainty-test` (the test image's entry point, bash, `set -uo pipefail`)

Arguments are suite names; none means `rust`. `NO_COLOR=1`. The only suite is **`rust`**:

- If `/usr/sbin/nft list tables` succeeds (the container's bounding set has `CAP_NET_ADMIN`), exports
  `DAINTY_TEST_FIREWALL=1` and prints `(firewall test: on, this container has CAP_NET_ADMIN)`; else prints
  `(firewall test: off; run with --cap-add NET_ADMIN to include it)`. With it on, the allowlist's firewall test
  applies its nftables rules for real in the container's own namespace.
- `cd /opt/src && cargo test --locked --no-fail-fast -p dainty-protocol -p dainty-proxy -p dainty-remote
  -p dainty-service-kit -p dainty-store -p dainty-gateway -p dainty-content -p dainty-admin -p dainty-hostkit
  -p dainty-host -p dainty-controller` (the list the test image pre-builds; after lane A, add the server package that
  moved to the root, Q-4).

An unknown suite prints `unknown suite <name> (rust)` to stderr and fails that suite with 2. Each suite runs in a
subshell, timed; then `========== summary` and one line per suite, `passed (<n> s)` or `FAILED (<n> s)`. The exit
status is 1 when any suite failed, else 0. (Compose's comment today also names `backend` and `director` suites; they
no longer exist and are not ported.)

### 6.2 `run-tests.sh`: the full run (CI's Docker job)

1. `docker compose build`, then `docker compose --profile test build test`.
2. `docker run --rm --cap-add NET_ADMIN <DAINTY_IMAGE or dainty-lass>-test:<DAINTY_TAG or local> <args…>` (the
   arguments pick the in-image suites).
3. Unless `SKIP_SMOKE=1`: the smoke test with `SMOKE_NO_BUILD=1` (and `MSYS_NO_PATHCONV` unset, since the smoke test
   passes Windows-safe paths itself).
4. Unless `SKIP_NETWORK=1`: the network harness with `NETWORK_OPTIONAL=1` (skipped with a note, not failed, where
   Docker cannot give `CAP_NET_ADMIN` or netem).
5. Unless `SKIP_REMOTE=1`: the remote harness with `REMOTE_NO_BUILD=1`.
6. Prints `full test run passed`.

`COMPOSE_FILE` is exported as `compose.yaml` (Call D-8: export it only when unset, so the oracle can point the
scripts at another file).

### 6.3 `smoke-test.sh`: a public server without a game

Environment it sets: `COMPOSE_PROJECT_NAME` (default `dainty-smoke`), `COMPOSE_PROFILES=public`, `COMPOSE_FILE`
(Call D-8), `DAINTY_GATEWAY_PORT=443`, `DAINTY_GATEWAY_PUBLISH=127.0.0.1:${SMOKE_PORT:-18443}`,
`DAINTY_GAME_PUBLISH_ADDRESS=127.0.0.1`, and fresh temporary folders for `DAINTY_GAME_DIR`, `DAINTY_IMPORT_DIR` and
`DAINTY_CONFIG_DIR` (removed on exit). Cleanup runs `down --volumes --remove-orphans` with
`COMPOSE_PROFILES=public,toml,mongo`. Builds first unless `SMOKE_NO_BUILD=1`.

Steps and the expected result of each (every check failing fails the test):

1. **A fresh `toml` apply with no MongoDB succeeds.** Writes `settings.toml` into the config folder:
   `version = 1`, `mode = "public"`, `host = "smoke.example.org"`, `game_address = "203.0.113.7"`,
   `name = "Fresh Toml Apply"`, `registration = "INVITECODE"`, `max_hunt_servers = 1`. Runs
   `COMPOSE_PROFILES=public,toml compose run --rm configure` (exit 0), then checks with
   `compose run --rm --entrypoint sh host -c 'cat /opt/dainty-lass/.workspace/config/backend.env'` that `backend.env`
   contains `Fresh Toml Apply`. Then `down --volumes`.
2. **[RB15] Seed a pre-cutover `mongo-data` volume.** `COMPOSE_PROFILES=public,mongo compose up -d --wait mongo`, then
   `mongosh` in `mongo` against `mongodb://127.0.0.1:27017/reforged?replicaSet=rs0` inserts: into `accounts` one
   document (`_id` ObjectId `507f1f77bcf86cd799439011`, `userId` `user-docker-01`, `name`, `displayName`
   `SlayerDocker`, `displayNameNormalized` `slayerdocker`, `email` `slayer.docker@example.invalid`, a dummy bcrypt
   `passwordHash` beginning `$2b$10$`, `status` `active`, `approvalStatus` `approved`, `roles` `["player"]`,
   `usernameSet` 1, `createdAt` 2026-01-15T12:00:00Z); into `characters` one (`_id` `507f1f77bcf86cd799439012`,
   `characterId` `char-docker-01`, `userId` `user-docker-01`, `name` `SlayerDocker`, `createdDate` the same time); into
   `reforgedInvites` one (`invitecode` `INV-DOCKER-01`, `usesRemaining` 5, `infiniteUses` false, a note, `createdAt`).
3. **[RB15] The backend refuses unmigrated data.** `compose run --rm host gamedata --examples` (exit 0); then
   `compose run --rm backend` must **fail**, and its output must contain `Unmigrated MongoDB data`.
4. **[RB15] Migrate and verify.** `compose run --rm host setup --mode public --host smoke.example.org --game-address
   203.0.113.7 --name "Smoke Test"`, then `compose run --rm host db migrate` and `compose run --rm host db verify`
   (each exit 0; verify writes the marker that lets the backend start); then `compose rm -sf mongo`.
5. **The `toml` profile applies `settings.toml` alone.** Copies the workspace's `settings.toml` out (through the `host`
   service with entrypoint `sh`), replaces its `name = …` line with `name = "Smoke Toml"`, writes it to the config
   folder, runs `configure` again, and checks `backend.env` contains `Smoke Toml`.
6. **Up, configure first.** `COMPOSE_PROFILES=public,toml compose up -d --wait --wait-timeout 300` succeeds (every
   service with a health check is healthy, `configure` completed); `compose ps`.
7. **The gateway serves setup's certificate.** The SHA-256 fingerprint `openssl s_client -connect 127.0.0.1:18443
   -servername smoke.example.org` sees equals that of `/opt/dainty-lass/.workspace/tls/gateway-cert.pem`.
8. **The gateway proxies the Rust backend.** With that certificate as the CA and `--resolve
   smoke.example.org:18443:127.0.0.1`: `GET https://smoke.example.org:18443/dainty/api/ServerStatus` succeeds and the
   body contains `"online":true`. **[RB15]** `POST /dainty/api/Register` with
   `{"Username":"SlayerDocker","InviteCode":"INV-DOCKER-01"}` answers a body containing `"username_taken"` (the
   migrated account). `POST /dainty/api/Register` with `{"Username":"DockerNewbie","InviteCode":"INV-DOCKER-01"}`
   succeeds with a body containing `"UUK"` (the migrated invite works). After RB15, with steps 2-4 gone, the invite
   comes from `host invite` instead (Q-6).
9. **[RB15] The migrated invite is listed.** `compose run --rm host invite --list` output contains `INV-DOCKER-01`.
10. **The allowlist closes the game ports.** `compose exec -T allowlist nft list table inet dainty_lass` contains
    `udp dport 8770-8777 drop`.
11. **Invite.** `compose run --rm host invite --for "Smoke"` output contains `dainty://`.
12. **Doctor.** `compose run --rm host doctor --no-hash`; its failure is ignored.
13. **Clean stop.** `compose stop`; afterwards `compose ps --status running --quiet` is empty. Prints
    `smoke test passed`.

### 6.4 `network/test.sh`: the `network` profile (DN12)

Settings: `COMPOSE_PROJECT_NAME` default `dainty-network`, `COMPOSE_PROFILES=network`, `COMPOSE_FILE` (Call D-8),
`NETWORK_TOLERANCE_MS` (default 25), `NETWORK_REGION_PING` (`auto` default, `require` in CI, or `skip`),
`NETWORK_NO_BUILD=1`, `NETWORK_OPTIONAL=1` (when Docker or netem is unavailable, print `network harness skipped:
<why>` and exit 0 instead of failing). Failures print `network test failed: <why>` and exit 1.

1. Docker with Compose must answer (`docker compose version`, `docker info`), else "unavailable".
2. Unless `NETWORK_NO_BUILD=1`, builds `net-oregon`, `configure-oregon`, `netem-oregon` (one per image).
3. **NET_ADMIN and netem:** `docker run --rm --cap-add NET_ADMIN --entrypoint sh <netem image> -c 'tc qdisc add dev
   lo root netem delay 1ms'` must succeed, else "unavailable" (missing capability or `sch_netem`). The image name is
   found in `compose config --images` by `-netem:`.
4. Reads back from `compose config --format json` the `NETEM_RTT` table and `NETEM_PLAYER_REGION`, so the expectations
   follow whatever `DAINTY_NETWORK_*` the run uses.
5. `compose up -d --wait --wait-timeout 300 network`.
6. **Peers and latencies:** in each `gateway-R`, `dainty-host network status --count 5 --env-file
   /opt/dainty-lass/.workspace/config/gateway.env` succeeds; its lines starting `Peer:` are read as whitespace
   fields `_ node peer state _ rtt unit _`: exactly 3, each `state` = `up`, `unit` = `ms`, and the rtt within
   `[injected - 2, injected + tolerance]`.
7. **The launcher's region ping** (unless skipped: `skip`; or in `auto` when `cargo` is missing or the oregon node's
   address is not reachable on 443 from this host, as on Docker Desktop; in `require` those fail): for each home
   region, with the published `net-R` port (0.0.0.0 read as 127.0.0.1), the home's gateway fingerprint, and the
   expected order of the other regions nearest-first from the player's region (`region=rtt` joined by `,`), runs the
   launcher's ignored test `the_region_order_follows` with `RR_HARNESS_HOME`, `RR_HARNESS_PIN`, `RR_HARNESS_EXPECT`,
   `RR_HARNESS_TOLERANCE_MS`. Q-7 covers which crate holds that test.
8. **Players:** `POST <oregon>/dainty/api/Register {"Username":"SlayerOregon"}` returns a `"UUK"`; `GET
   /dainty/api/GetUserInfo` with `x-dainty-user-api-key` returns `"UserId"` and an `x-dainty-account-id` header;
   `POST /account/api/oauth/token` (form `grant_type=exchange_code&exchange_code=<UUK>`) returns `"access_token"`. The
   same for `SlayerGermany` on germany. (All `curl -k`, through the published ports.)
9. **DN8, cross-node matchmaking:** both players `POST /candidate/join {"gameMode":"ISLAND","playerHuntId":
   "CR19_PlayerHunt_Example"}` on their nodes; polling `GET /candidate/status` (up to 40 × 0.5 s) both reach
   `"status":"IN_PROGRESS"` with the same `gameSessionId` and `host`. The session id ends `@<node-id>`, naming the
   host node. On the host node, with its `METAGAME_API_KEY` from `director.env` as `x-rustic-isles-gameserver-apikey`,
   raw HTTP to 127.0.0.1:41000 inside `gateway-<host>`:
   `POST /progression/<remote account>/ExperienceTrack_PlayerLevel/250`
   → 200; `POST /inventory` with `{accountId, characterId: "char-<account>", transactionId: "tx-ring-loot-1",
   addStackedItems: [{catalogId: "MonsterPart_Gnasher_Tail", quantity: 15}]}` → body with `"forwarded_to_home":true`;
   `POST /bounty/<account>` with `{"bounties":[{"bounty_id":"BOUNTY_RING_TEST","progress":1}]}` → 200. Then on the
   remote player's home node, with their bearer token: `GET /progression/<UserId>` contains `250`,
   `GET /inventory/<UserId>/char-<account>` contains `MonsterPart_Gnasher_Tail`, `GET /bounty/<UserId>` contains
   `BOUNTY_RING_TEST`.
10. **DN6, wrong node:** oregon's player joins `{"gameMode":"ISLAND","playerHuntId":"Example_Hunt_Easy"}` and gets an
    instance; germany's player joining `{"gameMode":"ISLAND","instanceId":<it>}` on germany answers **409** with
    `WRONG_NODE`. Then `wrong_node_policy = "redirect"` is added under `[network]` in germany's `settings.toml`,
    `compose restart gateway-germany`, wait for `ServerStatus` `"online":true`; a second oregon instance; germany's join
    now answers with `"hostNode":"oregon-1"` and an `x-dainty-session-ticket` header. On oregon, with only that header
    (no account key): `PUT /gamesession/epiceos {}` → 200 with `"sessiontoken"`; `POST /candidate/join` with the
    instance → 200 naming `"instanceId":<it>`.
11. **DN9, regional Ramsgate:** `POST <oregon>/network/v1/regions` (with the account key) reporting germany nearest
    (`germany 12, uk 35, new-york 85, oregon 150`) → **204**; a `{"gameMode":"CITY","playerHuntId":
    "ShatteredIsles_ReturnToRamsgate"}` join reaches a candidate with `"host":"203.0.113.14"`. Reporting oregon
    nearest (`oregon 8, new-york 70, uk 140, germany 150`) → 204, and the next city join gets `"host":"203.0.113.11"`.
12. **DN10b, chat federation:** a WebSocket upgrade to `<oregon>//` with `Sec-WebSocket-Protocol: xmpp` answers `101
    Switching Protocols` and echoes `sec-websocket-protocol: xmpp`. Through the chat plugin's test endpoints on
    127.0.0.1:41006 inside each `gateway-R` (`POST /__chat/client/login|send|close`, `GET /__chat/client/drain`,
    `/__chat/history/direct`, `/__chat/history/waiting`, `POST /__chat/mute|unmute`): a whisper oregon → germany
    arrives once and is in both nodes' direct history; a whisper to germany's player while offline queues at germany,
    is delivered once with `urn:xmpp:delay` when that player logs into chat on uk with a uk session ticket, and the
    queue clears; a hunt room joined on oregon and uk (status `110`) fans messages out across nodes; a mute applied
    at germany (the muted player's home) yields `<forbidden` on uk and the message does not reach oregon; after
    unmute it does. Prints `network harness passed`.

### 6.5 `remote/test.sh`: the `remote` profile (RM1-RM5)

Settings: `COMPOSE_PROJECT_NAME` default `dainty-remote`, `COMPOSE_PROFILES=remote`, `COMPOSE_FILE` (Call D-8),
`REMOTE_NO_BUILD=1` (also passes `--no-build` to `up`), `REMOTE_KEEP_UP=1` (skips the teardown). Failures print
`remote test failed: <why>`.

1. Unless `REMOTE_NO_BUILD=1`, builds `net-controller configure-controller gateway-controller host-controller`.
   Tears down any previous volumes, then `up -d --wait --wait-timeout 300 remote`.
2. **Enrollment:** `dainty-host remote invite --node alpha --region us-east --label "Install Alpha"` (and bravo,
   `eu-west`) in `host-controller` print a URI starting `dainty-lass-remote://enroll?`. `dainty-host remote enroll
   <uri> --json` in `host-alpha` reports `"node": "alpha"`; enrolling again with the same URI **fails** (single use);
   bravo enrolls the same way. `remote sync --json` on both reports `"applied_revision": 1`; `remote list --json` on
   the controller names both.
3. **Config sync:** on the controller, `settings.toml`'s `name` becomes `"Fleet Friday Hunts"` and `max_hunt_servers` 4;
   `remote set-config --json` reports `"revision": 2`; both installs' `sync` report `"applied_revision": 2`; each
   install's `settings.toml` has the new name and count but keeps its own `game_address` (`203.0.113.21`, `.22`); each
   install's `secrets.env` SHA-256 is unchanged, and the two differ.
4. **Remote ban:** `dainty-host invite --for Griefer99 --json` on alpha; on the controller `remote exec --node alpha
   --action ban --account Griefer99 --reason "fleet rule violation" --role admin --actor ctrl-admin --json` reports
   `"status": "queued"`; after alpha's `sync`, alpha's `config/remote-moderation.json` has `"target": "Griefer99"`,
   `"status": "banned"`, `"actor": "remote:controller:ctrl-admin"`, and the controller's `remote status --json`
   contains `banned applied for Griefer99 by remote:controller:ctrl-admin`.
5. **Controller offline:** `compose stop gateway-controller`; `dainty-healthcheck backend` and `gateway` still pass in
   alpha's and bravo's containers; alpha's `sync` fails; alpha's `remote status --json` shows `"detached": false` and
   `"applied_revision": 2`. `compose start gateway-controller` and its health check passes within 30 s.
6. **Revocation:** `remote revoke bravo --json` reports `"revoked": "bravo"`; bravo's `sync` fails and its status shows
   `"detached": true`; `remote exec --node bravo --action doctor …` on the controller fails; alpha's next sync reports
   `"applied_revision": 3`.
7. **Wire hygiene:** the three `wire.jsonl` files are non-empty; alpha's has lines starting `enroll_send:`,
   `enroll_recv:`, `managed_send:`, `managed_recv:`; the controller's `controller_recv:` and `controller_send:`. None
   of the three contains any of `DAINTY_ADMIN_KEY`, `DAINTY_ADMIN_API_KEY`, `DAINTY_SECRET`, `GATEWAY_SECRET`,
   `ALLOWLIST_SECRET`, `RUSTIC_ISLES_MANAGER_OPERATOR_SECRET`, `DAINTY_DB_PASSWORD`, `DLK_`, `PRIVATE KEY`,
   `SQLite format 3`, `$argon2`, `$scrypt`, nor any value of 8 or more characters from any of the three `secrets.env`
   files. Prints `remote management harness passed`.

---

## 7. After lane A: binaries from the root, version data as a shipped file

Lane A (port plan items 1 and 2) moves `dainty-server`, `dainty-server-minimal` and the module library to the root
workspace and makes the version data a file read at start-up, defaulting to `data/1.14.7/version_data.json` beside
the binary's install root. The MIT Dockerfile is written for that state; it does not build until lane A is on `plan`.

- **`rust-sources`** copies the root workspace only, plus nothing from `restricted/` (Call D-7). No
  `DAINTY_VERSION_DATA` is set at build time; today's restricted `.cargo/config.toml` that set it is not copied.
- **`rust-build`** works in `/src` (not `/src/restricted`), one target folder, one cargo invocation per artefact:
  the server package (its root name from lane A) with `DAINTY_MODULES`; the four plugin libraries, all root
  packages; `dainty-legacy`.
- **`server`** gains `/opt/dainty-lass/data/1.14.7/version_data.json` (section 2.4), owned `root` and read-only to
  `dainty` (Call D-10: it is shipped data, not a host's file, so a volume never covers it and the host cannot edit
  it in place; a host who wants another file names it in `settings.toml`, mounting it through `/import` or the
  workspace). The `front` image does not need it: no front service loads the module. If lane A's start-up check fails
  without the file, every server-image service refuses to start with lane A's message, which the smoke test's step 6
  would show.
- **`test`** runs the root workspace's suites; the root crates' tests read whatever fixtures they read from the
  root. Nothing in the test image comes from `restricted/`.
- Until lane A lands, restricted's Dockerfile stays the one CI builds. The MIT one is added beside it and the CI
  switch is the integration pass's (section 9).

---

## 8. The oracle runs

The reviewer (who may open `restricted/`) proves the new files are drop-in:

1. **New images, old compose.** From the root, build the four images with the new compose file and the default
   `DAINTY_IMAGE`/`DAINTY_TAG`, so they carry the names restricted's `compose.yaml` uses. Then, from `restricted/`,
   `SMOKE_NO_BUILD=1 bash docker/smoke-test.sh` and `docker run --rm --cap-add NET_ADMIN dainty-lass-test:local`. This
   checks the images behave as restricted's compose expects. (Do not run restricted's `run-tests.sh` for this: its
   first step rebuilds restricted's images over the new ones.)
2. **New images, new compose, old harness.** From the root, `SMOKE_NO_BUILD=1 COMPOSE_FILE=compose.yaml bash
   restricted/docker/smoke-test.sh`, and likewise `restricted/docker/network/test.sh` and
   `restricted/docker/remote/test.sh`
   with their `*_NO_BUILD=1`. This needs restricted's scripts to take `COMPOSE_FILE` from the environment when it is
   set (Call D-8): a one-line change per script in `restricted/`, which is the reviewer's oracle change, not the
   implementer's.
3. **New everything.** From the root, `bash docker/run-tests.sh`.

All three pass on a Linux Docker host with `CAP_NET_ADMIN` and `sch_netem` (CI's `linux-host` runner or the
maintainer's machine). The smoke test plays no game: nothing here checks Wine or a game server, and the report must
say so.

---

## 9. What the integration pass changes (not the implementer)

- `.forgejo/workflows/ci.yml` (and the `.github` mirror): the Docker and Network families' path filters gain
  `docker/**`, `compose*.yaml`, `.env.example`, and the jobs run from the root (`working-directory` removed) with
  `bash docker/run-tests.sh` and `bash docker/network/test.sh`. While both trees exist, both run.
- `restricted/docs/hosting-docker.md` and `hosting.md`: `cd dainty-lass` instead of `cd dainty-lass/restricted`,
  the `.env` move, the plugin file name if Q-3 changes it, and the rollback section's status (Call D-9). The docs
  already describe a `rollback` profile with `backend-rollback` and `director-rollback` services that today's
  `compose.yaml` does not have: that row is stale either way.
- `restricted/AGENTS.md`'s Docker section and the root `AGENTS.md`'s Docker paragraph.
- The migration guide's section 2 lines for the new files.
- Deleting `restricted/docker/`, `restricted/compose*.yaml` and `restricted/.env.example` once the oracle runs pass
  and the maintainer agrees (Call D-9 decides whether that waits for RB15).

---

## 10. Calls and open questions

**Calls made here** (the reviewer checks them; the maintainer may overturn any):

- **D-1** Root `compose.yaml` and `docker/`, context `.`; hosts run Compose from the root.
- **D-2** A Dockerfile-specific ignore file, `docker/Dockerfile.dockerignore`.
- **D-3** The project-root marker is the root's MIT `Cargo.toml` plus `modules/rustic-isles/`, not a fake
  `release.json`.
- **D-4** Licence and notice files for the AGPL data go under `data/` beside it, not at the image root.
- **D-5** `DAINTY_SOURCE_COMMIT` stays.
- **D-6** No C++ or loader sources in the MIT image; `--skip-install` keeps setup from asking for them.
- **D-7** The MIT build reads nothing from `restricted/` but the data files of section 2.4.
- **D-8** Every harness takes `COMPOSE_FILE` from the environment when set, else `compose.yaml`.
- **D-9** **[RB15]** The MIT Dockerfile has no `node-build`, `mongo-tools` or `server-rollback` stage and the
  director entry point no `DAINTY_ROLLBACK`. The `mongo` profile, the `mongo-data` mounts and smoke steps 2-4 and the
  migrated-account checks **are** ported (they test the MIT host kit's `db migrate`/`db verify`, which a host still
  needs until RB15), and each is marked in the files so RB15 deletes them in one sweep. A host needing the Node
  rollback before RB15 uses restricted's images, which therefore stay until RB15 or until the maintainer drops the
  rollback.
- **D-10** The version data in the image is root-owned and read-only.

**Open questions for the maintainer:**

- **Q-1 Image licence labels.** The binaries are MIT, the version data, example game data and store captures are
  AGPL. Assumed: `org.opencontainers.image.licenses="MIT AND AGPL-3.0-only"` on `server` (which carries the data),
  `"MIT"` on `front`, `netem` and `test`; the data's `LICENSE`, `NOTICE.md` and `ADDITIONAL_TERMS.md` in
  `/opt/dainty-lass/data/`.
- **Q-2 The example game data and store captures.** Are `*.example.json` and `store-captures.json` facts about the
  game (then MIT-shippable) or Paradox's? Assumed AGPL data, copied from where lane A or the repository keeps them,
  like the version data. Port plan item 9 may make the version data generated on each host; then section 2.4 loses
  that row.
- **Q-3 The module library's file name.** The release and `settings.toml` pins today say `librustic_isles_plugin.so`;
  the root's crate builds `librustic_isles_1_14_7_plugin.so`. A rename breaks every existing pin. Assumed: the image
  ships whatever name lane A's release ships; if lane A renames, ship both (a copy under the old name) for one
  release.
- **Q-4 The in-image suite list.** Assumed: today's eleven packages plus lane A's moved server package. Should it be
  the whole root workspace minus the launcher crates instead?
- **Q-5 Capabilities of the server-image services.** `backend`, `director`, `host`, `configure`, `remote-agent` keep
  Docker's default set and no `no-new-privileges`. Assumed: unchanged (Wine and the game servers are untested with
  less). Hardening them is a follow-up with a hands-on Wine check.
- **Q-6 The smoke test after RB15.** Steps 2-4, 8's migrated-account check and 9 go; step 8's register check then
  needs an invite, assumed from `host invite --json` before the request (or `registration = "OPEN"` in step 5's
  `settings.toml`).
- **Q-7 The region ping's test.** The network harness runs `cargo test -p dainty-launcher --lib --locked -- --ignored
  --nocapture the_region_order_follows`, a test in restricted's Tauri crate, which lane C is rewriting. Assumed: the
  root harness runs the same-named ignored test in the MIT launcher crate once lane C lands, and until then runs it
  from `restricted/` (`cargo test --manifest-path restricted/Cargo.toml …`), so CI's `require` keeps working. Lane C
  should keep the test name and its `RR_HARNESS_*` variables.
- **Q-8 Where lane A puts the shipped version data in the repository**, which `rust-sources` and the `server` stage
  copy from, and the `settings.toml` field name (port plan item 1's maintainer decision). The image relies only on the
  default path `data/1.14.7/version_data.json`.
