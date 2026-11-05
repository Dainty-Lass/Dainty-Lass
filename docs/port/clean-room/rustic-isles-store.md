# Clean-Room Provenance Record: `crates/rustic-isles-store` (RD27)

- **Component**: `crates/rustic-isles-store` (crate `rustic-isles-store`, `license = "MIT"`, `Copyright (c) 2026 Silent
  Spud`)
- **Behaviour specification**: `docs/port/spec-rustic-isles-store.md` (its closing "Coordinator decisions" override
  the rest), with `spec-rustic-isles-core.md` and `spec-rustic-isles-auth.md` for the contracts it depends on
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a4980ec2a36549798` (branch
  `feat/rustic-isles-store`, from `feat/rustic-isles-clean-room` `7ad80284`, fast-forwarded to `6f529777` mid-task
  for core's `ids.rs` change)
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, globbed or opened anything under `R:\source\dauntless\dainty-restricted` (any
worktree of it, including the session's additional working directory `splashloader-integration-90130d`),
`legacy-revived`, any `rustic-isles` or `rustic-rifts` tree outside dainty-lass, or any other repository. It ran no
`git show` or `git log -p` of another branch (only `git log --oneline` of `feat/rustic-isles-clean-room` to check its
base). It searched nothing on the web and did not look for golden files or captures.

### 1.2 Other sources

None beyond this repository: no crate sources from the cargo registry were opened.

## 2. Files read

- The coordinator's brief; `AGENTS.md`; `docs/port/spec-rustic-isles-store.md` (whole)
- Other specs, by grep only, for a contract fact this spec names but does not spell out: `spec-rustic-isles-core.md`
  (where `IslesStore` is named), `spec-rustic-isles-reforged-admin.md` (the `set_email` request, lines 963-970, and its
  repository list), `spec-rustic-isles-characters.md`, `spec-rustic-isles-economy-progression.md`,
  `spec-rustic-isles-gamedata.md` (the shape of journey nodes: `node_status` under a node-id key), and the economy
  specs (which repository methods they call)
- `crates/store/`: `Cargo.toml`, `src/lib.rs`, `connection.rs`, `portable.rs`, `database.rs`, `engine.rs`,
  `declared.rs` (whole); `history.rs` (`ChangeEntry`, `Actor`, the soft-delete trigger op by grep);
  `tests/database.rs` (lines 1-150: the throwaway Postgres harness)
- `crates/module-api/src/`: `lib.rs` (exports), `actor.rs` (`Actor`, `ActorContext`), `network.rs` (`NodeIdentity`,
  `GlobalId`, `HomeError`, `HomeApi`, `StandaloneHome`)
- `crates/rustic-isles/`: `Cargo.toml`, `src/lib.rs` (head), `ids.rs` (before and after the merge), `replay.rs`
  (lines 1-170); `season.rs`'s `to_iso_string` by grep
- `crates/rustic-isles-auth/`: `Cargo.toml`, `src/store.rs` (whole), `lib.rs` (exports), `email.rs`
  (`CODE_MAX_ATTEMPTS` by grep)
- `crates/service-kit/src/jsjson.rs`, `js.rs` (public functions by grep)
- `fuzz/Cargo.toml`, `fuzz/fuzz_targets/rustic_isles_gamedata_formats.rs` (head, for the form)
- `docs/security/rustic-isles-auth.md` and `docs/port/clean-room/rustic-isles.md`, `rustic-isles-auth.md` (format)

## 3. Files written

### 3.1 The crate, from the spec

| File | Spec |
| --- | --- |
| `Cargo.toml` | §13.1 (`serde_json` `preserve_order`, `float_roundtrip`; feature `test-support`) |
| `migrations/{sqlite,postgres}/<15 folders>/{up,down}.sql` | §4 (written fresh from the tables; no comments, §16 item 13) |
| `src/lib.rs` | §2.3, §2.4, §3, §13.2 (`SCHEMA`, `SOFT_DELETE`, `IslesStore`, `store_actor`, `RepoError`) |
| `src/sql.rs` | §14's SQL rule: bound `?` queries numbered by `store::portable::numbered` |
| `src/repo/mod.rs` | §5.1 (`Repositories`, `Stub`, the repository macro whose defaults answer `Unimplemented`) |
| `src/repo/auth.rs` | §6 and the decisions' `set_email` |
| `src/repo/characters.rs`, `characters/slots.rs` | §7 |
| `src/repo/chat.rs` | §8 |
| `src/repo/inventory.rs` | §9 |
| `src/repo/progression.rs` | §10 |
| `src/repo/bounties.rs` | §11.1 |
| `src/repo/parties.rs` | §11.2 |
| `src/repo/reforged.rs` | §11.3 (with `erase_other_areas`, `purge_expired_ttl_rows`) |
| `src/adapter.rs` | §12 (`IslesAuthStore`) |
| `src/test_support.rs` | §13.2 (`TempStore`) |
| `src/fuzzing.rs` | §14's fuzz target (two private decoders exposed, `#[doc(hidden)]`) |
| `tests/{common/mod.rs,auth.rs,areas.rs,parties.rs,reforged.rs,adapter.rs,schema.rs}` | §15.2 rows S1-U1, plus §4's facts |

### 3.2 Elsewhere

- `Cargo.toml`: `rustic-isles-store` in `[workspace.dependencies]`; `Cargo.lock`. A seam change in its own commit.
- `fuzz/fuzz_targets/rustic_isles_store_decode.rs`, its `[[bin]]` and the dependency in `fuzz/Cargo.toml`,
  `fuzz/Cargo.lock`
- `docs/security/rustic-isles-store.md`

## 4. Verification

`cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` clean;
`cargo test -p rustic-isles-store --locked --no-fail-fast`: 42 tests pass (13 unit, 29 integration) on SQLite. No
Postgres server was reachable on the implementer's machine, so every test's Postgres half printed that it was
skipped: the Postgres migrations and SQL are unexercised until a run with `REFORGED_TEST_POSTGRES_URL` set.

## 5. Deviations and ambiguities, and how they were resolved

1. **Diesel's column limit.** The decision enables `32-column-tables`; the crate declares no Diesel tables at all
   (every query is bound SQL read through `QueryableByName`), so the feature is not needed and not enabled.
2. **Change-log op of a soft delete.** A7 says the soft delete is logged as `update`; `store`'s triggers name a
   `deleted_at` transition `soft_delete`. The test checks `soft_delete`.
3. **`open_configured`'s config error** has no plain `RepoError` variant; it is `Store(Config(text))`, whose display
   prefixes `database settings: `.
4. **Outbox `party` column.** The spec does not say which party id it holds. Events store the party's client id
   (Node's `partyId`, as `PartyEvent.party_id` is named); the erasure deletes an emptied party's outbox rows by either
   id.
5. **"At most `limit`"** where the spec does not say 0 is no limit (grants, activity, invites) is a literal `LIMIT`;
   the paged lists (`list_audit`, `list_flags`, `list_sessions`, `list_findings`) take 0 as no limit, as
   `list_for_admin` does.
6. **Unspecified shapes, chosen here:** `merge_objectives` answers `()`; `find_daily_rows_in_range` answers
   `(AccountId, document)` pairs; `WalletRecord::balance` answers `Option<f64>`; `close_hunt_if_open` takes the reason
   as text and the result as an option; `set_email` answers `CreateOutcome` (`EmailTaken` or `Created`); admin and
   Guard session accounts are `AccountId`s (data error when they do not parse); an activity flag's `detail` is JSON.
7. **Lenient reads chosen where the spec is silent:** a Guard `detail` that is not JSON reads as all parts empty;
   `draft_data` that is not an object reads as no drafts (not JSON is a data error); a `closedAt` of the wrong type
   reads as none.
8. **Postgres types.** Existence checks select `CAST(1 AS BIGINT)` so the same row type reads on both engines.
9. **Adapter registration name.** `create_launcher_account` runs §6.2 `create` and, when that wrote the game
   account, sets its name to `game_name` in the same write.
10. **The adapter's unverified delete** has no cut-off (decision): it needs the pending mark set and the row
    unverified.
11. **`rand`, `tracing`, `serde`** from §13.1's list are not dependencies: UUIDs come from core's
    `replay::random_uuid` (system randomness) and the crate logs nothing.

## 6. Similarity

To be measured by the coordinator against restricted `e0e238b`.
