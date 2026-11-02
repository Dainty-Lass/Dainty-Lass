# Clean-Room Record: `protocol` (`crates/protocol`)

## Provenance Summary

| Field | Value |
| --- | --- |
| Target crate | `protocol` (`crates/protocol`) |
| Licence | `MIT` (`Copyright (c) 2026 Silent Spud`) |
| Behaviour specification | `docs/port/spec-protocol.md` |
| Spec commit SHA | `1610b0b` (`7736fa5`) |
| Implementer session | Subagent `6bd23c4c-2567-465d-a969-d517f3d5409c` (`Clean-Room Protocol Implementer`) |
| Isolated worktree | `r:\source\dauntless\dainty-lass\.claude\worktrees\l2-protocol` (`feat/l2-protocol-mit`) |
| Worktree start SHA | `212856d` |
| Implementation end SHA | `6676f49` |

## Sparse-Checkout Configuration

Before the implementer subagent was invoked, the worktree was configured with `git sparse-checkout set --no-cone --stdin` and all `crates/protocol/src/*.rs` files were deleted from the working tree (`git rm -rf crates/protocol/src`):

```text
/*
!/legacy/
!/modules/
!/rust/services/
!/rust/launcher/
!/rust/crates/proxy/
!/rust/crates/store/
!/rust/crates/service-kit/
```

The implementer session had no access to the prior `crates/protocol/src/` implementation or any AGPL service/module/legacy tree, and did not run `git log`, `git show`, `git diff`, or `git blame` on any prior commits.

## Scope Implemented

- `crates/protocol/Cargo.toml` (`license = "MIT"`)
- `crates/protocol/src/lib.rs`
- `crates/protocol/src/text.rs`
- `crates/protocol/src/account.rs`
- `crates/protocol/src/invite.rs`
- `crates/protocol/src/status.rs`
- `crates/protocol/src/manifest.rs`
- `crates/protocol/tests/invite.rs` (SPDX MIT header added to fixture-table test harness)
- `crates/protocol/tests/manifest.rs` (SPDX MIT header added to fixture-table test harness)
- `crates/protocol/tests/tables.rs` (SPDX MIT header added to fixture-table test harness)
- `crates/protocol/tests/username_status.rs` (SPDX MIT header added to fixture-table test harness)

## Spec Questions and Clarifications During Implementation

1. **Question from implementer (`6bd23c4c-2567-465d-a969-d517f3d5409c`)**:
   - The existing fixture test harnesses (`tests/invite.rs`, `tests/manifest.rs`, `tests/username_status.rs`) imported crate-boundary constants and helpers (`REGISTER_PATH`, `ACCOUNT_KEY_PREFIX`, `RUNTIME_MANIFEST_PATH`, `STATUS_PATH`, `FormatError`, `DEFAULT_PUBLIC_PORT`, `MAX_INVITE_LENGTH`, `clean_https_url`, `clean_text`, `iso_date`, `limited_view`) and expected `ManifestGame::build: Option<String>`, `StatusGame::changelist: Option<u32>`, and `ServerStatus::uptime_seconds: u32`.
2. **Spec answer**:
   - Confirmed the public crate-boundary constants (`REGISTER_PATH = "/reforged/api/Register"`, `ACCOUNT_KEY_PREFIX = "RRK_"`, `RUNTIME_MANIFEST_PATH = "/content/v1/runtime/manifest"`, `STATUS_PATH = "/reforged/api/ServerStatus"`, `DEFAULT_PUBLIC_PORT = 443`, `MAX_INVITE_LENGTH = 2048`), the `FormatError` enum (`Host`, `Port`, `Code`, `Name`, `Share`, `Fp`, `Version`), and the exact field types (`ManifestGame::build: Option<String>`, `StatusGame::changelist: Option<u32>`, `ServerStatus::uptime_seconds: u32`).

## Verification

- `cargo test -p protocol -j 2 -- --test-threads=1`: 27 passed (13 in `tests/invite.rs`, 2 in `tests/manifest.rs`, 4 in `tests/tables.rs`, 8 in `tests/username_status.rs`).
- `cargo test -p protocol --all-features -j 2 -- --test-threads=1`: 27 passed.
- `cargo clippy -p protocol --all-features --all-targets -j 2 -- -D warnings`: 0 warnings.
