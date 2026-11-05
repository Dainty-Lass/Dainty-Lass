# Clean-Room Provenance Record: `crates/rustic-isles-reforged` (RD27)

- **Component**: `crates/rustic-isles-reforged` (crate `rustic-isles-reforged`, `license = "MIT"`, `Copyright (c)
  2026 Silent Spud`)
- **Behaviour specifications**: `docs/port/spec-rustic-isles-reforged-public.md` and
  `docs/port/spec-rustic-isles-reforged-admin.md` at dainty-lass `fb3484f8` (branch `feat/rustic-isles-clean-room`),
  each with its closing coordinator decisions
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a30ed78069a0910d1` (branch
  `feat/rustic-isles-reforged`, reset to `feat/rustic-isles-clean-room` at `fb3484f8` before the work; later merged
  with that branch again to take matchmaking and db)
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator. The session was cut off once
  by a power loss and resumed in the same worktree; the files on disk were checked for truncation and the build
  artifacts cleaned.

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, globbed or opened anything under `R:\source\dauntless\dainty-restricted` (any
worktree of it, including the session's additional working directory `splashloader-integration-90130d`),
`legacy-revived`, any `rustic-isles` or `rustic-rifts` tree outside dainty-lass, or any other repository. It ran no
`git show` or `git log -p` of another branch (it looked at the stat of its own branch's commits only). It searched
nothing on the web. It did not look for the goldens the specs keep in dainty-restricted
(`tests/golden/reforged_api.json` and its capture script); the tests here are written from the specs' tables with
synthetic data and their own Ed25519 key pairs.

### 1.2 Other sources

None outside this repository.

## 2. Files read

- The two Reforged specs (whole), the brief, `AGENTS.md`; from `spec-rustic-isles-core.md` §3 (whole) and §17; from
  `spec-rustic-isles-store.md` §11.3 (whole); from `spec-rustic-isles-economy-progression.md` §9.4 to §10.4 and from
  `-inventory.md` §12.2 to §12.3 (the tester funding calls, by grep). The auth and chat specs were not read: their
  contracts were taken from the crates' public items below.
- `Cargo.toml` (workspace dependencies and lints), `fuzz/Cargo.toml`, `fuzz/fuzz_targets/rustic_isles_economy_text.rs`
- `crates/rustic-isles`: `lib.rs`, `shared.rs`, `handlers.rs`, `composer.rs`, `pipeline.rs`, `observe.rs`, `replay.rs`,
  `settings.rs`, `body.rs`, `testing.rs`, `ids.rs`, `game_data.rs` (first 120 lines), `module.rs` (lines 180-480), the
  public items of `season.rs`, `pattern.rs`, `version.rs`, `dedicated.rs`, the route table's rows (by grep),
  `tests/pipeline.rs` (lines 660-720)
- `crates/rustic-isles-store`: `Cargo.toml`, `lib.rs`, `repo/mod.rs`, `repo/reforged.rs` (records and traits),
  `repo/auth.rs` (records and traits), the traits and records of `repo/progression.rs`, `repo/characters.rs`,
  `repo/inventory.rs` (and its ledger row's JSON), `repo/bounties.rs`, `test_support.rs`
- `crates/rustic-isles-auth`: `Cargo.toml`, `lib.rs`, `seams.rs`, `store.rs` (first 200 lines), `launcher.rs` (lines
  70-290), `client.rs`, `errors.rs`, `totp.rs`, `launcher_build.rs` (first 110 lines), `limiter.rs` (lines 100-150),
  `hashes.rs` (first 80 lines), `discord.rs`, `util.rs`, the public items of `test_support.rs` and `password.rs`
- `crates/rustic-isles-economy`: `Cargo.toml`, `lib.rs`, `inventory/mod.rs`, `inventory/ports.rs`,
  `inventory/transaction.rs` (first 330 lines), `progression/mod.rs` (first 80 lines), `progression/tracks.rs` (lines
  60-330), `bounties/mod.rs` (first 80 lines), `util.rs` (first 80 lines), the public items of `storage_policy.rs` and
  `platinum.rs`, `tests/common/mod.rs` (the test harness here follows its shape)
- `crates/rustic-isles-chat`: `Cargo.toml`, `lib.rs`, `home.rs` (first 80 lines), `source.rs` (lines 75-120)
- `crates/rustic-isles-matchmaking` (after the merge): `state.rs` (first 60 lines), the lines naming `HuntActivity`,
  `GamePresence`, `has_candidate` (by grep)
- `crates/module-api`: `module.rs` (admin calls, restrictions), `network.rs` (`HomeInvocation`, `OperationName`,
  `HomeApi`'s registration calls)
- `crates/service-kit`: `js.rs`, `jsjson.rs` (parts), `json.rs`, `body.rs` (first 60 lines)
- `crates/protocol/src/status.rs` (the status structs), `crates/admin/src/upstream.rs` (the operator signature and
  `Operator`), `crates/admin/Cargo.toml`, `crates/admin/src/lib.rs`'s module list
- `docs/security/rustic-isles-economy.md` (first 40 lines) and `docs/port/clean-room/rustic-isles-economy.md` (first
  60 lines) for the formats

## 3. Files written

- `crates/rustic-isles-reforged/` (the crate, both halves, its unit tests, `tests/common/mod.rs`, `tests/public.rs`,
  `tests/admin.rs`)
- `fuzz/fuzz_targets/rustic_isles_reforged_{guard,text,paths,admin_auth,admin_text}.rs` and their `fuzz/Cargo.toml`
  entries (`rustic-isles` there now with `test-support`)
- `docs/security/rustic-isles-reforged.md`, this record
- Outside the crate: `crates/rustic-isles/src/{shared,lib,pipeline}.rs` and `tests/pipeline.rs` (the authenticator
  change, its own commit), the root `Cargo.toml`'s `[workspace.dependencies]` (its own commit)

## 4. Deviations, ambiguities and how they were resolved

1. **Core's `AdminAuthenticator`** took only the headers and peer; the operator credential signs the method, target
   and body. The trait now takes an `AdminRequestHead` (method, target, peer, headers, body) and the pipeline reads a
   relayed `/admin/` row's body before asking it (a seam change in core, its own commit). Without a store the slot
   refuses as an empty slot does (core's `admin_refusal`), since the trait cannot answer "anonymous".
2. **`ServerStatus` key order**: the public spec's table order (its §2 says every key order is the wire order), not
   `crates/protocol`'s `ServerStatus` field order, which differs. Flagged for the golden replay.
3. **Quiet request-log routes** are `/rustic/v1/hunts/<id>/lease|event-batches` (core's D3 cutover, no aliases); the
   spec's text still names `/mystic/v1/hunts`.
4. **`/health/ready`'s trivial query** is the operations policy row read through the repositories (the crate does not
   issue raw SQL).
5. **`ReforgedAdmin::new(discord, funding)`** takes no `Shared` (the composer fills the slots before `Shared`
   exists); its methods take the module's `Shared`. `ReforgedSeams` and `ActivityHooks` are bound at start-up for the
   same reason (`public_startups` binds them from the slots). `with_chat` hands it the chat crate's `StoreChat` for
   `chat.*`.
6. **Tester funding** is the economy crate's (`EconomyFunding`: approved tracks, caps, curves, the storage policy,
   `run_inventory_transaction` with operator funding), per the admin decisions' item 12; `TesterFunding` stays a trait
   so tests use a fake.
7. **Matchmaking (after the merge)**: `ActivityHooks` implements its `HuntActivity`; `MatchmakingState` fills the
   `MatchmakingPresence` and `MatchmakingCandidates` slots.
8. **Discord bootstrap** creates only the launcher account (the spec names no game-account row); its userId is then
   the account id text until a game account exists.
9. **A progression step's failure** stops that track's remaining blocks (the spec says only that the row is marked
   `error`).
10. **The upload body read** first swapped the whole request out, losing the peer for the publisher and upload checks;
    found by the black-box tests and fixed with the admin tests' commit.
11. **TOTP in the step-up** reads the system clock (a trust decision), while its attempt window runs on core's clock
    (the public decisions).

## 5. Similarity

To be measured by the coordinator against restricted e0e238b.
