<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the content server (RD27)

The checklist for `crates/dainty-content`, the server players download the game runtime, the server's news and its art
pack from, and the launcher reads a game module's client profile and Epic manifest from. Written with the clean-room
MIT rewrite from `docs/port/spec-rd27-content.md`. `SECURITY_REVIEW.md` indexes it.

How to use it: before the rewrite replaces the AGPL server on a host, copy this list into the cutover commit, check
each item in the code (not from memory), link the code or test that shows it, and sign at the end with the date and
the commit reviewed.

## What it is and where the boundary is

Players reach content only through the gateway. What content serves decides which bytes a launcher writes next to the
game, so the boundary it guards is **which files leave the host's folders**, and **that an account key is only ever
sent to the backend on this machine**.

- **In, from players (through the gateway):** the raw request target, `If-None-Match`, `x-dainty-user-api-key`,
  `x-forwarded-for` (`content::input`).
- **In, from the host:** `content.env`, `runtime.json` and the runtime files, `branding.json` and the art pack, the
  news file.
- **In, from elsewhere:** the backend's `GetUserInfo` answer; the GitHub news file (`CONTENT_NEWS_URL`); what game
  modules register (manifest, client profile, the `epic/manifest` file).
- **Out:** the runtime manifest and its two files, art pack images, news, module bodies; log lines.

Fuzz targets in `fuzz`: `content_request` (path, headers, version query), `content_runtime_index` (`runtime.json`),
`content_news` (the news document).

## Checklist

### Listening

- [ ] Every bind host is loopback (`localhost`, a strict `127.x.y.z`, `::1`); anything else stops the start
  (`config::tests::c2_bind_hosts_are_loopback_only`).
- [ ] At most 512 connections per listener, a 20 s header time-out and a 120 s idle time-out, so a stalled client
  cannot hold a download slot for ever (`server::serve`, `Idle`; `tests/app.rs` `stream_limits`).

### What is served

- [ ] A runtime file is served only under the names `rustic-isles-runtime.dll` and `winmm.dll`, matched
  byte for byte on the raw path; since R21 a `runtime.json` naming `Rustic-Isles.dll`, `MysticParadox.dll` or
  `MystPaxInternalServer.dll` is refused as an unknown file and nothing is served (spec-runtime-r21 P-3,
  `runtime::tests::t6_a_retired_name_is_refused_and_never_served`); no decoding, so `%2e%2e`, `..` and sub-paths are 404 (`input::tests::r2_*`,
  parity script `content`).
- [ ] The path read is the runtime folder joined with the **listed** name, never the requested one; listed files are
  checked without following links, and a runtime with any file of the wrong type, size or SHA-256 serves nothing
  (`runtime::load`, `runtime::tests::t7_*`).
- [ ] A file that changed on disk after the check is never served past its listed size, and makes the next request
  check the folder again (`app.rs` `runtime_file`; `tests/app.rs` `a_file_changed_under_the_list`).
- [ ] A versioned runtime folder is one of the version's own names (lowercase letters, digits, `.`, `-`, not starting
  with `.`) under the runtime folder (`ContentModules::runtime_folders`).
- [ ] Art pack images are safe names listed by the folder scan, regular files (not links) of 1 byte to 25 MiB whose
  first bytes match their extension (`branding::scan`, `input::is_safe_image_name`).
- [ ] An Epic manifest is served only while its SHA-1 equals the one the module's client profile pins, from inside
  the module's folder (`modules.rs`).

### Account keys

- [ ] A key is sent only to `BACKEND_URL`, which must be `http://` on this machine (`config::tests::c3_*`).
- [ ] An implausible key (empty, over 256 bytes, a space, a comma from a repeated header, non-ASCII) is refused
  without asking the backend (`auth::tests::implausible_keys_never_reach_the_backend`).
- [ ] The cache holds the key's SHA-256, never the key; at most 2000 entries; a refused key lives 30 s and an
  unavailable answer is not cached (`auth::tests`).
- [ ] No log line carries a key (`tests/app.rs` `l1_the_download_line_and_no_key_anywhere`).
- [ ] Downloads take a slot per account and per server, released when the reply body ends or drops
  (`limits.rs`, `tests/app.rs` `stream_limits`).

### Outgoing requests

- [ ] `CONTENT_NEWS_URL` is an `https://raw.githubusercontent.com/<owner>/<repo>/<branch>/<file>` URL with no
  credentials, port, query or fragment (`config::tests::c6_news_url`).
- [ ] Both outgoing GETs refuse redirects, have a 5 s time-out over the whole exchange and a 1 MiB body limit
  (`client.rs`).
- [ ] A bad news document or a failed fetch keeps the last good news (`news::tests`).

Reviewed by: \_\_\_\_\_\_ on \_\_\_\_\_\_ at commit \_\_\_\_\_\_
