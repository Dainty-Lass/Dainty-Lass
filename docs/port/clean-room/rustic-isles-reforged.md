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
12. **Rewrites after the similarity review** (precautionary, from public spec §4.4 only, without reading the
    earlier text): `metrics.rs`' registry is now a `Store` of three maps keyed by a `Series` (name and kept labels,
    ordered by its sort text) with a `Latency` value for histograms. Its exposition was checked byte-identical against
    a capture taken before the rewrite (counters at the cap, dropped and duplicate labels, the four-label cut, gauges
    floored at 0, hidden section gauges, histograms with and without labels, the invalid-name fallback). The
    operations tests' `record(guard, profile, expires)` helper is now `policy_row(expires_at, profile, enforcement)`.

## 5. Similarity

Measured by the RD27 reviewer (who may read the AGPL code) with `similarity.md` §1's method, on the crate as merged
at `54b380ec` (last touched by `c95ad3b6`): every `.rs` file in `src/` and `tests/` against each of 34 files of
restricted `e0e238b`'s `modules/rustic-isles/crates/isles-core/`: `src/areas/reforged_api.rs` and the 24 files of
`src/areas/reforged_api/`, `src/bootstrap.rs`, `src/module.rs` (which holds the admin actions and home operations),
`src/http.rs`, and the tests that drive them, `tests/{reforged_admin,reforged_api,reforged_api_golden,
reforged_updates,auth_reforged_seams}.rs` and `tests/{common,support}/mod.rs`. The best single file is named; "Any"
counts a line matched by a 3+-line run in any of the 34. Paths in the third column are under `isles-core/`, with
`areas/` short for `src/areas/`; a file with no match names none.

| File | New lines | Best restricted file (old lines) | Matched | % | Any, % |
| --- | ---: | --- | ---: | ---: | ---: |
| `src/activity.rs` | 1343 | `areas/reforged_api/activity.rs` (1389) | 154 | **11.5** | 13.6 |
| `src/admin/actions.rs` | 545 | `src/module.rs` (1349) | 46 | **8.4** | 9.2 |
| `src/admin/auth.rs` | 413 | `areas/reforged_api/admin.rs` (1093) | 30 | **7.3** | 11.6 |
| `src/admin/bootstrap.rs` | 112 | `src/bootstrap.rs` (117) | 18 | **16.1** | 16.1 |
| `src/admin/discord.rs` | 582 | `areas/reforged_api/admin_discord.rs` (709) | 82 | **14.1** | 15.3 |
| `src/admin/host_routes.rs` | 281 | `areas/reforged_api/host_routes.rs` (304) | 46 | **16.4** | 19.2 |
| `src/admin/mod.rs` | 519 | `src/module.rs` (1349) | 15 | **2.9** | 4.0 |
| `src/admin/players.rs` | 490 | `areas/reforged_api/admin.rs` (1093) | 55 | **11.2** | 12.9 |
| `src/admin/release_gates.rs` | 225 | `areas/reforged_api/release_gates.rs` (272) | 27 | **12.0** | 15.6 |
| `src/admin/signin.rs` | 184 | `areas/reforged_api/admin.rs` (1093) | 16 | **8.7** | 8.7 |
| `src/admin/tester_max.rs` | 768 | `areas/reforged_api/tester_max.rs` (977) | 86 | **11.2** | 12.9 |
| `src/guard.rs` | 1020 | `areas/reforged_api/guard.rs` (1442) | 110 | **10.8** | 11.9 |
| `src/guard_manifest.rs` | 346 | `areas/reforged_api/guard_manifest.rs` (345) | 72 | **20.8** | 23.7 |
| `src/jobs.rs` | 84 | `areas/reforged_api/tester_max.rs` (977) | 3 | **3.6** | 3.6 |
| `src/launcher_auth.rs` | 35 | none | 0 | **0.0** | 0.0 |
| `src/lib.rs` | 191 | `areas/reforged_api.rs` (42) | 6 | **3.1** | 3.1 |
| `src/logs.rs` | 745 | `areas/reforged_api/player_logs.rs` (644) | 55 | **7.4** | 13.3 |
| `src/metrics.rs` | 303 | `areas/reforged_api/metrics.rs` (250) | 50 | **16.5** | 16.8 |
| `src/notices.rs` | 542 | `areas/reforged_api/notices.rs` (628) | 59 | **10.9** | 13.3 |
| `src/observe.rs` | 209 | `areas/reforged_api/observe.rs` (173) | 38 | **18.2** | 21.1 |
| `src/operations.rs` | 415 | `areas/reforged_api/operations.rs` (401) | 73 | **17.6** | 19.0 |
| `src/probes.rs` | 159 | `src/http.rs` (717) | 10 | **6.3** | 8.2 |
| `src/reforged_api.rs` | 847 | `areas/reforged_api/rules.rs` (451) | 79 | **9.3** | 18.2 |
| `src/seams.rs` | 165 | `src/module.rs` (1349) | 12 | **7.3** | 8.5 |
| `src/state.rs` | 221 | `areas/reforged_api/admin_discord.rs` (709) | 9 | **4.1** | 7.2 |
| `src/test_support.rs` | 94 | `areas/reforged_api/tester_max.rs` (977) | 3 | **3.2** | 3.2 |
| `src/updates.rs` | 1045 | `areas/reforged_api/updates.rs` (1117) | 42 | **4.0** | 4.7 |
| `src/util.rs` | 330 | `areas/reforged_api/admin_discord.rs` (709) | 23 | **7.0** | 15.2 |
| `tests/admin.rs` | 1024 | `tests/reforged_admin.rs` (1371) | 97 | 9.5 | 12.5 |
| `tests/common/mod.rs` | 387 | `tests/reforged_admin.rs` (1371) | 34 | 8.8 | 14.2 |
| `tests/public.rs` | 1305 | `tests/reforged_api.rs` (630) | 97 | 7.4 | 14.3 |
| The 28 `src` files | 12213 | | 1219 | **10.0** | 12.5 |
| The 3 test files | 2716 | | 228 | **8.4** | 13.6 |
| All 31 | 14929 | | 1447 | **9.7** | 12.7 |

No file reaches 25 %, on either column; the highest are `guard_manifest.rs` (20.8, any 23.7), `observe.rs` (18.2,
21.1) and `operations.rs` (17.6, 19.0). Core's change `f418cddd` (`AdminRequestHead` and the pipeline's body read for
a relayed `/admin/` row: 52 added code lines in `crates/rustic-isles/src/{lib,pipeline,shared}.rs` and
`tests/pipeline.rs`) was measured hunk by hunk against the same 34 files plus every core `src/` file outside `areas/`
and `repo/`: no 3-line run matches, and the type and its fields have no restricted counterpart. The fuzz targets
(`fuzz/fuzz_targets/rustic_isles_reforged_*.rs`) have no restricted counterpart and were not measured.

**What the matches are.** Every run of 5 lines or more against any of the 34 files was read: 208 of them over 116
distinct spans of the crate, 107 of which are, or end in, the `} } #[cfg(test)] mod tests { use super::*; #[test]`
boilerplate. The rest:

- **Wire key lists and spec tables**: the `ServerStatus` keys (`tests/public.rs:282-302`, with public §14's
  `Fox hold`, `1.2.3` and `https://example.com/src`), the update manifest's keys (`tests/public.rs:387-403`), the audit
  row's and the player row's keys (`tests/admin.rs:375-387`, `93-103`), the activity counters
  (`activity.rs:854-861`), the metrics label set (`metrics.rs:10-20`), the game-data file list
  (`reforged_api.rs:494-501`), the release gates' names (`admin/release_gates.rs:242-247`) and the request class rule
  (`observe.rs:44-53`, public §4.5's four prefixes).
- **Spec enums with their `as_str` and parse**: `Refusal` and its codes (`reforged_api.rs:124-146`),
  `RegistrationMode` with `OPEN`/`INVITECODE`/`NONE` and its fallback (`32-55`), `GameDataKind` (`522-526`), and the
  `HuntResult` and `GuardStatus` choices (`activity.rs:225-229`, `guard.rs:256-260`).
- **Records of the MIT store crate and of the specs**: struct literals of `rustic-isles-store`'s launcher account,
  Discord OAuth transaction, invite, access change and admin session records in their declared field order
  (`reforged_api.rs:318-342`, `admin/discord.rs:294-300`, `484-492`, `admin/host_routes.rs:82-87`,
  `admin/bootstrap.rs:109-114`, `tests/common/mod.rs:305-321`, `364-370`, `tests/public.rs:1310-1315`); the store's
  `read`/`write` closures over its repositories (`admin/discord.rs:301-308` with admin's log text, `368-372`,
  `admin/auth.rs:324-328`, `admin/host_routes.rs:167-172`, `reforged_api.rs:346-351`); and the specs' records
  (`BootstrapAdminRequest`, `Gate`, `UnexpectedModule`, the operations policy, the purge counts:
  `admin/bootstrap.rs:18-26`, `admin/release_gates.rs:18-31`, `guard.rs:32-38`, `81-85`, `operations.rs:25-54`,
  `logs.rs:189-193`).
- **Spec signatures and rules**: `signature_material`'s arguments and the `MYSTIC-GUARD-2` line order
  (`guard.rs:141-165`, with the `too_many_arguments` allow clippy needs), the manifest's destructure in the spec's
  field order and its timestamp errors (`guard_manifest.rs:119-126`, `269-276`), the character-set predicates
  (`guard_manifest.rs:216-220`, `admin/tester_max.rs:161-166`), the joined-header rule (`util.rs:355-360`, admin §2's
  `String(array)`), the launcher version filter (`updates.rs:209-214`), the target tail rule
  (`reforged_api.rs:468-472`), the Discord authorize query (`admin/discord.rs:310-316`) and spec messages
  (`admin/signin.rs:122-128`, `admin/tester_max.rs:221-225`, `notices.rs:166-170`, `activity.rs:1232-1236`,
  `observe.rs:123-127`).
- **Test vectors from the specs' test tables** (public §14, admin §12): the redaction vectors (`observe.rs:208-216`),
  the log names and sessions (`logs.rs:742-750`, `776-780`, `tests/public.rs:733-753`), the notice times and
  expected-version vectors (`notices.rs:451-456`, `560-564`), the guard reasons (`guard.rs:961-971`), the label vector
  (`metrics.rs:296-301`), the Discord env values (`tests/admin.rs:666-672`), paging (`tests/admin.rs:306-311`), the
  admin headers (`tests/admin.rs:85-89`), the build record (`tests/public.rs:305-309`), the manifest's cache header
  (`tests/public.rs:378-382`), and `assert_eq!(… .status, 200)` / `.json()["code"]` chains.

No comment was copied: no comment line in the crate equals a restricted one. Of the 1547 string literals of 12
characters or more that also occur in the 34 files, 1466 are verbatim in the two specs; the other 81 are spec texts
with a placeholder filled in (`announcements[0]…`, `tags[0]…`), messages the specs wrap across a line break
(`Session limit reached…`, `Refusing to remove uploaded logs…`, the manifest errors), the specs' test times and
versions, `observe.rs`'s two redaction regexes (public §4.5's rules written out), test paths built from the specs'
queries, and format fragments. 207 of the crate's 616 function names and 41 of its 175 type and constant names also
occur in restricted; the 49 and 13 the specs do not spell out are plain names for the specs' own words (the
"serving" and "locate" steps of updates, guard "findings", the log "sync", the "active play" gate, the key "holder",
the "status cache") or stock helpers (`sha256_hex`, `hmac_sha256`, `random_bytes`, `read_json`, `peer_ip`) that other
MIT crates also define. No unusual structure is shared: the largest files (`activity.rs`, `updates.rs`, `guard.rs`,
`logs.rs`, `admin/tester_max.rs`, the black-box tests) match at 4 to 12 %, and their dispatch, upload, purge, scoring
and job logic share nothing beyond the lines above.

**Two spans that go a little beyond the spec**, both small and both stock:

1. **`src/metrics.rs:33-46` (the private `Histogram` and `Registry`).** The spec gives the registry's rules and the
   sample order, not its representation. The private struct names, the three `BTreeMap<String, (String, Labels, _)>`
   maps named `counters`, `gauges` and `histograms`, and `Histogram`'s `sum`/`count` tail (an 8-line run) coincide
   with a restricted layout; the crate's own parts differ (the `sections` field, 7 buckets, one registry per module).
2. **`src/operations.rs:376-385` (the test helper `record(guard, profile, expires)`).** Its values are public §14's
   operations vector (`version` 3, `u`, `b`), but the helper's name and argument list coincide with a restricted test
   helper (a 14-line run with the `mod tests` head).

**Verdict: clean of structural or expressive copying.** Everything else is spec-given wire strings and key lists,
enum and record tables in the specs' or the MIT store crate's order, spec signatures, the specs' own test vectors and
boilerplate. As a precaution, and from the spec only without looking at the current text, the two spans above may be
rewritten: the metrics registry's private representation in the implementer's own shape (for instance one map per
kind keyed by a `(name, labels)` struct, or a sample struct in place of the tuples), and the operations test helper
under another name and argument order. Neither is needed for the crate to stand as clean-room.
