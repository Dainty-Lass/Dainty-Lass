# Clean-Room Provenance Record: `crates/rustic-isles-gamedata` (RD27)

- **Component**: `crates/rustic-isles-gamedata` (crate `rustic-isles-gamedata`, `license = "MIT"`,
  `Copyright (c) 2026 Silent Spud`)
- **Behaviour specification**: `docs/port/spec-rustic-isles-gamedata.md` (§13's coordinator decisions override the
  rest), with the `rustic-isles` and `rustic-isles-auth` crates for the contracts it builds on
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-aa3ba7934852c1bbb` (branch
  `feat/rustic-isles-gamedata`, reset to `feat/rustic-isles-clean-room` at `7ad80284` before work began)
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, globbed or opened anything under `R:\source\dauntless\dainty-restricted` (any
worktree of it, including the session's additional working directory `splashloader-integration-90130d`),
`legacy-revived`, any `rustic-isles` or `rustic-rifts` tree outside dainty-lass, or any other repository. It ran no
`git show` or `git log -p` of another branch (one `git log --format` of this branch's seam commit `e12e8715`, for its
message form). It searched nothing on the web and did not look for the golden files or captures §10.1 keeps in
dainty-restricted. It did not read the other `spec-rustic-isles-*.md` files in the tree.

### 1.2 Other sources

None beyond this repository: no crate sources from the cargo registry were opened.

## 2. Files read

- The coordinator's brief; `docs/port/spec-rustic-isles-gamedata.md` (whole); `AGENTS.md`
- `crates/rustic-isles/`: `Cargo.toml`; `src/lib.rs`, `handlers.rs`, `version.rs`, `season.rs`, `game_data.rs`,
  `shared.rs`, `body.rs`, `testing.rs` (whole); `settings.rs` (lines 1-80, 230-260 and its public items by grep);
  `replay.rs` (lines 1-160); `routes/mod.rs` (lines 1-200); `routes/table.rs` (the `GameData` rows by grep);
  `pattern.rs` (lines 80-200); `composer.rs` (lines 1-120); `pipeline.rs` (`app` and the Gauntlet check by grep);
  `tests/pipeline.rs` (lines 1-120). Core's code stood in for spec-rustic-isles-core §5.7 and §8, whose text was not
  read
- `crates/rustic-isles-auth/`: `Cargo.toml`; `src/game.rs` (lines 40-75 and 100-241), `lib.rs` (lines 280-330),
  `test_support.rs` (public items by grep, lines 60-70 and 270-290); `tests/common/mod.rs` (lines 1-80). Auth's code
  stood in for spec-rustic-isles-auth §4.1
- `crates/service-kit/src/`: `json.rs` (lines 1-60), `jsjson.rs` (lines 1-210), `js.rs` (lines 1-180),
  `collate.rs` (lines 1-60), `body.rs` (lines 40-66 and public items by grep)
- `crates/hostkit/src/paths.rs` (the four `Paths` fields by grep), `gamedata.rs` (`GeneratorKind` by grep)
- `Cargo.toml`; `fuzz/Cargo.toml`, `fuzz/fuzz_targets/rustic_isles_routes.rs` (whole),
  `rustic_isles_pipeline_text.rs` (first 30 lines)
- `docs/port/clean-room/rustic-isles.md` (first 120 lines and its headings), `rustic-isles-auth.md` (headings);
  `docs/security/rustic-isles-auth.md` (format)

## 3. Files written

### 3.1 The crate, from the spec

| File | Spec |
| --- | --- |
| `src/lib.rs` | §8.2 `register`, `routes`, `envelope`; §3.1's 29 rows |
| `src/areas.rs` | §3.1-§3.7 handlers, gates and logs |
| `src/tuning.rs` | §3.4 |
| `src/gauntlet.rs` | §3.5 |
| `src/patchnotes.rs` | §3.6, with the project's own prose (§13 item 1) |
| `src/progression_config.rs` | §3.7 |
| `src/generators/mod.rs` | §5.9, §5.10, §8.2 (`run`, `BackendStep`, `GeneratorPaths`, `VersionInputs`) |
| `src/generators/pretty.rs` | §2.1 |
| `src/generators/io.rs` | §4.1 readers, item data, texture references; §4.2 backups |
| `src/generators/progression.rs` | §5.1 |
| `src/generators/slayers.rs` | §5.2, §5.3 |
| `src/generators/reward_cache.rs` | §5.4, §8.2 `RewardCachePlan` |
| `src/generators/ladyluck.rs` | §5.5, §5.6 |
| `src/generators/platinum.rs` | §5.7 |
| `src/generators/policy.rs` | §5.8, §6 `TokenSource` |
| `tests/routes.rs`, `tests/generators.rs` | §10.2's table, on synthetic fixtures only |

### 3.2 Elsewhere

- `Cargo.toml`: `rustic-isles-gamedata` in `[workspace.dependencies]`; `Cargo.lock`. A seam change in a commit of its
  own. No new external dependency.
- `fuzz/fuzz_targets/rustic_isles_gamedata_{windows,formats}.rs`, their `[[bin]]` entries and the
  `rustic-isles-gamedata` dependency in `fuzz/Cargo.toml`, `fuzz/Cargo.lock`
- `docs/security/rustic-isles-gamedata.md` (§13 item 8: its own page), this record

## 4. Verification

- `cargo test -p rustic-isles-gamedata --locked --no-fail-fast`: 31 tests (13 unit, 9 routes, 9 generators), all pass
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean
- `cargo check` of the two new bins in `fuzz/`: clean (not fuzzed; the targets need nightly)
- Not run against the real 1.14.7 client, restricted's game-data golden or the Node byte-for-byte generator test;
  those run in dainty-restricted once the composer and host call this crate (§10.3)

## 5. Deviations and ambiguities, and how they were resolved

1. **`ItemGrantWindow`** (§8.1, §8.2) is core's `BountyItemGrant` (`item_id`, `amount`, `start`, `end`); the tuning
   functions take `&[BountyItemGrant]`.
2. **Survey status** (§3.2): "not a valid HTTP status" is read as outside 100-999 (Express's `res.status` range); the
   answer is core's 500 error page.
3. **Log texts the spec leaves open**: the Gauntlet config log (`Gauntlet config requested from {host} (probe on|off)`)
   and the patch-notes log (`Patch notes language=.. gameversion=..`) are worded here.
4. **`gauntlet::personal_rewards_payload()`** is added beside the four the spec names, for B49.
5. **JSON error messages**: where an error carries `{message}` (`invalid JSON in {path}: {message}`, `SyntaxError:
   {message}`), it is serde_json's text, not V8's. Restricted's byte-for-byte test compares written files; a report or
   error text comparison would differ there.
6. **§5.2 prose the spec only describes** (the two lines on dropped ids, the two restart lines) is worded here; the
   dropped ids are listed indented four spaces.
7. **§5.3**: a later row that is left out (all three lists empty) does not remove an earlier row's definition;
   `({rows} nodes)` counts every parsed row; `changed` lists ids in next's order.
8. **§5.5's refusal** is one error text in Node's order: the report lines, the problem list, then `ERROR: refusing
   ...`. hostkit shows an error's first non-empty line, so a host sees `store entries : n` rather than the refusal.
   Flagged for the coordinator: putting the refusal first would read better in hostkit but departs from the oracle.
9. **§5.6**: a live capture or store that fails to parse is `invalid JSON in {path}: {message}`; the store file is not
   BOM-stripped (the spec says so only of the capture); "the previous SKU's images when it had any" is an `images`
   key that is present and not null; "Retired" lists existing ids absent from the new store.
10. **§5.7**:
    - The two required data-folder stores' missing-file errors are worded here (`Reward Cache store not found: ..`,
      `Lady Luck store data not found: ..`); a capture that cannot be read counts as missing; one BOM is dropped.
    - The catalog-icon fallback takes, per id, the first of the custom, large and small icons that starts `/Game/`.
    - **The sort key's tag is the tag text** inside `{section}|{tag}|{price}|{id}` with `zzz` as the fallback, so
      section 1 comes out in the tags' collation order, not the tab table's. The tab table's "(it also orders the
      output)" is read as choosing which tag a SKU sorts by. Flagged: if Node sorts by the table index, this differs.
    - Quest-earned ids are compared exactly (the captures are upper-cased, catalog ids are already upper case).
    - Rank skips and passes check in the order the spec lists (baked, price, then claimed); pass 1 checks claimed
      first. Only skip counts, which are not reported, depend on it.
    - `tierB: no artwork resolved` cannot happen: a row that passed the artwork check always resolves `{}`.
11. **§5.8**: `Seeds` are candidate ids as given (not re-tokenised); a missing game data folder has nothing to walk;
    a folder reached twice through links is walked once; any failure to read the catalog is `Catalog dump not found`.
12. **§3.7**: the "no season template" warning is logged whenever no template exists, recent list empty or not (the
    spec's order); with none opened, only the warning is logged, not the info line.
13. **No `generators` feature**: the spec's "could" was not taken; `regex` and `sha2` are core's dependencies anyway.
14. **`generators::fuzzing`** (doc-hidden) exposes the rule helpers the formats fuzz target drives; not a contract.
15. **Backups** stamp with the system clock, not the replay clock (a host-side tool).
16. **`RewardCachePlan::from_json`** returns `Result<Self, String>`; a category missing from `expected` expects 0.
17. **Found in core, not changed**: with replay headers on, an extreme integer replay time overflows
    `season::active_season_window_ms` in a debug build (listed in the security page). The windows fuzz target keeps
    now within the JS date range.

## 6. Similarity

To be measured by the coordinator against restricted `e0e238b`.
