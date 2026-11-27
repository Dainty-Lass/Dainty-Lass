<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the host kit's game data, store captures, SDK and runtime publishing (RX3)

The checklist for how a server gets the files its game module serves and the runtime its game servers load:
`dainty-host gamedata` (generate, seed examples, export and import a pack), the pinned store captures it fetches,
`dainty-host preset`, the SDK folder the catalog exporter is built against, and `dainty-host publish-runtime`. The
code is `crates/dainty-hostkit` (`gamedata.rs`, `captures.rs`, `gamepack.rs`, `preset.rs`, `publish.rs`,
`commands/gamedata.rs`, `commands/preset.rs`, `commands/publish_runtime.rs`, `paths.rs`) and the in-process
generators `crates/dainty-host/src/generators.rs` runs from `crates/rustic-isles-gamedata` and
`rustic-isles-director::hunt_tables`. Written from the MIT code; `SECURITY_REVIEW.md` indexes it.

How to use it: before a release, copy this list into the release commit, check each item in the code (not from
memory), link the code or test that shows it, and sign at the end with the date and the commit reviewed.

## What it is and where the boundary is

- **In, from the checkout (trusted, reviewed):** `store-captures.json`, the pin naming a git repository, a commit
  and each file's SHA-256 (`Paths::captures_pin`, `modules/rustic-isles/game-data/1.14.7/store-captures.json` in
  the server folder).
- **In, from the network (untrusted):** the capture repository's objects, fetched by `git`.
- **In, from the host (trusted, but anyone's file):** a game-data pack (`.json.gz`) to import, a `preset/` folder and
  its `preset.json`, the game exports and raw sources in `.workspace`, the SDK in `.workspace/sdk`, and a `--from`
  folder for `publish-runtime`.
- **Out:** the backend's and the Director's game-data files, `.workspace/runtime/` (the runtime DLLs and
  `runtime.json`, which the content server serves to every launcher and the Director injects into every game
  server), and the backups each overwrite leaves.

The generators' own parsing is `rustic-isles-gamedata.md` and the hunt-table import `rustic-isles-director.md`;
the runtime's verification on a player's PC is `runtime-download.md`. This one covers what the host kit lets in.

Fuzz target in `fuzz`: `host_gamedata_input` (`parse_capture_pin` on any text: a pin that passes has a full commit,
real SHA-256 hashes and only plain relative paths under the captures folder; `example_file_name` on any name).
Related targets for the generators: `rustic_isles_gamedata_formats`, `rustic_isles_gamedata_windows`,
`rustic_isles_1_14_7_version_data`.

## Checklist

### Store captures

- [ ] The pin's name is `[A-Za-z0-9._-]+` and not `.` or `..`, since it names the bare repository under
  `.workspace/captures`; the commit is exactly 40 lowercase hex digits; every file is a plain relative path (no
  leading or trailing `/`, no `\`, `:`, control character, empty, `.` or `..` part) with a 64-lowercase-hex SHA-256;
  an empty file list is refused (`parse_capture_pin`; test `parses_valid_pin_and_rejects_bad_fields`).
- [ ] The repository is fetched at the pinned commit only (`git fetch --depth 1 <repository> <commit>` into a bare
  repository), and each file is read from that commit with `git show <commit>:<path>`, never from a branch.
- [ ] Every file is checked against its pinned SHA-256 (as fetched, or with CRLF turned to LF) before any is
  written; one mismatch writes nothing (`ensure_captures`).
- [ ] Files already present are rechecked against the pin and refetched when they differ (`missing_captures`; test
  `missing_captures_checks_presence_and_sha256`).
- [ ] `is_valid_repository` accepts only `https://` with a non-empty rest and no control or space character, as
  its error and `host_gamedata_input` say (fixed 2026-11-26: it also took `file://`, an absolute Unix path and a
  Windows drive path, so a pin could have git fetch from a path on the server).
- [ ] A capture failure is reported and the generators still run on what is there; the stores the captures feed
  fall back to their examples.

### Generators and the export

- [ ] The generators run in process (`dainty-host/src/generators.rs`), not as a child program; a failure is a
  `RunResult` with code 1 and a summary line of at most 300 characters, never a panic or an `Err` that stops the
  other steps.
- [ ] They read only the workspace's folders (`GeneratorPaths`: raw game data, backend game data, exports, store
  captures) and write only the backend's and the Director's game-data folders, backing up a file they replace.
- [ ] The version data they use is read and checked first (`version_data::load`, `install`), and a host export that
  changes or contradicts it is reported at the top of the output (V-4).
- [ ] Supplied sources are copied only under the fixed names in `SUPPLIED_SOURCES`, and only when the raw folder
  lacks them (`copy_supplied_sources`).
- [ ] Examples are written only under names from the fixed game-data lists, as `<name>.example.json`, and an embedded
  seed exists only for a fixed set of names (`example_file_name`, `embedded_example`).

### Game-data packs

- [ ] A pack must name `dainty-lass-gamedata` (or the old kit's format), version 1 and this game build; any other is
  refused (`parse_game_data_pack`).
- [ ] Every file in a pack must be one of the fixed entries `pack_entries` lists (`backend/…`, `director/…`,
  `backend/raw/…`), each at most once, so a pack cannot name a path of its own.
- [ ] Every file's data must match its SHA-256 and parse as JSON before anything is written; an import keeps a
  timestamped `.bak` of any changed file that is not still its example (test
  `exports_real_files_and_imports_with_backups`).
- [ ] The gzip stream is read to at most `MAX_PACK_JSON_BYTES` (256 MiB); a pack that inflates past it is refused
  (fixed 2026-11-26).
- [ ] A pack's SHA-256 values are its own, so they catch damage, not a hostile pack; a host should import only packs
  they trust.

### Preset

- [ ] A preset is for this game build, lists only `pack_entries` files, each once, and never the two stores the
  captures build (`PRESET_EXCLUDED`); every file must be there, match its SHA-256 (raw or LF) and parse as JSON,
  or nothing is written (`apply_preset_at`).
- [ ] A file the host changed is kept unless `--force`, which backs it up first (test
  `creates_preset_excluding_stores_and_applies_respecting_force`).
- [ ] A host's preset is their own and is never committed (AGENTS.md).
- [ ] **Accepted:** `executableSha256` is recorded when a preset is made but not compared on apply; the game build
  check is the only binding to the game. A preset holds only checked JSON game data, so a different executable of
  the same build gains nothing from it, and comparing would refuse a host whose game was repaired or re-downloaded.

### SDK

- [ ] The kit only names `.workspace/sdk` (`Paths::sdk_root`, `sdk_header`, `sdk_dir`) and counts its
  `*_functions.cpp` files for `doctor`; it never fetches or generates an SDK. The launcher's Import SDK puts it
  there (`host-page.md`) and the catalog exporter's build reads it (`catalog-exporter.md`).
- [ ] `.workspace/`, `CppSDK/` and `SDK.hpp` are in `.gitignore`: nothing generated from the game is committed.

### Runtime publishing

- [ ] `publish-runtime` fails when the MIT runtime DLL (`MIT_RUNTIME_DLL`) is not built; the loader is optional
  (test `the_loader_is_optional_and_the_runtime_is_not`).
- [ ] Only the fixed `RUNTIME_FILE_NAMES` are published, from the build's outputs, the release's `bin/` or `--from`.
- [ ] Each file is copied to `<name>.tmp` and renamed over the published one; a file the OS holds loaded is moved
  aside (`.rr-old-`) and put back if the rename still fails (`replace_loaded_file`).
- [ ] `runtime.json` lists each file's size and the SHA-256 of the copy as published, and is itself written through a
  temporary file and a rename; retired runtime DLLs and an unlisted loader are removed from the folder.
- [ ] A loader beside the server executable is removed only when its SHA-256 is the kit's published or retired
  loader's; someone else's `winmm.dll` is left alone (`retire_server_loader`, `RETIRED_LOADERS`).
- [ ] **Accepted:** what is published is what the host built or pointed `--from` at; nothing here checks a
  signature. `runtime.json`'s hashes are what launchers and the Director verify against, so they bind the copy, not
  its origin: the host is the origin, and a player trusts the set before it plays (`runtime-download.md`).

## Sign-off

| Date | Commit reviewed | Reviewer | Notes |
| --- | --- | --- | --- |
| | | | |
