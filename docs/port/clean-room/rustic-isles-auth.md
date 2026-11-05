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

To be measured by the coordinator against restricted `e0e238b` (`crates/isles-core/src/areas/auth.rs`,
`areas/auth/*` and `repo/auth.rs`) with `similarity.md` §1's method.
