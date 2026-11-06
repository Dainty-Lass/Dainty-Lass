<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Split roles: the metagame and the game servers on two machines

One DaintyLass server can run as two halves, each its own `reforged-host start` (one process, so one container):

| Half | `split.role` | Runs | Its other port |
| --- | --- | --- | --- |
| Metagame | `metagame` | the gateway, the backend (port 61000), content, the admin API | 61001, the Director's, is the tunnel to the game half |
| Game servers | `gameservers` | the gateway, the Director (port 61001) and every game server it launches | 61000, the backend's, is the tunnel to the metagame half |

Players connect to the metagame half as they connect to any server: its invite, its gateway, its certificate. A hunt
starts on the game half, and the backend hands the player that half's game address and port (`game_address` there).

Left out, `[split]` changes nothing: one server runs both, as before.

## How the halves talk

Both halves are nodes of one `[network]` (DD16), so they already share a node link: mutual TLS under the cluster CA,
on each gateway's port (`docs/security/network.md`). The split adds one node-link service, `Service::Tunnel`
(`crates/network/src/tunnel.rs`):

- On the metagame half, `127.0.0.1:61001` is a listener that carries each request to the game half, which replays it
  on its own `127.0.0.1:61001`, the Director. The backend's `DEPLOYSERVER_URL` is unchanged.
- On the game half, `127.0.0.1:61000` is a listener that carries each request to the metagame half's backend. The game
  servers' `REFORGED_BACKEND_URL` is unchanged, and so is their game server key: it travels inside the node link, never
  through a public gateway route, which still refuses it.

Nothing but the gateways listens off loopback. The Director's port carries only the Director's four routes; a call
from any node but the configured peer is refused. `docs/security/tunnel.md` is the checklist.

## Setting it up

1. Make the cluster CA and a node certificate for each half, as for any network
   ([hosting.md, "A network of servers"](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/restricted/docs/hosting.md#a-network-of-servers)).
2. On each half, list the other in `[[network.nodes]]` and name it in `[split]`:

   ```toml
   # The metagame half
   mode = "public"
   host = "play.example.org"

   [network]
   node = "meta-1"
   region = "oregon"

   [[network.nodes]]
   id = "games-1"
   region = "oregon"
   host = "games.internal"
   port = 443

   [split]
   role = "metagame"
   peer = "games-1"
   ```

   ```toml
   # The game servers half
   mode = "public"
   game_address = "203.0.113.7"   # what players' clients connect their hunts to

   [network]
   node = "games-1"
   region = "oregon"

   [[network.nodes]]
   id = "meta-1"
   region = "oregon"
   host = "meta.internal"
   port = 443

   [split]
   role = "gameservers"
   peer = "meta-1"
   open_game_ports = true
   ```

3. `reforged-host setup` (or `update`) on each half writes `RUSTIC_ISLES_PORT_SERVICES` into `backend.env`, which keeps
   the module from binding the other half's port. Then `reforged-host start` on each.

Both halves use the same `secrets.env`'s game server key (`GAMESERVER_API_KEY`): the game half's game servers present it
to the metagame half's backend. Copy the metagame half's `secrets.env` to the game half before its first `setup`.

### `open_game_ports`

A public server opens its game ports only to players whose heartbeat reached its gateway (`docs/security/allowlist.md`).
In a split, heartbeats reach the metagame half, so the game half's allowlist would never open a port. Until the
allowlist feed crosses the tunnel, a public game half runs with its game ports open to everyone, and `host.toml` must
say so: `split.open_game_ports = true`, or the file is refused. Put the game half behind a firewall of your own that
admits only your players' networks if that matters to you.

## Docker

Each half is the `server` image running `reforged-host start`, with its own `workspace` volume and `host.toml`. A sketch
(not yet run against the real client; the per-service `compose.yaml` in `restricted/` is unchanged):

```yaml
services:
  metagame:
    image: reforged-revival-server:local
    command: ["reforged-host", "start"]
    volumes: ["meta-workspace:/opt/reforged/.workspace"]
    ports: ["443:443"]           # the public gateway; the game half's node link arrives here too
    networks: [split]
  gameservers:
    image: reforged-revival-server:local
    command: ["reforged-host", "start"]
    volumes: ["games-workspace:/opt/reforged/.workspace", "./game:/game"]
    ports: ["8770-8777:8770-8777/udp"]   # the game ports only: its gateway is reached on the private network
    networks: [split]
networks:
  split: {}
volumes:
  meta-workspace: {}
  games-workspace: {}
```

`[[network.nodes]] host` is the other service's name on the `split` network (`metagame`, `gameservers`).

## Limits

- A tunnelled body is at most 512 KiB either way (the node link's 1 MiB message, base64 included).
- A tunnelled call waits up to 310 s, the Director's own 300 s wait for a hunt's game server and the link.
- The game half still starts the whole Rustic Isles module, its database included; only its backend port is not bound.
- One game half per metagame half. Several game halves need the backend to pick one, which is the ring's job (DN8).
