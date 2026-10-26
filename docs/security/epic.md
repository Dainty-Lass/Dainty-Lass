<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: installing the game from Epic (RX3)

The checklist for `rust/crates/epic`, which the launcher and the host kit use to install Dauntless from the player's
own Epic library (docs/dainty-lass-plan.md DD22, rows DE1 to DE7, DX2). `SECURITY_REVIEW.md` indexes it.

How to use it: before the first release that installs from Epic, copy this list into the release notes' review,
check each item in the code (not from memory), link the code or test that shows it, and sign at the end with the
date and the commit reviewed. An item that does not hold is a bug to fix before that release, not a box to leave
open. DE1 wrote the manifest and chunk readers and stubbed the rest, so the items under "Signing in" and "The CDN"
are for DE2 and DE5 to make true; they say what those rows must deliver.

## What it is and where the boundary is

An Epic session is worth an Epic account: whoever gets its refresh token can act as the player at Epic for weeks. A
manifest says which files go where on the player's disk, and a chunk is the bytes that go in them. A wrong manifest
or chunk is code the player will run.

- **In, from Epic's account service:** the token exchange and library answers, over HTTPS (DE2, DE3, `egs-api`).
- **In, from anywhere:** a download manifest. From a joined server's content service, the game module's own copy,
  or the community archive (DD22). None of these is trusted; the pin is.
- **In, from Epic's CDN:** `.chunk` files from three hosts, without a login (`download::CDN_BASES`).
- **In, from the player:** the authorization code, pasted or captured by the sign-in webview.
- **Out:** files written under the install folder the player chose, one folder per version.

Code: `manifest.rs` (header, sections, checks), `manifest/reader.rs` (bounded reads), `manifest/path.rs` (file
names), `chunk.rs` (chunk check), `auth.rs` (secret types, stubs), `library.rs` and `download.rs` (stubs, CDN
hosts).

Fuzz target: `epic_manifest` in `rust/fuzz` (a whole manifest file under its own pin, the unpacked sections behind
a valid header, and a chunk checked against an entry made from its payload). It checks that nothing panics, that a
wrong pin is always refused, that every accepted file name passes the folder rules again and stays under the root,
each once, with parts inside listed chunks, and that an accepted chunk is exactly the size and SHA-1 its entry says.

## Checklist

### Manifests are pinned before they are parsed

- [ ] `Manifest::parse` hashes the whole file and compares it with the pin before reading a byte of it; a
      mismatch is `PinMismatch` whatever the bytes. Test: `nothing_is_read_before_the_pin_matches`.
- [ ] The pin comes from compiled-in data (the game module's client profile, DE4), never from the server or the
      archive that supplied the manifest. Whichever source answered, the same pin applies (DD22).
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
- [ ] The installer (DE5) builds each path with `InstallPath::under(folder)`, which joins component by component,
      and writes nowhere else. It must also refuse to follow a symlink or junction already inside the folder
      (open with no-follow, or check each parent).
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
- [ ] The installer (DE5) checks every assembled file's SHA-1 against the manifest, then the executable's SHA-256
      against the client profile, before the version shows as installed.

### The CDN: TLS and no redirects

- [ ] Chunks are fetched only from `download::CDN_BASES`, over HTTPS with the webpki roots; plain HTTP is never
      tried, even though the manifests' own `BaseUrl` fields still list `http://` hosts. Those fields are ignored.
- [ ] No redirect is followed: anything but `200` is a failure for that host, and the next host is tried. A chunk
      body is capped at the entry's download size plus a little. (`tests/archive.rs`'s `get` does this today; the
      DE5 downloader must keep it.)
- [ ] The CDN needs no login, so no Epic token is ever sent to it. Ownership is checked first (DE3): nothing is
      downloaded for an account without Dauntless.

### Signing in (DE2)

- [ ] `auth::Token` has no `Debug`, `Display`, `Clone` or `Serialize`; reading one takes `expose()`. `Session`'s
      `Debug` shows the account and expiry, never a token. Tests: `a_session_debugs_without_its_tokens` and the two
      `compile_fail` doc tests in `auth.rs`.
- [ ] The session lives in the keystore, like the account key: never across the bridge, never in a log, never in a
      settings file. Sign out deletes the keystore entry.
- [ ] `egs-api` logs through `log`, and the workspace forwards `log` to `tracing`. Its `error!("{:?}", e)` on a
      failed request prints the URL, and its SID sign-in puts the SID in the URL. DE2 filters the `egs_api` target
      out of every log layer and never calls `auth_sid`. `egs-api`'s own `UserData` and `EpicGames` derive `Debug`
      with the tokens in them: they are never formatted, logged or stored as they are.
- [ ] The authorization code is checked (`AuthorizationCode::new`: letters and digits only) before it is sent, and
      dropped after the one exchange.

## Open points

These do not block DE1 but must be settled, or accepted in writing, before the first release that installs:

- **Epic's client id.** Signing in uses the Epic Games Launcher's own client id and secret, as Legendary and Heroic
  do. Epic does not sanction it (DD22) and could revoke it.
- **Tokens in memory.** `Token` does not zero its memory on drop (no `zeroize` in the workspace). A memory dump of
  the launcher shows the session, as it shows the account key.
- **Chunks Epic deletes.** A version whose chunks return 403 or 404 on every host shows as unavailable (DD22); the
  downloader must not fall back to any other source.
- **Rolling hash.** Chunks are checked by SHA-1 only; Epic's rolling hash is used for the CDN file name, not
  checked. SHA-1 is enough against corruption and, with the manifest pinned, against a CDN serving other bytes for
  a known chunk name (that would need a second preimage).

## Fuzzing

A short run of `epic_manifest` happens on every PR (CI's `fuzz` job). The run before the first release that
installs, from `rust/fuzz`:

```bash
cargo +nightly fuzz run epic_manifest -- -max_total_time=86400
```

Signed: ________ (date, commit)
