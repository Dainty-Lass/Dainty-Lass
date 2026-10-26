# Hosting a server on Linux

The same server as on Windows ([hosting.md](hosting.md)), with the game servers under **Wine** or **Proton** on
a virtual display. A public server on a VPS (Debian 13 is what this is tested on) is the main use. The
containers in [hosting-docker.md](hosting-docker.md) are this page with the packages baked in.

> [!WARNING]
> Experimental. The Windows game server is run through Wine on a server that has no GPU; it works in the
> maintainer's setup, and every host's VPS is different. Report what you find.

## What you need on the server

- Debian 13 (or any distribution with the same packages): `wine wine64 xvfb xauth nftables`, **Node.js 22**
  and **MongoDB** (`mongod`), or your own MongoDB elsewhere.
- Your **Dauntless 1.14.7** folder copied to the server (about 15 GB), writable by the user the server runs
  as: the game servers write logs and a switch file next to the executable.
- The **runtime** (`MysticParadox.dll`, `winmm.dll`) and the **game data**, made on a Windows PC with your own
  game ([sdk-and-game-data.md](sdk-and-game-data.md)) and brought over: the DLLs as a folder, the game data as
  a pack (`reforged-host gamedata --export`).
- The **server release** (`DaintyLassServer-<version>-linux-x64.tar.gz`).
- Public: TCP 443 and UDP 8770-8777 open at the provider's firewall.

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
