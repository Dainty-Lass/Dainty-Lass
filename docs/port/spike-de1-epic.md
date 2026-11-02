<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Spike: `egs-api` for installing from Epic (DE1)

Can [`egs-api`](https://docs.rs/egs-api/0.14.0/) 0.14.0 do what DD22 needs: sign in with an authorization code, find
`Jackal` in the library, parse the archived 1.14.7 manifest and fetch and check its chunks from Epic's CDN? Desk half
run on 2026-10-26; the hands-on half (an Epic account, the CDN) is the maintainer's, below.

**Verdict: the kill criterion applies to the manifest and the chunks.** `egs-api`'s parser reads every archived
Dauntless manifest correctly, but it doesn't download chunks at all, its chunk reader checks nothing and panics on
short input, and its manifest parser accepts file names that leave the install folder, swallows truncation as
defaults and panics on one malformed field. So, as DE1 says: **`egs-api` keeps sign-in and the library** (DE2,
DE3), and **the manifest parser and the chunk check are written here**, from the format Legendary documents. DE1
did that: `crates/epic`'s `manifest` and `chunk` modules read all nine archived manifests the same as `egs-api`
does and refuse everything listed under "What falls short". The chunk fetch itself is DE5's.

## Setup

- `egs-api` 0.14.0 from crates.io (MIT, by Milan Šťastný / Acheta Games, published September 2026), read in full and
  built in a toy crate outside the repository on toolchain 1.99.1.
- The nine Windows manifests from the
  [Dauntless Manifest Archive](https://github.com/addison2k/DauntlessManifestArchive) at `a730cae`, each matching
  the archive's `SHA1SUMS.txt`. 1.14.7's is `aa70748e4c0a776a991f2bac10ad342adaa31476`, the value Mystic's guide
  pins. They are Epic's files: none is committed. `crates/epic/tests/archive.rs` fetches 1.14.7's and 1.12.0's by
  pinned commit and SHA-1 when `REFORGED_TEST_EPIC_MANIFEST=1` is set.
- Both parsers were run over all nine and their file lists compared (name, size, SHA-1, number of parts), then fed
  hand-made manifests that are well formed except for one field.

## Results

Every archived manifest is binary, zlib-packed, header version 17 (2.1.1: 21). `egs-api` and `crates/epic` agree
on every one, file for file:

| Build | Files | Chunks | Installed (bytes) | Download (bytes) |
| --- | --- | --- | --- | --- |
| 1.5.4.259586 | 442 | 11,460 | 12,087,113,122 | 11,586,899,564 |
| 1.6.2.271281 | 444 | 11,767 | 12,412,094,015 | 11,902,335,068 |
| 1.10.5.365075 | 606 | 15,398 | 16,218,456,472 | 15,633,857,146 |
| 1.12.0.392819 | 609 | 15,583 | 16,421,864,653 | 15,822,495,601 |
| 1.14.1.559288 | 610 | 15,730 | 16,527,559,391 | 15,963,302,944 |
| 1.14.5.582369 | 611 | 15,767 | 16,531,913,892 | 15,994,706,977 |
| 1.14.6.615145 | 611 | 15,758 | 16,531,919,826 | 15,990,660,893 |
| **1.14.7.647472** | **611** | **15,749** | **16,521,629,423** (15.39 GiB) | **15,985,792,608** (14.89 GiB) |
| 2.1.1.682875 | 832 | 14,750 | 15,520,491,606 | 14,877,313,143 |

1.14.7 has the 611 files DE5 expects, `Version.txt` (59 bytes) and `Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe`
(121,407,264 bytes, SHA-1 `739e69ac336ff107ed263cca1de321b8d1236bd3`); the sizes are the archive's table to the
byte. `egs-api` parses it in 24 ms (release). No manifest has symlinks, install tags or a name the folder rules
refuse; files carry flag 4 (Unix executable) or none. 1.14.7's has no custom fields, so it names no CDN host; the
older ones still list `http://` hosts, which are not used (`docs/security/epic.md`).

## What `egs-api` does

- **Sign-in:** the authorization-code flow against Epic's account service with the Epic Games Launcher's client id,
  refresh and resume, sign out (`EpicGames::auth_code`, `login`, `logout`). The session is `UserData`, which
  serialises (tokens included) for storage.
- **Library:** `library_items` follows the pagination; each record has `app_name`, so `Jackal` is a lookup.
- **Manifests:** `DownloadManifest::parse` reads binary manifests (and Epic's JSON ones); `files()`, `total_size()`,
  `total_download_size()`; chunk GUIDs, rolling hashes, SHA-1s, groups and download sizes; the CDN path
  (`ChunksV4/{group}/{hash}_{GUID}.chunk`) when the manifest carries a `BaseUrl`.
- **Chunks:** `Chunk::from_vec` reads a chunk already in memory and unpacks it. **There is no chunk download**, and
  nothing assembles files.

## What falls short

The manifest parser, on a manifest that is not exactly well formed:

- Every read failure becomes a default (`unwrap_or(0)`, `unwrap_or_default()`), so a truncated manifest parses into a
  partial one; `parse` returns `Option`, with no reason.
- A file part whose declared size is over 28 bytes panics (`attempt to subtract with overflow`,
  `download_manifest.rs:357`) in a debug build and silently misreads in a release one. Reproduced.
- `../../evil.dll` and `C:/Windows/evil.dll` are accepted as file names, and so is a part that reads past its
  chunk's end. Reproduced. Symlink targets are read and thrown away.
- Counts are not bounded by the data: a count of 4 billion loops 4 billion times pushing default entries.
- Sections are not skipped by their declared size, so a newer writer's extra fields would misalign everything after
  them; data version 1 overwrites the build version with the build id.
- An uncompressed manifest's SHA-1 is not checked.
- `files()` drops every chunk part when the manifest has no `BaseUrl`, which is 1.14.7's case.
- The public types drop what an installer needs: file flags (the Unix executable bit Dauntless sets), install tags,
  each chunk's unpacked size, and 2.1.1's per-file MD5 and SHA-256.

The chunk reader: it indexes the buffer without bounds checks (a short chunk panics), ignores the header size, unpacks
without a cap, and checks neither the GUID, the size nor the SHA-1 against the manifest.

Not shortfalls for sign-in, but DE2 must work around them (`docs/security/epic.md`, "Signing in"):

- `EpicGames`, `UserData` and the internal `EpicAPI` derive `Debug` with the tokens in them.
- It logs through `log`, which the workspace forwards to `tracing`; failed requests are logged with `{:?}`, which
  prints the URL, and `auth_sid` puts the SID in the URL. Filter the `egs_api` target out; never use `auth_sid`.
- Its `reqwest` client follows redirects and keeps cookies, which is fine for Epic's account service and one more
  reason it isn't the CDN client.

## Licence and weight

MIT, which an MIT crate can depend on (DD26). It brings `reqwest` 0.13 with its default TLS, which is rustls on
aws-lc-rs with `rustls-platform-verifier`: **no `openssl-sys`**, so `deny.toml`'s ban holds. 189 crates in all; most
are already in the workspace's lock through Tauri, hyper and rustls. New ones: `cookie_store`, `publicsuffix`,
`psl-types`, `h2`, `quinn`, `quinn-proto`, `quinn-udp`, `lru-slab`, `rand_pcg` and a second SHA-1 (`sha-1` 0.10,
beside `sha1` 0.11), all MIT or Apache-2.0. DE2 adds it as `egs-api = "=0.14.0"` in `[workspace.dependencies]`; DE1
doesn't, since nothing calls it yet.

## What DE1 laid down

`crates/epic` (`license = "MIT"`, born MIT per DD26), used by the launcher and the host kit:

- `manifest`: `Manifest::parse(bytes, &pin)` hashes the file and refuses a mismatch before reading it (DD22), then
  reads the header, the meta, chunk, file and custom-field sections within their declared sizes. It refuses file
  names that leave the folder (`InstallPath`: no root, backslash, colon, `.`/`..`, device name, control character,
  trailing dot or space), symlinks, names given twice (case folded), unknown chunks, parts past a chunk's end and
  counts the data can't hold. `Sha1Digest`, `Guid`, `ChunkInfo::cdn_path`, `FileEntry` with flags, tags, MD5 and
  SHA-256, `install_size()`, `download_size()`.
- `chunk`: `decode(raw, &ChunkInfo)` for packed and stored chunks of header versions 1 to 3, checked against the
  manifest's GUID, unpacked size (unpacking capped there) and SHA-1.
- `auth`: `Token` (no `Debug`, `Display`, `Clone`), `AuthorizationCode`, `Session` (its `Debug` shows no token),
  `LOGIN_URL`; `sign_in`, `refresh`, `sign_out` return `NotImplemented { row: "DE2" }`.
- `library`: `APP_NAME = "Jackal"`; `owns_dauntless` returns `NotImplemented { row: "DE3" }`.
- `download`: `CDN_BASES` (the three hosts), `chunk_url`; `install` returns `NotImplemented { row: "DE5" }`.

Tests: 22 unit tests and 2 doc tests on synthetic manifests and chunks; the opt-in archive test passes against the
real 1.14.7 and 1.12.0 manifests. Fuzz target `epic_manifest`: 2.3 million runs in 90 s with no finding.
Checklist: `docs/security/epic.md`.

## The maintainer's hands-on steps

Not done by DE1, since they need an Epic account that owns Dauntless and download game data. About 15 minutes.

1. **Sign in with `egs-api`.** Unpack the crate somewhere outside the repository
   (`https://static.crates.io/crates/egs-api/egs-api-0.14.0.crate` is a `.tar.gz`) and from its folder run
   `cargo run --example auth`. It opens `auth::LOGIN_URL` in the browser; sign in, copy the `authorizationCode` from
   the JSON page and paste it. Expect "Logged in as: <your name>". Note whether the code was 32 hex digits
   (`AuthorizationCode::new` accepts letters and digits). Don't set `RUST_LOG`: at debug level the example's logger
   would print request details.
2. **Check the library.** In `examples/library.rs`, replace `.take(20)` with
   `.filter(|r| r.app_name == "Jackal")`, then `cargo run --example library`. Expect one record with
   `app_name: "Jackal"`, `namespace: "jackal"` and catalog item `2f415248cb524eaf950a16e55d315dac` (the archived
   manifests' `CatalogItemId`). Write down the namespace and catalog item id for DE3.
3. **Delete the saved session.** The examples store the tokens in plain text in `%USERPROFILE%\.egs-api\token.json`
   (`~/.egs-api/token.json` on Linux). Delete that file once done.
4. **Fetch chunks from each CDN host.** From `rust/` in this repository:

   ```bash
   REFORGED_TEST_EPIC_CDN=1 cargo test -p epic --test archive chunks_from_every_cdn_host -- --nocapture
   ```

   (PowerShell: `$env:REFORGED_TEST_EPIC_CDN = "1"` first.) It fetches the 1.14.7 manifest by pin, then from each of
   the three hosts in `download::CDN_BASES` the chunks of `Version.txt` and the first two of `Dauntless.exe` (about
   3 MB per host), over HTTPS with no redirect followed. Each must decode and match the manifest's SHA-1, and
   `Version.txt` assembled from them must match its SHA-1 (DE1's verify line). Expect one `ok <url>` line per chunk
   and a `Version.txt from <host>: ...` line per host, then `test result: ok`. A failure names the URL: `HTTP 403`
   or `404` means Epic has deleted that chunk on that host; `HTTP 301`/`302` means the host redirects, which the
   downloader refuses (tell DE5 which host); a decode error is a bug here.
5. **Record the outcome** (date, hosts that passed, the library record, the code format) for the integration pass
   to add here, then strike DE1.
