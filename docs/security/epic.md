<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: installing the game from Epic (RX3)

The checklist for `crates/epic`, which the launcher and the host kit use to install Dauntless from the player's
own Epic library (docs/dainty-lass-plan.md DD22, rows DE1 to DE7, DX2). `SECURITY_REVIEW.md` indexes it.

How to use it: before the first release that installs from Epic, copy this list into the release notes' review,
check each item in the code (not from memory), link the code or test that shows it, and sign at the end with the
date and the commit reviewed. An item that does not hold is a bug to fix before that release, not a box to leave
open. DE1 wrote the manifest and chunk readers, DE2 the sign-in, DE3 the ownership check, DE5 the installer
(`download.rs` and `download/`, and the launcher's `net/epic_install.rs` and `controller/install.rs`), DE6 the host
kit's `install-game` (`crates/host`: `epicinstall.rs`, `commands/install_game.rs`).

## What it is and where the boundary is

An Epic session is worth an Epic account: whoever gets its refresh token can act as the player at Epic for weeks. A
manifest says which files go where on the player's disk, and a chunk is the bytes that go in them. A wrong manifest
or chunk is code the player will run.

- **In, from Epic's account service:** the token exchange and library answers, over HTTPS (DE2, DE3, `egs-api`).
- **In, from anywhere:** a download manifest. From a joined server's content service, the game module's own copy,
  or the community archive (DD22). None of these is trusted; the pin is.
- **In, from Epic's CDN:** `.chunk` files from three hosts, without a login (`download::CDN_BASES`).
- **In, from the player:** the authorization code, pasted or captured by the sign-in webview.
- **In, from the data folder:** the resume file an earlier run left (`download/resume.rs`). Only the launcher writes
  it, but it is read as untrusted: a wrong line costs a refetch, never a file left unchecked.
- **Out:** files written under the install folder the player chose, one folder per version.

Code: `manifest.rs` (header, sections, checks), `manifest/reader.rs` (bounded reads), `manifest/path.rs` (file
names), `chunk.rs` (chunk check), `auth.rs` (secret types, sign-in through `egs-api`), `library.rs` (ownership and
its cache, DE3), `download.rs` (the installer and its `Gate`), `download/cdn.rs` (the HTTPS client),
`download/sources.rs` (where a manifest comes from), `download/resume.rs`, `download/md5.rs`. In the launcher:
`platform/epic_session.rs`, `platform/epic_login.rs`, `platform/keystore/epic.rs`, `platform/epic_library.rs`, the
`epic_*` commands in `bridge/commands.rs`, `controller/install.rs` (the profile's pin, the joined server's copy, the
ownership gate, the `game_install_*` commands) and `net/epic_install.rs` (DR's job: pause, resume, cancel,
progress).

The host kit installs through the same crate: `dainty-host install-game` (DE6) signs in with the pasted-code flow,
keeps the session in `.workspace/config/epic-session.env`, and installs one folder per version for the game servers.
Its extra boundaries are the **stdin or `--code-file` input** (the code, as in the launcher's paste fallback) and
the **session file**, a secret like the account key. `epic::builds` (the pins) is shared by the launcher and the host.

Fuzz target: `epic_manifest` in `fuzz` (a whole manifest file under its own pin, the unpacked sections behind
a valid header, a chunk checked against an entry made from its payload, and a resume file's body behind a matching
header). It checks that nothing panics, that a wrong pin is always refused, that every accepted file name passes the
folder rules again and stays under the root, each once, with parts inside listed chunks, that an accepted chunk is
exactly the size and SHA-1 its entry says, and that a resume file yields at most one entry per line and nothing
under another manifest's pin.

## Checklist

### Manifests are pinned before they are parsed

- [ ] `Manifest::parse` hashes the whole file and compares it with the pin before reading a byte of it; a
      mismatch is `PinMismatch` whatever the bytes. Test: `nothing_is_read_before_the_pin_matches`.
- [ ] The pin comes from compiled-in data (the game module's client profile in
      `net/versions/builtin.rs`, DE4; the install looks the version up there, never in the joined server's list),
      never from the server or the archive that supplied the manifest. `download::fetch_manifest` tries the joined server's copy, then the
      profile's sources in order, and parses only bytes whose SHA-1 is the pin; a source that gives other bytes is
      skipped for the next (DD22). Test: `the_manifest_comes_from_the_first_source_that_gives_the_pinned_one`.
- [ ] The header's own SHA-1 of the unpacked data is checked too (`DataHash`); encrypted manifests and JSON
      manifests are refused rather than half-read (`Encrypted`, `Json`).
- [ ] Unpacking is capped: the header's unpacked size must be at most 64 MiB (`MAX_DATA`) and the stream must give
      exactly that many bytes, no more (`TooLarge`, `Inflate`). Test: `a_header_that_lies_is_refused`.
- [ ] Every count is refused when the bytes left could not hold that many entries (`CountTooLarge`), so a count
      can't make the parser allocate or loop past the data. Test:
      `counts_larger_than_the_data_are_refused_before_allocating`.
- [ ] Each section is read inside its declared size and the rest of it skipped; a short or overlong section, a
      truncated string and trailing bytes are errors, never defaults. Tests: `truncated_and_padded_data_is_refused`,
      `every_prefix_and_flipped_byte_fails_cleanly`, `bad_strings_are_refused`.

### File names stay inside the install folder

- [ ] Every name passes `InstallPath::new` or the whole manifest is refused (`UnsafePath`): no root (`/`, `//`), no
      backslash (so no UNC or `\\?\` path), no colon (no drive, no NTFS stream), no `.`, `..` or empty component,
      no control character or `< > " | ? *`, no component ending in a dot or a space, no Windows device name in any
      component (`NUL.txt`, `COM1`, `CONIN$`). Tests: `names_that_leave_the_folder_or_alias_a_file_are_refused`,
      `names_that_leave_the_install_folder_are_refused`.
- [ ] A file with a symlink target is refused (`Symlink`): a link could point outside the folder.
- [ ] Two names that differ only in case are refused (`DuplicatePath`): they are one file on Windows.
- [ ] The installer builds each path with `InstallPath::under(folder)`, which joins component by component, and
      writes nowhere else. Before creating anything it checks the version folder and every parent of every file
      with `symlink_metadata` and refuses a symlink or junction (`LinkInFolder`; Rust's `is_symlink` counts
      junctions on Windows). Tests: `a_link_inside_the_folder_is_refused` (skipped where the account can't make
      symlinks), `names_that_leave_the_folder_never_reach_the_installer`. Accepted gap: a link planted inside the
      folder after that check and before a write (another process running as the player) is followed; such a
      process could write the files itself.
- [ ] Every file part names a chunk the manifest lists and stays inside its unpacked size (`UnknownChunk`,
      `PartOutOfRange`), so assembly never reads past a buffer. Test:
      `parts_must_name_a_listed_chunk_and_stay_inside_it`.

### Chunks are what the manifest says

- [ ] `chunk::decode` checks the chunk against the pinned manifest's entry, never against its own header: the GUID
      (`WrongChunk`), the unpacked size (`Size`, with unpacking capped at that size) and the SHA-1 of the unpacked
      data (`Hash`). Encrypted chunks are refused. Tests: `a_chunk_that_is_not_the_manifests_is_refused`,
      `malformed_chunks_fail_cleanly`.
- [ ] Both packed and stored chunks are handled (a quarter of 1.14.7's are stored). Test:
      `packed_and_stored_chunks_of_every_header_version_decode_and_assemble`.
- [ ] The installer checks every file's size and SHA-1 against the manifest, and its SHA-256 and MD5 where the
      manifest lists them, then the executable's SHA-256 against the client profile, before the version shows as
      installed (`download::verify`, `ExecutableHash`; the launcher writes `userData/epic/<version>.installed`
      only then). A file that fails has its chunks fetched again once; a second failure stops the install
      (`FileMismatch`). Tests: `installs_from_three_hosts_when_one_redirects_and_one_corrupts`,
      `a_file_that_fails_its_check_has_its_chunks_fetched_again`, `a_file_that_fails_twice_stops_the_install`,
      `the_executable_must_be_the_one_the_profile_pins`, `rfc_1321_test_suite`.
- [ ] A chunk that fails `chunk::decode` is fetched again from the next host, never written. Test:
      `installs_from_three_hosts_when_one_redirects_and_one_corrupts`.
- [ ] The resume file lives in the data folder, never the game folder, and names its manifest's pin and folder: one
      for another download is ignored and rewritten. A file the resume file marks as checked was flushed to disk
      (`sync_all`) before the line was written; a file missing or of another size on resume has its chunks fetched
      again. A file changed on disk after its check, with its size kept, is not checked again on resume (only the
      executable is, by SHA-256): repairing an install is a later row's. Tests:
      `a_download_killed_halfway_resumes_without_fetching_again`, `only_this_downloads_file_and_whole_lines_count`.

### The CDN: TLS and no redirects

- [ ] Chunks are fetched only from `download::CDN_BASES`, over HTTPS with the webpki roots (`download::Https`,
      `https_only`); plain HTTP is never tried, even though the manifests' own `BaseUrl` fields still list `http://`
      hosts. Those fields are ignored. The manifest sources are fetched by the same client.
- [ ] No redirect is followed: anything but `200` is a failure for that host, and the next host is tried; a host
      that failed goes to the back of the order. A chunk body is capped at the entry's download size plus 1 KiB, a
      manifest at 16 MiB, every request at 60 s. Test: `installs_from_three_hosts_when_one_redirects_and_one_corrupts`
      (a host answering `302` to a good copy is not followed).
- [ ] The CDN needs no login, so no Epic token or header of the player's is sent to it. Ownership is asked first
      (`download::Gate`; the launcher's refreshes the session and puts it through DE3's `require_ownership`, and
      `game_install_start` asks it too, so the page hears `epic_not_owned` at once): nothing is created or fetched
      for an account without Dauntless, and a library that can't be read, or no session, refuses as well. Test:
      `nothing_is_fetched_for_an_account_without_dauntless`.
- [ ] A chunk that answers 403 or 404 on every host ends the install as `ChunkUnavailable { deleted: true }`
      (`game_version_unavailable` on the page); there is no other source. Test:
      `a_chunk_epic_deleted_everywhere_makes_the_version_unavailable`.
- [ ] The free space the files still need is checked before anything is created (`NoSpace`); a disk that fills
      anyway stops the install with `disk_space`. Test: `too_little_space_is_refused_before_anything_is_fetched`.

### Signing in (DE2)

- [ ] `auth::Token` has no `Debug`, `Display`, `Clone` or `Serialize`; reading one takes `expose()`. `Session`'s
      `Debug` shows the account and expiry, never a token. Tests: `a_session_debugs_without_its_tokens` and the two
      `compile_fail` doc tests in `auth.rs`.
- [ ] The session lives in the keystore, like the account key: `userData/keys/epic.session`, encrypted by the
      account keys' encryptor (`platform/keystore/epic.rs`), with no Documents copy. Never across the bridge
      (`epic_get` answers `EpicView`: signed in, display name, signing in), never in a settings file. Its tokens go
      on the log's secret list when read or saved. Sign out deletes the file whether or not Epic answers. Tests:
      `a_session_round_trips_encrypted`, `signed_in_after_a_restart_until_signed_out`.
- [ ] `egs-api` logs through `log`, which the server GUI forwards to `tracing`. Its `error!("{:?}", e)` on a failed
      request prints the URL, and ending a session puts the access token in the URL. The launcher's log layer
      drops every event whose target, or forwarded `log` target, is `egs_api` (`log::is_muted`), and drops it for
      the whole subscriber (`event_enabled`), so the server GUI's log channel never sees it either. `auth_sid` is
      never called. `egs-api`'s `UserData` and `EpicGames` (which derive `Debug` with the tokens) live only inside
      `EpicAccounts`' calls, and its errors become `AuthError` without their text. Test:
      `no_code_or_token_reaches_any_log` (a double for Epic that leaks through both `tracing` and `log`, and
      every event and the log file checked for the code and the tokens).
- [ ] The authorization code is checked (`AuthorizationCode::new`: letters and digits only, at most 256) before it
      is sent, and spent by the one exchange. A pasted JSON page gives only its `authorizationCode`
      (`from_pasted`); the bridge refuses a paste over 1024 characters. Fuzzed: `epic_manifest`'s `Pasted` input.
- [ ] The sign-in window (`platform/epic_login.rs`) is private (`incognito`: no Epic cookies kept), loads only
      HTTPS, opens no popups and gets no bridge (the capability names the main window only). Its script acts only on
      `https://www.epicgames.com/id/api/redirect`, and a code is taken only when the page shown is that one, so a
      third-party login page can't sign the player in to an account of its choosing. The code reaches Rust as a
      navigation to `epic-sign-in.invalid` that is cancelled, never sent. Tests:
      `the_code_is_taken_only_from_epics_redirect_page`, `only_https_loads`.

### Ownership (DE3)

- [ ] A record is Dauntless when its `app_name` is exactly `Jackal`. `CATALOG_CONFIRMED` also requires namespace
      `jackal` and catalog item `2f415248cb524eaf950a16e55d315dac` once the maintainer has read them from a real
      library (spike DE1, step 2); until then it is off, so a wrong guess can't refuse an owner. Test:
      `jackal_is_matched_by_app_name_until_the_catalog_is_confirmed`.
- [ ] `egs-api`'s `library_items` logs a failed page and returns what it had, so a request that failed at once
      reads as an empty library. An empty library is `Unreachable`, never "not owned" (`from_egs`). A page failing
      halfway can still read as "not owned": that fails closed, and a refusal is kept only `REFUSED_FOR` (10 min).
- [ ] Only the access token is handed to `egs-api`, never the refresh token; an expired one is refused before
      Epic is asked (`SessionExpired`).
- [ ] The cache (`epic-ownership.json` in the user data folder, and memory) holds the account id, the answer and
      when: never a token. An owner's answer is kept `OWNED_FOR` (24 h), a refusal 10 min; errors are never
      cached; an answer for another account, from the future or unreadable is no answer. Signing out calls
      `Ownership::forget` (DE2). Tests: `the_file_keeps_the_answer_across_restarts_and_never_a_token`,
      `a_damaged_or_foreign_file_is_no_answer`, `answers_expire`, `a_network_error_refuses_and_is_never_cached`.
- [ ] A download starts only past `require_ownership` (the launcher's `epic_library::require`, `epic_not_owned`):
      DE5's `game_install_start` and the host kit's install (DE6) call it first. Tests:
      `an_account_without_dauntless_cant_start_a_download` (crate and launcher).
- [ ] The launcher's log drops every `egs_api` target (`epic::is_egs_api_target`). Test:
      `a_registered_key_never_reaches_the_file`.

### The host kit's install (DE6)

- [ ] The pins are compiled in (`epic::builds`, one table for the launcher's profiles and the host); `--version`
      only picks a row, and 1.4.4 or an unknown version is refused with what to do instead. No option, env
      variable or file names a manifest, a pin, a CDN host or an executable hash. Tests:
      `a_version_epic_cannot_give_is_refused_with_a_way_forward`,
      `the_command_refuses_what_it_cannot_do_before_asking_for_anything`, `the_hosts_pin_is_the_builds`,
      `every_build_is_whole`.
- [ ] The Epic session is in `epic-session.env` in the config folder (`.workspace/config`), written to a temporary
      file and renamed, mode 0600 on Unix (`write_secret_file`; Windows relies on the folder's ACL, as for
      `secrets.env`), and a file found group- or world-readable is closed to 0600 when read. It is never a
      `host.toml` field, an env file's variable, a command line option or a line of output. `secrets.env` is not
      used: it is made once by setup and its keys are checked at 32 characters, while a session turns over each
      refresh. Tests: `a_fresh_host_signs_in_with_a_pasted_code_and_installs_for_its_game_servers` (mode 0600; the
      code and both tokens appear in no output line and no file but the session file; host.toml holds nothing of
      Epic), `an_expired_access_token_is_refreshed_and_the_new_refresh_token_stored`.
- [ ] The authorization code comes only from stdin or `--code-file` (at most 16 KiB; a file of any other size is
      refused), is checked by `AuthorizationCode::from_pasted` before it is sent (the launcher's paste fallback,
      fuzzed as `epic_manifest`'s `Pasted` input), and is spent by the one exchange; it is not stored. It is shown
      when pasted into a terminal, as any pasted text is; it is not logged. Tests: `a_wrong_code_signs_nobody_in`,
      `a_closed_stdin_says_what_to_do`, `a_huge_paste_is_refused`.
- [ ] Nothing logs. The command installs no `log` or `tracing` subscriber, so `egs-api`'s events (which carry URLs)
      go nowhere; every message printed is the host kit's own wording of an error variant, never `egs-api`'s or the
      response's text. An expired or ended session asks for a new code, an unreachable Epic does not (the stored
      session stays for the next try). Test: `epic_being_down_does_not_ask_for_another_code`.
- [ ] Ownership is asked before the manifest is fetched or any folder is made (`install_game` calls
      `Ownership::require` first, then `download::install` asks the same gate again from the cache). Tests:
      `an_account_without_dauntless_gets_nothing_downloaded_or_created` (no request reaches the CDN, no folder is
      made, `game_dir` is unchanged; an unreadable library refuses as well).
- [ ] The manifest is parsed only if it matches the pin, and must also name `Jackal` and the profile's build
      version, so a source cannot substitute another build. The install is `epic::download::install` unchanged:
      the folder checks, per-file and executable checks and resume are the ones above. A finished install is
      recorded in `.workspace/epic/<version>.installed`; a repeat run trusts the record only if the executable still
      has the pinned SHA-256. Tests: `the_executable_the_profile_pins_must_be_the_one_downloaded` (an executable
      that is not the pinned one stops the install and leaves host.toml alone),
      `the_second_run_needs_no_code_and_fetches_nothing`,
      `a_download_killed_halfway_resumes_without_fetching_again`.
- [ ] Only a finished install of 1.14.7 on a reborn or both server changes `host.toml` (`game_dir`), keeping the
      host's comments, and rewrites the env files only if setup made them; a legacy server and 1.12.0 are left
      alone. Tests: `host_toml_follows_only_for_the_version_the_game_servers_run_on_a_server_that_runs_it`,
      `the_host_keeps_the_hosts_own_comments_in_host_toml`.
- [ ] `install-game --sign-out` ends the session at Epic and deletes the file and the cached ownership answer
      whether or not Epic answers. Test: `signing_out_ends_and_deletes_the_session_even_when_epic_is_down`.
- [ ] In Docker the command runs in the `host` container with `--dir /game` (`docker/host-entrypoint.sh`); the
      session lands in the `workspace` volume with the other secrets, and the container needs no capability.
- Accepted: the session file holds a refresh token in the clear (as `secrets.env` holds the gateway's secrets); the
  host's own user and root can read it. A host that wants it gone sooner runs `--sign-out` after the install.

## Open points

These do not block DE1 but must be settled, or accepted in writing, before the first release that installs:

- **Epic's client id.** Signing in uses the Epic Games Launcher's own client id and secret, as Legendary and Heroic
  do. Epic does not sanction it (DD22) and could revoke it.
- **The refresh token's life.** A stolen `epic.session` file is worth the account until Epic expires the refresh
  token (weeks) or the player signs out; DPAPI or the Secret Service key ties it to the player's own login, as for
  account keys.
- **Tokens in memory.** `Token` does not zero its memory on drop (no `zeroize` in the workspace). A memory dump of
  the launcher shows the session, as it shows the account key.
- **Chunks Epic deletes.** A version whose chunks return 403 or 404 on every host shows as unavailable (DD22); the
  downloader falls back to no other source. It finds out only when it reaches such a chunk, after downloading
  what came before.
- **The joined server's manifest route.** The launcher asks `GET /content/v1/versions/<version>/epic/manifest`
  (`net::versions::server_manifest_path`, DF4b) over the server's pinned connection with that server's own account
  key (`net::hostapi::key_headers`) and nowhere else; whatever it answers is checked against the pin like any
  source. A 401, a 404 (`no_epic_manifest`) or a 503 (`epic_manifest_invalid`) moves on to the profile's sources.
- **Rolling hash.** Chunks are checked by SHA-1 only; Epic's rolling hash is used for the CDN file name, not
  checked. SHA-1 is enough against corruption and, with the manifest pinned, against a CDN serving other bytes for
  a known chunk name (that would need a second preimage).

## Fuzzing

A short run of `epic_manifest` happens on every PR (CI's `fuzz` job). The run before the first release that
installs, from `fuzz`:

```bash
cargo +nightly fuzz run epic_manifest -- -max_total_time=86400
```

Signed: ________ (date, commit)
