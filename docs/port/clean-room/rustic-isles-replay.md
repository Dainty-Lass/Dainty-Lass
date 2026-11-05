# Clean-Room Provenance Record: `crates/rustic-isles-replay` (RD27)

- **Component**: `crates/rustic-isles-replay` (crate `rustic-isles-replay`, `license = "MIT"`, `Copyright (c) 2026
  Silent Spud`)
- **Behaviour specification**: `docs/port/spec-rustic-isles-replay.md` (its closing "Coordinator decisions" override
  the rest), with `spec-rustic-isles-core.md` §9 and §17 for the replay headers it sends
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a8bf1850cc6713e99` (branch
  `feat/rustic-isles-replay`, reset to `feat/rustic-isles-clean-room` `57495cb1`)
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, globbed or opened anything under `R:\source\dauntless\dainty-restricted` (any
worktree of it, including the session's additional working directory `splashloader-integration-90130d`),
`legacy-revived`, any `rustic-isles` or `rustic-rifts` tree outside dainty-lass, or any other repository. It ran no
`git show` or `git log -p` of another branch (only `git log --oneline` and `git merge-base --is-ancestor` to check the
base). It searched nothing on the web and did not look for golden files, captures or recorded corpora.

### 1.2 Other sources

None beyond this repository: no crate sources from the cargo registry were opened. Library APIs (aws-lc-rs, base64,
regex, percent-encoding, axum, tokio) were used as the implementer already knew them and as the compiler confirmed.

## 2. Files read

- The coordinator's brief; `AGENTS.md`; `docs/port/spec-rustic-isles-replay.md` (whole)
- `docs/port/spec-rustic-isles-core.md`: its heading list, §9 (lines 681-718) and §17 (lines 1274-end).
  `spec-rustic-isles-auth.md` was not opened: §12.2's test keys were taken from the auth crate's `test_support.rs`
- `crates/rustic-isles/`: `Cargo.toml`, `src/replay.rs` (whole), `src/lib.rs` (exports by grep), `src/testing.rs`
  (lines 1-60), `src/settings.rs` (`RUSTIC_ISLES_REPLAY_HEADERS` by grep)
- `crates/rustic-isles-auth/`: `Cargo.toml`, `src/lib.rs` (lines 15-330), `src/test_support.rs` (lines 1-140),
  `src/tokens.rs` (lines 1-260, to learn which claims the game token check wants)
- `crates/service-kit/src/`: `serve.rs` (whole), `jsjson.rs` (`ordered_entries`, `stringify`, `to_js`, `parse`),
  `js.rs` (`date_parse`; public functions by grep)
- `crates/gateway/src/capture.rs` and `redact.rs`: their regex helper and statics by grep (the pattern-helper style)
- `crates/rustic-isles-store/Cargo.toml`; the root `Cargo.toml`
- `fuzz/Cargo.toml`, `fuzz/fuzz_targets/rustic_isles_store_decode.rs` (head, for the form)
- `docs/security/rustic-isles-store.md` and `docs/port/clean-room/rustic-isles-store.md` (format)

## 3. Files written

### 3.1 The crate, from the spec

| File | Spec |
| --- | --- |
| `Cargo.toml` | §11.1 (`serde_json` `preserve_order`; feature `test-support` pulling in `rustic-isles-auth`) |
| `src/lib.rs` | §10.1 exports, §9.5 (`ReplayError`) |
| `src/capture.rs` | §3 (`CaptureBody`, `HttpRecord`, `ChatRecord`, `CaptureRecord`, `parse_capture_line`, `parse_capture_str`, `load_corpus`) |
| `src/credentials.rs` | §4, §11.2 (`ReplayTargetAuth`, `mint_rs256_jwt`, `mint_hs256_jwt`, `Substituted`, `substitute_str`, `substitute_value`) |
| `src/ids.rs` | §5 (`learn_ids`, `apply_ids_str`, `apply_ids_value`; the id shapes of §5.1 and §7.4) |
| `src/request.rs` | §6.1, §6.5 (`PreparedHttpRequest`, `prepare_http_request`, `ReplayHttpResponse::from_recorded`) |
| `src/transport.rs` | §6.4 (`send_http_request`, `parse_http_response`, `serve_ephemeral`, `dispatch_router_request`) |
| `src/normalize.rs` | §7.1-§7.7 (`NormalizerConfig`, `IdNormalizer`, `normalize_jwts_in_str`, `normalize_headers`, `IGNORED_RESPONSE_HEADERS`) |
| `src/compare.rs` | §7.8, §8, §9.4 (`DiffKind`, `compare_http_responses`, the chat stanza functions) |
| `src/runner.rs` | §6.2, §6.3, §9, §10.6 (`ReplayEndpoint`, `ReplayTarget`, `ReplayConfig`, `run_replay`, `ReplayEntryResult`, `ReplayReport`) and the pinning warning |
| `src/options.rs` | §10.2, §10.3 (`ReplayOptions`, `load_key_map`, `run_replay_cli`) |
| `src/cli.rs` | §10.5 (`parse_args`, `USAGE`, `main_from`) |
| `tests/{common/mod.rs,parsing.rs,substitution.rs,request.rs,compare.rs,runner.rs,cli.rs}` | §13.2 P1-P7, S1-S10, Q1-Q7, N1-N12, R1-R2, C1-C5, L1-L4, K1-K7, U1-U8 (L, N10-N12 and U5 partly as unit tests in `src/`) |

### 3.2 Elsewhere

- `Cargo.toml`: `rustic-isles-replay` in `[workspace.dependencies]`; `Cargo.lock`. A seam change in its own commit.
- `fuzz/fuzz_targets/rustic_isles_replay_input.rs`, its `[[bin]]` and the dependency in `fuzz/Cargo.toml`,
  `fuzz/Cargo.lock`
- `docs/security/rustic-isles-replay.md`

## 4. Verification

`cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` clean;
`cargo test -p rustic-isles-replay --locked --no-fail-fast`: 70 tests pass (16 unit, 54 integration). `cargo check`
of the fuzz target passes on stable. The targets are built in the tests: an axum app that mints ids through core's
replay scope (gated or not), raw TCP servers and a mock XMPP-over-TCP listener. Nothing was run against a real module
or the 1.14.7 client; §13.3's end-to-end runs stay in restricted.

## 5. Deviations and ambiguities, and how they were resolved

1. **Detecting ignored pinning** (decision: warn once). A pinned live side is unconfirmed until an answer carries a
   UUID that appears neither in the prepared request, nor in the recorded answer, nor among the side's learned ids.
   That answer decides: when one such UUID is among the first 64 ids core §9.1 derives from the request's seed the
   pinning is confirmed; otherwise one warning goes to stderr naming the side and `RUSTIC_ISLES_REPLAY_HEADERS`, and
   it is also kept in `ReplayReport::warnings` (not serialized, so the report's JSON is §10.6's). The derivation is
   the harness's own; a test checks it against core's `next_uuid` under a seeded scope.
2. **Async.** `run_replay`, `run_replay_cli`, `send_http_request`, `serve_ephemeral` and `dispatch_router_request`
   are async on tokio. `cli::main_from` builds a current-thread runtime itself; `cli::run_with_output` is the same with
   the output streams given (for tests and callers that capture output). `main_from` takes the arguments as
   `std::env::args` gives them: the program first (its file stem is the prefix, `isles-db replay`), then an optional
   `replay`, then the flags.
3. **Names the spec left open:** `ReplayOptions`' per-side fields are `left_jwt_secret`, `right_jwt_secret`,
   `left_signing_key`, `right_signing_key`, `left_gs_key`, `right_gs_key`; `IdMap` (a `BTreeMap`) is the learned map;
   `EntryKind` is the entry's `kind`; `CliCommand`/`CliArgs` are `parse_args`' result (`--json` lives in `CliArgs`,
   not in `ReplayOptions`); `timestamp()` is the raw `t` and `timestamp_ms()` its `Date.parse` value. Added public
   helpers: `parse_http_response`, `key_map_from_json`, `EphemeralServer`, `ReplayReport::to_text` and
   `to_json_pretty`, `PreparedHttpRequest::header`, `FALLBACK_NOW_MS`, `ANON_SUBJECT`, `DEFAULT_HS256_SECRET`.
4. **`normalize_headers`** takes the side's `&mut IdNormalizer` (the spec says the counters are shared).
5. **`PreparedHttpRequest`'s `Debug`** hides the target as well as header values and body, since a `<key:...>` or a
   learned id can sit in a path; it shows the method, header names, body length and the inexact flag.
6. **HMAC** comes from aws-lc-rs (already the RS256 signer) rather than the `hmac` crate, so no new pairing of digest
   versions; the output is the same HMAC-SHA-256.
7. **CLI details the spec does not fix:** `-h`/`--help` anywhere wins, even over a parse error before it; pinning in
   the standalone CLI follows §10.4's rule (on unless `--no-pin-clock` without `--pin-clock`); a value flag consumes
   the next argument even when it starts with `--`.
8. **`Host` header** is the authority as given (`127.0.0.1:61000`, or a bare host without the added `:80`).
9. **Corpus directories:** only regular files with the `jsonl` extension are read; a file's bytes are read as lossy
   UTF-8 rather than failing.
10. **Shapes:** `<plain:ACCOUNT>` takes one or more non-`>` characters, as `<token:SUBJECT>`; the grant shape has no
    word boundary (§7.4 gives none); the JWT run is the regex §7.2 gives, so "exactly three parts" holds by shape.
11. **Redaction alignment** with a non-string value on the live side: it counts as a secret by its key name only.
12. **Chat over TCP:** a read that times out or fails is no stanza; only connect and write failures end the run.
13. **Wrapped errors:** a corpus or run error inside a configuration error keeps its full text
    (`invalid replay configuration: I/O error at <path>: <message>`).
14. **A recorded side built from options** is named `left` or `right` (§10.3), not `recorded`.
15. **`--out` on Unix** is opened with mode 0600 and then narrowed to 0600, since an existing file keeps its mode
    through `open`.

## 6. Similarity

To be measured by the coordinator against restricted `e0e238b`.
