<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Isles shadow replay (RX3)

The checklist for `crates/rustic-isles-replay`, the MIT shadow replay harness of Rustic Isles (RD27,
`docs/port/spec-rustic-isles-replay.md` §14). It opens no listener a player reaches; it is a host tool that reads the
gateway's capture and sends the requests in it to targets the operator names. What it guards is the credentials it
holds and the personal data it reads.

How to use it: before a change to this crate merges, and before its cutover is signed (RD10), copy this list into the
change, check each item in the code (not from memory), link the code or test that shows it, and sign at the end with the
date and the commit reviewed.

## What it is and where the boundary is

- **In:** capture files (`--corpus`, the gateway's `CAPTURE_DIR`; personal data, editable by anyone with the files),
  a key map of raw account keys (`--key-map`), the secrets of each side (`AUTH_SIGNING_PRIVKEY_B64`, `JWT_SECRET`,
  `GAMESERVER_API_KEY`, `GATEWAY_SECRET`), and the targets' answers.
- **Out:** HTTP/1.1 requests over plain TCP to the backend and Director URLs given, raw TCP frames to the chat
  addresses given, the report on stdout and in `--out`, warnings on stderr.

Code: `crates/rustic-isles-replay/src/` (`capture.rs`, `credentials.rs`, `ids.rs`, `request.rs`, `transport.rs`,
`normalize.rs`, `compare.rs`, `runner.rs`, `options.rs`, `cli.rs`).

Fuzz target in `fuzz`: `rustic_isles_replay_input`: arbitrary bytes as capture lines and bodies, as a target's raw
HTTP answer (chunked included), as a key map, through `normalize_str`, `normalize_value`, `normalize_raw_text`,
header and chat stanza normalization, and as two compared answers, never panic; a parsed body survives its wire form
and an answer compared with itself has no differences.

## Checklist

### Credentials (§14 items 1, 2)

- [ ] The signing key mints any player's game token: it, the HS256 secret, the game-server key, the gateway secret
  and the raw account keys are never logged, printed or written to the report.
- [ ] `Debug` of `ReplayTargetAuth`, `ReplayOptions`, `ReplayEndpoint` and `PreparedHttpRequest` shows which secrets
  are set, never their values; a prepared request shows no target, header value or body (D2; tests
  `debug_hides_credentials`, `k7_prepared_debug_hides_values`, `k7_debug_shows_no_secrets`).
- [ ] A bad signing key is a token error turned into an inexact record; its message names the parse error, never the
  key (`credentials.rs` `read_signing_key`; test `s9_bad_signing_key_is_inexact_not_an_error`).
- [ ] The parsed RSA key is cached once per process, keyed by the key's SHA-256, never by its text.
- [ ] The secret flags (`--signing-key`, `--jwt-secret`, `--gs-key`, `--gateway-secret` and their per-side forms) put
  secrets in the process list; the host kit defaults them from `secrets.env`, which is the path to document
  (decision on §16 question 2: the flags stay; moving them is a hostkit row).

### Where requests go (§14 item 3)

- [ ] Requests go only to the URLs and chat addresses the operator gave; no address is read from the corpus (the
  recorded `host` header is dropped, `send_http_request` connects to the base URL only).
- [ ] Plain HTTP and TCP, no TLS: targets should be loopback. The harness does not enforce it (decision on §16
  question 3: parity; a candidate decision row).
- [ ] A router side is served on `127.0.0.1:0` for one request and stopped after it.

### Personal data and the report (§14 items 4, 5)

- [ ] Captures are read, never written back. Corpora, key maps and reports stay out of this repository.
- [ ] The `--out` report is created owner-only: mode 0600 on Unix, narrowed when the file already exists; the default
  ACL on Windows (decision on §16 question 4; `options.rs` `write_report`, checked in `u7_out_report` on Unix).
- [ ] Normalization masks tokens, opaque refresh tokens, nonces and challenges, but a minted value it does not
  recognize (a 64-hex exchange code, a 48-hex account key) appears verbatim in a `ValueDiff` or `TextDiff` when the
  sides disagree, in the text output and in `--out`. Treat the report as secret.

### Replay headers (§14 item 6)

- [ ] The harness is the only intended sender of `x-isles-replay-now-ms` and `x-isles-replay-seed`; a target honours
  them only with `RUSTIC_ISLES_REPLAY_HEADERS` on (core §17 item 1), which a production module leaves off.
- [ ] A pinned side whose first answer with a fresh UUID shows none its seed makes is warned about once on stderr
  (decision on §16 question 1); the run is not refused (`runner.rs` `check_pinning`; test
  `u6_cross_learning_and_the_pinning_warning`).

### Untrusted input (§14 item 7)

- [ ] A corpus line that is not a recognised record is skipped, never a panic (tests `p1_mixed_lines`,
  `p3_skipped_lines`; fuzz target).
- [ ] A target answer without a head terminator or with a bad status line is a transport error, which ends the run
  (test `u4_a_bad_answer_ends_the_run`); chunked decoding stops at a bad or short chunk.
- [ ] Recorded headers are forwarded as recorded (minus the hop-by-hop five); a value holding CR or LF reaches the
  operator's own target unchanged. Only the operator's corpus is replayed.

### Known weak spots (kept for parity, spec §15)

- Memory is not bounded: an answer is read to end of stream and a corpus is loaded whole.
- Any transport error ends the run without a report.
- `x-gateway-secret` is sent though nothing reads it (§15 item 7; decision on §16 question 5: kept).
- Learned-id replacement is a chained plain substring, so a short recorded id can hit inside an unrelated string.
- Live chat is one 50 ms read per frame over raw TCP: a slow or split answer loses stanzas.

## Sign-off

Reviewed by: ______ on ______ at commit ______.
