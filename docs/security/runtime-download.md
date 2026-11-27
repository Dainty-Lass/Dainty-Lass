<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the launcher's runtime download (RX3)

The checklist for how the launcher gets the game runtime from the joined server, decides it may install it, writes it
next to the game, and checks the game folder before every PLAY. Written from the MIT code:
`crates/dainty-launcher-kit/src/net/runtime_files.rs` (and `runtime_files/manifest.rs`), the manifest parser
`net/hostapi/parse.rs` (`parse_runtime_manifest`), `net/http/to_file.rs`, `net/verify.rs`, `controller/runtime.rs`,
`controller/play.rs`, `controller/loader.rs` and `platform/inject.rs`, with the shared names in `dainty-protocol`'s
`manifest`. How the checked files then reach the game is `docs/launcher-injection.md` and
`docs/security/launcher-injection.md` (`crates/dainty-inject`). `SECURITY_REVIEW.md` indexes it.

How to use it: before a launcher release, copy this list into the release commit, check each item in the code (not
from memory), link the code or test that shows it, and sign at the end with the date and the commit reviewed.

## What it is and where the boundary is

- **In, from the joined server (untrusted, over the pinned connection):** the runtime manifest
  (`RUNTIME_MANIFEST_PATH`) and each file from `/content/v1/runtime/files/<name>`.
- **In, from disk (anything running as the player can rewrite it):** the game folder's `Win64` (installed DLLs,
  `winmm.dll`, `mystic_loader.ini`, the marker `.reforged-runtime.json`), the launcher's `settings.json`
  (`trustedRuntimes`, `localRuntimeDir`) and its hash cache `file-hashes.json`, and a local runtime folder the player
  chose.
- **Out:** at most the runtime DLLs and the loader written into `Win64`, the marker, and the list of DLLs and SHA-256s
  handed to the injector or the loader.

The runtime is code the server chooses and the player runs. The launcher's job is that the player agreed to this
exact set from this server, that what lands on disk is that set byte for byte, that nothing else is written next to
the game, and that nothing else is loaded.

## Checklist

### The manifest

- [ ] `parse_runtime_manifest` takes 1–3 files, each named by one of `SET_FILE_NAMES` (`Rustic-Isles.dll`,
  `rustic-isles-runtime.dll`, `winmm.dll`; `MysticParadox.dll` stands for the first, DX4), each slot once, a
  lower-case 64-hex SHA-256 and a size from 1 to `MAX_RUNTIME_FILE_BYTES`; it must list a runtime; the version is
  cleaned and bounded (`MAX_MANIFEST_VERSION`). Anything else is no manifest (tests
  `manifest_parses_the_runtime_and_an_optional_loader`, `manifest_refuses_both_names_of_the_runtime`,
  `manifest_takes_either_shape_and_refuses_neither`).
- [ ] Only a 200 is read as a manifest; 404 and 503 mean none is offered; anything else is a bad answer
  (`runtime_manifest_reply`). A pin mismatch marks the server unreachable (`unreachable_by_pin`).
- [ ] File names are never paths: only the fixed slot names reach a file name or a URL path.

### Acceptance

- [ ] A set is trusted by its id, SHA-256 over its sorted `name:size:sha256` lines (`runtime_set_id`), and only for
  the server it came from (`trustedRuntimes` is keyed by server id); an empty set is never trusted (test
  `set_id_and_trust_follow_the_electron_launcher`).
- [ ] Only the player's `trust_runtime` adds a set; at most 8 sets per server are kept (`MAX_TRUSTED_SETS`).
- [ ] `install_runtime` refuses an untrusted offer (`RuntimeUntrusted`), while busy, and while the game runs.
- [ ] Any change to a name, size or hash makes a new set the player has to accept again.

### The download

- [ ] Each file is fetched over the server's pinned endpoint with the account key headers made for that endpoint
  (`key_headers(&endpoint, ..)`); a pin failure is `CertMismatch` and its error carries no file name.
- [ ] The body is capped at the manifest's size (`max_bytes`, `TooLarge` past it) with a 30 s idle timeout and the
  player's cancel.
- [ ] It is written to `<slot>.rr-download` beside the target, then its size and SHA-256 are compared with the
  manifest; a mismatch deletes it (`Different`). Only a matching file is moved over the target.
- [ ] Free space is checked against the total plus `DISK_MARGIN_BYTES` before anything is fetched.
- [ ] Only files the game folder holds for this way of loading are installed (`player_runtime_files`: the runtime
  alone when injected, the loader too under Proton without the helper).
- [ ] Before installing, every listed file the set does not list, and on an R21 set every retired runtime name, is
  removed (`unlisted`, `retired_present`); a failure to remove is an error, not a skip.
- [ ] The marker records the version, the source `host:port` and the installed files with their hashes.
- [ ] Status 401 and 403 mark the key refused; the page gets a code, not the server's text.

### Replacing a file in use

- [ ] `replace_file` renames over the target; when the game holds it, it renames the target aside
  (`.rr-old-<millis>`), moves the new file in and puts the old one back if that fails (test
  `replace_file_moves_a_locked_dll_aside_and_puts_it_back_if_the_second_rename_fails`).
- [ ] `sweep_aside` removes only `<runtime name>.rr-download` and `<runtime name>.rr-old-*` files, never anything
  else in `Win64`.

### The check before every launch

- [ ] PLAY first checks the game executable against the pinned SHA-256 (`checked_game`, `exe_pin`); a mismatch is
  `GameWrongVersion`.
- [ ] Leftovers refuse PLAY (`loader_leftovers`: the ini, what it names, an unfinished Host session's files, the
  Host tools' names, and, when injecting, a `winmm.dll` that does not stand down). See `host-page.md`.
- [ ] `check_runtime` rehashes every installed runtime file on every look (no cache) and, with an offer, is playable
  only when the player trusted the set and every listed file matches its hash and nothing unlisted or retired is
  present; an untrusted set is `Untrusted` even when the files match (fixed 2026-11-26), otherwise `Missing` or
  `Outdated` (test `check_runtime_states`). With no offer, a folder holding the required files is playable as it is.
- [ ] The DLLs injected are named by fixed slot and carry the offered set's hashes (`runtime_dlls`), so a file changed
  after the check is refused by the injector, which hashes through a held handle; with a local runtime or no offer
  they carry the hash as found.
- [ ] A local runtime folder (`choose_local_runtime`) must hold every required file; it is copied in at PLAY, hashed
  after the copy and recorded in the marker as `local`.

### Windows `unblock`

- [ ] `unblock` removes only the `:Zone.Identifier` stream of the `.rr-download` (or local copy) file, before it
  replaces the target; it is a no-op off Windows and its failure is ignored.

### The 1.14.7 executable

- [ ] The executable's hash goes through `HashCache` (`file-hashes.json`), keyed by path, size and mtime; a file whose
  size or mtime changed during hashing is not cached. At most `MAX_ENTRIES` entries are kept.

## Fuzz targets

- `runtime_input`: `parse_runtime_manifest` (only slot names, each once, sorted, a runtime always, plain hashes,
  sizes within the cap) and the hash cache file read back at start.
- `launcher_hostapi`: `runtime_manifest_reply` and the other host API replies, with every status.
- `content_runtime_index`: the content server's own `runtime.json`, which decides what the launcher is offered
  (`docs/security/content.md`).
- `inject_helper_args`: the injector helper's command line built from the checked DLL list.

## Open points

- [ ] **Accepted (local writer): the executable pin trusts the hash cache.** `file-hashes.json` is in the launcher's user data, which anything
  running as the player can write: an entry with the executable's path, size and mtime and the pinned hash makes
  `checked_game` pass for another file of the same size and mtime. Decide whether the pin should rehash at PLAY, or
  record that a local writer is out of scope.
- [ ] **Fixed 2026-11-26: a matching folder needs a trusted set.** `check_runtime` looks at `trusted` before the
  match, so files placed by something other than the launcher don't play under an offer the player never accepted;
  accepting it (`trust_runtime`) finds the files in place.
- [ ] **Accepted: with no offer** (404 or 503, or an unreachable manifest route) a folder holding the required files
  still plays as found. There is no set to trust, the files are ones an earlier offer installed, and only a process
  running as the player could have put others there.
- [ ] **Accepted (local writer): `loader_stands_down` is a byte search** for the stand-down marker, not a hash, so a `winmm.dll` holding the
  marker passes the leftover check (see `host-page.md`).
- [ ] **Accepted: `retire_loader` trusts the marker in the game folder** for which `winmm.dll` it may delete; a rewritten
  marker can only make it delete the `winmm.dll` already there, so the harm is small.
- [ ] **Accepted, page text to follow: a local runtime is loaded as found:** its hash is whatever the folder holds when PLAY copies it. That is the
  player's choice; the page should say so.
- [ ] **Deferred to that release: the pre-R21 pair (`Rustic-Isles.dll`, `MysticParadox.dll`) is still accepted** (spec-runtime-r21 L-1, DX4);
  drop it from `SET_FILE_NAMES` in the release that ends the pair.

## Sign-off

| Date | Commit reviewed | Reviewer | Notes |
| --- | --- | --- | --- |
| | | | |
