<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Launcher release

How DaintyLass Launcher and DaintyLass Manager are packaged, signed and checked (RA14, RD7). This page is the
maintainer's: the keys, the secrets and the checks on real machines are not an agent's job.

## What a run builds

`.forgejo/workflows/launcher-release.yml`, started with Run workflow on any branch, builds both apps at the version in
`rust/launcher/package.json` (Cargo.toml's must match):

| File | Built on | Installs |
| --- | --- | --- |
| `DaintyLass{Launcher,Manager}_<v>_x64-setup.exe` | `windows-msvc` | NSIS, per user, no administrator |
| `DaintyLass{Launcher,Manager}_<v>_amd64.AppImage` | `docker`, in `ubuntu:22.04` | Anywhere with glibc 2.35 or newer |
| `DaintyLass{Launcher,Manager}_<v>_amd64.deb` | the same | Ubuntu 22.04 and newer, Debian 12 and newer |
| `DaintyLass{Launcher,Manager}_<v>_x86_64.rpm` | the same | Fedora |

With the updater key set, each bundle gets a `.sig` and the run writes the two feeds, `latest.json` (the launcher)
and `latest-manager.json` (the manager), which point at the bundles in the `launcher-updates` release of the URL in
`tauri.conf.json`'s `plugins.updater.endpoints`. Everything, with `SHA256SUMS.txt`, is the artifact
`launcher-release` for 14 days. Nothing is published until DX5: then the files go up as the `launcher-updates`
release, feeds included.

Every bundle updates itself through `tauri-plugin-updater` (the deb and rpm through `pkexec`), and refuses a bundle
whose signature is not the key in `tauri.conf.json`. A build without that key, a local one included, never checks
for updates.

## Keys and secrets

Set the secrets on the `silentspud` organisation or the repository (settings, Actions, Secrets). Each is optional:
without it that signature is skipped and the run still builds.

| Secret | What |
| --- | --- |
| `LAUNCHER_UPDATER_KEY` | The updater's ed25519 private key, the whole key file's text |
| `LAUNCHER_UPDATER_KEY_PASSWORD` | Its password |
| `WINDOWS_CERTIFICATE` | The Authenticode certificate with its private key, a `.pfx` in base64 |
| `WINDOWS_CERTIFICATE_PASSWORD` | The `.pfx`'s password |

And the variable `WINDOWS_TIMESTAMP_URL` (settings, Actions, Variables) if the certificate's vendor wants its own
timestamp server; the default is DigiCert's.

### The updater key, once

1. On a trusted machine, from `rust/launcher` after `npm ci`:

   ```sh
   npx tauri signer generate -w ~/.tauri/dainty-lass-updater.key
   ```

   Give it a password. It writes the private key and `dainty-lass-updater.key.pub`.
2. Put the `.pub` file's text in `rust/launcher/src-tauri/tauri.conf.json`, `plugins.updater.pubkey`, and commit it
   (subject `RA14: the updater's public key`). It is public; the launcher and the manager both carry it.
3. Paste the private key file's text into `LAUNCHER_UPDATER_KEY` and the password into
   `LAUNCHER_UPDATER_KEY_PASSWORD`.
4. Keep two offline copies of the private key and its password. **Losing it strands every install** on its version:
   a new key means a manual reinstall for everyone (RD7). Never commit it, paste it in an issue or a log, or put it
   on the Windows runner's account.

The run fails if the private key is set while `tauri.conf.json` has no `pubkey`, so a signed feed never ships to
launchers that can't check it.

### The Windows certificate

1. Export the code-signing certificate with its private key as a `.pfx` with a password.
2. `base64 -w0 codesign.pfx` (or `[Convert]::ToBase64String([IO.File]::ReadAllBytes("codesign.pfx"))` in
   PowerShell) into `WINDOWS_CERTIFICATE`, the password into `WINDOWS_CERTIFICATE_PASSWORD`, then delete the
   exported file.
3. The job writes the `.pfx` to the runner's temporary folder, signs through `rust/launcher/scripts/sign-windows.ps1`
   (the newest Windows SDK's `signtool`) and removes it at the end, failed or not. The runner needs a Windows 10/11
   SDK, which `docs/ci-runners.md` already asks for.

A certificate on a hardware token or a cloud HSM (EV certificates, Azure Trusted Signing) can't be a `.pfx`: then
`sign-windows.ps1` is the one file to change, to that vendor's signing tool.

## Checks on real machines

The row's checks, on the artifact of a signed run. Nothing here runs in CI.

### Windows, a clean VM

1. Install `DaintyLassLauncher_<v>_x64-setup.exe` as a standard user: no UAC prompt, a Start menu entry, the
   publisher shown as the certificate's subject in the SmartScreen and properties dialogs.
2. Open a `dauntless://join?...` invite from a browser: the launcher opens on it. With the launcher running, a second
   invite reaches the same window.
3. The same for `DaintyLassManager_<v>_x64-setup.exe`, side by side with the launcher: two Start menu entries, two
   uninstall entries, the manager doesn't claim invites.
4. **Update:** build a second run at a higher version (`2.0.0-beta.1`), serve its `latest.json` and bundles over HTTPS
   from a host the VM trusts, and install a first-run build whose `plugins.updater.endpoints` points there (a
   `tauri build --config` overlay). The launcher shows the update, installs it on the button and restarts at the new
   version, settings and saved servers kept. Then edit one byte of the served installer: the update is refused.
5. **Uninstall** from Settings, Apps: the program folder and Start menu entries go; the user data folder stays.

### Linux

| Where | Install | Check |
| --- | --- | --- |
| Ubuntu 22.04, Ubuntu 24.04, Debian 12 | `sudo apt install ./DaintyLassLauncher_<v>_amd64.deb` | Starts from the menu; an invite link opens it; `sudo apt remove <package>` (the name `dpkg-deb --info` shows) removes it |
| Fedora (current) | `sudo dnf install ./DaintyLassLauncher_<v>_x86_64.rpm` | The same, `sudo dnf remove` |
| SteamOS (desktop mode) | `chmod +x` the AppImage and run it | Starts; an invite link opens it after the first start (the AppImage writes its own desktop entry) |

On one deb and one rpm system, also the update of step 4 above: the launcher asks for the password through `pkexec`
and comes back at the new version. And on SteamOS, the AppImage's update in place.

Repeat the install and start on one system per format with DaintyLass Manager.

Note what failed against the row (RA14) and the platform; a distribution that needs another dependency is a change to
`tauri.conf.json`'s `bundle.linux` lists.
