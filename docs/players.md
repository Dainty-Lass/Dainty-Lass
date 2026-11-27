<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Playing on a Dainty Lass server

Dainty Lass servers run Dauntless 1.14.7 after the official servers closed. To play on one you need three things:
your own copy of the game at version 1.14.7, the Dainty Lass Launcher, and an invite from the server's host. The
launcher does the rest: it checks the game, installs the server's runtime, keeps your account key and starts the game
signed in.

This page is for players. Hosts start at [hosting.md](hosting.md) and [sdk-and-game-data.md](sdk-and-game-data.md).

## 1. Install the launcher

The launcher is built for Windows and for 64-bit Linux:

| System | File | Notes |
| --- | --- | --- |
| Windows 10 or 11, 64-bit | `DaintyLassLauncher_<version>_x64-setup.exe` | Installs for your user only; no administrator rights needed. It brings Microsoft's WebView2 installer along and runs it if your PC lacks WebView2 |
| Debian, Ubuntu and their relatives | `DaintyLassLauncher_<version>_amd64.deb` | Needs `libwebkit2gtk-4.1-0`; recommends `gnome-keyring` and `pkexec` |
| Fedora and other rpm systems | `DaintyLassLauncher_<version>_x86_64.rpm` | Needs `webkit2gtk4.1` |
| Any other Linux, SteamOS | `DaintyLassLauncher_<version>_amd64.AppImage` | Make it executable and run it |

The packages are built on Ubuntu 22.04, so they run on that release and anything newer. Get the file from the link
your host gives you. A release's `SHA256SUMS.txt` lists every file's hash, so you can check the download before you run
it.

> **Maintainer:** where players download the launcher is not settled in code yet. The release workflows only build
> the files; publishing them waits for DX5 (`launcher-release.md`). Add the download link here once it exists.

Once installed, the launcher updates itself. It looks for a new version when it starts and every hour after that, and
shows **Restart to update** when one is ready. A server can also ask for a minimum launcher version: the launcher then
updates straight away, or, if it can't, tells you which version to install by hand.

## 2. Get the game

You need Dauntless **1.14.7**, build `1.14.7.647472`, and nothing else: no other version, no modified copy. The
launcher checks the game's executable against the hash of that build before every PLAY.

The first time you join a server, the launcher asks where your game is (**Find your Dauntless**). You have two ways:

- **Use an install you already have.** On Windows the launcher lists the installs the Epic Games launcher,
  Legendary and Heroic know about. Pick one, or choose the folder yourself: it is the folder that contains `Archon`.
- **Download it from your Epic account.** Open **Game management**, sign in with the Epic account that owns
  Dauntless, and install 1.14.7. The launcher first checks that your library has Dauntless; an account without it
  gets nothing. The download comes from Epic's own servers, and every file is checked against the archived manifest
  of that build. You can pause it, and a stopped download carries on from where it left off. The Epic sign-in stays
  on your PC and is never sent to a Dainty Lass server.

Keep that folder on 1.14.7. If another program updates or "repairs" it to a different build, the launcher refuses it
with "That folder holds 1.14.7, but not the build this launcher plays". **Verify game files** in Preferences checks
the files again.

A Dauntless Revived copy (version 1.4.4) can't play here, and the launcher says so.

## 3. Join with an invite

Your host sends you an invite. It looks like `dainty://join?...` or `dauntless://join?...`. Click it, or copy it and
paste it on the launcher's Play page, then press **Join**.

An invite names the server and how to reach it:

- **A private server** sits on the host's Tailscale network. Install [Tailscale](https://tailscale.com/download),
  sign in, and accept the share link your host sent (the launcher offers it when the invite has one). If the launcher
  says "Tailscale is not installed", that is the step left.
- **A public server** is on the internet. Its invite carries the fingerprint of the server's certificate. The launcher
  checks it before it sends anything. If the certificate doesn't match, it stops and warns you; only go on if your
  host told you they moved or rebuilt the server.

Invites from Dauntless Revived (`dauntless-revived://`) are refused: this launcher doesn't play those servers.

## 4. Your account key

When you join a server for the first time, you pick a player name (3 to 16 letters, digits or underscores) and press
**Create account**. The server gives your launcher an **account key**. It starts with `DLK_`, and it is what proves
the account is yours. There is no password.

- **One key per server.** Each server you join has its own key, kept for that server and its certificate only. The
  launcher never sends a key anywhere else.
- **It is kept protected on this PC.** On Windows the key file is sealed with your Windows account (DPAPI). On Linux
  it is encrypted with a key kept in your desktop's keyring (the Secret Service, which GNOME Keyring
  provides). Without one the launcher can't store the key; install `gnome-keyring` or your desktop's equivalent.
- **Back it up.** Press **Save backup** (on the Play page, or **Back up account key** in Preferences). The backup is a
  text file with the key in it. Treat it like a password and never post it. The launcher also keeps a recovery copy
  in your Documents folder, under `DaintyLass\Account Keys`.
- **Another PC.** Join the same server with the launcher there, choose **Sign in with an existing account key**, and
  paste the key or **Import key file** with your backup.
- **Sign in with Discord.** Some servers let you link an account (Discord, or another sign-in the host set up). Once
  linked, signing in gets the account back on any PC without the key file. The sign-in comes back to the launcher on
  port 61190, so another program must not hold it.

Signing out, or removing a server, deletes its key from this PC. Without a backup that account can't be recovered.

The game itself never sees your key where the server supports game tickets: the launcher asks the server for a
one-time ticket and starts the game with that. The key is hidden in the launcher's logs and in the game log it shows.

## 5. PLAY

The first time, the launcher asks whether to trust the server's **game runtime**: the files that make the game talk
to that server instead of the closed official ones. They load inside your game, so the launcher shows them and asks
first. Press **Trust**, then **Install**. When the host publishes a new runtime, you are asked again.

Then press **Play**. Before the game starts, the launcher:

1. checks your account key with the server (a banned account stops here);
2. puts you in line if the server is full ("You're number N in line"), and starts the game when a place opens;
3. checks the game folder and its executable;
4. refuses to start if old mod files sit beside the game (a stray `winmm.dll`, a `mystic_loader.ini`, a host
   tool's DLL), since they would load a second runtime. Remove them, then press Play again;
5. checks the runtime against the files you trusted;
6. starts the game paused, loads the runtime into it, and only then lets it run. If anything goes wrong in between,
   the game is closed rather than started half-ready.

Keep the launcher open while you play unless it tells you otherwise. With an older runtime, the launcher relays the
game's traffic to the server through `127.0.0.1:61100`, so nothing else on your PC may use port 61100.

Preferences has the game's graphics preset, windowed mode and in-game language, and a switch to show the game's own
console window. With it off, the console's lines appear under **Game log**.

## 6. Several servers

The **Servers** page lists every server you have joined, each with its own name and key. **Switch** makes one current;
**Remove** forgets it and deletes its key from this PC (back it up first if you want to come back). **Join another
server** takes a new invite.

## 7. Linux and Proton

The game is a Windows program, so on Linux the launcher runs it through Proton or Wine.

- **Proton.** The launcher looks for Proton in Steam's folders (`~/.local/share/Steam`, `~/.steam/root`,
  `~/.steam/steam`, and the Flatpak Steam's), in `steamapps/common` and `compatibilitytools.d`, and takes the newest.
  GE-Proton counts.
- **Wine.** Without Proton it uses Wine from your `PATH`. With neither, it offers to install Wine through your
  package manager, asking for your password with `pkexec`.
- **Choosing one yourself.** Set `DAUNTLESS_REVIVED_PROTON` to a Proton's `proton` script, or `DAUNTLESS_REVIVED_WINE`
  to a `wine` binary, before you start the launcher. `DAUNTLESS_REVIVED_AUTO_INSTALL=0` turns off installing Wine.
- **The prefix** is the launcher's own, under `~/.config/DaintyLass Launcher/compat/` (`proton/` or `wine/`). It
  is separate from Steam's.
- **How the runtime gets in.** By default the runtime loader (`winmm.dll`) sits beside the game and loads it. In
  Preferences, **Load the runtime with the inject helper** switches to a small Windows helper, `dainty-inject.exe`,
  that the Linux packages carry; it loads the runtime the way the Windows launcher does. The launcher installs what
  each way needs before the next launch. If PLAY says the inject helper is missing, reinstall the launcher or turn the
  switch off.
- **The game folder.** On Linux the launcher doesn't read other stores' lists, so choose the folder that contains
  `Archon`, or download the game in **Game management**.

> **Maintainer:** the Proton path has not been tried end to end with the game yet (MIGRATION_PLAN RA10, and the
> hands-on checks in [launcher-injection.md](launcher-injection.md)). Revisit this section once it has.

## 8. If something is wrong

The launcher's messages say what went wrong and what to do. The most common ones:

| What you see | What to do |
| --- | --- |
| The server is not answering | It may be offline. If your host moved it, ask for a new invite |
| The server's name could not be found | Check the invite, or try another DNS server (such as 1.1.1.1) in your network settings |
| The secure connection failed | A VPN, proxy or antivirus that inspects traffic can cause this. Try without it |
| The certificate does not match the invite | Stop, and ask your host. Only accept a new certificate if they changed it |
| Tailscale is not installed | Install Tailscale, sign in and accept the host's share |
| That name is taken / not valid | Pick another: 3 to 16 letters, digits or underscores |
| The server does not know that account key | The key belongs to another server, or the account was removed. Check with your host |
| This account is banned on this server | Talk to the host |
| Old mod files sit next to the game | Remove the files the message names from the game's `Archon\Binaries\Win64` folder |
| That folder holds another version / build | Point the launcher at a 1.14.7 install, or download it in Game management |
| That folder has Dauntless files but no Archon folder | Pick the folder one level up, the one that contains `Archon` |
| The game folder's path is too long | Move the game to a shorter path |
| Port 61100 is in use by another program | Close the program using it, or another launcher left running |
| This server offers no runtime | The host hasn't published one yet. Ask them |
| The runtime is in use. Close the game first | Quit the game, then install again |
| Proton or Wine was not found | Install Steam with Proton, or Wine, or set `DAUNTLESS_REVIVED_PROTON` (section 7) |
| Your account key could not be stored on this PC | On Linux, install and unlock a keyring (`gnome-keyring`) |
| This launcher is too old | Let it update, or install the version it names |

When a message doesn't help, look at the logs: **Open logs folder** in Preferences. The launcher keeps them in
`%APPDATA%\DaintyLass Launcher\logs` on Windows and `~/.config/DaintyLass Launcher/logs` on Linux. Account keys are
removed from them, so you can send them to your host. **Game log** shows what the game's console printed, and **Copy
the newest lines** puts them on your clipboard.

If the game crashes, the launcher can send the crash files and its log to the server you play on, so the host can see
what went wrong. Preferences sets whether it never does, asks each time, or always does; your account key is removed
first.

**Hide server details while streaming** in Preferences hides addresses, paths and player names on screen.

Bugs in the launcher itself go to the project's issue tracker: <https://github.com/Dainty-Lass/Dainty-Lass/issues>.
