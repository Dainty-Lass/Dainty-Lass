<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Hosting a server on Linux

A Linux server runs the same `dainty-host` as a Windows one, with the game servers (a Windows program) under Wine or
Proton. This page covers what differs on Linux. Read [hosting.md](hosting.md) first: setup, invites, the two modes,
the ports and every setting are described there. For Docker, see [hosting-docker.md](hosting-docker.md).

## What you need on the server

- A 64-bit (x86_64) Linux with systemd. The release is `DaintyLassServer-<version>-linux-x64.tar.gz`.
- An ordinary user that owns the server folder. `setup` refuses to run as root or under sudo, and the systemd units it
  writes refuse to run as root.
- Wine (`wine` and `wine64`; `dainty-host` takes `wine64` first), or a Proton or GE-Proton build.
- `xvfb-run` and `xauth` (on Debian and Ubuntu, the `xvfb` and `xauth` packages) unless the machine has a display.
- For a **public** server: nftables (`nft`, in `/usr/sbin` or `/sbin`), which the allowlist uses.
- For a **private** server: Tailscale, running and signed in.
- Optionally ufw, which the firewall script setup writes is for.

The game data, the SDK and a runtime you build yourself all come from a Windows PC with the game
([sdk-and-game-data.md](sdk-and-game-data.md)). The release ships the runtime prebuilt, so most hosts need only the
game data from Windows. Make it there, then move it as a pack:

```sh
# on the Windows PC
bin\dainty-host gamedata --export gamedata.zip
# on the Linux server
bin/dainty-host gamedata --import gamedata.zip
```

## Getting the game from Epic

`install-game` works on Linux as on Windows: it prints Epic's login link, you open it in a browser anywhere, sign in,
and paste the code Epic shows. It installs Dauntless 1.14.7 into `.workspace/game` unless `--dir` names another
folder (a full path), and points `game_dir` at it.

```sh
bin/dainty-host install-game
bin/dainty-host install-game --dir /srv/dauntless --code-file /tmp/epic-code.txt
```

A copy of the game from elsewhere works too: copy the whole 1.14.7 folder (the one with `Archon/`) to the server and
pass it to setup with `--game`.

## Setting up

Unpack the release as the user that will run the server, and run setup from the server folder:

```sh
tar -xzf DaintyLassServer-1.0.0-linux-x64.tar.gz
cd dainty-lass-server-1.0.0
bin/dainty-host setup --mode public --host play.example.org --game-address 203.0.113.7 \
  --name "Friday Hunts" --game /srv/dauntless
```

On Linux setup also does the following:

- writes `.workspace/bin/dainty-gameserver`, the wrapper the Director starts each game server with;
- makes the Wine prefix in `.workspace/wine` with `wineboot --init` (Proton keeps its own in `.workspace/proton`);
- writes the systemd units and their install script into `.workspace/systemd/`;
- writes `.workspace/config/firewall.sh`, the ufw rules for the server's ports.

The Linux options, which `settings.toml` keeps under `[linux]`:

| Option | Key | What it does |
| --- | --- | --- |
| `--wine <path or name>` | `linux.wine` | The wine program. A name is looked up on PATH now; empty finds `wine64` or `wine` on PATH each time a game server starts |
| `--proton <path>`, `--no-proton` | `linux.proton` | Runs the game servers with this Proton `proton` script instead of Wine |
| `--xvfb`, `--no-xvfb` | `linux.xvfb` | Runs the game servers under `xvfb-run` when there is no `DISPLAY` (on by default) |
| `--firewall-interface <name>`, `--no-firewall-interface` | `linux.firewall_interface` | Public: the network interface the allowlist filters, like `eth0`. Left out: every interface |

`linux.wine` and `linux.proton` in `settings.toml` must be full paths. Run `bin/dainty-host doctor` after setup: on
Linux it also checks Wine or Proton, the prefix, `xvfb-run` and `xauth`, the inject helper, the game server wrapper,
nftables and whether the gateway can bind its port.

Importing Ramsgate Reborn saves ([legacy-ramsgate.md](legacy-ramsgate.md)) takes the old MongoDB's URI on stdin:
`bin/dainty-host setup --legacy-mongo-uri < rr-uri.txt`.

## Wine and Proton

The Director starts each game server through `.workspace/bin/dainty-gameserver`, which runs `dainty-host gameserver`.
That runs `Dauntless-Win64-Shipping.exe` from `game_dir`:

- **under Wine** with `WINEPREFIX=.workspace/wine` and `WINEDEBUG=-all` (unless you set one), and Wine's own `winmm`,
  so a loader left beside the game's executable can't load a second runtime;
- **under Proton** as `proton run`, with `STEAM_COMPAT_DATA_PATH=.workspace/proton`, `STEAM_COMPAT_APP_ID=0` and
  `PROTON_LOG=0`. `STEAM_COMPAT_CLIENT_INSTALL_PATH` is taken from the Proton script's location when you don't set it;
- **under `xvfb-run`** when there is no `DISPLAY` and `linux.xvfb` is on. `DAINTY_XVFB_RUN` names another
  `xvfb-run`; otherwise it is found on PATH.

The game runtime is loaded into each game server before it runs, by the inject helper `dainty-inject.exe` inside the
prefix. The helper sits beside `dainty-host`; `doctor` fails when it is missing, since no game server could load its
runtime. A Linux game server gets 90 seconds to report ready, against 30 on Windows
(`[director] gameserver_ready_timeout_ms`).

The first game server after a new prefix can take a while, since Wine finishes setting the prefix up. If a game
server fails to start, look in `.workspace/logs/gameservers/` and `.workspace/logs/setup.log`; `wine.log` there holds
what `start` logged when it made the prefix.

## Running it

### With systemd

Setup writes the units into `.workspace/systemd/`, for the user that ran it and the `dainty-host` it found in the
server folder. Install and start them once:

```sh
sudo .workspace/systemd/install.sh
```

The script copies the units into `/etc/systemd/system/`, reloads systemd and enables and starts each one. Setup
rewrites the units every time it runs, but the installed copies change only when you run `install.sh` again.

| Unit | What it runs |
| --- | --- |
| `dainty-lass.service` | `dainty-host start` in the server folder, as your user. Restarts 5 s after a failure. A public server whose gateway port is below 1024 gets `CAP_NET_BIND_SERVICE`, and nothing else |
| `dainty-lass-allowlist.service` | Public servers only: `dainty-host allowlist` with `allowlist.env`, as your user with `CAP_NET_ADMIN` alone, so it can change its nftables table. Restarts 2 s after a failure |

On a public server the main unit sets `DAINTY_ALLOWLIST=external`, so `start` does not run the allowlist itself but
waits up to 30 seconds for the allowlist unit to answer on `127.0.0.1:41005`, and stops with a hint when it doesn't.

```sh
systemctl status dainty-lass
journalctl -u dainty-lass -f
sudo systemctl restart dainty-lass
```

The units pass the server no environment of their own beyond `DAINTY_ALLOWLIST`. To override a setting from the
environment ([hosting.md](hosting.md#environment-variables-override-settingstoml)), add it with a drop-in:
`sudo systemctl edit dainty-lass`, then `[Service]` and `Environment=DAINTY_DIRECTOR_LOG_LEVEL=debug`. Prefer
`settings.toml`, which survives a reinstall.

### By hand

`bin/dainty-host start` and `bin/dainty-host serve` work as on Windows, which is handy for a private server or a first
test. Before it starts, `start` writes the game server wrapper if it is missing and makes the Wine prefix if it is not
ready.

A public server is meant to run under the units. Binding port 443 needs `CAP_NET_BIND_SERVICE` (or root, which
`dainty-host` refuses for setup); without the unit, choose a higher port with `setup --public-port 4443` and forward
443 to it if you like. The allowlist needs `CAP_NET_ADMIN` to change nftables, which the allowlist unit has.

## The firewall

### nftables, the allowlist

On a public server the allowlist keeps the game's UDP ports closed to everyone but the players who signed in. The
gateway reports each signed-in player's address to it, and it lets that address in for `[allowlist] ttl_seconds`
(600 by default), renewed while the player stays signed in.

It owns one nftables table, `inet dainty_lass` (`[allowlist] nft_table`), and nothing else: it never touches your
other rules or tables. The table is replaced whole in one nft transaction, so the game ports are never open to
everyone and the table is never missing, even for a moment. It runs `nft` at a fixed path with fixed arguments, never
through a shell or PATH. `linux.firewall_interface` limits the filter to one interface.

It keeps its set in `.workspace/config/allowlist-state.json`, so a restart restores who may play but never for longer
than one TTL, and logs every change to `.workspace/logs/allowlist-audit.log`.

### ufw

`.workspace/config/firewall.sh` holds the ufw rules for the server's ports. For a public server it allows the
gateway's TCP port and the game's UDP range. For a private one it allows the same, but only in on `tailscale0`. Run it
once if you use ufw:

```sh
sudo .workspace/config/firewall.sh
```

If you use another firewall, open the same ports in it. A cloud provider's firewall needs them too.

## Managing several installs

A Linux install can be managed from a controller elsewhere, over node links (`[remote]` in
[hosting.md](hosting.md#remote-managing-several-installs), and [remote-management.md](remote-management.md)):

```sh
# on the controller
bin/dainty-host remote init-controller
bin/dainty-host remote invite --node linux-1 --label "Linux box"
# on the Linux install
bin/dainty-host remote enroll "<the invite>"
```

Node links run over the gateway's TLS port, so setup writes an install's node settings only on a public server.

## Updating

```sh
bin/dainty-host update
sudo systemctl restart dainty-lass
```

`update` installs the new release in the server folder and reruns setup, which rewrites the units in
`.workspace/systemd/`. Run `sudo .workspace/systemd/install.sh` again when a release changes them, and restart the
allowlist unit too on a public server. [hosting.md](hosting.md#updating) has the checklist for a live server, and what
setup does to an older install ([Upgrading from an older install](hosting.md#upgrading-from-an-older-install)).
