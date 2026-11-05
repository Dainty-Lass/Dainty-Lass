# Clean-Room Provenance Record: `crates/rustic-isles-characters` (RD27)

- **Component**: `crates/rustic-isles-characters` (crate `rustic-isles-characters`, `license = "MIT"`, `Copyright (c)
  2026 Silent Spud`), and the `RuntimeCaller` handle in `crates/rustic-isles-auth/src/seams.rs`
- **Behaviour specification**: `docs/port/spec-rustic-isles-characters.md` (its closing "Coordinator decisions",
  with the 2026-11-05 amendment, override the rest), with `spec-rustic-isles-core.md` (§5, §9, §11, §17),
  `spec-rustic-isles-store.md` (the repositories it calls) and `spec-rustic-isles-auth.md` (§4.1, §11) for the
  contracts it depends on
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a1dbb7217314674e8` (branch
  `feat/rustic-isles-characters`, from `feat/rustic-isles-clean-room` `57495cb1`)
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, globbed or opened anything under `R:\source\dauntless\dainty-restricted` (any
worktree of it, including the session's additional working directory `splashloader-integration-90130d`),
`legacy-revived`, any `rustic-isles` or `rustic-rifts` tree outside dainty-lass, or any other repository. It ran no
`git show` or `git log -p` of another branch; the one `git show` it ran was of `3c90ae1e`, an ancestor on its own
branch (the store's seam commit), to see how a seam commit is laid out. It searched nothing on the web and did not
look for golden files or captures.

### 1.2 Other sources

None beyond this repository: no crate sources from the cargo registry were opened.

## 2. Files read

- The coordinator's brief; `AGENTS.md`; `docs/port/spec-rustic-isles-characters.md` (whole)
- `docs/port/spec-rustic-isles-core.md`: the section index, §5, §9, §11, §17; `spec-rustic-isles-store.md`: §2, §3,
  §6.1, §7, §9.1, §10.5, §11.1, §16; `spec-rustic-isles-auth.md`: the `RuntimeCaller` and `AuthData` lines by grep
- `crates/rustic-isles/`: `Cargo.toml`, `src/lib.rs`, `shared.rs`, `handlers.rs`, `version.rs`, `body.rs`,
  `replay.rs`, `testing.rs`, `pipeline.rs` (whole); `settings.rs` (readers), `ids.rs` (lines 1-80), `routes/table.rs`
  (the character and loadout rows by grep), `tests/pipeline.rs` (head and the fake guard, lines 100-240)
- `crates/rustic-isles-store/`: `Cargo.toml`, `src/lib.rs`, `test_support.rs`, `repo/mod.rs`, `repo/characters.rs`,
  `repo/characters/slots.rs` (whole); `repo/inventory.rs` (records, trait, `update_both_if_revision_matches`);
  `repo/auth.rs`, `repo/bounties.rs`, `repo/progression.rs` (the records and traits it calls, by grep);
  `adapter.rs` (`IslesAuthStore::new`, `gameserver_key_hashes`); the `0030` and `0050` SQLite migrations;
  `tests/common/mod.rs` (lines 100-200), `tests/areas.rs` (lines 265-285)
- `crates/rustic-isles-auth/`: `Cargo.toml`, `src/seams.rs` (whole), `lib.rs` (exports, `AuthData`, the token
  methods), `game.rs` (lines 1-420); `hashes.rs`, `policy.rs`, `limiter.rs`, `test_support.rs` by grep
- `crates/rifts-seam/src/`: `lib.rs`; `hooks.rs` (lines 210-265), `dedicated.rs` (`Rifts`, `RiftsParts`, the
  dedicated-only refusals), `http.rs` (`Route`, `Json`, `Reply`, `RuntimeAuth`, `StubRoutes`) by grep and range
- `crates/rustic-isles-gamedata/`: `Cargo.toml`, `src/lib.rs`, `src/areas.rs` (lines 1-120), `tests/routes.rs` (lines
  1-260), as the pattern of an area crate over core
- `crates/service-kit/src/`: `js.rs` (lines 1-140), `jsjson.rs` (number, stringify, `to_js`, parse), `json.rs` (head),
  `body.rs` (`error_page`); `crates/module-api/src/actor.rs` (`ActorContext`); `crates/store/src/database.rs` (`Tx`,
  lines 190-260), `history.rs` (`ChangeEntry`)
- `fuzz/Cargo.toml`, `fuzz/fuzz_targets/rustic_isles_store_decode.rs`
- `docs/security/rustic-isles-store.md` and `docs/port/clean-room/rustic-isles-store.md` (format)

## 3. Files written

### 3.1 The crate, from the spec

| File | Spec |
| --- | --- |
| `Cargo.toml` | §8.1 (`serde_json` `preserve_order`) |
| `src/lib.rs` | §6.1, §6.2, §8.2 (`CharacterData`, `StarterKit`, `CharactersError`, `EnsureError`, `routes`) |
| `src/routes.rs` | §3, §4, §11 (the eight handlers, the order of checks, the 409 page) |
| `src/controller.rs` | §4.1, §4.2, §5.1-§5.3, §5.6, §5.9-§5.12, §7.2 (`Characters` and its seam, `is_fresh_onboarding`) |
| `src/data.rs` | §2's dates, §5.4-§5.8, §5.12's validation |
| `src/starter.rs` | §6.3, §6.4, §7.1, §7.2's repair and reconcile |
| `tests/{common/mod.rs,characters.rs,loadouts.rs,data.rs}` | §13.2's rows, with synthetic fixtures (§13.1) |

### 3.2 Elsewhere

- `crates/rustic-isles-auth/src/seams.rs`: `RuntimeCaller` gains the opaque handle (`with_handle`, `handle`,
  `handle_as`), a hand-written `Debug` that shows only whether one is set, and equality by roster and handle
  identity, with a unit test. Its own commit (decision on §15 item 2).
- `Cargo.toml`: `rustic-isles-characters` in `[workspace.dependencies]`; `Cargo.lock`. A seam change in its own commit.
- `fuzz/fuzz_targets/rustic_isles_characters_data.rs`, its `[[bin]]` and the dependency in `fuzz/Cargo.toml`,
  `fuzz/Cargo.lock`
- `docs/security/rustic-isles-characters.md`

## 4. Verification

`cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` clean;
`cargo test -p rustic-isles-characters --locked --no-fail-fast`: 44 tests (7 unit, 37 integration) pass on SQLite;
`cargo test -p rustic-isles-auth --locked` passes with the new unit test. Postgres is not exercised (the harness opens
`TempStore`, SQLite only). Nothing was run against the 1.14.7 client.

## 5. Deviations and ambiguities, and how they were resolved

1. **The slot's appearance.** §6.4 puts the default appearance (a `CharacterData` value) in the slot, but §8.2's
   `build_manifest(&StarterKit, &VersionTables)` takes no `CharacterData`. `StarterKit` carries
   `default_appearance`, and `Characters::new` overwrites it with `CharacterData`'s so the two never disagree.
2. **`authoritativeTotalSlots`** (the unlock's log line and host-write summary) is the total derived from the
   journey, not the visible count after the monotonic grow.
3. **A soft-deleted starter row cannot be recreated.** Inventories and loadouts are keyed on the character id and the
   store refuses an insert over a soft-deleted key (store spec §16 item 3), so §13.2's "inventory row soft-deleted →
   GET loadout recreates it" is a database error (500 page) on this store. The repair tests remove the row outright;
   the weak spot is listed on the security page. A question for the coordinator: whether the repair should restore
   the row, or the soft delete is not how a row goes missing.
4. **The request id on the actor** is attached (`ActorContext::with_request`) as §3.1 says, but `store_actor` keeps
   only the actor, so the change log cannot show it; the actor test checks the names only.
5. **The loadout write reason** writes the index with `escape_debug()` (`unsupported loadout index a\nb`), so the
   error log stays one line; plain indexes read as the spec writes them.
6. **Data error texts the spec leaves open:** a trigger series that does not parse is `{trigger key}: {error}`; a
   manifest error is a `RepoError::Data` (`stored data: Missing inventory instance ID ...`), since §8.2 gives
   `build_manifest` a `RepoResult`; records gone after a repair is `starter records for {id} vanished`.
7. **`needs_repair` draws no ids**: it only lists what the reconcile would append, so a read with nothing to repair
   leaves the replay draw order and the change log untouched.
8. **Retries inside one transaction.** §5.10-§5.12 retry up to 10 times inside one `account_write`; writes are
   serial, so a revision conflict cannot clear within it. Kept as specified (only logs show it, §14 item 9).
9. **"No authority at all"** (§13.2's unlock row) cannot come through the app, since a request with no auth header
   answers 501; the test puts an empty `AuthData` in the request's extensions (§3.1 step 1).
10. **The POST version row** ("`"abc"` or 1.5 → 200 unchanged") holds for a character still at version 0; against a
    newer stored version a player's 1.5 is stale and refused (409) by §5.6's order, which the test also pins.
11. **The kept Dojo series** (§5.6 step 4.4) is read from the current wire data's member; when it is non-null the
    result goes through the read form with the flag on.
12. **Dates** are computed with a civil-from-days routine rather than `chrono`, so any `i64` second is arithmetic
    and never fails.
13. **A guard refusal's status** is answered when `http` accepts it (100-999), else 403.
14. **Reconcile log lines** join the added ids with `, `.

## 6. Similarity

To be measured by the coordinator against restricted `e0e238b`.
