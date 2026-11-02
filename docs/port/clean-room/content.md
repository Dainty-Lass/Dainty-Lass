# Clean-Room Provenance Record — `services/content` (RD27)

- **Component**: `services/content` (`content` crate, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`)
- **Behaviour Specification**: `docs/port/spec-rd27-content.md` at commit `df11ac9` (branch `docs/rd27-spec-content`)
- **Clean-Room Worktree**: `R:\source\dauntless\.wt\content-mit` (branch `feat/rd27-content-mit`, created from
  `docs/rd27-spec-content` at `df11ac9`)

## 1. Worktree Isolation (`git sparse-checkout list`)

The worktree is a dainty-lass worktree. At `df11ac9` dainty-lass holds only its MIT tree (the AGPL services, the
launcher and the game modules are dainty-restricted's since the RD27 split), so no AGPL source exists in it to exclude.
The sparse checkout additionally leaves out `ui/`, which the implementation does not need:

```text
/*
!/ui/
```

Checks run before any code was written, from the worktree root:

- `Test-Path ui`, `Test-Path services/content`, `Test-Path rust`, `Test-Path modules` all returned `False`.
- `git ls-files | grep -E '^(services/content|rust/|modules/|legacy/|reference/)'` returned nothing.

The implementer never opened, listed, searched or ran git commands against dainty-restricted, `legacy-revived`,
`rustic-*`, `dainty-dashboard`, `dainty-legacy`, `dainty-lass-ui` or another worktree under `R:\source\dauntless\.wt`,
and never looked at any historical version of `services/content`, `rust/services/content` or
`services/content-plugin` in dainty-lass's history. No web search was made. (The session's own working directory was
a dainty-restricted worktree; every command used absolute paths into `content-mit` and nothing under the working
directory was read.)

## 2. Category A Files Not Carried Here

The Category A files of the spec's section 1.2 live in dainty-restricted, which the implementer may not open. The
implementer left them for the integration pass (section 5), which copied them in:

- `services/content/src/modules.rs` (with its two log calls pointed at `service_kit::linelog`, spec 1.4). The
  implementer's branch held a **stand-in** `modules.rs` written clean-room from the spec's summary (section 2.14), so
  the crate built and its routes could be tested. It assumed `ContentModules::{new(Arc<dyn RegistryView>), none(),
  resolve, resolve_edition, versions, client_profile, runtime_folders, epic_manifest(&GameVersion) ->
  Result<EpicManifest { bytes, sha1 }, EpicManifestError>}`, the `CorePlugin` impl and
  `RegistryView::module_dir`.
- `services/content-plugin/{Cargo.toml, src/lib.rs, tests/parity.rs}`.
- `services/content/tests/parity/{content,content-none,content-invalid,content-index}.{requests,expected}.json`.
  The implementer's `tests/parity.rs` skipped those four scripts with a `SKIPPED` line until they arrived;
  `spec-selftest`, a script written for this rewrite from the spec alone, ran the harness meanwhile.

## 3. Clean-Room Implementer Session

One Claude Code subagent session (2026-11-02), launched by the maintainer's session with the clean-room rules.

- **Files Read**:
  - `docs/port/spec-rd27-content.md`, `docs/port/clean-room/hostkit.md`, `AGENTS.md`
  - `Cargo.toml`, `Cargo.lock` (versions only), `.forgejo/workflows/ci.yml` (the "every package is MIT" step)
  - `crates/service-plugin/src/lib.rs`, `crates/service-plugin/src/chat.rs` (head), `crates/service-plugin/Cargo.toml`
  - `crates/module-api/src/{lib,version,manifest,module,registrar,stub,ids}.rs` (the parts naming the types used)
  - `crates/protocol/src/{manifest,invite}.rs`, `crates/protocol/Cargo.toml`, `crates/protocol/src/lib.rs` (grep)
  - `crates/service-kit/src/{env,js,jsjson,linelog,context,serve}.rs`, `crates/service-kit/Cargo.toml`
  - `crates/controller/src/service_library.rs` (head), `crates/controller/tests/echo-service/{Cargo.toml,src/lib.rs}`
  - `services/admin/{Cargo.toml,src/lib.rs,src/server.rs,src/upstream.rs}` (patterns for an MIT service)
  - `crates/hostkit/src/hostconfig.rs` (the `content.env` lines, by grep)
  - `fuzz/Cargo.toml`, `fuzz/fuzz_targets/runtime_input.rs`, `docs/security/plugin-host.md`
- **Files Written**:
  - `services/content/Cargo.toml`
  - `services/content/src/{lib,app,auth,branding,cli,client,config,input,limits,log,modules,news,runtime,server,
    service}.rs` (`modules.rs` is the stand-in of section 2)
  - `services/content/tests/app.rs`, `services/content/tests/parity.rs`, `services/content/tests/parity/harness.rs`
  - `services/content/tests/parity/spec-selftest.{requests,expected}.json` (expected replies derived by hand from the
    spec, not recorded)
  - `fuzz/fuzz_targets/{content_request,content_runtime_index,content_news}.rs` and their `[[bin]]` entries in
    `fuzz/Cargo.toml`
  - `docs/security/content.md`, this record
  - `Cargo.toml`: the `services/content` member; `content`, `bytes`, `http`, `http-body` in
    `[workspace.dependencies]`

## 4. Verification

With `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=line-tables-only`, `-j 4`, on Windows:

- `cargo fmt --all --check`: clean
- `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings
- `cargo test -p content`: 46 unit tests, 7 HTTP tests (`tests/app.rs`), 5 parity tests (4 skipped until the
  fixtures arrive, `spec_selftest` run); 0 failed
- `cargo test --workspace --no-fail-fast`: 673 passed, 0 failed
- `cargo metadata --no-deps`: all 19 packages MIT
- `cargo check --manifest-path fuzz/Cargo.toml --bin content_request --bin content_runtime_index --bin content_news`:
  builds

## 5. Integration Pass

One Claude Code subagent session (2026-11-02), the integration pass and similarity reviewer, which may read the AGPL
code. Worktree `R:\source\dauntless\.wt\content-int`, detached at `16e66e0` (the implementer's worktree still
holds the branch), its commit pushed to `feat/rd27-content-mit`.

**Copied in**, byte for byte apart from an `SPDX-License-Identifier: MIT` / copyright header (spec 4.1):

| File | From (dainty-restricted) | Change |
| --- | --- | --- |
| `services/content/src/modules.rs` | `ee7d90b` (unchanged through `75e0552`) | header; `use crate::log` becomes `use service_kit::linelog`, and the two `log::info` calls `linelog::info` (spec 1.4) |
| `services/content-plugin/Cargo.toml` | `75e0552` (`feat/rd27-same-both-ways`) | header; `license.workspace = true` becomes `license = "MIT"` (the workspace's is MIT too; spelled out as the other MIT crates do) |
| `services/content-plugin/src/lib.rs` | `75e0552` | header. The one-line `export_service!(content::service::ContentService::default())`: restricted's newer shape, which the rewrite's `content::service` already matches |
| `services/content-plugin/tests/parity.rs` | `75e0552` | header. Runs every recorded script compiled in (`controller::service_compiled_in`) and through the library |
| `services/content/tests/parity/{content,content-none,content-invalid,content-index}.{requests,expected}.json` | `ee7d90b` | none (checked with `cmp`) |

`75e0552` was taken over `ee7d90b` for the three `content-plugin` files because `ee7d90b`'s `lib.rs` holds its own
`ContentService` glue, which the rewrite's `content::service::ContentService` replaces. `Cargo.toml` (workspace) gains
the `services/content-plugin` member. `content-plugin`'s dev-dependencies are `75e0552`'s as they were; the rewritten
harness needs no more than they give.

**Integration edits to the rewrite** (the real `modules.rs` differs from the stand-in in two places):

1. `ContentModules::none()` does not exist; the real table is `Default` (no module folders). `server.rs` (2 calls)
   and `tests/app.rs` (7 calls) use `ContentModules::default()`.
2. `epic_manifest` returns `Result<(Bytes, Sha1Hex), EpicManifestError>`, not an `EpicManifest { bytes, sha1 }`
   struct. `app.rs`'s `epic_manifest` route destructures the pair; the ETag formats the `Sha1Hex` (its `Display` is
   the lowercase hex the struct's `String` held). `EpicManifestError::Mismatch`'s fields are `Sha1Hex` too; the route
   only formats them, so it needed no change.
3. The `SKIPPED` path is gone: `tests/parity.rs` runs the four recorded scripts unconditionally, and the harness's
   `present()` (used only by it) is removed.

`service.rs` needed no change: the real `RegistryView` and `ContentModules::new` are what the stand-in assumed.

**Results** (`CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=line-tables-only`, `-j 2`, Windows):

- Parity, all four recorded scripts plus `spec-selftest`: in process (`content`'s `tests/parity.rs`) 5/5 pass;
  compiled in (`service_compiled_in`) and through the library (`cargo build -p content-plugin` first,
  `service_library`) 4/4 pass, each test running its script both ways. A deliberately wrong `expected` status made
  the harness fail, so the scripts are compared, not skipped.
- `cargo test -p content -p content-plugin`: 51 unit (including `modules.rs`'s own 5), 7 HTTP, 5 + 4 parity; 0 failed.
- `cargo check --manifest-path fuzz/Cargo.toml --bin content_request --bin content_runtime_index --bin content_news`:
  builds.
- `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets --locked -- -D warnings` 0 warnings.
- `cargo test --workspace --locked --no-fail-fast`: 681 passed, 1 failed: `service-kit`'s
  `metrics::tests::collectors_run_before_a_render_and_clear_drops_gone_series`, which shares the process-wide metrics
  registry with its neighbours and passed in six reruns of `service-kit`'s suite. Nothing here touches it.
- Every package is MIT (CI's `cargo metadata --no-deps --locked` check): 20 packages, 0 others.
- **CI gap:** dainty-lass's `rust` job runs `cargo test --workspace` without building the `content-plugin` cdylib
  first, so `content-plugin`'s parity test fails there with its "run `cargo build -p content-plugin`" message.
  dainty-restricted's workflow has a `cargo build --locked -p … -p content-plugin …` step for this. Workflows are
  RR3's, so it is left to the maintainer.

## 6. Similarity

Run by the integration pass (section 5), the method of `similarity.md` section 1 (3+-line runs of trimmed, non-blank,
non-comment lines; matched lines over the new file's lines). The new text is the implementer's commit `16e66e0`, before
the integration edits. Two baselines: dainty-restricted's `services/content` at `ee7d90b` (its `service.rs` at
`75e0552`, and `content-plugin/src/lib.rs` at `ee7d90b`, where that code lived before) and DR's
`rust/services/content` at `24b7624`. Each file is measured against its counterpart and, to catch code moved between
files, against the whole baseline crate concatenated ("all"; restricted's includes its three content fuzz targets).
The script is in `similarity.md` section 5.

| File (new) | New lines | vs restricted (old / matched / %) | vs restricted, all | vs DR (old / matched / %) | vs DR, all |
| --- | ---: | ---: | ---: | ---: | ---: |
| `src/lib.rs` | 44 | 19 / 13 / **29.5%** | 13 / 29.5% | 20 / 3 / 6.8% | 3 / 6.8% |
| `src/app.rs` | 626 | 572 / 25 / 4.0% | 37 / 5.9% | 491 / 13 / 2.1% | 28 / 4.5% |
| `src/auth.rs` | 359 | 384 / 8 / 2.2% | 11 / 3.1% | 253 / 8 / 2.2% | 11 / 3.1% |
| `src/branding.rs` | 289 | 428 / 30 / 10.4% | 44 / 15.2% | 290 / 30 / 10.4% | 39 / 13.5% |
| `src/cli.rs` (DR: `main.rs`) | 98 | 60 / 25 / **25.5%** | 28 / 28.6% | 88 / 19 / 19.4% | 22 / 22.4% |
| `src/client.rs` | 124 | 58 / 29 / 23.4% | 35 / **28.2%** | 58 / 29 / 23.4% | 35 / **28.2%** |
| `src/config.rs` | 303 | 218 / 57 / 18.8% | 63 / 20.8% | 259 / 37 / 12.2% | 40 / 13.2% |
| `src/input.rs` (DR: `app.rs` + `news.rs`) | 632 | 339 / 113 / 17.9% | 161 / **25.5%** | 797 / 26 / 4.1% | 64 / 10.1% |
| `src/limits.rs` | 96 | 107 / 19 / 19.8% | 17 / 17.7% | 97 / 16 / 16.7% | 16 / 16.7% |
| `src/log.rs` | 1 | 34 / 0 / 0.0% | 0 / 0.0% | 1 / 0 / 0.0% | 0 / 0.0% |
| `src/news.rs` | 332 | 369 / 54 / 16.3% | 67 / 20.2% | 306 / 54 / 16.3% | 57 / 17.2% |
| `src/runtime.rs` (no DR file) | 321 | 444 / 17 / 5.3% | 23 / 7.2% | — | 21 / 6.5% |
| `src/server.rs` | 204 | 190 / 50 / 24.5% | 50 / 24.5% | 224 / 38 / 18.6% | 38 / 18.6% |
| `src/service.rs` (restricted: `service.rs` + `content-plugin/src/lib.rs`) | 77 | 143 / 38 / **49.4%** | 41 / 53.2% | — | 3 / 3.9% |
| `tests/app.rs` (DR: `tests/streams.rs`) | 534 | 545 / 36 / 6.7% | 36 / 6.7% | 173 / 7 / 1.3% | 13 / 2.4% |
| `tests/parity.rs` | 49 | 38 / 18 / **36.7%** | 18 / 36.7% | 252 / 3 / 6.1% | 3 / 6.1% |
| `tests/parity/harness.rs` (DR: `tests/parity.rs`) | 379 | 233 / 16 / 4.2% | 22 / 5.8% | 252 / 12 / 3.2% | 15 / 4.0% |
| `fuzz/fuzz_targets/content_request.rs` | 45 | 64 / 3 / 6.7% | 3 / 6.7% | — | 0 / 0.0% |
| `fuzz/fuzz_targets/content_runtime_index.rs` | 21 | 23 / 11 / **52.4%** | 11 / 52.4% | — | 0 / 0.0% |
| `fuzz/fuzz_targets/content_news.rs` | 23 | 30 / 0 / 0.0% | 0 / 0.0% | — | 0 / 0.0% |
| `Cargo.toml` (`#` lines are not stripped) | 41 | 41 / 16 / **39.0%** | — | 37 / 15 / 36.6% | — |

Paths are under `services/content/` unless given. Over the 17 rewritten `.rs` files of the crate, 4,468 new lines
match 548 against restricted's counterparts (12.3%) and 295 against DR's (6.6%), against 47–98% for the same files
before the rewrite (spec section 1.3).

### 6.1 The files at 25% or more

- **`lib.rs` (29.5%):** one 13-line run, the `pub mod` list, forced by the public module paths of spec 2.1.
- **`cli.rs` (25.5%):** the `clap` derive header with the name and about text spec 2.3 gives, the `main_from`
  signature of spec 2.1, and the Tokio runtime builder and `ExitCode::FAILURE` tails.
- **`client.rs` (28.2% against all):** one 10-line run, the TLS connector (`with_provider_and_webpki_roots` with
  aws-lc-rs, falling back to `with_webpki_roots`, `https_or_http().enable_http1()`) in a `OnceLock` named `CLIENT`.
  Spec 2.9 describes exactly that fallback, and the same eight lines are in dainty-lass's MIT
  `crates/history-ship/src/s3.rs`; only the static's name is not from the spec.
- **`input.rs` (25.5% against all):** the `Route` enum (spec 2.1 gives every variant, in order), the `VersionKey` enum
  and `version_query`, the first eight lines of `is_safe_image_name`, and two test bodies (22 and 21 lines). See 6.2.
- **`service.rs` (49.4%):** the `service_plugin::Service` impl, whose method signatures the trait fixes, and the
  `PoisonError::into_inner` lock idiom. The baseline is itself Category A (`content-plugin`, RD27), MIT-eligible.
- **`tests/parity.rs` (36.7%):** the `Server { port, stop: Box::pin(async move {` literal the harness contract
  (spec 3.1) fixes, and the four `#[tokio::test(flavor = "multi_thread")] async fn content…()` headers named after
  the scripts.
- **`content_runtime_index.rs` (52.4%, 11 of 21 lines):** the `#![no_main]` / `fuzz_target!` boilerplate, a
  `let … else { return; }` on `parse_runtime_json`, and the invariant that every size is in range and every SHA-256
  is 64 lowercase hex digits (spec 3.3, T).
- **`Cargo.toml` (39.0%):** the `*.workspace = true` lines of the dependency list spec 4.4 names.

### 6.2 What the numbers do not explain away

No file is a copy: the large files sit at 2–10% against their counterparts, and the runs above are what the spec's
fixed contracts force. But a few runs carry **private names the spec never gives**, identical in the rewrite and the
originals:

- `input.rs`: `clean_accent`, `clean_credit` and `is_safe_image_name` (restricted and DR have them in
  `branding.rs`), the test `fn accent_and_credit_cleaning()` with a 21-line body identical to restricted's
  `branding.rs` test, and `version_query`'s five-line body (`url::form_urlencoded::parse(query?.as_bytes())
  .find_map(|(key, value)| match key.as_ref() { … value.into_owned() … })`) identical to restricted's.
- `news.rs`: an 11-line `match result { Ok(None) => {} Ok(Some((items, etag))) => { state.remote_items = Some(items);
  state.etag = etag; } Err(error) => log::warn(…) }` and an 8-line ETag read ending `Ok(Some((items, etag)))`, the
  same in restricted and DR, private field name `remote_items` included.
- `client.rs`: the static `CLIENT` (above).

The spec does supply most of what these lines say: the test vectors of section 3.3 in the order the original tests use
them (`#C8A24A`, `" #abc "`, `red`, `#abcd`, `#12345g`, `url(x)`, `5`, null; the R3 route list), the phrase
"accent and credit cleaning", the version-query rule of 2.5, the image-name rule of 2.12.1 in the order the code
checks it, and the news fallback of 2.13.3. With `rustfmt` fixing the layout, the shortest idiomatic code for those
rules is these lines, so convergence explains much of it. It does not explain the identical private names
(`remote_items`, `clean_accent`, `clean_credit`, `is_safe_image_name`) as well. All four are in DR's public
`rust/services/content` at `24b7624`, so the model may have seen DR's code in training; the implementer's record
(section 3) shows it never opened either baseline. The matching lines total about 80 of the 4,468.

**Verdict:** an independent implementation, with a small, named residue that resembles DR and restricted more than
the spec alone accounts for. If the maintainer wants that residue gone, it is local: rename `clean_accent`,
`clean_credit`, `is_safe_image_name` and `remote_items`, and rewrite `version_query`, the `news.rs` result handling
and the two `input.rs` tests in another shape. The public paths of spec 2.1 do not change.

## 7. Redo of overlapping items

A second implementer session, in its own worktree (`R:\source\dauntless\.wt\content-redo`, branch
`feat/rd27-content-mit-redo` from `feat/rd27-content-mit` at `7609f9d`), replaced the items the similarity review
flagged. It did not open dainty-restricted, legacy-revived, any other worktree, the history of `services/content`
before `16e66e0`, sections 5–6 of this file or section 5 of `similarity.md`.

**Deleted unread.** Line ranges were found with `grep -n ... | cut -d: -f1` and a brace-matching script that printed
only line numbers, then removed with `sed -i 'A,Bd'`:

- `src/input.rs`: `version_query` with its doc line, the helpers `is_safe_image_name`, `clean_accent`,
  `clean_credit` with their doc lines, and the whole `#[cfg(test)]` module (every `#[test]` fn in the file).
- `src/client.rs`: the line of the `static CLIENT` the client pool lived in.
- `src/news.rs`: emptied whole. Only its `pub` signature lines were printed first (`awk` stopping at the first `{` or
  `;`), so the public items keep their names and shapes.

**Rewritten from `spec-rd27-content.md`**, with new private names and structure:

- `input::version_query` (section 2.5) on `url::form_urlencoded`, first matching pair wins.
- `input::is_image_file_name`, `accent_colour`, `credit_line` (section 2.12.1–2.12.2), callers in `app.rs` and
  `branding.rs` renamed to match.
- `client.rs`: the pool is a function-local `OnceLock` named `POOL`.
- `news.rs` (section 2.13.3–2.13.4): file and GitHub state in two structs, the GitHub refresh spawned on the current
  tokio runtime with a drop guard clearing its in-flight flag, the reply judged by its own function (304, non-2xx,
  over 1 MiB, JSON, `parse_news`, no usable entries), the ETag read with `input::header_text`.
- Tests: `input.rs`'s 16 tests from the vectors of section 3.3 (R1–R5, T1–T4, the recorded manifests, N, B);
  `news.rs`'s 3 tests (the file states, GitHub once a minute with the 304/503/bad JSON/`{"items":[{}]}`/over 1 MiB/
  transport cases, concurrent polls).

**CI:** `.forgejo/workflows/ci.yml`'s Rust job builds `content-plugin` before the tests, since `cargo test` builds no
cdylib for its own tests and content-plugin's parity test loads the library.

**Checks** (`CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=line-tables-only`, `-j 2`): `cargo fmt --all --check`
clean; `cargo clippy --workspace --all-targets --locked -- -D warnings` clean; `cargo build --locked -p content-plugin`;
`cargo test --workspace --locked --no-fail-fast` all passed, the content lib's 53 unit tests included and the four
recorded parity scripts (`content`, `content-none`, `content-invalid`, `content-index`) passing both in process
(`services/content/tests/parity.rs`) and through the built library (`services/content-plugin/tests/parity.rs`).
