# Clean-Room Provenance Record: `crates/rustic-isles-auth` (RD27)

- **Component**: `crates/rustic-isles-auth` (crate `rustic-isles-auth`, `license = "MIT"`, `Copyright (c) 2026 Silent
  Spud`)
- **Behaviour specification**: `docs/port/spec-rustic-isles-auth.md` at dainty-lass `7851ea5` (branch
  `feat/isles-auth-clean-room`), §15's coordinator decisions included
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\hopeful-volhard-1b89b9` (branch
  `feat/isles-auth-clean-room`), shared with other agents writing `docs/port/spec-*.md`
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, globbed or opened anything under `R:\source\dauntless\dainty-restricted` (any
worktree of it, including the session's additional working directory `splashloader-integration-90130d`),
`legacy-revived`, any `rustic-isles` or `rustic-rifts` tree, or any other repository. It ran no `git show` or `git log
-p` of another branch. It searched nothing on the web. It did not read, copy or look for the golden files §12.1 names
(`golden.json`, `scenario.json`); §15 keeps them in dainty-restricted, and the tests here are written from §12's
tables. Its inputs were the spec, `AGENTS.md`, the MIT crates in this repository, this folder's `chat.md` for the
record's format, and `docs/security/gateway.md` for the checklist's.

### 1.2 Other sources

Only published crates' sources in the local cargo registry, read for their public API: `aws-lc-rs` 1.18.2
(`rsa/key.rs`, `rsa/signature.rs`, `rsa.rs` exports, `hmac.rs`), `argon2` 0.5.3 (`Cargo.toml` features) and
`password-hash` 0.5.0 (`salt.rs` constructors).

## 2. Files read

- `docs/port/spec-rustic-isles-auth.md` (whole), `AGENTS.md`, `docs/port/clean-room/chat.md`, `docs/security/gateway.md`
- `Cargo.toml`, `Cargo.lock` (versions of `aws-lc-rs`, `base64`, `digest`, `getrandom`, `hmac`, `rand`, `rand_core`,
  `sha2`, `subtle`, `uuid`, `webpki-roots` only), `rust-toolchain.toml`
- `crates/module-api/src/{lib.rs, actor.rs}`, `crates/module-api/Cargo.toml`; from `network.rs` `NodeIdentity`,
  `GlobalId`, `SessionTicket::from_token`, `HomeError`, `HomeApi` and `StandaloneHome`; from `version.rs`
  `GameVersion`; the `BoxFuture` line of `module.rs`; `ids.rs`' public items (by grep)
- `crates/service-kit/src/{lib.rs, json.rs, body.rs, routing.rs, js.rs}`, `serve.rs` (first 45 lines),
  `collate.rs` (first 60 lines), `jsjson.rs` (lines 140-215), `ratelimit.rs` (lines 170-195), every module's public
  items (by grep), `crates/service-kit/Cargo.toml`
- `crates/chat/Cargo.toml`, the first lines of `crates/chat/src/lib.rs` (header style); how `rand` 0.10 and
  `aws_lc_rs::rand` are called (one grep over `crates/`)
- `fuzz/Cargo.toml` (header, the dependency list and the last `[[bin]]`), `fuzz/fuzz_targets/protocol_input.rs`

## 3. Files written

### 3.1 The crate, from the spec

| File | Spec |
| --- | --- |
| `src/lib.rs` | §11.2 (`AuthSettings`, `Clock`, `IdSource`, `NoPlayerPolicy`, `AuthConfig`, `AuthData`, `Auth`, tokens on `Auth`), §4.4 (`rifts_caller_kind`) |
| `src/util.rs` | §2.1, §2.2 (bodies, queries, answers) |
| `src/errors.rs` | §2.3 |
| `src/store.rs` | §9 (records, `AuthStore`, the `Arc<T>` forwarding impl) |
| `src/seams.rs` | §11.2 (`Seams` and its traits), §3.4.9 notices |
| `src/game.rs` | §3.1-§3.3, §4.1 |
| `src/launcher.rs` | §3.4, §4.2, §4.3, §4.5, §7.3, §7.5.1-§7.5.4 |
| `src/tokens.rs` | §5.1 |
| `src/hashes.rs` | §5.3 |
| `src/password.rs` | §5.4 |
| `src/totp.rs` | §5.5 |
| `src/enckeys.rs` | §5.6 |
| `src/policy.rs` | §6.1-§6.5 |
| `src/runtime.rs` | §6.6 |
| `src/launcher_build.rs` | §6.7 |
| `src/email.rs` | §7.1, §7.3 |
| `src/mail.rs` | §7.2, §7.4 (the mail's copy and HTML are this implementer's own, §15 item 7) |
| `src/discord.rs` | §7.5, §7.5.5 |
| `src/client.rs` | §4.6 |
| `src/limiter.rs` | §4.7, §10.1 (`BudgetConfig`) |
| `src/jobs.rs` | §8 |
| `src/routes.rs` | §3 (the 49 routes), §11.2 (`handle`, `router`) |
| `src/test_support.rs` | §11.2 (`MemoryStore`, `RecordingMail`, `StubDiscord`), plus `MapSettings` and a throwaway key pair |
| `tests/{common/mod.rs, launcher.rs, game.rs, jobs.rs}` | §12.2's table and §12.3's rows, black-box through `Auth::router()` |

### 3.2 Elsewhere

- `Cargo.toml`: `argon2` (0.5, `alloc` and `password-hash` only), `webpki-roots` (1) and `rustic-isles-auth` in
  `[workspace.dependencies]`; `Cargo.lock`. A seam change for the integration pass, in a commit of its own.
- `fuzz/fuzz_targets/rustic_isles_auth_{text,tokens}.rs`, their `[[bin]]` entries and the `rustic-isles-auth` and
  `axum` dependencies in `fuzz/Cargo.toml`, `fuzz/Cargo.lock`
- `docs/security/rustic-isles-auth.md`, this record

## 4. Verification

- `cargo test -p rustic-isles-auth --locked --no-fail-fast`: 47 unit tests and 46 black-box tests (`launcher` 24,
  `game` 11, `jobs` 11), 0 failed
- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean
- `cargo check --bins` in `fuzz/`: clean (not fuzzed; the targets need nightly)
- Every vector §12.3 calls "computed" was checked against this implementation and agreed: the three HMACs, the two
  `pk` values, the core artifact-set digest and the fingerprint `ee5827e0de42`; so did the RFC vectors (SHA-256 of
  `abc`, RFC 6238's `287082`, RFC 7636's challenge)
- Not run against the real 1.14.7 client, a launcher, a real SMTP server or Discord, and not against the golden
  replay, which runs in dainty-restricted once the isles adapter lands (§15 item 6)

## 5. Deviations and ambiguities, and how they were resolved

1. **`CONSENT_REQUIRED`'s status.** §2.3's table gives 400 (the registration refusal), while §4.3, §6.2 and §3.4.10
   say the enforced refusal is 403. The enforced one carries an explicit 403; registration's keeps the table's 400.
2. **Realm ids.** §3.2 step 4 ("does not parse as a realm id", 503) and step 5 (the bind fails, 503) answer the same.
   The crate leaves parsing to the realms seam: `Realms::bind_realm_presence` refuses an id it can't parse.
3. **The player host runtime caller** (§4.1 step 2) has no userId and `is_gameserver` false; only its roster is
   carried.
4. **An empty `authorization` header** with a game server key counts as absent (Node's falsy empty string).
5. **Roster lookups and unknown tokens.** §5.6 has the fallback flag "unknown token"; §12.3 has roster lookups
   "flag ... unknown tokens". A roster lookup flags a given token that names no live key, and still never falls back.
6. **The token normalizer's invariant.** §13 says a normalized token has no leading `bearer` and a space, but §5.6
   removes one prefix only (`bearer bearer x` gives `bearer x`). The rule is kept; the fuzz target asserts one
   removal and the trim instead.
7. **TOTP with an empty secret** (only white space, `=` or `-`) refuses.
8. **Registration consent `null`.** §6.2's rule (not an object: refused when required) is applied; §12.3's "`null`
   none" is read as the SQLite case.
9. **Store ids and the player actor.** Records hold account ids as text; a player write parses the text as a
   `GlobalId` and a text that doesn't parse is a store error (`INTERNAL`), as DD17's refusal surfaces.
10. **`AuthStore` additions** the spec's §9.3 list implies but doesn't name: `count_email_code_try` takes the try
    limit, `reset_password` takes one time for both `email_verified_at` and the revocation, and
    `NewLauncherAccount` carries the game account's name. `ErasureCounts` doesn't count refresh sessions (§8.2 deletes
    them but lists no count).
11. **Bodies the crate parses itself** (no `ExpressBody` from the pipeline) are absent without a `content-length` or
    `transfer-encoding` header, as Express's `hasBody` reads it, and a leading BOM is skipped before the JSON check.
12. **Mail send slots** are checked as a chain (§4.7): a refusal stops at the first window that refuses, and the
    windows before it keep their attempt.
13. **The redirect's HTML choice** reads `*/*`'s position only when `text/plain` is absent.
14. **No pre-rename names** (§15 item 9): neither the `mysticparadox-*` audiences and issuer nor the
    `MysticParadox.dll` alias is accepted.
15. **Coordinator addition:** `impl<T: AuthStore + ?Sized> AuthStore for Arc<T>`, so the core can hold
    `Auth<Arc<dyn AuthStore>>`; the trait is dyn-compatible (no generic methods), and a test builds one.

## 6. Similarity

Measured by the RD27 reviewer (who may read the AGPL code) with `similarity.md` §1's method, on the crate as committed
at `b4172b7`: every `.rs` file in `src/` and `tests/` against each of the 28 files of restricted `e0e238b`'s
`modules/rustic-isles/crates/isles-core/` that hold auth (`src/areas/auth.rs`, the 22 files of `src/areas/auth/`,
`src/repo/auth.rs`, and `tests/auth_{flows,golden,reforged_seams,repos}.rs`), the best single file named. "Any" counts
a line matched by a 3+-line run in any of the 28. Paths in the third column are under `isles-core/src/` unless they
start with `tests/`.

| File | New lines | Best restricted file (old lines) | Matched | % | Any, % |
| --- | ---: | --- | ---: | ---: | ---: |
| `src/lib.rs` | 326 | `areas/auth/policy.rs` (1300) | 15 | **4.6** | 8.3 |
| `src/util.rs` | 321 | `areas/auth/discord_flow.rs` (401) | 18 | **5.6** | 11.5 |
| `src/errors.rs` | 204 | `areas/auth/errors.rs` (211) | 47 | **23.0** | 23.5 |
| `src/store.rs` | 745 | `repo/auth.rs` (1655) | 127 | **17.0** | 17.6 |
| `src/seams.rs` | 183 | `areas/auth/runtime.rs` (652) | 33 | **18.0** | 18.6 |
| `src/game.rs` | 678 | `areas/auth/game.rs` (746) | 36 | **5.3** | 9.4 |
| `src/launcher.rs` | 1371 | `areas/auth/discord_flow.rs` (401) | 42 | **3.1** | 8.8 |
| `src/tokens.rs` | 245 | `areas/auth/tokens.rs` (323) | 6 | **2.4** | 2.4 |
| `src/hashes.rs` | 124 | `areas/auth/crypto.rs` (250) | 16 | **12.9** | 16.1 |
| `src/password.rs` | 66 | `areas/auth/launcher.rs` (765) | 10 | **15.2** | 15.2 |
| `src/totp.rs` | 78 | `areas/auth/totp.rs` (85) | 18 | **23.1** | 23.1 |
| `src/enckeys.rs` | 160 | `areas/auth/runtime.rs` (652) | 18 | **11.3** | 16.9 |
| `src/policy.rs` | 557 | `areas/auth/policy.rs` (1300) | 138 | **24.8** | 27.1 |
| `src/runtime.rs` | 379 | `areas/auth/runtime_authorization.rs` (543) | 62 | **16.4** | 19.8 |
| `src/launcher_build.rs` | 315 | `areas/auth/policy.rs` (1300) | 52 | **16.5** | 16.8 |
| `src/email.rs` | 66 | `areas/auth/crypto.rs` (250) | 16 | **24.2** | 25.8 |
| `src/mail.rs` | 552 | `areas/auth/mail.rs` (676) | 48 | **8.7** | 9.6 |
| `src/discord.rs` | 233 | `areas/auth/discord.rs` (164) | 58 | **24.9** | 30.0 |
| `src/client.rs` | 90 | `areas/auth/util.rs` (167) | 19 | **21.1** | 24.4 |
| `src/limiter.rs` | 249 | `areas/auth/runtime.rs` (652) | 31 | **12.4** | 14.1 |
| `src/jobs.rs` | 176 | `areas/auth/erasure.rs` (240) | 19 | **10.8** | 19.3 |
| `src/routes.rs` | 384 | `areas/auth/game.rs` (746) | 9 | **2.3** | 3.1 |
| `src/test_support.rs` | 758 | `areas/auth/runtime.rs` (652) | 30 | **4.0** | 6.6 |
| `tests/common/mod.rs` | 241 | `areas/auth/test_support.rs` (400) | 6 | 2.5 | 2.5 |
| `tests/launcher.rs` | 1165 | `tests/auth_flows.rs` (699) | 15 | 1.3 | 2.1 |
| `tests/game.rs` | 518 | `areas/auth/runtime_authorization.rs` (543) | 12 | 2.3 | 5.2 |
| `tests/jobs.rs` | 383 | `areas/auth/erasure.rs` (240) | 17 | 4.4 | 6.8 |
| The 23 `src` files | 8260 | | 868 | **10.5** | 13.5 |
| The 4 test files | 2307 | | 50 | **2.2** | 3.6 |
| All 27 | 10567 | | 918 | **8.7** | 11.3 |

No file reaches 25 % against its best single file; three do on the "any" column (`discord.rs` 30.0, `policy.rs` 27.1,
`email.rs` 25.8). The fuzz targets (`fuzz/fuzz_targets/rustic_isles_auth_{text,tokens}.rs`) have no restricted
counterpart and were not measured.

**What the matches are.** Every run of 5 lines or more against any of the 28 files was read: 136 of them over 87
distinct spans of the crate, 68 of which are the `} } #[cfg(test)] mod tests { use super::*; #[test]` boilerplate.
The rest:

- **Word lists and tables** (`policy.rs`, 138 matched lines): the offensive, impersonation and reserved name lists
  (runs of 34, 13 and 19 lines at `policy.rs:157-228`), the digit-to-letter map (`policy.rs:266-277`), and the
  username test vectors (`fox-x`, `Mystic_Fox`...). §6.4 gives every term and the map, one per line after rustfmt.
  The code around them differs: restricted builds the forms in a closure and sorts them, the crate has a free `leet`
  and keeps first-seen order; `collapse` is written differently (and restricted's line-terminator case is absent).
- **Records and enums §9 and §11.2 name** (`store.rs`, 127; `seams.rs`, 33; `limiter.rs`, `enckeys.rs`, `jobs.rs`):
  field lists of `RefreshSession`, `ExchangeCode`, `DiscordTransaction`, `EmailCode`, the consent and timeout
  records, the `as_str`/`parse` matches of `Status`, `Approval`, `Channel` and `Purpose` over their wire strings,
  `BudgetVerdict`'s four fields, `ErasureOutcome`'s variants, and `OperationsPolicy` with §4.3's
  `ENFORCE`/`PRODUCTION`/0 default (`seams.rs:40-58`, 17 lines). Two writers of §9.2's column table give the same
  `pub x: String` lines. The store trait, the memory store and restricted's SQL repository share no logic.
- **The error envelope** (`errors.rs`, 47): `LauncherError`'s four fields (§11.2), `new`, `with_status`,
  `with_detail`, `status()` and the head of `body()` (`errors.rs:23-59`, runs of 16 and 9 lines). The builder
  methods are not named in the spec, but the fields, the precedence (§2.3: the error's own status, else the table)
  and the details-first order are, and a `with_*(mut self, ..) -> Self` builder over those fields is Rust's stock
  form. The status table itself does not match: restricted's is a `(code, status)` array in Node's order, the
  crate's a `match` grouping §2.3's codes by status. The unit test `details_come_first_and_never_replace_the_fixed_fields` shares
  restricted's name and §12.3's vector; the name restates §12.3's row and the bodies differ.
- **Wire text and test vectors**: refusal messages (`runtime.rs:27-31, 65-71, 131-135, 238-243`, `policy.rs:65-70`,
  `launcher.rs:978-982`), the redirect body (`launcher.rs:1460-1464`), `cache-control` (`launcher.rs:1296-1300`), the
  `[EmailSweep]` log line (`jobs.rs:130-137`), the SMTP setting names (`mail.rs:464-468`), Discord's token form
  fields (`discord.rs:205-209`, §7.5), and §12.3's vectors for `mask_email`, `hashes_equal`, TOTP and
  `version_at_least` (`email.rs:65-69`, `hashes.rs:135-139`, `totp.rs:67-92`, `launcher_build.rs:252-257`).
- **Signatures §11.2 gives**: `DiscordConfig` with its secret-free `Debug` (§13; restricted's also hides
  `redirect_uri`), the `DiscordApi::exchange_code` and `MailMessage` shapes, `Cors`'s three variants, `CallerKind`'s
  four, `verify_password_blocking` (`password.rs:66-73`, the one way to run §5.4's "off the async threads" with
  `spawn_blocking`).
- **Idiom**: the four lines that read a header's values as Latin-1 text (`util.rs:135-139`; the joining after them
  differs), the preflight answer copying headers onto a 204 (`launcher.rs:171-176`), the poisoned-lock counters
  (`limiter.rs:182-196`), the semver core check (`runtime.rs:92-96`, five lines of §6.6's regex `\d+\.\d+\.\d+`; the
  suffix check differs).

No comment was copied: of the crate's comment lines, two short ones equal a restricted line (`/// Not a launcher path,
or no Origin` at `launcher.rs:79`, `/// Also clears the pending mark` at `store.rs:381`), both describing a §4.5 and a
§9.3 rule in the obvious words. 134 of the crate's 491 function names also occur in restricted; nearly all are the
spec's own (§11.2's free functions and jobs, route handlers named after §3's paths, `as_str`, `from_settings`). The
few the spec doesn't spell (`with_status`, `with_detail`, `unhex`, `code_at`, `has_bearer_prefix`, `unsupported`,
`note_sign_in`) are the plain name for what the spec describes, and their bodies differ except as above. No unusual
structure is shared: the crate is laid out by spec section (§11.2's module table), restricted by Node file, and the
large files (`launcher.rs`, `game.rs`, `routes.rs`, `test_support.rs`, the black-box tests) match at 1 to 5 %.

**Verdict: clean.** The matches are spec-given wire strings, word lists, status and record tables, pinned test
vectors, public signatures, and boilerplate; none is expressive or structural copying, and no span needs a rewrite.
The closest to the line is `errors.rs:23-59`, the builder over §11.2's `LauncherError`, which is the idiomatic form
of what the spec fixes.
