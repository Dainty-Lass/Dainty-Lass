# Hosting a server on Linux

The same server as on Windows ([hosting.md](hosting.md)), with the game servers under **Wine** or **Proton** on
a virtual display. A public server on a VPS (Debian 13 is what this is tested on) is the main use. The
containers in [hosting-docker.md](hosting-docker.md) are this page with the packages baked in.

> [!WARNING]
> Experimental. The Windows game server is run through Wine on a server that has no GPU; it works in the
> maintainer's setup, and every host's VPS is different. Report what you find.

## What you need on the server

- Debian 13 (or any distribution with the same packages): `wine wine64 xvfb xauth nftables`. No Node.js or MongoDB is
  needed on the default path: the 1.14.7 backend, Director, gateway, content server, allowlist helper and admin API
  are compiled into `reforged-host` and store player data in `.workspace/config/dainty-lass.db` (SQLite) or Postgres
  (`[database]`).
- Your **Dauntless 1.14.7** folder (about 15 GB), writable by the user the server runs as: the game servers write
  logs and a switch file next to the executable. Copy it from a PC that has it, or let the server fetch it from
  your own Epic library with `install-game` ([below](#getting-the-game-from-epic)).
- The **runtime** (`MysticParadox.dll`, `winmm.dll`) and the **game data**, made on a Windows PC with your own
  game ([sdk-and-game-data.md](sdk-and-game-data.md)) and brought over: the DLLs as a folder, the game data as
  a pack (`reforged-host gamedata --export`).
- The **server release** (`ReforgedRevivalServer-<version>-linux-x64.tar.gz`).
- Public: TCP 443 and UDP 8770-8777 open at the provider's firewall.

## Getting the game from Epic

A server with no Dauntless folder installs one from the owner's Epic library, as the launcher does for players.
As the same ordinary user, from the unpacked release (the server need not be set up yet):

```bash
bin/reforged-host install-game --dir /srv/dauntless          # 1.14.7, into /srv/dauntless/1.14.7
```

It prints Epic's login address. Open it in a browser on your own PC, sign in, and paste the `authorizationCode`
Epic shows when asked (or put it in a file, `--code-file /srv/code.txt`, and delete the file after). The session it
opens is kept in `.workspace/config/epic-session.env`, mode 0600; `install-game --sign-out` ends and deletes it, and
should be the last thing you run. A stopped install picks up where it left off when you run the command again.
`setup --game /srv/dauntless/1.14.7` is then the Dauntless folder; if `host.toml` exists already, `install-game`
sets `game_dir` and rewrites the env files itself. [hosting.md](hosting.md#getting-the-game-from-epic) has the rest.
Files are written as the server's user, so the game servers can write next to the executable.

## Setting up

As an ordinary user (`reforged`, say), from the unpacked release:

```bash
bin/reforged-host setup --mode public --host play.example.org --game-address 203.0.113.10 \
    --name "Friday Hunts" --game /srv/dauntless
bin/reforged-host publish-runtime --from /srv/import/runtime
bin/reforged-host gamedata --import /srv/import/gamedata.zip
bin/reforged-host doctor
```

`setup` makes the Wine prefix in `.workspace/wine` and writes two systemd units and a firewall script into
`.workspace/`:

| File | What |
| --- | --- |
| `reforged.service` | The server (`reforged-host start`), as your user, at boot |
| `reforged-allowlist.service` | Public: the allowlist helper with `CAP_NET_ADMIN` only, for its nftables table `reforged_revival` |
| `firewall.sh` | Lets a host firewall (ufw) pass TCP 443 and the game's UDP ports |
| `install-units.sh` | Installs and starts the units (needs root once) |

Run `sudo .workspace/install-units.sh`, then `systemctl status reforged`. Without systemd, `bin/reforged-host
start` runs it in the foreground like on Windows.

Linux options of `setup`: `--wine <path>` or `--proton <script>` (a Proton or GE-Proton script instead of
Wine; `--no-proton` goes back), `--xvfb` (the default) or `--no-xvfb`, and `--interface eth0` to make the
allowlist filter one interface only.

A server in a network of servers lists its peers under `[[network.nodes]]` and keeps its node certificate in
`.workspace/tls` ([hosting.md](hosting.md#a-network-of-servers)); the nodes meet on the gateway's TLS port, so no
firewall rule beyond the gateway's own is needed. Backend rule and request-budget settings in `[switches]`
(`reforge`, `mastery_persistence`, `bounty_persistence`, `cooldown_persistence`, `account_erasure`, `log_purge`,
`rate_limit`, `rate_limit_per_minute`, `gameserver_rate_limit_per_minute`, `host_rate_limit_per_minute`) are written by
`setup` into `backend.env` (`MYSTICPARADOX_RATE_LIMIT`, `MYSTICPARADOX_RATE_LIMIT_PER_MINUTE`,
`MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE`, `MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE` and the persistence
switches; see the settings table in [hosting.md](hosting.md#game-modules-and-plugins)).

Postgres instead of SQLite for the Rust services' database is `[database]` in `host.toml`
([hosting.md](hosting.md#the-database-engine)). On Debian or Ubuntu, with the role's password from `secrets.env`:

```bash
sudo apt install postgresql
pw=$(sed -n 's/^REFORGED_DB_PASSWORD=//p' .workspace/config/secrets.env)
sudo -u postgres psql -c "CREATE ROLE dainty LOGIN PASSWORD '$pw'" -c "CREATE DATABASE dainty OWNER dainty"
```

Debian's package listens on localhost only, which is what `setup` wants: the connection is not encrypted.

## Running it

```bash
bin/reforged-host invite --for "Alex"
journalctl -u reforged -f                      # the services
ls .workspace/logs/gameservers                  # the game servers
bin/reforged-host update                        # a newer release, then setup again
bin/reforged-host backup                        # mongodump into .workspace
```

## What to expect

- The first hunt after a start takes longer: Wine warms up. `--server-fps` and `--idle-fps` (`[performance]`
  in `host.toml`) keep idle game servers cheap.
- `doctor` checks Wine or Proton, `xvfb-run`, `xauth`, `nft`, the Wine prefix, the units and the firewall
  script, and says what each needs.
- The allowlist's nftables table is swapped in one transaction and filters only the game's UDP ports; it
  touches no other rule on the machine.

## Updating and rollback (`isles-db export-mongo` and `paradox`)

When `bin/reforged-host update` runs on a Linux host that previously ran the Node backend with MongoDB data, it runs
`isles-db migrate` and `isles-db verify` into the configured store (`.workspace/config/dainty-lass.db` or Postgres)
before installing the new release, refuses to switch if `verify` reports any difference, and leaves `.workspace/mongodb`
untouched ([hosting.md](hosting.md#live-server-cutover-checklist-backup-update-smoke-rollback)). For one release after
the cutover (until RB15), you can roll back to the Node backend and MongoDB by exporting the store back to MongoDB with
`isles-db export-mongo --from .workspace/config/dainty-lass.db --to mongodb://127.0.0.1:27017/reforged` and running a
`reforged-host` binary built with `--features paradox` ([hosting.md](hosting.md#rollback-to-the-node-backend-and-mongodb-paradox-feature)).
