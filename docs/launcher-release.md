<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Building and signing a launcher release

How `launcher-release.yml` builds Dainty Lass Launcher and Dainty Lass Manager, what the signing secrets do, and the
maintainer's checks before a release goes out (MIGRATION_PLAN RA14, decision RD7). There are two copies of the
workflow, `.forgejo/workflows/launcher-release.yml` and `.github/workflows/launcher-release.yml`; both cite this page.
Workflows belong to RR3 ([ci-runners.md](ci-runners.md) has the runners).

## What a run builds

A run is started by hand ("Run workflow"). Its version is `launcher/package.json`'s `version` (the app's
`tauri.conf.json` reads it from there), and a run that finds anything but `x.y.z` or `x.y.z-prerelease` stops.

| Job | Runner | What it makes |
| --- | --- | --- |
| Version | `docker` (GitHub: `ubuntu-latest`) | Reads and checks the version |
| Bundle (Windows NSIS) | `windows-msvc` (GitHub: `windows-latest`) | `DaintyLassLauncher_<version>_x64-setup.exe` and `DaintyLassManager_<version>_x64-setup.exe`, per-user NSIS installers |
| Bundle (Linux) | Ubuntu 22.04 (a container on Forgejo, the hosted image on GitHub) | `DaintyLass{Launcher,Manager}_<version>_amd64.AppImage`, `_amd64.deb` and `_x86_64.rpm` |
| Release files | `docker` (GitHub: `ubuntu-latest`) | `latest.json`, `latest-manager.json` and `SHA256SUMS.txt` over every file |

Both apps are built from `crates/dainty-launcher-app` with the Tauri CLI that `launcher/package.json` pins: the
launcher with `--features realms`, the manager with `--no-default-features --features server-gui` and its own config
(`tauri.server-gui.conf.json`). The manager also carries the 1.14.7 version data and facts beside its executable. The
Linux launcher (not the manager) carries `dainty-inject.exe`, the Proton inject helper, cross-built for
`x86_64-pc-windows-gnu` ([launcher-injection.md](launcher-injection.md)); the job fails a deb or rpm without it, and
one whose desktop entry lacks the `dauntless` invite scheme.

No build cache is used: every file comes from the run's commit alone.

The result is the `launcher-release` artifact, kept 14 days. GitHub's run also makes `launcher-sbom`, a CycloneDX
SBOM of the launcher's Rust and JavaScript dependencies. **Nothing is published yet.** On Forgejo the workflow is a
build only. On GitHub a `publish` job is written (build provenance and SBOM attestations, then the files uploaded to
the `launcher-updates` release with `--clobber`), but the Version job never turns it on until DX5.

## The updater feeds

Installed apps update themselves with `tauri-plugin-updater`, which accepts an update only with a valid ed25519
signature from the updater key (RD7). Each app has its own feed on the rolling `launcher-updates` release (decision
L7):

- the launcher reads `https://github.com/Dainty-Lass/Dainty-Lass/releases/download/launcher-updates/latest.json`;
- the manager reads `.../launcher-updates/latest-manager.json`.

The Release files job writes the two feeds only when every bundle has its `.sig`. Each lists `version`, `notes`,
`pub_date` and, for `windows-x86_64-nsis`, `linux-x86_64-appimage`, `linux-x86_64-deb` and `linux-x86_64-rpm`, the
signature and the URL of the file beside the feed. `windows-x86_64` and `linux-x86_64` repeat the NSIS and AppImage
entries for builds that ask without a bundle type. With no signatures at all it writes no feed and says so; with some
missing it fails. On Windows the update installs in `passive` mode.

The launcher checks the feed at start and every hour. It updates itself only as a bundle (any Windows build, or a
deb, rpm or AppImage) whose config holds a public key; otherwise it logs `self-update is off` and never checks.

The two apps' identifiers (`io.github.freezamotoqueiro.dauntlessreforgedrevival` and
`...dauntlessreforgedrevival.server`) and their product and binary names must not change: NSIS installs per user under
the identifier, so an install
updates in place only while they hold (`crates/dainty-launcher-app/tests/bundles.rs` checks them).

## Keys and secrets

Every secret is optional. Unset, that signature is skipped and the run still builds; the files are just unsigned.

| Secret or variable | What it is | Used for |
| --- | --- | --- |
| `LAUNCHER_UPDATER_KEY` | The updater's ed25519 private key, as the Tauri CLI writes it | A `.sig` beside every bundle, and so the feeds |
| `LAUNCHER_UPDATER_KEY_PASSWORD` | Its password | The same |
| `WINDOWS_CERTIFICATE` | The Authenticode code-signing certificate, a `.pfx` file in base64 | Signing the Windows binaries and installers |
| `WINDOWS_CERTIFICATE_PASSWORD` | Its password; leave it empty for a certificate without one | The same |
| `WINDOWS_TIMESTAMP_URL` (a variable, not a secret) | An RFC 3161 timestamp server | Optional; `http://timestamp.digicert.com` when unset |

The Signing step hands them to the build as `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and
writes the certificate to the runner's temporary folder, where a later step deletes it whatever happened (a host-mode
runner keeps its files). `launcher/scripts/release-config.ts` then writes the `--config` overlays: with the updater key
it turns on `createUpdaterArtifacts`, and with the certificate it sets `launcher/scripts/sign-windows.ts` as the
Windows sign command. That script runs `signtool sign /fd sha256 /td sha256 /tr <timestamp>` with the certificate for
each file, taking `signtool.exe` from `SIGNTOOL` or the newest Windows 10/11 SDK, and never prints the certificate or
its password. The run then checks that every installer's Authenticode signature is `Valid`. The Linux packages get no
code signature, only the updater's `.sig`.

### The updater key, once

The key is made once and kept for good. Every installed launcher trusts the public key it was built with, so a lost
or replaced private key strands every install on its current version; they would have to reinstall by hand.

1. On a trusted PC, from the repository root after `bun install`, make the pair with the Tauri CLI's signer:

   ```powershell
   bun launcher/node_modules/@tauri-apps/cli/tauri.js signer generate -w "$HOME\.tauri\dainty-lass-updater.key"
   ```

   Give it a strong password. It writes the private key and, beside it, the public key (`.pub`). (This is the Tauri
   CLI's own command, not a script of this repository: check `signer generate --help` for the CLI version pinned.)
2. Store the private key's text as `LAUNCHER_UPDATER_KEY` and the password as `LAUNCHER_UPDATER_KEY_PASSWORD`, on both
   forges. Keep an offline backup of the key and the password; never commit either.
3. Put the public key's text in the apps' config as `plugins.updater.pubkey`. The manager's config inherits it.

> **Maintainer (spec-tauri-shell T-6):** today `tauri.conf.json`'s `pubkey` is empty, and
> `crates/dainty-launcher-app/src/updater.rs`'s test `the_repository_config_has_no_key_and_the_l7_feed` requires it to
> be. `release-config.ts` refuses to build with the updater key while the `pubkey` is empty
> (`TAURI_SIGNING_PRIVATE_KEY is set but tauri.conf.json has no updater pubkey`), because a signed feed the apps can't
> verify would strand them. So no signed release can be built until you decide where the public key goes (the
> committed config, with that test changed, or an overlay the release adds) and whether any release already shipped
> with a key.

### The Windows certificate

1. Get an Authenticode code-signing certificate as a `.pfx`.
2. Encode it and store it as `WINDOWS_CERTIFICATE`, its password as `WINDOWS_CERTIFICATE_PASSWORD`:

   ```powershell
   [Convert]::ToBase64String([IO.File]::ReadAllBytes("C:\keys\codesign.pfx")) | Set-Clipboard
   ```

3. Set `WINDOWS_TIMESTAMP_URL` if the certificate's vendor names its own timestamp server.

The runner needs a Windows 10 or 11 SDK for `signtool.exe` (or `SIGNTOOL` pointing at one).

## Making a release

1. Set the version in `launcher/package.json` on the branch to release from, and land it through the integration
   pass.
2. Run "Launcher release" by hand on that commit, on Forgejo or on the GitHub mirror.
3. Download `launcher-release`. Check `SHA256SUMS.txt` (`sha256sum --check --strict SHA256SUMS.txt`), that every bundle
   has its `.sig` and that `latest.json` and `latest-manager.json` name this version.
4. Run the checks on real machines below with these files.
5. Publishing waits for DX5. Until then, a release reaches players only by handing them the files.

> **Maintainer:** RD7 also plans a one-time Electron **1.6.0** bridge release for installs of the old Electron
> launcher. Nothing in these workflows builds it.

## Checks on real machines

Not run by CI (MIGRATION_PLAN RA14). Each needs the files from one run.

### Windows, a clean VM

1. Install `DaintyLassLauncher_<version>_x64-setup.exe` as an ordinary user. It installs without administrator rights,
   and WebView2 is installed if the VM lacks it. With a certificate set, Windows shows the publisher, and the file's
   properties show a valid signature with a timestamp.
2. Open the launcher. Join a server with an invite (a click on the link opens the launcher), install the runtime and
   reach Ready to play.
3. Install the previous release first, then this one over it: the install updates in place, and the saved server and
   key are still there. With a published feed, an installed older version finds the update, shows Restart to update,
   and comes back as the new version.
4. Uninstall from Windows' settings: the app's files go. Note what happens to the keys and settings in
   `%APPDATA%\DaintyLass Launcher` and the recovery copies in `Documents\DaintyLass`; a player who reinstalls expects
   their keys back.
5. The same for `DaintyLassManager_<version>_x64-setup.exe`: it installs beside the launcher as a separate app, with
   its own entry and its own update feed.

### Linux

1. Install the deb on Ubuntu 22.04, Ubuntu 24.04 and Debian 12, and the rpm on Fedora. The package manager pulls in
   WebKitGTK. The launcher starts from the desktop menu.
2. An invite link (`dainty://` or `dauntless://`) opens the launcher.
3. The key is stored (a keyring is present), and PLAY works through Proton with the inject helper switched on and off.
4. Run the AppImage on SteamOS: it starts, and an update from the feed replaces it.
5. Remove the packages: the app goes. Note whether `~/.config/DaintyLass Launcher` (the keys) stays.
