# Clean-Room Record: `service-kit` (`crates/service-kit`)

## Provenance Summary

| Field | Value |
| --- | --- |
| Target crate | `service-kit` (`crates/service-kit`) |
| Licence | `MIT` (`Copyright (c) 2026 Silent Spud`) |
| Behaviour specification | `docs/port/spec-service-kit.md` |
| Spec commit SHA | `07a12ea` (`4d0d712`) |
| Implementer session | Subagent `bb7fbcb7-852c-4ca3-ad3d-0cf8122ca638` (`Clean-Room Service-Kit Implementer`) |
| Isolated worktree | `r:\source\dauntless\dainty-lass\.claude\worktrees\l2-service-kit` (`feat/l2-service-kit-mit`) |
| Worktree start SHA | `212856d` |
| Implementation end SHA | `d973d9f` |

## Sparse-Checkout Configuration

Before the implementer subagent was invoked, the worktree was configured with `git sparse-checkout set --no-cone --stdin` and all 12 DR-derived source files and 3 integration test files of `crates/service-kit` were removed from the working tree:

```text
/*
!/legacy/
!/modules/
!/rust/services/
!/rust/launcher/
!/rust/crates/protocol/
!/rust/crates/proxy/
!/rust/crates/store/
!/rust/crates/service-kit/src/lib.rs
!/rust/crates/service-kit/src/body.rs
!/rust/crates/service-kit/src/collate.rs
!/rust/crates/service-kit/src/env.rs
!/rust/crates/service-kit/src/js.rs
!/rust/crates/service-kit/src/jsjson.rs
!/rust/crates/service-kit/src/json.rs
!/rust/crates/service-kit/src/linelog.rs
!/rust/crates/service-kit/src/log.rs
!/rust/crates/service-kit/src/ratelimit.rs
!/rust/crates/service-kit/src/routing.rs
!/rust/crates/service-kit/src/serve.rs
!/rust/crates/service-kit/tests/
```

The implementer session had no access to any of the 12 replaced source files, the 3 replaced test files, or any AGPL service/module/legacy tree, and did not run `git log`, `git show`, `git diff`, or `git blame` on any prior commits.

## Scope Implemented & Retained

### Clean-Room Implemented (`MIT`)
- `crates/service-kit/Cargo.toml` (`license = "MIT"`)
- `crates/service-kit/src/lib.rs`
- `crates/service-kit/src/body.rs`
- `crates/service-kit/src/collate.rs`
- `crates/service-kit/src/env.rs`
- `crates/service-kit/src/js.rs`
- `crates/service-kit/src/jsjson.rs`
- `crates/service-kit/src/json.rs`
- `crates/service-kit/src/linelog.rs`
- `crates/service-kit/src/log.rs`
- `crates/service-kit/src/ratelimit.rs`
- `crates/service-kit/src/routing.rs`
- `crates/service-kit/src/serve.rs`
- `crates/service-kit/tests/body.rs`
- `crates/service-kit/tests/ratelimit.rs`
- `crates/service-kit/tests/routing.rs`

### Born-Here Files Retained (`MIT`)
- `crates/service-kit/src/context.rs`
- `crates/service-kit/src/telemetry.rs`

## Spec Questions and Clarifications During Implementation

None. `docs/port/spec-service-kit.md` provided complete signatures and behavioural contracts for all 12 modules and 3 test suites.

## Verification

- `cargo test -p service-kit -j 2 -- --test-threads=1`: 32 passed (26 unit tests + 6 integration tests across `body`, `ratelimit`, and `routing`).
- `cargo clippy -p service-kit --all-targets -j 2 -- -D warnings`: 0 warnings.
