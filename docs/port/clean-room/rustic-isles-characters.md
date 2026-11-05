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

Measured by the RD27 reviewer (who may read the AGPL code) with `similarity.md` §1's method, on the crate as committed
at `05701862`: every `.rs` file in `src/` and `tests/` against each of 12 files of restricted `e0e238b`'s
`modules/rustic-isles/crates/`: the area (`isles-core/src/areas/characters.rs` and `areas/characters/{controller,
data,golden,starter}.rs`), the repositories it calls (`src/repo/characters.rs`, `repo/characters/slots.rs`), the
manifest and version tables (`src/areas/reforged_api/guard_manifest.rs`, `src/tables.rs`, `isles-1-14-7/src/lib.rs`
with the 1.14.7 starter values), and the tests (`tests/characters.rs`, `tests/common/mod.rs`), the best single file
named. "Any" counts a line matched by a 3+-line run in any of the 12. Paths in the third column are under
`isles-core/`.

| File | New lines | Best restricted file (old lines) | Matched | % | Any, % |
| --- | ---: | --- | ---: | ---: | ---: |
| `src/controller.rs` | 804 | `src/areas/characters/controller.rs` (609) | 114 | **14.2** | 17.3 |
| `src/data.rs` | 345 | `src/areas/characters/data.rs` (580) | 40 | **11.6** | 15.7 |
| `src/lib.rs` | 76 | `src/areas/characters/controller.rs` (609) | 7 | **9.2** | 18.4 |
| `src/routes.rs` | 550 | `src/areas/characters.rs` (682) | 55 | **10.0** | 11.5 |
| `src/starter.rs` | 334 | `src/areas/characters/starter.rs` (674) | 47 | **14.1** | 15.3 |
| `tests/characters.rs` | 450 | `src/areas/characters/data.rs` (580) | 3 | 0.7 | 0.7 |
| `tests/common/mod.rs` | 652 | `tests/characters.rs` (664) | 46 | 7.1 | 9.8 |
| `tests/data.rs` | 343 | `src/areas/characters/data.rs` (580) | 32 | 9.3 | 10.2 |
| `tests/loadouts.rs` | 462 | `src/areas/characters/data.rs` (580) | 3 | 0.6 | 1.3 |
| The 5 `src` files | 2109 | | 263 | **12.5** | 15.2 |
| The 4 test files | 1907 | | 84 | **4.4** | 5.7 |
| All 9 | 4016 | | 347 | **8.6** | 10.7 |

No file reaches 25 %, against its best file or on the "any" column. The `RuntimeCaller` handle in
`crates/rustic-isles-auth/src/seams.rs` (`0192eabf`'s 57 added code lines, against the 22 files of restricted's
`areas/auth/`, the characters area, `http.rs` and `hooks.rs`) matches 3 lines (5.3 %), the `#[cfg(test)] mod tests {
use super::*;` head. The fuzz target (`fuzz/fuzz_targets/rustic_isles_characters_data.rs`) has no restricted
counterpart and was not measured.

**What the matches are.** Every run of 5 lines or more against any of the 12 files was read: 30 of them, each a
distinct span. Four are the `} #[cfg(test)] mod tests { use super::*; #[test]` boilerplate or closing braces
(`data.rs:355-361`, `routes.rs:588-594`, `starter.rs:363-369`, `starter.rs:49-53`). The rest:

- **Spec signatures** (12 runs, `controller.rs`): the parameter lists of §8.2's methods and the controller functions
  behind §5.2-§5.12 (`store, tables, actor, user_id, character_id` and the in-transaction `tx, repos, tables, account`
  form: `249-253`, `292-297`, `359-363`, `411-415`, `620-624`, `665-669`, `692-696`, `705-710`, `765-769`,
  `813-817`); `needs_repair` (`482-488`), whose head is §8.2's and whose first statement is §7.2's "both rows exist";
  and `ensure_starter_in`'s return type and ownership gate (`503-507`, §8.2 and §7.2's "`characters.find`; none →
  `NotOwned { character_id, user }`" first).
- **Spec records and tables**: `LoadoutState` (`controller.rs:26-31`, §8.2's three fields), `Manifest`
  (`starter.rs:23-28`, §8.2's four), `EnsureError` (`lib.rs:89-95`, §8.2's variants and text), the progress step
  names (`data.rs:22-27`, §5.5), §5.12's refusal reasons (`data.rs:348-355`) and §13.2's normalised key order
  (`tests/data.rs:33-39`, `41-46`).
- **Wire fields**: `body.get("data")`, `body.get("updateVersion")` in `post_character`'s call (`routes.rs:341-345`).
- **A published algorithm**: `data.rs:310-317`, Howard Hinnant's public-domain `civil_from_days` with his variable
  names and constants (credited in the comment above it, deviation 12).
- **Seam implementations in the fixtures** (`tests/common/mod.rs`): `RuntimeAuth`'s fields (`193-197`, an MIT
  `rifts-seam` struct), a guard method head (`269-273`) and `VersionHooks` stubs (`155-162`, `167-171`), whose method
  names and signatures the MIT traits fix.
- `data.rs:80-85`: the tail of a `match` (`_ => None`, braces).

No comment was copied: no comment line of 20 characters or more equals a restricted one. The crate holds 200 distinct
string literals of 12 characters or more; 77 also occur in the 12 files, and every one is in the characters, core or
store spec (routes, log lines, refusal and error texts, the 409 page, wire keys). 76 of the crate's 223 function names
also occur in restricted: the spec's (§8.2, core §5.7), `std` trait methods, the MIT seam traits' (`rifts_seam`'s
guard and realms hooks, `VersionHooks`), or plain renderings of a spec name (`ensure_total_slots`,
`set_active_index`, `set_loadout_data` from §5.10-§5.12's Node names; `create_character`, `get_`/`put_`/
`post_character`, `set_loadout` from §3's routes; `progress_ordinal`, `progress_name` from §5.5; `nullish`,
`state_of`). Of the 63 type and constant names, 17 also occur; besides the spec's, those are `SAVED_CHARACTER` and
`ENTERED_RAMSGATE` (§5.5's client enum names), `DataError` (§5's "data error"; `data.rs:37-39`, a `String` newtype
with `#[error("{0}")]`), `CONFLICT_PAGE` (§3.4's "the conflict page"), `Repaired` (`starter.rs:279`, §7.2's
repaired items, known ids and "each is a change") and `Ctx` (`routes.rs:93`, a handler context whose first two
fields are core's `Call` fields). No unusual structure is shared: the order of checks, the retry loops, the progression
guard, the trigger reader, the repair and reconcile, and the unlock path match nothing beyond the lines above.

**Verdict: clean of structural or expressive copying; no rewrite needed.** At 8.6 % overall and at most 14.2 % for
any file, every matched span is a §8.2 signature or record, a spec table or wire text, a seam method fixed by an MIT
trait, a published algorithm, or boilerplate. The name coincidences above (`Repaired`, `DataError`, `Ctx`) are each a
stock rendering of the spec's own words and none carries shared logic, so they are recorded rather than rewritten.
