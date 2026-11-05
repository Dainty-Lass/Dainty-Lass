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

Measured by the RD27 reviewer (who may read the AGPL code) with `similarity.md` §1's method, on the crate as committed
at `f0d7b376`: every `.rs` file in `src/` and `tests/` against each of the 39 Rust files of restricted `e0e238b`'s
`modules/rustic-isles/crates/isles-core/` that hold or exercise the database layer (`src/db.rs`, the 13 files of
`src/repo/`, and all 25 files of `tests/`, the support modules included), the best single file named; and separately
every migration `.sql` file against the 60 of restricted's `migrations/{sqlite,postgres}/`, with `--` lines as the
comments. "Any" counts a line matched by a 3+-line run in any file of the set. Paths in the third column are under
`isles-core/`; a file with no match names none.

### 6.1 Rust

| File | New lines | Best restricted file (old lines) | Matched | % | Any, % |
| --- | ---: | --- | ---: | ---: | ---: |
| `src/adapter.rs` | 914 | `tests/reforged_admin.rs` (1371) | 12 | **1.3** | 2.8 |
| `src/fuzzing.rs` | 8 | none | 0 | **0.0** | 0.0 |
| `src/lib.rs` | 189 | `src/db.rs` (293) | 65 | **34.4** | 49.7 |
| `src/repo/auth.rs` | 1777 | `src/repo/auth.rs` (1655) | 560 | **31.5** | 34.0 |
| `src/repo/bounties.rs` | 710 | `src/repo/bounties.rs` (732) | 192 | **27.0** | 28.3 |
| `src/repo/characters.rs` | 363 | `src/repo/characters.rs` (365) | 114 | **31.4** | 33.3 |
| `src/repo/characters/slots.rs` | 148 | `src/repo/characters/slots.rs` (148) | 48 | **32.4** | 33.1 |
| `src/repo/chat.rs` | 178 | `src/repo/chat.rs` (181) | 60 | **33.7** | 38.8 |
| `src/repo/inventory.rs` | 334 | `src/repo/inventory.rs` (371) | 110 | **32.9** | 36.8 |
| `src/repo/mod.rs` | 134 | `src/repo/mod.rs` (170) | 71 | **53.0** | 53.0 |
| `src/repo/parties.rs` | 905 | `src/repo/parties/sql.rs` (861) | 122 | **13.5** | 24.6 |
| `src/repo/progression.rs` | 1097 | `src/repo/progression/sql.rs` (970) | 279 | **25.4** | 34.5 |
| `src/repo/reforged.rs` | 1952 | `src/repo/reforged.rs` (2248) | 503 | **25.8** | 26.9 |
| `src/sql.rs` | 121 | none | 0 | **0.0** | 0.0 |
| `src/test_support.rs` | 26 | `tests/chat_golden.rs` (297) | 3 | **11.5** | 11.5 |
| `tests/adapter.rs` | 252 | `tests/matchmaking_controller.rs` (805) | 30 | 11.9 | 18.7 |
| `tests/areas.rs` | 763 | `tests/bounties_repos.rs` (245) | 89 | 11.7 | 20.6 |
| `tests/auth.rs` | 694 | `tests/auth_repos.rs` (806) | 87 | 12.5 | 18.2 |
| `tests/common/mod.rs` | 154 | `tests/auth_repos.rs` (806) | 34 | 22.1 | 42.9 |
| `tests/parties.rs` | 361 | `tests/auth_repos.rs` (806) | 56 | 15.5 | 19.7 |
| `tests/reforged.rs` | 555 | `tests/auth_repos.rs` (806) | 43 | 7.7 | 12.1 |
| `tests/schema.rs` | 122 | `tests/auth_repos.rs` (806) | 11 | 9.0 | 9.0 |
| The 15 `src` files | 8856 | | 2139 | **24.2** | 28.1 |
| The 7 test files | 2901 | | 350 | **12.1** | 18.8 |
| All 22 | 11757 | | 2489 | **21.2** | 25.8 |

Ten files reach 25 % against their best single file: `repo/mod.rs` 53.0, `lib.rs` 34.4, `repo/chat.rs` 33.7,
`repo/inventory.rs` 32.9, `repo/characters/slots.rs` 32.4, `repo/auth.rs` 31.5, `repo/characters.rs` 31.4,
`repo/bounties.rs` 27.0, `repo/reforged.rs` 25.8 and `repo/progression.rs` 25.4. `tests/common/mod.rs` also does on
the "any" column (42.9). The fuzz target (`fuzz/fuzz_targets/rustic_isles_store_decode.rs`) has no restricted
counterpart and was not measured.

**What the matches are.** Every run of 5 lines or more against any of the 39 files was read: 232 of them over 226
distinct spans of the crate. None contains SQL: not one line of a query string, bind chain or row decoder's column
list matches (the crate's statements go through its own `sql()` builder, `src/sql.rs`, 0 %). The runs are:

- **Records and enums the spec lists field by field** (most of every repository file's matched lines): the record
  structs in §6 to §11's field order (`LauncherAccountRecord`, `RefreshSessionRecord`, `PartyOutboxRecord`,
  `NewInvite`, `AcceptInvite`, `PartyEvent`, `ActivityHuntRecord`, `SlayersPathUnlockRecord`, `FriendEdgeRecord`,
  `RewardCredit`, `StoredBounty`, `AdminAccountFilter`, `AccessChange`; e.g. `repo/auth.rs:60-143`, a run of 76;
  `repo/parties.rs:135-186`, 48; `repo/reforged.rs:163-202`, 37; `repo/bounties.rs:31-64`, 31), and each text enum
  with its `as_str`/`parse` over §4.12's strings. The redacting `Debug` impls (`repo/auth.rs:131-143`, `194-202`)
  print exactly the fields §6.2 and §6.4 name, the hash as `<set>` where §6.2 says so; `timeout_at` is §6.2's sentence.
- **Trait method lists and impl signatures**: the `repository!` declarations and every `impl ... for Sql*` method
  head, with §5 to §11's names, arguments and order. The long tail of 5- to 10-line runs is these heads (e.g.
  `repo/characters.rs:48-94`, `repo/reforged.rs:223-280`, `repo/progression.rs:152-283`), usually joined to the
  previous method's `Ok(()) }`.
- **`Repositories` and its constructors** (`repo/mod.rs:47-81`, 34; `86-113`): §5.1's thirty fields in its table's
  order, and `on_store()`'s `Arc::new(<module>::Sql<Name>)` entries. The unit type names are `Sql` plus the field's
  name (`SqlLauncherAccounts`, `SqlEconomyRollups`, one `SqlProgression` for §5.1's six progression traits): a rule
  derivable from the table, which does not name them.
- **`lib.rs`'s seam**: `SOFT_DELETE` (§2.2's sixteen tables in its order, `38-54`), `RepoError` with §2.4's variants
  and display texts (`62-91`), and `IslesStore`'s accessors and `read`/`write`/`account_write` heads (`105-210`,
  §3.2's signatures with the `Send + 'static` bounds `store::Database` requires). The bodies (`open_configured`, the
  replay scope, `check_home`, `store_actor`) do not match.
- **Spec texts and constants**: `PLAYER_JOURNEY_LOADOUT_NODE_IDS` and `SlotRangeError` (`slots.rs:11-21`, §7.3); the
  removal vocabulary (`repo/parties.rs:444-468`: `Removal { Leave, Kick, Move }` with §11.2's `LEAVE`/`KICK`/`MOVE`,
  and `remove_member` with exactly the argument list of §11.2's "Removing a member (party, target, now, removal,
  actor: Option)"); the data-error texts of §6 to §11 (`table.column` labels, `... is not a list`).
- **Short logic runs**, each a spec sentence in its plainest form: `leave`/`kick`'s guards (`repo/parties.rs:808-829`,
  §11.2's "Nothing unless the actor leads ... and the target is not the actor"), the slot bound check
  (`repo/characters.rs:327-331`), `if delta < 0.0 { return Ok(None) }` (`repo/bounties.rs:645`), the Guard finding's
  account cap (`repo/reforged.rs:1840`, §11.3.4's `GUARD_FINDING_ACCOUNT_LIMIT`), and the reforge decoder's
  `int("priorProgress")?` chain (`repo/progression.rs:62-66`, §10.1's keys).
- **Tests**: `tests/common/mod.rs`'s Postgres harness (`19-62`) matches `crates/store/tests/database.rs`, the MIT
  harness the implementer read (43 lines), which is why it also matches restricted's support module; its `launcher`
  builder (`134-164`) is `LauncherAccountRecord` in §6.2's field order with `None`s. The rest are
  `}).await.unwrap(); assert_eq!(` closers and §2.1-form sample times.

**Comments, strings, names.** No explanatory comment was copied. The only comment lines equal to restricted ones are
four section banners in `repo/auth.rs` (`536`, `610`, `1249`, `1674`: `// ----...---- accounts` and the like), the
house style of `crates/admin`, naming §6's repositories. Of the 105 string literals of 12 characters or more that
also occur in restricted and are not in the specs, all are §2.1-form test times, the `LoadoutSlot_0n` ids, the embed
paths, `table.column` labels, `CURRENCY_RAMS`, the harness's `CREATE`/`DROP DATABASE` (from `crates/store`), and one
test placeholder, `"$argon2id$hash"` (`tests/common/mod.rs:147`). 200 of the crate's 392 function names also occur in
restricted; all but ten are the spec's or `std`'s. The ten private helpers (`launcher_record`, `insert_balances`,
`loadout_row`, `write_event`, `remove_member`, `upsert_base`, `json_list`, `parse_json`, `insert_audit`, `daily_doc`)
are plain names for a spec step (§10.2's "Upsert base", §11.2's "Removing a member", §11.3.3's day document) and
differ from restricted's in arguments, return type or module. The module layout is §5's. The one shared shape the
spec does not give is the macro: `repository!` (`repo/mod.rs:9-28`) declares each trait from a list of method heads
and gives every method an `Unimplemented` default; its name and metavariable names (`$doc`, `$mdoc`, `$name`,
`$label`, `$method`, `$arg`, `$ty`, `$ret`) coincide with restricted's, though the spec names no macro. The
default-in-trait body and the `= "label"` syntax are the implementer's own.

### 6.2 Migrations

| Migration (each engine) | `up.sql` lines | Matched | % | `down.sql` lines | Matched | Restricted comment lines in `up.sql` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `0001_accounts` | 7 | 7 | 100.0 | 1 | identical | 1 |
| `0020_auth` | 100 | 96 | 96.0 | 8 | 8 | 3 |
| `0030_characters_loadouts` | 24 | 24 | 100.0 | 2 | identical | 3 |
| `0050_inventory` | 25 | 25 | 100.0 | 2 | identical | 4 |
| `0055_bounties_store` | 51 | 51 | 100.0 | 6 | 6 | 7 |
| `0070_chat_friend_edges` | 12 | 12 | 100.0 | 1 | identical | 3 |
| `0200_reforged_invites` | 7 | 7 | 100.0 | 1 | identical | 1 |
| `0201_admin` | 26 | 26 | 100.0 | 2 | identical | 2 |
| `0202_launcher_notices` | 8 | 8 | 100.0 | 1 | identical | 2 |
| `0203_operations_policy` | 11 | 9 | 81.8 | 1 | identical | 3 |
| `0204_launcher_guard` | 34 | 34 | 100.0 | 2 | identical | 3 |
| `0205_activity` | 48 | 48 | 100.0 | 4 | 4 | 1 |
| `0500_progression` | 76 | 76 | 100.0 | 8 | 8 | 6 |
| `0600_parties` | 47 | 47 | 100.0 | 4 | 4 | 5 |
| `0610_timeouts` | 3 | 3 | 100.0 | 3 | 3 | 3 |
| All 30 files of one engine | 525 | 506 | 96.4 | | | 47 |

Both engines give the same figures (the two sets differ only in the identity-key clause, §4.2), so a file's best
match is sometimes the other engine's copy of the same migration. "identical" marks a down file under the 3-line
threshold whose text is the same. The crate's files carry no comment (restricted's 47 per engine are absent). Their
other lines agree with restricted's down to the indentation; the only layout differences are the wrapped partial
indexes of `0020` and the wrapped seed `INSERT` of `0203`.

§4 forces that agreement. §4.1 fixes the folders, the tables in each and the down files' drop order. §4.2 to §4.11
give every column in table order with its declared type spelling, `NOT NULL`, the clause order of its constraints
(`PRIMARY KEY`, `UNIQUE`, `DEFAULT`, `REFERENCES accounts (id)`, `ON DELETE CASCADE`), the identity-key spelling per
engine, composite keys as a table constraint after the last column, and each index's name, columns and predicate
after its table. In the stock layout of hand-written DDL (one column per line, four-space indent) a table can only be
written one way. The spec leaves no alias, view, trigger or statement order open.

**Verdict: clean of structural or expressive copying.** The Rust matches are §2 to §11's records, enums, trait lists
and signatures in their order, the spec's error and data texts, bounds fixed by `module-api` and `store`, boilerplate
and the MIT `store` test harness. No SQL statement, row decoder or method body is shared beyond one-line guards that
restate a spec sentence. The migrations match because the spec gives the schema at statement level, and their
comments were not carried. Two cosmetic rewrites are asked for as a precaution, from §5.1 only and without looking at
the current text: `src/repo/mod.rs:9-28`'s macro, under a name and metavariables of the implementer's choosing (the
declaration syntax may stay), and `tests/common/mod.rs:147`'s password-hash placeholder. Nothing else needs to change.
