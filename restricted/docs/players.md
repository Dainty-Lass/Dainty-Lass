# Playing on a DaintyLass server

You need **Dauntless 1.14.7** on your PC, the **DaintyLass launcher**, and an **invite** from the server's host.
The launcher speaks English and Brazilian Portuguese.

## 1. The game

The launcher never downloads the game: it uses your own copy. Epic only serves the last version of Dauntless
today, so 1.14.7 is installed from your Epic library with [Legendary](https://github.com/legendary-gl/legendary)
and the archived 1.14.7 manifest, which only works for an account that had Dauntless before it left the store.
Mystic Paradox's guide has every step and check:
[Installing Dauntless 1.14.7](https://github.com/pranav158/Mystic-Paradox/blob/main/docs/INSTALLING_DAUNTLESS.md).
Legendary and the manifest are third-party tools; use them at your own risk. Installing from your library
inside the launcher is planned ([dainty-lass-plan.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/dainty-lass-plan.md), DD22).

## 2. The launcher

Install the launcher from a release and start it. Then:

1. **Paste the invite** your host sent (`reforged-revival://join?…` or `dauntless://join?…`). The launcher
   checks the server; a public server's invite carries the fingerprint of its certificate, and the launcher
   talks to that server only.
2. **Pick a name** (3 to 16 letters, digits and underscores). The server gives the launcher an **account
   key**, which is your login. It is kept in your OS's keystore; back it up once from the launcher's menu if
   you want to move PCs. Nobody can give you a new one.
3. **Point it at Dauntless.** The launcher finds Epic, Legendary and Heroic installs on its own; otherwise
   choose the folder with `Archon` in it. It checks the executable's hash: only 1.14.7 (`rel-1.14.7-Archon`)
   works.
4. **Accept the host's runtime.** The host built `Rustic-Isles.dll` and `winmm.dll` from their own game; the
   launcher shows their sizes and hashes and asks you once. They are native code the host made, so accept
   them only from a host you trust. The launcher checks the files before every launch and asks again if they
   change.
5. **PLAY.** The launcher starts the game with your key, carries its connection to the server through a relay
   on your PC, and shows the server's status: who is online, the hunts running, the host's news.

Settings: the language, the game's language, windowed mode, a graphics preset, and the game folder.

## Several servers

Each invite you paste is kept; switch between servers from the list. Your account key is per server. A host
who moves their server sends a new invite, and the launcher offers to carry your account along when the
certificate changed.

## Dauntless Revived servers

DaintyLass plays Dauntless 1.14.7 only. A `dauntless-revived://` invite (a Dauntless Revived 1.4.4 server) is
refused, and a 1.4.4 server saved by an older launcher stays in the list, greyed out, with its key kept.

## If something is wrong

- "This invite is for another kind of server": a `dauntless-revived://` invite, for a Dauntless Revived 1.4.4
  server.
- "Not reachable": a private server needs Tailscale running on both PCs and the host's PC shared with you.
- "Unverified game": the folder is not 1.14.7, or the install is incomplete; verify it with Legendary.
- Logs: the launcher's menu opens its log folder; send the log to your host.
