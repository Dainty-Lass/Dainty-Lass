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

To be measured by the coordinator against restricted `e0e238b`.
