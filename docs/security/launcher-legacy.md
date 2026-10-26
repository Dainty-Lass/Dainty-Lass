<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the launcher's legacy edition (RX3)

The checklist for the launcher's Dauntless 1.4.4 path for Dauntless Revived's servers, behind its `legacy` feature
(RD5 reversed): the edition setting, DR's API, the 1.4.4 download into its own folder, the 1.4.4 DLL through the runtime
loader, DR's client mods, and PLAY. A build without the feature compiles none of it in. `SECURITY_REVIEW.md` indexes it.

How to use it: before a release with the feature on, copy this list into the release PR, check each item in the
code (not from memory), link the code or test that shows it, and sign at the end with the date and the commit
reviewed. An item that does not hold is a bug to fix before the release, not a box to leave open.

## What it is and where the boundary is

A Dauntless Revived server hands out the whole 1.4.4 game (about 30 GB), and the launcher writes it under a folder
the player picked or Dauntless Revived's own launcher's folder. The maintainer's UndauntedInternalServer.dll, the
runtime loader (`winmm.dll`) with a `mystic_loader.ini` naming that DLL, and the UE4SS client mods go next to the
game's executable and run as the player. So the server is trusted only for files whose path, size and hash the
compiled-in manifest names, and the DLLs and mods only for the bytes their pins and the bundle's manifest name.

- **In from the server:** `/undaunted/api/ServerStatus`, `Register`, `GetUserInfo`, `HuntRegion` (DR's names: it
  is their server), `HEAD`/`GET /content/v1/manifest`, `GET /content/v1/files/<path>` with the account key, or a
  `downloadBaseUrl` CDN without it. All to the invite's host only, pinned to the invite's certificate in public
  mode, plain HTTP into the tailnet or to loopback in private mode.
- **In from the internet:** `UndauntedInternalServer.dll` and `winmm.dll` from the maintainer's release
  (`DLL_SOURCE`; HTTPS with the usual CA checks, no redirects, no key), unless a build bundles them. The pins and
  the source are empty until the first release, and nothing is installed until then.
- **In from disk:** Dauntless Revived's launcher's `settings.json` (`installDir`), the bundled `client-mods/`
  with its `manifest.json`, `revived/verified-files.json` and `revived/dlls/` in the launcher's data folder.
- **Out:** the 1.4.4 files under the chosen folder; the two DLLs, `mystic_loader.ini` and the mods in its `Win64`
  folder; DR's `dxgi.dll` removed from there; `Engine.ini`; and the game's command line with the account key.

Code: `rust/launcher/src-tauri/src/controller/legacy.rs`, `core/manifest.rs`, `core/manifest_check.rs`,
`core/legacy_pins.rs`, `net/downloader.rs`, `net/verify/install.rs`, `net/autoregion.rs`, `net/hostapi/legacy.rs`,
`platform/client_mods.rs`, `platform/launch.rs` (`legacy`).

Fuzz target: `launcher_legacy` in `rust/fuzz` (the server's manifest and the path rule). DR's downloader tests
(`net/downloader/tests.rs`) cover resume, a corrupt and a missing file, a wrong certificate and the CDN.

## Checklist

### The manifest and the download

- [ ] The compiled-in manifest is checked by build.rs (`validate_manifest` with the pinned executable's hash and
      size); a malformed one stops the build.
- [ ] The server's manifest must equal the compiled-in one (`compare_manifests`: build, count, every size and
      hash). Anything else is `manifest_mismatch` and nothing is downloaded.
- [ ] Every path passes `is_safe_relative_path` and `resolve_inside`: no `..`, no root, no drive, no stream, no
      device name, no backslash, and the joined path stays inside the folder. Nothing the launcher puts beside the
      game (`MANAGED_FILES`: both DLLs, the loader's list, DR's `dxgi.dll`) may be listed.
- [ ] Each file is written to `<file>.part` and renamed only after its size and SHA-256 match; a resumed `.part` is
      hashed again first.
- [ ] In public mode every connection is pinned to the invite's certificate; a mismatch stops the whole job before
      any request (`cert_mismatch`). A CDN never gets the key.
- [ ] Free space is checked first (what is left to download plus 1 GiB).

### The folder

- [ ] A folder that holds another Dauntless (1.14.7) is refused (`folder_other_game`); one with other files is
      refused (`folder_not_empty`). Only an empty folder, one with 1.4.4's own top folders, or one holding 1.4.4 is
      used.
- [ ] Dauntless Revived's launcher's `installDir` is used only when it is absolute, under 200 characters and holds
      the 1.4.4 executable at its pinned size.

### DLLs and mods

- [ ] A DLL is kept in `revived/dlls/` only when its SHA-256 is the pin; a download that does not match is deleted,
      and an empty pin installs nothing (`the_release_is_fetched_once_checked_and_loaded_through_the_loader_list`).
      The fetch has no redirects, a size cap and no key or player data.
- [ ] `mystic_loader.ini` names `UndauntedInternalServer.dll` and nothing else, rewritten when it says anything else.
- [ ] DR's `dxgi.dll` (by its hash) is removed before the DLL step and before PLAY; any other `dxgi.dll` there is left
      alone and refused (`loader_leftovers`), since it could load the DLL twice
      (`dr_s_dxgi_proxy_goes_and_another_dxgi_dll_is_refused`).
- [ ] Under Wine or Proton the launch has `winmm=n,b` and no `dxgi` override.
- [ ] Each DLL and mod file is checked before copying, after copying to `.new` and after the rename; a mismatch is
      `dll_failed` or leaves the mods out (never fatal for the mods).
- [ ] Mod paths pass `safe_relative`; `remove` never deletes a file the bundle still ships, an unsafe path or the
      `Win64` folder; `keep` files are only written when missing.

### PLAY

- [ ] PLAY refuses while any 1.4.4 file is missing or unverified, and hashes the executable against the manifest
      before every launch (`game_files_invalid`).
- [ ] The key goes only on the command line of the game the launcher starts, is masked in the log, and is read from
      the key store for each request (`PlayingEnded` too), never kept in memory.
- [ ] The relay (public mode, port 61000) keeps its owner check (RD21) and pins the certificate.
- [ ] Nothing of Reforged's 1.14.7 runtime is involved: no `REFORGED_` or `MYSTICPARADOX_` variable reaches the game
      and no `MysticParadox.dll` is written. The 1.4.4 DLL reaches the game only through the runtime loader's list
      (RD9).

### The edition

- [ ] Without the feature, a `dauntless-revived://` invite is still refused (RD5) and a saved 1.4.4 server is listed
      disabled (RD22); the page's 1.4.4 calls answer `unknown`.
- [ ] The current server is always one of the chosen edition (`sanitize_settings`); the edition changes only while
      no task or game runs.

Reviewed by: ______ on ______, at commit ______.
