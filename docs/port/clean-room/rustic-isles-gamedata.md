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

- `cargo test -p rustic-isles-gamedata --locked --no-fail-fast`: 32 tests (14 unit, 9 routes, 9 generators), all pass
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
8. **§5.5's refusal** (settled by the amended decisions, follow-up commit): the error text starts with `ERROR:
   refusing ...`, then the report lines, the empty line, `VALIDATION PROBLEMS (n):` and the problem lines, so
   hostkit's first-line summary is the refusal; it is not repeated at the end. A `null` or other non-object entry
   reports the three problems. The success reports keep the empty line before `Dry run complete. Nothing written.`
   and before `Wrote ladyluck_store.json (..)`. First implemented with the refusal last and no empty lines.
9. **§5.6**: a live capture or store that fails to parse is `invalid JSON in {path}: {message}`; the store file is not
   BOM-stripped (the spec says so only of the capture); "the previous SKU's images when it had any" is an `images`
   key that is present and not null; "Retired" lists existing ids absent from the new store.
10. **§5.7**:
    - The two required data-folder stores' missing-file errors are worded here (`Reward Cache store not found: ..`,
      `Lady Luck store data not found: ..`); a capture that cannot be read counts as missing; one BOM is dropped.
    - The catalog-icon fallback takes, per id, the first of the custom, large and small icons that starts `/Game/`.
    - **The sort key's tag is the tag text** inside `{section}|{tag}|{price}|{id}` with `zzz` as the fallback, so
      section 1 comes out in the tags' collation order, not the tab table's. The tab table's "(it also orders the
      output)" is read as choosing which tag a SKU sorts by. Confirmed by the amended decisions.
    - **The price part** (amended decisions, follow-up commit) is `String(price)` left-padded with `0` to at least 6
      characters, never cut (`12.5` is `0012.5`, `-5` is `0000-5`, `"500"` is `000500`); first implemented as the
      safe integer, else 0, zero-padded.
    - **A `null` platinum price counts as missing** (amended decisions: "passes 1-3 skip a null price"), so passes 1-3
      skip it; first implemented as a price.
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
17. **Found in core**: an extreme integer replay time overflowed `season::active_season_window_ms` in a debug
    build. Core's window now saturates (merged from `feat/rustic-isles-clean-room`); the windows fuzz target still
    keeps now within the JS date range.
18. **`src/tuning.rs` rewritten** (follow-up commit, a precaution from the similarity review): written again from
    §3.4 alone without the earlier file's text in view. Each payload is §3.4's one-line JSON text, parsed once
    (`preserve_order` keeps the wire order); B85's `item_grant_data` is set in place from the version's windows. The
    public API is unchanged, and a unit test checks every text round-trips byte for byte.

## 6. Similarity

Measured by the RD27 reviewer (who may read the AGPL code) with `similarity.md` §1's method, on the crate as committed
at `4bc7ca77` (unchanged at `e9bd9b3b`): every `.rs` file in `src/` and `tests/` against each of the 13 files of
restricted `e0e238b`'s `modules/rustic-isles/crates/isles-core/` that hold the game-data area and its generators
(`src/areas/game_data.rs`, `src/game_data.rs`, the 9 files of `src/game_data/`, and the tests that drive them,
`tests/{gamedata_generators,pipeline}.rs` and `tests/common/mod.rs`), the best single file named. "Any" counts a line
matched by a 3+-line run in any of the 13. Paths in the third column are under `isles-core/`; a file with no match
names none.

| File | New lines | Best restricted file (old lines) | Matched | % | Any, % |
| --- | ---: | --- | ---: | ---: | ---: |
| `src/areas.rs` | 262 | `src/areas/game_data.rs` (323) | 13 | **5.0** | 5.0 |
| `src/gauntlet.rs` | 46 | `src/game_data/gauntlet.rs` (62) | 16 | **34.8** | 43.5 |
| `src/generators/io.rs` | 209 | `src/game_data/gauntlet.rs` (62) | 6 | **2.9** | 3.3 |
| `src/generators/ladyluck.rs` | 262 | `src/game_data/generators.rs` (2449) | 15 | **5.7** | 5.7 |
| `src/generators/mod.rs` | 99 | none | 0 | **0.0** | 0.0 |
| `src/generators/platinum.rs` | 679 | `src/game_data/generators.rs` (2449) | 34 | **5.0** | 7.1 |
| `src/generators/policy.rs` | 126 | `src/game_data/generators.rs` (2449) | 16 | **12.7** | 15.9 |
| `src/generators/pretty.rs` | 99 | `src/game_data/generators.rs` (2449) | 11 | **11.1** | 16.2 |
| `src/generators/progression.rs` | 53 | `src/game_data/generators.rs` (2449) | 3 | **5.7** | 5.7 |
| `src/generators/reward_cache.rs` | 338 | `src/game_data/generators.rs` (2449) | 33 | **9.8** | 10.9 |
| `src/generators/slayers.rs` | 209 | `src/game_data/generators.rs` (2449) | 18 | **8.6** | 10.0 |
| `src/lib.rs` | 53 | `src/game_data/gauntlet.rs` (62) | 4 | **7.5** | 7.5 |
| `src/patchnotes.rs` | 70 | `src/game_data/patchnotes.rs` (86) | 9 | **12.9** | 12.9 |
| `src/progression_config.rs` | 251 | `src/game_data/progression_config.rs` (297) | 42 | **16.7** | 17.9 |
| `src/tuning.rs` | 101 | `src/game_data/tuning.rs` (105) | 87 | **86.1** | 86.1 |
| `tests/generators.rs` | 874 | `src/game_data/progression_config.rs` (297) | 12 | 1.4 | 1.4 |
| `tests/routes.rs` | 557 | `tests/pipeline.rs` (686) | 16 | 2.9 | 4.1 |
| The 15 `src` files | 2857 | | 307 | **10.7** | 12.1 |
| The 2 test files | 1431 | | 28 | **2.0** | 2.4 |
| All 17 | 4288 | | 335 | **7.8** | 8.9 |

Two files reach 25 % against their best single file (`tuning.rs` 86.1, `gauntlet.rs` 34.8). The fuzz targets
(`fuzz/fuzz_targets/rustic_isles_gamedata_{windows,formats}.rs`) have no restricted counterpart and were not measured.

**What the matches are.** Every run of 5 lines or more against any of the 13 files was read: 24 of them over 21
distinct spans of the crate, 3 of which are the `} #[cfg(test)] mod tests { use super::*; #[test]` boilerplate. The
rest, and the shorter runs in the large files:

- **§3.4's wire JSON** (`tuning.rs`, 87 lines): the seven payloads written as `json!` literals, one key per line in
  §3.4's key order (`tuning.rs:11-24`, `46-74`, `76-98`, `100-117`, runs of 12 to 27), under §8.2's function names.
  Every key and value is §3.4's; what is shared beyond them is the layout (which small objects sit on one line,
  such as `GlobalConfig` and each `bounty_data` entry). The grant list `item_grant_data` matches only in its closing
  lines.
- **The Gauntlet stubs** (`gauntlet.rs`, 16): §8.2's five payload functions, each one `json!` of §3.5's value, and
  `progression_payload`'s Unicode lower-case test (§3.5) in its only form (`gauntlet.rs:15-25`, `29-38`).
- **Spec tables in the spec's order**: the Slayer's Path unlock gates (§5.2, `slayers.rs:17-28`), the platinum
  exclusion tags (§5.7, `platinum.rs:24-32`), `RewardCacheCategory`'s six variants with its `as_str` and `tag`
  matches (§8.2, §5.4; `reward_cache.rs:17-57`), the trials leaderboard document (§3.3, `areas.rs:243-249`), the
  patch-notes structure (§3.6, `patchnotes.rs:18-36`), and `envelope` (§2, `lib.rs:69-70`).
- **Seam signatures and spec log texts**: `progression_config::served` (§8.2's signature over core's two
  `ProgressionConfigRule` variants, `progression_config.rs:21-32`), the window rewrite's counters, template guard and
  closing log (§3.7's step order and its two log texts, `progression_config.rs:100-112`, `129-160`), and the
  generator heads `(…, source_version, apply) -> Result<String, String>` and `(…, capture_dir, data_dir)` (§8.2).
- **Short idiom runs** (3-4 lines) in `platinum.rs`, `ladyluck.rs`, `policy.rs`, `pretty.rs` and `io.rs`: `serde_json`
  accessor chains (`.and_then(Value::as_array).cloned().unwrap_or_default()`), `} else { json!(1) };`, the
  comma-between-items loop of a pretty printer (`pretty.rs:44-48`, `57-60`; §2.1), and closing braces before
  a report's `.join`. None carries logic beyond the line.

No comment was copied: no comment line of 20 characters or more equals a restricted one. Of the crate's string
literals of 12 characters or more that also occur in the 13 files, all are §3-§5's wire, report and error texts or
test vectors derived from them (the season window dates, §3.7's worked example, core's replay header); the rest of the
hits are fragments between two literals. 55 of the crate's 187 function names also occur in restricted; all but
`gauntlet_config`, `backup_stamp` and `reconstructed_price` (the plain names for B47, §4.2's backup stamp and §5.7's
"reconstructed price") are the spec's (§8.2), core's `VersionHooks` methods the test version implements, or generic
test helpers (`send`, `post_json`, `patch_notes`). No unusual structure is shared: the generators, which hold most of
the crate's logic, match their restricted counterpart at 3 to 13 %, and the platinum passes, the Lady Luck enrich
checks, the policy walk, the Reward Cache selection and the pretty printer share nothing beyond the lines above.

**The one span that goes beyond the spec.** **`src/tuning.rs` (whole file, 86.1 %).** The content is §3.4's, and a
constant `json!` per payload is the idiomatic form; but the line layout of each literal is the implementer's choice
and agrees with restricted's line for line, which makes it the one file that reads as a copy.

**Verdict: clean of structural or expressive copying, with one small rewrite asked for as a precaution.** Everything
else is spec-given wire and report text, tables in the spec's order, §8.2's signatures, test vectors and
boilerplate. The rewrite, from §3.4 only and without looking at the current text: `src/tuning.rs`'s seven payloads
in a form of the implementer's own (for instance each one parsed once from §3.4's one-line JSON text, which keeps the
key order, or a different literal layout). `gauntlet.rs` needs nothing: its matched lines are §3.5's five values in
the only form they take.
