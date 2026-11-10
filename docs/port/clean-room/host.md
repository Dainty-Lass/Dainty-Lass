# Clean-Room Provenance Record: `crates/dainty-host`'s game-data generator hook (RD27)

- **Component**: `crates/dainty-host/src/generators.rs` (crate `dainty-host`, `license = "MIT"`,
  `Copyright (c) 2026 Silent Spud`), the shell's one Category B part
- **Behaviour specification**: `docs/port/spec-rd27-host.md` §2.1 (refreshed 2026-11-09) at dainty-lass `dd76bd05`
- **Clean-room worktree**: `R:\source\wt-host-generators` (branch `feat/rd27-host-generators-mit`, at `dd76bd05`)
- **Implementer**: one Claude Code subagent session, briefed by the RD27 coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped, `git show`ed, `git log -p`ed or diffed anything under `restricted/`, in this worktree
or any other checkout; it did not open the main checkout `R:\source\dainty-lass` at all. It looked at no earlier
version of `crates/dainty-host` in git history (no `git log -p`, `git show <rev>:...`, `git blame` or cross-commit
diff of the crate); the only git commands run were `git branch --show-current`, `git status`, `git sparse-checkout
list`, `git log --oneline -3`, `git diff` of its own uncommitted work, `git add` and `git commit`. It searched and
fetched nothing on the web. Its inputs were the spec, the MIT code in this worktree as it is at `dd76bd05`,
`AGENTS.md`, and this folder's `chat.md` and `similarity.md` §1 for the record's format.

### 1.2 Worktree (`git sparse-checkout list`)

```text
/*
!/restricted/
!/.claude/
```

`restricted/` is not in the worktree, so no AGPL source was on disk in it.

## 2. Files read

- `docs/port/spec-rd27-host.md` (whole; §2.1 is the specification, the rest context)
- `AGENTS.md` (whole, given with the brief); `docs/port/clean-room/chat.md` (whole), `docs/port/clean-room/similarity.md`
  (§1 and the heading list)
- `crates/dainty-host/src/generators.rs` (the stub, whole), `crates/dainty-host/src/version_data.rs` (whole),
  `crates/dainty-host/build.rs` (whole), `crates/dainty-host/Cargo.toml` (whole)
- `crates/dainty-host/src/lib.rs`: lines 1-80 (module declarations, `BUILT_IN_SERVICES`, `ensure_hooks`,
  `db_cfg_to_target`), lines 490-520 (`install_director_runner` and the `run_generator` hook), and the lines a grep for
  `generator|^use|^mod|^pub mod|HostError|cfg(feature = "isles` and for `rustic_isles_gamedata|rustic_isles_director`
  printed (single lines of the `db` hooks' error mapping, line 1414)
- `crates/dainty-hostkit/src/gamedata.rs`: `GeneratorKind` with its derives, aliases and `label()` (lines 68-118),
  `Step` and the head of `Step::skip` (120-160), the list of `pub fn`s, `game_data_steps` and `run_generator`
  (415-495)
- `crates/dainty-hostkit/src/run.rs`: `RunResult` (lines 19-45) and its `pub fn` list
- `crates/dainty-hostkit/src/paths.rs`: the `Paths` fields (161-211) and `Paths::new` (240-300)
- `crates/dainty-hostkit/src/error.rs` lines 1-40 (`HostError`, `message`); `crates/dainty-hostkit/src/hooks.rs`
  lines 24-34 (the default `run_generator`)
- `crates/rustic-isles-director/src/hunt_tables.rs`: `ImportHuntTablesOptions` (111-127), the head of
  `TableMergeSummary`, `ImportHuntTablesReport` (161-170), the head of `import_hunt_tables` (1061-1083); the `pub mod`
  and `pub use` lines of `crates/rustic-isles-director/src/lib.rs`; the directory listing of its `src/`
- `crates/rustic-isles-gamedata/src/generators/mod.rs`: lines 58-130 (`BackendStep`, `GeneratorPaths`, `VersionInputs`,
  `run`); the `pub mod`/`pub use` lines of `crates/rustic-isles-gamedata/src/lib.rs`; listings of `src/` and
  `src/generators/`
- `crates/rustic-isles-1-14-7/src/data.rs`: `DATA_FILE` (lines 17-21) and `generator_inputs` (309-313)
- The directory listings of `crates/dainty-host/src` and `docs/port/clean-room`

## 3. Files written

| File | Spec |
| --- | --- |
| `crates/dainty-host/src/generators.rs` | §2.1: `run_generator` (signature kept), the `RunResult` mapping, the feature-off message, `HuntTables` through `import_hunt_tables`, the backend kinds through `version_data::install`, `load`, `generator_inputs` and `generators::run`; the tests §2.1 lists, as unit tests |
| `docs/port/clean-room/host.md` | this record |

No other file was changed: `lib.rs`'s hook already called `generators::run_generator`, and `version_data.rs`, the
manifest and `Cargo.lock` are as they were.

## 4. Verification

From the worktree root, on Windows:

- `cargo fmt --all --check`: clean
- `cargo clippy -p dainty-host --all-targets --locked -- -D warnings`: clean
- `cargo clippy -p dainty-host --no-default-features --all-targets --locked -- -D warnings`: **fails, not from this
  file.** The crate does not build cleanly that way at `dd76bd05`: `dainty-hostkit` with its default features off
  has two unused imports (`commands/remote.rs:8` `BTreeMap`, `guarded.rs:12` `Paths`), `dainty-host`'s `lib.rs` two
  (`Duration` at 27, `compose as modules` at 29) and `compose.rs:36` an `allow` without a reason, and
  `tests/admin_network.rs` does not compile (`dainty_gateway` missing, E0433). Without `-D warnings`, clippy reports
  nothing in `generators.rs`.
- `cargo test -p dainty-host --no-default-features --lib --locked generators`: 6 passed, 0 failed (the feature-off
  path, `without_the_module_every_kind_fails_and_writes_nothing` included)
- `cargo build --locked -p dainty-gateway-plugin`, then `cargo test -p dainty-host --locked --no-fail-fast`: unit tests
  11 passed (7 in `generators`), `admin_network` 4, `controller_allowlist` 6, `controller_network_wiring` 3, `history` 3,
  `install_game` 12, `network` 2, doc-tests 0; 0 failed. Built without `DAINTY_VERSION_DATA`, so the
  compiled-in-document path ran only as far as `load()`'s error.
- `--locked` needed no `Cargo.lock` change.
- Not run against real game data or the 1.14.7 client, nor against restricted's `tests/gamedata.rs` (spec §7: a
  reviewer outside the clean room does that).

## 5. Ambiguities and the assumptions taken

1. **"Trimmed" and "line".** Lines are `str::lines` (`\n` or `\r\n`), trimmed with `str::trim` (Unicode
   whitespace).
2. **The 300-character cut** is applied to the trimmed line, in `char`s, with no ellipsis or marker.
3. **Text with no non-blank line** gives `last_line: None` for success and failure alike; `code` and `output` are
   unchanged.
4. **`repo_root`** is `Some(paths.repo.clone())`; the other paths are cloned from `Paths` as named.
5. **The no-document case of step 1** is left to `version_data::install`, which already returns without writing; the
   hook adds no check of its own.
6. **The `generator_inputs` message** spells `version_data.json` literally, as the spec gives it; the install
   message takes the name from `rustic_isles_1_14_7::DATA_FILE`.
7. **The backend-kind test** uses `RewardCache` as its one kind and is compiled only without `dainty_version_data`,
   since with a compiled-in document `load()` succeeds.
8. **The `HuntTables` test** checks, beyond §2.1, that `last_line` is the output's first non-blank line and that the
   Director's game-data folder is not created; it does not pin the Director's error text.
9. **The feature-off test.** The crate as a whole does not build with `--no-default-features` (§4), so the feature-off
   test runs as a lib unit test with that flag, and a test of the message text itself is compiled in every build
   (`unavailable` is `cfg(any(test, not(feature = "isles-1-14-7")))`). It also checks that the temporary server folder
   stays empty ("nothing is read or written").

## 6. Similarity

Measured by the coordinator outside the clean room (similarity.md §1): `crates/dainty-host/src/generators.rs` at
`615bc64c` against the whole of restricted's `crates/dainty-host/src/lib.rs` at `cad68dd3`, the last version with the
derived dispatch in it.

| Code lines | In a run of 3+ identical lines | Similarity | Longest run |
| ---: | ---: | ---: | --- |
| 170 | 18 | 10.6% | 8 lines: the match arms from `GeneratorKind` to the `BackendStep` of the same name |

The other run is the 4 fields of `ImportHuntTablesOptions` that §2.1 fixes (`apply`, `update_existing`, `verbose`,
`backup_stamp`). Both are allowed by spec §7 (below 30%, no run over 6 lines outside import lists and match arms
naming the same variants).

## 7. Acceptance outside the clean room

- restricted's `dainty-host-parity` `tests/gamedata.rs` against the merged rewrite (`e8c8f097`): 21 passed, 0 failed,
  including `runs_the_rust_generators_and_names_the_error_while_continuing_other_steps` and
  `generates_all_seven_game_data_steps_in_rust_without_node`, which failed on the stub.
- `cargo test -p dainty-host` with `DAINTY_VERSION_DATA` set to restricted's document: all pass but
  `compose_starts_rust_1_14_7_backend_and_director_without_node_or_mongo`, which binds port 61000 and fails on the
  maintainer's Windows PC because the port is in Windows' reserved range (os error 10013); unrelated to this file.
- No AGPL crate can reach `dainty-host`'s tree: it is a root workspace member, whose CI step refuses a non-MIT package
  or a path into `restricted/`.
