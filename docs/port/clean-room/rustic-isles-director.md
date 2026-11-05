# Clean-Room Provenance Record: `crates/rustic-isles-director` (RD27)

- **Component**: `crates/rustic-isles-director` (crate `rustic-isles-director`, `license = "MIT"`,
  `Copyright (c) 2026 Silent Spud`), with the test bin `rustic-isles-fake-gameserver`
- **Behaviour specification**: `docs/port/spec-rustic-isles-director.md` (its closing "Coordinator decisions" override
  the rest), with `docs/port/spec-rustic-isles-core.md` for the contracts it plugs into (§5.6, §6, §8, §11)
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-a811e622ee2a07e79`, branch
  `feat/rustic-isles-director`, cut from `feat/rustic-isles-clean-room` at `7ad80284` and merged with it again before
  delivery
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, globbed or opened anything under `R:\source\dauntless\dainty-restricted` (any
worktree, including the session's additional working directory `splashloader-integration-90130d`),
`legacy-revived`, any `rustic-isles` or `rustic-rifts` tree outside dainty-lass, or any other repository. It ran no
`git show` or `git log -p` of another branch (`git log --oneline` of `feat/rustic-isles-clean-room` only, to reset and
merge). It searched nothing on the web and did not look for recorded golden files or captures.

### 1.2 Other sources

Generic library sources in the cargo registry, for API shapes only: `windows` 0.62.2 (the signatures and constants of
`CreateProcessW`, the attribute-list functions, `PROC_THREAD_ATTRIBUTE_*`, `GlobalMemoryStatusEx`,
`SetHandleInformation`, `STARTUPINFOEXW`).

## 2. Files read

- `docs/port/spec-rustic-isles-director.md` (whole); from `docs/port/spec-rustic-isles-core.md` §3.8-§3.9, §5.6-§5.7,
  §6, §8.1-§8.3, §11-§13, §16 and §17; `AGENTS.md`; the brief
- `docs/port/clean-room/rustic-isles.md` and `docs/security/rustic-isles-auth.md` (format)
- `Cargo.toml`, `fuzz/Cargo.toml`, `fuzz/fuzz_targets/rustic_isles_routes.rs` and `rustic_isles_auth_text.rs` (heads)
- `crates/rustic-isles/`: `Cargo.toml`, `src/lib.rs`, `dedicated.rs`, `shared.rs`, `settings.rs`, `game_data.rs`,
  `handlers.rs`, `module.rs` (whole); `pipeline.rs` (lines 1-60, 150-420, the `Call` construction by grep),
  `routes/table.rs` (the D rows), `testing.rs` (whole), `replay.rs` (`random_uuid` by grep), `tests/module.rs`
  (first 120 lines)
- `crates/service-kit/src/`: `body.rs`, `json.rs` (whole), `jsjson.rs` (lines 1-160), `js.rs` (lines 1-140)
- `crates/module-api/`: `Cargo.toml`, `src/tree.rs` (whole), `PortRange` in `manifest.rs`
- `rand` usage in other crates of this repository (by grep, for the 0.10 trait names)

## 3. Files written

### 3.1 The crate, from the spec

| File | Spec |
| --- | --- |
| `src/lib.rs` | §12.2-§12.4: `GameserverArgs`, `DirectorConfig`, `DirectorService`, `InProcessDirector` |
| `src/api.rs` | §3: D1-D4, the body parser and the Express 404 page |
| `src/matchmaker.rs` | §4 |
| `src/gameservers.rs` | §5, §7.2, §7.3, §5.11 |
| `src/process.rs` | §5.5, §5.6 |
| `src/launcher.rs`, `src/os.rs` | §5.7, §12.5, §8 (free memory) |
| `src/cmdline.rs` | §5.3 |
| `src/config.rs` | §11 |
| `src/port_pool.rs` | §6 |
| `src/hub.rs` | §7.1 |
| `src/occupancy.rs` | §7.3, §7.4 |
| `src/memory.rs` | §8 |
| `src/hunt.rs` | §9 |
| `src/hunt_tables.rs` | §10 |
| `src/bin/rustic-isles-fake-gameserver.rs` | §13 |
| `tests/common/mod.rs`, `tests/e2e.rs`, `tests/module.rs`, `tests/hunt.rs`, `tests/import.rs` | §14.2 |

### 3.2 Elsewhere

- `Cargo.toml`: `rustic-isles-director` in `[workspace.dependencies]`, `Cargo.lock`; a seam change in its own commit.
  `windows` 0.62.2 and `libc` 0.2.191 are target dependencies in the crate's own manifest (coordinator decision).
- `fuzz/fuzz_targets/rustic_isles_director_{cmdline,matchmaking,tag_query,lines,hunt_import}.rs`, their `[[bin]]`
  entries, the `rustic-isles-director` dependency and tokio's `time` feature in `fuzz/Cargo.toml`, `fuzz/Cargo.lock`
- `docs/security/rustic-isles-director.md`, this record

## 4. Verification

- `cargo test -p rustic-isles-director --locked --no-fail-fast`: see the commit report
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean on Windows
- The Unix platform module and the launcher seam, copied into a scratch crate: `cargo clippy --target
  x86_64-unknown-linux-gnu -- -D warnings` clean (the whole crate cannot be cross-checked here: `aws-lc-sys` needs a
  Linux C compiler). Not run on Linux.
- `cargo check` of the five fuzz bins: clean (not fuzzed; cargo-fuzz needs nightly)
- Not run against the real 1.14.7 client or restricted's importer comparison (§14.1); the "real tables" test skips
  unless `RUSTIC_ISLES_REAL_HUNT_TABLES` names a folder

## 5. Deviations and ambiguities, and how they were resolved

1. **The robustness fixes** (coordinator decisions): a NUL anywhere in the command or environment refuses the spawn;
   non-Unicode process variables are skipped; a tag query's count stops at the end of the stream. The tag query also
   counts nesting past 64 as an overflow (§16 item 14's fallback), since a stream of nested `4`s nests as deep as it
   is long.
2. **Windows pipes.** The parent's ends are overlapped named pipes read through tokio; a read before `connect()`
   fails, so each is connected in its own task and copied into an in-memory pipe the pumps read.
3. **Hunt capacity race.** Node checks the cap and lists the server with no await between; here hunts past the check
   and not yet listed count toward the cap, so a burst cannot pass it.
4. **`LiveHubStatus`** in core names the Training Grounds `training`, not `training_dojo` (§12.3); core's name is used.
5. **`load_tables(dir)`** loads all seven (1.14.7); `load_tables_for(dir, trials)` and `table_files(trials)` give the
   five-table list for a version without Trials, which the "file list equals core's" test compares.
6. **`[HuntLaunchCoverage]`'s `gameModes=`** is written as a count, like the fields around it (§9.7 does not say).
7. **`GAMESERVER_LOG_CMDS` set but blank** counts as unset (no warning); §11 says "set but not so" warns.
8. **The importer with no manifest file**: the version is "(missing)" and the export is refused (§10.1 reads the
   version from the manifest only).
9. **Importer refusals in apply mode** return an error whose text is the report followed by the `REFUSING TO WRITE`
   lines; a dry run puts its closing line after the report.
10. **`Wrote {file}` and `Backed up:`** name the file, not its path; list texts use `, ...(+n)` after the shown items
    and counts `{name} x{n}`.
11. **Shutdown** also releases every remaining lease after the trees are gone, so "shutdown frees every port" holds
    without waiting for each exit event.
12. **The idle stop's release reason** is `idle stop` (not given by the spec).
13. **The 1.14.7 values** (§12.4) are not a constant of the crate (unit 12 carries them); the tests and the fuzz
    target define their own.
14. **`InProcessDirector::launch`** logs a failure with the route's text before answering `Refused { reason: None }`.
15. **Training's "not at boot"** is read as `TRAINING_HUB` off.

## 6. Similarity

Measured by the RD27 reviewer (who may read the AGPL code) with `similarity.md` §1's method, on the crate as merged at
`772e3c3d`: every `.rs` file in `src/` and `tests/` against each of the 24 `.rs` files of restricted `e0e238b`'s
`modules/rustic-isles/crates/isles-director/` (`src/`, `src/bin/` and `tests/`), the best single file named. "Any"
counts a line matched by a 3+-line run in any of the 24. Paths in the third column are under `isles-director/`; a
file with no match names none.

| File | New lines | Best restricted file (old lines) | Matched | % | Any, % |
| --- | ---: | --- | ---: | ---: | ---: |
| `src/api.rs` | 133 | `src/api.rs` (130) | 22 | **16.5** | 19.5 |
| `src/bin/rustic-isles-fake-gameserver.rs` | 128 | `src/bin/isles-fake-gameserver.rs` (125) | 13 | **10.2** | 12.5 |
| `src/cmdline.rs` | 62 | `src/cmdline.rs` (51) | 25 | **40.3** | 45.2 |
| `src/config.rs` | 217 | `src/config.rs` (130) | 20 | **9.2** | 10.6 |
| `src/gameservers.rs` | 1039 | `src/gameservers.rs` (1257) | 98 | **9.4** | 10.5 |
| `src/hub.rs` | 643 | `src/hub.rs` (332) | 93 | **14.5** | 17.9 |
| `src/hunt.rs` | 853 | `src/hunt.rs` (633) | 93 | **10.9** | 15.7 |
| `src/hunt_tables.rs` | 1318 | `src/hunt_tables.rs` (1476) | 161 | **12.2** | 14.9 |
| `src/launcher.rs` | 55 | `src/process.rs` (220) | 26 | **47.3** | 54.5 |
| `src/lib.rs` | 271 | `src/lib.rs` (319) | 80 | **29.5** | 29.5 |
| `src/matchmaker.rs` | 179 | `src/gameservers.rs` (1257) | 11 | **6.1** | 10.1 |
| `src/memory.rs` | 255 | `src/memory.rs` (167) | 28 | **11.0** | 20.4 |
| `src/occupancy.rs` | 194 | `src/occupancy.rs` (137) | 41 | **21.1** | 23.7 |
| `src/os.rs` | 343 | `src/win_spawn.rs` (273) | 43 | **12.5** | 20.7 |
| `src/port_pool.rs` | 143 | `src/port_pool.rs` (81) | 42 | **29.4** | 35.7 |
| `src/process.rs` | 201 | `src/gameservers.rs` (1257) | 17 | **8.5** | 14.4 |
| `tests/common/mod.rs` | 256 | `tests/common/mod.rs` (317) | 17 | 6.6 | 9.4 |
| `tests/e2e.rs` | 316 | `tests/director.rs` (474) | 51 | 16.1 | 16.1 |
| `tests/hunt.rs` | 197 | `tests/hunt.rs` (370) | 13 | 6.6 | 6.6 |
| `tests/import.rs` | 170 | `tests/hunt.rs` (370) | 3 | 1.8 | 3.5 |
| `tests/module.rs` | 123 | none | 0 | 0.0 | 0.0 |
| The 16 `src` files | 6034 | | 813 | **13.5** | 17.0 |
| The 5 test files | 1062 | | 84 | 7.9 | 8.9 |
| All 21 | 7096 | | 897 | **12.6** | 15.8 |

Four files reach 25 % against their best single file (`launcher.rs` 47.3, `cmdline.rs` 40.3, `lib.rs` 29.5,
`port_pool.rs` 29.4); all four are short (55 to 271 lines). The five fuzz targets
(`fuzz/fuzz_targets/rustic_isles_director_*.rs`) have no restricted counterpart and were not measured.

**What the matches are.** Every run of 5 lines or more against any of the 24 files was read: 66 of them over 57
distinct spans of the crate, 9 of which are, or end in, the `} #[cfg(test)] mod tests { use super::*; #[test]`
boilerplate. The rest:

- **§12.2's interface** (`lib.rs`, 80 matched lines; `launcher.rs`; `port_pool.rs`; parts of `hub.rs`, `memory.rs`,
  `hunt.rs`, `hunt_tables.rs`, `config.rs`): the `pub mod` list (`lib.rs:16-25`), `GameserverArgs` and
  `DirectorConfig` with their fields (`44-48`, `94-99`), `DirectorService`'s `new`/`with_launcher`/`config`/`director`
  (`179-197`), the `DedicatedServers` and `PortService` impls (`140-169`, `231-292`: the method heads `rustic_isles`
  fixes, the port `director`); `Command`, `Exit`, `Started`, `Launcher` and `OsLauncher` (`launcher.rs:14-65`);
  `Lease` and `PortPool` with §12.2's method names over §6's FIFO of free ports and leases held in the order taken
  (`port_pool.rs:6-42`, `90-98`: the one-line bodies of `new`, `take`, `free_count`, `held_count`); `Level` and
  `Options::new` over §12.2's field list (`hub.rs:33-43`, `98-107`); `MemoryAdmission::with(options, available, now)`
  (`memory.rs:128-164`); `Tables`, `ResolverOptions::new`, `HuntResolver::new` and the head of
  `tag_routed_matchmaker_hunt_ids` (`hunt.rs:56-61`, `289-295`, `340-345`, `398-404`); `ImportHuntTablesOptions`,
  `ImportHuntTablesReport` and `TableMergeSummary`'s fields (`hunt_tables.rs:107-145`); `Config::from_settings`'s
  head (`config.rs:113-118`); `GameServer`'s first fields (`gameservers.rs:98-102`, §3.4's instance fields).
- **§5.3's quoting rule** (`cmdline.rs`, 25): the loop counting backslashes and writing `2n+1` of them before an
  escaped `"` and `2n` before the closing quote (`cmdline.rs:15-37`) is the MSVC rule §5.3 states, in its stock form
  (Rust's standard library quotes the same way); `needs_quotes`, the `other` arm and `command_line`'s body do not
  match.
- **Spec tables and wire text**: §10.2's two game-type lists (`hunt_tables.rs:22-45`), §10.4's FName fields
  (`48-59`), §10.3's player and matchmaker key orders (`61-101`, runs of 23 and 16); §12.4's switches in the unit and
  black-box tests (`lib.rs:300-306`, `tests/common/mod.rs:26-32`, `tests/e2e.rs:129-134`); §9.2's `no Rows` error
  (`hunt.rs:79-88`); §9.4's any/all/none combinators (`hunt.rs:213-217`); §11's `GAMESERVER_LOG_CMDS` check
  (`lib.rs:51-55`); §7.4's edge grammar read in its order (digits, ` connCount `, a signed integer, ` -> `;
  `occupancy.rs:19-22`, `33-39`) and its re-base/poll state (position, partial line, count; `48-61`, `78-85`); §3.2's
  200/503 (`api.rs:51-55`) and D4's registration (`api.rs:70-90`); §14.2's Instances row as assertions in its order
  (`tests/e2e.rs:180-186`); §8's option defaults and problem text in `memory.rs`'s tests; §7.1's log lines in
  `hub.rs`.
- **Platform calls** (`os.rs`, 43): the job object with kill-on-close (`os.rs:63-74`), `CreateProcessW`'s argument
  list (`203-209`), the environment block's closing NULs (`133-137`) and Linux's `PR_SET_PDEATHSIG` (`353-358`): the
  sequences the `windows` and `libc` APIs dictate for §5.7. The job code and its two `SAFETY` comments also stand,
  word for word, in this repository's MIT `crates/module-api/src/tree.rs`, which §2 lists as read and §12.5 names as
  the pattern.

No comment was copied: the two comment lines that equal restricted ones are those `SAFETY` comments (`os.rs:48`,
`66`), from `tree.rs`. Of the crate's string literals of 12 characters or more that also occur in the 24 files, all
but a few are the spec's: the exceptions are §5.5's `M/D/YYYY, h:mm:ss AM` written as a `chrono` pattern
(`gameservers.rs:506`), §5.11's `Training Dojo down ({reason})! Restarting!` (which the check split at its `{`),
column names counted once per use, test fixture names (`tests/hunt.rs:32`, `55-56`, `tests/e2e.rs:291`), tokio's
`multi_thread` flavour, and the two pipe errors `no stdout pipe`/`no stderr pipe` (`os.rs:364-365`), the only shared
literals no spec gives. 143 of the crate's 347 function names also occur in restricted; all but about two dozen are
§12.2's, `std` trait methods or the seam traits' (`PortService`, `DedicatedServers`, `TreeControl`); the rest are
plain names for a spec step (`express_404`, `start_server`, `on_process_exit`, `live_hub`, `arena_week`,
`signal_name`, `merge_row`, `merge_table`, `read_json`, `handle_key` and the like), except `code_text` and
`signal_text` (below). No unusual structure is shared: the large files (`gameservers.rs`, `hunt.rs`,
`hunt_tables.rs`, `hub.rs`, the black-box tests) match at 2 to 16 %, and the registry, the start and readiness path,
the supervisor, the resolver, the tag query, the importer's merge, the Windows pipes and the shutdown share no logic
beyond the lines above.

**The one span that goes beyond the spec.** `src/launcher.rs:29-36`: `Exit::code_text` and `Exit::signal_text`, two
helpers the spec does not name, whose names and one-line bodies (`null` for none, through `map_or_else` and
`unwrap_or_else`) are identical to a restricted pair. They render §5.5's `code={code or null} signal={signal or
null}`, so the behaviour is the spec's and each line is a stock choice; but following `Exit`'s spec-given fields they
make an 11-line run the spec alone does not explain.

**Verdict: clean of structural or expressive copying, with one small rewrite asked for as a precaution.** Everything
else is §12.2's interface, §10's and §12.4's tables, §5.3's and §7.4's rules in their stock form, the platform APIs'
call sequences (shared with the MIT `tree.rs`), pinned test vectors and boilerplate. The rewrite, from §5.5 and §12.2
only and without looking at the current text: how an `Exit` becomes the log's `code=`/`signal=` text (for instance a
formatter at the call site, or a helper of another shape and name). Optionally, the `no stdout pipe`/`no stderr
pipe` texts (`os.rs:364-365`) in the implementer's own words. `cmdline.rs`, `lib.rs` and `port_pool.rs` need
nothing: their matched lines are §5.3's rule, §12.2's signatures and §6's pool in the only idiomatic form.
