# Clean-Room Provenance Record: `crates/rustic-isles-chat` (RD27)

- **Component**: `crates/rustic-isles-chat` (crate `rustic-isles-chat`, `license = "MIT"`, `Copyright (c) 2026 Silent
  Spud`)
- **Behaviour specification**: `docs/port/spec-rustic-isles-chat.md` (its closing "Coordinator decisions" override
  the rest), with `spec-rustic-isles-core.md` (§5.5, §7.5, §11, §17), `spec-rustic-isles-store.md` (the accounts,
  launcher accounts, friendships and parties repositories) and `spec-rustic-isles-auth.md` (§5.1, §11) for the
  contracts it depends on
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a55cc8531050f5375` (branch
  `feat/rustic-isles-chat`, reset to `feat/rustic-isles-clean-room` at `57495cb1`)
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, globbed or opened anything under `R:\source\dauntless\dainty-restricted` (any
worktree of it, including the session's additional working directory `splashloader-integration-90130d`),
`legacy-revived`, any `rustic-isles` or `rustic-rifts` tree outside dainty-lass, or any other repository. It ran no
`git show` or `git log -p` of another branch (only `git log --oneline` of `feat/rustic-isles-clean-room` to check its
base). It searched nothing on the web, did not look for golden files or captures, and did not open the `chat` plugin
crate's sources (the crate talks to chat only through `module-api`).

### 1.2 Other sources

None beyond this repository: no crate sources from the cargo registry were opened.

## 2. Files read

- The coordinator's brief; `AGENTS.md`; `docs/port/spec-rustic-isles-chat.md` (whole)
- `docs/port/spec-rustic-isles-core.md`: the headings, §2, §3.6, §3.9, §4.1, §5.1-§5.7, §7.5, §7.7, §7.8, §9.1-§9.2,
  §11, §13.2-§13.3, §17
- `docs/port/spec-rustic-isles-store.md`: the headings, §2.4, §3, §6.1, §6.2, §8, §11.2
- `docs/port/spec-rustic-isles-auth.md`: the headings, §4.1, §5.1, §6.1, §11
- `crates/rustic-isles/`: `Cargo.toml`, `src/lib.rs`, `handlers.rs`, `composer.rs`, `shared.rs`, `ids.rs`, `body.rs`
  (whole); `testing.rs` (head and its public items); `replay.rs` (lines 100-160); `settings.rs` (lines 175-200 and
  its public items); `pipeline.rs` (lines 150-200 and 380-460); `pattern.rs` (`Params`); `routes/table.rs` (the chat
  rows, by grep); `season.rs` and `routes/mod.rs` (public items by grep); `tests/pipeline.rs` (lines 1-80)
- `crates/module-api/src/`: `chat.rs`, `actor.rs`, `lib.rs` (whole); `ids.rs` (lines 1-38); `network.rs` (lines
  1-170, 355-520, 610-640, 800-960); `module.rs` (`ErrorKind`, `ModuleError`)
- `crates/rustic-isles-store/`: `Cargo.toml`, `src/lib.rs`, `repo/mod.rs`, `repo/chat.rs`, `test_support.rs`,
  `fuzzing.rs`, `tests/common/mod.rs` (whole); `repo/auth.rs` (the record and repository declarations, lines
  120-146 and 574-585); `repo/parties.rs` (`PartyRecord`, `NewParty`, the repository declaration); `adapter.rs`
  (lines 20-120, 420-434 and its function list); `tests/parties.rs` (by grep)
- `crates/rustic-isles-auth/`: `Cargo.toml` (features), `src/lib.rs` (lines 1-340), `game.rs` (lines 1-260),
  `policy.rs` (lines 1-110), `test_support.rs` (its public items, lines 55-86), `tokens.rs` (`TokenError`)
- `crates/service-kit/src/`: `json.rs` (lines 1-25), `body.rs` (`parse_form`, `BodyError::status`), `js.rs` (`trim`)
- `crates/store/src/database.rs` (`Tx::conn`, by grep); the root `Cargo.toml`
- `fuzz/Cargo.toml`, `fuzz/fuzz_targets/rustic_isles_store_decode.rs` (for the form)
- `docs/security/rustic-isles-store.md`, `docs/port/clean-room/rustic-isles-store.md` (format)

## 3. Files written

### 3.1 The crate, from the spec

| File | Spec |
| --- | --- |
| `Cargo.toml` | §9.1 (`serde_json` `preserve_order` in the crate's manifest) |
| `src/lib.rs` | §8, §9.2 (`XMPP_DOMAIN`, `HEADLINE_FROM`, `CLIENT_CREDENTIALS_USER_ID`, `ChatSettings`, `Player`, `ChatSource`, `TokenCheck`) |
| `src/ids.rs` | §4.3 (`room_id_text`, `instance_id`) |
| `src/hubs.rs` | §5.10 (`HubSessions`, `HUB_SESSION_TTL_MS`, `MAX_ACCOUNTS`) |
| `src/headline.rs` | §4.4, §5.11 (`event_name`, `public_event`, `frame`, the re-exports) |
| `src/login.rs` | §5.5 steps 1-5 (`Refusal`, `authenticate`) |
| `src/rules.rs` | §5.2-§5.8 (`ChatRules`, `IslesChat`, `room_kind_of`) |
| `src/source.rs` | §5.1, §7 (`StoreChat`) |
| `src/home.rs` | §5.9 (`home_operation`) |
| `src/routes.rs` | §6 (`register`, `register_with`, `Clock`, `include_pending`, `slayer_link_availability`, `slayer_link_status`, `SLAYER_LINK_MAX_ACCOUNTS`) |
| `tests/routes.rs`, `tests/rules.rs`, `tests/common/mod.rs` | §11.2's two tables |

### 3.2 Elsewhere

- `Cargo.toml`: `rustic-isles-chat` in `[workspace.dependencies]`; `Cargo.lock`. A seam change in its own commit.
- `fuzz/fuzz_targets/rustic_isles_chat_names.rs`, its `[[bin]]` and the dependency in `fuzz/Cargo.toml`,
  `fuzz/Cargo.lock`
- `docs/security/rustic-isles-chat.md`

## 4. Verification

`cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings` clean;
`cargo test -p rustic-isles-chat --locked --no-fail-fast`: 25 tests pass (10 unit, 5 route, 10 rule). The fuzz target
builds with `cargo check` in `fuzz/` but has not run: `cargo +nightly fuzz run rustic_isles_chat_names -s none` failed
to link on the implementer's Windows machine (`__start/__stop___sancov_pcs` unresolved, a toolchain issue, not the
target's), so its first run is CI's. Nothing was played against the 1.14.7 client; restricted's `tests/golden/chat/`
replay is still owed (spec §1.3).

## 5. Deviations and ambiguities, and how they were resolved

1. **The Slayer link body reader** (decision item 3: restricted's, with bare 400/413, no charset or encoding check, a
   BOM kept). The spec lists only those three differences from core's reader, so everything else follows core §3.9:
   the has-a-body rule, JSON and form bodies parsed, an empty JSON body read as `{}`, the first non-blank character
   `{` or `[`, a form's parameter limit answered with its bare status.
2. **The cross-node login's "else"** (§5.5): the forward of `chat.login` (step 2) is tried only when the token does
   not parse as a session ticket; a ticket that parses but does not verify, or verifies for another player, is "no
   player".
3. **Where the routes and the composer find the state**: the spec names no slot types. The routes read
   `Shared::slot::<IslesStore>()` (no slot is core's 501); the composer is expected to put an `Arc<HubSessions>` in a
   slot for matchmaking and to build `StoreChat` with `StoreChat::for_module(&shared, module_id, store)`.
4. **The source's shape**: the token check is `token_check() -> Option<&dyn TokenCheck>` (none without a store), with
   `TokenCheck::validate` answering the claims or `InvalidToken`. Public items beyond §9.2: `TokenCheck`,
   `InvalidToken`, `StoreChat::for_module`, `store()`, `module_id()`, `room_kind_of` (the `RoomOf` rule as a pure
   function, which the trait default and the fuzz target call), `ids::MAX_ID_TEXT`, `HubSessions::len`/`is_empty`,
   and `Display` for `Refusal`.
5. **The hub registry's key** is the account's `GlobalId`; `record(account, game_session_id, region, now)`.
6. **Eligibility** reads auth's `LauncherAccount` through `IslesAuthStore` (store §12.1's mapping, as §7 allows).
   `AccountOf`'s launcher account whose stored id does not parse as a global id is none.
7. **Order of the no-database and id-kind checks**: a lookup without a database is `Unsupported` whatever the id's
   kind.
8. **Trimming** (§8, §5.8) uses `service_kit::js::trim`, JavaScript's `String#trim` set, as Node trimmed.
9. **`home_operation`** answers `None` for every name but the three, so core answers `chat.anything` as an unknown
   operation; without a database it is `Unsupported` `chat without a database`; an invocation id that is not an
   account is `Invalid`; a store failure is `Failed` with the error's text.
10. **Remove** runs the caller's edge removals as one account write (listed and removed in one transaction), even when
    there are none.
11. **Dependencies**: `serde` and `thiserror` from §9.1's list are not needed; `ulid` (a UUID read as the ULID with
    the same bits), `http-body-util` (the body reader) and `tempfile` (tests) are existing workspace dependencies.

## 6. Similarity

Measured by the RD27 reviewer (who may read the AGPL code) with `similarity.md` §1's method, on the crate as committed
at `92af120c` (unchanged at `86da5583`): every `.rs` file in `src/` and `tests/` against each of the 12 files of
restricted `e0e238b` that hold its chat: `modules/rustic-isles/crates/isles-chat/src/` (`lib.rs`, `fake.rs`,
`headline.rs`, `hubs.rs`, `ids.rs`, `login.rs`, `rooms.rs`, `source.rs`), `isles-core/src/areas/chat.rs`,
`isles-core/src/areas/chat/rules.rs` and the tests that drive them, `isles-core/tests/chat_{golden,rules}.rs`, the
best single file named. "Any" counts a line matched by a 3+-line run in any of the 12. Paths in the third column are
under `modules/rustic-isles/crates/`; a file with no match names none.

| File | New lines | Best restricted file (old lines) | Matched | % | Any, % |
| --- | ---: | --- | ---: | ---: | ---: |
| `src/headline.rs` | 122 | `isles-chat/src/headline.rs` (85) | 20 | **16.4** | 23.8 |
| `src/home.rs` | 54 | none | 0 | **0.0** | 0.0 |
| `src/hubs.rs` | 128 | `isles-chat/src/hubs.rs` (146) | 16 | **12.5** | 13.3 |
| `src/ids.rs` | 113 | `isles-chat/src/ids.rs` (91) | 15 | **13.3** | 18.6 |
| `src/lib.rs` | 70 | `isles-chat/src/source.rs` (41) | 30 | **42.9** | 42.9 |
| `src/login.rs` | 51 | `isles-chat/src/login.rs` (98) | 21 | **41.2** | 41.2 |
| `src/routes.rs` | 407 | `isles-core/src/areas/chat.rs` (511) | 40 | **9.8** | 10.1 |
| `src/rules.rs` | 142 | `isles-chat/src/lib.rs` (110) | 38 | **26.8** | 33.8 |
| `src/source.rs` | 268 | `isles-core/src/areas/chat/rules.rs` (271) | 42 | **15.7** | 15.7 |
| `tests/common/mod.rs` | 113 | `isles-core/tests/chat_golden.rs` (297) | 3 | 2.7 | 2.7 |
| `tests/routes.rs` | 349 | `isles-core/tests/chat_golden.rs` (297) | 6 | 1.7 | 2.0 |
| `tests/rules.rs` | 589 | `isles-core/tests/chat_rules.rs` (217) | 24 | 4.1 | 7.6 |
| The 9 `src` files | 1355 | | 222 | **16.4** | 18.4 |
| The 3 test files | 1051 | | 33 | **3.1** | 5.2 |
| All 12 | 2406 | | 255 | **10.6** | 12.6 |

Three files reach 25 % against their best single file (`lib.rs` 42.9, `login.rs` 41.2, `rules.rs` 26.8); none other
does on the "any" column. The fuzz target (`fuzz/fuzz_targets/rustic_isles_chat_names.rs`) has no restricted
counterpart and was not measured.

**What the matches are.** Every run of 5 lines or more against any of the 12 files was read: 26 of them over 20
distinct spans of the crate, 7 of which (over 3 spans: `hubs.rs:106-112`, `ids.rs:54-60`, `routes.rs:407-414`) are, or
end in, the `} } #[cfg(test)] mod tests { use super::*;` boilerplate. The rest:

- **§9.2's records and traits** (`lib.rs`, 30 matched lines): `ChatSettings` with its two fields and `Default`
  off/on (`lib.rs:45-60`, the longest run, 13), `Player`'s three fields (`74-79`), and the `ChatSource` method heads
  that §9.2 lists (`node`, `settings`, `party_of`, `friends`, `93-118`) over `module_api`'s `BoxFuture` and
  `ModuleError`. `from_settings` (§8's table) and the token check do not match.
- **§5.5's refusals** (`login.rs`, 21): `Refusal`'s variants and `as_str` with §5.5's six reasons in its order
  (`login.rs:13-34`), and steps 3-4 as two early returns (`52-57`). Four variant names agree; each is the obvious
  name of its reason. The token and claim reading (steps 1-2) and step 5 do not match.
- **The rules' trait** (`rules.rs`, 38): §5.4's open and refused rules as two constants (`rules.rs:16-25`, the spec's
  own words for their names; the same pair in `tests/rules.rs:32-41`), §9.2's `answer` signature with its
  `Jid`/`Friends` arms (`32-53`), and the `room`, `login` and `friends` method heads (`69-75`, `96-102`, `123-129`;
  `friends`' default is the one call to the source). `room`'s body, `login`'s fallback chain, `room_kind_of` and
  `jid` do not match.
- **Store-spec records and calls** (`routes.rs`, `source.rs`): the friend list's JSON in §6's key order
  (`routes.rs:183-192`), `FriendEdgeRecord`'s literal (store §8, `204-212`), the `list_for_owner` read
  (`source.rs:301-306`), the `ChatSource` impl's `node`/`settings` getters (`205-212`), and `unavailable` with §5.1's
  `chat rules: {error}` warning and `Unavailable` kind (`54-57`).
- **Wire text and test vectors**: §5.11's event table (`headline.rs:13-19`), `public_event`'s §9.2 signature
  (`31-35`) and §11.2's headline vector (`87-91`); the temp-store `DatabaseConfig::Sqlite` literal
  (`tests/common/mod.rs:128-130`) and `tokio::test` heads.

No comment was copied: no comment line of 20 characters or more equals a restricted one. Every string literal of 12
characters or more that also occurs in the 12 files is in the chat, core, auth or store spec, except the tokio flavour
`multi_thread` and §11.2's clock written as a format string (`tests/routes.rs:51`) and one of its values (`172`). 40
of the crate's 127 function names also occur in restricted; they are §9.2's names, `module_api::chat`'s query names
(`jid`, `room`, `login`, `friends`, `may_whisper`, `room_of`, `account_of`), `std` trait methods, or plain words for a
spec item (`envelope`, `friends_list`, `unavailable`, `account_id`, `store`, the tests' `send`, and `Fake` for
§11.2's "fake token check"), except three private helpers: `live` (one obvious word) and the two below. The large files
(`routes.rs`, `source.rs`, the two test suites) match at 2 to 16 %, and the route handlers, the body reader, the
cross-node login, eligibility and the home operations share no logic beyond the lines above.

**The three spans that go beyond the spec.** None is 5 lines of copied logic; together they are the only private
choices the crate shares with restricted.

1. **`src/hubs.rs:31-37` and `66-74` (the registry's private layout).** §5.10 fixes the behaviour (replace and make
   newest, evict the oldest past 50,000) but not the structure. The private state has the same three field names
   (`by_account`, `order`, `next`), the same insertion-sequence `BTreeMap<u64, GlobalId>` with a counter, and an
   eviction loop (`while … len() > MAX_ACCOUNTS { let Some((_, oldest)) = ….pop_first() else { break }; ….remove(&oldest) }`)
   that matches line for line but for the binding's name, so the 3-line count misses it. Each is a stock choice for an
   insertion-ordered map in Rust; together they read as the same design.
2. **`src/ids.rs:40-41` (`uuid_bits`).** A private helper with the same name, signature and first line (splitting on
   `-` into a `Vec<&str>`). The rest of its body is written differently.
3. **`src/routes.rs:197-201` (`edge`).** A private helper building `FriendEdgeRecord` with the same name and its first
   four parameters in the same order and types (`owner`, `other: &AccountRecord`, `status`, `direction`). The body is
   store §8's record literal, which is fixed.

**Verdict: clean of structural or expressive copying, with three small rewrites asked for as a precaution.** The
files over 25 % are §9.2's records and trait heads, §5.5's reason table and §5.4's two rules in the only idiomatic
form; everything else is spec-given wire strings, store-spec records, `module-api`-fixed signatures, pinned test
vectors and boilerplate. The rewrites, from §5.10, §4.3 and §6 only and without looking at the current text:
`hubs.rs`'s registry state in the implementer's own design (for instance a different ordered structure or eviction,
or at least its own names for the state and its fields), `uuid_bits` under another name and shape, and the edge
builder under another name with its own parameter list. `lib.rs`, `login.rs` and `rules.rs` need nothing.
