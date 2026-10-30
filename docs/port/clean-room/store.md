# Clean-room Provenance Record: `store` (`rust/crates/store`)

## 1. Specification Version

- **Spec file**: `docs/port/spec-store.md`
- **Spec commit on `plan`**: `07a12ea` (`Merge DD26+DL3a+DL4a: add clean-room behaviour spec for service-kit crate`; branch base `212856d`)

## 2. Implementer Session & Worktree Isolation

- **Implementer agent session**: [`964fb3dc-e3af-4f99-b3cd-a7de5eba2689`](conversation://964fb3dc-e3af-4f99-b3cd-a7de5eba2689) (`Clean-Room Store Implementer`)
- **Isolated worktree path**: `r:\source\dauntless\dainty-lass\.claude\worktrees\l2-store`
- **Branch**: `feat/l2-store-mit`
- **Pre-invocation removal**: `git rm -f` was executed before the implementer session was launched to remove all 6 DR-derived source files (`src/lib.rs`, `src/adopt.rs`, `src/backup.rs`, `src/drizzle.rs`, `src/readonly.rs`, `src/snapshot.rs`) and `tests/store.rs` from the working tree, leaving only the 13 born-here files (`DD15`, `DD23`, `DH3`, `DR4`).
- **`git sparse-checkout list` (`--no-cone`)**:
  ```text
  /*
  !/legacy/
  !/modules/
  !/rust/services/gateway/
  !/rust/services/allowlist/
  !/rust/services/content/
  !/rust/services/host/
  !/rust/launcher/
  !/rust/crates/protocol/
  !/rust/crates/proxy/
  !/rust/crates/service-kit/
  ```

## 3. Commit Range

- **Start commit (`plan` base)**: `212856db8b0fa4c2044719c91387516aa52369dc`
- **Implementation commit**: `5b0b2b6cf53aee3a6772e44132246eb4096d3839` (`DD26+DL3b+DL4b: reimplement store SQLite and Drizzle layer clean-room under MIT`)

## 4. Spec Writer / Implementer Q&A Log

- **Questions asked by implementer**: None — `docs/port/spec-store.md` fully specified the Diesel/r2d2 pool customiser, WAL pragma check, Drizzle journal and SQL breakpoint replay, baseline adoption, online backup snapshot transaction, read-only URI formatting, and schema snapshot order.

## 5. Verification

- `cargo test -p store -j 2 -- --test-threads=1`: 50 passed, 0 failed (20 unit tests, 13 in `tests/database.rs`, 7 in `tests/revert.rs`, 10 in `tests/store.rs`).
- `cargo clippy -p store --all-targets -j 2 -- -D warnings`: 0 warnings.
- `cargo tree -e normal -p store`: depends only on `module-api` and third-party MIT/Apache crates (zero AGPL dependencies).
- Provenance grep across `rust/crates/store`: 0 matches for `Dauntless Revived`, `ReforgedLauncher`, `Mystic Paradox`, `MIGRATION_PLAN.md`, `Freeza`, `mixutin`, or `e1c5fec`.
