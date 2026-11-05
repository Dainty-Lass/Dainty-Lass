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

To be measured by the coordinator against restricted `e0e238b`.
